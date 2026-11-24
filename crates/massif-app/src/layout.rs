//! Window layout: workspaces made of areas.
//!
//! A workspace is a binary tree. Leaves are *areas*, each showing one editor;
//! inner nodes split their rectangle in two, side by side or stacked. Areas
//! can be split, closed, swapped and dropped onto each other, which is all it
//! takes to build any arrangement.

use crate::theme::Icon;
use crate::map_render::MapMode;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// What an area shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EditorKind {
    Empty,
    Map,
    View3d,
    /// The world in 3D with the tools to build and dig in it.
    Sculpt3d,
    Tools,
    ToolSettings,
    BrushSettings,
    Brushes,
    CustomBrushes,
    Terrain,
    /// Every real block of the game, to paint.
    Blocks,
    Biomes,
    /// Ground covers, object layers and terrain mixes, as WorldPainter's
    /// layers: to paint, show, hide, rename and remove.
    Layers,
    /// Ready-made trees, plants and mixes of blocks, to add and paint.
    Packs,
    Levels,
    Info,
    /// The terrain graph (procedural generation and erosion).
    Graph,
    /// The graph's 3D viewport: the selected node as a smooth relief.
    GraphView,
    /// The settings of the selected node of the graph.
    GraphProps,
    /// Sun, clouds and the rest of how the graph's viewport renders.
    RenderSettings,
    /// How Massif runs, live (with Settings ▸ Developer on).
    Developer,
    /// The map's save points: to restore, open, compare.
    Versions,
    /// The Render side's view: the map path-traced.
    RenderView,
    /// The Render side's sun, sky, air and clouds.
    RenderSky,
    /// The Render side's camera: where it stands, its lens.
    RenderCamera,
    /// The Render side's image: its exposure, and stills saved from it.
    RenderOutput,
    /// Keyframes of the camera and the sun over time.
    Timeline,
}

/// A section of the editors an area can show: its title and its editors.
pub type Section = (&'static str, &'static [EditorKind]);

impl EditorKind {
    /// The editors an area can show, column by column, in sections: the
    /// World side's (views and the world, tools, paint), then the Terrain
    /// side's and the Render side's. Developer's comes after the world's,
    /// with developer mode.
    pub const COLUMNS: [&'static [Section]; 5] = [
        &[
            (tn!("View"), &[EditorKind::Map, EditorKind::View3d, EditorKind::Sculpt3d]),
            (tn!("World"), &[EditorKind::Info, EditorKind::Versions]),
        ],
        &[(
            tn!("Tools"),
            &[
                EditorKind::Tools,
                EditorKind::ToolSettings,
                EditorKind::Brushes,
                EditorKind::CustomBrushes,
                EditorKind::BrushSettings,
            ],
        )],
        &[(tn!("Paint"), &[EditorKind::Terrain, EditorKind::Layers, EditorKind::Packs, EditorKind::Blocks, EditorKind::Biomes, EditorKind::Levels])],
        &[(
            tn!("Terrain"),
            &[EditorKind::GraphView, EditorKind::Graph, EditorKind::GraphProps, EditorKind::RenderSettings],
        )],
        &[(
            tn!("Render"),
            &[EditorKind::RenderView, EditorKind::RenderSky, EditorKind::RenderCamera, EditorKind::RenderOutput, EditorKind::Timeline],
        )],
    ];

    /// Developer's section, after the others.
    pub const DEVELOPER: Section = (tn!("Developer"), &[EditorKind::Developer]);

