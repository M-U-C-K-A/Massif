//! Byte-level codecs shared by cold tiles and the file format.
//!
//! * LEB128 varints (zig-zag for signed values);
//! * height planes: fixed-point 1/256 block, 2D gradient predictor, zig-zag
//!   varint residuals — smooth terrain ends up near one byte per column
//!   *before* the entropy coder;
//! * categorical planes: local palette + split-stream RLE (run values and run
//!   lengths in separate streams), dense fallback for chaotic planes.

use crate::coords::{TILE_AREA, TILE_SIZE};
use crate::plane::Plane;

#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    #[error("unexpected end of data")]
    Eof,
    #[error("corrupt data: {0}")]
    Corrupt(&'static str),
    #[error("unsupported tile encoding version {0}")]
    Version(u8),
    #[error("zstd: {0}")]
    Zstd(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, CodecError>;

/// Heights are stored with 8 fractional bits.
pub const HEIGHT_SCALE: f32 = 256.0;

/// Snap a height to the 1/256 grid the codec stores exactly.
#[inline]
pub fn snap_height(h: f32) -> f32 {
    (h * HEIGHT_SCALE).round() / HEIGHT_SCALE
}

#[inline]
fn quantize(h: f32) -> i32 {
    // `as` saturates and maps NaN to 0, so any f32 is safe here.
    (h * HEIGHT_SCALE).round() as i32
}

// ---------------------------------------------------------------- varints --

#[inline]
pub fn put_varu(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

#[inline]
pub fn put_vari(out: &mut Vec<u8>, v: i64) {
    put_varu(out, ((v << 1) ^ (v >> 63)) as u64);
}

/// A bounds-checked cursor over a byte slice.
pub struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.pos >= self.data.len()
    }

    #[inline]
    pub fn u8(&mut self) -> Result<u8> {
        let b = *self.data.get(self.pos).ok_or(CodecError::Eof)?;
        self.pos += 1;
        Ok(b)
    }

    pub fn bytes(&mut self, n: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(n).ok_or(CodecError::Eof)?;
        let s = self.data.get(self.pos..end).ok_or(CodecError::Eof)?;
        self.pos = end;
        Ok(s)
    }

    #[inline]
    pub fn varu(&mut self) -> Result<u64> {
        let mut v = 0u64;
        let mut shift = 0u32;
        loop {
            let b = self.u8()?;
            if shift >= 64 || (shift == 63 && b > 1) {
                return Err(CodecError::Corrupt("varint overflow"));
            }
            v |= ((b & 0x7f) as u64) << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
            shift += 7;
        }
    }

    #[inline]
    pub fn vari(&mut self) -> Result<i64> {
        let u = self.varu()?;
        Ok(((u >> 1) as i64) ^ -((u & 1) as i64))
    }

    /// [`Reader::vari`] truncated to 32 bits, quick for the one byte
    /// varints that make up most planes.
    #[inline(always)]
    pub fn vari32(&mut self) -> Result<i32> {
        match self.data.get(self.pos) {
            Some(&b) if b < 0x80 => {
                self.pos += 1;
                let u = b as i32;
                Ok((u >> 1) ^ -(u & 1))
            }
            _ => Ok(self.vari()? as i32),
        }
    }

    /// Fill `out` with [`Reader::vari32`]s, eight at a time where eight
    /// one byte varints follow each other, as they mostly do.
    pub fn vari32s(&mut self, out: &mut [i32]) -> Result<()> {
        let zigzag = |u: u8| ((u >> 1) as i32) ^ -((u & 1) as i32);
        for chunk in out.chunks_mut(8) {
            if let Some(bytes) = self.data.get(self.pos..self.pos + 8)
                && chunk.len() == 8
                && u64::from_le_bytes(bytes.try_into().unwrap()) & 0x8080_8080_8080_8080 == 0
            {
                for (v, &b) in chunk.iter_mut().zip(bytes) {
                    *v = zigzag(b);
                }
                self.pos += 8;
            } else {
                for v in chunk {
                    *v = self.vari32()?;
                }
            }
        }
        Ok(())
    }

    /// A varint that must fit `0..=max`.
    pub fn varu_max(&mut self, max: u64) -> Result<u64> {
        let v = self.varu()?;
        if v > max {
            return Err(CodecError::Corrupt("value out of range"));
        }
        Ok(v)
    }
}

// ---------------------------------------------------------------- heights --

const H_UNIFORM: u8 = 0;
const H_PREDICTED: u8 = 1;

/// Encode a height plane.
pub fn encode_heights(plane: &Plane<f32>, out: &mut Vec<u8>) {
    if let Plane::Uniform(h) = plane {
        out.push(H_UNIFORM);
        put_vari(out, quantize(*h) as i64);
        return;
    }
    let dense = plane.dense();
    let q0 = quantize(dense[0]);
    if dense.iter().all(|&h| quantize(h) == q0) {
        out.push(H_UNIFORM);
        put_vari(out, q0 as i64);
        return;
    }

    out.push(H_PREDICTED);
    out.reserve(TILE_AREA + TILE_AREA / 4);
    let n = TILE_SIZE as usize;
    // `up` is the previous row (quantized); the predictor is the plane through
    // the three already-coded neighbours: left + up - up_left.
    let mut up = vec![0i32; n];
    for z in 0..n {
        let row = &dense[z * n..(z + 1) * n];
        let mut left = 0i32;
        let mut up_left = 0i32;
        for x in 0..n {
            let q = quantize(row[x]);
            let pred = match (x, z) {
                (0, 0) => 0,
                (_, 0) => left,
                (0, _) => up[0],
                _ => left.wrapping_add(up[x]).wrapping_sub(up_left),
            };
            put_vari(out, q.wrapping_sub(pred) as i64);
            up_left = up[x];
            up[x] = q;
            left = q;
        }
    }
}

/// Decode a height plane written by [`encode_heights`].
pub fn decode_heights(r: &mut Reader) -> Result<Plane<f32>> {
    match r.u8()? {
        H_UNIFORM => {
            let q = r.vari()?;
            Ok(Plane::Uniform(q as f32 / HEIGHT_SCALE))
        }
        H_PREDICTED => {
            // Undoing the predictor `left + up - up_left` of `encode_heights`
            // is a 2D running sum of the residuals: along each row, then
            // down the columns (wrapping, as the encoder computed them).
            let n = TILE_SIZE as usize;
            let mut q = vec![0i32; TILE_AREA];
            r.vari32s(&mut q)?;
            for row in q.chunks_exact_mut(n) {
                let mut sum = 0i32;
                for v in row {
                    sum = sum.wrapping_add(*v);
                    *v = sum;
                }
            }
            for z in 1..n {
                let (done, rest) = q.split_at_mut(z * n);
                for (v, up) in rest[..n].iter_mut().zip(&done[(z - 1) * n..]) {
                    *v = v.wrapping_add(*up);
                }
            }
            Ok(Plane::from_vec(q.into_iter().map(|q| q as f32 / HEIGHT_SCALE).collect()))
        }
        _ => Err(CodecError::Corrupt("unknown height plane mode")),
    }
}

// ------------------------------------------------------------ categorical --

const C_UNIFORM: u8 = 0;
const C_RLE: u8 = 1;
const C_DENSE: u8 = 2;

/// Above this many runs the RLE streams stop paying for themselves.
const MAX_RLE_RUNS: usize = TILE_AREA / 4;

/// A tiny palette builder. Planes rarely hold more than a handful of distinct
/// values, so a linear scan with a last-hit cache beats hashing.
struct Palette {
    values: Vec<u16>,
    last: (u16, u16),
}

impl Palette {
    fn new() -> Self {
        Self {
            values: Vec::with_capacity(8),
            last: (0, u16::MAX),
        }
    }

    #[inline]
    fn index(&mut self, v: u16) -> u16 {
        if self.last.1 != u16::MAX && self.last.0 == v {
            return self.last.1;
        }
        let idx = match self.values.iter().position(|&p| p == v) {
            Some(i) => i as u16,
            None => {
                self.values.push(v);
                (self.values.len() - 1) as u16
            }
        };
        self.last = (v, idx);
        idx
    }
}

fn put_palette(out: &mut Vec<u8>, pal: &[u16]) {
    put_varu(out, pal.len() as u64);
    for &v in pal {
        put_varu(out, v as u64);
    }
}

fn put_indices(out: &mut Vec<u8>, indices: &[u16], wide: bool) {
    if wide {
        for &i in indices {
            out.extend_from_slice(&i.to_le_bytes());
        }
    } else {
        out.extend(indices.iter().map(|&i| i as u8));
    }
}

/// Encode a categorical plane of `u16` values.
pub fn encode_categorical(plane: &Plane<u16>, out: &mut Vec<u8>) {
    if let Some(v) = plane.uniform_value() {
        out.push(C_UNIFORM);
        put_varu(out, v as u64);
        return;
    }

    let mut pal = Palette::new();
    let mut run_values: Vec<u16> = Vec::new();
    let mut run_lens: Vec<u32> = Vec::new();
    plane.for_each_run(|v, len| {
        run_values.push(pal.index(v));
        run_lens.push(len as u32);
    });
    let wide = pal.values.len() > 256;

    if run_values.len() > MAX_RLE_RUNS {
        out.push(C_DENSE);
        put_palette(out, &pal.values);
        let mut indices = Vec::with_capacity(TILE_AREA);
        for (&v, &len) in run_values.iter().zip(&run_lens) {
            indices.extend(std::iter::repeat_n(v, len as usize));
        }
        put_indices(out, &indices, wide);
        return;
    }

    out.push(C_RLE);
    put_palette(out, &pal.values);
    put_varu(out, run_values.len() as u64);
    // Stream A: one palette index per run. Stream B: `length - 1` per run.
    put_indices(out, &run_values, wide);
    for &len in &run_lens {
        put_varu(out, (len - 1) as u64);
    }
}

fn read_palette(r: &mut Reader) -> Result<Vec<u16>> {
    let n = r.varu_max(u16::MAX as u64 + 1)? as usize;
    if n == 0 {
        return Err(CodecError::Corrupt("empty palette"));
    }
    let mut pal = Vec::with_capacity(n);
    for _ in 0..n {
        pal.push(r.varu_max(u16::MAX as u64)? as u16);
    }
    Ok(pal)
}

#[inline]
fn read_index(r: &mut Reader, wide: bool, pal: &[u16]) -> Result<u16> {
    let i = if wide {
        let b = r.bytes(2)?;
        u16::from_le_bytes([b[0], b[1]]) as usize
    } else {
        r.u8()? as usize
    };
    pal.get(i)
        .copied()
        .ok_or(CodecError::Corrupt("palette index out of range"))
}

/// Decode a categorical plane written by [`encode_categorical`].
pub fn decode_categorical(r: &mut Reader) -> Result<Plane<u16>> {
    match r.u8()? {
        C_UNIFORM => Ok(Plane::Uniform(r.varu_max(u16::MAX as u64)? as u16)),
        C_RLE => {
            let pal = read_palette(r)?;
            let wide = pal.len() > 256;
            let runs = r.varu_max(TILE_AREA as u64)? as usize;
            if runs == 0 {
                return Err(CodecError::Corrupt("RLE plane without runs"));
            }
            let mut values = Vec::with_capacity(runs);
            for _ in 0..runs {
                values.push(read_index(r, wide, &pal)?);
            }
            let mut ends = Vec::with_capacity(runs);
            let mut end = 0usize;
            for _ in 0..runs {
                end += r.varu_max(TILE_AREA as u64)? as usize + 1;
                if end > TILE_AREA {
                    return Err(CodecError::Corrupt("RLE runs overflow the tile"));
                }
                ends.push(end as u16);
            }
            if end != TILE_AREA {
                return Err(CodecError::Corrupt("RLE runs do not cover the tile"));
            }
            Ok(Plane::from_runs(values, ends))
        }
        C_DENSE => {
            let pal = read_palette(r)?;
            let wide = pal.len() > 256;
            let width = if wide { 2 } else { 1 };
            // Read at once when the whole plane is there (a short one fails
            // index by index, as it always has).
            if r.remaining() >= TILE_AREA * width {
                let raw = r.bytes(TILE_AREA * width)?;
                let out: Vec<u16> = if wide {
                    raw.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b)).collect()
                } else {
                    raw.iter().map(|&b| b as u16).collect()
                };
                if out.iter().any(|&i| i as usize >= pal.len()) {
                    return Err(CodecError::Corrupt("palette index out of range"));
                }
                return Ok(Plane::from_vec(out.into_iter().map(|i| pal[i as usize]).collect()));
            }
            let mut out = Vec::with_capacity(TILE_AREA);
            for _ in 0..TILE_AREA {
                out.push(read_index(r, wide, &pal)?);
            }
            Ok(Plane::from_vec(out))
        }
        _ => Err(CodecError::Corrupt("unknown categorical plane mode")),
    }
}

