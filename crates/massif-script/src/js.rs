//! The JavaScript side: a QuickJS runtime per run, the natives `wp.js` and
//! `massif.js` are written over (an object `__m` they take and hide), and
//! the run itself: bindings, the script, its errors with their line.
//!
//! Natives find the run's [`Session`] through a thread-local, so that each
//! is a plain function taking numbers and strings: the calls a script's
//! loops make cost about a tenth of a microsecond. Errors become
//! JavaScript exceptions with the session's message, thrown where the
//! script called, so that their stack names its line.

use crate::session::{CANCELLED, Line, R, Session};
use rquickjs::{Ctx, Exception, Function, Object, TypedArray, Value};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
    static SCRIPT: RefCell<String> = const { RefCell::new(String::new()) };
}

type Q<T> = rquickjs::Result<T>;

/// What WorldPainter's errors are, as a script's `print(e)` shows them.
pub const SCRIPT_EXCEPTION: &str = "org.pepsoft.worldpainter.tools.scripts.ScriptException";

/// Throw `message` into the script, as WorldPainter's `ScriptException`.
fn throw(ctx: &Ctx<'_>, message: &str) -> rquickjs::Error {
    match Exception::from_message(ctx.clone(), message) {
        Ok(e) => {
            let _ = e.as_object().set("name", SCRIPT_EXCEPTION);
            ctx.throw(e.into_value())
        }
        Err(e) => e,
    }
}

/// Run `f` on the session, its error thrown into the script.
fn with<T>(ctx: &Ctx<'_>, f: impl FnOnce(&mut Session) -> R<T>) -> Q<T> {
    let s = SESSION.with(|s| s.borrow().clone()).expect("a script runs");
    let out = f(&mut s.borrow_mut());
    out.map_err(|e| throw(ctx, &e))
}

fn json<T: serde::de::DeserializeOwned>(text: &str) -> R<T> {
    serde_json::from_str(text).map_err(|e| format!("Massif could not read what the script gave: {e}"))
}

// The natives, in the order of wp.js.

fn script_name() -> String {
    SCRIPT.with(|s| s.borrow().clone())
}

fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The terrains as scripts number them (`Terrain.VALUES`), with each
/// one's ordinal in WorldPainter's enum.
fn terrain_names() -> String {
    let ordinals: Vec<i32> = (0..crate::session::VALUES.len() as i32).map(|k| crate::session::ordinal_of_index(k).map_or(-1, |o| o as i32)).collect();
    serde_json::json!({ "names": crate::session::VALUES.as_slice(), "ordinals": ordinals }).to_string()
}

fn terrain_name(ctx: Ctx<'_>, w: f64, id: f64) -> Q<String> {
    with(&ctx, |s| Ok(s.slot(w as usize)?.world.registry.terrains.get(id as usize).map_or_else(|| format!("Terrain {id}"), |t| t.name.clone())))
}

fn print(ctx: Ctx<'_>, text: String) -> Q<()> {
    with(&ctx, |s| {
        s.print(Line::Out(text));
        Ok(())
    })
}

fn print_warning(ctx: Ctx<'_>, text: String) -> Q<()> {
    with(&ctx, |s| {
        s.print(Line::Warn(text));
        Ok(())
    })
}

fn print_error(ctx: Ctx<'_>, text: String) -> Q<()> {
    with(&ctx, |s| {
        s.print(Line::Error(text));
        Ok(())
    })
}

fn cancelled(ctx: Ctx<'_>) -> Q<bool> {
    with(&ctx, |s| Ok(s.io.cancelled()))
}

fn layer_info(ctx: Ctx<'_>, l: f64) -> Q<String> {
    with(&ctx, |s| {
        let def = s.layer(l as usize)?;
        Ok(serde_json::json!({ "name": def.name, "size": def.size.name() }).to_string())
    })
}

fn layer_by_name(ctx: Ctx<'_>, name: String) -> Q<f64> {
    with(&ctx, |s| s.standard_layer(&name).map(|l| l as f64))
}

fn layer_from_world(ctx: Ctx<'_>, w: f64, name: String) -> Q<f64> {
    with(&ctx, |s| s.world_layer(w as usize, &name).map(|l| l as f64))
}

fn layer_from_file(ctx: Ctx<'_>, file: String) -> Q<f64> {
    with(&ctx, |s| s.layer_file(&file).map(|l| l as f64))
}

