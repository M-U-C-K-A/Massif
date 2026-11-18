//! Laying a structure's blocks: its pieces placed one after another, their
//! templates' blocks turned, processed and set, their entities and block
//! entities kept, the features among them grown.

use crate::geom::{BBox, place, place_exact};
use crate::ground::Ground;
use crate::jigsaw::Placed;
use crate::library::Library;
use crate::nbt::{self, Compound};
use crate::pool::{Element, Projection};
use crate::processor::{self, BlockInfo, Ctx, Heightmap, Processor};
use crate::rng::Rng;
use fastnbt::Value;
use massif_core::structures::{BlockEntity, Entity, Piece, Section, Structure};
use std::collections::HashMap;
use std::sync::Arc;

/// A data marker of a template (a structure block in data mode), where the
/// game's code puts a chest, a mob: kept for the structures built by code.
#[derive(Debug, Clone)]
pub struct Marker {
    pub pos: [i32; 3],
    pub metadata: String,
}

/// The blocks of a structure as they are laid.
pub struct Builder<'a> {
    pub ground: &'a dyn Ground,
    pub seed: u64,
    sections: HashMap<[i32; 3], Box<[u32; 4096]>>,
    states: Vec<Arc<str>>,
    ids: HashMap<Arc<str>, u32>,
    block_entities: HashMap<[i32; 3], Compound>,
    entities: Vec<([f64; 3], Compound)>,
    pub markers: Vec<Marker>,
    pub pieces: Vec<Piece>,
}

impl<'a> Builder<'a> {
    pub fn new(ground: &'a dyn Ground, seed: u64) -> Self {
        Self {
            ground,
            seed,
            sections: HashMap::new(),
            states: Vec::new(),
            ids: HashMap::new(),
            block_entities: HashMap::new(),
            entities: Vec::new(),
            markers: Vec::new(),
            pieces: Vec::new(),
        }
    }

    fn intern(&mut self, state: &str) -> u32 {
        if let Some(&k) = self.ids.get(state) {
            return k;
        }
        let s: Arc<str> = state.into();
        self.states.push(s.clone());
        self.ids.insert(s, self.states.len() as u32 - 1);
        self.states.len() as u32 - 1
    }

    /// Set the block at `p`, with its block entity's data (or none: what
    /// was there goes).
    pub fn set(&mut self, p: [i32; 3], state: &str, data: Option<Compound>) {
        if p[1] < self.ground.min_y() || p[1] >= self.ground.max_y() {
            return;
        }
        let k = self.intern(state) + 1;
        let section = self.sections.entry([p[0] >> 4, p[1] >> 4, p[2] >> 4]).or_insert_with(|| Box::new([0; 4096]));
        section[Section::index(p[0], p[1], p[2]) as usize] = k;
        match data {
            Some(d) => {
                self.block_entities.insert(p, d);
            }
            None => {
                self.block_entities.remove(&p);
            }
        }
    }

    /// The block the structure set at `p`, if any.
    pub fn get(&self, p: [i32; 3]) -> Option<&str> {
        let section = self.sections.get(&[p[0] >> 4, p[1] >> 4, p[2] >> 4])?;
        match section[Section::index(p[0], p[1], p[2]) as usize] {
            0 => None,
            k => Some(&self.states[k as usize - 1]),
        }
    }

    /// The block at `p`: the structure's, or the land's.
    pub fn block(&self, p: [i32; 3]) -> String {
        match self.get(p) {
            Some(s) => s.to_owned(),
            None => self.ground.block(p[0], p[1], p[2]).state().to_owned(),
        }
    }

    pub fn is_air(&self, p: [i32; 3]) -> bool {
        match self.get(p) {
            Some(s) => is_air(s),
            None => self.ground.is_air(p[0], p[1], p[2]),
        }
    }

    /// Whether the block at `p` holds others up from below: a whole block
    /// of the structure's, or the land's solid ground.
    pub fn is_sturdy(&self, p: [i32; 3]) -> bool {
        match self.get(p) {
            Some(s) => is_sturdy(s),
            None => self.ground.block(p[0], p[1], p[2]).is_solid(),
        }
    }

    pub fn block_entity(&mut self, p: [i32; 3]) -> Option<&mut Compound> {
        self.block_entities.get_mut(&p)
    }

    pub fn add_entity(&mut self, pos: [f64; 3], data: Compound) {
        self.entities.push((pos, data));
    }

