//! `hoi4.image_process`: the deterministic chain from a raw candidate (or imported art) to profile-ready pixels.

use imagegen_core::error::RetryClass;
use imagegen_core::export;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::naming::{self, asset_id, candidate_id, processed_id};
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba, SourceFormat};
use imagegen_core::family::{Family, Group};
use imagegen_core::lock;
use imagegen_core::process::{edge_stats, run_chain_limited, Op, ICON_MAX_UPSCALE};
use imagegen_core::reuse;
use imagegen_core::profile::{Background, Profile};
use imagegen_core::records::{kind, CandidateRecord, CostRecord, ProcessedRecord};
use imagegen_core::validate::{validate_image, Options};
use serde_json::{json, Value};

use crate::api::{ImportSpec, ProcessParams};

use super::{Artifact, EResult, Engine, EngineError};

impl Engine {
    /// Process a candidate into profile-ready art.
    pub fn process(&self, p: ProcessParams) -> EResult<Value> {
        let (artifact, imported) = match (&p.candidate_id, &p.import) {
            (Some(_), Some(_)) | (None, None) => return Err(EngineError::invalid("give exactly one of candidate_id or import")),
            (Some(id), None) => {
                let a = self.artifact(id)?;
                if a.processed {
                    return Err(EngineError::invalid(format!("{id} is already a processed artifact; process its source candidate {} again to try other steps", a.root_id)));
                }
                let imported = a.provider == "import";
                (a, imported)
            }
            (None, Some(spec)) => (self.import(spec, p.canvas)?, true),
        };
        let profile = self.profile_of(&artifact)?.clone();
        let (bytes, sha) = self.read_artifact(&artifact)?;
        if sha != artifact.recorded_sha256 {
            return Err(EngineError::refused(
                "candidate_changed",
                format!("{} was modified after it was recorded; restore it or import the new file as a new candidate", artifact.id),
                RetryClass::RefreshPreconditions,
            ));
        }
        let (img, _) = decode(&bytes, Some(&artifact.path), DecodeLimits::default())?;
        let steps = match &p.steps {
            Some(s) if !s.is_empty() => s.clone(),
            Some(_) => Vec::new(),
            None => default_chain(&profile, artifact.canvas, &img, imported)?,
        };
        let (out, records) = run_chain_limited(&img, &steps, max_upscale(profile.family))?;
        let png = encode_png(&out)?;

        let slug = &artifact.feature_slug;
        let dir = format!("docs/assets/{slug}/processed_png");
        let entries = self.ledger.entries()?;
        let mut run = entries.iter().filter(|e| e.kind == kind::PROCESSED && e.data.get("source_id").and_then(Value::as_str) == Some(artifact.id.as_str())).count() as u32 + 1;
        let parsed = naming::ArtifactId::parse(&artifact.id)?;
        let (written_rel, run) = loop {
            let rel = format!("{dir}/{}-v{}-p{run}.png", artifact.asset, parsed.version);
            match self.fs.write_new(&rel, &png) {
                Ok(_) => break (rel, run),
                Err(imagegen_core::Error::Exists(_)) if run < 500 => run += 1,
                Err(e) => return Err(e.into()),
            }
        };
        let id = processed_id(&candidate_id(&artifact.asset_id, parsed.version), run);
        let record = ProcessedRecord {
            artifact_id: id.clone(),
            source_id: artifact.id.clone(),
            asset_id: artifact.asset_id.clone(),
            run,
            family: artifact.family,
            canvas: artifact.canvas,
            path: written_rel.clone(),
            sha256: sha256_hex(&png),
            pixels_sha256: out.pixels_sha256(),
            size: [out.width, out.height],
            steps: records,
            profile: artifact.profile.clone(),
        };
        self.ledger.append(kind::PROCESSED, Some(&artifact.asset_id), Some(&id), &record)?;

        let opts = Options { canvas: (profile.canvas == [0, 0]).then_some(artifact.canvas) };
        let quick = validate_image(&out, &profile, &profile.imagegen.checks, &opts);
        Ok(json!({
            "artifact_id": id,
            "path": written_rel,
            "sha256": record.sha256,
            "pixels_sha256": record.pixels_sha256,
            "size": record.size,
            "steps": record.steps,
            "quick_check": {
                "status": quick.status,
                "errors": quick.errors(),
                "warnings": quick.warnings(),
                "findings": quick.findings,
                "note": "This is a preview of the verdict only. Run hoi4.image_validate to record it; install reads the recorded validation."
            },
            "next": "hoi4.image_validate, then hoi4.image_preview, hoi4.image_review, hoi4.image_install"
        }))
    }