fn world_info(ctx: Ctx<'_>, w: f64) -> Q<String> {
    with(&ctx, |s| {
        let slot = s.slot(w as usize)?;
        let world = &slot.world;
        let dims: Vec<i32> = world.dims.iter().map(|d| d.kind as i32).collect();
        let ground = world.dims.first();
        Ok(serde_json::json!({
            "name": world.meta.name,
            "dims": dims,
            "minHeight": ground.map_or(-64, |d| d.min_y),
            "maxHeight": ground.map_or(320, |d| d.max_y),
            "seaLevel": world.meta.sea_level - 1,
            "spawn": world.meta.spawn,
            "seed": world.meta.seed,
            "format": slot.format.clone().unwrap_or_else(|| "org.pepsoft.anvil.1.18".into()),
        })
        .to_string())
    })
}

fn set_world_name(ctx: Ctx<'_>, w: f64, name: String) -> Q<()> {
    with(&ctx, |s| {
        let slot = s.slot(w as usize)?;
        slot.world.meta.name = name;
        slot.changed = true;
        Ok(())
    })
}

fn set_spawn(ctx: Ctx<'_>, w: f64, x: f64, y: f64) -> Q<()> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, 0)?;
        let h = slot.height(d, x as i32, y as i32);
        slot.world.meta.spawn = [x as i32, if h > -1e30 { h as i32 + 1 } else { 80 }, y as i32];
        slot.changed = true;
        Ok(())
    })
}

fn map_format(ctx: Ctx<'_>, id: Option<String>, name: Option<String>) -> Q<String> {
    with(&ctx, |_| {
        let (id, name) = crate::worlds::map_format(id.as_deref(), name.as_deref())?;
        Ok(serde_json::json!({ "id": id, "name": name }).to_string())
    })
}

fn set_format(ctx: Ctx<'_>, w: f64, id: String) -> Q<()> {
    with(&ctx, |s| {
        crate::worlds::map_format(Some(&id), None)?;
        s.slot(w as usize)?.format = Some(id);
        Ok(())
    })
}

fn has_dimension(ctx: Ctx<'_>, w: f64, d: f64) -> Q<bool> {
    with(&ctx, |s| Ok(s.slot(w as usize)?.dim_index(d as i32).is_ok()))
}

fn height(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        Ok(slot.height(d, x as i32, y as i32) as f64)
    })
}

fn raw_height(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let min = slot.world.dims[d].min_y;
        let h = slot.height(d, x as i32, y as i32);
        // WorldPainter's raw heights: 1/256ths of a block over the floor.
        Ok(if h <= -1e30 { 0.0 } else { ((h - min as f32) * 256.0).round().max(0.0) as f64 })
    })
}

fn int_height(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64, default: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        Ok(slot.read(d, x as i32, y as i32, |t, i| crate::session::java_round(t.top.get(i) - 1.0) as f64).unwrap_or(default))
    })
}

fn set_height(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64, h: f64) -> Q<()> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let (min, max) = (slot.world.dims[d].min_y as f32, slot.world.dims[d].max_y as f32);
        let top = (h as f32 + 1.0).clamp(min, max);
        slot.write(d, x as i32, y as i32, |t, i| t.top.set(i, top));
        Ok(())
    })
}

fn water(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        Ok(slot.water(d, x as i32, y as i32) as f64)
    })
}

fn set_water(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64, level: f64) -> Q<()> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let min = slot.world.dims[d].min_y;
        let level = level as i32;
        let fluid = if level < min { massif_core::tile::NO_FLUID } else { (level + 1).clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16 };
        slot.write(d, x as i32, y as i32, |t, i| t.fluid.set(i, fluid));
        Ok(())
    })
}

fn lava(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<bool> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let reader_lava = slot.world.meta.lava;
        let covers: Vec<bool> = slot.world.registry.covers.iter().map(|c| c.lava && !c.removed).collect();
        Ok(reader_lava || slot.read(d, x as i32, y as i32, |t, i| covers.get((t.cover.get(i) as usize).wrapping_sub(1)).copied().unwrap_or(false)).unwrap_or(false))
    })
}

