//! Doctrine icons: units drawn in black with a pale outline and glow, warm accents kept, as vanilla draws them.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, the 105 files of `gfx/interface/doctrines/icons/`, 64x64; measured by the
//! tests below):
//!
//! * The units (tanks, soldiers, ships, aircraft) are solid black silhouettes: inside a dark shape the median
//!   luminance is 0 (75th percentile 27), and its edge pixels are a little lighter (median 31). In the median icon 36
//!   percent of the opaque pixels are black.
//! * Accents (arrows, chevrons, flags, bursts) are warm orange and yellow, and some families paint aircraft and ships
//!   blue or teal. The rule for generated doctrine icons is the black form: units black, warm accents kept.
//! * Around a black shape lies a pale outline and glow: one pixel out the median is luminance 180 (light grey to
//!   white, up to 214) at alpha 136, two pixels out luminance 214 at alpha 37, three pixels out nothing.
//! * Around a coloured shape lies a soft dark shadow instead: luminance 62 at alpha 140 one pixel out, 57 at 65,
//!   41 at 24 and nothing at 4 pixels.
//! * Nothing sits behind the icon: the canvas outside the shapes and their glow is fully transparent.
//!
//! **How the art is made.** The fitted subject's pixels are split into warm accents (orange to yellow hue, saturated
//! and not dark) and everything else. Everything else becomes the black unit fill, with vanilla's lighter edge. Then
//! every clear pixel within four pixels of the shape gets the glow of the nearest shape pixel: the pale outline next
//! to black, the dark shadow next to an accent, at vanilla's alpha for that distance. The subject's own soft edge is
//! laid over it.

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};

/// Version of the doctrine recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// The unit fill: vanilla's median luminance inside a dark shape.
pub const FILL: u8 = 0;
/// The unit fill on its edge pixels (median luminance of the edge of a dark shape).
pub const FILL_EDGE: u8 = 31;
/// Pale outline next to a black shape, at 1 and 2 px: luminance and alpha.
pub const PALE_GLOW: [(f64, f64); 2] = [(180.0, 136.0), (214.0, 37.0)];
/// Dark shadow next to an accent, at 1 to 4 px: luminance and alpha.
pub const DARK_GLOW: [(f64, f64); 4] = [(62.0, 140.0), (57.0, 65.0), (41.0, 24.0), (0.0, 4.0)];

/// How far the glow reaches with alpha 16 and up (alpha 37 two pixels out, nothing three out). The chain fits the
/// subject this much further inside the canvas, so the glow fills the family's margin and never touches the edge.
pub const GLOW_PX: u32 = 2;

/// True for a warm accent colour: hue 15 to 60 degrees (orange to yellow), saturation 0.45 and up, value 0.45 and up.
pub fn is_accent(p: [u8; 4]) -> bool {
    let (r, g, b) = (f64::from(p[0]) / 255.0, f64::from(p[1]) / 255.0, f64::from(p[2]) / 255.0);
    let (mx, mn) = (r.max(g).max(b), r.min(g).min(b));
    if mx < 0.45 || mx <= 0.0 || (mx - mn) / mx < 0.45 {
        return false;
    }
    let d = mx - mn;
    let h = if mx == r {
        60.0 * ((g - b) / d).rem_euclid(6.0)
    } else if mx == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (15.0..=60.0).contains(&h)
}

/// Linear between the measured rings (distance 1, 2 ...), fading to nothing one pixel past the last.
fn ring(table: &[(f64, f64)], d: f64) -> (f64, f64) {
    if d <= 1.0 {
        return table[0];
    }
    let i = d.floor() as usize;
    let Some(&a) = table.get(i - 1) else { return (0.0, 0.0) };
    let b = table.get(i).copied().unwrap_or((a.0, 0.0));
    let f = d - i as f64;
    (a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f)
}

/// The vanilla doctrine look on a fitted subject.
pub fn doctrine_look(img: &Rgba) -> Result<Rgba> {
    let (w, h) = (img.width as i64, img.height as i64);
    let solid = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && img.alpha(x as u32, y as u32) >= 128;
    let accent: Vec<bool> = img.data.chunks_exact(4).map(|p| is_accent([p[0], p[1], p[2], p[3]])).collect();
    if !(0..h).any(|y| (0..w).any(|x| solid(x, y))) {
        return Err(Error::Image("the doctrine icon has no solid shape".into()));
    }
    let mut out = Rgba::new(img.width, img.height);
    const R: i64 = 4;
    for y in 0..h {
        for x in 0..w {
            // glow of the nearest solid pixel (ties: the first in scan order)
            let mut best: Option<(i64, usize)> = None;
            if !solid(x, y) {
                for dy in -R..=R {
                    for dx in -R..=R {
                        let d2 = dx * dx + dy * dy;
                        if d2 > R * R || !solid(x + dx, y + dy) {
                            continue;
                        }
                        let i = ((y + dy) * w + x + dx) as usize;
                        if best.is_none_or(|(bd, bi)| d2 < bd || (d2 == bd && i < bi)) {
                            best = Some((d2, i));
                        }
                    }
                }
            }
            let glow = match best {
                Some((d2, i)) => {
                    let d = (d2 as f64).sqrt();
                    let (l, a) = if accent[i] { ring(&DARK_GLOW, d) } else { ring(&PALE_GLOW, d) };
                    let l = l.round().clamp(0.0, 255.0) as u8;
                    [l, l, l, a.round().clamp(0.0, 255.0) as u8]
                }
                None => [0, 0, 0, 0],
            };
            let p = img.px(x as u32, y as u32);
            let i = (y * w + x) as usize;
            let own = if p[3] == 0 {
                [0, 0, 0, 0]
            } else if accent[i] {
                p
            } else {
                let edge = solid(x, y) && !(solid(x - 1, y) && solid(x + 1, y) && solid(x, y - 1) && solid(x, y + 1));
                let v = if edge { FILL_EDGE } else { FILL };
                [v, v, v, p[3]]
            };
            out.set_px(x as u32, y as u32, over(own, glow));
        }
    }
    Ok(out)
}

