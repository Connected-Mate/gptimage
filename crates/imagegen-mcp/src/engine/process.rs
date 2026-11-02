//! `hoi4.image_process`: the family's processing, from a raw candidate (or imported art) to finished pixels.
//!
//! A chain has two parts. The caller may add pre-steps that prepare the source (`chroma_key`, `crop_to_aspect`,
//! `defringe`, `trim_to_subject`). The family's own processing is added and run by this module, always, and is
//! recorded with a recipe id and version so validation and the install gate can refuse an artifact that lacks it.
//! See `imagegen_core::chain` for the table of recipes.

use imagegen_core::chain::{self, recipe_of, Finisher, Recipe};
use imagegen_core::error::RetryClass;
use imagegen_core::export;
use imagegen_core::family::{Family, Group};
use imagegen_core::hashing::sha256_hex;
use imagegen_core::lock;
use imagegen_core::naming::{self, asset_id, candidate_id, processed_id};
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba, SourceFormat};
use imagegen_core::process::{run_chain_limited, Op, ICON_MAX_UPSCALE};
use imagegen_core::recipes::strip::build_strip;
use imagegen_core::records::{kind, CandidateRecord, CompanionRecord, CostRecord, FinisherRecord, ProcessedRecord, RecipeRecord};
use imagegen_core::reuse;
use imagegen_core::validate::{validate_image, Options};
use serde_json::{json, Value};

use crate::api::{ImportSpec, ProcessParams};

use super::{Artifact, EResult, Engine, EngineError};

