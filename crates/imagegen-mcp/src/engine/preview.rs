//! `hoi4.image_preview`: a contact sheet for looking at the art the way the game will show it.
//!
//! Panels, left to right and wrapping: native size on a dark background, native size on a light one, then an
//! enlarged copy (nearest neighbour, so pixels stay visible) on dark, light and checker backgrounds, then up to
//! three of the plan's references scaled to the same height. A reviewer must look at the native-size panels:
//! an enlarged image can look fine and still be unreadable at game size.
//!
//! An animated strip also gets a frame sheet (every frame side by side, enlarged) and a looping GIF of the
//! animation as the game plays it, both next to the contact sheet and recorded with it.
//!
//! An artifact also gets the vanilla side-by-side sheet (`vanilla.rs`): the candidate beside the family's most
//! typical vanilla images at the same sizes, with the measured vanilla-match result. The contact sheet is written into
//! the mod and holds no vanilla art; the vanilla sheet holds vanilla pixels and is written outside the mod. An
//! approval needs a preview of the exact file that included the vanilla sheet.

use imagegen_core::error::RetryClass;
use imagegen_core::fsio::ProjectFs;
use imagegen_core::hashing::sha256_hex;
use imagegen_core::naming::ArtifactId;
use imagegen_core::pixels::{decode, encode_png, DecodeLimits, Rgba};
use imagegen_core::records::{kind, CandidateRecord};
use imagegen_core::refs::{load_reference, ReferenceInput};
use imagegen_core::resize::resize_lanczos3;
use serde_json::{json, Value};

use crate::api::PreviewParams;

use super::{EResult, Engine, EngineError};

const PAD: u32 = 8;
const MAX_ROW: u32 = 1800;
const DARK: [u8; 4] = [32, 32, 36, 255];
const LIGHT: [u8; 4] = [232, 232, 226, 255];

/// A rendered contact sheet and what is on it.
pub struct Sheet {
    /// PNG bytes.
    pub png: Vec<u8>,
    /// Panel names in order.
    pub panels: Vec<String>,
    /// Pixel size of the sheet.
    pub size: [u32; 2],
}

fn checker(w: u32, h: u32) -> Rgba {
    let mut img = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let v = if (x / 8 + y / 8) % 2 == 0 { 190 } else { 245 };
            img.set_px(x, y, [v, v, v, 255]);
        }
    }
    img
}

pub(crate) fn upscale_nearest(img: &Rgba, s: u32) -> Rgba {
    let mut out = Rgba::new(img.width * s, img.height * s);
    for y in 0..out.height {
        for x in 0..out.width {
            out.set_px(x, y, img.px(x / s, y / s));
        }
    }
    out
}

pub(crate) fn panel(content: &Rgba, bg: &str) -> Rgba {
    let (w, h) = (content.width + 2 * PAD, content.height + 2 * PAD);
    let mut p = match bg {
        "dark" => Rgba::filled(w, h, DARK),
        "light" => Rgba::filled(w, h, LIGHT),
        _ => checker(w, h),
    };
    p.paste_over(content, i64::from(PAD), i64::from(PAD));
    p
}

