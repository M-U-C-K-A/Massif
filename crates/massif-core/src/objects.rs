//! Custom objects — a tree, a rock, a ruin: blocks in a box read from a
//! schematic — and the object layers that stand them over the map.
//!
//! An object layer is painted like a ground cover: a tile's object plane
//! holds 0, or the number of the layer painted there (its index in
//! [`Registry::object_layers`] plus one). Where its objects stand is decided
//! by a grid of squares the size of the layer's spacing: each square may
//! hold one, at a spot, of a kind and a turn picked from the world's seed,
//! where the top of the land under that spot has the layer painted. The
//! map, the 3D view and the export all ask [`tile_placements`], so they
//! agree.

use crate::compose::{Materializer, TileStack, hash2};
use crate::coords::{BlockRect, TilePos, local_index};
use crate::registry::{Registry, is_soil};
use serde::{Deserialize, Serialize};

/// Whether a block of a schematic means "nothing here": air, and the
/// structure void that marks the empty parts of a structure.
pub fn is_nothing(state: &str) -> bool {
    let name = state.split('[').next().unwrap_or(state);
    matches!(name, "" | "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air" | "minecraft:structure_void")
}

/// A block state with its properties in the game's order (by name), so
/// that the same block reads the same from every file.
pub fn canonical(state: &str) -> String {
    let state = state.trim();
    let Some((name, rest)) = state.split_once('[') else { return state.to_owned() };
    let mut props: Vec<&str> = rest.trim_end_matches(']').split(',').map(str::trim).filter(|p| !p.is_empty()).collect();
    props.sort_by_key(|p| p.split('=').next().unwrap_or(""));
    if props.is_empty() { name.to_owned() } else { format!("{name}[{}]", props.join(",")) }
}

/// A custom object: blocks in a box.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectDef {
    pub name: String,
    /// Width (x), height (y) and length (z) of the box.
    pub size: [u16; 3],
    /// Block states (`minecraft:oak_log[axis=y]`); index 0 is nothing: it
    /// leaves what is there.
    pub palette: Vec<String>,
    /// The palette index of each block of the box, x first, then z, then y.
    #[serde(with = "packed")]
    pub blocks: Vec<u16>,
    /// The block of the box set on the ground: the foot of a trunk. It goes
    /// in the first block above the surface.
    pub anchor: [u16; 3],
    /// The file it was read from.
    #[serde(default)]
    pub source: String,
    /// The colour of each palette entry, the game's own when it was known
    /// (see [`ObjectDef::color`]).
    #[serde(default)]
    pub colors: Vec<[u8; 3]>,
}

/// A colour for a block, told from its name, when the game's own is not
/// known: leaves green, logs brown, stone grey…
pub fn guess_color(state: &str) -> [u8; 3] {
    let name = state.split('[').next().unwrap_or(state);
    let n = name.strip_prefix("minecraft:").unwrap_or(name);
    if let Some(c) = crate::registry::plant_color(state) {
        return c;
    }
    let has = |w: &str| n.contains(w);
    let wood = |kind: &str| match kind {
        k if k.contains("birch") => [216, 212, 200],
        k if k.contains("dark_oak") => [62, 46, 28],
        k if k.contains("spruce") => [84, 62, 40],
        k if k.contains("acacia") => [104, 96, 86],
        k if k.contains("jungle") => [86, 68, 38],
        k if k.contains("cherry") => [54, 34, 44],
        k if k.contains("mangrove") => [84, 66, 46],
        k if k.contains("crimson") => [96, 40, 56],
        k if k.contains("warped") => [56, 90, 90],
        _ => [104, 82, 50],
    };
    if has("vine") || has("lichen") || has("hanging_roots") {
        return [62, 112, 40];
    }
    if has("leaves") || has("wart_block") && has("warped") {
        return match n {
            k if k.contains("birch") => [104, 140, 66],
            k if k.contains("spruce") => [58, 92, 60],
            k if k.contains("dark_oak") => [52, 98, 32],
            k if k.contains("jungle") => [50, 136, 28],
            k if k.contains("acacia") => [84, 128, 38],
            k if k.contains("cherry") => [232, 174, 204],
            k if k.contains("azalea") => [96, 130, 58],
            k if k.contains("pale_oak") => [178, 184, 170],
            _ => [64, 120, 42],
        };
    }
    if has("log") || has("wood") || has("stem") || has("hyphae") {
        return wood(n);
    }
    if has("planks") || has("fence") || has("stairs") && wood(n) != [104, 82, 50] || has("slab") && wood(n) != [104, 82, 50] {
        let w = wood(n);
        return [w[0].saturating_add(50), w[1].saturating_add(40), w[2].saturating_add(30)];
    }
    let table: [(&str, [u8; 3]); 24] = [
        ("snow", [244, 250, 252]),
        ("ice", [146, 182, 250]),
        ("water", [62, 118, 228]),
        ("lava", [208, 92, 16]),
        ("sand", [218, 206, 160]),
        ("gravel", [134, 126, 124]),
        ("grass_block", [96, 148, 62]),
        ("moss", [90, 110, 46]),
        ("dirt", [134, 96, 67]),
        ("mud", [60, 57, 61]),
        ("clay", [160, 166, 179]),
        ("terracotta", [152, 94, 67]),
        ("brick", [150, 98, 82]),
        ("deepslate", [80, 80, 82]),
        ("blackstone", [42, 36, 41]),
        ("basalt", [72, 72, 78]),
        ("calcite", [223, 224, 220]),
        ("granite", [149, 103, 85]),
        ("diorite", [188, 188, 188]),
        ("netherrack", [97, 38, 38]),
        ("mushroom_block", [150, 110, 80]),
        ("glass", [200, 220, 230]),
        ("wool", [230, 230, 230]),
        ("pumpkin", [214, 128, 30]),
    ];
    if let Some((_, c)) = table.iter().find(|(w, _)| has(w)) {
        return *c;
    }
    // Stone, cobblestone, andesite and the rest of the rock.
    [125, 125, 125]
}

