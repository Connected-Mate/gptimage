//! Report event card: a tilted sepia photo with a soft drop shadow on a transparent 210x176 canvas.
//!
//! Port of the kit's `process_report_event_image.py` with the shadow fixed: the card alpha is padded by
//! `ceil(3 * blur) + 1` before the Gaussian blur and the shadow goes through a full-canvas layer, so nothing is
//! clipped at the card's bounding box. The defaults are the measured ones (angle 4.0, card 192x154, shadow offset
//! (0, 1), blur sigma 2.5, opacity 0.96); every number lives in [`ReportCardParams`] so they can be adjusted.
//!
//! Steps, in order: cover-crop the source to the photo size, autocontrast (cutoff 1), contrast 1.10,
//! brightness 0.98, greyscale, colorize `#282018` to `#eee2c7`, seeded grain, optional paper border, rotation with
//! supersampling and a soft alpha edge, shadow, centre on the canvas.
//!
//! The grain uses the same Mersenne Twister and `randint` as the Python tool, so for the same seed the noise is
//! identical.

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::pillow::{
    alpha_composite, alpha_plane, autocontrast_rgb, colorize_lut, enhance_brightness, enhance_contrast, gaussian_blur_plane, luma, py_round,
    rotate_bicubic_expand, PyRandom,
};
use crate::resize::resize_lanczos3;

/// Version of the recipe, mirroring the `recipe_version` the fixed Python tool writes into provenance.
pub const RECIPE_VERSION: u32 = 2;

/// Every tunable of the report card.
#[derive(Debug, Clone, PartialEq)]
pub struct ReportCardParams {
    /// Output canvas (vanilla report pictures are 210x176).
    pub canvas_size: (u32, u32),
    /// Card size before rotation, border included.
    pub card_size: (u32, u32),
    /// Paper border in pixels on every side (0 for none).
    pub border: u32,
    /// Counter-clockwise tilt in degrees.
    pub angle: f64,
    /// Shadow offset in pixels, x then y.
    pub shadow_offset: (i32, i32),
    /// Gaussian sigma of the shadow.
    pub shadow_blur: f64,
    /// Shadow opacity from 0 to 1.
    pub shadow_opacity: f64,
    /// Grain amplitude: noise is uniform in `-grain..=grain` per pixel, the same for R, G and B.
    pub grain: i32,
    /// Paper texture amplitude, used only with a border.
    pub paper_grain: i32,
    /// Grain seed (`random.Random(seed)`); the paper uses `seed + 1000`.
    pub seed: i64,
    /// Supersampling factor for the rotation.
    pub rotate_supersample: u32,
    /// Gaussian sigma applied to the card's alpha after rotation.
    pub edge_soften: f64,
    /// Where the card sits, in pixels from the canvas centre (x right, y down).
    pub card_offset: (i64, i64),
}

impl ReportCardParams {
    /// The operation phase picture: the same card, tilted the other way and lower.
    ///
    /// Vanilla's 71 phase pictures (`gfx/interface/operations/images/phase_*.dds`, HOI4 1.19.4.1) use four alpha masks.
    /// 46 share one that is not a report card's: the card is tilted clockwise (top-left corner highest, at x 16 y 12;
    /// right edge at y 25) and its box runs from (7, 11) to (205, 173) instead of (3, 4) to (206, 171). A rotated
    /// rectangle fits it best at the report card's size, 3.5 degrees clockwise, centred 2 px right and 4 px down of
    /// the report card (IoU 0.972, the same fit quality as the report card's own mask). 15 use a smaller
    /// counter-clockwise card, and 10 use report card masks unchanged. This preset draws the majority mask.
    pub fn phase() -> ReportCardParams {
        ReportCardParams { angle: -3.5, card_offset: (2, 4), ..Default::default() }
    }
}

impl Default for ReportCardParams {
    fn default() -> Self {
        ReportCardParams {
            canvas_size: (210, 176),
            card_size: (192, 154),
            border: 0,
            angle: 4.0,
            shadow_offset: (0, 1),
            shadow_blur: 2.5,
            shadow_opacity: 0.96,
            grain: 7,
            paper_grain: 0,
            seed: 1337,
            rotate_supersample: 4,
            edge_soften: 0.35,
            card_offset: (0, 0),
        }
    }
}

