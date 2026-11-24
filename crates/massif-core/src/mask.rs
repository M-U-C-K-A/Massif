//! Import mask: an image painted onto the map, as WorldPainter's Import
//! mask. Its pixels set the annotations, a layer, a terrain, a biome, the
//! water or the height of the columns they land on.
//!
//! Images are read a row at a time and never held whole: PNG (8 or 16
//! bits, grey, colour, palette, with or without alpha), TIFF (8, 16 or
//! 32-bit integers or floats, in strips or tiles), BMP (1 to 32 bits, not
//! compressed). JPEG, and BMP of a compressed kind, are decoded whole at
//! their 8 bits, then handed out by rows the same way. An import reads the
//! file twice: [`survey`] once, for the range of its values and a small
//! picture of it, then [`apply`] once more, a band of 128 rows of the map
//! at a time while a thread of its own decodes the next rows.

use crate::auto::{Changes, Steps};
use crate::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use crate::history::Edit;
use crate::registry::Slot;
use crate::tile::{ANNOTATION_COLORS, NO_FLUID, Tile};
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

/// The longest side of the small picture [`survey`] makes of an image,
/// to show what an import does.
pub const PREVIEW_SIDE: usize = 512;
/// The longest side of the larger one, to trace over on the map.
pub const GUIDE_SIDE: usize = 2048;

/// One row of an image as the readers hand it out.
#[derive(Debug, Clone, Default)]
pub struct Row {
    /// Each pixel's grey, in the image's own units (0 to 255 for 8 bits,
    /// 0 to 65,535 for 16, as stored for floats); for colour, its luma.
    pub grey: Vec<f32>,
    /// Each pixel's colour at 8 bits, for an image in colour (else empty).
    pub rgb: Vec<[u8; 3]>,
    /// Each pixel's opacity, 0 to 255, for an image with alpha (else empty).
    pub alpha: Vec<u8>,
}

impl Row {
    fn clear(&mut self) {
        self.grey.clear();
        self.rgb.clear();
        self.alpha.clear();
    }

    #[inline]
    fn opaque(&self, i: usize) -> bool {
        self.alpha.is_empty() || self.alpha[i] >= 128
    }
}

/// What a file holds, as its header says.
#[derive(Debug, Clone, PartialEq)]
pub struct Header {
    pub width: usize,
    pub height: usize,
    /// Bits a sample: 1 to 32.
    pub bits: u8,
    pub colour: bool,
    pub alpha: bool,
    /// Samples of floating point.
    pub float: bool,
    /// The kind of file: "PNG", "TIFF", "BMP", "JPEG".
    pub format: &'static str,
    /// Compressed with loss: its blacks are not quite black.
    pub lossy: bool,
}

trait Rows: Send {
    fn header(&self) -> &Header;
    /// Fill `row` with the next row; false at the end.
    fn next(&mut self, row: &mut Row) -> Result<bool, String>;
}

fn luma(r: f32, g: f32, b: f32) -> f32 {
    0.299 * r + 0.587 * g + 0.114 * b
}

/// Open an image for reading by rows, its kind told by its first bytes.
fn open(path: &Path) -> Result<Box<dyn Rows>, String> {
    let mut magic = [0u8; 8];
    let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let n = f.read(&mut magic).map_err(|e| e.to_string())?;
    let magic = &magic[..n];
    if magic.starts_with(b"\x89PNG") {
        PngRows::open(path)
    } else if magic.starts_with(b"II*\0") || magic.starts_with(b"MM\0*") || magic.starts_with(b"II+\0") || magic.starts_with(b"MM\0+") {
        TiffRows::open(path)
    } else if magic.starts_with(b"BM") {
        match BmpRows::open(path)? {
            Some(rows) => Ok(Box::new(rows)),
            None => WholeRows::open(path, "BMP"),
        }
    } else if magic.starts_with(&[0xff, 0xd8]) {
        WholeRows::open(path, "JPEG")
    } else {
        Err("not an image Massif reads: PNG, JPEG, TIFF or BMP".into())
    }
}

/// Fill `row` from interleaved samples, `channels` a pixel (1: grey, 2:
/// grey and alpha, 3: RGB, 4: RGBA), `max` the full scale of a sample.
fn fill_row<T: Copy>(row: &mut Row, samples: &[T], channels: usize, max: f32, value: impl Fn(T) -> f32) {
    row.clear();
    let to8 = 255.0 / max;
    match channels {
        1 => row.grey.extend(samples.iter().map(|&s| value(s))),
        2 => {
            for p in samples.as_chunks::<2>().0 {
                row.grey.push(value(p[0]));
                row.alpha.push((value(p[1]) * to8).round().clamp(0.0, 255.0) as u8);
            }
        }
        _ => {
            for p in samples.chunks_exact(channels) {
                let (r, g, b) = (value(p[0]), value(p[1]), value(p[2]));
                row.grey.push(luma(r, g, b));
                row.rgb.push([(r * to8).round().clamp(0.0, 255.0) as u8, (g * to8).round().clamp(0.0, 255.0) as u8, (b * to8).round().clamp(0.0, 255.0) as u8]);
                if channels == 4 {
                    row.alpha.push((value(p[3]) * to8).round().clamp(0.0, 255.0) as u8);
                }
            }
        }
    }
}

struct PngRows {
    reader: png::Reader<BufReader<std::fs::File>>,
    header: Header,
    channels: usize,
    /// An interlaced image, decoded whole: rows come out of it.
    whole: Option<Vec<u8>>,
    line: usize,
    y: usize,
}

impl PngRows {
    fn open(path: &Path) -> Result<Box<dyn Rows>, String> {
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut dec = png::Decoder::new_with_limits(BufReader::new(file), png::Limits { bytes: usize::MAX });
        dec.set_transformations(png::Transformations::EXPAND);
        let mut reader = dec.read_info().map_err(|e| e.to_string())?;
        let (ct, depth) = reader.output_color_type();
        let channels = match ct {
            png::ColorType::Grayscale => 1,
            png::ColorType::GrayscaleAlpha => 2,
            png::ColorType::Rgb => 3,
            png::ColorType::Rgba => 4,
            png::ColorType::Indexed => return Err("a PNG palette that could not be expanded".into()),
        };
        let bits = if depth == png::BitDepth::Sixteen { 16 } else { 8 };
        let info = reader.info();
        let (width, height) = (info.width as usize, info.height as usize);
        let line = width * channels * bits / 8;
        let whole = if info.interlaced {
            let mut buf = vec![0; reader.output_buffer_size().ok_or("a PNG too large")?];
            reader.next_frame(&mut buf).map_err(|e| e.to_string())?;
            Some(buf)
        } else {
            None
        };
        let header = Header {
            width,
            height,
            bits: bits as u8,
            colour: channels >= 3,
            alpha: channels == 2 || channels == 4,
            float: false,
            format: "PNG",
            lossy: false,
        };
        Ok(Box::new(Self { reader, header, channels, whole, line, y: 0 }))
    }
}

impl Rows for PngRows {
    fn header(&self) -> &Header {
        &self.header
    }

    fn next(&mut self, row: &mut Row) -> Result<bool, String> {
        if self.y >= self.header.height {
            return Ok(false);
        }
        let bytes: &[u8] = match &self.whole {
            Some(buf) => &buf[self.y * self.line..(self.y + 1) * self.line],
            None => match self.reader.next_row().map_err(|e| e.to_string())? {
                Some(r) => r.data(),
                None => return Ok(false),
            },
        };
        if self.header.bits == 16 {
            let samples: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_be_bytes(*b)).collect();
            fill_row(row, &samples, self.channels, 65535.0, |s| s as f32);
        } else {
            fill_row(row, bytes, self.channels, 255.0, |s| s as f32);
        }
        self.y += 1;
        Ok(true)
    }
}

struct TiffRows {
    dec: tiff::decoder::Decoder<BufReader<std::fs::File>>,
    header: Header,
    channels: usize,
    max: f32,
    /// The size of a chunk (a strip or a tile), and how many there are across.
    chunk: (usize, usize),
    across: usize,
    /// The band of rows of the last row of chunks read, interleaved.
    band: Vec<f32>,
    band_y: usize,
    band_rows: usize,
    y: usize,
}

impl TiffRows {
    fn open(path: &Path) -> Result<Box<dyn Rows>, String> {
        use tiff::ColorType;
        use tiff::decoder::{Decoder, Limits};
        use tiff::tags::Tag;
        let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let mut dec = Decoder::new(BufReader::new(file)).map_err(|e| e.to_string())?.with_limits(Limits::unlimited());
        let (w, h) = dec.dimensions().map_err(|e| e.to_string())?;
        let (channels, bits) = match dec.colortype().map_err(|e| e.to_string())? {
            ColorType::Gray(b) => (1, b),
            ColorType::GrayA(b) => (2, b),
            ColorType::RGB(b) => (3, b),
            ColorType::RGBA(b) => (4, b),
            other => return Err(format!("a TIFF of colours Massif does not read ({other:?})")),
        };
        if channels > 1 && dec.find_tag_unsigned::<u16>(Tag::PlanarConfiguration).ok().flatten() == Some(2) {
            return Err("a TIFF with its channels in separate planes".into());
        }
        let float = dec.find_tag_unsigned::<u16>(Tag::SampleFormat).ok().flatten() == Some(3);
        let (cw, ch) = dec.chunk_dimensions();
        let (cw, ch) = (cw as usize, ch as usize);
        let max = if float {
            1.0
        } else {
            (2f64.powi(bits as i32) - 1.0) as f32
        };
        let header = Header {
            width: w as usize,
            height: h as usize,
            bits,
            colour: channels >= 3,
            alpha: channels == 2 || channels == 4,
            float,
            format: "TIFF",
            lossy: false,
        };
        let across = (w as usize).div_ceil(cw.max(1));
        Ok(Box::new(Self {
            dec,
            header,
            channels,
            max,
            chunk: (cw.max(1), ch.max(1)),
            across,
            band: Vec::new(),
            band_y: 0,
            band_rows: 0,
            y: 0,
        }))
    }

    /// Read the row of chunks holding row `y`.
    fn read_band(&mut self, y: usize) -> Result<(), String> {
        use tiff::decoder::DecodingResult as D;
        let (cw, ch) = self.chunk;
        let r = y / ch;
        let w = self.header.width;
        let rows = ch.min(self.header.height - r * ch);
        let c = self.channels;
        self.band.clear();
        self.band.resize(w * rows * c, 0.0);
        for k in 0..self.across {
            let index = (r * self.across + k) as u32;
            let (dw, dh) = self.dec.chunk_data_dimensions(index);
            let (dw, dh) = (dw as usize, (dh as usize).min(rows));
            let data: Vec<f32> = match self.dec.read_chunk(index).map_err(|e| e.to_string())? {
                D::U8(v) => v.into_iter().map(|x| x as f32).collect(),
                D::U16(v) => v.into_iter().map(|x| x as f32).collect(),
                D::U32(v) => v.into_iter().map(|x| x as f32).collect(),
                D::U64(v) => v.into_iter().map(|x| x as f32).collect(),
                D::I8(v) => v.into_iter().map(|x| x as f32).collect(),
                D::I16(v) => v.into_iter().map(|x| x as f32).collect(),
                D::I32(v) => v.into_iter().map(|x| x as f32).collect(),
                D::F32(v) => v,
                D::F64(v) => v.into_iter().map(|x| x as f32).collect(),
                D::F16(v) => v.into_iter().map(f32::from).collect(),
                _ => return Err("a TIFF of a kind of samples Massif does not read".into()),
            };
            let x0 = k * cw;
            for j in 0..dh {
                let src = &data[j * dw * c..((j + 1) * dw * c).min(data.len())];
                let dst = (j * w + x0) * c;
                self.band[dst..dst + src.len()].copy_from_slice(src);
            }
        }
        self.band_y = r * ch;
        self.band_rows = rows;
        Ok(())
    }
}

impl Rows for TiffRows {
    fn header(&self) -> &Header {
        &self.header
    }

    fn next(&mut self, row: &mut Row) -> Result<bool, String> {
        let y = self.y;
        if y >= self.header.height {
            return Ok(false);
        }
        if self.band_rows == 0 || y >= self.band_y + self.band_rows {
            self.read_band(y)?;
        }
        let w = self.header.width * self.channels;
        let at = (y - self.band_y) * w;
        fill_row(row, &self.band[at..at + w], self.channels, self.max, |s| s);
        self.y += 1;
        Ok(true)
    }
}

