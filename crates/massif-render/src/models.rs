//! The game's block models, for the blocks that are not whole cubes:
//! slabs, stairs, fences, walls, panes, doors, carpets, snow layers,
//! torches, lanterns, the cross of two planes plants are drawn with…
//!
//! A block state picks its models in its `blockstates` file (variants, or
//! the parts of a multipart that apply), each turned by 90° steps; a model
//! takes its elements from the nearest of its parents that has them, and
//! its textures from all of them. Each face of each element becomes a
//! [`Quad`] in the block's space (0 to 1), with its texture, its corner of
//! that texture, and whether the biome tints it.
//!
//! The files are read as the game reads them: from its jar, then the mods'
//! and the resource packs' (a later archive's replace an earlier one's),
//! when a state first needs them (the catalogue keeps only the textures of
//! whole blocks).

use massif_assets::zip::Archive;
use massif_assets::{Catalogue, Texture};
use serde_json::Value;
use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// A face of a model: the point at its texture's (u0, v0) corner and the
/// edges toward u1 and v1, in the block's space.
#[derive(Debug, Clone, PartialEq)]
pub struct Quad {
    pub origin: [f32; 3],
    pub u: [f32; 3],
    pub v: [f32; 3],
    /// The texture's key, and the part of it the face shows (pixels of 16).
    pub texture: Arc<str>,
    pub uv: [f32; 4],
    /// The texture turned by 90° steps on the face.
    pub rotation: u8,
    pub tinted: bool,
}

/// The models of a game and its mods, read when needed.
pub struct Library {
    archives: Vec<Archive>,
    /// Where each file is: archive and entry, by key.
    states: HashMap<String, (usize, String)>,
    models: HashMap<String, (usize, String)>,
    textures: HashMap<String, (usize, String)>,
    parsed: Mutex<HashMap<String, Option<Arc<Value>>>>,
}

impl std::fmt::Debug for Library {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Library({} archives, {} states, {} models)", self.archives.len(), self.states.len(), self.models.len())
    }
}

/// The key of a model or texture reference: the game's without their
/// namespace, a mod's with it (as `massif_assets` keys them).
fn key(path: &str) -> String {
    let path = path.trim();
    path.strip_prefix("minecraft:").or_else(|| path.strip_prefix(':')).unwrap_or(path).to_owned()
}

/// The key of a jar's file under `assets/<namespace>/<kind>/`.
fn asset_key(name: &str, kind: &str, ext: &str) -> Option<String> {
    let rest = name.strip_prefix("assets/")?;
    let (ns, rest) = rest.split_once('/')?;
    let path = rest.strip_prefix(kind)?.strip_prefix('/')?.strip_suffix(ext)?;
    if ns.is_empty() || path.is_empty() {
        return None;
    }
    Some(if ns == "minecraft" { path.to_owned() } else { format!("{ns}:{path}") })
}

impl Library {
    /// The archives a catalogue was read from, in its order.
    pub fn of(cat: &Catalogue) -> Self {
        let paths = std::iter::once(cat.source.as_path()).chain(cat.mods.iter().map(|m| m.path.as_path()));
        Self::open(paths)
    }

