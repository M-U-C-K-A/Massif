//! The map's blocks in the colours the game will give them (the True Colour
//! view of the 2D map and of the 3D views, and the Diorama): each block's
//! top texture, untinted, times the tint of its biome (grass, leaves,
//! water), as the game and the resource packs read (see
//! `massif_assets::true_color`).
//!
//! [`TrueColors`] is a table by the registry's numbers, made once for a
//! registry and the blocks read, and cheap for each column: the views ask
//! [`current`] for it, which makes it again when the registry's blocks,
//! biomes or plants change, or other blocks or packs are read.

use massif_assets::Catalogue;
use massif_assets::true_color::{Climate, Kind, Rules, kind_of};
use massif_core::registry::{BlockId, Registry};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex, RwLock};

pub type Rgb = [f32; 3];

/// A block's top: its texture's colour before any tint, and how the game
/// tints it.
#[derive(Debug, Clone)]
struct Top {
    id: String,
    base: Rgb,
    kind: Option<Kind>,
    /// A pack's map for this very block, or one by height: asked each time.
    custom: bool,
    /// Its top's connected texture, if it has one (see [`Joined`]).
    joined: Option<Arc<Joined>>,
}

/// A connected texture's colours, untinted: what a top shows by its
/// neighbours, without drawing its tiles.
#[derive(Debug)]
pub struct Joined {
    texture: Arc<massif_assets::connected::Connected>,
    /// For compact tiles: the mean of each quarter of each tile.
    quarters: Option<[[Rgb; 4]; 5]>,
    /// For the others: each tile and its mean.
    tiles: Vec<(Arc<massif_assets::Texture>, Rgb)>,
}

fn mean_of(t: &massif_assets::Texture, x0: u32, y0: u32, w: u32, h: u32) -> Rgb {
    let (mut sum, mut n) = ([0.0f32; 3], 0.0f32);
    for y in y0..(y0 + h).min(t.height) {
        for x in x0..(x0 + w).min(t.width) {
            let p = &t.rgba[((y * t.width + x) * 4) as usize..][..4];
            if p[3] > 16 {
                for k in 0..3 {
                    sum[k] += p[k] as f32;
                }
                n += 1.0;
            }
        }
    }
    sum.map(|v| v / n.max(1.0))
}

impl Joined {
    fn new(texture: Arc<massif_assets::connected::Connected>) -> Joined {
        use massif_assets::connected::Kind;
        let whole = |t: &massif_assets::Texture| mean_of(t, 0, 0, t.width, t.height);
        let (quarters, tiles) = match &texture.kind {
            Kind::Compact(t) => (
                Some(std::array::from_fn(|k| {
                    let (w, h) = (t[k].width / 2, t[k].height / 2);
                    [mean_of(&t[k], 0, 0, w, h), mean_of(&t[k], w, 0, w, h), mean_of(&t[k], 0, h, w, h), mean_of(&t[k], w, h, w, h)]
                })),
                Vec::new(),
            ),
            Kind::Horizontal(t) | Kind::Vertical(t) => (None, t.iter().map(|x| (x.clone(), whole(x))).collect()),
            Kind::Repeat { tiles, .. } => (None, tiles.iter().map(|x| (x.clone(), whole(x))).collect()),
            Kind::Random(t) => (None, t.iter().map(|x| (x.0.clone(), whole(&x.0))).collect()),
        };
        Joined { texture, quarters, tiles }
    }

    /// The mean colour of a top face with `around` it at `(x, y, z)`.
    fn mean(&self, around: massif_assets::connected::Around, x: i32, y: i32, z: i32) -> Rgb {
        if let Some(q) = &self.quarters {
            let k = |side: bool, vertical: bool, corner: bool| match (side, vertical) {
                (false, false) => 0,
                (true, false) => 1,
                (false, true) => 2,
                (true, true) if corner => 3,
                (true, true) => 4,
            };
            let parts = [
                q[k(around.left, around.up, around.up_left)][0],
                q[k(around.right, around.up, around.up_right)][1],
                q[k(around.left, around.down, around.down_left)][2],
                q[k(around.right, around.down, around.down_right)][3],
            ];
            return std::array::from_fn(|c| parts.iter().map(|p| p[c]).sum::<f32>() / 4.0);
        }
        let tile = self.texture.face(around, x, z, (x, y, z));
        self.tiles.iter().find(|t| Arc::ptr_eq(&t.0, &tile)).map_or_else(|| mean_of(&tile, 0, 0, tile.width, tile.height), |t| t.1)
    }
}

