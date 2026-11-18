//! What the renderer draws a block state with: its *material*.
//!
//! Every block state the chunks hold (`minecraft:oak_log[axis=x]`) is
//! resolved once to a material: the layers of the textures of its top, its
//! sides and its bottom, the tints the game multiplies them by, and how light
//! goes through it (its [`Class`]). Materials are numbered from 1 in the
//! order they are met; 0 is air. The textures come from the game's jar, the
//! mods and the resource packs `massif_assets` read, as the 3D view's do.

use massif_assets::{Catalogue, Texture, Tint};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

/// Pixels on a side of a texture layer: the game's blocks are 16 × 16.
pub const SIDE: usize = 16;
/// Bytes of a layer, RGBA.
pub const LAYER_BYTES: usize = SIDE * SIDE * 4;
/// One layer of the texture array.
pub type Layer = [u8; LAYER_BYTES];

/// Air: nothing to draw.
pub const AIR: u16 = 0;

/// How light goes through a block: what a ray stops at, and which faces of
/// a block can be seen at all.
#[repr(u8)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Class {
    Air = 0,
    /// A whole block light does not go through.
    Opaque = 1,
    /// A whole block with holes in its texture: leaves, glass.
    Cutout = 2,
    Water = 3,
    Lava = 4,
}

impl Class {
    /// Whether a face of a block of this class is seen from a block of
    /// class `next` beside it: through air and holes, and between a fluid
    /// and anything but itself and opaque blocks.
    pub fn shows_to(self, next: Class) -> bool {
        match next {
            Class::Air | Class::Cutout => true,
            Class::Opaque => false,
            Class::Water | Class::Lava => next != self,
        }
    }
}

/// A material as the shader reads it: four words.
///
/// * 0: top layer | side layer << 16
/// * 1: bottom layer | flags << 16 (class: 3 bits, axis of a log: 2 bits,
///   emissive: 1 bit)
/// * 2, 3: the tints of the top and of the sides, sRGB, `r | g << 8 | b << 16`
pub type GpuMaterial = [u32; 4];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Material {
    top: u16,
    side: u16,
    bottom: u16,
    class: Class,
    /// Along which axis a log lies: 0 y, 1 x, 2 z.
    axis: u8,
    emissive: bool,
    top_tint: [u8; 3],
    side_tint: [u8; 3],
}

impl Material {
    fn gpu(&self) -> GpuMaterial {
        let flags = self.class as u32 | (self.axis as u32) << 3 | (self.emissive as u32) << 5;
        let rgb = |c: [u8; 3]| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16;
        [self.top as u32 | (self.side as u32) << 16, self.bottom as u32 | flags << 16, rgb(self.top_tint), rgb(self.side_tint)]
    }

    const AIR: Material = Material { top: 0, side: 0, bottom: 0, class: Class::Air, axis: 0, emissive: false, top_tint: [255; 3], side_tint: [255; 3] };
}

struct Inner {
    by_state: HashMap<String, u16>,
    list: Vec<Material>,
    layers: Vec<Layer>,
    layer_of: HashMap<String, u16>,
    /// Changes whenever a material or a layer is added.
    revision: u64,
}

/// The materials of every block state met so far.
pub struct Materials {
    cat: Option<Arc<Catalogue>>,
    inner: Mutex<Inner>,
    /// The class of every material, for the builders of the scene.
    classes: RwLock<Vec<Class>>,
    /// The most layers the texture array may have (the graphics card's
    /// limit); the textures past them keep their colour only.
    most_layers: usize,
}

impl Materials {
    /// Materials textured from `cat`, or in flat colours without it.
    pub fn new(cat: Option<Arc<Catalogue>>, most_layers: usize) -> Self {
        // Layer 0 is plain white: what has no texture keeps its tint.
        let inner = Inner { by_state: HashMap::new(), list: vec![Material::AIR], layers: vec![[255; LAYER_BYTES]], layer_of: HashMap::new(), revision: 1 };
        Self { cat, inner: Mutex::new(inner), classes: RwLock::new(vec![Class::Air]), most_layers: most_layers.clamp(1, u16::MAX as usize) }
    }

