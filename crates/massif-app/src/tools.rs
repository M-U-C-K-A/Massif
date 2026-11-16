//! Editing tools and how a mouse stroke turns into world edits.

use crate::brushes::BrushSource;
use crate::doc::Document;
use crate::theme::Icon;
use massif_core::compose::block_top;
use massif_core::level::LevelKind;
use massif_core::ops::{self, Anchor, Flood, Surface};
use massif_core::registry::{BiomeId, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::{Brush, TilePos};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Tool {
    Raise,
    Smooth,
    Flatten,
    SetHeight,
    Roughen,
    PaintTerrain,
    PaintBiome,
    /// What grows on the ground: grass, flowers, snow… (a cover).
    Cover,
    /// Objects of a layer — trees, rocks — standing where it is painted.
    Objects,
    /// The biome the land calls for; the terrain the biome calls for.
    AutoBiome,
    AutoPaint,
    Eyedropper,
    Flood,
    Drain,
    Water,
    /// A river laid out from its source: the water finds its way down,
    /// guided by the points it is dragged through.
    #[serde(alias = "Spring")]
    River,
    StampSlab,
    EraseSlab,
    /// True 3D: add matter, dig, round off.
    Build,
    Dig,
    Smooth3d,
    /// Selections: the part of the map the other tools change.
    SelectRect,
    SelectEllipse,
    SelectLasso,
    /// A selection painted with the brush, as WorldPainter's.
    SelectBrush,
    Pan,
}

impl Tool {
    pub const GROUPS: [(&'static str, Icon, &'static [Tool]); 7] = [
        (
            "Sculpt terrain",
            Icon::Mountain,
            &[Tool::Raise, Tool::Smooth, Tool::Flatten, Tool::SetHeight, Tool::Roughen],
        ),
        (
            "Paint",
            Icon::Paintbrush,
            &[Tool::PaintTerrain, Tool::PaintBiome, Tool::AutoPaint, Tool::AutoBiome],
        ),
        ("Select", Icon::SquareDashed, &[Tool::SelectRect, Tool::SelectEllipse, Tool::SelectLasso, Tool::SelectBrush]),
        ("Decor", Icon::Shrub, &[Tool::Cover, Tool::Objects]),
        ("Water", Icon::Droplet, &[Tool::River, Tool::Flood, Tool::Drain, Tool::Water]),
        ("Levels", Icon::Layers, &[Tool::StampSlab, Tool::EraseSlab]),
        ("Navigate", Icon::Compass, &[Tool::Pan, Tool::Eyedropper]),
    ];

    pub const fn label(self) -> &'static str {
        match self {
            Tool::Raise => "Raise / Lower",
            Tool::Smooth => "Smooth",
            Tool::Flatten => "Flatten",
            Tool::SetHeight => "Set height",
            Tool::Roughen => "Roughen",
            Tool::PaintTerrain => "Paint terrain",
            Tool::PaintBiome => "Paint biome",
            Tool::Cover => "Ground cover",
            Tool::Objects => "Objects",
            Tool::AutoBiome => "Auto biome",
            Tool::AutoPaint => "Auto paint",
            Tool::Eyedropper => "Eyedropper",
            Tool::Flood => "Flood",
            Tool::Drain => "Drain",
            Tool::Water => "Water brush",
            Tool::River => "River",
            Tool::StampSlab => "Stamp slab",
            Tool::EraseSlab => "Erase slab",
            Tool::Build => "Sculpt",
            Tool::Dig => "Dig",
            Tool::Smooth3d => "Smooth",
            Tool::SelectRect => "Rectangle selection",
            Tool::SelectEllipse => "Ellipse selection",
            Tool::SelectLasso => "Lasso selection",
            Tool::SelectBrush => "Selection brush",
            Tool::Pan => "Pan",
        }
    }

    pub const fn icon(self) -> Icon {
        match self {
            Tool::Raise => Icon::ChevronsUp,
            Tool::Smooth => Icon::Waves,
            Tool::Flatten => Icon::Equal,
            Tool::SetHeight => Icon::Ruler,
            Tool::Roughen => Icon::Sparkles,
            Tool::PaintTerrain => Icon::Paintbrush,
            Tool::PaintBiome => Icon::Trees,
            Tool::Cover => Icon::Sprout,
            Tool::Objects => Icon::TreePine,
            Tool::AutoBiome => Icon::Earth,
            Tool::AutoPaint => Icon::WandSparkles,
            Tool::Eyedropper => Icon::Pipette,
            Tool::Flood => Icon::PaintBucket,
            Tool::Drain => Icon::DropletOff,
            Tool::Water => Icon::Droplets,
            Tool::River => Icon::Route,
            Tool::StampSlab => Icon::Cuboid,
            Tool::EraseSlab => Icon::Eraser,
            Tool::Build => Icon::Pickaxe,
            Tool::Dig => Icon::Shovel,
            Tool::Smooth3d => Icon::Blend,
            Tool::SelectRect => Icon::SquareDashed,
            Tool::SelectEllipse => Icon::CircleDashed,
            Tool::SelectLasso => Icon::Lasso,
            Tool::SelectBrush => Icon::Highlighter,
            Tool::Pan => Icon::Hand,
        }
    }

    /// Default shortcuts, WorldPainter's: Alt and the tool's letter.
    pub fn shortcuts(self) -> &'static [(egui::Modifiers, egui::Key)] {
        use egui::{Key, Modifiers};
        const ALT: Modifiers = Modifiers::ALT;
        match self {
            Tool::Raise => &[(ALT, Key::H)],
            Tool::Flatten => &[(ALT, Key::A)],
            Tool::Smooth => &[(ALT, Key::S)],
            Tool::PaintTerrain => &[(ALT, Key::P), (ALT, Key::R)],
            Tool::Eyedropper => &[(ALT, Key::Y)],
            Tool::Flood => &[(ALT, Key::F)],
            _ => &[],
        }
    }

    /// The first shortcut as text (⌥H).
    pub fn shortcut_text(self) -> Option<String> {
        let (mods, key) = self.shortcuts().first()?;
        let mut s = String::new();
        if mods.alt {
            s.push('⌥');
        }
        if mods.shift {
            s.push('⇧');
        }
        if mods.command {
            s.push('⌘');
        }
        s.push_str(key.name());
        Some(s)
    }

    pub const fn help(self) -> &'static str {
        match self {
            Tool::Raise => "Raise the ground. Right button: lower.",
            Tool::Smooth => "Smooth the ground.",
            Tool::Flatten => "Flatten to the height where the stroke starts.",
            Tool::SetHeight => "Set the ground to the target height.",
            Tool::Roughen => "Roughen the ground. Right button: smoother.",
            Tool::PaintTerrain => {
                "Paint a terrain, or several mixed by a noise. Right button: back to automatic."
            }
            Tool::PaintBiome => "Paint a biome. Right button: back to automatic.",
            Tool::Cover => "Paint what grows: grass, flowers, snow… Right button: remove it.",
            Tool::Objects => "Paint where objects stand: trees, rocks… Right button: remove them.",
            Tool::AutoBiome => {
                "Sets the biome the land calls for: beach, forest, peaks…"
            }
            Tool::AutoPaint => {
                "Paints what the land calls for: rock on cliffs, sand on shores…"
            }
            Tool::Eyedropper => "Pick the terrain, biome, height and water under the cursor.",
            Tool::Flood => "Fill a basin with water. Right button: drain.",
            Tool::Drain => "Drain a body of water. Right button: fill.",
            Tool::Water => "Set the water level. Right button: remove water.",
            Tool::River => {
                "Click a source: the water finds its way down. Drag it to guide it."
            }
            Tool::StampSlab => {
                "Add rock to the active level. Right button: erase."
            }
            Tool::EraseSlab => "Erase the active level. Right button: add.",
            Tool::Build => {
                "Left button: add matter. Right button: dig."
            }
            Tool::Dig => {
                "Dig in 3D: caves, tunnels. Right button: build."
            }
            Tool::Smooth3d => {
                "Round off in 3D."
            }
            Tool::SelectRect => "Drag a rectangle: the other tools then act only inside it.",
            Tool::SelectEllipse => "Drag an ellipse: the other tools then act only inside it.",
            Tool::SelectLasso => "Draw around an area: the other tools then act only inside it.",
            Tool::SelectBrush => "Paint the selection with the brush. Right button: take it out.",
            Tool::Pan => "Move the map (also: middle button, Space).",
        }
    }

    pub const fn uses_brush(self) -> bool {
        !matches!(self, Tool::Eyedropper | Tool::Flood | Tool::Drain | Tool::Pan | Tool::River | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso)
    }

    /// The tools that draw the selection.
    pub const fn is_select(self) -> bool {
        matches!(self, Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush)
    }

    /// Tools that apply once per click rather than continuously.
    pub const fn is_click(self) -> bool {
        matches!(self, Tool::Eyedropper | Tool::Flood | Tool::Drain)
    }

    pub const fn needs_slab_level(self) -> bool {
        matches!(self, Tool::StampSlab | Tool::EraseSlab)
    }

    /// Tools that shape the world in 3D (sculpted voxels).
    pub const fn is_sculpt(self) -> bool {
        matches!(self, Tool::Build | Tool::Dig | Tool::Smooth3d)
    }

    /// What a sculpting tool does, inverted or not.
    pub fn sculpt_mode(self, inverse: bool) -> Option<massif_core::voxel::SculptMode> {
        use massif_core::voxel::SculptMode;
        Some(match (self, inverse) {
            (Tool::Build, false) | (Tool::Dig, true) => SculptMode::Add,
            (Tool::Dig, false) | (Tool::Build, true) => SculptMode::Remove,
            (Tool::Smooth3d, _) => SculptMode::Smooth,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SurfaceChoice {
    Top,
    Bottom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AnchorMode {
    /// Centre the slab at a distance above (or below) the ground surface.
    AboveGround,
    /// Centre the slab at a fixed elevation.
    Absolute,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToolState {
    pub tool: Tool,
    pub brush: Brush,
    /// Where the brush comes from: its image is found again from this.
    pub brush_source: BrushSource,
    /// Where on the map brushes act (WorldPainter's brush options).
    pub filter: massif_core::filter::Filter,
    /// Blocks per second at the brush centre, full strength.
    pub raise_speed: f32,
    pub target_height: f32,
    /// Which way Flatten moves the land, and whether it paints the land it
    /// flattens with the terrain its new height calls for.
    pub flatten_mode: ops::FlattenMode,
    pub flatten_theme: bool,
    pub roughness: f32,
    pub terrain: u16,
    pub biome: u16,
    /// The cover painted, by its number.
    pub cover: u16,
    /// The object layer painted, by its number (0: none yet).
    pub object_layer: u16,
    pub water_level: i16,
    pub slab_thickness: f32,
    pub anchor: AnchorMode,
    pub slab_offset: f32,
    pub slab_height: f32,
    pub surface: SurfaceChoice,
    /// Tool to come back to after the eyedropper.
    pub previous: Tool,
    /// The biome of each kind of land, and the terrains of each biome.
    pub auto: massif_core::auto::AutoRules,
    /// Asked from a panel: open the rules of the automatic tools.
    #[serde(skip)]
    pub open_rules: bool,
    /// Asked from a panel: run an automatic tool over the whole map (biomes
    /// when `true`, paint otherwise).
    #[serde(skip)]
    pub run_auto: Option<bool>,
    /// What the last automatic pass over the whole map changed: whether
    /// it was of biomes, and how many columns.
    #[serde(skip)]
    pub auto_done: Option<(bool, usize)>,
    /// How rivers are dug, and the river being laid out.
    pub river: RiverSettings,
    #[serde(skip)]
    pub river_draft: Option<RiverDraft>,
    /// What the last river dug.
    #[serde(skip)]
    pub river_note: Option<String>,
    /// What the last import of objects read.
    #[serde(skip)]
    pub objects_note: Option<String>,
    /// Paint a pattern of terrains rather than one.
    pub pattern: PatternSettings,
    /// The tool of the 3D Sculpt editor.
    pub sculpt: SculptSettings,
    /// How hard the brush presses drawn over the map, under the pointer (its
    /// lines show either way).
    pub show_strength: bool,
    /// What a new shape does to the selection when no key says otherwise.
    pub select_mode: massif_core::selection::Combine,
    /// Blocks over which a new shape's edge fades, on either side.
    pub select_feather: i32,
    /// "Select by": what, and its limits.
    pub select_by: crate::select::SelectBy,
}

/// The tool of the 3D Sculpt editor, apart from the map's; its brush is
/// the brush of every tool, set in Brush Settings (see
/// [`ToolState::sculpt_brush`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SculptSettings {
    /// Sculpt (left adds, right digs), Smooth or Paint.
    pub tool: Tool,
    /// Walking in the view as a player would (for the session only).
    #[serde(skip)]
    pub player: bool,
    /// How fast the player flies, as a share of the game's speed (Axiom's
    /// flight speed).
    pub fly_speed: f32,
}

impl Default for SculptSettings {
    fn default() -> Self {
        Self {
            tool: Tool::Build,
            player: false,
            fly_speed: 1.0,
        }
    }
}

impl SculptSettings {
    /// The tools of the 3D Sculpt editor: one to add matter and (with the
    /// right button) dig, one to smooth, one to paint the terrain.
    pub const TOOLS: [Tool; 3] = [Tool::Build, Tool::Smooth3d, Tool::PaintTerrain];

    /// How many blocks a dab of the Sculpt tool pushes the surface by in a
    /// second, at full strength.
    pub const GROWTH: f32 = 16.0;

    /// The slowest and the fastest flight, as shares of the game's.
    pub const FLY_SPEEDS: (f32, f32) = (0.1, 8.0);
}

/// Several terrains mixed by a noise for the Paint tool: Massif's noise
/// painter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct PatternSettings {
    pub on: bool,
    pub pattern: massif_core::pattern::Pattern,
    /// Terrains by name, in the order the noise lays them out (neighbours
    /// in the list meet on the ground), each with its share of the ground.
    pub terrains: Vec<(String, f32)>,
    /// Patterns saved by name, to paint with again.
    pub saved: Vec<PatternPreset>,
    /// Asked from a panel: paint the pattern over the whole map.
    #[serde(skip)]
    pub paint_map: bool,
    /// What the last pass over the whole map changed, in columns.
    #[serde(skip)]
    pub done: Option<usize>,
}

impl Default for PatternSettings {
    fn default() -> Self {
        let meadow = PatternPreset::built_in().swap_remove(0);
        Self {
            on: false,
            pattern: meadow.pattern,
            terrains: meadow.terrains,
            saved: Vec::new(),
            paint_map: false,
            done: None,
        }
    }
}

impl PatternSettings {
    /// The pattern with its terrains, those of the registry.
    pub fn sampler(&self, reg: &massif_core::Registry) -> Option<massif_core::pattern::PatternSampler> {
        let terrains: Vec<(TerrainId, f32)> = self.terrains.iter().filter_map(|(name, w)| Some((reg.terrain_by_name(name)?, *w))).collect();
        (!terrains.is_empty()).then(|| self.pattern.sampler(&terrains))
    }

    /// Terrain `k`'s share of the ground, 0 to 1.
    pub fn share(&self, k: usize) -> f32 {
        let total: f32 = self.terrains.iter().map(|t| t.1.max(0.0)).sum();
        match self.terrains.get(k) {
            Some(t) if total > 0.0 => t.1.max(0.0) / total,
            Some(_) => 1.0 / self.terrains.len() as f32,
            None => 0.0,
        }
    }

    /// Give terrain `k` this share of the ground; the others share the rest
    /// as they shared theirs (evenly, if they had none).
    pub fn set_share(&mut self, k: usize, share: f32) {
        let n = self.terrains.len();
        if k >= n {
            return;
        }
        let share = if n == 1 { 1.0 } else { share.clamp(0.0, 1.0) };
        let others: f32 = (0..n).filter(|&j| j != k).map(|j| self.share(j)).sum();
        let shares: Vec<f32> = (0..n)
            .map(|j| match j {
                _ if j == k => share,
                _ if others > 1e-6 => self.share(j) / others * (1.0 - share),
                _ => (1.0 - share) / (n - 1) as f32,
            })
            .collect();
        for (t, s) in self.terrains.iter_mut().zip(shares) {
            t.1 = s;
        }
    }

    /// Add a terrain with an even share (`1 / n` of the ground), the others
    /// keeping theirs in proportion; a terrain already there is left as is.
    pub fn add(&mut self, name: &str) {
        if self.terrains.iter().any(|t| t.0.eq_ignore_ascii_case(name)) {
            return;
        }
        self.normalize();
        self.terrains.push((name.to_owned(), 0.0));
        let n = self.terrains.len();
        self.set_share(n - 1, 1.0 / n as f32);
    }

    /// Take terrain `k` out; the others share its ground.
    pub fn remove(&mut self, k: usize) {
        if k < self.terrains.len() {
            self.terrains.remove(k);
            self.normalize();
        }
    }

    /// Move terrain `from` to the place of `to`, the others making room.
    pub fn reorder(&mut self, from: usize, to: usize) {
        let n = self.terrains.len();
        if from < n && to < n && from != to {
            let t = self.terrains.remove(from);
            self.terrains.insert(to, t);
        }
    }

    /// Shares adding up to one.
    pub fn normalize(&mut self) {
        let shares: Vec<f32> = (0..self.terrains.len()).map(|k| self.share(k)).collect();
        for (t, s) in self.terrains.iter_mut().zip(shares) {
            t.1 = s;
        }
    }

    /// The preset, saved or built in, that this pattern is as it stands.
    pub fn preset_name(&self) -> Option<String> {
        let same = |p: &PatternPreset| {
            p.pattern == self.pattern
                && p.terrains.len() == self.terrains.len()
                && p.terrains.iter().zip(&self.terrains).all(|(a, b)| a.0 == b.0 && (a.1 - b.1).abs() < 1e-4)
        };
        self.saved.iter().chain(PatternPreset::built_in().iter()).find(|p| same(p)).map(|p| p.name.clone())
    }

    /// Paint with a preset from now on.
    pub fn load(&mut self, preset: &PatternPreset) {
        self.pattern = preset.pattern.clone();
        self.terrains = preset.terrains.clone();
    }

    /// Save the pattern as it stands under `name`, over a saved one of
    /// that name.
    pub fn save(&mut self, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        self.normalize();
        let preset = PatternPreset {
            name: name.to_owned(),
            pattern: self.pattern.clone(),
            terrains: self.terrains.clone(),
        };
        match self.saved.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(p) => *p = preset,
            None => self.saved.push(preset),
        }
    }
}

/// A pattern kept to paint with again: its noise and its terrains.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PatternPreset {
    pub name: String,
    pub pattern: massif_core::pattern::Pattern,
    pub terrains: Vec<(String, f32)>,
}

impl PatternPreset {
    /// Grounds to start from, made of the terrains every map has.
    pub fn built_in() -> Vec<PatternPreset> {
        use massif_core::noise::Basis;
        use massif_core::pattern::{Pattern, PatternKind};
        let preset = |name: &str, pattern: Pattern, terrains: &[(&str, f32)]| PatternPreset {
            name: name.to_owned(),
            pattern,
            terrains: terrains.iter().map(|(n, w)| ((*n).to_owned(), *w)).collect(),
        };
        let base = Pattern::default();
        vec![
            preset(
                "Meadow",
                Pattern { scale: 18.0, ..base.clone() },
                &[("Grass", 0.64), ("Moss", 0.08), ("Coarse Dirt", 0.16), ("Dirt", 0.12)],
            ),
            preset(
                "Forest floor",
                Pattern { scale: 9.0, octaves: 5, gain: 0.6, seed: 7, ..base.clone() },
                &[("Moss", 0.15), ("Grass", 0.3), ("Podzol", 0.35), ("Coarse Dirt", 0.2)],
            ),
            preset(
                "Rocky ground",
                Pattern { scale: 7.0, lacunarity: 2.5, softness: 0.4, seed: 3, ..base.clone() },
                &[("Andesite", 0.26), ("Stone", 0.3), ("Cobblestone", 0.1), ("Gravel", 0.14), ("Coarse Dirt", 0.2)],
            ),
            preset(
                "Gravel paths",
                Pattern { kind: PatternKind::Veins, scale: 48.0, octaves: 3, softness: 0.1, seed: 5, ..base.clone() },
                &[("Gravel", 0.08), ("Coarse Dirt", 0.06), ("Grass", 0.86)],
            ),
            preset(
                "Snowy rocks",
                Pattern { kind: PatternKind::Blobs, scale: 22.0, octaves: 5, gain: 0.55, seed: 11, ..base.clone() },
                &[("Stone", 0.2), ("Powder Snow", 0.15), ("Snow", 0.65)],
            ),
            preset(
                "Dunes",
                Pattern { kind: PatternKind::Stripes, noise: Basis::Perlin, scale: 10.0, octaves: 3, stretch: 3.0, angle: 20.0, softness: 0.2, seed: 2, ..base.clone() },
                &[("Sandstone", 0.2), ("Sand", 0.8)],
            ),
            preset(
                "Fields",
                Pattern { kind: PatternKind::Cells, scale: 40.0, stretch: 2.0, angle: 10.0, softness: 0.0, seed: 9, ..base },
                &[("Grass", 0.4), ("Moss", 0.2), ("Coarse Dirt", 0.2), ("Podzol", 0.2)],
            ),
        ]
    }
}

/// The settings of the River tool.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct RiverSettings {
    /// Width of the water at the source and at the mouth.
    pub width: f32,
    pub width_end: f32,
    pub depth: f32,
    pub banks: f32,
    pub water: bool,
    /// Terrain of the bed, by name ("": left as it is).
    pub bed: String,
    /// Make the water a river biome.
    pub biome: bool,
    /// From a spring: how much water — the river grows wider and deeper
    /// downstream (0..1).
    pub flow: f32,
    /// From a spring: how much the water wears the land beside its bed
    /// (0..1).
    pub erosion: f32,
    /// From a spring: which hollows it fills with a lake (0: none, it cuts
    /// through their rims; 1: all).
    pub lakes: f32,
}

