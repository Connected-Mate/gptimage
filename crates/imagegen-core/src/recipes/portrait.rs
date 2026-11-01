//! Portrait source crop: an exact crop of the master and the 156x210 portrait made from it.
//!
//! Port of the geometry and resize of the kit's `extract_portrait_source_crop.py`. Detection is deliberately
//! fail-closed: exactly one face must pass, otherwise the caller supplies a manual crop rectangle. Face detection
//! itself sits behind the [`FaceDetector`] trait; [`crate::recipes::face`] provides the YuNet implementation, and any
//! other detector (or a test double) can stand in.
//!
//! Left to the engine, as in the other ports: the package evidence (source copy, JSON, provenance text, staged
//! atomic writes). The crop is a lossless sub-rectangle of the decoded master, and the portrait is that crop
//! converted to opaque RGB and resized with Lanczos3.

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::pillow::py_round;
use crate::resize::resize_lanczos3;

/// Vanilla advisor and leader portraits are 156x210.
pub const PORTRAIT_SIZE: (u32, u32) = (156, 210);
/// Score at or above which a detection counts.
pub const FACE_SCORE_THRESHOLD: f32 = 0.85;
/// Non-maximum-suppression IoU of the detector.
pub const FACE_NMS_THRESHOLD: f32 = 0.3;
/// Overlap at which two detections are the same face.
pub const DEDUPLICATE_IOU: f64 = 0.35;

/// A face box, `(left, top, width, height)` in source pixels.
pub type FaceBox = (i64, i64, i64, i64);

/// Anything that finds faces. Implementations return boxes that passed [`FACE_SCORE_THRESHOLD`].
pub trait FaceDetector {
    /// Candidate faces in `image`.
    fn detect(&self, image: &Rgba) -> Result<Vec<FaceBox>>;
}

/// How the crop was chosen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropChoice {
    /// Exactly one face was found; the box is recorded.
    Detected(FaceBox),
    /// The caller supplied the rectangle.
    Manual,
}

/// The crop and the portrait made from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortraitCrop {
    /// Half-open `[left, top, right, bottom]` in master pixels.
    pub crop: [u32; 4],
    /// How it was chosen.
    pub choice: CropChoice,
    /// The lossless sub-rectangle of the master.
    pub crop_image: Rgba,
    /// 156x210, opaque.
    pub portrait: Rgba,
}

/// Intersection over union of two `(left, top, width, height)` boxes.
pub fn iou(a: FaceBox, b: FaceBox) -> f64 {
    let (ar, ab) = (a.0 + a.2, a.1 + a.3);
    let (br, bb) = (b.0 + b.2, b.1 + b.3);
    let iw = (ar.min(br) - a.0.max(b.0)).max(0);
    let ih = (ab.min(bb) - a.1.max(b.1)).max(0);
    let inter = iw * ih;
    if inter == 0 {
        return 0.0;
    }
    inter as f64 / (a.2 * a.3 + b.2 * b.3 - inter) as f64
}

/// Collapse overlapping detections (IoU 0.35 or more, larger box wins) without hiding distinct faces; the result
/// is ordered top to bottom, then left to right.
pub fn deduplicate_faces(candidates: &[FaceBox]) -> Vec<FaceBox> {
    let mut ordered: Vec<FaceBox> = candidates.to_vec();
    ordered.sort_by_key(|c| (-(c.2 * c.3), c.1, c.0));
    let mut kept: Vec<FaceBox> = Vec::new();
    for c in ordered {
        if c.2 <= 0 || c.3 <= 0 {
            continue;
        }
        if kept.iter().any(|k| iou(c, *k) >= DEDUPLICATE_IOU) {
            continue;
        }
        kept.push(c);
    }
    kept.sort_by_key(|c| (c.1, c.0, -(c.2 * c.3)));
    kept
}

/// Require exactly one usable face inside the image.
pub fn resolve_single_face(image_size: (u32, u32), candidates: &[FaceBox]) -> Result<FaceBox> {
    let (w, h) = (i64::from(image_size.0), i64::from(image_size.1));
    let valid: Vec<FaceBox> =
        candidates.iter().copied().filter(|c| c.0 >= 0 && c.1 >= 0 && c.2 > 0 && c.3 > 0 && c.0 + c.2 <= w && c.1 + c.3 <= h).collect();
    let valid = deduplicate_faces(&valid);
    match valid.len() {
        0 => Err(Error::Invalid("automatic subject detection found no unambiguous face; provide an explicit crop rectangle".into())),
        1 => Ok(valid[0]),
        n => Err(Error::Invalid(format!(
            "automatic subject detection found {n} faces; refusing to guess which subject to archive, provide an explicit crop rectangle"
        ))),
    }
}

