//! Custom objects read from the files map makers keep them in:
//!
//! - Sponge schematics (`.schem`, versions 1 to 3: WorldEdit, FAWE),
//! - MCEdit and Schematica schematics (`.schematic`: numbered blocks of the
//!   versions before 1.13),
//! - Axiom blueprints (`.bp`),
//! - Litematica schematics (`.litematic`, every region in one object),
//! - structures saved by a structure block (`.nbt`).
//!
//! Each becomes a [`massif_core::objects::ObjectDef`]: block states in a
//! box, air and structure voids left out, standing on its lowest layer
//! that is not soil. Block entities (chests' contents, signs' text) and
//! entities are not kept.

mod legacy;

use fastnbt::Value;
use massif_core::objects::ObjectDef;
use std::collections::HashMap;
use std::io::Read;
use std::path::Path;

/// The largest object read: 16 million blocks (256 × 256 × 256).
pub const MAX_VOLUME: usize = 1 << 24;

/// The extensions of the files objects are read from.
pub const EXTENSIONS: [&str; 5] = ["schem", "schematic", "bp", "litematic", "nbt"];

#[derive(Debug, thiserror::Error)]
pub enum ObjectError {
    #[error("{0}")]
    Io(#[from] std::io::Error),
    #[error("the file is not a schematic Massif knows: {0}")]
    Unknown(String),
    #[error("the schematic is damaged: {0}")]
    Corrupt(String),
    #[error("the object is too large: {0} blocks, at most {MAX_VOLUME}")]
    TooLarge(usize),
    #[error("the object holds no blocks")]
    Empty,
}

/// An object read from a file.
#[derive(Debug, Clone)]
pub struct Imported {
    pub object: ObjectDef,
    /// What the file was: "Sponge schematic", "Axiom blueprint"…
    pub format: &'static str,
    /// Blocks left out because their number meant nothing known (MCEdit
    /// schematics only).
    pub left_out: usize,
}

/// Read the object of a file, named after the file unless it names itself.
pub fn read(path: &Path) -> Result<Imported, ObjectError> {
    let bytes = std::fs::read(path)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Object".to_owned());
    let mut imported = read_bytes(&stem, &bytes)?;
    imported.object.source = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(imported)
}

/// Read an object from the bytes of a file; `name` when it has none.
pub fn read_bytes(name: &str, bytes: &[u8]) -> Result<Imported, ObjectError> {
    if bytes.starts_with(&AXIOM_MAGIC) {
        return axiom(name, bytes);
    }
    let raw = if bytes.starts_with(&[0x1f, 0x8b]) {
        let mut out = Vec::new();
        flate2::read::MultiGzDecoder::new(bytes).read_to_end(&mut out)?;
        out
    } else {
        bytes.to_vec()
    };
    let root: Value = fastnbt::from_bytes(&raw).map_err(|e| ObjectError::Unknown(e.to_string()))?;
    let Value::Compound(root) = root else { return Err(ObjectError::Unknown("no compound at the root".into())) };
    // Sponge's third version keeps everything in a compound of its own.
    if let Some(Value::Compound(inner)) = root.get("Schematic") {
        return sponge(name, inner);
    }
    if root.contains_key("Regions") {
        return litematic(name, &root);
    }
    if root.contains_key("BlockData") || root.contains_key("Palette") && root.contains_key("Width") {
        return sponge(name, &root);
    }
    if root.contains_key("Blocks") && root.contains_key("Width") {
        return mcedit(name, &root);
    }
    if root.contains_key("blocks") && root.contains_key("palette") && root.contains_key("size") {
        return structure(name, &root);
    }
    Err(ObjectError::Unknown("no blocks in a layout Massif knows".into()))
}

// ------------------------------------------------------------------ NBT --

type Compound = HashMap<String, Value>;

fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Byte(b) => Some(*b as i64),
        Value::Short(s) => Some(*s as i64),
        Value::Int(i) => Some(*i as i64),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

fn compound<'a>(c: &'a Compound, key: &str) -> Option<&'a Compound> {
    match c.get(key)? {
        Value::Compound(c) => Some(c),
        _ => None,
    }
}