impl ObjectDef {
    /// An object of the given blocks, its anchor found from them (see
    /// [`ObjectDef::find_anchor`]). States that mean nothing become 0.
    pub fn new(name: &str, size: [u16; 3], palette: Vec<String>, blocks: Vec<u16>) -> Self {
        let mut def = Self {
            name: name.to_owned(),
            size,
            palette,
            blocks,
            anchor: [0; 3],
            source: String::new(),
            colors: Vec::new(),
        };
        def.normalise();
        def.anchor = def.find_anchor();
        def
    }

    /// Make palette entry 0 nothing, and every state that means nothing 0;
    /// drop entries no block uses.
    fn normalise(&mut self) {
        let volume = self.volume();
        self.blocks.resize(volume, 0);
        let mut palette = vec!["minecraft:air".to_owned()];
        let mut map = Vec::with_capacity(self.palette.len());
        let mut used = vec![false; self.palette.len()];
        for &b in &self.blocks {
            if let Some(u) = used.get_mut(b as usize) {
                *u = true;
            }
        }
        for (k, state) in self.palette.iter().enumerate() {
            let state = canonical(state);
            if is_nothing(&state) || !used[k] {
                map.push(0u16);
            } else if let Some(i) = palette.iter().position(|p| *p == state) {
                map.push(i as u16);
            } else {
                palette.push(state);
                map.push((palette.len() - 1) as u16);
            }
        }
        for b in &mut self.blocks {
            *b = map.get(*b as usize).copied().unwrap_or(0);
        }
        self.palette = palette;
    }

    /// The colour of palette entry `k`.
    pub fn color(&self, k: u16) -> [u8; 3] {
        match self.colors.get(k as usize) {
            Some(&c) => c,
            None => self.palette.get(k as usize).map_or([125, 125, 125], |s| guess_color(s)),
        }
    }

    pub fn volume(&self) -> usize {
        self.size.iter().map(|&s| s as usize).product()
    }

    /// The palette index of block `(x, y, z)` of the box.
    #[inline]
    pub fn at(&self, x: usize, y: usize, z: usize) -> u16 {
        let (w, l) = (self.size[0] as usize, self.size[2] as usize);
        self.blocks[x + z * w + y * w * l]
    }

    /// How many blocks it places.
    pub fn count(&self) -> usize {
        self.blocks.iter().filter(|&&b| b != 0).count()
    }