    /// The structure laid, named `name`, of the game's kind `kind`.
    pub fn finish(self, id: u32, name: &str, kind: &str, waterlog: bool, data_version: i32) -> Structure {
        let mut sections: Vec<Section> = self
            .sections
            .into_iter()
            .map(|(pos, cells)| Section { pos, blocks: cells.iter().enumerate().filter(|(_, k)| **k != 0).map(|(i, k)| (i as u16, (*k - 1) as u16)).collect() })
            .collect();
        sections.sort_by_key(|s| (s.pos[0], s.pos[2], s.pos[1]));
        let mut block_entities: Vec<BlockEntity> = self
            .block_entities
            .into_iter()
            .map(|(pos, mut data)| {
                for k in ["x", "y", "z"] {
                    data.remove(k);
                }
                BlockEntity { pos, nbt: nbt::to_bytes(&data) }
            })
            .collect();
        block_entities.sort_by_key(|b| b.pos);
        let mut rng = Rng::new(self.seed ^ 0x7575_6964);
        let entities = self
            .entities
            .into_iter()
            .map(|(pos, mut data)| {
                data.insert("Pos".into(), nbt::doubles(pos));
                let uuid: [i32; 4] = std::array::from_fn(|_| rng.next_u64() as i32);
                data.insert("UUID".into(), Value::IntArray(fastnbt::IntArray::new(uuid.to_vec())));
                Entity { pos, nbt: nbt::to_bytes(&data) }
            })
            .collect();
        let mut s = Structure {
            id,
            name: name.to_owned(),
            kind: kind.to_owned(),
            seed: self.seed,
            palette: self.states.iter().map(|s| s.to_string()).collect(),
            sections,
            block_entities,
            entities,
            pieces: self.pieces,
            min: [0; 3],
            max: [0; 3],
            waterlog,
            biome: None,
            data_version,
        };
        s.finish();
        s
    }
}

pub fn is_air(state: &str) -> bool {
    matches!(nbt::block_name(state), "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air")
}

/// Whether a block is whole, from its name: what plants stand on.
pub fn is_sturdy(state: &str) -> bool {
    let name = nbt::block_name(state);
    if is_air(name) || name.contains("water") || name.contains("lava") {
        return false;
    }
    let half = nbt::property(state, "type").is_some_and(|t| t == "bottom" || t == "top");
    !half
        && ![
            "_stairs", "_slab", "_wall", "_fence", "_bars", "chain", "_button", "lantern", "torch", "_door", "_trapdoor", "_pane", "_carpet", "_sign", "ladder", "vine", "flower", "grass", "fern", "sapling", "lever", "rail", "_bed", "candle", "sculk_vein", "lichen", "sculk_sensor", "sculk_shrieker", "skull", "_head", "banner", "pot", "chest", "campfire", "snow[",
        ]
        .iter()
        .any(|w| state.contains(w))
}

/// The processors a piece is placed with, as the game sets them up.
fn processors_of(processors: &Arc<Vec<Processor>>, projection: Projection, legacy: bool) -> Vec<Processor> {
    let mut list = Vec::with_capacity(processors.len() + 3);
    if !legacy {
        list.push(Processor::Ignore(vec!["minecraft:structure_block".into()]));
    }
    list.push(Processor::JigsawReplacement);
    list.extend(processors.iter().cloned());
    if projection == Projection::TerrainMatching {
        list.push(Processor::Gravity { heightmap: Heightmap::WorldSurface, offset: -1 });
    }
    if legacy {
        list.push(Processor::Ignore(vec!["minecraft:structure_block".into(), "minecraft:air".into()]));
    }
    list
}

/// Place a template as a piece: its blocks moved to `origin` and turned,
/// through `list`; its entities too unless `no_entities`. Its data markers
/// are kept in the builder.
#[allow(clippy::too_many_arguments)]
pub fn place_template(lib: &Library, b: &mut Builder, location: &str, origin: [i32; 3], turn: u8, list: &[Processor], center: [i32; 3], no_entities: bool) -> Option<BBox> {
    let t = lib.template(location)?;
    let mut rng = Rng::at(b.seed ^ 0x7061_6c, origin);
    let palette = &t.palettes[if t.palettes.len() > 1 { rng.below(t.palettes.len() as u32) as usize } else { 0 }];
    let ctx = Ctx { ground: b.ground, seed: b.seed, center };
    let mut locals = Vec::with_capacity(t.blocks.len());
    let mut processed = Vec::with_capacity(t.blocks.len());
    for block in &t.blocks {
        let Some(state) = palette.get(block.state as usize) else { continue };
        let pos = place(block.pos, origin, turn);
        if nbt::block_name(state) == "minecraft:structure_block" {
            let data = block.nbt.as_ref();
            if data.and_then(|d| nbt::string(d, "mode")).is_some_and(|m| m.eq_ignore_ascii_case("data")) {
                b.markers.push(Marker { pos, metadata: data.and_then(|d| nbt::string(d, "metadata")).unwrap_or("").to_owned() });
            }
        }
        let mut info = Some(BlockInfo { pos, state: state.as_str().into(), nbt: block.nbt.clone() });
        for p in list {
            info = match info {
                Some(i) => p.process(&ctx, block.pos, i),
                None => break,
            };
        }
        if let Some(info) = info {
            locals.push(block.pos);
            processed.push(info);
        }
    }
    processor::finalize(list, &ctx, origin, &locals, &mut processed);
    for info in processed {
        let name = nbt::block_name(&info.state);
        if name == "minecraft:structure_void" || name == "minecraft:structure_block" {
            continue;
        }
        let state = massif_core::objects::transform_state(&info.state, turn, false);
        b.set(info.pos, &state, info.nbt);
    }
    if !no_entities {
        for e in &t.entities {
            let pos = place_exact(e.pos, origin, turn);
            let mut data = e.nbt.clone();
            data.remove("UUID");
            if let Some(Value::List(r)) = data.get_mut("Rotation")
                && let Some(Value::Float(yaw)) = r.first_mut()
            {
                *yaw = (*yaw + 90.0 * turn as f32).rem_euclid(360.0);
            }
            b.add_entity(pos, data);
        }
    }
    Some(t.bbox(origin, turn))
}

