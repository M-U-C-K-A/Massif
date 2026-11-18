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

use crate::compose::{Materializer, Span, TileStack, hash2};
use crate::coords::{BlockRect, TilePos, local_index};
use crate::registry::{Registry, is_soil};
use serde::{Deserialize, Serialize};

/// `state` with property `key` set to `value`, added when it has none.
pub fn set_property(state: &str, key: &str, value: &str) -> String {
    let (name, rest) = state.split_once('[').unwrap_or((state, ""));
    let mut props: Vec<String> = rest.trim_end_matches(']').split(',').map(str::trim).filter(|p| !p.is_empty() && p.split('=').next() != Some(key)).map(str::to_owned).collect();
    props.push(format!("{key}={value}"));
    format!("{name}[{}]", props.join(","))
}

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
    /// Its tree size, set by hand: an index into
    /// [`Registry::tree_sizes`](crate::registry::Registry::tree_sizes)
    /// plus one; 0 to have it guessed from its height among the objects
    /// of its layer (see [`ObjectLayerDef::size_classes`]).
    #[serde(default, skip_serializing_if = "is_zero_u8")]
    pub tree_size: u8,
    /// How it stands, its own rules rather than its layer's: WorldPainter's
    /// object attributes. `None`: as its layer says.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rules: Option<Box<ObjectRules>>,
    /// The radius of its canopy in blocks, set by hand; 0 to have it
    /// measured from its leaves (see [`ObjectDef::canopy_radius`]).
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub canopy: f32,
    /// Its canopy's radius as measured, once.
    #[serde(skip)]
    pub measured: Measured,
}

/// A value measured from an object's blocks once, then kept (never part
/// of what the object is: two objects are equal whatever was measured).
#[derive(Debug, Clone, Default)]
pub struct Measured(std::sync::OnceLock<f32>);

impl PartialEq for Measured {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

fn is_zero_f32(n: &f32) -> bool {
    *n == 0.0
}

fn is_zero_u8(n: &u8) -> bool {
    *n == 0
}

/// How an object is turned: at random a quarter turn, or always as given.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Turning {
    Random,
    /// Quarter turns clockwise, 0 to 3.
    Fixed(u8),
}

/// How an object is mirrored (east and west swapped).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Mirroring {
    Random,
    Never,
    Always,
}

/// What blocks already there an object's blocks give way to, as
/// WorldPainter's collision and underground modes: `All` gives way to
/// any block (it only takes the place of air), `Solid` to solid blocks
/// (it takes the place of air, water and plants), `Nothing` takes the
/// place of everything.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Collision {
    All,
    Solid,
    Nothing,
}

/// Which of an object's blocks in the ground take the place of the ground,
/// as WorldPainter's underground modes: all of them, its solid ones (the
/// others only where there is air), or none (they only go where there is
/// air).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Underground {
    All,
    Solid,
    None,
}

/// What an object's leaves do in the game.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Leaves {
    /// As the object has them.
    AsTheyAre,
    /// They fall away when no log holds them.
    Decay,
    /// They never fall away.
    Persistent,
}

/// An object's own rules, all of WorldPainter's object attributes: where
/// it may stand, how it is turned and set in the ground, what it gives
/// way to. They stand in for its layer's turning, mirroring, Under water
/// and Replace the ground; the layer's Only on soil and Sink still hold.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct ObjectRules {
    pub turning: Turning,
    pub mirroring: Mirroring,
    /// On dry land.
    pub on_land: bool,
    /// Without a solid block under it: at a fixed height over the ground,
    /// it floats rather than being left out.
    pub on_air: bool,
    /// On the bottom of water, under it.
    pub under_water: bool,
    /// On the top of water, floating.
    pub on_water: bool,
    /// Standing on water, its blocks under the water stop at the floor
    /// (rather than going through it).
    pub collide_with_floor: bool,
    /// On the bottom of lava, and on its top (a map of lava seas).
    pub under_lava: bool,
    pub on_lava: bool,
    /// What its blocks over the ground give way to.
    pub collision: Collision,
    /// Which of its blocks in the ground replace it.
    pub underground: Underground,
    pub leaves: Leaves,
    /// Its blocks that can hold water do where they stand in water, and
    /// do not out of it; otherwise they stay as the object has them.
    pub manage_waterlogged: bool,
    /// A block of the object (a state, `minecraft:glass`) placed as air
    /// rather than itself: to carve a hollow underground.
    pub replace_with_air: Option<String>,
    /// Its lowest blocks go on down to the ground when it would float.
    pub extend_foundation: bool,
    /// At a fixed height (`offset` is then that height) rather than over
    /// the ground.
    pub fixed_height: bool,
    /// How many blocks over the ground (or, at a fixed height, at what
    /// height) its anchor stands.
    pub offset: i16,
    /// How many blocks higher or lower it may stand, at random: 0 to
    /// this, about evenly up and down.
    pub variation: u8,
    /// Fences, panes, bars and walls of the object join their neighbours.
    pub connect_blocks: bool,
}

impl Default for ObjectRules {
    /// WorldPainter's defaults.
    fn default() -> Self {
        Self {
            turning: Turning::Random,
            mirroring: Mirroring::Random,
            on_land: true,
            on_air: false,
            under_water: false,
            on_water: false,
            collide_with_floor: true,
            under_lava: false,
            on_lava: false,
            collision: Collision::Solid,
            underground: Underground::All,
            leaves: Leaves::AsTheyAre,
            manage_waterlogged: true,
            replace_with_air: None,
            extend_foundation: false,
            fixed_height: false,
            offset: 0,
            variation: 0,
            connect_blocks: false,
        }
    }
}

impl ObjectRules {
    /// The rules a layer gives its objects, as an object's own.
    pub fn of_layer(layer: &ObjectLayerDef) -> Self {
        Self {
            turning: if layer.rotate { Turning::Random } else { Turning::Fixed(0) },
            mirroring: if layer.mirror { Mirroring::Random } else { Mirroring::Never },
            on_land: !layer.underwater,
            under_water: layer.underwater,
            collision: if layer.replace { Collision::Nothing } else { Collision::Solid },
            underground: if layer.replace { Underground::All } else { Underground::None },
            offset: 0,
            ..Self::default()
        }
    }

    /// Whether it may stand where the top is dry (`None`), under water
    /// or lava (`Some(lava)`), on the floor (`floor`) or on the top of the
    /// liquid.
    pub fn may_stand(&self, liquid: Option<bool>, floor: bool) -> bool {
        match (liquid, floor) {
            (None, _) => self.on_land,
            (Some(false), true) => self.under_water,
            (Some(false), false) => self.on_water,
            (Some(true), true) => self.under_lava,
            (Some(true), false) => self.on_lava,
        }
    }
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
            tree_size: 0,
            rules: None,
            canopy: 0.0,
            measured: Measured::default(),
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