fn list<'a>(c: &'a Compound, key: &str) -> &'a [Value] {
    match c.get(key) {
        Some(Value::List(l)) => l,
        _ => &[],
    }
}

fn string<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    match c.get(key)? {
        Value::String(s) => Some(s),
        _ => None,
    }
}

fn bytes<'a>(c: &'a Compound, key: &str) -> Option<&'a [i8]> {
    match c.get(key)? {
        Value::ByteArray(b) => Some(b),
        _ => None,
    }
}

fn longs<'a>(c: &'a Compound, key: &str) -> Option<&'a [i64]> {
    match c.get(key)? {
        Value::LongArray(l) => Some(l),
        _ => None,
    }
}

/// Three numbers: an int array, a list of numbers, or a compound of x, y
/// and z.
fn triple(v: Option<&Value>) -> Option<[i64; 3]> {
    match v? {
        Value::IntArray(a) if a.len() >= 3 => Some([a[0] as i64, a[1] as i64, a[2] as i64]),
        Value::List(l) if l.len() >= 3 => Some([int(l.first())?, int(l.get(1))?, int(l.get(2))?]),
        Value::Compound(c) => Some([int(c.get("x"))?, int(c.get("y"))?, int(c.get("z"))?]),
        _ => None,
    }
}

/// A block state from a palette entry: `Name` and its `Properties`, in the
/// game's order (by name).
fn state_of(entry: &Value) -> String {
    let Value::Compound(c) = entry else { return "minecraft:air".to_owned() };
    let name = string(c, "Name").unwrap_or("minecraft:air");
    let mut props: Vec<(String, String)> = compound(c, "Properties")
        .map(|p| {
            p.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => int(Some(other)).map_or_else(String::new, |n| n.to_string()),
                    };
                    (k.clone(), v)
                })
                .collect()
        })
        .unwrap_or_default();
    if props.is_empty() {
        return name.to_owned();
    }
    props.sort();
    let props: Vec<String> = props.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", props.join(","))
}

/// Unsigned LEB128 numbers, as Sponge schematics pack their blocks.
fn varints(data: &[i8], count: usize) -> Result<Vec<u16>, ObjectError> {
    let mut out = Vec::with_capacity(count);
    let (mut v, mut shift) = (0u32, 0u32);
    for &b in data {
        let b = b as u8;
        if shift > 28 {
            return Err(ObjectError::Corrupt("a block number is too long".into()));
        }
        v |= ((b & 0x7f) as u32) << shift;
        if b & 0x80 != 0 {
            shift += 7;
        } else {
            out.push(v.min(u16::MAX as u32) as u16);
            (v, shift) = (0, 0);
            if out.len() == count {
                break;
            }
        }
    }
    if out.len() < count {
        return Err(ObjectError::Corrupt(format!("{} blocks of {count}", out.len())));
    }
    Ok(out)
}

/// Palette indices packed in longs, `bits` each: never across two longs
/// (the game's chunks, Axiom), or end to end (Litematica).
fn unpack(longs: &[i64], bits: u32, count: usize, across: bool) -> Vec<u16> {
    let mask = (1u64 << bits) - 1;
    let mut out = Vec::with_capacity(count);
    if across {
        for i in 0..count {
            let bit = i as u64 * bits as u64;
            let (word, shift) = ((bit / 64) as usize, (bit % 64) as u32);
            let Some(&lo) = longs.get(word) else { break };
            let mut v = (lo as u64) >> shift;
            if shift + bits > 64
                && let Some(&hi) = longs.get(word + 1)
            {
                v |= (hi as u64) << (64 - shift);
            }
            out.push((v & mask) as u16);
        }
    } else {
        let per = (64 / bits) as usize;
        for i in 0..count {
            let Some(&word) = longs.get(i / per) else { break };
            out.push((((word as u64) >> ((i % per) as u32 * bits)) & mask) as u16);
        }
    }
    out.resize(count, 0);
    out
}

fn bits_for(n: usize, least: u32) -> u32 {
    (usize::BITS - n.saturating_sub(1).leading_zeros()).max(least)
}