/// Resize so the image covers `target`, then crop the centre (the kit's `cover_crop`).
pub fn cover_crop(img: &Rgba, target: (u32, u32)) -> Result<Rgba> {
    let (tw, th) = (f64::from(target.0), f64::from(target.1));
    let (sw, sh) = (f64::from(img.width), f64::from(img.height));
    let scale = (tw / sw).max(th / sh);
    let rw = (py_round(sw * scale).max(1)) as u32;
    let rh = (py_round(sh * scale).max(1)) as u32;
    let resized = resize_lanczos3(img, rw, rh)?;
    let left = (rw.saturating_sub(target.0)) / 2;
    let top = (rh.saturating_sub(target.1)) / 2;
    if rw < target.0 || rh < target.1 {
        return Err(Error::Image(format!("cover crop to {}x{} from {rw}x{rh} is impossible", target.0, target.1)));
    }
    resized.crop(left, top, target.0, target.1)
}

pub(crate) fn add_grain(img: &mut Rgba, strength: i32, seed: i64) {
    if strength <= 0 {
        return;
    }
    let mut rng = PyRandom::new(seed);
    for p in img.data.chunks_exact_mut(4) {
        let noise = rng.randint(-strength, strength);
        for v in &mut p[..3] {
            *v = (i32::from(*v) + noise).clamp(0, 255) as u8;
        }
    }
}

/// Autocontrast, contrast, brightness, sepia colorize and grain; alpha is kept.
pub fn tone_photo(img: &Rgba, grain: i32, seed: i64) -> Rgba {
    let mut out = img.clone();
    autocontrast_rgb(&mut out, 1);
    enhance_contrast(&mut out, 1.10);
    enhance_brightness(&mut out, 0.98);
    let lut = colorize_lut([0x28, 0x20, 0x18], [0xee, 0xe2, 0xc7]);
    for p in out.data.chunks_exact_mut(4) {
        let l = luma(p[0], p[1], p[2]);
        let c = lut[usize::from(l)];
        p[0] = c[0];
        p[1] = c[1];
        p[2] = c[2];
    }
    add_grain(&mut out, grain, seed);
    out
}

fn paper_texture(size: (u32, u32), seed: i64, strength: i32) -> Rgba {
    let mut base = Rgba::filled(size.0, size.1, [232, 225, 207, 255]);
    add_grain(&mut base, strength, seed);
    base
}

pub(crate) fn rotate_card(card: &Rgba, angle: f64, supersample: u32, edge_soften: f64) -> Result<Rgba> {
    let ss = supersample.max(1);
    let mut rotated = if ss > 1 {
        let large = resize_lanczos3(card, card.width * ss, card.height * ss)?;
        let rotated_large = rotate_bicubic_expand(&large, angle)?;
        let fw = (py_round(f64::from(rotated_large.width) / f64::from(ss)).max(1)) as u32;
        let fh = (py_round(f64::from(rotated_large.height) / f64::from(ss)).max(1)) as u32;
        resize_lanczos3(&rotated_large, fw, fh)?
    } else {
        rotate_bicubic_expand(card, angle)?
    };
    if edge_soften > 0.0 {
        let alpha = gaussian_blur_plane(&alpha_plane(&rotated), rotated.width as usize, rotated.height as usize, edge_soften);
        for (p, a) in rotated.data.chunks_exact_mut(4).zip(alpha) {
            p[3] = a;
        }
    }
    Ok(rotated)
}

