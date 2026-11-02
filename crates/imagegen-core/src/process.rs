//! The deterministic processing chain: crop, trim, fit, resize, defringe, key, grayscale.
//!
//! Every operation is a pure function of its input pixels and parameters. A chain records the SHA-256 of
//! each intermediate so the ledger can show exactly how a candidate became the installed art.
//! Resizing is a step, never a verdict: a resized image still has to pass validation.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::hashing::sha256_hex;
use crate::pixels::Rgba;
use crate::resize::resize_lanczos3;

/// One processing step.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "op", rename_all = "snake_case", deny_unknown_fields)]
pub enum Op {
    /// Centre-crop to the aspect ratio `width:height` (no scaling).
    CropToAspect {
        /// Aspect width term.
        width: u32,
        /// Aspect height term.
        height: u32,
    },
    /// Crop to the bounding box of pixels with alpha at least `min_alpha`.
    TrimToSubject {
        /// Alpha threshold (the kit's margin checks use 16).
        #[serde(default = "default_min_alpha")]
        min_alpha: u8,
    },
    /// Trim to the subject, scale it to fit inside the canvas minus `margin` on every side, and
    /// centre it on a transparent canvas of exactly `width` x `height`.
    FitSubject {
        /// Canvas width.
        width: u32,
        /// Canvas height.
        height: u32,
        /// Transparent margin in pixels on every side.
        margin: u32,
        /// Alpha threshold for the subject bounds.
        #[serde(default = "default_min_alpha")]
        min_alpha: u8,
    },
    /// Resize to exactly `width` x `height` (Lanczos3, premultiplied alpha).
    Resize {
        /// Target width.
        width: u32,
        /// Target height.
        height: u32,
    },
    /// Centre-crop to the target aspect, then resize. For opaque pictures such as events.
    CoverResize {
        /// Target width.
        width: u32,
        /// Target height.
        height: u32,
    },
    /// Erode alpha by `erosion` pixels and bleed uncontaminated interior colour into the rim
    /// (the kit's `gfx.py defringe` algorithm). Fails on images without a cut-out.
    Defringe {
        /// Erosion in pixels (0 or 1 in practice).
        #[serde(default = "default_erosion")]
        erosion: u32,
    },
    /// Make a flat background colour transparent by flood fill from the image border.
    /// Never applied automatically: the caller must ask for it.
    ChromaKey {
        /// Key colour R, G, B.
        key: [u8; 3],
        /// Per-channel tolerance.
        #[serde(default = "default_tolerance")]
        tolerance: u8,
    },
    /// Convert colour to luminance, keep alpha (news pictures are black and white).
    Grayscale,
    /// Composite over a flat colour and make the result fully opaque.
    FlattenOn {
        /// Background R, G, B.
        background: [u8; 3],
    },
    /// Accept the image only when it is exactly `width` x `height` and change nothing. This is the finishing
    /// step of finished art that was imported at its final size: the size requirement is still recorded and
    /// enforced, the pixels are not touched.
    Conform {
        /// Required width.
        width: u32,
        /// Required height.
        height: u32,
    },
    /// The report-event card (sepia, grain, tilt, soft shadow) on a transparent 210x176 canvas, from a picture of
    /// any size. The grain seed is derived from the candidate's SHA-256, so a candidate always gives one card.
    ReportCard {
        /// Grain seed.
        seed: i64,
    },
    /// Two-frame strip: the fitted icon, then the same icon with the selected-state stroke and glow
    /// (faction and agency logos).
    SelectedFrame {
        /// Frames in the strip; two.
        frames: u32,
    },
    /// Two-frame strip: the fitted icon, then the same icon with its colour mixed 45 percent toward luminance
    /// (building icons).
    DisabledFrame {
        /// Frames in the strip; two.
        frames: u32,
    },
}

impl Op {
    /// The snake_case name of the operation (the `op` tag).
    pub fn name(&self) -> &'static str {
        match self {
            Op::CropToAspect { .. } => "crop_to_aspect",
            Op::TrimToSubject { .. } => "trim_to_subject",
            Op::FitSubject { .. } => "fit_subject",
            Op::Resize { .. } => "resize",
            Op::CoverResize { .. } => "cover_resize",
            Op::Defringe { .. } => "defringe",
            Op::ChromaKey { .. } => "chroma_key",
            Op::Grayscale => "grayscale",
            Op::FlattenOn { .. } => "flatten_on",
            Op::Conform { .. } => "conform",
            Op::ReportCard { .. } => "report_card",
            Op::SelectedFrame { .. } => "selected_frame",
            Op::DisabledFrame { .. } => "disabled_frame",
        }
    }
}

