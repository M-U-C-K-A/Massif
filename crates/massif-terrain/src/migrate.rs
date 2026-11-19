//! Graphs of Massif 1.2 (`massif_core::graph`) turned into terrain graphs:
//! each node into the node — or the few nodes — doing the same, with its
//! settings, its links, its place in the editor and its seed. What has no
//! node of its own runs 1.2's code (the `legacy.*` nodes), so that nothing a
//! graph did is lost. The noises are not 1.2's: patterns differ, shapes and
//! settings stay.

use crate::compile::Diagnostic;
use crate::model::{Graph, Node, NodeUi};
use massif_core::graph::{self as v1, CellStyle, CombineOp, MountainStyle, NodeId, NodeKind, NoiseStyle, ShapeKind, TintMode};
use serde_json::{Value, json};
use std::collections::HashMap;

/// What a graph of 1.2 is migrated against.
pub struct World<'a> {
    /// The terrain's side, in blocks (the graph's own size, or the map's).
    pub side: u32,
    /// The world's sea level, in blocks.
    pub sea_level: f32,
    /// The name of each of the world's terrains, by id.
    pub terrain_name: &'a dyn Fn(u16) -> String,
}

/// The Map Output's name in a migrated graph.
pub const OUT: &str = "out";

/// How many terrains and layers the Map Output and Terrain layers take.
const TERRAIN_SLOTS: usize = 6;
const LAYER_SLOTS: usize = 4;

struct Mig<'a> {
    w: &'a World<'a>,
    g: Graph,
    diags: Vec<Diagnostic>,
    /// What each output of 1.2 became, as inputs name it.
    outs: HashMap<(NodeId, u8), String>,
    /// Where each input of 1.2 goes: inputs of the new nodes.
    ins: HashMap<(NodeId, u8), Vec<(String, String)>>,
    out: Node,
    /// Each terrain output's Terrain layers node and slot, six a node.
    terrain_slots: HashMap<NodeId, (String, usize)>,
    layers: usize,
    palette: Option<&'static str>,
}

fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// The way a direction in degrees (0 north, 90 east) points, as a step on
/// the terrain (east, south).
fn heading(degrees: f32) -> (f64, f64) {
    let a = (degrees as f64).to_radians();
    (a.sin(), -a.cos())
}

/// A straight line through the middle of the terrain along `degrees`, its
/// ends `reach` from the middle (fractions of the terrain).
fn line(degrees: f32, reach: f64, points: usize) -> Value {
    let (dx, dz) = heading(degrees);
    Value::Array(
        (0..points)
            .map(|k| {
                let t = -reach + 2.0 * reach * k as f64 / (points - 1) as f64;
                json!({ "x": 0.5 + dx * t, "z": 0.5 + dz * t })
            })
            .collect(),
    )
}

/// 1.2's curve: a gamma, then an S (contrast above 0) or its inverse.
fn v1_curve(t: f32, gamma: f32, contrast: f32) -> f32 {
    let t = t.clamp(0.0, 1.0).powf(gamma.clamp(0.05, 20.0));
    let c = contrast.clamp(-1.0, 1.0);
    if c >= 0.0 {
        t + (t * t * (3.0 - 2.0 * t) - t) * c
    } else {
        let inv = 0.5 - ((1.0 - 2.0 * t).clamp(-1.0, 1.0).asin() / 3.0).sin();
        t + (inv - t) * -c
    }
}

fn palette_name(p: v1::BlockPalette) -> &'static str {
    match p {
        v1::BlockPalette::Natural => "natural",
        v1::BlockPalette::Rock => "rock",
        v1::BlockPalette::Desert => "desert",
        v1::BlockPalette::Snowy => "snowy",
        v1::BlockPalette::Everything => "every",
    }
}