    /// What the editor is for, in a line.
    pub fn about(self) -> &'static str {
        match self {
            EditorKind::Empty => tr!("Nothing yet: choose what it shows"),
            EditorKind::Map => tr!("The map from above, to paint and shape"),
            EditorKind::View3d => tr!("The whole map in 3D, block by block"),
            EditorKind::Sculpt3d => tr!("Build, dig and smooth in 3D, or walk it as a player"),
            EditorKind::Tools => tr!("Every tool, a row for each theme"),
            EditorKind::ToolSettings => tr!("The options of the tool in hand"),
            EditorKind::BrushSettings => tr!("Size, intensity, rotation and where the brush paints"),
            EditorKind::Brushes => tr!("Shapes for the brush, and stamps of terrain"),
            EditorKind::CustomBrushes => tr!("Brushes from greyscale images, yours and WorldPainter's"),
            EditorKind::Terrain => tr!("The block on top of the ground, to paint"),
            EditorKind::Blocks => tr!("Every block of the game, to paint as a terrain"),
            EditorKind::Biomes => tr!("The biomes of the game and its mods, to paint over the automatic ones"),
            EditorKind::Layers => tr!("Ground covers, objects and terrain mixes: paint, show or hide them"),
            EditorKind::Packs => tr!("Ready-made trees, plants and mixes of blocks to paint"),
            EditorKind::Levels => tr!("Levels that add rock over the ground or carve it away"),
            EditorKind::Info => tr!("Name, seed, sea level and size; the column under the cursor"),
            EditorKind::Graph => tr!("The nodes the terrain is generated from"),
            EditorKind::GraphView => tr!("The graph's terrain in relief, under its sky"),
            EditorKind::GraphProps => tr!("The settings of the selected node"),
            EditorKind::RenderSettings => tr!("Sun, clouds and haze, and images saved from the viewport"),
            EditorKind::Developer => tr!("Frames, memory and processor, live"),
            EditorKind::Versions => tr!("Save points of the map: restore, open or compare them"),
            EditorKind::RenderView => tr!("The map path-traced under a real sky, growing clean while it stays still"),
            EditorKind::RenderSky => tr!("A look to start from, the sun by the hour, haze, fog and clouds"),
            EditorKind::RenderCamera => tr!("Where the camera stands and looks, its angle and depth of field"),
            EditorKind::RenderOutput => tr!("Exposure and bounces; save the view as an image up to 8K"),
            EditorKind::Timeline => tr!("Keyframes of the camera and the sun, to scrub and play"),
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            EditorKind::Empty => tr!("Empty area"),
            EditorKind::Map => tr!("Map"),
            EditorKind::View3d => tr!("3D View"),
            EditorKind::Sculpt3d => tr!("3D Sculpt"),
            EditorKind::Tools => tr!("Tools"),
            EditorKind::ToolSettings => tr!("Tool Settings"),
            EditorKind::BrushSettings => tr!("Brush Settings"),
            EditorKind::Brushes => tr!("Brushes"),
            EditorKind::CustomBrushes => tr!("Custom Brushes"),
            EditorKind::Terrain => tr!("Terrain"),
            EditorKind::Blocks => tr!("Blocks"),
            EditorKind::Biomes => tr!("Biomes"),
            EditorKind::Layers => tr!("Layers"),
            EditorKind::Packs => tr!("Packs"),
            EditorKind::Levels => tr!("Levels"),
            EditorKind::Info => tr!("Info"),
            EditorKind::Graph => tr!("Terrain Graph"),
            EditorKind::GraphView => tr!("Graph Viewport"),
            EditorKind::GraphProps => tr!("Node Properties"),
            EditorKind::RenderSettings => tr!("Render Settings"),
            EditorKind::Developer => tr!("Developer"),
            EditorKind::Versions => tr!("Versions"),
            EditorKind::RenderView => tr!("Render Viewport"),
            EditorKind::RenderSky => tr!("Sun and Sky"),
            EditorKind::RenderCamera => tr!("Camera"),
            EditorKind::RenderOutput => tr!("Render Output"),
            EditorKind::Timeline => tr!("Timeline"),
        }
    }

    pub const fn icon(self) -> Icon {
        match self {
            EditorKind::Empty => Icon::LayoutGrid,
            EditorKind::Map => Icon::Map,
            EditorKind::View3d => Icon::Box,
            EditorKind::Sculpt3d => Icon::Pickaxe,
            EditorKind::Tools => Icon::Wrench,
            EditorKind::ToolSettings => Icon::SlidersHorizontal,
            EditorKind::BrushSettings => Icon::Settings2,
            EditorKind::Brushes => Icon::Brush,
            EditorKind::CustomBrushes => Icon::Images,
            EditorKind::Terrain => Icon::Mountain,
            EditorKind::Blocks => Icon::Blocks,
            EditorKind::Biomes => Icon::Trees,
            EditorKind::Layers => Icon::Layers2,
            EditorKind::Packs => Icon::Package,
            EditorKind::Levels => Icon::Layers,
            EditorKind::Info => Icon::Info,
            EditorKind::Graph => Icon::Workflow,
            EditorKind::GraphView => Icon::MountainSnow,
            EditorKind::GraphProps => Icon::SlidersVertical,
            EditorKind::RenderSettings => Icon::Sun,
            EditorKind::Developer => Icon::Activity,
            EditorKind::Versions => Icon::History,
            EditorKind::RenderView => Icon::Aperture,
            EditorKind::RenderSky => Icon::SunMedium,
            EditorKind::RenderCamera => Icon::Video,
            EditorKind::RenderOutput => Icon::ImageDown,
            EditorKind::Timeline => Icon::Film,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Dir {
    /// Side by side: `a` on the left, `b` on the right.
    Row,
    /// Stacked: `a` on top, `b` below.
    Column,
}

/// Where an area is dropped on another one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Zone {
    Left,
    Right,
    Top,
    Bottom,
    /// Swap the two areas.
    Center,
}

pub type AreaId = u64;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Node {
    Area {
        id: AreaId,
        editor: EditorKind,
    },
    Split {
        dir: Dir,
        /// Share of the rectangle given to `a` (0..1).
        ratio: f32,
        a: Box<Node>,
        b: Box<Node>,
    },
}

impl Node {
    pub fn areas(&self, out: &mut Vec<(AreaId, EditorKind)>) {
        match self {
            Node::Area { id, editor } => out.push((*id, *editor)),
            Node::Split { a, b, .. } => {
                a.areas(out);
                b.areas(out);
            }
        }
    }

    pub fn count(&self) -> usize {
        match self {
            Node::Area { .. } => 1,
            Node::Split { a, b, .. } => a.count() + b.count(),
        }
    }

    /// Id of the first (top-left) area under this node.
    #[cfg(test)]
    pub fn first_area(&self) -> AreaId {
        match self {
            Node::Area { id, .. } => *id,
            Node::Split { a, .. } => a.first_area(),
        }
    }

    /// Whether an area under this node shows `kind`.
    pub fn shows(&self, kind: EditorKind) -> bool {
        match self {
            Node::Area { editor, .. } => *editor == kind,
            Node::Split { a, b, .. } => a.shows(kind) || b.shows(kind),
        }
    }

    /// Split the first area showing `above` in two, `below` under it, the
    /// first keeping `ratio` of the height. Whether there was one.
    fn add_below(&mut self, above: EditorKind, below: EditorKind, ratio: f32, next: &mut AreaId) -> bool {
        match self {
            Node::Area { editor, .. } if *editor == above => {
                *next += 1;
                let old = std::mem::replace(self, Node::Area { id: 0, editor: EditorKind::Empty });
                *self = Node::Split {
                    dir: Dir::Column,
                    ratio,
                    a: Box::new(old),
                    b: Box::new(Node::Area { id: *next, editor: below }),
                };
                true
            }
            Node::Area { .. } => false,
            Node::Split { a, b, .. } => a.add_below(above, below, ratio, next) || b.add_below(above, below, ratio, next),
        }
    }

    pub fn editor_of(&self, target: AreaId) -> Option<EditorKind> {
        match self {
            Node::Area { id, editor } => (*id == target).then_some(*editor),
            Node::Split { a, b, .. } => a.editor_of(target).or_else(|| b.editor_of(target)),
        }
    }

    pub fn set_editor(&mut self, target: AreaId, new: EditorKind) -> bool {
        match self {
            Node::Area { id, editor } if *id == target => {
                *editor = new;
                true
            }
            Node::Area { .. } => false,
            Node::Split { a, b, .. } => a.set_editor(target, new) || b.set_editor(target, new),
        }
    }

    /// Replace area `target` by a split holding it and `new`.
    pub fn split(&mut self, target: AreaId, dir: Dir, new_first: bool, new: Node) -> bool {
        match self {
            Node::Area { id, .. } if *id == target => {
                let old = std::mem::replace(
                    self,
                    Node::Area {
                        id: 0,
                        editor: EditorKind::Empty,
                    },
                );
                let (a, b) = if new_first { (new, old) } else { (old, new) };
                *self = Node::Split {
                    dir,
                    ratio: 0.5,
                    a: Box::new(a),
                    b: Box::new(b),
                };
                true
            }
            Node::Area { .. } => false,
            Node::Split { a, b, .. } => {
                // `new` can only be consumed once: find the side first.
                if a.editor_of(target).is_some() {
                    a.split(target, dir, new_first, new)
                } else {
                    b.split(target, dir, new_first, new)
                }
            }
        }
    }

    /// Remove area `target`; its sibling takes the freed space. The root
    /// area cannot be removed.
    pub fn remove(&mut self, target: AreaId) -> Option<Node> {
        let Node::Split { a, b, .. } = self else {
            return None;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        let (a_is, b_is) = (is(a), is(b));
        if a_is || b_is {
            let placeholder = Node::Area {
                id: 0,
                editor: EditorKind::Empty,
            };
            let Node::Split { a, b, .. } = std::mem::replace(self, placeholder) else {
                unreachable!()
            };
            let (removed, kept) = if a_is { (*a, *b) } else { (*b, *a) };
            *self = kept;
            return Some(removed);
        }
        a.remove(target).or_else(|| b.remove(target))
    }

    fn area_mut(&mut self, target: AreaId) -> Option<(&mut AreaId, &mut EditorKind)> {
        match self {
            Node::Area { id, editor } if *id == target => Some((id, editor)),
            Node::Area { .. } => None,
            Node::Split { a, b, .. } => a.area_mut(target).or_else(|| b.area_mut(target)),
        }
    }

    /// Exchange two areas (their identity travels with them, so each keeps
    /// its own view state).
    pub fn swap(&mut self, x: AreaId, y: AreaId) -> bool {
        let (Some(ex), Some(ey)) = (self.editor_of(x), self.editor_of(y)) else {
            return false;
        };
        if x == y {
            return false;
        }
        // Two passes through a sentinel id keep the borrows simple.
        const SENTINEL: AreaId = u64::MAX;
        if let Some((id, editor)) = self.area_mut(x) {
            (*id, *editor) = (SENTINEL, ey);
        }
        if let Some((id, editor)) = self.area_mut(y) {
            (*id, *editor) = (x, ex);
        }
        if let Some((id, _)) = self.area_mut(SENTINEL) {
            *id = y;
        }
        true
    }

    /// Move area `src` next to (or onto) area `dst`.
    pub fn drop_on(&mut self, src: AreaId, dst: AreaId, zone: Zone) -> bool {
        if src == dst || self.editor_of(src).is_none() || self.editor_of(dst).is_none() {
            return false;
        }
        if zone == Zone::Center {
            return self.swap(src, dst);
        }
        let Some(moved) = self.remove(src) else {
            return false;
        };
        let (dir, first) = match zone {
            Zone::Left => (Dir::Row, true),
            Zone::Right => (Dir::Row, false),
            Zone::Top => (Dir::Column, true),
            Zone::Bottom => (Dir::Column, false),
            Zone::Center => unreachable!(),
        };
        self.split(dst, dir, first, moved)
    }

    /// The area sharing `target`'s split, when it is a single area:
    /// `(its id, its editor, the split direction, whether target comes first)`.
    pub fn sibling(&self, target: AreaId) -> Option<(AreaId, EditorKind, Dir, bool)> {
        let Node::Split { dir, a, b, .. } = self else {
            return None;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        let area = |n: &Node| match n {
            Node::Area { id, editor } => Some((*id, *editor)),
            _ => None,
        };
        if is(a) {
            return area(b).map(|(id, e)| (id, e, *dir, true));
        }
        if is(b) {
            return area(a).map(|(id, e)| (id, e, *dir, false));
        }
        a.sibling(target).or_else(|| b.sibling(target))
    }

    /// Set the ratio of the split that holds `target` directly.
    pub fn set_parent_ratio(&mut self, target: AreaId, value: f32) -> bool {
        let Node::Split { ratio, a, b, .. } = self else {
            return false;
        };
        let is = |n: &Node| matches!(n, Node::Area { id, .. } if *id == target);
        if is(a) || is(b) {
            *ratio = value;
            return true;
        }
        a.set_parent_ratio(target, value) || b.set_parent_ratio(target, value)
    }

    /// The newest area of the tree (tests).
    #[cfg(test)]
    pub fn max_id_pub(&self) -> Option<AreaId> {
        Some(self.max_id())
    }

    fn max_id(&self) -> AreaId {
        match self {
            Node::Area { id, .. } => *id,
            Node::Split { a, b, .. } => a.max_id().max(b.max_id()),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Workspace {
    pub name: String,
    pub root: Node,
    /// Area temporarily shown alone.
    #[serde(default)]
    pub maximized: Option<AreaId>,
    /// A workspace of the terrain generator (Gaea's side) rather than of the
    /// map editor (WorldPainter's side).
    #[serde(default)]
    pub terrain: bool,
    /// A workspace of the renderer (the Render side).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub render: bool,
    /// Panels taken out of the layout, in windows of their own over it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub floating: Vec<Floating>,
}

/// A panel floating over the workspace: its area's id (its view state goes
/// with it), what it shows, where and how large, in points.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Floating {
    pub id: AreaId,
    pub editor: EditorKind,
    pub pos: [f32; 2],
    pub size: [f32; 2],
}

impl Workspace {
    /// The side it belongs to.
    pub fn side(&self) -> Side {
        match (self.render, self.terrain) {
            (true, _) => Side::Render,
            (false, true) => Side::Terrain,
            (false, false) => Side::World,
        }
    }

    /// What area `id` shows, docked or floating.
    #[cfg(test)]
    pub fn editor_of(&self, id: AreaId) -> Option<EditorKind> {
        self.root.editor_of(id).or_else(|| self.floating.iter().find(|f| f.id == id).map(|f| f.editor))
    }

    /// Whether an area of it, docked or floating, shows `kind`.
    #[cfg(test)]
    pub fn shows(&self, kind: EditorKind) -> bool {
        self.root.shows(kind) || self.floating.iter().any(|f| f.editor == kind)
    }
}

/// All workspaces of the window.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    pub workspaces: Vec<Workspace>,
    pub active: usize,
    next_id: AreaId,
    /// The workspace last used on each side: map editor, terrain generator.
    #[serde(default)]
    pub last: [usize; 2],
    /// The workspace last used on the Render side.
    #[serde(default)]
    pub last_render: usize,
    /// Which workspaces of later versions it has had added: 1, 3D Sculpt.
    #[serde(default)]
    version: u32,
    /// What each 3D View area shows (its mode, its water), by area: an
    /// area missing here shows the defaults.
    #[serde(default)]
    pub view3d: BTreeMap<AreaId, View3dLook>,
}

/// What a 3D View area shows: kept with the layout, so that each area
/// opens again as it was left.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct View3dLook {
    pub mode: MapMode,
    pub water: bool,
}

impl Default for View3dLook {
    fn default() -> Self {
        Self { mode: MapMode::Terrain, water: true }
    }
}

/// The sides of Massif, in the order of the switcher: the terrain
/// generator, the map editor and the renderer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Terrain,
    World,
    Render,
}

/// The workspaces a layout saved by an older version lacks are added to it
/// once: this is the version they come with.
const LAYOUT_VERSION: u32 = 8;

/// The share of the window's width the column of the tools takes on the
/// left of the Layout, Sculpting and Painting workspaces: the same in all
/// three, so that switching between them does not move the map's edge.
const SIDEBAR: f32 = 0.17;
/// What Sculpting and Painting gave it before version 4.
const OLD_SIDEBAR: f32 = 0.16;

/// Builds trees with fresh area ids.
struct Builder<'a>(&'a mut AreaId);

impl Builder<'_> {
    fn area(&mut self, editor: EditorKind) -> Node {
        *self.0 += 1;
        Node::Area {
            id: *self.0,
            editor,
        }
    }

    fn row(&mut self, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split {
            dir: Dir::Row,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }

    fn column(&mut self, ratio: f32, a: Node, b: Node) -> Node {
        Node::Split {
            dir: Dir::Column,
            ratio,
            a: Box::new(a),
            b: Box::new(b),
        }
    }
}

/// The 3D Sculpt workspace: the tools and their settings on the left, as
/// in Sculpting (its editor works with the tool of the Tools panel), its
/// editor, the map and the levels beside.
fn sculpt_workspace(b: &mut Builder) -> Node {
    let left = {
        let (tools, settings, brush) = (b.area(EditorKind::Tools), b.area(EditorKind::ToolSettings), b.area(EditorKind::BrushSettings));
        let lower = b.column(0.45, settings, brush);
        b.column(0.3, tools, lower)
    };
    // The map over the brushes (3D Sculpt shapes with the brush of the
    // Brushes panel) and the levels.
    let side = {
        let (map, brushes, levels) = (b.area(EditorKind::Map), b.area(EditorKind::Brushes), b.area(EditorKind::Levels));
        let lower = b.column(0.62, brushes, levels);
        b.column(0.45, map, lower)
    };
    let view = b.area(EditorKind::Sculpt3d);
    let centre = b.row(0.78, view, side);
    b.row(SIDEBAR, left, centre)
}

/// The Render side's workspaces: Still, the view with its sky, camera and
/// output beside; Animation, the view over the timeline.
fn render_workspaces(b: &mut Builder) -> Vec<Workspace> {
    use EditorKind::*;
    let view = b.area(RenderView);
    let side = {
        let (sky, camera, output) = (b.area(RenderSky), b.area(RenderCamera), b.area(RenderOutput));
        let lower = b.column(0.42, camera, output);
        b.column(0.52, sky, lower)
    };
    let still = b.row(0.76, view, side);
    let left = {
        let (view, timeline) = (b.area(RenderView), b.area(Timeline));
        b.column(0.74, view, timeline)
    };
    let side = {
        let (camera, sky) = (b.area(RenderCamera), b.area(RenderSky));
        b.column(0.58, camera, sky)
    };
    let animation = b.row(0.76, left, side);
    [(tn!("Still"), still), (tn!("Animation"), animation)]
        .into_iter()
        .map(|(name, root)| Workspace { name: name.into(), root, maximized: None, terrain: false, render: true, floating: Vec::new() })
        .collect()
}

/// The WorldPainter workspace, for those who come from it: the map big in
/// the middle, the tools on the left as WorldPainter's tool palette (and
/// the tool's and the brush's settings under them), the terrain, the
/// layers and the brushes on the right. No 3D view and no graph: both stay
/// one click away (an area's editor, the Terrain side).
fn worldpainter_workspace(b: &mut Builder) -> Node {
    use EditorKind::*;
    let left = {
        let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
        let lower = b.column(0.45, settings, brush);
        b.column(0.34, tools, lower)
    };
    let right = {
        let (terrain, layers, brushes) = (b.area(Terrain), b.area(Layers), b.area(Brushes));
        let lower = b.column(0.45, brushes, layers);
        b.column(0.36, terrain, lower)
    };
    let map = b.area(Map);
    let centre = b.row(0.78, map, right);
    b.row(SIDEBAR, left, centre)
}

/// The name of the WorldPainter workspace.
pub const WORLDPAINTER: &str = tn!("WorldPainter");

/// Give every area of `node` a new id, from `next` on, so that no view
/// state is shared with the areas it was copied from.
fn renumber(node: &mut Node, next: &mut AreaId) {
    match node {
        Node::Area { id, .. } => {
            *next += 1;
            *id = *next;
        }
        Node::Split { a, b, .. } => {
            renumber(a, next);
            renumber(b, next);
        }
    }
}

impl Default for Layout {
    fn default() -> Self {
        use EditorKind::*;
        let mut next_id = 0;
        let mut b = Builder(&mut next_id);
        let mut workspaces = Vec::new();
        let mut add = |name: &str, root: Node| {
            workspaces.push(Workspace {
                name: name.into(),
                root,
                maximized: None,
                terrain: false,
                render: false,
                floating: Vec::new(),
            });
        };

        // Layout: tools and their settings on the left, the map in the
        // middle, 3D, brushes, terrain and levels on the right.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.5, settings, brush);
            b.column(0.26, tools, lower)
        };
        let right = {
            let view = b.area(View3d);
            let (brushes, terrain, layers) = (b.area(Brushes), b.area(Terrain), b.area(Layers));
            let paint = b.column(0.55, terrain, layers);
            let palettes = b.row(0.5, brushes, paint);
            let (levels, info) = (b.area(Levels), b.area(Info));
            let world = b.row(0.5, levels, info);
            let lower = b.column(0.58, palettes, world);
            b.column(0.4, view, lower)
        };
        let map = b.area(Map);
        let centre = b.row(0.64, map, right);
        add(tn!("Layout"), b.row(SIDEBAR, left, centre));

        // Sculpting: a big map, the brushes and the 3D view beside it.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.45, settings, brush);
            b.column(0.3, tools, lower)
        };
        let right = {
            let view = b.area(View3d);
            let (brushes, custom) = (b.area(Brushes), b.area(CustomBrushes));
            let lower = b.column(0.6, brushes, custom);
            b.column(0.45, view, lower)
        };
        let map = b.area(Map);
        let views = b.row(0.7, map, right);
        add(tn!("Sculpting"), b.row(SIDEBAR, left, views));

        // Painting: palettes on both sides of the map.
        let left = {
            let (tools, settings, brush) = (b.area(Tools), b.area(ToolSettings), b.area(BrushSettings));
            let lower = b.column(0.45, settings, brush);
            b.column(0.3, tools, lower)
        };
        let right = {
            let (terrain, layers, blocks, biomes) = (b.area(Terrain), b.area(Layers), b.area(Blocks), b.area(Biomes));
            let upper = b.column(0.55, terrain, layers);
            let lower = b.column(0.62, blocks, biomes);
            b.column(0.5, upper, lower)
        };
        let map = b.area(Map);
        let centre = b.row(0.72, map, right);
        add(tn!("Painting"), b.row(SIDEBAR, left, centre));

        // 3D: the preview takes the stage.
        let side = {
            let (map, levels) = (b.area(Map), b.area(Levels));
            b.column(0.6, map, levels)
        };
        let view = b.area(View3d);
        add("3D", b.row(0.72, view, side));

        // 3D Sculpt: the world in 3D to build and dig in, with its tools.
        add(tn!("3D Sculpt"), sculpt_workspace(&mut b));

        // Maps: two map views side by side, to compare display modes.
        let (m1, m2) = (b.area(Map), b.area(Map));
        add(tn!("Maps"), b.row(0.5, m1, m2));

        // Minimal: just tools and the map.
        let (tools, map) = (b.area(Tools), b.area(Map));
        add(tn!("Minimal"), b.row(0.12, tools, map));

        // The terrain generator. Graph: as in Gaea, the viewport above the
        // graph, the selected node's settings on the right.
        let (view, graph) = (b.area(GraphView), b.area(Graph));
        let left = b.column(0.58, view, graph);
        let props = b.area(GraphProps);
        add(tn!("Graph"), b.row(0.79, left, props));
        // Render: the view takes the stage, with how it is rendered.
        let view = b.area(GraphView);
        let (settings, props) = (b.area(RenderSettings), b.area(GraphProps));
        let side = b.column(0.7, settings, props);
        add(tn!("Render"), b.row(0.79, view, side));
        for w in workspaces.iter_mut().rev().take(2) {
            w.terrain = true;
        }
        // WorldPainter: laid out as WorldPainter is. Last in the list, so
        // that the others keep their place; its tab is the map editor's
        // last.
        workspaces.push(Workspace { name: WORLDPAINTER.into(), root: worldpainter_workspace(&mut b), maximized: None, terrain: false, render: false, floating: Vec::new() });
        workspaces.extend(render_workspaces(&mut b));

        let first_terrain = workspaces.iter().position(|w| w.terrain).unwrap_or(0);
        let first_render = workspaces.iter().position(|w| w.render).unwrap_or(0);
        Layout {
            workspaces,
            active: 0,
            next_id,
            last: [0, first_terrain],
            last_render: first_render,
            version: LAYOUT_VERSION,
            view3d: BTreeMap::new(),
        }
    }
}

