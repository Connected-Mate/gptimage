//! Pillow-compatible primitives for the kit ports.
//!
//! The kit's Python tools lean on Pillow's exact integer arithmetic: `alpha_composite`, `ImageOps.autocontrast`,
//! `ImageOps.colorize`, `ImageEnhance`, `Image.blend`, the `L` conversion, and the bicubic affine transform behind
//! `Image.rotate` and `Image.transform`. Reproducing them bit for bit keeps the Rust recipes close to the originals
//! (a probe against Pillow 12 found zero mismatches for each of these). Everything is integer or IEEE-754 basic
//! arithmetic; trigonometry and `exp` use the crate's own series so results do not depend on the platform libm.
//!
//! Not reproduced bit for bit: Pillow's `GaussianBlur` (three extended box blurs in 24-bit fixed point) is
//! replaced by a true Gaussian with the same sigma, and Pillow's Lanczos resize keeps an 8-bit intermediate between
//! its two passes where [`crate::resize::resize_lanczos3`] stays in floating point. The parity tests measure both.

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::resize::det_sin;

const PI: f64 = std::f64::consts::PI;

/// Cosine from the platform-independent sine.
pub fn det_cos(x: f64) -> f64 {
    det_sin(x + PI / 2.0)
}

/// `exp` from range reduction and a Taylor series; identical on every platform.
pub fn det_exp(x: f64) -> f64 {
    if x < -700.0 {
        return 0.0;
    }
    let ln2 = std::f64::consts::LN_2;
    let k = (x / ln2).round();
    let r = x - k * ln2;
    let mut term = 1.0;
    let mut sum = 1.0;
    for n in 1..=24 {
        term *= r / f64::from(n);
        sum += term;
    }
    let k = k as i64;
    // 2^k from the exponent bits: exact
    let scale = f64::from_bits(((1023 + k) as u64) << 52);
    sum * scale
}

/// Python's `round(x)` on a float: halves go to the even neighbour.
pub fn py_round(x: f64) -> i64 {
    x.round_ties_even() as i64
}

// ---------------------------------------------------------------- Mersenne Twister

/// MT19937 seeded exactly like CPython's `random.Random(int)`, so the grain matches the Python tool byte for byte.
#[derive(Debug, Clone)]
pub struct PyRandom {
    mt: [u32; 624],
    index: usize,
}

impl PyRandom {
    /// `random.Random(seed)` for an integer seed (the sign is ignored, as CPython does).
    pub fn new(seed: i64) -> Self {
        let mut n = seed.unsigned_abs();
        let mut key: Vec<u32> = Vec::new();
        if n == 0 {
            key.push(0);
        }
        while n > 0 {
            key.push((n & 0xFFFF_FFFF) as u32);
            n >>= 32;
        }
        let mut rng = PyRandom { mt: [0; 624], index: 624 };
        rng.init_by_array(&key);
        rng
    }

    fn init_genrand(&mut self, s: u32) {
        self.mt[0] = s;
        for i in 1..624 {
            let prev = self.mt[i - 1];
            self.mt[i] = 1_812_433_253u32.wrapping_mul(prev ^ (prev >> 30)).wrapping_add(i as u32);
        }
        self.index = 624;
    }