    /// Where it stands: the lowest layer of the box that holds anything
    /// but soil (the dirt a tree was copied with goes into the ground) and
    /// loose plants (the vines hanging from it), at the block of that layer
    /// nearest the middle of the whole object — the foot of a trunk.
    pub fn find_anchor(&self) -> [u16; 3] {
        let [w, h, l] = self.size.map(|s| s as usize);
        let soil: Vec<bool> = self.palette.iter().map(|p| is_soil(p)).collect();
        let loose: Vec<bool> = self.palette.iter().map(|p| crate::registry::plant_color(p).is_some() || p.contains("vine") || p.contains("leaves")).collect();
        let layer = |y: usize, keep: &dyn Fn(u16) -> bool| -> Vec<(usize, usize)> {
            (0..l).flat_map(|z| (0..w).map(move |x| (x, z))).filter(|&(x, z)| keep(self.at(x, y, z))).collect()
        };
        let some = |b: u16| b != 0;
        let solid = |b: u16| b != 0 && !soil[b as usize] && !loose[b as usize];
        let plant = |b: u16| b != 0 && !soil[b as usize];
        // The first layer with a block that holds the rest up, else the
        // first with anything but soil, else the first with anything.
        let (y, blocks) = [&solid as &dyn Fn(u16) -> bool, &plant, &some]
            .into_iter()
            .find_map(|keep| (0..h).map(|y| (y, layer(y, keep))).find(|(_, b)| !b.is_empty()))
            .unwrap_or((0, vec![(w / 2, l / 2)]));
        // The middle of the whole object, seen from above.
        let (mut mx, mut mz, mut n) = (0.0f64, 0.0f64, 0.0f64);
        for y in 0..h {
            for z in 0..l {
                for x in 0..w {
                    if self.at(x, y, z) != 0 {
                        (mx, mz, n) = (mx + x as f64, mz + z as f64, n + 1.0);
                    }
                }
            }
        }
        let (mx, mz) = if n > 0.0 { ((mx / n) as f32, (mz / n) as f32) } else { (w as f32 / 2.0, l as f32 / 2.0) };
        let &(x, z) = blocks
            .iter()
            .min_by(|a, b| {
                let d = |p: &(usize, usize)| (p.0 as f32 - mx).powi(2) + (p.1 as f32 - mz).powi(2);
                d(a).total_cmp(&d(b))
            })
            .expect("a layer with blocks");
        [x as u16, y as u16, z as u16]
    }

    /// How far it reaches from its anchor across the land, in blocks.
    pub fn reach(&self) -> i32 {
        let [w, _, l] = self.size.map(|s| s as i32);
        let [ax, _, az] = self.anchor.map(|a| a as i32);
        ax.max(w - 1 - ax).max(az).max(l - 1 - az)
    }

    /// Each column of the box seen from above: the palette index of its
    /// topmost block and that block's height in the box (0 for none).
    pub fn top_view(&self) -> Vec<(u16, u16)> {
        let [w, h, l] = self.size.map(|s| s as usize);
        let mut out = Vec::with_capacity(w * l);
        for z in 0..l {
            for x in 0..w {
                let top = (0..h).rev().find(|&y| self.at(x, y, z) != 0);
                out.push(top.map_or((0, 0), |y| (self.at(x, y, z), y as u16)));
            }
        }
        out
    }
}

/// Objects' blocks in files: zstd-compressed little-endian `u16`s, in
/// base 64.
mod packed {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(blocks: &[u16], s: S) -> Result<S::Ok, S::Error> {
        let bytes: Vec<u8> = blocks.iter().flat_map(|b| b.to_le_bytes()).collect();
        let packed = zstd::bulk::compress(&bytes, 9).map_err(serde::ser::Error::custom)?;
        s.serialize_str(&super::base64::encode(&packed))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u16>, D::Error> {
        let text = String::deserialize(d)?;
        let packed = super::base64::decode(&text).ok_or_else(|| serde::de::Error::custom("the blocks of an object are not in base 64"))?;
        let bytes = zstd::stream::decode_all(&packed[..]).map_err(serde::de::Error::custom)?;
        Ok(bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
    }
}

mod base64 {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

    pub fn encode(bytes: &[u8]) -> String {
        let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
        for chunk in bytes.chunks(3) {
            let b = [chunk[0], chunk.get(1).copied().unwrap_or(0), chunk.get(2).copied().unwrap_or(0)];
            let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
            for k in 0..4 {
                if k <= chunk.len() {
                    out.push(ALPHABET[(n >> (18 - 6 * k) & 63) as usize] as char);
                } else {
                    out.push('=');
                }
            }
        }
        out
    }

