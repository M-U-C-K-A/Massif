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
    /// A block drawn by its model's faces (a slab, a fence, a torch, a
    /// plant): light goes around them.
    Shaped = 5,
}

impl Class {
    /// Whether a face of a block of this class is seen from a block of
    /// class `next` beside it: through air and holes, and between a fluid
    /// and anything but itself and opaque blocks.
    pub fn shows_to(self, next: Class) -> bool {
        match next {
            Class::Air | Class::Cutout | Class::Shaped => true,
            Class::Opaque => false,
            Class::Water | Class::Lava => next != self,
        }
    }
}

/// A material as the shader reads it: twelve words.
///
/// * 0: top layer | side layer << 16
/// * 1: bottom layer | flags << 16 (class: 3 bits, axis of a log: 2 bits,
///   glows: 1 bit)
/// * 2, 3: the tints of the top and of the sides, sRGB, `r | g << 8 | b << 16`
/// * 4: the colour of its light, sRGB, | its light level (the game's, 0 to
///   15) << 24
/// * 5, 6: a shaped block's first face and how many (see [`GpuQuad`])
/// * 7: the box around its faces, in pixels: x0, y0, z0, x1, y1, z1, 5
///   bits each from the lowest
/// * 8, 9: its roughness and how metallic it is, 0 to 1, as float bits
/// * 10, 11: unused
pub type GpuMaterial = [u32; 12];

/// A face of a shaped block as the shader reads it: sixteen words, the
/// floats as their bits.
///
/// * 0..3: its corner (the block's space, 0 to 1), its texture layer |
///   the texture's turn << 16 | tinted << 18 | its texture has holes << 19
/// * 4..6: the edge toward u1; 8..10: the edge toward v1
/// * 12..15: the part of the texture it shows (u0, v0, u1, v1), 0 to 1
pub type GpuQuad = [u32; 16];

#[derive(Debug, Clone, Copy, PartialEq)]
struct Material {
    top: u16,
    side: u16,
    bottom: u16,
    class: Class,
    /// Along which axis a log lies: 0 y, 1 x, 2 z.
    axis: u8,
    /// The light it gives, as the game's levels (0: none), and its colour.
    light: u8,
    glow: [u8; 3],
    top_tint: [u8; 3],
    side_tint: [u8; 3],
    /// A shaped block's faces: the first, and how many, and the box
    /// around them (see [`GpuMaterial`]).
    first_quad: u32,
    quads: u16,
    bounds: u32,
    /// How rough its surface is (1: no shine at all) and how metallic.
    roughness: f32,
    metal: f32,
}

impl Material {
    fn gpu(&self) -> GpuMaterial {
        let flags = self.class as u32 | (self.axis as u32) << 3 | ((self.light > 0) as u32) << 5;
        let rgb = |c: [u8; 3]| c[0] as u32 | (c[1] as u32) << 8 | (c[2] as u32) << 16;
        [self.top as u32 | (self.side as u32) << 16, self.bottom as u32 | flags << 16, rgb(self.top_tint), rgb(self.side_tint), rgb(self.glow) | (self.light as u32) << 24, self.first_quad, self.quads as u32, self.bounds, self.roughness.to_bits(), self.metal.to_bits(), 0, 0]
    }

    const AIR: Material = Material { top: 0, side: 0, bottom: 0, class: Class::Air, axis: 0, light: 0, glow: [0; 3], top_tint: [255; 3], side_tint: [255; 3], first_quad: 0, quads: 0, bounds: 0, roughness: 0.9, metal: 0.0 };

    /// A light without a block to draw yet (a torch, a lantern: their
    /// models come with milestone 3).
    fn glow_only(light: u8, glow: [u8; 3]) -> Material {
        Material { light, glow, ..Material::AIR }
    }
}

struct Inner {
    by_state: HashMap<String, u16>,
    list: Vec<Material>,
    quads: Vec<GpuQuad>,
    layers: Vec<Layer>,
    /// A resource pack's LabPBR maps of each layer (`_n`: normal, ambient
    /// occlusion, height; `_s`: smoothness, reflectance or metal, porosity,
    /// emission), flat and empty where it has none; whether any has.
    normals: Vec<Layer>,
    speculars: Vec<Layer>,
    pbr: bool,
    layer_of: HashMap<String, u16>,
    /// Changes whenever a material or a layer is added.
    revision: u64,
}

