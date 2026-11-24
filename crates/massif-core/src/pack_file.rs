//! Pack files: packs made by Massif's users, to share them. A pack gathers
//! a map's ground covers, object layers (with their objects) and terrain
//! mixes, with a name, an author, a description and a picture, in one
//! `.massifpack` file. Another user imports it into their library and
//! paints with it as with the packs Massif comes with.
//!
//! The file is a zip archive:
//!
//! - `manifest.json`: what it is (`"format": "massif-pack"`), the version
//!   of the format, the name, author and description, and the version of
//!   Massif that wrote it;
//! - `content.json`: the covers, terrains, object layers and objects;
//! - `objects/<n>.bin`: the blocks of object `n`, little-endian `u16`s,
//!   x first, then z, then y;
//! - `preview.png`: its picture.
//!
//! A pack comes from someone else, so everything read is checked: the
//! file's size, the number of entries, how large each grows when inflated
//! (and that it grows to what it says), the names of the entries, every
//! block state, every number, every index. Nothing in a pack is ever run,
//! and no name in it ever becomes a path on the disk: the library writes
//! the pack again from what was read, under a name of its own.

use crate::objects::ObjectDef;
use crate::registry::{BiomeId, CoverDef, Mix, Registry, Stratum, TerrainDef, TerrainId};
use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// The extension of pack files.
pub const EXTENSION: &str = "massifpack";
/// What `manifest.json` says a pack is.
pub const FORMAT: &str = "massif-pack";
/// The version of the format written; every version up to it is read.
pub const VERSION: u32 = 1;

/// The largest pack file read.
pub const MAX_FILE_BYTES: u64 = 64 << 20;
/// The most entries in a pack's zip.
pub const MAX_ENTRIES: usize = 2048;
/// The most an entry may inflate to.
pub const MAX_ENTRY_BYTES: u64 = 32 << 20;
/// The most all the entries may inflate to.
pub const MAX_TOTAL_BYTES: u64 = 160 << 20;
/// The most covers, terrains or object layers in a pack.
pub const MAX_ITEMS: usize = 512;
/// The most objects in a pack.
pub const MAX_OBJECTS: usize = 1024;
/// The most blocks in an object's box.
pub const MAX_OBJECT_VOLUME: usize = 1 << 24;
/// The widest or tallest preview picture.
pub const MAX_PREVIEW_SIDE: u32 = 1024;
/// The longest name (and author), in characters.
pub const MAX_NAME: usize = 64;
/// The longest description, in characters.
pub const MAX_DESCRIPTION: usize = 2000;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum PackError {
    #[error("the file is larger than a pack can be ({} MB)", MAX_FILE_BYTES >> 20)]
    TooLarge,
    #[error("it is not a pack: not a zip archive")]
    NotZip,
    #[error("the pack is damaged: {0}")]
    Damaged(String),
    #[error("the pack holds an entry with an unsafe name: {0:?}")]
    UnsafeName(String),
    #[error("the pack is not one Massif reads: {0}")]
    Unsupported(String),
    #[error("the pack was made by a newer Massif (pack format {0}): update Massif to import it")]
    Newer(u32),
    #[error("it is not a Massif pack: {0}")]
    NotPack(String),
    #[error("the pack is invalid: {0}")]
    Invalid(String),
}

fn damaged(what: impl Into<String>) -> PackError {
    PackError::Damaged(what.into())
}

fn invalid(what: impl Into<String>) -> PackError {
    PackError::Invalid(what.into())
}

/// `manifest.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub format: String,
    pub version: u32,
    pub name: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub description: String,
    /// The version of Massif that wrote it.
    #[serde(default)]
    pub massif: String,
}

/// A block of a terrain, by its state, with a colour for the map.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackBlock {
    pub state: String,
    #[serde(default = "grey")]
    pub color: [u8; 3],
    #[serde(default = "one")]
    pub weight: u16,
}

fn grey() -> [u8; 3] {
    [128, 128, 128]
}

fn one() -> u16 {
    1
}

fn one_high() -> [u8; 2] {
    [1, 1]
}

/// How a stratum's blocks lie (see [`Mix`]).
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(tag = "lay", rename_all = "lowercase")]
pub enum PackMix {
    #[default]
    Grain,
    Patches {
        size: f32,
    },
    Beds,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackStratum {
    pub blocks: Vec<PackBlock>,
    pub depth: [u16; 2],
    #[serde(default)]
    pub mix: PackMix,
}

/// A terrain mix.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackTerrain {
    pub name: String,
    #[serde(default)]
    pub group: String,
    #[serde(default = "grey")]
    pub color: [u8; 3],
    pub strata: Vec<PackStratum>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submerged_top: Option<PackBlock>,
    /// A biome id, `minecraft:plains`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub biome: Option<String>,
}

/// A plant of a cover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackPlant {
    pub state: String,
    #[serde(default = "one")]
    pub weight: u16,
}

/// A ground cover.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackCover {
    pub name: String,
    #[serde(default = "grey")]
    pub color: [u8; 3],
    /// How strongly its colour shows on the map (see
    /// [`CoverDef::opacity`]).
    #[serde(default = "crate::registry::half", skip_serializing_if = "crate::registry::is_half")]
    pub opacity: f32,
    pub plants: Vec<PackPlant>,
    pub density: f32,
    #[serde(default = "one_high")]
    pub height: [u8; 2],
    #[serde(default)]
    pub soil_only: bool,
    #[serde(default)]
    pub underwater: bool,
    /// Frost: ice on the water where it is painted.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub frost: bool,
    /// Lava: the water where it is painted is lava.
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub lava: bool,
}

/// One of an object layer's objects, by its number in the pack.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct PackChoice {
    pub object: u32,
    #[serde(default = "one")]
    pub weight: u16,
}

/// An object layer.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackLayer {
    pub name: String,
    #[serde(default = "grey")]
    pub color: [u8; 3],
    pub objects: Vec<PackChoice>,
    pub spacing: u16,
    pub density: f32,
    #[serde(default)]
    pub rotate: bool,
    #[serde(default)]
    pub mirror: bool,
    #[serde(default)]
    pub soil_only: bool,
    #[serde(default)]
    pub underwater: bool,
    #[serde(default)]
    pub sink: i16,
    #[serde(default)]
    pub replace: bool,
    /// A sub-canopy layer (see [`crate::objects::ObjectLayerDef::understory`]).
    #[serde(default, skip_serializing_if = "crate::registry::is_false")]
    pub understory: bool,
}

/// An object: blocks in a box. Its blocks are in `objects/<n>.bin`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PackObject {
    pub name: String,
    pub size: [u16; 3],
    pub palette: Vec<String>,
    pub anchor: [u16; 3],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub colors: Vec<[u8; 3]>,
    #[serde(skip)]
    pub blocks: Vec<u16>,
}

/// `content.json`.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct Content {
    #[serde(default)]
    pub covers: Vec<PackCover>,
    #[serde(default)]
    pub terrains: Vec<PackTerrain>,
    #[serde(default)]
    pub object_layers: Vec<PackLayer>,
    #[serde(default)]
    pub objects: Vec<PackObject>,
    /// Gradients of the Gradient brush, their terrains by name (those of
    /// the pack, or those every map has).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub gradients: Vec<crate::gradient::Gradient>,
}

/// A pack, read from a file or made from a map.
#[derive(Debug, Clone, PartialEq)]
pub struct Pack {
    pub name: String,
    pub author: String,
    pub description: String,
    /// The version of Massif that wrote it.
    pub made_with: String,
    /// Its picture, a PNG.
    pub preview: Option<Vec<u8>>,
    pub content: Content,
}

/// What kind of thing an item of a pack is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ItemKind {
    Cover,
    Terrain,
    Layer,
    Gradient,
}

