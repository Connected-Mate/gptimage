//! Deterministic Lanczos3 resampling in premultiplied alpha.
//!
//! Determinism matters: the same candidate must give the same installed bytes on Windows, macOS and
//! Linux. The kernel therefore uses a hand-written sine built from additions and multiplications
//! (IEEE-754 basic operations are exact across platforms), not the platform's `sin`.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

const PI: f64 = std::f64::consts::PI;

/// Sine from range reduction plus a Taylor series; identical on every platform.
pub(crate) fn det_sin(x: f64) -> f64 {
    let two_pi = 2.0 * PI;
    let mut r = x - two_pi * (x / two_pi).round();
    if r > PI {
        r -= two_pi;
    } else if r < -PI {
        r += two_pi;
    }
    let r2 = r * r;
    let mut term = r;
    let mut sum = r;
    let mut n = 1.0;
    for _ in 0..13 {
        term *= -r2 / ((n + 1.0) * (n + 2.0));
        sum += term;
        n += 2.0;
    }
    sum
}

fn lanczos3(x: f64) -> f64 {
    let x = x.abs();
    if x < 1e-12 {
        1.0
    } else if x >= 3.0 {
        0.0
    } else {
        let px = PI * x;
        3.0 * det_sin(px) * det_sin(px / 3.0) / (px * px)
    }
}

struct Taps {
    start: Vec<i64>,
    weights: Vec<Vec<f64>>,
}

fn taps(src_len: u32, dst_len: u32) -> Taps {
    let scale = f64::from(src_len) / f64::from(dst_len);
    let filter_scale = scale.max(1.0);
    let support = 3.0 * filter_scale;
    let mut start = Vec::with_capacity(dst_len as usize);
    let mut weights = Vec::with_capacity(dst_len as usize);
    for d in 0..dst_len {
        let center = (f64::from(d) + 0.5) * scale;
        let lo = (center - support).floor() as i64;
        let hi = (center + support).ceil() as i64;
        let mut w = Vec::new();
        let mut sum = 0.0;
        for i in lo..hi {
            let weight = lanczos3((i as f64 + 0.5 - center) / filter_scale);
            w.push(weight);
            sum += weight;
        }
        if sum.abs() < 1e-12 {
            sum = 1.0;
        }
        for weight in &mut w {
            *weight /= sum;
        }
        start.push(lo);
        weights.push(w);
    }
    Taps { start, weights }
}

#[inline]
fn clamp_index(i: i64, len: u32) -> usize {
    i.clamp(0, i64::from(len) - 1) as usize
}

