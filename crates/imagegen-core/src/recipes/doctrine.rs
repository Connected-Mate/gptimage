//! Doctrine icons: ground units in black with a pale outline and glow, aircraft, ships and accents in simple colour, as
//! vanilla draws them.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, the 105 files of `gfx/interface/doctrines/icons/`, 64x64; measured by the
//! tests below):
//!
//! * The units (tanks, soldiers, ships, aircraft) are solid black silhouettes: inside a dark shape the median
//!   luminance is 0 (75th percentile 27), and its edge pixels are a little lighter (median 31). In the median icon 36
//!   percent of the opaque pixels are black.
//! * Colour is simple and sparse: in the median icon 22 percent of the opaque pixels are warm (arrows, chevrons, flags,
//!   bursts: hue 25 in the median, 18 to 47 at the 5th and 95th percentile, saturation 0.71, value 0.90), 19 percent are
//!   cool (aircraft in sky blue, hue about 218, ships and some vehicles in teal, hue about 190: the 5th to 95th
//!   percentile is 185 to 219, saturation a flat 0.36 to 0.46, value 0.36 to 0.99) and 18 percent are pale grey or
//!   white highlights. 98 percent of the icons carry some colour, 72 percent a warm accent and 67 percent a cool unit.
//!   Soldiers, tanks and guns are black. Green, red and purple are practically absent (under 1 percent).
//! * A coloured unit is not flat: its value falls from 0.77 in the top third of the colour's box to 0.58 in the
//!   bottom third, its saturation staying 0.41.
//! * Around a black shape lies a pale outline and glow: one pixel out the median is luminance 180 (light grey to
//!   white, up to 214) at alpha 136, two pixels out luminance 214 at alpha 37, three pixels out nothing.
//! * Around a coloured shape lies a soft dark shadow instead: luminance 62 at alpha 140 one pixel out, 57 at 65,
//!   41 at 24 and nothing at 4 pixels.
//! * Nothing sits behind the icon: the canvas outside the shapes and their glow is fully transparent.
//!
//! **How the art is made.** The fitted subject's pixels are split into warm accents (orange to yellow hue, saturated
//! and not dark), cool units (blue to teal hue, saturated and not dark) and everything else. Everything else becomes
//! the black unit fill, with vanilla's lighter edge. A cool unit keeps its drawing but gets vanilla's blue range (hue
//! between 185 and 225, saturation 0.34 to 0.48) and vanilla's soft fall of value from top to bottom. Then every clear
//! pixel within four pixels of the shape gets the glow of the nearest shape pixel: the pale outline next to black, the
//! dark shadow next to a coloured shape, at vanilla's alpha for that distance. The subject's own soft edge is laid
//! over it.

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};

/// Version of the doctrine recipe (bump when an output could change). 2: the subject is fitted 3 px inside the margin, not 2.
/// 3: blue and teal units keep a vanilla-toned colour instead of turning black.
pub const RECIPE_VERSION: u32 = 3;

/// The unit fill: vanilla's median luminance inside a dark shape.
pub const FILL: u8 = 0;
/// The unit fill on its edge pixels (median luminance of the edge of a dark shape).
pub const FILL_EDGE: u8 = 31;
/// Pale outline next to a black shape, at 1 and 2 px: luminance and alpha.
pub const PALE_GLOW: [(f64, f64); 2] = [(180.0, 136.0), (214.0, 37.0)];
/// Dark shadow next to an accent, at 1 to 4 px: luminance and alpha.
pub const DARK_GLOW: [(f64, f64); 4] = [(62.0, 140.0), (57.0, 65.0), (41.0, 24.0), (0.0, 4.0)];

/// How far the glow reaches with alpha 16 and up: the pale outline to 2 px (alpha 37 two pixels out, 28 at a
/// knight's move), the dark shadow around an accent to 3 px (alpha 24 three pixels out, 4 at four). The chain fits the
/// subject this much further inside the canvas than the margin the `margin` check reads, so neither glow eats it.
/// Until 0.3.4 this was 2, and a doctrine icon whose accent reached the fitted box kept only 1 px
/// (`GFX-MARGIN` on the gallery's battle_line_broadside). [`glow_reach`] computes it from the rings.
pub const GLOW_PX: u32 = 3;

/// The farthest whole-pixel offset at which either glow still has alpha 16 or more, from [`PALE_GLOW`] and
/// [`DARK_GLOW`] as [`doctrine_look`] spreads them (the Euclidean distance of every offset within the reach).
pub fn glow_reach() -> u32 {
    let mut reach = 0;
    for table in [&PALE_GLOW[..], &DARK_GLOW[..]] {
        for dx in 0..8i64 {
            for dy in 0..=dx {
                let d = ((dx * dx + dy * dy) as f64).sqrt();
                if d > 0.0 && ring(table, d).1.round() >= 16.0 {
                    reach = reach.max(dx as u32);
                }
            }
        }
    }
    reach
}

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

