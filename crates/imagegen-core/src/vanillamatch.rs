//! The vanilla-match check of the review: does a candidate measure like its family's vanilla art?
//!
//! The 0.3.1 review approved whatever the caller answered `true` to, so caller answers alone could approve art of
//! the wrong form: a loose tank where vanilla commander traits are ribbon medals, a grey-filled rocket where vanilla
//! special project icons are pale line drawings, a medal far larger on its canvas than any vanilla medal. The review
//! now runs this check itself, and an approval needs it to pass.
//!
//! [`measure`] turns an image into named features (see [`NAMES`] and [`MEANINGS`]): palette (lightness,
//! saturation, value and their spread, dark, bright and middle shares, the colours needed for most pixels, gold and
//! bronze tones), outline share, edge density, alpha coverage, soft alpha and the opaque border (a background fill),
//! and form (the subject's size, position and aspect, how much of its box, convex hull and inscribed ellipse it
//! fills, left-right symmetry, the saturation of its top band where a ribbon sits).
//!
//! [`calibrate`] fits an accepted range per feature from the family's own vanilla files: the 1st to 99th percentile,
//! widened on each side by half of that width, and never narrower than the feature's floor. Every feature is
//! measured and reported, but only the [`CHECKED`] ones can fail a candidate: the form features and the two medium
//! features that tell painted art, line art and grey art apart. The others (contrast, edge density, soft alpha,
//! palette size) are only reported: art of the right form spreads as widely in them as art of the wrong form, so a
//! range on them would refuse right art without refusing wrong art. The checked list and the widening keep at least
//! 97 percent of every family's own vanilla files inside (measured on HOI4 1.19.4.1); `tests/vanilla_match.rs`
//! asserts those pass rates and refuses synthetic art drawn outside each rule.
//!
//! A family whose vanilla art is one consistent palette can add palette features of its own, with a narrower range
//! ([`family_checks`]): agency upgrades check their saturation, value and gold tones, so a saturated, warm painted
//! medallion that the general features passed is refused, and the four portrait families check the backdrop in the top
//! corners against their own vanilla files. The version is `vanilla_match_v2` since that change.
//!
//! Everything is a pure function of pixels and the vanilla files, so a check repeats exactly. The vanilla files
//! are read from the installed game and are never copied anywhere.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::family::Family;
use crate::pixels::Rgba;
use crate::resize::resize_lanczos3;

/// Version of the features and the calibration rule. Recorded with every check, so a later change is visible.
pub const VERSION: &str = "vanilla_match_v2";

/// Lower and upper percentile of the accepted range.
pub const RANGE_PERCENTILES: (f64, f64) = (0.01, 0.99);

/// Share of the percentile range added on each side.
pub const RANGE_WIDENING: f64 = 0.5;

/// Fewest vanilla files a family needs for a calibration.
pub const MIN_FILES: usize = 3;

/// Most vanilla files measured per family. Larger sets are sampled evenly by path.
pub const MAX_FILES: usize = 400;

/// Longest side measured. Larger images are shrunk first, as the style descriptor does.
pub const MAX_SIDE: u32 = 160;

/// Number of features.
pub const N: usize = 29;

/// Feature names, in vector order.
pub const NAMES: [&str; N] = [
    "lightness_mean",
    "lightness_spread",
    "saturation_mean",
    "saturation_spread",
    "value_mean",
    "value_spread",
    "dark_share",
    "bright_share",
    "palette_colours",
    "edge_density",
    "outline_share",
    "alpha_coverage",
    "soft_alpha_share",
    "border_opacity",
    "subject_span",
    "subject_center_x",
    "subject_center_y",
    "subject_aspect",
    "box_fill",
    "hull_fill",
    "mirror_symmetry",
    "top_band_saturation",
    "mid_tone_share",
    "ellipse_fill",
    "warm_metal_share",
    "rim_warm_metal_share",
    "backdrop_lightness",
    "backdrop_yellow",
    "backdrop_glow",
];

/// The features that can fail a candidate: form (coverage, background fill, size, position, aspect, the fills of the
/// box, hull and ellipse, symmetry, the top band) and the two medium features that separate painted, line and grey
/// art (the middle-value share and the saturation). See the module notes for why the others are only reported.
pub const CHECKED: [&str; 13] = [
    "alpha_coverage",
    "border_opacity",
    "subject_span",
    "subject_center_x",
    "subject_center_y",
    "subject_aspect",
    "box_fill",
    "hull_fill",
    "mirror_symmetry",
    "ellipse_fill",
    "top_band_saturation",
    "mid_tone_share",
    "saturation_mean",
];

