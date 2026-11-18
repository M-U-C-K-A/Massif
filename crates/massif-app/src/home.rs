//! The home screen: where Massif opens, and where a new map is made.
//!
//! No map is shown before there is one. On the left, the ways in — a new
//! map, a map from a height map, a map of one's own to open, a WorldPainter
//! world to import — and the maps opened lately; in the middle, the
//! settings of a new map with its preview. Behind them, ridges drawn in
//! dots, lit by the accent colour.

use crate::backdrop;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, Part, button_group, input_group};
use egui::{Align, Align2, Color32, CursorIcon, FontId, Rect, Response, RichText, Sense, Stroke, StrokeKind, TextureHandle, Ui, UiBuilder, pos2, vec2};
use massif_core::generate::{FromHeightmap, GenParams, Landform};
use massif_core::graph::Template;
use massif_core::materials::Palette as BlockPalette;
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
    /// The block palettes to make it of (those of Massif, the player's).
    pub palettes: Vec<BlockPalette>,
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
            palettes: vec![BlockPalette::vanilla()],
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
            p.name = tr!("New World").into();
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
        let name = path.file_stem().and_then(|s| s.to_str()).unwrap_or(tr!("Height map")).to_owned();
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
    /// A new world from the relief of a real place.
    RealPlace,
    /// Open a map of one's own (`.massif`).
    Open,
    /// Import a WorldPainter world.
    Import,
    OpenRecent(PathBuf),
    /// Take a map off the list of recent ones.
    Forget(PathBuf),
    Reveal(PathBuf),
    /// Open a sample map: made from this template, its relief built.
    Sample(Template),
    /// Back to the map that was open.
    Back,
    Settings,
    /// A file let go over the window.
    Dropped(PathBuf),
    /// What is new in the newer version out.
    Update,
    /// The block palettes window.
    EditPalettes,
}

/// The background behind the home screen: one frame of it, dithered for
/// one size and theme, on the graphics card.
struct Waves {
    key: WavesKey,
    /// The frame shown, by its number (seconds × [`backdrop::FPS`]).
    frame: i64,
    texture: TextureHandle,
}

/// What the ridges are drawn for.
#[derive(Clone, Copy, PartialEq, Debug)]
struct WavesKey {
    cols: usize,
    rows: usize,
    /// Where the words are, in cells.
    calm: [i32; 4],
    /// Pixels a cell.
    pitch: usize,
    look: LookKey,
}

/// What the ridges' colours are worked out for.
#[derive(Clone, Copy, PartialEq, Debug)]
struct LookKey {
    dark: bool,
    high: bool,
    accent: Color32,
}

/// When maps were last changed, `None` for those that are not there.
type Ages = Vec<(PathBuf, Option<SystemTime>)>;

pub struct Home {
    pub page: Page,
    pub new_map: NewMap,
    pub heightmap: Option<HeightmapPick>,
    /// The name of the map open before, to go back to.
    pub back_to: Option<String>,
    waves: Option<Waves>,
    /// The ridges' colours and tables, for the theme they were made for.
    look: Option<(LookKey, backdrop::Look)>,
    /// The frame the background stands at while it does not move.
    still_frame: i64,
    /// What is behind it (the settings).
    pub background: backdrop::Background,
    /// The waves shown still at this time, in seconds (the tests' renders).
    pub wave_time: Option<f64>,
    /// When each recent map was last changed (`None`: it is not there),
    /// read on a thread of its own: a map on a drive that is gone, or on a
    /// network share that does not answer, would hold up the window.
    ages: HashMap<PathBuf, Option<SystemTime>>,
    ages_asked: Option<Receiver<Ages>>,
    ages_at: f64,
    /// Make the pictures (the preview, the samples') and read the recent
    /// maps in the frame itself, as tests want; the app makes them on other
    /// threads, so that its first frame waits on none of it.
    pub in_frame: bool,
    /// Whether the ridges or the picture behind move (the settings).
    pub animate: bool,
    /// A newer version out, by its number: a line above Settings says so.
    pub update: Option<String>,
    logo: Option<TextureHandle>,
    /// How tall what is written in the middle was last laid out: lifted
    /// when it would not fit under the top margin.
    content_height: f32,
    /// The samples' pictures, and those being drawn.
    sample_pictures: HashMap<Template, TextureHandle>,
    sample_drawing: Option<Receiver<(Template, egui::ColorImage)>>,
}

/// The sample maps the home screen offers: made from templates of the
/// terrain generator when one is opened, so that none is shipped.
pub const SAMPLES: [Template; 3] = [Template::Alpine, Template::Archipelago, Template::FloatingMountains];

/// The side of a sample map, in blocks.
const SAMPLE_SIDE: i32 = 768;

/// The settings a sample map is made with: always the same, a flat map
/// whose relief the template builds.
pub fn sample_params(t: Template) -> GenParams {
    let mut p = GenParams {
        name: crate::i18n::tr_name(t.label()).to_owned(),
        seed: 20_240_611 + t as i64,
        tiles_x: SAMPLE_SIDE / 128,
        tiles_z: SAMPLE_SIDE / 128,
        ..GenParams::default()
    };
    p.landform = Landform::Flat;
    p.level = None;
    p.flat_height = p.sea_level as f32 + 1.0;
    p
}

