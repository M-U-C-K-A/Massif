//! Massif's own scripting API, in Luau: a thin layer over the operations
//! the WorldPainter side uses too ([`crate::api`], [`crate::region`],
//! [`crate::session`]). A map is a `Map`; a rectangle of its columns, read
//! or to write, a `Grid`.
//!
//! Luau runs sandboxed: no files but through Massif (and only those the
//! script may touch, see [`crate::access`]), no network, no programs, the
//! global environment read-only so that a script cannot change the API.

use crate::js::{Bindings, End, ScriptError};
use crate::region::Data;
use crate::session::{CANCELLED, Line, R, Session};
use mlua::{Lua, LuaSerdeExt, MetaMethod, MultiValue, Table, UserData, UserDataFields, UserDataMethods, Value, VmState};
use std::cell::RefCell;
use std::rc::Rc;

thread_local! {
    static SESSION: RefCell<Option<Rc<RefCell<Session>>>> = const { RefCell::new(None) };
}

/// Run `f` on the session, its error raised in the script.
fn with<T>(f: impl FnOnce(&mut Session) -> R<T>) -> mlua::Result<T> {
    let s = SESSION.with(|s| s.borrow().clone()).expect("a script runs");
    let out = f(&mut s.borrow_mut());
    out.map_err(mlua::Error::RuntimeError)
}

fn call(name: &str, args: serde_json::Value) -> mlua::Result<serde_json::Value> {
    with(|s| crate::api::call(s, name, &args))
}

/// WorldPainter's dimension numbers, by Massif's names.
fn dimension(name: Option<&str>) -> mlua::Result<i32> {
    match name.map(str::to_ascii_lowercase).as_deref() {
        None | Some("surface") | Some("overworld") => Ok(0),
        Some("nether") => Ok(1),
        Some("end") => Ok(2),
        Some(other) => Err(mlua::Error::RuntimeError(format!("Unknown dimension \"{other}\": surface, nether or end"))),
    }
}

/// A rectangle of a map's columns: the values of one kind of data (heights,
/// terrains…), row after row, `x` first.
#[derive(Clone)]
pub struct Grid {
    kind: String,
    x0: i32,
    z0: i32,
    width: usize,
    depth: usize,
    data: GridData,
}

#[derive(Clone)]
enum GridData {
    Float(Vec<f32>),
    Whole(Vec<u16>),
}

impl Grid {
    fn len(&self) -> usize {
        self.width * self.depth
    }

    fn index_of(&self, x: i32, z: i32) -> Option<usize> {
        let (dx, dz) = (x - self.x0, z - self.z0);
        (dx >= 0 && dz >= 0 && (dx as usize) < self.width && (dz as usize) < self.depth).then(|| dz as usize * self.width + dx as usize)
    }

    fn get(&self, k: usize) -> Value {
        match &self.data {
            GridData::Float(v) if v[k].is_nan() => Value::Nil,
            GridData::Float(v) => Value::Number(v[k] as f64),
            GridData::Whole(v) if v[k] == u16::MAX => Value::Nil,
            GridData::Whole(v) => Value::Integer(v[k] as i64),
        }
    }

    fn put(&mut self, k: usize, value: Value) -> mlua::Result<()> {
        let number = match value {
            Value::Nil => None,
            Value::Integer(i) => Some(i as f64),
            Value::Number(n) => Some(n),
            other => return Err(mlua::Error::RuntimeError(format!("A grid of {} holds numbers, not {}", self.kind, other.type_name()))),
        };
        match &mut self.data {
            GridData::Float(v) => v[k] = number.map_or(f32::NAN, |n| n as f32),
            GridData::Whole(v) => v[k] = number.map_or(u16::MAX, |n| n.round().clamp(0.0, u16::MAX as f64) as u16),
        }
        Ok(())
    }

    fn into_data(self) -> Data {
        match self.data {
            GridData::Float(v) => Data::F32(v),
            GridData::Whole(v) => Data::U16(v),
        }
    }
}

