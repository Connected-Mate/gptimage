//! Uncompressed 32-bit truecolor TGA writer and reader, used for HOI4 flags.
//!
//! HOI4 flags are looked up by file name (`gfx/flags/<TAG>.tga`, `medium/`, `small/`). The kit's flag
//! rule is bottom-origin; vanilla 1.19.3.0 ships 5 of 2,203 top-origin files in the normal folder,
//! 221 (10.0 percent) in `medium` and 3 in `small`, so the engine loads both, and the writer lets the
//! caller choose.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Where row zero of the pixel data is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// Rows are stored bottom to top (descriptor bit 5 clear). The kit default.
    BottomLeft,
    /// Rows are stored top to bottom (descriptor bit 5 set).
    TopLeft,
}

/// Header length.
pub const HEADER_LEN: usize = 18;

/// Write an uncompressed 32-bit TGA with an 8-bit alpha channel.
pub fn write_truecolor32(img: &Rgba, origin: Origin) -> Vec<u8> {
    let mut out = Vec::with_capacity(HEADER_LEN + img.data.len());
    out.extend_from_slice(&[0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0]);
    out.extend_from_slice(&(img.width as u16).to_le_bytes());
    out.extend_from_slice(&(img.height as u16).to_le_bytes());
    out.push(32);
    out.push(match origin {
        Origin::BottomLeft => 0x08,
        Origin::TopLeft => 0x28,
    });
    let rows: Vec<u32> = match origin {
        Origin::BottomLeft => (0..img.height).rev().collect(),
        Origin::TopLeft => (0..img.height).collect(),
    };
    for y in rows {
        for x in 0..img.width {
            let p = img.px(x, y);
            out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
        }
    }
    out
}

/// Parsed header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TgaInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Bits per pixel.
    pub bits_per_pixel: u8,
    /// Image type code (2 = uncompressed truecolor, 10 = RLE truecolor).
    pub image_type: u8,
    /// True when rows are stored top to bottom.
    pub top_origin: bool,
    /// Length of the image ID field.
    pub id_len: usize,
}

/// Read the 18-byte header.
pub fn parse_header(bytes: &[u8]) -> Result<TgaInfo> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::Image("TGA header is truncated".into()));
    }
    Ok(TgaInfo {
        width: u32::from(u16::from_le_bytes([bytes[12], bytes[13]])),
        height: u32::from(u16::from_le_bytes([bytes[14], bytes[15]])),
        bits_per_pixel: bytes[16],
        image_type: bytes[2],
        top_origin: bytes[17] & 0x20 != 0,
        id_len: usize::from(bytes[0]),
    })
}

/// Decode an uncompressed 24 or 32-bit truecolor TGA.
pub fn decode(bytes: &[u8]) -> Result<Rgba> {
    let info = parse_header(bytes)?;
    if info.image_type != 2 || !(info.bits_per_pixel == 24 || info.bits_per_pixel == 32) {
        return Err(Error::Image(format!(
            "only uncompressed 24/32-bit truecolor TGA can be decoded (type {}, {} bpp)",
            info.image_type, info.bits_per_pixel
        )));
    }
    if info.width == 0 || info.height == 0 {
        return Err(Error::Image("TGA has zero size".into()));
    }
    let bpp = usize::from(info.bits_per_pixel / 8);
    let start = HEADER_LEN + info.id_len;
    let need = info.width as usize * info.height as usize * bpp;
    if bytes.len() < start + need {
        return Err(Error::Image(format!(
            "TGA holds {} bytes, its header needs at least {}",
            bytes.len(),
            start + need
        )));
    }
    let mut img = Rgba::new(info.width, info.height);
    for row in 0..info.height {
        let y = if info.top_origin { row } else { info.height - 1 - row };
        for x in 0..info.width {
            let o = start + (row as usize * info.width as usize + x as usize) * bpp;
            let a = if bpp == 4 { bytes[o + 3] } else { 255 };
            img.set_px(x, y, [bytes[o + 2], bytes[o + 1], bytes[o], a]);
        }
    }
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rgba {
        let mut img = Rgba::new(4, 3);
        img.set_px(0, 0, [10, 20, 30, 255]); // top-left
        img.set_px(3, 2, [200, 100, 50, 128]); // bottom-right
        img
    }

    #[test]
    fn bottom_origin_stores_the_last_row_first() {
        let tga = write_truecolor32(&sample(), Origin::BottomLeft);
        assert_eq!(tga.len(), 18 + 4 * 3 * 4);
        assert_eq!(tga[2], 2);
        assert_eq!(tga[16], 32);
        assert_eq!(tga[17], 0x08);
        // first stored pixel is the bottom-left one (transparent black), the bottom-right is fourth
        assert_eq!(&tga[18 + 3 * 4..18 + 4 * 4], &[50, 100, 200, 128]);
        assert!(!parse_header(&tga).unwrap().top_origin);
    }

    #[test]
    fn top_origin_flag_and_round_trip() {
        for origin in [Origin::BottomLeft, Origin::TopLeft] {
            let img = sample();
            let tga = write_truecolor32(&img, origin);
            let info = parse_header(&tga).unwrap();
            assert_eq!(info.top_origin, origin == Origin::TopLeft);
            assert_eq!(decode(&tga).unwrap(), img);
        }
    }

    #[test]
    fn truncated_and_unsupported_fail() {
        let tga = write_truecolor32(&sample(), Origin::BottomLeft);
        assert!(decode(&tga[..tga.len() - 1]).is_err());
        let mut rle = tga.clone();
        rle[2] = 10;
        assert!(decode(&rle).is_err());
        assert!(parse_header(&tga[..10]).is_err());
    }
}
