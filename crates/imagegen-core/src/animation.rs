//! Animated sprites: the `frameAnimatedSpriteType` strips of the `animation` group.
//!
//! Vanilla 1.19.4.1 registers 160 `frameAnimatedSpriteType` sprites (`data/vanilla-measurement.json`,
//! `animated_sprites`, produced by `scripts/measure_animated.py`). Four kinds cover most of them, and each is one
//! family here:
//!
//! | Family | Frames | fps | Effect | How the frames are made |
//! |---|---|---|---|---|
//! | `anim_blend_glow` | 2 | 1 | `buttonstate_blendframes.lua` | the model draws the bright glow on black; the tool turns black into alpha and derives the dim frame |
//! | `anim_blend_keyframes` | 3 | 1 | `buttonstate_blendframes.lua` | the base frame is generated; frames 2 and 3 are `hoi4.image_edit` candidates of it, fitted together with it |
//! | `anim_scroll` | 9 | 15 | none | the base is made seamless across its width and scrolls one width per loop |
//! | `anim_spin` | 20 | 7 | none | the base is fitted by its radius and turned 360 degrees per loop |
//!
//! Consistency comes from making one base frame and deriving the others from it. Where the motion is procedural
//! (a glow pulse, a scroll, a turn) the other frames are a pure function of the base, so shape, palette and
//! registration cannot drift. Where the art itself changes (the key frames) the edits are fitted with one shared
//! crop and scale, never one fit each, so an edit that kept the framing stays registered. The `animation` check
//! then measures what remains: alignment, silhouette, palette, the loop seam and frames that do not move.
//!
//! Thresholds are set from the vanilla strips (`families[].animation` in the measurement): the key-frame strips
//! drift at most 5 px (p90) on a 141x115 frame and keep their mean colour within 1.7 levels, the turning icons
//! keep a silhouette overlap above 0.83 (p10), and no vanilla loop seam is more than 2.7 times the median step.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::pillow::{affine_bicubic, premultiply, unpremultiply};
use crate::recipes::strip::build_strip;
use crate::resize::resize_lanczos3;

/// Version of the animation recipes (bump when an output could change).
pub const RECIPE_VERSION: u32 = 1;

/// The effect file of vanilla's blended animations, exactly as vanilla writes it.
pub const BLEND_EFFECT: &str = "gfx/FX/buttonstate_blendframes.lua";

/// Alpha of the dim frame of a blend glow relative to the bright one: the median over the 66 vanilla two-frame
/// glow textures whose dim frame is neither empty nor as bright (0.1698, measured on 1.19.4.1; 21 more fade to
/// nothing).
pub const GLOW_DIM_RATIO: f64 = 0.17;

/// Alpha below which a pixel does not count for the subject bounds of a glow (a glow's soft edge is faint).
pub const GLOW_MIN_ALPHA: u8 = 4;

/// How the frames of an animated family move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    /// Two frames the engine cross-fades: dim, then bright.
    BlendGlow,
    /// Key frames the engine cross-fades; frames after the first are edits of it.
    BlendKeyframes,
    /// The base scrolls horizontally by one width per loop.
    Scroll,
    /// The base turns a full circle per loop.
    Spin,
}

impl Motion {
    /// Stable id.
    pub fn as_str(self) -> &'static str {
        match self {
            Motion::BlendGlow => "blend_glow",
            Motion::BlendKeyframes => "blend_keyframes",
            Motion::Scroll => "scroll",
            Motion::Spin => "spin",
        }
    }
}

/// The animation fields of a `frameAnimatedSpriteType`, plus how the frames are made.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AnimationSpec {
    /// How the frames move.
    pub motion: Motion,
    /// `animation_rate_fps`, written exactly as given (vanilla writes `1`, `1.5`, `15`).
    pub fps: String,
    /// `looping`.
    pub looping: bool,
    /// `play_on_show`.
    pub play_on_show: bool,
    /// `pause_on_loop`, written exactly as given (vanilla writes `0.0`).
    pub pause_on_loop: String,
    /// `effectFile`, when the engine blends the frames.
    #[serde(default)]
    pub effect_file: Option<String>,
    /// `alwaystransparent = yes` (the overlay never takes clicks), as vanilla's blend glows have.
    #[serde(default)]
    pub always_transparent: bool,
    /// `loadType`, as vanilla's frame strips have (`INGAME`).
    #[serde(default)]
    pub load_type: Option<String>,
    /// `transparencecheck = yes`, as vanilla's frame strips have.
    #[serde(default)]
    pub transparence_check: bool,
}

impl AnimationSpec {
    /// The frame rate as a number.
    pub fn fps_value(&self) -> Result<f64> {
        let v: f64 = self.fps.trim().parse().map_err(|_| Error::Config(format!("animation fps '{}' is not a number", self.fps)))?;
        if !(v > 0.0 && v <= 120.0) {
            return Err(Error::Config(format!("animation fps {v} is outside 0 to 120")));
        }
        Ok(v)
    }

    /// The pause at the end of a loop, in seconds.
    pub fn pause_value(&self) -> f64 {
        self.pause_on_loop.trim().parse::<f64>().ok().filter(|v| v.is_finite() && *v >= 0.0).unwrap_or(0.0)
    }

    /// True when the engine cross-fades the frames.
    pub fn blends(&self) -> bool {
        self.effect_file.as_deref().map(|e| e.to_ascii_lowercase().ends_with("buttonstate_blendframes.lua")).unwrap_or(false)
    }
}

