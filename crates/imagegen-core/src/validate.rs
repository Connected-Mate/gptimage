//! Deterministic validation.
//!
//! Everything here is a pure function of pixels or bytes and a profile, so two runs always agree. Finding
//! codes reuse the kit's `gfx.py` codes where the rule is the same (`GFX-OPAQUE-CANVAS`, `GFX-EMPTY-SUBJECT`,
//! `GFX-GHOSTED`, `GFX-CHROMA-FRINGE`, `GFX-FRAME-WIDTH`, `GFX-FLAG-ORIGIN`, `GFX-TRUNCATED`, `GFX-LFS-POINTER`).
//! Deterministic checks say nothing about composition, style or readability at game size: those go to the
//! visual review, which is recorded separately. A resized image that passes the size check is never reported
//! as "suitable".

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::dds;
use crate::pixels::Rgba;
use crate::process::edge_stats;
use crate::profile::{Background, Profile};
use crate::styledesc::{self, DriftModel};
use crate::tga;

/// Severity of a finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    /// Blocks install.
    Error,
    /// Should be reviewed.
    Warning,
    /// Heuristic hint, never blocks.
    Advisory,
}

/// One finding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Finding {
    /// Stable code.
    pub code: String,
    /// Severity.
    pub level: Level,
    /// Explanation.
    pub message: String,
    /// 1-based frame number for per-frame findings.
    pub frame: Option<u32>,
    /// Numbers behind the finding.
    pub evidence: Value,
}

impl Finding {
    fn new(code: &str, level: Level, message: String, frame: Option<u32>, evidence: Value) -> Self {
        Finding { code: code.to_string(), level, message, frame, evidence }
    }
}

/// Pass or fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// No error-level finding.
    Pass,
    /// At least one error-level finding.
    Fail,
}

/// Measurements of one frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameStats {
    /// Mean alpha over pixels with alpha above zero.
    pub subject_alpha_mean: f64,
    /// Smallest alpha.
    pub alpha_min: u8,
    /// Largest alpha.
    pub alpha_max: u8,
    /// Pixels with alpha above zero.
    pub subject_pixels: u64,
    /// Pixels with alpha 255.
    pub opaque_pixels: u64,
    /// `[x0, y0, x1, y1)` of pixels with alpha >= 16, if any.
    pub visible_bounds: Option<[u32; 4]>,
    /// Left, top, right, bottom margins to that box.
    pub margins: Option<[u32; 4]>,
    /// Alpha of the four corners (TL, TR, BL, BR).
    pub corner_alpha: [u8; 4],
    /// Visible edge pixels.
    pub edge_pixels: u64,
    /// Edge pixels in the magenta or green key colours.
    pub chroma_edge_pixels: u64,
}

/// Result of a validation run.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Report {
    /// Pass or fail.
    pub status: Status,
    /// All findings.
    pub findings: Vec<Finding>,
    /// Per-frame measurements.
    pub frames: Vec<FrameStats>,
    /// Image size.
    pub size: [u32; 2],
    /// SHA-256 of the RGBA pixels.
    pub pixels_sha256: String,
    /// Checks that ran.
    pub checks_run: Vec<String>,
}

impl Report {
    /// Number of error-level findings.
    pub fn errors(&self) -> usize {
        self.findings.iter().filter(|f| f.level == Level::Error).count()
    }

    /// Number of warning-level findings.
    pub fn warnings(&self) -> usize {
        self.findings.iter().filter(|f| f.level == Level::Warning).count()
    }
}

/// Per-run options.
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// Canvas to expect instead of the profile's (custom and GUI categories).
    pub canvas: Option<[u32; 2]>,
}

/// Every check id this crate knows.
pub const ALL_CHECKS: &[&str] = &[
    "size",
    "alpha",
    "opaque_canvas",
    "opaque",
    "near_opaque",
    "card_corners",
    "fringe",
    "margin",
    "fill",
    "checker",
    "ghost",
    "frames",
    "animation",
    "line_art",
    "unit_material",
    "side_view",
    "style",
    "dds",
    "tga",
];

/// Checks that judge the main image only: a companion (a tinted text icon, a smaller strip) is not the family's
/// finished art and skips them.
pub const MAIN_IMAGE_ONLY_CHECKS: &[&str] = &["unit_material", "side_view", "line_art"];

/// Code of a special project drawing with a filled area (`line_art` check).
pub const FILLED_CODE: &str = "GFX-FILLED-INTERIOR";
/// Code of finished unit art that is not in vanilla's material (`unit_material` check).
pub const UNIT_MATERIAL_CODE: &str = "GFX-UNIT-MATERIAL";
/// Code of a technology picture that may not be vanilla's side view (`side_view` check).
pub const SIDE_VIEW_CODE: &str = "GFX-TECH-VIEW";

/// The narrowest a technology picture's subject may be, width over height of its visible box, before the side view
/// is doubted: vanilla's 10th percentile (HOI4 1.19.4.1: 2.30 over 269 pictures at 146x54, 1.81 over 55 at 131x52;
/// medians 3.09 and 2.29). A side profile of a tank, gun, aircraft or ship is long and low; a three-quarter or front
/// view is not.
pub fn side_view_min_aspect(canvas: [u32; 2]) -> f64 {
    match canvas {
        [131, 52] => 1.81,
        _ => 2.30,
    }
}

/// Alpha every pixel of a template-backed picture (the achievement icons) must reach. Vanilla's templates sit at
/// 254 and 255.
pub const NEAR_OPAQUE_MIN_ALPHA: u8 = 250;

