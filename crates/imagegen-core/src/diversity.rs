//! Near-duplicate frames and compositions: a perceptual hash and its family threshold.
//!
//! The hash is a 128-bit difference hash (dHash) of the image composited over mid grey at its own size: 64 bits
//! compare horizontal neighbours of a 9x8 area-averaged grey thumbnail, 64 bits vertical neighbours of an 8x9 one.
//! It sees the outline of the frame and the placement of the large shapes, not colour or fine detail.
//!
//! What counts as near is the family's own: the threshold is the 1st percentile of the distances between distinct
//! files of the family's reference set (vanilla when it falls to vanilla, up to 256 files sampled evenly), so a warning
//! means two outputs are closer than 99 in 100 pairs of the family's own art. Measured on HOI4 1.19.4.1: focus 28,
//! national spirits 31, decisions 27, decision categories 15, report events 41, unit leader traits 22 and
//! technologies 24 bits. Vanilla's own recolours and copies sit at 0 to 6 bits.

use crate::pixels::Rgba;

/// Bits in the hash.
pub const HASH_BITS: u32 = 128;
/// The percentile of a family's own pairwise distances used as its threshold.
pub const THRESHOLD_PERCENTILE: f64 = 0.01;
/// Lowest threshold, whatever the family's spread: below it two images are near copies anyway.
pub const MIN_THRESHOLD: u32 = 6;
/// Threshold used when the family has no reference set to measure (the median of the measured families' thresholds
/// is 28; the fallback stays well below it so that only clear near-copies warn).
pub const FALLBACK_THRESHOLD: u32 = 16;

fn grey_over_mid(px: [u8; 4]) -> f64 {
    let a = f64::from(px[3]) / 255.0;
    let lum = 0.299 * f64::from(px[0]) + 0.587 * f64::from(px[1]) + 0.114 * f64::from(px[2]);
    lum * a + 128.0 * (1.0 - a)
}

/// Area-averaged grey thumbnail of `w` by `h`.
fn thumbnail(img: &Rgba, w: usize, h: usize) -> Vec<f64> {
    let (iw, ih) = (img.width as usize, img.height as usize);
    let mut sum = vec![0f64; w * h];
    let mut weight = vec![0f64; w * h];
    if iw == 0 || ih == 0 {
        return sum;
    }
    for y in 0..ih {
        let ty = y * h / ih;
        for x in 0..iw {
            let tx = x * w / iw;
            sum[ty * w + tx] += grey_over_mid(img.px(x as u32, y as u32));
            weight[ty * w + tx] += 1.0;
        }
    }
    sum.iter().zip(&weight).map(|(s, n)| if *n > 0.0 { s / n } else { 128.0 }).collect()
}

/// The 128-bit dHash of an image at its own size, composited over mid grey.
pub fn dhash(img: &Rgba) -> u128 {
    let mut bits: u128 = 0;
    let a = thumbnail(img, 9, 8);
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u128::from(a[y * 9 + x] < a[y * 9 + x + 1]);
        }
    }
    let b = thumbnail(img, 8, 9);
    for y in 0..8 {
        for x in 0..8 {
            bits = (bits << 1) | u128::from(b[y * 8 + x] < b[(y + 1) * 8 + x]);
        }
    }
    bits
}

/// Bits that differ.
pub fn distance(a: u128, b: u128) -> u32 {
    (a ^ b).count_ones()
}

/// The family threshold from its own set: the [`THRESHOLD_PERCENTILE`] (nearest rank) of all pairwise distances,
/// at least [`MIN_THRESHOLD`]. `None` with fewer than three images.
pub fn threshold_from(hashes: &[u128]) -> Option<(u32, usize)> {
    if hashes.len() < 3 {
        return None;
    }
    let mut d: Vec<u32> = Vec::with_capacity(hashes.len() * (hashes.len() - 1) / 2);
    for i in 0..hashes.len() {
        for j in i + 1..hashes.len() {
            d.push(distance(hashes[i], hashes[j]));
        }
    }
    d.sort_unstable();
    let rank = ((d.len() as f64 * THRESHOLD_PERCENTILE).ceil() as usize).clamp(1, d.len()) - 1;
    Some((d[rank].max(MIN_THRESHOLD), d.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plaque(cx: f64, r: f64, shade: u8) -> Rgba {
        let mut img = Rgba::new(100, 88);
        for y in 0..88u32 {
            for x in 0..100u32 {
                let d = ((f64::from(x) - cx).powi(2) + (f64::from(y) - 44.0).powi(2)).sqrt();
                if d < r {
                    let v = if d > r - 6.0 { 200 } else { shade.wrapping_add((x % 7) as u8) };
                    img.set_px(x, y, [v, v, v, 255]);
                }
            }
        }
        img
    }

    #[test]
    fn the_hash_sees_frame_and_layout_not_small_changes() {
        let a = dhash(&plaque(50.0, 38.0, 60));
        let b = dhash(&plaque(50.0, 38.0, 70));
        let c = dhash(&plaque(30.0, 22.0, 60));
        assert!(distance(a, b) <= 6, "same frame and layout: {}", distance(a, b));
        assert!(distance(a, c) > 20, "another layout: {}", distance(a, c));
        assert_eq!(distance(a, a), 0);
        // the hash ignores the size the image is stored at
        let mut big = Rgba::new(200, 176);
        let small = plaque(50.0, 38.0, 60);
        for y in 0..176 {
            for x in 0..200 {
                big.set_px(x, y, small.px(x / 2, y / 2));
            }
        }
        assert!(distance(a, dhash(&big)) <= 4);
    }

    #[test]
    fn the_threshold_is_a_low_percentile_of_the_set() {
        assert_eq!(threshold_from(&[0, 1]), None);
        let set: Vec<u128> = (0..40u32).map(|i| ((1u128 << (i % 128)) * 0xFFFF_FFFF) | u128::from(i)).collect();
        let (t, pairs) = threshold_from(&set).unwrap();
        assert_eq!(pairs, 40 * 39 / 2);
        assert!(t >= MIN_THRESHOLD);
        let identical = vec![7u128; 5];
        assert_eq!(threshold_from(&identical).unwrap().0, MIN_THRESHOLD);
    }
}