impl Default for RiverSettings {
    fn default() -> Self {
        Self {
            width: 12.0,
            width_end: 20.0,
            depth: 4.0,
            banks: 12.0,
            water: true,
            bed: "Gravel".into(),
            biome: true,
            flow: 0.5,
            erosion: 0.3,
            lakes: 0.5,
        }
    }
}

impl RiverSettings {
    /// The least depth of a hollow a river from a spring fills with a lake.
    pub fn lake_depth(&self) -> f32 {
        if self.lakes <= 0.001 { f32::MAX } else { 1.0 + (1.0 - self.lakes.min(1.0)) * 24.0 }
    }

    /// How a river from a spring is dug: wider and deeper downstream with
    /// its flow, its banks wider with its erosion.
    pub fn spring_spec(&self, reg: &massif_core::registry::Registry) -> massif_core::river::RiverSpec {
        let flow = self.flow.clamp(0.0, 1.0);
        massif_core::river::RiverSpec {
            width: self.width,
            width_end: self.width * (1.0 + 4.0 * flow),
            depth: self.depth,
            depth_end: self.depth * (1.0 + 1.5 * flow),
            banks: self.banks + self.erosion.clamp(0.0, 1.0) * 4.0 * self.width,
            water: self.water,
            bed: if self.bed.is_empty() { None } else { reg.terrain_by_name(&self.bed) },
            biome: self.biome.then(|| reg.biome_by_id("minecraft:river")).flatten(),
        }
    }
}

