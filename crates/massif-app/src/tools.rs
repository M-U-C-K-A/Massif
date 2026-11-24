//! Editing tools and how a mouse stroke turns into world edits.

use crate::brushes::BrushSource;
use crate::doc::Document;
use crate::theme::Icon;
use massif_core::compose::block_top;
use massif_core::level::LevelKind;
use massif_core::ops::{self, Anchor, Flood, Surface, TooMany};
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
    /// A road laid out through points: graded into the land, painted,
    /// bridged over water.
    Road,
    PaintTerrain,
    /// Terrains blended across a boundary, A → B → C, by a gradient.
    Gradient,
    PaintBiome,
    /// What grows on the ground: grass, flowers, snow… (a cover).
    Cover,
    /// Objects of a layer — trees, rocks — standing where it is painted.
    Objects,
    /// Marks in colour on the map, as WorldPainter's annotations: never
    /// exported.
    Annotate,
    /// The size of tree the objects painted there take: bushes, small,
    /// medium or large trees.
    TreeSize,
    /// Pockets of other blocks in the rock under the ground.
    Pockets,
    /// Smoothing: where Smooth terrain is on or off whatever the map's
    /// switch, or a slab chosen by hand (shown as "Smoothing").
    Slabs,
    /// A combined layer: several layers painted at once.
    Combined,
    /// The biome the land calls for; the terrain the biome calls for.
    AutoBiome,
    AutoPaint,
    Eyedropper,
    /// Fills a hollow with water, raises or lowers a body of water; drains
    /// it whole (Drain, a tool of its own before, is Flood's Shift + right
    /// click).
    #[serde(alias = "Drain")]
    Flood,
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

/// The names of the groups of tools the WorldPainter tour points at.
pub const SCULPT_GROUP: &str = tn!("Sculpt terrain");
pub const SELECT_GROUP: &str = tn!("Select");
pub const WATER_GROUP: &str = tn!("Water");

impl Tool {
    pub const GROUPS: [(&'static str, Icon, &'static [Tool]); 7] = [
        (
            SCULPT_GROUP,
            Icon::Mountain,
            &[Tool::Raise, Tool::Smooth, Tool::Flatten, Tool::SetHeight, Tool::Roughen, Tool::Road],
        ),
        (
            tn!("Paint"),
            Icon::Paintbrush,
            &[Tool::PaintTerrain, Tool::Gradient, Tool::PaintBiome, Tool::AutoPaint, Tool::AutoBiome, Tool::Annotate],
        ),
        (SELECT_GROUP, Icon::SquareDashed, &[Tool::SelectRect, Tool::SelectEllipse, Tool::SelectLasso, Tool::SelectBrush]),
        (tn!("Decor"), Icon::Shrub, &[Tool::Cover, Tool::Objects, Tool::TreeSize, Tool::Pockets, Tool::Slabs, Tool::Combined]),
        (WATER_GROUP, Icon::Droplet, &[Tool::Water, Tool::Flood, Tool::River]),
        (tn!("Levels"), Icon::Layers, &[Tool::StampSlab, Tool::EraseSlab]),
        (tn!("Navigate"), Icon::Compass, &[Tool::Pan, Tool::Eyedropper]),
    ];

    /// Tools new in this version: the Tools panel marks them until they
    /// are first tried.
    pub const NEW: [Tool; 2] = [Tool::Gradient, Tool::Slabs];

    pub fn label(self) -> &'static str {
        match self {
            Tool::Raise => tr!("Raise / Lower"),
            Tool::Smooth => tr!("Smooth"),
            Tool::Flatten => tr!("Flatten"),
            Tool::SetHeight => tr!("Set height"),
            Tool::Roughen => tr!("Roughen"),
            Tool::Road => tr!("Road"),
            Tool::PaintTerrain => tr!("Paint terrain"),
            Tool::Gradient => tr!("Gradient brush"),
            Tool::PaintBiome => tr!("Paint biome"),
            Tool::Cover => tr!("Ground cover"),
            Tool::Objects => tr!("Objects"),
            Tool::Annotate => tr!("Annotations"),
            Tool::TreeSize => tr!("Tree sizes"),
            Tool::Pockets => tr!("Underground pockets"),
            Tool::Slabs => tr!("Smoothing"),
            Tool::Combined => tr!("Combined layer"),
            Tool::AutoBiome => tr!("Auto biome"),
            Tool::AutoPaint => tr!("Auto paint"),
            Tool::Eyedropper => tr!("Eyedropper"),
            Tool::Flood => tr!("Flood"),
            Tool::Water => tr!("Water brush"),
            Tool::River => tr!("River"),
            Tool::StampSlab => tr!("Stamp slab"),
            Tool::EraseSlab => tr!("Erase slab"),
            Tool::Build => tr!("Sculpt"),
            Tool::Dig => tr!("Dig"),
            Tool::Smooth3d => tr!("Smooth"),
            Tool::SelectRect => tr!("Rectangle selection"),
            Tool::SelectEllipse => tr!("Ellipse selection"),
            Tool::SelectLasso => tr!("Lasso selection"),
            Tool::SelectBrush => tr!("Selection brush"),
            Tool::Pan => tr!("Pan"),
        }
    }

    pub const fn icon(self) -> Icon {
        match self {
            Tool::Raise => Icon::ChevronsUp,
            Tool::Smooth => Icon::Waves,
            Tool::Flatten => Icon::Equal,
            Tool::SetHeight => Icon::Ruler,
            Tool::Roughen => Icon::Sparkles,
            Tool::Road => Icon::Signpost,
            Tool::PaintTerrain => Icon::Paintbrush,
            Tool::Gradient => Icon::Blend,
            Tool::PaintBiome => Icon::Trees,
            Tool::Cover => Icon::Sprout,
            Tool::Objects => Icon::TreePine,
            Tool::Annotate => Icon::PenLine,
            Tool::TreeSize => Icon::ChartNoAxesColumnIncreasing,
            Tool::Pockets => Icon::Gem,
            Tool::Slabs => Icon::ChartNoAxesColumn,
            Tool::Combined => Icon::Combine,
            Tool::AutoBiome => Icon::Earth,
            Tool::AutoPaint => Icon::WandSparkles,
            Tool::Eyedropper => Icon::Pipette,
            Tool::Flood => Icon::PaintBucket,
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

    /// Its first shortcut as text (⌥H), as the keymap has it now.
    pub fn shortcut_text(self) -> Option<String> {
        crate::keymap::get().label(crate::keymap::Action::Tool(self))
    }

    /// What WorldPainter calls it (or what of WorldPainter it does), for
    /// those who come from it: its tools' names are WorldPainter's own, in
    /// English, as its interface shows them.
    pub fn worldpainter(self) -> Option<&'static str> {
        Some(match self {
            Tool::Raise => "Height, Raise Mountain, Raise Pyramid",
            Tool::Smooth => "Smooth",
            Tool::Flatten => "Flatten",
            Tool::Road => "Path",
            Tool::PaintTerrain => "Pencil, Spray Paint, Fill (a terrain)",
            Tool::PaintBiome => "Pencil, Spray Paint, Fill (a biome)",
            Tool::Cover | Tool::Objects | Tool::Combined => "Pencil, Spray Paint, Fill (a layer)",
            Tool::Annotate => "Annotations layer, Text",
            Tool::Pockets => "Underground Pockets layer",
            Tool::Flood => "Flood, Sponge",
            Tool::SelectRect => "Select",
            Tool::SelectBrush => "Select with Brush",
            _ => return None,
        })
    }

    pub fn help(self) -> &'static str {
        match self {
            Tool::Raise => tr!("Raise the ground. Right button: lower."),
            Tool::Smooth => tr!("Smooth the ground."),
            Tool::Flatten => tr!("Flatten to the height where the stroke starts."),
            Tool::SetHeight => tr!("Set the ground to the target height."),
            Tool::Roughen => tr!("Roughen the ground. Right button: smoother."),
            Tool::Road => tr!("Click points: a road runs through them. Enter builds it."),
            Tool::PaintTerrain => {
                tr!("Paint a terrain, or several mixed by a noise. Right button: back to the map's own terrain, or to the one the height calls for (below).")
            }
            Tool::Gradient => tr!("Paint over the edge between two terrains: they blend through the gradient's bands. Right button: the other way round."),
            Tool::PaintBiome => tr!("Paint a biome. Right button: back to automatic."),
            Tool::Cover => tr!("Paint what grows: grass, flowers, snow… Right button: remove it."),
            Tool::Objects => tr!("Paint where objects stand: trees, rocks… Right button: remove them."),
            Tool::Annotate => tr!("Mark the map in colour, to plan it: never exported. Right button: rub out."),
            Tool::TreeSize => tr!("Mark where bushes, small, medium or large trees grow: the object layers painted there take objects of that size. Right button: any size."),
            Tool::Pockets => tr!("Paint pockets of other blocks into the rock under the ground: gravel, clay, a mod's stone. Right button: rub them out."),
            Tool::Slabs => tr!("Paint where Smooth terrain builds the top of the land in partial blocks, or where it does not, whatever the map's setting. Right button: back to the map's setting."),
            Tool::Combined => tr!("Paint several layers at once: a terrain, a biome, a ground cover, trees, pockets, each over its share. Right button: rub out what it paints."),
            Tool::AutoBiome => {
                tr!("Sets the biome the land calls for: beach, forest, peaks…")
            }
            Tool::AutoPaint => {
                tr!("Paints what the land calls for: rock on cliffs, sand on shores…")
            }
            Tool::Eyedropper => tr!("Pick the terrain, biome, height and water under the cursor."),
            Tool::Flood => tr!("Fill a basin with water; on water, raise it a block. Right button: lower it a block, Shift: drain it. Hold to repeat."),
            Tool::Water => tr!("Lay water under the brush. Right button: take it off."),
            Tool::River => {
                tr!("Click a source: the water finds its way down. Drag it to guide it.")
            }
            Tool::StampSlab => {
                tr!("Add rock to the active level. Right button: erase.")
            }
            Tool::EraseSlab => tr!("Erase the active level. Right button: add."),
            Tool::Build => {
                tr!("Left button: add matter. Right button: dig.")
            }
            Tool::Dig => {
                tr!("Dig in 3D: caves, tunnels. Right button: build.")
            }
            Tool::Smooth3d => {
                tr!("Round off in 3D.")
            }
            Tool::SelectRect => tr!("Drag a rectangle: the other tools then act only inside it. A right-click or Esc clears it."),
            Tool::SelectEllipse => tr!("Drag an ellipse: the other tools then act only inside it. A right-click or Esc clears it."),
            Tool::SelectLasso => tr!("Draw around an area: the other tools then act only inside it. A right-click or Esc clears it."),
            Tool::SelectBrush => tr!("Paint the selection with the brush. Right button: take it out. Esc clears it."),
            Tool::Pan => tr!("Move the map (also: middle button, Space)."),
        }
    }

    pub const fn uses_brush(self) -> bool {
        !matches!(self, Tool::Eyedropper | Tool::Flood | Tool::Pan | Tool::River | Tool::Road | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso)
    }

    /// The tools that draw the selection.
    pub const fn is_select(self) -> bool {
        matches!(self, Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush)
    }

    /// Tools that apply once per click rather than continuously.
    pub const fn is_click(self) -> bool {
        matches!(self, Tool::Eyedropper | Tool::Flood)
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

    /// What the tool does in the 3D Sculpt editor, which works with the
    /// tool of the Tools panel.
    pub const fn in_3d(self) -> Use3d {
        match self {
            Tool::Raise | Tool::Build | Tool::Dig => Use3d::Sculpt,
            // Smooth smooths as on the map (its kernel, strength and
            // passes): Smooth3d, from its shortcut, melts sculpted matter.
            Tool::Smooth3d => Use3d::Smooth,
            Tool::PaintTerrain => Use3d::Paint,
            Tool::Eyedropper | Tool::Flood => Use3d::Click,
            Tool::Pan | Tool::Road | Tool::River | Tool::Annotate | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush => Use3d::MapOnly,
            Tool::Smooth
            | Tool::Flatten
            | Tool::SetHeight
            | Tool::Roughen
            | Tool::Gradient
            | Tool::PaintBiome
            | Tool::Cover
            | Tool::Objects
            | Tool::TreeSize
            | Tool::Pockets
            | Tool::Slabs
            | Tool::Combined
            | Tool::AutoBiome
            | Tool::AutoPaint
            | Tool::Water
            | Tool::StampSlab
            | Tool::EraseSlab => Use3d::Brush,
        }
    }
}

/// What a tool does in the 3D Sculpt editor ([`Tool::in_3d`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Use3d {
    /// Pushes the surface out along its slope, or in (Dig, and the
    /// inverse): matter added on any side, overhangs, caves.
    Sculpt,
    /// Melts the bumps of the surface, in 3D.
    Smooth,
    /// Paints the terrain of the columns under the brush, and of the matter
    /// sculpted there.
    Paint,
    /// The map's own brush, on the columns around where the view points.
    Brush,
    /// A click where the view points.
    Click,
    /// Only on the map, where its shapes, points and marks are drawn.
    MapOnly,
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

/// What Paint terrain's right button puts back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Unpaint {
    /// The map's own terrain ([`Document::map_terrain`]): what it was
    /// filled with. By height on a map whose terrains the height chose.
    #[default]
    MapTerrain,
    /// The terrain the height calls for, as a new map is painted: grass,
    /// sand by the water, rock and snow up high.
    ByHeight,
}

/// The terrain Paint terrain's right button puts back, `None`: the one the
/// height calls for.
pub fn unpaint_to(doc: &Document, tools: &ToolState) -> Option<TerrainId> {
    match tools.unpaint {
        Unpaint::MapTerrain => doc.map_terrain(),
        Unpaint::ByHeight => None,
    }
}

/// [`unpaint_to`] for a stroke beginning: a terrain read from the land is
/// kept with the map, so that the right button keeps putting it back once
/// the map is painted over.
fn unpaint_for_stroke(doc: &mut Document, tools: &ToolState) -> Option<TerrainId> {
    let back = unpaint_to(doc, tools)?;
    doc.world.meta.fill.get_or_insert(back);
    Some(back)
}

/// Where a water tool takes the water's height from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum WaterHeight {
    /// From where it is used: Flood, the hollow clicked up to where it
    /// spills; the Water brush, where each stroke starts.
    #[default]
    Start,
    /// The map's sea level.
    Sea,
    /// The level set ([`ToolState::water_level`]).
    Fixed,
}

impl WaterHeight {
    pub const ALL: [WaterHeight; 3] = [WaterHeight::Start, WaterHeight::Sea, WaterHeight::Fixed];

    /// The water's level it sets, `None` for Where I start / click (the
    /// land says).
    pub fn level(self, doc: &Document, tools: &ToolState) -> Option<i16> {
        match self {
            WaterHeight::Start => None,
            WaterHeight::Sea => Some(doc.world.meta.sea_level.clamp(i16::MIN as i32, i16::MAX as i32) as i16),
            WaterHeight::Fixed => Some(tools.water_level),
        }
    }
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
    /// What Paint terrain's right button puts back.
    pub unpaint: Unpaint,
    /// The new tools ([`Tool::NEW`]) tried once: no longer marked.
    pub tried: Vec<Tool>,
    pub roughness: f32,
    pub terrain: u16,
    pub biome: u16,
    /// The cover painted, by its number.
    pub cover: u16,
    /// The object layer painted, by its number (0: none yet).
    pub object_layer: u16,
    /// The colour annotations are painted in: 1 to 15, WorldPainter's, or
    /// one of the map's own (see [`massif_core::annotations`]).
    pub annotation: u16,
    /// The Annotations tool draws curves through points rather than
    /// painting with the brush.
    pub annotation_curve: bool,
    /// The annotations show, on the map and in 3D (the eye of the
    /// Annotations tool, the pen of the map's header, View ▸ Show
    /// Annotations).
    pub show_annotations: bool,
    /// How its curves are drawn, and the curve being drawn.
    pub curve: crate::curve_draft::CurveSettings,
    #[serde(skip)]
    pub curve_draft: Option<crate::curve_draft::CurveDraft>,
    /// The tree size painted, 1 (bushes) to 4 (large trees).
    pub tree_size: u16,
    /// The layer of underground pockets painted, by its number (0: none
    /// yet).
    pub pocket_layer: u16,
    /// What the Smoothing brush paints: Smooth terrain on here (`None`,
    /// with the shapes of each terrain), or a slab chosen by hand (its
    /// state), its stairs at the steps.
    pub slab_block: Option<String>,
    /// The Smoothing brush paints Smooth terrain off.
    pub smooth_off: bool,
    /// The combined layer painted, by its number (0: none yet).
    pub combined: u16,
    /// The tree sizes shown over the map even when another tool is in hand.
    pub show_sizes: bool,
    /// The water's level of the Fixed water height.
    pub water_level: i16,
    /// Where Flood takes the water's height from: the hollow the water
    /// runs into, up to where it spills; the sea's level; or `water_level`.
    pub flood_height: WaterHeight,
    /// Flood's water height was picked in Tool Settings. Until it is, it
    /// is Where I click, the one that puts water anywhere: a height read
    /// from an older version, which never said, is set back to it
    /// ([`ToolState::settle`]).
    pub flood_height_chosen: bool,
    /// Where the Water brush takes it from: where each stroke starts (the
    /// water's surface there, or a block over the ground); the sea's level;
    /// or `water_level`.
    pub brush_height: WaterHeight,
    /// From a version before the water heights: the water tools took their
    /// level from where they were used, or (false) `water_level`. Read once
    /// ([`ToolState::settle`]), never written.
    #[serde(rename = "water_from_click", alias = "flood_from_click", skip_serializing)]
    pub(crate) old_from_click: Option<bool>,
    /// Flood from the click, on a spot that is not in a hollow: ask first
    /// (the water would spread as far as the land lies under it), or flood.
    pub flood_warn: bool,
    /// Blocks a second a held Flood raises or lowers the water by.
    pub water_rate: f32,
    /// A flood beyond a hollow waiting for an answer.
    #[serde(skip)]
    pub flood_ask: Option<FloodAsk>,
    /// The water tools never lay water higher than `water_limit`.
    pub water_limit_on: bool,
    pub water_limit: i16,
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
    /// How roads are built, and the road being laid out.
    pub road: crate::road_draft::RoadSettings,
    #[serde(skip)]
    pub road_draft: Option<crate::road_draft::RoadDraft>,
    /// What the last road built.
    #[serde(skip)]
    pub road_note: Option<String>,
    /// What the last import of objects read.
    #[serde(skip)]
    pub objects_note: Option<String>,
    /// Paint a pattern of terrains rather than one.
    pub pattern: PatternSettings,
    /// The Gradient brush: its gradient, how it lays it out, the library.
    pub gradient: crate::gradient_ui::GradientSettings,
    /// The tool of the 3D Sculpt editor.
    pub sculpt: SculptSettings,
    /// How hard the brush presses drawn over the map, under the pointer (its
    /// lines show either way).
    pub show_strength: bool,
    /// The Pencil, as WorldPainter's: the brushes draw a hard line at full
    /// strength, block after block, unbroken however fast the mouse goes.
    pub pencil: bool,
    /// The Pencil's width in blocks (odd: 1, 3, 5…).
    pub pencil_width: u16,
    /// The order of the filters' list of what the map holds.
    pub filter_sort: FilterSort,
    /// What the filters were last set to be on, latest first.
    pub filter_recent: Vec<massif_core::filter::Match>,
    /// What the map holds, for the filters' list to show first; set by
    /// the window while the list is open.
    #[serde(skip)]
    pub filter_used: Option<std::sync::Arc<std::collections::HashSet<massif_core::filter::Match>>>,
    /// The filters' list is open: the window keeps `filter_used` up to date.
    #[serde(skip)]
    pub filter_list_open: bool,
    /// The eyedropper of a filter is in hand: a click on the map (2D or
    /// 3D) picks what the filter is on there, for Only on (`true`) or
    /// Except on (`false`), rather than the tool acting.
    #[serde(skip)]
    pub filter_pick: Option<bool>,
    /// Where it was clicked: the column, and the point of the screen to
    /// show what is there by.
    #[serde(skip)]
    pub filter_pick_at: Option<((i32, i32), egui::Pos2)>,
    /// Test Export's regions being picked on the map: a click or a drag
    /// takes whole region files rather than the tool acting.
    #[serde(skip)]
    pub region_pick: Option<RegionPick>,
    /// What a new shape does to the selection when no key says otherwise.
    pub select_mode: massif_core::selection::Combine,
    /// Blocks over which a new shape's edge fades, on either side.
    pub select_feather: i32,
    /// "Select by": what, and its limits.
    pub select_by: crate::select::SelectBy,
}