    /// The material of a block state: `name` with its properties.
    pub fn id(&self, name: &str, properties: &[(&str, &str)]) -> u16 {
        let mut key = String::new();
        state_key(&mut key, name, properties);
        self.id_of(&key, name, properties)
    }

    /// The material of the state written `key` (see [`state_key`]).
    pub fn id_of(&self, key: &str, name: &str, properties: &[(&str, &str)]) -> u16 {
        let mut inner = self.inner.lock().unwrap();
        if let Some(&id) = inner.by_state.get(key) {
            return id;
        }
        let material = self.resolve(&mut inner, name, properties);
        let id = if material.class == Class::Air {
            AIR
        } else if let Some(k) = inner.list.iter().position(|m| *m == material) {
            k as u16
        } else if inner.list.len() < u16::MAX as usize {
            inner.list.push(material);
            inner.revision += 1;
            self.classes.write().unwrap().push(material.class);
            (inner.list.len() - 1) as u16
        } else {
            AIR
        };
        inner.by_state.insert(key.to_owned(), id);
        id
    }

    /// The class of every material so far, by number.
    pub fn classes(&self) -> Vec<Class> {
        self.classes.read().unwrap().clone()
    }

    /// Changes whenever a material or a texture is added.
    pub fn revision(&self) -> u64 {
        self.inner.lock().unwrap().revision
    }

    /// The materials as the shader reads them, and the texture layers, with
    /// the revision they are of.
    pub fn snapshot(&self) -> (u64, Vec<GpuMaterial>, Vec<Layer>) {
        let inner = self.inner.lock().unwrap();
        (inner.revision, inner.list.iter().map(Material::gpu).collect(), inner.layers.clone())
    }

    /// How many materials there are, air included.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 1
    }

    fn resolve(&self, inner: &mut Inner, name: &str, properties: &[(&str, &str)]) -> Material {
        let short = name.rsplit(':').next().unwrap_or(name);
        let prop = |p: &str| properties.iter().find(|(k, _)| *k == p).map(|(_, v)| *v);
        const NOTHING: [&str; 6] = ["air", "cave_air", "void_air", "barrier", "light", "structure_void"];
        if NOTHING.contains(&short) {
            return Material::AIR;
        }
        const WATER: [&str; 6] = ["water", "bubble_column", "seagrass", "tall_seagrass", "kelp", "kelp_plant"];
        if WATER.contains(&short) {
            return self.fluid(inner, Class::Water, "block/water_still", Tint::Water.color());
        }
        if short == "lava" {
            return self.fluid(inner, Class::Lava, "block/lava_still", [255; 3]);
        }
        let axis = match prop("axis") {
            Some("x") => 1,
            Some("z") => 2,
            _ => 0,
        };
        let Some(block) = self.cat.as_ref().and_then(|c| c.block(name)) else {
            // Not in the game's blocks read (or none read): a cube of the
            // colour its name suggests.
            if !cube_without_catalogue(short) {
                return if prop("waterlogged") == Some("true") { self.fluid(inner, Class::Water, "block/water_still", Tint::Water.color()) } else { Material::AIR };
            }
            let color = guess_color(short);
            let class = if short.contains("leaves") || short.contains("glass") { Class::Cutout } else { Class::Opaque };
            return Material { top: 0, side: 0, bottom: 0, class, axis, emissive: emits(short, prop("lit")), top_tint: color, side_tint: color };
        };
        let cube = block.full_cube || nearly_whole(short);
        if !cube || thin(short) {
            // Plants and the blocks not drawn as cubes yet (their models
            // come with milestone 3): air, or the water they stand in.
            return if prop("waterlogged") == Some("true") { self.fluid(inner, Class::Water, "block/water_still", Tint::Water.color()) } else { Material::AIR };
        }
        let side_key = block.side.as_deref().or(block.top.as_deref());
        let top_key = block.top.as_deref().or(side_key);
        let bottom_key = block.bottom.as_deref().or(top_key);
        let mut layer = |key: Option<&str>| key.map_or(0, |k| self.layer(inner, k));
        let (top, side, bottom) = (layer(top_key), layer(side_key), layer(bottom_key));
        // A face without its texture keeps the block's colour.
        let tint = |l: u16, t: Tint, color: [u8; 3]| if l == 0 { color } else { t.color() };
        let class = if short.contains("leaves") || (short.contains("glass") && !short.contains("pane")) || short == "spawner" { Class::Cutout } else { Class::Opaque };
        Material {
            top,
            side,
            bottom,
            class,
            axis,
            emissive: emits(short, prop("lit")),
            top_tint: tint(top, block.top_tint, block.color),
            side_tint: tint(side, block.side_tint, block.side_color),
        }
    }

    fn fluid(&self, inner: &mut Inner, class: Class, key: &str, tint: [u8; 3]) -> Material {
        let layer = self.layer(inner, key);
        let color = if layer == 0 { if class == Class::Water { [0x3f, 0x76, 0xe4] } else { [0xcf, 0x5b, 0x14] } } else { tint };
        Material { top: layer, side: layer, bottom: layer, class, axis: 0, emissive: class == Class::Lava, top_tint: color, side_tint: color }
    }

    /// The layer of a texture, read once.
    fn layer(&self, inner: &mut Inner, key: &str) -> u16 {
        if let Some(&k) = inner.layer_of.get(key) {
            return k;
        }
        let k = match self.cat.as_ref().and_then(|c| c.texture(key)) {
            Some(tex) if inner.layers.len() < self.most_layers => {
                inner.layers.push(make_layer(tex));
                inner.revision += 1;
                (inner.layers.len() - 1) as u16
            }
            _ => 0,
        };
        inner.layer_of.insert(key.to_owned(), k);
        k
    }
}

