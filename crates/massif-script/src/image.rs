//! Images as WorldPainter's scripts read them (`wp.getHeightMap()`): the
//! raw samples of one channel, as Java's `Raster.getSample` hands them to
//! `BitmapHeightMap` — 0 to 255 for 8 bits, 0 to 65,535 for 16, 0 or 1 for
//! a 1-bit image, the palette index for an image with a palette — and, for
//! colour mappings, each pixel's ARGB as `BufferedImage.getRGB`.
//!
//! PNG is read by the `png` crate untouched (no palette expanded, no bits
//! widened), so masks of few bits keep the values WorldPainter saw; other
//! formats (JPEG, BMP, TIFF) by the `image` crate.

use std::fs::File;
use std::io::BufReader;
use std::path::Path;

/// What a file's header says, without decoding it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Header {
    pub width: u32,
    pub height: u32,
    /// Bits a sample.
    pub bits: u8,
    /// Channels a pixel: 1 for grey or a palette, 2 grey and alpha, 3 RGB,
    /// 4 RGBA.
    pub bands: u8,
    /// One band of grey of 8 bits or fewer (Java's `TYPE_BYTE_GRAY` or
    /// `TYPE_BYTE_BINARY`), on which WorldPainter refuses a channel.
    pub grey: bool,
}

/// One channel's samples, row by row.
#[derive(Debug, Clone)]
pub enum Samples {
    U8(Vec<u8>),
    U16(Vec<u16>),
    F32(Vec<f32>),
}

/// A decoded image: one channel of it.
#[derive(Debug, Clone)]
pub struct Image {
    pub width: u32,
    pub height: u32,
    pub samples: Samples,
}

impl Image {
    /// The sample of pixel `i` (row by row).
    #[inline]
    pub fn at(&self, i: usize) -> f32 {
        match &self.samples {
            Samples::U8(v) => v[i] as f32,
            Samples::U16(v) => v[i] as f32,
            Samples::F32(v) => v[i],
        }
    }

    /// The memory it takes.
    pub fn bytes(&self) -> usize {
        match &self.samples {
            Samples::U8(v) => v.len(),
            Samples::U16(v) => v.len() * 2,
            Samples::F32(v) => v.len() * 4,
        }
    }
}

fn is_png(path: &Path) -> bool {
    let mut magic = [0u8; 8];
    File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic)).is_ok() && magic == *b"\x89PNG\r\n\x1a\n"
}

fn png_reader(path: &Path) -> Result<png::Reader<BufReader<File>>, String> {
    let file = File::open(path).map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))?;
    let mut decoder = png::Decoder::new(BufReader::new(file));
    decoder.set_transformations(png::Transformations::IDENTITY);
    // Masks of 30,000 × 30,000 pixels are not unheard of.
    decoder.set_limits(png::Limits { bytes: 8 << 30 });
    decoder.read_info().map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))
}

fn bands(c: png::ColorType) -> u8 {
    match c {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    }
}

/// The header of the image at `path`.
pub fn header(path: &Path) -> Result<Header, String> {
    if is_png(path) {
        let reader = png_reader(path)?;
        let info = reader.info();
        let bits = info.bit_depth as u8;
        return Ok(Header { width: info.width, height: info.height, bits, bands: bands(info.color_type), grey: info.color_type == png::ColorType::Grayscale && bits <= 8 });
    }
    let reader = image::ImageReader::open(path).and_then(|r| r.with_guessed_format()).map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))?;
    let decoder = reader.into_decoder().map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))?;
    let (width, height) = image::ImageDecoder::dimensions(&decoder);
    let colour = image::ImageDecoder::color_type(&decoder);
    let bands = colour.channel_count();
    let bits = colour.bytes_per_pixel() / bands * 8;
    Ok(Header { width, height, bits, bands, grey: bands == 1 && bits <= 8 })
}

