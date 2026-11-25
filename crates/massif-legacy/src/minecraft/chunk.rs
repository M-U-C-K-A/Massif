//! A chunk's blocks and biomes, read from its NBT in every shape the game
//! has written it:
//!
//! - 1.18 and later: `sections`, each with `block_states` and `biomes`
//!   palettes (from 26.3 a block's keys are `id` and `properties`);
//! - 1.13 to 1.17: `Level.Sections` with `Palette` and `BlockStates` (whose
//!   values straddle the longs before 1.16), biomes in `Level.Biomes` (256
//!   numbers before 1.15, 1024 cells of 4×4×4 after);
//! - before 1.13: `Level.Sections` with `Blocks`, `Data` and `Add`, biomes
//!   as 256 bytes.

use fastnbt::Value;
use serde::Deserialize;
use std::collections::{BTreeMap, HashMap};

/// The blocks of a 16³ section, as indices into [`Decoded::states`], in the
/// game's order: `(y * 16 + z) * 16 + x`.
pub enum Blocks {
    Uniform(u16),
    Mixed(Box<[u16; 4096]>),
}

impl Blocks {
    #[inline]
    pub fn get(&self, i: usize) -> u16 {
        match self {
            Blocks::Uniform(b) => *b,
            Blocks::Mixed(b) => b[i],
        }
    }
}

/// A chunk's biomes, as indices into [`Decoded::biomes`].
pub enum Biomes {
    None,
    /// 4×4×4 cells per section (by section y): `(y * 4 + z) * 4 + x`.
    Cells(Vec<(i32, [u16; 64])>),
    /// One per column, before 1.15: `z * 16 + x`.
    Columns(Box<[u16; 256]>),
}

pub struct Decoded {
    pub x: i32,
    pub z: i32,
    pub data_version: i32,
    /// Every block state of the chunk, its properties sorted
    /// (`minecraft:oak_stairs[facing=east,half=bottom,…]`).
    pub states: Vec<String>,
    /// Sections by their y (in sections), bottom up.
    pub sections: Vec<(i32, Blocks)>,
    pub biomes: Vec<String>,
    pub biome_cells: Biomes,
}

impl Decoded {
    /// The block at chunk column `(x, z)` and height `y`, `None` in a
    /// section the chunk does not have (air).
    #[inline]
    pub fn section(&self, sy: i32) -> Option<&Blocks> {
        self.sections.iter().find(|s| s.0 == sy).map(|s| &s.1)
    }

    /// The biome at `(x, y, z)` (x, z within the chunk).
    pub fn biome_at(&self, x: usize, y: i32, z: usize) -> Option<&str> {
        let i = match &self.biome_cells {
            Biomes::None => return None,
            Biomes::Columns(c) => c[z * 16 + x],
            Biomes::Cells(cells) => {
                let sy = y.div_euclid(16);
                let cell = cells.iter().find(|c| c.0 == sy).or_else(|| if sy < cells.first()?.0 { cells.first() } else { cells.last() })?;
                let ly = if cell.0 == sy { (y.rem_euclid(16) / 4) as usize } else if sy < cell.0 { 0 } else { 3 };
                cell.1[(ly * 4 + z / 4) * 4 + x / 4]
            }
        };
        self.biomes.get(i as usize).map(String::as_str)
    }
}

#[derive(Deserialize)]
#[serde(untagged)]
enum StateEntry {
    Full {
        #[serde(rename = "Name", alias = "id")]
        name: String,
        #[serde(rename = "Properties", alias = "properties", default)]
        props: Option<BTreeMap<String, Value>>,
    },
    Short(String),
}