/// An uncompressed BMP, read a row at a time from where it lies in the
/// file.
struct BmpRows {
    file: BufReader<std::fs::File>,
    header: Header,
    offset: u64,
    stride: usize,
    bpp: u16,
    bottom_up: bool,
    palette: Vec<[u8; 3]>,
    /// The masks of 16 and 32-bit pixels: red, green, blue, alpha.
    masks: [u32; 4],
    buf: Vec<u8>,
    y: usize,
}

impl BmpRows {
    /// `None`: a compressed BMP, to read whole instead.
    fn open(path: &Path) -> Result<Option<Self>, String> {
        let mut file = BufReader::new(std::fs::File::open(path).map_err(|e| e.to_string())?);
        let mut head = [0u8; 54];
        file.read_exact(&mut head).map_err(|_| "a BMP cut short")?;
        let u32_at = |i: usize| u32::from_le_bytes([head[i], head[i + 1], head[i + 2], head[i + 3]]);
        let offset = u32_at(10) as u64;
        let dib = u32_at(14) as usize;
        if dib < 40 {
            return Ok(None);
        }
        let width = u32_at(18) as i32;
        let height = u32_at(22) as i32;
        let bpp = u16::from_le_bytes([head[28], head[29]]);
        let compression = u32_at(30);
        let colours = u32_at(46) as usize;
        if width <= 0 || height == 0 || !matches!(bpp, 1 | 4 | 8 | 16 | 24 | 32) {
            return Err(format!("a BMP of {bpp} bits a pixel"));
        }
        let mut masks = match bpp {
            16 => [0x7c00, 0x03e0, 0x001f, 0],
            32 => [0x00ff_0000, 0x0000_ff00, 0x0000_00ff, 0],
            _ => [0; 4],
        };
        match compression {
            0 => {}
            3 | 6 if bpp == 16 || bpp == 32 => {
                // The masks follow the 40 bytes of the header, inside it or
                // after it.
                let mut m = [0u8; 16];
                file.seek(SeekFrom::Start(14 + 40)).map_err(|e| e.to_string())?;
                let n = if compression == 6 || dib >= 56 { 16 } else { 12 };
                file.read_exact(&mut m[..n]).map_err(|_| "a BMP cut short")?;
                for (k, mask) in masks.iter_mut().enumerate().take(n / 4) {
                    *mask = u32::from_le_bytes([m[k * 4], m[k * 4 + 1], m[k * 4 + 2], m[k * 4 + 3]]);
                }
            }
            _ => return Ok(None),
        }
        // 32-bit pixels of version 4 or 5 headers say whether they have alpha.
        if bpp == 32 && compression == 0 && dib >= 56 {
            masks[3] = 0;
        }
        let mut palette = Vec::new();
        if bpp <= 8 {
            let n = if colours == 0 { 1 << bpp } else { colours.min(256) };
            file.seek(SeekFrom::Start(14 + dib as u64)).map_err(|e| e.to_string())?;
            let mut p = vec![0u8; n * 4];
            file.read_exact(&mut p).map_err(|_| "a BMP cut short")?;
            palette = p.as_chunks::<4>().0.iter().map(|c| [c[2], c[1], c[0]]).collect();
        }
        let colour = palette.iter().any(|c| c[0] != c[1] || c[1] != c[2]) || bpp >= 16 && palette.is_empty() && bpp != 8;
        let width = width as usize;
        let stride = (width * bpp as usize).div_ceil(32) * 4;
        Ok(Some(Self {
            file,
            header: Header {
                width,
                height: height.unsigned_abs() as usize,
                bits: 8,
                colour,
                alpha: masks[3] != 0,
                float: false,
                format: "BMP",
                lossy: false,
            },
            offset,
            stride,
            bpp,
            bottom_up: height > 0,
            palette,
            masks,
            buf: vec![0; stride],
            y: 0,
        }))
    }
}

/// The 8-bit value of the bits of `v` under `mask`.
fn masked(v: u32, mask: u32) -> u8 {
    if mask == 0 {
        return 255;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).count_ones();
    let x = (v & mask) >> shift;
    ((x as u64 * 255) / ((1u64 << bits) - 1)) as u8
}

impl Rows for BmpRows {
    fn header(&self) -> &Header {
        &self.header
    }

    fn next(&mut self, row: &mut Row) -> Result<bool, String> {
        let (w, h) = (self.header.width, self.header.height);
        if self.y >= h {
            return Ok(false);
        }
        let line = if self.bottom_up { h - 1 - self.y } else { self.y };
        self.file.seek(SeekFrom::Start(self.offset + (line * self.stride) as u64)).map_err(|e| e.to_string())?;
        self.file.read_exact(&mut self.buf).map_err(|_| "a BMP cut short")?;
        row.clear();
        let colour = self.header.colour;
        let put = |row: &mut Row, rgb: [u8; 3], a: Option<u8>| {
            row.grey.push(luma(rgb[0] as f32, rgb[1] as f32, rgb[2] as f32));
            if colour {
                row.rgb.push(rgb);
            }
            if let Some(a) = a {
                row.alpha.push(a);
            }
        };
        let b = &self.buf;
        let alpha = self.header.alpha;
        for x in 0..w {
            match self.bpp {
                1 | 4 | 8 => {
                    let bits = self.bpp as usize;
                    let byte = b[x * bits / 8];
                    let shift = 8 - bits - (x * bits % 8);
                    let index = (byte >> shift) as usize & ((1 << bits) - 1);
                    put(row, self.palette.get(index).copied().unwrap_or([0; 3]), None);
                }
                16 => {
                    let v = u16::from_le_bytes([b[x * 2], b[x * 2 + 1]]) as u32;
                    let m = self.masks;
                    put(row, [masked(v, m[0]), masked(v, m[1]), masked(v, m[2])], alpha.then(|| masked(v, m[3])));
                }
                24 => put(row, [b[x * 3 + 2], b[x * 3 + 1], b[x * 3]], None),
                _ => {
                    let v = u32::from_le_bytes([b[x * 4], b[x * 4 + 1], b[x * 4 + 2], b[x * 4 + 3]]);
                    let m = self.masks;
                    put(row, [masked(v, m[0]), masked(v, m[1]), masked(v, m[2])], alpha.then(|| masked(v, m[3])));
                }
            }
        }
        self.y += 1;
        Ok(true)
    }
}

/// An image decoded whole (JPEG, compressed BMP), handed out by rows.
struct WholeRows {
    image: image::DynamicImage,
    header: Header,
    y: usize,
}

impl WholeRows {
    fn open(path: &Path, format: &'static str) -> Result<Box<dyn Rows>, String> {
        let image = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?;
        let mut image = image;
        image.no_limits();
        let image = image.decode().map_err(|e| e.to_string())?;
        let colour = image.color().has_color();
        let alpha = image.color().has_alpha();
        let image = match (colour, alpha) {
            (false, false) => image::DynamicImage::ImageLuma8(image.into_luma8()),
            (false, true) => image::DynamicImage::ImageLumaA8(image.into_luma_alpha8()),
            (true, false) => image::DynamicImage::ImageRgb8(image.into_rgb8()),
            (true, true) => image::DynamicImage::ImageRgba8(image.into_rgba8()),
        };
        let header = Header {
            width: image.width() as usize,
            height: image.height() as usize,
            bits: 8,
            colour,
            alpha,
            float: false,
            format,
            lossy: format == "JPEG",
        };
        Ok(Box::new(Self { image, header, y: 0 }))
    }
}

impl Rows for WholeRows {
    fn header(&self) -> &Header {
        &self.header
    }

    fn next(&mut self, row: &mut Row) -> Result<bool, String> {
        if self.y >= self.header.height {
            return Ok(false);
        }
        let channels = self.image.color().channel_count() as usize;
        let line = self.header.width * channels;
        let bytes = &self.image.as_bytes()[self.y * line..(self.y + 1) * line];
        fill_row(row, bytes, channels, 255.0, |s| s as f32);
        self.y += 1;
        Ok(true)
    }
}

/// What [`survey`] found in an image.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageInfo {
    pub header: Header,
    /// Some pixel is not grey.
    pub colour: bool,
    /// Some pixel is not opaque.
    pub transparent: bool,
    /// The lowest and highest grey of its opaque pixels, in its units.
    pub low: f32,
    pub high: f32,
    /// The grey of white, in its units.
    pub white: f32,
    /// A picture of it, [`PREVIEW_SIDE`] pixels at most across: each
    /// pixel the average of the ones it stands for.
    pub preview: Preview,
    /// The same, [`GUIDE_SIDE`] pixels at most across.
    pub guide: Preview,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct Preview {
    pub width: usize,
    pub height: usize,
    /// The grey of each pixel, in the image's units (NaN: transparent).
    pub grey: Vec<f32>,
    /// Its colour.
    pub rgb: Vec<[u8; 3]>,
}

/// Read an image once through: what it holds, the range of its values and
/// a small picture of it, telling how far it is in `steps` (rows) and
/// stopping when asked.
pub fn survey(path: &Path, steps: &Steps) -> Result<ImageInfo, String> {
    let (rows, stop) = (&steps.done, &steps.cancel);
    let mut reader = open(path)?;
    let header = reader.header().clone();
    let (w, h) = (header.width, header.height);
    steps.total.store(h, Ordering::Relaxed);
    if w == 0 || h == 0 {
        return Err("an empty image".into());
    }
    let step = w.max(h).div_ceil(GUIDE_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(step), h.div_ceil(step));
    let mut sum = vec![[0f64; 4]; pw];
    let mut count = vec![0u32; pw];
    let mut preview = Preview {
        width: pw,
        height: ph,
        grey: Vec::with_capacity(pw * ph),
        rgb: Vec::with_capacity(pw * ph),
    };
    let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
    let (mut colour, mut transparent) = (false, false);
    let mut row = Row::default();
    let mut y = 0;
    let flush = |sum: &mut Vec<[f64; 4]>, count: &mut Vec<u32>, preview: &mut Preview| {
        for (s, c) in sum.iter_mut().zip(count.iter_mut()) {
            if *c == 0 {
                preview.grey.push(f32::NAN);
                preview.rgb.push([0; 3]);
            } else {
                let n = *c as f64;
                preview.grey.push((s[0] / n) as f32);
                preview.rgb.push([(s[1] / n).round() as u8, (s[2] / n).round() as u8, (s[3] / n).round() as u8]);
            }
            *s = [0.0; 4];
            *c = 0;
        }
    };
    while reader.next(&mut row)? {
        if stop.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        for x in 0..w {
            if !row.opaque(x) {
                transparent = true;
                continue;
            }
            let g = row.grey[x];
            if g.is_nan() {
                continue;
            }
            low = low.min(g);
            high = high.max(g);
            let c = row.rgb.get(x).copied().unwrap_or_else(|| {
                let v = (g / white_of(&header) * 255.0).round().clamp(0.0, 255.0) as u8;
                [v; 3]
            });
            if !colour && !row.rgb.is_empty() && (c[0] != c[1] || c[1] != c[2]) {
                colour = true;
            }
            let s = &mut sum[x / step];
            s[0] += g as f64;
            s[1] += c[0] as f64;
            s[2] += c[1] as f64;
            s[3] += c[2] as f64;
            count[x / step] += 1;
        }
        y += 1;
        rows.store(y, Ordering::Relaxed);
        if y % step == 0 || y == h {
            flush(&mut sum, &mut count, &mut preview);
        }
    }
    if y < h {
        return Err(format!("the image stops at row {y} of {h}"));
    }
    if low > high {
        (low, high) = (0.0, 0.0);
    }
    let white = white_of(&header);
    let guide = preview;
    let preview = guide.shrunk(guide.width.max(guide.height).div_ceil(PREVIEW_SIDE).max(1));
    Ok(ImageInfo {
        header,
        colour,
        transparent,
        low,
        high,
        white,
        preview,
        guide,
    })
}