/// A palette feature checked for one family only: its name, the share of the percentile range added on each side (in
/// place of [`RANGE_WIDENING`]) and the percentiles of that range (in place of [`RANGE_PERCENTILES`]).
pub type FamilyCheck = (&'static str, f64, (f64, f64));

/// Palette features checked for one family only. The general palette features are left unchecked because art of the
/// right form spreads widely in them, but where a family's vanilla art is one consistent palette or backdrop,
/// a candidate far from it is the wrong medium. Measured on HOI4 1.19.4.1:
///
/// * `agency_upgrade`: vanilla's 15 upgrades are muted vignettes, mean saturation 0.115 to 0.402 (median 0.249), mean
///   value 0.239 to 0.518 (median 0.423), gold and bronze tones on 0 to 0.399 of the disc (median 0.106). With the
///   general widening the saturation range ran from -0.017 to 0.541, so a saturated, warm painted medallion passed.
///   A quarter of the percentile range on each side keeps every vanilla file inside and refuses it: saturation 0.053
///   to 0.471, value 0.179 to 0.586, gold tones up to 0.476.
/// * Portraits, the backdrop in the top corners. Leaders and commanders (one pale cream-grey template, yellow 8.5 in
///   the median, about one file in twenty a special dark or patterned backdrop): the yellow-blue tone with 0.65 of the
///   percentile range, -10.7 to 27.1, which keeps every vanilla file and refuses the scientists' blue-grey (-14.4).
///   Their lightness spreads over the special portraits and is checked with the general rule. Scientists (217 files of
///   one pale blue-grey template, lightness 66.0 and yellow -14.4 in the median, but three special ones down to
///   lightness 19.5): their whole range widened by a tenth, so every vanilla file passes (lightness 14.5 to 74.6, yellow
///   -16.5 to 8.7) and the leaders' cream (lightness 89.8) does not. Operatives (La Resistance's 163 files, corners 11.9 lightness, 5.3 yellow): their whole
///   range widened by a quarter, which refuses the leaders' cream and the scientists' blue-grey, and the light
///   behind the figure (`backdrop_glow`, 17.1 to 29.2, checked 14.2 to 32.2), which refuses a dark but evenly lit backdrop.
pub fn family_checks(family: Family) -> &'static [FamilyCheck] {
    match family {
        Family::AgencyUpgrade => {
            &[("saturation_mean", 0.25, RANGE_PERCENTILES), ("value_mean", 0.25, RANGE_PERCENTILES), ("warm_metal_share", 0.25, RANGE_PERCENTILES)]
        }
        Family::LeaderPortrait | Family::CommanderPortrait => {
            &[("backdrop_lightness", RANGE_WIDENING, RANGE_PERCENTILES), ("backdrop_yellow", 0.65, RANGE_PERCENTILES)]
        }
        Family::ScientistPortrait => &[("backdrop_lightness", 0.1, (0.0, 1.0)), ("backdrop_yellow", 0.1, (0.0, 1.0))],
        Family::OperativePortrait => {
            &[("backdrop_lightness", 0.25, (0.0, 1.0)), ("backdrop_yellow", 0.25, (0.0, 1.0)), ("backdrop_glow", 0.25, (0.0, 1.0))]
        }
        _ => &[],
    }
}

/// What each feature means, for messages and the docs.
pub const MEANINGS: [&str; N] = [
    "mean CIE L* lightness of the visible pixels",
    "spread of L* lightness",
    "mean HSV saturation",
    "spread of saturation",
    "mean HSV value",
    "spread of value",
    "share of visible pixels with value below 0.2",
    "share of visible pixels with value above 0.8",
    "colour buckets needed for 90% of the opaque pixels",
    "share of interior pixels on a lightness edge",
    "share of the subject's rim darker than its mean",
    "share of the canvas that is visible",
    "share of visible pixels that are only partly opaque (line art and glows)",
    "share of the canvas border that is opaque (a background fill)",
    "larger side of the subject's box as a share of the canvas",
    "horizontal centre of the subject (0 left, 1 right)",
    "vertical centre of the subject (0 top, 1 bottom)",
    "width of the subject's box divided by its height",
    "share of the subject's box that is opaque",
    "share of the subject's convex hull that is opaque (a plaque or disc is near 1, a loose subject lower)",
    "left-right symmetry of the subject's shape",
    "mean saturation of the top fifth of the subject (a ribbon)",
    "share of visible pixels with a middle value, 0.25 to 0.75 (shading and fill, where line art is all light)",
    "share of the ellipse inscribed in the subject's box that is opaque (a plaque, disc or badge is near 1)",
    "share of visible pixels in gold or bronze tones (hue 20 to 60 degrees, saturation 0.25 to 0.9, value 0.35 or more)",
    "share of the subject's outer ring (opaque pixels within two pixels of a clear one) in gold or bronze tones (a metal rim)",
    "mean CIE L* lightness of the two top corners (a tenth of the width, 7.5 percent of the height each): a portrait's backdrop",
    "mean CIE b* of the two top corners, yellow above 0 and blue below: a portrait's backdrop, cream for leaders, blue-grey for scientists",
    "CIE L* of the side edges at two thirds of the height (a tenth of the width each, 62 to 72 percent of the height) minus that of the top corners: the light behind a portrait's figure, high for an operative's spotlight",
];

/// Smallest half-width of an accepted range, per feature. Keeps a family whose vanilla art is nearly uniform in a
/// feature from refusing a rounding difference.
pub const FLOOR: [f64; N] = [
    4.0, 3.0, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 2.0, 0.03, 0.05, 0.03, 0.03, 0.03, 0.03, 0.03, 0.03, 0.08, 0.04, 0.04, 0.04, 0.04, 0.04, 0.04,
    0.04, 0.04, 4.0, 3.0, 4.0,
];

/// The measured features of one image.
pub type Features = [f64; N];