/// The 3D Sculpt editor's own settings. It works with the tool of the
/// Tools panel ([`Tool::in_3d`]) and the brush of every tool, set in Brush
/// Settings (see [`ToolState::sculpt_brush`]). (Its own Sculpt, Smooth and
/// Paint of before, saved as `tool`, are let go as settings are read.)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SculptSettings {
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
            player: false,
            fly_speed: 1.0,
        }
    }
}

impl SculptSettings {
    /// How many blocks a dab of the Build and Dig tools pushes the surface
    /// by in a second, at full strength (Raise / Lower goes at its speed).
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
        let mut preset = PatternPreset {
            name: name.to_owned(),
            pattern: self.pattern.clone(),
            terrains: self.terrains.clone(),
            palette: String::new(),
        };
        match self.saved.iter_mut().find(|p| p.name.eq_ignore_ascii_case(name)) {
            Some(p) => {
                // Saved over, it stays in its palette.
                preset.palette = std::mem::take(&mut p.palette);
                *p = preset
            }
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
    /// The palette of the Layers panel it is sorted into ("": none).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub palette: String,
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
            palette: String::new(),
        };
        let base = Pattern::default();
        vec![
            preset(
                tr!("Meadow"),
                Pattern { scale: 18.0, ..base.clone() },
                &[("Grass", 0.64), ("Moss", 0.08), ("Coarse Dirt", 0.16), ("Dirt", 0.12)],
            ),
            preset(
                tr!("Forest floor"),
                Pattern { scale: 9.0, octaves: 5, gain: 0.6, seed: 7, ..base.clone() },
                &[("Moss", 0.15), ("Grass", 0.3), ("Podzol", 0.35), ("Coarse Dirt", 0.2)],
            ),
            preset(
                tr!("Rocky ground"),
                Pattern { scale: 7.0, lacunarity: 2.5, softness: 0.4, seed: 3, ..base.clone() },
                &[("Andesite", 0.26), ("Stone", 0.3), ("Cobblestone", 0.1), ("Gravel", 0.14), ("Coarse Dirt", 0.2)],
            ),
            preset(
                tr!("Gravel paths"),
                Pattern { kind: PatternKind::Veins, scale: 48.0, octaves: 3, softness: 0.1, seed: 5, ..base.clone() },
                &[("Gravel", 0.08), ("Coarse Dirt", 0.06), ("Grass", 0.86)],
            ),
            preset(
                tr!("Snowy rocks"),
                Pattern { kind: PatternKind::Blobs, scale: 22.0, octaves: 5, gain: 0.55, seed: 11, ..base.clone() },
                &[("Stone", 0.2), ("Powder Snow", 0.15), ("Snow", 0.65)],
            ),
            preset(
                tr!("Dunes"),
                Pattern { kind: PatternKind::Stripes, noise: Basis::Perlin, scale: 10.0, octaves: 3, stretch: 3.0, angle: 20.0, softness: 0.2, seed: 2, ..base.clone() },
                &[("Sandstone", 0.2), ("Sand", 0.8)],
            ),
            preset(
                tr!("Fields"),
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
            // Sand in the shallows, wherever the bed is painted.
            shore: reg.terrain_by_name("Sand"),
            clay: reg.terrain_by_name("Clay"),
            sea: None,
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
        (doc.revision, lake_depth.to_bits(), moving, settings.width.to_bits(), settings.flow.to_bits()).hash(&mut h);
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
            let key = (quarter(w[0]), quarter(w[1]), cells);
            if !self.legs.contains_key(&key) {
                // Worked out finely where it was shown coarsely while
                // dragged: along the way shown, not down another valley.
                let guide = if cells == FINE_LEG { self.legs.get(&(key.0, key.1, COARSE_LEG)).cloned().unwrap_or_default() } else { Vec::new() };
                self.legs.insert(key, massif_core::river::route_near(level, w[0], w[1], cells, &guide));
            }
            course.extend(self.legs[&key].iter().skip(1));
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
        let sea = (mouth == Some(massif_core::river::Mouth::Sea)).then_some(sea);
        // Its loops cut, its runs along the grid smoothed, meanders on the
        // flat: what is shown is what is dug.
        let width = settings.width * (1.0 + 2.0 * settings.flow.clamp(0.0, 1.0));
        let (course, stops) = massif_core::river::natural_course(level, &course, width, &stops);
        self.plan = massif_core::river::Descent { course, lakes, mouth, sea };
        self.stops = stops;
        self.planned = Some(key);
    }

    /// Its length in blocks, and where it ends, in words.
    pub fn summary(&self) -> String {
        let length: f32 = self.plan.course.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).sum();
        let lakes = match self.plan.lakes.len() {
            0 => String::new(),
            n => trn!(" · {n} lake", " · {n} lakes", n = n),
        };
        let into = self.plan.mouth.map_or("", |m| crate::i18n::tr_name(m.label()));
        tr!("{length:.0} blocks to {into}{lakes}", length, into, lakes)
    }
}

/// Take point `k` (in the order of [`RiverLayout::anchors`]) away from
/// the river being laid out, the others staying where they are: the next
/// one becomes the source, the one before it the end. False when it was
/// the only point: no river is left.
pub fn remove_river_point(draft: &mut RiverDraft, k: usize) -> bool {
    let layout = &mut draft.layout;
    match k {
        0 if !layout.through.is_empty() => layout.source = layout.through.remove(0),
        0 => match layout.end.take() {
            Some(end) => layout.source = end,
            None => return false,
        },
        k if k <= layout.through.len() => {
            layout.through.remove(k - 1);
        }
        _ => layout.end = layout.through.pop(),
    }
    true
}

/// Begin a river at block `(x, z)`: its source. Refused in water.
pub fn start_river(doc: &Document, tools: &mut ToolState, x: f32, z: f32) -> Feedback {
    let sea = doc.world.meta.sea_level as f32;
    match probe(doc, x.floor() as i32, z.floor() as i32) {
        None => return Some(tr!("That is off the map: click on land").into()),
        Some(p) if p.height < sea || (p.fluid != NO_FLUID && p.fluid as f32 > p.height) => {
            return Some(tr!("Already water: click on dry land, where it springs").into());
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
        let note = tr!("Too short to dig: drag its end further").to_owned();
        tools.river_note = Some(note.clone());
        return Some(note);
    }
    let spec = tools.river.spring_spec(&doc.world.registry);
    doc.history.begin(tr!("River"), doc.dim);
    let report = massif_core::river::dig_descent(&mut doc.history.edit(&mut doc.world, 0), &draft.plan, &spec);
    doc.commit();
    let note = tr!("Dug a river {summary} · from Y {source:.0}", summary = draft.summary(), source = report.source);
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
            unpaint: Unpaint::MapTerrain,
            tried: Vec::new(),
            roughness: 4.0,
            terrain: 1,
            biome: 0,
            cover: 1,
            object_layer: 0,
            annotation: 14,
            annotation_curve: false,
            show_annotations: true,
            curve: Default::default(),
            curve_draft: None,
            tree_size: 2,
            pocket_layer: 0,
            slab_block: None,
            smooth_off: false,
            combined: 0,
            show_sizes: false,
            water_level: 63,
            flood_height: WaterHeight::Start,
            flood_height_chosen: false,
            brush_height: WaterHeight::Start,
            old_from_click: None,
            flood_warn: true,
            water_rate: 6.0,
            flood_ask: None,
            water_limit_on: false,
            water_limit: 63,
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
            road: Default::default(),
            road_draft: None,
            road_note: None,
            objects_note: None,
            pattern: PatternSettings::default(),
            gradient: Default::default(),
            show_strength: true,
            pencil: false,
            pencil_width: 1,
            filter_sort: FilterSort::Used,
            filter_recent: Vec::new(),
            filter_used: None,
            filter_list_open: false,
            filter_pick: None,
            filter_pick_at: None,
            region_pick: None,
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
        if Tool::NEW.contains(&tool) && !self.tried.contains(&tool) {
            self.tried.push(tool);
        }
        self.tool = tool;
    }

    /// Settings saved by an earlier version, made right for this one. The
    /// 3D Sculpt editor had a tool of its own (Sculpt, Smooth or Paint,
    /// saved as `sculpt.tool`): it now works with the tool of the Tools
    /// panel, so that one is let go; and the 3D tools that panel does not
    /// show are its Raise / Lower (which sculpts in 3D) and Smooth.
    pub fn settle(&mut self) {
        self.tool = match self.tool {
            Tool::Build | Tool::Dig => Tool::Raise,
            Tool::Smooth3d => Tool::Smooth,
            tool => tool,
        };
        if self.previous.is_sculpt() {
            self.previous = Tool::PaintTerrain;
        }
        // From the click off: both at the level that was set.
        if self.old_from_click.take() == Some(false) {
            (self.flood_height, self.brush_height) = (WaterHeight::Fixed, WaterHeight::Fixed);
        }
        // Flood's height never picked in Tool Settings: Where I click.
        if !self.flood_height_chosen {
            self.flood_height = WaterHeight::Start;
        }
    }

    /// The water height of the water tool `tool`.
    pub fn water_height(&self, tool: Tool) -> WaterHeight {
        if tool == Tool::Flood { self.flood_height } else { self.brush_height }
    }

    /// A new tool not tried yet: marked in the Tools panel.
    pub fn untried(&self, tool: Tool) -> bool {
        Tool::NEW.contains(&tool) && !self.tried.contains(&tool) && self.tool != tool
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

    /// The brush as the 3D Sculpt editor shows it: the sculpting brush for
    /// the tools that shape or paint in 3D, the map's for the others.
    pub fn brush_3d(&self) -> Brush {
        match self.tool.in_3d() {
            Use3d::Sculpt | Use3d::Smooth | Use3d::Paint => self.sculpt_brush(),
            _ => Brush {
                filter: None,
                ..self.brush.clone()
            },
        }
    }

    /// The brush strokes use: the brush with the filter, if one is on.
    pub fn stroke_brush(&self) -> Brush {
        let filter = self.filter.is_active().then(|| std::sync::Arc::new(self.filter.clone()));
        if self.pencil {
            return Brush { filter, ..self.pencil_brush() };
        }
        Brush { filter, ..self.brush.clone() }
    }

    /// The Pencil's brush: a square of [`ToolState::pencil_width`] blocks,
    /// hard and at full strength.
    pub fn pencil_brush(&self) -> Brush {
        let width = (self.pencil_width.max(1) | 1) as f32;
        Brush { shape: massif_core::brush::BrushShape::Square, profile: massif_core::brush::Profile::Constant, radius: width / 2.0, strength: 1.0, rotation: 0.0, mask: None, filter: None }
    }

    /// The brush the map shows and the strokes paint with: the Pencil's
    /// when it is on.
    pub fn shown_brush(&self) -> std::borrow::Cow<'_, Brush> {
        if self.pencil { std::borrow::Cow::Owned(self.pencil_brush()) } else { std::borrow::Cow::Borrowed(&self.brush) }
    }

    pub fn surface(&self) -> Surface {
        match self.surface {
            SurfaceChoice::Top => Surface::Top,
            SurfaceChoice::Bottom => Surface::Bottom,
        }
    }
}

/// Region files of the game being picked on the map for a Test Export.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RegionPick {
    /// Those picked, by their coordinates (`r.x.z.mca`).
    pub regions: std::collections::BTreeSet<(i32, i32)>,
    /// A drag under way: whether it takes (`true`) or leaves the regions it
    /// goes over, as the first one it pressed on was not or was picked.
    pub drag: Option<bool>,
}

impl RegionPick {
    /// The button pressed or held over block `(x, z)`: the region there
    /// changes as the press does, the first one deciding which way.
    pub fn press(&mut self, x: i32, z: i32) {
        let r = (x.div_euclid(crate::map_view::REGION_SIZE), z.div_euclid(crate::map_view::REGION_SIZE));
        let take = *self.drag.get_or_insert(!self.regions.contains(&r));
        if take {
            self.regions.insert(r);
        } else {
            self.regions.remove(&r);
        }
    }
}

/// The order of the filters' list of terrains, biomes and layers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum FilterSort {
    /// What the map holds first, then the rest, each by name.
    #[default]
    Used,
    /// What the filters were set to lately first.
    Recent,
    /// By name.
    Name,
}

impl ToolState {
    /// Put `m` first among what the filters were lately set to.
    pub fn filter_used_now(&mut self, m: massif_core::filter::Match) {
        self.filter_recent.retain(|r| *r != m);
        self.filter_recent.insert(0, m);
        self.filter_recent.truncate(24);
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
    /// The level the Water brush lays its water at.
    water_to: Option<i16>,
    /// Columns the automatic tools changed.
    changed: usize,
    /// What the automatic tools give the land, read once for the stroke.
    classes: Option<massif_core::auto::Classes>,
    /// The Gradient brush: which side of the boundary each column was on
    /// as the stroke began.
    sides: massif_core::gradient::Sides,
    /// Paint terrain's right button: the terrain it puts back (`None`: by
    /// height).
    back: Option<TerrainId>,
    /// The step to undo it records into, and its label.
    step: u64,
    label: String,
    /// With a selection on the map: whether a dab has reached it yet.
    reached: Option<bool>,
    /// What to say once of the stroke (it is all outside the selection).
    note: Option<String>,
    /// The Pencil drew its first block: the next part of the line starts
    /// past the block it ended on.
    drawn: bool,
}

impl Stroke {
    /// What the stroke has to say, once: that it is all outside the
    /// selection, where the tools do nothing.
    pub fn take_note(&mut self) -> Option<String> {
        self.note.take()
    }

    pub fn secondary(&self) -> bool {
        self.secondary
    }

