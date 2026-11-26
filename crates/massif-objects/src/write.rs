//! Writing a box of blocks as a schematic: Sponge `.schem` (versions 3
//! and 2, as WorldEdit and FAWE read them) and Litematica `.litematic`.
//!
//! A [`Volume`] holds block states by palette index, x first, then z, then
//! y, as both formats store them; its block entities keep their NBT as the
//! game writes it (an unnamed compound with its `id`). What is written is
//! read back by [`crate::read_bytes`] as the same blocks.

use fastnbt::{ByteArray, IntArray, LongArray, Value};
use std::collections::HashMap;
use std::io::Write;

/// The largest side a schematic may have: both formats keep sizes in 16
/// bits (Sponge as an unsigned short).
pub const MAX_SIDE: u32 = u16::MAX as u32;

/// A box of blocks to write.
#[derive(Debug, Clone, Default)]
pub struct Volume {
    pub name: String,
    pub author: String,
    /// Width (x), height (y) and length (z).
    pub size: [u32; 3],
    /// Block states (`minecraft:oak_log[axis=y]`), any of them air.
    pub palette: Vec<String>,
    /// The palette index of each block, `x + z * width + y * width * length`.
    pub blocks: Vec<u16>,
    /// Where the schematic is pasted from, from the box's lowest corner:
    /// the block a player stands on when pasting it.
    pub origin: [i32; 3],
    /// Block entities, by their place in the box.
    pub block_entities: Vec<BlockEntity>,
    /// The biome of each block, in the order of `blocks`: names and the
    /// index of each block's. Only Sponge's third version keeps them by the
    /// block, its second by the column (the lowest block's).
    pub biomes: Option<(Vec<String>, Vec<u16>)>,
    /// The game's data version the states are of.
    pub data_version: i32,
}

/// A block entity: a chest's contents, a sign's text.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockEntity {
    pub pos: [i32; 3],
    /// Its NBT as a file holds it: an unnamed compound with its `id` and
    /// its data, without its coordinates.
    pub nbt: Vec<u8>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    /// Sponge schematic version 3 (WorldEdit 7.3 and later).
    Sponge3,
    /// Sponge schematic version 2 (WorldEdit 7.0 to 7.2, FAWE).
    Sponge2,
    /// Litematica, schematic version 6.
    Litematic,
}