/// How far water let go at a spring may run, and the widest hollow it fills.
pub const SPRING_REACH: usize = 60_000;
pub const SPRING_LAKE: usize = 2_000_000;

/// Where a river laid out with the River tool runs: from its source
/// through the points it is dragged through, to the end it was given — or
/// on down to where its water goes.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RiverLayout {
    pub source: (f32, f32),
    pub through: Vec<(f32, f32)>,
    pub end: Option<(f32, f32)>,
}

impl RiverLayout {
    /// The source, the points it passes, and the end if it has one.
    pub fn anchors(&self) -> Vec<(f32, f32)> {
        let mut a = Vec::with_capacity(self.through.len() + 2);
        a.push(self.source);
        a.extend(&self.through);
        a.extend(self.end);
        a
    }

    /// Move anchor `k` (in the order of [`Self::anchors`]).
    pub fn set(&mut self, k: usize, p: (f32, f32)) {
        match k {
            0 => self.source = p,
            k if k <= self.through.len() => self.through[k - 1] = p,
            _ => self.end = Some(p),
        }
    }
}

/// A leg of a course already worked out: its ends (to a quarter of a
/// block) and how finely.
type LegKey = ((i32, i32), (i32, i32), usize);
/// Where a course runs on by itself from, how deep a hollow must be to
/// hold a lake, how far it may go.
type TailKey = ((i32, i32), u32, usize);

