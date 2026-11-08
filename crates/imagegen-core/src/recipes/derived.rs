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
//! | [`DeriveKind::MapMedallion`] | operation card | `map/<name>_map.dds` 48x48: the picture's centre at 0.6 scale in a round, bevelled gold ring | the central 64 px of the card in the ring of [`crate::recipes::operation_card`] |
//! | [`DeriveKind::HalfSize`] | medal (`_large` to the small strip) | medals: the small strip is the large one downscaled (median MAE 2.6 against Lanczos) | Lanczos3 to half size, frame by frame |
//! | [`DeriveKind::LandmarkSmall`] | landmark (large to `_small`) | `buildings/*_icon_small` 64x32: a flat orange pictogram, yellow-orange at the top to deep orange at the bottom, with a brown edge and a faint black rim, in a box of 26x19 inside 3 px at the sides, 6 at the top and 7 at the bottom | both frames cropped together, fitted into that box, then recoloured on vanilla's orange ramp ([`LANDMARK_SMALL`]) |

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
    /// The flat orange pictogram strip of a small landmark.
    LandmarkSmall,
    /// The 48x48 map medallion of an operation card: the card's centre in a gold ring.
    MapMedallion,
}

impl DeriveKind {
    /// Stable name recorded with the companion.
    pub fn name(self) -> &'static str {
        match self {
            DeriveKind::SmallEmblem => "small_emblem",
            DeriveKind::TextIcon => "text_icon",
            DeriveKind::Miniature => "miniature",
            DeriveKind::HalfSize => "half_size",
            DeriveKind::LandmarkSmall => "landmark_small",
            DeriveKind::MapMedallion => "map_medallion",
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
        DeriveKind::LandmarkSmall => landmark_small(main, frames),
        DeriveKind::MapMedallion => crate::recipes::operation_card::map_medallion(main),
    }
}

/// Vanilla's small landmark ramp (HOI4 1.19.4.1, frame 1 of the 23 `gfx/interface/buildings/*_icon_small.dds` strips
/// of 64x32): the median colour of the inside pixels whose luminance lies within 8 of 48, 64 ... 192, extended
/// linearly to black below 48 where vanilla has almost no pixels.
pub const LANDMARK_RAMP: [[u8; 3]; 13] = [
    [0, 0, 0],
    [28, 14, 3],
    [57, 28, 6],
    [85, 42, 9],
    [102, 55, 11],
    [133, 64, 13],
    [154, 82, 17],
    [182, 94, 20],
    [243, 102, 19],
    [246, 117, 23],
    [247, 141, 30],
    [249, 162, 36],
    [250, 182, 42],
];

/// The small landmark look: inside luminance deciles 1, 120, 135 ... 193 (median 148, five bands top to bottom
/// 185, 172, 160, 148, 139), edge luminance 103 (an orange-brown), and alpha 39 one pixel outside the shape.
pub const LANDMARK_SMALL: crate::recipes::raid::GradientStyle = crate::recipes::raid::GradientStyle {
    ramp: &LANDMARK_RAMP,
    quantiles: [1.0, 120.0, 135.0, 139.0, 143.0, 148.0, 153.0, 158.0, 165.0, 172.0, 193.0],
    outline: [166, 87, 18],
    height_weight: 0.5,
    rim_alpha: 39,
};

/// The box a small landmark's pictogram is fitted into, `[x0, y0, x1, y1)` (vanilla's median margins: 3 px at the
/// sides, 6 at the top, 7 at the bottom of the 32x32 frame).
pub const LANDMARK_SMALL_BOX: [u32; 4] = [3, 6, 29, 25];

