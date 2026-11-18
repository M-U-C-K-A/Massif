//! The game's structures, one click each: the jigsaw ones assembled from
//! their own files ([`crate::jigsaw`]), the others as the game's code
//! assembles them from their templates (an igloo, a shipwreck, ocean
//! ruins) or builds them block by block ([`crate::code`]).

use crate::build::{self, Builder};
use crate::geom::BBox;
use crate::ground::{Ground, WorldBlock};
use crate::jigsaw::{self, Spec};
use crate::library::Library;
use crate::nbt;
use crate::processor::{Heightmap, Processor, Test};
use crate::rng::Rng;
use crate::source::Kind as FileKind;
use massif_core::structures::{Piece, Structure};
use std::collections::HashMap;
use std::sync::Arc;

/// Where a structure is offered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Villages,
    Surface,
    Water,
    Underground,
    Nether,
}

/// Where a structure goes up and down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Height {
    /// On the land where it is placed.
    Surface,
    /// On the bottom of the water (or the land, out of it).
    Floor,
    /// At a height of its own: by default the height the game gives it.
    Depth(i32),
}

/// How a structure is put together.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Recipe {
    /// A jigsaw structure, by the name of its `worldgen/structure` file.
    Jigsaw(&'static str),
    Igloo,
    Shipwreck,
    OceanRuin { warm: bool },
    DesertPyramid,
    JunglePyramid,
    SwampHut,
    Stronghold,
}

/// A structure the Structure tool offers.
#[derive(Debug, Clone, Copy)]
pub struct Kind {
    /// Its key (`ancient_city`), kept with the tool's settings.
    pub id: &'static str,
    /// Its name, in English: the app translates it.
    pub name: &'static str,
    /// The game's own name of it, to tell the game where it stands.
    pub game_id: &'static str,
    pub group: Group,
    pub height: Height,
    pub recipe: Recipe,
    /// Whether the land under its buildings is flattened, by default.
    pub flattens: bool,
    /// The biome it brings with it, if any.
    pub biome: Option<&'static str>,
}

