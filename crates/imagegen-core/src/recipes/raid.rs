//! Raid icons: any fitted subject recoloured in vanilla's golden gradient with its black outline.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, the 17 `raid_type_icon_*.dds` map icons of
//! `gfx/interface/military_raids/map_icons/`, 32x32, the magenta placeholder `raid_type_icon_unknown` left out; measured
//! by the tests below):
//!
//! * Every pixel of the pictogram lies on one gold ramp: its colour is a function of its luminance alone (see
//!   [`RAMP`]: luminance 128 is (167, 126, 50), 192 is (214, 192, 120), green over red 0.818 and blue over red 0.419 in
//!   the median). Light pixels are pale gold, dark ones brown, and the darkest black.
//! * The luminance of the inside (two or more pixels from the edge) runs from light at the top to dark at the bottom:
//!   the medians of five horizontal bands, top to bottom, are 183, 172, 147, 139 and 112. Across the width it is flat
//!   (138, 146, 140, 136, 135). Its deciles are [`CORE_QUANTILES`] (median 138).
//! * The outline is inside the shape: the pixels at the edge of the opaque shape are black (median (3, 3, 1)), and
//!   outside it there is almost nothing (alpha 7 in the median one pixel out, mean 31).
//!
//! **How the art is made.** The subject's own shading is kept as an order, not as colours: each inside pixel gets a
//! score that mixes its luminance rank inside the subject with its height (top high), the scores are ranked again,
//! and the rank picks a luminance from vanilla's deciles. So the output has vanilla's distribution of light and dark
//! and its top-to-bottom gradient, whatever colours the model used. Luminance becomes colour through vanilla's ramp,
//! the edge becomes the black outline, and the soft rim outside the shape stays black at its own alpha.
//!
//! Everything is a pure function of the pixels, deterministic on every platform.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Version of the raid recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// Vanilla's ramp: the median colour of the opaque raid icon pixels whose luminance lies within 8 of 0, 16, 32 ... 224.
pub const RAMP: [[u8; 3]; 15] = [
    [0, 0, 0],
    [19, 15, 7],
    [40, 32, 16],
    [57, 48, 25],
    [80, 64, 30],
    [98, 78, 39],
    [119, 94, 45],
    [138, 110, 54],
    [167, 126, 50],
    [181, 141, 65],
    [192, 157, 81],
    [205, 173, 97],
    [214, 192, 120],
    [229, 211, 141],
    [234, 224, 155],
];

/// Deciles (0, 10 ... 100 percent) of the luminance of vanilla's inside pixels.
pub const CORE_QUANTILES: [f64; 11] = [0.0, 42.0, 84.0, 106.0, 126.0, 138.0, 151.0, 159.0, 173.0, 187.0, 223.0];

/// Vanilla's outline colour: the median of the opaque edge pixels.
pub const OUTLINE: [u8; 3] = [3, 3, 1];

/// How much of an inside pixel's score is its height (top light, bottom dark); the rest is its own luminance rank.
/// Chosen so the five band medians follow vanilla's (see the tests).
pub const HEIGHT_WEIGHT: f64 = 0.28;

/// A colour gradient fitted to one vanilla set: the ramp from luminance to colour, the deciles of the inside's
/// luminance, the edge colour, how strongly height drives the tone, and the black rim just outside the shape.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GradientStyle {
    /// Median colour of the pixels whose luminance lies within 8 of 0, 16, 32 ...
    pub ramp: &'static [[u8; 3]],
    /// Deciles (0, 10 ... 100 percent) of the inside's luminance.
    pub quantiles: [f64; 11],
    /// Colour of the shape's edge pixels.
    pub outline: [u8; 3],
    /// Share of an inside pixel's score that is its height (top light); the rest is its own luminance rank.
    pub height_weight: f64,
    /// Alpha of a black rim on the clear pixels next to the shape (0 for none).
    pub rim_alpha: u8,
}

/// Vanilla's raid map icons.
pub const RAID: GradientStyle =
    GradientStyle { ramp: &RAMP, quantiles: CORE_QUANTILES, outline: OUTLINE, height_weight: HEIGHT_WEIGHT, rim_alpha: 0 };