// ------------------------------------------------------------------- zstd --

/// Compression level of tile blobs: level 3 is the knee of the curve for the
/// small, already-structured frames we produce.
pub const ZSTD_LEVEL: i32 = 3;

/// Upper bound accepted for a decompressed tile. A legitimate tile is at most
/// a few hundred kilobytes; this only guards against corrupt size fields.
const MAX_RAW_TILE: usize = 16 << 20;

thread_local! {
    static CCTX: std::cell::RefCell<Option<zstd::bulk::Compressor<'static>>> =
        const { std::cell::RefCell::new(None) };
    static DCTX: std::cell::RefCell<Option<zstd::bulk::Decompressor<'static>>> =
        const { std::cell::RefCell::new(None) };
}

/// Compress one frame, reusing a per-thread compression context.
pub fn zstd_compress(raw: &[u8]) -> Vec<u8> {
    CCTX.with(|c| {
        let mut c = c.borrow_mut();
        if c.is_none() {
            *c = zstd::bulk::Compressor::new(ZSTD_LEVEL).ok();
        }
        match c.as_mut() {
            Some(cctx) => cctx.compress(raw),
            None => zstd::bulk::compress(raw, ZSTD_LEVEL),
        }
        .expect("zstd compression into a growable buffer cannot fail")
    })
}