pub const KINDS: &[Kind] = &[
    Kind { id: "village_plains", name: "Plains village", game_id: "minecraft:village_plains", group: Group::Villages, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:village_plains"), flattens: true, biome: None },
    Kind { id: "village_desert", name: "Desert village", game_id: "minecraft:village_desert", group: Group::Villages, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:village_desert"), flattens: true, biome: None },
    Kind { id: "village_savanna", name: "Savanna village", game_id: "minecraft:village_savanna", group: Group::Villages, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:village_savanna"), flattens: true, biome: None },
    Kind { id: "village_taiga", name: "Taiga village", game_id: "minecraft:village_taiga", group: Group::Villages, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:village_taiga"), flattens: true, biome: None },
    Kind { id: "village_snowy", name: "Snowy village", game_id: "minecraft:village_snowy", group: Group::Villages, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:village_snowy"), flattens: true, biome: None },
    Kind { id: "pillager_outpost", name: "Pillager outpost", game_id: "minecraft:pillager_outpost", group: Group::Surface, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:pillager_outpost"), flattens: true, biome: None },
    Kind { id: "desert_pyramid", name: "Desert temple", game_id: "minecraft:desert_pyramid", group: Group::Surface, height: Height::Surface, recipe: Recipe::DesertPyramid, flattens: true, biome: None },
    Kind { id: "jungle_pyramid", name: "Jungle temple", game_id: "minecraft:jungle_pyramid", group: Group::Surface, height: Height::Surface, recipe: Recipe::JunglePyramid, flattens: true, biome: None },
    Kind { id: "swamp_hut", name: "Witch hut", game_id: "minecraft:swamp_hut", group: Group::Surface, height: Height::Surface, recipe: Recipe::SwampHut, flattens: false, biome: None },
    Kind { id: "igloo", name: "Igloo", game_id: "minecraft:igloo", group: Group::Surface, height: Height::Surface, recipe: Recipe::Igloo, flattens: true, biome: None },
    Kind { id: "shipwreck", name: "Shipwreck", game_id: "minecraft:shipwreck", group: Group::Water, height: Height::Floor, recipe: Recipe::Shipwreck, flattens: false, biome: None },
    Kind { id: "ocean_ruin_cold", name: "Ocean ruins", game_id: "minecraft:ocean_ruin_cold", group: Group::Water, height: Height::Floor, recipe: Recipe::OceanRuin { warm: false }, flattens: false, biome: None },
    Kind { id: "ocean_ruin_warm", name: "Warm ocean ruins", game_id: "minecraft:ocean_ruin_warm", group: Group::Water, height: Height::Floor, recipe: Recipe::OceanRuin { warm: true }, flattens: false, biome: None },
    Kind { id: "ancient_city", name: "Ancient City", game_id: "minecraft:ancient_city", group: Group::Underground, height: Height::Depth(-27), recipe: Recipe::Jigsaw("minecraft:ancient_city"), flattens: false, biome: Some("minecraft:deep_dark") },
    Kind { id: "trial_chambers", name: "Trial Chambers", game_id: "minecraft:trial_chambers", group: Group::Underground, height: Height::Depth(-30), recipe: Recipe::Jigsaw("minecraft:trial_chambers"), flattens: false, biome: None },
    Kind { id: "stronghold", name: "Stronghold", game_id: "minecraft:stronghold", group: Group::Underground, height: Height::Depth(10), recipe: Recipe::Stronghold, flattens: false, biome: None },
    Kind { id: "trail_ruins", name: "Trail ruins", game_id: "minecraft:trail_ruins", group: Group::Underground, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:trail_ruins"), flattens: false, biome: None },
    Kind { id: "bastion_remnant", name: "Bastion remnant", game_id: "minecraft:bastion_remnant", group: Group::Nether, height: Height::Surface, recipe: Recipe::Jigsaw("minecraft:bastion_remnant"), flattens: false, biome: None },
];

pub fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|k| k.id == id)
}

/// What to place.
#[derive(Debug, Clone)]
pub struct Request {
    pub kind: &'static Kind,
    /// Where: the column clicked, and for [`Height::Depth`] the height.
    pub at: [i32; 3],
    /// Quarter turns clockwise; `None`: drawn.
    pub turn: Option<u8>,
    pub seed: u64,
    /// Flatten the land under its buildings.
    pub flatten: bool,
    /// The number it takes in the map.
    pub id: u32,
}

/// A structure ready to place: its blocks, and the heights to give the
/// columns flattened under it.
#[derive(Debug, Clone)]
pub struct Built {
    pub structure: Structure,
    pub heights: HashMap<(i32, i32), i32>,
    /// What Massif met and does not know.
    pub unknown: Vec<String>,
}

/// The land, some of its columns given another height (flattened under a
/// structure).
pub struct Adapted<'a> {
    pub inner: &'a dyn Ground,
    pub heights: HashMap<(i32, i32), i32>,
}

impl Ground for Adapted<'_> {
    fn min_y(&self) -> i32 {
        self.inner.min_y()
    }

    fn max_y(&self) -> i32 {
        self.inner.max_y()
    }

    fn surface(&self, x: i32, z: i32) -> i32 {
        self.heights.get(&(x, z)).copied().unwrap_or_else(|| self.inner.surface(x, z))
    }

    fn floor(&self, x: i32, z: i32) -> i32 {
        self.heights.get(&(x, z)).copied().unwrap_or_else(|| self.inner.floor(x, z))
    }

    fn is_air(&self, x: i32, y: i32, z: i32) -> bool {
        match self.heights.get(&(x, z)) {
            Some(&h) => y >= h,
            None => self.inner.is_air(x, y, z),
        }
    }

    fn block(&self, x: i32, y: i32, z: i32) -> WorldBlock {
        match self.heights.get(&(x, z)) {
            Some(&h) if y >= h => WorldBlock::Air,
            Some(_) => {
                // The ground's own block, from as high as the ground was.
                let top = self.inner.floor(x, z);
                match self.inner.block(x, y.min(top - 1), z) {
                    WorldBlock::Solid(s) => WorldBlock::Solid(s),
                    _ => WorldBlock::Solid("minecraft:dirt".into()),
                }
            }
            None => self.inner.block(x, y, z),
        }
    }
}

/// How the game fits a structure into the land around it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Adaptation {
    None,
    /// The land under each building raised or cut to its floor (villages).
    BeardThin,
    /// The box of each piece carved out above its floor (an Ancient City's
    /// halls in the deep dark).
    BeardBox,
    /// Buried in the land (trail ruins).
    Bury,
    /// Walled in by rock all around (Trial Chambers).
    Encapsulate,
}

impl Adaptation {
    fn parse(name: Option<&str>) -> Self {
        match name {
            Some("beard_thin") => Adaptation::BeardThin,
            Some("beard_box") => Adaptation::BeardBox,
            Some("bury") => Adaptation::Bury,
            Some("encapsulate") => Adaptation::Encapsulate,
            _ => Adaptation::None,
        }
    }
}

/// How the game assembles jigsaw structure `id`, read from its file: the
/// spec, the start's height, whether its blocks hold water where set in
/// it, and how it fits into the land.
pub fn jigsaw_spec(lib: &Library, id: &str) -> Option<(Spec, i32, bool, Adaptation)> {
    let json = lib.json(FileKind::Structure, id)?;
    let start_pool = json.get("start_pool")?.as_str()?.to_owned();
    let int = |v: Option<&serde_json::Value>| v.and_then(|v| v.as_i64()).map(|v| v as i32);
    let height = json.get("start_height").map_or(0, |h| match int(h.get("absolute")) {
        Some(y) => y,
        None => {
            let lo = h.get("min_inclusive").and_then(|m| int(m.get("absolute"))).unwrap_or(0);
            let hi = h.get("max_inclusive").and_then(|m| int(m.get("absolute"))).unwrap_or(lo);
            (lo + hi) / 2
        }
    });
    let max_distance = match json.get("max_distance_from_center") {
        Some(serde_json::Value::Object(o)) => o.get("horizontal").and_then(|v| v.as_i64()).unwrap_or(80) as i32,
        other => int(other).unwrap_or(80),
    };
    let padding = match json.get("dimension_padding") {
        Some(serde_json::Value::Object(o)) => (o.get("bottom").and_then(|v| v.as_i64()).unwrap_or(0) as i32, o.get("top").and_then(|v| v.as_i64()).unwrap_or(0) as i32),
        other => {
            let p = int(other).unwrap_or(0);
            (p, p)
        }
    };
    let waterlog = json.get("liquid_settings").and_then(|l| l.as_str()) != Some("ignore_waterlogging");
    let spec = Spec {
        start_pool,
        start_jigsaw: json.get("start_jigsaw_name").and_then(|n| n.as_str()).map(str::to_owned),
        depth: int(json.get("size")).unwrap_or(1).clamp(0, 20) as u32,
        max_distance,
        expansion_hack: json.get("use_expansion_hack").and_then(|v| v.as_bool()).unwrap_or(false),
        project: json.get("project_start_to_heightmap").and_then(|h| h.as_str()).map(Heightmap::parse),
        padding,
        aliases: jigsaw::parse_aliases(json.get("pool_aliases")),
    };
    Some((spec, height, waterlog, Adaptation::parse(json.get("terrain_adaptation").and_then(|a| a.as_str()))))
}

/// Build the structure `req` asks for, on `ground`.
pub fn build(lib: &Library, ground: &dyn Ground, req: &Request) -> Result<Built, String> {
    let mut rng = Rng::new(req.seed);
    let kind = req.kind;
    let (structure, heights) = match kind.recipe {
        Recipe::Jigsaw(id) => {
            let (spec, game_height, waterlog, adaptation) = jigsaw_spec(lib, id).ok_or_else(|| format!("the game's files have no {id}: this version of the game may not have it"))?;
            let start = match (kind.height, spec.project) {
                // Measured from the land by the game: its own offset.
                (_, Some(_)) => [req.at[0], game_height, req.at[2]],
                (Height::Depth(_), None) => req.at,
                (_, None) => [req.at[0], ground.surface(req.at[0], req.at[2]) + game_height.min(0), req.at[2]],
            };
            let pieces = jigsaw::assemble(lib, &spec, start, req.turn, &mut rng, ground);
            if pieces.is_empty() {
                return Err(format!("{} could not be assembled from the game's files", kind.name));
            }
            let heights = if req.flatten { build::flatten_under(&pieces, ground, 4) } else { HashMap::new() };
            let adapted = Adapted { inner: ground, heights };
            let mut b = Builder::new(&adapted, req.seed);
            let center = build::center_of(&pieces);
            match adaptation {
                Adaptation::BeardBox => build::carve_boxes(&mut b, &pieces),
                Adaptation::Encapsulate => build::encase(&mut b, &pieces),
                _ => {}
            }
            for p in &pieces {
                build::place_piece(lib, &mut b, p, center);
            }
            let mut s = b.finish(req.id, kind.name, kind.game_id, waterlog, 0);
            s.biome = kind.biome.map(str::to_owned);
            (s, adapted.heights)
        }
        Recipe::Igloo => igloo(lib, ground, req, &mut rng)?,
        Recipe::Shipwreck => shipwreck(lib, ground, req, &mut rng)?,
        Recipe::OceanRuin { warm } => ocean_ruin(lib, ground, req, warm, &mut rng)?,
        Recipe::DesertPyramid | Recipe::JunglePyramid | Recipe::SwampHut | Recipe::Stronghold => crate::code::build(ground, req, &mut rng)?,
    };
    let mut structure = structure;
    structure.data_version = lib.data_version();
    if structure.sections.is_empty() {
        return Err(format!("{} has no blocks", kind.name));
    }
    Ok(Built { structure, heights, unknown: lib.unknowns() })
}

/// A template placed as the game's code places them: turned about `pivot`
/// (a point of the template), its corner at `origin`.
#[allow(clippy::too_many_arguments)]
fn code_piece(lib: &Library, b: &mut Builder, location: &str, origin: [i32; 3], turn: u8, pivot: [i32; 3], list: &[Processor], center: [i32; 3]) -> Option<BBox> {
    // Turning about the pivot is turning about the corner, moved.
    let (px, pz) = crate::geom::turn_xz(pivot[0], pivot[2], turn);
    let corner = [origin[0] + pivot[0] - px, origin[1], origin[2] + pivot[2] - pz];
    let bbox = build::place_template(lib, b, location, corner, turn, list, center, false)?;
    b.pieces.push(Piece { min: bbox.min, max: bbox.max, template: crate::source::full_id(location), origin: corner, turn, depth: 0, ground_delta: 0, rigid: true, processors: String::new() });
    Some(bbox)
}

/// Data markers that name a loot chest one block below them.
fn loot_below(b: &mut Builder, tables: &[(&str, &str)], rng: &mut Rng) {
    let markers = std::mem::take(&mut b.markers);
    for m in &markers {
        let Some(&(_, table)) = tables.iter().find(|(name, _)| m.metadata == *name) else { continue };
        let below = [m.pos[0], m.pos[1] - 1, m.pos[2]];
        let state = b.get(below).map(str::to_owned);
        if let Some(state) = state.filter(|s| nbt::block_name(s).ends_with("chest") || nbt::block_name(s).ends_with("barrel")) {
            let id = nbt::block_name(&state).to_owned();
            b.set(below, &state, Some(build::loot(&id, table, rng)));
        }
    }
}

fn igloo(lib: &Library, ground: &dyn Ground, req: &Request, rng: &mut Rng) -> Result<(Structure, HashMap<(i32, i32), i32>), String> {
    let turn = req.turn.unwrap_or_else(|| rng.below(4) as u8);
    let (x, z) = (req.at[0], req.at[2]);
    let surface = ground.surface(x, z);
    // The pivot of the dome, (3, 5) of its template, on the column clicked.
    let pos = [x - 3, surface - 1, z - 5];
    let ignore = [Processor::Ignore(vec!["minecraft:structure_block".into()])];
    let mut heights = HashMap::new();
    if req.flatten {
        for dz in -1..=11 {
            for dx in -1..=7 {
                let (cx, cz) = (pos[0] + dx, pos[2] + dz);
                if ground.floor(cx, cz) != surface {
                    heights.insert((cx, cz), surface);
                }
            }
        }
    }
    let adapted = Adapted { inner: ground, heights };
    let mut b = Builder::new(&adapted, req.seed);
    let center = [x, surface, z];
    if rng.double() < 0.5 {
        let depth = rng.below(8) as i32 + 4;
        code_piece(lib, &mut b, "minecraft:igloo/bottom", [pos[0], pos[1] - 3 - depth * 3, pos[2] - 2], turn, [3, 6, 7], &ignore, center);
        for j in 0..depth - 1 {
            code_piece(lib, &mut b, "minecraft:igloo/middle", [pos[0] + 2, pos[1] - 3 - j * 3, pos[2] + 4], turn, [1, 3, 1], &ignore, center);
        }
    }
    code_piece(lib, &mut b, "minecraft:igloo/top", pos, turn, [3, 5, 5], &ignore, center).ok_or("the game's files have no igloo")?;
    // No way down: snow over the trapdoor.
    let trapdoor = [pos[0] + 3, pos[1], pos[2] + 5];
    let below = [trapdoor[0], trapdoor[1] - 1, trapdoor[2]];
    if !b.is_air(below) && !b.block(below).starts_with("minecraft:ladder") {
        b.set(trapdoor, "minecraft:snow_block", None);
    }
    // The chest of the lab, under its marker.
    let markers = std::mem::take(&mut b.markers);
    for m in markers.iter().filter(|m| m.metadata == "chest") {
        b.set(m.pos, "minecraft:air", None);
        let below = [m.pos[0], m.pos[1] - 1, m.pos[2]];
        if let Some(state) = b.get(below).map(str::to_owned).filter(|s| s.starts_with("minecraft:chest")) {
            b.set(below, &state, Some(build::loot("minecraft:chest", "minecraft:chests/igloo_chest", rng)));
        }
    }
    let heights = adapted.heights.clone();
    Ok((b.finish(req.id, req.kind.name, req.kind.game_id, false, 0), heights))
}

const SHIPWRECKS: [&str; 20] = [
    "with_mast",
    "upsidedown_full",
    "upsidedown_fronthalf",
    "upsidedown_backhalf",
    "sideways_full",
    "sideways_fronthalf",
    "sideways_backhalf",
    "rightsideup_full",
    "rightsideup_fronthalf",
    "rightsideup_backhalf",
    "with_mast_degraded",
    "upsidedown_full_degraded",
    "upsidedown_fronthalf_degraded",
    "upsidedown_backhalf_degraded",
    "sideways_full_degraded",
    "sideways_fronthalf_degraded",
    "sideways_backhalf_degraded",
    "rightsideup_full_degraded",
    "rightsideup_fronthalf_degraded",
    "rightsideup_backhalf_degraded",
];

/// Wrecks that lie on a beach rather than under water: those the game
/// keeps for beaches (none upside down).
fn beached(name: &str) -> bool {
    !name.starts_with("upsidedown") && !(name.starts_with("sideways") && name.ends_with("degraded"))
}

fn shipwreck(lib: &Library, ground: &dyn Ground, req: &Request, rng: &mut Rng) -> Result<(Structure, HashMap<(i32, i32), i32>), String> {
    let turn = req.turn.unwrap_or_else(|| rng.below(4) as u8);
    let (x, z) = (req.at[0], req.at[2]);
    let dry = ground.surface(x, z) <= ground.floor(x, z);
    let names: Vec<&str> = SHIPWRECKS.iter().copied().filter(|n| !dry || beached(n)).collect();
    let name = format!("minecraft:shipwreck/{}", rng.pick(&names).ok_or("no shipwreck")?);
    let t = lib.template(&name).ok_or("the game's files have no shipwrecks")?;
    // Its height: on the sea bed, the mean of its footprint; on a beach,
    // sunk half its height into the lowest of it.
    let pivot = [4, 0, 15];
    let (px, pz) = crate::geom::turn_xz(pivot[0], pivot[2], turn);
    let probe = BBox::of_template(t.size, [x - px, 0, z - pz], turn);
    let (mut sum, mut n, mut low) = (0i64, 0i64, i32::MAX);
    for cz in probe.min[2]..=probe.max[2] {
        for cx in probe.min[0]..=probe.max[0] {
            let h = if dry { ground.surface(cx, cz) } else { ground.floor(cx, cz) };
            sum += h as i64;
            n += 1;
            low = low.min(h);
        }
    }
    let y = if dry { low - t.size[1] / 2 - rng.below(3) as i32 } else { (sum / n.max(1)) as i32 };
    let mut b = Builder::new(ground, req.seed);
    let ignore = [Processor::Ignore(vec!["minecraft:structure_block".into(), "minecraft:air".into()])];
    code_piece(lib, &mut b, &name, [x - pivot[0], y, z - pivot[2]], turn, pivot, &ignore, [x, y, z]);
    loot_below(&mut b, &[("map_chest", "minecraft:chests/shipwreck_map"), ("treasure_chest", "minecraft:chests/shipwreck_treasure"), ("supply_chest", "minecraft:chests/shipwreck_supply")], rng);
    Ok((b.finish(req.id, req.kind.name, req.kind.game_id, true, 0), HashMap::new()))
}

fn ocean_ruin(lib: &Library, ground: &dyn Ground, req: &Request, warm: bool, rng: &mut Rng) -> Result<(Structure, HashMap<(i32, i32), i32>), String> {
    let turn = req.turn.unwrap_or_else(|| rng.below(4) as u8);
    let (x, z) = (req.at[0], req.at[2]);
    let mut b = Builder::new(ground, req.seed);
    let big = rng.float() <= 0.3;
    let center = [x, ground.floor(x, z), z];
    let place = |b: &mut Builder, rng: &mut Rng, at: [i32; 3], turn: u8, big: bool, integrity: f32| {
        // Sand or gravel that hides an archaeologist's find, five at most.
        let (find, sus, table) = if warm { ("minecraft:sand", "minecraft:suspicious_sand[dusted=0]", "minecraft:archaeology/ocean_ruin_warm") } else { ("minecraft:gravel", "minecraft:suspicious_gravel[dusted=0]", "minecraft:archaeology/ocean_ruin_cold") };
        let archaeology = Processor::Capped {
            limit: (5, 5),
            delegate: Box::new(Processor::Rule(vec![crate::processor::Rule {
                input: Test::Block(find.into()),
                location: Test::Always,
                position: crate::processor::PosTest::Always,
                output: Arc::from(sus),
                modifier: crate::processor::Modifier::Loot(table.into()),
            }])),
        };
        let list = |integrity: f32| vec![Processor::Rot { integrity, rottable: None }, Processor::Ignore(vec!["minecraft:structure_block".into(), "minecraft:air".into()]), archaeology.clone()];
        let names: Vec<String> = if warm {
            vec![if big { format!("minecraft:underwater_ruin/big_warm_{}", rng.between(4, 7)) } else { format!("minecraft:underwater_ruin/warm_{}", rng.between(1, 8)) }]
        } else {
            // Cold ruins are three of the same laid over one another: in
            // brick, then cracked, then mossy.
            let i = if big { *rng.pick(&[1, 2, 3, 8]).expect("four") } else { rng.between(1, 8) };
            let size = if big { "big_" } else { "" };
            ["brick", "cracked", "mossy"].iter().map(|m| format!("minecraft:underwater_ruin/{size}{m}_{i}")).collect()
        };
        for (k, name) in names.iter().enumerate() {
            let Some(t) = lib.template(name) else { continue };
            // On the lowest of the sea bed under it.
            let probe = BBox::of_template(t.size, at, turn);
            let mut low = i32::MAX;
            for (cx, cz) in [(probe.min[0], probe.min[2]), (probe.max[0], probe.min[2]), (probe.min[0], probe.max[2]), (probe.max[0], probe.max[2]), (at[0], at[2])] {
                low = low.min(ground.floor(cx, cz));
            }
            let integrity = if warm || k == 0 { integrity } else { [1.0, 0.7, 0.5][k] };
            let bbox = build::place_template(lib, b, name, [at[0], low, at[2]], turn, &list(integrity), center, false);
            if let Some(bbox) = bbox {
                b.pieces.push(Piece { min: bbox.min, max: bbox.max, template: name.clone(), origin: [at[0], low, at[2]], turn, depth: 0, ground_delta: 0, rigid: true, processors: String::new() });
            }
        }
        // Chests and drowned where the template marks them.
        let markers = std::mem::take(&mut b.markers);
        for m in markers {
            let wet = ground.block(m.pos[0], m.pos[1], m.pos[2]) == WorldBlock::Water;
            match m.metadata.as_str() {
                "chest" => {
                    let state = format!("minecraft:chest[facing=north,type=single,waterlogged={wet}]");
                    b.set(m.pos, &state, Some(build::loot("minecraft:chest", if big { "minecraft:chests/underwater_ruin_big" } else { "minecraft:chests/underwater_ruin_small" }, rng)));
                }
                "drowned" => {
                    b.set(m.pos, if wet { "minecraft:water[level=0]" } else { "minecraft:air" }, None);
                    let mut e = nbt::Compound::new();
                    e.insert("id".into(), fastnbt::Value::String("minecraft:drowned".into()));
                    e.insert("PersistenceRequired".into(), fastnbt::Value::Byte(1));
                    b.add_entity([m.pos[0] as f64 + 0.5, m.pos[1] as f64, m.pos[2] as f64 + 0.5], e);
                }
                _ => {}
            }
        }
    };
    let first = [x - 8, 0, z - 8];
    place(&mut b, rng, first, turn, big, if big { 0.9 } else { 0.8 });
    // Around a big one, a cluster of small ones.
    if big && rng.float() <= 0.9 {
        let around = [
            [-16 + rng.between(1, 8), 16 + rng.between(1, 7)],
            [-16 + rng.between(1, 8), rng.between(1, 7)],
            [-16 + rng.between(1, 8), -16 + rng.between(4, 8)],
            [rng.between(1, 7), 16 + rng.between(1, 7)],
            [rng.between(1, 7), -16 + rng.between(4, 6)],
            [16 + rng.between(1, 7), 16 + rng.between(3, 8)],
            [16 + rng.between(1, 7), rng.between(1, 7)],
            [16 + rng.between(1, 7), -16 + rng.between(4, 8)],
        ];
        let mut spots: Vec<[i32; 2]> = around.to_vec();
        let count = rng.between(4, 8);
        for _ in 0..count {
            if spots.is_empty() {
                break;
            }
            let k = rng.below(spots.len() as u32) as usize;
            let s = spots.remove(k);
            let small_turn = rng.below(4) as u8;
            place(&mut b, rng, [first[0] + s[0], 0, first[2] + s[1]], small_turn, false, 0.8);
        }
    }
    Ok((b.finish(req.id, req.kind.name, req.kind.game_id, true, 0), HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_is_unique_and_known() {
        let mut ids: Vec<&str> = KINDS.iter().map(|k| k.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), KINDS.len());
        assert!(kind("ancient_city").is_some_and(|k| k.biome == Some("minecraft:deep_dark")));
        assert!(beached("with_mast") && beached("rightsideup_full_degraded") && !beached("upsidedown_full"));
    }
}
