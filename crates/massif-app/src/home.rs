//! The home screen: where Massif opens, and where a new map is made.
//!
//! No map is shown before there is one. On the left, the ways in — a new
//! map, a map from a height map, a map of one's own to open, a WorldPainter
//! world to import — and the maps opened lately; in the middle, the
//! settings of a new map with its preview. Behind them, ridges drawn in
//! dots, lit by the accent colour.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, Part, button_group, input_group};
use egui::{Align, Align2, Color32, CursorIcon, FontId, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, UiBuilder, pos2, vec2};
use massif_core::generate::{FromHeightmap, GenParams, Landform};
use massif_core::graph::Template;
use massif_core::heightmap::Heightmap;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::SystemTime;

/// Width of the sidebar.
const SIDEBAR: f32 = 268.0;
/// Between the settings of a new map and its preview.
const GUTTER: f32 = 20.0;
/// The least room the settings take beside the preview.
const MIN_SETTINGS: f32 = 430.0;
/// The height of a row of settings: their controls (input and button groups).
const ROW_H: f32 = 30.0;

/// What the middle of the home screen shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Page {
    NewMap,
    FromHeightmap,
}

/// The settings of a new map, and its preview.
pub struct NewMap {
    pub params: GenParams,
    pub seed: String,
    pub width: i32,
    pub depth: i32,
    /// The terrains a new map starts with (for the surface and the preview).
    pub registry: massif_core::Registry,
    /// A template of the terrain generator to make the relief with, rather
    /// than the landform.
    pub template: Option<Template>,
    preview: Option<(String, TextureHandle)>,
    /// The preview being drawn on a thread of its own, for these settings.
    previewing: Option<(String, Receiver<egui::ColorImage>)>,
    /// The picture of the template, and the one being drawn.
    template_preview: Option<(Template, TextureHandle)>,
    drawing: Option<(Template, Receiver<egui::ColorImage>)>,
    /// How tall the settings were last laid out: the preview beside them is
    /// as tall.
    rows: f32,
}

impl NewMap {
    pub fn new(registry: massif_core::Registry) -> Self {
        let params = GenParams {
            name: String::new(),
            seed: random_seed(),
            ..GenParams::default()
        };
        Self {
            seed: params.seed.to_string(),
            width: params.tiles_x * 128,
            depth: params.tiles_z * 128,
            params,
            registry,
            template: None,
            preview: None,
            previewing: None,
            template_preview: None,
            drawing: None,
            rows: 6.0 * (ROW_H + 14.0) - 14.0,
        }
    }

    /// The settings as the generator takes them.
    pub fn params(&self) -> GenParams {
        let mut p = self.params.clone();
        p.seed = parse_seed(&self.seed);
        p.tiles_x = (self.width + 127) / 128;
        p.tiles_z = (self.depth + 127) / 128;
        if p.name.trim().is_empty() {
            p.name = "New World".into();
        }
        // A template makes the relief on the Terrain side: the map starts
        // flat.
        if self.template.is_some() {
            p.landform = Landform::Flat;
            p.level = None;
            p.flat_height = p.sea_level as f32 + 1.0;
        }
        p
    }
}

/// A height map picked to make a map of, and how.
pub struct HeightmapPick {
    pub path: PathBuf,
    pub map: Arc<Heightmap>,
    pub how: FromHeightmap,
    pub picture: Option<TextureHandle>,
    /// How tall its settings were last laid out (see [`NewMap`]).
    rows: f32,
}

impl HeightmapPick {
    /// Read the height map at `path`.
    pub fn read(ctx: &egui::Context, path: PathBuf) -> Result<Self, String> {
        let map = massif_core::heightmap::read(&path).map_err(|e| e.to_string())?;
        let side = 256usize;
        let (w, h) = (side, (side * map.height / map.width.max(1)).clamp(16, 512));
        let pixels: Vec<u8> = (0..w * h)
            .map(|k| (map.sample((k % w) as f32 * map.width as f32 / w as f32, (k / w) as f32 * map.height as f32 / h as f32) * 255.0) as u8)
            .collect();
        let picture = ctx.load_texture("height map", egui::ColorImage::from_gray([w, h], &pixels), egui::TextureOptions::LINEAR);
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or("Height map").to_owned();
        Ok(Self {
            path,
            map: Arc::new(map),
            how: FromHeightmap {
                name,
                scale: 1.0,
                low: 20.0,
                high: 230.0,
                sea_level: 63,
                min_y: massif_core::generate::VANILLA_LIMITS.0,
                max_y: massif_core::generate::VANILLA_LIMITS.1,
            },
            picture: Some(picture),
            rows: 5.0 * (ROW_H + 14.0) - 14.0,
        })
    }
}

/// What the home screen asks the application to do.
#[derive(Debug, Clone)]
pub enum HomeAction {
    /// Make a new map; with a template, its relief made from it.
    Create(GenParams, Option<Template>),
    FromHeightmap(Arc<Heightmap>, FromHeightmap),
    PickHeightmap,
    /// Open a map of one's own (`.massif`).
    Open,
    /// Import a WorldPainter world.
    Import,
    OpenRecent(PathBuf),
    /// Take a map off the list of recent ones.
    Forget(PathBuf),
    Reveal(PathBuf),
    /// Back to the map that was open.
    Back,
    Settings,
    /// A file let go over the window.
    Dropped(PathBuf),
    /// Another picture behind the home screen.
    AnotherPicture,
}

/// The pictures of `assets/backdrops` (listed by the build script).
mod pictures {
    include!(concat!(env!("OUT_DIR"), "/backdrops.rs"));
}

/// Seconds each frame of the backdrop stays: its dither moves on, about once
/// a second, as bast.sh's does.
const FRAME_SECS: f64 = 0.9;

/// The picture behind the home screen, dithered for one size and theme: a
/// few frames of it, each dithered a little differently, shown in turn.
struct Backdrop {
    key: BackdropKey,
    /// None when the picture would not decode.
    frames: Vec<TextureHandle>,
}

/// What a backdrop is made for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct BackdropKey {
    /// The picture, and whether at dawn (the light theme's) or at night.
    index: usize,
    dawn: bool,
    cols: usize,
    rows: usize,
    dark: bool,
    high: bool,
    /// Where the words are, in cells.
    calm: [i32; 4],
    /// Pixels a cell.
    pitch: usize,
}

/// A picture of `assets/backdrops`, decoded.
struct Picture {
    index: usize,
    /// At dawn (the light theme's), or at night.
    dawn: bool,
    image: Arc<image::RgbImage>,
}

/// A backdrop made on a thread of its own: its picture (once decoded) and
/// its frames, none when the picture would not decode.
type MadeBackdrop = (Option<Picture>, Vec<egui::ColorImage>);

/// When maps were last changed, `None` for those that are not there.
type Ages = Vec<(PathBuf, Option<SystemTime>)>;

pub struct Home {
    pub page: Page,
    pub new_map: NewMap,
    pub heightmap: Option<HeightmapPick>,
    /// The name of the map open before, to go back to.
    pub back_to: Option<String>,
    backdrop: Option<Backdrop>,
    /// The picture of the backdrop, chosen at random when Massif opens
    /// (among those not shown yet).
    choice: usize,
    choice_set: bool,
    picture: Option<Picture>,
    /// The backdrop being made on a thread of its own.
    making: Option<(BackdropKey, Receiver<MadeBackdrop>)>,
    /// When each recent map was last changed (`None`: it is not there),
    /// read on a thread of its own: a map on a drive that is gone, or on a
    /// network share that does not answer, would hold up the window.
    ages: HashMap<PathBuf, Option<SystemTime>>,
    ages_asked: Option<Receiver<Ages>>,
    ages_at: f64,
    /// Make the pictures (the backdrop, the preview) and read the recent
    /// maps in the frame itself, as tests want; the app makes them on other
    /// threads, so that its first frame waits on none of it.
    pub in_frame: bool,
    /// Whether the picture's dither moves on (the settings).
    pub animate: bool,
    logo: Option<TextureHandle>,
    /// How tall what is written in the middle was last laid out: lifted
    /// when it would not fit under the top margin.
    content_height: f32,
}

impl Home {
    pub fn new(registry: massif_core::Registry) -> Self {
        Self {
            page: Page::NewMap,
            new_map: NewMap::new(registry),
            heightmap: None,
            back_to: None,
            backdrop: None,
            choice: (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize) / 7,
            choice_set: false,
            picture: None,
            making: None,
            ages: HashMap::new(),
            ages_asked: None,
            ages_at: f64::NEG_INFINITY,
            in_frame: cfg!(test),
            animate: true,
            logo: None,
            content_height: 0.0,
        }
    }

