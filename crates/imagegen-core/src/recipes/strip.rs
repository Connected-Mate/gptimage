//! Horizontal frame strips: N frames side by side, `frame width = strip width / frames`.
//!
//! HOI4 sprites with `noOfFrames` read one image whose frames sit left to right. These helpers check the
//! arithmetic once so a recipe never produces a strip whose width does not divide.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Width of one frame, or an error when the strip does not divide evenly.
pub fn frame_width(strip_width: u32, frames: u32) -> Result<u32> {
    if frames == 0 {
        return Err(Error::Invalid("a strip needs at least one frame".into()));
    }
    if !strip_width.is_multiple_of(frames) {
        return Err(Error::Invalid(format!("strip width {strip_width} does not divide into {frames} frames")));
    }
    Ok(strip_width / frames)
}

/// Total size of a strip of `frames` frames of `frame_size`.
pub fn strip_size(frame_size: (u32, u32), frames: u32) -> Result<(u32, u32)> {
    if frames == 0 || frame_size.0 == 0 || frame_size.1 == 0 {
        return Err(Error::Invalid("frame size and count must be positive".into()));
    }
    Ok((frame_size.0 * frames, frame_size.1))
}

/// Join frames, which must all have `frame_size`, into one strip.
pub fn build_strip(frames: &[Rgba], frame_size: (u32, u32)) -> Result<Rgba> {
    if let Some(bad) = frames.iter().position(|f| (f.width, f.height) != frame_size) {
        return Err(Error::Invalid(format!(
            "frame {bad} is {}x{}, every frame must be {}x{}",
            frames[bad].width, frames[bad].height, frame_size.0, frame_size.1
        )));
    }
    Rgba::join_frames(frames)
}

/// Cut a strip back into its frames.
pub fn split_strip(strip: &Rgba, frames: u32) -> Result<Vec<Rgba>> {
    frame_width(strip.width, frames)?;
    strip.split_frames(frames)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(v: u8) -> Rgba {
        Rgba::filled(6, 4, [v, v, v, 255])
    }

    #[test]
    fn widths_divide_or_fail() {
        assert_eq!(frame_width(60, 3).unwrap(), 20);
        assert!(frame_width(61, 3).is_err());
        assert!(frame_width(60, 0).is_err());
        assert_eq!(strip_size((20, 10), 3).unwrap(), (60, 10));
        assert!(strip_size((20, 10), 0).is_err());
    }

    #[test]
    fn strip_round_trips_in_order() {
        let frames = [solid(10), solid(20), solid(30)];
        let strip = build_strip(&frames, (6, 4)).unwrap();
        assert_eq!((strip.width, strip.height), (18, 4));
        assert_eq!(strip.px(0, 0)[0], 10);
        assert_eq!(strip.px(6, 0)[0], 20);
        assert_eq!(strip.px(17, 3)[0], 30);
        assert_eq!(split_strip(&strip, 3).unwrap(), frames.to_vec());
        assert!(split_strip(&strip, 4).is_err());
    }

    #[test]
    fn a_wrong_sized_frame_is_named() {
        let err = build_strip(&[solid(1), Rgba::new(5, 4)], (6, 4)).unwrap_err().to_string();
        assert!(err.contains("frame 1"), "{err}");
        assert!(build_strip(&[], (6, 4)).is_err());
    }
}