    pub fn decode(text: &str) -> Option<Vec<u8>> {
        let digits: Vec<u8> = text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=').collect();
        let mut out = Vec::with_capacity(digits.len() * 3 / 4);
        for chunk in digits.chunks(4) {
            let mut n = 0u32;
            for (k, c) in chunk.iter().enumerate() {
                let v = ALPHABET.iter().position(|a| a == c)? as u32;
                n |= v << (18 - 6 * k);
            }
            let bytes = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
            if chunk.len() < 2 {
                return None;
            }
            out.extend_from_slice(&bytes[..chunk.len() - 1]);
        }
        Some(out)
    }
}

/// What a painted object layer stands on the land: objects of a library,
/// spaced, turned, on soil or under water.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ObjectLayerDef {
    pub name: String,
    /// Map colour.
    pub color: [u8; 3],
    /// Its objects, by index into [`Registry::objects`], with their
    /// weights: one is picked for each spot.
    pub objects: Vec<(u32, u16)>,
    /// The side of the squares of its grid, in blocks: at most one object
    /// stands in each.
    pub spacing: u16,
    /// Share of the squares where one stands, `0..=1`.
    pub density: f32,
    /// Turned a random quarter turn.
    pub rotate: bool,
    /// Mirrored, half of them.
    pub mirror: bool,
    /// Only on soil — grass, dirt, podzol, moss, mud…
    pub soil_only: bool,
    /// Under water rather than in the open air.
    pub underwater: bool,
    /// How many blocks lower than the ground's top its anchor goes (rocks
    /// bedded in the ground).
    pub sink: i16,
    /// Its blocks replace the ground they reach; otherwise they only take
    /// the place of air, water and plants.
    pub replace: bool,
    /// Removed: no longer offered, and nothing stands where it was painted.
    #[serde(default)]
    pub removed: bool,
}

impl ObjectLayerDef {
    /// A layer of the given objects, spaced so that they hardly touch.
    pub fn new(name: &str, objects: &[u32], reg: &Registry) -> Self {
        let reach = objects.iter().filter_map(|&k| reg.objects.get(k as usize)).map(ObjectDef::reach).max().unwrap_or(4);
        Self {
            name: name.to_owned(),
            color: [58, 112, 52],
            objects: objects.iter().map(|&k| (k, 1)).collect(),
            spacing: (2 * reach + 2).clamp(3, 256) as u16,
            density: 0.6,
            rotate: true,
            mirror: true,
            soil_only: true,
            underwater: false,
            sink: 0,
            replace: false,
            removed: false,
        }
    }

    /// Where an object of this layer, numbered `number`, may stand in
    /// square `(i, j)` of its grid, before the land there is asked: the
    /// spot, the object, its turn. `None` when the square stays empty.
    pub fn candidate(&self, seed: u64, number: u16, i: i32, j: i32) -> Option<Candidate> {
        if self.removed {
            return None;
        }
        let s = self.spacing.max(1) as i32;
        let salt = seed ^ (number as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x6f62_6a65_6374;
        let h = |k: u64| hash2(i, j, salt.wrapping_add(k.wrapping_mul(0x5851_f42d_4c95_7f2d)));
        let density = self.density.clamp(0.0, 1.0);
        if density < 1.0 && h(0) as f32 >= density * u32::MAX as f32 {
            return None;
        }
        let total: u32 = self.objects.iter().map(|o| o.1 as u32).sum();
        if total == 0 {
            return None;
        }
        let mut r = h(1) % total;
        let object = self.objects.iter().find(|o| {
            if r < o.1 as u32 {
                true
            } else {
                r -= o.1 as u32;
                false
            }
        })?;
        // Kept off the edges of the square: two objects are never closer
        // than a third of the spacing.
        let margin = s / 6;
        let room = (s - 2 * margin).max(1) as u32;
        Some(Candidate {
            x: i * s + margin + (h(2) % room) as i32,
            z: j * s + margin + (h(3) % room) as i32,
            object: object.0,
            turn: if self.rotate { (h(4) % 4) as u8 } else { 0 },
            mirror: self.mirror && h(5) & 1 == 1,
        })
    }
}

/// Where an object of a layer may stand, before the land is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub x: i32,
    pub z: i32,
    pub object: u32,
    pub turn: u8,
    pub mirror: bool,
}

/// An object standing on the map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    /// The number of its layer.
    pub layer: u16,
    /// Index into [`Registry::objects`].
    pub object: u32,
    /// Where its anchor block goes.
    pub x: i32,
    pub y: i32,
    pub z: i32,
    /// Quarter turns clockwise seen from above, after mirroring along x.
    pub turn: u8,
    pub mirror: bool,
}

