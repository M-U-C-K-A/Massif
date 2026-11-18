//! A node's settings, checked against its type and with defaults filled in.

use crate::compile::Diagnostic;
use crate::registry::{NodeSpec, ParamKind};
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
                };
                (p.name.to_owned(), j)
            })
            .collect()
    }
}
