//! Operation card and map medallion: the 85x85 picture of an operation and its 48x48 map icon.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, `gfx/interface/operations/<name>.dds` and `map/<name>_map.dds`, the
//! `GFX_operations_<name>` and `GFX_operations_<name>_map` sprites of `interface/lar_operations.gfx`, which the
//! operation's `icon` and `map_icon` name; 17 pairs measured):
//!
//! * The card is a close-up of hands and objects, painted or photographic, in a warm, muted palette (mean saturation
//!   0.35, mean RGB about 105, 91, 69), on a transparent 85x85 canvas: the four corners are clear and the card covers
//!   0.78 of the canvas (x 4 to 79, y 4 to 80).
//! * It is tilted a little, 1 to 4 degrees (1.9 in 7 of 17, either way), has a one pixel near-black outline and a soft
//!   drop shadow that shows about 3 px below the bottom edge (alpha 254, 192, 68, 3 on the rows below it) and not at
//!   the top.
//! * The map icon is the picture again as a 48x48 medallion: a disc of radius about 18.6 px showing the centre of the
//!   card at 0.6 scale (the central 64 px of the card), in a bevelled gold ring from radius 19 to 23 (a dark inner line,
//!   an inner bevel lit from below, an outer bevel lit from above with colours 219, 214, 188 at the top and 63, 50, 10
//!   at the bottom) and a soft outer edge (alpha 0.86 at radius 23, 0.42 at 23.5, 0.13 at 24).
//!
//! The card keeps the model's own colours, only brought toward vanilla's muted, warm range, so a picture in colour
//! stays in colour. The map icon is derived from the finished card by editing, never a second generation.
//!
//! Everything is a pure function of the pixels and the seed, deterministic on every platform.

use crate::error::Result;
use crate::pixels::Rgba;
use crate::recipes::pillow::{alpha_composite, autocontrast_rgb, det_exp, enhance_contrast};
use crate::recipes::report_card::{add_grain, cover_crop, rotate_card, shadow_layer, ReportCardParams};
use crate::resize::resize_lanczos3;

/// Version of the operation card recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// Side of the card canvas.
pub const CANVAS: u32 = 85;
/// Side of the picture inside the outline, before the tilt.
pub const PHOTO: u32 = 71;
/// Side of the map icon.
pub const MAP_CANVAS: u32 = 48;
/// Side of the central part of the card that the medallion shows.
pub const MAP_CROP: u32 = 64;
/// Diameter of the picture inside the ring.
pub const MAP_DISC: u32 = 38;
/// The outline colour of the card.
pub const OUTLINE: [u8; 3] = [11, 10, 10];
/// Vanilla's measured tilts (degrees), most common first; the seed picks one and the sign.
pub const TILTS: [f64; 7] = [1.9, 1.0, 1.9, 1.2, 1.7, 3.4, 3.9];

/// The tilt of a card for a seed: counter-clockwise when positive.
pub fn tilt_of(seed: i64) -> f64 {
    let s = seed.unsigned_abs();
    let mag = TILTS[((s >> 1) % TILTS.len() as u64) as usize];
    if s & 1 == 0 {
        mag
    } else {
        -mag
    }
}

/// Vanilla's cards are not dark: the 17 measured have a mean luminance from 41 to 155 (median 77). A picture below 76 is
/// lifted with a gamma (never stronger than 0.6) so a dim scene does not become the darkest card of the set.
pub const MIN_MEAN_LUMA: f64 = 76.0;
/// Vanilla's darkest tenth of a card is luminance 15 (median); the blacks are lifted to it.
pub const BLACK_LEVEL: f64 = 14.0;

fn lift_dark(img: &mut Rgba) {
    let (mut sum, mut n) = (0.0_f64, 0.0_f64);
    for p in img.data.chunks_exact(4) {
        sum += 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
        n += 1.0;
    }
    let mean = (sum / n.max(1.0)).max(1.0);
    if mean >= MIN_MEAN_LUMA {
        return;
    }
    let gamma = ((MIN_MEAN_LUMA / 255.0).ln() / (mean / 255.0).ln()).max(0.6);
    for p in img.data.chunks_exact_mut(4) {
        for c in p.iter_mut().take(3) {
            *c = (255.0 * (f64::from(*c) / 255.0).powf(gamma)).round().clamp(0.0, 255.0) as u8;
        }
    }
}

