//! The panel's state and layout: the library on the left, the viewer over
//! the graph in the middle, the selected node's settings on the right, a
//! status line under them all.

use super::eval::{self, Categories, Picture, Previews, Request, Shown, ViewerLook};
use super::shade::Surface;
use crate::theme::{self, Palette};
use egui::{Rect, TextureHandle, Ui, Vec2, pos2, vec2};
use massif_terrain::engine::Engine;
use massif_terrain::kernel::Env;
use massif_terrain::live::Live;
use massif_terrain::model::{Graph, Node, NodeUi};
use massif_terrain::registry::{self, NodeSpec, PortKind};
use massif_terrain::space::Space;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::sync::Arc;

/// What the previews were last asked for: the graph's revision, the node
/// viewed, the viewer's look, the thumbnails' surface, the number of
/// nodes, the terrain's square, the thumbnails' size, the environment's
/// revision (the map read by Map nodes).
type Asked = (u64, Option<Shown>, ViewerLook, Surface, usize, Space, usize, u64);

/// A frame around nodes, with a title.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub title: String,
    /// In graph units: x, y, width, height.
    pub rect: [f32; 4],
    /// sRGB, or the neutral grey when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<[u8; 3]>,
}

/// What the editor keeps of a graph beside its nodes: frames, reroute
/// points on wires, settings shown as sockets, folded groups.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Layout {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub frames: Vec<Frame>,
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

pub struct TerrainPanel {
    pub graph: Graph,
    pub layout: Layout,
    /// Bumped whenever the graph changes.
    pub revision: u64,
    pub space: Space,
    pub sim_res: u32,
    pub env: Arc<dyn Env>,
    pub categories: Arc<Categories>,

    pub selected: Vec<String>,
    pub viewed: Option<Shown>,
    pub view: View,
    pub wiring: Option<Wiring>,
    pub search: Option<Search>,
    pub library_query: String,
    pub library_open: BTreeSet<&'static str>,
    pub marquee: Option<egui::Pos2>,
    /// The setting under the pointer, whose help the panel's foot shows.
    pub hovered_help: Option<&'static str>,
    pub show_advanced: bool,
    /// The frame whose title is being edited.
    pub renaming_frame: Option<usize>,
    pub renaming_node: Option<String>,
    pub renaming_mask: Option<(String, String)>,

    undo: Vec<(Graph, Layout)>,
    redo: Vec<(Graph, Layout)>,
    /// An edit under way (a drag, a slider): one undo step for all of it.
    editing: bool,

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
}

