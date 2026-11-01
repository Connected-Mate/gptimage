//! Advisor card: a portrait placed beneath the canonical 65x67 dossier frame.
//!
//! Port of the core of the kit's `create_advisor_icon.py`. The whole source picture is scaled with one uniform
//! factor until it covers the measured opening plus a two pixel bleed under the frame, rotated to the opening's
//! measured tilt, lightly sepia toned (`strength` 0.18 by default), masked to the bleed mask and composited
//! under the template. No pre-crop and no stretch.
//!
//! The template (`advisor_template.png`) is Paradox-derived and is passed in by the caller.
//!
//! Ported: opening detection (largest enclosed transparent component), rotated-rectangle measurement, the
//! placement validators, portrait preparation, the covering geometry, bicubic affine placement (Pillow's own
//! arithmetic, see [`crate::recipes::pillow`]), the bleed mask with its exterior-leak check, and the alpha-coverage
//! audit (any gap or leak fails closed, as in the Python).
//!
//! Left out on purpose: the nearest-neighbour 4x review preview, the placement-study contact sheet, the 8x
//! alignment overlay, the metadata JSON and the DDS write-and-reopen check. They are review artefacts; the engine
//! writes the DDS with [`crate::dds::write_a8r8g8b8`] and keeps its own ledger. Trigonometry uses the crate's
//! series, so the measured angle can differ from Python's by float noise in the last digits only.

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::pillow::{affine_bicubic, alpha_composite, blend_u8, colorize_lut, det_cos, luma};
use crate::resize::det_sin;

/// Card size.
pub const CARD_SIZE: (u32, u32) = (65, 67);
/// Smallest rotation accepted without the explicit zero-rotation exception.
pub const MIN_VISIBLE_ROTATION: f64 = 0.25;
/// Alpha at or below which a template pixel belongs to the opening mask.
pub const OPENING_MASK_ALPHA_THRESHOLD: u8 = 192;
/// Alpha threshold used when measuring the opening's rotated rectangle.
pub const OPENING_GEOMETRY_ALPHA_THRESHOLD: u8 = 128;
/// Largest accepted rotation difference from the measured opening, degrees.
pub const MAX_ALIGNMENT_ERROR: f64 = 0.05;
/// Largest accepted centre offset from the measured opening, pixels.
pub const MAX_OPENING_CENTER_OFFSET: f64 = 0.001;
/// Largest accepted size difference from the measured opening, pixels.
pub const MAX_OPENING_SIZE_ERROR: f64 = 0.01;
/// Pixels the portrait extends under the frame.
pub const UNDER_FRAME_BLEED_PIXELS: i32 = 2;
/// Extra fill margin against resampling edges.
pub const PORTRAIT_EDGE_GUARD_PIXELS: i32 = 1;

/// Placement options. `None` for size or rotation means "use the measured opening", which is what canonical
/// cards require anyway (the validators reject anything else).
#[derive(Debug, Clone, PartialEq)]
pub struct AdvisorParams {
    /// Pre-rotation opening-fill plane, width and height.
    pub portrait_size: Option<(f64, f64)>,
    /// Offset from the measured opening centre; canonical cards use (0, 0).
    pub portrait_offset: (f64, f64),
    /// Rotation in degrees.
    pub rotation: Option<f64>,
    /// Portrait-only sepia blend, 0 to 1.
    pub sepia_strength: f64,
    /// Permit a near-zero rotation (an independently reviewed exception).
    pub allow_zero_rotation: bool,
}

impl Default for AdvisorParams {
    fn default() -> Self {
        AdvisorParams { portrait_size: None, portrait_offset: (0.0, 0.0), rotation: None, sepia_strength: 0.18, allow_zero_rotation: false }
    }
}

/// The measured portrait opening of a template.
#[derive(Debug, Clone, PartialEq)]
pub struct OpeningGeometry {
    /// Centre in card pixels.
    pub center: (f64, f64),
    /// Width and height of the rotated rectangle.
    pub size: (f64, f64),
    /// Rotation in degrees.
    pub rotation: f64,
}

/// Pixel counts that must all be zero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AlphaCoverage {
    /// Opening pixels that are not fully opaque.
    pub opening_alpha_gap_pixels: u32,
    /// Translucent inner-edge pixels (inside the bleed) that are not fully opaque.
    pub inner_edge_alpha_gap_pixels: u32,
    /// Pixels outside the bleed where the template is transparent but the card is not.
    pub exterior_alpha_leak_pixels: u32,
}

