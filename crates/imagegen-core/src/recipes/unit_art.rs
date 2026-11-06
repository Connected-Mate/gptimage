//! Unit art: the battalion counter, the on-map counter and the division template symbol, made from a generated
//! silhouette and finished in vanilla's own look.
//!
//! Vanilla (HOI4 1.19.4.1) draws all three in one consistent material, so the model is asked for the shape only and
//! this module renders the material itself. That is what makes generated art match vanilla reliably: the model's
//! colours and shading never reach the game.
//!
//! **What vanilla shows** (measured over every file of `gfx/interface/counters/`, see the tests):
//!
//! * Frame 1 of a battalion counter (`divisions_large`, 111 strips of 152x42) and a large division symbol
//!   (`division_templates_large`, 101 files of 76x42) is one side-view silhouette in a single green hue: red equals
//!   blue on every opaque pixel and green follows red linearly (`g = 0.924 r + 34.6` on counters, `1.021 r + 26.0` on
//!   symbols). It is lit from above: a vertical gradient, a light rim on top and left edges and a dark rim on bottom
//!   and right edges. Under it lies a soft black halo (alpha 113, 54, 24, 6 at 1 to 4 px outside on counters). The
//!   colours are fixed: the division designer does not tint them.
//! * Frame 1 of an on-map counter (`divisions_small`, 83 strips of 60x12) is the same silhouette in light grey
//!   (red, green and blue equal, median 156) with a tight dark halo (alpha 80 at 1 px). The game multiplies it by the
//!   country colour, so it must stay grey and light.
//! * Frame 2 of both counters is a NATO box: a grey box with a vertical gradient, a black border and black
//!   symbol marks (a cross for infantry, an oval for armour, a dot for artillery, bars for the tank weight). The box
//!   is the same in every vanilla file: 47x25 at (14, 10) with a soft shadow on the battalion counter, 25x12 at
//!   (2, 0) without one on the on-map counter.
//! * Support companies add a "+": blue and bevelled at the top left of frame 1, black in the box.
//!
//! **How the art is made.** [`silhouette_coverage`] reads the shape from the generated image (the model draws one
//! solid dark silhouette on a plain flat background; real alpha works too). [`fit_glyph`] scales it to vanilla's
//! size (the median ink area, capped by the 90th percentile box) and centres it where vanilla's sit.
//! [`render_glyph`] lights it with the material model fitted to vanilla, and [`counter_strip`] adds frame 2 from
//! [`nato_box`] with the marks of the sub-unit's NATO symbol ([`parse_symbol`], [`infer_symbol`]).
//!
//! Everything is a pure function of the pixels and the parameters, deterministic on every platform.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pixels::{over, Rgba};

/// Version of the unit art recipes (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// Which unit art a step makes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum UnitArtKind {
    /// Battalion counter, frame 1 of the 152x42 strip (green).
    CounterLarge,
    /// On-map counter, frame 1 of the 60x12 strip (grey, tinted by the game).
    CounterSmall,
    /// Large division template symbol, 76x42 (green).
    DivisionEmblem,
}

impl UnitArtKind {
    /// Stable name.
    pub fn as_str(self) -> &'static str {
        match self {
            UnitArtKind::CounterLarge => "counter_large",
            UnitArtKind::CounterSmall => "counter_small",
            UnitArtKind::DivisionEmblem => "division_emblem",
        }
    }
}

// ------------------------------------------------------------------ planes

/// A single-channel float image.
#[derive(Debug, Clone, PartialEq)]
pub struct Plane {
    /// Width.
    pub w: usize,
    /// Height.
    pub h: usize,
    /// Values, row by row.
    pub v: Vec<f32>,
}

impl Plane {
    /// A plane of zeros.
    pub fn new(w: usize, h: usize) -> Plane {
        Plane { w, h, v: vec![0.0; w * h] }
    }

    fn at(&self, x: usize, y: usize) -> f32 {
        self.v[y * self.w + x]
    }

    /// Bounding box `[x0, y0, x1, y1)` of the values at or above `t`.
    pub fn bbox(&self, t: f32) -> Option<[usize; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0, 0);
        for y in 0..self.h {
            for x in 0..self.w {
                if self.at(x, y) >= t {
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        (x0 != usize::MAX).then_some([x0, y0, x1, y1])
    }

    fn crop(&self, b: [usize; 4]) -> Plane {
        let (w, h) = (b[2] - b[0], b[3] - b[1]);
        let mut out = Plane::new(w, h);
        for y in 0..h {
            out.v[y * w..(y + 1) * w].copy_from_slice(&self.v[(b[1] + y) * self.w + b[0]..(b[1] + y) * self.w + b[0] + w]);
        }
        out
    }
}

/// Gaussian blur with zeros outside the plane (kernel radius `ceil(4 sigma)`).
pub fn gaussian(p: &Plane, sigma: f64) -> Plane {
    if sigma <= 0.0 {
        return p.clone();
    }
    let r = (4.0 * sigma).ceil() as i64;
    let k: Vec<f64> = (-r..=r).map(|i| (-(i * i) as f64 / (2.0 * sigma * sigma)).exp()).collect();
    let s: f64 = k.iter().sum();
    let k: Vec<f64> = k.iter().map(|v| v / s).collect();
    let pass = |src: &Plane, horizontal: bool| {
        let mut out = Plane::new(src.w, src.h);
        for y in 0..src.h as i64 {
            for x in 0..src.w as i64 {
                let mut acc = 0f64;
                for (j, kv) in k.iter().enumerate() {
                    let o = j as i64 - r;
                    let (xx, yy) = if horizontal { (x + o, y) } else { (x, y + o) };
                    if xx >= 0 && yy >= 0 && xx < src.w as i64 && yy < src.h as i64 {
                        acc += kv * f64::from(src.at(xx as usize, yy as usize));
                    }
                }
                out.v[(y as usize) * src.w + x as usize] = acc as f32;
            }
        }
        out
    };
    pass(&pass(p, true), false)
}

/// Resample by exact area averaging (a box filter over the source footprint of each target pixel). Thin parts keep
/// their weight as partial coverage instead of vanishing, which is what keeps a 30x12 glyph readable.
pub fn area_resample(p: &Plane, w: usize, h: usize) -> Plane {
    fn weights(src: usize, dst: usize) -> Vec<Vec<(usize, f64)>> {
        let scale = src as f64 / dst as f64;
        (0..dst)
            .map(|i| {
                let (a, b) = (i as f64 * scale, (i as f64 + 1.0) * scale);
                let mut ws = Vec::new();
                let mut j = a.floor() as usize;
                while (j as f64) < b && j < src {
                    let overlap = (b.min(j as f64 + 1.0) - a.max(j as f64)).max(0.0);
                    if overlap > 0.0 {
                        ws.push((j, overlap / (b - a)));
                    }
                    j += 1;
                }
                ws
            })
            .collect()
    }
    let (wx, wy) = (weights(p.w, w), weights(p.h, h));
    let mut tmp = Plane::new(w, p.h);
    for y in 0..p.h {
        for (x, ws) in wx.iter().enumerate() {
            tmp.v[y * w + x] = ws.iter().map(|(j, f)| f64::from(p.at(*j, y)) * f).sum::<f64>() as f32;
        }
    }
    let mut out = Plane::new(w, h);
    for (y, ws) in wy.iter().enumerate() {
        for x in 0..w {
            out.v[y * w + x] = ws.iter().map(|(j, f)| f64::from(tmp.at(x, *j)) * f).sum::<f64>() as f32;
        }
    }
    out
}

/// Distance from each solid pixel to the nearest pixel that is not solid (pixel centres, outside the plane does
/// not count), up to `limit`; 0 off the mask.
fn inner_distance(mask: &[bool], w: usize, h: usize, limit: i64) -> Vec<f32> {
    let mut out = vec![0f32; w * h];
    for y in 0..h as i64 {
        for x in 0..w as i64 {
            if !mask[(y as usize) * w + x as usize] {
                continue;
            }
            let mut best = i64::MAX;
            for dy in -limit..=limit {
                for dx in -limit..=limit {
                    let (xx, yy) = (x + dx, y + dy);
                    if xx >= 0 && yy >= 0 && xx < w as i64 && yy < h as i64 && !mask[(yy as usize) * w + xx as usize] {
                        best = best.min(dx * dx + dy * dy);
                    }
                }
            }
            out[(y as usize) * w + x as usize] = if best == i64::MAX { limit as f32 + 1.0 } else { (best as f32).sqrt() };
        }
    }
    out
}

fn dilate(mask: &[bool], w: usize, h: usize, n: u32) -> Vec<bool> {
    let mut m = mask.to_vec();
    for _ in 0..n {
        let prev = m.clone();
        for y in 0..h {
            for x in 0..w {
                if prev[y * w + x] {
                    continue;
                }
                let near = (x > 0 && prev[y * w + x - 1])
                    || (x + 1 < w && prev[y * w + x + 1])
                    || (y > 0 && prev[(y - 1) * w + x])
                    || (y + 1 < h && prev[(y + 1) * w + x]);
                m[y * w + x] = near;
            }
        }
    }
    m
}

// ------------------------------------------------------------------ silhouette

/// Share of border pixels that may differ from the background colour before the background counts as not flat.
pub const BORDER_TOLERANCE: f64 = 0.12;

fn luminance(r: f64, g: f64, b: f64) -> f64 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// On an opaque image, how far from the dark shape's median luminance toward white a light mark inside it becomes a
/// hole (see [`silhouette_coverage`]).
pub const OPAQUE_HOLE_SHARE: f64 = 0.45;

/// Coverage of the silhouette in a generated image: 1 on the shape, 0 on the background, soft in between.
///
/// * An image with real transparency (at least 5 percent of its pixels clear) gives its alpha. When the shape is dark,
///   light marks drawn inside it (detail lines, gaps) become holes, as vanilla's engraved details are.
/// * An opaque image must have a flat background: its colour is the median of the two-pixel border, and coverage
///   rises from 30 to 60 percent of the strongest contrast against it. Light details inside a dark shape become holes
///   the same way.
pub fn silhouette_coverage(img: &Rgba) -> Result<Plane> {
    coverage_with(img, true)
}

/// [`silhouette_coverage`], with the coloured-mark holes of an opaque image on or off (the tests compare both).
fn coverage_with(img: &Rgba, coloured_holes: bool) -> Result<Plane> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 4 || h < 4 {
        return Err(Error::Image(format!("a {w}x{h} image is too small to read a silhouette from")));
    }
    let n = w * h;
    let clear = img.data.chunks_exact(4).filter(|p| p[3] < 16).count();
    let mut cov = Plane::new(w, h);
    if clear as f64 >= 0.05 * n as f64 {
        let mut lums: Vec<f64> =
            img.data.chunks_exact(4).filter(|p| p[3] >= 128).map(|p| luminance(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))).collect();
        if lums.is_empty() {
            return Err(Error::Image("the image has no visible shape: every pixel is transparent or nearly so".into()));
        }
        lums.sort_by(f64::total_cmp);
        let median = lums[lums.len() / 2];
        let hole = (median < 128.0).then_some(median + 0.6 * (255.0 - median));
        for (i, p) in img.data.chunks_exact(4).enumerate() {
            let mut c = f64::from(p[3]) / 255.0;
            if let Some(t) = hole {
                let l = luminance(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
                c *= 1.0 - ((l - (t - 20.0)) / 40.0).clamp(0.0, 1.0);
            }
            cov.v[i] = c as f32;
        }
    } else {
        // composite over white, then measure the contrast against the border colour
        let flat: Vec<[f64; 3]> = img
            .data
            .chunks_exact(4)
            .map(|p| {
                let a = f64::from(p[3]) / 255.0;
                [0, 1, 2].map(|c| f64::from(p[c]) * a + 255.0 * (1.0 - a))
            })
            .collect();
        let mut border = Vec::new();
        for y in 0..h {
            for x in 0..w {
                if x < 2 || y < 2 || x + 2 >= w || y + 2 >= h {
                    border.push(flat[y * w + x]);
                }
            }
        }
        let paper: [f64; 3] = [0, 1, 2].map(|c| {
            let mut v: Vec<f64> = border.iter().map(|p| p[c]).collect();
            v.sort_by(f64::total_cmp);
            v[v.len() / 2]
        });
        let diff = |p: &[f64; 3]| (0..3).map(|c| (p[c] - paper[c]).abs()).fold(0.0, f64::max);
        let off = border.iter().filter(|p| diff(p) > 32.0).count() as f64 / border.len() as f64;
        if off > BORDER_TOLERANCE {
            return Err(Error::Image(format!(
                "the background is not one flat colour ({:.0} percent of the border differs from it), so the silhouette cannot be separated from it. Generate the unit as one solid dark silhouette on a plain white background, or with a real transparent background",
                off * 100.0
            )));
        }
        let d: Vec<f64> = flat.iter().map(diff).collect();
        let mut sorted: Vec<f64> = d.iter().copied().filter(|v| *v > 48.0).collect();
        if sorted.len() < 16 {
            return Err(Error::Image("no silhouette stands out from the background: the image is blank or too faint".into()));
        }
        sorted.sort_by(f64::total_cmp);
        let ink = sorted[(sorted.len() - 1) * 9 / 10];
        let (lo, hi) = (0.3 * ink, 0.6 * ink);
        for (i, v) in d.iter().enumerate() {
            cov.v[i] = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
        }
        // light coloured marks inside a dark shape are holes, as on real alpha. Contrast with a white border alone
        // cannot see them: a yellow bolt on a blue shield differs from white as much as the shield does, so 0.3.1 drew the whole
        // shield and lost the bolt. Here the threshold sits lower than on alpha (0.45 of the way from the shape's
        // median to white, not 0.6) because a coloured mark is darker than a white gap: gold (250, 200, 40) has
        // luminance 197, a mid gold 152, against 150 for a shield of median 65.
        let lum: Vec<f64> = flat.iter().map(|p| luminance(p[0], p[1], p[2])).collect();
        let mut shape: Vec<f64> = lum.iter().zip(&cov.v).filter(|(_, c)| **c >= 0.5).map(|(l, _)| *l).collect();
        shape.sort_by(f64::total_cmp);
        if let Some(&median) = shape.get(shape.len() / 2).filter(|_| coloured_holes) {
            if median < 128.0 {
                let t = median + OPAQUE_HOLE_SHARE * (255.0 - median);
                // only coloured marks (chroma 60 and up): a white or grey mark in a black drawing already differs little
                // from white paper, and the shape's grey antialiasing and shading must stay exactly as they were
                for ((c, l), p) in cov.v.iter_mut().zip(&lum).zip(&flat) {
                    let chroma = p[0].max(p[1]).max(p[2]) - p[0].min(p[1]).min(p[2]);
                    if chroma >= 60.0 {
                        *c *= (1.0 - ((l - (t - 20.0)) / 40.0).clamp(0.0, 1.0)) as f32;
                    }
                }
            }
        }
    }
    if cov.bbox(0.5).is_none() {
        return Err(Error::Image("the image has no solid shape to use as the unit's silhouette".into()));
    }
    Ok(cov)
}

// ------------------------------------------------------------------ style

/// How luminance follows position on the glyph. `L = base + t * slope + rim terms`, where `t` runs from 0 at the
/// glyph's top row to 1 at its bottom row and the rim terms weigh the outward normal's up, down, left and right
/// parts on edge pixels (`rim1`, inner distance below 1.5) and the next pixel in (`rim2`, below 2.5). Fitted by least
/// squares on the red channel of every opaque glyph pixel of the vanilla set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Lighting {
    /// Luminance at the top row, away from the rim.
    pub base: f64,
    /// Change from the top row to the bottom row.
    pub slope: f64,
    /// Rim terms on edge pixels: up, down, left, right.
    pub rim1: [f64; 4],
    /// Rim terms one pixel further in.
    pub rim2: [f64; 4],
}