/// The tints of a biome at the sea's height, by kind.
#[derive(Debug, Clone, Copy)]
struct Tints {
    grass: Rgb,
    foliage: Rgb,
    dry: Rgb,
    water: Rgb,
    spruce: Rgb,
    birch: Rgb,
}

pub struct TrueColors {
    rules: Arc<Rules>,
    blocks: Vec<Top>,
    biomes: Vec<Climate>,
    tints: Vec<Tints>,
    /// The plants of covers and the blocks of objects, by state.
    states: HashMap<String, Top>,
    water: Rgb,
    ice: Rgb,
    lava: Rgb,
    /// The packs whose colours were read, for the view's legend.
    pub packs: Vec<String>,
}

fn f(c: [u8; 3]) -> Rgb {
    c.map(|v| v as f32)
}

/// The game's plains tints, which the registry's colours carry when no
/// texture was read.
const PLAINS_GRASS: [u8; 3] = [0x91, 0xbd, 0x59];
const PLAINS_FOLIAGE: [u8; 3] = [0x77, 0xab, 0x2f];

fn top_of(cat: Option<&Catalogue>, rules: &Rules, state: &str, color: [u8; 3]) -> Top {
    let id = state.split('[').next().unwrap_or(state);
    let id = if id.contains(':') { id.to_owned() } else { format!("minecraft:{id}") };
    // Snowy grass, podzol and mycelium show the snow on top.
    let shown = if state.contains("snowy=true") { "minecraft:snow_block" } else { id.as_str() };
    let (base, kind) = match cat.and_then(|c| c.true_top(shown)) {
        Some((mean, kind)) => (f(mean), kind),
        None => {
            // No texture read: the registry's colour, which is the plains'
            // tint of a tinted block; take that tint off.
            let n = shown.rsplit(':').next().unwrap_or(shown);
            let tinted = n == "grass_block" || n.contains("leaves") || n == "vine" || n == "short_grass" || n == "grass" || n == "tall_grass" || n.contains("fern");
            let kind = kind_of(shown, tinted);
            let plains = match kind {
                Some(Kind::Grass) => Some(PLAINS_GRASS),
                Some(Kind::Foliage) => Some(PLAINS_FOLIAGE),
                _ => None,
            };
            let base = match plains {
                Some(p) => std::array::from_fn(|k| (color[k] as f32 * 255.0 / p[k].max(1) as f32).min(255.0)),
                None => f(color),
            };
            (base, if plains.is_some() || matches!(kind, Some(Kind::Water)) { kind } else { None })
        }
    };
    let custom = rules.blocks.contains_key(shown) || rules.by_height(shown, kind);
    // A connected top (OptiFine's CTM, Fusion's): its tiles' colours by
    // its neighbours.
    let joined = cat
        .and_then(|c| c.block(shown).and_then(|b| b.top.as_ref()).and_then(|t| c.connected.get(t)))
        .filter(|j| j.faces & massif_assets::connected::UP != 0)
        .map(|j| Arc::new(Joined::new(j.clone())));
    Top { id: shown.to_owned(), base, kind, custom, joined }
}

impl TrueColors {
    pub fn new(reg: &Registry, cat: Option<&Catalogue>) -> TrueColors {
        let rules = cat.map(|c| c.colors.clone()).unwrap_or_default();
        let blocks = reg.blocks.iter().map(|b| top_of(cat, &rules, &b.state, b.color)).collect();
        let biomes: Vec<Climate> = reg
            .biomes
            .iter()
            .map(|b| match cat.and_then(|c| c.biome(&b.id)) {
                Some(info) => info.climate.clone(),
                None => Climate { id: b.id.clone(), grass: b.grass, foliage: b.foliage, ..Climate::plains() },
            })
            .collect();
        let tints = biomes
            .iter()
            .map(|c| Tints {
                grass: f(rules.grass(c, 64)),
                foliage: f(rules.foliage(c, 64)),
                dry: f(rules.dry_foliage(c)),
                water: f(rules.water(c, 62)),
                spruce: f(rules.tint("minecraft:spruce_leaves", Some(Kind::Spruce), c, 64).unwrap_or(massif_assets::true_color::SPRUCE)),
                birch: f(rules.tint("minecraft:birch_leaves", Some(Kind::Birch), c, 64).unwrap_or(massif_assets::true_color::BIRCH)),
            })
            .collect();
        let mut states = HashMap::new();
        let mut add = |s: &str, color: [u8; 3]| {
            if !states.contains_key(s) {
                states.insert(s.to_owned(), top_of(cat, &rules, s, color));
            }
        };
        for c in reg.covers.iter().filter(|c| !c.removed) {
            for (p, _) in &c.plants {
                add(p, massif_core::registry::plant_color(p).unwrap_or(c.color));
            }
        }
        for o in &reg.objects {
            for (k, s) in o.palette.iter().enumerate() {
                add(s, o.color(k as u16));
            }
        }
        let plain = |id: &str, default: [u8; 3]| cat.and_then(|c| c.true_top(id)).map_or(f(default), |t| f(t.0));
        TrueColors {
            packs: rules.packs.clone(),
            water: plain("minecraft:water", [0xb0, 0xb0, 0xb0]),
            ice: plain("minecraft:ice", [0x91, 0xb7, 0xfd]),
            lava: plain("minecraft:lava", [0xcf, 0x5b, 0x14]),
            rules,
            blocks,
            biomes,
            tints,
            states,
        }
    }

