//! Ground covers of layer blocks turned into the ground's own layers.
//!
//! A WorldPainter map made for a mod such as Conquest Reforged often lays
//! the layers of a block over the land as a custom ground cover
//! (`conquest:grass_block_layer[layers=3]` over its grass): an eighth of a
//! block at a time, the land as high as it looks. In Massif that is the
//! land's own height and Smooth terrain's Layers ([`crate::shapes`]): the
//! ground raised by the cover's eighths, its top block the full block those
//! layers are of, and the column smoothed, so that the export lays the same
//! layers, and the map, the 3D view, the brushes and the smoothing all know
//! the land's true height. [`surface_covers`] finds the covers that are such
//! layers; [`to_ground`] turns those chosen into ground.

use crate::compose::block_top;
use crate::coords::TilePos;
use crate::registry::{Registry, Stratum, TerrainDef, TerrainId};
use crate::shapes::{Known, Shape};
use crate::world::{Dimension, WorldMeta};
use std::collections::HashMap;

/// A ground cover whose every block is the layers of a full block.
#[derive(Debug, Clone, PartialEq)]
pub struct SurfaceCover {
    /// Its number.
    pub cover: u16,
    pub name: String,
    /// For each of its plants, the full block its layers are of and how
    /// many eighths of a block they are; `None` for a plant that is not
    /// layers (small stones among the layers of forest floor), which stays
    /// a cover where it grows.
    pub plants: Vec<Option<(String, u8)>>,
}

impl SurfaceCover {
    /// Whether all its plants are layers: turned into ground, nothing is
    /// left of it.
    pub fn all_layers(&self) -> bool {
        self.plants.iter().all(Option::is_some)
    }

    /// The full blocks of its layers, each once.
    pub fn blocks(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for (b, _) in self.plants.iter().flatten() {
            if !out.contains(&b.as_str()) {
                out.push(b);
            }
        }
        out
    }
}

/// The full block a layer block state is of, and its eighths: the block
/// whose layers ([`crate::shapes::family`]) it is. `None` for anything else.
pub fn full_block_of(state: &str, known: &Known) -> Option<(String, u8)> {
    let (name, props) = state.split_once('[').map_or((state, ""), |(n, p)| (n, p.trim_end_matches(']')));
    let eighths = props.split(',').find_map(|kv| kv.trim().strip_prefix("layers=")).and_then(|v| v.parse::<u8>().ok()).unwrap_or(1).clamp(1, 8);
    if name == "minecraft:snow" {
        return Some(("minecraft:snow_block".into(), eighths));
    }
    let (ns, path) = name.split_once(':')?;
    let base = path.strip_suffix("_layer").or_else(|| path.strip_suffix("_layers"))?;
    let candidates = [format!("{ns}:{base}"), format!("{ns}:{base}_block"), format!("minecraft:{base}"), format!("minecraft:{base}_block")];
    if let Some(c) = candidates.into_iter().find(|c| known.exists(c) && crate::shapes::family(c, known).get(Shape::Layers) == Some(name)) {
        return Some((c, eighths));
    }
    // Layers with no full block of their own (Conquest's grass and sand,
    // grassy gravel…): eight of them are one.
    (known.exists(name) && crate::shapes::family(name, known).get(Shape::Layers) == Some(name)).then(|| (format!("{name}[layers=8]"), eighths))
}

/// The map's ground covers whose plants are layers of full blocks, all or
/// some of them, in their order. The mods' blocks the map uses count as
/// known, the mod read or not.
pub fn surface_covers(reg: &Registry, known: &Known) -> Vec<SurfaceCover> {
    let merged = crate::shapes::with_map_blocks(reg, known.mods);
    let known = Known { vanilla: known.vanilla, mods: merged.as_ref().unwrap_or(known.mods) };
    let mut out = Vec::new();
    for (k, def) in reg.covers.iter().enumerate() {
        if def.removed || def.changes_water() || def.underwater || def.plants.is_empty() {
            continue;
        }
        let plants: Vec<Option<(String, u8)>> = def.plants.iter().map(|(s, _)| full_block_of(s, &known)).collect();
        if plants.iter().any(Option::is_some) {
            out.push(SurfaceCover { cover: k as u16 + 1, name: def.name.clone(), plants });
        }
    }
    out
}

/// What [`to_ground`] made.
#[derive(Debug)]
pub struct Grounded {
    pub dimension: Dimension,
    pub meta: WorldMeta,
    pub registry: Registry,
    /// Columns raised.
    pub columns: usize,
}