/// `(dx, dz)` turned `turn` quarter turns clockwise seen from above (east
/// goes south).
#[inline]
pub fn turn_xz(dx: i32, dz: i32, turn: u8) -> (i32, i32) {
    match turn & 3 {
        0 => (dx, dz),
        1 => (-dz, dx),
        2 => (-dx, -dz),
        _ => (dz, -dx),
    }
}

impl Placement {
    /// Where block `(bx, by, bz)` of its object's box goes in the world.
    #[inline]
    pub fn world(&self, def: &ObjectDef, bx: i32, by: i32, bz: i32) -> (i32, i32, i32) {
        let mut dx = bx - def.anchor[0] as i32;
        if self.mirror {
            dx = -dx;
        }
        let (rx, rz) = turn_xz(dx, bz - def.anchor[2] as i32, self.turn);
        (self.x + rx, self.y + by - def.anchor[1] as i32, self.z + rz)
    }

    /// The column of its box that lands on world column `(x, z)`, if any.
    #[inline]
    pub fn box_column(&self, def: &ObjectDef, x: i32, z: i32) -> Option<(usize, usize)> {
        let (mut dx, dz) = turn_xz(x - self.x, z - self.z, (4 - (self.turn & 3)) & 3);
        if self.mirror {
            dx = -dx;
        }
        let (bx, bz) = (dx + def.anchor[0] as i32, dz + def.anchor[2] as i32);
        (bx >= 0 && bz >= 0 && bx < def.size[0] as i32 && bz < def.size[2] as i32).then_some((bx as usize, bz as usize))
    }

    /// The columns its box may cover.
    pub fn bounds(&self, def: &ObjectDef) -> BlockRect {
        let r = def.reach();
        BlockRect::new(self.x - r, self.z - r, self.x + r + 1, self.z + r + 1)
    }

    /// The block state `state` of its object as it stands: mirrored and
    /// turned like the object.
    pub fn state(&self, state: &str) -> String {
        transform_state(state, self.turn, self.mirror)
    }
}

const SIDES: [&str; 4] = ["north", "east", "south", "west"];

/// A side of the compass turned clockwise `turn` quarter turns after a
/// mirror along x (east and west swapped). Other words come back as they
/// are.
fn turn_side(side: &str, turn: u8, mirror: bool) -> &str {
    let Some(mut k) = SIDES.iter().position(|s| *s == side) else { return side };
    if mirror && k % 2 == 1 {
        k = 4 - k;
    }
    SIDES[(k + turn as usize) % 4]
}

/// A rail's shape (`north_east`, `ascending_west`…) turned.
fn turn_rail(shape: &str, turn: u8, mirror: bool) -> String {
    if let Some(side) = shape.strip_prefix("ascending_") {
        return format!("ascending_{}", turn_side(side, turn, mirror));
    }
    let Some((a, b)) = shape.split_once('_') else { return shape.to_owned() };
    let (a, b) = (turn_side(a, turn, mirror), turn_side(b, turn, mirror));
    // Rails are named north or south first, then east or west.
    let first = |s: &str| s == "north" || s == "south";
    match (first(a), first(b)) {
        (true, true) | (false, false) => if a == "north" || b == "north" { "north_south".into() } else { "east_west".into() },
        (true, false) => format!("{a}_{b}"),
        (false, true) => format!("{b}_{a}"),
    }
}

