//! Intelligence agency upgrades: any subject turned into vanilla's round, muted, darkened-edge vignette.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, the 15 `agency_*.dds` upgrade icons of `gfx/interface/operatives/icons/`,
//! 68x68; measured by the tests below and by the vanilla-match calibration):
//!
//! * Every file is one disc centred at (34, 34) with a radius of 26.1 px (diameter 52, 0.765 of the canvas), opaque
//!   inside and fully clear outside, with a one-pixel anti-aliased edge (alpha 0.96 half a pixel inside the radius,
//!   0.50 on it, 0.01 half a pixel outside). There is no ring or rim.
//! * The palette is muted: the mean HSV saturation is 0.249 in the median file (0.115 to 0.402, 25th and 75th
//!   percentiles 0.201 and 0.366), the mean value 0.423 (0.239 to 0.518, quartiles 0.362 and 0.466), and gold or
//!   bronze tones cover 0.106 of the disc in the median (0.218 at the 75th percentile).
//! * The edge is darkened: relative to the mean value of the inner 60 percent of the radius, the value is 0.47 within
//!   one pixel of the edge, 0.55, 0.70 and 0.81 one, two and three pixels further in, 0.82 at four to six pixels,
//!   0.89 at six to nine and 0.99 deeper (medians over the 15 files). [`EDGE`] holds these points as shares of the
//!   radius.
//!
//! **How the art is made.** The trimmed subject is cut to its centred square and to the disc inscribed in it, with
//! a one-pixel anti-aliased edge at the source resolution, so the fitted disc gets vanilla's edge. Clear or partly
//! clear pixels inside the disc are laid over [`BACKDROP`], vanilla's median disc colour, so the disc is always
//! opaque. The colours are then brought into vanilla's range: saturation is scaled down to vanilla's median when it
//! is above it, and the value gets a gamma that moves the finished disc's mean value to between vanilla's median and 75th
//! percentile when it is outside them. The edge is darkened toward vanilla's profile: each ring is darkened only as far
//! as needed to reach vanilla's ratio, so a subject that already has a dark edge is not darkened twice. Last, when gold
//! and bronze tones still cover more of the disc than in vanilla's 75th percentile, the saturation of those hues alone
//! is lowered until they do not.
//!
//! Everything is a pure function of the pixels, deterministic on every platform.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Version of the vignette recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// Vanilla's edge darkening: (depth inside the edge as a share of the radius, value relative to the inner 60 percent
/// of the radius). The depths are the centres of the measured one-pixel rings on vanilla's 26.1 px radius.
pub const EDGE: [(f64, f64); 8] =
    [(0.019, 0.471), (0.057, 0.551), (0.096, 0.695), (0.134, 0.807), (0.192, 0.824), (0.287, 0.890), (0.345, 0.989), (0.40, 1.0)];

/// Vanilla's median disc colour (the per-channel median of the 15 files' mean colours), behind clear source pixels.
pub const BACKDROP: [u8; 3] = [110, 94, 80];

/// A round vignette fitted to one vanilla set.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VignetteStyle {
    /// Edge darkening, see [`EDGE`].
    pub edge: &'static [(f64, f64)],
    /// Colour behind clear source pixels inside the disc.
    pub backdrop: [u8; 3],
    /// Highest mean saturation kept; a more saturated disc is scaled down to it.
    pub saturation_max: f64,
    /// Accepted mean value of the finished disc; outside it a gamma moves it to the nearer end.
    pub value_range: (f64, f64),
    /// Highest share of gold and bronze tones; above it their saturation is lowered further.
    pub warm_max: f64,
}

/// Vanilla's agency upgrades: mean saturation at most vanilla's median 0.249, mean value from the median 0.423 to the
/// 75th percentile 0.466, gold and bronze tones at most the 75th percentile 0.218.
pub const AGENCY_UPGRADE: VignetteStyle =
    VignetteStyle { edge: &EDGE, backdrop: BACKDROP, saturation_max: 0.249, value_range: (0.423, 0.466), warm_max: 0.218 };