impl StateEntry {
    fn state(&self) -> String {
        match self {
            StateEntry::Short(s) => massif_core::objects::canonical(s),
            StateEntry::Full { name, props } => {
                let name = if name.contains(':') { name.clone() } else { format!("minecraft:{name}") };
                match props {
                    Some(p) if !p.is_empty() => {
                        let kv: Vec<String> = p
                            .iter()
                            .map(|(k, v)| {
                                let v = match v {
                                    Value::String(s) => s.clone(),
                                    Value::Byte(b) => (*b != 0).to_string(),
                                    Value::Int(i) => i.to_string(),
                                    other => format!("{other:?}"),
                                };
                                format!("{k}={v}")
                            })
                            .collect();
                        format!("{name}[{}]", kv.join(","))
                    }
                    _ => name,
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct Container<T> {
    #[serde(default = "Vec::new")]
    palette: Vec<T>,
    #[serde(default)]
    data: Option<fastnbt::LongArray>,
}

#[derive(Deserialize)]
struct ModernSection {
    #[serde(rename = "Y")]
    y: i8,
    #[serde(default)]
    block_states: Option<Container<StateEntry>>,
    #[serde(default)]
    biomes: Option<Container<String>>,
}

#[derive(Deserialize)]
struct LevelSection {
    #[serde(rename = "Y")]
    y: i8,
    #[serde(rename = "Palette", default)]
    palette: Option<Vec<StateEntry>>,
    #[serde(rename = "BlockStates", default)]
    block_states: Option<fastnbt::LongArray>,
    #[serde(rename = "Blocks", default)]
    blocks: Option<fastnbt::ByteArray>,
    #[serde(rename = "Data", default)]
    data: Option<fastnbt::ByteArray>,
    #[serde(rename = "Add", default)]
    add: Option<fastnbt::ByteArray>,
}

#[derive(Deserialize)]
struct Level {
    #[serde(rename = "xPos")]
    x: i32,
    #[serde(rename = "zPos")]
    z: i32,
    #[serde(rename = "Status", default)]
    status: Option<String>,
    #[serde(rename = "Sections", default)]
    sections: Option<Vec<LevelSection>>,
    #[serde(rename = "Biomes", default)]
    biomes: Option<Value>,
}

#[derive(Deserialize)]
struct Root {
    #[serde(rename = "DataVersion", default)]
    data_version: Option<i32>,
    #[serde(rename = "xPos", default)]
    x: Option<i32>,
    #[serde(rename = "zPos", default)]
    z: Option<i32>,
    #[serde(rename = "Status", default)]
    status: Option<String>,
    #[serde(default)]
    sections: Option<Vec<ModernSection>>,
    #[serde(rename = "Level", default)]
    level: Option<Level>,
}

/// Whether a chunk of this status holds its land, its surface and what
/// grows on it (from `features` on); an unknown status counts as done.
pub fn finished(status: &str) -> bool {
    let s = status.strip_prefix("minecraft:").unwrap_or(status);
    !matches!(s, "empty" | "structure_starts" | "structure_references" | "biomes" | "noise" | "surface" | "carvers" | "liquid_carvers" | "carved" | "base")
}

/// What went wrong reading a chunk.
#[derive(Debug)]
pub enum ChunkError {
    /// Not generated far enough to hold its surface.
    Unfinished,
    Damaged(String),
}

/// Unpack `count` values of `bits` each from `longs`: straddling the longs
/// (before 1.16) or not, as their number says.
fn unpack(longs: &[i64], bits: usize, count: usize, out: &mut [u16]) {
    if bits == 0 {
        out[..count].fill(0);
        return;
    }
    let per = 64 / bits;
    let mask = (1u64 << bits) - 1;
    let padded = count.div_ceil(per);
    if longs.len() >= padded {
        for (i, o) in out[..count].iter_mut().enumerate() {
            let l = longs[i / per] as u64;
            *o = ((l >> ((i % per) * bits)) & mask) as u16;
        }
    } else {
        for (i, o) in out[..count].iter_mut().enumerate() {
            let bit = i * bits;
            let (w, off) = (bit / 64, bit % 64);
            let mut v = (longs.get(w).copied().unwrap_or(0) as u64) >> off;
            if off + bits > 64 {
                v |= (longs.get(w + 1).copied().unwrap_or(0) as u64) << (64 - off);
            }
            *o = (v & mask) as u16;
        }
    }
}

fn bits_for(len: usize, least: usize) -> usize {
    if len <= 1 {
        return 0;
    }
    ((usize::BITS - (len - 1).leading_zeros()) as usize).max(least)
}

/// The states of a section's palette made indices into `states`.
fn palette_indices(palette: &[StateEntry], states: &mut Vec<String>, index: &mut HashMap<String, u16>) -> Vec<u16> {
    palette
        .iter()
        .map(|p| {
            let s = p.state();
            *index.entry(s.clone()).or_insert_with(|| {
                states.push(s);
                (states.len() - 1) as u16
            })
        })
        .collect()
}

fn section_blocks(palette: &[u16], data: Option<&[i64]>, least_bits: usize) -> Blocks {
    match (palette.len(), data) {
        (0, _) => Blocks::Uniform(u16::MAX),
        (1, _) | (_, None) => Blocks::Uniform(palette[0]),
        (n, Some(longs)) => {
            let mut raw = Box::new([0u16; 4096]);
            unpack(longs, bits_for(n, least_bits), 4096, &mut raw[..]);
            for v in raw.iter_mut() {
                *v = palette.get(*v as usize).copied().unwrap_or(palette[0]);
            }
            Blocks::Mixed(raw)
        }
    }
}

/// Read a chunk's NBT. `legacy` turns a block number and its data (before
/// 1.13) into a state.
pub fn decode(nbt: &[u8], legacy: &dyn Fn(u16, u8) -> Option<String>, biome_name: &dyn Fn(i32) -> String) -> Result<Decoded, ChunkError> {
    let root: Root = fastnbt::from_bytes(nbt).map_err(|e| ChunkError::Damaged(e.to_string()))?;
    let data_version = root.data_version.unwrap_or(0);
    let mut states: Vec<String> = Vec::new();
    let mut index: HashMap<String, u16> = HashMap::new();
    let mut biomes: Vec<String> = Vec::new();
    let mut sections = Vec::new();
    if let Some(level) = root.level {
        if let Some(status) = &level.status
            && !finished(status)
        {
            return Err(ChunkError::Unfinished);
        }
        let mut numbered: HashMap<u16, u16> = HashMap::new();
        for s in level.sections.unwrap_or_default() {
            if let (Some(palette), Some(data)) = (&s.palette, &s.block_states) {
                let p = palette_indices(palette, &mut states, &mut index);
                sections.push((s.y as i32, section_blocks(&p, Some(data), 4)));
            } else if let Some(blocks) = &s.blocks {
                let nib = |a: &[i8], i: usize| ((a.get(i / 2).copied().unwrap_or(0) as u8) >> ((i & 1) * 4)) & 15;
                let mut raw = Box::new([0u16; 4096]);
                for (i, r) in raw.iter_mut().enumerate() {
                    let mut id = blocks.get(i).copied().unwrap_or(0) as u8 as u16;
                    if let Some(a) = &s.add {
                        id |= (nib(a, i) as u16) << 8;
                    }
                    let d = s.data.as_ref().map_or(0, |d| nib(d, i));
                    let key = id << 4 | d as u16;
                    *r = *numbered.entry(key).or_insert_with(|| {
                        let s = legacy(id, d).map(|s| massif_core::objects::canonical(&s)).unwrap_or_else(|| if id == 0 { "minecraft:air".into() } else { format!("minecraft:legacy_{id}_{d}") });
                        *index.entry(s.clone()).or_insert_with(|| {
                            states.push(s);
                            (states.len() - 1) as u16
                        })
                    });
                }
                sections.push((s.y as i32, Blocks::Mixed(raw)));
            }
        }
        // The upper half of every tall plant was 175:8: the plant of the
        // half under it.
        sections.sort_by_key(|s| s.0);
        let uppers: Vec<u16> = numbered.iter().filter(|(k, _)| **k >> 4 == 175 && **k & 8 != 0).map(|(_, v)| *v).collect();
        if !uppers.is_empty() {
            let mut below_top = [u16::MAX; 256];
            for (_, blocks) in sections.iter_mut() {
                let Blocks::Mixed(raw) = blocks else { continue };
                for i in 0..4096 {
                    if uppers.contains(&raw[i]) {
                        let under = if i >= 256 { raw[i - 256] } else { below_top[i] };
                        if under != u16::MAX && let Some(name) = states.get(under as usize).filter(|s| s.contains("half=lower")).cloned() {
                            let s = name.replace("half=lower", "half=upper");
                            raw[i] = *index.entry(s.clone()).or_insert_with(|| {
                                states.push(s);
                                (states.len() - 1) as u16
                            });
                        }
                    }
                }
                below_top.copy_from_slice(&raw[4096 - 256..]);
            }
        }
        let cells = match level.biomes {
            Some(Value::IntArray(a)) if a.len() >= 1024 => {
                let mut by_number: HashMap<i32, u16> = HashMap::new();
                let mut name = |n: i32| {
                    *by_number.entry(n).or_insert_with(|| {
                        biomes.push(biome_name(n));
                        (biomes.len() - 1) as u16
                    })
                };
                // 4×4×4 cells, 64 high: the 256 heights in 16 sections.
                let mut out = Vec::new();
                let layers = a.len() / 16;
                for sy in 0..(layers / 4) as i32 {
                    let mut c = [0u16; 64];
                    for (k, v) in c.iter_mut().enumerate() {
                        *v = name(a[sy as usize * 64 + k]);
                    }
                    out.push((sy, c));
                }
                Biomes::Cells(out)
            }
            Some(Value::IntArray(a)) if a.len() == 256 => {
                let mut c = Box::new([0u16; 256]);
                let mut by_number: HashMap<i32, u16> = HashMap::new();
                for (k, v) in c.iter_mut().enumerate() {
                    *v = *by_number.entry(a[k]).or_insert_with(|| {
                        biomes.push(biome_name(a[k]));
                        (biomes.len() - 1) as u16
                    });
                }
                Biomes::Columns(c)
            }
            Some(Value::ByteArray(a)) if a.len() == 256 => {
                let mut c = Box::new([0u16; 256]);
                let mut by_number: HashMap<i32, u16> = HashMap::new();
                for (k, v) in c.iter_mut().enumerate() {
                    let n = a[k] as u8 as i32;
                    *v = *by_number.entry(n).or_insert_with(|| {
                        biomes.push(biome_name(n));
                        (biomes.len() - 1) as u16
                    });
                }
                Biomes::Columns(c)
            }
            _ => Biomes::None,
        };
        sections.sort_by_key(|s| s.0);
        return Ok(Decoded { x: level.x, z: level.z, data_version, states, sections, biomes, biome_cells: cells });
    }
    if let Some(status) = &root.status
        && !finished(status)
    {
        return Err(ChunkError::Unfinished);
    }
    let (Some(x), Some(z)) = (root.x, root.z) else { return Err(ChunkError::Damaged("no position".into())) };
    let mut cells = Vec::new();
    let mut biome_index: HashMap<String, u16> = HashMap::new();
    for s in root.sections.unwrap_or_default() {
        if let Some(b) = &s.block_states
            && !b.palette.is_empty()
        {
            let p = palette_indices(&b.palette, &mut states, &mut index);
            sections.push((s.y as i32, section_blocks(&p, b.data.as_deref(), 4)));
        }
        if let Some(b) = &s.biomes
            && !b.palette.is_empty()
        {
            let p: Vec<u16> = b
                .palette
                .iter()
                .map(|n| {
                    *biome_index.entry(n.clone()).or_insert_with(|| {
                        biomes.push(n.clone());
                        (biomes.len() - 1) as u16
                    })
                })
                .collect();
            let mut c = [p[0]; 64];
            if p.len() > 1
                && let Some(data) = &b.data
            {
                let mut raw = [0u16; 64];
                unpack(data, bits_for(p.len(), 1), 64, &mut raw);
                for (v, r) in c.iter_mut().zip(raw) {
                    *v = p.get(r as usize).copied().unwrap_or(p[0]);
                }
            }
            cells.push((s.y as i32, c));
        }
    }
    sections.sort_by_key(|s| s.0);
    cells.sort_by_key(|c| c.0);
    let biome_cells = if cells.is_empty() { Biomes::None } else { Biomes::Cells(cells) };
    Ok(Decoded { x, z, data_version, states, sections, biomes, biome_cells })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unpacks_both_ways() {
        // 5 bits: 12 per long without straddling.
        let values: Vec<u16> = (0..4096).map(|i| (i * 7 % 32) as u16).collect();
        let mut longs = vec![0i64; 4096usize.div_ceil(12)];
        for (i, v) in values.iter().enumerate() {
            longs[i / 12] |= (*v as i64) << ((i % 12) * 5);
        }
        let mut out = [0u16; 4096];
        unpack(&longs, 5, 4096, &mut out);
        assert_eq!(&out[..], &values[..]);
        // Straddling: 4096 * 5 / 64 = 320 longs.
        let mut longs = vec![0u64; 320];
        for (i, v) in values.iter().enumerate() {
            let bit = i * 5;
            longs[bit / 64] |= (*v as u64) << (bit % 64);
            if bit % 64 + 5 > 64 {
                longs[bit / 64 + 1] |= (*v as u64) >> (64 - bit % 64);
            }
        }
        let longs: Vec<i64> = longs.into_iter().map(|l| l as i64).collect();
        unpack(&longs, 5, 4096, &mut out);
        assert_eq!(&out[..], &values[..]);
    }
}