impl AlphaCoverage {
    /// True when there is no gap and no leak.
    pub fn is_clean(&self) -> bool {
        *self == AlphaCoverage::default()
    }
}

/// A finished card.
#[derive(Debug, Clone, PartialEq)]
pub struct AdvisorCard {
    /// The 65x67 card.
    pub card: Rgba,
    /// The opening that was measured.
    pub opening: OpeningGeometry,
    /// The coverage audit (clean, or the call would have failed).
    pub coverage: AlphaCoverage,
}

fn check_template(template: &Rgba) -> Result<()> {
    if (template.width, template.height) != CARD_SIZE {
        return Err(Error::Invalid(format!(
            "template must be {}x{}, got {}x{}",
            CARD_SIZE.0, CARD_SIZE.1, template.width, template.height
        )));
    }
    Ok(())
}

/// The largest transparent component (alpha at or below `threshold`) that does not touch the image edge.
pub fn find_enclosed_opening(template: &Rgba, threshold: u8) -> Result<Vec<(i32, i32)>> {
    let (w, h) = (template.width as i32, template.height as i32);
    let mut seen = vec![false; (w * h) as usize];
    let mut best: Vec<(i32, i32)> = Vec::new();
    for sy in 0..h {
        for sx in 0..w {
            let idx = (sy * w + sx) as usize;
            if seen[idx] || template.alpha(sx as u32, sy as u32) > threshold {
                continue;
            }
            seen[idx] = true;
            let mut component = vec![(sx, sy)];
            let mut frontier = vec![(sx, sy)];
            let mut touches_edge = false;
            while let Some((x, y)) = frontier.pop() {
                if x == 0 || y == 0 || x == w - 1 || y == h - 1 {
                    touches_edge = true;
                }
                for (nx, ny) in [(x - 1, y), (x + 1, y), (x, y - 1), (x, y + 1)] {
                    if nx < 0 || ny < 0 || nx >= w || ny >= h {
                        continue;
                    }
                    let ni = (ny * w + nx) as usize;
                    if !seen[ni] && template.alpha(nx as u32, ny as u32) <= threshold {
                        seen[ni] = true;
                        component.push((nx, ny));
                        frontier.push((nx, ny));
                    }
                }
            }
            if !touches_edge && component.len() > best.len() {
                best = component;
            }
        }
    }
    if best.is_empty() {
        return Err(Error::Image("advisor template has no enclosed transparent portrait opening".into()));
    }
    Ok(best)
}

fn opening_mask(template: &Rgba) -> Result<Vec<bool>> {
    let mut mask = vec![false; template.width as usize * template.height as usize];
    for (x, y) in find_enclosed_opening(template, OPENING_MASK_ALPHA_THRESHOLD)? {
        mask[y as usize * template.width as usize + x as usize] = true;
    }
    Ok(mask)
}

/// The opening grown by `bleed` pixels (a square max filter), refused if it would reach the transparent exterior.
pub fn bleed_mask(template: &Rgba, bleed: i32) -> Result<Vec<bool>> {
    if bleed < 0 {
        return Err(Error::Invalid("advisor portrait bleed must not be negative".into()));
    }
    let opening = opening_mask(template)?;
    if bleed == 0 {
        return Ok(opening);
    }
    let (w, h) = (template.width as i32, template.height as i32);
    let mut out = vec![false; opening.len()];
    for y in 0..h {
        for x in 0..w {
            'window: for dy in -bleed..=bleed {
                for dx in -bleed..=bleed {
                    let (nx, ny) = (x + dx, y + dy);
                    if nx >= 0 && ny >= 0 && nx < w && ny < h && opening[(ny * w + nx) as usize] {
                        out[(y * w + x) as usize] = true;
                        break 'window;
                    }
                }
            }
        }
    }
    for y in 0..h {
        for x in 0..w {
            let i = (y * w + x) as usize;
            if out[i] && !opening[i] && template.alpha(x as u32, y as u32) == 0 {
                return Err(Error::Image(
                    "advisor under-frame bleed reaches transparent exterior pixels; use a compatible canonical template".into(),
                ));
            }
        }
    }
    Ok(out)
}

