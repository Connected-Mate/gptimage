//! Technology picture overlays: the tier numeral (I, II, III ...) and the blue gem of the capital-ship pictures.
//!
//! **What vanilla does** (HOI4 1.19.4.1). The numerals are baked into the textures. The technology tree's item
//! (`interface/countrytechtreeview.gui`, for example the `iconType` named `Icon` at `position = { x=3 y=3 }` with
//! `spriteType = "GFX_technology_medium"`) draws one sprite and no numeral layer, no `.gfx` or `.gui` file names a
//! numeral or tier sprite, and the sprites themselves point at single textures (`interface/Technologies.gfx`:
//! `GFX_basic_ship_hull_heavy_medium` is `gfx/interface/technologies/basic_battleship.dds`). The numeral is in the
//! pixels of those textures:
//!
//! * 270 of the 1318 textures of `gfx/interface/technologies/` carry gold serif numerals: 8 of the 269 at 146x54, 13
//!   of the 55 at 131x52, 168 of the 328 at 64x64, and most of the wide capital-ship pictures (285x56 and the like).
//! * One `I` is 8 px wide and 13 px tall: two rows of serif at the top and at the bottom across the full width and a
//!   3 px stem between them. Numerals are set 1 px apart. Their gold runs down the rows from (182, 156, 102) and
//!   (194, 164, 105) at the top to (162, 137, 82) and (183, 156, 87) in the bottom serif ([`GOLD_ROWS`]).
//! * A black shadow surrounds them: alpha 143 one pixel out, 27 to 36 two pixels out, 2 to 5 three pixels out.
//! * On 146x54 pictures the numerals sit in the top right, their middle at x 121 and their top at y 5; on 131x52
//!   their middle is at x 119 and their top at y 3.
//! * Battleship, cruiser and carrier pictures put a blue gem before the numeral: 11 px wide, 14 px tall, its top at
//!   the numeral's top, 2 px before it. Its upper-left facet is light blue (117, 184, 224), the upper-right and
//!   lower-left facets blue (53, 107, 155), the lower-right facet dark.
//!
//! So the overlay is stamped into the finished picture, in vanilla's place, glyphs and colours; no sprite or script
//! changes are needed.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};

/// Version of the overlay (bump when an output could change).
pub const OVERLAY_VERSION: u32 = 1;

/// What to stamp on a technology picture.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TechOverlay {
    /// The tier, written as a Roman numeral: 1 to 10 (I to X).
    pub tier: u32,
    /// Put vanilla's blue gem before the numeral, as the battleship, cruiser and carrier pictures have it.
    #[serde(default)]
    pub gem: bool,
    /// Top-left corner of the badge (the gem, or the numeral when there is no gem). Omit for vanilla's place: the
    /// numeral's middle at x 121 and top at y 5 on 146x54, x 119 and y 3 on 131x52, and 83 and 9 percent of the
    /// canvas otherwise.
    #[serde(default)]
    pub position: Option<[u32; 2]>,
}

/// Height of a numeral.
pub const GLYPH_HEIGHT: u32 = 13;
/// Space between numerals.
pub const GLYPH_GAP: u32 = 1;
/// Gem size and the space between gem and numeral.
pub const GEM: [u32; 2] = [11, 14];
/// See [`GEM`].
pub const GEM_GAP: u32 = 2;
/// The numeral's gold, row by row from the top (vanilla medians).
pub const GOLD_ROWS: [[u8; 3]; 13] = [
    [182, 156, 102],
    [191, 163, 98],
    [194, 164, 105],
    [165, 144, 91],
    [168, 142, 87],
    [177, 153, 94],
    [165, 139, 88],
    [159, 135, 87],
    [162, 137, 86],
    [161, 137, 85],
    [162, 142, 85],
    [162, 137, 82],
    [183, 156, 87],
];
/// Shadow alpha at 1, 2 and 3 px from a glyph or the gem.
pub const SHADOW: [f64; 3] = [143.0, 30.0, 4.0];
const GEM_LIGHT: [u8; 3] = [117, 184, 224];
const GEM_MID: [u8; 3] = [53, 107, 155];
const GEM_DARK: [u8; 3] = [22, 45, 85];

/// The Roman numeral of a tier.
pub fn roman(tier: u32) -> Result<&'static str> {
    const R: [&str; 10] = ["I", "II", "III", "IV", "V", "VI", "VII", "VIII", "IX", "X"];
    R.get(tier.wrapping_sub(1) as usize).copied().ok_or_else(|| Error::Invalid(format!("the tier is {tier}; it must be 1 to 10 (I to X)")))
}