    /// Show a picture not shown yet (`seen`: the names shown, kept in the
    /// settings), so that pictures just added come first; once all were,
    /// any but the last one.
    pub fn pick_backdrop(&mut self, seen: &mut Vec<String>) {
        let all = pictures::BACKDROPS;
        if all.is_empty() {
            return;
        }
        seen.retain(|s| all.iter().any(|(n, _, _)| n == s));
        let mut pool: Vec<usize> = (0..all.len()).filter(|&k| !seen.iter().any(|s| s == all[k].0)).collect();
        if pool.is_empty() {
            let last = seen.last().cloned();
            seen.clear();
            pool = (0..all.len()).filter(|&k| Some(all[k].0) != last.as_deref()).collect();
        }
        let k = pool.get(self.choice % pool.len().max(1)).copied().unwrap_or(0);
        self.choice = k;
        self.choice_set = true;
        seen.push(all[k].0.to_owned());
        // A new draw for the next pick.
        self.choice = k + (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos()) as usize / 7) * all.len();
    }

    /// Show the picture `k` of `assets/backdrops` (by its place in the
    /// folder, in name order).
    #[cfg(test)]
    pub fn choose_backdrop(&mut self, k: usize) -> &'static str {
        self.choice = k;
        pictures::BACKDROPS[k % pictures::BACKDROPS.len()].0
    }

    /// The number of pictures there are.
    #[cfg(test)]
    pub fn backdrops() -> usize {
        pictures::BACKDROPS.len()
    }

    /// Show the home screen over the whole of `ui`. `recent`: the maps
    /// opened lately; `note`: a message to show, and whether it is an error.
    pub fn ui(&mut self, ui: &mut Ui, recent: &[PathBuf], note: Option<(&str, bool)>) -> Option<HomeAction> {
        let pal = Palette::of(ui);
        let full = ui.max_rect();
        let side_w = SIDEBAR.min(full.width() * 0.4);
        let sidebar = Rect::from_min_size(full.min, vec2(side_w, full.height()));
        let main = Rect::from_min_max(pos2(sidebar.max.x, full.min.y), full.max);
        let mut action = None;
        if self.logo.is_none() {
            self.logo = image::load_from_memory(include_bytes!("../assets/icon.png")).ok().map(|img| {
                let img = img.to_rgba8();
                let size = [img.width() as usize, img.height() as usize];
                ui.ctx()
                    .load_texture("massif logo", egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw()), egui::TextureOptions::LINEAR)
            });
        }

        crate::startup::frame_step(ui.ctx(), "the home screen: the picture behind it");
        self.paint_backdrop(ui, main, column_rect(main, self.content_height));
        ui.painter().rect_filled(sidebar, 0, sidebar_fill(&pal));
        ui.painter().vline(sidebar.max.x - 0.5, sidebar.y_range(), Stroke::new(1.0, pal.border));

        let mut side = ui.new_child(
            UiBuilder::new()
                .id_salt("home sidebar")
                .max_rect(sidebar.shrink2(vec2(12.0, 0.0)))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        crate::startup::frame_step(ui.ctx(), "the home screen: the side bar and the recent maps");
        self.sidebar(&mut side, recent, &mut action);
        crate::startup::frame_step(ui.ctx(), "the home screen: the new map's settings and preview");
        let mut centre = ui.new_child(UiBuilder::new().id_salt("home main").max_rect(main));
        self.main(&mut centre, main, &mut action);

        // Another picture, bottom right: quiet, frameless.
        if !pictures::BACKDROPS.is_empty() {
            let at = Rect::from_center_size(pos2(main.right() - 26.0, main.bottom() - 26.0), vec2(28.0, 28.0));
            let response = ui.interact(at, egui::Id::unique("home another picture"), Sense::click()).on_hover_text("Another picture");
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Another picture"));
            let color = if response.hovered() { pal.text } else { art_weak(&pal) };
            let glyph = ui.painter().layout_no_wrap(theme::glyph(Icon::Images).to_string(), FontId::proportional(15.0), color);
            paint_on_art(ui.painter(), at.center() - glyph.size() / 2.0, glyph, &pal, None);
            if response.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if response.clicked() {
                action = Some(HomeAction::AnotherPicture);
            }
        }

        // What happened, at the bottom.
        if let Some((text, error)) = note {
            let painter = ui.painter_at(main);
            let galley = painter.layout_no_wrap(text.to_owned(), FontId::proportional(12.5), pal.text);
            let back = Rect::from_center_size(pos2(main.center().x, main.bottom() - 34.0), galley.size() + vec2(28.0, 14.0));
            painter.rect(back, 10, theme::mix(sidebar_fill(&pal), pal.text, 0.04), Stroke::new(1.0, if error { pal.danger } else { pal.border }), StrokeKind::Inside);
            painter.galley(back.min + vec2(14.0, 7.0), galley, pal.text);
        }

        // Files let go over the window open, or make a map.
        let (hovering, dropped) = ui.ctx().input(|i| (!i.raw.hovered_files.is_empty(), i.raw.dropped_files.first().map(|f| f.path().to_path_buf())));
        if hovering {
            let painter = ui.painter_at(full);
            let zone = main.shrink(18.0);
            painter.rect(zone, 16, pal.accent.gamma_multiply(0.10), Stroke::new(1.5, pal.accent), StrokeKind::Inside);
            painter.text(zone.center(), Align2::CENTER_CENTER, "Drop it to open it", FontId::proportional(18.0), pal.text);
        }
        if let Some(path) = dropped {
            action = Some(HomeAction::Dropped(path));
        }
        if action.is_none()
            && self.back_to.is_some()
            && !ui.ctx().egui_wants_keyboard_input()
            && ui.input(|i| i.key_pressed(egui::Key::Escape))
        {
            action = Some(HomeAction::Back);
        }
        action
    }

    // ------------------------------------------------------------ sidebar --

    fn sidebar(&mut self, ui: &mut Ui, recent: &[PathBuf], action: &mut Option<HomeAction>) {
        let pal = Palette::of(ui);
        self.read_ages(ui.ctx(), recent);
        // Room for the window's buttons, on macOS.
        ui.add_space(if cfg!(target_os = "macos") { 44.0 } else { 16.0 });
        ui.horizontal(|ui| {
            ui.add_space(8.0);
            if let Some(logo) = &self.logo {
                ui.image((logo.id(), vec2(22.0, 22.0)));
            }
            ui.label(RichText::new("Massif").bold().size(15.0));
        });
        ui.add_space(16.0);
        if nav(ui, Icon::Plus, "New map", self.page == Page::NewMap).clicked() {
            self.page = Page::NewMap;
        }
        if nav(ui, Icon::ImageUp, "From a height map", self.page == Page::FromHeightmap).clicked() {
            self.page = Page::FromHeightmap;
        }
        if nav(ui, Icon::FolderOpen, "Open a map…", false).on_hover_text("A Massif map (.massif)").clicked() {
            *action = Some(HomeAction::Open);
        }
        if nav(ui, Icon::Import, "Import from WorldPainter…", false).on_hover_text("A WorldPainter world (.world)").clicked() {
            *action = Some(HomeAction::Import);
        }
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new("Recent maps").size(11.5).color(pal.weak));
        });
        ui.add_space(4.0);
        let bottom = if self.back_to.is_some() { 84.0 } else { 52.0 };
        let list = ui.available_height() - bottom;
        egui::ScrollArea::vertical()
            .id_salt("recent maps")
            .max_height(list.max(40.0))
            .auto_shrink([false, true])
            .show(ui, |ui| {
                if recent.is_empty() {
                    empty_state(ui);
                }
                for path in recent {
                    let row = recent_row(ui, path, self.ages.get(path).copied());
                    if row.clicked() {
                        *action = Some(HomeAction::OpenRecent(path.clone()));
                    }
                    row.context_menu(|ui| {
                        if ui.button(theme::with_icon(Icon::FolderSearch, "Show in Finder")).clicked() {
                            *action = Some(HomeAction::Reveal(path.clone()));
                        }
                        if ui.button(theme::with_icon(Icon::X, "Remove from the list")).clicked() {
                            *action = Some(HomeAction::Forget(path.clone()));
                        }
                    });
                }
            });
        ui.with_layout(egui::Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(12.0);
            if nav(ui, Icon::Settings, "Settings", false).on_hover_text("Appearance, accessibility, typeface, sounds… (⌘,)").clicked() {
                *action = Some(HomeAction::Settings);
            }
            if let Some(name) = &self.back_to
                && nav(ui, Icon::ArrowLeft, &format!("Back to {name}"), false).on_hover_text("Esc").clicked()
            {
                *action = Some(HomeAction::Back);
            }
        });
    }

    // --------------------------------------------------------------- main --

    fn main(&mut self, ui: &mut Ui, area: Rect, action: &mut Option<HomeAction>) {
        let pal = Palette::of(ui);
        let column = column_rect(area, self.content_height);
        let width = column.width();
        let mut ui = ui.new_child(UiBuilder::new().id_salt("home column").max_rect(column).layout(egui::Layout::top_down(Align::Center)));
        let ui = &mut ui;
        if let Some(logo) = &self.logo {
            ui.image((logo.id(), vec2(64.0, 64.0)));
        }
        ui.add_space(14.0);
        let (title, subtitle) = match self.page {
            Page::NewMap => ("What will you build?", "A new map: its size and its land, then make it."),
            Page::FromHeightmap => ("A map from a height map", "A greyscale picture from Gaea, World Machine or an image editor."),
        };
        art_label(ui, RichText::new(title).size(28.0).medium().color(pal.text));
        ui.add_space(2.0);
        art_label(ui, RichText::new(subtitle).size(13.0).color(art_weak(&pal)));
        ui.add_space(22.0);
        // Laid on the backdrop itself, not in a card.
        ui.allocate_ui_with_layout(vec2(width - 34.0, 0.0), egui::Layout::top_down(Align::Min), |ui| {
            ui.set_width(width - 34.0);
            match self.page {
                Page::NewMap => self.new_map_card(ui, action),
                Page::FromHeightmap => self.heightmap_card(ui, action),
            }
        });
        ui.add_space(14.0);
        let links: [(Icon, &str, &str, HomeAction); 3] = [
            (Icon::FolderOpen, "Open a map", "A Massif map (.massif)", HomeAction::Open),
            (Icon::Import, "Import from WorldPainter", "A WorldPainter world (.world)", HomeAction::Import),
            (Icon::ImageUp, "From a height map", "A greyscale picture: 16-bit PNG, JPEG, .r16 or .r32", HomeAction::PickHeightmap),
        ];
        if let Some(a) = link_row(ui, &links) {
            if matches!(a, HomeAction::PickHeightmap) {
                self.page = Page::FromHeightmap;
            }
            *action = Some(a);
        }
        ui.add_space(6.0);
        art_label(ui, RichText::new("or drop a .massif, a .world or a height map on the window").size(11.5).color(art_weak(&pal)));
        self.content_height = ui.min_rect().height();
    }

    fn new_map_card(&mut self, ui: &mut Ui, action: &mut Option<HomeAction>) {
        let pal = Palette::of(ui);
        let nm = &mut self.new_map;
        name_field(ui, &mut nm.params.name);
        // The settings on the left; beside them, the preview, as tall as
        // they are.
        let side = picture_side(ui.available_width(), nm.rows);
        let card = ui.available_rect_before_wrap();
        let left = if side > 0.0 { card.width() - side - GUTTER } else { card.width() };
        let mut rows: Option<Rect> = None;
        ui.vertical(|ui| {
            ui.set_width(left);
            let third = ((left - 76.0 - 12.0) / 3.0).floor();
            let mut row = |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
                let r = setting(ui, name, add);
                rows = Some(rows.map_or(r, |all| all.union(r)));
            };
            row(ui, "Size", &mut |ui| {
                input_group(
                    ui,
                    "home size",
                    &mut [Part::Int(&mut nm.width, 128..=65536, 64.0), Part::Text("×"), Part::Int(&mut nm.depth, 128..=65536, 64.0), Part::Text("blocks")],
                );
                // Whole tiles only.
                nm.width = (nm.width + 127) / 128 * 128;
                nm.depth = (nm.depth + 127) / 128 * 128;
            });
            row(ui, "Height", &mut |ui| limits(ui, &mut nm.params.min_y, &mut nm.params.max_y));
            row(ui, "Land", &mut |ui| {
                let shown = nm.template.map_or(nm.params.landform.label(), |t| t.label());
                crate::widgets::combo("home landform").selected_text(shown).width(third * 1.4).height(360.0).show_ui(ui, |ui| {
                    for l in Landform::ALL {
                        if ui.selectable_label(nm.template.is_none() && nm.params.landform == l, l.label()).clicked() {
                            nm.params.landform = l;
                            nm.template = None;
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new("From the terrain generator").size(11.0).color(pal.weak));
                    for t in Template::ALL.into_iter().filter(|t| !t.reads_the_map()) {
                        if ui.selectable_label(nm.template == Some(t), t.label()).on_hover_text(t.description()).clicked() {
                            nm.template = Some(t);
                        }
                    }
                });
                let mut automatic = nm.params.surface.is_none();
                let reg = &nm.registry;
                let shown = match nm.params.surface {
                    None => "Automatic surface".to_owned(),
                    Some(t) => reg.terrain(t).name.clone(),
                };
                let mut chosen = nm.params.surface;
                crate::widgets::combo("home surface").selected_text(shown).width(third * 1.4).height(320.0).show_ui(ui, |ui| {
                    if ui
                        .selectable_label(automatic, "Automatic surface")
                        .on_hover_text("Grass, sand, rock and snow by height and slope")
                        .clicked()
                    {
                        automatic = true;
                        chosen = None;
                    }
                    for (i, t) in reg.terrains.iter().enumerate() {
                        let id = massif_core::TerrainId(i as u16);
                        if ui.selectable_label(chosen == Some(id), &t.name).clicked() {
                            chosen = Some(id);
                        }
                    }
                });
                nm.params.surface = chosen;
            });
            let hilly = nm.params.landform != Landform::Flat && nm.template.is_none();
            let (bottom, top) = massif_core::generate::build_limits(nm.params.min_y, nm.params.max_y);
            // Taller worlds take taller mountains.
            let most = if top > 320 { 1500.0 } else { 400.0 };
            row(ui, "Relief", &mut |ui| {
                ui.add_enabled(hilly, Field::new(&mut nm.params.relief, 0.0..=most).text("Height").suffix("%").max_decimals(0).width(third));
                ui.add_enabled(hilly, Field::new(&mut nm.params.feature_size, 10.0..=1000.0).text("Features").suffix("%").max_decimals(0).width(third));
                let mut level = nm.params.level.unwrap_or(if hilly { nm.params.sea_level as f32 } else { nm.params.flat_height });
                let free = nm.template.is_none();
                if ui.add_enabled(free, Field::new(&mut level, (bottom + 1) as f32..=(top - 2) as f32).text("Ground").max_decimals(0).width(third)).changed() {
                    nm.params.level = Some(level);
                    nm.params.flat_height = level;
                }
            });
            row(ui, "Water", &mut |ui| {
                ui.add(Field::new(&mut nm.params.sea_level, bottom + 1..=top - 2).text("Level").width(third));
                let lava = nm.params.lava;
                match button_group(ui, "home fluid", &["Water", "Lava"], &[!lava, lava]) {
                    Some(0) => nm.params.lava = false,
                    Some(_) => nm.params.lava = true,
                    None => {}
                }
                if !nm.params.lava {
                    let beaches = nm.params.beaches && nm.params.surface.is_none();
                    if button_group(ui, "home beaches", &["Beaches"], &[beaches]).is_some() {
                        nm.params.beaches = !nm.params.beaches;
                    }
                }
            });
            row(ui, "Seed", &mut |ui| {
                let (_, dice) = input_group(ui, "home seed", &mut [Part::Edit(&mut nm.seed, third * 1.4), Part::Button(Icon::Dices, "Another seed")]);
                if dice.is_some() {
                    nm.seed = random_seed().to_string();
                }
            });
        });
        if let Some(rows) = rows {
            nm.rows = rows.height();
            let side = side.min(rows.height());
            if side > 0.0 {
                preview(ui, nm, Rect::from_min_size(pos2(card.right() - side, rows.top()), vec2(side, side)), self.in_frame);
            }
        }
        ui.add_space(4.0);
        hairline(ui);
        ui.add_space(10.0);
        let params = nm.params();
        ui.horizontal(|ui| {
            let tiles = params.tiles_x as i64 * params.tiles_z as i64;
            let minutes = (params.tiles_x.max(params.tiles_z) * 128) as f32 / 4.317 / 60.0;
            let across = if minutes < 1.0 { "under a minute across".to_owned() } else { format!("{minutes:.0} min. across on foot") };
            let (bottom, top) = massif_core::generate::build_limits(params.min_y, params.max_y);
            let pack = if (bottom, top) != massif_core::generate::VANILLA_LIMITS { " (with a data pack)" } else { "" };
            art_label(
                ui,
                RichText::new(format!(
                    "Minecraft Java 1.18+  ·  Y {bottom} to {top}{pack}  ·  {across}  ·  about {}",
                    crate::panels::human((tiles * 12_000) as u64)
                ))
                .size(11.5)
                .color(art_weak(&pal)),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let create = primary(ui, "Create map", "⏎").on_hover_text("Enter");
                let enter = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if create.clicked() || enter {
                    *action = Some(HomeAction::Create(params.clone(), nm.template));
                }
            });
        });
    }

    fn heightmap_card(&mut self, ui: &mut Ui, action: &mut Option<HomeAction>) {
        let pal = Palette::of(ui);
        let Some(pick) = &mut self.heightmap else {
            // Nothing picked yet: a place to pick one, or to drop it.
            let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 190.0), Sense::click());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Choose a height map"));
            let lit = response.hovered();
            dashed_rect(ui.painter(), rect.shrink(1.0), if lit { pal.accent } else { theme::mix(pal.border, pal.text, if pal.high { 0.5 } else { 0.25 }) });
            if lit {
                ui.painter().rect_filled(rect.shrink(2.0), 12, pal.accent.gamma_multiply(0.06));
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let c = rect.center();
            let painter = ui.painter();
            let line = |text: String, size: f32, ink: Color32, y: f32| {
                let g = painter.layout_no_wrap(text, FontId::proportional(size), ink);
                paint_on_art(painter, pos2(c.x - g.size().x / 2.0, c.y + y - g.size().y / 2.0), g, &pal, None);
            };
            line(theme::glyph(Icon::ImageUp).to_string(), 28.0, if lit { pal.accent } else { art_weak(&pal) }, -28.0);
            line("Choose a height map".to_owned(), 15.0, pal.text, 10.0);
            line("16-bit PNG, JPEG, .r16 or .r32 — or drop one here".to_owned(), 12.0, art_weak(&pal), 32.0);
            if response.clicked() {
                *action = Some(HomeAction::PickHeightmap);
            }
            return;
        };
        name_field(ui, &mut pick.how.name);
        let side = picture_side(ui.available_width(), pick.rows);
        let card = ui.available_rect_before_wrap();
        let left = if side > 0.0 { card.width() - side - GUTTER } else { card.width() };
        let mut rows: Option<Rect> = None;
        ui.vertical(|ui| {
            ui.set_width(left);
            let w = ((left - 76.0 - 8.0) / 2.0).floor();
            let mut row = |ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)| {
                let r = setting(ui, name, add);
                rows = Some(rows.map_or(r, |all| all.union(r)));
            };
            let h = &mut pick.how;
            let file = pick.path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            let file = shorten(&file, 34);
            row(ui, "File", &mut |ui| {
                let (_, another) = input_group(ui, "height map file", &mut [Part::Text(&file), Part::Button(Icon::FolderOpen, "Choose another height map")]);
                if another.is_some() {
                    *action = Some(HomeAction::PickHeightmap);
                }
            });
            row(ui, "Height", &mut |ui| limits(ui, &mut h.min_y, &mut h.max_y));
            let (bottom, top) = massif_core::generate::build_limits(h.min_y, h.max_y);
            row(ui, "Scale", &mut |ui| {
                ui.add(Field::new(&mut h.scale, 0.25..=16.0).logarithmic(true).text("Blocks a pixel").max_decimals(2).width(w * 2.0 + 6.0));
            });
            row(ui, "Heights", &mut |ui| {
                ui.add(Field::new(&mut h.low, bottom as f32..=(top - 1) as f32).text("Black is Y").max_decimals(0).width(w));
                ui.add(Field::new(&mut h.high, bottom as f32..=(top - 1) as f32).text("White is Y").max_decimals(0).width(w));
            });
            row(ui, "Water", &mut |ui| {
                ui.add(Field::new(&mut h.sea_level, bottom..=top - 1).text("Level").width(w));
            });
        });
        if let Some(rows) = rows {
            pick.rows = rows.height();
            let side = side.min(rows.height());
            if side > 0.0
                && let Some(t) = &pick.picture
            {
                let rect = Rect::from_min_size(pos2(card.right() - side, rows.top()), vec2(side, side));
                let size = t.size_vec2();
                let k = (side / size.x).min(side / size.y);
                picture_tile(ui, rect, t.id(), size * k, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)));
                ui.interact(rect, egui::Id::unique("home height map picture"), Sense::hover())
                    .on_hover_text(format!("{}\n{} × {} pixels", pick.path.display(), pick.map.width, pick.map.height));
            }
        }
        ui.add_space(4.0);
        hairline(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (w, d) = (pick.map.width as f32 * pick.how.scale, pick.map.height as f32 * pick.how.scale);
            art_label(
                ui,
                RichText::new(format!("{} × {} pixels  →  {w:.0} × {d:.0} blocks, Y {:.0} to {:.0}", pick.map.width, pick.map.height, pick.how.low, pick.how.high))
                    .size(11.5)
                    .color(art_weak(&pal)),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let create = primary(ui, "Create map", "⏎").on_hover_text("Enter");
                let enter = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if create.clicked() || enter {
                    *action = Some(HomeAction::FromHeightmap(pick.map.clone(), pick.how.clone()));
                }
            });
        });
    }

    // ----------------------------------------------------------- backdrop --

    /// The picture behind `rect`: one of `assets/backdrops`, dithered in
    /// its own palette and dimmed where the settings are written (`calm`),
    /// its dither moving on every [`FRAME_SECS`]. Made again, on a thread of
    /// its own, when the window grows past it or the theme changes; the one
    /// before stays until then (none at first: the page's colour).
    fn paint_backdrop(&mut self, ui: &Ui, rect: Rect, calm: Rect) {
        let pal = Palette::of(ui);
        let bg = backdrop_fill(&pal);
        ui.painter().rect_filled(rect, 0, bg);
        let all = pictures::BACKDROPS;
        if all.is_empty() {
            return;
        }
        let index = self.choice % all.len();
        // The light theme shows the scene at dawn, where there is one.
        let dawn = !pal.dark && all[index].2.is_some();
        let ppp = ui.ctx().pixels_per_point();
        let pitch = ((2.0 * ppp).round() as usize).max(3);
        let cell = pitch as f32 / ppp;
        // Made a little larger than needed, so that resizing the window
        // does not make it again at every step.
        let bucket = |n: usize| n.div_ceil(32) * 32;
        let cols = bucket(((rect.width() * ppp) as usize / pitch).max(1));
        let rows = bucket(((rect.height() * ppp) as usize / pitch).max(1));
        // Anchored to the bottom left: the land stays, the sky gives way.
        let placed = |cols: usize, rows: usize| {
            let size = vec2(cols as f32, rows as f32) * cell;
            Rect::from_min_size(pos2(rect.left(), rect.bottom() - size.y), size)
        };
        let shown = placed(cols, rows);
        let in_cells = |p: egui::Pos2| [((p.x - shown.left()) / cell) as i32, ((p.y - shown.top()) / cell) as i32];
        let ([x0, y0], [x1, y1]) = (in_cells(calm.min), in_cells(calm.max));
        let calm = [x0, y0, x1, y1].map(|v| v.div_euclid(8) * 8);
        let key = BackdropKey { index, dawn, cols, rows, dark: pal.dark, high: pal.high, calm, pitch };
        // What a thread made.
        if let Some((made_for, rx)) = &self.making {
            match rx.try_recv() {
                Ok(made) => {
                    let made_for = *made_for;
                    self.making = None;
                    self.take_backdrop(ui.ctx(), made_for, made);
                }
                Err(TryRecvError::Disconnected) => self.making = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        // One at a time: while the window is resized, the next is made for
        // its size once the one on its way is done.
        if self.backdrop.as_ref().is_none_or(|b| b.key != key) && self.making.is_none() {
            let decoded = self.picture.as_ref().filter(|p| p.index == index && p.dawn == dawn).map(|p| p.image.clone());
            let bytes = if dawn { all[index].2.unwrap_or(all[index].1) } else { all[index].1 };
            let light = if dawn { Light::Dawn } else if pal.dark { Light::Night } else { Light::Faded };
            let high = pal.high;
            let make = move || -> MadeBackdrop {
                let image = decoded.or_else(|| image::load_from_memory(bytes).ok().map(|img| Arc::new(img.to_rgb8())));
                let Some(image) = image else { return (None, Vec::new()) };
                let frames = dither_frames(&image, cols, rows, light, bg, calm, high);
                (Some(Picture { index, dawn, image }), frames)
            };
            if self.in_frame {
                self.take_backdrop(ui.ctx(), key, make());
            } else {
                let (tx, rx) = channel();
                let ctx = ui.ctx().clone();
                let spawned = std::thread::Builder::new().name("massif home picture".into()).spawn(move || {
                    let _ = tx.send(make());
                    ctx.request_repaint();
                });
                if spawned.is_ok() {
                    self.making = Some((key, rx));
                }
            }
        }
        if let Some(b) = &self.backdrop
            && !b.frames.is_empty()
        {
            let t = ui.input(|i| i.time);
            let k = if self.animate { (t / FRAME_SECS) as usize % b.frames.len() } else { 0 };
            // Where it was made for, until the one for this size comes.
            let at = if b.key.pitch == pitch { placed(b.key.cols, b.key.rows) } else { shown };
            ui.painter_at(rect).image(b.frames[k].id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            if self.animate {
                ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(FRAME_SECS - t % FRAME_SECS + 0.01));
            }
        }
    }

    /// A backdrop made for `key`: its frames sent to the graphics card.
    fn take_backdrop(&mut self, ctx: &egui::Context, key: BackdropKey, (picture, frames): MadeBackdrop) {
        if picture.is_some() {
            self.picture = picture;
        }
        let frames = frames.into_iter().enumerate().map(|(k, image)| ctx.load_texture(format!("home backdrop {k}"), image, egui::TextureOptions::NEAREST)).collect();
        self.backdrop = Some(Backdrop { key, frames });
    }

    /// When the recent maps were last changed, read again every few
    /// seconds on a thread of its own.
    fn read_ages(&mut self, ctx: &egui::Context, recent: &[PathBuf]) {
        let now = ctx.input(|i| i.time);
        if let Some(rx) = &self.ages_asked {
            match rx.try_recv() {
                Ok(ages) => {
                    self.ages_asked = None;
                    self.ages = ages.into_iter().collect();
                }
                Err(TryRecvError::Disconnected) => self.ages_asked = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let unknown = recent.iter().any(|p| !self.ages.contains_key(p));
        if self.ages_asked.is_some() || recent.is_empty() || !(unknown || now - self.ages_at >= 10.0) {
            return;
        }
        self.ages_at = now;
        let paths = recent.to_vec();
        let read = move || paths.into_iter().map(|p| {
            let modified = std::fs::metadata(&p).and_then(|m| m.modified()).ok();
            (p, modified)
        });
        if self.in_frame {
            self.ages = read().collect();
            return;
        }
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("massif recent maps".into()).spawn(move || {
            let _ = tx.send(read().collect());
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.ages_asked = Some(rx);
        }
    }
}

/// Where what is written in the middle of `area` goes: `content` tall (as
/// last laid out), lifted when it would not fit under the top margin.
fn column_rect(area: Rect, content: f32) -> Rect {
    let width = (area.width() - 96.0).clamp(420.0, 780.0);
    let mut top = (area.height() * 0.12).max(44.0);
    if content > 0.0 {
        top = top.min(area.height() - 16.0 - content).max(16.0);
    }
    Rect::from_min_max(pos2(area.center().x - width / 2.0, area.top() + top), pos2(area.center().x + width / 2.0, area.bottom() - 16.0))
}

/// The ink of muted words written on the picture: stronger than muted text
/// elsewhere, as the picture shows around them.
fn art_weak(pal: &Palette) -> Color32 {
    if pal.high { pal.weak } else { theme::mix(pal.weak, pal.text, 0.4) }
}

/// `galley` at `at`, written straight on the picture: a halo of the page's
/// colour around its letters, so that it reads over any part of the picture
/// — in `ink`, or in its own colours.
fn paint_on_art(painter: &egui::Painter, at: egui::Pos2, galley: Arc<egui::Galley>, pal: &Palette, ink: Option<Color32>) {
    let halo = backdrop_fill(pal);
    let rings: [(f32, f32); 2] = if pal.high { [(2.2, 0.55), (1.2, 0.95)] } else { [(2.0, 0.16), (1.1, 0.5)] };
    for (r, alpha) in rings {
        for k in 0..12 {
            let a = k as f32 * std::f32::consts::TAU / 12.0;
            painter.galley_with_override_text_color(at + vec2(a.cos(), a.sin()) * r, galley.clone(), halo.gamma_multiply(alpha));
        }
    }
    match ink {
        Some(ink) => painter.galley_with_override_text_color(at, galley, ink),
        None => painter.galley(at, galley, pal.text),
    }
}

/// Words written straight on the picture (see [`paint_on_art`]).
fn art_label(ui: &mut Ui, text: RichText) -> Response {
    let pal = Palette::of(ui);
    let words = text.text().to_owned();
    let galley = egui::WidgetText::from(text).into_galley(ui, Some(egui::TextWrapMode::Extend), f32::INFINITY, egui::TextStyle::Body);
    let (rect, response) = ui.allocate_exact_size(galley.size(), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, &words));
    paint_on_art(ui.painter(), rect.min, galley, &pal, None);
    response
}

/// The name of the new map: large, frameless, a hairline under it across
/// the card.
fn name_field(ui: &mut Ui, name: &mut String) {
    let pal = Palette::of(ui);
    ui.add(
        egui::TextEdit::singleline(name)
            .hint_text(RichText::new("Name your map").color(art_weak(&pal)))
            .font(FontId::proportional(17.0))
            .frame(egui::Frame::NONE)
            .desired_width(f32::INFINITY),
    );
    ui.add_space(8.0);
    hairline(ui);
    ui.add_space(14.0);
}

/// A hairline across `ui`, where its cursor is.
fn hairline(ui: &mut Ui) {
    let pal = Palette::of(ui);
    let r = ui.available_rect_before_wrap();
    let ink = if pal.high { pal.border } else { theme::mix(pal.border, pal.text, 0.12) };
    ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, ink));
}

/// The side of the picture beside the settings, in a card `width` wide: as
/// tall as the settings, less when they would not fit beside it, none on a
/// narrow window.
fn picture_side(width: f32, rows: f32) -> f32 {
    let side = rows.min(width - GUTTER - MIN_SETTINGS).floor();
    if side < 120.0 { 0.0 } else { side }
}

/// A picture beside the settings: a tile of the well's colour with a line
/// inside its edge, the picture in its middle at `shown` size (its
/// proportions kept), `uv` of its texture.
fn picture_tile(ui: &Ui, rect: Rect, texture: egui::TextureId, shown: egui::Vec2, uv: Rect) {
    let pal = Palette::of(ui);
    let r = theme::CARD_RADIUS;
    let painter = ui.painter();
    painter.rect_filled(rect, r, pal.well());
    let image = Rect::from_center_size(rect.center(), shown);
    painter.add(egui::epaint::RectShape::filled(image, r, Color32::WHITE).with_texture(texture, uv));
    let edge = if pal.high { pal.control_border() } else if pal.dark { Color32::from_white_alpha(26) } else { Color32::from_black_alpha(34) };
    painter.rect_stroke(rect, r, Stroke::new(1.0, edge), StrokeKind::Inside);
}

/// `s`, cut to `n` characters with an ellipsis in its middle.
fn shorten(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count <= n {
        return s.to_owned();
    }
    let head: String = s.chars().take(n / 2).collect();
    let tail: String = s.chars().skip(count - (n - n / 2 - 1)).collect();
    format!("{head}…{tail}")
}

/// `img` over `cols` × `rows` cells, in frames: covered (its lower part
/// kept), toned for the theme — and darker (or paler) within `calm` and
/// around it, where text is written —, then reduced to a palette of its own
/// sixteen colours with an ordered dither, a little shifted from frame to
/// frame.
/// How a picture is laid behind the home screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Light {
    /// A night scene in the dark theme: night falling from the top.
    Night,
    /// A scene at dawn in the light theme: as it is, calmer under the words.
    Dawn,
    /// A night scene with no dawn, in the light theme: faded into the page.
    Faded,
}