/// The small landmark strip: frame 1's building fitted into [`LANDMARK_SMALL_BOX`] of a 32x32 frame, every other frame
/// moved and scaled the same way (so the ruin keeps its size and place against the intact building, and its flames
/// may rise above the box as vanilla's do), then recoloured as vanilla's flat orange pictograms.
pub fn landmark_small(strip: &Rgba, frames: u32) -> Result<Rgba> {
    let parts = strip.split_frames(frames.max(1))?;
    let b1 = parts[0].alpha_bbox(16).ok_or_else(|| Error::Image("the landmark's frame 1 has no visible subject".into()))?;
    let [x0, y0, x1, y1] = LANDMARK_SMALL_BOX;
    let scale = (f64::from(x1 - x0) / f64::from(b1[2] - b1[0])).min(f64::from(y1 - y0) / f64::from(b1[3] - b1[1]));
    let (fw, fh) = (parts[0].width, parts[0].height);
    let (tw, th) = (((f64::from(fw) * scale).round() as u32).max(1), ((f64::from(fh) * scale).round() as u32).max(1));
    // where frame 1's box lands: centred in the vanilla box
    let ox = f64::from(x0) + (f64::from(x1 - x0) - f64::from(b1[2] - b1[0]) * scale) / 2.0 - f64::from(b1[0]) * scale;
    let oy = f64::from(y0) + (f64::from(y1 - y0) - f64::from(b1[3] - b1[1]) * scale) / 2.0 - f64::from(b1[1]) * scale;
    let mut out = Vec::with_capacity(parts.len());
    for p in &parts {
        let small = resize_lanczos3(p, tw, th)?;
        let mut frame = Rgba::new(32, 32);
        frame.paste(&small, ox.round() as i64, oy.round() as i64);
        let solid = frame.data.chunks_exact(4).filter(|p| p[3] >= 128).count();
        out.push(if solid >= 4 { crate::recipes::raid::gradient_map(&frame, &LANDMARK_SMALL)? } else { frame });
    }
    Rgba::join_frames(&out)
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

    /// Vanilla's large landmark strips, made small by the derivation, against vanilla's own small strips of the same
    /// landmarks: the same orange (inside median within 15 per channel), the gradient from top to bottom, and about
    /// the same size in the frame.
    #[test]
    fn the_small_landmark_matches_vanillas_orange_pictograms() {
        use crate::recipes::raid::raid_stats;
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let mut stack = vec![g.join("gfx/interface/buildings")];
        let mut files = std::collections::BTreeMap::new();
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if let Some(img) = std::fs::read(&p).ok().and_then(|b| crate::dds::decode(&b).ok()) {
                    files.insert(p.file_stem().unwrap().to_string_lossy().to_string(), img);
                }
            }
        }
        let pairs: Vec<(Rgba, Rgba)> = files
            .iter()
            .filter(|(n, i)| (i.width, i.height) == (128, 64) && !n.ends_with("_small"))
            .filter_map(|(n, large)| files.get(&format!("{n}_small")).filter(|s| (s.width, s.height) == (64, 32)).map(|s| (large.clone(), s.clone())))
            .collect();
        if pairs.len() < 10 {
            eprintln!("SKIPPED small landmark vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        let first = |s: &Rgba| s.split_frames(2).unwrap().swap_remove(0);
        let van: Vec<Rgba> = pairs.iter().map(|(_, s)| first(s)).collect();
        let ours: Vec<Rgba> = pairs.iter().map(|(l, _)| first(&landmark_small(l, 2).unwrap())).collect();
        let halved: Vec<Rgba> = pairs.iter().map(|(l, _)| first(&half_size(l, 2).unwrap())).collect();
        let stats = |set: &[Rgba]| raid_stats(&set.iter().collect::<Vec<_>>());
        let (v, o, h) = (stats(&van), stats(&ours), stats(&halved));
        let span = |set: &[Rgba]| {
            let mut w: Vec<u32> = set.iter().filter_map(|i| i.alpha_bbox(128)).map(|b| b[2] - b[0]).collect();
            w.sort();
            w[w.len() / 2]
        };
        eprintln!(
            "small landmark, {} pairs, medians\n  vanilla {v:?} width {}\n  half size (before) {h:?} width {}\n  ours {o:?} width {}",
            pairs.len(),
            span(&van),
            span(&halved),
            span(&ours)
        );
        for c in 0..3 {
            assert!((o.core_rgb[c] - v.core_rgb[c]).abs() <= 15.0, "channel {c}: {o:?} vs {v:?}");
        }
        assert!(o.bands[0] > o.bands[4] + 20.0, "light at the top: {o:?}");
        for (a, b) in o.bands.iter().zip(v.bands) {
            assert!((a - b).abs() <= 20.0, "bands {:?} vs {:?}", o.bands, v.bands);
        }
        assert!((i64::from(span(&ours)) - i64::from(span(&van))).abs() <= 4);
    }

    #[test]
    fn half_size_halves_every_frame() {
        let strip = Rgba::filled(128, 64, [10, 20, 30, 255]);
        let h = half_size(&strip, 2).unwrap();
        assert_eq!((h.width, h.height), (64, 32));
        assert!(half_size(&Rgba::new(57, 84), 1).is_err());
    }
}
