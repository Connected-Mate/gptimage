//! The engine-level finishing stages of the family chains: the ones that need a template or a model from the mod
//! (the advisor card frame, the achievement backgrounds, the face detector) or that produce several images (the
//! achievement states, the flag sizes).
//!
//! The mod's copy of the asset skill ships the templates. They are found under the project root, never
//! outside it: `.claude/skills/*/assets/*/` (the kit names its folder `vanilla_reference`, Chaos Redux
//! `gfx_references`), or the folder named by `[templates] dir` in `.hoi4-imagegen/config.toml`. A missing template is
//! a refusal that names the paths it looked for.

use imagegen_core::chain::{Finisher, Recipe};
use imagegen_core::error::RetryClass;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::pixels::{decode, DecodeLimits, Rgba};
use imagegen_core::process::{apply, Op};
use imagegen_core::profile::Profile;
use imagegen_core::recipes::achievement::{achievement_triplet, templates_from_pixels, COMPLETED_TEMPLATE_SHA256, GREY_TEMPLATE_SHA256};
use imagegen_core::recipes::advisor::{advisor_card, AdvisorParams};
use imagegen_core::recipes::derived;
use imagegen_core::recipes::flags::flag_ladder;
use imagegen_core::recipes::pillow::alpha_composite;
use imagegen_core::recipes::portrait::{extract_portrait, CropChoice, FaceBox, FaceDetector};
use imagegen_core::recipes::strip::build_strip;
use serde_json::{json, Value};

use super::{EResult, Engine, EngineError};

/// Advisor frame template, relative to an assets folder.
pub const ADVISOR_TEMPLATE: &str = "portraits/advisors/advisor_template.png";
/// Achievement background templates, relative to an assets folder.
pub const ACHIEVEMENT_COMPLETED: &str = "icons/achievements/template/achievement_template.png";
/// Grey background.
pub const ACHIEVEMENT_GREY: &str = "icons/achievements/template/achievement_template_grey.png";
/// The red-cross overlay of the not-eligible state.
pub const ACHIEVEMENT_OVERLAY: &str = "icons/achievements/template/overlay.png";
/// SHA-256 of the kit's `overlay.png` (the kit's reference documents it).
pub const OVERLAY_SHA256: &str = "89BC80C6AC975BF6F1FF000FF3070B20C337BFB8B8AE966AE35A5540C004D6DD";
/// File name of the YuNet model.
pub const YUNET_MODEL: &str = "face_detection_yunet_2026may.onnx";

/// What a finishing stage made.
pub struct FinishOutput {
    /// The main image.
    pub main: Rgba,
    /// Companion images by key.
    pub companions: Vec<(String, Rgba)>,
    /// What was used and decided, for the processed record.
    pub detail: Value,
}

struct NoDetector;

impl FaceDetector for NoDetector {
    fn detect(&self, _: &Rgba) -> imagegen_core::Result<Vec<FaceBox>> {
        Err(imagegen_core::Error::Invalid("no face detector is used with a manual crop".into()))
    }
}

impl Engine {
    /// Project-relative folders that may hold the asset skill's templates, in search order.
    fn template_bases(&self) -> Vec<String> {
        if let Some(dir) = &self.config.templates.dir {
            return vec![dir.trim_end_matches(['/', '\\']).to_string()];
        }
        let mut out = Vec::new();
        let mut skills = self.fs.list_names(".claude/skills").unwrap_or_default();
        skills.sort();
        for s in skills {
            let assets = format!(".claude/skills/{s}/assets");
            let mut subs = self.fs.list_names(&assets).unwrap_or_default();
            subs.sort();
            out.extend(subs.into_iter().map(|a| format!("{assets}/{a}")));
        }
        out
    }

    /// The first assets folder that holds every one of `rels`.
    fn find_assets_base(&self, rels: &[&str]) -> Option<String> {
        self.template_bases().into_iter().find(|b| rels.iter().all(|r| self.fs.exists(&format!("{b}/{r}"))))
    }

    fn templates_missing(&self, what: &str, rels: &[&str]) -> EngineError {
        let root = self.config.templates.dir.clone().unwrap_or_else(|| ".claude/skills/*/assets/*".into());
        let expected: Vec<String> = rels.iter().map(|r| format!("{root}/{r}")).collect();
        EngineError::refused(
            "templates_missing",
            format!(
                "{what} needs templates that the mod's copy of the asset skill ships, and they were not found inside the project. Expected {}. Copy the skill's assets folder into the project, or name the folder with [templates] dir in .hoi4-imagegen/config.toml",
                expected.join(" and ")
            ),
            RetryClass::RefreshPreconditions,
        )
    }

    fn load_png(&self, rel: &str) -> EResult<(Rgba, String)> {
        let bytes = self.fs.read(rel, 8 * 1024 * 1024)?;
        let (img, _) = decode(&bytes, Some(rel), DecodeLimits::default())?;
        Ok((img, sha256_hex(&bytes)))
    }

