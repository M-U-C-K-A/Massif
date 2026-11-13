//! The application: window chrome, workspaces and areas, dialogs, jobs.

use crate::blocks::BlockLibrary;
use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
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
use massif_core::{TILE_SIZE, TilePos, World};
use massif_core::generate::{GenParams, Landform, generate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const HEADER: f32 = 36.0;
const GAP: f32 = 6.0;
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
    /// Show the welcome screen at startup.
    welcome: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            recent: Vec::new(),
            solo: false,
            light: crate::map_render::DEFAULT_LIGHT,
            minecraft_jar: None,
            welcome: true,
        }
    }
}

/// What a button of the welcome screen does, once it has closed.
type WelcomeAction = Box<dyn FnOnce(&mut MassifApp, &egui::Context)>;

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

/// A height map to make a world of, and how.
struct HeightmapForm {
    path: PathBuf,
    map: std::sync::Arc<massif_core::heightmap::Heightmap>,
    how: massif_core::generate::FromHeightmap,
    picture: Option<egui::TextureHandle>,
}

struct NewWorldForm {
    params: GenParams,
    seed: String,
    width: i32,
    depth: i32,
    /// The terrains a new world starts with (for the surface choice and the
    /// preview).
    registry: massif_core::Registry,
    /// The preview, and the settings it shows.
    preview: Option<(String, egui::TextureHandle)>,
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
    NewWorld(NewWorldForm),
    Export(ExportForm),
    Unsaved(Pending),
    About,
    /// Where to start: what Massif does, and the ways in.
    Welcome,
    /// A new world from a height map.
    FromHeightmap(HeightmapForm),
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
    next_generation: u64,
    layout: Layout,
    tools: ToolState,
    settings: Settings,
    maps: HashMap<AreaId, MapView>,
    map_cache: MapCache,
    view3d: View3d,
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
        let tools: ToolState = cc
            .storage
            .and_then(|s| eframe::get_value(s, "massif.tools"))
            .unwrap_or_default();
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
        } else if app.settings.welcome {
            app.dialog = Some(Dialog::Welcome);
        }
        app
    }

    /// Pick a height map, and show how it would become a world.
    fn pick_heightmap(&mut self, ctx: &egui::Context) {
        let Some(path) = rfd::FileDialog::new()
            .set_title("A height map to make a world of")
            .add_filter("Height maps", &["png", "jpg", "jpeg", "r16", "r32", "raw"])
            .pick_file()
        else {
            return;
        };
        match massif_core::heightmap::read(&path) {
            Ok(map) => {
                // A small picture of it.
                let side = 256usize;
                let (w, h) = (side, (side * map.height / map.width.max(1)).clamp(16, 512));
                let pixels: Vec<u8> = (0..w * h)
                    .map(|k| (map.sample((k % w) as f32 * map.width as f32 / w as f32, (k / w) as f32 * map.height as f32 / h as f32) * 255.0) as u8)
                    .collect();
                let picture = ctx.load_texture("height map", egui::ColorImage::from_gray([w, h], &pixels), egui::TextureOptions::LINEAR);
                let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Height map").to_owned();
                self.dialog = Some(Dialog::FromHeightmap(HeightmapForm {
                    path,
                    map: std::sync::Arc::new(map),
                    how: massif_core::generate::FromHeightmap {
                        name,
                        scale: 1.0,
                        low: 20.0,
                        high: 230.0,
                        sea_level: 63,
                    },
                    picture: Some(picture),
                }));
            }
            Err(e) => self.say(ctx, format!("That height map cannot be read: {e}"), true, None),
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
        Self {
            doc: Document::new(world, None, 1),
            next_generation: 1,
            layout,
            tools,
            settings,
            maps: HashMap::new(),
            map_cache: MapCache::default(),
            view3d: View3d::default(),
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

    fn open_new_dialog(&mut self) {
        let params = GenParams {
            name: "New World".into(),
            seed: random_seed(),
            ..GenParams::default()
        };
        self.dialog = Some(Dialog::NewWorld(NewWorldForm {
            seed: params.seed.to_string(),
            width: params.tiles_x * 128,
            depth: params.tiles_z * 128,
            params,
            registry: {
                let mut reg = massif_core::Registry::vanilla();
                if let Some(cat) = &self.blocks.catalogue {
                    massif_assets::apply_colors(&mut reg, cat);
                }
                reg
            },
            preview: None,
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
                    match path {
                        Some(p) => {
                            self.remember(p);
                            self.say(ctx, format!("Opened “{name}” in {secs:.1} s"), false, None);
                        }
                        None => self.say(ctx, format!("Created “{name}” in {secs:.1} s"), false, None),
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
            self.open_new_dialog();
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
                    self.open_new_dialog();
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
                    self.pick_heightmap(&ctx);
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
                if ui.button("Reset workspaces").clicked() {
                    self.layout = Layout::default();
                }
            });
            ui.menu_button("Help", |ui| {
                if ui.button(theme::with_icon(Icon::House, "Welcome: where to start")).clicked() {
                    self.dialog = Some(Dialog::Welcome);
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
                self.open_new_dialog();
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
                    RichText::new("Ctrl+Space maximises the area under the mouse · drag an area's title to move it")
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
        painter.rect(rect, 12, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        painter.hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0, pal.border));
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
            EditorKind::View3d => self.view3d.ui(&mut cui, frame, &mut self.doc, &self.tools, self.map_center),
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
                }
            }
            EditorKind::Info => {
                let stats = self.map_cache.stats();
                let mut affects_map = false;
                scrolled(&mut cui, &mut |ui| affects_map = panels::info_panel(ui, &mut self.doc, self.hover, stats));
                if affects_map {
                    self.map_cache.invalidate_all();
                    self.view3d.invalidate_all();
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
            Dialog::FromHeightmap(mut form) => {
                let mut make = false;
                let response = egui::Modal::new(Id::unique("from height map")).show(ctx, |ui| {
                    ui.set_width(560.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new("A new world from a height map").bold().size(16.0));
                    ui.label(RichText::new(form.path.display().to_string()).size(11.0).color(pal.weak));
                    ui.add_space(8.0);
                    ui.horizontal_top(|ui| {
                        if let Some(t) = &form.picture {
                            let size = t.size_vec2();
                            let k = (200.0 / size.x).min(200.0 / size.y);
                            ui.image((t.id(), size * k));
                        }
                        ui.vertical(|ui| {
                            let h = &mut form.how;
                            ui.horizontal(|ui| {
                                ui.label("Name");
                                ui.text_edit_singleline(&mut h.name);
                            });
                            ui.add(egui::Slider::new(&mut h.scale, 0.25..=16.0).logarithmic(true).text("blocks a pixel").max_decimals(2));
                            ui.add(egui::Slider::new(&mut h.low, -64.0..=319.0).text("black is Y").max_decimals(0));
                            ui.add(egui::Slider::new(&mut h.high, -64.0..=319.0).text("white is Y").max_decimals(0));
                            ui.add(egui::Slider::new(&mut h.sea_level, -64..=319).text("sea level"));
                            let (w, d) = (form.map.width as f32 * h.scale, form.map.height as f32 * h.scale);
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!(
                                    "{} × {} pixels → a map of {w:.0} × {d:.0} blocks, from Y {:.0} to {:.0}; terrains painted from height and slope.",
                                    form.map.width, form.map.height, h.low, h.high
                                ))
                                .size(11.5)
                                .color(pal.weak),
                            );
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_button(ui, "Make the world")).clicked() {
                            make = true;
                        }
                        if ui.button("Cancel").clicked() {
                            keep = false;
                        }
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if make {
                    self.request(ctx, Pending::FromHeightmap(form.map.clone(), form.how.clone()));
                } else if keep {
                    self.dialog = Some(Dialog::FromHeightmap(form));
                }
            }
            Dialog::Welcome => {
                use massif_core::graph::Template;
                let mut action: Option<WelcomeAction> = None;
                let response = egui::Modal::new(Id::unique("welcome")).show(ctx, |ui| {
                    ui.set_width(860.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new(format!("{}  Welcome to Massif", theme::glyph(Icon::MountainSnow))).bold().size(20.0));
                    ui.label(
                        RichText::new("Minecraft maps of any size: paint them by hand, or generate them from nodes, then export them to the game.")
                            .size(13.0)
                            .color(pal.weak),
                    );
                    ui.add_space(10.0);
                    // The way it goes, in three steps.
                    ui.horizontal(|ui| {
                        for (k, (title, what)) in [
                            ("Make the land", "paint it on the World side, or generate it on the Terrain side"),
                            ("Shape it", "rivers, paint, biomes, caves and overhangs"),
                            ("Export it", "File → Export (⌘E), for the version you play"),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            egui::Frame::new().fill(pal.header).corner_radius(10).inner_margin(egui::Margin::same(10)).show(ui, |ui| {
                                ui.set_width(256.0);
                                ui.label(RichText::new(format!("{}  {title}", k + 1)).bold().size(13.5));
                                ui.label(RichText::new(what).size(11.5).color(pal.weak));
                            });
                        }
                    });
                    ui.add_space(12.0);
                    ui.columns(3, |cols| {
                        let card = |ui: &mut Ui, icon: Icon, title: &str, what: &str| {
                            ui.label(RichText::new(format!("{}  {title}", theme::glyph(icon))).bold().size(14.5));
                            ui.label(RichText::new(what).size(11.5).color(pal.weak));
                            ui.add_space(6.0);
                        };
                        let wide = |ui: &mut Ui, icon: Icon, text: &str| ui.add_sized([ui.available_width(), 30.0], egui::Button::new(theme::with_icon(icon, text)));
                        // A map of one's own.
                        card(&mut cols[0], Icon::Map, "A map", "Start a map to paint, or open one");
                        if wide(&mut cols[0], Icon::FilePlus2, "New map…").clicked() {
                            action = Some(Box::new(|app, _| app.open_new_dialog()));
                        }
                        if wide(&mut cols[0], Icon::FolderOpen, "Open a .massif world…").clicked() {
                            action = Some(Box::new(|app, ctx| app.request(ctx, Pending::Open(None))));
                        }
                        if wide(&mut cols[0], Icon::ImageUp, "From a height map (Gaea, World Machine…)").clicked() {
                            action = Some(Box::new(|app, ctx| app.pick_heightmap(ctx)));
                        }
                        if wide(&mut cols[0], Icon::Import, "Import a WorldPainter world…").clicked() {
                            action = Some(Box::new(|app, ctx| {
                                if let Some(path) = rfd::FileDialog::new().set_title("Import a WorldPainter world").add_filter("WorldPainter world", &["world"]).pick_file() {
                                    app.request(ctx, Pending::Open(Some(path)));
                                }
                            }));
                        }
                        let recent: Vec<PathBuf> = self.settings.recent.iter().take(4).cloned().collect();
                        if !recent.is_empty() {
                            cols[0].add_space(6.0);
                            cols[0].label(RichText::new("Recent").size(11.0).color(pal.weak));
                            for path in recent {
                                if cols[0].link(jobs::display_name(&path)).on_hover_text(path.display().to_string()).clicked() {
                                    action = Some(Box::new(move |app, ctx| app.request(ctx, Pending::Open(Some(path)))));
                                }
                            }
                        }
                        // A landscape from nodes.
                        card(&mut cols[1], Icon::Mountain, "A landscape", "Generate one on the Terrain side, then Convert to voxels");
                        for t in [Template::Continents, Template::Alpine, Template::Archipelago, Template::RiverValley, Template::FloatingMountains, Template::DesertCanyon] {
                            if wide(&mut cols[1], Icon::LayoutTemplate, t.label()).on_hover_text(t.description()).clicked() {
                                action = Some(Box::new(move |app, _| app.start_terrain(t)));
                            }
                        }
                        // The map one has.
                        card(&mut cols[2], Icon::Brush, "The map you have", "Work on the map that is open");
                        if wide(&mut cols[2], Icon::Paintbrush, "Paint it from its relief").on_hover_text(Template::PaintMap.description()).clicked() {
                            action = Some(Box::new(|app, _| app.start_terrain(Template::PaintMap)));
                        }
                        if wide(&mut cols[2], Icon::Waves, "Give it rivers").on_hover_text(Template::MapRivers.description()).clicked() {
                            action = Some(Box::new(|app, _| app.start_terrain(Template::MapRivers)));
                        }
                        if wide(&mut cols[2], Icon::Route, "Draw a river by hand").on_hover_text("The River tool: drag along a river's course, let go").clicked() {
                            action = Some(Box::new(|app, _| {
                                app.layout.switch_side(false);
                                app.tools.select(Tool::River);
                            }));
                        }
                        if wide(&mut cols[2], Icon::Paintbrush2, "Paint by hand").on_hover_text("The brushes of the World side, as in WorldPainter").clicked() {
                            action = Some(Box::new(|app, _| {
                                app.layout.switch_side(false);
                                app.tools.select(Tool::PaintTerrain);
                            }));
                        }
                    });
                    ui.add_space(12.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.checkbox(&mut self.settings.welcome, "Show this at startup");
                        ui.label(RichText::new("· Help → Welcome brings it back").size(11.0).color(pal.weak));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Close").clicked() {
                                keep = false;
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if let Some(act) = action {
                    keep = false;
                    act(self, ctx);
                }
                if keep {
                    self.dialog = Some(Dialog::Welcome);
                }
            }
            Dialog::AutoRules(mut paint) => {
                let response = egui::Modal::new(Id::unique("auto rules")).show(ctx, |ui| {
                    ui.set_width(700.0);
                    ui.label(RichText::new("Automatic biomes and paint").bold().size(15.0));
                    ui.label(
                        RichText::new("Auto biome reads the land and gives it biomes; Auto paint gives each biome its ground. Both follow these rules, with a brush or over the whole map.")
                            .size(11.5)
                            .color(Palette::of(ui).weak),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut paint, false, theme::with_icon(Icon::Earth, "Biomes from the land"));
                        ui.selectable_value(&mut paint, true, theme::with_icon(Icon::WandSparkles, "Paint from the biomes"));
                    });
                    ui.separator();
                    egui::ScrollArea::vertical().max_height(440.0).auto_shrink([false, true]).show(ui, |ui| {
                        if paint {
                            panels::paint_rules(ui, &mut self.tools.auto.paint, &self.doc.world.registry);
                        } else {
                            panels::land_rules(ui, &mut self.tools.auto.land, &self.doc.world.registry);
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
                            let label = if paint { "Paint the whole map" } else { "Give the whole map its biomes" };
                            if ui.button(label).on_hover_text("One undoable step").clicked() {
                                let changed = if paint { self.doc.auto_paint(&self.tools.auto) } else { self.doc.auto_biome(&self.tools.auto) };
                                self.tools.auto_done = Some((!paint, changed));
                                self.feedback = Some(format!("{} over the whole map: {changed} columns changed", if paint { "Auto paint" } else { "Auto biome" }));
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
            Dialog::NewWorld(mut form) => {
                let mut create = false;
                let response = egui::Modal::new(Id::unique("new world")).show(ctx, |ui| {
                    ui.set_width(920.0);
                    ui.label(RichText::new("Create New World").bold().size(16.0));
                    ui.add_space(6.0);
                    let max_height = (ctx.content_rect().height() - 200.0).max(240.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(560.0);
                            egui::ScrollArea::vertical().id_salt("new world form").max_height(max_height).show(ui, |ui| {
                                ui.set_width(550.0);
                                new_world_form(ui, &mut form);
                            });
                        });
                        ui.add_space(12.0);
                        ui.vertical(|ui| new_world_preview(ui, &mut form));
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                            if ui.add(theme::primary_button(ui, "Create")).clicked() {
                                create = true;
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if create {
                    let mut params = form.params.clone();
                    params.seed = parse_seed(&form.seed);
                    params.tiles_x = (form.width + 127) / 128;
                    params.tiles_z = (form.depth + 127) / 128;
                    if params.name.trim().is_empty() {
                        params.name = "New World".into();
                    }
                    self.request(ctx, Pending::New(params));
                } else if keep {
                    self.dialog = Some(Dialog::NewWorld(form));
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
                                egui::ScrollArea::vertical().id_salt("export form").max_height(max_height).show(ui, |ui| {
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
                        ui.painter().vline(x, form_rect.y_range(), Stroke::new(1.0, pal.border));
                        ui.add_space(7.0);
                        ui.vertical(|ui| {
                            ui.set_min_width(110.0);
                            for (tab, label) in [(ExportTab::General, "General"), (ExportTab::World, "World")] {
                                if ui.add_sized([110.0, 24.0], egui::Button::selectable(form.tab == tab, label)).clicked() {
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
                        ui.checkbox(&mut form.overwrite, "Replace its region files (everything else is kept)");
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let test = ui
                            .button("Test Export…")
                            .on_hover_text("Export only the 3 × 3 tiles (384 × 384 blocks) around the centre of the map view, to a separate folder, to check the result quickly in Minecraft");
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
                    ui.label(RichText::new(format!("{}  Massif", theme::glyph(Icon::MountainSnow))).bold().size(18.0));
                    ui.label(RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(Palette::of(ui).weak));
                    ui.add_space(6.0);
                    ui.label("A map editor for Minecraft worlds of any size: sculpt and paint in 2D on a multi-level surface (ground, overhangs, floating islands, caves), preview in 3D, export to Anvil.");
                    ui.add_space(6.0);
                    ui.label(RichText::new("Worlds are saved as .massif files: an indexed container of independently compressed, checksummed tiles.").size(11.5).color(Palette::of(ui).weak));
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
        }
        self.handle_close(&ctx);
        self.shortcuts(&ctx);
        self.map_cache.sync_generation(self.doc.generation);
        if let Some(msg) = self.feedback.take() {
            self.say(&ctx, msg, false, None);
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
    card(ui, "Where to save it", "The folder the world is saved in, and the name it has in Minecraft.", |ui| {
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
            egui::ComboBox::from_id_salt("export version")
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
            let note = match form.target.era() {
                massif_anvil::Era::Modern => "today's chunks, −64 to 320",
                massif_anvil::Era::Flat16 | massif_anvil::Era::Flat15 | massif_anvil::Era::Flat14 => "named blocks, 0 to 255",
                massif_anvil::Era::Numbered => "numbered blocks, 0 to 255",
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
                egui::ComboBox::from_id_salt("game mode")
                    .selected_text(format!("{:?}", form.game_mode))
                    .show_ui(ui, |ui| {
                        for m in [GameMode::Survival, GameMode::Creative, GameMode::Adventure, GameMode::Spectator] {
                            ui.selectable_value(&mut form.game_mode, m, format!("{m:?}"));
                        }
                    });
            });
            row(ui, "Difficulty", |ui| {
                use massif_anvil::Difficulty;
                egui::ComboBox::from_id_salt("difficulty")
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
    card(ui, "Data packs", "Folders or zip files copied into the world's datapacks folder and enabled. Massif does not check that they suit the Minecraft version.", |ui| {
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
            ui.add(egui::DragValue::new(&mut form.spawn[1]).range(-64..=319));
            ui.add(egui::DragValue::new(&mut form.spawn[2]));
        });
        ui.label(RichText::new("The world's spawn point, set in the Info panel; changed here for this export only.").size(11.5).color(pal.weak));
    });
}

fn new_world_form(ui: &mut Ui, form: &mut NewWorldForm) {
    let pal = Palette::of(ui);
    egui::ScrollArea::vertical().max_height(560.0).auto_shrink([false, true]).show(ui, |ui| {
        ui.columns(2, |cols| {
            let (left, right) = cols.split_at_mut(1);
            let (ui, ui2) = (&mut left[0], &mut right[0]);
            card(ui, "Map", "The name of the map, and the seed Minecraft uses around it.", |ui| {
                row(ui, "Name", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.params.name).desired_width(f32::INFINITY));
                });
                row(ui, "Seed", |ui| {
                    ui.add(egui::TextEdit::singleline(&mut form.seed).desired_width(110.0));
                    if ui.button(theme::glyph(Icon::Dice5).to_string()).on_hover_text("Random seed").clicked() {
                        form.seed = random_seed().to_string();
                    }
                });
                row(ui, "Map format", |ui| {
                    ui.label("Minecraft Java 1.18 and later");
                });
            });
            card(ui2, "Size and shape", "How large the map is, and how high and deep it can go.", |ui| {
                row(ui, "Dimensions", |ui| {
                    ui.add(egui::DragValue::new(&mut form.width).range(128..=65536).speed(128.0));
                    ui.label("×");
                    ui.add(egui::DragValue::new(&mut form.depth).range(128..=65536).speed(128.0));
                });
                ui.horizontal_wrapped(|ui| {
                    ui.add_space(100.0);
                    for (blocks, label) in [(512, "512"), (1024, "1k"), (2048, "2k"), (4096, "4k"), (8192, "8k"), (16384, "16k")] {
                        let selected = form.width == blocks && form.depth == blocks;
                        if ui.selectable_label(selected, label).clicked() {
                            form.width = blocks;
                            form.depth = blocks;
                        }
                    }
                });
                // Whole tiles only.
                form.width = (form.width + 127) / 128 * 128;
                form.depth = (form.depth + 127) / 128 * 128;
                row(ui, "", |ui| {
                    ui.label(RichText::new("blocks, in multiples of 128").size(11.5).color(pal.weak));
                });
                row(ui, "Build limits", |ui| {
                    ui.label("-64 to 320");
                });
                // About 4.3 blocks per second on foot.
                let minutes = form.width.max(form.depth) as f32 / 4.317 / 60.0;
                row(ui, "Walking time", |ui| {
                    ui.label(if minutes < 1.0 { "under a minute".to_owned() } else { format!("{minutes:.0} min. across") });
                });
                let tiles = (form.width / 128) as i64 * (form.depth / 128) as i64;
                row(ui, "On disk", |ui| {
                    ui.label(RichText::new(format!("{tiles} tiles · roughly {}", panels::human((tiles * 12_000) as u64))).color(pal.weak));
                });
            });
        });
        ui.columns(2, |cols| {
            let (left, right) = cols.split_at_mut(1);
            let (ui, ui2) = (&mut left[0], &mut right[0]);
            card(ui, "Land", "The shape the map starts with, before you paint it.", |ui| {
                row(ui, "Topography", |ui| {
                    egui::ComboBox::from_id_salt("landform")
                        .selected_text(form.params.landform.label())
                        .show_ui(ui, |ui| {
                            for l in Landform::ALL {
                                ui.selectable_value(&mut form.params.landform, l, l.label());
                            }
                        });
                });
                let hilly = form.params.landform != Landform::Flat;
                row(ui, "Relief height", |ui| {
                    ui.add_enabled(hilly, egui::DragValue::new(&mut form.params.relief).range(0.0..=400.0).speed(1.0).suffix(" %"));
                });
                row(ui, "Feature size", |ui| {
                    ui.add_enabled(hilly, egui::DragValue::new(&mut form.params.feature_size).range(10.0..=1000.0).speed(1.0).suffix(" %"));
                });
                let mut level = form.params.level.unwrap_or(if hilly { form.params.sea_level as f32 } else { form.params.flat_height });
                row(ui, "Level", |ui| {
                    if ui.add(egui::DragValue::new(&mut level).range(-63.0..=318.0).speed(0.5)).changed() {
                        form.params.level = Some(level);
                        form.params.flat_height = level;
                    }
                });
                row(ui, "Water level", |ui| {
                    ui.add(egui::DragValue::new(&mut form.params.sea_level).range(-63..=318));
                    ui.label(RichText::new("(Minecraft: 63)").size(11.5).color(pal.weak));
                });
                row(ui, "", |ui| {
                    ui.checkbox(&mut form.params.lava, "Lava instead of water");
                });
                row(ui, "", |ui| {
                    ui.add_enabled(form.params.surface.is_none(), egui::Checkbox::new(&mut form.params.beaches, "Beaches"));
                });
            });
            card(ui2, "Surface", "What the land is covered with.", |ui| {
                let mut simple = form.params.surface.is_some();
                ui.radio_value(&mut simple, false, "Automatic: grass, sand, rock and snow by height and slope");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut simple, true, "One terrain:");
                    let reg = &form.registry;
                    let current = form.params.surface.unwrap_or_else(|| reg.terrain_by_name("Grass").unwrap_or(massif_core::TerrainId::ROCK));
                    let mut chosen = current;
                    ui.add_enabled_ui(simple, |ui| {
                        egui::ComboBox::from_id_salt("surface terrain")
                            .selected_text(&reg.terrain(current).name)
                            .height(320.0)
                            .show_ui(ui, |ui| {
                                for (i, t) in reg.terrains.iter().enumerate() {
                                    ui.selectable_value(&mut chosen, massif_core::TerrainId(i as u16), &t.name);
                                }
                            });
                    });
                    if simple {
                        form.params.surface = Some(chosen);
                    }
                });
                if !simple {
                    form.params.surface = None;
                }
            });
        });
    });
}

fn new_world_preview(ui: &mut Ui, form: &mut NewWorldForm) {
    let pal = Palette::of(ui);
    let mut params = form.params.clone();
    params.seed = parse_seed(&form.seed);
    params.tiles_x = (form.width + 127) / 128;
    params.tiles_z = (form.depth + 127) / 128;
    let key = format!("{params:?}");
    if form.preview.as_ref().is_none_or(|(k, _)| *k != key) {
        let image = preview_image(&params, &form.registry, 180);
        let texture = ui.ctx().load_texture("new world preview", image, egui::TextureOptions::LINEAR);
        form.preview = Some((key, texture));
    }
    let side = 320.0;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    ui.painter().rect_filled(rect, 8, pal.canvas);
    if let Some((_, texture)) = &form.preview {
        // Non-square maps keep their proportions.
        let (w, d) = (form.width as f32, form.depth as f32);
        let k = side / w.max(d);
        let r = Rect::from_center_size(rect.center(), vec2(w * k, d * k));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(w / w.max(d), d / w.max(d)));
        ui.painter().image(texture.id(), r, uv, egui::Color32::WHITE);
    }
    ui.label(RichText::new("Preview of the whole map; the cross is the centre.").size(11.0).color(pal.weak));
    let c = rect.center();
    ui.painter().line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], Stroke::new(1.0, egui::Color32::WHITE));
    ui.painter().line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], Stroke::new(1.0, egui::Color32::WHITE));
}

/// What a new world will look like from above: terrain colours, water and
/// hill shading, over the whole map.
fn preview_image(params: &GenParams, reg: &massif_core::Registry, n: usize) -> egui::ColorImage {
    let p = massif_core::generate::preview(params, reg, n);
    let sea = params.sea_level as f32;
    let fluid = if params.lava { [214.0, 96.0, 18.0] } else { [48.0, 96.0, 200.0] };
    let s = p.spacing as f32;
    let mut pixels = Vec::with_capacity(n * n);
    for z in 0..n {
        for x in 0..n {
            let at = |x: usize, z: usize| p.heights[z.min(n - 1) * n + x.min(n - 1)];
            let h = at(x, z);
            let (top, _) = panels::terrain_colors(reg, p.terrains[z * n + x].0);
            let mut c = [top.r() as f32, top.g() as f32, top.b() as f32];
            // Lit from the north-west.
            let dx = (at(x + 1, z) - at(x.saturating_sub(1), z)) / (2.0 * s);
            let dz = (at(x, z + 1) - at(x, z.saturating_sub(1))) / (2.0 * s);
            let shade = (1.0 - (dx + dz) * 1.6).clamp(0.45, 1.35);
            for v in &mut c {
                *v *= shade;
            }
            if h < sea {
                let t = 0.55 + 0.4 * ((sea - h) / 20.0).min(1.0);
                for i in 0..3 {
                    c[i] = c[i] * (1.0 - t) + fluid[i] * t;
                }
            }
            pixels.push(egui::Color32::from_rgb(c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8));
        }
    }
    egui::ColorImage::new([n, n], pixels)
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

fn random_seed() -> i64 {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0) as u64;
    let mut x = nanos ^ 0x9e37_79b9_7f4a_7c15;
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    (x >> 16) as i64
}

/// Numbers are used as-is; any other text is hashed, like Minecraft does.
fn parse_seed(text: &str) -> i64 {
    let t = text.trim();
    if let Ok(n) = t.parse::<i64>() {
        return n;
    }
    // Java's String.hashCode.
    t.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

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
            app.layout.active = 3; // 3D: the preview takes the stage
            app.tools.select(Tool::Dig);
            app.tools.brush.radius = 14.0;
            app.tools.brush.strength = 1.0;
            app.view3d.camera.distance = 160.0;
        }
        h.run_steps(30);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas
            .into_iter()
            .find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::View3d))
            .map(|(_, r)| r)
            .expect("the 3D workspace shows the 3D view");
        let at = view.center();
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(3);
        assert!(h.state().view3d.hover().is_some(), "the cursor points at the terrain");
        let press = |pressed, button| egui::Event::PointerButton {
            pos: at,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.event(press(true, egui::PointerButton::Primary));
        for _ in 0..20 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.event(press(false, egui::PointerButton::Primary));
        h.run_steps(30);
        assert_eq!(h.state().doc.history.undo_label(), Some("Dig (3D)"));
        assert!(!h.state().doc.dimension().voxels.is_empty());
        h.render().unwrap().save(dir.join("sculpt-dig.png")).unwrap();
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("sculpt-dig-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // The right button builds.
        h.event(press(true, egui::PointerButton::Secondary));
        for _ in 0..30 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.event(press(false, egui::PointerButton::Secondary));
        h.run_steps(30);
        assert_eq!(h.state().doc.history.undo_label(), Some("Dig (3D)"));
        assert!(h.state().doc.history.can_undo());
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
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
            app.layout.activate(6); // Graph, on the terrain side
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
        h.state_mut().layout.activate(7);
        h.run_steps(5);
        wait_for_graph(&mut h);
        h.render().unwrap().save(dir.join("graph-render-workspace.png")).unwrap();
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.active = 6;
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
        let rect = map_rect(h.state(), &h.ctx);
        let click = |h: &mut egui_kittest::Harness<'_, MassifApp>, p: egui::Pos2, button: egui::PointerButton| {
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
            h.run_steps(30);
        };
        let c = rect.center();
        for d in [vec2(-320.0, -260.0), vec2(-150.0, -60.0), vec2(40.0, 30.0), vec2(200.0, 200.0), vec2(330.0, 330.0)] {
            click(&mut h, c + d, egui::PointerButton::Primary);
        }
        // A stray point, taken back with the right button.
        click(&mut h, c + vec2(-330.0, 300.0), egui::PointerButton::Primary);
        click(&mut h, c, egui::PointerButton::Secondary);
        assert_eq!(h.state().tools.river_points.len(), 5);
        h.event(egui::Event::PointerMoved(c + vec2(360.0, 360.0)));
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
        assert!(h.state().tools.river_points.is_empty());
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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
            h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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

    /// A river drawn by dragging along its course: let go, and it is dug,
    /// with water. No GPU needed.
    #[test]
    fn a_river_is_drawn_by_dragging() {
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
            app.tools.river.width = 9.0;
            app.tools.river.width_end = 20.0;
        }
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let c = rect.center();
        let from = c + vec2(-300.0, -220.0);
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        let mut last = from;
        for k in 1..=40 {
            let t = k as f32 / 40.0;
            last = from + vec2(600.0 * t, 440.0 * t + 60.0 * (t * 6.0).sin());
            h.event(egui::Event::PointerMoved(last));
            h.step();
        }
        assert!(h.state().tools.river_points.len() > 8, "the course follows the pointer: {}", h.state().tools.river_points.len());
        h.event(egui::Event::PointerButton {
            pos: last,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(5);
        assert_eq!(h.state().doc.history.undo_label(), Some("River"), "let go, the river is dug");
        assert!(h.state().tools.river_points.is_empty());
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
        let rect = map_rect(h.state(), &h.ctx);
        let screen = |x: f64, z: f64| rect.center() + vec2(((x - centre[0]) * 0.75) as f32, ((z - centre[1]) * 0.75) as f32);
        let from = screen(150.0, -560.0);
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: Modifiers::NONE,
        });
        h.step();
        let mut last = from;
        for k in 1..=60 {
            let t = k as f64 / 60.0;
            last = screen(150.0 + 500.0 * t + 70.0 * (t * 9.0).sin(), -560.0 + 790.0 * t);
            h.event(egui::Event::PointerMoved(last));
            h.step();
        }
        h.render().unwrap().save(dir.join("real-river-tool-course.png")).unwrap();
        h.event(egui::Event::PointerButton {
            pos: last,
            button: egui::PointerButton::Primary,
            pressed: false,
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
            app.tools.select(Tool::Spring);
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
        let note = h.state().tools.river_note.clone().unwrap_or_default();
        eprintln!("{note}");
        assert_eq!(h.state().doc.history.undo_label(), Some("River from a source"), "{note}");
        assert!(note.starts_with("A river"), "{note}");
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
        let map = std::sync::Arc::new(massif_core::heightmap::read(&png).unwrap());
        h.state_mut().dialog = Some(Dialog::FromHeightmap(HeightmapForm {
            path: png.clone(),
            map,
            how: massif_core::generate::FromHeightmap {
                name: "Cone".into(),
                scale: 2.0,
                low: 40.0,
                high: 200.0,
                sea_level: 63,
            },
            picture: None,
        }));
        h.run_steps(3);
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label("Make the world").click();
        }
        h.run_steps(5);
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
        h.state_mut().layout.activate(6);
        h.run_steps(2);
        {
            let app = h.state_mut();
            app.graph.use_template(&mut app.doc, Template::Alpine);
            let mut g = massif_core::graph::Graph::default();
            let pic = g.add(
                NodeKind::Heightmap {
                    path: png.display().to_string(),
                    low: 60.0,
                    high: 160.0,
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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
        h.state_mut().layout.activate(6);
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

    /// The new-world and export dialogs, rendered.
    #[test]
    fn dialogs_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(5);
        // The welcome screen, and a template started from it.
        h.state_mut().dialog = Some(Dialog::Welcome);
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-welcome.png")).unwrap();
        {
            use egui_kittest::kittest::Queryable;
            h.get_by_label_contains("Give it rivers").click();
        }
        h.run_steps(5);
        assert!(h.state().dialog.is_none(), "the welcome screen closes");
        assert!(h.state().layout.terrain(), "on the Terrain side");
        assert!(h.state().doc.world.meta.graph.as_ref().is_some_and(|g| g.reads_the_map()), "with rivers on the map");
        h.state_mut().layout.switch_side(false);
        h.run_steps(3);
        h.state_mut().open_new_dialog();
        if let Some(Dialog::NewWorld(form)) = &mut h.state_mut().dialog {
            form.params.landform = Landform::Mountains;
            form.width = 4096;
            form.depth = 4096;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-new-world.png")).unwrap();
        assert!(matches!(&h.state().dialog, Some(Dialog::NewWorld(f)) if f.preview.is_some()));
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
        h.state_mut().layout.active = 5; // Minimal: tools | map
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