impl UserData for Grid {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("width", |_, g| Ok(g.width));
        fields.add_field_method_get("depth", |_, g| Ok(g.depth));
        fields.add_field_method_get("x0", |_, g| Ok(g.x0));
        fields.add_field_method_get("z0", |_, g| Ok(g.z0));
        fields.add_field_method_get("x1", |_, g| Ok(g.x0 + g.width as i32));
        fields.add_field_method_get("z1", |_, g| Ok(g.z0 + g.depth as i32));
        fields.add_field_method_get("kind", |_, g| Ok(g.kind.clone()));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        // g[i], 1 to #g, row after row.
        methods.add_meta_method(MetaMethod::Index, |_, g, k: i64| {
            if k < 1 || k as usize > g.len() {
                return Ok(Value::Nil);
            }
            Ok(g.get(k as usize - 1))
        });
        methods.add_meta_method_mut(MetaMethod::NewIndex, |_, g, (k, v): (i64, Value)| {
            if k < 1 || k as usize > g.len() {
                return Err(mlua::Error::RuntimeError(format!("Index {k} out of the grid (1 to {})", g.len())));
            }
            g.put(k as usize - 1, v)
        });
        methods.add_meta_method(MetaMethod::Len, |_, g, ()| Ok(g.len()));
        methods.add_method("at", |_, g, (x, z): (i32, i32)| Ok(g.index_of(x, z).map_or(Value::Nil, |k| g.get(k))));
        methods.add_method_mut("set", |_, g, (x, z, v): (i32, i32, Value)| match g.index_of(x, z) {
            Some(k) => g.put(k, v),
            None => Err(mlua::Error::RuntimeError(format!("Column ({x}, {z}) is out of the grid"))),
        });
        methods.add_method_mut("fill", |_, g, v: Value| {
            for k in 0..g.len() {
                g.put(k, v.clone())?;
            }
            Ok(())
        });
        // Sums over the grid, fast: no loop in the script.
        methods.add_method("sum", |_, g, ()| {
            Ok(match &g.data {
                GridData::Float(v) => v.iter().filter(|x| !x.is_nan()).map(|&x| x as f64).sum::<f64>(),
                GridData::Whole(v) => v.iter().filter(|&&x| x != u16::MAX).map(|&x| x as f64).sum::<f64>(),
            })
        });
        methods.add_method("min", |_, g, ()| {
            Ok(match &g.data {
                GridData::Float(v) => v.iter().filter(|x| !x.is_nan()).fold(None, |m: Option<f32>, &x| Some(m.map_or(x, |m| m.min(x)))).map(|x| x as f64),
                GridData::Whole(v) => v.iter().filter(|&&x| x != u16::MAX).min().map(|&x| x as f64),
            })
        });
        methods.add_method("max", |_, g, ()| {
            Ok(match &g.data {
                GridData::Float(v) => v.iter().filter(|x| !x.is_nan()).fold(None, |m: Option<f32>, &x| Some(m.map_or(x, |m| m.max(x)))).map(|x| x as f64),
                GridData::Whole(v) => v.iter().filter(|&&x| x != u16::MAX).max().map(|&x| x as f64),
            })
        });
        methods.add_method("count", |_, g, v: f64| {
            Ok(match &g.data {
                GridData::Float(d) => d.iter().filter(|&&x| x as f64 == v).count(),
                GridData::Whole(d) => d.iter().filter(|&&x| x as f64 == v).count(),
            })
        });
        methods.add_method("copy", |_, g, ()| Ok(g.clone()));
    }
}

/// A map a script holds: one of its dimensions.
#[derive(Clone, Copy)]
pub struct Map {
    w: usize,
    dim: i32,
}

impl Map {
    fn info(&self) -> mlua::Result<serde_json::Value> {
        call("info", serde_json::json!({ "world": self.w }))
    }

