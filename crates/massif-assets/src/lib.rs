//! Minecraft's own blocks: their ids, models, textures and colours, read
//! from the player's installation of the game.
//!
//! Nothing of the game is shipped with Massif. The client jar of a version
//! the player has installed (with the official launcher, CurseForge, Prism…)
//! is opened in place: every block state file gives a block, its model gives
//! the textures of its top, sides and bottom, and those give its real
//! colour, with grass and leaves tinted the way the game tints them.

pub mod zip;

use rayon::prelude::*;
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Debug, thiserror::Error)]
pub enum AssetError {
    #[error("{0}")]
    Zip(#[from] zip::ZipError),
    #[error("this file has no Minecraft blocks in it")]
    NoBlocks,
}

/// A Minecraft client jar found on this computer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jar {
    pub version: String,
    pub path: PathBuf,
}

/// Version numbers of a release (`1.21.11`, `26.3`), for ordering; `None`
/// for snapshots, previews and loaders.
pub fn release_numbers(version: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = version.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| p.len() >= 2)
}

/// Folders where launchers keep their game versions.
fn version_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(home) = &home {
        dirs.push(home.join("Library/Application Support/minecraft/versions"));
        dirs.push(home.join(".minecraft/versions"));
        dirs.push(home.join("Documents/curseforge/minecraft/Install/versions"));
        dirs.push(home.join("curseforge/minecraft/Install/versions"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        dirs.push(PathBuf::from(appdata).join(".minecraft/versions"));
    }
    // Launchers installed on other drives (CurseForge often is).
    for root in ["/Volumes", "/media", "/mnt"] {
        if let Ok(read) = std::fs::read_dir(root) {
            for volume in read.flatten() {
                for sub in ["minecraft/Install/versions", "curseforge/minecraft/Install/versions", "minecraft/versions"] {
                    dirs.push(volume.path().join(sub));
                }
            }
        }
    }
    dirs
}

/// Client jars of released versions, newest first. Prism and MultiMC keep
/// them among their libraries.
pub fn find_jars() -> Vec<Jar> {
    let mut jars = Vec::new();
    for dir in version_dirs() {
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let version = entry.file_name().to_string_lossy().into_owned();
            let jar = entry.path().join(format!("{version}.jar"));
            if release_numbers(&version).is_some() && jar.is_file() {
                jars.push(Jar { version, path: jar });
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for lib in [
            "Library/Application Support/PrismLauncher/libraries/com/mojang/minecraft",
            ".local/share/PrismLauncher/libraries/com/mojang/minecraft",
        ] {
            let Ok(read) = std::fs::read_dir(home.join(lib)) else { continue };
            for entry in read.flatten() {
                let version = entry.file_name().to_string_lossy().into_owned();
                let jar = entry.path().join(format!("minecraft-{version}-client.jar"));
                if release_numbers(&version).is_some() && jar.is_file() {
                    jars.push(Jar { version, path: jar });
                }
            }
        }
    }
    jars.sort_by_key(|j| std::cmp::Reverse(release_numbers(&j.version)));
    jars.dedup_by(|a, b| a.version == b.version);
    jars
}

/// The number a client jar stamps on the worlds it saves (its
/// `version.json` says, from 1.14 on).
pub fn data_version(jar: &Path) -> Option<i32> {
    let archive = zip::Archive::open(jar).ok()?;
    let json: Value = serde_json::from_slice(&archive.read("version.json")?.ok()?).ok()?;
    let version = json.get("world_version")?.as_i64()?;
    i32::try_from(version).ok()
}

/// How the game colours a texture.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tint {
    None,
    /// By the biome's grass colour (plains here).
    Grass,
    /// By the biome's foliage colour.
    Foliage,
    Water,
    Fixed([u8; 3]),
}

impl Tint {
    pub fn color(self) -> [u8; 3] {
        match self {
            Tint::None => [255, 255, 255],
            Tint::Grass => [0x91, 0xbd, 0x59],
            Tint::Foliage => [0x77, 0xab, 0x2f],
            Tint::Water => [0x3f, 0x76, 0xe4],
            Tint::Fixed(c) => c,
        }
    }
}

/// Groups of the block palette.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Category {
    Natural,
    Stone,
    Ores,
    Wood,
    Plants,
    Building,
    Colored,
    Nether,
    End,
    Other,
}