    fn init_by_array(&mut self, key: &[u32]) {
        self.init_genrand(19_650_218);
        let (mut i, mut j) = (1usize, 0usize);
        let mut k = 624usize.max(key.len());
        while k > 0 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_664_525)).wrapping_add(key[j]).wrapping_add(j as u32);
            i += 1;
            j += 1;
            if i >= 624 {
                self.mt[0] = self.mt[623];
                i = 1;
            }
            if j >= key.len() {
                j = 0;
            }
            k -= 1;
        }
        k = 623;
        while k > 0 {
            let prev = self.mt[i - 1];
            self.mt[i] = (self.mt[i] ^ (prev ^ (prev >> 30)).wrapping_mul(1_566_083_941)).wrapping_sub(i as u32);
            i += 1;
            if i >= 624 {
                self.mt[0] = self.mt[623];
                i = 1;
            }
            k -= 1;
        }
        self.mt[0] = 0x8000_0000;
        self.index = 624;
    }

    fn next_u32(&mut self) -> u32 {
        if self.index >= 624 {
            for kk in 0..624 {
                let y = (self.mt[kk] & 0x8000_0000) | (self.mt[(kk + 1) % 624] & 0x7FFF_FFFF);
                let mut v = self.mt[(kk + 397) % 624] ^ (y >> 1);
                if y & 1 != 0 {
                    v ^= 0x9908_B0DF;
                }
                self.mt[kk] = v;
            }
            self.index = 0;
        }
        let mut y = self.mt[self.index];
        self.index += 1;
        y ^= y >> 11;
        y ^= (y << 7) & 0x9D2C_5680;
        y ^= (y << 15) & 0xEFC6_0000;
        y ^= y >> 18;
        y
    }

    /// `random._randbelow(n)` for `1 <= n < 2^32`: rejection sampling on `getrandbits(bit_length(n))`.
    fn randbelow(&mut self, n: u32) -> u32 {
        let k = 32 - n.leading_zeros();
        loop {
            let r = self.next_u32() >> (32 - k);
            if r < n {
                return r;
            }
        }
    }

    /// `random.randint(lo, hi)`.
    pub fn randint(&mut self, lo: i32, hi: i32) -> i32 {
        lo + self.randbelow((hi - lo + 1) as u32) as i32
    }
}

// ---------------------------------------------------------------- compositing

/// Pillow's integer `Image.alpha_composite` for one pixel: `s` over `d` (7 extra bits of precision).
pub fn over_pillow(s: [u8; 4], d: [u8; 4]) -> [u8; 4] {
    let sa = u32::from(s[3]);
    if sa == 0 {
        return d;
    }
    let shift = |a: u32| ((a >> 8) + a) >> 8;
    let blend = u32::from(d[3]) * (255 - sa);
    let outa255 = sa * 255 + blend;
    let coef1 = sa * 255 * 255 * 128 / outa255;
    let coef2 = 255 * 128 - coef1;
    let mut out = [0u8; 4];
    for c in 0..3 {
        let t = u32::from(s[c]) * coef1 + u32::from(d[c]) * coef2;
        out[c] = (shift(t + (0x80 << 7)) >> 7) as u8;
    }
    out[3] = shift(outa255 + 0x80) as u8;
    out
}

/// `dst.alpha_composite(src, (x, y))`, clipped to `dst`.
pub fn alpha_composite(dst: &mut Rgba, src: &Rgba, x: i64, y: i64) {
    for sy in 0..i64::from(src.height) {
        let dy = y + sy;
        if dy < 0 || dy >= i64::from(dst.height) {
            continue;
        }
        for sx in 0..i64::from(src.width) {
            let dx = x + sx;
            if dx < 0 || dx >= i64::from(dst.width) {
                continue;
            }
            let s = src.px(sx as u32, sy as u32);
            let d = dst.px(dx as u32, dy as u32);
            dst.set_px(dx as u32, dy as u32, over_pillow(s, d));
        }
    }
}

// ---------------------------------------------------------------- tone

/// Pillow's `L` conversion.
#[inline]
pub fn luma(r: u8, g: u8, b: u8) -> u8 {
    ((u32::from(r) * 19595 + u32::from(g) * 38470 + u32::from(b) * 7471 + 0x8000) >> 16) as u8
}

