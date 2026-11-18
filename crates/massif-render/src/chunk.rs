//! Minecraft chunks as the renderer reads them: their sections of 16³
//! blocks, each block the number of its material (see [`crate::materials`]).
//!
//! The open map and a world's region files give the same thing: the open
//! map's chunks are built by the exporter itself (so the image shows what
//! the game would get), and both are read from their NBT here.

use crate::materials::{AIR, Materials};
use serde::Deserialize;
use std::borrow::Cow;
use std::collections::BTreeMap;

/// Blocks in a section.
pub const VOLUME: usize = 4096;

/// The blocks of a section, in the game's order: `(y * 16 + z) * 16 + x`.
#[derive(Debug, Clone, PartialEq)]
pub enum Section {
    /// One material throughout (air, often).
    Uniform(u16),
    Mixed(Box<[u16; VOLUME]>),
}

impl Section {
    pub const EMPTY: Section = Section::Uniform(AIR);

    pub fn get(&self, x: usize, y: usize, z: usize) -> u16 {
        match self {
            Section::Uniform(m) => *m,
            Section::Mixed(b) => b[(y * 16 + z) * 16 + x],
        }
    }
}

/// The sections of a chunk, from the bottom of the scene up.
#[derive(Debug, Clone, PartialEq)]
pub struct Chunk {
    pub x: i32,
    pub z: i32,
    pub sections: Vec<Section>,
}

#[derive(Deserialize)]
struct NbtChunk<'a> {
    #[serde(rename = "xPos")]
    x: i32,
    #[serde(rename = "zPos")]
    z: i32,
    #[serde(borrow, default)]
    sections: Vec<NbtSection<'a>>,
}

#[derive(Deserialize)]
struct NbtSection<'a> {
    #[serde(rename = "Y")]
    y: i8,
    #[serde(borrow)]
    block_states: Option<BlockStates<'a>>,
}

#[derive(Deserialize)]
struct BlockStates<'a> {
    #[serde(borrow)]
    palette: Vec<PaletteEntry<'a>>,
    #[serde(borrow)]
    data: Option<fastnbt::borrow::LongArray<'a>>,
}

#[derive(Deserialize)]
struct PaletteEntry<'a> {
    #[serde(rename = "Name", borrow)]
    name: Cow<'a, str>,
    #[serde(rename = "Properties", default)]
    properties: BTreeMap<String, String>,
}

/// A chunk of 1.18 or later (the exporter's own, or a region file's),
/// with the sections from `min_y` (a multiple of 16) up, `sections` of
/// them. `None`: not a chunk of that kind.
pub fn decode(nbt: &[u8], materials: &Materials, min_y: i32, sections: usize) -> Option<Chunk> {
    let chunk: NbtChunk = fastnbt::from_bytes(nbt).ok()?;
    let mut out = Chunk { x: chunk.x, z: chunk.z, sections: vec![Section::EMPTY; sections] };
    let mut ids = Vec::new();
    for section in &chunk.sections {
        let Some(states) = &section.block_states else { continue };
        let k = (section.y as i32 * 16 - min_y).div_euclid(16);
        if k < 0 || k as usize >= sections || states.palette.is_empty() {
            continue;
        }
        ids.clear();
        for entry in &states.palette {
            let props: Vec<(&str, &str)> = entry.properties.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            ids.push(materials.id(&entry.name, &props));
        }
        out.sections[k as usize] = unpack(&ids, states.data.as_ref().map(|d| d.iter()));
    }
    Some(out)
}

/// A section's blocks from its palette (as materials) and its packed
/// indices: at least 4 bits each, never across two longs (1.16 and later).
fn unpack(ids: &[u16], data: Option<impl Iterator<Item = i64>>) -> Section {
    let Some(data) = data.filter(|_| ids.len() > 1) else {
        return Section::Uniform(ids[0]);
    };
    if ids.iter().all(|&m| m == ids[0]) {
        return Section::Uniform(ids[0]);
    }
    let bits = (usize::BITS - (ids.len() - 1).leading_zeros()).max(4) as usize;
    let per = 64 / bits;
    let mask = (1u64 << bits) - 1;
    let mut blocks = Box::new([AIR; VOLUME]);
    let mut k = 0;
    'longs: for long in data {
        let mut v = long as u64;
        for _ in 0..per {
            if k == VOLUME {
                break 'longs;
            }
            blocks[k] = ids.get((v & mask) as usize).copied().unwrap_or(AIR);
            v >>= bits;
            k += 1;
        }
    }
    Section::Mixed(blocks)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn packed_indices_read_back() {
        // 3 states: 4 bits each, 16 to a long.
        let ids = [0u16, 5, 9];
        let mut want = [0u16; VOLUME];
        let mut longs = vec![0i64; VOLUME / 16];
        for k in 0..VOLUME {
            let index = (k * 7 + k / 5) % 3;
            want[k] = ids[index];
            longs[k / 16] |= (index as i64) << ((k % 16) * 4);
        }
        assert_eq!(unpack(&ids, Some(longs.into_iter())), Section::Mixed(Box::new(want)));
        // 33 states: 6 bits, 10 to a long, 4 bits left over in each.
        let ids: Vec<u16> = (0..33).collect();
        let mut longs = vec![0i64; VOLUME.div_ceil(10)];
        for k in 0..VOLUME {
            longs[k / 10] |= ((k % 33) as i64) << ((k % 10) * 6);
        }
        let Section::Mixed(got) = unpack(&ids, Some(longs.into_iter())) else { panic!("mixed") };
        assert!((0..VOLUME).all(|k| got[k] == (k % 33) as u16));
        assert_eq!(unpack(&[3], None::<std::iter::Empty<i64>>), Section::Uniform(3));
    }
}