impl Layout {
    /// Make a deserialized layout safe to use.
    pub fn sanitize(mut self) -> Self {
        if self.workspaces.is_empty() {
            return Self::default();
        }
        self.active = self.active.min(self.workspaces.len() - 1);
        for l in self.last.iter_mut().chain([&mut self.last_render]) {
            *l = (*l).min(self.workspaces.len() - 1);
        }
        let max = self.workspaces.iter().map(|w| w.floating.iter().map(|f| f.id).fold(w.root.max_id(), AreaId::max)).max();
        self.next_id = self.next_id.max(max.unwrap_or(0));
        if self.version < 1 && !self.workspaces.iter().any(|w| w.root.shows(EditorKind::Sculpt3d)) {
            // Laid out before the 3D Sculpt editor: its workspace, after
            // the 3D one (or the last of the map editor's).
            let at = self
                .workspaces
                .iter()
                .position(|w| !w.terrain && w.name == "3D")
                .or_else(|| self.workspaces.iter().rposition(|w| !w.terrain))
                .map_or(0, |k| k + 1);
            let root = sculpt_workspace(&mut Builder(&mut self.next_id));
            self.workspaces.insert(
                at,
                Workspace {
                    name: tn!("3D Sculpt").into(),
                    root,
                    maximized: None,
                    terrain: false,
                    render: false,
                    floating: Vec::new(),
                },
            );
            for k in [&mut self.active, &mut self.last_render].into_iter().chain(self.last.iter_mut()) {
                if *k >= at {
                    *k += 1;
                }
            }
        }
        // Version 2: the 3D Sculpt workspace shows Brush Settings, whose
        // brush it sculpts with.
        if self.version < 2
            && let Some(k) = self.workspaces.iter().position(|w| w.name == "3D Sculpt" && !w.terrain)
        {
            self.reset_workspace(k);
        }
        // Version 3: the Layers panel, under the Terrain panel of every
        // workspace of the map editor that shows one.
        if self.version < 3 {
            for w in self.workspaces.iter_mut().filter(|w| !w.terrain) {
                if !w.root.shows(EditorKind::Layers) {
                    w.root.add_below(EditorKind::Terrain, EditorKind::Layers, 0.55, &mut self.next_id);
                }
            }
        }
        // Version 4: the tools' column as wide in Sculpting and Painting as
        // in Layout, where it was left as it came.
        if self.version < 4 {
            for w in self.workspaces.iter_mut().filter(|w| !w.terrain && (w.name == "Sculpting" || w.name == "Painting")) {
                if let Node::Split { dir: Dir::Row, ratio, .. } = &mut w.root
                    && (*ratio - OLD_SIDEBAR).abs() < 1e-4
                {
                    *ratio = SIDEBAR;
                }
            }
        }
        // Version 5: the 3D Sculpt editor works with the tool of the Tools
        // panel, which its workspace shows from then on, with Tool Settings
        // (unless it was given them by hand).
        if self.version < 5
            && let Some(k) = self.workspaces.iter().position(|w| w.name == "3D Sculpt" && !w.terrain)
            && !self.workspaces[k].root.shows(EditorKind::Tools)
        {
            self.reset_workspace(k);
        }
        // Version 6: 3D Sculpt shapes with the brush of the Brushes panel,
        // its own picker gone: the panel under the map of its workspace.
        if self.version < 6 {
            for w in self.workspaces.iter_mut().filter(|w| !w.terrain && w.root.shows(EditorKind::Sculpt3d) && !w.root.shows(EditorKind::Brushes)) {
                let _ = w.root.add_below(EditorKind::Map, EditorKind::Brushes, 0.45, &mut self.next_id)
                    || w.root.add_below(EditorKind::BrushSettings, EditorKind::Brushes, 0.5, &mut self.next_id);
            }
        }
        // Version 7: the WorldPainter workspace, at the end (its tab after
        // the map editor's others).
        if self.version < 7 && !self.workspaces.iter().any(|w| !w.terrain && w.name == WORLDPAINTER) {
            let root = worldpainter_workspace(&mut Builder(&mut self.next_id));
            // Before the Render side's, as a new layout has it.
            let at = self.workspaces.iter().position(|w| w.render).unwrap_or(self.workspaces.len());
            self.workspaces.insert(at, Workspace { name: WORLDPAINTER.into(), root, maximized: None, terrain: false, render: false, floating: Vec::new() });
            for k in [&mut self.active, &mut self.last_render].into_iter().chain(self.last.iter_mut()) {
                if *k >= at && self.workspaces.len() > at + 1 {
                    *k += 1;
                }
            }
        }
        // Version 8: the Render side is made of workspaces, as the others.
        if self.version < 8 && !self.workspaces.iter().any(|w| w.render) {
            self.last_render = self.workspaces.len();
            let added = render_workspaces(&mut Builder(&mut self.next_id));
            self.workspaces.extend(added);
        }
        // A floating panel whose id an area has too: a new id.
        for k in 0..self.workspaces.len() {
            for f in 0..self.workspaces[k].floating.len() {
                let id = self.workspaces[k].floating[f].id;
                let twice = self.workspaces[k].root.editor_of(id).is_some() || self.workspaces[k].floating[..f].iter().any(|o| o.id == id);
                if twice {
                    self.next_id += 1;
                    self.workspaces[k].floating[f].id = self.next_id;
                }
            }
        }
        self.version = LAYOUT_VERSION;
        // The looks of 3D View areas no longer there.
        let mut kept = Vec::new();
        for w in &self.workspaces {
            w.root.areas(&mut kept);
            kept.extend(w.floating.iter().map(|f| (f.id, f.editor)));
        }
        self.view3d.retain(|id, _| kept.iter().any(|(k, e)| k == id && *e == EditorKind::View3d));
        self
    }

