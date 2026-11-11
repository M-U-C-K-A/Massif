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
    Eyedropper,
    Flood,
    Drain,
    Water,
    StampSlab,
    EraseSlab,
    /// True 3D: add matter, dig, round off.
    Build,
    Dig,
    Smooth3d,
    Pan,
}

impl Tool {
    pub const GROUPS: [(&'static str, Icon, &'static [Tool]); 6] = [
        (
            "Sculpt terrain",
            Icon::Mountain,
            &[Tool::Raise, Tool::Smooth, Tool::Flatten, Tool::SetHeight, Tool::Roughen],
        ),
        (
            "Paint",
            Icon::Paintbrush,
            &[Tool::PaintTerrain, Tool::PaintBiome, Tool::Eyedropper],
        ),
        ("Water", Icon::Droplet, &[Tool::Flood, Tool::Drain, Tool::Water]),
        ("Levels", Icon::Layers, &[Tool::StampSlab, Tool::EraseSlab]),
        ("Sculpt 3D", Icon::Box, &[Tool::Build, Tool::Dig, Tool::Smooth3d]),
        ("Navigate", Icon::Compass, &[Tool::Pan]),
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
            Tool::Eyedropper => "Eyedropper",
            Tool::Flood => "Flood",
            Tool::Drain => "Drain",
            Tool::Water => "Water brush",
            Tool::StampSlab => "Stamp slab",
            Tool::EraseSlab => "Erase slab",
            Tool::Build => "Build (3D)",
            Tool::Dig => "Dig (3D)",
            Tool::Smooth3d => "Smooth (3D)",
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
            Tool::Eyedropper => Icon::Pipette,
            Tool::Flood => Icon::PaintBucket,
            Tool::Drain => Icon::DropletOff,
            Tool::Water => Icon::Droplets,
            Tool::StampSlab => Icon::Cuboid,
            Tool::EraseSlab => Icon::Eraser,
            Tool::Build => Icon::Hammer,
            Tool::Dig => Icon::Shovel,
            Tool::Smooth3d => Icon::Blend,
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
            Tool::Raise => "Raise the surface under the brush; the right button (or Alt) lowers it.",
            Tool::Smooth => "Blend the surface toward its local average.",
            Tool::Flatten => "Flatten toward the height where the stroke starts, as WorldPainter does.",
            Tool::SetHeight => "Set the surface to exactly the target height (the eyedropper picks it).",
            Tool::Roughen => "Add noise to break up smooth areas; the right button subtracts it.",
            Tool::PaintTerrain => {
                "Paint the selected terrain; the right button puts back the terrain the height calls for."
            }
            Tool::PaintBiome => "Paint the selected biome; the right button resets columns to automatic.",
            Tool::Eyedropper => "Click to pick the terrain, biome, height and water level under the cursor.",
            Tool::Flood => "Fill the basin under the cursor with water up to the water level; the right button drains it.",
            Tool::Drain => "Remove the body of water under the cursor; the right button floods it.",
            Tool::Water => "Set the water level under the brush; the right button removes water.",
            Tool::StampSlab => {
                "Add a slab to the active Solid or Carve level (overhangs, arches, floating islands, caves); the right button erases."
            }
            Tool::EraseSlab => "Remove the active Solid or Carve level under the brush; the right button stamps.",
            Tool::Build => {
                "Add matter in a ball where the 3D view points (on the map: at the surface): overhangs past vertical, arches, spires. The right button digs."
            }
            Tool::Dig => {
                "Dig a ball out where the 3D view points (on the map: at the surface): caves of any shape, tunnels, cut-backs. The right button builds."
            }
            Tool::Smooth3d => {
                "Round off in 3D where the 3D view points: cliff edges become rounded, sharp hollows fill in; flat faces stay put."
            }
            Tool::Pan => "Drag to move the map. With any tool the middle button, or Space held, moves it too.",
        }
    }

    pub const fn uses_brush(self) -> bool {
        !matches!(self, Tool::Eyedropper | Tool::Flood | Tool::Drain | Tool::Pan)
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
    pub roughness: f32,
    pub terrain: u16,
    pub biome: u16,
    pub water_level: i16,
    pub slab_thickness: f32,
    pub anchor: AnchorMode,
    pub slab_offset: f32,
    pub slab_height: f32,
    pub surface: SurfaceChoice,
    /// Tool to come back to after the eyedropper.
    pub previous: Tool,
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
            roughness: 4.0,
            terrain: 1,
            biome: 0,
            water_level: 63,
            slab_thickness: 16.0,
            anchor: AnchorMode::AboveGround,
            slab_offset: 40.0,
            slab_height: 140.0,
            surface: SurfaceChoice::Top,
            previous: Tool::PaintTerrain,
        }
    }
}