impl Mig<'_> {
    fn warn(&mut self, n: &v1::Node, message: impl Into<String>) {
        self.diags.push(Diagnostic::warning(format!("1.2.nodes.{}", n.id), format!("{} (1.2): {}", n.kind.label(), message.into())));
    }

    /// A new node of type `kind` with `params`, `dx` to the right of 1.2's node.
    fn add(&mut self, n: &v1::Node, kind: &str, params: Value, dx: f32) -> String {
        let mut node = Node::new(kind);
        if let Value::Object(o) = params {
            node.params.extend(o);
        }
        let id = self.g.add(node);
        self.g.ui.insert(id.clone(), NodeUi { pos: [n.pos[0] + dx, n.pos[1]], collapsed: false });
        id
    }

    /// 1.2's output `k` is now `source`.
    fn output(&mut self, n: &v1::Node, k: u8, source: impl Into<String>) {
        self.outs.insert((n.id, k), source.into());
    }

    /// 1.2's input `k` now feeds `input` of `node`.
    fn input(&mut self, n: &v1::Node, k: u8, node: &str, input: &str) {
        self.ins.entry((n.id, k)).or_default().push((node.to_owned(), input.to_owned()));
    }

    /// One node for one, its inputs and outputs by name in order.
    fn one(&mut self, n: &v1::Node, kind: &str, params: Value, inputs: &[&str], outputs: &[&str]) -> String {
        let id = self.add(n, kind, params, 0.0);
        for (k, name) in inputs.iter().enumerate() {
            self.input(n, k as u8, &id, name);
        }
        for (k, name) in outputs.iter().enumerate() {
            let src = if k == 0 { id.clone() } else { format!("{id}.{name}") };
            self.output(n, k as u8, src);
        }
        id
    }

    /// The Map node, made once.
    fn map_node(&mut self, n: &v1::Node) -> String {
        if let Some((id, _)) = self.g.nodes.iter().find(|(_, node)| node.kind == "input.map") {
            return id.clone();
        }
        self.add(n, "input.map", json!({}), -260.0)
    }

    fn node(&mut self, n: &v1::Node) {
        let side = self.w.side.max(1) as f64;
        let sea = self.w.sea_level;
        use NodeKind as K;
        match &n.kind {
            K::Noise { style, scale, octaves, roughness, height, base, seed } => {
                let style = match style {
                    NoiseStyle::Fbm => "fbm",
                    NoiseStyle::Ridged => "ridged",
                    NoiseStyle::Billow => "billow",
                };
                self.one(n, "noise.fbm", json!({ "style": style, "scale": scale, "octaves": octaves, "roughness": roughness, "height": height, "base": base, "seed": seed }), &[], &["height"]);
            }
            K::Cells { style, scale, jitter, height, base, seed } => {
                // Cones: high at each point, down with the distance to it.
                let (mode, height, base) = match style {
                    CellStyle::Cones => ("f1", -height, base + height),
                    CellStyle::Ridges => ("edges", *height, *base),
                    CellStyle::Plates => ("cells", *height, *base),
                };
                self.one(n, "noise.cells", json!({ "mode": mode, "scale": scale, "jitter": jitter, "height": height, "base": base, "seed": seed }), &[], &["height"]);
            }
            K::Radial { inner, outer, height, base } => {
                let o = outer.max(0.01);
                let profile = if inner < outer { json!([[0.0, 1.0], [inner / o, 1.0], [1.0, 0.0]]) } else { json!([[0.0, 1.0], [1.0, 0.0]]) };
                let ends = json!([{ "x": 0.5, "z": 0.5 }, { "x": 0.5 + 0.5 * o, "z": 0.5 }]);
                self.one(n, "pattern.gradient", json!({ "kind": "radial", "ends": ends, "profile": profile, "height": height, "base": base }), &[], &["value"]);
            }
            K::Constant { value } => {
                self.one(n, "pattern.constant", json!({ "value": value }), &[], &["height"]);
            }
            K::MapHeight => {
                let id = self.map_node(n);
                self.output(n, 0, id.clone());
                self.output(n, 1, format!("{id}.water"));
                self.output(n, 2, format!("{id}.color"));
                self.output(n, 3, format!("{id}.level"));
            }
            K::FileInput { path, low, high, fit } => {
                if v1::is_world(std::path::Path::new(path)) {
                    self.one(n, "legacy.file_input", json!({ "path": path, "low": low, "high": high, "fit": fit }), &[], &["height", "water", "colour"]);
                } else {
                    let id = self.one(n, "input.heightmap", json!({ "file": path, "low": low, "high": high, "fit": "stretch" }), &[], &["height"]);
                    if !fit {
                        self.warn(n, format!("the height map is stretched over the terrain in “{id}”"));
                    }
                }
            }
            K::LinearGradient { angle, height, base } => {
                self.one(n, "pattern.gradient", json!({ "kind": "linear", "ends": line(*angle, 0.5, 2), "height": height, "base": base }), &[], &["value"]);
            }
            K::Shape { shape, x, z, radius, falloff, height, base } => match shape {
                ShapeKind::Circle | ShapeKind::Square => {
                    let name = if *shape == ShapeKind::Circle { "circle" } else { "square" };
                    let mask = self.add(n, "pattern.shape", json!({ "shape": name, "center": [{ "x": x, "z": z }], "radius": radius, "softness": falloff * radius }), 0.0);
                    let lv = self.add(n, "adjust.levels", json!({ "in_low": 0.0, "in_high": 1.0, "out_low": base, "out_high": base + height }), 200.0);
                    self.g.link(&mask, &lv, "in");
                    self.output(n, 0, lv);
                }
                ShapeKind::Cone | ShapeKind::Dome => {
                    let profile = if *shape == ShapeKind::Cone { json!([[0.0, 1.0], [1.0, 0.0]]) } else { json!([[0.0, 1.0], [0.5, 0.87], [0.8, 0.6], [1.0, 0.0]]) };
                    let ends = json!([{ "x": x, "z": z }, { "x": *x as f64 + *radius as f64 / side, "z": z }]);
                    self.one(n, "pattern.gradient", json!({ "kind": "radial", "ends": ends, "profile": profile, "height": height, "base": base }), &[], &["value"]);
                }
            },
            K::Mountain { style, x, z, radius, height, base, detail, seed } => {
                let mut p = json!({ "peaks": [{ "x": x, "z": z, "height": height, "radius": radius }], "base": base, "roughness": detail, "seed": seed });
                match style {
                    MountainStyle::Alpine => p["arete_strength"] = json!(0.7),
                    MountainStyle::Rounded => {
                        p["arete_strength"] = json!(0.2);
                        p["sharpness"] = json!(0.12);
                        p["profile"] = json!([[0.0, 0.0], [0.3, 0.3], [0.7, 0.82], [1.0, 1.0]]);
                    }
                    MountainStyle::Eroded => {
                        p["arete_strength"] = json!(0.5);
                        p["roughness"] = json!(0.6);
                    }
                    MountainStyle::Spiky => {
                        p["arete_strength"] = json!(0.9);
                        p["sharpness"] = json!(1.0);
                    }
                }
                self.one(n, "mountain.peaks", p, &[], &["height"]);
            }
            K::MountainRange { angle, width, scale, height, base, seed } => {
                self.one(n, "mountain.range", json!({ "spine": line(*angle, 0.42, 3), "width": width, "peak_spacing": scale, "height": height, "base": base, "seed": seed }), &[], &["height"]);
            }
            K::Volcano { x, z, radius, height, base, crater, depth, seed } => {
                self.one(n, "mountain.volcano", json!({ "volcanoes": [{ "x": x, "z": z, "height": height, "radius": radius }], "crater": crater, "crater_depth": depth, "base": base, "seed": seed }), &[], &["height"]);
            }
            K::Crater { x, z, radius, depth, rim, peak, base, seed } => {
                let p = json!({ "craters": [{ "x": x, "z": z, "radius": radius, "depth": depth }], "count": 0, "rim": (rim / depth.max(1.0)).clamp(0.0, 1.0), "peak_size": if *peak > 0.0 { radius * 0.3 } else { 0.0 }, "base": base, "seed": seed });
                self.one(n, "crater.field", p, &[], &["height"]);
            }
            K::CraterField { count, min_radius, max_radius, depth, base, seed } => {
                self.one(n, "crater.field", json!({ "count": count, "smallest": min_radius, "largest": max_radius, "depth_ratio": depth, "base": base, "seed": seed }), &[], &["height"]);
            }
            K::Dunes { angle, wavelength, height, base, sharpness, seed } => {
                self.one(n, "land.dunes", json!({ "kind": "transverse", "direction": angle, "wavelength": wavelength, "height": height, "base": base, "irregularity": (1.0 - sharpness).clamp(0.0, 1.0), "seed": seed }), &[], &["height"]);
            }
            K::Canyon { angle, width, depth, meander, steps, base, seed } => {
                // 1.2's course, point by point: across = wind(along).
                let noise = massif_core::noise::Noise::new(*seed as u64 ^ 0xca71);
                let (s, c) = (*angle as f64).to_radians().sin_cos();
                let wave = meander.max(1.0) as f64 * 3.0;
                let reach = side * 0.75;
                let count = ((2.0 * reach / (wave / 4.0)).ceil() as usize).clamp(8, 160);
                let path: Vec<Value> = (0..=count)
                    .map(|k| {
                        let a = -reach + 2.0 * reach * k as f64 / count as f64;
                        let w = noise.fbm(a / wave, 0.3, 4, 2.0, 0.5) * *meander as f64 * 1.6;
                        json!({ "x": 0.5 + (s * a + c * w) / side, "z": 0.5 + (-c * a + s * w) / side })
                    })
                    .collect();
                let p = json!({ "path": path, "width": width, "depth": depth, "meander": 0.0, "steps": steps, "base": base, "seed": seed });
                self.one(n, "valley.canyon", p, &[], &["height"]);
            }
            K::Mesas { scale, coverage, height, base, steps, seed } => {
                self.one(n, "plateau.mesa", json!({ "scale": scale, "coverage": coverage, "height": height, "base": base, "layers": steps, "seed": seed }), &[], &["height"]);
            }
            K::Island { size, height, base, coast, seed } => {
                // 1.2's land faded out to `size / 2` of the terrain; its shore
                // lay where the faded relief crossed the sea.
                let r0 = size.clamp(0.05, 1.0) as f64 * side / 2.0;
                let q = (((sea - base) / height.max(1.0)) as f64 / 0.775).clamp(0.0, 1.0).powf(1.0 / 1.4);
                let t = 0.5 - ((1.0 - 2.0 * q).clamp(-1.0, 1.0).asin() / 3.0).sin();
                let shore = r0 * (1.0 - 0.75 * t);
                let p = json!({ "center": [{ "x": 0.5, "z": 0.5 }], "radius": shore.max(16.0), "height": (base + height - sea).max(1.0), "depth": (sea - base).max(0.0), "roughness": coast, "seed": seed });
                self.one(n, "coast.island", p, &[], &["height"]);
            }
            K::FloatingIslands { altitude, spread, size, coverage, depth, seed } => {
                self.one(n, "legacy.floating_islands", json!({ "altitude": altitude, "spread": spread, "size": size, "coverage": coverage, "depth": depth, "seed": seed }), &[], &["top", "bottom", "islands"]);
            }
            K::Rocks { scale, height, sharpness, seed } => {
                self.one(n, "legacy.rocks", json!({ "scale": scale, "height": height, "sharpness": sharpness, "seed": seed }), &[], &["out"]);
            }
            K::Tectonic { plates, relief, width, roughness, sea, seed } => {
                let plates = serde_json::to_string(plates).unwrap_or_else(|_| "[]".to_owned());
                self.one(n, "legacy.tectonic", json!({ "plates": plates, "relief": relief, "width": width, "roughness": roughness, "sea": sea, "seed": seed }), &[], &["height", "ranges", "faults"]);
            }
            K::Levels { in_min, in_max, out_min, out_max, gamma } => {
                // 1.2 raised t to gamma; the levels now to 1 / gamma.
                self.one(n, "adjust.levels", json!({ "in_low": in_min, "in_high": in_max, "out_low": out_min, "out_high": out_max, "gamma": 1.0 / gamma.max(0.01) }), &["in"], &["out"]);
            }
            K::Terrace { steps, strength } => {
                self.one(n, "adjust.terrace", json!({ "steps": steps, "strength": strength, "sharpness": 0.85, "range": "own" }), &["in"], &["out"]);
            }
            K::Clamp { min, max } => {
                self.one(n, "adjust.clamp", json!({ "min": min, "max": max }), &["in"], &["out"]);
            }
            K::Blur { radius } => {
                // Two box passes of `radius` samples: a Gaussian of about 0.8 radius.
                self.one(n, "adjust.blur", json!({ "radius": radius * 2.5 }), &["in"], &["out"]);
            }
            K::Warp { amount, scale, seed } => {
                self.one(n, "adjust.warp", json!({ "amount": amount, "scale": scale, "seed": seed }), &["in"], &["out"]);
            }
            K::Normalize { min, max } => {
                self.one(n, "adjust.normalize", json!({ "low": min, "high": max }), &["in"], &["out"]);
            }
            K::Curve { gamma, contrast } => {
                let curve: Vec<[f32; 2]> = (0..=16).map(|k| k as f32 / 16.0).map(|t| [t, v1_curve(t, *gamma, *contrast)]).collect();
                self.one(n, "adjust.curve", json!({ "curve": curve, "range": "own" }), &["in"], &["out"]);
            }
            K::Transform { scale, rotate, offset_x, offset_z } => {
                self.one(n, "adjust.transform", json!({ "scale": scale, "rotate": rotate, "offset_x": offset_x, "offset_z": offset_z }), &["in"], &["out"]);
            }
            K::Swirl { angle, radius } => {
                self.one(n, "adjust.swirl", json!({ "angle": angle, "radius": radius }), &["in"], &["out"]);
            }
            K::Fold { level, strength } => {
                self.one(n, "adjust.fold", json!({ "level": level, "strength": strength }), &["in"], &["out"]);
            }
            K::Stratify { layers, strength, tilt, seed } => {
                self.one(n, "adjust.stratify", json!({ "layers": layers, "strength": strength, "tilt": tilt, "seed": seed }), &["in"], &["out"]);
            }
            K::Sharpen { radius, amount } => {
                self.one(n, "adjust.sharpen", json!({ "radius": radius * 2.5, "amount": amount }), &["in"], &["out"]);
            }
            K::InvertHeight => {
                self.one(n, "adjust.invert", json!({}), &["in"], &["out"]);
            }
            K::Overhang { reach, thickness, drop, variation, seed } => {
                self.one(n, "legacy.overhang", json!({ "reach": reach, "thickness": thickness, "drop": drop, "variation": variation, "seed": seed }), &["in"], &["top", "bottom"]);
            }
            K::Erosion { rain, erosion, deposition, capacity, evaporation, radius, seed } => {
                let p = json!({ "rain": rain, "erosion": erosion, "deposition": deposition, "capacity": capacity, "evaporation": evaporation, "radius": radius, "seed": seed });
                self.one(n, "legacy.erosion", p, &["in"], &["height", "flow", "deposits", "wear"]);
            }
            K::Thermal { talus, iterations, strength } => {
                self.one(n, "erosion.thermal", json!({ "angle": talus, "iterations": iterations, "strength": strength }), &["height"], &["height", "talus"]);
            }
            K::Snow { line, slope, depth, softness } => {
                let id = self.add(n, "mask.snow", json!({ "line": line, "max_slope": slope, "depth": depth, "softness": softness }), 0.0);
                self.input(n, 0, &id, "height");
                self.output(n, 0, format!("{id}.height"));
                self.output(n, 1, id);
            }
            K::Rivers { threshold, depth, width, sea, banks, lakes } => {
                let p = json!({ "threshold": threshold, "depth": depth, "width": width, "sea": sea, "banks": banks, "lakes": lakes });
                self.one(n, "legacy.rivers", p, &["in", "water_level"], &["height", "rivers", "water"]);
            }
            K::Lakes { amount, sea } => {
                self.one(n, "legacy.lakes", json!({ "amount": amount, "sea": sea }), &["in"], &["height", "lakes", "water"]);
            }
            K::Sea { level, shore } => {
                self.one(n, "legacy.sea", json!({ "level": level, "shore": shore }), &["in"], &["height", "water", "shore"]);
            }
            K::Flow { spread } => {
                self.one(n, "legacy.flow", json!({ "spread": spread }), &["in"], &["mask"]);
            }
            K::Combine { op, weight } => {
                let mode = match op {
                    CombineOp::Add => "add",
                    CombineOp::Subtract => "subtract",
                    CombineOp::Multiply => "multiply",
                    CombineOp::Max => "max",
                    CombineOp::Min => "min",
                    CombineOp::Average => "average",
                    CombineOp::Lerp => "replace",
                    CombineOp::Screen => "screen",
                    CombineOp::Overlay => "overlay",
                    CombineOp::Difference => "difference",
                };
                if *op == CombineOp::Lerp || *weight == 1.0 {
                    self.one(n, "combine.blend", json!({ "mode": mode, "strength": weight.clamp(0.0, 1.0) }), &["a", "b"], &["height"]);
                } else {
                    // 1.2 scaled B by the weight before combining.
                    let w = self.add(n, "pattern.constant", json!({ "value": weight }), -400.0);
                    let scaled = self.add(n, "combine.blend", json!({ "mode": "multiply", "strength": 1.0 }), -200.0);
                    let id = self.add(n, "combine.blend", json!({ "mode": mode, "strength": 1.0 }), 0.0);
                    self.g.link(&w, &scaled, "b");
                    self.g.link(&scaled, &id, "b");
                    self.input(n, 0, &id, "a");
                    self.input(n, 1, &scaled, "a");
                    self.output(n, 0, id);
                }
            }
            K::Blend => {
                self.one(n, "combine.blend", json!({ "mode": "replace", "strength": 1.0 }), &["a", "b", "mask"], &["height"]);
            }
            K::SelectHeight { min, max, falloff } => {
                self.one(n, "mask.height", json!({ "from": min.clamp(-2048.0, 4096.0), "to": max.clamp(-2048.0, 4096.0), "softness": falloff.clamp(0.0, 1024.0) }), &["height"], &["mask"]);
            }
            K::SelectSlope { min, max, falloff } => {
                let p = json!({ "from": (min - falloff / 2.0).max(0.0), "to": (min + falloff / 2.0).clamp(0.0, 90.0), "upper": max.clamp(0.0, 90.0) });
                self.one(n, "mask.slope", p, &["height"], &["mask"]);
            }
            K::Invert => {
                self.one(n, "mask.invert", json!({}), &["mask"], &["mask"]);
            }
            K::Curvature { bend, radius, strength } => {
                let mode = if *bend == v1::Bend::Convex { "convex" } else { "concave" };
                self.one(n, "mask.curvature", json!({ "mode": mode, "scale": radius, "sensitivity": (strength * 0.5).clamp(0.0, 1.0) }), &["height"], &["mask"]);
            }
            K::Facing { angle, spread } => {
                self.one(n, "mask.facing", json!({ "direction": angle, "spread": spread }), &["height"], &["mask"]);
            }
            K::Occlusion { radius } => {
                self.one(n, "mask.occlusion", json!({ "radius": radius }), &["height"], &["mask"]);
            }
            K::Gradient { preset, auto_range, min, max, reverse, variation, seed } => {
                let stops: Vec<Value> = preset.stops().iter().map(|(at, c)| json!({ "at": at, "color": hex(*c) })).collect();
                let p = json!({ "preset": "custom", "gradient": stops, "low": min, "high": max, "auto": auto_range, "reverse": reverse, "variation": variation, "seed": seed });
                self.one(n, "color.gradient", p, &["in"], &["color"]);
            }
            K::SolidColor { color } => {
                self.one(n, "color.solid", json!({ "color": hex(*color) }), &[], &["color"]);
            }
            K::MixColors => {
                self.one(n, "color.mix", json!({ "mode": "mix" }), &["a", "b", "mask"], &["color"]);
            }
            K::AdjustColor { hue, saturation, brightness, contrast } => {
                self.one(n, "color.adjust", json!({ "hue": hue, "saturation": saturation, "brightness": brightness, "contrast": contrast }), &["color"], &["color"]);
            }
            K::Tint { color, mode, strength } => {
                let mode = match mode {
                    TintMode::Mix => "mix",
                    TintMode::Multiply => "multiply",
                    TintMode::Screen => "screen",
                };
                let tint = self.add(n, "color.solid", json!({ "color": hex(*color) }), -200.0);
                let mix = self.add(n, "color.mix", json!({ "mode": mode, "strength": strength }), 0.0);
                self.g.link(&tint, &mix, "b");
                self.input(n, 0, &mix, "a");
                self.input(n, 1, &mix, "mask");
                self.output(n, 0, mix);
            }
            K::AutoColor { palette, snow_line, rock_slope, seed } => {
                let palette = palette.label().to_ascii_lowercase();
                self.one(n, "legacy.auto_color", json!({ "palette": palette, "snow_line": snow_line, "rock_slope": rock_slope, "seed": seed }), &["height"], &["colour"]);
            }
            K::Temperature { warm, cold, toward, lapse, variation, seed, .. } => {
                self.one(n, "climate.temperature", json!({ "toward": toward, "warm": warm, "cold": cold, "lapse": lapse, "variation": variation, "seed": seed }), &["height"], &["temperature"]);
            }
            K::Humidity { from, reach, shadow, variation, seed, .. } => {
                let hum = self.add(n, "climate.humidity", json!({ "inland": reach.max(1.0), "rain_weight": shadow.clamp(0.0, 1.0), "variation": variation, "seed": seed }), 0.0);
                self.input(n, 0, &hum, "height");
                if *shadow > 0.0 {
                    // 1.2 said where the wind blows from; rainfall, where to.
                    let rain = self.add(n, "climate.rainfall", json!({ "direction": (from + 180.0).rem_euclid(360.0) }), -220.0);
                    self.input(n, 0, &rain, "height");
                    self.g.link(&rain, &hum, "rainfall");
                }
                self.output(n, 0, hum);
            }
            K::Biomes { .. } => {
                let id = self.add(n, "climate.biomes", json!({}), 0.0);
                for (k, name) in ["height", "temperature", "humidity"].iter().enumerate() {
                    self.input(n, k as u8, &id, name);
                }
                self.output(n, 0, format!("{id}.color"));
            }
            K::HeightOutput => self.input(n, 0, OUT, "height"),
            K::WaterOutput => self.input(n, 0, OUT, "water"),
            K::TerrainOutput { .. } => {
                if let Some((node, slot)) = self.terrain_slots.get(&n.id).cloned() {
                    self.input(n, 0, &node, &format!("mask_{slot}"));
                }
            }
            K::ColorOutput { blocks, palette } => {
                if *blocks || !self.terrain_slots.is_empty() {
                    self.input(n, 0, OUT, "albedo");
                    self.palette = Some(palette_name(*palette));
                } else {
                    self.warn(n, "the colours 1.2 only showed are not written to the map");
                }
            }
            K::LayerOutput { palette, .. } => {
                if self.layers < LAYER_SLOTS {
                    self.layers += 1;
                    let k = self.layers;
                    for (i, part) in ["top", "bottom", "color"].iter().enumerate() {
                        self.input(n, i as u8, OUT, &format!("level.{k}.{part}"));
                    }
                    self.palette.get_or_insert(palette_name(*palette));
                } else {
                    self.warn(n, format!("the Map Output takes {LAYER_SLOTS} layers: this one is left out"));
                }
            }
            K::BiomeOutput { paint, .. } => {
                let id = self.add(n, "climate.biomes", json!({}), -220.0);
                self.input(n, 0, &id, "height");
                self.input(n, 1, &id, "temperature");
                self.input(n, 2, &id, "humidity");
                self.input(n, 1, OUT, "temperature");
                self.input(n, 2, OUT, "humidity");
                self.g.link(&id, OUT, "biome");
                if *paint {
                    self.out.params.insert("paint_biomes".into(), json!(true));
                }
            }
        }
    }
}