fn to_hsv(p: [u8; 4]) -> (f64, f64, f64) {
    crate::styledesc::hsv(p[0], p[1], p[2])
}

fn from_hsv(h: f64, s: f64, v: f64) -> [u8; 3] {
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

/// One disc pixel: index, depth inside the edge as a share of the radius, hue, saturation and value.
struct DiscPx {
    i: usize,
    depth: f64,
    h: f64,
    s: f64,
    v: f64,
}

/// The darkening factor per edge point: vanilla's ratio over the current one, never above 1.
fn edge_factors(edge: &[(f64, f64)], px: &[&DiscPx], value: impl Fn(&DiscPx) -> f64) -> Vec<f64> {
    let inner: Vec<f64> = px.iter().filter(|p| p.depth >= 0.4).map(|p| value(p)).collect();
    let inner_mean = if inner.is_empty() { 1.0 } else { inner.iter().sum::<f64>() / inner.len() as f64 };
    // each edge point owns the depths nearer to it than to its neighbours
    let bounds: Vec<f64> = (0..edge.len()).map(|k| if k + 1 < edge.len() { (edge[k].0 + edge[k + 1].0) / 2.0 } else { 0.4 }).collect();
    let mut sums = vec![(0.0f64, 0usize); edge.len()];
    for p in px.iter().filter(|p| p.depth < 0.4) {
        let k = bounds.iter().position(|b| p.depth < *b).unwrap_or(edge.len() - 1);
        sums[k].0 += value(p);
        sums[k].1 += 1;
    }
    edge.iter()
        .zip(&sums)
        .map(|((_, want), (s, n))| {
            if *n == 0 || inner_mean <= 1e-6 {
                return 1.0;
            }
            let now = s / *n as f64 / inner_mean;
            if now <= *want {
                1.0
            } else {
                want / now
            }
        })
        .collect()
}

/// Factor at a depth, linear between the edge points' factors.
fn factor_at(edge: &[(f64, f64)], factors: &[f64], t: f64) -> f64 {
    if t >= 0.4 {
        return 1.0;
    }
    if t <= edge[0].0 {
        return factors[0];
    }
    for k in 0..edge.len() - 1 {
        if t <= edge[k + 1].0 {
            let f = (t - edge[k].0) / (edge[k + 1].0 - edge[k].0);
            return factors[k] + (factors[k + 1] - factors[k]) * f;
        }
    }
    1.0
}

/// A gold or bronze tone, as the vanilla-match check counts it (hue 20 to 60 degrees, saturation 0.25 to 0.9, value
/// 0.35 or more), with saturation and value counted from 0.03 and 0.02 lower, so that rounding to 8-bit colour and
/// the reduction to the canvas cannot lift a tone back over the line.
fn is_warm(h: f64, s: f64, v: f64) -> bool {
    (20.0..=60.0).contains(&h) && (0.22..=0.9).contains(&s) && v >= 0.33
}

/// Extra saturation factor of a hue: `wk` for gold and bronze hues (20 to 60 degrees), 1 from 10 degrees beyond them,
/// linear between.
fn warm_weight(h: f64, wk: f64) -> f64 {
    let out = if (20.0..=60.0).contains(&h) {
        0.0
    } else if (10.0..20.0).contains(&h) {
        (20.0 - h) / 10.0
    } else if (60.0..70.0).contains(&h) {
        (h - 60.0) / 10.0
    } else {
        1.0
    };
    wk + (1.0 - wk) * out
}

/// The agency upgrade vignette, see the module notes.
pub fn round_vignette(img: &Rgba) -> Result<Rgba> {
    round_vignette_with(img, &AGENCY_UPGRADE)
}

/// A round vignette in a vanilla style.
pub fn round_vignette_with(img: &Rgba, style: &VignetteStyle) -> Result<Rgba> {
    let side = img.width.min(img.height);
    if side < 8 || img.data.chunks_exact(4).filter(|p| p[3] >= 128).count() < 16 {
        return Err(Error::Image("the picture has no subject to turn into a round vignette".into()));
    }
    let sq = img.crop((img.width - side) / 2, (img.height - side) / 2, side, side)?;
    let r = f64::from(side) / 2.0;
    let n = (side * side) as usize;
    // the disc: coverage of each pixel, and the source laid over the backdrop
    let mut cover = vec![0f64; n];
    let mut disc: Vec<DiscPx> = Vec::new();
    for y in 0..side {
        for x in 0..side {
            let i = (y * side + x) as usize;
            let d = ((f64::from(x) + 0.5 - r).powi(2) + (f64::from(y) + 0.5 - r).powi(2)).sqrt();
            let c = (r - d + 0.5).clamp(0.0, 1.0);
            if c <= 0.0 {
                continue;
            }
            cover[i] = c;
            let p = sq.px(x, y);
            let a = f64::from(p[3]) / 255.0;
            let px = [0, 1, 2].map(|k| (f64::from(p[k]) * a + f64::from(style.backdrop[k]) * (1.0 - a)).round() as u8);
            let (h, s, v) = to_hsv([px[0], px[1], px[2], 255]);
            disc.push(DiscPx { i, depth: ((r - d) / r).max(0.0), h, s, v });
        }
    }
    // saturation down to the style's highest mean
    let mean_s = disc.iter().map(|p| p.s).sum::<f64>() / disc.len() as f64;
    let k = if mean_s > style.saturation_max { style.saturation_max / mean_s } else { 1.0 };
    // the value gamma, chosen on an even sample of the disc so the finished mean value is in the style's range
    let step = ((disc.len() as f64 / 40_000.0).sqrt().floor() as usize).max(1);
    let sample: Vec<&DiscPx> =
        disc.iter().filter(|p| (p.i % side as usize).is_multiple_of(step) && (p.i / side as usize).is_multiple_of(step)).collect();
    let finished_mean = |g: f64| {
        let f = edge_factors(style.edge, &sample, |p| p.v.powf(g));
        sample.iter().map(|p| p.v.powf(g) * factor_at(style.edge, &f, p.depth)).sum::<f64>() / sample.len() as f64
    };
    let now = finished_mean(1.0);
    let want = now.clamp(style.value_range.0, style.value_range.1);
    let mut gamma = 1.0;
    if (now - want).abs() > 1e-4 {
        // the mean falls as gamma rises
        let (mut lo, mut hi) = if now < want { (0.2, 1.0) } else { (1.0, 5.0) };
        for _ in 0..40 {
            let mid = (lo + hi) / 2.0;
            if finished_mean(mid) > want {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        gamma = (lo + hi) / 2.0;
    }
    let all: Vec<&DiscPx> = disc.iter().collect();
    let factors = edge_factors(style.edge, &all, |p| p.v.powf(gamma));
    let value = |p: &DiscPx| (p.v.powf(gamma) * factor_at(style.edge, &factors, p.depth)).clamp(0.0, 1.0);
    // gold and bronze tones down to the style's highest share: their saturation is lowered further
    let warm_share = |wk: f64| {
        let warm = sample.iter().filter(|p| is_warm(p.h, p.s * k * warm_weight(p.h, wk), value(p))).count();
        warm as f64 / sample.len() as f64
    };
    let mut wk = 1.0;
    if warm_share(1.0) > style.warm_max {
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..30 {
            let mid = (lo + hi) / 2.0;
            if warm_share(mid) > style.warm_max {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        wk = lo;
    }
    let mut out = Rgba::new(side, side);
    for p in &disc {
        let c = from_hsv(p.h, p.s * k * warm_weight(p.h, wk), value(p));
        let x = p.i as u32 % side;
        let y = p.i as u32 / side;
        out.set_px(x, y, [c[0], c[1], c[2], (cover[p.i] * 255.0).round() as u8]);
    }
    Ok(out)
}

/// What the tests compare on a finished 68x68 icon: the edge darkening (value relative to the inner 60 percent of the
/// radius, in one-pixel rings from the edge: 0-1, 1-2, 2-3, 3-4, 4-6, 6-9 and 9 or more pixels deep), and the alpha
/// half a pixel inside, on and half a pixel outside the radius. The disc's centre and radius come from the alpha.
#[derive(Debug, Clone, PartialEq)]
pub struct DiscStats {
    /// Centre x and y and radius, in pixels.
    pub centre_radius: [f64; 3],
    /// Value relative to the inner 60 percent of the radius, by depth ring.
    pub edge: [f64; 7],
    /// Mean alpha half a pixel inside, on and half a pixel outside the radius.
    pub alpha: [f64; 3],
}

/// Measure a finished vignette, see [`DiscStats`].
pub fn disc_stats(img: &Rgba) -> DiscStats {
    let (w, h) = (img.width, img.height);
    let (mut cx, mut cy, mut a) = (0.0, 0.0, 0.0);
    for y in 0..h {
        for x in 0..w {
            let al = f64::from(img.alpha(x, y)) / 255.0;
            cx += (f64::from(x) + 0.5) * al;
            cy += (f64::from(y) + 0.5) * al;
            a += al;
        }
    }
    if a <= 0.0 {
        return DiscStats { centre_radius: [0.0; 3], edge: [0.0; 7], alpha: [0.0; 3] };
    }
    cx /= a;
    cy /= a;
    let r = (a / std::f64::consts::PI).sqrt();
    let rings = [0.0, 1.0, 2.0, 3.0, 4.0, 6.0, 9.0, f64::INFINITY];
    let mut vv = [(0.0f64, 0.0f64); 7];
    let mut al = [(0.0f64, 0.0f64); 3];
    let mut inner = (0.0, 0.0);
    for y in 0..h {
        for x in 0..w {
            let d = ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - cy).powi(2)).sqrt();
            let k = ((d - r + 0.75) / 0.5).floor();
            if (0.0..3.0).contains(&k) {
                al[k as usize].0 += f64::from(img.alpha(x, y)) / 255.0;
                al[k as usize].1 += 1.0;
            }
            let p = img.px(x, y);
            if p[3] < 200 {
                continue;
            }
            let (_, _, v) = to_hsv(p);
            if d <= 0.6 * r {
                inner.0 += v;
                inner.1 += 1.0;
            }
            let depth = r - d;
            if let Some(b) = (0..7).find(|b| depth >= rings[*b] && depth < rings[b + 1]) {
                vv[b].0 += v;
                vv[b].1 += 1.0;
            }
        }
    }
    let im = if inner.1 > 0.0 { inner.0 / inner.1 } else { 1.0 };
    DiscStats {
        centre_radius: [cx, cy, r],
        edge: vv.map(|(s, n)| if n > 0.0 && im > 0.0 { s / n / im } else { 0.0 }),
        alpha: al.map(|(s, n)| if n > 0.0 { s / n } else { 0.0 }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::process::{apply, Op};
    use crate::vanillamatch::{measure, NAMES};

    /// A saturated, warm painted medallion like the 0.3.2 gallery's agency upgrades, exaggerated: an orange-brown disc
    /// on transparency with its own shading and a dark rim, at a model's source size.
    fn warm_medallion(side: u32) -> Rgba {
        let mut img = Rgba::new(side, side);
        let c = f64::from(side) / 2.0;
        let r = c - 8.0;
        for y in 0..side {
            for x in 0..side {
                let (dx, dy) = (f64::from(x) + 0.5 - c, f64::from(y) + 0.5 - c);
                let d = dx.hypot(dy);
                if d > r {
                    continue;
                }
                // a lamp-lit desk: bands of light and shade, darker toward the rim
                let band = ((f64::from(x) / f64::from(side) * 9.0).sin() * 0.5 + 0.5) * 0.35;
                let v = (0.25 + band + 0.2 * (1.0 - d / r)).min(1.0) * if d > r - 6.0 { 0.35 } else { 1.0 };
                let h = 28.0 + 10.0 * (f64::from(y) / f64::from(side));
                let rgb = from_hsv(h, 0.72, v);
                img.set_px(x, y, [rgb[0], rgb[1], rgb[2], 255]);
            }
        }
        img
    }

    fn feature(img: &Rgba, name: &str) -> f64 {
        measure(img).unwrap()[NAMES.iter().position(|n| *n == name).unwrap()]
    }

    fn finish(src: &Rgba) -> Rgba {
        let t = apply(src, &Op::TrimToSubject { min_alpha: 16 }).unwrap();
        let v = round_vignette(&t).unwrap();
        apply(&v, &Op::FitSubject { width: 68, height: 68, margin: 8, min_alpha: 16 }).unwrap()
    }

    /// The warm medallion comes out in vanilla's palette: mean saturation at most vanilla's median 0.249, mean value
    /// between vanilla's median 0.423 and 75th percentile 0.466, gold tones at most vanilla's 75th percentile 0.218
    /// (each within what the reduction from the source size to 68x68 moves), on vanilla's disc and edge.
    #[test]
    fn a_saturated_warm_medallion_comes_out_in_vanillas_palette_and_disc() {
        let src = warm_medallion(400);
        let trimmed = apply(&src, &Op::TrimToSubject { min_alpha: 16 }).unwrap();
        let fit_only = apply(&trimmed, &Op::FitSubject { width: 68, height: 68, margin: 8, min_alpha: 16 }).unwrap();
        let out = finish(&src);
        let (s0, v0, w0) = (feature(&fit_only, "saturation_mean"), feature(&fit_only, "value_mean"), feature(&fit_only, "warm_metal_share"));
        let (s1, v1, w1) = (feature(&out, "saturation_mean"), feature(&out, "value_mean"), feature(&out, "warm_metal_share"));
        eprintln!("warm medallion: saturation {s0:.3} -> {s1:.3}, value {v0:.3} -> {v1:.3}, gold tones {w0:.3} -> {w1:.3}");
        assert!(s0 > 0.6 && w0 > 0.5, "the source is saturated and warm: {s0} {w0}");
        assert!(s1 <= 0.249 + 0.02, "saturation {s1}");
        assert!((0.423 - 0.02..=0.466 + 0.02).contains(&v1), "value {v1}");
        assert!(w1 <= 0.218 + 0.03, "gold tones {w1}");
        let d = disc_stats(&out);
        assert!((d.centre_radius[0] - 34.0).abs() < 0.3 && (d.centre_radius[1] - 34.0).abs() < 0.3, "{d:?}");
        assert!((d.centre_radius[2] - 26.1).abs() < 0.5, "radius {d:?}");
        assert!(d.alpha[0] > 0.85 && (d.alpha[1] - 0.5).abs() < 0.12 && d.alpha[2] < 0.1, "a one-pixel edge: {d:?}");
        assert!(d.edge[0] < 0.6 && d.edge[6] > 0.9, "the edge is darkened: {d:?}");
        assert_eq!(out, finish(&src), "deterministic");
    }

    /// A flat source gets vanilla's whole edge profile. An opaque square picture and a loose object on transparency
    /// both become a full disc.
    #[test]
    fn any_source_becomes_a_full_disc_with_vanillas_edge() {
        let flat = Rgba::filled(300, 300, [120, 130, 125, 255]);
        let out = finish(&flat);
        let d = disc_stats(&out);
        for (k, want) in [0.471, 0.551, 0.695, 0.807, 0.824, 0.890, 0.989].iter().enumerate() {
            assert!((d.edge[k] - want).abs() < 0.08, "ring {k}: {:?}", d.edge);
        }
        assert!((feature(&out, "ellipse_fill") - 1.0).abs() < 0.02 && feature(&out, "border_opacity") == 0.0);
        // a loose object, trimmed to itself, fills the disc over the backdrop
        let mut loose = Rgba::new(300, 300);
        for y in 100..200 {
            for x in 60..240 {
                loose.set_px(x, y, [200, 200, 200, if y < 110 { 120 } else { 255 }]);
            }
        }
        let out = finish(&loose);
        assert!(feature(&out, "hull_fill") > 0.95 && (feature(&out, "subject_aspect") - 1.0).abs() < 0.05, "a full disc");
        assert!(round_vignette(&Rgba::new(40, 40)).is_err(), "nothing to cut");
    }

    fn vanilla() -> Vec<Rgba> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let dir = g.join("gfx/interface/operatives/icons");
        let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        files
            .iter()
            .filter(|p| p.file_name().unwrap().to_string_lossy().starts_with("agency_"))
            .filter_map(|p| crate::dds::decode(&std::fs::read(p).ok()?).ok())
            .filter(|i| (i.width, i.height) == (68, 68))
            .collect()
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        let n = v.len();
        if n % 2 == 1 {
            v[n / 2]
        } else {
            (v[n / 2 - 1] + v[n / 2]) / 2.0
        }
    }

    /// The constants are vanilla's: the disc's centre, radius and edge, the edge darkening, the backdrop colour and the
    /// palette medians. Measured on HOI4 1.19.4.1 (15 files): centre (34.0, 34.0), radius 26.1, alpha 0.96, 0.50 and
    /// 0.01 across the edge, darkening 0.47, 0.55, 0.70, 0.81, 0.82, 0.89 and 0.99, mean colour (110, 94, 80),
    /// saturation 0.249, value 0.423.
    #[test]
    fn the_constants_are_vanillas_medians() {
        let files = vanilla();
        if files.is_empty() {
            eprintln!("SKIPPED agency vignette vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        assert_eq!(files.len(), 15, "vanilla 1.19.4.1 has 15 agency upgrades");
        let stats: Vec<DiscStats> = files.iter().map(disc_stats).collect();
        for (k, want) in [34.0, 34.0, 26.1].iter().enumerate() {
            let m = median(stats.iter().map(|s| s.centre_radius[k]).collect());
            assert!((m - want).abs() < 0.1, "centre and radius {k}: {m}");
        }
        for (k, want) in [0.96, 0.50, 0.01].iter().enumerate() {
            let m = median(stats.iter().map(|s| s.alpha[k]).collect());
            assert!((m - want).abs() < 0.02, "alpha {k}: {m}");
        }
        for (k, point) in EDGE.iter().take(7).enumerate() {
            let m = median(stats.iter().map(|s| s.edge[k]).collect());
            assert!((m - point.1).abs() < 0.02, "edge ring {k}: {m} vs {}", point.1);
        }
        let means: Vec<[f64; 3]> = files
            .iter()
            .map(|f| {
                let (mut m, mut n) = ([0.0; 3], 0.0);
                for p in f.data.chunks_exact(4).filter(|p| p[3] >= 200) {
                    for (c, slot) in m.iter_mut().enumerate() {
                        *slot += f64::from(p[c]);
                    }
                    n += 1.0;
                }
                m.map(|s| s / n)
            })
            .collect();
        for (c, want) in BACKDROP.iter().enumerate() {
            let m = median(means.iter().map(|x| x[c]).collect());
            assert!((m - f64::from(*want)).abs() < 1.5, "backdrop channel {c}: {m}");
        }
        let f = |name: &str| median(files.iter().map(|i| feature(i, name)).collect());
        assert!((f("saturation_mean") - AGENCY_UPGRADE.saturation_max).abs() < 0.002, "{}", f("saturation_mean"));
        assert!((f("value_mean") - AGENCY_UPGRADE.value_range.0).abs() < 0.002, "{}", f("value_mean"));
    }
}
