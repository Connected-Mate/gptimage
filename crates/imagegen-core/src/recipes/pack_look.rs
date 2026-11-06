//! A style pack's look on kept-colours unit art, measured from the pack's own references: the palette the glyph is
//! snapped to (`palette_snap`) and the soft edge glow drawn around the glyph and the NATO outline (`edge_glow`).
//!
//! Why: a counter made with a pack that keeps the art's own colours (`keep_colours`) keeps whatever shade the model
//! drew. TNO's 8 battalion strips draw every glyph in one light teal: k-means finds one colour, (89, 199, 194), with 95
//! percent of their 3,612 opaque frame-1 pixels within 0.75 (CIE76) of it. The 0.3.4 gallery's 7 teal TNO counters drew
//! it 4.1 to 7.3 darker, so 6 of them fell inside the middle value band where the references sit just above it. TNO's
//! art also carries a soft glow: the pixels one step outside a shape's core have alpha 73 on frame 1 and 60 on frame 2
//! (medians over the 8 references), two steps outside 12 and 18. The kept outline had none, so a generated strip had a
//! third of a reference's visible pixels on frame 2. [`ink_box`] scales the glyph to the references' size: their
//! frame-1 glyphs hold 342 to 604 opaque pixels (median 451), the gallery's solid APC and truck, fitted to vanilla's
//! 62x30 box, 1,118 and 1,188.
//!
//! [`measure`] reads all three from the references, [`snap`] moves each glyph pixel to its palette colour, and [`glow`]
//! draws the measured glow. Everything is a pure function of the pixels, deterministic on every platform. The
//! references are the mod's own files and are only read.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Alpha at and above which a reference pixel counts as glyph fill for the palette.
pub const OPAQUE: u8 = 250;

/// Alpha at and above which a pixel is the core of a shape when its glow is measured or drawn.
pub const CORE_ALPHA: u8 = 200;

/// Most palette colours.
pub const MAX_COLOURS: usize = 4;

/// The palette grows (one colour at a time, up to [`MAX_COLOURS`]) until 95 percent of the reference pixels lie
/// within this CIE76 distance of their palette colour. 6 is about two just-noticeable differences: TNO's references
/// need one colour (95th percentile 0.4).
pub const PALETTE_TOLERANCE: f64 = 6.0;

/// A colour holding less than this share of the reference pixels is left out of the palette (stray pixels).
pub const MIN_SHARE: f64 = 0.02;

/// A glyph pixel within this CIE76 distance of a palette colour is that colour drawn a few shades off and is snapped
/// to it; anything further is a colour the pack does not have and is kept. Measured: the 0.3.4 gallery's 8 TNO counter
/// glyphs were 4.1 to 8.2 from TNO's teal, its naval salmon is 83, white 39, a blue or a green about 61.
pub const SNAP_DISTANCE: f64 = 20.0;

/// Fewest reference pixels a palette is measured from.
pub const MIN_PIXELS: usize = 50;

/// One palette colour and its share of the reference pixels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PaletteColour {
    /// sRGB colour.
    pub rgb: [u8; 3],
    /// Share of the measured reference pixels nearest to it.
    pub share: f64,
}

/// What [`measure`] read from a pack's references.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PackLook {
    /// Glyph palette, most used first. Empty when the references have too few opaque pixels.
    pub palette: Vec<PaletteColour>,
    /// 95th percentile CIE76 distance of the reference pixels to their palette colour.
    pub palette_residual: f64,
    /// Opaque reference pixels the palette was measured from.
    pub palette_pixels: usize,
    /// Edge glow per frame: the median alpha one and two pixels (Chebyshev) outside the core. `[0, 0]` is no glow.
    pub glow: Vec<[u8; 2]>,
    /// The references' glyph size: the median count of frame-1 pixels with alpha 128 and up. 0 when none.
    pub glyph_ink: f64,
}