fn srgb_lut() -> [f64; 256] {
    let mut lut = [0f64; 256];
    for (i, slot) in lut.iter_mut().enumerate() {
        let c = i as f64 / 255.0;
        *slot = if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) };
    }
    lut
}

fn lightness(r: u8, g: u8, b: u8, lut: &[f64; 256]) -> f64 {
    let y = 0.212_672_9 * lut[usize::from(r)] + 0.715_152_2 * lut[usize::from(g)] + 0.072_175_0 * lut[usize::from(b)];
    let f = if y > 0.008_856 { y.cbrt() } else { 7.787 * y + 16.0 / 116.0 };
    116.0 * f - 16.0
}

/// The two top corners of an image, each a tenth of the width and 7.5 percent of the height (16 by 16 px on a 156x210
/// portrait): where a portrait shows its backdrop beside the head.
pub const BACKDROP_CORNER: (f64, f64) = (0.10, 0.075);

/// Mean CIE L*a*b* of the opaque pixels (alpha 128 and up) in the two top corners, see [`BACKDROP_CORNER`]. `None`
/// when no corner pixel is opaque.
pub fn backdrop_lab(img: &Rgba) -> Option<[f64; 3]> {
    let cw = ((f64::from(img.width) * BACKDROP_CORNER.0).round() as u32).max(1);
    let ch = ((f64::from(img.height) * BACKDROP_CORNER.1).round() as u32).max(1);
    let xs: Vec<u32> = (0..cw.min(img.width)).chain(img.width.saturating_sub(cw)..img.width).collect();
    mean_lab(img, 0..ch.min(img.height), &xs)
}

/// The side edges at two thirds of a portrait's height: a tenth of the width at each side, 62 to 72 percent of the
/// height, beside the shoulders, where the light behind the figure shows.
pub const BACKDROP_GLOW_BAND: (f64, f64, f64) = (0.10, 0.62, 0.72);

/// Mean CIE L*a*b* of the opaque pixels in the side band [`BACKDROP_GLOW_BAND`]. `None` when none is opaque.
pub fn glow_band_lab(img: &Rgba) -> Option<[f64; 3]> {
    let cw = ((f64::from(img.width) * BACKDROP_GLOW_BAND.0).round() as u32).max(1);
    let y0 = (f64::from(img.height) * BACKDROP_GLOW_BAND.1).round() as u32;
    let y1 = ((f64::from(img.height) * BACKDROP_GLOW_BAND.2).round() as u32).max(y0 + 1).min(img.height);
    let xs: Vec<u32> = (0..cw.min(img.width)).chain(img.width.saturating_sub(cw)..img.width).collect();
    mean_lab(img, y0..y1, &xs)
}

/// How much lighter the side band at two thirds of the height is than the top corners, in CIE L*
/// ([`glow_band_lab`] minus [`backdrop_lab`]). Vanilla's operative portraits sit in a soft light behind the figure,
/// so it is high for them, and it is below zero for the evenly lit or top-lit leader and scientist backdrops.
pub fn backdrop_glow(img: &Rgba) -> Option<f64> {
    Some(glow_band_lab(img)?[0] - backdrop_lab(img)?[0])
}

fn mean_lab(img: &Rgba, ys: std::ops::Range<u32>, xs: &[u32]) -> Option<[f64; 3]> {
    let lut = srgb_lut();
    let (mut s, mut n) = ([0f64; 3], 0f64);
    for y in ys {
        for &x in xs {
            let p = img.px(x, y);
            if p[3] < 128 {
                continue;
            }
            let lin = |c: u8| lut[usize::from(c)];
            let (r, g, b) = (lin(p[0]), lin(p[1]), lin(p[2]));
            let xx = (0.412_456_4 * r + 0.357_576_1 * g + 0.180_437_5 * b) / 0.950_47;
            let yy = 0.212_672_9 * r + 0.715_152_2 * g + 0.072_175_0 * b;
            let zz = (0.019_333_9 * r + 0.119_192_0 * g + 0.950_304_1 * b) / 1.088_83;
            let f = |t: f64| if t > 0.008_856 { t.cbrt() } else { 7.787 * t + 16.0 / 116.0 };
            let (fx, fy, fz) = (f(xx), f(yy), f(zz));
            s[0] += 116.0 * fy - 16.0;
            s[1] += 500.0 * (fx - fy);
            s[2] += 200.0 * (fy - fz);
            n += 1.0;
        }
    }
    (n > 0.0).then(|| s.map(|v| v / n))
}

/// Twice the signed area of the triangle o, a, b.
fn cross(o: (i64, i64), a: (i64, i64), b: (i64, i64)) -> i64 {
    (a.0 - o.0) * (b.1 - o.1) - (a.1 - o.1) * (b.0 - o.0)
}