/// Decode channel `channel` of the image at `path`.
pub fn decode(path: &Path, channel: usize) -> Result<Image, String> {
    let fail = |e: String| format!("I/O error while loading image {}: {e}", path.display());
    if is_png(path) {
        let mut reader = png_reader(path)?;
        let size = reader.output_buffer_size().ok_or_else(|| fail("too large".into()))?;
        let mut buf = vec![0u8; size];
        let out = reader.next_frame(&mut buf).map_err(|e| fail(e.to_string()))?;
        let (w, h) = (out.width as usize, out.height as usize);
        let n = bands(out.color_type) as usize;
        if channel >= n {
            return Err(format!("Image {} has no channel {channel}", path.display()));
        }
        let bits = out.bit_depth as usize;
        let line = out.line_size;
        let samples = match bits {
            16 => {
                let mut v = Vec::with_capacity(w * h);
                for row in buf.chunks_exact(line).take(h) {
                    v.extend((0..w).map(|x| {
                        let k = (x * n + channel) * 2;
                        u16::from_be_bytes([row[k], row[k + 1]])
                    }));
                }
                Samples::U16(v)
            }
            8 => {
                let mut v = Vec::with_capacity(w * h);
                for row in buf.chunks_exact(line).take(h) {
                    if n == 1 {
                        v.extend_from_slice(&row[..w]);
                    } else {
                        v.extend((0..w).map(|x| row[x * n + channel]));
                    }
                }
                Samples::U8(v)
            }
            // 1, 2 or 4 bits: grey or a palette, one band, packed from the
            // high bits of each byte.
            _ => {
                let mask = (1u8 << bits) - 1;
                let per = 8 / bits;
                let mut v = Vec::with_capacity(w * h);
                for row in buf.chunks_exact(line).take(h) {
                    v.extend((0..w).map(|x| (row[x / per] >> (8 - bits * (x % per + 1))) & mask));
                }
                Samples::U8(v)
            }
        };
        return Ok(Image { width: w as u32, height: h as u32, samples });
    }
    let img = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| fail(e.to_string()))?
        .decode()
        .map_err(|e| fail(e.to_string()))?;
    let (w, h) = (img.width(), img.height());
    let n = img.color().channel_count() as usize;
    if channel >= n {
        return Err(format!("Image {} has no channel {channel}", path.display()));
    }
    let pick8 = |raw: &[u8]| -> Vec<u8> { if n == 1 { raw.to_vec() } else { raw.iter().skip(channel).step_by(n).copied().collect() } };
    let pick16 = |raw: &[u16]| -> Vec<u16> { if n == 1 { raw.to_vec() } else { raw.iter().skip(channel).step_by(n).copied().collect() } };
    let samples = match &img {
        image::DynamicImage::ImageLuma8(b) => Samples::U8(pick8(b.as_raw())),
        image::DynamicImage::ImageLumaA8(b) => Samples::U8(pick8(b.as_raw())),
        image::DynamicImage::ImageRgb8(b) => Samples::U8(pick8(b.as_raw())),
        image::DynamicImage::ImageRgba8(b) => Samples::U8(pick8(b.as_raw())),
        image::DynamicImage::ImageLuma16(b) => Samples::U16(pick16(b.as_raw())),
        image::DynamicImage::ImageLumaA16(b) => Samples::U16(pick16(b.as_raw())),
        image::DynamicImage::ImageRgb16(b) => Samples::U16(pick16(b.as_raw())),
        image::DynamicImage::ImageRgba16(b) => Samples::U16(pick16(b.as_raw())),
        other => {
            let f = other.to_rgba32f();
            Samples::F32(f.as_raw().iter().skip(channel.min(3)).step_by(4).copied().collect())
        }
    };
    Ok(Image { width: w, height: h, samples })
}

/// Every pixel's colour as `BufferedImage.getRGB` gives it: alpha, red,
/// green, blue, 8 bits each, the palette looked up and grey made colour.
pub fn decode_argb(path: &Path) -> Result<(u32, u32, Vec<u32>), String> {
    let img = image::ImageReader::open(path)
        .and_then(|r| r.with_guessed_format())
        .map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))?
        .decode()
        .map_err(|e| format!("I/O error while loading image {}: {e}", path.display()))?
        .to_rgba8();
    let (w, h) = img.dimensions();
    let argb = img.pixels().map(|p| ((p[3] as u32) << 24) | ((p[0] as u32) << 16) | ((p[1] as u32) << 8) | p[2] as u32).collect();
    Ok((w, h, argb))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_png(path: &Path, w: u32, h: u32, colour: png::ColorType, depth: png::BitDepth, data: &[u8], palette: Option<Vec<u8>>) {
        let file = File::create(path).unwrap();
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
        enc.set_color(colour);
        enc.set_depth(depth);
        if let Some(p) = palette {
            enc.set_palette(p);
        }
        enc.write_header().unwrap().write_image_data(data).unwrap();
    }

    /// A 1-bit mask reads 0 and 1, a palette its indices, 16 bits its
    /// values: what WorldPainter's scripts see of them.
    #[test]
    fn raw_samples_as_java_reads_them() {
        let dir = std::env::temp_dir().join(format!("massif-script-image-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let one = dir.join("one.png");
        // 10 pixels: 1010 0000 11 → packed 0b1010_0000, 0b1100_0000.
        write_png(&one, 10, 1, png::ColorType::Grayscale, png::BitDepth::One, &[0b1010_0000, 0b1100_0000], None);
        let img = decode(&one, 0).unwrap();
        let v: Vec<f32> = (0..10).map(|i| img.at(i)).collect();
        assert_eq!(v, vec![1.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 1.0]);
        let h = header(&one).unwrap();
        assert_eq!((h.width, h.height, h.bits, h.bands, h.grey), (10, 1, 1, 1, true));

        let pal = dir.join("pal.png");
        write_png(&pal, 3, 1, png::ColorType::Indexed, png::BitDepth::Eight, &[2, 0, 1], Some(vec![0, 0, 0, 255, 255, 255, 255, 0, 0]));
        let img = decode(&pal, 0).unwrap();
        assert_eq!((img.at(0), img.at(1), img.at(2)), (2.0, 0.0, 1.0));
        let (_, _, argb) = decode_argb(&pal).unwrap();
        assert_eq!(argb, vec![0xffff0000, 0xff000000, 0xffffffff]);

        let deep = dir.join("deep.png");
        write_png(&deep, 2, 1, png::ColorType::Grayscale, png::BitDepth::Sixteen, &[0x12, 0x34, 0xff, 0xff], None);
        let img = decode(&deep, 0).unwrap();
        assert_eq!((img.at(0), img.at(1)), (0x1234 as f32, 65535.0));

        let rgb = dir.join("rgb.png");
        write_png(&rgb, 1, 1, png::ColorType::Rgb, png::BitDepth::Eight, &[10, 20, 30], None);
        assert_eq!(decode(&rgb, 1).unwrap().at(0), 20.0);
        assert!(decode(&rgb, 3).is_err());
        std::fs::remove_dir_all(&dir).ok();
    }
}
