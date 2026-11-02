//! Companion images the game looks up next to a family's main sprite, derived from the main art.
//!
//! Each derivation was fitted to the installed game (HOI4 1.19.4.1) and none is exact: vanilla draws these
//! companions by hand, so a derived one is a faithful stand-in, not a copy of a vanilla process.
//!
//! | Kind | Families | What vanilla shows | Derivation |
//! |---|---|---|---|
//! | [`DeriveKind::SmallEmblem`] | division emblem | `division_templates_small/` 30x12: the large emblem fitted to about 0.4 scale, light grey (values 124 to 211), with a black outline | fit the subject to 28x10 at most 30/76 scale, remap its luminance to 124..211, add a black outline at 20 percent of the neighbouring alpha |
//! | [`DeriveKind::TextIcon`] | on-map counter | `gfx/texticons/unit_*_icon_small.dds`: the on-map strip, frame 1 tinted green, frame 2 (the NATO box) unchanged in 22 of 34 pairs | frame 1 RGB multiplied by the least-squares tint (0.573, 0.735, 0.573), frame 2 copied |
//! | [`DeriveKind::Miniature`] | faction logo | `*_miniature` 32x32: frame 1's emblem fitted edge to edge (median bounding box 32 px) | frame 1's subject fitted to 32x32 |
//! | [`DeriveKind::HalfSize`] | medal (`_large` to the small strip), landmark (large to `_small`) | medals: the small strip is the large one downscaled (median MAE 2.6 against Lanczos) | Lanczos3 to half size, frame by frame |

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};
use crate::resize::resize_lanczos3;

/// Size of a small division emblem.
pub const SMALL_EMBLEM: [u32; 2] = [30, 12];
/// Largest scale from the large emblem: the canvas width ratio, 30/76 (vanilla's median is 0.40).
pub const SMALL_EMBLEM_MAX_SCALE: f64 = 30.0 / 76.0;
/// Luminance range of a small emblem's opaque pixels (vanilla deciles 10 and 90 of all 100 files).
pub const SMALL_EMBLEM_GREY: [f64; 2] = [124.0, 211.0];
/// Outline alpha as a share of the brightest neighbouring alpha (vanilla's outline is 36 to 54 beside opaque pixels).
pub const SMALL_EMBLEM_OUTLINE: f64 = 0.2;
/// Text icon tint of frame 1, fitted by least squares on the opaque pixels of the 34 vanilla pairs whose on-map
/// counter and text icon are both 60x12.
pub const TEXT_ICON_TINT: [f64; 3] = [0.573, 0.735, 0.573];
/// Size of a faction logo miniature.
pub const MINIATURE: [u32; 2] = [32, 32];

/// How a companion is made from the main art.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeriveKind {
    /// The 30x12 grey division emblem.
    SmallEmblem,
    /// The green text icon of an on-map counter strip.
    TextIcon,
    /// The 32x32 faction logo miniature from frame 1.
    Miniature,
    /// Every frame at half size.
    HalfSize,
}

impl DeriveKind {
    /// Stable name recorded with the companion.
    pub fn name(self) -> &'static str {
        match self {
            DeriveKind::SmallEmblem => "small_emblem",
            DeriveKind::TextIcon => "text_icon",
            DeriveKind::Miniature => "miniature",
            DeriveKind::HalfSize => "half_size",
        }
    }
}

/// Make one companion from `main`, a strip of `frames` frames.
pub fn derive(kind: DeriveKind, main: &Rgba, frames: u32) -> Result<Rgba> {
    match kind {
        DeriveKind::SmallEmblem => small_emblem(main),
        DeriveKind::TextIcon => text_icon(main, frames),
        DeriveKind::Miniature => miniature(main, frames),
        DeriveKind::HalfSize => half_size(main, frames),
    }
}

