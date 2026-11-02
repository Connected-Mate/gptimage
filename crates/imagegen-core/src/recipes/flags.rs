//! Flag ladder: the three sizes HOI4 loads for one country tag, as 32-bit TGA files.
//!
//! The 82x52 master is the game's `gfx/flags/<TAG>.tga`; `medium/` holds the 41x26 version and `small/` the 10x7
//! one. The two smaller flags are resampled from the master with the crate's deterministic Lanczos3, so the three
//! always agree. Rows are stored bottom to top by default, the kit's rule; vanilla ships both origins and the engine
//! loads either, so the origin is a parameter of [`FlagLadder::tga_files`].
//!
//! A source that is not 82x52 is cover-resized to it (centre crop to the 41:26 aspect, then resize). Flags are not
//! flattened: vanilla flags are opaque and an image with transparency stays as it is.

use crate::error::{Error, Result};
use crate::naming::validate_flag_name;
use crate::pixels::Rgba;
use crate::process::{apply, Op};
use crate::resize::resize_lanczos3;
use crate::tga::{write_truecolor32, Origin};

/// `gfx/flags/<TAG>.tga`
pub const NORMAL_SIZE: (u32, u32) = (82, 52);
/// `gfx/flags/medium/<TAG>.tga`
pub const MEDIUM_SIZE: (u32, u32) = (41, 26);
/// `gfx/flags/small/<TAG>.tga`
pub const SMALL_SIZE: (u32, u32) = (10, 7);

/// Which of the three folders a flag belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlagSize {
    /// 82x52, `gfx/flags/`
    Normal,
    /// 41x26, `gfx/flags/medium/`
    Medium,
    /// 10x7, `gfx/flags/small/`
    Small,
}

impl FlagSize {
    /// Pixel size.
    pub fn size(self) -> (u32, u32) {
        match self {
            FlagSize::Normal => NORMAL_SIZE,
            FlagSize::Medium => MEDIUM_SIZE,
            FlagSize::Small => SMALL_SIZE,
        }
    }

    /// Folder below the mod root.
    pub fn folder(self) -> &'static str {
        match self {
            FlagSize::Normal => "gfx/flags",
            FlagSize::Medium => "gfx/flags/medium",
            FlagSize::Small => "gfx/flags/small",
        }
    }

    /// Mod-relative path for a flag name such as `IRP` or `IRP_fascism`.
    pub fn path(self, flag_name: &str) -> Result<String> {
        validate_flag_name(flag_name)?;
        Ok(format!("{}/{flag_name}.tga", self.folder()))
    }
}

/// The three flag images.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlagLadder {
    /// 82x52 master.
    pub normal: Rgba,
    /// 41x26.
    pub medium: Rgba,
    /// 10x7.
    pub small: Rgba,
}

impl FlagLadder {
    /// The three TGA files, normal first.
    pub fn tga_files(&self, origin: Origin) -> [(FlagSize, Vec<u8>); 3] {
        [
            (FlagSize::Normal, write_truecolor32(&self.normal, origin)),
            (FlagSize::Medium, write_truecolor32(&self.medium, origin)),
            (FlagSize::Small, write_truecolor32(&self.small, origin)),
        ]
    }
}

/// Build the ladder from any source picture.
pub fn flag_ladder(source: &Rgba) -> Result<FlagLadder> {
    if source.width < 2 || source.height < 2 {
        return Err(Error::Invalid(format!("a {}x{} picture is too small to become a flag", source.width, source.height)));
    }
    let normal = if (source.width, source.height) == NORMAL_SIZE {
        source.clone()
    } else {
        apply(source, &Op::CoverResize { width: NORMAL_SIZE.0, height: NORMAL_SIZE.1 })?
    };
    let medium = resize_lanczos3(&normal, MEDIUM_SIZE.0, MEDIUM_SIZE.1)?;
    let small = resize_lanczos3(&normal, SMALL_SIZE.0, SMALL_SIZE.1)?;
    Ok(FlagLadder { normal, medium, small })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tga::{decode, parse_header};

    fn flag(w: u32, h: u32) -> Rgba {
        let mut img = Rgba::new(w, h);
        for y in 0..h {
            let band = if y < h / 3 {
                [200, 30, 30]
            } else if y < 2 * h / 3 {
                [240, 240, 240]
            } else {
                [30, 30, 200]
            };
            for x in 0..w {
                img.set_px(x, y, [band[0], band[1], band[2], 255]);
            }
        }
        img
    }

    #[test]
    fn ladder_has_the_three_vanilla_sizes() {
        let l = flag_ladder(&flag(82, 52)).unwrap();
        assert_eq!((l.normal.width, l.normal.height), (82, 52));
        assert_eq!((l.medium.width, l.medium.height), (41, 26));
        assert_eq!((l.small.width, l.small.height), (10, 7));
        assert_eq!(l.normal, flag(82, 52), "an 82x52 source is used as it is");
    }

    #[test]
    fn other_sizes_are_cover_resized() {
        let l = flag_ladder(&flag(600, 600)).unwrap();
        assert_eq!((l.normal.width, l.normal.height), (82, 52));
        let l = flag_ladder(&flag(1000, 100)).unwrap();
        assert_eq!((l.normal.width, l.normal.height), (82, 52));
        assert!(flag_ladder(&Rgba::new(1, 1)).is_err());
    }

    #[test]
    fn tga_files_are_32_bit_bottom_origin_and_decode_back() {
        let l = flag_ladder(&flag(164, 104)).unwrap();
        for (size, bytes) in l.tga_files(Origin::BottomLeft) {
            let info = parse_header(&bytes).unwrap();
            assert_eq!((info.width, info.height), size.size());
            assert_eq!(info.bits_per_pixel, 32);
            assert!(!info.top_origin);
            let img = decode(&bytes).unwrap();
            match size {
                FlagSize::Normal => assert_eq!(img, l.normal),
                FlagSize::Medium => assert_eq!(img, l.medium),
                FlagSize::Small => assert_eq!(img, l.small),
            }
        }
        let top = l.tga_files(Origin::TopLeft);
        assert!(parse_header(&top[0].1).unwrap().top_origin);
    }

    #[test]
    fn small_flag_keeps_the_stripes_in_order() {
        let l = flag_ladder(&flag(82, 52)).unwrap();
        let top = l.small.px(5, 0);
        let bottom = l.small.px(5, 6);
        assert!(top[0] > top[2] && bottom[2] > bottom[0], "{top:?} {bottom:?}");
    }

    #[test]
    fn paths_follow_the_game_folders() {
        assert_eq!(FlagSize::Medium.path("IRP_fascism").unwrap(), "gfx/flags/medium/IRP_fascism.tga");
        assert_eq!(FlagSize::Small.path("IRP").unwrap(), "gfx/flags/small/IRP.tga");
        assert!(FlagSize::Normal.path("iron_pact").is_err());
    }
}
