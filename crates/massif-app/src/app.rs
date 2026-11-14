//! The application: window chrome, workspaces and areas, dialogs, jobs.

use crate::blocks::BlockLibrary;
use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
use crate::home::{self, HomeAction, Page};
use crate::jobs::{self, Job, Outcome};
use crate::layout::{AreaId, Dir, EditorKind, Layout, Node, Zone};
use crate::map_render::MapCache;
use crate::map_view::{MapInput, MapView};
use crate::panels;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::{Tool, ToolState};
use crate::view3d::View3d;
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Id, Key, KeyboardShortcut, LayerId,
    Modifiers, Order, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, ViewportCommand,
    pos2, vec2,
};
use massif_anvil::{ExportOptions, GameMode, Surroundings};
use massif_core::generate::{GenParams, generate};
use massif_core::{TILE_SIZE, TilePos, World};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const HEADER: f32 = 32.0;
const GAP: f32 = 4.0;
const MIN_AREA: f32 = 90.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
enum ThemeChoice {
    #[default]
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
struct Settings {
    theme: ThemeChoice,
    recent: Vec<PathBuf>,
    solo: bool,
    /// Where the light of the map comes from (R and L turn it).
    light: u8,
    /// The Minecraft client jar the real blocks are read from; the newest
    /// one found when unset.
    minecraft_jar: Option<PathBuf>,
    /// The accent colour: what is chosen, the main buttons, the art of the
    /// home screen.
    accent: [u8; 3],
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            recent: Vec::new(),
            solo: false,
            light: crate::map_render::DEFAULT_LIGHT,
            minecraft_jar: None,
            accent: theme::DEFAULT_ACCENT,
        }
    }
}