impl Format {
    pub fn extension(self) -> &'static str {
        match self {
            Format::Sponge3 | Format::Sponge2 => "schem",
            Format::Litematic => "litematic",
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WriteError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the box is too large for a schematic: {0} × {1} × {2}, at most {MAX_SIDE} each way")]
    TooLarge(u32, u32, u32),
    #[error("the box holds no block")]
    Empty,
    #[error("{0}")]
    Nbt(String),
}

impl Volume {
    pub fn volume(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    /// The blocks that are not air (nor cave air, void air or structure
    /// voids).
    pub fn count(&self) -> usize {
        let nothing: Vec<bool> = self.palette.iter().map(|s| massif_core::objects::is_nothing(s)).collect();
        self.blocks.iter().filter(|&&b| !nothing.get(b as usize).copied().unwrap_or(true)).count()
    }

    /// The volume of an object of the map, its anchor the origin.
    pub fn of_object(object: &massif_core::objects::ObjectDef, data_version: i32) -> Self {
        let size = object.size.map(u32::from);
        // Index 0 of an object is "nothing": air in a schematic.
        let mut palette = object.palette.clone();
        if let Some(first) = palette.first_mut() {
            *first = "minecraft:air".to_owned();
        }
        Self {
            name: object.name.clone(),
            author: String::new(),
            size,
            palette,
            blocks: object.blocks.clone(),
            // The anchor goes in the block above the ground: the origin is
            // the block under it.
            origin: [object.anchor[0] as i32, object.anchor[1] as i32 - 1, object.anchor[2] as i32],
            block_entities: Vec::new(),
            biomes: None,
            data_version,
        }
    }

    /// How many of each block it holds, by the block's name (its
    /// properties left out), the most first; air left out.
    pub fn materials(&self) -> Vec<(String, usize)> {
        let mut counts = vec![0usize; self.palette.len()];
        for &b in &self.blocks {
            if let Some(c) = counts.get_mut(b as usize) {
                *c += 1;
            }
        }
        let mut by_name: HashMap<&str, usize> = HashMap::new();
        for (state, &n) in self.palette.iter().zip(&counts) {
            if n > 0 && !massif_core::objects::is_nothing(state) {
                *by_name.entry(state.split('[').next().unwrap_or(state)).or_default() += n;
            }
        }
        let mut out: Vec<(String, usize)> = by_name.into_iter().map(|(k, n)| (k.to_owned(), n)).collect();
        out.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
        out
    }

    fn check(&self) -> Result<(), WriteError> {
        let [w, h, l] = self.size;
        if w > MAX_SIDE || h > MAX_SIDE || l > MAX_SIDE {
            return Err(WriteError::TooLarge(w, h, l));
        }
        if self.volume() == 0 || self.blocks.len() != self.volume() {
            return Err(WriteError::Empty);
        }
        Ok(())
    }
}

/// The schematic's file, gzipped, as `format` writes it.
pub fn write(volume: &Volume, format: Format) -> Result<Vec<u8>, WriteError> {
    volume.check()?;
    let root = match format {
        Format::Sponge3 => sponge(volume, 3),
        Format::Sponge2 => sponge(volume, 2),
        Format::Litematic => litematic(volume),
    };
    let raw = fastnbt::to_bytes(&root).map_err(|e| WriteError::Nbt(e.to_string()))?;
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(raw.len() / 8), flate2::Compression::default());
    gz.write_all(&raw)?;
    Ok(gz.finish()?)
}

/// The palette used, made of the entries `blocks` uses (air first, so that
/// Litematica's empty cells mean air), and each block's index in it.
fn compact(palette: &[String], blocks: &[u16]) -> (Vec<String>, Vec<u16>) {
    let air = "minecraft:air";
    let mut map: Vec<u32> = vec![u32::MAX; palette.len()];
    let mut out = vec![air.to_owned()];
    for (i, s) in palette.iter().enumerate() {
        if s == air {
            map[i] = 0;
        }
    }
    let mut used = vec![false; palette.len()];
    for &b in blocks {
        if let Some(u) = used.get_mut(b as usize) {
            *u = true;
        }
    }
    for (i, s) in palette.iter().enumerate() {
        if used[i] && map[i] == u32::MAX {
            map[i] = out.len() as u32;
            out.push(s.clone());
        }
    }
    let blocks = blocks.iter().map(|&b| map.get(b as usize).copied().filter(|&m| m != u32::MAX).unwrap_or(0) as u16).collect();
    (out, blocks)
}

fn compound(entries: Vec<(&str, Value)>) -> Value {
    Value::Compound(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
}

fn varints(values: impl Iterator<Item = u32>, hint: usize) -> Vec<i8> {
    let mut out = Vec::with_capacity(hint);
    for mut v in values {
        loop {
            let b = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                out.push(b as i8);
                break;
            }
            out.push((b | 0x80) as i8);
        }
    }
    out
}

/// The block entity's NBT as a compound, without the coordinates it may
/// carry.
fn entity_compound(be: &BlockEntity) -> Option<HashMap<String, Value>> {
    let Ok(Value::Compound(mut c)) = fastnbt::from_bytes::<Value>(&be.nbt) else { return None };
    for k in ["x", "y", "z", "keepPacked"] {
        c.remove(k);
    }
    Some(c)
}

fn now_millis() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64)
}

