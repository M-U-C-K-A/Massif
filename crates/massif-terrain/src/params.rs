//! A node's settings, checked against its type and with defaults filled in.

use crate::compile::Diagnostic;
use crate::registry::{Field, NodeSpec, ParamKind};
use crate::util::{closest, hash_str};
use std::collections::BTreeMap;
use std::hash::Hasher;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Num(f64),
    Bool(bool),
    /// The index of the choice.
    Choice(usize),
    /// sRGB, from 0 to 1.
    Color([f32; 3]),
    Seed(u32),
    /// Points or a curve's control points, in order.
    Points(Vec<Point>),
    /// Control points of a profile, by increasing x, both from 0 to 1.
    Profile(Vec<[f64; 2]>),
    /// Colour stops (sRGB) by increasing position, from 0 to 1.
    Gradient(Vec<(f64, [f32; 3])>),
    /// Text, or a file's path.
    Text(String),
}

/// A point placed on the terrain: where (fractions of its side), and the
/// values its setting's fields name, in their order.
#[derive(Debug, Clone, PartialEq)]
pub struct Point {
    pub x: f64,
    pub z: f64,
    pub values: Vec<f64>,
}

impl Point {
    /// The value of field `k`.
    pub fn value(&self, k: usize) -> f64 {
        self.values.get(k).copied().unwrap_or(0.0)
    }
}

/// A colour written `#rrggbb`, as sRGB from 0 to 1.
pub fn parse_color(s: &str) -> Option<[f32; 3]> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = |k: usize| u8::from_str_radix(&hex[k..k + 2], 16).ok().map(|b| b as f32 / 255.0);
    Some([v(0)?, v(2)?, v(4)?])
}

/// `#rrggbb` of an sRGB colour.
pub fn color_hex(c: [f32; 3]) -> String {
    let b = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{:02x}{:02x}{:02x}", b[0], b[1], b[2])
}

/// sRGB to linear light.
pub fn linear(c: [f32; 3]) -> [f32; 3] {
    c.map(|v| if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) })
}

/// Every setting of a node, in its type's order.
#[derive(Debug, Clone)]
pub struct Params {
    spec: &'static NodeSpec,
    values: Vec<Value>,
}

impl Params {
    /// The defaults of `spec` for the node named `id` (whose name makes
    /// its default seed).
    pub fn defaults(spec: &'static NodeSpec, id: &str) -> Self {
        let values = spec
            .params
            .iter()
            .map(|p| match p.kind {
                ParamKind::Float { default, .. } => Value::Num(default),
                ParamKind::Int { default, .. } => Value::Num(default as f64),
                ParamKind::Bool { default } => Value::Bool(default),
                ParamKind::Choice { options, default } => Value::Choice(options.iter().position(|o| o.value == default).unwrap_or(0)),
                ParamKind::Color { default } => Value::Color(parse_color(default).unwrap_or([0.5; 3])),
                ParamKind::Points { .. } | ParamKind::Curve { .. } => Value::Points(Vec::new()),
                ParamKind::Profile { default } => Value::Profile(default.to_vec()),
                ParamKind::Gradient { default } => Value::Gradient(default.iter().map(|(at, c)| (*at, parse_color(c).unwrap_or([0.5; 3]))).collect()),
                ParamKind::Text { default } => Value::Text(default.to_owned()),
                ParamKind::File { .. } => Value::Text(String::new()),
                ParamKind::Seed => Value::Seed((hash_str(id) & 0xffff_ffff) as u32),
            })
            .collect();
        Self { spec, values }
    }