/// A river being laid out: where it runs, and what that gives.
#[derive(Debug, Clone, Default)]
pub struct RiverDraft {
    pub layout: RiverLayout,
    /// Its course, the lakes on its way and where it ends.
    pub plan: massif_core::river::Descent,
    /// Where each anchor lies on the course (indices into it).
    pub stops: Vec<usize>,
    planned: Option<u64>,
    revision: u64,
    legs: std::collections::HashMap<LegKey, Vec<(f32, f32)>>,
    tail: Option<(TailKey, massif_core::river::Descent)>,
}

impl PartialEq for RiverDraft {
    fn eq(&self, other: &Self) -> bool {
        self.layout == other.layout
    }
}

/// Samples of the ground a leg of a river is routed over: finely for the
/// real thing, coarsely while its anchors are dragged.
const FINE_LEG: usize = 400_000;
const COARSE_LEG: usize = 40_000;

impl RiverDraft {
    pub fn new(source: (f32, f32)) -> Self {
        Self {
            layout: RiverLayout {
                source,
                ..RiverLayout::default()
            },
            ..Self::default()
        }
    }

    /// Work out its course, again only where its anchors moved. `moving`:
    /// the anchor being dragged, whose legs are worked out coarsely.
    pub fn plan(&mut self, doc: &Document, settings: &RiverSettings, moving: Option<usize>) {
        use std::hash::{Hash, Hasher};
        let lake_depth = settings.lake_depth();
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.layout).hash(&mut h);
        (doc.revision, lake_depth.to_bits(), moving).hash(&mut h);
        let key = h.finish();
        if self.planned == Some(key) {
            return;
        }
        if self.revision != doc.revision {
            self.legs.clear();
            self.tail = None;
            self.revision = doc.revision;
        }
        let level = doc.dimension().ground();
        let sea = doc.world.meta.sea_level as f32;
        let quarter = |p: (f32, f32)| ((p.0 * 4.0).round() as i32, (p.1 * 4.0).round() as i32);
        let anchors = self.layout.anchors();
        let mut course = vec![anchors[0]];
        let mut stops = vec![0];
        for (k, w) in anchors.windows(2).enumerate() {
            let cells = if moving.is_some_and(|m| m == k || m == k + 1) { COARSE_LEG } else { FINE_LEG };
            let leg = self
                .legs
                .entry((quarter(w[0]), quarter(w[1]), cells))
                .or_insert_with(|| massif_core::river::route(level, w[0], w[1], cells));
            course.extend(leg.iter().skip(1));
            stops.push(course.len() - 1);
        }
        let (lakes, mouth) = match self.layout.end {
            Some(end) => {
                let probe = probe(doc, end.0.floor() as i32, end.1.floor() as i32);
                let mouth = match probe {
                    None => massif_core::river::Mouth::Edge,
                    Some(p) if p.height < sea => massif_core::river::Mouth::Sea,
                    Some(p) if p.fluid != NO_FLUID && p.fluid as f32 > p.height => massif_core::river::Mouth::Water,
                    _ => massif_core::river::Mouth::Chosen,
                };
                (Vec::new(), Some(mouth))
            }
            None => {
                let last = *anchors.last().expect("a source");
                let reach = if moving == Some(anchors.len() - 1) { 6_000 } else { SPRING_REACH };
                let tk = ((last.0.floor() as i32, last.1.floor() as i32), lake_depth.to_bits(), reach);
                if self.tail.as_ref().is_none_or(|t| t.0 != tk) {
                    let lake_cap = if reach < SPRING_REACH { 150_000 } else { SPRING_LAKE };
                    let d = massif_core::river::descend(level, tk.0, sea, reach, lake_cap, lake_depth);
                    self.tail = Some((tk, d));
                }
                let tail = &self.tail.as_ref().expect("worked out").1;
                course.extend(tail.course.iter().skip(1));
                (tail.lakes.clone(), tail.mouth)
            }
        };
        self.plan = massif_core::river::Descent { course, lakes, mouth };
        self.stops = stops;
        self.planned = Some(key);
    }

    /// Its length in blocks, and where it ends, in words.
    pub fn summary(&self) -> String {
        let length: f32 = self.plan.course.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
        let lakes = match self.plan.lakes.len() {
            0 => String::new(),
            1 => " · 1 lake".to_owned(),
            n => format!(" · {n} lakes"),
        };
        let into = self.plan.mouth.map_or("", |m| m.label());
        format!("{length:.0} blocks to {into}{lakes}")
    }
}

/// Begin a river at block `(x, z)`: its source. Refused in water.
pub fn start_river(doc: &Document, tools: &mut ToolState, x: f32, z: f32) -> Feedback {
    let sea = doc.world.meta.sea_level as f32;
    match probe(doc, x.floor() as i32, z.floor() as i32) {
        None => return Some("That is off the map: click on land".into()),
        Some(p) if p.height < sea || (p.fluid != NO_FLUID && p.fluid as f32 > p.height) => {
            return Some("Already water: click on dry land, where it springs".into());
        }
        _ => {}
    }
    let mut draft = RiverDraft::new((x, z));
    draft.plan(doc, &tools.river, None);
    tools.river_draft = Some(draft);
    None
}

/// Dig the river being laid out (one undoable step).
pub fn dig_draft(doc: &mut Document, tools: &mut ToolState) -> Feedback {
    let mut draft = tools.river_draft.take()?;
    draft.plan(doc, &tools.river, None);
    if draft.plan.course.len() < 3 {
        let note = "Too short to dig: drag its end further".to_owned();
        tools.river_note = Some(note.clone());
        return Some(note);
    }
    let spec = tools.river.spring_spec(&doc.world.registry);
    doc.history.begin("River", doc.dim);
    let report = massif_core::river::dig_descent(&mut doc.history.edit(&mut doc.world, 0), &draft.plan, &spec);
    doc.commit();
    let note = format!("Dug a river {} · from Y {:.0}", draft.summary(), report.source);
    tools.river_note = Some(note.clone());
    Some(note)
}

impl Default for ToolState {
    fn default() -> Self {
        Self {
            tool: Tool::Raise,
            brush: Brush::default(),
            brush_source: BrushSource::default(),
            filter: massif_core::filter::Filter::default(),
            raise_speed: 24.0,
            target_height: 64.0,
            flatten_mode: ops::FlattenMode::Both,
            flatten_theme: false,
            roughness: 4.0,
            terrain: 1,
            biome: 0,
            cover: 1,
            object_layer: 0,
            water_level: 63,
            slab_thickness: 16.0,
            anchor: AnchorMode::AboveGround,
            slab_offset: 40.0,
            slab_height: 140.0,
            surface: SurfaceChoice::Top,
            previous: Tool::PaintTerrain,
            auto: massif_core::auto::AutoRules::default(),
            open_rules: false,
            run_auto: None,
            auto_done: None,
            river: RiverSettings::default(),
            river_draft: None,
            sculpt: SculptSettings::default(),
            river_note: None,
            objects_note: None,
            pattern: PatternSettings::default(),
            show_strength: true,
            select_mode: Default::default(),
            select_feather: 0,
            select_by: Default::default(),
        }
    }
}