fn check_text_field(label: &str, v: &str) -> Result<()> {
    if v.is_empty() || v.chars().any(|c| c == '"' || c == '{' || c == '}' || c == '#' || c.is_control()) {
        return Err(Error::Config(format!("animation {label} '{v}' cannot be written into a .gfx file")));
    }
    Ok(())
}

/// The `frameAnimatedSpriteType` block, tab-indented one level and ending in a newline. Field order follows
/// vanilla: the strip fields (`loadType`, `transparencecheck`) come before the rate as in `leadergroups.gfx`, the
/// blend fields (`alwaystransparent`, `effectFile`) after the pause as in `alerts.gfx`.
pub fn sprite_block(name: &str, texture_path: &str, frames: u32, spec: &AnimationSpec) -> Result<String> {
    spec.fps_value()?;
    check_text_field("fps", &spec.fps)?;
    check_text_field("pause_on_loop", &spec.pause_on_loop)?;
    let yn = |b: bool| if b { "yes" } else { "no" };
    let mut t = String::from("\tframeAnimatedSpriteType = {\n");
    t.push_str(&format!("\t\tname = \"{name}\"\n\t\ttexturefile = \"{texture_path}\"\n\t\tnoOfFrames = {frames}\n"));
    if let Some(l) = &spec.load_type {
        check_text_field("load_type", l)?;
        t.push_str(&format!("\t\tloadType = \"{l}\"\n"));
    }
    if spec.transparence_check {
        t.push_str("\t\ttransparencecheck = yes\n");
    }
    t.push_str(&format!(
        "\t\tanimation_rate_fps = {}\n\t\tlooping = {}\n\t\tplay_on_show = {}\n\t\tpause_on_loop = {}\n",
        spec.fps.trim(),
        yn(spec.looping),
        yn(spec.play_on_show),
        spec.pause_on_loop.trim()
    ));
    if spec.always_transparent {
        t.push_str("\t\talwaystransparent = yes\n");
    }
    if let Some(e) = &spec.effect_file {
        check_text_field("effect_file", e)?;
        t.push_str(&format!("\t\teffectFile = \"{e}\"\n"));
    }
    t.push_str("\t}\n");
    Ok(t)
}

// ---------------------------------------------------------------- derivations

/// Light on black to alpha: alpha becomes the brightest channel and colour is divided by it, so a glow the model
/// drew on a black background becomes a transparent glow that composites back to the same picture over black.
pub fn black_to_alpha(img: &Rgba) -> Rgba {
    let mut out = img.clone();
    for p in out.data.chunks_exact_mut(4) {
        // composite over black first: whatever alpha the provider sent is folded into the light
        let a0 = u32::from(p[3]);
        let rgb = [p[0], p[1], p[2]].map(|v| (u32::from(v) * a0 + 127) / 255);
        let a = rgb.iter().copied().max().unwrap_or(0);
        if a == 0 {
            p.copy_from_slice(&[0, 0, 0, 0]);
            continue;
        }
        for (i, v) in rgb.iter().enumerate() {
            p[i] = ((v * 255 + a / 2) / a).min(255) as u8;
        }
        p[3] = a as u8;
    }
    out
}

/// A two-frame blend glow from its bright frame: frame 1 is the same glow with alpha times `dim`, frame 2 the glow
/// itself. Colour is untouched, so the engine's cross-fade only changes intensity.
pub fn glow_strip(bright: &Rgba, frames: u32, dim: f64) -> Result<Rgba> {
    if frames != 2 {
        return Err(Error::Invalid(format!("a blend glow has two frames, not {frames}")));
    }
    if !(0.0..1.0).contains(&dim) {
        return Err(Error::Invalid(format!("the dim ratio must be below 1, not {dim}")));
    }
    let mut low = bright.clone();
    for p in low.data.chunks_exact_mut(4) {
        p[3] = (f64::from(p[3]) * dim).round() as u8;
        if p[3] == 0 {
            p.copy_from_slice(&[0, 0, 0, 0]);
        }
    }
    build_strip(&[low, bright.clone()], (bright.width, bright.height))
}

/// Mean absolute premultiplied difference between the image and itself shifted left by `s` columns, over the
/// columns that do not wrap.
fn shift_error(pre: &Rgba, s: usize) -> f64 {
    let (w, h) = (pre.width as usize, pre.height as usize);
    let mut sum = 0u64;
    for y in 0..h {
        let row = &pre.data[y * w * 4..(y + 1) * w * 4];
        for x in 0..w - s {
            for c in 0..4 {
                sum += u64::from(row[x * 4 + c].abs_diff(row[(x + s) * 4 + c]));
            }
        }
    }
    sum as f64 / ((w - s).max(1) * h * 4) as f64
}

/// The horizontal period of a repeating picture: the shift between an eighth and half of the width at which it
/// matches itself best, when that match is clearly better than a typical shift (below 0.6 of the median error).
/// `None` for a picture that does not repeat.
fn pattern_period(pre: &Rgba) -> Option<usize> {
    let w = pre.width as usize;
    let lo = (w / 8).max(2);
    let hi = w / 2;
    if hi <= lo {
        return None;
    }
    let errs: Vec<(usize, f64)> = (lo..=hi).map(|s| (s, shift_error(pre, s))).collect();
    let (p, best) = errs.iter().copied().fold((0, f64::MAX), |acc, (s, e)| if e < acc.1 - 1e-9 { (s, e) } else { acc });
    let med = median(&errs.iter().map(|(_, e)| *e).collect::<Vec<_>>());
    (best < 0.6 * med).then_some(p)
}