    /// The object made smaller (or larger) by `scale`, named `name`: each
    /// block of the new box takes the most telling block of the part of the
    /// old box it covers — wood over leaves over anything else over air —,
    /// so that a trunk stays whole and a crown stays a crown.
    pub fn scaled(&self, scale: f32, name: &str) -> ObjectDef {
        let scale = scale.clamp(0.1, 4.0);
        let old = self.size.map(|s| s as usize);
        let new = self.size.map(|s| ((s as f32 * scale).round() as usize).max(1));
        let rank = |k: u16| -> u8 {
            let state = self.palette.get(k as usize).map_or("", |s| s.as_str());
            match state {
                _ if k == 0 => 0,
                s if ["_log", "_wood", "_stem", "_hyphae", "mushroom_stem"].iter().any(|w| s.contains(w)) => 3,
                s if s.contains("leaves") => 2,
                _ => 1,
            }
        };
        let span = |n: usize, axis: usize| -> (usize, usize) {
            let a = ((n as f32 / scale).floor() as usize).min(old[axis] - 1);
            let b = (((n + 1) as f32 / scale).floor() as usize).clamp(a + 1, old[axis]);
            (a, b)
        };
        let mut blocks = vec![0u16; new[0] * new[1] * new[2]];
        for y in 0..new[1] {
            let (y0, y1) = span(y, 1);
            for z in 0..new[2] {
                let (z0, z1) = span(z, 2);
                for x in 0..new[0] {
                    let (x0, x1) = span(x, 0);
                    let mut best = (0u8, 0u16);
                    for sy in y0..y1 {
                        for sz in z0..z1 {
                            for sx in x0..x1 {
                                let k = self.at(sx, sy, sz);
                                let r = rank(k);
                                if r > best.0 {
                                    best = (r, k);
                                }
                            }
                        }
                    }
                    blocks[x + z * new[0] + y * new[0] * new[2]] = best.1;
                }
            }
        }
        let mut def = ObjectDef::new(name, new.map(|s| s as u16), self.palette.clone(), blocks);
        def.anchor = std::array::from_fn(|k| ((self.anchor[k] as f32 * scale).floor() as u16).min(new[k] as u16 - 1));
        def.source = self.source.clone();
        def.rules = self.rules.clone();
        def
    }

    /// The radius of its canopy, in blocks: set by hand, or measured (see
    /// [`ObjectDef::measure_canopy`]).
    pub fn canopy_radius(&self) -> f32 {
        if self.canopy > 0.0 { self.canopy } else { self.measured_canopy() }
    }

    /// [`ObjectDef::measure_canopy`], measured once.
    pub fn measured_canopy(&self) -> f32 {
        *self.measured.0.get_or_init(|| self.measure_canopy())
    }

    /// The radius of its canopy as its blocks say: how far from the middle
    /// of its leaves the columns holding leaves reach, the farthest tenth
    /// of them left out (a branch or a leaf sticking out); its trunk is not
    /// leaves. An object without leaves (a rock, a dead tree) is measured by
    /// all its blocks. Half a block at least.
    pub fn measure_canopy(&self) -> f32 {
        let [w, h, l] = self.size.map(|s| s as usize);
        let leafy: Vec<bool> = self.palette.iter().map(|s| s.split('[').next().unwrap_or(s).ends_with("leaves")).collect();
        let columns = |leaves_only: bool| -> Vec<(f32, f32)> {
            let mut out = Vec::new();
            for z in 0..l {
                for x in 0..w {
                    if (0..h).any(|y| {
                        let k = self.at(x, y, z) as usize;
                        k != 0 && (!leaves_only || leafy[k])
                    }) {
                        out.push((x as f32 + 0.5, z as f32 + 0.5));
                    }
                }
            }
            out
        };
        let mut cols = columns(true);
        if cols.is_empty() {
            cols = columns(false);
        }
        if cols.is_empty() {
            return 0.5;
        }
        let n = cols.len() as f32;
        let (cx, cz) = (cols.iter().map(|c| c.0).sum::<f32>() / n, cols.iter().map(|c| c.1).sum::<f32>() / n);
        let mut d: Vec<f32> = cols.iter().map(|c| ((c.0 - cx).powi(2) + (c.1 - cz).powi(2)).sqrt()).collect();
        d.sort_by(f32::total_cmp);
        let k = ((d.len() as f32 * 0.9).ceil() as usize).clamp(1, d.len()) - 1;
        (d[k] + 0.5).max(0.5)
    }

    /// The object as its own rules place it, when they change its blocks:
    /// its leaves made to decay or not, its fences, panes, bars and walls
    /// joined to their neighbours in it. `None` when they change nothing.
    pub fn shaped(&self) -> Option<ObjectDef> {
        let rules = self.rules.as_deref()?;
        if rules.leaves == Leaves::AsTheyAre && !rules.connect_blocks {
            return None;
        }
        let mut def = self.clone();
        if rules.leaves != Leaves::AsTheyAre {
            let persistent = if rules.leaves == Leaves::Persistent { "true" } else { "false" };
            for state in def.palette.iter_mut().skip(1) {
                let name = state.split('[').next().unwrap_or(state);
                if name.ends_with("leaves") {
                    *state = canonical(&set_property(state, "persistent", persistent));
                }
            }
        }
        if rules.connect_blocks {
            let [w, h, l] = self.size.map(|s| s as usize);
            let kind = |k: u16| -> Option<&'static str> {
                let name = self.palette.get(k as usize)?.split('[').next()?;
                if name.ends_with("_fence") {
                    Some("fence")
                } else if name.ends_with("_pane") || name.ends_with("iron_bars") {
                    Some("pane")
                } else if name.ends_with("_wall") {
                    Some("wall")
                } else {
                    None
                }
            };
            // Whether a block of the object a fence or a pane joins: its
            // own kind, or a whole block (not a plant, leaves or the like).
            let joins = |k: u16, own: &str| -> bool {
                if k == 0 {
                    return false;
                }
                if let Some(other) = kind(k) {
                    return other == own || own != "fence" && other != "fence";
                }
                let state = &self.palette[k as usize];
                let name = state.split('[').next().unwrap_or(state);
                crate::registry::plant_color(state).is_none() && !["leaves", "slab", "stairs", "door", "torch", "button", "sign", "carpet", "trapdoor", "ladder", "lantern", "chain", "rail"].iter().any(|w| name.contains(w))
            };
            for y in 0..h {
                for z in 0..l {
                    for x in 0..w {
                        let i = x + z * w + y * w * l;
                        let k = self.blocks[i];
                        let Some(own) = kind(k) else { continue };
                        let near = |dx: i32, dz: i32| -> bool {
                            let (nx, nz) = (x as i32 + dx, z as i32 + dz);
                            nx >= 0 && nz >= 0 && (nx as usize) < w && (nz as usize) < l && joins(self.blocks[nx as usize + nz as usize * w + y * w * l], own)
                        };
                        let (on, off) = if own == "wall" { ("low", "none") } else { ("true", "false") };
                        let mut state = self.palette[k as usize].clone();
                        for (side, dx, dz) in [("north", 0, -1), ("east", 1, 0), ("south", 0, 1), ("west", -1, 0)] {
                            state = set_property(&state, side, if near(dx, dz) { on } else { off });
                        }
                        let state = canonical(&state);
                        let n = match def.palette.iter().position(|p| *p == state) {
                            Some(n) => n,
                            None => {
                                def.palette.push(state);
                                def.palette.len() - 1
                            }
                        };
                        def.blocks[i] = n as u16;
                    }
                }
            }
        }
        Some(def)
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
        let bytes = crate::codec::zstd_decompress_section(&packed).map_err(serde::de::Error::custom)?;
        Ok(bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect())
    }
}