fn dither_frames(img: &image::RgbImage, cols: usize, rows: usize, light: Light, bg: Color32, calm: [i32; 4], high: bool) -> Vec<egui::ColorImage> {
    let dark = light == Light::Night;
    // How much of the picture is taken away under the words: more in high
    // contrast.
    let hush = |standard: f32| if high { 0.9 } else { standard };
    use rayon::prelude::*;
    let (iw, ih) = (img.width() as f32, img.height() as f32);
    let scale = (cols as f32 / iw).max(rows as f32 / ih);
    let (cw, ch) = (((cols as f32 / scale).round() as u32).clamp(1, img.width()), ((rows as f32 / scale).round() as u32).clamp(1, img.height()));
    let (x0, y0) = ((img.width() - cw) / 2, img.height() - ch);
    let crop = image::imageops::crop_imm(img, x0, y0, cw, ch).to_image();
    let small = image::imageops::resize(&crop, cols as u32, rows as u32, image::imageops::FilterType::Triangle);
    let back = [bg.r(), bg.g(), bg.b()].map(|v| v as f32 / 255.0);
    // Where the text is: a soft oval over the upper part of the column, the
    // picture dimmed under it and less and less around it — no edge.
    let [cx0, cy0, cx1, cy1] = calm.map(|v| v as f32);
    let (ex, ey) = ((cx0 + cx1) / 2.0, cy0 + 0.4 * (cy1 - cy0));
    let (rx, ry) = (0.78 * (cx1 - cx0).max(1.0), 0.55 * (cy1 - cy0).max(1.0));
    let pixels: Vec<[f32; 3]> = (0..rows * cols)
        .into_par_iter()
        .map(|i| {
            let (x, y) = ((i % cols) as f32, (i / cols) as f32);
            let c = small.get_pixel(x as u32, y as u32).0.map(|v| v as f32 / 255.0);
            // A little more colour, as a dither of few colours loses some.
            let lum = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            let c = c.map(|v| (lum + (v - lum) * 1.25).clamp(0.0, 1.0));
            let near = (-(((x - ex) / rx).powi(2) + ((y - ey) / ry).powi(2)) * 1.1).exp();
            match light {
                // Night falls from the top: the sky darker than the land.
                Light::Night => {
                    let k = (0.46 + 0.42 * smooth(0.0, 0.8, y / rows as f32)) * (1.0 - hush(0.72) * near);
                    c.map(|v| v.powf(1.15) * k)
                }
                // Dawn: the picture as it is, quieter where the words are.
                Light::Dawn => {
                    let k = 0.94 * (1.0 - hush(0.62) * near);
                    [0, 1, 2].map(|j| back[j] + (c[j] - back[j]) * k)
                }
                Light::Faded => {
                    let k = 0.42 * (1.0 - hush(0.7) * near);
                    [0, 1, 2].map(|j| back[j] + (c[j] - back[j]) * k)
                }
            }
        })
        .collect();
    let palette = median_cut(&pixels, 16);
    const BAYER: [[u8; 8]; 8] = [
        [0, 32, 8, 40, 2, 34, 10, 42],
        [48, 16, 56, 24, 50, 18, 58, 26],
        [12, 44, 4, 36, 14, 46, 6, 38],
        [60, 28, 52, 20, 62, 30, 54, 22],
        [3, 35, 11, 43, 1, 33, 9, 41],
        [51, 19, 59, 27, 49, 17, 57, 25],
        [15, 47, 7, 39, 13, 45, 5, 37],
        [63, 31, 55, 23, 61, 29, 53, 21],
    ];
    let spread = if dark { 0.13 } else { 0.1 };
    [(0usize, 0usize), (4, 4), (2, 6), (6, 2)]
        .par_iter()
        .map(|&(ox, oy)| {
            let out = pixels
                .iter()
                .enumerate()
                .map(|(i, c)| {
                    let (x, y) = (i % cols, i / cols);
                    let t = (BAYER[(y + oy) % 8][(x + ox) % 8] as f32 + 0.5) / 64.0 - 0.5;
                    let v = c.map(|ch| ch + t * spread);
                    let best = palette
                        .iter()
                        .min_by(|a, b| {
                            let d = |p: &[f32; 3]| (p[0] - v[0]).powi(2) + (p[1] - v[1]).powi(2) + (p[2] - v[2]).powi(2);
                            d(a).total_cmp(&d(b))
                        })
                        .copied()
                        .unwrap_or(back);
                    let [r, g, b] = best.map(|ch| (ch.clamp(0.0, 1.0) * 255.0).round() as u8);
                    Color32::from_rgb(r, g, b)
                })
                .collect();
            egui::ColorImage::new([cols, rows], out)
        })
        .collect()
}