impl Home {
    pub fn new(registry: massif_core::Registry) -> Self {
        Self {
            page: Page::NewMap,
            new_map: NewMap::new(registry),
            heightmap: None,
            back_to: None,
            waves: None,
            look: None,
            still_frame: 0,
            background: backdrop::Background::default(),
            wave_time: None,
            ages: HashMap::new(),
            ages_asked: None,
            ages_at: f64::NEG_INFINITY,
            in_frame: cfg!(test),
            animate: true,
            update: None,
            logo: None,
            content_height: 0.0,
            sample_pictures: HashMap::new(),
            sample_drawing: None,
        }
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

        crate::startup::frame_step(ui.ctx(), "the home screen: the waves behind it");
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
            painter.text(zone.center(), Align2::CENTER_CENTER, tr!("Drop it to open it"), FontId::proportional(18.0), pal.text);
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
            ui.label(RichText::new(tr!("Massif")).bold().size(15.0));
        });
        ui.add_space(16.0);
        if nav(ui, Icon::Plus, tr!("New map"), self.page == Page::NewMap).clicked() {
            self.page = Page::NewMap;
        }
        if nav(ui, Icon::ImageUp, tr!("From a height map"), self.page == Page::FromHeightmap).clicked() {
            self.page = Page::FromHeightmap;
        }
        if nav(ui, Icon::FolderOpen, tr!("Open a map…"), false).on_hover_text(tr!("A Massif map (.massif)")).clicked() {
            *action = Some(HomeAction::Open);
        }
        if nav(ui, Icon::Import, tr!("Import from WorldPainter…"), false).on_hover_text(tr!("A WorldPainter world (.world)")).clicked() {
            *action = Some(HomeAction::Import);
        }
        ui.add_space(20.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new(tr!("Samples")).size(11.5).color(pal.weak));
        });
        ui.add_space(4.0);
        self.sample_rows(ui, action);
        ui.add_space(16.0);
        ui.horizontal(|ui| {
            ui.add_space(10.0);
            ui.label(RichText::new(tr!("Recent maps")).size(11.5).color(pal.weak));
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
                        if ui.button(theme::with_icon(Icon::FolderSearch, tr!("Show in Finder"))).clicked() {
                            *action = Some(HomeAction::Reveal(path.clone()));
                        }
                        if ui.button(theme::with_icon(Icon::X, tr!("Remove from the list"))).clicked() {
                            *action = Some(HomeAction::Forget(path.clone()));
                        }
                    });
                }
            });
        ui.with_layout(egui::Layout::bottom_up(Align::Min), |ui| {
            ui.add_space(12.0);
            if nav(ui, Icon::Settings, tr!("Settings"), false).on_hover_text(tr!("Appearance, accessibility, typeface, sounds… (⌘,)")).clicked() {
                *action = Some(HomeAction::Settings);
            }
            if let Some(version) = &self.update
                && update_nav(ui, version).clicked()
            {
                *action = Some(HomeAction::Update);
            }
            if let Some(name) = &self.back_to
                && nav(ui, Icon::ArrowLeft, &tr!("Back to {name}", name), false).on_hover_text(tr!("Esc")).clicked()
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
            Page::NewMap => (tr!("What will you build?"), tr!("A new map: its size and its land, then make it.")),
            Page::FromHeightmap => (tr!("A map from a height map"), tr!("A greyscale picture from Gaea, World Machine or an image editor.")),
        };
        art_label(ui, RichText::new(title).size(28.0).medium().color(pal.text));
        ui.add_space(2.0);
        art_label(ui, RichText::new(subtitle).size(13.0).color(art_weak(&pal)));
        ui.add_space(22.0);
        // Laid on the ridges themselves; on a background of colours, on a
        // solid surface of the card's colour, so that it reads over any.
        let surface = (self.background.style != backdrop::Style::Ridges).then(|| ui.painter().add(egui::Shape::Noop));
        let laid = ui.allocate_ui_with_layout(vec2(width - 34.0, 0.0), egui::Layout::top_down(Align::Min), |ui| {
            ui.set_width(width - 34.0);
            match self.page {
                Page::NewMap => self.new_map_card(ui, action),
                Page::FromHeightmap => self.heightmap_card(ui, action),
            }
        });
        if let Some(surface) = surface {
            let rect = laid.response.rect.expand2(vec2(17.0, 14.0));
            let edge = if pal.high { pal.control_border() } else { pal.border };
            ui.painter().set(surface, egui::epaint::RectShape::new(rect, theme::CARD_RADIUS, pal.panel, Stroke::new(1.0, edge), StrokeKind::Inside));
        }
        ui.add_space(14.0);
        let links: [(Icon, &str, &str, HomeAction); 4] = [
            (Icon::FolderOpen, tr!("Open a map"), tr!("A Massif map (.massif)"), HomeAction::Open),
            (Icon::Import, tr!("Import from WorldPainter"), tr!("A WorldPainter world (.world)"), HomeAction::Import),
            (Icon::ImageUp, tr!("From a height map"), tr!("A greyscale picture: 16-bit PNG, JPEG, .r16 or .r32"), HomeAction::PickHeightmap),
            (Icon::Earth, tr!("From a real place"), tr!("The relief of a place on the Earth, at the scale you choose"), HomeAction::RealPlace),
        ];
        if let Some(a) = link_row(ui, &links) {
            if matches!(a, HomeAction::PickHeightmap) {
                self.page = Page::FromHeightmap;
            }
            *action = Some(a);
        }
        ui.add_space(6.0);
        art_label(ui, RichText::new(tr!("or drop a .massif, a .world or a height map on the window")).size(11.5).color(art_weak(&pal)));
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
            row(ui, tr!("Size"), &mut |ui| {
                input_group(
                    ui,
                    "home size",
                    &mut [
                        Part::Snapped(&mut nm.width, MAP_SIDES, 64.0, whole_tiles, tr!("whole tiles of 128 blocks")),
                        Part::Text("×"),
                        Part::Snapped(&mut nm.depth, MAP_SIDES, 64.0, whole_tiles, tr!("whole tiles of 128 blocks")),
                        Part::Text(tr!("blocks")),
                    ],
                );
            });
            row(ui, tr!("Height"), &mut |ui| limits(ui, &mut nm.params.min_y, &mut nm.params.max_y));
            row(ui, tr!("Land"), &mut |ui| {
                let shown = nm.template.map_or(crate::i18n::tr_name(nm.params.landform.label()), |t| crate::i18n::tr_name(t.label()));
                crate::widgets::combo("home landform").selected_text(shown).width(third * 1.4).height(360.0).show_ui(ui, |ui| {
                    for l in Landform::ALL {
                        if ui.selectable_label(nm.template.is_none() && nm.params.landform == l, crate::i18n::tr_name(l.label())).clicked() {
                            nm.params.landform = l;
                            nm.template = None;
                        }
                    }
                    ui.separator();
                    ui.label(RichText::new(tr!("From the terrain generator")).size(11.0).color(pal.weak));
                    for t in Template::ALL.into_iter().filter(|t| !t.reads_the_map()) {
                        if ui.selectable_label(nm.template == Some(t), crate::i18n::tr_name(t.label())).on_hover_text(crate::i18n::tr_name(t.description())).clicked() {
                            nm.template = Some(t);
                        }
                    }
                });
                let mut automatic = nm.params.surface.is_none();
                let reg = &nm.registry;
                let shown = match nm.params.surface {
                    None => tr!("Automatic surface").to_owned(),
                    Some(t) => reg.terrain(t).name.clone(),
                };
                let mut chosen = nm.params.surface;
                crate::widgets::combo("home surface").selected_text(shown).width(third * 1.4).height(320.0).show_ui(ui, |ui| {
                    if ui
                        .selectable_label(automatic, tr!("Automatic surface"))
                        .on_hover_text(tr!("Grass, sand, rock and snow by height and slope"))
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
            row(ui, tr!("Relief"), &mut |ui| {
                ui.add_enabled(hilly, Field::new(&mut nm.params.relief, 0.0..=most).text(tr!("Height")).suffix("%").max_decimals(0).width(third));
                ui.add_enabled(hilly, Field::new(&mut nm.params.feature_size, 10.0..=1000.0).text(tr!("Features")).suffix("%").max_decimals(0).width(third));
                let mut level = nm.params.level.unwrap_or(if hilly { nm.params.sea_level as f32 } else { nm.params.flat_height });
                let free = nm.template.is_none();
                if ui.add_enabled(free, Field::new(&mut level, (bottom + 1) as f32..=(top - 2) as f32).text(tr!("Ground")).max_decimals(0).width(third)).changed() {
                    nm.params.level = Some(level);
                    nm.params.flat_height = level;
                }
            });
            row(ui, tr!("Water"), &mut |ui| {
                ui.add(Field::new(&mut nm.params.sea_level, bottom + 1..=top - 2).text(tr!("Level")).width(third));
                let lava = nm.params.lava;
                match button_group(ui, "home fluid", &[tr!("Water"), tr!("Lava")], &[!lava, lava]) {
                    Some(0) => nm.params.lava = false,
                    Some(_) => nm.params.lava = true,
                    None => {}
                }
                if !nm.params.lava {
                    let beaches = nm.params.beaches && nm.params.surface.is_none();
                    if button_group(ui, "home beaches", &[tr!("Beaches")], &[beaches]).is_some() {
                        nm.params.beaches = !nm.params.beaches;
                    }
                }
            });
            row(ui, tr!("Blocks"), &mut |ui| match crate::palettes::choose(ui, "home palette", &nm.palettes, &nm.params.palette, third * 1.4) {
                Some(crate::palettes::Picked::Use(p)) => nm.params.palette = p,
                Some(crate::palettes::Picked::Edit) => *action = Some(HomeAction::EditPalettes),
                None => {}
            });
            row(ui, tr!("Seed"), &mut |ui| {
                let (_, dice) = input_group(ui, "home seed", &mut [Part::Edit(&mut nm.seed, third * 1.4), Part::Button(Icon::Dices, tr!("Another seed"))]);
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
            let across = if minutes < 1.0 { tr!("under a minute across").to_owned() } else { tr!("{minutes:.0} min. across on foot", minutes) };
            let (bottom, top) = massif_core::generate::build_limits(params.min_y, params.max_y);
            let pack = if (bottom, top) != massif_core::generate::VANILLA_LIMITS { tr!(" (with a data pack)") } else { "" };
            let game = crate::palettes::game_of(&params.palette);
            art_label(
                ui,
                RichText::new(tr!("{game}  ·  Y {bottom} to {top}{pack}  ·  {across}  ·  about {size}", game, size = crate::panels::human((tiles * 12_000) as u64), bottom, top, pack, across))
                .size(11.5)
                .color(art_weak(&pal)),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let create = primary(ui, tr!("Create map"), "⏎").on_hover_text(tr!("Enter"));
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
            response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Choose a height map")));
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
            line(tr!("Choose a height map").to_owned(), 15.0, pal.text, 10.0);
            line(tr!("16-bit PNG, JPEG, .r16 or .r32 — or drop one here").to_owned(), 12.0, art_weak(&pal), 32.0);
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
            row(ui, tr!("File"), &mut |ui| {
                let (_, another) = input_group(ui, "height map file", &mut [Part::Text(&file), Part::Button(Icon::FolderOpen, tr!("Choose another height map"))]);
                if another.is_some() {
                    *action = Some(HomeAction::PickHeightmap);
                }
            });
            row(ui, tr!("Height"), &mut |ui| limits(ui, &mut h.min_y, &mut h.max_y));
            let (bottom, top) = massif_core::generate::build_limits(h.min_y, h.max_y);
            row(ui, tr!("Scale"), &mut |ui| {
                ui.add(Field::new(&mut h.scale, 0.25..=16.0).logarithmic(true).text(tr!("Blocks a pixel")).max_decimals(2).width(w * 2.0 + 6.0));
            });
            row(ui, tr!("Heights"), &mut |ui| {
                ui.add(Field::new(&mut h.low, bottom as f32..=(top - 1) as f32).text(tr!("Black is Y")).max_decimals(0).width(w));
                ui.add(Field::new(&mut h.high, bottom as f32..=(top - 1) as f32).text(tr!("White is Y")).max_decimals(0).width(w));
            });
            row(ui, tr!("Water"), &mut |ui| {
                ui.add(Field::new(&mut h.sea_level, bottom..=top - 1).text(tr!("Level")).width(w));
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
                    .on_hover_text(tr!("{path}\n{width} × {height} pixels", path = pick.path.display(), width = pick.map.width, height = pick.map.height));
            }
        }
        ui.add_space(4.0);
        hairline(ui);
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (w, d) = (pick.map.width as f32 * pick.how.scale, pick.map.height as f32 * pick.how.scale);
            art_label(
                ui,
                RichText::new(tr!("{width} × {height} pixels  →  {w:.0} × {d:.0} blocks, Y {low:.0} to {high:.0}", width = pick.map.width, height = pick.map.height, low = pick.how.low, high = pick.how.high, w, d))
                    .size(11.5)
                    .color(art_weak(&pal)),
            );
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                let create = primary(ui, tr!("Create map"), "⏎").on_hover_text(tr!("Enter"));
                let enter = !ui.ctx().egui_wants_keyboard_input() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if create.clicked() || enter {
                    *action = Some(HomeAction::FromHeightmap(pick.map.clone(), pick.how.clone()));
                }
            });
        });
    }

    // ----------------------------------------------------------- backdrop --

    /// The background behind `rect`, on the page's colour, in the style of
    /// the settings (see [`backdrop`]), its light held where the settings are
    /// written (`calm`, the words' column; the smooth styles only as far down
    /// as the words go). Drawn again for each frame of [`backdrop::FPS`]
    /// while it moves: with the setting on, Reduce motion off and the window
    /// in front; still otherwise, at its first frame or where it stopped.
    fn paint_backdrop(&mut self, ui: &Ui, rect: Rect, calm: Rect) {
        let pal = Palette::of(ui);
        ui.painter().rect_filled(rect, 0, backdrop_fill(&pal));
        let (now, focused) = ui.input(|i| (i.time, i.viewport().focused.unwrap_or(true)));
        let moving = self.animate && !theme::reduce_motion(ui.ctx()) && self.wave_time.is_none();
        let frame = match self.wave_time {
            Some(t) => (t * backdrop::FPS) as i64,
            // Where it stopped while the window was behind another.
            None if moving && !focused => self.waves.as_ref().map_or(self.still_frame, |w| w.frame),
            None if moving => (now * backdrop::FPS) as i64,
            None => self.still_frame,
        };
        if moving && focused {
            self.still_frame = frame;
        }
        let b = self.background;
        if b.style == backdrop::Style::Ridges {
            self.paint_ridges(ui, rect, calm, frame);
        } else {
            self.waves = None;
            let column = Rect::from_min_max(calm.min, pos2(calm.max.x, (calm.min.y + self.content_height + 12.0).min(calm.max.y)));
            let guard = backdrop::Guard::new(&pal, backdrop_fill(&pal), art_weak(&pal), Some(column), rect);
            let t = (frame as f64 / backdrop::FPS) as f32;
            if let Some(meshes) = backdrop::meshes(&b, &pal, backdrop_fill(&pal), &guard, rect, t) {
                let painter = ui.painter_at(rect);
                for mesh in meshes {
                    painter.add(egui::Shape::mesh(mesh));
                }
            }
        }
        // The next frame, only while it moves and the window is in front:
        // coming back to it asks for a frame of its own.
        if moving && focused {
            let next = (frame + 1) as f64 / backdrop::FPS - now;
            ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(next.clamp(0.005, 1.0 / backdrop::FPS)));
        }
    }

    /// The ridges behind `rect`, frame `frame`: dithered on the processor
    /// into a texture of a cell every two points (see [`backdrop::frame`]),
    /// drawn again only when the frame or what it is drawn for changes.
    fn paint_ridges(&mut self, ui: &Ui, rect: Rect, calm: Rect, frame: i64) {
        let pal = Palette::of(ui);
        let ppp = ui.ctx().pixels_per_point();
        let pitch = ((2.0 * ppp).round() as usize).max(3);
        let cell = pitch as f32 / ppp;
        let cols = ((rect.width() / cell).ceil() as usize).max(1);
        let rows = ((rect.height() / cell).ceil() as usize).max(1);
        // Anchored to the bottom left, as the land is.
        let shown = Rect::from_min_size(pos2(rect.left(), rect.bottom() - rows as f32 * cell), vec2(cols as f32, rows as f32) * cell);
        let in_cells = |p: egui::Pos2| [((p.x - shown.left()) / cell) as i32, ((p.y - shown.top()) / cell) as i32];
        let ([x0, y0], [x1, y1]) = (in_cells(calm.min), in_cells(calm.max));
        let calm = [x0, y0, x1, y1].map(|v| v.div_euclid(8) * 8);
        let look_key = LookKey { dark: pal.dark, high: pal.high, accent: pal.accent };
        let key = WavesKey { cols, rows, calm, pitch, look: look_key };
        if self.waves.as_ref().is_none_or(|w| w.key != key || w.frame != frame) {
            if self.look.as_ref().is_none_or(|(k, _)| *k != look_key) {
                self.look = Some((look_key, backdrop::Look::new(&pal, backdrop_fill(&pal), art_weak(&pal))));
            }
            let Some((_, look)) = &self.look else { return };
            let image = backdrop::frame(look, cols, rows, Some(calm), frame as f64 / backdrop::FPS);
            match &mut self.waves {
                Some(w) => {
                    w.texture.set(image, egui::TextureOptions::NEAREST);
                    (w.key, w.frame) = (key, frame);
                }
                None => {
                    let texture = ui.ctx().load_texture("home background", image, egui::TextureOptions::NEAREST);
                    self.waves = Some(Waves { key, frame, texture });
                }
            }
        }
        if let Some(w) = &self.waves {
            ui.painter_at(rect).image(w.texture.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
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

/// The ink of muted words written over the waves: stronger than muted text
/// elsewhere, as the waves show around them.
pub(crate) fn art_weak(pal: &Palette) -> Color32 {
    if pal.high { pal.weak } else { theme::mix(pal.weak, pal.text, 0.4) }
}

/// `galley` at `at`, written straight over the waves: a halo of the page's
/// colour around its letters, so that it reads over any part of them
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

/// Words written straight over the waves (see [`paint_on_art`]).
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
            .hint_text(RichText::new(tr!("Name your map")).color(art_weak(&pal)))
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

// ---------------------------------------------------------------- parts --

fn sidebar_fill(pal: &Palette) -> Color32 {
    if pal.dark { Color32::from_rgb(14, 15, 17) } else { Color32::from_rgb(241, 241, 243) }
}

pub(crate) fn backdrop_fill(pal: &Palette) -> Color32 {
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

/// The sidebar's word that a newer version is out: an entry as the others,
/// its icon and words in the accent.
fn update_nav(ui: &mut Ui, version: &str) -> Response {
    let pal = Palette::of(ui);
    let label = tr!("Massif {version} is out", version);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
    let fill = if response.hovered() { theme::mix(sidebar_fill(&pal), pal.accent, 0.16) } else { theme::mix(sidebar_fill(&pal), pal.accent, 0.10) };
    ui.painter().rect_filled(rect, 8, fill);
    ui.painter().text(rect.left_center() + vec2(12.0, 0.0), Align2::LEFT_CENTER, theme::glyph(Icon::CircleArrowUp).to_string(), FontId::proportional(14.0), pal.accent);
    ui.painter().text(rect.left_center() + vec2(36.0, 0.0), Align2::LEFT_CENTER, label, FontId::proportional(13.0), pal.text);
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
    let name = fit_with_ellipsis(&stem, room, |s| ui.painter().layout_no_wrap(s.to_owned(), font.clone(), ink).size().x);
    ui.painter().text(rect.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, name, font, ink);
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, age, FontId::proportional(11.5), pal.weak);
    response.on_hover_text(path.display().to_string())
}

/// `text` cut to fit in `room` as `width` measures it, with an ellipsis:
/// as many of its letters as fit, never fewer than three. A few
/// measures, whatever the text and the room: the loop it replaces took
/// the ellipsis it had just added off again, and never ended on a name too
/// long for the side bar (Massif 0.3.1 to 0.6.0, a window that did not
/// answer at its first frame).
fn fit_with_ellipsis(text: &str, room: f32, width: impl Fn(&str) -> f32) -> String {
    if width(text) <= room {
        return text.to_owned();
    }
    let chars: Vec<char> = text.chars().collect();
    let cut = |k: usize| format!("{}…", chars[..k].iter().collect::<String>().trim_end());
    // The longest cut that fits, by halves: `lo` fits (or is the least
    // allowed), `hi` does not.
    let (mut lo, mut hi) = (3.min(chars.len()), chars.len());
    while lo + 1 < hi {
        let mid = (lo + hi) / 2;
        if width(&cut(mid)) <= room {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    cut(lo)
}

impl Home {
    /// The sample maps, one row each: a picture of the land, its name, its
    /// size. Their pictures are drawn once, on a thread of their own (in
    /// the frame for tests).
    fn sample_rows(&mut self, ui: &mut Ui, action: &mut Option<HomeAction>) {
        if let Some(rx) = &self.sample_drawing {
            loop {
                match rx.try_recv() {
                    Ok((t, image)) => {
                        let texture = ui.ctx().load_texture(format!("sample {t:?}"), image, egui::TextureOptions::LINEAR);
                        self.sample_pictures.insert(t, texture);
                    }
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => {
                        self.sample_drawing = None;
                        break;
                    }
                }
            }
        }
        if self.sample_pictures.len() < SAMPLES.len() && self.sample_drawing.is_none() {
            let (tx, rx) = channel();
            let reg = self.new_map.registry.clone();
            let sea = self.new_map.params.sea_level as f32;
            let ctx = ui.ctx().clone();
            let draw = move || {
                for t in SAMPLES {
                    let side = SAMPLE_SIDE as f64;
                    let extent = massif_core::graph::Extent { x0: -side / 2.0, z0: -side / 2.0, size: side, n: 64 };
                    let image = crate::graph_editor::template_picture(t, &reg, sea, &extent, None).unwrap_or_else(|| egui::ColorImage::filled([1, 1], Color32::GRAY));
                    if tx.send((t, image)).is_err() {
                        return;
                    }
                    ctx.request_repaint();
                }
            };
            if self.in_frame {
                draw();
                self.sample_drawing = Some(rx);
                return self.sample_rows(ui, action);
            }
            if std::thread::Builder::new().name("massif home samples".into()).spawn(|| crate::jobs::in_loading_pool(draw)).is_ok() {
                self.sample_drawing = Some(rx);
            }
        }
        for t in SAMPLES {
            if sample_row(ui, t, self.sample_pictures.get(&t)).clicked() {
                *action = Some(HomeAction::Sample(t));
            }
        }
    }
}

/// A sample map to open: its picture in a small rounded tile, its name and
/// its size.
fn sample_row(ui: &mut Ui, t: Template, picture: Option<&TextureHandle>) -> Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, crate::i18n::tr_name(t.label())));
    if response.hovered() {
        ui.painter().rect_filled(rect, theme::TILE_RADIUS, theme::mix(sidebar_fill(&pal), pal.text, 0.05));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let tile = Rect::from_center_size(rect.left_center() + vec2(19.0, 0.0), vec2(22.0, 22.0));
    match picture {
        Some(p) => {
            egui::Image::new((p.id(), tile.size())).corner_radius(theme::TILE_RADIUS / 2).paint_at(ui, tile);
            ui.painter().rect_stroke(tile, theme::TILE_RADIUS / 2, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        }
        None => {
            ui.painter().rect_filled(tile, theme::TILE_RADIUS / 2, theme::mix(sidebar_fill(&pal), pal.text, 0.06));
        }
    }
    let ink = theme::mix(pal.text, pal.weak, 0.2);
    ui.painter().text(rect.left_center() + vec2(40.0, 0.0), Align2::LEFT_CENTER, crate::i18n::tr_name(t.label()), FontId::proportional(13.0), ink);
    ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, format!("{SAMPLE_SIDE}²"), FontId::proportional(11.5), pal.weak);
    response.on_hover_text(crate::i18n::tr_name(t.description()))
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
        ui.label(RichText::new(tr!("No maps yet")).medium().size(13.0));
        ui.add_space(2.0);
        ui.label(RichText::new(tr!("The maps you make or open show up here.")).size(11.5).color(pal.weak));
    });
}

/// How long ago `t` was, shortly: 5m, 3h, 2d, 1w, 4mo, 1y.
fn ago(t: std::time::SystemTime) -> String {
    let s = std::time::SystemTime::now().duration_since(t).map_or(0, |d| d.as_secs());
    match s {
        0..60 => tr!("now").into(),
        60..3_600 => tr!("{n}m", n = s / 60),
        3_600..86_400 => tr!("{n}h", n = s / 3_600),
        86_400..604_800 => tr!("{n}d", n = s / 86_400),
        604_800..2_592_000 => tr!("{n}w", n = s / 604_800),
        2_592_000..31_536_000 => tr!("{n}mo", n = s / 2_592_000),
        _ => tr!("{n}y", n = s / 31_536_000),
    }
}

/// The sides a new map may have, in blocks.
const MAP_SIDES: std::ops::RangeInclusive<i32> = 128..=65536;

/// A side of a map made whole tiles: the nearest multiple of 128 blocks.
fn whole_tiles(v: i32) -> i32 {
    ((v as f64 / 128.0).round() as i32 * 128).clamp(*MAP_SIDES.start(), *MAP_SIDES.end())
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
    input_group(ui, "build bottom", &mut [Part::Text(tr!("from Y")), Part::Snapped(&mut bottom, -2032..=-64, 16.0, |v| (v as f64 / 16.0).round() as i32 * 16, tr!("whole sections of 16 blocks"))]);
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
                let _ = tx.send(crate::jobs::in_loading_pool(|| preview_image(&params, &reg, 200)));
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
            crate::loader::paint(ui, Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), crate::loader::pattern(ui.ctx()));
        }
    }
    // The centre: a cross, outlined to show on snow as on water.
    let c = rect.center();
    for (width, color) in [(3.0, Color32::from_black_alpha(70)), (1.2, Color32::WHITE)] {
        ui.painter().line_segment([c - vec2(6.0, 0.0), c + vec2(6.0, 0.0)], Stroke::new(width, color));
        ui.painter().line_segment([c - vec2(0.0, 6.0), c + vec2(0.0, 6.0)], Stroke::new(width, color));
    }
    ui.interact(rect, egui::Id::unique("home map preview"), Sense::hover()).on_hover_text(tr!("The whole map, seen from above. The cross marks its centre."));
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
            if let Some(image) = crate::jobs::in_loading_pool(|| crate::graph_editor::template_picture(t, &reg, sea, &extent, None)) {
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
            crate::loader::paint(ui, Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), crate::loader::pattern(ui.ctx()));
        }
    }
    ui.interact(rect, egui::Id::unique("home template preview"), Sense::hover()).on_hover_text(tr!("What the template's nodes make: the map is made on the Terrain side, from them."));
}