    /// The level a stroke of the Water brush lays its water at.
    pub fn water_to(&self) -> Option<i16> {
        self.water_to
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

impl ToolState {
    /// `level` under the water tools' limit, if it is on.
    pub fn limit_water(&self, level: i16) -> i16 {
        if self.water_limit_on { level.min(self.water_limit) } else { level }
    }
}

/// What the water tools say when their limit keeps the water under the
/// spot used.
fn under_the_limit(tools: &ToolState) -> String {
    tr!("The height limit {water_limit} is under this spot: raise it or turn it off in Tool Settings.", water_limit = tools.water_limit)
}

/// What a stroke all outside the selection says: the tools act only inside
/// it.
pub const OUTSIDE_SELECTION: &str = tn!("A selection is on: the tools act only inside it. Press Esc with a Select tool, or Edit ▸ Deselect, to act everywhere again.");

/// The blocks from `from` to `to`, both in, each beside the last along one
/// of its sides: a line one block wide without a step across a corner, as
/// WorldPainter's Pencil draws it ("joined up").
pub fn pencil_path(from: (i32, i32), to: (i32, i32)) -> Vec<(i32, i32)> {
    let (dx, dz) = ((to.0 - from.0).abs() as i64, (to.1 - from.1).abs() as i64);
    let (sx, sz) = ((to.0 - from.0).signum(), (to.1 - from.1).signum());
    let mut out = Vec::with_capacity((dx + dz + 1) as usize);
    let (mut x, mut z) = from;
    out.push((x, z));
    let (mut ix, mut iz) = (0i64, 0i64);
    while ix < dx || iz < dz {
        // Step along the axis whose next block boundary the line crosses
        // first.
        if iz >= dz || (ix < dx && (1 + 2 * ix) * dz < (1 + 2 * iz) * dx) {
            x += sx;
            ix += 1;
        } else {
            z += sz;
            iz += 1;
        }
        out.push((x, z));
    }
    out
}

/// Whether the selection takes any column of `rect`.
pub fn touches_selection(sel: &massif_core::selection::Selection, rect: massif_core::coords::BlockRect) -> bool {
    let Some(bounds) = sel.bounds() else { return false };
    let r = rect.intersect(&bounds);
    r.width() > 0 && r.depth() > 0 && sel.weights(r).is_some_and(|w| w.iter().any(|&v| v > 0))
}

/// Begin a stroke at block position `at`. Returns `None` when the tool
/// cannot act here (the reason is in the feedback).
pub fn begin_stroke(doc: &mut Document, tools: &ToolState, at: (f32, f32), inverse: bool, secondary: bool) -> (Option<Stroke>, Feedback) {
    let tool = tools.tool;
    if tool == Tool::Pan || tool.is_click() {
        return (None, None);
    }
    let level = match tool {
        Tool::Flood | Tool::Build | Tool::Dig | Tool::Smooth3d => 0,
        _ => doc.active_level,
    };
    if tool.needs_slab_level() && doc.active_kind() == LevelKind::Ground {
        return (
            None,
            Some(tr!("Stamps need a Solid or Carve level (Levels panel).").into()),
        );
    }
    let flatten_to = (tool == Tool::Flatten)
        .then(|| ops::height_at(doc.dimension(), level, at.0.floor() as i32, at.1.floor() as i32, tools.surface()))
        .flatten();
    // The Water brush's level: from where the stroke starts (its water's
    // surface, or a block over its ground), the sea's or the one set; under
    // the limit.
    let water_to = if tool == Tool::Water && !inverse {
        let here = probe(doc, at.0.floor() as i32, at.1.floor() as i32);
        let level = tools.limit_water(brush_level(doc, tools, at.0.floor() as i32, at.1.floor() as i32));
        if tools.water_limit_on && here.is_some_and(|p| block_top(p.height) >= level as i32) {
            return (None, Some(under_the_limit(tools)));
        }
        Some(level)
    } else {
        None
    };
    // The right button puts back the terrain: Paint terrain's, and a
    // combined layer's that paints one.
    let terrain = tool == Tool::PaintTerrain || (tool == Tool::Combined && doc.world.registry.combined_layer(tools.combined).is_some_and(|c| c.terrain().is_some()));
    let back = (terrain && inverse).then(|| unpaint_for_stroke(doc, tools)).flatten();
    let label = crate::i18n::tr_name(tool.label()).to_owned();
    let step = doc.history.begin(label.clone(), doc.dim);
    let salt = doc.revision.wrapping_mul(0x9e37_79b9) ^ 0x5bd1_e995;
    let stroke = Stroke {
        last: at,
        salt,
        level,
        tool,
        inverse,
        secondary,
        flatten_to,
        water_to,
        changed: 0,
        classes: None,
        sides: gradient_sides(doc, tools, tool, inverse, level),
        back,
        step,
        label,
        reached: (doc.selection().is_some() && tool != Tool::SelectBrush).then_some(false),
        note: None,
        drawn: false,
    };
    if tool == Tool::Gradient {
        // The map keeps the gradients it is painted with.
        tools.gradient.keep_in(doc);
    }
    (Some(stroke), None)
}

/// The Gradient brush's sides for a stroke beginning: those its strokes
/// before learnt, when they were of the same gradient, the same way, on the
/// same level of the same map.
fn gradient_sides(doc: &mut Document, tools: &ToolState, tool: Tool, inverse: bool, level: usize) -> massif_core::gradient::Sides {
    use std::hash::{Hash, Hasher};
    if tool != Tool::Gradient {
        return massif_core::gradient::Sides::new();
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    serde_json::to_string(&tools.gradient.gradient).unwrap_or_default().hash(&mut h);
    (inverse, format!("{:?}", tools.gradient.layout), level, doc.dim, doc.generation).hash(&mut h);
    let mut sides = doc.gradient_sides.take().unwrap_or_default();
    sides.begin(h.finish());
    sides
}

/// Before a stroke edits the map: its step still the one open, or, when
/// something closed it under the stroke (an undo, a selection, a task's or
/// the terrain graph's result put in place), a new one for the rest of the
/// stroke, said in the log. Editing with no step open stopped the app.
fn keep_step(doc: &mut Document, step: &mut u64, label: &str) {
    if doc.history.open_step() != Some(*step) {
        crate::startup::note(&format!("the step of a stroke ({label}) was closed under it: the rest of the stroke is a step of its own"));
        *step = doc.history.begin(label.to_owned(), doc.dim);
    }
}

/// End a stroke's step, if it is still the one open (something else may
/// have closed it, and begun its own).
fn end_step(doc: &mut Document, step: u64) {
    if doc.history.open_step() == Some(step) {
        doc.commit();
    }
}

/// Continue a stroke to `to`. `dt` is the time since the previous call.
pub fn continue_stroke(doc: &mut Document, tools: &ToolState, stroke: &mut Stroke, to: (f32, f32), dt: f32) {
    keep_step(doc, &mut stroke.step, &stroke.label);
    let invert = stroke.inverse;
    // The automatic tools read the land with the rules of the settings.
    let reader = matches!(stroke.tool, Tool::AutoBiome | Tool::AutoPaint)
        .then(|| massif_core::auto::Reader::new(&tools.auto, &doc.world.registry, doc.world.meta.sea_level).with_base(doc.auto_base()));
    let pattern = (stroke.tool == Tool::PaintTerrain && tools.pattern.on && !invert)
        .then(|| tools.pattern.sampler(&doc.world.registry))
        .flatten();
    let gradient = (stroke.tool == Tool::Gradient)
        .then(|| {
            let g = if invert { tools.gradient.gradient.reversed() } else { tools.gradient.gradient.clone() };
            g.sampler(&doc.world.registry, doc.world.meta.seed)
        })
        .flatten();
    // A combined layer's parts, as their own tools paint them, or rubbed
    // out; the terrain it paints, if any. Its pockets are in the rock of
    // the ground: painted on the ground's level whatever level is edited.
    let combined = (stroke.tool == Tool::Combined).then(|| doc.world.registry.combined_layer(tools.combined)).flatten().map(|c| {
        let reg = &doc.world.registry;
        (if invert { c.to_rub_out(reg) } else { c.to_paint(reg) }, c.terrain().is_some())
    });
    let theme = ((matches!(stroke.tool, Tool::PaintTerrain | Tool::Combined) && invert && stroke.back.is_none()) || (stroke.tool == Tool::AutoPaint && invert) || (stroke.tool == Tool::Flatten && tools.flatten_theme)).then(|| {
        (
            massif_core::generate::Theme::new(&doc.world.registry),
            doc.world.meta.sea_level as f32,
        )
    });
    let brush = &tools.stroke_brush();
    let (dx, dz) = (to.0 - stroke.last.0, to.1 - stroke.last.1);
    let len = (dx * dx + dz * dz).sqrt();
    let spacing = (brush.radius * 0.25).max(1.0);
    // The Pencil goes block by block, each the next one along a side of
    // the last (never across a corner), at their centres: a line without
    // a gap.
    let path = tools.pencil.then(|| {
        let cell = |p: (f32, f32)| (p.0.floor() as i32, p.1.floor() as i32);
        let (from, to) = (cell(stroke.last), cell(to));
        let mut path = pencil_path(from, to);
        if stroke.drawn {
            path.remove(0);
        }
        path.into_iter().map(|(x, z)| (x as f32 + 0.5, z as f32 + 0.5)).collect::<Vec<_>>()
    });
    stroke.drawn = true;
    let n = match &path {
        Some(p) if p.is_empty() => {
            stroke.last = to;
            return;
        }
        Some(p) => p.len(),
        None => ((len / spacing).ceil() as usize).clamp(1, 512),
    };
    let dt = dt.clamp(0.0, 0.1);
    // Annotations are marks on the map, whatever the level: on the ground's.
    // Pockets are in the rock of the ground: on its level too.
    let level = if matches!(stroke.tool, Tool::Annotate | Tool::Pockets | Tool::Slabs) { 0 } else { stroke.level.min(doc.dimension().levels.len() - 1) };
    let understory = doc.world.registry.object_layer(tools.object_layer).is_some_and(|l| l.understory);
    let (parts, below): (Vec<massif_core::combined::Component>, Vec<massif_core::combined::Component>) = match &combined {
        Some((parts, _)) if level != 0 => parts.iter().partition(|c| c.part != massif_core::combined::Part::Pockets),
        Some((parts, _)) => (parts.clone(), Vec::new()),
        None => (Vec::new(), Vec::new()),
    };
    // Painted all outside the selection, the stroke does nothing: said once.
    if stroke.reached == Some(false)
        && let Some(sel) = doc.selection()
    {
        let r = brush.radius.ceil() as i32 + 1;
        let reached = (1..=n).any(|k| {
            let t = k as f32 / n as f32;
            let (x, z) = ((stroke.last.0 + dx * t).floor() as i32, (stroke.last.1 + dz * t).floor() as i32);
            touches_selection(sel, massif_core::coords::BlockRect::new(x - r, z - r, x + r + 1, z + r + 1))
        });
        if reached {
            stroke.reached = Some(true);
        } else {
            stroke.reached = None;
            stroke.note = Some(crate::i18n::tr_name(OUTSIDE_SELECTION).to_owned());
        }
    }
    let mut dabs = Vec::new();
    // Locked layers keep their blocks.
    doc.history.locks = massif_core::registry::Locks::of(&doc.world.registry);
    let surface = tools.surface();
    let slabs = if stroke.tool == Tool::Slabs { slab_value(tools, &mut doc.world.registry) } else { 0 };
    let mut edit = doc.history.edit(&mut doc.world, level);
    for k in 1..=n {
        let t = k as f32 / n as f32;
        let (x, z) = match &path {
            Some(p) => p[k - 1],
            None => (stroke.last.0 + dx * t, stroke.last.1 + dz * t),
        };
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
            Tool::PaintTerrain => match (stroke.back.filter(|_| invert), &theme, &pattern) {
                // The inverse puts back the map's own terrain, or the one
                // the height calls for.
                (Some(back), _, _) => ops::paint_terrain(&mut edit, brush, x, z, back, stroke.salt),
                (None, Some((theme, sea)), _) => ops::paint_terrain_with(&mut edit, brush, x, z, stroke.salt, |h| theme.terrain(h - 1.0, 0.0, *sea, 0.0)),
                (None, None, Some(pattern)) => ops::paint_pattern(&mut edit, brush, x, z, stroke.salt, pattern),
                (None, None, None) => ops::paint_terrain(&mut edit, brush, x, z, TerrainId(tools.terrain), stroke.salt),
            },
            Tool::Gradient => {
                if let Some(g) = &gradient {
                    ops::paint_gradient(&mut edit, brush, x, z, stroke.salt, g, tools.gradient.layout, &mut stroke.sides);
                }
            }
            Tool::PaintBiome => {
                let biome = if invert { 0 } else { tools.biome };
                ops::paint_biome(&mut edit, brush, x, z, BiomeId(biome), stroke.salt)
            }
            // The right button rubs out the layer chosen, the others left
            // where they are (as WorldPainter's does).
            Tool::Cover if invert => ops::erase_cover(&mut edit, brush, x, z, tools.cover, stroke.salt),
            Tool::Cover => ops::paint_cover(&mut edit, brush, x, z, tools.cover, stroke.salt),
            // A sub-canopy layer is painted beside the base one.
            Tool::Objects if understory && invert => ops::erase_understory(&mut edit, brush, x, z, tools.object_layer, stroke.salt),
            Tool::Objects if understory => ops::paint_understory(&mut edit, brush, x, z, tools.object_layer, stroke.salt),
            Tool::Objects if invert => ops::erase_objects(&mut edit, brush, x, z, tools.object_layer, stroke.salt),
            Tool::Objects => ops::paint_objects(&mut edit, brush, x, z, tools.object_layer, stroke.salt),
            Tool::TreeSize => ops::paint_tree_size(&mut edit, brush, x, z, if invert { 0 } else { tools.tree_size.clamp(1, massif_core::tile::SIZE_MAX) }, stroke.salt),
            Tool::Pockets if invert => ops::erase_pockets(&mut edit, brush, x, z, tools.pocket_layer, stroke.salt),
            Tool::Pockets => ops::paint_pockets(&mut edit, brush, x, z, tools.pocket_layer, stroke.salt),
            Tool::Slabs => ops::paint_slabs(&mut edit, brush, x, z, if invert { 0 } else { slabs }, stroke.salt),
            Tool::Combined => {
                ops::paint_parts(&mut edit, brush, x, z, &parts, stroke.salt);
                // The terrain put back as Paint terrain's right button does.
                if invert && combined.as_ref().is_some_and(|c| c.1) {
                    match (stroke.back, &theme) {
                        (Some(back), _) => ops::paint_terrain(&mut edit, brush, x, z, back, stroke.salt),
                        (None, Some((theme, sea))) => ops::paint_terrain_with(&mut edit, brush, x, z, stroke.salt, |h| theme.terrain(h - 1.0, 0.0, *sea, 0.0)),
                        (None, None) => {}
                    }
                }
                if !below.is_empty() {
                    dabs.push((x, z, stroke.salt));
                }
            }
            Tool::Annotate => ops::paint_annotation(&mut edit, brush, x, z, if invert { 0 } else { tools.annotation.clamp(1, 15) }, stroke.salt),
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
                let level = if invert { NO_FLUID } else { stroke.water_to.unwrap_or(tools.water_level) };
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
            Tool::Eyedropper | Tool::Flood | Tool::Pan | Tool::River | Tool::Road | Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush => {}
        }
    }
    // A combined layer's pockets, painted from another level: the same
    // dabs on the ground's.
    if !dabs.is_empty() {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        for (x, z, salt) in dabs {
            ops::paint_parts(&mut edit, brush, x, z, &below, salt);
        }
    }
    stroke.last = to;
}

/// End a stroke: one step to undo. The automatic tools say what they did.
pub fn end_stroke(doc: &mut Document, mut stroke: Stroke) -> Feedback {
    end_step(doc, stroke.step);
    if stroke.tool == Tool::Gradient {
        // What it learnt of the ground holds for the ground it left.
        stroke.sides.end(doc.dimension(), stroke.level.min(doc.dimension().levels.len() - 1));
        doc.gradient_sides = Some(std::mem::take(&mut stroke.sides));
    }
    match stroke.tool {
        Tool::AutoPaint | Tool::AutoBiome if !stroke.inverse => Some(match (stroke.changed, stroke.tool) {
            (0, _) => tr!("Nothing to change here: the land already follows the rules").to_owned(),
            (n, Tool::AutoPaint) => tr!("Repainted {n} columns", n),
            (n, _) => tr!("Changed the biome of {n} columns", n),
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
    /// The step to undo its 3D edits records into, and its label (a
    /// stroke of the map's brushes keeps its own, a click none).
    step: u64,
    label: String,
}

impl SculptStroke {
    /// What a stroke of the map's brushes has to say (see
    /// [`Stroke::take_note`]).
    pub fn take_note(&mut self) -> Option<String> {
        match &mut self.what {
            Stroke3d::Map(map) => map.take_note(),
            _ => None,
        }
    }
}

enum Stroke3d {
    /// Sculpting, pushing the surface by up to so many blocks a second
    /// (Raise / Lower's speed, set in Tool Settings).
    Sculpt(massif_core::voxel::SculptMode, f32),
    /// Paint the terrain; the inverse puts back `back`, the map's own
    /// terrain, or (`None`) the one the height calls for.
    Paint { inverse: bool, back: Option<TerrainId> },
    /// The map's own brush, as on the map, on the columns where the view
    /// points.
    Map(Box<Stroke>),
    /// A click, done as the button went down.
    Clicked,
    /// Flood held down: the water raised or lowered a block at a time.
    Water(WaterHold),
}

/// Begin a stroke in the 3D Sculpt editor with the tool of the Tools panel,
/// on the surface at `at`. `None` when the tool does nothing there (the
/// reason, or what a click did, in the feedback).
pub fn begin_sculpt(doc: &mut Document, tools: &mut ToolState, inverse: bool, at: &massif_core::voxel::Frame) -> (Option<SculptStroke>, Feedback) {
    use massif_core::voxel::SculptMode;
    let tool = tools.tool;
    let salt = doc.revision.wrapping_mul(0x9e37_79b9) ^ 0x3d5c_ab71;
    let (x, z) = (at.center[0], at.center[2]);
    let what = match tool.in_3d() {
        Use3d::MapOnly => return (None, None),
        Use3d::Click if tool == Tool::Flood => {
            let (hold, note) = press_water(doc, tools, x.floor() as i32, z.floor() as i32, inverse, false);
            let what = hold.map_or(Stroke3d::Clicked, Stroke3d::Water);
            return (Some(SculptStroke { what, salt, step: 0, label: String::new() }), note);
        }
        Use3d::Click => {
            let note = click(doc, tools, x.floor() as i32, z.floor() as i32, inverse);
            return (Some(SculptStroke { what: Stroke3d::Clicked, salt, step: 0, label: String::new() }), note);
        }
        Use3d::Brush => {
            let (stroke, note) = begin_stroke(doc, tools, (x, z), inverse, false);
            let Some(mut stroke) = stroke else { return (None, note) };
            continue_stroke(doc, tools, &mut stroke, (x, z), 1.0 / 60.0);
            return (Some(SculptStroke { what: Stroke3d::Map(Box::new(stroke)), salt, step: 0, label: String::new() }), note);
        }
        Use3d::Paint => Stroke3d::Paint { inverse, back: if inverse { unpaint_for_stroke(doc, tools) } else { None } },
        Use3d::Smooth => Stroke3d::Sculpt(SculptMode::Smooth, 0.0),
        // Dig digs, the others add; the inverse the other way. Raise /
        // Lower goes at its own speed.
        Use3d::Sculpt => {
            let speed = if tool == Tool::Raise { tools.raise_speed } else { SculptSettings::GROWTH };
            Stroke3d::Sculpt(if (tool == Tool::Dig) != inverse { SculptMode::Remove } else { SculptMode::Add }, speed)
        }
    };
    // Raise / Lower shapes the land in 3D here: a step of Sculpt.
    let label = if tool == Tool::Raise { tr!("Sculpt") } else { crate::i18n::tr_name(tool.label()) }.to_owned();
    let step = doc.history.begin(label.clone(), doc.dim);
    (Some(SculptStroke { what, salt, step, label }), None)
}

/// One dab of a 3D stroke on the surface at `frame`, lasting `dt` seconds:
/// the brush laid along the slope there. Sculpting pushes the surface out
/// (or in) a little — Raise / Lower's speed in blocks a second at full
/// strength, [`SculptSettings::GROWTH`] for Build and Dig —, smoothing
/// melts its bumps, painting paints the terrain of the columns under the
/// brush, the map's brushes act on the columns around, as on the map.
pub fn sculpt_dab(doc: &mut Document, tools: &ToolState, stroke: &mut SculptStroke, frame: &massif_core::voxel::Frame, dt: f32) -> usize {
    use massif_core::voxel::{self, SculptMode};
    // (A stroke of the map's brushes keeps its own step, a click none.)
    if matches!(stroke.what, Stroke3d::Sculpt(..) | Stroke3d::Paint { .. }) {
        keep_step(doc, &mut stroke.step, &stroke.label);
    }
    let brush = tools.sculpt_brush();
    let dt = dt.clamp(0.0, 0.1);
    stroke.salt = stroke.salt.wrapping_add(0x632b_e59b);
    match &mut stroke.what {
        Stroke3d::Map(map) => {
            continue_stroke(doc, tools, map, (frame.center[0], frame.center[2]), dt);
            1
        }
        Stroke3d::Clicked => 0,
        Stroke3d::Water(hold) => hold_water(doc, tools, hold, dt) as usize,
        Stroke3d::Sculpt(SculptMode::Smooth, _) => {
            let mut edit = doc.history.edit(&mut doc.world, 0);
            voxel::sculpt_at(&mut edit, &brush, frame.center, Some(frame.normal), sculpt_strength(&brush, dt), SculptMode::Smooth)
        }
        &mut Stroke3d::Sculpt(mode, speed) => {
            let mut edit = doc.history.edit(&mut doc.world, 0);
            voxel::displace_at(&mut edit, &brush, frame.center, frame.normal, brush.strength * speed * dt, mode)
        }
        &mut Stroke3d::Paint { inverse, back } => {
            let reg = &doc.world.registry;
            let pattern = (!inverse && tools.pattern.on).then(|| tools.pattern.sampler(reg)).flatten();
            let theme = (inverse && back.is_none()).then(|| (massif_core::generate::Theme::new(reg), doc.world.meta.sea_level as f32));
            let brush = Brush {
                filter: tools.filter.is_active().then(|| std::sync::Arc::new(tools.filter.clone())),
                ..brush
            };
            let level = doc.active_level.min(doc.dimension().levels.len() - 1);
            let (x, z) = (frame.center[0], frame.center[2]);
            let mut edit = doc.history.edit(&mut doc.world, level);
            match (back.filter(|_| inverse), &theme, &pattern) {
                (Some(back), _, _) => ops::paint_terrain(&mut edit, &brush, x, z, back, stroke.salt),
                (None, Some((theme, sea)), _) => ops::paint_terrain_with(&mut edit, &brush, x, z, stroke.salt, |h| theme.terrain(h - 1.0, 0.0, *sea, 0.0)),
                (None, None, Some(pattern)) => ops::paint_pattern(&mut edit, &brush, x, z, stroke.salt, pattern),
                (None, None, None) => ops::paint_terrain(&mut edit, &brush, x, z, TerrainId(tools.terrain), stroke.salt),
            }
            // The matter sculpting moved keeps its own terrain: it is
            // painted too (back to the column's with the inverse).
            let painted = TerrainId(tools.terrain);
            match (inverse, &pattern) {
                (true, _) => voxel::paint_sculpted(&mut edit, &brush, x, z, |_, _| None),
                (false, Some(pattern)) => voxel::paint_sculpted(&mut edit, &brush, x, z, |px, pz| pattern.terrain(px, pz)),
                (false, None) => voxel::paint_sculpted(&mut edit, &brush, x, z, |_, _| Some(painted)),
            };
            1
        }
    }
}

/// End a stroke of the 3D Sculpt editor: one step to undo. The automatic
/// tools say what they did.
pub fn end_sculpt(doc: &mut Document, stroke: SculptStroke) -> Feedback {
    match stroke.what {
        Stroke3d::Map(map) => end_stroke(doc, *map),
        Stroke3d::Clicked => None,
        Stroke3d::Water(hold) => release_water(doc, hold),
        Stroke3d::Sculpt(..) | Stroke3d::Paint { .. } => {
            end_step(doc, stroke.step);
            None
        }
    }
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

/// The columns a flood changes at most: past them, it changes none.
const FLOOD_MAX: usize = 64 << 20;

/// [`FLOOD_MAX`], or the limit a test sets on its thread.
fn flood_max() -> usize {
    #[cfg(test)]
    if let Some(n) = FLOOD_LIMIT.with(|l| l.get()) {
        return n;
    }
    FLOOD_MAX
}

#[cfg(test)]
thread_local! {
    static FLOOD_LIMIT: std::cell::Cell<Option<usize>> = const { std::cell::Cell::new(None) };
}

/// A flood from the click on a spot not in a hollow, to ask about: where,
/// the level the water would stand at (a block over the spot), and how
/// many columns it would cover.
#[derive(Debug, Clone, PartialEq)]
pub struct FloodAsk {
    pub x: i32,
    pub z: i32,
    pub level: i16,
    pub columns: usize,
    /// Raising water at the rim of its hollow, rather than on dry land.
    pub at_rim: bool,
}

/// Flood as asked: up to the level of `ask`, from where it was clicked;
/// one step to undo. What it did.
pub fn flood_anyway(doc: &mut Document, ask: &FloodAsk) -> String {
    doc.history.begin(tr!("Flood"), doc.dim);
    let n = ops::flood_fill(&mut doc.history.edit(&mut doc.world, 0), ask.x, ask.z, Flood::Fill(ask.level), flood_max());
    doc.commit();
    match n {
        Ok(n) => tr!("Flooded {n} columns up to {water_level}", water_level = ask.level, n),
        Err(TooMany) => too_many_to_flood(),
    }
}

/// What raising water at `level` (raised `blocks` so far) says when a block
/// more would spread it over more than [`FLOOD_MAX`] columns.
fn too_many_to_raise(level: i16, blocks: u32) -> String {
    let columns = crate::select::thousands(flood_max() as u64);
    if blocks == 0 {
        tr!("The water stays at {level}: a block higher, it would spread over more than {columns} columns, too much for one flood.", level, columns)
    } else {
        trn!("Water raised by {n} block, to {level}: a block higher, it would spread over more than {columns} columns, too much for one flood.", "Water raised by {n} blocks, to {level}: a block higher, it would spread over more than {columns} columns, too much for one flood.", n = blocks, level, columns)
    }
}

/// What a flood past [`FLOOD_MAX`] columns says: it floods none, since
/// stopped halfway, it would leave the water it laid standing as a wall
/// where it stopped.
fn too_many_to_flood() -> String {
    tr!("Too much water for one flood: it would cover more than {columns} columns. Nothing was flooded.", columns = crate::select::thousands(flood_max() as u64))
}

/// The level a Water brush stroke starting on column `(x, z)` lays its
/// water at, before the limit: the water's surface there, or a block over
/// the ground, with Where I start; the sea's or the one set otherwise.
fn brush_level(doc: &Document, tools: &ToolState, x: i32, z: i32) -> i16 {
    if let Some(level) = tools.brush_height.level(doc, tools) {
        return level;
    }
    match (ops::water_at(doc.dimension(), x, z), ground_top(doc, x, z)) {
        (Some(w), _) => w,
        (None, Some(g)) => (g + 1).clamp(i16::MIN as i32, i16::MAX as i32) as i16,
        (None, None) => tools.water_level,
    }
}

/// Columns of water a hollow holds at least for Flood to stop at it:
/// water poured on a slope runs on past a smaller puddle.
const POND: usize = 12;
/// Blocks from the column clicked water poured there runs at most, down to
/// the hollow it fills.
const POUR_REACH: i32 = 256;

/// Where Flood fills from, on dry land, and up to what level.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Fill {
    /// The column it fills from: the one clicked, or the bottom of the
    /// hollow the water runs into.
    x: i32,
    z: i32,
    level: i16,
    /// Up to the level set (Sea level or Fixed), rather than to where a
    /// hollow spills.
    set: bool,
    /// The level set, as high as the land clicked or under it: the hollow
    /// the water runs into instead.
    above: Option<i16>,
    /// The water poured on the column clicked runs away: the nearest
    /// hollow around it.
    near: bool,
}

/// Where Flood fills from on dry column `(x, z)`, and up to what level,
/// under the limit; or what to say instead, a question asked included.
/// Under a level set, the area clicked up to it; otherwise (Where I click,
/// or land as high as the level set) the hollow the water poured there
/// runs into, up to where it spills.
fn pour_mode(doc: &Document, tools: &mut ToolState, x: i32, z: i32) -> Result<Fill, Feedback> {
    // Never over the top of the world.
    let top = doc.dimension().max_y as i16;
    let ground = ground_top(doc, x, z);
    let above = tools.flood_height.level(doc, tools).map(|level| tools.limit_water(level).min(top));
    if let Some(level) = above
        && ground.is_some_and(|g| g < level as i32)
    {
        return Ok(Fill { x, z, level, set: true, above: None, near: false });
    }
    match ops::pour_target(doc.dimension(), x, z, POND, POUR_REACH, 16 << 20) {
        None => Err(Some(tr!("There is no land here to hold water.").to_owned())),
        Some(ops::Pour::Hollow { x: hx, z: hz, level, near, .. }) => {
            let level = tools.limit_water(level).min(top);
            // Only a height limit keeps a hollow dry.
            if ground_top(doc, hx, hz).is_some_and(|g| g >= level as i32) {
                return Err(Some(under_the_limit(tools)));
            }
            Ok(Fill { x: hx, z: hz, level, set: false, above, near })
        }
        // Not in a hollow: the water would run away. Made to stay a block
        // over the spot, it spreads as far as the land lies under it:
        // asked first, unless told not to.
        Some(_) => {
            let Some(level) = ops::pour_level(doc.dimension(), x, z) else {
                return Err(Some(tr!("There is no land here to hold water.").to_owned()));
            };
            let level = tools.limit_water(level).min(top);
            if ground.is_some_and(|g| g >= level as i32) {
                return Err(Some(under_the_limit(tools)));
            }
            if tools.flood_warn {
                let Ok(columns) = ops::flood_count(doc.dimension(), x, z, Flood::Fill(level), flood_max()) else {
                    return Err(Some(too_many_to_flood()));
                };
                tools.flood_ask = Some(FloodAsk { x, z, level, columns, at_rim: false });
                return Err(None);
            }
            Ok(Fill { x, z, level, set: false, above, near: false })
        }
    }
}

/// The top of the ground at column `(x, z)`, as a block's y.
fn ground_top(doc: &Document, x: i32, z: i32) -> Option<i32> {
    probe(doc, x, z).map(|p| block_top(p.height))
}

/// What a flood of `n` columns up to `level` (or a drain, `flood` false)
/// says.
fn poured(doc: &Document, tools: &ToolState, x: i32, z: i32, flood: bool, level: i16, n: usize) -> String {
    match (flood, n) {
        (true, 0) if ground_top(doc, x, z).is_some_and(|g| g >= level as i32) => dry_here(tools, level),
        (_, 0) => tr!("Nothing to fill or drain here").to_owned(),
        (true, n) => tr!("Flooded {n} columns up to {water_level}", water_level = level, n),
        (false, n) => tr!("Drained {n} columns", n),
    }
}

/// What the water tools say on ground as high as the water they lay, at
/// `level`: what keeps it dry.
fn dry_here(tools: &ToolState, level: i16) -> String {
    if tools.water_limit_on && level >= tools.water_limit {
        tr!("Dry here: the height limit, y {water_limit}, is under the ground.", water_limit = tools.water_limit)
    } else {
        tr!("Dry here: the ground is at or above y {level}.", level)
    }
}

/// What the water tool in hand would do at column `(x, z)`, for the tag
/// by the cursor: the level it fills to, or why it stays dry. A stroke of
/// the Water brush under way lays its water at `stroke`.
#[cfg(test)]
pub fn water_hint(doc: &Document, tools: &ToolState, x: i32, z: i32, stroke: Option<i16>) -> Option<String> {
    water_tag(doc, tools, x, z, stroke).map(|t| t.0)
}

/// [`water_hint`], and where Flood's water would run down to when it
/// leaves the column dry: the bottom of the hollow it fills.
pub fn water_tag(doc: &Document, tools: &ToolState, x: i32, z: i32, stroke: Option<i16>) -> Option<(String, Option<(i32, i32)>)> {
    let ground = ground_top(doc, x, z)?;
    let water = ops::water_at(doc.dimension(), x, z);
    let fills = |level: i16| if ground >= level as i32 { dry_here(tools, level) } else { tr!("Fills to y {level}", level) };
    match tools.tool {
        Tool::Water => Some((fills(stroke.unwrap_or_else(|| tools.limit_water(brush_level(doc, tools, x, z)))), None)),
        Tool::Flood => {
            // On water, whatever the height chosen: a block higher. The rim
            // is worked out over a hollow of a reasonable size only, the
            // cursor moving on.
            if let Some(w) = water {
                let higher = w.saturating_add(1);
                return Some((if higher > doc.dimension().max_y as i16 {
                    tr!("The water here is at the top of the world already").to_owned()
                } else if tools.limit_water(higher) < higher {
                    limit_reached(tools)
                } else if tools.flood_warn && ops::spill_level_within(doc.dimension(), x, z, 1 << 16).is_some_and(|rim| rim.is_none_or(|rim| higher > rim)) {
                    tr!("At the rim, y {level}: a block more spills", level = w)
                } else {
                    tr!("Raise this water to y {level}", level = higher)
                }, None));
            }
            flood_hint(doc, tools, x, z, ground)
        }
        _ => None,
    }
}

/// What Flood would do on dry column `(x, z)`, whose ground is at
/// `ground`, for the tag by the cursor; and the bottom of the hollow the
/// water would run down into, when it leaves the column dry. The hollow
/// is looked for over a reasonable stretch only, the cursor moving on.
fn flood_hint(doc: &Document, tools: &ToolState, x: i32, z: i32, ground: i32) -> Option<(String, Option<(i32, i32)>)> {
    let top = doc.dimension().max_y as i16;
    let above = tools.flood_height.level(doc, tools).map(|level| tools.limit_water(level).min(top));
    if let Some(level) = above
        && ground < level as i32
    {
        return Some((tr!("Fill to y {level}", level), None));
    }
    Some(match ops::pour_target(doc.dimension(), x, z, POND, POUR_REACH, 1 << 16)? {
        ops::Pour::Hollow { x: hx, z: hz, level, capped, near, .. } => {
            let level = tools.limit_water(level).min(top);
            // The hollow the water runs down into covers the column if it
            // fills higher; the nearest one around lies apart from it.
            let here = !near && ground < level as i32;
            let target = (!here).then_some((hx, hz));
            if ground_top(doc, hx, hz).is_some_and(|g| g >= level as i32) {
                (dry_here(tools, level), None)
            } else {
                let text = match (above, near, capped, here) {
                    (None, true, ..) => tr!("No hollow here: fills the nearest one, to y {level}", level),
                    (Some(above), true, ..) => tr!("Above y {above}: fills the nearest hollow, to y {level}", above, level),
                    (None, false, true, _) => tr!("Fills the hollow up to where it spills").to_owned(),
                    (Some(above), false, true, _) => tr!("Above y {above}: fills the hollow up to where it spills", above),
                    (None, false, false, true) => tr!("Fill to y {level}", level),
                    (Some(above), false, false, true) => tr!("Above y {above}: fills this hollow instead, to y {level}", above, level),
                    (None, false, false, false) => tr!("Runs down into a hollow: fills it to y {level}", level),
                    (Some(above), false, false, false) => tr!("Above y {above}: runs down into a hollow, fills it to y {level}", above, level),
                };
                (text, target)
            }
        }
        _ => match above {
            None => (tr!("Not in a hollow: the water would spread").to_owned(), None),
            Some(above) => (tr!("Above y {above}, and not in a hollow: the water would spread", above), None),
        },
    })
}

/// What Flood says when the water stands at the height limit.
fn limit_reached(tools: &ToolState) -> String {
    tr!("Height limit y {water_limit} reached", water_limit = tools.water_limit)
}

/// The Flood tool held down: the water raised (or lowered) a block a
/// step, at the pace set in Tool Settings, all of it one step to undo.
pub struct WaterHold {
    /// A column of the body of water: where it was pressed, or, lowering,
    /// one it still covers.
    x: i32,
    z: i32,
    lower: bool,
    /// The step to undo it records into.
    step: u64,
    label: String,
    /// The highest the water may rise: the rim of its hollow (`None`: no
    /// rim to keep to, as told). Worked out at the first rise.
    rim: Option<Option<i16>>,
    /// Seconds since the last block.
    wait: f32,
    /// Blocks raised or lowered.
    blocks: u32,
    /// The level the water stands at, `None` once it is gone.
    level: Option<i16>,
    /// The press began on dry land: the columns its flood covered.
    flooded: Option<usize>,
    /// The water ran down into a hollow, leaving the column pressed dry.
    ran: bool,
    /// The level set, as high as the land pressed: the hollow the water
    /// runs into was filled instead.
    above: Option<i16>,
    /// The water poured where pressed ran away: the nearest hollow around
    /// was filled.
    near: bool,
    /// Why it goes no further.
    end: Option<HoldEnd>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum HoldEnd {
    /// The water is at the rim of its hollow.
    Rim,
    /// At the height limit set in Tool Settings.
    Limit,
    /// At the top of the world.
    Top,
    /// The last layer is gone.
    Gone,
    /// Nothing more to change.
    Still,
    /// A block more (or less) would change more than [`FLOOD_MAX`] columns.
    TooMany,
}

impl WaterHold {
    /// Whether it has gone as far as it goes.
    pub fn ended(&self) -> bool {
        self.end.is_some()
    }

    /// The blocks raised or lowered so far.
    pub fn blocks(&self) -> u32 {
        self.blocks
    }
}

/// The time between two blocks of a held Flood, in seconds.
fn hold_period(tools: &ToolState) -> f32 {
    1.0 / tools.water_rate.clamp(1.0, 30.0)
}

/// Press the Flood tool on column `(x, z)`: the left button floods the
/// hollow, or, on water, raises it a block; the right one (`lower`) lowers
/// it a block; with Shift (`drain_all`) drains it whole. A hold to keep
/// going while the button is down ([`hold_water`]), and to let go
/// ([`release_water`]); `None` when it is done (what it did, or why not,
/// in the feedback).
pub fn press_water(doc: &mut Document, tools: &mut ToolState, x: i32, z: i32, lower: bool, drain_all: bool) -> (Option<WaterHold>, Feedback) {
    if drain_all {
        doc.history.begin(tr!("Drain"), doc.dim);
        let n = ops::flood_fill(&mut doc.history.edit(&mut doc.world, 0), x, z, Flood::Drain, flood_max());
        doc.commit();
        return (None, Some(match n {
            Ok(n) => poured(doc, tools, x, z, false, 0, n),
            Err(TooMany) => tr!("Too much water to drain at once: it covers more than {columns} columns. Nothing was drained.", columns = crate::select::thousands(flood_max() as u64)),
        }));
    }
    let water = ops::water_at(doc.dimension(), x, z);
    let mut hold = WaterHold { x, z, lower, step: 0, label: String::new(), rim: None, wait: 0.0, blocks: 0, level: water, flooded: None, ran: false, above: None, near: false, end: None };
    match (lower, water) {
        (true, None) => return (None, Some(tr!("No water here to lower").to_owned())),
        (true, Some(_)) => hold.label = tr!("Lower the water").to_owned(),
        // On water, whatever the height chosen: raised a block. The height
        // set is for a new fill only.
        (false, Some(level)) => {
            let higher = level.saturating_add(1);
            if higher > doc.dimension().max_y as i16 {
                return (None, Some(tr!("The water here is at the top of the world already").to_owned()));
            }
            if tools.limit_water(higher) < higher {
                return (None, Some(limit_reached(tools)));
            }
            // At the rim already: a block more, and it spills. Asked first.
            let rim = ops::spill_level(doc.dimension(), x, z, 16 << 20);
            if tools.flood_warn && rim.is_none_or(|rim| higher > rim) {
                let Ok(columns) = ops::flood_count(doc.dimension(), x, z, Flood::Fill(higher), flood_max()) else {
                    return (None, Some(too_many_to_raise(level, 0)));
                };
                tools.flood_ask = Some(FloodAsk { x, z, level: higher, columns, at_rim: true });
                return (None, None);
            }
            hold.rim = Some(rim.filter(|_| tools.flood_warn));
            hold.label = tr!("Raise the water").to_owned();
        }
        // Dry land: flooded up to the level chosen (Where I click, the
        // hold raising it on from there).
        (false, None) => {
            let fill = match pour_mode(doc, tools, x, z) {
                Ok(fill) => fill,
                Err(note) => return (None, note),
            };
            hold.label = tr!("Flood").to_owned();
            hold.step = doc.history.begin(hold.label.clone(), doc.dim);
            let n = match ops::flood_fill(&mut doc.history.edit(&mut doc.world, 0), fill.x, fill.z, Flood::Fill(fill.level), flood_max()) {
                Ok(n) => n,
                Err(TooMany) => {
                    doc.commit();
                    return (None, Some(too_many_to_flood()));
                }
            };
            if n == 0 {
                doc.commit();
                return (None, Some(poured(doc, tools, fill.x, fill.z, true, fill.level, n)));
            }
            // Held, the water it laid rises on.
            (hold.x, hold.z) = (fill.x, fill.z);
            hold.flooded = Some(n);
            hold.level = ops::water_at(doc.dimension(), fill.x, fill.z);
            hold.ran = ops::water_at(doc.dimension(), x, z).is_none();
            hold.above = fill.above;
            hold.near = fill.near;
            if fill.set {
                hold.end = Some(HoldEnd::Still);
            }
            return (Some(hold), None);
        }
    }
    hold.step = doc.history.begin(hold.label.clone(), doc.dim);
    water_step(doc, tools, &mut hold);
    (Some(hold), None)
}

/// A block more of a held Flood (or less).
fn water_step(doc: &mut Document, tools: &ToolState, hold: &mut WaterHold) {
    keep_step(doc, &mut hold.step, &hold.label);
    let Some(level) = hold.level else {
        hold.end = Some(HoldEnd::Gone);
        return;
    };
    if hold.lower {
        let mut edit = doc.history.edit(&mut doc.world, 0);
        match ops::lower_water(&mut edit, hold.x, hold.z, flood_max()) {
            Some(Err(TooMany)) => hold.end = Some(HoldEnd::TooMany),
            Some(Ok((n, kept))) if n > 0 => {
                hold.blocks += 1;
                match kept {
                    Some((x, z)) => {
                        (hold.x, hold.z) = (x, z);
                        hold.level = Some(level - 1);
                    }
                    None => {
                        hold.level = None;
                        hold.end = Some(HoldEnd::Gone);
                    }
                }
            }
            _ => hold.end = Some(HoldEnd::Still),
        }
        return;
    }
    let higher = level.saturating_add(1);
    // No higher than the height limit, nor than the top of the world.
    if higher > doc.dimension().max_y as i16 {
        hold.end = Some(HoldEnd::Top);
        return;
    }
    if tools.limit_water(higher) < higher {
        hold.end = Some(HoldEnd::Limit);
        return;
    }
    let (x, z) = (hold.x, hold.z);
    let rim = *hold.rim.get_or_insert_with(|| tools.flood_warn.then(|| ops::spill_level(doc.dimension(), x, z, 16 << 20).unwrap_or(level)));
    if rim.is_some_and(|rim| higher > rim) {
        hold.end = Some(HoldEnd::Rim);
        return;
    }
    match ops::raise_water(&mut doc.history.edit(&mut doc.world, 0), hold.x, hold.z, flood_max()) {
        Some((Ok(n), level)) if n > 0 => {
            hold.blocks += 1;
            hold.level = Some(level);
        }
        Some((Err(TooMany), _)) => hold.end = Some(HoldEnd::TooMany),
        _ => hold.end = Some(HoldEnd::Still),
    }
}

/// Keep a held Flood going for `dt` more seconds: a block each time the
/// pace set comes round, until the water reaches the rim of its hollow
/// (unless told to flood beyond it), the height limit or the bottom.
/// Whether it changed the water.
pub fn hold_water(doc: &mut Document, tools: &ToolState, hold: &mut WaterHold, dt: f32) -> bool {
    if hold.end.is_some() {
        return false;
    }
    let period = hold_period(tools);
    // A slow frame does not let the water jump: two blocks at most.
    hold.wait = (hold.wait + dt.max(0.0)).min(period * 2.0);
    let mut changed = false;
    while hold.wait >= period && hold.end.is_none() {
        hold.wait -= period;
        let before = hold.blocks;
        water_step(doc, tools, hold);
        changed |= hold.blocks > before;
    }
    changed
}

/// Let go of a held Flood: its step closed. What it did.
pub fn release_water(doc: &mut Document, hold: WaterHold) -> Feedback {
    end_step(doc, hold.step);
    let level = hold.level;
    Some(match (hold.flooded, hold.lower, hold.end, level) {
        (Some(n), _, _, Some(level)) if hold.blocks == 0 => match (hold.above, hold.ran, hold.near) {
            (None, false, _) => tr!("Flooded {n} columns up to {water_level}", water_level = level, n),
            (None, true, false) => tr!("The water ran down into a hollow: flooded {n} columns up to {water_level}", water_level = level, n),
            (None, true, true) => tr!("No hollow here: filled the nearest one, {n} columns up to {water_level}", water_level = level, n),
            (Some(above), false, _) => tr!("The land is above y {above}: filled the hollow here instead, {n} columns up to {water_level}", above, water_level = level, n),
            (Some(above), true, false) => tr!("The land is above y {above}: the water ran down into a hollow, {n} columns up to {water_level}", above, water_level = level, n),
            (Some(above), true, true) => tr!("The land is above y {above}: filled the nearest hollow, {n} columns up to {water_level}", above, water_level = level, n),
        },
        (_, true, Some(HoldEnd::TooMany), _) => tr!("Too much water to lower at once: it covers more than {columns} columns.", columns = crate::select::thousands(flood_max() as u64)),
        (_, false, Some(HoldEnd::TooMany), Some(level)) => too_many_to_raise(level, hold.blocks),
        (_, true, _, None) => tr!("The water is gone").to_owned(),
        (_, true, _, Some(level)) => trn!("Water lowered by {n} block, to {level}", "Water lowered by {n} blocks, to {level}", n = hold.blocks, level),
        (_, false, Some(HoldEnd::Rim), Some(level)) => trn!("Water raised by {n} block, to {level}: it is at the rim of its hollow, a block more and it spills.", "Water raised by {n} blocks, to {level}: it is at the rim of its hollow, a block more and it spills.", n = hold.blocks, level),
        (_, false, Some(HoldEnd::Limit), Some(level)) => trn!("Water raised by {n} block, to {level}: height limit reached", "Water raised by {n} blocks, to {level}: height limit reached", n = hold.blocks, level),
        (_, false, _, Some(level)) => trn!("Water raised by {n} block, to {level}", "Water raised by {n} blocks, to {level}", n = hold.blocks, level),
        _ => tr!("Nothing to fill or drain here").to_owned(),
    })
}

/// Alt + click, as WorldPainter's eyedropper: what is under the cursor, for
/// the tool in hand: the biome for Paint biome, the ground cover, the
/// object layer for theirs, the terrain for the others (Paint terrain
/// leaving its noise for that terrain). What was picked, or why nothing.
pub fn pick(doc: &Document, tools: &mut ToolState, x: i32, z: i32) -> Feedback {
    let Some(p) = probe(doc, x, z) else {
        return Some(tr!("Nothing here to pick").to_owned());
    };
    let reg = &doc.world.registry;
    Some(match tools.tool {
        Tool::PaintBiome if p.biome != 0 => {
            tools.biome = p.biome;
            tr!("Picked the biome {name}", name = reg.biome(massif_core::registry::BiomeId(p.biome)).name)
        }
        Tool::PaintBiome => tr!("No biome painted here").to_owned(),
        Tool::Cover if p.cover != 0 => {
            tools.cover = p.cover;
            tr!("Picked the ground cover {name}", name = reg.covers.get(p.cover as usize - 1).map_or("", |c| c.name.as_str()))
        }
        Tool::Cover => tr!("No ground cover here").to_owned(),
        Tool::Objects if p.objects != 0 => {
            tools.object_layer = p.objects;
            tr!("Picked the layer {name}", name = reg.object_layers.get(p.objects as usize - 1).map_or("", |l| l.name.as_str()))
        }
        Tool::Objects => tr!("No trees nor objects here").to_owned(),
        _ => {
            tools.terrain = p.terrain;
            tools.pattern.on = false;
            tr!("Picked {terrain_name}", terrain_name = reg.terrain(TerrainId(p.terrain)).name)
        }
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
            Some(tr!("Picked {terrain_name} at height {block_top}", block_top = block_top(p.height), terrain_name))
        }
        // Flood's own button, the right one: a block lower (Shift: drained
        // whole).
        Tool::Flood => {
            let (hold, note) = press_water(doc, tools, x, z, inverse, false);
            match hold {
                Some(hold) => release_water(doc, hold).or(note),
                None => note,
            }
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

    /// Paint terrain's right button puts back the map's own terrain: on a
    /// map filled with rock, rock, not the grass (or snow) its height calls
    /// for; on a map saved before it kept its terrain, the one covering
    /// nearly all of it, kept with the map from then on; by height on a map
    /// whose terrains its height chose, or when asked.
    #[test]
    fn the_right_button_puts_back_the_maps_terrain() {
        let rocky = || {
            let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Flat, surface: Some(TerrainId::ROCK), ..GenParams::default() });
            Document::new(world, None, 1)
        };
        let paint_then_unpaint = |d: &mut Document, tools: &mut ToolState| {
            let moss = TerrainId((2..).find(|t| *t != TerrainId::ROCK.0 && TerrainId(probe(d, 20, 20).unwrap().terrain) != TerrainId(*t)).unwrap());
            tools.select(Tool::PaintTerrain);
            tools.terrain = moss.0;
            tools.brush.strength = 1.0;
            tools.brush.radius = 6.0;
            tools.brush.profile = massif_core::Profile::Constant;
            for inverse in [false, true] {
                let (stroke, _) = begin_stroke(d, tools, (20.5, 20.5), inverse, inverse);
                let mut stroke = stroke.unwrap();
                continue_stroke(d, tools, &mut stroke, (20.5, 20.5), 0.1);
                end_stroke(d, stroke);
                if !inverse {
                    assert_eq!(TerrainId(probe(d, 20, 20).unwrap().terrain), moss);
                }
            }
            TerrainId(probe(d, 20, 20).unwrap().terrain)
        };
        let by_height = |d: &Document| massif_core::generate::Theme::new(&d.world.registry).terrain(height(d, 20, 20) - 1.0, 0.0, d.world.meta.sea_level as f32, 0.0);
        let mut tools = ToolState::default();
        assert_eq!(tools.unpaint, Unpaint::MapTerrain);

        // A new map of rock keeps it.
        let mut d = rocky();
        assert_eq!(d.world.meta.fill, Some(TerrainId::ROCK));
        assert_ne!(by_height(&d), TerrainId::ROCK, "the height calls for another terrain there");
        assert_eq!(paint_then_unpaint(&mut d, &mut tools), TerrainId::ROCK);
        // Asked, by height.
        tools.unpaint = Unpaint::ByHeight;
        assert_eq!(paint_then_unpaint(&mut d, &mut tools), by_height(&d));
        tools.unpaint = Unpaint::MapTerrain;

        // A map of rock saved before: read from the land, then kept.
        let mut d = rocky();
        d.world.meta.fill = None;
        assert_eq!(d.map_terrain(), Some(TerrainId::ROCK));
        assert_eq!(paint_then_unpaint(&mut d, &mut tools), TerrainId::ROCK);
        assert_eq!(d.world.meta.fill, Some(TerrainId::ROCK), "kept with the map");

        // A map whose terrains the height chose (here, grass all over): by
        // height.
        let mut d = doc();
        assert_eq!(d.world.meta.fill, None);
        assert_eq!(d.map_terrain(), None);
        assert_eq!(paint_then_unpaint(&mut d, &mut tools), by_height(&d));
        assert_eq!(d.world.meta.fill, None);

        // The terrain chosen for the map in Tool Settings.
        let sand = d.world.registry.terrain_by_name("Sand").unwrap();
        d.world.meta.fill = Some(sand);
        assert_eq!(paint_then_unpaint(&mut d, &mut tools), sand);
        // And in 3D Sculpt, as the player.
        let mut d = rocky();
        tools.select(Tool::PaintTerrain);
        tools.terrain = sand.0;
        let frame = massif_core::voxel::Frame { center: [20.5, height(&d, 20, 20), 20.5], normal: [0.0, 1.0, 0.0] };
        for inverse in [false, true] {
            let mut stroke = begin_sculpt(&mut d, &mut tools, inverse, &frame).0.unwrap();
            sculpt_dab(&mut d, &tools, &mut stroke, &frame, 0.1);
            end_sculpt(&mut d, stroke);
            assert_eq!(TerrainId(probe(&d, 20, 20).unwrap().terrain), if inverse { TerrainId::ROCK } else { sand });
        }
    }

    /// Clicking the Gradient brush again where it painted changes nothing,
    /// even a gradient of two terrains whose noise scatters each into the
    /// other (the document keeps what it learnt of the ground); after an
    /// undo it paints the same again; the sand it does not paint stays.
    #[test]
    fn clicking_the_gradient_again_changes_nothing() {
        let mut doc = doc();
        let mut tools = ToolState::default();
        let reg = doc.world.registry.clone();
        let id = |n: &str| reg.terrain_by_name(n).unwrap().0;
        let click = |doc: &mut Document, tools: &ToolState, at: (f32, f32)| {
            let (stroke, _) = begin_stroke(doc, tools, at, false, false);
            let mut stroke = stroke.unwrap();
            continue_stroke(doc, tools, &mut stroke, at, 0.02);
            end_stroke(doc, stroke);
        };
        let snapshot = |doc: &Document| -> Vec<u16> { (-40..80).flat_map(|z| (-40..80).map(move |x| (x, z))).map(|(x, z)| probe(doc, x, z).unwrap().terrain).collect() };
        // Podzol over the east, grass west of it, a strip of sand.
        tools.select(Tool::PaintTerrain);
        for (t, x, r) in [("Grass", 20.0, 60.0), ("Podzol", 60.0, 30.0), ("Sand", 12.0, 2.0)] {
            tools.terrain = id(t);
            tools.brush = Brush::symmetric(massif_core::BrushShape::Square, massif_core::Profile::Constant, r);
            click(&mut doc, &tools, (x, 20.0));
        }
        tools.select(Tool::Gradient);
        tools.gradient.gradient = massif_core::gradient::Gradient { wander: 0.5, ..massif_core::gradient::Gradient::new("two", &[("Podzol", 6.0), ("Grass", 10.0)], &[massif_core::gradient::Transition::noise(10.0, 0.9, 3.0)]) };
        tools.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 30.0);
        let before = snapshot(&doc);
        click(&mut doc, &tools, (30.0, 20.0));
        let once = snapshot(&doc);
        assert_ne!(once, before);
        for _ in 0..4 {
            click(&mut doc, &tools, (30.0, 20.0));
        }
        assert_eq!(snapshot(&doc), once, "five clicks paint what one did");
        assert!((10..14).all(|x| probe(&doc, x, 20).unwrap().terrain == id("Sand")), "the sand stays");
        // Undone, then clicked again: the same.
        for _ in 0..5 {
            doc.undo();
        }
        assert_eq!(snapshot(&doc), before);
        click(&mut doc, &tools, (30.0, 20.0));
        assert_eq!(snapshot(&doc), once);
    }

    /// The Gradient brush on a real shore (MASSIF_SHORE_WORLD: a .massif;
    /// MASSIF_SNAPSHOT_DIR): a forest of podzol laid inland of grass by a
    /// sand shore and the sea, then the terraformers' gradient clicked on
    /// its edge by the shore with a Sine circle of radius 131, once and
    /// twice. Renders the place before, after one click and after two;
    /// the second only fills in the soft rim the first left half painted,
    /// as a hard brush paints it in one click; the beach and the sea floor
    /// stay as they were.
    #[test]
    fn the_gradient_by_a_real_shore() {
        let (Some(path), Some(dir)) = (std::env::var_os("MASSIF_SHORE_WORLD"), std::env::var_os("MASSIF_SNAPSHOT_DIR")) else { return };
        let dir = std::path::PathBuf::from(dir);
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let mut doc = Document::new(world, None, 1);
        let reg = doc.world.registry.clone();
        let id = |n: &str| reg.terrain_by_name(n).unwrap().0;
        let (grass, sand, beach) = (id("Grass"), id("Sand"), id("Beach mix"));
        let dry = |doc: &Document, x: i32, z: i32, t: u16| probe(doc, x, z).is_some_and(|p| p.terrain == t && (p.fluid == NO_FLUID || (p.fluid as f32) <= p.height));
        let wet = |doc: &Document, x: i32, z: i32| probe(doc, x, z).is_some_and(|p| p.fluid != NO_FLUID && p.fluid as f32 > p.height + 2.0);
        // Grass, then a beach a few blocks on, then the sea, any way: the
        // nearest to the map's middle.
        let ways = [(1, 0), (-1, 0), (0, 1), (0, -1)];
        let shore = |doc: &Document, x: i32, z: i32, (dx, dz): (i32, i32)| {
            (0..4).all(|k| dry(doc, x - dx * k * 6, z - dz * k * 6, grass))
                && (4..30).any(|k| dry(doc, x + dx * k, z + dz * k, sand) || dry(doc, x + dx * k, z + dz * k, beach))
                && (20..90).any(|k| wet(doc, x + dx * k, z + dz * k))
        };
        // (MASSIF_SHORE_AT: "x,z,dx,dz", where it is, to look no further.)
        let given = std::env::var("MASSIF_SHORE_AT").ok().map(|v| v.split(',').map(|n| n.trim().parse::<i32>().unwrap()).collect::<Vec<_>>());
        let (x, z, (dx, dz)) = match given.as_deref() {
            Some(&[x, z, dx, dz]) => (x, z, (dx, dz)),
            _ => {
                let mut spots: Vec<(i32, i32)> = (-2900..2900).step_by(32).flat_map(|z| (-2900..2900).step_by(32).map(move |x| (x, z))).collect();
                spots.sort_by_key(|&(x, z)| x.abs() + z.abs());
                spots.into_iter().flat_map(|(x, z)| ways.map(|w| (x, z, w))).find(|&(x, z, w)| shore(&doc, x, z, w)).expect("a shore")
            }
        };
        eprintln!("shore at {x}, {z}, the sea toward {dx}, {dz}");
        let click = |doc: &mut Document, tools: &ToolState, at: (f32, f32)| {
            let (stroke, _) = begin_stroke(doc, tools, at, false, false);
            let mut stroke = stroke.unwrap();
            continue_stroke(doc, tools, &mut stroke, at, 0.02);
            end_stroke(doc, stroke);
        };
        // The forest: a disc of podzol whose east edge is by the shore.
        let mut tools = ToolState::default();
        tools.select(Tool::PaintTerrain);
        tools.terrain = id("Podzol");
        tools.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 60.0);
        tools.filter = massif_core::filter::Filter { only_on: Some(vec![massif_core::filter::Match::Terrain(grass)]), ..Default::default() };
        click(&mut doc, &tools, ((x - dx * 70) as f32, (z - dz * 70) as f32));
        tools.filter = massif_core::filter::Filter::default();
        tools.select(Tool::Gradient);
        tools.gradient.gradient = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
        tools.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Sine, 131.0);
        tools.brush.strength = 1.0;
        let sea = doc.world.meta.sea_level as f32;
        let (x0, z0) = (x - 192, z - 128);
        let render = |doc: &Document, name: &str| {
            let image = crate::map_render::render_region(doc.dimension(), &doc.world.registry, sea, crate::map_render::MapStyle::default(), 0, x0, z0, 3, 2);
            let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_srgba_unmultiplied()).collect();
            image::save_buffer(dir.join(name), &rgba, image.width() as u32, image.height() as u32, image::ColorType::Rgba8).unwrap();
        };
        let snapshot = |doc: &Document| -> Vec<(u16, bool)> { (z0..z0 + 256).flat_map(|zz| (x0..x0 + 384).map(move |xx| (xx, zz))).map(|(xx, zz)| probe(doc, xx, zz).map_or((0, false), |p| (p.terrain, wet(doc, xx, zz)))).collect() };
        render(&doc, "gradient-shore-before.png");
        let before = snapshot(&doc);
        let at = ((x - dx * 12) as f32, (z - dz * 12) as f32);
        click(&mut doc, &tools, at);
        render(&doc, "gradient-shore-1.png");
        let one = snapshot(&doc);
        click(&mut doc, &tools, at);
        render(&doc, "gradient-shore-2.png");
        let two = snapshot(&doc);
        // What the second click added, at the soft rim the first left
        // half painted, is what one click of a hard brush lays there: the
        // same positions, nothing further.
        let mut hard = Document::new(massif_format::load(std::path::Path::new(&path)).unwrap(), None, 1);
        {
            let mut t = tools.clone();
            t.select(Tool::PaintTerrain);
            t.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 60.0);
            t.filter = massif_core::filter::Filter { only_on: Some(vec![massif_core::filter::Match::Terrain(grass)]), ..Default::default() };
            click(&mut hard, &t, ((x - dx * 70) as f32, (z - dz * 70) as f32));
            t.filter = massif_core::filter::Filter::default();
            t.select(Tool::Gradient);
            t.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 131.0);
            click(&mut hard, &t, at);
        }
        let reference = snapshot(&hard);
        let filled = one.iter().zip(&two).zip(&reference).filter(|((a, b), _)| a != b);
        assert!(filled.clone().all(|((_, b), r)| b == r), "the rim filled in as a hard brush paints it");
        let changed = filled.count();
        let painted = before.iter().zip(&one).filter(|(a, b)| a != b).count();
        // Every column of a terrain the gradient does not start or end with
        // (the beach, the sea floor…) as it was.
        let ends = [id("Podzol"), grass];
        let others = before.iter().zip(&two).filter(|(a, _)| !ends.contains(&a.0));
        let moved = others.clone().filter(|(a, b)| a != b).count();
        let wet_painted = before.iter().zip(&two).filter(|(a, b)| a.1 && a != b).count();
        eprintln!("first click: {painted} columns painted; second: {changed} filled in at the rim; other terrains changed: {moved} of {}; under water painted: {wet_painted}", others.count());
        assert!(painted > 1000);
        assert_eq!(moved, 0, "the beach and the sea floor stay as they were");
        assert_eq!(wet_painted, 0, "nothing under the water is painted");
    }