/// Count opening gaps, translucent inner-edge gaps and exterior spills of a finished card.
pub fn audit_alpha_coverage(card: &Rgba, template: &Rgba) -> Result<AlphaCoverage> {
    let opening = opening_mask(template)?;
    let bleed = bleed_mask(template, UNDER_FRAME_BLEED_PIXELS)?;
    let mut cov = AlphaCoverage::default();
    for y in 0..template.height {
        for x in 0..template.width {
            let i = y as usize * template.width as usize + x as usize;
            let ta = template.alpha(x, y);
            let ca = card.alpha(x, y);
            if opening[i] && ca < 255 {
                cov.opening_alpha_gap_pixels += 1;
            } else if bleed[i] && ta > 0 && ta < 255 && ca < 255 {
                cov.inner_edge_alpha_gap_pixels += 1;
            }
            if !bleed[i] && ta == 0 && ca > 0 {
                cov.exterior_alpha_leak_pixels += 1;
            }
        }
    }
    Ok(cov)
}

/// Measure the opening as the minimum-area rotated rectangle that is taller than wide: a coarse sweep of
/// -15 to 15 degrees in 0.1 steps, then a 0.01 step refinement around the best coarse angle.
pub fn measure_opening_geometry(template: &Rgba) -> Result<OpeningGeometry> {
    check_template(template)?;
    let opening = find_enclosed_opening(template, OPENING_GEOMETRY_ALPHA_THRESHOLD)?;
    // (area, width, height, centre x, centre y)
    let at_angle = |angle: f64| -> (f64, f64, f64, f64, f64) {
        let rad = angle.to_radians();
        let (c, s) = (det_cos(rad), det_sin(rad));
        let (mut x_lo, mut x_hi, mut y_lo, mut y_hi) = (f64::MAX, f64::MIN, f64::MAX, f64::MIN);
        for (x, y) in &opening {
            let (x, y) = (f64::from(*x), f64::from(*y));
            let xv = x * c + y * s;
            let yv = -x * s + y * c;
            x_lo = x_lo.min(xv);
            x_hi = x_hi.max(xv);
            y_lo = y_lo.min(yv);
            y_hi = y_hi.max(yv);
        }
        let (w, h) = (x_hi - x_lo, y_hi - y_lo);
        (w * h, w, h, (x_lo + x_hi) / 2.0, (y_lo + y_hi) / 2.0)
    };
    // Python's tuple min: smallest area, then smallest |angle|, then smallest angle.
    let pick = |angles: Vec<f64>| -> Option<(f64, f64, f64, f64, f64, f64)> {
        let mut best: Option<(f64, f64, f64, f64, f64, f64)> = None;
        for angle in angles {
            let (area, w, h, cx, cy) = at_angle(angle);
            if w > h {
                continue;
            }
            let better = match &best {
                None => true,
                Some(b) => (area, angle.abs(), angle) < (b.0, b.1.abs(), b.1),
            };
            if better {
                best = Some((area, angle, w, h, cx, cy));
            }
        }
        best
    };
    let coarse = pick((-150..=150).map(|s| f64::from(s) / 10.0).collect())
        .ok_or_else(|| Error::Image("unable to derive vertical advisor opening geometry".into()))?;
    let (_, angle, w, h, lcx, lcy) = pick((-20..=20).map(|s| coarse.1 + f64::from(s) / 100.0).collect())
        .ok_or_else(|| Error::Image("unable to derive vertical advisor opening geometry".into()))?;
    let rad = angle.to_radians();
    let (c, s) = (det_cos(rad), det_sin(rad));
    Ok(OpeningGeometry { center: (lcx * c - lcy * s, lcx * s + lcy * c), size: (w, h), rotation: angle })
}

fn validate_transform(size: (f64, f64), offset: (f64, f64), rotation: f64, allow_zero_rotation: bool) -> Result<()> {
    if size.0 <= 0.0 || size.1 <= 0.0 {
        return Err(Error::Invalid("portrait size values must be positive".into()));
    }
    if ![size.0, size.1, offset.0, offset.1, rotation].iter().all(|v| v.is_finite()) {
        return Err(Error::Invalid("advisor placement values must be finite".into()));
    }
    if !allow_zero_rotation && rotation.abs() < MIN_VISIBLE_ROTATION {
        return Err(Error::Invalid(
            "advisor dossier portraits require an explicit visible rotation aligned to the template opening; \
             allow_zero_rotation is only for an independently reviewed unrotated exception"
                .into(),
        ));
    }
    Ok(())
}

