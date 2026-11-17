//! WorldPainter `.layer` files: one custom layer, as the Export of its
//! menu writes it (the layer Java-serialised, gzipped, with what it holds:
//! a custom object layer's objects, a combined layer's layers and custom
//! terrain), brought into a map's registry as Massif's layers by the same
//! mapping as a `.world`'s ([`crate::layers::Converter`]). Experimental.

use crate::javaser::{Item, Parser, Value};
use crate::layers::{Conv, Converter};
use crate::terrains::{CustomMaterial, Mapper};
use crate::worldpainter::read_mixed;
use massif_core::registry::{CoverDef, Registry, Stratum, TerrainDef};
use std::io::Read;

const LAYERS: &str = "org.pepsoft.worldpainter.layers.";

/// A layer a `.layer` file became in the registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Made {
    /// A ground cover, by number: plants, Frost, Lava, or the blocks of a
    /// WorldPainter ground cover laid over the land.
    Cover(u16),
    /// An object layer, by number, with how many objects it has.
    Objects(u16, usize),
    /// A layer of underground pockets, by number.
    Pockets(u16),
    /// A combined layer, by number.
    Combined(u16),
    /// A terrain, by id: a WorldPainter ground cover that takes the place
    /// of the top of the land.
    Terrain(u16),
}

/// Settings of a WorldPainter layer Massif does not take.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    /// A custom object layer's objects: collision, underground and leaf
    /// decay modes, foundations, height modes, lava and water rules.
    ObjectRules,
    /// A ground cover's edge shape and width.
    Edges,
    /// A ground cover's noise.
    Noise,
    /// Farmland under a plant layer's crops.
    Farmland,
}

/// What a `.layer` file held that Massif did not take.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LeftOut {
    /// A tunnel layer, or (`floating`) a floating one: Massif carves and
    /// raises the land with levels.
    Tunnel { name: String, floating: bool },
    /// A custom annotation layer: Massif's annotations are fifteen colours.
    Annotations(String),
    /// A combined layer's custom terrain, which could not be read.
    CustomTerrain(String),
    /// A kind of layer Massif does not know: WorldPainter's class.
    Unknown(String),
    /// Settings of layer `0` left out.
    Settings(String, Setting),
    /// Layer `0`, imported as a layer of its own, which combined layer `1`
    /// does not paint: it paints one layer of each slot, and another of
    /// that slot comes first.
    NotInCombined(String, String),
    /// Of object layer `0`, `1` objects of no blocks: air, or entities
    /// only, which Massif does not place.
    EmptyObjects(String, usize),
}

/// What one `.layer` file became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LayerFile {
    /// The layer's name in WorldPainter.
    pub name: String,
    /// WorldPainter's kind of layer: its class, after
    /// `org.pepsoft.worldpainter.layers.` ("plants.PlantLayer").
    pub class: String,
    /// The layers it became, the one to paint with first.
    pub made: Vec<Made>,
    pub left_out: Vec<LeftOut>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LayerFileError {
    /// Not a WorldPainter layer: not gzip, not a Java stream, or another
    /// object in it.
    NotALayer,
    /// A WorldPainter layer that could not be read whole.
    Broken(String),
}

impl std::fmt::Display for LayerFileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LayerFileError::NotALayer => write!(f, "not a WorldPainter layer file"),
            LayerFileError::Broken(why) => write!(f, "a WorldPainter layer file that could not be read: {why}"),
        }
    }
}