fn check_size(size: [i64; 3]) -> Result<[u16; 3], ObjectError> {
    if size.iter().any(|&s| s <= 0) {
        return Err(ObjectError::Empty);
    }
    let volume = size.iter().map(|&s| s as usize).product::<usize>();
    if volume > MAX_VOLUME || size.iter().any(|&s| s > u16::MAX as i64) {
        return Err(ObjectError::TooLarge(volume));
    }
    Ok(size.map(|s| s as u16))
}

fn finish(name: &str, format: &'static str, size: [u16; 3], palette: Vec<String>, blocks: Vec<u16>, left_out: usize) -> Result<Imported, ObjectError> {
    let object = ObjectDef::new(name, size, palette, blocks);
    if object.count() == 0 {
        return Err(ObjectError::Empty);
    }
    Ok(Imported { object, format, left_out })
}

/// The name an object gives itself, if any.
fn own_name<'a>(c: &'a Compound, fallback: &'a str) -> &'a str {
    compound(c, "Metadata").and_then(|m| string(m, "Name")).filter(|n| !n.trim().is_empty()).unwrap_or(fallback)
}

// --------------------------------------------------------------- Sponge --

fn sponge(name: &str, c: &Compound) -> Result<Imported, ObjectError> {
    let size = [int(c.get("Width")), int(c.get("Height")), int(c.get("Length"))];
    let size = check_size(size.map(|s| s.map_or(0, |s| s & 0xffff)))?;
    let volume = size.iter().map(|&s| s as usize).product();
    // Version 3 keeps its palette and blocks in `Blocks`.
    let (palette, data) = match compound(c, "Blocks") {
        Some(b) => (compound(b, "Palette"), bytes(b, "Data")),
        None => (compound(c, "Palette"), bytes(c, "BlockData")),
    };
    let (Some(palette), Some(data)) = (palette, data) else { return Err(ObjectError::Corrupt("no palette or no blocks".into())) };
    let mut states = vec!["minecraft:air".to_owned(); palette.len()];
    for (state, index) in palette {
        let i = int(Some(index)).unwrap_or(-1);
        if i < 0 || i >= u16::MAX as i64 {
            return Err(ObjectError::Corrupt(format!("palette entry {state} numbered {i}")));
        }
        if i as usize >= states.len() {
            states.resize(i as usize + 1, "minecraft:air".to_owned());
        }
        states[i as usize] = state.clone();
    }
    let blocks = varints(data, volume)?;
    finish(own_name(c, name), "Sponge schematic", size, states, blocks, 0)
}

// --------------------------------------------------------------- MCEdit --

fn mcedit(name: &str, c: &Compound) -> Result<Imported, ObjectError> {
    let size = [int(c.get("Width")), int(c.get("Height")), int(c.get("Length"))];
    let [w, h, l] = check_size(size.map(|s| s.map_or(0, |s| s & 0xffff)))?;
    let volume = w as usize * h as usize * l as usize;
    let ids = bytes(c, "Blocks").ok_or_else(|| ObjectError::Corrupt("no blocks".into()))?;
    let data = bytes(c, "Data").unwrap_or(&[]);
    let add = bytes(c, "AddBlocks").unwrap_or(&[]);
    if ids.len() < volume {
        return Err(ObjectError::Corrupt(format!("{} blocks of {volume}", ids.len())));
    }
    // MCEdit's layout: y, then z, then x — the same order as Sponge's.
    let id_at = |i: usize| -> u16 {
        let low = ids[i] as u8 as u16;
        let high = match add.get(i >> 1) {
            Some(&a) if i & 1 == 0 => ((a as u8 & 0x0f) as u16) << 8,
            Some(&a) => ((a as u8 & 0xf0) as u16) << 4,
            None => 0,
        };
        high | low
    };
    let mut palette = vec!["minecraft:air".to_owned()];
    let mut known: HashMap<(u16, u8), u16> = HashMap::new();
    let mut blocks = Vec::with_capacity(volume);
    let mut left_out = 0;
    for i in 0..volume {
        let (id, meta) = (id_at(i), data.get(i).map_or(0, |&d| d as u8 & 15));
        if id == 0 {
            blocks.push(0);
            continue;
        }
        // The top half of a tall plant is told by the half below it.
        let below = (i >= w as usize * l as usize).then(|| i - w as usize * l as usize).map(|j| (id_at(j), data.get(j).map_or(0, |&d| d as u8 & 15)));
        let key = if id == 175 && meta >= 8 { (id, 8 | below.filter(|b| b.0 == 175).map_or(2, |b| b.1 & 7)) } else { (id, meta) };
        let index = match known.get(&key) {
            Some(&k) => k,
            None => {
                let k = match legacy::state(key.0, key.1) {
                    Some(state) => {
                        palette.push(state);
                        (palette.len() - 1) as u16
                    }
                    None => 0,
                };
                known.insert(key, k);
                k
            }
        };
        if index == 0 {
            left_out += 1;
        }
        blocks.push(index);
    }
    finish(name, "MCEdit schematic", [w, h, l], palette, blocks, left_out)
}

