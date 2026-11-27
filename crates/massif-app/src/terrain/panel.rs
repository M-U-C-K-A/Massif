//! The panel's state and layout: the library on the left, the viewer over
//! the graph in the middle, the selected node's settings on the right, a
//! status line under them all. The graph shown is the document's, or the
//! inside of a composite opened in it (a breadcrumb leads back); editing
//! goes through the graph's own steps (`format::apply`), each change one
//! step that can be undone.

use super::eval::{self, Categories, Picture, Previews, Request, Shown, ViewerLook};
use super::shade::Surface;
use super::shape::{self, Crumb, NodeShape};
use crate::theme::{self, Palette};
use egui::{Rect, TextureHandle, Ui, Vec2, pos2, vec2};
use massif_terrain::engine::Engine;
use massif_terrain::kernel::Env;
use massif_terrain::live::Live;
use massif_terrain::model::{Graph, Node, NodeUi};
use massif_terrain::registry::{self, NodeSpec, PortKind};
use massif_terrain::space::Space;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;
use std::sync::Arc;

/// What the previews were last asked for: the graph's revision, the node
/// viewed, the viewer's look, the thumbnails' surface, the number of
/// nodes, the terrain's square, the thumbnails' size, the environment's
/// revision (the map read by Map nodes), the scope shown.
type Asked = (u64, Option<Shown>, ViewerLook, Surface, usize, Space, usize, u64, u64);

/// What the editor keeps of a graph beside its nodes and frames: reroute
/// points on wires, settings shown as sockets, folded groups, the view.
/// Keys inside a composite start with its type and a slash.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    /// By wire, `node.input` of the input it feeds: the points it goes
    /// through, in graph units, from the source to the input.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub reroutes: BTreeMap<String, Vec<[f32; 2]>>,
    /// Settings shown as a socket (◇) though nothing drives them yet, by
    /// node: `node.setting`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub exposed: BTreeSet<String>,
    /// Groups of a Map Output folded, by node: `node.group`.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    pub folded: BTreeSet<String>,
    /// The canvas's view when last saved: offset x, y and zoom.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub view: Option<[f32; 3]>,
    /// Nodes snap to the grid when let go.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_snap: bool,
}

impl Layout {
    /// The layout a graph keeps under `editor` (empty when it has none or
    /// it does not read).
    pub fn of(g: &Graph) -> Layout {
        serde_json::from_value(g.editor.clone()).unwrap_or_default()
    }

    /// As the graph keeps it: nothing when empty.
    pub fn value(&self) -> serde_json::Value {
        if *self == Layout::default() { serde_json::Value::Null } else { serde_json::to_value(self).unwrap_or_default() }
    }
}

/// The canvas's view: where graph point (0, 0) is, and the zoom.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    pub offset: Vec2,
    pub zoom: f32,
}

impl Default for View {
    fn default() -> Self {
        Self { offset: vec2(40.0, 40.0), zoom: 1.0 }
    }
}

/// A wire being drawn from a socket.
#[derive(Debug, Clone, PartialEq)]
pub enum Wiring {
    /// From a node's output.
    From { node: String, port: u8 },
}

/// The search to add a node, open at a place of the canvas.
#[derive(Debug, Clone, Default)]
pub struct Search {
    /// Where the node goes, in graph units.
    pub at: [f32; 2],
    /// On screen.
    pub screen: egui::Pos2,
    pub query: String,
    /// Only nodes that can take this type (a wire dropped on the canvas).
    pub takes: Option<PortKind>,
    /// The wire to connect to the node added.
    pub wire: Option<(String, u8)>,
    pub hot: usize,
}

/// What the panel tells its host.
#[derive(Debug, Clone, PartialEq)]
pub enum Event {
    /// The graph changed (a new revision).
    Changed,
    /// Apply to map asked.
    Apply,
}

/// The state an undo step brings back: the document, its layout, and the
/// composites open.
#[derive(Clone)]
struct Snap {
    doc: Graph,
    layout: Layout,
    crumbs: Vec<Crumb>,
}

/// A scope left to open a composite: the step in, the graph shown there,
/// its view.
#[derive(Clone)]
pub struct Outer {
    pub crumb: Crumb,
    pub graph: Graph,
    pub view: View,
}

/// The shapes of the nodes shown, made again when the graph changes.
#[derive(Default)]
struct Shapes {
    key: (u64, u64),
    by_kind: HashMap<String, Rc<NodeShape>>,
    columns: Option<(Rc<NodeShape>, Rc<NodeShape>)>,
}

pub struct TerrainPanel {
    /// The graph shown and edited: the document's, or the inside of the
    /// composite open (its nodes, places and frames; the document's
    /// composites beside them).
    pub graph: Graph,
    /// The scopes left to get here, outermost first.
    pub outer: Vec<Outer>,
    pub layout: Layout,
    /// Bumped whenever the graph changes.
    pub revision: u64,
    /// Bumped whenever a composite is opened or left.
    pub scope_revision: u64,
    pub space: Space,
    pub sim_res: u32,
    pub env: Arc<dyn Env>,
    pub categories: Arc<Categories>,

    pub selected: Vec<String>,
    /// The frame selected (its settings shown).
    pub selected_frame: Option<String>,
    /// The node viewed, in the scope it was viewed in.
    pub viewed: Option<(Vec<Crumb>, Shown)>,
    pub view: View,
    pub wiring: Option<Wiring>,
    pub search: Option<Search>,
    pub library_query: String,
    pub library_open: BTreeSet<&'static str>,
    pub marquee: Option<egui::Pos2>,
    /// The box started with B: it follows the pointer until a click.
    pub box_by_key: bool,
    /// The selection moved with the pointer (G, Shift+D) until a click.
    pub grab: Option<super::canvas::Grab>,
    /// A click that ended a grab or started a box: its release selects
    /// nothing.
    pub swallow_click: bool,
    /// Tab pressed over the canvas this frame, taken before egui's focus.
    pub tab_key: bool,
    /// A text field had the keys at the last frame.
    pub was_typing: bool,
    /// The setting under the pointer, whose help the panel's foot shows.
    pub hovered_help: Option<&'static str>,
    pub show_advanced: bool,
    /// The frame whose title is being edited.
    pub renaming_frame: Option<String>,
    pub renaming_node: Option<String>,
    pub renaming_mask: Option<(String, String)>,
    /// A setting of the composite open being exposed: `node.setting`.
    pub exposing: Option<super::props::Exposing>,
    /// Something to tell, shown on the canvas for a moment.
    pub notice: Option<(String, f64)>,
    /// Apply was asked for: the app starts it at its next frame.
    pub apply_asked: bool,
    /// Apply under way.
    pub applying: Option<super::apply::Applying>,
    /// Apply writes over the map, its ground left as it is (levels of their
    /// own take what the graph digs and raises), rather than into it.
    pub apply_over: bool,
    /// A height map being saved: what to say once it is.
    pub heightmap: Option<std::sync::mpsc::Receiver<Result<String, String>>>,
    /// The world's lowest and highest Y, which a height map's black and
    /// white stand for.
    pub y_range: (i32, i32),

    undo: Vec<Snap>,
    redo: Vec<Snap>,
    /// An edit under way (a drag, a slider): one undo step for all of it.
    editing: bool,
    shapes: RefCell<Shapes>,
    /// By node shown, what its thumbnail is of once composites unfold.
    pub thumb_keys: HashMap<String, Shown>,
    /// What each node of the compiled graph last cost to make, by its
    /// unfolded id: milliseconds and tiles (see [`Self::cost`]).
    pub costs: HashMap<String, (f64, u64)>,
    /// The engine's counts when the graph last changed.
    cost_start: HashMap<String, (u64, f64)>,
    keys_for: (u64, u64),

    pub look: ViewerLook,
    pub thumb_surface: Surface,
    pub live_on: bool,
    pub live: Option<Arc<Live>>,