fn slope(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let mut h = |x: i32, z: i32| slot.height(d, x, z);
        let (x, z) = (x as i32, y as i32);
        // WorldPainter's Dimension.getSlope.
        let (e, wv, s_, n) = (h(x + 1, z), h(x - 1, z), h(x, z + 1), h(x, z - 1));
        let (se, nw, sw, ne) = (h(x + 1, z + 1), h(x - 1, z - 1), h(x - 1, z + 1), h(x + 1, z - 1));
        let values = [e, wv, s_, n, se, nw, sw, ne];
        let at = |k: usize| values[k];
        let slope = crate::apply::slope(
            |a, b| match (a - x, b - z) {
                (1, 0) => at(0),
                (-1, 0) => at(1),
                (0, 1) => at(2),
                (0, -1) => at(3),
                (1, 1) => at(4),
                (-1, -1) => at(5),
                (-1, 1) => at(6),
                _ => at(7),
            },
            x,
            z,
        );
        Ok(slope as f64)
    })
}

fn tile_present(ctx: Ctx<'_>, w: f64, d: f64, tx: f64, ty: f64) -> Q<bool> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        Ok(slot.world.dims[d].ground().contains(massif_core::TilePos::new(tx as i32, ty as i32)))
    })
}

/// WorldPainter's terrain of a column: its ordinal, or `-(id + 2)` for a
/// terrain of Massif's that WorldPainter has not, -1 where there is no tile.
fn terrain_at(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let Some(id) = slot.read(d, x as i32, y as i32, |t, i| t.terrain.get(i)) else { return Ok(-1.0) };
        let o = slot.index_of(id);
        Ok(if o >= 0 { o as f64 } else { -(id as f64) - 2.0 })
    })
}

fn set_terrain(ctx: Ctx<'_>, w: f64, d: f64, x: f64, y: f64, code: f64) -> Q<()> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let code = code as i32;
        let id = if code >= 0 { slot.terrain_id(code)? } else { (-code - 2) as u16 };
        let (x, z) = (x as i32, y as i32);
        let Some(top) = slot.read(d, x, z, |t, i| t.top.get(i)) else { return Ok(()) };
        let id = slot.place_terrain(id, x, z, top);
        slot.write(d, x, z, |t, i| t.terrain.set(i, id));
        Ok(())
    })
}

fn layer_value(ctx: Ctx<'_>, w: f64, d: f64, l: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        s.flush()?;
        let bound = s.bind(l as usize, w as usize)?;
        let (slot, d) = s.column(w as usize, d as i32)?;
        let (x, z) = (x as i32, y as i32);
        let pos = massif_core::TilePos::containing(x, z);
        slot.read(d, x, z, |_, _| ());
        let reader = slot.reader(d);
        let level = slot.world.dims[d].ground();
        Ok(level.view(pos).map_or(0, |t| reader.value(bound, &t, pos, massif_core::coords::local_index(x, z))) as f64)
    })
}

fn set_layer_value(ctx: Ctx<'_>, w: f64, dim: f64, l: f64, x: f64, y: f64, v: f64) -> Q<()> {
    with(&ctx, |s| {
        s.flush()?;
        let bound = s.bind(l as usize, w as usize)?;
        let order = crate::session::bucket(&s.layer(l as usize)?.id);
        let (slot, d) = s.column(w as usize, dim as i32)?;
        let (x, z, v) = (x as i32, y as i32, v as i32);
        match bound {
            crate::session::Bound::Biome => {
                let id = slot.biome_id(v)?;
                slot.write(d, x, z, |t, i| t.biome.set(i, id));
            }
            b if crate::session::overlaid(b) => {
                let pos = massif_core::TilePos::containing(x, z);
                if !slot.world.dims[d].ground().contains(pos) {
                    return Ok(());
                }
                crate::session::paint(&mut slot.painted, d, pos, b, order, massif_core::coords::local_index(x, z), v);
                slot.changed = true;
            }
            crate::session::Bound::GroundCover(k) => {
                if v != 0 {
                    let cover = s.ground_covers.get(k as usize).cloned().ok_or("no such ground cover")?;
                    let (slot, d) = s.column(w as usize, 0)?;
                    let massif_core::World { registry, dims, .. } = &mut slot.world;
                    if let Some(t) = dims[d].ground_mut().tile_mut(massif_core::TilePos::containing(x, z)) {
                        crate::session::paint_ground_cover(&cover, k, t, massif_core::coords::local_index(x, z), registry, &mut slot.ground_made);
                        slot.changed = true;
                    }
                }
            }
            b => {
                let min = slot.world.dims[d].min_y;
                let combined: Vec<Vec<massif_core::combined::Component>> = if matches!(b, crate::session::Bound::Combined(_)) { std::iter::once(Vec::new()).chain(slot.world.registry.combined.iter().map(|c| c.parts.clone())).collect() } else { Vec::new() };
                slot.write(d, x, z, |t, i| crate::session::set_value(b, v, t, i, x, z, min, &combined));
            }
        }
        Ok(())
    })
}