    /// Take area `id` of the current workspace out of its layout, into a
    /// window floating at `rect` (`[x, y, w, h]`). Not the last area.
    pub fn detach(&mut self, id: AreaId, rect: [f32; 4]) -> bool {
        let ws = self.current_mut();
        let Some(Node::Area { editor, .. }) = ws.root.remove(id) else { return false };
        if ws.maximized == Some(id) {
            ws.maximized = None;
        }
        ws.floating.push(Floating { id, editor, pos: [rect[0], rect[1]], size: [rect[2], rect[3]] });
        true
    }

    /// Put floating panel `id` back into the layout: by area `dst` at
    /// `zone`, or, with none, beside the largest area (`areas` the areas'
    /// rectangles, `[x, y, w, h]`).
    pub fn dock_back(&mut self, id: AreaId, dst: Option<(AreaId, Zone)>, areas: &[(AreaId, [f32; 4])]) -> bool {
        let ws = self.current_mut();
        let Some(k) = ws.floating.iter().position(|f| f.id == id) else { return false };
        let editor = ws.floating[k].editor;
        let (dst, zone) = match dst.filter(|(d, _)| ws.root.editor_of(*d).is_some()) {
            Some(d) => d,
            None => {
                let largest = areas.iter().filter(|(a, _)| ws.root.editor_of(*a).is_some()).max_by(|a, b| (a.1[2] * a.1[3]).total_cmp(&(b.1[2] * b.1[3])));
                let mut all = Vec::new();
                ws.root.areas(&mut all);
                match largest {
                    // Split the long way of the largest.
                    Some((a, r)) => (*a, if r[2] >= r[3] { Zone::Right } else { Zone::Bottom }),
                    None => (all.first().map_or(0, |a| a.0), Zone::Right),
                }
            }
        };
        let (dir, first) = match zone {
            Zone::Left => (Dir::Row, true),
            Zone::Right | Zone::Center => (Dir::Row, false),
            Zone::Top => (Dir::Column, true),
            Zone::Bottom => (Dir::Column, false),
        };
        if !ws.root.split(dst, dir, first, Node::Area { id, editor }) {
            return false;
        }
        let window = ws.floating.remove(k);
        if zone == Zone::Center
            && let Some(Node::Area { editor: other, .. }) = ws.root.remove(dst)
        {
            // Onto an area: in its place, and that one floating where the
            // window was.
            ws.floating.push(Floating { id: dst, editor: other, ..window });
        }
        ws.maximized = None;
        true
    }