/// `n` colours that stand for `pixels` (median cut): the set split again
/// and again where its colours spread the most.
fn median_cut(pixels: &[[f32; 3]], n: usize) -> Vec<[f32; 3]> {
    let step = (pixels.len() / 40_000).max(1);
    let mut boxes: Vec<Vec<[f32; 3]>> = vec![pixels.iter().step_by(step).copied().collect()];
    let widest = |b: &[[f32; 3]]| -> (usize, f32) {
        (0..3)
            .map(|axis| {
                let (lo, hi) = b.iter().fold((f32::MAX, f32::MIN), |(lo, hi), p| (lo.min(p[axis]), hi.max(p[axis])));
                (axis, hi - lo)
            })
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .unwrap_or((0, 0.0))
    };
    while boxes.len() < n {
        let Some((k, axis)) = boxes
            .iter()
            .enumerate()
            .filter(|(_, b)| b.len() > 1)
            .map(|(k, b)| {
                let (axis, range) = widest(b);
                (k, axis, range * (b.len() as f32).sqrt())
            })
            .max_by(|a, b| a.2.total_cmp(&b.2))
            .map(|(k, axis, _)| (k, axis))
        else {
            break;
        };
        let mut b = boxes.swap_remove(k);
        b.sort_by(|p, q| p[axis].total_cmp(&q[axis]));
        let upper = b.split_off(b.len() / 2);
        boxes.push(b);
        boxes.push(upper);
    }
    boxes
        .iter()
        .filter(|b| !b.is_empty())
        .map(|b| {
            let sum = b.iter().fold([0.0f32; 3], |s, p| [s[0] + p[0], s[1] + p[1], s[2] + p[2]]);
            sum.map(|v| v / b.len() as f32)
        })
        .collect()
}