impl Category {
    pub const ALL: [Category; 10] = [
        Category::Natural,
        Category::Stone,
        Category::Ores,
        Category::Wood,
        Category::Plants,
        Category::Building,
        Category::Colored,
        Category::Nether,
        Category::End,
        Category::Other,
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Category::Natural => "Soil, sand and snow",
            Category::Stone => "Stone",
            Category::Ores => "Ores and minerals",
            Category::Wood => "Wood",
            Category::Plants => "Plants and leaves",
            Category::Building => "Building blocks",
            Category::Colored => "Wool, concrete, terracotta, glass",
            Category::Nether => "Nether",
            Category::End => "End",
            Category::Other => "Other",
        }
    }

    fn of(id: &str) -> Self {
        let n = id.trim_start_matches("minecraft:");
        let has = |s: &str| n.contains(s);
        const COLORS: [&str; 16] = [
            "white", "orange", "magenta", "light_blue", "yellow", "lime", "pink", "gray", "light_gray", "cyan", "purple", "blue", "brown",
            "green", "red", "black",
        ];
        if has("wool") || has("concrete") || has("terracotta") || has("glass") || COLORS.iter().any(|c| n.starts_with(c) && (has("carpet") || has("candle") || has("bed") || has("banner") || has("shulker"))) {
            Category::Colored
        } else if has("nether") || has("crimson") || has("warped") || has("soul") || has("basalt") || has("blackstone") || has("magma") || has("glowstone") || has("shroomlight") || has("quartz") {
            Category::Nether
        } else if has("end_") || has("purpur") || has("chorus") || n == "end_stone" {
            Category::End
        } else if has("_ore") || n.starts_with("raw_") || ["coal_block", "iron_block", "gold_block", "diamond_block", "emerald_block", "lapis_block", "redstone_block", "copper_block", "netherite_block", "amethyst_block", "budding_amethyst"].contains(&n) {
            Category::Ores
        } else if has("leaves") || has("sapling") || has("flower") || has("grass") && n != "grass_block" || has("fern") || has("mushroom") || has("vine") || has("moss") && !has("cobblestone") && !has("brick") || has("azalea") || has("bush") || has("tulip") || has("orchid") || has("daisy") || has("poppy") || has("dandelion") || has("lily") || has("kelp") || has("coral") || has("bamboo") && !has("planks") && !has("mosaic") && !has("block") || has("cactus") || has("cane") || has("roots") || has("dripleaf") || has("petals") {
            Category::Plants
        } else if has("planks") || has("_log") || has("_wood") || has("stripped") || has("bamboo") {
            Category::Wood
        } else if ["grass_block", "dirt", "coarse_dirt", "rooted_dirt", "podzol", "mycelium", "mud", "clay", "gravel", "sand", "red_sand", "snow_block", "snow", "ice", "packed_ice", "blue_ice", "powder_snow", "dirt_path", "farmland", "suspicious_sand", "suspicious_gravel", "pale_moss_block"].contains(&n) {
            Category::Natural
        } else if has("brick") || has("polished") || has("chiseled") || has("cut_") || has("smooth") || has("tiles") || has("slab") || has("stairs") || has("wall") || has("pillar") {
            Category::Building
        } else if has("stone") || has("granite") || has("diorite") || has("andesite") || has("deepslate") || has("tuff") || has("calcite") || has("dripstone") || has("obsidian") || n == "bedrock" {
            Category::Stone
        } else {
            Category::Other
        }
    }
}