    /// The Gradient brush dragged over the edge of a patch: the rim
    /// terrains around it, one undo, the map keeping the gradient.
    #[test]
    fn a_gradient_stroke_blends_the_edge_of_a_patch() {
        let mut doc = doc();
        let mut tools = ToolState::default();
        let reg = doc.world.registry.clone();
        let name = |doc: &Document, x: i32, z: i32| reg.terrains[probe(doc, x, z).unwrap().terrain as usize].name.clone();
        tools.select(Tool::PaintTerrain);
        tools.terrain = reg.terrain_by_name("Grass").unwrap().0;
        tools.brush = Brush::symmetric(massif_core::BrushShape::Square, massif_core::Profile::Constant, 60.0);
        let paint = |doc: &mut Document, tools: &ToolState, from: (f32, f32), to: (f32, f32)| {
            let (stroke, _) = begin_stroke(doc, tools, from, false, false);
            let mut stroke = stroke.unwrap();
            continue_stroke(doc, tools, &mut stroke, from, 0.02);
            continue_stroke(doc, tools, &mut stroke, to, 0.02);
            end_stroke(doc, stroke);
        };
        paint(&mut doc, &tools, (0.0, 0.0), (0.0, 0.0));
        tools.terrain = reg.terrain_by_name("Stone").unwrap().0;
        tools.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 12.0);
        paint(&mut doc, &tools, (0.0, 0.0), (0.0, 0.0));
        assert_eq!(name(&doc, 0, 0), "Stone");
        assert_eq!(name(&doc, 20, 0), "Grass");
        tools.select(Tool::Gradient);
        tools.gradient.gradient = massif_core::gradient::Gradient::new("rings", &[("Stone", 2.0), ("Andesite", 4.0), ("Moss", 4.0), ("Grass", 4.0)], &[massif_core::gradient::Transition::hard(); 3]);
        tools.gradient.gradient.edge = 2.0;
        tools.gradient.gradient.wander = 0.0;
        tools.brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 14.0);
        let undo = doc.history.undo_count();
        paint(&mut doc, &tools, (14.0, -8.0), (14.0, 8.0));
        assert_eq!(doc.history.undo_count(), undo + 1);
        assert_eq!(doc.history.undo_label(), Some("Gradient brush"));
        let row: Vec<String> = (0..26).map(|x| name(&doc, x, 0)).collect();
        assert_eq!(&row[..12], vec!["Stone"; 12].as_slice(), "{row:?}");
        assert_eq!(&row[12..16], vec!["Andesite"; 4].as_slice(), "{row:?}");
        assert_eq!(&row[16..20], vec!["Moss"; 4].as_slice(), "{row:?}");
        assert_eq!(row[20], "Grass");
        assert_eq!(doc.world.meta.gradients, [tools.gradient.gradient.clone()]);
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