/// Area of the convex hull of the pixel squares in `rows` (`(y, first x, last x)` per row with any pixel).
fn hull_area(rows: &[(i64, i64, i64)]) -> f64 {
    let mut pts: Vec<(i64, i64)> = Vec::with_capacity(rows.len() * 4);
    for &(y, a, b) in rows {
        pts.extend([(a, y), (a, y + 1), (b + 1, y), (b + 1, y + 1)]);
    }
    pts.sort_unstable();
    pts.dedup();
    if pts.len() < 3 {
        return 0.0;
    }
    let mut lower: Vec<(i64, i64)> = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0 {
            lower.pop();
        }
        lower.push(p);
    }
    let mut upper: Vec<(i64, i64)> = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0 {
            upper.pop();
        }
        upper.push(p);
    }
    lower.pop();
    upper.pop();
    lower.extend(upper);
    let mut twice = 0i64;
    for i in 0..lower.len() {
        let (a, b) = (lower[i], lower[(i + 1) % lower.len()]);
        twice += a.0 * b.1 - b.0 * a.1;
    }
    twice.abs() as f64 / 2.0
}

/// Measure an image. `None` when it has no visible pixel (alpha of 16 or more) or no opaque one (128 or more).
pub fn measure(src: &Rgba) -> Option<Features> {
    let longest = src.width.max(src.height);
    let shrunk;
    let img = if longest > MAX_SIDE {
        let scale = f64::from(MAX_SIDE) / f64::from(longest);
        let nw = ((f64::from(src.width) * scale).round() as u32).max(1);
        let nh = ((f64::from(src.height) * scale).round() as u32).max(1);
        shrunk = resize_lanczos3(src, nw, nh).ok()?;
        &shrunk
    } else {
        src
    };
    let (w, h) = (img.width as usize, img.height as usize);
    let n = w * h;
    if n == 0 {
        return None;
    }
    let lut = srgb_lut();
    let alpha = |i: usize| img.data[i * 4 + 3];
    let mut vis = vec![false; n];
    let mut lum = vec![0f64; n];
    let (mut count, mut soft) = (0u64, 0u64);
    let (mut sl, mut ql, mut ss, mut qs, mut sv, mut qv) = (0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
    let (mut dark, mut bright, mut mid) = (0u64, 0u64, 0u64);
    let mut buckets: BTreeMap<u16, u64> = BTreeMap::new();
    let mut sat = vec![0f64; n];
    let mut warm = vec![false; n];
    let mut warm_n = 0u64;
    for i in 0..n {
        let a = alpha(i);
        if a < 16 {
            continue;
        }
        let p = &img.data[i * 4..i * 4 + 4];
        vis[i] = true;
        count += 1;
        if a < 224 {
            soft += 1;
        }
        let l = lightness(p[0], p[1], p[2], &lut);
        lum[i] = l;
        sl += l;
        ql += l * l;
        let (hd, s, v) = crate::styledesc::hsv(p[0], p[1], p[2]);
        sat[i] = s;
        if (20.0..=60.0).contains(&hd) && (0.25..=0.9).contains(&s) && v >= 0.35 {
            warm[i] = true;
            warm_n += 1;
        }
        ss += s;
        qs += s * s;
        sv += v;
        qv += v * v;
        if v < 0.2 {
            dark += 1;
        }
        if v > 0.8 {
            bright += 1;
        }
        if (0.25..=0.75).contains(&v) {
            mid += 1;
        }
        if a >= 128 {
            *buckets.entry((u16::from(p[0] >> 5) << 6) | (u16::from(p[1] >> 5) << 3) | u16::from(p[2] >> 5)).or_default() += 1;
        }
    }
    if count == 0 {
        return None;
    }
    let c = count as f64;
    let sd = |q: f64, m: f64| (q / c - m * m).max(0.0).sqrt();
    let (ml, ms, mv) = (sl / c, ss / c, sv / c);

    // palette: buckets needed for 90% of the opaque pixels
    let mut sizes: Vec<u64> = buckets.values().copied().collect();
    sizes.sort_unstable_by(|a, b| b.cmp(a));
    let opaque_total: u64 = sizes.iter().sum();
    if opaque_total == 0 {
        return None;
    }
    let (mut acc, mut colours) = (0u64, 0usize);
    for s in &sizes {
        if acc * 10 >= opaque_total * 9 {
            break;
        }
        acc += s;
        colours += 1;
    }

    // edges (as the style descriptor measures them)
    let (mut interior, mut edgy) = (0u64, 0u64);
    if w >= 3 && h >= 3 {
        for y in 1..h - 1 {
            for x in 1..w - 1 {
                let idx = |dx: usize, dy: usize| (y + dy - 1) * w + (x + dx - 1);
                if !(0..3).all(|dy| (0..3).all(|dx| vis[idx(dx, dy)])) {
                    continue;
                }
                let l = |dx: usize, dy: usize| lum[idx(dx, dy)];
                let gx = (l(2, 0) + 2.0 * l(2, 1) + l(2, 2)) - (l(0, 0) + 2.0 * l(0, 1) + l(0, 2));
                let gy = (l(0, 2) + 2.0 * l(1, 2) + l(2, 2)) - (l(0, 0) + 2.0 * l(1, 0) + l(2, 0));
                interior += 1;
                if gx.hypot(gy) / 4.0 > crate::styledesc::EDGE_THRESHOLD {
                    edgy += 1;
                }
            }
        }
    }
    let edge_density = if interior < 8 { 0.0 } else { edgy as f64 / interior as f64 };

    // outline: rim pixels (opaque, next to a transparent one, or on the canvas edge) darker than the mean
    let (mut rim, mut rim_dark) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            let i = y * w + x;
            if alpha(i) < 128 {
                continue;
            }
            let edge = x == 0 || y == 0 || x + 1 == w || y + 1 == h;
            let next_to_clear = [(x > 0).then(|| i - 1), (x + 1 < w).then(|| i + 1), (y > 0).then(|| i - w), (y + 1 < h).then(|| i + w)]
                .into_iter()
                .flatten()
                .any(|j| alpha(j) < 16);
            if edge || next_to_clear {
                rim += 1;
                if lum[i] < ml - 12.0 {
                    rim_dark += 1;
                }
            }
        }
    }
    let outline = if rim == 0 { 0.0 } else { rim_dark as f64 / rim as f64 };

    // border opacity
    let mut border = 0u64;
    let mut border_opaque = 0u64;
    for y in 0..h {
        for x in 0..w {
            if x == 0 || y == 0 || x + 1 == w || y + 1 == h {
                border += 1;
                if alpha(y * w + x) >= 128 {
                    border_opaque += 1;
                }
            }
        }
    }

    // the subject: opaque pixels (alpha of 128 or more)
    let solid = |x: usize, y: usize| alpha(y * w + x) >= 128;
    let (mut x0, mut y0, mut x1, mut y1) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let (mut sx, mut sy, mut solid_n) = (0f64, 0f64, 0u64);
    let mut rows: Vec<(i64, i64, i64)> = Vec::new();
    for y in 0..h {
        let mut first = None;
        let mut last = 0usize;
        for x in 0..w {
            if solid(x, y) {
                first.get_or_insert(x);
                last = x;
                solid_n += 1;
                sx += x as f64 + 0.5;
                sy += y as f64 + 0.5;
            }
        }
        if let Some(f) = first {
            rows.push((y as i64, f as i64, last as i64));
            x0 = x0.min(f);
            x1 = x1.max(last + 1);
            y0 = y0.min(y);
            y1 = y1.max(y + 1);
        }
    }
    if solid_n == 0 {
        return None;
    }
    let (bw, bh) = ((x1 - x0) as f64, (y1 - y0) as f64);
    let hull = hull_area(&rows);
    let (mut both, mut either) = (0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let m = x0 + x1 - 1 - x;
            let (a, b) = (solid(x, y), solid(m, y));
            if a && b {
                both += 1;
            }
            if a || b {
                either += 1;
            }
        }
    }
    let (ecx, ecy, erx, ery) = ((x0 + x1) as f64 / 2.0, (y0 + y1) as f64 / 2.0, bw / 2.0, bh / 2.0);
    let (mut in_ellipse, mut in_ellipse_solid) = (0u64, 0u64);
    for y in y0..y1 {
        for x in x0..x1 {
            let (dx, dy) = ((x as f64 + 0.5 - ecx) / erx, (y as f64 + 0.5 - ecy) / ery);
            if dx * dx + dy * dy <= 1.0 {
                in_ellipse += 1;
                if solid(x, y) {
                    in_ellipse_solid += 1;
                }
            }
        }
    }
    // the outer ring: opaque pixels within two pixels (Chebyshev) of a clear pixel or the canvas edge
    let (mut ring, mut ring_warm) = (0u64, 0u64);
    for y in 0..h {
        for x in 0..w {
            if !solid(x, y) {
                continue;
            }
            let near_clear = (y.saturating_sub(2)..(y + 3).min(h + 2))
                .any(|yy| (x.saturating_sub(2)..(x + 3).min(w + 2)).any(|xx| xx >= w || yy >= h || alpha(yy * w + xx) < 16))
                || x < 2
                || y < 2;
            if near_clear {
                ring += 1;
                if warm[y * w + x] {
                    ring_warm += 1;
                }
            }
        }
    }
    let backdrop = backdrop_lab(img).map_or([0.0, 0.0], |c| [c[0], c[2]]);
    let glow = backdrop_glow(img).unwrap_or(0.0);
    let band_end = y0 + ((y1 - y0) / 5).max(1);
    let (mut band_sat, mut band_n) = (0f64, 0u64);
    for y in y0..band_end {
        for x in x0..x1 {
            if solid(x, y) {
                band_sat += sat[y * w + x];
                band_n += 1;
            }
        }
    }

    Some([
        ml,
        sd(ql, ml),
        ms,
        sd(qs, ms),
        mv,
        sd(qv, mv),
        dark as f64 / c,
        bright as f64 / c,
        colours as f64,
        edge_density,
        outline,
        c / n as f64,
        soft as f64 / c,
        border_opaque as f64 / border.max(1) as f64,
        (bw / w as f64).max(bh / h as f64),
        sx / solid_n as f64 / w as f64,
        sy / solid_n as f64 / h as f64,
        bw / bh.max(1.0),
        solid_n as f64 / (bw * bh).max(1.0),
        if hull > 0.0 { (solid_n as f64 / hull).min(1.0) } else { 1.0 },
        if either == 0 { 1.0 } else { both as f64 / either as f64 },
        if band_n == 0 { 0.0 } else { band_sat / band_n as f64 },
        mid as f64 / c,
        if in_ellipse == 0 { 1.0 } else { in_ellipse_solid as f64 / in_ellipse as f64 },
        warm_n as f64 / c,
        if ring == 0 { 0.0 } else { ring_warm as f64 / ring as f64 },
        backdrop[0],
        backdrop[1],
        glow,
    ])
}