/// `ImageOps.autocontrast(rgb, cutoff=cutoff_percent)` applied per channel to the RGB of `img`; alpha is untouched.
/// `cutoff_percent` is an integer percent, as the kit uses (`cutoff=1`).
pub fn autocontrast_rgb(img: &mut Rgba, cutoff_percent: u64) {
    let mut hist = [[0u64; 256]; 3];
    for p in img.data.chunks_exact(4) {
        for c in 0..3 {
            hist[c][usize::from(p[c])] += 1;
        }
    }
    let mut luts = [[0u8; 256]; 3];
    for c in 0..3 {
        let mut h = hist[c];
        if cutoff_percent > 0 {
            let n: u64 = h.iter().sum();
            let mut cut = n * cutoff_percent / 100;
            for v in h.iter_mut() {
                if cut > *v {
                    cut -= *v;
                    *v = 0;
                } else {
                    *v -= cut;
                    cut = 0;
                }
                if cut == 0 {
                    break;
                }
            }
            let mut cut = n * cutoff_percent / 100;
            for v in h.iter_mut().rev() {
                if cut > *v {
                    cut -= *v;
                    *v = 0;
                } else {
                    *v -= cut;
                    cut = 0;
                }
                if cut == 0 {
                    break;
                }
            }
        }
        let lo = h.iter().position(|v| *v != 0).unwrap_or(0);
        let hi = h.iter().rposition(|v| *v != 0).unwrap_or(0);
        if hi <= lo {
            for (i, v) in luts[c].iter_mut().enumerate() {
                *v = i as u8;
            }
        } else {
            let scale = 255.0 / (hi - lo) as f64;
            let offset = -(lo as f64) * scale;
            for (i, v) in luts[c].iter_mut().enumerate() {
                let x = (i as f64 * scale + offset).trunc();
                *v = x.clamp(0.0, 255.0) as u8;
            }
        }
    }
    for p in img.data.chunks_exact_mut(4) {
        for c in 0..3 {
            p[c] = luts[c][usize::from(p[c])];
        }
    }
}

/// One output of `Image.blend(in1, in2, alpha)` for 8-bit samples: float32 arithmetic, truncated, clamped.
#[inline]
pub fn blend_u8(in1: u8, in2: u8, alpha: f32) -> u8 {
    let t = f32::from(in1) + alpha * (f32::from(in2) - f32::from(in1));
    if t <= 0.0 {
        0
    } else if t >= 255.0 {
        255
    } else {
        t as u8
    }
}

/// `ImageEnhance.Contrast(rgb).enhance(factor)` on the RGB of `img`.
pub fn enhance_contrast(img: &mut Rgba, factor: f32) {
    let mut sum = 0u64;
    for p in img.data.chunks_exact(4) {
        sum += u64::from(luma(p[0], p[1], p[2]));
    }
    let count = u64::from(img.width) * u64::from(img.height);
    let mean = (sum as f64 / count as f64 + 0.5) as i64 as u8;
    for p in img.data.chunks_exact_mut(4) {
        for v in &mut p[..3] {
            *v = blend_u8(mean, *v, factor);
        }
    }
}

/// `ImageEnhance.Brightness(rgb).enhance(factor)` on the RGB of `img`.
pub fn enhance_brightness(img: &mut Rgba, factor: f32) {
    for p in img.data.chunks_exact_mut(4) {
        for v in &mut p[..3] {
            *v = blend_u8(0, *v, factor);
        }
    }
}

/// `ImageOps.colorize` lookup table for a grey level: `black + i * (white - black) // 255`, floor division.
pub fn colorize_lut(black: [u8; 3], white: [u8; 3]) -> [[u8; 3]; 256] {
    let mut lut = [[0u8; 3]; 256];
    for (i, entry) in lut.iter_mut().enumerate() {
        for c in 0..3 {
            entry[c] = if i == 255 {
                white[c]
            } else {
                (i32::from(black[c]) + (i as i32 * (i32::from(white[c]) - i32::from(black[c]))).div_euclid(255)) as u8
            };
        }
    }
    lut
}

// ---------------------------------------------------------------- premultiplied alpha

#[inline]
fn muldiv255(a: u32, b: u32) -> u32 {
    let t = a * b + 128;
    ((t >> 8) + t) >> 8
}

/// Pillow's `RGBA` to `RGBa` conversion.
pub fn premultiply(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for v in &mut p[..3] {
            *v = muldiv255(u32::from(*v), a) as u8;
        }
    }
    out
}

/// Pillow's `RGBa` to `RGBA` conversion (truncating division; alpha 0 gives black).
pub fn unpremultiply(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for v in &mut p[..3] {
            *v = (255 * u32::from(*v)).checked_div(a).map_or(0, |q| q.min(255) as u8);
        }
    }
    out
}

// ---------------------------------------------------------------- affine transform

