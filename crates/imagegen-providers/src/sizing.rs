//! Choosing a provider output size for a HOI4 canvas.
//!
//! HOI4 targets are tiny (100x88 up to 457x328). Providers cannot go that small, so the pipeline asks for the
//! smallest allowed size with the canvas aspect, then crops and downscales deterministically.

use crate::types::{ProviderError, SizeRule};

/// Aspect ratios Gemini accepts that are closest to a canvas, as `w:h`.
pub fn nearest_ratio(ratios: &[String], canvas: [u32; 2]) -> Option<String> {
    let target = f64::from(canvas[0]) / f64::from(canvas[1].max(1));
    ratios
        .iter()
        .filter_map(|r| {
            let (a, b) = r.split_once(':')?;
            let v = a.parse::<f64>().ok()? / b.parse::<f64>().ok()?;
            Some((r, (v.ln() - target.ln()).abs()))
        })
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
        .map(|(r, _)| r.clone())
}

/// Nominal pixel size (about one megapixel) of an aspect ratio written `w:h`, for providers that choose size by
/// ratio and tier.
pub fn tier_size(ratio: &str) -> (u32, u32) {
    let (a, b) = ratio.split_once(':').unwrap_or(("1", "1"));
    let (a, b): (f64, f64) = (a.parse().unwrap_or(1.0), b.parse().unwrap_or(1.0));
    let scale = (1024.0 * 1024.0 / (a * b)).sqrt();
    ((a * scale).round() as u32, (b * scale).round() as u32)
}

/// Smallest allowed `(w, h)` whose aspect is closest to `canvas`.
pub fn choose_size(rule: &SizeRule, canvas: [u32; 2]) -> Result<(u32, u32), ProviderError> {
    let aspect = f64::from(canvas[0]) / f64::from(canvas[1].max(1));
    match rule {
        SizeRule::Fixed { sizes } => sizes
            .iter()
            .min_by(|a, b| {
                let da = (f64::from(a[0]) / f64::from(a[1]) / aspect).ln().abs();
                let db = (f64::from(b[0]) / f64::from(b[1]) / aspect).ln().abs();
                da.partial_cmp(&db)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then((u64::from(a[0]) * u64::from(a[1])).cmp(&(u64::from(b[0]) * u64::from(b[1]))))
            })
            .map(|s| (s[0], s[1]))
            .ok_or_else(|| ProviderError::unsupported("the model has no allowed sizes")),
        SizeRule::Arbitrary { multiple, min_pixels, max_pixels, max_edge, max_ratio } => {
            let m = (*multiple).max(1);
            if aspect > *max_ratio || aspect < 1.0 / *max_ratio {
                return Err(ProviderError::unsupported(format!("aspect {}:{} is outside the model's {max_ratio}:1 limit", canvas[0], canvas[1])));
            }
            // walk the short edge up in steps of `multiple` until the pixel floor is met
            let landscape = aspect >= 1.0;
            let mut short = m;
            loop {
                let long_f = f64::from(short) * if landscape { aspect } else { 1.0 / aspect };
                let long = ((long_f / f64::from(m)).round() as u32).max(1) * m;
                let (w, h) = if landscape { (long, short) } else { (short, long) };
                if u64::from(w) * u64::from(h) >= *min_pixels {
                    if u64::from(w) * u64::from(h) > *max_pixels || w > *max_edge || h > *max_edge {
                        return Err(ProviderError::unsupported("no allowed size fits this aspect ratio"));
                    }
                    return Ok((w, h));
                }
                short += m;
                if short > *max_edge {
                    return Err(ProviderError::unsupported("no allowed size fits this aspect ratio"));
                }
            }
        }
        SizeRule::AspectTiers { .. } => Err(ProviderError::unsupported("this model chooses sizes by aspect ratio and tier, not by pixels")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gpt2() -> SizeRule {
        SizeRule::Arbitrary { multiple: 16, min_pixels: 655_360, max_pixels: 8_294_400, max_edge: 3840, max_ratio: 3.0 }
    }

    #[test]
    fn arbitrary_sizes_meet_every_constraint_for_every_hoi4_canvas() {
        let canvases = [
            [100, 88],
            [65, 67],
            [33, 32],
            [52, 40],
            [210, 176],
            [397, 153],
            [457, 328],
            [156, 210],
            [82, 52],
            [10, 7],
            [114, 101],
            [45, 45],
            [146, 54],
        ];
        for c in canvases {
            let (w, h) = choose_size(&gpt2(), c).unwrap_or_else(|e| panic!("{c:?}: {e}"));
            assert_eq!(w % 16, 0, "{c:?} -> {w}x{h}");
            assert_eq!(h % 16, 0, "{c:?} -> {w}x{h}");
            let px = u64::from(w) * u64::from(h);
            assert!((655_360..=8_294_400).contains(&px), "{c:?} -> {w}x{h} = {px}");
            let ratio = f64::from(w.max(h)) / f64::from(w.min(h));
            assert!(ratio <= 3.0, "{c:?} -> {w}x{h}");
            // the aspect is within one grid step of the canvas aspect
            let want = f64::from(c[0]) / f64::from(c[1]);
            let got = f64::from(w) / f64::from(h);
            assert!((got / want).ln().abs() < 0.12, "{c:?} -> {w}x{h}: aspect {got} vs {want}");
        }
    }

    #[test]
    fn smallest_square_is_the_pixel_floor() {
        // 640x1024 = 655360 is not square; the smallest square multiple of 16 above the floor is 816x816
        let (w, h) = choose_size(&gpt2(), [1, 1]).unwrap();
        assert_eq!((w, h), (816, 816));
    }

    #[test]
    fn extreme_ratios_are_refused() {
        assert!(choose_size(&gpt2(), [10, 1]).is_err());
        assert!(choose_size(&gpt2(), [1, 10]).is_err());
        assert!(choose_size(&gpt2(), [3, 1]).is_ok());
    }

    #[test]
    fn fixed_sizes_pick_the_closest_aspect() {
        let rule = SizeRule::Fixed { sizes: vec![[1024, 1024], [1536, 1024], [1024, 1536]] };
        assert_eq!(choose_size(&rule, [397, 153]).unwrap(), (1536, 1024));
        assert_eq!(choose_size(&rule, [156, 210]).unwrap(), (1024, 1536));
        assert_eq!(choose_size(&rule, [100, 88]).unwrap(), (1024, 1024));
    }

    #[test]
    fn nearest_gemini_ratio() {
        let ratios: Vec<String> = ["1:1", "2:3", "3:2", "3:4", "4:3", "4:5", "5:4", "9:16", "16:9", "21:9"].iter().map(|s| s.to_string()).collect();
        assert_eq!(nearest_ratio(&ratios, [100, 88]).unwrap(), "5:4");
        assert_eq!(nearest_ratio(&ratios, [397, 153]).unwrap(), "21:9");
        assert_eq!(nearest_ratio(&ratios, [156, 210]).unwrap(), "3:4");
        assert_eq!(nearest_ratio(&ratios, [64, 64]).unwrap(), "1:1");
    }
}