    /// Bring an existing image under the ledger as a candidate.
    ///
    /// The family lock applies here: an image whose native size belongs to another family is refused
    /// (`GFX-FAMILY-MISMATCH`), and so are the pixels of an asset already installed for another family
    /// (`GFX-FAMILY-REUSE`).
    fn import(&self, spec: &ImportSpec, canvas_override: Option<[u32; 2]>) -> EResult<Artifact> {
        export::validate_asset_name(spec.family, &spec.asset)?;
        naming::validate_snake("feature slug", &spec.feature_slug)?;
        let profile = self.family_profile(spec.family)?.clone();
        if self.config.policy.forbidden_families.contains(&spec.family) {
            return Err(EngineError::refused("family_forbidden_by_project", format!("this project forbids {}", spec.family), RetryClass::Terminal));
        }
        let bytes = self.fs.read(&spec.path, 32 * 1024 * 1024)?;
        let (img, format) = decode(&bytes, Some(&spec.path), DecodeLimits::default())?;
        reuse::ensure_no_cross_family_reuse(&self.fs, &self.profiles, spec.family, &reuse::pixel_hash(&img), &format!("the image {}", spec.path))?;
        lock::check_native_size(&self.profiles, spec.family, [img.width, img.height])?;
        let frames = profile.frames.max(1);
        let canvas = if profile.needs_mod_canvas() {
            canvas_override.ok_or_else(|| EngineError::invalid(format!("{} has no fixed canvas: pass canvas [width, height]", spec.family)))?
        } else {
            // finished art of an accepted size keeps that size; anything else is fitted to the family's canvas
            let frame = [img.width / frames, img.height];
            if img.width % frames == 0 && profile.accepted.contains(&frame) { frame } else { profile.canvas }
        };
        let ext = match format {
            SourceFormat::Png => "png",
            SourceFormat::Jpeg => "jpg",
            SourceFormat::WebP => "webp",
            SourceFormat::Dds => "dds",
            SourceFormat::Tga => "tga",
        };
        let aid = asset_id(&spec.feature_slug, &spec.asset);
        let min = self.ledger.highest_version(&aid)?;
        let dir = format!("docs/assets/{}/source_png", spec.feature_slug);
        let written = self.fs.write_next_version(&dir, &spec.asset, ext, min, &bytes)?;
        let sha = sha256_hex(&bytes);
        let rec = CandidateRecord {
            candidate_id: candidate_id(&aid, written.version),
            asset_id: aid.clone(),
            family: spec.family,
            feature_slug: spec.feature_slug.clone(),
            asset: spec.asset.clone(),
            profile: self.profile_ref(&profile),
            canvas,
            version: written.version,
            path: written.rel.clone(),
            sha256: sha.clone(),
            pixels_sha256: img.pixels_sha256(),
            size: [img.width, img.height],
            has_transparency: img.has_transparency(),
            provider: "import".into(),
            model: "import".into(),
            plan_id: None,
            plan_hash: None,
            request_hash: sha.clone(),
            idempotency_key: None,
            prompt_sha256: String::new(),
            references: vec![],
            cost: CostRecord { estimated_usd: 0.0, reserved_usd: 0.0, actual_usd: Some(0.0), usage_unknown: false, funding: "local".into(), basis: "imported file, no provider call".into() },
            parent: None,
            provider_request_id: None,
            usage: None,
            note: Some(format!("imported from {}", spec.path)),
        };
        self.ledger.append(kind::IMPORTED, Some(&aid), Some(&rec.candidate_id), &rec)?;
        self.artifact(&rec.candidate_id)
    }
}

/// The largest enlargement the family tolerates in a fit or resize step: icon families only.
pub fn max_upscale(family: Family) -> Option<f64> {
    (family.group() == Group::Icon).then_some(ICON_MAX_UPSCALE)
}

/// What provider alpha is composited over in an opaque family.
const FLATTEN_BACKGROUND: [u8; 3] = [0, 0, 0];