/// The first frame of a texture, as RGBA.
#[derive(Debug, Clone, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl Texture {
    /// Mean colour of the visible pixels, tinted.
    pub fn mean(&self, tint: Tint) -> [u8; 3] {
        let (mut sum, mut n) = ([0u64; 3], 0u64);
        for p in self.rgba.as_chunks::<4>().0 {
            if p[3] > 16 {
                for k in 0..3 {
                    sum[k] += p[k] as u64;
                }
                n += 1;
            }
        }
        let t = tint.color();
        [0, 1, 2].map(|k| ((sum[k] / n.max(1)) * t[k] as u64 / 255) as u8)
    }

    /// The texture with a tint applied.
    pub fn tinted(&self, tint: Tint) -> Texture {
        if tint == Tint::None {
            return self.clone();
        }
        let t = tint.color();
        let mut rgba = self.rgba.clone();
        for p in rgba.as_chunks_mut::<4>().0 {
            for k in 0..3 {
                p[k] = (p[k] as u32 * t[k] as u32 / 255) as u8;
            }
        }
        Texture { rgba, ..*self }
    }
}

/// One block of the game.
#[derive(Debug, Clone, PartialEq)]
pub struct Block {
    /// Namespaced id, `minecraft:grass_block`.
    pub id: String,
    /// `Grass Block`.
    pub name: String,
    /// Texture keys (`block/grass_block_top`).
    pub top: Option<String>,
    pub side: Option<String>,
    pub bottom: Option<String>,
    /// Tints of the top and the sides.
    pub top_tint: Tint,
    pub side_tint: Tint,
    /// A whole cube: usable as ground.
    pub full_cube: bool,
    /// Real colours of the top and the sides.
    pub color: [u8; 3],
    pub side_color: [u8; 3],
    pub category: Category,
}

/// Every block of a version of the game.
#[derive(Debug, Clone)]
pub struct Catalogue {
    pub version: String,
    pub source: PathBuf,
    /// Sorted by id.
    pub blocks: Vec<Block>,
    pub textures: HashMap<String, Arc<Texture>>,
}

impl Catalogue {
    pub fn block(&self, id: &str) -> Option<&Block> {
        let id = id.split('[').next().unwrap_or(id);
        self.blocks.binary_search_by(|b| b.id.as_str().cmp(id)).ok().map(|i| &self.blocks[i])
    }

    pub fn texture(&self, key: &str) -> Option<&Arc<Texture>> {
        self.textures.get(key)
    }
}