// ---------------------------------------------------------------- parts --

fn sidebar_fill(pal: &Palette) -> Color32 {
    if pal.dark { Color32::from_rgb(14, 15, 17) } else { Color32::from_rgb(241, 241, 243) }
}

fn backdrop_fill(pal: &Palette) -> Color32 {
    if pal.dark { Color32::from_rgb(8, 8, 9) } else { Color32::from_rgb(248, 248, 249) }
}

/// An entry of the sidebar.
fn nav(ui: &mut Ui, icon: Icon, label: &str, selected: bool) -> Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    if selected || response.hovered() {
        let fill = if selected { theme::mix(sidebar_fill(&pal), pal.text, 0.08) } else { theme::mix(sidebar_fill(&pal), pal.text, 0.05) };
        ui.painter().rect_filled(rect, 8, fill);
    }
    let ink = if selected { pal.text } else { theme::mix(pal.text, pal.weak, 0.35) };
    ui.painter().text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        theme::glyph(icon).to_string(),
        FontId::proportional(14.0),
        if selected { pal.accent } else { pal.weak },
    );
    ui.painter().text(rect.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.0), ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// A map of the list of recent ones: its name, and how long ago it changed.
/// A recent map, `modified` when it was last changed as read so far
/// (`Some(None)`: it is not there).
fn recent_row(ui: &mut Ui, path: &Path, modified: Option<Option<SystemTime>>) -> Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &stem));
    if response.hovered() {
        ui.painter().rect_filled(rect, 8, theme::mix(sidebar_fill(&pal), pal.text, 0.05));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let (age, ink) = match modified {
        Some(Some(t)) => (ago(t), theme::mix(pal.text, pal.weak, 0.2)),
        Some(None) => ("missing".to_owned(), pal.weak),
        None => (String::new(), theme::mix(pal.text, pal.weak, 0.2)),
    };
    let world = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("world"));
    ui.painter().text(
        rect.left_center() + vec2(12.0, 0.0),
        Align2::LEFT_CENTER,
        theme::glyph(if world { Icon::Import } else { Icon::Map }).to_string(),
        FontId::proportional(12.5),
        pal.weak,
    );
    let font = FontId::proportional(13.0);
    let room = rect.width() - 36.0 - 44.0;
    let mut name = stem;
    while name.chars().count() > 4 && ui.painter().layout_no_wrap(name.clone(), font.clone(), ink).size().x > room {
        name.pop();
        while name.ends_with('…') || name.ends_with(' ') {
            name.pop();
        }
        name.push('…');
    }
    ui.painter().text(rect.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, name, font, ink);
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, age, FontId::proportional(11.5), pal.weak);
    response.on_hover_text(path.display().to_string())
}