/// Hash of a check set (order does not matter). The install gate compares the hash recorded with a validation
/// against the full set of the artifact's family, so a validation that skipped a check cannot open it.
pub fn check_set_hash(checks: &[String]) -> String {
    let mut sorted: Vec<&str> = checks.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    sorted.dedup();
    crate::hashing::sha256_hex(
        format!(
            "hoi4-imagegen-check-set-v1
{}",
            sorted.join(
                "
"
            )
        )
        .as_bytes(),
    )
}

/// Validate decoded pixels against a profile. `checks` selects which checks run; pass the profile's list
/// for the default set.
pub fn validate_image(img: &Rgba, profile: &Profile, checks: &[String], opts: &Options) -> Report {
    let wants = |name: &str| checks.iter().any(|c| c == name);
    let mut findings = Vec::new();
    let mut run = Vec::new();

    let frames = profile.frames.max(1);
    let canvas = opts.canvas.unwrap_or(profile.canvas);
    let accepted: Vec<[u32; 2]> = if opts.canvas.is_some() || profile.accepted.is_empty() {
        if canvas == [0, 0] {
            Vec::new()
        } else {
            vec![canvas]
        }
    } else {
        profile.accepted.clone()
    };

    // frame split (falls back to one frame when the width does not divide)
    let divisible = img.width.is_multiple_of(frames);
    if !divisible && (wants("frames") || wants("size")) {
        run.push("frames".to_string());
        findings.push(Finding::new(
            "GFX-FRAME-WIDTH",
            Level::Error,
            format!(
                "width {} does not divide into {frames} frames; per-frame width would be {:.2}",
                img.width,
                f64::from(img.width) / f64::from(frames)
            ),
            None,
            json!({"width": img.width, "frames": frames}),
        ));
    }
    let frame_imgs: Vec<Rgba> =
        if divisible && frames > 1 { img.split_frames(frames).unwrap_or_else(|_| vec![img.clone()]) } else { vec![img.clone()] };
    let frame_count = frame_imgs.len() as u32;

    if wants("size") {
        run.push("size".into());
        if accepted.is_empty() {
            findings.push(Finding::new(
                "GFX-CANVAS-UNSPECIFIED",
                Level::Advisory,
                "this profile has no fixed canvas; pass an explicit canvas to check the size".into(),
                None,
                json!({}),
            ));
        } else {
            let fw = img.width / frame_count.max(1);
            let ok = divisible && accepted.iter().any(|a| a[0] == fw && a[1] == img.height);
            if !ok {
                let want: Vec<String> = accepted.iter().map(|a| format!("{}x{}", a[0] * frame_count.max(1), a[1])).collect();
                findings.push(Finding::new(
                    "GFX-CANVAS-SIZE",
                    Level::Error,
                    format!("canvas is {}x{}, expected {}", img.width, img.height, want.join(" or ")),
                    None,
                    json!({"size": [img.width, img.height], "accepted": accepted, "frames": frames}),
                ));
            } else if [fw, img.height] != accepted[0] && opts.canvas.is_none() {
                findings.push(Finding::new(
                    "GFX-CANVAS-ALTERNATIVE",
                    Level::Advisory,
                    format!("size {fw}x{} is accepted but new art uses {}x{}", img.height, accepted[0][0], accepted[0][1]),
                    None,
                    json!({"accepted_first": accepted[0]}),
                ));
            }
        }
    }

    let transparent_profile = profile.imagegen.background == Background::Transparent;
    let mut stats = Vec::new();
    for (i, f) in frame_imgs.iter().enumerate() {
        stats.push(frame_stats(f));
        let n = Some(i as u32 + 1);
        let s = stats.last().expect("pushed");

        if wants("alpha") {
            if i == 0 {
                run.push("alpha".into());
            }
            if s.alpha_max == 0 {
                findings.push(Finding::new("GFX-EMPTY-SUBJECT", Level::Error, "the frame has no visible pixel".into(), n, json!({})));
                continue;
            }
            if transparent_profile && s.alpha_min == 255 {
                findings.push(Finding::new(
                    "GFX-NO-ALPHA",
                    Level::Error,
                    "the frame is fully opaque, so the art carries no transparency. A prompt that asks for a transparent background proves nothing; the saved pixels must".into(),
                    n,
                    json!({"alpha_min": s.alpha_min}),
                ));
            }
        } else if s.alpha_max == 0 {
            // an empty frame makes every other per-frame check meaningless
            continue;
        }
        if wants("opaque_canvas") && transparent_profile {
            if i == 0 {
                run.push("opaque_canvas".into());
            }
            if s.corner_alpha.iter().all(|a| *a == 255) {
                findings.push(Finding::new(
                    "GFX-OPAQUE-CANVAS",
                    Level::Error,
                    "all four corners are fully opaque, so the unused canvas has a background".into(),
                    n,
                    json!({"corner_alpha": s.corner_alpha}),
                ));
            }
        }
        if wants("ghost") {
            if i == 0 {
                run.push("ghost".into());
            }
            if s.subject_alpha_mean < 200.0 {
                findings.push(Finding::new(
                    "GFX-GHOSTED",
                    Level::Warning,
                    format!("visible pixels average alpha {:.1}, below 200", s.subject_alpha_mean),
                    n,
                    json!({"subject_alpha_mean": s.subject_alpha_mean}),
                ));
            }
        }
        if wants("fringe") {
            if i == 0 {
                run.push("fringe".into());
            }
            if s.chroma_edge_pixels > 0 {
                findings.push(Finding::new(
                    "GFX-CHROMA-FRINGE",
                    Level::Warning,
                    format!(
                        "{} of {} edge pixels carry the magenta or green key colour; legitimate green or purple art also triggers this",
                        s.chroma_edge_pixels, s.edge_pixels
                    ),
                    n,
                    json!({"chroma_edge_pixels": s.chroma_edge_pixels, "edge_pixels": s.edge_pixels}),
                ));
            }
        }
        // A selected strip's frame 2 is frame 1 with a glow that runs to the frame edge, as vanilla's does (most
        // vanilla faction logos keep 0 or 1 px there), so only frame 1 holds the margin.
        let derived_frame = i > 0 && crate::recipes::frames::selected_fit_margin(profile.family) > 0;
        if wants("margin") && transparent_profile && profile.margin > 0 && !derived_frame {
            if i == 0 {
                run.push("margin".into());
            }
            if let Some(m) = s.margins {
                if m.iter().copied().min().unwrap_or(0) < profile.margin {
                    findings.push(Finding::new(
                        "GFX-MARGIN",
                        Level::Error,
                        format!("margins L/T/R/B are {m:?}, the minimum is {}", profile.margin),
                        n,
                        json!({"margins": m, "minimum": profile.margin}),
                    ));
                }
            }
        }
        if wants("fill") && transparent_profile {
            if i == 0 {
                run.push("fill".into());
            }
            if let Some(b) = s.visible_bounds {
                let span = (f64::from(b[2] - b[0]) / f64::from(f.width)).max(f64::from(b[3] - b[1]) / f64::from(f.height));
                // a selected-strip emblem sits as far inside frame 1 as vanilla's (about 76 percent of an agency
                // frame), so the floor follows that fit, less a little for the subject's own shape. A family fitted
                // with a wide vanilla margin (an MIO trait sits 8 px inside its 48 px canvas) follows its margin too
                let fit = f64::from(crate::recipes::frames::selected_fit_margin(profile.family).max(profile.margin));
                let mut fitted = (1.0 - 2.0 * fit / f64::from(f.width)).max(1.0 - 2.0 * fit / f64::from(f.height));
                // a special project drawing fills vanilla's box (86 by 74 percent of the canvas), so a tall subject
                // spans 74 percent of the height, as vanilla's do
                if profile.family == crate::family::Family::SpecialProjectIcon {
                    let b = crate::recipes::line_art::STYLE.fit_box;
                    fitted = b[0].min(b[1]);
                }
                // a medal sits in vanilla's 33x56 box of its 56x84 frame, spanning two thirds of it
                if profile.family == crate::family::Family::Medal && [f.width, f.height] == [56, 84] {
                    let b = crate::chain::MEDAL_BOX;
                    fitted = (f64::from(b[0]) / 56.0).max(f64::from(b[1]) / 84.0);
                }
                if span < 0.80_f64.min(fitted - 0.05) {
                    findings.push(Finding::new(
                        "GFX-UNDERSIZED-SUBJECT",
                        Level::Warning,
                        format!("the subject spans {:.1} percent of the frame; review undersized artwork", span * 100.0),
                        n,
                        json!({"span": span}),
                    ));
                }
            }
        }
    }

    if wants("opaque") && !transparent_profile {
        run.push("opaque".into());
        let translucent = img.data.chunks_exact(4).filter(|p| p[3] < 255).count();
        if translucent > 0 {
            findings.push(Finding::new(
                "GFX-ALPHA-IN-OPAQUE",
                Level::Error,
                format!(
                    "{translucent} pixels have alpha below 255, but this family is an opaque picture; flatten it so provider alpha never survives"
                ),
                None,
                json!({"translucent_pixels": translucent}),
            ));
        }
    }

    if wants("near_opaque") {
        run.push("near_opaque".into());
        let thin = img.data.chunks_exact(4).filter(|p| p[3] < NEAR_OPAQUE_MIN_ALPHA).count();
        if thin > 0 {
            findings.push(Finding::new(
                "GFX-ALPHA-IN-OPAQUE",
                Level::Error,
                format!("{thin} pixels have alpha below {NEAR_OPAQUE_MIN_ALPHA}, but this family is a picture on a template that is opaque to within one level"),
                None,
                json!({"thin_pixels": thin, "minimum": NEAR_OPAQUE_MIN_ALPHA}),
            ));
        }
    }

    if wants("card_corners") {
        run.push("card_corners".into());
        for (i, f) in frame_imgs.iter().enumerate() {
            let s = frame_stats(f);
            if s.alpha_max == 0 {
                continue;
            }
            if s.corner_alpha.iter().any(|a| *a != 0) {
                findings.push(Finding::new(
                    "GFX-CARD-CORNERS",
                    Level::Error,
                    "a card has fully transparent corners and this one has an opaque or partly opaque corner, so the card treatment did not run or the art is not a card".into(),
                    Some(i as u32 + 1),
                    json!({"corner_alpha": s.corner_alpha}),
                ));
            }
        }
    }

    if wants("animation") {
        if let Some(anim) = &profile.imagegen.animation {
            run.push("animation".into());
            if frame_imgs.len() > 1 {
                findings.extend(animation_findings(&frame_imgs, anim));
            }
        }
    }

    if wants("line_art") {
        run.push("line_art".into());
        let area = crate::recipes::line_art::largest_filled_area(img);
        if area > crate::recipes::line_art::FILLED_AREA_LIMIT {
            findings.push(Finding::new(
                FILLED_CODE,
                Level::Error,
                format!(
                    "a filled area of {area} px: a special project icon is thin pale lines with nothing filled (vanilla's largest is {} px, its median {}). Process the source again so the line_art recipe redraws it",
                    crate::recipes::line_art::VANILLA_FILLED_AREA[1],
                    crate::recipes::line_art::VANILLA_FILLED_AREA[0]
                ),
                None,
                json!({"filled_area": area, "limit": crate::recipes::line_art::FILLED_AREA_LIMIT}),
            ));
        }
    }

    if wants("unit_material") {
        if let Some(kind) = crate::chain::unit_art_kind(profile.family) {
            run.push("unit_material".into());
            let style = crate::recipes::unit_art::GlyphStyle::of(kind);
            let want = [style.frame[0] * frames, style.frame[1]];
            if [img.width, img.height] == want {
                let problems = crate::recipes::unit_art::material_problems(img, kind);
                if !problems.is_empty() {
                    findings.push(Finding::new(
                        UNIT_MATERIAL_CODE,
                        Level::Error,
                        format!(
                            "this is not vanilla's {} unit art: {}. Process it again: the unit art recipe draws any silhouette in vanilla's material",
                            profile.family,
                            problems.join("; ")
                        ),
                        None,
                        json!({"problems": problems}),
                    ));
                }
            }
        }
    }

    if wants("side_view") {
        run.push("side_view".into());
        if let Some(b) = img.alpha_bbox(16) {
            let aspect = f64::from(b[2] - b[0]) / f64::from((b[3] - b[1]).max(1));
            let min = side_view_min_aspect(canvas);
            if aspect < min {
                findings.push(Finding::new(
                    SIDE_VIEW_CODE,
                    Level::Warning,
                    format!(
                        "the subject is {aspect:.2} times as wide as tall; vanilla's technology pictures are side profiles facing right, at least {min:.2} times as wide as tall in 9 of 10 pictures. Check in the review that this is the side view, not a three-quarter or front view"
                    ),
                    None,
                    json!({"aspect": aspect, "vanilla_p10": min}),
                ));
            }
        }
    }

    if wants("checker") {
        run.push("checker".into());
        if let Some(c) = detect_fake_checkerboard(img) {
            findings.push(Finding::new(
                "GFX-FAKE-CHECKERBOARD",
                Level::Error,
                format!(
                    "opaque grey and white squares of {} px look like a drawn transparency checkerboard (match {:.0} percent)",
                    c.block,
                    c.score * 100.0
                ),
                None,
                json!({"block": c.block, "score": c.score, "samples": c.samples}),
            ));
        }
    }

    let status = if findings.iter().any(|f| f.level == Level::Error) { Status::Fail } else { Status::Pass };
    Report { status, findings, frames: stats, size: [img.width, img.height], pixels_sha256: img.pixels_sha256(), checks_run: run }
}