impl Preview {
    /// Each `k` × `k` pixels made one, their average.
    fn shrunk(&self, k: usize) -> Preview {
        if k <= 1 {
            return self.clone();
        }
        let (w, h) = (self.width.div_ceil(k), self.height.div_ceil(k));
        let mut out = Preview {
            width: w,
            height: h,
            grey: Vec::with_capacity(w * h),
            rgb: Vec::with_capacity(w * h),
        };
        for py in 0..h {
            for px in 0..w {
                let (mut s, mut n) = ([0f64; 4], 0u32);
                for y in py * k..((py + 1) * k).min(self.height) {
                    for x in px * k..((px + 1) * k).min(self.width) {
                        let i = y * self.width + x;
                        if !self.grey[i].is_nan() {
                            s[0] += self.grey[i] as f64;
                            for c in 0..3 {
                                s[c + 1] += self.rgb[i][c] as f64;
                            }
                            n += 1;
                        }
                    }
                }
                if n == 0 {
                    out.grey.push(f32::NAN);
                    out.rgb.push([0; 3]);
                } else {
                    let n = n as f64;
                    out.grey.push((s[0] / n) as f32);
                    out.rgb.push([(s[1] / n).round() as u8, (s[2] / n).round() as u8, (s[3] / n).round() as u8]);
                }
            }
        }
        out
    }
}

fn white_of(header: &Header) -> f32 {
    if header.float { 1.0 } else { (2f64.powi(header.bits as i32) - 1.0) as f32 }
}

/// What an import paints.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// The annotations: each pixel's nearest of the 15 colours (`None`),
    /// or that one colour (1 to 15) where the pixel is on.
    Annotations(Option<u16>),
    /// A terrain, by id.
    Terrain(u16),
    /// A biome, by id.
    Biome(u16),
    /// A ground cover, by number.
    Cover(u16),
    /// An object layer under trees, by number.
    Trees(u16),
    /// An object layer under the trees, by number.
    Understory(u16),
    /// A size of tree, 1 (bushes) to 4 (large).
    TreeSize(u16),
    /// A layer of underground pockets, by number.
    Pockets(u16),
    /// Water, its surface at this height.
    Water(i16),
    /// The height of the ground: black gives `black`, white `white`.
    Height { black: f32, white: f32 },
}

impl Target {
    /// The plane's slot, whose locked layers keep their columns.
    fn slot(self) -> Option<Slot> {
        match self {
            Self::Cover(_) => Some(Slot::Cover),
            Self::Trees(_) | Self::Understory(_) => Some(Slot::Objects),
            Self::Pockets(_) => Some(Slot::Pockets),
            _ => None,
        }
    }

    /// Whether the value is the nearest colour of each pixel.
    pub fn by_colour(self) -> bool {
        matches!(self, Self::Annotations(None))
    }
}

/// How what is imported goes with what the map has.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Mode {
    /// The image decides, where it lies: off rubs out what was there.
    #[default]
    Replace,
    /// What the image has is added to what is there.
    Add,
    /// The most of the two.
    Max,
    /// The least of the two.
    Min,
}

impl Mode {
    pub const ALL: [Mode; 4] = [Mode::Replace, Mode::Add, Mode::Max, Mode::Min];
}

/// How a pixel's value becomes a strength from 0 to 1.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Mapping {
    /// From black to white (true), or from the image's lowest value to
    /// its highest.
    pub full_range: bool,
    pub invert: bool,
    /// Black pixels leave the map as it is.
    pub black_is_none: bool,
    /// `Some(t)`: on from `t` up; `None`: the strength, scattered.
    pub threshold: Option<f32>,
    /// Levels: the strengths taken as black and as white (0 to 1), those
    /// between stretched over the whole range.
    pub levels: [f32; 2],
    /// The midtones: a strength between black and white raised to
    /// `1 / gamma` (1: as it is; more: lighter).
    pub gamma: f32,
    /// Blurred over this many blocks around each column (0: sharp).
    pub blur: f32,
}

impl Mapping {
    /// A strength from 0 to 1 through the levels and the midtones.
    #[inline]
    pub fn shape(&self, v: f32) -> f32 {
        let [a, b] = self.levels;
        let v = if b > a { ((v - a) / (b - a)).clamp(0.0, 1.0) } else if v >= a { 1.0 } else { 0.0 };
        if self.gamma > 0.0 && self.gamma != 1.0 && v > 0.0 && v < 1.0 { v.powf(1.0 / self.gamma) } else { v }
    }

    /// Whether levels, midtones or blur change anything.
    pub fn adjusted(&self) -> bool {
        self.levels != [0.0, 1.0] || self.gamma != 1.0 || self.blur > 0.0
    }
}

impl Default for Mapping {
    fn default() -> Self {
        Self {
            full_range: true,
            invert: false,
            black_is_none: true,
            threshold: Some(0.5),
            levels: [0.0, 1.0],
            gamma: 1.0,
            blur: 0.0,
        }
    }
}

/// Where the image lands: its top left corner on block `(x, z)`, a pixel
/// `scale` blocks across.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub x: i32,
    pub z: i32,
    pub scale: f32,
}

impl Placement {
    /// The blocks the image covers.
    pub fn rect(&self, width: usize, height: usize) -> BlockRect {
        let w = (width as f64 * self.scale as f64).round().max(1.0) as i32;
        let h = (height as f64 * self.scale as f64).round().max(1.0) as i32;
        BlockRect::new(self.x, self.z, self.x.saturating_add(w), self.z.saturating_add(h))
    }
}

/// Everything an import needs.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskSettings {
    pub target: Target,
    pub mapping: Mapping,
    pub mode: Mode,
    pub placement: Placement,
    /// For the height: over this many blocks in from the image's edges,
    /// the ground goes from its own height to the image's, so that a
    /// heightmap of a part of the map meets the land around it without a
    /// step (0: none).
    pub feather: u32,
}

/// The nearest of the annotation colours to `c` (1 to 15).
pub fn nearest_annotation(c: [u8; 3]) -> u16 {
    static TABLE: std::sync::OnceLock<Vec<u8>> = std::sync::OnceLock::new();
    let table = TABLE.get_or_init(|| {
        // 6 bits a channel: the middle of each cell decides.
        (0..1 << 18)
            .map(|k: u32| {
                let c = [((k >> 12) & 63) * 4 + 2, ((k >> 6) & 63) * 4 + 2, (k & 63) * 4 + 2];
                nearest_exact([c[0] as u8, c[1] as u8, c[2] as u8]) as u8
            })
            .collect()
    });
    table[((c[0] as usize >> 2) << 12) | ((c[1] as usize >> 2) << 6) | (c[2] as usize >> 2)] as u16
}

fn nearest_exact(c: [u8; 3]) -> u16 {
    let d = |a: [u8; 3]| (0..3).map(|k| (a[k] as i32 - c[k] as i32).pow(2)).sum::<i32>();
    (0..15).min_by_key(|&k| d(ANNOTATION_COLORS[k])).unwrap() as u16 + 1
}

/// What a column of the map gets from the image: a strength (NaN: the
/// image leaves it alone) and, for colours, the nearest annotation.
#[derive(Debug, Clone, Copy)]
struct Sample {
    v: f32,
    c: u16,
}

const LEAVE: Sample = Sample { v: f32::NAN, c: 0 };

/// Turns pixels into strengths.
struct Reading {
    low: f32,
    span: f32,
    invert: bool,
    black_is_none: bool,
    mapping: Mapping,
    /// The grey at or under which a pixel is black.
    black: f32,
    /// The grey of white.
    white: f32,
}

impl Reading {
    fn new(info: &ImageInfo, m: &Mapping) -> Self {
        let (low, high) = if m.full_range {
            (0.0, info.white)
        } else {
            (info.low, info.high)
        };
        let black = if info.header.lossy { 8.0 / 255.0 * info.white } else { 0.0 };
        Self {
            low,
            span: (high - low).max(f32::MIN_POSITIVE),
            invert: m.invert,
            black_is_none: m.black_is_none,
            mapping: *m,
            black,
            white: info.white,
        }
    }

    #[inline]
    fn strength(&self, grey: f32) -> f32 {
        if self.black_is_none && grey <= self.black {
            return f32::NAN;
        }
        let v = self.mapping.shape(((grey - self.low) / self.span).clamp(0.0, 1.0));
        if self.invert { 1.0 - v } else { v }
    }
}

/// The rows of the image a band of the map reads, kept as they come.
struct Window {
    first: usize,
    rows: std::collections::VecDeque<Row>,
}

impl Window {
    fn get(&self, y: usize) -> &Row {
        &self.rows[y - self.first]
    }
}

/// The image's rows under map row `z` that it reads: from the first to
/// the one after the last.
fn rows_under(z: i32, p: &Placement, h: usize) -> (usize, usize) {
    let s = p.scale as f64;
    let (a, b) = if s >= 1.0 {
        let v = (z as f64 + 0.5 - p.z as f64) / s - 0.5;
        (v.floor(), v.floor() + 2.0)
    } else {
        (((z - p.z) as f64 / s).floor(), ((z + 1 - p.z) as f64 / s).ceil())
    };
    (a.clamp(0.0, h as f64) as usize, b.clamp(0.0, h as f64) as usize)
}

/// What the image gives column `x` of map row `z`.
#[allow(clippy::too_many_arguments)]
fn sample(win: &Window, x: i32, z: i32, p: &Placement, w: usize, h: usize, r: &Reading, colours: bool) -> Sample {
    let s = p.scale as f64;
    let at = |row: &Row, i: usize| -> Sample {
        if !row.opaque(i) {
            return LEAVE;
        }
        let v = r.strength(row.grey[i]);
        let c = if colours && !v.is_nan() { row.rgb.get(i).map_or_else(|| nearest_annotation([(row.grey[i] / r.white * 255.0).clamp(0.0, 255.0) as u8; 3]), |&c| nearest_annotation(c)) } else { 0 };
        Sample { v, c }
    };
    if s >= 1.0 {
        let u = (x as f64 + 0.5 - p.x as f64) / s - 0.5;
        let v = (z as f64 + 0.5 - p.z as f64) / s - 0.5;
        let (i, j) = (u.round().clamp(0.0, (w - 1) as f64) as usize, v.round().clamp(0.0, (h - 1) as f64) as usize);
        let near = at(win.get(j), i);
        if near.v.is_nan() || colours {
            return near;
        }
        // Grey between the four pixels around, where they all have one.
        let (i0, j0) = (u.floor().clamp(0.0, (w - 1) as f64) as usize, v.floor().clamp(0.0, (h - 1) as f64) as usize);
        let (i1, j1) = ((i0 + 1).min(w - 1), (j0 + 1).min(h - 1));
        let (fu, fv) = ((u - i0 as f64).clamp(0.0, 1.0) as f32, (v - j0 as f64).clamp(0.0, 1.0) as f32);
        let (a, b, c, d) = (at(win.get(j0), i0).v, at(win.get(j0), i1).v, at(win.get(j1), i0).v, at(win.get(j1), i1).v);
        if [a, b, c, d].iter().any(|v| v.is_nan()) {
            return near;
        }
        let top = a + (b - a) * fu;
        let bottom = c + (d - c) * fu;
        Sample { v: top + (bottom - top) * fv, c: 0 }
    } else {
        // The average of the pixels the column stands on.
        let (i0, i1) = (((x - p.x) as f64 / s).floor().clamp(0.0, w as f64) as usize, ((x + 1 - p.x) as f64 / s).ceil().clamp(0.0, w as f64) as usize);
        let (j0, j1) = rows_under(z, p, h);
        if colours {
            let (i, j) = (((i0 + i1) / 2).min(w - 1), ((j0 + j1) / 2).min(h - 1));
            return at(win.get(j), i);
        }
        let (mut sum, mut n, mut all) = (0f32, 0u32, 0u32);
        for j in j0..j1 {
            let row = win.get(j);
            for i in i0..i1 {
                all += 1;
                if row.opaque(i) {
                    sum += row.grey[i];
                    n += 1;
                }
            }
        }
        if n == 0 || n * 2 < all {
            return LEAVE;
        }
        Sample { v: r.strength(sum / n as f32), c: 0 }
    }
}

/// Whether a column of strength `p` (0 to 1) is on: scattered, the same
/// columns for the same image.
#[inline]
fn hit(p: f32, x: i32, z: i32) -> bool {
    p >= 1.0 || (p > 0.0 && (crate::compose::hash2(x, z, 0x6d61_736b) as f32) < p * u32::MAX as f32)
}