/// The accepted range of one feature.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FeatureRange {
    /// Feature name.
    pub feature: String,
    /// Lowest accepted value.
    pub min: f64,
    /// Highest accepted value.
    pub max: f64,
    /// Vanilla's median.
    pub median: f64,
    /// True when a value outside the range fails the check (the [`CHECKED`] features).
    pub checked: bool,
}

/// A family's accepted ranges, fitted from its vanilla files.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Calibration {
    /// Family.
    pub family: Family,
    /// Feature and rule version.
    pub version: String,
    /// Vanilla files measured.
    pub files: usize,
    /// One range per feature, in [`NAMES`] order.
    pub ranges: Vec<FeatureRange>,
}

/// A feature outside vanilla's range.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Outside {
    /// Feature name.
    pub feature: String,
    /// What it measures.
    pub meaning: String,
    /// The candidate's value.
    pub value: f64,
    /// Accepted minimum.
    pub min: f64,
    /// Accepted maximum.
    pub max: f64,
    /// Vanilla's median.
    pub median: f64,
}

/// The outcome of a check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MatchResult {
    /// True when every checked feature is inside vanilla's range.
    pub pass: bool,
    /// Checked features outside their range, furthest (in range widths) first.
    pub outside: Vec<Outside>,
    /// Every measured feature.
    pub features: BTreeMap<String, f64>,
}