    /// The settings a graph gives, checked: what is unknown, of the wrong
    /// kind or out of range is said in `diags` (under `path`), and replaced
    /// by the default or the nearest allowed value.
    pub fn decode(spec: &'static NodeSpec, id: &str, given: &BTreeMap<String, serde_json::Value>, path: &str, diags: &mut Vec<Diagnostic>) -> Self {
        let mut out = Self::defaults(spec, id);
        for (name, v) in given {
            let here = format!("{path}.{name}");
            let Some((k, p)) = spec.param(name) else {
                let hint = closest(name, spec.params.iter().map(|p| p.name)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                diags.push(Diagnostic::error(here, format!("{} has no setting “{name}”{hint}", spec.id)));
                continue;
            };
            let wrong = |diags: &mut Vec<Diagnostic>, what: &str| diags.push(Diagnostic::error(here.clone(), format!("“{name}” takes {what}, not {v}")));
            match p.kind {
                ParamKind::Float { min, max, unit, .. } => match v.as_f64() {
                    Some(x) => {
                        let c = x.clamp(min, max);
                        if c != x {
                            diags.push(Diagnostic::warning(here.clone(), format!("“{name}” goes from {min} to {max} {}: {x} taken as {c}", unit.id())));
                        }
                        out.values[k] = Value::Num(c);
                    }
                    None => wrong(diags, "a number"),
                },
                ParamKind::Int { min, max, .. } => match v.as_f64() {
                    Some(x) if x.fract() == 0.0 => {
                        let c = (x as i64).clamp(min, max);
                        if c as f64 != x {
                            diags.push(Diagnostic::warning(here.clone(), format!("“{name}” goes from {min} to {max}: {x} taken as {c}")));
                        }
                        out.values[k] = Value::Num(c as f64);
                    }
                    _ => wrong(diags, "a whole number"),
                },
                ParamKind::Bool { .. } => match v.as_bool() {
                    Some(b) => out.values[k] = Value::Bool(b),
                    None => wrong(diags, "true or false"),
                },
                ParamKind::Choice { options, .. } => match v.as_str() {
                    Some(s) => match options.iter().position(|o| o.value.eq_ignore_ascii_case(s)) {
                        Some(i) => out.values[k] = Value::Choice(i),
                        None => {
                            let list = options.iter().map(|o| o.value).collect::<Vec<_>>().join(", ");
                            let hint = closest(s, options.iter().map(|o| o.value)).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                            diags.push(Diagnostic::error(here.clone(), format!("“{name}” is one of {list}, not “{s}”{hint}")));
                        }
                    },
                    None => wrong(diags, "a name"),
                },
                ParamKind::Color { .. } => {
                    let from_list = v.as_array().filter(|a| a.len() == 3).and_then(|a| {
                        let c: Option<Vec<f32>> = a.iter().map(|x| x.as_f64().map(|f| f.clamp(0.0, 1.0) as f32)).collect();
                        c.map(|c| [c[0], c[1], c[2]])
                    });
                    match v.as_str().and_then(parse_color).or(from_list) {
                        Some(c) => out.values[k] = Value::Color(c),
                        None => wrong(diags, "a colour, \"#rrggbb\""),
                    }
                }
                ParamKind::Seed => match v.as_u64() {
                    Some(s) => out.values[k] = Value::Seed((s & 0xffff_ffff) as u32),
                    None => wrong(diags, "a whole number from 0"),
                },
                ParamKind::Points { fields, max } => match decode_points(v, fields, &here, diags) {
                    Some(mut pts) => {
                        if pts.len() > max {
                            diags.push(Diagnostic::warning(here.clone(), format!("“{name}” holds at most {max} points; the first {max} are kept")));
                            pts.truncate(max);
                        }
                        out.values[k] = Value::Points(pts);
                    }
                    None => wrong(diags, "a list of points, [{ \"x\": 0.5, \"z\": 0.5 }, …]"),
                },
                ParamKind::Curve { fields, .. } => match decode_points(v, fields, &here, diags) {
                    Some(pts) => out.values[k] = Value::Points(pts),
                    None => wrong(diags, "a list of points, [{ \"x\": 0.5, \"z\": 0.5 }, …]"),
                },
                ParamKind::Profile { .. } => {
                    let pts: Option<Vec<[f64; 2]>> = v.as_array().and_then(|a| {
                        a.iter()
                            .map(|p| {
                                let p = p.as_array().filter(|p| p.len() == 2)?;
                                Some([p[0].as_f64()?.clamp(0.0, 1.0), p[1].as_f64()?.clamp(0.0, 1.0)])
                            })
                            .collect()
                    });
                    match pts.filter(|p| p.len() >= 2) {
                        Some(mut pts) => {
                            pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
                            pts.dedup_by(|a, b| a[0] == b[0]);
                            out.values[k] = Value::Profile(pts);
                        }
                        None => wrong(diags, "two points or more, [[0, 0], [1, 1]]"),
                    }
                }
                ParamKind::Gradient { .. } => {
                    let stops: Option<Vec<(f64, [f32; 3])>> = v.as_array().and_then(|a| {
                        a.iter()
                            .map(|s| match s {
                                serde_json::Value::Object(o) => Some((o.get("at")?.as_f64()?.clamp(0.0, 1.0), parse_color(o.get("color")?.as_str()?)?)),
                                serde_json::Value::Array(p) if p.len() == 2 => Some((p[0].as_f64()?.clamp(0.0, 1.0), parse_color(p[1].as_str()?)?)),
                                _ => None,
                            })
                            .collect()
                    });
                    match stops.filter(|s| !s.is_empty()) {
                        Some(mut stops) => {
                            stops.sort_by(|a, b| a.0.total_cmp(&b.0));
                            out.values[k] = Value::Gradient(stops);
                        }
                        None => wrong(diags, "colour stops, [{ \"at\": 0, \"color\": \"#rrggbb\" }, …]"),
                    }
                }
                ParamKind::Text { .. } | ParamKind::File { .. } => match v.as_str() {
                    Some(t) => out.values[k] = Value::Text(t.to_owned()),
                    None => wrong(diags, "text"),
                },
            }
        }
        out
    }

    pub fn spec(&self) -> &'static NodeSpec {
        self.spec
    }