/// Paint the image at `path` onto the edited level as `settings` say,
/// telling how far it is in `steps` (rows of the image) and stopping when
/// asked. Only the tiles the level has are painted, within its selection
/// when it has one; the locked layers keep their columns.
pub fn apply(edit: &mut Edit, path: &Path, info: &ImageInfo, settings: &MaskSettings, steps: &Steps) -> Result<Changes, String> {
    steps.total.store(info.header.height, Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    apply_from(edit, path, info, settings, steps, 0)
}

/// Several images, each with its settings, one after the other into the
/// same edit (one step to undo): the heights first, so that what is
/// painted after reads the new ground.
pub fn apply_all(edit: &mut Edit, images: &[(std::path::PathBuf, std::sync::Arc<ImageInfo>, MaskSettings)], steps: &Steps) -> Result<Changes, String> {
    let mut order: Vec<&(std::path::PathBuf, std::sync::Arc<ImageInfo>, MaskSettings)> = images.iter().collect();
    order.sort_by_key(|(_, _, s)| !matches!(s.target, Target::Height { .. }));
    steps.total.store(images.iter().map(|(_, i, _)| i.header.height).sum(), Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let mut all = Changes::default();
    let mut base = 0;
    for (path, info, settings) in order {
        let changes = apply_from(edit, path, info, settings, steps, base)?;
        base += info.header.height;
        all.columns += changes.columns;
        all.tiles.extend(changes.tiles);
        if changes.cancelled {
            all.cancelled = true;
            break;
        }
    }
    all.tiles.sort_unstable_by_key(|p| (p.z, p.x));
    all.tiles.dedup();
    Ok(all)
}

/// [`apply`], the rows read counted in `steps` from `base`.
fn apply_from(edit: &mut Edit, path: &Path, info: &ImageInfo, settings: &MaskSettings, steps: &Steps, base: usize) -> Result<Changes, String> {
    let (w, h) = (info.header.width, info.header.height);
    let p = settings.placement;
    if !(p.scale > 0.0 && p.scale.is_finite()) {
        return Err("a scale of 0".into());
    }
    let footprint = p.rect(w, h);
    let selection = edit.dim().selection.clone();
    let mut bands: BTreeMap<i32, Vec<TilePos>> = BTreeMap::new();
    for pos in footprint.tiles() {
        if edit.level().contains(pos) && selection.as_ref().is_none_or(|s| s.tile(pos).is_some()) {
            bands.entry(pos.z).or_default().push(pos);
        }
    }
    let mut changes = Changes::default();
    if bands.is_empty() {
        steps.done.store(base + h, Ordering::Relaxed);
        return Ok(changes);
    }
    let reading = Reading::new(info, &settings.mapping);
    let colours = settings.target.by_colour();
    let locks = settings.target.slot().filter(|_| !edit.locks().is_empty()).map(|s| (s, edit.locks().clone()));
    let (lo, hi) = (edit.dim().min_y as f32, edit.dim().max_y as f32);
    let blur = if colours { 0 } else { settings.mapping.blur.max(0.0).round() as i32 };

    // A thread decodes the rows ahead while the map is painted.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<Row, String>>(512);
    let path = path.to_path_buf();
    let reader = std::thread::spawn(move || {
        let mut rows = match open(&path) {
            Ok(r) => r,
            Err(e) => {
                let _ = tx.send(Err(e));
                return;
            }
        };
        loop {
            let mut row = Row::default();
            match rows.next(&mut row) {
                Ok(true) => {
                    if tx.send(Ok(row)).is_err() {
                        return;
                    }
                }
                Ok(false) => return,
                Err(e) => {
                    let _ = tx.send(Err(e));
                    return;
                }
            }
        }
    });
    let mut win = Window { first: 0, rows: Default::default() };
    let mut read = 0usize;
    let columns = AtomicUsize::new(0);
    let touched = Mutex::new(Vec::new());
    let mut result = Ok(());
    for (&tz, positions) in &bands {
        if steps.cancel.load(Ordering::Relaxed) {
            changes.cancelled = true;
            break;
        }
        let band = BlockRect::new(footprint.x0, (tz * TILE_SIZE).max(footprint.z0), footprint.x1, ((tz + 1) * TILE_SIZE).min(footprint.z1));
        // A blur reads the rows around the band too.
        let (ez0, ez1) = ((band.z0 - blur).max(footprint.z0), (band.z1 + blur).min(footprint.z1));
        let (need0, _) = rows_under(ez0, &p, h);
        let (_, need1) = rows_under(ez1 - 1, &p, h);
        let need1 = need1.max(need0 + 1).min(h);
        // Rows the band does not read go; the ones it reads come.
        while win.first < need0 && !win.rows.is_empty() {
            win.rows.pop_front();
            win.first += 1;
        }
        if win.rows.is_empty() {
            win.first = read.max(need0);
        }
        while read < need1 {
            let row = match rx.recv() {
                Ok(Ok(row)) => row,
                Ok(Err(e)) => {
                    result = Err(e);
                    break;
                }
                Err(_) => {
                    result = Err(format!("the image stops at row {read} of {h}"));
                    break;
                }
            };
            if read >= win.first {
                win.rows.push_back(row);
            }
            read += 1;
            steps.done.store(base + read, Ordering::Relaxed);
        }
        if result.is_err() {
            break;
        }
        let need0 = need0.max(win.first);
        if need0 >= need1 || win.rows.is_empty() {
            continue;
        }
        // What the image gives each column of the band, row by row.
        let width = band.width() as usize;
        let mut samples: Vec<Vec<Sample>> = (ez0..ez1)
            .into_par_iter()
            .map(|z| (band.x0..band.x1).map(|x| sample(&win, x, z, &p, w, h, &reading, colours)).collect())
            .collect();
        if blur > 0 {
            let mut v: Vec<f32> = samples.iter().flatten().map(|s| s.v).collect();
            blur_grid(&mut v, width, blur as usize);
            for (s, v) in samples.iter_mut().flatten().zip(v) {
                s.v = v;
            }
        }
        let target = settings.target;
        let mode = settings.mode;
        // In from the image's edges, the height fades in.
        let feather = if matches!(target, Target::Height { .. }) { settings.feather as f32 } else { 0.0 };
        let fade = |x: i32, z: i32| {
            if feather <= 0.0 {
                return 1.0;
            }
            let d = (x - footprint.x0).min(footprint.x1 - 1 - x).min(z - footprint.z0).min(footprint.z1 - 1 - z) as f32 + 0.5;
            let t = (d / feather).clamp(0.0, 1.0);
            t * t * (3.0 - 2.0 * t)
        };
        edit.par_tiles(positions.iter().copied(), band, false, |pos, clip, tile| {
            let cells = selection.as_ref().and_then(|s| s.tile(pos));
            let mut n = 0;
            for z in clip.z0..clip.z1 {
                let line = &samples[(z - ez0) as usize];
                for x in clip.x0..clip.x1 {
                    let s = line[(x - band.x0) as usize];
                    let _ = width;
                    if s.v.is_nan() {
                        continue;
                    }
                    let i = local_index(x, z);
                    if cells.is_some_and(|c| !crate::selection::lets(c.get(i), x, z)) {
                        continue;
                    }
                    if put(tile, i, x, z, s, target, mode, &settings.mapping, locks.as_ref(), (lo, hi), fade(x, z)) {
                        n += 1;
                    }
                }
            }
            if n > 0 {
                tile.compact();
                columns.fetch_add(n, Ordering::Relaxed);
                touched.lock().expect("not poisoned").push(pos);
            }
        });
        let next = bands.range(tz + 1..).next().map(|(&z, _)| z);
        if next.is_none() {
            break;
        }
    }
    drop(rx);
    let _ = reader.join();
    result?;
    if !changes.cancelled {
        steps.done.store(base + h, Ordering::Relaxed);
    }
    changes.columns = columns.into_inner();
    changes.tiles = touched.into_inner().expect("not poisoned");
    changes.tiles.sort_unstable_by_key(|p| (p.z, p.x));
    Ok(changes)
}

/// Paint one column; true if it changed.
#[allow(clippy::too_many_arguments)]
#[inline]
fn put(tile: &mut Tile, i: usize, x: i32, z: i32, s: Sample, target: Target, mode: Mode, m: &Mapping, locks: Option<&(Slot, crate::registry::Locks)>, heights: (f32, f32), fade: f32) -> bool {
    if let Target::Height { black, white } = target {
        let old = tile.top.get(i);
        let h = height_of(old, s.v, black, white, mode, heights, fade);
        if tile.bottom.is_some() || h == old {
            return false;
        }
        tile.top.set(i, h);
        return true;
    }
    if tile.bottom.is_some() && !tile.present(i) {
        return false;
    }
    let strength = strength_of(s.v, target, m);
    let plane = match target {
        Target::Annotations(_) => &mut tile.annotations,
        Target::Terrain(_) => &mut tile.terrain,
        Target::Biome(_) => &mut tile.biome,
        Target::Cover(_) => &mut tile.cover,
        Target::Trees(_) => &mut tile.objects,
        Target::Understory(_) => &mut tile.understory,
        Target::TreeSize(_) => &mut tile.sizes,
        Target::Pockets(_) => &mut tile.pockets,
        Target::Water(level) => {
            let old = tile.fluid.get(i);
            let new = water_of(old, strength, level, mode, x, z);
            if new == old {
                return false;
            }
            tile.fluid.set(i, new);
            return true;
        }
        Target::Height { .. } => unreachable!(),
    };
    let old = plane.get(i);
    if let Some((slot, locks)) = locks
        && locks.holds(*slot, old)
    {
        return false;
    }
    let new = plane_of(old, strength, s.c, target, mode, x, z);
    if new == old {
        return false;
    }
    plane.set(i, new);
    true
}

/// The height a column of height `old` takes from a strength `v`.
#[inline]
fn height_of(old: f32, v: f32, black: f32, white: f32, mode: Mode, (lo, hi): (f32, f32), fade: f32) -> f32 {
    let new = black + v * (white - black);
    let h = match mode {
        Mode::Replace => new,
        Mode::Add => old + new,
        Mode::Max => old.max(new),
        Mode::Min => old.min(new),
    };
    let h = old + (h - old) * fade;
    crate::codec::snap_height(h.clamp(lo, hi))
}

/// The strength a column is painted with: on or off from the threshold,
/// whole for the colours, else the image's.
#[inline]
fn strength_of(v: f32, target: Target, m: &Mapping) -> f32 {
    match m.threshold {
        Some(t) if !target.by_colour() => {
            if v >= t {
                1.0
            } else {
                0.0
            }
        }
        _ if target.by_colour() => 1.0,
        _ => v,
    }
}

/// The water a column with `old` takes.
#[inline]
fn water_of(old: i16, strength: f32, level: i16, mode: Mode, x: i32, z: i32) -> i16 {
    let had = old != NO_FLUID;
    let on = hit(combine(mode, had, strength), x, z);
    if on {
        level
    } else if had {
        NO_FLUID
    } else {
        old
    }
}

/// The value a column of a plane with `old` takes (`old` when it stays).
#[inline]
fn plane_of(old: u16, strength: f32, colour: u16, target: Target, mode: Mode, x: i32, z: i32) -> u16 {
    let (value, none) = match target {
        Target::Annotations(None) => (colour, Some(0)),
        Target::Annotations(Some(c)) => (c.clamp(1, 15), Some(0)),
        Target::Terrain(t) => (t, None),
        Target::Biome(b) => (b, Some(0)),
        Target::TreeSize(n) => (n.clamp(1, crate::tile::SIZE_MAX), Some(0)),
        Target::Cover(n) | Target::Trees(n) | Target::Understory(n) | Target::Pockets(n) => (n, Some(0)),
        _ => return old,
    };
    let had = if target.by_colour() { old != 0 } else { old == value };
    let on = hit(combine(mode, had, strength), x, z);
    if on {
        value
    } else if had {
        none.unwrap_or(old)
    } else {
        old
    }
}

/// Blur the strengths of a grid `w` cells wide over `r` cells: twice a box
/// of half that each way, near a Gaussian. Cells left alone (NaN) stay so
/// and count for nothing.
pub fn blur_grid(v: &mut [f32], w: usize, r: usize) {
    if r == 0 || w == 0 || v.is_empty() {
        return;
    }
    let h = v.len() / w;
    let half = r.div_ceil(2);
    let mut tmp = vec![0f32; v.len()];
    for _ in 0..2 {
        // Across, row by row.
        tmp.par_chunks_mut(w).zip(v.par_chunks(w)).for_each(|(out, src)| {
            let (mut sum, mut n) = (0f64, 0u32);
            let add = |k: usize, sum: &mut f64, n: &mut u32, sign: i32| {
                let x = src[k];
                if !x.is_nan() {
                    *sum += sign as f64 * x as f64;
                    *n = n.wrapping_add_signed(sign);
                }
            };
            for k in 0..half.min(w) {
                add(k, &mut sum, &mut n, 1);
            }
            for k in 0..w {
                if k + half < w {
                    add(k + half, &mut sum, &mut n, 1);
                }
                if k > half {
                    add(k - half - 1, &mut sum, &mut n, -1);
                }
                out[k] = if src[k].is_nan() || n == 0 { src[k] } else { (sum / n as f64) as f32 };
            }
        });
        // Down, all the columns at once.
        let (mut sum, mut n) = (vec![0f64; w], vec![0u32; w]);
        let add = |row: &[f32], sum: &mut [f64], n: &mut [u32], sign: i32| {
            for ((s, c), &x) in sum.iter_mut().zip(n.iter_mut()).zip(row) {
                if !x.is_nan() {
                    *s += sign as f64 * x as f64;
                    *c = c.wrapping_add_signed(sign);
                }
            }
        };
        for j in 0..half.min(h) {
            add(&tmp[j * w..(j + 1) * w], &mut sum, &mut n, 1);
        }
        for j in 0..h {
            if j + half < h {
                add(&tmp[(j + half) * w..(j + half + 1) * w], &mut sum, &mut n, 1);
            }
            if j > half {
                add(&tmp[(j - half - 1) * w..(j - half) * w], &mut sum, &mut n, -1);
            }
            for k in 0..w {
                let src = tmp[j * w + k];
                v[j * w + k] = if src.is_nan() || n[k] == 0 { src } else { (sum[k] / n[k] as f64) as f32 };
            }
        }
    }
}

/// A column of the map as the preview of an import reads it.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Column {
    /// The level has it (an empty column of a slab has not).
    pub present: bool,
    pub top: f32,
    pub fluid: i16,
    pub terrain: u16,
    pub biome: u16,
    pub annotations: u16,
    pub cover: u16,
    pub objects: u16,
    pub understory: u16,
    pub sizes: u16,
    pub pockets: u16,
}

impl Column {
    /// What it has of the target's kind: an id, a height, a water level.
    pub fn value(&self, target: Target) -> f32 {
        match target {
            Target::Annotations(_) => self.annotations as f32,
            Target::Terrain(_) => self.terrain as f32,
            Target::Biome(_) => self.biome as f32,
            Target::Cover(_) => self.cover as f32,
            Target::Trees(_) => self.objects as f32,
            Target::Understory(_) => self.understory as f32,
            Target::TreeSize(_) => self.sizes as f32,
            Target::Pockets(_) => self.pockets as f32,
            Target::Water(_) => self.fluid as f32,
            Target::Height { .. } => self.top,
        }
    }
}

/// The map's columns, one every `step` blocks from block `(x0, z0)`, `w`
/// by `h` of them: what the preview of an import is drawn from.
#[derive(Debug, Clone, Default)]
pub struct Columns {
    pub x0: i32,
    pub z0: i32,
    pub step: i32,
    pub w: usize,
    pub h: usize,
    /// Row by row; `None` where the level has no tile.
    pub cells: Vec<Option<Column>>,
}

/// The columns of `level` over `bounds` (west, north, east, south), the
/// middle one of every square of blocks of a grid at most `side` across.
pub fn columns(level: &crate::level::Level, bounds: [i32; 4], side: usize) -> Columns {
    let [x0, z0, x1, z1] = bounds;
    let longest = (x1 - x0).max(z1 - z0).max(1);
    let step = (longest as usize).div_ceil(side.max(1)).max(1) as i32;
    let (w, h) = (((x1 - x0).max(0) / step) as usize, ((z1 - z0).max(0) / step) as usize);
    let at = |k: usize, base: i32| base + k as i32 * step + step / 2;
    // A row of tiles at a time, each tile read once.
    let mut cells = vec![None; w * h];
    let rows: Vec<(usize, Vec<Option<Column>>)> = (0..h)
        .collect::<Vec<_>>()
        .chunk_by(|&a, &b| at(a, z0).div_euclid(TILE_SIZE) == at(b, z0).div_euclid(TILE_SIZE))
        .map(|c| c.to_vec())
        .collect::<Vec<_>>()
        .into_par_iter()
        .flat_map_iter(|js| {
            let tz = at(js[0], z0).div_euclid(TILE_SIZE);
            let mut out: Vec<(usize, Vec<Option<Column>>)> = js.iter().map(|&j| (j, vec![None; w])).collect();
            let mut i = 0;
            while i < w {
                let tx = at(i, x0).div_euclid(TILE_SIZE);
                let mut end = i;
                while end < w && at(end, x0).div_euclid(TILE_SIZE) == tx {
                    end += 1;
                }
                if let Some(tile) = level.view(TilePos::new(tx, tz)) {
                    for (j, row) in out.iter_mut() {
                        let z = at(*j, z0);
                        for (k, cell) in row.iter_mut().enumerate().take(end).skip(i) {
                            let li = local_index(at(k, x0), z);
                            *cell = Some(Column {
                                present: tile.bottom.is_none() || tile.present(li),
                                top: tile.top.get(li),
                                fluid: tile.fluid.get(li),
                                terrain: tile.terrain.get(li),
                                biome: tile.biome.get(li),
                                annotations: tile.annotations.get(li),
                                cover: tile.cover.get(li),
                                objects: tile.objects.get(li),
                                understory: tile.understory.get(li),
                                sizes: tile.sizes.get(li),
                                pockets: tile.pockets.get(li),
                            });
                        }
                    }
                }
                i = end;
            }
            out
        })
        .collect();
    for (j, row) in rows {
        cells[j * w..(j + 1) * w].copy_from_slice(&row);
    }
    Columns { x0, z0, step, w, h, cells }
}

/// What a column of the map becomes when the image gives it strength `v`
/// (NaN: left alone) and annotation colour `c`, at block `(x, z)`: the
/// new value of the target's kind (see [`Column::value`]), as an import
/// would paint it, the locks aside.
#[allow(clippy::too_many_arguments)]
pub fn outcome(col: &Column, v: f32, c: u16, target: Target, mode: Mode, m: &Mapping, x: i32, z: i32, heights: (f32, f32)) -> f32 {
    let old = col.value(target);
    if v.is_nan() {
        return old;
    }
    if let Target::Height { black, white } = target {
        return height_of(col.top, v, black, white, mode, heights, 1.0);
    }
    if !col.present {
        return old;
    }
    let strength = strength_of(v, target, m);
    match target {
        Target::Water(level) => water_of(col.fluid, strength, level, mode, x, z) as f32,
        _ => plane_of(old as u16, strength, c, target, mode, x, z) as f32,
    }
}

/// The strength of a column that had the value (`had`) or not, with the
/// image's `s`.
#[inline]
fn combine(mode: Mode, had: bool, s: f32) -> f32 {
    let e = if had { 1.0 } else { 0.0 };
    match mode {
        Mode::Replace => s,
        Mode::Add => (e + s).min(1.0),
        Mode::Max => e.max(s),
        Mode::Min => e.min(s),
    }
}

/// The strength of pixel `k` of the image's picture as `mapping` reads it,
/// before any threshold (NaN: left alone); and the grey it has from black
/// to white of its range, before the levels (NaN: transparent).
pub fn preview_value(info: &ImageInfo, mapping: &Mapping, k: usize) -> (f32, f32) {
    let r = Reading::new(info, mapping);
    let g = info.preview.grey[k];
    if g.is_nan() {
        return (f32::NAN, f32::NAN);
    }
    (r.strength(g), ((g - r.low) / r.span).clamp(0.0, 1.0))
}

/// What a pixel of the image's picture becomes, for showing where it
/// lands: its strength (NaN: left alone) and its colour of annotation.
pub fn preview_strength(info: &ImageInfo, mapping: &Mapping, target: Target, k: usize) -> (f32, u16) {
    let r = Reading::new(info, mapping);
    let g = info.preview.grey[k];
    if g.is_nan() {
        return (f32::NAN, 0);
    }
    let v = r.strength(g);
    if v.is_nan() {
        return (v, 0);
    }
    if target.by_colour() {
        return (1.0, nearest_annotation(info.preview.rgb[k]));
    }
    match (mapping.threshold, target) {
        (_, Target::Height { .. }) => (v, 0),
        (Some(t), _) => (if v >= t { 1.0 } else { 0.0 }, 0),
        (None, _) => (v, 0),
    }
}

/// The file beside the images of a region that says where it lies.
pub const SIDECAR: &str = "region.txt";

/// A part of the map written as images (see [`export_region`]): its top
/// left block, its size, and the heights black and white stand for in
/// its heightmap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Region {
    pub x: i32,
    pub z: i32,
    pub width: i32,
    pub depth: i32,
    pub black: f32,
    pub white: f32,
}