    fn yunet_path(&self) -> Option<String> {
        if let Some(p) = &self.config.templates.face_model {
            return self.fs.exists(p).then(|| p.clone());
        }
        let mut skills = self.fs.list_names(".claude/skills").unwrap_or_default();
        skills.sort();
        skills.into_iter().map(|s| format!(".claude/skills/{s}/tools/models/{YUNET_MODEL}")).find(|p| self.fs.exists(p))
    }

    fn yunet_missing(&self) -> EngineError {
        let expected = self.config.templates.face_model.clone().unwrap_or_else(|| format!(".claude/skills/*/tools/models/{YUNET_MODEL}"));
        EngineError::refused(
            "face_model_missing",
            format!(
                "portraits are cropped around a detected face and the YuNet model was not found inside the project (expected {expected}). Copy the asset skill's tools/models folder into the project, set [templates] face_model, or pass portrait_crop [left, top, right, bottom] to choose the crop by hand"
            ),
            RetryClass::RefreshPreconditions,
        )
    }

    /// Refuse early, before anything is spent, when a family's finishing needs a template the project lacks.
    pub(crate) fn check_finishing_resources(&self, recipe: Recipe) -> EResult<()> {
        match recipe {
            Recipe::AdvisorCard if self.find_assets_base(&[ADVISOR_TEMPLATE]).is_none() => {
                Err(self.templates_missing("an advisor card", &[ADVISOR_TEMPLATE]))
            }
            Recipe::Achievement if self.find_assets_base(&[ACHIEVEMENT_COMPLETED, ACHIEVEMENT_GREY, ACHIEVEMENT_OVERLAY]).is_none() => {
                Err(self.templates_missing("an achievement icon", &[ACHIEVEMENT_COMPLETED, ACHIEVEMENT_GREY, ACHIEVEMENT_OVERLAY]))
            }
            _ => Ok(()),
        }
    }

    /// Whether the face model can be found (a plan warns when it cannot).
    pub fn face_model_available(&self) -> bool {
        self.yunet_path().is_some()
    }