    fn index(&self, name: &str) -> usize {
        self.spec.param(name).unwrap_or_else(|| panic!("{} has no setting {name}", self.spec.id)).0
    }

    /// A number setting.
    pub fn num(&self, name: &str) -> f64 {
        match self.values[self.index(name)] {
            Value::Num(v) => v,
            ref other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    pub fn f(&self, name: &str) -> f32 {
        self.num(name) as f32
    }

    pub fn int(&self, name: &str) -> i64 {
        self.num(name) as i64
    }

    pub fn flag(&self, name: &str) -> bool {
        match self.values[self.index(name)] {
            Value::Bool(b) => b,
            ref other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// The value of a choice setting.
    pub fn choice(&self, name: &str) -> &'static str {
        let k = self.index(name);
        match (&self.values[k], self.spec.params[k].kind) {
            (Value::Choice(i), ParamKind::Choice { options, .. }) => options[*i].value,
            (other, _) => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// A Points or Curve setting's points.
    pub fn points(&self, name: &str) -> &[Point] {
        match &self.values[self.index(name)] {
            Value::Points(p) => p,
            other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// A profile setting at `x` (0–1): smooth between its points, never
    /// overshooting them (monotone cubic).
    pub fn profile(&self, name: &str, x: f64) -> f64 {
        match &self.values[self.index(name)] {
            Value::Profile(p) => eval_profile(p, x),
            other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// A gradient setting at `t` (0–1), in linear light.
    pub fn gradient(&self, name: &str, t: f64) -> [f32; 3] {
        match &self.values[self.index(name)] {
            Value::Gradient(stops) => eval_gradient(stops, t),
            other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// A text or file setting.
    pub fn text(&self, name: &str) -> &str {
        match &self.values[self.index(name)] {
            Value::Text(t) => t,
            other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// A colour setting, in linear light.
    pub fn color(&self, name: &str) -> [f32; 3] {
        match self.values[self.index(name)] {
            Value::Color(c) => linear(c),
            ref other => panic!("{}.{name} is {other:?}", self.spec.id),
        }
    }

    /// The node's seed, when it has one.
    pub fn seed(&self) -> Option<u32> {
        self.values.iter().find_map(|v| match v {
            Value::Seed(s) => Some(*s),
            _ => None,
        })
    }

    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Sets a number setting (in range).
    pub fn set_num(&mut self, name: &str, v: f64) {
        let k = self.index(name);
        self.values[k] = Value::Num(v);
    }

    /// Feeds every setting to a hasher.
    pub fn hash_into(&self, h: &mut impl Hasher) {
        for v in &self.values {
            match v {
                Value::Num(x) => {
                    h.write_u8(0);
                    h.write_u64(x.to_bits());
                }
                Value::Bool(b) => {
                    h.write_u8(1);
                    h.write_u8(*b as u8);
                }
                Value::Choice(i) => {
                    h.write_u8(2);
                    h.write_u64(*i as u64);
                }
                Value::Seed(s) => {
                    h.write_u8(3);
                    h.write_u32(*s);
                }
                Value::Color(c) => {
                    h.write_u8(4);
                    for v in c {
                        h.write_u32(v.to_bits());
                    }
                }
                Value::Points(pts) => {
                    h.write_u8(5);
                    h.write_u64(pts.len() as u64);
                    for p in pts {
                        h.write_u64(p.x.to_bits());
                        h.write_u64(p.z.to_bits());
                        for v in &p.values {
                            h.write_u64(v.to_bits());
                        }
                    }
                }
                Value::Profile(pts) => {
                    h.write_u8(6);
                    for p in pts {
                        h.write_u64(p[0].to_bits());
                        h.write_u64(p[1].to_bits());
                    }
                }
                Value::Gradient(stops) => {
                    h.write_u8(7);
                    for (at, c) in stops {
                        h.write_u64(at.to_bits());
                        for v in c {
                            h.write_u32(v.to_bits());
                        }
                    }
                }
                Value::Text(t) => {
                    h.write_u8(8);
                    h.write(t.as_bytes());
                    h.write_u8(0xff);
                }
            }
        }
    }

    /// Every setting as a graph would write it.
    pub fn to_json(&self) -> BTreeMap<String, serde_json::Value> {
        self.spec
            .params
            .iter()
            .zip(&self.values)
            .map(|(p, v)| {
                let j = match (v, p.kind) {
                    (Value::Num(x), ParamKind::Int { .. }) => serde_json::Value::from(*x as i64),
                    (Value::Num(x), _) => serde_json::Value::from(*x),
                    (Value::Bool(b), _) => serde_json::Value::from(*b),
                    (Value::Choice(i), ParamKind::Choice { options, .. }) => serde_json::Value::from(options[*i].value),
                    (Value::Choice(i), _) => serde_json::Value::from(*i),
                    (Value::Seed(s), _) => serde_json::Value::from(*s),
                    (Value::Color(c), _) => serde_json::Value::from(color_hex(*c)),
                    (Value::Points(pts), ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. }) => serde_json::Value::Array(
                        pts.iter()
                            .map(|pt| {
                                let mut o = serde_json::Map::new();
                                o.insert("x".into(), pt.x.into());
                                o.insert("z".into(), pt.z.into());
                                for (f, v) in fields.iter().zip(&pt.values) {
                                    o.insert(f.name.into(), (*v).into());
                                }
                                serde_json::Value::Object(o)
                            })
                            .collect(),
                    ),
                    (Value::Points(_), _) => serde_json::Value::Array(Vec::new()),
                    (Value::Profile(pts), _) => serde_json::Value::Array(pts.iter().map(|p| serde_json::json!([p[0], p[1]])).collect()),
                    (Value::Gradient(stops), _) => serde_json::Value::Array(stops.iter().map(|(at, c)| serde_json::json!({ "at": at, "color": color_hex(*c) })).collect()),
                    (Value::Text(t), _) => serde_json::Value::from(t.clone()),
                };
                (p.name.to_owned(), j)
            })
            .collect()
    }
}

/// Points from JSON: objects `{ "x", "z", field… }` or arrays `[x, z,
/// field…]`; fields left out take their defaults, positions are held
/// within the terrain and values within their range.
fn decode_points(v: &serde_json::Value, fields: &[Field], path: &str, diags: &mut Vec<Diagnostic>) -> Option<Vec<Point>> {
    let list = v.as_array()?;
    let mut out = Vec::with_capacity(list.len());
    for (k, item) in list.iter().enumerate() {
        let (x, z, mut values) = match item {
            serde_json::Value::Object(o) => {
                for key in o.keys() {
                    if key != "x" && key != "z" && !fields.iter().any(|f| f.name == key) {
                        let hint = crate::util::closest(key, fields.iter().map(|f| f.name).chain(["x", "z"])).map(|c| format!(" (did you mean “{c}”?)")).unwrap_or_default();
                        diags.push(Diagnostic::error(format!("{path}.{k}.{key}"), format!("a point has no value “{key}”{hint}")));
                    }
                }
                let values = fields.iter().map(|f| o.get(f.name).and_then(|v| v.as_f64()).unwrap_or(f.default)).collect::<Vec<_>>();
                (o.get("x")?.as_f64()?, o.get("z")?.as_f64()?, values)
            }
            serde_json::Value::Array(a) if a.len() >= 2 => {
                let values = fields.iter().enumerate().map(|(i, f)| a.get(2 + i).and_then(|v| v.as_f64()).unwrap_or(f.default)).collect::<Vec<_>>();
                (a[0].as_f64()?, a[1].as_f64()?, values)
            }
            _ => return None,
        };
        for (f, v) in fields.iter().zip(values.iter_mut()) {
            *v = v.clamp(f.min, f.max);
        }
        out.push(Point { x: x.clamp(-1.0, 2.0), z: z.clamp(-1.0, 2.0), values });
    }
    Some(out)
}

/// A monotone cubic (Fritsch–Carlson) through `pts` at `x`, flat beyond
/// the ends.
pub fn eval_profile(pts: &[[f64; 2]], x: f64) -> f64 {
    match pts.len() {
        0 => return x,
        1 => return pts[0][1],
        _ => {}
    }
    if x <= pts[0][0] {
        return pts[0][1];
    }
    if x >= pts[pts.len() - 1][0] {
        return pts[pts.len() - 1][1];
    }
    let k = pts.windows(2).position(|w| x < w[1][0]).unwrap_or(pts.len() - 2);
    let slope = |i: usize| (pts[i + 1][1] - pts[i][1]) / (pts[i + 1][0] - pts[i][0]);
    let tangent = |i: usize| -> f64 {
        if i == 0 {
            return slope(0);
        }
        if i == pts.len() - 1 {
            return slope(i - 1);
        }
        let (a, b) = (slope(i - 1), slope(i));
        if a * b <= 0.0 { 0.0 } else { 2.0 / (1.0 / a + 1.0 / b) }
    };
    let (x0, x1) = (pts[k][0], pts[k + 1][0]);
    let (y0, y1) = (pts[k][1], pts[k + 1][1]);
    let h = x1 - x0;
    let t = (x - x0) / h;
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * y0 + (t3 - 2.0 * t2 + t) * h * tangent(k) + (-2.0 * t3 + 3.0 * t2) * y1 + (t3 - t2) * h * tangent(k + 1)
}

/// Colour stops at `t`, blended in linear light.
pub fn eval_gradient(stops: &[(f64, [f32; 3])], t: f64) -> [f32; 3] {
    let Some(first) = stops.first() else { return [0.5; 3] };
    if t <= first.0 {
        return linear(first.1);
    }
    for w in stops.windows(2) {
        if t <= w[1].0 {
            let f = ((t - w[0].0) / (w[1].0 - w[0].0).max(1e-9)) as f32;
            let (a, b) = (linear(w[0].1), linear(w[1].1));
            return [0, 1, 2].map(|c| a[c] + (b[c] - a[c]) * f);
        }
    }
    linear(stops[stops.len() - 1].1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn profiles_pass_through_their_points_without_overshoot() {
        let pts = [[0.0, 0.0], [0.3, 0.8], [0.6, 0.8], [1.0, 1.0]];
        for p in pts {
            assert!((eval_profile(&pts, p[0]) - p[1]).abs() < 1e-12);
        }
        for k in 0..=100 {
            let y = eval_profile(&pts, k as f64 / 100.0);
            assert!((-1e-9..=1.0 + 1e-9).contains(&y));
        }
        // Flat between equal points: no bump.
        assert!((eval_profile(&pts, 0.45) - 0.8).abs() < 1e-12);
    }

    use crate::registry::{ParamSpec, Unit};

    static FIELDS: &[Field] = &[Field { name: "height", label: "Height", unit: Unit::Blocks, default: 500.0, min: 0.0, max: 4000.0, help: "The peak's height." }];
    static SPEC: NodeSpec = NodeSpec {
        id: "test.points",
        params: &[
            ParamSpec::points("peaks", "Peaks", FIELDS, 3, "Peaks."),
            ParamSpec::curve("spine", "Spine", FIELDS, false, "A spine."),
            ParamSpec::profile("shape", "Shape", &[[0.0, 0.0], [1.0, 1.0]], "A profile."),
            ParamSpec::gradient("colors", "Colours", &[(0.0, "#000000"), (1.0, "#ffffff")], "Colours."),
            ParamSpec::file("image", "Image", &["png"], "An image."),
        ],
        ..NodeSpec::BASE
    };

    #[test]
    fn shapes_drawn_on_the_terrain_read_and_write() {
        let given: BTreeMap<String, serde_json::Value> = serde_json::from_str(
            r##"{ "peaks": [{ "x": 0.1, "z": 0.2, "height": 900 }, { "x": 0.5, "z": 0.5 }, [0.9, 0.9, 5000], { "x": 0, "z": 0 }],
                 "spine": [[0.2, 0.3], [0.5, 0.45, 1200], { "x": 0.8, "z": 0.4, "hieght": 3 }],
                 "shape": [[1, 1], [0, 0], [0.5, 0.9]],
                 "colors": [{ "at": 1, "color": "#ffffff" }, [0, "#112233"]],
                 "image": "maps/alps.png" }"##,
        )
        .unwrap();
        let mut diags = Vec::new();
        let p = Params::decode(&SPEC, "n", &given, "nodes.n.params", &mut diags);
        assert!(diags.iter().any(|d| d.path == "nodes.n.params.peaks" && d.message.contains("at most 3")), "{diags:?}");
        assert!(diags.iter().any(|d| d.path == "nodes.n.params.spine.2.hieght" && d.message.contains("“height”")), "{diags:?}");
        let peaks = p.points("peaks");
        assert_eq!(peaks.len(), 3);
        assert_eq!((peaks[0].x, peaks[0].value(0)), (0.1, 900.0));
        assert_eq!(peaks[1].value(0), 500.0, "a value left out takes its default");
        assert_eq!(peaks[2].value(0), 4000.0, "held within its range");
        assert_eq!(p.points("spine")[1].value(0), 1200.0);
        assert!((p.profile("shape", 0.5) - 0.9).abs() < 1e-12, "points are sorted by x");
        assert_eq!(p.gradient("colors", 0.0), linear(parse_color("#112233").unwrap()));
        assert_eq!(p.text("image"), "maps/alps.png");
        // Written back, read again: the same settings.
        let again = Params::decode(&SPEC, "n", &p.to_json(), "x", &mut Vec::new());
        assert_eq!(again.values(), p.values());
    }

    #[test]
    fn gradients_blend_in_linear_light() {
        let stops = [(0.0, [0.0, 0.0, 0.0]), (1.0, [1.0, 1.0, 1.0])];
        assert_eq!(eval_gradient(&stops, 0.0), [0.0; 3]);
        assert!((eval_gradient(&stops, 0.5)[0] - 0.5).abs() < 1e-6);
        assert_eq!(color_hex(parse_color("#1a2b3c").unwrap()), "#1a2b3c");
    }
}
