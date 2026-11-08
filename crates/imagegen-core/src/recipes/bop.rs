//! Balance of power side icons: one emblem in its stages.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, the 36 `bop_<TAG>_<name>_side` and `_improved_side` pairs of
//! `gfx/interface/bop/`, 72x88, alpha cut-outs with no frame and no plate):
//!
//! * The standard stage is the emblem in grey (a bust, a head, a symbol): mean colour about (51, 51, 51) and
//!   (65, 65, 65) on the two pairs measured.
//! * The improved stage is the same emblem and the same silhouette in its own colours (gold, flame, a green sprig, a
//!   skin tone, a red crown), a little stronger in contrast, with a soft coloured glow just outside the silhouette:
//!   pale gold for a good side, red for a bad one. The pairs measured keep 99 to 132 percent of the standard's opaque
//!   pixels, the extra being the glow.
//! * Sides with a good and a bad form (`_good_side`, `_bad_side`) draw the bad one with cracks through the same emblem
//!   (a split helmet, a fractured glasses lens): the bad standard stage is the grey emblem cracked with no glow, the bad
//!   improved stage is the cracked colour emblem in a red glow.
//!
//! **How the stages are made.** The processed art is the emblem in colour. Every stage is an edit of it, so the emblem
//! is identical in all of them and only the finish changes:
//!
//! * standard: luminance of the emblem, alpha untouched;
//! * improved: the emblem's colours with their saturation and contrast raised, and a pale gold glow four pixels wide
//!   outside the silhouette (alpha 136, 96, 58 and 26 on the four rings);
//! * weakened (only when asked), vanilla's `<name>_bad_side` and `<name>_bad_improved_side`: the standard stage with a
//!   few dark cracks that run in from the edge with a pale lip beside them, and the improved stage with the same
//!   cracks and a dull red glow in place of the gold one. The cracks are drawn from a seed taken from the pixels, so one
//!   emblem always gives the same cracks, in both stages.
//!
//! Everything is a pure function of the pixels, deterministic on every platform.

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};
use crate::refselect::Rng;

/// Version of the stage recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;
/// Width of the glow outside the silhouette; the fit margin leaves this much room.
pub const GLOW_WIDTH: u32 = 4;
/// Glow alpha on the first, second and third ring outside the silhouette: a soft fall-off, as vanilla's glow.
pub const GLOW_ALPHA: [u8; 4] = [136, 96, 58, 26];
/// Glow colour of the improved stage.
pub const GLOW_GOOD: [u8; 3] = [232, 208, 128];
/// Glow colour of the cracked stage.
pub const GLOW_CRACKED: [u8; 3] = [176, 44, 40];
/// Saturation factor of the improved stage.
pub const SATURATION: f64 = 1.45;
/// Contrast factor of the improved stage, about mid grey.
pub const CONTRAST: f64 = 1.2;
/// Brightness factor of the cracked stage.
pub const CRACKED_DARKEN: f64 = 0.9;
/// Cracks drawn.
pub const CRACKS: usize = 10;
/// Bites taken out of the silhouette.
pub const CHIPS: usize = 6;
/// Share of its value a crack pixel keeps: the crack darkens what is under it, so it follows the shading.
pub const CRACK_KEEP: f64 = 0.3;
/// Share of its value the emblem's outermost edge pixel keeps, and the next one in: vanilla's dark rim (edge luminance 7 in a median 88).
pub const RIM_KEEP: [f64; 2] = [0.12, 0.6];
/// Alpha at which a pixel counts as the emblem.
const SOLID: u8 = 96;

/// The stages of one emblem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stages {
    /// `_side`
    pub standard: Rgba,
    /// `_improved_side`
    pub improved: Rgba,
    /// `_bad_side`, when asked for: the standard stage cracked.
    pub cracked: Option<Rgba>,
    /// `_bad_improved_side`, when asked for: the improved stage cracked, in a red glow.
    pub cracked_improved: Option<Rgba>,
}

fn luma(p: [u8; 4]) -> f64 {
    0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])
}