/// What the list of recent maps shows while it is empty.
fn empty_state(ui: &mut Ui) {
    let pal = Palette::of(ui);
    ui.add_space(14.0);
    ui.vertical_centered(|ui| {
        let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
        ui.painter().circle(rect.center(), 21.0, theme::mix(sidebar_fill(&pal), pal.text, 0.05), Stroke::new(1.0, pal.border));
        ui.painter().text(rect.center(), Align2::CENTER_CENTER, theme::glyph(Icon::Map).to_string(), FontId::proportional(17.0), pal.weak);
        ui.add_space(8.0);
        ui.label(RichText::new("No maps yet").medium().size(13.0));
        ui.add_space(2.0);
        ui.label(RichText::new("The maps you make or open show up here.").size(11.5).color(pal.weak));
    });
}

/// How long ago `t` was, shortly: 5m, 3h, 2d, 1w, 4mo, 1y.
fn ago(t: std::time::SystemTime) -> String {
    let s = std::time::SystemTime::now().duration_since(t).map_or(0, |d| d.as_secs());
    match s {
        0..60 => "now".into(),
        60..3_600 => format!("{}m", s / 60),
        3_600..86_400 => format!("{}h", s / 3_600),
        86_400..604_800 => format!("{}d", s / 86_400),
        604_800..2_592_000 => format!("{}w", s / 604_800),
        2_592_000..31_536_000 => format!("{}mo", s / 2_592_000),
        _ => format!("{}y", s / 31_536_000),
    }
}

/// A setting of the new map: its name on the left (written on the
/// picture), its controls after. Where the row is.
fn setting(ui: &mut Ui, name: &str, add: &mut dyn FnMut(&mut Ui)) -> Rect {
    let pal = Palette::of(ui);
    let rect = ui
        .horizontal(|ui| {
            ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
            let (rect, response) = ui.allocate_exact_size(vec2(70.0, ROW_H), Sense::hover());
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, name));
            let g = ui.painter().layout_no_wrap(name.to_owned(), FontId::proportional(12.5), theme::mix(art_weak(&pal), pal.text, 0.3));
            paint_on_art(ui.painter(), pos2(rect.left(), rect.center().y - g.size().y / 2.0), g, &pal, None);
            add(ui);
        })
        .response
        .rect;
    ui.add_space(8.0);
    rect
}

/// The build limits of a new map: the top among Minecraft's own and taller
/// ones, the bottom as deep as it may go.
fn limits(ui: &mut Ui, min_y: &mut i32, max_y: &mut i32) {
    const TOPS: [i32; 4] = [320, 512, 1024, 2032];
    let on = TOPS.map(|t| *max_y == t);
    if let Some(k) = button_group(ui, "build limits", &["320", "512", "1024", "2032"], &on) {
        *max_y = TOPS[k];
    }
    let mut bottom = *min_y;
    input_group(ui, "build bottom", &mut [Part::Text("from Y"), Part::Int(&mut bottom, -2032..=-64, 16.0)]);
    let (b, t) = massif_core::generate::build_limits(bottom, *max_y);
    *min_y = b;
    *max_y = t;
}

/// The main button: filled with the accent colour, its key after it.
fn primary(ui: &mut Ui, text: &str, key: &str) -> Response {
    let accent = theme::accent();
    // Dark ink on light accents, white on dark ones.
    let light = 0.2126 * accent.r() as f32 + 0.7152 * accent.g() as f32 + 0.0722 * accent.b() as f32 > 150.0;
    let ink = if light { Color32::from_rgb(18, 14, 10) } else { Color32::WHITE };
    let label = ui.painter().layout_no_wrap(text.to_owned(), FontId::proportional(13.5), ink);
    let hint = ui.painter().layout_no_wrap(key.to_owned(), FontId::proportional(12.0), ink.gamma_multiply(0.6));
    let (rect, response) = ui.allocate_exact_size(vec2(label.size().x + hint.size().x + 42.0, 34.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.hovered() { theme::mix(accent, Color32::WHITE, 0.12) } else { accent };
    ui.painter().rect_filled(rect, theme::PILL, fill);
    let y = rect.center().y;
    ui.painter().galley(pos2(rect.min.x + 18.0, y - label.size().y / 2.0), label, ink);
    ui.painter().galley(pos2(rect.max.x - 16.0 - hint.size().x, y - hint.size().y / 2.0), hint, ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// Links in a row, centred: an icon and a name each, written on the
/// picture; hovered, the name underlined and what it opens said.
fn link_row(ui: &mut Ui, links: &[(Icon, &str, &str, HomeAction)]) -> Option<HomeAction> {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let (gap, icon_gap) = (22.0, 6.0);
    let parts: Vec<_> = links
        .iter()
        .map(|(icon, name, _, _)| {
            (
                ui.painter().layout_no_wrap(theme::glyph(*icon).to_string(), font.clone(), pal.weak),
                ui.painter().layout_no_wrap((*name).to_owned(), font.clone(), pal.text),
            )
        })
        .collect();
    let widths: Vec<f32> = parts.iter().map(|(i, n)| i.size().x + icon_gap + n.size().x).collect();
    let total = widths.iter().sum::<f32>() + gap * (links.len() - 1) as f32;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let mut x = row.center().x - total / 2.0;
    let mut chosen = None;
    for (k, (((_, name, tip, act), (icon, label)), w)) in links.iter().zip(parts).zip(widths).enumerate() {
        let h = icon.size().y.max(label.size().y);
        let rect = Rect::from_min_size(pos2(x, row.center().y - h / 2.0), vec2(w, h));
        let response = ui.interact(rect.expand(4.0), egui::Id::unique(("home link", k)), Sense::click()).on_hover_text(*tip);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, *name));
        let hovered = response.hovered();
        let painter = ui.painter();
        let icon_at = pos2(x, row.center().y - icon.size().y / 2.0);
        let at = pos2(x + icon.size().x + icon_gap, row.center().y - label.size().y / 2.0);
        let label_size = label.size();
        paint_on_art(painter, icon_at, icon, &pal, Some(if hovered { pal.accent } else { art_weak(&pal) }));
        paint_on_art(painter, at, label, &pal, Some(pal.text));
        if hovered || pal.high {
            let y = (at.y + label_size.y - 1.5).round() + 0.5;
            painter.hline(at.x..=at.x + label_size.x, y, Stroke::new(3.0, backdrop_fill(&pal).gamma_multiply(0.6)));
            painter.hline(at.x..=at.x + label_size.x, y, Stroke::new(1.0, pal.text));
        }
        if hovered {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if response.clicked() {
            chosen = Some(act.clone());
        }
        x += w + gap;
    }
    chosen
}

/// A rectangle with rounded corners drawn in dashes.
fn dashed_rect(painter: &egui::Painter, rect: Rect, color: Color32) {
    let r = 12.0;
    let mut points = Vec::new();
    let corners = [
        (pos2(rect.max.x - r, rect.min.y + r), -90.0f32),
        (pos2(rect.max.x - r, rect.max.y - r), 0.0),
        (pos2(rect.min.x + r, rect.max.y - r), 90.0),
        (pos2(rect.min.x + r, rect.min.y + r), 180.0),
    ];
    for (c, start) in corners {
        for k in 0..=8 {
            let a = (start + k as f32 * 90.0 / 8.0).to_radians();
            points.push(c + vec2(a.cos(), a.sin()) * r);
        }
    }
    points.push(points[0]);
    painter.add(egui::Shape::dashed_line(&points, Stroke::new(1.3, color), 6.0, 5.0));
}

/// The preview of a new map in `rect`: its colours and relief from above,
/// the whole map (not square ones keep their proportions), a cross at its
/// centre. Drawn on a thread of its own (in the frame when `in_frame`): the
/// one before stays until it comes.
fn preview(ui: &mut Ui, nm: &mut NewMap, rect: Rect, in_frame: bool) {
    if let Some(t) = nm.template {
        template_preview(ui, nm, t, rect);
        return;
    }
    let params = nm.params();
    let key = format!("{params:?}");
    if let Some((drawn, rx)) = &nm.previewing {
        match rx.try_recv() {
            Ok(image) => {
                let texture = ui.ctx().load_texture("new map preview", image, egui::TextureOptions::LINEAR);
                nm.preview = Some((drawn.clone(), texture));
                nm.previewing = None;
            }
            Err(TryRecvError::Disconnected) => nm.previewing = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    // One at a time: while a setting is dragged, the next is drawn for it
    // once the one on its way is done.
    if nm.preview.as_ref().is_none_or(|(k, _)| *k != key) && nm.previewing.is_none() {
        if in_frame {
            let image = preview_image(&params, &nm.registry, 200);
            nm.preview = Some((key, ui.ctx().load_texture("new map preview", image, egui::TextureOptions::LINEAR)));
        } else {
            let (tx, rx) = channel();
            let reg = nm.registry.clone();
            let ctx = ui.ctx().clone();
            let spawned = std::thread::Builder::new().name("massif home preview".into()).spawn(move || {
                let _ = tx.send(preview_image(&params, &reg, 200));
                ctx.request_repaint();
            });
            if spawned.is_ok() {
                nm.previewing = Some((key, rx));
            }
        }
    }
    match &nm.preview {
        Some((_, texture)) => {
            let (w, d) = (nm.width as f32, nm.depth as f32);
            let k = rect.width() / w.max(d);
            let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(w / w.max(d), d / w.max(d)));
            picture_tile(ui, rect, texture.id(), vec2(w * k, d * k), uv);
        }
        None => {
            let pal = Palette::of(ui);
            ui.painter().rect(rect, theme::CARD_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            ui.put(Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), egui::Spinner::new());
        }
    }
    // The centre: a cross, outlined to show on snow as on water.
    let c = rect.center();
    for (width, color) in [(3.0, Color32::from_black_alpha(70)), (1.2, Color32::WHITE)] {
        ui.painter().line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], Stroke::new(width, color));
        ui.painter().line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(width, color));
    }
    ui.interact(rect, egui::Id::unique("home map preview"), Sense::hover()).on_hover_text("The whole map, seen from above. The cross marks its centre.");
}

/// The picture of what a template makes, drawn on another thread, in
/// `rect`.
fn template_preview(ui: &mut Ui, nm: &mut NewMap, t: Template, rect: Rect) {
    let pal = Palette::of(ui);
    if let Some((drawn, rx)) = &nm.drawing {
        match rx.try_recv() {
            Ok(image) => {
                let texture = ui.ctx().load_texture("template preview", image, egui::TextureOptions::LINEAR);
                nm.template_preview = Some((*drawn, texture));
                nm.drawing = None;
            }
            Err(TryRecvError::Disconnected) => nm.drawing = None,
            Err(TryRecvError::Empty) => {}
        }
    }
    let shown = nm.template_preview.as_ref().filter(|p| p.0 == t).map(|p| p.1.id());
    if shown.is_none() && nm.drawing.as_ref().is_none_or(|d| d.0 != t) {
        let (tx, rx) = channel();
        let reg = nm.registry.clone();
        let sea = nm.params.sea_level as f32;
        let side = nm.width.max(nm.depth) as f64;
        let ctx = ui.ctx().clone();
        let spawned = std::thread::Builder::new().name("massif home template".into()).spawn(move || {
            let extent = massif_core::graph::Extent {
                x0: -side / 2.0,
                z0: -side / 2.0,
                size: side,
                n: 160,
            };
            if let Some(image) = crate::graph_editor::template_picture(t, &reg, sea, &extent, None) {
                let _ = tx.send(image);
                ctx.request_repaint();
            }
        });
        if spawned.is_ok() {
            nm.drawing = Some((t, rx));
        }
    }
    match shown {
        Some(id) => picture_tile(ui, rect, id, rect.size(), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0))),
        None => {
            ui.painter().rect(rect, theme::CARD_RADIUS, pal.well(), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            ui.put(Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), egui::Spinner::new());
        }
    }
    ui.interact(rect, egui::Id::unique("home template preview"), Sense::hover()).on_hover_text("What the template's nodes make: the map is made on the Terrain side, from them.");
}

