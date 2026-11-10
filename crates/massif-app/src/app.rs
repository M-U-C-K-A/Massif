//! The application: window chrome, workspaces and areas, dialogs, jobs.

use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
use crate::jobs::{self, Job, Outcome};
use crate::layout::{AreaId, Dir, EditorKind, Layout, Node, Zone};
use crate::map_render::MapCache;
use crate::map_view::{MapInput, MapView};
use crate::panels;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Tool, ToolState};
use crate::view3d::View3d;
use egui::{
    Align, Align2, CornerRadius, CursorIcon, FontId, Id, Key, KeyboardShortcut, LayerId,
    Modifiers, Order, Rect, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, ViewportCommand,
    pos2, vec2,
};
use massif_anvil::{ExportOptions, GameMode, Surroundings};
use massif_core::World;
use massif_core::generate::{GenParams, Landform, generate};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

const HEADER: f32 = 32.0;
const GAP: f32 = 5.0;
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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemeChoice::System,
            recent: Vec::new(),
            solo: false,
            light: crate::map_render::DEFAULT_LIGHT,
        }
    }
}

/// An action that replaces or closes the document, run once unsaved changes
/// are dealt with.
#[derive(Debug, Clone)]
enum Pending {
    New(GenParams),
    Open(Option<PathBuf>),
    Quit,
}

struct NewWorldForm {
    params: GenParams,
    seed: String,
    width: i32,
    depth: i32,
}

struct ExportForm {
    parent: PathBuf,
    name: String,
    game_mode: GameMode,
    surroundings: Surroundings,
    structures: bool,
    overwrite: bool,
}

