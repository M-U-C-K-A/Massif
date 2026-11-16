//! WorldPainter's layers as Massif has them: forests and the trees of the
//! tree pack become object layers, plant layers ground covers, Frost the
//! frost cover (snow on the ground, ice on the water).
//!
//! A layer is a key of a tile's `layerData` (a nibble or a byte a column)
//! or `bitLayerData` (a bit a column) map. Its strength in a column, 0 to
//! 255, becomes Massif's single layer of that column where a hash of the
//! column falls under it: a forest painted at half strength keeps about
//! that share of its columns, as WorldPainter plants about that share of
//! trees.

use crate::javaser::{Arena, Array, Value};
use massif_core::registry::{CoverDef, Registry};
use massif_core::treepack::TreeKind;

const LAYERS: &str = "org.pepsoft.worldpainter.layers.";
/// What the layers of WorldPainter's tree pack say they are.
const TREE_PACK: &str = " (WorldPainter tree pack)";

/// How a layer's strength turns into the share of its columns kept.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Curve {
    /// As much as its strength: objects and plants.
    Linear,
    /// The square of it, as WorldPainter's own forests plant their trees.
    Square,
}

/// What a WorldPainter layer becomes.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Conv {
    /// A ground cover, by number.
    Cover(u16),
    /// An object layer, by number.
    Objects(u16, Curve),
    /// Nothing at all in the column, down to the floor of the world.
    Void,
    /// Annotations: their colour, 1 to 15, a nibble a column.
    Annotations,
}

/// One layer of a tile: what it became, and its strength in each column
/// (0 to 255).
pub struct Painted {
    pub conv: Conv,
    pub strength: Vec<u8>,
    /// Painted before the others (0: Frost and Lava, whose snow, ice and
    /// lava take the place of plants), as the covers of a column are one.
    pub priority: u8,
}

/// What layer `key` (of class `class`) becomes in `reg`, adding what it
/// needs there; `None` for a layer Massif does not convert.
pub fn convert(arena: &Arena, key: Value, class: &str, reg: &mut Registry) -> Option<Conv> {
    let short = class.strip_prefix(LAYERS)?;
    let string = |name: &str| arena.field(key, name).and_then(|v| arena.string(v)).map(str::to_owned);
    let forest = |name: &str, kinds: &[(TreeKind, u16)], reg: &mut Registry| Some(Conv::Objects(massif_core::treepack::install_mix(name, kinds, reg), Curve::Square));
    match short {
        "Frost" => Some(Conv::Cover(frost_cover(reg))),
        "FloodWithLava" => Some(Conv::Cover(match reg.covers.iter().position(|c| c.lava && !c.removed) {
            Some(k) => k as u16 + 1,
            None => reg.add_cover(CoverDef::lava()),
        })),
        "DeciduousForest" => forest("Deciduous", &[(TreeKind::BigOak, 3), (TreeKind::Birch, 2)], reg),
        "PineForest" => forest("Pine", &[(TreeKind::TallSpruce, 2), (TreeKind::SlenderSpruce, 2), (TreeKind::Fir, 1)], reg),
        "Jungle" => forest("Jungle", &[(TreeKind::Rainforest, 3), (TreeKind::Bush, 1)], reg),
        "SwampLand" => forest("Swamp", &[(TreeKind::Willow, 1)], reg),
        "Bo2Layer" => {
            // The trees of the tree pack (which WorldPainter Modernised
            // describes as such, and older versions only name): Massif has
            // the same, made the same way.
            let name = string("name")?;
            match TreeKind::ALL.into_iter().find(|k| k.name() == name || string("description").is_some_and(|d| d.ends_with(TREE_PACK) && d.starts_with(k.name()))) {
                Some(kind) => Some(Conv::Objects(kind.install(reg), Curve::Linear)),
                // A layer of the user's own objects, which the world holds.
                None => object_layer(arena, key, &name, reg).map(|n| Conv::Objects(n, Curve::Square)),
            }
        }
        "plants.PlantLayer" => plant_cover(arena, key, reg).map(Conv::Cover),
        "Void" => Some(Conv::Void),
        "Annotations" => Some(Conv::Annotations),
        _ => None,
    }
}