/// How luminance becomes colour.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Tint {
    /// Vanilla's green: red and blue equal `L`, green `k * L + c`.
    Green {
        /// Slope of green against red.
        k: f64,
        /// Offset of green.
        c: f64,
    },
    /// Grey: red, green and blue equal `L`.
    Grey,
    /// The blue of the support "+": dark, body and light colours, mixed by `L` around `mid`.
    Blue {
        /// Luminance of the body colour.
        mid: f64,
    },
}

/// The black halo under the glyph: `alpha = clamp(opacity * gaussian(sigma) + wide_opacity * gaussian(wide_sigma))`
/// of the mask dilated by `dilate`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Halo {
    /// Gaussian sigma.
    pub sigma: f64,
    /// Peak opacity before clamping.
    pub opacity: f64,
    /// Sigma of a second, wider term (the on-map counter's faint outer halo).
    pub wide_sigma: f64,
    /// Opacity of the wider term; 0 for none.
    pub wide_opacity: f64,
    /// Dilation of the mask in pixels.
    pub dilate: u32,
}

/// Everything that makes one kind of unit art look like vanilla's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GlyphStyle {
    /// Frame size.
    pub frame: [u32; 2],
    /// Centre of the glyph's bounding box (median of vanilla's).
    pub centre: [f64; 2],
    /// Largest glyph box (90th percentile of vanilla's widths and heights).
    pub max_box: [f64; 2],
    /// Ink area the glyph is scaled to (median of vanilla's opaque glyph pixels), unless the box caps it.
    pub ink_area: f64,
    /// Contrast applied to the resampled coverage around 0.5, so edges stay crisp as vanilla's are.
    pub sharpen: f64,
    /// Lighting.
    pub lighting: Lighting,
    /// Colour.
    pub tint: Tint,
    /// Halo.
    pub halo: Halo,
}

impl GlyphStyle {
    /// The style of a kind, fitted to HOI4 1.19.4.1.
    pub fn of(kind: UnitArtKind) -> GlyphStyle {
        match kind {
            // 111 battalion strips: ink area p10 367, p50 546, p90 809; width p50 47, p90 62; height p50 23, p90 30;
            // bounding box centre median (38.5, 22.5)
            UnitArtKind::CounterLarge => GlyphStyle {
                frame: [76, 42],
                centre: [38.5, 22.5],
                max_box: [62.0, 30.0],
                ink_area: 546.0,
                sharpen: 2.0,
                lighting: Lighting { base: 114.0, slope: -33.1, rim1: [42.4, -35.5, 10.5, -10.8], rim2: [19.4, -12.0, 12.1, -13.2] },
                tint: Tint::Green { k: 0.924, c: 34.6 },
                halo: Halo { sigma: 1.8, opacity: 1.239, wide_sigma: 0.0, wide_opacity: 0.0, dilate: 0 },
            },
            // 101 large symbols: ink area p10 200, p50 353, p90 568; width p50 27, p90 51; height p50 23, p90 26;
            // centre median (38, 21.5)
            UnitArtKind::DivisionEmblem => GlyphStyle {
                frame: [76, 42],
                centre: [38.0, 21.5],
                max_box: [51.0, 26.0],
                ink_area: 353.0,
                sharpen: 2.0,
                lighting: Lighting { base: 101.1, slope: -27.8, rim1: [46.2, -36.3, 22.2, -21.3], rim2: [13.4, -8.6, 12.1, -9.1] },
                tint: Tint::Green { k: 1.021, c: 26.0 },
                halo: Halo { sigma: 1.8, opacity: 1.159, wide_sigma: 0.0, wide_opacity: 0.0, dilate: 0 },
            },
            // 81 grey on-map strips: ink area p10 62, p50 98, p90 136; width p50 20, p90 25; height p50 9, p90 10;
            // centre median (15, 6). The halo is a tight dark outline plus a faint wider glow: alpha 80, 16, 6, 2 at 1 to
            // 4 px outside, which one Gaussian cannot follow
            UnitArtKind::CounterSmall => GlyphStyle {
                frame: [30, 12],
                centre: [15.0, 6.0],
                max_box: [25.0, 10.0],
                ink_area: 98.0,
                sharpen: 2.5,
                lighting: Lighting { base: 180.6, slope: -39.8, rim1: [6.0, -28.8, -2.2, -7.4], rim2: [8.3, -6.4, 7.8, -6.6] },
                tint: Tint::Grey,
                halo: Halo { sigma: 0.8, opacity: 1.0, wide_sigma: 2.0, wide_opacity: 0.3, dilate: 0 },
            },
        }
    }
}

// ------------------------------------------------------------------ fit and render

/// Scale a full-size coverage plane to the kind's vanilla size and place it on its frame.
pub fn fit_glyph(cov: &Plane, style: &GlyphStyle) -> Result<Plane> {
    let b = cov.bbox(0.5).ok_or_else(|| Error::Image("no solid shape to fit".into()))?;
    let sub = cov.crop(b);
    let (w, h) = (sub.w as f64, sub.h as f64);
    let ink: f64 = sub.v.iter().map(|v| f64::from(*v)).sum::<f64>().max(1.0);
    let s = (style.max_box[0] / w).min(style.max_box[1] / h).min((style.ink_area / ink).sqrt());
    let [fw, fh] = [style.frame[0] as usize, style.frame[1] as usize];
    let tw = ((w * s).round() as usize).clamp(1, fw);
    let th = ((h * s).round() as usize).clamp(1, fh);
    let mut small = area_resample(&sub, tw, th);
    for v in small.v.iter_mut() {
        *v = ((f64::from(*v) - 0.5) * style.sharpen + 0.5).clamp(0.0, 1.0) as f32;
    }
    let ox = ((style.centre[0] - tw as f64 / 2.0).round().max(0.0) as usize).min(fw - tw);
    let oy = ((style.centre[1] - th as f64 / 2.0).round().max(0.0) as usize).min(fh - th);
    let mut out = Plane::new(fw, fh);
    for y in 0..th {
        for x in 0..tw {
            out.v[(oy + y) * fw + ox + x] = small.at(x, y);
        }
    }
    if out.bbox(0.5).is_none() {
        return Err(Error::Image("the silhouette vanishes at the unit art's size: it is too thin. Draw a bolder, simpler shape".into()));
    }
    Ok(out)
}

const BLUE_DARK: [f64; 3] = [34.0, 48.0, 92.0];
const BLUE_BODY: [f64; 3] = [85.0, 121.0, 222.0];
const BLUE_LIGHT: [f64; 3] = [174.0, 199.0, 242.0];

fn colour_of(tint: Tint, l: f64) -> [u8; 3] {
    let q = |v: f64| v.round().clamp(0.0, 255.0) as u8;
    match tint {
        Tint::Green { k, c } => {
            let r = l.clamp(0.0, 255.0);
            [q(r), q(k * r + c), q(r)]
        }
        Tint::Grey => [q(l), q(l), q(l)],
        Tint::Blue { mid } => {
            let (a, b, t) = if l <= mid { (BLUE_DARK, BLUE_BODY, (l - (mid - 50.0)) / 50.0) } else { (BLUE_BODY, BLUE_LIGHT, (l - mid) / 45.0) };
            let t = t.clamp(0.0, 1.0);
            [q(a[0] + (b[0] - a[0]) * t), q(a[1] + (b[1] - a[1]) * t), q(a[2] + (b[2] - a[2]) * t)]
        }
    }
}