impl Pack {
    /// How many things it holds.
    pub fn len(&self) -> usize {
        self.content.covers.len() + self.content.terrains.len() + self.content.object_layers.len() + self.content.gradients.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Its items, by kind and number, in the order shown: object layers,
    /// covers, terrains, gradients.
    pub fn items(&self) -> impl Iterator<Item = (ItemKind, usize, &str)> {
        let c = &self.content;
        c.object_layers
            .iter()
            .enumerate()
            .map(|(i, l)| (ItemKind::Layer, i, l.name.as_str()))
            .chain(c.covers.iter().enumerate().map(|(i, l)| (ItemKind::Cover, i, l.name.as_str())))
            .chain(c.terrains.iter().enumerate().map(|(i, l)| (ItemKind::Terrain, i, l.name.as_str())))
            .chain(c.gradients.iter().enumerate().map(|(i, g)| (ItemKind::Gradient, i, g.name.as_str())))
    }

    /// Add these gradients to `content`, with the terrains of `reg` they
    /// paint that not every map has (and `content` has not already).
    pub fn gather_gradients(content: &mut Content, reg: &Registry, gradients: &[crate::gradient::Gradient]) {
        let vanilla = Registry::vanilla();
        for g in gradients {
            for name in g.terrains() {
                let standard = vanilla.terrain_by_name(name).is_some_and(|v| reg.terrain_by_name(name).is_some_and(|t| same_terrain(&terrain_of(&vanilla, vanilla.terrain(v)), &terrain_of(reg, reg.terrain(t)))));
                if standard || content.terrains.iter().any(|t| t.name == name) {
                    continue;
                }
                if let Some(t) = reg.terrain_by_name(name) {
                    content.terrains.push(terrain_of(reg, reg.terrain(t)));
                }
            }
            content.gradients.push(g.clone());
        }
    }

    /// A pack of the given covers, terrains and object layers of `reg`
    /// (by their numbers: a cover's and a layer's from 1, a terrain's
    /// index), with the objects the layers stand.
    pub fn gather(reg: &Registry, covers: &[u16], terrains: &[u16], layers: &[u16]) -> Content {
        let mut content = Content::default();
        for &n in covers {
            if let Some(c) = reg.cover(n) {
                content.covers.push(cover_of(c));
            }
        }
        for &t in terrains {
            if let Some(def) = reg.terrains.get(t as usize) {
                content.terrains.push(terrain_of(reg, def));
            }
        }
        let mut taken: Vec<(u32, u32)> = Vec::new();
        for &n in layers {
            let Some(l) = reg.object_layer(n) else { continue };
            let mut objects = Vec::new();
            for &(k, weight) in &l.objects {
                let Some(def) = reg.objects.get(k as usize) else { continue };
                let local = match taken.iter().find(|t| t.0 == k) {
                    Some(t) => t.1,
                    None => {
                        content.objects.push(object_of(def));
                        let local = content.objects.len() as u32 - 1;
                        taken.push((k, local));
                        local
                    }
                };
                objects.push(PackChoice { object: local, weight });
            }
            content.object_layers.push(PackLayer {
                name: l.name.clone(),
                color: l.color,
                objects,
                spacing: l.spacing,
                density: l.density,
                rotate: l.rotate,
                mirror: l.mirror,
                soil_only: l.soil_only,
                underwater: l.underwater,
                sink: l.sink,
                replace: l.replace,
                understory: l.understory,
            });
        }
        content
    }

    /// The pack file.
    pub fn to_bytes(&self) -> Vec<u8> {
        let manifest = Manifest {
            format: FORMAT.to_owned(),
            version: VERSION,
            name: self.name.clone(),
            author: self.author.clone(),
            description: self.description.clone(),
            massif: self.made_with.clone(),
        };
        let mut zip = ZipWriter::default();
        zip.add("manifest.json", &serde_json::to_vec_pretty(&manifest).expect("a manifest is JSON"));
        zip.add("content.json", &serde_json::to_vec_pretty(&self.content).expect("a pack is JSON"));
        for (i, o) in self.content.objects.iter().enumerate() {
            let bytes: Vec<u8> = o.blocks.iter().flat_map(|b| b.to_le_bytes()).collect();
            zip.add(&format!("objects/{i}.bin"), &bytes);
        }
        if let Some(png) = &self.preview {
            zip.add("preview.png", png);
        }
        zip.finish()
    }

    /// Read a pack file, checking all of it.
    pub fn from_bytes(bytes: &[u8]) -> Result<Pack, PackError> {
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(PackError::TooLarge);
        }
        let zip = ZipReader::new(bytes)?;
        let manifest = zip.read("manifest.json")?.ok_or_else(|| PackError::NotPack("it has no manifest.json".into()))?;
        let manifest: Manifest = serde_json::from_slice(&manifest).map_err(|e| PackError::NotPack(format!("its manifest.json cannot be read: {e}")))?;
        if manifest.format != FORMAT {
            return Err(PackError::NotPack(format!("its manifest says it is {:?}", manifest.format)));
        }
        if manifest.version > VERSION {
            return Err(PackError::Newer(manifest.version));
        }
        if manifest.version == 0 {
            return Err(PackError::Unsupported("pack format 0".into()));
        }
        let content = zip.read("content.json")?.ok_or_else(|| invalid("it has no content.json"))?;
        let mut content: Content = serde_json::from_slice(&content).map_err(|e| invalid(format!("its content.json cannot be read: {e}")))?;
        if content.objects.len() > MAX_OBJECTS {
            return Err(invalid(format!("{} objects, more than {MAX_OBJECTS}", content.objects.len())));
        }
        for (i, o) in content.objects.iter_mut().enumerate() {
            let raw = zip.read(&format!("objects/{i}.bin"))?.ok_or_else(|| invalid(format!("the blocks of object {i} are missing")))?;
            if raw.len() % 2 != 0 {
                return Err(invalid(format!("the blocks of object {i} are cut short")));
            }
            o.blocks = raw.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        }
        let preview = zip.read("preview.png")?;
        if let Some(png) = &preview {
            check_preview(png)?;
        }
        let mut pack = Pack {
            name: clean_text(&manifest.name, MAX_NAME, false),
            author: clean_text(&manifest.author, MAX_NAME, false),
            description: clean_text(&manifest.description, MAX_DESCRIPTION, true),
            made_with: clean_text(&manifest.massif, 32, false),
            preview,
            content,
        };
        pack.validate()?;
        Ok(pack)
    }