impl ToolState {
    pub fn select(&mut self, tool: Tool) {
        if tool == Tool::Eyedropper && self.tool != Tool::Eyedropper {
            self.previous = self.tool;
        }
        if tool != self.tool {
            crate::sounds::play(crate::sounds::Cue::Select);
        }
        self.tool = tool;
    }

    /// The brush of the 3D Sculpt editor: the brush chosen for every tool
    /// (a custom one too, turned as it is), at the editor's own size and
    /// strength.
    pub fn sculpt_brush(&self) -> Brush {
        Brush {
            filter: None,
            radius: self.brush.radius.min(massif_core::voxel::MAX_SCULPT_RADIUS),
            ..self.brush.clone()
        }
    }

    /// The brush strokes use: the brush with the filter, if one is on.
    pub fn stroke_brush(&self) -> Brush {
        Brush {
            filter: self.filter.is_active().then(|| std::sync::Arc::new(self.filter.clone())),
            ..self.brush.clone()
        }
    }

    pub fn surface(&self) -> Surface {
        match self.surface {
            SurfaceChoice::Top => Surface::Top,
            SurfaceChoice::Bottom => Surface::Bottom,
        }
    }
}

/// What the user is told after a tool did (or refused to do) something.
pub type Feedback = Option<String>;

/// A mouse stroke in progress on the map.
pub struct Stroke {
    last: (f32, f32),
    salt: u64,
    /// Level the stroke edits.
    level: usize,
    tool: Tool,
    /// The inverse of the tool (right button or Alt).
    inverse: bool,
    /// Drawn with the right button.
    secondary: bool,
    /// Height to flatten toward: where the stroke started.
    flatten_to: Option<f32>,
    /// Columns the automatic tools changed.
    changed: usize,
    /// What the automatic tools give the land, read once for the stroke.
    classes: Option<massif_core::auto::Classes>,
}

impl Stroke {
    pub fn secondary(&self) -> bool {
        self.secondary
    }

    /// What the stroke read of the land, taken back.
    pub fn take_classes(&mut self) -> Option<massif_core::auto::Classes> {
        self.classes.take()
    }

    /// Start from what a preview already read of the land (if it is for
    /// this tool).
    pub fn with_classes(mut self, classes: Option<massif_core::auto::Classes>) -> Self {
        let biomes = self.tool == Tool::AutoBiome;
        self.classes = classes.filter(|c| c.biomes() == biomes && matches!(self.tool, Tool::AutoBiome | Tool::AutoPaint));
        self
    }
}

/// Begin a stroke at block position `at`. Returns `None` when the tool
/// cannot act here (the reason is in the feedback).
pub fn begin_stroke(doc: &mut Document, tools: &ToolState, at: (f32, f32), inverse: bool, secondary: bool) -> (Option<Stroke>, Feedback) {
    let tool = tools.tool;
    if tool == Tool::Pan || tool.is_click() {
        return (None, None);
    }
    let level = match tool {
        Tool::Flood | Tool::Drain | Tool::Build | Tool::Dig | Tool::Smooth3d => 0,
        _ => doc.active_level,
    };
    if tool.needs_slab_level() && doc.active_kind() == LevelKind::Ground {
        return (
            None,
            Some("Stamps need a Solid or Carve level (Levels panel).".into()),
        );
    }
    let flatten_to = (tool == Tool::Flatten)
        .then(|| ops::height_at(doc.dimension(), level, at.0.floor() as i32, at.1.floor() as i32, tools.surface()))
        .flatten();
    doc.history.begin(tool.label(), doc.dim);
    let salt = doc.revision.wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995;
    let stroke = Stroke {
        last: at,
        salt,
        level,
        tool,
        inverse,
        secondary,
        flatten_to,
        changed: 0,
        classes: None,
    };
    (Some(stroke), None)
}

/// Continue a stroke to `to`. `dt` is the time since the previous call.
pub fn continue_stroke(doc: &mut Document, tools: &ToolState, stroke: &mut Stroke, to: (f32, f32), dt: f32) {
    let invert = stroke.inverse;
    // The automatic tools read the land with the rules of the settings.
    let reader = matches!(stroke.tool, Tool::AutoBiome | Tool::AutoPaint)
        .then(|| massif_core::auto::Reader::new(&tools.auto, &doc.world.registry, doc.world.meta.sea_level).with_base(doc.auto_base()));
    let pattern = (stroke.tool == Tool::PaintTerrain && tools.pattern.on && !invert)
        .then(|| tools.pattern.sampler(&doc.world.registry))
        .flatten();
    let theme = ((matches!(stroke.tool, Tool::PaintTerrain | Tool::AutoPaint) && invert) || (stroke.tool == Tool::Flatten && tools.flatten_theme)).then(|| {
        (
            massif_core::generate::Theme::new(&doc.world.registry),
            doc.world.meta.sea_level as f32,
        )
    });
    let brush = &tools.stroke_brush();
    let (dx, dz) = (to.0 - stroke.last.0, to.1 - stroke.last.1);
    let len = (dx * dx + dz * dz).sqrt();
    let spacing = (brush.radius * 0.25).max(1.0);
    let n = ((len / spacing).ceil() as usize).clamp(1, 512);
    let dt = dt.clamp(0.0, 0.1);
    let level = stroke.level.min(doc.dimension().levels.len() - 1);
    let surface = tools.surface();
    let mut edit = doc.history.edit(&mut doc.world, level);
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let (x, z) = (stroke.last.0 + dx * t, stroke.last.1 + dz * t);
        stroke.salt = stroke.salt.wrapping_add(0x632b_e59b);
        let share = 1.0 / n as f32;
        match stroke.tool {
            Tool::Raise => {
                let amount = tools.raise_speed * brush.strength * dt * share;
                ops::raise(&mut edit, brush, x, z, if invert { -amount } else { amount }, surface);
            }
            Tool::Smooth => {
                let b = Brush {
                    strength: (brush.strength * dt * 10.0 * share).min(1.0),
                    ..brush.clone()
                };
                ops::smooth(&mut edit, &b, x, z, surface);
            }
            Tool::Flatten => {
                let b = Brush {
                    strength: (brush.strength * dt * 8.0 * share).min(1.0),
                    ..brush.clone()
                };
                let target = stroke.flatten_to.unwrap_or(tools.target_height);
                // The theme: the terrain the new height calls for, as the
                // inverse of Paint Terrain puts back.
                let paint = theme.as_ref().map(|(theme, sea)| move |h: f32| theme.terrain(h - 1.0, 0.0, *sea, 0.0));
                ops::flatten_with(&mut edit, &b, x, z, target, surface, tools.flatten_mode, paint.as_ref().map(|p| p as &(dyn Fn(f32) -> TerrainId + Sync)));
            }
            Tool::SetHeight => ops::set_height(&mut edit, brush, x, z, tools.target_height, surface),
            Tool::Roughen => {
                let amp = tools.roughness * brush.strength * dt * 4.0 * share;
                ops::roughen(&mut edit, brush, x, z, if invert { -amp } else { amp }, stroke.salt >> 7, surface);
            }
            Tool::PaintTerrain => match (&theme, &pattern) {
                // The inverse puts back the terrain the height calls for.
                (Some((theme, sea)), _) => ops::paint_terrain_with(&mut edit, brush, x, z, stroke.salt, |h| {
                    theme.terrain(h - 1.0, 0.0, *sea, 0.0)
                }),
                (None, Some(pattern)) => ops::paint_pattern(&mut edit, brush, x, z, stroke.salt, pattern),
                (None, None) => ops::paint_terrain(&mut edit, brush, x, z, TerrainId(tools.terrain), stroke.salt),
            },
            Tool::PaintBiome => {
                let biome = if invert { 0 } else { tools.biome };
                ops::paint_biome(&mut edit, brush, x, z, BiomeId(biome), stroke.salt)
            }
            Tool::Cover => ops::paint_cover(&mut edit, brush, x, z, if invert { 0 } else { tools.cover }, stroke.salt),
            Tool::Objects => ops::paint_objects(&mut edit, brush, x, z, if invert { 0 } else { tools.object_layer }, stroke.salt),
            Tool::AutoBiome => match &reader {
                Some(reader) if !invert => {
                    let classes = stroke.classes.get_or_insert_with(|| massif_core::auto::Classes::new(true));
                    stroke.changed += ops::auto_biome(&mut edit, brush, x, z, stroke.salt, reader, classes)
                }
                _ => ops::paint_biome(&mut edit, brush, x, z, BiomeId(0), stroke.salt),
            },
            Tool::AutoPaint => match (&theme, &reader) {
                (Some((theme, sea)), _) => ops::paint_terrain_with(&mut edit, brush, x, z, stroke.salt, |h| {
                    theme.terrain(h - 1.0, 0.0, *sea, 0.0)
                }),
                (None, Some(reader)) => {
                    let classes = stroke.classes.get_or_insert_with(|| massif_core::auto::Classes::new(false));
                    stroke.changed += ops::auto_paint(&mut edit, brush, x, z, stroke.salt, reader, classes)
                }
                (None, None) => {}
            },
            Tool::Water => {
                let level = if invert { NO_FLUID } else { tools.water_level };
                ops::set_fluid(&mut edit, brush, x, z, level);
            }
            Tool::StampSlab | Tool::EraseSlab if (stroke.tool == Tool::StampSlab) != invert => {
                let anchor = match tools.anchor {
                    AnchorMode::AboveGround => Anchor::Ground(tools.slab_offset),
                    AnchorMode::Absolute => Anchor::Absolute(tools.slab_height),
                };
                ops::stamp_slab(&mut edit, brush, x, z, anchor, tools.slab_thickness);
            }
            Tool::StampSlab | Tool::EraseSlab => ops::erase_slab(&mut edit, brush, x, z),
            Tool::Build | Tool::Dig | Tool::Smooth3d => {
                // On the map, sculpt where the surface is.
                let mode = stroke.tool.sculpt_mode(invert).expect("a sculpting tool");
                let y = surface_at(edit.dim(), x, z);
                if let Some(y) = y {
                    let strength = sculpt_strength(brush, dt * share);
                    massif_core::voxel::sculpt(&mut edit, brush, [x, y, z], strength, mode);
                }
            }
            Tool::Eyedropper | Tool::Flood | Tool::Drain | Tool::Pan | Tool::River | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush => {}
        }
    }
    stroke.last = to;
}