/// Dimension `dim` of the map with the covers `chosen` turned into ground:
/// on each column one grows on, the ground raised by its eighths, its
/// terrain one whose top block is the full block (the column's own when it
/// already is, else one made of that block over the column's terrain, once
/// for each cover and block), the column smoothed with layers, the cover
/// gone. The covers are removed from the map's layers. Returned with the
/// map's settings and registry as they become, to replace as one step.
pub fn to_ground(dimension: &Dimension, meta: &WorldMeta, registry: &Registry, chosen: &[SurfaceCover]) -> Grounded {
    let mut reg = registry.clone();
    let mut meta = meta.clone();
    let mut dim = dimension.clone();
    let seed = meta.seed as u64;
    let by_cover: HashMap<u16, &SurfaceCover> = chosen.iter().map(|c| (c.cover, c)).collect();
    // The terrain of each (terrain under it, full block), made once.
    let mut terrains: HashMap<(u16, String), u16> = HashMap::new();
    let mut made: HashMap<(u16, String), u16> = HashMap::new();
    let mut columns = 0;
    let positions: Vec<TilePos> = dim.ground().positions().collect();
    for pos in positions {
        let has = dim.ground().view(pos).is_some_and(|t| {
            by_cover.keys().any(|&c| t.stacked.holds(crate::layer_stack::Kind::Cover, c))
                || match t.cover.uniform_value() {
                    Some(v) => by_cover.contains_key(&v),
                    None => t.cover.dense().iter().any(|v| by_cover.contains_key(v)),
                }
        });
        if !has {
            continue;
        }
        let Some(tile) = dim.ground_mut().tile_mut(pos) else { continue };
        let (ox, oz) = pos.origin();
        for i in 0..crate::TILE_AREA {
            // The lowest cover of layers of the column's stack, the one
            // lying on the ground.
            let Some(cover) = tile.stacked.covers_under(i).iter().chain(std::iter::once(&tile.cover.get(i))).find_map(|c| by_cover.get(c)) else { continue };
            let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
            let Some(def) = reg.cover(cover.cover) else { continue };
            let color = def.color;
            let Some(growth) = def.growth(x, z, seed) else { continue };
            let Some(Some((full, eighths))) = cover.plants.get(growth.plant) else { continue };
            let base = tile.terrain.get(i);
            let terrain = *terrains.entry((base, full.clone())).or_insert_with(|| {
                if crate::shapes::top_block(&reg, TerrainId(base)).is_some_and(|b| reg.block(b).name() == full) {
                    return base;
                }
                *made.entry((cover.cover, full.clone())).or_insert_with(|| {
                    let under = reg.terrain(TerrainId(base)).clone();
                    let block = reg.intern_block(full, color);
                    let mut strata = vec![Stratum { blocks: vec![(block, 1000)], depth: (1, 1), mix: Default::default() }];
                    strata.extend(under.strata.iter().skip(1).cloned());
                    let name = unique_name(&reg, &cover.name);
                    reg.terrains.push(TerrainDef { name, group: under.group.clone(), color, strata, submerged_top: None, biome_hint: under.biome_hint });
                    (reg.terrains.len() - 1) as u16
                })
            });
            // The ground's surface is a whole block: its layers lie on it.
            let top = block_top(tile.top.get(i)) as f32;
            tile.top.set(i, top + *eighths as f32 / 8.0);
            tile.terrain.set(i, terrain);
            if tile.cover.get(i) == cover.cover {
                let next = tile.stacked.pop_cover(i).unwrap_or(0);
                tile.cover.set(i, next);
            } else {
                tile.stacked.remove(i, crate::layer_stack::Kind::Cover, cover.cover);
            }
            if *eighths < 8 {
                tile.slabs.set(i, crate::shapes::value(None, false));
            }
            columns += 1;
        }
        tile.compact();
    }
    dim.ground_mut().take_dirty();
    dim.ground_mut().freeze();
    // The terrains used may lay layers: the full blocks' own shapes, with
    // layers.
    let used: Vec<u16> = terrains.values().copied().collect();
    for t in used {
        let mut shapes = meta.smooth.shapes_of(&reg, TerrainId(t));
        if !shapes.has(Shape::Layers) {
            shapes.set(Shape::Layers, true);
            meta.smooth.terrains.insert(reg.terrain(TerrainId(t)).name.clone(), shapes);
        }
    }
    // A cover all of layers is gone; one with other plants stays where
    // those grow.
    for c in chosen.iter().filter(|c| c.all_layers()) {
        if let Some(def) = reg.covers.get_mut(c.cover as usize - 1) {
            def.removed = true;
        }
    }
    Grounded { dimension: dim, meta, registry: reg, columns }
}