/// Compose the sheet. Panels flow left to right and wrap before they exceed the row width. `companions` are the
/// other images an install writes with the main one (the achievement states, the smaller flag sizes): each gets a
/// native panel so the reviewer sees every file that will be written.
pub fn compose(img: &Rgba, references: &[Rgba], companions: &[(String, Rgba)]) -> imagegen_core::Result<Sheet> {
    let longest = img.width.max(img.height).max(1);
    let scale = (512 / longest).clamp(1, 6);
    let mut panels: Vec<(String, Rgba)> = vec![("native on dark".into(), panel(img, "dark")), ("native on light".into(), panel(img, "light"))];
    if scale > 1 {
        let big = upscale_nearest(img, scale);
        panels.push((format!("{scale}x on dark"), panel(&big, "dark")));
        panels.push((format!("{scale}x on light"), panel(&big, "light")));
        panels.push((format!("{scale}x on checker"), panel(&big, "checker")));
    } else {
        panels.push(("on checker".into(), panel(img, "checker")));
    }
    for (key, c) in companions {
        panels.push((format!("native {} on dark", key.trim_start_matches('_')), panel(c, "dark")));
    }
    let ref_height = (img.height * scale).clamp(32, 256);
    for (i, r) in references.iter().take(3).enumerate() {
        let w = ((f64::from(r.width) * f64::from(ref_height) / f64::from(r.height.max(1))).round() as u32).clamp(1, 512);
        let scaled = resize_lanczos3(r, w, ref_height)?;
        panels.push((format!("reference {}", i + 1), panel(&scaled, "dark")));
    }
    // flow layout
    let mut placements = Vec::new();
    let (mut x, mut y, mut row_h, mut sheet_w) = (0u32, 0u32, 0u32, 0u32);
    for (_, p) in &panels {
        if x > 0 && x + p.width > MAX_ROW {
            x = 0;
            y += row_h;
            row_h = 0;
        }
        placements.push((x, y));
        x += p.width;
        row_h = row_h.max(p.height);
        sheet_w = sheet_w.max(x);
    }
    let sheet_h = y + row_h;
    let mut sheet = Rgba::filled(sheet_w, sheet_h, [60, 60, 66, 255]);
    for ((_, p), (px, py)) in panels.iter().zip(&placements) {
        sheet.paste(p, i64::from(*px), i64::from(*py));
    }
    Ok(Sheet { png: encode_png(&sheet)?, panels: panels.into_iter().map(|(n, _)| n).collect(), size: [sheet.width, sheet.height] })
}

impl Engine {
    /// Write a contact sheet and record it. Returns the report and the sheets as PNG bytes: the contact sheet first,
    /// then the vanilla side-by-side sheet when there is one.
    pub fn preview_with_image(&self, p: &PreviewParams) -> EResult<(Value, Vec<Vec<u8>>)> {
        let mut companion_imgs: Vec<(String, Rgba)> = Vec::new();
        let (img, rel_source, sha, artifact, refs_dir) = if super::looks_like_artifact(&p.target) {
            let a = self.artifact(&p.target)?;
            let (bytes, sha) = self.read_artifact(&a)?;
            let (img, _) = decode(&bytes, Some(&a.path), DecodeLimits::default())?;
            for (c, data) in self.read_companions(&a)? {
                companion_imgs.push((c.key.clone(), decode(&data, Some(&c.path), DecodeLimits::default())?.0));
            }
            let dir = format!("docs/assets/{}/contact_sheets", a.feature_slug);
            (img, a.path.clone(), sha, Some(a), dir)
        } else {
            let bytes = self.fs.read(&p.target, 64 * 1024 * 1024)?;
            let (img, _) = decode(&bytes, Some(&p.target), DecodeLimits::default())?;
            let sha = sha256_hex(&bytes);
            (img, p.target.clone(), sha, None, ".hoi4-imagegen/previews".to_string())
        };
        let refs = match &artifact {
            Some(a) => self.root_references(&a.root_id),
            None => Vec::new(),
        };
        let sheet = compose(&img, &refs, &companion_imgs).map_err(EngineError::from)?;
        let stem = match &artifact {
            Some(a) => format!("{}-preview", a.id.replace('.', "_")),
            None => format!("{}-preview", rel_source.rsplit('/').next().unwrap_or("file").split('.').next().unwrap_or("file")),
        };
        // artifact ids contain a dot; keep file names friendly
        let stem = naming_safe(&stem);
        let written = self.fs.write_next_version(&refs_dir, &stem, "png", 0, &sheet.png)?;
        let animation = match &artifact {
            Some(a) => self.animation_preview(a, &img, &refs_dir, &stem)?,
            None => Value::Null,
        };
        // the vanilla side-by-side sheet and the measured check, for artifacts (their family is known)
        let (vanilla_sheet, vanilla_png, vanilla_check) = match &artifact {
            Some(a) => {
                let check = self.vanilla_check(a.family, &img);
                match self.vanilla_side_by_side(a.family, &img, &stem) {
                    Ok((report, png)) => (report, Some(png), check),
                    Err(reason) => (json!({"unavailable": reason}), None, check),
                }
            }
            None => (Value::Null, None, Value::Null),
        };
        if let Some(a) = &artifact {
            self.ledger.append(
                kind::PREVIEWED,
                Some(&a.asset_id),
                Some(&a.id),
                json!({"target": a.id, "sha256": sha, "sheet": written.rel, "sheet_sha256": sha256_hex(&sheet.png), "panels": sheet.panels, "animation": animation, "vanilla_sheet": vanilla_sheet, "vanilla_check": vanilla_check}),
            )?;
        }
        let report = json!({
            "target": p.target,
            "sha256": sha,
            "sheet": written.rel,
            "sheet_sha256": sha256_hex(&sheet.png),
            "sheet_size": sheet.size,
            "panels": sheet.panels,
            "how_to_look": "Judge the 'native' panels first: that is the size the game draws. Then put it beside the vanilla side-by-side sheet: same form, medium, palette and framing as vanilla? Record the verdict with hoi4.image_review, which runs the measured vanilla-match check itself.",
        });
        let mut report = report;
        if !animation.is_null() {
            report["animation"] = animation;
            report["how_to_look"] = json!("Judge the 'native' panels first: that is the size the game draws. Then play the loop GIF to see the motion as the game plays it (the frame sheet shows each frame enlarged), and compare against the vanilla side-by-side sheet. Record the verdict with hoi4.image_review.");
        }
        if artifact.is_some() {
            report["vanilla_sheet"] = vanilla_sheet;
            report["vanilla_check"] = vanilla_check;
        }
        let mut images = vec![sheet.png];
        images.extend(vanilla_png);
        Ok((report, images))
    }