/// A custom object layer as an object layer of Massif: its objects read
/// from what the world holds of them (Sponge schematics, MCEdit
/// schematics, structures, Bo2 objects), placed as WorldPainter places
/// them — about as many, standing on their offset, turned and mirrored if
/// they were. `None` when none of its objects could be read.
fn object_layer(arena: &Arena, key: Value, name: &str, reg: &mut Registry) -> Option<u16> {
    let provider = arena.field(key, "objectProvider")?;
    let items = arena.field(provider, "objects").and_then(|o| arena.list_items(o))?;
    let first = reg.objects.len() as u32;
    let mut weights = Vec::new();
    let (mut rotate, mut water, mut land) = (false, false, false);
    for item in items {
        let Some((mut def, shift)) = wp_object(arena, item) else { continue };
        let attributes: Vec<(String, Value)> = arena
            .field(item, "attributes")
            .and_then(|a| arena.map_entries(a))
            .unwrap_or_default()
            .into_iter()
            .filter_map(|(k, v)| Some((arena.string(k)?.to_owned(), v)))
            .collect();
        let attribute = |name: &str| attributes.iter().find(|a| a.0 == name).map(|a| a.1);
        let flag = |name: &str, default: bool| attribute(name).and_then(|v| arena.bool(v)).unwrap_or(default);
        // The offset: where the object stands from the spot it is placed
        // on, the block over the ground (WorldPainter's z is up). Objects
        // of blocks by their place stand on their place 0 without one.
        let offset = attribute("WPObject.offset").map(|o| {
            let c = |n: &str| arena.field(o, n).and_then(|v| arena.int(v)).unwrap_or(0);
            [c("x"), c("z"), c("y")]
        });
        if let Some(anchor) = anchor(offset, shift, def.size) {
            def.anchor = anchor;
        }
        rotate |= flag("WPObject.randomRotation", true);
        water |= flag("WPObject.spawnInWater", false);
        land |= flag("WPObject.spawnOnLand", true);
        let frequency = attribute("WPObject.frequency").and_then(|v| arena.int(v)).unwrap_or(100).clamp(1, u16::MAX as i64) as u16;
        def.source = format!("WorldPainter layer {name}");
        reg.objects.push(def);
        weights.push(frequency);
    }
    if weights.is_empty() {
        return None;
    }
    let indices: Vec<u32> = (first..first + weights.len() as u32).collect();
    let mut layer = massif_core::objects::ObjectLayerDef::new(name, &indices, reg);
    layer.objects = indices.iter().copied().zip(weights).collect();
    layer.rotate = rotate;
    layer.mirror = rotate;
    layer.underwater = water && !land;
    layer.soil_only = false;
    if let Some(c) = arena.field(key, "colour").and_then(|c| arena.int(c)).filter(|&c| c != 0) {
        layer.color = [(c >> 16) as u8, (c >> 8) as u8, c as u8];
    }
    // WorldPainter tries every column of its grid, and places an object
    // where a draw out of 64 × density falls under the square of the
    // strength: at full strength, 226 in 64 × density. Massif places one in
    // a share of the squares of its spacing, so as many stand.
    let int = |n: &str, default: i64| arena.field(key, n).and_then(|v| arena.int(v)).unwrap_or(default).max(1) as f32;
    let per_column = 226.0 / (64.0 * int("density", 20) * int("gridX", 1) * int("gridY", 1));
    layer.density = (per_column * (layer.spacing as f32).powi(2)).clamp(0.02, 1.0);
    Some(reg.add_object_layer(layer))
}

/// Where an object stands, in its box: the block WorldPainter puts on the
/// spot over the ground, from the object's offset (in Massif's order: x,
/// up, z) and, for an object of blocks by their place, where its own
/// coordinates start in the box (without an offset, it stands on its place
/// 0). `None` to keep the anchor Massif finds, or for a place outside the
/// box.
fn anchor(offset: Option<[i64; 3]>, shift: Option<[i64; 3]>, size: [u16; 3]) -> Option<[u16; 3]> {
    let offset = offset.or(shift.map(|_| [0; 3]))?;
    let shift = shift.unwrap_or([0; 3]);
    let anchor: [i64; 3] = std::array::from_fn(|k| -offset[k] - shift[k]);
    anchor.iter().zip(size).all(|(&a, s)| (0..s as i64).contains(&a)).then(|| anchor.map(|a| a as u16))
}