/// Light a frame-sized coverage plane in the style's material and put its halo beneath it.
pub fn render_glyph(cov: &Plane, style: &GlyphStyle) -> Result<Rgba> {
    let (w, h) = (cov.w, cov.h);
    let mask: Vec<bool> = cov.v.iter().map(|v| *v >= 0.5).collect();
    let rows: Vec<usize> = (0..h).filter(|y| (0..w).any(|x| mask[y * w + x])).collect();
    let (Some(&y0), Some(&y1)) = (rows.first(), rows.last()) else {
        return Err(Error::Image("no solid shape to render".into()));
    };
    let inner = inner_distance(&mask, w, h, 3);
    let solid = Plane { w, h, v: mask.iter().map(|m| if *m { 1.0 } else { 0.0 }).collect() };
    let blur = gaussian(&solid, 1.0);
    // halo from the binary mask, as fitted
    let halo_src = Plane { w, h, v: dilate(&mask, w, h, style.halo.dilate).iter().map(|m| if *m { 1.0 } else { 0.0 }).collect() };
    let halo = gaussian(&halo_src, style.halo.sigma);
    let wide = (style.halo.wide_opacity > 0.0).then(|| gaussian(&halo_src, style.halo.wide_sigma));
    let lt = style.lighting;
    let mut out = Rgba::new(w as u32, h as u32);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            let extra = wide.as_ref().map_or(0.0, |p| f64::from(p.v[i]) * style.halo.wide_opacity);
            let sa = (f64::from(halo.v[i]) * style.halo.opacity + extra).clamp(0.0, 1.0);
            let shadow = [0, 0, 0, (sa * 255.0).round() as u8];
            let c = f64::from(cov.v[i]);
            if c <= 0.0 {
                out.set_px(x as u32, y as u32, shadow);
                continue;
            }
            // outward normal from the blurred mask (central differences, one-sided at the border)
            let g = |xx: usize, yy: usize| f64::from(blur.at(xx, yy));
            let gx = match (x > 0, x + 1 < w) {
                (true, true) => (g(x + 1, y) - g(x - 1, y)) / 2.0,
                (false, true) => g(x + 1, y) - g(x, y),
                (true, false) => g(x, y) - g(x - 1, y),
                _ => 0.0,
            };
            let gy = match (y > 0, y + 1 < h) {
                (true, true) => (g(x, y + 1) - g(x, y - 1)) / 2.0,
                (false, true) => g(x, y + 1) - g(x, y),
                (true, false) => g(x, y) - g(x, y - 1),
                _ => 0.0,
            };
            let n = (gx * gx + gy * gy).sqrt() + 1e-9;
            let (nx, ny) = (-gx / n, -gy / n);
            let parts = [(-ny).max(0.0), ny.max(0.0), (-nx).max(0.0), nx.max(0.0)];
            let d = f64::from(inner[i]);
            let t = if y1 > y0 { ((y as f64 - y0 as f64) / (y1 - y0) as f64).clamp(0.0, 1.0) } else { 0.0 };
            let mut l = lt.base + lt.slope * t;
            let rim = if d < 1.5 {
                Some(lt.rim1)
            } else if d < 2.5 {
                Some(lt.rim2)
            } else {
                None
            };
            if let Some(r) = rim {
                l += parts.iter().zip(r).map(|(p, k)| p * k).sum::<f64>();
            }
            let rgb = colour_of(style.tint, l);
            let glyph = [rgb[0], rgb[1], rgb[2], (c * 255.0).round() as u8];
            out.set_px(x as u32, y as u32, over(glyph, shadow));
        }
    }
    Ok(out)
}

/// The finished frame of a kind from a generated image: silhouette, fit, material.
pub fn unit_glyph(img: &Rgba, kind: UnitArtKind) -> Result<Rgba> {
    let style = GlyphStyle::of(kind);
    let cov = silhouette_coverage(img)?;
    let fitted = fit_glyph(&cov, &style)?;
    render_glyph(&fitted, &style)
}

// ------------------------------------------------------------------ NATO box

/// One mark of a NATO symbol, as vanilla draws them in frame 2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Mark {
    /// Infantry: corner-to-corner cross.
    Cross,
    /// The cross with its middle left open (paratroopers, marines).
    CrossOpen,
    /// Armour: a stadium oval.
    Oval,
    /// Artillery: a centred dot.
    Dot,
    /// Motorised: two wheels on the bottom edge.
    Wheels,
    /// Anti-tank: a V from the top corners to the bottom centre.
    AntiTank,
    /// Anti-air: an arch from the bottom corners to the top centre with a cross bar.
    AntiAir,
    /// Cavalry and recon: one diagonal.
    Slash,
    /// Engineers: a bridge.
    Bridge,
    /// Mountain: a filled triangle on the bottom edge.
    Mountain,
    /// Airborne: two arcs in the middle.
    Airborne,
    /// Rocket artillery: two chevrons over a dot.
    Rocket,
    /// Medium tank: a thin bar at the left.
    BarMedium,
    /// Heavy tank: a thick bar at the left.
    BarHeavy,
    /// Super-heavy tank: a very thick bar at the left.
    BarSuperHeavy,
    /// Modern tank: bars at both sides.
    BarModern,
    /// Support company: a "+" at the top left (and a blue one on frame 1 of the battalion counter).
    Plus,
}

impl Mark {
    /// Every mark, for listing.
    pub const ALL: [Mark; 17] = [
        Mark::Cross,
        Mark::CrossOpen,
        Mark::Oval,
        Mark::Dot,
        Mark::Wheels,
        Mark::AntiTank,
        Mark::AntiAir,
        Mark::Slash,
        Mark::Bridge,
        Mark::Mountain,
        Mark::Airborne,
        Mark::Rocket,
        Mark::BarMedium,
        Mark::BarHeavy,
        Mark::BarSuperHeavy,
        Mark::BarModern,
        Mark::Plus,
    ];

    /// Stable name.
    pub fn name(self) -> &'static str {
        match self {
            Mark::Cross => "cross",
            Mark::CrossOpen => "cross_open",
            Mark::Oval => "oval",
            Mark::Dot => "dot",
            Mark::Wheels => "wheels",
            Mark::AntiTank => "anti_tank_v",
            Mark::AntiAir => "anti_air_arch",
            Mark::Slash => "slash",
            Mark::Bridge => "bridge",
            Mark::Mountain => "mountain_peak",
            Mark::Airborne => "airborne_arcs",
            Mark::Rocket => "rocket_chevrons",
            Mark::BarMedium => "bar_medium",
            Mark::BarHeavy => "bar_heavy",
            Mark::BarSuperHeavy => "bar_super_heavy",
            Mark::BarModern => "bar_modern",
            Mark::Plus => "plus",
        }
    }
}

/// Named NATO symbols, the way vanilla marks its sub-units (each checked against the vanilla strip of that name).
pub const PRESETS: &[(&str, &[Mark])] = &[
    ("infantry", &[Mark::Cross]),
    ("motorized", &[Mark::Cross, Mark::Wheels]),
    ("mechanized", &[Mark::Cross, Mark::Oval]),
    ("light_armor", &[Mark::Oval]),
    ("medium_armor", &[Mark::BarMedium, Mark::Oval]),
    ("heavy_armor", &[Mark::BarHeavy, Mark::Oval]),
    ("modern_armor", &[Mark::BarModern, Mark::Oval]),
    ("super_heavy_armor", &[Mark::BarSuperHeavy, Mark::Oval]),
    ("artillery", &[Mark::Dot]),
    ("motorized_artillery", &[Mark::Dot, Mark::Wheels]),
    ("anti_tank", &[Mark::AntiTank, Mark::Dot]),
    ("motorized_anti_tank", &[Mark::AntiTank, Mark::Dot, Mark::Wheels]),
    ("anti_air", &[Mark::AntiAir]),
    ("motorized_anti_air", &[Mark::AntiAir, Mark::Wheels]),
    ("cavalry", &[Mark::Slash]),
    ("recon", &[Mark::Slash]),
    ("engineer", &[Mark::Bridge]),
    ("mountain", &[Mark::Cross, Mark::Mountain]),
    ("paratroop", &[Mark::CrossOpen, Mark::Airborne]),
    ("rocket_artillery", &[Mark::Rocket]),
    ("support", &[Mark::Plus]),
];

fn canonical(marks: &[Mark]) -> Vec<Mark> {
    let mut m = marks.to_vec();
    m.sort();
    m.dedup();
    m
}

/// The canonical text of a symbol: its marks joined by `+`, in a fixed order. This is what the record stores.
pub fn symbol_text(marks: &[Mark]) -> String {
    canonical(marks).iter().map(|m| m.name()).collect::<Vec<_>>().join("+")
}

/// Parse a symbol: preset names and mark names joined by `+`, for example `medium_armor`, `infantry+support` or
/// `cross+wheels+plus`.
pub fn parse_symbol(text: &str) -> Result<Vec<Mark>> {
    let mut marks = Vec::new();
    for part in text.split('+').map(|s| s.trim().to_ascii_lowercase()) {
        if part.is_empty() {
            continue;
        }
        if let Some((_, m)) = PRESETS.iter().find(|(n, _)| *n == part) {
            marks.extend_from_slice(m);
        } else if let Some(m) = Mark::ALL.iter().find(|m| m.name() == part) {
            marks.push(*m);
        } else {
            let presets: Vec<&str> = PRESETS.iter().map(|(n, _)| *n).collect();
            let names: Vec<&str> = Mark::ALL.iter().map(|m| m.name()).collect();
            return Err(Error::Invalid(format!(
                "'{part}' is not a NATO symbol. Use a preset ({}) or marks ({}), joined by +",
                presets.join(", "),
                names.join(", ")
            )));
        }
    }
    if marks.is_empty() {
        return Err(Error::Invalid("the NATO symbol is empty".into()));
    }
    Ok(canonical(&marks))
}

/// The symbol a sub-unit key suggests, the way vanilla marks such sub-units (an infantry cross when nothing
/// matches). Words are the key's parts between underscores.
pub fn infer_symbol(asset: &str) -> Vec<Mark> {
    let lower = asset.to_ascii_lowercase();
    let words: Vec<&str> = lower.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()).collect();
    let has = |w: &str| words.contains(&w);
    let any = |ws: &[&str]| ws.iter().any(|w| has(w));
    let mut marks: Vec<Mark> = if any(&["spaa", "aa", "antiair"]) || (has("anti") && has("air")) {
        vec![Mark::AntiAir]
    } else if any(&["at", "antitank", "destroyer"]) || (has("anti") && has("tank")) {
        vec![Mark::AntiTank, Mark::Dot]
    } else if any(&["rocket", "rockets"]) {
        vec![Mark::Rocket]
    } else if any(&["art", "artillery", "spart", "howitzer", "guns", "gun"]) {
        vec![Mark::Dot]
    } else if any(&["engineer", "engineers", "pioneer", "pioneers", "sapper", "sappers"]) {
        vec![Mark::Bridge]
    } else if any(&["paratroop", "paratrooper", "paratroopers", "airborne", "para", "parachute"]) {
        vec![Mark::CrossOpen, Mark::Airborne]
    } else if any(&["mountain", "mountaineers", "alpine", "ski"]) {
        vec![Mark::Cross, Mark::Mountain]
    } else if any(&["cavalry", "recon", "camelry", "horse", "scout", "scouts", "reconnaissance"]) {
        vec![Mark::Slash]
    } else if any(&["mechanized", "mech", "mechanised"]) {
        vec![Mark::Cross, Mark::Oval]
    } else if any(&["armor", "armour", "tank", "tanks", "panzer"]) {
        let bar = if has("super") && has("heavy") {
            Some(Mark::BarSuperHeavy)
        } else if has("heavy") {
            Some(Mark::BarHeavy)
        } else if has("medium") {
            Some(Mark::BarMedium)
        } else if has("modern") {
            Some(Mark::BarModern)
        } else {
            None
        };
        bar.into_iter().chain([Mark::Oval]).collect()
    } else {
        vec![Mark::Cross]
    };
    if any(&["mot", "motorized", "motorised", "truck", "trucks"]) && !marks.contains(&Mark::Oval) {
        marks.push(Mark::Wheels);
    }
    if has("support") {
        marks.push(Mark::Plus);
    }
    canonical(&marks)
}

/// Large (battalion) or small (on-map) box.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoxSize {
    /// 76x42 frame, 47x25 box at (14, 10) with a soft shadow.
    Large,
    /// 30x12 frame, 25x12 box at (2, 0).
    Small,
}

/// Interior row values of the large box, rows 11 to 33 (the median of the 111 vanilla strips; the top row is the
/// highlight, the bottom row the shade).
pub const LARGE_BOX_ROWS: [u8; 23] =
    [238, 216, 214, 213, 210, 207, 204, 201, 197, 193, 189, 185, 181, 177, 173, 169, 165, 162, 158, 155, 153, 150, 117];
/// Interior row values of the small box, rows 1 to 10 (median of the 83 vanilla strips).
pub const SMALL_BOX_ROWS: [u8; 10] = [227, 221, 215, 208, 198, 190, 180, 171, 163, 158];
/// Large box shadow: peak alpha and the falloff 1 to 4 px outside its full-strength rectangle (x 17 to 59, y 13 to
/// 33), applied per axis and multiplied (the vanilla shadow is a blurred rectangle offset half a pixel down right).
pub const LARGE_SHADOW_PEAK: f64 = 147.0;
/// See [`LARGE_SHADOW_PEAK`].
pub const LARGE_SHADOW_FALLOFF: [f64; 4] = [0.894, 0.66, 0.32, 0.106];