/// `grass_block` → `Grass Block`.
fn display_name(id: &str) -> String {
    id.trim_start_matches("minecraft:")
        .split('_')
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// The texture key of a model reference (`minecraft:block/stone` →
/// `block/stone`).
fn key(path: &str) -> String {
    path.trim_start_matches("minecraft:").to_owned()
}

struct Models {
    raw: HashMap<String, Value>,
}

/// A model with its parents folded in: textures by variable, and the faces
/// of its elements (direction → texture variable, tinted?), and whether it
/// fills its whole cube.
#[derive(Default)]
struct Resolved {
    textures: HashMap<String, String>,
    faces: HashMap<String, (String, bool)>,
    full_cube: bool,
}

impl Models {
    fn resolve(&self, name: &str) -> Resolved {
        let mut chain = Vec::new();
        let mut current = key(name);
        for _ in 0..16 {
            let Some(model) = self.raw.get(&current) else { break };
            chain.push(model);
            match model.get("parent").and_then(Value::as_str) {
                Some(p) => current = key(p),
                None => break,
            }
        }
        let mut r = Resolved::default();
        // The root first, so that children override.
        for model in chain.iter().rev() {
            if let Some(t) = model.get("textures").and_then(Value::as_object) {
                for (k, v) in t {
                    if let Some(v) = v.as_str() {
                        r.textures.insert(k.clone(), v.to_owned());
                    }
                }
            }
        }
        // Elements come from the nearest model that has them.
        if let Some(elements) = chain.iter().find_map(|m| m.get("elements").and_then(Value::as_array)) {
            for el in elements {
                let corner = |k: &str| {
                    el.get(k)
                        .and_then(Value::as_array)
                        .map(|a| a.iter().filter_map(Value::as_f64).collect::<Vec<_>>())
                        .unwrap_or_default()
                };
                if corner("from") == [0.0, 0.0, 0.0] && corner("to") == [16.0, 16.0, 16.0] {
                    r.full_cube = true;
                }
                if let Some(faces) = el.get("faces").and_then(Value::as_object) {
                    for (dir, face) in faces {
                        if let Some(t) = face.get("texture").and_then(Value::as_str) {
                            r.faces.entry(dir.clone()).or_insert((t.to_owned(), face.get("tintindex").is_some()));
                        }
                    }
                }
            }
        }
        r
    }
}

impl Resolved {
    /// Follow `#variable` references to a texture key.
    fn texture(&self, var: &str) -> Option<String> {
        let mut var = var.to_owned();
        for _ in 0..8 {
            match var.strip_prefix('#') {
                Some(name) => var = self.textures.get(name)?.clone(),
                None => return Some(key(&var)),
            }
        }
        None
    }

    fn face(&self, dirs: &[&str]) -> Option<(String, bool)> {
        dirs.iter()
            .find_map(|d| self.faces.get(*d))
            .and_then(|(v, tint)| Some((self.texture(v)?, *tint)))
    }
}

/// The first model a block state file points to.
fn first_model(state: &Value) -> Option<String> {
    let pick = |v: &Value| -> Option<String> {
        let v = if let Some(a) = v.as_array() { a.first()? } else { v };
        v.get("model").and_then(Value::as_str).map(str::to_owned)
    };
    if let Some(variants) = state.get("variants").and_then(Value::as_object) {
        return variants.values().next().and_then(pick);
    }
    state
        .get("multipart")
        .and_then(Value::as_array)?
        .iter()
        .find_map(|part| part.get("apply").and_then(pick))
}

/// How the game tints a block's tinted faces.
fn tint_of(id: &str) -> Tint {
    let n = id.trim_start_matches("minecraft:");
    match n {
        "spruce_leaves" => Tint::Fixed([0x61, 0x99, 0x61]),
        "birch_leaves" => Tint::Fixed([0x80, 0xa7, 0x55]),
        "lily_pad" => Tint::Fixed([0x20, 0x80, 0x30]),
        "redstone_wire" => Tint::Fixed([0xd0, 0x10, 0x10]),
        _ if n.contains("water") || n == "bubble_column" => Tint::Water,
        _ if n.contains("leaves") || n == "vine" => Tint::Foliage,
        _ if n.contains("stem") => Tint::Fixed([0xe0, 0xc7, 0x1c]),
        _ => Tint::Grass,
    }
}

/// Read every block of a client jar.
pub fn load(path: &Path, version: &str) -> Result<Catalogue, AssetError> {
    let archive = zip::Archive::open(path)?;
    const STATES: &str = "assets/minecraft/blockstates/";
    const MODELS: &str = "assets/minecraft/models/";
    const TEXTURES: &str = "assets/minecraft/textures/";
    let names: Vec<String> = archive.names().map(str::to_owned).collect();
    let json = |name: &str| -> Option<Value> { serde_json::from_slice(&archive.read(name)?.ok()?).ok() };
    let models = Models {
        raw: names
            .iter()
            .filter(|n| n.starts_with(MODELS) && n.ends_with(".json"))
            .filter_map(|n| Some((n[MODELS.len()..n.len() - 5].to_owned(), json(n)?)))
            .collect(),
    };
    let states: Vec<(String, Value)> = names
        .iter()
        .filter(|n| n.starts_with(STATES) && n.ends_with(".json"))
        .filter_map(|n| Some((format!("minecraft:{}", &n[STATES.len()..n.len() - 5]), json(n)?)))
        .collect();
    if states.is_empty() {
        return Err(AssetError::NoBlocks);
    }
    // Textures: the first frame of every block texture, decoded in parallel.
    let png: Vec<(String, Vec<u8>)> = names
        .iter()
        .filter(|n| n.starts_with(TEXTURES) && n.contains("/block/") && n.ends_with(".png"))
        .filter_map(|n| Some((n[TEXTURES.len()..n.len() - 4].to_owned(), archive.read(n)?.ok()?)))
        .collect();
    let textures: HashMap<String, Arc<Texture>> = png
        .into_par_iter()
        .filter_map(|(k, bytes)| {
            let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
            let (w, h) = img.dimensions();
            // Animated textures are a strip of square frames.
            let side = w.min(h);
            let frame = image::imageops::crop_imm(&img, 0, 0, w, side).to_image();
            Some((
                k,
                Arc::new(Texture {
                    width: w,
                    height: side,
                    rgba: frame.into_raw(),
                }),
            ))
        })
        .collect();

    let mut blocks: Vec<Block> = states
        .iter()
        .map(|(id, state)| {
            let model = first_model(state).map(|m| models.resolve(&m)).unwrap_or_default();
            let particle = model.textures.get("particle").and_then(|_| model.texture("#particle"));
            let tint = tint_of(id);
            let pick = |dirs: &[&str]| -> (Option<String>, Tint) {
                match model.face(dirs) {
                    Some((t, tinted)) => (Some(t), if tinted { tint } else { Tint::None }),
                    None => (particle.clone(), Tint::None),
                }
            };
            let (top, top_tint) = pick(&["up"]);
            let (side, side_tint) = pick(&["north", "south", "east", "west"]);
            let (bottom, _) = pick(&["down"]);
            let color_of = |t: &Option<String>, tint: Tint| {
                t.as_ref()
                    .and_then(|k| textures.get(k))
                    .map_or([128, 128, 128], |tex| tex.mean(tint))
            };
            // Water and lava have no model: their colour is their texture's.
            let (top, top_tint) = match id.as_str() {
                "minecraft:water" => (Some("block/water_still".to_owned()), Tint::Water),
                "minecraft:lava" => (Some("block/lava_still".to_owned()), Tint::None),
                _ => (top, top_tint),
            };
            Block {
                id: id.clone(),
                name: display_name(id),
                color: color_of(&top, top_tint),
                side_color: color_of(&side, side_tint),
                top,
                side,
                bottom,
                top_tint,
                side_tint,
                full_cube: model.full_cube,
                category: Category::of(id),
            }
        })
        .collect();
    blocks.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(Catalogue {
        version: version.to_owned(),
        source: path.to_owned(),
        blocks,
        textures,
    })
}

/// The newest jar found on this computer, loaded.
pub fn load_newest() -> Option<Result<Catalogue, AssetError>> {
    let jar = find_jars().into_iter().next()?;
    Some(load(&jar.path, &jar.version))
}

// --------------------------------------------------------------- registry --

use massif_core::registry::{BlockId, Registry, Stratum, TerrainDef, TerrainId};

/// Give every block and terrain of a registry its real colour: the mean of
/// its top texture, tinted as in the game; a terrain takes the mix of its
/// surface layer.
pub fn apply_colors(reg: &mut Registry, cat: &Catalogue) {
    for b in reg.blocks.iter_mut().skip(1) {
        if let Some(real) = cat.block(&b.state) {
            b.color = real.color;
        }
    }
    let blocks = reg.blocks.clone();
    for t in reg.terrains.iter_mut() {
        let Some(top) = t.strata.first() else { continue };
        let total: u32 = top.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
        let mut c = [0u32; 3];
        for &(id, w) in &top.blocks {
            let bc = blocks.get(id.index()).map_or([128; 3], |b| b.color);
            for k in 0..3 {
                c[k] += bc[k] as u32 * w as u32;
            }
        }
        t.color = c.map(|v| (v / total) as u8);
    }
}

/// The group of the terrains made of one real block.
pub const BLOCK_GROUP: &str = "Blocks";

/// A terrain made of one real block, added to the registry the first time:
/// a few blocks deep (grass, podzol and mycelium over dirt, as in the game),
/// natural rock below.
pub fn block_terrain(reg: &mut Registry, block: &Block) -> TerrainId {
    if let Some(i) = reg
        .terrains
        .iter()
        .position(|t| t.group == BLOCK_GROUP && t.strata.first().and_then(|s| s.blocks.first()).is_some_and(|b| reg.blocks[b.0.index()].name() == block.id))
    {
        return TerrainId(i as u16);
    }
    let id: BlockId = reg.intern_block(&block.id, block.color);
    let on_dirt = ["minecraft:grass_block", "minecraft:podzol", "minecraft:mycelium", "minecraft:moss_block", "minecraft:pale_moss_block"].contains(&block.id.as_str());
    let strata = if on_dirt {
        let dirt = reg.intern_block("minecraft:dirt", [134, 96, 67]);
        vec![
            Stratum {
                blocks: vec![(id, 1)],
                depth: (1, 1),
            },
            Stratum {
                blocks: vec![(dirt, 1)],
                depth: (3, 4),
            },
        ]
    } else {
        vec![Stratum {
            blocks: vec![(id, 1)],
            depth: (3, 5),
        }]
    };
    reg.terrains.push(TerrainDef {
        name: block.name.clone(),
        group: BLOCK_GROUP.to_owned(),
        color: block.color,
        strata,
        submerged_top: None,
        biome_hint: None,
    });
    TerrainId((reg.terrains.len() - 1) as u16)
}

/// The block shown for a terrain: the main block of its surface layer.
pub fn terrain_block<'a>(reg: &Registry, cat: &'a Catalogue, terrain: TerrainId) -> Option<&'a Block> {
    let top = reg.terrain(terrain).strata.first()?;
    let (id, _) = top.blocks.iter().max_by_key(|b| b.1)?;
    cat.block(&reg.block(*id).state)
}