fn default_min_alpha() -> u8 {
    16
}
fn default_erosion() -> u32 {
    1
}
fn default_tolerance() -> u8 {
    24
}

/// What a step did.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct StepRecord {
    /// The step.
    pub op: Op,
    /// SHA-256 of the input pixels.
    pub input_sha256: String,
    /// SHA-256 of the output pixels.
    pub output_sha256: String,
    /// Output size.
    pub output_size: [u32; 2],
    /// Version of the processing code, bumped when an operation's output could change.
    pub engine: String,
}

/// Version string of the processing code recorded with every step.
pub const ENGINE_VERSION: &str = concat!("imagegen-core/", env!("CARGO_PKG_VERSION"), "/process-2");

/// Apply one operation.
pub fn apply(img: &Rgba, op: &Op) -> Result<Rgba> {
    match op {
        Op::CropToAspect { width, height } => crop_to_aspect(img, *width, *height),
        Op::TrimToSubject { min_alpha } => {
            let b = img.alpha_bbox(*min_alpha).ok_or_else(|| Error::Image("no visible subject to trim to".into()))?;
            img.crop(b[0], b[1], b[2] - b[0], b[3] - b[1])
        }
        Op::FitSubject { width, height, margin, min_alpha } => fit_subject(img, *width, *height, *margin, *min_alpha),
        Op::Resize { width, height } => resize_lanczos3(img, *width, *height),
        Op::CoverResize { width, height } => {
            let cropped = crop_to_aspect(img, *width, *height)?;
            resize_lanczos3(&cropped, *width, *height)
        }
        Op::Defringe { erosion } => defringe(img, *erosion, None),
        Op::ChromaKey { key, tolerance } => Ok(chroma_key(img, *key, *tolerance)),
        Op::Grayscale => Ok(grayscale(img)),
        Op::FlattenOn { background } => Ok(flatten_on(img, *background)),
        Op::Conform { width, height } => {
            if (img.width, img.height) != (*width, *height) {
                return Err(Error::Invalid(format!(
                    "the art is {}x{} but must be exactly {width}x{height} to be accepted as it is",
                    img.width, img.height
                )));
            }
            Ok(img.clone())
        }
        Op::ReportCard { seed } => {
            crate::recipes::report_card::report_card(img, &crate::recipes::report_card::ReportCardParams { seed: *seed, ..Default::default() })
        }
        Op::SelectedFrame { frames } => crate::recipes::frames::selected_strip(img, *frames),
        Op::DisabledFrame { frames } => crate::recipes::frames::disabled_strip(img, *frames),
    }
}

/// Largest enlargement an icon family tolerates in a fit or resize step. A bigger factor invents detail an
/// icon never had, which is what turns a small wrong-family image into a "valid" icon.
pub const ICON_MAX_UPSCALE: f64 = 1.25;

/// Error code of a refused enlargement.
pub const UPSCALE_CODE: &str = "GFX-UPSCALE-LIMIT";

/// The enlargement factor a step would apply to `img` (above 1 enlarges), when it scales at all.
pub fn upscale_factor(img: &Rgba, op: &Op) -> Option<f64> {
    match op {
        Op::Resize { width, height } => Some((f64::from(*width) / f64::from(img.width)).max(f64::from(*height) / f64::from(img.height))),
        Op::CoverResize { width, height } => {
            let c = crop_to_aspect(img, *width, *height).ok()?;
            Some((f64::from(*width) / f64::from(c.width)).max(f64::from(*height) / f64::from(c.height)))
        }
        Op::FitSubject { width, height, margin, min_alpha } => {
            if *margin * 2 >= *width || *margin * 2 >= *height {
                return None;
            }
            let b = img.alpha_bbox(*min_alpha)?;
            let (sw, sh) = (f64::from(b[2] - b[0]), f64::from(b[3] - b[1]));
            let (aw, ah) = (f64::from(*width - 2 * margin), f64::from(*height - 2 * margin));
            Some((aw / sw).min(ah / sh))
        }
        _ => None,
    }
}