#[inline]
fn bicubic(v1: f64, v2: f64, v3: f64, v4: f64, d: f64) -> f64 {
    let p1 = v2;
    let p2 = -v1 + v3;
    let p3 = 2.0 * (v1 - v2) + v3 - v4;
    let p4 = -v1 + v2 - v3 + v4;
    p1 + (d * (p2 + d * (p3 + d * p4)))
}

/// `Image.transform(size, AFFINE, (a, b, c, d, e, f), resample=BICUBIC, fillcolor=(0, 0, 0, 0))` for RGBA, with
/// Pillow's coordinate convention (pixel centres at +0.5), its a = -1 cubic in Horner form (exact on flat areas),
/// premultiplied alpha, clamped taps and truncating 8-bit output.
pub fn affine_bicubic(src: &Rgba, size: (u32, u32), m: [f64; 6]) -> Result<Rgba> {
    if size.0 == 0 || size.1 == 0 || size.0 > 16384 || size.1 > 16384 {
        return Err(Error::Image(format!("cannot transform to {}x{}", size.0, size.1)));
    }
    let pre = premultiply(src);
    let (w, h) = (i64::from(src.width), i64::from(src.height));
    let mut out = Rgba::new(size.0, size.1);
    let [a, b, c, d, e, f] = m;
    for y in 0..size.1 {
        for x in 0..size.0 {
            let fx = f64::from(x) + 0.5;
            let fy = f64::from(y) + 0.5;
            let mut xin = a * fx + b * fy + c;
            let mut yin = d * fx + e * fy + f;
            if xin < 0.0 || xin >= w as f64 || yin < 0.0 || yin >= h as f64 {
                continue;
            }
            xin -= 0.5;
            yin -= 0.5;
            let x0 = xin.floor();
            let y0 = yin.floor();
            let dx = xin - x0;
            let dy = yin - y0;
            let (x0, y0) = (x0 as i64, y0 as i64);
            let xs = [-1i64, 0, 1, 2].map(|i| (x0 + i).clamp(0, w - 1) as usize);
            let ys = [-1i64, 0, 1, 2].map(|j| (y0 + j).clamp(0, h - 1) as usize);
            let mut px = [0u8; 4];
            for (ch, slot) in px.iter_mut().enumerate() {
                let mut rows = [0f64; 4];
                for (r, yy) in ys.iter().enumerate() {
                    let at = |xx: usize| f64::from(pre.data[(yy * src.width as usize + xx) * 4 + ch]);
                    rows[r] = bicubic(at(xs[0]), at(xs[1]), at(xs[2]), at(xs[3]), dx);
                }
                let v = bicubic(rows[0], rows[1], rows[2], rows[3], dy);
                *slot = if v <= 0.0 {
                    0
                } else if v >= 255.0 {
                    255
                } else {
                    v as u8
                };
            }
            out.set_px(x, y, px);
        }
    }
    Ok(unpremultiply(&out))
}