/// The `animation` check: what changes between the frames of an animated strip beyond its intended motion
/// (see [`crate::animation`] for the vanilla measurements behind the limits).
///
/// * `GFX-ANIM-STATIC` (error): the frames do not move at all.
/// * `GFX-ANIM-ALIGN`: a frame's alpha-weighted centroid drifts from frame 1's by more than the motion allows
///   (warning), or by more than twice that (error).
/// * `GFX-ANIM-SILHOUETTE`: a frame's silhouette overlaps frame 1's less than the motion allows.
/// * `GFX-ANIM-PALETTE`: a frame's mean colour moves away from frame 1's.
/// * `GFX-ANIM-SEAM`: the step from the last frame back to the first is much larger than the other steps, so the loop
///   visibly restarts (looping strips of three or more frames).
/// * `GFX-ANIM-JUMP` (warning): one step inside the loop is much larger than the others.
pub fn animation_findings(frames: &[Rgba], anim: &crate::animation::AnimationSpec) -> Vec<Finding> {
    use crate::animation::{frame_metrics, limits, STATIC_STEP};
    let m = frame_metrics(frames);
    let lim = limits(anim.motion);
    let mut out = Vec::new();
    let graded = |v: f64, warn: f64, err: f64, above: bool| -> Option<Level> {
        let (w, e) = if above { (v > warn, v > err) } else { (v < warn, v < err) };
        if e {
            Some(Level::Error)
        } else if w {
            Some(Level::Warning)
        } else {
            None
        }
    };
    if m.steps.iter().all(|s| *s < STATIC_STEP) {
        out.push(Finding::new(
            "GFX-ANIM-STATIC",
            Level::Error,
            format!("the {} frames are the same picture, so nothing moves; an animated sprite needs its motion", frames.len()),
            None,
            json!({"steps": m.steps, "static_below": STATIC_STEP}),
        ));
        return out;
    }
    let (w, h) = (f64::from(frames[0].width), f64::from(frames[0].height));
    for (i, _) in frames.iter().enumerate().skip(1) {
        let n = Some(i as u32 + 1);
        if let Some(share) = lim.drift_share {
            let allowed = (share * (w * w + h * h).sqrt()).max(1.5);
            let d = m.centroid_drift_px[i];
            if let Some(level) = graded(d, allowed, 2.0 * allowed, true) {
                out.push(Finding::new(
                    "GFX-ANIM-ALIGN",
                    level,
                    format!(
                        "frame {} sits {d:.1} px away from frame 1 (allowed {allowed:.1} px for {}): the subject jumps instead of animating in place",
                        i + 1,
                        anim.motion.as_str()
                    ),
                    n,
                    json!({"centroid_drift_px": d, "allowed_px": allowed}),
                ));
            }
        }
        if let Some((warn, err)) = lim.iou {
            let v = m.silhouette_iou[i];
            if let Some(level) = graded(v, warn, err, false) {
                out.push(Finding::new(
                    "GFX-ANIM-SILHOUETTE",
                    level,
                    format!("frame {} shares {:.0} percent of its silhouette with frame 1 (at least {:.0} percent expected): its shape changed beyond the motion", i + 1, v * 100.0, warn * 100.0),
                    n,
                    json!({"silhouette_iou": v, "warn_below": warn, "error_below": err}),
                ));
            }
        }
        if let Some((warn, err)) = lim.palette {
            let v = m.palette_drift[i];
            if let Some(level) = graded(v, warn, err, true) {
                out.push(Finding::new(
                    "GFX-ANIM-PALETTE",
                    level,
                    format!(
                        "frame {}'s mean colour is {v:.1} levels from frame 1's (at most {warn:.0} expected): the palette drifts between frames",
                        i + 1
                    ),
                    n,
                    json!({"palette_drift": v, "warn_above": warn, "error_above": err}),
                ));
            }
        }
    }
    if anim.looping {
        if let Some(r) = m.seam_ratio {
            if let Some(level) = graded(r, lim.seam.0, lim.seam.1, true) {
                out.push(Finding::new(
                    "GFX-ANIM-SEAM",
                    level,
                    format!("the step from the last frame back to frame 1 is {r:.1} times the median step: the loop visibly restarts"),
                    Some(frames.len() as u32),
                    json!({"seam_ratio": r, "steps": m.steps, "warn_above": lim.seam.0, "error_above": lim.seam.1}),
                ));
            }
        }
    }
    if let Some(r) = m.jump_ratio {
        if r > lim.seam.0 {
            let at = m.steps[..m.steps.len() - 1].iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i as u32 + 1).unwrap_or(1);
            out.push(Finding::new(
                "GFX-ANIM-JUMP",
                Level::Warning,
                format!("the step from frame {at} to frame {} is {r:.1} times the median step: the motion jerks there", at + 1),
                Some(at + 1),
                json!({"jump_ratio": r, "steps": m.steps}),
            ));
        }
    }
    out
}