// ------------------------------------------------------------------ icons --

/// A block drawn as a small isometric cube with its real textures: the top
/// lit, the two sides in shade. `size` pixels square, RGBA, transparent
/// around the cube.
pub fn cube_icon(cat: &Catalogue, block: &Block, size: usize) -> Vec<u8> {
    let s = size as f32;
    let tex = |key: &Option<String>, tint: Tint| key.as_ref().and_then(|k| cat.texture(k)).map(|t| t.tinted(tint));
    let top = tex(&block.top, block.top_tint);
    let side = tex(&block.side, block.side_tint);
    let mut out = vec![0u8; size * size * 4];
    // Corners of the cube's outline.
    let (t, r, b, l) = ((s / 2.0, 0.0), (s, s / 4.0), (s / 2.0, s / 2.0), (0.0, s / 4.0));
    let sample = |tx: &Option<Texture>, u: f32, v: f32, fallback: [u8; 3]| -> [u8; 4] {
        match tx {
            Some(t) => {
                let x = ((u.clamp(0.0, 0.999)) * t.width as f32) as usize;
                let y = ((v.clamp(0.0, 0.999)) * t.height as f32) as usize;
                let i = (y * t.width as usize + x) * 4;
                [t.rgba[i], t.rgba[i + 1], t.rgba[i + 2], t.rgba[i + 3]]
            }
            None => [fallback[0], fallback[1], fallback[2], 255],
        }
    };
    // Solve p = o + u·a + v·b for (u, v).
    let solve = |p: (f32, f32), o: (f32, f32), a: (f32, f32), bb: (f32, f32)| {
        let (px, py) = (p.0 - o.0, p.1 - o.1);
        let det = a.0 * bb.1 - a.1 * bb.0;
        ((px * bb.1 - py * bb.0) / det, (a.0 * py - a.1 * px) / det)
    };
    for y in 0..size {
        for x in 0..size {
            let p = (x as f32 + 0.5, y as f32 + 0.5);
            let inside = |u: f32, v: f32| (0.0..=1.0).contains(&u) && (0.0..=1.0).contains(&v);
            let (u, v) = solve(p, t, (r.0 - t.0, r.1 - t.1), (l.0 - t.0, l.1 - t.1));
            let (c, shade) = if inside(u, v) {
                (sample(&top, u, v, block.color), 1.0)
            } else {
                let (u, v) = solve(p, l, (b.0 - l.0, b.1 - l.1), (0.0, s / 2.0));
                if inside(u, v) {
                    (sample(&side, u, v, block.side_color), 0.78)
                } else {
                    let (u, v) = solve(p, b, (r.0 - b.0, r.1 - b.1), (0.0, s / 2.0));
                    if inside(u, v) {
                        (sample(&side, u, v, block.side_color), 0.6)
                    } else {
                        continue;
                    }
                }
            };
            let i = (y * size + x) * 4;
            for k in 0..3 {
                out[i + k] = (c[k] as f32 * shade) as u8;
            }
            out[i + 3] = c[3];
        }
    }
    out
}

