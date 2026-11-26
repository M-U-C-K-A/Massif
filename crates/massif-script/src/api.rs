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
        "addTerrain" => {
            let w = world_of(args)?;
            let reg = &mut s.slot(w)?.world.registry;
            let id = add_terrain(reg, args.get("def").unwrap_or(&Value::Null))?;
            s.slot(w)?.changed = true;
            Ok(json!(id))
        }
        "addCover" => {
            let w = world_of(args)?;
            let reg = &mut s.slot(w)?.world.registry;
            let id = add_cover(reg, args.get("def").unwrap_or(&Value::Null))?;
            s.slot(w)?.changed = true;
            Ok(json!({ "kind": "cover", "id": id }))
        }
        "addObjects" => {
            let w = world_of(args)?;
            let def = args.get("def").unwrap_or(&Value::Null);
            // The files read first, each one checked against what the
            // script may touch.
            let mut read = Vec::new();
            if let Some(files) = def.get("files") {
                let files = files.as_array().ok_or("files is a list of schematics")?;
                for f in files {
                    let name = f.as_str().ok_or("files is a list of schematics, by their paths")?;
                    let path = s.access.check(name, false)?;
                    let imported = massif_objects::read(&path).map_err(|e| format!("{name}: {e}"))?;
                    read.push(imported.object);
                }
                if read.is_empty() {
                    return Err("files lists no schematic: give one or more, or leave files out to retune the layer".into());
                }
            }
            let reg = &mut s.slot(w)?.world.registry;
            let id = add_objects(reg, def, read)?;
            s.slot(w)?.changed = true;
            Ok(json!({ "kind": "trees", "id": id }))
        }
        _ => Err(format!("massif.{name}() is not supported yet")),
    }
}

// ------------------------------------------------------ layers of a map --

fn number(def: &Value, key: &str) -> R<Option<f64>> {
    match def.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{key} is a number")),
    }
}

fn flag(def: &Value, key: &str) -> R<Option<bool>> {
    match def.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_bool().map(Some).ok_or_else(|| format!("{key} is true or false")),
    }
}

/// A block by its id, `minecraft:` taken for one without a namespace:
/// `"podzol"`, `"minecraft:snow[layers=2]"`, `"conquest:silt"`.
fn block_state(name: &str) -> R<String> {
    let name = name.trim();
    if name.is_empty() {
        return Err("a block is named by its id: \"podzol\", \"minecraft:snow[layers=2]\"".into());
    }
    let bare = name.split('[').next().unwrap_or(name);
    Ok(if bare.contains(':') { name.to_owned() } else { format!("minecraft:{name}") })
}

/// Blocks with their weights: `{ "podzol", { "grass_block", 3 } }`, a
/// block alone weighing 1.
fn weighted(list: &Value, what: &str) -> R<Vec<(String, u16)>> {
    let items = list.as_array().ok_or_else(|| format!("{what} is a list of blocks: {{ \"podzol\", {{ \"grass_block\", 3 }} }}"))?;
    items
        .iter()
        .map(|item| match item {
            Value::String(b) => Ok((block_state(b)?, 1)),
            Value::Array(pair) => {
                let b = pair.first().and_then(Value::as_str).ok_or_else(|| format!("each of {what} is a block, or {{ block, weight }}"))?;
                let w = pair.get(1).map_or(Some(1.0), Value::as_f64).ok_or_else(|| format!("the weight of {b} is a number"))?;
                Ok((block_state(b)?, w.round().clamp(1.0, u16::MAX as f64) as u16))
            }
            _ => Err(format!("each of {what} is a block, or {{ block, weight }}")),
        })
        .collect()
}

/// A colour as `{ r, g, b }`, 0 to 255.
fn colour(def: &Value) -> R<Option<[u8; 3]>> {
    match def.get("color") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(c)) if c.len() == 3 && c.iter().all(Value::is_number) => Ok(Some([0, 1, 2].map(|k| c[k].as_f64().unwrap_or(0.0).round().clamp(0.0, 255.0) as u8))),
        Some(_) => Err("color is { r, g, b }, each 0 to 255".into()),
    }
}