    pub fn open<'a>(paths: impl Iterator<Item = &'a Path>) -> Self {
        Self::of_archives(paths.filter_map(|p| Archive::open(p).ok()).collect())
    }

    /// Archives already open, a later one's files replacing an earlier's.
    pub fn of_archives(archives: Vec<Archive>) -> Self {
        let mut lib = Library { archives: Vec::new(), states: HashMap::new(), models: HashMap::new(), textures: HashMap::new(), parsed: Mutex::new(HashMap::new()) };
        for archive in archives {
            let k = lib.archives.len();
            for n in archive.names() {
                if let Some(key) = asset_key(n, "models", ".json") {
                    lib.models.insert(key, (k, n.to_owned()));
                } else if let Some(key) = asset_key(n, "blockstates", ".json") {
                    let id = if key.contains(':') { key } else { format!("minecraft:{key}") };
                    lib.states.insert(id, (k, n.to_owned()));
                } else if let Some(key) = asset_key(n, "textures", ".png") {
                    lib.textures.insert(key, (k, n.to_owned()));
                }
            }
            lib.archives.push(archive);
        }
        lib
    }

    fn json(&self, cache: &str, file: Option<&(usize, String)>) -> Option<Arc<Value>> {
        if let Some(v) = self.parsed.lock().unwrap().get(cache) {
            return v.clone();
        }
        let v = file.and_then(|(k, n)| self.archives[*k].read(n)?.ok()).and_then(|b| serde_json::from_slice::<Value>(&b).ok()).map(Arc::new);
        self.parsed.lock().unwrap().insert(cache.to_owned(), v.clone());
        v
    }

    /// A texture the catalogue does not hold: its first frame.
    pub fn texture(&self, key: &str) -> Option<Texture> {
        let (k, n) = self.textures.get(key)?;
        let bytes = self.archives[*k].read(n)?.ok()?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let side = w.min(h);
        let frame = image::imageops::crop_imm(&img, 0, 0, w, side).to_image();
        Some(Texture { width: w, height: side, rgba: frame.into_raw() })
    }

    /// The faces of the block state `name[properties]`, in the block's
    /// space; empty when it has no model with elements (an entity's block:
    /// a chest, a sign, a bed).
    pub fn quads(&self, name: &str, properties: &[(&str, &str)]) -> Vec<Quad> {
        let Some(state) = self.json(&format!("state {name}"), self.states.get(name)) else { return Vec::new() };
        let mut out = Vec::new();
        for (model, x, y) in models_of(&state, properties) {
            self.model_quads(&model, x, y, &mut out);
        }
        out
    }

    fn model_quads(&self, model: &str, x: i32, y: i32, out: &mut Vec<Quad>) {
        // The model and its parents, the model first.
        let mut chain: Vec<Arc<Value>> = Vec::new();
        let mut current = key(model);
        for _ in 0..16 {
            let Some(m) = self.json(&format!("model {current}"), self.models.get(&current)) else { break };
            let parent = m.get("parent").and_then(Value::as_str).map(key);
            chain.push(m);
            match parent {
                Some(p) => current = p,
                None => break,
            }
        }
        let mut textures: HashMap<String, String> = HashMap::new();
        for m in chain.iter().rev() {
            if let Some(t) = m.get("textures").and_then(Value::as_object) {
                for (k, v) in t {
                    // A name, or since 26.1 `{"sprite": name, …}`.
                    if let Some(v) = v.as_str().or_else(|| v.get("sprite").and_then(Value::as_str)) {
                        textures.insert(k.clone(), v.to_owned());
                    }
                }
            }
        }
        let resolve = |var: &str| -> Option<String> {
            let mut v = var.to_owned();
            for _ in 0..8 {
                match v.strip_prefix('#') {
                    Some(name) => v = textures.get(name)?.clone(),
                    None => return Some(key(&v)),
                }
            }
            None
        };
        let Some(elements) = chain.iter().find_map(|m| m.get("elements").and_then(Value::as_array).cloned()) else { return };
        for el in &elements {
            let corner = |k: &str| -> Option<[f32; 3]> {
                let a = el.get(k)?.as_array()?;
                Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32])
            };
            let (Some(from), Some(to)) = (corner("from"), corner("to")) else { continue };
            let Some(faces) = el.get("faces").and_then(Value::as_object) else { continue };
            let rotation = el.get("rotation").and_then(ElementRotation::of);
            for (dir, face) in faces {
                let Some(texture) = face.get("texture").and_then(Value::as_str).and_then(resolve) else { continue };
                let Some((origin, u, v, default_uv)) = face_frame(dir, from, to) else { continue };
                let uv = face.get("uv").and_then(Value::as_array).and_then(|a| Some([a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32, a.get(2)?.as_f64()? as f32, a.get(3)?.as_f64()? as f32])).unwrap_or(default_uv);
                let turn = (face.get("rotation").and_then(Value::as_i64).unwrap_or(0).rem_euclid(360) / 90) as u8;
                let mut q = Quad { origin, u, v, texture: Arc::from(texture), uv, rotation: turn, tinted: face.get("tintindex").is_some() };
                if let Some(r) = &rotation {
                    r.apply(&mut q);
                }
                turn_state(&mut q, x, y);
                // Model space (pixels) to the block's (0 to 1).
                q.origin = q.origin.map(|c| c / 16.0);
                q.u = q.u.map(|c| c / 16.0);
                q.v = q.v.map(|c| c / 16.0);
                out.push(q);
            }
        }
    }
}