/// Bring a picture toward vanilla's warm, muted card palette: levels, a little less saturation, a warm tint, a
/// slightly darker edge and fine grain.
fn tone(img: &Rgba, seed: i64) -> Rgba {
    let mut out = img.clone();
    autocontrast_rgb(&mut out, 1);
    enhance_contrast(&mut out, 1.06);
    lift_dark(&mut out);
    let (w, h) = (f64::from(out.width), f64::from(out.height));
    let width = out.width;
    for (i, p) in out.data.chunks_exact_mut(4).enumerate() {
        let x = f64::from(i as u32 % width) + 0.5;
        let y = f64::from(i as u32 / width) + 0.5;
        let l = 0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2]);
        let d = (((x - w / 2.0) / (w / 2.0)).powi(2) + ((y - h / 2.0) / (h / 2.0)).powi(2)) / 2.0;
        let shade = 1.0 - 0.14 * d * d.sqrt();
        for (c, tint) in p[..3].iter_mut().zip([1.04, 1.0, 0.92]) {
            let desat = l + (f64::from(*c) - l) * 0.78;
            *c = (BLACK_LEVEL + desat * tint * shade * 0.97 * (1.0 - BLACK_LEVEL / 255.0)).round().clamp(0.0, 255.0) as u8;
        }
    }
    add_grain(&mut out, 3, seed);
    out
}

/// The card parameters of the shadow and canvas (the report card's machinery at vanilla's operation card numbers).
fn params(seed: i64) -> ReportCardParams {
    ReportCardParams {
        canvas_size: (CANVAS, CANVAS),
        card_size: (PHOTO + 2, PHOTO + 2),
        border: 0,
        angle: tilt_of(seed),
        shadow_offset: (0, 1),
        shadow_blur: 1.1,
        shadow_opacity: 1.0,
        grain: 3,
        paper_grain: 0,
        seed,
        rotate_supersample: 4,
        edge_soften: 0.35,
        card_offset: (0, 0),
    }
}

/// Build the 85x85 operation card from a picture of any size.
pub fn operation_card(source: &Rgba, seed: i64) -> Result<Rgba> {
    let p = params(seed);
    let photo = tone(&cover_crop(source, (PHOTO, PHOTO))?, seed);
    let mut card = Rgba::filled(PHOTO + 2, PHOTO + 2, [OUTLINE[0], OUTLINE[1], OUTLINE[2], 255]);
    card.paste(&photo, 1, 1);
    let rotated = rotate_card(&card, p.angle, p.rotate_supersample, p.edge_soften)?;
    let x = (i64::from(CANVAS) - i64::from(rotated.width)).div_euclid(2);
    let y = (i64::from(CANVAS) - i64::from(rotated.height)).div_euclid(2);
    let mut canvas = Rgba::new(CANVAS, CANVAS);
    let shadow = shadow_layer(&rotated, (x, y), &p);
    alpha_composite(&mut canvas, &shadow, 0, 0);
    alpha_composite(&mut canvas, &rotated, x, y);
    Ok(canvas)
}

/// Bilinear sample of `img` at continuous coordinates (pixel centres at +0.5), clamped at the edges.
fn sample(img: &Rgba, u: f64, v: f64) -> [f64; 3] {
    let (fx, fy) = ((u - 0.5).clamp(0.0, f64::from(img.width - 1)), (v - 0.5).clamp(0.0, f64::from(img.height - 1)));
    let (x0, y0) = (fx.floor() as u32, fy.floor() as u32);
    let (x1, y1) = ((x0 + 1).min(img.width - 1), (y0 + 1).min(img.height - 1));
    let (tx, ty) = (fx - f64::from(x0), fy - f64::from(y0));
    let mut out = [0.0; 3];
    for (c, o) in out.iter_mut().enumerate() {
        let a = f64::from(img.px(x0, y0)[c]) * (1.0 - tx) + f64::from(img.px(x1, y0)[c]) * tx;
        let b = f64::from(img.px(x0, y1)[c]) * (1.0 - tx) + f64::from(img.px(x1, y1)[c]) * tx;
        *o = a * (1.0 - ty) + b * ty;
    }
    out
}