/// Make an image seamless across its width.
///
/// A repeating pattern (a period found by [`pattern_period`]) is first cropped to a whole number of periods and
/// resampled back to the width, so the width holds the pattern exactly `n` times. Then, within a quarter of the
/// width from either edge, the image is blended with its copy shifted by a whole number of periods (half the width
/// for a picture that does not repeat), so the last column runs into the first. For a pattern both copies are in
/// phase and nothing ghosts. The middle half keeps its pixels.
pub fn tileable_x(img: &Rgba) -> Result<Rgba> {
    let (w, h) = (img.width as usize, img.height as usize);
    if w < 4 {
        return Err(Error::Invalid(format!("a {w} px wide image cannot be made seamless")));
    }
    let mut pre = premultiply(img);
    let mut shift = w / 2;
    if let Some(p) = pattern_period(&pre) {
        let n = (w / p).max(1);
        let crop = n * p;
        if crop < w {
            let cut = img.crop(0, 0, crop as u32, img.height)?;
            pre = premultiply(&resize_lanczos3(&cut, img.width, img.height)?);
        }
        let period = w as f64 / n as f64;
        shift = ((((w as f64 / 2.0) / period).round().max(1.0) * period).round() as usize).clamp(1, w - 1);
    }
    let band = (w as f64 / 4.0).max(1.0);
    let mut out = Rgba::new(img.width, img.height);
    for x in 0..w {
        let d = (x as f64 + 0.5).min(w as f64 - x as f64 - 0.5);
        let t = (d / band).min(1.0);
        let wgt = t * t * (3.0 - 2.0 * t);
        let xs = (x + shift) % w;
        for y in 0..h {
            let a = &pre.data[(y * w + x) * 4..(y * w + x) * 4 + 4];
            let b = &pre.data[(y * w + xs) * 4..(y * w + xs) * 4 + 4];
            let mut px = [0u8; 4];
            for c in 0..4 {
                px[c] = (wgt * f64::from(a[c]) + (1.0 - wgt) * f64::from(b[c])).round().clamp(0.0, 255.0) as u8;
            }
            out.set_px(x as u32, y as u32, px);
        }
    }
    Ok(unpremultiply(&out))
}

/// Shift a seamless image right by `dx` pixels (fractional, wrapping), linear between columns.
fn shift_wrap(pre: &Rgba, dx: f64) -> Rgba {
    let (w, h) = (pre.width as usize, pre.height as usize);
    let mut out = Rgba::new(pre.width, pre.height);
    for x in 0..w {
        let src = (x as f64 - dx).rem_euclid(w as f64);
        let x0 = src.floor() as usize % w;
        let x1 = (x0 + 1) % w;
        let f = src - src.floor();
        for y in 0..h {
            let a = &pre.data[(y * w + x0) * 4..(y * w + x0) * 4 + 4];
            let b = &pre.data[(y * w + x1) * 4..(y * w + x1) * 4 + 4];
            let mut px = [0u8; 4];
            for c in 0..4 {
                px[c] = ((1.0 - f) * f64::from(a[c]) + f * f64::from(b[c])).round().clamp(0.0, 255.0) as u8;
            }
            out.set_px(x as u32, y as u32, px);
        }
    }
    out
}

/// A scrolling strip: the image made seamless, then moved right by one frame's share of its width per frame, so the
/// frame after the last is the first again.
pub fn scroll_strip(img: &Rgba, frames: u32) -> Result<Rgba> {
    if frames < 2 {
        return Err(Error::Invalid("a scrolling strip needs at least two frames".into()));
    }
    let pre = premultiply(&tileable_x(img)?);
    let step = f64::from(img.width) / f64::from(frames);
    let out: Vec<Rgba> = (0..frames).map(|k| unpremultiply(&shift_wrap(&pre, step * f64::from(k)))).collect();
    build_strip(&out, (img.width, img.height))
}

/// Centre and farthest visible distance of the subject (pixels with alpha at least `min_alpha`), measured from the
/// centre of its bounding box.
fn subject_circle(img: &Rgba, min_alpha: u8) -> Option<(f64, f64, f64)> {
    let b = img.alpha_bbox(min_alpha)?;
    let cx = f64::from(b[0] + b[2]) / 2.0;
    let cy = f64::from(b[1] + b[3]) / 2.0;
    let mut r2: f64 = 0.0;
    for y in b[1]..b[3] {
        for x in b[0]..b[2] {
            if img.alpha(x, y) >= min_alpha {
                // the far corner of the pixel
                let dx = (f64::from(x) + 0.5 - cx).abs() + 0.5;
                let dy = (f64::from(y) + 0.5 - cy).abs() + 0.5;
                r2 = r2.max(dx * dx + dy * dy);
            }
        }
    }
    Some((cx, cy, r2.sqrt()))
}