    previews: Option<Previews>,
    pub thumbs: HashMap<Shown, (TextureHandle, Picture)>,
    pub viewer: Option<(TextureHandle, Picture)>,
    asked: Option<Asked>,
    asks: u64,
    pub events: Vec<Event>,
    /// The viewer's and the graph's share of the middle column's height.
    pub split: f32,
    /// Fit the graph in view at the next frame (a graph just opened).
    pub fit_pending: bool,
    /// Show the whole graph at the next frame, however small (the frame
    /// button).
    pub frame_all_pending: bool,
    /// Where the canvas was drawn last, on screen.
    pub canvas_rect: Rect,
    /// While a wire is drawn: the node near the pointer and what the wire
    /// carries (its groups that take it open).
    pub magnet: Option<(String, PortKind)>,
    /// What the panel last read from or wrote into the document's
    /// `terrain_graph`: anything else there is another world's graph.
    written: Option<Option<serde_json::Value>>,
    /// Changed since written into the document.
    dirty: bool,
    /// ⌘Z and ⌘⇧Z over the canvas undo here (alone, in the tests); in the
    /// app, its own shortcuts route them (`undo`, `redo`).
    pub own_undo_keys: bool,
    /// What the environment was made for: whether it reads the map, the
    /// map's revision, the world, the folder files are read from.
    env_key: Option<(bool, u64, u64, std::path::PathBuf)>,
    env_at: Option<std::time::Instant>,
    /// Bumped whenever the environment is made again.
    pub env_revision: u64,
    /// The document's revision the panel last saw or made.
    doc_seen: u64,
    /// Bumped when the document changed otherwise than by the panel.
    pub map_revision: u64,
    /// What reading the world's graph said (a graph of 1.2 migrated).
    pub notes: Vec<String>,
    /// The Script pane beside the graph (MassifScript).
    pub script: super::script::ScriptPane,
    /// Drawn as separate panels of the app's layout (`Part`), each in an
    /// area of its own with its own title, rather than as one area.
    pub docked: bool,
    /// The frame (egui pass) the panels last started, so that what is done
    /// once a frame is done once whichever panels are shown.
    pass: Option<u64>,
    /// Pixels of the viewer's picture, as it was last drawn.
    viewer_px: [usize; 2],
    /// The viewer in 3D: the graph viewport's view, the picture it shows
    /// (its relief handed to the view), the app's graphics it draws with.
    pub view3d: crate::terrain_view::TerrainView,
    pub viewer3d: Option<Picture>,
    pub gpu_view: Option<egui_wgpu::RenderState>,
    /// The 3D view's render settings shown; the last image saved.
    pub render_open: bool,
    pub saved: Option<Result<String, String>>,
}

/// One of the generator's panels, each an area of the app's layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Search and the nodes by category, to add.
    Library,
    /// The selected node's terrain, 2D or 3D; Live and Apply.
    Viewer,
    /// The canvas, its commands, minimap and the MassifScript pane.
    Graph,
    /// The selected node's settings, or the graph's.
    Properties,
}