/// Hue, saturation and value of a pixel (hue in degrees).
fn hsv(p: [u8; 4]) -> (f64, f64, f64) {
    crate::styledesc::hsv(p[0], p[1], p[2])
}

/// True for a cool unit colour: hue 170 to 260 degrees (teal to blue), saturation 0.3 and up, value 0.3 and up. Vanilla's
/// aircraft and ships are 185 to 219 degrees at saturation 0.36 to 0.46.
pub fn is_cool(p: [u8; 4]) -> bool {
    let (h, s, v) = hsv(p);
    (170.0..=260.0).contains(&h) && s >= 0.3 && v >= 0.3
}

/// Vanilla's cool hue range, the saturation range and the fall of value (0.82 at the top of the colour's box, 0.29 less at
/// the bottom, measured as 0.77 and 0.58 in the top and bottom thirds).
pub const COOL_HUE: (f64, f64) = (185.0, 225.0);
/// Vanilla's cool saturation range (5th to 95th percentile 0.36 to 0.46, a little wider here).
pub const COOL_SAT: (f64, f64) = (0.34, 0.48);
/// Value of a cool unit at the top of its box and the drop to the bottom.
pub const COOL_VALUE: (f64, f64) = (0.82, 0.29);

fn hsv_to_rgb(h: f64, s: f64, v: f64) -> [u8; 3] {
    let c = v * s;
    let hp = (h / 60.0).rem_euclid(6.0);
    let x = c * (1.0 - (hp.rem_euclid(2.0) - 1.0).abs());
    let (r, g, b) = match hp as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    [r, g, b].map(|t| ((t + m) * 255.0).round().clamp(0.0, 255.0) as u8)
}

