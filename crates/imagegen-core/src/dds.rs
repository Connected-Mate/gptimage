//! Uncompressed A8R8G8B8 DDS writer and header reader.
//!
//! The writer produces the canonical one-level legacy BGRA layout that the Agentic HOI4 Modding Kit's
//! `convert_to_dds.py` (`write_bgra_dds`) writes and its `gfx.py` `strict_dds` accepts: a 128-byte
//! header, `RGB | ALPHAPIXELS`, straight alpha, `DDSCAPS_TEXTURE`, file length `128 + w * h * 4`.
//! Vanilla 1.19.3.0: 95.8 percent of 15,126 surveyed DDS files use this format.

use crate::error::{Error, Result};
use crate::pixels::Rgba;

/// Size of the legacy DDS header including the magic.
pub const HEADER_LEN: usize = 128;

const DDSD_CAPS: u32 = 0x1;
const DDSD_HEIGHT: u32 = 0x2;
const DDSD_WIDTH: u32 = 0x4;
const DDSD_PITCH: u32 = 0x8;
const DDSD_PIXELFORMAT: u32 = 0x1000;
const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_RGB: u32 = 0x40;
const DDSCAPS_TEXTURE: u32 = 0x1000;

/// Write an uncompressed A8R8G8B8 DDS with one mip level.
pub fn write_a8r8g8b8(img: &Rgba) -> Vec<u8> {
    let (w, h) = (img.width, img.height);
    let words: [u32; 31] = [
        124,
        DDSD_CAPS | DDSD_HEIGHT | DDSD_WIDTH | DDSD_PITCH | DDSD_PIXELFORMAT,
        h,
        w,
        w * 4,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0, // reserved1[11]
        32,
        DDPF_RGB | DDPF_ALPHAPIXELS,
        0,
        32,
        0x00FF_0000,
        0x0000_FF00,
        0x0000_00FF,
        0xFF00_0000,
        DDSCAPS_TEXTURE,
        0,
        0,
        0,
        0, // caps2, caps3, caps4, reserved2
    ];
    let mut out = Vec::with_capacity(HEADER_LEN + img.data.len());
    out.extend_from_slice(b"DDS ");
    for word in words {
        out.extend_from_slice(&word.to_le_bytes());
    }
    for px in img.data.chunks_exact(4) {
        out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    out
}

/// Pixel formats the header reader distinguishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DdsFormat {
    /// Uncompressed 32-bit with masks R=00FF0000 G=0000FF00 B=000000FF A=FF000000.
    A8R8G8B8,
    /// A FourCC-compressed format such as DXT1/DXT3/DXT5.
    FourCc(String),
    /// A DX10 extension header follows the legacy header.
    Dx10,
    /// Any other layout.
    Other,
}

/// Parsed header facts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DdsInfo {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Mip level count (0 in the header means one level).
    pub mip_levels: u32,
    /// Pixel format.
    pub format: DdsFormat,
    /// Header flags word.
    pub flags: u32,
    /// Total file length in bytes.
    pub file_len: usize,
}

fn word(bytes: &[u8], index: usize) -> u32 {
    let o = 4 + index * 4;
    u32::from_le_bytes([bytes[o], bytes[o + 1], bytes[o + 2], bytes[o + 3]])
}

/// Read the header. Needs at least 128 bytes.
pub fn parse_header(bytes: &[u8]) -> Result<DdsInfo> {
    if bytes.len() < HEADER_LEN {
        return Err(Error::Image("DDS header is truncated".into()));
    }
    if &bytes[0..4] != b"DDS " {
        return Err(Error::Image("missing DDS magic".into()));
    }
    let flags = word(bytes, 1);
    let height = word(bytes, 2);
    let width = word(bytes, 3);
    let mips = word(bytes, 6);
    let pf_flags = word(bytes, 19);
    let fourcc = &bytes[4 + 20 * 4..4 + 20 * 4 + 4];
    let bit_count = word(bytes, 21);
    let masks = [word(bytes, 22), word(bytes, 23), word(bytes, 24), word(bytes, 25)];
    let format = if pf_flags & 0x4 != 0 {
        if fourcc == b"DX10" {
            DdsFormat::Dx10
        } else {
            DdsFormat::FourCc(String::from_utf8_lossy(fourcc).trim_end_matches('\0').to_string())
        }
    } else if pf_flags & DDPF_RGB != 0
        && pf_flags & DDPF_ALPHAPIXELS != 0
        && bit_count == 32
        && masks == [0x00FF_0000, 0x0000_FF00, 0x0000_00FF, 0xFF00_0000]
    {
        DdsFormat::A8R8G8B8
    } else {
        DdsFormat::Other
    };
    Ok(DdsInfo { width, height, mip_levels: mips.max(1), format, flags, file_len: bytes.len() })
}