/// `name`, or `name 2`, `name 3`… if a terrain has it.
fn unique_name(reg: &Registry, name: &str) -> String {
    let taken = |n: &str| reg.terrains.iter().any(|t| t.name == n);
    if !taken(name) {
        return name.to_owned();
    }
    (2..).map(|k| format!("{name} {k}")).find(|n| !taken(n)).expect("a free name")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::CoverDef;
    use crate::tile::{NO_FLUID, Tile};

    fn known(mods: &HashMap<String, Vec<String>>) -> Known<'_> {
        Known { vanilla: &|id: &str| matches!(id, "minecraft:grass_block" | "minecraft:snow_block" | "minecraft:snow" | "minecraft:dirt"), mods }
    }

    fn mods() -> HashMap<String, Vec<String>> {
        [("conquest:grass_block_layer", vec!["layers"]), ("conquest:sand_layer", vec!["layers"]), ("conquest:sand", vec![]), ("conquest:small_stones", vec![])].into_iter().map(|(k, v)| (k.to_owned(), v.into_iter().map(str::to_owned).collect())).collect()
    }

    #[test]
    fn layer_blocks_know_their_full_block() {
        let mods = mods();
        let k = known(&mods);
        assert_eq!(full_block_of("conquest:grass_block_layer[layers=3]", &k), Some(("minecraft:grass_block".into(), 3)));
        assert_eq!(full_block_of("conquest:sand_layer[layers=8]", &k), Some(("conquest:sand".into(), 8)));
        assert_eq!(full_block_of("minecraft:snow[layers=2]", &k), Some(("minecraft:snow_block".into(), 2)));
        assert_eq!(full_block_of("conquest:small_stones", &k), None);
        assert_eq!(full_block_of("minecraft:water", &k), None);
    }

    /// Two covers of grass layers (one over grass, one over dirt) and one of
    /// small stones: the first two are found; turned into ground, their
    /// columns rise by their eighths and are smoothed with layers, grass
    /// keeps its terrain, dirt gets a terrain of grass over it, and the
    /// covers are gone; the stones stay a cover.
    #[test]
    fn layer_covers_become_the_grounds_own_layers() {
        let mods = mods();
        let k = known(&mods);
        let mut world = crate::generate::generate(&crate::generate::GenParams { tiles_x: 1, tiles_z: 1, ..Default::default() });
        let reg = &mut world.registry;
        let grass = reg.terrain_by_name("Grass").unwrap().0;
        let dirt = reg.terrain_by_name("Dirt").unwrap().0;
        let mut def = |name: &str, state: &str| {
            let mut c = CoverDef::new(name, [90, 140, 60], &[(state, 1)], 1.0);
            c.soil_only = false;
            reg.add_cover(c)
        };
        let (thin, full, stones) = (def("Grass layers", "conquest:grass_block_layer[layers=3]"), def("Thick grass", "conquest:grass_block_layer[layers=6]"), def("Small stones", "conquest:small_stones"));
        let found = surface_covers(&world.registry, &k);
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Snow", "Grass layers", "Thick grass"], "the map's own snow is layers too");
        let found: Vec<SurfaceCover> = found.into_iter().filter(|c| c.cover == thin || c.cover == full).collect();
        let pos = world.dims[0].ground().positions().next().unwrap();
        let mut tile = Tile::ground(70.0, grass, NO_FLUID);
        for i in 0..crate::TILE_AREA {
            let row = i / 128;
            tile.terrain.set(i, if row >= 48 { dirt } else { grass });
            tile.cover.set(i, [thin, full, stones, 0][row / 32]);
        }
        world.dims[0].ground_mut().insert(pos, tile);
        let before = world.registry.terrains.len();
        let g = to_ground(&world.dims[0], &world.meta, &world.registry, &found);
        assert_eq!(g.columns, 64 * 128);
        let t = g.dimension.ground().view(pos).unwrap();
        assert_eq!((t.top.get(0), t.terrain.get(0), t.cover.get(0)), (70.375, grass, 0));
        assert_eq!(crate::shapes::decode(t.slabs.get(0)), Some(Some((None, false))), "smoothed");
        assert_eq!((t.top.get(40 * 128), t.cover.get(40 * 128)), (70.75, 0));
        assert_eq!((t.top.get(70 * 128), t.cover.get(70 * 128)), (70.0, stones), "the stones stay");
        assert_eq!(g.registry.terrains.len(), before + 1, "grass over grass needs no terrain, grass over dirt one");
        assert_eq!(t.terrain.get(50 * 128), before as u16);
        let made = &g.registry.terrains[before];
        assert_eq!((made.name.as_str(), g.registry.block(made.strata[0].blocks[0].0).name()), ("Thick grass", "minecraft:grass_block"));
        assert!(g.registry.covers[thin as usize - 1].removed && g.registry.covers[full as usize - 1].removed && !g.registry.covers[stones as usize - 1].removed);
        assert!(g.meta.smooth.shapes_of(&g.registry, TerrainId(grass)).has(Shape::Layers));
    }

    /// A WorldPainter map of Conquest Reforged, the mod not read: its covers
    /// of layers are found by the blocks the map uses itself. A cover mixing
    /// layers and small stones is offered, and turned into ground where its
    /// layers grow, the stones staying a cover; layers with no full block
    /// of their own use eight of them; a cover lying under plants is turned
    /// into ground, the plants staying on top.
    #[test]
    fn covers_of_layers_are_found_without_the_mod_mixed_and_stacked() {
        use crate::layer_stack::{Entry, Kind};
        let none = HashMap::new();
        let k = Known { vanilla: &|id: &str| matches!(id, "minecraft:grass_block" | "minecraft:snow_block" | "minecraft:snow" | "minecraft:dirt"), mods: &none };
        let mut world = crate::generate::generate(&crate::generate::GenParams { tiles_x: 1, tiles_z: 1, ..Default::default() });
        let reg = &mut world.registry;
        let grass = reg.terrain_by_name("Grass").unwrap().0;
        let mut def = |name: &str, plants: &[(&str, u16)]| {
            let mut c = CoverDef::new(name, [90, 140, 60], plants, 1.0);
            c.soil_only = false;
            reg.add_cover(c)
        };
        let mixed = def("Forest floor", &[("conquest:grass_block_layer[layers=2]", 1), ("conquest:small_stones", 1)]);
        let sand = def("Grass and sand", &[("conquest:grass_and_sand_layer[layers=4]", 1)]);
        let flowers = def("Flowers", &[("minecraft:poppy", 1)]);
        let found = surface_covers(&world.registry, &k);
        let get = |n: u16| found.iter().find(|c| c.cover == n).unwrap_or_else(|| panic!("cover {n} found: {found:?}"));
        assert!(!get(mixed).all_layers() && get(mixed).blocks() == ["minecraft:grass_block"]);
        assert_eq!(get(sand).blocks(), ["conquest:grass_and_sand_layer[layers=8]"]);
        assert!(found.iter().all(|c| c.cover != flowers));
        let pos = world.dims[0].ground().positions().next().unwrap();
        let mut tile = Tile::ground(70.0, grass, NO_FLUID);
        for i in 0..crate::TILE_AREA {
            if i / 128 < 64 {
                tile.cover.set(i, mixed);
            } else {
                // Grass and sand under flowers.
                tile.cover.set(i, flowers);
                tile.stacked.set(i, &[Entry::new(Kind::Cover, sand, 255)]);
            }
        }
        world.dims[0].ground_mut().insert(pos, tile);
        let chosen: Vec<SurfaceCover> = found.into_iter().filter(|c| c.cover == mixed || c.cover == sand).collect();
        let g = to_ground(&world.dims[0], &world.meta, &world.registry, &chosen);
        let t = g.dimension.ground().view(pos).unwrap();
        let (layered, stones) = (0..64 * 128).partition::<Vec<usize>, _>(|&i| t.cover.get(i) == 0);
        assert!(layered.len() > 2000 && stones.len() > 2000, "{} {}", layered.len(), stones.len());
        assert!(layered.iter().all(|&i| t.top.get(i) == 70.25) && stones.iter().all(|&i| t.top.get(i) == 70.0 && t.cover.get(i) == mixed));
        let i = 100 * 128;
        assert_eq!((t.top.get(i), t.cover.get(i), t.stacked.covers_under(i)), (70.5, flowers, vec![]), "the flowers stay on the raised ground");
        let top = crate::shapes::top_block(&g.registry, TerrainId(t.terrain.get(i))).unwrap();
        assert_eq!(g.registry.block(top).state, "conquest:grass_and_sand_layer[layers=8]");
        assert!(!g.registry.covers[mixed as usize - 1].removed && g.registry.covers[sand as usize - 1].removed);
    }
}