/// A turning strip: the subject is centred on its bounding box, scaled so its farthest visible pixel stays
/// `margin` inside the frame at every angle, and turned clockwise by `degrees * k / frames` in frame `k`. Each
/// frame is turned at the source resolution and only then reduced, so the small frames stay sharp.
pub fn spin_strip(img: &Rgba, width: u32, height: u32, margin: u32, frames: u32, degrees: f64, min_alpha: u8) -> Result<Rgba> {
    if frames < 2 {
        return Err(Error::Invalid("a turning strip needs at least two frames".into()));
    }
    let room = f64::from(width.min(height)) / 2.0 - f64::from(margin);
    if room < 1.0 {
        return Err(Error::Invalid(format!("margin {margin} leaves no room in a {width}x{height} frame")));
    }
    let (cx, cy, r) = subject_circle(img, min_alpha).ok_or_else(|| Error::Image("no visible subject to turn".into()))?;
    // a square working canvas centred on the subject, just big enough for it at every angle
    let side = (2.0 * r).ceil() as u32 + 2;
    let mut work = Rgba::new(side, side);
    let half = f64::from(side) / 2.0;
    work.paste(img, (half - cx).round() as i64, (half - cy).round() as i64);
    let target = ((f64::from(side) * room / r).round() as u32).clamp(1, width.min(height));
    let mut out = Vec::with_capacity(frames as usize);
    for k in 0..frames {
        let th = (degrees * f64::from(k) / f64::from(frames)).to_radians();
        let (s, c) = th.sin_cos();
        // output pixel p comes from R(-th)(p - centre) + centre: a clockwise turn on screen (y down)
        let m = [c, s, half - c * half - s * half, -s, c, half + s * half - c * half];
        let turned = if k == 0 { work.clone() } else { affine_bicubic(&work, (side, side), m)? };
        let small = resize_lanczos3(&turned, target, target)?;
        let mut frame = Rgba::new(width, height);
        frame.paste(&small, i64::from((width - target) / 2), i64::from((height - target) / 2));
        out.push(frame);
    }
    build_strip(&out, (width, height))
}

/// Fit several frames of one subject into the frame with one shared crop and scale: the union of their visible
/// bounds is scaled to fit `margin` inside the frame and every frame gets exactly that transform. An edit that kept
/// the framing of its base therefore lands where the base does.
pub fn fit_together(frames: &[Rgba], width: u32, height: u32, margin: u32, min_alpha: u8) -> Result<Vec<Rgba>> {
    let first = frames.first().ok_or_else(|| Error::Invalid("no frames to fit".into()))?;
    if frames.iter().any(|f| (f.width, f.height) != (first.width, first.height)) {
        let sizes: Vec<String> = frames.iter().map(|f| format!("{}x{}", f.width, f.height)).collect();
        return Err(Error::Invalid(format!(
            "the key frames must share one source size so one crop and scale fits them all; they are {}",
            sizes.join(", ")
        )));
    }
    if width == 0 || height == 0 || margin * 2 >= width || margin * 2 >= height {
        return Err(Error::Invalid(format!("margin {margin} leaves no room in a {width}x{height} canvas")));
    }
    let mut u: Option<[u32; 4]> = None;
    for f in frames {
        let b = f.alpha_bbox(min_alpha).ok_or_else(|| Error::Image("a key frame has no visible subject".into()))?;
        u = Some(match u {
            None => b,
            Some(o) => [o[0].min(b[0]), o[1].min(b[1]), o[2].max(b[2]), o[3].max(b[3])],
        });
    }
    let u = u.expect("at least one frame");
    let (uw, uh) = (u[2] - u[0], u[3] - u[1]);
    let (aw, ah) = (width - 2 * margin, height - 2 * margin);
    let scale = (f64::from(aw) / f64::from(uw)).min(f64::from(ah) / f64::from(uh));
    let tw = ((f64::from(uw) * scale).round() as u32).clamp(1, aw);
    let th = ((f64::from(uh) * scale).round() as u32).clamp(1, ah);
    frames
        .iter()
        .map(|f| {
            let crop = f.crop(u[0], u[1], uw, uh)?;
            let scaled = resize_lanczos3(&crop, tw, th)?;
            let mut out = Rgba::new(width, height);
            out.paste(&scaled, i64::from((width - tw) / 2), i64::from((height - th) / 2));
            Ok(out)
        })
        .collect()
}

// ---------------------------------------------------------------- measurement

/// What changes between the frames of a strip.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct FrameMetrics {
    /// Distance in pixels of each frame's alpha-weighted centroid from frame 1's (frame 1 is 0).
    pub centroid_drift_px: Vec<f64>,
    /// Overlap (intersection over union) of each frame's silhouette (alpha at least 64) with frame 1's.
    pub silhouette_iou: Vec<f64>,
    /// Distance in RGB levels of each frame's alpha-weighted mean colour from frame 1's.
    pub palette_drift: Vec<f64>,
    /// Mean absolute premultiplied difference (0 to 255) from frame k to frame k+1, the last one to frame 1.
    pub steps: Vec<f64>,
    /// The step from the last frame back to the first over the median of the other steps (strips of three or more).
    pub seam_ratio: Option<f64>,
    /// The largest step other than the seam over the median of those steps (strips of four or more).
    pub jump_ratio: Option<f64>,
}

fn centroid(f: &Rgba) -> Option<(f64, f64)> {
    let (mut sx, mut sy, mut s) = (0.0, 0.0, 0.0);
    for y in 0..f.height {
        for x in 0..f.width {
            let a = f64::from(f.alpha(x, y));
            sx += a * f64::from(x);
            sy += a * f64::from(y);
            s += a;
        }
    }
    (s > 0.0).then(|| (sx / s, sy / s))
}

fn mean_rgb(f: &Rgba) -> [f64; 3] {
    let mut acc = [0.0; 3];
    let mut s = 0.0;
    for p in f.data.chunks_exact(4) {
        let a = f64::from(p[3]);
        for c in 0..3 {
            acc[c] += a * f64::from(p[c]);
        }
        s += a;
    }
    if s > 0.0 {
        acc.map(|v| v / s)
    } else {
        acc
    }
}

fn iou(a: &Rgba, b: &Rgba) -> f64 {
    let (mut i, mut u) = (0u64, 0u64);
    for (p, q) in a.data.chunks_exact(4).zip(b.data.chunks_exact(4)) {
        let (x, y) = (p[3] >= 64, q[3] >= 64);
        i += u64::from(x && y);
        u += u64::from(x || y);
    }
    if u == 0 {
        1.0
    } else {
        i as f64 / u as f64
    }
}