/// A cool pixel in vanilla's blue: hue and saturation into vanilla's ranges, the value on vanilla's fall from the top
/// (`t` 0) to the bottom (`t` 1) of the cool shapes' box, with a third of the drawing's own light and shade kept.
fn cool_tone(p: [u8; 4], t: f64, mean_v: f64) -> [u8; 4] {
    let (h, s, v) = hsv(p);
    let target = COOL_VALUE.0 - COOL_VALUE.1 * t;
    let value = (target + 0.35 * (v - mean_v)).clamp(0.3, 1.0);
    let c = hsv_to_rgb(h.clamp(COOL_HUE.0, COOL_HUE.1), s.clamp(COOL_SAT.0, COOL_SAT.1), value);
    [c[0], c[1], c[2], p[3]]
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
    let cool: Vec<bool> = img.data.chunks_exact(4).enumerate().map(|(i, p)| !accent[i] && p[3] >= 128 && is_cool([p[0], p[1], p[2], p[3]])).collect();
    // the box of the cool shapes (for the fall of value) and their mean value
    let (mut y0, mut y1, mut vs, mut vn) = (h, 0i64, 0.0, 0usize);
    for i in (0..cool.len()).filter(|i| cool[*i]) {
        let y = i as i64 / w;
        y0 = y0.min(y);
        y1 = y1.max(y);
        let p = &img.data[i * 4..i * 4 + 4];
        vs += hsv([p[0], p[1], p[2], 255]).2;
        vn += 1;
    }
    let cool_mean_v = if vn > 0 { vs / vn as f64 } else { 0.6 };
    let span = (y1 - y0).max(1) as f64;
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
                    let (l, a) = if accent[i] || cool[i] { ring(&DARK_GLOW, d) } else { ring(&PALE_GLOW, d) };
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
            } else if cool[i] {
                cool_tone(p, ((i as i64 / w - y0) as f64 / span).clamp(0.0, 1.0), cool_mean_v)
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
    /// Median share of the opaque pixels in warm accent colours ([`is_accent`]).
    pub warm_share: f64,
    /// Median share of the opaque pixels in cool unit colours ([`is_cool`]).
    pub cool_share: f64,
    /// Mean hue (degrees), saturation and value of the cool pixels over all the icons that have some.
    pub cool_tone: [f64; 3],
    /// Mean value of the cool pixels in the top and the bottom third of their box, over the icons that have enough.
    pub cool_fall: [f64; 2],
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
    let (mut warm, mut coolv) = (Vec::new(), Vec::new());
    let (mut tone, mut fall) = (Vec::<[f64; 3]>::new(), Vec::<[f64; 2]>::new());
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
        let (mut nb, mut far, mut nfar, mut nw, mut nc) = (0usize, 0usize, 0usize, 0usize, 0usize);
        let mut cools: Vec<(i64, [f64; 3])> = Vec::new();
        for y in 0..h {
            for x in 0..w {
                let p = img.px(x as u32, y as u32);
                if solid(x, y) {
                    if lum(p) < 40.0 {
                        nb += 1;
                    }
                    if is_accent(p) {
                        nw += 1;
                    } else if is_cool(p) {
                        nc += 1;
                        let (hh, ss, vv) = hsv(p);
                        cools.push((y, [hh, ss, vv]));
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
        warm.push(nw as f64 / n_solid as f64);
        coolv.push(nc as f64 / n_solid as f64);
        if nc >= 30 {
            let n = nc as f64;
            tone.push([0, 1, 2].map(|k| cools.iter().map(|c| c.1[k]).sum::<f64>() / n));
            let (lo, hi) = (cools.iter().map(|c| c.0).min().unwrap(), cools.iter().map(|c| c.0).max().unwrap());
            if hi - lo >= 8 {
                let third = (hi - lo) as f64 / 3.0;
                let part = |from: f64, to: f64| {
                    let v: Vec<f64> = cools.iter().filter(|c| (c.0 - lo) as f64 >= from && ((c.0 - lo) as f64) <= to).map(|c| c.1[2]).collect();
                    if v.is_empty() {
                        None
                    } else {
                        Some(v.iter().sum::<f64>() / v.len() as f64)
                    }
                };
                if let (Some(t), Some(b)) = (part(0.0, third), part(2.0 * third, 3.0 * third)) {
                    fall.push([t, b]);
                }
            }
        }
        behind.push(if nfar == 0 { 0.0 } else { far as f64 / nfar as f64 });
    }
    DoctrineStats {
        black_share: median(black),
        fill: median(fill),
        pale: pale.map(|r| r.map(median)),
        dark: dark.map(median),
        behind: median(behind),
        warm_share: median(warm),
        cool_share: median(coolv),
        cool_tone: if tone.is_empty() { [0.0; 3] } else { [0, 1, 2].map(|k| tone.iter().map(|t| t[k]).sum::<f64>() / tone.len() as f64) },
        cool_fall: if fall.is_empty() { [0.0; 2] } else { [0, 1].map(|k| fall.iter().map(|t| t[k]).sum::<f64>() / fall.len() as f64) },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dark grey units on the left (a tank or a soldier) and one orange chevron.
    fn teal_subject() -> Rgba {
        let mut img = Rgba::new(64, 64);
        for y in 10..50u32 {
            for x in 8..40u32 {
                img.set_px(x, y, [70, 76, 72, 255]);
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
        assert_eq!(&out.px(48, 36)[..3], &[0, 0, 0], "the unit is black");
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

    /// A sky-blue aircraft and a teal ship keep a simple colour in vanilla's blue range, with vanilla's fall of value from
    /// top to bottom and its dark shadow, while dark units next to them stay black and white or grey parts turn black.
    #[test]
    fn aircraft_and_ships_keep_a_vanilla_toned_blue() {
        let mut img = Rgba::new(64, 64);
        for y in 8..28u32 {
            for x in 6..30u32 {
                img.set_px(x, y, [110, 170, 250, 255]); // a saturated model blue, hue 215
            }
        }
        for y in 34..54u32 {
            for x in 6..30u32 {
                img.set_px(x, y, [50, 170, 160, 255]); // teal
            }
        }
        for y in 20..50u32 {
            for x in 40..58u32 {
                img.set_px(x, y, [200, 200, 200, 255]); // a pale grey part
            }
        }
        let out = doctrine_look(&img).unwrap();
        let plane_top = out.px(16, 9);
        let plane_bottom = out.px(16, 52);
        for p in [plane_top, plane_bottom, out.px(16, 20)] {
            let (h, s, v) = hsv(p);
            assert!((COOL_HUE.0 - 1.0..=COOL_HUE.1 + 1.0).contains(&h), "hue {h} of {p:?}");
            assert!((COOL_SAT.0 - 0.03..=COOL_SAT.1 + 0.03).contains(&s), "saturation {s} of {p:?}");
            assert!((0.3..=1.0).contains(&v));
        }
        assert!(hsv(plane_top).2 > hsv(plane_bottom).2 + 0.15, "light at the top, darker at the bottom: {plane_top:?} {plane_bottom:?}");
        assert_eq!(&out.px(48, 36)[..3], &[0, 0, 0][..], "the pale grey part is the black unit fill");
        assert_eq!(out.px(16, 20)[3], 255);
        let shadow = out.px(16, 7);
        assert_eq!((shadow[0], shadow[3]), (62, 140), "the dark shadow of a coloured shape: {shadow:?}");
        assert_eq!(out.px(0, 0), [0, 0, 0, 0]);
    }

    #[test]
    fn accents_are_warm_saturated_and_light() {
        assert!(is_accent([240, 140, 40, 255]));
        assert!(is_accent([230, 200, 60, 255]));
        assert!(!is_accent([60, 140, 150, 255]), "teal is a unit colour, not an accent");
        assert!(is_cool([60, 140, 150, 255]) && is_cool([110, 170, 250, 255]), "teal and sky blue are cool unit colours");
        assert!(!is_cool([240, 140, 40, 255]) && !is_cool([70, 76, 72, 255]) && !is_cool([20, 28, 60, 255]), "orange, grey and near black are not");
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

    /// Vanilla's own shapes as a model draws them (black units painted dark grey, pale parts light grey, aircraft and
    /// ships in a saturated model blue or teal, warm accents kept, no glow) come out of the recipe with vanilla's black
    /// fill, pale outline, shadow, clear background and simple colour. Measured on HOI4 1.19.4.1: black share 0.39,
    /// warm share 0.21, cool share 0.19, cool hue 185 to 219 at saturation 0.41, value 0.77 at the top and 0.58 at the
    /// bottom, pale outline luminance 180 alpha 136 at 1 px, 214 and 37 at 2 px, shadow alpha 140, 65 and 24 at 1 to 3
    /// px, nothing behind.
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
                    let c = if is_accent(px) {
                        px
                    } else if is_cool(px) {
                        let (h, _, _) = hsv(px);
                        if h > 205.0 {
                            [90, 150, 240, 255]
                        } else {
                            [40, 160, 160, 255]
                        }
                    } else if lum(px) < 60.0 {
                        [30, 30, 34, 255]
                    } else {
                        [190, 190, 185, 255]
                    };
                    m.data[i * 4..i * 4 + 4].copy_from_slice(&c);
                }
                m
            })
            .collect();
        let before = doctrine_stats(&model.iter().collect::<Vec<_>>());
        let made: Vec<Rgba> = model.iter().map(|m| doctrine_look(m).unwrap()).collect();
        let ours = doctrine_stats(&made.iter().collect::<Vec<_>>());
        eprintln!(
            "doctrine look, medians over {} files
  vanilla {van:?}
  before  {before:?}
  ours    {ours:?}",
            files.len()
        );
        assert!(ours.black_share >= van.black_share - 0.05, "black share {} vs {}", ours.black_share, van.black_share);
        assert!(ours.fill <= van.fill + 10.0);
        assert!((ours.pale[0][0] - van.pale[0][0]).abs() <= 25.0 && (ours.pale[0][1] - van.pale[0][1]).abs() <= 25.0, "{ours:?} {van:?}");
        assert!((ours.pale[1][1] - van.pale[1][1]).abs() <= 20.0, "{ours:?} {van:?}");
        // the shadow one pixel out around a coloured shape (accents and now also blue and teal units)
        assert!((ours.dark[0] - van.dark[0]).abs() <= 25.0, "shadow at 1 px: {ours:?} {van:?}");
        assert!(ours.behind <= van.behind + 0.01);
        // simple colour: the cool and warm shares survive, in vanilla's blue range and with its fall of value
        assert!((ours.cool_share - van.cool_share).abs() <= 0.06, "cool share {} vs {}", ours.cool_share, van.cool_share);
        assert!((ours.warm_share - van.warm_share).abs() <= 0.06, "warm share {} vs {}", ours.warm_share, van.warm_share);
        assert!((COOL_HUE.0 - 1.0..=COOL_HUE.1 + 1.0).contains(&ours.cool_tone[0]), "hue {:?}", ours.cool_tone);
        assert!((ours.cool_tone[1] - van.cool_tone[1]).abs() <= 0.08, "saturation {:?} vs {:?}", ours.cool_tone, van.cool_tone);
        assert!(ours.cool_fall[0] > ours.cool_fall[1] + 0.1, "value falls from top to bottom: {:?}", ours.cool_fall);
        assert!(
            (ours.cool_fall[0] - van.cool_fall[0]).abs() <= 0.12 && (ours.cool_fall[1] - van.cool_fall[1]).abs() <= 0.12,
            "{:?} vs {:?}",
            ours.cool_fall,
            van.cool_fall
        );
    }
}