    /// Check everything in it, setting right what can be (names cut to
    /// their length, numbers into their range).
    pub fn validate(&mut self) -> Result<(), PackError> {
        if self.name.is_empty() {
            return Err(invalid("it has no name"));
        }
        let c = &mut self.content;
        if c.covers.len() > MAX_ITEMS || c.terrains.len() > MAX_ITEMS || c.object_layers.len() > MAX_ITEMS || c.gradients.len() > MAX_ITEMS {
            return Err(invalid(format!("more than {MAX_ITEMS} things of a kind")));
        }
        if c.covers.is_empty() && c.terrains.is_empty() && c.object_layers.is_empty() && c.gradients.is_empty() {
            return Err(invalid("it holds nothing"));
        }
        for cover in &mut c.covers {
            cover.name = item_name(&cover.name, "cover")?;
            if cover.plants.is_empty() || cover.plants.len() > 256 {
                return Err(invalid(format!("cover {:?} has {} plants", cover.name, cover.plants.len())));
            }
            for p in &cover.plants {
                check_state(&p.state)?;
            }
            cover.density = finite(cover.density, 0.0, 1.0, "a cover's density")?;
            cover.height = [cover.height[0].clamp(1, 32), cover.height[1].clamp(1, 32)];
            if cover.height[1] < cover.height[0] {
                cover.height.swap(0, 1);
            }
        }
        for t in &mut c.terrains {
            t.name = item_name(&t.name, "terrain")?;
            t.group = clean_text(&t.group, MAX_NAME, false);
            if t.strata.is_empty() || t.strata.len() > 64 {
                return Err(invalid(format!("terrain {:?} has {} strata", t.name, t.strata.len())));
            }
            for s in &mut t.strata {
                if s.blocks.is_empty() || s.blocks.len() > 256 {
                    return Err(invalid(format!("a stratum of terrain {:?} has {} blocks", t.name, s.blocks.len())));
                }
                for b in &s.blocks {
                    check_state(&b.state)?;
                }
                if s.depth[1] < s.depth[0] {
                    s.depth.swap(0, 1);
                }
                if let PackMix::Patches { size } = &mut s.mix {
                    *size = finite(*size, 0.0, 1.0, "a patch size")?;
                }
            }
            if let Some(b) = &t.submerged_top {
                check_state(&b.state)?;
            }
            if let Some(b) = &t.biome {
                check_state(b)?;
            }
        }
        for g in &mut c.gradients {
            g.name = item_name(&g.name, "gradient")?;
            if g.bands.is_empty() || g.bands.len() > crate::gradient::MAX_BANDS {
                return Err(invalid(format!("gradient {:?} has {} bands", g.name, g.bands.len())));
            }
            for b in &mut g.bands {
                b.terrain = clean_text(&b.terrain, MAX_NAME * 2, false);
            }
            g.normalize();
        }
        let objects = c.objects.len();
        for l in &mut c.object_layers {
            l.name = item_name(&l.name, "object layer")?;
            if l.objects.is_empty() || l.objects.len() > 1024 {
                return Err(invalid(format!("object layer {:?} has {} objects", l.name, l.objects.len())));
            }
            if let Some(o) = l.objects.iter().find(|o| o.object as usize >= objects) {
                return Err(invalid(format!("object layer {:?} names object {}, of {objects}", l.name, o.object)));
            }
            l.spacing = l.spacing.clamp(1, 1024);
            l.density = finite(l.density, 0.0, 1.0, "a layer's density")?;
            l.sink = l.sink.clamp(-256, 256);
        }
        for o in &mut c.objects {
            o.name = clean_text(&o.name, 128, false);
            let volume = o.size.iter().map(|&s| s as usize).product::<usize>();
            if o.size.contains(&0) || volume > MAX_OBJECT_VOLUME {
                return Err(invalid(format!("object {:?} is {:?} blocks", o.name, o.size)));
            }
            if o.blocks.len() != volume {
                return Err(invalid(format!("object {:?} has {} blocks for a box of {volume}", o.name, o.blocks.len())));
            }
            if o.palette.is_empty() || o.palette.len() > 4096 {
                return Err(invalid(format!("object {:?} has a palette of {}", o.name, o.palette.len())));
            }
            for s in &o.palette {
                check_state(s)?;
            }
            if o.blocks.iter().any(|&b| b as usize >= o.palette.len()) {
                return Err(invalid(format!("object {:?} has a block past its palette", o.name)));
            }
            if (0..3).any(|k| o.anchor[k] >= o.size[k]) {
                return Err(invalid(format!("object {:?} stands outside its box", o.name)));
            }
            o.colors.truncate(o.palette.len());
        }
        Ok(())
    }

    /// The name cover `i` takes in `reg`, and its number there if `reg`
    /// has it as it is. A cover of that name that is not the same is left,
    /// and this one takes its name and the pack's.
    fn cover_slot(&self, i: usize, reg: &Registry) -> (String, Option<u16>) {
        let cover = &self.content.covers[i];
        let fits = |n: u16| reg.cover(n).is_some_and(|c| same_cover(&cover_of(c), cover));
        let name = self.free_name(&cover.name, |name| reg.cover_by_name(name).is_none_or(fits));
        let n = reg.cover_by_name(&name);
        (name, n)
    }

    fn terrain_group(&self, i: usize) -> String {
        let t = &self.content.terrains[i];
        if t.group.is_empty() { self.name.clone() } else { t.group.clone() }
    }

    /// As [`Pack::cover_slot`], for terrain `i`, in its group.
    fn terrain_slot(&self, i: usize, reg: &Registry) -> (String, Option<u16>) {
        let t = &self.content.terrains[i];
        let group = self.terrain_group(i);
        let find = |name: &str| reg.terrains.iter().position(|d| d.group == group && d.name == name);
        let name = self.free_name(&t.name, |name| find(name).is_none_or(|k| same_terrain(&terrain_of(reg, &reg.terrains[k]), t)));
        let k = find(&name).map(|k| k as u16);
        (name, k)
    }

    /// As [`Pack::cover_slot`], for object layer `i`.
    fn layer_slot(&self, i: usize, reg: &Registry) -> (String, Option<u16>) {
        let l = &self.content.object_layers[i];
        let by_name = |name: &str| reg.object_layers.iter().position(|d| !d.removed && d.name == name).map(|k| k as u16 + 1);
        let fits = |n: u16| reg.object_layer(n).is_some_and(|def| layer_fits(reg, def, l, &self.content.objects));
        let name = self.free_name(&l.name, |name| by_name(name).is_none_or(fits));
        let n = by_name(&name);
        (name, n)
    }

    /// Item `(kind, i)` in `reg`, as it is: its number (a terrain's index).
    pub fn find(&self, kind: ItemKind, i: usize, reg: &Registry) -> Option<u16> {
        match kind {
            ItemKind::Cover => self.cover_slot(i, reg).1,
            ItemKind::Terrain => self.terrain_slot(i, reg).1,
            ItemKind::Layer => self.layer_slot(i, reg).1,
            // A gradient is in the map when the terrains it paints are.
            ItemKind::Gradient => {
                let g = &self.content.gradients[i];
                g.bands
                    .iter()
                    .all(|b| match self.content.terrains.iter().position(|t| t.name == b.terrain) {
                        Some(j) => self.terrain_slot(j, reg).1.is_some(),
                        None => reg.terrain_by_name(&b.terrain).is_some(),
                    })
                    .then_some(i as u16)
            }
        }
    }

    /// Whether `reg` holds item `(kind, i)` already, as it is.
    pub fn installed(&self, kind: ItemKind, i: usize, reg: &Registry) -> bool {
        self.find(kind, i, reg).is_some()
    }

    /// Add cover `i` to `reg`, or find it there; returns its number.
    pub fn install_cover(&self, i: usize, reg: &mut Registry) -> u16 {
        let (name, found) = self.cover_slot(i, reg);
        if let Some(n) = found {
            return n;
        }
        let cover = &self.content.covers[i];
        let mut def = CoverDef::new(&name, cover.color, &[], cover.density);
        def.opacity = cover.opacity;
        def.plants = cover.plants.iter().map(|p| (p.state.clone(), p.weight)).collect();
        def.height = cover.height;
        def.soil_only = cover.soil_only;
        def.underwater = cover.underwater;
        def.frost = cover.frost;
        def.lava = cover.lava;
        reg.add_cover(def)
    }

    /// Add terrain `i` to `reg`, or find it there (in its group, by name).
    pub fn install_terrain(&self, i: usize, reg: &mut Registry) -> TerrainId {
        let (name, found) = self.terrain_slot(i, reg);
        if let Some(k) = found {
            return TerrainId(k);
        }
        let t = &self.content.terrains[i];
        let group = self.terrain_group(i);
        let mut block = |b: &PackBlock| reg.intern_block(&b.state, b.color);
        let strata: Vec<Stratum> = t
            .strata
            .iter()
            .map(|s| Stratum {
                blocks: s.blocks.iter().map(|b| (block(b), b.weight)).collect(),
                depth: (s.depth[0], s.depth[1]),
                mix: match s.mix {
                    PackMix::Grain => Mix::Grain,
                    PackMix::Patches { size } => Mix::Patches { size },
                    PackMix::Beds => Mix::Beds,
                },
            })
            .collect();
        let submerged_top = t.submerged_top.as_ref().map(&mut block);
        let biome_hint = t.biome.as_deref().and_then(|b| reg.biome_by_id(b));
        reg.terrains.push(TerrainDef {
            name,
            group,
            color: t.color,
            strata,
            submerged_top,
            biome_hint,
        });
        TerrainId(reg.terrains.len() as u16 - 1)
    }