fn clear_layer(ctx: Ctx<'_>, w: f64, d: f64, l: f64) -> Q<()> {
    with(&ctx, |s| {
        let bound = s.bind(l as usize, w as usize)?;
        let (slot, d) = s.column(w as usize, d as i32)?;
        use crate::session::Bound;
        let positions: Vec<massif_core::TilePos> = slot.world.dims[d].ground().positions().collect();
        // What was painted of it with a strength goes too.
        for layers in slot.painted.values_mut() {
            layers.retain(|e| e.0 != bound);
        }
        if let Bound::Caves(_) = bound {
            return Ok(());
        }
        let level = slot.world.dims[d].ground_mut();
        for pos in positions {
            let Some(t) = level.tile_mut(pos) else { continue };
            let clear = |p: &mut massif_core::plane::Plane<u16>, n: u16| {
                for i in 0..massif_core::TILE_AREA {
                    if p.get(i) == n {
                        p.set(i, 0);
                    }
                }
            };
            match bound {
                Bound::Annotations => t.annotations.fill(0),
                Bound::Biome => t.biome.fill(0),
                Bound::Cover { n, .. } => clear(&mut t.cover, n),
                Bound::Objects { n, under, .. } => clear(if under { &mut t.understory } else { &mut t.objects }, n),
                Bound::Pockets(n) => clear(&mut t.pockets, n),
                Bound::Void | Bound::Terrain { .. } | Bound::GroundCover(_) | Bound::Combined(_) => return Err(format!("dimension.clearLayerData() of layer {} is not supported yet", s_name(bound))),
                Bound::Caves(_) | Bound::Inert => {}
            }
        }
        slot.changed = true;
        Ok(())
    })
}

fn s_name(b: crate::session::Bound) -> String {
    format!("{b:?}")
}

fn extent(ctx: Ctx<'_>, w: f64, d: f64) -> Q<String> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let level = slot.world.dims[d].ground();
        let tiles = level.len();
        Ok(match level.bounds() {
            Some((lo, hi)) => serde_json::json!({ "x": lo.x, "y": lo.z, "width": hi.x - lo.x + 1, "height": hi.z - lo.z + 1, "tiles": tiles }),
            None => serde_json::json!({ "x": 0, "y": 0, "width": 0, "height": 0, "tiles": 0 }),
        }
        .to_string())
    })
}

fn tile_coords(ctx: Ctx<'_>, w: f64, d: f64) -> Q<String> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let mut p: Vec<[i32; 2]> = slot.world.dims[d].ground().positions().map(|p| [p.x, p.z]).collect();
        p.sort_unstable_by_key(|p| (p[1], p[0]));
        Ok(serde_json::to_string(&p).expect("numbers"))
    })
}

fn dim_info(ctx: Ctx<'_>, w: f64, d: f64) -> Q<String> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let dim = &slot.world.dims[d];
        Ok(serde_json::json!({ "minHeight": dim.min_y, "maxHeight": dim.max_y }).to_string())
    })
}

fn height_range(ctx: Ctx<'_>, w: f64, d: f64) -> Q<String> {
    with(&ctx, |s| {
        let (slot, d) = s.column(w as usize, d as i32)?;
        let positions: Vec<massif_core::TilePos> = slot.world.dims[d].ground().positions().collect();
        slot.world.dims[d].ground_mut().warm(&positions);
        let level = slot.world.dims[d].ground();
        let (mut lo, mut hi) = (f32::MAX, f32::MIN);
        for p in positions {
            if let Some(t) = level.view(p) {
                t.top.for_each_run(|v, _| {
                    lo = lo.min(v - 1.0);
                    hi = hi.max(v - 1.0);
                });
            }
        }
        Ok(serde_json::to_string(&if lo > hi { [0.0, 0.0] } else { [lo, hi] }).expect("numbers"))
    })
}