/// `Image.rotate(angle, resample=BICUBIC, expand=True)`: counter-clockwise degrees, the canvas grows to hold the card.
pub fn rotate_bicubic_expand(src: &Rgba, angle: f64) -> Result<Rgba> {
    let angle = angle.rem_euclid(360.0);
    if angle == 0.0 {
        return Ok(src.clone());
    }
    if angle == 180.0 {
        let mut out = Rgba::new(src.width, src.height);
        for y in 0..src.height {
            for x in 0..src.width {
                out.set_px(src.width - 1 - x, src.height - 1 - y, src.px(x, y));
            }
        }
        return Ok(out);
    }
    if angle == 90.0 || angle == 270.0 {
        let mut out = Rgba::new(src.height, src.width);
        for y in 0..src.height {
            for x in 0..src.width {
                if angle == 90.0 {
                    out.set_px(y, src.width - 1 - x, src.px(x, y));
                } else {
                    out.set_px(src.height - 1 - y, x, src.px(x, y));
                }
            }
        }
        return Ok(out);
    }
    let (w, h) = (f64::from(src.width), f64::from(src.height));
    let center = (w / 2.0, h / 2.0);
    let rad = -angle * PI / 180.0;
    let r15 = |v: f64| (v * 1e15).round() / 1e15;
    let (cs, sn) = (r15(det_cos(rad)), r15(det_sin(rad)));
    let mut m = [cs, sn, 0.0, -sn, cs, 0.0];
    let tr = |x: f64, y: f64, m: &[f64; 6]| (m[0] * x + m[1] * y + m[2], m[3] * x + m[4] * y + m[5]);
    let (c, f) = tr(-center.0, -center.1, &m);
    m[2] = c + center.0;
    m[5] = f + center.1;
    let corners = [(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)].map(|(x, y)| tr(x, y, &m));
    let max_x = corners.iter().map(|p| p.0).fold(f64::MIN, f64::max);
    let min_x = corners.iter().map(|p| p.0).fold(f64::MAX, f64::min);
    let max_y = corners.iter().map(|p| p.1).fold(f64::MIN, f64::max);
    let min_y = corners.iter().map(|p| p.1).fold(f64::MAX, f64::min);
    let nw = (max_x.ceil() - min_x.floor()) as i64;
    let nh = (max_y.ceil() - min_y.floor()) as i64;
    let (c, f) = tr(-(nw as f64 - w) / 2.0, -(nh as f64 - h) / 2.0, &m);
    m[2] = c;
    m[5] = f;
    affine_bicubic(src, (nw.max(1) as u32, nh.max(1) as u32), m)
}

// ---------------------------------------------------------------- Gaussian blur

/// Separable true Gaussian of an 8-bit plane, replicated edges, kernel radius `ceil(3 sigma)`, rounded to nearest.
/// Pillow's `GaussianBlur(radius)` takes `radius` as the standard deviation, so `sigma` is the same number.
pub fn gaussian_blur_plane(plane: &[u8], w: usize, h: usize, sigma: f64) -> Vec<u8> {
    if sigma <= 0.0 || w == 0 || h == 0 {
        return plane.to_vec();
    }
    let radius = ((3.0 * sigma).ceil() as i64).max(1);
    let mut kernel: Vec<f64> = (-radius..=radius).map(|i| det_exp(-((i * i) as f64) / (2.0 * sigma * sigma))).collect();
    let total: f64 = kernel.iter().sum();
    for k in &mut kernel {
        *k /= total;
    }
    let mut mid = vec![0f64; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (k, wgt) in kernel.iter().enumerate() {
                let sx = (x as i64 + k as i64 - radius).clamp(0, w as i64 - 1) as usize;
                acc += f64::from(plane[y * w + sx]) * wgt;
            }
            mid[y * w + x] = acc;
        }
    }
    let mut out = vec![0u8; w * h];
    for y in 0..h {
        for x in 0..w {
            let mut acc = 0.0;
            for (k, wgt) in kernel.iter().enumerate() {
                let sy = (y as i64 + k as i64 - radius).clamp(0, h as i64 - 1) as usize;
                acc += mid[sy * w + x] * wgt;
            }
            out[y * w + x] = (acc + 0.5).floor().clamp(0.0, 255.0) as u8;
        }
    }
    out
}