/// A range of whole numbers: `n`, or `{ least, most }`.
fn range(def: &Value, key: &str, max: f64) -> R<Option<(f64, f64)>> {
    match def.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Array(r)) if (1..=2).contains(&r.len()) && r.iter().all(Value::is_number) => {
            let lo = r[0].as_f64().unwrap_or(0.0).round().clamp(0.0, max);
            let hi = r.last().and_then(Value::as_f64).unwrap_or(lo).round().clamp(0.0, max);
            Ok(Some((lo.min(hi), lo.max(hi))))
        }
        Some(v) => v.as_f64().map(|n| Some((n.round().clamp(0.0, max), n.round().clamp(0.0, max)))).ok_or_else(|| format!("{key} is a number, or {{ least, most }}")),
    }
}

fn name_of(def: &Value) -> R<String> {
    let name = def.get("name").and_then(Value::as_str).map(str::trim).unwrap_or_default();
    if name.is_empty() { Err("a name is needed".into()) } else { Ok(name.to_owned()) }
}

/// `map:add_terrain{…}`: a terrain made of strata, or the one of that name
/// redefined; its number.
fn add_terrain(reg: &mut massif_core::registry::Registry, def: &Value) -> R<u16> {
    use massif_core::registry::{Mix, Stratum, TerrainDef};
    let name = name_of(def)?;
    let given = def.get("strata").and_then(Value::as_array).filter(|s| !s.is_empty()).ok_or("strata is a list of layers from the surface down: { { blocks = { \"podzol\" }, depth = 1 }, … }")?;
    let mut strata = Vec::with_capacity(given.len());
    for (k, s) in given.iter().enumerate() {
        let blocks = weighted(s.get("blocks").ok_or_else(|| format!("stratum {} has no blocks", k + 1))?, "blocks")?;
        if blocks.is_empty() {
            return Err(format!("stratum {} has no blocks", k + 1));
        }
        let (lo, hi) = range(s, "depth", 4096.0)?.unwrap_or((1.0, 1.0));
        let mix = match s.get("mix").and_then(Value::as_str) {
            None | Some("grain") => Mix::Grain,
            Some("patches") => Mix::Patches { size: number(s, "size")?.unwrap_or(4.0).clamp(1.0, 256.0) as f32 },
            Some("beds") => Mix::Beds,
            Some(other) => return Err(format!("Unknown mix \"{other}\": grain, patches or beds")),
        };
        let blocks = blocks.into_iter().map(|(b, w)| (reg.intern_block(&b, massif_core::objects::guess_color(&b)), w)).collect();
        strata.push(Stratum { blocks, depth: (lo as u16, hi as u16), mix });
    }
    let top = reg.block(strata[0].blocks[0].0).color;
    let existing = reg.terrain_by_name(&name);
    let old = existing.map(|t| reg.terrain(t).clone());
    let terrain = TerrainDef {
        group: def.get("group").and_then(Value::as_str).map(str::to_owned).or_else(|| old.as_ref().map(|o| o.group.clone())).unwrap_or_else(|| "Custom".into()),
        color: colour(def)?.or_else(|| old.as_ref().map(|o| o.color)).unwrap_or(top),
        strata,
        submerged_top: old.as_ref().and_then(|o| o.submerged_top),
        biome_hint: old.as_ref().and_then(|o| o.biome_hint),
        name,
    };
    match existing {
        Some(t) => {
            reg.terrains[t.index()] = terrain;
            Ok(t.0)
        }
        None => {
            if reg.terrains.len() >= u16::MAX as usize {
                return Err("the map holds as many terrains as it can".into());
            }
            reg.terrains.push(terrain);
            Ok((reg.terrains.len() - 1) as u16)
        }
    }
}