fn world_layers(ctx: Ctx<'_>, w: f64, _d: f64) -> Q<String> {
    with(&ctx, |s| {
        let names: Vec<String> = {
            let reg = &s.slot(w as usize)?.world.registry;
            let covers = reg.covers.iter().filter(|c| !c.removed).map(|c| c.name.clone());
            let objects = reg.object_layers.iter().filter(|l| !l.removed).map(|l| l.name.clone());
            let pockets = reg.pockets.iter().filter(|p| !p.removed).map(|p| p.name.clone());
            let combined = reg.combined.iter().filter(|c| !c.removed).map(|c| c.name.clone());
            covers.chain(objects).chain(pockets).chain(combined).collect()
        };
        let mut handles = Vec::new();
        for n in names {
            if let Ok(l) = s.world_layer(w as usize, &n) {
                handles.push(l);
            }
        }
        Ok(serde_json::to_string(&handles).expect("numbers"))
    })
}

fn heightmap_info(ctx: Ctx<'_>, h: f64) -> Q<String> {
    with(&ctx, |s| {
        let hm = s.heightmap(h as usize)?;
        Ok(serde_json::json!({ "name": hm.name, "width": hm.header.width, "height": hm.header.height, "bits": hm.header.bits }).to_string())
    })
}

fn heightmap_value(ctx: Ctx<'_>, h: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let img = s.heightmap(h as usize)?.image()?;
        let (x, y) = (x as i64, y as i64);
        Ok(if x >= 0 && y >= 0 && x < img.width as i64 && y < img.height as i64 { img.at(y as usize * img.width as usize + x as usize) as f64 } else { 0.0 })
    })
}

fn heightmap_colour(ctx: Ctx<'_>, h: f64, x: f64, y: f64) -> Q<f64> {
    with(&ctx, |s| {
        let hm = s.heightmap(h as usize)?;
        let (w, ht) = (hm.header.width as i64, hm.header.height as i64);
        let argb = hm.argb()?;
        let (x, y) = (x as i64, y as i64);
        // As Java's int: negative for opaque colours.
        Ok(if x >= 0 && y >= 0 && x < w && y < ht { argb[(y * w + x) as usize] as i32 as f64 } else { 0.0 })
    })
}

fn open_world(ctx: Ctx<'_>, file: String) -> Q<f64> {
    with(&ctx, |s| s.open_world(&file).map(|w| w as f64))
}

fn material_name(ctx: Ctx<'_>, k: f64) -> Q<String> {
    with(&ctx, |s| s.materials.get(k as usize).map(|m| m.name.clone()).ok_or_else(|| "terrain may not be null".into()))
}

fn terrain_file(ctx: Ctx<'_>, file: String) -> Q<f64> {
    with(&ctx, |s| s.terrain_file(&file).map(|k| k as f64))
}

fn install_terrain(ctx: Ctx<'_>, k: f64, w: f64, slot: f64) -> Q<f64> {
    with(&ctx, |s| {
        let m = s.materials.get(k as usize).cloned().ok_or("terrain may not be null")?;
        s.flush()?;
        s.slot(w as usize)?.install_terrain(&m, slot as i32).map(|o| o as f64)
    })
}

fn create_world(ctx: Ctx<'_>, spec: String) -> Q<f64> {
    with(&ctx, |s| {
        let spec = json(&spec)?;
        s.create_world(spec).map(|w| w as f64)
    })
}

fn save_world(ctx: Ctx<'_>, w: f64, file: String) -> Q<String> {
    with(&ctx, |s| s.save_world(w as usize, &file).map(|p| p.display().to_string()))
}

fn export_world(ctx: Ctx<'_>, w: f64, dir: Option<String>) -> Q<String> {
    with(&ctx, |s| s.export_world(w as usize, dir.as_deref()).map(|p| p.display().to_string()))
}

fn load_heightmap(ctx: Ctx<'_>, file: String, channel: f64) -> Q<f64> {
    with(&ctx, |s| s.load_heightmap(&file, (channel >= 0.0).then_some(channel as usize)).map(|h| h as f64))
}

fn queue(ctx: Ctx<'_>, op: String) -> Q<()> {
    with(&ctx, |s| {
        let op = json(&op)?;
        s.queue(op)
    })
}

fn filter(ctx: Ctx<'_>, def: String) -> Q<f64> {
    with(&ctx, |s| {
        let def = json(&def)?;
        s.filters.push(def);
        Ok((s.filters.len() - 1) as f64)
    })
}