    /// Show `kind` in area `id` of the current workspace, docked or floating.
    pub fn set_editor(&mut self, id: AreaId, kind: EditorKind) {
        let ws = self.current_mut();
        if !ws.root.set_editor(id, kind)
            && let Some(f) = ws.floating.iter_mut().find(|f| f.id == id)
        {
            f.editor = kind;
        }
    }

    /// The WorldPainter workspace, shown (made again if it was removed).
    pub fn show_worldpainter(&mut self) {
        let index = match self.workspaces.iter().position(|w| !w.terrain && w.name == WORLDPAINTER) {
            Some(k) => k,
            None => {
                let root = worldpainter_workspace(&mut Builder(&mut self.next_id));
                self.workspaces.push(Workspace { name: WORLDPAINTER.into(), root, maximized: None, terrain: false, render: false, floating: Vec::new() });
                self.workspaces.len() - 1
            }
        };
        self.activate(index);
        self.current_mut().maximized = None;
    }

    /// Show an editor of `kind` in the current workspace: an area of it
    /// there already, else one made under the first of `under` it shows
    /// (or beside the first area). The maximized area gives way to it.
    pub fn bring(&mut self, kind: EditorKind, under: &[EditorKind]) {
        let next = &mut self.next_id;
        let w = &mut self.workspaces[self.active];
        w.maximized = None;
        if w.root.shows(kind) || under.iter().any(|&above| w.root.add_below(above, kind, 0.5, next)) {
            return;
        }
        let mut areas = Vec::new();
        w.root.areas(&mut areas);
        if let Some(&(first, _)) = areas.first() {
            *next += 1;
            w.root.split(first, Dir::Row, false, Node::Area { id: *next, editor: kind });
        }
    }

    pub fn current(&self) -> &Workspace {
        &self.workspaces[self.active]
    }

    /// Is the terrain generator's side shown?
    pub fn terrain(&self) -> bool {
        self.side() == Side::Terrain
    }

    /// The side shown.
    pub fn side(&self) -> Side {
        self.current().side()
    }

    /// Go to a side, where it was left.
    pub fn show_side(&mut self, side: Side) {
        if self.side() == side {
            return;
        }
        let last = match side {
            Side::Render => self.last_render,
            Side::Terrain => self.last[1],
            Side::World => self.last[0],
        };
        let index = if self.workspaces.get(last).is_some_and(|w| w.side() == side) {
            last
        } else {
            match self.workspaces.iter().position(|w| w.side() == side) {
                Some(i) => i,
                None => return,
            }
        };
        self.activate(index);
    }

    /// Show a workspace, remembering it as the last of its side.
    pub fn activate(&mut self, index: usize) {
        if index < self.workspaces.len() {
            self.active = index;
            match self.workspaces[index].side() {
                Side::Render => self.last_render = index,
                Side::Terrain => self.last[1] = index,
                Side::World => self.last[0] = index,
            }
        }
    }

    /// Go to the map editor's side or the terrain generator's, where it was
    /// left.
    pub fn switch_side(&mut self, terrain: bool) {
        self.show_side(if terrain { Side::Terrain } else { Side::World });
    }

    pub fn current_mut(&mut self) -> &mut Workspace {
        &mut self.workspaces[self.active]
    }

    fn fresh_id(&mut self) -> AreaId {
        self.next_id += 1;
        self.next_id
    }

    /// Split an area of the current workspace; the new half starts empty.
    pub fn split(&mut self, target: AreaId, dir: Dir) -> AreaId {
        let id = self.fresh_id();
        let new = Node::Area {
            id,
            editor: EditorKind::Empty,
        };
        self.current_mut().root.split(target, dir, false, new);
        id
    }

    /// Split an area at `ratio` of its width (`Dir::Row`) or height; the new,
    /// empty half comes first when `new_first`.
    pub fn split_at(&mut self, target: AreaId, dir: Dir, new_first: bool, ratio: f32) -> AreaId {
        let id = self.fresh_id();
        let new = Node::Area {
            id,
            editor: EditorKind::Empty,
        };
        let root = &mut self.current_mut().root;
        if root.split(target, dir, new_first, new) {
            root.set_parent_ratio(id, ratio.clamp(0.05, 0.95));
        }
        id
    }

    /// Merge `target` with the area sharing its split: the sibling goes, the
    /// target takes its place.
    pub fn join(&mut self, target: AreaId) -> bool {
        match self.current().root.sibling(target) {
            Some((other, ..)) => {
                self.close(other);
                true
            }
            None => false,
        }
    }

    /// Close an area, or a floating panel.
    pub fn close(&mut self, target: AreaId) {
        let ws = self.current_mut();
        ws.floating.retain(|f| f.id != target);
        if ws.root.remove(target).is_some() && ws.maximized == Some(target) {
            ws.maximized = None;
        }
    }

    pub fn toggle_maximize(&mut self, target: AreaId) {
        let ws = self.current_mut();
        ws.maximized = match ws.maximized {
            Some(_) => None,
            None => Some(target),
        };
    }

    /// Add a copy of the current workspace and switch to it.
    pub fn duplicate_current(&mut self) {
        let mut copy = self.current().clone();
        // Give the copy its own area ids so view state is not shared; its
        // 3D views show what the originals do.
        let (mut old, mut new) = (Vec::new(), Vec::new());
        copy.root.areas(&mut old);
        renumber(&mut copy.root, &mut self.next_id);
        copy.root.areas(&mut new);
        for ((o, _), (n, _)) in old.iter().zip(&new) {
            if let Some(look) = self.view3d.get(o).copied() {
                self.view3d.insert(*n, look);
            }
        }
        for f in &mut copy.floating {
            self.next_id += 1;
            if let Some(look) = self.view3d.get(&f.id).copied() {
                self.view3d.insert(self.next_id, look);
            }
            f.id = self.next_id;
        }
        copy.maximized = None;
        let base = copy.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ').to_owned();
        let taken = |name: &str| self.workspaces.iter().any(|w| w.name == name);
        let n = (2..).find(|n| !taken(&format!("{base} {n}"))).unwrap_or(2);
        copy.name = format!("{base} {n}");
        self.workspaces.push(copy);
        self.activate(self.workspaces.len() - 1);
    }

    /// The workspace `index` was at first: the default one of its name, or
    /// of the name it was copied from ("Painting 2" is a Painting).
    fn default_of(&self, index: usize) -> Option<Workspace> {
        let w = self.workspaces.get(index)?;
        let base = w.name.trim_end_matches(|c: char| c.is_ascii_digit() || c == ' ');
        Layout::default()
            .workspaces
            .into_iter()
            .find(|d| d.side() == w.side() && (d.name == w.name || d.name == base))
    }

    /// Whether workspace `index` has a first state to go back to.
    pub fn can_reset(&self, index: usize) -> bool {
        self.default_of(index).is_some()
    }

    /// Put workspace `index` back as it was at first (its name kept).
    pub fn reset_workspace(&mut self, index: usize) {
        let Some(mut fresh) = self.default_of(index) else { return };
        renumber(&mut fresh.root, &mut self.next_id);
        let w = &mut self.workspaces[index];
        w.root = fresh.root;
        w.maximized = None;
        w.floating.clear();
    }

    /// How many workspaces the side of workspace `index` has.
    pub fn side_len(&self, index: usize) -> usize {
        match self.workspaces.get(index) {
            Some(w) => self.workspaces.iter().filter(|o| o.side() == w.side()).count(),
            None => 0,
        }
    }

