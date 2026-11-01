//! Flatten on a colour: composite over a flat background and make the result fully opaque.
//!
//! For opaque families (news pictures, event pictures, anything the game draws without transparency). This is the
//! pipeline's own [`Op::FlattenOn`] under a recipe name, so the arithmetic is the same everywhere:
//! `(c * a + bg * (255 - a) + 127) / 255` per channel.

use crate::error::Result;
use crate::pixels::Rgba;
use crate::process::{apply, Op};

/// Composite `img` over `background` and return a fully opaque image.
pub fn flatten_on_colour(img: &Rgba, background: [u8; 3]) -> Result<Rgba> {
    apply(img, &Op::FlattenOn { background })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transparent_becomes_the_background_and_opaque_is_kept() {
        let mut img = Rgba::new(3, 1);
        img.set_px(0, 0, [10, 20, 30, 255]);
        img.set_px(1, 0, [99, 99, 99, 0]);
        img.set_px(2, 0, [255, 255, 255, 128]);
        let out = flatten_on_colour(&img, [0, 0, 0]).unwrap();
        assert_eq!(out.px(0, 0), [10, 20, 30, 255]);
        assert_eq!(out.px(1, 0), [0, 0, 0, 255]);
        assert_eq!(out.px(2, 0), [128, 128, 128, 255]);
        assert!(!out.has_transparency());
    }

    #[test]
    fn an_opaque_image_is_unchanged() {
        let img = Rgba::filled(4, 4, [1, 2, 3, 255]);
        assert_eq!(flatten_on_colour(&img, [200, 200, 200]).unwrap(), img);
    }
}
