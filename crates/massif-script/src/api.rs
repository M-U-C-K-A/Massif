//! Massif's own scripting API (`massif.js`): the calls that take and give
//! JSON, by name. Columns in bulk go through [`crate::region`].

use crate::session::{R, Session};
use massif_core::coords::{BlockRect, TILE_SIZE};
use serde_json::{Value, json};

fn int(args: &Value, key: &str) -> Option<i64> {
    args.get(key).and_then(Value::as_f64).map(|v| v as i64)
}

fn world_of(args: &Value) -> R<usize> {
    int(args, "world").map(|w| w as usize).ok_or_else(|| "a world is needed: massif.open(…), massif.create(…) or the map open in Massif".into())
}

/// `"minecraft:podzol"`, `"podzol"` or `{"minecraft:podzol", 3}`: the block
/// state and its weight.
fn block_and_weight(v: &Value) -> R<(String, u16)> {
    let (state, weight) = match v {
        Value::String(s) => (s.as_str(), 1),
        Value::Array(a) => (a.first().and_then(Value::as_str).ok_or("a block is \"minecraft:name[state]\" or {\"minecraft:name\", weight}")?, a.get(1).and_then(Value::as_u64).unwrap_or(1)),
        _ => return Err("a block is \"minecraft:name[state]\" or {\"minecraft:name\", weight}".into()),
    };
    let state = state.trim();
    if state.is_empty() {
        return Err("an empty block name".into());
    }
    let state = if state.contains(':') { state.to_owned() } else { format!("minecraft:{state}") };
    Ok((state, weight.clamp(1, u16::MAX as u64) as u16))
}

/// `color = {r, g, b}`, if given.
fn color_of(args: &Value) -> Option<[u8; 3]> {
    let c = args.get("color")?.as_array()?;
    let n = |i: usize| c.get(i).and_then(Value::as_u64).map(|v| v.min(255) as u8);
    Some([n(0)?, n(1)?, n(2)?])
}

fn text<'a>(args: &'a Value, key: &str) -> R<&'a str> {
    args.get(key).and_then(Value::as_str).ok_or_else(|| format!("{key} is needed"))
}

/// The settings of a pass over the whole map, by name, as Edit ▸ Whole map
/// starts them for a map `side` blocks across with its sea at `sea`.
fn whole_op(name: &str, sea: f32, side: f32, options: &Value) -> R<massif_core::graph::Reshape> {
    use massif_core::graph::{NodeKind, Reshape};
    let f = (side / 2048.0).clamp(0.5, 3.0);
    let kind = match name {
        "erosion" => NodeKind::Erosion { rain: 1.0, erosion: 0.3, deposition: 0.3, capacity: 4.0, evaporation: 0.02, radius: 4.0, seed: 5 },
        "thermalErosion" | "weathering" => NodeKind::Thermal { talus: 40.0, iterations: 40, strength: 0.5 },
        "smooth" => NodeKind::Blur { radius: 3.0 },
        "sharpen" => NodeKind::Sharpen { radius: 24.0, amount: 0.5 },
        "terraces" => NodeKind::Terrace { steps: 12, strength: 0.6 },
        "strata" => NodeKind::Stratify { layers: 14, strength: 0.6, tilt: 6.0, seed: 5 },
        "lakes" => NodeKind::Lakes { amount: 0.0, sea },
        "rivers" => NodeKind::Rivers { threshold: 0.5, depth: 4.0, width: 10.0 * f.clamp(1.0, 2.5), sea, banks: 1.0, lakes: 0.5 },
        "scaleAndRaise" | "relief" => {
            let scale = options.get("scale").and_then(Value::as_f64).unwrap_or(1.0) as f32;
            let shift = options.get("shift").and_then(Value::as_f64).unwrap_or(0.0) as f32;
            return Ok(Reshape::Relief { scale, shift });
        }
        _ => return Err(format!("Unknown whole-map operation \"{name}\": erosion, thermalErosion, smooth, sharpen, terraces, strata, lakes, rivers or scaleAndRaise")),
    };
    // The options over the defaults, by the names of the node's settings.
    let mut value = serde_json::to_value(&kind).map_err(|e| e.to_string())?;
    if let (Some(fields), Some(given)) = (value.as_object_mut().and_then(|o| o.values_mut().next()).and_then(Value::as_object_mut), options.as_object()) {
        for (k, v) in given {
            match fields.get_mut(k) {
                Some(slot) => *slot = v.clone(),
                None => return Err(format!("{name} has no setting \"{k}\": {}", fields.keys().cloned().collect::<Vec<_>>().join(", "))),
            }
        }
    }
    let kind: NodeKind = serde_json::from_value(value).map_err(|e| format!("{name}: {e}"))?;
    Ok(Reshape::Filter(kind))
}