/// The style distribution the `style` check compares an image with. It is built from the family's chosen style
/// source (see `stylesrc`), so it is not part of [`Options`] and `validate_image` never runs it.
#[derive(Debug, Clone)]
pub struct StyleModel {
    /// The drift model.
    pub drift: DriftModel,
    /// Tier of the source, 1 to 4.
    pub tier: u8,
    /// Tier name.
    pub tier_name: String,
    /// What the distribution describes, for the message.
    pub subject: String,
}

/// The `style` check: a candidate far from the family's style distribution gets `GFX-STYLE-DRIFT` at warning level
/// (never an error, so it never blocks an install). Within the distribution it gets the advisory `GFX-STYLE-MATCH`,
/// which exists so the recorded validation carries the numbers either way. `None` for an image with no visible pixel.
pub fn style_check(img: &Rgba, model: &StyleModel) -> Option<Finding> {
    let a = styledesc::analyse(img)?;
    let d = model.drift.check(&a.v);
    let round = |x: f64| (x * 1000.0).round() / 1000.0;
    let worst: Vec<Value> = d.worst.iter().map(|(n, z)| json!({"dimension": n, "z": round(*z)})).collect();
    let evidence = json!({
        "z_rms": round(d.z_rms),
        "threshold": round(d.threshold),
        "p95_leave_one_out": round(model.drift.p95_leave_one_out),
        "images": model.drift.images,
        "tier": model.tier,
        "tier_name": model.tier_name,
        "furthest_dimensions": worst,
    });
    let furthest = d.worst.iter().map(|(n, z)| format!("{n} {z:+.1}")).collect::<Vec<_>>().join(", ");
    Some(if d.drifted {
        Finding::new(
            "GFX-STYLE-DRIFT",
            Level::Warning,
            format!(
                "the image is far from {}: RMS z-score {:.2} against a threshold of {:.2} (furthest: {furthest}). Review it against the references; this never blocks an install",
                model.subject, d.z_rms, d.threshold
            ),
            None,
            evidence,
        )
    } else {
        Finding::new(
            "GFX-STYLE-MATCH",
            Level::Advisory,
            format!("the image is within the style of {}: RMS z-score {:.2}, threshold {:.2}", model.subject, d.z_rms, d.threshold),
            None,
            evidence,
        )
    })
}