fn glyph_width(c: char) -> u32 {
    match c {
        'I' => 8,
        _ => 11,
    }
}

fn seg(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

/// Is the point (glyph coordinates, pixel units) inside the glyph? `I` is vanilla's; `V` and `X` use the same serifs
/// and a stroke as heavy as the `I`'s stem.
fn inside(c: char, p: [f64; 2]) -> bool {
    let h = f64::from(GLYPH_HEIGHT);
    let w = f64::from(glyph_width(c));
    let top = p[1] < 2.0;
    let bottom = p[1] >= h - 2.0;
    match c {
        'I' => top || bottom || (3.0..6.0).contains(&p[0]),
        'V' => {
            let serif = top && (p[0] < 4.5 || p[0] >= w - 4.5);
            serif || seg(p, [2.2, 1.0], [w / 2.0, h - 0.8]) <= 1.4 || seg(p, [w - 2.2, 1.0], [w / 2.0, h - 0.8]) <= 0.9
        }
        _ => {
            let serif = (top || bottom) && (p[0] < 4.5 || p[0] >= w - 4.5);
            serif || seg(p, [2.2, 1.0], [w - 2.2, h - 1.0]) <= 1.4 || seg(p, [w - 2.2, 1.0], [2.2, h - 1.0]) <= 0.9
        }
    }
}

/// Coverage of a shape at 8x8 samples per pixel.
fn coverage(w: u32, h: u32, f: impl Fn([f64; 2]) -> bool) -> Vec<f64> {
    let s = 8u32;
    let mut out = vec![0.0; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let mut hit = 0;
            for sy in 0..s {
                for sx in 0..s {
                    let p = [f64::from(x) + (f64::from(sx) + 0.5) / f64::from(s), f64::from(y) + (f64::from(sy) + 0.5) / f64::from(s)];
                    if f(p) {
                        hit += 1;
                    }
                }
            }
            out[(y * w + x) as usize] = f64::from(hit) / f64::from(s * s);
        }
    }
    out
}

/// The badge alone, on a transparent canvas with a 3 px border for the shadow.
pub fn badge(o: &TechOverlay) -> Result<Rgba> {
    let text = roman(o.tier)?;
    let text_w: u32 = text.chars().map(glyph_width).sum::<u32>() + GLYPH_GAP * (text.len() as u32 - 1);
    let lead = if o.gem { GEM[0] + GEM_GAP } else { 0 };
    let (bw, bh) = (lead + text_w + 6, (if o.gem { GEM[1] } else { GLYPH_HEIGHT }) + 6);
    // colour and coverage of every pixel of the badge body
    let mut body = Rgba::new(bw, bh);
    if o.gem {
        let (gw, gh) = (f64::from(GEM[0]), f64::from(GEM[1]));
        let (cx, cy) = (gw / 2.0, gh * 0.53);
        let rhombus = |p: [f64; 2]| {
            let dx = (p[0] - cx).abs() / (gw / 2.0);
            let dy = if p[1] < cy { (cy - p[1]) / cy } else { (p[1] - cy) / (gh - cy) };
            dx + dy <= 1.0
        };
        let cov = coverage(GEM[0], GEM[1], rhombus);
        for y in 0..GEM[1] {
            for x in 0..GEM[0] {
                let c = cov[(y * GEM[0] + x) as usize];
                if c <= 0.0 {
                    continue;
                }
                let (u, v) = (f64::from(x) + 0.5, f64::from(y) + 0.5);
                let rgb = match (u < cx, v < cy) {
                    (true, true) => GEM_LIGHT,
                    (false, false) => GEM_DARK,
                    _ => GEM_MID,
                };
                body.set_px(x + 3, y + 3, [rgb[0], rgb[1], rgb[2], (c * 255.0).round() as u8]);
            }
        }
    }
    let mut x0 = 3 + lead;
    for c in text.chars() {
        let gw = glyph_width(c);
        let cov = coverage(gw, GLYPH_HEIGHT, |p| inside(c, p));
        for y in 0..GLYPH_HEIGHT {
            for x in 0..gw {
                let a = cov[(y * gw + x) as usize];
                if a > 0.0 {
                    let rgb = GOLD_ROWS[y as usize];
                    body.set_px(x0 + x, y + 3, [rgb[0], rgb[1], rgb[2], (a * 255.0).round() as u8]);
                }
            }
        }
        x0 += gw + GLYPH_GAP;
    }
    // the shadow: by distance to the nearest half-covered pixel
    let solid: Vec<bool> = body.data.chunks_exact(4).map(|p| p[3] >= 128).collect();
    let mut out = Rgba::new(bw, bh);
    for y in 0..bh as i64 {
        for x in 0..bw as i64 {
            let mut best = f64::MAX;
            for dy in -3i64..=3 {
                for dx in -3i64..=3 {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < i64::from(bw) && yy < i64::from(bh) && solid[(yy * i64::from(bw) + xx) as usize] {
                        best = best.min(((dx * dx + dy * dy) as f64).sqrt());
                    }
                }
            }
            let a = if best <= 1.0 {
                SHADOW[0]
            } else if best < 3.5 {
                let i = (best.floor() as usize).min(2);
                let f = best - i as f64;
                let next = SHADOW.get(i).copied().unwrap_or(0.0);
                SHADOW[i - 1] + (next - SHADOW[i - 1]) * f.min(1.0)
            } else {
                0.0
            };
            let shadow = [0, 0, 0, a.round() as u8];
            out.set_px(x as u32, y as u32, over(body.px(x as u32, y as u32), shadow));
        }
    }
    Ok(out)
}