/// An action that replaces or closes the document, run once unsaved changes
/// are dealt with.
#[derive(Debug, Clone)]
enum Pending {
    New(GenParams),
    Open(Option<PathBuf>),
    /// A new world from a height map.
    FromHeightmap(std::sync::Arc<massif_core::heightmap::Heightmap>, massif_core::generate::FromHeightmap),
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExportTab {
    General,
    World,
}

struct ExportForm {
    tab: ExportTab,
    parent: PathBuf,
    name: String,
    game_mode: GameMode,
    difficulty: massif_anvil::Difficulty,
    hardcore: bool,
    allow_cheats: bool,
    surroundings: Surroundings,
    structures: bool,
    overwrite: bool,
    border: massif_anvil::WorldBorder,
    datapacks: Vec<PathBuf>,
    selected_pack: Option<usize>,
    spawn: [i32; 3],
    /// Centre of the test export, in blocks (the map view's centre).
    test_centre: (i32, i32),
    /// A border just around the map: centre and size.
    fit: Option<([f64; 2], f64)>,
    /// The version exported for, among those offered: the releases known,
    /// and the games installed here (marked).
    target: massif_anvil::Target,
    versions: Vec<(massif_anvil::Target, bool)>,
    /// The heights of the map, to say what a version cuts.
    heights: (i32, i32),
}

/// The versions the export offers, newest first: every release known, and
/// the games installed on this computer (true), whose own data version is
/// read from them when they are newer than the list.
fn export_versions() -> Vec<(massif_anvil::Target, bool)> {
    let mut versions: Vec<(massif_anvil::Target, bool)> = massif_anvil::Target::releases().into_iter().map(|t| (t, false)).collect();
    for jar in massif_assets::find_jars() {
        match versions.iter_mut().find(|(t, _)| t.name == jar.version) {
            Some(entry) => entry.1 = true,
            None => {
                if let Some(data_version) = massif_assets::data_version(&jar.path) {
                    versions.push((massif_anvil::Target::installed(&jar.version, data_version), true));
                }
            }
        }
    }
    versions.sort_by_key(|(t, _)| std::cmp::Reverse(t.data_version));
    versions
}

enum Dialog {
    Export(ExportForm),
    Unsaved(Pending),
    About,
    /// The theme and the accent colour.
    Preferences,
    /// The rules of the automatic biomes (false) or paint (true).
    AutoRules(bool),
    /// What an import brought in, with the file's thumbnail.
    ImportReport(Box<massif_legacy::ImportReport>, Option<egui::TextureHandle>),
}

struct Status {
    text: String,
    error: bool,
    at: f64,
    reveal: Option<PathBuf>,
}

/// A separator between the two halves of a split.
struct Sep {
    path: Vec<bool>,
    dir: Dir,
    rect: Rect,
    parent: Rect,
}

pub struct MassifApp {
    doc: Document,
    /// Whether the document is a map the user made or opened (not the one
    /// held while the home screen is shown at startup).
    has_map: bool,
    /// The home screen, shown instead of the editor.
    home: Option<home::Home>,
    /// The template the map being made takes its relief from.
    after_create: Option<massif_core::graph::Template>,
    next_generation: u64,
    layout: Layout,
    tools: ToolState,
    settings: Settings,
    maps: HashMap<AreaId, MapView>,
    map_cache: MapCache,
    view3d: View3d,
    /// The 3D Sculpt editor's view: its own camera and tiles.
    sculpt3d: View3d,
    hover: Option<(i32, i32)>,
    map_center: Option<[f64; 2]>,
    /// The map editor keyboard zooming applies to: the last one used.
    active_map: Option<AreaId>,
    jobs: Vec<Job>,
    dialog: Option<Dialog>,
    after_save: Option<Pending>,
    status: Option<Status>,
    feedback: Option<String>,
    dragging_area: Option<AreaId>,
    rename: Option<(usize, String)>,
    terrain_search: String,
    biome_search: String,
    /// The built-in, generated and custom brushes.
    brushes: BrushLibrary,
    /// Name of the brush group being made, while its field is shown.
    new_group: Option<String>,
    /// The terrain graph editor.
    graph: crate::graph_editor::GraphEditor,
    /// The game's real blocks.
    blocks: BlockLibrary,
    allow_close: bool,
    title: String,
    applied_theme: Option<ThemeChoice>,
    /// The window has no native title bar: the top bar hosts the traffic
    /// lights and moves the window.
    unified_titlebar: bool,
}

/// A world from a file: a Massif world, or a WorldPainter one imported.
fn read_any_world(path: &std::path::Path) -> Result<World, String> {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .as_deref()
    {
        Some("world") => massif_legacy::import_world(path, None)
            .map(|(w, _)| w)
            .map_err(|e| e.to_string()),
        _ => massif_format::load(path).map_err(|e| e.to_string()),
    }
}

fn default_world() -> World {
    generate(&GenParams::default())
}

impl MassifApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        theme::install(&cc.egui_ctx);
        let layout = cc
            .storage
            .and_then(|s| eframe::get_value::<Layout>(s, LAYOUT_KEY))
            .map(Layout::sanitize)
            .unwrap_or_default();
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "massif.settings"))
            .unwrap_or_default();
        let mut tools: ToolState = cc
            .storage
            .and_then(|s| eframe::get_value(s, "massif.tools"))
            .unwrap_or_default();
        // The 3D tools live in the 3D Sculpt editor now, not on the map;
        // building and digging are one tool there.
        if tools.tool.is_sculpt() {
            tools.sculpt.tool = tools.tool;
            tools.tool = Tool::Raise;
        }
        if tools.sculpt.tool == Tool::Dig {
            tools.sculpt.tool = Tool::Build;
        }
        theme::set_accent(&cc.egui_ctx, settings.accent);
        let mut app = Self::with_world(default_world(), layout, tools, settings);
        if let Some(render) = cc.storage.and_then(|s| eframe::get_value(s, "massif.render")) {
            app.graph.view.render = render;
        }
        app.unified_titlebar = cfg!(target_os = "macos");
        app.brushes = BrushLibrary::installed();
        app.brushes.load_in_background(&cc.egui_ctx);
        app.blocks.load(&cc.egui_ctx, app.settings.minecraft_jar.clone());
        if let Some(path) = open {
            app.jobs.push(Job::open(&cc.egui_ctx, path));
        } else {
            // No map yet: the home screen, not a map made up for it.
            app.has_map = false;
            app.open_home(Page::NewMap);
        }
        app
    }

    /// Pick a height map, and show on the home screen how it would become
    /// a map.
    fn pick_heightmap(&mut self, ctx: &egui::Context) {
        if let Some(path) = rfd::FileDialog::new()
            .set_title("A height map to make a map of")
            .add_filter("Height maps", &["png", "jpg", "jpeg", "r16", "r32", "raw"])
            .pick_file()
        {
            self.take_heightmap(ctx, path);
        }
    }

    /// Read the height map at `path` into the home screen.
    fn take_heightmap(&mut self, ctx: &egui::Context, path: PathBuf) {
        match home::HeightmapPick::read(ctx, path) {
            Ok(pick) => {
                self.open_home(Page::FromHeightmap);
                if let Some(home) = &mut self.home {
                    home.heightmap = Some(pick);
                }
            }
            Err(e) => self.say(ctx, format!("That height map cannot be read: {e}"), true, None),
        }
    }

    /// Show the home screen, on `page`; with a map open, the way back to it.
    fn open_home(&mut self, page: Page) {
        if self.home.is_none() {
            let mut registry = massif_core::Registry::vanilla();
            if let Some(cat) = &self.blocks.catalogue {
                massif_assets::apply_colors(&mut registry, cat);
            }
            self.home = Some(home::Home::new(registry));
        }
        let back_to = self.has_map.then(|| self.doc.world.meta.name.clone());
        if let Some(home) = &mut self.home {
            home.page = page;
            home.back_to = back_to;
        }
    }

    /// Do what the home screen asks.
    fn home_action(&mut self, ctx: &egui::Context, action: HomeAction) {
        match action {
            HomeAction::Create(params, template) => {
                self.after_create = template;
                self.request(ctx, Pending::New(params));
            }
            HomeAction::FromHeightmap(map, how) => self.request(ctx, Pending::FromHeightmap(map, how)),
            HomeAction::PickHeightmap => self.pick_heightmap(ctx),
            HomeAction::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Open a map")
                    .add_filter("Massif map", &[massif_format::EXTENSION])
                    .pick_file()
                {
                    self.request(ctx, Pending::Open(Some(path)));
                }
            }
            HomeAction::Import => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title("Import a WorldPainter world")
                    .add_filter("WorldPainter world", &["world"])
                    .pick_file()
                {
                    self.request(ctx, Pending::Open(Some(path)));
                }
            }
            HomeAction::OpenRecent(path) => {
                if path.exists() {
                    self.request(ctx, Pending::Open(Some(path)));
                } else {
                    self.say(ctx, format!("{} is no longer there", path.display()), true, None);
                }
            }
            HomeAction::Forget(path) => self.settings.recent.retain(|p| *p != path),
            HomeAction::Reveal(path) => jobs::reveal(&path),
            HomeAction::Back => {
                self.home = None;
                self.after_create = None;
            }
            HomeAction::Settings => self.dialog = Some(Dialog::Preferences),
            HomeAction::Dropped(path) => {
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                match ext.as_str() {
                    "png" | "jpg" | "jpeg" | "r16" | "r32" | "raw" => self.take_heightmap(ctx, path),
                    "world" => self.request(ctx, Pending::Open(Some(path))),
                    e if e == massif_format::EXTENSION => self.request(ctx, Pending::Open(Some(path))),
                    _ => self.say(ctx, "Massif opens .massif maps, WorldPainter .world worlds and height maps", true, None),
                }
            }
        }
    }

    /// The home screen over the whole window, and the dialogs over it.
    fn home_ui(&mut self, ui: &mut Ui, frame: &eframe::Frame, ctx: &egui::Context) {
        if self.unified_titlebar {
            frame.set_traffic_lights_position(ctx, egui::Rangef::new(0.0, 40.0), 16.0);
        }
        let now = ctx.input(|i| i.time);
        let note = self.status.as_ref().filter(|s| now - s.at < 8.0).map(|s| (s.text.clone(), s.error));
        let recent = self.settings.recent.clone();
        let mut action = None;
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
            if let Some(home) = &mut self.home {
                action = home.ui(ui, &recent, note.as_ref().map(|(t, e)| (t.as_str(), *e)));
            }
        });
        if let Some(action) = action {
            self.home_action(ctx, action);
        }
        if note.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    /// Save the map's surface as a 16-bit height map: black the bottom of
    /// the world, white its top.
    fn export_heightmap(&mut self, ctx: &egui::Context) {
        if self.doc.dimension().bounds().is_none() {
            self.say(ctx, "The map is empty", true, None);
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title("Save the height map")
            .add_filter("16-bit PNG", &["png"])
            .set_file_name(format!("{} height map.png", self.doc.world.meta.name))
            .save_file()
        else {
            return;
        };
        self.save_heightmap(ctx, &path);
    }

    /// Save the map's surface at `path` as a 16-bit height map.
    fn save_heightmap(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        let dim = self.doc.dimension();
        let Some((lo, hi)) = dim.bounds() else { return };
        let (w, d) = ((hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE);
        let side = w.max(d) as usize;
        let n = side.min(16384);
        let rect = massif_core::BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE);
        let extent = massif_core::graph::Extent::covering(rect, n);
        let (min_y, max_y) = (dim.min_y as f32, dim.max_y as f32);
        let read = massif_core::graph::sample_map(dim, &extent, min_y, &[], &[]);
        // Only the map itself, not the square around it.
        let k = n as f32 / side as f32;
        let (pw, pd) = (((w as f32) * k).round() as usize, ((d as f32) * k).round() as usize);
        let samples: Vec<f32> = (0..pw * pd).map(|c| (read.heights.data[(c / pw) * n + c % pw] - min_y) / (max_y - min_y)).collect();
        match massif_core::heightmap::write_png16(path, pw, pd, &samples) {
            Ok(()) => self.say(
                ctx,
                format!("Saved a height map of {pw} × {pd}: black is Y {min_y:.0}, white Y {max_y:.0}{}", if k < 1.0 { format!(", {:.1} blocks a pixel", 1.0 / k) } else { String::new() }),
                false,
                path.parent().map(|p| p.to_path_buf()),
            ),
            Err(e) => self.say(ctx, format!("The height map could not be saved: {e}"), true, None),
        }
    }

    /// Go to the Terrain side and start from a template.
    fn start_terrain(&mut self, t: massif_core::graph::Template) {
        self.layout.switch_side(true);
        self.graph.use_template(&mut self.doc, t);
    }

    fn with_world(world: World, layout: Layout, tools: ToolState, settings: Settings) -> Self {
        // Graphs' File input nodes read worlds with the readers of the
        // formats.
        massif_core::graph::set_world_reader(read_any_world);
        Self {
            doc: Document::new(world, None, 1),
            has_map: true,
            home: None,
            after_create: None,
            next_generation: 1,
            layout,
            tools,
            settings,
            maps: HashMap::new(),
            map_cache: MapCache::default(),
            view3d: View3d::default(),
            sculpt3d: View3d::default(),
            hover: None,
            map_center: None,
            active_map: None,
            jobs: Vec::new(),
            dialog: None,
            after_save: None,
            status: None,
            feedback: None,
            dragging_area: None,
            rename: None,
            terrain_search: String::new(),
            biome_search: String::new(),
            brushes: BrushLibrary::default(),
            new_group: None,
            graph: Default::default(),
            blocks: BlockLibrary::default(),
            allow_close: false,
            title: String::new(),
            applied_theme: None,
            unified_titlebar: false,
        }
    }

    /// An app for tests: no persistence, a small world.
    #[cfg(test)]
    pub fn for_tests(world: World) -> Self {
        Self::with_world(world, Layout::default(), ToolState::default(), Settings::default())
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        });
    }

    fn say(&mut self, ctx: &egui::Context, text: impl Into<String>, error: bool, reveal: Option<PathBuf>) {
        let text = text.into();
        if error {
            log::error!("{text}");
        } else {
            log::info!("{text}");
        }
        self.status = Some(Status {
            text,
            error,
            at: ctx.input(|i| i.time),
            reveal,
        });
    }

    fn blocking_job(&self) -> Option<&Job> {
        self.jobs.iter().find(|j| j.kind.blocks_editing())
    }

    fn remember(&mut self, path: PathBuf) {
        self.settings.recent.retain(|p| p != &path);
        self.settings.recent.insert(0, path);
        self.settings.recent.truncate(10);
    }

    // ------------------------------------------------------------ actions --

    fn request(&mut self, ctx: &egui::Context, pending: Pending) {
        if self.doc.is_modified() {
            self.dialog = Some(Dialog::Unsaved(pending));
        } else {
            self.run(ctx, pending);
        }
    }

    fn run(&mut self, ctx: &egui::Context, pending: Pending) {
        match pending {
            Pending::New(params) => self.jobs.push(Job::generate(ctx, params)),
            Pending::Open(path) => {
                let path = path.or_else(|| {
                    rfd::FileDialog::new()
                        .set_title("Open a world")
                        .add_filter("Worlds", &[massif_format::EXTENSION, "world"])
                        .add_filter("Massif world", &[massif_format::EXTENSION])
                        .add_filter("WorldPainter world", &["world"])
                        .pick_file()
                });
                if let Some(path) = path {
                    self.jobs.push(Job::open(ctx, path));
                }
            }
            Pending::FromHeightmap(map, how) => {
                let started = std::time::Instant::now();
                let world = massif_core::generate::from_heightmap(&map, &how);
                self.next_generation += 1;
                self.doc = Document::new(world, None, self.next_generation);
                self.rename = None;
                self.home = None;
                self.has_map = true;
                let (w, d) = (map.width as f32 * how.scale, map.height as f32 * how.scale);
                self.say(
                    ctx,
                    format!("Made a world of {w:.0} × {d:.0} blocks from the height map in {:.1} s", started.elapsed().as_secs_f32()),
                    false,
                    None,
                );
            }
            Pending::Quit => {
                self.allow_close = true;
                ctx.send_viewport_cmd(ViewportCommand::Close);
            }
        }
    }

    /// Start saving. Returns false if the user cancelled the file dialog.
    fn save(&mut self, ctx: &egui::Context, save_as: bool) -> bool {
        if self.jobs.iter().any(|j| j.kind == jobs::JobKind::Save) {
            return true;
        }
        let path = match (&self.doc.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .set_title("Save the world")
                .add_filter("Massif world", &[massif_format::EXTENSION])
                .set_file_name(format!("{}.{}", self.doc.world.meta.name, massif_format::EXTENSION))
                .save_file(),
        };
        let Some(mut path) = path else {
            return false;
        };
        if path.extension().is_none() {
            path.set_extension(massif_format::EXTENSION);
        }
        let snapshot = self.doc.world.snapshot();
        self.jobs.push(Job::save(ctx, snapshot, path, self.doc.revision));
        true
    }

    fn open_export_dialog(&mut self) {
        let parent = massif_anvil::default_saves_dir()
            .or_else(|| self.doc.path.as_ref().and_then(|p| p.parent().map(PathBuf::from)))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let unique = massif_anvil::unique_dir(&parent, &self.doc.world.meta.name);
        let name = unique
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.doc.world.meta.name.clone());
        let centre = self.map_center.map_or((0, 0), |c| (c[0].floor() as i32, c[1].floor() as i32));
        // For the game installed here, the newest; without one, 1.18.2,
        // which every later version opens.
        let versions = export_versions();
        let target = versions.iter().find(|(_, installed)| *installed).map_or_else(massif_anvil::Target::default, |(t, _)| t.clone());
        self.dialog = Some(Dialog::Export(ExportForm {
            target,
            versions,
            heights: (self.doc.dimension().min_y, self.doc.dimension().max_y),
            tab: ExportTab::General,
            parent,
            name,
            game_mode: GameMode::Creative,
            difficulty: massif_anvil::Difficulty::Normal,
            hardcore: false,
            allow_cheats: true,
            surroundings: Surroundings::Vanilla,
            structures: true,
            overwrite: false,
            border: massif_anvil::WorldBorder::default(),
            datapacks: Vec::new(),
            selected_pack: None,
            spawn: self.doc.world.meta.spawn,
            test_centre: centre,
            fit: self.doc.dimension().bounds().map(|(lo, hi)| {
                let (x0, z0) = (lo.x as f64 * 128.0, lo.z as f64 * 128.0);
                let (x1, z1) = ((hi.x + 1) as f64 * 128.0, (hi.z + 1) as f64 * 128.0);
                ([(x0 + x1) / 2.0, (z0 + z1) / 2.0], (x1 - x0).max(z1 - z0))
            }),
        }));
    }

    fn undo(&mut self, ctx: &egui::Context) {
        // On the Terrain side, the graph's changes and its conversions, in
        // the order they were made.
        if self.layout.terrain()
            && let Some(done) = self.graph.undo(&mut self.doc)
            && matches!(done, crate::graph_editor::Undone::Graph)
        {
            self.say(ctx, "Undid a change to the graph", false, None);
            return;
        }
        if let Some(label) = self.doc.undo() {
            self.say(ctx, format!("Undid {label}"), false, None);
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
        }
    }

    fn redo(&mut self, ctx: &egui::Context) {
        if self.layout.terrain()
            && let Some(done) = self.graph.redo(&mut self.doc)
            && matches!(done, crate::graph_editor::Undone::Graph)
        {
            self.say(ctx, "Redid a change to the graph", false, None);
            return;
        }
        if let Some(label) = self.doc.redo() {
            self.say(ctx, format!("Redid {label}"), false, None);
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
        }
    }

    // --------------------------------------------------------------- jobs --

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        let mut i = 0;
        while i < self.jobs.len() {
            let Some(outcome) = self.jobs[i].poll() else {
                i += 1;
                continue;
            };
            let job = self.jobs.remove(i);
            let secs = job.started.elapsed().as_secs_f32();
            match outcome {
                Outcome::World { world, path } => {
                    self.next_generation += 1;
                    let name = world.meta.name.clone();
                    self.doc = Document::new(world, path.clone(), self.next_generation);
                    self.rename = None;
                    self.home = None;
                    self.has_map = true;
                    match path {
                        Some(p) => {
                            self.remember(p);
                            self.say(ctx, format!("Opened “{name}” in {secs:.1} s"), false, None);
                        }
                        None => {
                            self.say(ctx, format!("Created “{name}” in {secs:.1} s"), false, None);
                            // Its relief from a template: on the Terrain side.
                            if let Some(t) = self.after_create.take() {
                                self.start_terrain(t);
                            }
                        }
                    }
                }
                Outcome::Saved { path, revision, bytes } => {
                    self.doc.mark_saved(revision, path.clone());
                    self.remember(path.clone());
                    let msg = format!("Saved {} ({}) in {secs:.1} s", jobs::display_name(&path), panels::human(bytes));
                    self.say(ctx, msg, false, Some(path));
                    if let Some(pending) = self.after_save.take() {
                        self.run(ctx, pending);
                    }
                }
                Outcome::Exported { dir, stats } => {
                    let msg = format!(
                        "Exported {} chunks in {} region files ({}) in {:.1} s",
                        stats.chunks,
                        stats.regions,
                        panels::human(stats.bytes),
                        stats.elapsed.as_secs_f32()
                    );
                    self.say(ctx, msg, false, Some(dir));
                }
                Outcome::Failed(message) => {
                    self.after_save = None;
                    self.say(ctx, message, true, None);
                }
                Outcome::Imported { world, report, source } => {
                    self.next_generation += 1;
                    self.doc = Document::new(world, None, self.next_generation);
                    self.rename = None;
                    self.home = None;
                    self.has_map = true;
                    self.remember(source.clone());
                    let msg = format!("Imported {} in {secs:.1} s: {}", jobs::display_name(&source), report.summary());
                    self.say(ctx, msg, false, None);
                    let thumbnail = report.thumbnail.as_deref().and_then(|jpeg| {
                        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.to_rgba8();
                        let size = [img.width() as usize, img.height() as usize];
                        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                        Some(ctx.load_texture("import thumbnail", color, egui::TextureOptions::LINEAR))
                    });
                    self.dialog = Some(Dialog::ImportReport(report, thumbnail));
                }
                Outcome::Cancelled => self.say(ctx, "Cancelled", false, None),
            }
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // ---------------------------------------------------------- shortcuts --

    fn shortcuts(&mut self, ctx: &egui::Context) {
        if self.dialog.is_some() || self.blocking_job().is_some() {
            return;
        }
        let cmd = |key| KeyboardShortcut::new(Modifiers::COMMAND, key);
        let cmd_shift = |key| KeyboardShortcut::new(Modifiers::COMMAND | Modifiers::SHIFT, key);
        let pressed = |s: KeyboardShortcut| ctx.input_mut(|i| i.consume_shortcut(&s));
        if pressed(cmd(Key::Comma)) {
            self.dialog = Some(Dialog::Preferences);
            return;
        }
        if self.home.is_some() {
            // No map on the home screen: only the ways in.
            if pressed(cmd(Key::O)) {
                self.home_action(ctx, HomeAction::Open);
            }
            return;
        }
        if pressed(cmd_shift(Key::Z)) || pressed(cmd(Key::Y)) {
            self.redo(ctx);
        } else if pressed(cmd(Key::Z)) {
            self.undo(ctx);
        }
        if pressed(cmd_shift(Key::S)) {
            self.save(ctx, true);
        } else if pressed(cmd(Key::S)) {
            self.save(ctx, false);
        }
        if pressed(cmd(Key::O)) {
            self.request(ctx, Pending::Open(None));
        }
        if pressed(cmd(Key::N)) {
            self.open_home(Page::NewMap);
        }
        if pressed(cmd(Key::E)) {
            self.open_export_dialog();
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        if pressed(KeyboardShortcut::new(Modifiers::CTRL, Key::Space)) {
            let pointer = ctx.input(|i| i.pointer.hover_pos());
            if let Some(p) = pointer
                && let Some(id) = self.area_at(ctx, p)
            {
                self.layout.toggle_maximize(id);
            }
        }
        // WorldPainter's keys: 1…0 brush intensity, − and + brush size,
        // [ and ] (or Alt with − and +) brush rotation, R and L the light of
        // the map, Alt and a letter the tools, ⌘ with −, + and 0 the zoom.
        let presses: Vec<(Key, Option<Key>, Modifiers)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        physical_key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some((*key, *physical_key, *modifiers)),
                    _ => None,
                })
                .collect()
        });
        for (key, physical, mods) in presses {
            let is = |k: Key| key == k || physical == Some(k);
            let digit = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9, Key::Num0]
                .iter()
                .position(|d| is(*d));
            let brush = &mut self.tools.brush;
            if mods.command {
                let zoom = if key == Key::Minus {
                    Some(0.5)
                } else if key == Key::Plus || key == Key::Equals {
                    Some(2.0)
                } else if is(Key::Num0) {
                    Some(0.0)
                } else {
                    None
                };
                if let Some(factor) = zoom {
                    let id = self.active_map.or_else(|| self.maps.keys().next().copied());
                    if let Some(view) = id.and_then(|id| self.maps.get_mut(&id)) {
                        if factor == 0.0 {
                            view.zoom_reset();
                        } else {
                            view.zoom_by(factor);
                        }
                    }
                }
                continue;
            }
            if mods.alt {
                match digit {
                    Some(9) => brush.rotation = 0.0,
                    Some(2) => brush.rotate(30.0),
                    Some(3) => brush.rotate(45.0),
                    Some(8) => brush.rotate(90.0),
                    _ if key == Key::Minus => brush.rotate(-15.0),
                    _ if key == Key::Plus || key == Key::Equals => brush.rotate(15.0),
                    _ => {
                        for (_, _, list) in Tool::GROUPS {
                            for &tool in list {
                                if tool.shortcuts().iter().any(|(m, k)| m.alt && is(*k)) {
                                    self.tools.select(tool);
                                }
                            }
                        }
                    }
                }
                continue;
            }
            if let Some(d) = digit {
                brush.strength = (d + 1) as f32 / 10.0;
            } else if key == Key::Minus {
                let r = brush.radius;
                brush.radius = if mods.shift { r - 1.0 } else { (r / 1.1).min(r - 1.0) }.round().max(1.0);
            } else if key == Key::Plus || key == Key::Equals {
                let r = brush.radius;
                brush.radius = (r * 1.1).max(r + 1.0).round().min(massif_core::MAX_RADIUS);
            } else if is(Key::OpenBracket) {
                brush.rotate(-15.0);
            } else if is(Key::CloseBracket) {
                brush.rotate(15.0);
            } else if is(Key::R) && mods.is_none() {
                self.settings.light = (self.settings.light + 1) % 16;
            } else if is(Key::L) && mods.is_none() {
                self.settings.light = (self.settings.light + 15) % 16;
            }
        }
    }

    /// The area under a screen position, from the last layout pass.
    fn area_at(&self, ctx: &egui::Context, p: egui::Pos2) -> Option<AreaId> {
        let areas: Vec<(AreaId, Rect)> = ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap_or_default();
        areas.into_iter().find(|(_, r)| r.contains(p)).map(|(id, _)| id)
    }

    // ------------------------------------------------------------ chrome --

    fn top_bar(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        if self.unified_titlebar {
            // Dragging the empty parts of the bar moves the window.
            let bar = ui.max_rect().expand2(vec2(8.0, 6.0));
            let response = ui.interact(bar, Id::unique("title bar"), Sense::click_and_drag());
            if response.drag_started() {
                ctx.send_viewport_cmd(ViewportCommand::StartDrag);
            }
            if response.double_clicked() {
                let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
            }
            ui.add_space(70.0);
        }
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button(theme::with_icon(Icon::FilePlus2, "New World…")).clicked() {
                    self.open_home(Page::NewMap);
                }
                if ui.button(theme::with_icon(Icon::FolderOpen, "Open…")).clicked() {
                    self.request(&ctx, Pending::Open(None));
                }
                if ui.button(theme::with_icon(Icon::Import, "Import WorldPainter World…")).clicked()
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title("Import a WorldPainter world")
                        .add_filter("WorldPainter world", &["world"])
                        .pick_file()
                {
                    self.request(&ctx, Pending::Open(Some(path)));
                }
                if ui.button(theme::with_icon(Icon::ImageUp, "New World from Height Map…")).clicked() {
                    self.open_home(Page::FromHeightmap);
                }
                let recent = self.settings.recent.clone();
                ui.add_enabled_ui(!recent.is_empty(), |ui| {
                    ui.menu_button(theme::with_icon(Icon::FolderInput, "Open Recent"), |ui| {
                        for path in recent {
                            if ui.button(jobs::display_name(&path)).on_hover_text(path.display().to_string()).clicked() {
                                self.request(&ctx, Pending::Open(Some(path)));
                            }
                        }
                    });
                });
                ui.separator();
                if ui.button(theme::with_icon(Icon::Save, "Save")).clicked() {
                    self.save(&ctx, false);
                }
                if ui.button(theme::with_icon(Icon::Save, "Save As…")).clicked() {
                    self.save(&ctx, true);
                }
                ui.separator();
                if ui.button(theme::with_icon(Icon::ImageDown, "Export Height Map…")).clicked() {
                    self.export_heightmap(&ctx);
                }
                if ui.button(theme::with_icon(Icon::Package, "Export to Minecraft…")).clicked() {
                    self.open_export_dialog();
                }
                ui.separator();
                if ui.button(theme::with_icon(Icon::X, "Quit")).clicked() {
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            });
            ui.menu_button("Edit", |ui| {
                let undo = self.doc.history.undo_label().map(|l| format!("Undo {l}"));
                if ui.add_enabled(undo.is_some(), egui::Button::new(theme::with_icon(Icon::Undo2, undo.as_deref().unwrap_or("Undo")))).clicked() {
                    self.undo(&ctx);
                }
                let redo = self.doc.history.redo_label().map(|l| format!("Redo {l}"));
                if ui.add_enabled(redo.is_some(), egui::Button::new(theme::with_icon(Icon::Redo2, redo.as_deref().unwrap_or("Redo")))).clicked() {
                    self.redo(&ctx);
                }
                ui.separator();
                if ui
                    .button(theme::with_icon(Icon::Earth, "Auto biome: whole map"))
                    .on_hover_text("Give every column the biome its land calls for")
                    .clicked()
                {
                    let changed = self.doc.auto_biome(&self.tools.auto);
                    self.tools.auto_done = Some((true, changed));
                    self.feedback = Some(format!("Auto biome over the whole map: {changed} columns changed"));
                }
                if ui
                    .button(theme::with_icon(Icon::WandSparkles, "Auto paint: whole map"))
                    .on_hover_text("Paint every column as its biome calls for")
                    .clicked()
                {
                    let changed = self.doc.auto_paint(&self.tools.auto);
                    self.tools.auto_done = Some((false, changed));
                    self.feedback = Some(format!("Auto paint over the whole map: {changed} columns changed"));
                }
                if ui.button(theme::with_icon(Icon::Settings2, "Automatic rules…")).clicked() {
                    self.dialog = Some(Dialog::AutoRules(false));
                }
            });
            ui.menu_button("View", |ui| {
                for (choice, label) in [(ThemeChoice::System, "Follow the system"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")] {
                    if ui.radio(self.settings.theme == choice, label).clicked() {
                        self.settings.theme = choice;
                    }
                }
                ui.separator();
                if ui.button(theme::with_icon(Icon::Settings, "Settings… (⌘,)")).clicked() {
                    self.dialog = Some(Dialog::Preferences);
                }
                if ui.button("Reset workspaces").clicked() {
                    self.layout = Layout::default();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button(theme::with_icon(Icon::House, "Home screen")).clicked() {
                    self.open_home(Page::NewMap);
                }
                if ui.button(theme::with_icon(Icon::Info, "About Massif")).clicked() {
                    self.dialog = Some(Dialog::About);
                }
                ui.separator();
                ui.label(RichText::new("Shortcuts (as in WorldPainter)").bold());
                for (keys, what) in [
                    ("Left / right button", "Apply the tool / its inverse (also Alt + left)"),
                    ("Scroll", "Brush size (Shift: one block at a time)"),
                    ("⌘, Ctrl or Alt + scroll, pinch", "Zoom"),
                    ("Middle button, Space + drag", "Move the map"),
                    ("− / +", "Brush size"),
                    ("1 … 9, 0", "Brush intensity 10 % … 100 %"),
                    ("[ / ]", "Turn the brush"),
                    ("R / L", "Turn the light of the map"),
                    ("⌥H ⌥A ⌥S ⌥P ⌥R ⌥F ⌥Y", "Height, Flatten, Smooth, Paint, Spray, Flood, Eyedropper"),
                    ("⌘− / ⌘+ / ⌘0", "Zoom out / in / 100 %"),
                    ("⌘Z / ⇧⌘Z", "Undo / redo"),
                    ("⌘S, ⌘O, ⌘N, ⌘E", "Save, open, new, export"),
                    ("Esc", "Cancel the stroke being drawn"),
                    ("Ctrl + Space", "Maximise the area under the mouse"),
                    ("Shift over an area's header", "A button that closes the area"),
                    ("Drag an area's top-right corner", "Split it where you let go"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(keys).monospace());
                        ui.label(RichText::new(what).color(Palette::of(ui).weak));
                    });
                }
                ui.separator();
                ui.label(RichText::new("3D view").bold());
                for (keys, what) in [
                    ("Left drag / right drag", "Turn the view / move it"),
                    ("Scroll, pinch", "Come closer / go back"),
                    ("W A S D, arrows · Q / E", "Fly over the map · down / up (Shift: faster)"),
                    ("Double-click", "Look at the point under the cursor"),
                    ("With Build, Dig, Smooth (3D)", "Left sculpts where the cursor points, right inverts"),
                    ("   middle drag, Alt + drag", "Turn the view while sculpting"),
                    ("   Shift + middle drag", "Move the view while sculpting"),
                ] {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(keys).monospace());
                        ui.label(RichText::new(what).color(Palette::of(ui).weak));
                    });
                }
            });

            ui.separator();
            let busy = self.blocking_job().is_some();
            let icon = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| {
                ui.add_enabled(enabled, egui::Button::new(theme::glyph(icon).to_string()).frame(false))
                    .on_hover_text(tip)
                    .clicked()
            };
            if icon(ui, Icon::FilePlus2, "New world (⌘N)", !busy) {
                self.open_home(Page::NewMap);
            }
            if icon(ui, Icon::FolderOpen, "Open (⌘O)", !busy) {
                self.request(&ctx, Pending::Open(None));
            }
            if icon(ui, Icon::Save, "Save (⌘S)", !busy) {
                self.save(&ctx, false);
            }
            if icon(ui, Icon::Package, "Export to Minecraft (⌘E)", !busy) {
                self.open_export_dialog();
            }
            ui.separator();
            if icon(ui, Icon::Undo2, "Undo (⌘Z)", self.doc.history.can_undo()) {
                self.undo(&ctx);
            }
            if icon(ui, Icon::Redo2, "Redo (⇧⌘Z)", self.doc.history.can_redo()) {
                self.redo(&ctx);
            }
            ui.separator();
            for tool in [Tool::Raise, Tool::Smooth, Tool::Flatten, Tool::PaintTerrain, Tool::Flood, Tool::StampSlab] {
                let selected = self.tools.tool == tool;
                let text = RichText::new(theme::glyph(tool.icon()).to_string()).color(if selected { pal.accent } else { pal.text });
                if ui.add(egui::Button::new(text).frame(selected)).on_hover_text(tool.label()).clicked() {
                    self.tools.select(tool);
                }
            }

            // Centred title.
            let title = format!("{}{}", self.doc.title(), if self.doc.is_modified() { " •" } else { "" });
            let bar = ui.max_rect();
            ui.painter().text(
                pos2(bar.center().x, bar.center().y),
                Align2::CENTER_CENTER,
                title,
                FontId::proportional(13.0),
                pal.text,
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let (icon, next) = match self.settings.theme {
                    ThemeChoice::System => (Icon::SunMoon, ThemeChoice::Light),
                    ThemeChoice::Light => (Icon::Sun, ThemeChoice::Dark),
                    ThemeChoice::Dark => (Icon::Moon, ThemeChoice::System),
                };
                if ui.add(egui::Button::new(theme::glyph(icon).to_string()).frame(false)).on_hover_text("Theme").clicked() {
                    self.settings.theme = next;
                }
            });
        });
    }

    fn workspace_tabs(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.horizontal(|ui| {
            // The two sides of Massif: the map editor (WorldPainter's) and
            // the terrain generator (Gaea's).
            let terrain = self.layout.terrain();
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (side, icon, label, hint) in [
                            (false, Icon::Map, "World", "Edit the map: sculpt, paint, levels, 3D, export"),
                            (true, Icon::MountainSnow, "Terrain", "Generate terrain from nodes, render it, convert it to the map"),
                        ] {
                            let active = terrain == side;
                            let text = RichText::new(theme::with_icon(icon, label)).color(if active { pal.text } else { pal.weak });
                            let button = egui::Button::new(if active { text.medium() } else { text })
                                .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE)
                                .corner_radius(7)
                                .min_size(vec2(0.0, 24.0));
                            if ui.add(button).on_hover_text(hint).clicked() {
                                self.layout.switch_side(side);
                            }
                        }
                    });
                });
            ui.add_space(4.0);
            // A segmented control: the workspaces of this side in one pill.
            let mut remove = None;
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(10)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for i in 0..self.layout.workspaces.len() {
                            if self.layout.workspaces[i].terrain != terrain {
                                continue;
                            }
                            let active = self.layout.active == i;
                            let name = self.layout.workspaces[i].name.clone();
                            let text = RichText::new(name).color(if active { pal.text } else { pal.weak });
                            let button = egui::Button::new(if active { text.medium() } else { text })
                                .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE)
                                .corner_radius(7)
                                .min_size(vec2(0.0, 24.0));
                            let r = ui.add(button);
                            if r.clicked() {
                                self.layout.activate(i);
                            }
                            r.context_menu(|ui| {
                                if ui.button("Duplicate").clicked() {
                                    self.layout.activate(i);
                                    self.layout.duplicate_current();
                                }
                                if ui.add_enabled(self.layout.side_len(i) > 1, egui::Button::new("Delete")).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        let plus = egui::Button::new(RichText::new(theme::glyph(Icon::Plus).to_string()).color(pal.weak))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(7)
                            .min_size(vec2(24.0, 24.0));
                        if ui.add(plus).on_hover_text("New workspace (a copy of this one)").clicked() {
                            self.layout.duplicate_current();
                        }
                    });
                });
            if let Some(i) = remove {
                self.layout.remove_workspace(i);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.label(
                    RichText::new("Ctrl+Space: maximise · drag a title: move")
                        .size(11.5)
                        .color(pal.weak),
                );
            });
        });
    }

    fn status_bar(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let now = ui.input(|i| i.time);
        ui.horizontal(|ui| {
            if let Some(status) = &self.status {
                let age = now - status.at;
                if age < 12.0 || status.error {
                    let (icon, color) = if status.error { (Icon::TriangleAlert, pal.danger) } else { (Icon::CircleCheck, pal.success) };
                    ui.label(RichText::new(theme::glyph(icon).to_string()).color(color).size(12.0));
                    ui.label(RichText::new(&status.text).color(if status.error { pal.danger } else { pal.text }).size(12.0));
                    if let Some(path) = &status.reveal
                        && ui.small_button("Show in Finder").clicked() {
                            jobs::reveal(path);
                        }
                    if status.error && ui.small_button(theme::glyph(Icon::X).to_string()).clicked() {
                        self.status = None;
                    }
                }
            }
            for job in &self.jobs {
                ui.separator();
                ui.spinner();
                let elapsed = job.started.elapsed().as_secs_f32();
                match &job.progress {
                    Some(p) => {
                        ui.add(egui::ProgressBar::new(p.fraction()).desired_width(140.0).show_percentage());
                        ui.label(RichText::new(&job.label).size(12.0));
                        if ui.small_button("Cancel").clicked() {
                            job.cancel();
                        }
                    }
                    None => {
                        ui.label(RichText::new(format!("{} · {elapsed:.0} s", job.label)).size(12.0));
                    }
                }
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let dim = self.doc.dimension();
                let tiles: usize = dim.levels.iter().map(|l| l.len()).sum();
                let levels = dim.levels.len();
                ui.label(
                    RichText::new(format!(
                        "{} level{} · {} tiles · {}",
                        levels,
                        if levels == 1 { "" } else { "s" },
                        tiles,
                        self.doc.history.undo_label().map(|l| format!("last edit: {l}")).unwrap_or_else(|| "no edits".into())
                    ))
                    .size(11.5)
                    .color(pal.weak),
                );
            });
        });
    }

    // --------------------------------------------------------------- dock --

    fn dock(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let rect = ui.available_rect_before_wrap();
        let ws = self.layout.current();
        let mut areas = Vec::new();
        let mut seps = Vec::new();
        match ws.maximized.and_then(|id| ws.root.editor_of(id).map(|e| (id, e))) {
            Some((id, editor)) => areas.push((id, editor, rect)),
            None => compute(&ws.root, rect, &mut Vec::new(), &mut areas, &mut seps),
        }
        let area_rects: Vec<(AreaId, Rect)> = areas.iter().map(|(id, _, r)| (*id, *r)).collect();
        ui.ctx().data_mut(|d| d.insert_temp(Id::unique("massif areas"), area_rects));

        // Splitters.
        for sep in &seps {
            let id = Id::unique(("separator", &sep.path, self.layout.active));
            let response = ui.interact(sep.rect, id, Sense::drag());
            let cursor = match sep.dir {
                Dir::Row => CursorIcon::ResizeHorizontal,
                Dir::Column => CursorIcon::ResizeVertical,
            };
            if response.hovered() || response.dragged() {
                ui.ctx().set_cursor_icon(cursor);
            }
            if response.dragged()
                && let Some(p) = response.interact_pointer_pos()
            {
                let (pos, start, len) = match sep.dir {
                    Dir::Row => (p.x, sep.parent.left(), sep.parent.width()),
                    Dir::Column => (p.y, sep.parent.top(), sep.parent.height()),
                };
                let min = (MIN_AREA / len).min(0.45);
                let ratio = ((pos - start) / len).clamp(min, 1.0 - min);
                if let Node::Split { ratio: r, .. } = node_at(&mut self.layout.current_mut().root, &sep.path) {
                    *r = ratio;
                }
            }
        }

        let shown: HashSet<EditorKind> = areas.iter().map(|a| a.1).collect();
        self.map_center = None;
        for (id, editor, rect) in &areas {
            self.area(ui, frame, *id, *editor, *rect, &shown);
        }
        self.maps.retain(|id, _| areas.iter().any(|a| a.0 == *id));

        // Moving an area by its title.
        if let Some(src) = self.dragging_area {
            let (pointer, down) = ui.input(|i| (i.pointer.hover_pos(), i.pointer.primary_down()));
            let target = pointer.and_then(|p| {
                areas
                    .iter()
                    .find(|(id, _, r)| *id != src && r.contains(p))
                    .map(|(id, _, r)| (*id, *r, zone_of(*r, p)))
            });
            let painter = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::unique("dock overlay")));
            if let Some((_, r, zone)) = target {
                let pal = Palette::of(ui);
                let half = |r: Rect, z: Zone| match z {
                    Zone::Left => Rect::from_min_max(r.min, pos2(r.center().x, r.max.y)),
                    Zone::Right => Rect::from_min_max(pos2(r.center().x, r.min.y), r.max),
                    Zone::Top => Rect::from_min_max(r.min, pos2(r.max.x, r.center().y)),
                    Zone::Bottom => Rect::from_min_max(pos2(r.min.x, r.center().y), r.max),
                    Zone::Center => r,
                };
                let zone_rect = half(r, zone).shrink(4.0);
                painter.rect(zone_rect, 8, pal.accent.gamma_multiply(0.22), Stroke::new(2.0, pal.accent), StrokeKind::Inside);
                let label = if zone == Zone::Center { "Swap" } else { "Split" };
                painter.text(zone_rect.center(), Align2::CENTER_CENTER, label, FontId::proportional(14.0), pal.accent);
            }
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
            if !down {
                if let Some((dst, _, zone)) = target {
                    self.layout.current_mut().root.drop_on(src, dst, zone);
                }
                self.dragging_area = None;
            }
        }
    }

    fn area(&mut self, ui: &mut Ui, frame: &eframe::Frame, id: AreaId, editor: EditorKind, rect: Rect, shown: &HashSet<EditorKind>) {
        let pal = Palette::of(ui);
        let painter = ui.painter_at(rect.expand(1.0));
        painter.rect_filled(rect, 8, pal.panel);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        painter.rect_filled(header, egui::CornerRadius { nw: 8, ne: 8, sw: 0, se: 0 }, pal.header);
        painter.hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0, pal.border));
        painter.rect_stroke(rect, 8, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let content = Rect::from_min_max(pos2(rect.min.x + 1.0, header.bottom() + 1.0), pos2(rect.max.x - 1.0, rect.max.y - 1.0));
        let maximized = self.layout.current().maximized == Some(id);
        let single = self.layout.current().root.count() == 1;

        // ---- header --------------------------------------------------------
        let mut hui = ui.new_child(
            UiBuilder::new()
                .id_salt(("area header", id))
                .max_rect(header.shrink2(vec2(8.0, 4.0)))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        hui.set_clip_rect(header.intersect(ui.clip_rect()));
        hui.spacing_mut().item_spacing.x = 4.0;
        hui.menu_button(format!("{} {}", theme::glyph(editor.icon()), theme::glyph(Icon::ChevronDown)), |ui| {
            for (group, kinds) in EditorKind::GROUPS {
                ui.label(RichText::new(group).size(11.0).color(Palette::of(ui).weak));
                for &kind in kinds {
                    if ui.selectable_label(kind == editor, theme::with_icon(kind.icon(), kind.label())).clicked() {
                        self.layout.current_mut().root.set_editor(id, kind);
                    }
                }
            }
        });
        let title = hui
            .add(egui::Label::new(RichText::new(editor.label()).bold()).sense(Sense::drag()).selectable(false))
            .on_hover_cursor(CursorIcon::Grab);
        if title.drag_started() && !single {
            self.dragging_area = Some(id);
        }
        match editor {
            EditorKind::Map => {
                hui.add_space(8.0);
                self.maps.entry(id).or_default().header(&mut hui);
            }
            EditorKind::View3d => {
                hui.add_space(8.0);
                self.view3d.header(&mut hui);
                hui.separator();
                if hui
                    .button(theme::with_icon(Icon::Pickaxe, "Sculpt"))
                    .on_hover_text("Build and dig in 3D: the 3D Sculpt workspace")
                    .clicked()
                    && let Some(k) = self.layout.workspaces.iter().position(|w| w.root.shows(EditorKind::Sculpt3d))
                {
                    self.layout.activate(k);
                }
            }
            EditorKind::Sculpt3d => {
                hui.add_space(8.0);
                self.sculpt3d.header(&mut hui);
            }
            _ => {}
        }
        let shift = ui.input(|i| i.modifiers.shift);
        hui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            ui.add_space(8.0); // room for the corner handle
            // As in the modernised WorldPainter: a menu of commands, and with
            // Shift held, a button that closes the area at once.
            if shift && !single && !maximized {
                if ui
                    .add(egui::Button::new(RichText::new(theme::glyph(Icon::Trash2).to_string()).size(12.5)).frame(false))
                    .on_hover_text("Close this area")
                    .clicked()
                {
                    self.layout.close(id);
                }
                return;
            }
            let sibling = self.layout.current().root.sibling(id);
            ui.menu_button(RichText::new(theme::glyph(Icon::EllipsisVertical).to_string()).size(13.0), |ui| {
                if ui.button(theme::with_icon(Icon::Columns2, "Split vertically")).clicked() {
                    self.layout.split(id, Dir::Row);
                }
                if ui.button(theme::with_icon(Icon::Rows2, "Split horizontally")).clicked() {
                    self.layout.split(id, Dir::Column);
                }
                ui.add_enabled_ui(sibling.is_some() && !maximized, |ui| {
                    ui.menu_button(theme::with_icon(Icon::Combine, "Join with"), |ui| {
                        if let Some((_, editor, dir, first)) = sibling {
                            let side = match (dir, first) {
                                (Dir::Row, true) => "on the right",
                                (Dir::Row, false) => "on the left",
                                (Dir::Column, true) => "below",
                                (Dir::Column, false) => "above",
                            };
                            if ui.button(theme::with_icon(editor.icon(), &format!("{} {side}", editor.label()))).clicked() {
                                self.layout.join(id);
                            }
                        }
                    });
                });
                ui.separator();
                let label = if maximized { "Restore the layout" } else { "Maximise area" };
                let icon = if maximized { Icon::Minimize2 } else { Icon::Maximize2 };
                if ui
                    .add_enabled(!single || maximized, egui::Button::new(theme::with_icon(icon, label)).shortcut_text("Ctrl+Space"))
                    .clicked()
                {
                    self.layout.toggle_maximize(id);
                }
                if ui
                    .add_enabled(!single && !maximized, egui::Button::new(theme::with_icon(Icon::X, "Close area")))
                    .clicked()
                {
                    self.layout.close(id);
                }
            })
            .response
            .on_hover_text("Area commands · Shift: close button · drag the corner to split");
        });

        // ---- corner: drag it into the area to split ----------------------------
        let corner = Rect::from_min_size(pos2(rect.right() - 12.0, rect.top()), vec2(12.0, 12.0));
        let handle = ui.interact(corner, Id::unique(("area corner", id)), Sense::drag());
        let hot = handle.hovered() || handle.dragged();
        {
            let c = if hot { pal.accent } else { pal.weak.gamma_multiply(0.6) };
            for k in [4.0, 8.0] {
                painter.line_segment(
                    [pos2(corner.right() - k - 1.0, corner.top() + 1.5), pos2(corner.right() - 1.5, corner.top() + k + 1.0)],
                    Stroke::new(1.2, c),
                );
            }
        }
        if hot {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if handle.dragged()
            && let (Some(origin), Some(p)) = (handle.interact_pointer_pos().map(|_| corner.center()), ui.input(|i| i.pointer.interact_pos()))
        {
            let d = p - origin;
            let fg = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::unique(("corner preview", id))));
            if d.length() > 12.0 && rect.contains(p) {
                let line = if d.x.abs() > d.y.abs() {
                    [pos2(p.x, rect.top()), pos2(p.x, rect.bottom())]
                } else {
                    [pos2(rect.left(), p.y), pos2(rect.right(), p.y)]
                };
                fg.line_segment(line, Stroke::new(2.0, pal.accent));
            }
        }
        if handle.drag_stopped()
            && let Some(p) = ui.input(|i| i.pointer.interact_pos())
        {
            let d = p - corner.center();
            if d.length() > 12.0 && rect.contains(p) {
                if d.x.abs() > d.y.abs() {
                    // Leftward: the new area on the right, where the corner was.
                    self.layout.split_at(id, Dir::Row, false, (p.x - rect.left()) / rect.width());
                } else {
                    // Downward: the new area on top.
                    self.layout.split_at(id, Dir::Column, true, (p.y - rect.top()) / rect.height());
                }
            }
        }

        // ---- content -------------------------------------------------------
        let mut cui = ui.new_child(
            UiBuilder::new()
                .id_salt(("area", id))
                .max_rect(content)
                .layout(egui::Layout::top_down(Align::Min)),
        );
        cui.set_clip_rect(content.intersect(ui.clip_rect()));
        let scrolled = |ui: &mut Ui, f: &mut dyn FnMut(&mut Ui)| {
            egui::ScrollArea::vertical()
                .id_salt(("scroll", id))
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Frame::new().inner_margin(egui::Margin::symmetric(10, 8)).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        f(ui);
                    });
                });
        };
        match editor {
            EditorKind::Map => {
                let pan_key = !cui.ctx().egui_wants_keyboard_input() && cui.input(|i| i.key_down(Key::Space));
                if cui.input(|i| i.pointer.hover_pos()).is_some_and(|p| content.contains(p)) {
                    self.active_map = Some(id);
                }
                let view = self.maps.entry(id).or_default();
                view.ui(
                    &mut cui,
                    MapInput {
                        doc: &mut self.doc,
                        tools: &mut self.tools,
                        cache: &mut self.map_cache,
                        solo: self.settings.solo,
                        hover: &mut self.hover,
                        feedback: &mut self.feedback,
                        pan_key,
                        light: self.settings.light,
                    },
                );
                if self.map_center.is_none() {
                    self.map_center = Some(view.center);
                }
            }
            EditorKind::View3d => self.view3d.ui(&mut cui, frame, &mut self.doc, &mut self.tools, self.map_center, false),
            EditorKind::Sculpt3d => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(8, 6))
                    .show(&mut cui, |ui| panels::sculpt_toolbar(ui, &mut self.tools.sculpt, &mut self.brushes));
                self.sculpt3d.ui(&mut cui, frame, &mut self.doc, &mut self.tools, self.map_center, true);
            }
            EditorKind::Tools => scrolled(&mut cui, &mut |ui| panels::tools_panel(ui, &mut self.tools)),
            EditorKind::ToolSettings => {
                scrolled(&mut cui, &mut |ui| panels::tool_settings(ui, &mut self.tools, &mut self.doc, &mut self.blocks))
            }
            EditorKind::Blocks => {
                let mut feedback = None;
                scrolled(&mut cui, &mut |ui| {
                    feedback = panels::blocks_panel(ui, &mut self.tools, &mut self.doc, &mut self.blocks, &mut self.settings.minecraft_jar);
                });
                if feedback.is_some() {
                    self.feedback = feedback;
                }
            }
            EditorKind::BrushSettings => {
                let (min_y, max_y) = (self.doc.dimension().min_y, self.doc.dimension().max_y);
                scrolled(&mut cui, &mut |ui| {
                    let light = self.settings.light as f32 * 22.5;
                    panels::brush_settings(ui, &mut self.tools, &mut self.brushes, &self.doc.world.registry, min_y, max_y, light)
                })
            }
            EditorKind::Brushes => {
                let mut feedback = None;
                scrolled(&mut cui, &mut |ui| {
                    feedback = panels::brushes_panel(ui, &mut self.tools, &mut self.brushes, &mut self.new_group);
                });
                if feedback.is_some() {
                    self.feedback = feedback;
                }
            }
            EditorKind::CustomBrushes => {
                let mut feedback = None;
                scrolled(&mut cui, &mut |ui| feedback = panels::custom_brushes_panel(ui, &mut self.tools, &mut self.brushes));
                if feedback.is_some() {
                    self.feedback = feedback;
                }
            }
            EditorKind::Terrain => {
                let mut solo_changed = false;
                scrolled(&mut cui, &mut |ui| {
                    solo_changed |= panels::terrain_panel(
                        ui,
                        &mut self.tools,
                        &self.doc.world.registry,
                        &mut self.settings.solo,
                        &mut self.terrain_search,
                        &mut self.blocks,
                    );
                });
                let _ = solo_changed; // the solo style is part of the cache key
            }
            EditorKind::Biomes => scrolled(&mut cui, &mut |ui| panels::biome_panel(ui, &mut self.tools, &self.doc.world.registry, &mut self.biome_search)),
            EditorKind::Levels => {
                let mut change = panels::LevelsChange::default();
                scrolled(&mut cui, &mut |ui| change = panels::levels_panel(ui, &mut self.doc, &mut self.rename));
                if change.composition {
                    self.map_cache.invalidate_all();
                    self.view3d.invalidate_all();
                    self.sculpt3d.invalidate_all();
                }
            }
            EditorKind::Info => {
                let stats = self.map_cache.stats();
                let mut affects_map = false;
                scrolled(&mut cui, &mut |ui| affects_map = panels::info_panel(ui, &mut self.doc, self.hover, stats));
                if affects_map {
                    self.map_cache.invalidate_all();
                    self.view3d.invalidate_all();
                    self.sculpt3d.invalidate_all();
                }
            }
            EditorKind::Graph => {
                if let Some(msg) = self.graph.canvas_ui(&mut cui, &mut self.doc, &self.blocks) {
                    self.feedback = Some(msg);
                }
            }
            EditorKind::GraphView => self.graph.viewport_ui(&mut cui, frame, &self.doc),
            EditorKind::GraphProps => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(&mut cui, |ui| self.graph.properties_ui(ui, &mut self.doc));
            }
            EditorKind::RenderSettings => scrolled(&mut cui, &mut |ui| self.graph.render_ui(ui, &self.doc)),
            EditorKind::Empty => {
                let mut picked = None;
                scrolled(&mut cui, &mut |ui| picked = panels::empty_area(ui, shown));
                if let Some(kind) = picked {
                    self.layout.current_mut().root.set_editor(id, kind);
                }
            }
        }
    }

    // ------------------------------------------------------------ dialogs --

    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(job) = self.blocking_job() {
            let label = job.label.clone();
            let secs = job.started.elapsed().as_secs_f32();
            let progress = job.progress.as_ref().map(|p| p.fraction());
            let mut cancel = false;
            egui::Modal::new(Id::unique("busy")).show(ctx, |ui| {
                ui.set_width(360.0);
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new(label).bold());
                });
                if let Some(f) = progress {
                    ui.add(egui::ProgressBar::new(f).show_percentage());
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{secs:.0} s")).color(Palette::of(ui).weak));
                    if progress.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            cancel = ui.button("Cancel").clicked();
                        });
                    }
                });
            });
            if cancel && let Some(job) = self.blocking_job() {
                job.cancel();
            }
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
            return;
        }
        if std::mem::take(&mut self.tools.open_rules) && self.dialog.is_none() {
            self.dialog = Some(Dialog::AutoRules(self.tools.tool == Tool::AutoPaint));
        }
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        match dialog {
            Dialog::Preferences => {
                let response = egui::Modal::new(Id::unique("preferences")).show(ctx, |ui| {
                    ui.set_width(440.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new("Settings").bold().size(16.0));
                    ui.add_space(12.0);
                    ui.label(RichText::new("Appearance").size(11.5).color(pal.weak));
                    ui.add_space(6.0);
                    row(ui, "Theme", |ui| {
                        for (choice, label) in [(ThemeChoice::System, "System"), (ThemeChoice::Light, "Light"), (ThemeChoice::Dark, "Dark")] {
                            if ui.selectable_label(self.settings.theme == choice, label).clicked() {
                                self.settings.theme = choice;
                            }
                        }
                    });
                    ui.add_space(6.0);
                    row(ui, "Accent", |ui| {
                        let mut chosen = None;
                        for (name, rgb) in theme::ACCENTS {
                            let (rect, swatch) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                            let on = self.settings.accent == rgb;
                            ui.painter().circle_filled(rect.center(), 9.0, theme::rgb(rgb));
                            if on || swatch.hovered() {
                                ui.painter().circle_stroke(rect.center(), 11.0, Stroke::new(1.5, if on { pal.text } else { pal.weak }));
                            }
                            if swatch.on_hover_text(name).clicked() {
                                chosen = Some(rgb);
                            }
                        }
                        ui.add_space(6.0);
                        let mut custom = self.settings.accent;
                        if egui::color_picker::color_edit_button_srgb(ui, &mut custom).on_hover_text("Any colour").changed() {
                            chosen = Some(custom);
                        }
                        if let Some(rgb) = chosen {
                            self.settings.accent = rgb;
                            theme::set_accent(ctx, rgb);
                        }
                    });
                    ui.label(
                        RichText::new("The colour of what is chosen, of the main buttons and of the home screen's art.")
                            .size(11.5)
                            .color(pal.weak),
                    );
                    ui.add_space(14.0);
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::primary_button(ui, "Done")).clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if keep {
                    self.dialog = Some(Dialog::Preferences);
                }
            }
            Dialog::AutoRules(mut paint) => {
                let response = egui::Modal::new(Id::unique("auto rules")).show(ctx, |ui| {
                    ui.set_width(700.0);
                    ui.label(
                        RichText::new("Automatic biomes and paint")
                            .bold()
                            .size(15.0),
                    );
                    ui.label(
                        RichText::new("Biomes from the land, ground from the biomes.")
                            .size(11.5)
                            .color(Palette::of(ui).weak),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut paint,
                            false,
                            theme::with_icon(Icon::Earth, "Biomes from the land"),
                        );
                        ui.selectable_value(
                            &mut paint,
                            true,
                            theme::with_icon(Icon::WandSparkles, "Paint from the biomes"),
                        );
                    });
                    ui.separator();
                    egui::ScrollArea::vertical()
                        .max_height(440.0)
                        .auto_shrink([false, true])
                        .show(ui, |ui| {
                            if paint {
                                panels::paint_rules(
                                    ui,
                                    &mut self.tools.auto.paint,
                                    &self.doc.world.registry,
                                );
                            } else {
                                panels::land_rules(
                                    ui,
                                    &mut self.tools.auto.land,
                                    &self.doc.world.registry,
                                );
                            }
                        });
                    ui.separator();
                    ui.horizontal(|ui| {
                        if ui.button("Reset to defaults").clicked() {
                            if paint {
                                self.tools.auto.paint = Default::default();
                            } else {
                                self.tools.auto.land = Default::default();
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, "Done")).clicked() {
                                keep = false;
                            }
                            let label = if paint {
                                "Paint the whole map"
                            } else {
                                "Give the whole map its biomes"
                            };
                            if ui
                                .button(label)
                                .on_hover_text("One undoable step")
                                .clicked()
                            {
                                let changed = if paint {
                                    self.doc.auto_paint(&self.tools.auto)
                                } else {
                                    self.doc.auto_biome(&self.tools.auto)
                                };
                                self.tools.auto_done = Some((!paint, changed));
                                self.feedback = Some(format!(
                                    "{} over the whole map: {changed} columns changed",
                                    if paint { "Auto paint" } else { "Auto biome" }
                                ));
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if keep {
                    self.dialog = Some(Dialog::AutoRules(paint));
                }
            }
            Dialog::Unsaved(pending) => {
                let mut choice = None;
                let response = egui::Modal::new(Id::unique("unsaved")).show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(format!("Save the changes to “{}”?", self.doc.world.meta.name)).bold().size(15.0));
                    ui.label(RichText::new("Your changes will be lost if you don't save them.").color(Palette::of(ui).weak));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Don't Save").clicked() {
                            choice = Some(1);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, "Save")).clicked() {
                                choice = Some(2);
                            }
                            if ui.button("Cancel").clicked() {
                                choice = Some(0);
                            }
                        });
                    });
                });
                match choice {
                    Some(1) => {
                        keep = false;
                        self.run(ctx, pending);
                    }
                    Some(2) => {
                        keep = false;
                        self.after_save = Some(pending);
                        if !self.save(ctx, false) {
                            self.after_save = None;
                        }
                    }
                    Some(_) => keep = false,
                    None => {
                        if response.should_close() {
                            keep = false;
                        } else {
                            self.dialog = Some(Dialog::Unsaved(pending));
                        }
                    }
                }
                if keep {
                }
            }
            Dialog::Export(mut form) => {
                let mut start = None;
                let response = egui::Modal::new(Id::unique("export")).show(ctx, |ui| {
                    ui.set_width(840.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new("Export to Minecraft").bold().size(16.0));
                    ui.add_space(6.0);
                    // The form scrolls when the screen is short; the tabs
                    // sit beside it behind a hairline as tall as the form
                    // (a separator here would take all the height there is,
                    // and the window would grow forever).
                    let max_height = (ctx.content_rect().height() - 200.0).max(240.0);
                    ui.horizontal_top(|ui| {
                        let form_rect = ui
                            .vertical(|ui| {
                                ui.set_width(700.0);
                                egui::ScrollArea::vertical()
                                    .id_salt("export form")
                                    .max_height(max_height)
                                    .show(ui, |ui| {
                                        ui.set_width(690.0);
                                        match form.tab {
                                            ExportTab::General => export_general(ui, &mut form),
                                            ExportTab::World => export_world(ui, &mut form),
                                        }
                                    });
                            })
                            .response
                            .rect;
                        ui.add_space(6.0);
                        let x = ui.cursor().min.x;
                        ui.painter()
                            .vline(x, form_rect.y_range(), Stroke::new(1.0, pal.border));
                        ui.add_space(7.0);
                        ui.vertical(|ui| {
                            ui.set_min_width(110.0);
                            for (tab, label) in
                                [(ExportTab::General, "General"), (ExportTab::World, "World")]
                            {
                                if ui
                                    .add_sized(
                                        [110.0, 24.0],
                                        egui::Button::selectable(form.tab == tab, label),
                                    )
                                    .clicked()
                                {
                                    form.tab = tab;
                                }
                            }
                        });
                    });
                    let target = form.parent.join(form.name.trim());
                    let exists = target.exists();
                    if exists {
                        ui.add_space(4.0);
                        ui.colored_label(pal.danger, "This folder already exists.");
                        ui.checkbox(
                            &mut form.overwrite,
                            "Replace its region files (everything else is kept)",
                        );
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let test = ui
                            .button("Test Export…")
                            .on_hover_text("Export 384 × 384 blocks around the view, to test it");
                        if test.clicked() {
                            start = Some(true);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                            let ok = !form.name.trim().is_empty() && (!exists || form.overwrite);
                            if ui
                                .add_enabled(ok, theme::primary_button(ui, "Export"))
                                .clicked()
                            {
                                start = Some(false);
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if let Some(test) = start {
                    let mut options = ExportOptions {
                        level_name: Some(form.name.trim().to_owned()),
                        game_mode: form.game_mode,
                        allow_commands: form.allow_cheats,
                        surroundings: form.surroundings,
                        generate_structures: form.structures,
                        overwrite: form.overwrite,
                        difficulty: form.difficulty,
                        hardcore: form.hardcore,
                        border: form.border,
                        datapacks: form.datapacks.clone(),
                        spawn: Some(form.spawn),
                        target: form.target.clone(),
                        ..ExportOptions::default()
                    };
                    let mut dir = form.parent.join(form.name.trim());
                    if test {
                        let c = TilePos::containing(form.test_centre.0, form.test_centre.1);
                        options.area = Some((c.offset(-1, -1), c.offset(1, 1)));
                        options.overwrite = false;
                        options.level_name = Some(format!("{} (test)", form.name.trim()));
                        dir = massif_anvil::unique_dir(&form.parent, &format!("{} test", form.name.trim()));
                        // Spawn in the middle of what is exported.
                        options.spawn = Some([form.test_centre.0, form.spawn[1].max(self.doc.world.meta.sea_level + 1), form.test_centre.1]);
                    }
                    let snapshot = self.doc.world.snapshot();
                    self.jobs.push(Job::export(ctx, snapshot, dir, options));
                } else if keep {
                    self.dialog = Some(Dialog::Export(form));
                }
            }
            Dialog::ImportReport(report, thumbnail) => {
                let mut save = false;
                let response = egui::Modal::new(Id::unique("import report")).show(ctx, |ui| {
                    ui.set_width(520.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new(format!("Imported “{}”", report.name)).bold().size(16.0));
                    let origin: Vec<&str> = [report.saved_by.as_deref(), report.platform.as_deref()].into_iter().flatten().collect();
                    if !origin.is_empty() {
                        ui.label(RichText::new(origin.join(" · ")).size(11.5).color(pal.weak));
                    }
                    ui.add_space(6.0);
                    if let Some(tex) = &thumbnail {
                        let size = tex.size_vec2();
                        let scale = (520.0 / size.x).min(220.0 / size.y).min(1.0);
                        ui.image((tex.id(), size * scale));
                        ui.add_space(6.0);
                    }
                    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
                        egui::Grid::new("import grid").num_columns(2).spacing([12.0, 4.0]).show(ui, |ui| {
                            ui.label(RichText::new("Imported").color(pal.weak));
                            ui.vertical(|ui| {
                                for d in &report.dimensions {
                                    ui.label(d);
                                }
                            });
                            ui.end_row();
                            ui.label(RichText::new("Terrains").color(pal.weak));
                            let total: u64 = report.terrains.iter().map(|t| t.1).sum::<u64>().max(1);
                            ui.vertical(|ui| {
                                ui.label(format!(
                                    "{} kinds, {} custom terrain{} rebuilt from their materials",
                                    report.terrains.len(),
                                    report.custom_terrains,
                                    if report.custom_terrains == 1 { "" } else { "s" }
                                ));
                                for (name, columns) in report.terrains.iter().take(10) {
                                    let pretty = if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                                        let lower = name.replace('_', " ").to_lowercase();
                                        let mut c = lower.chars();
                                        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                                    } else {
                                        name.clone()
                                    };
                                    ui.label(
                                        RichText::new(format!("{pretty} — {:.1}%", *columns as f64 * 100.0 / total as f64))
                                            .size(11.5)
                                            .color(pal.weak),
                                    );
                                }
                            });
                            ui.end_row();
                            if !report.layers.is_empty() {
                                ui.label(RichText::new("Not imported yet").color(pal.weak));
                                ui.vertical(|ui| {
                                    ui.label("Layers (Massif has no trees, plants or frost layers yet):");
                                    let names: Vec<String> = report.layers.iter().map(|(n, t)| format!("{n} ({t} tiles)")).collect();
                                    ui.label(RichText::new(names.join(", ")).size(11.5).color(pal.weak));
                                });
                                ui.end_row();
                            }
                            if !report.skipped_dimensions.is_empty() {
                                ui.label(RichText::new("Skipped").color(pal.weak));
                                ui.label(report.skipped_dimensions.join(", "));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, "Save as .massif…")).clicked() {
                                save = true;
                            }
                            if ui.button("Close").clicked() {
                                keep = false;
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if save {
                    self.save(ctx, true);
                } else if keep {
                    self.dialog = Some(Dialog::ImportReport(report, thumbnail));
                }
            }
            Dialog::About => {
                let response = egui::Modal::new(Id::unique("about")).show(ctx, |ui| {
                    ui.set_width(400.0);
                    ui.label(
                        RichText::new(format!("{}  Massif", theme::glyph(Icon::MountainSnow)))
                            .bold()
                            .size(18.0),
                    );
                    ui.label(
                        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION")))
                            .color(Palette::of(ui).weak),
                    );
                    ui.add_space(6.0);
                    ui.label("A map editor for Minecraft worlds of any size.");
                    ui.add_space(6.0);
                    ui.add_space(8.0);
                    if ui.button("Close").clicked() {
                        keep = false;
                    }
                });
                if response.should_close() {
                    keep = false;
                }
                if keep {
                    self.dialog = Some(Dialog::About);
                }
            }
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.is_modified() && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Unsaved(Pending::Quit));
        }
    }

    /// One frame of the whole UI.
    pub fn frame_ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let ctx = ui.ctx().clone();
        if self.applied_theme != Some(self.settings.theme) {
            self.apply_theme(&ctx);
            self.applied_theme = Some(self.settings.theme);
        }
        self.poll_jobs(&ctx);
        if self.brushes.poll() {
            // The brush picked last time is an image: find it again.
            let source = self.tools.brush_source.clone();
            if !panels::select_brush(&mut self.tools, &self.brushes, source) {
                panels::select_brush(&mut self.tools, &self.brushes, BrushSource::default());
            }
            // The 3D Sculpt editor's too.
            let sculpt = &mut self.tools.sculpt;
            match self.brushes.brush(&sculpt.source, &sculpt.brush) {
                Some(brush) => sculpt.brush = brush,
                None => {
                    sculpt.source = BrushSource::default();
                    if let Some(brush) = self.brushes.brush(&sculpt.source, &sculpt.brush) {
                        sculpt.brush = brush;
                    }
                }
            }
        }
        // The game's blocks give the world's blocks and terrains their real
        // colours.
        self.blocks.poll();
        if let Some(cat) = self.blocks.catalogue.clone()
            && self.blocks.applied != Some(self.doc.generation)
        {
            massif_assets::apply_colors(&mut self.doc.world.registry, &cat);
            self.blocks.applied = Some(self.doc.generation);
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
        }
        self.handle_close(&ctx);
        self.shortcuts(&ctx);
        self.map_cache.sync_generation(self.doc.generation);
        if let Some(msg) = self.feedback.take() {
            self.say(&ctx, msg, false, None);
        }
        if self.home.is_some() {
            self.home_ui(ui, frame, &ctx);
            self.dialogs(&ctx);
            if self.title != "Massif" {
                ctx.send_viewport_cmd(ViewportCommand::Title("Massif".to_owned()));
                self.title = "Massif".to_owned();
            }
            return;
        }

        let pal = Palette::of(ui);
        let margin = if self.unified_titlebar { 9 } else { 6 };
        let bar = egui::Panel::top("menu bar")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin::symmetric(10, margin)))
            .show(ui, |ui| {
                ui.horizontal(|ui| self.top_bar(ui));
            });
        if self.unified_titlebar {
            frame.set_traffic_lights_position(&ctx, bar.response.rect.y_range(), 14.0);
        }
        egui::Panel::top("workspaces")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin { left: 10, right: 12, top: 0, bottom: 2 }))
            .show(ui, |ui| self.workspace_tabs(ui));
        egui::Panel::bottom("status bar")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin::symmetric(14, 5)))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 0 }))
            .show(ui, |ui| self.dock(ui, frame));

        self.dialogs(&ctx);

        let dirty = self.doc.take_dirty();
        if !dirty.is_empty() {
            self.map_cache.invalidate(&dirty);
            self.view3d.invalidate(&dirty);
            self.sculpt3d.invalidate(&dirty);
            ctx.request_repaint();
        }
        let title = format!("{}{} — Massif", self.doc.title(), if self.doc.is_modified() { " •" } else { "" });
        if title != self.title {
            ctx.send_viewport_cmd(ViewportCommand::Title(title.clone()));
            self.title = title;
        }
    }
}