/// The emblem in grey.
pub fn standard(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        let l = luma([p[0], p[1], p[2], p[3]]).round().clamp(0.0, 255.0) as u8;
        p[0] = l;
        p[1] = l;
        p[2] = l;
    }
    out
}

/// Chebyshev distance (1 or 2) of a transparent-ish pixel to the nearest emblem pixel, 0 when it is not in a ring.
fn ring(img: &Rgba, x: u32, y: u32) -> u32 {
    let solid_at = |dx: i64, dy: i64| {
        let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
        nx >= 0 && ny >= 0 && nx < i64::from(img.width) && ny < i64::from(img.height) && img.alpha(nx as u32, ny as u32) >= SOLID
    };
    for d in 1..=GLOW_WIDTH as i64 {
        for dy in -d..=d {
            for dx in -d..=d {
                if dx.abs().max(dy.abs()) == d && solid_at(dx, dy) {
                    return d as u32;
                }
            }
        }
    }
    0
}

/// Put a glow of `colour` under the emblem's silhouette.
fn glow(img: &Rgba, colour: [u8; 3]) -> Rgba {
    let mut out = img.clone();
    for y in 0..img.height {
        for x in 0..img.width {
            let own = img.px(x, y);
            if own[3] >= SOLID {
                continue;
            }
            let d = ring(img, x, y);
            if d == 0 {
                continue;
            }
            let g = [colour[0], colour[1], colour[2], GLOW_ALPHA[d as usize - 1]];
            out.set_px(x, y, over(own, g));
        }
    }
    out
}

/// The emblem in its own colours, stronger, with a pale gold glow.
pub fn improved(img: &Rgba) -> Rgba {
    glow(&improved_body(img), GLOW_GOOD)
}