// ---------------------------------------------------------------- Axiom --

const AXIOM_MAGIC: [u8; 4] = [0x0a, 0xe5, 0xbb, 0x36];

fn axiom(name: &str, bytes: &[u8]) -> Result<Imported, ObjectError> {
    let corrupt = |what: &str| ObjectError::Corrupt(format!("Axiom blueprint: {what}"));
    let mut at = 4usize;
    let mut chunk = |what: &str| -> Result<&[u8], ObjectError> {
        let len = bytes.get(at..at + 4).ok_or_else(|| corrupt(what))?;
        let len = u32::from_be_bytes([len[0], len[1], len[2], len[3]]) as usize;
        let part = bytes.get(at + 4..at + 4 + len).ok_or_else(|| corrupt(what))?;
        at += 4 + len;
        Ok(part)
    };
    let header = chunk("its header")?;
    let _thumbnail = chunk("its picture")?;
    let data = chunk("its blocks")?;
    let header: Option<Compound> = fastnbt::from_bytes::<Value>(header).ok().and_then(|v| match v {
        Value::Compound(c) => Some(c),
        _ => None,
    });
    let own = header.as_ref().and_then(|h| string(h, "Name")).filter(|n| !n.trim().is_empty()).map(str::to_owned);
    let mut raw = Vec::new();
    flate2::read::MultiGzDecoder::new(data).read_to_end(&mut raw)?;
    let root: Value = fastnbt::from_bytes(&raw).map_err(|e| corrupt(&e.to_string()))?;
    let Value::Compound(root) = root else { return Err(corrupt("no compound")) };
    // Sections of 16³ blocks, each with its palette.
    let mut placed: Vec<([i64; 3], String)> = Vec::new();
    for region in list(&root, "BlockRegion") {
        let Value::Compound(region) = region else { continue };
        let origin = [int(region.get("X")), int(region.get("Y")), int(region.get("Z"))].map(|v| v.unwrap_or(0) * 16);
        let Some(states) = compound(region, "BlockStates").or_else(|| compound(region, "data")) else { continue };
        let palette: Vec<String> = list(states, "palette").iter().map(state_of).collect();
        if palette.is_empty() {
            continue;
        }
        let indices = match longs(states, "data") {
            Some(l) => unpack(l, bits_for(palette.len(), 4), 4096, false),
            None => vec![0; 4096],
        };
        for (i, &k) in indices.iter().enumerate() {
            let Some(state) = palette.get(k as usize) else { continue };
            if massif_core::objects::is_nothing(state) {
                continue;
            }
            let (x, y, z) = ((i & 15) as i64, (i >> 8) as i64, ((i >> 4) & 15) as i64);
            placed.push(([origin[0] + x, origin[1] + y, origin[2] + z], state.clone()));
        }
    }
    let (size, palette, blocks) = boxed(&placed)?;
    finish(own.as_deref().unwrap_or(name), "Axiom blueprint", size, palette, blocks, 0)
}

/// An object's box: its size, its palette and its blocks.
type Boxed = ([u16; 3], Vec<String>, Vec<u16>);