/// Colour of a luminance on vanilla's raid ramp (linear between the measured points, the last one held).
pub fn ramp(l: f64) -> [u8; 3] {
    ramp_of(&RAID, l)
}

/// Colour of a luminance on a style's ramp.
pub fn ramp_of(style: &GradientStyle, l: f64) -> [u8; 3] {
    let r = style.ramp;
    let t = (l / 16.0).clamp(0.0, (r.len() - 1) as f64);
    let i = (t.floor() as usize).min(r.len() - 2);
    let f = t - i as f64;
    let (a, b) = (r[i], r[i + 1]);
    [0, 1, 2].map(|c| (f64::from(a[c]) + (f64::from(b[c]) - f64::from(a[c])) * f).round() as u8)
}

fn quantile(q: &[f64; 11], at: f64) -> f64 {
    let t = (at * 10.0).clamp(0.0, 10.0);
    let i = (t.floor() as usize).min(9);
    let f = t - i as f64;
    q[i] + (q[i + 1] - q[i]) * f
}

fn luminance(p: [u8; 4]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// Ranks of `v` scaled to 0..=1 (ties keep their order of appearance, so the result is deterministic).
fn ranks(v: &[f64]) -> Vec<f64> {
    let mut idx: Vec<usize> = (0..v.len()).collect();
    idx.sort_by(|a, b| v[*a].total_cmp(&v[*b]).then(a.cmp(b)));
    let mut out = vec![0.0; v.len()];
    let d = (v.len().max(2) - 1) as f64;
    for (r, i) in idx.into_iter().enumerate() {
        out[i] = r as f64 / d;
    }
    out
}

/// The raid icon look on a fitted subject of any size: vanilla's gold gradient, black outline.
pub fn gold_gradient(img: &Rgba) -> Result<Rgba> {
    gradient_map(img, &RAID)
}

/// Recolour a fitted subject with a vanilla gradient style.
pub fn gradient_map(img: &Rgba, style: &GradientStyle) -> Result<Rgba> {
    let (w, h) = (img.width as usize, img.height as usize);
    let solid: Vec<bool> = img.data.chunks_exact(4).map(|p| p[3] >= 128).collect();
    let n_solid = solid.iter().filter(|s| **s).count();
    if n_solid < 4 {
        return Err(Error::Image("the picture has no solid shape to colour".into()));
    }
    let at = |x: i64, y: i64| x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h && solid[y as usize * w + x as usize];
    // the edge of the solid shape: a solid pixel with a 4-neighbour that is not solid
    let edge: Vec<bool> = (0..w * h)
        .map(|i| {
            let (x, y) = ((i % w) as i64, (i / w) as i64);
            solid[i] && !(at(x - 1, y) && at(x + 1, y) && at(x, y - 1) && at(x, y + 1))
        })
        .collect();
    let rows: Vec<usize> = (0..h).filter(|y| (0..w).any(|x| solid[y * w + x])).collect();
    let (y0, y1) = (rows[0] as f64, *rows.last().expect("a solid row") as f64);
    let core: Vec<usize> = (0..w * h).filter(|i| solid[*i] && !edge[*i]).collect();
    let mut out = Rgba::new(img.width, img.height);
    let mut tone = vec![quantile(&style.quantiles, 0.5); w * h];
    if core.len() >= 4 {
        let lum: Vec<f64> = core.iter().map(|i| luminance(img.px((i % w) as u32, (i / w) as u32))).collect();
        let lr = ranks(&lum);
        let score: Vec<f64> = core
            .iter()
            .zip(&lr)
            .map(|(i, r)| {
                let t = if y1 > y0 { ((i / w) as f64 - y0) / (y1 - y0) } else { 0.5 };
                (1.0 - style.height_weight) * r + style.height_weight * (1.0 - t)
            })
            .collect();
        for (i, q) in core.iter().zip(ranks(&score)) {
            tone[*i] = quantile(&style.quantiles, q);
        }
    }
    for i in 0..w * h {
        let (x, y) = ((i % w) as i64, (i / w) as i64);
        let p = img.data[i * 4 + 3];
        if !solid[i] && style.rim_alpha > 0 && (at(x - 1, y) || at(x + 1, y) || at(x, y - 1) || at(x, y + 1)) {
            out.data[i * 4..i * 4 + 4].copy_from_slice(&[0, 0, 0, p.max(style.rim_alpha)]);
            continue;
        }
        if p == 0 {
            continue;
        }
        let rgb = if solid[i] && !edge[i] { ramp_of(style, tone[i]) } else { style.outline };
        out.data[i * 4..i * 4 + 4].copy_from_slice(&[rgb[0], rgb[1], rgb[2], p]);
    }
    Ok(out)
}

/// What the tests compare: the median colour of the inside, the luminance of five bands top to bottom, the
/// luminance deciles of the inside and the median luminance of the edge.
#[derive(Debug, Clone, PartialEq)]
pub struct RaidStats {
    /// Median red, green and blue of the inside pixels.
    pub core_rgb: [f64; 3],
    /// Median inside luminance of five horizontal bands, top to bottom.
    pub bands: [f64; 5],
    /// Median inside luminance.
    pub core_median: f64,
    /// Median edge luminance.
    pub edge: f64,
    /// Median green over red and blue over red of the inside.
    pub ratios: [f64; 2],
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Measure raid icons the way the vanilla measurement does (inside: opaque, two or more pixels from the edge).
pub fn raid_stats(imgs: &[&Rgba]) -> RaidStats {
    let (mut r, mut g, mut b, mut l, mut e, mut gr, mut br) = (vec![], vec![], vec![], vec![], vec![], vec![], vec![]);
    let mut bands: [Vec<f64>; 5] = Default::default();
    for img in imgs {
        let (w, h) = (img.width as i64, img.height as i64);
        let solid = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && img.alpha(x as u32, y as u32) >= 128;
        let mut ys = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if solid(x, y) {
                    ys.push(y);
                }
            }
        }
        let (Some(&y0), Some(&y1)) = (ys.iter().min(), ys.iter().max()) else { continue };
        for y in 0..h {
            for x in 0..w {
                if !solid(x, y) {
                    continue;
                }
                let p = img.px(x as u32, y as u32);
                let lum = luminance(p);
                let is_edge = !(solid(x - 1, y) && solid(x + 1, y) && solid(x, y - 1) && solid(x, y + 1));
                if is_edge {
                    e.push(lum);
                    continue;
                }
                let deep = (-1..=1).all(|dy: i64| (-1..=1).all(|dx: i64| solid(x + dx, y + dy)));
                if !deep {
                    continue;
                }
                r.push(f64::from(p[0]));
                g.push(f64::from(p[1]));
                b.push(f64::from(p[2]));
                l.push(lum);
                if p[0] > 0 {
                    gr.push(f64::from(p[1]) / f64::from(p[0]));
                    br.push(f64::from(p[2]) / f64::from(p[0]));
                }
                let t = if y1 > y0 { (y - y0) as f64 / (y1 - y0) as f64 } else { 0.0 };
                bands[((t * 5.0) as usize).min(4)].push(lum);
            }
        }
    }
    RaidStats {
        core_rgb: [median(r), median(g), median(b)],
        bands: bands.map(median),
        core_median: median(l),
        edge: median(e),
        ratios: [median(gr), median(br)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A red stick of dynamite with a dark band, the way the gallery's raid art came out (cut out, red and brown).
    fn red_subject() -> Rgba {
        let mut img = Rgba::new(32, 32);
        for y in 4..28u32 {
            for x in 10..22u32 {
                let band = (14..17).contains(&y);
                let shade = 150 - (y as i32 - 4) * 3;
                let px = if band { [40, 20, 10, 255] } else { [shade as u8, 30, 15, 255] };
                img.set_px(x, y, px);
            }
        }
        img
    }

    #[test]
    fn every_inside_pixel_lies_on_the_gold_ramp_and_the_edge_is_black() {
        let out = gold_gradient(&red_subject()).unwrap();
        assert_eq!(out.px(0, 0)[3], 0, "the background stays clear");
        assert_eq!(&out.px(10, 10)[..3], &OUTLINE, "the left edge is the outline");
        let inside = out.px(15, 10);
        let on_ramp = (0..=240).any(|l| ramp(f64::from(l)).iter().zip(&inside[..3]).all(|(a, b)| a.abs_diff(*b) <= 1));
        assert!(on_ramp, "inside pixels are on the ramp: {inside:?}");
        let s = raid_stats(&[&out]);
        assert!(s.bands[0] > s.bands[4] + 30.0, "light at the top, dark at the bottom: {s:?}");
        assert_eq!(out, gold_gradient(&red_subject()).unwrap(), "deterministic");
        assert!(gold_gradient(&Rgba::new(8, 8)).is_err());
    }

    #[test]
    fn the_ramp_runs_from_black_to_pale_gold() {
        assert_eq!(ramp(0.0), [0, 0, 0]);
        assert_eq!(ramp(128.0), [167, 126, 50]);
        assert_eq!(ramp(250.0), [234, 224, 155]);
        assert_eq!(ramp(136.0), [174, 134, 58]);
    }

    fn vanilla() -> Vec<Rgba> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let dir = g.join("gfx/interface/military_raids/map_icons");
        let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        files
            .iter()
            .filter(|p| {
                let n = p.file_name().unwrap().to_string_lossy();
                n.starts_with("raid_type_icon_") && !n.contains("unknown")
            })
            .filter_map(|p| crate::dds::decode(&std::fs::read(p).ok()?).ok())
            .filter(|i| (i.width, i.height) == (32, 32))
            .collect()
    }

    /// Vanilla's own shapes, recoloured the way a model draws them (a red-brown body with its own shading kept), go
    /// through the recipe and come out with vanilla's colour statistics. Measured on HOI4 1.19.4.1: inside median
    /// colour (176, 135, 64), band medians 183, 172, 147, 139, 112, edge luminance 3.
    #[test]
    fn the_gradient_matches_vanillas_colour_statistics() {
        let files = vanilla();
        if files.is_empty() {
            eprintln!("SKIPPED raid gradient vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        assert!(files.len() >= 15, "only {} vanilla raid icons", files.len());
        let van = raid_stats(&files.iter().collect::<Vec<_>>());
        let model: Vec<Rgba> = files
            .iter()
            .map(|f| {
                let mut m = f.clone();
                for p in m.data.chunks_exact_mut(4) {
                    let l = luminance([p[0], p[1], p[2], 255]);
                    p[0] = (60.0 + 0.7 * l).min(255.0) as u8;
                    p[1] = (20.0 + 0.2 * l) as u8;
                    p[2] = (10.0 + 0.1 * l) as u8;
                }
                m
            })
            .collect();
        let before = raid_stats(&model.iter().collect::<Vec<_>>());
        let made: Vec<Rgba> = model.iter().map(|m| gold_gradient(m).unwrap()).collect();
        let ours = raid_stats(&made.iter().collect::<Vec<_>>());
        eprintln!("raid gold gradient, medians over {} files\n  vanilla {van:?}\n  before  {before:?}\n  ours    {ours:?}", files.len());
        for c in 0..3 {
            assert!((ours.core_rgb[c] - van.core_rgb[c]).abs() <= 12.0, "channel {c}: {ours:?} vs {van:?}");
        }
        assert!((ours.core_median - van.core_median).abs() <= 10.0);
        for (o, v) in ours.bands.iter().zip(van.bands) {
            assert!((o - v).abs() <= 20.0, "bands {:?} vs {:?}", ours.bands, van.bands);
        }
        assert!(ours.bands[0] > ours.bands[4] + 40.0);
        assert!((ours.ratios[0] - van.ratios[0]).abs() <= 0.06 && (ours.ratios[1] - van.ratios[1]).abs() <= 0.08, "{ours:?} {van:?}");
        assert!(ours.edge <= 10.0 && van.edge <= 10.0, "black outline: ours {}, vanilla {}", ours.edge, van.edge);
    }
}