/// Run a chain and record every step.
pub fn run_chain(img: &Rgba, ops: &[Op]) -> Result<(Rgba, Vec<StepRecord>)> {
    run_chain_limited(img, ops, None)
}

/// Run a chain and refuse any step that enlarges its input by more than `max_upscale`.
pub fn run_chain_limited(img: &Rgba, ops: &[Op], max_upscale: Option<f64>) -> Result<(Rgba, Vec<StepRecord>)> {
    let mut current = img.clone();
    let mut records = Vec::with_capacity(ops.len());
    for op in ops {
        if let (Some(limit), Some(factor)) = (max_upscale, upscale_factor(&current, op)) {
            if factor > limit + 1e-9 {
                return Err(Error::refused(
                    UPSCALE_CODE,
                    format!(
                        "this step would enlarge the image {factor:.2} times, above the {limit:.2} times an icon family allows. Upscaling invents detail: bring a larger source or generate the asset"
                    ),
                    crate::error::RetryClass::Terminal,
                ));
            }
        }
        let input_sha256 = sha256_hex(&current.data);
        let next = apply(&current, op)?;
        records.push(StepRecord {
            op: op.clone(),
            input_sha256,
            output_sha256: sha256_hex(&next.data),
            output_size: [next.width, next.height],
            engine: ENGINE_VERSION.to_string(),
        });
        current = next;
    }
    Ok((current, records))
}

fn crop_to_aspect(img: &Rgba, aw: u32, ah: u32) -> Result<Rgba> {
    if aw == 0 || ah == 0 {
        return Err(Error::Invalid("aspect terms must be positive".into()));
    }
    // largest centred rectangle of aspect aw:ah that fits inside the image
    let (iw, ih) = (u64::from(img.width), u64::from(img.height));
    let (aw, ah) = (u64::from(aw), u64::from(ah));
    let (cw, ch) = if iw * ah >= ih * aw {
        // image is wider than the aspect: keep full height
        ((ih * aw) / ah, ih)
    } else {
        (iw, (iw * ah) / aw)
    };
    let (cw, ch) = (cw.max(1) as u32, ch.max(1) as u32);
    img.crop((img.width - cw) / 2, (img.height - ch) / 2, cw, ch)
}

fn fit_subject(img: &Rgba, w: u32, h: u32, margin: u32, min_alpha: u8) -> Result<Rgba> {
    if w == 0 || h == 0 || margin * 2 >= w || margin * 2 >= h {
        return Err(Error::Invalid(format!("margin {margin} leaves no room in a {w}x{h} canvas")));
    }
    let b = img.alpha_bbox(min_alpha).ok_or_else(|| Error::Image("no visible subject to fit".into()))?;
    let subject = img.crop(b[0], b[1], b[2] - b[0], b[3] - b[1])?;
    let (aw, ah) = (w - 2 * margin, h - 2 * margin);
    // scale so the subject fills one dimension of the available area (largest fit, aspect kept)
    let scale_w = f64::from(aw) / f64::from(subject.width);
    let scale_h = f64::from(ah) / f64::from(subject.height);
    let scale = scale_w.min(scale_h);
    let tw = ((f64::from(subject.width) * scale).round() as u32).clamp(1, aw);
    let th = ((f64::from(subject.height) * scale).round() as u32).clamp(1, ah);
    let scaled = resize_lanczos3(&subject, tw, th)?;
    let mut canvas = Rgba::new(w, h);
    canvas.paste(&scaled, i64::from((w - tw) / 2), i64::from((h - th) / 2));
    Ok(canvas)
}

fn is_chroma(px: [u8; 4], delta: i16) -> bool {
    let (r, g, b) = (i16::from(px[0]), i16::from(px[1]), i16::from(px[2]));
    (r > g + delta && b > g + delta) || (g > r + delta && g > b + delta)
}