/// The drop-shadow layer: a full-canvas, black, transparent-elsewhere image.
///
/// The card alpha is padded by `ceil(3 * blur) + 1` first, so the blurred skirt keeps its whole tail, and the
/// result is placed on a canvas-sized layer so it is cropped only by the canvas itself.
pub fn shadow_layer(card: &Rgba, card_xy: (i64, i64), p: &ReportCardParams) -> Rgba {
    let pad = (3.0 * p.shadow_blur.max(0.0)).ceil() as u32 + 1;
    let (pw, ph) = (card.width + 2 * pad, card.height + 2 * pad);
    let mut padded = vec![0u8; pw as usize * ph as usize];
    for y in 0..card.height {
        for x in 0..card.width {
            padded[(y + pad) as usize * pw as usize + (x + pad) as usize] = card.alpha(x, y);
        }
    }
    let blurred = gaussian_blur_plane(&padded, pw as usize, ph as usize, p.shadow_blur);
    let mut layer = Rgba::new(p.canvas_size.0, p.canvas_size.1);
    let ox = card_xy.0 - i64::from(pad) + i64::from(p.shadow_offset.0);
    let oy = card_xy.1 - i64::from(pad) + i64::from(p.shadow_offset.1);
    for y in 0..i64::from(ph) {
        let dy = oy + y;
        if dy < 0 || dy >= i64::from(layer.height) {
            continue;
        }
        for x in 0..i64::from(pw) {
            let dx = ox + x;
            if dx < 0 || dx >= i64::from(layer.width) {
                continue;
            }
            let a = blurred[y as usize * pw as usize + x as usize];
            let a = (f64::from(a) * p.shadow_opacity).trunc().clamp(0.0, 255.0) as u8;
            layer.set_px(dx as u32, dy as u32, [0, 0, 0, a]);
        }
    }
    layer
}

