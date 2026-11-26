//! The names scripts call nodes by: one CamelCase name for every type of
//! node of the registry and every composite Massif ships, made from the
//! live catalogue (so a new node is callable the day it is registered),
//! with aliases that read like the words people use.

use crate::format::current;
use crate::model::is_component_type;
use crate::registry::{Family, NodeSpec};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::sync::OnceLock;

/// What a name stands for.
#[derive(Debug, Clone, PartialEq)]
pub struct Target {
    /// The node type (`noise.fbm`, `massif:alpine-range`), without version.
    pub kind: String,
    /// Settings an alias gives (`Multiply` is a Blend in mode multiply).
    pub preset: Vec<(&'static str, Value)>,
}

/// Names chosen by hand where the label alone would be vague or taken
/// twice: the rest are the label in CamelCase.
const CANONICAL: &[(&str, &str)] = &[
    ("noise.fbm", "FractalNoise"),
    ("mountain.single", "SingleMountain"),
    ("combine.layers", "Mixer"),
    ("noise.eroded", "ErodedNoise"),
    ("noise.cells", "CellNoise"),
    ("pattern.gradient", "Gradient"),
    ("color.gradient", "ColorGradient"),
    ("color.ramp", "ColorRamp"),
    ("color.solid", "SolidColor"),
    ("color.mix", "MixColors"),
    ("color.adjust", "AdjustColors"),
    ("input.map", "MapInput"),
    ("output.map", "MapOutput"),
    ("mask.height", "HeightMask"),
    ("mask.slope", "SlopeMask"),
    ("mask.curvature", "CurvatureMask"),
    ("mask.facing", "FacingMask"),
    ("mask.exposure", "SunlightMask"),
    ("mask.occlusion", "OpennessMask"),
    ("mask.snow", "SnowMask"),
    ("mask.wetness", "WetnessMask"),
    ("mask.distance", "DistanceMask"),
    ("mask.threshold", "Threshold"),
    ("mask.invert", "InvertMask"),
    ("mask.expand", "GrowMask"),
    ("adjust.invert", "Invert"),
    ("adjust.stratify", "Strata"),
    ("adjust.terrace", "Terraces"),
    ("climate.wind", "Wind"),
    ("wind.erosion", "WindErosion"),
    ("water.flow", "Flow"),
    ("points.mask", "PointsToMask"),
    ("convert.luminance", "Brightness"),
    ("convert.grey", "Grey"),
    ("volume.from_height", "ReliefToVolume"),
    ("volume.sphere", "SphereVolume"),
    ("volume.box", "BoxVolume"),
    ("volume.tube", "TubeVolume"),
    ("pattern.constant", "Constant"),
    ("pattern.shape", "Shape"),
    ("plateau.mesa", "Mesas"),
    ("sediment.deposit", "Sediment"),
    ("ice.glacier", "Glaciers"),
    ("crater.field", "Craters"),
    ("fault.scarp", "Fault"),
];

/// Other names, some with settings of their own.
/// An alias, the type it names, and the settings it gives.
type Alias = (&'static str, &'static str, Vec<(&'static str, Value)>);

fn aliases() -> Vec<Alias> {
    let mode = |m: &str| vec![("mode", json!(m))];
    vec![
        ("Noise", "noise.fbm", vec![]),
        ("Fbm", "noise.fbm", vec![]),
        ("Perlin", "noise.fbm", vec![]),
        ("RidgedNoise", "noise.fbm", vec![("style", json!("ridged"))]),
        ("BillowNoise", "noise.fbm", vec![("style", json!("billow"))]),
        ("Voronoi", "noise.cells", vec![]),
        ("Cells", "noise.cells", vec![]),
        ("Add", "combine.blend", mode("add")),
        ("Subtract", "combine.blend", mode("subtract")),
        ("Multiply", "combine.blend", mode("multiply")),
        ("Max", "combine.blend", mode("max")),
        ("Min", "combine.blend", mode("min")),
        ("Average", "combine.blend", mode("average")),
        ("Difference", "combine.blend", mode("difference")),
        ("Screen", "combine.blend", mode("screen")),
        ("Overlay", "combine.blend", mode("overlay")),
        ("Combine", "combine.blend", vec![]),
        ("Remap", "adjust.levels", vec![]),
        ("DomainWarp", "adjust.warp", vec![]),
        ("HydraulicErosion", "erosion.stream", vec![]),
        ("Erosion", "erosion.stream", vec![]),
        ("Slope", "mask.slope", vec![]),
        ("Rivers", "water.rivers", vec![]),
        ("Output", "output.map", vec![]),
        ("Heightmap", "input.heightmap", vec![]),
        ("ColourRamp", "color.ramp", vec![]),
        ("ColourGradient", "color.gradient", vec![]),
    ]
}

/// `"Mesas and buttes"` → `MesasAndButtes`, `alpine-range` → `AlpineRange`.
pub fn camel(words: &str) -> String {
    let mut out = String::new();
    for w in words.split(|c: char| !c.is_ascii_alphanumeric()).filter(|w| !w.is_empty()) {
        let mut cs = w.chars();
        if let Some(f) = cs.next() {
            out.push(f.to_ascii_uppercase());
            out.push_str(cs.as_str());
        }
    }
    out.replace("Colour", "Color")
}

/// `MountainGenerator` → `mountain_generator`.
pub fn snake(name: &str) -> String {
    let mut out = String::new();
    let mut prev_lower = false;
    for c in name.chars() {
        if c.is_ascii_uppercase() {
            if prev_lower {
                out.push('_');
            }
            out.push(c.to_ascii_lowercase());
            prev_lower = false;
        } else if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_lower = true;
        } else {
            if !out.ends_with('_') && !out.is_empty() {
                out.push('_');
            }
            prev_lower = false;
        }
    }
    out.trim_matches('_').to_owned()
}

/// A type's name in scripts, made from its label when not chosen by hand.
fn made(spec: &NodeSpec) -> String {
    if let Some((_, n)) = CANONICAL.iter().find(|(id, _)| *id == spec.id) {
        return (*n).to_owned();
    }
    let label = spec.label.split(" (").next().unwrap_or(spec.label);
    let base = camel(label);
    if spec.id.starts_with("legacy.") { format!("Legacy{base}") } else { base }
}

struct Table {
    /// Canonical name by type.
    by_kind: BTreeMap<String, String>,
    /// Every name and alias, to what it stands for.
    by_name: BTreeMap<String, Target>,
}

fn table() -> &'static Table {
    static T: OnceLock<Table> = OnceLock::new();
    T.get_or_init(|| {
        let mut by_kind = BTreeMap::new();
        let mut by_name = BTreeMap::new();
        for s in crate::format::public() {
            let name = made(s);
            by_kind.entry(s.id.to_owned()).or_insert_with(|| name.clone());
            by_name.entry(name).or_insert_with(|| Target { kind: s.id.to_owned(), preset: vec![] });
        }
        for (kind, _, _) in crate::components::all() {
            let name = camel(kind.trim_start_matches("massif:"));
            by_kind.entry((*kind).to_owned()).or_insert_with(|| name.clone());
            by_name.entry(name).or_insert_with(|| Target { kind: (*kind).to_owned(), preset: vec![] });
        }
        for (alias, kind, preset) in aliases() {
            if by_kind.contains_key(kind) {
                by_name.entry(alias.to_owned()).or_insert(Target { kind: kind.to_owned(), preset });
            }
        }
        Table { by_kind, by_name }
    })
}

/// The name scripts call a type by (`noise.fbm` → `FractalNoise`); a type
/// of the document (`project:x`) by its name; `None` for a type unknown.
pub fn name_of(kind: &str) -> Option<String> {
    let base = kind.split('@').next().unwrap_or(kind);
    if let Some(n) = base.strip_prefix("project:") {
        return Some(n.to_owned());
    }
    table().by_kind.get(base).cloned()
}

/// What `name` stands for: a name, an alias, or a type written as it is
/// (`noise.fbm`, `massif:mountain`); case is forgiven when only one fits.
pub fn lookup(name: &str) -> Option<Target> {
    let t = table();
    if let Some(found) = t.by_name.get(name) {
        return Some(found.clone());
    }
    if is_component_type(name) || crate::registry::latest(name).is_some_and(|s| s.family != Family::Internal) {
        return Some(Target { kind: name.to_owned(), preset: vec![] });
    }
    let lower = name.to_ascii_lowercase();
    let mut fits = t.by_name.iter().filter(|(n, _)| n.to_ascii_lowercase() == lower);
    match (fits.next(), fits.next()) {
        (Some((_, target)), None) => Some(target.clone()),
        _ => None,
    }
}

/// The latest version of type `base` (a node type or a composite Massif
/// ships), the one a call without `@` makes; 1 for any other.
pub fn latest_version(base: &str) -> u16 {
    if base.starts_with("massif:") {
        return crate::components::latest(base).map_or(1, |(v, _)| v);
    }
    if is_component_type(base) {
        return 1;
    }
    crate::registry::latest(base).map_or(1, |s| s.version)
}

/// Every name a script can call, for suggestions.
pub fn all_names() -> impl Iterator<Item = &'static str> {
    table().by_name.keys().map(String::as_str)
}