fn quantile(sorted: &[f64], q: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let pos = q * (sorted.len() - 1) as f64;
    let (lo, hi) = (pos.floor() as usize, pos.ceil() as usize);
    sorted[lo] + (sorted[hi] - sorted[lo]) * (pos - lo as f64)
}

fn round(x: f64) -> f64 {
    (x * 10_000.0).round() / 10_000.0
}

/// Fit a family's ranges from its vanilla files' features. `None` with fewer than [`MIN_FILES`] rows.
pub fn calibrate(family: Family, rows: &[Features]) -> Option<Calibration> {
    if rows.len() < MIN_FILES {
        return None;
    }
    let mut ranges = Vec::with_capacity(N);
    for d in 0..N {
        let mut col: Vec<f64> = rows.iter().map(|r| r[d]).collect();
        col.sort_by(f64::total_cmp);
        let extra = family_checks(family).iter().find(|(n, _, _)| *n == NAMES[d]);
        let (widening, (qlo, qhi)) = extra.map_or((RANGE_WIDENING, RANGE_PERCENTILES), |(_, w, q)| (*w, *q));
        let (plo, p50, phi) = (quantile(&col, qlo), quantile(&col, 0.5), quantile(&col, qhi));
        let pad = widening * (phi - plo);
        let (mut lo, mut hi) = (plo - pad, phi + pad);
        let mid = (lo + hi) / 2.0;
        if hi - lo < 2.0 * FLOOR[d] {
            lo = mid - FLOOR[d];
            hi = mid + FLOOR[d];
        }
        ranges.push(FeatureRange {
            feature: NAMES[d].to_string(),
            min: round(lo),
            max: round(hi),
            median: round(p50),
            checked: CHECKED.contains(&NAMES[d]) || extra.is_some(),
        });
    }
    Some(Calibration { family, version: VERSION.to_string(), files: rows.len(), ranges })
}

/// Fit ranges from a small set (a style pack's references): each feature's minimum to maximum, widened on each side
/// by `widening` times that width, and never narrower than `floor_factor` times the feature's floor. `version` names
/// the rule in the result. `None` with fewer than [`MIN_FILES`] rows. The median is reported as for [`calibrate`].
///
/// `shade` (one value per feature, or `None`) is how far the feature moves when the references are drawn one shade
/// lighter or darker. Where the references barely spread in a feature (the floor decides its range), the half-width is
/// at least that: one shade step then cannot fail a candidate. `unchecked` names features that are measured and
/// reported but never fail a candidate of this family.
pub fn calibrate_min_max(
    family: Family,
    rows: &[Features],
    widening: f64,
    floor_factor: f64,
    version: &str,
    shade: Option<&Features>,
    unchecked: &[&str],
) -> Option<Calibration> {
    if rows.len() < MIN_FILES {
        return None;
    }
    let mut ranges = Vec::with_capacity(N);
    for d in 0..N {
        let mut col: Vec<f64> = rows.iter().map(|r| r[d]).collect();
        col.sort_by(f64::total_cmp);
        let (min, max) = (col[0], col[col.len() - 1]);
        let pad = widening * (max - min);
        let (mut lo, mut hi) = (min - pad, max + pad);
        let mid = (lo + hi) / 2.0;
        let floor = floor_factor * FLOOR[d];
        if hi - lo < 2.0 * floor {
            // degenerate: the references hardly spread, so the floor (or one shade step, if larger) decides
            let half = floor.max(shade.map_or(0.0, |s| s[d]));
            lo = mid - half;
            hi = mid + half;
        }
        ranges.push(FeatureRange {
            feature: NAMES[d].to_string(),
            min: round(lo),
            max: round(hi),
            median: round(quantile(&col, 0.5)),
            checked: CHECKED.contains(&NAMES[d]) && !unchecked.contains(&NAMES[d]),
        });
    }
    Some(Calibration { family, version: version.to_string(), files: rows.len(), ranges })
}