/// The family's default chain.
///
/// Opaque families always end in a flatten step, so provider alpha never survives into a picture, a portrait,
/// a flag or an achievement. The report card is the one exception for finished (imported) art: its alpha is
/// the card's corners and shadow.
pub fn default_chain(profile: &Profile, canvas: [u32; 2], img: &Rgba, imported: bool) -> EResult<Vec<Op>> {
    let frames = profile.frames.max(1);
    let strip = [canvas[0] * frames, canvas[1]];
    let opaque = profile.imagegen.background == Background::Opaque;
    let keeps_card_alpha = profile.family == Family::ReportEvent && imported;
    let flatten = opaque && !keeps_card_alpha;
    // art that already has an accepted size is taken as it is: a finished icon must not be re-fitted
    if imported && img.width % frames == 0 {
        let frame = [img.width / frames, img.height];
        if profile.accepted.contains(&frame) || frame == canvas {
            return Ok(if flatten { vec![Op::FlattenOn { background: FLATTEN_BACKGROUND }] } else { Vec::new() });
        }
    }
    if frames > 1 {
        return Err(EngineError::invalid(format!(
            "{} art is a strip of {frames} frames and needs separate art for each; assemble it first. Imported art of exactly {}x{} is accepted unchanged",
            profile.family, strip[0], strip[1]
        )));
    }
    match profile.imagegen.background {
        Background::Transparent => {
            if !img.has_transparency() {
                return Err(EngineError::refused(
                    "no_transparency_in_source",
                    "the source is fully opaque, so it carries no transparency. A transparent icon needs real alpha: generate with a model that supports it, run a matte workflow, or pass explicit steps with a chroma_key if you know the background is one flat colour",
                    RetryClass::Terminal,
                ));
            }
            let mut steps = Vec::new();
            if edge_stats(img).1 > 0 {
                steps.push(Op::Defringe { erosion: 1 });
            }
            steps.push(Op::TrimToSubject { min_alpha: 16 });
            steps.push(Op::FitSubject { width: canvas[0], height: canvas[1], margin: profile.margin, min_alpha: 16 });
            if profile.imagegen.grayscale {
                steps.push(Op::Grayscale);
            }
            Ok(steps)
        }
        Background::Opaque => {
            let mut steps = Vec::new();
            if flatten {
                steps.push(Op::FlattenOn { background: FLATTEN_BACKGROUND });
            }
            steps.push(Op::CoverResize { width: canvas[0], height: canvas[1] });
            if profile.imagegen.grayscale {
                steps.push(Op::Grayscale);
            }
            Ok(steps)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imagegen_core::family::Family;
    use imagegen_core::profile::ProfileSet;

    fn prof(c: Family) -> Profile {
        ProfileSet::builtin().for_family(c).unwrap().clone()
    }

    fn blob() -> Rgba {
        let mut img = Rgba::new(200, 180);
        for y in 30..150 {
            for x in 40..170 {
                img.set_px(x, y, [120, 100, 80, 255]);
            }
        }
        img
    }

    #[test]
    fn transparent_icons_trim_and_fit_with_the_profile_margin() {
        let p = prof(Family::Focus);
        let steps = default_chain(&p, p.canvas, &blob(), false).unwrap();
        assert_eq!(steps.len(), 2);
        assert!(matches!(steps[1], Op::FitSubject { width: 100, height: 88, margin: 2, .. }));
    }

    #[test]
    fn an_opaque_source_is_refused_for_a_transparent_icon() {
        let p = prof(Family::Focus);
        let err = default_chain(&p, p.canvas, &Rgba::filled(64, 64, [1, 2, 3, 255]), false).unwrap_err();
        assert_eq!(err.code, "no_transparency_in_source");
    }

    #[test]
    fn pictures_cover_resize_and_news_goes_grayscale() {
        let news = prof(Family::NewsEvent);
        let steps = default_chain(&news, news.canvas, &Rgba::filled(800, 600, [9, 9, 9, 255]), false).unwrap();
        assert_eq!(steps, vec![Op::FlattenOn { background: [0, 0, 0] }, Op::CoverResize { width: 397, height: 153 }, Op::Grayscale]);
        let report = prof(Family::ReportEvent);
        assert_eq!(default_chain(&report, report.canvas, &Rgba::filled(10, 10, [9, 9, 9, 255]), false).unwrap().len(), 2);
        assert!(default_chain(&report, report.canvas, &Rgba::filled(210, 176, [9, 9, 9, 255]), true).unwrap().is_empty(), "an imported report card keeps its card alpha");
    }

    #[test]
    fn finished_imports_are_left_alone_and_frame_strips_need_finished_art() {
        let idea = prof(Family::IdeaSpirit);
        let finished = blob().crop(0, 0, 60, 68).unwrap();
        assert!(default_chain(&idea, idea.canvas, &finished, true).unwrap().is_empty());
        assert!(!default_chain(&idea, idea.canvas, &finished, false).unwrap().is_empty(), "generated art is always fitted");
        let counter = prof(Family::CounterLarge);
        assert!(default_chain(&counter, counter.canvas, &Rgba::new(152, 42), true).unwrap().is_empty());
        assert!(default_chain(&counter, counter.canvas, &Rgba::new(100, 42), true).is_err());
    }

    #[test]
    fn magenta_rims_add_a_defringe_step() {
        let p = prof(Family::IdeaSpirit);
        let mut img = Rgba::new(80, 80);
        for y in 10..70 {
            for x in 10..70 {
                let edge = x == 10 || x == 69 || y == 10 || y == 69;
                img.set_px(x, y, if edge { [255, 0, 255, 255] } else { [100, 90, 80, 255] });
            }
        }
        let steps = default_chain(&p, p.canvas, &img, false).unwrap();
        assert!(matches!(steps[0], Op::Defringe { erosion: 1 }));
    }
}