fn validate_opening_alignment(size: (f64, f64), offset: (f64, f64), rotation: f64, g: &OpeningGeometry) -> Result<()> {
    let delta = (rotation - g.rotation + 90.0).rem_euclid(180.0) - 90.0;
    if delta.abs() > MAX_ALIGNMENT_ERROR {
        return Err(Error::Invalid(format!(
            "advisor portrait rotation must align with the measured dossier opening; selected {rotation:.4}, measured {:.4}, \
             maximum error {MAX_ALIGNMENT_ERROR:.2} degrees",
            g.rotation
        )));
    }
    if offset.0.abs() > MAX_OPENING_CENTER_OFFSET || offset.1.abs() > MAX_OPENING_CENTER_OFFSET {
        return Err(Error::Invalid(
            "advisor portrait centre must stay aligned with the measured opening; canonical cards use a 0 0 offset".into(),
        ));
    }
    if (size.0 - g.size.0).abs() > MAX_OPENING_SIZE_ERROR || (size.1 - g.size.1).abs() > MAX_OPENING_SIZE_ERROR {
        return Err(Error::Invalid(format!(
            "canonical advisor portraits must resize the complete image plane to the measured opening size; selected ({:.6}, {:.6}), measured ({:.6}, {:.6})",
            size.0, size.1, g.size.0, g.size.1
        )));
    }
    Ok(())
}

fn validate_fit_to_opening(size: (f64, f64), center: (f64, f64), rotation: f64, g: &OpeningGeometry) -> Result<()> {
    let delta = (rotation - g.rotation).to_radians();
    let (dc, ds) = (det_cos(delta).abs(), det_sin(delta).abs());
    let half_w = size.0 * dc / 2.0 + size.1 * ds / 2.0;
    let half_h = size.0 * ds / 2.0 + size.1 * dc / 2.0;
    let rad = g.rotation.to_radians();
    let (oc, os) = (det_cos(rad), det_sin(rad));
    let (dx, dy) = (center.0 - g.center.0, center.1 - g.center.1);
    let lx = dx * oc + dy * os;
    let ly = -dx * os + dy * oc;
    let (limit_x, limit_y) = (g.size.0 / 2.0, g.size.1 / 2.0);
    const EPS: f64 = 1e-9; // the series trigonometry may differ from libm by a few ulps
    if lx - half_w < -limit_x - EPS || ly - half_h < -limit_y - EPS || lx + half_w > limit_x + EPS || ly + half_h > limit_y + EPS {
        return Err(Error::Invalid(format!(
            "the advisor opening-fill plane must remain inside the measured rotated dossier opening before masking; \
             local bounds ({:.3}, {:.3}, {:.3}, {:.3}), half-size limits ({limit_x:.3}, {limit_y:.3})",
            lx - half_w,
            ly - half_h,
            lx + half_w,
            ly + half_h
        )));
    }
    Ok(())
}

/// Sepia-blend the portrait RGB (alpha kept): greyscale colorized `(24,18,12)` to `(244,224,188)`, then
/// `Image.blend(rgb, sepia, strength)`.
pub fn prepare_portrait(source: &Rgba, strength: f64) -> Result<Rgba> {
    if !(0.0..=1.0).contains(&strength) {
        return Err(Error::Invalid("sepia strength must be between 0.0 and 1.0".into()));
    }
    let lut = colorize_lut([24, 18, 12], [244, 224, 188]);
    let alpha = strength as f32;
    let mut out = source.clone();
    for p in out.data.chunks_exact_mut(4) {
        let sepia = lut[usize::from(luma(p[0], p[1], p[2]))];
        for c in 0..3 {
            p[c] = blend_u8(p[c], sepia[c], alpha);
        }
    }
    Ok(out)
}

/// The plane that must cover the opening plus bleed and edge guard.
pub fn under_frame_fill_size(opening: (f64, f64)) -> (f64, f64) {
    let extension = 2.0 * f64::from(UNDER_FRAME_BLEED_PIXELS + PORTRAIT_EDGE_GUARD_PIXELS);
    (opening.0 + extension, opening.1 + extension)
}

/// Width and height in card pixels.
pub type Size = (f64, f64);
/// Pixels clipped on the left, top, right and bottom.
pub type Clip = (f64, f64, f64, f64);