/// The emblem in its own colours, stronger, without the glow.
fn improved_body(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        if p[3] == 0 {
            continue;
        }
        let l = luma([p[0], p[1], p[2], p[3]]);
        for c in p.iter_mut().take(3) {
            let v = l + (f64::from(*c) - l) * SATURATION;
            let v = 128.0 + (v - 128.0) * CONTRAST;
            *c = v.round().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// A crack: a connected, slightly wandering line from `from` towards `to`, drawn in dark on the emblem with a pale lip
/// below and right of it. Every step moves one pixel, so the line has no gaps.
fn draw_crack(out: &mut Rgba, solid: &Rgba, rng: &mut Rng, from: (f64, f64), to: (f64, f64), depth: u32) {
    let (mut x, mut y) = from;
    let (dx, dy) = (to.0 - from.0, to.1 - from.1);
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let base = dy.atan2(dx);
    let mut ang = base;
    let steps = len.round() as u32;
    let mut branched = false;
    for i in 0..steps {
        // the heading wanders a little around the straight line
        let turn = (rng.next_u64() % 101) as f64 / 100.0 - 0.5; // -0.5 .. 0.5
        ang = (ang + turn * 0.9).clamp(base - 0.7, base + 0.7);
        x += ang.cos();
        y += ang.sin();
        let (px, py) = (x.round() as i64, y.round() as i64);
        if px < 0 || py < 0 || px >= i64::from(out.width) || py >= i64::from(out.height) {
            break;
        }
        let (px, py) = (px as u32, py as u32);
        if solid.alpha(px, py) < SOLID {
            continue;
        }
        let a = out.px(px, py)[3];
        let q = out.px(px, py);
        out.set_px(px, py, [(f64::from(q[0]) * CRACK_KEEP) as u8, (f64::from(q[1]) * CRACK_KEEP) as u8, (f64::from(q[2]) * CRACK_KEEP) as u8, a]);
        // a main crack is two pixels wide, so it still shows on dark cloth
        if depth == 0 {
            let (wx, wy) = if ang.cos().abs() > ang.sin().abs() { (px, py + 1) } else { (px + 1, py) };
            if wx < out.width && wy < out.height && solid.alpha(wx, wy) >= SOLID {
                let q = out.px(wx, wy);
                out.set_px(
                    wx,
                    wy,
                    [(f64::from(q[0]) * CRACK_KEEP) as u8, (f64::from(q[1]) * CRACK_KEEP) as u8, (f64::from(q[2]) * CRACK_KEEP) as u8, q[3]],
                );
            }
        }
        // the pale lip below and right of the crack, always visible: dark cloth gets a light edge
        let (lx, ly) = (px + 1, py + 2);
        if lx < out.width && ly < out.height && solid.alpha(lx, ly) >= SOLID {
            let q = out.px(lx, ly);
            let l = |c: u8| (u32::from(c) + 70).min(235) as u8;
            if q[0] > 70 || q[1] > 70 || q[2] > 70 || depth == 0 {
                out.set_px(lx, ly, [l(q[0]), l(q[1]), l(q[2]), q[3]]);
            }
        }
        if depth < 2 && !branched && i > steps / 4 && rng.next_u64().is_multiple_of(3) {
            branched = true;
            let side = if rng.next_u64().is_multiple_of(2) { 0.8 } else { -0.8 };
            let l = f64::from(steps - i) * 0.5;
            let a2 = ang + side;
            draw_crack(out, solid, rng, (x, y), (x + a2.cos() * l, y + a2.sin() * l), depth + 1);
        }
    }
}

/// Cracks drawn through `base` along `img`'s silhouette; the same `seed` and `img` give the same cracks on any base.
fn crack(mut out: Rgba, img: &Rgba, seed: u64) -> Result<Rgba> {
    // the emblem's own silhouette and its centre of mass
    let (mut sx, mut sy, mut n) = (0.0, 0.0, 0.0);
    let mut edge: Vec<(u32, u32)> = Vec::new();
    for y in 0..img.height {
        for x in 0..img.width {
            if img.alpha(x, y) < SOLID {
                continue;
            }
            sx += f64::from(x);
            sy += f64::from(y);
            n += 1.0;
            let open = |dx: i64, dy: i64| {
                let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                nx < 0 || ny < 0 || nx >= i64::from(img.width) || ny >= i64::from(img.height) || img.alpha(nx as u32, ny as u32) < SOLID
            };
            if open(1, 0) || open(-1, 0) || open(0, 1) || open(0, -1) {
                edge.push((x, y));
            }
        }
    }
    if n < 16.0 || edge.is_empty() {
        return Err(Error::Invalid("the emblem has no visible pixels to crack".into()));
    }
    let centre = (sx / n, sy / n);
    let mut rng = Rng::new(seed);
    let mut used: Vec<(u32, u32)> = Vec::new();
    for _ in 0..CRACKS {
        // starts apart: skip an edge pixel within 10 px of an earlier start
        let mut pick = edge[(rng.next_u64() % edge.len() as u64) as usize];
        for _ in 0..12 {
            let far = used.iter().all(|u| (i64::from(u.0) - i64::from(pick.0)).pow(2) + (i64::from(u.1) - i64::from(pick.1)).pow(2) > 100);
            if far {
                break;
            }
            pick = edge[(rng.next_u64() % edge.len() as u64) as usize];
        }
        used.push(pick);
        let from = (f64::from(pick.0), f64::from(pick.1));
        let (dx, dy) = (centre.0 - from.0, centre.1 - from.1);
        let reach = 0.7 + (rng.next_u64() % 30) as f64 / 100.0;
        draw_crack(&mut out, img, &mut rng, from, (from.0 + dx * reach, from.1 + dy * reach), 0);
    }
    // chips: small bites out of the silhouette, as vanilla's damaged sides have
    for _ in 0..CHIPS {
        let (cx, cy) = edge[(rng.next_u64() % edge.len() as u64) as usize];
        let r = 1 + (rng.next_u64() % 2) as i64;
        for dy in -r..=r {
            for dx in -r..=r {
                if dx.abs() + dy.abs() > r {
                    continue;
                }
                let (nx, ny) = (i64::from(cx) + dx, i64::from(cy) + dy);
                if nx < 0 || ny < 0 || nx >= i64::from(out.width) || ny >= i64::from(out.height) {
                    continue;
                }
                out.set_px(nx as u32, ny as u32, [0, 0, 0, 0]);
            }
        }
    }
    Ok(out)
}

/// The weakened standard stage (vanilla's `_bad_side`): the grey emblem, a little darker, cracked, with no glow.
pub fn cracked(img: &Rgba, seed: u64) -> Result<Rgba> {
    let mut base = standard(img);
    for p in base.data.chunks_exact_mut(4) {
        for c in p.iter_mut().take(3) {
            *c = (f64::from(*c) * CRACKED_DARKEN).round() as u8;
        }
    }
    crack(base, img, seed)
}

/// The weakened improved stage (vanilla's `_bad_improved_side`): the colour emblem, cracked the same way, in a dull red glow.
pub fn cracked_improved(img: &Rgba, seed: u64) -> Result<Rgba> {
    Ok(glow(&crack(improved_body(img), img, seed)?, GLOW_CRACKED))
}

/// Vanilla's dark rim: the emblem's outermost pixel keeps 12 percent of its value and the next one in 60 percent, so the
/// cut-out edge is near black as in the game's own sides. Alpha is untouched.
pub fn rim(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for y in 0..img.height {
        for x in 0..img.width {
            if img.alpha(x, y) < SOLID {
                continue;
            }
            let d = depth_in(img, x, y);
            if d == 0 || d > RIM_KEEP.len() as u32 {
                continue;
            }
            let k = RIM_KEEP[d as usize - 1];
            let p = img.px(x, y);
            out.set_px(x, y, [(f64::from(p[0]) * k) as u8, (f64::from(p[1]) * k) as u8, (f64::from(p[2]) * k) as u8, p[3]]);
        }
    }
    out
}

/// How many pixels inside the silhouette (1 = on its edge) a solid pixel lies, up to the rim's depth; 0 when deeper.
fn depth_in(img: &Rgba, x: u32, y: u32) -> u32 {
    let open = |dx: i64, dy: i64| {
        let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
        nx < 0 || ny < 0 || nx >= i64::from(img.width) || ny >= i64::from(img.height) || img.alpha(nx as u32, ny as u32) < SOLID
    };
    for d in 1..=RIM_KEEP.len() as i64 {
        for dy in -d..=d {
            for dx in -d..=d {
                if dx.abs() + dy.abs() <= d && open(dx, dy) {
                    return d as u32;
                }
            }
        }
    }
    0
}

/// Every stage of one emblem. `img` is the fitted emblem in colour; `seed` fixes the cracks.
pub fn stages(img: &Rgba, with_cracked: bool, seed: u64) -> Result<Stages> {
    if img.alpha_bbox(SOLID).is_none() {
        return Err(Error::Invalid("the emblem has no visible pixels".into()));
    }
    // the dark rim first, so every stage carries it and the cracks and glows follow the same silhouette
    let img = &rim(img);
    let (cracked, cracked_improved) = if with_cracked { (Some(cracked(img, seed)?), Some(cracked_improved(img, seed)?)) } else { (None, None) };
    Ok(Stages { standard: standard(img), improved: improved(img), cracked, cracked_improved })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A coloured disc with a darker core, 72x88 with a clear margin.
    fn emblem() -> Rgba {
        let mut img = Rgba::new(72, 88);
        for y in 0..88u32 {
            for x in 0..72u32 {
                let (dx, dy) = (f64::from(x) - 36.0, f64::from(y) - 44.0);
                let d = (dx * dx + dy * dy).sqrt();
                if d < 30.0 {
                    let shade = (200.0 - d * 3.0) as u8;
                    img.set_px(x, y, [shade, shade / 2 + 40, 60, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn stages_keep_the_canvas_and_the_emblem() {
        let e = emblem();
        let s = stages(&e, true, 7).unwrap();
        for st in [&s.standard, &s.improved, s.cracked.as_ref().unwrap(), s.cracked_improved.as_ref().unwrap()] {
            assert_eq!((st.width, st.height), (72, 88));
        }
        // the standard keeps the emblem's alpha exactly
        for y in 0..88 {
            for x in 0..72 {
                assert_eq!(s.standard.alpha(x, y), e.alpha(x, y));
            }
        }
        // the improved and cracked stages keep every emblem pixel opaque and only add glow outside
        for y in 0..88 {
            for x in 0..72 {
                if e.alpha(x, y) == 255 {
                    assert_eq!(s.improved.alpha(x, y), 255);
                } else if e.alpha(x, y) == 0 {
                    // glow reaches GLOW_WIDTH pixels and no more
                    let near = (-4i64..=4).any(|dy| {
                        (-4i64..=4).any(|dx| {
                            let (nx, ny) = (i64::from(x) + dx, i64::from(y) + dy);
                            nx >= 0 && ny >= 0 && nx < 72 && ny < 88 && e.alpha(nx as u32, ny as u32) == 255
                        })
                    });
                    assert_eq!(s.improved.alpha(x, y) > 0, near, "({x},{y})");
                }
            }
        }
    }

    #[test]
    fn standard_is_grey_improved_is_coloured_cracked_is_grey_with_dark_cracks() {
        let e = emblem();
        let s = stages(&e, true, 7).unwrap();
        let sat = |img: &Rgba| {
            img.data.chunks_exact(4).filter(|p| p[3] == 255).map(|p| u32::from(p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]))).sum::<u32>()
        };
        assert_eq!(sat(&s.standard), 0);
        assert!(sat(&s.improved) > sat(&e), "the improved stage is more saturated than the source");
        let c = s.cracked.unwrap();
        let darker = |img: &Rgba| {
            img.data
                .chunks_exact(4)
                .zip(s.standard.data.chunks_exact(4))
                .filter(|(p, q)| p[3] == 255 && f64::from(p[0]) < f64::from(q[0]) * 0.5 && q[0] > 60)
                .count()
        };
        let dark = darker(&c);
        assert!(dark >= 10, "cracks are drawn ({dark} dark pixels)");
        assert_eq!(darker(&s.standard), 0);
        // the bad standard stage has no glow, the bad improved stage has the red one, and both crack at the same pixels
        assert_eq!(c.data.chunks_exact(4).filter(|p| p[3] != 0 && p[3] != 255).count(), 0, "no glow on the cracked standard stage");
        let ci = s.cracked_improved.unwrap();
        assert!(ci.data.chunks_exact(4).any(|p| p[3] > 0 && p[3] < 255 && p[0] > p[2] + 40), "a red glow");
        // the same cracks in both stages (a lip pixel may brighten a crack pixel of the brighter colour stage)
        let (mut cracks, mut apart) = (0, 0);
        for y in 0..88 {
            for x in 0..72 {
                let cracked_in =
                    |img: &Rgba, base: &Rgba| img.alpha(x, y) == 255 && luma(img.px(x, y)) < luma(base.px(x, y)) * 0.5 && luma(base.px(x, y)) > 20.0;
                let (a, b) = (cracked_in(&ci, &improved_body(&rim(&e))), cracked_in(&c, &s.standard));
                cracks += usize::from(b);
                apart += usize::from(a != b);
            }
        }
        assert!(cracks >= 10 && apart * 5 <= cracks, "cracks in both stages: {apart} of {cracks} differ");
    }

    #[test]
    fn the_cracked_stages_have_a_few_chips_and_otherwise_keep_the_emblem() {
        let e = emblem();
        let s = stages(&e, true, 7).unwrap();
        let solid = |img: &Rgba| img.data.chunks_exact(4).filter(|p| p[3] == 255).count();
        for c in [s.cracked.as_ref().unwrap(), s.cracked_improved.as_ref().unwrap()] {
            let (before, after) = (solid(&s.standard), solid(c));
            assert!(after < before, "chips take pixels out of the silhouette");
            assert!(after * 100 >= before * 90, "but only a few ({after} of {before})");
        }
    }

    #[test]
    fn cracks_are_deterministic_and_follow_the_seed() {
        let e = emblem();
        assert_eq!(cracked(&e, 3).unwrap(), cracked(&e, 3).unwrap());
        assert_ne!(cracked(&e, 3).unwrap(), cracked(&e, 4).unwrap());
    }

    #[test]
    fn an_empty_emblem_is_refused() {
        assert!(stages(&Rgba::new(72, 88), false, 1).is_err());
        assert!(stages(&emblem(), false, 1).unwrap().cracked.is_none());
    }
}