/// A portrait-aspect head-and-shoulders crop around one face, as `[left, top, right, bottom]`.
///
/// Four and a half face heights tall (or the width equivalent, whichever is larger), the face centred
/// horizontally and its top 0.9 face heights below the crop's top, clamped to the image. Fails closed when the image
/// cannot hold a crop that contains the face.
pub fn compute_portrait_crop(image_size: (u32, u32), face: FaceBox) -> Result<[u32; 4]> {
    let (width, height) = (i64::from(image_size.0), i64::from(image_size.1));
    let (left, top, fw, fh) = face;
    if left < 0 || top < 0 || fw <= 0 || fh <= 0 || left + fw > width || top + fh > height {
        return Err(Error::Invalid(format!("detected face is outside the decoded image bounds: {face:?} in {image_size:?}")));
    }
    let aspect = f64::from(PORTRAIT_SIZE.0) / f64::from(PORTRAIT_SIZE.1);
    let mut crop_h = py_round(fh as f64 * 4.5).max(py_round(fw as f64 * 4.5 / aspect));
    let mut crop_w = py_round(crop_h as f64 * aspect).max(1);
    if crop_w > width {
        crop_w = width;
        crop_h = py_round(crop_w as f64 / aspect).max(1);
    }
    if crop_h > height {
        crop_h = height;
        crop_w = py_round(crop_h as f64 * aspect).max(1);
    }
    if crop_w > width {
        crop_w = width;
        crop_h = py_round(crop_w as f64 / aspect).max(1);
    }
    if crop_w < fw || crop_h < fh {
        return Err(Error::Invalid(
            "automatic head-and-shoulders crop cannot contain the detected face at portrait aspect; provide a larger source or an explicit crop rectangle".into(),
        ));
    }
    let face_cx = left as f64 + fw as f64 / 2.0;
    let l = py_round(face_cx - crop_w as f64 / 2.0).clamp(0, width - crop_w);
    let t = py_round(top as f64 - fh as f64 * 0.9).clamp(0, height - crop_h);
    let crop = [l, t, l + crop_w, t + crop_h];
    if !(crop[0] <= left && crop[1] <= top && crop[2] >= left + fw && crop[3] >= top + fh) {
        return Err(Error::Invalid(
            "automatic head-and-shoulders crop would clip the detected face; provide an explicit crop rectangle".into(),
        ));
    }
    Ok(crop.map(|v| v as u32))
}

/// Check a half-open `[left, top, right, bottom]` rectangle against the image.
pub fn validate_crop_box(image_size: (u32, u32), crop: [i64; 4]) -> Result<[u32; 4]> {
    let [l, t, r, b] = crop;
    if l < 0 || t < 0 || r > i64::from(image_size.0) || b > i64::from(image_size.1) {
        return Err(Error::Invalid(format!("crop {crop:?} is outside decoded master bounds {}x{}", image_size.0, image_size.1)));
    }
    if r <= l || b <= t {
        return Err(Error::Invalid(format!("crop {crop:?} must describe a nonempty rectangle")));
    }
    Ok([l as u32, t as u32, r as u32, b as u32])
}

/// Make the 156x210 portrait from an exact crop: drop alpha, resize with Lanczos3.
pub fn portrait_from_crop(crop_image: &Rgba) -> Result<Rgba> {
    let mut opaque = crop_image.clone();
    for p in opaque.data.chunks_exact_mut(4) {
        p[3] = 255;
    }
    resize_lanczos3(&opaque, PORTRAIT_SIZE.0, PORTRAIT_SIZE.1)
}

fn finish(master: &Rgba, crop: [u32; 4], choice: CropChoice) -> Result<PortraitCrop> {
    let crop_image = master.crop(crop[0], crop[1], crop[2] - crop[0], crop[3] - crop[1])?;
    let portrait = portrait_from_crop(&crop_image)?;
    Ok(PortraitCrop { crop, choice, crop_image, portrait })
}

/// Manual route: the caller's rectangle, validated.
pub fn extract_portrait_manual(master: &Rgba, crop: [i64; 4]) -> Result<PortraitCrop> {
    let crop = validate_crop_box(master.size(), crop)?;
    finish(master, crop, CropChoice::Manual)
}

/// Automatic route from already detected candidates: exactly one face, then the head-and-shoulders crop.
pub fn extract_portrait_from_faces(master: &Rgba, candidates: &[FaceBox]) -> Result<PortraitCrop> {
    let face = resolve_single_face(master.size(), candidates)?;
    let crop = compute_portrait_crop(master.size(), face)?;
    finish(master, crop, CropChoice::Detected(face))
}