/// Size of the uniformly scaled source that covers `opening`, plus how much is clipped on each side.
pub fn covering_geometry(source: (u32, u32), opening: (f64, f64)) -> Result<(Size, Clip)> {
    if source.0 == 0 || source.1 == 0 {
        return Err(Error::Invalid("advisor source dimensions must be positive".into()));
    }
    let source_aspect = f64::from(source.0) / f64::from(source.1);
    let opening_aspect = opening.0 / opening.1;
    if source_aspect >= opening_aspect {
        let content = (opening.1 * source_aspect, opening.1);
        let clip = (content.0 - opening.0) / 2.0;
        Ok((content, (clip, 0.0, clip, 0.0)))
    } else {
        let content = (opening.0, opening.0 / source_aspect);
        let clip = (content.1 - opening.1) / 2.0;
        Ok((content, (0.0, clip, 0.0, clip)))
    }
}

fn render_portrait_layer(portrait: &Rgba, target: (f64, f64), center: (f64, f64), rotation: f64) -> Result<Rgba> {
    let sx = target.0 / f64::from(portrait.width);
    let sy = target.1 / f64::from(portrait.height);
    if (sx - sy).abs() > 1e-9 * sx.abs().max(sy.abs()) + 1e-12 {
        return Err(Error::Invalid("advisor portraits must use one uniform scale factor".into()));
    }
    let rad = rotation.to_radians();
    let (cos, sin) = (det_cos(rad), det_sin(rad));
    let (a, b, d, e) = (cos / sx, sin / sx, -sin / sy, cos / sy);
    let scx = f64::from(portrait.width) / 2.0;
    let scy = f64::from(portrait.height) / 2.0;
    let c = scx - a * center.0 - b * center.1;
    let f = scy - d * center.0 - e * center.1;
    affine_bicubic(portrait, CARD_SIZE, [a, b, c, d, e, f])
}