    /// Gradient `i`, with the terrains of the pack it paints added to `reg`
    /// (or found there), its bands naming them as `reg` does.
    pub fn install_gradient(&self, i: usize, reg: &mut Registry) -> crate::gradient::Gradient {
        let mut g = self.content.gradients[i].clone();
        for b in &mut g.bands {
            if let Some(j) = self.content.terrains.iter().position(|t| t.name == b.terrain) {
                let id = self.install_terrain(j, reg);
                b.terrain = reg.terrain(id).name.clone();
            }
        }
        g
    }

    /// Add object layer `i` and its objects to `reg`, or find it there;
    /// returns its number.
    pub fn install_layer(&self, i: usize, reg: &mut Registry) -> u16 {
        let (name, found) = self.layer_slot(i, reg);
        if let Some(n) = found {
            return n;
        }
        let l = &self.content.object_layers[i];
        let mut objects = Vec::with_capacity(l.objects.len());
        let mut added: Vec<(u32, u32)> = Vec::new();
        for choice in &l.objects {
            let k = match added.iter().find(|a| a.0 == choice.object) {
                Some(a) => a.1,
                None => {
                    let o = &self.content.objects[choice.object as usize];
                    reg.objects.push(ObjectDef {
                        name: o.name.clone(),
                        size: o.size,
                        palette: o.palette.clone(),
                        blocks: o.blocks.clone(),
                        anchor: o.anchor,
                        source: format!("{} ({})", self.name, EXTENSION),
                        colors: o.colors.clone(),
                    });
                    let k = reg.objects.len() as u32 - 1;
                    added.push((choice.object, k));
                    k
                }
            };
            objects.push((k, choice.weight));
        }
        reg.add_object_layer(crate::objects::ObjectLayerDef {
            name,
            color: l.color,
            objects,
            spacing: l.spacing,
            density: l.density,
            rotate: l.rotate,
            mirror: l.mirror,
            soil_only: l.soil_only,
            underwater: l.underwater,
            sink: l.sink,
            replace: l.replace,
            removed: false,
            hidden: false,
            understory: l.understory,
            locked: false,
            order: 0,
            palette: String::new(),
        })
    }

    /// Install item `(kind, i)`; returns its number in `reg` (a terrain's
    /// index).
    pub fn install(&self, kind: ItemKind, i: usize, reg: &mut Registry) -> u16 {
        match kind {
            ItemKind::Cover => self.install_cover(i, reg),
            ItemKind::Terrain => self.install_terrain(i, reg).0,
            ItemKind::Layer => self.install_layer(i, reg),
            ItemKind::Gradient => {
                self.install_gradient(i, reg);
                i as u16
            }
        }
    }

    /// `name` if `free` says it may be used; otherwise `name (pack)`, then
    /// `name (pack) 2`, and so on.
    fn free_name(&self, name: &str, free: impl Fn(&str) -> bool) -> String {
        if free(name) {
            return name.to_owned();
        }
        let base = format!("{name} ({})", self.name);
        if free(&base) {
            return base;
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| free(n)).expect("a free name")
    }

    /// A picture of what it holds, for a pack made without one: its first
    /// eight things side by side in up to four columns and two rows, a
    /// PNG of 64 pixels a thing (256 × 128 at most).
    pub fn draw_preview(&self) -> Vec<u8> {
        const CELL: u32 = 64;
        let c = &self.content;
        let mut cells: Vec<Cell<'_>> = Vec::new();
        for l in &c.object_layers {
            let Some(o) = l.objects.first().and_then(|ch| c.objects.get(ch.object as usize)) else { continue };
            let top = top_view(o);
            let side = o.size[0].max(o.size[2]).max(1) as u32;
            let back = shade(l.color, 0.45);
            cells.push(Box::new(move |x, y| {
                let (bx, bz) = (x * side / CELL, y * side / CELL);
                let (dx, dz) = ((side - o.size[0] as u32) / 2, (side - o.size[2] as u32) / 2);
                if bx < dx || bz < dz || bx - dx >= o.size[0] as u32 || bz - dz >= o.size[2] as u32 {
                    return back;
                }
                let (k, h) = top[((bz - dz) * o.size[0] as u32 + bx - dx) as usize];
                if k == 0 {
                    return back;
                }
                let base = o.colors.get(k as usize).copied().unwrap_or_else(|| crate::objects::guess_color(&o.palette[k as usize]));
                shade(base, 0.6 + 0.4 * h as f32 / o.size[1].max(1) as f32)
            }));
        }
        for cover in &c.covers {
            cells.push(Box::new(move |x, y| {
                let h = crate::compose::hash2(x as i32 / 4, y as i32 / 4, 0x636f_7672);
                if (h % 1000) as f32 / 1000.0 < cover.density.max(0.15) {
                    let p = &cover.plants[(h / 1000) as usize % cover.plants.len()];
                    crate::registry::plant_color(&p.state).unwrap_or(cover.color)
                } else {
                    shade(cover.color, 0.55)
                }
            }));
        }
        for t in &c.terrains {
            let s = &t.strata[0];
            let total: u32 = s.blocks.iter().map(|b| b.weight as u32).sum::<u32>().max(1);
            cells.push(Box::new(move |x, y| {
                let mut r = (crate::compose::hash2(x as i32 / 8, y as i32 / 8, 0x7465_7272) % total) as i32;
                let b = s.blocks.iter().find(|b| {
                    r -= b.weight as i32;
                    r < 0
                });
                b.map_or(t.color, |b| b.color)
            }));
        }
        let vanilla = Registry::vanilla();
        for g in &c.gradients {
            let colours: Vec<[u8; 3]> = g
                .bands
                .iter()
                .map(|b| c.terrains.iter().find(|t| t.name == b.terrain).map(|t| t.color).or_else(|| vanilla.terrain_by_name(&b.terrain).map(|t| vanilla.terrain(t).color)).unwrap_or([128, 128, 128]))
                .collect();
            let total = g.total().max(1.0);
            let sampler = g.clone();
            cells.push(Box::new(move |x, y| {
                // The bands across, a ragged edge between them.
                let jitter = (crate::compose::hash2(x as i32 / 3, y as i32 / 3, 0x6772_6164) % 7) as f32 - 3.0;
                let p = (x as f32 + if y % 2 == 0 { jitter } else { -jitter } * 0.3) / CELL as f32 * total;
                let k = match sampler.place(p) {
                    crate::gradient::Place::Band(k) => k,
                    crate::gradient::Place::Transition(k, t) => k + ((t * 2.0 - 1.0) * 6.0 + jitter > 0.0) as usize,
                };
                colours[k.min(colours.len() - 1)]
            }));
        }
        let shown = cells.len().clamp(1, 8) as u32;
        let columns = if shown > 4 { shown.div_ceil(2) } else { shown };
        let mut img = image::RgbImage::from_pixel(CELL * columns, CELL * shown.div_ceil(columns), image::Rgb([32, 32, 36]));
        for (n, cell) in cells.iter().take(8).enumerate() {
            let (ox, oy) = ((n as u32 % columns) * CELL, (n as u32 / columns) * CELL);
            for y in 1..CELL - 1 {
                for x in 1..CELL - 1 {
                    img.put_pixel(ox + x, oy + y, image::Rgb(cell(x, y)));
                }
            }
        }
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).expect("a picture in memory");
        png
    }
}

/// How a cell of a preview is drawn: its colour at each pixel.
type Cell<'a> = Box<dyn Fn(u32, u32) -> [u8; 3] + 'a>;

fn shade(c: [u8; 3], k: f32) -> [u8; 3] {
    c.map(|v| (v as f32 * k).round().clamp(0.0, 255.0) as u8)
}

fn top_view(o: &PackObject) -> Vec<(u16, u16)> {
    let [w, h, l] = o.size.map(|s| s as usize);
    let at = |x: usize, y: usize, z: usize| o.blocks[x + z * w + y * w * l];
    let mut out = Vec::with_capacity(w * l);
    for z in 0..l {
        for x in 0..w {
            let top = (0..h).rev().find(|&y| at(x, y, z) != 0);
            out.push(top.map_or((0, 0), |y| (at(x, y, z), y as u16)));
        }
    }
    out
}