impl eframe::App for MassifApp {
    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        self.frame_ui(ui, frame);
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.layout);
        eframe::set_value(storage, "massif.settings", &self.settings);
        eframe::set_value(storage, "massif.render", &self.graph.view.render);
        eframe::set_value(storage, "massif.tools", &self.tools);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// A titled group of settings, as in WorldPainter's dialogs.
fn card(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    ui.add_space(4.0);
    ui.label(RichText::new(title).bold());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(11.5).color(pal.weak));
    }
    egui::Frame::new()
        .fill(pal.panel)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(8)
        .inner_margin(egui::Margin::symmetric(12, 8))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(6.0);
}

/// A row of a card: a label on the left, the control on the right.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 20.0], egui::Label::new(label).truncate());
        add(ui);
    });
}

fn export_general(ui: &mut Ui, form: &mut ExportForm) {
    let pal = Palette::of(ui);
    card(ui, "Where to save it", "Its folder, and its name in Minecraft.", |ui| {
        row(ui, "Saves folder", |ui| {
            ui.add(egui::Label::new(RichText::new(form.parent.display().to_string()).size(11.5)).truncate());
            if ui.small_button("…").on_hover_text("Choose the folder").clicked()
                && let Some(dir) = rfd::FileDialog::new().set_directory(&form.parent).pick_folder()
            {
                form.parent = dir;
            }
        });
        row(ui, "Name", |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
        });
        row(ui, "Minecraft", |ui| {
            crate::widgets::combo("export version")
                .selected_text(format!("Java {}", form.target.name))
                .width(170.0)
                .height(320.0)
                .show_ui(ui, |ui| {
                    for (version, installed) in &form.versions {
                        let label = if *installed { format!("{}  ·  installed", version.name) } else { version.name.clone() };
                        if ui.selectable_label(form.target == *version, label).clicked() {
                            form.target = version.clone();
                        }
                    }
                });
            let (bottom, top) = form.heights;
            let note = match form.target.era() {
                massif_anvil::Era::Modern if bottom < -64 || top > 320 => format!("today's chunks, Y {bottom} to {top}: a data pack gives the world that height"),
                massif_anvil::Era::Modern => format!("today's chunks, Y {bottom} to {top}"),
                massif_anvil::Era::Flat16 | massif_anvil::Era::Flat15 | massif_anvil::Era::Flat14 => "named blocks, 0 to 255".to_owned(),
                massif_anvil::Era::Numbered => "numbered blocks, 0 to 255".to_owned(),
            };
            ui.label(RichText::new(note).size(11.5).color(pal.weak));
        });
        let caveat = form.target.caveat(form.heights.0, form.heights.1);
        if !caveat.is_empty() {
            row(ui, "", |ui| {
                ui.add(egui::Label::new(RichText::new(format!("For {}, {caveat}.", form.target.name)).size(11.5).color(pal.weak)).wrap());
            });
        }
    });
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        card(&mut left[0], "Game", "The rules of the world when it is first played.", |ui| {
            row(ui, "Game mode", |ui| {
                crate::widgets::combo("game mode")
                    .selected_text(format!("{:?}", form.game_mode))
                    .show_ui(ui, |ui| {
                        for m in [GameMode::Survival, GameMode::Creative, GameMode::Adventure, GameMode::Spectator] {
                            ui.selectable_value(&mut form.game_mode, m, format!("{m:?}"));
                        }
                    });
            });
            row(ui, "Difficulty", |ui| {
                use massif_anvil::Difficulty;
                crate::widgets::combo("difficulty")
                    .selected_text(format!("{:?}", form.difficulty))
                    .show_ui(ui, |ui| {
                        for d in [Difficulty::Peaceful, Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
                            ui.selectable_value(&mut form.difficulty, d, format!("{d:?}"));
                        }
                    });
            });
            row(ui, "", |ui| {
                ui.checkbox(&mut form.allow_cheats, "Allow cheats");
            });
            row(ui, "", |ui| {
                ui.checkbox(&mut form.hardcore, "Hardcore");
            });
            row(ui, "", |ui| {
                ui.checkbox(&mut form.structures, "Generate structures beyond the map");
            });
        });
        card(&mut right[0], "World border", "The border Minecraft shows around the world.", |ui| {
            row(ui, "Centre", |ui| {
                ui.add(egui::DragValue::new(&mut form.border.center[0]).speed(16.0));
                ui.add(egui::DragValue::new(&mut form.border.center[1]).speed(16.0));
            });
            row(ui, "Size", |ui| {
                ui.add(egui::DragValue::new(&mut form.border.size).range(16.0..=60_000_000.0).speed(64.0).suffix(" blocks"));
            });
            row(ui, "", |ui| {
                if ui.small_button("Fit the map").on_hover_text("Centre the border on the map, just large enough for it").clicked()
                    && let Some((c, size)) = form.fit
                {
                    form.border.center = c;
                    form.border.size = size;
                }
            });
        });
    });
    card(ui, "Data packs", "Data packs copied into the world.", |ui| {
        ui.horizontal_top(|ui| {
            egui::Frame::new().stroke(Stroke::new(1.0, pal.border)).corner_radius(4).inner_margin(4).show(ui, |ui| {
                ui.set_min_size(vec2(ui.available_width() - 40.0, 90.0));
                if form.datapacks.is_empty() {
                    ui.label(RichText::new("No data packs").color(pal.weak));
                }
                for (i, p) in form.datapacks.iter().enumerate() {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.selectable_label(form.selected_pack == Some(i), name).on_hover_text(p.display().to_string()).clicked() {
                        form.selected_pack = Some(i);
                    }
                }
            });
            ui.vertical(|ui| {
                if ui.button(theme::glyph(Icon::Plus).to_string()).on_hover_text("Add data packs (zip files)").clicked()
                    && let Some(files) = rfd::FileDialog::new().add_filter("Data packs", &["zip"]).pick_files()
                {
                    form.datapacks.extend(files);
                }
                if ui.button(theme::glyph(Icon::FolderOpen).to_string()).on_hover_text("Add a data pack folder").clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    form.datapacks.push(dir);
                }
                if ui
                    .add_enabled(form.selected_pack.is_some(), egui::Button::new(theme::glyph(Icon::Trash2).to_string()))
                    .on_hover_text("Remove")
                    .clicked()
                    && let Some(i) = form.selected_pack.take()
                    && i < form.datapacks.len()
                {
                    form.datapacks.remove(i);
                }
            });
        });
    });
}

