//! The application: window chrome, workspaces and areas, dialogs, jobs.

use crate::access::Follow;
use crate::blocks::BlockLibrary;
use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
use crate::home::{self, HomeAction, Page};
use crate::jobs::{self, Job, Outcome};
use crate::layout::{AreaId, Dir, EditorKind, Layout, Node, Zone};
use crate::map_render::MapCache;
use crate::map_view::{MapInput, MapView};
use crate::menu::Command;
use crate::panels;
use crate::sounds::Cue;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::{Tool, ToolState};
use crate::view3d::View3d;
use egui::{
    Align, Align2, Color32, CursorIcon, FontId, Id, LayerId,
    Order, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, ViewportCommand,
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
pub(crate) enum ThemeChoice {
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
    /// The jars of the mods whose blocks are painted with too (Conquest
    /// Reforged and the like).
    mod_jars: Vec<PathBuf>,
    /// The accent colour: what is chosen, the main buttons, the art of the
    /// home screen.
    accent: [u8; 3],
    /// Which sounds play, and how loud (0–1).
    sounds: crate::sounds::SoundLevel,
    volume: f32,
    /// When the system tells that a long task ended.
    notify: crate::notify::NotifyWhen,
    /// Memory for what is kept decoded (the map's tiles and its images),
    /// in megabytes; 0: as much as it takes.
    memory_mb: u32,
    /// How the dots of the loaders light while Massif works.
    loader: crate::loader::Pattern,
    /// The tools' icons: Line (Lucide's style) or Classic.
    tool_icons: crate::icons::IconStyle,
    /// The home screen's picture moves (its dither, about once a second).
    animate_backdrop: bool,
    /// The crosshair of the player's view.
    reticle: crate::reticle::Reticle,
    /// The home screen's pictures shown, by name: those not yet come first.
    backdrops_seen: Vec<String>,
    /// The Developer panel is offered: frames, memory, the processor, live.
    developer: bool,
    /// Developer mode: how many frames a second the window may draw, and
    /// whether it draws every frame, changed or not (to measure).
    frame_limit: crate::devtools::FrameLimit,
    draw_always: bool,
    /// The typeface the interface is set in.
    typeface: theme::Typeface,
    /// High contrast: stronger text, lines and edges — as the system says,
    /// or always off, or always on.
    high_contrast: Follow,
    /// Less motion: views stop where they are let go, pictures stay still.
    reduce_motion: Follow,
    /// The size of the whole interface, 1 as it is drawn (⌘+ and ⌘−
    /// change it too).
    ui_scale: f32,
    /// A backup of the map while it has changes not saved (see `autosave`),
    /// every `autosave_minutes`.
    autosave: bool,
    autosave_minutes: u32,
    /// Ask GitHub, once a day at most, whether a newer version is out
    /// (see `update`).
    check_updates: bool,
    /// The version the user chose not to hear about again.
    skipped_version: Option<String>,
    /// The author written in the packs exported.
    pack_author: String,
    /// The language of the interface: the system's, or one chosen.
    language: crate::i18n::Language,
    /// The keyboard shortcuts: a preset and what was changed in it.
    keys: crate::keymap::Saved,
    /// The guided tour was taken or skipped: it does not show on its own
    /// again (Help ▸ Take the tour still shows it). Settings saved before
    /// it existed are of someone who knows Massif already.
    #[serde(default = "yes")]
    tour_done: bool,
    /// What the graphics card is asked to do (Settings ▸ Graphics); None
    /// until the first start picks a preset for the card.
    graphics: Option<crate::graphics::Graphics>,
    /// The card the first start picked the preset for, and how long its
    /// trial frame took: what Settings ▸ Graphics says of the choice.
    graphics_chosen_for: Option<(String, f32)>,
}

fn yes() -> bool {
    true
}

/// The intervals of the backups offered in the settings, in minutes.
const AUTOSAVE_MINUTES: [u32; 5] = [1, 2, 5, 10, 15];

/// The sizes of the interface offered in the settings.
const UI_SCALES: [f32; 4] = [1.0, 1.1, 1.25, 1.5];

/// A section of the settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PrefsSection {
    Appearance,
    Accessibility,
    Typeface,
    Sounds,
    Notifications,
    Memory,
    Autosave,
    Keyboard,
    Updates,
    Graphics,
    Developer,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            recent: Vec::new(),
            solo: false,
            light: crate::map_render::DEFAULT_LIGHT,
            minecraft_jar: None,
            mod_jars: Vec::new(),
            accent: theme::DEFAULT_ACCENT,
            sounds: crate::sounds::SoundLevel::Essential,
            volume: 0.6,
            notify: crate::notify::NotifyWhen::Background,
            memory_mb: 0,
            loader: crate::loader::Pattern::Orbit,
            tool_icons: crate::icons::IconStyle::Line,
            animate_backdrop: true,
            reticle: crate::reticle::Reticle::default(),
            backdrops_seen: Vec::new(),
            developer: false,
            frame_limit: crate::devtools::FrameLimit::Vsync,
            draw_always: false,
            typeface: theme::Typeface::System,
            high_contrast: Follow::System,
            reduce_motion: Follow::System,
            ui_scale: 1.0,
            autosave: true,
            autosave_minutes: 5,
            check_updates: true,
            skipped_version: None,
            pack_author: String::new(),
            language: crate::i18n::Language::System,
            tour_done: false,
            graphics: None,
            graphics_chosen_for: None,
            keys: crate::keymap::Saved::default(),
        }
    }
}

/// How the accessibility settings were last applied.
#[derive(Default)]
struct AccessState {
    /// What the system asks for, read every few seconds.
    system: crate::access::System,
    checked: Option<f64>,
    /// The size of the interface last set: when egui's differs, ⌘+ or ⌘−
    /// changed it.
    zoom: Option<f32>,
}

/// An action that replaces or closes the document, run once unsaved changes
/// are dealt with.
#[derive(Debug, Clone)]
enum Pending {
    New(GenParams),
    Open(Option<PathBuf>),
    /// A new world from a height map.
    FromHeightmap(std::sync::Arc<massif_core::heightmap::Heightmap>, massif_core::generate::FromHeightmap),
    /// The newest backup left by a run that closed without saving (see
    /// `autosave`), restored.
    Recover,
    /// A new world from the relief of a real place.
    Real(Box<crate::real_terrain::RealJob>),
    /// A save point of the map, opened as a map of its own: read-only, or
    /// as a copy.
    OpenPoint { world: std::sync::Arc<massif_core::World>, name: String, read_only: bool },
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
    /// The export lays ores (see Edit ▸ Ores).
    ores: bool,
    structures: bool,
    /// The map has a selection, and only its tiles are to be exported.
    has_selection: bool,
    selection_only: bool,
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
    /// The mods whose blocks the map uses, to warn of a version or a mod
    /// that does not match.
    mods: Vec<ModUse>,
}

/// The blocks of a mod a map uses.
#[derive(Debug, Clone, PartialEq)]
struct ModUse {
    /// The mod's name, or its namespace when it is not read.
    name: String,
    namespace: String,
    /// How many of the map's blocks are its.
    blocks: usize,
    /// The version of the game it is made for, when it is read.
    minecraft: Option<String>,
    /// Whether the Blocks panel reads it.
    loaded: bool,
    /// Its states the game does not have, by its list of blocks.
    unknown: Vec<String>,
}

/// The mods whose blocks a map's terrains, covers and objects use, by
/// namespace, with what the catalogue knows of them.
fn mod_uses(reg: &massif_core::registry::Registry, cat: Option<&massif_assets::Catalogue>) -> Vec<ModUse> {
    let mut states: Vec<&str> = reg.blocks.iter().map(|b| b.state.as_str()).collect();
    states.extend(reg.covers.iter().filter(|c| !c.removed).flat_map(|c| c.plants.iter().map(|p| p.0.as_str())));
    states.extend(reg.objects.iter().flat_map(|o| o.palette.iter().map(String::as_str)));
    states.sort_unstable();
    states.dedup();
    let mut out: Vec<ModUse> = Vec::new();
    for state in states {
        let ns = massif_assets::namespace(state);
        if ns == "minecraft" || !state.contains(':') {
            continue;
        }
        let k = match out.iter().position(|u| u.namespace == ns) {
            Some(k) => k,
            None => {
                let info = cat.and_then(|c| c.mod_of(state));
                out.push(ModUse {
                    name: info.map_or_else(|| ns.to_owned(), |m| m.name.clone()),
                    namespace: ns.to_owned(),
                    blocks: 0,
                    minecraft: info.and_then(|m| m.minecraft.clone()),
                    loaded: info.is_some(),
                    unknown: Vec::new(),
                });
                out.len() - 1
            }
        };
        out[k].blocks += 1;
        if let Some(Err(problem)) = cat.map(|c| c.check_state(state)) {
            out[k].unknown.push(problem.to_string());
        }
    }
    out
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
    /// The settings, at a section.
    Preferences(PrefsSection),
    /// The rules of the automatic biomes (false) or paint (true).
    AutoRules(bool),
    /// What an import brought in, with the file's thumbnail.
    ImportReport(Box<massif_legacy::ImportReport>, Option<egui::TextureHandle>),
    /// The help, at a topic.
    Help(crate::help::HelpState),
    /// A filter over the whole map: its settings, the map before and after.
    Reshape(Box<crate::reshape::ReshapeForm>),
    /// The survival check of the whole map.
    Analysis(Box<crate::analysis::AnalysisView>),
    /// The ores the export lays.
    Ores(Box<crate::ores::OresView>),
    /// Backups left by a run of Massif that closed without saving, to
    /// restore or discard.
    Recover,
    /// A newer version is out: its notes, and its page to open.
    Update,
    /// Caves to carve: their settings.
    Caves(Box<crate::caves::CaveForm>),
    /// The relief of a real place: where, at what scale, how high.
    Real(Box<crate::real_terrain::RealForm>),
    /// A save point to make (`None`) or rename (its index): its name.
    PointName(Option<usize>, String),
    /// A save point to delete, once sure.
    DeletePoint(usize),
    /// Massif stopped on an error during a frame: what it said, and the
    /// copy of the map written then.
    Crashed(String, Option<PathBuf>),
    /// A problem report to save (see `report`).
    Report(Box<crate::report_view::ReportForm>),
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
    /// A copy of the settings eframe keeps, written whole or not at all.
    settings_backup: crate::settings_backup::SettingsBackup,
    /// When frames stopped on an error lately.
    panics: Vec<std::time::Instant>,
    /// Whether the document is a map the user made or opened (not the one
    /// held while the home screen is shown at startup).
    has_map: bool,
    /// The home screen, shown instead of the editor.
    home: Option<home::Home>,
    /// The template the map being made takes its relief from.
    after_create: Option<massif_core::graph::Template>,
    /// What the covers looked like last frame: when it changes, the map
    /// and the 3D views redraw.
    covers_key: Option<u64>,
    next_generation: u64,
    /// The settings caves were last carved with (per run of the app).
    caves: Option<massif_core::caves::CaveSettings>,
    /// The place, scale and heights the relief of a real place was last
    /// made with (per run of the app).
    real: Option<crate::real_terrain::Remembered>,
    /// The Versions panel: the save points' pictures and the comparison.
    versions: crate::versions::VersionsUi,
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
    /// Backups of the map while it has changes not saved.
    autosave: crate::autosave::Autosave,
    /// The backups' folder, being opened on a thread of its own at the
    /// start (see [`crate::autosave::Autosave::open_in_background`]).
    autosave_opening: Option<std::sync::mpsc::Receiver<crate::autosave::Autosave>>,
    /// Backups were found: the Recover dialog is shown once no other is.
    offer_recovery: bool,
    /// The latest version published, as GitHub last said.
    updates: crate::update::Updates,
    /// Whether a dialog was open last frame (for its sound).
    dialog_was_open: bool,
    /// The survival check last closed: opened again as it was while the
    /// map has not changed.
    analysis: Option<Box<crate::analysis::AnalysisView>>,
    /// When the memory limit was last applied (seconds).
    memory_checked: f64,
    /// The first start's trial of the graphics card, running (see
    /// `graphics::suggest`): the card, and its frame time to come.
    graphics_probe: Option<(crate::graphics::Card, std::sync::mpsc::Receiver<f32>)>,
    /// Frames of the 3D views watched for slowness, and the lighter preset
    /// a quiet note suggests.
    slow_frames: crate::graphics::SlowWatch,
    slow_note: Option<crate::graphics::Preset>,
    /// When the user last touched anything, and when the last frame began:
    /// an idle window draws at most `idle_fps` frames a second.
    last_input: f64,
    frame_began: Option<std::time::Instant>,
    /// When the last frame began, for the limit of Settings ▸ Developer.
    paced: Option<std::time::Instant>,
    /// What the Developer panel shows, measured frame after frame.
    dev: crate::devtools::DevStats,
    /// A performance recording, on or done, for the problem report.
    recording: Option<crate::report::Recording>,
    /// The problem report's dialog while a recording is on, to open again
    /// as it was once it stops.
    report_aside: Option<Box<crate::report_view::ReportForm>>,
    /// The dialog the report was opened from (after a crash, after an
    /// error), shown again once it closes.
    report_return: Option<Dialog>,
    /// A problem report being written: where, or why it could not be.
    report_saving: Option<std::sync::mpsc::Receiver<Result<PathBuf, String>>>,
    /// The graphics card, as wgpu says it, for the problem report.
    gpu: Option<crate::report::Gpu>,
    after_save: Option<Pending>,
    status: Option<Status>,
    feedback: Option<String>,
    dragging_area: Option<AreaId>,
    rename: Option<(usize, String)>,
    /// The Layers panel's layer being renamed.
    layers_ui: crate::layers::LayersUi,
    /// A dialog about packs: export one, import one whose name is taken,
    /// remove one.
    pack_dialog: Option<crate::pack_share::PackDialog>,
    /// Pack files to import (dropped, opened from the Finder or Explorer).
    pack_inbox: Vec<PathBuf>,
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
    /// The accessibility settings, as applied.
    access: AccessState,
    /// The window has no native title bar: the top bar hosts the traffic
    /// lights and moves the window.
    unified_titlebar: bool,
    /// File, Edit, View and Help are in the system's menu bar (macOS), not
    /// in the top bar.
    menus_in_menu_bar: bool,
    /// The guided tour, while it is shown.
    tour: Option<crate::tour::Tour>,
    /// The tour asked for (Help ▸ Take the tour, or the first run): shown
    /// once a map is open, with nothing else in the way.
    tour_wanted: bool,
    /// A sample map being made: its relief built from its template, then
    /// the World side shown.
    sample: Option<SampleStage>,
}

/// Where a sample map is in its making.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SampleStage {
    /// Asked for: the map is being made.
    Making,
    /// Made: its relief is being built from the template's graph.
    Building,
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

/// A typeface to choose, shown in itself: its name and a line set in it,
/// a dot on the right for the one chosen, what it is like on hover.
fn typeface_row(ui: &mut Ui, face: theme::Typeface, on: bool) -> egui::Response {
    const SAMPLE: &str = tn!("Ridges, rivers and coasts — 2 048 × 2 048 blocks");
    let pal = Palette::of(ui);
    let first = ui.cursor().top() <= ui.max_rect().top() + 0.5;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 54.0), Sense::click());
    if !first {
        ui.painter().hline(rect.x_range().expand(12.0), rect.top(), Stroke::new(1.0, pal.border));
    }
    let hovered = response.hovered();
    let family = face.family();
    let name = egui::WidgetText::from(RichText::new(face.name()).family(family.clone()).size(15.0).variation(theme::WGHT, theme::SEMIBOLD))
        .into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let sample = egui::WidgetText::from(RichText::new(SAMPLE).family(family).size(12.5))
        .into_galley(ui, Some(egui::TextWrapMode::Truncate), rect.width() - 32.0, egui::TextStyle::Body);
    let (name_h, gap) = (name.size().y, 2.0);
    let top = rect.center().y - (name_h + gap + sample.size().y) / 2.0;
    let strong = if on || hovered { pal.text } else { theme::mix(pal.text, pal.weak, 0.3) };
    ui.painter().galley(egui::pos2(rect.left(), top), name, strong);
    ui.painter().galley(egui::pos2(rect.left(), top + name_h + gap), sample, pal.weak);
    let dot = egui::pos2(rect.right() - 9.0, rect.center().y);
    if on {
        ui.painter().circle_stroke(dot, 7.5, Stroke::new(1.5, pal.text));
        ui.painter().circle_filled(dot, 3.5, pal.text);
    } else {
        ui.painter().circle_stroke(dot, 7.5, Stroke::new(1.0, if hovered { pal.text } else { pal.weak }));
    }
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, face.name()));
    response.on_hover_cursor(egui::CursorIcon::PointingHand).on_hover_text(crate::i18n::tr_name(face.about()))
}

impl MassifApp {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        // Each part written down in startup.log with the time it took, so
        // that a start held up on a slow computer names it.
        use crate::startup::timed;
        timed("the fonts and the theme", || theme::install(&cc.egui_ctx));
        // eframe's copy first; the backup when a broken write lost it.
        let backup = timed("the settings' backup copy", crate::settings_backup::SettingsBackup::installed);
        let layout = cc
            .storage
            .and_then(|s| eframe::get_value::<Layout>(s, LAYOUT_KEY))
            .or_else(|| backup.get(LAYOUT_KEY))
            .map(Layout::sanitize)
            .unwrap_or_default();
        let settings: Settings = cc
            .storage
            .and_then(|s| eframe::get_value(s, "massif.settings"))
            .or_else(|| backup.get("massif.settings"))
            .unwrap_or_default();
        let mut tools: ToolState = cc
            .storage
            .and_then(|s| eframe::get_value(s, "massif.tools"))
            .or_else(|| backup.get("massif.tools"))
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
        crate::i18n::set_language(settings.language);
        theme::set_accent(&cc.egui_ctx, settings.accent);
        crate::loader::set_pattern(&cc.egui_ctx, settings.loader);
        crate::icons::set_style(&cc.egui_ctx, settings.tool_icons);
        crate::reticle::set(&cc.egui_ctx, settings.reticle);
        timed("the typeface chosen", || theme::set_typeface(&cc.egui_ctx, settings.typeface));
        let world = timed("the map made up before one is opened", default_world);
        let mut app = timed("the app's state", || Self::with_world(world, layout, tools, settings));
        timed("the graphics card's share of the app", || app.start_graphics(cc));
        if let Some(render) = cc.storage.and_then(|s| eframe::get_value(s, "massif.render")).or_else(|| backup.get("massif.render")) {
            app.graph.view.render = render;
        }
        app.settings_backup = backup;
        app.unified_titlebar = cfg!(target_os = "macos");
        app.menus_in_menu_bar = crate::menubar::NATIVE;
        timed("the brushes, loading in the background", || {
            app.brushes = BrushLibrary::installed();
            app.brushes.load_in_background(&cc.egui_ctx);
        });
        timed("the game's blocks, loading in the background", || app.blocks.load(&cc.egui_ctx, app.settings.minecraft_jar.clone(), app.settings.mod_jars.clone()));
        // What a run before left unsaved, offered first, once the folder
        // is read (on a thread of its own).
        if let Some(dir) = crate::autosave::default_dir() {
            app.autosave_opening = Some(crate::autosave::Autosave::open_in_background(&cc.egui_ctx, dir));
        }
        // Whether a newer version is out, asked off the main thread.
        timed("whether a newer version is out (asked in the background)", || app.check_updates(&cc.egui_ctx));
        // The first time Massif opens: the tour, once a map is open.
        app.tour_wanted = !app.settings.tour_done;
        // Tests keep away from the packs of this computer.
        if !cfg!(test) {
            app.layers_ui.library = crate::pack_share::PackLibrary::installed();
        }
        crate::startup::step("the home screen");
        if let Some(path) = open.as_ref().filter(|p| crate::pack_share::is_pack_file(p)) {
            // A pack opened from the Finder or Explorer: into the packs,
            // from the home screen.
            app.pack_inbox.push(path.clone());
            app.has_map = false;
            app.open_home(Page::NewMap);
        } else if let Some(path) = open {
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
            .set_title(tr!("A height map to make a map of"))
            .add_filter(tr!("Height maps"), &["png", "jpg", "jpeg", "r16", "r32", "raw"])
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
            Err(e) => self.say(ctx, tr!("That height map cannot be read: {e}", e), true, None),
        }
    }

    /// The backups' folder, once read: what a run before left unsaved is
    /// offered (unless a dialog is up: then once it closes).
    fn poll_autosave_opening(&mut self) {
        if let Some(rx) = &self.autosave_opening {
            match rx.try_recv() {
                Ok(opened) => {
                    self.autosave_opening = None;
                    // Until now the backups went nowhere (`Autosave::off`):
                    // nothing of the one being replaced is lost.
                    self.autosave = opened;
                    self.offer_recovery = !self.autosave.found.is_empty();
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.autosave_opening = None,
            }
        }
        if self.offer_recovery && self.dialog.is_none() {
            self.offer_recovery = false;
            self.dialog = Some(Dialog::Recover);
        }
    }

    /// Show the home screen, on `page`; with a map open, the way back to it.
    fn open_home(&mut self, page: Page) {
        // A tour under way starts again over the next map.
        if self.tour.take().is_some() {
            self.tour_wanted = true;
        }
        if self.home.is_none() {
            let mut registry = massif_core::Registry::vanilla();
            if let Some(cat) = &self.blocks.catalogue {
                massif_assets::apply_colors(&mut registry, cat);
            }
            let mut home = home::Home::new(registry);
            home.pick_backdrop(&mut self.settings.backdrops_seen);
            self.home = Some(home);
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
                self.sample = None;
                self.request(ctx, Pending::New(params));
            }
            HomeAction::Sample(t) => {
                self.after_create = Some(t);
                self.sample = Some(SampleStage::Making);
                self.request(ctx, Pending::New(home::sample_params(t)));
            }
            HomeAction::FromHeightmap(map, how) => self.request(ctx, Pending::FromHeightmap(map, how)),
            HomeAction::PickHeightmap => self.pick_heightmap(ctx),
            HomeAction::RealPlace => self.open_real(true),
            HomeAction::AnotherPicture => {
                if let Some(home) = &mut self.home {
                    home.pick_backdrop(&mut self.settings.backdrops_seen);
                }
            }
            HomeAction::Open => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(tr!("Open a map"))
                    .add_filter(tr!("Massif map"), &[massif_format::EXTENSION])
                    .pick_file()
                {
                    self.request(ctx, Pending::Open(Some(path)));
                }
            }
            HomeAction::Import => {
                if let Some(path) = rfd::FileDialog::new()
                    .set_title(tr!("Import a WorldPainter world"))
                    .add_filter(tr!("WorldPainter world"), &["world"])
                    .pick_file()
                {
                    self.request(ctx, Pending::Open(Some(path)));
                }
            }
            HomeAction::OpenRecent(path) => {
                if path.exists() {
                    self.request(ctx, Pending::Open(Some(path)));
                } else {
                    self.say(ctx, tr!("{path} is no longer there", path = path.display()), true, None);
                }
            }
            HomeAction::Forget(path) => self.settings.recent.retain(|p| *p != path),
            HomeAction::Reveal(path) => jobs::reveal(&path),
            HomeAction::Back => {
                self.home = None;
                self.after_create = None;
            }
            HomeAction::Settings => self.dialog = Some(Dialog::Preferences(PrefsSection::Appearance)),
            HomeAction::Update => self.dialog = Some(Dialog::Update),
            HomeAction::Dropped(path) => {
                let ext = path.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
                match ext.as_str() {
                    "png" | "jpg" | "jpeg" | "r16" | "r32" | "raw" => self.take_heightmap(ctx, path),
                    "world" => self.request(ctx, Pending::Open(Some(path))),
                    e if e == massif_format::EXTENSION => self.request(ctx, Pending::Open(Some(path))),
                    e if e == massif_core::pack_file::EXTENSION => self.pack_inbox.push(path),
                    _ => self.say(ctx, tr!("Massif opens .massif maps, WorldPainter .world worlds, height maps and .massifpack packs"), true, None),
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
        let animate = self.settings.animate_backdrop;
        let update = self.update_offer().map(|r| r.tag_name.trim_start_matches('v').to_owned());
        egui::CentralPanel::default().frame(egui::Frame::new()).show(ui, |ui| {
            if let Some(home) = &mut self.home {
                home.animate = animate;
                home.update = update;
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
            self.say(ctx, tr!("The map is empty"), true, None);
            return;
        }
        let Some(path) = rfd::FileDialog::new()
            .set_title(tr!("Save the height map"))
            .add_filter(tr!("16-bit PNG"), &["png"])
            .set_file_name(tr!("{name} height map.png", name = self.doc.world.meta.name))
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
                tr!("Saved a height map of {pw} × {pd}: black is Y {min_y:.0}, white Y {max_y:.0}{value}", value = if k < 1.0 { tr!(", {k:.1} blocks a pixel", k = 1.0 / k) } else { String::new() }, pw, pd, min_y, max_y),
                false,
                path.parent().map(|p| p.to_path_buf()),
            ),
            Err(e) => self.say(ctx, tr!("The height map could not be saved: {e}", e), true, None),
        }
    }

    /// Go to the Terrain side and start from a template.
    /// The guided tour: started once a map is shown with nothing else in
    /// the way (a dialog, a map being opened, a sample being built), drawn
    /// over the editor, and what its closing says kept.
    fn tour_ui(&mut self, ctx: &egui::Context) {
        // A sample's relief built: its map shown.
        if self.sample == Some(SampleStage::Building) && !self.graph.converting() {
            self.sample = None;
            self.layout.switch_side(false);
            // As it was made: nothing to save yet.
            self.doc.forget_changes();
        }
        let clear = self.dialog.is_none() && self.pack_dialog.is_none() && self.blocking_job().is_none() && self.sample.is_none();
        if self.tour.is_none() && self.tour_wanted && self.has_map && clear {
            self.tour_wanted = false;
            self.start_tour();
        }
        if !clear {
            return;
        }
        if let Some(tour) = &mut self.tour
            && let Some(_closed) = tour.ui(ctx)
        {
            // "Don't show again" unticked: shown again at the next start.
            self.settings.tour_done = tour.dont_show;
            self.tour = None;
            crate::sounds::play(Cue::Close);
        }
    }

    /// Show the tour, on the World side, in a workspace with the parts it
    /// talks about.
    fn start_tour(&mut self) {
        use crate::layout::EditorKind as E;
        self.layout.switch_side(false);
        let parts = [E::Map, E::Tools, E::BrushSettings, E::View3d];
        let has_all = |w: &crate::layout::Workspace| !w.terrain && parts.iter().all(|k| w.root.shows(*k));
        if !has_all(self.layout.current())
            && let Some(i) = self.layout.workspaces.iter().position(has_all)
        {
            self.layout.activate(i);
        }
        self.layout.current_mut().maximized = None;
        self.tour = Some(crate::tour::Tour::default());
        crate::sounds::play(Cue::Open);
    }

    /// Help ▸ Take the tour: over the map open, or over a sample made for
    /// it when there is none.
    fn take_the_tour(&mut self, ctx: &egui::Context) {
        self.tour_wanted = true;
        if self.home.is_some() {
            if self.has_map {
                self.home_action(ctx, HomeAction::Back);
            } else {
                self.home_action(ctx, HomeAction::Sample(home::SAMPLES[0]));
            }
        }
    }

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
            settings_backup: Default::default(),
            panics: Vec::new(),
            has_map: true,
            home: None,
            after_create: None,
            covers_key: None,
            next_generation: 1,
            caves: None,
            real: None,
            versions: Default::default(),
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
            autosave: crate::autosave::Autosave::off(),
            autosave_opening: None,
            offer_recovery: false,
            updates: crate::update::Updates::default(),
            dialog_was_open: false,
            analysis: None,
            memory_checked: f64::NEG_INFINITY,
            graphics_probe: None,
            slow_frames: Default::default(),
            slow_note: None,
            last_input: 0.0,
            frame_began: None,
            paced: None,
            dev: crate::devtools::DevStats::default(),
            recording: None,
            report_aside: None,
            report_return: None,
            report_saving: None,
            gpu: None,
            after_save: None,
            status: None,
            feedback: None,
            dragging_area: None,
            rename: None,
            layers_ui: Default::default(),
            pack_dialog: None,
            pack_inbox: Vec::new(),
            terrain_search: String::new(),
            biome_search: String::new(),
            brushes: BrushLibrary::default(),
            new_group: None,
            graph: Default::default(),
            blocks: BlockLibrary::default(),
            allow_close: false,
            title: String::new(),
            applied_theme: None,
            access: AccessState::default(),
            unified_titlebar: false,
            menus_in_menu_bar: false,
            tour: None,
            tour_wanted: false,
            sample: None,
        }
    }

    /// The open map, for tests outside this module.
    #[cfg(test)]
    pub fn doc_for_tests(&mut self) -> &mut Document {
        &mut self.doc
    }

    /// The tools, for tests outside this module.
    #[cfg(test)]
    pub fn tools_for_tests(&self) -> &ToolState {
        &self.tools
    }

    /// An app for tests: no persistence, a small world.
    #[cfg(test)]
    pub fn for_tests(world: World) -> Self {
        // The same whatever the Mac running the tests asks for.
        let settings = Settings {
            high_contrast: Follow::Off,
            reduce_motion: Follow::Off,
            tour_done: true,
            ..Settings::default()
        };
        Self::with_world(world, Layout::default(), ToolState::default(), settings)
    }

    fn apply_theme(&self, ctx: &egui::Context) {
        ctx.set_theme(match self.settings.theme {
            ThemeChoice::System => egui::ThemePreference::System,
            ThemeChoice::Light => egui::ThemePreference::Light,
            ThemeChoice::Dark => egui::ThemePreference::Dark,
        });
    }

    /// High contrast, reduced motion and the size of the interface, as the
    /// settings say — and the system, read every few seconds while one of
    /// them follows it. ⌘+ and ⌘− (egui's own) change the size: kept.
    fn apply_access(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let follows = self.settings.high_contrast == Follow::System || self.settings.reduce_motion == Follow::System;
        if crate::access::FOLLOWS_SYSTEM && follows && self.access.checked.is_none_or(|t| now - t >= 2.0) {
            self.access.system = crate::access::system();
            self.access.checked = Some(now);
        }
        let high = self.settings.high_contrast.on(self.access.system.contrast);
        let still = self.settings.reduce_motion.on(self.access.system.reduce_motion);
        if (high, still) != (theme::high_contrast(ctx), theme::reduce_motion(ctx)) {
            theme::set_access(ctx, high, still);
        }
        let zoom = ctx.zoom_factor();
        if let Some(set) = self.access.zoom
            && (zoom - set).abs() > 1e-3
        {
            // Changed with the keyboard.
            self.settings.ui_scale = zoom;
            self.access.zoom = Some(zoom);
        }
        let scale = self.settings.ui_scale.clamp(0.5, 3.0);
        if self.access.zoom != Some(scale) {
            ctx.set_zoom_factor(scale);
            self.access.zoom = Some(scale);
        }
    }

    /// A task ended: its sound, and the system's notice when it took a while
    /// and the settings ask for one (while Massif is behind, or always).
    fn task_done(&mut self, ctx: &egui::Context, cue: Cue, title: &str, body: &str, secs: f32) {
        crate::sounds::play(cue);
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        let tell = match self.settings.notify {
            crate::notify::NotifyWhen::Never => false,
            crate::notify::NotifyWhen::Background => !focused,
            crate::notify::NotifyWhen::Always => true,
        };
        if tell && secs >= 2.0 {
            crate::notify::send(&format!("Massif — {title}"), body);
        }
    }

    /// Keep within the memory the settings give: three fifths for decoded
    /// tiles (at the size they take now), a quarter for the column data of
    /// the map's images, the rest for their textures. Once a second.
    fn apply_memory_limit(&mut self, now: f64) {
        if now - self.memory_checked < 1.0 {
            return;
        }
        self.memory_checked = now;
        let mb = self.settings.memory_mb as usize;
        if mb == 0 {
            self.doc.max_hot_tiles = crate::doc::MAX_HOT_TILES;
            self.map_cache.max_grids = 384;
            self.map_cache.max_chunks = 1400;
            self.map_cache.max_summary_bytes = crate::map_render::DEFAULT_SUMMARY_BYTES;
            return;
        }
        let budget = mb << 20;
        let dim = self.doc.dimension();
        let (hot, bytes) = dim.levels.iter().map(|l| l.stats()).fold((0usize, 0usize), |(t, b), s| (t + s.hot_tiles, b + s.hot_bytes));
        let per_tile = bytes.checked_div(hot).map_or(64 << 10, |b| b.max(8 << 10));
        let levels = dim.levels.len().max(1);
        self.doc.max_hot_tiles = (budget * 3 / 5 / per_tile / levels).clamp(64, 1 << 20);
        self.map_cache.max_grids = (budget / 4 / crate::map_render::GRID_BYTES).clamp(64, 8192);
        self.map_cache.max_summary_bytes = (budget / 10).clamp(16 << 20, 1 << 30);
        self.map_cache.max_chunks = (budget * 3 / 20 / (64 << 10)).clamp(256, 32768).min(crate::graphics::get().map_images as usize);
        if bytes > budget * 3 / 5 {
            let d = self.doc.dim;
            let max = self.doc.max_hot_tiles;
            self.doc.world.dims[d].trim(max);
        }
    }

    // --------------------------------------------------------- settings --

    fn prefs_header(ui: &mut Ui, title: &str, about: &str) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(title).size(15.0).medium());
        ui.add(egui::Label::new(RichText::new(about).size(12.0).color(pal.weak)).wrap());
        ui.add_space(12.0);
    }

    fn prefs_appearance(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        let pal = Palette::of(ui);
        Self::prefs_header(ui, tr!("Appearance"), tr!("How Massif looks: its language, its theme, and the colour of what is chosen, of the main buttons and of the home screen's art."));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Language"), |ui| {
                use crate::i18n::Language;
                let name = |l: Language| if l == Language::System { tr!("System ({language})", language = l.resolve().native_name()) } else { l.native_name().to_owned() };
                crate::widgets::combo("language").selected_text(name(self.settings.language)).width(170.0).show_ui(ui, |ui| {
                    for l in std::iter::once(Language::System).chain(Language::ALL).filter(|l| l.offered()) {
                        if ui.selectable_label(self.settings.language == l, name(l)).clicked() && self.settings.language != l {
                            self.settings.language = l;
                            crate::i18n::set_language(l);
                            crate::sounds::play(Cue::Select);
                        }
                    }
                });
            });
            crate::widgets::row(ui, tr!("Theme"), |ui| {
                let choices = [ThemeChoice::System, ThemeChoice::Light, ThemeChoice::Dark];
                let on = choices.map(|c| c == self.settings.theme);
                if let Some(k) = crate::widgets::button_group(ui, "theme", &[tr!("System"), tr!("Light"), tr!("Dark")], &on) {
                    self.settings.theme = choices[k];
                }
            });
            crate::widgets::row(ui, tr!("Accent"), |ui| {
                // Laid from the right: any colour last, the swatches before it.
                let mut chosen = None;
                let mut custom = self.settings.accent;
                if egui::color_picker::color_edit_button_srgb(ui, &mut custom).on_hover_text(tr!("Any colour")).changed() {
                    chosen = Some(custom);
                }
                ui.add_space(4.0);
                ui.spacing_mut().item_spacing.x = 3.0;
                for (name, rgb) in theme::ACCENTS.iter().rev() {
                    let (rect, swatch) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::click());
                    let on = self.settings.accent == *rgb;
                    ui.painter().circle_filled(rect.center(), 8.0, theme::rgb(*rgb));
                    if on || swatch.hovered() {
                        ui.painter().circle_stroke(rect.center(), 10.5, Stroke::new(1.5, if on { pal.text } else { pal.weak }));
                    }
                    swatch.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, *name));
                    if swatch.on_hover_text(*name).clicked() {
                        chosen = Some(*rgb);
                    }
                }
                if let Some(rgb) = chosen {
                    self.settings.accent = rgb;
                    theme::set_accent(ctx, rgb);
                    crate::sounds::play(Cue::Select);
                }
            });
            crate::widgets::row(ui, tr!("Loader"), |ui| {
                // Each pattern alive in its tile, laid from the right.
                ui.spacing_mut().item_spacing.x = 3.0;
                for p in crate::loader::Pattern::ALL.iter().rev() {
                    let (rect, tile) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::click());
                    let on = self.settings.loader == *p;
                    if on || tile.hovered() {
                        let fill = if on { theme::mix(pal.well(), pal.text, 0.08) } else { theme::mix(pal.well(), pal.text, 0.04) };
                        ui.painter().rect(rect, theme::TILE_RADIUS, fill, Stroke::new(1.0, if on { pal.accent } else { pal.border }), egui::StrokeKind::Inside);
                    }
                    crate::loader::paint(ui, rect.shrink(5.0), *p);
                    tile.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, crate::i18n::tr_name(p.label())));
                    if tile.on_hover_text(crate::i18n::tr_name(p.label())).clicked() {
                        self.settings.loader = *p;
                        crate::loader::set_pattern(ctx, *p);
                        crate::sounds::play(Cue::Select);
                    }
                }
            });
            crate::widgets::row(ui, tr!("Tool icons"), |ui| {
                let styles = crate::icons::IconStyle::ALL;
                let labels: Vec<&str> = styles.iter().map(|s| crate::i18n::tr_name(s.label())).collect();
                let on: Vec<bool> = styles.iter().map(|s| self.settings.tool_icons == *s).collect();
                if let Some(k) = crate::widgets::button_group(ui, "tool icons", &labels, &on) {
                    self.settings.tool_icons = styles[k];
                    crate::icons::set_style(ctx, styles[k]);
                    crate::sounds::play(Cue::Select);
                }
            });
            crate::widgets::row(ui, tr!("Animated home screen"), |ui| {
                let still = theme::reduce_motion(ui.ctx());
                let mut on = self.settings.animate_backdrop && !still;
                let tip = if still { tr!("Still: Reduce motion is on (Accessibility)") } else { tr!("The dither of its picture moves on about once a second") };
                if ui.add_enabled_ui(!still, |ui| crate::widgets::switch(ui, &mut on)).inner.on_hover_text(tip).changed() {
                    self.settings.animate_backdrop = on;
                }
            });
        });
        ui.add_space(14.0);
        ui.label(RichText::new(tr!("Crosshair")).size(13.0).medium());
        ui.add_space(4.0);
        self.prefs_reticle(ui, ctx);
        if crate::i18n::current() != crate::i18n::Language::English {
            ui.add_space(8.0);
            ui.add(egui::Label::new(RichText::new(tr!("Massif Help and the guide on the website are in English for now.")).size(11.5).color(pal.weak)).wrap());
        }
    }

    /// The crosshair of the player's view: its shape, size and opacity,
    /// and beside them how it looks on light ground and on dark.
    fn prefs_reticle(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        use crate::reticle::{SIZES, Shape};
        let pal = Palette::of(ui);
        let mut r = self.settings.reticle;
        // The preview's side.
        let side = 96.0;
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(ui.available_width() - side - 2.0 * ui.spacing().item_spacing.x - 8.0);
                crate::widgets::well(ui, |ui| {
                    crate::widgets::row(ui, tr!("Shape"), |ui| {
                        let labels: Vec<&str> = Shape::ALL.iter().map(|s| crate::i18n::tr_name(s.label())).collect();
                        let on: Vec<bool> = Shape::ALL.iter().map(|s| *s == r.shape).collect();
                        if let Some(k) = crate::widgets::button_group(ui, "reticle shape", &labels, &on) {
                            r.shape = Shape::ALL[k];
                            crate::sounds::play(Cue::Select);
                        }
                    });
                    let shown = r.shape != Shape::None;
                    crate::widgets::row(ui, tr!("Size"), |ui| {
                        ui.add_enabled(shown, crate::widgets::Field::new(&mut r.size, SIZES).suffix(" pt").max_decimals(0).width(150.0));
                    });
                    crate::widgets::row(ui, tr!("Opacity"), |ui| {
                        ui.add_enabled(
                            shown,
                            crate::widgets::Field::new(&mut r.opacity, 0.2..=1.0)
                                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
                                .width(150.0),
                        );
                    });
                });
            });
            ui.add_space(8.0);
            // The preview: a light half (snow) and a dark one (deep water),
            // the crosshair across both.
            let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
            let painter = ui.painter_at(rect);
            let k = theme::TILE_RADIUS;
            let (left, right) = (Rect::from_min_max(rect.min, pos2(rect.center().x, rect.max.y)), Rect::from_min_max(pos2(rect.center().x, rect.min.y), rect.max));
            painter.rect_filled(left, egui::CornerRadius { nw: k, sw: k, ne: 0, se: 0 }, theme::rgb([0xe4, 0xec, 0xf2]));
            painter.rect_filled(right, egui::CornerRadius { nw: 0, sw: 0, ne: k, se: k }, theme::rgb([0x1d, 0x3c, 0x6a]));
            painter.rect_stroke(rect, k, Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
            crate::reticle::paint(&painter, rect.center(), &r);
        });
        if r != self.settings.reticle {
            self.settings.reticle = r;
            crate::reticle::set(ctx, r);
        }
    }

    fn prefs_accessibility(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        Self::prefs_header(
            ui,
            tr!("Accessibility"),
            if crate::access::FOLLOWS_SYSTEM {
                tr!("Massif easier to see and calmer to use. System follows macOS's own settings, in System Settings ▸ Accessibility ▸ Display.")
            } else {
                tr!("Massif easier to see and calmer to use.")
            },
        );
        let system = self.access.system;
        // A choice that may follow the system: what the system says beside it.
        let follow = |ui: &mut Ui, id: &str, value: &mut Follow, system_on: bool| {
            let choices: &[Follow] = if crate::access::FOLLOWS_SYSTEM { &[Follow::System, Follow::Off, Follow::On] } else { &[Follow::Off, Follow::On] };
            let labels: &[&str] = if crate::access::FOLLOWS_SYSTEM { &[tr!("System"), tr!("Off"), tr!("On")] } else { &[tr!("Off"), tr!("On")] };
            let shown = if crate::access::FOLLOWS_SYSTEM || *value != Follow::System { *value } else { Follow::Off };
            let on: Vec<bool> = choices.iter().map(|c| *c == shown).collect();
            if let Some(k) = crate::widgets::button_group(ui, id, labels, &on) {
                *value = choices[k];
            }
            if crate::access::FOLLOWS_SYSTEM && *value == Follow::System {
                ui.label(RichText::new(if system_on { tr!("on in macOS") } else { tr!("off in macOS") }).size(11.5).color(pal.weak));
            }
        };
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("High contrast"), |ui| follow(ui, "high contrast", &mut self.settings.high_contrast, system.contrast));
            crate::widgets::row(ui, tr!("Reduce motion"), |ui| follow(ui, "reduce motion", &mut self.settings.reduce_motion, system.reduce_motion));
            crate::widgets::row(ui, tr!("Interface size"), |ui| {
                let labels = UI_SCALES.map(|s| format!("{:.0}%", s * 100.0));
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let on = UI_SCALES.map(|s| (s - self.settings.ui_scale).abs() < 0.01);
                if let Some(k) = crate::widgets::button_group(ui, "interface size", &labels, &on) {
                    self.settings.ui_scale = UI_SCALES[k];
                }
                if !on.contains(&true) {
                    ui.label(RichText::new(tr!("{value:.0}% now", value = self.settings.ui_scale * 100.0)).size(11.5).color(pal.weak));
                }
            });
        });
        ui.add_space(12.0);
        for (name, what) in [
            (tr!("High contrast"), tr!("Text black or white, muted text and lines drawn stronger, the accent darkened or lightened until it reads; the home screen's picture calmer behind its words.")),
            (tr!("Reduce motion"), tr!("The map and the 3D views stop where they are let go and go straight where they are sent; the home screen's picture and the tools' cards stay still.")),
            (tr!("Interface size"), tr!("Everything larger, text and controls alike. ⌘+ and ⌘− change it too, ⌘0 puts it back.")),
        ] {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(104.0, 16.0), egui::Layout::left_to_right(Align::Min), |ui| {
                    ui.set_width(104.0);
                    ui.label(RichText::new(name).size(12.0).medium());
                });
                ui.add(egui::Label::new(RichText::new(what).size(12.0).color(pal.weak)).wrap());
            });
            ui.add_space(6.0);
        }
    }

    fn prefs_typeface(&mut self, ui: &mut Ui, ctx: &egui::Context) {
        Self::prefs_header(ui, tr!("Typeface"), tr!("What the interface is set in: the system's own typeface, or one of those built into Massif. Each shows here in itself."));
        crate::widgets::well(ui, |ui| {
            for face in theme::Typeface::ALL {
                if typeface_row(ui, face, self.settings.typeface == face).clicked() && self.settings.typeface != face {
                    self.settings.typeface = face;
                    theme::set_typeface(ctx, face);
                    crate::sounds::play(Cue::Select);
                }
            }
        });
    }

    fn prefs_sounds(&mut self, ui: &mut Ui) {
        use crate::sounds::SoundLevel;
        let pal = Palette::of(ui);
        Self::prefs_header(ui, tr!("Sounds"), tr!("Little sounds for what happens, made by Massif as it plays them. Essential: what ends and what fails. All: the interface too — choices, switches, windows, workspaces."));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Play"), |ui| {
                let choices = [SoundLevel::Off, SoundLevel::Essential, SoundLevel::All];
                let on = choices.map(|c| c == self.settings.sounds);
                if let Some(k) = crate::widgets::button_group(ui, "sounds", &[tr!("Off"), tr!("Essential"), tr!("All")], &on) {
                    self.settings.sounds = choices[k];
                    crate::sounds::configure(self.settings.sounds, self.settings.volume);
                    crate::sounds::play(Cue::Toggle);
                }
            });
            crate::widgets::row(ui, tr!("Volume"), |ui| {
                let before = self.settings.volume;
                ui.add_enabled(
                    self.settings.sounds != SoundLevel::Off,
                    crate::widgets::Field::new(&mut self.settings.volume, 0.0..=1.0)
                        .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                        .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
                        .width(220.0),
                );
                if self.settings.volume != before {
                    crate::sounds::configure(self.settings.sounds, self.settings.volume);
                }
            });
        });
        ui.add_space(14.0);
        ui.label(RichText::new(tr!("Hear them")).size(13.0).medium());
        ui.add_space(4.0);
        crate::widgets::well(ui, |ui| {
            for cue in crate::sounds::Cue::ALL {
                crate::widgets::row(ui, crate::i18n::tr_name(cue.label()), |ui| {
                    if ui.button(theme::with_icon(Icon::Play, tr!("Play"))).clicked() {
                        crate::sounds::preview(cue);
                    }
                    let (tag, color) = if cue.essential() { (tr!("Essential"), pal.text) } else { (tr!("All"), pal.weak) };
                    ui.label(RichText::new(tag).size(11.5).color(color));
                });
            }
        });
    }

    fn prefs_notifications(&mut self, ui: &mut Ui) {
        use crate::notify::NotifyWhen;
        Self::prefs_header(ui, tr!("Notifications"), tr!("The system tells when a long task ends — an export, an import, a conversion, a pass over the whole map — so that you know while Massif is behind other windows. What Massif does shows at the bottom right of its window either way."));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Tell me"), |ui| {
                let choices = [NotifyWhen::Never, NotifyWhen::Background, NotifyWhen::Always];
                let on = choices.map(|c| c == self.settings.notify);
                if let Some(k) = crate::widgets::button_group(ui, "notify", &[tr!("Never"), tr!("When in the background"), tr!("Always")], &on) {
                    self.settings.notify = choices[k];
                }
            });
            crate::widgets::row(ui, tr!("Try it"), |ui| {
                if ui.button(theme::with_icon(Icon::Bell, tr!("Send a notification"))).clicked() {
                    crate::notify::send("Massif", tr!("Notifications work: you will hear from Massif when a long task ends."));
                    crate::sounds::preview(Cue::Ready);
                }
            });
        });
    }

    fn prefs_memory(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        Self::prefs_header(ui, tr!("Memory"), tr!("What Massif keeps decoded to be quick: the tiles of the map, the images of the map view. Past the limit, what was used least is put away again, compressed, and decoded when it is needed. The map itself stays in memory compressed."));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Limit"), |ui| {
                let choices = [0u32, 1024, 2048, 4096, 8192];
                let on = choices.map(|c| c == self.settings.memory_mb);
                if let Some(k) = crate::widgets::button_group(ui, "memory", &[tr!("Automatic"), tr!("1 GB"), tr!("2 GB"), tr!("4 GB"), tr!("8 GB")], &on) {
                    self.settings.memory_mb = choices[k];
                    self.memory_checked = f64::NEG_INFINITY;
                }
            });
        });
        ui.add_space(14.0);
        ui.label(RichText::new(tr!("In use")).size(13.0).medium());
        ui.add_space(4.0);
        let dim = self.doc.dimension();
        let stats: Vec<massif_core::level::LevelStats> = dim.levels.iter().map(|l| l.stats()).collect();
        let (hot, hot_bytes, cold_bytes, tiles) = stats.iter().fold((0, 0, 0, 0), |(h, hb, cb, t), s| (h + s.hot_tiles, hb + s.hot_bytes, cb + s.cold_bytes, t + s.tiles));
        let (chunks, grids, _) = self.map_cache.stats();
        let (columns, textures) = self.map_cache.bytes();
        let voxels = dim.voxels.len();
        let mb = |b: usize| tr!("{f64:.0} MB", f64 = b as f64 / (1u64 << 20) as f64);
        crate::widgets::well(ui, |ui| {
            let line = |ui: &mut Ui, name: &str, value: String, note: String| {
                crate::widgets::row(ui, name, |ui| {
                    ui.label(RichText::new(note).size(11.5).color(pal.weak));
                    ui.label(RichText::new(value).size(12.5));
                });
            };
            line(ui, tr!("Decoded tiles"), mb(hot_bytes), tr!("{hot} of {tiles} tiles, at most {max_hot_tiles} a level", max_hot_tiles = self.doc.max_hot_tiles, hot, tiles));
            line(ui, tr!("The map, compressed"), mb(cold_bytes), String::new());
            line(ui, tr!("Map view"), mb(columns), tr!("{grids} tiles of columns, {chunks} images of {mb} on the graphics card (up to {max_grids} and {max_chunks})", mb = mb(textures), max_grids = self.map_cache.max_grids, max_chunks = self.map_cache.max_chunks, grids, chunks));
            line(ui, tr!("Sculpted in 3D"), mb(voxels * 4096), tr!("{voxels} chunks of 16³", voxels));
        });
    }

    fn prefs_keyboard(&mut self, ui: &mut Ui) {
        Self::prefs_header(ui, tr!("Keyboard"), tr!("Every shortcut of Massif. Click one, then press the new keys; right-click it to remove it. Changes apply at once and are kept through updates."));
        crate::keys_page::page(ui, &mut self.settings.keys);
    }

    fn prefs_autosave(&mut self, ui: &mut Ui) {
        Self::prefs_header(
            ui,
            tr!("Autosave"),
            tr!("While the map has changes not saved, Massif keeps a copy of it, in a folder of its own: never over your file. Should Massif close without saving (a crash, a forced quit, a power cut), it offers to restore the copy the next time it starts. Saving the map, or closing it, removes its copy."),
        );
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Autosave"), |ui| {
                crate::widgets::switch(ui, &mut self.settings.autosave);
            });
            if self.settings.autosave {
                crate::widgets::row(ui, tr!("Every"), |ui| {
                    let labels = AUTOSAVE_MINUTES.map(|m| tr!("{m} min", m));
                    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                    let on = AUTOSAVE_MINUTES.map(|m| m == self.settings.autosave_minutes);
                    if let Some(k) = crate::widgets::button_group(ui, "autosave every", &labels, &on) {
                        self.settings.autosave_minutes = AUTOSAVE_MINUTES[k];
                    }
                });
            }
        });
        if let Some(dir) = crate::autosave::default_dir() {
            ui.add_space(10.0);
            let pal = Palette::of(ui);
            ui.add(egui::Label::new(RichText::new(tr!("Kept in {dir}, {max_backups} copies at most.", dir = dir.display(), max_backups = crate::autosave::MAX_BACKUPS)).size(11.5).color(pal.weak)).wrap());
        }
    }

    fn prefs_updates(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        Self::prefs_header(
            ui,
            tr!("Updates"),
            tr!("Once a day at most, Massif asks GitHub which version of Massif is the latest, and tells you when it is newer than yours. It sends nothing about you or your maps, and never downloads or installs anything itself: the release's page opens in your browser."),
        );
        let ctx = ui.ctx().clone();
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Check for new versions"), |ui| {
                if crate::widgets::switch(ui, &mut self.settings.check_updates).changed() && self.settings.check_updates {
                    self.check_updates(&ctx);
                }
            });
            if let Some(skipped) = self.settings.skipped_version.clone() {
                crate::widgets::row(ui, &tr!("Skipped {skipped}", skipped), |ui| {
                    if ui.button(tr!("Tell me again")).clicked() {
                        self.settings.skipped_version = None;
                    }
                });
            }
        });
        ui.add_space(10.0);
        let state = match (self.update_offer(), &self.updates.latest) {
            (Some(r), _) => tr!("Massif {trim_start_matches} is out. You have {running}.", trim_start_matches = r.tag_name.trim_start_matches('v'), running = crate::update::RUNNING),
            _ if self.updates.waiting() => tr!("Asking GitHub…").to_owned(),
            (None, Some(_)) => tr!("You have the latest version, {running}.", running = crate::update::RUNNING),
            (None, None) => tr!("You have version {running}.", running = crate::update::RUNNING),
        };
        ui.horizontal(|ui| {
            ui.label(RichText::new(state).size(11.5).color(pal.weak));
            if self.update_offer().is_some() && ui.small_button(tr!("See what is new")).clicked() {
                self.dialog = Some(Dialog::Update);
            }
        });
    }

    /// Start asking whether a newer version is out, if that is on (once a
    /// run; the answer is kept a day).
    fn check_updates(&mut self, ctx: &egui::Context) {
        if self.settings.check_updates
            && let Some(cache) = crate::update::default_cache()
        {
            self.updates.start(ctx, cache, crate::update::fetch_github);
        }
    }

    /// The newer version to tell of, if any.
    fn update_offer(&self) -> Option<&crate::update::Release> {
        if !self.settings.check_updates {
            return None;
        }
        self.updates.offer(crate::update::RUNNING, self.settings.skipped_version.as_deref())
    }

    fn prefs_graphics(&mut self, ui: &mut Ui) {
        use crate::graphics::{Graphics, Light, Preset};
        let pal = Palette::of(ui);
        Self::prefs_header(
            ui,
            tr!("Graphics"),
            tr!("How much the graphics card is asked to do in the 3D views and the map. Low suits integrated graphics and older cards, High a recent graphics card. Every change applies at once."),
        );
        let mut g = self.settings.graphics.unwrap_or_else(crate::graphics::get);
        let before = g;
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Preset"), |ui| {
                let now = g.preset();
                let all = [Preset::Low, Preset::Medium, Preset::High, Preset::Custom];
                let on = all.map(|p| p == now);
                if let Some(k) = crate::widgets::button_group(ui, "graphics preset", &[tr!("Low"), tr!("Medium"), tr!("High"), tr!("Custom")], &on)
                    && all[k] != Preset::Custom
                {
                    g = Graphics::of(all[k]);
                }
            });
        });
        if let Some((card, ms)) = &self.settings.graphics_chosen_for {
            ui.add_space(4.0);
            ui.add(egui::Label::new(RichText::new(tr!("Chosen at the first start for {card}, from a trial frame of {ms:.1} ms.", card, ms)).size(11.5).color(pal.weak)).wrap());
        }
        ui.add_space(12.0);
        ui.label(RichText::new(tr!("3D views")).size(13.0).medium());
        ui.add_space(4.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Render scale"), |ui| {
                let scales = [0.5f32, 0.67, 0.85, 1.0];
                let on = scales.map(|s| (s - g.render_scale).abs() < 0.01);
                if let Some(k) = crate::widgets::button_group(ui, "render scale", &["50%", "67%", "85%", "100%"], &on) {
                    g.render_scale = scales[k];
                }
            });
            crate::widgets::row(ui, tr!("Smooth edges"), |ui| {
                crate::widgets::switch(ui, &mut g.msaa);
            });
            crate::widgets::row(ui, tr!("Fog"), |ui| {
                crate::widgets::switch(ui, &mut g.fog);
            });
            crate::widgets::row(ui, tr!("Water reflections"), |ui| {
                crate::widgets::switch(ui, &mut g.reflections);
            });
            crate::widgets::row(ui, tr!("Game textures in the 3D views"), |ui| {
                crate::widgets::switch(ui, &mut g.textures);
            });
            crate::widgets::row(ui, tr!("Map kept on the graphics card"), |ui| {
                let pages = [64u32, 160, 400];
                let labels = pages.map(|p| tr!("{size} MB", size = (p as u64 * crate::graphics::PAGE_BYTES / 1_000_000)));
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let on = pages.map(|p| p == g.gpu_pages);
                if let Some(k) = crate::widgets::button_group(ui, "gpu pages", &labels, &on) {
                    g.gpu_pages = pages[k];
                }
            });
        });
        ui.add_space(12.0);
        ui.label(RichText::new(tr!("Terrain viewport")).size(13.0).medium());
        ui.add_space(4.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Light and shadows"), |ui| {
                let all = [Light::Low, Light::Medium, Light::High];
                let on = all.map(|l| l == g.light);
                if let Some(k) = crate::widgets::button_group(ui, "light", &[tr!("Low"), tr!("Medium"), tr!("High")], &on) {
                    g.light = all[k];
                }
            });
            crate::widgets::row(ui, tr!("Clouds"), |ui| {
                crate::widgets::switch(ui, &mut g.clouds);
            });
        });
        ui.add_space(12.0);
        ui.label(RichText::new(tr!("Map and window")).size(13.0).medium());
        ui.add_space(4.0);
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Map images on the graphics card"), |ui| {
                let counts = [2048u32, 8192, 32768];
                let labels = counts.map(|c| tr!("{size} MB", size = (c as u64 * crate::graphics::MAP_IMAGE_BYTES) >> 20));
                let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                let on = counts.map(|c| c == g.map_images);
                if let Some(k) = crate::widgets::button_group(ui, "map images", &labels, &on) {
                    g.map_images = counts[k];
                }
            });
            crate::widgets::row(ui, tr!("Frames a second when idle"), |ui| {
                let rates = [20u32, 30, 60, 0];
                let on = rates.map(|r| r == g.idle_fps);
                if let Some(k) = crate::widgets::button_group(ui, "idle fps", &["20", "30", "60", tr!("No limit")], &on) {
                    g.idle_fps = rates[k];
                }
            });
        });
        ui.add_space(12.0);
        for (name, what) in [
            (tr!("Render scale"), tr!("The 3D views drawn smaller and stretched to fit: what saves the most time in them. While a view moves it is drawn smaller still.")),
            (tr!("Map kept on the graphics card"), tr!("A map larger than this is drawn in 3D with a column every 2, 4 or 8 blocks: all of it, coarser, and much faster on a huge map.")),
            (tr!("Clouds"), tr!("What costs the most in the Terrain viewport. Saved images keep them.")),
            (tr!("Frames a second when idle"), tr!("While nothing is touched but something moves on screen (a loader, the 3D view building), the window draws at most this often.")),
        ] {
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(vec2(150.0, 16.0), egui::Layout::left_to_right(Align::Min), |ui| {
                    ui.set_width(150.0);
                    ui.add(egui::Label::new(RichText::new(name).size(12.0).medium()).wrap());
                });
                ui.add(egui::Label::new(RichText::new(what).size(12.0).color(pal.weak)).wrap());
            });
            ui.add_space(6.0);
        }
        if g != before {
            self.use_graphics(g);
        }
    }

    /// Draw with `g` from now on, and keep it as the user's choice.
    fn use_graphics(&mut self, g: crate::graphics::Graphics) {
        self.settings.graphics = Some(g);
        crate::graphics::set(g);
        // The map view's images: kept within the new limit.
        self.memory_checked = f64::NEG_INFINITY;
        self.slow_note = None;
        // A preset chosen by hand is the user's: no trial changes it.
        self.graphics_probe = None;
    }

    /// At the start: draw with the settings kept, or, the very first time,
    /// try the graphics card and pick a preset for it (see
    /// `graphics::suggest`).
    fn start_graphics(&mut self, cc: &eframe::CreationContext<'_>) {
        if let Some(g) = self.settings.graphics {
            let g = g.updated().sane();
            self.settings.graphics = Some(g);
            crate::graphics::set(g);
            return;
        }
        crate::graphics::set(crate::graphics::Graphics::default());
        // Tests draw with the default, the same in every run.
        if cfg!(test) {
            return;
        }
        let Some(render) = &cc.wgpu_render_state else { return };
        let card = crate::graphics::Card::of(&render.adapter.get_info());
        crate::startup::step(&format!("graphics card: {} ({:?}, {:?}): trying it to pick a graphics preset", card.name, card.device_type, card.backend));
        let (tx, rx) = std::sync::mpsc::channel();
        let (device, queue, ctx) = (render.device.clone(), render.queue.clone(), cc.egui_ctx.clone());
        let started = std::thread::Builder::new().name("graphics trial".into()).spawn(move || {
            let ms = crate::view3d::probe_ms(&device, &queue);
            let _ = tx.send(ms);
            ctx.request_repaint();
        });
        if started.is_ok() {
            self.graphics_probe = Some((card, rx));
        }
    }

    /// Each frame: the first start's trial, once it is done; frames of the
    /// 3D views watched; an idle window kept to its frames a second.
    fn watch_graphics(&mut self, ctx: &egui::Context) {
        if let Some((card, rx)) = &self.graphics_probe {
            match rx.try_recv() {
                Ok(ms) => {
                    let ms = (ms.is_finite() && ms > 0.0).then_some(ms);
                    let preset = crate::graphics::suggest(card, ms);
                    crate::startup::step(&format!("graphics trial: a frame in {:.1} ms on {}: the {preset:?} graphics preset", ms.unwrap_or(f32::NAN), card.name));
                    // Unless the user chose meanwhile.
                    if self.settings.graphics.is_none() {
                        let g = crate::graphics::Graphics::of(preset);
                        self.settings.graphics = Some(g);
                        self.settings.graphics_chosen_for = Some((card.name.clone(), ms.unwrap_or(0.0)));
                        crate::graphics::set(g);
                        self.memory_checked = f64::NEG_INFINITY;
                    }
                    self.graphics_probe = None;
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.graphics_probe = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
        let (now, touched, dt) = ctx.input(|i| (i.time, !i.events.is_empty() || i.pointer.any_down() || !i.keys_down.is_empty(), i.unstable_dt as f64));
        if touched {
            self.last_input = now;
        }
        // Slow frames of a 3D view being moved: a lighter preset suggested,
        // once a run, never past Low.
        if crate::graphics::take_drew_3d() && now - self.last_input < 0.5 {
            let preset = crate::graphics::get().preset();
            if preset != crate::graphics::Preset::Low && self.slow_frames.frame(dt) {
                self.slow_note = preset.lower();
                if let Some(lower) = self.slow_note {
                    log::info!("3D frames slow for a while: the {lower:?} graphics preset suggested");
                }
            }
        }
        // An idle window that keeps drawing (a loader, tiles being built)
        // draws at most `idle_fps` frames a second.
        let fps = crate::graphics::get().idle_fps;
        let began = self.frame_began.replace(std::time::Instant::now());
        let measuring = self.settings.developer && self.settings.draw_always;
        if !cfg!(test) && !measuring && fps > 0 && now - self.last_input > 1.0 && ctx.has_requested_repaint() {
            if let Some(began) = began {
                let least = std::time::Duration::from_secs_f64(1.0 / fps as f64);
                let spent = began.elapsed();
                if spent < least {
                    std::thread::sleep(least - spent);
                }
            }
            self.frame_began = Some(std::time::Instant::now());
        }
    }

    /// Settings ▸ Developer's frames a second, live: the window's images
    /// shown in step with the screen or not, the frames held to the number
    /// chosen, and drawn every frame if asked. Out of developer mode, the
    /// screen's pace as always.
    fn limit_frames(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let limit = if self.settings.developer { self.settings.frame_limit } else { crate::devtools::FrameLimit::Vsync };
        if let Some(config) = frame.wgpu_surface_config()
            && config.present_mode != limit.present_mode()
        {
            log::info!("frames: {:?} ({:?})", limit, limit.present_mode());
            frame.set_wgpu_surface_config(eframe::egui_wgpu::SurfaceConfig { present_mode: limit.present_mode(), ..config });
        }
        if let Some(fps) = limit.fps()
            && !cfg!(test)
        {
            crate::devtools::pace(&mut self.paced, fps);
        }
        if self.settings.developer && self.settings.draw_always {
            ctx.request_repaint();
        }
    }

    fn prefs_developer(&mut self, ui: &mut Ui) {
        Self::prefs_header(ui, tr!("Developer"), tr!("A panel of how Massif runs, live: frames a second and the time each takes, the memory the system counts, the processor's time, what Massif keeps decoded, the map, its tasks, the graphics card, and why it draws again. Offered in every area's menu, as Brushes or Info are."));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, tr!("Developer mode"), |ui| {
                crate::widgets::switch(ui, &mut self.settings.developer).on_hover_text(tr!("The Developer panel in the areas' menus"));
            });
            if self.settings.developer {
                crate::widgets::row(ui, tr!("The panel"), |ui| {
                    let shown = self.layout.current().root.shows(EditorKind::Developer);
                    if ui.add_enabled(!shown, egui::Button::new(theme::with_icon(Icon::Activity, if shown { tr!("Shown") } else { tr!("Show it here") }))).on_hover_text(tr!("Beside the largest area of this workspace")).clicked() {
                        self.show_developer_panel();
                    }
                });
                crate::widgets::row(ui, tr!("Frames a second"), |ui| {
                    use crate::devtools::FrameLimit;
                    let labels: Vec<String> = FrameLimit::ALL.iter().map(|l| l.label()).collect();
                    let labels: Vec<&str> = labels.iter().map(String::as_str).collect();
                    let on: Vec<bool> = FrameLimit::ALL.iter().map(|l| *l == self.settings.frame_limit).collect();
                    if let Some(i) = crate::widgets::button_group(ui, "frame limit", &labels, &on) {
                        self.settings.frame_limit = FrameLimit::ALL[i];
                    }
                });
                crate::widgets::row(ui, tr!("Draw every frame"), |ui| {
                    crate::widgets::switch(ui, &mut self.settings.draw_always).on_hover_text(tr!("Even when nothing changes: the charts of the Developer panel then show how many frames the computer makes"));
                });
            }
        });
        if self.settings.developer {
            ui.add_space(6.0);
            ui.label(
                RichText::new(tr!("Vsync draws in step with the screen. Without it, frames go out as soon as they are drawn (Immediate, or Mailbox where the graphics card has no Immediate; where it has neither, the screen still sets the pace), up to the number chosen or as many as the computer makes. Massif still draws only when something changes, unless it draws every frame."))
                    .size(11.5)
                    .color(Palette::of(ui).weak),
            );
        }
    }

    /// What the Developer panel shows of Massif itself.
    fn dev_info(&self) -> crate::devtools::DevInfo<'static> {
        let mb = |b: u64| tr!("{f64:.1} MB", f64 = b as f64 / (1u64 << 20) as f64);
        let dim = self.doc.dimension();
        let stats: Vec<massif_core::level::LevelStats> = dim.levels.iter().map(|l| l.stats()).collect();
        let (hot, hot_bytes, cold_bytes, tiles) = stats.iter().fold((0, 0, 0, 0), |(h, hb, cb, t), s| (h + s.hot_tiles, hb + s.hot_bytes, cb + s.cold_bytes, t + s.tiles));
        let (chunks, grids, _) = self.map_cache.stats();
        let (columns, textures) = self.map_cache.bytes();
        let voxels = dim.voxels.len();
        let (t3, g3) = (self.view3d.tiles_built() + self.sculpt3d.tiles_built(), self.view3d.gpu_bytes() + self.sculpt3d.gpu_bytes());
        let keeps = vec![
            (tr!("Decoded tiles"), mb(hot_bytes as u64), tr!("{hot} of {tiles}, at most {max_hot_tiles} a level", max_hot_tiles = self.doc.max_hot_tiles, hot, tiles)),
            (tr!("Map, compressed"), mb(cold_bytes as u64), String::new()),
            (tr!("Map view"), mb(columns as u64), tr!("{grids} tiles of columns, {chunks} images ({mb} on the graphics card){value}", mb = mb(textures as u64), value = if self.map_cache.busy() { tr!(", drawing") } else { "" }, grids, chunks)),
            (tr!("3D views"), mb(g3), tr!("{t3} tiles on the graphics card", t3)),
            (tr!("Sculpted in 3D"), mb(voxels as u64 * 4096), tr!("{voxels} chunks of 16³", voxels)),
            (tr!("Undo"), tr!("{undo_count} steps", undo_count = self.doc.history.undo_count()), String::new()),
        ];
        let map = match dim.bounds() {
            Some((lo, hi)) => {
                let t = massif_core::coords::TILE_SIZE;
                vec![
                    (tr!("Size"), tr!("{t} × {t2} blocks", t = (hi.x - lo.x + 1) * t, t2 = (hi.z - lo.z + 1) * t)),
                    (tr!("Height"), tr!("{min_y} to {max_y}", min_y = dim.min_y, max_y = dim.max_y)),
                    (tr!("Levels"), format!("{}", dim.levels.len())),
                    (tr!("Revision"), format!("{}", self.doc.revision)),
                ]
            }
            None => vec![(tr!("Size"), tr!("nothing yet").to_owned())],
        };
        crate::devtools::DevInfo { keeps, map, jobs: self.job_lines(), gpu: None }
    }

    /// The tasks running, a line each: name, then how far or how long.
    fn job_lines(&self) -> Vec<String> {
        self.jobs
            .iter()
            .map(|j| match &j.progress {
                Some(p) => format!("{} · {:.0} %", j.label, p.fraction() * 100.0),
                None => format!("{} · {:.0} s", j.label, j.started.elapsed().as_secs_f32()),
            })
            .collect()
    }

    /// The settings that change how Massif draws, for the problem report.
    fn report_settings(&self) -> Vec<(String, String)> {
        let s = &self.settings;
        let follow = |f: Follow| format!("{f:?}").to_lowercase();
        let mut v = vec![
            ("Theme".to_owned(), format!("{:?}", s.theme)),
            ("Typeface".to_owned(), s.typeface.name().to_owned()),
            ("Interface scale".to_owned(), format!("{}", s.ui_scale)),
            ("High contrast".to_owned(), format!("{} (now {})", follow(s.high_contrast), if s.high_contrast.on(self.access.system.contrast) { "on" } else { "off" })),
            ("Reduce motion".to_owned(), format!("{} (now {})", follow(s.reduce_motion), if s.reduce_motion.on(self.access.system.reduce_motion) { "on" } else { "off" })),
            ("Moving home picture".to_owned(), if s.animate_backdrop { "on" } else { "off" }.to_owned()),
            ("Memory for decoded tiles".to_owned(), if s.memory_mb == 0 { "as much as it takes".to_owned() } else { format!("{} MB", s.memory_mb) }),
            ("Autosave".to_owned(), if s.autosave { format!("every {} min", s.autosave_minutes) } else { "off".to_owned() }),
            ("Developer mode".to_owned(), if s.developer { "on" } else { "off" }.to_owned()),
            ("Frames a second".to_owned(), format!("{:?}{}", s.frame_limit, if s.draw_always { ", every frame" } else { "" })),
        ];
        let g = crate::graphics::get();
        v.push(("Graphics".to_owned(), format!("{:?}{} {g:?}", g.preset(), if s.graphics.is_none() { " (not chosen yet)" } else { "" })));
        if let Some((card, ms)) = &s.graphics_chosen_for {
            v.push(("Graphics chosen for".to_owned(), format!("{card}, trial {ms:.1} ms a frame")));
        }
        if let Some(asked) = std::env::var_os("WGPU_BACKEND") {
            v.push(("WGPU_BACKEND".to_owned(), asked.to_string_lossy().into_owned()));
        }
        v
    }

    /// Open the problem report's dialog: what it takes is read now (the
    /// last minute is the one before). `after_crash`: from the dialog of a
    /// start after a crash, gone back to after.
    fn open_report(&mut self, ctx: &egui::Context, after_crash: bool) {
        let window = ctx.input(|i| i.viewport().inner_rect.map(|r| ([r.width(), r.height()], i.pixels_per_point)));
        let info = self.dev_info();
        let keeps = info.keeps.iter().map(|(a, b, c)| ((*a).to_owned(), b.clone(), c.clone())).collect();
        let map = if self.has_map { info.map.iter().filter(|(k, _)| *k != "Revision").map(|(a, b)| ((*a).to_owned(), b.clone())).collect() } else { Vec::new() };
        let inputs = crate::report::Inputs {
            system: crate::report::System::read(self.gpu.clone(), window),
            settings: self.report_settings(),
            last_minute: self.dev.last_minute(keeps),
            map,
            log_dir: crate::startup::log_dir(),
            home: std::env::home_dir(),
            what_happened: String::new(),
        };
        let mut form = crate::report_view::ReportForm::new(inputs, self.has_map);
        form.after_crash = after_crash;
        self.dialog = Some(Dialog::Report(Box::new(form)));
    }

    /// Each frame: a recording's frame noted, its pill shown (Stop opens
    /// the report again), a report written in the background answered.
    fn report_frame(&mut self, ctx: &egui::Context) {
        let mut stopped = false;
        if self.recording.as_ref().is_some_and(|r| r.is_on()) {
            let jobs = self.job_lines();
            let last = self.dev.last_frame();
            let (memory, cpu) = self.dev.latest();
            if let Some(r) = &mut self.recording {
                if let Some((begun, gap, work)) = last {
                    r.frame(begun, gap, work, memory, cpu, jobs);
                }
                if crate::report_view::recording_pill(ctx, r) || !r.is_on() {
                    r.stop();
                    stopped = true;
                }
            }
        }
        if stopped {
            if let Some(form) = self.report_aside.take() {
                if self.dialog.is_none() {
                    self.dialog = Some(Dialog::Report(form));
                }
            } else if self.dialog.is_none() {
                self.open_report(ctx, false);
            }
        }
        if let Some(rx) = &self.report_saving {
            match rx.try_recv() {
                Ok(Ok(path)) => {
                    self.report_saving = None;
                    let name = jobs::display_name(&path);
                    self.say(ctx, tr!("Saved the report “{name}”: send it to us on Discord", name), false, Some(path));
                }
                Ok(Err(e)) => {
                    self.report_saving = None;
                    self.say(ctx, tr!("Could not save the report: {e}", e), true, None);
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => self.report_saving = None,
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
            }
        }
    }

    /// Write the report to `path` in the background, with the map if
    /// `form` asks for it; `report_frame` says when it is done.
    fn save_report(&mut self, ctx: &egui::Context, form: &crate::report_view::ReportForm, path: PathBuf) {
        let mut parts = crate::report::parts(&form.inputs, self.recording.as_ref(), None);
        let map = (form.include_map && self.has_map).then(|| self.doc.world.snapshot());
        let (tx, rx) = std::sync::mpsc::channel();
        self.report_saving = Some(rx);
        let ctx = ctx.clone();
        let _ = std::thread::Builder::new().name("massif report".into()).spawn(move || {
            let result = (|| {
                if let Some(world) = map {
                    let mut bytes = Vec::new();
                    massif_format::write(&world, &mut bytes).map_err(|e| e.to_string())?;
                    parts.push(crate::report::map_part(crate::report::MapFile { bytes }));
                }
                crate::report::save(&path, &parts).map_err(|e| e.to_string())?;
                Ok(path)
            })();
            let _ = tx.send(result);
            ctx.request_repaint();
        });
    }

    /// The dialog of a filter over the whole map, with a cold copy of the
    /// map for its pictures.
    fn open_reshape(&mut self, what: crate::reshape::WholeOp) {
        let map = self.doc.world.dims[self.doc.dim].snapshot();
        self.dialog = Some(Dialog::Reshape(Box::new(crate::reshape::ReshapeForm::new(what, map, self.doc.world.meta.sea_level))));
    }

    /// Put the Developer panel beside the largest area of the workspace.
    fn show_developer_panel(&mut self) {
        let mut areas = Vec::new();
        self.layout.current().root.areas(&mut areas);
        let largest = areas.iter().find(|(_, k)| matches!(k, EditorKind::Map | EditorKind::View3d | EditorKind::Sculpt3d | EditorKind::Graph | EditorKind::GraphView)).or(areas.first()).map(|(id, _)| *id);
        if let Some(id) = largest {
            let new = self.layout.split_at(id, crate::layout::Dir::Row, false, 0.64);
            self.layout.current_mut().root.set_editor(new, EditorKind::Developer);
        }
    }

    /// The survival check of the whole map, worked out on a cold copy of it.
    /// The one last closed comes back as it was while the map is the same.
    fn open_analysis(&mut self, ctx: &egui::Context) {
        let view = match self.analysis.take() {
            Some(view) if view.revision() == self.doc.revision => view,
            Some(mut view) => {
                let snapshot = self.doc.world.snapshot();
                view.again(ctx, snapshot, self.doc.dim, self.doc.revision);
                view
            }
            None => {
                let snapshot = self.doc.world.snapshot();
                Box::new(crate::analysis::AnalysisView::start(ctx, snapshot, self.doc.dim, self.doc.revision))
            }
        };
        self.dialog = Some(Dialog::Analysis(view));
    }

    /// The ores the export lays, over a cold copy of the map.
    /// The dialog of the caves, with the settings they were last carved
    /// with (their range kept inside this map's heights).
    /// What the Versions panel (or File ▸ Save Point…) asks for.
    fn versions_action(&mut self, ctx: &egui::Context, action: crate::versions::Action) {
        use crate::versions::Action;
        match action {
            Action::Make => {
                if self.doc.read_only.is_some() {
                    self.say(ctx, tr!("This save point is open read-only: Save As… makes it a map of its own"), true, None);
                    return;
                }
                let n = self.doc.world.points.len() + 1;
                self.dialog = Some(Dialog::PointName(None, tr!("Version {n}", n)));
            }
            Action::Restore(index) => {
                if self.doc.read_only.is_some() {
                    return;
                }
                if let Some(name) = self.doc.restore_point(index) {
                    self.map_cache.invalidate_all();
                    self.view3d.invalidate_all();
                    self.sculpt3d.invalidate_all();
                    self.say(ctx, tr!("Restored “{name}”: undo to take it back", name), false, None);
                }
            }
            Action::Open { index, read_only } => {
                if let Some(p) = self.doc.world.points.get(index) {
                    let pending = Pending::OpenPoint {
                        world: p.world.clone(),
                        name: p.name.clone(),
                        read_only,
                    };
                    self.request(ctx, pending);
                }
            }
            Action::Rename(index) => {
                if let Some(p) = self.doc.world.points.get(index) {
                    self.dialog = Some(Dialog::PointName(Some(index), p.name.clone()));
                }
            }
            Action::Delete(index) => {
                if index < self.doc.world.points.len() {
                    self.dialog = Some(Dialog::DeletePoint(index));
                }
            }
            Action::Compare(index) => self.versions.compare_with(&self.doc.world.points, index),
        }
    }

    fn open_caves(&mut self) {
        let dim = &self.doc.world.dims[self.doc.dim];
        let start = massif_core::caves::CaveSettings::for_dimension(dim, self.doc.world.meta.sea_level);
        let mut settings = self.caves.clone().unwrap_or_else(|| massif_core::caves::CaveSettings { seed: self.doc.world.meta.seed as u32 % 1_000_000, ..start.clone() });
        let (lo, hi) = (dim.min_y + 1, dim.max_y - 1);
        for y in [&mut settings.low, &mut settings.high, &mut settings.lake_level] {
            *y = (*y).clamp(lo, hi);
        }
        let side = dim.bounds().map_or(0, |(a, b)| (b.x - a.x + 1).max(b.z - a.z + 1) * massif_core::TILE_SIZE);
        let form = crate::caves::CaveForm::new(settings, (dim.min_y, dim.max_y), dim.selection.is_some(), side, dim.ground().len());
        self.dialog = Some(Dialog::Caves(Box::new(form)));
    }

    /// The dialog of the relief of a real place: for a new world, or into
    /// the map (its selection, else all of it).
    fn open_real(&mut self, new_world: bool) {
        use crate::real_terrain::{RealForm, Target};
        let (target, sea) = if new_world || !self.has_map {
            (Target::NewWorld, massif_core::generate::GenParams::default().sea_level)
        } else {
            let dim = &self.doc.world.dims[self.doc.dim];
            let tiles = |(lo, hi): (massif_core::TilePos, massif_core::TilePos)| massif_core::coords::BlockRect::new(lo.x * massif_core::TILE_SIZE, lo.z * massif_core::TILE_SIZE, (hi.x + 1) * massif_core::TILE_SIZE, (hi.z + 1) * massif_core::TILE_SIZE);
            let rect = dim.selection.as_ref().and_then(|s| s.bounds()).or_else(|| dim.bounds().map(tiles));
            let Some(rect) = rect else { return };
            (Target::Map { rect, selected: dim.selection.is_some(), min_y: dim.min_y, max_y: dim.max_y }, self.doc.world.meta.sea_level)
        };
        let form = RealForm::new(target, sea, self.real, crate::tiles::TileStore::global());
        self.dialog = Some(Dialog::Real(Box::new(form)));
    }

    fn open_ores(&mut self, ctx: &egui::Context) {
        let snapshot = self.doc.world.snapshot();
        self.dialog = Some(Dialog::Ores(Box::new(crate::ores::OresView::new(ctx, snapshot, self.doc.dim))));
    }

    /// Show column `(x, z)` on a map view of the World side: it glides
    /// there and points the column out.
    fn show_on_map(&mut self, ctx: &egui::Context, x: i32, z: i32) {
        self.layout.switch_side(false);
        let mut areas = Vec::new();
        self.layout.current().root.areas(&mut areas);
        let map = areas
            .iter()
            .filter(|(_, k)| *k == EditorKind::Map)
            .map(|(id, _)| *id)
            .find(|id| Some(*id) == self.active_map)
            .or_else(|| areas.iter().find(|(_, k)| *k == EditorKind::Map).map(|(id, _)| *id));
        match map {
            Some(id) => {
                self.maps.entry(id).or_default().show_spot(x as f64 + 0.5, z as f64 + 0.5);
                self.active_map = Some(id);
            }
            None => self.say(ctx, tr!("{x}, {z}: no map is shown in this workspace", x, z), false, None),
        }
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
                        .set_title(tr!("Open a world"))
                        .add_filter(tr!("Worlds"), &[massif_format::EXTENSION, "world"])
                        .add_filter(tr!("Massif world"), &[massif_format::EXTENSION])
                        .add_filter(tr!("WorldPainter world"), &["world"])
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
                    tr!("Made a world of {w:.0} × {d:.0} blocks from the height map in {as_secs_f32:.1} s", as_secs_f32 = started.elapsed().as_secs_f32(), w, d),
                    false,
                    None,
                );
            }
            Pending::Real(job) => {
                let rules = self.tools.auto.clone();
                self.jobs.push(Job::real(ctx, crate::tiles::TileStore::global(), *job, None, 0, rules, 0));
            }
            Pending::OpenPoint { world, name, read_only } => {
                let mut world = (*world).clone();
                world.points.clear();
                world.meta.name = tr!("{name2} — {name}", name2 = self.doc.world.meta.name, name);
                self.next_generation += 1;
                self.doc = Document::new(world, None, self.next_generation);
                if read_only {
                    self.doc.read_only = Some(name.clone());
                }
                self.rename = None;
                self.versions.compare = None;
                let how = if read_only { "read-only" } else { tr!("as a copy: save it to keep it") };
                self.say(ctx, tr!("Opened the save point “{name}” {how}", name, how), false, None);
            }
            Pending::Recover => {
                if !self.autosave.found.is_empty() {
                    let b = self.autosave.found.remove(0);
                    self.jobs.push(Job::recover(ctx, b));
                }
            }
            Pending::Quit => {
                if crate::quit::waiting() {
                    // At a logout, a restart or a shut-down, the system waits
                    // to end Massif itself: told yes, it does (see `quit`).
                    self.autosave.close();
                    crate::quit::reply(true);
                } else {
                    self.allow_close = true;
                    ctx.send_viewport_cmd(ViewportCommand::Close);
                }
            }
        }
    }

    /// Start saving. Returns false if the user cancelled the file dialog.
    fn save(&mut self, ctx: &egui::Context, save_as: bool) -> bool {
        if self.jobs.iter().any(|j| j.kind == jobs::JobKind::Save) {
            return true;
        }
        let save_as = save_as || self.doc.read_only.is_some();
        let path = match (&self.doc.path, save_as) {
            (Some(p), false) => Some(p.clone()),
            _ => rfd::FileDialog::new()
                .set_title(tr!("Save the world"))
                .add_filter(tr!("Massif world"), &[massif_format::EXTENSION])
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
        self.jobs.push(Job::save(ctx, snapshot, path, self.doc.revision).on(self.doc.generation));
        true
    }

    /// File ▸ Import Pack…: pack files to add to the packs.
    fn pick_pack(&mut self) {
        let files = rfd::FileDialog::new().set_title(tr!("Import packs")).add_filter(tr!("Massif pack"), &[massif_core::pack_file::EXTENSION]).pick_files().unwrap_or_default();
        self.pack_inbox.extend(files);
    }

    /// Import the pack files that came (one at a time while a dialog about
    /// a name taken is open), and do what the Packs panel asked.
    fn packs(&mut self, ctx: &egui::Context) {
        use crate::pack_share::{PackDialog, Request};
        let library = &mut self.layers_ui.library;
        match library.request.take() {
            Some(Request::Import) => self.pick_pack(),
            Some(Request::Remove(i)) => self.pack_dialog = Some(PackDialog::Remove(i)),
            Some(Request::Share(i)) => {
                if let Some(s) = library.packs.get(i)
                    && let Some(path) = rfd::FileDialog::new()
                        .set_title(tr!("Export a copy of the pack"))
                        .add_filter(tr!("Massif pack"), &[massif_core::pack_file::EXTENSION])
                        .set_file_name(format!("{}.{}", massif_core::pack_file::file_stem(&s.pack.name), massif_core::pack_file::EXTENSION))
                        .save_file()
                {
                    let name = s.pack.name.clone();
                    match massif_core::atomic::write(&path, &s.pack.to_bytes(), true) {
                        Ok(()) => self.say(ctx, tr!("Exported a copy of the pack “{name}”", name), false, Some(path)),
                        Err(e) => self.say(ctx, tr!("Could not export the pack. {why}", why = massif_core::atomic::describe(&e, &path)), true, None),
                    }
                }
            }
            None => {}
        }
        while self.pack_dialog.is_none() && !self.pack_inbox.is_empty() {
            let path = self.pack_inbox.remove(0);
            self.import_pack(ctx, &path);
        }
    }

    /// Read a pack file and add it to the packs, or ask about its name.
    fn import_pack(&mut self, ctx: &egui::Context, path: &std::path::Path) {
        let library = &mut self.layers_ui.library;
        library.ensure_loaded();
        match crate::pack_share::read_file(path) {
            Err(e) => self.say(ctx, tr!("Could not import {display_name}: {e}", display_name = jobs::display_name(path), e), true, None),
            Ok(pack) => match library.find(&pack.name) {
                Some(existing) => {
                    let rename = (2..).map(|k| format!("{} {k}", pack.name)).find(|n| library.find(n).is_none()).expect("a free name");
                    self.pack_dialog = Some(crate::pack_share::PackDialog::Clash { pack: Box::new(pack), existing, rename });
                }
                None => {
                    let outcome = crate::pack_share::added(library.add(pack.clone(), None), &pack);
                    self.pack_outcome(ctx, outcome);
                }
            },
        }
    }

    fn pack_outcome(&mut self, ctx: &egui::Context, outcome: crate::pack_share::Outcome) {
        match outcome {
            crate::pack_share::Outcome::Say(text, error, reveal) => self.say(ctx, text, error, reveal),
            crate::pack_share::Outcome::ShowPacks(text) => {
                self.layers_ui.packs = true;
                self.say(ctx, text, false, None);
            }
        }
    }

    fn open_export_dialog(&mut self) {
        let parent = massif_anvil::default_saves_dir()
            .or_else(|| self.doc.path.as_ref().and_then(|p| p.parent().map(PathBuf::from)))
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        let centre = self.map_center.map_or((0, 0), |c| (c[0].floor() as i32, c[1].floor() as i32));
        // For the version the map's mods are made for (Conquest Reforged's
        // 1.20.1), else for the game installed here, the newest; without
        // one, 1.18.2, which every later version opens.
        let versions = export_versions();
        let mods = mod_uses(&self.doc.world.registry, self.blocks.catalogue.as_deref());
        let target = mods
            .iter()
            .filter_map(|m| m.minecraft.as_deref())
            .find_map(|v| versions.iter().find(|(t, _)| t.name == v))
            .or_else(|| versions.iter().find(|(_, installed)| *installed))
            .map_or_else(massif_anvil::Target::default, |(t, _)| t.clone());
        // Into the saves of the modpack the map's mods come from.
        let parent = self
            .blocks
            .catalogue
            .as_ref()
            .and_then(|c| mods.iter().filter(|m| m.loaded).find_map(|m| c.mods.iter().find(|i| i.name == m.name && i.kind == massif_assets::SourceKind::Mod)))
            .and_then(|m| m.path.parent().filter(|d| d.file_name().is_some_and(|n| n == "mods")).and_then(|d| d.parent()).map(|d| d.join("saves")))
            .filter(|d| d.is_dir())
            .unwrap_or(parent);
        let unique = massif_anvil::unique_dir(&parent, &self.doc.world.meta.name);
        let name = unique.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| self.doc.world.meta.name.clone());
        self.dialog = Some(Dialog::Export(ExportForm {
            target,
            versions,
            heights: (self.doc.dimension().min_y, self.doc.dimension().max_y),
            mods,
            tab: ExportTab::General,
            parent,
            name,
            game_mode: GameMode::Creative,
            difficulty: massif_anvil::Difficulty::Normal,
            hardcore: false,
            allow_cheats: true,
            surroundings: Surroundings::Vanilla,
            ores: self.doc.world.meta.ores.on,
            structures: true,
            has_selection: self.doc.selection().is_some(),
            selection_only: false,
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
            self.say(ctx, tr!("Undid a change to the graph"), false, None);
            return;
        }
        if let Some(label) = self.doc.undo() {
            self.say(ctx, tr!("Undid {label}", label), false, None);
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
            self.say(ctx, tr!("Redid a change to the graph"), false, None);
            return;
        }
        if let Some(label) = self.doc.redo() {
            self.say(ctx, tr!("Redid {label}", label), false, None);
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
            // Worked out on a map that is no longer open: none of it goes
            // into this one.
            let other_map = job.map.is_some_and(|g| g != self.doc.generation);
            let meanwhile = if other_map { tr!("another map was opened meanwhile") } else { tr!("the map changed meanwhile") };
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
                            self.sample = None;
                            self.remember(p);
                            self.say(ctx, tr!("Opened “{name}” in {secs:.1} s", name, secs), false, None);
                            self.task_done(ctx, Cue::Ready, tr!("Map opened"), &name, secs);
                        }
                        None => {
                            self.say(ctx, tr!("Created “{name}” in {secs:.1} s", name, secs), false, None);
                            self.task_done(ctx, Cue::Ready, tr!("Map created"), &name, secs);
                            // Its relief from a template: on the Terrain side.
                            if let Some(t) = self.after_create.take() {
                                self.start_terrain(t);
                                // A sample: its relief built at once.
                                if self.sample == Some(SampleStage::Making)
                                    && let Some(graph) = self.doc.world.meta.graph.clone()
                                {
                                    self.graph.start_build(ctx, &mut self.doc, &graph, None);
                                    self.sample = Some(SampleStage::Building);
                                } else {
                                    self.sample = None;
                                }
                            }
                        }
                    }
                }
                Outcome::SavePoint(point) => {
                    if other_map {
                        self.say(ctx, tr!("The save point “{name}” was dropped: {meanwhile}", name = point.name, meanwhile), true, None);
                    } else {
                        let name = point.name.clone();
                        self.doc.world.points.push(point);
                        self.doc.mark_changed();
                        let msg = match &self.doc.path {
                            Some(_) => tr!("Saved the point “{name}”: it is kept in the file at the next save", name),
                            None => tr!("Saved the point “{name}”: save the map to keep it", name),
                        };
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::Recovered { world, backup } => {
                    self.next_generation += 1;
                    let name = world.meta.name.clone();
                    self.doc = Document::new(world, backup.info.path.clone(), self.next_generation);
                    // Not what its file holds: changes to save.
                    self.doc.mark_changed();
                    self.rename = None;
                    self.home = None;
                    self.has_map = true;
                    self.autosave.adopt(backup, self.doc.generation, self.doc.revision);
                    self.say(ctx, tr!("Restored “{name}”: save it to keep it", name), false, None);
                    self.task_done(ctx, Cue::Ready, tr!("Map restored"), &name, secs);
                }
                Outcome::Saved { path, revision, bytes } => {
                    if other_map {
                        // Not this map's file, nor what to do after saving it.
                        self.after_save = None;
                    } else {
                        self.doc.mark_saved(revision, path.clone());
                        self.doc.read_only = None;
                        self.autosave.saved(self.doc.generation, revision);
                    }
                    self.remember(path.clone());
                    let msg = tr!("Saved {display_name} ({human}) in {secs:.1} s", display_name = jobs::display_name(&path), human = panels::human(bytes), secs);
                    self.task_done(ctx, Cue::Saved, tr!("Map saved"), &msg, secs);
                    self.say(ctx, msg, false, Some(path));
                    if let Some(pending) = self.after_save.take() {
                        self.run(ctx, pending);
                    }
                }
                Outcome::Exported { dir, stats } => {
                    let msg = tr!("Exported {chunks} chunks in {regions} region files ({human}) in {as_secs_f32:.1} s", chunks = stats.chunks, regions = stats.regions, human = panels::human(stats.bytes), as_secs_f32 = stats.elapsed.as_secs_f32());
                    self.task_done(ctx, Cue::Ready, tr!("Export done"), &msg, secs);
                    self.say(ctx, msg, false, Some(dir));
                }
                Outcome::Failed(message) => {
                    self.after_save = None;
                    self.task_done(ctx, Cue::Error, tr!("Something failed"), &message, secs);
                    self.say(ctx, message, true, None);
                }
                Outcome::Imported { world, report, source } => {
                    self.next_generation += 1;
                    self.doc = Document::new(world, None, self.next_generation);
                    self.rename = None;
                    self.home = None;
                    self.has_map = true;
                    self.remember(source.clone());
                    let msg = tr!("Imported {display_name} in {secs:.1} s: {summary}", display_name = jobs::display_name(&source), summary = report.summary(), secs);
                    self.task_done(ctx, Cue::Ready, tr!("Import done"), &msg, secs);
                    self.say(ctx, msg, false, None);
                    let thumbnail = report.thumbnail.as_deref().and_then(|jpeg| {
                        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.to_rgba8();
                        let size = [img.width() as usize, img.height() as usize];
                        let color = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
                        Some(ctx.load_texture("import thumbnail", color, egui::TextureOptions::LINEAR))
                    });
                    self.dialog = Some(Dialog::ImportReport(report, thumbnail));
                }
                Outcome::Auto { biomes, tiles, columns, revision, dim } => {
                    let label = if biomes { tr!("Auto biome") } else { tr!("Auto paint") };
                    if other_map || revision != self.doc.revision || dim != self.doc.dim {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("{label}: {meanwhile}, nothing was done", label, meanwhile), true, None);
                    } else {
                        self.doc.history.begin(label, dim);
                        self.doc.history.edit(&mut self.doc.world, 0).replace_tiles(tiles);
                        self.doc.commit();
                        self.tools.auto_done = Some((biomes, columns));
                        let msg = tr!("{label} over the whole map: {columns} columns changed in {secs:.1} s", label, columns, secs);
                        self.task_done(ctx, Cue::Ready, &tr!("{label} done", label), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::Pattern { tiles, columns, revision, dim, level } => {
                    let label = tr!("Noise paint");
                    if other_map || revision != self.doc.revision || dim != self.doc.dim || level >= self.doc.dimension().levels.len() {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("{label}: {meanwhile}, nothing was done", label, meanwhile), true, None);
                    } else {
                        self.doc.history.begin(label, dim);
                        self.doc.history.edit(&mut self.doc.world, level).replace_tiles(tiles);
                        self.doc.commit();
                        self.tools.pattern.done = Some(columns);
                        let msg = tr!("{label} over the whole map: {columns} columns changed in {secs:.1} s", label, columns, secs);
                        self.task_done(ctx, Cue::Ready, &tr!("{label} done", label), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::Reshaped { label, tiles, columns, revision, dim } => {
                    if other_map || revision != self.doc.revision || dim != self.doc.dim {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("{label}: {meanwhile}, nothing was done", label, meanwhile), true, None);
                    } else {
                        self.doc.history.begin(label, dim);
                        self.doc.history.edit(&mut self.doc.world, 0).replace_tiles(tiles);
                        self.doc.commit();
                        let msg = tr!("{label} over the whole map: {columns} columns changed in {secs:.1} s", label, columns, secs);
                        self.task_done(ctx, Cue::Ready, &tr!("{label} done", label), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::RealTerrain { tiles, columns, revision, dim } => {
                    let label = tr!("Real terrain");
                    if other_map || revision != self.doc.revision || dim != self.doc.dim {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("Relief of a real place: {meanwhile}, nothing was done", meanwhile), true, None);
                    } else {
                        self.doc.history.begin(label, dim);
                        self.doc.history.edit(&mut self.doc.world, 0).replace_tiles(tiles);
                        self.doc.commit();
                        let msg = tr!("Relief of a real place: {columns} columns changed in {secs:.1} s", columns, secs);
                        self.task_done(ctx, Cue::Ready, tr!("Relief of a real place done"), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::RealMap { dimension, columns, revision, dim } => {
                    if other_map || revision != self.doc.revision || dim != self.doc.dim {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("Relief of a real place: {meanwhile}, nothing was done", meanwhile), true, None);
                    } else {
                        let rect = dimension.bounds().map(|(lo, hi)| ((hi.x - lo.x + 1) * massif_core::TILE_SIZE, (hi.z - lo.z + 1) * massif_core::TILE_SIZE));
                        self.doc.replace_dimension(dim, *dimension, tr!("Real terrain"));
                        self.map_cache.invalidate_all();
                        self.view3d.invalidate_all();
                        self.sculpt3d.invalidate_all();
                        let (w, d) = rect.unwrap_or_default();
                        let msg = tr!("Relief of a real place: the map is now {w} × {d} blocks, {columns} columns written in {secs:.1} s", w, d, columns, secs);
                        self.task_done(ctx, Cue::Ready, tr!("Relief of a real place done"), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::Caves { chunks, blocks, flooded, revision, dim } => {
                    let label = tr!("Caves");
                    if other_map || revision != self.doc.revision || dim != self.doc.dim {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("{label}: {meanwhile}, nothing was done", label, meanwhile), true, None);
                    } else if chunks.is_empty() {
                        crate::sounds::play(Cue::Warning);
                        self.say(ctx, tr!("Caves: nothing to carve there (the range lies above the ground, or the selection is empty)"), true, None);
                    } else {
                        self.doc.history.begin(label, dim);
                        self.doc.history.edit(&mut self.doc.world, 0).set_voxels(chunks);
                        self.doc.commit();
                        let lakes = if flooded > 0 { tr!(", {flooded} of fluid", flooded) } else { String::new() };
                        let msg = tr!("Caves carved: {blocks} blocks{lakes}, in {secs:.1} s", blocks, lakes, secs);
                        self.task_done(ctx, Cue::Ready, tr!("Caves carved"), &msg, secs);
                        self.say(ctx, msg, false, None);
                    }
                }
                Outcome::Cancelled => {
                    crate::sounds::play(Cue::Warning);
                    self.say(ctx, tr!("Cancelled"), false, None);
                }
            }
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    // ---------------------------------------------------------- shortcuts --

    fn shortcuts(&mut self, ctx: &egui::Context) {
        use crate::keymap::{self, Action};
        let take = |action| keymap::take(ctx, action);
        let pressed = |action| take(action).is_some();
        // The size of the interface, as a browser's, whatever is open: the
        // keymap's keys rather than egui's own.
        if ctx.options(|o| o.zoom_with_keyboard) {
            ctx.options_mut(|o| o.zoom_with_keyboard = false);
        }
        if pressed(Action::UiZoomReset) {
            ctx.set_zoom_factor(1.0);
        }
        if pressed(Action::UiZoomIn) {
            egui::gui_zoom::zoom_in(ctx);
        }
        if pressed(Action::UiZoomOut) {
            egui::gui_zoom::zoom_out(ctx);
        }
        if self.dialog.is_some() || self.blocking_job().is_some() {
            return;
        }
        // The keys the menus show beside their items.
        for (action, command) in [(Action::Settings, Command::Settings), (Action::Help, Command::Help)] {
            if pressed(action) {
                self.command(ctx, command);
                return;
            }
        }
        if self.home.is_some() {
            // No map on the home screen: only the ways in.
            for (action, command) in [(Action::Open, Command::Open), (Action::NewWorld, Command::NewWorld)] {
                if pressed(action) {
                    self.command(ctx, command);
                }
            }
            return;
        }
        if pressed(Action::Redo) {
            self.redo(ctx);
        } else if pressed(Action::Undo) {
            self.undo(ctx);
        }
        if pressed(Action::SaveAs) {
            self.save(ctx, true);
        } else if pressed(Action::Save) {
            self.save(ctx, false);
        }
        for (action, command) in [(Action::Open, Command::Open), (Action::NewWorld, Command::NewWorld), (Action::Export, Command::Export)] {
            if pressed(action) {
                self.command(ctx, command);
            }
        }
        if ctx.egui_wants_keyboard_input() {
            return;
        }
        // Select all, deselect and invert, but in the graph, which picks
        // its nodes with ⌘A and ⌘D.
        let over_graph = ctx.input(|i| i.pointer.hover_pos()).and_then(|p| self.area_at(ctx, p)).and_then(|id| self.layout.current().root.editor_of(id)).is_some_and(|k| matches!(k, EditorKind::Graph | EditorKind::GraphView | EditorKind::GraphProps));
        if !over_graph {
            for (action, command) in [(Action::InvertSelection, Command::InvertSelection), (Action::SelectAll, Command::SelectAll), (Action::Deselect, Command::Deselect)] {
                if pressed(action) {
                    self.command(ctx, command);
                }
            }
        }
        if pressed(Action::MaximizeArea) {
            let pointer = ctx.input(|i| i.pointer.hover_pos());
            if let Some(p) = pointer
                && let Some(id) = self.area_at(ctx, p)
            {
                self.layout.toggle_maximize(id);
            }
        }
        // The map's zoom (WorldPainter's ⌘ with +, − and 0).
        for (action, factor) in [(Action::MapZoomIn, 2.0), (Action::MapZoomOut, 0.5), (Action::MapZoomReset, 0.0)] {
            if pressed(action) {
                let id = self.active_map.or_else(|| self.maps.keys().next().copied());
                if let Some(view) = id.and_then(|id| self.maps.get_mut(&id)) {
                    if factor == 0.0 {
                        view.zoom_reset();
                    } else {
                        view.zoom_smoothly(factor);
                    }
                }
            }
        }
        // WorldPainter's keys: Alt and a letter the tools, 1…0 the brush's
        // intensity, − and + its size, [ and ] (or Alt with − and +) its
        // rotation, R and L the light of the map.
        for tool in keymap::tools() {
            if pressed(Action::Tool(tool)) {
                self.tools.select(tool);
            }
        }
        let brush = &mut self.tools.brush;
        for n in 1..=10u8 {
            if pressed(Action::Intensity(n)) {
                brush.strength = n as f32 / 10.0;
            }
        }
        if let Some(press) = take(Action::BrushSmaller) {
            let r = brush.radius;
            brush.radius = if press.shift { r - 1.0 } else { (r / 1.1).min(r - 1.0) }.round().max(1.0);
        }
        if pressed(Action::BrushLarger) {
            let r = brush.radius;
            brush.radius = (r * 1.1).max(r + 1.0).round().min(massif_core::MAX_RADIUS);
        }
        for (action, degrees) in [(Action::TurnLeft, -15.0), (Action::TurnRight, 15.0), (Action::Turn30, 30.0), (Action::Turn45, 45.0), (Action::Turn90, 90.0)] {
            if pressed(action) {
                brush.rotate(degrees);
            }
        }
        if pressed(Action::TurnReset) {
            brush.rotation = 0.0;
        }
        if pressed(Action::LightRight) {
            self.settings.light = (self.settings.light + 1) % 16;
        }
        if pressed(Action::LightLeft) {
            self.settings.light = (self.settings.light + 15) % 16;
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
            // Room for the window's buttons (close, minimise, zoom), and a
            // gap after them.
            ui.add_space(80.0);
        }
        // egui's menu bar pads its buttons by 2 points and no more: a hit
        // as narrow as the word. These are as roomy as a system menu bar's,
        // side by side with no gap between them.
        let roomy = |style: &mut egui::Style| {
            egui::containers::menu::menu_style(style);
            style.spacing.button_padding = vec2(10.0, 5.0);
            style.spacing.item_spacing.x = 0.0;
        };
        // The menus, but for the system's menu bar showing them (macOS): its
        // buttons are all the bar keeps then.
        let menus = if self.menus_in_menu_bar { Vec::new() } else { self.menus() };
        let mut picked = None;
        egui::MenuBar::new().style(roomy).ui(ui, |ui| {
            for menu in &menus {
                ui.menu_button(menu.title, |ui| {
                    if let Some(command) = crate::menu::ui(ui, &menu.entries) {
                        picked = Some(command);
                    }
                });
            }
            if !menus.is_empty() {
                ui.separator();
            }
            let busy = self.blocking_job().is_some();
            let icon = |ui: &mut Ui, icon: Icon, tip: &str, enabled: bool| {
                let clicked = bar_button(ui, tip, enabled, false, |p, r, c| glyph_in(p, r, icon, c)).clicked();
                if clicked {
                    crate::sounds::play(Cue::Tap);
                }
                clicked
            };
            if icon(ui, Icon::FilePlus2, tr!("New world (⌘N)"), !busy) {
                self.open_home(Page::NewMap);
            }
            if icon(ui, Icon::FolderOpen, tr!("Open (⌘O)"), !busy) {
                self.request(&ctx, Pending::Open(None));
            }
            let save_from = ui.cursor().min.x;
            if icon(ui, Icon::Save, tr!("Save (⌘S)"), !busy) {
                self.save(&ctx, false);
            }
            if icon(ui, Icon::Package, tr!("Export to Minecraft (⌘E)"), !busy) {
                self.open_export_dialog();
            }
            let commands = Rect::from_x_y_ranges(save_from..=ui.cursor().min.x - ui.spacing().item_spacing.x, ui.max_rect().y_range());
            crate::tour::mark(&ctx, crate::tour::Target::SaveExport, commands);
            ui.separator();
            if icon(ui, Icon::Undo2, tr!("Undo (⌘Z)"), self.doc.history.can_undo()) {
                self.undo(&ctx);
            }
            if icon(ui, Icon::Redo2, tr!("Redo (⇧⌘Z)"), self.doc.history.can_redo()) {
                self.redo(&ctx);
            }
            ui.separator();
            for tool in [Tool::Raise, Tool::Smooth, Tool::Flatten, Tool::PaintTerrain, Tool::Flood, Tool::StampSlab] {
                let selected = self.tools.tool == tool;
                let r = bar_button(ui, crate::i18n::tr_name(tool.label()), true, selected, |p, rect, c| crate::icons::paint(p, rect, tool, c));
                r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, crate::i18n::tr_name(tool.label())));
                if r.clicked() {
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
                if bar_button(ui, tr!("Theme"), true, false, |p, r, c| glyph_in(p, r, icon, c)).clicked() {
                    self.settings.theme = next;
                    crate::sounds::play(Cue::Toggle);
                }
                if let Some(version) = self.update_offer().map(|r| r.tag_name.trim_start_matches('v').to_owned())
                    && update_pill(ui, &version).clicked()
                {
                    self.dialog = Some(Dialog::Update);
                }
            });
        });
        if let Some(command) = picked {
            self.command(&ctx, command);
        }
    }

    /// What the menus depend on, now.
    fn menu_state(&self) -> crate::menu::State<'_> {
        crate::menu::State {
            editor: self.home.is_none(),
            // Every dialog is modal, as is the wait for a map being opened
            // or made: the window's menus cannot be reached then.
            idle: self.dialog.is_none() && self.blocking_job().is_none(),
            undo: self.doc.history.undo_label(),
            redo: self.doc.history.redo_label(),
            recent: &self.settings.recent,
            theme: self.settings.theme,
            selected: self.doc.selection().is_some(),
        }
    }

    /// The menus of the top bar (and of macOS's menu bar), as they are now.
    fn menus(&self) -> Vec<crate::menu::Menu> {
        crate::menu::menus(&self.menu_state())
    }

    /// Do what an item of the menus does, wherever it was picked: in the
    /// window's menus, or in macOS's menu bar.
    fn command(&mut self, ctx: &egui::Context, command: Command) {
        match command {
            Command::NewWorld => self.open_home(Page::NewMap),
            // On the home screen, as its own Open… does.
            Command::Open if self.home.is_some() => self.home_action(ctx, HomeAction::Open),
            Command::Open => self.request(ctx, Pending::Open(None)),
            Command::ImportWorld => self.home_action(ctx, HomeAction::Import),
            Command::ImportPack => self.pick_pack(),
            Command::ExportPack => {
                let name = self.doc.world.meta.name.clone();
                self.pack_dialog = Some(crate::pack_share::PackDialog::Export(Box::new(crate::pack_share::ExportForm::new(&self.doc.world.registry, &self.settings.pack_author, &name).with_gradients(&self.doc.world.meta.gradients))));
            }
            Command::FromHeightMap => self.open_home(Page::FromHeightmap),
            Command::OpenRecent(path) => self.request(ctx, Pending::Open(Some(path))),
            Command::Save => {
                self.save(ctx, false);
            }
            Command::SaveAs => {
                self.save(ctx, true);
            }
            Command::ExportHeightMap => self.export_heightmap(ctx),
            Command::Export => self.open_export_dialog(),
            // Asked first if the map has changes (see `handle_close`).
            Command::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Command::Undo => self.undo(ctx),
            Command::Redo => self.redo(ctx),
            Command::SelectAll => {
                crate::select::select_all(&mut self.doc);
            }
            Command::Deselect => {
                crate::select::deselect(&mut self.doc);
            }
            Command::InvertSelection => {
                crate::select::invert(&mut self.doc);
            }
            Command::AutoBiome => self.tools.run_auto = Some(true),
            Command::AutoPaint => self.tools.run_auto = Some(false),
            Command::AutoRules => self.dialog = Some(Dialog::AutoRules(false)),
            Command::WholeMap(what) => self.open_reshape(what),
            Command::Caves => self.open_caves(),
            Command::RealTerrain => self.open_real(false),
            Command::FromRealPlace => self.open_real(true),
            Command::SavePoint => self.versions_action(ctx, crate::versions::Action::Make),
            Command::Ores => self.open_ores(ctx),
            Command::Analyse => self.open_analysis(ctx),
            Command::Theme(choice) => self.settings.theme = choice,
            Command::Settings => self.dialog = Some(Dialog::Preferences(PrefsSection::Appearance)),
            Command::ResetWorkspaces => self.layout = Layout::default(),
            Command::Help => self.dialog = Some(Dialog::Help(crate::help::HelpState::default())),
            Command::Tour => self.take_the_tour(ctx),
            Command::Shortcuts => {
                self.dialog = Some(Dialog::Help(crate::help::HelpState {
                    topic: crate::help::topic_index("Shortcuts"),
                    query: String::new(),
                }));
            }
            Command::HomeScreen => self.open_home(Page::NewMap),
            Command::About => self.dialog = Some(Dialog::About),
            Command::ReportProblem => self.open_report(ctx, false),
        }
    }

    /// What was picked in macOS's menu bar since the last frame, if it can
    /// still be done.
    fn menu_bar_commands(&mut self, ctx: &egui::Context) {
        let picked = crate::menubar::take();
        if picked.is_empty() {
            return;
        }
        for command in picked {
            if crate::menu::enabled(&self.menus(), &command) {
                self.command(ctx, command);
            }
        }
        ctx.request_repaint();
    }

    fn workspace_tabs(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.horizontal(|ui| {
            // The two sides of Massif: the map editor (WorldPainter's) and
            // the terrain generator (Gaea's).
            let terrain = self.layout.terrain();
            let sides = egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(theme::PILL)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for (side, icon, label, hint) in [
                            (false, Icon::Map, tr!("World"), tr!("Edit the map: sculpt, paint, levels, 3D, export")),
                            (true, Icon::MountainSnow, tr!("Terrain"), tr!("Generate terrain from nodes, render it, convert it to the map")),
                        ] {
                            let active = terrain == side;
                            let text = RichText::new(theme::with_icon(icon, label)).color(if active { pal.text } else { pal.weak });
                            let button = egui::Button::new(if active { text.medium() } else { text })
                                .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE)
                                .corner_radius(theme::PILL)
                                .min_size(vec2(0.0, 24.0));
                            if ui.add(button).on_hover_text(hint).clicked() && terrain != side {
                                self.layout.switch_side(side);
                                crate::sounds::play(Cue::Navigate);
                            }
                        }
                    });
                });
            crate::tour::mark(ui.ctx(), crate::tour::Target::Sides, sides.response.rect);
            ui.add_space(4.0);
            // A segmented control: the workspaces of this side in one pill.
            let mut remove = None;
            egui::Frame::new()
                .fill(pal.panel)
                .stroke(Stroke::new(1.0, pal.border))
                .corner_radius(theme::PILL)
                .inner_margin(egui::Margin::same(3))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for i in 0..self.layout.workspaces.len() {
                            if self.layout.workspaces[i].terrain != terrain {
                                continue;
                            }
                            let active = self.layout.active == i;
                            let name = crate::i18n::tr_name(&self.layout.workspaces[i].name).to_owned();
                            let text = RichText::new(name).color(if active { pal.text } else { pal.weak });
                            let button = egui::Button::new(if active { text.medium() } else { text })
                                .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
                                .stroke(Stroke::NONE)
                                .corner_radius(theme::PILL)
                                .min_size(vec2(0.0, 24.0));
                            let r = ui.add(button).on_hover_text(tr!("Right-click: reset, duplicate, delete"));
                            if r.clicked() && !active {
                                self.layout.activate(i);
                                crate::sounds::play(Cue::Navigate);
                            }
                            r.context_menu(|ui| {
                                if ui
                                    .add_enabled(self.layout.can_reset(i), egui::Button::new(theme::with_icon(Icon::RotateCcw, tr!("Reset"))))
                                    .on_hover_text(tr!("Back to how this workspace was at first"))
                                    .clicked()
                                {
                                    self.layout.reset_workspace(i);
                                }
                                if ui.button(theme::with_icon(Icon::Copy, tr!("Duplicate"))).clicked() {
                                    self.layout.activate(i);
                                    self.layout.duplicate_current();
                                }
                                if ui.add_enabled(self.layout.side_len(i) > 1, egui::Button::new(theme::with_icon(Icon::Trash2, tr!("Delete")))).clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        let plus = egui::Button::new(RichText::new(theme::glyph(Icon::Plus).to_string()).color(pal.weak))
                            .fill(Color32::TRANSPARENT)
                            .stroke(Stroke::NONE)
                            .corner_radius(theme::PILL)
                            .min_size(vec2(24.0, 24.0));
                        if ui.add(plus).on_hover_text(tr!("New workspace (a copy of this one)")).clicked() {
                            self.layout.duplicate_current();
                        }
                    });
                });
            if let Some(i) = remove {
                self.layout.remove_workspace(i);
            }
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let keys = crate::keymap::get().label(crate::keymap::Action::MaximizeArea).unwrap_or_default();
                ui.label(
                    RichText::new(tr!("{keys}: maximise · drag a title: move", keys))
                        .size(11.5)
                        .color(pal.weak),
                );
            });
        });
    }

    /// What happened and what runs, at the bottom right over the areas: the
    /// last message (a few seconds; an error until it is closed) and the
    /// jobs running in the background, with how far they are.
    fn notices(&mut self, ctx: &egui::Context) {
        let now = ctx.input(|i| i.time);
        let message = self
            .status
            .as_ref()
            .filter(|s| s.error || now - s.at < 8.0)
            .map(|s| (s.text.clone(), s.error, s.reveal.clone(), now - s.at));
        let running: Vec<usize> = (0..self.jobs.len()).filter(|&k| !self.jobs[k].kind.blocks_editing()).collect();
        let slow = self.slow_note;
        if message.is_none() && running.is_empty() && slow.is_none() {
            return;
        }
        let (mut close, mut cancel, mut reveal) = (false, None, None);
        let (mut lighter, mut not_now) = (false, false);
        egui::Area::new(Id::unique("notices"))
            .order(egui::Order::Foreground)
            .anchor(egui::Align2::RIGHT_BOTTOM, vec2(-20.0, -20.0))
            .show(ctx, |ui| {
                let pal = Palette::of(ui);
                let card = |ui: &mut Ui, add: &mut dyn FnMut(&mut Ui)| {
                    egui::Frame::popup(ui.style())
                        .inner_margin(egui::Margin::symmetric(14, 10))
                        .show(ui, |ui| {
                            ui.set_max_width(400.0);
                            ui.horizontal(|ui| add(ui));
                        });
                };
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 8.0;
                    for &k in &running {
                        let job = &self.jobs[k];
                        card(ui, &mut |ui| {
                            crate::loader::loader(ui, 16.0);
                            ui.label(RichText::new(&job.label).size(12.5));
                            match &job.progress {
                                Some(p) => {
                                    ui.add(egui::ProgressBar::new(p.fraction()).desired_width(120.0).show_percentage());
                                    if ui.button(tr!("Cancel")).clicked() {
                                        cancel = Some(k);
                                    }
                                }
                                None => {
                                    let secs = job.started.elapsed().as_secs_f32();
                                    ui.label(RichText::new(tr!("{secs:.0} s", secs)).size(12.0).color(pal.weak));
                                }
                            }
                        });
                    }
                    if let Some(lower) = slow {
                        // Frames stayed slow: a lighter preset, offered
                        // once, quietly; nothing changes unless asked.
                        card(ui, &mut |ui| {
                            ui.label(RichText::new(theme::glyph(Icon::Gauge).to_string()).color(pal.weak).size(13.0));
                            let name = if lower == crate::graphics::Preset::Medium { tr!("Medium") } else { tr!("Low") };
                            ui.add(egui::Label::new(RichText::new(tr!("The 3D views are slow here. The {preset} graphics preset would be lighter.", preset = name)).size(12.5)).wrap());
                            if ui.button(tr!("Use {preset}", preset = name)).clicked() {
                                lighter = true;
                            }
                            let x = egui::Button::new(RichText::new(theme::glyph(Icon::X).to_string()).color(pal.weak)).fill(Color32::TRANSPARENT).stroke(Stroke::NONE);
                            if ui.add(x).on_hover_text(tr!("Not now")).clicked() {
                                not_now = true;
                            }
                        });
                    }
                    if let Some((text, error, path, age)) = &message {
                        // Fades away over its last second, unless an error.
                        if !error {
                            ui.multiply_opacity(((8.0 - age) as f32).clamp(0.0, 1.0));
                        }
                        card(ui, &mut |ui| {
                            let (icon, color) = if *error { (Icon::TriangleAlert, pal.danger) } else { (Icon::CircleCheck, pal.success) };
                            ui.label(RichText::new(theme::glyph(icon).to_string()).color(color).size(13.0));
                            ui.add(egui::Label::new(RichText::new(text).size(12.5).color(if *error { pal.danger } else { pal.text })).wrap());
                            if let Some(path) = path
                                && ui.button(tr!("Show in Finder")).clicked()
                            {
                                reveal = Some(path.clone());
                            }
                            let x = egui::Button::new(RichText::new(theme::glyph(Icon::X).to_string()).color(pal.weak)).fill(Color32::TRANSPARENT).stroke(Stroke::NONE);
                            if ui.add(x).on_hover_text(tr!("Close")).clicked() {
                                close = true;
                            }
                        });
                    }
                });
            });
        if let Some(k) = cancel {
            self.jobs[k].cancel();
        }
        if let Some(path) = reveal {
            jobs::reveal(&path);
        }
        if close {
            self.status = None;
        }
        if lighter && let Some(lower) = slow {
            self.use_graphics(crate::graphics::Graphics::of(lower));
            let name = if lower == crate::graphics::Preset::Medium { tr!("Medium") } else { tr!("Low") };
            self.say(ctx, tr!("Graphics: the {preset} preset, in Settings ▸ Graphics.", preset = name), false, None);
        }
        if not_now {
            self.slow_note = None;
        }
        // Drawn again only for what moves: the progress of tasks, the fade
        // of a message, and its end (an error stays, still).
        match &message {
            _ if !running.is_empty() => ctx.request_repaint_after(std::time::Duration::from_millis(100)),
            Some((_, false, _, age)) if *age >= 7.0 => ctx.request_repaint(),
            Some((_, false, _, age)) => ctx.request_repaint_after(std::time::Duration::from_secs_f64(7.0 - age)),
            _ => {}
        }
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
                let label = if zone == Zone::Center { tr!("Swap") } else { tr!("Split") };
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
        crate::tour::mark(ui.ctx(), crate::tour::Target::Editor(editor), rect);
        let painter = ui.painter_at(rect.expand(1.0));
        // A card: its title and commands along its top, on the card itself.
        painter.rect_filled(rect, theme::CARD_RADIUS, pal.panel);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        let content = Rect::from_min_max(pos2(rect.min.x + 1.0, header.bottom()), pos2(rect.max.x - 1.0, rect.max.y - 1.0));
        let maximized = self.layout.current().maximized == Some(id);
        let single = self.layout.current().root.count() == 1;

        // ---- header --------------------------------------------------------
        let mut hui = ui.new_child(
            UiBuilder::new()
                .id_salt(("area header", id))
                .max_rect(header.shrink2(vec2(8.0, 5.0)))
                .layout(egui::Layout::left_to_right(Align::Center)),
        );
        hui.set_clip_rect(header.intersect(ui.clip_rect()));
        hui.spacing_mut().item_spacing.x = 4.0;
        hui.spacing_mut().interact_size.y = 24.0;
        hui.spacing_mut().button_padding = vec2(8.0, 3.0);
        // The commands of a header have no outline: they show under the
        // pointer, and when chosen.
        {
            let w = &mut hui.visuals_mut().widgets;
            w.inactive.weak_bg_fill = Color32::TRANSPARENT;
            w.inactive.bg_fill = Color32::TRANSPARENT;
            for state in [&mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
                state.bg_stroke = Stroke::NONE;
            }
        }
        // What the area shows: every editor on one page, in columns.
        hui.menu_button(format!("{} {}", theme::glyph(editor.icon()), theme::glyph(Icon::ChevronDown)), |ui| {
            let columns = crate::picker::columns_for(ui.ctx().content_rect().width() - 32.0);
            if let Some(kind) = crate::picker::page(ui, Some(editor), shown, self.settings.developer, columns) {
                self.layout.current_mut().root.set_editor(id, kind);
                ui.close();
            }
        })
        .response
        .on_hover_text(tr!("What this area shows"));
        let title = hui
            .add(egui::Label::new(RichText::new(crate::i18n::tr_name(editor.label())).medium()).sense(Sense::drag()).selectable(false))
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
                    .button(theme::with_icon(Icon::Pickaxe, tr!("Sculpt")))
                    .on_hover_text(tr!("Build and dig in 3D: the 3D Sculpt workspace"))
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
            // As in the modernised WorldPainter: a menu of commands, and with
            // Shift held, a button that closes the area at once.
            if shift && !single && !maximized {
                if ui
                    .add(egui::Button::new(RichText::new(theme::glyph(Icon::Trash2).to_string()).size(12.5)).frame(false))
                    .on_hover_text(tr!("Close this area"))
                    .clicked()
                {
                    self.layout.close(id);
                }
                return;
            }
            let sibling = self.layout.current().root.sibling(id);
            ui.menu_button(RichText::new(theme::glyph(Icon::Ellipsis).to_string()).size(14.0).color(pal.weak), |ui| {
                if ui.button(theme::with_icon(Icon::Columns2, tr!("Split vertically"))).clicked() {
                    self.layout.split(id, Dir::Row);
                }
                if ui.button(theme::with_icon(Icon::Rows2, tr!("Split horizontally"))).clicked() {
                    self.layout.split(id, Dir::Column);
                }
                ui.add_enabled_ui(sibling.is_some() && !maximized, |ui| {
                    ui.menu_button(theme::with_icon(Icon::Combine, tr!("Join with")), |ui| {
                        if let Some((_, editor, dir, first)) = sibling {
                            let side = match (dir, first) {
                                (Dir::Row, true) => tr!("on the right"),
                                (Dir::Row, false) => tr!("on the left"),
                                (Dir::Column, true) => "below",
                                (Dir::Column, false) => "above",
                            };
                            if ui.button(theme::with_icon(editor.icon(), &tr!("{label} {side}", label = crate::i18n::tr_name(editor.label()), side))).clicked() {
                                self.layout.join(id);
                            }
                        }
                    });
                });
                ui.separator();
                let label = if maximized { tr!("Restore the layout") } else { tr!("Maximise area") };
                let icon = if maximized { Icon::Minimize2 } else { Icon::Maximize2 };
                if ui
                    .add_enabled(!single || maximized, egui::Button::new(theme::with_icon(icon, label)).shortcut_text(crate::keymap::get().label(crate::keymap::Action::MaximizeArea).unwrap_or_default()))
                    .clicked()
                {
                    self.layout.toggle_maximize(id);
                }
                if ui
                    .add_enabled(!single && !maximized, egui::Button::new(theme::with_icon(Icon::X, tr!("Close area"))))
                    .clicked()
                {
                    self.layout.close(id);
                }
            })
            .response
            .on_hover_text(tr!("Area commands · Shift: close button · drag a corner into the area to split it"));
        });

        // ---- corners: drag one into the area to split it ----------------------
        // The top ones over the header; the bottom ones go over the content,
        // after it.
        for corner in [Corner::TopLeft, Corner::TopRight] {
            if let Some((dir, first, at)) = corner_split(ui, &painter, rect, id, corner) {
                self.layout.split_at(id, dir, first, at);
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
                let pan_key = !cui.ctx().egui_wants_keyboard_input() && cui.input(|i| crate::keymap::held(i, crate::keymap::Action::Pan));
                if cui.input(|i| i.pointer.hover_pos()).is_some_and(|p| content.contains(p)) {
                    self.active_map = Some(id);
                }
                let overlay = self.versions.overlay(cui.ctx(), &self.doc);
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
                        overlay,
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
                    .show(&mut cui, |ui| panels::sculpt_toolbar(ui, &mut self.tools, &mut self.brushes, &self.doc.world.registry));
                self.sculpt3d.ui(&mut cui, frame, &mut self.doc, &mut self.tools, self.map_center, true);
            }
            EditorKind::Tools => scrolled(&mut cui, &mut |ui| panels::tools_panel(ui, &mut self.tools)),
            EditorKind::ToolSettings => {
                scrolled(&mut cui, &mut |ui| panels::tool_settings(ui, &mut self.tools, &mut self.doc, &mut self.blocks))
            }
            EditorKind::Blocks => {
                let mut feedback = None;
                scrolled(&mut cui, &mut |ui| {
                    feedback = panels::blocks_panel(ui, &mut self.tools, &mut self.doc, &mut self.blocks, &mut self.settings.minecraft_jar, &mut self.settings.mod_jars);
                });
                if feedback.is_some() {
                    self.feedback = feedback;
                }
            }
            EditorKind::BrushSettings => {
                let (min_y, max_y) = (self.doc.dimension().min_y, self.doc.dimension().max_y);
                let heights = self.doc.heights(&cui.ctx().clone());
                let sea = self.doc.world.meta.sea_level;
                scrolled(&mut cui, &mut |ui| {
                    let light = self.settings.light as f32 * 22.5;
                    panels::brush_settings(ui, &mut self.tools, &mut self.brushes, &self.doc.world.registry, min_y, max_y, light, heights.as_deref(), sea)
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
            EditorKind::Packs => {
                let mut said = None;
                let catalogue = self.blocks.catalogue.clone();
                scrolled(&mut cui, &mut |ui| said = crate::packs::packs_panel(ui, &mut self.tools, &mut self.doc, catalogue.as_deref(), &mut self.layers_ui.search, &mut self.layers_ui.library));
                if said.is_some() {
                    self.feedback = said;
                }
            }
            EditorKind::Layers => {
                let mut said = None;
                scrolled(&mut cui, &mut |ui| said = crate::layers::layers_panel(ui, &mut self.tools, &mut self.doc, &mut self.blocks, &mut self.layers_ui));
                if said.is_some() {
                    self.feedback = said;
                }
            }
            EditorKind::Levels => {
                let mut change = panels::LevelsChange::default();
                scrolled(&mut cui, &mut |ui| change = panels::levels_panel(ui, &mut self.doc, &mut self.rename));
                if change.composition {
                    self.map_cache.invalidate_all();
                    self.view3d.invalidate_all();
                    self.sculpt3d.invalidate_all();
                }
            }
            EditorKind::Versions => {
                let busy = self.jobs.iter().any(|j| j.kind == jobs::JobKind::SavePoint);
                let mut action = None;
                scrolled(&mut cui, &mut |ui| action = self.versions.panel(ui, &self.doc, busy));
                if let Some(action) = action {
                    let ctx = cui.ctx().clone();
                    self.versions_action(&ctx, action);
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
                if let Some((ok, msg, secs)) = self.graph.done.take() {
                    let ctx = cui.ctx().clone();
                    if ok {
                        self.task_done(&ctx, Cue::Ready, tr!("Conversion done"), &msg, secs);
                    } else {
                        self.task_done(&ctx, Cue::Error, tr!("Conversion failed"), &msg, secs);
                    }
                }
            }
            EditorKind::GraphView => self.graph.viewport_ui(&mut cui, frame, &self.doc),
            EditorKind::GraphProps => {
                egui::Frame::new()
                    .inner_margin(egui::Margin::symmetric(10, 8))
                    .show(&mut cui, |ui| self.graph.properties_ui(ui, &mut self.doc));
            }
            EditorKind::RenderSettings => scrolled(&mut cui, &mut |ui| self.graph.render_ui(ui, &self.doc)),
            EditorKind::Developer => {
                if self.settings.developer {
                    let info = crate::devtools::DevInfo { gpu: frame.wgpu_render_state(), ..self.dev_info() };
                    let stats = &self.dev;
                    scrolled(&mut cui, &mut |ui| crate::devtools::panel(ui, stats, &info));
                } else {
                    let mut turn_on = false;
                    scrolled(&mut cui, &mut |ui| {
                        ui.label(RichText::new(tr!("The Developer panel needs developer mode.")).color(Palette::of(ui).weak));
                        turn_on = ui.button(theme::with_icon(Icon::Activity, tr!("Turn developer mode on"))).clicked();
                    });
                    if turn_on {
                        self.settings.developer = true;
                    }
                }
            }
            EditorKind::Empty => {
                let mut picked = None;
                let developer = self.settings.developer;
                scrolled(&mut cui, &mut |ui| picked = panels::empty_area(ui, shown, developer));
                if let Some(kind) = picked {
                    self.layout.current_mut().root.set_editor(id, kind);
                }
            }
        }
        // The card's rounded corners over what its editor drew square, and
        // its hairline over that.
        round_off(&painter, rect, theme::CARD_RADIUS as f32, pal.app_bg);
        painter.rect_stroke(rect, theme::CARD_RADIUS, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        // The bottom corners, over what the editor drew (a map or a 3D view
        // takes every drag there otherwise).
        for corner in [Corner::BottomLeft, Corner::BottomRight] {
            if let Some((dir, first, at)) = corner_split(ui, &painter, rect, id, corner) {
                self.layout.split_at(id, dir, first, at);
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
            modal(ctx, "busy").show(ctx, |ui| {
                ui.set_width(360.0);
                ui.horizontal(|ui| {
                    crate::loader::loader(ui, 16.0);
                    ui.label(RichText::new(label).bold());
                });
                if let Some(f) = progress {
                    ui.add(egui::ProgressBar::new(f).show_percentage());
                }
                ui.horizontal(|ui| {
                    ui.label(RichText::new(tr!("{secs:.0} s", secs)).color(Palette::of(ui).weak));
                    if progress.is_some() {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            cancel = ui.button(tr!("Cancel")).clicked();
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
        if self.tools.gradient.editor.is_some() {
            crate::gradient_ui::dialog(ctx, &mut self.tools, &mut self.doc, &mut self.blocks, &self.settings.pack_author);
        }
        if self.pack_dialog.is_some()
            && let Some(outcome) = crate::pack_share::dialog(ctx, &mut self.pack_dialog, &mut self.layers_ui.library, &self.doc.world.registry, &mut self.settings.pack_author)
        {
            self.pack_outcome(ctx, outcome);
        }
        if let Some(biomes) = self.tools.run_auto.take() {
            let snapshot = self.doc.world.snapshot();
            self.jobs.push(Job::auto(ctx, snapshot, self.doc.dim, self.tools.auto.clone(), biomes, self.doc.revision).on(self.doc.generation));
        }
        if std::mem::take(&mut self.tools.pattern.paint_map) {
            let snapshot = self.doc.world.snapshot();
            let level = self.doc.active_level.min(self.doc.dimension().levels.len() - 1);
            let (settings, filter) = (self.tools.pattern.clone(), self.tools.filter.clone());
            self.jobs.push(Job::pattern(ctx, snapshot, self.doc.dim, level, settings, filter, self.doc.revision).on(self.doc.generation));
        }
        if std::mem::take(&mut self.tools.open_rules) && self.dialog.is_none() {
            self.dialog = Some(Dialog::AutoRules(self.tools.tool == Tool::AutoPaint));
        }
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        match dialog {
            Dialog::Help(mut state) => {
                let response = modal(ctx, "help").show(ctx, |ui| {
                    if crate::help::ui(ui, &mut state) {
                        keep = false;
                    }
                });
                if response.should_close() {
                    keep = false;
                }
                if keep {
                    self.dialog = Some(Dialog::Help(state));
                }
            }
            Dialog::Preferences(mut section) => {
                let response = modal(ctx, "preferences").show(ctx, |ui| {
                    // The keyboard's shortcuts take three columns.
                    let keys = section == PrefsSection::Keyboard;
                    let (content, height) = if keys { (crate::keys_page::WIDTH, 560.0) } else { (520.0, 360.0) };
                    ui.set_width(content + 240.0);
                    let pal = Palette::of(ui);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(theme::glyph(Icon::Settings).to_string()).size(17.0).color(pal.accent));
                        ui.label(RichText::new(tr!("Settings")).size(17.0).medium());
                    });
                    ui.add_space(14.0);
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_width(176.0);
                            ui.spacing_mut().item_spacing.y = 2.0;
                            for (s, icon, label) in [
                                (PrefsSection::Appearance, Icon::Palette, tr!("Appearance")),
                                (PrefsSection::Accessibility, Icon::Accessibility, tr!("Accessibility")),
                                (PrefsSection::Typeface, Icon::Type, tr!("Typeface")),
                                (PrefsSection::Sounds, Icon::Volume2, tr!("Sounds")),
                                (PrefsSection::Notifications, Icon::Bell, tr!("Notifications")),
                                (PrefsSection::Memory, Icon::MemoryStick, tr!("Memory")),
                                (PrefsSection::Autosave, Icon::History, tr!("Autosave")),
                                (PrefsSection::Keyboard, Icon::Keyboard, tr!("Keyboard")),
                                (PrefsSection::Updates, Icon::CircleArrowUp, tr!("Updates")),
                                (PrefsSection::Graphics, Icon::Gauge, tr!("Graphics")),
                                (PrefsSection::Developer, Icon::Activity, tr!("Developer")),
                            ] {
                                let on = section == s;
                                let text = RichText::new(theme::with_icon(icon, label)).size(13.0).color(if on { pal.text } else { pal.weak });
                                let button = egui::Button::new(if on { text.medium() } else { text })
                                    .fill(if on { pal.accent_soft } else { Color32::TRANSPARENT })
                                    .stroke(Stroke::NONE)
                                    .corner_radius(theme::PILL)
                                    .min_size(vec2(176.0, 30.0));
                                if ui.add(button).clicked() && !on {
                                    section = s;
                                    crate::sounds::play(Cue::Navigate);
                                }
                            }
                        });
                        ui.add_space(10.0);
                        let x = ui.cursor().min.x;
                        let top = ui.cursor().min.y;
                        ui.painter().vline(x, top..=top + height, Stroke::new(1.0, pal.border));
                        ui.add_space(22.0);
                        // Every section in the same room: the window keeps its
                        // size, a long section scrolls.
                        ui.vertical(|ui| {
                            ui.set_width(content);
                            ui.set_min_height(height);
                            egui::ScrollArea::vertical().id_salt(("settings", section as u8)).max_height(height).show(ui, |ui| {
                                ui.set_width(content - 12.0);
                                match section {
                                    PrefsSection::Appearance => self.prefs_appearance(ui, ctx),
                                    PrefsSection::Accessibility => self.prefs_accessibility(ui),
                                    PrefsSection::Typeface => self.prefs_typeface(ui, ctx),
                                    PrefsSection::Sounds => self.prefs_sounds(ui),
                                    PrefsSection::Notifications => self.prefs_notifications(ui),
                                    PrefsSection::Memory => self.prefs_memory(ui),
                                    PrefsSection::Autosave => self.prefs_autosave(ui),
                                    PrefsSection::Keyboard => self.prefs_keyboard(ui),
                                    PrefsSection::Updates => self.prefs_updates(ui),
                                    PrefsSection::Graphics => self.prefs_graphics(ui),
                                    PrefsSection::Developer => self.prefs_developer(ui),
                                }
                            });
                        });
                    });
                    ui.add_space(14.0);
                    ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(theme::primary_button(ui, tr!("Done"))).clicked() {
                            keep = false;
                        }
                    });
                });
                // Escape is a key like any other while one is recorded.
                if response.should_close() && !crate::keymap::recording() {
                    keep = false;
                }
                // A section may open another dialog (an update's notes).
                if keep && self.dialog.is_none() {
                    self.dialog = Some(Dialog::Preferences(section));
                }
            }
            Dialog::Reshape(mut form) => {
                use crate::reshape::Choice;
                let mut choice = Choice::Keep;
                let response = modal(ctx, "whole map").show(ctx, |ui| {
                    choice = form.ui(ui, &self.doc.world.registry);
                });
                if response.should_close() {
                    keep = false;
                }
                match choice {
                    Choice::Keep => {}
                    Choice::Close => keep = false,
                    Choice::Apply(op) => {
                        keep = false;
                        let snapshot = self.doc.world.snapshot();
                        let bed = self.doc.world.registry.terrain_by_name("Gravel");
                        self.jobs.push(Job::reshape(ctx, snapshot, self.doc.dim, op, bed, self.doc.revision).on(self.doc.generation));
                    }
                }
                if keep {
                    self.dialog = Some(Dialog::Reshape(form));
                }
            }
            Dialog::Analysis(mut view) => {
                use crate::analysis::Choice;
                let mut choice = Choice::Keep;
                let revision = self.doc.revision;
                let response = modal(ctx, "analysis").show(ctx, |ui| {
                    choice = view.ui(ui, revision);
                });
                if response.should_close() {
                    keep = false;
                }
                match choice {
                    Choice::Keep => {}
                    Choice::Close => keep = false,
                    Choice::Again => {
                        let snapshot = self.doc.world.snapshot();
                        view.again(ctx, snapshot, self.doc.dim, revision);
                    }
                    Choice::Show(x, z) => {
                        keep = false;
                        self.show_on_map(ctx, x, z);
                    }
                    Choice::Say(text, error) => self.say(ctx, text, error, None),
                    Choice::Ores => {
                        self.analysis = Some(view);
                        self.open_ores(ctx);
                        return;
                    }
                }
                if keep {
                    self.dialog = Some(Dialog::Analysis(view));
                } else {
                    self.analysis = Some(view);
                }
            }
            Dialog::PointName(index, mut name) => {
                let mut done = None;
                let response = modal(ctx, "save point").show(ctx, |ui| {
                    ui.set_width(380.0);
                    let pal = Palette::of(ui);
                    let title = if index.is_some() { tr!("Rename the save point") } else { tr!("Save a point") };
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(theme::glyph(Icon::BookmarkPlus).to_string()).size(17.0).color(pal.accent));
                        ui.label(RichText::new(title).bold().size(15.0));
                    });
                    if index.is_none() {
                        ui.add(egui::Label::new(RichText::new(tr!("The map as it is now, kept in its file under this name: to restore, open or compare later.")).color(pal.weak)).wrap());
                    }
                    ui.add_space(10.0);
                    let edit = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Before the rivers")).desired_width(f32::INFINITY));
                    if !edit.has_focus() && !ui.memory(|m| m.focused().is_some()) {
                        edit.request_focus();
                    }
                    let ok = !name.trim().is_empty();
                    if ok && edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        done = Some(true);
                    }
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            let label = if index.is_some() { tr!("Rename") } else { tr!("Save Point") };
                            if ui.add_enabled(ok, theme::primary_button(ui, label)).clicked() {
                                done = Some(true);
                            }
                            if ui.button(tr!("Cancel")).clicked() {
                                done = Some(false);
                            }
                        });
                    });
                });
                let trimmed = name.trim().to_owned();
                match done {
                    Some(true) => {
                        let name = trimmed;
                        match index {
                            Some(i) => {
                                if let Some(p) = self.doc.world.points.get_mut(i)
                                    && p.name != name
                                {
                                    p.name = name;
                                    self.doc.mark_changed();
                                }
                            }
                            None => {
                                let snapshot = massif_core::versions::SavePoint::copy_of(&mut self.doc.world);
                                self.jobs.push(Job::save_point(ctx, snapshot, name).on(self.doc.generation));
                            }
                        }
                    }
                    Some(false) => {}
                    None => {
                        if !response.should_close() {
                            self.dialog = Some(Dialog::PointName(index, name));
                        }
                    }
                }
            }
            Dialog::Crashed(said, copy) => {
                let mut choice = None;
                let response = modal(ctx, "crashed").show(ctx, |ui| {
                    ui.set_width(460.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new(tr!("Massif ran into an error")).bold().size(15.0));
                    let about = if copy.is_some() {
                        tr!("Your map was not lost: a copy of it with every change was written. Save it under a name of your own now, then carry on or quit. If you quit, Massif offers the copy back when it starts again.")
                    } else if self.doc.is_modified() {
                        tr!("Massif could not write a copy of your map. Save it under a new name now, then quit and start Massif again.")
                    } else {
                        tr!("Your map has no unsaved changes. You can carry on, or quit and start Massif again.")
                    };
                    ui.add(egui::Label::new(RichText::new(about).color(pal.weak)).wrap());
                    ui.add_space(6.0);
                    ui.add(egui::Label::new(RichText::new(&said).size(11.0).monospace().color(pal.weak)).wrap());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Quit")).clicked() {
                            choice = Some(2);
                        }
                        if ui.button(theme::with_icon(Icon::Bug, tr!("Report the Problem…"))).on_hover_text(tr!("A file to send us, saying what Massif was doing when the error came")).clicked() {
                            choice = Some(3);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if self.doc.is_modified() && ui.add(theme::primary_button(ui, tr!("Save As…"))).clicked() {
                                choice = Some(0);
                            }
                            if ui.button(tr!("Carry On")).clicked() {
                                choice = Some(1);
                            }
                        });
                    });
                });
                match choice {
                    Some(0) => {
                        self.save(ctx, true);
                    }
                    Some(1) => {}
                    Some(3) => {
                        self.open_report(ctx, false);
                        self.report_return = Some(Dialog::Crashed(said, copy));
                    }
                    Some(2) => {
                        // Not a clean close: the copy stays for the next start.
                        self.autosave.keep();
                        self.allow_close = true;
                        ctx.send_viewport_cmd(ViewportCommand::Close);
                    }
                    _ => {
                        if !response.should_close() {
                            self.dialog = Some(Dialog::Crashed(said, copy));
                        }
                    }
                }
            }
            Dialog::DeletePoint(index) => {
                let mut choice = None;
                let name = self.doc.world.points.get(index).map(|p| p.name.clone()).unwrap_or_default();
                let response = modal(ctx, "delete point").show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(tr!("Delete the save point “{name}”?", name)).bold().size(15.0));
                    ui.add(egui::Label::new(RichText::new(tr!("It goes from the file at the next save, and cannot be brought back then.")).color(Palette::of(ui).weak)).wrap());
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("Delete"))).clicked() {
                                choice = Some(true);
                            }
                            if ui.button(tr!("Cancel")).clicked() {
                                choice = Some(false);
                            }
                        });
                    });
                });
                match choice {
                    Some(true) => {
                        if index < self.doc.world.points.len() {
                            self.doc.world.points.remove(index);
                            self.doc.mark_changed();
                            self.say(ctx, tr!("Deleted the save point “{name}”", name), false, None);
                        }
                    }
                    Some(false) => {}
                    None => {
                        if !response.should_close() {
                            self.dialog = Some(Dialog::DeletePoint(index));
                        }
                    }
                }
            }
            Dialog::Caves(mut form) => {
                use crate::caves::Choice;
                let mut choice = Choice::Keep;
                let response = modal(ctx, "caves").show(ctx, |ui| {
                    choice = form.ui(ui);
                });
                if response.should_close() {
                    keep = false;
                }
                match choice {
                    Choice::Keep => {}
                    Choice::Close => keep = false,
                    Choice::Carve(settings) => {
                        keep = false;
                        self.caves = Some(settings.clone());
                        let snapshot = self.doc.world.snapshot();
                        self.jobs.push(Job::caves(ctx, snapshot, self.doc.dim, settings, self.doc.revision).on(self.doc.generation));
                    }
                }
                if keep {
                    self.dialog = Some(Dialog::Caves(form));
                }
            }
            Dialog::Real(mut form) => {
                use crate::real_terrain::Choice;
                let mut choice = Choice::Keep;
                let response = modal(ctx, "real terrain").show(ctx, |ui| {
                    choice = form.ui(ui);
                });
                if response.should_close() {
                    keep = false;
                }
                self.real = Some(form.remembered());
                match choice {
                    Choice::Keep => {}
                    Choice::Close => keep = false,
                    Choice::Make(job) => {
                        keep = false;
                        if job.target == crate::real_terrain::Target::NewWorld {
                            self.request(ctx, Pending::Real(job));
                        } else {
                            let snapshot = self.doc.world.snapshot();
                            let rules = self.tools.auto.clone();
                            self.jobs.push(Job::real(ctx, crate::tiles::TileStore::global(), *job, Some(snapshot), self.doc.dim, rules, self.doc.revision).on(self.doc.generation));
                        }
                    }
                }
                if keep {
                    self.dialog = Some(Dialog::Real(form));
                }
            }
            Dialog::Ores(mut view) => {
                let mut close = None;
                let response = modal(ctx, "ores").show(ctx, |ui| {
                    if let crate::ores::Choice::Close(settings) = view.ui(ui) {
                        close = Some(settings);
                    }
                });
                if response.should_close() && close.is_none() {
                    close = Some(view.settings.clone());
                }
                match close {
                    Some(mut settings) => {
                        settings.tidy();
                        if settings != self.doc.world.meta.ores {
                            self.doc.world.meta.ores = settings;
                            self.doc.mark_changed();
                            self.say(ctx, tr!("Ores set: the next export lays them so"), false, None);
                        }
                    }
                    None => self.dialog = Some(Dialog::Ores(view)),
                }
            }
            Dialog::AutoRules(mut paint) => {
                let response = modal(ctx, "auto rules").show(ctx, |ui| {
                    ui.set_width(700.0);
                    ui.label(
                        RichText::new(tr!("Automatic biomes and paint"))
                            .bold()
                            .size(15.0),
                    );
                    ui.label(
                        RichText::new(tr!("Biomes from the land, ground from the biomes."))
                            .size(11.5)
                            .color(Palette::of(ui).weak),
                    );
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        ui.selectable_value(
                            &mut paint,
                            false,
                            theme::with_icon(Icon::Earth, tr!("Biomes from the land")),
                        );
                        ui.selectable_value(
                            &mut paint,
                            true,
                            theme::with_icon(Icon::WandSparkles, tr!("Paint from the biomes")),
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
                        if ui.button(tr!("Reset to defaults")).clicked() {
                            if paint {
                                self.tools.auto.paint = Default::default();
                            } else {
                                self.tools.auto.land = Default::default();
                            }
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("Done"))).clicked() {
                                keep = false;
                            }
                            let label = if paint {
                                tr!("Paint the whole map")
                            } else {
                                tr!("Give the whole map its biomes")
                            };
                            if ui
                                .button(label)
                                .on_hover_text(tr!("One undoable step"))
                                .clicked()
                            {
                                self.tools.run_auto = Some(!paint);
                                keep = false;
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
            Dialog::Recover => {
                // A choice per backup: restore one (the map open now, if it
                // has changes, asked about first), discard it, or decide later.
                // Or first a report of the crash, back here after.
                let mut report = false;
                let mut restore = None;
                let mut discard = None;
                let response = modal(ctx, "recover").show(ctx, |ui| {
                    ui.set_width(460.0);
                    let pal = Palette::of(ui);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(theme::glyph(Icon::History).to_string()).size(17.0).color(pal.accent));
                        ui.label(RichText::new(tr!("Restore unsaved changes?")).bold().size(15.0));
                    });
                    ui.add_space(2.0);
                    let found = &self.autosave.found;
                    let about = if found.len() == 1 {
                        tr!("Massif closed without saving this map. A copy of it was kept: restore it, then save it to keep it.")
                    } else {
                        tr!("Massif closed without saving these maps. A copy of each was kept: restore one, then save it to keep it.")
                    };
                    ui.add(egui::Label::new(RichText::new(about).color(pal.weak)).wrap());
                    ui.add_space(10.0);
                    crate::widgets::well(ui, |ui| {
                        for (k, b) in found.iter().enumerate() {
                            if k > 0 {
                                let r = ui.available_rect_before_wrap();
                                ui.painter().hline(r.x_range().expand(12.0), r.top() + 2.0, Stroke::new(1.0, pal.border));
                                ui.add_space(5.0);
                            }
                            let file = b.info.path.as_ref().map_or_else(|| tr!("Never saved").to_owned(), |p| jobs::display_name(p));
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 1.0;
                                    ui.label(RichText::new(&b.info.name).size(13.5).medium());
                                    ui.label(RichText::new(tr!("Taken {age} · {file} · {human}", age = b.age(), file, human = panels::human(b.bytes))).size(11.5).color(pal.weak));
                                });
                                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                                    if ui.add(theme::primary_button(ui, tr!("Restore"))).clicked() {
                                        restore = Some(k);
                                    }
                                    if ui.button(tr!("Discard")).clicked() {
                                        discard = Some(k);
                                    }
                                });
                            });
                        }
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(tr!("Kept until you choose.")).size(11.5).color(pal.weak));
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(tr!("Later")).clicked() {
                                keep = false;
                            }
                            if ui.button(theme::with_icon(Icon::Bug, tr!("Report the Problem…"))).on_hover_text(tr!("A file to send us, saying what Massif was doing when it closed")).clicked() {
                                report = true;
                            }
                        });
                    });
                });
                if let Some(k) = discard {
                    let b = self.autosave.found[k].clone();
                    self.autosave.discard(&b);
                    self.say(ctx, tr!("Discarded the copy of “{name}”", name = b.info.name), false, None);
                }
                if let Some(k) = restore {
                    let b = self.autosave.found.remove(k);
                    keep = false;
                    if self.has_map && self.doc.is_modified() {
                        // The map open now first: its changes are asked about.
                        self.autosave.found.insert(0, b);
                        self.dialog = Some(Dialog::Unsaved(Pending::Recover));
                    } else {
                        self.jobs.push(Job::recover(ctx, b));
                    }
                } else if response.should_close() {
                    keep = false;
                }
                if keep && !self.autosave.found.is_empty() {
                    self.dialog = Some(Dialog::Recover);
                }
                if report {
                    self.open_report(ctx, true);
                    self.report_return = Some(Dialog::Recover);
                }
            }
            Dialog::Report(mut form) => {
                use crate::report_view::Choice;
                let map_note = match self.doc.path.as_ref().and_then(|p| std::fs::metadata(p).ok()) {
                    Some(m) => tr!("The map as it is now, about {size} (its file on disk). Only when the problem needs it: it may be large, and it is yours to share.", size = panels::human(m.len())),
                    None => tr!("The map as it is now. Only when the problem needs it: it may be large, and it is yours to share.").to_owned(),
                };
                match crate::report_view::ui(ctx, &mut form, self.recording.as_ref(), &map_note) {
                    Choice::Stay => {}
                    Choice::Close => {
                        keep = false;
                        self.recording = None;
                    }
                    Choice::Record => {
                        self.recording = Some(crate::report::Recording::start());
                        self.report_aside = Some(form.clone());
                        keep = false;
                    }
                    Choice::DropRecording => self.recording = None,
                    Choice::Copy => {
                        ctx.copy_text(crate::report::short_summary(&form.inputs, self.recording.as_ref()));
                        self.say(ctx, tr!("Copied a summary of the problem: paste it in your message"), false, None);
                    }
                    Choice::Save => {
                        let path = rfd::FileDialog::new().set_title(tr!("Save the report")).add_filter(tr!("Zip archive"), &["zip"]).set_file_name(crate::report::file_name()).save_file();
                        if let Some(mut path) = path {
                            if path.extension().is_none() {
                                path.set_extension("zip");
                            }
                            self.save_report(ctx, &form, path);
                            self.recording = None;
                            keep = false;
                        }
                    }
                }
                if keep {
                    self.dialog = Some(Dialog::Report(form));
                } else if self.report_aside.is_none()
                    && let Some(back) = self.report_return.take()
                    && !(matches!(back, Dialog::Recover) && self.autosave.found.is_empty())
                {
                    self.dialog = Some(back);
                }
            }
            Dialog::Unsaved(pending) => {
                let mut choice = None;
                let response = modal(ctx, "unsaved").show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(tr!("Save the changes to “{name}”?", name = self.doc.world.meta.name)).bold().size(15.0));
                    ui.label(RichText::new(tr!("Your changes will be lost if you don't save them.")).color(Palette::of(ui).weak));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Don't Save")).clicked() {
                            choice = Some(1);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("Save"))).clicked() {
                                choice = Some(2);
                            }
                            if ui.button(tr!("Cancel")).clicked() {
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
                let response = modal(ctx, "export").show(ctx, |ui| {
                    ui.set_width(840.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new(tr!("Export to Minecraft")).bold().size(16.0));
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
                                [(ExportTab::General, tr!("General")), (ExportTab::World, tr!("World"))]
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
                        ui.colored_label(pal.danger, tr!("This folder already exists."));
                        crate::widgets::checkbox(ui, &mut form.overwrite, tr!("Replace its region files (everything else is kept)"));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        let test = ui
                            .button(tr!("Test Export…"))
                            .on_hover_text(tr!("Export 384 × 384 blocks around the view, to test it"));
                        if test.clicked() {
                            start = Some(true);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(tr!("Cancel")).clicked() {
                                keep = false;
                            }
                            let ok = !form.name.trim().is_empty() && (!exists || form.overwrite);
                            if ui
                                .add_enabled(ok, theme::primary_button(ui, tr!("Export")))
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
                        selection_only: form.has_selection && form.selection_only,
                        ..ExportOptions::default()
                    };
                    // Keeping to the selection, players appear in it.
                    if options.selection_only
                        && let Some(sel) = self.doc.selection()
                        && sel.tile(TilePos::containing(form.spawn[0], form.spawn[2])).is_none()
                        && let Some(b) = sel.bounds()
                    {
                        let centre = ((b.x0 + b.x1) / 2, (b.z0 + b.z1) / 2);
                        let at = (0..64 * 64).map(|k| (centre.0 + (k % 64 - 32) * 16, centre.1 + (k / 64 - 32) * 16)).filter(|&(x, z)| sel.get(x, z) == massif_core::selection::IN).min_by_key(|&(x, z)| (x - centre.0).pow(2) + (z - centre.1).pow(2));
                        if let Some((x, z)) = at {
                            options.spawn = Some([x, form.spawn[1], z]);
                        }
                    }
                    let mut dir = form.parent.join(form.name.trim());
                    if test {
                        let c = TilePos::containing(form.test_centre.0, form.test_centre.1);
                        options.area = Some((c.offset(-1, -1), c.offset(1, 1)));
                        options.overwrite = false;
                        options.level_name = Some(tr!("{trim} (test)", trim = form.name.trim()));
                        dir = massif_anvil::unique_dir(&form.parent, &format!("{} test", form.name.trim()));
                        // Spawn in the middle of what is exported.
                        options.spawn = Some([form.test_centre.0, form.spawn[1].max(self.doc.world.meta.sea_level + 1), form.test_centre.1]);
                    }
                    if form.ores != self.doc.world.meta.ores.on {
                        self.doc.world.meta.ores.on = form.ores;
                        self.doc.mark_changed();
                    }
                    let snapshot = self.doc.world.snapshot();
                    self.jobs.push(Job::export(ctx, snapshot, dir, options));
                } else if keep {
                    self.dialog = Some(Dialog::Export(form));
                }
            }
            Dialog::ImportReport(report, thumbnail) => {
                let mut save = false;
                let response = modal(ctx, "import report").show(ctx, |ui| {
                    ui.set_width(520.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new(tr!("Imported “{name}”", name = report.name)).bold().size(16.0));
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
                            ui.label(RichText::new(tr!("Imported")).color(pal.weak));
                            ui.vertical(|ui| {
                                for d in &report.dimensions {
                                    ui.label(d);
                                }
                            });
                            ui.end_row();
                            ui.label(RichText::new(tr!("Terrains")).color(pal.weak));
                            let total: u64 = report.terrains.iter().map(|t| t.1).sum::<u64>().max(1);
                            ui.vertical(|ui| {
                                ui.label(trn!("{kinds} kinds, {n} custom terrain rebuilt from their materials", "{kinds} kinds, {n} custom terrains rebuilt from their materials", n = report.custom_terrains, kinds = report.terrains.len()));
                                for (name, columns) in report.terrains.iter().take(10) {
                                    let pretty = if name.chars().all(|c| c.is_ascii_uppercase() || c == '_') {
                                        let lower = name.replace('_', " ").to_lowercase();
                                        let mut c = lower.chars();
                                        c.next().map(|f| f.to_uppercase().collect::<String>() + c.as_str()).unwrap_or_default()
                                    } else {
                                        name.clone()
                                    };
                                    ui.label(
                                        RichText::new(tr!("{pretty} — {f64:.1}%", f64 = *columns as f64 * 100.0 / total as f64, pretty))
                                            .size(11.5)
                                            .color(pal.weak),
                                    );
                                }
                            });
                            ui.end_row();
                            if !report.converted.is_empty() {
                                ui.label(RichText::new(tr!("Layers")).color(pal.weak));
                                ui.vertical(|ui| {
                                    ui.label(tr!("Imported as object layers, ground covers, terrains, levels and caves:"));
                                    let names: Vec<String> = report.converted.iter().map(|(n, t)| tr!("{n} ({t} tiles)", n, t)).collect();
                                    ui.label(RichText::new(names.join(", ")).size(11.5).color(pal.weak));
                                });
                                ui.end_row();
                            }
                            if !report.layers.is_empty() {
                                ui.label(RichText::new(tr!("Not imported yet")).color(pal.weak));
                                ui.vertical(|ui| {
                                    ui.label(tr!("Layers Massif does not have yet:"));
                                    let names: Vec<String> = report.layers.iter().map(|(n, t)| tr!("{n} ({t} tiles)", n, t)).collect();
                                    ui.label(RichText::new(names.join(", ")).size(11.5).color(pal.weak));
                                });
                                ui.end_row();
                            }
                            if !report.skipped_dimensions.is_empty() {
                                ui.label(RichText::new(tr!("Skipped")).color(pal.weak));
                                ui.label(report.skipped_dimensions.join(", "));
                                ui.end_row();
                            }
                        });
                    });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("Save as .massif…"))).clicked() {
                                save = true;
                            }
                            if ui.button(tr!("Close")).clicked() {
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
            Dialog::Update => {
                let Some(release) = self.update_offer().cloned() else { return };
                let version = release.tag_name.trim_start_matches('v').to_owned();
                let (mut open, mut skip) = (false, false);
                let response = modal(ctx, "update").show(ctx, |ui| {
                    ui.set_width(520.0);
                    let pal = Palette::of(ui);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(theme::glyph(Icon::CircleArrowUp).to_string()).size(17.0).color(pal.accent));
                        ui.label(RichText::new(tr!("Massif {version} is out", version)).bold().size(15.0));
                    });
                    ui.add_space(2.0);
                    ui.label(RichText::new(tr!("You have {running}. Download the new version from its page, then install it as you did this one; your maps and settings stay.", running = crate::update::RUNNING)).color(pal.weak));
                    ui.add_space(10.0);
                    let notes = crate::help::blocks(&release.notes_since(crate::update::RUNNING));
                    if !notes.is_empty() {
                        let height = (ctx.content_rect().height() - 260.0).clamp(120.0, 420.0);
                        crate::widgets::well(ui, |ui| {
                            egui::ScrollArea::vertical().id_salt("update notes").max_height(height).auto_shrink([false, true]).show(ui, |ui| {
                                ui.set_width(ui.available_width() - 10.0);
                                crate::help::blocks_ui(ui, &notes);
                            });
                        });
                        ui.add_space(12.0);
                    }
                    ui.horizontal(|ui| {
                        if ui.button(tr!("Skip this version")).clicked() {
                            skip = true;
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("{glyph}  Open the release page", glyph = theme::glyph(Icon::ExternalLink)))).clicked() {
                                open = true;
                            }
                            if ui.button(tr!("Later")).clicked() {
                                keep = false;
                            }
                        });
                    });
                });
                if open {
                    ctx.open_url(egui::OpenUrl::new_tab(release.page()));
                    keep = false;
                }
                if skip {
                    self.settings.skipped_version = Some(version);
                    keep = false;
                }
                if response.should_close() {
                    keep = false;
                }
                if keep {
                    self.dialog = Some(Dialog::Update);
                }
            }
            Dialog::About => {
                let response = modal(ctx, "about").show(ctx, |ui| {
                    ui.set_width(400.0);
                    ui.label(
                        RichText::new(format!("{}  Massif", theme::glyph(Icon::MountainSnow)))
                            .bold()
                            .size(18.0),
                    );
                    ui.label(
                        RichText::new(tr!("Version {value}", value = env!("CARGO_PKG_VERSION")))
                            .color(Palette::of(ui).weak),
                    );
                    ui.add_space(6.0);
                    ui.label(tr!("A map editor for Minecraft worlds of any size."));
                    ui.add_space(6.0);
                    ui.add_space(8.0);
                    if ui.button(tr!("Close")).clicked() {
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

    /// Back the map up while it has changes not saved (see `autosave`).
    fn autosave_tick(&mut self, ctx: &egui::Context) {
        let busy = self.doc.history.is_recording() || self.jobs.iter().any(|j| j.kind.blocks_editing() || j.kind == jobs::JobKind::Save);
        let every = std::time::Duration::from_secs(60 * self.settings.autosave_minutes.max(1) as u64);
        let on = self.settings.autosave && self.has_map;
        if let Some(after) = self.autosave.tick(&self.doc, on, every, busy) {
            ctx.request_repaint_after(after);
        }
        if self.autosave.writing() {
            // Its end looked at a moment after it.
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }

    fn handle_close(&mut self, ctx: &egui::Context) {
        if ctx.input(|i| i.viewport().close_requested()) && self.doc.is_modified() && !self.allow_close {
            ctx.send_viewport_cmd(ViewportCommand::CancelClose);
            self.dialog = Some(Dialog::Unsaved(Pending::Quit));
        }
    }

    /// Whether quitting is under way: asked about (Unsaved), or waiting for
    /// the map to be saved.
    fn quitting(&self) -> bool {
        matches!(self.dialog, Some(Dialog::Unsaved(Pending::Quit))) || matches!(self.after_save, Some(Pending::Quit))
    }

    /// The system's Quit, on macOS (the Dock's, the app switcher's, a
    /// logout's): as the app's own, asked first when the map has changes. A
    /// quit the system waits for is told no once it is given up: the dialog
    /// cancelled, its save cancelled or failed.
    fn system_quit(&mut self, ctx: &egui::Context) {
        if crate::quit::take() && !self.quitting() {
            self.request(ctx, Pending::Quit);
        }
        if crate::quit::waiting() && !self.quitting() {
            crate::quit::reply(false);
        }
    }

    /// One frame of the whole UI.
    pub fn frame_ui(&mut self, ui: &mut Ui, frame: &eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Until the first frame is shown, startup.log names each part.
        let step = |what: &str| crate::startup::frame_step(&ctx, what);
        step("the theme and the accessibility settings");
        if self.applied_theme != Some(self.settings.theme) {
            self.apply_theme(&ctx);
            self.applied_theme = Some(self.settings.theme);
        }
        self.apply_access(&ctx);
        crate::sounds::configure(self.settings.sounds, self.settings.volume);
        step("the work done in the background (opening, saving, exporting)");
        self.poll_autosave_opening();
        self.poll_jobs(&ctx);
        // A save point open read-only keeps as it is.
        if self.doc.take_back() {
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
            self.say(&ctx, tr!("This save point is open read-only: Save As… makes it a map of its own to change"), true, None);
        }
        let now = ctx.input(|i| i.time);
        self.apply_memory_limit(now);
        // Walking in 3D Sculpt held the mouse, and its view is gone.
        self.sculpt3d.release_if_hidden(&ctx);
        let open = self.dialog.is_some();
        if open != self.dialog_was_open {
            crate::sounds::play(if open { Cue::Open } else { Cue::Close });
            self.dialog_was_open = open;
        }
        step("the brushes and the game's blocks, if loaded");
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
        // The player's view draws the game's own textures.
        self.view3d.catalogue.clone_from(&self.blocks.catalogue);
        self.sculpt3d.catalogue.clone_from(&self.blocks.catalogue);
        if let Some(cat) = self.blocks.catalogue.clone()
            && self.blocks.applied != Some(self.doc.generation)
        {
            massif_assets::apply_colors(&mut self.doc.world.registry, &cat);
            self.blocks.applied = Some(self.doc.generation);
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
        }
        // A cover or an object layer changed how it looks.
        let covers = covers_key(&self.doc.world.registry);
        if self.covers_key.is_some_and(|k| k != covers) {
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
            self.sculpt3d.invalidate_all();
        }
        self.covers_key = Some(covers);
        step("the files the system asks to open, and the shortcuts");
        // A map or a world the Finder asks to open (the first of them).
        let (packs, maps): (Vec<PathBuf>, Vec<PathBuf>) = crate::finder::take().into_iter().partition(|p| crate::pack_share::is_pack_file(p));
        self.pack_inbox.extend(packs);
        if let Some(path) = maps.into_iter().find(|p| {
            let ext = p.extension().map(|e| e.to_string_lossy().to_lowercase()).unwrap_or_default();
            ext == massif_format::EXTENSION || ext == "world"
        }) {
            self.request(&ctx, Pending::Open(Some(path)));
        }
        // Objects dropped on the window: a layer of their own.
        if self.home.is_none() {
            let packs: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| crate::pack_share::is_pack_file(p)).collect());
            self.pack_inbox.extend(packs);
            let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| panels::is_object_file(p)).collect());
            if !dropped.is_empty() {
                let note = panels::import_objects(&mut self.doc, &mut self.tools, &dropped, None, self.blocks.catalogue.as_deref());
                self.feedback = Some(note);
            }
        }
        self.packs(&ctx);
        self.handle_close(&ctx);
        crate::keymap::apply(&ctx, &self.settings.keys);
        // The tour takes the keys while it is shown.
        if self.tour.is_none() {
            self.shortcuts(&ctx);
        }
        self.menu_bar_commands(&ctx);
        self.system_quit(&ctx);
        self.map_cache.sync_generation(self.doc.generation);
        if let Some(msg) = self.feedback.take() {
            self.say(&ctx, msg, false, None);
        }
        if self.home.is_some() {
            step("the home screen");
            self.home_ui(ui, frame, &ctx);
            step("the dialogs");
            self.dialogs(&ctx);
            if self.title != "Massif" {
                ctx.send_viewport_cmd(ViewportCommand::Title("Massif".to_owned()));
                self.title = "Massif".to_owned();
            }
            return;
        }

        step("the editor: its bars and panels");
        let pal = Palette::of(ui);
        let margin = if self.unified_titlebar { 9 } else { 6 };
        let bar = egui::Panel::top("menu bar")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin::symmetric(14, margin)))
            .show(ui, |ui| {
                ui.horizontal(|ui| self.top_bar(ui));
            });
        if self.unified_titlebar {
            frame.set_traffic_lights_position(&ctx, bar.response.rect.y_range(), 14.0);
        }
        egui::Panel::top("workspaces")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin { left: 12, right: 16, top: 0, bottom: 2 }))
            .show(ui, |ui| self.workspace_tabs(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin { left: 8, right: 8, top: 4, bottom: 8 }))
            .show(ui, |ui| self.dock(ui, frame));
        self.notices(&ctx);

        step("the dialogs");
        self.dialogs(&ctx);
        self.tour_ui(&ctx);
        self.autosave_tick(&ctx);

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

impl MassifApp {
    /// A frame stopped on an error: back the map up at once and say so,
    /// with a way to save it elsewhere. Errors that keep coming give up:
    /// the process ends as before, the map's copy offered at the next
    /// start.
    fn after_panic(&mut self, ctx: &egui::Context, said: String) {
        let now = std::time::Instant::now();
        self.panics.retain(|t| now.duration_since(*t) < std::time::Duration::from_secs(10));
        self.panics.push(now);
        let rescued = crate::startup::catch(|| self.autosave.rescue(&self.doc));
        let copy = match rescued {
            Ok(Ok(b)) => b.map(|b| b.file),
            Ok(Err(e)) | Err(e) => {
                crate::startup::step(&format!("the map could not be backed up after the error: {e}"));
                None
            }
        };
        crate::startup::step(&format!("a frame stopped on an error ({}): {said}", if copy.is_some() { "the map backed up" } else { "no backup" }));
        if self.panics.len() > 3 {
            // The same error every frame: nothing to carry on with. The
            // copy stays for the next start.
            self.autosave.keep();
            crate::startup::tell(&tr!("Massif stopped on an error:\n\n{said}\n\nA copy of your map was kept: Massif offers it back when it starts again.", said));
            std::process::exit(1);
        }
        self.dialog = Some(Dialog::Crashed(said, copy));
        ctx.request_repaint();
    }
}

impl eframe::App for MassifApp {
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // The player's view holding the mouse keeps the pointer at its
        // crosshair.
        self.sculpt3d.pin_pointer(raw_input);
    }

    fn ui(&mut self, ui: &mut Ui, frame: &mut eframe::Frame) {
        // The system's Quit is not answered now during a frame (see `quit`).
        crate::quit::frame_begins();
        // Until the first frame is shown, startup.log follows each step.
        crate::startup::frame_begins(ui.ctx(), frame);
        self.dev.begin();
        self.graph.keep_tab(ui.ctx());
        // GitHub's answer about new versions, once it came.
        self.updates.poll();
        // A panic in the frame is stopped here, the map kept (see
        // `after_panic`); the next frame goes on.
        let drawn = crate::startup::catch(|| {
            #[cfg(test)]
            if tests::PANIC_NEXT_FRAME.take() {
                panic!("an error made by a test");
            }
            self.frame_ui(ui, frame);
            // macOS's menu bar shows the menus as this frame left them.
            if crate::menubar::NATIVE {
                crate::menubar::sync(&self.menus());
            }
        });
        if let Err(said) = drawn {
            self.after_panic(ui.ctx(), said);
        }
        self.watch_graphics(ui.ctx());
        let now = ui.input(|i| i.time);
        self.dev.end(now);
        // (After the frame's work is measured: the wait is not work.)
        self.limit_frames(ui.ctx(), frame);
        if self.gpu.is_none() {
            self.gpu = frame.wgpu_render_state().map(|rs| crate::report::Gpu::of(&rs.adapter.get_info()));
        }
        self.report_frame(ui.ctx());
        crate::startup::frame_laid_out(ui);
        // Until the next frame, the system's Quit is answered from what this
        // one leaves: now, or not before the app asks.
        crate::quit::frame_ends(self.doc.is_modified() && !self.allow_close);
    }

    fn on_exit(&mut self) {
        // Closed cleanly: the map was saved, or its changes let go.
        self.autosave.close();
    }

    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        eframe::set_value(storage, LAYOUT_KEY, &self.layout);
        eframe::set_value(storage, "massif.settings", &self.settings);
        eframe::set_value(storage, "massif.render", &self.graph.view.render);
        eframe::set_value(storage, "massif.tools", &self.tools);
        let b = &mut self.settings_backup;
        b.set(LAYOUT_KEY, &self.layout);
        b.set("massif.settings", &self.settings);
        b.set("massif.render", &self.graph.view.render);
        b.set("massif.tools", &self.tools);
        b.write();
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// A command of the bar at the top: an icon in a square, all of them the
/// same size, on one line, whatever draws them (a glyph or a tool's icon).
fn bar_button(ui: &mut Ui, tip: &str, enabled: bool, selected: bool, paint: impl FnOnce(&egui::Painter, Rect, Color32)) -> egui::Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(28.0, 26.0), if enabled { Sense::click() } else { Sense::hover() });
    if selected || (enabled && response.hovered()) {
        let fill = if selected { pal.accent_soft } else { theme::mix(pal.app_bg, pal.text, 0.08) };
        ui.painter().rect_filled(rect.shrink(1.0), theme::TILE_RADIUS, fill);
    }
    let color = if !enabled {
        pal.weak.gamma_multiply(0.55)
    } else if selected {
        pal.accent
    } else {
        pal.text
    };
    paint(ui.painter(), Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), color);
    response.on_hover_text(tip)
}

/// The top bar's word that a newer version is out: a small pill in the
/// accent's soft tint.
fn update_pill(ui: &mut Ui, version: &str) -> egui::Response {
    let pal = Palette::of(ui);
    let text = RichText::new(tr!("{glyph}  {version} is out", glyph = theme::glyph(Icon::CircleArrowUp), version)).size(12.0).color(pal.accent);
    let pill = egui::Button::new(text).fill(pal.accent_soft).stroke(Stroke::NONE).corner_radius(theme::PILL).min_size(vec2(0.0, 22.0));
    let response = ui.add(pill).on_hover_text(tr!("See what is new"));
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A glyph of the icon font, centred on `rect`.
fn glyph_in(painter: &egui::Painter, rect: Rect, icon: Icon, color: Color32) {
    let galley = painter.layout_no_wrap(theme::glyph(icon).to_string(), FontId::proportional(rect.height()), color);
    // The icons sit in the middle of the line the font gives them.
    painter.galley(rect.center() - galley.size() / 2.0, galley, color);
}

/// A dialog: generous corners and margins, as the references draw them.
pub(crate) fn modal(ctx: &egui::Context, id: &str) -> egui::Modal {
    let frame = egui::Frame::window(&ctx.global_style()).inner_margin(egui::Margin::same(22));
    egui::Modal::new(Id::unique(id)).frame(frame)
}

/// A titled group of settings: its name, a word on it, its settings in a
/// well.
fn card(ui: &mut Ui, title: &str, subtitle: &str, add: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    ui.add_space(4.0);
    ui.label(RichText::new(title).medium());
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).size(11.5).color(pal.weak));
    }
    ui.add_space(2.0);
    egui::Frame::new()
        .fill(pal.well())
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(theme::GROUP_RADIUS)
        .inner_margin(egui::Margin::symmetric(14, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
    ui.add_space(8.0);
}

/// A row of a card: a label on the left, the control on the right.
fn row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.add_sized([96.0, 20.0], egui::Label::new(label).truncate());
        add(ui);
    });
}

/// The mods whose blocks the map uses: the version each is made for, and
/// what does not match.
fn export_mods(ui: &mut Ui, form: &mut ExportForm) {
    let pal = Palette::of(ui);
    let mut pick: Option<massif_anvil::Target> = None;
    card(ui, tr!("Mods"), tr!("The map uses blocks of mods: the game needs them, in the version they are made for."), |ui| {
        for m in &form.mods {
            // The mod's whole name: longer than the labels of the rows.
            ui.horizontal(|ui| {
                ui.label(RichText::new(&m.name).medium());
                let (text, ok) = match (&m.minecraft, m.loaded) {
                    (_, false) => (trn!("{n} block · not read in Blocks", "{n} blocks · not read in Blocks", n = m.blocks), false),
                    (Some(v), true) if *v != form.target.name => (trn!("{n} block · made for Minecraft {v}", "{n} blocks · made for Minecraft {v}", n = m.blocks, v), false),
                    (Some(v), true) => (trn!("{n} block · made for Minecraft {v}", "{n} blocks · made for Minecraft {v}", n = m.blocks, v), true),
                    (None, true) => (trn!("{n} block", "{n} blocks", n = m.blocks), true),
                };
                ui.label(RichText::new(text).size(11.5).color(if ok { pal.weak } else { pal.danger }));
            });
            let say = |ui: &mut Ui, text: String| {
                ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal.danger)).wrap());
            };
            if form.target.era() == massif_anvil::Era::Numbered {
                say(ui, tr!("Java {name} numbers its blocks: {mod_name}'s are written as stone.", name = form.target.name, mod_name = m.name));
            } else if let Some(v) = m.minecraft.as_ref().filter(|v| **v != form.target.name) {
                say(ui, tr!("{mod_name} is made for Minecraft {v}: in Java {name}, the game may not load its blocks.", mod_name = m.name, v, name = form.target.name));
                if let Some((t, _)) = form.versions.iter().find(|(t, _)| t.name == *v)
                    && ui.button(tr!("Export for {v}", v)).clicked()
                {
                    pick = Some(t.clone());
                }
            }
            if !m.loaded {
                say(ui, tr!("Massif has not read the mod of “{namespace}”: its blocks are written by their ids, and only a game with that mod will have them. Add it in Blocks ▸ Mods to check them.", namespace = m.namespace));
            }
            if !m.unknown.is_empty() {
                let first: Vec<&str> = m.unknown.iter().take(3).map(String::as_str).collect();
                say(ui, trn!("{n} state is none of the game's, by the mod's list: {first}", "{n} states are none of the game's, by the mod's list: {first}", n = m.unknown.len(), first = first.join("; ")));
            }
        }
    });
    if let Some(t) = pick {
        form.target = t;
    }
}

fn export_general(ui: &mut Ui, form: &mut ExportForm) {
    let pal = Palette::of(ui);
    card(ui, tr!("Where to save it"), tr!("Its folder, and its name in Minecraft."), |ui| {
        row(ui, tr!("Saves folder"), |ui| {
            ui.add(egui::Label::new(RichText::new(form.parent.display().to_string()).size(11.5)).truncate());
            if ui.button(theme::glyph(Icon::FolderOpen).to_string()).on_hover_text(tr!("Choose the folder")).clicked()
                && let Some(dir) = rfd::FileDialog::new().set_directory(&form.parent).pick_folder()
            {
                form.parent = dir;
            }
        });
        row(ui, tr!("Name"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.name).desired_width(f32::INFINITY));
        });
        row(ui, tr!("Minecraft"), |ui| {
            crate::widgets::combo("export version")
                .selected_text(tr!("Java {name}", name = form.target.name))
                .width(170.0)
                .height(320.0)
                .show_ui(ui, |ui| {
                    for (version, installed) in &form.versions {
                        let label = if *installed { tr!("{name}  ·  installed", name = version.name) } else { version.name.clone() };
                        if ui.selectable_label(form.target == *version, label).clicked() {
                            form.target = version.clone();
                        }
                    }
                });
            let (bottom, top) = form.heights;
            let note = match form.target.era() {
                massif_anvil::Era::Modern if bottom < -64 || top > 320 => tr!("today's chunks, Y {bottom} to {top}: a data pack gives the world that height", bottom, top),
                massif_anvil::Era::Modern => tr!("today's chunks, Y {bottom} to {top}", bottom, top),
                massif_anvil::Era::Flat16 | massif_anvil::Era::Flat15 | massif_anvil::Era::Flat14 => tr!("named blocks, 0 to 255").to_owned(),
                massif_anvil::Era::Numbered => tr!("numbered blocks, 0 to 255").to_owned(),
            };
            ui.label(RichText::new(note).size(11.5).color(pal.weak));
        });
        let caveat = form.target.caveat(form.heights.0, form.heights.1);
        if !caveat.is_empty() {
            row(ui, "", |ui| {
                ui.add(egui::Label::new(RichText::new(tr!("For {name}, {caveat}.", name = form.target.name, caveat)).size(11.5).color(pal.weak)).wrap());
            });
        }
    });
    if !form.mods.is_empty() {
        export_mods(ui, form);
    }
    ui.columns(2, |cols| {
        let (left, right) = cols.split_at_mut(1);
        card(&mut left[0], tr!("Game"), tr!("The rules of the world when it is first played."), |ui| {
            row(ui, tr!("Game mode"), |ui| {
                let mode = |m: GameMode| match m {
                    GameMode::Survival => tr!("Survival"),
                    GameMode::Creative => tr!("Creative"),
                    GameMode::Adventure => tr!("Adventure"),
                    GameMode::Spectator => tr!("Spectator"),
                };
                crate::widgets::combo("game mode")
                    .selected_text(mode(form.game_mode))
                    .show_ui(ui, |ui| {
                        for m in [GameMode::Survival, GameMode::Creative, GameMode::Adventure, GameMode::Spectator] {
                            ui.selectable_value(&mut form.game_mode, m, mode(m));
                        }
                    });
            });
            row(ui, tr!("Difficulty"), |ui| {
                use massif_anvil::Difficulty;
                let name = |d: Difficulty| match d {
                    Difficulty::Peaceful => tr!("Peaceful"),
                    Difficulty::Easy => tr!("Easy"),
                    Difficulty::Normal => tr!("Normal"),
                    Difficulty::Hard => tr!("Hard"),
                };
                crate::widgets::combo("difficulty")
                    .selected_text(name(form.difficulty))
                    .show_ui(ui, |ui| {
                        for d in [Difficulty::Peaceful, Difficulty::Easy, Difficulty::Normal, Difficulty::Hard] {
                            ui.selectable_value(&mut form.difficulty, d, name(d));
                        }
                    });
            });
            row(ui, "", |ui| {
                crate::widgets::checkbox(ui, &mut form.allow_cheats, tr!("Allow cheats"));
            });
            row(ui, "", |ui| {
                crate::widgets::checkbox(ui, &mut form.hardcore, tr!("Hardcore"));
            });
            row(ui, "", |ui| {
                crate::widgets::checkbox(ui, &mut form.structures, tr!("Structures beyond the map")).on_hover_text(tr!("Minecraft generates villages, strongholds and the like in the land it makes around the map"));
            });
            if form.has_selection {
                row(ui, "", |ui| {
                    crate::widgets::checkbox(ui, &mut form.selection_only, tr!("Only the selection")).on_hover_text(tr!("Export only the tiles of 128 × 128 blocks the selection reaches, to try a part of the map in the game"));
                });
            }
        });
        card(&mut right[0], tr!("World border"), tr!("The border Minecraft shows around the world."), |ui| {
            row(ui, tr!("Centre"), |ui| {
                crate::widgets::drag(ui, egui::DragValue::new(&mut form.border.center[0]).speed(16.0));
                crate::widgets::drag(ui, egui::DragValue::new(&mut form.border.center[1]).speed(16.0));
            });
            row(ui, tr!("Size"), |ui| {
                crate::widgets::drag(ui, egui::DragValue::new(&mut form.border.size).range(16.0..=60_000_000.0).speed(64.0).suffix(tr!(" blocks")));
            });
            row(ui, "", |ui| {
                if ui.small_button(tr!("Fit the map")).on_hover_text(tr!("Centre the border on the map, just large enough for it")).clicked()
                    && let Some((c, size)) = form.fit
                {
                    form.border.center = c;
                    form.border.size = size;
                }
            });
        });
    });
    card(ui, tr!("Data packs"), tr!("Data packs copied into the world."), |ui| {
        ui.horizontal_top(|ui| {
            egui::Frame::new().stroke(Stroke::new(1.0, pal.border)).corner_radius(theme::TILE_RADIUS).inner_margin(6).show(ui, |ui| {
                ui.set_min_size(vec2(ui.available_width() - 40.0, 90.0));
                if form.datapacks.is_empty() {
                    ui.label(RichText::new(tr!("No data packs")).color(pal.weak));
                }
                for (i, p) in form.datapacks.iter().enumerate() {
                    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                    if ui.selectable_label(form.selected_pack == Some(i), name).on_hover_text(p.display().to_string()).clicked() {
                        form.selected_pack = Some(i);
                    }
                }
            });
            ui.vertical(|ui| {
                if ui.button(theme::glyph(Icon::Plus).to_string()).on_hover_text(tr!("Add data packs (zip files)")).clicked()
                    && let Some(files) = rfd::FileDialog::new().add_filter(tr!("Data packs"), &["zip"]).pick_files()
                {
                    form.datapacks.extend(files);
                }
                if ui.button(theme::glyph(Icon::FolderOpen).to_string()).on_hover_text(tr!("Add a data pack folder")).clicked()
                    && let Some(dir) = rfd::FileDialog::new().pick_folder()
                {
                    form.datapacks.push(dir);
                }
                if ui
                    .add_enabled(form.selected_pack.is_some(), egui::Button::new(theme::glyph(Icon::Trash2).to_string()))
                    .on_hover_text(tr!("Remove"))
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
    card(ui, tr!("Around the map"), tr!("What Minecraft generates beyond the edges of the map."), |ui| {
        ui.radio_value(&mut form.surroundings, Surroundings::Vanilla, tr!("Vanilla terrain from the world's seed"));
        ui.radio_value(&mut form.surroundings, Surroundings::Void, tr!("Nothing: the map floats in the void"));
    });
    card(ui, tr!("Ores"), tr!("Laid in the rock as Minecraft lays them: Minecraft adds none to the map's finished chunks."), |ui| {
        row(ui, tr!("Lay ores"), |ui| {
            crate::widgets::switch(ui, &mut form.ores);
        });
        ui.label(RichText::new(tr!("Where, how many and how large: Edit ▸ Ores…")).size(11.5).color(pal.weak));
    });
    card(ui, tr!("Spawn point"), tr!("Where players appear the first time."), |ui| {
        row(ui, "x, y, z", |ui| {
            crate::widgets::drag(ui, egui::DragValue::new(&mut form.spawn[0]));
            crate::widgets::drag(ui, egui::DragValue::new(&mut form.spawn[1]).range(form.heights.0..=form.heights.1 - 1));
            crate::widgets::drag(ui, egui::DragValue::new(&mut form.spawn[2]));
        });
        ui.label(RichText::new(tr!("Spawn point for this export.")).size(11.5).color(pal.weak));
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

/// A corner of an area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Corner {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
}

impl Corner {
    fn left(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::BottomLeft)
    }

    fn top(self) -> bool {
        matches!(self, Corner::TopLeft | Corner::TopRight)
    }
}

/// A corner of the area `rect`, as in Blender: dragged into the area, it
/// splits it where it is let go, the new area on the corner's side (across:
/// left or right; up or down: above or below). Nothing shows there until
/// the pointer is on it: then the corner lights up, with a cross for a
/// cursor, and while dragging, the line of the split and the new area's side.
/// Returns the split asked for: its direction, whether the new area comes
/// first, where (from 0 to 1).
fn corner_split(ui: &Ui, painter: &egui::Painter, rect: Rect, id: AreaId, corner: Corner) -> Option<(Dir, bool, f32)> {
    use std::f32::consts::{FRAC_PI_2, PI};
    const SIDE: f32 = 10.0;
    let pal = Palette::of(ui);
    let zone = Rect::from_min_size(
        pos2(if corner.left() { rect.left() } else { rect.right() - SIDE }, if corner.top() { rect.top() } else { rect.bottom() - SIDE }),
        vec2(SIDE, SIDE),
    );
    let handle = ui.interact(zone, Id::unique(("area corner", id, corner as u8)), Sense::drag());
    if handle.hovered() || handle.dragged() {
        // The quarter of the card's rounded corner.
        let r = theme::CARD_RADIUS as f32;
        let c = pos2(if corner.left() { rect.left() + r } else { rect.right() - r }, if corner.top() { rect.top() + r } else { rect.bottom() - r });
        let start = match corner {
            Corner::TopRight => -FRAC_PI_2,
            Corner::BottomRight => 0.0,
            Corner::BottomLeft => FRAC_PI_2,
            Corner::TopLeft => PI,
        };
        let arc: Vec<egui::Pos2> = (0..=8).map(|k| c + egui::Vec2::angled(start + k as f32 / 8.0 * FRAC_PI_2) * (r - 1.0)).collect();
        painter.add(egui::Shape::line(arc, Stroke::new(2.0, pal.accent)));
        ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
    }
    let split = |p: egui::Pos2| -> Option<(Dir, bool, f32)> {
        let d = p - zone.center();
        if d.length() <= 12.0 || !rect.contains(p) {
            return None;
        }
        Some(if d.x.abs() > d.y.abs() {
            (Dir::Row, corner.left(), (p.x - rect.left()) / rect.width())
        } else {
            (Dir::Column, corner.top(), (p.y - rect.top()) / rect.height())
        })
    };
    let pointer = ui.input(|i| i.pointer.interact_pos());
    if handle.dragged()
        && let Some(p) = pointer
        && let Some((dir, first, _)) = split(p)
    {
        let fg = ui.ctx().layer_painter(LayerId::new(Order::Foreground, Id::unique(("corner preview", id))));
        let (line, side) = match dir {
            Dir::Row => (
                [pos2(p.x, rect.top()), pos2(p.x, rect.bottom())],
                if first { Rect::from_min_max(rect.min, pos2(p.x, rect.bottom())) } else { Rect::from_min_max(pos2(p.x, rect.top()), rect.max) },
            ),
            Dir::Column => (
                [pos2(rect.left(), p.y), pos2(rect.right(), p.y)],
                if first { Rect::from_min_max(rect.min, pos2(rect.right(), p.y)) } else { Rect::from_min_max(pos2(rect.left(), p.y), rect.max) },
            ),
        };
        fg.rect_filled(side, 0, pal.accent.gamma_multiply(0.08));
        fg.line_segment(line, Stroke::new(2.0, pal.accent));
    }
    if handle.drag_stopped() { pointer.and_then(split) } else { None }
}

/// Paint `outside` over the corners of `rect` past a rounding of `radius`:
/// what was drawn square there is cut round.
fn round_off(painter: &egui::Painter, rect: Rect, radius: f32, outside: Color32) {
    const STEPS: u32 = 10;
    let mut mesh = egui::Mesh::default();
    for (corner, sx, sy) in [(rect.left_top(), 1.0, 1.0), (rect.right_top(), -1.0, 1.0), (rect.right_bottom(), -1.0, -1.0), (rect.left_bottom(), 1.0, -1.0)] {
        let centre = corner + vec2(sx, sy) * radius;
        let base = mesh.vertices.len() as u32;
        mesh.colored_vertex(corner, outside);
        for k in 0..=STEPS {
            let a = k as f32 / STEPS as f32 * std::f32::consts::FRAC_PI_2;
            mesh.colored_vertex(centre - vec2(sx * a.sin(), sy * a.cos()) * radius, outside);
        }
        for k in 0..STEPS {
            mesh.add_triangle(base, base + 1 + k, base + 2 + k);
        }
    }
    painter.add(egui::Shape::mesh(mesh));
}

/// What of the covers and object layers shows on the map and in 3D.
fn covers_key(reg: &massif_core::Registry) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for c in &reg.covers {
        (c.color, c.density.to_bits(), c.removed, c.hidden, c.soil_only, c.underwater, c.height, &c.plants).hash(&mut h);
    }
    for l in &reg.object_layers {
        (l.spacing, l.density.to_bits(), l.rotate, l.mirror, l.soil_only, l.underwater, l.sink, l.replace, l.removed, l.hidden, &l.objects).hash(&mut h);
    }
    for o in &reg.objects {
        (o.size, o.anchor, &o.colors).hash(&mut h);
    }
    h.finish()
}

#[cfg(test)]
mod tests {
    thread_local! {
        /// Make the next frame panic, as a bug would.
        pub static PANIC_NEXT_FRAME: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    }

    /// A panic in a frame: the map is backed up at once, the error said in
    /// a dialog, and frames go on. Quitting from it keeps the copy, which
    /// the next start offers back.
    #[test]
    fn a_panic_in_a_frame_keeps_the_map() {
        let dir = std::env::temp_dir().join(format!("massif-panic-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(3);
        h.state_mut().autosave = crate::autosave::Autosave::open(dir.clone());
        h.state_mut().doc.world.meta.name = "Before the error".into();
        h.state_mut().doc.mark_changed();
        PANIC_NEXT_FRAME.set(true);
        h.run_steps(3);
        let copy = match &h.state().dialog {
            Some(Dialog::Crashed(said, Some(copy))) => {
                assert!(said.contains("an error made by a test"), "{said}");
                copy.clone()
            }
            _ => panic!("no dialog with a copy after the error"),
        };
        assert_eq!(massif_format::load(&copy).unwrap().meta.name, "Before the error");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("dialog-crashed.png")).unwrap();
        }
        // Frames go on; quitting is not a clean close, the copy stays.
        h.run_steps(5);
        h.state_mut().autosave.keep();
        h.state_mut().autosave.close();
        assert!(copy.exists());
        let next = crate::autosave::Autosave::open(dir.clone());
        assert_eq!(next.found.len(), 1);
        assert_eq!(next.found[0].file, copy);
        drop(next);
        let _ = std::fs::remove_dir_all(&dir);
    }

    use super::*;
    use egui::{Key, Modifiers};
    use crate::home::parse_seed;
    use massif_core::generate::Landform;

    /// The brush over the map, close up, for the eye (MASSIF_SNAPSHOT_DIR):
    /// a square turned 45° and one that is not, the pointer between blocks,
    /// painting, painting a noise and flattening.
    #[test]
    fn brush_over_the_map() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        // A coast: where water meets land east of the middle.
        let (cx, cz) = {
            let doc = &h.state().doc;
            let mut last = None;
            let mut at = (0, 0);
            for x in 0..512 {
                if let Some(p) = crate::tools::probe(doc, x, 0) {
                    let wet = p.fluid != massif_core::NO_FLUID;
                    if last.is_some_and(|l| l != wet) {
                        at = (x, 0);
                        break;
                    }
                    last = Some(wet);
                }
            }
            at
        };
        let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
        {
            let view = h.state_mut().maps.get_mut(&id).unwrap();
            view.center = [cx as f64 + 0.5, cz as f64 + 0.5];
            view.zoom = 9.0;
        }
        h.run_steps(5);
        use massif_core::{BrushShape, Profile};
        let cases: [(&str, Tool, BrushShape, Profile, f32, bool, f32); 8] = [
            ("paint-square45", Tool::PaintTerrain, BrushShape::Square, Profile::Constant, 45.0, false, 9.0),
            ("paint-square", Tool::PaintTerrain, BrushShape::Square, Profile::Constant, 0.0, false, 9.0),
            ("paint-square-sine", Tool::PaintTerrain, BrushShape::Square, Profile::Sine, 0.0, false, 9.0),
            ("paint-square30-sine", Tool::PaintTerrain, BrushShape::Square, Profile::Sine, 30.0, false, 9.0),
            ("paint-circle-sine", Tool::PaintTerrain, BrushShape::Circle, Profile::Sine, 0.0, false, 9.0),
            ("paint-square-sine-far", Tool::PaintTerrain, BrushShape::Square, Profile::Sine, 0.0, false, 2.0),
            ("noise-square45", Tool::PaintTerrain, BrushShape::Square, Profile::Constant, 45.0, true, 9.0),
            ("flatten-square45", Tool::Flatten, BrushShape::Square, Profile::Constant, 45.0, false, 9.0),
        ];
        for (name, tool, shape, profile, rotation, noise, zoom) in cases {
            {
                let app = h.state_mut();
                app.maps.get_mut(&id).unwrap().zoom = zoom;
                let tools = &mut app.tools;
                tools.select(tool);
                tools.brush.shape = shape;
                tools.brush.profile = profile;
                tools.brush.radius = 14.0;
                tools.brush.rotation = rotation;
                tools.pattern.on = noise;
            }
            h.run_steps(2);
            let p = h.state().maps[&id].screen_of(cx as f64 + 0.37, cz as f64 + 0.61);
            h.event(egui::Event::PointerMoved(p));
            h.run_steps(4);
            let image = h.render().expect("rendered");
            let half = (14.0 * 1.5 * zoom) as u32 + 24;
            let (x0, y0) = ((p.x as u32).saturating_sub(half), (p.y as u32).saturating_sub(half));
            let crop = image::imageops::crop_imm(&image, x0, y0, 2 * half, 2 * half).to_image();
            crop.save(dir.join(format!("brush-{name}.png"))).unwrap();
        }
        // The line under the map: the column under the pointer, in whole
        // blocks, with its biome.
        let image = h.render().expect("rendered");
        let map = map_rect(h.state(), &h.ctx);
        let footer = image::imageops::crop_imm(&image, map.left() as u32, (map.bottom() - 30.0) as u32, map.width() as u32, 30).to_image();
        footer.save(dir.join("brush-footer.png")).unwrap();
    }

    /// A backup left by a run that crashed: offered at the start, restored
    /// as the map it was (its name, its file, changes to save), then
    /// removed once the map is saved (a render of the dialog with
    /// MASSIF_SNAPSHOT_DIR).
    /// A problem report: from the Help menu, its files shown, saved with
    /// paths made relative to the home folder and the map only when ticked;
    /// a performance recording times each frame's parts; from the dialog of
    /// a start after a crash and back to it.
    #[test]
    fn a_problem_report() {
        use crate::report::{Recording, unzip};
        use egui_kittest::kittest::Queryable;
        let dir = std::env::temp_dir().join(format!("massif-report-app-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let home = dir.join("home").join("tester");
        let logs = home.join("Library").join("Logs").join("Massif");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("startup.log"), format!("[      0 ms] Massif\n[     12 ms] asked to open {}/Maps/Isles.massif\n[   2210 ms] frame 1 shown\n", home.display())).unwrap();
        std::fs::write(logs.join("startup.previous.log"), "[      0 ms] Massif\n[   1500 ms] panic in thread main: index out of bounds\n").unwrap();
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(30);
        let ctx = h.ctx.clone();
        h.state_mut().command(&ctx, crate::menu::Command::ReportProblem);
        let Some(Dialog::Report(form)) = &mut h.state_mut().dialog else { panic!("the report's dialog opens") };
        // This computer's own logs stay out of the test.
        form.inputs.log_dir = Some(logs.clone());
        form.inputs.home = Some(home.clone());
        form.inputs.what_happened = "The 3D view stutters when I sculpt".into();
        assert!(!form.inputs.last_minute.frames.is_empty(), "the frames before it opened are taken");
        assert!(form.inputs.map.iter().any(|(k, v)| k == "Size" && v == "1024 × 1024 blocks"), "{:?}", form.inputs.map);
        form.shown = Some("startup.log".into());
        h.run_steps(4);
        h.get_by_label("startup.previous.log");
        h.get_by_label_contains("asked to open ~/Maps/Isles.massif");
        if let Some(out) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(out.join("dialog-report.png")).unwrap();
        }
        let saved = |h: &mut egui_kittest::Harness<'static, MassifApp>, map: bool, name: &str| {
            let Some(Dialog::Report(form)) = &h.state().dialog else { panic!("the report's dialog is open") };
            let mut form = form.clone();
            form.include_map = map;
            let path = dir.join(name);
            let ctx = h.ctx.clone();
            h.state_mut().save_report(&ctx, &form, path.clone());
            let started = std::time::Instant::now();
            while h.state().report_saving.is_some() {
                assert!(started.elapsed().as_secs() < 30, "the report is written");
                std::thread::sleep(std::time::Duration::from_millis(5));
                h.state_mut().report_frame(&ctx);
            }
            unzip(&std::fs::read(&path).unwrap())
        };
        let files = saved(&mut h, false, "without.zip");
        let names: Vec<&str> = files.iter().map(|f| f.0.as_str()).collect();
        assert_eq!(names, ["report.txt", "startup.log", "startup.previous.log", "last-minute.csv"]);
        for (name, bytes) in &files {
            let text = String::from_utf8_lossy(bytes);
            assert!(!text.contains(&*home.to_string_lossy()) && !text.contains("tester"), "{name} keeps the user's folder: {text}");
        }
        let report = String::from_utf8_lossy(&files[0].1);
        assert!(report.contains("The 3D view stutters when I sculpt") && report.contains("Reduce motion:") && report.contains("Graphics: ") && report.contains("Levels: "), "{report}");
        let files = saved(&mut h, true, "with.zip");
        let (name, bytes) = files.last().unwrap();
        assert_eq!(name, crate::report::MAP_NAME, "the map goes in once ticked");
        assert_eq!(massif_format::read(bytes).unwrap().meta.name, h.state().doc.world.meta.name);

        // A recording: each frame and its parts.
        h.state_mut().recording = Some(Recording::start());
        h.run_steps(12);
        let r = h.state_mut().recording.as_mut().unwrap();
        r.stop();
        assert!(r.frames() >= 10, "{} frames", r.frames());
        let summary = r.summary();
        assert!(summary.contains("the theme and the accessibility settings"), "{summary}");
        let Some(Dialog::Report(form)) = &h.state().dialog else { panic!() };
        let form = form.clone();
        let parts = crate::report::parts(&form.inputs, h.state().recording.as_ref(), None);
        assert!(parts.iter().any(|p| p.name == "recording/trace.json"));
        h.state_mut().dialog = None;
        // Its pill, over the map.
        h.state_mut().recording = Some(Recording::start());
        h.run_steps(5);
        h.get_by_label_contains("Recording 0:");
        if let Some(out) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(out.join("report-recording.png")).unwrap();
        }
        h.get_by_label_contains("Stop").click();
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Report(_))), "Stop opens the report");
        assert!(h.state().recording.as_ref().is_some_and(|r| !r.is_on()));
        h.state_mut().dialog = None;
        h.state_mut().recording = None;

        // After a crash: from the dialog that offers the backup, and back.
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        world.meta.name = "Lost".into();
        let info = crate::autosave::Info { session: "1-1".into(), generation: 1, name: "Lost".into(), path: None, taken: 0, revision: 3 };
        let backups = dir.join("backups");
        std::fs::create_dir_all(&backups).unwrap();
        crate::autosave::write(&backups, world.copy_unfrozen(), info).unwrap();
        h.state_mut().autosave = crate::autosave::Autosave::open(backups);
        h.state_mut().dialog = Some(Dialog::Recover);
        h.run_steps(3);
        h.get_by_label_contains("Report the Problem").click();
        h.run_steps(3);
        assert!(matches!(&h.state().dialog, Some(Dialog::Report(f)) if f.after_crash));
        h.get_by_label_contains("closed without warning");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Recover)), "back to the backup");
        // After an error in a frame: from its dialog, and back.
        h.state_mut().dialog = Some(Dialog::Crashed("boom".into(), None));
        h.run_steps(3);
        h.get_by_label_contains("Report the Problem").click();
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Report(_))));
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(matches!(&h.state().dialog, Some(Dialog::Crashed(said, None)) if said == "boom"), "back to the error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn restoring_a_backup_left_by_a_crash() {
        use crate::autosave::{Autosave, Info};
        use egui_kittest::kittest::Queryable;
        let dir = std::env::temp_dir().join(format!("massif-recover-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut world = generate(&GenParams { tiles_x: 4, tiles_z: 3, landform: Landform::Mountains, ..GenParams::default() });
        world.meta.name = "Northern Isles".into();
        let file = dir.join("Northern Isles.massif");
        let taken = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() - 7 * 60;
        let info = Info { session: "1-1".into(), generation: 4, name: world.meta.name.clone(), path: Some(file.clone()), taken, revision: 12 };
        let backup = crate::autosave::write(&dir, world.copy_unfrozen(), info).unwrap();
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(3);
        h.state_mut().autosave = Autosave::open(dir.clone());
        assert_eq!(h.state().autosave.found, std::slice::from_ref(&backup), "a backup whose run is over is offered");
        h.state_mut().dialog = Some(Dialog::Recover);
        h.run_steps(4);
        if let Some(out) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(out.join("dialog-recover.png")).unwrap();
        }
        h.get_by_label("Restore").click();
        h.run_steps(2);
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        {
            let doc = &h.state().doc;
            assert_eq!(doc.world.meta.name, "Northern Isles");
            assert_eq!(doc.path.as_deref(), Some(file.as_path()));
            assert!(doc.is_modified(), "restored, not saved");
        }
        assert!(backup.file.exists(), "kept until the map is saved");
        // Saved to its file: the backup goes.
        let ctx = h.ctx.clone();
        assert!(h.state_mut().save(&ctx, false));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!h.state().doc.is_modified() && file.exists());
        assert!(!backup.file.exists(), "a save removes the backup");
        h.state_mut().autosave.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A newer version out (GitHub replaced by a fixture): a pill in the top
    /// bar and a line on the home screen open its notes; Skip this version
    /// hides it, the Settings switch too. Renders the pill, the line and
    /// the dialog with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn a_newer_version_is_told_of() {
        use crate::update::{Updates, tests::fixture};
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        assert!(h.query_by_label_contains("is out").is_none(), "nothing known, nothing said");
        h.state_mut().updates = Updates::known(fixture());
        h.run_steps(3);
        let out = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let ppp = h.ctx.pixels_per_point();
        if let Some(dir) = &out {
            let image = h.render().unwrap();
            let crop = image::imageops::crop_imm(&image, image.width() - (520.0 * ppp) as u32, 0, (520.0 * ppp) as u32, (44.0 * ppp) as u32).to_image();
            crop.save(dir.join("update-pill.png")).unwrap();
        }
        h.get_by_label_contains("9.1.0 is out").click();
        h.run_steps(4);
        assert!(matches!(h.state().dialog, Some(Dialog::Update)));
        assert!(h.query_by_label_contains("Rivers that find their way").is_some(), "the notes since this version");
        assert!(h.query_by_label_contains("Old news").is_none(), "not those of versions one has");
        if let Some(dir) = &out {
            h.render().unwrap().save(dir.join("update-dialog.png")).unwrap();
        }
        h.get_by_label("Later").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        assert!(h.query_by_label_contains("9.1.0 is out").is_some(), "Later keeps the pill");
        // On the home screen, above Settings.
        h.state_mut().open_home(Page::NewMap);
        h.run_steps(4);
        if let Some(dir) = &out {
            let image = h.render().unwrap();
            let crop = image::imageops::crop_imm(&image, 0, image.height() - (180.0 * ppp) as u32, (300.0 * ppp) as u32, (180.0 * ppp) as u32).to_image();
            crop.save(dir.join("update-home.png")).unwrap();
        }
        h.get_by_label("Massif 9.1.0 is out").click();
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Update)));
        h.get_by_label("Skip this version").click();
        h.run_steps(3);
        assert_eq!(h.state().settings.skipped_version.as_deref(), Some("9.1.0"));
        assert!(h.state().dialog.is_none() && h.query_by_label_contains("is out").is_none(), "skipped, not told of again");
        // Told again from Settings; the switch off says nothing.
        h.state_mut().settings.skipped_version = None;
        h.state_mut().settings.check_updates = false;
        h.run_steps(3);
        assert!(h.query_by_label_contains("is out").is_none());
        h.state_mut().settings.check_updates = true;
        h.run_steps(3);
        assert!(h.query_by_label_contains("is out").is_some());
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Updates));
        h.run_steps(4);
        assert!(h.query_by_label_contains("Massif 9.1.0 is out. You have").is_some());
        if let Some(dir) = &out {
            h.render().unwrap().save(dir.join("update-settings.png")).unwrap();
        }
        h.get_by_label("See what is new").click();
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Update)), "the notes, from Settings");
    }

    /// Flatten's options in Tool Settings: the way it moves the land, picked
    /// in a segmented control, and Apply theme ticked (a render of the panel
    /// with MASSIF_SNAPSHOT_DIR).
    /// Drag a rectangle on the map, add an ellipse with Shift, and see the
    /// outline and the options; a raise stroke across its edge changes only
    /// what is inside. Renders with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn selections_drawn_on_the_map() {
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(30);
        h.state_mut().tools.select(Tool::SelectRect);
        h.run_steps(3);
        let rect = map_rect(h.state(), &h.ctx);
        let drag = |h: &mut egui_kittest::Harness<'static, MassifApp>, from: egui::Pos2, to: egui::Pos2, modifiers: Modifiers| {
            let button = |pressed, pos| egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers,
            };
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(button(true, from));
            h.step();
            for i in 1..=8 {
                h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 8.0)));
                h.step();
            }
            h.event(button(false, to));
            h.step();
            h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
            h.run_steps(4);
        };
        let c = rect.center();
        drag(&mut h, c - vec2(150.0, 90.0), c + vec2(20.0, 60.0), Modifiers::NONE);
        assert_eq!(h.state().doc.history.undo_label(), Some("Select"));
        let first = h.state().doc.selection().unwrap().area();
        h.state_mut().tools.select(Tool::SelectEllipse);
        h.run_steps(2);
        drag(&mut h, c - vec2(10.0, 40.0), c + vec2(140.0, 110.0), Modifiers::SHIFT);
        assert_eq!(h.state().doc.history.undo_label(), Some("Add to selection"));
        assert!(h.state().doc.selection().unwrap().area() > first);
        h.run_steps(4);
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("selection-map.png")).unwrap();
            // Tool Settings, maximised: every option of the tool.
            let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
            let id = areas.iter().map(|(id, _)| *id).find(|id| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::ToolSettings)).unwrap();
            h.state_mut().layout.toggle_maximize(id);
            h.run_steps(4);
            let image = h.render().unwrap();
            let ppp = h.ctx.pixels_per_point();
            let crop = image::imageops::crop_imm(&image, 0, 0, image.width(), (640.0 * ppp) as u32).to_image();
            crop.save(dir.join("tool-settings-selection.png")).unwrap();
            h.state_mut().layout.toggle_maximize(id);
            h.run_steps(4);
        }
        // A raise across its edge: the land outside stays as it was.
        let sel = h.state().doc.selection().cloned().unwrap();
        let outside = (-300..300).step_by(7).flat_map(|x| (-300..300).step_by(7).map(move |z| (x, z))).find(|&(x, z)| sel.get(x, z) == 0 && sel.get(x + 40, z) == 255).expect("an edge");
        let before = massif_core::ops::height_at(h.state().doc.dimension(), 0, outside.0, outside.1, massif_core::ops::Surface::Top);
        h.state_mut().tools.select(Tool::Raise);
        h.run_steps(2);
        let at = h.state().maps.values().next().unwrap().screen_of(outside.0 as f64 + 20.0, outside.1 as f64);
        drag(&mut h, at, at + vec2(2.0, 0.0), Modifiers::NONE);
        assert_eq!(massif_core::ops::height_at(h.state().doc.dimension(), 0, outside.0, outside.1, massif_core::ops::Surface::Top), before);
        // ⌘D deselects; ⌘Z brings it back.
        h.key_press_modifiers(Modifiers::COMMAND, egui::Key::D);
        h.run_steps(2);
        assert!(h.state().doc.selection().is_none());
    }

    #[test]
    fn flatten_options_in_tool_settings() {
        use egui_kittest::kittest::Queryable;
        use massif_core::ops::FlattenMode;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        h.state_mut().tools.select(Tool::Flatten);
        h.run_steps(3);
        assert_eq!(h.state().tools.flatten_mode, FlattenMode::Both);
        assert!(!h.state().tools.flatten_theme);
        h.get_by_label("Only raise").click();
        h.run_steps(2);
        assert_eq!(h.state().tools.flatten_mode, FlattenMode::RaiseOnly);
        h.get_by_label("Only lower").click();
        h.run_steps(2);
        assert_eq!(h.state().tools.flatten_mode, FlattenMode::LowerOnly);
        h.get_by_label("Apply theme").click();
        h.run_steps(2);
        assert!(h.state().tools.flatten_theme);
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            let image = h.render().unwrap();
            let rect = h.get_by_label("Apply theme").rect();
            let ppp = h.ctx.pixels_per_point();
            let (x0, y0) = (((rect.left() - 40.0).max(0.0) * ppp) as u32, ((rect.top() - 220.0).max(0.0) * ppp) as u32);
            let crop = image::imageops::crop_imm(&image, x0, y0, (360.0 * ppp) as u32, (300.0 * ppp) as u32).to_image();
            crop.save(dir.join("tool-settings-flatten.png")).unwrap();
        }
        h.get_by_label("Both ways").click();
        h.run_steps(2);
        assert_eq!(h.state().tools.flatten_mode, FlattenMode::Both);
    }

    /// Flood's From the click, on by default, turned off in Tool Settings
    /// (rendered for the eye with MASSIF_SNAPSHOT_DIR).
    #[test]
    fn flood_from_the_click_in_tool_settings() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        h.state_mut().tools.select(Tool::Flood);
        h.run_steps(3);
        assert!(h.state().tools.water_from_click);
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            let image = h.render().unwrap();
            let rect = h.get_by_label("From the click").rect();
            let ppp = h.ctx.pixels_per_point();
            let (x0, y0) = (((rect.left() - 40.0).max(0.0) * ppp) as u32, ((rect.top() - 60.0).max(0.0) * ppp) as u32);
            let crop = image::imageops::crop_imm(&image, x0, y0, (360.0 * ppp) as u32, (200.0 * ppp) as u32).to_image();
            crop.save(dir.join("tool-settings-flood.png")).unwrap();
        }
        h.get_by_label("From the click").click();
        h.run_steps(2);
        assert!(!h.state().tools.water_from_click);
        // The limit, off by default, turned on.
        assert!(!h.state().tools.water_limit_on);
        h.get_by_label("Limit the water's height").click();
        h.run_steps(3);
        assert!(h.state().tools.water_limit_on);
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            let image = h.render().unwrap();
            let rect = h.get_by_label("From the click").rect();
            let ppp = h.ctx.pixels_per_point();
            let (x0, y0) = (((rect.left() - 40.0).max(0.0) * ppp) as u32, ((rect.top() - 60.0).max(0.0) * ppp) as u32);
            let crop = image::imageops::crop_imm(&image, x0, y0, (360.0 * ppp) as u32, (260.0 * ppp) as u32).to_image();
            crop.save(dir.join("tool-settings-flood-limit.png")).unwrap();
        }
    }

    /// The menu bar, the pointer over View, for the eye (MASSIF_SNAPSHOT_DIR);
    /// and its menus hold their hit as wide as their padding.
    #[test]
    fn menu_bar_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        let view = h.get_by_label("View").rect();
        assert!(view.width() >= 40.0 && view.height() >= 22.0, "View's hit is {view:?}");
        h.event(egui::Event::PointerMoved(view.center()));
        h.run_steps(3);
        let image = h.render().expect("rendered");
        let crop = image::imageops::crop_imm(&image, 0, 0, 420, 44).to_image();
        crop.save(dir.join("menu-bar.png")).unwrap();
        // File and Edit open, their shortcuts beside their items.
        add_tower(&mut h);
        for (menu, height) in [("File", 380), ("Edit", 380)] {
            h.get_by_role_and_label(egui::accesskit::Role::Button, menu).click();
            h.run_steps(3);
            let image = h.render().expect("rendered");
            let crop = image::imageops::crop_imm(&image, 0, 0, 460, height).to_image();
            crop.save(dir.join(format!("menu-{}.png", menu.to_lowercase()))).unwrap();
            h.key_press(Key::Escape);
            h.run_steps(2);
        }
        // As on a Mac: the window's buttons at the left, the menus in the
        // system's menu bar.
        h.state_mut().unified_titlebar = true;
        h.state_mut().menus_in_menu_bar = true;
        h.run_steps(3);
        let image = h.render().expect("rendered");
        let crop = image::imageops::crop_imm(&image, 0, 0, 640, 48).to_image();
        crop.save(dir.join("menu-bar-mac.png")).unwrap();
    }

    /// On a Mac the system's menu bar holds File, Edit, View and Help, and
    /// the top bar no longer does; elsewhere it keeps them.
    #[test]
    fn the_mac_menu_bar_takes_the_menus_out_of_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        let menus = |h: &egui_kittest::Harness<'_, MassifApp>| -> Vec<&str> {
            ["File", "Edit", "View", "Help"].into_iter().filter(|t| h.query_by_role_and_label(egui::accesskit::Role::Button, t).is_some()).collect()
        };
        assert_eq!(menus(&h), ["File", "Edit", "View", "Help"]);
        h.state_mut().menus_in_menu_bar = true;
        h.run_steps(3);
        assert!(menus(&h).is_empty(), "in the window too: {:?}", menus(&h));
        // Their keys still work: ⌘, opens the settings.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::Preferences(_))));
    }

    /// One step to undo: a level added, named "Tower".
    fn add_tower(h: &mut egui_kittest::Harness<'_, MassifApp>) {
        let doc = &mut h.state_mut().doc;
        doc.history.begin("Tower", doc.dim);
        doc.history.edit(&mut doc.world, 0).structure().add_level("Tower", massif_core::level::LevelKind::Solid);
        doc.commit();
    }

    /// The window's menus are drawn from the list macOS's menu bar shows,
    /// and the same commands run whether an item is picked in the window,
    /// in the menu bar (here as the bar hands it over) or by its keys; what
    /// cannot be done then is not done.
    #[test]
    fn the_menus_do_what_they_say() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(3);
        // In the window: View ▸ Light, then Edit ▸ Undo, named after the
        // step it takes back.
        let menu = |h: &egui_kittest::Harness<'_, MassifApp>, title: &str| h.get_by_role_and_label(egui::accesskit::Role::Button, title).click();
        menu(&h, "View");
        h.run_steps(2);
        h.get_by_label("Light").click();
        h.run_steps(2);
        assert_eq!(h.state().settings.theme, ThemeChoice::Light);
        add_tower(&mut h);
        let levels = h.state().doc.dimension().levels.len();
        menu(&h, "Edit");
        h.run_steps(2);
        h.get_by_label_contains("Undo Tower").click();
        h.run_steps(2);
        assert_eq!(h.state().doc.dimension().levels.len(), levels - 1, "the tower is undone");
        // Picked in the menu bar: done at the next frame.
        crate::menubar::pick(Command::About);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::About)));
        // A dialog holds the window: Settings is greyed, and not done.
        crate::menubar::pick(Command::Settings);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::About)));
        h.state_mut().dialog = None;
        h.run_steps(1);
        // The keys the menus show: ⌘, and F1.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Comma);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::Preferences(_))));
        h.state_mut().dialog = None;
        h.key_press(Key::F1);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::Help(_))));
        h.state_mut().dialog = None;
        h.run_steps(1);
        // The home screen: the ways in can be picked, not what acts on the
        // map.
        crate::menubar::pick(Command::HomeScreen);
        h.run_steps(2);
        assert!(h.state().home.is_some());
        crate::menubar::pick(Command::Export);
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "no export from the home screen");
        crate::menubar::pick(Command::FromHeightMap);
        h.run_steps(2);
        assert_eq!(h.state().home.as_ref().map(|home| home.page), Some(Page::FromHeightmap));
        // ⌘N, as New World… does there too.
        h.key_press_modifiers(Modifiers::COMMAND, Key::N);
        h.run_steps(2);
        assert_eq!(h.state().home.as_ref().map(|home| home.page), Some(Page::NewMap));
    }

    /// The system's Quit (on macOS: the Dock's, the app switcher's, a
    /// logout's), here as AppKit asks it, goes the way of the app's own: at
    /// once with nothing to lose; with changes, not now, and the app asks at
    /// its next frame and closes if told to. At a logout the system waits for
    /// the answer: no when the dialog is cancelled or its save fails, yes
    /// once the map is saved or its changes dropped — the system then ends
    /// Massif, which closes nothing itself.
    #[test]
    fn the_systems_quit_asks_first() {
        use crate::quit::{self, Reply};
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(2);
        assert_eq!(quit::ask(false), Reply::Now);
        assert_eq!(quit::ask(true), Reply::Now);
        h.run_steps(2);
        assert!(h.state().dialog.is_none(), "nothing to ask");
        // A change: not now, and asked as Quit asks.
        add_tower(&mut h);
        h.run_steps(1);
        assert_eq!(quit::ask(false), Reply::Cancel);
        h.run_steps(2);
        assert!(matches!(h.state().dialog, Some(Dialog::Unsaved(Pending::Quit))));
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none() && h.state().doc.is_modified() && !h.state().allow_close);
        assert_eq!(quit::ask(false), Reply::Cancel, "still not now");
        h.run_steps(2);
        h.get_by_label("Don't Save").click();
        h.run_steps(2);
        assert!(h.state().allow_close, "closed as by File ▸ Quit");
        assert_eq!(quit::ask(false), Reply::Now, "and nothing left to ask");
        assert_eq!(quit::replied(), None, "nothing waited");

        // At a logout: later.
        h.state_mut().allow_close = false;
        h.run_steps(1);
        assert_eq!(quit::ask(true), Reply::Later);
        h.run_steps(2);
        assert!(quit::waiting() && matches!(h.state().dialog, Some(Dialog::Unsaved(Pending::Quit))));
        // Asked again meanwhile: the question is up already.
        assert_eq!(quit::ask(true), Reply::Cancel);
        h.run_steps(1);
        assert!(quit::waiting() && matches!(h.state().dialog, Some(Dialog::Unsaved(Pending::Quit))));
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert_eq!(quit::replied(), Some(false), "cancelled: told no");
        assert!(!quit::waiting() && h.state().dialog.is_none() && h.state().doc.is_modified());
        // The save runs aside: told once it ends.
        let answered = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            for _ in 0..1000 {
                h.step();
                if let Some(yes) = quit::replied() {
                    return Some(yes);
                }
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            None
        };
        h.state_mut().doc.path = Some(PathBuf::from("/no-such-folder-for-massif/map.massif"));
        assert_eq!(quit::ask(true), Reply::Later);
        h.run_steps(2);
        h.get_by_label("Save").click();
        assert_eq!(answered(&mut h), Some(false), "the save failed: told no");
        assert!(h.state().doc.is_modified());
        let file = std::env::temp_dir().join(format!("massif-quit-{}.massif", std::process::id()));
        h.state_mut().doc.path = Some(file.clone());
        assert_eq!(quit::ask(true), Reply::Later);
        h.run_steps(2);
        h.get_by_label("Save").click();
        let saved = answered(&mut h);
        let _ = std::fs::remove_file(&file);
        assert_eq!(saved, Some(true), "saved: told yes");
        assert!(!h.state().doc.is_modified() && !h.state().allow_close, "ended by the system");
        add_tower(&mut h);
        h.run_steps(1);
        assert_eq!(quit::ask(true), Reply::Later);
        h.run_steps(2);
        h.get_by_label("Don't Save").click();
        h.run_steps(2);
        assert_eq!(quit::replied(), Some(true), "changes dropped: told yes");
        assert!(!h.state().allow_close && !quit::waiting());
    }

    /// The app on a small map of islands (or the .massif MASSIF_HEIGHTS_WORLD
    /// names), tall enough for Brush Settings to show all of itself, once the
    /// heights of the map are read.
    fn heights_harness(theme: ThemeChoice, ppp: f32) -> egui_kittest::Harness<'static, MassifApp> {
        let world = match std::env::var_os("MASSIF_HEIGHTS_WORLD") {
            Some(path) => massif_format::load(std::path::Path::new(&path)).expect("the map opens"),
            None => generate(&GenParams {
                tiles_x: 8,
                tiles_z: 8,
                landform: Landform::Islands,
                ..GenParams::default()
            }),
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 2600.0))
            .with_pixels_per_point(ppp)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                let mut app = MassifApp::for_tests(world);
                app.settings.theme = theme;
                app
            });
        h.run_steps(3);
        let ctx = h.ctx.clone();
        for _ in 0..400 {
            h.step();
            if h.state_mut().doc.heights(&ctx).is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(h.state_mut().doc.heights(&ctx).is_some(), "the heights were read");
        h.run_steps(2);
        h
    }

    /// Brush Settings' height limits, for the eye (MASSIF_SNAPSHOT_DIR):
    /// the heights of the map read, no limit, a band, outside a band, a
    /// limit under the whole map, a soft one alone; in the dark and the
    /// light themes.
    #[test]
    fn height_limits_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        // MASSIF_SNAPSHOT_PPP=2: as on a Retina screen.
        let ppp: f32 = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        for (theme, name) in [(ThemeChoice::Dark, "dark"), (ThemeChoice::Light, "light")] {
            let mut h = heights_harness(theme, ppp);
            let cases = [
                ("none", None, None, false, false),
                ("band", Some(60), Some(90), false, false),
                ("outside", Some(60), Some(90), true, false),
                ("all", Some(-64), None, false, false),
                ("soft", None, Some(70), false, true),
            ];
            for (case, above, below, beyond, fade) in cases {
                {
                    let f = &mut h.state_mut().tools.filter;
                    (f.above, f.below, f.beyond, f.fade) = (above, below, beyond, fade);
                }
                h.run_steps(4);
                let image = h.render().expect("rendered");
                // The Brush Settings area, as wide as it is in the layout.
                let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
                let area = areas
                    .into_iter()
                    .find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::BrushSettings))
                    .map(|(_, r)| r)
                    .expect("Brush Settings in the layout");
                let crop = image::imageops::crop_imm(&image, (area.left() * ppp) as u32, (area.top() * ppp) as u32, (area.width() * ppp) as u32, (area.height() * ppp) as u32).to_image();
                crop.save(dir.join(format!("height-limits-{name}-{case}.png"))).unwrap();
            }
            // The pointer over the land: its height across, and the share of
            // the map at or above it.
            let (plot, _, _): (Rect, f32, f32) = h.ctx.data(|d| d.get_temp(Id::unique("height profile"))).expect("the picture of the land");
            h.event(egui::Event::PointerMoved(plot.center()));
            h.run_steps(30);
            let image = h.render().expect("rendered");
            let around = plot.expand(60.0);
            let crop = image::imageops::crop_imm(&image, (around.left() * ppp) as u32, (around.top() * ppp) as u32, (around.width() * ppp) as u32, (around.height() * ppp) as u32).to_image();
            crop.save(dir.join(format!("height-limits-{name}-hover.png"))).unwrap();
        }
    }

    /// The height limits are dragged on Brush Settings' picture of the land:
    /// a limit's line goes where it is let go, rounded to a block, and stops
    /// at the other limit; pressed away from the lines, the nearest one comes
    /// to the pointer; Shift drags finely; the panel does not scroll.
    #[test]
    fn dragging_the_height_limits() {
        let mut h = heights_harness(ThemeChoice::Dark, 1.0);
        {
            let f = &mut h.state_mut().tools.filter;
            (f.above, f.below) = (Some(60), Some(90));
        }
        h.run_steps(3);
        let (plot, v0, v1): (Rect, f32, f32) = h.ctx.data(|d| d.get_temp(Id::unique("height profile"))).expect("the picture of the land");
        assert!(v0 < 59.5 && v1 > 90.5, "the limits are in the picture: {v0} to {v1}");
        let y_of = |e: f32| plot.bottom() - (e - v0) / (v1 - v0) * plot.height();
        let x = plot.center().x;
        let button = |pressed, pos| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let drag = |h: &mut egui_kittest::Harness<'static, MassifApp>, from: f32, to: f32, shift: bool| {
            let modifiers = if shift { Modifiers::SHIFT } else { Modifiers::NONE };
            h.event(egui::Event::PointerMoved(pos2(x, from)));
            h.step();
            h.event(button(true, pos2(x, from)));
            h.step();
            h.event(egui::Event::ModifiersChanged(modifiers));
            for i in 1..=10 {
                h.event(egui::Event::PointerMoved(pos2(x, from + (to - from) * i as f32 / 10.0)));
                h.step();
            }
            h.event(button(false, pos2(x, to)));
            h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
            h.run_steps(3);
        };
        let limits = |h: &egui_kittest::Harness<'static, MassifApp>| (h.state().tools.filter.above, h.state().tools.filter.below);
        // The line under the blocks at 60, let go over those at 70.
        drag(&mut h, y_of(59.5), y_of(69.6), false);
        assert_eq!(limits(&h), (Some(70), Some(90)));
        // Dragged past the other limit, it stops at it.
        drag(&mut h, y_of(69.5), y_of(100.0), false);
        assert_eq!(limits(&h), (Some(90), Some(90)));
        // Pressed under both lines, the one at the bottom of the band comes.
        drag(&mut h, y_of(75.0), y_of(64.6), false);
        assert_eq!(limits(&h), (Some(65), Some(90)));
        // Shift: a tenth as far (the first points of a drag are a click's).
        let far = y_of(50.0) - y_of(84.0);
        drag(&mut h, y_of(90.5), y_of(90.5) + far, true);
        let (above, below) = limits(&h);
        assert!(above == Some(65) && below.is_some_and(|b| (85..=89).contains(&b)), "34 blocks down, finely: {below:?}");
        let (after, _, _): (Rect, f32, f32) = h.ctx.data(|d| d.get_temp(Id::unique("height profile"))).unwrap();
        assert_eq!(after, plot, "the panel did not scroll under the drags");
    }

    fn harness(theme: ThemeChoice) -> egui_kittest::Harness<'static, MassifApp> {
        egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            // MASSIF_SNAPSHOT_PPP=2: as on a Retina screen.
            .with_pixels_per_point(std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0))
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

    /// Walking on a real WorldPainter map, where a river comes down a
    /// slope: the views at a player's eyes, for a look. Needs a GPU and a
    /// map: MASSIF_SNAPSHOT_DIR, MASSIF_WALK_WORLD.
    #[test]
    fn walking_on_a_real_world() {
        let (Some(dir), Some(path)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_WALK_WORLD").map(PathBuf::from),
        ) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.state_mut().dialog = None;
        // Water stepping down: a column whose water is two blocks over its
        // neighbour's, on land well over the sea.
        let (x, z, level, down) = {
            let doc = &h.state().doc;
            let (lo, hi) = doc.dimension().bounds().unwrap();
            let t = massif_core::coords::TILE_SIZE;
            let sea = doc.world.meta.sea_level as i16;
            let fluid = |x: i32, z: i32| crate::tools::probe(doc, x, z).map(|p| p.fluid).filter(|f| *f != massif_core::tile::NO_FLUID);
            let mut best = None;
            'scan: for z in (lo.z * t..(hi.z + 1) * t).step_by(3) {
                for x in (lo.x * t..(hi.x + 1) * t).step_by(3) {
                    let Some(f) = fluid(x, z) else { continue };
                    if f < sea + 6 {
                        continue;
                    }
                    for (dx, dz) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                        if let Some(g) = fluid(x + dx, z + dz)
                            && g <= f - 2
                        {
                            best = Some((x, z, f, (dx, dz)));
                            break 'scan;
                        }
                    }
                }
            }
            best.expect("a river stepping down")
        };
        eprintln!("water at {x}, {z}: level {level}, lower toward {down:?}");
        h.state_mut().layout.active = 4; // 3D Sculpt
        h.state_mut().tools.sculpt.tool = Tool::Build;
        h.state_mut().tools.brush.radius = 4.0;
        h.run_steps(5);
        // Look at it from upstream, then walk.
        let yaw = (-(down.0 as f32)).atan2(-(down.1 as f32));
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [x as f64 + 0.5, level as f64, z as f64 + 0.5], yaw: yaw + std::f32::consts::PI, pitch: 0.55, distance: 24.0 };
        let mut frames = 0;
        while frames < 3 || h.state().sculpt3d.pending() > 0 {
            h.step();
            frames += 1;
            if frames > 2000 {
                break;
            }
        }
        let shot = |h: &mut egui_kittest::Harness<'_, MassifApp>, name: &str| {
            let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join(format!("{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        };
        shot(&mut h, "walk-river-orbit");
        h.state_mut().tools.sculpt.player = true;
        h.run_steps(20);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas.into_iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d)).map(|(_, r)| r).unwrap();
        h.event(egui::Event::PointerMoved(view.center()));
        h.event(egui::Event::PointerButton { pos: view.center(), button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: view.center(), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(10);
        eprintln!("landed: {:?}", h.state().sculpt3d.player());
        shot(&mut h, "walk-river-0");
        // Look down the river, then up the slope, then fly over it.
        h.event(egui::Event::MouseMoved(vec2(0.0, 80.0)));
        h.run_steps(3);
        shot(&mut h, "walk-river-1");
        h.event(egui::Event::MouseMoved(vec2(1200.0, -80.0)));
        h.run_steps(3);
        shot(&mut h, "walk-river-2");
        h.key_down(Key::W);
        h.run_steps(15);
        h.key_up(Key::W);
        h.run_steps(5);
        eprintln!("walked: {:?}", h.state().sculpt3d.player());
        shot(&mut h, "walk-river-3");
        h.key_press(Key::Space);
        h.step();
        h.key_press(Key::Space);
        h.step();
        h.key_down(Key::Space);
        h.run_steps(10);
        h.key_up(Key::Space);
        h.event(egui::Event::MouseMoved(vec2(-1200.0, 200.0)));
        h.run_steps(5);
        eprintln!("flying: {:?}", h.state().sculpt3d.player());
        shot(&mut h, "walk-river-4");
        h.render().unwrap().save(dir.join("walk-river-ui.png")).unwrap();
    }

    /// The player's view looks like the game, as Axiom shows it: the game's
    /// textures, its light, sky and clouds, the blocks the brush takes
    /// marked, the block looked at outlined. Renders it (MASSIF_SNAPSHOT_DIR)
    /// on a real map (MASSIF_WALK_WORLD) or a made one, with the game's
    /// blocks where Minecraft is installed.
    #[test]
    fn the_players_view_looks_like_the_game() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        if let Some(path) = std::env::var_os("MASSIF_WALK_WORLD").map(PathBuf::from) {
            h.state_mut().jobs.push(Job::open(&ctx, path));
            while !h.state().jobs.is_empty() {
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.state_mut().dialog = None;
        }
        if !massif_assets::find_jars().is_empty() {
            h.state_mut().blocks.load(&ctx, None, Vec::new());
            let started = std::time::Instant::now();
            while h.state().blocks.catalogue.is_none() {
                assert!(started.elapsed().as_secs() < 60, "the blocks did not load: {:?}", h.state().blocks.error);
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
        // A shore near the middle of the map: dry land over the sea, more
        // land ahead, then the water.
        let (x, z, top, toward) = {
            let doc = &h.state().doc;
            let sea = doc.world.meta.sea_level as f32;
            let (lo, hi) = doc.dimension().bounds().unwrap();
            let t = massif_core::coords::TILE_SIZE;
            let (mx, mz) = ((lo.x + hi.x + 1) * t / 2, (lo.z + hi.z + 1) * t / 2);
            let dry = |x: i32, z: i32| crate::tools::probe(doc, x, z).filter(|p| p.fluid == massif_core::tile::NO_FLUID);
            let wet = |x: i32, z: i32| crate::tools::probe(doc, x, z).is_some_and(|p| p.fluid != massif_core::tile::NO_FLUID);
            let mut spots: Vec<(i32, i32)> = (lo.z * t + 64..(hi.z + 1) * t - 64)
                .step_by(16)
                .flat_map(|z| (lo.x * t + 64..(hi.x + 1) * t - 64).step_by(16).map(move |x| (x, z)))
                .collect();
            spots.sort_by_key(|&(x, z)| (x - mx).abs() + (z - mz).abs());
            spots
                .into_iter()
                .find_map(|(x, z)| {
                    let p = dry(x, z).filter(|p| p.height >= sea + 4.0 && p.height <= sea + 24.0)?;
                    [(1, 0), (-1, 0), (0, 1), (0, -1)]
                        .into_iter()
                        .find(|&(dx, dz)| dry(x + dx * 10, z + dz * 10).is_some() && wet(x + dx * 48, z + dz * 48))
                        .map(|d| (x, z, p.height, d))
                })
                .expect("a shore")
        };
        eprintln!("shore at {x}, {z}, {top}, looking {toward:?}");
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            app.tools.sculpt.tool = Tool::Build;
            app.tools.brush.radius = 3.0;
        }
        h.run_steps(5);
        let yaw = (-(toward.0 as f32)).atan2(-(toward.1 as f32));
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [x as f64 + 0.5, top as f64, z as f64 + 0.5], yaw, pitch: 0.35, distance: 20.0 };
        h.state_mut().tools.sculpt.player = true;
        h.run_steps(3);
        let wait = |h: &mut egui_kittest::Harness<'_, MassifApp>| {
            let mut frames = 0;
            while frames < 3 || h.state().sculpt3d.pending() > 0 {
                h.step();
                frames += 1;
                if frames > 3000 {
                    break;
                }
            }
        };
        wait(&mut h);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas.into_iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d)).map(|(_, r)| r).unwrap();
        let shot = |h: &mut egui_kittest::Harness<'_, MassifApp>, name: &str| {
            let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join(format!("{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        };
        // The cursor on the land ahead: the brush marks it, the block under
        // it is outlined.
        h.state_mut().sculpt3d.camera.pitch = 0.32;
        h.event(egui::Event::PointerMoved(view.center() + vec2(-60.0, 150.0)));
        h.run_steps(4);
        assert!(h.state().sculpt3d.hover().is_some(), "the cursor points at the land");
        shot(&mut h, "game-shore");
        h.render().unwrap().save(dir.join("game-shore-ui.png")).unwrap();
        // The right button uses the tool under the cursor, as in Axiom's
        // editor.
        let steps = h.state().doc.history.undo_count();
        let at = view.center() + vec2(-60.0, 150.0);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
        h.run_steps(4);
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(2);
        assert_eq!(h.state().doc.history.undo_count(), steps + 1, "the right button sculpts");
        assert_eq!(h.state().doc.history.undo_label(), Some("Sculpt"));
        assert!(h.state().sculpt3d.player().is_some_and(|p| !p.flying), "and the player still walks");
        // A click takes the mouse, as in the game: the crosshair, the block
        // it points at outlined.
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
        h.state_mut().sculpt3d.camera.pitch = 0.55;
        h.run_steps(3);
        assert!(h.state().sculpt3d.hover().is_some(), "the crosshair points at the land");
        h.render().unwrap().save(dir.join("game-crosshair-ui.png")).unwrap();
        // The crosshair close up: as it is by default, then each shape of
        // Settings ▸ Appearance.
        let ppp = h.ctx.pixels_per_point();
        let crop = |img: &image::RgbaImage, name: &str| {
            let (x, y) = (((view.center().x - 80.0) * ppp) as u32, ((view.center().y - 20.0) * ppp) as u32);
            image::imageops::crop_imm(img, x, y, (160.0 * ppp) as u32, (110.0 * ppp) as u32).to_image().save(dir.join(format!("{name}.png"))).unwrap();
        };
        crop(&h.render().unwrap(), "crosshair-default");
        for shape in crate::reticle::Shape::ALL {
            crate::reticle::set(&h.ctx, crate::reticle::Reticle { shape, ..Default::default() });
            h.run_steps(2);
            crop(&h.render().unwrap(), &format!("crosshair-{shape:?}").to_lowercase());
        }
        crate::reticle::set(&h.ctx, crate::reticle::Reticle::default());
        h.key_press(Key::Escape);
        h.step();
        // Up in the air, toward the horizon: the land fading into the fog,
        // the clouds.
        let p = h.state().sculpt3d.player().unwrap();
        h.state_mut().sculpt3d.place_player([p.feet[0], p.feet[1] + 70.0, p.feet[2]], true);
        h.state_mut().sculpt3d.camera.pitch = 0.12;
        h.event(egui::Event::PointerMoved(view.left_top() + vec2(20.0, 20.0)));
        h.run_steps(4);
        wait(&mut h);
        shot(&mut h, "game-high");
        h.state_mut().sculpt3d.camera.pitch = -0.35;
        h.run_steps(4);
        shot(&mut h, "game-sky");
        // Over the sea: deep water darker, as the game's light fades down
        // in it.
        let sea = {
            let doc = &h.state().doc;
            let (lo, hi) = doc.dimension().bounds().unwrap();
            let t = massif_core::coords::TILE_SIZE;
            let (mx, mz) = ((lo.x + hi.x + 1) * t / 2, (lo.z + hi.z + 1) * t / 2);
            let mut spots: Vec<(i32, i32)> = (lo.z * t..(hi.z + 1) * t).step_by(32).flat_map(|z| (lo.x * t..(hi.x + 1) * t).step_by(32).map(move |x| (x, z))).collect();
            spots.sort_by_key(|&(x, z)| (x - mx).abs() + (z - mz).abs());
            spots.into_iter().find_map(|(x, z)| {
                let p = crate::tools::probe(doc, x, z)?;
                (p.fluid != massif_core::tile::NO_FLUID && p.fluid as f32 - p.height > 12.0).then_some((x, z, p.fluid))
            })
        };
        if let Some((x, z, level)) = sea {
            h.state_mut().sculpt3d.place_player([x as f64 + 0.5, level as f64 + 14.0, z as f64 + 0.5], true);
            h.state_mut().sculpt3d.camera.pitch = 0.45;
            h.run_steps(4);
            wait(&mut h);
            shot(&mut h, "game-sea");
        }
    }

    /// Developer mode offers the Developer panel: the frames, the memory
    /// and the processor, live, and what Massif keeps. Renders it with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn the_developer_panel_shows_the_figures() {
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        assert!(!h.state().layout.current().root.shows(EditorKind::Developer));
        {
            let app = h.state_mut();
            app.settings.developer = true;
            app.show_developer_panel();
        }
        assert!(h.state().layout.current().root.shows(EditorKind::Developer));
        for _ in 0..40 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(12));
        }
        assert!(h.state().dev.fps() > 0.0, "frames counted");
        assert!(crate::devtools::memory_in_use().is_some_and(|b| b > 10 << 20), "the memory the system counts");
        assert!(crate::devtools::cpu_time().is_some());
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("developer-panel.png")).unwrap();
        }
        // Settings ▸ Developer: the frames a second, chosen with a click;
        // drawing every frame keeps the window drawing.
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Developer));
        h.run_steps(3);
        {
            use egui_kittest::kittest::Queryable;
            assert!(h.query_by_label("Draw every frame").is_some());
        }
        h.state_mut().settings.frame_limit = crate::devtools::FrameLimit::Unlimited;
        h.state_mut().settings.draw_always = true;
        h.run_steps(2);
        assert!(h.ctx.has_requested_repaint(), "drawing every frame asks for the next one");
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("developer-settings.png")).unwrap();
        }
        assert_eq!(crate::devtools::FrameLimit::Vsync.present_mode(), eframe::wgpu::PresentMode::AutoVsync);
        assert_eq!(crate::devtools::FrameLimit::Fps144.present_mode(), eframe::wgpu::PresentMode::AutoNoVsync);
        assert_eq!(crate::devtools::FrameLimit::Fps144.fps(), Some(144));
        // A cap of 60: two frames at least a sixtieth of a second apart.
        let mut last = None;
        crate::devtools::pace(&mut last, 60);
        let t = std::time::Instant::now();
        crate::devtools::pace(&mut last, 60);
        assert!(t.elapsed().as_secs_f64() > 0.014, "{:?}", t.elapsed());
    }

    /// The page of editors: an area's header opens it, the editor under the
    /// pointer says what it is for, a click shows it in the area; an empty
    /// area holds the same page. Renders both, in both themes, with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn the_editor_page_switches_an_area() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        for (choice, name) in [(ThemeChoice::Light, "light"), (ThemeChoice::Dark, "dark")] {
            let mut h = harness(choice);
            h.state_mut().settings.developer = true;
            h.run_steps(5);
            let mut areas = Vec::new();
            h.state().layout.current().root.areas(&mut areas);
            let map = areas.iter().find(|a| a.1 == EditorKind::Map).expect("a map area").0;
            let header = format!("{} {}", theme::glyph(EditorKind::Map.icon()), theme::glyph(Icon::ChevronDown));
            h.query_all_by_label(&header).next().expect("the map area's header").click();
            h.run_steps(3);
            h.get_by_label("Graph Viewport").hover();
            h.run_steps(3);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("editor-page-{name}.png"))).unwrap();
            }
            h.get_by_label("Node Properties").click();
            h.run_steps(3);
            assert_eq!(h.state().layout.current().root.editor_of(map), Some(EditorKind::GraphProps));
            assert!(h.query_by_label("Render Settings").is_none(), "the page closes once an editor is chosen");

            h.state_mut().layout.current_mut().root.set_editor(map, EditorKind::Empty);
            h.run_steps(3);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("empty-area-{name}.png"))).unwrap();
            }
            h.get_by_label("Render Settings").click();
            h.run_steps(3);
            assert_eq!(h.state().layout.current().root.editor_of(map), Some(EditorKind::RenderSettings));
        }
    }

    /// Opening a real map (MASSIF_MOVE_WORLD: a .massif), timed: until the
    /// map view has drawn what it shows, and until the 3D view has built
    /// every tile, nothing moving.
    #[test]
    fn opening_a_real_world() {
        let Some(path) = std::env::var_os("MASSIF_MOVE_WORLD").map(PathBuf::from) else {
            return;
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .with_step_dt(1.0 / 60.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(generate(&GenParams {
                    tiles_x: 2,
                    tiles_z: 2,
                    ..GenParams::default()
                }))
            });
        h.run_steps(3);
        let ctx = h.ctx.clone();
        let started = std::time::Instant::now();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        while !h.state().jobs.is_empty() {
            h.step();
        }
        h.state_mut().dialog = None;
        eprintln!("opened in {:.2} s", started.elapsed().as_secs_f32());
        // Built for at most MASSIF_MOVE_SECONDS (60): each frame's time.
        let cap = std::env::var("MASSIF_MOVE_SECONDS").ok().and_then(|s| s.parse::<f32>().ok()).unwrap_or(60.0);
        let shown = std::time::Instant::now();
        let mut times: Vec<f32> = Vec::new();
        while (h.state().view3d.pending() > 0 || times.len() < 5) && shown.elapsed().as_secs_f32() < cap {
            let t = std::time::Instant::now();
            h.step();
            times.push(t.elapsed().as_secs_f32() * 1000.0);
        }
        let took = shown.elapsed().as_secs_f32();
        let mut sorted = times.clone();
        sorted.sort_by(f32::total_cmp);
        let mean = times.iter().sum::<f32>() / times.len().max(1) as f32;
        eprintln!(
            "3D: {} tiles built, {} still to build after {took:.1} s; {} frames: mean {mean:.1} ms, p95 {:.1} ms, max {:.1} ms",
            h.state().view3d.tiles_built(),
            h.state().view3d.pending(),
            times.len(),
            sorted[sorted.len() * 95 / 100],
            sorted.last().copied().unwrap_or(0.0)
        );
        // At rest: no frame asked for (nothing moves, nothing to build).
        h.run_steps(30);
        let (mut asked, mut causes) = (0, std::collections::BTreeSet::new());
        for _ in 0..600 {
            h.step();
            if h.ctx.requested_repaint_last_pass_for(&h.ctx.viewport_id()) {
                asked += 1;
                causes.extend(h.ctx.repaint_causes().iter().map(|c| c.to_string()));
            }
        }
        eprintln!("at rest: {asked} of 600 frames asked for another at once; causes: {causes:?}");
        eprintln!("memory in use: {} MB", crate::devtools::memory_in_use().unwrap_or(0) >> 20);
        for (what, size, note) in h.state().dev_info().keeps {
            eprintln!("  {what}: {size} ({note})");
        }
    }

    #[test]
    fn starting_up() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let started = std::time::Instant::now();
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .with_step_dt(1.0 / 60.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(|cc| MassifApp::new(cc, None));
        eprintln!("harness and app: {:.1} ms", started.elapsed().as_secs_f32() * 1000.0);
        for k in 0..12 {
            let t = std::time::Instant::now();
            h.step();
            let work = t.elapsed().as_secs_f32() * 1000.0;
            let t = std::time::Instant::now();
            let _ = h.render();
            eprintln!("frame {k}: work {work:.1} ms, drawing {:.1} ms", t.elapsed().as_secs_f32() * 1000.0);
        }
        // Idle: what a frame with nothing to do costs, and whether one is asked for.
        let t = std::time::Instant::now();
        for _ in 0..30 {
            h.step();
        }
        eprintln!("idle frame: {:.2} ms", t.elapsed().as_secs_f32() * 1000.0 / 30.0);
    }

    /// A disk that takes seconds to read the backups' folder, and a network
    /// that never answers about new versions, do not hold up the start:
    /// the app is made and its first frames laid out at once (on a slow
    /// Windows PC, a main thread held for 5 s is a window "not responding").
    #[test]
    fn a_slow_disk_or_network_does_not_hold_up_the_start() {
        crate::autosave::TEST_SLOW_DISK.store(10_000, std::sync::atomic::Ordering::SeqCst);
        crate::update::TEST_BLOCKED_NETWORK.store(30_000, std::sync::atomic::Ordering::SeqCst);
        let started = std::time::Instant::now();
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_accessibility_check(false)
            .build_eframe(|cc| {
                let mut app = MassifApp::new(cc, None);
                // Not the backups this computer has.
                app.offer_recovery = false;
                app
            });
        let made = started.elapsed();
        h.run_steps(3);
        let frames = started.elapsed();
        assert!(h.state().autosave_opening.is_some(), "the backups' folder is still being read");
        assert!(h.state().updates.waiting() || h.state().updates.latest.is_some() || !h.state().settings.check_updates, "GitHub is still being asked");
        eprintln!("made in {:.0} ms, three frames in {:.0} ms", made.as_secs_f64() * 1000.0, frames.as_secs_f64() * 1000.0);
        assert!(frames < std::time::Duration::from_secs(5), "the start waited on the disk or the network: {frames:?}");
    }

    /// The zoom buttons and keys glide: the zoom gets there over a few
    /// frames, around the middle of the view, and lands exactly; "Fit the
    /// whole map" glides there too.
    #[test]
    fn the_map_view_glides() {
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
        let (z0, c0) = (h.state().maps[&id].zoom, h.state().maps[&id].center);
        h.state_mut().maps.get_mut(&id).unwrap().zoom_smoothly(2.0);
        h.step();
        let z1 = h.state().maps[&id].zoom;
        assert!(z1 > z0 * 1.2, "it heads there at once: {z0} → {z1}");
        h.run_steps(10);
        let view = &h.state().maps[&id];
        assert!((view.zoom - 2.0 * z0).abs() < 1e-4, "it lands on twice the zoom: {}", view.zoom);
        assert!((view.center[0] - c0[0]).abs() < 0.01 && (view.center[1] - c0[1]).abs() < 0.01, "around the middle of the view");
    }

    /// How smoothly a real map moves (MASSIF_MOVE_WORLD: a .massif): the
    /// time of each frame while the map view zooms in, pans and zooms out,
    /// and while the 3D view turns and moves, on a Retina-like screen.
    #[test]
    fn moving_around_a_real_world() {
        let Some(path) = std::env::var_os("MASSIF_MOVE_WORLD").map(PathBuf::from) else {
            return;
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .with_step_dt(1.0 / 60.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(generate(&GenParams {
                    tiles_x: 2,
                    tiles_z: 2,
                    ..GenParams::default()
                }))
            });
        h.run_steps(3);
        let ctx = h.ctx.clone();
        h.state_mut().jobs.push(Job::open(&ctx, path));
        while !h.state().jobs.is_empty() {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.state_mut().dialog = None;
        // MASSIF_MAP_ONLY: the map view alone, without the 3D view beside it
        // building the map at the same time.
        if std::env::var_os("MASSIF_MAP_ONLY").is_some() {
            let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
            h.state_mut().layout.current_mut().maximized = Some(id);
        }
        // MASSIF_MAP_SMOOTH: the map's smooth shading rather than its steps.
        if std::env::var_os("MASSIF_MAP_SMOOTH").is_some() {
            h.state_mut().maps.values_mut().for_each(|view| view.smooth = true);
        }
        eprintln!("opened: memory in use {} MB", crate::devtools::memory_in_use().unwrap_or(0) >> 20);
        // The whole map drawn: frames, time and the processor's time.
        let (started, cpu) = (std::time::Instant::now(), crate::devtools::cpu_time().unwrap_or_default());
        let mut frames = 0;
        h.step();
        while h.state().map_cache.busy() && frames < 5000 {
            h.step();
            frames += 1;
        }
        eprintln!(
            "whole map drawn in {frames} frames, {:.2} s, CPU {:.2} s; {:?} (images, grids, summaries)",
            started.elapsed().as_secs_f32(),
            (crate::devtools::cpu_time().unwrap_or_default() - cpu).as_secs_f32(),
            h.state().map_cache.stats()
        );
        h.run_steps(30);
        type H = egui_kittest::Harness<'static, MassifApp>;
        // The work of a frame (the interface, what it builds, what it sends
        // to the graphics card), and the time the card then takes to draw
        // it, less that of a frame where nothing moved (reading the image
        // back, which a window does not do).
        let still = |h: &mut H| {
            let mut t = Vec::new();
            for _ in 0..8 {
                h.step();
                let started = std::time::Instant::now();
                let _ = h.render();
                t.push(started.elapsed().as_secs_f32() * 1000.0);
            }
            t.sort_by(f32::total_cmp);
            t[t.len() / 2]
        };
        let run = |h: &mut H, name: &str, frames: usize, change: &dyn Fn(&mut MassifApp)| {
            let base = still(h);
            let (mut work, mut draw) = (Vec::new(), Vec::new());
            let mut cpu = std::time::Duration::ZERO;
            for _ in 0..frames {
                change(h.state_mut());
                let started = std::time::Instant::now();
                let before = crate::devtools::cpu_time().unwrap_or_default();
                h.step();
                cpu += crate::devtools::cpu_time().unwrap_or_default().saturating_sub(before);
                work.push(started.elapsed().as_secs_f32() * 1000.0);
                let started = std::time::Instant::now();
                let _ = h.render();
                draw.push((started.elapsed().as_secs_f32() * 1000.0 - base).max(0.0));
            }
            let stats = |v: &mut Vec<f32>| {
                v.sort_by(f32::total_cmp);
                let mean = v.iter().sum::<f32>() / v.len() as f32;
                (mean, v[(v.len() * 95 / 100).min(v.len() - 1)], *v.last().unwrap())
            };
            // The processor's time of a frame's work, every thread.
            let cpu = cpu.as_secs_f32() * 1000.0 / frames as f32;
            let (w, d) = (stats(&mut work), stats(&mut draw));
            eprintln!(
                "{name}: work mean {:.1} p95 {:.1} max {:.1} ms · drawing mean {:.1} p95 {:.1} max {:.1} ms (still frame {base:.1} ms) · CPU {cpu:.1} ms a frame",
                w.0, w.1, w.2, d.0, d.1, d.2
            );
        };
        let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
        run(&mut h, "map, whole, panning", 60, &|app| {
            let view = app.maps.get_mut(&id).unwrap();
            view.center[0] += 6.0 / view.zoom as f64;
        });
        run(&mut h, "map, zooming in", 50, &|app| app.maps.get_mut(&id).unwrap().zoom_by(1.07));
        run(&mut h, "map, panning", 120, &|app| {
            let view = app.maps.get_mut(&id).unwrap();
            view.center[0] += 24.0 / view.zoom as f64;
            view.center[1] += 10.0 / view.zoom as f64;
        });
        run(&mut h, "map, zooming out", 70, &|app| app.maps.get_mut(&id).unwrap().zoom_by(1.0 / 1.07));
        // A Raise stroke across the middle, the whole map in view, then
        // closer: each frame's work while the map follows the brush.
        let stroke = |h: &mut H, name: &str| {
            let rect = map_rect(h.state(), &h.ctx);
            let start = rect.center() - vec2(90.0, 0.0);
            let button = |pressed, pos| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
            h.event(egui::Event::PointerMoved(start));
            h.run_steps(20);
            h.event(button(true, start));
            let (mut work, mut cpu) = (Vec::new(), std::time::Duration::ZERO);
            let frames = std::env::var("MASSIF_STROKE_FRAMES").ok().and_then(|n| n.parse::<usize>().ok()).unwrap_or(60);
            for i in 1..=frames {
                h.event(egui::Event::PointerMoved(start + vec2((i % 120) as f32 * 3.0, (i / 120) as f32 * 3.0)));
                let started = std::time::Instant::now();
                let before = crate::devtools::cpu_time().unwrap_or_default();
                h.step();
                cpu += crate::devtools::cpu_time().unwrap_or_default().saturating_sub(before);
                work.push(started.elapsed().as_secs_f32() * 1000.0);
            }
            h.event(button(false, start));
            let mut settle = 0;
            h.step();
            while h.state().map_cache.busy() && settle < 600 {
                h.step();
                settle += 1;
            }
            work.sort_by(f32::total_cmp);
            eprintln!(
                "{name}: work mean {:.1} p95 {:.1} max {:.1} ms · CPU {:.1} ms a frame; drawn {settle} frames after",
                work.iter().sum::<f32>() / work.len() as f32,
                work[work.len() * 95 / 100],
                work[work.len() - 1],
                cpu.as_secs_f32() * 1000.0 / frames as f32
            );
        };
        h.state_mut().tools.brush.radius = 60.0;
        stroke(&mut h, "map, a stroke, whole map");
        h.state_mut().maps.get_mut(&id).unwrap().zoom_by(1.07f32.powi(40));
        h.run_steps(5);
        while h.state().map_cache.busy() {
            h.step();
        }
        stroke(&mut h, "map, a stroke, close");
        h.state_mut().maps.get_mut(&id).unwrap().zoom_by(1.07f32.powi(-40));
        // At rest on the whole map: whether frames are still asked for.
        h.run_steps(60);
        let mut asked = 0;
        for _ in 0..120 {
            h.step();
            asked += h.ctx.requested_repaint_last_pass_for(&h.ctx.viewport_id()) as usize;
        }
        eprintln!("map at rest: {asked} of 120 frames asked for another; repaint causes {:?}", h.ctx.repaint_causes());
        eprintln!("memory in use: {} MB", crate::devtools::memory_in_use().unwrap_or(0) >> 20);
        for (what, size, note) in h.state().dev_info().keeps {
            eprintln!("  {what}: {size} ({note})");
        }
        if std::env::var_os("MASSIF_MOVE_2D").is_some() {
            return;
        }
        // The 3D view: everything built, then turning, then moving, drawn
        // smaller while it moves, as in the app.
        h.state_mut().layout.active = 3;
        h.state_mut().view3d.reduce_while_moving = std::env::var_os("MASSIF_FULL_SIZE").is_none();
        let mut frames = 0;
        while frames < 3 || h.state().view3d.pending() > 0 {
            h.step();
            frames += 1;
            if frames > 3000 {
                break;
            }
        }
        run(&mut h, "3D, turning", 90, &|app| app.view3d.camera.yaw += 0.025);
        run(&mut h, "3D, moving in", 90, &|app| {
            let c = &mut app.view3d.camera;
            c.distance *= 0.985;
            c.target[0] += 6.0;
        });
        // Drawn smaller while it moves, sharp once it stops.
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.state_mut().view3d.camera.yaw += 0.05;
            h.step();
            h.render().unwrap().save(dir.join("view3d-moving.png")).unwrap();
            h.run_steps(20);
            h.render().unwrap().save(dir.join("view3d-still.png")).unwrap();
        }
    }

    /// Edit ▸ Whole map ▸ Erosion: the dialog shows the map before and
    /// after, Apply wears the whole map in the background, in one step that
    /// undo takes back.
    #[test]
    fn erosion_over_the_whole_map() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let heights = |h: &egui_kittest::Harness<'static, MassifApp>| -> Vec<f32> {
            let doc = &h.state().doc;
            (0..1024).step_by(9).flat_map(|z| (0..1024).step_by(9).map(move |x| (x, z))).filter_map(|(x, z)| crate::tools::probe(doc, x, z).map(|p| p.height)).collect()
        };
        let before = heights(&h);
        h.state_mut().open_reshape(crate::reshape::WholeOp::Erosion);
        // The pictures come, made on another thread.
        let started = std::time::Instant::now();
        while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Reshape(f) if f.has_pictures())) {
            assert!(started.elapsed().as_secs() < 60, "the pictures never came");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("dialog-whole-map-erosion.png")).unwrap();
        }
        h.get_by_label("Apply to the whole map").click();
        h.run_steps(2);
        assert!(h.state().dialog.is_none() && !h.state().jobs.is_empty(), "the job runs");
        let started = std::time::Instant::now();
        while !h.state().jobs.is_empty() {
            assert!(started.elapsed().as_secs() < 120, "the erosion timed out");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        let after = heights(&h);
        assert_eq!(before.len(), after.len());
        let moved = before.iter().zip(&after).filter(|(a, b)| (*a - *b).abs() > 0.1).count();
        assert!(moved > before.len() / 50, "{moved} of {} columns looked at changed", before.len());
        assert_eq!(h.state().doc.history.undo_label(), Some("Erosion"));
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("whole-map-eroded.png")).unwrap();
        }
        let ctx = h.ctx.clone();
        h.state_mut().undo(&ctx);
        h.run_steps(2);
        assert_eq!(heights(&h), before, "one undo takes it all back");
    }

    /// Edit ▸ Caves: the dialog carves tunnels, ravines and caverns in the
    /// background, in one step that undo takes back; they show in the 3D
    /// view and in 3D Sculpt (renders with MASSIF_SNAPSHOT_DIR).
    #[test]
    fn carving_caves() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
            let mut h = harness(theme);
            h.run_steps(5);
            h.state_mut().open_caves();
            h.run_steps(3);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("dialog-caves-{theme:?}.png").to_lowercase())).unwrap();
            }
            if theme == ThemeChoice::Light {
                continue;
            }
            // Many, and open: the ravines show from above.
            if let Some(Dialog::Caves(form)) = &mut h.state_mut().dialog {
                form.settings.density = 1.0;
                form.settings.openings = 1.0;
                form.settings.lake = Some(massif_core::voxel::Fluid::Lava);
            }
            h.get_by_label("Carve").click();
            h.run_steps(2);
            assert!(h.state().dialog.is_none() && !h.state().jobs.is_empty(), "the job runs");
            let started = std::time::Instant::now();
            while !h.state().jobs.is_empty() {
                assert!(started.elapsed().as_secs() < 120, "carving timed out");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            h.run_steps(3);
            assert_eq!(h.state().doc.history.undo_label(), Some("Caves"));
            let chunks = h.state().doc.dimension().voxels.len();
            assert!(chunks > 100, "{chunks} chunks carved");
            if let Some(dir) = &dir {
                h.state_mut().layout.active = 3;
                let mut frames = 0;
                while frames < 5 || h.state().view3d.pending() > 0 {
                    h.step();
                    frames += 1;
                    assert!(frames < 3000, "the 3D view never finished");
                }
                h.run_steps(10);
                h.render().unwrap().save(dir.join("caves-3d-view.png")).unwrap();
                // The deepest cut open to the sky on land: a ravine.
                let (x, top, z) = {
                    let dim = h.state().doc.dimension();
                    let mut best = (0, 0.0f32, 0, 0.0f32);
                    let mut spans = Vec::new();
                    for (pos, _) in dim.voxels.iter() {
                        let (x0, _, z0) = pos.origin();
                        let (x, z) = (x0 + 8, z0 + 8);
                        let Some(stack) = massif_core::TileStack::new(dim, massif_core::TilePos::containing(x, z), false) else { continue };
                        let cell = massif_core::coords::local_index(x, z);
                        let ground = stack.ground().top.get(cell);
                        stack.spans(cell, &mut spans);
                        let Some(surface) = spans.last() else { continue };
                        let cut = ground - surface.top;
                        if ground > 68.0 && cut > best.3 {
                            best = (x, ground, z, cut);
                        }
                    }
                    assert!(best.3 > 4.0, "a ravine open to the sky on land");
                    (best.0, best.1, best.2)
                };
                h.state_mut().layout.active = 4;
                h.run_steps(10);
                h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [x as f64 + 0.5, top as f64 - 8.0, z as f64 + 0.5], yaw: 0.65, pitch: 0.7, distance: 70.0 };
                h.run_steps(40);
                h.render().unwrap().save(dir.join("caves-sculpt.png")).unwrap();
                if let Some((w, hgt, rgba)) = h.state().sculpt3d.read_back() {
                    image::save_buffer(dir.join("caves-sculpt-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
                }
            }
            let ctx = h.ctx.clone();
            h.state_mut().undo(&ctx);
            h.run_steps(2);
            assert!(h.state().doc.dimension().voxels.is_empty(), "one undo takes the caves back");
        }
    }

    /// Edit ▸ Relief of a Real Place and File ▸ New World from a Real
    /// Place, from tiles put in memory (no network): the dialog shows the
    /// place and the ground it makes (renders with MASSIF_SNAPSHOT_DIR);
    /// applying writes heights, sea and paint as one step to undo, and a
    /// new world is made of it.
    #[test]
    fn relief_of_a_real_place() {
        use egui_kittest::kittest::Queryable;
        let _store = crate::real_terrain::tests::STORE.lock().unwrap_or_else(|e| e.into_inner());
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let store = crate::tiles::TileStore::global();
        let wait_for_picture = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            let started = std::time::Instant::now();
            loop {
                h.step();
                if let Some(Dialog::Real(form)) = &h.state().dialog
                    && form.has_picture()
                {
                    break;
                }
                assert!(started.elapsed().as_secs() < 30, "no preview");
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.run_steps(3);
        };
        let wait_for_jobs = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            let started = std::time::Instant::now();
            while !h.state().jobs.is_empty() {
                assert!(started.elapsed().as_secs() < 120, "the job timed out");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            h.run_steps(3);
        };
        for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
            let mut h = harness(theme);
            h.run_steps(5);
            h.state_mut().open_real(false);
            let area = {
                let Some(Dialog::Real(form)) = &mut h.state_mut().dialog else { panic!("the dialog opens") };
                form.area.blocks_per_metre = 0.02;
                form.area
            };
            crate::real_terrain::tests::seed_store(&store, &area);
            wait_for_picture(&mut h);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("dialog-real-terrain-{theme:?}.png").to_lowercase())).unwrap();
            }
            if theme == ThemeChoice::Light {
                continue;
            }
            // Covers painted all over the old map: the import paints the
            // land again from the new relief by default, and takes them off.
            if let Some(Dialog::Real(form)) = &h.state().dialog {
                assert!(form.repaint, "repainted by default");
            }
            {
                let doc = &mut h.state_mut().doc;
                let positions: Vec<massif_core::TilePos> = doc.dimension().ground().positions().collect();
                let ground = doc.world.dims[doc.dim].ground_mut();
                for pos in positions {
                    ground.tile_mut(pos).unwrap().cover = massif_core::plane::Plane::Uniform(1);
                }
                doc.mark_changed();
            }
            h.run_steps(1);
            let before = h.state().doc.revision;
            h.get_by_label("Apply").click();
            h.run_steps(2);
            assert!(h.state().dialog.is_none() && !h.state().jobs.is_empty(), "the job runs");
            wait_for_jobs(&mut h);
            assert_eq!(h.state().doc.history.undo_label(), Some("Real terrain"), "{:?}", h.state().status.as_ref().map(|s| s.text.clone()));
            assert_ne!(h.state().doc.revision, before);
            // The sea to the west, a peak to the east, squeezed into the world.
            let (west, east, wet) = {
                let dim = h.state().doc.dimension();
                let (lo, hi) = dim.bounds().unwrap();
                let (x0, x1) = (lo.x * massif_core::TILE_SIZE, (hi.x + 1) * massif_core::TILE_SIZE);
                let z = (lo.z + hi.z + 1) * massif_core::TILE_SIZE / 2;
                let column = |x: i32| {
                    let stack = massif_core::TileStack::new(dim, massif_core::TilePos::containing(x, z), false).unwrap();
                    let i = massif_core::coords::local_index(x, z);
                    (stack.ground().top.get(i), stack.ground().fluid.get(i))
                };
                let peak = (x0..x1).step_by(4).map(|x| column(x).0).fold(f32::MIN, f32::max);
                (column(x0 + 2), peak, column(x0 + 2).1 != massif_core::tile::NO_FLUID)
            };
            assert!(west.0 < 62.0 && wet, "the sea in the west: {west:?}");
            // About 3100 m at 50 m a block: 62 blocks over the sea.
            assert!((east - 124.0).abs() < 6.0, "a peak in the east: {east}");
            {
                let dim = h.state().doc.dimension();
                let covered = dim.ground().positions().filter(|&p| dim.ground().view(p).unwrap().cover.uniform_value() != Some(0)).count();
                assert_eq!(covered, 0, "the old covers taken off");
            }
            let ctx = h.ctx.clone();
            h.state_mut().undo(&ctx);
            h.run_steps(2);
            assert_eq!(h.state().doc.history.undo_label(), None, "one undo takes it back");
        }
        // A new world.
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        h.state_mut().open_real(true);
        let area = {
            let Some(Dialog::Real(form)) = &mut h.state_mut().dialog else { panic!("the dialog opens") };
            form.area.blocks_per_metre = 0.02;
            form.area.width = 512;
            form.area.depth = 384;
            form.name = "Test place".into();
            form.area
        };
        crate::real_terrain::tests::seed_store(&store, &area);
        wait_for_picture(&mut h);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("dialog-real-place-new-world.png")).unwrap();
        }
        h.get_by_label("Make the world").click();
        h.run_steps(2);
        wait_for_jobs(&mut h);
        let world = &h.state().doc.world;
        assert_eq!(world.meta.name, "Test place");
        let (lo, hi) = world.dims[0].bounds().unwrap();
        assert_eq!(((hi.x - lo.x + 1) * massif_core::TILE_SIZE, (hi.z - lo.z + 1) * massif_core::TILE_SIZE), (512, 384));
        if let Some(dir) = &dir {
            h.run_steps(20);
            h.render().unwrap().save(dir.join("real-place-new-world.png")).unwrap();
        }
    }

    /// Edit ▸ Relief of a Real Place over the whole map, made another size
    /// in the same dialog: narrower and deeper, around its middle. The map
    /// takes the new size, its land replaced, a slab level's tile outside
    /// the new size dropped and one inside it kept; one undo puts the map
    /// back as it was, size and all, and redo makes it again. The dialog is
    /// drawn keeping the size, changing it, and at the largest size (renders
    /// with MASSIF_SNAPSHOT_DIR).
    #[test]
    fn relief_of_a_real_place_resizes_the_map() {
        use egui_kittest::kittest::Queryable;
        let _store = crate::real_terrain::tests::STORE.lock().unwrap_or_else(|e| e.into_inner());
        use massif_core::{TILE_SIZE, TilePos};
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let store = crate::tiles::TileStore::global();
        let size = |dim: &massif_core::world::Dimension| {
            let (lo, hi) = dim.bounds().unwrap();
            (lo, hi, (hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE)
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let (lo, hi, w, d) = size(h.state().doc.dimension());
        assert!(w >= 512, "a map to narrow: {w}");
        // A slab level with a tile at the west edge (dropped) and one in
        // the middle (kept).
        let (west, middle) = (TilePos::new(lo.x, (lo.z + hi.z) / 2), TilePos::new((lo.x + hi.x) / 2, (lo.z + hi.z) / 2));
        {
            let doc = &mut h.state_mut().doc;
            let slab = doc.world.dims[doc.dim].add_level("Island", massif_core::level::LevelKind::Solid);
            for pos in [west, middle] {
                doc.world.dims[doc.dim].levels[slab].insert(pos, massif_core::tile::Tile::ground(200.0, 1, massif_core::tile::NO_FLUID));
            }
            doc.mark_changed();
        }
        h.run_steps(2);
        let before = h.state_mut().doc.world.snapshot();
        h.state_mut().open_real(false);
        let (new_w, new_d) = (w - 2 * TILE_SIZE, d + 2 * TILE_SIZE);
        let area = {
            let Some(Dialog::Real(form)) = &mut h.state_mut().dialog else { panic!("the dialog opens") };
            assert!(form.keep_size && form.resize().is_none(), "the size is kept by default");
            form.area.blocks_per_metre = 0.02;
            form.area
        };
        crate::real_terrain::tests::seed_store(&store, &area);
        let started = std::time::Instant::now();
        while !matches!(&h.state().dialog, Some(Dialog::Real(form)) if form.has_picture()) {
            assert!(started.elapsed().as_secs() < 30, "no preview");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        let shot = |h: &mut egui_kittest::Harness<'static, MassifApp>, name: &str| {
            if let Some(dir) = &dir {
                h.run_steps(3);
                h.render().unwrap().save(dir.join(format!("dialog-real-terrain-{name}.png"))).unwrap();
            }
        };
        shot(&mut h, "keep-size");
        // At the largest size a real place may be.
        if let Some(Dialog::Real(form)) = &mut h.state_mut().dialog {
            form.keep_size = false;
            form.new_size = (massif_core::real::MAX_SIZE, massif_core::real::MAX_SIZE);
        }
        h.run_steps(2);
        if let Some(Dialog::Real(form)) = &h.state().dialog {
            let rect = form.resize().expect("a new size");
            assert_eq!((rect.width(), rect.depth()), (32_768, 32_768));
            assert_eq!((form.area.width, form.area.depth), (32_768, 32_768), "the area follows the size");
        }
        shot(&mut h, "largest");
        if let Some(Dialog::Real(form)) = &mut h.state_mut().dialog {
            form.new_size = (new_w, new_d);
        }
        h.run_steps(2);
        if let Some(Dialog::Real(form)) = &h.state().dialog {
            crate::real_terrain::tests::seed_store(&store, &form.area);
        }
        shot(&mut h, "new-size");
        h.get_by_label("Apply").click();
        h.run_steps(2);
        let started = std::time::Instant::now();
        while !h.state().jobs.is_empty() {
            assert!(started.elapsed().as_secs() < 120, "the job timed out");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        h.run_steps(3);
        assert_eq!(h.state().doc.history.undo_label(), Some("Real terrain"), "{:?}", h.state().status.as_ref().map(|s| s.text.clone()));
        let expected = massif_core::world::resized_rect(massif_core::coords::BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE), new_w, new_d);
        let check_resized = |h: &egui_kittest::Harness<'static, MassifApp>| {
            let dim = h.state().doc.dimension();
            let (nlo, _, nw, nd) = size(dim);
            assert_eq!((nw, nd), (new_w, new_d));
            assert_eq!((nlo.x * TILE_SIZE, nlo.z * TILE_SIZE), (expected.x0, expected.z0), "around the middle");
            assert_eq!(dim.ground().len() as i32, (new_w / TILE_SIZE) * (new_d / TILE_SIZE), "every tile of the new size");
            assert!(!dim.levels[1].contains(west) && dim.levels[1].contains(middle), "the slab cropped to the new size");
            assert!(dim.selection.is_none());
        };
        check_resized(&h);
        // The new ground has the place's heights, not the blank it began as.
        {
            let dim = h.state().doc.dimension();
            let (nlo, nhi, ..) = size(dim);
            let corner = massif_core::TileStack::new(dim, nhi, false).unwrap();
            let far = massif_core::TileStack::new(dim, nlo, false).unwrap();
            let (a, b) = (corner.ground().top.get(0), far.ground().top.get(massif_core::TILE_AREA - 1));
            assert!(a != b, "the new tiles are written: {a} {b}");
        }
        if let Some(dir) = &dir {
            h.run_steps(20);
            h.render().unwrap().save(dir.join("real-terrain-resized-map.png")).unwrap();
        }
        let ctx = h.ctx.clone();
        h.state_mut().undo(&ctx);
        h.run_steps(2);
        assert_eq!(h.state().doc.history.undo_label(), None, "one undo takes it back");
        {
            let dim = h.state().doc.dimension();
            assert_eq!((size(dim).2, size(dim).3), (w, d), "the old size back");
            assert_eq!(dim.ground().len(), before.dims[0].ground().len());
            assert!(dim.levels[1].contains(west) && dim.levels[1].contains(middle));
            let pos = before.dims[0].ground().positions().next().unwrap();
            let stack = massif_core::TileStack::new(dim, pos, false).unwrap();
            let old = massif_core::TileStack::new(&before.dims[0], pos, false).unwrap();
            assert_eq!(stack.ground().top.get(77), old.ground().top.get(77), "the old heights back");
        }
        h.state_mut().redo(&ctx);
        h.run_steps(2);
        check_resized(&h);
    }

    /// The sizes typed for the map are whole tiles of 128 blocks, from one
    /// tile to the largest real place.
    #[test]
    fn a_map_size_typed_is_whole_tiles_within_the_limits() {
        use crate::real_terrain::whole_tiles;
        assert_eq!(whole_tiles(1000), 1024);
        assert_eq!(whole_tiles(1100), 1152);
        assert_eq!(whole_tiles(5), 128);
        assert_eq!(whole_tiles(-40), 128);
        assert_eq!(whole_tiles(40_000), massif_core::real::MAX_SIZE);
        assert_eq!(massif_core::real::MAX_SIZE, 32_768);
    }

    /// Edit ▸ Analyse the map: the survival check reads the whole map in
    /// the background and says what a generated map misses (wood) and has
    /// (the ores the export lays);
    /// its tabs draw the numbers; a spot shown glides a map view there.
    #[test]
    fn analysing_the_map() {
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            let mut h = harness(theme);
            h.run_steps(5);
            let ctx = h.ctx.clone();
            h.state_mut().open_analysis(&ctx);
            let started = std::time::Instant::now();
            while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Analysis(v) if v.is_done())) {
                assert!(started.elapsed().as_secs() < 60, "the analysis never came");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.run_steps(3);
            let titles: Vec<String> = match &h.state().dialog {
                Some(Dialog::Analysis(v)) => v.findings().iter().map(|f| f.title.clone()).collect(),
                _ => unreachable!(),
            };
            assert!(titles.iter().any(|t| t == "No wood anywhere"), "{titles:?}");
            assert!(titles.iter().any(|t| t == "Ores to mine"), "a new map lays ores: {titles:?}");
            let shade = if theme == ThemeChoice::Dark { "dark" } else { "light" };
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("dialog-analysis-survival-{shade}.png"))).unwrap();
            }
            for (k, tab) in ["survival", "relief", "blocks", "biomes"].into_iter().enumerate() {
                if let Some(Dialog::Analysis(v)) = &mut h.state_mut().dialog {
                    v.set_tab(k);
                }
                h.run_steps(3);
                if let Some(dir) = &dir {
                    h.render().unwrap().save(dir.join(format!("dialog-analysis-{tab}-{shade}.png"))).unwrap();
                    // And further down.
                    h.event(egui::Event::PointerMoved(pos2(620.0, 640.0)));
                    for _ in 0..2 {
                        h.event(egui::Event::MouseWheel {
                            unit: egui::MouseWheelUnit::Point,
                            delta: vec2(0.0, -460.0),
                            phase: egui::TouchPhase::Move,
                            source: egui::MouseWheelSource::Wheel,
                            modifiers: Modifiers::NONE,
                        });
                        h.run_steps(4);
                    }
                    h.render().unwrap().save(dir.join(format!("dialog-analysis-{tab}-{shade}-below.png"))).unwrap();
                }
            }
            // Closed (Escape) and opened again on the same map: the same
            // check, at once.
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert!(h.state().dialog.is_none() && h.state().analysis.is_some(), "closed and kept");
            h.state_mut().open_analysis(&ctx);
            assert!(matches!(&h.state().dialog, Some(Dialog::Analysis(v)) if v.is_done()), "the same check, at once");
            h.state_mut().dialog = None;
        }
        // A spot shown: the map glides there.
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().show_on_map(&ctx, 300, -200);
        h.run_steps(5);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("analysis-show-on-map.png")).unwrap();
        }
        h.run_steps(40);
        let id = h.state().active_map.expect("a map view");
        let c = h.state().maps[&id].center;
        assert!((c[0] - 300.5).abs() < 1.0 && (c[1] + 199.5).abs() < 1.0, "the map is at {c:?}");

    }

    /// The survival check of a real map (MASSIF_RIVER_WORLD: a .massif),
    /// every tab drawn. Runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn analysing_a_real_world() {
        let (Some(dir), Some(path)) = (std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from), std::env::var_os("MASSIF_RIVER_WORLD")) else {
            return;
        };
        let world = massif_format::load(std::path::Path::new(&path)).expect("the world loads");
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(world)
            });
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().open_analysis(&ctx);
        let started = std::time::Instant::now();
        while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Analysis(v) if v.is_done())) {
            assert!(started.elapsed().as_secs() < 120, "the analysis never came");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        eprintln!("analysed in {:.2} s", started.elapsed().as_secs_f32());
        for (k, tab) in ["survival", "relief", "blocks", "biomes"].into_iter().enumerate() {
            if let Some(Dialog::Analysis(v)) = &mut h.state_mut().dialog {
                v.set_tab(k);
            }
            h.run_steps(3);
            h.render().unwrap().save(dir.join(format!("real-analysis-{tab}.png"))).unwrap();
        }
        // The ores of the same map, cut through.
        h.state_mut().dialog = None;
        let t = std::time::Instant::now();
        for _ in 0..10 {
            h.step();
        }
        eprintln!("a frame without a window: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
        let started = std::time::Instant::now();
        h.state_mut().open_ores(&ctx);
        while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Ores(v) if v.is_ready())) {
            assert!(started.elapsed().as_secs() < 120, "the ores window never got ready");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        eprintln!("ores window ready in {:.2} s", started.elapsed().as_secs_f32());
        let t = std::time::Instant::now();
        for _ in 0..10 {
            h.step();
        }
        eprintln!("a frame of the ores window: {:.1} ms", t.elapsed().as_secs_f32() * 100.0);
        h.render().unwrap().save(dir.join("real-ores.png")).unwrap();
    }

    /// Edit ▸ Ores: the window cuts the map through with the ores the
    /// export lays and a candle for each band; dragging the top of the
    /// diamonds' candle raises their band; closing keeps it in the map. The
    /// survival check of a map without ores leads there.
    #[test]
    fn setting_the_ores() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let wait = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            let started = std::time::Instant::now();
            while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Ores(v) if v.is_ready())) {
                assert!(started.elapsed().as_secs() < 60, "the ores window never got ready");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.run_steps(3);
        };
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            let mut h = harness(theme);
            h.run_steps(5);
            let ctx = h.ctx.clone();
            h.state_mut().open_ores(&ctx);
            wait(&mut h);
            let shade = if theme == ThemeChoice::Dark { "dark" } else { "light" };
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("dialog-ores-{shade}.png"))).unwrap();
            }
            if theme == ThemeChoice::Light {
                continue;
            }
            for scale in ["Log", "Share"] {
                h.get_by_label(scale).click();
                h.run_steps(3);
                if let Some(dir) = &dir {
                    h.render().unwrap().save(dir.join(format!("dialog-ores-{}.png", scale.to_lowercase()))).unwrap();
                }
            }
            h.get_by_label("Blocks").click();
            // Drag the top of the diamonds' candle up 60 points.
            let diamond = h.state().doc.world.meta.ores.ores.iter().position(|o| o.mineral == massif_core::analysis::Mineral::Diamond).unwrap();
            let (from, before) = match &mut h.state_mut().dialog {
                Some(Dialog::Ores(v)) => {
                    v.select(diamond);
                    (v.grip(diamond, 0, true), v.settings.ores[diamond].bands[0].max)
                }
                _ => unreachable!(),
            };
            h.run_steps(2);
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.step();
            for k in 1..=6 {
                h.event(egui::Event::PointerMoved(from - vec2(0.0, 10.0 * k as f32)));
                h.step();
            }
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join("dialog-ores-dragging.png")).unwrap();
            }
            let to = from - vec2(0.0, 60.0);
            h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
            let after = match &h.state().dialog {
                Some(Dialog::Ores(v)) => v.settings.ores[diamond].bands[0].max,
                _ => unreachable!(),
            };
            assert!(after > before + 20, "the band rose: {before} → {after}");
            let revision = h.state().doc.revision;
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert!(h.state().dialog.is_none());
            assert_eq!(h.state().doc.world.meta.ores.ores[diamond].bands[0].max, after, "kept in the map");
            assert!(h.state().doc.revision > revision, "the map changed");
            // Without ores, the survival check leads to the window.
            h.state_mut().doc.world.meta.ores.on = false;
            h.state_mut().open_analysis(&ctx);
            let started = std::time::Instant::now();
            while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Analysis(v) if v.is_done())) {
                assert!(started.elapsed().as_secs() < 60, "the analysis never came");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.run_steps(3);
            // Down to the card about the ores.
            h.event(egui::Event::PointerMoved(pos2(620.0, 640.0)));
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, -260.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(4);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join("dialog-analysis-ores-off.png")).unwrap();
            }
            h.get_by_label_contains("Set up the ores").click();
            h.run_steps(2);
            assert!(matches!(h.state().dialog, Some(Dialog::Ores(_))), "the ores window opened");
            wait(&mut h);
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join("dialog-ores-off.png")).unwrap();
            }
        }
    }

    /// The ores window by keys and biomes: Delete (or Backspace) takes the
    /// ore picked out, ⌘Z puts it back where it was, the + adds one back as
    /// it was; nothing while a number is typed. The details light the band
    /// picked and the one whose candle is pointed at, the cut the candle of
    /// a row pointed at. An ore kept to some biomes is laid only there, in
    /// the cut and in the amounts, and the map keeps the choice. Draws with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn removing_ores_and_keeping_them_to_biomes() {
        use egui_kittest::kittest::Queryable;
        use massif_core::analysis::Mineral;
        type H = egui_kittest::Harness<'static, MassifApp>;
        fn view(h: &H) -> &crate::ores::OresView {
            match &h.state().dialog {
                Some(Dialog::Ores(v)) => v,
                _ => panic!("the ores window is closed"),
            }
        }
        fn view_mut(h: &mut H) -> &mut crate::ores::OresView {
            match &mut h.state_mut().dialog {
                Some(Dialog::Ores(v)) => v,
                _ => panic!("the ores window is closed"),
            }
        }
        let has = |h: &H, m: Mineral| view(h).settings.ores.iter().any(|o| o.mineral == m);
        let click = |h: &mut H, at: egui::Pos2| {
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.step();
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(2);
        };
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let shot = |h: &mut H, name: &str| {
            if let Some(dir) = &dir {
                h.render().unwrap().save(dir.join(format!("dialog-ores-{name}.png"))).unwrap();
            }
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().open_ores(&ctx);
        let started = std::time::Instant::now();
        while !view(&h).is_ready() {
            assert!(started.elapsed().as_secs() < 60, "the ores window never got ready");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        let all = view(&h).settings.clone();
        let index = |m: Mineral| all.ores.iter().position(|o| o.mineral == m).unwrap();
        let (diamond, iron) = (index(Mineral::Diamond), index(Mineral::Iron));

        // Delete takes the diamonds out, and the ore after them is picked.
        view_mut(&mut h).select(diamond);
        h.run_steps(2);
        h.key_press(Key::Delete);
        h.run_steps(2);
        assert!(!has(&h, Mineral::Diamond) && view(&h).settings.ores.len() == all.ores.len() - 1, "Delete takes the ore out");
        assert_eq!(view(&h).settings.ores[diamond].mineral, Mineral::Emerald);
        assert_eq!(view(&h).picked(), (diamond, 0), "the emeralds after them are picked");
        shot(&mut h, "removed");
        // ⌘Z puts them back where they were, picked.
        h.key_press_modifiers(Modifiers::COMMAND, Key::Z);
        h.run_steps(2);
        assert_eq!(view(&h).settings, all, "⌘Z puts them back");
        assert_eq!(view(&h).picked(), (diamond, 0));
        // Backspace too, and the + adds them back as they were.
        h.key_press(Key::Backspace);
        h.run_steps(2);
        assert!(!has(&h, Mineral::Diamond), "Backspace takes the ore out");
        h.get_by_label("Add an ore").click();
        h.run_steps(2);
        shot(&mut h, "add");
        h.get_by_label("Diamond").click();
        h.run_steps(2);
        assert_eq!(view(&h).settings, all, "the + adds them back");
        assert_eq!(view(&h).picked(), (diamond, 0));
        // Typing a number, Delete edits it and takes no ore out.
        let veins = h.get_by_label("Veins a chunk").rect();
        click(&mut h, pos2(veins.right() - 10.0, veins.center().y));
        assert!(h.ctx.text_edit_focused(), "the number is being typed");
        h.key_press(Key::Delete);
        h.run_steps(2);
        assert!(has(&h, Mineral::Diamond), "nothing taken out while a number is typed");
        h.key_press(Key::Enter);
        h.run_steps(2);
        assert!(!h.ctx.text_edit_focused());

        // The details light the band picked, and the one whose candle the
        // pointer is over; the cut, the candle of a row pointed at.
        view_mut(&mut h).select_band(iron, 0);
        h.run_steps(2);
        let candle = view(&h).grip(iron, 2, true);
        h.event(egui::Event::PointerMoved(candle));
        h.run_steps(2);
        assert_eq!(view(&h).lit().0, Some((iron, 2)), "the candle pointed at");
        shot(&mut h, "lit");
        let row = h.get_by_label("Band 2").rect();
        h.event(egui::Event::PointerMoved(row.center()));
        h.run_steps(3);
        assert_eq!(view(&h).lit(), (None, Some(1)), "the row pointed at");
        shot(&mut h, "row");

        // Diamonds only in some biomes: none until one is ticked, then fewer
        // than everywhere, and only where the biome is in the cut.
        view_mut(&mut h).select(diamond);
        h.run_steps(2);
        let biomes = view(&h).biomes_by_rock();
        assert!(biomes.len() >= 2, "the map has rock in several biomes: {biomes:?}");
        let (everywhere, in_cut) = (view(&h).total(diamond), view(&h).in_cut(diamond));
        assert!(everywhere > 0.0 && in_cut > 0);
        h.get_by_label("Only in").click();
        h.run_steps(2);
        assert_eq!(view(&h).settings.ores[diamond].biomes.only, Some(Vec::new()));
        assert_eq!((view(&h).total(diamond), view(&h).in_cut(diamond)), (0.0, 0), "only in no biome: nowhere");
        // Some sixty biomes: the one wanted found by its name.
        let name = h.state().doc.world.registry.biome(massif_core::registry::BiomeId(biomes[0])).name.clone();
        h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("ore only in", "search")), name.clone()));
        h.get_by_label(&theme::with_icon(Icon::Plus, "Add")).click();
        h.run_steps(2);
        let at = h.get_by_label(&name).rect().center();
        click(&mut h, at);
        shot(&mut h, "biomes-menu");
        assert_eq!(view(&h).settings.ores[diamond].biomes.only, Some(vec![biomes[0]]), "ticked in the menu");
        assert!(egui::Popup::is_any_open(&h.ctx), "the menu stays open to tick more");
        // Another ticked, then ticked off again, the menu open all along.
        let other = h.state().doc.world.registry.biome(massif_core::registry::BiomeId(biomes[1])).name.clone();
        h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("ore only in", "search")), other.clone()));
        h.run_steps(2);
        let at = h.get_by_label(&other).rect().center();
        click(&mut h, at);
        assert_eq!(view(&h).settings.ores[diamond].biomes.only, Some(vec![biomes[0], biomes[1]]), "a second one ticked");
        click(&mut h, at);
        assert_eq!(view(&h).settings.ores[diamond].biomes.only, Some(vec![biomes[0]]), "and ticked off");
        assert!(egui::Popup::is_any_open(&h.ctx));
        let some = view(&h).total(diamond);
        assert!(some > 0.0 && some < everywhere, "fewer diamonds: {some} of {everywhere}");
        assert!(view(&h).in_cut(diamond) <= in_cut);
        egui::Popup::close_all(&h.ctx);
        h.run_steps(2);
        // And never in another: ticked, then in a chip.
        view_mut(&mut h).settings.ores[diamond].biomes.except = Some(vec![biomes[1]]);
        h.run_steps(3);
        shot(&mut h, "biomes");
        // Closed, the map keeps the choice.
        h.key_press(Key::Escape);
        h.run_steps(2);
        assert!(h.state().dialog.is_none());
        let kept = &h.state().doc.world.meta.ores.ores[diamond].biomes;
        assert_eq!((kept.only.clone(), kept.except.clone()), (Some(vec![biomes[0]]), Some(vec![biomes[1]])), "kept in the map");
    }

    /// A map with diamond blocks and bedrock painted at its surface: the
    /// check says it breaks survival, in red. Draws with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn a_map_full_of_diamonds() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else { return };
        let mut h = harness(ThemeChoice::Dark);
        {
            let world = &mut h.state_mut().doc.world;
            let reg = &mut world.registry;
            let bedrock = reg.terrain_by_name("Bedrock").unwrap();
            let diamond = reg.intern_block("minecraft:diamond_block", [98, 237, 228]);
            let ore = reg.intern_block("minecraft:diamond_ore", [121, 158, 152]);
            reg.terrains.push(massif_core::registry::TerrainDef {
                name: "Diamond Block".into(),
                group: "Blocks".into(),
                color: [98, 237, 228],
                strata: vec![massif_core::registry::Stratum { blocks: vec![(diamond, 3), (ore, 1)], depth: (3, 5), mix: Default::default() }],
                submerged_top: None,
                biome_hint: None,
            });
            let diamonds = reg.terrains.len() as u16 - 1;
            let ground = world.dims[0].ground_mut();
            for (k, pos) in ground.positions().collect::<Vec<_>>().into_iter().enumerate() {
                let tile = ground.tile_mut(pos).unwrap();
                for cell in (0..massif_core::TILE_AREA).step_by(3) {
                    tile.terrain.set(cell, if k % 2 == 0 { diamonds } else { bedrock.0 });
                }
            }
        }
        h.run_steps(5);
        let ctx = h.ctx.clone();
        h.state_mut().open_analysis(&ctx);
        let started = std::time::Instant::now();
        while !h.state().dialog.as_ref().is_some_and(|d| matches!(d, Dialog::Analysis(v) if v.is_done())) {
            assert!(started.elapsed().as_secs() < 60, "the analysis never came");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        let titles: Vec<String> = match &h.state().dialog {
            Some(Dialog::Analysis(v)) => v.findings().iter().filter(|f| f.severity == massif_core::analysis::Severity::Critical).map(|f| f.title.clone()).collect(),
            _ => unreachable!(),
        };
        assert!(titles.contains(&"Diamonds lying around".to_owned()) && titles.contains(&"Bedrock where players walk".to_owned()), "{titles:?}");
        h.render().unwrap().save(dir.join("dialog-analysis-diamonds.png")).unwrap();
    }

    /// The typefaces: the settings show each in itself, a click sets the
    /// interface in it, and each draws the whole interface.
    #[test]
    fn choosing_a_typeface() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Typeface));
        h.run_steps(4);
        h.get_by_label("Geist").click();
        h.run_steps(4);
        assert_eq!(h.state().settings.typeface, theme::Typeface::Geist);
        assert_eq!(theme::typeface(), theme::Typeface::Geist);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("dialog-settings-typeface-geist.png")).unwrap();
        }
        h.state_mut().dialog = None;
        for face in theme::Typeface::ALL {
            theme::set_typeface(&h.ctx, face);
            h.run_steps(4);
            if let Some(dir) = &dir {
                let name = face.name().to_lowercase().replace(' ', "-");
                h.render().unwrap().save(dir.join(format!("typeface-{name}.png"))).unwrap();
            }
        }
        theme::set_typeface(&h.ctx, theme::Typeface::System);
    }

    /// Walking in 3D Sculpt: the player comes down onto the land looked at,
    /// Space twice flies, the click that lends the view the mouse sculpts
    /// nothing, the buttons then sculpt at the crosshair, and the orbit
    /// camera comes back from the player's eyes. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    /// Strokes in 3D Sculpt, timed frame by frame (MASSIF_PERF): building
    /// and smoothing with a large brush, the view showing each dab.
    #[test]
    fn sculpting_strokes_in_3d() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .with_step_dt(1.0 / 60.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(|cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(generate(&GenParams {
                    tiles_x: 8,
                    tiles_z: 8,
                    landform: Landform::Hills,
                    ..GenParams::default()
                }))
            });
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            app.tools.brush.strength = 1.0;
        }
        h.run_steps(20);
        let top = crate::tools::probe(&h.state().doc, 0, 0).map_or(64.0, |p| p.height);
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [0.5, top as f64, 0.5], yaw: 0.6, pitch: 0.9, distance: 220.0 };
        while h.state().sculpt3d.pending() > 0 {
            h.step();
        }
        h.run_steps(5);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas
            .into_iter()
            .find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d))
            .map(|(_, r)| r)
            .expect("the 3D Sculpt workspace shows its editor");
        for (tool, radius) in [(Tool::Build, 24.0), (Tool::Build, 48.0), (Tool::Smooth3d, 48.0), (Tool::Smooth3d, 64.0)] {
            h.state_mut().tools.sculpt.tool = tool;
            h.state_mut().tools.brush.radius = radius;
            let from = view.center() - vec2(60.0, 0.0);
            h.event(egui::Event::PointerMoved(from));
            h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            let mut times = Vec::new();
            for k in 0..40 {
                h.event(egui::Event::PointerMoved(from + vec2(k as f32 * 3.0, 0.0)));
                let t = std::time::Instant::now();
                h.step();
                times.push(t.elapsed().as_secs_f32() * 1000.0);
            }
            h.event(egui::Event::PointerButton { pos: from + vec2(120.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.run_steps(3);
            times.sort_by(f32::total_cmp);
            let mean = times.iter().sum::<f32>() / times.len() as f32;
            eprintln!("{tool:?} r{radius}: frame mean {mean:.1} ms, p95 {:.1} ms, max {:.1} ms", times[times.len() * 95 / 100], times.last().unwrap());
        }
    }

    /// A brush of radius 506 held in place on the map (MASSIF_PERF), as a
    /// user holds one: the time of the frames early on and after a while,
    /// the processor's time, and how many frames the map takes to catch
    /// up once it is let go. MASSIF_MAP_ONLY: without the 3D view beside it.
    #[test]
    fn holding_a_big_brush() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1440.0, 900.0))
            .with_pixels_per_point(2.0)
            .with_step_dt(1.0 / 60.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(|cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(generate(&GenParams {
                    tiles_x: 32,
                    tiles_z: 32,
                    landform: Landform::Mountains,
                    ..GenParams::default()
                }))
            });
        h.run_steps(5);
        let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
        // MASSIF_MAP_ONLY: the map view alone, without the 3D view beside it.
        if std::env::var_os("MASSIF_MAP_ONLY").is_some() {
            h.state_mut().layout.current_mut().maximized = Some(id);
        }
        {
            let view = h.state_mut().maps.get_mut(&id).unwrap();
            view.center = [300.0, 200.0];
            view.zoom = 0.6;
        }
        h.run_steps(5);
        while h.state().map_cache.busy() {
            h.step();
        }
        let rect = map_rect(h.state(), &h.ctx);
        for tool in [Tool::Raise, Tool::Smooth] {
            {
                let app = h.state_mut();
                app.tools.tool = tool;
                app.tools.brush.radius = 506.0;
                app.tools.brush.strength = 1.0;
            }
            // Everything drawn first, the 3D view too.
            let mut waited = 0;
            while (h.state().map_cache.busy() || h.state().view3d.pending() > 0) && waited < 3000 {
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(2));
                waited += 1;
            }
            let at = rect.center() + vec2(-150.0, 80.0);
            h.event(egui::Event::PointerMoved(at));
            h.step();
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            let (mut times, mut cpus) = (Vec::new(), Vec::new());
            let frames = 240;
            for _ in 0..frames {
                let (t, cpu) = (std::time::Instant::now(), crate::devtools::cpu_time().unwrap_or_default());
                h.step();
                times.push(t.elapsed().as_secs_f32() * 1000.0);
                cpus.push((crate::devtools::cpu_time().unwrap_or_default() - cpu).as_secs_f32() * 1000.0);
            }
            let busy = h.state().map_cache.busy();
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            let mut after = 0;
            while h.state().map_cache.busy() && after < 2000 {
                h.step();
                after += 1;
            }
            let mean = |v: &[f32]| v.iter().sum::<f32>() / v.len() as f32;
            eprintln!(
                "{tool:?} r506 held {frames} frames: first 30 {:.1} ms ({:.1} CPU), frames 105-135 {:.1} ms ({:.1} CPU), last 30 {:.1} ms ({:.1} CPU); map still drawing at release: {busy}, done {after} frames after",
                mean(&times[..30]),
                mean(&cpus[..30]),
                mean(&times[105..135]),
                mean(&cpus[105..135]),
                mean(&times[frames - 30..]),
                mean(&cpus[frames - 30..]),
            );
            h.run_steps(3);
        }
    }

    /// The right button uses the tool as the player, the mouse free or held,
    /// and never lets the view's hold of the mouse go: on macOS the events
    /// of a locked pointer come with points of their own (here, over the
    /// side panel), which moved the pointer out of the view, onto a panel,
    /// and brought it back to the middle in a way that undid the lock.
    #[test]
    fn the_right_button_as_the_player_keeps_the_view() {
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            app.tools.sculpt.tool = Tool::Build;
            app.tools.brush.radius = 3.0;
        }
        h.run_steps(20);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas.into_iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d)).map(|(_, r)| r).unwrap();
        h.state_mut().tools.sculpt.player = true;
        let at = view.center() + vec2(0.0, 60.0);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(20);
        // A frame as eframe gives it: the input through the app's hook first.
        let step = |h: &mut egui_kittest::Harness<'_, MassifApp>| {
            let mut input = std::mem::take(h.input_mut());
            h.state_mut().sculpt3d.pin_pointer(&mut input);
            *h.input_mut() = input;
            h.step();
        };
        // (Events straight into the frame's input, which the hook sees.)
        let send = |h: &mut egui_kittest::Harness<'_, MassifApp>, e: egui::Event| h.input_mut().events.push(e);
        let warps = |h: &egui_kittest::Harness<'_, MassifApp>| h.output().viewport_output.values().flat_map(|v| v.commands.iter()).filter(|c| matches!(c, egui::ViewportCommand::CursorPosition(_))).count();
        // The mouse free: the right button sculpts under the cursor, the view
        // stays as it is.
        let (yaw, pitch) = (h.state().sculpt3d.camera.yaw, h.state().sculpt3d.camera.pitch);
        let steps = h.state().doc.history.undo_count();
        send(&mut h, egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
        step(&mut h);
        for k in 1..4 {
            send(&mut h, egui::Event::PointerMoved(at + vec2(k as f32 * 6.0, k as f32 * 3.0)));
            step(&mut h);
        }
        send(&mut h, egui::Event::PointerButton { pos: at + vec2(18.0, 9.0), button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        step(&mut h);
        assert_eq!(h.state().doc.history.undo_count(), steps + 1, "the right button sculpts");
        assert!(!h.state().sculpt3d.captured(), "and does not take the mouse");
        assert_eq!((h.state().sculpt3d.camera.yaw, h.state().sculpt3d.camera.pitch), (yaw, pitch), "nor turn the view");
        // A click takes the mouse.
        send(&mut h, egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        step(&mut h);
        send(&mut h, egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        step(&mut h);
        step(&mut h);
        assert!(h.state().sculpt3d.captured());
        send(&mut h, egui::Event::MouseMoved(vec2(0.0, 150.0)));
        step(&mut h);
        // The right button, its events said to be over a panel beside.
        let elsewhere = if view.left() > 60.0 { pos2(view.left() - 30.0, view.center().y) } else { pos2(view.right() + 30.0, view.center().y) };
        assert!(!view.contains(elsewhere));
        let steps = h.state().doc.history.undo_count();
        send(&mut h, egui::Event::PointerMoved(elsewhere));
        send(&mut h, egui::Event::PointerButton { pos: elsewhere, button: egui::PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
        step(&mut h);
        let (mut moved, mut crosshair) = (0, None);
        for k in 1..6 {
            send(&mut h, egui::Event::MouseMoved(vec2(4.0, 0.0)));
            send(&mut h, egui::Event::PointerMoved(elsewhere + vec2(k as f32 * 4.0, 0.0)));
            step(&mut h);
            moved += warps(&h);
            let p = h.ctx.input(|i| i.pointer.latest_pos()).unwrap();
            assert!(view.contains(p) && *crosshair.get_or_insert(p) == p, "the pointer stays at the crosshair: {p:?}");
            assert!(h.state().sculpt3d.captured(), "the view keeps the mouse");
            assert_eq!(h.ctx.memory(|m| m.focused()), None, "no widget takes the focus");
        }
        send(&mut h, egui::Event::PointerButton { pos: elsewhere, button: egui::PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
        step(&mut h);
        assert_eq!(h.state().doc.history.undo_count(), steps + 1, "the right button sculpts at the crosshair");
        assert_eq!(h.state().doc.history.undo_label(), Some("Sculpt"));
        if cfg!(target_os = "macos") {
            assert_eq!(moved, 0, "the locked pointer is never moved (it would undo the lock)");
        }
        // Esc lets it go; the pointer is the system's again.
        h.key_press(Key::Escape);
        step(&mut h);
        assert!(!h.state().sculpt3d.captured());
        send(&mut h, egui::Event::PointerMoved(elsewhere));
        step(&mut h);
        assert_eq!(h.ctx.input(|i| i.pointer.latest_pos()), Some(elsewhere));
    }

    /// Paint terrain's Tool Settings say what the right button puts back:
    /// by height on a map whose terrains came from its height, the map's own
    /// terrain once it has one. Renders both (MASSIF_SNAPSHOT_DIR).
    #[test]
    fn paint_terrain_says_what_the_right_button_puts_back() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.active = 2; // Painting
        h.state_mut().tools.select(Tool::PaintTerrain);
        h.run_steps(5);
        assert_eq!(h.state().doc.map_terrain(), None, "a generated map: its height chose its terrains");
        assert!(h.query_by_label("By height, for now").is_some());
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("unpaint-by-height.png")).unwrap();
        }
        h.state_mut().doc.world.meta.fill = Some(massif_core::TerrainId::ROCK);
        h.run_steps(3);
        assert!(h.query_by_label("Right button: back to Rock, the map's own terrain.").is_some());
        // The panel scrolled down to it.
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let panel = areas.into_iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::ToolSettings)).map(|(_, r)| r).unwrap();
        h.event(egui::Event::PointerMoved(panel.center()));
        h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -400.0), phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel, modifiers: Modifiers::NONE });
        h.run_steps(10);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("unpaint-rock.png")).unwrap();
        }
        h.state_mut().tools.unpaint = crate::tools::Unpaint::ByHeight;
        h.run_steps(3);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("unpaint-height-chosen.png")).unwrap();
        }
    }

    /// A first-time user finds the Gradient brush: in the Paint row, its
    /// own icon marked with a dot until it is tried, its card saying it is
    /// new; and among Paint terrain's ways to paint (One terrain, Noise,
    /// Gradient), which the Gradient brush shows too, to go back. Renders
    /// each (MASSIF_SNAPSHOT_DIR).
    #[test]
    fn a_first_time_user_finds_the_gradient_brush() {
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.active = 2; // Painting
        h.state_mut().tools.select(Tool::PaintTerrain);
        h.run_steps(5);
        assert!(h.state().tools.untried(Tool::Gradient), "new, not tried yet: marked");
        h.get_by_label("Gradient brush").hover();
        h.run_steps(30);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("gradient-find-tools.png")).unwrap();
        }
        assert!(h.query_by_label("New").is_some(), "its card says it is new");
        // Paint terrain's third way to paint.
        h.get_by_label("Gradient").click();
        h.run_steps(3);
        assert_eq!(h.state().tools.tool, Tool::Gradient);
        assert!(!h.state().tools.untried(Tool::Gradient), "tried: no longer marked");
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("gradient-find-settings.png")).unwrap();
        }
        // And back.
        h.get_by_label("Noise").click();
        h.run_steps(3);
        assert_eq!(h.state().tools.tool, Tool::PaintTerrain);
        assert!(h.state().tools.pattern.on);
    }

    #[test]
    fn walking_in_3d_sculpt() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        // The highest dry land of the map.
        let (x, z, top) = {
            let doc = &h.state().doc;
            let (lo, hi) = doc.dimension().bounds().unwrap();
            let t = massif_core::coords::TILE_SIZE;
            let mut best = (0, 0, f32::MIN);
            for z in (lo.z * t..(hi.z + 1) * t).step_by(8) {
                for x in (lo.x * t..(hi.x + 1) * t).step_by(8) {
                    if let Some(p) = crate::tools::probe(doc, x, z)
                        && p.fluid == massif_core::tile::NO_FLUID
                        && p.height > best.2
                    {
                        best = (x, z, p.height);
                    }
                }
            }
            best
        };
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            app.tools.sculpt.tool = Tool::Build;
            app.tools.brush.radius = 3.0;
            app.tools.brush.strength = 1.0;
        }
        // (A new map frames the whole of it, at the first frame.)
        h.run_steps(20);
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [x as f64 + 0.5, top as f64, z as f64 + 0.5], yaw: 0.6, pitch: 1.3, distance: 40.0 };
        h.run_steps(5);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas
            .into_iter()
            .find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d))
            .map(|(_, r)| r)
            .expect("the 3D Sculpt workspace shows its editor");
        h.state_mut().tools.sculpt.player = true;
        h.event(egui::Event::PointerMoved(view.center() + vec2(0.0, 60.0)));
        h.run_steps(20);
        let p = h.state().sculpt3d.player().expect("walking");
        assert!(p.on_ground && !p.flying, "{p:?}");
        assert!((p.feet[1] - top as f64).abs() < 4.0, "on the land looked at: {:?}, top {top}", p.feet);
        let eye = h.state().sculpt3d.camera.eye();
        assert!((eye[1] - p.feet[1] - crate::player::EYE).abs() < 0.01, "the camera at the eyes");
        assert!((eye[0] - p.feet[0]).hypot(eye[2] - p.feet[2]) < 0.01);

        // The click that lends the view the mouse sculpts nothing.
        let steps = h.state().doc.history.undo_count();
        let press = |pressed, button| egui::Event::PointerButton {
            pos: view.center() + vec2(0.0, 60.0),
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        h.event(press(true, egui::PointerButton::Primary));
        h.run_steps(3);
        h.event(press(false, egui::PointerButton::Primary));
        h.run_steps(3);
        assert_eq!(h.state().doc.history.undo_count(), steps, "the first click only takes the mouse");
        // Looking down a little, the crosshair is on the land: the buttons
        // sculpt there.
        h.event(egui::Event::MouseMoved(vec2(0.0, 150.0)));
        h.run_steps(2);
        assert!(h.state().sculpt3d.hover().is_some(), "the crosshair points at the land");
        h.event(press(true, egui::PointerButton::Primary));
        h.run_steps(4);
        h.event(press(false, egui::PointerButton::Primary));
        h.run_steps(4);
        assert_eq!(h.state().doc.history.undo_label(), Some("Sculpt"));
        h.render().unwrap().save(dir.join("walk.png")).unwrap();
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("walk-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();

        // Space twice: flying; held, up.
        h.event(egui::Event::MouseMoved(vec2(0.0, -150.0)));
        // (A frame an event: pressed, then let go.)
        h.key_press(Key::Space);
        h.step();
        h.key_press(Key::Space);
        h.step();
        assert!(h.state().sculpt3d.player().unwrap().flying, "Space twice flies");
        let y = h.state().sculpt3d.player().unwrap().feet[1];
        h.key_down(Key::Space);
        h.run_steps(8);
        h.key_up(Key::Space);
        h.step();
        let p = h.state().sculpt3d.player().unwrap();
        assert!(p.flying && p.feet[1] > y + 3.0, "up: {y} → {:?}", p.feet);
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("walk-fly-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();

        // Esc gives the mouse back; then a drag, and the arrows, turn the
        // view all the same.
        h.key_press(Key::Escape);
        h.step();
        let yaw = h.state().sculpt3d.camera.yaw;
        let from = view.center() + vec2(0.0, 60.0);
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        for k in 1..=8 {
            h.event(egui::Event::PointerMoved(from + vec2(k as f32 * 12.0, 0.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: from + vec2(96.0, 0.0), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.step();
        let dragged = h.state().sculpt3d.camera.yaw;
        assert!(dragged < yaw - 0.2, "a drag turns the view: {yaw} → {dragged}");
        assert!(h.state().sculpt3d.player().is_some());
        h.key_down(Key::ArrowLeft);
        h.run_steps(4);
        h.key_up(Key::ArrowLeft);
        h.step();
        assert!(h.state().sculpt3d.camera.yaw > dragged + 0.2, "the arrows turn it");
        // Slope: walls in the colour of their column.
        h.state_mut().sculpt3d.mode = crate::map_render::MapMode::Slope;
        h.run_steps(30);
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("walk-slope-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        h.state_mut().sculpt3d.mode = crate::map_render::MapMode::Terrain;
        h.run_steps(5);
        // Player off, the orbit camera returns, from the eyes.
        let eye = h.state().sculpt3d.camera.eye();
        h.state_mut().tools.sculpt.player = false;
        h.run_steps(2);
        assert!(h.state().sculpt3d.player().is_none());
        let camera = h.state().sculpt3d.camera;
        assert!(camera.distance >= 8.0, "{camera:?}");
        let back = camera.eye();
        assert!((back[0] - eye[0]).hypot(back[2] - eye[2]) < 1.0 && (back[1] - eye[1]).abs() < 1.0, "{eye:?} → {back:?}");
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
            app.tools.brush.radius = 14.0;
            app.tools.brush.strength = 1.0;
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
        let wheel = |h: &mut egui_kittest::Harness<'_, MassifApp>, modifiers: Modifiers| {
            h.event(egui::Event::PointerMoved(at));
            h.event(egui::Event::ModifiersChanged(modifiers));
            h.event(egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Point,
                delta: vec2(0.0, 120.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Unknown,
                modifiers,
            });
            h.run_steps(10);
            h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
            h.step();
        };
        // ⌘ + wheel: the size of the brush.
        let radius = h.state().tools.brush.radius;
        wheel(&mut h, Modifiers::COMMAND);
        assert!(h.state().tools.brush.radius > radius, "{} after {radius}", h.state().tools.brush.radius);
        // Alt + wheel: toward the point under the cursor.
        let (target, distance) = (h.state().sculpt3d.camera.target, h.state().sculpt3d.camera.distance);
        let pointed = h.state().sculpt3d.hover().expect("the cursor points at the terrain").point;
        wheel(&mut h, Modifiers::ALT);
        let camera = h.state().sculpt3d.camera;
        assert!(camera.distance < distance, "nearer: {} after {distance}", camera.distance);
        let gap = |t: [f64; 3]| ((t[0] - pointed[0]).powi(2) + (t[2] - pointed[2]).powi(2)).sqrt();
        assert!(gap(camera.target) < gap(target), "toward the point: {:?} after {target:?}", camera.target);
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target, distance, ..camera };
        h.run_steps(5);
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
        // Paint, in 3D: the terrain of the columns under the brush.
        let sand = h.state().doc.world.registry.terrain_by_name("Sand").unwrap();
        {
            let app = h.state_mut();
            app.tools.sculpt.tool = Tool::PaintTerrain;
            app.tools.terrain = sand.0;
            app.tools.brush.strength = 1.0;
        }
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(3);
        let hit = h.state().sculpt3d.hover().expect("the cursor points at the terrain");
        h.event(press(true, egui::PointerButton::Primary));
        for _ in 0..10 {
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.event(press(false, egui::PointerButton::Primary));
        h.run_steps(20);
        assert_eq!(h.state().doc.history.undo_label(), Some("Paint terrain"));
        let (x, z) = (hit.block.0, hit.block.2);
        let tile = h.state().doc.dimension().ground().view(TilePos::containing(x, z)).unwrap();
        assert_eq!(tile.terrain.get(massif_core::coords::local_index(x, z)), sand.0, "painted at ({x}, {z})");
        h.render().unwrap().save(dir.join("sculpt-paint.png")).unwrap();
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

    /// The pictures behind the home screen (assets/backdrops), rendered from
    /// templates of the terrain graph at dusk. Writes them, as JPEG, to
    /// MASSIF_BACKDROP_DIR; MASSIF_BACKDROP_ONLY (a name) renders one,
    /// MASSIF_BACKDROP_CAM (yaw,pitch,distance,height) and
    /// MASSIF_BACKDROP_SUN (direction,height) try other framings of it.
    /// Needs a GPU.
    #[test]
    fn render_the_home_backdrops() {
        use massif_core::graph::Template;
        let Some(dir) = std::env::var_os("MASSIF_BACKDROP_DIR").map(PathBuf::from) else {
            return;
        };
        let numbers = |name: &str| -> Option<Vec<f32>> { std::env::var(name).ok().map(|v| v.split(',').filter_map(|n| n.trim().parse().ok()).collect()) };
        let only = std::env::var("MASSIF_BACKDROP_ONLY").ok();
        // Name, template, camera (yaw, pitch, distance, height looked at),
        // sun (direction, height), haze.
        // Name, template, camera (yaw, pitch, distance, height looked at),
        // sun (direction, height), haze, exaggeration.
        type Preset = (&'static str, Template, [f32; 4], [f32; 2], f32, f32);
        let presets: [Preset; 4] = [
            ("floating-mountains", Template::FloatingMountains, [0.4, 0.18, 0.75, 120.0], [240.0, 8.0], 0.4, 1.0),
            ("archipelago-sunset", Template::Archipelago, [3.0, 0.16, 0.7, 20.0], [260.0, 5.0], 0.45, 1.0),
            ("karst-twilight", Template::KarstTowers, [1.7, 0.07, 0.45, 90.0], [200.0, 4.0], 0.5, 1.0),
            ("canyon-sunset", Template::DesertCanyon, [3.6, 0.16, 0.5, 40.0], [235.0, 7.0], 0.3, 1.3),
        ];
        let exaggeration = std::env::var("MASSIF_BACKDROP_EXAGGERATION").ok().and_then(|v| v.parse::<f32>().ok());
        for (name, template, cam, sun, haze, relief) in presets {
            if only.as_deref().is_some_and(|o| o != name) {
                continue;
            }
            let cam = numbers("MASSIF_BACKDROP_CAM").filter(|v| v.len() == 4).unwrap_or(cam.to_vec());
            let sun = numbers("MASSIF_BACKDROP_SUN").filter(|v| v.len() == 2).unwrap_or(sun.to_vec());
            let mut h = harness(ThemeChoice::Dark);
            {
                let app = h.state_mut();
                app.layout.activate(8); // Render, on the terrain side
                let sea = app.doc.world.meta.sea_level as f32;
                let g = template.graph(&app.doc.world.registry, sea, 4096.0);
                app.doc.world.meta.graph = Some(g);
                app.graph.graph_replaced();
            }
            wait_for_graph(&mut h);
            {
                let v = &mut h.state_mut().graph.view;
                v.render.clouds = false;
                v.render.sea = true;
                v.render.haze = haze;
                v.render.exaggeration = exaggeration.unwrap_or(relief);
                v.render.sun_azimuth = sun[0];
                v.render.sun_elevation = sun[1];
                v.camera.yaw = cam[0];
                v.camera.pitch = cam[1];
                v.camera.distance = cam[2];
                v.camera.target[1] = cam[3];
            }
            h.run_steps(8);
            let (w, hgt, rgba) = h.state_mut().graph.view.render_image([1920, 1080]).expect("rendered");
            let rgb = image::RgbImage::from_fn(w, hgt, |x, y| {
                let i = ((y * w + x) * 4) as usize;
                image::Rgb([rgba[i], rgba[i + 1], rgba[i + 2]])
            });
            let mut out = std::fs::File::create(dir.join(format!("{name}.jpg"))).unwrap();
            image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 86).encode_image(&rgb).unwrap();
        }
    }

    /// The starter graph's viewport over a small terrain and over a huge
    /// one (a sample every hundred blocks), for the eye (MASSIF_SNAPSHOT_DIR):
    /// the land keeps its colours whatever the size.
    #[test]
    fn graph_viewport_at_every_size() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        for (name, side) in [("starter", 4096u32), ("starter", 46592), ("archipelago", 4096)] {
            let mut h = harness(ThemeChoice::Dark);
            {
                let app = h.state_mut();
                app.layout.activate(7); // Graph, on the terrain side
                let get = |name: &str| app.doc.world.registry.terrain_by_name(name).map_or(0, |t| t.0);
                let mut graph = match name {
                    "starter" => massif_core::graph::Graph::starter(get("Grass"), massif_core::TerrainId::ROCK.0, get("Snow"), get("Gravel")),
                    _ => massif_core::graph::Template::Archipelago.graph(&app.doc.world.registry, app.doc.world.meta.sea_level as f32, side as f32),
                };
                graph.size = Some(side);
                app.doc.world.meta.graph = Some(graph);
                app.graph.graph_replaced();
            }
            wait_for_graph(&mut h);
            let (fw, fh, full) = h.state_mut().graph.view.render_image([960, 540]).expect("rendered");
            image::save_buffer(dir.join(format!("graph-render-{name}-{side}.png")), &full, fw, fh, image::ColorType::Rgba8).unwrap();
            let (w, hgt, rgba) = h.state().graph.view.read_back().expect("the viewport rendered");
            image::save_buffer(dir.join(format!("graph-viewport-{name}-{side}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            // How dark the land is: the mean of the middle of the picture.
            let (mut sum, mut count) = (0u64, 0u64);
            for y in hgt / 3..2 * hgt / 3 {
                for x in w / 3..2 * w / 3 {
                    let i = ((y * w + x) * 4) as usize;
                    sum += rgba[i] as u64 + rgba[i + 1] as u64 + rgba[i + 2] as u64;
                    count += 3;
                }
            }
            eprintln!("{name} {side} blocks: mean of the middle {}", sum / count.max(1));
        }
    }

    /// Converting the starter graph over a map wider than deep, a sample
    /// every few blocks (as on a very large map): every column of every tile
    /// gets the build's height, the first rows and columns too.
    #[test]
    fn converting_leaves_no_column_out() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .with_pixels_per_point(1.0)
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                MassifApp::for_tests(generate(&GenParams {
                    tiles_x: 8,
                    tiles_z: 6,
                    landform: Landform::Flat,
                    ..GenParams::default()
                }))
            });
        {
            let app = h.state_mut();
            app.layout.activate(7); // Graph, on the terrain side
            let get = |name: &str| app.doc.world.registry.terrain_by_name(name).map_or(0, |t| t.0);
            let mut starter = massif_core::graph::Graph::starter(get("Grass"), massif_core::TerrainId::ROCK.0, get("Snow"), get("Gravel"));
            // MASSIF_GRAPH_SIDE: a square of that many blocks rather than the map.
            starter.size = std::env::var("MASSIF_GRAPH_SIDE").ok().and_then(|v| v.parse().ok());
            app.doc.world.meta.graph = Some(starter);
            app.graph.graph_replaced();
        }
        wait_for_graph(&mut h);
        h.state_mut().graph.draft = Some(360);
        // MASSIF_GRAPH_OVER: over the map (levels), rather than into its ground.
        if std::env::var_os("MASSIF_GRAPH_OVER").is_some() {
            h.state_mut().graph.over = Some(true);
        }
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
        h.run_steps(5);
        // Columns left without a height, tile by tile.
        let ground = h.state().doc.dimension().ground();
        let mut empty: Vec<(TilePos, usize)> = Vec::new();
        let mut tiles: Vec<TilePos> = ground.positions().collect();
        tiles.sort_by_key(|t| (t.z, t.x));
        let dim = h.state().doc.dimension();
        let mut spans = Vec::new();
        for t in &tiles {
            let tile = ground.view(*t).unwrap();
            let stack = massif_core::TileStack::new(dim, *t, false).unwrap();
            let n = (0..128 * 128)
                .filter(|&i| {
                    stack.spans(i, &mut spans);
                    !tile.top.get(i).is_finite() || spans.is_empty()
                })
                .count();
            if n > 0 {
                empty.push((*t, n));
            }
        }
        eprintln!("levels: {:?}", dim.levels.iter().map(|l| (l.name.clone(), l.len())).collect::<Vec<_>>());
        eprintln!("{} tiles, those with empty columns: {empty:?}", tiles.len());
        h.state_mut().layout.activate(0);
        h.run_steps(20);
        h.render().unwrap().save(dir.join("converted-map.png")).unwrap();
        assert!(empty.is_empty(), "columns without a height: {empty:?}");
    }

    /// The map that replaces the open one while it works: a small one, two
    /// tiles a side, as a new map of 256 blocks.
    fn open_another_map(app: &mut MassifApp) {
        let small = generate(&GenParams {
            tiles_x: 2,
            tiles_z: 2,
            landform: Landform::Flat,
            ..GenParams::default()
        });
        app.next_generation += 1;
        app.doc = crate::doc::Document::new(small, None, app.next_generation);
    }

    fn ground_tiles(app: &MassifApp) -> Vec<massif_core::TilePos> {
        let mut tiles: Vec<_> = app.doc.dimension().ground().positions().collect();
        tiles.sort_by_key(|p| (p.z, p.x));
        tiles
    }

    fn status_text(app: &MassifApp) -> String {
        app.status.as_ref().map(|s| s.text.clone()).unwrap_or_default()
    }

    /// A conversion started on one map and done once another is open
    /// writes nothing into the new one: its tiles landed among the new
    /// map's, and left rows of holes between them.
    #[test]
    fn a_conversion_stays_with_its_map() {
        use massif_core::graph::Template;
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        let ctx = h.ctx.clone();
        {
            let app = h.state_mut();
            app.graph.use_template(&mut app.doc, Template::Alpine);
            app.graph.draft = Some(128);
            let graph = app.doc.world.meta.graph.clone().unwrap();
            app.graph.start_build(&ctx, &mut app.doc, &graph, None);
            open_another_map(app);
        }
        let before = ground_tiles(h.state());
        let started = std::time::Instant::now();
        while h.state().graph.converting() {
            assert!(started.elapsed().as_secs() < 60, "the conversion did not end");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        h.run_steps(3);
        assert_eq!(ground_tiles(h.state()), before, "the new map's tiles, and no others");
        assert_eq!(h.state().doc.history.undo_label(), None, "nothing written");
        let said = status_text(h.state());
        assert!(said.contains("Conversion stopped"), "{said}");
    }

    /// Auto paint worked out on one map and done once another is open, at
    /// the same revision (both just opened), is not put into the new one;
    /// nor does saving the first give the new one its file.
    #[test]
    fn a_job_stays_with_its_map() {
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(2);
        let ctx = h.ctx.clone();
        let wait = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            let started = std::time::Instant::now();
            while !h.state().jobs.is_empty() {
                assert!(started.elapsed().as_secs() < 60, "the job did not end");
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            h.run_steps(2);
        };
        h.state_mut().tools.run_auto = Some(false);
        h.step();
        assert_eq!(h.state().jobs.len(), 1, "auto paint started");
        open_another_map(h.state_mut());
        let before = ground_tiles(h.state());
        wait(&mut h);
        assert_eq!(ground_tiles(h.state()), before, "the new map's tiles, and no others");
        assert_eq!(h.state().doc.history.undo_label(), None, "nothing painted");
        let said = status_text(h.state());
        assert!(said.contains("another map was opened meanwhile"), "{said}");
        // Saved, as the next map opens.
        let file = std::env::temp_dir().join(format!("massif-saved-{}.massif", std::process::id()));
        {
            let app = h.state_mut();
            app.doc.path = Some(file.clone());
            assert!(app.save(&ctx, false));
            open_another_map(app);
        }
        wait(&mut h);
        let _ = std::fs::remove_file(&file);
        assert_eq!(h.state().doc.path, None, "the new map keeps no file of the other's");
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
        // The view's settings, over it.
        h.state_mut().graph.render_open = true;
        h.state_mut().graph.view.render.clouds = true;
        h.run_steps(6);
        h.render().unwrap().save(dir.join("graph-render-popover.png")).unwrap();
        h.state_mut().graph.render_open = false;
        h.state_mut().graph.view.render.clouds = false;
        wait_for_graph(&mut h);
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
        // Hovering: what the noise would paint under the brush, before it
        // is painted.
        h.state_mut().tools.pattern.pattern.seed = 4;
        h.event(egui::Event::PointerMoved(c + vec2(-150.0, 60.0)));
        h.run_steps(6);
        h.render().unwrap().save(dir.join("noise-preview.png")).unwrap();
        // The whole map, in the background, as one step to undo.
        h.state_mut().tools.pattern.paint_map = true;
        for _ in 0..200 {
            h.run_steps(5);
            if h.state().jobs.is_empty() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert_eq!(h.state().doc.history.undo_label(), Some("Noise paint"));
        assert!(h.state().tools.pattern.done.is_some_and(|n| n > 10_000), "{:?}", h.state().tools.pattern.done);
        h.event(egui::Event::PointerMoved(c + vec2(2000.0, 0.0)));
        h.run_steps(10);
        h.render().unwrap().save(dir.join("noise-whole-map.png")).unwrap();
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

    /// The Road tool: points clicked on the map, the road edited (a point
    /// added by dragging the road, one taken off with a right-click, one
    /// with Delete), Enter, and the road is graded and painted in one step
    /// to undo. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn roads_are_drawn_on_the_map() {
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
            app.tools.select(Tool::Road);
            app.tools.road.width = 7.0;
        }
        h.run_steps(5);
        h.render().unwrap().save(dir.join("road-start.png")).unwrap();
        let (high, low) = river_ends(h.state());
        let at = |h: &egui_kittest::Harness<'_, MassifApp>, p: (f32, f32)| h.state().maps.values().next().unwrap().screen_of(p.0 as f64, p.1 as f64);
        let (a, b) = (at(&h, high), at(&h, low));
        let side = vec2(-(b - a).y, (b - a).x).normalized() * 70.0;
        for p in [a - (b - a) * 0.3, a + side, b, b + (b - a) * 0.3 - side] {
            click_on(&mut h, p, egui::PointerButton::Primary);
        }
        assert_eq!(h.state().tools.road_draft.as_ref().unwrap().points.len(), 4);
        // A point added by dragging the road.
        let course = h.state().tools.road_draft.as_ref().unwrap().plan.course.clone();
        let mid = at(&h, course[course.len() / 3]);
        drag_on(&mut h, mid, mid + vec2(-50.0, 40.0));
        assert_eq!(h.state().tools.road_draft.as_ref().unwrap().points.len(), 5);
        // One taken off with a right-click, another with Delete.
        click_on(&mut h, a + side, egui::PointerButton::Secondary);
        assert_eq!(h.state().tools.road_draft.as_ref().unwrap().points.len(), 4);
        click_on(&mut h, b, egui::PointerButton::Primary);
        h.event(egui::Event::Key {
            key: egui::Key::Delete,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(3);
        let draft = h.state().tools.road_draft.clone().unwrap();
        assert_eq!(draft.points.len(), 3);
        assert!(draft.plan.within, "{}", draft.summary());
        h.event(egui::Event::PointerMoved(mid + vec2(200.0, 200.0)));
        h.run_steps(3);
        h.render().unwrap().save(dir.join("road-draft.png")).unwrap();
        let revision = h.state().doc.revision;
        h.event(egui::Event::Key {
            key: egui::Key::Enter,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(10);
        assert_eq!(h.state().doc.history.undo_label(), Some("Road"));
        assert!(h.state().tools.road_draft.is_none());
        let note = h.state().tools.road_note.clone().unwrap_or_default();
        assert!(note.starts_with("Built a road"), "{note}");
        // Painted with its surface along its middle.
        let path = h.state().doc.world.registry.terrain_by_name("Trodden path").expect("the mix was added");
        let mid = draft.plan.course[draft.plan.course.len() / 2];
        let probe = crate::tools::probe(&h.state().doc, mid.0.floor() as i32, mid.1.floor() as i32).unwrap();
        assert!(probe.terrain == path.0 || draft.plan.bridge[draft.plan.course.len() / 2], "{probe:?}");
        h.render().unwrap().save(dir.join("road-built.png")).unwrap();
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
        image::save_buffer(dir.join("road-built-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        // Undone in one step.
        h.state_mut().doc.undo();
        assert!(h.state().doc.revision != revision);
        let probe = crate::tools::probe(&h.state().doc, mid.0.floor() as i32, mid.1.floor() as i32).unwrap();
        assert_ne!(probe.terrain, path.0);
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

    /// The Gradient brush: a dark pit painted in grass, the gradient laid
    /// around its edge with the mouse, one undo; then its editor. Renders
    /// with MASSIF_SNAPSHOT_DIR (with the game's textures when a Minecraft
    /// jar is installed).
    #[test]
    fn gradient_brush_blends_an_edge_and_its_editor_renders() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        for (theme, suffix) in [(ThemeChoice::Light, "light"), (ThemeChoice::Dark, "dark")] {
            let mut h = harness(theme);
            let ctx = h.ctx.clone();
            if !massif_assets::find_jars().is_empty() {
                h.state_mut().blocks.load(&ctx, None, Vec::new());
                let started = std::time::Instant::now();
                while h.state().blocks.catalogue.is_none() && started.elapsed().as_secs() < 60 {
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(20));
                }
            }
            h.run_steps(5);
            // Dry land well away from the water.
            let (cx, cz) = {
                let doc = &h.state().doc;
                let dry = |x: i32, z: i32| crate::tools::probe(doc, x, z).is_some_and(|p| p.fluid == massif_core::NO_FLUID);
                (-400..400)
                    .step_by(8)
                    .flat_map(|x| (-400..400).step_by(8).map(move |z| (x, z)))
                    .find(|&(x, z)| (-60..=60).step_by(10).all(|d| dry(x + d, z) && dry(x, z + d) && dry(x + d, z + d) && dry(x + d, z - d)))
                    .expect("dry land")
            };
            let id = h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view");
            {
                let app = h.state_mut();
                let view = app.maps.get_mut(&id).unwrap();
                view.center = [cx as f64, cz as f64];
                view.zoom = 7.0;
                // A square of grass, a patch of stone in it.
                let reg = &app.doc.world.registry;
                let grass = reg.terrain_by_name("Grass").unwrap();
                app.tools.select(Tool::PaintTerrain);
                app.tools.pattern.on = false;
                app.tools.terrain = grass.0;
                app.tools.brush = massif_core::Brush::symmetric(massif_core::BrushShape::Square, massif_core::Profile::Constant, 56.0);
            }
            h.run_steps(3);
            let at = |h: &egui_kittest::Harness<'_, MassifApp>, x: f32, z: f32| h.state().maps[&id].screen_of(x as f64, z as f64);
            let p = at(&h, cx as f32, cz as f32);
            drag_on(&mut h, p, p + vec2(1.0, 0.0));
            {
                let app = h.state_mut();
                app.tools.terrain = app.doc.world.registry.terrain_by_name("Deepslate").unwrap().0;
                app.tools.brush = massif_core::Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 20.0);
            }
            h.run_steps(2);
            let p = at(&h, cx as f32, cz as f32);
            drag_on(&mut h, p - vec2(40.0, 0.0), p + vec2(40.0, 0.0));
            // The gradient around the patch's edge, in one stroke.
            {
                let app = h.state_mut();
                app.tools.select(Tool::Gradient);
                app.tools.gradient.gradient = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Pit to grass").unwrap();
                app.tools.brush = massif_core::Brush::symmetric(massif_core::BrushShape::Circle, massif_core::Profile::Constant, 14.0);
            }
            h.run_steps(2);
            let ring: Vec<egui::Pos2> = (0..=16).map(|k| {
                let a = k as f32 / 16.0 * std::f32::consts::TAU;
                at(&h, cx as f32 + 26.0 * a.cos(), cz as f32 + 22.0 * a.sin())
            }).collect();
            h.event(egui::Event::PointerMoved(ring[0]));
            h.step();
            h.event(egui::Event::PointerButton { pos: ring[0], button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
            h.step();
            for p in &ring[1..] {
                for k in 1..=3 {
                    h.event(egui::Event::PointerMoved(*p + vec2(0.0, k as f32 * 0.01)));
                    h.step();
                }
            }
            h.event(egui::Event::PointerButton { pos: *ring.last().unwrap(), button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
            h.event(egui::Event::PointerMoved(at(&h, cx as f32 + 200.0, cz as f32 + 200.0)));
            h.run_steps(20);
            let app = h.state();
            assert_eq!(app.doc.history.undo_label(), Some("Gradient brush"));
            assert_eq!(app.doc.world.meta.gradients.len(), 1, "the map keeps the gradient");
            // Two greys now ring the pit, where its edge was.
            let reg = &app.doc.world.registry;
            let rim = ["Andesite", "Gravel"].map(|n| reg.terrain_by_name(n).unwrap().0);
            let ringed = (0..64)
                .filter(|k| {
                    let a = *k as f32 / 64.0 * std::f32::consts::TAU;
                    let (x, z) = ((cx as f32 + 16.0 * a.cos()) as i32, (cz as f32 + 16.0 * a.sin()) as i32);
                    (0..16).any(|d| crate::tools::probe(&app.doc, x + (d as f32 * a.cos()) as i32, z + (d as f32 * a.sin()) as i32).is_some_and(|p| rim.contains(&p.terrain)))
                })
                .count();
            h.render().unwrap().save(dir.join(format!("gradient-map-{suffix}.png"))).unwrap();
            assert!(ringed > 32, "a rim around the patch: {ringed} of 64");
            // The editor on the gradient of the terraformers' sketch, its
            // blobs chosen.
            h.state_mut().tools.gradient.gradient = massif_core::gradient::built_in().into_iter().find(|g| g.name == "Deciduous forest to grass").unwrap();
            h.state_mut().tools.gradient.edit(false);
            h.state_mut().tools.gradient.editor.as_mut().unwrap().selected = 3;
            h.run_steps(24);
            h.render().unwrap().save(dir.join(format!("gradient-editor-{suffix}.png"))).unwrap();
            h.state_mut().tools.gradient.editor.as_mut().unwrap().selected = 2;
            h.run_steps(24);
            h.render().unwrap().save(dir.join(format!("gradient-editor-band-{suffix}.png"))).unwrap();
        }
    }

    /// The Gradient brush's editor on a gradient of Conquest Reforged's
    /// blocks, as the terraformers who asked for it would build one: the
    /// bands with the mod's textures, a transition chosen, then a band's
    /// block picked straight from the mod's blocks. Renders with
    /// MASSIF_SNAPSHOT_DIR and MASSIF_CONQUEST_JAR (the mod's jar).
    #[test]
    fn gradient_editor_with_conquest_blocks() {
        use egui_kittest::kittest::Queryable;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let Some((_, Some(cat))) = crate::gradient_ui::shots::catalogues() else {
            eprintln!("needs Minecraft and MASSIF_CONQUEST_JAR: skipping");
            return;
        };
        for (theme, suffix) in [(ThemeChoice::Light, "light"), (ThemeChoice::Dark, "dark")] {
            let mut h = harness(theme);
            {
                let app = h.state_mut();
                app.blocks.catalogue = Some(std::sync::Arc::new(cat.clone()));
                massif_assets::apply_colors(&mut app.doc.world.registry, &cat);
                let shots = crate::gradient_ui::shots::conquest();
                for (_, g, _, _) in &shots {
                    for name in g.terrains() {
                        crate::gradient_ui::shots::terrain(&mut app.doc.world.registry, &cat, name);
                    }
                    app.doc.world.meta.gradients.push(g.clone());
                }
                app.tools.select(Tool::Gradient);
                app.tools.gradient.gradient = shots.iter().find(|s| s.0 == "conquest-sketch-30-blocks").unwrap().1.clone();
                app.tools.gradient.edit(false);
                app.tools.gradient.editor.as_mut().unwrap().selected = 3;
            }
            h.run_steps(24);
            h.render().unwrap().save(dir.join(format!("conquest-editor-{suffix}.png"))).unwrap();
            // The rock example, its gold-green rim chosen.
            {
                let app = h.state_mut();
                let g = crate::gradient_ui::shots::conquest().into_iter().find(|s| s.0 == "conquest-rock-gold-green-rim").unwrap().1;
                let ed = app.tools.gradient.editor.as_mut().unwrap();
                ed.draft = g;
                ed.selected = 4;
            }
            h.run_steps(24);
            h.render().unwrap().save(dir.join(format!("conquest-editor-rim-{suffix}.png"))).unwrap();
            if theme == ThemeChoice::Light {
                // A band's block, searched among the mod's.
                h.state_mut().tools.gradient.editor.as_mut().unwrap().selected = 0;
                h.run_steps(4);
                h.ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("gradient band pick", 0usize)), "limestone".to_owned()));
                h.get_by_label_contains("Gray Limestone  ").click();
                h.run_steps(12);
                h.render().unwrap().save(dir.join("conquest-editor-block-picker.png")).unwrap();
            }
        }
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

    /// Auto biome with an image brush, hovered beside a dab it painted:
    /// the columns it would change in their new colour, those it painted
    /// already hatched, and under the map what they become and how much
    /// stays. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn auto_biome_shows_what_stays_as_it_is() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        let n = 64usize;
        let mut blob = vec![0u8; n * n];
        for j in 0..n {
            for i in 0..n {
                let (x, y) = (i as f32 / n as f32 - 0.5, j as f32 / n as f32 - 0.5);
                let r = 0.42 + 0.06 * (5.0 * y.atan2(x)).sin() + 0.03 * (11.0 * y.atan2(x)).cos();
                if (x * x + y * y).sqrt() < r {
                    blob[j * n + i] = 255;
                }
            }
        }
        {
            let app = h.state_mut();
            app.doc = crate::doc::Document::new(generate(&GenParams { tiles_x: 8, tiles_z: 8, ..GenParams::default() }), None, 13);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::AutoBiome);
            app.tools.brush.mask = Some(std::sync::Arc::new(massif_core::brush::BrushMask::new("Blob", n, n, blob)));
            app.tools.brush.radius = 70.0;
            app.tools.brush.rotation = 20.0;
            app.tools.brush.strength = 1.0;
        }
        h.run_steps(5);
        let (center, zoom) = {
            let m = h.state().maps.values().next().unwrap();
            (m.center, m.zoom)
        };
        let screen = |h: &egui_kittest::Harness<'_, MassifApp>, x: f64, z: f64| h.state().maps.values().next().unwrap().screen_of(x, z);
        let a = screen(&h, center[0], center[1]);
        drag_on(&mut h, a, a + vec2(1.0, 0.0));
        let note = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        assert!(!note.starts_with("Nothing"), "{note}");
        // Beside it, half over what it painted.
        let b = screen(&h, center[0] + 60.0, center[1] + 15.0);
        h.event(egui::Event::PointerMoved(b));
        h.run_steps(5);
        eprintln!("zoom {zoom}");
        h.render().unwrap().save(dir.join("auto-biome-preview.png")).unwrap();
        // Over it again: nothing left to change, and it says so.
        h.event(egui::Event::PointerMoved(a + vec2(1.0, 0.0)));
        h.run_steps(5);
        h.render().unwrap().save(dir.join("auto-biome-preview-done.png")).unwrap();
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

    /// A speckled custom brush over the map, at three sizes: its outline
    /// drawn round its specks, merged where they are too small to see, and
    /// no line straying across the map. The brush is "brushes 3" of
    /// MASSIF_BRUSH_DIRS (folders, separated by `:`) when there, else specks
    /// made here. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn a_speckled_brush_is_outlined_cleanly() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let found = std::env::var_os("MASSIF_BRUSH_DIRS")
            .map(|v| std::env::split_paths(&v).map(|d| d.join("brush3.png")).filter(|p| p.is_file()).collect::<Vec<_>>())
            .and_then(|p| p.into_iter().next())
            .and_then(|p| crate::brushes::load_image(&p).ok());
        let mask = found.unwrap_or_else(|| {
            let n = 300;
            let mut h = 7u32;
            let specks = (0..n * n)
                .map(|_| {
                    h ^= h << 13;
                    h ^= h >> 17;
                    h ^= h << 5;
                    [0, 0, 0, 0, 0, 0, 90, 255][(h >> 29) as usize]
                })
                .collect();
            massif_core::brush::BrushMask::new("Specks", n, n, specks)
        });
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.tools.select(Tool::Raise);
            app.tools.brush.mask = Some(std::sync::Arc::new(mask));
            app.tools.brush.rotation = 15.0;
        }
        h.run_steps(5);
        let at = {
            let m = h.state().maps.values().next().unwrap();
            m.screen_of(m.center[0], m.center[1])
        };
        for radius in [40.0f32, 150.0, 400.0] {
            h.state_mut().tools.brush.radius = radius;
            h.event(egui::Event::PointerMoved(at + vec2(radius * 0.01, 0.0)));
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("speckled-brush-{radius:.0}.png"))).unwrap();
        }
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

    /// A river from a source high on a steep slope of a real map
    /// (MASSIF_RIVER_WORLD), down to the sea: the 2D map and two 3D views
    /// of it, and its water level along its course, for the eye. Needs a
    /// GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn steep_river_on_a_real_world() {
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
        let source = (-102, -1607);
        let course = {
            let app = h.state();
            let sea = app.doc.world.meta.sea_level as f32;
            massif_core::river::descend(app.doc.dimension().ground(), source, sea, 60_000, 2_000_000, app.tools.river.lake_depth()).course
        };
        assert!(course.len() > 100, "{} points", course.len());
        let mid = course[course.len() / 2];
        {
            let app = h.state_mut();
            app.dialog = None;
            app.tools.select(Tool::River);
            app.tools.river = crate::tools::RiverSettings::default();
        }
        h.run_steps(5);
        for m in h.state_mut().maps.values_mut() {
            m.center = [mid.0 as f64, mid.1 as f64];
            m.zoom = 1.4;
        }
        h.run_steps(5);
        let p = h.state().maps.values().next().unwrap().screen_of(source.0 as f64 + 0.5, source.1 as f64 + 0.5);
        click_on(&mut h, p, egui::PointerButton::Primary);
        h.run_steps(10);
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
        // The water's level along the course, and its drops.
        {
            let app = h.state();
            let mut levels: Vec<i16> = Vec::new();
            for p in course.iter().step_by(2) {
                if let Some(probe) = crate::tools::probe(&app.doc, p.0.floor() as i32, p.1.floor() as i32)
                    && probe.fluid != massif_core::tile::NO_FLUID
                    && levels.last() != Some(&probe.fluid)
                {
                    levels.push(probe.fluid);
                }
            }
            let drops: Vec<i16> = levels.windows(2).map(|w| w[0] - w[1]).filter(|d| *d > 0).collect();
            eprintln!("levels along the course: {levels:?}");
            eprintln!("drops: {} of 1, {} of 2-3, {} of 4+", drops.iter().filter(|d| **d == 1).count(), drops.iter().filter(|d| (2..=3).contains(*d)).count(), drops.iter().filter(|d| **d >= 4).count());
        }
        h.render().unwrap().save(dir.join("steep-river-map.png")).unwrap();
        let ground = |app: &MassifApp, p: (f32, f32)| crate::tools::probe(&app.doc, p.0 as i32, p.1 as i32).map_or(80.0, |p| p.height);
        // From downstream, a little to the side, looking up the river.
        for (name, at, distance, pitch, turn) in [("steep-river-3d", course.len() / 4, 240.0, 0.75, 0.5f32), ("steep-river-3d-2", course.len() / 2, 200.0, 0.6, -0.6)] {
            let target = course[at];
            let ahead = course[(at + 40).min(course.len() - 1)];
            let yaw = (ahead.0 - target.0).atan2(ahead.1 - target.1) + turn;
            {
                let app = h.state_mut();
                let y = ground(app, target);
                app.layout.activate(3);
                app.view3d.camera.target = [target.0 as f64, y as f64, target.1 as f64];
                app.view3d.camera.distance = distance;
                app.view3d.camera.pitch = pitch;
                app.view3d.camera.yaw = yaw;
            }
            // Until the tiles around are built, the builder's included, for
            // a few frames in a row.
            let mut quiet = 0;
            for _ in 0..1500 {
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(15));
                quiet = if h.state().view3d.pending() == 0 { quiet + 1 } else { 0 };
                if quiet >= 40 {
                    break;
                }
            }
            let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join(format!("{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            h.render().unwrap().save(dir.join(format!("{name}-window.png"))).unwrap();
        }
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
    /// The node library: Tab opens it at the pointer, typing finds nodes,
    /// ↓ and Enter add the second one found; it stays in the window by an
    /// edge, and lists the node added last first. Renders it with
    /// MASSIF_SNAPSHOT_DIR, in both themes, at a normal and a small size.
    #[test]
    fn graph_library_searches_and_takes_keys() {
        use massif_core::graph::Template;
        crate::i18n::set_language(crate::i18n::Language::English);
        for theme in [ThemeChoice::Dark, ThemeChoice::Light] {
            let mut h = harness(theme);
            let name = if theme == ThemeChoice::Dark { "dark" } else { "light" };
            h.state_mut().layout.activate(7);
            h.run_steps(2);
            {
                let app = h.state_mut();
                app.graph.use_template(&mut app.doc, Template::Alpine);
            }
            h.run_steps(8);
            let nodes = |h: &egui_kittest::Harness<'_, MassifApp>| h.state().doc.world.meta.graph.clone().unwrap().nodes;
            let count = nodes(&h).len();
            let canvas = h.state().graph.canvas_rect();
            h.event(egui::Event::PointerMoved(canvas.center()));
            h.step();
            h.key_press(Key::Tab);
            h.run_steps(3);
            let (rect, pick) = h.state().graph.library_state().expect("Tab opens the library");
            assert!(rect.width() >= 420.0 && rect.height() >= 380.0, "a roomy picker: {rect:?}");
            assert!(pick.is_some(), "a row is chosen");
            if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
                h.render().unwrap().save(dir.join(format!("graph-library-{name}.png"))).unwrap();
            }
            h.event(egui::Event::Text("erosion".into()));
            h.run_steps(3);
            assert_eq!(h.state().graph.library_state().unwrap().1, Some("Erosion"), "the best match first");
            h.key_press(Key::ArrowDown);
            h.run_steps(2);
            assert_eq!(h.state().graph.library_state().unwrap().1, Some("Thermal erosion"), "↓ chooses the next");
            if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
                h.render().unwrap().save(dir.join(format!("graph-library-search-{name}.png"))).unwrap();
            }
            h.key_press(Key::ArrowUp);
            h.key_press(Key::ArrowDown);
            h.key_press(Key::Enter);
            h.run_steps(3);
            assert!(h.state().graph.library_state().is_none(), "Enter closes it");
            let all = nodes(&h);
            assert_eq!(all.len(), count + 1, "Enter adds a node");
            assert_eq!(all.last().unwrap().kind.label(), "Thermal erosion");
            // A small window, opened near its bottom: inside it, the node added last first.
            h.set_size(vec2(760.0, 520.0));
            h.run_steps(4);
            let window = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(760.0, 520.0));
            let canvas = h.state().graph.canvas_rect().intersect(window);
            // (The panel's right part is cut by the small window: by its bottom.)
            h.event(egui::Event::PointerMoved(canvas.left_bottom() + vec2(120.0, -50.0)));
            h.step();
            h.key_press(Key::Tab);
            h.run_steps(3);
            let (rect, pick) = h.state().graph.library_state().expect("open again");
            assert!(window.contains_rect(rect), "inside the window: {rect:?}");
            assert_eq!(pick, Some("Thermal erosion"), "the node added last comes first");
            if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
                h.render().unwrap().save(dir.join(format!("graph-library-small-{name}.png"))).unwrap();
            }
            h.key_press(Key::Escape);
            h.run_steps(2);
            assert!(h.state().graph.library_state().is_none(), "Esc closes it");
        }
    }

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

    /// The automatic biomes over a real map, as big as they come
    /// (MASSIF_AUTO_WORLD: a .massif), as a user gets them: the whole map at
    /// once, then the map in biomes. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn auto_biome_on_a_real_world() {
        let (Some(dir), Some(path)) = (
            std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from),
            std::env::var_os("MASSIF_AUTO_WORLD").map(PathBuf::from),
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
        {
            let app = h.state_mut();
            let id = app.active_map.or_else(|| app.maps.keys().next().copied()).expect("a map area");
            app.maps.get_mut(&id).unwrap().mode = crate::map_render::MapMode::Biome;
            app.tools.select(Tool::AutoBiome);
        }
        h.run_steps(10);
        // In the background: the window keeps drawing, the job reports how
        // far it is, and its result is one undoable step.
        let t = std::time::Instant::now();
        h.state_mut().tools.run_auto = Some(true);
        h.step();
        assert_eq!(h.state().jobs.len(), 1, "a job runs");
        let mut slowest = std::time::Duration::ZERO;
        let mut shown = false;
        while !h.state().jobs.is_empty() {
            assert!(t.elapsed().as_secs() < 300, "the job timed out");
            let f = std::time::Instant::now();
            h.step();
            slowest = slowest.max(f.elapsed());
            if !shown && h.state().jobs.first().and_then(|j| j.progress.as_ref()).is_some_and(|p| p.fraction() > 0.3) {
                h.render().unwrap().save(dir.join("auto-biome-progress.png")).unwrap();
                shown = true;
            }
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        let (biomes, changed) = h.state().tools.auto_done.expect("it reports");
        assert!(biomes && changed > 1_000_000, "{changed} columns");
        eprintln!("auto biome: {changed} columns in {:.1?}, slowest frame {:.0?}", t.elapsed(), slowest);
        let t = std::time::Instant::now();
        h.run_steps(30);
        eprintln!("30 frames after: {:.1?}", t.elapsed());
        assert_eq!(h.state().doc.history.undo_label(), Some("Auto biome"));
        h.render().unwrap().save(dir.join("auto-biome-real.png")).unwrap();
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

    /// The viewport's look, up close: the stone towers among themselves and
    /// from above them, the floating mountains, an alpine range, at the
    /// preview's finest (2048 samples over a map of 1024 blocks), as saved
    /// images. Needs a GPU and takes a while: runs with MASSIF_VIEWPORT_DIR.
    #[test]
    fn viewport_shots() {
        use massif_core::graph::Template;
        let Some(dir) = std::env::var_os("MASSIF_VIEWPORT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        let shots: [(Template, &str, f32, f32, f32, bool); 6] = [
            (Template::KarstTowers, "karst-among", 0.42, 0.1, 2.4, false),
            (Template::KarstTowers, "karst-above", 0.8, 0.42, 0.7, false),
            (Template::KarstTowers, "karst-cliffs", 0.2, 0.16, 1.2, false),
            (Template::FloatingMountains, "floating", 0.62, 0.3, 2.2, true),
            (Template::Alpine, "alpine", 0.9, 0.36, 0.6, false),
            (Template::Alpine, "alpine-low", 0.42, 0.14, 2.0, false),
        ];
        let mut current = None;
        for (t, name, distance, pitch, yaw, clouds) in shots {
            if current != Some(t) {
                let app = h.state_mut();
                app.graph.preview = 2048;
                app.graph.use_template(&mut app.doc, t);
                wait_for_graph(&mut h);
                current = Some(t);
            }
            let view = &mut h.state_mut().graph.view;
            view.camera.distance = distance;
            view.camera.pitch = pitch;
            view.camera.yaw = yaw;
            view.render.clouds = clouds;
            let (w, hgt, rgba) = view.render_image([1920, 1080]).expect("an image");
            image::save_buffer(dir.join(format!("viewport-{name}.png")), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            if name == "karst-among" {
                // As large as the viewport on a Retina screen.
                let [_, still, last] = view.time_qualities([2880, 1300]);
                // While it moves, the view is drawn at half the size.
                let [moving, _, _] = view.time_qualities([1440, 650]);
                eprintln!("{name} at 2880 × 1300: moving {moving:.1} ms (at half the size), still {still:.0} ms, final {last:.0} ms");
            }
        }
    }

    /// A preset chosen in Settings ▸ Graphics applies at once: Low draws
    /// the 3D view at one sample a pixel and two thirds of its size, keeps
    /// fewer of the map view's images on the card; High puts back four
    /// samples and the whole size. Needs a GPU (skipped without one).
    #[test]
    fn graphics_presets_apply_at_once() {
        use crate::graphics::{Graphics, Preset};
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(8);
        if h.state().view3d.samples().is_none() {
            eprintln!("no 3D view drawn (no GPU?): skipping");
            return;
        }
        assert_eq!(h.state().view3d.samples(), Some(4));
        let full = h.state().view3d.last_region();
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Graphics));
        h.run_steps(3);
        h.get_all_by_label("Low").next().expect("the Low preset").click();
        h.run_steps(3);
        assert_eq!(h.state().settings.graphics.map(|g| g.preset()), Some(Preset::Low));
        assert_eq!(crate::graphics::get(), Graphics::of(Preset::Low));
        h.state_mut().dialog = None;
        h.run_steps(8);
        assert_eq!(h.state().view3d.samples(), Some(1));
        let low = h.state().view3d.last_region();
        assert!(low[0] < full[0] * 3 / 4 && low[1] < full[1] * 3 / 4, "Low draws {low:?} of {full:?}");
        assert!(h.state().map_cache.max_chunks <= 2048);
        // One setting changed by hand: Custom.
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Graphics));
        h.run_steps(3);
        h.get_by_label("100%").click();
        h.run_steps(3);
        assert_eq!(h.state().settings.graphics.map(|g| g.preset()), Some(Preset::Custom));
        h.get_all_by_label("High").next().expect("the High preset").click();
        h.run_steps(3);
        assert_eq!(h.state().settings.graphics.map(|g| g.preset()), Some(Preset::High));
        h.state_mut().dialog = None;
        h.run_steps(8);
        assert_eq!(h.state().view3d.samples(), Some(4));
        assert_eq!(h.state().view3d.last_region(), full);
    }

    /// What Settings ▸ Graphics costs the Terrain viewport: a still frame
    /// of the stone towers among themselves, under clouds, at 2,880 × 1,300
    /// (the viewport on a Retina screen), each setting changed alone from
    /// High, then each preset. Needs a GPU: runs with MASSIF_PERF.
    #[test]
    fn graphics_settings_in_the_viewport() {
        use crate::graphics::{Graphics, Light, Preset};
        use massif_core::graph::Template;
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.activate(7);
        h.run_steps(2);
        let app = h.state_mut();
        app.graph.preview = 2048;
        app.graph.use_template(&mut app.doc, Template::KarstTowers);
        wait_for_graph(&mut h);
        let view = &mut h.state_mut().graph.view;
        view.camera.distance = 0.42;
        view.camera.pitch = 0.1;
        view.camera.yaw = 2.4;
        view.render.clouds = true;
        let high = Graphics::of(Preset::High);
        let mut runs: Vec<(String, Graphics)> = vec![("High".into(), high)];
        runs.push(("High, no MSAA".into(), Graphics { msaa: false, ..high }));
        runs.push(("High, scale 0.67".into(), Graphics { render_scale: 0.67, ..high }));
        runs.push(("High, light Medium".into(), Graphics { light: Light::Medium, ..high }));
        runs.push(("High, light Low".into(), Graphics { light: Light::Low, ..high }));
        runs.push(("High, no clouds".into(), Graphics { clouds: false, ..high }));
        runs.push(("Medium".into(), Graphics::of(Preset::Medium)));
        runs.push(("Low".into(), Graphics::of(Preset::Low)));
        for (name, g) in runs {
            crate::graphics::set(g);
            let ms = view.time_still([2880, 1300]);
            eprintln!("viewport, {name}: {ms:.1} ms a still frame");
        }
        crate::graphics::set(Graphics::default());
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

    /// A mod's blocks in the Blocks panel: in sections of their own named
    /// after the mod, beside the game's, a chip for the mod; painted, the
    /// terrain keeps the mod's block id for the export. Renders with
    /// MASSIF_SNAPSHOT_DIR, when a Minecraft jar is installed.
    #[test]
    fn mod_blocks_show_and_paint() {
        use egui_kittest::kittest::Queryable;
        if massif_assets::find_jars().is_empty() {
            eprintln!("no Minecraft installation: skipping");
            return;
        }
        let mut h = harness(ThemeChoice::Light);
        let ctx = h.ctx.clone();
        let jar = PathBuf::from("/mods/conquest-reforged.jar");
        h.state_mut().settings.mod_jars = vec![jar.clone()];
        h.state_mut().blocks.load(&ctx, None, Vec::new());
        h.state_mut().layout.active = 2;
        let started = std::time::Instant::now();
        while h.state().blocks.catalogue.is_none() {
            assert!(started.elapsed().as_secs() < 60, "the blocks did not load: {:?}", h.state().blocks.error);
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        // The game's catalogue with a mod's blocks, as a mod jar gives them.
        let mut cat = (*h.state().blocks.catalogue.clone().unwrap()).clone();
        let stone = cat.block("minecraft:stone_bricks").unwrap().clone();
        for (id, name, color) in [("conquest:old_red_bricks", "Old Red Bricks", [170, 60, 50]), ("conquest:mossy_flagstones", "Mossy Flagstones", [110, 120, 90])] {
            cat.blocks.push(massif_assets::Block { id: id.to_owned(), name: name.to_owned(), color, side_color: color, ..stone.clone() });
        }
        cat.blocks.sort_by(|a, b| a.id.cmp(&b.id));
        cat.mods = vec![massif_assets::ModInfo { name: "Conquest Reforged".to_owned(), path: jar, namespaces: vec!["conquest".to_owned()], blocks: 2, ..Default::default() }];
        h.state_mut().blocks.catalogue = Some(std::sync::Arc::new(cat.clone()));
        h.run_steps(5);
        h.get_by_label_contains("CONQUEST REFORGED");
        h.get_by_label_contains("Conquest Reforged");
        let bricks = cat.block("conquest:old_red_bricks").unwrap().clone();
        let app = h.state_mut();
        let terrain = massif_assets::block_terrain(&mut app.doc.world.registry, &bricks);
        let reg = &app.doc.world.registry;
        assert_eq!(reg.block(reg.terrain(terrain).strata[0].blocks[0].0).state, "conquest:old_red_bricks");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("blocks-mods.png")).unwrap();
        }
    }

    /// Conquest Reforged from end to end, when MASSIF_CONQUEST_DIR names
    /// its instance or a folder of instances (the one with Conquest's lists
    /// is read; the test reads the player's own game, so it runs only when
    /// asked) and the version of the game it plays is installed: offered in the
    /// Blocks panel and read in one click, its tabs shown; a map of hills
    /// painted with its ground by the automatic paint (its cliffs in beds
    /// of three limestones), a meadow of a mix and a forest floor of a
    /// pattern, a plaza paved with a hundred of its blocks, its plants
    /// grown over its own ground, temples of its shaped blocks around the
    /// plaza; the export offered for 1.20.1 into its instance's saves,
    /// without a warning. Renders the panel, the map and the 3D views
    /// (MASSIF_SNAPSHOT_DIR); keeps the map and its export in
    /// MASSIF_CONQUEST_WORLD=<dir>, to play.
    #[test]
    fn conquest_reforged_from_end_to_end() {
        use egui_kittest::kittest::Queryable;
        use massif_core::registry::{Mix, Stratum, TerrainDef, TerrainId};
        use massif_core::Brush;
        let Some(dir) = std::env::var_os("MASSIF_CONQUEST_DIR").map(PathBuf::from) else {
            eprintln!("MASSIF_CONQUEST_DIR=<a Conquest Reforged instance, or a folder of instances> runs Conquest from end to end: skipping");
            return;
        };
        let Some(instance) = massif_assets::instance::with_mod_in(&dir, "conquest").into_iter().next() else {
            eprintln!("no Conquest Reforged instance in {}: skipping", dir.display());
            return;
        };
        if instance.lists.is_empty() {
            eprintln!("{} has no lists of Conquest's blocks: skipping", instance.name);
            return;
        }
        let jars = massif_assets::find_jars();
        let Some(game) = jars.iter().find(|j| Some(&j.version) == instance.minecraft.as_ref()).cloned() else {
            eprintln!("Minecraft {:?} is not installed here: skipping", instance.minecraft);
            return;
        };
        let shots = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let keep = std::env::var_os("MASSIF_CONQUEST_WORLD").map(PathBuf::from);
        let world = generate(&GenParams { tiles_x: 3, tiles_z: 3, landform: Landform::Mountains, relief: 90.0, seed: 2027, name: "Massif Conquest Test".into(), ..GenParams::default() });
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1600.0, 1000.0))
            .with_pixels_per_point(std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0))
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                let mut app = MassifApp::for_tests(world.clone());
                app.settings.theme = ThemeChoice::Dark;
                app
            });
        let ctx = h.ctx.clone();
        let wait_for = |h: &mut egui_kittest::Harness<'_, MassifApp>, what: &str, done: &dyn Fn(&MassifApp) -> bool| {
            let started = std::time::Instant::now();
            while !done(h.state()) {
                assert!(started.elapsed().as_secs() < 120, "{what}: {:?}", h.state().blocks.error);
                h.step();
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        };
        let has_conquest = |app: &MassifApp| app.blocks.catalogue.as_ref().is_some_and(|c| c.mods.iter().any(|m| m.kind == massif_assets::SourceKind::Mod && m.namespaces.iter().any(|n| n == "conquest")));

        // The game alone: Conquest Reforged, installed here, is offered.
        h.state_mut().layout.active = 2; // Painting: terrain, blocks, biomes
        h.state_mut().settings.minecraft_jar = None;
        h.state_mut().settings.mod_jars.clear();
        h.state_mut().blocks.load(&ctx, None, Vec::new());
        wait_for(&mut h, "the game's blocks", &|app| app.blocks.catalogue.is_some() && !app.blocks.is_loading());
        assert_eq!(h.state().blocks.conquest.first().map(|i| i.dir.clone()), Some(instance.dir.clone()), "the instance is found, the one with Conquest's lists first");
        h.run_steps(4);
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-offered.png")).unwrap();
        }
        // One click: its jar, its list and its packs, over 1.20.1.
        let before = crate::memory_tests::resident();
        let started = std::time::Instant::now();
        // Its row's button: the first, for the instance read best.
        h.query_all_by_label("Use").next().expect("a Use button").click();
        h.run_steps(2);
        wait_for(&mut h, "Conquest Reforged's blocks", &|app| has_conquest(app) && !app.blocks.is_loading());
        let took = started.elapsed();
        let after = crate::memory_tests::resident();
        let cat = h.state().blocks.catalogue.clone().unwrap();
        eprintln!("Conquest Reforged read in {took:?}: {} blocks, {} textures; memory {:.0} MB → {:.0} MB (most {:.0} MB)", cat.blocks.len(), cat.textures.len(), before.0, after.0, after.1);
        assert_eq!(h.state().settings.minecraft_jar.as_ref(), Some(&game.path), "over the version it plays");
        assert_eq!(h.state().settings.mod_jars, instance.files(), "remembered");
        assert_eq!(cat.version, game.version);
        assert!(h.state().blocks.conquest.is_empty(), "no longer offered");
        let conquest = cat.mods.iter().find(|m| m.kind == massif_assets::SourceKind::Mod && m.namespaces.iter().any(|n| n == "conquest")).unwrap().clone();
        assert_eq!(conquest.name, "Conquest Reforged");
        // Its tabs, in its order, after the game's blocks; made again as
        // fast as one types.
        h.state_mut().blocks.all = true;
        let started = std::time::Instant::now();
        let every = h.state_mut().blocks.sections("");
        eprintln!("{} sections of every shape made in {:?}", every.len(), started.elapsed());
        assert!(started.elapsed().as_millis() < 250);
        h.state_mut().blocks.all = false;
        h.state_mut().blocks.search = "conquest".into();
        let sections = h.state_mut().blocks.sections("conquest");
        let titles: Vec<&str> = sections.iter().map(|s| s.title.as_str()).collect();
        assert!(titles.first().is_some_and(|t| t.starts_with("CONQUEST REFORGED  ·  COBBLE AND BRICK")), "{titles:?}");
        assert!(titles.len() >= 30, "{} tabs: {titles:?}", titles.len());
        h.get_by_label_contains("CONQUEST REFORGED  ·  COBBLE AND BRICK");
        // The Blocks area made as large as the window, to see the tabs.
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let blocks_area = areas.iter().map(|a| a.0).find(|&id| h.state().layout.current().root.editor_of(id) == Some(EditorKind::Blocks)).unwrap();
        h.state_mut().layout.toggle_maximize(blocks_area);
        h.run_steps(6);
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-blocks.png")).unwrap();
        }
        // Every shape of a block follows it.
        h.state_mut().blocks.search = "travertine brick".into();
        h.state_mut().blocks.all = true;
        let shapes = h.state_mut().blocks.sections("travertine brick");
        let shown: Vec<&str> = shapes.iter().flat_map(|s| s.blocks.iter().map(|&i| cat.blocks[i as usize].id.as_str())).collect();
        assert!(shown.contains(&"conquest:travertine_brick_small_arch") && shown.contains(&"conquest:travertine_brick_capital"), "{shown:?}");
        h.run_steps(6);
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-shapes.png")).unwrap();
        }
        h.state_mut().blocks.search.clear();
        h.state_mut().blocks.all = false;
        h.state_mut().layout.toggle_maximize(blocks_area);

        // The map painted with Conquest Reforged's blocks.
        let block = |id: &str| cat.block(id).unwrap_or_else(|| panic!("{id}")).clone();
        let (centre, plaza_y) = {
            let app = h.state_mut();
            let reg = &mut app.doc.world.registry;
            let terrain = |reg: &mut massif_core::registry::Registry, id: &str| {
                let b = block(id);
                assert!(b.full_cube, "{id} is a whole block");
                let t = massif_assets::block_terrain(reg, &b);
                reg.terrain(t).name.clone()
            };
            // The automatic paint with Conquest's ground: its grass, forest
            // floor, sand, silt and gravel, and cliffs in beds of three
            // limestones.
            let grass = terrain(reg, "conquest:clover_covered_grass");
            let forest = terrain(reg, "conquest:taiga_forest_floor");
            let sand = terrain(reg, "conquest:tan_sand");
            let silt = terrain(reg, "conquest:dark_silt");
            let gravel = terrain(reg, "conquest:brown_gravel");
            let limestones: Vec<massif_core::registry::BlockId> = ["conquest:gray_limestone", "conquest:tan_limestone", "conquest:dark_limestone", "conquest:tan_limestone"]
                .iter()
                .map(|id| reg.intern_block(id, block(id).color))
                .collect();
            let cliffs = "Limestone Beds".to_owned();
            reg.terrains.push(TerrainDef {
                name: cliffs.clone(),
                group: massif_assets::BLOCK_GROUP.to_owned(),
                color: block("conquest:gray_limestone").color,
                strata: vec![Stratum { blocks: limestones.iter().zip([3, 2, 1, 2]).map(|(&b, w)| (b, w)).collect(), depth: (6, 8), mix: Mix::Beds }],
                submerged_top: None,
                biome_hint: None,
            });
            let mut rules = app.tools.auto.clone();
            rules.paint.steep = 24.0;
            for b in &mut rules.paint.biomes {
                let swap = |name: &str| match name {
                    "Grass" => grass.clone(),
                    "Podzol" => forest.clone(),
                    "Sand" | "Red Sand" => sand.clone(),
                    "Dirt" | "Mud" | "Clay" => silt.clone(),
                    "Gravel" => gravel.clone(),
                    "Stone" | "Calcite" | "Andesite" | "Granite" => cliffs.clone(),
                    other => other.to_owned(),
                };
                b.ground = swap(&b.ground);
                b.cliffs = swap(&b.cliffs);
                b.underwater = swap(&b.underwater);
            }
            app.doc.auto_biome(&rules);
            let painted = app.doc.auto_paint(&rules);
            assert!(painted > 50_000, "{painted} columns painted");
            let reg = &mut app.doc.world.registry;
            // A meadow of a mix: grass with patches of stones.
            let meadow_blocks: Vec<(massif_core::registry::BlockId, u16)> = [("conquest:clover_covered_grass", 6), ("conquest:small_stones", 2), ("conquest:brown_moss", 1)].iter().map(|&(id, w)| (reg.intern_block(id, block(id).color), w)).collect();
            reg.terrains.push(TerrainDef { name: "Stony Meadow".into(), group: massif_assets::BLOCK_GROUP.into(), color: block("conquest:clover_covered_grass").color, strata: vec![Stratum { blocks: meadow_blocks, depth: (3, 4), mix: Mix::Patches { size: 3.0 } }], submerged_top: None, biome_hint: None });
            let meadow = TerrainId((reg.terrains.len() - 1) as u16);
            let floors: Vec<TerrainId> = ["conquest:taiga_forest_floor", "conquest:autumnal_forest_floor", "conquest:mossy_forest_floor_with_roots"].iter().map(|id| massif_assets::block_terrain(reg, &block(id))).collect();
            // The plaza: a hundred of its whole blocks from its tabs.
            let mut paving: Vec<String> = Vec::new();
            for tab in ["Cobble And Brick", "Mosaics Tiles And Floors", "Plaster Stucco And Paint", "Roofing", "Planks And Beams", "Stone", "Sand And Gravel", "Advanced Masonry", "Metal", "Cloth And Fibers"] {
                let Some(g) = conquest.groups.iter().find(|g| g.name.eq_ignore_ascii_case(tab)) else { continue };
                let cubes: Vec<&String> = g.blocks.iter().filter(|id| id.starts_with("conquest:") && cat.block(id).is_some_and(|b| b.full_cube)).collect();
                let step = (cubes.len() / 12).max(1);
                paving.extend(cubes.into_iter().step_by(step).take(12).cloned());
            }
            paving.truncate(120);
            assert!(paving.len() >= 100, "{} paving blocks", paving.len());
            let paving: Vec<TerrainId> = paving.iter().map(|id| massif_assets::block_terrain(reg, &block(id))).collect();
            // Its plants: covers of grasses and flowers, two of them tall.
            let plants: Vec<u16> = ["conquest:timothy_grass", "conquest:kentucky_bluegrass", "conquest:meadow_fescue", "conquest:common_meadow_grass", "conquest:poppy", "conquest:oxeye_daisy", "conquest:buttercups", "conquest:cornflower", "conquest:tall_lilac", "conquest:peony"]
                .iter()
                .filter_map(|id| cat.block(id).cloned())
                .map(|b| crate::panels::block_cover(reg, &b))
                .collect();
            assert!(plants.len() >= 8);
            let tall: Vec<&str> = reg.covers.iter().flat_map(|c| c.plants.iter().map(|p| p.0.as_str())).filter(|s| s.ends_with("[half=lower]")).collect();
            assert!(!tall.is_empty(), "a tall plant of the mod, kept as its lower half");
            // Temples of its shaped blocks, three stones.
            let temple = |m: &str| {
                let (w, hgt, l) = (7usize, 7usize, 7usize);
                let mut palette = vec!["minecraft:air".to_owned()];
                let mut add = |s: String| {
                    palette.push(s);
                    (palette.len() - 1) as u16
                };
                let floor = add(format!("conquest:{m}"));
                let pillar = add(format!("conquest:{m}_pillar[layer=3,waterlogged=false]"));
                let capital = add(format!("conquest:{m}_capital[facing=flat,type=bottom,waterlogged=false]"));
                let arch_x = add(format!("conquest:{m}_small_arch[direction=eastwest,waterlogged=false]"));
                let arch_z = add(format!("conquest:{m}_small_arch[direction=northsouth,waterlogged=false]"));
                let roof = add(format!("conquest:{m}_slab[layers=4,type=bottom,waterlogged=false]"));
                let stairs = add(format!("conquest:{m}_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"));
                // Not every stone has a sphere.
                let sphere = cat.block(&format!("conquest:{m}_sphere")).is_some().then(|| add(format!("conquest:{m}_sphere[type=large,waterlogged=false]")));
                let mut blocks = vec![0u16; w * hgt * l];
                let at = |x: usize, y: usize, z: usize| x + z * w + y * w * l;
                for x in 0..w {
                    for z in 0..l {
                        blocks[at(x, 0, z)] = floor;
                        blocks[at(x, 6, z)] = roof;
                    }
                }
                for (x, z) in [(0, 0), (6, 0), (0, 6), (6, 6), (3, 0), (3, 6), (0, 3), (6, 3)] {
                    for y in 1..4 {
                        blocks[at(x, y, z)] = pillar;
                    }
                    blocks[at(x, 4, z)] = capital;
                }
                for x in 1..6 {
                    for z in [0, 6] {
                        if blocks[at(x, 4, z)] == 0 {
                            blocks[at(x, 5, z)] = arch_x;
                        }
                    }
                }
                for z in 1..6 {
                    for x in [0, 6] {
                        if blocks[at(x, 4, z)] == 0 {
                            blocks[at(x, 5, z)] = arch_z;
                        }
                    }
                }
                for (x, z) in [(0, 0), (6, 0), (0, 6), (6, 6)] {
                    blocks[at(x, 5, z)] = capital;
                }
                blocks[at(3, 0, 0)] = stairs;
                if let Some(sphere) = sphere {
                    blocks[at(3, 1, 3)] = sphere;
                }
                for state in &palette[1..] {
                    assert_eq!(cat.check_state(state), Ok(()), "{state}");
                }
                massif_core::objects::ObjectDef::new(&format!("{} temple", block(&format!("conquest:{m}")).name), [w as u16, hgt as u16, l as u16], palette, blocks)
            };
            let first = reg.objects.len() as u32;
            for m in ["travertine_brick", "carved_limestone", "large_limestone_block"] {
                reg.objects.push(temple(m));
            }
            let mut layer = massif_core::objects::ObjectLayerDef::new("Conquest temples", &[first, first + 1, first + 2], reg);
            layer.spacing = 30;
            layer.density = 1.0;
            layer.soil_only = false;
            let temples = reg.add_object_layer(layer);

            let doc = &mut app.doc;
            let (lo, hi) = doc.dimension().bounds().unwrap();
            let t = massif_core::coords::TILE_SIZE;
            let centre = ((lo.x + hi.x + 1) * t / 2, (lo.z + hi.z + 1) * t / 2);
            let ground = crate::tools::probe(doc, centre.0, centre.1).map_or(doc.world.meta.sea_level + 8, |p| p.height.round() as i32).max(doc.world.meta.sea_level + 4);
            let square = |radius: f32| Brush { shape: massif_core::BrushShape::Square, radius, profile: massif_core::Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
            let disc = |radius: f32| Brush { shape: massif_core::BrushShape::Circle, radius, profile: massif_core::Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
            doc.history.begin("Conquest", 0);
            {
                let mut e = doc.history.edit(&mut doc.world, 0);
                let (cx, cz) = (centre.0 as f32, centre.1 as f32);
                // The plaza, level, 12 by 10 squares of 5 blocks.
                massif_core::ops::set_height(&mut e, &square(40.0), cx, cz, ground as f32, massif_core::ops::Surface::Top);
                for (k, t) in paving.iter().enumerate() {
                    let (x, z) = (cx - 33.0 + (k % 12) as f32 * 6.0, cz - 27.0 + (k / 12) as f32 * 6.0);
                    massif_core::ops::paint_terrain(&mut e, &square(2.5), x, z, *t, 100 + k as u64);
                }
                // Temples around it.
                massif_core::ops::paint_objects(&mut e, &disc(12.0), cx - 70.0, cz, temples, 1);
                massif_core::ops::paint_objects(&mut e, &disc(12.0), cx + 70.0, cz, temples, 2);
                massif_core::ops::paint_objects(&mut e, &disc(12.0), cx, cz + 64.0, temples, 3);
                // A stony meadow to the north, a forest floor of three to the
                // south-west, the plants over the grass all round.
                massif_core::ops::paint_terrain(&mut e, &disc(36.0), cx, cz - 110.0, meadow, 7);
                let sampler = massif_core::pattern::Pattern { scale: 10.0, ..Default::default() }.sampler(&floors.iter().map(|&f| (f, 1.0)).collect::<Vec<_>>());
                massif_core::ops::paint_pattern(&mut e, &disc(40.0), cx - 110.0, cz + 110.0, 8, &sampler);
                for (k, c) in plants.iter().enumerate() {
                    let a = k as f32 / plants.len() as f32 * std::f32::consts::TAU;
                    massif_core::ops::paint_cover(&mut e, &disc(30.0), cx + a.cos() * 120.0, cz + a.sin() * 120.0, *c, 200 + k as u64);
                }
            }
            doc.commit();
            app.map_cache.invalidate_all();
            (centre, ground)
        };

        // The map, and the land seen in 3D.
        {
            let app = h.state_mut();
            app.layout.active = 0;
            app.dialog = None;
        }
        h.run_steps(4);
        for view in h.state_mut().maps.values_mut() {
            view.center = [centre.0 as f64, centre.1 as f64];
            view.zoom = 2.0;
        }
        h.state_mut().view3d.camera = crate::view3d::Camera { target: [centre.0 as f64, plaza_y as f64, centre.1 as f64], yaw: 0.6, pitch: 0.5, distance: 190.0 };
        let mut frames = 0;
        while frames < 20 || h.state().view3d.pending() > 0 {
            h.step();
            frames += 1;
            if frames > 3000 {
                break;
            }
        }
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-map.png")).unwrap();
            // The 3D view as large as the window.
            let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
            let view = areas.iter().map(|a| a.0).find(|&id| h.state().layout.current().root.editor_of(id) == Some(EditorKind::View3d)).unwrap();
            h.state_mut().layout.toggle_maximize(view);
            let mut frames = 0;
            while frames < 20 || h.state().view3d.pending() > 0 {
                h.step();
                frames += 1;
                if frames > 3000 {
                    break;
                }
            }
            let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join("conquest-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
            h.state_mut().layout.toggle_maximize(view);
        }
        // The player's view of the plaza, with the game's textures.
        h.state_mut().layout.active = 4; // 3D Sculpt
        h.run_steps(4);
        // The player standing at the north edge of the plaza, looking
        // across it to the temples south of it (the camera set before the
        // player's view starts from it).
        h.state_mut().sculpt3d.camera = crate::view3d::Camera { target: [centre.0 as f64 - 9.5, plaza_y as f64 + 1.0, centre.1 as f64 - 25.5], yaw: (-21.0f32).atan2(-90.0), pitch: 0.12, distance: 20.0 };
        h.state_mut().tools.sculpt.player = true;
        h.run_steps(3);
        let mut frames = 0;
        while frames < 20 || h.state().sculpt3d.pending() > 0 {
            h.step();
            frames += 1;
            if frames > 3000 {
                break;
            }
        }
        if let Some(dir) = &shots {
            let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
            image::save_buffer(dir.join("conquest-player.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        }
        h.state_mut().tools.sculpt.player = false;

        // The export: for 1.20.1, into the instance's saves, no warning.
        h.state_mut().layout.active = 0;
        h.state_mut().open_export_dialog();
        h.run_steps(4);
        let Some(Dialog::Export(form)) = &h.state().dialog else { panic!("the export dialog") };
        assert_eq!(form.target.name, "1.20.1", "the version Conquest Reforged plays");
        assert_eq!(form.parent, instance.dir.join("saves"), "its instance's saves");
        assert_eq!(form.mods.len(), 1, "{:?}", form.mods);
        let m = &form.mods[0];
        assert!(m.loaded && m.minecraft.as_deref() == Some("1.20.1") && m.unknown.is_empty(), "{m:?}");
        assert!(m.blocks >= 150, "{} blocks", m.blocks);
        let target = form.target.clone();
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-export.png")).unwrap();
        }
        // Another version: warned in red, with the way back (the dialog
        // grows with the warning and settles in a few frames).
        if let Some(Dialog::Export(form)) = &mut h.state_mut().dialog {
            form.target = massif_anvil::Target::release("1.21.11").unwrap();
        }
        h.run_steps(12);
        h.get_by_label_contains("is made for Minecraft 1.20.1: in Java 1.21.11");
        if let Some(dir) = &shots {
            h.render().unwrap().save(dir.join("conquest-export-warning.png")).unwrap();
        }
        h.get_by_label("Export for 1.20.1").click();
        h.run_steps(2);
        let Some(Dialog::Export(form)) = &h.state().dialog else { panic!("the export dialog") };
        assert_eq!(form.target.name, "1.20.1", "put back in one click");
        h.state_mut().dialog = None;
        let out = keep.clone().unwrap_or_else(|| std::env::temp_dir().join(format!("massif-conquest-{}", std::process::id())));
        std::fs::create_dir_all(&out).unwrap();
        let world_dir = out.join("Massif Conquest Test");
        let _ = std::fs::remove_dir_all(&world_dir);
        let options = ExportOptions {
            level_name: Some("Massif Conquest Test".into()),
            game_mode: GameMode::Creative,
            spawn: Some([centre.0, plaza_y + 1, centre.1 + 34]),
            target,
            ..ExportOptions::default()
        };
        let snapshot = h.state_mut().doc.world.snapshot();
        let job = Job::export(&ctx, snapshot, world_dir.clone(), options);
        h.state_mut().jobs.push(job);
        wait_for(&mut h, "the export", &|app| app.jobs.is_empty());
        assert!(world_dir.join("level.dat").is_file() && world_dir.join("region").is_dir(), "exported");
        massif_format::save(&h.state().doc.world, &out.join("Massif Conquest Test.massif")).unwrap();
        eprintln!("exported to {}", world_dir.display());
        if keep.is_none() {
            let _ = std::fs::remove_dir_all(&out);
        }
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
            h.state_mut().blocks.load(&ctx, None, Vec::new());
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
            // Every picture of the backdrop, dithered.
            for k in 0..home::Home::backdrops() {
                let name = h.state_mut().home.as_mut().unwrap().choose_backdrop(k);
                h.run_steps(3);
                h.render().unwrap().save(dir.join(format!("home-backdrop-{name}.png"))).unwrap();
            }
            // And in the light theme: each scene at dawn.
            h.state_mut().settings.theme = ThemeChoice::Light;
            h.run_steps(3);
            for k in 0..home::Home::backdrops() {
                let name = h.state_mut().home.as_mut().unwrap().choose_backdrop(k);
                h.run_steps(3);
                h.render().unwrap().save(dir.join(format!("home-backdrop-light-{name}.png"))).unwrap();
            }
            h.state_mut().settings.theme = ThemeChoice::Dark;
            h.run_steps(3);
        }
        // The highest Minecraft allows, and back.
        h.get_by_label("2032").click();
        h.run_steps(2);
        let p = &h.state().home.as_ref().unwrap().new_map.params;
        assert_eq!((p.min_y, p.max_y), (-64, 2032));
        h.get_by_label("320").click();
        h.run_steps(2);
        let p = &h.state().home.as_ref().unwrap().new_map.params;
        assert_eq!((p.min_y, p.max_y), (-64, 320));
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

    /// Shortcuts as the keymap has them: a key rebound does its new action
    /// and no longer its old one, the menus show the new keys, keys typed
    /// in a text field stay there, and Settings ▸ Keyboard records keys,
    /// asks before taking another action's and renders in light and dark.
    #[test]
    fn keyboard_shortcuts_rebind() {
        use crate::keymap::{Action, Chord, Mods};
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(5);
        // The defaults: 3 is 30 %.
        h.key_press(Key::Num3);
        h.run_steps(2);
        assert_eq!(h.state().tools.brush.strength, 0.3);
        // 30 % on K: K does it, 3 no longer.
        h.state_mut().settings.keys.set(Action::Intensity(3), &[Chord::key(Key::K)]);
        h.state_mut().tools.brush.strength = 1.0;
        h.run_steps(1);
        h.key_press(Key::Num3);
        h.run_steps(2);
        assert_eq!(h.state().tools.brush.strength, 1.0);
        h.key_press(Key::K);
        h.run_steps(2);
        assert_eq!(h.state().tools.brush.strength, 0.3);
        // Save on ⌘K: the menus show it.
        h.state_mut().settings.keys.set(Action::Save, &[Chord::with(Mods::COMMAND, Key::K)]);
        h.run_steps(1);
        let menus = h.state().menus();
        let save = menus.iter().flat_map(|m| &m.entries).find_map(|e| match e {
            crate::menu::Entry::Item(i) if i.command == Command::Save => Some(i.shortcut),
            _ => None,
        });
        assert_eq!(save, Some(Some(egui::KeyboardShortcut::new(Modifiers::COMMAND, Key::K))));
        // A text field typed in keeps its keys: 5 is typed, not 50 %.
        h.state_mut().settings.keys = crate::keymap::Saved::default();
        h.state_mut().tools.brush.strength = 1.0;
        h.run_steps(2);
        let field = h.query_all_by_role(egui::accesskit::Role::TextInput).next().map(|n| n.focus()).is_some();
        assert!(field, "a text field in the window");
        {
            h.run_steps(2);
            assert!(h.ctx.egui_wants_keyboard_input(), "the field has the keys");
            h.key_press(Key::Num5);
            let tool = h.state().tools.tool;
            h.key_press_modifiers(Modifiers::ALT, Key::Y);
            h.run_steps(2);
            assert_eq!(h.state().tools.brush.strength, 1.0, "5 typed in a field sets no intensity");
            assert_eq!(h.state().tools.tool, tool, "⌥Y typed in a field picks no tool");
            h.key_press(Key::Escape);
            h.ctx.memory_mut(|m| m.stop_text_input());
            h.run_steps(2);
        }
        // Settings ▸ Keyboard: Raise / Lower's key clicked, ⌥R pressed, which
        // Paint terrain has: asked, then replaced.
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Keyboard));
        h.run_steps(4);
        let raise = if crate::keymap::MAC { "⌥H" } else { "Alt+H" };
        h.get_by_label(raise).click();
        h.run_steps(2);
        assert!(crate::keymap::recording(), "listening for keys");
        h.key_press_modifiers(Modifiers::ALT, Key::R);
        h.run_steps(3);
        assert!(matches!(h.state().dialog, Some(Dialog::Preferences(PrefsSection::Keyboard))), "still open");
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("dialog-settings-keyboard-clash.png")).unwrap();
        }
        h.get_by_label("Replace").click();
        h.run_steps(3);
        let map = h.state().settings.keys.keymap();
        assert_eq!(map.keys(Action::Tool(Tool::Raise)), [Chord::with(Mods::ALT, Key::R)]);
        assert_eq!(map.keys(Action::Tool(Tool::PaintTerrain)), [Chord::with(Mods::ALT, Key::P)]);
        // Escape while listening is a key and leaves the dialog open; it
        // cancels on the map, so that is asked first: Cancel keeps ⌥P.
        h.get_by_label(if crate::keymap::MAC { "⌥P" } else { "Alt+P" }).click();
        h.run_steps(2);
        h.key_press(Key::Escape);
        h.run_steps(3);
        assert!(h.state().dialog.is_some(), "Escape recorded, not closing");
        h.get_by_label("Cancel").click();
        h.run_steps(2);
        assert_eq!(h.state().settings.keys.keymap().keys(Action::Tool(Tool::PaintTerrain)), [Chord::with(Mods::ALT, Key::P)]);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("dialog-settings-keyboard.png")).unwrap();
            h.state_mut().settings.theme = ThemeChoice::Dark;
            h.run_steps(5);
            h.render().unwrap().save(dir.join("dialog-settings-keyboard-dark.png")).unwrap();
            for (language, name) in [(crate::i18n::Language::French, "fr"), (crate::i18n::Language::German, "de")] {
                crate::i18n::set_language(language);
                h.run_steps(4);
                h.render().unwrap().save(dir.join(format!("dialog-settings-keyboard-{name}.png"))).unwrap();
            }
            crate::i18n::set_language(crate::i18n::Language::English);
        }
    }

    /// Settings ▸ Accessibility: high contrast and reduced motion as the
    /// settings say (the tests never follow the Mac running them), the size
    /// of the interface too, and ⌘+ kept; with motion reduced, the map goes
    /// straight where it is sent. With MASSIF_SNAPSHOT_DIR, the section and
    /// the home screen in high contrast are saved.
    #[test]
    fn accessibility_settings_apply() {
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Light);
        h.run_steps(4);
        assert!(!theme::high_contrast(&h.ctx) && !theme::reduce_motion(&h.ctx));
        // Reduced motion: a zoom asked for is there at once.
        h.state_mut().settings.reduce_motion = Follow::On;
        h.run_steps(2);
        assert!(theme::reduce_motion(&h.ctx), "motion reduced");
        let before = h.state().maps.values().next().expect("a map view").zoom;
        h.state_mut().maps.values_mut().for_each(|m| m.zoom_smoothly(2.0));
        h.run_steps(1);
        let after = h.state().maps.values().next().unwrap().zoom;
        assert!((after / before - 2.0).abs() < 1e-3, "zoomed at once: {before} to {after}");
        // High contrast.
        h.state_mut().settings.high_contrast = Follow::On;
        h.run_steps(2);
        assert!(theme::high_contrast(&h.ctx), "high contrast");
        // The size of the interface; ⌘+ changes it, and the change is kept.
        h.state_mut().settings.ui_scale = 1.25;
        h.run_steps(2);
        assert!((h.ctx.zoom_factor() - 1.25).abs() < 1e-3, "zoom {}", h.ctx.zoom_factor());
        h.event(egui::Event::Key {
            key: egui::Key::Plus,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: Modifiers::COMMAND,
        });
        h.run_steps(3);
        assert!(h.state().settings.ui_scale > 1.3, "⌘+ kept: {}", h.state().settings.ui_scale);
        h.state_mut().settings.ui_scale = 1.0;
        h.run_steps(2);
        assert!((h.ctx.zoom_factor() - 1.0).abs() < 1e-3);
        if let Some(dir) = &dir {
            h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Accessibility));
            h.run_steps(4);
            h.render().unwrap().save(dir.join("dialog-settings-accessibility-high.png")).unwrap();
            h.state_mut().dialog = None;
            h.state_mut().open_home(Page::NewMap);
            h.run_steps(5);
            h.render().unwrap().save(dir.join("home-light-high.png")).unwrap();
            h.state_mut().settings.theme = ThemeChoice::Dark;
            h.run_steps(5);
            h.render().unwrap().save(dir.join("home-dark-high.png")).unwrap();
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
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Appearance));
        h.run_steps(5);
        h.render().unwrap().save(dir.join("dialog-settings.png")).unwrap();
        // Further down, the crosshair, its preview on light ground and dark.
        h.event(egui::Event::PointerMoved(pos2(880.0, 560.0)));
        for _ in 0..6 {
            h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -120.0), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
            h.run_steps(2);
        }
        h.run_steps(20);
        h.render().unwrap().save(dir.join("dialog-settings-crosshair.png")).unwrap();
        h.state_mut().settings.reticle.shape = crate::reticle::Shape::Circle;
        h.state_mut().settings.reticle.size = 24.0;
        h.run_steps(3);
        h.render().unwrap().save(dir.join("dialog-settings-crosshair-circle.png")).unwrap();
        h.state_mut().settings.reticle = crate::reticle::Reticle::default();
        for (section, name) in [
            (PrefsSection::Accessibility, "accessibility"),
            (PrefsSection::Typeface, "typeface"),
            (PrefsSection::Sounds, "sounds"),
            (PrefsSection::Notifications, "notifications"),
            (PrefsSection::Memory, "memory"),
            (PrefsSection::Autosave, "autosave"),
            (PrefsSection::Graphics, "graphics"),
        ] {
            h.state_mut().dialog = Some(Dialog::Preferences(section));
            h.run_steps(4);
            h.render().unwrap().save(dir.join(format!("dialog-settings-{name}.png"))).unwrap();
        }
        // The rest of the graphics page, scrolled.
        h.state_mut().dialog = Some(Dialog::Preferences(PrefsSection::Graphics));
        h.run_steps(3);
        h.event(egui::Event::PointerMoved(pos2(880.0, 560.0)));
        for _ in 0..6 {
            h.event(egui::Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta: vec2(0.0, -120.0), modifiers: egui::Modifiers::NONE, phase: egui::TouchPhase::Move, source: egui::MouseWheelSource::Wheel });
            h.run_steps(2);
        }
        h.run_steps(20);
        h.render().unwrap().save(dir.join("dialog-settings-graphics-more.png")).unwrap();
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

    /// Save points through the app: made from File ▸ Save Point…, listed
    /// in the Versions panel, compared on the map, restored as one step to
    /// undo, saved in the file, opened as a copy and read-only. With
    /// MASSIF_SNAPSHOT_DIR, the panel and the dialog are saved.
    #[test]
    fn save_points_in_the_app() {
        use crate::versions::Action;
        use egui_kittest::kittest::Queryable;
        let dir = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from);
        let mut h = harness(ThemeChoice::Dark);
        let ctx = h.ctx.clone();
        h.run_steps(5);
        // The Versions panel where the tools' settings were.
        let mut areas = Vec::new();
        h.state().layout.current().root.areas(&mut areas);
        let area = areas.iter().find(|a| a.1 != EditorKind::Map).expect("an area beside the map").0;
        h.state_mut().layout.current_mut().root.set_editor(area, EditorKind::Versions);
        h.run_steps(3);
        let wait = |h: &mut egui_kittest::Harness<'static, MassifApp>| {
            while !h.state().jobs.is_empty() {
                std::thread::sleep(std::time::Duration::from_millis(5));
                h.run_steps(1);
            }
            h.run_steps(2);
        };
        // A point made through its dialog.
        h.state_mut().command(&ctx, crate::menu::Command::SavePoint);
        h.run_steps(3);
        if let Some(Dialog::PointName(None, name)) = &mut h.state_mut().dialog {
            *name = "Before the rivers".into();
        } else {
            panic!("File ▸ Save Point… asks for a name");
        }
        h.run_steps(3);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("versions-dialog.png")).unwrap();
        }
        h.get_by_label("Save Point").click();
        h.run_steps(2);
        wait(&mut h);
        assert_eq!(h.state().doc.world.points.len(), 1);
        assert_eq!(h.state().doc.world.points[0].name, "Before the rivers");
        assert!(h.state().doc.is_modified(), "a point is a change to save");

        // The map changes, and a second point is made of it.
        let brush = massif_core::Brush {
            shape: massif_core::BrushShape::Circle,
            radius: 60.0,
            profile: massif_core::Profile::Dome,
            strength: 1.0,
            rotation: 0.0,
            mask: None,
            filter: None,
        };
        let raise = |app: &mut MassifApp| {
            let doc = &mut app.doc;
            doc.history.begin("Raise", 0);
            massif_core::ops::raise(&mut doc.history.edit(&mut doc.world, 0), &brush, 300.0, 300.0, 30.0, massif_core::ops::Surface::Top);
            doc.commit();
            app.map_cache.invalidate_all();
        };
        raise(h.state_mut());
        h.state_mut().versions_action(&ctx, Action::Make);
        if let Some(Dialog::PointName(None, name)) = &mut h.state_mut().dialog {
            *name = "Version 2".into();
        }
        h.run_steps(2);
        h.get_by_label("Save Point").click();
        h.run_steps(2);
        wait(&mut h);
        assert_eq!(h.state().doc.world.points.len(), 2);

        // Compared with the first point: the raised land shows.
        h.state_mut().versions_action(&ctx, Action::Compare(Some(0)));
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().versions.compare.as_ref().is_some_and(|c| c.diff.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        h.run_steps(3);
        let diff = h.state().versions.compare.as_ref().and_then(|c| c.diff.clone()).expect("the comparison is worked out");
        assert!(diff.raised > 1000 && diff.lowered == 0, "{} raised, {} lowered", diff.raised, diff.lowered);
        assert!(diff.most_up > 20.0, "{}", diff.most_up);
        assert!(diff.tiles <= 9, "only the tiles under the brush: {}", diff.tiles);
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("versions-panel-dark.png")).unwrap();
            // The panel alone, tall enough for what the comparison found.
            h.state_mut().layout.toggle_maximize(area);
            h.run_steps(3);
            h.render().unwrap().save(dir.join("versions-compare.png")).unwrap();
            h.state_mut().layout.toggle_maximize(area);
            h.run_steps(2);
        }
        // ... and with the second, the same as the map.
        h.state_mut().versions_action(&ctx, Action::Compare(Some(1)));
        for _ in 0..400 {
            h.run_steps(1);
            if h.state().versions.compare.as_ref().is_some_and(|c| c.diff.is_some()) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        assert!(h.state().versions.compare.as_ref().unwrap().diff.as_ref().unwrap().is_empty());
        h.state_mut().versions_action(&ctx, Action::Compare(None));

        // Restored: the map is the first point's; undone, it is back.
        let tiles = |app: &mut MassifApp| -> Vec<(TilePos, massif_core::level::Blob)> { app.doc.world.snapshot().dims[0].levels[0].blobs().unwrap() };
        let raised = tiles(h.state_mut());
        h.state_mut().versions_action(&ctx, Action::Restore(0));
        h.run_steps(2);
        let first = h.state().doc.world.points[0].world.dims[0].levels[0].blobs().unwrap();
        assert!(tiles(h.state_mut()) == first);
        assert_eq!(h.state().doc.history.undo_label(), Some("Restore “Before the rivers”"));
        h.state_mut().undo(&ctx);
        assert!(tiles(h.state_mut()) == raised);
        assert_eq!(h.state().doc.world.points.len(), 2, "restoring keeps the points");

        // Renamed and saved: the file holds both points.
        h.state_mut().versions_action(&ctx, Action::Rename(1));
        if let Some(Dialog::PointName(Some(1), name)) = &mut h.state_mut().dialog {
            *name = "After the hill".into();
        }
        h.run_steps(2);
        h.get_by_label("Rename").click();
        h.run_steps(2);
        assert_eq!(h.state().doc.world.points[1].name, "After the hill");
        let folder = std::env::temp_dir().join(format!("massif-points-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("points.massif");
        h.state_mut().doc.path = Some(file.clone());
        h.state_mut().command(&ctx, crate::menu::Command::Save);
        wait(&mut h);
        assert!(!h.state().doc.is_modified());
        let back = massif_format::load(&file).unwrap();
        assert_eq!(back.points.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Before the rivers", "After the hill"]);

        // Opened as a copy: a map of its own, never saved, without points.
        h.state_mut().versions_action(&ctx, Action::Open { index: 0, read_only: false });
        h.run_steps(2);
        assert!(h.state().doc.path.is_none() && h.state().doc.read_only.is_none());
        assert!(h.state().doc.world.points.is_empty());
        assert!(h.state().doc.world.meta.name.ends_with("— Before the rivers"));
        assert!(tiles(h.state_mut()) == first);

        // Read-only: what is changed is taken back.
        h.state_mut().run(&ctx, Pending::Open(Some(file.clone())));
        wait(&mut h);
        assert_eq!(h.state().doc.world.points.len(), 2);
        h.state_mut().layout.current_mut().root.set_editor(area, EditorKind::Versions);
        h.state_mut().versions_action(&ctx, Action::Open { index: 1, read_only: true });
        h.run_steps(3);
        assert_eq!(h.state().doc.read_only.as_deref(), Some("After the hill"));
        let before = tiles(h.state_mut());
        raise(h.state_mut());
        assert!(tiles(h.state_mut()) != before);
        h.run_steps(2);
        assert!(tiles(h.state_mut()) == before, "a map open read-only keeps as it is");
        assert!(!h.state().doc.is_modified());
        if let Some(dir) = &dir {
            h.render().unwrap().save(dir.join("versions-read-only.png")).unwrap();
        }
        // Deleting a point (of the map reopened) asks first.
        h.state_mut().run(&ctx, Pending::Open(Some(file.clone())));
        wait(&mut h);
        h.state_mut().versions_action(&ctx, Action::Delete(0));
        h.run_steps(2);
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert_eq!(h.state().doc.world.points.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["After the hill"]);
        assert!(h.state().doc.is_modified());
        std::fs::remove_dir_all(&folder).unwrap();
    }

    /// The Ground cover tool: a cover picked in its settings, painted on
    /// the map, taken off with the right button. A plant of the game makes
    /// a cover of its own. With MASSIF_SNAPSHOT_DIR, the map is saved.
    #[test]
    fn covers_are_painted_and_shown() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 4,
                tiles_z: 4,
                landform: Landform::Hills,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 9);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            app.tools.select(Tool::Cover);
            app.tools.brush.radius = 50.0;
            app.tools.brush.strength = 1.0;
            app.tools.brush.profile = massif_core::Profile::Constant;
        }
        h.run_steps(5);
        use egui_kittest::kittest::Queryable;
        // In Tool Settings, and in the Layers panel.
        assert_eq!(h.get_all_by_label("Flowers").count(), 2);
        h.get_all_by_label("Flowers").next().unwrap().click();
        h.run_steps(3);
        let flowers = h.state().doc.world.registry.cover_by_name("Flowers").unwrap();
        assert_eq!(h.state().tools.cover, flowers);
        let rect = map_rect(h.state(), &h.ctx);
        let c = rect.center();
        drag_on(&mut h, c, c + vec2(60.0, 0.0));
        h.run_steps(10);
        assert_eq!(h.state().doc.history.undo_label(), Some("Ground cover"));
        let count = |h: &egui_kittest::Harness<'static, MassifApp>, n: u16| {
            let app = h.state();
            let at = app.maps.get(&app.active_map.unwrap()).unwrap().center;
            let (cx, cz) = (at[0].floor() as i32, at[1].floor() as i32);
            let mut k = 0;
            for z in cz - 30..cz + 30 {
                for x in cx - 30..cx + 30 {
                    let tile = app.doc.dimension().ground().view(TilePos::containing(x, z)).unwrap();
                    k += (tile.cover.get(massif_core::coords::local_index(x, z)) == n) as usize;
                }
            }
            k
        };
        assert!(count(&h, flowers) > 3000, "{}", count(&h, flowers));
        // What lies under the cursor says so.
        let probe = crate::tools::probe(&h.state().doc, h.state().maps.get(&h.state().active_map.unwrap()).unwrap().center[0] as i32, h.state().maps.get(&h.state().active_map.unwrap()).unwrap().center[1] as i32).unwrap();
        assert_eq!(probe.cover, flowers);
        h.event(egui::Event::PointerMoved(rect.right_bottom() - vec2(40.0, 60.0)));
        h.run_steps(10);
        h.render().unwrap().save(dir.join("cover-paint.png")).unwrap();
        // Close up, each plant where it grows.
        let id = h.state().active_map.unwrap();
        let zoom = h.state().maps.get(&id).unwrap().zoom;
        h.state_mut().maps.get_mut(&id).unwrap().zoom = zoom * 4.0;
        h.run_steps(20);
        h.render().unwrap().save(dir.join("cover-close.png")).unwrap();
        h.state_mut().maps.get_mut(&id).unwrap().zoom = zoom;
        h.run_steps(5);
        // The settings of the tool, the whole window for them.
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let settings = areas.iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::ToolSettings)).unwrap().0;
        h.state_mut().layout.toggle_maximize(settings);
        h.run_steps(10);
        h.render().unwrap().save(dir.join("cover-settings.png")).unwrap();
        h.state_mut().layout.toggle_maximize(settings);
        h.run_steps(5);
        // The right button takes it off.
        h.event(egui::Event::PointerMoved(c));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: c,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(4);
        }
        h.run_steps(5);
        assert_eq!(count(&h, flowers), 0);
        // A plant of the game makes a cover that grows it alone, once.
        let poppy = massif_assets::Block {
            id: "minecraft:poppy".into(),
            name: "Poppy".into(),
            top: None,
            side: None,
            bottom: None,
            top_tint: massif_assets::Tint::None,
            side_tint: massif_assets::Tint::None,
            full_cube: false,
            tall: false,
            color: [200, 40, 30],
            side_color: [200, 40, 30],
            category: massif_assets::Category::Plants,
        };
        let reg = &mut h.state_mut().doc.world.registry;
        let n = crate::panels::block_cover(reg, &poppy);
        assert_eq!(crate::panels::block_cover(reg, &poppy), n);
        let def = reg.cover(n).unwrap();
        assert_eq!((def.name.as_str(), def.plants.clone(), def.soil_only), ("Poppy", vec![("minecraft:poppy".to_owned(), 1)], true));
    }

    /// A Sponge schematic of a little oak — a trunk on a block of dirt, a
    /// crown of leaves, a vine — written to `path`.
    fn write_oak(path: &std::path::Path) {
        use fastnbt::Value;
        use std::io::Write;
        let (w, h, l) = (5i32, 7i32, 5i32);
        let states = ["minecraft:air", "minecraft:dirt", "minecraft:oak_log[axis=y]", "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]", "minecraft:vine[east=false,north=false,south=false,up=false,west=true]"];
        let mut data = Vec::new();
        for y in 0..h {
            for z in 0..l {
                for x in 0..w {
                    let crown = (3..7).contains(&y) && (x - 2).abs() + (z - 2).abs() <= if y == 6 { 1 } else { 3 };
                    let k = match (x, y, z) {
                        (2, 0, 2) => 1,
                        (2, 1..=4, 2) => 2,
                        (3, 2, 2) => 4,
                        _ if crown => 3,
                        _ => 0,
                    };
                    data.push(k as i8);
                }
            }
        }
        let palette = Value::Compound(states.iter().enumerate().map(|(k, s)| ((*s).to_owned(), Value::Int(k as i32))).collect());
        let root = Value::Compound(
            [
                ("Version", Value::Int(2)),
                ("Width", Value::Short(w as i16)),
                ("Height", Value::Short(h as i16)),
                ("Length", Value::Short(l as i16)),
                ("Palette", palette),
                ("BlockData", Value::ByteArray(fastnbt::ByteArray::new(data))),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        );
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(&fastnbt::to_bytes(&root).unwrap()).unwrap();
        std::fs::write(path, e.finish().unwrap()).unwrap();
    }

    /// Objects: a schematic read into a layer of its own, painted on the
    /// map, shown there and in 3D, taken off with the right button. With
    /// MASSIF_SNAPSHOT_DIR, the map, the 3D view and the settings are
    /// saved.
    #[test]
    fn objects_are_imported_painted_and_shown() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let file = std::env::temp_dir().join(format!("massif-oak-{}.schem", std::process::id()));
        write_oak(&file);
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            let world = generate(&GenParams {
                tiles_x: 4,
                tiles_z: 4,
                landform: Landform::Hills,
                ..GenParams::default()
            });
            app.doc = crate::doc::Document::new(world, None, 9);
            app.map_cache.invalidate_all();
            app.view3d.invalidate_all();
            let app = &mut *app;
            let note = panels::import_objects(&mut app.doc, &mut app.tools, std::slice::from_ref(&file), None, None);
            assert_eq!(note, "Read 1 object (Sponge schematic)");
            app.tools.brush.radius = 60.0;
            app.tools.brush.strength = 1.0;
            app.tools.brush.profile = massif_core::Profile::Constant;
        }
        let _ = std::fs::remove_file(&file);
        let reg = &h.state().doc.world.registry;
        assert_eq!((reg.objects.len(), reg.object_layers.len()), (1, 1));
        assert_eq!(reg.objects[0].anchor, [2, 1, 2]);
        assert_eq!(h.state().tools.object_layer, 1);
        assert_eq!(h.state().tools.tool, Tool::Objects);
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let c = rect.center();
        drag_on(&mut h, c, c + vec2(30.0, 0.0));
        h.run_steps(10);
        assert_eq!(h.state().doc.history.undo_label(), Some("Objects"));
        let at = |h: &egui_kittest::Harness<'static, MassifApp>| {
            let app = h.state();
            let at = app.maps.get(&app.active_map.unwrap()).unwrap().center;
            (at[0].floor() as i32, at[1].floor() as i32)
        };
        let painted = |h: &egui_kittest::Harness<'static, MassifApp>| {
            let (cx, cz) = at(h);
            let dim = h.state().doc.dimension();
            (cz - 20..cz + 20).flat_map(|z| (cx - 20..cx + 20).map(move |x| (x, z))).filter(|&(x, z)| dim.ground().view(TilePos::containing(x, z)).unwrap().objects.get(massif_core::coords::local_index(x, z)) == 1).count()
        };
        assert_eq!(painted(&h), 1600);
        // Trees stand there: as many as the grid holds, more or less.
        let (cx, cz) = at(&h);
        let app = h.state();
        let dim = app.doc.dimension();
        let mat = massif_core::compose::Materializer::new(&app.doc.world.registry, app.doc.world.meta.seed, dim.min_y);
        let pos = TilePos::containing(cx, cz);
        let stack = massif_core::TileStack::new(dim, pos, false).unwrap();
        let trees = massif_core::objects::tile_placements(&app.doc.world.registry, &mat, app.doc.world.meta.seed as u64, false, &stack, pos);
        assert!(!trees.is_empty());
        // Close up on the map, each tree from above.
        let id = h.state().active_map.unwrap();
        let zoom = h.state().maps.get(&id).unwrap().zoom;
        h.state_mut().maps.get_mut(&id).unwrap().zoom = zoom * 3.0;
        h.event(egui::Event::PointerMoved(rect.right_bottom() - vec2(40.0, 60.0)));
        h.run_steps(20);
        h.render().unwrap().save(dir.join("objects-map.png")).unwrap();
        h.state_mut().maps.get_mut(&id).unwrap().zoom = zoom;
        // The settings, the whole window for them.
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let settings = areas.iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::ToolSettings)).unwrap().0;
        h.state_mut().layout.toggle_maximize(settings);
        h.run_steps(10);
        h.render().unwrap().save(dir.join("objects-settings.png")).unwrap();
        h.state_mut().layout.toggle_maximize(settings);
        h.run_steps(5);
        // In 3D, close.
        {
            let app = h.state_mut();
            app.layout.activate(3);
            let y = trees[0].y as f64;
            app.view3d.camera.target = [trees[0].x as f64, y, trees[0].z as f64];
            app.view3d.camera.distance = 70.0;
            app.view3d.camera.pitch = 0.5;
        }
        h.run_steps(5);
        while h.state().view3d.pending() > 0 {
            h.step();
        }
        h.run_steps(3);
        let (w, hgt, rgba) = h.state().view3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("objects-3d.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        h.state_mut().layout.activate(0);
        h.run_steps(5);
        // The right button takes them off.
        let c = map_rect(h.state(), &h.ctx).center();
        h.event(egui::Event::PointerMoved(c));
        h.step();
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: c,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: Modifiers::NONE,
            });
            h.run_steps(4);
        }
        h.run_steps(5);
        assert_eq!(painted(&h), 0);
    }

    /// Three clicks of the Sculpt tool with a large brush of the shape
    /// chosen for every tool (a stamp here) raise a bump in that shape, a
    /// few blocks high — not a mountain. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn three_clicks_raise_a_bump() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        {
            let app = h.state_mut();
            app.layout.active = 4; // 3D Sculpt
            let source = BrushSource::Stamp(massif_core::stamps::Stamp::ALL[3]);
            assert!(panels::select_brush(&mut app.tools, &app.brushes, source), "the stamp is a brush");
            app.tools.sculpt.tool = Tool::Build;
            app.tools.brush.radius = 40.0;
            app.tools.brush.strength = 0.5;
        }
        // Once the view has framed the map: over the highest land near
        // its middle.
        h.run_steps(5);
        {
            let app = h.state_mut();
            let (x, z, y) = (-300..300)
                .step_by(10)
                .flat_map(|z| (-300..300).step_by(10).map(move |x| (x, z)))
                .filter_map(|(x, z)| crate::tools::probe(&app.doc, x, z).map(|p| (x, z, p.height)))
                .fold((0, 0, f32::MIN), |b, c| if c.2 > b.2 { c } else { b });
            app.sculpt3d.camera = crate::view3d::Camera { target: [x as f64, y as f64, z as f64], yaw: 0.6, pitch: 0.55, distance: 220.0 };
        }
        h.run_steps(30);
        let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
        let view = areas.into_iter().find(|(id, _)| h.state().layout.current().root.editor_of(*id) == Some(EditorKind::Sculpt3d)).map(|(_, r)| r).unwrap();
        let at = view.center();
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(3);
        let hit = h.state().sculpt3d.hover().expect("the cursor points at the terrain");
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("sculpt-three-clicks-before.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
        let tops = |h: &egui_kittest::Harness<'static, MassifApp>| -> Vec<f32> {
            let dim = h.state().doc.dimension();
            let mut spans = Vec::new();
            (-40..=40)
                .flat_map(|dz| (-40..=40).map(move |dx| (hit.block.0 + dx, hit.block.2 + dz)))
                .map(|(x, z)| {
                    let stack = massif_core::TileStack::new(dim, TilePos::containing(x, z), false).unwrap();
                    stack.spans(massif_core::coords::local_index(x, z), &mut spans);
                    spans.last().map_or(0.0, |s| s.top)
                })
                .collect()
        };
        let before = tops(&h);
        // Three clicks, as quick as a hand makes them.
        for _ in 0..3 {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
                for _ in 0..6 {
                    h.step();
                    std::thread::sleep(std::time::Duration::from_millis(16));
                }
            }
        }
        h.run_steps(20);
        let after = tops(&h);
        let rise = before.iter().zip(&after).map(|(a, b)| b - a).fold(0.0f32, f32::max);
        // A few blocks (more on a steep slope, pushed along it); the brush
        // 80 blocks across used to fill tens of blocks of it.
        assert!(rise > 0.5 && rise < 12.0, "the land rose by {rise} blocks");
        let (w, hgt, rgba) = h.state().sculpt3d.read_back().expect("the 3D view rendered");
        image::save_buffer(dir.join("sculpt-three-clicks.png"), &rgba, w, hgt, image::ColorType::Rgba8).unwrap();
    }

    /// Menus as they show: the commands of an area, the list of a combo
    /// box. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn menus_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        // The "…" of the map's area, flush with its right edge.
        click_on(&mut h, pos2(rect.right() - 20.0, rect.top() + 17.0), egui::PointerButton::Primary);
        h.run_steps(5);
        use egui_kittest::kittest::Queryable;
        h.get_by_label_contains("Split vertically");
        h.render().unwrap().save(dir.join("menu-area.png")).unwrap();
    }

    /// A graph whose output takes nothing says so, and starts from the
    /// map in one click.
    #[test]
    fn a_graph_without_relief_says_so() {
        use egui_kittest::kittest::Queryable;
        use massif_core::graph::{Graph, NodeKind};
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        {
            let app = h.state_mut();
            let mut g = Graph::default();
            g.add(NodeKind::HeightOutput, [0.0, 0.0]);
            app.doc.world.meta.graph = Some(g);
            app.layout.switch_side(true);
            app.graph.graph_replaced();
        }
        h.run_steps(10);
        h.get_by_label("No relief to show");
        h.get_by_label_contains("Start from this map").click();
        h.run_steps(10);
        assert!(h.query_by_label("No relief to show").is_none(), "the map is shown");
        let g = h.state().doc.world.meta.graph.as_ref().unwrap();
        assert!(g.nodes.iter().any(|n| n.kind == NodeKind::MapHeight));
    }

    /// The help: F1 opens it, a search narrows its topics. Needs a GPU:
    /// renders with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn the_help_opens_and_searches() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        h.key_press(Key::F1);
        h.run_steps(4);
        h.get_by_label("Massif Help");
        assert!(h.query_all_by_label("Getting started").count() >= 2, "in the list, and as the title");
        h.get_by_label("Terrain graphs");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("help.png")).unwrap();
        }
        if let Some(Dialog::Help(state)) = &mut h.state_mut().dialog {
            state.query = "beaches".into();
        }
        h.run_steps(4);
        assert!(h.query_by_label("Terrain graphs").is_none(), "topics without it are hidden");
        h.get_by_label("WorldPainter worlds");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("help-search.png")).unwrap();
        }
    }

    /// Right-click on a workspace: back to how it was.
    #[test]
    fn a_workspace_resets_from_its_tab() {
        use egui_kittest::kittest::Queryable;
        let mut h = harness(ThemeChoice::Dark);
        h.run_steps(5);
        let first = h.state().layout.current().root.count();
        {
            let app = h.state_mut();
            let area = app.layout.current().root.first_area();
            app.layout.split(area, crate::layout::Dir::Row);
        }
        h.run_steps(3);
        assert_eq!(h.state().layout.current().root.count(), first + 1);
        h.get_by_label("Layout").click_secondary();
        h.run_steps(3);
        h.get_by_label_contains("Reset").click();
        h.run_steps(3);
        assert_eq!(h.state().layout.current().root.count(), first);
    }

    /// Every corner of an area splits it, the new area on the corner's
    /// side: across from a left corner, on the left; up from a bottom
    /// corner, below; and so on.
    #[test]
    fn every_corner_splits_the_area() {
        if std::env::var_os("MASSIF_SNAPSHOT_DIR").is_none() {
            return;
        }
        let button = |pressed, pos| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        // Corner, where the drag ends (fractions of the area), and on which
        // side of the map the new area should be: (left, above).
        type Case = (Corner, [f32; 2], (Option<bool>, Option<bool>));
        let cases: [Case; 6] = [
            (Corner::TopLeft, [0.4, 0.05], (Some(true), None)),
            (Corner::TopLeft, [0.02, 0.5], (None, Some(true))),
            (Corner::BottomLeft, [0.4, 0.97], (Some(true), None)),
            (Corner::BottomLeft, [0.03, 0.4], (None, Some(false))),
            (Corner::BottomRight, [0.6, 0.95], (Some(false), None)),
            (Corner::BottomRight, [0.97, 0.6], (None, Some(false))),
        ];
        for (corner, to, (left, above)) in cases {
            let mut h = harness(ThemeChoice::Dark);
            h.state_mut().layout.active = 6; // Minimal: tools | map
            h.run_steps(5);
            let rect = map_rect(h.state(), &h.ctx);
            let before = h.state().layout.current().root.count();
            let from = pos2(if corner.left() { rect.left() + 4.0 } else { rect.right() - 4.0 }, if corner.top() { rect.top() + 4.0 } else { rect.bottom() - 4.0 });
            let to = rect.min + vec2(to[0] * rect.width(), to[1] * rect.height());
            h.event(egui::Event::PointerMoved(from));
            h.step();
            h.event(button(true, from));
            h.step();
            for i in 1..=10 {
                h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
                h.step();
            }
            h.event(button(false, to));
            h.run_steps(5);
            assert_eq!(h.state().layout.current().root.count(), before + 1, "{corner:?} to {to:?}");
            let new = h.state().layout.current().root.max_id_pub().unwrap();
            assert_eq!(h.state().layout.current().root.editor_of(new), Some(EditorKind::Empty));
            let areas: Vec<(AreaId, Rect)> = h.ctx.data(|d| d.get_temp(Id::unique("massif areas"))).unwrap();
            let new_rect = areas.iter().find(|(id, _)| *id == new).map(|(_, r)| *r).unwrap();
            let map = map_rect(h.state(), &h.ctx);
            if let Some(left) = left {
                assert_eq!(new_rect.center().x < map.center().x, left, "{corner:?}: the new area on the {} side", if left { "left" } else { "right" });
            }
            if let Some(above) = above {
                assert_eq!(new_rect.center().y < map.center().y, above, "{corner:?}: the new area {}", if above { "above" } else { "below" });
            }
        }
    }

    /// A corner dragged, half way, for the eye (MASSIF_SNAPSHOT_DIR): the
    /// bottom-left corner of the map lit, the split's line and the side the
    /// new area opens on.
    #[test]
    fn a_corner_dragged_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let mut h = harness(ThemeChoice::Dark);
        h.state_mut().layout.active = 6; // Minimal: tools | map
        h.run_steps(5);
        let rect = map_rect(h.state(), &h.ctx);
        let from = pos2(rect.left() + 4.0, rect.bottom() - 4.0);
        let to = rect.min + vec2(0.35 * rect.width(), 0.9 * rect.height());
        h.event(egui::Event::PointerMoved(from));
        h.step();
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
        h.step();
        for i in 1..=10 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 10.0)));
            h.step();
        }
        let image = h.render().expect("rendered");
        let crop = image::imageops::crop_imm(&image, rect.left() as u32, (rect.bottom() - 260.0) as u32, 640, 260).to_image();
        crop.save(dir.join("corner-drag.png")).unwrap();
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
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
        assert_eq!(areas.len(), 10);
        assert_eq!(seps.len(), 9);
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

    /// An app as at the very first start: settings never saved, so the
    /// tour wanted; a map open, in a window of `size`.
    fn first_run(theme: ThemeChoice, size: egui::Vec2) -> egui_kittest::Harness<'static, MassifApp> {
        egui_kittest::Harness::builder()
            .with_size(size)
            .with_pixels_per_point(std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0))
            .with_accessibility_check(false)
            .wgpu()
            .build_eframe(move |cc| {
                theme::install(&cc.egui_ctx);
                let world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Islands, ..GenParams::default() });
                let mut app = MassifApp::for_tests(world);
                app.settings.theme = theme;
                app.settings.tour_done = false;
                app.tour_wanted = true;
                app
            })
    }

    /// The tour shows on the first run, goes step by step, and once skipped
    /// is marked done: the next start, from the settings saved, has none.
    /// Settings saved by a version before the tour count as done.
    #[test]
    fn the_tour_shows_on_the_first_run_only() {
        use egui_kittest::kittest::Queryable;
        let mut h = first_run(ThemeChoice::Light, vec2(1600.0, 1000.0));
        h.run_steps(4);
        assert_eq!(h.state().tour.as_ref().map(|t| t.title()), Some("Welcome to Massif"));
        h.get_by_label("Next").click();
        h.run_steps(3);
        assert_eq!(h.state().tour.as_ref().map(|t| t.step), Some(1));
        h.get_by_label("Back").click();
        h.run_steps(2);
        assert_eq!(h.state().tour.as_ref().map(|t| t.step), Some(0));
        h.get_by_label("Skip tour").click();
        h.run_steps(2);
        assert!(h.state().tour.is_none(), "skipped");
        assert!(h.state().settings.tour_done, "skipping marks it done");
        // The next start reads the settings saved.
        let saved: Settings = serde_json::from_str(&serde_json::to_string(&h.state().settings).unwrap()).unwrap();
        assert!(saved.tour_done);
        // Settings from before the tour: someone who knows Massif.
        let old: Settings = serde_json::from_str(r#"{"theme":"Dark","recent":[]}"#).unwrap();
        assert!(old.tour_done);
        // The very first settings: not done.
        assert!(!Settings::default().tour_done);
    }

    /// Escape skips; "Don't show again" unticked leaves the tour for the
    /// next start; Done at the last step ends it; Help ▸ Take the tour
    /// shows it again.
    #[test]
    fn the_tour_ends_as_asked() {
        use egui_kittest::kittest::Queryable;
        let mut h = first_run(ThemeChoice::Dark, vec2(1600.0, 1000.0));
        h.run_steps(4);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().tour.is_none() && h.state().settings.tour_done, "Escape skips");
        let ctx = h.ctx.clone();
        h.state_mut().command(&ctx, Command::Tour);
        h.run_steps(3);
        assert!(h.state().tour.is_some(), "Help ▸ Take the tour");
        h.get_by_label("Don't show again").click();
        for _ in 0..crate::tour::STEP_COUNT - 1 {
            h.get_by_label("Next").click();
            h.run_steps(2);
        }
        assert_eq!(h.state().tour.as_ref().map(|t| t.title()), Some("Save and export"));
        h.get_by_label("Done").click();
        h.run_steps(2);
        assert!(h.state().tour.is_none());
        assert!(!h.state().settings.tour_done, "shown again at the next start");
    }

    /// Every step of the tour, for the eye (MASSIF_SNAPSHOT_DIR): in a
    /// common window and a small one, light and dark.
    #[test]
    fn tour_snapshots() {
        use egui_kittest::kittest::Queryable;
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        for (size_name, size) in [("1440", vec2(1440.0, 900.0)), ("1024", vec2(1024.0, 680.0))] {
            for theme in [ThemeChoice::Light, ThemeChoice::Dark] {
                let mut h = first_run(theme, size);
                h.run_steps(6);
                for k in 0..crate::tour::STEP_COUNT {
                    // Past the light's glide.
                    for _ in 0..30 {
                        h.step();
                    }
                    std::thread::sleep(std::time::Duration::from_millis(350));
                    h.run_steps(3);
                    let name = format!("tour-{size_name}-{}-{}.png", if theme == ThemeChoice::Dark { "dark" } else { "light" }, k + 1);
                    h.render().unwrap().save(dir.join(name)).unwrap();
                    if k + 1 < crate::tour::STEP_COUNT {
                        h.get_by_label("Next").click();
                    }
                }
            }
        }
    }

    /// A sample of the home screen opens in one click: its relief built
    /// from its template, shown on the World side, with nothing to save;
    /// the tour of a first run waits for it.
    #[test]
    fn a_sample_opens_with_its_relief() {
        use egui_kittest::kittest::Queryable;
        let mut h = first_run(ThemeChoice::Light, vec2(1440.0, 900.0));
        h.state_mut().has_map = false;
        h.state_mut().open_home(Page::NewMap);
        h.run_steps(3);
        assert!(h.state().tour.is_none() && h.state().tour_wanted, "no tour over the home screen");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("home-samples.png")).unwrap();
        }
        let started = std::time::Instant::now();
        h.get_by_label("Alpine range").click();
        h.run_steps(2);
        while h.state().sample.is_some() || !h.state().jobs.is_empty() {
            assert!(started.elapsed().as_secs() < 120, "the sample was not made");
            assert!(h.state().tour.is_none(), "the tour waits for the sample");
            h.step();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let took = started.elapsed().as_secs_f32();
        h.run_steps(3);
        let app = h.state();
        eprintln!("sample made in {took:.2} s");
        assert_eq!(app.doc.world.meta.name, "Alpine range");
        assert!(!app.layout.terrain(), "on the World side");
        assert!(!app.doc.is_modified(), "nothing to save");
        let dim = app.doc.dimension();
        let heights: Vec<f32> = (0..16).flat_map(|i| (0..16).map(move |j| (i * 48 - 384, j * 48 - 384))).filter_map(|(x, z)| massif_core::ops::height_at(dim, 0, x, z, massif_core::ops::Surface::Top)).collect();
        let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
        assert!(hi - lo > 10.0, "relief: {lo} … {hi}");
        assert!(app.tour.is_some(), "then the tour of the first run");
        if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
            h.render().unwrap().save(dir.join("sample-opened.png")).unwrap();
        }
    }
}