/// Read a `.layer` file's bytes into `registry`: the layers it becomes are
/// added to it (names made unique among the map's), and what it held that
/// Massif does not take is said. The registry is left as it was if the
/// file cannot be read.
pub fn read_layer_file(bytes: &[u8], registry: &mut Registry) -> Result<LayerFile, LayerFileError> {
    let mut raw = Vec::new();
    let data: &[u8] = if bytes.starts_with(&[0x1f, 0x8b]) {
        flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw).map_err(|_| LayerFileError::NotALayer)?;
        &raw
    } else {
        bytes
    };
    let mut parser = Parser::new(data).map_err(|_| LayerFileError::NotALayer)?;
    let value = match parser.next_object() {
        Ok(Some(v @ Value::Ref(_))) => v,
        Ok(_) => return Err(LayerFileError::NotALayer),
        Err(e) => return Err(LayerFileError::Broken(format!("{e:?}"))),
    };
    let arena = &parser.arena;
    let Value::Ref(handle) = value else { return Err(LayerFileError::NotALayer) };
    let class = arena.class_name(value).and_then(|c| c.strip_prefix(LAYERS)).ok_or(LayerFileError::NotALayer)?.to_owned();
    let name = arena.field(value, "name").and_then(|v| arena.string(v)).unwrap_or(&class).to_owned();

    let before = Lengths::of(registry);
    let mut mapper = Mapper::new(registry.clone());
    let mut conv = Converter::default();
    let parts = conv.parts(arena, value, handle, &mut mapper);
    let mut out = LayerFile { name: name.clone(), class: class.clone(), made: Vec::new(), left_out: Vec::new() };

    // The custom terrains of the combined layers in it, written after
    // their fields.
    let mut materials: Vec<Option<CustomMaterial>> = Vec::new();
    for h in 0..arena.items.len() {
        let v = Value::Ref(h);
        if arena.class_name(v) != Some("org.pepsoft.worldpainter.layers.CombinedLayer") {
            continue;
        }
        let Some(index) = arena.field(v, "terrain").and_then(|t| arena.enum_name(t)).and_then(|t| t.strip_prefix("CUSTOM_")).and_then(|n| n.parse::<usize>().ok()).and_then(|n| n.checked_sub(1)) else { continue };
        let material = arena.annotation(v, "org.pepsoft.worldpainter.layers.CombinedLayer").into_iter().flatten().find_map(|item| match item {
            Item::Value(m) => read_mixed(arena, *m),
            Item::Block(_) => None,
        });
        if materials.len() <= index {
            materials.resize(index + 1, None);
        }
        materials[index] = material;
    }
    mapper.finish_custom(&materials);
    // A custom terrain not read stays a terrain of nothing: none painted.
    let empty: Vec<u16> = (before.terrains..mapper.registry.terrains.len()).filter(|&t| mapper.registry.terrains[t].strata.is_empty()).map(|t| t as u16).collect();
    if !empty.is_empty() {
        for c in &mut mapper.registry.combined[before.combined..] {
            if c.get(massif_core::combined::Part::Terrain).is_some_and(|(t, _)| empty.contains(&t)) {
                c.set(massif_core::combined::Part::Terrain, None);
                out.left_out.push(LeftOut::CustomTerrain(c.name.clone()));
            }
        }
    }

    // What became what; a combined layer first, the layers it holds after.
    if class == "CombinedLayer"
        && let Some(n) = (before.combined < mapper.registry.combined.len()).then_some(mapper.registry.combined.len() as u16)
    {
        out.made.push(Made::Combined(n));
    }
    for (part, _) in &parts {
        let made = match *part {
            Conv::Cover(n) => Made::Cover(n),
            Conv::Objects(n, _, _) => Made::Objects(n, mapper.registry.object_layer(n).map_or(0, |l| l.objects.len())),
            Conv::Pockets(n) => Made::Pockets(n),
            Conv::GroundCover(k) => {
                let cover = &conv.ground_covers[k as usize];
                ground_cover(cover, &mut mapper.registry)
            }
            Conv::Tunnel(k) => {
                let t = &conv.tunnels[k as usize];
                out.left_out.push(LeftOut::Tunnel { name: t.name.clone(), floating: t.floating });
                continue;
            }
            // A combined layer's terrain and biome are in its own settings.
            Conv::Terrain(_) | Conv::Biome(_) => continue,
            // WorldPainter's own layers in a combined layer: caves, void,
            // rivers, annotations are painted with Massif's tools.
            other => {
                let what = match other {
                    Conv::Caves(kind) => format!("{kind:?}"),
                    Conv::Void => "Void".to_owned(),
                    Conv::River => "River".to_owned(),
                    Conv::Annotations => "Annotations".to_owned(),
                    _ => class.clone(),
                };
                let left = LeftOut::Unknown(what);
                if !out.left_out.contains(&left) {
                    out.left_out.push(left);
                }
                continue;
            }
        };
        if !out.made.contains(&made) {
            out.made.push(made);
        }
    }
    keep_the_maps(registry, &mut mapper.registry, &mut out, &before);
    // A combined layer paints one layer of a slot: the others it held are
    // layers of their own.
    if let Some(&Made::Combined(c)) = out.made.first() {
        use massif_core::combined::Part;
        let reg = &mapper.registry;
        let def = &reg.combined[c as usize - 1];
        for m in &out.made[1..] {
            let (part, n, name) = match *m {
                Made::Cover(n) => (Part::Cover, n, reg.covers[n as usize - 1].name.clone()),
                Made::Objects(n, _) => (if reg.object_layers[n as usize - 1].understory { Part::Under } else { Part::Trees }, n, reg.object_layers[n as usize - 1].name.clone()),
                Made::Pockets(n) => (Part::Pockets, n, reg.pockets[n as usize - 1].name.clone()),
                Made::Combined(_) | Made::Terrain(_) => continue,
            };
            if def.get(part).is_none_or(|(layer, _)| layer != n) {
                out.left_out.push(LeftOut::NotInCombined(name, def.name.clone()));
            }
        }
    }
    // Layers of no slot of Massif's.
    for h in 0..arena.items.len() {
        let v = Value::Ref(h);
        let Some(c) = arena.class_name(v).and_then(|c| c.strip_prefix(LAYERS)) else { continue };
        let layer = arena.field(v, "name").and_then(|n| arena.string(n)).unwrap_or(c).to_owned();
        match c {
            "CustomAnnotationLayer" => out.left_out.push(LeftOut::Annotations(layer)),
            "Bo2Layer" if object_rules(arena, v) => out.left_out.push(LeftOut::Settings(layer, Setting::ObjectRules)),
            "groundcover.GroundCoverLayer" => {
                let edges = arena.field(v, "edgeShape").and_then(|e| arena.enum_name(e)).is_some_and(|e| e != "SHEER") || arena.field(v, "edgeWidth").and_then(|w| arena.int(w)).is_some_and(|w| w > 1);
                if edges {
                    out.left_out.push(LeftOut::Settings(layer.clone(), Setting::Edges));
                }
                let noise = arena.field(v, "noiseSettings").filter(|n| *n != Value::Null).and_then(|n| arena.field(n, "range")).and_then(|r| arena.int(r)).is_some_and(|r| r > 0);
                if noise {
                    out.left_out.push(LeftOut::Settings(layer, Setting::Noise));
                }
            }
            _ => {}
        }
    }
    if out.made.is_empty() && out.left_out.is_empty() {
        out.left_out.push(LeftOut::Unknown(class.clone()));
    }
    // Objects of nothing.
    for m in &out.made {
        if let Made::Objects(n, _) = *m {
            let reg = &mapper.registry;
            let layer = &reg.object_layers[n as usize - 1];
            let empty = layer.objects.iter().filter(|(o, _)| reg.objects.get(*o as usize).is_none_or(|o| o.blocks.iter().all(|&b| b == 0))).count();
            if empty > 0 {
                out.left_out.push(LeftOut::EmptyObjects(layer.name.clone(), empty));
            }
        }
    }
    // Farmland under crops.
    for m in &out.made {
        if let Made::Cover(n) = *m
            && let Some(c) = mapper.registry.covers.get(n as usize - 1)
            && c.plants.iter().any(|(p, _)| ["wheat", "carrots", "potatoes", "beetroots"].iter().any(|crop| p.contains(crop)))
        {
            out.left_out.push(LeftOut::Settings(c.name.clone(), Setting::Farmland));
        }
    }

    unique_names(&mut mapper.registry, &before);
    *registry = mapper.registry;
    Ok(out)
}