/// End a stroke: one step to undo. The automatic tools say what they did.
pub fn end_stroke(doc: &mut Document, stroke: Stroke) -> Feedback {
    doc.commit();
    match stroke.tool {
        Tool::AutoPaint | Tool::AutoBiome if !stroke.inverse => Some(match (stroke.changed, stroke.tool) {
            (0, _) => "Nothing to change here: the land already follows the rules".to_owned(),
            (n, Tool::AutoPaint) => format!("Repainted {n} columns"),
            (n, _) => format!("Changed the biome of {n} columns"),
        }),
        _ => None,
    }
}

/// How far one sculpting dab goes, for a dab lasting `dt` seconds.
pub fn sculpt_strength(brush: &Brush, dt: f32) -> f32 {
    (brush.strength * dt * 6.0).clamp(0.0, 1.0)
}

/// Elevation of the topmost surface of column `(x, z)`, as composed.
fn surface_at(dim: &massif_core::Dimension, x: f32, z: f32) -> Option<f32> {
    let (bx, bz) = (x.floor() as i32, z.floor() as i32);
    let stack = massif_core::TileStack::new(dim, TilePos::containing(bx, bz), false)?;
    let mut spans = Vec::new();
    stack.spans(massif_core::coords::local_index(bx, bz), &mut spans);
    spans.last().map(|s| s.y1 as f32)
}

/// A stroke in the 3D Sculpt editor: dabs where the view points.
pub struct SculptStroke {
    what: Stroke3d,
    salt: u64,
}

#[derive(Clone, Copy)]
enum Stroke3d {
    Sculpt(massif_core::voxel::SculptMode),
    /// Paint the terrain; the inverse puts back what the height calls for.
    Paint { inverse: bool },
}

/// Begin a stroke in the 3D Sculpt editor. `None` when its tool does
/// nothing there.
pub fn begin_sculpt(doc: &mut Document, tools: &ToolState, inverse: bool) -> Option<SculptStroke> {
    let tool = tools.sculpt.tool;
    let what = match tool {
        Tool::PaintTerrain => Stroke3d::Paint { inverse },
        _ => Stroke3d::Sculpt(tool.sculpt_mode(inverse)?),
    };
    doc.history.begin(tool.label(), doc.dim);
    let salt = doc.revision.wrapping_mul(0x9e37_79b9) ^ 0x3d5c_ab71;
    Some(SculptStroke { what, salt })
}

/// One dab of a 3D stroke on the surface at `frame`, lasting `dt` seconds:
/// the brush laid along the slope there. Sculpting pushes the surface out
/// (or in) a little — [`SculptSettings::GROWTH`] blocks a second at full
/// strength —, smoothing melts its bumps, painting paints the terrain of
/// the columns under the brush.
pub fn sculpt_dab(doc: &mut Document, tools: &ToolState, stroke: &mut SculptStroke, frame: &massif_core::voxel::Frame, dt: f32) -> usize {
    use massif_core::voxel::{self, SculptMode};
    let brush = tools.sculpt_brush();
    let dt = dt.clamp(0.0, 0.1);
    stroke.salt = stroke.salt.wrapping_add(0x632b_e59b);
    match stroke.what {
        Stroke3d::Sculpt(SculptMode::Smooth) => {
            let mut edit = doc.history.edit(&mut doc.world, 0);
            voxel::sculpt_at(&mut edit, &brush, frame.center, Some(frame.normal), sculpt_strength(&brush, dt), SculptMode::Smooth)
        }
        Stroke3d::Sculpt(mode) => {
            let mut edit = doc.history.edit(&mut doc.world, 0);
            voxel::displace_at(&mut edit, &brush, frame.center, frame.normal, brush.strength * SculptSettings::GROWTH * dt, mode)
        }
        Stroke3d::Paint { inverse } => {
            let reg = &doc.world.registry;
            let pattern = (!inverse && tools.pattern.on).then(|| tools.pattern.sampler(reg)).flatten();
            let theme = inverse.then(|| (massif_core::generate::Theme::new(reg), doc.world.meta.sea_level as f32));
            let brush = Brush {
                filter: tools.filter.is_active().then(|| std::sync::Arc::new(tools.filter.clone())),
                ..brush
            };
            let level = doc.active_level.min(doc.dimension().levels.len() - 1);
            let (x, z) = (frame.center[0], frame.center[2]);
            let mut edit = doc.history.edit(&mut doc.world, level);
            match (&theme, &pattern) {
                (Some((theme, sea)), _) => ops::paint_terrain_with(&mut edit, &brush, x, z, stroke.salt, |h| theme.terrain(h - 1.0, 0.0, *sea, 0.0)),
                (None, Some(pattern)) => ops::paint_pattern(&mut edit, &brush, x, z, stroke.salt, pattern),
                (None, None) => ops::paint_terrain(&mut edit, &brush, x, z, TerrainId(tools.terrain), stroke.salt),
            }
            // The matter sculpting moved keeps its own terrain: it is
            // painted too (back to the column's with the inverse).
            let painted = TerrainId(tools.terrain);
            match (&theme, &pattern) {
                (Some(_), _) => voxel::paint_sculpted(&mut edit, &brush, x, z, |_, _| None),
                (None, Some(pattern)) => voxel::paint_sculpted(&mut edit, &brush, x, z, |px, pz| pattern.terrain(px, pz)),
                (None, None) => voxel::paint_sculpted(&mut edit, &brush, x, z, |_, _| Some(painted)),
            };
            1
        }
    }
}