/// Upper bound accepted for a decompressed section of a file: the sculpted
/// chunks of a dimension, a selection, the metadata (with every object of
/// the map's layers) or the save points' list. Each grew past
/// [`MAX_RAW_TILE`] on real maps; this bound only stops a hostile frame.
pub const MAX_RAW_SECTION: usize = 1 << 30;

/// Decompress a section of a file, up to [`MAX_RAW_SECTION`] bytes. Memory
/// grows with what the frame really holds, whatever size it declares.
pub fn zstd_decompress_section(frame: &[u8]) -> Result<Vec<u8>> {
    if let Some(size) = zstd::bulk::Decompressor::upper_bound(frame)
        && size <= MAX_RAW_TILE
    {
        return zstd_decompress(frame);
    }
    use std::io::Read;
    let decoder = zstd::stream::read::Decoder::with_buffer(frame).map_err(CodecError::Zstd)?;
    let mut out = Vec::new();
    decoder.take(MAX_RAW_SECTION as u64 + 1).read_to_end(&mut out).map_err(CodecError::Zstd)?;
    if out.len() > MAX_RAW_SECTION {
        return Err(CodecError::Corrupt("zstd frame is implausibly large"));
    }
    Ok(out)
}

/// The size a frame says it decompresses to, if it says.
pub fn zstd_content_size(frame: &[u8]) -> Option<usize> {
    zstd::bulk::Decompressor::upper_bound(frame)
}