impl Region {
    pub fn to_text(&self) -> String {
        format!(
            "# A region of a Massif map: its images are 1 pixel a block, their top left corner\n# on block x, z. In height.png, black is Y `black` and white Y `white`.\n# Import mask reads this file to put the images back in their place.\nx = {}\nz = {}\nwidth = {}\ndepth = {}\nblack = {}\nwhite = {}\n",
            self.x, self.z, self.width, self.depth, self.black, self.white
        )
    }

    pub fn parse(text: &str) -> Option<Region> {
        let mut values = std::collections::HashMap::new();
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('#') {
                continue;
            }
            if let Some((k, v)) = line.split_once(['=', ':']) {
                values.insert(k.trim().to_ascii_lowercase(), v.trim().to_owned());
            }
        }
        let int = |k: &str| values.get(k).and_then(|v| v.parse::<i32>().ok());
        let float = |k: &str| values.get(k).and_then(|v| v.parse::<f32>().ok());
        Some(Region {
            x: int("x")?,
            z: int("z")?,
            width: int("width")?,
            depth: int("depth")?,
            black: float("black").unwrap_or(-64.0),
            white: float("white").unwrap_or(320.0),
        })
    }

    /// The region of the image at `image`: the sidecar in its folder.
    pub fn beside(image: &Path) -> Option<Region> {
        let text = std::fs::read_to_string(image.parent()?.join(SIDECAR)).ok()?;
        Self::parse(&text)
    }

    /// Where an image of `width` × `height` pixels of this region lands:
    /// on its blocks, scaled if it was resized meanwhile.
    pub fn placement(&self, width: usize) -> Placement {
        Placement { x: self.x, z: self.z, scale: self.width as f32 / width.max(1) as f32 }
    }
}

/// What the name of an image of a region says it holds (see
/// [`export_region`]): a layer by its name, which the caller looks for.
#[derive(Debug, Clone, PartialEq)]
pub enum Named {
    Height,
    Annotations,
    Water,
    Terrain(String),
    Biome(String),
    Cover(String),
    Trees(String),
    Understory(String),
    Pockets(String),
    TreeSize(u16),
}

const SIZE_NAMES: [&str; 4] = ["bushes", "small", "medium", "large"];

/// What the file name of an image says it holds, as [`export_region`]
/// names them; `None` for another name.
pub fn named(path: &Path) -> Option<Named> {
    let stem = path.file_stem()?.to_str()?.trim().to_ascii_lowercase();
    let rest = |prefix: &str| stem.strip_prefix(prefix).map(|r| r.trim().to_owned()).filter(|r| !r.is_empty());
    Some(match stem.as_str() {
        "height" | "heightmap" | "height map" => Named::Height,
        "annotations" => Named::Annotations,
        "water" => Named::Water,
        _ => {
            if let Some(r) = rest("under the trees ") {
                Named::Understory(r)
            } else if let Some(r) = rest("tree size ") {
                Named::TreeSize(SIZE_NAMES.iter().position(|n| *n == r)? as u16 + 1)
            } else if let Some(r) = rest("terrain ") {
                Named::Terrain(r)
            } else if let Some(r) = rest("biome ") {
                Named::Biome(r)
            } else if let Some(r) = rest("cover ") {
                Named::Cover(r)
            } else if let Some(r) = rest("trees ") {
                Named::Trees(r)
            } else {
                Named::Pockets(rest("pockets ")?)
            }
        }
    })
}