/// Blocks given by their place, in the smallest box holding them all.
fn boxed(placed: &[([i64; 3], String)]) -> Result<Boxed, ObjectError> {
    if placed.is_empty() {
        return Err(ObjectError::Empty);
    }
    let mut lo = [i64::MAX; 3];
    let mut hi = [i64::MIN; 3];
    for (p, _) in placed {
        for a in 0..3 {
            lo[a] = lo[a].min(p[a]);
            hi[a] = hi[a].max(p[a]);
        }
    }
    let size = check_size([hi[0] - lo[0] + 1, hi[1] - lo[1] + 1, hi[2] - lo[2] + 1])?;
    let [w, h, l] = size.map(|s| s as usize);
    let mut palette = vec!["minecraft:air".to_owned()];
    let mut index: HashMap<&str, u16> = HashMap::new();
    let mut blocks = vec![0u16; w * h * l];
    for (p, state) in placed {
        let k = *index.entry(state.as_str()).or_insert_with(|| {
            palette.push(state.clone());
            (palette.len() - 1) as u16
        });
        let (x, y, z) = ((p[0] - lo[0]) as usize, (p[1] - lo[1]) as usize, (p[2] - lo[2]) as usize);
        blocks[x + z * w + y * w * l] = k;
    }
    Ok((size, palette, blocks))
}

// ----------------------------------------------------------- Litematica --

fn litematic(name: &str, root: &Compound) -> Result<Imported, ObjectError> {
    let mut placed: Vec<([i64; 3], String)> = Vec::new();
    let Some(regions) = compound(root, "Regions") else { return Err(ObjectError::Corrupt("no regions".into())) };
    let mut names: Vec<&String> = regions.keys().collect();
    names.sort();
    for region_name in names {
        let Some(Value::Compound(region)) = regions.get(region_name) else { continue };
        let (Some(pos), Some(size)) = (triple(region.get("Position")), triple(region.get("Size"))) else { continue };
        // A negative size runs from the position back.
        let origin = [0, 1, 2].map(|a| if size[a] >= 0 { pos[a] } else { pos[a] + size[a] + 1 });
        let dims = check_size(size.map(i64::abs))?.map(|s| s as usize);
        let palette: Vec<String> = list(region, "BlockStatePalette").iter().map(state_of).collect();
        let Some(data) = longs(region, "BlockStates") else { continue };
        let volume = dims[0] * dims[1] * dims[2];
        let indices = unpack(data, bits_for(palette.len(), 2), volume, true);
        for (i, &k) in indices.iter().enumerate() {
            let Some(state) = palette.get(k as usize) else { continue };
            if massif_core::objects::is_nothing(state) {
                continue;
            }
            let (x, z, y) = (i % dims[0], (i / dims[0]) % dims[2], i / (dims[0] * dims[2]));
            placed.push(([origin[0] + x as i64, origin[1] + y as i64, origin[2] + z as i64], state.clone()));
        }
    }
    let (size, palette, blocks) = boxed(&placed)?;
    finish(own_name(root, name), "Litematica schematic", size, palette, blocks, 0)
}

// ------------------------------------------------------------ structure --