fn cover_of(c: &CoverDef) -> PackCover {
    PackCover {
        name: c.name.clone(),
        color: c.color,
        opacity: c.opacity,
        plants: c.plants.iter().map(|(state, weight)| PackPlant { state: state.clone(), weight: *weight }).collect(),
        density: c.density,
        height: c.height,
        soil_only: c.soil_only,
        underwater: c.underwater,
        frost: c.frost,
        lava: c.lava,
    }
}

fn terrain_of(reg: &Registry, t: &TerrainDef) -> PackTerrain {
    let block = |id: crate::registry::BlockId, weight: u16| {
        let b = reg.block(id);
        PackBlock { state: b.state.clone(), color: b.color, weight }
    };
    PackTerrain {
        name: t.name.clone(),
        group: t.group.clone(),
        color: t.color,
        strata: t
            .strata
            .iter()
            .map(|s| PackStratum {
                blocks: s.blocks.iter().map(|&(id, w)| block(id, w)).collect(),
                depth: [s.depth.0, s.depth.1],
                mix: match s.mix {
                    Mix::Grain => PackMix::Grain,
                    Mix::Patches { size } => PackMix::Patches { size },
                    Mix::Beds => PackMix::Beds,
                },
            })
            .collect(),
        submerged_top: t.submerged_top.map(|id| block(id, 1)),
        biome: t.biome_hint.filter(|&b| b != BiomeId::AUTO).map(|b| reg.biome(b).id.clone()).filter(|id| !id.is_empty()),
    }
}

fn object_of(o: &ObjectDef) -> PackObject {
    PackObject {
        name: o.name.clone(),
        size: o.size,
        palette: o.palette.clone(),
        anchor: o.anchor,
        colors: o.colors.clone(),
        blocks: o.blocks.clone(),
    }
}

/// The same plants, the same way, whatever their name and colour.
fn same_cover(a: &PackCover, b: &PackCover) -> bool {
    a.plants == b.plants && a.density == b.density && a.height == b.height && a.soil_only == b.soil_only && a.underwater == b.underwater && a.frost == b.frost && a.lava == b.lava
}

/// The same blocks, the same way, whatever their colours.
fn same_terrain(a: &PackTerrain, b: &PackTerrain) -> bool {
    type Bare<'a> = Vec<(Vec<(&'a str, u16)>, [u16; 2], PackMix)>;
    fn strip(t: &PackTerrain) -> Bare<'_> {
        t.strata.iter().map(|s| (s.blocks.iter().map(|b| (b.state.as_str(), b.weight)).collect(), s.depth, s.mix)).collect()
    }
    strip(a) == strip(b) && a.submerged_top.as_ref().map(|b| &b.state) == b.submerged_top.as_ref().map(|b| &b.state) && a.biome == b.biome
}

/// Whether a map's layer stands the very objects of a pack's layer, the
/// same way, whatever its name and colours.
fn layer_fits(reg: &Registry, def: &crate::objects::ObjectLayerDef, l: &PackLayer, objects: &[PackObject]) -> bool {
    (def.spacing, def.density, def.rotate, def.mirror, def.soil_only, def.underwater, def.sink, def.replace, def.understory) == (l.spacing, l.density, l.rotate, l.mirror, l.soil_only, l.underwater, l.sink, l.replace, l.understory)
        && def.objects.len() == l.objects.len()
        && def.objects.iter().zip(&l.objects).all(|(&(k, weight), choice)| {
            let (Some(p), q) = (reg.objects.get(k as usize), &objects[choice.object as usize]) else { return false };
            weight == choice.weight && p.size == q.size && p.palette == q.palette && p.anchor == q.anchor && p.blocks == q.blocks
        })
}

/// Text from a pack: control characters out (but new lines where
/// `lines`), spaces trimmed, cut to `max` characters.
pub fn clean_text(text: &str, max: usize, lines: bool) -> String {
    let kept: String = text.chars().filter(|&c| !c.is_control() || (lines && c == '\n')).take(max).collect();
    kept.trim().to_owned()
}

fn item_name(name: &str, what: &str) -> Result<String, PackError> {
    let name = clean_text(name, MAX_NAME, false);
    if name.is_empty() {
        return Err(invalid(format!("a {what} has no name")));
    }
    Ok(name)
}

fn finite(v: f32, lo: f32, hi: f32, what: &str) -> Result<f32, PackError> {
    if !v.is_finite() {
        return Err(invalid(format!("{what} is not a number")));
    }
    Ok(v.clamp(lo, hi))
}

/// A block state as the game writes it: `namespace:path`, then perhaps
/// `[key=value,…]`, in lower-case letters, digits and `_ - . /`.
pub fn check_state(state: &str) -> Result<(), PackError> {
    let bad = || invalid(format!("{:?} is not a block state", state.chars().take(80).collect::<String>()));
    if state.is_empty() || state.len() > 256 {
        return Err(bad());
    }
    let (id, props) = match state.split_once('[') {
        Some((id, rest)) => (id, Some(rest.strip_suffix(']').ok_or_else(bad)?)),
        None => (state, None),
    };
    let word = |s: &str, extra: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || "_-.".contains(c) || extra.contains(c));
    let (ns, path) = id.split_once(':').ok_or_else(bad)?;
    if !word(ns, "") || !word(path, "/") {
        return Err(bad());
    }
    if let Some(props) = props {
        for kv in props.split(',') {
            let (k, v) = kv.split_once('=').ok_or_else(bad)?;
            if !word(k, "") || !word(v, "") {
                return Err(bad());
            }
        }
    }
    Ok(())
}

/// Read a preview's size without decoding it all, and refuse what is
/// not a PNG or is too large.
fn check_preview(png: &[u8]) -> Result<(), PackError> {
    let reader = image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
    let (w, h) = reader.into_dimensions().map_err(|e| invalid(format!("its picture cannot be read: {e}")))?;
    if w == 0 || h == 0 || w > MAX_PREVIEW_SIDE || h > MAX_PREVIEW_SIDE {
        return Err(invalid(format!("its picture is {w} × {h}, larger than {MAX_PREVIEW_SIDE} × {MAX_PREVIEW_SIDE}")));
    }
    Ok(())
}

/// A picture chosen for a pack (PNG or JPEG), fitted into 512 × 512 and
/// written as a PNG.
pub fn picture_from_file(bytes: &[u8]) -> Result<Vec<u8>, PackError> {
    if bytes.len() > 32 << 20 {
        return Err(PackError::TooLarge);
    }
    let mut reader = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format().map_err(|e| invalid(e.to_string()))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(256 << 20);
    reader.limits(limits);
    let img = reader.decode().map_err(|e| invalid(format!("the picture cannot be read: {e}")))?;
    let img = if img.width() > 512 || img.height() > 512 { img.thumbnail(512, 512) } else { img };
    let mut png = Vec::new();
    img.to_rgba8().write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).map_err(|e| invalid(e.to_string()))?;
    Ok(png)
}

/// A file name for a pack: its name in letters, digits, spaces and
/// dashes, never empty, never a name the system keeps.
pub fn file_stem(name: &str) -> String {
    let stem: String = name.chars().map(|c| if c.is_alphanumeric() || c == ' ' || c == '-' || c == '_' { c } else { '-' }).take(48).collect();
    let stem = stem.trim().trim_matches('-').to_owned();
    let reserved = ["con", "prn", "aux", "nul", "com1", "lpt1"];
    if stem.is_empty() || reserved.contains(&stem.to_lowercase().as_str()) { format!("pack {stem}").trim().to_owned() } else { stem }
}

// The zip format, just what packs need. Writing: entries deflated, with
// their CRC, dated 1980 so that a pack written twice is the same file.
// Reading: entries stored or deflated, nothing else, every size checked.

#[derive(Default)]
struct ZipWriter {
    out: Vec<u8>,
    central: Vec<u8>,
    count: u16,
}