/// The alpha channel as a plane.
pub fn alpha_plane(img: &Rgba) -> Vec<u8> {
    img.data.chunks_exact(4).map(|p| p[3]).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mersenne_twister_matches_cpython() {
        // Reference values from CPython 3.13.
        let mut r = PyRandom::new(0);
        let first: Vec<u32> = (0..3).map(|_| r.next_u32()).collect();
        assert_eq!(first, vec![3_626_764_237, 1_654_615_998, 3_255_389_356]);
        let mut r = PyRandom::new(1337);
        let noise: Vec<i32> = (0..12).map(|_| r.randint(-7, 7)).collect();
        assert_eq!(noise, vec![2, 7, 1, 4, -2, 2, 2, 4, -5, 5, 6, -2]);
        // seeds wider than 32 bits use a two-word key
        let mut r = PyRandom::new((1 << 40) + 5);
        assert_eq!((r.next_u32(), r.next_u32()), (2_166_296_868, 2_220_160_828));
    }

    #[test]
    fn det_exp_and_cos_track_std() {
        for i in -300..300 {
            let x = f64::from(i) * 0.037;
            assert!((det_exp(x) / x.exp() - 1.0).abs() < 1e-12, "exp {x}");
            assert!((det_cos(x) - x.cos()).abs() < 1e-9, "cos {x}");
        }
    }

    #[test]
    fn py_round_is_bankers() {
        assert_eq!(py_round(0.5), 0);
        assert_eq!(py_round(1.5), 2);
        assert_eq!(py_round(2.5), 2);
        assert_eq!(py_round(-0.5), 0);
        assert_eq!(py_round(192.6), 193);
    }

    #[test]
    fn composite_with_opaque_source_replaces_and_transparent_keeps() {
        assert_eq!(over_pillow([10, 20, 30, 255], [200, 200, 200, 255]), [10, 20, 30, 255]);
        assert_eq!(over_pillow([10, 20, 30, 0], [200, 100, 50, 77]), [200, 100, 50, 77]);
        let half = over_pillow([255, 255, 255, 128], [0, 0, 0, 255]);
        assert_eq!(half[3], 255);
        assert!((i32::from(half[0]) - 128).abs() <= 1);
    }

    #[test]
    fn premultiply_round_trip_keeps_opaque_pixels() {
        let img = Rgba::filled(3, 3, [12, 200, 99, 255]);
        assert_eq!(unpremultiply(&premultiply(&img)), img);
    }

    #[test]
    fn rotation_fast_paths_and_expand() {
        let mut img = Rgba::new(4, 2);
        img.set_px(0, 0, [1, 2, 3, 255]);
        let r90 = rotate_bicubic_expand(&img, 90.0).unwrap();
        assert_eq!((r90.width, r90.height), (2, 4));
        assert_eq!(r90.px(0, 3), [1, 2, 3, 255]); // counter-clockwise: top-left goes to bottom-left
        let r180 = rotate_bicubic_expand(&img, 180.0).unwrap();
        assert_eq!(r180.px(3, 1), [1, 2, 3, 255]);
        let r = rotate_bicubic_expand(&Rgba::filled(100, 80, [9, 9, 9, 255]), 4.2).unwrap();
        assert!(r.width > 100 && r.height > 80);
        assert_eq!(r.px(r.width / 2, r.height / 2), [9, 9, 9, 255]);
        assert_eq!(r.px(0, 0)[3], 0);
    }

    #[test]
    fn gaussian_keeps_mass_and_flat_fields() {
        let flat = vec![200u8; 10 * 10];
        assert_eq!(gaussian_blur_plane(&flat, 10, 10, 2.3), flat);
        let mut dot = vec![0u8; 21 * 21];
        dot[10 * 21 + 10] = 255;
        let b = gaussian_blur_plane(&dot, 21, 21, 1.5);
        assert!(b[10 * 21 + 10] < 255 && b[10 * 21 + 10] > 0);
        assert_eq!(b[10 * 21 + 9], b[10 * 21 + 11]);
        assert_eq!(gaussian_blur_plane(&dot, 21, 21, 0.0), dot);
    }

    #[test]
    fn autocontrast_stretches_a_narrow_range() {
        let mut img = Rgba::new(10, 10);
        for (i, p) in img.data.chunks_exact_mut(4).enumerate() {
            let v = 100 + (i % 20) as u8;
            p.copy_from_slice(&[v, v, v, 255]);
        }
        autocontrast_rgb(&mut img, 1);
        let min = img.data.chunks_exact(4).map(|p| p[0]).min().unwrap();
        let max = img.data.chunks_exact(4).map(|p| p[0]).max().unwrap();
        assert_eq!(min, 0);
        assert!(max >= 254, "{max}"); // Pillow truncates, so the top can land on 254
    }

    #[test]
    fn colorize_lut_hits_both_ends() {
        let lut = colorize_lut([0x28, 0x20, 0x18], [0xee, 0xe2, 0xc7]);
        assert_eq!(lut[0], [0x28, 0x20, 0x18]);
        assert_eq!(lut[255], [0xee, 0xe2, 0xc7]);
    }
}
