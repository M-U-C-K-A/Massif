//! A 128×128 tile of one level, and its compressed ("cold") encoding.
//!
//! Cold layout, before zstd:
//!
//! ```text
//! u8  version (1)
//! repeated:  u8 tag · varint payload_len · payload
//! u8  0      end marker
//! ```
//!
//! Planes equal to their default are simply omitted, and unknown tags are
//! carried through untouched, so newer writers can add planes without
//! breaking older readers.

use crate::codec::{self, CodecError, Reader, Result};
use crate::coords::TILE_AREA;
use crate::plane::Plane;

/// Fluid plane value meaning "no fluid in this column".
pub const NO_FLUID: i16 = i16::MIN;

const VERSION: u8 = 1;
const TAG_END: u8 = 0;
const TAG_TOP: u8 = 1;
const TAG_BOTTOM: u8 = 2;
const TAG_TERRAIN: u8 = 3;
const TAG_FLUID: u8 = 4;
const TAG_BIOME: u8 = 5;

/// One tile of one level.
///
/// Heights are *surface elevations*: a column is solid for every block `y`
/// with `y < round(top)`, so `top = 64.0` means the highest block is at
/// `y = 63` and one stands at `y = 64`. Fluid uses the same convention.
#[derive(Debug, Clone, PartialEq)]
pub struct Tile {
    /// Upper surface of the level.
    pub top: Plane<f32>,
    /// Lower surface. `None` for the ground level, whose slab reaches the
    /// world floor. A column of a slab level exists iff `top > bottom`.
    pub bottom: Option<Plane<f32>>,
    /// Surface material ([`crate::TerrainId`]) per column.
    pub terrain: Plane<u16>,
    /// Elevation of the fluid surface, or [`NO_FLUID`].
    pub fluid: Plane<i16>,
    /// Painted biome ([`crate::BiomeId`]); 0 lets the exporter decide.
    pub biome: Plane<u16>,
    /// Planes written by a newer version, preserved verbatim.
    pub unknown: Vec<(u8, Vec<u8>)>,
}

impl Tile {
    /// A flat ground tile.
    pub fn ground(height: f32, terrain: u16, fluid: i16) -> Self {
        Self {
            top: Plane::Uniform(height),
            bottom: None,
            terrain: Plane::Uniform(terrain),
            fluid: Plane::Uniform(fluid),
            biome: Plane::Uniform(0),
            unknown: Vec::new(),
        }
    }

    /// An empty tile of a slab (solid or carve) level: no column exists yet.
    pub fn empty_slab() -> Self {
        Self {
            top: Plane::Uniform(0.0),
            bottom: Some(Plane::Uniform(0.0)),
            terrain: Plane::Uniform(0),
            fluid: Plane::Uniform(NO_FLUID),
            biome: Plane::Uniform(0),
            unknown: Vec::new(),
        }
    }

    /// Whether the slab exists in column `i` (always true for ground tiles).
    #[inline]
    pub fn present(&self, i: usize) -> bool {
        match &self.bottom {
            None => true,
            Some(b) => self.top.get(i) > b.get(i),
        }
    }

    /// True if no column of a slab tile exists; such tiles can be dropped.
    pub fn is_empty_slab(&self) -> bool {
        let Some(bottom) = &self.bottom else {
            return false;
        };
        match (&self.top, bottom) {
            (Plane::Uniform(t), Plane::Uniform(b)) => t <= b,
            _ => {
                let top = self.top.dense();
                let bot = bottom.dense();
                top.iter().zip(bot.iter()).all(|(t, b)| t <= b)
            }
        }
    }

    /// Re-pack every plane after an edit.
    pub fn compact(&mut self) {
        self.top.compact();
        if let Some(b) = &mut self.bottom {
            b.compact();
        }
        self.terrain.compact();
        self.fluid.compact();
        self.biome.compact();
    }

    pub fn heap_bytes(&self) -> usize {
        self.top.heap_bytes()
            + self.bottom.as_ref().map_or(0, Plane::heap_bytes)
            + self.terrain.heap_bytes()
            + self.fluid.heap_bytes()
            + self.biome.heap_bytes()
    }

    /// Encode to the compressed cold form (a zstd frame).
    pub fn encode(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity(TILE_AREA + 256);
        let mut scratch = Vec::new();
        raw.push(VERSION);

        let mut plane = |tag: u8, fill: &mut dyn FnMut(&mut Vec<u8>)| {
            scratch.clear();
            fill(&mut scratch);
            raw.push(tag);
            codec::put_varu(&mut raw, scratch.len() as u64);
            raw.extend_from_slice(&scratch);
        };

        if self.top.uniform_value() != Some(0.0) {
            plane(TAG_TOP, &mut |o| codec::encode_heights(&self.top, o));
        }
        if let Some(bottom) = &self.bottom {
            plane(TAG_BOTTOM, &mut |o| codec::encode_heights(bottom, o));
        }
        if self.terrain.uniform_value() != Some(0) {
            plane(TAG_TERRAIN, &mut |o| {
                codec::encode_categorical(&self.terrain, o)
            });
        }
        if self.fluid.uniform_value() != Some(NO_FLUID) {
            let as_u16 = map_plane(&self.fluid, |v| v as u16);
            plane(TAG_FLUID, &mut |o| codec::encode_categorical(&as_u16, o));
        }
        if self.biome.uniform_value() != Some(0) {
            plane(TAG_BIOME, &mut |o| codec::encode_categorical(&self.biome, o));
        }
        for (tag, payload) in &self.unknown {
            plane(*tag, &mut |o| o.extend_from_slice(payload));
        }
        raw.push(TAG_END);
        codec::zstd_compress(&raw)
    }