/// One of WorldPainter's objects as Massif's: its blocks in a box, `None`
/// for a kind not read here or one too large. With it, for an object of
/// blocks by their place, where its own coordinates (from which its offset
/// counts) start in the box, in Massif's order (x, up, z).
fn wp_object(arena: &Arena, object: Value) -> Option<(massif_core::objects::ObjectDef, Option<[i64; 3]>)> {
    use massif_core::objects::ObjectDef;
    let class = arena.class_name(object)?;
    let name = arena.field(object, "name").and_then(|n| arena.string(n)).unwrap_or("Object").to_owned();
    let int = |n: &str| arena.field(object, n).and_then(|v| arena.int(v)).filter(|&v| v > 0).map(|v| v as usize);
    let state = |material: Value| -> String {
        // A material of before 1.13 is known by its number.
        if arena.field(material, "identity").is_none_or(|i| i == Value::Null) {
            let n = |f: &str| arena.field(material, f).and_then(|v| arena.int(v)).unwrap_or(-1);
            let (id, data) = (n("blockType"), n("data"));
            if id >= 0 {
                return massif_objects::legacy::state(id as u16, data.max(0) as u8).unwrap_or_else(|| "minecraft:air".to_owned());
            }
        }
        crate::worldpainter::material_state(arena, material)
    };
    let fits = |w: usize, h: usize, l: usize| w * h * l <= massif_objects::MAX_VOLUME;
    match class.rsplit('.').next()? {
        // Sponge schematics: a palette and a block of it each, x first,
        // then WorldPainter's y (Massif's z), then up — as Massif lays
        // them.
        "Schem" => {
            let (w, h, l) = (int("width")?, int("height")?, int("length")?);
            let palette: Vec<String> = match arena.field(object, "palette").and_then(|p| arena.array(p))? {
                Array::Object(items) => items.iter().map(|&m| if m == Value::Null { "minecraft:air".to_owned() } else { state(m) }).collect(),
                _ => return None,
            };
            let blocks: Vec<u16> = match arena.field(object, "blocks").and_then(|b| arena.array(b))? {
                Array::Int(b) if b.len() == w * h * l => b.iter().map(|&k| if (k as usize) < palette.len() { k as u16 } else { 0 }).collect(),
                _ => return None,
            };
            fits(w, h, l).then(|| (ObjectDef::new(&name, [w as u16, h as u16, l as u16], palette, blocks), None))
        }
        // MCEdit schematics: numbered blocks, in the same order.
        "Schematic" => {
            let (w, h, l) = (int("width")?, int("height")?, int("length")?);
            let bytes = |n: &str| match arena.field(object, n).and_then(|b| arena.array(b)) {
                Some(Array::Byte(b)) => Some(b.as_slice()),
                _ => None,
            };
            let (ids, data, add) = (bytes("blocks")?, bytes("data")?, bytes("addBlocks"));
            if ids.len() != w * h * l || data.len() != ids.len() || !fits(w, h, l) {
                return None;
            }
            let mut palette = vec!["minecraft:air".to_owned()];
            let mut index: std::collections::HashMap<(u16, u8), u16> = std::collections::HashMap::new();
            let blocks = (0..ids.len())
                .map(|i| {
                    let high = add.and_then(|a| a.get(i >> 1)).map_or(0, |&a| if i & 1 == 0 { a & 15 } else { a >> 4 }) as u16;
                    let key = ((high << 8) | ids[i] as u16, data[i] & 15);
                    *index.entry(key).or_insert_with(|| {
                        palette.push(massif_objects::legacy::state(key.0, key.1).unwrap_or_else(|| "minecraft:air".to_owned()));
                        (palette.len() - 1) as u16
                    })
                })
                .collect();
            Some((ObjectDef::new(&name, [w as u16, h as u16, l as u16], palette, blocks), None))
        }
        // Structures and Bo2 objects: blocks by their place.
        "Structure" | "Bo2Object" => {
            let entries = arena.field(object, "blocks").and_then(|b| arena.map_entries(b))?;
            let mut placed = Vec::with_capacity(entries.len());
            for (point, value) in entries {
                let c = |n: &str| arena.field(point, n).and_then(|v| arena.int(v));
                let material = if class.ends_with("Bo2Object") { arena.field(value, "material")? } else { value };
                placed.push(([c("x")?, c("z")?, c("y")?], state(material)));
            }
            let lo: [i64; 3] = std::array::from_fn(|k| placed.iter().map(|p| p.0[k]).min().unwrap_or(0));
            let hi: [i64; 3] = std::array::from_fn(|k| placed.iter().map(|p| p.0[k]).max().unwrap_or(0));
            let size: [usize; 3] = std::array::from_fn(|k| (hi[k] - lo[k] + 1) as usize);
            if placed.is_empty() || !fits(size[0], size[1], size[2]) {
                return None;
            }
            let mut palette = vec!["minecraft:air".to_owned()];
            let mut blocks = vec![0u16; size[0] * size[1] * size[2]];
            for (p, s) in placed {
                let k = palette.iter().position(|q| *q == s).unwrap_or_else(|| {
                    palette.push(s);
                    palette.len() - 1
                });
                let [x, y, z] = [(p[0] - lo[0]) as usize, (p[1] - lo[1]) as usize, (p[2] - lo[2]) as usize];
                blocks[x + z * size[0] + y * size[0] * size[2]] = k as u16;
            }
            let def = ObjectDef::new(&name, size.map(|s| s as u16), palette, blocks);
            // A Bo2 object's own coordinates are its blocks' places moved
            // by its origin.
            let origin = arena.field(object, "origin").filter(|o| *o != Value::Null).map_or([0; 3], |o| {
                let c = |n: &str| arena.field(o, n).and_then(|v| arena.int(v)).unwrap_or(0);
                [c("x"), c("z"), c("y")]
            });
            Some((def, Some(std::array::from_fn(|k| lo[k] + origin[k]))))
        }
        _ => None,
    }
}