/// Call `name` with `args`.
pub fn call(s: &mut Session, name: &str, args: &Value) -> R<Value> {
    match name {
        "open" => Ok(json!(s.open_world(text(args, "file")?)?)),
        "create" => {
            let mut params = serde_json::to_value(massif_core::generate::GenParams::default()).map_err(|e| e.to_string())?;
            let given = args.get("options").cloned().unwrap_or(Value::Null);
            if let (Some(p), Some(g)) = (params.as_object_mut(), given.as_object()) {
                for (k, v) in g {
                    match k.as_str() {
                        "width" => p.insert("tiles_x".into(), json!(((v.as_f64().unwrap_or(1024.0) / TILE_SIZE as f64).ceil() as i64).max(1))),
                        "depth" => p.insert("tiles_z".into(), json!(((v.as_f64().unwrap_or(1024.0) / TILE_SIZE as f64).ceil() as i64).max(1))),
                        "seaLevel" => p.insert("sea_level".into(), json!(v.as_f64().unwrap_or(62.0) as i64 + 1)),
                        "minY" => p.insert("min_y".into(), v.clone()),
                        "maxY" => p.insert("max_y".into(), v.clone()),
                        "featureSize" => p.insert("feature_size".into(), v.clone()),
                        "flatHeight" => p.insert("flat_height".into(), json!(v.as_f64().unwrap_or(64.0) + 1.0)),
                        "landform" => p.insert("landform".into(), json!(capitalised(v.as_str().unwrap_or("Hills")))),
                        other => p.insert(other.into(), v.clone()),
                    };
                }
            }
            let params: massif_core::generate::GenParams = serde_json::from_value(params).map_err(|e| format!("massif.create(): {e}"))?;
            if params.tiles_x > 4096 || params.tiles_z > 4096 {
                return Err("massif.create(): at most 524,288 blocks a side".into());
            }
            let world = massif_core::generate::generate(&params);
            let w = s.add_world(world, None);
            s.slot(w)?.changed = true;
            Ok(json!(w))
        }
        "save" => {
            let w = world_of(args)?;
            Ok(json!(s.save_world(w, text(args, "file")?)?.display().to_string()))
        }
        "export" => {
            let w = world_of(args)?;
            s.flush()?;
            let dir = s.access.check(text(args, "dir")?, true)?;
            let mut options = massif_anvil::ExportOptions { overwrite: args.get("overwrite").and_then(Value::as_bool).unwrap_or(false), ..Default::default() };
            if let Some(v) = args.get("version").and_then(Value::as_str) {
                options.target = massif_anvil::Target::release(v).ok_or_else(|| format!("Massif does not export for Minecraft {v}"))?;
            }
            let slot = s.settle(w)?;
            let stats = massif_anvil::export(&slot.world, &dir, &options, &massif_anvil::Progress::default()).map_err(|e| e.to_string())?;
            Ok(json!({ "dir": dir.display().to_string(), "chunks": stats.chunks, "regions": stats.regions }))
        }
        "info" => {
            let w = world_of(args)?;
            s.flush()?;
            let slot = s.slot(w)?;
            let world = &slot.world;
            let dims: Vec<Value> = world
                .dims
                .iter()
                .map(|d| {
                    let bounds = d.bounds().map(|(lo, hi)| [lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE]);
                    json!({ "kind": d.kind.label(), "minY": d.min_y, "maxY": d.max_y, "bounds": bounds, "tiles": d.ground().len(), "selection": d.selection.as_ref().and_then(|s| s.bounds()).map(|b| [b.x0, b.z0, b.x1, b.z1]) })
                })
                .collect();
            Ok(json!({ "name": world.meta.name, "seed": world.meta.seed, "seaLevel": world.meta.sea_level - 1, "spawn": world.meta.spawn, "lava": world.meta.lava, "dims": dims, "file": slot.path.as_ref().map(|p| p.display().to_string()) }))
        }
        "set" => {
            let w = world_of(args)?;
            let slot = s.slot(w)?;
            let meta = &mut slot.world.meta;
            if let Some(n) = args.get("name").and_then(Value::as_str) {
                meta.name = n.to_owned();
            }
            if let Some(v) = int(args, "seaLevel") {
                meta.sea_level = v as i32 + 1;
            }
            if let Some(v) = int(args, "seed") {
                meta.seed = v;
            }
            if let Some(sp) = args.get("spawn").and_then(Value::as_array) {
                let c = |k: usize| sp.get(k).and_then(Value::as_f64).unwrap_or(0.0) as i32;
                meta.spawn = [c(0), c(1), c(2)];
            }
            slot.changed = true;
            Ok(Value::Null)
        }
        "terrains" => {
            let w = world_of(args)?;
            let reg = &s.slot(w)?.world.registry;
            Ok(json!(reg.terrains.iter().enumerate().map(|(k, t)| json!({ "id": k, "name": t.name, "group": t.group })).collect::<Vec<_>>()))
        }
        "biomes" => {
            let w = world_of(args)?;
            let reg = &s.slot(w)?.world.registry;
            Ok(json!(reg.biomes.iter().enumerate().map(|(k, b)| json!({ "id": k, "key": b.id, "name": b.name })).collect::<Vec<_>>()))
        }
        "layers" => {
            let w = world_of(args)?;
            let reg = &s.slot(w)?.world.registry;
            let list = |names: Vec<(usize, &str, bool)>| names.into_iter().filter(|n| !n.2).map(|(k, n, _)| json!({ "id": k + 1, "name": n })).collect::<Vec<_>>();
            Ok(json!({
                "cover": list(reg.covers.iter().enumerate().map(|(k, c)| (k, c.name.as_str(), c.removed)).collect()),
                "trees": list(reg.object_layers.iter().enumerate().map(|(k, c)| (k, c.name.as_str(), c.removed)).collect()),
                "pockets": list(reg.pockets.iter().enumerate().map(|(k, c)| (k, c.name.as_str(), c.removed)).collect()),
                "combined": list(reg.combined.iter().enumerate().map(|(k, c)| (k, c.name.as_str(), c.removed)).collect()),
            }))
        }
        "terrainId" => {
            let w = world_of(args)?;
            let n = text(args, "name")?;
            s.slot(w)?.world.registry.terrain_by_name(n).map(|t| json!(t.0)).ok_or_else(|| format!("This map has no terrain named \"{n}\""))
        }
        "biomeId" => {
            let w = world_of(args)?;
            let n = text(args, "name")?;
            let reg = &s.slot(w)?.world.registry;
            let key = if n.contains(':') { n.to_owned() } else { format!("minecraft:{}", n.to_ascii_lowercase().replace(' ', "_")) };
            reg.biomes.iter().position(|b| b.id == key || b.name.eq_ignore_ascii_case(n)).map(|k| json!(k)).ok_or_else(|| format!("This map has no biome \"{n}\""))
        }
        "addTerrain" => {
            let w = world_of(args)?;
            let name = text(args, "name")?.trim().to_owned();
            if name.is_empty() {
                return Err("A terrain needs a name".into());
            }
            let layers = args.get("strata").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or(
                "strata is needed: its layers from the surface down, each { blocks = { {\"minecraft:grass_block\", 3}, {\"minecraft:podzol\", 1} }, depth = {1, 1}, mix = \"grain\" | \"patches\" | \"beds\", size = 4 }",
            )?;
            let reg = &mut s.slot(w)?.world.registry;
            let mut strata = Vec::new();
            let mut colors = Vec::new();
            for (k, layer) in layers.iter().enumerate() {
                let at = |what: &str| format!("strata[{}]: {what}", k + 1);
                let blocks = layer.get("blocks").and_then(Value::as_array).filter(|b| !b.is_empty()).ok_or_else(|| at("blocks is needed, a list of {block, weight}"))?;
                let mut weighted = Vec::new();
                for b in blocks {
                    let (state, weight) = block_and_weight(b).map_err(|e| at(&e))?;
                    let color = massif_core::objects::guess_color(&state);
                    if k == 0 {
                        colors.push((color, weight));
                    }
                    weighted.push((reg.intern_block(&state, color), weight));
                }
                let depth = match layer.get("depth") {
                    None => (1, 1),
                    Some(Value::Array(d)) => {
                        let n = |i: usize| d.get(i).and_then(Value::as_u64).map(|v| v.clamp(0, 512) as u16);
                        let lo = n(0).ok_or_else(|| at("depth is {least, most} or a number"))?;
                        (lo, n(1).unwrap_or(lo).max(lo))
                    }
                    Some(v) => {
                        let d = v.as_u64().ok_or_else(|| at("depth is {least, most} or a number"))?.clamp(0, 512) as u16;
                        (d, d)
                    }
                };
                let mix = match layer.get("mix").and_then(Value::as_str).unwrap_or("grain") {
                    "grain" => massif_core::registry::Mix::Grain,
                    "patches" | "blobs" => massif_core::registry::Mix::Patches { size: layer.get("size").and_then(Value::as_f64).unwrap_or(4.0).clamp(1.0, 256.0) as f32 },
                    "beds" => massif_core::registry::Mix::Beds,
                    other => return Err(at(&format!("mix \"{other}\": grain, patches or beds"))),
                };
                strata.push(massif_core::registry::Stratum { blocks: weighted, depth, mix });
            }
            let color = match color_of(args) {
                Some(c) => c,
                None => {
                    let total: f32 = colors.iter().map(|c| c.1 as f32).sum();
                    let mean = |i: usize| (colors.iter().map(|c| c.0[i] as f32 * c.1 as f32).sum::<f32>() / total.max(1.0)) as u8;
                    [mean(0), mean(1), mean(2)]
                }
            };
            let group = args.get("group").and_then(Value::as_str).unwrap_or("Custom").to_owned();
            let def = massif_core::registry::TerrainDef { name: name.clone(), group, color, strata, submerged_top: None, biome_hint: None };
            // One of the same name is redefined (the built-in Grass made a
            // mix, say): what is painted with it changes with it.
            let id = match reg.terrain_by_name(&name) {
                Some(t) => {
                    let old = &mut reg.terrains[t.0 as usize];
                    let (submerged_top, biome_hint) = (old.submerged_top, old.biome_hint);
                    *old = massif_core::registry::TerrainDef { submerged_top, biome_hint, ..def };
                    t.0
                }
                None => {
                    if reg.terrains.len() >= u16::MAX as usize {
                        return Err("The map has as many terrains as it can hold".into());
                    }
                    reg.terrains.push(def);
                    (reg.terrains.len() - 1) as u16
                }
            };
            s.slot(w)?.changed = true;
            Ok(json!(id))
        }
        "addCover" => {
            let w = world_of(args)?;
            let name = text(args, "name")?.trim().to_owned();
            let plants = args.get("plants").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or("plants is needed: { {\"minecraft:short_grass\", 10}, {\"minecraft:fern\", 2} }")?;
            let mut weighted = Vec::new();
            for p in plants {
                weighted.push(block_and_weight(p)?);
            }
            let total: f32 = weighted.iter().map(|p| p.1 as f32).sum();
            let mean = |i: usize| (weighted.iter().map(|p| massif_core::objects::guess_color(&p.0)[i] as f32 * p.1 as f32).sum::<f32>() / total.max(1.0)) as u8;
            let color = color_of(args).unwrap_or([mean(0), mean(1), mean(2)]);
            let borrowed: Vec<(&str, u16)> = weighted.iter().map(|(s, w)| (s.as_str(), *w)).collect();
            let density = args.get("density").and_then(Value::as_f64).unwrap_or(0.5).clamp(0.0, 1.0) as f32;
            let mut def = massif_core::registry::CoverDef::new(&name, color, &borrowed, density);
            if let Some(h) = args.get("height").and_then(Value::as_array) {
                let n = |i: usize| h.get(i).and_then(Value::as_u64).map(|v| v.clamp(1, 16) as u8);
                let lo = n(0).unwrap_or(1);
                def.height = [lo, n(1).unwrap_or(lo).max(lo)];
            }
            if let Some(b) = args.get("soil_only").and_then(Value::as_bool) {
                def.soil_only = b;
            }
            if let Some(b) = args.get("underwater").and_then(Value::as_bool) {
                def.underwater = b;
            }
            let reg = &mut s.slot(w)?.world.registry;
            let id = match reg.cover_by_name(&name) {
                Some(n) => {
                    reg.covers[n as usize - 1] = def;
                    n
                }
                None => reg.add_cover(def),
            };
            s.slot(w)?.changed = true;
            Ok(json!({ "kind": "cover", "id": id }))
        }
        "addObjects" => {
            let w = world_of(args)?;
            let name = text(args, "name")?.trim().to_owned();
            let files = args.get("files").and_then(Value::as_array).filter(|a| !a.is_empty()).ok_or("files is needed: the schematics (.schem, .schematic, .nbt, .litematic, .bo2, .bo3, Axiom .bp) of the layer")?;
            let mut read = Vec::new();
            for f in files {
                let f = f.as_str().ok_or("files: names of files")?;
                let path = s.access.check(f, false)?;
                let weight = 1;
                let mut imported = massif_objects::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let o = &mut imported.object;
                o.colors = o.palette.iter().map(|state| massif_core::objects::guess_color(state)).collect();
                read.push((imported.object, weight));
            }
            let reg = &mut s.slot(w)?.world.registry;
            let mut added = Vec::new();
            for (o, _) in read {
                reg.objects.push(o);
                added.push((reg.objects.len() - 1) as u32);
            }
            let mut layer = massif_core::objects::ObjectLayerDef::new(&name, &added, reg);
            layer.spacing_mode = massif_core::objects::Spacing::Canopy;
            if let Some(c) = color_of(args) {
                layer.color = c;
            }
            if let Some(d) = args.get("density").and_then(Value::as_f64) {
                layer.density = d.clamp(0.0, 1.0) as f32;
            }
            match args.get("spacing") {
                Some(Value::String(m)) if m == "canopy" => {}
                Some(v) if v.as_u64().is_some() => {
                    layer.spacing_mode = massif_core::objects::Spacing::Grid;
                    layer.spacing = v.as_u64().unwrap_or(8).clamp(1, 256) as u16;
                }
                None => {}
                Some(_) => return Err("spacing is \"canopy\" (by their crowns) or a number of blocks (a grid)".into()),
            }
            if let Some(g) = args.get("gap").and_then(Value::as_u64) {
                layer.min_gap = g.clamp(0, 64) as u8;
            }
            for (key, slot) in [("rotate", &mut layer.rotate), ("mirror", &mut layer.mirror), ("soil_only", &mut layer.soil_only), ("replace", &mut layer.replace), ("understory", &mut layer.understory)] {
                if let Some(b) = args.get(key).and_then(Value::as_bool) {
                    *slot = b;
                }
            }
            if let Some(k) = args.get("sink").and_then(Value::as_i64) {
                layer.sink = k.clamp(-64, 64) as i16;
            }
            let understory = layer.understory;
            let found = reg.object_layers.iter().position(|l| !l.removed && l.name.eq_ignore_ascii_case(&name));
            let id = match found {
                Some(k) => {
                    reg.object_layers[k] = layer;
                    k as u16 + 1
                }
                None => reg.add_object_layer(layer),
            };
            s.slot(w)?.changed = true;
            Ok(json!({ "kind": if understory { "understory" } else { "trees" }, "id": id, "objects": added.len() }))
        }
        "layerId" => {
            let w = world_of(args)?;
            let n = text(args, "name")?;
            let reg = &s.slot(w)?.world.registry;
            let find = |names: Vec<(&str, bool)>| names.into_iter().position(|(m, removed)| !removed && m.eq_ignore_ascii_case(n)).map(|k| k + 1);
            let r = find(reg.covers.iter().map(|c| (c.name.as_str(), c.removed)).collect())
                .map(|k| json!({ "kind": "cover", "id": k }))
                .or_else(|| find(reg.object_layers.iter().map(|c| (c.name.as_str(), c.removed)).collect()).map(|k| json!({ "kind": "trees", "id": k })))
                .or_else(|| find(reg.pockets.iter().map(|c| (c.name.as_str(), c.removed)).collect()).map(|k| json!({ "kind": "pockets", "id": k })));
            r.ok_or_else(|| format!("This map has no layer named \"{n}\""))
        }
        "select" => {
            let w = world_of(args)?;
            let dim = int(args, "dim").unwrap_or(0) as i32;
            let (slot, d) = s.column(w, dim)?;
            let sel = match args.get("rect").and_then(Value::as_array) {
                None => None,
                Some(r) => {
                    let c = |k: usize| r.get(k).and_then(Value::as_f64).unwrap_or(0.0) as i32;
                    let rect = BlockRect::new(c(0), c(1), c(2), c(3));
                    Some(std::sync::Arc::new(massif_core::selection::Selection::rasterize(rect, |_, _, row| row.fill(255))))
                }
            };
            slot.world.dims[d].selection = sel;
            slot.changed = true;
            Ok(Value::Null)
        }
        "wholeMap" => {
            let w = world_of(args)?;
            let dim = int(args, "dim").unwrap_or(0) as i32;
            let op_name = text(args, "op")?.to_owned();
            let options = args.get("options").cloned().unwrap_or(Value::Null);
            let io = s.io.clone();
            let (slot, d) = s.column(w, dim)?;
            let sea = slot.world.meta.sea_level;
            let side = slot.world.dims[d].bounds().map(|(lo, hi)| ((hi.x - lo.x + 1).max(hi.z - lo.z + 1) * TILE_SIZE) as f32).unwrap_or(1024.0);
            let op = whole_op(&op_name, sea as f32, side, &options)?;
            let bed = slot.world.registry.terrain_by_name("Gravel");
            let shore = slot.world.registry.terrain_by_name("Sand");
            let steps = massif_core::auto::Steps::default();
            let mut history = massif_core::history::History::new(1);
            history.begin(op_name.clone(), d);
            let world = &mut slot.world;
            let changes = std::thread::scope(|sc| {
                let job = sc.spawn(|| massif_core::graph::reshape_ground(&mut history.edit(world, 0), &op, sea, bed, shore, &steps));
                while !job.is_finished() {
                    if io.cancelled() {
                        steps.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    io.progress.set(steps.fraction(), &op_name);
                    std::thread::sleep(std::time::Duration::from_millis(50));
                }
                job.join().expect("a whole-map pass does not panic")
            });
            drop(history);
            if changes.cancelled {
                return Err(crate::session::CANCELLED.into());
            }
            slot.changed = true;
            Ok(json!({ "columns": changes.columns, "tiles": changes.tiles.len() }))
        }
        "importMask" => {
            let w = world_of(args)?;
            let dim = int(args, "dim").unwrap_or(0) as i32;
            let file = text(args, "file")?;
            let o = args.get("options").cloned().unwrap_or(Value::Null);
            let channel = match o.get("channel").and_then(Value::as_str) {
                None => None,
                Some("red") => Some(0),
                Some("green") => Some(1),
                Some("blue") => Some(2),
                Some(other) => return Err(format!("Unknown channel \"{other}\": red, green or blue")),
            };
            let h = s.load_heightmap(file, channel)?;
            let bits = s.heightmaps[h].header.bits.min(16);
            let white = ((1u32 << bits) - 1) as i32;
            let (native, grey) = native_mask(&o, white)?;
            let num = |k: &str| o.get(k).and_then(Value::as_f64);
            s.queue(crate::apply::MapOp {
                world: w,
                dim,
                ceiling: false,
                height_map: Some(h),
                scale: (num("scale").unwrap_or(1.0) * 100.0).round() as i32,
                dx: num("x").unwrap_or(0.0) as i32,
                dy: num("z").unwrap_or(0.0) as i32,
                grey,
                colour: Vec::new(),
                layer: None,
                terrain: -1,
                layer_value: 0,
                mode: crate::apply::Mode::Set,
                filter: None,
                line: 0,
                call: "import_mask".into(),
                native: Some(native),
            })?;
            Ok(Value::Null)
        }
        _ => Err(format!("massif.{name}() is not supported yet")),
    }
}

/// What `map:import_mask()` paints, and the mapping of the image's levels
/// (0 to `white`) that does it: where a pixel is on (`threshold`, 0 to 1 of
/// white, 0.5 by default; `invert` for the dark ones), the target's value;
/// for heights, `black` to `white` across the levels.
fn native_mask(o: &Value, white: i32) -> R<(crate::apply::Native, Vec<[i32; 4]>)> {
    use crate::apply::{HEIGHT_OFFSET, Native};
    let num = |k: &str| o.get(k).and_then(Value::as_f64);
    if let Some(m) = o.get("mode").and_then(Value::as_str).filter(|m| *m != "replace") {
        return Err(format!("map:import_mask() with mode \"{m}\" is not supported yet: it paints where the image is on"));
    }
    let paint = o.get("paint").and_then(Value::as_str).unwrap_or("annotations");
    let value = || num("value").map(|v| v as i64).ok_or_else(|| format!("A mask painting {paint} needs a value: which one"));
    let id = |v: i64| u16::try_from(v).map_err(|_| format!("{v} is not one of the map's"));
    let native = match paint {
        "annotations" => Native::Annotations(value()?.clamp(1, 15) as u16),
        "terrain" => Native::Terrain(id(value()?)?),
        "biome" => Native::Biome(id(value()?)?),
        "cover" => Native::Cover(id(value()?)?),
        "trees" => Native::Trees(id(value()?)?),
        "understory" => Native::Understory(id(value()?)?),
        "pockets" => Native::Pockets(id(value()?)?),
        "sizes" => Native::Sizes(id(value()?)?.clamp(1, 4)),
        "water" => Native::Water((value()? + 1).clamp(i16::MIN as i64 + 1, i16::MAX as i64) as i16),
        "height" => {
            let (black, white_h) = (num("black").unwrap_or(0.0).round() as i32, num("white").unwrap_or(255.0).round() as i32);
            let from = if o.get("black_is_none").and_then(Value::as_bool).unwrap_or(false) { 1 } else { 0 };
            return Ok((Native::Height, vec![[from, white, black + HEIGHT_OFFSET, white_h + HEIGHT_OFFSET]]));
        }
        other => return Err(format!("A mask cannot paint \"{other}\": annotations, terrain, biome, cover, trees, understory, pockets, sizes, water or height")),
    };
    let t = match o.get("threshold") {
        Some(Value::Bool(false)) => return Err("map:import_mask() with threshold = false (a strength scattered over the columns) is not supported yet: give a threshold".into()),
        Some(v) => v.as_f64().ok_or("threshold is a number from 0 to 1")?,
        None => 0.5,
    };
    let at = ((t.clamp(0.0, 1.0) * white as f64).ceil() as i32).max(if t > 0.0 { 1 } else { 0 });
    let rule = if o.get("invert").and_then(Value::as_bool).unwrap_or(false) { [0, white - at, 1, 1] } else { [at, white, 1, 1] };
    Ok((native, vec![rule]))
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().collect::<String>() + &c.as_str().to_ascii_lowercase()).unwrap_or_default()
}
