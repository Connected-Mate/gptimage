//! Special project icons: any generated image turned into vanilla's pale line drawing.
//!
//! **What vanilla shows** (HOI4 1.19.4.1, all 55 files of `gfx/interface/special_project/project_icons/`, 161x98,
//! measured by the tests below):
//!
//! * Only lines: every visible pixel is pure white (255, 255, 255). The lines are pale because they are partly
//!   transparent, not because they are grey: the median alpha of a visible pixel is 90 and the 90th percentile 163.
//!   No pixel is filled or shaded, fewer than 1 percent are opaque.
//! * The lines are thin: about one pixel, antialiased (the median of the shorter run through a pixel of alpha 128
//!   and up is 1 px, its mean 1.56 px).
//! * The drawing is sparse: about 10.5 percent of the canvas is visible (alpha 16 and up) and the summed alpha is
//!   3.8 percent of the canvas (10th to 90th percentile 2.4 to 6.3 percent).
//! * It sits centred in a box of about 86 by 74 percent of the canvas: wide subjects span 83 to 89 percent of the
//!   width, tall ones 70 to 74 percent of the height, so the margins are about 11 px at the sides and 13 px at the
//!   top and bottom.
//!
//! **How the art is made.** The model is asked for a clean line drawing or plain silhouette, but whatever it returns
//! (a shaded grey render, a painted picture, pale lines, blue paper with white lines) goes through the same steps:
//!
//! 1. The subject is read from real alpha, or, on an opaque image, from its contrast with the flat border colour
//!    ([`crate::recipes::unit_art::silhouette_coverage`]).
//! 2. It is fitted into vanilla's box at [`LineStyle::supersample`] times the final size.
//! 3. A subject that is already thin lines keeps its own lines. Anything else is reduced to its edges: the
//!    gradient of its alpha (the outline) and of its luminance inside it (the inner details), after a small blur
//!    that removes texture noise.
//! 4. The strokes are reduced to the final size by area averaging, which gives vanilla's antialiased one-pixel
//!    lines, then mapped to alpha. When the drawing is denser than vanilla's 90th percentile, the threshold rises
//!    until it reaches vanilla's median, so a busy render loses its faint texture lines first.
//! 5. Every visible pixel is pure white.
//!
//! Everything is a pure function of the pixels and the parameters, deterministic on every platform.

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::unit_art::{area_resample, gaussian, silhouette_coverage, Plane};
use crate::resize::resize_lanczos3;

/// Version of the line art recipe (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// The constants of the line drawing, fitted to vanilla's special project icons.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStyle {
    /// The box the drawing is fitted into, as shares of the canvas width and height.
    pub fit_box: [f64; 2],
    /// Edges are found at this many times the final size, then averaged down.
    pub supersample: u32,
    /// Blur before the edges are taken, in supersampled pixels.
    pub edge_sigma: f64,
    /// How much a luminance edge inside the subject counts against an outline edge of the same step.
    pub inner_gain: f64,
    /// How an edge's strength (after averaging to the final size) becomes alpha: where alpha starts to rise, where
    /// it stops, and the alpha there. Linear in between.
    pub edge_ramp: [f64; 3],
    /// The same for a subject that is line art already, whose strength is its own coverage.
    pub line_ramp: [f64; 3],
    /// Line colour. Vanilla's is pure white.
    pub rgb: [u8; 3],
    /// Summed alpha as a share of the canvas: vanilla's 10th percentile, median and 90th percentile.
    pub ink: [f64; 3],
    /// A subject counts as line art already when no more than this share of it survives an erosion of
    /// `thin_px` final pixels.
    pub line_like_share: f64,
    /// Half the widest line, in final pixels, that still counts as a line.
    pub thin_px: f64,
}

/// Vanilla's special project icon look.
pub const STYLE: LineStyle = LineStyle {
    fit_box: [0.86, 0.74],
    supersample: 4,
    edge_sigma: 1.2,
    inner_gain: 2.5,
    edge_ramp: [0.14, 0.6, 205.0],
    line_ramp: [0.08, 0.85, 255.0],
    rgb: [255, 255, 255],
    ink: [0.024, 0.038, 0.063],
    line_like_share: 0.12,
    thin_px: 1.5,
};