impl TerrainPanel {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            graph: Graph::default(),
            outer: Vec::new(),
            layout: Layout::default(),
            revision: 1,
            scope_revision: 0,
            space: Space::new(8192),
            sim_res: 1024,
            env: eval::no_env(),
            categories: Arc::new(Categories::of(&massif_core::registry::Registry::vanilla())),
            selected: Vec::new(),
            selected_frame: None,
            viewed: None,
            view: View::default(),
            wiring: None,
            search: None,
            library_query: String::new(),
            library_open: BTreeSet::from(["Composites", "Shapes"]),
            marquee: None,
            box_by_key: false,
            grab: None,
            swallow_click: false,
            tab_key: false,
            was_typing: false,
            hovered_help: None,
            show_advanced: false,
            renaming_frame: None,
            renaming_node: None,
            renaming_mask: None,
            exposing: None,
            notice: None,
            apply_asked: false,
            applying: None,
            apply_over: false,
            heightmap: None,
            y_range: (-64, 320),
            undo: Vec::new(),
            redo: Vec::new(),
            editing: false,
            shapes: RefCell::new(Shapes::default()),
            thumb_keys: HashMap::new(),
            costs: HashMap::new(),
            cost_start: HashMap::new(),
            keys_for: (0, u64::MAX),
            look: ViewerLook::default(),
            thumb_surface: Surface::Clay,
            live_on: false,
            live: None,
            previews: Some(Previews::new(engine)),
            thumbs: HashMap::new(),
            viewer: None,
            asked: None,
            asks: 0,
            events: Vec::new(),
            split: 0.42,
            fit_pending: true,
            frame_all_pending: false,
            canvas_rect: Rect::NOTHING,
            magnet: None,
            written: None,
            dirty: false,
            own_undo_keys: true,
            docked: false,
            view3d: crate::terrain_view::TerrainView::default(),
            viewer3d: None,
            gpu_view: None,
            render_open: false,
            saved: None,
            pass: None,
            viewer_px: [640, 360],
            env_key: None,
            env_at: None,
            env_revision: 0,
            doc_seen: 0,
            map_revision: 0,
            notes: Vec::new(),
            script: Default::default(),
        }
    }

    /// The app's panel: its engine on the app's graphics card, so that
    /// tiles and pictures stay where the views draw them; on the CPU when
    /// the app draws without wgpu.
    pub fn for_app(frame: &eframe::Frame, reg: &massif_core::registry::Registry) -> Self {
        let engine = match frame.wgpu_render_state() {
            Some(rs) => {
                let name = rs.adapter.get_info().name;
                let gpu = massif_terrain::gpu::Gpu::from_device(rs.device.clone(), rs.queue.clone(), name);
                Engine::with_gpu(gpu, Default::default())
            }
            None => Engine::new(Default::default()),
        };
        let engine = Arc::new(engine);
        let mut p = Self::new(engine.clone());
        p.gpu_view = frame.wgpu_render_state().cloned();
        p.categories = Arc::new(Categories::of(reg));
        // One Live per app, over the same engine: the World map's preview
        // layer (`terrain_live.rs`) draws what it makes.
        let live = Live::new(engine, massif_terrain::live::Settings { mode: massif_terrain::live::Mode::Off, outputs: vec!["height".to_owned(), "albedo".to_owned()], ..Default::default() });
        p.live = Some(Arc::new(live));
        p
    }

    /// Live, which makes the graph's result over the World map.
    pub fn live(&self) -> Option<&Arc<Live>> {
        self.live.as_ref()
    }

    /// Turns Live on or off.
    pub fn set_live(&mut self, on: bool) {
        self.live_on = on;
        let doc = self.document();
        if let Some(l) = &self.live {
            let mut s = l.settings();
            s.mode = if on { massif_terrain::live::Mode::Visible } else { massif_terrain::live::Mode::Off };
            l.set_settings(s);
            if on {
                l.set_graph(&doc, self.space, self.env.clone(), self.revision);
            } else {
                l.clear();
            }
        }
    }

    /// A panel for `graph`, its terrain `size` blocks a side.
    pub fn with_graph(engine: Arc<Engine>, graph: Graph) -> Self {
        let mut p = Self::new(engine);
        p.set_graph(graph);
        p
    }

    /// Opens another graph (not undoable: a new document).
    pub fn set_graph(&mut self, graph: Graph) {
        if let Some(size) = graph.terrain.size {
            self.space.size = size;
        }
        self.space.seed = graph.terrain.seed;
        if let Some(sea) = graph.terrain.sea_level {
            self.space.sea_level = sea;
        }
        self.layout = Layout::of(&graph);
        self.graph = graph;
        self.outer.clear();
        self.scope_revision += 1;
        self.auto_place();
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.selected_frame = None;
        self.viewed = None;
        self.thumbs.clear();
        self.viewer = None;
        self.viewer3d = None;
        self.revision += 1;
        match self.layout.view {
            Some([x, y, z]) => {
                self.view = View { offset: vec2(x, y), zoom: z.clamp(super::canvas::ZOOM_MIN, super::canvas::ZOOM_MAX) };
                self.fit_pending = false;
            }
            None => {
                self.view = View::default();
                self.fit_pending = true;
            }
        }
    }

    /// A graph to start from on a world that has none: its Map Output
    /// alone, to connect what is added.
    pub fn starter() -> Graph {
        let mut g = Graph::default();
        g.nodes.insert("out".to_owned(), Node::new("output.map"));
        g.ui.insert("out".to_owned(), NodeUi { pos: [420.0, 40.0], collapsed: false });
        g
    }

    /// Takes the document's graph when it is not the one the panel has
    /// (another world opened, or the first time), and the square of the
    /// world it covers.
    pub fn sync(&mut self, doc: &mut crate::doc::Document) {
        if self.written.as_ref() != Some(&doc.world.meta.terrain_graph) {
            // The world's graph, or its graph of 1.2 migrated (kept in the
            // world from now on, the same graph, so that saving keeps it
            // without the world counting as changed), or a starter.
            self.notes.clear();
            let had = doc.world.meta.terrain_graph.is_some();
            match massif_terrain::world::graph_of(&doc.world) {
                Some((g, diags)) => {
                    self.notes = diags.iter().map(|d| d.to_string()).collect();
                    if !had {
                        doc.world.meta.terrain_graph = serde_json::to_value(&g).ok();
                    }
                    self.set_graph(g);
                }
                None => self.set_graph(Self::starter()),
            }
            self.written = Some(doc.world.meta.terrain_graph.clone());
            self.dirty = false;
            self.doc_seen = doc.revision;
        }
        // The square of the world, as Live and Apply take it, with the
        // graph's own settings over the world's (as compiling does).
        let terrain = self.root_terrain();
        let mut space = massif_terrain::world::space_of(&doc.world, doc.dim, &Graph { terrain: terrain.clone(), ..Graph::default() });
        space.seed = terrain.seed;
        if let Some(sea) = terrain.sea_level {
            space.sea_level = sea;
        }
        self.space = space;
        let dim = doc.dimension();
        self.y_range = (dim.min_y, dim.max_y);
        // Another change to the document than the panel's own: the map may
        // have changed under the graph's Map nodes.
        if doc.revision != self.doc_seen {
            self.doc_seen = doc.revision;
            self.map_revision += 1;
        }
        let has_map = self.root_nodes().values().any(|n| n.kind == "input.map");
        let folder = doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        let want = (has_map, if has_map { self.map_revision } else { 0 }, doc.generation, folder.clone());
        if self.env_key.as_ref() != Some(&want) {
            // While the map is painted, its snapshot is taken again at most
            // four times a second.
            let same_map = self.env_key.as_ref().is_some_and(|k| k.0 && has_map && k.2 == doc.generation && k.3 == folder);
            let recent = self.env_at.is_some_and(|t| t.elapsed() < std::time::Duration::from_millis(250));
            if !(same_map && recent) {
                self.env = if has_map {
                    Arc::new(massif_terrain::world::MapEnv::of(doc.world.dims[doc.dim].snapshot(), &doc.world.registry, self.map_revision, folder))
                } else {
                    Arc::new(massif_terrain::files::DiskEnv::new(folder))
                };
                self.env_key = Some(want);
                self.env_at = Some(std::time::Instant::now());
                self.env_revision += 1;
            }
        }
    }

    /// Saves the graph's height over its whole terrain, a sample a block, as
    /// a 16-bit PNG at `path` (black the world's lowest Y, white its
    /// highest) or, for an `.exr` path, an OpenEXR of the heights in blocks,
    /// on a thread of its own: [`Self::heightmap`] says when done.
    pub fn save_heightmap(&mut self, path: std::path::PathBuf) {
        let Some(engine) = self.engine().cloned() else { return };
        let graph = self.document();
        let (space, env, (lo, hi)) = (self.space, self.env.clone(), self.y_range);
        let (tx, rx) = std::sync::mpsc::channel();
        self.heightmap = Some(rx);
        std::thread::spawn(move || {
            let started = std::time::Instant::now();
            let result = (|| {
                let (c, diags) = massif_terrain::compile::compile_in(&graph, space, space.size.min(4096), env);
                if let Some(d) = diags.iter().find(|d| d.severity == massif_terrain::compile::Severity::Error) {
                    return Err(d.to_string());
                }
                let h = c.output("height").ok_or_else(|| t!("The graph's Map Output takes no Height: nothing to save").to_owned())?;
                let grid = engine.whole(&c, h, 0, &|| false).map_err(|e| e.to_string())?;
                let n = grid.rect.w as usize;
                let (lo, hi) = (lo as f32, hi as f32);
                if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("exr")) {
                    let heights = &grid.data[..n * n];
                    let (low, high) = heights.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(a, b), &v| (a.min(v), b.max(v)));
                    massif_core::heightmap::write_exr(&path, n, n, heights).map_err(|e| t!("The height map could not be saved: {e}", e = e))?;
                    return Ok(t!("Saved the terrain as an EXR height map of {n} × {n} in {s} s, heights in blocks from Y {lo} to {hi}", n = n, s = format!("{:.1}", started.elapsed().as_secs_f64()), lo = format!("{low:.1}"), hi = format!("{high:.1}")));
                }
                let samples: Vec<f32> = grid.data[..n * n].iter().map(|v| (v - lo) / (hi - lo)).collect();
                massif_core::heightmap::write_png16(&path, n, n, &samples).map_err(|e| t!("The height map could not be saved: {e}", e = e))?;
                Ok(t!("Saved the terrain as a height map of {n} × {n} in {s} s: black is Y {lo}, white Y {hi}", n = n, s = format!("{:.1}", started.elapsed().as_secs_f64()), lo = format!("{lo:.0}"), hi = format!("{hi:.0}")))
            })();
            let _ = tx.send(result);
        });
    }

    /// Writes the graph, with its layout and view, into the document when
    /// it changed, so that saving keeps it.
    pub fn commit(&mut self, doc: &mut crate::doc::Document) {
        if !self.dirty {
            return;
        }
        self.dirty = false;
        let value = serde_json::to_value(self.saved()).ok();
        if self.written.as_ref() != Some(&value) {
            doc.world.meta.terrain_graph = value.clone();
            doc.mark_changed();
            self.doc_seen = doc.revision;
            self.written = Some(value);
        }
    }

    /// The graph as saved: the document, with its layout and the canvas's
    /// view (the outermost scope's).
    pub fn saved(&self) -> Graph {
        let mut g = self.document();
        let mut l = self.layout.clone();
        let v = self.outer.first().map_or(self.view, |o| o.view);
        l.view = Some([v.offset.x, v.offset.y, v.zoom]);
        g.editor = l.value();
        g
    }

    /// The panel in the app: the document's graph, edited, written back.
    pub fn ui_in(&mut self, ui: &mut Ui, doc: &mut crate::doc::Document) {
        self.own_undo_keys = false;
        self.script.title = doc.world.meta.name.clone();
        self.sync(doc);
        self.ui(ui);
        self.commit(doc);
    }

    /// One panel of the generator in the app (`part` in `ui`'s room): the
    /// document's graph taken once a frame, edited, written back.
    pub fn part_in(&mut self, ui: &mut Ui, doc: &mut crate::doc::Document, part: Part) {
        self.own_undo_keys = false;
        self.docked = true;
        if self.begin_pass(ui.ctx()) {
            self.script.title = doc.world.meta.name.clone();
            self.sync(doc);
        }
        self.part(ui, part);
        self.commit(doc);
    }

    /// At the first panel drawn in a frame: the previous frame's previews
    /// asked for (their size the viewer's as last drawn), its edit ended
    /// when nothing is held. True when this frame had not started yet.
    fn begin_pass(&mut self, ctx: &egui::Context) -> bool {
        let pass = ctx.cumulative_pass_nr();
        if self.pass == Some(pass) {
            return false;
        }
        let first = self.pass.is_none();
        self.pass = Some(pass);
        self.events.clear();
        if !first {
            self.after_frame(ctx, self.viewer_px);
        }
        true
    }

    /// What follows the drawing of a frame: previews asked for, an edit
    /// ended when nothing is held, a repaint while previews come.
    fn after_frame(&mut self, ctx: &egui::Context, viewer_px: [usize; 2]) {
        let thumb_px = if ctx.pixels_per_point() > 1.5 { 256 } else { 128 };
        self.update_previews(ctx, viewer_px, thumb_px);
        let held = ctx.input(|i| i.pointer.any_down()) || ctx.memory(|m| m.focused().is_some());
        if !held {
            self.editing = false;
        }
        if !self.previews_idle() && !theme::reduce_motion(ctx) {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// The viewer's pixels for `viewer` (its card), at most 1600 × 1200,
    /// keeping its shape.
    fn viewer_pixels(viewer: Rect, px: f32) -> [usize; 2] {
        let inner = super::viewer::picture_rect(viewer);
        let (vw, vh) = (inner.width().max(1.0) * px, inner.height().max(1.0) * px);
        let k = (1600.0 / vw).min(1200.0 / vh).min(1.0);
        [((vw * k) as usize).max(64), ((vh * k) as usize).max(64)]
    }

    /// One panel, filling `ui`'s room.
    pub fn part(&mut self, ui: &mut Ui, part: Part) {
        let pal = Palette::of(ui);
        let full = ui.available_rect_before_wrap();
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(full).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(full.intersect(ui.clip_rect()));
        let ui = &mut child;
        match part {
            Part::Library => super::library::ui(self, ui),
            Part::Viewer => {
                self.viewer_px = Self::viewer_pixels(full, ui.ctx().pixels_per_point());
                super::viewer::ui(self, ui);
            }
            Part::Graph => {
                // The status line at the foot; the Script pane beside the
                // graph, or in its place when narrow.
                let status_h = 22.0;
                let body = Rect::from_min_max(full.min, pos2(full.right(), (full.bottom() - status_h).max(full.top())));
                let status = Rect::from_min_max(pos2(full.left() + theme::PAD, body.bottom()), full.max);
                let (graph, script) = match (self.script.open, body.width() >= 640.0) {
                    (false, _) => (body, None),
                    (true, true) => {
                        let w = (body.width() * 0.46).clamp(300.0, 620.0);
                        (Rect::from_min_max(body.min, pos2(body.right() - w, body.bottom())), Some(Rect::from_min_max(pos2(body.right() - w, body.top()), body.max)))
                    }
                    (true, false) => (Rect::NOTHING, Some(body)),
                };
                if graph.is_positive() {
                    let mut g = ui.new_child(egui::UiBuilder::new().max_rect(graph).layout(egui::Layout::top_down(egui::Align::Min)));
                    g.set_clip_rect(graph.intersect(ui.clip_rect()));
                    super::canvas::ui(self, &mut g);
                }
                if let Some(r) = script {
                    if graph.is_positive() {
                        ui.painter().vline(r.left(), r.y_range(), egui::Stroke::new(1.0, pal.border));
                    }
                    let mut s = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::top_down(egui::Align::Min)));
                    s.set_clip_rect(r.intersect(ui.clip_rect()));
                    super::script::ui(self, &mut s);
                }
                ui.painter().hline(full.x_range(), body.bottom(), egui::Stroke::new(1.0, pal.border));
                self.status_ui(ui, status);
            }
            Part::Properties => {
                self.hovered_help = None;
                super::props::ui(self, ui);
            }
        }
        // The dialog that names an exposed setting, over whichever panel.
        if self.exposing.is_some() {
            super::props::expose_dialog(self, ui);
        }
    }

    pub fn engine(&self) -> Option<&Arc<Engine>> {
        self.previews.as_ref().map(|p| p.engine())
    }

    pub fn status(&self) -> eval::Status {
        self.previews.as_ref().map(|p| p.status()).unwrap_or_default()
    }

    /// Whether the previews are all made (for tests).
    pub fn previews_idle(&self) -> bool {
        self.previews.as_ref().is_none_or(|p| p.idle())
    }

    // ---- Scopes ------------------------------------------------------

    /// The steps into the composite open, outermost first.
    pub fn crumbs(&self) -> Vec<Crumb> {
        self.outer.iter().map(|o| o.crumb.clone()).collect()
    }

    /// The type of the composite open, if any.
    pub fn scope_kind(&self) -> Option<&str> {
        self.outer.last().map(|o| o.crumb.kind.as_str())
    }

    /// The whole document's terrain settings.
    pub fn root_terrain(&self) -> &massif_terrain::model::Terrain {
        self.outer.first().map_or(&self.graph.terrain, |o| &o.graph.terrain)
    }

    /// The document's own nodes (outside every composite).
    pub fn root_nodes(&self) -> &BTreeMap<String, Node> {
        self.outer.first().map_or(&self.graph.nodes, |o| &o.graph.nodes)
    }

    /// Whether what is shown can be changed: not inside one of Massif's
    /// composites (edited as a copy).
    pub fn read_only(&self) -> bool {
        let comps = &self.graph.components;
        self.outer.iter().any(|o| !comps.contains_key(base_kind(&o.crumb.kind)))
    }

    /// `inner`, the graph of a composite of type `kind` shown inside
    /// `outer`, written back into `outer`: its composites (the latest
    /// copy), its nodes, places and frames into its own composite.
    fn fold(mut outer: Graph, inner: &Graph, kind: &str) -> Graph {
        outer.components = inner.components.clone();
        if let Some(c) = outer.components.get_mut(base_kind(kind)) {
            c.nodes = inner.nodes.clone();
            c.ui = inner.ui.clone();
            c.frames = inner.frames.clone();
        }
        outer
    }

    /// The document as it is now, whatever composite is open.
    pub fn document(&self) -> Graph {
        let mut g = self.graph.clone();
        for o in self.outer.iter().rev() {
            g = Self::fold(o.graph.clone(), &g, &o.crumb.kind);
        }
        g
    }

    /// The graph of composite node `id` of the graph shown: its nodes,
    /// places and frames, the document's composites beside them.
    fn inside_of(&self, id: &str) -> Option<(Crumb, Graph)> {
        let node = self.graph.nodes.get(id)?;
        if !node.is_component() {
            return None;
        }
        let comp = massif_terrain::lower::component(&self.graph, &node.kind)?;
        let g = Graph { terrain: self.graph.terrain.clone(), nodes: comp.nodes.clone(), ui: comp.ui.clone(), frames: comp.frames.clone(), components: self.graph.components.clone(), ..Graph::default() };
        Some((Crumb { node: id.to_owned(), kind: node.kind.clone() }, g))
    }

    fn push_scope(&mut self, id: &str) -> bool {
        let Some((crumb, inside)) = self.inside_of(id) else { return false };
        let graph = std::mem::replace(&mut self.graph, inside);
        self.outer.push(Outer { crumb, graph, view: self.view });
        self.scope_revision += 1;
        true
    }

    /// Opens composite node `id`: the canvas shows its inside.
    pub fn open(&mut self, id: &str) -> bool {
        self.editing = false;
        if !self.push_scope(id) {
            return false;
        }
        let had_places = !self.graph.ui.is_empty();
        self.auto_place();
        if !had_places && !self.read_only() {
            // Places made for a composite that had none are kept with it.
            self.dirty = true;
        }
        self.selected.clear();
        self.selected_frame = None;
        self.search = None;
        self.wiring = None;
        self.view = View::default();
        self.fit_pending = true;
        true
    }

    /// Leaves the composite open for the scope around it.
    pub fn close(&mut self) -> bool {
        let Some(o) = self.outer.pop() else { return false };
        let inner = std::mem::take(&mut self.graph);
        self.graph = Self::fold(o.graph, &inner, &o.crumb.kind);
        self.view = o.view;
        self.scope_revision += 1;
        self.selected = vec![o.crumb.node];
        self.selected_frame = None;
        self.search = None;
        self.wiring = None;
        self.editing = false;
        true
    }

    /// Leaves composites until `depth` are open.
    pub fn close_to(&mut self, depth: usize) {
        while self.outer.len() > depth {
            self.close();
        }
    }

    /// Opens the scopes `crumbs` names, as far as they still exist.
    fn reopen(&mut self, crumbs: &[Crumb]) {
        for c in crumbs {
            let same = self.graph.nodes.get(&c.node).is_some_and(|n| n.kind == c.kind);
            if !same || !self.push_scope(&c.node) {
                break;
            }
        }
    }

    /// A key of the layout for something of the graph shown: as it is at
    /// the top, after the composite's type and a slash inside one.
    pub fn lkey(&self, key: &str) -> String {
        match self.scope_kind() {
            Some(k) => format!("{k}/{key}"),
            None => key.to_owned(),
        }
    }

    // ---- Editing, with undo ------------------------------------------

    fn snapshot(&self) -> Snap {
        Snap { doc: self.document(), layout: self.layout.clone(), crumbs: self.crumbs() }
    }

    /// The graph to change: the first change of an edit records an undo
    /// step.
    pub fn edit(&mut self) -> &mut Graph {
        if !self.editing {
            let s = self.snapshot();
            self.undo.push(s);
            if self.undo.len() > 200 {
                self.undo.remove(0);
            }
            self.redo.clear();
            self.editing = true;
        }
        self.revision += 1;
        self.dirty = true;
        &mut self.graph
    }

    /// The layout to change, recorded as [`Self::edit`] records.
    pub fn edit_layout(&mut self) -> &mut Layout {
        if !self.editing {
            let s = self.snapshot();
            self.undo.push(s);
            self.redo.clear();
            self.editing = true;
        }
        self.dirty = true;
        &mut self.layout
    }

    /// Ends the edit under way: the next change is a step of its own.
    pub fn end_edit(&mut self) {
        self.editing = false;
    }

    /// Whether an edit is under way (a drag, a slider, a title typed).
    pub fn is_editing(&self) -> bool {
        self.editing
    }

    /// Goes on with the edit under way: the next change joins its step.
    pub fn keep_editing(&mut self) {
        self.editing = true;
    }

    /// The layout changed with no step of its own (places snapped).
    pub fn dirty_layout(&mut self) {
        self.dirty = true;
    }

    /// One of the graph's editing steps, on the graph shown: done (giving
    /// the node it named or added) or refused (giving why), the graph then
    /// as it was. Every change of what the graph computes goes through
    /// here, as scripts and models make them.
    pub fn op(&mut self, op: massif_terrain::format::Op) -> Result<String, String> {
        if self.read_only() {
            return Err(t!("Massif's composites are read only: edit a copy").to_owned());
        }
        let mut g = self.graph.clone();
        let done = massif_terrain::format::apply(&mut g, &op)?;
        // One undo step with what else this edit does (a slider dragged,
        // what one click makes), until nothing is held.
        self.edit();
        self.graph = g;
        Ok(done)
    }

    /// Several steps as one: all done, or none.
    pub fn ops(&mut self, ops: Vec<massif_terrain::format::Op>) -> Result<Vec<String>, String> {
        if self.read_only() {
            return Err(t!("Massif's composites are read only: edit a copy").to_owned());
        }
        let mut g = self.graph.clone();
        let mut out = Vec::new();
        for op in &ops {
            out.push(massif_terrain::format::apply(&mut g, op)?);
        }
        self.edit();
        self.graph = g;
        Ok(out)
    }

    /// The whole document replaced by `graph` (a script applied), as one
    /// step to undo: the composites open are closed, the nodes that were
    /// there already stay where they were, the others are laid out.
    pub fn replace_document(&mut self, mut graph: Graph) {
        self.close_to(0);
        let old = self.document();
        massif_terrain::mscript::lay_out(&mut graph, Some(&old));
        graph.editor = old.editor.clone();
        self.edit();
        self.graph = graph;
        self.editing = false;
        self.scope_revision += 1;
        self.selected.retain(|s| self.graph.nodes.contains_key(s));
        if self.selected_frame.as_ref().is_some_and(|f| !self.graph.frames.contains_key(f)) {
            self.selected_frame = None;
        }
        self.viewed = None;
    }

    /// The composite open, its nodes as they are now: what steps on the
    /// composite itself (exposing a setting…) check against.
    pub fn sync_scope(&mut self) {
        let Some(kind) = self.scope_kind().map(base_kind).map(str::to_owned) else { return };
        let (nodes, ui, frames) = (self.graph.nodes.clone(), self.graph.ui.clone(), self.graph.frames.clone());
        if let Some(c) = self.graph.components.get_mut(&kind) {
            c.nodes = nodes;
            c.ui = ui;
            c.frames = frames;
        }
    }

    /// Tells something on the canvas for a few seconds.
    pub fn say(&mut self, text: impl Into<String>, now: f64) {
        self.notice = Some((text.into(), now));
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    fn restore(&mut self, s: Snap) {
        let view = self.view;
        let depth = self.outer.len();
        self.graph = s.doc;
        self.layout = s.layout;
        self.outer.clear();
        self.reopen(&s.crumbs);
        if self.outer.len() == depth {
            self.view = view;
        }
        self.scope_revision += 1;
        self.revision += 1;
        self.dirty = true;
        self.editing = false;
        self.selected.retain(|s| self.graph.nodes.contains_key(s));
        if self.selected_frame.as_ref().is_some_and(|f| !self.graph.frames.contains_key(f)) {
            self.selected_frame = None;
        }
    }

    /// Undoes the last change; whether there was one.
    pub fn undo(&mut self) -> bool {
        let Some(s) = self.undo.pop() else { return false };
        let now = self.snapshot();
        self.redo.push(now);
        self.restore(s);
        true
    }

    /// Redoes what was undone; whether there was something.
    pub fn redo(&mut self) -> bool {
        let Some(s) = self.redo.pop() else { return false };
        let now = self.snapshot();
        self.undo.push(now);
        self.restore(s);
        true
    }

    // ---- The graph ----------------------------------------------------

    /// What node `id` of the graph shown is, as the canvas draws it (the
    /// two columns of an opened composite included).
    pub fn shape(&self, id: &str) -> Option<Rc<NodeShape>> {
        let mut s = self.shapes.borrow_mut();
        let key = (self.revision, self.scope_revision);
        if s.key != key {
            *s = Shapes { key, ..Shapes::default() };
        }
        if (id == shape::IN_COLUMN || id == shape::OUT_COLUMN) && !self.outer.is_empty() && !self.graph.nodes.contains_key(id) {
            if s.columns.is_none() {
                let kind = self.scope_kind()?;
                let comp = massif_terrain::lower::component(&self.graph, kind)?;
                let (l, r) = NodeShape::columns(&comp, &self.graph);
                s.columns = Some((Rc::new(l), Rc::new(r)));
            }
            let (l, r) = s.columns.as_ref()?;
            return Some(if id == shape::IN_COLUMN { l.clone() } else { r.clone() });
        }
        let kind = &self.graph.nodes.get(id)?.kind;
        if let Some(sh) = s.by_kind.get(kind) {
            return Some(sh.clone());
        }
        let sh = Rc::new(NodeShape::of_kind(&self.graph, kind)?);
        s.by_kind.insert(kind.clone(), sh.clone());
        Some(sh)
    }

    /// The ids drawn on the canvas: the nodes, and inside a composite its
    /// two columns.
    pub fn ids(&self) -> Vec<String> {
        let mut ids: Vec<String> = self.graph.nodes.keys().cloned().collect();
        if !self.outer.is_empty() {
            ids.push(shape::IN_COLUMN.to_owned());
            ids.push(shape::OUT_COLUMN.to_owned());
        }
        ids
    }

    /// Whether `id` is one of an opened composite's columns.
    pub fn is_column(&self, id: &str) -> bool {
        !self.outer.is_empty() && (id == shape::IN_COLUMN || id == shape::OUT_COLUMN) && !self.graph.nodes.contains_key(id)
    }

    pub fn spec(&self, id: &str) -> Option<&'static NodeSpec> {
        self.graph.nodes.get(id).and_then(|n| registry::spec(&n.kind))
    }

    pub fn pos(&self, id: &str) -> [f32; 2] {
        if self.is_column(id) {
            return self.column_pos(id == shape::IN_COLUMN);
        }
        self.graph.ui.get(id).map_or([0.0, 0.0], |u| u.pos)
    }

    /// Where an opened composite's columns stand: left and right of its
    /// nodes.
    fn column_pos(&self, left: bool) -> [f32; 2] {
        let mut lo = [f32::MAX; 2];
        let mut hi = [f32::MIN; 2];
        for (id, u) in &self.graph.ui {
            if !self.graph.nodes.contains_key(id) {
                continue;
            }
            let w = if self.shape(id).is_some_and(|s| s.is_output()) { super::canvas::OUTPUT_WIDTH } else { super::canvas::WIDTH };
            lo = [lo[0].min(u.pos[0]), lo[1].min(u.pos[1])];
            hi = [hi[0].max(u.pos[0] + w), hi[1].max(u.pos[1])];
        }
        if lo[0] > hi[0] {
            lo = [0.0, 0.0];
            hi = [400.0, 0.0];
        }
        if left { [lo[0] - super::canvas::WIDTH - 90.0, lo[1]] } else { [hi[0] + 90.0, lo[1]] }
    }

    pub fn collapsed(&self, id: &str) -> bool {
        self.graph.ui.get(id).is_some_and(|u| u.collapsed)
    }

    /// The name a node shows: its own, or its type's.
    pub fn display_name(&self, id: &str) -> String {
        if let Some(n) = self.graph.nodes.get(id).and_then(|n| n.name.clone()) {
            return n;
        }
        self.shape(id).map_or_else(|| id.to_owned(), |s| s.label.clone())
    }

    /// Adds a node of type `kind` (of the registry, or a composite) at
    /// `at`, selected; gives its id. A type of the registry named without
    /// its version gets its latest one.
    pub fn add_node(&mut self, kind: &str, at: [f32; 2]) -> String {
        let kind = if kind.contains('@') || massif_terrain::model::is_component_type(kind) { kind.to_owned() } else { registry::latest(kind).map_or_else(|| kind.to_owned(), |s| s.type_name()) };
        match self.op(massif_terrain::format::Op::Add { id: None, kind, params: BTreeMap::new(), inputs: BTreeMap::new() }) {
            Ok(id) => {
                self.graph.ui.insert(id.clone(), NodeUi { pos: at, collapsed: false });
                self.selected = vec![id.clone()];
                id
            }
            Err(e) => {
                self.notice = Some((e, 0.0));
                String::new()
            }
        }
    }

    /// Removes nodes, and the wires to them.
    pub fn remove_nodes(&mut self, ids: &[String]) {
        let ids: Vec<String> = ids.iter().filter(|id| self.graph.nodes.contains_key(*id)).cloned().collect();
        if ids.is_empty() {
            return;
        }
        let ops = ids.iter().map(|id| massif_terrain::format::Op::Remove { node: id.clone() }).collect();
        if self.ops(ops).is_err() {
            return;
        }
        // The composite's outputs made by them are left empty.
        if let Some(kind) = self.scope_kind().map(base_kind).map(str::to_owned)
            && let Some(c) = self.graph.components.get_mut(&kind)
        {
            for o in &mut c.outputs {
                if ids.iter().any(|id| massif_terrain::model::Source::parse(&o.from).node == id) {
                    o.from.clear();
                }
            }
        }
        let keep: Vec<String> = self.graph.nodes.iter().flat_map(|(id, n)| n.inputs.keys().map(move |k| format!("{id}.{k}"))).map(|k| self.lkey(&k)).collect();
        let scope = self.scope_kind().map(|k| format!("{k}/"));
        let here = |k: &str| match &scope {
            Some(p) => k.starts_with(p.as_str()),
            None => !k.contains('/'),
        };
        self.layout.reroutes.retain(|k, _| !here(k) || keep.contains(k));
        self.selected.retain(|s| !ids.contains(s));
        let crumbs = self.crumbs();
        if self.viewed.as_ref().is_some_and(|v| v.0 == crumbs && ids.contains(&v.1.node)) {
            self.viewed = None;
        }
    }

    /// Feeds `to`'s `input` from `node`'s output `port`.
    pub fn connect(&mut self, node: &str, port: u8, to: &str, input: &str) {
        let Some(shape) = self.shape(node) else { return };
        let out = shape.outputs.get(port as usize).map_or("", |p| p.name.as_str()).to_owned();
        // Types nothing converts are never linked: why is said.
        if let (Some(from), Some(to_kind)) = (shape.outputs.get(port as usize).map(|o| o.kind), self.shape(to).and_then(|s| s.input_kind(input)))
            && from.conversion(to_kind).is_none()
            && !self.is_column(to)
        {
            self.notice = Some((super::canvas::refusal(from, to_kind), 0.0));
            return;
        }
        let src = if port == 0 && !self.is_column(node) { node.to_owned() } else { format!("{node}.{out}") };
        if self.is_column(to) {
            self.set_output(input, &src);
            return;
        }
        if self.is_column(node) {
            // From an input of the composite (`in.<name>`): a step inside it,
            // where its inputs are known.
            let r = self.op_inside(massif_terrain::format::Op::Link { from: src, to: to.to_owned(), input: input.to_owned() });
            if let Err(e) = r {
                self.notice = Some((e, 0.0));
            }
            return;
        }
        let r = self.op(massif_terrain::format::Op::Link { from: src, to: to.to_owned(), input: input.to_owned() });
        if let Err(e) = r {
            self.notice = Some((e, 0.0));
        }
    }

    pub fn disconnect(&mut self, to: &str, input: &str) {
        if self.is_column(to) {
            self.set_output(input, "");
            return;
        }
        let key = self.lkey(&format!("{to}.{input}"));
        if self.op(massif_terrain::format::Op::Unlink { to: to.to_owned(), input: input.to_owned() }).is_ok() {
            self.layout.reroutes.remove(&key);
        }
    }

    /// Makes the composite open's output `name` come from `src` inside
    /// (`""`: nothing).
    pub fn set_output(&mut self, name: &str, src: &str) {
        let Some(kind) = self.scope_kind().map(base_kind).map(str::to_owned) else { return };
        if self.read_only() {
            return;
        }
        let Some(label) = self.graph.components.get(&kind).and_then(|c| c.outputs.iter().find(|o| o.name == name)).map(|o| o.label.clone()) else { return };
        if src.is_empty() {
            // Left empty, waiting for another wire: no step of the graph
            // empties an output, so done in place, one step undone as any.
            let g = self.edit();
            if let Some(o) = g.components.get_mut(&kind).and_then(|c| c.outputs.iter_mut().find(|o| o.name == name)) {
                o.from.clear();
            }
            return;
        }
        let decl = massif_terrain::model::OutputDecl { name: name.to_owned(), label, from: src.to_owned() };
        if let Err(e) = self.doc_op(massif_terrain::format::Op::Output { component: kind, decl }) {
            self.notice = Some((e, 0.0));
        }
    }

    /// A step inside the composite open, as the graph does it there (its
    /// inputs known as `in.<name>`).
    pub fn op_inside(&mut self, op: massif_terrain::format::Op) -> Result<String, String> {
        let Some(kind) = self.scope_kind().map(base_kind).map(str::to_owned) else { return self.op(op) };
        self.doc_op(massif_terrain::format::Op::Inside { component: kind, ops: vec![op] })
    }

    /// One of the graph's steps on the whole document (a composite's
    /// inputs, outputs and exposed settings, which its nodes everywhere
    /// follow), the composites open staying open.
    pub fn doc_op(&mut self, op: massif_terrain::format::Op) -> Result<String, String> {
        if self.read_only() {
            return Err(t!("Massif's composites are read only: edit a copy").to_owned());
        }
        let mut doc = self.document();
        let done = massif_terrain::format::apply(&mut doc, &op)?;
        self.edit();
        let crumbs = self.crumbs();
        let views: Vec<View> = self.outer.iter().map(|o| o.view).collect();
        let view = self.view;
        self.graph = doc;
        self.outer.clear();
        self.reopen(&crumbs);
        for (o, v) in self.outer.iter_mut().zip(views) {
            o.view = v;
        }
        self.view = view;
        self.scope_revision += 1;
        self.selected.retain(|s| self.graph.nodes.contains_key(s));
        Ok(done)
    }

    /// The node and output feeding `to`'s `input`.
    pub fn source_of(&self, to: &str, input: &str) -> Option<(String, u8)> {
        let src = if self.is_column(to) {
            let kind = self.scope_kind()?;
            massif_terrain::lower::component(&self.graph, kind)?.outputs.iter().find(|o| o.name == input)?.from.clone()
        } else {
            self.graph.nodes.get(to)?.inputs.get(input)?.clone()
        };
        let s = massif_terrain::model::Source::parse(&src);
        let shape = self.shape(s.node)?;
        let port = match s.output {
            Some(o) => shape.output(o)? as u8,
            None => 0,
        };
        Some((s.node.to_owned(), port))
    }

    /// The type an output of a node carries.
    pub fn out_kind(&self, node: &str, port: u8) -> Option<PortKind> {
        self.shape(node)?.outputs.get(port as usize).map(|p| p.kind)
    }

    /// The active Map Output (of the document's own nodes).
    pub fn active_output(&self) -> Option<String> {
        let nodes = self.root_nodes();
        let outs: Vec<&String> = nodes.iter().filter(|(_, n)| n.kind == "output.map").map(|(id, _)| id).collect();
        outs.iter().find(|id| nodes[**id].params.get("active").and_then(|v| v.as_bool()) != Some(false)).or(outs.first()).map(|s| (*s).clone())
    }

    /// Makes `id` the active Map Output, the others not.
    pub fn set_active_output(&mut self, id: &str) {
        let outs: Vec<String> = self.graph.nodes.iter().filter(|(_, n)| n.kind == "output.map").map(|(k, _)| k.clone()).collect();
        let ops = outs.into_iter().map(|o| massif_terrain::format::Op::Set { value: (o == id).into(), node: o, param: "active".into() }).collect();
        let _ = self.ops(ops);
    }

    /// Places nodes that have no place yet, column by column by depth.
    pub fn auto_place(&mut self) {
        let ids: Vec<String> = self.graph.nodes.keys().filter(|k| !self.graph.ui.contains_key(*k)).cloned().collect();
        if ids.is_empty() {
            return;
        }
        let mut depth: HashMap<String, usize> = HashMap::new();
        fn depth_of(g: &Graph, id: &str, memo: &mut HashMap<String, usize>, guard: usize) -> usize {
            if let Some(d) = memo.get(id) {
                return *d;
            }
            if guard > 64 {
                return 0;
            }
            let d = g.nodes.get(id).map_or(0, |n| n.inputs.values().map(|s| depth_of(g, massif_terrain::model::Source::parse(s).node, memo, guard + 1) + 1).max().unwrap_or(0));
            memo.insert(id.to_owned(), d);
            d
        }
        for id in self.graph.nodes.keys() {
            depth_of(&self.graph, id, &mut depth, 0);
        }
        let mut rows: HashMap<usize, f32> = HashMap::new();
        let mut order: Vec<&String> = self.graph.nodes.keys().collect();
        order.sort_by_key(|id| depth[*id]);
        let mut place = Vec::new();
        for id in order {
            let d = depth[id];
            let y = rows.entry(d).or_insert(0.0);
            let h = super::canvas::node_size(self, id).y;
            if ids.contains(id) {
                place.push((id.clone(), [d as f32 * 212.0, *y]));
            }
            *y += h + 22.0;
        }
        for (id, pos) in place {
            self.graph.ui.insert(id, NodeUi { pos, collapsed: false });
        }
    }

    // ---- Previews ----------------------------------------------------

    /// What the viewer shows, as the compiled graph names it: the node
    /// viewed, or the active Map Output.
    pub fn shown(&self) -> Option<Shown> {
        if let Some((crumbs, v)) = &self.viewed {
            let doc = self.document();
            if let Some(s) = lowered(&doc, crumbs, v) {
                return Some(s);
            }
        }
        self.active_output().map(|node| Shown { node, port: None })
    }

    /// What the thumbnail of node `id` of the graph shown is of.
    pub fn thumb_key(&self, id: &str) -> Option<&Shown> {
        self.thumb_keys.get(id)
    }

    /// The engine's counts of what each node made: tiles and milliseconds.
    fn engine_counts(&self) -> HashMap<String, (u64, f64)> {
        self.engine().map(|e| e.counters.by_node.lock().map(|m| m.clone()).unwrap_or_default()).unwrap_or_default()
    }

    /// What was made since the graph last changed, as each node's cost.
    fn track_costs(&mut self) {
        for (id, (tiles, ms)) in self.engine_counts() {
            let (t0, m0) = self.cost_start.get(&id).copied().unwrap_or_default();
            if tiles > t0 || ms > m0 + 0.01 {
                self.costs.insert(id, (ms - m0, tiles - t0));
            }
        }
    }

    /// What node `id` of the graph shown last cost to make (a composite:
    /// everything inside it): milliseconds, and the memory of the tiles it
    /// made, in bytes. `None` before it was made.
    pub fn cost(&self, id: &str) -> Option<(f64, u64)> {
        let me = super::shape::unfolded_id(&self.crumbs(), id);
        let inside = format!("{me}{}", massif_terrain::lower::SEP);
        let planes = self.shape(id).map_or(1, |s| s.outputs.iter().map(|o| o.kind.planes()).sum::<usize>().max(1)) as u64;
        let tile = (massif_terrain::TILE as u64).pow(2) * 4;
        let mut found = None;
        for (k, (ms, tiles)) in &self.costs {
            if *k == me || k.starts_with(&inside) {
                let (m, b) = found.unwrap_or((0.0, 0));
                found = Some((m + ms, b + tiles * tile * if *k == me { planes } else { 1 }));
            }
        }
        found
    }

    /// Takes the pictures made, and asks for those wanted.
    fn update_previews(&mut self, ctx: &egui::Context, viewer_px: [usize; 2], thumb_px: usize) {
        if self.previews.is_none() {
            return;
        }
        self.track_costs();
        for mut p in self.previews.as_ref().map(|p| p.take()).unwrap_or_default() {
            if let Some(relief) = p.relief.take() {
                if p.viewer {
                    self.view3d.set(relief);
                    self.viewer3d = Some(p);
                }
                continue;
            }
            let image = egui::ColorImage::from_rgba_unmultiplied(p.size, &p.rgba);
            if p.viewer {
                match &mut self.viewer {
                    Some((tex, old)) => {
                        tex.set(image, egui::TextureOptions::LINEAR);
                        *old = Picture { rgba: Vec::new(), ..p };
                    }
                    None => self.viewer = Some((ctx.load_texture("terrain viewer", image, egui::TextureOptions::LINEAR), Picture { rgba: Vec::new(), ..p })),
                }
            } else {
                let key = p.what.clone();
                match self.thumbs.get_mut(&key) {
                    Some((tex, old)) => {
                        tex.set(image, egui::TextureOptions::LINEAR);
                        *old = Picture { rgba: Vec::new(), ..p };
                    }
                    None => {
                        let tex = ctx.load_texture(format!("terrain thumb {}", key.node), image, egui::TextureOptions::LINEAR);
                        self.thumbs.insert(key, (tex, Picture { rgba: Vec::new(), ..p }));
                    }
                }
            }
        }
        let doc = if self.outer.is_empty() { None } else { Some(self.document()) };
        let doc = doc.as_ref().unwrap_or(&self.graph);
        // What each node's thumbnail is of, once composites unfold.
        if self.keys_for != (self.revision, self.scope_revision) {
            self.keys_for = (self.revision, self.scope_revision);
            let crumbs = self.crumbs();
            self.thumb_keys = self
                .graph
                .nodes
                .keys()
                .filter_map(|id| {
                    let is_output = self.graph.nodes[id].kind.starts_with("output.");
                    let key = if is_output && crumbs.is_empty() { Some(Shown { node: id.clone(), port: None }) } else { lowered(doc, &crumbs, &Shown { node: id.clone(), port: None }) };
                    key.map(|k| (id.clone(), k))
                })
                .collect();
        }
        let wanted: std::collections::HashSet<&Shown> = self.thumb_keys.values().collect();
        // Thumbnails of nodes still in the document are kept (another scope
        // shows them again).
        let mut all: BTreeSet<String> = doc.nodes.keys().cloned().collect();
        all.extend(self.thumb_keys.values().map(|s| s.node.clone()));
        self.thumbs.retain(|k, _| wanted.contains(k) || all.contains(&k.node) || k.node.contains(massif_terrain::lower::SEP));
        let mut look = self.look;
        look.size = viewer_px;
        if look.three_d {
            // The 3D view lights and frames the relief itself: only what
            // makes the relief asks for it again.
            look = ViewerLook { three_d: true, surface: look.surface, relief: look.relief, overlay: look.overlay, size: look.size, resolution: look.resolution, ..ViewerLook::default() };
        }
        let shown = self.shown();
        let nodes = self.graph.nodes.len();
        let key = (self.revision, shown.clone(), look, self.thumb_surface, nodes, self.space, thumb_px, self.env_revision, self.scope_revision);
        if self.asked.as_ref() == Some(&key) {
            return;
        }
        let graph_changed = self.asked.as_ref().is_none_or(|a| a.0 != self.revision || a.3 != self.thumb_surface || a.4 != nodes || a.5 != self.space || a.6 != thumb_px || a.7 != self.env_revision || a.8 != self.scope_revision);
        let doc_changed = self.asked.as_ref().is_none_or(|a| a.0 != self.revision || a.5 != self.space || a.7 != self.env_revision);
        self.asked = Some(key);
        if graph_changed {
            // What is made from now on is this version's cost.
            self.cost_start = self.engine_counts();
        }
        self.asks += 1;
        let mut thumbs = Vec::new();
        if graph_changed {
            let first: Vec<String> = self.selected.clone();
            let mut ids: Vec<&String> = self.graph.nodes.keys().collect();
            ids.sort_by_key(|id| !first.contains(id));
            for id in ids {
                if self.shape(id).is_some_and(|s| s.is_output() || !s.outputs.is_empty())
                    && let Some(k) = self.thumb_keys.get(id)
                    && !thumbs.contains(k)
                {
                    thumbs.push(k.clone());
                }
            }
        }
        // Live compiles the graph the map shows; previews of the same graph
        // evaluate that compiled graph, so that they find Live's tiles.
        let mut compiled = None;
        if let Some(live) = self.live.as_ref().filter(|_| self.live_on) {
            if doc_changed {
                live.set_graph(doc, self.space, self.env.clone(), self.revision);
            }
            compiled = live.compiled();
        }
        let request = Request {
            revision: self.asks,
            graph: doc.clone(),
            space: self.space,
            sim_res: self.sim_res,
            env: self.env.clone(),
            thumbs,
            thumb_px,
            compiled,
            thumb_surface: self.thumb_surface,
            viewer: shown.map(|s| (s, look)),
            categories: self.categories.clone(),
        };
        if let Some(previews) = &self.previews {
            previews.ask(request, ctx);
        }
        if doc_changed {
            self.events.push(Event::Changed);
        }
    }

    // ---- Layout ------------------------------------------------------

    /// The whole panel, in `ui`'s room.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.events.clear();
        self.docked = false;
        let pal = Palette::of(ui);
        let full = ui.available_rect_before_wrap();
        ui.painter().rect_filled(full, 0.0, pal.app_bg);
        let gap = 6.0;
        let status_h = 22.0;
        let wide = full.width() >= 980.0;
        let lib_w = if wide { 214.0 } else { 0.0 };
        let props_w = if full.width() >= 700.0 { 300.0f32.min(full.width() * 0.3) } else { 0.0 };
        let body = Rect::from_min_max(full.min + vec2(gap, gap), full.max - vec2(gap, gap + status_h));
        let lib = Rect::from_min_size(body.min, vec2(lib_w, body.height()));
        let props = Rect::from_min_max(pos2(body.right() - props_w, body.top()), body.max);
        let mid_left = if wide { lib.right() + gap } else { body.left() };
        let mid_right = if props_w > 0.0 { props.left() - gap } else { body.right() };
        let mid = Rect::from_min_max(pos2(mid_left, body.top()), pos2(mid_right, body.bottom()));
        let viewer_h = (mid.height() * self.split).clamp(140.0, mid.height() - 160.0).max(80.0);
        let viewer = Rect::from_min_size(mid.min, vec2(mid.width(), viewer_h));
        let below = Rect::from_min_max(pos2(mid.left(), viewer.bottom() + gap), mid.max);
        // The Script pane beside the graph, or in its place when narrow.
        let (graph, script) = match (self.script.open, below.width() >= 640.0) {
            (false, _) => (below, None),
            (true, true) => {
                let w = (below.width() * 0.46).clamp(300.0, 620.0);
                (Rect::from_min_max(below.min, pos2(below.right() - w - gap, below.bottom())), Some(Rect::from_min_max(pos2(below.right() - w, below.top()), below.max)))
            }
            (true, false) => (Rect::NOTHING, Some(below)),
        };
        let status = Rect::from_min_max(pos2(full.left() + gap + 4.0, body.bottom() + 2.0), pos2(full.right() - gap, full.bottom()));

        // The divider between the viewer and the graph, dragged.
        let divider = Rect::from_min_max(pos2(mid.left(), viewer.bottom()), pos2(mid.right(), graph.top()));
        let d = ui.interact(divider, ui.scope_id().with("terrain split"), egui::Sense::drag());
        if d.hovered() || d.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        if d.dragged() {
            self.split = ((viewer.height() + d.drag_delta().y) / mid.height()).clamp(0.2, 0.8);
        }

        self.hovered_help = None;
        let px = ui.ctx().pixels_per_point();
        let viewer_px = Self::viewer_pixels(viewer, px);
        if lib_w > 0.0 {
            card(ui, lib, |ui| super::library::ui(self, ui));
        }
        card(ui, viewer, |ui| super::viewer::ui(self, ui));
        if graph.is_positive() {
            card(ui, graph, |ui| super::canvas::ui(self, ui));
        }
        if let Some(r) = script {
            card(ui, r, |ui| super::script::ui(self, ui));
        }
        if props_w > 0.0 {
            card(ui, props, |ui| super::props::ui(self, ui));
        }
        super::props::expose_dialog(self, ui);
        self.status_ui(ui, status);
        self.after_frame(ui.ctx(), viewer_px);
    }

    fn status_ui(&self, ui: &mut Ui, rect: Rect) {
        let pal = Palette::of(ui);
        let s = self.status();
        let mut parts = Vec::new();
        if s.busy {
            parts.push(t!("Computing: {what}", what = s.doing));
        } else if let Some((node, e)) = &s.error {
            parts.push(t!("{node}: {why}", node = node, why = e));
        } else {
            parts.push(t!("Ready").to_owned());
        }
        if s.last_ms > 0.0 {
            parts.push(t!("last {ms} ms", ms = format!("{:.0}", s.last_ms)));
        }
        parts.push(match &s.gpu {
            Some(g) => t!("GPU: {name}", name = g),
            None => t!("CPU").to_owned(),
        });
        parts.push(t!("cache {mb} MB", mb = format!("{:.0}", s.cache_mb)));
        parts.push(t!("terrain {size} blocks", size = self.space.size));
        let text = parts.join("  ·  ");
        let color = if s.error.is_some() { pal.danger } else { pal.weak };
        ui.painter().with_clip_rect(rect).text(rect.left_center(), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(11.5), color);
    }
}