/// A block state written out, `name[p=v,q=w]`, its properties sorted, into
/// `key`.
pub fn state_key(key: &mut String, name: &str, properties: &[(&str, &str)]) {
    key.clear();
    key.push_str(name);
    if !properties.is_empty() {
        let mut sorted: Vec<&(&str, &str)> = properties.iter().collect();
        sorted.sort_unstable();
        key.push('[');
        for (k, (p, v)) in sorted.into_iter().enumerate() {
            if k > 0 {
                key.push(',');
            }
            key.push_str(p);
            key.push('=');
            key.push_str(v);
        }
        key.push(']');
    }
}

/// The materials a thread has met, so that it asks [`Materials`] (and waits
/// for its lock) once a state only.
#[derive(Default)]
pub struct Lookup {
    key: String,
    known: std::collections::HashMap<String, u16>,
}

impl Lookup {
    pub fn id(&mut self, materials: &Materials, name: &str, properties: &[(&str, &str)]) -> u16 {
        state_key(&mut self.key, name, properties);
        if let Some(&id) = self.known.get(self.key.as_str()) {
            return id;
        }
        let id = materials.id_of(&self.key, name, properties);
        self.known.insert(self.key.clone(), id);
        id
    }
}

/// A texture as a 16 × 16 layer: the first square of an animated strip,
/// averaged down from a pack's larger textures (by coverage, so that holes
/// stay holes), or repeated up from a smaller one.
pub fn make_layer(tex: &Texture) -> Layer {
    let mut out = [0u8; LAYER_BYTES];
    let w = tex.width.max(1) as usize;
    let h = (tex.height as usize).min(w).max(1);
    if w == SIDE && h == SIDE {
        out.copy_from_slice(&tex.rgba[..LAYER_BYTES]);
        return out;
    }
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (x0, x1) = (x * w / SIDE, ((x + 1) * w / SIDE).max(x * w / SIDE + 1));
            let (y0, y1) = (y * h / SIDE, ((y + 1) * h / SIDE).max(y * h / SIDE + 1));
            let (mut sum, mut alpha, mut n) = ([0u32; 3], 0u32, 0u32);
            for sy in y0..y1 {
                for sx in x0..x1 {
                    let p = &tex.rgba[(sy * w + sx) * 4..][..4];
                    for k in 0..3 {
                        sum[k] += p[k] as u32 * p[3] as u32;
                    }
                    alpha += p[3] as u32;
                    n += 1;
                }
            }
            let o = &mut out[(y * SIDE + x) * 4..][..4];
            for k in 0..3 {
                o[k] = sum[k].checked_div(alpha).unwrap_or(0) as u8;
            }
            o[3] = (alpha / n.max(1)) as u8;
        }
    }
    out
}