/// The materials of every block state met so far.
pub struct Materials {
    cat: Option<Arc<Catalogue>>,
    /// The game's models and the textures the catalogue does not keep.
    library: Option<crate::models::Library>,
    inner: Mutex<Inner>,
    /// The class of every material, and the light it gives, for the
    /// builders of the scene.
    classes: RwLock<Vec<Class>>,
    lights: RwLock<Vec<u8>>,
    /// The most layers the texture array may have (the graphics card's
    /// limit); the textures past them keep their colour only.
    most_layers: usize,
}

impl Materials {
    /// Materials textured from `cat`, or in flat colours without it.
    pub fn new(cat: Option<Arc<Catalogue>>, most_layers: usize) -> Self {
        // Layer 0 is plain white: what has no texture keeps its tint.
        let library = cat.as_deref().map(crate::models::Library::of);
        Self::with_library(cat, library, most_layers)
    }

    /// Materials whose models and packs come from `library`.
    pub fn with_library(cat: Option<Arc<Catalogue>>, library: Option<crate::models::Library>, most_layers: usize) -> Self {
        let inner = Inner { by_state: HashMap::new(), list: vec![Material::AIR], quads: Vec::new(), layers: vec![[255; LAYER_BYTES]], normals: vec![FLAT], speculars: vec![[0; LAYER_BYTES]], pbr: false, layer_of: HashMap::new(), revision: 1 };
        Self { cat, library, inner: Mutex::new(inner), classes: RwLock::new(vec![Class::Air]), lights: RwLock::new(vec![0]), most_layers: most_layers.clamp(1, u16::MAX as usize) }
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
        let id = if material.class == Class::Air && material.light == 0 {
            AIR
        } else if let Some(k) = inner.list.iter().position(|m| *m == material) {
            k as u16
        } else if inner.list.len() < u16::MAX as usize {
            inner.list.push(material);
            inner.revision += 1;
            self.classes.write().unwrap().push(material.class);
            self.lights.write().unwrap().push(material.light);
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

    /// The light every material gives so far (the game's levels), by number.
    pub fn lights(&self) -> Vec<u8> {
        self.lights.read().unwrap().clone()
    }

    /// Changes whenever a material or a texture is added.
    pub fn revision(&self) -> u64 {
        self.inner.lock().unwrap().revision
    }

    /// The materials as the shader reads them, the faces of the shaped
    /// ones, and the texture layers, with the revision they are of.
    pub fn snapshot(&self) -> (u64, Vec<GpuMaterial>, Vec<GpuQuad>, Vec<Layer>) {
        let inner = self.inner.lock().unwrap();
        (inner.revision, inner.list.iter().map(Material::gpu).collect(), inner.quads.clone(), inner.layers.clone())
    }

    /// The LabPBR maps of every layer (normals, speculars), if a pack has
    /// any.
    pub fn pbr(&self) -> Option<(Vec<Layer>, Vec<Layer>)> {
        let inner = self.inner.lock().unwrap();
        inner.pbr.then(|| (inner.normals.clone(), inner.speculars.clone()))
    }

    /// How many materials there are, air included.
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().list.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() <= 1
    }

    /// How material `id` looks from straight above (see [`TopView`]).
    pub fn top_view(&self, id: u16) -> TopView {
        let inner = self.inner.lock().unwrap();
        let Some(m) = inner.list.get(id as usize).copied() else { return TopView::AIR };
        top_view(&m, &inner.quads, &inner.layers)
    }

    /// How every material so far looks from above, by number.
    pub fn top_views(&self) -> Vec<TopView> {
        let inner = self.inner.lock().unwrap();
        inner.list.iter().map(|m| top_view(m, &inner.quads, &inner.layers)).collect()
    }

    fn resolve(&self, inner: &mut Inner, name: &str, properties: &[(&str, &str)]) -> Material {
        let mut m = self.resolve_shape(inner, name, properties);
        if !matches!(m.class, Class::Water | Class::Lava) {
            let short = name.rsplit(':').next().unwrap_or(name);
            (m.roughness, m.metal) = surface(short);
        }
        m
    }

    fn resolve_shape(&self, inner: &mut Inner, name: &str, properties: &[(&str, &str)]) -> Material {
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
        let light = light_level(short, &prop);
        let Some(block) = self.cat.as_ref().and_then(|c| c.block(name)) else {
            // Not in the game's blocks read (or none read): a cube of the
            // colour its name suggests.
            if !cube_without_catalogue(short) {
                return if light > 0 {
                    Material::glow_only(light, glow_color(short))
                } else if prop("waterlogged") == Some("true") {
                    self.fluid(inner, Class::Water, "block/water_still", Tint::Water.color())
                } else {
                    Material::AIR
                };
            }
            let color = guess_color(short);
            let class = if short.contains("leaves") || short.contains("glass") { Class::Cutout } else { Class::Opaque };
            return Material { class, axis, light, glow: color, top_tint: color, side_tint: color, ..Material::AIR };
        };
        // A block that is not a whole cube: its model's faces.
        if !block.full_cube
            && let Some(shaped) = self.shaped(inner, name, properties, block, light)
        {
            return shaped;
        }
        let cube = block.full_cube || nearly_whole(short);
        if !cube || thin(short) {
            // Plants and the blocks not drawn as cubes yet (their models
            // come with milestone 3): air, the water they stand in, or the
            // light they give (a torch, a lantern).
            return if light > 0 {
                Material::glow_only(light, glow_color(short))
            } else if prop("waterlogged") == Some("true") {
                self.fluid(inner, Class::Water, "block/water_still", Tint::Water.color())
            } else {
                Material::AIR
            };
        }
        // The catalogue's textures, or where it has none (a model of 26.1
        // and later naming its texture as `{"sprite": …}`: glass), those of
        // the model's faces.
        let from_model = if block.top.is_none() && block.side.is_none() { self.library.as_ref().map(|l| cube_textures(&l.quads(name, properties))) } else { None };
        let (model_top, model_side, model_bottom) = from_model.unwrap_or_default();
        let side_key = block.side.as_deref().or(model_side.as_deref()).or(block.top.as_deref());
        let top_key = block.top.as_deref().or(model_top.as_deref()).or(side_key);
        let bottom_key = block.bottom.as_deref().or(model_bottom.as_deref()).or(top_key);
        let mut layer = |key: Option<&str>| key.map_or(0, |k| self.layer(inner, k));
        let (top, side, bottom) = (layer(top_key), layer(side_key), layer(bottom_key));
        // A face without its texture keeps the block's colour.
        let tint = |l: u16, t: Tint, color: [u8; 3]| if l == 0 { color } else { t.color() };
        let class = if short.contains("leaves") || (short.contains("glass") && !short.contains("pane")) || short == "spawner" { Class::Cutout } else { Class::Opaque };
        // A glowing block's light has the colour of its texture.
        let glow = if light == 0 {
            [0; 3]
        } else {
            top_key.and_then(|k| self.cat.as_ref().and_then(|c| c.texture(k))).map_or(block.color, |t| t.mean(Tint::None))
        };
        Material {
            top,
            side,
            bottom,
            class,
            axis,
            light,
            glow,
            top_tint: tint(top, block.top_tint, block.color),
            side_tint: tint(side, block.side_tint, block.side_color),
            ..Material::AIR
        }
    }

    /// A block drawn by its model, if its model has faces (an entity's
    /// block, a chest or a sign, has none).
    fn shaped(&self, inner: &mut Inner, name: &str, properties: &[(&str, &str)], block: &massif_assets::Block, light: u8) -> Option<Material> {
        let quads = self.library.as_ref()?.quads(name, properties);
        if quads.is_empty() || inner.quads.len() + quads.len() > u32::MAX as usize {
            return None;
        }
        let first = inner.quads.len() as u32;
        let count = quads.len().min(u16::MAX as usize);
        for q in quads.iter().take(count) {
            let layer = self.layer(inner, &q.texture) as u32;
            // Only a texture with holes needs looking at where a ray meets it.
            let holes = inner.layers.get(layer as usize).is_some_and(|l| l.chunks(4).any(|p| p[3] < 128));
            let f = |x: f32| x.to_bits();
            inner.quads.push([
                f(q.origin[0]),
                f(q.origin[1]),
                f(q.origin[2]),
                layer | (q.rotation as u32 & 3) << 16 | (q.tinted as u32) << 18 | (holes as u32) << 19,
                f(q.u[0]),
                f(q.u[1]),
                f(q.u[2]),
                0,
                f(q.v[0]),
                f(q.v[1]),
                f(q.v[2]),
                0,
                f(q.uv[0] / 16.0),
                f(q.uv[1] / 16.0),
                f(q.uv[2] / 16.0),
                f(q.uv[3] / 16.0),
            ]);
        }
        inner.revision += 1;
        // The box around the faces, in whole pixels, within the block.
        let (mut lo, mut hi) = ([16.0f32; 3], [0.0f32; 3]);
        for q in quads.iter().take(count) {
            for (a, b) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
                for k in 0..3 {
                    let c = (q.origin[k] + q.u[k] * a + q.v[k] * b) * 16.0;
                    lo[k] = lo[k].min(c);
                    hi[k] = hi[k].max(c);
                }
            }
        }
        let px = |c: f32, up: bool| (if up { c.ceil() } else { c.floor() }).clamp(0.0, 16.0) as u32;
        let bounds = px(lo[0], false) | px(lo[1], false) << 5 | px(lo[2], false) << 10 | px(hi[0], true) << 15 | px(hi[1], true) << 20 | px(hi[2], true) << 25;
        // The biome's tint of its tinted faces: the block's own.
        let tint = if block.top_tint != Tint::None { block.top_tint } else if block.side_tint != Tint::None { block.side_tint } else { Tint::Grass };
        let short = name.rsplit(':').next().unwrap_or(name);
        let glow = if light == 0 { [0; 3] } else { glow_color(short) };
        Some(Material { class: Class::Shaped, light, glow, top_tint: tint.color(), side_tint: tint.color(), first_quad: first, quads: count as u16, bounds, ..Material::AIR })
    }