impl Engine {
    /// Process a candidate into the family's finished art.
    ///
    /// The caller may add pre-steps that prepare the source. The family's own processing always runs after them: it
    /// is chosen here from the family alone, recorded in the processed record with the recipe id and version, and
    /// checked again by validation and by the install gate.
    pub fn process(&self, p: ProcessParams) -> EResult<Value> {
        let pre: Vec<Op> = match &p.pre_steps {
            Some(steps) => {
                chain::check_pre_steps(steps)?;
                steps.clone()
            }
            None => Vec::new(),
        };
        let (artifact, imported) = match (&p.candidate_id, &p.import) {
            (Some(_), Some(_)) | (None, None) => return Err(EngineError::invalid("give exactly one of candidate_id or import")),
            (Some(id), None) => {
                let a = self.artifact(id)?;
                if a.processed {
                    return Err(EngineError::invalid(format!(
                        "{id} is already a processed artifact; process its source candidate {} again to try other pre-steps",
                        a.root_id
                    )));
                }
                let imported = a.provider == "import";
                (a, imported)
            }
            (None, Some(spec)) => (self.import(spec, p.canvas)?, true),
        };
        let profile = self.profile_of(&artifact)?.clone();
        let recipe = recipe_of(&profile);
        if p.portrait_crop.is_some() && recipe != Recipe::Portrait {
            return Err(EngineError::invalid(format!("portrait_crop is for portrait families, not {}", profile.family)));
        }
        let (bytes, sha) = self.read_artifact(&artifact)?;
        if sha != artifact.recorded_sha256 {
            return Err(EngineError::refused(
                "candidate_changed",
                format!("{} was modified after it was recorded; restore it or import the new file as a new candidate", artifact.id),
                RetryClass::RefreshPreconditions,
            ));
        }
        let (img, _) = decode(&bytes, Some(&artifact.path), DecodeLimits::default())?;
        let limit = max_upscale(profile.family);
        // 1. the caller's pre-steps, 2. the family's finishing steps, 3. the engine-level stages
        let (prepared, mut records) = run_chain_limited(&img, &pre, limit)?;
        let keyed = pre.iter().any(|o| matches!(o, Op::ChromaKey { .. }));
        let plan = chain::plan_finishing(&profile, artifact.canvas, &prepared, imported, keyed, &artifact.recorded_sha256)?;
        let (mut main, finishing) = run_chain_limited(&prepared, &plan.ops, limit)?;
        records.extend(finishing);
        if p.second_frame_candidate.is_some() && !plan.finishers.contains(&Finisher::SecondFrame) {
            return Err(EngineError::invalid(format!("second_frame_candidate is for a generated building strip, not {}", profile.family)));
        }
        let second = match (&p.second_frame_candidate, plan.finishers.contains(&Finisher::SecondFrame)) {
            (Some(id), true) => Some((id.clone(), self.second_frame(&artifact, id, &pre, limit)?)),
            _ => None,
        };
        if p.frame_candidates.is_some() && !plan.finishers.contains(&Finisher::KeyFrames) {
            return Err(EngineError::invalid(format!("frame_candidates is for generated anim_blend_keyframes strips, not {}", profile.family)));
        }
        let keyframes = match &p.frame_candidates {
            Some(ids) => ids.iter().map(|id| Ok((id.clone(), self.key_frame(&artifact, id, &pre, limit)?))).collect::<EResult<Vec<_>>>()?,
            None => Vec::new(),
        };
        let mut finishers: Vec<FinisherRecord> = Vec::new();
        let mut companions: Vec<(String, Rgba)> = Vec::new();
        for f in &plan.finishers {
            let input_sha256 = main.pixels_sha256();
            let made = self.run_finisher(
                *f,
                &main,
                p.portrait_crop,
                second.as_ref().map(|(id, img)| (id.as_str(), img)),
                &keyframes,
                &profile,
                artifact.canvas,
            )?;
            finishers.push(FinisherRecord {
                name: f.name().to_string(),
                input_sha256,
                output_sha256: made.main.pixels_sha256(),
                detail: made.detail,
            });
            main = made.main;
            companions.extend(made.companions);
        }
        let png = encode_png(&main)?;

        let slug = &artifact.feature_slug;
        let dir = format!("docs/assets/{slug}/processed_png");
        let entries = self.ledger.entries()?;
        let mut run = entries
            .iter()
            .filter(|e| e.kind == kind::PROCESSED && e.data.get("source_id").and_then(Value::as_str) == Some(artifact.id.as_str()))
            .count() as u32
            + 1;
        let parsed = naming::ArtifactId::parse(&artifact.id)?;
        let (written_rel, run) = loop {
            let rel = format!("{dir}/{}-v{}-p{run}.png", artifact.asset, parsed.version);
            match self.fs.write_new(&rel, &png) {
                Ok(_) => break (rel, run),
                Err(imagegen_core::Error::Exists(_)) if run < 500 => run += 1,
                Err(e) => return Err(e.into()),
            }
        };
        // companion images sit next to the main one; if one cannot be written, nothing of this run stays behind
        let stem = written_rel.trim_end_matches(".png").to_string();
        let mut companion_records: Vec<CompanionRecord> = Vec::new();
        for (key, img) in &companions {
            let bytes = encode_png(img)?;
            let rel = format!("{stem}_{}.png", key.trim_start_matches('_'));
            if let Err(e) = self.fs.write_new(&rel, &bytes) {
                for r in companion_records.iter().map(|c| c.path.clone()).chain(std::iter::once(written_rel.clone())) {
                    if let Ok(abs) = self.fs.resolve(&r) {
                        let _ = std::fs::remove_file(abs);
                    }
                }
                return Err(e.into());
            }
            companion_records.push(CompanionRecord {
                key: key.clone(),
                path: rel,
                sha256: sha256_hex(&bytes),
                pixels_sha256: img.pixels_sha256(),
                size: [img.width, img.height],
            });
        }
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
            pixels_sha256: main.pixels_sha256(),
            size: [main.width, main.height],
            steps: records,
            profile: artifact.profile.clone(),
            recipe: Some(RecipeRecord { id: recipe.id().to_string(), version: recipe.version(), finishers }),
            companions: companion_records,
        };
        self.ledger.append(kind::PROCESSED, Some(&artifact.asset_id), Some(&id), &record)?;