fn sponge(v: &Volume, version: i32) -> Value {
    let [w, h, l] = v.size;
    let (palette, blocks) = compact(&v.palette, &v.blocks);
    let palette_tag = Value::Compound(palette.iter().enumerate().map(|(k, s)| (s.clone(), Value::Int(k as i32))).collect());
    let data = Value::ByteArray(ByteArray::new(varints(blocks.iter().map(|&b| b as u32), blocks.len())));
    // Pasting puts the origin where the player stands: the box's lowest
    // corner lies at the origin plus the offset.
    let offset = [-v.origin[0], -v.origin[1], -v.origin[2]];
    let entities: Vec<Value> = v
        .block_entities
        .iter()
        .filter_map(|be| {
            let mut c = entity_compound(be)?;
            let id = match c.remove("id") {
                Some(Value::String(id)) => id,
                _ => return None,
            };
            let pos = ("Pos", Value::IntArray(IntArray::new(be.pos.to_vec())));
            Some(if version == 3 {
                compound(vec![pos, ("Id", Value::String(id)), ("Data", Value::Compound(c))])
            } else {
                c.insert("Pos".into(), pos.1);
                c.insert("Id".into(), Value::String(id));
                Value::Compound(c)
            })
        })
        .collect();
    let mut meta = vec![("Name", Value::String(v.name.clone())), ("Date", Value::Long(now_millis()))];
    if !v.author.is_empty() {
        meta.push(("Author", Value::String(v.author.clone())));
    }
    let mut body = vec![
        ("Version", Value::Int(version)),
        ("DataVersion", Value::Int(v.data_version)),
        ("Width", Value::Short(w as u16 as i16)),
        ("Height", Value::Short(h as u16 as i16)),
        ("Length", Value::Short(l as u16 as i16)),
    ];
    if version == 3 {
        body.push(("Offset", Value::IntArray(IntArray::new(offset.to_vec()))));
        // WorldEdit's own: the origin in the box's frame, its corner at
        // the origin plus the offset (that is, at zero).
        meta.push(("WorldEdit", compound(vec![("Origin", Value::IntArray(IntArray::new(v.origin.to_vec())))])));
        let mut blocks_tag = vec![("Palette", palette_tag), ("Data", data)];
        if !entities.is_empty() {
            blocks_tag.push(("BlockEntities", Value::List(entities)));
        }
        body.push(("Blocks", compound(blocks_tag)));
        if let Some((names, ids)) = &v.biomes {
            let (names, ids) = compact_biomes(names, ids);
            body.push((
                "Biomes",
                compound(vec![
                    ("Palette", Value::Compound(names.iter().enumerate().map(|(k, s)| (s.clone(), Value::Int(k as i32))).collect())),
                    ("Data", Value::ByteArray(ByteArray::new(varints(ids.iter().copied(), ids.len())))),
                ]),
            ));
        }
        body.push(("Metadata", compound(meta)));
        compound(vec![("Schematic", compound(body))])
    } else {
        // WorldEdit 7.2 reads the box's corner from `Offset` and the
        // origin from these.
        body.push(("Offset", Value::IntArray(IntArray::new(vec![0, 0, 0]))));
        meta.push(("WEOffsetX", Value::Int(offset[0])));
        meta.push(("WEOffsetY", Value::Int(offset[1])));
        meta.push(("WEOffsetZ", Value::Int(offset[2])));
        body.push(("PaletteMax", Value::Int(palette.len() as i32)));
        body.push(("Palette", palette_tag));
        body.push(("BlockData", data));
        body.push(("BlockEntities", Value::List(entities)));
        if let Some((names, ids)) = &v.biomes {
            // By the column: its lowest block's.
            let column: Vec<u16> = ids.iter().take(w as usize * l as usize).copied().collect();
            let (names, column) = compact_biomes(names, &column);
            body.push(("BiomePaletteMax", Value::Int(names.len() as i32)));
            body.push(("BiomePalette", Value::Compound(names.iter().enumerate().map(|(k, s)| (s.clone(), Value::Int(k as i32))).collect())));
            body.push(("BiomeData", Value::ByteArray(ByteArray::new(varints(column.iter().copied(), column.len())))));
        }
        body.push(("Metadata", compound(meta)));
        compound(body)
    }
}

fn compact_biomes(names: &[String], ids: &[u16]) -> (Vec<String>, Vec<u32>) {
    let mut map = vec![u32::MAX; names.len()];
    let mut out: Vec<String> = Vec::new();
    let ids = ids
        .iter()
        .map(|&b| {
            let Some(m) = map.get_mut(b as usize) else { return 0 };
            if *m == u32::MAX {
                *m = out.len() as u32;
                out.push(names[b as usize].clone());
            }
            *m
        })
        .collect();
    if out.is_empty() {
        out.push("minecraft:plains".to_owned());
    }
    (out, ids)
}

/// A state as a palette entry: `Name` and its `Properties`.
fn state_entry(state: &str) -> Value {
    let (name, props) = match state.split_once('[') {
        Some((n, p)) => (n, p.trim_end_matches(']')),
        None => (state, ""),
    };
    let props: HashMap<String, Value> = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.to_owned(), Value::String(v.to_owned()))).collect();
    let mut entry = vec![("Name", Value::String(name.to_owned()))];
    if !props.is_empty() {
        entry.push(("Properties", Value::Compound(props)));
    }
    compound(entry)
}

/// Indices of `bits` each, end to end across longs, as Litematica packs
/// them.
fn pack_spanning(values: &[u16], bits: u32) -> Vec<i64> {
    let total = values.len() as u64 * bits as u64;
    let mut out = vec![0u64; total.div_ceil(64) as usize];
    for (i, &v) in values.iter().enumerate() {
        let bit = i as u64 * bits as u64;
        let (word, shift) = ((bit / 64) as usize, (bit % 64) as u32);
        let v = v as u64;
        out[word] |= v << shift;
        if shift + bits > 64 {
            out[word + 1] |= v >> (64 - shift);
        }
    }
    out.into_iter().map(|v| v as i64).collect()
}

fn xyz(p: [i32; 3]) -> Value {
    compound(vec![("x", Value::Int(p[0])), ("y", Value::Int(p[1])), ("z", Value::Int(p[2]))])
}