/// The frost cover of `reg`, added if it has none.
pub fn frost_cover(reg: &mut Registry) -> u16 {
    match reg.covers.iter().position(|c| c.frost && !c.removed) {
        Some(k) => k as u16 + 1,
        None => reg.add_cover(CoverDef::frost()),
    }
}

/// The strength of a layer in each column of a tile, from its data: a
/// nibble or a byte a column, or a `java.util.BitSet`.
pub fn strengths(arena: &Arena, value: Value) -> Option<Vec<u8>> {
    const AREA: usize = massif_core::TILE_AREA;
    if let Some(array) = arena.array(value) {
        return match array {
            Array::Byte(raw) if raw.len() == AREA / 2 => Some((0..AREA).map(|i| ((raw[i / 2] >> ((i & 1) * 4)) & 15) * 17).collect()),
            Array::Byte(raw) if raw.len() == AREA => Some(raw.clone()),
            _ => None,
        };
    }
    let words = match arena.field(value, "bits").and_then(|b| arena.array(b)) {
        Some(Array::Long(words)) => words,
        _ => return None,
    };
    Some((0..AREA).map(|i| if words.get(i >> 6).is_some_and(|w| (w >> (i & 63)) & 1 != 0) { 255 } else { 0 }).collect())
}

/// Whether a column painted at `strength` keeps the layer.
pub fn kept(strength: u8, curve: Curve, x: i32, z: i32, salt: u64) -> bool {
    let share = match curve {
        Curve::Linear => strength as u32,
        Curve::Square => (strength as u32 * strength as u32) / 255,
    };
    share >= 255 || (share > 0 && massif_core::compose::hash2(x, z, 0x7770_6c61_7965_7273 ^ salt) % 255 < share)
}

