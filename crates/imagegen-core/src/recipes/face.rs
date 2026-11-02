//! YuNet face detection through pure-Rust `tract-onnx` (cargo feature `face-detect`).
//!
//! Loads the kit's bundled `face_detection_yunet_2026may.onnx` (MIT, OpenCV Zoo) and reproduces what OpenCV's
//! `FaceDetectorYN` does around it: the image is fed as BGR floats in 0..255, padded with zeros at the bottom and
//! right to a multiple of 32, the twelve outputs (class, objectness, box and five landmarks at strides 8, 16 and 32)
//! are decoded as `score = sqrt(cls * obj)`, `centre = (cell + offset) * stride`, `size = exp(offset) * stride`, and
//! greedy non-maximum suppression runs at the kit's thresholds (score 0.85, IoU 0.3). Boxes are truncated to
//! integers as OpenCV does, then [`crate::recipes::portrait::deduplicate_faces`] collapses overlaps exactly like
//! the Python tool.
//!
//! Evidence status: the decoding follows OpenCV's source as remembered, and the model's twelve outputs were checked
//! by shape. No OpenCV was available on the development machine, so agreement with `cv2.FaceDetectorYN` on the
//! same images is unmeasured; the tests check plausibility on real portraits instead.

use tract_onnx::prelude::*;

use crate::error::{Error, Result};
use crate::pixels::Rgba;
use crate::recipes::portrait::{deduplicate_faces, iou, FaceBox, FaceDetector, FACE_NMS_THRESHOLD, FACE_SCORE_THRESHOLD};

const STRIDES: [usize; 3] = [8, 16, 32];

/// One decoded detection after suppression.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Detection {
    /// `(left, top, width, height)` truncated to integers.
    pub bbox: FaceBox,
    /// `sqrt(cls * obj)`.
    pub score: f32,
}

/// A loaded YuNet model.
pub struct YuNet {
    model: InferenceModel,
}

fn tract_err(what: &str, e: impl std::fmt::Display) -> Error {
    Error::Image(format!("YuNet {what}: {e}"))
}

impl YuNet {
    /// Load the ONNX model from bytes.
    pub fn from_onnx_bytes(bytes: &[u8]) -> Result<Self> {
        let model = tract_onnx::onnx().model_for_read(&mut std::io::Cursor::new(bytes)).map_err(|e| tract_err("could not load the model", e))?;
        Ok(YuNet { model })
    }

    /// Load the ONNX model from a file.
    pub fn from_path(path: &std::path::Path) -> Result<Self> {
        let bytes = std::fs::read(path).map_err(|e| Error::io(path.display(), e))?;
        Self::from_onnx_bytes(&bytes)
    }