pub(crate) mod base64 {
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
    /// Left off the map and the 3D view; the export writes it all the same.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub hidden: bool,
    /// A sub-canopy layer: it is painted beside the base layer of a column
    /// rather than in its place, and its objects grow under the canopy,
    /// in the air the base layer's leave (hazel, holly, bushes).
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub understory: bool,
    /// Locked: brushes neither paint another object layer of its slot over
    /// its blocks nor rub it out.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub locked: bool,
    /// Its place among the object layers in the Layers panel (0: by its
    /// number).
    #[serde(default, skip_serializing_if = "crate::registry::is_zero")]
    pub order: u32,
    /// The palette it is sorted into in the Layers panel ("": none, it
    /// stays in its section).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub palette: String,
    /// How its objects are spaced: on a grid of its spacing, or by their
    /// canopies. Layers of before canopies were measured keep the grid.
    #[serde(default, skip_serializing_if = "Spacing::is_grid")]
    pub spacing_mode: Spacing,
    /// By canopies: the least distance between two trunks, in blocks.
    #[serde(default = "default_gap", skip_serializing_if = "is_default_gap")]
    pub min_gap: u8,
    /// The density of each tree size (by size − 1), as a share of the
    /// layer's: 1 for a size not listed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub size_density: Vec<f32>,
}

/// How an object layer spaces its objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Spacing {
    /// At most one in each square of the layer's spacing: the same
    /// distance for every object, as WorldPainter places them.
    #[default]
    Grid,
    /// By their canopies: two neighbours stand at least as far apart as
    /// their canopies' radii added (their crowns touch, at full density,
    /// and do not overlap), and never closer than the layer's least
    /// distance between trunks.
    Canopy,
}

impl Spacing {
    fn is_grid(&self) -> bool {
        *self == Spacing::Grid
    }
}

fn default_gap() -> u8 {
    3
}

fn is_default_gap(n: &u8) -> bool {
    *n == 3
}

impl ObjectLayerDef {
    /// The density of tree size `size` (from 1; 0: any), as a share of the
    /// layer's.
    pub fn size_share(&self, size: u16) -> f32 {
        (size as usize).checked_sub(1).and_then(|k| self.size_density.get(k)).copied().unwrap_or(1.0).clamp(0.0, 1.0)
    }
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
            hidden: false,
            understory: false,
            locked: false,
            order: 0,
            palette: String::new(),
            spacing_mode: Spacing::Grid,
            min_gap: default_gap(),
            size_density: Vec::new(),
        }
    }

    /// The size of each of its objects (in the order of
    /// [`ObjectLayerDef::objects`]), a size of
    /// [`Registry::tree_sizes`] from 1: the size set on the object by
    /// hand, else a guess from its height among the others guessed, the
    /// lowest a quarter bushes, then small, medium and large trees.
    pub fn size_classes(&self, reg: &Registry) -> Vec<u8> {
        let def = |k: usize| self.objects.get(k).and_then(|o| reg.objects.get(o.0 as usize));
        let height = |k: usize| def(k).map_or(0, |d| d.size[1]);
        let set = |k: usize| def(k).map_or(0, |d| reg.live_size(d.tree_size as u16) as u8);
        let mut order: Vec<usize> = (0..self.objects.len()).filter(|&k| set(k) == 0).collect();
        order.sort_by_key(|&k| (height(k), k));
        let n = order.len().max(1);
        let most = reg.tree_sizes.len().clamp(1, 4);
        let mut classes: Vec<u8> = (0..self.objects.len()).map(set).collect();
        for (rank, k) in order.into_iter().enumerate() {
            classes[k] = (1 + rank * 4 / n).min(most) as u8;
        }
        classes
    }

    /// The object one of its spots gets, from the draw `r` of its
    /// [`Candidate`]: by their weights among all of them for size 0 (any),
    /// else among those of the size asked (see
    /// [`ObjectLayerDef::size_classes`], given as `classes`), or the nearest
    /// size it has.
    pub fn pick(&self, r: u32, size: u16, classes: &[u8]) -> Option<u32> {
        // The size it has nearest to the one asked, the smaller of two as
        // near.
        let class = if size == 0 { None } else { classes.iter().copied().min_by_key(|&c| ((c as i32 - size as i32).abs(), c)) };
        let wanted = |k: usize| class.is_none_or(|c| classes.get(k) == Some(&c));
        // An object of weight 0 stands only where its size is asked for.
        let weight = |w: u16| if size == 0 { w as u32 } else { w.max(1) as u32 };
        let total: u32 = self.objects.iter().enumerate().filter(|(k, _)| wanted(*k)).map(|(_, o)| weight(o.1)).sum();
        if total == 0 {
            return None;
        }
        let mut r = r % total;
        self.objects.iter().enumerate().filter(|(k, _)| wanted(*k)).find(|(_, o)| {
            if r < weight(o.1) {
                true
            } else {
                r -= weight(o.1);
                false
            }
        }).map(|(_, o)| o.0)
    }

    /// Where an object of this layer, numbered `number`, may stand in
    /// square `(i, j)` of its grid, before the land there is asked: the
    /// spot, the object, its turn. `None` when the square stays empty.
    pub fn candidate(&self, seed: u64, number: u16, i: i32, j: i32) -> Option<Candidate> {
        self.candidate_in(seed, number, 0, self.spacing.max(1) as i32, self.density, i, j)
    }

    /// [`ObjectLayerDef::candidate`] in a grid of its own (`size`, 0 for
    /// the layer's), of squares `spacing` blocks wide, standing in a share
    /// `density` of them.
    #[allow(clippy::too_many_arguments)]
    pub fn candidate_in(&self, seed: u64, number: u16, size: u16, spacing: i32, density: f32, i: i32, j: i32) -> Option<Candidate> {
        if self.removed {
            return None;
        }
        let s = spacing.max(1);
        let salt = seed ^ (number as u64).wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ 0x6f62_6a65_6374 ^ (size as u64).wrapping_mul(0xa076_1d64_78bd_642f);
        let h = |k: u64| hash2(i, j, salt.wrapping_add(k.wrapping_mul(0x5851_f42d_4c95_7f2d)));
        let density = density.clamp(0.0, 1.0);
        if density < 1.0 && h(0) as f32 >= density * u32::MAX as f32 {
            return None;
        }
        let pick = h(1);
        let object = self.pick(pick, 0, &[])?;
        // Kept off the edges of the square: two objects are never closer
        // than a third of the spacing.
        let margin = s / 6;
        let room = (s - 2 * margin).max(1) as u32;
        Some(Candidate {
            x: i * s + margin + (h(2) % room) as i32,
            z: j * s + margin + (h(3) % room) as i32,
            object,
            pick,
            turn: if self.rotate { (h(4) % 4) as u8 } else { 0 },
            mirror: self.mirror && h(5) & 1 == 1,
            spin: (h(4) % 4) as u8,
            flip: h(5) & 1 == 1,
            lift: h(6),
            keep: h(7),
            rank: h(8),
        })
    }
}