fn structure(name: &str, root: &Compound) -> Result<Imported, ObjectError> {
    let palette: Vec<String> = match root.get("palette") {
        Some(Value::List(p)) => p.iter().map(state_of).collect(),
        _ => Vec::new(),
    };
    let mut placed = Vec::new();
    for block in list(root, "blocks") {
        let Value::Compound(b) = block else { continue };
        let (Some(pos), Some(k)) = (triple(b.get("pos")), int(b.get("state"))) else { continue };
        let Some(state) = palette.get(k as usize) else { continue };
        if !massif_core::objects::is_nothing(state) {
            placed.push((pos, state.clone()));
        }
    }
    let (size, palette, blocks) = boxed(&placed)?;
    finish(name, "structure", size, palette, blocks, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gzip(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }

    fn c(entries: Vec<(&str, Value)>) -> Value {
        Value::Compound(entries.into_iter().map(|(k, v)| (k.to_owned(), v)).collect())
    }

    fn nbt(v: &Value) -> Vec<u8> {
        gzip(&fastnbt::to_bytes(v).unwrap())
    }

    /// The tree every format holds in the tests: a trunk 3 high on a
    /// dirt block, a crown of leaves 3 × 3 around its top, a box 3 wide,
    /// 5 high and 3 long.
    fn tree_blocks() -> Vec<([i64; 3], &'static str)> {
        let mut b = vec![([1, 0, 1], "minecraft:dirt")];
        for y in 1..4 {
            b.push(([1, y, 1], "minecraft:oak_log[axis=y]"));
        }
        for y in 3..5 {
            for z in 0..3 {
                for x in 0..3 {
                    if !(x == 1 && z == 1 && y == 3) {
                        b.push(([x, y, z], "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]"));
                    }
                }
            }
        }
        b
    }

    fn check_tree(i: &Imported) {
        let o = &i.object;
        assert_eq!(o.size, [3, 5, 3], "{}", i.format);
        assert_eq!(o.anchor, [1, 1, 1], "{}", i.format);
        assert_eq!(o.count(), 1 + 3 + 17, "{}", i.format);
        let state = |x, y, z| o.palette[o.at(x, y, z) as usize].clone();
        assert_eq!(state(1, 2, 1), "minecraft:oak_log[axis=y]", "{}", i.format);
        assert_eq!(state(1, 0, 1), "minecraft:dirt", "{}", i.format);
        assert_eq!(state(0, 4, 2), "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]", "{}", i.format);
        assert_eq!(state(0, 0, 0), "minecraft:air", "{}", i.format);
    }

    fn sponge_body(version: i32) -> Vec<(&'static str, Value)> {
        let mut palette: Vec<&str> = vec!["minecraft:air"];
        let mut data = Vec::new();
        let blocks = tree_blocks();
        for y in 0..5 {
            for z in 0..3 {
                for x in 0..3 {
                    let state = blocks.iter().find(|b| b.0 == [x, y, z]).map_or("minecraft:air", |b| b.1);
                    let k = palette.iter().position(|p| *p == state).unwrap_or_else(|| {
                        palette.push(state);
                        palette.len() - 1
                    });
                    // Palette numbers past 127 take two bytes: add some.
                    let k = k as u32 + if k > 0 { 200 } else { 0 };
                    let mut v = k;
                    loop {
                        let b = (v & 0x7f) as u8;
                        v >>= 7;
                        if v == 0 {
                            data.push(b as i8);
                            break;
                        }
                        data.push((b | 0x80) as i8);
                    }
                }
            }
        }
        let palette = c(palette.iter().enumerate().map(|(k, p)| (*p, Value::Int(k as i32 + if k > 0 { 200 } else { 0 }))).collect());
        let mut body = vec![("Version", Value::Int(version)), ("Width", Value::Short(3)), ("Height", Value::Short(5)), ("Length", Value::Short(3))];
        if version == 3 {
            body.push(("Blocks", c(vec![("Palette", palette), ("Data", Value::ByteArray(fastnbt::ByteArray::new(data)))])));
            body.push(("Metadata", c(vec![("Name", Value::String("Oak".into()))])));
        } else {
            body.push(("Palette", palette));
            body.push(("BlockData", Value::ByteArray(fastnbt::ByteArray::new(data))));
        }
        body
    }

    #[test]
    fn sponge_schematics_are_read() {
        let v2 = read_bytes("tree", &nbt(&c(sponge_body(2)))).unwrap();
        check_tree(&v2);
        assert_eq!((v2.object.name.as_str(), v2.format), ("tree", "Sponge schematic"));
        let v3 = read_bytes("tree", &nbt(&c(vec![("Schematic", c(sponge_body(3)))]))).unwrap();
        check_tree(&v3);
        assert_eq!(v3.object.name, "Oak");
    }

    #[test]
    fn mcedit_schematics_are_read() {
        // Oak log 17:0, leaves 18:0 (no decay: 18:4), dirt 3; a vine on
        // the trunk's west (106, data 8: east face), an unknown block 255.
        let (w, h, l) = (3usize, 5usize, 3usize);
        let mut ids = vec![0i8; w * h * l];
        let mut data = vec![0i8; w * h * l];
        let at = |x: usize, y: usize, z: usize| (y * l + z) * w + x;
        for ([x, y, z], state) in tree_blocks() {
            let (id, d) = match state {
                "minecraft:dirt" => (3, 0),
                "minecraft:oak_log[axis=y]" => (17, 0),
                _ => (18, 4),
            };
            ids[at(x as usize, y as usize, z as usize)] = id;
            data[at(x as usize, y as usize, z as usize)] = d;
        }
        ids[at(0, 1, 1)] = 106;
        data[at(0, 1, 1)] = 8;
        ids[at(2, 0, 2)] = -1;
        let file = c(vec![
            ("Width", Value::Short(3)),
            ("Height", Value::Short(5)),
            ("Length", Value::Short(3)),
            ("Materials", Value::String("Alpha".into())),
            ("Blocks", Value::ByteArray(fastnbt::ByteArray::new(ids))),
            ("Data", Value::ByteArray(fastnbt::ByteArray::new(data))),
        ]);
        let i = read_bytes("old tree", &nbt(&file)).unwrap();
        assert_eq!((i.format, i.left_out), ("MCEdit schematic", 1));
        let o = &i.object;
        assert_eq!(o.size, [3, 5, 3]);
        assert_eq!(o.palette[o.at(1, 2, 1) as usize], "minecraft:oak_log[axis=y]");
        assert_eq!(o.palette[o.at(0, 4, 2) as usize], "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]");
        assert_eq!(o.palette[o.at(0, 1, 1) as usize], "minecraft:vine[east=true,north=false,south=false,up=false,west=false]");
        assert_eq!(o.at(2, 0, 2), 0);
        assert_eq!(o.anchor, [1, 1, 1]);
    }

    #[test]
    fn axiom_blueprints_are_read() {
        // One section with the tree at block (16 + x, y, z): a palette of
        // more than 16 states, 5 bits each.
        let mut palette = vec![c(vec![("Name", Value::String("minecraft:structure_void".into()))])];
        for k in 0..18 {
            palette.push(c(vec![("Name", Value::String(format!("minecraft:white_wool{}", if k == 0 { "" } else { "_x" })))]));
        }
        let mut states: Vec<String> = palette.iter().map(state_of).collect();
        let mut indices = vec![0u64; 4096];
        for ([x, y, z], state) in tree_blocks() {
            let k = states.iter().position(|s| s == state).unwrap_or_else(|| {
                let (name, props) = state.split_once('[').map_or((state, ""), |(n, p)| (n, p.trim_end_matches(']')));
                let props: Vec<(&str, Value)> = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k, Value::String(v.to_owned()))).collect();
                let mut entry = vec![("Name", Value::String(name.to_owned()))];
                if !props.is_empty() {
                    entry.push(("Properties", c(props)));
                }
                palette.push(c(entry));
                states.push(state.to_owned());
                states.len() - 1
            });
            indices[((y * 16 + z) * 16 + x) as usize] = k as u64;
        }
        let bits = bits_for(palette.len(), 4) as usize;
        assert_eq!(bits, 5);
        let per = 64 / bits;
        let mut data = vec![0i64; 4096usize.div_ceil(per)];
        for (i, &k) in indices.iter().enumerate() {
            data[i / per] |= (k << ((i % per) * bits)) as i64;
        }
        let region = c(vec![
            ("X", Value::Int(1)),
            ("Y", Value::Int(0)),
            ("Z", Value::Int(0)),
            ("BlockStates", c(vec![("palette", Value::List(palette)), ("data", Value::LongArray(fastnbt::LongArray::new(data)))])),
        ]);
        let blocks = nbt(&c(vec![("DataVersion", Value::Int(4189)), ("BlockRegion", Value::List(vec![region]))]));
        let header = fastnbt::to_bytes(&c(vec![("Name", Value::String("Axiom oak".into())), ("Version", Value::Long(0))])).unwrap();
        let mut file = AXIOM_MAGIC.to_vec();
        for part in [&header[..], b"\x89PNG", &blocks[..]] {
            file.extend_from_slice(&(part.len() as u32).to_be_bytes());
            file.extend_from_slice(part);
        }
        let i = read_bytes("bp", &file).unwrap();
        check_tree(&i);
        assert_eq!((i.object.name.as_str(), i.format), ("Axiom oak", "Axiom blueprint"));
        // Cut short: an error, not a panic.
        assert!(read_bytes("bp", &file[..file.len() - 9]).is_err());
    }

    #[test]
    fn litematics_and_structures_are_read() {
        // A region given from its far corner (negative size).
        let mut palette = vec!["minecraft:air".to_owned()];
        let mut indices = vec![0u64; 3 * 5 * 3];
        for ([x, y, z], state) in tree_blocks() {
            let k = palette.iter().position(|p| p == state).unwrap_or_else(|| {
                palette.push(state.to_owned());
                palette.len() - 1
            });
            indices[(x + z * 3 + y * 9) as usize] = k as u64;
        }
        let bits = bits_for(palette.len(), 2) as u64;
        let mut data = vec![0u64; (indices.len() as u64 * bits).div_ceil(64) as usize];
        for (i, &k) in indices.iter().enumerate() {
            let bit = i as u64 * bits;
            data[(bit / 64) as usize] |= k << (bit % 64);
            if bit % 64 + bits > 64 {
                data[(bit / 64) as usize + 1] |= k >> (64 - bit % 64);
            }
        }
        let entry = |s: &str| {
            let (name, props) = s.split_once('[').map_or((s, ""), |(n, p)| (n, p.trim_end_matches(']')));
            let props: Vec<(&str, Value)> = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k, Value::String(v.to_owned()))).collect();
            c(vec![("Name", Value::String(name.to_owned())), ("Properties", c(props))])
        };
        let xyz = |x, y, z| c(vec![("x", Value::Int(x)), ("y", Value::Int(y)), ("z", Value::Int(z))]);
        let region = c(vec![
            ("Position", xyz(10, 64, 10)),
            ("Size", xyz(-3, 5, -3)),
            ("BlockStatePalette", Value::List(palette.iter().map(|p| entry(p)).collect())),
            ("BlockStates", Value::LongArray(fastnbt::LongArray::new(data.into_iter().map(|v| v as i64).collect()))),
        ]);
        let file = c(vec![("Version", Value::Int(6)), ("Metadata", c(vec![("Name", Value::String("Litematic oak".into()))])), ("Regions", c(vec![("Main", region)]))]);
        let i = read_bytes("lm", &nbt(&file)).unwrap();
        check_tree(&i);
        assert_eq!(i.object.name, "Litematic oak");

        // A structure block's file: blocks listed with their place.
        let mut palette: Vec<String> = Vec::new();
        let mut blocks = Vec::new();
        for ([x, y, z], state) in tree_blocks() {
            let k = palette.iter().position(|p| p == state).unwrap_or_else(|| {
                palette.push(state.to_owned());
                palette.len() - 1
            });
            blocks.push(c(vec![("pos", Value::List(vec![Value::Int(x as i32), Value::Int(y as i32), Value::Int(z as i32)])), ("state", Value::Int(k as i32))]));
        }
        let file = c(vec![
            ("size", Value::List(vec![Value::Int(3), Value::Int(5), Value::Int(3)])),
            ("palette", Value::List(palette.iter().map(|p| entry(p)).collect())),
            ("blocks", Value::List(blocks)),
        ]);
        check_tree(&read_bytes("nbt", &nbt(&file)).unwrap());
    }

    #[test]
    fn what_is_not_a_schematic_is_refused() {
        assert!(matches!(read_bytes("x", b"hello"), Err(ObjectError::Unknown(_))));
        assert!(matches!(read_bytes("x", &nbt(&c(vec![("Foo", Value::Int(1))]))), Err(ObjectError::Unknown(_))));
        // Only air: nothing to place.
        let empty = c(vec![
            ("Width", Value::Short(1)),
            ("Height", Value::Short(1)),
            ("Length", Value::Short(1)),
            ("Palette", c(vec![("minecraft:air", Value::Int(0))])),
            ("BlockData", Value::ByteArray(fastnbt::ByteArray::new(vec![0]))),
        ]);
        assert!(matches!(read_bytes("x", &nbt(&empty)), Err(ObjectError::Empty)));
        // Too few blocks for its size.
        let short = c(vec![
            ("Width", Value::Short(4)),
            ("Height", Value::Short(1)),
            ("Length", Value::Short(1)),
            ("Palette", c(vec![("minecraft:stone", Value::Int(0))])),
            ("BlockData", Value::ByteArray(fastnbt::ByteArray::new(vec![0]))),
        ]);
        assert!(matches!(read_bytes("x", &nbt(&short)), Err(ObjectError::Corrupt(_))));
    }
}