fn biome_id(ctx: Ctx<'_>, name: String, w: f64) -> Q<f64> {
    with(&ctx, |s| {
        if let Some(n) = crate::worlds::biome_number(&name) {
            return Ok(n as f64);
        }
        if w >= 0.0 {
            let reg = &s.slot(w as usize)?.world.registry;
            if reg.biomes.iter().any(|b| b.name.eq_ignore_ascii_case(&name)) {
                return Err(format!("Biome \"{name}\" is a custom biome of this map, which WorldPainter's biome numbers cannot name"));
            }
        }
        Err(format!("No biome found with name or ID \"{name}\""))
    })
}

fn flush(ctx: Ctx<'_>) -> Q<()> {
    with(&ctx, |s| s.flush())
}

fn progress(ctx: Ctx<'_>, fraction: f64, label: Option<String>) -> Q<()> {
    with(&ctx, |s| {
        match label {
            Some(l) => s.io.progress.set(fraction as f32, &l),
            None => s.io.progress.set_fraction(fraction as f32),
        }
        Ok(())
    })
}

// Massif's own API: regions of columns at once, as typed arrays.

/// The rectangle `[x0, z0, x1, z1]` a script gave.
fn rect(ctx: &Ctx<'_>, r: &[f64]) -> Q<(i32, i32, i32, i32)> {
    match r {
        [x0, z0, x1, z1] => Ok((*x0 as i32, *z0 as i32, *x1 as i32, *z1 as i32)),
        _ => Err(Exception::throw_type(ctx, "A region is [x0, z0, x1, z1]")),
    }
}

fn region_get<'js>(ctx: Ctx<'js>, w: f64, d: f64, what: String, r: Vec<f64>) -> Q<Value<'js>> {
    let (x0, z0, x1, z1) = rect(&ctx, &r)?;
    let data = with(&ctx, |s| crate::region::get(s, w as usize, d as i32, &what, x0, z0, x1, z1))?;
    Ok(match data {
        crate::region::Data::F32(v) => TypedArray::<f32>::new(ctx.clone(), v)?.into_value(),
        crate::region::Data::U16(v) => TypedArray::<u16>::new(ctx.clone(), v)?.into_value(),
        crate::region::Data::I32(v) => TypedArray::<i32>::new(ctx.clone(), v)?.into_value(),
    })
}

fn region_set<'js>(ctx: Ctx<'js>, w: f64, d: f64, what: String, r: Vec<f64>, values: Value<'js>) -> Q<()> {
    let (x0, z0, x1, z1) = rect(&ctx, &r)?;
    let data = if let Some(a) = values.as_object().and_then(|o| o.as_typed_array::<f32>()) {
        crate::region::Data::F32(unsafe { a.as_slice() }.to_vec())
    } else if let Some(a) = values.as_object().and_then(|o| o.as_typed_array::<u16>()) {
        crate::region::Data::U16(unsafe { a.as_slice() }.to_vec())
    } else if let Some(a) = values.as_object().and_then(|o| o.as_typed_array::<i32>()) {
        crate::region::Data::I32(unsafe { a.as_slice() }.to_vec())
    } else if let Some(arr) = values.as_array() {
        let mut v = Vec::with_capacity(arr.len());
        for k in 0..arr.len() {
            v.push(arr.get::<f64>(k)? as f32);
        }
        crate::region::Data::F32(v)
    } else {
        return Err(Exception::throw_type(&ctx, "values must be a Float32Array, Uint16Array, Int32Array or an array of numbers"));
    };
    with(&ctx, |s| crate::region::set(s, w as usize, d as i32, &what, x0, z0, x1, z1, data))
}

fn massif_call(ctx: Ctx<'_>, name: String, args: String) -> Q<String> {
    with(&ctx, |s| {
        let args: serde_json::Value = json(&args)?;
        crate::api::call(s, &name, &args).map(|v| v.to_string())
    })
}