    /// Decode a cold blob produced by [`Tile::encode`].
    pub fn decode(blob: &[u8]) -> Result<Self> {
        let raw = codec::zstd_decompress(blob)?;
        let mut r = Reader::new(&raw);
        let version = r.u8()?;
        if version != VERSION {
            return Err(CodecError::Version(version));
        }
        let mut tile = Tile {
            top: Plane::Uniform(0.0),
            bottom: None,
            terrain: Plane::Uniform(0),
            fluid: Plane::Uniform(NO_FLUID),
            biome: Plane::Uniform(0),
            unknown: Vec::new(),
        };
        loop {
            let tag = r.u8()?;
            if tag == TAG_END {
                break;
            }
            let len = r.varu()? as usize;
            let payload = r.bytes(len)?;
            let mut p = Reader::new(payload);
            match tag {
                TAG_TOP => tile.top = codec::decode_heights(&mut p)?,
                TAG_BOTTOM => tile.bottom = Some(codec::decode_heights(&mut p)?),
                TAG_TERRAIN => tile.terrain = codec::decode_categorical(&mut p)?,
                TAG_FLUID => {
                    tile.fluid = map_plane(&codec::decode_categorical(&mut p)?, |v| v as i16)
                }
                TAG_BIOME => tile.biome = codec::decode_categorical(&mut p)?,
                _ => tile.unknown.push((tag, payload.to_vec())),
            }
        }
        Ok(tile)
    }
}

/// Convert a plane cell by cell, keeping its representation.
fn map_plane<A: Copy + PartialEq, B: Copy + PartialEq>(
    p: &Plane<A>,
    f: impl Fn(A) -> B,
) -> Plane<B> {
    match p {
        Plane::Uniform(v) => Plane::Uniform(f(*v)),
        Plane::Rle { values, ends } => Plane::Rle {
            values: values.iter().map(|&v| f(v)).collect(),
            ends: ends.clone(),
        },
        Plane::Dense(d) => Plane::from_vec(d.iter().map(|&v| f(v)).collect()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codec::snap_height;

    fn sample_tile() -> Tile {
        let mut t = Tile::ground(64.0, 1, 63);
        let top = t.top.dense_mut();
        for (i, h) in top.iter_mut().enumerate() {
            let x = (i % 128) as f32;
            let z = (i / 128) as f32;
            *h = snap_height(64.0 + 20.0 * (x * 0.05).sin() * (z * 0.04).cos());
        }
        for i in 4000..6000 {
            t.terrain.set(i, 5);
        }
        for i in 0..300 {
            t.fluid.set(i, NO_FLUID);
        }
        t.biome.set(9, 3);
        t
    }

    #[test]
    fn tiles_round_trip() {
        let t = sample_tile();
        let blob = t.encode();
        let back = Tile::decode(&blob).unwrap();
        assert_eq!(back.top.dense(), t.top.dense());
        assert_eq!(back.bottom, None);
        assert_eq!(back.terrain.dense(), t.terrain.dense());
        assert_eq!(back.fluid.dense(), t.fluid.dense());
        assert_eq!(back.biome.dense(), t.biome.dense());
        // Encoding is canonical: re-encoding the decoded tile is identical.
        assert_eq!(back.encode(), blob);
    }

    #[test]
    fn flat_tiles_are_tiny() {
        let blob = Tile::ground(64.0, 1, 63).encode();
        assert!(blob.len() < 40, "{} bytes", blob.len());
        let blob = Tile::empty_slab().encode();
        assert!(blob.len() < 24, "{} bytes", blob.len());
        let back = Tile::decode(&blob).unwrap();
        assert!(back.bottom.is_some());
        assert!(back.is_empty_slab());
    }

    #[test]
    fn slab_presence() {
        let mut t = Tile::empty_slab();
        assert!(!t.present(10));
        t.top.set(10, 90.0);
        t.bottom.as_mut().unwrap().set(10, 80.0);
        assert!(t.present(10));
        assert!(!t.present(11));
        assert!(!t.is_empty_slab());
        let back = Tile::decode(&t.encode()).unwrap();
        assert!(back.present(10) && !back.present(11));
    }

    #[test]
    fn unknown_planes_survive_a_round_trip() {
        let mut t = sample_tile();
        t.unknown.push((200, vec![1, 2, 3, 4]));
        let back = Tile::decode(&t.encode()).unwrap();
        assert_eq!(back.unknown, vec![(200, vec![1, 2, 3, 4])]);
    }

    #[test]
    fn garbage_is_an_error_not_a_panic() {
        assert!(Tile::decode(&[]).is_err());
        assert!(Tile::decode(&[0u8; 64]).is_err());
        let mut blob = sample_tile().encode();
        let n = blob.len();
        blob.truncate(n / 2);
        assert!(Tile::decode(&blob).is_err());
    }
}