/// Build the advisor card.
pub fn advisor_card(source: &Rgba, template: &Rgba, params: &AdvisorParams) -> Result<AdvisorCard> {
    check_template(template)?;
    let opening = measure_opening_geometry(template)?;
    let size = params.portrait_size.unwrap_or(opening.size);
    let rotation = params.rotation.unwrap_or(opening.rotation);
    let offset = params.portrait_offset;
    validate_transform(size, offset, rotation, params.allow_zero_rotation)?;
    validate_opening_alignment(size, offset, rotation, &opening)?;
    let center = (opening.center.0 + offset.0, opening.center.1 + offset.1);
    validate_fit_to_opening(size, center, rotation, &opening)?;

    let portrait = prepare_portrait(source, params.sepia_strength)?;
    let fill = under_frame_fill_size(size);
    let (content, _) = covering_geometry((portrait.width, portrait.height), fill)?;
    // the covering plane is centred on the opening (local centre offset is zero)
    let mut card = render_portrait_layer(&portrait, content, center, rotation)?;
    let mask = bleed_mask(template, UNDER_FRAME_BLEED_PIXELS)?;
    for (i, p) in card.data.chunks_exact_mut(4).enumerate() {
        if !mask[i] {
            p[3] = 0;
        }
    }
    alpha_composite(&mut card, template, 0, 0);
    let coverage = audit_alpha_coverage(&card, template)?;
    if !coverage.is_clean() {
        return Err(Error::Image(format!(
            "advisor portrait alpha coverage failed: opening_alpha_gap_pixels={}, inner_edge_alpha_gap_pixels={}, exterior_alpha_leak_pixels={}",
            coverage.opening_alpha_gap_pixels, coverage.inner_edge_alpha_gap_pixels, coverage.exterior_alpha_leak_pixels
        )));
    }
    Ok(AdvisorCard { card, opening, coverage })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A synthetic dossier frame: opaque frame ring, a transparent opening rotated by `deg` and a transparent exterior
    /// outside the card outline (like the real template, whose corners are transparent).
    pub(crate) fn synthetic_template() -> Rgba {
        let mut t = Rgba::new(65, 67);
        let (cx, cy) = (24.5f64, 30.5f64);
        let rad = (-3.0f64).to_radians();
        let (c, s) = (rad.cos(), rad.sin());
        for y in 0..67u32 {
            for x in 0..65u32 {
                // card body: a rounded-off rectangle 4..61 x 3..64
                let inside_card = (3..62).contains(&x) && (3..65).contains(&y);
                if !inside_card {
                    continue;
                }
                // local coords in the opening's frame
                let (dx, dy) = (f64::from(x) - cx, f64::from(y) - cy);
                let lx = dx * c + dy * s;
                let ly = -dx * s + dy * c;
                let in_opening = lx.abs() < 16.0 && ly.abs() < 20.0;
                t.set_px(x, y, if in_opening { [0, 0, 0, 0] } else { [120, 100, 60, 255] });
            }
        }
        t
    }

    fn portrait(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_px(x, y, [(x * 255 / w) as u8, (y * 255 / h) as u8, 120, 255]);
            }
        }
        img
    }

    #[test]
    fn opening_is_found_and_measured() {
        let t = synthetic_template();
        let g = measure_opening_geometry(&t).unwrap();
        assert!((g.rotation - -3.0).abs() < 1.5, "{g:?}");
        assert!(g.size.0 < g.size.1);
        assert!((g.center.0 - 24.5).abs() < 1.5 && (g.center.1 - 30.5).abs() < 1.5, "{g:?}");
    }

    #[test]
    fn card_has_the_template_size_and_no_alpha_gaps() {
        let t = synthetic_template();
        for (w, h) in [(300, 400), (400, 300), (65, 67), (1000, 1000)] {
            let out = advisor_card(&portrait(w, h), &t, &AdvisorParams::default()).unwrap();
            assert_eq!((out.card.width, out.card.height), (65, 67));
            assert!(out.coverage.is_clean());
            // frame pixels come from the template, opening pixels from the portrait
            assert_eq!(out.card.px(10, 5), [120, 100, 60, 255]);
            assert_eq!(out.card.px(24, 30)[3], 255);
            // outside the card outline nothing is painted
            assert_eq!(out.card.px(0, 0)[3], 0);
        }
    }

    #[test]
    fn portrait_is_tinted_and_deterministic() {
        let t = synthetic_template();
        let src = portrait(200, 260);
        let a = advisor_card(&src, &t, &AdvisorParams::default()).unwrap();
        assert_eq!(a, advisor_card(&src, &t, &AdvisorParams::default()).unwrap());
        let strong = advisor_card(&src, &t, &AdvisorParams { sepia_strength: 1.0, ..Default::default() }).unwrap();
        let p = strong.card.px(24, 30);
        assert!(p[0] >= p[1] && p[1] >= p[2], "full sepia is a brown ramp: {p:?}");
    }

    #[test]
    fn bad_placements_fail_closed() {
        let t = synthetic_template();
        let src = portrait(200, 260);
        let g = measure_opening_geometry(&t).unwrap();
        let p = AdvisorParams { rotation: Some(g.rotation + 1.0), ..Default::default() };
        assert!(advisor_card(&src, &t, &p).unwrap_err().to_string().contains("rotation"));
        let p = AdvisorParams { portrait_offset: (1.0, 0.0), ..Default::default() };
        assert!(advisor_card(&src, &t, &p).is_err());
        let p = AdvisorParams { portrait_size: Some((g.size.0 + 1.0, g.size.1)), ..Default::default() };
        assert!(advisor_card(&src, &t, &p).is_err());
        let p = AdvisorParams { sepia_strength: 1.5, ..Default::default() };
        assert!(advisor_card(&src, &t, &p).is_err());
        assert!(advisor_card(&src, &Rgba::new(10, 10), &AdvisorParams::default()).is_err());
        // a template with no opening
        assert!(advisor_card(&src, &Rgba::filled(65, 67, [1, 1, 1, 255]), &AdvisorParams::default()).is_err());
    }

    #[test]
    fn covering_geometry_covers_without_stretching() {
        let (content, clip) = covering_geometry((200, 100), (30.0, 40.0)).unwrap();
        assert!(content.0 >= 30.0 && (content.1 - 40.0).abs() < 1e-9);
        assert!(clip.0 > 0.0 && clip.1 == 0.0);
        let (content, clip) = covering_geometry((100, 400), (30.0, 40.0)).unwrap();
        assert!(content.1 >= 40.0 && (content.0 - 30.0).abs() < 1e-9);
        assert!(clip.1 > 0.0 && clip.0 == 0.0);
        assert!(covering_geometry((0, 5), (1.0, 1.0)).is_err());
    }
}