/// The models a state's file picks for `properties`: the first variant
/// that matches, or every part of a multipart that applies, each with its
/// turns around x and y (degrees).
fn models_of(state: &Value, properties: &[(&str, &str)]) -> Vec<(String, i32, i32)> {
    let prop = |k: &str| properties.iter().find(|(p, _)| *p == k).map(|(_, v)| *v);
    let refs = |v: &Value| -> Vec<(String, i32, i32)> {
        let one = |m: &Value| Some((m.get("model")?.as_str()?.to_owned(), m.get("x").and_then(Value::as_i64).unwrap_or(0) as i32, m.get("y").and_then(Value::as_i64).unwrap_or(0) as i32));
        match v {
            // Several weighted models: the first.
            Value::Array(a) => a.first().and_then(one).into_iter().collect(),
            _ => one(v).into_iter().collect(),
        }
    };
    if let Some(variants) = state.get("variants").and_then(Value::as_object) {
        for (k, v) in variants {
            let matches = k.is_empty()
                || k.split(',').all(|pair| match pair.split_once('=') {
                    Some((p, want)) => prop(p).is_none_or(|have| have == want),
                    None => true,
                });
            if matches {
                return refs(v);
            }
        }
        return Vec::new();
    }
    let Some(parts) = state.get("multipart").and_then(Value::as_array) else { return Vec::new() };
    let mut out = Vec::new();
    for part in parts {
        if part.get("when").is_none_or(|w| when(w, &prop)) && let Some(apply) = part.get("apply") {
            out.extend(refs(apply));
        }
    }
    out
}

/// Whether a multipart's condition holds: every property listed takes one
/// of the values (`a|b`), or any of an `OR`, or all of an `AND`.
fn when<'a>(w: &Value, prop: &impl Fn(&str) -> Option<&'a str>) -> bool {
    let Some(map) = w.as_object() else { return false };
    if let Some(any) = map.get("OR").and_then(Value::as_array) {
        return any.iter().any(|c| when(c, prop));
    }
    if let Some(all) = map.get("AND").and_then(Value::as_array) {
        return all.iter().all(|c| when(c, prop));
    }
    map.iter().all(|(k, v)| {
        let want = match v {
            Value::String(s) => s.clone(),
            Value::Bool(b) => b.to_string(),
            Value::Number(n) => n.to_string(),
            _ => return false,
        };
        let have = prop(k).unwrap_or("");
        want.split('|').any(|w| w == have)
    })
}

/// Where a face of an element from `from` to `to` lies (pixels): its
/// corner at the texture's (u0, v0), its edges toward u1 and v1, and the
/// texture's part it shows when the model does not say.
/// A face's corner, its two edges, and the part of its texture.
type Frame = ([f32; 3], [f32; 3], [f32; 3], [f32; 4]);

fn face_frame(dir: &str, f: [f32; 3], t: [f32; 3]) -> Option<Frame> {
    let (dx, dy, dz) = (t[0] - f[0], t[1] - f[1], t[2] - f[2]);
    Some(match dir {
        "up" => ([f[0], t[1], f[2]], [dx, 0.0, 0.0], [0.0, 0.0, dz], [f[0], f[2], t[0], t[2]]),
        "down" => ([f[0], f[1], t[2]], [dx, 0.0, 0.0], [0.0, 0.0, -dz], [f[0], 16.0 - t[2], t[0], 16.0 - f[2]]),
        "north" => ([t[0], t[1], f[2]], [-dx, 0.0, 0.0], [0.0, -dy, 0.0], [16.0 - t[0], 16.0 - t[1], 16.0 - f[0], 16.0 - f[1]]),
        "south" => ([f[0], t[1], t[2]], [dx, 0.0, 0.0], [0.0, -dy, 0.0], [f[0], 16.0 - t[1], t[0], 16.0 - f[1]]),
        "west" => ([f[0], t[1], f[2]], [0.0, 0.0, dz], [0.0, -dy, 0.0], [f[2], 16.0 - t[1], t[2], 16.0 - f[1]]),
        "east" => ([t[0], t[1], t[2]], [0.0, 0.0, -dz], [0.0, -dy, 0.0], [16.0 - t[2], 16.0 - t[1], 16.0 - f[2], 16.0 - f[1]]),
        _ => return None,
    })
}