impl TerrainPanel {
    pub fn new(engine: Arc<Engine>) -> Self {
        Self {
            graph: Graph::default(),
            layout: Layout::default(),
            revision: 1,
            space: Space::new(8192),
            sim_res: 1024,
            env: eval::no_env(),
            categories: Arc::new(Categories::of(&massif_core::registry::Registry::vanilla())),
            selected: Vec::new(),
            viewed: None,
            view: View::default(),
            wiring: None,
            search: None,
            library_query: String::new(),
            library_open: BTreeSet::from(["Shapes"]),
            marquee: None,
            hovered_help: None,
            show_advanced: false,
            renaming_frame: None,
            renaming_node: None,
            renaming_mask: None,
            undo: Vec::new(),
            redo: Vec::new(),
            editing: false,
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
            written: None,
            dirty: false,
            own_undo_keys: true,
            env_key: None,
            env_at: None,
            env_revision: 0,
            doc_seen: 0,
            map_revision: 0,
            notes: Vec::new(),
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
        if let Some(l) = &self.live {
            let mut s = l.settings();
            s.mode = if on { massif_terrain::live::Mode::Visible } else { massif_terrain::live::Mode::Off };
            l.set_settings(s);
            if on {
                l.set_graph(&self.graph, self.space, self.env.clone(), self.revision);
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
        self.auto_place();
        self.undo.clear();
        self.redo.clear();
        self.selected.clear();
        self.viewed = None;
        self.thumbs.clear();
        self.viewer = None;
        self.revision += 1;
        match self.layout.view {
            Some([x, y, z]) => {
                self.view = View { offset: vec2(x, y), zoom: z.clamp(0.25, 2.5) };
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
        let mut space = massif_terrain::world::space_of(&doc.world, doc.dim, &self.graph);
        space.seed = self.graph.terrain.seed;
        if let Some(sea) = self.graph.terrain.sea_level {
            space.sea_level = sea;
        }
        self.space = space;
        // Another change to the document than the panel's own: the map may
        // have changed under the graph's Map nodes.
        if doc.revision != self.doc_seen {
            self.doc_seen = doc.revision;
            self.map_revision += 1;
        }
        let has_map = self.graph.nodes.values().any(|n| n.kind == "input.map");
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

    /// The graph as saved: with its layout and the canvas's view.
    pub fn saved(&self) -> Graph {
        let mut g = self.graph.clone();
        let mut l = self.layout.clone();
        l.view = Some([self.view.offset.x, self.view.offset.y, self.view.zoom]);
        g.editor = l.value();
        g
    }

    /// The panel in the app: the document's graph, edited, written back.
    pub fn ui_in(&mut self, ui: &mut Ui, doc: &mut crate::doc::Document) {
        self.own_undo_keys = false;
        self.sync(doc);
        self.ui(ui);
        self.commit(doc);
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

    // ---- Editing, with undo ------------------------------------------

    /// The graph to change: the first change of an edit records an undo
    /// step.
    pub fn edit(&mut self) -> &mut Graph {
        if !self.editing {
            self.undo.push((self.graph.clone(), self.layout.clone()));
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
            self.undo.push((self.graph.clone(), self.layout.clone()));
            self.redo.clear();
            self.editing = true;
        }
        self.dirty = true;
        &mut self.layout
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Undoes the last change; whether there was one.
    pub fn undo(&mut self) -> bool {
        let Some((g, l)) = self.undo.pop() else { return false };
        self.redo.push((std::mem::replace(&mut self.graph, g), std::mem::replace(&mut self.layout, l)));
        self.revision += 1;
        self.dirty = true;
        self.editing = false;
        self.selected.retain(|s| self.graph.nodes.contains_key(s));
        true
    }

    /// Redoes what was undone; whether there was something.
    pub fn redo(&mut self) -> bool {
        let Some((g, l)) = self.redo.pop() else { return false };
        self.undo.push((std::mem::replace(&mut self.graph, g), std::mem::replace(&mut self.layout, l)));
        self.revision += 1;
        self.dirty = true;
        self.editing = false;
        self.selected.retain(|s| self.graph.nodes.contains_key(s));
        true
    }

    // ---- The graph ----------------------------------------------------

    pub fn spec(&self, id: &str) -> Option<&'static NodeSpec> {
        self.graph.nodes.get(id).and_then(|n| registry::spec(&n.kind))
    }

    pub fn pos(&self, id: &str) -> [f32; 2] {
        self.graph.ui.get(id).map_or([0.0, 0.0], |u| u.pos)
    }

    pub fn collapsed(&self, id: &str) -> bool {
        self.graph.ui.get(id).is_some_and(|u| u.collapsed)
    }

    /// Adds a node of type `kind` at `at`, selected; gives its name. A
    /// type named without its version gets its latest one.
    pub fn add_node(&mut self, kind: &str, at: [f32; 2]) -> String {
        let kind = if kind.contains('@') { kind.to_owned() } else { registry::latest(kind).map_or_else(|| kind.to_owned(), |s| s.type_name()) };
        let g = self.edit();
        let id = g.add(Node::new(&kind));
        g.ui.insert(id.clone(), NodeUi { pos: at, collapsed: false });
        self.selected = vec![id.clone()];
        id
    }

    /// Removes nodes, and the wires to them.
    pub fn remove_nodes(&mut self, ids: &[String]) {
        if ids.is_empty() {
            return;
        }
        let g = self.edit();
        for id in ids {
            g.nodes.remove(id);
            g.ui.remove(id);
        }
        for n in g.nodes.values_mut() {
            n.inputs.retain(|_, src| !ids.iter().any(|id| massif_terrain::model::Source::parse(src).node == id));
        }
        let keep: Vec<String> = self.graph.nodes.iter().flat_map(|(id, n)| n.inputs.keys().map(move |k| format!("{id}.{k}"))).collect();
        self.layout.reroutes.retain(|k, _| keep.contains(k));
        self.selected.retain(|s| !ids.contains(s));
        if self.viewed.as_ref().is_some_and(|v| ids.contains(&v.node)) {
            self.viewed = None;
        }
    }

    /// Copies of nodes, beside them, wired among themselves as they were.
    pub fn duplicate(&mut self, ids: &[String]) {
        let mut names = HashMap::new();
        let copies: Vec<(String, Node, [f32; 2])> = ids.iter().filter_map(|id| self.graph.nodes.get(id).map(|n| (id.clone(), n.clone(), self.pos(id)))).collect();
        let g = self.edit();
        for (id, node, pos) in &copies {
            let new = g.add(node.clone());
            g.ui.insert(new.clone(), NodeUi { pos: [pos[0] + 40.0, pos[1] + 40.0], collapsed: false });
            names.insert(id.clone(), new);
        }
        for new in names.values() {
            if let Some(n) = g.nodes.get_mut(new) {
                for src in n.inputs.values_mut() {
                    let s = massif_terrain::model::Source::parse(src);
                    if let Some(to) = names.get(s.node) {
                        *src = match s.output {
                            Some(o) => format!("{to}.{o}"),
                            None => to.clone(),
                        };
                    }
                }
            }
        }
        self.selected = names.into_values().collect();
    }

    /// Renames a node, and what names it.
    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        if from == to || !massif_terrain::model::valid_id(to) || self.graph.nodes.contains_key(to) {
            return false;
        }
        let g = self.edit();
        let Some(node) = g.nodes.remove(from) else { return false };
        g.nodes.insert(to.to_owned(), node);
        if let Some(u) = g.ui.remove(from) {
            g.ui.insert(to.to_owned(), u);
        }
        for n in g.nodes.values_mut() {
            for src in n.inputs.values_mut() {
                let s = massif_terrain::model::Source::parse(src);
                if s.node == from {
                    *src = match s.output {
                        Some(o) => format!("{to}.{o}"),
                        None => to.to_owned(),
                    };
                }
            }
        }
        let moved: Vec<(String, Vec<[f32; 2]>)> = self.layout.reroutes.iter().filter(|(k, _)| k.split('.').next() == Some(from)).map(|(k, v)| (k.clone(), v.clone())).collect();
        for (k, v) in moved {
            self.layout.reroutes.remove(&k);
            self.layout.reroutes.insert(format!("{to}{}", &k[from.len()..]), v);
        }
        for s in self.selected.iter_mut().filter(|s| *s == from) {
            *s = to.to_owned();
        }
        if let Some(v) = self.viewed.as_mut().filter(|v| v.node == from) {
            v.node = to.to_owned();
        }
        true
    }

    /// Feeds `to`'s `input` from `node`'s output `port`.
    pub fn connect(&mut self, node: &str, port: u8, to: &str, input: &str) {
        let Some(spec) = self.spec(node) else { return };
        let out = spec.outputs.get(port as usize).map_or("", |p| p.name);
        let src = if port == 0 { node.to_owned() } else { format!("{node}.{out}") };
        let g = self.edit();
        g.link(&src, to, input);
    }

    pub fn disconnect(&mut self, to: &str, input: &str) {
        let key = format!("{to}.{input}");
        self.edit().nodes.get_mut(to).map(|n| n.inputs.remove(input));
        self.layout.reroutes.remove(&key);
    }

    /// The node and output feeding `to`'s `input`.
    pub fn source_of(&self, to: &str, input: &str) -> Option<(String, u8)> {
        let src = self.graph.nodes.get(to)?.inputs.get(input)?;
        let s = massif_terrain::model::Source::parse(src);
        let spec = self.spec(s.node)?;
        let port = match s.output {
            Some(o) => spec.output(o)? as u8,
            None => 0,
        };
        Some((s.node.to_owned(), port))
    }

    /// The type an output of a node carries.
    pub fn out_kind(&self, node: &str, port: u8) -> Option<PortKind> {
        self.spec(node)?.outputs.get(port as usize).map(|p| p.kind)
    }

    /// The active Map Output.
    pub fn active_output(&self) -> Option<String> {
        let outs: Vec<&String> = self.graph.nodes.iter().filter(|(_, n)| n.kind == "output.map").map(|(id, _)| id).collect();
        outs.iter().find(|id| self.graph.nodes[**id].params.get("active").and_then(|v| v.as_bool()) != Some(false)).or(outs.first()).map(|s| (*s).clone())
    }

    /// Makes `id` the active Map Output, the others not.
    pub fn set_active_output(&mut self, id: &str) {
        let outs: Vec<String> = self.graph.nodes.iter().filter(|(_, n)| n.kind == "output.map").map(|(k, _)| k.clone()).collect();
        let g = self.edit();
        for o in outs {
            g.set(&o, "active", o == id);
        }
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

    /// What the viewer shows: the node viewed, or the active Map Output.
    pub fn shown(&self) -> Option<Shown> {
        self.viewed.clone().filter(|v| self.graph.nodes.contains_key(&v.node)).or_else(|| self.active_output().map(|node| Shown { node, port: None }))
    }

    /// Takes the pictures made, and asks for those wanted.
    fn update_previews(&mut self, ctx: &egui::Context, viewer_px: [usize; 2], thumb_px: usize) {
        let Some(previews) = &self.previews else { return };
        for p in previews.take() {
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
        self.thumbs.retain(|k, _| self.graph.nodes.contains_key(&k.node));
        let mut look = self.look;
        look.size = viewer_px;
        let shown = self.shown();
        let nodes = self.graph.nodes.len();
        let key = (self.revision, shown.clone(), look, self.thumb_surface, nodes, self.space, thumb_px, self.env_revision);
        if self.asked.as_ref() == Some(&key) {
            return;
        }
        let graph_changed = self.asked.as_ref().is_none_or(|a| a.0 != self.revision || a.3 != self.thumb_surface || a.4 != nodes || a.5 != self.space || a.6 != thumb_px || a.7 != self.env_revision);
        self.asked = Some(key);
        self.asks += 1;
        let mut thumbs = Vec::new();
        if graph_changed {
            let first: Vec<String> = self.selected.clone();
            let mut ids: Vec<&String> = self.graph.nodes.keys().collect();
            ids.sort_by_key(|id| !first.contains(id));
            for id in ids {
                if self.spec(id).is_some_and(|s| s.is_output() || !s.outputs.is_empty()) {
                    thumbs.push(Shown { node: id.clone(), port: None });
                }
            }
        }
        // Live compiles the graph the map shows; previews of the same graph
        // evaluate that compiled graph, so that they find Live's tiles.
        let mut compiled = None;
        if let Some(live) = self.live.as_ref().filter(|_| self.live_on) {
            if graph_changed {
                live.set_graph(&self.graph, self.space, self.env.clone(), self.revision);
            }
            compiled = live.compiled();
        }
        previews.ask(
            Request {
                revision: self.asks,
                graph: self.graph.clone(),
                space: self.space,
                sim_res: self.sim_res,
                env: self.env.clone(),
                thumbs,
                thumb_px,
                compiled,
                thumb_surface: self.thumb_surface,
                viewer: shown.map(|s| (s, look)),
                categories: self.categories.clone(),
            },
            ctx,
        );
        if graph_changed {
            self.events.push(Event::Changed);
        }
    }

    // ---- Layout ------------------------------------------------------

    /// The whole panel, in `ui`'s room.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.events.clear();
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
        let graph = Rect::from_min_max(pos2(mid.left(), viewer.bottom() + gap), mid.max);
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
        let inner = super::viewer::picture_rect(viewer);
        // The viewer's pixels, at most 1600 × 1200, keeping its shape.
        let (vw, vh) = (inner.width().max(1.0) * px, inner.height().max(1.0) * px);
        let k = (1600.0 / vw).min(1200.0 / vh).min(1.0);
        let viewer_px = [((vw * k) as usize).max(64), ((vh * k) as usize).max(64)];
        if lib_w > 0.0 {
            card(ui, lib, |ui| super::library::ui(self, ui, graph));
        }
        card(ui, viewer, |ui| super::viewer::ui(self, ui));
        card(ui, graph, |ui| super::canvas::ui(self, ui));
        if props_w > 0.0 {
            card(ui, props, |ui| super::props::ui(self, ui));
        }
        self.status_ui(ui, status);
        let thumb_px = if px > 1.5 { 256 } else { 128 };
        self.update_previews(ui.ctx(), viewer_px, thumb_px);

        // An edit ends when nothing is held.
        let held = ui.input(|i| i.pointer.any_down()) || ui.ctx().memory(|m| m.focused().is_some());
        if !held {
            self.editing = false;
        }
        if !self.previews_idle() && !theme::reduce_motion(ui.ctx()) {
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(50));
        }
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