/// The coverage and colour of the subject, cropped to it.
fn subject(img: &Rgba) -> Result<Rgba> {
    let cov = silhouette_coverage(img)?;
    let b = cov.bbox(16.0 / 255.0).ok_or_else(|| Error::Image("the image has no visible subject to draw".into()))?;
    let mut out = Rgba::new((b[2] - b[0]) as u32, (b[3] - b[1]) as u32);
    for y in b[1]..b[3] {
        for x in b[0]..b[2] {
            let p = img.px(x as u32, y as u32);
            let a = (f64::from(cov.v[y * cov.w + x]) * 255.0).round().clamp(0.0, 255.0) as u8;
            out.set_px((x - b[0]) as u32, (y - b[1]) as u32, [p[0], p[1], p[2], a]);
        }
    }
    Ok(out)
}

/// The enlargement the fit applies to `img` for a `width` x `height` canvas (above 1 enlarges).
pub fn fit_scale(img: &Rgba, width: u32, height: u32) -> Option<f64> {
    let s = subject(img).ok()?;
    let (bw, bh) = (f64::from(width) * STYLE.fit_box[0], f64::from(height) * STYLE.fit_box[1]);
    Some((bw / f64::from(s.width)).min(bh / f64::from(s.height)))
}

fn erode(mask: &[bool], w: usize, h: usize, n: u32) -> Vec<bool> {
    let mut cur = mask.to_vec();
    for _ in 0..n {
        let mut next = vec![false; cur.len()];
        for y in 1..h.saturating_sub(1) {
            for x in 1..w.saturating_sub(1) {
                let i = y * w + x;
                next[i] = cur[i] && cur[i - 1] && cur[i + 1] && cur[i - w] && cur[i + w];
            }
        }
        cur = next;
    }
    cur
}

/// Central-difference gradient magnitude of a plane.
fn gradient(p: &Plane) -> Plane {
    let mut out = Plane::new(p.w, p.h);
    for y in 0..p.h {
        for x in 0..p.w {
            let at = |xx: usize, yy: usize| f64::from(p.v[yy * p.w + xx]);
            let gx = (at((x + 1).min(p.w - 1), y) - at(x.saturating_sub(1), y)) / 2.0;
            let gy = (at(x, (y + 1).min(p.h - 1)) - at(x, y.saturating_sub(1))) / 2.0;
            out.v[y * p.w + x] = gx.hypot(gy) as f32;
        }
    }
    out
}

/// The line drawing of `img` on a transparent `width` x `height` canvas, in vanilla's style.
pub fn line_art(img: &Rgba, width: u32, height: u32) -> Result<Rgba> {
    line_art_with(img, width, height, &STYLE)
}