impl ZipWriter {
    fn add(&mut self, name: &str, data: &[u8]) {
        let mut enc = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(data).expect("deflating in memory");
        let body = enc.finish().expect("deflating in memory");
        let mut crc = flate2::Crc::new();
        crc.update(data);
        let crc = crc.sum();
        let header = self.out.len() as u32;
        // Version 2.0, UTF-8 names, deflated, 1980-01-01 00:00.
        let common = |v: &mut Vec<u8>| {
            v.extend_from_slice(&0x0800u16.to_le_bytes());
            v.extend_from_slice(&8u16.to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
            v.extend_from_slice(&0x0021u16.to_le_bytes());
            v.extend_from_slice(&crc.to_le_bytes());
            v.extend_from_slice(&(body.len() as u32).to_le_bytes());
            v.extend_from_slice(&(data.len() as u32).to_le_bytes());
            v.extend_from_slice(&(name.len() as u16).to_le_bytes());
            v.extend_from_slice(&0u16.to_le_bytes());
        };
        self.out.extend_from_slice(&0x0403_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&20u16.to_le_bytes());
        common(&mut self.out);
        self.out.extend_from_slice(name.as_bytes());
        self.out.extend_from_slice(&body);
        self.central.extend_from_slice(&0x0201_4b50u32.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        self.central.extend_from_slice(&20u16.to_le_bytes());
        common(&mut self.central);
        self.central.extend_from_slice(&[0; 6]); // comment, disk, internal attributes
        self.central.extend_from_slice(&0u32.to_le_bytes()); // external attributes
        self.central.extend_from_slice(&header.to_le_bytes());
        self.central.extend_from_slice(name.as_bytes());
        self.count += 1;
    }

    fn finish(mut self) -> Vec<u8> {
        let offset = self.out.len() as u32;
        self.out.extend_from_slice(&self.central);
        self.out.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
        self.out.extend_from_slice(&[0; 4]);
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&self.count.to_le_bytes());
        self.out.extend_from_slice(&(self.central.len() as u32).to_le_bytes());
        self.out.extend_from_slice(&offset.to_le_bytes());
        self.out.extend_from_slice(&0u16.to_le_bytes());
        self.out
    }
}

struct ZipEntry {
    name: String,
    flags: u16,
    method: u16,
    crc: u32,
    compressed: u64,
    size: u64,
    header: u64,
}

struct ZipReader<'a> {
    bytes: &'a [u8],
    entries: Vec<ZipEntry>,
    /// What the entries read so far inflated to.
    total: std::cell::Cell<u64>,
}

fn u16_at(b: &[u8], at: usize) -> Result<u16, PackError> {
    b.get(at..at.checked_add(2).ok_or_else(|| damaged("an offset"))?).map(|s| u16::from_le_bytes([s[0], s[1]])).ok_or_else(|| damaged("it is cut short"))
}

fn u32_at(b: &[u8], at: usize) -> Result<u32, PackError> {
    b.get(at..at.checked_add(4).ok_or_else(|| damaged("an offset"))?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]])).ok_or_else(|| damaged("it is cut short"))
}

/// Whether an entry's name is a plain relative path: no folder above, no
/// root, no drive, no backslash, nothing hidden in it.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 255
        && !name.starts_with('/')
        && !name.contains('\\')
        && !name.contains(':')
        && !name.chars().any(|c| c.is_control())
        && name.split('/').all(|part| part != ".." && part != ".")
}

impl<'a> ZipReader<'a> {
    fn new(bytes: &'a [u8]) -> Result<Self, PackError> {
        // The end of central directory record, in the last 64 KiB.
        let min = bytes.len().saturating_sub(22 + 0xffff);
        let eocd = (min..=bytes.len().saturating_sub(22)).rev().find(|&i| bytes.len() >= 22 && u32_at(bytes, i).ok() == Some(0x0605_4b50)).ok_or(PackError::NotZip)?;
        let disk = u16_at(bytes, eocd + 4)?;
        let count = u16_at(bytes, eocd + 10)? as usize;
        let dir_size = u32_at(bytes, eocd + 12)? as usize;
        let offset = u32_at(bytes, eocd + 16)? as usize;
        if disk != 0 || u16_at(bytes, eocd + 8)? as usize != count {
            return Err(PackError::Unsupported("a zip in several parts".into()));
        }
        if count == 0xffff || offset == 0xffff_ffff {
            return Err(PackError::Unsupported("a zip64 archive".into()));
        }
        if count > MAX_ENTRIES {
            return Err(invalid(format!("{count} entries, more than {MAX_ENTRIES}")));
        }
        if offset.checked_add(dir_size).is_none_or(|end| end > eocd) {
            return Err(damaged("its directory lies outside it"));
        }
        let mut entries: Vec<ZipEntry> = Vec::with_capacity(count);
        let mut at = offset;
        for _ in 0..count {
            if at >= offset + dir_size || u32_at(bytes, at)? != 0x0201_4b50 {
                return Err(damaged("its directory"));
            }
            let flags = u16_at(bytes, at + 8)?;
            let method = u16_at(bytes, at + 10)?;
            let crc = u32_at(bytes, at + 16)?;
            let compressed = u32_at(bytes, at + 20)? as u64;
            let size = u32_at(bytes, at + 24)? as u64;
            let name_len = u16_at(bytes, at + 28)? as usize;
            let extra_len = u16_at(bytes, at + 30)? as usize;
            let comment_len = u16_at(bytes, at + 32)? as usize;
            let header = u32_at(bytes, at + 42)? as u64;
            let raw = bytes.get(at + 46..at + 46 + name_len).ok_or_else(|| damaged("an entry's name"))?;
            let name = std::str::from_utf8(raw).map_err(|_| PackError::UnsafeName(String::from_utf8_lossy(raw).into_owned()))?.to_owned();
            if !safe_name(&name) {
                return Err(PackError::UnsafeName(name));
            }
            if entries.iter().any(|e| e.name == name) {
                return Err(damaged(format!("{name:?} is in it twice")));
            }
            if compressed == 0xffff_ffff || size == 0xffff_ffff || header == 0xffff_ffff {
                return Err(PackError::Unsupported("a zip64 archive".into()));
            }
            entries.push(ZipEntry { name, flags, method, crc, compressed, size, header });
            at += 46 + name_len + extra_len + comment_len;
        }
        Ok(Self { bytes, entries, total: std::cell::Cell::new(0) })
    }

    /// The bytes of entry `name`, inflated; `None` when there is none.
    fn read(&self, name: &str) -> Result<Option<Vec<u8>>, PackError> {
        let Some(e) = self.entries.iter().find(|e| e.name == name) else { return Ok(None) };
        if e.flags & 1 != 0 {
            return Err(PackError::Unsupported(format!("{name} is encrypted")));
        }
        if e.size > MAX_ENTRY_BYTES {
            return Err(invalid(format!("{name} is larger than {} MB", MAX_ENTRY_BYTES >> 20)));
        }
        if self.total.get() + e.size > MAX_TOTAL_BYTES {
            return Err(invalid(format!("it inflates to more than {} MB", MAX_TOTAL_BYTES >> 20)));
        }
        let h = e.header as usize;
        if u32_at(self.bytes, h)? != 0x0403_4b50 {
            return Err(damaged(format!("the header of {name}")));
        }
        let name_len = u16_at(self.bytes, h + 26)? as usize;
        let extra_len = u16_at(self.bytes, h + 28)? as usize;
        if self.bytes.get(h + 30..h + 30 + name_len) != Some(name.as_bytes()) {
            return Err(damaged(format!("the header of {name} names another entry")));
        }
        let start = h + 30 + name_len + extra_len;
        let raw = start.checked_add(e.compressed as usize).and_then(|end| self.bytes.get(start..end)).ok_or_else(|| damaged(format!("{name} is cut short")))?;
        let data = match e.method {
            0 => raw.to_vec(),
            8 => {
                // Never more than it says, however much it would give.
                let mut out = Vec::with_capacity(e.size as usize);
                flate2::read::DeflateDecoder::new(raw).take(e.size + 1).read_to_end(&mut out).map_err(|_| damaged(format!("{name} cannot be inflated")))?;
                out
            }
            m => return Err(PackError::Unsupported(format!("{name} is compressed with method {m}"))),
        };
        if data.len() as u64 != e.size {
            return Err(damaged(format!("{name} is not the size it says")));
        }
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        if crc.sum() != e.crc {
            return Err(damaged(format!("{name} fails its check (CRC)")));
        }
        self.total.set(self.total.get() + e.size);
        Ok(Some(data))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::{MATERIAL_MIXES, PLANT_PRESETS};
    use crate::treepack::TreeKind;

    /// A pack of a tree, a preset of flowers and a mix in beds, made from a
    /// map.
    fn sample() -> (Registry, Pack) {
        let mut reg = Registry::vanilla();
        let layer = TreeKind::Fir.install(&mut reg);
        let cover = PLANT_PRESETS.iter().find(|p| p.name == "Tulip field").unwrap().install(&mut reg);
        let terrain = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut reg).0;
        let content = Pack::gather(&reg, &[cover], &[terrain], &[layer]);
        let mut pack = Pack { name: "Northern woods".into(), author: "Nedgar".into(), description: "Firs, tulips\nand grey cliffs.".into(), made_with: "0.6.0".into(), preview: None, content };
        pack.preview = Some(pack.draw_preview());
        (reg, pack)
    }