/// What a new map will look like from above: terrain colours, water and
/// hill shading, over the whole map.
pub fn preview_image(params: &GenParams, reg: &massif_core::Registry, n: usize) -> egui::ColorImage {
    // The terrains made of the blocks chosen, in their colours.
    let mut made_of = None;
    if !params.palette.is_vanilla() {
        let mut r = reg.clone();
        massif_core::materials::apply(&mut r, &params.palette);
        made_of = Some(r);
    }
    let reg = made_of.as_ref().unwrap_or(reg);
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
    /// A name cut to fit takes letters off, and ends whatever the room: the
    /// loop before took off the ellipsis it had just added, and never ended.
    #[test]
    fn a_long_name_is_cut_and_the_cut_ends() {
        let width = |s: &str| s.chars().count() as f32 * 7.0;
        assert_eq!(fit_with_ellipsis("Alps", 100.0, width), "Alps");
        let cut = fit_with_ellipsis("Mein riesiges Königreich mit Bergen und Flüssen", 140.0, width);
        assert!(cut.ends_with('…') && width(&cut) <= 140.0 && cut.chars().count() == 20, "{cut}");
        assert!(cut.starts_with("Mein riesiges"), "{cut}");
        // No room at all: the least, at once.
        assert_eq!(fit_with_ellipsis("Northern Isles", 0.0, width), "Nor…");
        assert_eq!(fit_with_ellipsis("ab", -5.0, width), "ab…");
        let long: String = "x".repeat(100_000);
        assert!(fit_with_ellipsis(&long, 300.0, width).chars().count() <= 43);
    }

    /// The home screen's side bar draws its recent maps whatever they are:
    /// names too long for it (Zumonym's start stayed for ever at "the side
    /// bar and the recent maps" in 0.6.0), Windows paths with drive letters,
    /// backslashes and shares, letters beyond ASCII, a folder, a file not
    /// there, no list at all; in a window from the smallest to a large one,
    /// at the scales Windows offers, in German. A frame that does not end is
    /// a failure here, not a test that hangs.
    #[test]
    fn the_side_bar_draws_any_recent_maps() {
        let dir = std::env::temp_dir().join(format!("massif-recent-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("a folder.massif")).unwrap();
        let lists: Vec<Vec<PathBuf>> = vec![
            Vec::new(),
            vec![
                dir.join("Mein riesiges Königreich mit Bergen, Flüssen und einer Küste im Norden.massif"),
                PathBuf::from(r"C:\Users\Jürgen Müller\Documents\Minecraft Karten\Überlebensinsel Version 2 (final).massif"),
                PathBuf::from(r"\\NAS\maps\server spawn.world"),
                dir.join("a folder.massif"),
                dir.join("not there.massif"),
                PathBuf::from("x.massif"),
                PathBuf::from(format!("{}.massif", "Ä".repeat(300))),
            ],
        ];
        let (done, finished) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            crate::i18n::set_language(crate::i18n::Language::German);
            for recent in lists {
                for (size, ppp) in [(vec2(960.0, 640.0), 1.5), (vec2(1280.0, 720.0), 1.25), (vec2(1366.0, 768.0), 1.0), (vec2(1920.0, 1080.0), 1.0)] {
                    let mut home = Home::new(massif_core::Registry::vanilla());
                    home.in_frame = false;
                    let recent = recent.clone();
                    let mut h = egui_kittest::Harness::builder()
                        .with_size(size)
                        .with_pixels_per_point(ppp)
                        .with_accessibility_check(false)
                        .build_ui_state(move |ui, home: &mut Home| {
                            crate::theme::install(ui.ctx());
                            home.ui(ui, &recent, None);
                        }, home);
                    h.run_steps(3);
                }
            }
            let _ = done.send(());
        });
        let drawn = finished.recv_timeout(std::time::Duration::from_secs(60));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(drawn.is_ok(), "a frame of the home screen did not end");
    }

    /// The home screen's first frame waits on no work of rayon's global
    /// pool, which slow loads may hold: see the notes above `BusyPool`; the
    /// ages of the recent maps come later too.
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
            let done = home.waves.is_some() && home.new_map.preview.is_some() && home.ages.len() == 1;
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

    /// The size of a new map typed a key at a time: what shows is what was
    /// typed, and the map keeps its size, until it is done (Enter); then it
    /// is whole tiles, the nearest. Enter only ends the typing: the map is
    /// not made. Renders the field and its note with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn typing_a_size_waits_for_enter() {
        use egui_kittest::kittest::Queryable;
        use egui::accesskit::Role;
        let mut home = Home::new(massif_core::Registry::vanilla());
        home.in_frame = false;
        // The preview of the new map is drawn on a thread that asks for a
        // frame when done: on a busy machine that comes a few frames late.
        let mut h = egui_kittest::Harness::builder()
            .with_size(vec2(1280.0, 800.0))
            .with_pixels_per_point(std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|s| s.parse().ok()).unwrap_or(1.0))
            .with_accessibility_check(false)
            .with_max_steps(32)
            .wgpu()
            .build_ui_state(move |ui, (made, shown, home): &mut (bool, bool, Home)| {
                // The app's fonts first, as the app has them when it starts.
                if !*shown {
                    crate::theme::install(ui.ctx());
                    crate::theme::set_typeface(ui.ctx(), Default::default());
                    *shown = true;
                    return;
                }
                *made |= home.ui(ui, &[], None).is_some();
            }, (false, false, home));
        h.run();
        let snapshots = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from);
        for (typed, expected) in [("1024", 1024), ("1000", 1024), ("5000", 4992)] {
            let width = h.get_all_by_role(Role::SpinButton).next().expect("the width").rect();
            h.get_all_by_role(Role::SpinButton).next().unwrap().click();
            h.run();
            let before = h.state().2.new_map.width;
            let field = |h: &egui_kittest::Harness<'_, (bool, bool, Home)>| h.get_all_by_role(Role::TextInput).find(|n| n.rect().intersects(width)).map(|n| n.value().unwrap_or_default()).expect("the width typed in");
            h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
            h.run();
            for k in 1..=typed.len() {
                h.get_all_by_role(Role::TextInput).find(|n| n.rect().intersects(width)).unwrap().type_text(&typed[k - 1..k]);
                h.run();
                assert_eq!(field(&h), typed[..k], "the field shows what was typed");
                assert_eq!(h.state().2.new_map.width, before, "the size waits while {} is typed", &typed[..k]);
            }
            h.key_press(egui::Key::Enter);
            h.run();
            assert_eq!(h.state().2.new_map.width, expected, "{typed} became whole tiles");
            assert!(!h.state().0, "Enter ended the typing, and did not make the map");
            assert!(h.get_all_by_role(Role::TextInput).all(|n| !n.rect().intersects(width)), "no longer typed in");
            if let Some(dir) = &snapshots
                && typed == "5000"
            {
                std::fs::create_dir_all(dir).unwrap();
                h.render().unwrap().save(dir.join("new-map-size-snapped.png")).unwrap();
            }
        }
    }
}