/// What the map had stays as it was: a layer of the file that took the
/// place of one of the map's of the same name (as a `.world`'s plants and
/// custom terrains do in a new map) is added as a layer of its own instead,
/// and what refers to it follows.
fn keep_the_maps(original: &Registry, reg: &mut Registry, out: &mut LayerFile, before: &Lengths) {
    use massif_core::combined::Part;
    let follow = |reg: &mut Registry, out: &mut LayerFile, part: Part, old: u16, new: u16| {
        for m in &mut out.made {
            match m {
                Made::Cover(n) if part == Part::Cover && *n == old => *n = new,
                Made::Terrain(n) if part == Part::Terrain && *n == old => *n = new,
                _ => {}
            }
        }
        for c in &mut reg.combined[before.combined..] {
            if let Some((layer, share)) = c.get(part)
                && layer == old
            {
                c.set(part, Some((new, share)));
            }
        }
    };
    for k in 0..before.covers {
        if reg.covers[k] != original.covers[k] {
            let def = std::mem::replace(&mut reg.covers[k], original.covers[k].clone());
            let new = reg.add_cover(def);
            follow(reg, out, Part::Cover, k as u16 + 1, new);
        }
    }
    for k in 0..before.terrains {
        if reg.terrains[k] != original.terrains[k] {
            let def = std::mem::replace(&mut reg.terrains[k], original.terrains[k].clone());
            reg.terrains.push(def);
            let new = reg.terrains.len() as u16 - 1;
            follow(reg, out, Part::Terrain, k as u16, new);
        }
    }
}