/// `map:add_cover{…}`: a ground cover, or the one of that name redefined;
/// its number.
fn add_cover(reg: &mut massif_core::registry::Registry, def: &Value) -> R<u16> {
    use massif_core::registry::{CoverDef, plant_color};
    let name = name_of(def)?;
    let existing = reg.cover_by_name(&name);
    let mut cover = match existing {
        Some(n) => reg.covers[n as usize - 1].clone(),
        None => CoverDef::new(&name, [120, 170, 90], &[], 0.5),
    };
    match def.get("plants") {
        Some(p) => cover.plants = weighted(p, "plants")?,
        None if existing.is_none() => return Err("plants is a list of blocks: { { \"short_grass\", 8 }, { \"poppy\", 1 } }".into()),
        None => {}
    }
    if let Some(d) = number(def, "density")? {
        cover.density = d.clamp(0.0, 1.0) as f32;
    }
    if let Some((lo, hi)) = range(def, "height", 255.0)? {
        cover.height = [lo.max(1.0) as u8, hi.max(1.0) as u8];
    }
    if let Some(v) = flag(def, "soil_only")? {
        cover.soil_only = v;
    }
    if let Some(v) = flag(def, "underwater")? {
        cover.underwater = v;
    }
    match colour(def)? {
        Some(c) => cover.color = c,
        None if existing.is_none() => cover.color = cover.plants.first().map(|p| plant_color(&p.0).unwrap_or_else(|| massif_core::objects::guess_color(&p.0))).unwrap_or(cover.color),
        None => {}
    }
    Ok(match existing {
        Some(n) => {
            reg.covers[n as usize - 1] = cover;
            n
        }
        None => reg.add_cover(cover),
    })
}

/// `map:add_objects{…}`: a layer of the objects read, or the one of that
/// name retuned (its objects kept unless others were read); its number.
fn add_objects(reg: &mut massif_core::registry::Registry, def: &Value, read: Vec<massif_core::objects::ObjectDef>) -> R<u16> {
    use massif_core::objects::{ObjectLayerDef, Spacing};
    let name = name_of(def)?;
    let existing = reg.object_layers.iter().position(|l| !l.removed && l.name.eq_ignore_ascii_case(&name));
    if existing.is_none() && read.is_empty() {
        return Err(format!("This map has no object layer named \"{name}\": give its files to make one"));
    }
    let mut added = Vec::with_capacity(read.len());
    for mut o in read {
        o.colors = o.palette.iter().map(|state| massif_core::objects::guess_color(state)).collect();
        reg.objects.push(o);
        added.push((reg.objects.len() - 1) as u32);
    }
    let mut layer = match existing {
        Some(k) => {
            let mut l = reg.object_layers[k].clone();
            if !added.is_empty() {
                l.objects = added.iter().map(|&o| (o, 1)).collect();
            }
            l
        }
        None => {
            let mut l = ObjectLayerDef::new(&name, &added, reg);
            // As the app makes a layer of objects imported: spaced by
            // their canopies.
            l.spacing_mode = Spacing::Canopy;
            l
        }
    };
    match def.get("spacing") {
        None | Some(Value::Null) => {}
        Some(Value::String(s)) if s == "canopy" => layer.spacing_mode = Spacing::Canopy,
        Some(v) => {
            let n = v.as_f64().ok_or("spacing is \"canopy\", or a number of blocks")?;
            layer.spacing_mode = Spacing::Grid;
            layer.spacing = n.round().clamp(1.0, 256.0) as u16;
        }
    }
    if let Some(d) = number(def, "density")? {
        layer.density = d.clamp(0.0, 1.0) as f32;
    }
    if let Some(g) = number(def, "gap")? {
        layer.min_gap = g.round().clamp(0.0, 255.0) as u8;
    }
    if let Some(s) = number(def, "sink")? {
        layer.sink = s.round().clamp(-256.0, 256.0) as i16;
    }
    for (key, slot) in [("rotate", &mut layer.rotate), ("mirror", &mut layer.mirror), ("soil_only", &mut layer.soil_only), ("replace", &mut layer.replace), ("understory", &mut layer.understory), ("underwater", &mut layer.underwater)] {
        if let Some(v) = flag(def, key)? {
            *slot = v;
        }
    }
    if let Some(c) = colour(def)? {
        layer.color = c;
    }
    Ok(match existing {
        Some(k) => {
            reg.object_layers[k] = layer;
            k as u16 + 1
        }
        None => reg.add_object_layer(layer),
    })
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