/// Blocks a little less than whole that look like cubes from afar: drawn
/// as cubes until their models come.
fn nearly_whole(short: &str) -> bool {
    ["dirt_path", "farmland", "soul_sand", "mud", "honey_block", "slab", "stairs", "cactus", "chest", "barrel", "composter", "cauldron", "bed", "snow_block", "powder_snow", "suspicious", "anvil", "beacon", "conduit", "lectern", "enchanting_table", "end_portal_frame", "daylight_detector", "stonecutter", "grindstone", "bell"]
        .iter()
        .any(|w| short.contains(w))
        && !thin(short)
}

/// Blocks too thin or too small to draw as cubes: left out until their
/// models come.
fn thin(short: &str) -> bool {
    [
        "carpet", "rail", "pressure_plate", "button", "torch", "sign", "banner", "pane", "bars", "fence", "wall", "door", "ladder", "vine", "lantern", "chain", "rod", "head", "skull", "candle", "flower_pot", "potted", "pickle", "egg", "lever", "tripwire", "redstone_wire", "repeater", "comparator", "snow", "moss_carpet", "lily_pad", "scaffolding", "cobweb", "frogspawn", "petals", "leaf_litter", "dripstone", "amethyst_bud", "amethyst_cluster", "sculk_vein", "glow_lichen", "hanging_roots", "spore", "coral", "sapling", "bush", "grass", "fern", "flower", "mushroom", "roots", "sprouts", "fungus", "azalea", "dripleaf", "bamboo", "sugar_cane", "kelp", "seagrass", "tulip", "orchid", "allium", "bluet", "daisy", "poppy", "dandelion", "cornflower", "lily", "rose", "peony", "lilac", "sunflower", "wheat", "carrots", "potatoes", "beetroots", "stem", "berry", "cocoa", "nether_wart", "torchflower", "pitcher", "eyeblossom", "cactus_flower", "firefly", "shelf",
    ]
    .iter()
    .any(|w| short.contains(w))
        && !short.ends_with("_block")
        && short != "grass_block"
        && !short.ends_with("_slab")
        && !short.ends_with("_stairs")
        && short != "snow_block"
}

/// Whether a block gives light.
fn emits(short: &str, lit: Option<&str>) -> bool {
    matches!(short, "glowstone" | "sea_lantern" | "shroomlight" | "magma_block" | "jack_o_lantern" | "ochre_froglight" | "verdant_froglight" | "pearlescent_froglight" | "beacon" | "crying_obsidian" | "end_gateway" | "respawn_anchor")
        || (lit == Some("true") && matches!(short, "redstone_lamp" | "furnace" | "smoker" | "blast_furnace" | "redstone_ore" | "deepslate_redstone_ore" | "copper_bulb"))
}

/// Without the game's blocks: whether a block is a cube, by its name.
fn cube_without_catalogue(short: &str) -> bool {
    !thin(short) || short == "grass_block"
}