fn mix(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t]
}

fn ramp(dark: [f64; 3], mid: [f64; 3], light: [f64; 3], t: f64) -> [f64; 3] {
    if t < 0.5 {
        mix(dark, mid, t * 2.0)
    } else {
        mix(mid, light, (t - 0.5) * 2.0)
    }
}

fn smooth(lo: f64, hi: f64, x: f64) -> f64 {
    let t = ((x - lo) / (hi - lo)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

const INNER_BEVEL: ([f64; 3], [f64; 3], [f64; 3]) = ([70.0, 68.0, 58.0], [110.0, 93.0, 47.0], [150.0, 144.0, 123.0]);

/// The colour and alpha of the medallion at a point, `dx` and `dy` from the centre in pixels (y down).
fn medallion_at(pic: &Rgba, dx: f64, dy: f64) -> ([f64; 3], f64) {
    let r = dx.hypot(dy);
    let theta = dy.atan2(dx);
    // light from the top and a little from the left
    let lit = (0.5 - 0.5 * (0.92 * theta.sin() + 0.38 * theta.cos())).clamp(0.0, 1.0);
    let dark_line = [40.0, 36.0, 22.0];
    let alpha = 1.0 / (1.0 + det_exp((r - 23.45) / 0.35));
    let half = f64::from(MAP_DISC) / 2.0;
    let inner = ramp(INNER_BEVEL.0, INNER_BEVEL.1, INNER_BEVEL.2, 1.0 - lit);
    let colour = if r < 18.6 {
        let mut c = sample(pic, dx + half, dy + half);
        let shade = 1.0 - 0.2 * smooth(14.0, 18.6, r);
        for v in &mut c {
            *v *= shade;
        }
        c
    } else if r < 19.2 {
        let c = sample(pic, dx + half, dy + half);
        mix([c[0] * 0.8, c[1] * 0.8, c[2] * 0.8], dark_line, smooth(18.6, 19.2, r))
    } else if r < 20.0 {
        dark_line
    } else if r < 21.0 {
        // the inner bevel is lit from below
        mix(dark_line, inner, smooth(20.0, 20.4, r))
    } else if r < 22.6 {
        let base = ramp([64.0, 51.0, 10.0], [112.0, 92.0, 30.0], [219.0, 214.0, 190.0], lit);
        let deepen = 1.0 - (1.0 - lit) * 0.6 * smooth(21.5, 22.6, r);
        let c = [base[0] * deepen, base[1] * deepen, base[2] * deepen];
        mix(inner, c, smooth(20.8, 21.2, r))
    } else {
        mix([40.0, 32.0, 8.0], [14.0, 12.0, 4.0], smooth(22.6, 23.4, r))
    };
    (colour, alpha)
}

/// The 48x48 map icon from a finished card: its centre in a round medallion with a gold ring.
pub fn map_medallion(card: &Rgba) -> Result<Rgba> {
    let side = MAP_CROP.min(card.width).min(card.height);
    let (x0, y0) = ((card.width - side) / 2, (card.height - side) / 2);
    let centre = card.crop(x0, y0, side, side)?;
    let pic = resize_lanczos3(&centre, MAP_DISC, MAP_DISC)?;
    let mut out = Rgba::new(MAP_CANVAS, MAP_CANVAS);
    const SS: u32 = 4;
    let half = f64::from(MAP_CANVAS) / 2.0;
    for y in 0..MAP_CANVAS {
        for x in 0..MAP_CANVAS {
            let (mut acc, mut aa) = ([0.0; 3], 0.0);
            for sy in 0..SS {
                for sx in 0..SS {
                    let px = f64::from(x) + (f64::from(sx) + 0.5) / f64::from(SS) - half;
                    let py = f64::from(y) + (f64::from(sy) + 0.5) / f64::from(SS) - half;
                    let (c, a) = medallion_at(&pic, px, py);
                    for k in 0..3 {
                        acc[k] += c[k] * a;
                    }
                    aa += a;
                }
            }
            if aa > 0.0 {
                let px = [0, 1, 2].map(|k| (acc[k] / aa).round().clamp(0.0, 255.0) as u8);
                let a = (aa / f64::from(SS * SS) * 255.0).round().clamp(0.0, 255.0) as u8;
                out.set_px(x, y, [px[0], px[1], px[2], a]);
            }
        }
    }
    Ok(out)
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
    fn the_card_is_85_square_with_clear_corners_an_outline_and_a_shadow_below() {
        let card = operation_card(&picture(300, 300), 1337).unwrap();
        assert_eq!((card.width, card.height), (85, 85));
        for (x, y) in [(0, 0), (84, 0), (0, 84), (84, 84)] {
            assert_eq!(card.px(x, y)[3], 0, "corner {x},{y}");
        }
        assert_eq!(card.px(42, 42)[3], 255);
        let solid = card.data.chunks_exact(4).filter(|p| p[3] > 200).count() as f64 / (85.0 * 85.0);
        assert!((0.70..=0.86).contains(&solid), "the card covers {solid} of the canvas (vanilla 0.78)");
        let b = card.alpha_bbox(200).unwrap();
        assert!(b[0] >= 2 && b[1] >= 2 && b[2] <= 82 && b[3] <= 83, "{b:?}");
        // the shadow shows below the card but hardly above it
        let column: Vec<u32> = (0..85).map(|y| u32::from(card.px(42, y)[3])).collect();
        let last = column.iter().rposition(|a| *a > 200).unwrap();
        assert!(column[last + 1] > 0, "{column:?}");
    }

    #[test]
    fn the_tilt_is_one_of_vanillas_and_follows_the_seed() {
        for seed in 0..40 {
            let t = tilt_of(seed).abs();
            assert!(TILTS.contains(&t), "{t}");
        }
        assert!(tilt_of(2) > 0.0 && tilt_of(3) < 0.0);
        assert_eq!(operation_card(&picture(120, 90), 5).unwrap(), operation_card(&picture(120, 90), 5).unwrap());
        assert_ne!(operation_card(&picture(120, 90), 5).unwrap(), operation_card(&picture(120, 90), 6).unwrap());
    }

    #[test]
    fn colour_stays_colour_but_gets_muted() {
        let mut red = Rgba::filled(80, 80, [230, 20, 20, 255]);
        red.set_px(0, 0, [20, 20, 230, 255]);
        let card = operation_card(&red, 2).unwrap();
        let p = card.px(42, 42);
        assert!(p[0] > p[2] + 40, "{p:?}: still red");
        assert!(p[1] > 20, "{p:?}: muted toward grey");
    }

    #[test]
    fn the_medallion_is_a_48_disc_with_a_gold_ring_and_clear_corners() {
        let card = operation_card(&picture(300, 300), 7).unwrap();
        let map = map_medallion(&card).unwrap();
        assert_eq!((map.width, map.height), (48, 48));
        assert_eq!(map.px(0, 0)[3], 0);
        assert_eq!(map.px(47, 47)[3], 0);
        assert_eq!(map.px(24, 24)[3], 255);
        let covered = map.data.chunks_exact(4).filter(|p| p[3] > 200).count() as f64 / (48.0 * 48.0);
        assert!((0.68..=0.80).contains(&covered), "the disc covers {covered} (vanilla 0.74)");
        // the ring is lit from above: its top is much lighter than its bottom
        let top = map.px(24, 2);
        let bottom = map.px(24, 45);
        assert!(top[0] > bottom[0] + 60, "{top:?} {bottom:?}");
        assert!(top[0] > top[2] && bottom[0] > bottom[2], "gold, not grey");
    }
}