/// Make `__m`, the natives.
fn natives<'js>(ctx: &Ctx<'js>) -> Q<Object<'js>> {
    let m = Object::new(ctx.clone())?;
    macro_rules! add {
        ($($name:literal => $f:expr),* $(,)?) => {
            $( m.set($name, Function::new(ctx.clone(), $f)?.with_name($name)?)?; )*
        };
    }
    add! {
        "scriptName" => script_name,
        "version" => version,
        "terrainNames" => terrain_names,
        "terrainName" => terrain_name,
        "print" => print,
        "printWarning" => print_warning,
        "printError" => print_error,
        "cancelled" => cancelled,
        "layerInfo" => layer_info,
        "layerByName" => layer_by_name,
        "layerFromWorld" => layer_from_world,
        "layerFromFile" => layer_from_file,
        "worldInfo" => world_info,
        "setWorldName" => set_world_name,
        "setSpawn" => set_spawn,
        "mapFormat" => map_format,
        "setFormat" => set_format,
        "hasDimension" => has_dimension,
        "height" => height,
        "rawHeight" => raw_height,
        "intHeight" => int_height,
        "setHeight" => set_height,
        "water" => water,
        "setWater" => set_water,
        "lava" => lava,
        "slope" => slope,
        "tilePresent" => tile_present,
        "terrainAt" => terrain_at,
        "setTerrain" => set_terrain,
        "layerValue" => layer_value,
        "setLayerValue" => set_layer_value,
        "clearLayer" => clear_layer,
        "extent" => extent,
        "tileCoords" => tile_coords,
        "dimInfo" => dim_info,
        "heightRange" => height_range,
        "worldLayers" => world_layers,
        "heightMapInfo" => heightmap_info,
        "heightMapValue" => heightmap_value,
        "heightMapColour" => heightmap_colour,
        "openWorld" => open_world,
        "materialName" => material_name,
        "terrainFile" => terrain_file,
        "installTerrain" => install_terrain,
        "createWorld" => create_world,
        "saveWorld" => save_world,
        "exportWorld" => export_world,
        "loadHeightMap" => load_heightmap,
        "queue" => queue,
        "filter" => filter,
        "biomeId" => biome_id,
        "flush" => flush,
        "progress" => progress,
        "regionGet" => region_get,
        "regionSet" => region_set,
        "call" => massif_call,
    }
    Ok(m)
}

/// Why a script stopped, as the console shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptError {
    pub message: String,
    /// Where in the script, when known (1-based).
    pub line: Option<u32>,
    pub column: Option<u32>,
    /// The script's frames of the stack, outermost last.
    pub stack: String,
}

impl std::fmt::Display for ScriptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "{} (line {l}, column {c})", self.message),
            (Some(l), None) => write!(f, "{} (line {l})", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

/// How a run ended.
pub enum End {
    Done,
    Exit(i32),
    Cancelled,
    Failed(ScriptError),
}

/// The line and column of the first frame of `stack` in `file`.
fn where_in(stack: &str, file: &str) -> Option<(u32, Option<u32>)> {
    let needle = format!("{file}:");
    for line in stack.lines() {
        let Some(k) = line.find(&needle) else { continue };
        let rest = &line[k + needle.len()..];
        let mut parts = rest.split(|c: char| !c.is_ascii_digit());
        let l = parts.next().and_then(|p| p.parse().ok())?;
        let c = parts.next().and_then(|p| p.parse().ok());
        return Some((l, c));
    }
    None
}

/// "(in the applyHeightMap() of line 12)": the line an operation applied
/// later gave.
fn later_line(message: &str) -> Option<u32> {
    let k = message.rfind(" of line ")?;
    message[k + 9..].trim_end_matches(')').parse().ok()
}

fn ended<'js>(ctx: &Ctx<'js>, e: rquickjs::Error, file: &str) -> End {
    if !matches!(e, rquickjs::Error::Exception) {
        return End::Failed(ScriptError { message: e.to_string(), line: None, column: None, stack: String::new() });
    }
    let v = ctx.catch();
    let Some(obj) = v.as_object() else {
        let text = v.as_string().and_then(|s| s.to_string().ok()).unwrap_or_else(|| format!("{v:?}"));
        return End::Failed(ScriptError { message: format!("Uncaught {text}"), line: None, column: None, stack: String::new() });
    };
    let name: String = obj.get::<_, Option<String>>("name").ok().flatten().unwrap_or_default();
    let message: String = obj.get::<_, Option<String>>("message").ok().flatten().unwrap_or_default();
    let stack: String = obj.get::<_, Option<String>>("stack").ok().flatten().unwrap_or_default();
    if name == "Exit" {
        let code: i32 = obj.get::<_, Option<i32>>("exitCode").ok().flatten().unwrap_or(0);
        return End::Exit(code);
    }
    if message == CANCELLED || (name == "InternalError" && message == "interrupted") {
        return End::Cancelled;
    }
    let (line, column) = match where_in(&stack, file) {
        Some((l, c)) => (Some(l), c),
        None => (later_line(&message), None),
    };
    let script_frames: Vec<&str> = stack.lines().filter(|l| l.contains(&format!("{file}:"))).collect();
    let message = match name.as_str() {
        "" | "Error" | "ScriptException" | SCRIPT_EXCEPTION => message,
        _ => format!("{name}: {message}"),
    };
    End::Failed(ScriptError { message, line, column, stack: script_frames.join("\n") })
}