    /// Remove a workspace, unless it is the last of its side.
    pub fn remove_workspace(&mut self, index: usize) {
        if self.side_len(index) < 2 {
            return;
        }
        let side = self.workspaces[index].side();
        let was_active = self.active == index;
        // The tab before it, on its side, takes its place.
        let before = self.workspaces[..index].iter().rposition(|w| w.side() == side);
        self.workspaces.remove(index);
        let shift = |i: &mut usize| {
            if *i > index {
                *i -= 1;
            }
        };
        shift(&mut self.active);
        for l in self.last.iter_mut().chain([&mut self.last_render]) {
            shift(l);
        }
        // A side's last workspace may be the one removed.
        let first = |s: Side| self.workspaces.iter().position(|w| w.side() == s).unwrap_or(0);
        let fixed = [Side::World, Side::Terrain, Side::Render].map(|s| {
            let l = match s {
                Side::World => self.last[0],
                Side::Terrain => self.last[1],
                Side::Render => self.last_render,
            };
            if self.workspaces.get(l).is_none_or(|w| w.side() != s) { first(s) } else { l }
        });
        (self.last, self.last_render) = ([fixed[0], fixed[1]], fixed[2]);
        if was_active {
            match before {
                Some(i) => self.activate(i),
                None => {
                    self.active = match side {
                        Side::World => self.last[0],
                        Side::Terrain => self.last[1],
                        Side::Render => self.last_render,
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {

    /// A workspace changed, or a copy of one, goes back to how it was.
    #[test]
    fn workspaces_reset_to_their_first_state() {
        let mut layout = Layout::default();
        let painting = layout.workspaces.iter().position(|w| w.name == "Painting").unwrap();
        layout.activate(painting);
        let first = layout.current().root.count();
        let area = layout.current().root.first_area();
        layout.split(area, Dir::Row);
        assert_eq!(layout.current().root.count(), first + 1);
        layout.reset_workspace(painting);
        assert_eq!(layout.current().root.count(), first);
        assert_eq!(layout.current().name, "Painting");
        // A copy resets to what it was copied from.
        layout.duplicate_current();
        let copy = layout.active;
        assert_eq!(layout.current().name, "Painting 2");
        let area = layout.current().root.first_area();
        layout.split(area, Dir::Column);
        layout.reset_workspace(copy);
        assert_eq!(layout.current().root.count(), first);
        assert!(layout.can_reset(copy));
    }
    use super::*;

    pub(super) fn ids(node: &Node) -> Vec<AreaId> {
        let mut v = Vec::new();
        node.areas(&mut v);
        v.into_iter().map(|(id, _)| id).collect()
    }

    fn area(id: AreaId, editor: EditorKind) -> Node {
        Node::Area { id, editor }
    }

    fn tree() -> Node {
        // (1 | (2 / 3))
        Node::Split {
            dir: Dir::Row,
            ratio: 0.3,
            a: Box::new(area(1, EditorKind::Tools)),
            b: Box::new(Node::Split {
                dir: Dir::Column,
                ratio: 0.5,
                a: Box::new(area(2, EditorKind::Map)),
                b: Box::new(area(3, EditorKind::View3d)),
            }),
        }
    }

    #[test]
    fn default_layout_is_well_formed() {
        let layout = Layout::default();
        assert_eq!(layout.workspaces.len(), 12);
        // Three sides: eight workspaces to edit the map (the WorldPainter
        // workspace last), two to generate terrain, two to render.
        assert_eq!(layout.workspaces.iter().filter(|w| w.terrain).count(), 2);
        assert_eq!(layout.workspaces.iter().filter(|w| w.render).count(), 2);
        let mut seen = std::collections::HashSet::new();
        for ws in &layout.workspaces {
            for id in ids(&ws.root) {
                assert!(id > 0 && seen.insert(id), "duplicate area id {id}");
            }
        }
        assert_eq!(seen.len() as u64, layout.next_id);
        let first = &layout.workspaces[0].root;
        assert_eq!(first.count(), 10);
        // Layouts survive serialisation (they are persisted between runs).
        let json = serde_json::to_string(&layout).unwrap();
        let back: Layout = serde_json::from_str(&json).unwrap();
        assert_eq!(back.sanitize(), layout);
    }

    /// The Render side has workspaces of its own, as the others: going to
    /// it and back goes where each side was left, and it is kept.
    #[test]
    fn the_render_side_has_its_workspaces() {
        let mut layout = Layout::default();
        let world = layout.active;
        layout.show_side(Side::Render);
        assert_eq!(layout.side(), Side::Render);
        assert_eq!(layout.current().name, "Still");
        assert!(layout.current().root.shows(EditorKind::RenderView) && layout.current().root.shows(EditorKind::RenderSky));
        assert!(!layout.terrain(), "the graph's undo is not the Render side's");
        let animation = layout.workspaces.iter().position(|w| w.name == "Animation").unwrap();
        assert!(layout.workspaces[animation].root.shows(EditorKind::Timeline));
        layout.activate(animation);
        let back: Layout = serde_json::from_str(&serde_json::to_string(&layout).unwrap()).unwrap();
        assert_eq!((back.side(), back.current().name.as_str()), (Side::Render, "Animation"), "kept");
        layout.show_side(Side::World);
        assert_eq!((layout.side(), layout.active), (Side::World, world));
        layout.show_side(Side::Terrain);
        assert_eq!(layout.side(), Side::Terrain);
        layout.show_side(Side::Render);
        assert_eq!(layout.active, animation, "back where the Render side was left");
        // Its own workspaces only: the last one stays.
        assert_eq!(layout.side_len(animation), 2);
        layout.remove_workspace(animation);
        assert_eq!(layout.current().name, "Still");
        let still = layout.active;
        layout.remove_workspace(still);
        assert_eq!(layout.current().name, "Still", "the last of its side stays");
    }

    /// A layout of an older version gets the Render side's workspaces once,
    /// after the others, with ids of their own.
    #[test]
    fn old_layouts_get_the_render_workspaces() {
        let mut old = Layout::default();
        old.workspaces.retain(|w| !w.render);
        old.version = 7;
        let json = serde_json::to_string(&old).unwrap();
        let layout = serde_json::from_str::<Layout>(&json).unwrap().sanitize();
        let names: Vec<&str> = layout.workspaces.iter().filter(|w| w.render).map(|w| w.name.as_str()).collect();
        assert_eq!(names, ["Still", "Animation"]);
        assert_eq!(layout.active, 0);
        let mut ids = Vec::new();
        for w in &layout.workspaces {
            ids.extend(super::tests::ids(&w.root));
        }
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "fresh area ids");
        let again = layout.clone().sanitize();
        assert_eq!(again.workspaces.len(), layout.workspaces.len(), "once");
    }

    /// A panel floats out of its layout, keeps its id, survives being saved
    /// and read back, and docks back, beside an area or in its place.
    #[test]
    fn panels_float_and_dock_back() {
        let mut layout = Layout::default();
        let before = layout.current().root.count();
        let mut areas = Vec::new();
        layout.current().root.areas(&mut areas);
        let (info, _) = *areas.iter().find(|a| a.1 == EditorKind::Info).unwrap();
        assert!(layout.detach(info, [300.0, 200.0, 320.0, 240.0]));
        assert_eq!(layout.current().root.count(), before - 1);
        assert_eq!(layout.current().editor_of(info), Some(EditorKind::Info));
        assert!(layout.current().shows(EditorKind::Info) && !layout.current().root.shows(EditorKind::Info));
        layout.set_editor(info, EditorKind::Versions);
        assert_eq!(layout.current().floating[0].editor, EditorKind::Versions);
        // Saved and read back, as it was.
        let back = serde_json::from_str::<Layout>(&serde_json::to_string(&layout).unwrap()).unwrap().sanitize();
        assert_eq!(back.current().floating, layout.current().floating);
        // A copy of the workspace floats a panel of its own.
        let mut copy = back.clone();
        copy.duplicate_current();
        assert_ne!(copy.current().floating[0].id, info);
        // Docked back beside the largest area.
        let map = areas.iter().find(|a| a.1 == EditorKind::Map).unwrap().0;
        let rects = [(map, [0.0, 0.0, 800.0, 600.0])];
        assert!(layout.dock_back(info, None, &rects));
        assert!(layout.current().floating.is_empty());
        assert_eq!(layout.current().root.count(), before);
        assert_eq!(layout.current().root.sibling(info).map(|s| (s.0, s.2, s.3)), Some((map, Dir::Row, false)));
        // Onto an area: in its place, that one floating.
        layout.detach(info, [10.0, 10.0, 200.0, 200.0]);
        assert!(layout.dock_back(info, Some((map, Zone::Center)), &[]));
        assert_eq!(layout.current().root.editor_of(info), Some(EditorKind::Versions));
        assert_eq!(layout.current().floating.iter().map(|f| (f.id, f.editor)).collect::<Vec<_>>(), [(map, EditorKind::Map)]);
        // The last area does not float; a floating panel closes.
        layout.close(map);
        assert!(layout.current().floating.is_empty());
        let mut minimal = Layout { active: 6, ..Layout::default() };
        let first = minimal.current().root.first_area();
        let second = first + 1;
        assert!(minimal.detach(first, [0.0; 4]));
        assert!(!minimal.detach(second, [0.0; 4]), "the last area stays");
        // Reset: the panels back as they were.
        minimal.reset_workspace(6);
        assert!(minimal.current().floating.is_empty());
    }

    /// The WorldPainter workspace: the map big in the middle, the tools on
    /// the left, Terrain, Layers and Brushes on the right, no 3D view and
    /// no graph; given once to layouts saved before it, made again when
    /// it is asked for after being removed.
    #[test]
    fn the_worldpainter_workspace() {
        let layout = Layout::default();
        let wp = layout.workspaces.iter().find(|w| w.name == WORLDPAINTER).unwrap();
        assert!(!wp.terrain);
        for kind in [EditorKind::Map, EditorKind::Tools, EditorKind::ToolSettings, EditorKind::BrushSettings, EditorKind::Terrain, EditorKind::Layers, EditorKind::Brushes] {
            assert!(wp.root.shows(kind), "{kind:?}");
        }
        for kind in [EditorKind::View3d, EditorKind::Sculpt3d, EditorKind::Graph, EditorKind::GraphView] {
            assert!(!wp.root.shows(kind), "{kind:?}");
        }
        // Tools left, the map in the middle and widest, the palettes right.
        let Node::Split { dir: Dir::Row, ratio, a, b } = &wp.root else { panic!() };
        assert!((ratio - SIDEBAR).abs() < 1e-6 && a.shows(EditorKind::Tools));
        let Node::Split { dir: Dir::Row, ratio, a, b } = &**b else { panic!() };
        assert!(matches!(**a, Node::Area { editor: EditorKind::Map, .. }) && *ratio > 0.7);
        assert!(b.shows(EditorKind::Terrain) && b.shows(EditorKind::Layers) && b.shows(EditorKind::Brushes));

        // A layout of version 6: given it once, at the end, the active
        // workspace and the Graph remembered still the same.
        let mut old = Layout::default();
        old.workspaces.retain(|w| w.name != WORLDPAINTER);
        old.version = 6;
        old.activate(old.workspaces.iter().position(|w| w.name == "Graph").unwrap());
        let json = serde_json::to_string(&old).unwrap();
        let back = serde_json::from_str::<Layout>(&json).unwrap().sanitize();
        assert_eq!(back.workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>(), Layout::default().workspaces.iter().map(|w| w.name.as_str()).collect::<Vec<_>>());
        assert_eq!(back.current().name, "Graph");
        let again = serde_json::from_str::<Layout>(&serde_json::to_string(&back).unwrap()).unwrap().sanitize();
        assert_eq!(again.workspaces.len(), back.workspaces.len(), "once");

        // Removed by hand: shown again when asked for.
        let mut layout = back;
        let k = layout.workspaces.iter().position(|w| w.name == WORLDPAINTER).unwrap();
        layout.remove_workspace(k);
        let removed = serde_json::from_str::<Layout>(&serde_json::to_string(&layout).unwrap()).unwrap().sanitize();
        assert!(!removed.workspaces.iter().any(|w| w.name == WORLDPAINTER), "not forced back on");
        layout.show_worldpainter();
        assert_eq!((layout.current().name.as_str(), layout.side()), (WORLDPAINTER, Side::World));
        layout.switch_side(true);
        assert_eq!(layout.current().name, "Graph");
    }

    #[test]
    fn the_two_sides_remember_their_workspace() {
        let mut layout = Layout::default();
        assert!(!layout.terrain());
        layout.activate(2);
        layout.switch_side(true);
        assert!(layout.terrain());
        assert_eq!(layout.current().name, "Graph");
        layout.activate(8);
        layout.switch_side(false);
        assert_eq!(layout.active, 2, "back where the map editor was left");
        layout.switch_side(true);
        assert_eq!(layout.current().name, "Render");
    }

    /// A layout saved before the 3D Sculpt editor gets its workspace, once,
    /// after the 3D one.
    #[test]
    fn old_layouts_get_the_sculpt_workspace() {
        let mut old = Layout::default();
        old.workspaces.retain(|w| w.name != "3D Sculpt");
        old.version = 0;
        old.active = old.workspaces.iter().position(|w| w.name == "Graph").unwrap();
        let json = serde_json::to_string(&old).unwrap().replace(",\"version\":0", "");
        let back: Layout = serde_json::from_str(&json).unwrap();
        let back = back.sanitize();
        let names: Vec<&str> = back.workspaces.iter().map(|w| w.name.as_str()).collect();
        assert_eq!(names[3..5], ["3D", "3D Sculpt"]);
        assert_eq!(back.current().name, "Graph", "the same workspace shown");
        let ids: std::collections::HashSet<AreaId> = back
            .workspaces
            .iter()
            .flat_map(|w| {
                let mut v = Vec::new();
                w.root.areas(&mut v);
                v.into_iter().map(|a| a.0)
            })
            .collect();
        let count: usize = back.workspaces.iter().map(|w| w.root.count()).sum();
        assert_eq!(ids.len(), count, "fresh area ids");
        // Removed by the user, it does not come back.
        let mut gone = back.clone();
        gone.workspaces.retain(|w| w.name != "3D Sculpt");
        let again = serde_json::from_str::<Layout>(&serde_json::to_string(&gone).unwrap()).unwrap().sanitize();
        assert!(!again.workspaces.iter().any(|w| w.name == "3D Sculpt"));
    }

    /// The 3D Sculpt workspace of an older layout gets the Tools panel and
    /// Tool Settings, whose tool its editor works with, once; one given
    /// the Tools panel by hand is left as it is.
    #[test]
    fn the_sculpt_workspace_gets_the_tools() {
        let sculpt = |l: &Layout| l.workspaces.iter().find(|w| w.name == "3D Sculpt").unwrap().root.clone();
        let fresh = Layout::default();
        for kind in [EditorKind::Tools, EditorKind::ToolSettings, EditorKind::BrushSettings, EditorKind::Sculpt3d, EditorKind::Map, EditorKind::Levels] {
            assert!(sculpt(&fresh).shows(kind), "{kind:?}");
        }
        // As version 4 laid it out: its editor, Brush Settings, the map and
        // the levels.
        let mut old = Layout { version: 4, ..Layout::default() };
        let k = old.workspaces.iter().position(|w| w.name == "3D Sculpt").unwrap();
        let mut b = Builder(&mut old.next_id);
        let (brush, map, levels) = (b.area(EditorKind::BrushSettings), b.area(EditorKind::Map), b.area(EditorKind::Levels));
        let lower = b.column(0.6, map, levels);
        let side = b.column(0.42, brush, lower);
        let view = b.area(EditorKind::Sculpt3d);
        old.workspaces[k].root = b.row(0.76, view, side);
        let json = serde_json::to_string(&old).unwrap();
        let layout = serde_json::from_str::<Layout>(&json).unwrap().sanitize();
        assert!(sculpt(&layout).shows(EditorKind::Tools) && sculpt(&layout).shows(EditorKind::ToolSettings));
        // Given the Tools panel by hand: kept.
        let mut mine = Layout { version: 4, ..Layout::default() };
        let mut b = Builder(&mut mine.next_id);
        let (tools, view) = (b.area(EditorKind::Tools), b.area(EditorKind::Sculpt3d));
        mine.workspaces[k].root = b.row(0.2, tools, view);
        let kept = mine.workspaces[k].root.clone();
        assert_eq!(sculpt(&mine.sanitize()), kept);
    }

    /// 3D Sculpt shapes with the brush of the Brushes panel: its workspace
    /// shows that panel, one of version 5 gets it under its map, once.
    #[test]
    fn the_sculpt_workspace_gets_the_brushes() {
        let sculpt = |l: &Layout| l.workspaces.iter().find(|w| w.name == "3D Sculpt").unwrap().root.clone();
        assert!(sculpt(&Layout::default()).shows(EditorKind::Brushes));
        let mut old = Layout { version: 5, ..Layout::default() };
        let k = old.workspaces.iter().position(|w| w.name == "3D Sculpt").unwrap();
        let mut b = Builder(&mut old.next_id);
        let (tools, map, levels, view) = (b.area(EditorKind::Tools), b.area(EditorKind::Map), b.area(EditorKind::Levels), b.area(EditorKind::Sculpt3d));
        let side = b.column(0.6, map, levels);
        let centre = b.row(0.78, view, side);
        old.workspaces[k].root = b.row(0.2, tools, centre);
        let layout = serde_json::from_str::<Layout>(&serde_json::to_string(&old).unwrap()).unwrap().sanitize();
        let root = sculpt(&layout);
        assert!(root.shows(EditorKind::Brushes) && root.shows(EditorKind::Levels) && root.shows(EditorKind::Tools));
        assert_eq!(root.count(), 5);
        // Read again: not added twice.
        let again = serde_json::from_str::<Layout>(&serde_json::to_string(&old.clone().sanitize()).unwrap()).unwrap().sanitize();
        assert_eq!(sculpt(&again).count(), 5);
        // Called from the header where it is not: there, the area alone shown
        // no longer.
        let mut bare = Layout { active: k, ..Layout::default() };
        let mut b = Builder(&mut bare.next_id);
        let (tools, view) = (b.area(EditorKind::Tools), b.area(EditorKind::Sculpt3d));
        bare.workspaces[k].root = b.row(0.2, tools, view);
        bare.workspaces[k].maximized = Some(7);
        bare.bring(EditorKind::Brushes, &[EditorKind::Map, EditorKind::BrushSettings]);
        assert!(bare.current().root.shows(EditorKind::Brushes) && bare.current().maximized.is_none());
        assert_eq!(bare.current().root.count(), 3);
        bare.bring(EditorKind::Brushes, &[EditorKind::Map]);
        assert_eq!(bare.current().root.count(), 3, "once");
    }

    #[test]
    fn each_side_keeps_a_workspace() {
        let mut layout = Layout::default();
        let n = layout.workspaces.len();
        layout.activate(8);
        layout.remove_workspace(8);
        assert_eq!(layout.workspaces.len(), n - 1);
        assert!(layout.terrain(), "still on the terrain side");
        assert_eq!(layout.current().name, "Graph");
        layout.remove_workspace(7);
        assert_eq!(layout.workspaces.len(), n - 1, "the last terrain workspace stays");
        layout.switch_side(false);
        layout.activate(1);
        layout.remove_workspace(0);
        assert_eq!(layout.current().name, "Sculpting", "the active workspace stays shown");
        layout.switch_side(true);
        assert_eq!(layout.current().name, "Graph");
    }

    #[test]
    fn corner_splits_and_joins() {
        // Minimal: tools | map.
        let mut layout = Layout {
            active: 6,
            ..Layout::default()
        };
        let map = layout.current().root.first_area() + 1;
        assert_eq!(layout.current().root.editor_of(map), Some(EditorKind::Map));
        let new = layout.split_at(map, Dir::Row, false, 0.7);
        let (sib, editor, dir, first) = layout.current().root.sibling(new).unwrap();
        assert_eq!((sib, editor, dir, first), (map, EditorKind::Map, Dir::Row, false));
        assert!(layout.join(map));
        assert_eq!(layout.current().root.editor_of(new), None);
        assert_eq!(layout.current().root.count(), 2);
    }

    #[test]
    fn split_and_close_are_inverse() {
        let mut t = tree();
        assert!(t.split(2, Dir::Row, false, area(9, EditorKind::Empty)));
        assert_eq!(ids(&t), vec![1, 2, 9, 3]);
        assert!(t.split(1, Dir::Column, true, area(8, EditorKind::Info)));
        assert_eq!(ids(&t), vec![8, 1, 2, 9, 3]);
        assert!(!t.split(77, Dir::Row, false, area(10, EditorKind::Empty)));

        assert_eq!(t.remove(9), Some(area(9, EditorKind::Empty)));
        assert_eq!(t.remove(8), Some(area(8, EditorKind::Info)));
        assert_eq!(t, tree());
        assert_eq!(t.remove(77), None);
    }

    #[test]
    fn the_last_area_cannot_be_closed() {
        let mut t = tree();
        assert!(t.remove(1).is_some());
        assert!(t.remove(2).is_some());
        assert_eq!(t, area(3, EditorKind::View3d));
        assert_eq!(t.remove(3), None);
    }

    #[test]
    fn swap_exchanges_positions() {
        let mut t = tree();
        assert!(t.swap(1, 3));
        assert_eq!(ids(&t), vec![3, 2, 1]);
        assert_eq!(t.editor_of(3), Some(EditorKind::View3d));
        assert_eq!(t.editor_of(1), Some(EditorKind::Tools));
        assert!(!t.swap(1, 1));
        assert!(!t.swap(1, 42));
    }

    #[test]
    fn drop_moves_an_area_next_to_another() {
        let mut t = tree();
        assert!(t.drop_on(1, 3, Zone::Bottom));
        // 1 left the row, so (2 / (3 / 1)) remains.
        assert_eq!(ids(&t), vec![2, 3, 1]);
        assert!(t.drop_on(1, 2, Zone::Left));
        assert_eq!(ids(&t), vec![1, 2, 3]);
        assert!(t.drop_on(3, 1, Zone::Center));
        assert_eq!(ids(&t), vec![3, 2, 1]);
        assert!(!t.drop_on(2, 2, Zone::Left));
        assert_eq!(t.count(), 3);
    }

    #[test]
    fn layout_operations() {
        let mut layout = Layout::default();
        let before = layout.current().root.count();
        let target = layout.current().root.first_area();
        let new = layout.split(target, Dir::Row);
        assert_eq!(layout.current().root.count(), before + 1);
        assert_eq!(layout.current().root.editor_of(new), Some(EditorKind::Empty));

        layout.toggle_maximize(new);
        assert_eq!(layout.current().maximized, Some(new));
        layout.close(new);
        assert_eq!(layout.current().maximized, None);
        assert_eq!(layout.current().root.count(), before);

        let n = layout.workspaces.len();
        layout.duplicate_current();
        assert_eq!((layout.workspaces.len(), layout.active), (n + 1, n));
        assert_eq!(layout.current().name, "Layout 2");
        let (a, b) = (ids(&layout.workspaces[0].root), ids(&layout.current().root));
        assert!(a.iter().all(|id| !b.contains(id)));
        layout.remove_workspace(n);
        assert_eq!(layout.workspaces.len(), n);
        assert_eq!(layout.current().name, WORLDPAINTER, "the tab before it, on the same side");
    }

    /// A layout saved before the Layers panel gets it under the Terrain
    /// panel of each map workspace that shows one, once; new layouts have it.
    #[test]
    fn layers_panel_is_added_once() {
        assert!(Layout::default().workspaces.iter().filter(|w| w.root.shows(EditorKind::Terrain)).all(|w| w.root.shows(EditorKind::Layers)));
        let mut old = Layout::default();
        for w in &mut old.workspaces {
            let mut areas = Vec::new();
            w.root.areas(&mut areas);
            for (id, kind) in areas {
                if kind == EditorKind::Layers {
                    w.root.set_editor(id, EditorKind::Info);
                }
            }
        }
        old.version = 2;
        let json = serde_json::to_string(&old).unwrap();
        let layout: Layout = serde_json::from_str::<Layout>(&json).unwrap().sanitize();
        let painting = layout.workspaces.iter().find(|w| w.name == "Painting").unwrap();
        assert!(painting.root.shows(EditorKind::Layers));
        assert!(!layout.workspaces.iter().find(|w| w.name == "Minimal").unwrap().root.shows(EditorKind::Layers));
        let count = |l: &Layout| l.workspaces.iter().map(|w| { let mut a = Vec::new(); w.root.areas(&mut a); a.iter().filter(|(_, k)| *k == EditorKind::Layers).count() }).sum::<usize>();
        let n = count(&layout);
        let again = layout.clone().sanitize();
        assert_eq!(count(&again), n);
        let mut ids = Vec::new();
        for w in &layout.workspaces { let mut a = Vec::new(); w.root.areas(&mut a); ids.extend(a.into_iter().map(|(id, _)| id)); }
        let before = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), before, "every area has an id of its own");
    }

    /// The tools' column takes the same share of the width in Layout,
    /// Sculpting and Painting; a layout saved before gets it where it was
    /// left as it came, and keeps a column the hand moved.
    #[test]
    fn the_tools_column_is_as_wide_everywhere() {
        let ratio = |l: &Layout, name: &str| match &l.workspaces.iter().find(|w| w.name == name).unwrap().root {
            Node::Split { dir: Dir::Row, ratio, .. } => *ratio,
            other => panic!("{name}: {other:?}"),
        };
        let fresh = Layout::default();
        for name in ["Layout", "Sculpting", "Painting"] {
            assert_eq!(ratio(&fresh, name), SIDEBAR, "{name}");
        }
        let mut old = Layout { version: 3, ..Layout::default() };
        let set = |l: &mut Layout, name: &str, v: f32| {
            if let Node::Split { ratio, .. } = &mut l.workspaces.iter_mut().find(|w| w.name == name).unwrap().root {
                *ratio = v;
            }
        };
        set(&mut old, "Painting", OLD_SIDEBAR);
        set(&mut old, "Sculpting", 0.23);
        let layout = old.sanitize();
        assert_eq!(ratio(&layout, "Painting"), SIDEBAR);
        assert_eq!(ratio(&layout, "Sculpting"), 0.23, "moved by hand: kept");
    }
}