    fn climate(&self, biome: u16) -> &Climate {
        self.biomes.get(biome as usize).or(self.biomes.get(1)).or(self.biomes.first()).unwrap_or_else(|| PLAINS.get_or_init(Climate::plains))
    }

    fn tinted(&self, top: &Top, biome: u16, y: i32) -> Rgb {
        self.tinted_from(top, top.base, biome, y)
    }

    fn tinted_from(&self, top: &Top, base: Rgb, biome: u16, y: i32) -> Rgb {
        let Some(kind) = top.kind else { return base };
        let tint = if top.custom {
            f(self.rules.tint(&top.id, Some(kind), self.climate(biome), y).unwrap_or([255; 3]))
        } else {
            let t = self.tints.get(biome as usize).or(self.tints.get(1)).copied();
            match (kind, t) {
                (Kind::Grass, Some(t)) => t.grass,
                (Kind::Foliage, Some(t)) => t.foliage,
                (Kind::DryFoliage, Some(t)) => t.dry,
                (Kind::Water, Some(t)) => t.water,
                (Kind::Spruce, Some(t)) => t.spruce,
                (Kind::Birch, Some(t)) => t.birch,
                (Kind::Fixed(c), _) => f(c),
                (_, None) => f(self.rules.tint(&top.id, Some(kind), self.climate(biome), y).unwrap_or([255; 3])),
            }
        };
        std::array::from_fn(|k| base[k] * tint[k] / 255.0)
    }

    /// Whether a block's top joins its neighbours' (a connected texture
    /// that depends on them, or on its place).
    pub fn joins(&self, block: BlockId) -> bool {
        self.blocks.get(block.0 as usize).is_some_and(|t| t.joined.is_some())
    }

    /// [`TrueColors::block`] for a block whose top is a connected texture,
    /// at `(x, y, z)` with `around` it: the tiles it shows there.
    pub fn block_at(&self, block: BlockId, biome: u16, around: massif_assets::connected::Around, x: i32, y: i32, z: i32) -> Rgb {
        let Some(top) = self.blocks.get(block.0 as usize) else { return [118.0; 3] };
        match &top.joined {
            Some(j) => self.tinted_from(top, j.mean(around, x, y, z), biome, y),
            None => self.tinted(top, biome, y),
        }
    }

    /// The colour of a block's top in a biome (the registry's number) at a
    /// height.
    pub fn block(&self, block: BlockId, biome: u16, y: i32) -> Rgb {
        self.blocks.get(block.0 as usize).map_or([118.0; 3], |t| self.tinted(t, biome, y))
    }

    /// The same as bytes.
    pub fn block_u8(&self, block: BlockId, biome: u16, y: i32) -> [u8; 3] {
        self.block(block, biome, y).map(|v| v.round().clamp(0.0, 255.0) as u8)
    }

    /// The colour of a block given by its state (a cover's plant, an
    /// object's block).
    pub fn state(&self, state: &str, biome: u16, y: i32) -> Option<Rgb> {
        self.states.get(state).map(|t| self.tinted(t, biome, y))
    }