/// A name as a file name has it: what a file system refuses made `_`.
pub fn file_safe(name: &str) -> String {
    name.chars().map(|c| if c.is_alphanumeric() || " -_'(),.".contains(c) { c } else { '_' }).collect::<String>().trim().to_owned()
}

/// What [`export_region`] writes besides the heightmap.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RegionParts {
    pub height: bool,
    pub annotations: bool,
    pub water: bool,
    /// One image for each terrain, biome or layer found in the region.
    pub terrains: bool,
    pub biomes: bool,
    pub layers: bool,
}

impl Default for RegionParts {
    fn default() -> Self {
        Self { height: true, annotations: true, water: false, terrains: true, biomes: false, layers: true }
    }
}

/// A plane of a tile an image of the region is made from.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Part {
    Height,
    Annotations,
    Water,
    Terrain(u16),
    Biome(u16),
    Cover(u16),
    Trees(u16),
    Understory(u16),
    Pockets(u16),
    TreeSize(u16),
}

impl Part {
    /// The value of column `i` of `tile`: 0 to 65,535 for the height, an
    /// annotation's number, else 255 where it is and 0 where not.
    #[inline]
    fn value(self, tile: &Tile, i: usize, lo: f32, span: f32) -> u16 {
        let on = |b: bool| if b { 255 } else { 0 };
        match self {
            Part::Height => (((tile.top.get(i) - lo) / span).clamp(0.0, 1.0) * 65535.0).round() as u16,
            Part::Annotations => tile.annotations.get(i),
            Part::Water => on(tile.fluid.get(i) != NO_FLUID),
            Part::Terrain(v) => on(tile.terrain.get(i) == v),
            Part::Biome(v) => on(tile.biome.get(i) == v),
            Part::Cover(v) => on(tile.cover.get(i) == v),
            Part::Trees(v) => on(tile.objects.get(i) == v),
            Part::Understory(v) => on(tile.understory.get(i) == v),
            Part::Pockets(v) => on(tile.pockets.get(i) == v),
            Part::TreeSize(v) => on(tile.sizes.get(i) == v),
        }
    }
}

/// The values a plane has within `clip` of a tile.
fn values_in(plane: &crate::plane::Plane<u16>, clip: &BlockRect, into: &mut std::collections::BTreeSet<u16>) {
    if let Some(v) = plane.uniform_value() {
        into.insert(v);
        return;
    }
    let dense = plane.dense();
    for z in clip.z0..clip.z1 {
        for x in clip.x0..clip.x1 {
            into.insert(dense[local_index(x, z)]);
        }
    }
}