/// Place a piece of a jigsaw structure.
pub fn place_piece(lib: &Library, b: &mut Builder, piece: &Placed, center: [i32; 3]) {
    place_element(lib, b, &piece.element, piece, center);
}

fn place_element(lib: &Library, b: &mut Builder, element: &Element, piece: &Placed, center: [i32; 3]) {
    match element {
        Element::Single { location, processors, processors_name, projection, legacy } => {
            let list = processors_of(processors, *projection, *legacy);
            if place_template(lib, b, location, piece.origin, piece.turn, &list, center, false).is_some() {
                b.pieces.push(Piece {
                    min: piece.bbox.min,
                    max: piece.bbox.max,
                    template: location.clone(),
                    origin: piece.origin,
                    turn: piece.turn,
                    depth: piece.depth as u16,
                    ground_delta: piece.ground_delta,
                    rigid: *projection == Projection::Rigid,
                    processors: processors_name.clone(),
                });
            }
        }
        Element::List { elements, .. } => {
            for e in elements {
                place_element(lib, b, e, piece, center);
            }
        }
        Element::Feature { feature, .. } => {
            let mut rng = Rng::at(b.seed ^ 0x6665_6174, piece.origin);
            if !crate::features::place(lib, b, feature, piece.origin, &mut rng) {
                lib.unknown(format!("feature {feature}"));
            }
        }
        Element::Empty => {}
    }
}

/// The columns to flatten under a structure's pieces that stand on the
/// land, and the height to give each (the first free block above it):
/// under each piece its floor, then a slope of `margin` blocks down (or up)
/// to the land around. Pieces that follow the land (paths) leave it as it
/// is.
pub fn flatten_under(pieces: &[Placed], ground: &dyn Ground, margin: i32) -> HashMap<(i32, i32), i32> {
    // Each column: the weight of the nearest piece's floor and that floor.
    let mut best: HashMap<(i32, i32), (f32, i32)> = HashMap::new();
    for p in pieces {
        if p.element.projection() != Projection::Rigid || matches!(*p.element, Element::Feature { .. } | Element::Empty) || p.ground_delta != 1 {
            continue;
        }
        let floor = p.bbox.min[1] + p.ground_delta;
        for z in p.bbox.min[2] - margin..=p.bbox.max[2] + margin {
            for x in p.bbox.min[0] - margin..=p.bbox.max[0] + margin {
                let dx = (p.bbox.min[0] - x).max(x - p.bbox.max[0]).max(0);
                let dz = (p.bbox.min[2] - z).max(z - p.bbox.max[2]).max(0);
                let d = ((dx * dx + dz * dz) as f32).sqrt();
                let w = if d <= 0.0 { 1.0 } else { 1.0 - d / (margin as f32 + 1.0) };
                if w <= 0.0 {
                    continue;
                }
                let e = best.entry((x, z)).or_insert((0.0, floor));
                if w > e.0 {
                    *e = (w, floor);
                }
            }
        }
    }
    best.into_iter()
        .filter_map(|((x, z), (w, floor))| {
            let here = ground.floor(x, z);
            let h = if w >= 1.0 { floor } else { (here as f32 + (floor - here) as f32 * w).round() as i32 };
            (h != here).then_some(((x, z), h))
        })
        .collect()
}