/// Edge pixels as `gfx.py` counts them: visible and either translucent or touching a non-visible
/// 4-neighbour (outside the image counts as non-visible).
pub fn edge_mask(img: &Rgba) -> Vec<bool> {
    let (w, h) = (img.width as i64, img.height as i64);
    let visible = |x: i64, y: i64| -> bool { x >= 0 && y >= 0 && x < w && y < h && img.alpha(x as u32, y as u32) > 0 };
    let mut out = vec![false; (w * h) as usize];
    for y in 0..h {
        for x in 0..w {
            let a = img.alpha(x as u32, y as u32);
            if a == 0 {
                continue;
            }
            let interior = visible(x - 1, y) && visible(x + 1, y) && visible(x, y - 1) && visible(x, y + 1);
            out[(y * w + x) as usize] = a < 255 || !interior;
        }
    }
    out
}

/// Number of visible edge pixels and how many of them carry the magenta or green key colour.
pub fn edge_stats(img: &Rgba) -> (u64, u64) {
    let mask = edge_mask(img);
    let mut edge = 0u64;
    let mut chroma = 0u64;
    for (i, e) in mask.iter().enumerate() {
        if *e {
            edge += 1;
            let p = [img.data[i * 4], img.data[i * 4 + 1], img.data[i * 4 + 2], img.data[i * 4 + 3]];
            if is_chroma(p, 40) {
                chroma += 1;
            }
        }
    }
    (edge, chroma)
}

/// Erode alpha and bleed interior colour into the rim. Same algorithm and thresholds as `gfx.py defringe`:
/// the core is opaque (alpha >= 240), not key-coloured, and survives a one-pixel cross erosion; alpha is
/// eroded with a zero-padded minimum filter; rim pixels take the colour of the nearest core pixel.
pub fn defringe(img: &Rgba, erosion: u32, spill_mask: Option<&[bool]>) -> Result<Rgba> {
    let (w, h) = (img.width as i64, img.height as i64);
    let n = (w * h) as usize;
    let alpha = |i: usize| img.data[i * 4 + 3];
    if (0..n).all(|i| alpha(i) == 0) {
        return Err(Error::Image("empty subject".into()));
    }
    if (0..n).all(|i| alpha(i) > 0) {
        return Err(Error::Image("no transparent canvas; cannot infer a cut-out".into()));
    }
    if let Some(m) = spill_mask {
        if m.len() != n {
            return Err(Error::Image("spill mask must match the frame canvas".into()));
        }
    }
    let spill = |i: usize| spill_mask.map(|m| m[i]).unwrap_or(false);
    let px = |i: usize| [img.data[i * 4], img.data[i * 4 + 1], img.data[i * 4 + 2], img.data[i * 4 + 3]];

    // core = opaque, not chroma, not spill; then 4-neighbour erosion with outside = false
    let base: Vec<bool> = (0..n).map(|i| alpha(i) >= 240 && !is_chroma(px(i), 40) && !spill(i)).collect();
    let core = erode_cross(&base, w, h, 1);
    if !core.iter().any(|c| *c) {
        return Err(Error::Image("no uncontaminated opaque interior; repair needs source review".into()));
    }

    // alpha erosion: minimum filter of (2*erosion+1)^2 with zero padding
    let e = erosion as i64;
    let mut alpha_out: Vec<u8> = (0..n).map(alpha).collect();
    if e > 0 {
        for y in 0..h {
            for x in 0..w {
                let mut m = 255u8;
                for dy in -e..=e {
                    for dx in -e..=e {
                        let (nx, ny) = (x + dx, y + dy);
                        let a = if nx < 0 || ny < 0 || nx >= w || ny >= h { 0 } else { alpha((ny * w + nx) as usize) };
                        m = m.min(a);
                    }
                }
                alpha_out[(y * w + x) as usize] = m;
            }
        }
    }
    if !alpha_out.iter().any(|a| *a > 0) {
        return Err(Error::Image("erosion would erase the subject".into()));
    }

    // boundary band: not inside a 2-pixel erosion of (alpha >= 240)
    let solid: Vec<bool> = (0..n).map(|i| alpha(i) >= 240).collect();
    let inner = erode_cross(&solid, w, h, 2);
    let rim: Vec<bool> = (0..n).map(|i| alpha_out[i] < 255 || (alpha_out[i] > 0 && is_chroma(px(i), 40) && !inner[i]) || spill(i)).collect();

    // only rim pixels that stay visible need a colour: a fully transparent pixel never shows, and every later step
    // resizes with premultiplied alpha
    let want: Vec<bool> = (0..n).map(|i| rim[i] && alpha_out[i] > 0).collect();
    let nearest = nearest_core(&core, w, h, &want);
    let mut out = img.clone();
    for i in 0..n {
        if rim[i] {
            if let Some(src) = nearest[i] {
                out.data[i * 4] = img.data[src * 4];
                out.data[i * 4 + 1] = img.data[src * 4 + 1];
                out.data[i * 4 + 2] = img.data[src * 4 + 2];
            }
        }
        out.data[i * 4 + 3] = alpha_out[i];
    }
    Ok(out)
}