/// A block state as it stands in an object mirrored (east and west
/// swapped) and then turned `turn` quarter turns clockwise: its facing,
/// axis, rotation, sides and shapes follow.
pub fn transform_state(state: &str, turn: u8, mirror: bool) -> String {
    let turn = turn & 3;
    let Some((name, rest)) = state.split_once('[') else { return state.to_owned() };
    if turn == 0 && !mirror {
        return state.to_owned();
    }
    let props: Vec<(&str, &str)> = rest.trim_end_matches(']').split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim(), v.trim())).collect();
    // Values of the sides move with them.
    let side_value = |side: &str| props.iter().find(|(k, _)| *k == side).map(|p| p.1);
    let mut out = Vec::with_capacity(props.len());
    for &(k, v) in &props {
        let value: String = match k {
            "facing" | "horizontal_facing" => turn_side(v, turn, mirror).to_owned(),
            "axis" if turn % 2 == 1 => match v {
                "x" => "z".into(),
                "z" => "x".into(),
                _ => v.into(),
            },
            "rotation" => match v.parse::<u8>() {
                Ok(r) => {
                    let r = if mirror { (16 - r) % 16 } else { r };
                    ((r + 4 * turn) % 16).to_string()
                }
                Err(_) => v.into(),
            },
            "north" | "east" | "south" | "west" => {
                // This side now shows what the side that turned into it had.
                let from = SIDES.iter().copied().find(|s| turn_side(s, turn, mirror) == k).unwrap_or(k);
                side_value(from).unwrap_or(v).to_owned()
            }
            "shape" if v.contains("left") || v.contains("right") => {
                if mirror {
                    if v.contains("left") { v.replace("left", "right") } else { v.replace("right", "left") }
                } else {
                    v.into()
                }
            }
            "shape" => turn_rail(v, turn, mirror),
            "hinge" | "type" if mirror => match v {
                "left" => "right".into(),
                "right" => "left".into(),
                _ => v.into(),
            },
            "orientation" => match v.split_once('_') {
                Some((a, b)) => format!("{}_{}", turn_side(a, turn, mirror), turn_side(b, turn, mirror)),
                None => v.into(),
            },
            _ => v.into(),
        };
        out.push(format!("{k}={value}"));
    }
    format!("{name}[{}]", out.join(","))
}

/// The objects standing with their anchor in tile `pos`, whose stack is
/// `stack`: those of every layer painted there whose spot has the layer
/// on its top, on soil or under water as the layer wants (never in lava).
/// In a fixed order: by layer, then row by row of their grid.
pub fn tile_placements(reg: &Registry, mat: &Materializer, seed: u64, lava: bool, stack: &TileStack, pos: TilePos) -> Vec<Placement> {
    let mut out = Vec::new();
    // The layers painted on the tile at all.
    let mut painted: Vec<u16> = Vec::new();
    for (_, _, tile) in stack.layers() {
        match tile.objects.uniform_value() {
            Some(0) => {}
            Some(v) => painted.push(v),
            None => painted.extend(tile.objects.dense().iter().copied().filter(|&v| v != 0)),
        }
        painted.sort_unstable();
        painted.dedup();
    }
    if painted.is_empty() {
        return out;
    }
    let (ox, oz) = pos.origin();
    let (mut spans, mut fluids) = (Vec::new(), Vec::new());
    for n in painted {
        let Some(layer) = reg.object_layer(n) else { continue };
        let s = layer.spacing.max(1) as i32;
        let (i0, i1) = (ox.div_euclid(s), (ox + 127).div_euclid(s));
        let (j0, j1) = (oz.div_euclid(s), (oz + 127).div_euclid(s));
        for j in j0..=j1 {
            for i in i0..=i1 {
                let Some(c) = layer.candidate(seed, n, i, j) else { continue };
                if TilePos::containing(c.x, c.z) != pos || reg.objects.get(c.object as usize).is_none() {
                    continue;
                }
                let cell = local_index(c.x, c.z);
                stack.spans(cell, &mut spans);
                let Some(top) = spans.last().copied() else { continue };
                if stack.surface_objects(cell, &top) != n {
                    continue;
                }
                stack.fluids(cell, &spans, &mut fluids);
                let wet = fluids.iter().any(|f| f.0 == top.y1);
                if wet != layer.underwater || (wet && lava) {
                    continue;
                }
                if layer.soil_only && !is_soil(&reg.block(mat.block(&top, c.x, top.y1 - 1, c.z, wet)).state) {
                    continue;
                }
                out.push(Placement {
                    layer: n,
                    object: c.object,
                    x: c.x,
                    y: top.y1 - layer.sink as i32,
                    z: c.z,
                    turn: c.turn,
                    mirror: c.mirror,
                });
            }
        }
    }
    out
}