    fn rect(args: &MultiValue, from: usize) -> mlua::Result<(i32, i32, i32, i32)> {
        let n = |k: usize| -> mlua::Result<i32> {
            match args.get(from + k) {
                Some(Value::Integer(i)) => Ok(*i as i32),
                Some(Value::Number(f)) => Ok(f.floor() as i32),
                _ => Err(mlua::Error::RuntimeError("A region is x0, z0, x1, z1: from its first column to past its last".into())),
            }
        };
        Ok((n(0)?, n(1)?, n(2)?, n(3)?))
    }
}

impl UserData for Map {
    fn add_fields<F: UserDataFields<Self>>(fields: &mut F) {
        fields.add_field_method_get("name", |_, m| Ok(m.info()?["name"].as_str().unwrap_or_default().to_owned()));
        fields.add_field_method_set("name", |_, m, name: String| call("set", serde_json::json!({ "world": m.w, "name": name })).map(|_| ()));
        fields.add_field_method_get("seed", |_, m| Ok(m.info()?["seed"].as_i64().unwrap_or(0)));
        fields.add_field_method_set("seed", |_, m, seed: i64| call("set", serde_json::json!({ "world": m.w, "seed": seed })).map(|_| ()));
        fields.add_field_method_get("sea_level", |_, m| Ok(m.info()?["seaLevel"].as_i64().unwrap_or(62)));
        fields.add_field_method_set("sea_level", |_, m, v: i64| call("set", serde_json::json!({ "world": m.w, "seaLevel": v })).map(|_| ()));
        fields.add_field_method_get("spawn", |lua, m| lua.to_value(&m.info()?["spawn"]));
        fields.add_field_method_set("spawn", |_, m, v: Vec<f64>| call("set", serde_json::json!({ "world": m.w, "spawn": v })).map(|_| ()));
        fields.add_field_method_get("dimension", |_, m| Ok(["surface", "nether", "end"][m.dim.clamp(0, 2) as usize]));
    }