/// What the runner binds before the script: `argv`, `params`…
pub struct Bindings {
    pub args: Vec<String>,
    pub params: Vec<(String, crate::descriptor::Value)>,
    pub script_dir: String,
    /// The open world and the dimension shown (WorldPainter's numbers).
    pub open: Option<(usize, i32)>,
}

/// Run `source` (named `file`) on `session`.
pub fn run(session: Session, source: &str, file: &str, bindings: &Bindings) -> (Session, End) {
    let cancel = session.io.cancel.clone();
    let shared = Rc::new(RefCell::new(session));
    SESSION.with(|s| *s.borrow_mut() = Some(shared.clone()));
    SCRIPT.with(|s| file.clone_into(&mut s.borrow_mut()));
    let end = run_in(source, file, bindings, cancel);
    SESSION.with(|s| *s.borrow_mut() = None);
    let session = Rc::try_unwrap(shared).ok().expect("natives keep no session").into_inner();
    (session, end)
}

fn run_in(source: &str, file: &str, bindings: &Bindings, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> End {
    let rt = match rquickjs::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => return End::Failed(ScriptError { message: format!("JavaScript could not start: {e}"), line: None, column: None, stack: String::new() }),
    };
    rt.set_memory_limit(4 << 30);
    rt.set_max_stack_size(8 << 20);
    rt.set_interrupt_handler(Some(Box::new(move || cancel.load(std::sync::atomic::Ordering::Relaxed))));
    let ctx = match rquickjs::Context::full(&rt) {
        Ok(c) => c,
        Err(e) => return End::Failed(ScriptError { message: format!("JavaScript could not start: {e}"), line: None, column: None, stack: String::new() }),
    };
    ctx.with(|ctx| {
        let setup = || -> Q<()> {
            let g = ctx.globals();
            let m = natives(&ctx)?;
            g.set("__m", m.clone())?;
            g.set("__massif", m)?;
            let argv: Vec<String> = std::iter::once(file.to_owned()).chain(bindings.args.iter().cloned()).collect();
            g.set("argc", argv.len() as i32)?;
            g.set("argv", argv)?;
            g.set("arguments", bindings.args.clone())?;
            g.set("scriptDir", bindings.script_dir.clone())?;
            let params = Object::new(ctx.clone())?;
            for (name, v) in &bindings.params {
                use crate::descriptor::Value as P;
                match v {
                    P::String(s) => params.set(name.as_str(), s.as_str())?,
                    P::Integer(i) => params.set(name.as_str(), *i as f64)?,
                    P::Float(f) => params.set(name.as_str(), *f)?,
                    P::Boolean(b) => params.set(name.as_str(), *b)?,
                    P::File(p) => params.set(name.as_str(), p.as_ref().map(|p| p.display().to_string()))?,
                }
            }
            g.set("params", params)?;
            let mut o = rquickjs::context::EvalOptions::default();
            o.strict = false;
            o.filename = Some("wp.js".into());
            ctx.eval_with_options::<(), _>(include_str!("wp.js"), o)?;
            let mut o = rquickjs::context::EvalOptions::default();
            o.strict = false;
            o.filename = Some("massif.js".into());
            ctx.eval_with_options::<(), _>(include_str!("massif.js"), o)?;
            if let Some((w, d)) = bindings.open {
                ctx.eval::<(), _>(format!("var world = __wp.world({w}); var dimension = __wp.dimension({w}, {d}); __massifOpen({w}, {d});"))?;
            }
            Ok(())
        };
        if let Err(e) = setup() {
            return match ended(&ctx, e, "wp.js") {
                End::Failed(mut err) => {
                    err.message = format!("Massif's scripting could not start: {}", err.message);
                    End::Failed(err)
                }
                other => other,
            };
        }
        let mut o = rquickjs::context::EvalOptions::default();
        o.strict = false;
        o.filename = Some(file.to_owned());
        if let Err(e) = ctx.eval_with_options::<(), _>(source, o) {
            return ended(&ctx, e, file);
        }
        if let Err(e) = ctx.eval::<(), _>("__wp.end()") {
            return ended(&ctx, e, file);
        }
        End::Done
    })
}