impl ToolState {
    pub fn select(&mut self, tool: Tool) {
        if tool == Tool::Eyedropper && self.tool != Tool::Eyedropper {
            self.previous = self.tool;
        }
        self.tool = tool;
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
}

impl Stroke {
    pub fn secondary(&self) -> bool {
        self.secondary
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
            Some("Stamping needs a Solid or Carve level: add one in the Levels panel and select it.".into()),
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
    };
    (Some(stroke), None)
}

/// Continue a stroke to `to`. `dt` is the time since the previous call.
pub fn continue_stroke(doc: &mut Document, tools: &ToolState, stroke: &mut Stroke, to: (f32, f32), dt: f32) {
    let invert = stroke.inverse;
    let theme = (stroke.tool == Tool::PaintTerrain && invert).then(|| {
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
                ops::flatten(&mut edit, &b, x, z, stroke.flatten_to.unwrap_or(tools.target_height), surface);
            }
            Tool::SetHeight => ops::set_height(&mut edit, brush, x, z, tools.target_height, surface),
            Tool::Roughen => {
                let amp = tools.roughness * brush.strength * dt * 4.0 * share;
                ops::roughen(&mut edit, brush, x, z, if invert { -amp } else { amp }, stroke.salt >> 7, surface);
            }
            Tool::PaintTerrain => match &theme {
                // The inverse puts back the terrain the height calls for.
                Some((theme, sea)) => ops::paint_terrain_with(&mut edit, brush, x, z, stroke.salt, |h| {
                    theme.terrain(h - 1.0, 0.0, *sea, 0.0)
                }),
                None => ops::paint_terrain(&mut edit, brush, x, z, TerrainId(tools.terrain), stroke.salt),
            },
            Tool::PaintBiome => {
                let biome = if invert { 0 } else { tools.biome };
                ops::paint_biome(&mut edit, brush, x, z, BiomeId(biome), stroke.salt)
            }
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
            Tool::Eyedropper | Tool::Flood | Tool::Drain | Tool::Pan => {}
        }
    }
    stroke.last = to;
}

pub fn end_stroke(doc: &mut Document, _stroke: Stroke) {
    doc.commit();
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

/// A sculpting stroke in the 3D view: dabs where the view points.
pub struct SculptStroke {
    mode: massif_core::voxel::SculptMode,
}

/// Begin sculpting in the 3D view. `None` when the tool does not sculpt.
pub fn begin_sculpt(doc: &mut Document, tools: &ToolState, inverse: bool) -> Option<SculptStroke> {
    let mode = tools.tool.sculpt_mode(inverse)?;
    doc.history.begin(tools.tool.label(), doc.dim);
    Some(SculptStroke { mode })
}

/// One dab of a 3D stroke at `center`, lasting `dt` seconds.
pub fn sculpt_dab(doc: &mut Document, tools: &ToolState, stroke: &SculptStroke, center: [f32; 3], dt: f32) -> usize {
    let strength = sculpt_strength(&tools.brush, dt.clamp(0.0, 0.1));
    let mut edit = doc.history.edit(&mut doc.world, 0);
    massif_core::voxel::sculpt(&mut edit, &tools.brush, center, strength, stroke.mode)
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
        assert_eq!(d.history.undo_label(), Some("Dig (3D)"));
        d.undo();
        assert_eq!(height(&d, 20, 20), 64.0);
        assert!(d.dimension().voxels.is_empty());

        // In the 3D view, the right button builds with the Dig tool.
        let s = begin_sculpt(&mut d, &tools, true).unwrap();
        let n = sculpt_dab(&mut d, &tools, &s, [5.5, 70.5, 5.5], 0.1);
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