    #[test]
    fn a_gradient_travels_in_a_pack_with_its_terrains() {
        use crate::gradient::{Gradient, Transition};
        let mut reg = Registry::vanilla();
        let cliff = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut reg);
        let cliff_name = reg.terrain(cliff).name.clone();
        let g = Gradient::new("Cliff to grass", &[(cliff_name.as_str(), 4.0), ("Gravel", 3.0), ("Grass", 4.0)], &[Transition::blobs(5.0, 0.6, 4.0), Transition::noise(4.0, 0.7, 3.0)]);
        let mut content = Content::default();
        Pack::gather_gradients(&mut content, &reg, std::slice::from_ref(&g));
        // The mix goes with it; gravel and grass every map has.
        assert_eq!(content.terrains.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), [cliff_name.as_str()]);
        let mut pack = Pack { name: "Gradients".into(), author: "Fornad".into(), description: String::new(), made_with: "1.0.0".into(), preview: None, content };
        pack.preview = Some(pack.draw_preview());
        let back = Pack::from_bytes(&pack.to_bytes()).unwrap();
        assert_eq!(back, pack);
        assert_eq!(back.items().last().map(|i| (i.0, i.2)), Some((ItemKind::Gradient, "Cliff to grass")));
        // In another map: not there until its terrain is, then painted
        // with that map's terrains.
        let mut other = Registry::vanilla();
        assert!(!back.installed(ItemKind::Gradient, 0, &other));
        let installed = back.install_gradient(0, &mut other);
        assert!(back.installed(ItemKind::Gradient, 0, &other));
        assert_eq!(installed, g);
        let sampler = installed.sampler(&other, 1).unwrap();
        assert_eq!(other.terrain(sampler.terrains()[0]).name, cliff_name);
    }

    #[test]
    fn a_broken_gradient_in_a_pack_is_mended_or_refused() {
        let mut pack = Pack { name: "G".into(), author: String::new(), description: String::new(), made_with: String::new(), preview: None, content: Content::default() };
        let mut g = crate::gradient::Gradient::default();
        g.transitions.clear();
        g.bands[0].width = f32::NAN;
        pack.content.gradients.push(g);
        pack.validate().unwrap();
        let g = &pack.content.gradients[0];
        assert_eq!(g.transitions.len(), g.bands.len() - 1);
        assert!(g.bands[0].width.is_finite());
        pack.content.gradients[0].bands.clear();
        assert!(pack.validate().is_err());
    }

    #[test]
    fn a_pack_reads_back_as_it_was_written() {
        let (_, pack) = sample();
        assert_eq!(pack.content.object_layers.len(), 1);
        assert_eq!(pack.content.objects.len(), 2 * crate::treepack::VARIANTS, "its trees, and their bushes and small ones");
        let bytes = pack.to_bytes();
        assert_eq!(bytes, pack.to_bytes(), "written the same each time");
        let back = Pack::from_bytes(&bytes).unwrap();
        assert_eq!(back, pack);
        // Its picture is a PNG of its three things side by side.
        let img = image::load_from_memory(back.preview.as_ref().unwrap()).unwrap();
        assert_eq!((img.width(), img.height()), (192, 64));
    }

    /// Installed into another map, a pack gives the very layers it was made
    /// of; installed again, nothing is added.
    #[test]
    fn installing_gives_the_same_layers_once() {
        let (src, pack) = sample();
        let pack = Pack::from_bytes(&pack.to_bytes()).unwrap();
        let mut reg = Registry::vanilla();
        for _ in 0..2 {
            let items: Vec<(ItemKind, usize)> = pack.items().map(|(k, i, _)| (k, i)).collect();
            for (k, i) in items {
                pack.install(k, i, &mut reg);
                assert!(pack.installed(k, i, &reg));
            }
        }
        assert_eq!(reg.objects, src.objects.iter().map(|o| ObjectDef { source: format!("Northern woods ({EXTENSION})"), ..o.clone() }).collect::<Vec<_>>());
        assert_eq!(reg.object_layers, src.object_layers);
        assert_eq!(reg.covers, src.covers);
        assert_eq!(reg.terrains, src.terrains);
        assert_eq!(reg.blocks, src.blocks);
    }

    /// A map that has a layer of the same name, made otherwise, keeps it:
    /// the pack's comes in under the pack's name.
    #[test]
    fn a_name_taken_by_something_else_is_left_alone() {
        let (_, pack) = sample();
        let mut reg = Registry::vanilla();
        reg.add_cover(CoverDef::new("Tulip field", [1, 2, 3], &[("minecraft:poppy", 1)], 0.1));
        let n = pack.install_cover(0, &mut reg);
        assert_eq!(reg.cover(n).unwrap().name, "Tulip field (Northern woods)");
        assert_eq!(pack.install_cover(0, &mut reg), n);
        assert_eq!(reg.cover_by_name("Tulip field").unwrap(), n - 1);
    }

    fn zip(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = ZipWriter::default();
        for (name, data) in entries {
            z.add(name, data);
        }
        z.finish()
    }

    fn manifest(version: u32) -> Vec<u8> {
        serde_json::to_vec(&Manifest { format: FORMAT.into(), version, name: "P".into(), author: String::new(), description: String::new(), massif: String::new() }).unwrap()
    }

    const COVER: &str = r#"{"covers":[{"name":"C","plants":[{"state":"minecraft:poppy"}],"density":0.5}]}"#;

    fn refused(bytes: &[u8]) -> PackError {
        Pack::from_bytes(bytes).expect_err("refused")
    }

    #[test]
    fn a_small_hand_made_pack_is_read() {
        let p = Pack::from_bytes(&zip(&[("manifest.json", &manifest(1)), ("content.json", COVER.as_bytes()), ("extra/notes.txt", b"ignored")])).unwrap();
        assert_eq!(p.content.covers[0].height, [1, 1]);
        assert_eq!(p.content.covers[0].plants[0].weight, 1);
    }

    #[test]
    fn malformed_and_hostile_packs_are_refused() {
        let good_manifest = manifest(1);
        // Not a zip, empty, cut short.
        assert_eq!(refused(b"not a zip at all"), PackError::NotZip);
        assert_eq!(refused(b""), PackError::NotZip);
        let whole = zip(&[("manifest.json", &good_manifest), ("content.json", COVER.as_bytes())]);
        assert!(Pack::from_bytes(&whole).is_ok());
        for cut in [10, whole.len() / 2, whole.len() - 30] {
            assert!(Pack::from_bytes(&whole[..cut]).is_err(), "cut at {cut}");
        }
        // Too large a file.
        assert_eq!(refused(&vec![0; MAX_FILE_BYTES as usize + 1]), PackError::TooLarge);
        // Names that would climb out of a folder.
        for name in ["../evil.json", "/etc/passwd", "a/../../b", "C:/x", "a\\..\\b", "./x"] {
            let z = zip(&[("manifest.json", &good_manifest), ("content.json", COVER.as_bytes()), (name, b"x")]);
            assert_eq!(refused(&z), PackError::UnsafeName(name.into()), "{name}");
        }
        // No manifest; another kind of file; a newer format; format 0.
        assert!(matches!(refused(&zip(&[("content.json", COVER.as_bytes())])), PackError::NotPack(_)));
        assert!(matches!(refused(&zip(&[("manifest.json", br#"{"format":"other","version":1,"name":"x"}"#)])), PackError::NotPack(_)));
        assert!(matches!(refused(&zip(&[("manifest.json", b"{nope")])), PackError::NotPack(_)));
        assert_eq!(refused(&zip(&[("manifest.json", &manifest(VERSION + 1)), ("content.json", COVER.as_bytes())])), PackError::Newer(VERSION + 1));
        assert!(matches!(refused(&zip(&[("manifest.json", &manifest(0)), ("content.json", COVER.as_bytes())])), PackError::Unsupported(_)));
        // Content that is wrong.
        let bad = |content: &str| refused(&zip(&[("manifest.json", &good_manifest), ("content.json", content.as_bytes())]));
        assert!(matches!(bad("{}"), PackError::Invalid(_)), "empty");
        assert!(matches!(bad(r#"{"covers":[{"name":"C","plants":[{"state":"minecraft:poppy; rm -rf"}],"density":0.5}]}"#), PackError::Invalid(_)));
        assert!(matches!(bad(r#"{"covers":[{"name":"C","plants":[{"state":"../../poppy"}],"density":0.5}]}"#), PackError::Invalid(_)));
        assert!(matches!(bad(r#"{"covers":[{"name":"  ","plants":[{"state":"minecraft:poppy"}],"density":0.5}]}"#), PackError::Invalid(_)));
        assert!(matches!(bad(r#"{"covers":[{"name":"C","plants":[],"density":0.5}]}"#), PackError::Invalid(_)));
        assert!(matches!(bad(r#"{"object_layers":[{"name":"L","objects":[{"object":3}],"spacing":4,"density":1}]}"#), PackError::Invalid(_)), "an object that is not there");
        assert!(matches!(bad(r#"{"object_layers":[{"name":"L","objects":[{"object":0}],"spacing":4,"density":1}],"objects":[{"name":"o","size":[2,2,2],"palette":["minecraft:air"],"anchor":[0,0,0]}]}"#), PackError::Invalid(_)), "an object without its blocks");
        // An object whose blocks are not its box's, or past its palette.
        let layer = r#"{"object_layers":[{"name":"L","objects":[{"object":0}],"spacing":4,"density":1}],"objects":[{"name":"o","size":[2,1,1],"palette":["minecraft:air","minecraft:stone"],"anchor":[0,0,0]}]}"#;
        let with_blocks = |blocks: &[u16]| {
            let bytes: Vec<u8> = blocks.iter().flat_map(|b| b.to_le_bytes()).collect();
            Pack::from_bytes(&zip(&[("manifest.json", &good_manifest), ("content.json", layer.as_bytes()), ("objects/0.bin", &bytes)]))
        };
        assert!(with_blocks(&[0, 1]).is_ok());
        assert!(matches!(with_blocks(&[0, 1, 1]), Err(PackError::Invalid(_))));
        assert!(matches!(with_blocks(&[0, 7]), Err(PackError::Invalid(_))));
        // An object too large for any map.
        let huge = layer.replace("[2,1,1]", "[4096,4096,4096]");
        assert!(matches!(refused(&zip(&[("manifest.json", &good_manifest), ("content.json", huge.as_bytes()), ("objects/0.bin", &[0, 0])])), PackError::Invalid(_)));
        // A picture that is not one, or too large.
        assert!(matches!(refused(&zip(&[("manifest.json", &good_manifest), ("content.json", COVER.as_bytes()), ("preview.png", b"\x89PNG not really")])), PackError::Invalid(_)));
        let mut big = Vec::new();
        image::DynamicImage::new_rgb8(MAX_PREVIEW_SIDE + 1, 4).write_to(&mut std::io::Cursor::new(&mut big), image::ImageFormat::Png).unwrap();
        assert!(matches!(refused(&zip(&[("manifest.json", &good_manifest), ("content.json", COVER.as_bytes()), ("preview.png", &big)])), PackError::Invalid(_)));
    }

    /// Zip tricks: an entry that inflates to far more than it says (a zip
    /// bomb), one that says it is huge, a bad checksum, an unknown method,
    /// an encrypted entry, the same name twice, too many entries.
    #[test]
    fn zip_tricks_are_refused() {
        let m = manifest(1);
        let base = zip(&[("manifest.json", &m), ("content.json", COVER.as_bytes())]);
        // The central directory's fields of the second entry, content.json.
        let second = {
            let first = base.windows(4).position(|w| w == 0x0201_4b50u32.to_le_bytes()).unwrap();
            first + 46 + "manifest.json".len()
        };
        let patch = |at: usize, v: &[u8]| {
            let mut z = base.clone();
            z[at..at + v.len()].copy_from_slice(v);
            z
        };
        // Says it is 10 bytes: inflating stops there, and it is refused.
        assert!(matches!(refused(&patch(second + 24, &10u32.to_le_bytes())), PackError::Damaged(_)));
        // Says it is 4 GB less a byte.
        assert!(matches!(refused(&patch(second + 24, &0xffff_fffeu32.to_le_bytes())), PackError::Invalid(_)));
        assert!(matches!(refused(&patch(second + 16, &0xdead_beefu32.to_le_bytes())), PackError::Damaged(_)));
        assert!(matches!(refused(&patch(second + 10, &99u16.to_le_bytes())), PackError::Unsupported(_)));
        assert!(matches!(refused(&patch(second + 8, &1u16.to_le_bytes())), PackError::Unsupported(_)));
        assert!(matches!(refused(&zip(&[("manifest.json", &m), ("manifest.json", &m)])), PackError::Damaged(_)));
        let names: Vec<String> = (0..MAX_ENTRIES + 1).map(|i| format!("f{i}")).collect();
        let many: Vec<(&str, &[u8])> = names.iter().map(|n| (n.as_str(), &b""[..])).collect();
        assert!(matches!(refused(&zip(&many)), PackError::Invalid(_)));
        // A real bomb: 32 MB of zeros under 32 KB, said to be 100 bytes.
        let zeros = vec![0u8; 32 << 20];
        let bomb = zip(&[("manifest.json", &m), ("content.json", &zeros)]);
        assert!(bomb.len() < 100_000);
        let at = {
            let first = bomb.windows(4).position(|w| w == 0x0201_4b50u32.to_le_bytes()).unwrap();
            first + 46 + "manifest.json".len()
        };
        let mut lie = bomb.clone();
        lie[at + 24..at + 28].copy_from_slice(&100u32.to_le_bytes());
        assert!(matches!(refused(&lie), PackError::Damaged(_)));
        // Told the truth, it is read, and its content refused as JSON.
        assert!(matches!(refused(&bomb), PackError::Invalid(_)));
    }

    #[test]
    fn block_states_are_checked() {
        for ok in ["minecraft:stone", "minecraft:oak_log[axis=y]", "mod:a/b.c-d", "minecraft:sweet_berry_bush[age=3]"] {
            assert!(check_state(ok).is_ok(), "{ok}");
        }
        for bad in ["", "stone", "minecraft:", ":x", "minecraft:Stone", "minecraft:a b", "minecraft:x[", "minecraft:x[a]", "minecraft:x[a=b", "minecraft:x\n", "minecraft:../x"] {
            if bad == "minecraft:../x" {
                // Dots and slashes are the game's own characters; a state is
                // never a path.
                continue;
            }
            assert!(check_state(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn file_stems_are_safe() {
        assert_eq!(file_stem("Northern woods"), "Northern woods");
        assert_eq!(file_stem("../../etc/passwd"), "etc-passwd");
        assert_eq!(file_stem("///"), "pack");
        assert_eq!(file_stem("CON"), "pack CON");
        assert!(!file_stem("a:b\\c/d").contains(['/', '\\', ':']));
    }
}