/// A plant layer as a ground cover: its plants with their occurrences.
fn plant_cover(arena: &Arena, key: Value, reg: &mut Registry) -> Option<u16> {
    let name = arena.field(key, "name").and_then(|v| arena.string(v)).unwrap_or("Plants").to_owned();
    let settings = match arena.field(key, "settings").and_then(|s| arena.array(s)) {
        Some(Array::Object(items)) => items.clone(),
        _ => return None,
    };
    let mut plants: Vec<(String, u16)> = Vec::new();
    let (mut wet, mut dry) = (0u32, 0u32);
    let mut height = [u8::MAX, 0u8];
    for (k, item) in settings.iter().enumerate() {
        let Some(occurrence) = arena.field(*item, "occurrence").and_then(|o| arena.int(o)) else { continue };
        let Some(&(state, kind)) = PLANTS.get(k) else { continue };
        if occurrence <= 0 || state.is_empty() {
            continue;
        }
        let weight = occurrence.min(u16::MAX as i64) as u16;
        match kind {
            Kind::Water => wet += weight as u32,
            _ => dry += weight as u32,
        }
        if kind == Kind::Tall {
            let int = |n: &str| arena.field(*item, n).and_then(|v| arena.int(v)).unwrap_or(1).clamp(1, 16) as u8;
            height = [height[0].min(int("growthFrom")), height[1].max(int("growthTo"))];
        } else {
            height = [1, height[1].max(1)];
        }
        plants.push((format!("minecraft:{state}"), weight));
    }
    if plants.is_empty() {
        return None;
    }
    // A cover grows in the air or in the water: the plants of the other
    // side are left out.
    let underwater = wet > dry;
    plants.retain(|(state, _)| PLANTS.iter().any(|(s, kind)| state.ends_with(s) && !s.is_empty() && (*kind == Kind::Water) == underwater));
    let color = arena.field(key, "colour").and_then(|c| arena.int(c)).map_or([96, 160, 72], |c| [(c >> 16) as u8, (c >> 8) as u8, c as u8]);
    let soil_only = arena.field(key, "onlyOnValidBlocks").and_then(|v| arena.bool(v)).unwrap_or(true) && !underwater;
    let mut def = CoverDef::new(&name, color, &[], 1.0);
    def.plants = plants;
    def.underwater = underwater;
    def.soil_only = soil_only;
    def.height = if height[0] == u8::MAX { [1, 1] } else { [height[0], height[1].max(height[0])] };
    // A cover of a new world with that name gives its place to the
    // layer's own plants: the map keeps the name it had.
    match reg.cover_by_name(&name) {
        Some(n) => {
            reg.covers[n as usize - 1] = def;
            Some(n)
        }
        None => Some(reg.add_cover(def)),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kind {
    /// One block or a two-block plant.
    Plain,
    /// Grows some blocks high (cactus, cane, bamboo).
    Tall,
    /// Grows under water.
    Water,
}

use Kind::{Plain, Tall, Water};

/// WorldPainter's plants (`Plants.ALL_PLANTS`, in its order, the index of a
/// plant layer's settings) as blocks; an empty name for a plant that hangs
/// or climbs, which a cover does not grow.
const PLANTS: &[(&str, Kind)] = &[
    ("short_grass", Plain),
    ("tall_grass", Plain),
    ("fern", Plain),
    ("large_fern", Plain),
    ("dead_bush", Plain),
    ("dandelion", Plain),
    ("poppy", Plain),
    ("blue_orchid", Plain),
    ("allium", Plain),
    ("azure_bluet", Plain),
    ("red_tulip", Plain),
    ("orange_tulip", Plain),
    ("white_tulip", Plain),
    ("pink_tulip", Plain),
    ("oxeye_daisy", Plain),
    ("sunflower", Plain),
    ("lilac", Plain),
    ("rose_bush", Plain),
    ("peony", Plain),
    ("oak_sapling", Plain),
    ("dark_oak_sapling", Plain),
    ("spruce_sapling", Plain),
    ("birch_sapling", Plain),
    ("jungle_sapling", Plain),
    ("acacia_sapling", Plain),
    ("red_mushroom", Plain),
    ("brown_mushroom", Plain),
    ("wheat[age=7]", Plain),
    ("carrots[age=7]", Plain),
    ("potatoes[age=7]", Plain),
    ("pumpkin_stem[age=7]", Plain),
    ("melon_stem[age=7]", Plain),
    ("cactus", Tall),
    ("sugar_cane", Tall),
    ("lily_pad", Plain),
    ("beetroots[age=3]", Plain),
    ("nether_wart[age=3]", Plain),
    ("", Plain), // chorus plant
    ("tube_coral", Water),
    ("brain_coral", Water),
    ("bubble_coral", Water),
    ("fire_coral", Water),
    ("horn_coral", Water),
    ("tube_coral_fan", Water),
    ("brain_coral_fan", Water),
    ("bubble_coral_fan", Water),
    ("fire_coral_fan", Water),
    ("horn_coral_fan", Water),
    ("kelp", Water),
    ("seagrass", Water),
    ("tall_seagrass", Water),
    ("sea_pickle", Water),
    ("cornflower", Plain),
    ("lily_of_the_valley", Plain),
    ("wither_rose", Plain),
    ("sweet_berry_bush[age=3]", Plain),
    ("bamboo", Tall),
    ("azalea", Plain),
    ("flowering_azalea", Plain),
    ("crimson_fungus", Plain),
    ("warped_fungus", Plain),
    ("crimson_roots", Plain),
    ("warped_roots", Plain),
    ("nether_sprouts", Plain),
    ("", Plain), // twisting vines
    ("", Plain), // glow lichen
    ("moss_carpet", Plain),
    ("", Plain), // big dripleaf
    ("pumpkin", Plain),
    ("melon", Plain),
    ("carved_pumpkin", Plain),
    ("jack_o_lantern", Plain),
    ("", Plain), // vine
    ("", Plain), // spore blossom
    ("", Plain), // weeping vines
    ("", Plain), // hanging roots
    ("", Plain), // glow berries
    ("", Plain), // small dripleaf
    ("", Plain), // mangrove propagule
    ("cherry_sapling", Plain),
    ("pink_petals", Plain),
    ("", Plain), // pitcher pod
    ("pitcher_plant", Plain),
    ("", Plain), // torchflower seed
    ("torchflower", Plain),
    ("closed_eyeblossom", Plain),
    ("", Plain), // pale hanging moss
    ("pale_moss_carpet", Plain),
    ("wildflowers", Plain),
    ("leaf_litter", Plain),
    ("bush", Plain),
    ("firefly_bush", Plain),
    ("cactus_flower", Plain),
    ("short_dry_grass", Plain),
    ("tall_dry_grass", Plain),
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plants_are_wordpainters_in_its_order() {
        // Plants.ALL_PLANTS has 95 plants.
        assert_eq!(PLANTS.len(), 95);
        assert_eq!(PLANTS[0].0, "short_grass");
        assert_eq!(PLANTS[33].0, "sugar_cane");
        assert_eq!(PLANTS[48].0, "kelp");
        assert_eq!(PLANTS[52].0, "cornflower");
        assert_eq!(PLANTS[80].0, "pink_petals");
        assert_eq!(PLANTS[94].0, "tall_dry_grass");
    }

    /// A Sponge schematic and an MCEdit schematic as WorldPainter keeps them
    /// in a world: their blocks in Massif's order (x, then WorldPainter's y
    /// that is Massif's z, then up), their states read.
    #[test]
    fn objects_are_read_from_the_world() {
        use crate::javaser::tests::StreamWriter;
        use crate::javaser::*;
        let null = |w: &mut StreamWriter| {
            w.u8(TC_NULL);
        };
        let mut w = StreamWriter::new();
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.bo2.Schem",
            SC_SERIALIZABLE,
            &[
                (b'I', "height", None),
                (b'I', "length", None),
                (b'I', "width", None),
                (b'[', "blocks", Some("[I")),
                (b'L', "name", Some("Ljava/lang/String;")),
                (b'[', "palette", Some("[Lorg/pepsoft/minecraft/Material;")),
            ],
            null,
        );
        w.handle();
        w.i32(4).i32(3).i32(3);
        // A trunk 3 high in the middle, a layer of leaves on it.
        w.u8(TC_ARRAY).class("[I", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(36);
        for i in 0..36 {
            let (x, y, z) = (i % 3, i / 3 % 3, i / 9);
            w.i32(if z == 3 { 2 } else if x == 1 && y == 1 { 1 } else { 0 });
        }
        w.string("Little tree");
        w.u8(TC_ARRAY).class("[Lorg.pepsoft.minecraft.Material;", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(3);
        let mut descs = None;
        for name in ["minecraft:air", "minecraft:oak_log", "minecraft:oak_leaves"] {
            match descs {
                None => {
                    let material = w.next;
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material",
                        SC_SERIALIZABLE,
                        &[(b'I', "blockType", None), (b'I', "data", None), (b'L', "identity", Some("Lorg/pepsoft/minecraft/Material$Identity;"))],
                        null,
                    );
                    w.handle();
                    w.i32(-1).i32(-1);
                    let identity = w.next;
                    w.u8(TC_OBJECT).class(
                        "org.pepsoft.minecraft.Material$Identity",
                        SC_SERIALIZABLE,
                        &[(b'L', "name", Some("Ljava/lang/String;")), (b'L', "properties", Some("Ljava/util/Map;"))],
                        null,
                    );
                    descs = Some((material, identity));
                }
                Some((material, identity)) => {
                    w.u8(TC_OBJECT).reference(material);
                    w.handle();
                    w.i32(-1).i32(-1);
                    w.u8(TC_OBJECT).reference(identity);
                }
            }
            w.handle();
            w.string(name);
            w.u8(TC_NULL);
        }
        // An MCEdit schematic: a log (17) under leaves (18), numbered.
        w.u8(TC_OBJECT).class(
            "org.pepsoft.worldpainter.layers.bo2.Schematic",
            SC_SERIALIZABLE,
            &[
                (b'S', "height", None),
                (b'S', "length", None),
                (b'S', "width", None),
                (b'[', "blocks", Some("[B")),
                (b'[', "data", Some("[B")),
                (b'L', "name", Some("Ljava/lang/String;")),
            ],
            null,
        );
        w.handle();
        w.u16(2).u16(1).u16(1);
        let bytes = w.next;
        w.u8(TC_ARRAY).class("[B", SC_SERIALIZABLE, &[], null);
        w.handle();
        w.i32(2).u8(17).u8(18);
        w.u8(TC_ARRAY).reference(bytes);
        w.handle();
        w.i32(2).u8(0).u8(0);
        w.string("Old bush");

        let mut parser = Parser::new(&w.out[..]).unwrap();
        let (schem, schematic) = (parser.next_object().unwrap().unwrap(), parser.next_object().unwrap().unwrap());
        let arena = &parser.arena;
        let (tree, shift) = wp_object(arena, schem).unwrap();
        assert_eq!(shift, None);
        assert_eq!((tree.name.as_str(), tree.size), ("Little tree", [3, 4, 3]));
        let at = |x: usize, y: usize, z: usize| tree.palette[tree.blocks[x + z * 3 + y * 9] as usize].as_str();
        assert_eq!(at(1, 0, 1), "minecraft:oak_log");
        assert_eq!(at(1, 2, 1), "minecraft:oak_log");
        assert_eq!(at(0, 0, 0), "minecraft:air");
        assert_eq!(at(2, 3, 0), "minecraft:oak_leaves");
        // Its offset, (-1, -1, 0) as WorldPainter writes it (z up): the
        // foot of its trunk on the spot.
        assert_eq!(anchor(Some([-1, 0, -1]), None, tree.size), Some([1, 0, 1]));
        assert_eq!(anchor(None, None, tree.size), None);
        assert_eq!(anchor(Some([-5, 0, 0]), None, tree.size), None, "outside its box");
        // Blocks by their place, from (-2, 0, -2): on their place 0.
        assert_eq!(anchor(None, Some([-2, 0, -2]), [5, 6, 5]), Some([2, 0, 2]));

        let (bush, _) = wp_object(arena, schematic).unwrap();
        assert_eq!(bush.size, [1, 2, 1]);
        assert!(bush.palette[bush.blocks[0] as usize].starts_with("minecraft:oak_log"), "{:?}", bush.palette);
        assert!(bush.palette[bush.blocks[1] as usize].starts_with("minecraft:oak_leaves"), "{:?}", bush.palette);
    }

    #[test]
    fn strength_keeps_its_share_of_columns() {
        let count = |s: u8, curve| (0..10_000).filter(|&i| kept(s, curve, i % 100, i / 100, 1)).count();
        assert_eq!(count(255, Curve::Linear), 10_000);
        assert_eq!(count(0, Curve::Square), 0);
        let half = count(128, Curve::Linear);
        assert!((4_700..5_300).contains(&half), "{half}");
        let quarter = count(128, Curve::Square);
        assert!((2_200..2_800).contains(&quarter), "{quarter}");
    }
}