/// [`line_art`] with explicit constants.
pub fn line_art_with(img: &Rgba, width: u32, height: u32, style: &LineStyle) -> Result<Rgba> {
    if width < 8 || height < 8 {
        return Err(Error::Invalid(format!("a {width}x{height} canvas is too small for a line drawing")));
    }
    let src = subject(img)?;
    let k = style.supersample.max(1);
    let (sw, sh) = (width * k, height * k);
    let (bw, bh) = (f64::from(sw) * style.fit_box[0], f64::from(sh) * style.fit_box[1]);
    let scale = (bw / f64::from(src.width)).min(bh / f64::from(src.height));
    let tw = ((f64::from(src.width) * scale).round() as u32).clamp(1, sw);
    let th = ((f64::from(src.height) * scale).round() as u32).clamp(1, sh);
    let fitted = resize_lanczos3(&src, tw, th)?;
    let mut big = Rgba::new(sw, sh);
    big.paste(&fitted, i64::from((sw - tw) / 2), i64::from((sh - th) / 2));

    let (w, h) = (sw as usize, sh as usize);
    let mut alpha = Plane::new(w, h);
    let mut lum = Plane::new(w, h);
    for (i, p) in big.data.chunks_exact(4).enumerate() {
        let a = f64::from(p[3]) / 255.0;
        alpha.v[i] = a as f32;
        let l = (0.299 * f64::from(p[0]) + 0.587 * f64::from(p[1]) + 0.114 * f64::from(p[2])) / 255.0;
        lum.v[i] = (l * a) as f32;
    }

    // a subject that is thin lines already keeps them; anything with a body is reduced to its edges
    let solid: Vec<bool> = alpha.v.iter().map(|a| *a >= 0.25).collect();
    let solid_n = solid.iter().filter(|s| **s).count();
    let thick_n = erode(&solid, w, h, (style.thin_px * f64::from(k)).round() as u32).iter().filter(|s| **s).count();
    let line_like = solid_n > 0 && (thick_n as f64) <= style.line_like_share * solid_n as f64;
    let stroke = if line_like {
        alpha
    } else {
        let norm = style.edge_sigma * (2.0 * std::f64::consts::PI).sqrt();
        let a_blur = gaussian(&alpha, style.edge_sigma);
        let outline = gradient(&a_blur);
        let inner = gradient(&gaussian(&lum, style.edge_sigma));
        let mut e = Plane::new(w, h);
        for i in 0..w * h {
            let o = f64::from(outline.v[i]) * norm;
            let d = f64::from(inner.v[i]) * norm * style.inner_gain * f64::from(a_blur.v[i]).min(1.0);
            e.v[i] = o.max(d).min(1.0) as f32;
        }
        e
    };
    let small = area_resample(&stroke, width as usize, height as usize);

    let n = (width * height) as f64;
    let ramp = if line_like { style.line_ramp } else { style.edge_ramp };
    let render = |shift: f64| -> (Vec<f64>, f64) {
        let (lo, hi) = (ramp[0] + shift, ramp[1] + shift);
        let a: Vec<f64> = small.v.iter().map(|s| (((f64::from(*s) - lo) / (hi - lo)).clamp(0.0, 1.0) * ramp[2]).round()).collect();
        let ink = a.iter().sum::<f64>() / 255.0 / n;
        (a, ink)
    };
    let (mut a, ink) = render(0.0);
    if ink > style.ink[2] {
        // too dense for vanilla: raise the threshold until the drawing is as dense as vanilla's median
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if render(mid).1 > style.ink[1] {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        a = render(hi).0;
    } else if ink < style.ink[0] {
        // sparser than vanilla: let fainter strokes in, never below half the ramp's start
        let floor = -0.5 * ramp[0];
        let (mut lo, mut hi) = (floor, 0.0);
        if render(floor).1 <= style.ink[0] {
            a = render(floor).0;
        } else {
            for _ in 0..24 {
                let mid = (lo + hi) / 2.0;
                if render(mid).1 >= style.ink[0] {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            a = render(lo).0;
        }
    }

    let mut out = Rgba::new(width, height);
    for (i, v) in a.iter().enumerate() {
        let al = v.clamp(0.0, 255.0) as u8;
        if al > 0 {
            out.data[i * 4..i * 4 + 4].copy_from_slice(&[style.rgb[0], style.rgb[1], style.rgb[2], al]);
        }
    }
    if out.alpha_bbox(16).is_none() {
        return Err(Error::Image("no line of the subject is strong enough to draw: the image is blank or too faint".into()));
    }
    Ok(out)
}

/// The largest filled area of a vanilla special project icon, in pixels: the median and the largest over the 55
/// vanilla files (HOI4 1.19.4.1), measured by [`largest_filled_area`].
pub const VANILLA_FILLED_AREA: [usize; 2] = [4, 171];

/// A drawing whose largest filled area is larger than this is not line art (`GFX-FILLED-INTERIOR`): half again
/// vanilla's largest. The gallery's filled rocket (hoi4-imagegen 0.2.0) has 917.
pub const FILLED_AREA_LIMIT: usize = 250;

/// The largest filled area of a drawing: the pixels of alpha 64 and up that survive two erosions with the
/// 4-neighbour cross (outside the canvas counts as clear), in their largest 4-connected group. A line of vanilla's
/// width (one pixel, antialiased) never survives; a filled body does.
pub fn largest_filled_area(img: &Rgba) -> usize {
    let (w, h) = (img.width as usize, img.height as usize);
    let ink: Vec<bool> = img.data.chunks_exact(4).map(|p| p[3] >= 64).collect();
    let mut solid = ink;
    for _ in 0..2 {
        let prev = solid.clone();
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                solid[i] = prev[i] && x > 0 && x + 1 < w && y > 0 && y + 1 < h && prev[i - 1] && prev[i + 1] && prev[i - w] && prev[i + w];
            }
        }
    }
    let mut seen = vec![false; w * h];
    let mut best = 0;
    for start in 0..w * h {
        if !solid[start] || seen[start] {
            continue;
        }
        let (mut n, mut stack) = (0, vec![start]);
        seen[start] = true;
        while let Some(i) = stack.pop() {
            n += 1;
            let (x, y) = (i % w, i / w);
            let mut push = |j: usize| {
                if solid[j] && !seen[j] {
                    seen[j] = true;
                    stack.push(j);
                }
            };
            if x > 0 {
                push(i - 1);
            }
            if x + 1 < w {
                push(i + 1);
            }
            if y > 0 {
                push(i - w);
            }
            if y + 1 < h {
                push(i + w);
            }
        }
        best = best.max(n);
    }
    best
}

/// What the measurement tests compare: summed alpha share, visible share, median and 90th percentile alpha of a
/// visible pixel, mean line width, the drawing's box as shares of the canvas, and the share of visible pixels that are
/// not pure white.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LineStats {
    /// Summed alpha over the canvas area.
    pub ink: f64,
    /// Share of pixels with alpha 16 and up.
    pub visible: f64,
    /// Median alpha of a visible pixel.
    pub alpha_median: f64,
    /// 90th percentile alpha of a visible pixel.
    pub alpha_p90: f64,
    /// Mean of the shorter run (horizontal or vertical) through each pixel of alpha 128 and up.
    pub line_width: f64,
    /// Width and height of the visible box as shares of the canvas.
    pub span: [f64; 2],
    /// Share of visible pixels whose colour is not (255, 255, 255).
    pub off_white: f64,
}