/// Decompress one frame, reusing a per-thread decompression context.
pub fn zstd_decompress(frame: &[u8]) -> Result<Vec<u8>> {
    let size = zstd::bulk::Decompressor::upper_bound(frame)
        .ok_or(CodecError::Corrupt("zstd frame without a content size"))?;
    if size > MAX_RAW_TILE {
        return Err(CodecError::Corrupt("zstd frame is implausibly large"));
    }
    DCTX.with(|d| {
        let mut d = d.borrow_mut();
        if d.is_none() {
            *d = zstd::bulk::Decompressor::new().ok();
        }
        match d.as_mut() {
            Some(dctx) => dctx.decompress(frame, size),
            None => zstd::bulk::decompress(frame, size),
        }
        .map_err(CodecError::Zstd)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lcg(state: &mut u64) -> u32 {
        *state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (*state >> 33) as u32
    }

    #[test]
    fn varints_round_trip() {
        let mut buf = Vec::new();
        let us = [0u64, 1, 127, 128, 300, 16384, u32::MAX as u64, u64::MAX];
        let is = [0i64, -1, 1, -64, 64, -65, i32::MIN as i64, i64::MAX, i64::MIN];
        for &u in &us {
            put_varu(&mut buf, u);
        }
        for &i in &is {
            put_vari(&mut buf, i);
        }
        let mut r = Reader::new(&buf);
        for &u in &us {
            assert_eq!(r.varu().unwrap(), u);
        }
        for &i in &is {
            assert_eq!(r.vari().unwrap(), i);
        }
        assert!(r.is_empty());
        assert!(matches!(r.u8(), Err(CodecError::Eof)));
    }

    #[test]
    fn heights_round_trip_exactly_on_the_grid() {
        let mut s = 7u64;
        let mut v = vec![0f32; TILE_AREA];
        for (i, h) in v.iter_mut().enumerate() {
            let x = (i % 128) as f32;
            let z = (i / 128) as f32;
            let noise = (lcg(&mut s) % 64) as f32 / 256.0;
            *h = snap_height(70.0 + 12.0 * (x * 0.07).sin() + 9.0 * (z * 0.05).cos() + noise);
        }
        v[5000] = -64.0;
        v[5001] = 320.0 - 1.0 / 256.0;
        let plane = Plane::from_vec(v.clone());
        let mut buf = Vec::new();
        encode_heights(&plane, &mut buf);
        // Smooth terrain must land well under two bytes per column.
        assert!(buf.len() < TILE_AREA * 2, "{} bytes", buf.len());
        let back = decode_heights(&mut Reader::new(&buf)).unwrap();
        assert_eq!(&back.dense()[..], &v[..]);
    }

    /// The height decoder as first written: one column after the other,
    /// predicting each from its three decoded neighbours.
    fn walk_heights(r: &mut Reader) -> Result<Vec<f32>> {
        assert_eq!(r.u8()?, H_PREDICTED);
        let n = TILE_SIZE as usize;
        let mut out = vec![0f32; TILE_AREA];
        let mut up = vec![0i32; n];
        for z in 0..n {
            let (mut left, mut up_left) = (0i32, 0i32);
            for x in 0..n {
                let pred = match (x, z) {
                    (0, 0) => 0,
                    (_, 0) => left,
                    (0, _) => up[0],
                    _ => left.wrapping_add(up[x]).wrapping_sub(up_left),
                };
                let q = pred.wrapping_add(r.vari()? as i32);
                out[z * n + x] = q as f32 / HEIGHT_SCALE;
                up_left = up[x];
                up[x] = q;
                left = q;
            }
        }
        Ok(out)
    }

    #[test]
    fn heights_decode_as_the_column_walk_does() {
        let mut s = 11u64;
        // Residuals of every length, small ones mostly, extreme ones that
        // wrap, varints longer than 32 bits; then streams cut short and
        // plain noise, which both must refuse or read alike.
        let mut streams = Vec::new();
        for spread in [3u32, 200, 40_000, u32::MAX] {
            let mut buf = vec![H_PREDICTED];
            for _ in 0..TILE_AREA {
                let v = match lcg(&mut s) % 100 {
                    0 => i64::MIN + lcg(&mut s) as i64,
                    1 => i32::MAX as i64 + lcg(&mut s) as i64,
                    _ => (lcg(&mut s) % spread.max(1)) as i64 - (spread / 2) as i64,
                };
                put_vari(&mut buf, v);
            }
            streams.push(buf.clone());
            for cut in [1, 2, 9, buf.len() / 2] {
                streams.push(buf[..buf.len() - cut].to_vec());
            }
        }
        for _ in 0..4 {
            let mut noise = vec![H_PREDICTED];
            noise.extend((0..TILE_AREA * 2).map(|_| lcg(&mut s) as u8));
            streams.push(noise);
        }
        for buf in &streams {
            let (mut a, mut b) = (Reader::new(buf), Reader::new(buf));
            match (decode_heights(&mut a), walk_heights(&mut b)) {
                (Ok(p), Ok(v)) => {
                    let bits = |h: &[f32]| h.iter().map(|h| h.to_bits()).collect::<Vec<u32>>();
                    assert_eq!(bits(&p.dense()[..]), bits(&v));
                    assert_eq!(a.remaining(), b.remaining());
                }
                (Err(_), Err(_)) => {}
                (p, v) => panic!("the decoders disagree: {:?} against {:?}", p.map(|_| ()), v.map(|_| ())),
            }
        }
    }

    #[test]
    fn uniform_heights_take_a_few_bytes() {
        let mut buf = Vec::new();
        encode_heights(&Plane::Uniform(63.0), &mut buf);
        assert!(buf.len() <= 4);
        let back = decode_heights(&mut Reader::new(&buf)).unwrap();
        assert_eq!(back.uniform_value(), Some(63.0));

        // A dense plane that happens to be flat collapses too.
        let mut buf2 = Vec::new();
        encode_heights(&Plane::from_vec(vec![63.0; TILE_AREA]), &mut buf2);
        assert_eq!(buf, buf2);
    }

    #[test]
    fn categorical_round_trip_all_modes() {
        // Uniform.
        let mut buf = Vec::new();
        encode_categorical(&Plane::Uniform(9), &mut buf);
        assert_eq!(buf, [C_UNIFORM, 9]);
        let p = decode_categorical(&mut Reader::new(&buf)).unwrap();
        assert_eq!(p.uniform_value(), Some(9));

        // A few large runs -> RLE.
        let mut v = vec![1u16; TILE_AREA];
        v[100..9000].fill(700);
        v[9000..9001].fill(3);
        let mut buf = Vec::new();
        encode_categorical(&Plane::from_vec(v.clone()), &mut buf);
        assert_eq!(buf[0], C_RLE);
        assert!(buf.len() < 32);
        let p = decode_categorical(&mut Reader::new(&buf)).unwrap();
        assert_eq!(&p.dense()[..], &v[..]);

        // Noise -> dense fallback.
        let mut s = 3u64;
        let v: Vec<u16> = (0..TILE_AREA).map(|_| (lcg(&mut s) % 5) as u16).collect();
        let mut buf = Vec::new();
        encode_categorical(&Plane::from_vec(v.clone()), &mut buf);
        assert_eq!(buf[0], C_DENSE);
        let p = decode_categorical(&mut Reader::new(&buf)).unwrap();
        assert_eq!(&p.dense()[..], &v[..]);

        // More than 256 distinct values -> wide indices.
        let v: Vec<u16> = (0..TILE_AREA).map(|i| (i / 40) as u16).collect();
        let mut buf = Vec::new();
        encode_categorical(&Plane::from_vec(v.clone()), &mut buf);
        let p = decode_categorical(&mut Reader::new(&buf)).unwrap();
        assert_eq!(&p.dense()[..], &v[..]);
    }

    #[test]
    fn corrupt_planes_are_rejected() {
        assert!(decode_categorical(&mut Reader::new(&[])).is_err());
        assert!(decode_categorical(&mut Reader::new(&[7])).is_err());
        // RLE whose runs stop short of the tile.
        let bad = [C_RLE, 1, 0, 1, 0, 5];
        assert!(decode_categorical(&mut Reader::new(&bad)).is_err());
        // Palette index out of range.
        let bad = [C_RLE, 1, 0, 1, 9, 0xff, 0x7f];
        assert!(decode_categorical(&mut Reader::new(&bad)).is_err());
        assert!(decode_heights(&mut Reader::new(&[H_PREDICTED, 0, 0])).is_err());
    }

    #[test]
    fn zstd_round_trip() {
        let raw: Vec<u8> = (0..10_000u32).map(|i| (i % 7) as u8).collect();
        let z = zstd_compress(&raw);
        assert!(z.len() < raw.len() / 10);
        assert_eq!(zstd_decompress(&z).unwrap(), raw);
        assert!(zstd_decompress(&[1, 2, 3]).is_err());
    }
}