    /// The frame sheet and loop GIF of an animated artifact (`Value::Null` for any other). Only a whole strip is
    /// previewed as an animation: a raw candidate is one base frame.
    fn animation_preview(&self, a: &super::Artifact, img: &Rgba, dir: &str, stem: &str) -> EResult<Value> {
        let profile = self.profile_of(a)?;
        let Some(anim) = profile.imagegen.animation.clone() else { return Ok(Value::Null) };
        let frames = profile.frames.max(1);
        if !a.processed || frames < 2 || !img.width.is_multiple_of(frames) {
            return Ok(Value::Null);
        }
        let split = img.split_frames(frames)?;
        let longest = split[0].width.max(split[0].height).max(1);
        let scale = (256 / longest).clamp(1, 8);
        let matte = [DARK[0], DARK[1], DARK[2]];
        let sheet = imagegen_core::animation::contact_sheet(&split, matte, scale, PAD)?;
        let sheet_png = encode_png(&sheet)?;
        let sheet_file = self.fs.write_next_version(dir, &format!("{stem}-frames"), "png", 0, &sheet_png)?;
        let looped = imagegen_core::animation::preview_loop(&split, &anim)?;
        let gif = imagegen_core::animation::encode_gif(&looped, matte, scale)?;
        let gif_file = self.fs.write_next_version(dir, &format!("{stem}-loop"), "gif", 0, &gif)?;
        let metrics = imagegen_core::animation::frame_metrics(&split);
        Ok(json!({
            "frames": frames,
            "fps": anim.fps,
            "motion": anim.motion,
            "effect_file": anim.effect_file,
            "frame_sheet": sheet_file.rel,
            "frame_sheet_sha256": sha256_hex(&sheet_png),
            "loop_gif": gif_file.rel,
            "loop_gif_sha256": sha256_hex(&gif),
            "loop_gif_pictures": looped.len(),
            "scale": scale,
            "metrics": metrics,
            "note": if anim.blends() { "the engine cross-fades the frames; the GIF approximates that with linear blends at 20 pictures a second" } else { "each frame is shown for 1 / fps seconds" },
        }))
    }