/// Measure a line drawing the way the vanilla measurement does.
pub fn line_stats(img: &Rgba) -> LineStats {
    let (w, h) = (img.width as usize, img.height as usize);
    let n = (w * h) as f64;
    let alphas: Vec<u8> = img.data.chunks_exact(4).map(|p| p[3]).collect();
    let ink = alphas.iter().map(|a| f64::from(*a)).sum::<f64>() / 255.0 / n;
    let mut vis: Vec<f64> = alphas.iter().filter(|a| **a >= 16).map(|a| f64::from(*a)).collect();
    vis.sort_by(f64::total_cmp);
    let pct = |q: f64| if vis.is_empty() { 0.0 } else { vis[((vis.len() - 1) as f64 * q).round() as usize] };
    let off_white = img.data.chunks_exact(4).filter(|p| p[3] >= 16 && (p[0], p[1], p[2]) != (255, 255, 255)).count() as f64 / vis.len().max(1) as f64;
    let strong: Vec<bool> = alphas.iter().map(|a| *a >= 128).collect();
    let mut hr = vec![0usize; w * h];
    let mut vr = vec![0usize; w * h];
    for y in 0..h {
        let mut x = 0;
        while x < w {
            if strong[y * w + x] {
                let s = x;
                while x < w && strong[y * w + x] {
                    x += 1;
                }
                for i in s..x {
                    hr[y * w + i] = x - s;
                }
            } else {
                x += 1;
            }
        }
    }
    for x in 0..w {
        let mut y = 0;
        while y < h {
            if strong[y * w + x] {
                let s = y;
                while y < h && strong[y * w + x] {
                    y += 1;
                }
                for i in s..y {
                    vr[i * w + x] = y - s;
                }
            } else {
                y += 1;
            }
        }
    }
    let runs: Vec<f64> = (0..w * h).filter(|i| strong[*i]).map(|i| hr[i].min(vr[i]) as f64).collect();
    let line_width = if runs.is_empty() { 0.0 } else { runs.iter().sum::<f64>() / runs.len() as f64 };
    let span = img.alpha_bbox(16).map_or([0.0, 0.0], |b| [f64::from(b[2] - b[0]) / w as f64, f64::from(b[3] - b[1]) / h as f64]);
    LineStats { ink, visible: vis.len() as f64 / n, alpha_median: pct(0.5), alpha_p90: pct(0.9), line_width, span, off_white }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shaded grey rocket on a gantry, as a model draws it when it ignores "line art": filled bodies, a vertical
    /// gradient, darker panel lines and a little noise, on real transparency.
    fn shaded_render(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        let mut seed = 0x9e37_79b9_u32;
        for y in 0..h {
            for x in 0..w {
                let (u, v) = (f64::from(x) / f64::from(w), f64::from(y) / f64::from(h));
                let body = (0.44..0.56).contains(&u) && (0.12..0.9).contains(&v);
                let nose = (0.04..0.12).contains(&v) && (u - 0.5).abs() < (v - 0.04) * 0.75;
                let fin = (0.78..0.9).contains(&v) && (u - 0.5).abs() < 0.06 + (v - 0.78) * 0.9;
                let gantry = (0.3..0.36).contains(&u) && (0.1..0.92).contains(&v);
                let base = (0.25..0.75).contains(&u) && (0.9..0.95).contains(&v);
                if body || nose || fin || gantry || base {
                    seed ^= seed << 13;
                    seed ^= seed >> 17;
                    seed ^= seed << 5;
                    let noise = f64::from(seed % 9) - 4.0;
                    let panel = body && ((v * 20.0).fract() < 0.04);
                    let l = if panel { 70.0 } else { 190.0 - 90.0 * v + noise };
                    let g = l.clamp(0.0, 255.0) as u8;
                    img.set_px(x, y, [g, g, g, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn a_shaded_render_becomes_thin_white_lines_in_vanillas_box() {
        let img = shaded_render(600, 900);
        let out = line_art(&img, 161, 98).unwrap();
        assert_eq!((out.width, out.height), (161, 98));
        let s = line_stats(&out);
        assert_eq!(s.off_white, 0.0, "every visible pixel is pure white");
        assert!(s.line_width < 2.2, "thin lines: {s:?}");
        assert!(s.ink >= STYLE.ink[0] * 0.5 && s.ink <= STYLE.ink[2], "{s:?}");
        // the inside of the rocket body is not filled: only its outline and panel lines are drawn
        let b = out.alpha_bbox(16).unwrap();
        assert!(b[3] - b[1] >= 70 && b[3] - b[1] <= 74, "a tall subject fills vanilla's box height: {b:?}");
        let (cx, cy) = (f64::from(b[0] + b[2]) / 2.0, f64::from(b[1] + b[3]) / 2.0);
        assert!((cx - 80.5).abs() <= 6.0 && (cy - 49.0).abs() <= 1.5, "centred: {cx},{cy}");
        let filled = out.data.chunks_exact(4).filter(|p| p[3] >= 250).count() as f64 / (161.0 * 98.0);
        assert!(filled < 0.01, "no filled areas: {filled}");
        // deterministic
        assert_eq!(out, line_art(&img, 161, 98).unwrap());
    }

    #[test]
    fn an_opaque_picture_on_a_flat_background_and_a_finished_line_drawing_both_work() {
        // a dark shape on flat blue paper
        let mut paper = Rgba::filled(800, 500, [30, 60, 120, 255]);
        for y in 150..350 {
            for x in 100..700 {
                paper.set_px(x, y, [200, 210, 220, 255]);
            }
        }
        let out = line_art(&paper, 161, 98).unwrap();
        let s = line_stats(&out);
        assert!(s.span[0] > 0.83 && s.span[0] < 0.89, "a wide subject spans vanilla's box width: {s:?}");
        assert!(out.alpha(80, 49) == 0, "the box's inside stays empty");
        // pale lines on transparency are kept as lines, not doubled into two edges each
        let mut lines = Rgba::new(640, 392);
        for x in 40..600 {
            for t in 0..4 {
                lines.set_px(x, 100 + t, [240, 240, 240, 200]);
                lines.set_px(x, 300 + t, [240, 240, 240, 200]);
            }
        }
        for y in 100..304 {
            for t in 0..4 {
                lines.set_px(40 + t, y, [240, 240, 240, 200]);
                lines.set_px(596 + t, y, [240, 240, 240, 200]);
            }
        }
        let out = line_art(&lines, 161, 98).unwrap();
        let row: Vec<u8> = (0..98).map(|y| out.alpha(80, y)).collect();
        let peaks = row.windows(3).filter(|w| w[1] >= 16 && w[1] >= w[0] && w[1] > w[2]).count();
        assert_eq!(peaks, 2, "one stroke per drawn line: {row:?}");
    }

    /// The gallery's approved special project icon (hoi4-imagegen 0.2.0, before the line art recipe) is a filled grey
    /// rocket: validation refuses it with an error, and the recipe's drawing of it passes.
    #[test]
    fn a_filled_drawing_cannot_pass_validation_and_the_redrawn_one_does() {
        let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/gallery/rocket_programme-v1-p1.png");
        let (filled, _) = crate::pixels::decode(&std::fs::read(p).unwrap(), None, Default::default()).unwrap();
        let profile = crate::profile::ProfileSet::builtin().for_family(crate::family::Family::SpecialProjectIcon).unwrap().clone();
        assert!(profile.imagegen.checks.iter().any(|c| c == "line_art"));
        let area = largest_filled_area(&filled);
        eprintln!("gallery rocket: largest filled area {area} px");
        assert!(area > FILLED_AREA_LIMIT);
        let report = crate::validate::validate_image(&filled, &profile, &profile.imagegen.checks, &Default::default());
        let hit = report.findings.iter().find(|f| f.code == crate::validate::FILLED_CODE).expect("a filled-interior finding");
        assert_eq!(hit.level, crate::validate::Level::Error);
        assert_eq!(report.status, crate::validate::Status::Fail);
        let drawn = line_art(&filled, 161, 98).unwrap();
        let after = largest_filled_area(&drawn);
        eprintln!("gallery rocket redrawn: largest filled area {after} px, {:?}", line_stats(&drawn));
        let report = crate::validate::validate_image(&drawn, &profile, &profile.imagegen.checks, &Default::default());
        assert!(report.findings.iter().all(|f| f.level != crate::validate::Level::Error), "{:?}", report.findings);
        // a shaded render redrawn has no filled area either
        assert!(largest_filled_area(&line_art(&shaded_render(600, 900), 161, 98).unwrap()) <= VANILLA_FILLED_AREA[1]);
    }

    #[test]
    fn blank_or_tiny_inputs_are_refused() {
        assert!(line_art(&Rgba::new(100, 100), 161, 98).is_err());
        assert!(line_art(&shaded_render(100, 100), 4, 4).is_err());
    }

    /// Redraw the PNG files named in `HOI4_LINE_ART_DUMP` (separated by `;`), and with the game installed a sheet of
    /// vanilla drawings beside the recipe's drawings of their shaded renders, into `HOI4_LINE_ART_OUT`. For looking
    /// at, not a check.
    #[test]
    fn dump_line_art_when_asked() {
        let (Ok(files), Ok(out)) = (std::env::var("HOI4_LINE_ART_DUMP"), std::env::var("HOI4_LINE_ART_OUT")) else { return };
        let out = std::path::PathBuf::from(out);
        let on_dark = |img: &Rgba| {
            let mut bg = Rgba::filled(img.width, img.height, [38, 40, 46, 255]);
            bg.paste_over(img, 0, 0);
            bg
        };
        for f in files.split(';').filter(|f| !f.is_empty()) {
            let bytes = std::fs::read(f).unwrap();
            let (img, _) = crate::pixels::decode(&bytes, Some(f), Default::default()).unwrap();
            let drawn = line_art(&img, 161, 98).unwrap();
            let name = std::path::Path::new(f).file_stem().unwrap().to_string_lossy().to_string();
            eprintln!("{name}: {:?}", line_stats(&drawn));
            std::fs::write(out.join(format!("{name}_line_art.png")), crate::pixels::encode_png(&on_dark(&drawn)).unwrap()).unwrap();
        }
        let v = vanilla();
        if v.is_empty() {
            return;
        }
        let mut sheet = Rgba::filled(161 * 3, 98 * 12, [38, 40, 46, 255]);
        for (i, (_, f)) in v.iter().take(12).enumerate() {
            let render = as_render(f);
            let small_render = resize_lanczos3(&render, 161, 98).unwrap();
            let y = i64::from(i as u32 * 98);
            sheet.paste_over(f, 0, y);
            sheet.paste_over(&small_render, 161, y);
            sheet.paste_over(&line_art(&render, 161, 98).unwrap(), 322, y);
        }
        std::fs::write(out.join("vanilla_render_redrawn.png"), crate::pixels::encode_png(&sheet).unwrap()).unwrap();
    }

    // ------------------------------------------------------------ against the installed game

    fn vanilla() -> Vec<(String, Rgba)> {
        let g = std::env::var("HOI4_GAME_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| "C:/Program Files (x86)/Steam/steamapps/common/Hearts of Iron IV".into());
        let dir = g.join("gfx/interface/special_project/project_icons");
        let mut files: Vec<_> = std::fs::read_dir(&dir).map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).collect()).unwrap_or_default();
        files.sort();
        files
            .iter()
            .filter_map(|p| {
                let img = crate::dds::decode(&std::fs::read(p).ok()?).ok()?;
                ((img.width, img.height) == (161, 98)).then(|| (p.file_name().unwrap().to_string_lossy().to_string(), img))
            })
            .collect()
    }

    /// What a model returns when it ignores "line art": vanilla's drawing filled in as a shaded grey body with its
    /// lines as darker detail, six times larger, on transparency.
    fn as_render(f: &Rgba) -> Rgba {
        let (w, h) = (f.width as usize, f.height as usize);
        let ink: Vec<bool> = f.data.chunks_exact(4).map(|p| p[3] >= 40).collect();
        // close the drawing (dilate 2), fill everything the border cannot reach, then erode back
        let mut grown = ink.clone();
        for _ in 0..2 {
            let prev = grown.clone();
            for y in 0..h {
                for x in 0..w {
                    let i = y * w + x;
                    grown[i] =
                        prev[i] || (x > 0 && prev[i - 1]) || (x + 1 < w && prev[i + 1]) || (y > 0 && prev[i - w]) || (y + 1 < h && prev[i + w]);
                }
            }
        }
        let mut outside = vec![false; w * h];
        let mut stack: Vec<usize> = (0..w).flat_map(|x| [x, (h - 1) * w + x]).chain((0..h).flat_map(|y| [y * w, y * w + w - 1])).collect();
        while let Some(i) = stack.pop() {
            if outside[i] || grown[i] {
                continue;
            }
            outside[i] = true;
            let (x, y) = (i % w, i / w);
            if x > 0 {
                stack.push(i - 1);
            }
            if x + 1 < w {
                stack.push(i + 1);
            }
            if y > 0 {
                stack.push(i - w);
            }
            if y + 1 < h {
                stack.push(i + w);
            }
        }
        let body = erode(&outside.iter().map(|o| !o).collect::<Vec<_>>(), w, h, 2);
        let mut r = Rgba::new(f.width, f.height);
        for y in 0..h {
            for x in 0..w {
                let i = y * w + x;
                if body[i] || ink[i] {
                    let l = if ink[i] && body[i] { 80.0 } else { 170.0 - 60.0 * y as f64 / h as f64 };
                    let g = l as u8;
                    r.set_px(x as u32, y as u32, [g, g, g, 255]);
                }
            }
        }
        resize_lanczos3(&r, f.width * 6, f.height * 6).unwrap()
    }

    fn median(mut v: Vec<f64>) -> f64 {
        v.sort_by(f64::total_cmp);
        v[v.len() / 2]
    }

    /// The recipe's output, made from shaded renders of vanilla's own drawings, is as sparse, as pale, as thin and as
    /// large as vanilla's drawings, and pure white.
    #[test]
    fn the_line_drawing_is_close_to_vanilla_on_vanillas_own_subjects() {
        let files = vanilla();
        if files.is_empty() {
            eprintln!("SKIPPED line art vs vanilla: Hearts of Iron IV not found (set HOI4_GAME_DIR)");
            return;
        }
        assert!(files.len() >= 50, "only {} vanilla special project icons decoded", files.len());
        let van: Vec<LineStats> = files.iter().map(|(_, f)| line_stats(f)).collect();
        let ours: Vec<LineStats> = files.iter().map(|(_, f)| line_stats(&line_art(&as_render(f), 161, 98).unwrap())).collect();
        let m = |s: &[LineStats], g: fn(&LineStats) -> f64| median(s.iter().map(g).collect());
        type Row = (&'static str, fn(&LineStats) -> f64, f64);
        let rows: [Row; 7] = [
            ("ink", |s| s.ink, 0.008),
            ("visible", |s| s.visible, 0.03),
            ("alpha median", |s| s.alpha_median, 20.0),
            ("alpha p90", |s| s.alpha_p90, 25.0),
            ("line width", |s| s.line_width, 0.35),
            ("span width", |s| s.span[0], 0.04),
            ("span height", |s| s.span[1], 0.05),
        ];
        // vanilla's own drawing, enlarged, must come back as a drawing like it, not as doubled edges
        let back: Vec<LineStats> =
            files.iter().map(|(_, f)| line_stats(&line_art(&resize_lanczos3(f, f.width * 4, f.height * 4).unwrap(), 161, 98).unwrap())).collect();
        let mut report = String::new();
        let mut off = Vec::new();
        for (set, stats) in [("shaded renders", &ours), ("vanilla drawings redrawn", &back)] {
            report.push_str(&format!("{set}:\n"));
            for (name, g, tol) in rows {
                let (v, o) = (m(&van, g), m(stats, g));
                report.push_str(&format!("  {name}: vanilla {v:.3}, ours {o:.3} (tolerance {tol})\n"));
                if (v - o).abs() > tol {
                    off.push(format!("{set}: {name}"));
                }
            }
        }
        eprintln!("line art vs vanilla, medians over {} files\n{report}", files.len());
        assert!(off.is_empty(), "outside the tolerance: {off:?}\n{report}");
        // no vanilla drawing has a filled area above the measured largest, and neither has ours
        let areas: Vec<f64> = files.iter().map(|(_, f)| largest_filled_area(f) as f64).collect();
        let largest = areas.iter().copied().fold(0.0, f64::max);
        eprintln!("filled area: vanilla median {}, largest {largest}", median(areas.clone()));
        assert_eq!((median(areas) as usize, largest as usize), (VANILLA_FILLED_AREA[0], VANILLA_FILLED_AREA[1]));
        assert!(files.iter().all(|(_, f)| largest_filled_area(&line_art(&as_render(f), 161, 98).unwrap()) <= FILLED_AREA_LIMIT));
        let van_off = median(van.iter().map(|s| s.off_white).collect());
        assert!(van_off < 0.01, "vanilla's lines are white: {van_off}");
        assert!(ours.iter().chain(&back).all(|s| s.off_white == 0.0));
    }
}