fn export_world(ui: &mut Ui, form: &mut ExportForm) {
    let pal = Palette::of(ui);
    card(ui, "Around the map", "What Minecraft generates beyond the edges of the map.", |ui| {
        ui.radio_value(&mut form.surroundings, Surroundings::Vanilla, "Vanilla terrain from the world's seed");
        ui.radio_value(&mut form.surroundings, Surroundings::Void, "Nothing: the map floats in the void");
    });
    card(ui, "Spawn point", "Where players appear the first time.", |ui| {
        row(ui, "x, y, z", |ui| {
            ui.add(egui::DragValue::new(&mut form.spawn[0]));
            ui.add(egui::DragValue::new(&mut form.spawn[1]).range(form.heights.0..=form.heights.1 - 1));
            ui.add(egui::DragValue::new(&mut form.spawn[2]));
        });
        ui.label(RichText::new("Spawn point for this export.").size(11.5).color(pal.weak));
    });
}

/// Where the layout is saved. A new key when the default layouts change a
/// lot, so that everyone gets the new editors.
const LAYOUT_KEY: &str = "massif.layout.v6";

/// Lay out the area tree.
fn compute(node: &Node, rect: Rect, path: &mut Vec<bool>, areas: &mut Vec<(AreaId, EditorKind, Rect)>, seps: &mut Vec<Sep>) {
    match node {
        Node::Area { id, editor } => areas.push((*id, *editor, rect)),
        Node::Split { dir, ratio, a, b } => {
            let (ra, rb, sep) = match dir {
                Dir::Row => {
                    let x = rect.left() + rect.width() * ratio;
                    (
                        Rect::from_min_max(rect.min, pos2(x - GAP / 2.0, rect.max.y)),
                        Rect::from_min_max(pos2(x + GAP / 2.0, rect.min.y), rect.max),
                        Rect::from_min_max(pos2(x - GAP, rect.min.y), pos2(x + GAP, rect.max.y)),
                    )
                }
                Dir::Column => {
                    let y = rect.top() + rect.height() * ratio;
                    (
                        Rect::from_min_max(rect.min, pos2(rect.max.x, y - GAP / 2.0)),
                        Rect::from_min_max(pos2(rect.min.x, y + GAP / 2.0), rect.max),
                        Rect::from_min_max(pos2(rect.min.x, y - GAP), pos2(rect.max.x, y + GAP)),
                    )
                }
            };
            seps.push(Sep {
                path: path.clone(),
                dir: *dir,
                rect: sep,
                parent: rect,
            });
            path.push(false);
            compute(a, ra, path, areas, seps);
            path.pop();
            path.push(true);
            compute(b, rb, path, areas, seps);
            path.pop();
        }
    }
}

fn node_at<'a>(node: &'a mut Node, path: &[bool]) -> &'a mut Node {
    match (node, path.split_first()) {
        (Node::Split { a, b, .. }, Some((&right, rest))) => node_at(if right { b } else { a }, rest),
        (node, _) => node,
    }
}