/// Without the game's textures: a colour for a block, by its name.
fn guess_color(short: &str) -> [u8; 3] {
    const COLORS: [(&str, [u8; 3]); 24] = [
        ("grass_block", [0x5d, 0x9b, 0x3a]),
        ("leaves", [0x3b, 0x7a, 0x22]),
        ("log", [0x6b, 0x51, 0x33]),
        ("wood", [0x6b, 0x51, 0x33]),
        ("planks", [0xa0, 0x82, 0x4e]),
        ("podzol", [0x5b, 0x40, 0x1d]),
        ("dirt", [0x86, 0x60, 0x43]),
        ("mud", [0x3c, 0x39, 0x3d]),
        ("red_sand", [0xbe, 0x66, 0x21]),
        ("sandstone", [0xd8, 0xcb, 0x9b]),
        ("sand", [0xdb, 0xcf, 0xa3]),
        ("gravel", [0x83, 0x7f, 0x7e]),
        ("snow", [0xf3, 0xf8, 0xf8]),
        ("ice", [0x91, 0xb6, 0xf3]),
        ("clay", [0xa0, 0xa6, 0xb3]),
        ("deepslate", [0x50, 0x50, 0x52]),
        ("granite", [0x95, 0x67, 0x55]),
        ("diorite", [0xbc, 0xbc, 0xbc]),
        ("andesite", [0x88, 0x88, 0x89]),
        ("terracotta", [0x98, 0x5e, 0x43]),
        ("bedrock", [0x55, 0x55, 0x55]),
        ("obsidian", [0x14, 0x12, 0x1d]),
        ("netherrack", [0x61, 0x26, 0x26]),
        ("stone", [0x7e, 0x7e, 0x7e]),
    ];
    COLORS.iter().find(|(w, _)| short.contains(w)).map_or([0x90, 0x90, 0x90], |(_, c)| *c)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_resolve_once_and_air_is_nothing() {
        let m = Materials::new(None, 64);
        assert_eq!(m.id("minecraft:air", &[]), AIR);
        assert_eq!(m.id("minecraft:cave_air", &[]), AIR);
        let stone = m.id("minecraft:stone", &[]);
        assert_ne!(stone, AIR);
        assert_eq!(m.id("minecraft:stone", &[]), stone, "the same state, the same material");
        let water = m.id("minecraft:water", &[("level", "0")]);
        assert_eq!(m.classes()[water as usize], Class::Water);
        // A log across x is its own material; along y, the same as no axis.
        let y = m.id("minecraft:oak_log", &[("axis", "y")]);
        let x = m.id("minecraft:oak_log", &[("axis", "x")]);
        assert_ne!(x, y);
        assert_eq!(m.id("minecraft:oak_log", &[]), y);
        // Properties in any order are the same state.
        assert_eq!(m.id("minecraft:oak_stairs", &[("facing", "east"), ("half", "top")]), m.id("minecraft:oak_stairs", &[("half", "top"), ("facing", "east")]));
        assert_eq!(m.id("minecraft:short_grass", &[]), AIR, "plants wait for their models");
        assert_eq!(m.classes()[m.id("minecraft:seagrass", &[]) as usize], Class::Water, "a plant under water shows the water");
    }

    #[test]
    fn faces_show_through_air_holes_and_other_fluids() {
        use Class::*;
        assert!(Opaque.shows_to(Air) && Opaque.shows_to(Water) && Opaque.shows_to(Cutout));
        assert!(!Opaque.shows_to(Opaque));
        assert!(!Water.shows_to(Water) && Water.shows_to(Air) && !Water.shows_to(Opaque));
        assert!(Cutout.shows_to(Cutout));
    }

    #[test]
    fn a_larger_texture_is_averaged_down_keeping_holes() {
        let mut rgba = vec![0u8; 32 * 32 * 4];
        for (k, p) in rgba.chunks_mut(4).enumerate() {
            let (x, y) = (k % 32, k / 32);
            // The left half opaque red, the right half a hole.
            if x < 16 {
                p.copy_from_slice(&[200, 10, 10, 255]);
            }
            let _ = y;
        }
        let layer = make_layer(&Texture { width: 32, height: 32, rgba });
        assert_eq!(&layer[..4], &[200, 10, 10, 255]);
        assert_eq!(layer[(SIDE - 1) * 4 + 3], 0);
    }
}