/// Reasons a file is not the canonical layout this tool writes. Empty means canonical.
pub fn canonical_problems(bytes: &[u8]) -> Vec<String> {
    let mut problems = Vec::new();
    let info = match parse_header(bytes) {
        Ok(i) => i,
        Err(e) => return vec![e.to_string()],
    };
    if info.width == 0 || info.height == 0 {
        problems.push("zero width or height".into());
        return problems;
    }
    let expected: [u32; 31] = [
        124,
        0x100F,
        info.height,
        info.width,
        info.width.wrapping_mul(4),
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        0,
        32,
        65,
        0,
        32,
        0x00FF_0000,
        0x0000_FF00,
        0x0000_00FF,
        0xFF00_0000,
        0x1000,
        0,
        0,
        0,
        0,
    ];
    let names = ["size", "flags", "height", "width", "pitch"];
    for (i, want) in expected.iter().enumerate() {
        let got = word(bytes, i);
        if got != *want {
            let label = names.get(i).map(|s| s.to_string()).unwrap_or_else(|| format!("word {i}"));
            problems.push(format!("header {label} is {got:#x}, canonical is {want:#x}"));
        }
    }
    let want_len = HEADER_LEN + info.width as usize * info.height as usize * 4;
    if bytes.len() != want_len {
        problems.push(format!("file is {} bytes, canonical length is {want_len}", bytes.len()));
    }
    problems
}

/// Decode an uncompressed A8R8G8B8 DDS (first mip level only).
pub fn decode(bytes: &[u8]) -> Result<Rgba> {
    let info = parse_header(bytes)?;
    if info.format != DdsFormat::A8R8G8B8 {
        return Err(Error::Image(format!("only uncompressed A8R8G8B8 DDS can be decoded, this file is {:?}", info.format)));
    }
    if info.width == 0 || info.height == 0 || info.width > 16384 || info.height > 16384 {
        return Err(Error::Image(format!("implausible DDS size {}x{}", info.width, info.height)));
    }
    let need = info.width as usize * info.height as usize * 4;
    let payload = &bytes[HEADER_LEN..];
    if payload.len() < need {
        return Err(Error::Image(format!("DDS payload has {} bytes, the header needs {need}", payload.len())));
    }
    let mut data = Vec::with_capacity(need);
    for px in payload[..need].chunks_exact(4) {
        data.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
    }
    Rgba::from_raw(info.width, info.height, data)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Rgba {
        let mut img = Rgba::new(3, 2);
        img.set_px(0, 0, [1, 2, 3, 4]);
        img.set_px(2, 1, [250, 128, 7, 255]);
        img
    }

    #[test]
    fn layout_matches_the_documented_canonical_form() {
        let dds = write_a8r8g8b8(&sample());
        assert_eq!(dds.len(), 128 + 3 * 2 * 4);
        assert_eq!(&dds[0..4], b"DDS ");
        assert_eq!(word(&dds, 0), 124);
        assert_eq!(word(&dds, 1), 0x100F);
        assert_eq!(word(&dds, 2), 2); // height
        assert_eq!(word(&dds, 3), 3); // width
        assert_eq!(word(&dds, 4), 12); // pitch
        assert_eq!(word(&dds, 18), 32);
        assert_eq!(word(&dds, 19), 65);
        assert_eq!(word(&dds, 25), 0xFF00_0000);
        assert_eq!(word(&dds, 26), 0x1000);
        // first pixel stored as B G R A
        assert_eq!(&dds[128..132], &[3, 2, 1, 4]);
        assert!(canonical_problems(&dds).is_empty());
    }

    #[test]
    fn round_trip_is_lossless() {
        let img = sample();
        let back = decode(&write_a8r8g8b8(&img)).unwrap();
        assert_eq!(back, img);
        assert_eq!(back.pixels_sha256(), img.pixels_sha256());
    }

    #[test]
    fn parse_reports_format_and_size() {
        let info = parse_header(&write_a8r8g8b8(&sample())).unwrap();
        assert_eq!((info.width, info.height, info.mip_levels), (3, 2, 1));
        assert_eq!(info.format, DdsFormat::A8R8G8B8);
    }

    #[test]
    fn noncanonical_and_truncated_are_reported() {
        let mut dds = write_a8r8g8b8(&sample());
        dds.truncate(dds.len() - 4);
        let problems = canonical_problems(&dds);
        assert!(problems.iter().any(|p| p.contains("canonical length")));
        assert!(decode(&dds).is_err());
        assert!(parse_header(&dds[..64]).is_err());
        let mut mipped = write_a8r8g8b8(&sample());
        mipped[4 + 6 * 4] = 3; // mipmap count
        assert!(!canonical_problems(&mipped).is_empty());
    }
}