    fn fluid(&self, inner: &mut Inner, class: Class, key: &str, tint: [u8; 3]) -> Material {
        let layer = self.layer(inner, key);
        let color = if layer == 0 { if class == Class::Water { [0x3f, 0x76, 0xe4] } else { [0xcf, 0x5b, 0x14] } } else { tint };
        let (light, glow) = if class == Class::Lava { (15, [255, 130, 40]) } else { (0, [0; 3]) };
        Material { top: layer, side: layer, bottom: layer, class, axis: 0, light, glow, top_tint: color, side_tint: color, ..Material::AIR }
    }

    /// The layer of a texture, read once.
    fn layer(&self, inner: &mut Inner, key: &str) -> u16 {
        if let Some(&k) = inner.layer_of.get(key) {
            return k;
        }
        let owned;
        let tex = match self.cat.as_ref().and_then(|c| c.texture(key)) {
            Some(t) => Some(&**t),
            None => {
                owned = self.library.as_ref().and_then(|l| l.texture(key));
                owned.as_ref()
            }
        };
        let k = match tex {
            Some(tex) if inner.layers.len() < self.most_layers => {
                inner.layers.push(make_layer(tex));
                // Its LabPBR maps, from a pack.
                let map = |suffix: &str| self.library.as_ref().and_then(|l| l.texture(&format!("{key}{suffix}"))).map(|t| make_layer(&t));
                let (n, sp) = (map("_n"), map("_s"));
                inner.pbr |= n.is_some() || sp.is_some();
                inner.normals.push(n.unwrap_or(FLAT));
                inner.speculars.push(sp.unwrap_or([0; LAYER_BYTES]));
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

/// How rough a block's surface is and how metallic, by its name: ice and
/// glass smooth, polished stone and glazed terracotta with some shine, the
/// blocks of metal metallic; everything else matt.
fn surface(short: &str) -> (f32, f32) {
    let has = |w: &str| short.contains(w);
    if has("ice") || has("glass") {
        (0.06, 0.0)
    } else if ["iron_block", "gold_block", "netherite_block", "raw_iron_block", "raw_gold_block", "raw_copper_block", "iron_bars", "iron_door", "iron_trapdoor", "chain", "lantern", "anvil", "cauldron", "bell", "heavy_core"].iter().any(|n| short.ends_with(n) && !short.starts_with("waxed_") || short == *n) || (has("copper") && !has("ore")) {
        (if has("raw_") { 0.6 } else { 0.32 }, 1.0)
    } else if has("polished") || has("smooth") || has("glazed_terracotta") || has("quartz") || has("prismarine") || has("purpur") || has("_concrete") && !has("powder") || has("amethyst") || has("obsidian") || matches!(short, "diamond_block" | "emerald_block" | "lapis_block") {
        (0.38, 0.0)
    } else if has("leaves") || has("grass") || has("fern") || has("flower") {
        (0.75, 0.0)
    } else {
        (0.88, 0.0)
    }
}

/// The textures of a cube's top, sides and bottom, from its model's faces.
fn cube_textures(quads: &[crate::models::Quad]) -> (Option<String>, Option<String>, Option<String>) {
    let (mut top, mut side, mut bottom) = (None, None, None);
    for q in quads {
        let n = [q.u[1] * q.v[2] - q.u[2] * q.v[1], q.u[2] * q.v[0] - q.u[0] * q.v[2], q.u[0] * q.v[1] - q.u[1] * q.v[0]];
        let slot = if n[1].abs() > n[0].abs().max(n[2].abs()) { if q.origin[1] > 0.5 { &mut top } else { &mut bottom } } else { &mut side };
        slot.get_or_insert_with(|| q.texture.to_string());
    }
    (top, side, bottom)
}

/// A flat normal map: straight out, nothing occluded, at the top.
const FLAT: Layer = {
    let mut l = [0u8; LAYER_BYTES];
    let mut k = 0;
    while k < LAYER_BYTES {
        l[k] = 128;
        l[k + 1] = 128;
        l[k + 2] = 255;
        l[k + 3] = 255;
        k += 4;
    }
    l
};

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
        && !matches!(short, "sea_lantern" | "jack_o_lantern")
}

/// The light a block gives, as the game's levels (0 to 15).
fn light_level<'a>(short: &str, prop: &impl Fn(&str) -> Option<&'a str>) -> u8 {
    let lit = prop("lit") != Some("false");
    let count = |p: &str| prop(p).and_then(|v| v.parse::<u8>().ok()).unwrap_or(1);
    match short {
        "glowstone" | "sea_lantern" | "shroomlight" | "ochre_froglight" | "verdant_froglight" | "pearlescent_froglight" | "jack_o_lantern" | "lava" | "beacon" | "conduit" | "end_portal" | "end_gateway" | "fire" | "lantern" => 15,
        "end_rod" | "torch" | "wall_torch" => 14,
        "soul_torch" | "soul_wall_torch" | "soul_lantern" | "soul_fire" | "crying_obsidian" => 10,
        "campfire" => if lit { 15 } else { 0 },
        "soul_campfire" => if lit { 10 } else { 0 },
        "redstone_lamp" | "copper_bulb" | "waxed_copper_bulb" => if prop("lit") == Some("true") { 15 } else { 0 },
        "exposed_copper_bulb" | "waxed_exposed_copper_bulb" => if prop("lit") == Some("true") { 12 } else { 0 },
        "weathered_copper_bulb" | "waxed_weathered_copper_bulb" => if prop("lit") == Some("true") { 8 } else { 0 },
        "oxidized_copper_bulb" | "waxed_oxidized_copper_bulb" => if prop("lit") == Some("true") { 4 } else { 0 },
        "furnace" | "smoker" | "blast_furnace" => if prop("lit") == Some("true") { 13 } else { 0 },
        "redstone_torch" | "redstone_wall_torch" => if lit { 7 } else { 0 },
        "redstone_ore" | "deepslate_redstone_ore" => if prop("lit") == Some("true") { 9 } else { 0 },
        "magma_block" => 3,
        "glow_lichen" | "enchanting_table" | "ender_chest" => 7,
        "nether_portal" => 11,
        "amethyst_cluster" => 5,
        "large_amethyst_bud" => 4,
        "medium_amethyst_bud" => 2,
        "small_amethyst_bud" => 1,
        "sea_pickle" => if prop("waterlogged") == Some("true") { 3 + 3 * count("pickles").min(4) } else { 0 },
        "cave_vines" | "cave_vines_plant" => if prop("berries") == Some("true") { 14 } else { 0 },
        "respawn_anchor" => [0, 3, 7, 11, 15][prop("charges").and_then(|v| v.parse::<usize>().ok()).unwrap_or(0).min(4)],
        _ if short.ends_with("candle") && prop("lit") == Some("true") => 3 * count("candles").min(4),
        _ => 0,
    }
}

/// The colour of the light of a block not drawn yet (as its flame).
fn glow_color(short: &str) -> [u8; 3] {
    if short.starts_with("soul_") {
        [110, 220, 255]
    } else if short.starts_with("redstone") {
        [255, 70, 45]
    } else if short == "end_rod" {
        [255, 240, 235]
    } else if short == "sea_pickle" {
        [190, 255, 160]
    } else if short == "glow_lichen" {
        [200, 245, 215]
    } else if short.contains("amethyst") {
        [205, 150, 255]
    } else {
        [255, 185, 105]
    }
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
        // A torch is not drawn yet, but gives its light.
        let torch = m.id("minecraft:wall_torch", &[("facing", "north")]);
        assert_ne!(torch, AIR);
        assert_eq!((m.classes()[torch as usize], m.lights()[torch as usize]), (Class::Air, 14));
        let candles = m.id("minecraft:candle", &[("candles", "3"), ("lit", "true")]);
        assert_eq!(m.lights()[candles as usize], 9);
        assert_eq!(m.id("minecraft:candle", &[("candles", "3"), ("lit", "false")]), AIR);
        let sea_lantern = m.id("minecraft:sea_lantern", &[]);
        assert_eq!(m.classes()[sea_lantern as usize], Class::Opaque, "a sea lantern is a whole block");
        let glowstone = m.id("minecraft:glowstone", &[]);
        assert_eq!(m.lights()[glowstone as usize], 15);
        let seagrass = m.id("minecraft:seagrass", &[]);
        assert_eq!(m.classes()[seagrass as usize], Class::Water, "a plant under water shows the water");
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

/// What a block is to a map seen from above.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TopKind {
    /// Nothing to see: what is under it shows.
    Air,
    /// A ground the eye stops at: a whole block, or a shaped one covering
    /// its square (a slab, stairs, a carpet).
    Block,
    /// Something standing on the ground that leaves it showing around:
    /// a plant, a torch, a fence.
    Overlay,
    Water,
    Lava,
}

/// How a block looks from straight above: what the Diorama draws of it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TopView {
    pub kind: TopKind,
    /// The texture layer of its top.
    pub layer: u16,
    /// What that texture is multiplied by, sRGB.
    pub tint: [u8; 3],
    /// The part of the texture shown, 0 to 1: u0, v0, u1, v1.
    pub uv: [f32; 4],
    /// The part of the block's square it covers, 0 to 1: x0, z0, x1, z1.
    pub area: [f32; 4],
    /// The mean colour of its top, sRGB, and how much of the square it
    /// covers (alpha).
    pub mean: [u8; 4],
    /// The light it gives, the game's levels.
    pub light: u8,
}

impl TopView {
    pub const AIR: TopView = TopView { kind: TopKind::Air, layer: 0, tint: [255; 3], uv: [0.0, 0.0, 1.0, 1.0], area: [0.0, 0.0, 1.0, 1.0], mean: [0; 4], light: 0 };
}

fn top_view(m: &Material, quads: &[GpuQuad], layers: &[Layer]) -> TopView {
    let mean = |layer: u16, tint: [u8; 3], uv: [f32; 4], cover: f32| -> [u8; 4] {
        let Some(px) = layers.get(layer as usize).filter(|_| layer != 0) else {
            return [tint[0], tint[1], tint[2], (cover * 255.0) as u8];
        };
        let (x0, x1) = ((uv[0].min(uv[2]) * 16.0) as usize, ((uv[0].max(uv[2]) * 16.0).ceil() as usize).clamp(1, 16));
        let (y0, y1) = ((uv[1].min(uv[3]) * 16.0) as usize, ((uv[1].max(uv[3]) * 16.0).ceil() as usize).clamp(1, 16));
        let (mut sum, mut alpha, mut n) = ([0u64; 3], 0u64, 0u64);
        for y in y0.min(15)..y1.max(y0 + 1) {
            for x in x0.min(15)..x1.max(x0 + 1) {
                let p = &px[(y * SIDE + x) * 4..][..4];
                for k in 0..3 {
                    sum[k] += p[k] as u64 * p[3] as u64;
                }
                alpha += p[3] as u64;
                n += 1;
            }
        }
        let c = |k: usize| (sum[k].checked_div(alpha).unwrap_or(0) * tint[k] as u64 / 255) as u8;
        let a = alpha as f32 / (n.max(1) * 255) as f32;
        [c(0), c(1), c(2), (a * cover * 255.0).round() as u8]
    };
    let whole = [0.0, 0.0, 1.0, 1.0];
    let kind = match m.class {
        Class::Air => return TopView::AIR,
        Class::Water => TopKind::Water,
        Class::Lava => TopKind::Lava,
        Class::Opaque | Class::Cutout => TopKind::Block,
        Class::Shaped => TopKind::Overlay,
    };
    if kind != TopKind::Overlay {
        let mut v = TopView { kind, layer: m.top, tint: m.top_tint, uv: whole, area: whole, mean: [0; 4], light: m.light };
        v.mean = mean(m.top, m.top_tint, whole, 1.0);
        if m.class == Class::Cutout {
            v.mean[3] = 255;
        }
        return v;
    }
    // A shaped block: the faces looking up, the highest first.
    let f = |w: u32| f32::from_bits(w);
    let faces: Vec<&GpuQuad> = quads.iter().skip(m.first_quad as usize).take(m.quads as usize).collect();
    let normal_y = |q: &GpuQuad| {
        let (u, v) = ([f(q[4]), f(q[5]), f(q[6])], [f(q[8]), f(q[9]), f(q[10])]);
        let n = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt().max(1e-6);
        n[1] / len
    };
    let rect = |q: &GpuQuad| {
        let (o, u, v) = ([f(q[0]), f(q[2])], [f(q[4]), f(q[6])], [f(q[8]), f(q[10])]);
        let xs = [o[0], o[0] + u[0], o[0] + v[0], o[0] + u[0] + v[0]];
        let zs = [o[1], o[1] + u[1], o[1] + v[1], o[1] + u[1] + v[1]];
        let lo = |a: [f32; 4]| a.iter().copied().fold(f32::MAX, f32::min).clamp(0.0, 1.0);
        let hi = |a: [f32; 4]| a.iter().copied().fold(f32::MIN, f32::max).clamp(0.0, 1.0);
        [lo(xs), lo(zs), hi(xs), hi(zs)]
    };
    let mut up: Vec<&GpuQuad> = faces.iter().copied().filter(|q| normal_y(q).abs() > 0.7).collect();
    if up.iter().any(|q| normal_y(q) > 0.7) {
        up.retain(|q| normal_y(q) > 0.7);
    }
    up.sort_by(|a, b| f(b[1]).total_cmp(&f(a[1])));
    let tint_of = |q: &GpuQuad| if q[3] >> 18 & 1 == 1 { m.top_tint } else { [255; 3] };
    let uv_of = |q: &GpuQuad| [f(q[12]), f(q[13]), f(q[14]), f(q[15])];
    let Some(&first) = up.first().or(faces.first()) else {
        return TopView::AIR;
    };
    let layer = (first[3] & 0xffff) as u16;
    // How much of the square the faces looking up cover, on a 16 × 16 grid.
    let mut covered = [false; 256];
    for q in &up {
        let r = rect(q);
        for z in (r[1] * 16.0).round() as usize..(r[3] * 16.0).round() as usize {
            for x in (r[0] * 16.0).round() as usize..(r[2] * 16.0).round() as usize {
                covered[(z.min(15)) * 16 + x.min(15)] = true;
            }
        }
    }
    let cover = covered.iter().filter(|c| **c).count() as f32 / 256.0;
    let tint = tint_of(first);
    if cover >= 0.85 {
        let mut v = TopView { kind: TopKind::Block, layer, tint, uv: whole, area: whole, mean: [0; 4], light: m.light };
        v.mean = mean(layer, tint, whole, 1.0);
        v.mean[3] = 255;
        return v;
    }
    let (uv, area) = if up.is_empty() { (uv_of(first), whole) } else { (uv_of(first), rect(first)) };
    let size = ((area[2] - area[0]) * (area[3] - area[1])).clamp(0.0, 1.0);
    let mut v = TopView { kind: TopKind::Overlay, layer, tint, uv, area, mean: [0; 4], light: m.light };
    v.mean = mean(layer, tint, uv, if up.is_empty() { 0.6 } else { size });
    v
}