impl Calibration {
    /// Check a candidate's features against the ranges.
    pub fn check(&self, f: &Features) -> MatchResult {
        let mut outside: Vec<(f64, Outside)> = Vec::new();
        for (d, r) in self.ranges.iter().enumerate().take(N) {
            let v = f[d];
            if r.checked && (v < r.min || v > r.max) {
                let width = (r.max - r.min).max(1e-9);
                let by = if v < r.min { (r.min - v) / width } else { (v - r.max) / width };
                outside.push((
                    by,
                    Outside {
                        feature: r.feature.clone(),
                        meaning: MEANINGS[d].to_string(),
                        value: round(v),
                        min: r.min,
                        max: r.max,
                        median: r.median,
                    },
                ));
            }
        }
        outside.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.feature.cmp(&b.1.feature)));
        MatchResult {
            pass: outside.is_empty(),
            outside: outside.into_iter().map(|(_, o)| o).collect(),
            features: NAMES.iter().zip(f.iter()).map(|(n, v)| (n.to_string(), round(*v))).collect(),
        }
    }

    /// Indices of the `k` rows nearest vanilla's median (range-normalised), for the side-by-side preview.
    pub fn most_typical(&self, rows: &[Features], k: usize) -> Vec<usize> {
        let mut scored: Vec<(f64, usize)> = rows
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let d: f64 = self
                    .ranges
                    .iter()
                    .enumerate()
                    .filter(|(_, rg)| rg.checked)
                    .map(|(j, rg)| ((r[j] - rg.median) / (rg.max - rg.min).max(1e-9)).powi(2))
                    .sum();
                (d, i)
            })
            .collect();
        scored.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        scored.into_iter().take(k).map(|(_, i)| i).collect()
    }
}

/// One vanilla file, decoded and measured.
#[derive(Debug, Clone)]
pub struct VanillaImage {
    /// Absolute path inside the installed game.
    pub path: std::path::PathBuf,
    /// Frame number (from 1) when the image is one frame of a strip.
    pub frame: Option<u32>,
    /// SHA-256 of the file.
    pub sha256: String,
    /// Pixels.
    pub image: Rgba,
    /// Features.
    pub features: Features,
}

/// A family's vanilla art, measured, and its calibration.
#[derive(Debug, Clone)]
pub struct VanillaSet {
    /// The measured files, sorted by path.
    pub images: Vec<VanillaImage>,
    /// The fitted ranges.
    pub calibration: Calibration,
}

impl VanillaSet {
    /// The `k` most typical files, for the side-by-side preview.
    pub fn typical(&self, k: usize) -> Vec<&VanillaImage> {
        let rows: Vec<Features> = self.images.iter().map(|i| i.features).collect();
        self.calibration.most_typical(&rows, k).into_iter().map(|i| &self.images[i]).collect()
    }
}

/// Families whose vanilla art is mostly frame strips: each frame of the family's canvas size is one image.
/// Vanilla keeps only 2 single 56x84 medals but about 80 large medal frames in 56x84 strips.
pub fn uses_strips(family: Family) -> bool {
    family == Family::Medal
}

/// A family's vanilla images from the installed game: files of `sizes` (the family's vanilla sizes), and for a
/// strip family every `frame`-sized frame of a strip. Larger sets are sampled evenly by path up to [`MAX_FILES`]
/// files. Returns (path, frame number for a strip, file SHA-256, pixels), sorted by path and frame.
pub fn vanilla_images(
    game: &std::path::Path,
    family: Family,
    frame: [u32; 2],
    sizes: &[[u32; 2]],
) -> Vec<(std::path::PathBuf, Option<u32>, String, Rgba)> {
    let strips = uses_strips(family) && frame[0] > 0 && frame[1] > 0;
    let files: Vec<std::path::PathBuf> = if strips {
        crate::stylesrc::vanilla_files(game, family, &[])
            .into_iter()
            .filter(|p| crate::stylesrc::header_size(p).is_some_and(|[w, h]| h == frame[1] && w >= frame[0] && w % frame[0] == 0))
            .collect()
    } else {
        crate::stylesrc::vanilla_files(game, family, sizes)
    };
    let picked: Vec<&std::path::PathBuf> =
        if files.len() > MAX_FILES { (0..MAX_FILES).map(|i| &files[i * files.len() / MAX_FILES]).collect() } else { files.iter().collect() };
    let mut out = Vec::new();
    for p in picked {
        let Ok(bytes) = crate::fsio::read_limited(p, crate::refs::MAX_REFERENCE_BYTES) else { continue };
        let name = p.file_name().map(|n| n.to_string_lossy().to_string());
        let Ok((image, _)) = crate::pixels::decode(&bytes, name.as_deref(), crate::pixels::DecodeLimits::default()) else { continue };
        let sha = crate::hashing::sha256_hex(&bytes);
        if strips && image.width > frame[0] {
            let Ok(frames) = image.split_frames(image.width / frame[0]) else { continue };
            for (i, f) in frames.into_iter().enumerate() {
                out.push((p.clone(), Some(i as u32 + 1), sha.clone(), f));
            }
        } else {
            out.push((p.clone(), None, sha, image));
        }
    }
    out
}

