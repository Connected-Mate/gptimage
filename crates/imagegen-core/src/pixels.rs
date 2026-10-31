//! The in-memory pixel buffer and image decoding with hard limits.
//!
//! Everything downstream (processing, validation, DDS/TGA export) works on [`Rgba`]: 8-bit straight
//! (non-premultiplied) RGBA, row-major, top row first.

use std::io::Cursor;

use image::{DynamicImage, ImageEncoder, ImageFormat, ImageReader, Limits};

use crate::error::{Error, Result};
use crate::hashing::sha256_hex;

/// A straight-alpha RGBA8 image.
#[derive(Clone, PartialEq, Eq)]
pub struct Rgba {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// `width * height * 4` bytes, R G B A.
    pub data: Vec<u8>,
}

impl std::fmt::Debug for Rgba {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Rgba({}x{})", self.width, self.height)
    }
}

impl Rgba {
    /// A fully transparent image.
    pub fn new(width: u32, height: u32) -> Self {
        Rgba { width, height, data: vec![0; width as usize * height as usize * 4] }
    }

    /// An image filled with one colour.
    pub fn filled(width: u32, height: u32, px: [u8; 4]) -> Self {
        let mut data = Vec::with_capacity(width as usize * height as usize * 4);
        for _ in 0..(width as usize * height as usize) {
            data.extend_from_slice(&px);
        }
        Rgba { width, height, data }
    }

    /// Wrap raw bytes, checking the length.
    pub fn from_raw(width: u32, height: u32, data: Vec<u8>) -> Result<Self> {
        let expected = width as usize * height as usize * 4;
        if width == 0 || height == 0 || data.len() != expected {
            return Err(Error::Image(format!(
                "pixel buffer has {} bytes, expected {expected} for {width}x{height}",
                data.len()
            )));
        }
        Ok(Rgba { width, height, data })
    }

    /// Pixel at (x, y).
    #[inline]
    pub fn px(&self, x: u32, y: u32) -> [u8; 4] {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        [self.data[i], self.data[i + 1], self.data[i + 2], self.data[i + 3]]
    }

    /// Set the pixel at (x, y).
    #[inline]
    pub fn set_px(&mut self, x: u32, y: u32, px: [u8; 4]) {
        let i = (y as usize * self.width as usize + x as usize) * 4;
        self.data[i..i + 4].copy_from_slice(&px);
    }

    /// Alpha at (x, y).
    #[inline]
    pub fn alpha(&self, x: u32, y: u32) -> u8 {
        self.data[(y as usize * self.width as usize + x as usize) * 4 + 3]
    }

    /// SHA-256 of the RGBA bytes. Matches the kit's `pixels_sha256` (`image.convert('RGBA').tobytes()`),
    /// so a decoded DDS can be compared with the recorded processed pixels.
    pub fn pixels_sha256(&self) -> String {
        sha256_hex(&self.data)
    }

    /// Copy a rectangle. The rectangle must lie inside the image.
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Result<Rgba> {
        if w == 0 || h == 0 || x + w > self.width || y + h > self.height {
            return Err(Error::Image(format!(
                "crop {w}x{h}+{x}+{y} is outside {}x{}",
                self.width, self.height
            )));
        }
        let mut out = Rgba::new(w, h);
        for row in 0..h {
            let src = ((y + row) as usize * self.width as usize + x as usize) * 4;
            let dst = row as usize * w as usize * 4;
            out.data[dst..dst + w as usize * 4].copy_from_slice(&self.data[src..src + w as usize * 4]);
        }
        Ok(out)
    }

    /// Overwrite a region with `src`, clipping to the image. Offsets may be negative.
    pub fn paste(&mut self, src: &Rgba, x: i64, y: i64) {
        for sy in 0..src.height as i64 {
            let dy = y + sy;
            if dy < 0 || dy >= self.height as i64 {
                continue;
            }
            for sx in 0..src.width as i64 {
                let dx = x + sx;
                if dx < 0 || dx >= self.width as i64 {
                    continue;
                }
                self.set_px(dx as u32, dy as u32, src.px(sx as u32, sy as u32));
            }
        }
    }

