//! What every type of node is: its inputs, outputs and settings, with words
//! for people and for models, and the kernel that computes it.
//!
//! The registry is the single description of the node set: the editor
//! builds its panels from it, the format checks graphs against it, the
//! schema and the catalogue a model reads are written from it.

use crate::kernel::{FootprintFn, GeometryFn, GlobalFn, GlobalGpuFn, GpuKernel, LocalFn, PointFn, StatsFn, VolumeFn};

/// Families of nodes, in the order the library shows them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Family {
    Input,
    Primitive,
    Shape,
    Adjust,
    Simulate,
    Climate,
    Mask,
    Color,
    Combine,
    /// Points, lines, footprints and what stands on them.
    Geometry,
    /// Fields over the volume: caves, arches, overhangs.
    Volume,
    Output,
    /// Nodes a compiled graph adds itself (conversions): never in a graph's
    /// JSON nor in the library.
    Internal,
}

impl Family {
    pub const ALL: [Family; 12] = [Family::Input, Family::Primitive, Family::Shape, Family::Adjust, Family::Simulate, Family::Climate, Family::Mask, Family::Color, Family::Combine, Family::Geometry, Family::Volume, Family::Output];

    pub const fn label(self) -> &'static str {
        match self {
            Family::Input => "Inputs",
            Family::Primitive => "Primitives",
            Family::Shape => "Shapes",
            Family::Adjust => "Adjust",
            Family::Simulate => "Simulate",
            Family::Climate => "Climate",
            Family::Mask => "Masks",
            Family::Color => "Colours",
            Family::Combine => "Combine",
            Family::Geometry => "Geometry",
            Family::Volume => "Volumes",
            Family::Output => "Outputs",
            Family::Internal => "Internal",
        }
    }
}

/// What a socket carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortKind {
    /// Heights of the ground, in blocks.
    Height,
    /// A value from 0 to 1 over the terrain: where something is, how much.
    Mask,
    /// A colour, in linear RGB.
    Color,
    /// A direction and its strength: wind, the way water flows.
    Vector,
    /// A terrain (the block material) at each sample, by its id.
    Terrain,
    /// A biome of the game at each sample, by its id.
    Biome,
    /// A painted layer of the world at each sample (ground cover, objects,
    /// pockets, caves, annotations): its number in its family, then how
    /// strongly (0 to 1).
    Layer,
    /// Where things stand: [`massif_core::geometry::Points`].
    Points,
    /// Lines through the terrain (roads, rivers, walls):
    /// [`massif_core::geometry::Splines`].
    Splines,
    /// The ground things take: [`massif_core::geometry::Footprints`].
    Footprints,
    /// What stands there (a pack's building, a structure, an object):
    /// [`massif_core::geometry::Instances`].
    Instances,
    /// A field over the volume, in bricks of 16³ blocks: negative inside
    /// matter, positive in the air, its distance to the surface where it
    /// can (a signed distance), which caves, arches and overhangs are made
    /// of. See [`crate::kernel::Brick`].
    Density,
}

/// What a value goes through to feed a socket of another type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Conversion {
    /// Taken as it is (heights and masks are both numbers).
    AsIs,
    /// A colour's brightness.
    Luminance,
    /// A number as a grey.
    Grey,
}

impl PortKind {
    pub const ALL: [PortKind; 12] = [
        PortKind::Height,
        PortKind::Mask,
        PortKind::Color,
        PortKind::Vector,
        PortKind::Terrain,
        PortKind::Biome,
        PortKind::Layer,
        PortKind::Points,
        PortKind::Splines,
        PortKind::Footprints,
        PortKind::Instances,
        PortKind::Density,
    ];

    /// The type a socket's id names (`height`, `points`…).
    pub fn from_id(id: &str) -> Option<PortKind> {
        PortKind::ALL.into_iter().find(|k| k.id() == id)
    }

    /// Samples over the terrain, made by tiles: what every kernel but the
    /// geometry and volume ones makes.
    pub const fn is_raster(self) -> bool {
        !self.is_geometry() && !self.is_volume()
    }

    /// Geometry, made and read whole ([`crate::kernel::Geo`]).
    pub const fn is_geometry(self) -> bool {
        matches!(self, PortKind::Points | PortKind::Splines | PortKind::Footprints | PortKind::Instances)
    }

    /// A field over the volume, made by bricks.
    pub const fn is_volume(self) -> bool {
        matches!(self, PortKind::Density)
    }