/// A composite's type without its version (`project:hills@2` →
/// `project:hills`), as the document's `components` keys it.
pub fn base_kind(kind: &str) -> &str {
    kind.split('@').next().unwrap_or(kind)
}

/// `what`, a node and output of the scope `crumbs` leads to, as the
/// compiled graph names it once composites unfold and bypassed nodes step
/// aside.
pub fn lowered(doc: &Graph, crumbs: &[Crumb], what: &Shown) -> Option<Shown> {
    let nodes = shape::scope_nodes(doc, crumbs)?;
    let node = nodes.get(&what.node)?;
    if node.kind.starts_with("output.") {
        return crumbs.is_empty().then(|| what.clone());
    }
    let path = shape::unfolded_id(crumbs, &what.node);
    let src = match &what.port {
        Some(p) => format!("{path}.{p}"),
        None => path,
    };
    let made = massif_terrain::lower::resolve(doc, &src)?;
    let s = massif_terrain::model::Source::parse(&made);
    Some(Shown { node: s.node.to_owned(), port: s.output.map(str::to_owned) })
}

/// A card of the panel: the panel's fill, a hairline, rounded; `add` in it.
pub fn card(ui: &mut Ui, rect: Rect, add: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    ui.painter().rect(rect, theme::CARD_RADIUS, pal.panel, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::top_down(egui::Align::Min)));
    child.set_clip_rect(rect.intersect(ui.clip_rect()));
    add(&mut child);
}

/// A card's title, at its top left.
pub fn title(ui: &mut Ui, rect: Rect, text: &str) {
    let pal = Palette::of(ui);
    ui.painter().text(rect.min + vec2(theme::PAD, 18.0), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(13.0), pal.text);
}