/// Automatic route with a detector. A `manual` rectangle skips detection.
pub fn extract_portrait(master: &Rgba, detector: &dyn FaceDetector, manual: Option<[i64; 4]>) -> Result<PortraitCrop> {
    match manual {
        Some(crop) => extract_portrait_manual(master, crop),
        None => extract_portrait_from_faces(master, &detector.detect(master)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Vec<FaceBox>);
    impl FaceDetector for Fixed {
        fn detect(&self, _: &Rgba) -> Result<Vec<FaceBox>> {
            Ok(self.0.clone())
        }
    }

    fn master(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            for x in 0..w {
                img.set_px(x, y, [(x % 256) as u8, (y % 256) as u8, ((x ^ y) % 256) as u8, 255]);
            }
        }
        img
    }

    #[test]
    fn iou_and_deduplication() {
        assert_eq!(iou((0, 0, 10, 10), (20, 20, 5, 5)), 0.0);
        assert!((iou((0, 0, 10, 10), (0, 0, 10, 10)) - 1.0).abs() < 1e-12);
        // two boxes on one face collapse to the larger one; a distinct face survives
        let kept = deduplicate_faces(&[(10, 10, 40, 40), (12, 12, 38, 38), (200, 10, 40, 40)]);
        assert_eq!(kept, vec![(10, 10, 40, 40), (200, 10, 40, 40)]);
    }

    #[test]
    fn exactly_one_face_is_required() {
        assert!(resolve_single_face((400, 400), &[]).is_err());
        let two = resolve_single_face((400, 400), &[(10, 10, 40, 40), (200, 10, 40, 40)]).unwrap_err();
        assert!(two.to_string().contains("2 faces"), "{two}");
        assert_eq!(resolve_single_face((400, 400), &[(10, 10, 40, 40), (11, 11, 40, 40)]).unwrap(), (10, 10, 40, 40));
        // a box that leaves the image is not usable
        assert!(resolve_single_face((100, 100), &[(80, 80, 40, 40)]).is_err());
    }

    #[test]
    fn crop_has_portrait_aspect_contains_the_face_and_stays_inside() {
        let face = (300, 200, 80, 100);
        let crop = compute_portrait_crop((800, 900), face).unwrap();
        let (w, h) = (crop[2] - crop[0], crop[3] - crop[1]);
        assert!((f64::from(w) / f64::from(h) - 156.0 / 210.0).abs() < 0.01, "{w}x{h}");
        assert!(i64::from(crop[0]) <= face.0 && i64::from(crop[2]) >= face.0 + face.2);
        assert!(i64::from(crop[1]) <= face.1 && i64::from(crop[3]) >= face.1 + face.3);
        assert!(crop[2] <= 800 && crop[3] <= 900);
        assert_eq!(h, 485, "the wider of 4.5 face heights and 4.5 face widths at portrait aspect");
        // the face's top sits 0.9 face heights below the crop's top
        assert_eq!(i64::from(crop[1]), 200 - 90);
    }

    #[test]
    fn crops_near_the_edge_are_clamped_and_tight_images_fail_closed() {
        let crop = compute_portrait_crop((800, 900), (0, 0, 80, 100)).unwrap();
        assert_eq!((crop[0], crop[1]), (0, 0));
        // a face that fills the picture cannot be framed at portrait aspect
        assert!(compute_portrait_crop((100, 100), (0, 0, 100, 100)).is_err());
        assert!(compute_portrait_crop((100, 100), (90, 90, 40, 40)).is_err());
    }

    #[test]
    fn automatic_route_gives_a_156x210_opaque_portrait() {
        let m = master(800, 900);
        let out = extract_portrait(&m, &Fixed(vec![(300, 200, 80, 100)]), None).unwrap();
        assert_eq!(out.choice, CropChoice::Detected((300, 200, 80, 100)));
        assert_eq!((out.portrait.width, out.portrait.height), PORTRAIT_SIZE);
        assert!(!out.portrait.has_transparency());
        // the crop is a lossless sub-rectangle of the master
        let c = out.crop;
        assert_eq!(out.crop_image, m.crop(c[0], c[1], c[2] - c[0], c[3] - c[1]).unwrap());
    }

    #[test]
    fn zero_or_many_faces_fail_and_manual_recovers() {
        let m = master(400, 500);
        assert!(extract_portrait(&m, &Fixed(vec![]), None).is_err());
        assert!(extract_portrait(&m, &Fixed(vec![(10, 10, 40, 40), (200, 10, 40, 40)]), None).is_err());
        let manual = extract_portrait(&m, &Fixed(vec![]), Some([50, 60, 250, 340])).unwrap();
        assert_eq!(manual.choice, CropChoice::Manual);
        assert_eq!(manual.crop, [50, 60, 250, 340]);
        assert_eq!((manual.portrait.width, manual.portrait.height), (156, 210));
    }

    #[test]
    fn manual_crop_is_validated() {
        let m = master(100, 100);
        assert!(extract_portrait_manual(&m, [-1, 0, 50, 50]).is_err());
        assert!(extract_portrait_manual(&m, [0, 0, 101, 50]).is_err());
        assert!(extract_portrait_manual(&m, [10, 10, 10, 50]).is_err());
        assert!(extract_portrait_manual(&m, [0, 0, 100, 100]).is_ok());
    }

    #[test]
    fn alpha_is_dropped_from_the_portrait() {
        let mut m = master(100, 100);
        m.set_px(5, 5, [200, 100, 50, 0]);
        let out = extract_portrait_manual(&m, [0, 0, 100, 100]).unwrap();
        assert!(!out.portrait.has_transparency());
    }
}