/// Measure one frame.
pub fn frame_stats(f: &Rgba) -> FrameStats {
    let mut subject = 0u64;
    let mut sum = 0u64;
    let mut opaque = 0u64;
    let (mut amin, mut amax) = (255u8, 0u8);
    for p in f.data.chunks_exact(4) {
        let a = p[3];
        amin = amin.min(a);
        amax = amax.max(a);
        if a > 0 {
            subject += 1;
            sum += u64::from(a);
        }
        if a == 255 {
            opaque += 1;
        }
    }
    let bounds = f.alpha_bbox(16);
    let margins = bounds.map(|b| [b[0], b[1], f.width - b[2], f.height - b[3]]);
    let (edge, chroma) = edge_stats(f);
    let (w, h) = (f.width - 1, f.height - 1);
    FrameStats {
        subject_alpha_mean: if subject == 0 { 0.0 } else { (sum as f64 / subject as f64 * 1000.0).round() / 1000.0 },
        alpha_min: amin,
        alpha_max: amax,
        subject_pixels: subject,
        opaque_pixels: opaque,
        visible_bounds: bounds,
        margins,
        corner_alpha: [f.alpha(0, 0), f.alpha(w, 0), f.alpha(0, h), f.alpha(w, h)],
        edge_pixels: edge,
        chroma_edge_pixels: chroma,
    }
}