/// The subject of `img` (alpha of at least 16), resized by `scale` within `max` and centred on a `canvas`.
fn fit_centred(img: &Rgba, canvas: [u32; 2], max: [u32; 2], scale_cap: Option<f64>) -> Result<Rgba> {
    let b = img.alpha_bbox(16).ok_or_else(|| Error::Image("the art has no visible subject to derive a companion from".into()))?;
    let (bw, bh) = (b[2] - b[0], b[3] - b[1]);
    let subject = img.crop(b[0], b[1], bw, bh)?;
    let mut scale = (f64::from(max[0]) / f64::from(bw)).min(f64::from(max[1]) / f64::from(bh));
    if let Some(cap) = scale_cap {
        scale = scale.min(cap);
    }
    let w = ((f64::from(bw) * scale).round() as u32).clamp(1, max[0]);
    let h = ((f64::from(bh) * scale).round() as u32).clamp(1, max[1]);
    let resized = resize_lanczos3(&subject, w, h)?;
    let mut out = Rgba::new(canvas[0], canvas[1]);
    out.paste(&resized, i64::from((canvas[0] - w) / 2), i64::from((canvas[1] - h) / 2));
    Ok(out)
}

fn luminance(p: [u8; 4]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// The 30x12 small division emblem: the subject fitted to 28x10 (at most 30/76 scale), its luminance remapped to
/// vanilla's light grey range and a black outline around it.
pub fn small_emblem(large: &Rgba) -> Result<Rgba> {
    let fitted = fit_centred(large, SMALL_EMBLEM, [SMALL_EMBLEM[0] - 2, SMALL_EMBLEM[1] - 2], Some(SMALL_EMBLEM_MAX_SCALE))?;
    // deciles 10 and 90 of the opaque pixels' luminance, mapped onto vanilla's range
    let mut lum: Vec<f64> = fitted.data.chunks_exact(4).filter(|p| p[3] >= 128).map(|p| luminance([p[0], p[1], p[2], p[3]])).collect();
    lum.sort_by(f64::total_cmp);
    let (lo, hi) = match lum.len() {
        0 => (0.0, 0.0),
        n => (lum[(n - 1) / 10], lum[(n - 1) * 9 / 10]),
    };
    let [g0, g1] = SMALL_EMBLEM_GREY;
    let mut grey = fitted.clone();
    for p in grey.data.chunks_exact_mut(4) {
        if p[3] == 0 {
            p[..3].copy_from_slice(&[0, 0, 0]);
            continue;
        }
        let l = luminance([p[0], p[1], p[2], p[3]]);
        let v = if hi - lo < 1.0 { (g0 + g1) / 2.0 } else { g0 + (l - lo) * (g1 - g0) / (hi - lo) };
        let v = v.round().clamp(0.0, 255.0) as u8;
        p[..3].copy_from_slice(&[v, v, v]);
    }
    // black outline under the grey subject
    let (w, h) = (grey.width, grey.height);
    let mut out = Rgba::new(w, h);
    for y in 0..h {
        for x in 0..w {
            let mut m = 0u8;
            for dy in -1i64..=1 {
                for dx in -1i64..=1 {
                    let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                    if (dx != 0 || dy != 0) && nx >= 0 && ny >= 0 && nx < i64::from(w) && ny < i64::from(h) {
                        m = m.max(grey.alpha(nx as u32, ny as u32));
                    }
                }
            }
            let outline = [0, 0, 0, (f64::from(m) * SMALL_EMBLEM_OUTLINE).round() as u8];
            out.set_px(x, y, over(grey.px(x, y), outline));
        }
    }
    Ok(out)
}

/// The green text icon of an on-map counter strip: frame 1 tinted, the other frames copied.
pub fn text_icon(onmap: &Rgba, frames: u32) -> Result<Rgba> {
    let mut parts = onmap.split_frames(frames.max(1))?;
    for p in parts[0].data.chunks_exact_mut(4) {
        for (c, t) in p[..3].iter_mut().zip(TEXT_ICON_TINT) {
            *c = (f64::from(*c) * t).round().clamp(0.0, 255.0) as u8;
        }
    }
    Rgba::join_frames(&parts)
}

/// The 32x32 miniature: frame 1's subject fitted edge to edge.
pub fn miniature(strip: &Rgba, frames: u32) -> Result<Rgba> {
    let first = strip.split_frames(frames.max(1))?.swap_remove(0);
    fit_centred(&first, MINIATURE, MINIATURE, None)
}

/// Every frame at half size (Lanczos3).
pub fn half_size(strip: &Rgba, frames: u32) -> Result<Rgba> {
    let parts = strip.split_frames(frames.max(1))?;
    let mut out = Vec::with_capacity(parts.len());
    for p in &parts {
        if p.width < 2 || p.height < 2 || p.width % 2 != 0 || p.height % 2 != 0 {
            return Err(Error::Image(format!("a {}x{} frame has no exact half size", p.width, p.height)));
        }
        out.push(resize_lanczos3(p, p.width / 2, p.height / 2)?);
    }
    Rgba::join_frames(&out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(w: u32, h: u32, cx: f64, cy: f64, r: f64, rgb: [u8; 3]) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - cy).powi(2)).sqrt();
                if d <= r {
                    img.set_px(x, y, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        img
    }

    #[test]
    fn the_small_emblem_is_a_grey_30x12_with_a_dark_outline() {
        let large = disc(76, 42, 38.0, 21.0, 16.0, [60, 110, 70]);
        let small = small_emblem(&large).unwrap();
        assert_eq!((small.width, small.height), (30, 12));
        let b = small.alpha_bbox(1).unwrap();
        assert!(b[3] - b[1] >= 10, "the subject fills the height: {b:?}");
        // grey: every visible pixel has r == g == b, opaque pixels are light, the outline is black
        let mut light = 0;
        for p in small.data.chunks_exact(4).filter(|p| p[3] > 0) {
            assert!(p[0] == p[1] && p[1] == p[2], "{p:?}");
            if p[3] == 255 {
                assert!(p[0] >= 100, "{p:?}");
                light += 1;
            }
        }
        assert!(light > 30);
        assert_eq!(small.px(b[0], 6)[0], 0, "the left edge is the outline");
        // deterministic
        assert_eq!(small_emblem(&large).unwrap().pixels_sha256(), small.pixels_sha256());
        assert!(small_emblem(&Rgba::new(76, 42)).is_err(), "an empty emblem has nothing to derive from");
    }

    #[test]
    fn the_text_icon_tints_frame_one_and_keeps_frame_two() {
        let mut strip = Rgba::filled(60, 12, [200, 200, 200, 255]);
        strip.set_px(40, 5, [10, 20, 30, 128]);
        let t = text_icon(&strip, 2).unwrap();
        assert_eq!(t.px(3, 3), [115, 147, 115, 255]);
        assert_eq!(t.px(40, 5), [10, 20, 30, 128]);
        assert_eq!(t.crop(30, 0, 30, 12).unwrap(), strip.crop(30, 0, 30, 12).unwrap());
    }

    #[test]
    fn the_miniature_fits_frame_one_edge_to_edge() {
        let mut strip = Rgba::new(200, 100);
        strip.paste(&disc(100, 100, 50.0, 50.0, 40.0, [200, 30, 30]), 0, 0);
        strip.paste(&disc(100, 100, 50.0, 50.0, 45.0, [255, 255, 255]), 100, 0);
        let m = miniature(&strip, 2).unwrap();
        assert_eq!((m.width, m.height), (32, 32));
        let b = m.alpha_bbox(16).unwrap();
        assert_eq!((b[2] - b[0], b[3] - b[1]), (32, 32));
        assert!(m.px(16, 16)[0] > 150 && m.px(16, 16)[1] < 80, "frame 1, not frame 2");
    }

    #[test]
    fn half_size_halves_every_frame() {
        let strip = Rgba::filled(128, 64, [10, 20, 30, 255]);
        let h = half_size(&strip, 2).unwrap();
        assert_eq!((h.width, h.height), (64, 32));
        assert!(half_size(&Rgba::new(57, 84), 1).is_err());
    }
}