/// What the tests compare, over a set of icons.
#[derive(Debug, Clone, PartialEq)]
pub struct DoctrineStats {
    /// Median share of the opaque pixels that are black (luminance below 40).
    pub black_share: f64,
    /// Median luminance of dark (unsaturated) opaque pixels two or more pixels inside the shape.
    pub fill: f64,
    /// Next to a dark shape, 1 and 2 px out: median luminance and alpha.
    pub pale: [[f64; 2]; 2],
    /// Next to a coloured shape, 1 to 3 px out: median alpha.
    pub dark: [f64; 3],
    /// Share of pixels more than 5 px from the shape that are not fully clear.
    pub behind: f64,
}

fn lum(p: [u8; 4]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

fn median(mut v: Vec<f64>) -> f64 {
    if v.is_empty() {
        return 0.0;
    }
    v.sort_by(f64::total_cmp);
    v[v.len() / 2]
}

/// Measure doctrine icons the way the vanilla measurement does. A shape pixel counts as dark when the median
/// luminance of the opaque pixels of its 5x5 neighbourhood is below 50.
pub fn doctrine_stats(imgs: &[&Rgba]) -> DoctrineStats {
    let (mut black, mut fill, mut behind) = (Vec::new(), Vec::new(), Vec::new());
    let mut pale: [[Vec<f64>; 2]; 2] = Default::default();
    let mut dark: [Vec<f64>; 3] = Default::default();
    for img in imgs {
        let (w, h) = (img.width as i64, img.height as i64);
        let solid = |x: i64, y: i64| x >= 0 && y >= 0 && x < w && y < h && img.alpha(x as u32, y as u32) >= 200;
        let n_solid = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).filter(|(x, y)| solid(*x, *y)).count();
        if n_solid < 50 {
            continue;
        }
        let local = |x: i64, y: i64| {
            let mut v = Vec::new();
            for dy in -2..=2 {
                for dx in -2..=2 {
                    if solid(x + dx, y + dy) {
                        v.push(lum(img.px((x + dx) as u32, (y + dy) as u32)));
                    }
                }
            }
            median(v)
        };
        let (mut nb, mut far, mut nfar) = (0usize, 0usize, 0usize);
        for y in 0..h {
            for x in 0..w {
                let p = img.px(x as u32, y as u32);
                if solid(x, y) {
                    if lum(p) < 40.0 {
                        nb += 1;
                    }
                    let sat = i32::from(*p[..3].iter().max().unwrap()) - i32::from(*p[..3].iter().min().unwrap());
                    let deep = (-2..=2).all(|dy: i64| (-2..=2).all(|dx: i64| solid(x + dx, y + dy)));
                    if deep && sat < 40 {
                        fill.push(lum(p));
                    }
                    continue;
                }
                let mut best: Option<(i64, i64, i64)> = None;
                for dy in -6i64..=6 {
                    for dx in -6i64..=6 {
                        let d2 = dx * dx + dy * dy;
                        if solid(x + dx, y + dy) && best.is_none_or(|(bd, _, _)| d2 < bd) {
                            best = Some((d2, x + dx, y + dy));
                        }
                    }
                }
                let Some((d2, sx, sy)) = best else {
                    nfar += 1;
                    if p[3] > 0 {
                        far += 1;
                    }
                    continue;
                };
                let d = (d2 as f64).sqrt().round() as usize;
                if d > 5 {
                    nfar += 1;
                    if p[3] > 0 {
                        far += 1;
                    }
                }
                if !(1..=3).contains(&d) {
                    continue;
                }
                if local(sx, sy) < 50.0 {
                    if d <= 2 {
                        pale[d - 1][0].push(lum(p));
                        pale[d - 1][1].push(f64::from(p[3]));
                    }
                } else {
                    dark[d - 1].push(f64::from(p[3]));
                }
            }
        }
        black.push(nb as f64 / n_solid as f64);
        behind.push(if nfar == 0 { 0.0 } else { far as f64 / nfar as f64 });
    }
    DoctrineStats {
        black_share: median(black),
        fill: median(fill),
        pale: pale.map(|r| r.map(median)),
        dark: dark.map(median),
        behind: median(behind),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Teal arrows over grey rubble, the way the gallery's doctrine art came out, with one orange chevron.
    fn teal_subject() -> Rgba {
        let mut img = Rgba::new(64, 64);
        for y in 10..50u32 {
            for x in 8..40u32 {
                img.set_px(x, y, [60, 140, 150, 255]);
            }
        }
        for y in 20..30u32 {
            for x in 44..56u32 {
                img.set_px(x, y, [240, 140, 40, 255]);
            }
        }
        img
    }

    #[test]
    fn units_turn_black_accents_stay_and_a_pale_glow_surrounds_the_black() {
        let out = doctrine_look(&teal_subject()).unwrap();
        assert_eq!(&out.px(20, 30)[..3], &[0, 0, 0], "the unit is black");
        assert_eq!(out.px(20, 30)[3], 255);
        assert_eq!(&out.px(8, 30)[..3], &[FILL_EDGE; 3], "its edge is a little lighter");
        assert_eq!(out.px(50, 25), [240, 140, 40, 255], "the warm accent keeps its colour");
        let pale = out.px(7, 30);
        assert_eq!((pale[0], pale[3]), (180, 136), "pale outline one pixel out: {pale:?}");
        assert_eq!(out.px(6, 30)[3], 37, "the glow two pixels out");
        assert_eq!(out.px(5, 30)[3], 0, "nothing three pixels out");
        let shadow = out.px(56, 25);
        assert_eq!((shadow[0], shadow[3]), (62, 140), "a dark shadow beside the accent: {shadow:?}");
        assert_eq!(out.px(0, 0), [0, 0, 0, 0], "nothing behind the icon");
        assert_eq!(out, doctrine_look(&teal_subject()).unwrap(), "deterministic");
        assert!(doctrine_look(&Rgba::new(8, 8)).is_err());
    }

    #[test]
    fn accents_are_warm_saturated_and_light() {
        assert!(is_accent([240, 140, 40, 255]));
        assert!(is_accent([230, 200, 60, 255]));
        assert!(!is_accent([60, 140, 150, 255]), "teal is a unit colour");
        assert!(!is_accent([90, 60, 30, 255]), "dark brown is not an accent");
        assert!(!is_accent([180, 170, 160, 255]), "grey is not an accent");
    }

    fn vanilla() -> Vec<Rgba> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let dir = g.join("gfx/interface/doctrines/icons");
        let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        files.iter().filter_map(|p| crate::dds::decode(&std::fs::read(p).ok()?).ok()).filter(|i| (i.width, i.height) == (64, 64)).collect()
    }

    /// Vanilla's own shapes as a model draws them (dark and blue units painted teal, warm accents kept, no glow) come
    /// out of the recipe with vanilla's black fill, pale outline, shadow and clear background. Measured on HOI4
    /// 1.19.4.1: black share 0.36, fill 0, pale outline luminance 180 alpha 136 at 1 px, 214 and 37 at 2 px, shadow
    /// alpha 140, 65 and 24 at 1 to 3 px, nothing behind.
    #[test]
    fn the_look_matches_vanillas_doctrine_icons() {
        let files = vanilla();
        if files.is_empty() {
            eprintln!("SKIPPED doctrine look vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        assert!(files.len() >= 100, "only {} vanilla doctrine icons", files.len());
        let van = doctrine_stats(&files.iter().collect::<Vec<_>>());
        let model: Vec<Rgba> = files
            .iter()
            .map(|f| {
                let mut m = Rgba::new(64, 64);
                for (i, p) in f.data.chunks_exact(4).enumerate() {
                    if p[3] < 128 {
                        continue;
                    }
                    let px = [p[0], p[1], p[2], 255];
                    let c = if is_accent(px) { px } else { [60, 140, 150, 255] };
                    m.data[i * 4..i * 4 + 4].copy_from_slice(&c);
                }
                m
            })
            .collect();
        let before = doctrine_stats(&model.iter().collect::<Vec<_>>());
        let made: Vec<Rgba> = model.iter().map(|m| doctrine_look(m).unwrap()).collect();
        let ours = doctrine_stats(&made.iter().collect::<Vec<_>>());
        eprintln!("doctrine look, medians over {} files\n  vanilla {van:?}\n  before  {before:?}\n  ours    {ours:?}", files.len());
        assert!(ours.black_share >= van.black_share - 0.05, "black share {} vs {}", ours.black_share, van.black_share);
        assert!(ours.fill <= van.fill + 10.0);
        assert!((ours.pale[0][0] - van.pale[0][0]).abs() <= 25.0 && (ours.pale[0][1] - van.pale[0][1]).abs() <= 25.0, "{ours:?} {van:?}");
        assert!((ours.pale[1][1] - van.pale[1][1]).abs() <= 20.0, "{ours:?} {van:?}");
        // the shadow one pixel out; further out vanilla's "coloured" neighbours include its blue and teal units, which
        // the black form paints black, so their glow is pale and the medians are not comparable
        assert!((ours.dark[0] - van.dark[0]).abs() <= 25.0, "shadow at 1 px: {ours:?} {van:?}");
        assert!(ours.behind <= van.behind + 0.01);
    }
}