    /// Detections after NMS at the kit's thresholds, best first.
    pub fn detections(&self, image: &Rgba) -> Result<Vec<Detection>> {
        let (w, h) = (image.width as usize, image.height as usize);
        let pad_w = (w - 1) / 32 * 32 + 32;
        let pad_h = (h - 1) / 32 * 32 + 32;
        let mut input = tract_ndarray::Array4::<f32>::zeros((1, 3, pad_h, pad_w));
        for y in 0..h {
            for x in 0..w {
                let p = image.px(x as u32, y as u32);
                // BGR order, 0..255, no normalisation
                input[[0, 0, y, x]] = f32::from(p[2]);
                input[[0, 1, y, x]] = f32::from(p[1]);
                input[[0, 2, y, x]] = f32::from(p[0]);
            }
        }
        let runnable = self
            .model
            .clone()
            .with_input_fact(0, f32::fact([1, 3, pad_h, pad_w]).into())
            .and_then(|m| m.into_optimized())
            .and_then(|m| m.into_runnable())
            .map_err(|e| tract_err("could not prepare the model for this image size", e))?;
        let out = runnable.run(tvec!(Tensor::from(input).into())).map_err(|e| tract_err("inference failed", e))?;
        if out.len() != 12 {
            return Err(Error::Image(format!("YuNet produced {} outputs, expected 12", out.len())));
        }
        let plane = |i: usize| -> Result<Vec<f32>> {
            Ok(out[i].to_array_view::<f32>().map_err(|e| tract_err("unexpected output type", e))?.iter().copied().collect())
        };
        let mut candidates: Vec<Detection> = Vec::new();
        for (level, stride) in STRIDES.iter().enumerate() {
            let (cols, rows) = (pad_w / stride, pad_h / stride);
            let (cls, obj, bbox) = (plane(level)?, plane(3 + level)?, plane(6 + level)?);
            if cls.len() != rows * cols || obj.len() != rows * cols || bbox.len() != rows * cols * 4 {
                return Err(Error::Image(format!("YuNet output {level} has an unexpected size for {pad_w}x{pad_h}")));
            }
            let s = *stride as f32;
            for r in 0..rows {
                for c in 0..cols {
                    let idx = r * cols + c;
                    let score = (cls[idx].clamp(0.0, 1.0) * obj[idx].clamp(0.0, 1.0)).sqrt();
                    if score < FACE_SCORE_THRESHOLD {
                        continue;
                    }
                    let b = &bbox[idx * 4..idx * 4 + 4];
                    let cx = (c as f32 + b[0]) * s;
                    let cy = (r as f32 + b[1]) * s;
                    let bw = b[2].exp() * s;
                    let bh = b[3].exp() * s;
                    let (x1, y1) = (cx - bw / 2.0, cy - bh / 2.0);
                    candidates.push(Detection { bbox: (x1 as i64, y1 as i64, bw as i64, bh as i64), score });
                }
            }
        }
        Ok(non_max_suppression(candidates, f64::from(FACE_NMS_THRESHOLD)))
    }
}

/// Greedy NMS: best score first, drop boxes whose IoU with a kept box exceeds `threshold`.
pub fn non_max_suppression(mut candidates: Vec<Detection>, threshold: f64) -> Vec<Detection> {
    candidates.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
    let mut kept: Vec<Detection> = Vec::new();
    for c in candidates {
        if kept.iter().all(|k| iou(c.bbox, k.bbox) <= threshold) {
            kept.push(c);
        }
    }
    kept
}

/// Longest side YuNet sees. Its anchors are sized for faces up to a few hundred pixels, so a face that fills a
/// large provider portrait (about 1080x1456) is missed at full size and found with a score above 0.93 once the image
/// is scaled to this. Images at or below it are detected unchanged, as the kit's Python tool does.
pub const DETECT_MAX_SIDE: u32 = 640;

impl FaceDetector for YuNet {
    fn detect(&self, image: &Rgba) -> Result<Vec<FaceBox>> {
        let longest = image.width.max(image.height);
        let boxes: Vec<FaceBox> = if longest > DETECT_MAX_SIDE {
            let f = f64::from(DETECT_MAX_SIDE) / f64::from(longest);
            let (w, h) = (((f64::from(image.width) * f).round() as u32).max(1), ((f64::from(image.height) * f).round() as u32).max(1));
            let small = crate::resize::resize_lanczos3(image, w, h)?;
            let (sx, sy) = (f64::from(image.width) / f64::from(w), f64::from(image.height) / f64::from(h));
            self.detections(&small)?
                .into_iter()
                .map(|d| {
                    let (x, y, bw, bh) = d.bbox;
                    ((x as f64 * sx) as i64, (y as f64 * sy) as i64, (bw as f64 * sx) as i64, (bh as f64 * sy) as i64)
                })
                .collect()
        } else {
            self.detections(image)?.into_iter().map(|d| d.bbox).collect()
        };
        Ok(deduplicate_faces(&boxes))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nms_keeps_the_best_of_overlapping_boxes() {
        let d = |x, s| Detection { bbox: (x, 0, 100, 100), score: s };
        let kept = non_max_suppression(vec![d(0, 0.9), d(5, 0.95), d(400, 0.88)], 0.3);
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].bbox.0, 5);
        assert_eq!(kept[1].bbox.0, 400);
    }
}