    /// Composite `src` over this image (straight alpha "over"), clipping to the image.
    pub fn paste_over(&mut self, src: &Rgba, x: i64, y: i64) {
        for sy in 0..src.height as i64 {
            let dy = y + sy;
            if dy < 0 || dy >= self.height as i64 {
                continue;
            }
            for sx in 0..src.width as i64 {
                let dx = x + sx;
                if dx < 0 || dx >= self.width as i64 {
                    continue;
                }
                let s = src.px(sx as u32, sy as u32);
                let d = self.px(dx as u32, dy as u32);
                self.set_px(dx as u32, dy as u32, over(s, d));
            }
        }
    }

    /// Bounding box `[x0, y0, x1, y1)` of pixels whose alpha is at least `min_alpha`.
    pub fn alpha_bbox(&self, min_alpha: u8) -> Option<[u32; 4]> {
        let (mut x0, mut y0, mut x1, mut y1) = (u32::MAX, u32::MAX, 0u32, 0u32);
        let mut any = false;
        for y in 0..self.height {
            for x in 0..self.width {
                if self.alpha(x, y) >= min_alpha && self.alpha(x, y) > 0 {
                    any = true;
                    x0 = x0.min(x);
                    y0 = y0.min(y);
                    x1 = x1.max(x + 1);
                    y1 = y1.max(y + 1);
                }
            }
        }
        any.then_some([x0, y0, x1, y1])
    }

    /// Split a horizontal strip into `count` equal frames.
    pub fn split_frames(&self, count: u32) -> Result<Vec<Rgba>> {
        if count == 0 || self.width % count != 0 {
            return Err(Error::Image(format!(
                "width {} does not divide into {count} frames",
                self.width
            )));
        }
        let fw = self.width / count;
        (0..count).map(|i| self.crop(i * fw, 0, fw, self.height)).collect()
    }

    /// Join equal-sized frames into a horizontal strip.
    pub fn join_frames(frames: &[Rgba]) -> Result<Rgba> {
        let first = frames.first().ok_or_else(|| Error::Image("no frames to join".into()))?;
        if frames.iter().any(|f| f.width != first.width || f.height != first.height) {
            return Err(Error::Image("frames differ in size".into()));
        }
        let mut out = Rgba::new(first.width * frames.len() as u32, first.height);
        for (i, f) in frames.iter().enumerate() {
            out.paste(f, i64::from(first.width) * i as i64, 0);
        }
        Ok(out)
    }

    /// True when any pixel has alpha below 255.
    pub fn has_transparency(&self) -> bool {
        self.data.chunks_exact(4).any(|p| p[3] < 255)
    }
}

/// Straight-alpha "over": `s` composited on `d`.
pub fn over(s: [u8; 4], d: [u8; 4]) -> [u8; 4] {
    let sa = f32::from(s[3]) / 255.0;
    let da = f32::from(d[3]) / 255.0;
    let oa = sa + da * (1.0 - sa);
    if oa <= 0.0 {
        return [0, 0, 0, 0];
    }
    let mut out = [0u8; 4];
    for c in 0..3 {
        let v = (f32::from(s[c]) * sa + f32::from(d[c]) * da * (1.0 - sa)) / oa;
        out[c] = v.round().clamp(0.0, 255.0) as u8;
    }
    out[3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
    out
}

/// Container formats this crate reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceFormat {
    /// PNG
    Png,
    /// JPEG
    Jpeg,
    /// WebP
    WebP,
    /// DirectDraw Surface (uncompressed A8R8G8B8 only)
    Dds,
    /// Targa (uncompressed truecolor only)
    Tga,
}