    fn add_methods<M: UserDataMethods<Self>>(methods: &mut M) {
        methods.add_method("info", |lua, m, ()| lua.to_value(&m.info()?));
        methods.add_method("bounds", |_, m, ()| {
            let info = m.info()?;
            let dim = info["dims"].as_array().and_then(|d| d.iter().find(|d| d["kind"].as_str().is_some_and(|k| dimension(Some(k)).ok() == Some(m.dim)))).cloned();
            match dim.and_then(|d| d["bounds"].as_array().cloned()) {
                Some(b) if b.len() == 4 => Ok((b[0].as_i64(), b[1].as_i64(), b[2].as_i64(), b[3].as_i64())),
                _ => Ok((None, None, None, None)),
            }
        });
        // for x0, z0, x1, z1 in map:tiles() do … end
        methods.add_method("tiles", |lua, m, ()| {
            let w = m.w;
            let dim = m.dim;
            let mut tiles: Vec<(i32, i32)> = with(|s| {
                let (slot, d) = s.column(w, dim)?;
                Ok(slot.world.dims[d].ground().positions().map(|p| (p.x, p.z)).collect())
            })?;
            tiles.sort_unstable_by_key(|&(x, z)| (z, x));
            let mut k = 0;
            lua.create_function_mut(move |_, ()| {
                let Some(&(x, z)) = tiles.get(k) else { return Ok((None, None, None, None)) };
                k += 1;
                let t = massif_core::TILE_SIZE;
                Ok((Some(x * t), Some(z * t), Some((x + 1) * t), Some((z + 1) * t)))
            })
        });
        methods.add_method("get", |_, m, args: MultiValue| {
            let kind: String = match args.front() {
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                _ => return Err(mlua::Error::RuntimeError("map:get(kind, x0, z0, x1, z1): the kind first (\"height\", \"terrain\"…)".into())),
            };
            let (x0, z0, x1, z1) = Map::rect(&args, 1)?;
            let data = with(|s| crate::region::get(s, m.w, m.dim, &kind, x0, z0, x1, z1))?;
            let data = match data {
                Data::F32(v) => GridData::Float(v),
                Data::U16(v) => GridData::Whole(v),
                Data::I32(v) => GridData::Float(v.into_iter().map(|x| x as f32).collect()),
            };
            Ok(Grid { kind, x0, z0, width: (x1 - x0) as usize, depth: (z1 - z0) as usize, data })
        });
        methods.add_method("set", |_, m, args: MultiValue| {
            let kind: String = match args.front() {
                Some(Value::String(s)) => s.to_str()?.to_owned(),
                _ => return Err(mlua::Error::RuntimeError("map:set(kind, x0, z0, x1, z1, values): the kind first".into())),
            };
            let (x0, z0, x1, z1) = Map::rect(&args, 1)?;
            let n = ((x1 - x0).max(0) as usize) * ((z1 - z0).max(0) as usize);
            let data = match args.get(5) {
                Some(Value::UserData(ud)) => ud.borrow::<Grid>()?.clone().into_data(),
                Some(Value::Table(t)) => {
                    let mut v = Vec::with_capacity(n);
                    for k in 1..=n {
                        v.push(match t.get::<Value>(k)? {
                            Value::Integer(i) => i as f32,
                            Value::Number(f) => f as f32,
                            _ => f32::NAN,
                        });
                    }
                    Data::F32(v)
                }
                Some(Value::Integer(i)) => Data::F32(vec![*i as f32; n]),
                Some(Value::Number(f)) => Data::F32(vec![*f as f32; n]),
                _ => return Err(mlua::Error::RuntimeError("map:set(kind, x0, z0, x1, z1, values): values are a grid, a table of numbers or one number".into())),
            };
            with(|s| crate::region::set(s, m.w, m.dim, &kind, x0, z0, x1, z1, data))
        });
        methods.add_method("height", |_, m, (x, z): (i32, i32)| {
            let d = with(|s| crate::region::get(s, m.w, m.dim, "height", x, z, x + 1, z + 1))?;
            Ok(match d {
                Data::F32(v) if !v[0].is_nan() => Some(v[0] as f64),
                _ => None,
            })
        });
        methods.add_method("set_height", |_, m, (x, z, h): (i32, i32, f64)| with(|s| crate::region::set(s, m.w, m.dim, "height", x, z, x + 1, z + 1, Data::F32(vec![h as f32]))));
        methods.add_method("terrain_id", |_, m, name: String| Ok(call("terrainId", serde_json::json!({ "world": m.w, "name": name }))?.as_i64()));
        methods.add_method("biome_id", |_, m, name: String| Ok(call("biomeId", serde_json::json!({ "world": m.w, "name": name }))?.as_i64()));
        methods.add_method("layer", |lua, m, name: String| lua.to_value(&call("layerId", serde_json::json!({ "world": m.w, "name": name }))?));
        methods.add_method("terrains", |lua, m, ()| lua.to_value(&call("terrains", serde_json::json!({ "world": m.w }))?));
        methods.add_method("biomes", |lua, m, ()| lua.to_value(&call("biomes", serde_json::json!({ "world": m.w }))?));
        methods.add_method("layers", |lua, m, ()| lua.to_value(&call("layers", serde_json::json!({ "world": m.w }))?));
        // New terrains, ground covers and object layers, or redefined ones
        // of the same name: what WorldPainter's custom terrains, plant
        // layers and custom object layers are.
        for (method, name) in [("add_terrain", "addTerrain"), ("add_cover", "addCover"), ("add_objects", "addObjects")] {
            methods.add_method(method, move |lua, m, def: Value| {
                let mut args: serde_json::Value = lua.from_value(def)?;
                let Some(o) = args.as_object_mut() else { return Err(mlua::Error::runtime(format!("{method} takes a table"))) };
                o.insert("world".into(), m.w.into());
                lua.to_value(&call(name, args)?)
            });
        }
        methods.add_method("select", |_, m, args: MultiValue| {
            let rect = if args.front().is_none_or(|v| v.is_nil()) {
                serde_json::Value::Null
            } else {
                let (x0, z0, x1, z1) = Map::rect(&args, 0)?;
                serde_json::json!([x0, z0, x1, z1])
            };
            call("select", serde_json::json!({ "world": m.w, "dim": m.dim, "rect": rect })).map(|_| ())
        });
        methods.add_method("whole_map", |lua, m, (op, options): (String, Option<Value>)| {
            let options: serde_json::Value = match options {
                Some(v) => lua.from_value(v)?,
                None => serde_json::json!({}),
            };
            lua.to_value(&call("wholeMap", serde_json::json!({ "world": m.w, "dim": m.dim, "op": op, "options": options }))?)
        });
        methods.add_method("import_mask", |lua, m, (file, options): (String, Option<Value>)| {
            let options: serde_json::Value = match options {
                Some(v) => lua.from_value(v)?,
                None => serde_json::json!({}),
            };
            lua.to_value(&call("importMask", serde_json::json!({ "world": m.w, "dim": m.dim, "file": file, "options": options }))?)
        });
        methods.add_method("save", |_, m, file: String| Ok(call("save", serde_json::json!({ "world": m.w, "file": file }))?.as_str().unwrap_or_default().to_owned()));
        methods.add_method("export", |lua, m, (dir, options): (String, Option<Table>)| {
            let mut args = serde_json::json!({ "world": m.w, "dir": dir });
            if let Some(o) = options {
                if let Some(v) = o.get::<Option<String>>("version")? {
                    args["version"] = v.into();
                }
                if let Some(v) = o.get::<Option<bool>>("overwrite")? {
                    args["overwrite"] = v.into();
                }
            }
            lua.to_value(&call("export", args)?)
        });
        methods.add_method("dimension", |_, m, name: String| Ok(Map { w: m.w, dim: dimension(Some(&name))? }));
        methods.add_meta_method(MetaMethod::ToString, |_, m, ()| Ok(format!("Map {}", m.info()?["name"].as_str().unwrap_or_default())));
    }
}