fn mean_diff(a: &Rgba, b: &Rgba) -> f64 {
    let (pa, pb) = (premultiply(a), premultiply(b));
    let n = pa.data.len().max(1) as f64;
    pa.data.iter().zip(pb.data.iter()).map(|(x, y)| (f64::from(*x) - f64::from(*y)).abs()).sum::<f64>() / n
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(|a, b| a.total_cmp(b));
    let n = s.len();
    if n == 0 {
        0.0
    } else if n % 2 == 1 {
        s[n / 2]
    } else {
        (s[n / 2 - 1] + s[n / 2]) / 2.0
    }
}

/// Measure a strip's frames (at least one).
pub fn frame_metrics(frames: &[Rgba]) -> FrameMetrics {
    let n = frames.len();
    let c0 = frames.first().and_then(centroid);
    let m0 = frames.first().map(mean_rgb).unwrap_or([0.0; 3]);
    let mut centroid_drift_px = Vec::with_capacity(n);
    let mut silhouette_iou = Vec::with_capacity(n);
    let mut palette_drift = Vec::with_capacity(n);
    for f in frames {
        centroid_drift_px.push(match (c0, centroid(f)) {
            (Some(a), Some(b)) => ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt(),
            _ => 0.0,
        });
        silhouette_iou.push(iou(&frames[0], f));
        let m = mean_rgb(f);
        palette_drift.push(((m[0] - m0[0]).powi(2) + (m[1] - m0[1]).powi(2) + (m[2] - m0[2]).powi(2)).sqrt());
    }
    let steps: Vec<f64> = if n < 2 { Vec::new() } else { (0..n).map(|i| mean_diff(&frames[i], &frames[(i + 1) % n])).collect() };
    let inner = if n >= 2 { &steps[..n - 1] } else { &steps[..] };
    let med = median(inner);
    let seam_ratio = (n >= 3 && med > 0.0).then(|| steps[n - 1] / med);
    let jump_ratio = (n >= 4 && med > 0.0).then(|| inner.iter().copied().fold(0.0, f64::max) / med);
    FrameMetrics { centroid_drift_px, silhouette_iou, palette_drift, steps, seam_ratio, jump_ratio }
}

/// Limits of the `animation` check for one motion, set from the vanilla strips of that kind.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Limits {
    /// Centroid drift allowed, as a share of the frame diagonal, with a floor of 1.5 px; twice this is an error.
    pub drift_share: Option<f64>,
    /// Silhouette overlap below which a frame is reported (warning), and below which it is an error.
    pub iou: Option<(f64, f64)>,
    /// Mean-colour drift in RGB levels: warning, error.
    pub palette: Option<(f64, f64)>,
    /// Loop seam ratio: warning, error.
    pub seam: (f64, f64),
}

/// The limits of a motion. Vanilla: key-frame strips drift 5.0 px at p90 on 141x115 (2.8 percent of the
/// diagonal) with palette drift 1.65 at p90 and silhouette overlap 0.86 at p10; turning icons drift 2.2 px at p90 on
/// 26x26 (5.9 percent), overlap 0.83 at p10, palette drift 10.5 at p90; scroll strips keep their palette within
/// 0.62 levels; the largest vanilla seam ratio is 2.7.
pub fn limits(motion: Motion) -> Limits {
    match motion {
        Motion::BlendGlow => Limits { drift_share: Some(0.02), iou: None, palette: Some((12.0, 40.0)), seam: (3.0, 6.0) },
        Motion::BlendKeyframes => Limits { drift_share: Some(0.04), iou: Some((0.75, 0.5)), palette: Some((24.0, 60.0)), seam: (3.0, 6.0) },
        Motion::Scroll => Limits { drift_share: None, iou: None, palette: Some((12.0, 40.0)), seam: (3.0, 6.0) },
        Motion::Spin => Limits { drift_share: Some(0.09), iou: Some((0.7, 0.45)), palette: Some((20.0, 50.0)), seam: (3.0, 6.0) },
    }
}

/// Mean step below which a strip does not move at all.
pub const STATIC_STEP: f64 = 0.25;

// ---------------------------------------------------------------- preview

/// One frame of the preview loop with its display time in centiseconds.
pub struct LoopFrame {
    /// The picture.
    pub image: Rgba,
    /// Display time in hundredths of a second.
    pub delay_cs: u16,
}

fn mix(a: &Rgba, b: &Rgba, t: f64) -> Rgba {
    let (pa, pb) = (premultiply(a), premultiply(b));
    let mut out = pa.clone();
    for (o, (x, y)) in out.data.iter_mut().zip(pa.data.iter().zip(pb.data.iter())) {
        *o = ((1.0 - t) * f64::from(*x) + t * f64::from(*y)).round().clamp(0.0, 255.0) as u8;
    }
    unpremultiply(&out)
}