fn litematic(v: &Volume) -> Value {
    let [w, h, l] = v.size.map(|s| s as i32);
    let (palette, blocks) = compact(&v.palette, &v.blocks);
    let bits = (u32::BITS - (palette.len().saturating_sub(1) as u32).leading_zeros()).max(2);
    let states = pack_spanning(&blocks, bits);
    let entities: Vec<Value> = v
        .block_entities
        .iter()
        .filter_map(|be| {
            let mut c = entity_compound(be)?;
            c.insert("x".into(), Value::Int(be.pos[0]));
            c.insert("y".into(), Value::Int(be.pos[1]));
            c.insert("z".into(), Value::Int(be.pos[2]));
            Some(Value::Compound(c))
        })
        .collect();
    // The region from the origin: the schematic's own frame starts there.
    let position = [-v.origin[0], -v.origin[1], -v.origin[2]];
    let region = compound(vec![
        ("Position", xyz(position)),
        ("Size", xyz([w, h, l])),
        ("BlockStatePalette", Value::List(palette.iter().map(|s| state_entry(s)).collect())),
        ("BlockStates", Value::LongArray(LongArray::new(states))),
        ("TileEntities", Value::List(entities)),
        ("Entities", Value::List(Vec::new())),
        ("PendingBlockTicks", Value::List(Vec::new())),
        ("PendingFluidTicks", Value::List(Vec::new())),
    ]);
    let total_blocks = blocks.iter().filter(|&&b| b != 0 && !massif_core::objects::is_nothing(&palette[b as usize])).count();
    let now = now_millis();
    let volume = (w as i64 * h as i64 * l as i64).min(i32::MAX as i64) as i32;
    let meta = compound(vec![
        ("Name", Value::String(v.name.clone())),
        ("Author", Value::String(v.author.clone())),
        ("Description", Value::String(String::new())),
        ("RegionCount", Value::Int(1)),
        ("TotalVolume", Value::Int(volume)),
        ("TotalBlocks", Value::Int(total_blocks.min(i32::MAX as usize) as i32)),
        ("TimeCreated", Value::Long(now)),
        ("TimeModified", Value::Long(now)),
        ("EnclosingSize", xyz([w, h, l])),
    ]);
    let name = if v.name.trim().is_empty() { "Main".to_owned() } else { v.name.clone() };
    compound(vec![
        ("Version", Value::Int(6)),
        ("SubVersion", Value::Int(1)),
        ("MinecraftDataVersion", Value::Int(v.data_version)),
        ("Metadata", meta),
        ("Regions", Value::Compound(std::iter::once((name, region)).collect())),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A box of 5 × 4 × 3 with a log, leaves, stairs, a chest and air, and
    /// more than 16 states so that the indices take five bits.
    fn sample() -> Volume {
        let mut palette = vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned(), "minecraft:oak_log[axis=y]".to_owned(), "minecraft:oak_stairs[facing=east,half=bottom,shape=straight,waterlogged=false]".to_owned(), "minecraft:chest[facing=north,type=single,waterlogged=false]".to_owned()];
        for k in 0..14 {
            palette.push(format!("minecraft:{}_wool", ["white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown", "green"][k]));
        }
        let size = [5u32, 4, 3];
        let n = 60;
        let blocks: Vec<u16> = (0..n).map(|i| if i % 7 == 3 { 0 } else { (i as u16 * 5) % palette.len() as u16 }).collect();
        let chest = fastnbt::to_bytes(&compound(vec![("id", Value::String("minecraft:chest".into())), ("Items", Value::List(Vec::new())), ("CustomName", Value::String("\"Loot\"".into()))])).unwrap();
        Volume {
            name: "Sample".into(),
            author: "Massif".into(),
            size,
            palette,
            blocks,
            origin: [2, 0, 1],
            block_entities: vec![BlockEntity { pos: [1, 2, 0], nbt: chest }],
            biomes: Some((vec!["minecraft:plains".into(), "minecraft:forest".into()], (0..n).map(|i| (i % 2) as u16).collect())),
            data_version: 3955,
        }
    }

    fn state_at(v: &Volume, x: usize, y: usize, z: usize) -> &str {
        let [w, _, l] = v.size.map(|s| s as usize);
        &v.palette[v.blocks[x + z * w + y * w * l] as usize]
    }

    #[test]
    fn written_schematics_read_back_as_the_same_blocks() {
        let v = sample();
        for format in [Format::Sponge3, Format::Sponge2, Format::Litematic] {
            let bytes = write(&v, format).unwrap();
            let read = crate::read_bytes("x", &bytes).unwrap();
            let o = &read.object;
            // The reader trims the box to its blocks: here they reach every
            // side.
            assert_eq!(o.size.map(u32::from), v.size, "{format:?}");
            for y in 0..4 {
                for z in 0..3 {
                    for x in 0..5 {
                        let want = state_at(&v, x, y, z);
                        let got = &o.palette[o.at(x, y, z) as usize];
                        assert_eq!(got, if massif_core::objects::is_nothing(want) { "minecraft:air" } else { want }, "{format:?} at {x} {y} {z}");
                    }
                }
            }
            assert_eq!(o.name, "Sample", "{format:?}");
        }
    }

    #[test]
    fn the_layouts_follow_the_specifications() {
        let v = sample();
        let root = |format| match crate::parse_nbt(&write(&v, format).unwrap()).unwrap() {
            Value::Compound(c) => c,
            _ => panic!(),
        };
        // Sponge 3: everything under `Schematic`, blocks under `Blocks`.
        let r = root(Format::Sponge3);
        let Some(Value::Compound(s)) = r.get("Schematic") else { panic!() };
        assert_eq!(s.get("Version"), Some(&Value::Int(3)));
        assert_eq!(s.get("DataVersion"), Some(&Value::Int(3955)));
        assert_eq!(s.get("Width"), Some(&Value::Short(5)));
        assert_eq!(s.get("Offset"), Some(&Value::IntArray(IntArray::new(vec![-2, 0, -1]))));
        let Some(Value::Compound(b)) = s.get("Blocks") else { panic!() };
        let Some(Value::List(bes)) = b.get("BlockEntities") else { panic!() };
        let Value::Compound(be) = &bes[0] else { panic!() };
        assert_eq!(be.get("Id"), Some(&Value::String("minecraft:chest".into())));
        assert_eq!(be.get("Pos"), Some(&Value::IntArray(IntArray::new(vec![1, 2, 0]))));
        let Some(Value::Compound(data)) = be.get("Data") else { panic!() };
        assert!(data.contains_key("CustomName") && !data.contains_key("id"));
        let Some(Value::Compound(biomes)) = s.get("Biomes") else { panic!() };
        assert!(matches!(biomes.get("Data"), Some(Value::ByteArray(d)) if d.len() == 60));
        // Sponge 2: flat, its palette's size given.
        let r = root(Format::Sponge2);
        assert_eq!(r.get("Version"), Some(&Value::Int(2)));
        assert!(matches!(r.get("PaletteMax"), Some(Value::Int(n)) if *n as usize == 19));
        assert!(matches!(r.get("BiomeData"), Some(Value::ByteArray(d)) if d.len() == 15));
        // Litematica: one region, air first, five bits a block.
        let r = root(Format::Litematic);
        assert_eq!(r.get("Version"), Some(&Value::Int(6)));
        let Some(Value::Compound(regions)) = r.get("Regions") else { panic!() };
        let Some(Value::Compound(region)) = regions.get("Sample") else { panic!() };
        let Some(Value::List(palette)) = region.get("BlockStatePalette") else { panic!() };
        assert_eq!(palette[0], state_entry("minecraft:air"));
        assert!(matches!(region.get("BlockStates"), Some(Value::LongArray(l)) if l.len() == (60 * 5usize).div_ceil(64)));
        assert_eq!(region.get("Position"), Some(&xyz([-2, 0, -1])));
        let Some(Value::Compound(meta)) = r.get("Metadata") else { panic!() };
        assert_eq!(meta.get("TotalBlocks"), Some(&Value::Int(v.count() as i32)));
    }

    #[test]
    fn objects_are_written_from_their_anchor() {
        let mut blocks = vec![0u16; 27];
        blocks[1 + 3 + 9] = 1;
        blocks[1 + 3] = 2;
        let o = massif_core::objects::ObjectDef::new("Bush", [3, 3, 3], vec!["minecraft:air".into(), "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]".into(), "minecraft:dirt".into()], blocks);
        let v = Volume::of_object(&o, 3955);
        assert_eq!(v.count(), 2);
        let back = crate::read_bytes("b", &write(&v, Format::Sponge3).unwrap()).unwrap();
        assert_eq!(back.object.count(), 2);
        assert_eq!(back.object.name, "Bush");
    }

    #[test]
    fn boxes_too_large_are_refused() {
        let v = Volume { size: [70_000, 1, 1], palette: vec!["minecraft:stone".into()], blocks: vec![0; 70_000], ..Default::default() };
        assert!(matches!(write(&v, Format::Sponge3), Err(WriteError::TooLarge(..))));
    }
}