    pub const fn id(self) -> &'static str {
        match self {
            PortKind::Height => "height",
            PortKind::Mask => "mask",
            PortKind::Color => "color",
            PortKind::Vector => "vector",
            PortKind::Terrain => "terrain",
            PortKind::Biome => "biome",
            PortKind::Layer => "layer",
            PortKind::Points => "points",
            PortKind::Splines => "splines",
            PortKind::Footprints => "footprints",
            PortKind::Instances => "instances",
            PortKind::Density => "density",
        }
    }

    /// Values per sample, each kept as a plane of the tile.
    pub const fn planes(self) -> usize {
        match self {
            PortKind::Color => 3,
            PortKind::Vector | PortKind::Layer => 2,
            PortKind::Points | PortKind::Splines | PortKind::Footprints | PortKind::Instances | PortKind::Density => 0,
            _ => 1,
        }
    }

    /// Ids rather than amounts: never averaged, resampled to the nearest.
    pub const fn categorical(self) -> bool {
        matches!(self, PortKind::Terrain | PortKind::Biome | PortKind::Layer)
    }

    /// How a value of this type feeds a socket of type `to`; `None` when
    /// it cannot without a node saying how.
    pub const fn conversion(self, to: PortKind) -> Option<Conversion> {
        use PortKind::*;
        match (self, to) {
            (a, b) if a as u8 == b as u8 => Some(Conversion::AsIs),
            (Height | Mask, Height | Mask) => Some(Conversion::AsIs),
            (Color, Height | Mask) => Some(Conversion::Luminance),
            (Height | Mask, Color) => Some(Conversion::Grey),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct PortSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub kind: PortKind,
    /// An input that can be left unconnected.
    pub optional: bool,
    pub help: &'static str,
    /// What an optional input left empty stands for.
    pub default: &'static str,
}

impl PortSpec {
    pub const fn new(name: &'static str, label: &'static str, kind: PortKind, help: &'static str) -> Self {
        Self { name, label, kind, optional: false, help, default: "" }
    }

    pub const fn optional(mut self) -> Self {
        self.optional = true;
        self
    }

    /// Optional, standing for `default` when left empty.
    pub const fn or(mut self, default: &'static str) -> Self {
        self.optional = true;
        self.default = default;
        self
    }
}

/// The unit of a number setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    None,
    /// Blocks of the world (distances and heights).
    Blocks,
    /// A share of the terrain's side, from 0 to 1 (positions).
    Fraction,
    Degrees,
    /// A pure number (a factor, a share).
    Ratio,
    Count,
    Celsius,
    Percent,
}

impl Unit {
    pub const fn id(self) -> &'static str {
        match self {
            Unit::None => "",
            Unit::Blocks => "blocks",
            Unit::Fraction => "fraction",
            Unit::Degrees => "degrees",
            Unit::Ratio => "ratio",
            Unit::Count => "count",
            Unit::Celsius => "celsius",
            Unit::Percent => "percent",
        }
    }

    /// What follows a value in the interface.
    pub const fn suffix(self) -> &'static str {
        match self {
            Unit::Blocks => " b",
            Unit::Degrees => "°",
            Unit::Celsius => " °C",
            Unit::Percent => " %",
            _ => "",
        }
    }
}

/// One choice of a choice setting.
#[derive(Debug, Clone, Copy)]
pub struct Choice {
    pub value: &'static str,
    pub label: &'static str,
    pub help: &'static str,
}