/// Where vanilla puts the badge on a canvas: the top-left corner of the badge image (including its 3 px shadow border).
pub fn vanilla_position(o: &TechOverlay, canvas: [u32; 2]) -> Result<[i64; 2]> {
    let text = roman(o.tier)?;
    let text_w: u32 = text.chars().map(glyph_width).sum::<u32>() + GLYPH_GAP * (text.len() as u32 - 1);
    let (mid, top) = match canvas {
        [146, 54] => (121.0, 5.0),
        [131, 52] => (119.0, 3.0),
        [w, h] => ((f64::from(w) * 0.83).round(), (f64::from(h) * 0.09).round()),
    };
    let text_x = (mid - f64::from(text_w) / 2.0).floor() as i64;
    let lead = if o.gem { i64::from(GEM[0] + GEM_GAP) } else { 0 };
    Ok([text_x - lead - 3, top as i64 - 3])
}

/// Stamp the overlay on a finished technology picture. The badge must fit inside the canvas.
pub fn stamp(img: &Rgba, o: &TechOverlay) -> Result<Rgba> {
    let b = badge(o)?;
    let [x, y] = match o.position {
        Some([x, y]) => [i64::from(x) - 3, i64::from(y) - 3],
        None => vanilla_position(o, [img.width, img.height])?,
    };
    // the visible badge (without its shadow border) must lie inside the picture
    if x + 3 < 0 || y + 3 < 0 || x + i64::from(b.width) - 3 > i64::from(img.width) || y + i64::from(b.height) - 3 > i64::from(img.height) {
        return Err(Error::Invalid(format!(
            "the {} badge ({}x{} px) does not fit at {},{} on a {}x{} picture",
            roman(o.tier)?,
            b.width - 6,
            b.height - 6,
            x + 3,
            y + 3,
            img.width,
            img.height
        )));
    }
    let mut out = img.clone();
    out.paste_over(&b, x, y);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overlay(tier: u32, gem: bool) -> TechOverlay {
        TechOverlay { tier, gem, position: None }
    }

    #[test]
    fn numerals_are_roman_and_out_of_range_tiers_are_refused() {
        assert_eq!(roman(1).unwrap(), "I");
        assert_eq!(roman(4).unwrap(), "IV");
        assert_eq!(roman(10).unwrap(), "X");
        assert!(roman(0).is_err() && roman(11).is_err());
    }

    #[test]
    fn the_i_is_vanillas_glyph_with_its_shadow() {
        let b = badge(&overlay(1, false)).unwrap();
        assert_eq!((b.width, b.height), (8 + 6, 13 + 6));
        // serif rows across the full width, a 3 px stem between them
        assert_eq!(b.px(3, 3), [182, 156, 102, 255]);
        assert_eq!(b.px(10, 4)[3], 255);
        assert_eq!(b.px(4, 9), [0, 0, 0, 30], "left of the stem is only shadow, two pixels out");
        assert_eq!(b.px(7, 9), [165, 139, 88, 255], "the stem, row 6");
        assert_eq!(b.px(3, 15), [183, 156, 87, 255], "bottom serif");
        // the shadow: alpha 143 one pixel out
        assert_eq!(b.px(2, 9), [0, 0, 0, 0], "three pixels left of the stem is clear");
        assert_eq!(b.px(5, 9)[3], 143, "one pixel left of the stem");
        assert_eq!(b.px(2, 4)[3], 143, "one pixel left of the serif");
    }

    #[test]
    fn the_badge_lands_where_vanilla_puts_it() {
        let pic = Rgba::new(146, 54);
        let one = stamp(&pic, &overlay(1, false)).unwrap();
        let b = one.alpha_bbox(200).unwrap();
        assert_eq!(b, [117, 5, 125, 18], "vanilla's single I on 146x54 is at x 117 to 124, y 5 to 17");
        let two = stamp(&pic, &overlay(2, false)).unwrap().alpha_bbox(200).unwrap();
        assert_eq!(two, [112, 5, 129, 18], "vanilla's II spans x 112 to 128");
        let small = stamp(&Rgba::new(131, 52), &overlay(1, false)).unwrap().alpha_bbox(200).unwrap();
        assert_eq!(small, [115, 3, 123, 16]);
        // the gem sits 2 px before the numeral, as on the battleship pictures
        let gem = badge(&overlay(3, true)).unwrap();
        let p = gem.px(3 + 3, 3 + 4);
        assert_eq!(&p[..3], &GEM_LIGHT[..], "upper-left facet: {p:?}");
        assert!(gem.px(3 + 5, 3 + 12)[2] < 120, "lower-right facet is dark");
        assert_eq!(gem.width, 11 + 2 + 26 + 6);
        // a badge that does not fit is refused
        let tiny = stamp(&Rgba::new(20, 10), &overlay(8, true));
        assert!(tiny.unwrap_err().to_string().contains("does not fit"));
        let placed = stamp(&pic, &TechOverlay { tier: 1, gem: false, position: Some([4, 4]) }).unwrap().alpha_bbox(200).unwrap();
        assert_eq!(placed, [4, 4, 12, 17]);
        assert_eq!(stamp(&pic, &overlay(5, true)).unwrap(), stamp(&pic, &overlay(5, true)).unwrap(), "deterministic");
    }

    /// Our I and II against vanilla's on the 146x54 pictures that carry them (FIN armoured cars): same box, same gold.
    #[test]
    fn the_numeral_matches_vanillas_on_its_own_pictures() {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let read = |n: &str| crate::dds::decode(&std::fs::read(g.join("gfx/interface/technologies").join(n)).ok()?).ok();
        let (Some(one), Some(two)) = (read("FIN_armored_car_equipment_1.dds"), read("FIN_armored_car_equipment_2.dds")) else {
            eprintln!("SKIPPED tech numerals vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        };
        let gold = |img: &Rgba, b: [u32; 4]| {
            let mut v = Vec::new();
            for y in b[1]..b[3] {
                for x in b[0]..b[2] {
                    let p = img.px(x, y);
                    if p[3] > 200 && p[0] > p[1] && p[1] > p[2] && i32::from(p[0]) - i32::from(p[2]) > 50 {
                        v.push((x, y, p));
                    }
                }
            }
            v
        };
        for (van, tier) in [(one, 1), (two, 2)] {
            let ours = stamp(&Rgba::new(146, 54), &overlay(tier, false)).unwrap();
            let ob = ours.alpha_bbox(200).unwrap();
            let vg = gold(&van, [100, 0, 146, 25]);
            let (vx0, vx1) = (vg.iter().map(|p| p.0).min().unwrap(), vg.iter().map(|p| p.0).max().unwrap() + 1);
            let (vy0, vy1) = (vg.iter().map(|p| p.1).min().unwrap(), vg.iter().map(|p| p.1).max().unwrap() + 1);
            eprintln!("tier {tier}: vanilla numeral box {vx0},{vy0} to {vx1},{vy1}; ours {ob:?}");
            assert!((i64::from(ob[0]) - i64::from(vx0)).abs() <= 1 && (i64::from(ob[2]) - i64::from(vx1)).abs() <= 1, "x");
            assert!((i64::from(ob[1]) - i64::from(vy0)).abs() <= 1 && (i64::from(ob[3]) - i64::from(vy1)).abs() <= 1, "y");
            let mean = |v: &[(u32, u32, [u8; 4])], c: usize| v.iter().map(|p| f64::from(p.2[c])).sum::<f64>() / v.len() as f64;
            let og = gold(&ours, [100, 0, 146, 25]);
            for c in 0..3 {
                assert!((mean(&og, c) - mean(&vg, c)).abs() <= 15.0, "tier {tier} channel {c}: {} vs {}", mean(&og, c), mean(&vg, c));
            }
        }
    }
}