/// Resize with Lanczos3. Colour is filtered in premultiplied space so transparent pixels never bleed
/// their (meaningless) RGB into the result.
pub fn resize_lanczos3(src: &Rgba, new_w: u32, new_h: u32) -> Result<Rgba> {
    if new_w == 0 || new_h == 0 || new_w > 16384 || new_h > 16384 {
        return Err(Error::Image(format!("cannot resize to {new_w}x{new_h}")));
    }
    if src.width == new_w && src.height == new_h {
        return Ok(src.clone());
    }
    let (sw, sh) = (src.width as usize, src.height as usize);
    // premultiplied f32 planes: r, g, b, a in 0..=1
    let mut pre = vec![0f64; sw * sh * 4];
    for (i, p) in src.data.chunks_exact(4).enumerate() {
        let a = f64::from(p[3]) / 255.0;
        pre[i * 4] = f64::from(p[0]) / 255.0 * a;
        pre[i * 4 + 1] = f64::from(p[1]) / 255.0 * a;
        pre[i * 4 + 2] = f64::from(p[2]) / 255.0 * a;
        pre[i * 4 + 3] = a;
    }
    // horizontal pass
    let tx = taps(src.width, new_w);
    let nw = new_w as usize;
    let mut mid = vec![0f64; nw * sh * 4];
    for y in 0..sh {
        for dx in 0..nw {
            let lo = tx.start[dx];
            let mut acc = [0f64; 4];
            for (k, w) in tx.weights[dx].iter().enumerate() {
                let sx = clamp_index(lo + k as i64, src.width);
                let o = (y * sw + sx) * 4;
                for c in 0..4 {
                    acc[c] += pre[o + c] * w;
                }
            }
            let o = (y * nw + dx) * 4;
            mid[o..o + 4].copy_from_slice(&acc);
        }
    }
    // vertical pass
    let ty = taps(src.height, new_h);
    let nh = new_h as usize;
    let mut out = Rgba::new(new_w, new_h);
    for dy in 0..nh {
        let lo = ty.start[dy];
        for dx in 0..nw {
            let mut acc = [0f64; 4];
            for (k, w) in ty.weights[dy].iter().enumerate() {
                let sy = clamp_index(lo + k as i64, src.height);
                let o = (sy * nw + dx) * 4;
                for c in 0..4 {
                    acc[c] += mid[o + c] * w;
                }
            }
            let a = acc[3].clamp(0.0, 1.0);
            let a8 = (a * 255.0).round();
            let o = (dy * nw + dx) * 4;
            if a8 <= 0.0 {
                out.data[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
            } else {
                for (dst, v) in out.data[o..o + 3].iter_mut().zip(&acc) {
                    *dst = ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                out.data[o + 3] = a8 as u8;
            }
        }
    }
    Ok(out)
}

/// Coverage of source cells by each destination cell along one axis: (first source index, weights summing to 1).
fn area_taps(src: u32, dst: u32) -> Vec<(usize, Vec<f64>)> {
    let scale = f64::from(src) / f64::from(dst);
    (0..dst)
        .map(|d| {
            let (a, b) = (f64::from(d) * scale, f64::from(d + 1) * scale);
            let first = a.floor() as usize;
            let last = ((b.ceil() as usize).max(first + 1)).min(src as usize);
            let w: Vec<f64> = (first..last).map(|s| (b.min(s as f64 + 1.0) - a.max(s as f64)).max(0.0)).collect();
            let sum: f64 = w.iter().sum::<f64>().max(1e-12);
            (first, w.into_iter().map(|x| x / sum).collect())
        })
        .collect()
}

/// Resize with an area (box) filter in premultiplied alpha: each output pixel is the exact area-weighted mean of the
/// source pixels it covers. Every output colour is a convex mix of source colours, so a flat-coloured drawing keeps
/// its palette with no ringing halo (Lanczos3 over- and undershoots at hard edges). Used by the `crisp` processing
/// switch of a style pack.
pub fn resize_area(src: &Rgba, new_w: u32, new_h: u32) -> Result<Rgba> {
    if new_w == 0 || new_h == 0 || new_w > 16384 || new_h > 16384 {
        return Err(Error::Image(format!("cannot resize to {new_w}x{new_h}")));
    }
    if src.width == new_w && src.height == new_h {
        return Ok(src.clone());
    }
    let (sw, sh, nw) = (src.width as usize, src.height as usize, new_w as usize);
    let mut pre = vec![0f64; sw * sh * 4];
    for (i, p) in src.data.chunks_exact(4).enumerate() {
        let a = f64::from(p[3]) / 255.0;
        for c in 0..3 {
            pre[i * 4 + c] = f64::from(p[c]) / 255.0 * a;
        }
        pre[i * 4 + 3] = a;
    }
    let tx = area_taps(src.width, new_w);
    let mut mid = vec![0f64; nw * sh * 4];
    for y in 0..sh {
        for (dx, (first, w)) in tx.iter().enumerate() {
            let mut acc = [0f64; 4];
            for (k, wk) in w.iter().enumerate() {
                let o = (y * sw + first + k) * 4;
                for c in 0..4 {
                    acc[c] += pre[o + c] * wk;
                }
            }
            mid[(y * nw + dx) * 4..(y * nw + dx) * 4 + 4].copy_from_slice(&acc);
        }
    }
    let ty = area_taps(src.height, new_h);
    let mut out = Rgba::new(new_w, new_h);
    for (dy, (first, w)) in ty.iter().enumerate() {
        for dx in 0..nw {
            let mut acc = [0f64; 4];
            for (k, wk) in w.iter().enumerate() {
                let o = ((first + k) * nw + dx) * 4;
                for c in 0..4 {
                    acc[c] += mid[o + c] * wk;
                }
            }
            let a = acc[3].clamp(0.0, 1.0);
            let a8 = (a * 255.0).round();
            let o = (dy * nw + dx) * 4;
            if a8 <= 0.0 {
                out.data[o..o + 4].copy_from_slice(&[0, 0, 0, 0]);
            } else {
                for (dst, v) in out.data[o..o + 3].iter_mut().zip(&acc) {
                    *dst = ((v / a).clamp(0.0, 1.0) * 255.0).round() as u8;
                }
                out.data[o + 3] = a8 as u8;
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat teal pictogram with hard edges (a ring and a bar), drawn at 16 times `size`.
    fn pictogram(size: u32) -> Rgba {
        let big = size * 16;
        let mut img = Rgba::new(big, big);
        let c = f64::from(big) / 2.0;
        for y in 0..big {
            for x in 0..big {
                let (dx, dy) = (f64::from(x) + 0.5 - c, f64::from(y) + 0.5 - c);
                let r = (dx * dx + dy * dy).sqrt();
                let ring =
                    (r > 0.30 * f64::from(big) && r < 0.42 * f64::from(big)) || (dy.abs() < 0.05 * f64::from(big) && dx.abs() < 0.3 * f64::from(big));
                if ring {
                    img.set_px(x, y, [76, 198, 192, 255]);
                }
            }
        }
        img
    }

    /// Visible pixels whose colour is more than 6 levels from the art's one colour: shading a resize added.
    fn off_palette(img: &Rgba) -> usize {
        img.data.chunks_exact(4).filter(|p| p[3] > 0 && (0..3).any(|c| (i32::from(p[c]) - [76, 198, 192][c]).abs() > 6)).count()
    }

    #[test]
    fn area_resize_keeps_a_flat_palette_where_lanczos_rings() {
        // the measurement behind the crisp switch: a 640 px flat teal pictogram shrunk to 40 px and to 37 px
        for size in [40u32, 37] {
            let src = pictogram(40);
            let lanczos = resize_lanczos3(&src, size, size).unwrap();
            let area = resize_area(&src, size, size).unwrap();
            assert_eq!(off_palette(&area), 0, "the area filter adds no colour of its own at {size} px");
            assert!(off_palette(&lanczos) > 0, "Lanczos3 rings at hard edges at {size} px");
            let visible = |i: &Rgba| i.data.chunks_exact(4).filter(|p| p[3] >= 128).count();
            let diff = visible(&area) as i64 - visible(&lanczos) as i64;
            assert!(diff.abs() * 10 < visible(&area) as i64, "both cover the same shape");
        }
        // an exact integer reduction of blocky art gives the blocks back unchanged
        let mut blocks = Rgba::new(8, 8);
        blocks.set_px(2, 3, [255, 128, 100, 255]);
        let big = resize_area(&blocks, 64, 64).unwrap();
        assert_eq!(resize_area(&big, 8, 8).unwrap(), blocks);
    }

    #[test]
    fn det_sin_matches_std_closely() {
        for i in -400..400 {
            let x = f64::from(i) * 0.0173;
            assert!((det_sin(x) - x.sin()).abs() < 1e-9, "x={x}");
        }
    }

    #[test]
    fn identity_and_flat_colour_are_preserved() {
        let flat = Rgba::filled(20, 10, [90, 120, 200, 255]);
        let same = resize_lanczos3(&flat, 20, 10).unwrap();
        assert_eq!(same, flat);
        let down = resize_lanczos3(&flat, 7, 5).unwrap();
        assert!(down.data.chunks_exact(4).all(|p| p == [90, 120, 200, 255]));
        let up = resize_lanczos3(&flat, 41, 23).unwrap();
        assert!(up.data.chunks_exact(4).all(|p| p == [90, 120, 200, 255]));
    }

    #[test]
    fn transparent_pixels_do_not_bleed_colour() {
        // left half opaque red, right half transparent with green RGB that must not leak
        let mut img = Rgba::new(8, 2);
        for y in 0..2 {
            for x in 0..4 {
                img.set_px(x, y, [255, 0, 0, 255]);
            }
            for x in 4..8 {
                img.set_px(x, y, [0, 255, 0, 0]);
            }
        }
        let small = resize_lanczos3(&img, 4, 1).unwrap();
        for x in 0..4 {
            let p = small.px(x, 0);
            if p[3] > 0 {
                assert!(p[1] < 8, "green leaked into pixel {x}: {p:?}");
            }
        }
        assert_eq!(small.px(0, 0), [255, 0, 0, 255]);
        assert_eq!(small.px(3, 0)[3], 0);
    }

    #[test]
    fn deterministic_across_calls() {
        let mut img = Rgba::new(31, 17);
        for y in 0..17 {
            for x in 0..31 {
                img.set_px(x, y, [(x * 8) as u8, (y * 14) as u8, ((x * y) % 256) as u8, 255 - (x * 3) as u8]);
            }
        }
        let a = resize_lanczos3(&img, 13, 9).unwrap();
        let b = resize_lanczos3(&img, 13, 9).unwrap();
        assert_eq!(a, b);
        assert_eq!(a.pixels_sha256(), b.pixels_sha256());
        assert!(resize_lanczos3(&img, 0, 4).is_err());
    }
}