/// Write the part `rect` of the ground of `dim` into the folder `dir` as
/// images of 1 pixel a block, with a [`SIDECAR`] saying where it lies:
/// `height.png` (16 bits, black the world's floor, white its top),
/// `annotations.png` (their colours, black where none; when the region
/// has some), `water.png` (white where there is water), and
/// for each terrain, biome or layer found in it an image of where it is
/// (white) and is not (black), named as [`named`] reads them back. Rows
/// are written as they are read, a row of tiles at a time. Returns the
/// files written.
pub fn export_region(dim: &crate::world::Dimension, reg: &crate::registry::Registry, rect: BlockRect, dir: &Path, parts: &RegionParts, steps: &Steps) -> Result<Vec<std::path::PathBuf>, String> {
    use std::collections::BTreeSet;
    if rect.is_empty() {
        return Err("an empty region".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let ground = dim.ground();
    let (lo, span) = (dim.min_y as f32, (dim.max_y - dim.min_y) as f32);
    // What the region has, to give each its image.
    let mut found: [BTreeSet<u16>; 8] = Default::default();
    if parts.terrains || parts.biomes || parts.layers || parts.annotations {
        for pos in rect.tiles() {
            let Some(tile) = ground.view(pos) else { continue };
            let clip = rect.intersect(&pos.rect());
            if parts.terrains {
                values_in(&tile.terrain, &clip, &mut found[0]);
            }
            if parts.biomes {
                values_in(&tile.biome, &clip, &mut found[1]);
            }
            if parts.annotations {
                values_in(&tile.annotations, &clip, &mut found[7]);
            }
            if parts.layers {
                for (k, plane) in [&tile.cover, &tile.objects, &tile.understory, &tile.pockets, &tile.sizes].into_iter().enumerate() {
                    values_in(plane, &clip, &mut found[k + 2]);
                }
            }
        }
    }
    let mut outs: Vec<(Part, String)> = Vec::new();
    if parts.height {
        outs.push((Part::Height, "height".into()));
    }
    // Annotations only where there are some.
    if parts.annotations && found[7].iter().any(|&a| a != 0) {
        outs.push((Part::Annotations, "annotations".into()));
    }
    if parts.water {
        outs.push((Part::Water, "water".into()));
    }
    for &t in &found[0] {
        if let Some(def) = reg.terrains.get(t as usize) {
            outs.push((Part::Terrain(t), format!("terrain {}", def.name)));
        }
    }
    for &b in found[1].iter().filter(|&&b| b != 0) {
        if let Some(def) = reg.biomes.get(b as usize) {
            outs.push((Part::Biome(b), format!("biome {}", def.name)));
        }
    }
    for &n in found[2].iter().filter(|&&n| n != 0) {
        if let Some(def) = reg.cover(n) {
            outs.push((Part::Cover(n), format!("cover {}", def.name)));
        }
    }
    for &n in found[3].iter().filter(|&&n| n != 0) {
        if let Some(def) = reg.object_layer(n) {
            outs.push((Part::Trees(n), format!("trees {}", def.name)));
        }
    }
    for &n in found[4].iter().filter(|&&n| n != 0) {
        if let Some(def) = reg.object_layer(n) {
            outs.push((Part::Understory(n), format!("under the trees {}", def.name)));
        }
    }
    for &n in found[5].iter().filter(|&&n| n != 0) {
        if let Some(def) = reg.pocket(n) {
            outs.push((Part::Pockets(n), format!("pockets {}", def.name)));
        }
    }
    for &n in found[6].iter().filter(|&&n| (1..=4).contains(&n)) {
        outs.push((Part::TreeSize(n), format!("tree size {}", SIZE_NAMES[n as usize - 1])));
    }
    let (w, d) = (rect.width() as usize, rect.depth() as usize);
    let mut files = Vec::new();
    let mut writers = Vec::new();
    let mut used = std::collections::HashSet::new();
    for (part, name) in &outs {
        let mut name = file_safe(name);
        while !used.insert(name.to_ascii_lowercase()) {
            name.push('_');
        }
        let path = dir.join(format!("{name}.png"));
        let file = std::fs::File::create(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w as u32, d as u32);
        let (colour, depth) = match part {
            Part::Height => (png::ColorType::Grayscale, png::BitDepth::Sixteen),
            Part::Annotations => (png::ColorType::Rgb, png::BitDepth::Eight),
            _ => (png::ColorType::Grayscale, png::BitDepth::Eight),
        };
        enc.set_color(colour);
        enc.set_depth(depth);
        enc.set_compression(png::Compression::Fast);
        let writer = enc.write_header().map_err(|e| e.to_string())?.into_stream_writer().map_err(|e| e.to_string())?;
        writers.push((*part, writer));
        files.push(path);
    }
    steps.total.store(d, Ordering::Relaxed);
    steps.done.store(0, Ordering::Relaxed);
    let (tz0, tz1) = (rect.z0.div_euclid(TILE_SIZE), (rect.z1 - 1).div_euclid(TILE_SIZE));
    let (tx0, tx1) = (rect.x0.div_euclid(TILE_SIZE), (rect.x1 - 1).div_euclid(TILE_SIZE));
    for tz in tz0..=tz1 {
        if steps.cancel.load(Ordering::Relaxed) {
            return Err("stopped".into());
        }
        let tiles: Vec<Option<crate::level::TileRef<'_>>> = (tx0..=tx1).map(|tx| ground.view(TilePos::new(tx, tz))).collect();
        let (z0, z1) = ((tz * TILE_SIZE).max(rect.z0), ((tz + 1) * TILE_SIZE).min(rect.z1));
        // Each image's rows of this band, made side by side.
        let bands: Vec<Vec<u8>> = writers
            .par_iter()
            .map(|(part, _)| {
                let bytes = match part {
                    Part::Height => 2,
                    Part::Annotations => 3,
                    _ => 1,
                };
                let mut out = Vec::with_capacity(w * (z1 - z0) as usize * bytes);
                for z in z0..z1 {
                    for x in rect.x0..rect.x1 {
                        let v = tiles[(x.div_euclid(TILE_SIZE) - tx0) as usize].as_ref().map_or(0, |t| part.value(t, local_index(x, z), lo, span));
                        match part {
                            Part::Height => out.extend_from_slice(&v.to_be_bytes()),
                            Part::Annotations => out.extend_from_slice(&reg.annotations.color(v).unwrap_or([0; 3])),
                            _ => out.push(v as u8),
                        }
                    }
                }
                out
            })
            .collect();
        for ((_, writer), band) in writers.iter_mut().zip(bands) {
            std::io::Write::write_all(writer, &band).map_err(|e| e.to_string())?;
        }
        steps.done.store((z1 - rect.z0) as usize, Ordering::Relaxed);
    }
    for (_, writer) in writers {
        writer.finish().map_err(|e| e.to_string())?;
    }
    let region = Region { x: rect.x0, z: rect.z0, width: rect.width(), depth: rect.depth(), black: lo, white: lo + span };
    let sidecar = dir.join(SIDECAR);
    std::fs::write(&sidecar, region.to_text()).map_err(|e| e.to_string())?;
    files.push(sidecar);
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::History;
    use crate::world::{Dimension, DimensionKind, World, WorldMeta};

    fn dir() -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("massif-mask-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A world of `n` × `n` tiles of grass at 64, without water.
    fn world(n: i32) -> World {
        let mut world = World::new(WorldMeta::default());
        let grass = world.registry.terrain_by_name("Grass").unwrap();
        let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
        for tz in 0..n {
            for tx in 0..n {
                dim.ground_mut().insert(TilePos::new(tx, tz), Tile::ground(64.0, grass.0, NO_FLUID));
            }
        }
        world.dims = vec![dim];
        world
    }

    /// Survey and apply, in one step to undo; the history to undo it.
    fn import(world: &mut World, path: &Path, settings: MaskSettings) -> (Changes, History) {
        let info = survey(path, &Steps::default()).unwrap();
        let mut history = History::new(4);
        history.locks = crate::registry::Locks::of(&world.registry);
        history.begin("import mask", 0);
        let steps = Steps::default();
        let changes = apply(&mut history.edit(world, 0), path, &info, &settings, &steps).unwrap();
        history.commit();
        assert_eq!(steps.fraction(), 1.0);
        (changes, history)
    }

    fn at<T: Copy + PartialEq>(world: &World, x: i32, z: i32, f: impl Fn(&Tile) -> &crate::plane::Plane<T>) -> T {
        let tile = world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
        f(&tile).get(local_index(x, z))
    }

    fn settings(target: Target) -> MaskSettings {
        MaskSettings {
            target,
            mapping: Mapping::default(),
            mode: Mode::Replace,
            placement: Placement { x: 0, z: 0, scale: 1.0 },
            feather: 0,
        }
    }

    /// A grey PNG of `w` × `h`, 8 bits, `f(x, y)` each pixel.
    fn grey_png(name: &str, w: u32, h: u32, f: impl Fn(u32, u32) -> u8) -> std::path::PathBuf {
        let path = dir().join(name);
        image::GrayImage::from_fn(w, h, |x, y| image::Luma([f(x, y)])).save(&path).unwrap();
        path
    }

    #[test]
    fn levels_and_midtones_shape_the_strength() {
        let mut m = Mapping::default();
        assert_eq!(m.shape(0.3), 0.3, "as it is by default");
        m.levels = [0.2, 0.6];
        assert_eq!(m.shape(0.1), 0.0);
        assert!((m.shape(0.4) - 0.5).abs() < 1e-6);
        assert_eq!(m.shape(0.9), 1.0);
        m.gamma = 2.0;
        assert!((m.shape(0.4) - 0.5f32.sqrt()).abs() < 1e-6, "lighter midtones");
        // A threshold of 50 % takes in a grey of 43 %.
        let mut w = world(1);
        let png = grey_png("levels.png", 128, 128, |_, _| 110);
        let mut s = settings(Target::Cover(1));
        s.mapping.levels = [0.2, 0.6];
        import(&mut w, &png, s);
        assert_eq!(at(&w, 10, 10, |t| &t.cover), 1);
    }

    #[test]
    fn a_blur_spreads_across_the_bands_without_a_seam() {
        let mut v = vec![f32::NAN; 3];
        v.extend([0.0, 0.0, 1.0, 1.0, 0.0, 0.0, f32::NAN]);
        blur_grid(&mut v, 10, 2);
        assert!(v[0].is_nan() && v[9].is_nan(), "left alone stays left alone");
        assert!(v[4] > 0.0 && v[4] < v[5] && (v[5] - v[6]).abs() < 1e-6, "{v:?}");
        // A heightmap of a step, blurred over 16 blocks: the slope crosses
        // the seam between two rows of tiles (z 128) smoothly.
        let mut w = world(2);
        let png = grey_png("step.png", 256, 256, |_, y| if y < 128 { 0 } else { 255 });
        let mut s = settings(Target::Height { black: 64.0, white: 96.0 });
        s.mapping.threshold = None;
        s.mapping.black_is_none = false;
        s.mapping.blur = 16.0;
        import(&mut w, &png, s);
        let column: Vec<f32> = (100..160).map(|z| at(&w, 50, z, |t| &t.top)).collect();
        assert!(column.windows(2).all(|p| p[1] >= p[0] && p[1] - p[0] < 3.0), "{column:?}");
        assert!(at(&w, 50, 100, |t| &t.top) < 66.0 && at(&w, 50, 160, |t| &t.top) > 94.0);
    }

    #[test]
    fn the_preview_predicts_what_the_import_paints() {
        let w = world(2);
        let sand = w.registry.terrain_by_name("Sand").unwrap().0;
        let png = grey_png("predict.png", 256, 256, |x, y| ((x * 7 + y * 3) % 256) as u8);
        let info = survey(&png, &Steps::default()).unwrap();
        for (target, threshold) in [(Target::Terrain(sand), Some(0.5)), (Target::Cover(2), None), (Target::Biome(3), Some(0.3)), (Target::Water(70), Some(0.6)), (Target::Height { black: 40.0, white: 120.0 }, None)] {
            let mut w = w.clone();
            let mut s = settings(target);
            s.mapping.threshold = threshold;
            s.mapping.black_is_none = false;
            let before = columns(w.dims[0].ground(), [0, 0, 256, 256], 256);
            assert_eq!((before.w, before.h, before.step), (256, 256, 1));
            import(&mut w, &png, s);
            let after = columns(w.dims[0].ground(), [0, 0, 256, 256], 256);
            let heights = (w.dims[0].min_y as f32, w.dims[0].max_y as f32);
            let r = Reading::new(&info, &s.mapping);
            for k in (0..256 * 256).step_by(37) {
                let (x, z) = ((k % 256) as i32, (k / 256) as i32);
                let v = r.strength(((x * 7 + z * 3) % 256) as f32);
                let col = before.cells[k].unwrap();
                let want = after.cells[k].unwrap().value(target);
                assert_eq!(outcome(&col, v, 0, target, s.mode, &s.mapping, x, z, heights), want, "{target:?} at {x}, {z}");
            }
        }
        // Coarser: a column every 4 blocks, in the middle of its square.
        let c = columns(w.dims[0].ground(), [0, 0, 256, 256], 64);
        assert_eq!((c.w, c.h, c.step), (64, 64, 4));
        assert!(c.cells.iter().all(Option::is_some));
        let c = columns(w.dims[0].ground(), [-128, 0, 256, 256], 96);
        assert!(c.cells[0].is_none(), "no tile there");
    }

    #[test]
    fn a_layer_is_painted_where_the_image_is_white_and_undo_takes_it_back() {
        let mut w = world(2);
        // White on the left half of 256 × 128.
        let png = grey_png("half.png", 256, 128, |x, _| if x < 128 { 255 } else { 0 });
        let before = w.dims[0].ground().view(TilePos::new(0, 0)).unwrap().clone();
        let (changes, mut history) = import(&mut w, &png, settings(Target::Cover(1)));
        assert_eq!(changes.columns, 128 * 128);
        assert_eq!(at(&w, 5, 5, |t| &t.cover), 1);
        assert_eq!(at(&w, 200, 5, |t| &t.cover), 0);
        assert_eq!(at(&w, 5, 200, |t| &t.cover), 0, "below the image, nothing");
        history.undo(&mut w).unwrap();
        assert_eq!(w.dims[0].ground().view(TilePos::new(0, 0)).unwrap().clone(), before);
    }

    #[test]
    fn a_grey_ramp_scatters_a_layer_as_strong_as_it_is_bright() {
        let mut w = world(1);
        // Four bands of 32 columns: 0, 1/3, 2/3, white.
        let png = grey_png("ramp.png", 128, 128, |x, _| [0, 85, 170, 255][x as usize / 32]);
        let mut s = settings(Target::Trees(1));
        s.mapping.threshold = None;
        s.mapping.black_is_none = false;
        import(&mut w, &png, s);
        let share = |band: i32| (0..128).flat_map(|z| (band * 32..band * 32 + 32).map(move |x| (x, z))).filter(|&(x, z)| at(&w, x, z, |t| &t.objects) == 1).count() as f32 / 4096.0;
        assert_eq!(share(0), 0.0);
        assert!((share(1) - 1.0 / 3.0).abs() < 0.04, "{}", share(1));
        assert!((share(2) - 2.0 / 3.0).abs() < 0.04, "{}", share(2));
        assert_eq!(share(3), 1.0);
    }

    #[test]
    fn colours_become_the_nearest_annotations_and_black_none() {
        let mut w = world(1);
        let path = dir().join("colours.png");
        // Orange, red, a near-blue, black.
        let colours = [[250u8, 120, 20], [160, 40, 35], [50, 60, 150], [0, 0, 0]];
        image::RgbImage::from_fn(128, 128, |x, _| image::Rgb(colours[x as usize / 32])).save(&path).unwrap();
        import(&mut w, &path, settings(Target::Annotations(None)));
        assert_eq!(at(&w, 0, 0, |t| &t.annotations), 2, "orange");
        assert_eq!(at(&w, 40, 0, |t| &t.annotations), 14, "red");
        assert_eq!(at(&w, 70, 0, |t| &t.annotations), 11, "blue");
        assert_eq!(at(&w, 100, 0, |t| &t.annotations), 0, "black is none");
        // One colour where the image is not black.
        let mut w = world(1);
        import(&mut w, &path, settings(Target::Annotations(Some(6))));
        assert_eq!(at(&w, 0, 0, |t| &t.annotations), 6);
        assert_eq!(at(&w, 100, 0, |t| &t.annotations), 0);
    }

    #[test]
    fn a_terrain_and_a_biome_take_the_threshold_and_invert() {
        let mut w = world(1);
        let sand = w.registry.terrain_by_name("Sand").unwrap().0;
        let grass = w.registry.terrain_by_name("Grass").unwrap().0;
        let png = grey_png("terrain.png", 128, 128, |x, _| (x * 2) as u8);
        let mut s = settings(Target::Terrain(sand));
        s.mapping.threshold = Some(0.5);
        s.mapping.invert = true;
        s.mapping.black_is_none = false;
        import(&mut w, &png, s);
        assert_eq!(at(&w, 10, 3, |t| &t.terrain), sand, "dark, inverted: on");
        assert_eq!(at(&w, 120, 3, |t| &t.terrain), grass, "a terrain is not rubbed out");
        let mut s = settings(Target::Biome(7));
        s.mapping.threshold = Some(0.5);
        import(&mut w, &png, s);
        assert_eq!(at(&w, 120, 3, |t| &t.biome), 7);
        assert_eq!(at(&w, 10, 3, |t| &t.biome), 0);
    }

    #[test]
    fn modes_add_max_and_min_go_with_what_is_painted() {
        let png = grey_png("left.png", 128, 128, |x, _| if x < 64 { 255 } else { 0 });
        let painted = |mode: Mode| {
            let mut w = world(1);
            // The top half has the layer already.
            {
                let mut h = History::new(2);
                h.begin("setup", 0);
                let mut e = h.edit(&mut w, 0);
                let t = e.tile_mut(TilePos::new(0, 0)).unwrap();
                for z in 0..64 {
                    for x in 0..128 {
                        t.cover.set(local_index(x, z), 2);
                    }
                }
            }
            let mut s = settings(Target::Cover(2));
            s.mode = mode;
            s.mapping.black_is_none = false;
            import(&mut w, &png, s);
            [(10, 10), (100, 10), (10, 100), (100, 100)].map(|(x, z)| at(&w, x, z, |t| &t.cover))
        };
        // Top left, top right, bottom left, bottom right.
        assert_eq!(painted(Mode::Replace), [2, 0, 2, 0]);
        assert_eq!(painted(Mode::Add), [2, 2, 2, 0]);
        assert_eq!(painted(Mode::Max), [2, 2, 2, 0]);
        assert_eq!(painted(Mode::Min), [2, 0, 0, 0]);
    }

    #[test]
    fn a_locked_layer_keeps_its_columns() {
        let mut w = world(1);
        w.registry.covers[0].locked = true;
        {
            let mut h = History::new(2);
            h.begin("setup", 0);
            let mut e = h.edit(&mut w, 0);
            let t = e.tile_mut(TilePos::new(0, 0)).unwrap();
            for x in 0..128 {
                t.cover.set(local_index(x, 0), 1);
            }
        }
        let png = grey_png("all.png", 128, 128, |_, _| 255);
        import(&mut w, &png, settings(Target::Cover(2)));
        assert_eq!(at(&w, 5, 0, |t| &t.cover), 1, "the locked cover stays");
        assert_eq!(at(&w, 5, 1, |t| &t.cover), 2);
    }

    #[test]
    fn a_16_bit_heightmap_sets_the_height_with_scale_and_offset() {
        let mut w = world(1);
        let path = dir().join("height.png");
        image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_fn(128, 128, |x, _| image::Luma([(x * 516) as u16])).save(&path).unwrap();
        let mut s = settings(Target::Height { black: 40.0, white: 140.0 });
        s.mapping.black_is_none = false;
        let info = survey(&path, &Steps::default()).unwrap();
        assert_eq!((info.header.bits, info.high), (16, 127.0 * 516.0));
        import(&mut w, &path, s);
        assert_eq!(at(&w, 0, 0, |t| &t.top), 40.0);
        let h = at(&w, 127, 5, |t| &t.top);
        assert!((h - (40.0 + 100.0 * 127.0 * 516.0 / 65535.0)).abs() < 0.01, "{h}");
        // Add raises what is there.
        let mut s = settings(Target::Height { black: 0.0, white: 10.0 });
        s.mode = Mode::Add;
        s.mapping.black_is_none = false;
        let before = at(&w, 64, 5, |t| &t.top);
        import(&mut w, &path, s);
        let after = at(&w, 64, 5, |t| &t.top);
        assert!((after - before - 10.0 * 64.0 * 516.0 / 65535.0).abs() < 0.01);
    }

    #[test]
    fn water_fills_where_the_image_says_and_replace_drains_the_rest() {
        let mut w = world(1);
        let png = grey_png("water.png", 128, 128, |_, y| if y < 64 { 255 } else { 0 });
        let mut s = settings(Target::Water(70));
        s.mapping.black_is_none = false;
        import(&mut w, &png, s);
        assert_eq!(at(&w, 3, 3, |t| &t.fluid), 70);
        assert_eq!(at(&w, 3, 100, |t| &t.fluid), NO_FLUID);
    }

    #[test]
    fn placement_scales_and_offsets_the_image() {
        let mut w = world(4);
        // 64 × 64: white in the top-left quarter, scaled ×4 at (100, 50).
        let png = grey_png("small.png", 64, 64, |x, y| if x < 32 && y < 32 { 255 } else { 0 });
        let mut s = settings(Target::TreeSize(3));
        s.placement = Placement { x: 100, z: 50, scale: 4.0 };
        import(&mut w, &png, s);
        assert_eq!(at(&w, 99, 60, |t| &t.sizes), 0);
        assert_eq!(at(&w, 100, 50, |t| &t.sizes), 3);
        assert_eq!(at(&w, 100 + 127, 50 + 127, |t| &t.sizes), 3);
        assert_eq!(at(&w, 100 + 129, 50 + 129, |t| &t.sizes), 0);
        // Shrunk to a quarter: 16 blocks of white.
        let mut w = world(1);
        let mut s = settings(Target::TreeSize(2));
        s.placement = Placement { x: 0, z: 0, scale: 0.25 };
        import(&mut w, &png, s);
        assert_eq!(at(&w, 7, 7, |t| &t.sizes), 2);
        assert_eq!(at(&w, 8, 7, |t| &t.sizes), 0);
    }

    #[test]
    fn tiff_bmp_and_jpeg_read_as_png_does() {
        let png = grey_png("same.png", 300, 200, |x, y| ((x * 7 + y * 3) % 256) as u8);
        let img = image::open(&png).unwrap().into_luma8();
        let mut want = Row::default();
        let read_all = |path: &Path| {
            let mut r = open(path).unwrap();
            let mut rows = Vec::new();
            let mut row = Row::default();
            while r.next(&mut row).unwrap() {
                rows.push(row.grey.clone());
            }
            rows
        };
        // TIFF in strips, 16-bit, and BMP of 24 and 8 bits.
        let tif = dir().join("same.tif");
        {
            use tiff::encoder::{TiffEncoder, colortype::Gray8};
            let mut enc = TiffEncoder::new(std::fs::File::create(&tif).unwrap()).unwrap();
            enc.write_image::<Gray8>(300, 200, img.as_raw()).unwrap();
        }
        let bmp = dir().join("same.bmp");
        image::DynamicImage::ImageLuma8(img.clone()).to_rgb8().save(&bmp).unwrap();
        let expected: Vec<Vec<f32>> = img.rows().map(|r| r.map(|p| p.0[0] as f32).collect()).collect();
        assert_eq!(read_all(&png), expected);
        assert_eq!(read_all(&tif), expected);
        let got = read_all(&bmp);
        assert_eq!(got.len(), 200);
        for (a, b) in got.iter().zip(&expected) {
            for (x, y) in a.iter().zip(b) {
                assert!((x - y).abs() < 1.0);
            }
        }
        let jpg = dir().join("same.jpg");
        image::DynamicImage::ImageLuma8(img).save(&jpg).unwrap();
        assert_eq!(read_all(&jpg).len(), 200);
        want.clear();
    }

    #[test]
    fn a_heightmap_fades_into_the_land_around_it() {
        let mut w = world(2);
        // A plateau at 100 over a part of the map, faded over 16 blocks.
        let png = grey_png("plateau.png", 128, 128, |_, _| 255);
        let mut s = settings(Target::Height { black: 0.0, white: 100.0 });
        s.placement = Placement { x: 64, z: 64, scale: 1.0 };
        s.feather = 16;
        import(&mut w, &png, s);
        assert_eq!(at(&w, 63, 100, |t| &t.top), 64.0, "outside, as it was");
        let edge = at(&w, 64, 100, |t| &t.top);
        assert!(edge > 64.0 && edge < 66.0, "at the edge, almost the land's: {edge}");
        let mid = at(&w, 72, 100, |t| &t.top);
        assert!(mid > 75.0 && mid < 90.0, "half way: {mid}");
        assert_eq!(at(&w, 100, 100, |t| &t.top), 100.0, "inside, the image's");
        // Each step to the next column stays small.
        let steps: Vec<f32> = (64..81).map(|x| at(&w, x, 100, |t| &t.top)).collect();
        assert!(steps.windows(2).all(|p| p[1] - p[0] < 4.5), "{steps:?}");
    }

    #[test]
    fn several_images_are_one_step_to_undo_heights_first() {
        let mut w = world(1);
        let before = w.dims[0].ground().view(TilePos::new(0, 0)).unwrap().clone();
        let a = grey_png("several-a.png", 128, 128, |x, _| if x < 64 { 255 } else { 0 });
        let b = grey_png("several-b.png", 128, 128, |_, y| if y < 64 { 255 } else { 0 });
        let info = |p: &Path| std::sync::Arc::new(survey(p, &Steps::default()).unwrap());
        let mut height = settings(Target::Height { black: 64.0, white: 80.0 });
        height.mapping.black_is_none = false;
        let images = vec![(a.clone(), info(&a), settings(Target::Cover(1))), (b.clone(), info(&b), height)];
        let mut history = History::new(4);
        history.begin("import masks", 0);
        let steps = Steps::default();
        let changes = apply_all(&mut history.edit(&mut w, 0), &images, &steps).unwrap();
        history.commit();
        assert_eq!(steps.fraction(), 1.0);
        assert_eq!(changes.tiles, vec![TilePos::new(0, 0)]);
        assert_eq!((at(&w, 10, 10, |t| &t.cover), at(&w, 10, 10, |t| &t.top)), (1, 80.0));
        assert_eq!((at(&w, 100, 100, |t| &t.cover), at(&w, 100, 100, |t| &t.top)), (0, 64.0));
        assert_eq!(history.undo_count(), 1);
        history.undo(&mut w).unwrap();
        assert_eq!(*w.dims[0].ground().view(TilePos::new(0, 0)).unwrap(), before);
    }

    #[test]
    fn a_region_goes_out_and_comes_back_in_its_place() {
        // Heights, a terrain and a cover painted in a part of the map.
        let mut w = world(3);
        let sand = w.registry.terrain_by_name("Sand").unwrap().0;
        {
            let mut h = History::new(2);
            h.begin("setup", 0);
            let mut e = h.edit(&mut w, 0);
            let t = e.tile_mut(TilePos::new(1, 1)).unwrap();
            for z in 0..128 {
                for x in 0..128 {
                    let i = local_index(x, z);
                    t.top.set(i, 50.0 + ((x * 3 + z) % 90) as f32 + 0.5);
                    if x > z {
                        t.terrain.set(i, sand);
                    }
                    if (x / 16 + z / 16) % 2 == 0 {
                        t.cover.set(i, 2);
                    }
                    if (30..40).contains(&x) {
                        t.annotations.set(i, 4);
                    }
                }
            }
        }
        let rect = BlockRect::new(150, 140, 250, 260);
        let dir = dir().join("region");
        let _ = std::fs::remove_dir_all(&dir);
        let files = export_region(&w.dims[0], &w.registry, rect, &dir, &RegionParts::default(), &Steps::default()).unwrap();
        let names: Vec<String> = files.iter().map(|f| f.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.contains(&"height.png".to_owned()) && names.contains(&"terrain Sand.png".to_owned()) && names.contains(&SIDECAR.to_owned()), "{names:?}");
        let cover = w.registry.cover(2).unwrap().name.clone();
        assert!(names.contains(&format!("cover {}.png", file_safe(&cover))), "{names:?}");
        let region = Region::beside(&dir.join("height.png")).unwrap();
        assert_eq!((region.x, region.z, region.width, region.depth, region.black, region.white), (150, 140, 100, 120, -64.0, 320.0));
        assert_eq!(named(&dir.join("terrain Sand.png")), Some(Named::Terrain("sand".into())));
        assert_eq!(named(&dir.join("tree size medium.png")), Some(Named::TreeSize(3)));

        // Into a flat map, each where it came from.
        let mut back = world(3);
        let place = region.placement(100);
        let mut height = settings(Target::Height { black: region.black, white: region.white });
        height.placement = place;
        height.mapping.black_is_none = false;
        let mut terrain = settings(Target::Terrain(sand));
        terrain.placement = place;
        let mut covers = settings(Target::Cover(2));
        covers.placement = place;
        let mut notes = settings(Target::Annotations(None));
        notes.placement = place;
        let info = |name: &str| (dir.join(name), std::sync::Arc::new(survey(&dir.join(name), &Steps::default()).unwrap()));
        let images: Vec<_> = [("height.png", height), ("terrain Sand.png", terrain), (&format!("cover {}.png", file_safe(&cover)) as &str, covers), ("annotations.png", notes)]
            .into_iter()
            .map(|(n, s)| {
                let (p, i) = info(n);
                (p, i, s)
            })
            .collect();
        let mut history = History::new(2);
        history.begin("back", 0);
        apply_all(&mut history.edit(&mut back, 0), &images, &Steps::default()).unwrap();
        for z in rect.z0..rect.z1 {
            for x in rect.x0..rect.x1 {
                let (a, b) = (at(&w, x, z, |t| &t.top), at(&back, x, z, |t| &t.top));
                assert!((a - b).abs() <= 384.0 / 65535.0 + 1.0 / 256.0, "height at {x}, {z}: {a} then {b}");
                assert_eq!(at(&w, x, z, |t| &t.terrain), at(&back, x, z, |t| &t.terrain), "terrain at {x}, {z}");
                assert_eq!(at(&w, x, z, |t| &t.cover), at(&back, x, z, |t| &t.cover), "cover at {x}, {z}");
                assert_eq!(at(&w, x, z, |t| &t.annotations), at(&back, x, z, |t| &t.annotations), "annotation at {x}, {z}");
            }
        }
        assert_eq!(at(&back, 149, 200, |t| &t.top), 64.0, "outside the region, as it was");
    }

    /// The time to import a large image, and the memory it takes: with
    /// `MASSIF_MASK_SIDE` (default 16384) a side of the image and the map.
    #[test]
    #[ignore]
    fn a_large_image_streams() {
        let side: u32 = std::env::var("MASSIF_MASK_SIDE").ok().and_then(|s| s.parse().ok()).unwrap_or(16384);
        let path = dir().join(format!("large-{side}.png"));
        if !path.exists() {
            let t = std::time::Instant::now();
            let file = std::fs::File::create(&path).unwrap();
            let mut enc = png::Encoder::new(std::io::BufWriter::new(file), side, side);
            enc.set_color(png::ColorType::Grayscale);
            enc.set_depth(png::BitDepth::Eight);
            let mut writer = enc.write_header().unwrap().into_stream_writer().unwrap();
            let mut line = vec![0u8; side as usize];
            for y in 0..side {
                for (x, v) in line.iter_mut().enumerate() {
                    let (dx, dy) = (x as f32 - side as f32 / 2.0, y as f32 - side as f32 / 2.0);
                    *v = ((dx * dx + dy * dy).sqrt() / side as f32 * 512.0) as u8;
                }
                std::io::Write::write_all(&mut writer, &line).unwrap();
            }
            writer.finish().unwrap();
            eprintln!("wrote {side}² in {:.1} s", t.elapsed().as_secs_f32());
        }
        let n = side as i32 / TILE_SIZE;
        let mut w = world(n);
        let t = std::time::Instant::now();
        let info = survey(&path, &Steps::default()).unwrap();
        let surveyed = t.elapsed().as_secs_f32();
        let mut s = settings(Target::Trees(1));
        s.mapping.threshold = None;
        let mut history = History::new(4);
        history.begin("import mask", 0);
        let t = std::time::Instant::now();
        let changes = apply(&mut history.edit(&mut w, 0), &path, &info, &s, &Steps::default()).unwrap();
        let applied = t.elapsed().as_secs_f32();
        eprintln!("{side}² ({} tiles): survey {surveyed:.2} s, apply {applied:.2} s, {} columns changed", n * n, changes.columns);
        drop(history);
        let out = dir().join(format!("large-region-{side}"));
        let t = std::time::Instant::now();
        let parts = RegionParts { annotations: false, ..RegionParts::default() };
        let files = export_region(&w.dims[0], &w.registry, BlockRect::new(0, 0, side as i32, side as i32), &out, &parts, &Steps::default()).unwrap();
        eprintln!("region of {side}²: {} files in {:.2} s", files.len(), t.elapsed().as_secs_f32());
        let _ = std::fs::remove_dir_all(&out);
    }
}