/// CIE L*a*b* (D65) of an sRGB colour.
pub fn lab(c: [u8; 3]) -> [f64; 3] {
    let lin = |v: u8| {
        let v = f64::from(v) / 255.0;
        if v <= 0.04045 {
            v / 12.92
        } else {
            ((v + 0.055) / 1.055).powf(2.4)
        }
    };
    let (r, g, b) = (lin(c[0]), lin(c[1]), lin(c[2]));
    let x = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
    let y = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
    let z = (0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b) / 1.088_83;
    let f = |t: f64| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
    let (fx, fy, fz) = (f(x), f(y), f(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn dist(a: [f64; 3], b: [f64; 3]) -> f64 {
    ((a[0] - b[0]).powi(2) + (a[1] - b[1]).powi(2) + (a[2] - b[2]).powi(2)).sqrt()
}

/// Deterministic k-means in Lab over distinct colours weighted by count. Seeds: the most common colour, then each
/// next seed the colour furthest from the seeds so far (ties by colour order).
fn kmeans(colours: &[([u8; 3], [f64; 3], u64)], k: usize) -> Vec<([f64; 3], [f64; 3], u64)> {
    let mut seeds: Vec<[f64; 3]> = vec![colours[0].1];
    while seeds.len() < k {
        let far = colours
            .iter()
            .map(|c| (seeds.iter().map(|s| dist(c.1, *s)).fold(f64::MAX, f64::min), c))
            .max_by(|a, b| a.0.total_cmp(&b.0).then(b.1 .0.cmp(&a.1 .0)))
            .map(|(_, c)| c.1);
        match far {
            Some(f) if !seeds.contains(&f) => seeds.push(f),
            _ => break,
        }
    }
    let mut out = Vec::new();
    for _ in 0..30 {
        // (lab sum, rgb sum, count) per centre
        let mut acc = vec![([0f64; 3], [0f64; 3], 0u64); seeds.len()];
        for (rgb, l, n) in colours {
            let i = nearest(&seeds, *l).0;
            for c in 0..3 {
                acc[i].0[c] += l[c] * *n as f64;
                acc[i].1[c] += f64::from(rgb[c]) * *n as f64;
            }
            acc[i].2 += n;
        }
        let next: Vec<[f64; 3]> = acc.iter().zip(&seeds).map(|(a, s)| if a.2 == 0 { *s } else { a.0.map(|v| v / a.2 as f64) }).collect();
        out = acc.iter().map(|a| (a.0.map(|v| v / a.2.max(1) as f64), a.1.map(|v| v / a.2.max(1) as f64), a.2)).collect();
        if next == seeds {
            break;
        }
        seeds = next;
    }
    out
}

fn nearest(centres: &[[f64; 3]], l: [f64; 3]) -> (usize, f64) {
    centres.iter().enumerate().map(|(i, c)| (i, dist(*c, l))).fold((0, f64::MAX), |b, x| if x.1 < b.1 { x } else { b })
}

/// The glyph palette of `glyphs` (frame 1 of each reference): k-means in Lab on their opaque pixels, with the
/// smallest k (1 to [`MAX_COLOURS`]) at which 95 percent of the pixels lie within [`PALETTE_TOLERANCE`] of their
/// colour, colours under [`MIN_SHARE`] left out. Each colour is the mean sRGB of its pixels, rounded. Returns the
/// palette, the 95th percentile distance and the pixel count; an empty palette under [`MIN_PIXELS`] pixels.
pub fn palette(glyphs: &[Rgba]) -> (Vec<PaletteColour>, f64, usize) {
    let mut counts: std::collections::BTreeMap<[u8; 3], u64> = std::collections::BTreeMap::new();
    for g in glyphs {
        for p in g.data.chunks_exact(4).filter(|p| p[3] >= OPAQUE) {
            *counts.entry([p[0], p[1], p[2]]).or_default() += 1;
        }
    }
    let total: u64 = counts.values().sum();
    if (total as usize) < MIN_PIXELS {
        return (Vec::new(), 0.0, total as usize);
    }
    let mut colours: Vec<([u8; 3], [f64; 3], u64)> = counts.iter().map(|(c, n)| (*c, lab(*c), *n)).collect();
    colours.sort_by(|a, b| b.2.cmp(&a.2).then(a.0.cmp(&b.0)));
    let p95 = |centres: &[[f64; 3]]| {
        let mut d: Vec<(f64, u64)> = colours.iter().map(|c| (nearest(centres, c.1).1, c.2)).collect();
        d.sort_by(|a, b| a.0.total_cmp(&b.0));
        let target = (total as f64 * 0.95).ceil() as u64;
        let mut acc = 0u64;
        for (v, n) in &d {
            acc += n;
            if acc >= target {
                return *v;
            }
        }
        d.last().map_or(0.0, |x| x.0)
    };
    let mut best = Vec::new();
    for k in 1..=MAX_COLOURS {
        let found: Vec<([f64; 3], [f64; 3], u64)> = kmeans(&colours, k).into_iter().filter(|c| c.2 as f64 >= MIN_SHARE * total as f64).collect();
        let centres: Vec<[f64; 3]> = found.iter().map(|c| c.0).collect();
        let residual = p95(&centres);
        best = found;
        if residual <= PALETTE_TOLERANCE || best.len() < k {
            break;
        }
    }
    let centres: Vec<[f64; 3]> = best.iter().map(|c| c.0).collect();
    let residual = (p95(&centres) * 100.0).round() / 100.0;
    // shares by nearest colour over every pixel, so they sum to one
    let mut share = vec![0u64; best.len()];
    for c in &colours {
        share[nearest(&centres, c.1).0] += c.2;
    }
    let mut out: Vec<PaletteColour> = best
        .iter()
        .zip(share)
        .map(|(c, n)| PaletteColour {
            rgb: c.1.map(|v| v.round().clamp(0.0, 255.0) as u8),
            share: (n as f64 / total as f64 * 10_000.0).round() / 10_000.0,
        })
        .collect();
    out.sort_by(|a, b| b.share.total_cmp(&a.share).then(a.rgb.cmp(&b.rgb)));
    (out, residual, total as usize)
}

/// How far each pixel of `img` (alpha `min_alpha` and up) lies from a shape core: 1 or 2 for the two rings outside
/// the core (Chebyshev), 0 for the core itself and anything further. With the index of a core pixel at that distance
/// (the first in raster order among the most opaque).
fn rings(img: &Rgba) -> Vec<(u8, usize)> {
    let (w, h) = (img.width as i64, img.height as i64);
    let core = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && img.data[((y * w + x) * 4 + 3) as usize] >= CORE_ALPHA;
    let mut out = vec![(0u8, 0usize); (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            if core(x, y) {
                continue;
            }
            'ring: for d in 1..=2i64 {
                let mut pick: Option<(u8, usize)> = None;
                for yy in y - d..=y + d {
                    for xx in x - d..=x + d {
                        if (xx - x).abs().max((yy - y).abs()) == d && core(xx, yy) {
                            let i = (yy * w + xx) as usize;
                            let a = img.data[i * 4 + 3];
                            if pick.is_none_or(|(pa, _)| a > pa) {
                                pick = Some((a, i));
                            }
                        }
                    }
                }
                if let Some((_, i)) = pick {
                    out[(y * w + x) as usize] = (d as u8, i);
                    break 'ring;
                }
            }
        }
    }
    out
}

/// The edge glow of `frames` (the same frame of each reference): the median alpha of the pixels one and two steps
/// outside a shape's core. `[0, 0]` when no frame has a core.
pub fn measure_glow(frames: &[Rgba]) -> [u8; 2] {
    let mut d: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
    for f in frames {
        for (i, (ring, _)) in rings(f).into_iter().enumerate() {
            if ring > 0 {
                d[usize::from(ring) - 1].push(f.data[i * 4 + 3]);
            }
        }
    }
    let median = |v: &mut Vec<u8>| {
        if v.is_empty() {
            return 0;
        }
        v.sort_unstable();
        v[v.len() / 2]
    };
    [median(&mut d[0]), median(&mut d[1])]
}

/// Read a pack's look from its references for a family with `frames` frames of `frame_width` each. Frame 1 of each
/// reference gives the palette; each frame gives its glow. A reference that is not a strip of that many frames counts
/// as frame 1, and a frame no reference has takes frame 1's glow.
pub fn measure(references: &[Rgba], frames: u32, frame_width: u32) -> PackLook {
    let frames = frames.max(1);
    let mut per_frame: Vec<Vec<Rgba>> = vec![Vec::new(); frames as usize];
    for r in references {
        if frames > 1 && r.width == frames * frame_width {
            if let Ok(parts) = r.split_frames(frames) {
                for (i, p) in parts.into_iter().enumerate() {
                    per_frame[i].push(p);
                }
                continue;
            }
        }
        per_frame[0].push(r.clone());
    }
    let (palette, palette_residual, palette_pixels) = palette(&per_frame[0]);
    let first = measure_glow(&per_frame[0]);
    let glow = per_frame.iter().map(|f| if f.is_empty() { first } else { measure_glow(f) }).collect();
    let mut ink: Vec<usize> = per_frame[0].iter().map(|f| f.data.chunks_exact(4).filter(|p| p[3] >= 128).count()).filter(|n| *n > 0).collect();
    ink.sort_unstable();
    let glyph_ink = match ink.len() {
        0 => 0.0,
        n if n % 2 == 1 => ink[n / 2] as f64,
        n => (ink[n / 2 - 1] + ink[n / 2]) as f64 / 2.0,
    };
    PackLook { palette, palette_residual, palette_pixels, glow, glyph_ink }
}

/// The box a kept-colours glyph is fitted into when a pack's references give its size: the box that scales the
/// subject of `img` (alpha 16 and up) to `ink` opaque pixels, assuming its opaque share of its box stays as it is, and
/// never larger than `max_box`. A thin subject that the box caps keeps `max_box` (the gallery's infantry, 21x30 with
/// 243 opaque pixels, and destroyer, 62x17 with 374), and a dense one is drawn smaller, at the references' size.
pub fn ink_box(img: &Rgba, max_box: [u32; 2], ink: f64) -> [u32; 2] {
    let Some(b) = img.alpha_bbox(16) else { return max_box };
    let (sw, sh) = (f64::from(b[2] - b[0]), f64::from(b[3] - b[1]));
    let mut solid = 0usize;
    for y in b[1]..b[3] {
        for x in b[0]..b[2] {
            if img.alpha(x, y) >= 128 {
                solid += 1;
            }
        }
    }
    if solid == 0 || ink <= 0.0 {
        return max_box;
    }
    let fill = solid as f64 / (sw * sh);
    let cap = (f64::from(max_box[0]) / sw).min(f64::from(max_box[1]) / sh);
    let s = (ink / (fill * sw * sh)).sqrt().min(cap);
    [((sw * s).round() as u32).clamp(1, max_box[0]), ((sh * s).round() as u32).clamp(1, max_box[1])]
}

/// How a snap went: pixels moved to a palette colour, pixels kept because no palette colour is near.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct SnapStats {
    /// Visible pixels moved onto the palette.
    pub snapped: usize,
    /// Visible pixels kept: no palette colour within the distance.
    pub kept: usize,
}

/// Snap each visible pixel (alpha 16 and up) of `img` to its nearest palette colour when that is within
/// `max_distance` (CIE76), keeping its alpha, so a soft edge stays a soft edge in the snapped colour. A pixel between
/// two palette colours (an anti-aliased border inside the glyph: on the segment between them, nearer than half their
/// distance) becomes the matching blend of the two. Pixels further from every colour keep their colour.
pub fn snap(img: &Rgba, palette: &[[u8; 3]], max_distance: f64) -> (Rgba, SnapStats) {
    let mut out = img.clone();
    let mut stats = SnapStats::default();
    if palette.is_empty() {
        return (out, stats);
    }
    let labs: Vec<[f64; 3]> = palette.iter().map(|c| lab(*c)).collect();
    let mut cache: std::collections::HashMap<[u8; 3], Option<[u8; 3]>> = std::collections::HashMap::new();
    for p in out.data.chunks_exact_mut(4) {
        if p[3] < 16 {
            continue;
        }
        let c = [p[0], p[1], p[2]];
        let to = *cache.entry(c).or_insert_with(|| target(c, palette, &labs, max_distance));
        match to {
            Some(t) => {
                p[..3].copy_from_slice(&t);
                stats.snapped += 1;
            }
            None => stats.kept += 1,
        }
    }
    (out, stats)
}

fn target(c: [u8; 3], palette: &[[u8; 3]], labs: &[[f64; 3]], max_distance: f64) -> Option<[u8; 3]> {
    let l = lab(c);
    let mut order: Vec<(usize, f64)> = labs.iter().enumerate().map(|(i, p)| (i, dist(*p, l))).collect();
    order.sort_by(|a, b| a.1.total_cmp(&b.1).then(a.0.cmp(&b.0)));
    let (i, d) = order[0];
    if let Some(&(j, _)) = order.get(1) {
        // a blend of two palette colours: project onto the segment between them
        let (a, b) = (labs[i], labs[j]);
        let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let len2 = ab.iter().map(|v| v * v).sum::<f64>();
        if len2 > 0.0 {
            let t = ((l[0] - a[0]) * ab[0] + (l[1] - a[1]) * ab[1] + (l[2] - a[2]) * ab[2]) / len2;
            let on = [a[0] + t * ab[0], a[1] + t * ab[1], a[2] + t * ab[2]];
            if (0.15..=0.85).contains(&t) && dist(on, l) < 0.5 * len2.sqrt() && dist(on, l) <= max_distance {
                let (pa, pb) = (palette[i], palette[j]);
                return Some([0, 1, 2].map(|k| (f64::from(pa[k]) + t * (f64::from(pb[k]) - f64::from(pa[k]))).round().clamp(0.0, 255.0) as u8));
            }
        }
    }
    (d <= max_distance).then_some(palette[i])
}

/// Draw an edge glow on each of `frames` frames of `img`: a pixel one or two steps (Chebyshev) outside a shape's core
/// gets at least that ring's alpha from `glow` (one `[ring 1, ring 2]` per frame), in the colour of the core pixel next
/// to it when it was clear. Pixels already more opaque keep their alpha and colour.
pub fn glow(img: &Rgba, frames: u32, glow: &[[u8; 2]]) -> Result<Rgba> {
    let frames = frames.max(1);
    if glow.len() != frames as usize {
        return Err(Error::Invalid(format!("edge_glow has {} frame values for {frames} frames", glow.len())));
    }
    let parts = if frames > 1 { img.split_frames(frames)? } else { vec![img.clone()] };
    let mut done = Vec::with_capacity(parts.len());
    for (f, g) in parts.into_iter().zip(glow) {
        let mut out = f.clone();
        for (i, (ring, from)) in rings(&f).into_iter().enumerate() {
            if ring == 0 {
                continue;
            }
            let a = g[usize::from(ring) - 1];
            let p = &mut out.data[i * 4..i * 4 + 4];
            if a <= p[3] {
                continue;
            }
            if p[3] < 16 {
                p[..3].copy_from_slice(&f.data[from * 4..from * 4 + 3]);
            }
            p[3] = a;
        }
        done.push(out);
    }
    if done.len() == 1 {
        Ok(done.remove(0))
    } else {
        Rgba::join_frames(&done)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAL: [u8; 3] = [89, 199, 194];

    fn glyph(colour: [u8; 3]) -> Rgba {
        let mut img = Rgba::new(76, 42);
        for y in 14..30 {
            for x in 20..56 {
                img.set_px(x, y, [colour[0], colour[1], colour[2], 255]);
            }
        }
        img
    }

    #[test]
    fn a_flat_reference_set_has_one_colour_and_a_two_colour_set_two() {
        let refs = vec![glyph(TEAL), glyph([89, 200, 194])];
        let (p, residual, n) = palette(&refs);
        assert_eq!(p.len(), 1, "{p:?}");
        assert!(p[0].rgb[0].abs_diff(89) <= 1 && p[0].rgb[1].abs_diff(199) <= 1, "{p:?}");
        assert!(residual < 1.0 && n == 2 * 36 * 16);
        let mut two = glyph(TEAL);
        for y in 14..18 {
            for x in 20..56 {
                two.set_px(x, y, [240, 240, 240, 255]);
            }
        }
        let (p, _, _) = palette(&[two]);
        assert_eq!(p.iter().map(|c| c.rgb).collect::<Vec<_>>(), vec![TEAL, [240, 240, 240]]);
        assert!(palette(&[Rgba::new(10, 10)]).0.is_empty(), "no opaque pixels, no palette");
    }

    #[test]
    fn snapping_moves_a_darker_teal_to_the_palette_and_keeps_other_colours() {
        let mut img = glyph([59, 181, 177]);
        img.set_px(19, 20, [59, 181, 177, 90]);
        img.set_px(40, 20, [213, 102, 100, 255]);
        let (out, stats) = snap(&img, &[TEAL], SNAP_DISTANCE);
        assert_eq!(out.px(30, 20), [89, 199, 194, 255]);
        assert_eq!(out.px(19, 20), [89, 199, 194, 90], "a soft edge keeps its alpha");
        assert_eq!(out.px(40, 20), [213, 102, 100, 255], "salmon is no shade of teal");
        assert_eq!((stats.snapped, stats.kept), (36 * 16, 1));
        // a border pixel between two palette colours becomes their blend
        let (out, _) = snap(&Rgba::filled(1, 1, [165, 220, 217, 255]), &[TEAL, [240, 240, 240]], SNAP_DISTANCE);
        let p = out.px(0, 0);
        assert!(p[0] > 120 && p[0] < 200 && p[0] != 240 && p[0] != 89, "{p:?}");
    }

    /// Calibration aid, run by hand on a mod's own files, which are only read: `PACK_EVAL_MOD=<mod root>
    /// PACK_EVAL_RECORDS=<folder of gallery result records> cargo test -p imagegen-core evaluate_pack_counters --
    /// --ignored --nocapture`. For each record's processed counter it prints the pack-match verdict before (the 0.3.5
    /// rule on the recorded output) and after (the 0.3.6 rule on that output's frame 1 snapped, outlined and glowing,
    /// as the 0.3.6 chain makes it from the same fit), and the measured palette and glow.
    #[test]
    #[ignore]
    fn evaluate_pack_counters() {
        use crate::family::Family;
        use crate::vanillamatch as vm;
        let root = std::path::PathBuf::from(std::env::var("PACK_EVAL_MOD").expect("PACK_EVAL_MOD"));
        let records = std::env::var("PACK_EVAL_RECORDS").expect("PACK_EVAL_RECORDS");
        let fs = crate::fsio::ProjectFs::new(&root).unwrap();
        let decode = |rel: &str| crate::pixels::decode(&fs.read(rel, 64 << 20).unwrap(), Some(rel), Default::default()).unwrap().0;
        let pack = crate::pack::load(&fs, "tno").unwrap();
        let entry = pack.resolve(Family::CounterLarge, None).unwrap();
        let refs: Vec<Rgba> = entry.references.iter().map(|r| decode(&r.path)).collect();
        let rows: Vec<crate::pack::ReferenceRow> = refs.iter().map(|r| crate::pack::measure_reference(r).unwrap()).collect();
        let feats: Vec<vm::Features> = rows.iter().map(|r| r.features).collect();
        let v1 = vm::calibrate_min_max(Family::CounterLarge, &feats, 1.0, 2.0, "pack_match_v1", None, &[]).unwrap();
        let v2 = crate::pack::calibrate(Family::CounterLarge, &rows).unwrap();
        let look = measure(&refs, 2, 76);
        let idx = |n: &str| vm::NAMES.iter().position(|x| *x == n).unwrap();
        println!("palette {:?} residual {} pixels {} glow {:?}", look.palette, look.palette_residual, look.palette_pixels, look.glow);
        for n in ["mid_tone_share", "saturation_mean", "top_band_saturation", "alpha_coverage", "value_mean"] {
            let col: Vec<f64> = rows.iter().map(|r| r.features[idx(n)]).collect();
            let sh: Vec<f64> = rows.iter().map(|r| r.shade[idx(n)]).collect();
            let r1 = &v1.ranges[idx(n)];
            let r2 = &v2.ranges[idx(n)];
            println!(
                "{n}: refs {:.4}..{:.4}, one shade moves up to {:.4}; v1 {}..{} checked {}; v2 {}..{} checked {}",
                col.iter().copied().fold(f64::MAX, f64::min),
                col.iter().copied().fold(f64::MIN, f64::max),
                sh.iter().copied().fold(0.0, f64::max),
                r1.min,
                r1.max,
                r1.checked,
                r2.min,
                r2.max,
                r2.checked
            );
        }
        // the pack self-check of every family entry with at least 4 references, under the 0.3.6 rule
        let (mut pass, mut of) = (0, 0);
        for (family, e) in &pack.families {
            let mut sets = vec![e.references.clone()];
            sets.extend(e.kinds.values().filter(|k| !k.references.is_empty()).map(|k| k.references.clone()));
            for set in sets {
                let rows: Vec<crate::pack::ReferenceRow> = set.iter().filter_map(|r| crate::pack::measure_reference(&decode(&r.path))).collect();
                if let Some((p, n)) = crate::pack::leave_one_out(*family, &rows) {
                    println!("self-check {family}: {p} of {n}");
                    pass += p;
                    of += n;
                }
            }
        }
        println!("self-check total: {pass} of {of}");
        let colours: Vec<[u8; 3]> = look.palette.iter().map(|c| c.rgb).collect();
        let mut names: Vec<_> = std::fs::read_dir(&records).unwrap().flatten().map(|e| e.path()).collect();
        names.sort();
        for p in names {
            let rec: serde_json::Value = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
            let rel = rec["process"]["path"].as_str().unwrap();
            let symbol = rec["process"]["counter_symbol"]["symbol"].as_str().unwrap();
            let before = decode(rel);
            let frame1 = before.split_frames(2).unwrap().remove(0);
            let (snapped, stats) = snap(&frame1, &colours, SNAP_DISTANCE);
            let marks = crate::recipes::unit_art::parse_symbol(symbol).unwrap();
            let outlined = crate::recipes::unit_art::outline_strip(&snapped, crate::recipes::unit_art::UnitArtKind::CounterLarge, &marks).unwrap();
            let after = glow(&outlined, 2, &look.glow).unwrap();
            // the whole 0.3.6 chain from the recorded raw candidate: the fit to the references' glyph size too
            let src_rel = rec["generate"]["generation"]["candidates"][0]["path"].as_str().unwrap();
            let src = decode(src_rel);
            let base = crate::profile::ProfileSet::builtin().for_family(Family::CounterLarge).unwrap().clone();
            let profile = crate::pack::apply_profile(&base, "tno", &pack.families[&Family::CounterLarge]);
            let opts = crate::chain::FinishOptions {
                counter_symbol: Some(symbol.to_string()),
                pack_look: Some(crate::chain::PackFinish { pack: "tno".into(), references: vec![], look: look.clone() }),
                ..Default::default()
            };
            let plan = crate::chain::plan_finishing_with(&profile, profile.canvas, &src, false, false, "", &opts).unwrap();
            let (full, steps) = crate::process::run_chain(&src, &plan.ops).unwrap();
            let fit = steps.iter().find_map(|s| match &s.op {
                crate::process::Op::AreaFit { box_width, box_height, .. } => Some([*box_width, *box_height]),
                _ => None,
            });
            let ff = vm::measure(&full).unwrap();
            let fb = vm::measure(&before).unwrap();
            let fa = vm::measure(&after).unwrap();
            let show = |r: &vm::MatchResult| {
                if r.pass {
                    "pass".to_string()
                } else {
                    format!("fail: {}", r.outside.iter().map(|o| format!("{} {}", o.feature, o.value)).collect::<Vec<_>>().join(", "))
                }
            };
            println!(
                "{} | before (v1) {} | v2 rule on the 0.3.5 output {} | after {} | snapped {} kept {} | alpha_coverage {:.4} -> {:.4} sat {:.4} -> {:.4}",
                rel,
                show(&v1.check(&fb)),
                show(&v2.check(&fb)),
                show(&v2.check(&fa)),
                stats.snapped,
                stats.kept,
                fb[idx("alpha_coverage")],
                fa[idx("alpha_coverage")],
                fb[idx("saturation_mean")],
                fa[idx("saturation_mean")]
            );
            println!("  full 0.3.6 chain from {src_rel}: box {fit:?} {} | alpha_coverage {:.4}", show(&v2.check(&ff)), ff[idx("alpha_coverage")]);
        }
    }

    #[test]
    fn the_glow_is_measured_and_drawn_per_frame() {
        let mut r = Rgba::new(152, 42);
        for y in 10..30 {
            for x in 20..50 {
                r.set_px(x, y, [89, 199, 194, 255]);
            }
            r.set_px(100, y, [89, 199, 194, 255]);
        }
        let g = glow(&r, 2, &[[70, 12], [60, 18]]).unwrap();
        assert_eq!(g.px(19, 15), [89, 199, 194, 70]);
        assert_eq!(g.px(18, 15)[3], 12);
        assert_eq!(g.px(101, 15)[3], 60, "frame 2's own glow");
        assert_eq!(g.px(30, 20), [89, 199, 194, 255], "the core is unchanged");
        let look = measure(&[g], 2, 76);
        assert_eq!(look.glow, vec![[70, 12], [60, 18]]);
        assert_eq!(look.palette[0].rgb, TEAL);
        assert!(glow(&r, 2, &[[1, 1]]).is_err());
    }
}