fn text(args: MultiValue) -> String {
    args.iter()
        .map(|v| match v {
            Value::String(s) => s.to_string_lossy(),
            Value::Nil => "nil".into(),
            Value::Boolean(b) => b.to_string(),
            Value::Integer(i) => i.to_string(),
            Value::Number(n) => n.to_string(),
            other => format!("{other:?}"),
        })
        .collect::<Vec<_>>()
        .join("\t")
}

/// The globals of a run: `massif`, `print`.
fn install(lua: &Lua, bindings: &Bindings) -> mlua::Result<()> {
    let g = lua.globals();
    let massif = lua.create_table()?;
    massif.set("version", env!("CARGO_PKG_VERSION"))?;
    massif.set("args", bindings.args.clone())?;
    let params = lua.create_table()?;
    for (name, v) in &bindings.params {
        use crate::descriptor::Value as P;
        match v {
            P::String(s) => params.set(name.as_str(), s.as_str())?,
            P::Integer(i) => params.set(name.as_str(), *i)?,
            P::Float(f) => params.set(name.as_str(), *f)?,
            P::Boolean(b) => params.set(name.as_str(), *b)?,
            P::File(p) => params.set(name.as_str(), p.as_ref().map(|p| p.display().to_string()))?,
        }
    }
    massif.set("params", params)?;
    massif.set("script_dir", bindings.script_dir.clone())?;
    if let Some((w, d)) = bindings.open {
        massif.set("map", Map { w, dim: d })?;
    }
    massif.set("open", lua.create_function(|_, file: String| Ok(Map { w: call("open", serde_json::json!({ "file": file }))?.as_u64().unwrap_or(0) as usize, dim: 0 }))?)?;
    massif.set(
        "create",
        lua.create_function(|lua, options: Option<Value>| {
            let options: serde_json::Value = match options {
                Some(v) => lua.from_value(v)?,
                None => serde_json::json!({}),
            };
            Ok(Map { w: call("create", serde_json::json!({ "options": options }))?.as_u64().unwrap_or(0) as usize, dim: 0 })
        })?,
    )?;
    massif.set(
        "progress",
        lua.create_function(|_, (fraction, label): (f64, Option<String>)| {
            with(|s| {
                match label {
                    Some(l) => s.io.progress.set(fraction as f32, &l),
                    None => s.io.progress.set_fraction(fraction as f32),
                }
                Ok(())
            })
        })?,
    )?;
    massif.set("cancelled", lua.create_function(|_, ()| with(|s| Ok(s.io.cancelled())))?)?;
    massif.set(
        "warn",
        lua.create_function(|_, args: MultiValue| {
            let t = text(args);
            with(|s| {
                s.print(Line::Warn(t));
                Ok(())
            })
        })?,
    )?;
    g.set("massif", massif)?;
    // No module loading: mlua's `require` reads files from the disk.
    g.set("require", Value::Nil)?;
    g.set(
        "print",
        lua.create_function(|_, args: MultiValue| {
            let t = text(args);
            with(|s| {
                s.print(Line::Out(t));
                Ok(())
            })
        })?,
    )?;
    Ok(())
}