/// What a new map will look like from above: terrain colours, water and
/// hill shading, over the whole map.
pub fn preview_image(params: &GenParams, reg: &massif_core::Registry, n: usize) -> egui::ColorImage {
    let p = massif_core::generate::preview(params, reg, n);
    let sea = params.sea_level as f32;
    let fluid = if params.lava { [238.0, 112.0, 22.0] } else { [48.0, 96.0, 200.0] };
    let s = p.spacing as f32;
    let mut pixels = Vec::with_capacity(n * n);
    for z in 0..n {
        for x in 0..n {
            let at = |x: usize, z: usize| p.heights[z.min(n - 1) * n + x.min(n - 1)];
            let h = at(x, z);
            let (top, _) = crate::panels::terrain_colors(reg, p.terrains[z * n + x].0);
            let mut c = [top.r() as f32, top.g() as f32, top.b() as f32];
            // Lit from the north-west.
            let dx = (at(x + 1, z) - at(x.saturating_sub(1), z)) / (2.0 * s);
            let dz = (at(x, z + 1) - at(x, z.saturating_sub(1))) / (2.0 * s);
            let shade = (1.0 - (dx + dz) * 1.6).clamp(0.45, 1.35);
            for v in &mut c {
                *v *= shade;
            }
            if h < sea && params.lava {
                // Lava hides what lies under it: bright, a darker crust
                // along its shores.
                let crust = (1.0 - (sea - h) / 2.5).clamp(0.0, 1.0);
                let dark = [150.0, 42.0, 12.0];
                for i in 0..3 {
                    c[i] = fluid[i] + (dark[i] - fluid[i]) * 0.65 * crust;
                }
            } else if h < sea {
                let t = 0.55 + 0.4 * ((sea - h) / 20.0).min(1.0);
                for i in 0..3 {
                    c[i] = c[i] * (1.0 - t) + fluid[i] * t;
                }
            }
            pixels.push(Color32::from_rgb(c[0].clamp(0.0, 255.0) as u8, c[1].clamp(0.0, 255.0) as u8, c[2].clamp(0.0, 255.0) as u8));
        }
    }
    egui::ColorImage::new([n, n], pixels)
}