/// A flat picture of a block that is not a cube (flowers, torches…): its
/// side texture, as the game shows it in the inventory.
pub fn flat_icon(cat: &Catalogue, block: &Block, size: usize) -> Vec<u8> {
    let tex = block
        .side
        .as_ref()
        .or(block.top.as_ref())
        .and_then(|k| cat.texture(k))
        .map(|t| t.tinted(block.side_tint));
    let mut out = vec![0u8; size * size * 4];
    if let Some(t) = tex {
        for y in 0..size {
            for x in 0..size {
                let sx = x * t.width as usize / size;
                let sy = y * t.height as usize / size;
                let si = (sy * t.width as usize + sx) * 4;
                out[(y * size + x) * 4..][..4].copy_from_slice(&t.rgba[si..si + 4]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;


    /// The games installed here say which data version they write (only
    /// checked where a game is installed).
    #[test]
    fn installed_games_tell_their_data_version() {
        for jar in find_jars() {
            let version = data_version(&jar.path);
            eprintln!("{}: data version {version:?}", jar.version);
            if release_numbers(&jar.version).is_some_and(|n| n >= vec![1, 14]) {
                assert!(version.is_some_and(|v| v >= 1952), "{}: {version:?}", jar.version);
            }
        }
    }

    #[test]
    fn versions_order_like_the_game() {
        let mut v = ["1.20.4", "26.3", "1.21.11", "1.21.5", "26.1.2"].map(|s| release_numbers(s).unwrap());
        v.sort();
        assert_eq!(v.last().unwrap(), &vec![26, 3]);
        assert_eq!(v[0], vec![1, 20, 4]);
        assert!(release_numbers("25w16a").is_none());
        assert!(release_numbers("neoforge-21.5.38-beta").is_none());
        assert_eq!(display_name("minecraft:grass_block"), "Grass Block");
    }

    #[test]
    fn categories() {
        assert_eq!(Category::of("minecraft:grass_block"), Category::Natural);
        assert_eq!(Category::of("minecraft:stone"), Category::Stone);
        assert_eq!(Category::of("minecraft:stone_bricks"), Category::Building);
        assert_eq!(Category::of("minecraft:diamond_ore"), Category::Ores);
        assert_eq!(Category::of("minecraft:oak_planks"), Category::Wood);
        assert_eq!(Category::of("minecraft:oak_leaves"), Category::Plants);
        assert_eq!(Category::of("minecraft:red_wool"), Category::Colored);
        assert_eq!(Category::of("minecraft:netherrack"), Category::Nether);
        assert_eq!(Category::of("minecraft:end_stone"), Category::End);
    }

    /// A tiny jar: a block state, its model chain and a texture.
    #[test]
    fn blocks_resolve_through_their_models() {
        let mut png = Vec::new();
        let img = image::RgbaImage::from_pixel(16, 16, image::Rgba([200, 200, 200, 255]));
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let files: Vec<(&str, &[u8], bool)> = vec![
            ("assets/minecraft/blockstates/grass_block.json", br#"{"variants":{"snowy=false":[{"model":"minecraft:block/grass_block"}]}}"#, true),
            ("assets/minecraft/models/block/block.json", br#"{"textures":{}}"#, true),
            (
                "assets/minecraft/models/block/grass_block.json",
                br##"{"parent":"block/block","textures":{"particle":"block/dirt","top":"block/grass_block_top","side":"block/dirt"},
                "elements":[{"from":[0,0,0],"to":[16,16,16],"faces":{"up":{"texture":"#top","tintindex":0},"north":{"texture":"#side"},"down":{"texture":"#side"}}}]}"##,
                true,
            ),
            ("assets/minecraft/textures/block/grass_block_top.png", &png, false),
        ];
        let zip = zip::tests_support::tiny_zip(&files);
        let dir = std::env::temp_dir().join(format!("massif-assets-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.jar");
        std::fs::write(&path, zip).unwrap();
        let cat = load(&path, "1.21").unwrap();
        let grass = cat.block("minecraft:grass_block[snowy=false]").unwrap();
        assert_eq!(grass.name, "Grass Block");
        assert!(grass.full_cube);
        assert_eq!(grass.top.as_deref(), Some("block/grass_block_top"));
        assert_eq!(grass.side.as_deref(), Some("block/dirt"));
        assert_eq!(grass.top_tint, Tint::Grass);
        // Grey tinted by the plains grass colour.
        assert_eq!(grass.color, [0x91 * 200 / 255, 0xbd * 200 / 255, 0x59 * 200 / 255].map(|v| v as u8));
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The jars of this computer, when there are any (not in CI).
    #[test]
    fn real_jars_load() {
        let Some(result) = load_newest() else { return };
        let cat = result.unwrap();
        eprintln!("{} blocks, {} textures from {}", cat.blocks.len(), cat.textures.len(), cat.source.display());
        assert!(cat.blocks.len() > 800);
        let grass = cat.block("minecraft:grass_block").unwrap();
        assert!(grass.full_cube && grass.top_tint == Tint::Grass);
        assert!(grass.color[1] > grass.color[0] && grass.color[1] > grass.color[2], "grass is green: {:?}", grass.color);
        let stone = cat.block("minecraft:stone").unwrap();
        assert!(stone.full_cube);
        let log = cat.block("minecraft:oak_log").unwrap();
        assert_ne!(log.top, log.side);
        let flower = cat.block("minecraft:poppy").unwrap();
        assert!(!flower.full_cube);

        // Real colours in the registry, and a terrain of any block.
        let mut reg = Registry::vanilla();
        apply_colors(&mut reg, &cat);
        let grass_terrain = reg.terrain_by_name("Grass").unwrap();
        let t = reg.terrain(grass_terrain).color;
        assert!(t[1] > t[0] && t[1] > t[2], "the grass terrain is green: {t:?}");
        let bricks = cat.block("minecraft:stone_bricks").unwrap();
        let id = block_terrain(&mut reg, bricks);
        assert_eq!(block_terrain(&mut reg, bricks), id, "made once");
        assert_eq!(reg.terrain(id).name, "Stone Bricks");
        assert_eq!(terrain_block(&reg, &cat, id).unwrap().id, "minecraft:stone_bricks");
        let icon = cube_icon(&cat, grass, 32);
        assert_eq!(icon.len(), 32 * 32 * 4);
        assert_eq!(icon[3], 0, "transparent corner");
        assert_eq!(icon[(16 * 32 + 16) * 4 + 3], 255, "opaque middle");
    }
}