/// Carve the box of each piece that stands as it is, from its floor up, as
/// the game carves an Ancient City's halls out of the deep dark: the land
/// there becomes air before the pieces are laid.
pub fn carve_boxes(b: &mut Builder, pieces: &[Placed]) {
    for p in pieces {
        if p.element.projection() != Projection::Rigid || !matches!(*p.element, Element::Single { .. } | Element::List { .. }) {
            continue;
        }
        let floor = (p.bbox.min[1] + p.ground_delta).max(p.bbox.min[1]);
        for y in floor..=p.bbox.max[1] {
            for z in p.bbox.min[2]..=p.bbox.max[2] {
                for x in p.bbox.min[0]..=p.bbox.max[0] {
                    if b.get([x, y, z]).is_none() && !b.ground.is_air(x, y, z) {
                        b.set([x, y, z], "minecraft:cave_air", None);
                    }
                }
            }
        }
    }
}

/// Wall the pieces in, as the game walls Trial Chambers in: where the land
/// around a piece's box is air or water (a cave going by), rock.
pub fn encase(b: &mut Builder, pieces: &[Placed]) {
    let boxes: Vec<BBox> = pieces.iter().filter(|p| matches!(*p.element, Element::Single { .. } | Element::List { .. })).map(|p| p.bbox).collect();
    for bx in &boxes {
        for y in bx.min[1] - 1..=bx.max[1] + 1 {
            for z in bx.min[2] - 1..=bx.max[2] + 1 {
                for x in bx.min[0] - 1..=bx.max[0] + 1 {
                    let shell = x < bx.min[0] || x > bx.max[0] || y < bx.min[1] || y > bx.max[1] || z < bx.min[2] || z > bx.max[2];
                    if !shell || boxes.iter().any(|o| o.contains([x, y, z])) || b.get([x, y, z]).is_some() {
                        continue;
                    }
                    if !b.ground.block(x, y, z).is_solid() {
                        b.set([x, y, z], if y < 0 { "minecraft:deepslate[axis=y]" } else { "minecraft:stone" }, None);
                    }
                }
            }
        }
    }
}

/// The middle of the bottom of the pieces' boxes, which some processors
/// measure from.
pub fn center_of(pieces: &[Placed]) -> [i32; 3] {
    let all = pieces.iter().map(|p| p.bbox).reduce(|a, b| a.union(&b));
    all.map_or([0; 3], |b| {
        let c = b.center();
        [c[0], b.min[1], c[2]]
    })
}

/// The block entity a loot chest gets: its loot table and the draw it is
/// filled from when first opened.
pub fn loot(id: &str, table: &str, rng: &mut Rng) -> Compound {
    let mut c = Compound::new();
    c.insert("id".into(), Value::String(id.into()));
    c.insert("LootTable".into(), Value::String(table.into()));
    c.insert("LootTableSeed".into(), Value::Long(rng.next_u64() as i64));
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;
    use crate::jigsaw::{assemble, tests::{village, village_spec}};

    #[test]
    fn a_village_is_laid_with_its_chests_and_villagers() {
        let lib = village();
        let ground = Flat { height: 64, water: None };
        let pieces = assemble(&lib, &village_spec(), [0, 0, 0], Some(1), &mut Rng::new(8), &ground);
        let mut b = Builder::new(&ground, 8);
        let center = center_of(&pieces);
        for p in &pieces {
            place_piece(&lib, &mut b, p, center);
        }
        let s = b.finish(1, "Village", "minecraft:village_plains", true, 4671);
        let houses = pieces.iter().filter(|p| p.element.location() == Some("t:house")).count();
        assert!(houses >= 2);
        // A chest and a villager in each house, their loot table kept.
        assert_eq!(s.block_entities.len(), houses);
        assert_eq!(s.entities.len(), houses);
        let chest = nbt::from_bytes(&s.block_entities[0].nbt).unwrap();
        assert_eq!(nbt::string(&chest, "LootTable"), Some("minecraft:chests/village/village_plains_house"));
        let villager = nbt::from_bytes(&s.entities[0].nbt).unwrap();
        assert_eq!(nbt::string(&villager, "id"), Some("minecraft:villager"));
        assert!(matches!(villager.get("UUID"), Some(Value::IntArray(_))));
        // Old-style pieces leave the land where their template has air, and
        // their jigsaws become what they leave (the house's door: air, so
        // nothing).
        assert!(!s.palette.iter().any(|p| p.starts_with("minecraft:jigsaw") || p.starts_with("minecraft:air") || p.starts_with("minecraft:structure")), "{:?}", s.palette);
        // Streets lie on the land: their path one below the surface.
        let mut column = Vec::new();
        let street = pieces.iter().find(|p| p.element.location() == Some("t:street")).unwrap();
        let c = street.bbox.center();
        s.column(c[0], c[2], &mut column);
        assert!(column.iter().any(|&(y, k)| y == 63 && s.palette[k as usize].starts_with("minecraft:dirt_path") || s.palette[k as usize].starts_with("minecraft:cobblestone")), "{column:?}");
        // The houses' floors are at the land's surface: nothing to flatten
        // on flat land.
        assert!(flatten_under(&pieces, &ground, 3).is_empty());
    }
}