    /// Preview without the image bytes (CLI and plain callers).
    pub fn preview(&self, p: PreviewParams) -> EResult<Value> {
        Ok(self.preview_with_image(&p)?.0)
    }

    /// The references of the plan an artifact came from, as pixels (best effort; missing files are skipped).
    fn root_references(&self, root_id: &str) -> Vec<Rgba> {
        let Ok(entries) = self.ledger.entries() else { return Vec::new() };
        let Some(rec) = entries
            .iter()
            .filter(|e| matches!(e.kind.as_str(), kind::GENERATED | kind::EDITED | kind::IMPORTED))
            .find(|e| e.artifact_id.as_deref() == Some(root_id))
            .and_then(|e| serde_json::from_value::<CandidateRecord>(e.data.clone()).ok())
        else {
            return Vec::new();
        };
        load_refs(&self.fs, self.game_dir().as_deref(), &rec)
    }
}

/// References shown beside a candidate on its contact sheet. Paradox art is left out: the sheet is written into the
/// mod, and the tool never copies vanilla art there.
fn load_refs(fs: &ProjectFs, game: Option<&std::path::Path>, rec: &CandidateRecord) -> Vec<Rgba> {
    rec.references
        .iter()
        .filter(|r| r.role != imagegen_core::refs::RefRole::EditTarget && !r.origin.is_vanilla())
        .filter_map(|r| load_reference(fs, game, &ReferenceInput { path: r.path.clone(), role: r.role }).ok())
        .filter_map(|l| decode(&l.png, None, DecodeLimits::default()).ok().map(|(i, _)| i))
        .collect()
}

fn naming_safe(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' { c } else { '_' }).collect()
}

#[allow(dead_code)]
fn _unused(_: RetryClass, _: ArtifactId) {}

#[cfg(test)]
mod tests {
    use super::*;

    fn icon() -> Rgba {
        let mut img = Rgba::new(33, 32);
        for y in 4..28 {
            for x in 5..28 {
                img.set_px(x, y, [200, 60, 40, 255]);
            }
        }
        img
    }

    #[test]
    fn sheet_has_native_and_enlarged_panels_on_three_backgrounds() {
        let s = compose(&icon(), &[], &[]).unwrap();
        // 512 / 33 = 15, clamped to the 6x ceiling
        assert_eq!(s.panels, vec!["native on dark", "native on light", "6x on dark", "6x on light", "6x on checker"]);
        let (img, _) = decode(&s.png, None, DecodeLimits::default()).unwrap();
        assert_eq!([img.width, img.height], s.size);
        assert!(img.width <= MAX_ROW + 400);
    }

    #[test]
    fn native_panel_keeps_the_exact_pixels() {
        let icon = icon();
        let s = compose(&icon, &[], &[]).unwrap();
        let (sheet, _) = decode(&s.png, None, DecodeLimits::default()).unwrap();
        // first panel: PAD offset, exact pixels
        assert_eq!(sheet.px(PAD + 10, PAD + 10), icon.px(10, 10));
        assert_eq!(sheet.px(PAD, PAD), DARK, "transparent pixels show the dark background");
    }

    #[test]
    fn references_are_added_and_large_images_wrap() {
        let reference = Rgba::filled(100, 88, [30, 90, 30, 255]);
        let s =
            compose(&Rgba::filled(397, 153, [90, 90, 90, 255]), &[reference.clone(), reference.clone(), reference.clone(), reference], &[]).unwrap();
        assert_eq!(s.panels.iter().filter(|p| p.starts_with("reference")).count(), 3, "at most three references");
        assert!(s.size[0] <= MAX_ROW + 520, "{:?}", s.size);
        assert!(s.size[1] > 153 * 2, "wide panels wrap onto a second row: {:?}", s.size);
    }

    #[test]
    fn one_pixel_images_do_not_panic() {
        assert!(compose(&Rgba::filled(1, 1, [1, 2, 3, 255]), &[], &[]).is_ok());
    }
}