/// Where an object of a layer may stand, before the land is asked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub x: i32,
    pub z: i32,
    /// The object, of any size.
    pub object: u32,
    /// The draw that chose it, to choose again among the objects of one
    /// size (see [`ObjectLayerDef::pick`]).
    pub pick: u32,
    pub turn: u8,
    pub mirror: bool,
    /// Its draws for the objects of their own rules: a quarter turn, a
    /// mirroring, a height within their variation.
    pub spin: u8,
    pub flip: bool,
    pub lift: u32,
    /// Its draw against the density of its tree size.
    pub keep: u32,
    /// Spaced by canopies, its rank: of two too close, the higher stands.
    pub rank: u32,
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
    /// Of a sub-canopy layer: it only takes the place of air (and water and
    /// plants), never of the base layer's objects.
    pub under: bool,
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
    // Only a rail's shape names two sides: a mod's shapes keep theirs
    // (Conquest Reforged's `three_middle`).
    if !SIDES.contains(&a) || !SIDES.contains(&b) {
        return shape.to_owned();
    }
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
            // Conquest Reforged's left and right shapes (`two_l`, `edge_r`).
            "shape" if v.ends_with("_l") || v.ends_with("_r") => {
                if mirror {
                    let (stem, side) = v.split_at(v.len() - 1);
                    format!("{stem}{}", if side == "l" { "r" } else { "l" })
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
            // Conquest Reforged's arches and beams lie north–south or
            // east–west.
            "direction" if turn % 2 == 1 && (v == "northsouth" || v == "eastwest") => (if v == "northsouth" { "eastwest" } else { "northsouth" }).into(),
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
    // The layers painted on the tile at all: the base ones, then the
    // sub-canopy ones, which stand where the base ones leave air.
    let mut painted: Vec<(bool, u16)> = Vec::new();
    for (_, _, tile) in stack.layers() {
        for (under, plane) in [(false, &tile.objects), (true, &tile.understory)] {
            match plane.uniform_value() {
                Some(0) => {}
                Some(v) => painted.push((under, v)),
                None => painted.extend(plane.dense().iter().copied().filter(|&v| v != 0).map(|v| (under, v))),
            }
        }
        painted.sort_unstable();
        painted.dedup();
    }
    if painted.is_empty() {
        return out;
    }
    let (ox, oz) = pos.origin();
    let (mut spans, mut fluids) = (Vec::new(), Vec::new());
    for (under, n) in painted {
        let Some(layer) = reg.object_layer(n) else { continue };
        let classes = layer.size_classes(reg);
        let class_of = |object: u32| layer.objects.iter().position(|o| o.0 == object).and_then(|row| classes.get(row)).map_or(0, |&c| c as u16);
        // What is painted on the top of a column of the tile: its layer
        // there, and the size asked.
        let top_of = |x: i32, z: i32, spans: &mut Vec<Span>| {
            let cell = local_index(x, z);
            stack.spans(cell, spans);
            let top = spans.last().copied()?;
            let here = if under { stack.surface_understory(cell, &top) } else { stack.surface_objects(cell, &top) };
            (here == n).then(|| (top, reg.live_size(stack.surface_size(cell, &top))))
        };
        // The object of a spot, and its size: of the size asked there, or
        // any.
        let choose = |c: &Candidate, asked: u16, thin: bool| -> Option<(u32, u16)> {
            let (object, size) = match asked {
                0 => (c.object, class_of(c.object)),
                size => (layer.pick(c.pick, size, &classes)?, size),
            };
            reg.objects.get(object as usize)?;
            // The density of its size.
            (!thin || layer.size_density.is_empty() || (c.keep as f32) < layer.size_share(size) * u32::MAX as f32).then_some((object, size))
        };
        let spots: Vec<(Candidate, u32, u16)> = match layer.spacing_mode {
            Spacing::Grid => {
                let mut spots = Vec::new();
                let s = layer.spacing.max(1) as i32;
                let (i0, i1) = (ox.div_euclid(s), (ox + 127).div_euclid(s));
                let (j0, j1) = (oz.div_euclid(s), (oz + 127).div_euclid(s));
                for j in j0..=j1 {
                    for i in i0..=i1 {
                        let Some(c) = layer.candidate(seed, n, i, j) else { continue };
                        if TilePos::containing(c.x, c.z) != pos || reg.objects.get(c.object as usize).is_none() {
                            continue;
                        }
                        let Some((_, asked)) = top_of(c.x, c.z, &mut spans) else { continue };
                        if let Some((object, size)) = choose(&c, asked, true) {
                            spots.push((c, object, size));
                        }
                    }
                }
                spots
            }
            Spacing::Canopy => canopy_spots(reg, layer, seed, n, under, stack, pos, &choose),
        };
        for (c, object, _) in spots {
            let cell = local_index(c.x, c.z);
            stack.spans(cell, &mut spans);
            let Some(top) = spans.last().copied() else { continue };
            stack.fluids(cell, &spans, &mut fluids);
            let liquid = fluids.iter().find(|f| f.0 == top.y1).copied();
            let wet = liquid.is_some();
            let Some(rules) = reg.objects[object as usize].rules.as_deref() else {
                if wet != layer.underwater || (wet && lava) {
                    continue;
                }
                if layer.soil_only && !is_soil(&reg.block(mat.block(&top, c.x, top.y1 - 1, c.z, wet)).state) {
                    continue;
                }
                out.push(Placement {
                    layer: n,
                    object,
                    x: c.x,
                    y: top.y1 - layer.sink as i32,
                    z: c.z,
                    turn: c.turn,
                    mirror: c.mirror,
                    under,
                });
                continue;
            };
            // Its own rules: on the floor of the water or lava, or on its
            // top, if it may.
            let kind = liquid.map(|_| lava);
            let floor = !wet || rules.may_stand(kind, true);
            if !rules.may_stand(kind, floor) {
                continue;
            }
            if floor && layer.soil_only && !is_soil(&reg.block(mat.block(&top, c.x, top.y1 - 1, c.z, wet)).state) {
                continue;
            }
            let ground = if floor { top.y1 } else { liquid.map_or(top.y1, |f| f.1) };
            let v = rules.variation as i32;
            let lift = if v > 0 { (c.lift % (v as u32 + 1)) as i32 - (v + 1) / 2 } else { 0 };
            let y = if rules.fixed_height { rules.offset as i32 + lift } else { ground + rules.offset as i32 + lift - layer.sink as i32 };
            // Floating over the ground only when it may stand on air.
            if rules.fixed_height && y > ground && !rules.on_air {
                continue;
            }
            out.push(Placement {
                layer: n,
                object,
                x: c.x,
                y,
                z: c.z,
                turn: match rules.turning {
                    Turning::Random => c.spin,
                    Turning::Fixed(t) => t & 3,
                },
                mirror: match rules.mirroring {
                    Mirroring::Random => c.flip,
                    Mirroring::Never => false,
                    Mirroring::Always => true,
                },
                under,
            });
        }
    }
    out
}

/// A spot of a layer spaced by canopies: where, the object, its tree size,
/// its canopy's radius.
type Spot = (Candidate, u32, u16, f32);

/// The object of a spot and its tree size, given the size asked there and
/// whether a size's density thins it (see [`tile_placements`]).
type Choose<'a> = &'a dyn Fn(&Candidate, u16, bool) -> Option<(u32, u16)>;

/// The spots of layer `n` (`layer`) spaced by their canopies standing with
/// their anchor in tile `pos`: its whole spacing model in one place.
///
/// Candidates lie on a fine grid (a square of the layer's least distance
/// between trunks), each with a rank, the wider crowns ranked first. A
/// candidate stands unless one of a higher rank that stands itself is too
/// close: closer than the two canopies' radii added, or than the least
/// distance between trunks.
/// That decides the same on every tile (a candidate's neighbours across
/// a tile's edge are read from what is painted on the ground there), and
/// at full density fills the land with crowns that touch without
/// overlapping, narrow and wide ones mixed. The layer's density, and each
/// size's, then thin the trees that stand.
#[allow(clippy::too_many_arguments)]
fn canopy_spots(reg: &Registry, layer: &ObjectLayerDef, seed: u64, n: u16, under: bool, stack: &TileStack, pos: TilePos, choose: Choose) -> Vec<(Candidate, u32, u16)> {
    use std::collections::HashMap;
    const SALT: u16 = 0xca;
    let gap = layer.min_gap.max(1) as f32;
    let g = (gap.ceil() as i32).max(2);
    let r_max = layer.objects.iter().filter_map(|o| reg.objects.get(o.0 as usize)).map(ObjectDef::canopy_radius).fold(0.0f32, f32::max);
    // What would stand in cell `(i, j)`, its neighbours aside.
    let look = |i: i32, j: i32| -> Option<Spot> {
        let c = layer.candidate_in(seed, n, SALT, g, 1.0, i, j)?;
        let (objects, understory, asked) = if TilePos::containing(c.x, c.z) == pos {
            let cell = local_index(c.x, c.z);
            let mut spans = Vec::new();
            stack.spans(cell, &mut spans);
            let top = spans.last().copied()?;
            (stack.surface_objects(cell, &top), stack.surface_understory(cell, &top), stack.surface_size(cell, &top))
        } else {
            stack.ground_layers_at(c.x, c.z)
        };
        if (if under { understory } else { objects }) != n {
            return None;
        }
        let (object, size) = choose(&c, reg.live_size(asked), false)?;
        Some((c, object, size, reg.objects[object as usize].canopy_radius()))
    };
    struct Model<'a> {
        look: &'a dyn Fn(i32, i32) -> Option<Spot>,
        seen: HashMap<(i32, i32), Option<Spot>>,
        decided: HashMap<(i32, i32), bool>,
        g: i32,
        gap: f32,
        r_max: f32,
    }
    impl Model<'_> {
        fn spot(&mut self, at: (i32, i32)) -> Option<Spot> {
            let look = self.look;
            *self.seen.entry(at).or_insert_with(|| look(at.0, at.1))
        }

        /// Whether the candidate of cell `at` stands.
        fn stands(&mut self, at: (i32, i32)) -> bool {
            if let Some(&d) = self.decided.get(&at) {
                return d;
            }
            let Some(a) = self.spot(at) else {
                self.decided.insert(at, false);
                return false;
            };
            let reach = ((a.3 + self.r_max).max(self.gap) / self.g as f32).ceil() as i32 + 1;
            // The wider crowns first, then by rank: the narrow trees fill
            // what the wide ones leave, as under a real canopy.
            let order = |s: &Spot, at: (i32, i32)| ((s.3 * 4.0) as u32, s.0.rank, at);
            let mine = order(&a, at);
            let mut free = true;
            'near: for dj in -reach..=reach {
                for di in -reach..=reach {
                    let at_b = (at.0 + di, at.1 + dj);
                    let Some(b) = (if (di, dj) == (0, 0) { None } else { self.spot(at_b) }) else { continue };
                    if order(&b, at_b) <= mine {
                        continue;
                    }
                    let need = (a.3 + b.3).max(self.gap);
                    let (dx, dz) = ((b.0.x - a.0.x) as f32, (b.0.z - a.0.z) as f32);
                    if dx * dx + dz * dz < need * need && self.stands(at_b) {
                        free = false;
                        break 'near;
                    }
                }
            }
            self.decided.insert(at, free);
            free
        }
    }
    let mut model = Model { look: &look, seen: HashMap::new(), decided: HashMap::new(), g, gap, r_max };
    let (ox, oz) = pos.origin();
    let mut out = Vec::new();
    for j in oz.div_euclid(g)..=(oz + 127).div_euclid(g) {
        for i in ox.div_euclid(g)..=(ox + 127).div_euclid(g) {
            let Some(a) = model.spot((i, j)) else { continue };
            // The density thins the trees that stand, leaving clearings
            // (rather than leaving room to others: at full density the
            // crowns would fill the land whatever it is).
            let kept = layer.density.clamp(0.0, 1.0) * layer.size_share(a.2);
            if TilePos::containing(a.0.x, a.0.z) == pos && model.stands((i, j)) && (kept >= 1.0 || (a.0.keep as f32) < kept * u32::MAX as f32) {
                out.push((a.0, a.1, a.2));
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
        // Conquest Reforged's own properties.
        assert_eq!(transform_state("conquest:limestone_small_arch[direction=northsouth,waterlogged=false]", 1, false), "conquest:limestone_small_arch[direction=eastwest,waterlogged=false]");
        assert_eq!(transform_state("conquest:limestone_small_arch[direction=northsouth,waterlogged=false]", 2, false), "conquest:limestone_small_arch[direction=northsouth,waterlogged=false]");
        assert_eq!(transform_state("conquest:limestone_capital[facing=flat,type=top]", 1, false), "conquest:limestone_capital[facing=flat,type=top]");
        assert_eq!(transform_state("conquest:limestone_capital[facing=north,type=top]", 1, false), "conquest:limestone_capital[facing=east,type=top]");
        assert_eq!(transform_state("conquest:oak_beam[shape=three_middle]", 1, false), "conquest:oak_beam[shape=three_middle]");
        assert_eq!(transform_state("conquest:oak_beam[shape=two_l]", 1, false), "conquest:oak_beam[shape=two_l]");
        assert_eq!(transform_state("conquest:oak_beam[shape=two_l]", 0, true), "conquest:oak_beam[shape=two_r]");
        assert_eq!(transform_state("conquest:wall[north=low,east=true,south=false,west=false]", 1, false), "conquest:wall[north=false,east=low,south=true,west=false]");
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
        let p = Placement { layer: 1, object: 0, x: 100, y: 64, z: 50, turn: 1, mirror: false, under: false };
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
            let stack = TileStack::from_above(dim, pos, true).unwrap();
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
        let stack = TileStack::from_above(dim, TilePos::containing(p.x, p.z), true).unwrap();
        let mut spans = Vec::new();
        stack.spans(local_index(p.x, p.z), &mut spans);
        assert_eq!(p.y, spans.last().unwrap().y1);
    }

    /// An object of its own rules: turned and mirrored as they say, raised,
    /// varied, at a fixed height only where it may float; the others of
    /// the layer stand as before.
    #[test]
    fn objects_stand_by_their_own_rules() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, seed: 5, ..GenParams::default() });
        w.registry.objects.push(tree());
        let flat = crate::Brush { radius: 60.0, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() };
        let mut layer = ObjectLayerDef::new("Oaks", &[0], &w.registry);
        (layer.soil_only, layer.density) = (false, 1.0);
        let n = w.registry.add_object_layer(layer);
        let mut h = History::default();
        h.begin("objects", 0);
        crate::ops::set_height(&mut h.edit(&mut w, 0), &flat, 0.0, 0.0, 80.0, crate::ops::Surface::Top);
        crate::ops::paint_objects(&mut h.edit(&mut w, 0), &crate::Brush { radius: 40.0, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() }, 0.0, 0.0, n, 1);
        h.commit();
        let placements = |w: &crate::world::World| {
            let dim = &w.dims[0];
            let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
            let mut all = Vec::new();
            for pos in [TilePos::new(-1, -1), TilePos::new(0, -1), TilePos::new(-1, 0), TilePos::new(0, 0)] {
                all.extend(tile_placements(&w.registry, &mat, 1, false, &TileStack::new(dim, pos, true).unwrap(), pos));
            }
            all
        };
        let before = placements(&w);
        // As its layer's: the same spots, turns and heights.
        w.registry.objects[0].rules = Some(Box::new(ObjectRules::of_layer(&w.registry.object_layers[0])));
        assert_eq!(placements(&w), before);
        // Turned a half turn, always mirrored, three blocks up, varied by
        // up to four.
        let rules = ObjectRules { turning: Turning::Fixed(2), mirroring: Mirroring::Always, offset: 3, variation: 4, ..ObjectRules::default() };
        w.registry.objects[0].rules = Some(Box::new(rules.clone()));
        let own = placements(&w);
        assert_eq!(own.len(), before.len());
        assert!(own.iter().all(|p| p.turn == 2 && p.mirror));
        let heights: std::collections::BTreeSet<i32> = own.iter().map(|p| p.y).collect();
        let ground = before[0].y;
        assert!(before.iter().all(|p| p.y == ground), "flat ground");
        assert_eq!(heights, (ground + 3 - 2..=ground + 3 + 2).collect(), "raised 3, ±2");
        // At a fixed height over the ground: left out unless it may float.
        w.registry.objects[0].rules = Some(Box::new(ObjectRules { fixed_height: true, offset: 120, ..rules.clone() }));
        assert!(placements(&w).is_empty());
        w.registry.objects[0].rules = Some(Box::new(ObjectRules { fixed_height: true, offset: 120, variation: 0, on_air: true, ..rules.clone() }));
        assert!(placements(&w).iter().all(|p| p.y == 120));
        // Not on land: none on dry ground.
        w.registry.objects[0].rules = Some(Box::new(ObjectRules { on_land: false, under_water: true, ..rules }));
        assert!(placements(&w).is_empty());
    }

    /// Own rules that change an object's blocks: leaves made to decay or
    /// not, fences joined to their neighbours in it.
    #[test]
    fn rules_shape_leaves_and_fences() {
        let mut fence = ObjectDef::new("Fence", [3, 1, 1], vec!["minecraft:air".into(), "minecraft:oak_fence".into(), "minecraft:oak_leaves[distance=7,persistent=false]".into()], vec![1, 1, 2]);
        assert_eq!(fence.shaped(), None, "no rules, as it is");
        fence.rules = Some(Box::new(ObjectRules::default()));
        assert_eq!(fence.shaped(), None, "rules that leave its blocks alone");
        fence.rules = Some(Box::new(ObjectRules { leaves: Leaves::Persistent, connect_blocks: true, ..ObjectRules::default() }));
        let shaped = fence.shaped().unwrap();
        let at = |x: usize| shaped.palette[shaped.at(x, 0, 0) as usize].as_str();
        assert_eq!(at(0), "minecraft:oak_fence[east=true,north=false,south=false,west=false]");
        assert_eq!(at(1), "minecraft:oak_fence[east=false,north=false,south=false,west=true]", "leaves do not join");
        assert_eq!(at(2), "minecraft:oak_leaves[distance=7,persistent=true]");
        assert_eq!(set_property("minecraft:stone", "a", "b"), "minecraft:stone[a=b]");
    }

    /// A column of `h` logs: an object `h` blocks tall.
    fn pole(name: &str, h: u16) -> ObjectDef {
        ObjectDef::new(name, [1, h, 1], vec!["minecraft:air".into(), "minecraft:oak_log[axis=y]".into()], vec![1; h as usize])
    }

    /// A layer's objects split by height into four sizes, a quarter each;
    /// a size asked for takes the objects of that size, or of the nearest
    /// it has (the smaller of two as near), by their weights.
    #[test]
    fn sizes_split_a_layers_objects_by_height() {
        let mut reg = Registry::vanilla();
        for h in [9u16, 2, 7, 4, 3, 8, 5, 6] {
            reg.objects.push(pole(&format!("{h} high"), h));
        }
        let all: Vec<u32> = (0..8).collect();
        let layer = ObjectLayerDef::new("Poles", &all, &reg);
        let classes = layer.size_classes(&reg);
        assert_eq!(classes, [4, 1, 3, 2, 1, 4, 2, 3]);
        let height = |o: u32| reg.objects[o as usize].size[1];
        for r in 0..200u32 {
            assert!(matches!(height(layer.pick(r, 1, &classes).unwrap()), 2 | 3));
            assert!(matches!(height(layer.pick(r, 4, &classes).unwrap()), 8 | 9));
        }
        // Size 0 picks as the candidate does, among all of them.
        let picked: std::collections::HashSet<u32> = (0..64).map(|r| layer.pick(r, 0, &classes).unwrap()).collect();
        assert_eq!(picked.len(), 8);
        let c = layer.candidate(3, 1, 4, 7).unwrap();
        assert_eq!(layer.pick(c.pick, 0, &classes), Some(c.object));
        // Fewer than four: the nearest size it has.
        let few = ObjectLayerDef::new("Two", &[1, 0], &reg);
        let classes = few.size_classes(&reg);
        assert_eq!(classes, [1, 3]);
        assert_eq!(few.pick(5, 4, &classes), Some(0), "large: the 9 high");
        assert_eq!(few.pick(5, 2, &classes), Some(1), "small: as near to the bush as to the medium, the smaller");
    }

    /// A size set by hand stands in for the guess from the height; the
    /// others are guessed among themselves.
    #[test]
    fn sizes_set_by_hand() {
        let mut reg = Registry::vanilla();
        for h in [9u16, 2, 7, 4] {
            reg.objects.push(pole(&format!("{h} high"), h));
        }
        reg.objects[0].tree_size = 1;
        reg.tree_sizes.push(crate::registry::TreeSize { name: "Forest".into(), color: [1, 2, 3], removed: false });
        reg.objects[2].tree_size = 5;
        let layer = ObjectLayerDef::new("Poles", &[0, 1, 2, 3], &reg);
        let classes = layer.size_classes(&reg);
        assert_eq!(classes[0], 1, "the 9 high, a bush by hand");
        assert_eq!(classes[2], 5, "a size of the map's own");
        assert_eq!((classes[1], classes[3]), (1, 3), "the two others guessed between themselves: the lower half, then the upper");
        assert_eq!(layer.pick(17, 5, &classes), Some(2));
    }

    /// A tree of a round crown `r` blocks across from its trunk: a trunk
    /// three high, a disc of leaves on it.
    fn crowned(name: &str, r: i32) -> ObjectDef {
        let side = (2 * r + 1) as usize;
        let mut blocks = vec![0u16; side * 4 * side];
        let at = |x: usize, y: usize, z: usize| x + z * side + y * side * side;
        for y in 0..3 {
            blocks[at(r as usize, y, r as usize)] = 1;
        }
        for z in 0..side {
            for x in 0..side {
                let (dx, dz) = (x as i32 - r, z as i32 - r);
                if dx * dx + dz * dz <= r * r {
                    blocks[at(x, 3, z)] = 2;
                }
            }
        }
        ObjectDef::new(name, [side as u16, 4, side as u16], vec!["minecraft:air".into(), "minecraft:oak_log[axis=y]".into(), "minecraft:oak_leaves".into()], blocks)
    }

    /// A canopy is measured from its leaves, not its trunk, a stray leaf
    /// left out; by hand, it is as set.
    #[test]
    fn canopies_are_measured_from_the_leaves() {
        let mut wide = crowned("Wide", 5);
        assert!((wide.canopy_radius() - 5.0).abs() < 0.8, "{}", wide.canopy_radius());
        let narrow = crowned("Narrow", 1);
        assert!(narrow.canopy_radius() < 2.0, "{}", narrow.canopy_radius());
        // A stray leaf far out on a branch hardly counts.
        let mut stray = crowned("Stray", 5);
        let side = 11;
        stray.blocks[side - 1] = 2;
        stray.measured = Measured::default();
        assert!(stray.canopy_radius() < wide.canopy_radius() + 0.6);
        wide.canopy = 7.5;
        assert_eq!(wide.canopy_radius(), 7.5);
        assert!((pole("Rock", 3).canopy_radius() - 0.5).abs() < 1e-3, "no leaves: by its blocks");
    }

    /// A forest of narrow and wide trees mixed, spaced by their canopies at
    /// full density: no two crowns overlap, across the edges of tiles too,
    /// and they fill the land with no big gaps — measured as the share of
    /// the land under a crown, and the widest clearing. On a grid wide
    /// enough for the wide crowns, as before, the narrow trees stand far
    /// apart and the land is barer. A size's density thins its trees.
    #[test]
    fn trees_are_spaced_by_their_canopies() {
        let mut w = generate(&GenParams { tiles_x: 4, tiles_z: 2, seed: 5, ..GenParams::default() });
        for (name, r) in [("Narrow", 1), ("Slim", 2), ("Wide", 5), ("Broad", 6)] {
            let mut t = crowned(name, r);
            t.tree_size = if r < 3 { 2 } else { 4 };
            w.registry.objects.push(t);
        }
        let mut forest = ObjectLayerDef::new("Forest", &[0, 1, 2, 3], &w.registry);
        (forest.soil_only, forest.density, forest.spacing, forest.spacing_mode) = (false, 1.0, 14, Spacing::Canopy);
        let n = w.registry.add_object_layer(forest);
        let flat = crate::Brush { radius: 200.0, shape: crate::BrushShape::Square, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() };
        let mut h = History::default();
        h.begin("forest", 0);
        {
            let mut e = h.edit(&mut w, 0);
            crate::ops::set_height(&mut e, &flat, 0.0, 0.0, 80.0, crate::ops::Surface::Top);
            crate::ops::paint_objects(&mut e, &flat, 0.0, 0.0, n, 1);
        }
        h.commit();
        let tiles: Vec<TilePos> = (-2..2).flat_map(|x| (-1..1).map(move |z| TilePos::new(x, z))).collect();
        let placements = |w: &crate::world::World| {
            let dim = &w.dims[0];
            let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
            let mut all = Vec::new();
            for &pos in &tiles {
                let stack = TileStack::new(dim, pos, true).unwrap();
                let here = tile_placements(&w.registry, &mat, 1, false, &stack, pos);
                assert_eq!(here, tile_placements(&w.registry, &mat, 1, false, &stack, pos), "the same every time");
                all.extend(here);
            }
            all
        };
        // The share of the inner land under a crown, and the widest
        // clearing (the farthest a column is from every crown).
        let measure = |w: &crate::world::World, all: &[Placement]| -> (f64, f64) {
            let radius = |p: &Placement| w.registry.objects[p.object as usize].canopy_radius() as f64;
            let (mut covered, mut total, mut widest) = (0usize, 0usize, 0f64);
            for z in -110..110 {
                for x in (-190..190).step_by(2) {
                    total += 1;
                    let clear = all.iter().map(|p| (((p.x - x) as f64).powi(2) + ((p.z - z) as f64).powi(2)).sqrt() - radius(p)).fold(f64::MAX, f64::min);
                    if clear <= 0.0 {
                        covered += 1;
                    }
                    widest = widest.max(clear);
                }
            }
            (covered as f64 / total as f64, widest)
        };
        let all = placements(&w);
        let r = |p: &Placement| w.registry.objects[p.object as usize].canopy_radius();
        for (k, a) in all.iter().enumerate() {
            for b in &all[k + 1..] {
                let d = (((a.x - b.x).pow(2) + (a.z - b.z).pow(2)) as f32).sqrt();
                assert!(d >= (r(a) + r(b)).max(3.0) - 1e-3, "{a:?} and {b:?} {d} apart, crowns {} and {}", r(a), r(b));
            }
        }
        let narrow = all.iter().filter(|p| p.object < 2).count();
        assert!(narrow > 200 && all.len() - narrow > 50, "both kinds: {narrow} narrow, {} wide", all.len() - narrow);
        let (cover, clearing) = measure(&w, &all);
        // The grid of 14 the wide crowns need, as before.
        w.registry.object_layers[0].spacing_mode = Spacing::Grid;
        let grid = placements(&w);
        let (grid_cover, grid_clearing) = measure(&w, &grid);
        println!("by canopies: {} trees, {:.0}% of the land under a crown, widest clearing {clearing:.1}; on a grid of 14: {} trees, {:.0}%, {grid_clearing:.1}", all.len(), cover * 100.0, grid.len(), grid_cover * 100.0);
        assert!(cover > 0.58, "crowns touch: {:.0}% covered", cover * 100.0);
        assert!(clearing < 5.0, "no big gaps (narrower than a wide crown): widest clearing {clearing:.1}");
        assert!(cover > grid_cover + 0.2 && all.len() > 2 * grid.len());
        // Large trees at a quarter of the density: fewer of them.
        w.registry.object_layers[0].spacing_mode = Spacing::Canopy;
        w.registry.object_layers[0].size_density = vec![1.0, 1.0, 1.0, 0.25];
        let thinned = placements(&w);
        let wide = |all: &[Placement]| all.iter().filter(|p| p.object >= 2).count();
        assert!(wide(&thinned) * 2 < wide(&all), "{} large trees, {} before", wide(&thinned), wide(&all));
    }

    /// A sub-canopy layer is painted beside the base one and stands its
    /// objects with them; the sizes painted choose each spot's object; a
    /// layer turned sub-canopy takes what was painted of it along, and an
    /// undo puts it back.
    #[test]
    fn sub_canopy_layers_and_sizes() {
        let mut w = generate(&GenParams { tiles_x: 2, tiles_z: 2, seed: 5, ..GenParams::default() });
        for (name, h) in [("small", 3u16), ("big", 12), ("bush", 2)] {
            w.registry.objects.push(pole(name, h));
        }
        let mut oaks = ObjectLayerDef::new("Oaks", &[0, 1], &w.registry);
        (oaks.soil_only, oaks.density, oaks.spacing) = (false, 1.0, 6);
        let oaks = w.registry.add_object_layer(oaks);
        let mut hazel = ObjectLayerDef::new("Hazel", &[2], &w.registry);
        (hazel.soil_only, hazel.density, hazel.spacing, hazel.understory) = (false, 1.0, 5, true);
        let hazel = w.registry.add_object_layer(hazel);
        let flat = crate::Brush { radius: 60.0, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() };
        let mut h = History::default();
        h.begin("forest", 0);
        {
            let mut e = h.edit(&mut w, 0);
            crate::ops::set_height(&mut e, &flat, 0.0, 0.0, 80.0, crate::ops::Surface::Top);
            crate::ops::paint_objects(&mut e, &flat, 0.0, 0.0, oaks, 1);
            crate::ops::paint_understory(&mut e, &flat, 0.0, 0.0, hazel, 2);
            // Small trees west, large ones east.
            let half = crate::Brush { radius: 30.0, shape: crate::BrushShape::Square, profile: crate::Profile::Constant, strength: 1.0, ..crate::Brush::default() };
            crate::ops::paint_tree_size(&mut e, &half, -30.0, 0.0, crate::tile::SIZE_BUSHES + 1, 3);
            crate::ops::paint_tree_size(&mut e, &half, 30.0, 0.0, crate::tile::SIZE_LARGE, 4);
        }
        h.commit();
        let placements = |w: &crate::world::World| {
            let dim = &w.dims[0];
            let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
            let mut all = Vec::new();
            for pos in [TilePos::new(-1, -1), TilePos::new(0, -1), TilePos::new(-1, 0), TilePos::new(0, 0)] {
                let stack = TileStack::from_above(dim, pos, true).unwrap();
                all.extend(tile_placements(&w.registry, &mat, 1, false, &stack, pos));
            }
            all
        };
        let all = placements(&w);
        let (base, under): (Vec<&Placement>, Vec<&Placement>) = all.iter().partition(|p| !p.under);
        assert!(base.len() > 100 && under.len() > 100, "{} oaks, {} hazels", base.len(), under.len());
        assert!(base.iter().all(|p| p.layer == oaks) && under.iter().all(|p| p.layer == hazel && p.object == 2));
        // West of x = 0 the small oaks, east the big ones.
        for p in &base {
            if p.x < -2 && p.x > -58 && p.z.abs() < 28 {
                assert_eq!(p.object, 0, "small at {}", p.x);
            }
            if p.x > 2 && p.x < 58 && p.z.abs() < 28 {
                assert_eq!(p.object, 1, "big at {}", p.x);
            }
        }
        // Painting a base layer over another takes its place; the
        // sub-canopy stays.
        let tile = |w: &crate::world::World| w.dims[0].ground().view(TilePos::new(0, 0)).unwrap().understory.get(local_index(10, 10));
        assert_eq!(tile(&w), hazel);

        // Hazel turned base: its columns move to the base plane, where they
        // take the oaks' place; undone, they come back.
        h.begin("role", 0);
        let moved = crate::ops::move_object_layer(&mut h.edit(&mut w, 0), hazel, false);
        h.commit();
        assert!(moved > 10_000, "{moved}");
        let view = w.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!((view.objects.get(local_index(10, 10)), view.understory.get(local_index(10, 10))), (hazel, 0));
        drop(view);
        h.undo(&mut w);
        let view = w.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
        assert_eq!((view.objects.get(local_index(10, 10)), view.understory.get(local_index(10, 10))), (oaks, hazel));
    }
}