        let opts = Options { canvas: (profile.free_canvas() && artifact.canvas != profile.canvas).then_some(artifact.canvas) };
        let quick = validate_image(&main, &profile, &profile.imagegen.checks, &opts);
        let problems = chain::audit(&record, &profile);
        Ok(json!({
            "artifact_id": id,
            "path": written_rel,
            "sha256": record.sha256,
            "pixels_sha256": record.pixels_sha256,
            "size": record.size,
            "recipe": {"id": recipe.id(), "version": recipe.version()},
            "steps": record.steps,
            "finishers": record.recipe.as_ref().map(|r| r.finishers.clone()).unwrap_or_default(),
            "companions": record.companions,
            "chain": {"complete": problems.is_empty(), "problems": problems},
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
    /// (`GFX-FAMILY-REUSE`). A strip family may be imported as one strip or as one file per frame (`frames`).
    fn import(&self, spec: &ImportSpec, canvas_override: Option<[u32; 2]>) -> EResult<Artifact> {
        export::validate_asset_name(spec.family, &spec.asset)?;
        naming::validate_snake("feature slug", &spec.feature_slug)?;
        let profile = self.family_profile(spec.family)?.clone();
        if self.config.policy.forbidden_families.contains(&spec.family) {
            return Err(EngineError::refused("family_forbidden_by_project", format!("this project forbids {}", spec.family), RetryClass::Terminal));
        }
        let frames = profile.frames.max(1);
        let (bytes, format, img, source_label, note) = match (&spec.path, &spec.frames) {
            (Some(path), None) => {
                let bytes = self.fs.read(path, 32 * 1024 * 1024)?;
                let (img, format) = decode(&bytes, Some(path), DecodeLimits::default())?;
                (bytes, format, img, format!("the image {path}"), format!("imported from {path}"))
            }
            (None, Some(files)) => {
                if frames < 2 {
                    return Err(EngineError::invalid(format!("{} is a single image; give path, not frames", spec.family)));
                }
                if files.len() as u32 != frames {
                    return Err(EngineError::invalid(format!("{} is a strip of {frames} frames; frames lists {} files", spec.family, files.len())));
                }
                let mut imgs = Vec::new();
                let mut described = Vec::new();
                for f in files {
                    let raw = self.fs.read(f, 32 * 1024 * 1024)?;
                    let (frame, _) = decode(&raw, Some(f), DecodeLimits::default())?;
                    described.push(format!("{f} ({})", &sha256_hex(&raw)[..12]));
                    imgs.push(frame);
                }
                let size = (imgs[0].width, imgs[0].height);
                if !profile.accepted.contains(&[size.0, size.1]) {
                    let want: Vec<String> = profile.accepted.iter().map(|a| format!("{}x{}", a[0], a[1])).collect();
                    return Err(EngineError::invalid(format!(
                        "each frame of {} must be exactly {}; the first is {}x{}",
                        spec.family,
                        want.join(" or "),
                        size.0,
                        size.1
                    )));
                }
                let strip = build_strip(&imgs, size).map_err(|e| EngineError::invalid(format!("the frames do not fit together: {e}")))?;
                let png = encode_png(&strip)?;
                (
                    png,
                    SourceFormat::Png,
                    strip,
                    format!("the frames {}", files.join(", ")),
                    format!("imported as {frames} frame files: {}", described.join(", ")),
                )
            }
            _ => return Err(EngineError::invalid("give exactly one of import.path or import.frames")),
        };
        reuse::ensure_no_cross_family_reuse(&self.fs, &self.profiles, spec.family, &reuse::pixel_hash(&img), &source_label)?;
        lock::check_native_size(&self.profiles, spec.family, [img.width, img.height])?;
        let canvas = if profile.needs_mod_canvas() {
            canvas_override.ok_or_else(|| EngineError::invalid(format!("{} has no fixed canvas: pass canvas [width, height]", spec.family)))?
        } else {
            // finished art of an accepted size keeps that size; anything else is fitted to the family's canvas
            let frame = [img.width / frames, img.height];
            if img.width % frames == 0 && profile.accepted.contains(&frame) {
                frame
            } else {
                profile.canvas
            }
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
            cost: CostRecord {
                estimated_usd: 0.0,
                reserved_usd: 0.0,
                actual_usd: Some(0.0),
                usage_unknown: false,
                funding: "local".into(),
                basis: "imported file, no provider call".into(),
            },
            parent: None,
            provider_request_id: None,
            usage: None,
            note: Some(note),
        };
        self.ledger.append(kind::IMPORTED, Some(&aid), Some(&rec.candidate_id), &rec)?;
        self.artifact(&rec.candidate_id)
    }

    /// A key frame of a blend key-frame strip: another raw candidate of the same asset (an edit of the base), with the
    /// caller's pre-steps applied. It is fitted together with the base by the `key_frames` finisher, not on its own.
    fn key_frame(&self, base: &Artifact, id: &str, pre: &[Op], limit: Option<f64>) -> EResult<Rgba> {
        let frame = self.artifact(id)?;
        if frame.processed || frame.id == base.id || frame.asset_id != base.asset_id || frame.family != base.family {
            return Err(EngineError::invalid(format!(
                "every frame_candidates entry must be another raw candidate of {} (an hoi4.image_edit of {}), not {id}",
                base.asset_id, base.id
            )));
        }
        let (bytes, sha) = self.read_artifact(&frame)?;
        if sha != frame.recorded_sha256 {
            return Err(EngineError::refused(
                "candidate_changed",
                format!("{id} was modified after it was recorded"),
                RetryClass::RefreshPreconditions,
            ));
        }
        let (img, _) = decode(&bytes, Some(&frame.path), DecodeLimits::default())?;
        let (prepared, _) = run_chain_limited(&img, pre, limit)?;
        Ok(prepared)
    }

    /// Frame 2 of a landmark strip: another raw candidate of the same asset, fitted exactly like frame 1.
    fn second_frame(&self, first: &Artifact, id: &str, pre: &[Op], limit: Option<f64>) -> EResult<Rgba> {
        let second = self.artifact(id)?;
        if second.processed || second.id == first.id || second.asset_id != first.asset_id || second.family != first.family {
            return Err(EngineError::invalid(format!(
                "second_frame_candidate must be another raw candidate of {} (for example from hoi4.image_edit), not {id}",
                first.asset_id
            )));
        }
        let (bytes, sha) = self.read_artifact(&second)?;
        if sha != second.recorded_sha256 {
            return Err(EngineError::refused(
                "candidate_changed",
                format!("{id} was modified after it was recorded"),
                RetryClass::RefreshPreconditions,
            ));
        }
        let (img, _) = decode(&bytes, Some(&second.path), DecodeLimits::default())?;
        let profile = self.profile_of(&second)?.clone();
        let (prepared, _) = run_chain_limited(&img, pre, limit)?;
        let keyed = pre.iter().any(|o| matches!(o, Op::ChromaKey { .. }));
        let plan = chain::plan_finishing(&profile, second.canvas, &prepared, false, keyed, &second.recorded_sha256)?;
        let (fitted, _) = run_chain_limited(&prepared, &plan.ops, limit)?;
        Ok(fitted)
    }
}

/// The largest enlargement the family tolerates in a fit or resize step: icon families only.
pub fn max_upscale(family: Family) -> Option<f64> {
    (family.group() == Group::Icon).then_some(ICON_MAX_UPSCALE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_icon_families_have_an_upscale_limit() {
        assert_eq!(max_upscale(Family::Focus), Some(ICON_MAX_UPSCALE));
        assert_eq!(max_upscale(Family::NewsEvent), None);
        assert_eq!(max_upscale(Family::FactionLogo), None);
    }
}