    /// The colour of a terrain's top in a biome: its surface's blocks by
    /// their weights (for texels of many blocks).
    pub fn terrain(&self, reg: &Registry, terrain: u16, biome: u16, y: i32) -> Rgb {
        let Some(s) = reg.terrains.get(terrain as usize).and_then(|t| t.strata.first()) else { return [118.0; 3] };
        let total: f32 = s.blocks.iter().map(|&(_, w)| w as f32).sum();
        if total <= 0.0 {
            return [118.0; 3];
        }
        let mut out = [0.0; 3];
        for &(b, w) in &s.blocks {
            let c = self.block(b, biome, y);
            for k in 0..3 {
                out[k] += c[k] * w as f32 / total;
            }
        }
        out
    }

    /// Water seen from above in a biome.
    pub fn water(&self, biome: u16, y: i32) -> Rgb {
        let t = match self.tints.get(biome as usize) {
            Some(t) if !self.rules.by_height("minecraft:water", Some(Kind::Water)) => t.water,
            _ => f(self.rules.water(self.climate(biome), y)),
        };
        std::array::from_fn(|k| self.water[k] * t[k] / 255.0)
    }

    pub fn ice(&self) -> Rgb {
        self.ice
    }

    pub fn lava(&self) -> Rgb {
        self.lava
    }
}

static PLAINS: std::sync::OnceLock<Climate> = std::sync::OnceLock::new();

/// The blocks read, as the app last noted them (see `blocks::note_catalogue`).
static CATALOGUE: RwLock<Option<Arc<Catalogue>>> = RwLock::new(None);
/// The last table made, and what it was made for.
static CURRENT: Mutex<Option<(u64, Arc<TrueColors>)>> = Mutex::new(None);

/// Note the blocks read: the colours of the views follow them.
pub fn note_catalogue(cat: Option<&Arc<Catalogue>>) {
    if let Ok(mut c) = CATALOGUE.write() {
        let same = match (c.as_ref(), cat) {
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            (None, None) => true,
            _ => false,
        };
        if !same {
            *c = cat.cloned();
        }
    }
}

/// Whether the game's blocks were read: else the colours are the
/// registry's guesses.
pub fn has_catalogue() -> bool {
    CATALOGUE.read().is_ok_and(|c| c.is_some())
}

/// What the colours of a registry's blocks depend on: its blocks, biomes,
/// terrains' surfaces, plants and objects' blocks, and the blocks read.
fn key_of(reg: &Registry, cat: Option<&Arc<Catalogue>>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cat.map(|c| Arc::as_ptr(c) as usize).hash(&mut h);
    for b in &reg.blocks {
        b.state.hash(&mut h);
        b.color.hash(&mut h);
    }
    for b in &reg.biomes {
        b.id.hash(&mut h);
        b.grass.hash(&mut h);
        b.foliage.hash(&mut h);
    }
    for c in &reg.covers {
        c.removed.hash(&mut h);
        for (p, _) in &c.plants {
            p.hash(&mut h);
        }
    }
    reg.objects.len().hash(&mut h);
    for o in &reg.objects {
        o.palette.len().hash(&mut h);
        o.palette.first().hash(&mut h);
        o.palette.last().hash(&mut h);
    }
    h.finish()
}

/// The table for a registry with the blocks read, made again only when
/// what it depends on changed.
pub fn current(reg: &Registry) -> Arc<TrueColors> {
    let cat = CATALOGUE.read().ok().and_then(|c| c.clone());
    let key = key_of(reg, cat.as_ref());
    let mut cur = CURRENT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((k, t)) = cur.as_ref()
        && *k == key
    {
        return t.clone();
    }
    let table = Arc::new(TrueColors::new(reg, cat.as_deref()));
    *cur = Some((key, table.clone()));
    table
}

/// The key of the table [`current`] would give, without making it.
pub fn key(reg: &Registry) -> u32 {
    let cat = CATALOGUE.read().ok().and_then(|c| c.clone());
    (key_of(reg, cat.as_ref()) as u32) | 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn without_textures_grass_takes_its_biomes_tint() {
        let mut reg = Registry::vanilla();
        let grass = reg.blocks.iter().position(|b| b.name() == "minecraft:grass_block").map(|i| BlockId(i as u16)).unwrap();
        reg.blocks[grass.0 as usize].color = PLAINS_GRASS;
        let red = reg.biomes.len() as u16;
        reg.biomes.push(massif_core::registry::BiomeDef { id: "test:red".into(), name: "Red".into(), grass: Some([255, 0, 0]), foliage: Some([255, 0, 0]), ..Default::default() });
        let t = TrueColors::new(&reg, None);
        // White base: the tint itself.
        let c = t.block_u8(grass, red, 64);
        assert_eq!(c, [255, 0, 0]);
    }
}