/// Read and measure a family's vanilla images (see [`vanilla_images`]) and calibrate. `None` when fewer than
/// [`MIN_FILES`] images can be measured.
pub fn load_vanilla(game: &std::path::Path, family: Family, frame: [u32; 2], sizes: &[[u32; 2]]) -> Option<VanillaSet> {
    let mut images = Vec::new();
    for (path, frame_no, sha256, image) in vanilla_images(game, family, frame, sizes) {
        let Some(features) = measure(&image) else { continue };
        images.push(VanillaImage { path, frame: frame_no, sha256, image, features });
    }
    let rows: Vec<Features> = images.iter().map(|i| i.features).collect();
    let calibration = calibrate(family, &rows)?;
    Some(VanillaSet { images, calibration })
}

/// One-line summary of the features outside the range, for error messages.
pub fn describe(outside: &[Outside]) -> String {
    outside.iter().map(|o| format!("{} {} (vanilla {}..{}, median {})", o.feature, o.value, o.min, o.max, o.median)).collect::<Vec<_>>().join("; ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(w: u32, h: u32, r: f64, colour: [u8; 4]) -> Rgba {
        let mut img = Rgba::new(w, h);
        let (cx, cy) = (f64::from(w) / 2.0, f64::from(h) / 2.0);
        for y in 0..h {
            for x in 0..w {
                let (dx, dy) = (f64::from(x) + 0.5 - cx, f64::from(y) + 0.5 - cy);
                if dx * dx + dy * dy < r * r {
                    img.set_px(x, y, colour);
                }
            }
        }
        img
    }

    /// Calibration aid, run by hand: `VM_DUMP=<out.json> VM_EXTRA=<folder> cargo test -p imagegen-core
    /// dump_features -- --ignored`. Writes every family's vanilla features and the features of the PNG files in the extra folder.
    #[test]
    #[ignore]
    fn dump_features() {
        let out = std::env::var("VM_DUMP").expect("VM_DUMP");
        let game = crate::config::find_game_dir(&crate::config::Config::default(), &crate::config::ProcessEnv).expect("game");
        let set = crate::profile::ProfileSet::builtin();
        let mut fams = serde_json::Map::new();
        for f in Family::ALL.iter().copied() {
            let Some(p) = set.for_family(f) else { continue };
            let mut rows = Vec::new();
            for (path, frame, _, img) in vanilla_images(&game, f, p.canvas, &p.accepted_strips()) {
                if let Some(ft) = measure(&img) {
                    rows.push(serde_json::json!({"path": crate::fsio::display_path(&path), "frame": frame, "f": ft.to_vec()}));
                }
            }
            fams.insert(f.as_str().to_string(), serde_json::json!({"sizes": p.accepted_strips(), "rows": rows}));
        }
        let mut extra = serde_json::Map::new();
        if let Ok(dir) = std::env::var("VM_EXTRA") {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let name = e.file_name().to_string_lossy().to_string();
                if !name.ends_with(".png") || name.ends_with("_4x.png") {
                    continue;
                }
                let bytes = std::fs::read(e.path()).unwrap();
                let (img, _) = crate::pixels::decode(&bytes, Some(&name), crate::pixels::DecodeLimits::default()).unwrap();
                if let Some(ft) = measure(&img) {
                    extra.insert(name, serde_json::json!({"size": [img.width, img.height], "f": ft.to_vec()}));
                }
            }
        }
        std::fs::write(out, serde_json::to_vec(&serde_json::json!({"names": NAMES, "families": fams, "extra": extra})).unwrap()).unwrap();
    }

    #[test]
    fn a_disc_is_symmetric_convex_and_centred() {
        let f = measure(&disc(64, 64, 24.0, [150, 120, 60, 255])).unwrap();
        let get = |n: &str| f[NAMES.iter().position(|x| *x == n).unwrap()];
        assert!(get("hull_fill") > 0.9, "{}", get("hull_fill"));
        assert!(get("mirror_symmetry") > 0.95);
        assert!((get("subject_center_x") - 0.5).abs() < 0.02 && (get("subject_center_y") - 0.5).abs() < 0.02);
        assert!((get("subject_aspect") - 1.0).abs() < 0.05);
        assert_eq!(get("border_opacity"), 0.0);
        assert!(measure(&Rgba::new(8, 8)).is_none(), "an empty image has nothing to measure");
    }

    #[test]
    fn calibration_accepts_its_own_rows_and_names_the_feature_outside() {
        let rows: Vec<Features> = (0..20).map(|i| measure(&disc(64, 64, 20.0 + f64::from(i % 5), [140 + i as u8, 110, 60, 255])).unwrap()).collect();
        let cal = calibrate(Family::Focus, &rows).unwrap();
        assert!(rows.iter().all(|r| cal.check(r).pass));
        // an opaque square: a background fill vanilla never has
        let filled = measure(&Rgba::filled(64, 64, [150, 120, 60, 255])).unwrap();
        let res = cal.check(&filled);
        assert!(!res.pass);
        assert!(res.outside.iter().any(|o| o.feature == "border_opacity"), "{res:?}");
        assert!(calibrate(Family::Focus, &rows[..2]).is_none());
    }
}