/// An element's own turn: an angle about an axis through an origin, its
/// faces stretched back across when it says `rescale` (the cross of plants).
struct ElementRotation {
    origin: [f32; 3],
    axis: usize,
    angle: f32,
    rescale: bool,
}

impl ElementRotation {
    fn of(v: &Value) -> Option<Self> {
        let o = v.get("origin")?.as_array()?;
        let origin = [o.first()?.as_f64()? as f32, o.get(1)?.as_f64()? as f32, o.get(2)?.as_f64()? as f32];
        let axis = match v.get("axis")?.as_str()? {
            "x" => 0,
            "y" => 1,
            _ => 2,
        };
        let angle = v.get("angle").and_then(Value::as_f64).unwrap_or(0.0) as f32;
        Some(Self { origin, axis, angle: angle.to_radians(), rescale: v.get("rescale").and_then(Value::as_bool).unwrap_or(false) })
    }

    fn turn(&self, p: [f32; 3]) -> [f32; 3] {
        let (s, c) = self.angle.sin_cos();
        let scale = if self.rescale { 1.0 / c.abs().max(1e-3) } else { 1.0 };
        let (a, b) = match self.axis {
            0 => (1, 2),
            1 => (2, 0),
            _ => (0, 1),
        };
        let mut out = p;
        out[a] = (p[a] * c - p[b] * s) * scale;
        out[b] = (p[a] * s + p[b] * c) * scale;
        out
    }

    fn apply(&self, q: &mut Quad) {
        let rel = [q.origin[0] - self.origin[0], q.origin[1] - self.origin[1], q.origin[2] - self.origin[2]];
        let o = self.turn(rel);
        q.origin = [o[0] + self.origin[0], o[1] + self.origin[1], o[2] + self.origin[2]];
        q.u = self.turn(q.u);
        q.v = self.turn(q.v);
    }
}