impl SourceFormat {
    /// Lowercase name.
    pub fn name(self) -> &'static str {
        match self {
            SourceFormat::Png => "png",
            SourceFormat::Jpeg => "jpeg",
            SourceFormat::WebP => "webp",
            SourceFormat::Dds => "dds",
            SourceFormat::Tga => "tga",
        }
    }

    /// MIME type for provider requests.
    pub fn mime(self) -> &'static str {
        match self {
            SourceFormat::Png => "image/png",
            SourceFormat::Jpeg => "image/jpeg",
            SourceFormat::WebP => "image/webp",
            SourceFormat::Dds => "image/vnd.ms-dds",
            SourceFormat::Tga => "image/x-tga",
        }
    }
}

/// Hard limits applied before and during decoding.
#[derive(Debug, Clone, Copy)]
pub struct DecodeLimits {
    /// Largest accepted input in bytes.
    pub max_bytes: usize,
    /// Largest accepted width or height.
    pub max_dimension: u32,
    /// Largest accepted pixel count.
    pub max_pixels: u64,
}

impl Default for DecodeLimits {
    fn default() -> Self {
        DecodeLimits { max_bytes: 32 * 1024 * 1024, max_dimension: 8192, max_pixels: 40_000_000 }
    }
}

/// Identify the container from magic bytes. Text and markup (SVG, HTML) are refused outright.
pub fn sniff(bytes: &[u8], file_name_hint: Option<&str>) -> Result<SourceFormat> {
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Ok(SourceFormat::Png);
    }
    if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        return Ok(SourceFormat::Jpeg);
    }
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        return Ok(SourceFormat::WebP);
    }
    if bytes.starts_with(b"DDS ") {
        return Ok(SourceFormat::Dds);
    }
    let head: Vec<u8> = bytes.iter().copied().take(256).skip_while(|b| b.is_ascii_whitespace()).collect();
    if head.first() == Some(&b'<') {
        return Err(Error::Image("markup (SVG or HTML) is not an accepted image".into()));
    }
    let tga_hint = file_name_hint.map(|n| n.to_ascii_lowercase().ends_with(".tga")).unwrap_or(false);
    if tga_hint && bytes.len() >= 18 {
        return Ok(SourceFormat::Tga);
    }
    if bytes.starts_with(b"version https://git-lfs") {
        return Err(Error::Image("file is a Git LFS pointer, not image data".into()));
    }
    Err(Error::Image("unrecognized image format (PNG, JPEG, WebP, DDS and TGA are accepted)".into()))
}

fn image_format(f: SourceFormat) -> Option<ImageFormat> {
    match f {
        SourceFormat::Png => Some(ImageFormat::Png),
        SourceFormat::Jpeg => Some(ImageFormat::Jpeg),
        SourceFormat::WebP => Some(ImageFormat::WebP),
        _ => None,
    }
}

/// Decode an image of any supported container, enforcing the limits first.
pub fn decode(bytes: &[u8], file_name_hint: Option<&str>, limits: DecodeLimits) -> Result<(Rgba, SourceFormat)> {
    if bytes.len() > limits.max_bytes {
        return Err(Error::Image(format!("image is {} bytes, the limit is {}", bytes.len(), limits.max_bytes)));
    }
    let format = sniff(bytes, file_name_hint)?;
    let img = match format {
        SourceFormat::Dds => crate::dds::decode(bytes)?,
        SourceFormat::Tga => crate::tga::decode(bytes)?,
        other => {
            let fmt = image_format(other).expect("raster format");
            let (w, h) = ImageReader::with_format(Cursor::new(bytes), fmt)
                .into_dimensions()
                .map_err(|e| Error::Image(format!("cannot read {} header: {e}", other.name())))?;
            check_dimensions(w, h, &limits)?;
            let mut reader = ImageReader::with_format(Cursor::new(bytes), fmt);
            let mut lim = Limits::default();
            lim.max_image_width = Some(limits.max_dimension);
            lim.max_image_height = Some(limits.max_dimension);
            lim.max_alloc = Some(limits.max_pixels * 8);
            reader.limits(lim);
            let dynimg: DynamicImage =
                reader.decode().map_err(|e| Error::Image(format!("cannot decode {}: {e}", other.name())))?;
            let rgba = dynimg.into_rgba8();
            Rgba::from_raw(rgba.width(), rgba.height(), rgba.into_raw())?
        }
    };
    check_dimensions(img.width, img.height, &limits)?;
    Ok((img, format))
}