/// Binary erosion with the 4-connected structuring element, outside the image counting as false.
fn erode_cross(mask: &[bool], w: i64, h: i64, iterations: u32) -> Vec<bool> {
    let mut cur = mask.to_vec();
    for _ in 0..iterations {
        let mut next = vec![false; cur.len()];
        for y in 0..h {
            for x in 0..w {
                let at = |xx: i64, yy: i64| xx >= 0 && yy >= 0 && xx < w && yy < h && cur[(yy * w + xx) as usize];
                next[(y * w + x) as usize] = at(x, y) && at(x - 1, y) && at(x + 1, y) && at(x, y - 1) && at(x, y + 1);
            }
        }
        cur = next;
    }
    cur
}

/// For each pixel where `want` is true, the index of the nearest core pixel by squared Euclidean distance, ties
/// resolved by scan order (the lowest index), so the result is deterministic and equals a brute-force search.
///
/// The search grows square rings around the pixel until a core pixel is found at distance `d`, then finishes every
/// ring that can still hold a pixel at distance `d` or less. Cost is about the squared distance to the nearest
/// core pixel, so it stays fast on large provider images; the old brute force compared every pixel with every core
/// pixel and took minutes on a 1340x1174 icon.
fn nearest_core(core: &[bool], w: i64, h: i64, want: &[bool]) -> Vec<Option<usize>> {
    let mut out = vec![None; (w * h) as usize];
    if !core.iter().any(|c| *c) {
        return out;
    }
    let max_r = w.max(h);
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if !want[i] {
                continue;
            }
            let mut best: Option<(i64, usize)> = None;
            let mut r = 0i64;
            while r <= max_r {
                // stop once no pixel on ring r can be nearer than the best found (the closest point of ring r is r away)
                if let Some((bd, _)) = best {
                    if r * r > bd {
                        break;
                    }
                }
                let (x0, x1, y0, y1) = (x - r, x + r, y - r, y + r);
                let mut visit = |cx: i64, cy: i64| {
                    if cx < 0 || cy < 0 || cx >= w || cy >= h {
                        return;
                    }
                    let idx = (cy * w + cx) as usize;
                    if !core[idx] {
                        return;
                    }
                    let d = (cx - x) * (cx - x) + (cy - y) * (cy - y);
                    if best.map(|(bd, bi)| d < bd || (d == bd && idx < bi)).unwrap_or(true) {
                        best = Some((d, idx));
                    }
                };
                if r == 0 {
                    visit(x, y);
                } else {
                    for cx in x0..=x1 {
                        visit(cx, y0);
                        visit(cx, y1);
                    }
                    for cy in (y0 + 1)..y1 {
                        visit(x0, cy);
                        visit(x1, cy);
                    }
                }
                r += 1;
            }
            out[i] = best.map(|(_, idx)| idx);
        }
    }
    out
}

fn chroma_key(img: &Rgba, key: [u8; 3], tolerance: u8) -> Rgba {
    let (w, h) = (img.width as i64, img.height as i64);
    let tol = i16::from(tolerance);
    let matches = |x: i64, y: i64| -> bool {
        let p = img.px(x as u32, y as u32);
        (0..3).all(|c| (i16::from(p[c]) - i16::from(key[c])).abs() <= tol)
    };
    let mut seen = vec![false; (w * h) as usize];
    let mut stack: Vec<(i64, i64)> = Vec::new();
    for x in 0..w {
        stack.push((x, 0));
        stack.push((x, h - 1));
    }
    for y in 0..h {
        stack.push((0, y));
        stack.push((w - 1, y));
    }
    let mut out = img.clone();
    while let Some((x, y)) = stack.pop() {
        if x < 0 || y < 0 || x >= w || y >= h || seen[(y * w + x) as usize] || !matches(x, y) {
            continue;
        }
        seen[(y * w + x) as usize] = true;
        out.set_px(x as u32, y as u32, [0, 0, 0, 0]);
        stack.extend([(x + 1, y), (x - 1, y), (x, y + 1), (x, y - 1)]);
    }
    out
}