/// The farthest any object of the registry reaches from its anchor.
pub fn max_reach(reg: &Registry) -> i32 {
    reg.objects.iter().map(ObjectDef::reach).max().unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::{GenParams, generate};
    use crate::history::History;

    /// A small tree: a trunk three high on a block of dirt, leaves around
    /// its top, a vine on its east side.
    pub(crate) fn tree() -> ObjectDef {
        let (w, h, l) = (5usize, 6usize, 5usize);
        let palette = vec![
            "minecraft:air".to_owned(),
            "minecraft:dirt".to_owned(),
            "minecraft:oak_log[axis=y]".to_owned(),
            "minecraft:oak_leaves[distance=1,persistent=true]".to_owned(),
            "minecraft:vine[east=false,north=false,south=false,up=false,west=true]".to_owned(),
        ];
        let mut blocks = vec![0u16; w * h * l];
        let at = |x: usize, y: usize, z: usize| x + z * w + y * w * l;
        blocks[at(2, 0, 2)] = 1;
        for y in 1..4 {
            blocks[at(2, y, 2)] = 2;
        }
        for y in 3..6 {
            for z in 1..4 {
                for x in 1..4 {
                    if blocks[at(x, y, z)] == 0 {
                        blocks[at(x, y, z)] = 3;
                    }
                }
            }
        }
        blocks[at(3, 2, 2)] = 4;
        ObjectDef::new("Little oak", [w as u16, h as u16, l as u16], palette, blocks)
    }

    #[test]
    fn objects_stand_on_their_trunk() {
        let t = tree();
        // The dirt it was copied with goes into the ground.
        assert_eq!(t.anchor, [2, 1, 2]);
        assert_eq!(t.palette[0], "minecraft:air");
        assert_eq!(t.count(), 1 + 3 + 26 + 1);
        assert_eq!(t.reach(), 2);
        // Nothing-states fold into 0, unused entries go.
        let o = ObjectDef::new("x", [2, 1, 1], vec!["minecraft:stone".into(), "minecraft:structure_void".into(), "minecraft:sand".into()], vec![1, 0]);
        assert_eq!(o.palette, vec!["minecraft:air".to_owned(), "minecraft:stone".to_owned()]);
        assert_eq!(o.blocks, vec![0, 1]);
        assert_eq!(o.anchor, [1, 0, 0]);
    }

    #[test]
    fn objects_round_trip_through_json() {
        let t = tree();
        let json = serde_json::to_string(&t).unwrap();
        assert!(json.len() < 600, "{} bytes", json.len());
        let back: ObjectDef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, t);
        for bytes in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0, 255, 7, 128, 64]] {
            assert_eq!(base64::decode(&base64::encode(bytes)).unwrap(), bytes);
        }
        assert_eq!(base64::encode(b"Man"), "TWFu");
        assert_eq!(base64::encode(b"Ma"), "TWE=");
    }

    #[test]
    fn states_turn_with_their_object() {
        assert_eq!(transform_state("minecraft:oak_log[axis=x]", 1, false), "minecraft:oak_log[axis=z]");
        assert_eq!(transform_state("minecraft:oak_log[axis=y]", 1, false), "minecraft:oak_log[axis=y]");
        assert_eq!(transform_state("minecraft:oak_stairs[facing=north,half=bottom,shape=inner_left]", 1, false), "minecraft:oak_stairs[facing=east,half=bottom,shape=inner_left]");
        assert_eq!(transform_state("minecraft:oak_stairs[facing=east,shape=inner_left]", 0, true), "minecraft:oak_stairs[facing=west,shape=inner_right]");
        assert_eq!(transform_state("minecraft:oak_sign[rotation=0]", 1, false), "minecraft:oak_sign[rotation=4]");
        assert_eq!(transform_state("minecraft:oak_sign[rotation=4]", 0, true), "minecraft:oak_sign[rotation=12]");
        // A vine on the west of its block is on its north after a turn.
        assert_eq!(
            transform_state("minecraft:vine[east=false,north=false,south=false,up=false,west=true]", 1, false),
            "minecraft:vine[east=false,north=true,south=false,up=false,west=false]"
        );
        assert_eq!(transform_state("minecraft:rail[shape=north_east]", 1, false), "minecraft:rail[shape=south_east]");
        assert_eq!(transform_state("minecraft:rail[shape=north_south]", 1, false), "minecraft:rail[shape=east_west]");
        assert_eq!(transform_state("minecraft:rail[shape=ascending_north]", 2, false), "minecraft:rail[shape=ascending_south]");
        assert_eq!(transform_state("minecraft:stone", 3, true), "minecraft:stone");
        // Four turns, or two mirrors, give the block back.
        let s = "minecraft:vine[east=true,north=false,south=true,up=false,west=false]";
        let mut t = s.to_owned();
        for _ in 0..4 {
            t = transform_state(&t, 1, false);
        }
        assert_eq!(t, s);
        assert_eq!(transform_state(&transform_state(s, 0, true), 0, true), s);
    }

    #[test]
    fn placements_turn_their_blocks_around_the_anchor() {
        let t = tree();
        let p = Placement { layer: 1, object: 0, x: 100, y: 64, z: 50, turn: 1, mirror: false };
        // The anchor stays put; the vine east of the trunk goes south.
        assert_eq!(p.world(&t, 2, 1, 2), (100, 64, 50));
        assert_eq!(p.world(&t, 3, 2, 2), (100, 65, 51));
        assert_eq!(p.box_column(&t, 100, 51), Some((3, 2)));
        let m = Placement { mirror: true, turn: 0, ..p };
        assert_eq!(m.world(&t, 3, 2, 2), (99, 65, 50));
        assert_eq!(m.box_column(&t, 99, 50), Some((3, 2)));
        for turn in 0..4 {
            for mirror in [false, true] {
                let q = Placement { turn, mirror, ..p };
                for (bx, bz) in [(0, 0), (4, 1), (2, 3), (1, 4)] {
                    let (x, _, z) = q.world(&t, bx, 0, bz);
                    assert_eq!(q.box_column(&t, x, z), Some((bx as usize, bz as usize)), "turn {turn} mirror {mirror}");
                    assert!(x >= q.bounds(&t).x0 && x < q.bounds(&t).x1 && z >= q.bounds(&t).z0 && z < q.bounds(&t).z1);
                }
            }
        }
    }

    #[test]
    fn layers_stand_their_objects_where_they_are_painted() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, seed: 5, ..GenParams::default() });
        w.registry.objects.push(tree());
        let wide = crate::Brush { radius: 60.0, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() };
        let mut layer = ObjectLayerDef::new("Oaks", &[0], &w.registry);
        layer.soil_only = false;
        layer.density = 1.0;
        assert_eq!(layer.spacing, 6);
        let n = w.registry.add_object_layer(layer);
        let mut h = History::default();
        h.begin("objects", 0);
        crate::ops::set_height(&mut h.edit(&mut w, 0), &wide, 0.0, 0.0, 80.0, crate::ops::Surface::Top);
        crate::ops::paint_objects(&mut h.edit(&mut w, 0), &crate::Brush { radius: 40.0, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() }, 0.0, 0.0, n, 1);
        h.commit();
        let dim = &w.dims[0];
        let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
        let mut all = Vec::new();
        for pos in [TilePos::new(-1, -1), TilePos::new(0, -1), TilePos::new(-1, 0), TilePos::new(0, 0)] {
            let stack = TileStack::new(dim, pos, true).unwrap();
            let here = tile_placements(&w.registry, &mat, 1, false, &stack, pos);
            assert!(here.iter().all(|p| TilePos::containing(p.x, p.z) == pos));
            // The same every time it is asked.
            assert_eq!(here, tile_placements(&w.registry, &mat, 1, false, &stack, pos));
            all.extend(here);
        }
        // About one in each 6×6 square of the painted disc, none outside,
        // never two too close, every quarter turn.
        assert!(all.len() > 100 && all.len() < 160, "{}", all.len());
        assert!(all.iter().all(|p| p.x * p.x + p.z * p.z <= 41 * 41));
        for (k, a) in all.iter().enumerate() {
            for b in &all[k + 1..] {
                assert!((a.x - b.x).abs().max((a.z - b.z).abs()) >= 2, "{a:?} {b:?}");
            }
        }
        let turns: std::collections::BTreeSet<u8> = all.iter().map(|p| p.turn).collect();
        assert_eq!(turns.len(), 4);
        // On the ground: the anchor in the first block above it.
        let p = all[0];
        let stack = TileStack::new(dim, TilePos::containing(p.x, p.z), true).unwrap();
        let mut spans = Vec::new();
        stack.spans(local_index(p.x, p.z), &mut spans);
        assert_eq!(p.y, spans.last().unwrap().y1);
    }
}