fn check_dimensions(w: u32, h: u32, limits: &DecodeLimits) -> Result<()> {
    if w == 0 || h == 0 || w > limits.max_dimension || h > limits.max_dimension {
        return Err(Error::Image(format!(
            "image is {w}x{h}; each side must be 1 to {} pixels",
            limits.max_dimension
        )));
    }
    if u64::from(w) * u64::from(h) > limits.max_pixels {
        return Err(Error::Image(format!("image has {} pixels, the limit is {}", u64::from(w) * u64::from(h), limits.max_pixels)));
    }
    Ok(())
}

/// Encode as PNG. Deterministic for a given crate version.
pub fn encode_png(img: &Rgba) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    let enc = image::codecs::png::PngEncoder::new_with_quality(
        &mut out,
        image::codecs::png::CompressionType::Default,
        image::codecs::png::FilterType::Adaptive,
    );
    enc.write_image(&img.data, img.width, img.height, image::ExtendedColorType::Rgba8)
        .map_err(|e| Error::Image(format!("cannot encode PNG: {e}")))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_round_trip_keeps_pixels() {
        let mut img = Rgba::new(5, 3);
        img.set_px(1, 1, [10, 20, 30, 200]);
        img.set_px(4, 2, [255, 0, 255, 255]);
        let png = encode_png(&img).unwrap();
        let (back, fmt) = decode(&png, Some("x.png"), DecodeLimits::default()).unwrap();
        assert_eq!(fmt, SourceFormat::Png);
        assert_eq!(back, img);
    }

    #[test]
    fn svg_and_unknown_are_rejected() {
        assert!(sniff(b"  <svg xmlns='x'/>", None).is_err());
        assert!(sniff(b"<!doctype html>", None).is_err());
        assert!(sniff(b"hello world", None).is_err());
        assert!(sniff(b"version https://git-lfs.github.com/spec/v1", None).is_err());
    }

    #[test]
    fn limits_are_enforced() {
        let img = Rgba::new(40, 40);
        let png = encode_png(&img).unwrap();
        let lim = DecodeLimits { max_dimension: 32, ..Default::default() };
        assert!(decode(&png, None, lim).is_err());
        let lim = DecodeLimits { max_bytes: 10, ..Default::default() };
        assert!(decode(&png, None, lim).is_err());
    }

    #[test]
    fn truncated_png_fails_cleanly() {
        let png = encode_png(&Rgba::filled(16, 16, [1, 2, 3, 255])).unwrap();
        assert!(decode(&png[..png.len() / 2], None, DecodeLimits::default()).is_err());
    }

    #[test]
    fn frames_split_and_join() {
        let mut a = Rgba::new(4, 2);
        a.set_px(0, 0, [1, 1, 1, 255]);
        let mut b = Rgba::new(4, 2);
        b.set_px(3, 1, [2, 2, 2, 255]);
        let strip = Rgba::join_frames(&[a.clone(), b.clone()]).unwrap();
        assert_eq!((strip.width, strip.height), (8, 2));
        let parts = strip.split_frames(2).unwrap();
        assert_eq!(parts[0], a);
        assert_eq!(parts[1], b);
        assert!(strip.split_frames(3).is_err());
    }

    #[test]
    fn alpha_bbox_respects_threshold() {
        let mut img = Rgba::new(10, 10);
        img.set_px(2, 3, [0, 0, 0, 10]);
        img.set_px(6, 7, [0, 0, 0, 200]);
        assert_eq!(img.alpha_bbox(1), Some([2, 3, 7, 8]));
        assert_eq!(img.alpha_bbox(16), Some([6, 7, 7, 8]));
        assert_eq!(Rgba::new(3, 3).alpha_bbox(1), None);
    }
}