    /// A combined layer paints its terrain, biome, cover and pockets in one
    /// stroke, one step to undo; its right button rubs them out and puts
    /// the map's own terrain back; from another level, its pockets still go
    /// into the rock of the ground.
    #[test]
    fn a_combined_layer_paints_its_slots_in_one_stroke() {
        use massif_core::combined::{CombinedDef, Part};
        let mut d = doc();
        let reg = &mut d.world.registry;
        let stone = reg.terrain_by_name("Stone").unwrap().0;
        let forest = reg.biome_by_id("minecraft:forest").unwrap().0;
        let grass = reg.cover_by_name("Grass").unwrap();
        let clay = reg.intern_block("minecraft:clay", [160, 166, 179]);
        let pockets = reg.add_pocket(massif_core::pockets::PocketDef::new("Clay", [0; 3], &[(clay, 1)], 0.3, 4.0, -60, 40));
        let mut def = CombinedDef::new("Beach", [0; 3]);
        def.set(Part::Terrain, Some((stone, 1.0)));
        def.set(Part::Biome, Some((forest, 1.0)));
        def.set(Part::Cover, Some((grass, 1.0)));
        def.set(Part::Pockets, Some((pockets, 1.0)));
        let mut brush = Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 10.0);
        brush.strength = 1.0;
        let mut tools = ToolState { combined: reg.add_combined(def), brush, ..ToolState::default() };
        tools.select(Tool::Combined);
        let paint = |d: &mut Document, tools: &ToolState, inverse: bool| {
            let (stroke, _) = begin_stroke(d, tools, (0.5, 0.5), inverse, inverse);
            let mut stroke = stroke.unwrap();
            continue_stroke(d, tools, &mut stroke, (0.5, 0.5), 0.05);
            end_stroke(d, stroke);
        };
        let at = |d: &Document, x: i32, z: i32| {
            let t = d.world.dims[0].ground().view(TilePos::containing(x, z)).unwrap();
            let i = massif_core::coords::local_index(x, z);
            (t.terrain.get(i), t.biome.get(i), t.cover.get(i), t.pockets.get(i))
        };
        let before = at(&d, 0, 0);
        let undo = d.history.undo_count();
        paint(&mut d, &tools, false);
        assert_eq!(at(&d, 0, 0), (stone, forest, grass, pockets));
        assert_eq!(at(&d, 30, 0), before, "nothing out of the brush");
        assert_eq!(d.history.undo_count(), undo + 1, "one step");
        // The right button: its cover, pockets and biome gone, the terrain
        // Paint terrain's right button puts back there.
        let mut reference = Document::new(d.world.clone(), None, 1);
        let mut terrain = ToolState::default();
        terrain.select(Tool::PaintTerrain);
        terrain.brush = tools.brush.clone();
        paint(&mut reference, &terrain, true);
        let back = at(&reference, 0, 0).0;
        assert_ne!(back, stone);
        paint(&mut d, &tools, true);
        assert_eq!(at(&d, 0, 0), (back, 0, 0, 0));
        // From a Solid level, the pockets still go into the ground's rock.
        let slab = d.world.dims[0].add_level("Island", massif_core::level::LevelKind::Solid);
        d.active_level = slab;
        paint(&mut d, &tools, false);
        assert_eq!(at(&d, 0, 0).3, pockets);
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
        tools.flood_height = WaterHeight::Fixed;
        tools.water_level = 70;
        let msg = click(&mut d, &mut tools, 0, 0, false).unwrap();
        assert!(msg.starts_with("Flooded 65536 columns"), "{msg}");
        assert_eq!(probe(&d, 100, 100).unwrap().fluid, 70);
        d.undo();
        assert_eq!(probe(&d, 100, 100).unwrap().fluid, NO_FLUID);
    }

    /// Flood on flat ground: there is no hollow, and it asks before
    /// spreading the water, whatever the height chosen; under a height
    /// limit it says how to fill it.
    #[test]
    fn flood_says_why_nothing_fills() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Flood);
        assert_eq!(tools.flood_height, WaterHeight::Start);
        // Not a hollow: it asks first, changing nothing.
        let steps = d.history.undo_count();
        assert_eq!(click(&mut d, &mut tools, 50, 50, false), None);
        let ask = tools.flood_ask.take().expect("a question");
        assert_eq!((ask.x, ask.z), (50, 50));
        assert!(ask.columns > 1000, "{ask:?}");
        assert_eq!(d.history.undo_count(), steps);
        assert_eq!(probe(&d, 50, 50).unwrap().fluid, NO_FLUID);
        // A level of its own under the ground, and no hollow the water
        // would run into: asked as well.
        tools.flood_height = WaterHeight::Fixed;
        tools.water_level = 40;
        assert_eq!(click(&mut d, &mut tools, 50, 50, false), None);
        assert!(tools.flood_ask.take().is_some_and(|ask| (ask.x, ask.z) == (50, 50)));
        assert_eq!(d.history.undo_count(), steps);
        // With a limit, never higher than it; under the spot, it says so.
        tools.water_level = 70;
        tools.water_limit_on = true;
        tools.water_limit = 66;
        let msg = click(&mut d, &mut tools, 50, 50, false).unwrap();
        assert!(msg.ends_with("up to 66"), "{msg}");
        assert_eq!(probe(&d, 60, 60).unwrap().fluid, 66);
        tools.water_limit = 60;
        // On that water, now over the limit: it stays, and says why.
        assert_eq!(click(&mut d, &mut tools, 120, 120, false).unwrap(), "Height limit y 60 reached");
        // On dry land under the limit, it says how to fill it.
        d.undo();
        let msg = click(&mut d, &mut tools, 120, 120, false).unwrap();
        assert!(msg.starts_with("The height limit 60 is under this spot"), "{msg}");
    }

    /// Flood from the click beyond a hollow: asked, Flood anyway fills as
    /// many columns as it said, a block over the spot, in one step to undo,
    /// and only inside the selection when there is one; with the warning
    /// off, the click floods at once.
    #[test]
    fn flood_beyond_a_hollow_when_asked() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Flood);
        assert!(tools.flood_warn);
        assert_eq!(click(&mut d, &mut tools, 50, 50, false), None);
        let ask = tools.flood_ask.take().unwrap();
        assert_eq!(ask.level as i32, block_top(height(&d, 50, 50)) + 1);
        let steps = d.history.undo_count();
        let msg = flood_anyway(&mut d, &ask);
        assert_eq!(msg, format!("Flooded {} columns up to {}", ask.columns, ask.level));
        assert_eq!(d.history.undo_count(), steps + 1);
        assert_eq!(probe(&d, 50, 50).unwrap().fluid, ask.level);
        d.undo();
        assert_eq!(probe(&d, 50, 50).unwrap().fluid, NO_FLUID);
        // Inside a selection only.
        let sel = massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(massif_core::coords::BlockRect::new(0, 0, 100, 100)));
        assert!(d.history.select(&mut d.world, 0, "Select", Some(sel)));
        assert_eq!(click(&mut d, &mut tools, 50, 50, false), None);
        let inside = tools.flood_ask.take().unwrap();
        assert_eq!(inside.columns, 100 * 100, "{inside:?}");
        flood_anyway(&mut d, &inside);
        assert_eq!(probe(&d, -50, -50).unwrap().fluid, NO_FLUID);
        assert_eq!(probe(&d, 99, 99).unwrap().fluid, inside.level);
        // Not asked: flooded at once.
        d.undo();
        tools.flood_warn = false;
        let msg = click(&mut d, &mut tools, 50, 50, false).unwrap();
        assert!(tools.flood_ask.is_none() && msg.starts_with("Flooded 10000 columns"), "{msg}");
    }

    /// A dry pit 30 blocks across and 14 deep at (60, 60) in flat land: its
    /// bottom and its rim, as fluid levels (water at the rim stays in it).
    fn pit(d: &mut Document) -> (i16, i16) {
        let brush = Brush { shape: massif_core::BrushShape::Circle, radius: 15.0, profile: massif_core::Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let rim = block_top(height(d, 60, 60)) as i16;
        d.history.begin("Dig", d.dim);
        ops::raise(&mut d.history.edit(&mut d.world, 0), &brush, 60.0, 60.0, -14.0, ops::Surface::Top);
        // The sea's level under the land would fill it: none.
        for pos in d.dimension().ground().positions().collect::<Vec<_>>() {
            d.history.edit(&mut d.world, 0).tile_mut(pos).unwrap().fluid.fill(NO_FLUID);
        }
        d.commit();
        (block_top(height(d, 60, 60)) as i16, rim)
    }

    fn water(d: &Document, x: i32, z: i32) -> Option<i16> {
        ops::water_at(d.dimension(), x, z)
    }

    /// Every column's water, to compare a map before and after.
    fn fluids(d: &Document) -> Vec<i16> {
        let ground = d.dimension().ground();
        let mut out = Vec::new();
        for pos in ground.positions().collect::<Vec<_>>() {
            let tile = ground.view(pos).unwrap();
            out.extend((0..massif_core::coords::TILE_AREA).map(|i| tile.fluid.get(i)));
        }
        out
    }

    /// Every way of choosing the water's height keeps to a selection: the
    /// Water brush across its edge, and Flood at the sea's level or a level
    /// set, lay water inside it only; undone and redone exactly.
    #[test]
    fn the_water_heights_keep_to_a_selection() {
        let mut d = doc();
        let mut tools = ToolState { flood_warn: false, ..ToolState::default() };
        tools.brush.radius = 20.0;
        tools.brush.strength = 1.0;
        let sel = massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(massif_core::coords::BlockRect::new(0, 0, 100, 100)));
        assert!(d.history.select(&mut d.world, 0, "Select", Some(sel)));
        d.world.meta.sea_level = 80;
        tools.water_level = 75;
        for tool in [Tool::Water, Tool::Flood] {
            for mode in WaterHeight::ALL {
                tools.select(tool);
                if tool == Tool::Water {
                    tools.brush_height = mode;
                } else {
                    tools.flood_height = mode;
                }
                let before = fluids(&d);
                if tool == Tool::Water {
                    let (stroke, msg) = begin_stroke(&mut d, &tools, (0.5, 50.5), false, false);
                    let mut stroke = stroke.unwrap_or_else(|| panic!("{mode:?}: {msg:?}"));
                    continue_stroke(&mut d, &tools, &mut stroke, (0.5, 50.5), 0.1);
                    end_stroke(&mut d, stroke);
                } else {
                    click(&mut d, &mut tools, 50, 50, false);
                }
                assert!(water(&d, 5, 50).is_some(), "{tool:?} {mode:?}: inside");
                assert_eq!(water(&d, -5, 50), None, "{tool:?} {mode:?}: outside");
                assert_eq!(water(&d, 50, -5), None, "{tool:?} {mode:?}: outside");
                let after = fluids(&d);
                d.undo();
                assert!(fluids(&d) == before, "{tool:?} {mode:?}: undone");
                d.redo();
                assert!(fluids(&d) == after, "{tool:?} {mode:?}: redone");
                d.undo();
            }
        }
    }

    /// The whole of a map, to compare it before and after: every level's
    /// tiles, the sculpted chunks, the selection, the registry and the
    /// settings.
    #[allow(clippy::type_complexity)]
    fn whole(d: &Document) -> (Vec<Vec<(TilePos, Vec<u8>)>>, String, Option<Vec<u8>>, String) {
        let dim = d.dimension();
        let levels = dim
            .levels
            .iter()
            .map(|l| {
                let mut tiles: Vec<(TilePos, Vec<u8>)> = l.positions().map(|p| (p, {
                    // Compacted: the same content, however its planes are kept.
                    let mut t = (*l.view(p).unwrap()).clone();
                    t.compact();
                    t.encode()
                })).collect();
                tiles.sort_by_key(|t| (t.0.x, t.0.z));
                tiles
            })
            .collect();
        (levels, format!("{:?}", dim.voxels), dim.selection.as_ref().map(|s| s.encode()), format!("{:?}{:?}", d.world.registry, d.world.meta))
    }

    /// Each tool that paints with the brush, from either button: one stroke
    /// is one step, undo puts the map back exactly as it was and redo makes
    /// it exactly as the stroke left it.
    #[test]
    fn every_stroke_undoes_and_redoes_exactly() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.brush.radius = 12.0;
        tools.brush.strength = 1.0;
        let mut done = Vec::new();
        for &tool in Tool::GROUPS.iter().flat_map(|g| g.2.iter()) {
            tools.select(tool);
            // Layers to paint with where the tool needs one.
            if tool == Tool::StampSlab || tool == Tool::EraseSlab {
                if d.active_kind() == massif_core::level::LevelKind::Ground {
                    d.add_level(massif_core::level::LevelKind::Solid);
                }
            } else {
                d.active_level = 0;
            }
            for inverse in [false, true] {
                let before = whole(&d);
                let steps = d.history.undo_count();
                let (stroke, _) = begin_stroke(&mut d, &tools, (20.5, 20.5), inverse, inverse);
                let Some(mut stroke) = stroke else { continue };
                for k in 0..6 {
                    continue_stroke(&mut d, &tools, &mut stroke, (20.5 + k as f32 * 3.0, 20.5), 0.05);
                }
                end_stroke(&mut d, stroke);
                if d.history.undo_count() == steps {
                    continue;
                }
                assert_eq!(d.history.undo_count(), steps + 1, "{tool:?} {inverse}: one step");
                let after = whole(&d);
                d.undo();
                assert!(whole(&d) == before, "{tool:?} {inverse}: undone exactly");
                d.redo();
                assert!(whole(&d) == after, "{tool:?} {inverse}: redone exactly");
                done.push(format!("{tool:?}{}", if inverse { " (right)" } else { "" }));
            }
        }
        eprintln!("undone and redone: {}", done.join(", "));
        assert!(done.len() > 20, "{done:?}");
    }

    /// Flood held at the map's edge, without the warning: the water spreads
    /// to the edge and rises no higher than the top of the world; held
    /// with the right button, it goes down to nothing. Each press is one
    /// step, undone and redone exactly; a click off the map does nothing.
    #[test]
    fn flood_held_at_the_map_edge_stops_at_the_top() {
        let mut d = doc();
        let mut tools = ToolState { flood_warn: false, ..ToolState::default() };
        tools.select(Tool::Flood);
        let top = d.dimension().max_y as i16;
        assert!(click(&mut d, &mut tools, 500, 500, false).is_some_and(|m| !m.is_empty()), "off the map: said, not done");
        click(&mut d, &mut tools, 127, 127, false).unwrap();
        assert!(water(&d, 127, 127).is_some() && water(&d, -128, -128) == water(&d, 127, 127), "the whole flat map");
        let before = fluids(&d);
        let steps = d.history.undo_count();
        let (hold, _) = press_water(&mut d, &mut tools, 127, 127, false, false);
        let mut hold = hold.unwrap();
        for _ in 0..1000 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert!(hold.ended(), "it stops");
        release_water(&mut d, hold);
        let level = water(&d, 127, 127).unwrap();
        assert_eq!(level, top, "up to the top of the world, no higher");
        let msg = click(&mut d, &mut tools, 127, 127, false).unwrap();
        assert_eq!(msg, "The water here is at the top of the world already");
        assert_eq!(d.history.undo_count(), steps + 1, "one step");
        let after = fluids(&d);
        d.undo();
        assert!(fluids(&d) == before, "undone whole");
        d.redo();
        assert!(fluids(&d) == after, "redone exactly");
        // Lowered while held: down to nothing, one step.
        let (hold, _) = press_water(&mut d, &mut tools, -128, 127, true, false);
        let mut hold = hold.unwrap();
        for _ in 0..1000 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert!(hold.ended());
        release_water(&mut d, hold);
        assert_eq!(water(&d, 0, 0), None);
        d.undo();
        assert!(fluids(&d) == after, "the lowering undone whole");
        d.redo();
        assert_eq!(water(&d, 127, 127), None);
    }

    /// Flood's right button lowers the water clicked by one block, the
    /// shore it uncovers left as the ground was; Shift drains it whole.
    #[test]
    fn a_right_click_lowers_the_water_a_block() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Flood);
        let (bottom, rim) = pit(&mut d);
        let msg = click(&mut d, &mut tools, 60, 60, false).unwrap();
        assert_eq!(water(&d, 60, 60), Some(rim), "{msg}");
        assert_eq!(water(&d, 60 + 20, 60), None);
        let (steps, ground) = (d.history.undo_count(), height(&d, 60, 60));
        let msg = click(&mut d, &mut tools, 60, 60, true).unwrap();
        assert_eq!(water(&d, 60, 60), Some(rim - 1), "{msg}");
        assert_eq!(water(&d, 70, 60), Some(rim - 1));
        assert_eq!(msg, format!("Water lowered by 1 block, to {}", rim - 1));
        assert_eq!(d.history.undo_count(), steps + 1);
        assert_eq!(height(&d, 60, 60), ground);
        // Held to the bottom: a block at a time, then gone.
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, true, false);
        let mut hold = hold.unwrap();
        let mut beats = 1;
        while !hold.ended() {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
            beats += 1;
            assert!(beats < 40);
        }
        assert_eq!(release_water(&mut d, hold).unwrap(), "The water is gone");
        assert_eq!(water(&d, 60, 60), None);
        assert_eq!(d.history.undo_count(), steps + 2);
        assert!(beats as i16 >= rim - 1 - bottom, "{beats}");
        d.undo();
        assert_eq!(water(&d, 60, 60), Some(rim - 1));
        // Shift + right click: drained whole, at once.
        assert_eq!(click(&mut d, &mut tools, 60, 60, true).unwrap(), format!("Water lowered by 1 block, to {}", rim - 2));
        let (hold, msg) = press_water(&mut d, &mut tools, 60, 60, true, true);
        assert!(hold.is_none() && msg.unwrap().starts_with("Drained"));
        for (x, z) in [(60, 60), (50, 60), (70, 70)] {
            assert_eq!(water(&d, x, z), None);
        }
        assert_eq!(click(&mut d, &mut tools, 60, 60, true).unwrap(), "No water here to lower");
    }

    /// Flood held on water raises it a block at each beat, all of it one
    /// step to undo, and stops at the rim of its hollow; a click there asks
    /// before it spills.
    #[test]
    fn holding_flood_raises_the_water_to_the_rim() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Flood);
        let (bottom, rim) = pit(&mut d);
        tools.flood_height = WaterHeight::Fixed;
        tools.water_level = bottom + 2;
        click(&mut d, &mut tools, 60, 60, false).unwrap();
        assert_eq!(water(&d, 60, 60), Some(bottom + 2));
        // On water, Fixed at the water's level: a click raises it a block,
        // the level set being for new fills only.
        let steps = d.history.undo_count();
        assert_eq!(click(&mut d, &mut tools, 60, 60, false).unwrap(), format!("Water raised by 1 block, to {}", bottom + 3));
        assert_eq!(d.history.undo_count(), steps + 1);
        d.undo();
        tools.flood_height = WaterHeight::Sea;
        assert_eq!(click(&mut d, &mut tools, 60, 60, false).unwrap(), format!("Water raised by 1 block, to {}", bottom + 3));
        d.undo();
        tools.flood_height = WaterHeight::Start;
        // Where I click: a click, a block.
        assert_eq!(click(&mut d, &mut tools, 60, 60, false).unwrap(), format!("Water raised by 1 block, to {}", bottom + 3));
        // Held for 5 beats: 5 blocks, one step.
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, false, false);
        let mut hold = hold.unwrap();
        for _ in 1..5 {
            assert!(hold_water(&mut d, &tools, &mut hold, hold_period(&tools)));
        }
        // Between beats, nothing.
        assert!(!hold_water(&mut d, &tools, &mut hold, hold_period(&tools) * 0.5));
        assert_eq!(hold.blocks(), 5);
        assert_eq!(water(&d, 60, 60), Some(bottom + 8));
        release_water(&mut d, hold);
        assert_eq!(d.history.undo_count(), steps + 2);
        d.undo();
        assert_eq!(water(&d, 60, 60), Some(bottom + 3));
        // Held long: up to the rim, no higher.
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, false, false);
        let mut hold = hold.unwrap();
        for _ in 0..60 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert!(hold.ended());
        let msg = release_water(&mut d, hold).unwrap();
        assert!(msg.contains("rim"), "{msg}");
        assert_eq!(water(&d, 60, 60), Some(rim));
        assert_eq!(water(&d, 60 + 20, 60), None);
        // A click at the rim asks first; Flood anyway raises it a block.
        assert_eq!(click(&mut d, &mut tools, 60, 60, false), None);
        let ask = tools.flood_ask.take().expect("a question");
        assert!(ask.at_rim && ask.level == rim + 1, "{ask:?}");
        assert_eq!(water(&d, 60, 60), Some(rim));
        flood_anyway(&mut d, &ask);
        assert_eq!(water(&d, 60, 60), Some(rim + 1));
        d.undo();
        // Without the warning, held, it rises on past the rim; the height
        // limit still holds.
        tools.flood_warn = false;
        tools.water_limit_on = true;
        tools.water_limit = rim + 3;
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, false, false);
        let mut hold = hold.unwrap();
        for _ in 0..10 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert!(hold.ended() && hold.blocks() == 3);
        assert_eq!(release_water(&mut d, hold).unwrap(), format!("Water raised by 3 blocks, to {}: height limit reached", rim + 3));
        assert_eq!(water(&d, 60, 60), Some(rim + 3));
        assert_eq!(water(&d, 60 + 40, 60), Some(rim + 3));
        assert_eq!(click(&mut d, &mut tools, 60, 60, false).unwrap(), format!("Height limit y {} reached", rim + 3));
        // Fixed, held: the same, a block a beat up to the limit.
        tools.flood_height = WaterHeight::Fixed;
        tools.water_limit = rim + 5;
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, false, false);
        let mut hold = hold.unwrap();
        for _ in 0..10 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert_eq!(hold.blocks(), 2);
        release_water(&mut d, hold);
        assert_eq!(water(&d, 60, 60), Some(rim + 5));
    }

    /// A flood that would change more columns than the tools' limit changes
    /// none: stopped halfway, the water it laid would stand as a wall where
    /// it stopped. Flooded, raised, lowered or drained past the limit, the
    /// water stays as it was, no step is added, and the tool says why.
    #[test]
    fn a_flood_past_the_limit_changes_nothing_and_says_why() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Flood);
        // The flat map is 65,536 columns.
        FLOOD_LIMIT.with(|l| l.set(Some(5000)));
        let unchanged = |d: &Document, before: &[i16], steps: usize, what: &str| {
            assert!(fluids(d) == before, "{what}: the water as it was");
            assert_eq!(d.history.undo_count(), steps, "{what}: no step");
        };
        let (before, steps) = (fluids(&d), d.history.undo_count());
        // On flat land, water made to stay would cover the whole map: said
        // instead of asked, and said when not asked.
        let msg = click(&mut d, &mut tools, 50, 50, false).unwrap();
        assert_eq!(msg, "Too much water for one flood: it would cover more than 5,000 columns. Nothing was flooded.");
        assert!(tools.flood_ask.is_none());
        unchanged(&d, &before, steps, "asked");
        tools.flood_warn = false;
        assert_eq!(click(&mut d, &mut tools, 50, 50, false).unwrap(), msg);
        unchanged(&d, &before, steps, "not asked");
        // A pit of some 700 columns, within the limit, fills to its rim.
        let (_, rim) = pit(&mut d);
        assert!(click(&mut d, &mut tools, 60, 60, false).unwrap().starts_with("Flooded "));
        assert_eq!(water(&d, 60, 60), Some(rim));
        let (before, steps) = (fluids(&d), d.history.undo_count());
        // A block higher, it would spread over the whole map.
        let msg = click(&mut d, &mut tools, 60, 60, false).unwrap();
        assert_eq!(msg, format!("The water stays at {rim}: a block higher, it would spread over more than 5,000 columns, too much for one flood."));
        unchanged(&d, &before, steps, "raised");
        tools.flood_warn = true;
        assert_eq!(click(&mut d, &mut tools, 60, 60, false).unwrap(), msg);
        assert!(tools.flood_ask.is_none());
        unchanged(&d, &before, steps, "raised, asked");
        // Held, the same.
        tools.flood_warn = false;
        let (hold, _) = press_water(&mut d, &mut tools, 60, 60, false, false);
        let mut hold = hold.unwrap();
        for _ in 0..10 {
            hold_water(&mut d, &tools, &mut hold, hold_period(&tools));
        }
        assert!(hold.ended());
        assert_eq!(release_water(&mut d, hold).unwrap(), msg);
        unchanged(&d, &before, steps, "held");
        // Under a limit smaller than the pit's water: neither lowered nor
        // drained.
        FLOOD_LIMIT.with(|l| l.set(Some(100)));
        assert_eq!(click(&mut d, &mut tools, 60, 60, true).unwrap(), "Too much water to lower at once: it covers more than 100 columns.");
        unchanged(&d, &before, steps, "lowered");
        let (hold, msg) = press_water(&mut d, &mut tools, 60, 60, true, true);
        assert!(hold.is_none());
        assert_eq!(msg.unwrap(), "Too much water to drain at once: it covers more than 100 columns. Nothing was drained.");
        unchanged(&d, &before, steps, "drained");
        FLOOD_LIMIT.with(|l| l.set(None));
    }

    /// By the cursor, the water tools say the level they fill to, or why
    /// the ground stays dry: a level under it, or the height limit.
    #[test]
    fn the_water_tools_say_what_they_would_fill() {
        let mut d = doc();
        let mut tools = ToolState::default();
        let (bottom, rim) = pit(&mut d);
        let ground = block_top(height(&d, 120, 120));
        tools.select(Tool::Water);
        let hint = |d: &Document, tools: &ToolState, x, z| water_hint(d, tools, x, z, None).unwrap();
        assert_eq!(hint(&d, &tools, 120, 120), format!("Fills to y {}", ground + 1));
        assert_eq!(hint(&d, &tools, 60, 60), format!("Fills to y {}", bottom + 1));
        tools.brush_height = WaterHeight::Fixed;
        tools.water_level = ground as i16 - 2;
        assert_eq!(hint(&d, &tools, 120, 120), format!("Dry here: the ground is at or above y {}.", ground - 2));
        assert_eq!(hint(&d, &tools, 60, 60), format!("Fills to y {}", ground - 2));
        tools.brush_height = WaterHeight::Sea;
        assert_eq!(hint(&d, &tools, 60, 60), format!("Fills to y {}", d.world.meta.sea_level));
        // A stroke under way: its own level.
        assert_eq!(water_hint(&d, &tools, 120, 120, Some(bottom + 3)).unwrap(), format!("Dry here: the ground is at or above y {}.", bottom + 3));
        // The limit, under the ground.
        tools.brush_height = WaterHeight::Start;
        tools.water_limit_on = true;
        tools.water_limit = bottom - 1;
        assert_eq!(hint(&d, &tools, 60, 60), format!("Dry here: the height limit, y {}, is under the ground.", bottom - 1));
        tools.water_limit_on = false;
        // Flood, where I click: the hollow up to its rim; on flat ground the
        // nearest hollow, 64 blocks around at most; further, it spreads.
        tools.select(Tool::Flood);
        assert_eq!(hint(&d, &tools, 60, 60), format!("Fill to y {rim}"));
        assert_eq!(hint(&d, &tools, 120, 120), format!("No hollow here: fills the nearest one, to y {rim}"));
        assert_eq!(hint(&d, &tools, -100, -100), "Not in a hollow: the water would spread");
        tools.flood_height = WaterHeight::Fixed;
        tools.water_level = bottom + 2;
        assert_eq!(hint(&d, &tools, 60, 60), format!("Fill to y {}", bottom + 2));
        click(&mut d, &mut tools, 60, 60, false).unwrap();
        // On water, whatever the height chosen: a block higher.
        assert_eq!(hint(&d, &tools, 60, 60), format!("Raise this water to y {}", bottom + 3));
        tools.flood_height = WaterHeight::Start;
        assert_eq!(hint(&d, &tools, 60, 60), format!("Raise this water to y {}", bottom + 3));
        tools.water_limit_on = true;
        tools.water_limit = bottom + 2;
        assert_eq!(hint(&d, &tools, 60, 60), format!("Height limit y {} reached", bottom + 2));
        tools.water_limit_on = false;
        click(&mut d, &mut tools, 70, 60, true).unwrap();
        assert_eq!(hint(&d, &tools, 60, 60), format!("Raise this water to y {}", bottom + 2));
        d.history.begin("fill", d.dim);
        ops::flood_fill(&mut d.history.edit(&mut d.world, 0), 60, 60, Flood::Fill(rim), FLOOD_MAX).unwrap();
        d.commit();
        assert_eq!(hint(&d, &tools, 60, 60), format!("At the rim, y {rim}: a block more spills"));
    }

    /// The Water brush lays its water as high as where its stroke starts,
    /// at any height: a block over the ground there, or the surface of the
    /// water there; at the level set with From the click off; and never
    /// over the limit when there is one, saying so when the limit is under
    /// the spot.
    #[test]
    fn the_water_brush_lays_water_where_it_starts() {
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.select(Tool::Water);
        let paint = |d: &mut Document, tools: &ToolState, at: (f32, f32)| {
            let (stroke, msg) = begin_stroke(d, tools, at, false, false);
            let Some(mut stroke) = stroke else { return msg };
            continue_stroke(d, tools, &mut stroke, at, 0.1);
            end_stroke(d, stroke);
            None
        };
        let ground = block_top(probe(&d, 50, 50).unwrap().height);
        assert!(paint(&mut d, &tools, (50.5, 50.5)).is_none());
        assert_eq!(probe(&d, 50, 50).unwrap().fluid as i32, ground + 1);
        // Started in that water: its surface.
        assert!(paint(&mut d, &tools, (52.5, 50.5)).is_none());
        assert_eq!(probe(&d, 53, 50).unwrap().fluid as i32, ground + 1);
        tools.brush_height = WaterHeight::Fixed;
        tools.water_level = 90;
        assert!(paint(&mut d, &tools, (100.5, 100.5)).is_none());
        assert_eq!(probe(&d, 100, 100).unwrap().fluid, 90);
        tools.brush_height = WaterHeight::Start;
        tools.water_limit_on = true;
        tools.water_limit = 60;
        let msg = paint(&mut d, &tools, (-60.5, -60.5)).expect("it says why");
        assert!(msg.starts_with("The height limit 60 is under this spot"), "{msg}");
        assert_eq!(probe(&d, -61, -61).unwrap().fluid, NO_FLUID);
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
        tools.flood_height = WaterHeight::Fixed;
        tools.water_level = 70;
        click(&mut d, &mut tools, 50, 50, false);
        assert_eq!(probe(&d, 60, 60).unwrap().fluid, 70);
        // The right button: a block lower; with Shift, drained whole.
        let msg = click(&mut d, &mut tools, 50, 50, true).unwrap();
        assert_eq!(msg, "Water lowered by 1 block, to 69");
        assert_eq!(probe(&d, 60, 60).unwrap().fluid, 69);
        let (_, msg) = press_water(&mut d, &mut tools, 50, 50, true, true);
        assert!(msg.unwrap().starts_with("Drained"));
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

    /// A map holding sculpted chunks with nothing of their own (3D strokes
    /// made them over the whole reach of their brush): opened, its check
    /// lets them go and says so, and a brush on the map works there, where
    /// each chunk kept its 16³ cube as it was.
    #[test]
    fn a_map_opened_lets_idle_chunks_go() {
        use massif_core::voxel::{ChunkPos, densities_at};
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Flat, ..GenParams::default() });
        for cy in [3, 4] {
            let pos = ChunkPos::new(1, cy, 1);
            let base = densities_at(&world.dims[0], pos);
            world.dims[0].voxels.insert(pos, std::sync::Arc::new(base));
        }
        let mut d = Document::new(world, None, 1);
        assert!(d.dimension().voxels.is_empty(), "let go");
        assert!(d.take_checked().is_some_and(|m| m.starts_with("2 sculpted blocks")), "and said");
        assert!(d.take_checked().is_none(), "once");
        let tools = ToolState { tool: Tool::Raise, ..ToolState::default() };
        let (stroke, _) = begin_stroke(&mut d, &tools, (24.5, 24.5), false, false);
        let mut stroke = stroke.unwrap();
        continue_stroke(&mut d, &tools, &mut stroke, (24.5, 24.5), 0.1);
        end_stroke(&mut d, stroke);
        assert!(height(&d, 24, 24) > 64.5, "raised there");
    }

    /// The map a user got back after a crash of 1.0's betas
    /// (MASSIF_RECOVERED_WORLD, a .massif): smoothed in 3D for minutes, it
    /// held some 11,000 sculpted chunks, and flattened whole on the map it
    /// kept two islands of the old land where no brush did anything. Its
    /// check lets most of them go, and a brush wider than the map sets every
    /// column to its height.
    #[test]
    fn a_recovered_map_flattens_everywhere() {
        let Some(path) = std::env::var_os("MASSIF_RECOVERED_WORLD") else { return };
        let world = massif_format::load(std::path::Path::new(&path)).unwrap();
        let chunks = world.dims[0].voxels.len();
        let mut d = Document::new(world, None, 1);
        let kept = d.dimension().voxels.len();
        eprintln!("{chunks} sculpted chunks, {kept} kept by the check: {:?}", d.take_checked());
        assert!(kept < chunks);
        let mut tools = ToolState { tool: Tool::SetHeight, target_height: 50.0, ..ToolState::default() };
        tools.brush.radius = 900.0;
        tools.brush.strength = 1.0;
        tools.brush.profile = massif_core::Profile::Constant;
        let started = std::time::Instant::now();
        let (stroke, _) = begin_stroke(&mut d, &tools, (0.5, 0.5), false, false);
        let mut stroke = stroke.unwrap();
        for _ in 0..20 {
            continue_stroke(&mut d, &tools, &mut stroke, (0.5, 0.5), 0.1);
        }
        end_stroke(&mut d, stroke);
        eprintln!("flattened in {:.1} s, {} sculpted chunks left", started.elapsed().as_secs_f32(), d.dimension().voxels.len());
        let (lo, hi) = d.dimension().bounds().unwrap();
        let t = massif_core::coords::TILE_SIZE;
        let mut high = Vec::new();
        for z in (lo.z * t..(hi.z + 1) * t).step_by(4) {
            for x in (lo.x * t..(hi.x + 1) * t).step_by(4) {
                let stack = massif_core::TileStack::new(d.dimension(), TilePos::containing(x, z), false).unwrap();
                let mut spans = Vec::new();
                stack.spans(massif_core::coords::local_index(x, z), &mut spans);
                let top = spans.iter().map(|s| s.y1).max().unwrap_or(i32::MIN);
                if top > 52 {
                    high.push((x, z, top));
                }
            }
        }
        assert!(high.is_empty(), "{} columns above the height set, as {:?}", high.len(), &high[..high.len().min(8)]);
    }

    /// A user's overhang, built in 3D Sculpt, went whole at one light click
    /// of Raise beside it on the map: it stays, as Raise and Lower click
    /// lightly beside it, and as Smooth goes over it on the map and in 3D
    /// Sculpt, the land moving under it.
    #[test]
    fn an_overhang_stays_under_the_tools_of_the_map() {
        let mut d = doc();
        // A hill, and a ledge over its side: 3 blocks of sculpted matter
        // 6 over the highest ground under it.
        let hill = ToolState { tool: Tool::Raise, brush: Brush { radius: 24.0, strength: 1.0, ..Brush::default() }, ..ToolState::default() };
        let (s, _) = begin_stroke(&mut d, &hill, (40.5, 40.5), false, false);
        let mut s = s.unwrap();
        for _ in 0..6 {
            continue_stroke(&mut d, &hill, &mut s, (40.5, 40.5), 0.1);
        }
        end_stroke(&mut d, s);
        let ledge: Vec<(i32, i32)> = (48..56).flat_map(|x| (36..44).map(move |z| (x, z))).collect();
        let bottom = ledge.iter().map(|&(x, z)| block_top(height(&d, x, z))).max().unwrap() + 6;
        assert!(bottom > 72, "a hill: {bottom}");
        d.history.begin("Build", d.dim);
        {
            let mut e = d.history.edit(&mut d.world, 0);
            for &(x, z) in &ledge {
                for y in bottom..bottom + 3 {
                    let pos = massif_core::voxel::ChunkPos::containing(x, y, z);
                    let (x0, y0, z0) = pos.origin();
                    e.voxel_chunk_mut(pos).set(x - x0, y - y0, z - z0, 255);
                }
            }
        }
        d.commit();
        // The columns of the ledge with it whole: 3 blocks over air.
        let whole = |d: &Document| {
            ledge
                .iter()
                .filter(|&&(x, z)| {
                    let stack = massif_core::TileStack::new(d.dimension(), TilePos::containing(x, z), false).unwrap();
                    let mut spans = Vec::new();
                    stack.spans(massif_core::coords::local_index(x, z), &mut spans);
                    spans.len() == 2 && spans[1].y1 - spans[1].y0 == 3
                })
                .count()
        };
        assert_eq!(whole(&d), ledge.len());
        // The map's own ground (what the brushes move), under the ledge.
        let ground = |d: &Document| ledge.iter().map(|&(x, z)| ops::height_at(d.dimension(), 0, x, z, Surface::Top).unwrap()).collect::<Vec<f32>>();
        // A light click of Raise beside it, then of Lower (Raise's other
        // button), then Smooth over it.
        for (tool, inverse, strength, at) in [(Tool::Raise, false, 0.1, (50.5, 47.5)), (Tool::Raise, true, 0.1, (50.5, 47.5)), (Tool::Smooth, false, 1.0, (52.5, 40.5))] {
            let tools = ToolState { tool, brush: Brush { radius: 12.0, strength, ..Brush::default() }, ..ToolState::default() };
            let was = ground(&d);
            let (s, _) = begin_stroke(&mut d, &tools, at, inverse, false);
            let mut s = s.unwrap();
            for _ in 0..4 {
                continue_stroke(&mut d, &tools, &mut s, at, 1.0 / 60.0);
            }
            end_stroke(&mut d, s);
            assert_ne!(ground(&d), was, "{tool:?} moved the land under the ledge");
            assert_eq!(whole(&d), ledge.len(), "{tool:?} {inverse}: the ledge stays whole");
        }
        // Smooth in 3D Sculpt is the map's: the same.
        let mut tools = ToolState { tool: Tool::Smooth, brush: Brush { radius: 12.0, strength: 1.0, ..Brush::default() }, ..ToolState::default() };
        let frame = massif_core::voxel::Frame { center: [52.5, height(&d, 52, 40), 40.5], normal: [0.0, 1.0, 0.0] };
        let was = ground(&d);
        let (s, _) = begin_sculpt(&mut d, &mut tools, false, &frame);
        let mut s = s.unwrap();
        for _ in 0..6 {
            sculpt_dab(&mut d, &tools, &mut s, &frame, 0.05);
        }
        end_sculpt(&mut d, s);
        assert_ne!(ground(&d), was, "smoothed in 3D");
        assert_eq!(whole(&d), ledge.len(), "Smooth in 3D: the ledge stays whole");
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
        tools.brush = Brush::default();
        let frame = massif_core::voxel::Frame {
            center: [5.5, 64.0, 5.5],
            normal: [0.0, 1.0, 0.0],
        };
        let mut s = begin_sculpt(&mut d, &mut tools, true, &frame).0.unwrap();
        let n = sculpt_dab(&mut d, &tools, &mut s, &frame, 0.1);
        end_sculpt(&mut d, s);
        assert!(n > 0);
        assert!(height(&d, 5, 5) > 64.0);
    }

    /// One Smooth stroke over the same place, in 3D Sculpt and on the map,
    /// leaves the same heights: Smooth in 3D is the map's (its kernel,
    /// strength and passes), the brush's shape only shaping its falloff,
    /// laid flat whatever the slope the view points at.
    #[test]
    fn smooth_in_3d_is_smooth_on_the_map() {
        let hilly = || Document::new(generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Mountains, ..GenParams::default() }), None, 1);
        let mut tools = ToolState { tool: Tool::Smooth, ..ToolState::default() };
        tools.brush.radius = 14.0;
        tools.brush.strength = 1.0;
        // A slope near the middle of the map.
        let (lo, _) = hilly().dimension().bounds().unwrap();
        let (cx, cz) = (lo.x * 128 + 100, lo.z * 128 + 100);
        let at = (cx as f32 + 0.5, cz as f32 + 0.5);
        // On the map, as its view strokes: a dab where it begins, then one a
        // frame.
        let mut map = hilly();
        let (s, _) = begin_stroke(&mut map, &tools, at, false, false);
        let mut s = s.unwrap();
        continue_stroke(&mut map, &tools, &mut s, at, 1.0 / 60.0);
        for _ in 0..10 {
            continue_stroke(&mut map, &tools, &mut s, at, 0.05);
        }
        end_stroke(&mut map, s);
        // In 3D Sculpt, the view pointing at the same column on a slope.
        let mut view = hilly();
        let frame = massif_core::voxel::Frame { center: [at.0, height(&view, cx, cz), at.1], normal: [0.4, 0.8, 0.2] };
        let (s, _) = begin_sculpt(&mut view, &mut tools, false, &frame);
        let mut s = s.unwrap();
        for _ in 0..10 {
            sculpt_dab(&mut view, &tools, &mut s, &frame, 0.05);
        }
        end_sculpt(&mut view, s);
        let first = hilly();
        let (mut changed, mut differ) = (0, 0);
        for z in cz - 30..cz + 31 {
            for x in cx - 30..cx + 31 {
                let h = height(&view, x, z);
                changed += (h != height(&first, x, z)) as usize;
                differ += (h != height(&map, x, z)) as usize;
            }
        }
        assert!(changed > 300, "smoothed: {changed} columns");
        assert_eq!(differ, 0, "the same heights as on the map");
        assert!(view.dimension().voxels.is_empty(), "nothing sculpted");
        assert_eq!(view.history.undo_label(), Some("Smooth"));
    }

    /// Settings of a version whose 3D Sculpt editor had its own Sculpt,
    /// Smooth and Paint: they read, that tool is let go, and a 3D tool the
    /// Tools panel does not show becomes the one it does.
    #[test]
    fn the_old_3d_sculpt_tool_is_let_go() {
        let old = r#"{"tool": "PaintBiome", "sculpt": {"tool": "Smooth3d", "fly_speed": 2.5}}"#;
        let mut tools: ToolState = serde_json::from_str(old).unwrap();
        tools.settle();
        assert_eq!((tools.tool, tools.sculpt.fly_speed), (Tool::PaintBiome, 2.5));
        assert!(!serde_json::to_string(&tools).unwrap().contains("Smooth3d"), "no longer saved");
        for (saved, now) in [("Build", Tool::Raise), ("Dig", Tool::Raise), ("Smooth3d", Tool::Smooth), ("Gradient", Tool::Gradient)] {
            let mut tools: ToolState = serde_json::from_str(&format!(r#"{{"tool": "{saved}", "sculpt": {{"tool": "PaintTerrain"}}}}"#)).unwrap();
            tools.settle();
            assert_eq!(tools.tool, now, "{saved}");
        }
    }

    /// Settings of 0.3, whose 3D Sculpt editor had a brush of its own
    /// (`sculpt.brush`, `sculpt.source`): they read, that brush is let go,
    /// and 3D Sculpt shapes with the brush of every tool, at its size and
    /// intensity.
    #[test]
    fn the_old_3d_brush_gives_way_to_the_shared_one() {
        let mut shared = ToolState::default();
        shared.brush.radius = 30.0;
        shared.brush.strength = 0.4;
        let mut json = serde_json::to_value(&shared).unwrap();
        let own = Brush { radius: 12.0, strength: 0.9, ..shared.brush.clone() };
        json["sculpt"] = serde_json::json!({"tool": "Build", "brush": own, "source": json["brush_source"].clone(), "fly_speed": 2.0});
        let mut tools: ToolState = serde_json::from_value(json).unwrap();
        tools.settle();
        assert_eq!((tools.brush.radius, tools.brush.strength, tools.sculpt.fly_speed), (30.0, 0.4, 2.0));
        let b = tools.brush_3d();
        assert_eq!((b.radius, b.strength), (30.0, 0.4), "3D Sculpt shapes with the shared brush");
        let saved = serde_json::to_value(&tools).unwrap();
        assert!(saved["sculpt"].get("brush").is_none() && saved["sculpt"].get("source").is_none(), "no longer saved");
    }

    /// Each tool of the Tools panel says what it does in 3D Sculpt, and
    /// those that act there do, where the view points.
    #[test]
    fn the_tools_of_the_panel_act_in_3d() {
        use massif_core::voxel::Frame;
        for (_, _, list) in Tool::GROUPS {
            for &tool in list {
                // What draws on the map only: its selections, points, marks.
                let map_only = matches!(tool, Tool::Pan | Tool::Road | Tool::River | Tool::Annotate) || tool.is_select();
                assert_eq!(tool.in_3d() == Use3d::MapOnly, map_only, "{tool:?}");
            }
        }
        let at = |d: &Document| Frame { center: [20.5, height(d, 20, 20), 20.5], normal: [0.0, 1.0, 0.0] };
        let stroke = |d: &mut Document, tools: &mut ToolState, inverse: bool| {
            let frame = at(d);
            let (s, _) = begin_sculpt(d, tools, inverse, &frame);
            let mut s = s.expect("a stroke");
            for _ in 0..5 {
                sculpt_dab(d, tools, &mut s, &frame, 0.05);
            }
            end_sculpt(d, s)
        };
        // Raise adds matter, its inverse digs.
        let mut d = doc();
        let mut tools = ToolState::default();
        tools.brush.radius = 5.0;
        tools.brush.strength = 1.0;
        stroke(&mut d, &mut tools, false);
        assert!(height(&d, 20, 20) > 64.0 && !d.dimension().voxels.is_empty(), "raised in 3D");
        let mut d = doc();
        stroke(&mut d, &mut tools, true);
        assert!(height(&d, 20, 20) < 64.0, "dug in 3D");
        // A biome painted by the map's own brush, where the view points.
        let mut d = doc();
        tools.select(Tool::PaintBiome);
        tools.biome = 7;
        stroke(&mut d, &mut tools, false);
        assert_eq!(probe(&d, 20, 20).unwrap().biome, 7);
        assert_eq!(d.history.undo_label(), Some(crate::i18n::tr_name(Tool::PaintBiome.label())));
        // The eyedropper picks there, once.
        let mut d = doc();
        tools.select(Tool::Eyedropper);
        let frame = at(&d);
        tools.target_height = 0.0;
        let (s, note) = begin_sculpt(&mut d, &mut tools, false, &frame);
        assert!(note.is_some() && tools.target_height == 64.0, "picked the height: {note:?}");
        assert_eq!(tools.tool, Tool::PaintBiome, "back to the tool before");
        assert!(end_sculpt(&mut d, s.unwrap()).is_none());
        // A selection shape does nothing in 3D.
        tools.select(Tool::SelectRect);
        assert!(begin_sculpt(&mut d, &mut tools, false, &frame).0.is_none());
        assert!(d.history.undo_label().is_none());
    }
}


#[cfg(test)]
mod auto_biome_tests {
    use super::*;
    use massif_core::auto::{Classes, Reader};
    use massif_core::brush::BrushMask;
    use massif_core::generate::{GenParams, generate};
    use massif_core::ops::Preview;

    fn image_brush(radius: f32, rotation: f32) -> Brush {
        let mut brush = ToolState::default().brush;
        brush.mask = Some(std::sync::Arc::new(BrushMask::new("Square", 8, 8, vec![255; 64])));
        brush.radius = radius;
        brush.rotation = rotation;
        brush.strength = 1.0;
        brush
    }

    /// An image brush turned 45° reaches past its radius, into its corners:
    /// the preview the map shows works them out too, as the stroke does.
    /// It used to work out only the tiles within the radius, and showed the
    /// corners beyond as if they had what the land calls for already (728
    /// of 40,044 columns at the middle of the map).
    #[test]
    fn the_preview_of_auto_biome_reaches_the_corners_of_a_turned_image_brush() {
        let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, ..GenParams::default() });
        let d = Document::new(world, None, 1);
        let tools = ToolState { tool: Tool::AutoBiome, ..ToolState::default() };
        let reader = Reader::new(&tools.auto, &d.world.registry, d.world.meta.sea_level);
        let brush = image_brush(100.0, 45.0);
        for (cx, cz) in [(0.0, 0.0), (-400.0, 350.0), (-64.0, 10.0)] {
            let mut classes = Classes::new(true);
            let (_, cols) = ops::auto_preview_ensured(d.dimension(), &brush, cx, cz, &reader, &mut classes, 1);
            let (_, whole) = ops::auto_preview(d.dimension(), &brush, cx, cz, &reader, true);
            assert_eq!(cols, whole, "at {cx}, {cz}");
            assert!(cols.iter().filter(|c| c.becomes().is_some()).count() > 39_000, "at {cx}, {cz}");
        }
    }

    /// Where a dab of Auto biome has painted, the land has what it calls
    /// for: the preview of the next dab beside it leaves that part as it
    /// is, and changes only the rest. That is the brush "painting half of
    /// its outline": the half it leaves was done already.
    #[test]
    fn auto_biome_leaves_what_it_painted_already() {
        let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, ..GenParams::default() });
        let mut d = Document::new(world, None, 1);
        let mut tools = ToolState { tool: Tool::AutoBiome, ..ToolState::default() };
        tools.brush = image_brush(40.0, 30.0);
        let reader = Reader::new(&tools.auto, &d.world.registry, d.world.meta.sea_level);
        let (a, b) = ((0.0f32, 0.0f32), (45.0f32, 0.0f32));
        let before = ops::auto_preview(d.dimension(), &tools.brush, b.0, b.1, &reader, true).1;
        assert!(before.iter().all(|c| *c != Preview::Same), "a new map: every column under the brush changes");
        let (stroke, _) = begin_stroke(&mut d, &tools, a, false, false);
        let mut stroke = stroke.unwrap();
        continue_stroke(&mut d, &tools, &mut stroke, a, 0.05);
        assert!(end_stroke(&mut d, stroke).is_some());
        let (rect, after) = ops::auto_preview(d.dimension(), &tools.brush, b.0, b.1, &reader, true);
        let dab = |x: i32, z: i32| tools.brush.covers(x as f32 + 0.5 - a.0, z as f32 + 0.5 - a.1);
        let (mut kept, mut changed) = (0, 0);
        for (k, c) in after.iter().enumerate() {
            let (x, z) = (rect.x0 + k as i32 % rect.width(), rect.z0 + k as i32 / rect.width());
            match c {
                Preview::Same => {
                    assert!(dab(x, z), "{x}, {z} is left as it is, though the first dab did not reach it");
                    kept += 1;
                }
                Preview::Becomes(_) => {
                    assert!(!dab(x, z), "{x}, {z} was painted already");
                    changed += 1;
                }
                Preview::Outside => {}
            }
        }
        assert!(kept > 500 && changed > 500, "kept {kept}, changed {changed}");
    }

}

/// The value of the slabs plane the Smoothing brush lays, as `tools`
/// asks: the slab chosen by hand registered in `reg` first.
pub fn slab_value(tools: &ToolState, reg: &mut massif_core::Registry) -> u16 {
    if tools.smooth_off {
        return massif_core::shapes::OFF;
    }
    let block = tools.slab_block.as_deref().map(|state| reg.intern_block(state, massif_core::objects::guess_color(state)));
    massif_core::shapes::value(block, true)
}