/// The picture behind the home screen: a massif at dusk. Stars and the
/// milky way over the glow of a sun gone down, in the accent colour; ranges
/// of mountains in layers, the farthest pale in the haze, the nearest dark,
/// lit along their crests and on the flanks that face the glow, snow on the
/// highest; a valley in the middle, where the settings are written, and a
/// tall peak to the right. Drawn as a picture, `assets/backdrops/
/// massif-dusk.jpg` (see the test below), dithered as the others are.
#[cfg(test)]
fn nocturne(cols: usize, rows: usize, accent: Color32, bg: Color32) -> image::RgbImage {
    let dark = true;
    use rayon::prelude::*;
    let noise = massif_core::noise::Noise::new(0x6d61_7373);
    let a = cols as f64 / rows.max(1) as f64;
    // Far to near: where the range stands, how tall it rises, how fine
    // its peaks are, how pale the haze leaves it.
    const LAYERS: [(f64, f64, f64, f32); 5] = [(0.75, 0.08, 3.4, 0.30), (0.80, 0.11, 2.5, 0.21), (0.86, 0.15, 1.8, 0.13), (0.92, 0.17, 1.25, 0.07), (1.0, 0.20, 0.9, 0.03)];
    let n = LAYERS.len();
    let crests: Vec<Vec<f64>> = LAYERS
        .iter()
        .enumerate()
        .map(|(k, &(base, amp, freq, _))| {
            let kf = k as f64;
            (0..cols)
                .map(|i| {
                    let u = (i as f64 + 0.5) / cols as f64;
                    let x = u * a;
                    let ridge = noise.ridged(x * freq + kf * 7.31, kf * 3.7, 6, 2.0, 0.5);
                    let swell = 0.5 + 0.5 * noise.fbm(x * freq * 0.35 + 31.0, kf * 1.9, 3, 2.0, 0.5);
                    let mut c = base - amp * (0.72 * ridge + 0.38 * swell);
                    // A valley in the middle, under what is written; the
                    // ranges rise on either side.
                    c += (0.07 + 0.02 * kf) * (-((u - 0.47) / 0.33).powi(2)).exp();
                    c -= 0.05 * (-((u - 0.0) / 0.1).powi(2)).exp();
                    // The massif: a tall peak to the right, a lesser one to
                    // the left, far off.
                    if k == 2 {
                        c -= 0.30 * (-((u - 0.88) / 0.07).powi(2)).exp() * (0.8 + 0.2 * ridge);
                    }
                    if k == 1 {
                        c -= 0.07 * (-((u - 0.07) / 0.05).powi(2)).exp();
                    }
                    c
                })
                .collect()
        })
        .collect();
    // The glow of the sun gone down, right of the middle, low.
    let (gx, gy, gr) = (0.80, 0.76, 0.40);
    let glow = |u: f64, v: f64| (-(((u - gx) * a).powi(2) + ((v - gy) * 1.5).powi(2)) / (gr * gr)).exp() as f32;
    // The milky way, across the sky from the left, rising.
    let band_dir = {
        let (dx, dy) = (0.96 * a, -0.30);
        let len = (dx * dx + dy * dy).sqrt();
        (dx / len, dy / len)
    };
    let sky_ink = if dark { Color32::from_rgb(132, 142, 176) } else { Color32::from_rgb(110, 116, 132) };
    let rock_ink = if dark { Color32::from_rgb(170, 170, 180) } else { Color32::from_rgb(58, 60, 68) };
    let star_ink = Color32::from_rgb(236, 238, 246);
    let rows_out: Vec<(Vec<Color32>, Vec<usize>)> = (0..rows)
        .into_par_iter()
        .map(|j| {
            let v = (j as f64 + 0.5) / rows as f64;
            let mut row = vec![bg; cols];
            let mut stars = Vec::new();
            for (i, px) in row.iter_mut().enumerate() {
                let u = (i as f64 + 0.5) / cols as f64;
                let x = u * a;
                let g = glow(u, v);
                let hash = {
                    let h = (i as u32).wrapping_mul(0x9e37_79b1) ^ (j as u32).wrapping_mul(0x85eb_ca77);
                    h ^ (h >> 15)
                };
                let (light, ink, star) = match (0..n).rev().find(|&k| crests[k][i] <= v) {
                    None => {
                        // The sky: darker overhead, the glow low.
                        // Calm where the settings are written.
                        let calm = (-(((u - 0.5) / 0.3).powi(2) + ((v - 0.42) / 0.3).powi(2))).exp() as f32;
                        let mut l = 0.012 + 0.08 * smooth(0.15, 0.74, v as f32) + 0.36 * g;
                        let (px0, py0) = (x - 0.02 * a, v - 0.36);
                        let along = px0 * band_dir.0 + py0 * band_dir.1;
                        let across = (px0 - along * band_dir.0).hypot(py0 - along * band_dir.1);
                        let band = (-(across / 0.11).powi(2)).exp() as f32;
                        let clouds = 0.55 + 0.45 * noise.fbm(x * 3.1 + 5.0, v * 3.1, 4, 2.1, 0.55) as f32;
                        let dust = (noise.ridged(x * 5.2 + 9.0, v * 5.2, 3, 2.0, 0.5) as f32 - 0.58).max(0.0) * 1.6;
                        l += band * (0.12 * clouds - 0.07 * dust) * (1.0 - g) * (1.0 - 0.75 * calm);
                        // Stars, thicker in the milky way, none in the glow.
                        let fade = (1.0 - smooth(0.45, 0.72, v as f32)) * (1.0 - g * 1.4).max(0.0) * (1.0 - 0.8 * calm);
                        let density = (0.003 + 0.02 * band) * fade;
                        let star = ((hash & 0xffff) as f32) < density * 65536.0;
                        let ink = theme::mix(sky_ink, accent, (g * 1.3).min(1.0));
                        if !dark {
                            // By day: a clear sky, the glow low over the ranges.
                            let l = 0.42 * g + 0.06 * smooth(0.58, 0.8, v as f32);
                            (l, ink, None)
                        } else {
                            (l, ink, star.then(|| 0.55 + 0.45 * ((hash >> 16) & 0xff) as f32 / 255.0))
                        }
                    }
                    Some(k) => {
                        let (base, amp, _, haze) = LAYERS[k];
                        let near = k as f32 / (n - 1) as f32;
                        let c = crests[k][i];
                        let depth = (v - c) as f32;
                        // Under the crest, the flank faces the glow where the
                        // crest falls toward it; lower down, the rock of the
                        // face, ribs and gullies lit from the same side.
                        let (l0, l1) = (crests[k][i.saturating_sub(1)], crests[k][(i + 1).min(cols - 1)]);
                        let fall = ((l1 - l0) * cols as f64 / 2.0) as f32;
                        let crest_facing = (fall * 2.6).clamp(-1.0, 1.0);
                        let face = |x: f64| noise.ridged(x * 26.0 + k as f64 * 17.0, v * 15.0 + k as f64 * 5.0, 4, 2.0, 0.5);
                        let e = a / cols as f64;
                        let rib = ((face(x + e) - face(x - e)) / (2.0 * e) * 0.012).clamp(-1.0, 1.0) as f32;
                        let blend = (depth / 0.03).clamp(0.0, 1.0);
                        let facing = crest_facing * (1.0 - blend) + rib * blend;
                        let flank = (0.5 + 0.5 * facing) * (-depth / (0.06 + 0.08 * near)).exp();
                        // Snow on the highest: above a line, ragged.
                        let height = ((base - c) / amp) as f32;
                        let ragged = 0.08 * noise.fbm(x * 40.0, v * 40.0, 2, 2.0, 0.5) as f32;
                        let snow = if height > 0.55 && depth < (height - 0.55) * 0.09 + ragged.abs() * 0.3 { 0.28 * (0.6 + 0.4 * facing.max(0.0)) } else { 0.0 };
                        let rim = if depth < 1.6 / rows as f32 { 0.32 * (0.35 + 0.65 * glow(u, c)) } else { 0.0 };
                        let tone = haze * (0.5 + 0.4 * glow(u, c) + 0.45 * g);
                        let l = tone + 0.3 * (1.0 - 0.5 * near) * flank + snow + rim;
                        let ink = theme::mix(rock_ink, accent, (0.25 + 0.75 * glow(u, c)).min(1.0) * (0.55 + 0.45 * (1.0 - near)));
                        (l, ink, None)
                    }
                };
                if let Some(bright) = star {
                    *px = theme::scale(star_ink, bright);
                    stars.push(i);
                    continue;
                }
                let lit = theme::scale(ink, light.clamp(0.0, 1.0));
                *px = Color32::from_rgb(bg.r().max(lit.r()), bg.g().max(lit.g()), bg.b().max(lit.b()));
            }
            (row, stars)
        })
        .collect();
    let mut img = image::RgbImage::from_pixel(cols as u32, rows as u32, image::Rgb([bg.r(), bg.g(), bg.b()]));
    for (j, (row, stars)) in rows_out.into_iter().enumerate() {
        for (i, px) in row.into_iter().enumerate() {
            img.put_pixel(i as u32, j as u32, image::Rgb([px.r(), px.g(), px.b()]));
        }
        // A star is two pixels wide: still one once the picture is made
        // smaller for the cells.
        for i in stars {
            let px = img.get_pixel(i as u32, j as u32).0;
            for (dx, dy) in [(1, 0), (0, 1), (1, 1)] {
                if i + dx < cols && j + dy < rows {
                    img.put_pixel((i + dx) as u32, (j + dy) as u32, image::Rgb(px));
                }
            }
        }
    }
    img
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// A seed from the clock.
pub fn random_seed() -> i64 {
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

/// A seed typed in: a number, or any text (as Minecraft takes it).
pub fn parse_seed(text: &str) -> i64 {
    let t = text.trim();
    if let Ok(n) = t.parse::<i64>() {
        return n;
    }
    // Java's String.hashCode.
    t.encode_utf16().fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(c as i32)) as i64
}

#[cfg(test)]
mod backdrop_tests {
    use super::*;

    /// Every thread of rayon's global pool held until the guard is dropped
    /// (or, at the latest, after `most`), as a slow load in it would.
    pub(crate) struct BusyPool(Arc<std::sync::atomic::AtomicBool>);

    impl BusyPool {
        pub(crate) fn hold(most: std::time::Duration) -> Self {
            use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
            let free = Arc::new(AtomicBool::new(false));
            let held = Arc::new(AtomicUsize::new(0));
            let threads = rayon::current_num_threads();
            for _ in 0..threads {
                let (free, held) = (free.clone(), held.clone());
                rayon::spawn(move || {
                    held.fetch_add(1, Ordering::SeqCst);
                    let until = std::time::Instant::now() + most;
                    while !free.load(Ordering::SeqCst) && std::time::Instant::now() < until {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                });
            }
            let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while held.load(Ordering::SeqCst) < threads && std::time::Instant::now() < until {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Self(free)
        }
    }

    impl Drop for BusyPool {
        fn drop(&mut self) {
            self.0.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// The home screen's first frame waits on no work of rayon's global
    /// pool, which slow loads may hold (0.5.1 waited there for minutes on a
    /// slow computer with many WorldPainter brushes): its picture and the
    /// new map's preview come later, from threads of their own, as do the
    /// ages of the recent maps.
    #[test]
    fn the_first_frame_waits_on_nothing() {
        let mut home = Home::new(massif_core::Registry::vanilla());
        home.in_frame = false;
        let recent = vec![std::env::temp_dir().join("massif-no-such-map.massif")];
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_accessibility_check(false)
            .build_ui_state(move |ui, (shown, home): &mut (bool, Home)| {
                // The app's fonts first, as the app has them when it starts.
                if !*shown {
                    crate::theme::install(ui.ctx());
                    crate::theme::set_typeface(ui.ctx(), Default::default());
                    return;
                }
                home.ui(ui, &recent, None);
            }, (false, home));
        h.step();
        h.state_mut().0 = true;
        let busy = BusyPool::hold(std::time::Duration::from_secs(20));
        let start = std::time::Instant::now();
        h.step();
        let first = start.elapsed();
        drop(busy);
        assert!(first < std::time::Duration::from_secs(3), "the first frame took {first:?} while the pool was busy");
        // What was left to the threads comes.
        let until = std::time::Instant::now() + std::time::Duration::from_secs(30);
        loop {
            h.step();
            let home = &h.state().1;
            let done = home.backdrop.as_ref().is_some_and(|b| !b.frames.is_empty()) && home.new_map.preview.is_some() && home.ages.len() == 1;
            if done {
                assert_eq!(home.ages.values().next(), Some(&None), "the map is missing");
                break;
            }
            assert!(std::time::Instant::now() < until, "the picture, the preview and the ages never came");
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
    }

    /// What loads in the pool kept for loads leaves rayon's global pool to
    /// the interface.
    #[test]
    fn loads_leave_the_global_pool_free() {
        use rayon::prelude::*;
        let free = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let held = free.clone();
        let load = std::thread::spawn(move || {
            crate::jobs::in_loading_pool(|| {
                (0..64).into_par_iter().for_each(|_| {
                    let until = std::time::Instant::now() + std::time::Duration::from_secs(20);
                    while !held.load(std::sync::atomic::Ordering::SeqCst) && std::time::Instant::now() < until {
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                })
            })
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
        let start = std::time::Instant::now();
        let sum: u64 = (0..10_000u64).into_par_iter().sum();
        assert_eq!(sum, 49_995_000);
        assert!(start.elapsed() < std::time::Duration::from_secs(3), "the global pool waited {:?}", start.elapsed());
        free.store(true, std::sync::atomic::Ordering::SeqCst);
        load.join().unwrap();
    }

    /// The massif at dusk, drawn into `assets/backdrops` when
    /// MASSIF_BACKDROP_DIR says where.
    #[test]
    fn draw_the_massif_at_dusk() {
        let Some(dir) = std::env::var_os("MASSIF_BACKDROP_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let img = nocturne(1600, 900, Color32::from_rgb(249, 115, 22), Color32::from_rgb(8, 8, 9));
        let mut out = std::fs::File::create(dir.join("massif-dusk.jpg")).unwrap();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 90).encode_image(&img).unwrap();
    }

    /// Every picture of the folder decodes, and dithers into frames of a
    /// palette of its own.
    #[test]
    fn the_backdrops_dither() {
        assert!(pictures::BACKDROPS.len() >= 4, "{} pictures", pictures::BACKDROPS.len());
        for (name, bytes, dawn) in pictures::BACKDROPS {
            // Every scene has its dawn, for the light theme.
            assert!(dawn.is_some(), "{name}: no dawn");
            for (bytes, light, bg) in [(*bytes, Light::Night, Color32::from_rgb(8, 8, 9)), (dawn.unwrap_or(bytes), Light::Dawn, Color32::from_rgb(250, 250, 250))] {
                let img = image::load_from_memory(bytes).unwrap_or_else(|e| panic!("{name}: {e}")).to_rgb8();
                let frames = dither_frames(&img, 96, 64, light, bg, [20, 10, 70, 50], false);
                assert_eq!(frames.len(), 4);
                let colours: std::collections::HashSet<[u8; 4]> = frames[0].pixels.iter().map(|c| c.to_array()).collect();
                assert!((2..=16).contains(&colours.len()), "{name}: {} colours", colours.len());
                assert_ne!(frames[0].pixels, frames[1].pixels, "{name}: the frames differ");
            }
        }
    }
}