fn zone_of(r: Rect, p: egui::Pos2) -> Zone {
    let u = (p.x - r.left()) / r.width();
    let v = (p.y - r.top()) / r.height();
    if (0.3..0.7).contains(&u) && (0.3..0.7).contains(&v) {
        return Zone::Center;
    }
    let edges = [(u, Zone::Left), (1.0 - u, Zone::Right), (v, Zone::Top), (1.0 - v, Zone::Bottom)];
    edges
        .into_iter()
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|e| e.1)
        .unwrap_or(Zone::Center)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::home::parse_seed;
    use massif_core::generate::Landform;

    fn harness(theme: ThemeChoice) -> egui_kittest::Harness<'static, MassifApp> {
        egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                let world = generate(&GenParams {
                    tiles_x: 8,
                    tiles_z: 8,
                    landform: Landform::Islands,
                    ..GenParams::default()
                });
                let mut app = MassifApp::for_tests(world);
                app.settings.theme = theme;
                app
            })
    }

    fn map_rect(app: &MassifApp, ctx: &egui::Context) -> Rect {
        let areas: Vec<(AreaId, Rect)> = ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        areas
            .into_iter()
            .find(|(id, _)| app.layout.current().root.editor_of(*id) == Some(EditorKind::Map))
            .map(|(_, r)| r)
            .expect("the default workspace shows a map")
    }

    /// Render the whole window offscreen and paint on the map with the mouse.
    /// Needs a GPU, so it only runs when MASSIF_SNAPSHOT_DIR is set.
    #[test]
    fn window_renders_and_strokes_edit_the_world() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        for (theme, name) in [(ThemeChoice::Light, "ui-light.png"), (ThemeChoice::Dark, "ui-dark.png")] {
            let mut h = harness(theme);
            h.run_steps(40);
            h.render().unwrap().save(dir.join(name)).unwrap();
            let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
            eprintln!("3d {w}x{hgt} corner {:?} centre {:?}", &rgba[..4], &rgba[((hgt / 2 * w + w / 2) * 4) as usize..][..4]);
            image::save_buffer(dir.join(format!("3d-{name}")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            if theme == ThemeChoice::Dark {
                continue;
            }

            // A raise stroke across the middle of the map.
            let rect = map_rect(h.state(), &h.ctx);
            assert!(!h.state().doc.history.can_undo());
            let start = rect.center() - vec2(60.0, 0.0);
            let button = |pressed, pos| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            h.event(egui::Event::PointerMoved(start));
            h.step();
            h.event(button(true, start));
            h.step();
            for i in 1..=12 {
                h.event(egui::Event::PointerMoved(start + vec2(i as f32 * 10.0, 0.0)));
                h.step();
            }
            h.event(button(false, start + vec2(120.0, 0.0)));
            h.run_steps(20);
            assert_eq!(h.state().doc.history.undo_label(), Some("Raise / Lower"));
            assert!(h.state().doc.is_modified());
            h.render().unwrap().save(dir.join("ui-after-stroke.png")).unwrap();

            // Scrolling over the map sizes the brush, as in WorldPainter...
            let wheel = |dy: f32, modifiers| egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, dy),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers,
            };
            let r0 = h.state().tools.brush.radius;
            h.event(egui::Event::PointerMoved(rect.center()));
            h.step();
            h.event(wheel(80.0, Modifiers::NONE));
            h.run_steps(20);
            let r1 = h.state().tools.brush.radius;
            assert!(r1 < r0, "scrolling up should shrink the brush: {r0} -> {r1}");
            // ...and with Alt (or ⌘, or Ctrl) it zooms.
            let map = h.state().active_map.expect("the map was hovered");
            let z0 = h.state().maps[&map].zoom;
            h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
            h.event(wheel(-80.0, Modifiers::ALT));
            h.run_steps(20);
            h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
            let z1 = h.state().maps[&map].zoom;
            assert!(z1 < z0, "Alt + scrolling down should zoom out: {z0} -> {z1}");
            assert_eq!(h.state().tools.brush.radius, r1);
            // ⌘ and the wheel (the Windows key of a PC keyboard) turn the
            // brush, 5° a notch, without zooming.
            let turn = if cfg!(target_os = "macos") {
                Modifiers::MAC_CMD | Modifiers::COMMAND
            } else {
                Modifiers::CTRL | Modifiers::SHIFT | Modifiers::COMMAND
            };
            let a0 = h.state().tools.brush.rotation;
            h.event(egui::Event::ModifiersChanged(turn));
            h.event(wheel(80.0, turn));
            h.run_steps(20);
            h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
            let a1 = h.state().tools.brush.rotation;
            assert_eq!(a1, (a0 + 20.0).rem_euclid(360.0), "four notches up turn the brush 20° clockwise");
            assert_eq!(h.state().maps[&map].zoom, z1, "turning the brush does not zoom");
            assert_eq!(h.state().tools.brush.radius, r1);
            // Number keys set the intensity, Alt + a letter picks a tool.
            h.key_press(Key::Num3);
            h.key_press_modifiers(Modifiers::ALT, Key::A);
            h.run_steps(2);
            assert_eq!(h.state().tools.brush.strength, 0.3);
            assert_eq!(h.state().tools.tool, Tool::Flatten);

            // Undo through the keyboard.
            h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
            h.run_steps(4);
            assert!(!h.state().doc.history.can_undo());
        }
    }

    /// The brush editors: built-in brushes, stamps and the custom brushes of
    /// this computer (Massif's and WorldPainter's folders, only read), and a
    /// big custom brush drawn on the map. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn brush_editors_render_and_pick_brushes() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        for (theme, name) in [(ThemeChoice::Light, "brushes-light.png"), (ThemeChoice::Dark, "brushes-dark.png")] {
            let mut h = harness(theme);
            {
                let app = h.state_mut();
                app.brushes = BrushLibrary::installed();
                app.brushes.load_now();
                app.layout.active = 1; // Sculpting: brushes beside the map
            }
            h.run_steps(10);
            assert!(h.state().brushes.is_loaded());
            // Pick a stamp, make it big and turn it.
            {
                let app = h.state_mut();
                let source = BrushSource::Stamp(massif_core::stamps::Stamp::MountainChain);
                assert!(panels::select_brush(&mut app.tools, &app.brushes, source.clone()));
                assert_eq!(app.tools.brush_source, source);
                app.tools.brush.radius = 700.0;
                app.tools.brush.rotation = 30.0;
            }
            let rect = map_rect(h.state(), &h.ctx);
            h.event(egui::Event::PointerMoved(rect.center()));
            h.run_steps(20);
            h.render().unwrap().save(dir.join(name)).unwrap();
            // Brushes are no longer capped at 512 blocks: the wheel grows them on.
            let wheel = egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -200.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Modifiers::NONE,
            };
            h.event(wheel);
            h.run_steps(10);
            assert!(h.state().tools.brush.radius > 900.0, "{}", h.state().tools.brush.radius);
            assert!(h.state().tools.brush.radius <= massif_core::MAX_RADIUS);
            // Zoomed in, the outline follows the blocks the brush changes.
            {
                let app = h.state_mut();
                app.tools.brush.radius = 14.0;
                let id = app.active_map.expect("the map was hovered");
                app.maps.get_mut(&id).unwrap().zoom = 9.0;
            }
            h.event(egui::Event::PointerMoved(rect.center() + vec2(3.0, 2.0)));
            h.run_steps(10);
            h.render().unwrap().save(dir.join(format!("exact-{name}"))).unwrap();
            // A soft round brush shows where it presses hardest.
            {
                let app = h.state_mut();
                app.tools.brush.mask = None;
                app.tools.brush.shape = massif_core::BrushShape::Circle;
                app.tools.brush.profile = massif_core::Profile::Sine;
                app.tools.brush.radius = 90.0;
                app.tools.brush.rotation = 0.0;
                let id = app.active_map.expect("the map was hovered");
                app.maps.get_mut(&id).unwrap().zoom = 2.0;
            }
            h.event(egui::Event::PointerMoved(rect.center() + vec2(5.0, 3.0)));
            h.run_steps(10);
            h.render().unwrap().save(dir.join(format!("strength-{name}"))).unwrap();

            // The brush options, maximised: a height band, only on grass and
            // water, on slopes over 20°.
            {
                let app = h.state_mut();
                let grass = app.doc.world.registry.terrain_by_name("Grass").unwrap().0;
                app.tools.filter = massif_core::filter::Filter {
                    above: Some(40),
                    below: Some(120),
                    fade: true,
                    only_on: Some(vec![massif_core::filter::Match::Terrain(grass), massif_core::filter::Match::Water]),
                    slope_above: true,
                    slope: 20.0,
                    facing_on: true,
                    facing: 135.0,
                    facing_spread: 50.0,
                    ..Default::default()
                };
                let mut areas = Vec::new();
                app.layout.current().root.areas(&mut areas);
                let id = areas.iter().find(|(_, e)| *e == EditorKind::BrushSettings).unwrap().0;
                app.layout.toggle_maximize(id);
            }
            h.run_steps(10);
            h.render().unwrap().save(dir.join(format!("brush-settings-{name}"))).unwrap();
            let app = h.state_mut();
            let mut areas = Vec::new();
            app.layout.current().root.areas(&mut areas);
            let id = areas.iter().find(|(_, e)| *e == EditorKind::BrushSettings).unwrap().0;
            app.layout.toggle_maximize(id);
        }
    }

    /// Dig into the terrain from the 3D view, then build on it with the
    /// right button. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn sculpting_in_the_3d_view() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            app.tools.sculpt.tool = Tool::Build;
            app.tools.sculpt.brush.radius = 14.0;
            app.tools.sculpt.brush.strength = 1.0;
            app.sculpt3d.camera.distance = 160.0;
        }
        h.run_steps(30);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas
            .into_iter()
            .find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d))
            .map(|(_, r)| r)
            .expect("the 3D Sculpt workspace shows its editor");
        let at = view.center() + vec2(0.0, 20.0);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(3);
        assert!(h.state().sculpt3d.hover().is_some(), "the cursor points at the terrain");
        let press = |pressed, button| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // The right button digs.
        h.event(press(true, egui::PointerButton::Secondary));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.event(press(false, egui::PointerButton::Secondary));
        h.run_steps(30);
        assert_eq!(h.state().doc.history.undo_label(), Some("Sculpt"));
        assert!(!h.state().doc.dimension().voxels.is_empty());
        h.render().unwrap().save(dir.join("sculpt-dig.png")).unwrap();
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("sculpt-dig-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // Shift + drag moves the view, and sculpts nothing.
        let steps = h.state().doc.history.undo_count();
        let target = h.state().sculpt3d.camera.target;
        h.event(egui::Event::ModifiersChanged(Modifiers::SHIFT));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: at,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::SHIFT,
        });
        for k in 1..=10 {
            h.event(egui::Event::PointerMoved(at + vec2(k as f32 * 8.0, 0.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton {
            pos: at + vec2(80.0, 0.0),
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::SHIFT,
        });
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        h.run_steps(5);
        assert_eq!(h.state().doc.history.undo_count(), steps, "nothing sculpted");
        assert_ne!(h.state().sculpt3d.camera.target, target, "the view moved");
        // Alt + wheel: the size of the brush.
        let radius = h.state().tools.sculpt.brush.radius;
        h.event(egui::Event::PointerMoved(at));
        h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, 120.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: Modifiers::ALT,
        });
        h.run_steps(10);
        h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
        assert!(h.state().tools.sculpt.brush.radius > radius, "{} after {radius}", h.state().tools.sculpt.brush.radius);
        // The left button adds matter.
        h.event(egui::Event::PointerMoved(at));
        h.step();
        h.event(press(true, egui::PointerButton::Primary));
        for _ in 0..30 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.event(press(false, egui::PointerButton::Primary));
        h.run_steps(30);
        assert_eq!(h.state().doc.history.undo_label(), Some("Sculpt"));
        assert!(h.state().doc.history.can_undo());
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("sculpt-build-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Import a real WorldPainter world through the editor and show the
    /// report. Runs only with MASSIF_SNAPSHOT_DIR and MASSIF_WORLD_SAMPLES.
    #[test]
    fn worldpainter_worlds_import_through_the_editor() {
        let (Some(dir), Some(samples)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_WORLD_SAMPLES"),
        ) else {
            return;
        };
        let Some(path) = std::env::split_paths(&samples).next() else { return };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        let job = Job::open(&ctx, path.clone());
        assert_eq!(job.kind, jobs::JobKind::Import);
        h.state_mut().jobs.push(job);
        let started = std::time::Instant::now();
        while !h.state().jobs.is_empty() {
            assert!(started.elapsed().as_secs() < 120, "import timed out");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(40);
        let app = h.state();
        assert!(matches!(app.dialog, Some(Dialog::ImportReport(..))), "the import report is shown");
        assert!(!app.doc.dimension().ground().is_empty());
        assert!(app.doc.path.is_none(), "an imported world is saved as a new .massif file");
        h.render().unwrap().save(dir.join("ui-import.png")).unwrap();

        // The whole map in the 3D view, from far away, every block of it.
        {
            let app = h.state_mut();
            app.dialog = None;
            app.layout.active = 3;
            app.view3d.camera.distance = 5200.0;
            app.view3d.camera.pitch = 0.75;
        }
        let started = std::time::Instant::now();
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            assert!(started.elapsed().as_secs() < 60, "the 3D view did not finish building");
            h.step();
        }
        eprintln!("whole map built in the 3D view in {:.1} s", started.elapsed().as_secs_f32());
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("import-3d-whole.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        {
            let app = h.state_mut();
            app.view3d.camera.distance = 420.0;
            app.view3d.camera.pitch = 0.35;
        }
        for _ in 0..200 {
            h.step();
        }
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("import-3d-close.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Step until the terrain graph's preview has caught up with the graph.
    fn wait_for_graph(h: &mut egui_kittest::Harness<'static, MassifApp>) {
        let started = std::time::Instant::now();
        h.run_steps(3);
        while !h.state().graph.is_current() {
            assert!(started.elapsed().as_secs() < 90, "the preview did not come");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        h.run_steps(4);
    }

    /// The terrain graph: previews of every node and the viewport, then the
    /// conversion to the map, with real blocks from its colours. Needs a
    /// GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn terrain_graph_previews_and_converts() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.layout.activate(7); // Graph, on the terrain side
            let reg = &app.doc.world.registry;
            let get = |name: &str| reg.terrain_by_name(name).unwrap_or(massif_core::TerrainId::ROCK).0;
            let starter = massif_core::graph::Graph::starter(get("Grass"), massif_core::TerrainId::ROCK.0, get("Snow"), get("Gravel"));
            app.doc.world.meta.graph = Some(starter);
            app.graph.graph_replaced();
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("graph.png")).unwrap();
        let (w, hgt, rgba) = h.state().graph.view.read_back().expect("the viewport rendered");
        image::save_buffer(dir.join("graph-viewport.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // The relief fills a good part of the viewport.
        let distinct: std::collections::HashSet<[u8; 3]> = rgba.as_chunks::<4>().0.iter().map(|p| [p[0], p[1], p[2]]).collect();
        assert!(distinct.len() > 2000, "{} colours in the viewport", distinct.len());
        // A mask, selected, shows on the relief.
        let slope = h
            .state()
            .doc
            .world
            .meta
            .graph
            .as_ref()
            .and_then(|g| g.nodes.iter().find(|n| matches!(n.kind, massif_core::graph::NodeKind::SelectSlope { .. })).map(|n| n.id));
        if let Some(id) = slope {
            h.state_mut().graph.view(Some(id));
            wait_for_graph(&mut h);
            h.render().unwrap().save(dir.join("graph-mask.png")).unwrap();
        }
        h.state_mut().graph.view(None);
        // The Render workspace: the view, with its sun, sky and clouds.
        h.state_mut().layout.activate(8);
        h.run_steps(5);
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("graph-render-workspace.png")).unwrap();
        h.state_mut().layout.activate(7);
        h.run_steps(3);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("every block");
        }
        h.state_mut().graph.draft = Some(512);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        let started = std::time::Instant::now();
        while h.state().doc.history.undo_label() != Some("Terrain graph") {
            assert!(started.elapsed().as_secs() < 120, "the conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(10);
        h.render().unwrap().save(dir.join("graph-applied.png")).unwrap();
        // The colours became several kinds of ground.
        let tile = h.state().doc.dimension().ground().view(TilePos::new(0, 0)).unwrap();
        let kinds: std::collections::HashSet<u16> = (0..128 * 128).map(|i| tile.terrain.get(i)).collect();
        assert!(kinds.len() >= 3, "{} terrains", kinds.len());
        // And the map in 3D.
        h.state_mut().layout.active = 3;
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("graph-applied-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Graphs in the manner of Gaea's examples, as the viewport shows them:
    /// a cratered moon, a desert canyon, eroded peaks. Needs a GPU: runs
    /// with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn gaea_like_graphs_render() {
        use massif_core::graph::*;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let moon = {
            let mut g = Graph::default();
            let craters = g.add(
                NodeKind::CraterField {
                    count: 260,
                    min_radius: 10.0,
                    max_radius: 190.0,
                    depth: 0.28,
                    base: 90.0,
                    seed: 4,
                },
                [0.0, 0.0],
            );
            let range = g.add(
                NodeKind::MountainRange {
                    angle: 35.0,
                    width: 420.0,
                    scale: 260.0,
                    height: 110.0,
                    base: 0.0,
                    seed: 2,
                },
                [0.0, 200.0],
            );
            let add = g.add(NodeKind::Combine { op: CombineOp::Add, weight: 1.0 }, [220.0, 100.0]);
            let eroded = g.add(
                NodeKind::Erosion {
                    rain: 0.8,
                    erosion: 0.25,
                    deposition: 0.3,
                    capacity: 4.0,
                    evaporation: 0.02,
                    radius: 4.0,
                    seed: 1,
                },
                [440.0, 100.0],
            );
            let grad = g.add(
                NodeKind::Gradient {
                    preset: GradientPreset::Lunar,
                    auto_range: true,
                    min: 0.0,
                    max: 1.0,
                    reverse: false,
                    variation: 0.3,
                    seed: 1,
                },
                [660.0, 160.0],
            );
            let occ = g.add(NodeKind::Occlusion { radius: 24.0 }, [660.0, 320.0]);
            let dark = g.add(NodeKind::Invert, [860.0, 320.0]);
            let tint = g.add(
                NodeKind::Tint {
                    color: [60, 62, 66],
                    mode: TintMode::Multiply,
                    strength: 0.7,
                },
                [880.0, 160.0],
            );
            let out = g.add(NodeKind::HeightOutput, [660.0, 0.0]);
            let color = g.add(
                NodeKind::ColorOutput {
                    blocks: true,
                    palette: BlockPalette::Rock,
                },
                [1100.0, 160.0],
            );
            g.connect(craters, 0, add, 0);
            g.connect(range, 0, add, 1);
            g.connect(add, 0, eroded, 0);
            g.connect(eroded, 0, out, 0);
            g.connect(eroded, 0, grad, 0);
            g.connect(eroded, 0, occ, 0);
            g.connect(occ, 0, dark, 0);
            g.connect(grad, 0, tint, 0);
            g.connect(dark, 0, tint, 1);
            g.connect(tint, 0, color, 0);
            g
        };
        let canyon = {
            let mut g = Graph::default();
            let canyon = g.add(
                NodeKind::Canyon {
                    angle: 30.0,
                    width: 300.0,
                    depth: 100.0,
                    meander: 160.0,
                    steps: 7,
                    base: 170.0,
                    seed: 3,
                },
                [0.0, 0.0],
            );
            let rocks = g.add(
                NodeKind::Rocks {
                    scale: 60.0,
                    height: 8.0,
                    sharpness: 0.7,
                    seed: 2,
                },
                [0.0, 200.0],
            );
            let add = g.add(NodeKind::Combine { op: CombineOp::Add, weight: 1.0 }, [220.0, 100.0]);
            let strata = g.add(
                NodeKind::Stratify {
                    layers: 18,
                    strength: 0.6,
                    tilt: 4.0,
                    seed: 5,
                },
                [440.0, 100.0],
            );
            let grad = g.add(
                NodeKind::Gradient {
                    preset: GradientPreset::RedRock,
                    auto_range: true,
                    min: 0.0,
                    max: 1.0,
                    reverse: false,
                    variation: 0.4,
                    seed: 2,
                },
                [660.0, 160.0],
            );
            let out = g.add(NodeKind::HeightOutput, [660.0, 0.0]);
            let color = g.add(
                NodeKind::ColorOutput {
                    blocks: true,
                    palette: BlockPalette::Desert,
                },
                [880.0, 160.0],
            );
            g.connect(canyon, 0, add, 0);
            g.connect(rocks, 0, add, 1);
            g.connect(add, 0, strata, 0);
            g.connect(strata, 0, out, 0);
            g.connect(strata, 0, grad, 0);
            g.connect(grad, 0, color, 0);
            g
        };
        let peaks = {
            let mut g = Graph::default();
            let mountain = g.add(
                NodeKind::Mountain {
                    style: MountainStyle::Alpine,
                    x: 0.5,
                    z: 0.5,
                    radius: 520.0,
                    height: 230.0,
                    base: 50.0,
                    detail: 0.6,
                    seed: 8,
                },
                [0.0, 0.0],
            );
            let eroded = g.add(
                NodeKind::Erosion {
                    rain: 1.2,
                    erosion: 0.35,
                    deposition: 0.3,
                    capacity: 4.0,
                    evaporation: 0.02,
                    radius: 4.0,
                    seed: 3,
                },
                [220.0, 0.0],
            );
            let snow = g.add(
                NodeKind::Snow {
                    line: 190.0,
                    slope: 42.0,
                    depth: 2.0,
                    softness: 14.0,
                },
                [440.0, 0.0],
            );
            let color = g.add(
                NodeKind::AutoColor {
                    palette: AutoPalette::Alpine,
                    snow_line: 185.0,
                    rock_slope: 36.0,
                    seed: 2,
                },
                [660.0, 120.0],
            );
            let out = g.add(NodeKind::HeightOutput, [660.0, 0.0]);
            let color_out = g.add(
                NodeKind::ColorOutput {
                    blocks: true,
                    palette: BlockPalette::Natural,
                },
                [880.0, 120.0],
            );
            g.connect(mountain, 0, eroded, 0);
            g.connect(eroded, 0, snow, 0);
            g.connect(snow, 0, out, 0);
            g.connect(snow, 0, color, 0);
            g.connect(color, 0, color_out, 0);
            g
        };
        use crate::terrain_view::RenderSettings;
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.active = 7;
        let sunset = RenderSettings {
            sun_azimuth: 250.0,
            sun_elevation: 9.0,
            haze: 0.4,
            ..RenderSettings::default()
        };
        let cloudy = RenderSettings {
            clouds: true,
            cloud_base: 170.0,
            cloud_thickness: 90.0,
            cloud_cover: 0.5,
            cloud_size: 260.0,
            sea: true,
            ..RenderSettings::default()
        };
        for (k, (name, graph, render)) in [("moon", moon, RenderSettings::default()), ("canyon", canyon, sunset), ("peaks", peaks, cloudy)]
            .into_iter()
            .enumerate()
        {
            {
                let app = h.state_mut();
                app.doc.world.meta.graph = Some(graph);
                app.graph.graph_replaced();
                app.graph.preview = 512;
                app.graph.view.render = render;
            }
            wait_for_graph(&mut h);
            let (w, hgt, rgba) = h.state().graph.view.read_back().expect("the viewport rendered");
            image::save_buffer(dir.join(format!("graph-{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            if k == 0 {
                h.render().unwrap().save(dir.join("graph-ui.png")).unwrap();
            }
        }
        // A saved image, at full quality.
        let (w, hgt, rgba) = h.state_mut().graph.view.render_image([1280, 720]).expect("an image");
        assert_eq!((w, hgt), (1280, 720));
        image::save_buffer(dir.join("graph-render.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// The automatic tools over the whole map: biomes read from the land,
    /// shown by the biome mode, then the paint that follows a changed rule;
    /// and the dialog of their rules. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn auto_biome_and_paint_follow_their_rules() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        let rect = map_rect(h.state(), &h.ctx);
        h.event(egui::Event::PointerMoved(rect.center()));
        h.run_steps(3);
        let changed = {
            let app = h.state_mut();
            app.doc.auto_biome(&app.tools.auto)
        };
        assert!(changed > 100_000, "{changed} columns got a biome");
        assert_eq!(h.state().doc.history.undo_label(), Some("Auto biome"));
        // The sea and the land have their own biomes.
        let ids: std::collections::HashSet<String> = {
            let app = h.state();
            let reg = &app.doc.world.registry;
            let ground = app.doc.dimension().ground();
            ground
                .positions()
                .flat_map(|pos| {
                    let tile = ground.view(pos).unwrap();
                    (0..128 * 128).step_by(97).map(|i| reg.biome(massif_core::BiomeId(tile.biome.get(i))).id.clone()).collect::<Vec<_>>()
                })
                .collect()
        };
        for wanted in ["minecraft:ocean", "minecraft:plains", "minecraft:beach"] {
            assert!(ids.contains(wanted), "{wanted} among {ids:?}");
        }
        {
            let app = h.state_mut();
            let id = app.active_map.expect("the map was hovered");
            app.maps.get_mut(&id).unwrap().mode = crate::map_render::MapMode::Biome;
        }
        h.run_steps(20);
        h.render().unwrap().save(dir.join("auto-biomes.png")).unwrap();
        // Plains of podzol, by a rule of the settings.
        let painted = {
            let app = h.state_mut();
            app.tools.auto.paint.of_mut("minecraft:plains").ground = "Podzol".into();
            app.doc.auto_paint(&app.tools.auto)
        };
        assert!(painted > 10_000, "{painted} columns painted");
        {
            let app = h.state_mut();
            let id = app.active_map.unwrap();
            app.maps.get_mut(&id).unwrap().mode = crate::map_render::MapMode::Terrain;
            app.dialog = Some(Dialog::AutoRules(false));
        }
        h.run_steps(10);
        h.render().unwrap().save(dir.join("auto-rules-land.png")).unwrap();
        h.state_mut().dialog = Some(Dialog::AutoRules(true));
        h.run_steps(10);
        h.render().unwrap().save(dir.join("auto-rules-paint.png")).unwrap();
    }

    /// A very large map in the 3D view: drawn whole, a column every few
    /// blocks. Runs with MASSIF_SNAPSHOT_DIR and MASSIF_GIANT_3D (the size
    /// in blocks, e.g. "24000x24000"); best with `--release`.
    #[test]
    fn giant_maps_in_3d() {
        let (Some(dir), Ok(size)) = (std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from), std::env::var("MASSIF_GIANT_3D")) else {
            return;
        };
        let (w, d) = size.split_once('x').map(|(w, d)| (w.parse::<i32>().unwrap(), d.parse::<i32>().unwrap())).expect("WxD");
        let mut h = harness(ThemeChoice::Dark);
        let started = std::time::Instant::now();
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: (w + 127) / 128,
                tiles_z: (d + 127) / 128,
                landform: Landform::Islands,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 11);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.layout.activate(3);
        }
        eprintln!("generated in {:.1} s", started.elapsed().as_secs_f32());
        h.run_steps(5);
        let started = std::time::Instant::now();
        while h.state().view3d.pending() > 0 {
            assert!(started.elapsed().as_secs() < 600, "the 3D view did not finish");
            h.step();
        }
        h.run_steps(3);
        eprintln!("3D view built in {:.1} s", started.elapsed().as_secs_f32());
        h.render().unwrap().save(dir.join("giant-3d-ui.png")).unwrap();
        let (iw, ih, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("giant-3d.png"), &rgba, iw, ih, image::ColorType::Rgba8).unwrap();
        // The map view too, zoomed out over all of it.
        h.state_mut().layout.activate(0);
        let started = std::time::Instant::now();
        h.run_steps(40);
        eprintln!("map view: {:.1} s for 40 frames", started.elapsed().as_secs_f32());
        h.render().unwrap().save(dir.join("giant-map.png")).unwrap();
    }

    /// Paint with a pattern: one stroke lays several terrains out by a
    /// noise, each over its share. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn patterns_paint_the_ground() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 8,
                tiles_z: 8,
                landform: Landform::Hills,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 9);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::PaintTerrain);
            app.tools.brush.radius = 140.0;
            app.tools.brush.strength = 1.0;
            app.tools.brush.profile = massif_core::Profile::Constant;
            app.tools.pattern.on = true;
            app.tools.pattern.pattern.kind = massif_core::pattern::PatternKind::Patches;
            app.tools.pattern.pattern.scale = 14.0;
            app.tools.pattern.terrains = vec![("Grass".into(), 2.0), ("Podzol".into(), 1.0), ("Gravel".into(), 1.0)];
        }
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let c = rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos: c,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.event(egui::Event::PointerMoved(c));
        h.step();
        h.event(button(true));
        h.run_steps(4);
        h.event(button(false));
        h.run_steps(10);
        assert_eq!(h.state().doc.history.undo_label(), Some("Paint terrain"));
        // Under the stroke, the three terrains.
        let app = h.state();
        let reg = &app.doc.world.registry;
        let at = app.maps.get(&app.active_map.unwrap()).unwrap().center;
        let (cx, cz) = (at[0].floor() as i32, at[1].floor() as i32);
        let mut seen = std::collections::HashMap::new();
        for dz in -40..40 {
            for dx in -40..40 {
                let (x, z) = (cx + dx, cz + dz);
                let tile = app.doc.dimension().ground().view(TilePos::containing(x, z)).unwrap();
                let name = reg.terrain(massif_core::TerrainId(tile.terrain.get(massif_core::coords::local_index(x, z)))).name.clone();
                *seen.entry(name).or_insert(0usize) += 1;
            }
        }
        for name in ["Grass", "Podzol", "Gravel"] {
            assert!(seen.get(name).copied().unwrap_or(0) > 400, "{name}: {seen:?}");
        }
        h.event(egui::Event::PointerMoved(c + vec2(500.0, 0.0)));
        h.run_steps(10);
        h.render().unwrap().save(dir.join("pattern-paint.png")).unwrap();
    }

    /// The River tool: points clicked on the map, Enter, and a river is dug
    /// with water running down it. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn rivers_are_drawn_on_the_map() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 8,
                tiles_z: 8,
                landform: Landform::Hills,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 7);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::River);
            app.tools.river.width = 10.0;
            app.tools.river.width_end = 40.0;
        }
        h.run_steps(5);
        let (source, end) = river_ends(h.state());
        let at = |h: &egui_kittest::Harness<'_, MassifApp>, p: (f32, f32)| h.state().maps.values().next().unwrap().screen_of(p.0 as f64, p.1 as f64);
        let p = at(&h, source);
        click_on(&mut h, p, egui::PointerButton::Primary);
        h.render().unwrap().save(dir.join("river-source.png")).unwrap();
        let p = at(&h, end);
        click_on(&mut h, p, egui::PointerButton::Primary);
        // Bent through a point dragged off its course.
        let course = h.state().tools.river_draft.as_ref().unwrap().plan.course.clone();
        let mid = at(&h, course[course.len() / 2]);
        drag_on(&mut h, mid, mid + vec2(60.0, -40.0));
        assert_eq!(h.state().tools.river_draft.as_ref().unwrap().layout.through.len(), 1);
        h.event(egui::Event::PointerMoved(mid + vec2(60.0, -40.0)));
        h.run_steps(3);
        h.render().unwrap().save(dir.join("river-course.png")).unwrap();
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(10);
        assert_eq!(h.state().doc.history.undo_label(), Some("River"));
        assert!(h.state().tools.river_draft.is_none());
        let note = h.state().tools.river_note.clone().unwrap_or_default();
        assert!(note.starts_with("Dug a river"), "{note}");
        h.render().unwrap().save(dir.join("river-dug.png")).unwrap();
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.distance = 420.0;
            app.view3d.camera.pitch = 0.55;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("river-dug-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Rivers on the map, converted over it: the ground is left as it is,
    /// the beds and their water are in levels of the graph's own, written
    /// again by the next conversion. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn rivers_are_converted_over_the_map() {
        use massif_core::graph::Template;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        {
            let app = h.state_mut();
            app.graph.preview = 512;
            app.graph.use_template(&mut app.doc, Template::MapRivers);
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("rivers-graph.png")).unwrap();
        // The ground, before.
        let ground = |app: &MassifApp| -> Vec<f32> {
            let level = app.doc.dimension().ground();
            let mut tiles: Vec<_> = level.positions().collect();
            tiles.sort_by_key(|p| (p.z, p.x));
            tiles.iter().flat_map(|p| (0..128 * 128).step_by(61).map(|i| level.view(*p).unwrap().top.get(i)).collect::<Vec<_>>()).collect()
        };
        let before = ground(h.state());
        let convert = |h: &mut egui_kittest::Harness<'_, MassifApp>| {
            {
                use egui_kittest::kittest::Queryable;
                h.get_by_label_contains("Convert to voxels").click();
            }
            h.run_steps(2);
            assert!(h.state().graph.converting(), "the conversion started");
            let started = std::time::Instant::now();
            while h.state().graph.converting() {
                assert!(started.elapsed().as_secs() < 120, "the conversion did not finish");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            h.run_steps(3);
        };
        convert(&mut h);
        assert_eq!(h.state().doc.history.undo_label(), Some("Terrain graph"));
        assert_eq!(ground(h.state()), before, "the ground of the map did not move");
        let names: Vec<String> = h.state().doc.dimension().levels.iter().map(|l| l.name.clone()).collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"Dug (graph 0)".to_owned()) && names.contains(&"Raised (graph 0)".to_owned()), "{names:?}");
        // Water in the beds, above the sea.
        let sea = h.state().doc.world.meta.sea_level as i16;
        let wet = {
            let dim = h.state().doc.dimension();
            let dug = dim.levels.iter().find(|l| l.name == "Dug (graph 0)").unwrap();
            dug.positions()
                .map(|p| {
                    let tile = dug.view(p).unwrap();
                    (0..128 * 128).filter(|&i| tile.present(i) && tile.fluid.get(i) > sea).count()
                })
                .sum::<usize>()
        };
        assert!(wet > 200, "{wet} columns of river water above the sea");
        h.render().unwrap().save(dir.join("rivers-converted.png")).unwrap();
        // The map in 3D, with its rivers.
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.distance = 520.0;
            app.view3d.camera.pitch = 0.6;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("rivers-converted-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // Again: the same levels, written again; and into the map when
        // asked, the levels gone.
        h.state_mut().layout.activate(7);
        h.run_steps(3);
        convert(&mut h);
        assert_eq!(h.state().doc.dimension().levels.len(), 3);
        assert_eq!(ground(h.state()), before);
        h.state_mut().graph.over = Some(false);
        h.run_steps(2);
        convert(&mut h);
        assert_eq!(h.state().doc.dimension().levels.len(), 1, "written into the ground: no level of the graph's left");
        assert_ne!(ground(h.state()), before, "the beds are dug in the ground itself");
    }

    /// The two templates that start from the map, on a map with real
    /// relief (an alpine range converted into it) and a tower of rock as a
    /// level: the Map node sees the tower; painting paints its top and the
    /// ground, without moving either; rivers are dug into the valleys.
    /// Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn map_templates_on_a_real_map() {
        use massif_core::graph::Template;
        use massif_core::level::LevelKind;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        let convert = |h: &mut egui_kittest::Harness<'_, MassifApp>| {
            {
                use egui_kittest::kittest::Queryable;
                h.get_by_label_contains("Convert to voxels").click();
            }
            h.run_steps(2);
            assert!(h.state().graph.converting(), "the conversion started");
            let started = std::time::Instant::now();
            while h.state().graph.converting() {
                assert!(started.elapsed().as_secs() < 180, "the conversion did not finish");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            h.run_steps(3);
        };
        let shot_3d = |h: &mut egui_kittest::Harness<'static, MassifApp>, name: &str| {
            {
                let app = h.state_mut();
                app.layout.activate(3);
                app.view3d.camera.distance = 900.0;
                app.view3d.camera.pitch = 0.62;
                app.view3d.camera.yaw = 0.7;
            }
            h.run_steps(5);
            while h.state().view3d.pending() > 0 {
                h.step();
            }
            h.run_steps(3);
            let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join(name), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            h.state_mut().layout.activate(7);
            h.run_steps(3);
        };
        // Mountains, into the map's ground.
        {
            let app = h.state_mut();
            app.graph.preview = 512;
            app.graph.use_template(&mut app.doc, Template::Alpine);
            app.graph.over = Some(false);
        }
        wait_for_graph(&mut h);
        convert(&mut h);
        // A tower of rock, 40 blocks wide, as a level over the ground.
        let (tower, centre) = {
            let app = h.state_mut();
            let doc = &mut app.doc;
            doc.history.begin("Tower", doc.dim);
            let tower = doc.history.edit(&mut doc.world, 0).structure().add_level("Tower", LevelKind::Solid);
            let (lo, _) = doc.dimension().bounds().unwrap();
            let (cx, cz) = (lo.x * 128 + 300, lo.z * 128 + 300);
            let ground = doc.dimension().ground().view(massif_core::TilePos::containing(cx, cz)).unwrap().top.get(massif_core::coords::local_index(cx, cz));
            let rect = massif_core::BlockRect::new(cx - 20, cz - 20, cx + 20, cz + 20);
            let tiles: Vec<_> = rect.tiles().collect();
            doc.history.edit(&mut doc.world, tower).par_tiles(tiles, rect, true, |_, clip, tile| {
                for z in clip.z0..clip.z1 {
                    for x in clip.x0..clip.x1 {
                        let i = massif_core::coords::local_index(x, z);
                        tile.top.set(i, ground + 70.0);
                        tile.bottom.as_mut().unwrap().set(i, ground - 4.0);
                    }
                }
            });
            doc.commit();
            (tower, (cx, cz))
        };
        // Painted from its relief.
        {
            let app = h.state_mut();
            app.graph.over = None;
            app.graph.use_template(&mut app.doc, Template::PaintMap);
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("paint-map-graph.png")).unwrap();
        let tower_top = |app: &MassifApp| {
            let t = app.doc.dimension().levels[tower].view(massif_core::TilePos::containing(centre.0, centre.1)).unwrap();
            let i = massif_core::coords::local_index(centre.0, centre.1);
            (t.top.get(i), t.terrain.get(i))
        };
        let before = tower_top(h.state());
        convert(&mut h);
        let after = tower_top(h.state());
        assert_eq!(after.0, before.0, "the tower stands where it was");
        assert_ne!(after.1, before.1, "its top was painted");
        shot_3d(&mut h, "paint-map-3d.png");
        // Rivers in its valleys.
        {
            let app = h.state_mut();
            app.graph.use_template(&mut app.doc, Template::MapRivers);
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("map-rivers-graph.png")).unwrap();
        convert(&mut h);
        shot_3d(&mut h, "map-rivers-3d.png");
    }

    /// A terrain larger than the map: the toolbar says its size, and
    /// converting it makes the world that size, the new tiles built like
    /// the others, with the sea where they lie under it. Needs a GPU: runs
    /// with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn a_terrain_larger_than_the_map_grows_it() {
        use massif_core::graph::Template;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        let before = h.state().doc.dimension().ground().len();
        {
            let app = h.state_mut();
            app.graph.preview = 384;
            app.graph.use_template(&mut app.doc, Template::Archipelago);
            let mut graph = app.doc.world.meta.graph.clone().unwrap();
            graph.size = Some(2048);
            app.doc.world.meta.graph = Some(graph);
            app.doc.mark_changed();
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("graph-size-2k.png")).unwrap();
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("every block");
        }
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        h.run_steps(2);
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 180, "the conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        let dim = h.state().doc.dimension();
        assert_eq!(before, 64, "a map of 1024 blocks");
        assert_eq!(dim.ground().len(), 256, "a world of 2048 blocks");
        let (lo, hi) = dim.bounds().unwrap();
        assert_eq!((hi.x - lo.x + 1, hi.z - lo.z + 1), (16, 16));
        // A new tile at a corner, built and under the sea where it is low.
        let corner = dim.ground().view(lo).unwrap();
        let sea = h.state().doc.world.meta.sea_level as i16;
        let wet = (0..128 * 128).filter(|&i| corner.fluid.get(i) == sea && corner.top.get(i) < sea as f32).count();
        let shaped = (0..128 * 128).map(|i| corner.top.get(i)).fold((f32::MAX, f32::MIN), |(a, b), v| (a.min(v), b.max(v)));
        assert!(shaped.0 < shaped.1, "the new tile has relief: {shaped:?}");
        assert!(wet > 0 || shaped.0 >= sea as f32, "its sea: {wet}");
    }

    /// Continents from tectonic plates with their climate: the plates to
    /// move in the node's settings, temperature and humidity as masks, and
    /// converted, the biomes of the game written with the ground painted
    /// to suit. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn continents_with_climate() {
        use massif_core::graph::{NodeKind, Template};
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        {
            let app = h.state_mut();
            app.graph.preview = 512;
            let mut graph = app.doc.world.meta.graph.clone().unwrap_or_default();
            graph.size = Some(2048);
            app.doc.world.meta.graph = Some(graph);
            app.graph.use_template(&mut app.doc, Template::Continents);
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("continents-graph.png")).unwrap();
        let graph = h.state().doc.world.meta.graph.clone().unwrap();
        assert_eq!(graph.size, Some(2048), "the template keeps the terrain's size");
        let find = |pick: &dyn Fn(&NodeKind) -> bool| graph.nodes.iter().find(|n| pick(&n.kind)).unwrap().id;
        for (name, id) in [
            ("continents-plates.png", find(&|k| matches!(k, NodeKind::Tectonic { .. }))),
            ("continents-temperature.png", find(&|k| matches!(k, NodeKind::Temperature { .. }))),
            ("continents-humidity.png", find(&|k| matches!(k, NodeKind::Humidity { .. }))),
        ] {
            h.state_mut().graph.view(Some(id));
            wait_for_graph(&mut h);
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
        h.state_mut().graph.view(None);
        wait_for_graph(&mut h);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        h.run_steps(2);
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 240, "the conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        let doc = &h.state().doc;
        assert_eq!(doc.dimension().ground().len(), 256, "a world of 2048 blocks");
        let mut biomes = std::collections::BTreeSet::new();
        for p in doc.dimension().ground().positions() {
            let tile = doc.dimension().ground().view(p).unwrap();
            for i in (0..128 * 128).step_by(97) {
                biomes.insert(tile.biome.get(i));
            }
        }
        let names: Vec<&str> = biomes.iter().map(|&b| doc.world.registry.biome(massif_core::registry::BiomeId(b)).id.as_str()).collect();
        eprintln!("biomes written: {names:?}");
        assert!(!biomes.contains(&0) && biomes.len() >= 5, "climates of every kind: {names:?}");
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.distance = 1900.0;
            app.view3d.camera.pitch = 0.75;
            app.view3d.camera.yaw = 0.5;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("continents-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// A click and the pointer let go at `p`.
    fn click_on(h: &mut egui_kittest::Harness<'_, MassifApp>, p: egui::Pos2, button: egui::PointerButton) {
        h.event(egui::Event::PointerMoved(p));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: p,
                button,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.run_steps(3);
    }

    /// The left button held from `from` to `to`.
    fn drag_on(h: &mut egui_kittest::Harness<'_, MassifApp>, from: egui::Pos2, to: egui::Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        for k in 1..=12 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 12.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(3);
    }

    /// Where a river may spring on the map in view (its highest dry land,
    /// near the middle), and a point far below it to end at.
    fn river_ends(app: &MassifApp) -> ((f32, f32), (f32, f32)) {
        let m = app.maps.values().next().unwrap();
        let (cx, cz) = (m.center[0] as f32, m.center[1] as f32);
        let reach = 220.0 / m.zoom;
        let mut best = ((cx, cz), f32::MIN);
        let mut low = ((cx, cz), f32::MAX);
        for j in -6..=6 {
            for i in -6..=6 {
                let p = (cx + i as f32 * reach / 6.0, cz + j as f32 * reach / 6.0);
                if let Some(probe) = crate::tools::probe(&app.doc, p.0 as i32, p.1 as i32) {
                    let sea = app.doc.world.meta.sea_level as f32;
                    let dry = probe.height > sea && (probe.fluid == massif_core::tile::NO_FLUID || probe.fluid as f32 <= probe.height);
                    if dry && probe.height > best.1 {
                        best = (p, probe.height);
                    }
                    if probe.height < low.1 {
                        low = (p, probe.height);
                    }
                }
            }
        }
        (best.0, low.0)
    }

    /// Auto paint, hovered over the map: the columns it would repaint
    /// shown in their new colour before anything is painted. Needs a GPU:
    /// runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn auto_paint_shows_what_it_changes() {
        use massif_core::generate::{GenParams, Landform, generate};
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 8,
                tiles_z: 8,
                landform: Landform::Mountains,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 13);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::AutoPaint);
            app.tools.brush.radius = 60.0;
        }
        h.run_steps(5);
        // Where the rules have the most to change, in view.
        let (spot, count) = {
            let app = h.state();
            let m = app.maps.values().next().unwrap();
            let reader = massif_core::auto::Reader::new(&app.tools.auto, &app.doc.world.registry, app.doc.world.meta.sea_level);
            let mut best = ((0.0f32, 0.0f32), 0usize);
            for j in -4..=4 {
                for i in -4..=4 {
                    let p = (m.center[0] as f32 + i as f32 * 60.0, m.center[1] as f32 + j as f32 * 60.0);
                    let (_, cols) = massif_core::ops::auto_preview(app.doc.dimension(), &app.tools.brush, p.0, p.1, &reader, false);
                    let n = cols.iter().filter(|c| c.becomes().is_some()).count();
                    if n > best.1 {
                        best = (p, n);
                    }
                }
            }
            best
        };
        assert!(count > 100, "something to repaint: {count}");
        let at = h.state().maps.values().next().unwrap().screen_of(spot.0 as f64, spot.1 as f64);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(5);
        h.render().unwrap().save(dir.join("auto-paint-preview.png")).unwrap();
        drag_on(&mut h, at, at + vec2(4.0, 0.0));
        let note = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        assert!(note.starts_with("Repainted"), "{note}");
    }

    /// The River tool: a click and the water finds its way down from the
    /// source; the course dragged bends it, a click elsewhere gives it an
    /// end, a right-click takes a point back, Enter digs it. No GPU needed.
    #[test]
    fn a_river_is_laid_out_and_dug() {
        use massif_core::generate::{GenParams, Landform, generate};
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 8,
                tiles_z: 8,
                landform: Landform::Hills,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 9);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::River);
            app.tools.river.width = 8.0;
        }
        h.run_steps(5);
        let at = |h: &egui_kittest::Harness<'_, MassifApp>, p: (f32, f32)| h.state().maps.values().next().unwrap().screen_of(p.0 as f64, p.1 as f64);
        let (source, end) = river_ends(h.state());
        let p = at(&h, source);
        click_on(&mut h, p, egui::PointerButton::Primary);
        let draft = h.state().tools.river_draft.clone().expect("a click places the source");
        assert!(draft.plan.course.len() > 10, "the water finds its way: {}", draft.plan.course.len());
        assert!(draft.layout.through.is_empty() && draft.layout.end.is_none());
        // The course held and dragged: a point it must pass.
        let mid = draft.plan.course[draft.plan.course.len() / 3];
        let from = at(&h, mid);
        drag_on(&mut h, from, from + vec2(30.0, 25.0));
        let draft = h.state().tools.river_draft.clone().unwrap();
        assert_eq!(draft.layout.through.len(), 1, "{:?}", draft.layout);
        let bend = draft.layout.through[0];
        assert!(draft.plan.course.iter().any(|p| (p.0 - bend.0).hypot(p.1 - bend.1) < 2.0), "through the point it was dragged to");
        // A click elsewhere: where it ends.
        let p = at(&h, end);
        click_on(&mut h, p, egui::PointerButton::Primary);
        let draft = h.state().tools.river_draft.clone().unwrap();
        assert!(draft.layout.end.is_some());
        let last = *draft.plan.course.last().unwrap();
        assert!((last.0 - end.0).hypot(last.1 - end.1) < 2.0, "it ends there: {last:?} {end:?}");
        // The point taken back with the right button.
        let p = at(&h, bend);
        click_on(&mut h, p, egui::PointerButton::Secondary);
        assert!(h.state().tools.river_draft.as_ref().unwrap().layout.through.is_empty());
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(5);
        assert_eq!(h.state().doc.history.undo_label(), Some("River"), "Enter digs it");
        assert!(h.state().tools.river_draft.is_none());
        let note = h.state().tools.river_note.clone().unwrap_or_default();
        assert!(note.starts_with("Dug a river"), "{note}");
    }

    /// The River tool on a real map (MASSIF_RIVER_WORLD): a course dragged
    /// from the mountains to the sea, dug as the user digs it, seen in 3D.
    /// Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn river_tool_on_a_real_world() {
        let (Some(dir), Some(path)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_RIVER_WORLD").map(PathBuf::from),
        ) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(10);
        let centre = [400.0, -165.0];
        {
            let app = h.state_mut();
            app.dialog = None;
            app.tools.select(Tool::River);
            app.tools.river.width = 8.0;
            app.tools.river.width_end = 26.0;
        }
        h.run_steps(5);
        for m in h.state_mut().maps.values_mut() {
            m.center = centre;
            m.zoom = 0.75;
        }
        h.run_steps(5);
        let at = |h: &egui_kittest::Harness<'_, MassifApp>, x: f64, z: f64| h.state().maps.values().next().unwrap().screen_of(x, z);
        let p = at(&h, 150.0, -560.0);
        click_on(&mut h, p, egui::PointerButton::Primary);
        let p = at(&h, 650.0, 230.0);
        click_on(&mut h, p, egui::PointerButton::Primary);
        h.render().unwrap().save(dir.join("real-river-tool-course.png")).unwrap();
        eprintln!("{}", h.state().tools.river_draft.as_ref().map(|d| d.summary()).unwrap_or_default());
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(20);
        let note = h.state().tools.river_note.clone().unwrap_or_default();
        eprintln!("{note}");
        assert!(note.starts_with("Dug a river"), "{note}");
        h.render().unwrap().save(dir.join("real-river-tool-map.png")).unwrap();
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.target = [400.0, 80.0, -165.0];
            app.view3d.camera.distance = 700.0;
            app.view3d.camera.pitch = 0.62;
            app.view3d.camera.yaw = 0.4;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("real-river-tool-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// A river from a source on a real map (MASSIF_RIVER_WORLD): hovering
    /// shows where its water would go, a click lets it spring there and dig
    /// its way down. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn spring_on_a_real_world() {
        let (Some(dir), Some(path)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_RIVER_WORLD").map(PathBuf::from),
        ) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(10);
        let centre = [300.0, -200.0];
        {
            let app = h.state_mut();
            app.dialog = None;
            app.tools.select(Tool::River);
            app.tools.river.width = 6.0;
            app.tools.river.flow = 0.6;
            app.tools.river.depth = 3.0;
            app.tools.river.erosion = 0.3;
        }
        h.run_steps(5);
        for m in h.state_mut().maps.values_mut() {
            m.center = centre;
            m.zoom = 0.45;
        }
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let at = rect.center() + vec2(((150.0 - centre[0]) * 0.45) as f32, ((-560.0 - centre[1]) * 0.45) as f32);
        h.event(egui::Event::PointerMoved(at));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        h.render().unwrap().save(dir.join("real-spring-preview.png")).unwrap();
        {
            let app = h.state();
            let started = std::time::Instant::now();
            let d = massif_core::river::descend(app.doc.dimension().ground(), (150, -560), 63.0, 60_000, 2_000_000, app.tools.river.lake_depth());
            eprintln!("descent in {:.2} s: {} points, mouth {:?}", started.elapsed().as_secs_f32(), d.course.len(), d.mouth);
            for (k, p) in d.course.iter().enumerate().step_by(400) {
                eprintln!("  {k}: {:.0}, {:.0}", p.0, p.1);
            }
            for (cells, level) in d.lakes.iter().take(40) {
                let (mut x0, mut z0, mut x1, mut z1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
                for &(x, z) in cells {
                    (x0, z0, x1, z1) = (x0.min(x), z0.min(z), x1.max(x), z1.max(z));
                }
                eprintln!("  lake {} cells at {level:.1}: {x0},{z0} .. {x1},{z1}", cells.len());
            }
        }
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.run_steps(10);
        h.render().unwrap().save(dir.join("real-spring-draft.png")).unwrap();
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(10);
        let note = h.state().tools.river_note.clone().unwrap_or_default();
        eprintln!("{note}");
        assert_eq!(h.state().doc.history.undo_label(), Some("River"), "{note}");
        assert!(note.starts_with("Dug a river"), "{note}");
        h.render().unwrap().save(dir.join("real-spring-map.png")).unwrap();
        // A second one, in the hills over the western plain.
        let centre = [-560.0, 420.0];
        for m in h.state_mut().maps.values_mut() {
            m.center = centre;
            m.zoom = 0.4;
        }
        h.run_steps(5);
        let at = rect.center() + vec2(((-760.0 - centre[0]) * 0.4) as f32, ((180.0 - centre[1]) * 0.4) as f32);
        h.event(egui::Event::PointerMoved(at));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        h.render().unwrap().save(dir.join("real-spring-preview-2.png")).unwrap();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.step();
        }
        h.run_steps(10);
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(10);
        eprintln!("{}", h.state().tools.river_note.clone().unwrap_or_default());
        h.render().unwrap().save(dir.join("real-spring-map-2.png")).unwrap();
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.target = [230.0, 110.0, -470.0];
            app.view3d.camera.distance = 520.0;
            app.view3d.camera.pitch = 0.7;
            app.view3d.camera.yaw = 2.6;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("real-spring-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Height maps, in and out: a world made from a 16-bit picture, its
    /// relief saved back, the picture read by a graph's Height map node,
    /// and a graph's terrain saved as a height map. No GPU needed.
    #[test]
    fn height_maps_in_and_out() {
        use massif_core::graph::{NodeKind, Template};
        let dir = std::env::temp_dir().join(format!("massif-heightmaps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // A cone: white in the middle, black at the edges, 256 × 256.
        let n = 256;
        let cone: Vec<f32> = (0..n * n)
            .map(|k| {
                let (x, z) = ((k % n) as f32 - 127.5, (k / n) as f32 - 127.5);
                (1.0 - (x * x + z * z).sqrt() / 128.0).max(0.0)
            })
            .collect();
        let png = dir.join("cone.png");
        massif_core::heightmap::write_png16(&png, n, n, &cone).unwrap();
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        // Dropped on the home screen, it is read there.
        h.state_mut().open_home(Page::NewMap);
        let ctx = h.ctx.clone();
        h.state_mut().home_action(&ctx, HomeAction::Dropped(png.clone()));
        h.run_steps(3);
        {
            let home = h.state_mut().home.as_mut().expect("the home screen");
            assert_eq!(home.page, Page::FromHeightmap);
            let pick = home.heightmap.as_mut().expect("the height map read");
            pick.how = massif_core::generate::FromHeightmap {
                name: "Cone".into(),
                scale: 2.0,
                low: 40.0,
                high: 200.0,
                sea_level: 63,
                min_y: massif_core::generate::VANILLA_LIMITS.0,
                max_y: massif_core::generate::VANILLA_LIMITS.1,
            };
        }
        h.run_steps(3);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("Create map").click();
        }
        h.run_steps(5);
        assert!(h.state().home.is_none(), "the map is shown");
        let doc = &h.state().doc;
        assert_eq!(doc.world.meta.name, "Cone");
        let (lo, hi) = doc.dimension().bounds().unwrap();
        assert_eq!((hi.x - lo.x + 1, hi.z - lo.z + 1), (4, 4), "512 blocks a side");
        let top = |doc: &crate::doc::Document, x: i32, z: i32| doc.dimension().ground().view(massif_core::TilePos::containing(x, z)).unwrap().top.get(massif_core::coords::local_index(x, z));
        assert!((top(doc, 0, 0) - 200.0).abs() < 3.0, "the peak: {}", top(doc, 0, 0));
        assert!((top(doc, -255, -255) - 40.0).abs() < 1.0, "the foot: {}", top(doc, -255, -255));
        // Its relief, saved back: the peak white, the corners black.
        let out = dir.join("out.png");
        let ctx = h.ctx.clone();
        h.state_mut().save_heightmap(&ctx, &out);
        let back = massif_core::heightmap::read(&out).unwrap();
        assert_eq!((back.width, back.height), (512, 512));
        let y = |v: f32| -64.0 + v * 384.0;
        assert!((y(back.sample(255.5, 255.5)) - 200.0).abs() < 3.0, "{}", y(back.sample(255.5, 255.5)));
        assert!((y(back.data[0]) - 40.0).abs() < 3.0);
        // A graph reading the picture, and saved as a height map.
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        {
            let app = h.state_mut();
            app.graph.use_template(&mut app.doc, Template::Alpine);
            let mut g = massif_core::graph::Graph::default();
            let pic = g.add(
                NodeKind::FileInput {
                    path: png.display().to_string(),
                    low: 60.0,
                    high: 160.0,
                    fit: true,
                },
                [0.0, 0.0],
            );
            let out = g.add(NodeKind::HeightOutput, [220.0, 0.0]);
            g.connect(pic, 0, out, 0);
            app.doc.world.meta.graph = Some(g);
            app.doc.mark_changed();
            app.graph.draft = Some(128);
            let graph = app.doc.world.meta.graph.clone().unwrap();
            app.graph.start_build(&ctx, &mut app.doc, &graph, Some(dir.join("graph.png")));
        }
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 60);
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        let saved = massif_core::heightmap::read(&dir.join("graph.png")).unwrap();
        assert_eq!((saved.width, saved.height), (128, 128));
        assert!((y(saved.sample(63.5, 63.5)) - 160.0).abs() < 3.0, "the cone's top, as the node maps it: {}", y(saved.sample(63.5, 63.5)));
        assert!((y(saved.data[0]) - 60.0).abs() < 3.0);
        // A world file as input: the cone world saved, read back by the
        // node, fitted to the terrain — its peak in the middle.
        let massif = dir.join("cone.massif");
        let mut cone_world = massif_core::generate::from_heightmap(
            &massif_core::heightmap::read(&png).unwrap(),
            &massif_core::generate::FromHeightmap {
                name: "Cone".into(),
                scale: 2.0,
                low: 40.0,
                high: 200.0,
                sea_level: 63,
                min_y: massif_core::generate::VANILLA_LIMITS.0,
                max_y: massif_core::generate::VANILLA_LIMITS.1,
            },
        );
        cone_world.meta.graph = None;
        massif_format::save(&cone_world, &massif).unwrap();
        let mut g = massif_core::graph::Graph::default();
        let input = g.add(
            NodeKind::FileInput {
                path: massif.display().to_string(),
                low: 0.0,
                high: 0.0,
                fit: true,
            },
            [0.0, 0.0],
        );
        let out = g.add(NodeKind::HeightOutput, [220.0, 0.0]);
        g.connect(input, 0, out, 0);
        let ext = massif_core::graph::Extent {
            x0: 5000.0,
            z0: 5000.0,
            size: 1000.0,
            n: 64,
        };
        let built = g
            .build(&ext, &std::sync::atomic::AtomicBool::new(false))
            .unwrap();
        let hgt = built.heights.unwrap();
        // A sample covers 8 blocks of the cone: within a few blocks of its tip.
        assert!(
            (hgt.at(32, 32) - 200.0).abs() < 7.0,
            "the world's peak, fitted to the terrain: {}",
            hgt.at(32, 32)
        );
        assert!(
            (hgt.at(1, 1) - 40.0).abs() < 4.0,
            "its foot: {}",
            hgt.at(1, 1)
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The graph canvas worked with the mouse and the keyboard: a box
    /// selects several nodes, dragging one moves them all, ⌘D duplicates
    /// them, Delete removes them, ⌘Z takes it all back step by step; the
    /// wheel zooms; copied nodes paste with their wires.
    #[test]
    fn graph_canvas_selects_moves_and_undoes() {
        use massif_core::graph::Template;
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        {
            let app = h.state_mut();
            app.graph.use_template(&mut app.doc, Template::Alpine);
        }
        h.run_steps(8);
        let graph = |h: &egui_kittest::Harness<'_, MassifApp>| h.state().doc.world.meta.graph.clone().unwrap();
        let count = graph(&h).nodes.len();
        let canvas = h.state().graph.canvas_rect();
        assert!(canvas.width() > 100.0, "the canvas is laid out: {canvas:?}");
        let drag = |h: &mut egui_kittest::Harness<'_, MassifApp>, from: egui::Pos2, to: egui::Pos2, modifiers: Modifiers| {
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers,
            });
            h.step();
            for k in 1..=8 {
                h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 8.0)));
                h.step();
            }
            h.event(egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers,
            });
            h.run_steps(3);
        };
        // A box around the first three nodes of the graph.
        let g = graph(&h);
        let rects: Vec<egui::Rect> = g.nodes.iter().take(3).map(|n| h.state().graph.screen_of(&g, n.id)).collect();
        let around = rects.iter().fold(rects[0], |a, r| a.union(*r)).expand(12.0);
        assert!(canvas.contains(around.min) && canvas.contains(around.max), "the nodes are in view");
        let free = g.nodes.iter().all(|n| !h.state().graph.screen_of(&g, n.id).contains(around.min));
        assert!(free, "the box starts on the background");
        drag(&mut h, around.min, around.max, Modifiers::NONE);
        let picked = h.state().graph.picked();
        assert!(picked.len() >= 3, "a box selects several nodes: {picked:?}");
        // Drag one: they all move.
        let before: Vec<[f32; 2]> = picked.iter().map(|id| graph(&h).node(*id).unwrap().pos).collect();
        let grab = h.state().graph.screen_of(&graph(&h), picked[0]).center_top() + egui::vec2(0.0, 12.0);
        drag(&mut h, grab, grab + egui::vec2(60.0, 40.0), Modifiers::NONE);
        let zoom = h.state().graph.zoom();
        for (id, was) in picked.iter().zip(&before) {
            let now = graph(&h).node(*id).unwrap().pos;
            assert!((now[0] - was[0] - 60.0 / zoom).abs() < 2.0 && (now[1] - was[1] - 40.0 / zoom).abs() < 2.0, "node {id} moved with the others: {was:?} → {now:?}");
        }
        assert_eq!(h.state().graph.picked().len(), picked.len(), "and they stay selected");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("graph-selection.png")).unwrap();
        }
        // Duplicate, then delete the copies.
        h.event(egui::Event::PointerMoved(canvas.center()));
        h.key_press_modifiers(Modifiers::COMMAND, Key::D);
        h.run_steps(3);
        assert_eq!(graph(&h).nodes.len(), count + picked.len(), "⌘D duplicates the selection");
        let copies = h.state().graph.picked();
        assert!(copies.iter().all(|id| !picked.contains(id)), "the copies are selected");
        h.key_press(Key::Delete);
        h.run_steps(3);
        assert_eq!(graph(&h).nodes.len(), count, "Delete removes the selection");
        // Undo, step by step: the copies come back, go, the move is undone.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        assert_eq!(graph(&h).nodes.len(), count + picked.len());
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        assert_eq!(graph(&h).nodes.len(), count);
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(3);
        for (id, was) in picked.iter().zip(&before) {
            assert_eq!(graph(&h).node(*id).unwrap().pos, *was, "the move is undone");
        }
        h.key_press_modifiers(Modifiers::COMMAND | Modifiers::SHIFT, Key::Z);
        h.run_steps(3);
        assert_ne!(graph(&h).node(picked[0]).unwrap().pos, before[0], "and redone");
        // Copy and paste: the nodes and the wires between them.
        let g = graph(&h);
        let rects: Vec<egui::Rect> = picked.iter().map(|id| h.state().graph.screen_of(&g, *id)).collect();
        let around = rects.iter().fold(rects[0], |a, r| a.union(*r)).expand(12.0);
        drag(&mut h, around.min, around.max, Modifiers::NONE);
        let picked = h.state().graph.picked();
        assert!(picked.len() >= 3);
        h.event(egui::Event::PointerMoved(canvas.center()));
        let links = graph(&h).links.len();
        let inner = graph(&h).links.iter().filter(|l| picked.contains(&l.from) && picked.contains(&l.to)).count();
        h.event(egui::Event::Copy);
        h.step();
        let text = h
            .output()
            .platform_output
            .commands
            .iter()
            .find_map(|c| match c {
                egui::OutputCommand::CopyText(t) => Some(t.clone()),
                _ => None,
            })
            .expect("the selection is copied");
        assert!(text.contains("massif_nodes"), "copied as text: {text:.40}");
        h.event(egui::Event::Paste(text));
        h.run_steps(3);
        assert_eq!(graph(&h).nodes.len(), count + picked.len(), "pasted");
        assert_eq!(graph(&h).links.len(), links + inner, "with their wires");
        // The wheel zooms around the pointer.
        let zoom = h.state().graph.zoom();
        h.event(egui::Event::PointerMoved(canvas.center()));
        h.event(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 3.0),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Wheel,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(3);
        assert!(h.state().graph.zoom() > zoom * 1.2, "the wheel zooms: {zoom} → {}", h.state().graph.zoom());
    }

    /// Rivers on a real map (MASSIF_RIVER_WORLD: a .massif or .world), as a
    /// user gets them: the template, its preview, the conversion and the
    /// map in 3D. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn rivers_on_a_real_world() {
        use massif_core::graph::Template;
        let (Some(dir), Some(path)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_RIVER_WORLD").map(PathBuf::from),
        ) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        let started = std::time::Instant::now();
        while !h.state().jobs.is_empty() {
            assert!(started.elapsed().as_secs() < 120, "opening timed out");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(10);
        h.state_mut().dialog = None;
        let sea = h.state().doc.world.meta.sea_level;
        let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
        eprintln!("map {}x{} tiles, sea {sea}", hi.x - lo.x + 1, hi.z - lo.z + 1);
        // The map from above with the rivers the template traces, at 2048.
        {
            use massif_core::graph::{Extent, GroundMap, NodeKind, Progress};
            let doc = &mut h.state_mut().doc;
            let g = Template::MapRivers.graph(&doc.world.registry, sea as f32, 4096.0);
            let rect = {
                let (lo, hi) = doc.dimension().bounds().unwrap();
                massif_core::BlockRect::new(lo.x * 128, lo.z * 128, (hi.x + 1) * 128, (hi.z + 1) * 128)
            };
            let ext = Extent::covering(rect, 2048);
            let colors: Vec<[u8; 3]> = doc.world.registry.terrains.iter().map(|t| t.color).collect();
            let snapshot = doc.world.dims[doc.dim].snapshot();
            let map = GroundMap::new(snapshot, 1, sea as f32 - 8.0, colors, g.own_levels());
            let started = std::time::Instant::now();
            let out = g.build_on(Some(&map), &ext, &std::sync::atomic::AtomicBool::new(false), &Progress::default()).unwrap();
            eprintln!("built at 2048 in {:.1} s", started.elapsed().as_secs_f32());
            let (hgt, water) = (out.heights.unwrap(), out.water.unwrap());
            let n = hgt.n;
            let mut rgb = Vec::with_capacity(n * n * 3);
            for j in 0..n {
                for i in 0..n {
                    let v = hgt.at(i, j);
                    let dx = hgt.at(i + 1, j) - hgt.at(i.saturating_sub(1), j);
                    let dz = hgt.at(i, j + 1) - hgt.at(i, j.saturating_sub(1));
                    let shade = (0.75 + 0.08 * (-dx + dz)).clamp(0.25, 1.2);
                    let c = if water.at(i, j) > v + 0.25 {
                        [40.0, 110.0, 230.0]
                    } else if v < sea as f32 {
                        [30.0, 50.0, 90.0]
                    } else {
                        let t = ((v - sea as f32) / 120.0).clamp(0.0, 1.0);
                        [110.0 + 90.0 * t, 150.0 + 40.0 * t, 80.0 + 110.0 * t].map(|c| c * shade)
                    };
                    rgb.extend(c.map(|c| c.clamp(0.0, 255.0) as u8));
                }
            }
            image::save_buffer(dir.join("real-rivers-top.png"), &rgb, n as u32, n as u32, image::ColorType::Rgb8).unwrap();
            // The map as it was, to compare.
            let read = massif_core::graph::sample_map(&doc.world.dims[doc.dim], &ext, sea as f32 - 8.0, &[], &g.own_levels());
            let (hgt, water) = (read.heights, read.water);
            let mut rgb = Vec::with_capacity(n * n * 3);
            for j in 0..n {
                for i in 0..n {
                    let v = hgt.at(i, j);
                    let dx = hgt.at(i + 1, j) - hgt.at(i.saturating_sub(1), j);
                    let dz = hgt.at(i, j + 1) - hgt.at(i, j.saturating_sub(1));
                    let shade = (0.75 + 0.08 * (-dx + dz)).clamp(0.25, 1.2);
                    let c = if water.at(i, j) > 0.5 && v >= sea as f32 - 0.5 {
                        [40.0, 110.0, 230.0]
                    } else if v < sea as f32 {
                        [30.0, 50.0, 90.0]
                    } else {
                        let t = ((v - sea as f32) / 120.0).clamp(0.0, 1.0);
                        [110.0 + 90.0 * t, 150.0 + 40.0 * t, 80.0 + 110.0 * t].map(|c| c * shade)
                    };
                    rgb.extend(c.map(|c| c.clamp(0.0, 255.0) as u8));
                }
            }
            image::save_buffer(dir.join("real-map-top.png"), &rgb, n as u32, n as u32, image::ColorType::Rgb8).unwrap();
            let _ = NodeKind::HeightOutput;
        }
        h.state_mut().layout.activate(7);
        h.run_steps(3);
        {
            let app = h.state_mut();
            app.graph.preview = 768;
            app.graph.use_template(&mut app.doc, Template::MapRivers);
        }
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("real-rivers-graph.png")).unwrap();
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        h.run_steps(2);
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 400, "the conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        h.run_steps(3);
        let dim = h.state().doc.dimension();
        let names: Vec<String> = dim.levels.iter().map(|l| l.name.clone()).collect();
        let wet = dim
            .levels
            .iter()
            .filter(|l| l.name.starts_with("Dug"))
            .flat_map(|l| l.positions().map(move |p| (l, p)))
            .map(|(l, p)| {
                let t = l.view(p).unwrap();
                (0..128 * 128).filter(|&i| t.present(i) && t.fluid.get(i) as i32 > sea).count()
            })
            .sum::<usize>();
        eprintln!("levels {names:?}, {wet} columns of river water above the sea");
        // The middle of the map in 3D, from above.
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.distance = 2600.0;
            app.view3d.camera.pitch = 0.95;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("real-rivers-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// The templates: the picker with their pictures, a card replacing the
    /// graph, and every template in the viewport with its own sky. Needs a
    /// GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn templates_render() {
        use massif_core::graph::{NodeKind, Template};
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        h.state_mut().graph.open_templates();
        let started = std::time::Instant::now();
        while h.state().graph.pictures_ready() < Template::ALL.len() {
            assert!(started.elapsed().as_secs() < 60, "the pictures were not made");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        h.render().unwrap().save(dir.join("graph-templates.png")).unwrap();
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("Archipelago").click();
        }
        h.run_steps(3);
        let graph = h.state().doc.world.meta.graph.clone().expect("a graph");
        assert!(graph.nodes.iter().any(|n| matches!(n.kind, NodeKind::Sea { .. })), "the archipelago replaced the graph");
        for t in Template::ALL {
            {
                let app = h.state_mut();
                app.graph.preview = 512;
                app.graph.use_template(&mut app.doc, t);
            }
            wait_for_graph(&mut h);
            let (w, hgt, rgba) = h.state().graph.view.read_back().expect("the viewport rendered");
            let name = t.label().to_lowercase().replace(' ', "-");
            image::save_buffer(dir.join(format!("template-{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        }
        // The floating mountains close up, as a saved image.
        {
            let app = h.state_mut();
            app.graph.preview = 1024;
            app.graph.use_template(&mut app.doc, Template::FloatingMountains);
        }
        wait_for_graph(&mut h);
        let view = &mut h.state_mut().graph.view;
        view.camera.distance = 0.62;
        view.camera.pitch = 0.3;
        view.camera.yaw = 2.2;
        let (w, hgt, rgba) = view.render_image([1600, 900]).expect("an image");
        image::save_buffer(dir.join("template-floating-mountains-close.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // And from among the pillars, looking at their cliffs.
        view.camera.distance = 0.3;
        view.camera.pitch = 0.24;
        view.camera.yaw = 0.9;
        view.render.clouds = false;
        let (w, hgt, rgba) = view.render_image([1600, 900]).expect("an image");
        image::save_buffer(dir.join("template-floating-mountains-cliffs.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        view.render.clouds = true;
        view.camera.distance = 0.62;
        view.camera.pitch = 0.3;
        view.camera.yaw = 2.2;
        // Converted to voxels: a Solid level over the ground, columns of
        // several spans, seen in the 3D view of the map.
        h.state_mut().graph.draft = None;
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        let started = std::time::Instant::now();
        while h.state().doc.history.undo_label() != Some("Terrain graph") {
            assert!(started.elapsed().as_secs() < 120, "the conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        let names: Vec<String> = h.state().doc.dimension().levels.iter().map(|l| l.name.clone()).collect();
        assert!(names.iter().any(|n| n == "Floating islands (graph 1)"), "{names:?}");
        {
            let app = h.state_mut();
            app.layout.activate(3);
            app.view3d.camera.distance = 1250.0;
            app.view3d.camera.pitch = 0.42;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("template-floating-mountains-voxels.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // Converting again reuses the levels.
        let levels = h.state().doc.dimension().levels.len();
        h.state_mut().layout.activate(7);
        h.run_steps(3);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Convert to voxels").click();
        }
        h.run_steps(2);
        assert!(h.state().graph.converting(), "the second conversion started");
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 120, "the second conversion did not finish");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(h.state().doc.dimension().levels.len(), levels, "the same levels");
    }

    /// The game's real blocks: loaded from this computer's installation,
    /// shown with their textures, painted on the map. Runs with
    /// MASSIF_SNAPSHOT_DIR, when a Minecraft jar is installed.
    #[test]
    fn real_blocks_show_and_paint() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        if massif_assets::find_jars().is_empty() {
            eprintln!("no Minecraft installation: skipping");
            return;
        }
        for (theme, name) in [(ThemeChoice::Dark, "blocks-dark.png"), (ThemeChoice::Light, "blocks-light.png")] {
            let mut h = harness(theme);
            let ctx = h.ctx.clone();
            h.state_mut().blocks.load(&ctx, None);
            h.state_mut().layout.active = 2; // Painting: terrain, blocks, biomes
            let started = std::time::Instant::now();
            while h.state().blocks.catalogue.is_none() {
                assert!(started.elapsed().as_secs() < 60, "the blocks did not load: {:?}", h.state().blocks.error);
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            h.run_steps(10);
            // Real colours: the grass terrain is green, from the game's texture.
            let app = h.state();
            let reg = &app.doc.world.registry;
            let grass = reg.terrain(reg.terrain_by_name("Grass").unwrap()).color;
            assert!(grass[1] > grass[0] && grass[1] > grass[2], "{grass:?}");
            // Paint stone bricks: a terrain made of the real block.
            let cat = app.blocks.catalogue.clone().unwrap();
            let bricks = cat.block("minecraft:stone_bricks").unwrap().clone();
            let app = h.state_mut();
            let terrain = massif_assets::block_terrain(&mut app.doc.world.registry, &bricks);
            app.tools.terrain = terrain.0;
            app.tools.select(Tool::PaintTerrain);
            let rect = map_rect(h.state(), &h.ctx);
            let at = rect.center();
            let button = |pressed| egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: Modifiers::NONE,
            };
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(button(true));
            for _ in 0..10 {
                h.step();
            }
            h.event(button(false));
            h.run_steps(20);
            assert_eq!(h.state().doc.history.undo_label(), Some("Paint terrain"));
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
    }

    /// Massif opens on the home screen: no map, the ways in, the settings of
    /// a new map. Making one shows it; the home screen comes back with ⌘N,
    /// with the way back to the map; a template's map goes to the Terrain
    /// side. With MASSIF_SNAPSHOT_DIR, the screens are saved.
    #[test]
    fn the_home_screen_comes_first() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.has_map = false;
            app.settings.recent.clear();
            app.open_home(Page::NewMap);
        }
        h.run_steps(5);
        h.get_by_label("No maps yet");
        assert!(h.query_by_label_contains("Back to").is_none(), "no map to go back to");
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("home-dark.png")).unwrap();
        }
        {
            let home = h.state_mut().home.as_mut().unwrap();
            home.new_map.params.name = "Test map".into();
            home.new_map.width = 512;
            home.new_map.depth = 512;
        }
        h.run_steps(2);
        h.get_by_label("Create map").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        assert!(h.state().home.is_none(), "the new map is shown");
        assert_eq!(h.state().doc.world.meta.name, "Test map");
        // ⌘N: the home screen again, with the way back.
        h.event(egui::Event::Key {
            key: egui::Key::N,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        h.run_steps(3);
        assert!(h.state().home.is_some());
        h.get_by_label("Back to Test map").click();
        h.run_steps(3);
        assert!(h.state().home.is_none(), "back to the map");
        // A map whose relief comes from a template, made on the Terrain side.
        h.state_mut().open_home(Page::NewMap);
        {
            let home = h.state_mut().home.as_mut().unwrap();
            home.new_map.template = Some(massif_core::graph::Template::Archipelago);
            home.new_map.width = 512;
            home.new_map.depth = 512;
        }
        h.state_mut().settings.recent = vec![PathBuf::from("/nowhere/Alpine test.massif"), PathBuf::from("/nowhere/Old island.world")];
        for _ in 0..40 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(15));
        }
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("home-template.png")).unwrap();
        }
        h.get_by_label("Create map").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        assert!(h.state().home.is_none());
        assert!(h.state().layout.terrain(), "on the Terrain side");
        assert!(h.state().doc.world.meta.graph.is_some(), "with the template's graph");
    }

    /// A map of lava: its preview, then the map itself, opaque and bright
    /// with scorched shores, not water tinted orange. Needs a GPU: runs
    /// with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn lava_maps_look_like_lava() {
        use egui_kittest::kittest::Queryable;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().open_home(Page::NewMap);
        {
            let home = h.state_mut().home.as_mut().unwrap();
            home.new_map.params.lava = true;
            home.new_map.params.sea_level = 96;
            // The ground under the lava's level, so that it has lakes and
            // shores whatever the seed.
            home.new_map.params.level = Some(88.0);
            home.new_map.seed = "7".into();
            home.new_map.params.landform = Landform::Mountains;
            home.new_map.width = 1024;
            home.new_map.depth = 1024;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("home-lava.png")).unwrap();
        h.get_by_label("Create map").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        h.render().unwrap().save(dir.join("lava-map.png")).unwrap();
        // By name: the game's own blocks, once read, add terrains of the
        // same names.
        let reg = &h.state().doc.world.registry;
        let (mut scorched, mut sandy) = (0, 0);
        for z in (-500..500).step_by(4) {
            for x in (-500..500).step_by(4) {
                if let Some(p) = crate::tools::probe(&h.state().doc, x, z) {
                    let name = reg.terrain(massif_core::TerrainId(p.terrain)).name.as_str();
                    scorched += matches!(name, "Blackstone" | "Basalt") as usize;
                    sandy += (name == "Sand") as usize;
                }
            }
        }
        assert!(scorched > 100 && sandy == 0, "scorched shores, no sand: {scorched} / {sandy}");
    }

    /// A map taller than the game's own: made from the home screen up to
    /// 1024, its mountains past 320, shown in 2D and 3D. Needs a GPU: runs
    /// with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn tall_maps_are_made_and_shown() {
        use egui_kittest::kittest::Queryable;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().open_home(Page::NewMap);
        h.run_steps(3);
        h.get_by_label("1024").click();
        {
            let home = h.state_mut().home.as_mut().unwrap();
            home.new_map.params.landform = Landform::Mountains;
            home.new_map.params.relief = 700.0;
            home.new_map.width = 1024;
            home.new_map.depth = 1024;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("home-tall.png")).unwrap();
        h.get_by_label("Create map").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(5);
        let dim = h.state().doc.dimension();
        assert_eq!((dim.min_y, dim.max_y), (-64, 1024));
        let tallest = (-500..500).step_by(8).flat_map(|z| (-500..500).step_by(8).map(move |x| (x, z))).filter_map(|(x, z)| crate::tools::probe(&h.state().doc, x, z)).map(|p| p.height).fold(0.0f32, f32::max);
        assert!(tallest > 400.0, "mountains past 320: {tallest}");
        h.state_mut().maps.values_mut().for_each(|m| m.mode = crate::map_render::MapMode::Height);
        h.state_mut().map_cache.invalidate_all();
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        h.render().unwrap().save(dir.join("tall-map.png")).unwrap();
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("tall-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// The new-world and export dialogs, rendered.
    #[test]
    fn dialogs_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(5);
        // The home screen, with the settings of a new map.
        h.state_mut().open_home(Page::NewMap);
        if let Some(home) = &mut h.state_mut().home {
            home.new_map.params.landform = Landform::Mountains;
            home.new_map.width = 4096;
            home.new_map.depth = 4096;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("home-light.png")).unwrap();
        h.state_mut().home = None;
        // The settings: theme and accent colour.
        h.state_mut().dialog = Some(Dialog::Preferences);
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-settings.png")).unwrap();
        h.state_mut().dialog = None;
        h.state_mut().open_export_dialog();
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-export.png")).unwrap();
        // Once laid out, the window keeps its size: it used to grow taller
        // every frame.
        let height = |h: &egui_kittest::Harness<'static, MassifApp>| h.ctx.memory(|m| m.area_rect(Id::unique("export"))).map(|r| r.height());
        h.run_steps(10);
        let first = height(&h).expect("the export window is shown");
        h.run_steps(60);
        let later = height(&h).expect("the export window is shown");
        assert!(later < 1000.0 && (later - first).abs() < 1.0, "the export window went from {first} to {later} points high");
        if let Some(Dialog::Export(form)) = &mut h.state_mut().dialog {
            form.tab = ExportTab::World;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-export-world.png")).unwrap();
    }

    /// Dragging the corner of an area into it splits it there.
    #[test]
    fn dragging_a_corner_splits_the_area() {
        if std::env::var_os("MASSIF_SNAPSHOT_DIR").is_none() {
            return;
        }
        let mut h = harness(ThemeChoice::Light);
        h.state_mut().layout.active = 6; // Minimal: tools | map
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let before = h.state().layout.current().root.count();
        let corner = pos2(rect.right() - 5.0, rect.top() + 5.0);
        let to = pos2(rect.left() + rect.width() * 0.6, rect.top() + 30.0);
        let button = |pressed, pos| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.event(egui::Event::PointerMoved(corner));
        h.step();
        h.event(button(true, corner));
        h.step();
        for i in 1..=10 {
            h.event(egui::Event::PointerMoved(corner + (to - corner) * (i as f32 / 10.0)));
            h.step();
        }
        h.event(button(false, to));
        h.run_steps(5);
        assert_eq!(h.state().layout.current().root.count(), before + 1);
        let new = (h.state().layout.current().root.max_id_pub()).unwrap();
        assert_eq!(h.state().layout.current().root.editor_of(new), Some(EditorKind::Empty));
    }

    #[test]
    fn seeds_parse_like_minecraft() {
        assert_eq!(parse_seed(" 27594263 "), 27594263);
        assert_eq!(parse_seed("-5"), -5);
        assert_eq!(parse_seed("hello"), 99162322);
        assert_eq!(parse_seed(""), 0);
    }

    #[test]
    fn zones_and_paths() {
        let r = Rect::from_min_size(pos2(0.0, 0.0), vec2(100.0, 100.0));
        assert_eq!(zone_of(r, pos2(50.0, 50.0)), Zone::Center);
        assert_eq!(zone_of(r, pos2(5.0, 50.0)), Zone::Left);
        assert_eq!(zone_of(r, pos2(95.0, 40.0)), Zone::Right);
        assert_eq!(zone_of(r, pos2(50.0, 2.0)), Zone::Top);
        assert_eq!(zone_of(r, pos2(60.0, 99.0)), Zone::Bottom);

        let layout = Layout::default();
        let mut areas = Vec::new();
        let mut seps = Vec::new();
        let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 900.0));
        compute(&layout.workspaces[0].root, rect, &mut Vec::new(), &mut areas, &mut seps);
        assert_eq!(areas.len(), 9);
        assert_eq!(seps.len(), 8);
        // Areas tile the rectangle without overlapping.
        for (i, a) in areas.iter().enumerate() {
            for b in &areas[i + 1..] {
                assert!(!a.2.shrink(0.5).intersects(b.2.shrink(0.5)), "{a:?} overlaps {b:?}");
            }
        }
        let total: f32 = areas.iter().map(|a| a.2.area()).sum();
        assert!(total > rect.area() * 0.97 && total <= rect.area());
        let mut root = layout.workspaces[0].root.clone();
        if let Node::Split { ratio, .. } = node_at(&mut root, &seps[1].path) {
            *ratio = 0.25;
        }
        assert_ne!(root, layout.workspaces[0].root);
    }
}