struct Geometry {
    frame: [u32; 2],
    // continuous interior rectangle
    x0: f64,
    x1: f64,
    y0: f64,
    y1: f64,
    line: f64,
    dot: f64,
    wheels: [[f64; 2]; 2],
    wheel_r: f64,
    aa_bar: f64,
    arc_r: f64,
    arc_y: f64,
    oval: [f64; 4],
    bars: [(Mark, &'static [(f64, f64)]); 4],
    plus: [[f64; 4]; 2],
}

fn geometry(size: BoxSize) -> Geometry {
    match size {
        BoxSize::Large => Geometry {
            frame: [76, 42],
            x0: 15.0,
            x1: 60.0,
            y0: 11.0,
            y1: 34.0,
            line: 0.65,
            dot: 2.3,
            wheels: [[27.5, 32.5], [47.5, 32.5]],
            wheel_r: 2.6,
            aa_bar: 0.46,
            arc_r: 4.4,
            arc_y: 25.0,
            oval: [0.211, 0.152, 0.789, 0.848],
            bars: [
                (Mark::BarMedium, &[(19.0, 21.0)]),
                (Mark::BarHeavy, &[(17.0, 21.0)]),
                (Mark::BarSuperHeavy, &[(16.0, 22.0)]),
                (Mark::BarModern, &[(17.0, 20.0), (55.0, 58.0)]),
            ],
            plus: [[21.0, 14.0, 23.0, 22.0], [18.0, 17.0, 26.0, 19.0]],
        },
        BoxSize::Small => Geometry {
            frame: [30, 12],
            x0: 3.0,
            x1: 26.0,
            y0: 1.0,
            y1: 11.0,
            line: 0.5,
            dot: 1.5,
            wheels: [[9.5, 10.2], [18.5, 10.2]],
            wheel_r: 1.5,
            aa_bar: 0.55,
            arc_r: 2.5,
            arc_y: 6.5,
            oval: [0.239, 0.15, 0.717, 0.85],
            bars: [
                (Mark::BarMedium, &[(5.0, 6.0)]),
                (Mark::BarHeavy, &[(4.0, 7.0)]),
                (Mark::BarSuperHeavy, &[(4.0, 7.0)]),
                (Mark::BarModern, &[(4.0, 6.0), (23.0, 25.0)]),
            ],
            plus: [[5.0, 1.0, 6.0, 4.0], [4.0, 2.0, 7.0, 3.0]],
        },
    }
}

#[derive(Debug, Clone, Copy)]
enum Shape {
    Seg([f64; 2], [f64; 2], f64),
    Disc([f64; 2], f64),
    Stadium([f64; 4], f64),
    Rect([f64; 4]),
    Tri([f64; 2], [f64; 2], [f64; 2]),
    UpperArc([f64; 2], f64, f64),
}

fn seg_dist(p: [f64; 2], a: [f64; 2], b: [f64; 2]) -> f64 {
    let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 { (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0) } else { 0.0 };
    ((p[0] - a[0] - t * dx).powi(2) + (p[1] - a[1] - t * dy).powi(2)).sqrt()
}

impl Shape {
    fn inside(&self, p: [f64; 2]) -> bool {
        match *self {
            Shape::Seg(a, b, hw) => seg_dist(p, a, b) <= hw,
            Shape::Disc(c, r) => (p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2) <= r * r,
            Shape::Stadium([x0, y0, x1, y1], hw) => {
                let r = (y1 - y0) / 2.0;
                let cy = (y0 + y1) / 2.0;
                let d = seg_dist(p, [x0 + r, cy], [x1 - r, cy]);
                (d - r).abs() <= hw
            }
            Shape::Rect([x0, y0, x1, y1]) => p[0] >= x0 && p[0] < x1 && p[1] >= y0 && p[1] < y1,
            Shape::Tri(a, b, c) => {
                let s = |p1: [f64; 2], p2: [f64; 2]| (p[0] - p2[0]) * (p1[1] - p2[1]) - (p1[0] - p2[0]) * (p[1] - p2[1]);
                let (d1, d2, d3) = (s(a, b), s(b, c), s(c, a));
                let neg = d1 < 0.0 || d2 < 0.0 || d3 < 0.0;
                let pos = d1 > 0.0 || d2 > 0.0 || d3 > 0.0;
                !(neg && pos)
            }
            Shape::UpperArc(c, r, hw) => {
                let d = ((p[0] - c[0]).powi(2) + (p[1] - c[1]).powi(2)).sqrt();
                p[1] <= c[1] + hw && (d - r).abs() <= hw
            }
        }
    }
}

fn shapes(marks: &[Mark], g: &Geometry) -> Vec<Shape> {
    let at = |u: f64, v: f64| [g.x0 + u * (g.x1 - g.x0), g.y0 + v * (g.y1 - g.y0)];
    let hw = g.line;
    let mut out = Vec::new();
    for m in marks {
        match m {
            Mark::Cross => {
                out.push(Shape::Seg(at(0.0, 0.0), at(1.0, 1.0), hw));
                out.push(Shape::Seg(at(1.0, 0.0), at(0.0, 1.0), hw));
            }
            Mark::CrossOpen => {
                for (a, b) in [((0.0, 0.0), (1.0, 1.0)), ((1.0, 0.0), (0.0, 1.0))] {
                    let p = |t: f64| at(a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
                    out.push(Shape::Seg(p(0.0), p(0.3), hw));
                    out.push(Shape::Seg(p(0.7), p(1.0), hw));
                }
            }
            Mark::Oval => {
                let (a, b) = (at(g.oval[0], g.oval[1]), at(g.oval[2], g.oval[3]));
                out.push(Shape::Stadium([a[0], a[1], b[0], b[1]], hw));
            }
            Mark::Dot => out.push(Shape::Disc(at(0.5, 0.5), g.dot)),
            Mark::Wheels => {
                for c in g.wheels {
                    out.push(Shape::Disc(c, g.wheel_r));
                }
            }
            Mark::AntiTank => {
                out.push(Shape::Seg(at(0.0, 0.0), at(0.5, 1.0), hw));
                out.push(Shape::Seg(at(0.5, 1.0), at(1.0, 0.0), hw));
            }
            Mark::AntiAir => {
                out.push(Shape::Seg(at(0.0, 1.0), at(0.5, 0.0), hw));
                out.push(Shape::Seg(at(0.5, 0.0), at(1.0, 1.0), hw));
                let v = g.aa_bar;
                let u = 0.5 * (1.0 - v);
                out.push(Shape::Seg(at(u, v), at(1.0 - u, v), hw));
            }
            Mark::Slash => out.push(Shape::Seg(at(0.0, 1.0), at(1.0, 0.0), hw)),
            Mark::Bridge => {
                let (top, bottom) = (0.33, 0.75);
                out.push(Shape::Seg(at(0.28, top), at(0.72, top), hw));
                for u in [0.28, 0.5, 0.72] {
                    out.push(Shape::Seg(at(u, top), at(u, bottom), hw));
                }
            }
            Mark::Mountain => out.push(Shape::Tri(at(0.5, 0.52), at(0.39, 1.0), at(0.61, 1.0))),
            Mark::Airborne => {
                let cx = (g.x0 + g.x1) / 2.0;
                out.push(Shape::UpperArc([cx - g.arc_r, g.arc_y], g.arc_r, hw));
                out.push(Shape::UpperArc([cx + g.arc_r, g.arc_y], g.arc_r, hw));
            }
            Mark::Rocket => {
                let c = at(0.5, 0.0);
                let h = g.y1 - g.y0;
                let arm = h * 0.12;
                for v in [0.2, 0.38] {
                    let apex = [c[0], g.y0 + v * h];
                    out.push(Shape::Seg(apex, [apex[0] - arm, apex[1] + arm], hw));
                    out.push(Shape::Seg(apex, [apex[0] + arm, apex[1] + arm], hw));
                }
                out.push(Shape::Disc(at(0.5, 0.68), g.dot));
            }
            Mark::BarMedium | Mark::BarHeavy | Mark::BarSuperHeavy | Mark::BarModern => {
                if let Some((_, spans)) = g.bars.iter().find(|(b, _)| b == m) {
                    for (a, b) in spans.iter() {
                        out.push(Shape::Rect([*a, g.y0, *b, g.y1]));
                    }
                }
            }
            Mark::Plus => {
                for r in g.plus {
                    out.push(Shape::Rect(r));
                }
            }
        }
    }
    out
}

/// Supersampling per axis when marks are drawn.
const MARK_SAMPLES: u32 = 8;

/// Frame 2: the vanilla NATO box with `marks` drawn in black.
pub fn nato_box(size: BoxSize, marks: &[Mark]) -> Rgba {
    let g = geometry(size);
    let mut img = Rgba::new(g.frame[0], g.frame[1]);
    let (bx0, by0, bx1, by1) = match size {
        BoxSize::Large => (14u32, 10u32, 60u32, 34u32),
        BoxSize::Small => (2, 0, 26, 11),
    };
    if size == BoxSize::Large {
        // soft shadow: falloff per axis around the full-strength rectangle, multiplied
        let f = |i: i64, lo: i64, hi: i64| {
            let d = if i < lo {
                lo - i
            } else if i > hi {
                i - hi
            } else {
                0
            };
            if d == 0 {
                1.0
            } else {
                LARGE_SHADOW_FALLOFF.get((d - 1) as usize).copied().unwrap_or(0.0)
            }
        };
        for y in 0..g.frame[1] {
            for x in 0..g.frame[0] {
                let a = LARGE_SHADOW_PEAK * f(i64::from(x), 17, 59) * f(i64::from(y), 13, 33);
                img.set_px(x, y, [0, 0, 0, a.round() as u8]);
            }
        }
    }
    for y in by0..=by1 {
        for x in bx0..=bx1 {
            let border = x == bx0 || x == bx1 || y == by0 || y == by1;
            let v = if border {
                0
            } else {
                match size {
                    BoxSize::Large => {
                        let row = LARGE_BOX_ROWS[(y - 11) as usize];
                        let f = f64::from(row);
                        if y == 33 {
                            if x == 15 {
                                136
                            } else {
                                row
                            }
                        } else if x == 15 {
                            (f + (255.0 - f) * 0.28).round() as u8
                        } else if x == 59 {
                            (f * if y == 11 { 0.924 } else { 0.866 }).round() as u8
                        } else {
                            row
                        }
                    }
                    BoxSize::Small => SMALL_BOX_ROWS[(y - 1) as usize],
                }
            };
            img.set_px(x, y, [v, v, v, 255]);
        }
    }
    let list = shapes(&canonical(marks), &g);
    if list.is_empty() {
        return img;
    }
    let s = MARK_SAMPLES;
    let total = f64::from(s * s);
    for y in by0 + 1..by1 {
        for x in bx0 + 1..bx1 {
            let mut hit = 0u32;
            for sy in 0..s {
                for sx in 0..s {
                    let p = [f64::from(x) + (f64::from(sx) + 0.5) / f64::from(s), f64::from(y) + (f64::from(sy) + 0.5) / f64::from(s)];
                    if list.iter().any(|sh| sh.inside(p)) {
                        hit += 1;
                    }
                }
            }
            if hit > 0 {
                let a = (f64::from(hit) / total * 255.0).round() as u8;
                let below = img.px(x, y);
                img.set_px(x, y, over([0, 0, 0, a], below));
            }
        }
    }
    img
}

/// The blue "+" of a support company on frame 1 of a battalion counter, at vanilla's place (an 11x11 cross with
/// 3 px arms at (9, 8)), lit like the glyph and with its halo.
pub fn support_plus_layer() -> Result<Rgba> {
    let mut cov = Plane::new(76, 42);
    for y in 8..19 {
        for x in 9..20 {
            if (13..16).contains(&x) || (12..15).contains(&y) {
                cov.v[y * 76 + x] = 1.0;
            }
        }
    }
    let mut style = GlyphStyle::of(UnitArtKind::CounterLarge);
    style.tint = Tint::Blue { mid: style.lighting.base };
    render_glyph(&cov, &style)
}

/// The two-frame counter strip: the finished glyph frame, then the NATO box with `marks`. A support "+" also goes on
/// frame 1 of the battalion counter, as vanilla's support companies have it.
pub fn counter_strip(frame1: &Rgba, kind: UnitArtKind, marks: &[Mark]) -> Result<Rgba> {
    let size = match kind {
        UnitArtKind::CounterLarge => BoxSize::Large,
        UnitArtKind::CounterSmall => BoxSize::Small,
        UnitArtKind::DivisionEmblem => return Err(Error::Invalid("a division symbol has one frame and no NATO box".into())),
    };
    let g = geometry(size);
    if [frame1.width, frame1.height] != g.frame {
        return Err(Error::Invalid(format!(
            "frame 1 is {}x{}, the {} frame is {}x{}",
            frame1.width,
            frame1.height,
            kind.as_str(),
            g.frame[0],
            g.frame[1]
        )));
    }
    let mut first = frame1.clone();
    if size == BoxSize::Large && marks.contains(&Mark::Plus) {
        let plus = support_plus_layer()?;
        for y in 0..42 {
            for x in 0..76 {
                first.set_px(x, y, over(plus.px(x, y), first.px(x, y)));
            }
        }
    }
    Rgba::join_frames(&[first, nato_box(size, marks)])
}

/// The colour of a glyph kept in its own colours: the per-channel median of its opaque pixels (alpha 250 and up).
/// White when the frame has none.
pub fn glyph_colour(frame: &Rgba) -> [u8; 3] {
    let mut ch: [Vec<u8>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    for p in frame.data.chunks_exact(4).filter(|p| p[3] >= 250) {
        for c in 0..3 {
            ch[c].push(p[c]);
        }
    }
    if ch[0].is_empty() {
        return [255, 255, 255];
    }
    let mut out = [0u8; 3];
    for c in 0..3 {
        ch[c].sort_unstable();
        out[c] = ch[c][ch[c].len() / 2];
    }
    out
}

/// The counter strip of a style pack that keeps the art's own colours (`keep_colours`): frame 1 as it is, then
/// vanilla's NATO box redrawn as an outline in the glyph's colour ([`glyph_colour`]): the box border and the symbol's
/// marks in that colour, nothing filled and no shadow. The box sits where vanilla's does.
pub fn outline_strip(frame1: &Rgba, kind: UnitArtKind, marks: &[Mark]) -> Result<Rgba> {
    let size = match kind {
        UnitArtKind::CounterLarge => BoxSize::Large,
        UnitArtKind::CounterSmall => BoxSize::Small,
        UnitArtKind::DivisionEmblem => return Err(Error::Invalid("a division symbol has one frame and no NATO box".into())),
    };
    let g = geometry(size);
    if [frame1.width, frame1.height] != g.frame {
        return Err(Error::Invalid(format!(
            "frame 1 is {}x{}, the {} frame is {}x{}",
            frame1.width,
            frame1.height,
            kind.as_str(),
            g.frame[0],
            g.frame[1]
        )));
    }
    let colour = glyph_colour(frame1);
    let plain = nato_box(size, &[]);
    let marked = nato_box(size, marks);
    let mut second = Rgba::new(g.frame[0], g.frame[1]);
    for y in 0..g.frame[1] {
        for x in 0..g.frame[0] {
            let (p, m) = (plain.px(x, y), marked.px(x, y));
            if p[3] != 255 {
                continue;
            }
            let a = if p[0] == 0 { 1.0 } else { (1.0 - f64::from(m[0]) / f64::from(p[0])).clamp(0.0, 1.0) };
            if a > 0.0 {
                second.set_px(x, y, [colour[0], colour[1], colour[2], (a * 255.0).round() as u8]);
            }
        }
    }
    Rgba::join_frames(&[frame1.clone(), second])
}

// ------------------------------------------------------------------ glyph shape

/// Alpha at and above which a pixel of finished unit art counts as visible when its shape is measured. It is low enough
/// that the half-covered edge row of a fitted glyph still counts, so a glyph the recipe accepted never fails by a pixel.
pub const SHAPE_ALPHA: u8 = 64;

/// How many times wider than the glyph box's aspect a kept-colours glyph may be before it is more than one glyph.
pub const KEPT_WIDE_FACTOR: f64 = 2.0;

/// The widest and the narrowest glyph (width over height) vanilla draws, with a margin, measured on frame 1 of every
/// file of HOI4 1.19.4.1 (alpha 128 and up, and the limits hold at [`SHAPE_ALPHA`] too). Large counters (111 strips) run from 0.81 to 4.64, on-map counters (83)
/// from 0.50 to 4.00 and division symbols (101) from 0.48 to 6.10. The widest are special art such as the guided
/// missile counter and a few wide symbols.
pub fn vanilla_aspect_range(kind: UnitArtKind) -> [f64; 2] {
    match kind {
        UnitArtKind::CounterLarge => [0.6, 5.0],
        UnitArtKind::CounterSmall => [0.4, 4.5],
        UnitArtKind::DivisionEmblem => [0.35, 6.5],
    }
}

/// The narrowest and widest glyph aspect vanilla has (see [`vanilla_aspect_range`]).
pub fn vanilla_aspect_extremes(kind: UnitArtKind) -> [f64; 2] {
    match kind {
        UnitArtKind::CounterLarge => [0.81, 4.64],
        UnitArtKind::CounterSmall => [0.50, 4.00],
        UnitArtKind::DivisionEmblem => [0.48, 6.10],
    }
}

/// The aspect range a glyph of `kind` may have. With `kept` (a kept-colours style pack, whose glyph is fitted into the
/// glyph box) the wide limit is also at most [`KEPT_WIDE_FACTOR`] times the box's aspect and at most vanilla's widest
/// glyph: a wider drawing is a unit beside a second element (its own NATO box, a caption) that the fit would shrink into a sliver.
pub fn glyph_aspect_range(kind: UnitArtKind, kept: bool) -> [f64; 2] {
    let [min, mut max] = vanilla_aspect_range(kind);
    if kept {
        let b = GlyphStyle::of(kind).max_box;
        max = max.min(KEPT_WIDE_FACTOR * b[0] / b[1]).min(vanilla_aspect_extremes(kind)[1]);
    }
    [min, max]
}

/// What is wrong with the shape of a glyph of `w` x `h` visible pixels for `kind`, or `None` when it is one a unit
/// glyph can have. See [`glyph_aspect_range`].
pub fn glyph_shape_problem(kind: UnitArtKind, kept: bool, w: u32, h: u32) -> Option<String> {
    let (wf, hf) = (f64::from(w), f64::from(h.max(1)));
    let aspect = wf / hf;
    let [min, max] = glyph_aspect_range(kind, kept);
    let [vmin, vmax] = vanilla_aspect_extremes(kind);
    // half a pixel of slack on the short side: a fit rounds its size to whole pixels, and a 25x6 glyph on the small
    // counter may be a 4.0 drawing
    if wf > max * (hf + 0.5) {
        Some(format!(
            "{w}x{h} pixels, {aspect:.1} times as wide as tall, where a {} glyph is at most {max:.1} (vanilla's widest is {vmax:.1})",
            kind.as_str()
        ))
    } else if wf + 0.5 < min * hf {
        Some(format!(
            "{w}x{h} pixels, {:.1} times as tall as wide, where a {} glyph is at most {:.1} (vanilla's tallest is {:.1})",
            1.0 / aspect,
            kind.as_str(),
            1.0 / min,
            1.0 / vmin
        ))
    } else {
        None
    }
}

// ------------------------------------------------------------------ material check

/// Share of the glyph pixels of frame 1 that must lie on the kind's material for finished art to count as vanilla's.
pub const MATERIAL_SHARE: f64 = 0.85;
/// How far a glyph pixel may be from the material: green from `k * red + c`, red from blue, grey channels apart.
pub const MATERIAL_TOLERANCE: f64 = 14.0;
/// Median amount by which frame 2's box may be lighter than vanilla's grey (border left out). Only lighter counts: the
/// symbol's black marks darken the box, and on the 30x12 box they can cover most of it.
pub const BOX_TOLERANCE: f64 = 14.0;

/// Everything that keeps `img`, finished unit art of `kind` at its full size (the two-frame strip of a counter, the
/// single frame of a division symbol), from being vanilla's material. Empty when it matches.
///
/// Frame 1 is checked pixel by pixel: the opaque glyph pixels (alpha 250 and up, luminance 40 and up, blue support
/// "+" pixels left out) must lie on the kind's material: green with red equal to blue and green following red as
/// vanilla's does, or grey for the on-map counter. At least 20 such pixels must exist. Frame 2 of a counter must be
/// vanilla's NATO box: its black border and its grey rows (marks left out) within [`BOX_TOLERANCE`] of the template.
/// The vanilla files of `divisions_large`, `division_templates_large` and the grey `divisions_small` pass (see the
/// tests, all but 3 of 111 battalion strips, 2 of 81 on-map strips and 3 of 101 symbols, which are special art);
/// the gallery's imported white and yellow line art does not.
pub fn material_problems(img: &Rgba, kind: UnitArtKind) -> Vec<String> {
    let style = GlyphStyle::of(kind);
    let frames = if kind == UnitArtKind::DivisionEmblem { 1 } else { 2 };
    let want = [style.frame[0] * frames, style.frame[1]];
    if [img.width, img.height] != want {
        return vec![format!("finished {} art is {}x{}, this is {}x{}", kind.as_str(), want[0], want[1], img.width, img.height)];
    }
    let parts = match img.split_frames(frames) {
        Ok(p) => p,
        Err(e) => return vec![e.to_string()],
    };
    let mut problems = Vec::new();
    let f1 = &parts[0];
    let (mut n, mut on) = (0usize, 0usize);
    for p in f1.data.chunks_exact(4) {
        let [r, g, b] = [p[0], p[1], p[2]].map(f64::from);
        let lum = luminance(r, g, b);
        if p[3] < 250 || lum < 40.0 || b > g + 10.0 {
            continue;
        }
        n += 1;
        let ok = match style.tint {
            Tint::Green { k, c } => (r - b).abs() <= MATERIAL_TOLERANCE && (g - (k * r + c)).abs() <= MATERIAL_TOLERANCE,
            _ => (r - g).abs() <= MATERIAL_TOLERANCE && (g - b).abs() <= MATERIAL_TOLERANCE,
        };
        if ok {
            on += 1;
        }
    }
    let material = match style.tint {
        Tint::Green { .. } => "vanilla's green counter material (red equal to blue, green following red)",
        _ => "vanilla's light grey on-map material (red, green and blue equal)",
    };
    // the on-map glyph is tiny: vanilla's marine and no-intel glyphs have 9 such pixels
    let min_pixels = if kind == UnitArtKind::CounterSmall { 8 } else { 20 };
    if n < min_pixels {
        problems.push(format!("frame 1 has {n} opaque glyph pixels; vanilla's unit art is a solid silhouette in {material}"));
    } else if (on as f64) < MATERIAL_SHARE * n as f64 {
        problems.push(format!(
            "{:.0} percent of frame 1's glyph pixels are in {material}; at least {:.0} percent must be",
            100.0 * on as f64 / n as f64,
            100.0 * MATERIAL_SHARE
        ));
    }
    if frames == 2 {
        let size = if kind == UnitArtKind::CounterLarge { BoxSize::Large } else { BoxSize::Small };
        let template = nato_box(size, &[]);
        let f2 = &parts[1];
        let (mut border, mut border_ok, mut grey, mut diffs) = (0usize, 0usize, 0usize, Vec::new());
        for (t, p) in template.data.chunks_exact(4).zip(f2.data.chunks_exact(4)) {
            if t[3] != 255 {
                continue;
            }
            let lp = if p[3] < 128 { 255.0 } else { luminance(f64::from(p[0]), f64::from(p[1]), f64::from(p[2])) };
            if t[0] == 0 {
                border += 1;
                if lp < 60.0 {
                    border_ok += 1;
                }
            } else {
                diffs.push((lp - f64::from(t[0])).max(0.0));
                if lp >= 60.0 {
                    grey += 1;
                }
            }
        }
        let inside = diffs.len().max(1);
        diffs.sort_by(f64::total_cmp);
        let med = diffs.get(diffs.len() / 2).copied().unwrap_or(f64::MAX);
        if (border_ok as f64) < 0.6 * border as f64 || med > BOX_TOLERANCE || (grey as f64) < 0.3 * inside as f64 {
            problems.push(format!(
                "frame 2 is not vanilla's NATO box: {border_ok} of {border} border pixels are black, {grey} of {inside} box pixels are grey, and the box is {med:.1} lighter than vanilla's in the median (at most {BOX_TOLERANCE})"
            ));
        }
    }
    problems
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A black tank-like silhouette on white, with a white gap inside, at a size a model would return.
    fn drawn_tank(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::filled(w, h, [255, 255, 255, 255]);
        let (fw, fh) = (f64::from(w), f64::from(h));
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (f64::from(x) / fw, f64::from(y) / fh);
                let hull = (0.15..0.85).contains(&u) && (0.5..0.75).contains(&v);
                let turret = (0.35..0.6).contains(&u) && (0.35..0.5).contains(&v);
                let barrel = (0.6..0.9).contains(&u) && (0.4..0.44).contains(&v);
                let gap = (0.2..0.8).contains(&u) && (0.68..0.7).contains(&v);
                if (hull || turret || barrel) && !gap {
                    img.set_px(x, y, [12, 12, 12, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn coverage_reads_a_dark_shape_on_a_flat_background_and_keeps_light_gaps_as_holes() {
        let img = drawn_tank(400, 300);
        let cov = silhouette_coverage(&img).unwrap();
        assert_eq!(cov.at(200, 180), 1.0, "hull");
        assert_eq!(cov.at(5, 5), 0.0, "background");
        assert_eq!(cov.at(200, 206), 0.0, "the white gap inside is a hole");
        // the same shape on real transparency
        let mut clear = img.clone();
        for p in clear.data.chunks_exact_mut(4) {
            if p[0] == 255 {
                p[3] = 0;
            }
        }
        let cov2 = silhouette_coverage(&clear).unwrap();
        assert_eq!(cov2.bbox(0.5), cov.bbox(0.5));
        // a busy background is refused with a clear message
        let mut busy = img.clone();
        for (i, p) in busy.data.chunks_exact_mut(4).enumerate() {
            if i % 3 == 0 {
                p[..3].copy_from_slice(&[200, 30, 30]);
            }
        }
        assert!(silhouette_coverage(&busy).unwrap_err().to_string().contains("flat colour"));
        assert!(silhouette_coverage(&Rgba::filled(64, 64, [255, 255, 255, 255])).is_err(), "a blank image");
    }

    fn source(name: &str) -> Rgba {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gallery").join(name);
        crate::pixels::decode(&std::fs::read(p).unwrap(), Some(name), Default::default()).unwrap().0
    }

    /// The 0.3.1 lightning bolt (a yellow bolt on a blue shield, opaque on white, here at half size) came out as a
    /// plain green shield: contrast with the white paper saw the bolt as part of the shape. Coloured light marks inside
    /// a dark shape are now holes, so the bolt is cut into the shield.
    #[test]
    fn a_coloured_bolt_inside_a_dark_shield_stays_visible() {
        let bolt = source("v031_lightning_bolt-v1_half.png");
        let count_holes = |cov: &Plane| {
            let b = cov.bbox(0.5).unwrap();
            // clear pixels inside the shield's box, away from its curved bottom
            let mut n = 0;
            for y in b[1] + (b[3] - b[1]) / 6..b[1] + (b[3] - b[1]) * 2 / 3 {
                for x in b[0] + (b[2] - b[0]) / 6..b[2] - (b[2] - b[0]) / 6 {
                    if cov.at(x, y) < 0.5 {
                        n += 1;
                    }
                }
            }
            n
        };
        let (with, without) = (coverage_with(&bolt, true).unwrap(), coverage_with(&bolt, false).unwrap());
        let (h_with, h_without) = (count_holes(&with), count_holes(&without));
        eprintln!("bolt pixels cut into the shield: {h_with} now, {h_without} in 0.3.1");
        assert!(h_without < 200 && h_with > 3000, "{h_without} {h_with}");
        // and the bolt survives the fit to 76x42: a clear stroke crosses the green shield
        let glyph = unit_glyph(&bolt, UnitArtKind::DivisionEmblem).unwrap();
        let b = glyph.alpha_bbox(250).unwrap();
        let inner_clear =
            (b[1] + 3..b[3] - 6).flat_map(|y| (b[0] + 4..b[2] - 4).map(move |x| (x, y))).filter(|(x, y)| glyph.px(*x, *y)[3] < 128).count();
        assert!(inner_clear >= 15, "the bolt is {inner_clear} px inside the shield");
        // a synthetic thin bolt, 3 percent of the shield's width, is kept too
        let mut img = Rgba::filled(800, 600, [255, 255, 255, 255]);
        for y in 100..500u32 {
            for x in 200..600u32 {
                let on_bolt = (i64::from(x) - 400 - (i64::from(y) - 300) / 3).abs() < 12;
                img.set_px(x, y, if on_bolt { [235, 190, 40, 255] } else { [30, 60, 120, 255] });
            }
        }
        let cov = silhouette_coverage(&img).unwrap();
        assert!(cov.at(400, 300) < 0.1 && cov.at(300, 300) > 0.9, "a thin gold bolt is a hole");
    }

    /// The coloured-mark rule leaves art without coloured marks unchanged: the 0.3.1 counters and bear emblem. The bear
    /// (real alpha) and the rifle (real alpha) are pinned by hash; on the opaque black-on-white cavalry the rule
    /// changes no coverage value at all.
    #[test]
    fn the_031_unit_art_is_unchanged() {
        let bear = unit_glyph(&source("v031_bear_standing-v1.png"), UnitArtKind::DivisionEmblem).unwrap();
        assert_eq!(bear.pixels_sha256(), "97ebbf483fb30675b878691311d3362fc70d024a496608ff1577793b5c74ae33");
        let rifle = unit_glyph(&source("v031_rifle_infantry-v1.png"), UnitArtKind::CounterSmall).unwrap();
        assert_eq!(rifle.pixels_sha256(), "557f3c8fedb79f1dcd5740f59bf745665dfa7b1eba19a9118bc83ceabc75863f");
        let horse = source("v031_cavalry_horse-v1_half.png");
        assert_eq!(coverage_with(&horse, true).unwrap(), coverage_with(&horse, false).unwrap());
        assert_eq!(coverage_with(&drawn_tank(400, 300), true).unwrap(), coverage_with(&drawn_tank(400, 300), false).unwrap());
    }

    #[test]
    fn area_resampling_keeps_total_coverage() {
        let mut p = Plane::new(100, 40);
        for y in 10..30 {
            for x in 20..80 {
                p.v[y * 100 + x] = 1.0;
            }
        }
        let small = area_resample(&p, 25, 10);
        let (a, b): (f32, f32) = (p.v.iter().sum(), small.v.iter().sum());
        assert!((a / 16.0 - b).abs() < 0.01, "{a} {b}");
        // a one-pixel line at quarter scale survives as partial coverage
        let mut line = Plane::new(100, 40);
        for x in 0..100 {
            line.v[20 * 100 + x] = 1.0;
        }
        let s = area_resample(&line, 25, 10);
        assert!(s.v.iter().any(|v| *v > 0.2));
    }

    #[test]
    fn every_kind_fits_at_vanilla_size_and_place_and_renders_its_material() {
        for kind in [UnitArtKind::CounterLarge, UnitArtKind::CounterSmall, UnitArtKind::DivisionEmblem] {
            let style = GlyphStyle::of(kind);
            let out = unit_glyph(&drawn_tank(1536, 1024), kind).unwrap();
            assert_eq!([out.width, out.height], style.frame, "{kind:?}");
            let b = out.alpha_bbox(200).unwrap();
            let (cx, cy) = (f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0);
            assert!((cx - style.centre[0]).abs() <= 1.5 && (cy - style.centre[1]).abs() <= 1.5, "{kind:?}: centre {cx},{cy}");
            assert!(f64::from(b[2] - b[0]) <= style.max_box[0] + 2.0 && f64::from(b[3] - b[1]) <= style.max_box[1] + 2.0, "{kind:?}: {b:?}");
            // colour: green with r == b on the opaque glyph, or grey
            let opaque: Vec<[u8; 4]> = out.data.chunks_exact(4).filter(|p| p[3] == 255 && p[0] > 20).map(|p| [p[0], p[1], p[2], p[3]]).collect();
            assert!(opaque.len() > 20, "{kind:?}");
            for p in &opaque {
                assert_eq!(p[0], p[2], "{kind:?}: {p:?}");
                if kind == UnitArtKind::CounterSmall {
                    assert_eq!(p[0], p[1]);
                } else {
                    assert!(p[1] > p[0], "{kind:?}: green {p:?}");
                }
            }
            // a halo of black around it, nothing at the corners
            assert_eq!(out.px(0, 0)[3], 0);
            // deterministic
            assert_eq!(unit_glyph(&drawn_tank(1536, 1024), kind).unwrap(), out);
        }
    }

    #[test]
    fn the_top_is_lighter_than_the_bottom_and_the_top_rim_is_lightest() {
        let out = unit_glyph(&drawn_tank(800, 600), UnitArtKind::DivisionEmblem).unwrap();
        let b = out.alpha_bbox(250).unwrap();
        let mean_row = |y: u32| {
            let v: Vec<f64> = (b[0]..b[2]).map(|x| out.px(x, y)).filter(|p| p[3] == 255).map(|p| f64::from(p[0])).collect();
            v.iter().sum::<f64>() / v.len().max(1) as f64
        };
        let mid = (b[1] + b[3]) / 2;
        assert!(mean_row(b[1] + 1) > mean_row(mid), "top lighter than middle");
        assert!(mean_row(mid) > mean_row(b[3] - 1), "middle lighter than bottom");
    }

    #[test]
    fn symbols_parse_from_presets_and_marks_and_refuse_unknown_words() {
        assert_eq!(parse_symbol("medium_armor").unwrap(), canonical(&[Mark::BarMedium, Mark::Oval]));
        assert_eq!(parse_symbol("infantry+support").unwrap(), canonical(&[Mark::Cross, Mark::Plus]));
        assert_eq!(parse_symbol(" cross + wheels ").unwrap(), parse_symbol("motorized").unwrap());
        assert_eq!(symbol_text(&parse_symbol("motorized").unwrap()), "cross+wheels");
        let e = parse_symbol("cross+laser").unwrap_err().to_string();
        assert!(e.contains("laser") && e.contains("infantry") && e.contains("oval"), "{e}");
        assert!(parse_symbol("+").is_err());
    }

    #[test]
    fn symbols_are_inferred_from_vanilla_sub_unit_keys() {
        let t = |k: &str| symbol_text(&infer_symbol(k));
        assert_eq!(t("infantry"), "cross");
        assert_eq!(t("motorized"), "cross+wheels");
        assert_eq!(t("mechanized"), "cross+oval");
        assert_eq!(t("light_armor"), "oval");
        assert_eq!(t("medium_armor"), "oval+bar_medium");
        assert_eq!(t("heavy_armor"), "oval+bar_heavy");
        assert_eq!(t("super_heavy_armor"), "oval+bar_super_heavy");
        assert_eq!(t("modern_armor"), "oval+bar_modern");
        assert_eq!(t("artillery_brigade"), "dot");
        assert_eq!(t("mot_artillery_brigade"), "dot+wheels");
        assert_eq!(t("anti_tank_brigade"), "dot+anti_tank_v");
        assert_eq!(t("anti_air_brigade"), "anti_air_arch");
        assert_eq!(t("cavalry"), "slash");
        assert_eq!(t("engineer"), "bridge");
        assert_eq!(t("mountaineers"), "cross+mountain_peak");
        assert_eq!(t("paratrooper"), "cross_open+airborne_arcs");
        assert_eq!(t("rocket_artillery_brigade"), "rocket_chevrons");
        assert_eq!(t("support_engineer"), "bridge+plus");
        assert_eq!(t("blackshirt_assault_battalion"), "cross");
    }

    #[test]
    fn the_nato_box_is_vanillas_template() {
        let b = nato_box(BoxSize::Large, &[]);
        assert_eq!((b.width, b.height), (76, 42));
        assert_eq!(b.px(14, 20), [0, 0, 0, 255], "border");
        assert_eq!(b.px(30, 11)[0], 238, "highlight row");
        assert_eq!(b.px(30, 20)[0], 193);
        assert_eq!(b.px(30, 33)[0], 117, "shade row");
        assert_eq!(b.px(61, 20)[3], 97, "shadow at 1 px right");
        assert_eq!(b.px(0, 0)[3], 0);
        let s = nato_box(BoxSize::Small, &[]);
        assert_eq!((s.width, s.height), (30, 12));
        assert_eq!(s.px(2, 5), [0, 0, 0, 255]);
        assert_eq!(s.px(10, 1)[0], 227);
        assert_eq!(s.px(0, 5)[3], 0, "no shadow on the small box");
        // marks are black and stay inside the box
        let inf = nato_box(BoxSize::Large, &[Mark::Cross]);
        assert!(inf.px(37, 22)[0] < 60, "the cross meets in the middle: {:?}", inf.px(37, 22));
        assert_eq!(inf.px(5, 5), b.px(5, 5));
    }

    #[test]
    fn the_counter_strip_has_the_glyph_then_the_box_and_support_adds_the_blue_plus() {
        let glyph = unit_glyph(&drawn_tank(800, 600), UnitArtKind::CounterLarge).unwrap();
        let strip = counter_strip(&glyph, UnitArtKind::CounterLarge, &[Mark::Oval]).unwrap();
        assert_eq!((strip.width, strip.height), (152, 42));
        let f = strip.split_frames(2).unwrap();
        assert_eq!(f[0], glyph);
        assert_eq!(f[1], nato_box(BoxSize::Large, &[Mark::Oval]));
        let sup = counter_strip(&glyph, UnitArtKind::CounterLarge, &[Mark::Oval, Mark::Plus]).unwrap();
        let p = sup.px(14, 13);
        assert!(p[2] > p[1] && p[1] > p[0] && p[3] == 255, "blue plus: {p:?}");
        assert!(counter_strip(&glyph, UnitArtKind::DivisionEmblem, &[]).is_err());
        assert!(counter_strip(&glyph, UnitArtKind::CounterSmall, &[]).is_err(), "wrong frame size");
    }

    // ------------------------------------------------------------ against the installed game

    fn game_dir() -> Option<std::path::PathBuf> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        g.join("gfx/interface/counters").is_dir().then_some(g)
    }

    /// Every vanilla file of a folder at `size`, by name.
    fn vanilla(folder: &str, size: (u32, u32)) -> Vec<(String, Rgba)> {
        let Some(g) = game_dir() else { return Vec::new() };
        let dir = g.join("gfx/interface/counters").join(folder);
        let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        files
            .iter()
            .filter_map(|p| {
                let img = crate::dds::decode(&std::fs::read(p).ok()?).ok()?;
                ((img.width, img.height) == size).then(|| (p.file_name().unwrap().to_string_lossy().to_string(), img))
            })
            .collect()
    }

    /// Frame 1 of each vanilla file of a kind.
    fn vanilla_frames(kind: UnitArtKind) -> Vec<(String, Rgba)> {
        let (folder, size, frames) = match kind {
            UnitArtKind::CounterLarge => ("divisions_large", (152, 42), 2),
            UnitArtKind::CounterSmall => ("divisions_small", (60, 12), 2),
            UnitArtKind::DivisionEmblem => ("division_templates_large", (76, 42), 1),
        };
        vanilla(folder, size).into_iter().map(|(n, img)| (n, img.split_frames(frames).unwrap().swap_remove(0))).collect()
    }

    /// The glyph mask of a vanilla frame (the same rule as the measurement: green or light pixels of alpha 128 and
    /// up), and the pixels to leave out (around the blue support "+"). `None` for coloured or empty files.
    fn vanilla_mask(kind: UnitArtKind, f: &Rgba) -> Option<(Vec<bool>, Vec<bool>)> {
        let (w, h) = (f.width as usize, f.height as usize);
        let px: Vec<[i32; 4]> = f.data.chunks_exact(4).map(|p| [p[0], p[1], p[2], p[3]].map(i32::from)).collect();
        let (glyph, excl): (Vec<bool>, Vec<bool>) = if kind == UnitArtKind::CounterSmall {
            let visible: Vec<&[i32; 4]> = px.iter().filter(|p| p[3] > 0).collect();
            let coloured = visible.iter().filter(|p| (p[0] - p[1]).abs() > 12 || (p[2] - p[1]).abs() > 12).count();
            if coloured as f64 > 0.05 * visible.len() as f64 {
                return None;
            }
            (px.iter().map(|p| p[3] >= 128 && (p[0] + p[1] + p[2]) / 3 >= 40).collect(), vec![false; w * h])
        } else {
            let blue: Vec<bool> = px.iter().map(|p| p[2] > p[1] + 10 && p[3] > 64).collect();
            let mut excl = blue.clone();
            for _ in 0..4 {
                excl = dilate(&excl, w, h, 1);
            }
            (px.iter().map(|p| p[3] >= 128 && p[1] - p[0] >= 6).collect(), excl)
        };
        let glyph: Vec<bool> = glyph.iter().zip(&excl).map(|(g, e)| *g && !e).collect();
        (glyph.iter().filter(|g| **g).count() >= 20).then_some((glyph, excl))
    }

    struct Stats {
        files: usize,
        mean_r: [f64; 2],
        mean_g_minus_r: [f64; 2],
        bands: [[f64; 2]; 5],
        top_rim: [f64; 2],
        bottom_rim: [f64; 2],
        halo: [[f64; 2]; 4],
        alpha_mae: f64,
    }

    /// Render vanilla's own masks with the style and compare with vanilla, aggregated over all files of the kind.
    fn material_stats(kind: UnitArtKind) -> Option<Stats> {
        let style = GlyphStyle::of(kind);
        let frames = vanilla_frames(kind);
        let mut st = Stats {
            files: 0,
            mean_r: [0.0; 2],
            mean_g_minus_r: [0.0; 2],
            bands: [[0.0; 2]; 5],
            top_rim: [0.0; 2],
            bottom_rim: [0.0; 2],
            halo: [[0.0; 2]; 4],
            alpha_mae: 0.0,
        };
        let (mut n, mut nb, mut nt, mut nbt, mut nh, mut na) = (0f64, [0f64; 5], 0f64, 0f64, [0f64; 4], 0f64);
        for (_, f) in &frames {
            let Some((glyph, excl)) = vanilla_mask(kind, f) else { continue };
            let (w, h) = (f.width as usize, f.height as usize);
            let cov = Plane { w, h, v: glyph.iter().map(|g| if *g { 1.0 } else { 0.0 }).collect() };
            let ours = render_glyph(&cov, &style).unwrap();
            st.files += 1;
            let rows: Vec<usize> = (0..h).filter(|y| (0..w).any(|x| glyph[y * w + x])).collect();
            let (y0, y1) = (rows[0], *rows.last().unwrap());
            let inner = inner_distance(&glyph, w, h, 3);
            let not_glyph: Vec<bool> = glyph.iter().map(|g| !g).collect();
            let outer = inner_distance(&not_glyph, w, h, 6);
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    if excl[i] {
                        continue;
                    }
                    let (a, b) = (f.px(x as u32, y as u32), ours.px(x as u32, y as u32));
                    st.alpha_mae += (f64::from(a[3]) - f64::from(b[3])).abs();
                    na += 1.0;
                    if glyph[i] && a[3] >= 250 {
                        let (ar, br) = (f64::from(a[0]), f64::from(b[0]));
                        st.mean_r[0] += br;
                        st.mean_r[1] += ar;
                        st.mean_g_minus_r[0] += f64::from(b[1]) - br;
                        st.mean_g_minus_r[1] += f64::from(a[1]) - ar;
                        n += 1.0;
                        let t = if y1 > y0 { (y - y0) as f64 / (y1 - y0) as f64 } else { 0.0 };
                        let band = ((t * 5.0) as usize).min(4);
                        st.bands[band][0] += br;
                        st.bands[band][1] += ar;
                        nb[band] += 1.0;
                        if inner[i] < 1.5 {
                            let above = y > 0 && !glyph[i - w];
                            let below = y + 1 < h && !glyph[i + w];
                            if above && !below {
                                st.top_rim[0] += br;
                                st.top_rim[1] += ar;
                                nt += 1.0;
                            }
                            if below && !above {
                                st.bottom_rim[0] += br;
                                st.bottom_rim[1] += ar;
                                nbt += 1.0;
                            }
                        }
                    } else if !glyph[i] {
                        let d = outer[i].round() as usize;
                        if (1..=4).contains(&d) {
                            st.halo[d - 1][0] += f64::from(b[3]);
                            st.halo[d - 1][1] += f64::from(a[3]);
                            nh[d - 1] += 1.0;
                        }
                    }
                }
            }
        }
        if st.files == 0 {
            return None;
        }
        st.mean_r = st.mean_r.map(|v| v / n);
        st.mean_g_minus_r = st.mean_g_minus_r.map(|v| v / n);
        for (b, c) in st.bands.iter_mut().zip(nb) {
            *b = b.map(|v| v / c.max(1.0));
        }
        st.top_rim = st.top_rim.map(|v| v / nt.max(1.0));
        st.bottom_rim = st.bottom_rim.map(|v| v / nbt.max(1.0));
        for (b, c) in st.halo.iter_mut().zip(nh) {
            *b = b.map(|v| v / c.max(1.0));
        }
        st.alpha_mae /= na;
        Some(st)
    }

    #[test]
    fn the_material_is_close_to_vanilla_on_vanillas_own_shapes() {
        for kind in [UnitArtKind::CounterLarge, UnitArtKind::DivisionEmblem, UnitArtKind::CounterSmall] {
            let Some(s) = material_stats(kind) else {
                eprintln!("SKIPPED unit art material vs vanilla ({}): Hearts of Iron IV not found (set HOI4_GAME_DIR)", kind.as_str());
                return;
            };
            let k = kind.as_str();
            eprintln!("{k}: {} vanilla files, ours vs vanilla", s.files);
            eprintln!(
                "{k}:   mean red {:.1} vs {:.1}, green minus red {:.1} vs {:.1}",
                s.mean_r[0], s.mean_r[1], s.mean_g_minus_r[0], s.mean_g_minus_r[1]
            );
            for (i, b) in s.bands.iter().enumerate() {
                eprintln!("{k}:   band {} of 5 (top to bottom) red {:.1} vs {:.1}", i + 1, b[0], b[1]);
            }
            eprintln!("{k}:   top rim red {:.1} vs {:.1}, bottom rim {:.1} vs {:.1}", s.top_rim[0], s.top_rim[1], s.bottom_rim[0], s.bottom_rim[1]);
            for (i, d) in s.halo.iter().enumerate() {
                eprintln!("{k}:   halo alpha at {} px {:.1} vs {:.1}", i + 1, d[0], d[1]);
            }
            eprintln!("{k}:   alpha MAE over the frame {:.2}", s.alpha_mae);
            assert!(s.files >= 50, "{k}: only {} files", s.files);
            assert!((s.mean_r[0] - s.mean_r[1]).abs() < 6.0, "{k}: mean red");
            assert!((s.mean_g_minus_r[0] - s.mean_g_minus_r[1]).abs() < 6.0, "{k}: green against red");
            for b in s.bands {
                assert!((b[0] - b[1]).abs() < 12.0, "{k}: gradient band {b:?}");
            }
            assert!((s.top_rim[0] - s.top_rim[1]).abs() < 15.0 && (s.bottom_rim[0] - s.bottom_rim[1]).abs() < 15.0, "{k}: rims");
            for d in s.halo {
                assert!((d[0] - d[1]).abs() < 15.0, "{k}: halo {d:?}");
            }
            let limit = if kind == UnitArtKind::CounterSmall { 16.0 } else { 8.0 };
            assert!(s.alpha_mae < limit, "{k}: alpha MAE {}", s.alpha_mae);
        }
    }

    /// Vanilla's glyph drawn as a big black silhouette on white, the way a model would deliver it.
    fn as_drawing(glyph: &[bool], w: usize, h: usize, scale: usize) -> Rgba {
        let mut img = Rgba::filled((w * scale + 40) as u32, (h * scale + 40) as u32, [255, 255, 255, 255]);
        for y in 0..h * scale {
            for x in 0..w * scale {
                if glyph[(y / scale) * w + x / scale] {
                    img.set_px((x + 20) as u32, (y + 20) as u32, [10, 10, 10, 255]);
                }
            }
        }
        img
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    #[test]
    fn a_silhouette_lands_at_vanillas_size_and_place() {
        for kind in [UnitArtKind::CounterLarge, UnitArtKind::DivisionEmblem, UnitArtKind::CounterSmall] {
            let frames = vanilla_frames(kind);
            if frames.is_empty() {
                eprintln!("SKIPPED unit art fit vs vanilla ({}): Hearts of Iron IV not found (set HOI4_GAME_DIR)", kind.as_str());
                return;
            }
            let (mut hr, mut wr, mut dx, mut dy) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
            for (_, f) in &frames {
                let Some((glyph, _)) = vanilla_mask(kind, f) else { continue };
                let (w, h) = (f.width as usize, f.height as usize);
                let theirs = Plane { w, h, v: glyph.iter().map(|g| if *g { 1.0 } else { 0.0 }).collect() }.bbox(0.5).unwrap();
                let made = unit_glyph(&as_drawing(&glyph, w, h, 8), kind).unwrap();
                let mine = made.alpha_bbox(200).unwrap();
                let size = |b: [usize; 4]| ((b[2] - b[0]) as f64, (b[3] - b[1]) as f64);
                let (tw, th) = size(theirs);
                let (mw, mh) = ((mine[2] - mine[0]) as f64, (mine[3] - mine[1]) as f64);
                wr.push(mw / tw);
                hr.push(mh / th);
                dx.push(f64::from(mine[0] + mine[2]) / 2.0 - (theirs[0] + theirs[2]) as f64 / 2.0);
                dy.push(f64::from(mine[1] + mine[3]) / 2.0 - (theirs[1] + theirs[3]) as f64 / 2.0);
            }
            let k = kind.as_str();
            let (mw, mh, mx, my) = (median(wr.clone()), median(hr.clone()), median(dx), median(dy));
            eprintln!("{k}: {} silhouettes, our size over vanilla's: width {mw:.2}, height {mh:.2}; centre offset {mx:+.1}, {my:+.1}", wr.len());
            assert!((0.85..=1.2).contains(&mw) && (0.85..=1.2).contains(&mh), "{k}: size ratio {mw} {mh}");
            assert!(mx.abs() <= 2.0 && my.abs() <= 1.5, "{k}: centre offset {mx} {my}");
        }
    }

    /// Our frame 2 for each preset against the vanilla strip of that sub-unit.
    const PRESET_FILES: &[(&str, &str, &str)] = &[
        ("infantry", "unit_infantry_icon", "onmap_unit_infantry_icon"),
        ("motorized", "unit_motorized_icon", "onmap_unit_motorized_icon"),
        ("mechanized", "unit_mechanized_icon", "onmap_unit_mechanized_icon"),
        ("light_armor", "unit_light_tank_icon", "onmap_unit_light_tank_icon"),
        ("medium_armor", "unit_medium_tank_icon", "onmap_unit_medium_tank_icon"),
        ("heavy_armor", "unit_heavy_armor_icon", "onmap_unit_heavy_armor_icon"),
        ("modern_armor", "unit_modern_armor_icon", "onmap_unit_modern_armor_icon"),
        ("super_heavy_armor", "unit_super_heavy_armor_icon", "onmap_unit_super_heavy_armor_icon"),
        ("artillery", "unit_art_icon", "onmap_unit_art_icon"),
        ("motorized_artillery", "unit_mot_art_icon", "onmap_unit_mot_art_icon"),
        ("anti_tank", "unit_at_icon", "onmap_unit_at_icon"),
        ("motorized_anti_tank", "unit_mot_at_icon", "onmap_unit_mot_at_icon"),
        ("anti_air", "unit_anti_air_icon", "onmap_unit_anti_air_icon"),
        ("motorized_anti_air", "unit_mot_anti_air_icon", "onmap_unit_mot_anti_air_icon"),
        ("cavalry", "unit_cavalry_icon", "onmap_unit_cavalry_icon"),
        ("engineer", "unit_engineer_icon", "onmap_unit_engineer_icon"),
        ("mountain", "unit_mountain_icon", "onmap_unit_mountain_icon"),
        ("paratroop", "unit_paratroop_icon", "onmap_unit_paratroop_icon"),
        ("rocket_artillery", "unit_rocket_art_icon", "onmap_unit_rocket_artillery_icon"),
    ];

    /// Mean absolute difference per channel of the two frames composited over mid grey (the colour of a clear
    /// pixel does not matter, so it must not count).
    fn frame_mae(a: &Rgba, b: &Rgba) -> f64 {
        let bg = [128, 128, 128, 255];
        let mut sum = 0f64;
        for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
            let (x, y) = (over([p[0], p[1], p[2], p[3]], bg), over([q[0], q[1], q[2], q[3]], bg));
            sum += (0..3).map(|c| (f64::from(x[c]) - f64::from(y[c])).abs()).sum::<f64>();
        }
        sum / (a.data.len() / 4 * 3) as f64
    }

    #[test]
    fn nato_boxes_are_close_to_vanillas_frame_two() {
        let large: std::collections::BTreeMap<String, Rgba> = vanilla("divisions_large", (152, 42)).into_iter().collect();
        let small: std::collections::BTreeMap<String, Rgba> = vanilla("divisions_small", (60, 12)).into_iter().collect();
        if large.is_empty() || small.is_empty() {
            eprintln!("SKIPPED NATO box vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        let dump = std::env::var("HOI4_UNIT_ART_DUMP").ok();
        let mut sheet = Rgba::new(76 * 2 + 30 * 2 + 12, 44 * PRESET_FILES.len() as u32);
        let (mut worst_l, mut worst_s, mut sum_l, mut sum_s, mut n) = (0f64, 0f64, 0f64, 0f64, 0f64);
        for (row, (preset, l, s)) in PRESET_FILES.iter().enumerate() {
            let marks = parse_symbol(preset).unwrap();
            let (Some(vl), Some(vs)) = (large.get(&format!("{l}.dds")), small.get(&format!("{s}.dds"))) else {
                eprintln!("NATO box: vanilla file for {preset} not found, skipped");
                continue;
            };
            let (tl, ts) = (vl.split_frames(2).unwrap().swap_remove(1), vs.split_frames(2).unwrap().swap_remove(1));
            let (ol, os) = (nato_box(BoxSize::Large, &marks), nato_box(BoxSize::Small, &marks));
            let (el, es) = (frame_mae(&ol, &tl), frame_mae(&os, &ts));
            eprintln!("NATO box {preset:<20} MAE per channel: large {el:5.2}, small {es:5.2}");
            worst_l = worst_l.max(el);
            worst_s = worst_s.max(es);
            sum_l += el;
            sum_s += es;
            n += 1.0;
            let y = row as u32 * 44;
            for (img, x) in [(&ol, 0), (&tl, 76), (&os, 154), (&ts, 186)] {
                sheet.paste(img, x, i64::from(y));
            }
        }
        eprintln!("NATO box: mean MAE large {:.2}, small {:.2}; worst large {worst_l:.2}, small {worst_s:.2}", sum_l / n, sum_s / n);
        if let Some(dir) = dump {
            let _ = std::fs::write(std::path::Path::new(&dir).join("nato_vs_vanilla.png"), crate::pixels::encode_png(&sheet).unwrap());
        }
        assert!(n >= 15.0);
        // the small box is pixel art at 30x12, where vanilla is not even consistent with itself (its super-heavy oval is
        // faint grey, its wheels move with the other marks), so one pixel of difference costs much more there
        assert!(sum_l / n < 2.5 && sum_s / n < 8.0, "mean large {}, small {}", sum_l / n, sum_s / n);
        assert!(worst_l < 5.0 && worst_s < 18.0, "worst large {worst_l}, small {worst_s}");
    }

    /// Every vanilla strip and symbol counts as finished art in vanilla's material; the gallery's imported line art and
    /// a recoloured vanilla counter do not; our own processed strips do.
    #[test]
    fn the_material_check_accepts_vanilla_and_refuses_other_art() {
        // the gallery's imported art (hoi4-imagegen 0.2.0 conformed it unchanged)
        let fixture = |name: &str| {
            let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gallery").join(name);
            crate::pixels::decode(&std::fs::read(p).unwrap(), Some(name), Default::default()).unwrap().0
        };
        for (name, kind) in [
            ("combat_engineers-v2-p1.png", UnitArtKind::CounterLarge),
            ("infantry_rifle-v1-p1.png", UnitArtKind::CounterSmall),
            ("lightning_shield-v1-p1.png", UnitArtKind::DivisionEmblem),
        ] {
            let p = material_problems(&fixture(name), kind);
            eprintln!("{name}: {p:?}");
            assert!(!p.is_empty(), "{name} passed the material check");
        }
        // our own strips pass
        for kind in [UnitArtKind::CounterLarge, UnitArtKind::CounterSmall] {
            let glyph = unit_glyph(&drawn_tank(800, 600), kind).unwrap();
            let strip = counter_strip(&glyph, kind, &[Mark::Cross, Mark::Plus]).unwrap();
            assert!(material_problems(&strip, kind).is_empty(), "{kind:?}: {:?}", material_problems(&strip, kind));
        }
        let emblem = unit_glyph(&drawn_tank(800, 600), UnitArtKind::DivisionEmblem).unwrap();
        assert!(material_problems(&emblem, UnitArtKind::DivisionEmblem).is_empty());
        assert!(!material_problems(&emblem, UnitArtKind::CounterLarge).is_empty(), "wrong size");
        // vanilla
        let Some(_) = game_dir() else {
            eprintln!("SKIPPED material check vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        };
        for (kind, folder, size) in [
            (UnitArtKind::CounterLarge, "divisions_large", (152, 42)),
            (UnitArtKind::CounterSmall, "divisions_small", (60, 12)),
            (UnitArtKind::DivisionEmblem, "division_templates_large", (76, 42)),
        ] {
            let files = vanilla(folder, size);
            let (mut checked, mut failed) = (0, Vec::new());
            for (name, img) in &files {
                // coloured on-map counters (a few nations' own) are not the grey material, as the measurement found
                let f1 = img.split_frames(if kind == UnitArtKind::DivisionEmblem { 1 } else { 2 }).unwrap().swap_remove(0);
                if vanilla_mask(kind, &f1).is_none() {
                    continue;
                }
                checked += 1;
                let p = material_problems(img, kind);
                if !p.is_empty() {
                    failed.push(format!("{name}: {p:?}"));
                }
            }
            eprintln!("{}: {checked} vanilla files checked, {} refused", kind.as_str(), failed.len());
            assert!(checked >= 50, "{kind:?}: only {checked}");
            // HOI4 1.19.4.1: 3 of 111 battalion strips, 2 of 81 grey on-map strips and 3 of 101 symbols are refused,
            // special art such as the guided missile, transport helicopter and mothership counters, whose frame 2 is
            // not the standard NATO box
            assert!(failed.len() * 20 <= checked, "{kind:?}: more than 5 percent of vanilla refused: {failed:#?}");
            // vanilla's counter recoloured red is refused
            let (_, first) = files
                .iter()
                .find(|(_, i)| vanilla_mask(kind, &i.split_frames(if kind == UnitArtKind::DivisionEmblem { 1 } else { 2 }).unwrap()[0]).is_some())
                .unwrap();
            let mut red = first.clone();
            for p in red.data.chunks_exact_mut(4) {
                p.swap(1, 2);
                p[0] = p[0].saturating_add(60);
            }
            assert!(!material_problems(&red, kind).is_empty(), "{kind:?}: a recoloured strip passed");
        }
    }

    /// With HOI4_UNIT_ART_DUMP set, writes vanilla frame 1 next to our rendering of its own silhouette, for a look.
    #[test]
    fn dump_material_sheet_when_asked() {
        let Ok(dir) = std::env::var("HOI4_UNIT_ART_DUMP") else { return };
        for kind in [UnitArtKind::CounterLarge, UnitArtKind::DivisionEmblem, UnitArtKind::CounterSmall] {
            let frames: Vec<(String, Rgba)> = vanilla_frames(kind).into_iter().take(24).collect();
            let style = GlyphStyle::of(kind);
            let [fw, fh] = style.frame;
            let mut sheet = Rgba::filled(fw * 3 * 4, (fh + 2) * 6, [60, 90, 140, 255]);
            for (i, (_, f)) in frames.iter().enumerate() {
                let Some((glyph, _)) = vanilla_mask(kind, f) else { continue };
                let made = unit_glyph(&as_drawing(&glyph, fw as usize, fh as usize, 8), kind).unwrap();
                let (col, row) = (i as u32 % 4, i as u32 / 4);
                let mut a = Rgba::filled(fw, fh, [60, 90, 140, 255]);
                a.paste_over(f, 0, 0);
                let mut b = Rgba::filled(fw, fh, [60, 90, 140, 255]);
                b.paste_over(&made, 0, 0);
                sheet.paste(&a, i64::from(col * fw * 3), i64::from(row * (fh + 2)));
                sheet.paste(&b, i64::from(col * fw * 3 + fw), i64::from(row * (fh + 2)));
            }
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("{}_vs_vanilla.png", kind.as_str())),
                crate::pixels::encode_png(&sheet).unwrap(),
            );
        }
    }
}