/// The loop as the game plays it, for a preview. Strips show each frame for `1 / fps` seconds. Blended strips are
/// shown as the engine cross-fades them, approximated by linear blends at 20 pictures a second (the shader itself
/// was not traced). The pause on loop is added to the last picture.
pub fn preview_loop(frames: &[Rgba], spec: &AnimationSpec) -> Result<Vec<LoopFrame>> {
    if frames.is_empty() {
        return Err(Error::Invalid("no frames to preview".into()));
    }
    let fps = spec.fps_value()?;
    let mut out = Vec::new();
    if spec.blends() && frames.len() > 1 {
        let per = ((20.0 / fps).round() as usize).clamp(1, 200);
        for i in 0..frames.len() {
            let next = &frames[(i + 1) % frames.len()];
            for s in 0..per {
                out.push(LoopFrame { image: mix(&frames[i], next, s as f64 / per as f64), delay_cs: 5 });
            }
        }
    } else {
        let d = ((100.0 / fps).round() as u16).max(2);
        out.extend(frames.iter().map(|f| LoopFrame { image: f.clone(), delay_cs: d }));
    }
    let pause = (spec.pause_value() * 100.0).round() as u16;
    if let Some(last) = out.last_mut() {
        last.delay_cs = last.delay_cs.saturating_add(pause);
    }
    Ok(out)
}

fn on_matte(img: &Rgba, matte: [u8; 3], scale: u32) -> Vec<u8> {
    let s = scale.max(1);
    let (w, h) = (img.width * s, img.height * s);
    let mut rgb = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            let p = img.px(x / s, y / s);
            let a = u32::from(p[3]);
            for c in 0..3 {
                rgb.push(((u32::from(p[c]) * a + u32::from(matte[c]) * (255 - a) + 127) / 255) as u8);
            }
        }
    }
    rgb
}

/// An endlessly looping GIF of the preview loop, each picture over a flat `matte` and enlarged `scale` times with
/// nearest-neighbour sampling (a GIF has no partial transparency). Deterministic for the same input.
pub fn encode_gif(frames: &[LoopFrame], matte: [u8; 3], scale: u32) -> Result<Vec<u8>> {
    let first = frames.first().ok_or_else(|| Error::Invalid("no frames to encode".into()))?;
    let s = scale.max(1);
    let (w, h) = (first.image.width * s, first.image.height * s);
    if w > u32::from(u16::MAX) || h > u32::from(u16::MAX) {
        return Err(Error::Invalid(format!("a {w}x{h} GIF is too large")));
    }
    let mut bytes = Vec::new();
    {
        let mut enc = gif::Encoder::new(&mut bytes, w as u16, h as u16, &[]).map_err(|e| Error::Image(format!("cannot start the GIF: {e}")))?;
        enc.set_repeat(gif::Repeat::Infinite).map_err(|e| Error::Image(format!("cannot loop the GIF: {e}")))?;
        for f in frames {
            if (f.image.width * s, f.image.height * s) != (w, h) {
                return Err(Error::Invalid("the preview frames differ in size".into()));
            }
            let rgb = on_matte(&f.image, matte, s);
            let mut frame = gif::Frame::from_rgb_speed(w as u16, h as u16, &rgb, 10);
            frame.delay = f.delay_cs;
            enc.write_frame(&frame).map_err(|e| Error::Image(format!("cannot write a GIF frame: {e}")))?;
        }
    }
    Ok(bytes)
}