enum Dialog {
    NewWorld(NewWorldForm),
    Export(ExportForm),
    Unsaved(Pending),
    About,
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
        app.unified_titlebar = cfg!(target_os = "macos");
        app.brushes = BrushLibrary::installed();
        app.brushes.load_in_background(&cc.egui_ctx);
        if let Some(path) = open {
            app.jobs.push(Job::open(&cc.egui_ctx, path));
        }
        app
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
        self.dialog = Some(Dialog::Export(ExportForm {
            parent,
            name,
            game_mode: GameMode::Creative,
            surroundings: Surroundings::Vanilla,
            structures: true,
            overwrite: false,
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
        }));
    }

    fn undo(&mut self, ctx: &egui::Context) {
        if let Some(label) = self.doc.undo() {
            self.say(ctx, format!("Undid {label}"), false, None);
            self.map_cache.invalidate_all();
            self.view3d.invalidate_all();
        }
    }

    fn redo(&mut self, ctx: &egui::Context) {
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
                if ui.button(theme::with_icon(Icon::Info, "About Massif")).clicked() {
                    self.dialog = Some(Dialog::About);
                }
                ui.separator();
                ui.label(RichText::new("Shortcuts (as in WorldPainter)").strong());
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
                ui.label(RichText::new("3D view").strong());
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
            ui.spacing_mut().item_spacing.x = 2.0;
            let mut remove = None;
            for i in 0..self.layout.workspaces.len() {
                let active = self.layout.active == i;
                let name = self.layout.workspaces[i].name.clone();
                let text = if active { RichText::new(name).strong() } else { RichText::new(name) };
                let r = ui.selectable_label(active, text);
                if r.clicked() {
                    self.layout.active = i;
                }
                r.context_menu(|ui| {
                    if ui.button("Duplicate").clicked() {
                        self.layout.active = i;
                        self.layout.duplicate_current();
                    }
                    if ui.add_enabled(self.layout.workspaces.len() > 1, egui::Button::new("Delete")).clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.layout.remove_workspace(i);
            }
            if ui.small_button(theme::glyph(Icon::Plus).to_string()).on_hover_text("New workspace (a copy of this one)").clicked() {
                self.layout.duplicate_current();
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
                    let color = if status.error { pal.danger } else { pal.text };
                    let icon = if status.error { Icon::TriangleAlert } else { Icon::Check };
                    ui.label(RichText::new(theme::with_icon(icon, &status.text)).color(color).size(12.0));
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
        painter.rect(rect, 8, pal.panel, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let header = Rect::from_min_size(rect.min, vec2(rect.width(), HEADER));
        painter.rect_filled(
            header.shrink(1.0),
            CornerRadius {
                nw: 7,
                ne: 7,
                sw: 0,
                se: 0,
            },
            pal.header,
        );
        painter.hline(header.x_range(), header.bottom(), Stroke::new(1.0, pal.border));
        let content = Rect::from_min_max(pos2(rect.min.x + 1.0, header.bottom() + 1.0), pos2(rect.max.x - 1.0, rect.max.y - 1.0));
        let maximized = self.layout.current().maximized == Some(id);
        let single = self.layout.current().root.count() == 1;

        // ---- header --------------------------------------------------------
        let mut hui = ui.new_child(
            UiBuilder::new()
                .id_salt(("area header", id))
                .max_rect(header.shrink2(vec2(6.0, 3.0)))
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
            .add(egui::Label::new(RichText::new(editor.label()).strong()).sense(Sense::drag()).selectable(false))
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
            EditorKind::ToolSettings => scrolled(&mut cui, &mut |ui| panels::tool_settings(ui, &mut self.tools, &mut self.doc)),
            EditorKind::BrushSettings => scrolled(&mut cui, &mut |ui| panels::brush_settings(ui, &mut self.tools, &mut self.brushes)),
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
                    solo_changed |= panels::terrain_panel(ui, &mut self.tools, &self.doc.world.registry, &mut self.settings.solo, &mut self.terrain_search);
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
                    ui.label(RichText::new(label).strong());
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
        let Some(dialog) = self.dialog.take() else { return };
        let mut keep = true;
        match dialog {
            Dialog::Unsaved(pending) => {
                let mut choice = None;
                let response = egui::Modal::new(Id::unique("unsaved")).show(ctx, |ui| {
                    ui.set_width(380.0);
                    ui.label(RichText::new(format!("Save the changes to “{}”?", self.doc.world.meta.name)).strong().size(15.0));
                    ui.label(RichText::new("Your changes will be lost if you don't save them.").color(Palette::of(ui).weak));
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Don't Save").clicked() {
                            choice = Some(1);
                        }
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(RichText::new("Save").strong()).clicked() {
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
                    ui.set_width(460.0);
                    ui.label(RichText::new("New world").strong().size(16.0));
                    ui.add_space(6.0);
                    egui::Grid::new("new world form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut form.params.name);
                        ui.end_row();
                        ui.label("Seed");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut form.seed).desired_width(180.0));
                            if ui.button(theme::glyph(Icon::Dice5).to_string()).on_hover_text("Random seed").clicked() {
                                form.seed = random_seed().to_string();
                            }
                        });
                        ui.end_row();
                        ui.label("Size");
                        ui.vertical(|ui| {
                            ui.horizontal_wrapped(|ui| {
                                for (blocks, label) in [(512, "512"), (1024, "1k"), (2048, "2k"), (4096, "4k"), (8192, "8k"), (16384, "16k")] {
                                    let selected = form.width == blocks && form.depth == blocks;
                                    if ui.selectable_label(selected, label).clicked() {
                                        form.width = blocks;
                                        form.depth = blocks;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.add(egui::DragValue::new(&mut form.width).range(128..=65536).speed(128.0).suffix(" wide"));
                                ui.label("×");
                                ui.add(egui::DragValue::new(&mut form.depth).range(128..=65536).speed(128.0).suffix(" deep"));
                                ui.label(RichText::new("blocks").color(Palette::of(ui).weak));
                            });
                        });
                        ui.end_row();
                        ui.label("Landform");
                        ui.horizontal_wrapped(|ui| {
                            for l in Landform::ALL {
                                ui.selectable_value(&mut form.params.landform, l, l.label());
                            }
                        });
                        ui.end_row();
                        ui.label("Water level");
                        ui.add(egui::DragValue::new(&mut form.params.sea_level).range(-63..=318));
                        ui.end_row();
                    });
                    let tiles = ((form.width + 127) / 128) as i64 * ((form.depth + 127) / 128) as i64;
                    ui.add_space(4.0);
                    ui.label(
                        RichText::new(format!(
                            "{} tiles · roughly {} on disk. Tiles stay compressed in memory until you edit them.",
                            tiles,
                            panels::human((tiles * 12_000) as u64)
                        ))
                        .size(11.5)
                        .color(Palette::of(ui).weak),
                    );
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            if ui.button(RichText::new("Create").strong()).clicked() {
                                create = true;
                            }
                            if ui.button("Cancel").clicked() {
                                keep = false;
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
                let mut start = false;
                let response = egui::Modal::new(Id::unique("export")).show(ctx, |ui| {
                    ui.set_width(520.0);
                    let pal = Palette::of(ui);
                    ui.label(RichText::new("Export to Minecraft").strong().size(16.0));
                    ui.label(
                        RichText::new("Writes a Minecraft Java world (region files and level.dat). It opens in 1.18.2 and every later version, which upgrades it on load.")
                            .size(11.5)
                            .color(pal.weak),
                    );
                    ui.add_space(6.0);
                    egui::Grid::new("export form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Saves folder");
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(form.parent.display().to_string()).size(11.5));
                            if ui.small_button("Choose…").clicked()
                                && let Some(dir) = rfd::FileDialog::new().set_directory(&form.parent).pick_folder()
                            {
                                form.parent = dir;
                            }
                        });
                        ui.end_row();
                        ui.label("World folder");
                        ui.text_edit_singleline(&mut form.name);
                        ui.end_row();
                        ui.label("Game mode");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut form.game_mode, GameMode::Creative, "Creative");
                            ui.selectable_value(&mut form.game_mode, GameMode::Survival, "Survival");
                            ui.selectable_value(&mut form.game_mode, GameMode::Adventure, "Adventure");
                        });
                        ui.end_row();
                        ui.label("Around the map");
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut form.surroundings, Surroundings::Vanilla, "Vanilla terrain");
                            ui.selectable_value(&mut form.surroundings, Surroundings::Void, "Void");
                        });
                        ui.end_row();
                        ui.label("");
                        ui.checkbox(&mut form.structures, "Generate structures beyond the map");
                        ui.end_row();
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
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            let ok = !form.name.trim().is_empty() && (!exists || form.overwrite);
                            if ui.add_enabled(ok, egui::Button::new(RichText::new("Export").strong())).clicked() {
                                start = true;
                            }
                            if ui.button("Cancel").clicked() {
                                keep = false;
                            }
                        });
                    });
                });
                if response.should_close() {
                    keep = false;
                }
                if start {
                    let dir = form.parent.join(form.name.trim());
                    let options = ExportOptions {
                        level_name: Some(form.name.trim().to_owned()),
                        game_mode: form.game_mode,
                        surroundings: form.surroundings,
                        generate_structures: form.structures,
                        overwrite: form.overwrite,
                        ..ExportOptions::default()
                    };
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
                    ui.label(RichText::new(format!("Imported “{}”", report.name)).strong().size(16.0));
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
                            if ui.button(RichText::new("Save as .massif…").strong()).clicked() {
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
                    ui.label(RichText::new(format!("{}  Massif", theme::glyph(Icon::MountainSnow))).strong().size(18.0));
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
        self.handle_close(&ctx);
        self.shortcuts(&ctx);
        self.map_cache.sync_generation(self.doc.generation);
        if let Some(msg) = self.feedback.take() {
            self.say(&ctx, msg, false, None);
        }

        let pal = Palette::of(ui);
        let margin = if self.unified_titlebar { 7 } else { 4 };
        let bar = egui::Panel::top("menu bar")
            .frame(egui::Frame::new().fill(pal.header).inner_margin(egui::Margin::symmetric(8, margin)).stroke(Stroke::new(1.0, pal.border)))
            .show(ui, |ui| {
                ui.horizontal(|ui| self.top_bar(ui));
            });
        if self.unified_titlebar {
            frame.set_traffic_lights_position(&ctx, bar.response.rect.y_range(), 14.0);
        }
        egui::Panel::top("workspaces")
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin { left: 8, right: 10, top: 5, bottom: 1 }))
            .show(ui, |ui| self.workspace_tabs(ui));
        egui::Panel::bottom("status bar")
            .frame(egui::Frame::new().fill(pal.header).inner_margin(egui::Margin::symmetric(10, 4)).stroke(Stroke::new(1.0, pal.border)))
            .show(ui, |ui| self.status_bar(ui));
        egui::CentralPanel::default()
            .frame(egui::Frame::new().fill(pal.app_bg).inner_margin(egui::Margin::same(4)))
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
        eframe::set_value(storage, "massif.tools", &self.tools);
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 1.0]
    }
}

/// Where the layout is saved. A new key when the default layouts change a
/// lot, so that everyone gets the new editors.
const LAYOUT_KEY: &str = "massif.layout.v2";

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
            app.view3d.follow_map = false;
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

        // The whole map in the 3D view, from far away: the far terrain fills
        // in everything beyond the detailed area.
        {
            let app = h.state_mut();
            app.dialog = None;
            app.layout.active = 3;
            app.view3d.camera.distance = 5200.0;
            app.view3d.camera.pitch = 0.75;
        }
        for _ in 0..400 {
            h.step();
        }
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