fn grayscale(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        // Rec. 601 luma in integer arithmetic
        let l = ((u32::from(p[0]) * 299 + u32::from(p[1]) * 587 + u32::from(p[2]) * 114 + 500) / 1000) as u8;
        p[0] = l;
        p[1] = l;
        p[2] = l;
    }
    out
}

fn flatten_on(img: &Rgba, bg: [u8; 3]) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        let a = u32::from(p[3]);
        for c in 0..3 {
            p[c] = ((u32::from(p[c]) * a + u32::from(bg[c]) * (255 - a) + 127) / 255) as u8;
        }
        p[3] = 255;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_search_finds_exactly_what_a_brute_force_search_finds() {
        // deterministic pseudo-random masks, including ties and sparse cores far from most pixels
        let mut seed = 0x2545_f491_u64;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        for (w, h, density) in [(17i64, 11i64, 3u64), (40, 23, 50), (31, 31, 400), (9, 40, 7)] {
            let core: Vec<bool> = (0..w * h).map(|_| next() % density == 0).collect();
            if !core.iter().any(|c| *c) {
                continue;
            }
            let want = vec![true; (w * h) as usize];
            let fast = nearest_core(&core, w, h, &want);
            for y in 0..h {
                for x in 0..w {
                    let mut best: Option<(i64, usize)> = None;
                    for cy in 0..h {
                        for cx in 0..w {
                            let idx = (cy * w + cx) as usize;
                            let d = (cx - x) * (cx - x) + (cy - y) * (cy - y);
                            if core[idx] && best.map(|(bd, _)| d < bd).unwrap_or(true) {
                                best = Some((d, idx));
                            }
                        }
                    }
                    assert_eq!(fast[(y * w + x) as usize], best.map(|(_, i)| i), "{w}x{h} at {x},{y}");
                }
            }
        }
    }

    fn disc(size: u32) -> Rgba {
        let mut img = Rgba::new(size, size);
        let c = f64::from(size) / 2.0;
        for y in 0..size {
            for x in 0..size {
                let d = ((f64::from(x) + 0.5 - c).powi(2) + (f64::from(y) + 0.5 - c).powi(2)).sqrt();
                if d < c - 1.0 {
                    img.set_px(x, y, [200, 80, 40, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn crop_to_aspect_is_centred() {
        let img = Rgba::new(200, 100);
        let out = apply(&img, &Op::CropToAspect { width: 1, height: 1 }).unwrap();
        assert_eq!((out.width, out.height), (100, 100));
        let out = apply(&img, &Op::CropToAspect { width: 397, height: 153 }).unwrap();
        assert!(out.width <= 200 && out.height <= 100);
        let ratio = f64::from(out.width) / f64::from(out.height);
        assert!((ratio - 397.0 / 153.0).abs() < 0.05);
        assert!(apply(&img, &Op::CropToAspect { width: 0, height: 1 }).is_err());
    }

    #[test]
    fn fit_subject_places_a_centred_subject_with_margin() {
        let mut img = Rgba::new(300, 300);
        // off-centre blob
        for y in 40..120 {
            for x in 180..280 {
                img.set_px(x, y, [10, 200, 10, 255]);
            }
        }
        let out = apply(&img, &Op::FitSubject { width: 100, height: 88, margin: 2, min_alpha: 16 }).unwrap();
        assert_eq!((out.width, out.height), (100, 88));
        let b = out.alpha_bbox(16).unwrap();
        assert!(b[0] >= 2 && b[1] >= 2 && 100 - b[2] >= 2 && 88 - b[3] >= 2, "{b:?}");
        // the limiting dimension is filled
        let span = f64::from(b[2] - b[0]) / 100.0;
        let spanv = f64::from(b[3] - b[1]) / 88.0;
        assert!(span.max(spanv) >= 0.9);
        assert!(apply(&Rgba::new(10, 10), &Op::FitSubject { width: 10, height: 10, margin: 1, min_alpha: 16 }).is_err());
        assert!(apply(&img, &Op::FitSubject { width: 10, height: 10, margin: 5, min_alpha: 16 }).is_err());
    }

    #[test]
    fn chain_records_hashes_and_is_deterministic() {
        let img = disc(120);
        let ops = vec![Op::TrimToSubject { min_alpha: 16 }, Op::FitSubject { width: 65, height: 67, margin: 2, min_alpha: 16 }];
        let (a, rec_a) = run_chain(&img, &ops).unwrap();
        let (b, rec_b) = run_chain(&img, &ops).unwrap();
        assert_eq!(a, b);
        assert_eq!(rec_a, rec_b);
        assert_eq!(rec_a.len(), 2);
        assert_eq!(rec_a[0].input_sha256, img.pixels_sha256());
        assert_eq!(rec_a[0].output_sha256, rec_a[1].input_sha256);
        assert_eq!(rec_a[1].output_sha256, a.pixels_sha256());
    }

    #[test]
    fn chroma_key_floods_only_from_the_border() {
        let mut img = Rgba::filled(20, 20, [255, 0, 255, 255]);
        for y in 5..15 {
            for x in 5..15 {
                img.set_px(x, y, [30, 30, 30, 255]);
            }
        }
        // a magenta pixel enclosed by the subject must survive
        img.set_px(10, 10, [255, 0, 255, 255]);
        let out = apply(&img, &Op::ChromaKey { key: [255, 0, 255], tolerance: 10 }).unwrap();
        assert_eq!(out.alpha(0, 0), 0);
        assert_eq!(out.alpha(7, 7), 255);
        assert_eq!(out.alpha(10, 10), 255);
    }

    #[test]
    fn grayscale_and_flatten() {
        let img = Rgba::filled(2, 2, [255, 0, 0, 128]);
        let g = apply(&img, &Op::Grayscale).unwrap();
        assert_eq!(g.px(0, 0), [76, 76, 76, 128]);
        let f = apply(&img, &Op::FlattenOn { background: [0, 0, 255] }).unwrap();
        assert_eq!(f.px(0, 0)[3], 255);
        assert!(f.px(0, 0)[0] > 120 && f.px(0, 0)[2] > 100);
    }

    #[test]
    fn conform_accepts_the_exact_size_and_changes_nothing() {
        let img = Rgba::filled(6, 4, [1, 2, 3, 200]);
        assert_eq!(apply(&img, &Op::Conform { width: 6, height: 4 }).unwrap(), img);
        assert!(apply(&img, &Op::Conform { width: 6, height: 5 }).is_err());
        let (out, rec) = run_chain(&img, &[Op::Conform { width: 6, height: 4 }]).unwrap();
        assert_eq!(out, img);
        assert_eq!(rec[0].input_sha256, rec[0].output_sha256, "a conform step leaves the pixels alone");
    }

    #[test]
    fn report_card_op_is_deterministic_seeded_and_has_transparent_corners() {
        let mut photo = Rgba::new(300, 200);
        for y in 0..200 {
            for x in 0..300 {
                photo.set_px(x, y, [(x % 251) as u8, (y % 241) as u8, ((x * y) % 239) as u8, 255]);
            }
        }
        let a = apply(&photo, &Op::ReportCard { seed: 7 }).unwrap();
        assert_eq!((a.width, a.height), (210, 176));
        assert_eq!(a, apply(&photo, &Op::ReportCard { seed: 7 }).unwrap());
        assert_ne!(a, apply(&photo, &Op::ReportCard { seed: 8 }).unwrap(), "the seed changes the grain");
        let (w, h) = (a.width - 1, a.height - 1);
        assert_eq!([a.alpha(0, 0), a.alpha(w, 0), a.alpha(0, h), a.alpha(w, h)], [0, 0, 0, 0]);
        assert_eq!(a.alpha(105, 88), 255);
        // the op serialises with the snake_case tag
        assert_eq!(serde_json::to_value(Op::ReportCard { seed: 7 }).unwrap(), serde_json::json!({"op": "report_card", "seed": 7}));
    }

    #[test]
    fn frame_ops_build_two_frame_strips_and_serialise_like_the_others() {
        let icon = disc(64);
        let sel = apply(&icon, &Op::SelectedFrame { frames: 2 }).unwrap();
        assert_eq!((sel.width, sel.height), (128, 64));
        assert_eq!(sel.split_frames(2).unwrap()[0], icon);
        let dis = apply(&icon, &Op::DisabledFrame { frames: 2 }).unwrap();
        assert_eq!((dis.width, dis.height), (128, 64));
        assert_ne!(dis.split_frames(2).unwrap()[1], icon);
        assert!(apply(&icon, &Op::SelectedFrame { frames: 3 }).is_err());
        let v = serde_json::to_value(Op::DisabledFrame { frames: 2 }).unwrap();
        assert_eq!(v, serde_json::json!({"op": "disabled_frame", "frames": 2}));
        assert_eq!(serde_json::from_value::<Op>(serde_json::json!({"op": "selected_frame", "frames": 2})).unwrap(), Op::SelectedFrame { frames: 2 });
        assert!(serde_json::from_value::<Op>(serde_json::json!({"op": "selected_frame", "frames": 2, "x": 1})).is_err());
        for op in [Op::Conform { width: 1, height: 1 }, Op::ReportCard { seed: 1 }, Op::SelectedFrame { frames: 2 }, Op::DisabledFrame { frames: 2 }]
        {
            let tag = serde_json::to_value(&op).unwrap()["op"].as_str().unwrap().to_string();
            assert_eq!(tag, op.name(), "name() is the serde tag");
        }
    }

    #[test]
    fn defringe_removes_magenta_rim() {
        // opaque green-ish disc whose edge ring is magenta contamination
        let mut img = Rgba::new(40, 40);
        for y in 0..40u32 {
            for x in 0..40u32 {
                let d = ((f64::from(x) - 19.5).powi(2) + (f64::from(y) - 19.5).powi(2)).sqrt();
                if d < 15.0 {
                    img.set_px(x, y, [120, 100, 80, 255]);
                } else if d < 16.5 {
                    img.set_px(x, y, [255, 0, 255, 255]);
                }
            }
        }
        let (_, before) = edge_stats(&img);
        assert!(before > 0);
        let out = apply(&img, &Op::Defringe { erosion: 1 }).unwrap();
        let (_, after) = edge_stats(&out);
        assert!(after < before, "fringe {before} -> {after}");
        // errors on fully opaque or empty input
        assert!(apply(&Rgba::filled(4, 4, [1, 2, 3, 255]), &Op::Defringe { erosion: 1 }).is_err());
        assert!(apply(&Rgba::new(4, 4), &Op::Defringe { erosion: 1 }).is_err());
    }
}

#[cfg(test)]
mod upscale_tests {
    use super::*;

    fn blob(w: u32, h: u32) -> Rgba {
        Rgba::filled(w, h, [10, 20, 30, 255])
    }

    #[test]
    fn enlarging_beyond_the_limit_is_refused_and_shrinking_is_free() {
        let small = blob(40, 40);
        let err = run_chain_limited(&small, &[Op::Resize { width: 64, height: 64 }], Some(ICON_MAX_UPSCALE)).unwrap_err();
        assert_eq!(err.code(), UPSCALE_CODE);
        // 40 -> 50 is exactly 1.25 times: allowed
        assert!(run_chain_limited(&small, &[Op::Resize { width: 50, height: 50 }], Some(ICON_MAX_UPSCALE)).is_ok());
        assert!(run_chain_limited(&blob(200, 200), &[Op::Resize { width: 64, height: 64 }], Some(ICON_MAX_UPSCALE)).is_ok());
        // without a limit nothing changes (pictures may enlarge)
        assert!(run_chain(&small, &[Op::Resize { width: 64, height: 64 }]).is_ok());
    }

    #[test]
    fn fit_and_cover_steps_are_limited_too() {
        let small = blob(30, 30);
        let fit = Op::FitSubject { width: 64, height: 64, margin: 2, min_alpha: 16 };
        assert_eq!(run_chain_limited(&small, &[fit], Some(ICON_MAX_UPSCALE)).unwrap_err().code(), UPSCALE_CODE);
        let cover = Op::CoverResize { width: 100, height: 60 };
        assert_eq!(run_chain_limited(&small, &[cover], Some(ICON_MAX_UPSCALE)).unwrap_err().code(), UPSCALE_CODE);
        assert!(upscale_factor(&small, &Op::Grayscale).is_none());
    }
}