/// Whether a custom object layer's objects have settings Massif does not
/// take.
fn object_rules(arena: &crate::javaser::Arena, layer: Value) -> bool {
    const KEPT: [&str; 6] = ["WPObject.file", "WPObject.offset", "WPObject.randomRotation", "WPObject.spawnInWater", "WPObject.spawnOnLand", "WPObject.frequency"];
    let Some(items) = arena.field(layer, "objectProvider").and_then(|p| arena.field(p, "objects")).and_then(|o| arena.list_items(o)) else { return false };
    items.into_iter().any(|item| {
        arena
            .field(item, "attributes")
            .and_then(|a| arena.map_entries(a))
            .unwrap_or_default()
            .into_iter()
            .any(|(k, _)| arena.string(k).is_some_and(|k| k.starts_with("WPObject.") && !KEPT.contains(&k)))
    })
}

/// A WorldPainter ground cover as Massif can paint it: blocks laid over
/// the land, as many as it is thick, a ground cover; or, for one that
/// takes the place of the top of the land, a terrain of its blocks that
/// deep.
fn ground_cover(cover: &crate::layers::GroundCover, reg: &mut Registry) -> Made {
    let total: u64 = cover.blocks.iter().map(|b| b.1 as u64).sum::<u64>().max(1);
    let weights: Vec<(&str, u16)> = cover.blocks.iter().map(|(s, n)| (s.as_str(), ((*n as u64 * 1000) / total).clamp(1, 1000) as u16)).collect();
    let color = cover.blocks.first().map_or([128; 3], |b| crate::terrains::block_color(&b.0));
    let depth = cover.thickness.unsigned_abs().clamp(1, 16) as u8;
    if cover.thickness > 0 {
        let mut def = CoverDef::new(&cover.name, color, &weights, 1.0);
        (def.height, def.soil_only) = ([depth, depth], false);
        Made::Cover(reg.add_cover(def))
    } else {
        let blocks = cover.blocks.iter().map(|(s, n)| (reg.intern_block(s, crate::terrains::block_color(s)), ((*n as u64 * 1000) / total).clamp(1, 1000) as u16)).collect();
        reg.terrains.push(TerrainDef {
            name: cover.name.clone(),
            group: "Imported".into(),
            color,
            strata: vec![Stratum { blocks, depth: (depth as u16, depth as u16), mix: Default::default() }],
            submerged_top: None,
            biome_hint: None,
        });
        Made::Terrain(reg.terrains.len() as u16 - 1)
    }
}