/// Build one report card from a source picture of any size.
pub fn report_card(source: &Rgba, p: &ReportCardParams) -> Result<Rgba> {
    let bw = i64::from(p.border) * 2;
    let photo_w = i64::from(p.card_size.0) - bw;
    let photo_h = i64::from(p.card_size.1) - bw;
    if photo_w <= 0 || photo_h <= 0 {
        return Err(Error::Invalid("border is too large for the chosen card size".into()));
    }
    if p.canvas_size.0 == 0 || p.canvas_size.1 == 0 || !(0.0..=1.0).contains(&p.shadow_opacity) {
        return Err(Error::Invalid("canvas must be non-empty and shadow opacity between 0 and 1".into()));
    }
    let photo = cover_crop(source, (photo_w as u32, photo_h as u32))?;
    let photo = tone_photo(&photo, p.grain, p.seed);
    let card = if p.border > 0 {
        let mut paper = paper_texture(p.card_size, p.seed + 1000, p.paper_grain);
        alpha_composite(&mut paper, &photo, i64::from(p.border), i64::from(p.border));
        paper
    } else {
        photo
    };
    let rotated = rotate_card(&card, p.angle, p.rotate_supersample, p.edge_soften)?;
    let x = (i64::from(p.canvas_size.0) - i64::from(rotated.width)).div_euclid(2) + p.card_offset.0;
    let y = (i64::from(p.canvas_size.1) - i64::from(rotated.height)).div_euclid(2) + p.card_offset.1;
    let mut canvas = Rgba::new(p.canvas_size.0, p.canvas_size.1);
    let shadow = shadow_layer(&rotated, (x, y), p);
    alpha_composite(&mut canvas, &shadow, 0, 0);
    alpha_composite(&mut canvas, &rotated, x, y);
    Ok(canvas)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn picture(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_px(x, y, [(x * 3 % 256) as u8, (y * 5 % 256) as u8, ((x + y) * 2 % 256) as u8, 255]);
            }
        }
        img
    }

    #[test]
    fn defaults_are_the_measured_values() {
        let p = ReportCardParams::default();
        assert_eq!(p.canvas_size, (210, 176));
        assert_eq!(p.card_size, (192, 154));
        assert_eq!(p.angle, 4.0);
        assert_eq!(p.shadow_offset, (0, 1));
        assert_eq!(p.shadow_blur, 2.5);
        assert_eq!(p.shadow_opacity, 0.96);
    }

    #[test]
    fn card_has_the_canvas_size_a_transparent_corner_and_an_opaque_centre() {
        let out = report_card(&picture(300, 200), &ReportCardParams::default()).unwrap();
        assert_eq!((out.width, out.height), (210, 176));
        assert_eq!(out.px(0, 0)[3], 0);
        assert_eq!(out.px(105, 88)[3], 255);
        let b = out.alpha_bbox(1).unwrap();
        // the card is about 203 x 168 after rotation; the shadow adds a little on every side
        assert!(b[2] - b[0] >= 196 && b[2] - b[0] <= 210, "{b:?}");
        assert!(b[3] - b[1] >= 160 && b[3] - b[1] <= 176, "{b:?}");
    }

    #[test]
    fn processing_is_deterministic_and_the_seed_matters() {
        let src = picture(120, 90);
        let p = ReportCardParams::default();
        let a = report_card(&src, &p).unwrap();
        assert_eq!(a, report_card(&src, &p).unwrap());
        let b = report_card(&src, &ReportCardParams { seed: 1338, ..p }).unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn photo_is_sepia_toned() {
        let toned = tone_photo(&picture(40, 30), 0, 1);
        for p in toned.data.chunks_exact(4) {
            // the sepia ramp keeps R >= G >= B everywhere
            assert!(p[0] >= p[1] && p[1] >= p[2], "{p:?}");
        }
    }

    #[test]
    fn shadow_is_not_clipped_at_the_card_box() {
        // An opaque card with no soft edge: the shadow skirt must reach past the card by about 3 sigma.
        let p = ReportCardParams { angle: 0.0, edge_soften: 0.0, shadow_offset: (0, 0), ..Default::default() };
        let out = report_card(&picture(192, 154), &p).unwrap();
        // card spans x 9..201, y 11..165 on the canvas; a pixel 3 left of the edge still carries shadow
        assert!(out.px(5, 88)[3] > 0, "shadow clipped at the card box");
        assert!(out.px(5, 88)[3] < 128);
        assert_eq!(out.px(0, 0)[3], 0);
    }

    /// The phase card's alpha against vanilla's majority phase mask, and the report card's against vanilla's report
    /// masks, for comparison.
    #[test]
    fn the_phase_card_matches_vanillas_phase_picture_mask() {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let read = |rel: &str| crate::dds::decode(&std::fs::read(g.join(rel)).ok()?).ok();
        let (Some(phase), Some(report)) =
            (read("gfx/interface/operations/images/phase_bruneval_attack.dds"), read("gfx/event_pictures/report_event_AST_referendum.dds"))
        else {
            eprintln!("SKIPPED phase card vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        };
        let iou = |a: &Rgba, b: &Rgba| {
            let (mut i, mut u) = (0, 0);
            for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
                let (x, y) = (p[3] > 128, q[3] > 128);
                i += usize::from(x && y);
                u += usize::from(x || y);
            }
            i as f64 / u as f64
        };
        let mae = |a: &Rgba, b: &Rgba| {
            a.data.chunks_exact(4).zip(b.data.chunks_exact(4)).map(|(p, q)| (f64::from(p[3]) - f64::from(q[3])).abs()).sum::<f64>()
                / (a.data.len() / 4) as f64
        };
        let ours = report_card(&picture(400, 300), &ReportCardParams::phase()).unwrap();
        let card = report_card(&picture(400, 300), &ReportCardParams::default()).unwrap();
        let (pi, ri, cross) = (iou(&ours, &phase), iou(&card, &report), iou(&card, &phase));
        eprintln!(
            "phase card vs vanilla phase mask: IoU {pi:.3}, alpha MAE {:.2}; report card vs vanilla report mask: IoU {ri:.3}, alpha MAE {:.2}; report card vs phase mask: IoU {cross:.3}",
            mae(&ours, &phase),
            mae(&card, &report)
        );
        assert!(pi >= 0.97, "phase IoU {pi}");
        assert!(pi > cross + 0.05, "the phase preset is closer to the phase mask than a report card is");
        assert!(mae(&ours, &phase) < 8.0);
    }

    #[test]
    fn border_and_oversize_are_rejected() {
        let p = ReportCardParams { border: 200, ..Default::default() };
        assert!(report_card(&picture(50, 50), &p).is_err());
    }

    #[test]
    fn paper_border_is_drawn() {
        let p = ReportCardParams { border: 6, angle: 0.0, edge_soften: 0.0, ..Default::default() };
        let out = report_card(&picture(192, 154), &p).unwrap();
        // card spans x 9..201; the border pixel at x=10 is paper coloured (232, 225, 207)
        assert_eq!(&out.px(11, 88)[..3], &[232, 225, 207]);
    }
}