/// The types new nodes take, each with its script name, in the library's
/// order.
pub fn catalogue() -> Vec<(String, &'static NodeSpec)> {
    current().map(|s| (name_of(s.id).unwrap_or_else(|| s.id.to_owned()), s)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every type of node and every shipped composite has a name of its
    /// own, which leads back to it.
    #[test]
    fn every_node_has_one_name() {
        let mut seen = std::collections::BTreeMap::new();
        for s in crate::format::public() {
            let n = name_of(s.id).unwrap_or_else(|| panic!("{} has no name", s.id));
            assert!(n.chars().next().is_some_and(|c| c.is_ascii_uppercase()) && n.chars().all(|c| c.is_ascii_alphanumeric()), "{n}");
            if let Some(other) = seen.insert(n.clone(), s.id) {
                assert_eq!(other, s.id, "{n} names two types");
            }
            assert_eq!(lookup(&n).unwrap().kind, s.id);
        }
        for (kind, _, _) in crate::components::all().iter().filter(|(_, v, _)| *v == 1) {
            let n = name_of(kind).unwrap();
            assert!(seen.insert(n.clone(), kind).is_none(), "{n} names two types");
            assert_eq!(lookup(&n).unwrap().kind, *kind);
        }
        assert_eq!(name_of("noise.fbm").as_deref(), Some("FractalNoise"));
        assert_eq!(name_of("massif:alpine-range").as_deref(), Some("AlpineRange"));
        assert_eq!(lookup("Multiply").unwrap().preset, vec![("mode", json!("multiply"))]);
        assert_eq!(lookup("fractalnoise").unwrap().kind, "noise.fbm");
        assert_eq!(lookup("noise.fbm").unwrap().kind, "noise.fbm");
        assert_eq!(snake("MountainGenerator"), "mountain_generator");
    }
}