/// Numbers as 1.2 meant them: its settings are 32-bit, which written as
/// they widen read 0.699999988 for 0.7.
fn tidy(v: &mut Value) {
    match v {
        Value::Number(n) if n.is_f64() => {
            let x = n.as_f64().unwrap_or(0.0);
            let r = format!("{:.6}", x).parse::<f64>().unwrap_or(x);
            *v = if r.fract() == 0.0 && r.abs() < 1e15 { json!(r as i64) } else { json!(r) };
        }
        Value::Array(a) => a.iter_mut().for_each(tidy),
        Value::Object(o) => o.values_mut().for_each(tidy),
        _ => {}
    }
}

/// `old` as a terrain graph, and what could not be carried over as it was.
pub fn from_v1(old: &v1::Graph, world: &World) -> (Graph, Vec<Diagnostic>) {
    let mut m = Mig { w: world, g: Graph::default(), diags: Vec::new(), outs: HashMap::new(), ins: HashMap::new(), out: Node::new("output.map"), terrain_slots: HashMap::new(), layers: 0, palette: None };
    m.g.terrain.size = old.size;
    // The terrain outputs, six to a Terrain layers node, each laid over
    // the one before (the first over the map's terrains), later winning.
    let terrains: Vec<&v1::Node> = old.nodes.iter().filter(|n| matches!(n.kind, NodeKind::TerrainOutput { .. })).collect();
    let chain: Vec<String> = (0..terrains.len().div_ceil(TERRAIN_SLOTS)).map(|c| if c == 0 { "terrains".to_owned() } else { format!("terrains_{}", c + 1) }).collect();
    for (k, n) in terrains.iter().enumerate() {
        m.terrain_slots.insert(n.id, (chain[k / TERRAIN_SLOTS].clone(), k % TERRAIN_SLOTS + 1));
    }
    // The Map Output is linked to by name before it is added.
    m.g.nodes.insert(OUT.to_owned(), Node::new("output.map"));
    for n in &old.nodes {
        m.node(n);
    }
    if let Some(first) = terrains.first() {
        let map = m.map_node(first);
        let mut under = format!("{map}.terrain");
        for (c, name) in chain.iter().enumerate() {
            let mut layers = Node::new("terrain.layers");
            for slot in 1..=TERRAIN_SLOTS {
                let terrain = terrains.get(c * TERRAIN_SLOTS + slot - 1).and_then(|n| if let NodeKind::TerrainOutput { terrain } = n.kind { Some((world.terrain_name)(terrain)) } else { None });
                layers.params.insert(format!("terrain_{slot}"), json!(terrain.unwrap_or_default()));
            }
            m.g.nodes.insert(name.clone(), layers);
            m.g.ui.insert(name.clone(), NodeUi { pos: [first.pos[0] + 220.0 * c as f32, first.pos[1] + 120.0], collapsed: false });
            m.g.link(&under, name, "under");
            under = name.clone();
        }
        m.g.link(&under, OUT, "terrain");
    }
    for l in &old.links {
        let Some(src) = m.outs.get(&(l.from, l.output)).cloned() else {
            if let Some(n) = old.node(l.from) {
                m.warn(n, format!("its output {} feeds nothing now", l.output + 1));
            }
            continue;
        };
        for (node, input) in m.ins.get(&(l.to, l.input)).cloned().unwrap_or_default() {
            m.g.link(&src, &node, &input);
        }
    }
    // The Map Output, with what the nodes above linked to it.
    let mut out = m.g.nodes.remove(OUT).expect("the Map Output");
    out.params.extend(std::mem::take(&mut m.out.params));
    if let Some(p) = m.palette.filter(|p| *p != "natural") {
        out.params.insert("albedo_palette".into(), json!(p));
    }
    let right = old.nodes.iter().map(|n| n.pos[0]).fold(0.0f32, f32::max) + 260.0;
    let middle = old.nodes.iter().map(|n| n.pos[1]).sum::<f32>() / old.nodes.len().max(1) as f32;
    m.g.nodes.insert(OUT.to_owned(), out);
    m.g.ui.insert(OUT.to_owned(), NodeUi { pos: [right, middle], collapsed: false });
    for n in m.g.nodes.values_mut() {
        n.params.values_mut().for_each(tidy);
    }
    (m.g, m.diags)
}