#[derive(Debug, Clone, Copy)]
pub enum ParamKind {
    /// A number. `soft` is the range the slider shows; values up to `min`
    /// and `max` can be typed.
    Float { min: f64, max: f64, soft: [f64; 2], default: f64, unit: Unit },
    Int { min: i64, max: i64, default: i64 },
    Bool { default: bool },
    Choice { options: &'static [Choice], default: &'static str },
    /// A colour, written `#rrggbb` (sRGB).
    Color { default: &'static str },
    /// The node's seed; left out, it comes from the node's name, so two
    /// nodes of a type never repeat each other.
    Seed,
    /// Points placed on the terrain (peaks, craters…): positions as
    /// fractions of the terrain, each with the values `fields` names.
    /// Written `[{ "x": 0.3, "z": 0.6, "height": 900 }, …]`.
    Points { fields: &'static [Field], max: usize },
    /// A line drawn on the terrain through control points (a range's
    /// spine, a coast, a river), each with the values `fields` names;
    /// `closed`, an area. Written as points, in order.
    Curve { fields: &'static [Field], closed: bool },
    /// A curve from 0–1 to 0–1 through control points, smooth and never
    /// overshooting (a transfer curve, a profile). Written `[[0, 0], [1, 1]]`.
    Profile { default: &'static [[f64; 2]] },
    /// Colours along 0–1. Written `[{ "at": 0, "color": "#rrggbb" }, …]`.
    Gradient { default: &'static [(f64, &'static str)] },
    Text { default: &'static str },
    /// A file, by its path (relative to the world's folder, or absolute).
    File { extensions: &'static [&'static str] },
}

/// One value each point of a Points or Curve setting carries.
#[derive(Debug, Clone, Copy)]
pub struct Field {
    pub name: &'static str,
    pub label: &'static str,
    pub unit: Unit,
    pub default: f64,
    pub min: f64,
    pub max: f64,
    pub help: &'static str,
}

/// Where a setting shows in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Section {
    /// The settings that make the node what it is.
    Main,
    Shape,
    Detail,
    /// Shown on demand.
    Advanced,
}

#[derive(Debug, Clone, Copy)]
pub struct ParamSpec {
    pub name: &'static str,
    pub label: &'static str,
    pub help: &'static str,
    pub kind: ParamKind,
    pub section: Section,
    /// A number a field can drive (the ◇ beside it): connected through the
    /// input `@name`, it takes each sample's value there (on Point, Local
    /// and Global kernels; a local kernel gets the field at its own
    /// samples).
    pub drivable: bool,
}

impl ParamSpec {
    const fn of(name: &'static str, label: &'static str, kind: ParamKind, help: &'static str) -> Self {
        Self { name, label, help, kind, section: Section::Main, drivable: false }
    }

    pub const fn float(name: &'static str, label: &'static str, unit: Unit, default: f64, min: f64, max: f64, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Float { min, max, soft: [min, max], default, unit }, help)
    }

    pub const fn int(name: &'static str, label: &'static str, default: i64, min: i64, max: i64, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Int { min, max, default }, help)
    }

    pub const fn flag(name: &'static str, label: &'static str, default: bool, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Bool { default }, help)
    }

    pub const fn choice(name: &'static str, label: &'static str, options: &'static [Choice], default: &'static str, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Choice { options, default }, help)
    }

    pub const fn color(name: &'static str, label: &'static str, default: &'static str, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Color { default }, help)
    }

    pub const fn seed() -> Self {
        Self::of("seed", "Seed", ParamKind::Seed, "Another seed gives another variation of the same settings.").section(Section::Detail)
    }

    pub const fn points(name: &'static str, label: &'static str, fields: &'static [Field], max: usize, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Points { fields, max }, help)
    }

    pub const fn curve(name: &'static str, label: &'static str, fields: &'static [Field], closed: bool, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Curve { fields, closed }, help)
    }

    pub const fn profile(name: &'static str, label: &'static str, default: &'static [[f64; 2]], help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Profile { default }, help)
    }

    pub const fn gradient(name: &'static str, label: &'static str, default: &'static [(f64, &'static str)], help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Gradient { default }, help)
    }

    pub const fn text(name: &'static str, label: &'static str, default: &'static str, help: &'static str) -> Self {
        Self::of(name, label, ParamKind::Text { default }, help)
    }

    pub const fn file(name: &'static str, label: &'static str, extensions: &'static [&'static str], help: &'static str) -> Self {
        Self::of(name, label, ParamKind::File { extensions }, help)
    }

    /// A number a field can drive (through the input `@name`).
    pub const fn drivable(mut self) -> Self {
        self.drivable = true;
        self
    }

    /// With a slider range narrower than what can be typed.
    pub const fn soft(mut self, lo: f64, hi: f64) -> Self {
        if let ParamKind::Float { min, max, default, unit, .. } = self.kind {
            self.kind = ParamKind::Float { min, max, soft: [lo, hi], default, unit };
        }
        self
    }

    pub const fn section(mut self, section: Section) -> Self {
        self.section = section;
        self
    }
}