/// How many layers of each kind the registry had before the import.
struct Lengths {
    covers: usize,
    object_layers: usize,
    pockets: usize,
    combined: usize,
    terrains: usize,
}

impl Lengths {
    fn of(reg: &Registry) -> Self {
        Self { covers: reg.covers.len(), object_layers: reg.object_layers.len(), pockets: reg.pockets.len(), combined: reg.combined.len(), terrains: reg.terrains.len() }
    }
}

/// `name`, or `name 2`, `name 3`… if `taken` says it is taken.
fn unique(name: &str, taken: impl Fn(&str) -> bool) -> String {
    if !taken(name) {
        return name.to_owned();
    }
    (2..).map(|k| format!("{name} {k}")).find(|n| !taken(n)).expect("a free name")
}

/// Give the layers added since `before` names of their own among the
/// map's, so that a layer imported twice is told apart.
fn unique_names(reg: &mut Registry, before: &Lengths) {
    fn rename<T>(list: &mut [T], from: usize, name: impl Fn(&mut T) -> Option<&mut String>) {
        for k in from..list.len() {
            let (old, new) = list.split_at_mut(k);
            let Some(own) = name(&mut new[0]) else { continue };
            let taken: Vec<String> = old.iter_mut().filter_map(|t| name(t).map(|n| n.to_lowercase())).collect();
            *own = unique(own, |n| taken.contains(&n.to_lowercase()));
        }
    }
    rename(&mut reg.covers, before.covers, |c| (!c.removed).then_some(&mut c.name));
    rename(&mut reg.object_layers, before.object_layers, |l| (!l.removed).then_some(&mut l.name));
    rename(&mut reg.pockets, before.pockets, |p| (!p.removed).then_some(&mut p.name));
    rename(&mut reg.combined, before.combined, |c| (!c.removed).then_some(&mut c.name));
    let terrains: Vec<String> = reg.terrains[..before.terrains].iter().map(|t| t.name.to_lowercase()).collect();
    for t in &mut reg.terrains[before.terrains..] {
        t.name = unique(&t.name, |n| terrains.contains(&n.to_lowercase()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::combined::Part;
    use std::path::Path;

    /// A sample of `tests/data/layers` read into a new map's registry.
    fn read(name: &str) -> (Registry, LayerFile) {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/layers").join(format!("{name}.layer"));
        let mut reg = Registry::vanilla();
        let file = read_layer_file(&std::fs::read(path).unwrap(), &mut reg).unwrap();
        (reg, file)
    }

    fn states(reg: &Registry, blocks: &[(massif_core::registry::BlockId, u16)]) -> Vec<(String, u16)> {
        blocks.iter().map(|&(b, w)| (reg.block(b).state.clone(), w)).collect()
    }

    /// A plant layer becomes a ground cover of the same plants in the same
    /// shares, on valid blocks only as it asked.
    #[test]
    fn plants_become_a_ground_cover() {
        let (reg, file) = read("Meadow");
        assert_eq!((file.name.as_str(), file.class.as_str()), ("Meadow", "plants.PlantLayer"));
        let [Made::Cover(n)] = file.made[..] else { panic!("{file:?}") };
        let c = &reg.covers[n as usize - 1];
        assert_eq!(c.name, "Meadow");
        let mut plants = c.plants.clone();
        plants.sort();
        assert_eq!(plants, [("minecraft:dandelion".to_owned(), 8), ("minecraft:poppy".to_owned(), 10), ("minecraft:short_grass".to_owned(), 60), ("minecraft:tall_grass".to_owned(), 12)]);
        assert!(c.soil_only);
        assert!(file.left_out.is_empty(), "{:?}", file.left_out);
    }

    /// A ground cover laid over the land becomes a ground cover of its
    /// blocks as high as it is thick; one that takes the place of the top
    /// of the land, a terrain of its blocks that deep (River's water, an
    /// old numbered block), its edges and noise said to be left out.
    #[test]
    fn ground_covers_become_covers_or_terrains() {
        let (reg, file) = read("Snow cover");
        let [Made::Cover(n)] = file.made[..] else { panic!("{file:?}") };
        let c = &reg.covers[n as usize - 1];
        assert_eq!((c.plants.clone(), c.height, c.density, c.soil_only), (vec![("minecraft:snow_block".to_owned(), 1000)], [2, 2], 1.0, false));
        let (reg, file) = read("River");
        let [Made::Terrain(t)] = file.made[..] else { panic!("{file:?}") };
        let t = &reg.terrains[t as usize];
        assert_eq!((t.name.as_str(), t.strata.len(), t.strata[0].depth), ("River", 1, (5, 5)));
        assert!(reg.block(t.strata[0].blocks[0].0).state.starts_with("minecraft:water"), "{:?}", states(&reg, &t.strata[0].blocks));
        assert_eq!(file.left_out, [LeftOut::Settings("River".into(), Setting::Edges), LeftOut::Settings("River".into(), Setting::Noise)]);
    }

    /// Pockets come with their blocks, share, size and heights.
    #[test]
    fn pockets_come_with_their_settings() {
        let (reg, file) = read("Gravel pockets");
        let [Made::Pockets(n)] = file.made[..] else { panic!("{file:?}") };
        let p = reg.pocket(n).unwrap();
        assert_eq!(states(&reg, &p.blocks), [("minecraft:gravel".to_owned(), 750), ("minecraft:clay".to_owned(), 250)]);
        assert_eq!((p.share, p.low, p.high), (0.12, -40, 50));
        assert!((p.size - 4.099 * 1.5).abs() < 1e-4);
    }

    /// A combined layer becomes one: its terrain, biome, frost at its
    /// factor and pockets at theirs; the plants it also held are a layer of
    /// their own it does not paint (one cover a block), and it says so. A
    /// custom terrain comes from the material written with it.
    #[test]
    fn combined_layers_bring_their_layers_and_terrain() {
        let (reg, file) = read("Snowy meadow");
        let [Made::Combined(c), Made::Cover(frost), Made::Cover(meadow), Made::Pockets(pockets)] = file.made[..] else { panic!("{file:?}") };
        let def = &reg.combined[c as usize - 1];
        assert_eq!(def.name, "Snowy meadow");
        let (terrain, _) = def.get(Part::Terrain).unwrap();
        let (biome, _) = def.get(Part::Biome).unwrap();
        assert_eq!((reg.terrains[terrain as usize].name.as_str(), reg.biomes[biome as usize].id.as_str()), ("Grass", "minecraft:forest"));
        assert!(reg.covers[frost as usize - 1].frost);
        assert_eq!((def.get(Part::Cover), def.get(Part::Pockets)), (Some((frost, 0.5)), Some((pockets, 0.75))));
        assert_eq!(reg.covers[meadow as usize - 1].name, "Meadow");
        assert_eq!(file.left_out, [LeftOut::NotInCombined("Meadow".into(), "Snowy meadow".into())]);

        let (reg, file) = read("Mossy ruins");
        let [Made::Combined(c)] = file.made[..] else { panic!("{file:?}") };
        let (terrain, _) = reg.combined[c as usize - 1].get(Part::Terrain).expect("its custom terrain");
        let t = &reg.terrains[terrain as usize];
        assert_eq!(t.name, "Mossy stone");
        assert_eq!(states(&reg, &t.strata[0].blocks), [("minecraft:moss_block".to_owned(), 600), ("minecraft:mossy_cobblestone".to_owned(), 400)]);
        assert_eq!(file.left_out, [LeftOut::Annotations("Roads to build".into())]);
    }

    /// Tunnels and custom annotations have no layer in Massif: nothing is
    /// added, and what was left out is said.
    #[test]
    fn tunnels_and_annotations_are_left_out() {
        let vanilla = Registry::vanilla();
        let (reg, file) = read("Mine");
        assert_eq!((file.made.clone(), file.left_out.clone()), (vec![], vec![LeftOut::Tunnel { name: "Mine".into(), floating: false }]));
        assert_eq!(reg, vanilla);
        let (reg, file) = read("Roads to build");
        assert_eq!(file.left_out, [LeftOut::Annotations("Roads to build".into())]);
        assert_eq!(reg, vanilla);
    }

    /// A plant layer imported twice is the same ground cover; a cover of
    /// the map with its name but other plants keeps them, the layer one of
    /// its own; a layer of objects imported twice is told apart by its
    /// name; what is not a layer file is refused, the map left as it was.
    #[test]
    fn layers_are_named_apart_and_other_files_refused() {
        let bytes = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/layers/Meadow.layer")).unwrap();
        let mut reg = Registry::vanilla();
        let first = read_layer_file(&bytes, &mut reg).unwrap();
        let covers = reg.covers.len();
        let second = read_layer_file(&bytes, &mut reg).unwrap();
        assert_eq!((second.made.clone(), reg.covers.len()), (first.made.clone(), covers), "the same cover");
        let pockets = std::fs::read(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data/layers/Gravel pockets.layer")).unwrap();
        for _ in 0..2 {
            read_layer_file(&pockets, &mut reg).unwrap();
        }
        assert_eq!(reg.pockets.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Gravel pockets", "Gravel pockets 2"]);
        // A cover of the map with the layer's name keeps its plants: the
        // layer is one of its own.
        let mut reg = Registry::vanilla();
        let ferns = reg.cover_by_name("Ferns").unwrap();
        reg.covers[ferns as usize - 1].name = "Meadow".into();
        let theirs = reg.clone();
        let file = read_layer_file(&bytes, &mut reg).unwrap();
        assert_eq!(reg.covers[ferns as usize - 1], theirs.covers[ferns as usize - 1]);
        let [Made::Cover(n)] = file.made[..] else { panic!("{file:?}") };
        assert_eq!((n as usize, reg.covers[n as usize - 1].name.as_str()), (reg.covers.len(), "Meadow 2"));
        let before = reg.clone();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        std::io::Write::write_all(&mut gz, b"not a layer").unwrap();
        for bytes in [b"hello".to_vec(), gz.finish().unwrap(), Vec::new()] {
            assert_eq!(read_layer_file(&bytes, &mut reg), Err(LayerFileError::NotALayer));
        }
        assert_eq!(reg, before);
    }

    /// Every `.layer` of the folder `MASSIF_WP_LAYERS` names (layers of
    /// third-party objects: Bo2 trees, schematics, structures) is read, and
    /// a custom object layer brings its objects, blocks and all.
    #[test]
    fn a_folder_of_layers_is_read() {
        let Some(dir) = std::env::var_os("MASSIF_WP_LAYERS") else { return };
        let mut read = 0;
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.extension().is_none_or(|e| e != "layer") {
                continue;
            }
            let mut reg = Registry::vanilla();
            let file = read_layer_file(&std::fs::read(&path).unwrap(), &mut reg).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
            if file.class == "Bo2Layer" {
                let Some(&Made::Objects(n, count)) = file.made.first() else { panic!("{file:?}") };
                assert!(count > 0, "{}", path.display());
                // Objects of air or entities only are said to be left out.
                let layer = &reg.object_layers[n as usize - 1];
                let empty = layer.objects.iter().filter(|(o, _)| reg.objects[*o as usize].blocks.iter().all(|&b| b == 0)).count();
                assert_eq!(empty > 0, file.left_out.contains(&LeftOut::EmptyObjects(layer.name.clone(), empty)), "{}: {file:?}", path.display());
            }
            eprintln!("{}: {:?}", path.display(), file.made);
            read += 1;
        }
        assert!(read > 0, "no .layer file there");
    }
}