/// The state's turns of its model, by 90° steps about the block's middle:
/// first about x, then about y (clockwise seen from above).
fn turn_state(q: &mut Quad, x: i32, y: i32) {
    let about_x = |p: [f32; 3], centre: bool| {
        let c = if centre { 8.0 } else { 0.0 };
        let (py, pz) = (p[1] - c, p[2] - c);
        // 90°: up goes to north.
        [p[0], pz + c, -py + c]
    };
    let about_y = |p: [f32; 3], centre: bool| {
        let c = if centre { 8.0 } else { 0.0 };
        let (px, pz) = (p[0] - c, p[2] - c);
        // 90°: north goes to east.
        [-pz + c, p[1], px + c]
    };
    for _ in 0..(x.rem_euclid(360) / 90) {
        q.origin = about_x(q.origin, true);
        q.u = about_x(q.u, false);
        q.v = about_x(q.v, false);
    }
    for _ in 0..(y.rem_euclid(360) / 90) {
        q.origin = about_y(q.origin, true);
        q.u = about_y(q.u, false);
        q.v = about_y(q.v, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn props<'a>(p: &'a [(&'a str, &'a str)]) -> &'a [(&'a str, &'a str)] {
        p
    }

    #[test]
    fn variants_and_multiparts_pick_their_models() {
        let state: Value = serde_json::from_str(r#"{"variants":{"type=bottom":{"model":"block/oak_slab"},"type=top":{"model":"block/oak_slab_top","x":0}}}"#).unwrap();
        assert_eq!(models_of(&state, props(&[("type", "top"), ("waterlogged", "false")])), vec![("block/oak_slab_top".to_owned(), 0, 0)]);
        let fence: Value = serde_json::from_str(r#"{"multipart":[{"apply":{"model":"block/oak_fence_post"}},{"when":{"north":"true"},"apply":{"model":"block/oak_fence_side","uvlock":true}},{"when":{"east":"true"},"apply":{"model":"block/oak_fence_side","y":90}},{"when":{"OR":[{"north":"false"},{"east":"true|none"}]},"apply":{"model":"x"}}]}"#).unwrap();
        let picked = models_of(&fence, props(&[("north", "true"), ("east", "true"), ("south", "false"), ("west", "false")]));
        assert_eq!(picked, vec![("block/oak_fence_post".to_owned(), 0, 0), ("block/oak_fence_side".to_owned(), 0, 0), ("block/oak_fence_side".to_owned(), 0, 90), ("x".to_owned(), 0, 0)]);
    }

    #[test]
    fn faces_turn_with_the_state() {
        // The north face of a slab, turned 90° about y: the east face.
        let (origin, u, v, _) = face_frame("north", [0.0, 0.0, 0.0], [16.0, 8.0, 16.0]).unwrap();
        let mut q = Quad { origin, u, v, texture: Arc::from("x"), uv: [0.0; 4], rotation: 0, tinted: false };
        turn_state(&mut q, 0, 90);
        let n = [q.u[1] * q.v[2] - q.u[2] * q.v[1], q.u[2] * q.v[0] - q.u[0] * q.v[2], q.u[0] * q.v[1] - q.u[1] * q.v[0]];
        // (Faces are seen from both sides: their normal's sign does not matter.)
        assert!(n[0].abs() > 0.0 && n[1].abs() < 1e-4 && n[2].abs() < 1e-4, "now across x: {n:?}");
        assert!((q.origin[0] - 16.0).abs() < 1e-4, "on the east side: {:?}", q.origin);
        // Upside down (x 180): the top face at the bottom.
        let (origin, u, v, _) = face_frame("up", [0.0, 0.0, 0.0], [16.0, 8.0, 16.0]).unwrap();
        let mut q = Quad { origin, u, v, texture: Arc::from("x"), uv: [0.0; 4], rotation: 0, tinted: false };
        turn_state(&mut q, 180, 0);
        assert!((q.origin[1] - 8.0).abs() < 1e-4, "{:?}", q.origin);
    }

    #[test]
    fn a_cross_is_two_planes_across_the_block() {
        let r = ElementRotation { origin: [8.0, 8.0, 8.0], axis: 1, angle: 45f32.to_radians(), rescale: true };
        let (origin, u, v, _) = face_frame("north", [0.8, 0.0, 8.0], [15.2, 16.0, 8.0]).unwrap();
        let mut q = Quad { origin, u, v, texture: Arc::from("x"), uv: [0.0; 4], rotation: 0, tinted: true };
        r.apply(&mut q);
        let end = [q.origin[0] + q.u[0], q.origin[2] + q.u[2]];
        // From corner to corner of the block, diagonally (0.8 to 15.2, as
        // the game's cross).
        assert!(q.origin[0].min(end[0]) < 1.0 && q.origin[0].max(end[0]) > 15.0, "{:?} {:?}", q.origin, end);
        assert!(q.origin[2].min(end[1]) < 1.0 && q.origin[2].max(end[1]) > 15.0, "{:?} {:?}", q.origin, end);
    }
}

#[cfg(test)]
mod game {
    /// The game's own models, read from its jar when it is installed.
    #[test]
    fn the_games_models_have_faces() {
        let Some(Ok(cat)) = massif_assets::load_newest() else { return };
        let lib = super::Library::of(&cat);
        let count = |name: &str, props: &[(&str, &str)]| lib.quads(name, props).len();
        let pane = count("minecraft:glass_pane", &[("east", "true"), ("west", "true"), ("north", "false"), ("south", "false"), ("waterlogged", "false")]);
        let slab = count("minecraft:stone_slab", &[("type", "bottom"), ("waterlogged", "false")]);
        let stairs = count("minecraft:oak_stairs", &[("facing", "north"), ("half", "bottom"), ("shape", "straight"), ("waterlogged", "false")]);
        let poppy = count("minecraft:poppy", &[]);
        eprintln!("faces: pane {pane}, slab {slab}, stairs {stairs}, poppy {poppy}");
        assert_eq!(slab, 6);
        assert!(stairs >= 10 && poppy == 4 && pane > 6, "a cross is two planes, seen from both sides: four faces");
    }
}