/// How a node computes, which decides how the engine schedules it.
#[derive(Clone, Copy)]
pub enum Kernel {
    /// Each output sample from the input samples at the same place: any
    /// tile from the same tiles of its inputs.
    Point { cpu: PointFn, gpu: Option<&'static GpuKernel> },
    /// Each output sample from the input samples around it (`footprint`
    /// says how far, and at what level of detail it reads them).
    Local { cpu: LocalFn, gpu: Option<&'static GpuKernel>, footprint: FootprintFn },
    /// A point kernel that needs figures over the whole of its inputs
    /// (their lowest and highest values).
    Stats { cpu: StatsFn },
    /// The whole terrain at once, at the simulation's resolution: erosion,
    /// water. Finer tiles take the input's tile with the change the node
    /// made (`carry[output] = Some(input)`), or its output resampled.
    Global { cpu: GlobalFn, gpu: Option<GlobalGpuFn>, carry: &'static [Option<u8>] },
    /// Geometry (points, splines, footprints, instances) made once over the
    /// whole terrain, its raster inputs read whole at the simulations'
    /// level, its geometry inputs whole; cached as one value.
    Geometry { cpu: GeometryFn },
    /// A field over the volume, one brick of 16³ blocks at a time from the
    /// same bricks of its volume inputs and the columns under them of its
    /// raster inputs (with `halo` samples around), by
    /// [`crate::engine::Engine::bricks`]; cached brick by brick.
    Volume { cpu: VolumeFn, halo: u32 },
    /// What the graph gives the map: never evaluated, its inputs are.
    Output,
}

/// What a node reads outside the graph, which its results (and so its
/// hash) must follow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reads {
    Nothing,
    /// The map as it is ([`crate::kernel::Env::map_revision`]).
    Map,
    /// The files its File settings name ([`crate::kernel::Env::file_stamp`]).
    Files,
}

pub struct NodeSpec {
    /// `family.name`, as graphs write it.
    pub id: &'static str,
    /// Which version of the type this is: a graph writes `id` for the first
    /// and `id@N` for the others. A released version's settings, defaults
    /// and results never change; changing them makes a new version.
    pub version: u16,
    pub label: &'static str,
    pub family: Family,
    /// One sentence: what the node makes.
    pub summary: &'static str,
    /// When to use it and how its settings play together.
    pub help: &'static str,
    /// Other words it is searched by.
    pub keywords: &'static [&'static str],
    pub inputs: &'static [PortSpec],
    /// A family of inputs named by the graph: `name` is their prefix
    /// (`mask.` takes `mask.snow`, `mask.moss`…).
    pub open_inputs: Option<PortSpec>,
    /// On a node whose inputs grow ([`NodeSpec::grows`]): the mask each of
    /// its open inputs can take, named alike (`in.2` takes `mask.2`).
    pub open_masks: Option<PortSpec>,
    pub outputs: &'static [PortSpec],
    pub params: &'static [ParamSpec],
    pub kernel: Kernel,
    pub reads: Reads,
}

impl NodeSpec {
    /// What a node type leaves out: `NodeSpec { id: …, ..NodeSpec::BASE }`.
    pub const BASE: NodeSpec = NodeSpec {
        id: "",
        version: 1,
        label: "",
        family: Family::Primitive,
        summary: "",
        help: "",
        keywords: &[],
        inputs: &[],
        open_inputs: None,
        open_masks: None,
        outputs: &[],
        params: &[],
        kernel: Kernel::Output,
        reads: Reads::Nothing,
    };
}

impl std::fmt::Debug for NodeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id)
    }
}

impl NodeSpec {
    /// The type as graphs write it: `id`, or `id@N` from version 2.
    pub fn type_name(&self) -> String {
        if self.version == 1 { self.id.to_owned() } else { format!("{}@{}", self.id, self.version) }
    }

    pub fn param(&self, name: &str) -> Option<(usize, &ParamSpec)> {
        self.params.iter().enumerate().find(|(_, p)| p.name == name)
    }

    pub fn input(&self, name: &str) -> Option<usize> {
        self.inputs.iter().position(|p| p.name == name)
    }

    /// Whether `name` is one of the open families of inputs.
    pub fn open_input(&self, name: &str) -> Option<&PortSpec> {
        self.open_families().find(|o| name.strip_prefix(o.name).is_some_and(crate::model::valid_id))
    }

    /// The open families of inputs: `in.<name>`, then their masks.
    pub fn open_families(&self) -> impl Iterator<Item = &PortSpec> {
        self.open_inputs.iter().chain(self.open_masks.iter())
    }

    /// A node whose inputs grow as they are connected (a Combine): its open
    /// inputs feed its kernel after the declared ones, in their order
    /// ([`open_order`]), each followed by its mask.
    pub fn grows(&self) -> bool {
        self.open_inputs.is_some() && !self.is_output()
    }

    pub fn output(&self, name: &str) -> Option<usize> {
        self.outputs.iter().position(|p| p.name == name)
    }

    pub fn is_output(&self) -> bool {
        matches!(self.kernel, Kernel::Output)
    }

    pub fn has_seed(&self) -> bool {
        self.params.iter().any(|p| matches!(p.kind, ParamKind::Seed))
    }
}