/// The frames side by side over `matte`, `gap` pixels apart, enlarged `scale` times.
pub fn contact_sheet(frames: &[Rgba], matte: [u8; 3], scale: u32, gap: u32) -> Result<Rgba> {
    let first = frames.first().ok_or_else(|| Error::Invalid("no frames".into()))?;
    let s = scale.max(1);
    let (fw, fh) = (first.width * s, first.height * s);
    let n = frames.len() as u32;
    let mut sheet = Rgba::filled(n * fw + (n + 1) * gap, fh + 2 * gap, [matte[0], matte[1], matte[2], 255]);
    for (i, f) in frames.iter().enumerate() {
        let rgb = on_matte(f, matte, s);
        let x0 = gap + i as u32 * (fw + gap);
        for y in 0..fh {
            for x in 0..fw {
                let k = ((y * fw + x) * 3) as usize;
                sheet.set_px(x0 + x, gap + y, [rgb[k], rgb[k + 1], rgb[k + 2], 255]);
            }
        }
    }
    Ok(sheet)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disc(w: u32, h: u32, cx: f64, cy: f64, r: f64, rgb: [u8; 3]) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                let d = ((f64::from(x) + 0.5 - cx).powi(2) + (f64::from(y) + 0.5 - cy).powi(2)).sqrt();
                if d <= r {
                    img.set_px(x, y, [rgb[0], rgb[1], rgb[2], 255]);
                }
            }
        }
        img
    }

    /// A cog-like test subject: a disc with a notch, so a turn is visible.
    fn notched(w: u32) -> Rgba {
        let mut img = disc(w, w, f64::from(w) / 2.0, f64::from(w) / 2.0, f64::from(w) * 0.4, [200, 120, 40]);
        for y in 0..w / 2 {
            for x in w / 2 - 2..w / 2 + 2 {
                img.set_px(x, y, [0, 0, 0, 0]);
            }
        }
        img
    }

    fn spec(motion: Motion, fps: &str) -> AnimationSpec {
        AnimationSpec {
            motion,
            fps: fps.into(),
            looping: true,
            play_on_show: true,
            pause_on_loop: "0.0".into(),
            effect_file: matches!(motion, Motion::BlendGlow | Motion::BlendKeyframes).then(|| BLEND_EFFECT.to_string()),
            always_transparent: matches!(motion, Motion::BlendGlow | Motion::BlendKeyframes),
            load_type: (!matches!(motion, Motion::BlendGlow | Motion::BlendKeyframes)).then(|| "INGAME".to_string()),
            transparence_check: !matches!(motion, Motion::BlendGlow | Motion::BlendKeyframes),
        }
    }

    #[test]
    fn sprite_blocks_follow_the_vanilla_field_order() {
        let glow = sprite_block("GFX_x_glow", "gfx/interface/x.dds", 2, &spec(Motion::BlendGlow, "1")).unwrap();
        assert_eq!(
            glow,
            "\tframeAnimatedSpriteType = {\n\t\tname = \"GFX_x_glow\"\n\t\ttexturefile = \"gfx/interface/x.dds\"\n\t\tnoOfFrames = 2\n\t\tanimation_rate_fps = 1\n\t\tlooping = yes\n\t\tplay_on_show = yes\n\t\tpause_on_loop = 0.0\n\t\talwaystransparent = yes\n\t\teffectFile = \"gfx/FX/buttonstate_blendframes.lua\"\n\t}\n"
        );
        let spin = sprite_block("GFX_y", "gfx/y.dds", 20, &spec(Motion::Spin, "7")).unwrap();
        assert_eq!(
            spin,
            "\tframeAnimatedSpriteType = {\n\t\tname = \"GFX_y\"\n\t\ttexturefile = \"gfx/y.dds\"\n\t\tnoOfFrames = 20\n\t\tloadType = \"INGAME\"\n\t\ttransparencecheck = yes\n\t\tanimation_rate_fps = 7\n\t\tlooping = yes\n\t\tplay_on_show = yes\n\t\tpause_on_loop = 0.0\n\t}\n"
        );
        let mut bad = spec(Motion::Spin, "7");
        bad.fps = "fast".into();
        assert!(sprite_block("GFX_y", "gfx/y.dds", 20, &bad).is_err());
        bad.fps = "7\"}".into();
        assert!(sprite_block("GFX_y", "gfx/y.dds", 20, &bad).is_err());
    }

    #[test]
    fn black_becomes_alpha_and_composites_back() {
        let mut img = Rgba::filled(4, 1, [0, 0, 0, 255]);
        img.set_px(1, 0, [255, 128, 0, 255]);
        img.set_px(2, 0, [100, 50, 0, 255]);
        let out = black_to_alpha(&img);
        assert_eq!(out.px(0, 0), [0, 0, 0, 0]);
        assert_eq!(out.px(1, 0), [255, 128, 0, 255]);
        let p = out.px(2, 0);
        assert_eq!(p[3], 100);
        assert_eq!([p[0], p[1]], [255, 128]);
    }

    #[test]
    fn the_glow_strip_dims_frame_one_and_keeps_frame_two() {
        let bright = disc(20, 20, 10.0, 10.0, 7.0, [250, 200, 60]);
        let strip = glow_strip(&bright, 2, GLOW_DIM_RATIO).unwrap();
        let f = strip.split_frames(2).unwrap();
        assert_eq!(f[1], bright);
        assert_eq!(f[0].px(10, 10), [250, 200, 60, 43]);
        assert!(glow_strip(&bright, 3, 0.2).is_err());
        let m = frame_metrics(&f);
        assert!(m.centroid_drift_px[1] < 0.01 && m.palette_drift[1] < 0.5, "{m:?}");
    }

    #[test]
    fn a_scroll_loops_without_a_seam_and_keeps_its_palette() {
        let mut img = Rgba::new(36, 12);
        for y in 0..12 {
            for x in 0..36 {
                let v = if (x / 6) % 2 == 0 { 200 } else { 60 };
                img.set_px(x, y, [v, v / 2, 30, 255]);
            }
        }
        // a hard edge at the wrap that only the seamless pass hides
        for y in 0..12 {
            img.set_px(35, y, [255, 255, 255, 255]);
        }
        let strip = scroll_strip(&img, 9).unwrap();
        assert_eq!((strip.width, strip.height), (36 * 9, 12));
        let frames = strip.split_frames(9).unwrap();
        let m = frame_metrics(&frames);
        let seam = m.seam_ratio.unwrap();
        assert!((0.5..1.5).contains(&seam), "seam {seam} {:?}", m.steps);
        assert!(m.palette_drift.iter().all(|d| *d < 2.0), "{m:?}");
        // a pattern whose period does not divide half the width is blended with a copy in phase, so nothing ghosts:
        // a repeating picture is left exactly as it was
        let stripes = |w: u32| {
            let mut img = Rgba::new(w, 6);
            for y in 0..6 {
                for x in 0..w {
                    let v = if (x + y) % 8 < 4 { 220 } else { 40 };
                    img.set_px(x, y, [v, 90, 30, 255]);
                }
            }
            img
        };
        let whole = stripes(32);
        assert_eq!(tileable_x(&whole).unwrap(), whole, "a width of whole periods is already seamless");
        // 36 px is four and a half periods: the pattern is fitted to four periods of 9 px, so it repeats exactly
        let odd = stripes(36);
        let seamless = premultiply(&tileable_x(&odd).unwrap());
        let before = shift_error(&premultiply(&odd), 9);
        assert!(shift_error(&seamless, 9) < before / 2.0, "{} vs {before}", shift_error(&seamless, 9));
        let wrap: f64 = (0..6u32).map(|y| f64::from(seamless.px(35, y)[0].abs_diff(seamless.px(0, y)[0]))).sum::<f64>() / 6.0;
        let inner: f64 = (0..6u32).map(|y| f64::from(seamless.px(17, y)[0].abs_diff(seamless.px(18, y)[0]))).sum::<f64>() / 6.0;
        assert!(wrap <= inner + 60.0, "the wrap is no harder an edge than an inner one: {wrap} vs {inner}");
        // the next frame after the last is the first again
        let pre = premultiply(&tileable_x(&img).unwrap());
        assert_eq!(unpremultiply(&shift_wrap(&pre, 36.0)), frames[0]);
    }

    #[test]
    fn a_spin_fits_by_radius_turns_a_full_circle_and_stays_centred() {
        let src = notched(200);
        let strip = spin_strip(&src, 26, 26, 1, 20, 360.0, 16).unwrap();
        assert_eq!((strip.width, strip.height), (520, 26));
        let frames = strip.split_frames(20).unwrap();
        for f in &frames {
            let b = f.alpha_bbox(16).unwrap();
            assert!(b[0] >= 1 && b[1] >= 1 && b[2] <= 25 && b[3] <= 25, "{b:?}");
        }
        let m = frame_metrics(&frames);
        assert!(m.centroid_drift_px.iter().all(|d| *d < 1.5), "{:?}", m.centroid_drift_px);
        assert!(m.silhouette_iou.iter().all(|v| *v > 0.8), "{:?}", m.silhouette_iou);
        assert!(m.seam_ratio.unwrap() < 1.6, "{m:?}");
        assert!(m.steps.iter().all(|s| *s > STATIC_STEP), "the notch moves every frame: {:?}", m.steps);
        assert!(spin_strip(&Rgba::new(10, 10), 26, 26, 1, 20, 360.0, 16).is_err());
    }

    #[test]
    fn key_frames_share_one_crop_and_scale() {
        let a = disc(300, 300, 150.0, 150.0, 60.0, [200, 50, 50]);
        let b = disc(300, 300, 150.0, 150.0, 90.0, [200, 50, 50]);
        let fitted = fit_together(&[a.clone(), b], 100, 100, 2, 16).unwrap();
        // the larger frame fills the canvas; the smaller one keeps its relative size, both on one centre
        let ba = fitted[0].alpha_bbox(16).unwrap();
        let bb = fitted[1].alpha_bbox(16).unwrap();
        assert!(bb[2] - bb[0] > 90 && ba[2] - ba[0] < 70, "{ba:?} {bb:?}");
        let m = frame_metrics(&fitted);
        assert!(m.centroid_drift_px[1] < 1.0, "{m:?}");
        assert!(fit_together(&[a, Rgba::new(10, 10)], 100, 100, 2, 16).is_err());
    }

    #[test]
    fn metrics_see_a_frame_that_jumps_and_a_strip_that_stands_still() {
        let base = disc(30, 30, 15.0, 15.0, 8.0, [100, 200, 100]);
        let moved = disc(30, 30, 22.0, 15.0, 8.0, [100, 200, 100]);
        let m = frame_metrics(&[base.clone(), base.clone(), moved, base.clone()]);
        assert!(m.centroid_drift_px[2] > 6.0);
        assert!(m.silhouette_iou[2] < 0.5);
        let still = frame_metrics(&[base.clone(), base.clone(), base.clone()]);
        assert!(still.steps.iter().all(|s| *s < STATIC_STEP));
        let recolour = disc(30, 30, 15.0, 15.0, 8.0, [250, 20, 20]);
        assert!(frame_metrics(&[base, recolour]).palette_drift[1] > 100.0);
    }

    /// Vanilla strips of each kind pass the limits of their motion: p90 drift, p10 overlap, p90 palette drift and the
    /// largest seam from `data/vanilla-measurement.json` all sit inside the warning thresholds.
    #[test]
    fn the_limits_admit_the_vanilla_strips() {
        let m = crate::measurement::data();
        for (id, motion, frame) in [
            ("anim_blend_keyframes", Motion::BlendKeyframes, [141.0_f64, 115.0]),
            ("anim_spin", Motion::Spin, [26.0, 26.0]),
            ("anim_scroll", Motion::Scroll, [72.0, 72.0]),
        ] {
            let lim = limits(motion);
            let n = |k: &str| m.animation_number(id, k).unwrap_or_else(|| panic!("{id}: {k}"));
            if let Some(share) = lim.drift_share {
                let allowed = (share * (frame[0] * frame[0] + frame[1] * frame[1]).sqrt()).max(1.5);
                assert!(n("p90_centroid_drift_px_max") < allowed, "{id}");
            }
            if let Some((warn, _)) = lim.iou {
                assert!(n("p10_silhouette_iou_min") > warn, "{id}");
            }
            if let Some((warn, _)) = lim.palette {
                assert!(n("p90_palette_drift_max") < warn, "{id}");
            }
            assert!(n("max_loop_seam_ratio") < lim.seam.0, "{id}");
        }
    }

    #[test]
    fn the_preview_loop_blends_or_steps_and_encodes_as_a_looping_gif() {
        let a = disc(16, 16, 8.0, 8.0, 5.0, [255, 200, 0]);
        let b = glow_strip(&a, 2, 0.17).unwrap().split_frames(2).unwrap();
        let blend = preview_loop(&b, &spec(Motion::BlendGlow, "1")).unwrap();
        assert_eq!(blend.len(), 40, "two frames at 1 fps, 20 pictures a second");
        assert!(blend.iter().all(|f| f.delay_cs == 5));
        let steps = preview_loop(&b, &spec(Motion::Spin, "7")).unwrap();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].delay_cs, 14);
        let gif = encode_gif(&blend, [40, 40, 40], 2).unwrap();
        assert_eq!(&gif[..6], b"GIF89a");
        assert_eq!(gif, encode_gif(&blend, [40, 40, 40], 2).unwrap(), "deterministic");
        let sheet = contact_sheet(&b, [40, 40, 40], 3, 4).unwrap();
        assert_eq!((sheet.width, sheet.height), (2 * 48 + 3 * 4, 48 + 8));
    }
}