/// Evidence for a drawn checkerboard.
#[derive(Debug, Clone, PartialEq)]
pub struct CheckerEvidence {
    /// Square size in pixels.
    pub block: u32,
    /// Fraction of sampled pixels that fit the pattern.
    pub score: f64,
    /// Pixels sampled.
    pub samples: usize,
}

/// Look for the pale grey and white squares that image generators draw when asked for "transparency".
///
/// Only opaque, neutral, light pixels take part. They must split into two close brightness classes that
/// follow a square grid of some size (tried from 2 px up to a third of the smaller image side) at any phase.
pub fn detect_fake_checkerboard(img: &Rgba) -> Option<CheckerEvidence> {
    let (w, h) = (img.width as usize, img.height as usize);
    let mut cand: Vec<(u32, u32, u8)> = Vec::new();
    let mut opaque = 0usize;
    for y in 0..h {
        for x in 0..w {
            let p = img.px(x as u32, y as u32);
            if p[3] < 250 {
                continue;
            }
            opaque += 1;
            let (mx, mn) = (p[0].max(p[1]).max(p[2]), p[0].min(p[1]).min(p[2]));
            let lum = ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114) / 1000) as u8;
            if mx - mn <= 14 && lum >= 150 {
                cand.push((x as u32, y as u32, lum));
            }
        }
    }
    if cand.len() < 400 || opaque == 0 || (cand.len() as f64) < 0.25 * opaque as f64 {
        return None;
    }
    let step = cand.len() / 4096 + 1;
    let sample: Vec<(u32, u32, u8)> = cand.iter().copied().step_by(step).collect();
    let (lo, hi) = sample.iter().fold((255u8, 0u8), |(lo, hi), s| (lo.min(s.2), hi.max(s.2)));
    if hi - lo < 8 || hi - lo > 90 {
        return None;
    }
    let threshold = (u16::from(lo) + u16::from(hi)) / 2;
    let classes: Vec<bool> = sample.iter().map(|s| u16::from(s.2) >= threshold).collect();
    let high = classes.iter().filter(|c| **c).count();
    let n = sample.len();
    if high * 5 < n || (n - high) * 5 < n {
        return None;
    }
    let max_block = (w.min(h) / 3).min(24) as u32;
    let mut best: Option<CheckerEvidence> = None;
    for s in 2..=max_block {
        for ox in 0..s {
            for oy in 0..s {
                let mut agree = 0usize;
                for (i, (x, y, _)) in sample.iter().enumerate() {
                    let parity = (((x + ox) / s + (y + oy) / s) & 1) == 1;
                    if parity == classes[i] {
                        agree += 1;
                    }
                }
                let score = agree.max(n - agree) as f64 / n as f64;
                if best.as_ref().map(|b| score > b.score).unwrap_or(true) {
                    best = Some(CheckerEvidence { block: s, score, samples: n });
                }
            }
        }
    }
    best.filter(|b| b.score >= 0.93)
}