pub fn end_sculpt(doc: &mut Document, _stroke: SculptStroke) {
    doc.commit();
}

/// What lies under the cursor, for the eyedropper and the info panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Probe {
    pub height: f32,
    pub terrain: u16,
    pub biome: u16,
    pub fluid: i16,
    /// The cover painted on the surface; 0: none.
    pub cover: u16,
    /// The object layer painted on the surface; 0: none.
    pub objects: u16,
}

pub fn probe(doc: &Document, x: i32, z: i32) -> Option<Probe> {
    let dim = doc.dimension();
    let stack = massif_core::TileStack::new(dim, TilePos::containing(x, z), false)?;
    let cell = massif_core::coords::local_index(x, z);
    let mut sampler = crate::map_render::ColumnSampler::default();
    let s = sampler.sample(&stack, cell);
    if s.is_empty() {
        return None;
    }
    Some(Probe {
        height: s.height,
        terrain: s.terrain,
        biome: s.biome,
        fluid: s.fluid,
        cover: s.cover,
        objects: s.objects,
    })
}

/// A single-click tool at block `(x, z)`. `inverse` is the right button.
pub fn click(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32, inverse: bool) -> Feedback {
    match tools.tool {
        Tool::Eyedropper => {
            let p = probe(doc, x, z)?;
            let reg = &doc.world.registry;
            let terrain_name = reg.terrain(TerrainId(p.terrain)).name.clone();
            tools.terrain = p.terrain;
            tools.target_height = p.height.round();
            if p.biome != 0 {
                tools.biome = p.biome;
            }
            if p.fluid != NO_FLUID {
                tools.water_level = p.fluid;
            }
            if p.cover != 0 {
                tools.cover = p.cover;
            }
            if p.objects != 0 {
                tools.object_layer = p.objects;
            }
            tools.tool = tools.previous;
            Some(format!("Picked {terrain_name} at height {}", block_top(p.height)))
        }
        Tool::Flood | Tool::Drain => {
            let flood = (tools.tool == Tool::Flood) != inverse;
            let mode = if flood { Flood::Fill(tools.water_level) } else { Flood::Drain };
            doc.history.begin(if flood { "Flood" } else { "Drain" }, doc.dim);
            let n = ops::flood_fill(&mut doc.history.edit(&mut doc.world, 0), x, z, mode, 64 << 20);
            doc.commit();
            Some(match (flood, n) {
                (_, 0) => "Nothing to fill or drain here".to_owned(),
                (true, n) => format!("Flooded {n} columns up to {}", tools.water_level),
                (false, n) => format!("Drained {n} columns"),
            })
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, Landform, generate};

    fn doc() -> Document {
        let world = generate(&GenParams {
            tiles_x: 2,
            tiles_z: 2,
            landform: Landform::Flat,
            ..GenParams::default()
        });
        Document::new(world, None, 1)
    }

    fn height(doc: &Document, x: i32, z: i32) -> f32 {
        probe(doc, x, z).unwrap().height
    }

    #[test]
    fn auto_paint_does_what_its_preview_shows() {
        let world = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Mountains,
            ..GenParams::default()
        });
        let mut d = Document::new(world, None, 1);
        let mut tools = ToolState {
            tool: Tool::AutoPaint,
            ..ToolState::default()
        };
        tools.brush.radius = 30.0;
        tools.brush.strength = 1.0;
        let reader = massif_core::auto::Reader::new(&tools.auto, &d.world.registry, d.world.meta.sea_level);
        // Where the rules have the most to change.
        let mut best = (0, (0.0, 0.0), Vec::new(), massif_core::coords::BlockRect::new(0, 0, 0, 0));
        for z in (-240..240).step_by(40) {
            for x in (-240..240).step_by(40) {
                let at = (x as f32, z as f32);
                let (rect, cols) = ops::auto_preview(d.dimension(), &tools.brush, at.0, at.1, &reader, false);
                let n = cols.iter().filter(|c| c.becomes().is_some()).count();
                if n > best.0 {
                    best = (n, at, cols, rect);
                }
            }
        }
        let (n, at, cols, rect) = best;
        assert!(n > 50, "somewhere to repaint: {n}");
        let (stroke, _) = begin_stroke(&mut d, &tools, at, false, false);
        let mut stroke = stroke.unwrap();
        continue_stroke(&mut d, &tools, &mut stroke, at, 0.05);
        let note = end_stroke(&mut d, stroke).unwrap();
        assert!(note.contains("Repainted"), "{note}");
        // At full intensity, every column the preview showed is painted.
        for (k, c) in cols.iter().enumerate() {
            if let Some(t) = c.becomes() {
                let (x, z) = (rect.x0 + k as i32 % rect.width(), rect.z0 + k as i32 / rect.width());
                assert_eq!(probe(&d, x, z).unwrap().terrain, t, "at {x}, {z}");
            }
        }
        // Painted again, nothing is left to change, and it says so.
        let (stroke, _) = begin_stroke(&mut d, &tools, at, false, false);
        let mut stroke = stroke.unwrap();
        continue_stroke(&mut d, &tools, &mut stroke, at, 0.05);
        assert!(end_stroke(&mut d, stroke).unwrap().starts_with("Nothing to change"));
    }

    #[test]
    fn a_raise_stroke_is_one_undo_step() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.brush.strength = 1.0;
        let (stroke, msg) = begin_stroke(&mut d, &tools, (0.0, 0.0), false, false);
        assert!(msg.is_none());
        let mut stroke = stroke.unwrap();
        for i in 0..10 {
            continue_stroke(&mut d, &tools, &mut stroke, (i as f32 * 3.0, 0.0), 0.05);
        }
        end_stroke(&mut d, stroke);
        assert!(height(&d, 10, 0) > 64.0);
        assert!(d.history.can_undo());
        d.undo();
        assert_eq!(height(&d, 10, 0), 64.0);
        assert!(!d.history.can_undo());
    }

    #[test]
    fn stamping_requires_a_slab_level() {
        let mut d = doc();
        let mut tools = ToolState {
            tool: Tool::StampSlab,
            ..ToolState::default()
        };
        let (stroke, msg) = begin_stroke(&mut d, &tools, (0.0, 0.0), false, false);
        assert!(stroke.is_none() && msg.is_some());
        d.add_level(LevelKind::Solid);
        tools.slab_offset = 30.0;
        tools.brush.profile = massif_core::Profile::Plateau;
        let (stroke, _) = begin_stroke(&mut d, &tools, (0.0, 0.0), false, false);
        let mut stroke = stroke.unwrap();
        continue_stroke(&mut d, &tools, &mut stroke, (0.0, 0.0), 0.016);
        end_stroke(&mut d, stroke);
        // The island's top is now the highest surface.
        assert_eq!(height(&d, 0, 0), 64.0 + 30.0 + 8.0);
    }

    #[test]
    fn eyedropper_and_flood() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Eyedropper);
        assert_eq!(tools.previous, Tool::Raise);
        tools.terrain = 99;
        let msg = click(&mut d, &mut tools, 3, 3, false).unwrap();
        assert!(msg.starts_with("Picked Grass"), "{msg}");
        assert_eq!(tools.tool, Tool::Raise);
        assert_eq!(tools.terrain, d.world.registry.terrain_by_name("Grass").unwrap().0);

        tools.select(Tool::Flood);
        tools.water_level = 70;
        let msg = click(&mut d, &mut tools, 0, 0, false).unwrap();
        assert!(msg.starts_with("Flooded 65536 columns"), "{msg}");
        assert_eq!(probe(&d, 100, 100).unwrap().fluid, 70);
        d.undo();
        assert_eq!(probe(&d, 100, 100).unwrap().fluid, NO_FLUID);
    }

    #[test]
    fn the_right_button_inverts_tools() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.brush.strength = 1.0;
        let (stroke, _) = begin_stroke(&mut d, &tools, (0.0, 0.0), true, true);
        let mut stroke = stroke.unwrap();
        assert!(stroke.secondary());
        continue_stroke(&mut d, &tools, &mut stroke, (0.0, 0.0), 0.1);
        end_stroke(&mut d, stroke);
        assert!(height(&d, 0, 0) < 64.0);

        tools.select(Tool::Flood);
        tools.water_level = 70;
        click(&mut d, &mut tools, 50, 50, false);
        assert_eq!(probe(&d, 60, 60).unwrap().fluid, 70);
        let msg = click(&mut d, &mut tools, 50, 50, true).unwrap();
        assert!(msg.starts_with("Drained"), "{msg}");
        assert_eq!(probe(&d, 60, 60).unwrap().fluid, NO_FLUID);
    }

    /// Flatten's options, from a stroke as on the map: both ways levels a
    /// hill and a pit, Only raise fills the pit and leaves the hill, Only
    /// lower the other way round; Apply theme paints what it flattened with
    /// the terrain the height calls for, and only that.
    #[test]
    fn flatten_one_way_or_both_and_its_theme() {
        use massif_core::ops::FlattenMode;
        for (mode, theme) in [(FlattenMode::Both, false), (FlattenMode::RaiseOnly, false), (FlattenMode::LowerOnly, false), (FlattenMode::RaiseOnly, true)] {
            let mut d = doc();
            let mut tools = ToolState::default();
            // Flat ground at 64, a hill at 90 around (40, 40), a pit at 40
            // around (20, 40), all under the brush; the hill sand.
            d.history.begin("shape", 0);
            let small = Brush { radius: 4.0, profile: massif_core::Profile::Constant, ..Brush::default() };
            ops::set_height(&mut d.history.edit(&mut d.world, 0), &small, 40.0, 40.0, 90.0, Surface::Top);
            ops::set_height(&mut d.history.edit(&mut d.world, 0), &small, 20.0, 40.0, 40.0, Surface::Top);
            let ground = probe(&d, 30, 30).unwrap().terrain;
            let want = massif_core::generate::Theme::new(&d.world.registry).terrain(64.0, 0.0, d.world.meta.sea_level as f32, 0.0).0;
            let sand = TerrainId((1..).find(|t| *t != ground && *t != want).unwrap());
            ops::paint_terrain(&mut d.history.edit(&mut d.world, 0), &Brush { strength: 1.0, ..small.clone() }, 40.0, 40.0, sand, 1);
            d.commit();
            let (flat, hill0, pit0) = (height(&d, 30, 30), height(&d, 40, 40), height(&d, 20, 40));
            assert!(pit0 < flat && flat < hill0);
            tools.select(Tool::Flatten);
            tools.flatten_mode = mode;
            tools.flatten_theme = theme;
            tools.brush.strength = 1.0;
            tools.brush.radius = 40.0;
            tools.brush.profile = massif_core::Profile::Constant;
            let (stroke, _) = begin_stroke(&mut d, &tools, (30.5, 30.5), false, false);
            let mut stroke = stroke.unwrap();
            for _ in 0..60 {
                continue_stroke(&mut d, &tools, &mut stroke, (30.5, 30.5), 0.1);
            }
            end_stroke(&mut d, stroke);
            let (hill, pit) = (height(&d, 40, 40), height(&d, 20, 40));
            let expect = match mode {
                FlattenMode::Both => (flat, flat),
                FlattenMode::RaiseOnly => (hill0, flat),
                FlattenMode::LowerOnly => (flat, pit0),
            };
            assert_eq!((hill, pit), expect, "{mode:?}");
            let terrain = |x, z| probe(&d, x, z).unwrap().terrain;
            if theme {
                assert_eq!(terrain(20, 40), want, "the pit, raised, painted");
                assert_eq!(terrain(30, 30), want, "the ground at the height, painted");
                assert_eq!(terrain(40, 40), sand.0, "the hill, left, keeps its sand");
            } else {
                assert_eq!((terrain(20, 40), terrain(40, 40)), (ground, sand.0), "{mode:?}: no paint without the theme");
            }
        }
    }

    #[test]
    fn flatten_goes_toward_the_height_where_the_stroke_starts() {
        let mut d = doc();
        let mut tools = ToolState::default();
        // A hill at (40, 40), flat ground at 64 elsewhere.
        d.history.begin("hill", 0);
        ops::set_height(
            &mut d.history.edit(&mut d.world, 0),
            &Brush { radius: 6.0, profile: massif_core::Profile::Constant, ..Brush::default() },
            40.0,
            40.0,
            90.0,
            Surface::Top,
        );
        d.commit();
        tools.select(Tool::Flatten);
        tools.brush.strength = 1.0;
        tools.brush.radius = 30.0;
        tools.target_height = 10.0; // ignored: the start point decides
        let (stroke, _) = begin_stroke(&mut d, &tools, (10.5, 10.5), false, false);
        let mut stroke = stroke.unwrap();
        for _ in 0..40 {
            continue_stroke(&mut d, &tools, &mut stroke, (30.0, 30.0), 0.1);
        }
        end_stroke(&mut d, stroke);
        let h = height(&d, 38, 38);
        assert!((64.0..80.0).contains(&h), "{h}");
    }

    #[test]
    fn sculpting_from_the_map_digs_and_undoes() {
        let mut d = doc();
        let mut tools = ToolState {
            tool: Tool::Dig,
            ..ToolState::default()
        };
        tools.brush.radius = 6.0;
        tools.brush.strength = 1.0;
        let (stroke, msg) = begin_stroke(&mut d, &tools, (20.0, 20.0), false, false);
        assert!(msg.is_none());
        let mut stroke = stroke.unwrap();
        for _ in 0..10 {
            continue_stroke(&mut d, &tools, &mut stroke, (20.0, 20.0), 0.1);
        }
        end_stroke(&mut d, stroke);
        assert!(height(&d, 20, 20) < 64.0, "a pit was dug");
        assert!(!d.dimension().voxels.is_empty());
        assert_eq!(d.history.undo_label(), Some("Dig"));
        d.undo();
        assert_eq!(height(&d, 20, 20), 64.0);
        assert!(d.dimension().voxels.is_empty());

        // In the 3D Sculpt editor, the right button builds with the Dig tool.
        tools.sculpt.tool = Tool::Dig;
        tools.brush.radius = 6.0;
        tools.brush.strength = 1.0;
        tools.brush = Brush::default();
        let mut s = begin_sculpt(&mut d, &tools, true).unwrap();
        let frame = massif_core::voxel::Frame {
            center: [5.5, 64.0, 5.5],
            normal: [0.0, 1.0, 0.0],
        };
        let n = sculpt_dab(&mut d, &tools, &mut s, &frame, 0.1);
        end_sculpt(&mut d, s);
        assert!(n > 0);
        assert!(height(&d, 5, 5) > 64.0);
    }

    #[test]
    fn shortcuts_follow_worldpainter() {
        use egui::{Key, Modifiers};
        assert_eq!(Tool::Raise.shortcut_text().as_deref(), Some("⌥H"));
        assert_eq!(Tool::Flatten.shortcuts(), &[(Modifiers::ALT, Key::A)]);
        assert!(Tool::Roughen.shortcuts().is_empty());
        let mut all = Vec::new();
        for (_, _, list) in Tool::GROUPS {
            for tool in list {
                all.extend(tool.shortcuts().iter().map(|(m, k)| (m.alt, *k)));
            }
        }
        let n = all.len();
        all.sort_by_key(|(_, k)| k.name());
        all.dedup();
        assert_eq!(all.len(), n, "two tools share a shortcut");
    }
}