/// The line of `file` an error happened on, from its message
/// (`file:12: …`).
fn line_of(message: &str, file: &str) -> Option<u32> {
    let needle = format!("{file}:");
    let k = message.find(&needle)?;
    message[k + needle.len()..].split(|c: char| !c.is_ascii_digit()).next()?.parse().ok()
}

/// What an error says, without the chunk's name and line before it.
fn message_of(e: &mlua::Error, file: &str) -> String {
    let raw = match e {
        mlua::Error::CallbackError { cause, .. } => return message_of(cause, file),
        mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => m.clone(),
        mlua::Error::WithContext { cause, .. } => return message_of(cause, file),
        other => other.to_string(),
    };
    // "frost.lua:3: attempt to …" → "attempt to …"
    let prefix = format!("{file}:");
    match raw.find(&prefix) {
        Some(0) => raw.splitn(3, ':').nth(2).map_or(raw.clone(), |r| r.trim().to_owned()),
        _ => raw,
    }
}

/// Run `source` (named `file`) on `session`.
pub fn run(session: Session, source: &str, file: &str, bindings: &Bindings) -> (Session, End) {
    let cancel = session.io.cancel.clone();
    let shared = Rc::new(RefCell::new(session));
    SESSION.with(|s| *s.borrow_mut() = Some(shared.clone()));
    let end = run_in(source, file, bindings, cancel);
    SESSION.with(|s| *s.borrow_mut() = None);
    let session = Rc::try_unwrap(shared).ok().expect("callbacks keep no session").into_inner();
    (session, end)
}

fn run_in(source: &str, file: &str, bindings: &Bindings, cancel: std::sync::Arc<std::sync::atomic::AtomicBool>) -> End {
    let lua = Lua::new();
    let _ = lua.set_memory_limit(4 << 30);
    lua.set_interrupt(move |_| {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            Err(mlua::Error::RuntimeError(CANCELLED.into()))
        } else {
            Ok(VmState::Continue)
        }
    });
    if let Err(e) = install(&lua, bindings).and_then(|_| lua.sandbox(true)) {
        return End::Failed(ScriptError { message: format!("Massif's scripting could not start: {e}"), line: None, column: None, stack: String::new() });
    }
    match lua.load(source).set_name(format!("={file}")).exec() {
        Ok(()) => End::Done,
        Err(e) => {
            let message = message_of(&e, file);
            if message.contains(CANCELLED) {
                return End::Cancelled;
            }
            let full = e.to_string();
            let line = line_of(&full, file);
            let stack: Vec<&str> = full.lines().filter(|l| l.contains(&format!("{file}:"))).collect();
            End::Failed(ScriptError { message, line, column: None, stack: stack.join("\n") })
        }
    }
}