/// How the open inputs of a node that grows are ordered: by the number
/// after their prefix (`in.2` before `in.10`), then by name.
pub fn open_order(name: &str) -> (u64, &str) {
    let tail = name.rsplit('.').next().unwrap_or(name);
    (tail.parse().unwrap_or(u64::MAX), name)
}

/// Every type of node, in the library's order (with the internal ones,
/// whose family is [`Family::Internal`]).
pub fn all() -> &'static [&'static NodeSpec] {
    crate::nodes::all()
}

/// The type of node a graph names `id` (`family.name`, or `family.name@N`
/// for a version after the first).
pub fn spec(id: &str) -> Option<&'static NodeSpec> {
    let (base, version) = match id.split_once('@') {
        Some((base, v)) => (base, v.parse::<u16>().ok()?),
        None => (id, 1),
    };
    all().iter().copied().find(|s| s.id == base && s.version == version)
}

/// The latest version of type `id` (`family.name`), the one new nodes get.
pub fn latest(id: &str) -> Option<&'static NodeSpec> {
    all().iter().copied().filter(|s| s.id == id).max_by_key(|s| s.version)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sockets_convert_where_it_makes_sense() {
        use PortKind::*;
        assert_eq!(Height.conversion(Mask), Some(Conversion::AsIs));
        assert_eq!(Color.conversion(Mask), Some(Conversion::Luminance));
        assert_eq!(Mask.conversion(Color), Some(Conversion::Grey));
        assert_eq!(Terrain.conversion(Terrain), Some(Conversion::AsIs));
        assert_eq!(Mask.conversion(Terrain), None);
        assert_eq!(Biome.conversion(Height), None);
        assert_eq!(Vector.conversion(Height), None);
    }

    #[test]
    fn the_registry_holds_together() {
        let mut ids = std::collections::HashSet::new();
        for s in all() {
            assert!(ids.insert(s.type_name()), "{} twice", s.type_name());
            let (family, name) = s.id.split_once('.').expect("ids are family.name");
            assert!(!family.is_empty() && crate::model::valid_id(name), "{}", s.id);
            assert!(s.summary.ends_with('.') && !s.help.is_empty(), "{} needs words", s.id);
            for p in s.params {
                assert!(!p.help.is_empty(), "{}.{} needs words", s.id, p.name);
                match p.kind {
                    ParamKind::Float { min, max, soft, default, .. } => {
                        assert!(min <= soft[0] && soft[0] < soft[1] && soft[1] <= max && (min..=max).contains(&default), "{}.{}", s.id, p.name)
                    }
                    ParamKind::Int { min, max, default } => assert!((min..=max).contains(&default), "{}.{}", s.id, p.name),
                    ParamKind::Choice { options, default } => assert!(options.iter().any(|o| o.value == default), "{}.{}", s.id, p.name),
                    ParamKind::Color { default } => assert!(crate::params::parse_color(default).is_some(), "{}.{}", s.id, p.name),
                    ParamKind::Gradient { default } => assert!(!default.is_empty() && default.iter().all(|(at, c)| (0.0..=1.0).contains(at) && crate::params::parse_color(c).is_some()), "{}.{}", s.id, p.name),
                    ParamKind::Profile { default } => assert!(default.len() >= 2 && default.windows(2).all(|w| w[0][0] < w[1][0]), "{}.{}", s.id, p.name),
                    ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. } => {
                        assert!(fields.iter().all(|f| (f.min..=f.max).contains(&f.default) && !f.help.is_empty()), "{}.{}", s.id, p.name)
                    }
                    _ => {}
                }
            }
            for p in s.params.iter().filter(|p| p.drivable) {
                assert!(matches!(p.kind, ParamKind::Float { .. }) && matches!(s.kernel, Kernel::Point { .. } | Kernel::Local { .. } | Kernel::Global { .. }), "{}.{} cannot be driven", s.id, p.name);
            }
            let names: Vec<_> = s.params.iter().map(|p| p.name).chain(s.inputs.iter().map(|p| p.name)).collect();
            let unique: std::collections::HashSet<_> = names.iter().collect();
            assert_eq!(unique.len(), names.len(), "{}: a name twice", s.id);
            if let Kernel::Global { carry, .. } = s.kernel {
                assert_eq!(carry.len(), s.outputs.len(), "{}", s.id);
            }
            if s.is_output() {
                assert!(s.outputs.is_empty() && s.inputs.iter().all(|p| p.optional && !p.default.is_empty()), "{}", s.id);
            } else {
                assert!(!s.outputs.is_empty(), "{}", s.id);
            }
        }
    }
}