/// Container-level checks on file bytes (DDS, TGA, LFS pointer, truncation).
pub fn validate_container(bytes: &[u8], file_name: &str, profile: &Profile, checks: &[String]) -> Vec<Finding> {
    let wants = |name: &str| checks.iter().any(|c| c == name);
    let mut out = Vec::new();
    if bytes.starts_with(b"version https://git-lfs") {
        out.push(Finding::new(
            "GFX-LFS-POINTER",
            Level::Error,
            "the file is a Git LFS pointer, not the binary; run git lfs checkout for this path".into(),
            None,
            json!({}),
        ));
        return out;
    }
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".dds") {
        if bytes.len() < dds::HEADER_LEN {
            out.push(Finding::new("GFX-TRUNCATED", Level::Error, "the file has no complete DDS header".into(), None, json!({"bytes": bytes.len()})));
            return out;
        }
        if wants("dds") {
            for problem in dds::canonical_problems(bytes) {
                let code = if problem.contains("canonical length") { "GFX-TRUNCATED" } else { "GFX-DDS-HEADER" };
                out.push(Finding::new(
                    code,
                    Level::Error,
                    format!("DDS is not the canonical one-level uncompressed BGRA layout: {problem}"),
                    None,
                    json!({}),
                ));
            }
        }
    } else if lower.ends_with(".tga") {
        match tga::parse_header(bytes) {
            Err(_) => out.push(Finding::new(
                "GFX-TRUNCATED",
                Level::Error,
                "the file has no complete TGA header".into(),
                None,
                json!({"bytes": bytes.len()}),
            )),
            Ok(info) => {
                if wants("tga") {
                    if info.image_type != 2 || info.bits_per_pixel != 32 {
                        out.push(Finding::new(
                            "GFX-TGA-FORMAT",
                            Level::Warning,
                            format!(
                                "TGA is type {} at {} bits per pixel; flags written by this tool are uncompressed 32-bit",
                                info.image_type, info.bits_per_pixel
                            ),
                            None,
                            json!({}),
                        ));
                    }
                    if info.top_origin && profile.family == crate::family::Family::Flag {
                        out.push(Finding::new(
                            "GFX-FLAG-ORIGIN",
                            Level::Warning,
                            "the flag is stored top-origin; the kit's flag rule is bottom-origin (vanilla ships a few top-origin files, so the engine loads them)".into(),
                            None,
                            json!({}),
                        ));
                    }
                    let need = tga::HEADER_LEN + info.id_len + (info.width * info.height * u32::from(info.bits_per_pixel / 8)) as usize;
                    if bytes.len() < need {
                        out.push(Finding::new(
                            "GFX-TRUNCATED",
                            Level::Error,
                            format!("the file holds {} bytes, its TGA header needs at least {need}", bytes.len()),
                            None,
                            json!({}),
                        ));
                    }
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::family::Family;
    use crate::profile::ProfileSet;

    fn profile(c: Family) -> Profile {
        ProfileSet::builtin().for_family(c).unwrap().clone()
    }

    fn checks(p: &Profile) -> Vec<String> {
        p.imagegen.checks.clone()
    }

    fn good_focus() -> Rgba {
        let mut img = Rgba::new(100, 88);
        for y in 4..84 {
            for x in 6..94 {
                let dx = f64::from(x) - 50.0;
                let dy = (f64::from(y) - 44.0) * 1.1;
                if dx * dx + dy * dy < 1800.0 {
                    img.set_px(x, y, [180, 120, 60, 255]);
                }
            }
        }
        img
    }

    fn codes(r: &Report) -> Vec<&str> {
        r.findings.iter().map(|f| f.code.as_str()).collect()
    }

    #[test]
    fn clean_focus_icon_passes() {
        let p = profile(Family::Focus);
        let r = validate_image(&good_focus(), &p, &checks(&p), &Options::default());
        assert_eq!(r.status, Status::Pass, "{:?}", r.findings);
        assert!(r.checks_run.contains(&"margin".to_string()));
        assert_eq!(r.frames.len(), 1);
    }

    #[test]
    fn wrong_size_is_an_error_with_both_sizes_named() {
        let p = profile(Family::Focus);
        let r = validate_image(&Rgba::new(94, 86), &p, &checks(&p), &Options::default());
        assert_eq!(r.status, Status::Fail);
        let f = r.findings.iter().find(|f| f.code == "GFX-CANVAS-SIZE").unwrap();
        assert!(f.message.contains("94x86") && f.message.contains("100x88"), "{}", f.message);
    }

    #[test]
    fn opaque_canvas_and_missing_alpha_are_caught() {
        let p = profile(Family::Focus);
        let r = validate_image(&Rgba::filled(100, 88, [10, 10, 10, 255]), &p, &checks(&p), &Options::default());
        let c = codes(&r);
        assert!(c.contains(&"GFX-OPAQUE-CANVAS"), "{c:?}");
        assert!(c.contains(&"GFX-NO-ALPHA"), "{c:?}");
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn empty_subject_ghosting_margin_and_fill() {
        let p = profile(Family::Focus);
        let r = validate_image(&Rgba::new(100, 88), &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-EMPTY-SUBJECT"));

        let mut ghost = good_focus();
        for px in ghost.data.chunks_exact_mut(4) {
            if px[3] > 0 {
                px[3] = 120;
            }
        }
        let r = validate_image(&ghost, &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-GHOSTED"));
        assert_eq!(r.status, Status::Pass, "ghosting is a warning");

        let mut touching = good_focus();
        touching.set_px(0, 40, [1, 2, 3, 255]);
        let r = validate_image(&touching, &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-MARGIN"));

        let mut tiny = Rgba::new(100, 88);
        for y in 40..50 {
            for x in 45..55 {
                tiny.set_px(x, y, [1, 2, 3, 255]);
            }
        }
        let r = validate_image(&tiny, &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-UNDERSIZED-SUBJECT"));
    }

    #[test]
    fn chroma_fringe_is_reported() {
        let p = profile(Family::Focus);
        let mut img = good_focus();
        // recolour the rim of the disc magenta
        let b = img.alpha_bbox(1).unwrap();
        for x in b[0]..b[2] {
            img.set_px(x, b[1] + 8, [255, 0, 255, 255]);
        }
        let (_, chroma) = edge_stats(&img);
        let r = validate_image(&img, &p, &checks(&p), &Options::default());
        assert_eq!(codes(&r).contains(&"GFX-CHROMA-FRINGE"), chroma > 0);
    }

    #[test]
    fn opaque_pictures_need_only_size_and_no_checker() {
        let p = profile(Family::NewsEvent);
        let r = validate_image(&Rgba::filled(397, 153, [90, 90, 90, 255]), &p, &checks(&p), &Options::default());
        assert_eq!(r.status, Status::Pass, "{:?}", r.findings);
        let r = validate_image(&Rgba::filled(395, 153, [90, 90, 90, 255]), &p, &checks(&p), &Options::default());
        assert_eq!(r.status, Status::Fail);
    }

    #[test]
    fn frames_must_divide_and_each_frame_is_measured() {
        let p = profile(Family::CounterLarge);
        let mut strip = Rgba::new(152, 42);
        for x in 5..70 {
            for y in 3..39 {
                strip.set_px(x, y, [200, 30, 30, 255]);
            }
        }
        // second frame empty
        let r = validate_image(&strip, &p, &checks(&p), &Options::default());
        assert!(r.findings.iter().any(|f| f.code == "GFX-EMPTY-SUBJECT" && f.frame == Some(2)), "{:?}", r.findings);
        assert_eq!(r.frames.len(), 2);
        let r = validate_image(&Rgba::new(151, 42), &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-FRAME-WIDTH"));
    }

    #[test]
    fn canvas_override_for_custom_profiles() {
        let p = profile(Family::GuiTexture);
        let r = validate_image(&Rgba::new(10, 10), &p, &checks(&p), &Options::default());
        assert!(codes(&r).contains(&"GFX-CANVAS-UNSPECIFIED"));
        let img = Rgba::filled(64, 32, [1, 2, 3, 255]);
        let r = validate_image(&img, &p, &["size".to_string()], &Options { canvas: Some([64, 32]) });
        assert_eq!(r.status, Status::Pass);
        let r = validate_image(&img, &p, &["size".to_string()], &Options { canvas: Some([64, 33]) });
        assert_eq!(r.status, Status::Fail);
    }

    fn checker(w: u32, h: u32, block: u32, a: u8, b: u8, offset: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let v = if ((x + offset) / block + (y + offset) / block).is_multiple_of(2) { a } else { b };
                img.set_px(x, y, [v, v, v, 255]);
            }
        }
        img
    }

    #[test]
    fn card_corners_and_near_opaque_are_their_own_checks() {
        let set = crate::profile::ProfileSet::builtin();
        let report = set.for_family(crate::family::Family::ReportEvent).unwrap();
        let checks = report.imagegen.checks.clone();
        let mut card = Rgba::filled(210, 176, [90, 80, 70, 255]);
        for (x, y) in [(0, 0), (209, 0), (0, 175), (209, 175)] {
            card.set_px(x, y, [0, 0, 0, 0]);
        }
        let ok = validate_image(&card, report, &checks, &Options::default());
        assert_eq!(ok.status, Status::Pass, "{:?}", ok.findings);
        assert!(ok.checks_run.iter().any(|c| c == "card_corners"));
        // an opaque picture is not a card
        let flat = validate_image(&Rgba::filled(210, 176, [90, 80, 70, 255]), report, &checks, &Options::default());
        assert!(flat.findings.iter().any(|f| f.code == "GFX-CARD-CORNERS"), "{:?}", flat.findings);
        // and a card is not asked to be opaque
        assert!(!ok.findings.iter().any(|f| f.code == "GFX-ALPHA-IN-OPAQUE"));

        let ach = set.for_family(crate::family::Family::Achievement).unwrap();
        let near = validate_image(&Rgba::filled(64, 64, [20, 20, 20, 254]), ach, &ach.imagegen.checks, &Options::default());
        assert_eq!(near.status, Status::Pass, "alpha 254 is the templates' own alpha: {:?}", near.findings);
        let mut thin = Rgba::filled(64, 64, [20, 20, 20, 255]);
        thin.set_px(3, 3, [20, 20, 20, 90]);
        let bad = validate_image(&thin, ach, &ach.imagegen.checks, &Options::default());
        assert!(bad.findings.iter().any(|f| f.code == "GFX-ALPHA-IN-OPAQUE" && f.level == Level::Error));
    }

    #[test]
    fn fake_checkerboard_is_detected_at_any_phase_and_plain_art_is_not() {
        for (block, offset) in [(8, 0), (8, 3), (4, 1), (10, 7)] {
            let img = checker(96, 96, block, 255, 204, offset);
            let ev = detect_fake_checkerboard(&img).unwrap_or_else(|| panic!("missed block {block} offset {offset}"));
            assert!(ev.score >= 0.93);
        }
        // negatives
        assert!(detect_fake_checkerboard(&Rgba::filled(96, 96, [255, 255, 255, 255])).is_none());
        let mut gradient = Rgba::new(96, 96);
        for y in 0..96 {
            for x in 0..96 {
                let v = 150 + ((x + y) * 100 / 190) as u8;
                gradient.set_px(x, y, [v, v, v, 255]);
            }
        }
        assert!(detect_fake_checkerboard(&gradient).is_none());
        let mut halves = Rgba::filled(96, 96, [250, 250, 250, 255]);
        for y in 0..96 {
            for x in 48..96 {
                halves.set_px(x, y, [200, 200, 200, 255]);
            }
        }
        assert!(detect_fake_checkerboard(&halves).is_none(), "two flat halves are not a checkerboard");
        // transparent art is never a fake checkerboard
        assert!(detect_fake_checkerboard(&good_focus()).is_none());
    }

    #[test]
    fn container_checks_flag_bad_dds_tga_and_lfs() {
        let p = profile(Family::Focus);
        let good = dds::write_a8r8g8b8(&good_focus());
        assert!(validate_container(&good, "a.dds", &p, &checks(&p)).is_empty());
        let mut cut = good.clone();
        cut.truncate(200);
        assert!(validate_container(&cut, "a.dds", &p, &checks(&p)).iter().any(|f| f.code == "GFX-TRUNCATED"));
        assert!(validate_container(&good[..50], "a.dds", &p, &checks(&p)).iter().any(|f| f.code == "GFX-TRUNCATED"));
        let lfs = b"version https://git-lfs.github.com/spec/v1\noid sha256:abc\nsize 1\n";
        assert!(validate_container(lfs, "a.dds", &p, &checks(&p)).iter().any(|f| f.code == "GFX-LFS-POINTER"));
        let flag = profile(Family::Flag);
        let top = tga::write_truecolor32(&Rgba::filled(82, 52, [1, 2, 3, 255]), tga::Origin::TopLeft);
        let fc = vec!["tga".to_string()];
        assert!(validate_container(&top, "GER.tga", &flag, &fc).iter().any(|f| f.code == "GFX-FLAG-ORIGIN"));
        let bottom = tga::write_truecolor32(&Rgba::filled(82, 52, [1, 2, 3, 255]), tga::Origin::BottomLeft);
        assert!(validate_container(&bottom, "GER.tga", &flag, &fc).is_empty());
    }
}