    /// Run one finishing stage on the image the chain has produced so far.
    pub(crate) fn run_finisher(
        &self,
        finisher: Finisher,
        img: &Rgba,
        portrait_crop: Option<[i64; 4]>,
        second: Option<(&str, &Rgba)>,
        profile: &Profile,
        canvas: [u32; 2],
    ) -> EResult<FinishOutput> {
        match finisher {
            Finisher::DerivedCompanions => {
                let mut companions = Vec::new();
                let mut made = Vec::new();
                for d in imagegen_core::lookup::derived_companions(profile.family, canvas) {
                    let c = derived::derive(d.kind, img, profile.frames.max(1)).map_err(|e| {
                        EngineError::refused(
                            "companion_derivation_failed",
                            format!("the {} companion of this {} could not be made: {e}", d.key, profile.family),
                            RetryClass::Terminal,
                        )
                    })?;
                    if [c.width, c.height] != d.strip() {
                        return Err(EngineError::refused(
                            "companion_derivation_failed",
                            format!(
                                "the {} companion came out {}x{}, and the game needs {}x{}. The {} canvas is {}x{} (from {}); vanilla's is {}x{}. Fix the canvas (for a kit project, the row in .tools/gfx/profiles.json) and process again",
                                d.key,
                                c.width,
                                c.height,
                                d.strip()[0],
                                d.strip()[1],
                                profile.family,
                                canvas[0],
                                canvas[1],
                                match profile.origin {
                                    imagegen_core::profile::ProfileOrigin::Builtin => "the built-in table",
                                    imagegen_core::profile::ProfileOrigin::Kit => "the kit's profiles.json",
                                    imagegen_core::profile::ProfileOrigin::Project => "the project override",
                                },
                                profile.vanilla.as_ref().map_or(0, |v| v.canvas[0]),
                                profile.vanilla.as_ref().map_or(0, |v| v.canvas[1]),
                            ),
                            RetryClass::Terminal,
                        ));
                    }
                    made.push(json!({"key": d.key, "derivation": d.kind.name(), "size": d.strip(), "frames": d.frames}));
                    companions.push((d.key.to_string(), c));
                }
                Ok(FinishOutput {
                    main: img.clone(),
                    companions,
                    detail: json!({"companions": made, "note": "derived from the finished main art; vanilla draws these by hand, so they approximate its look"}),
                })
            }
            Finisher::AdvisorCard => {
                let base =
                    self.find_assets_base(&[ADVISOR_TEMPLATE]).ok_or_else(|| self.templates_missing("an advisor card", &[ADVISOR_TEMPLATE]))?;
                let rel = format!("{base}/{ADVISOR_TEMPLATE}");
                let (template, sha) = self.load_png(&rel)?;
                let made = advisor_card(img, &template, &AdvisorParams::default()).map_err(|e| {
                    let msg = e.to_string();
                    let code = if msg.contains("alpha coverage") { "advisor_alpha_coverage_failed" } else { "advisor_card_failed" };
                    EngineError::refused(code, format!("the advisor card could not be made: {msg}"), RetryClass::Terminal)
                })?;
                Ok(FinishOutput {
                    main: made.card,
                    companions: vec![],
                    detail: json!({
                        "template": rel, "template_sha256": sha,
                        "opening": {"center": made.opening.center, "size": made.opening.size, "rotation": made.opening.rotation},
                        "alpha_coverage": {"opening_gap": made.coverage.opening_alpha_gap_pixels, "inner_edge_gap": made.coverage.inner_edge_alpha_gap_pixels, "exterior_leak": made.coverage.exterior_alpha_leak_pixels},
                    }),
                })
            }
            Finisher::AchievementTriplet => {
                let rels = [ACHIEVEMENT_COMPLETED, ACHIEVEMENT_GREY, ACHIEVEMENT_OVERLAY];
                let base = self.find_assets_base(&rels).ok_or_else(|| self.templates_missing("an achievement icon", &rels))?;
                let (completed, c_sha) = self.load_png(&format!("{base}/{ACHIEVEMENT_COMPLETED}"))?;
                let (grey_bg, g_sha) = self.load_png(&format!("{base}/{ACHIEVEMENT_GREY}"))?;
                let (overlay, o_sha) = self.load_png(&format!("{base}/{ACHIEVEMENT_OVERLAY}"))?;
                if (overlay.width, overlay.height) != (64, 64) {
                    return Err(EngineError::invalid(format!(
                        "{base}/{ACHIEVEMENT_OVERLAY} must be 64x64, it is {}x{}",
                        overlay.width, overlay.height
                    )));
                }
                let templates = templates_from_pixels(completed, grey_bg)?;
                // the kit's rule: grey = grayscale of the subject, not-eligible = that grey with the red cross on top
                let grey = apply(img, &Op::Grayscale)?;
                let mut not_eligible = grey.clone();
                alpha_composite(&mut not_eligible, &overlay, 0, 0);
                let triplet = achievement_triplet(&templates, img, &grey, &not_eligible, false)?;
                Ok(FinishOutput {
                    main: triplet.completed,
                    companions: vec![("_grey".into(), triplet.grey), ("_not_eligible".into(), triplet.not_eligible)],
                    detail: json!({
                        "templates": base,
                        "completed_template_sha256": c_sha, "grey_template_sha256": g_sha, "overlay_sha256": o_sha,
                        "pinned_hashes_match": {
                            "completed": c_sha.eq_ignore_ascii_case(COMPLETED_TEMPLATE_SHA256),
                            "grey": g_sha.eq_ignore_ascii_case(GREY_TEMPLATE_SHA256),
                            "overlay": o_sha.eq_ignore_ascii_case(OVERLAY_SHA256),
                        },
                        "grey_source": "grayscale of the subject layer",
                        "not_eligible_source": "the grey source with overlay.png composited on top at exact alignment",
                    }),
                })
            }
            Finisher::ExtractPortrait => {
                let (detector, model): (Box<dyn FaceDetector>, Value) = if portrait_crop.is_some() {
                    (Box::new(NoDetector), json!(null))
                } else {
                    let rel = self.yunet_path().ok_or_else(|| self.yunet_missing())?;
                    let bytes = self.fs.read(&rel, 64 * 1024 * 1024)?;
                    let net = imagegen_core::recipes::face::YuNet::from_onnx_bytes(&bytes)?;
                    (Box::new(net), json!({"path": rel, "sha256": sha256_hex(&bytes)}))
                };
                let made = extract_portrait(img, &*detector, portrait_crop).map_err(|e| {
                    EngineError::refused(
                        "portrait_crop_failed",
                        format!("the portrait crop failed: {e}. Pass portrait_crop [left, top, right, bottom] in source pixels to choose it by hand"),
                        RetryClass::Terminal,
                    )
                })?;
                let choice = match made.choice {
                    CropChoice::Detected(b) => json!({"detected_face": b}),
                    CropChoice::Manual => json!({"manual": true}),
                };
                Ok(FinishOutput { main: made.portrait, companions: vec![], detail: json!({"crop": made.crop, "choice": choice, "model": model}) })
            }
            Finisher::SecondFrame => {
                let (second_id, second) = second.ok_or_else(|| {
                    EngineError::refused(
                        "second_frame_needed",
                        "a building is a two-frame strip and vanilla shows the ruined landmark in frame 2, which cannot be derived from frame 1. Make it with hoi4.image_edit on this candidate (for example \"the same landmark in ruins, burning, same angle and framing\"), then process again with second_frame_candidate",
                        RetryClass::Terminal,
                    )
                })?;
                let strip = build_strip(&[img.clone(), second.clone()], (img.width, img.height))
                    .map_err(|e| EngineError::invalid(format!("the two frames do not fit together: {e}")))?;
                Ok(FinishOutput {
                    main: strip,
                    companions: vec![],
                    detail: json!({"second_frame_candidate": second_id, "second_frame_sha256": second.pixels_sha256()}),
                })
            }
            Finisher::FlagLadder => {
                let ladder = flag_ladder(img)?;
                Ok(FinishOutput {
                    main: ladder.normal,
                    companions: vec![("medium".into(), ladder.medium), ("small".into(), ladder.small)],
                    detail: json!({"sizes": ["82x52", "41x26", "10x7"], "origin": "bottom-left TGA at install"}),
                })
            }
        }
    }
}
