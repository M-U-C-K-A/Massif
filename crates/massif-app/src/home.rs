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
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// Width of the sidebar.
const SIDEBAR: f32 = 268.0;
/// Side of the preview of a new map.
const PREVIEW: f32 = 184.0;

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
    /// The picture of the template, and the one being drawn.
    template_preview: Option<(Template, TextureHandle)>,
    drawing: Option<(Template, std::sync::mpsc::Receiver<egui::ColorImage>)>,
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
            template_preview: None,
            drawing: None,
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
}

/// The dots behind the home screen, for one size and colour.
struct Backdrop {
    key: (usize, usize, [u8; 4], bool),
    cells: TextureHandle,
    grid: TextureHandle,
    pitch: usize,
    twinkles: Vec<Twinkle>,
}

/// A dot of the ridges that lights up now and then, each at its own pace.
#[derive(Debug, Clone, Copy)]
struct Twinkle {
    col: u16,
    row: u16,
    color: Color32,
    phase: f32,
    speed: f32,
}

pub struct Home {
    pub page: Page,
    pub new_map: NewMap,
    pub heightmap: Option<HeightmapPick>,
    /// The name of the map open before, to go back to.
    pub back_to: Option<String>,
    backdrop: Option<Backdrop>,
    logo: Option<TextureHandle>,
}

impl Home {
    pub fn new(registry: massif_core::Registry) -> Self {
        Self {
            page: Page::NewMap,
            new_map: NewMap::new(registry),
            heightmap: None,
            back_to: None,
            backdrop: None,
            logo: None,
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

        self.paint_backdrop(ui, main);
        ui.painter().rect_filled(sidebar, 0, sidebar_fill(&pal));
        ui.painter().vline(sidebar.max.x - 0.5, sidebar.y_range(), Stroke::new(1.0, pal.border));

        let mut side = ui.new_child(
            UiBuilder::new()
                .id_salt("home sidebar")
                .max_rect(sidebar.shrink2(vec2(12.0, 0.0)))
                .layout(egui::Layout::top_down(Align::Min)),
        );
        self.sidebar(&mut side, recent, &mut action);
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
                    let row = recent_row(ui, path);
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
            if nav(ui, Icon::Settings, "Settings", false).on_hover_text("Theme and accent colour (⌘,)").clicked() {
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
        let width = (area.width() - 96.0).clamp(420.0, 780.0);
        let top = area.top() + (area.height() * 0.12).max(44.0);
        let column = Rect::from_min_max(pos2(area.center().x - width / 2.0, top), pos2(area.center().x + width / 2.0, area.bottom() - 16.0));
        let mut ui = ui.new_child(UiBuilder::new().id_salt("home column").max_rect(column).layout(egui::Layout::top_down(Align::Center)));
        let ui = &mut ui;
        // A veil behind what is written, so that it reads over the dots;
        // laid once the column is known.
        let veil = ui.painter().add(egui::Shape::Noop);
        if let Some(logo) = &self.logo {
            ui.image((logo.id(), vec2(64.0, 64.0)));
        }
        ui.add_space(14.0);
        let (title, subtitle) = match self.page {
            Page::NewMap => ("What will you build?", "A new map: its size and its land, then make it."),
            Page::FromHeightmap => ("A map from a height map", "A greyscale picture from Gaea, World Machine or an image editor."),
        };
        ui.label(RichText::new(title).size(28.0).medium().color(pal.text));
        ui.add_space(2.0);
        ui.label(RichText::new(subtitle).size(13.0).color(pal.weak));
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
        let links: [(Icon, &str, HomeAction); 3] = [
            (Icon::FolderOpen, "Open a map", HomeAction::Open),
            (Icon::Import, "Import from WorldPainter", HomeAction::Import),
            (Icon::ImageUp, "From a height map", HomeAction::PickHeightmap),
        ];
        if let Some(a) = link_row(ui, &links) {
            if matches!(a, HomeAction::PickHeightmap) {
                self.page = Page::FromHeightmap;
            }
            *action = Some(a);
        }
        ui.add_space(6.0);
        ui.label(RichText::new("or drop a .massif, a .world or a height map on the window").size(11.5).color(pal.weak));
        let bg = backdrop_fill(&pal);
        let shade = egui::epaint::Shadow {
            offset: [0, 0],
            blur: 150,
            spread: 24,
            color: Color32::from_rgba_unmultiplied(bg.r(), bg.g(), bg.b(), if pal.dark { 242 } else { 232 }),
        };
        ui.painter().set(veil, shade.as_shape(ui.min_rect().expand2(vec2(40.0, 24.0)), 80));
    }

    fn new_map_card(&mut self, ui: &mut Ui, action: &mut Option<HomeAction>) {
        let pal = Palette::of(ui);
        let nm = &mut self.new_map;
        ui.horizontal_top(|ui| {
            let left = (ui.available_width() - PREVIEW - 18.0).max(260.0);
            ui.vertical(|ui| {
                ui.set_width(left);
                ui.add(
                    egui::TextEdit::singleline(&mut nm.params.name)
                        .hint_text("Name your map")
                        .font(FontId::proportional(17.0))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                ui.add_space(8.0);
                let r = ui.available_rect_before_wrap();
                ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
                ui.add_space(12.0);
                let third = ((left - 76.0 - 12.0) / 3.0).floor();
                setting(ui, "Size", |ui| {
                    input_group(
                        ui,
                        "home size",
                        &mut [Part::Int(&mut nm.width, 128..=65536, 64.0), Part::Text("×"), Part::Int(&mut nm.depth, 128..=65536, 64.0), Part::Text("blocks")],
                    );
                    // Whole tiles only.
                    nm.width = (nm.width + 127) / 128 * 128;
                    nm.depth = (nm.depth + 127) / 128 * 128;
                });
                setting(ui, "Height", |ui| limits(ui, &mut nm.params.min_y, &mut nm.params.max_y));
                setting(ui, "Land", |ui| {
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
                setting(ui, "Relief", |ui| {
                    ui.add_enabled(hilly, Field::new(&mut nm.params.relief, 0.0..=most).text("Height").suffix("%").max_decimals(0).width(third));
                    ui.add_enabled(hilly, Field::new(&mut nm.params.feature_size, 10.0..=1000.0).text("Features").suffix("%").max_decimals(0).width(third));
                    let mut level = nm.params.level.unwrap_or(if hilly { nm.params.sea_level as f32 } else { nm.params.flat_height });
                    let free = nm.template.is_none();
                    if ui.add_enabled(free, Field::new(&mut level, (bottom + 1) as f32..=(top - 2) as f32).text("Ground").max_decimals(0).width(third)).changed() {
                        nm.params.level = Some(level);
                        nm.params.flat_height = level;
                    }
                });
                setting(ui, "Water", |ui| {
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
                setting(ui, "Seed", |ui| {
                    let (_, dice) = input_group(ui, "home seed", &mut [Part::Edit(&mut nm.seed, third * 1.4), Part::Button(Icon::Dices, "Another seed")]);
                    if dice.is_some() {
                        nm.seed = random_seed().to_string();
                    }
                });
            });
            ui.add_space(18.0);
            ui.vertical(|ui| preview(ui, nm));
        });
        ui.add_space(12.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
        ui.add_space(10.0);
        let params = nm.params();
        ui.horizontal(|ui| {
            let tiles = params.tiles_x as i64 * params.tiles_z as i64;
            let minutes = (params.tiles_x.max(params.tiles_z) * 128) as f32 / 4.317 / 60.0;
            let across = if minutes < 1.0 { "under a minute across".to_owned() } else { format!("{minutes:.0} min. across on foot") };
            let (bottom, top) = massif_core::generate::build_limits(params.min_y, params.max_y);
            let pack = if (bottom, top) != massif_core::generate::VANILLA_LIMITS { " (with a data pack)" } else { "" };
            ui.label(
                RichText::new(format!(
                    "Minecraft Java 1.18+  ·  Y {bottom} to {top}{pack}  ·  {across}  ·  about {}",
                    crate::panels::human((tiles * 12_000) as u64)
                ))
                .size(11.5)
                .color(pal.weak),
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
            dashed_rect(ui.painter(), rect.shrink(1.0), if lit { pal.accent } else { theme::mix(pal.border, pal.text, 0.15) });
            if lit {
                ui.painter().rect_filled(rect.shrink(2.0), 12, pal.accent.gamma_multiply(0.06));
                ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
            }
            let c = rect.center();
            ui.painter().text(c - vec2(0.0, 28.0), Align2::CENTER_CENTER, theme::glyph(Icon::ImageUp).to_string(), FontId::proportional(28.0), if lit { pal.accent } else { pal.weak });
            ui.painter().text(c + vec2(0.0, 10.0), Align2::CENTER_CENTER, "Choose a height map", FontId::proportional(15.0), pal.text);
            ui.painter().text(c + vec2(0.0, 32.0), Align2::CENTER_CENTER, "16-bit PNG, JPEG, .r16 or .r32 — or drop one here", FontId::proportional(12.0), pal.weak);
            if response.clicked() {
                *action = Some(HomeAction::PickHeightmap);
            }
            return;
        };
        ui.horizontal_top(|ui| {
            if let Some(t) = &pick.picture {
                let size = t.size_vec2();
                let k = (PREVIEW / size.x).min(PREVIEW / size.y);
                let (rect, _) = ui.allocate_exact_size(vec2(PREVIEW, PREVIEW), Sense::hover());
                let shown = Rect::from_center_size(rect.center(), size * k);
                ui.painter().rect_filled(rect, 10, pal.canvas);
                ui.painter().image(t.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            ui.add_space(18.0);
            ui.vertical(|ui| {
                let h = &mut pick.how;
                ui.add(
                    egui::TextEdit::singleline(&mut h.name)
                        .hint_text("Name your map")
                        .font(FontId::proportional(17.0))
                        .frame(egui::Frame::NONE)
                        .desired_width(f32::INFINITY),
                );
                ui.label(RichText::new(pick.path.display().to_string()).size(11.0).color(pal.weak));
                ui.add_space(10.0);
                let w = ((ui.available_width() - 76.0 - 8.0) / 2.0).floor();
                setting(ui, "Height", |ui| limits(ui, &mut h.min_y, &mut h.max_y));
                let (bottom, top) = massif_core::generate::build_limits(h.min_y, h.max_y);
                setting(ui, "Scale", |ui| {
                    ui.add(Field::new(&mut h.scale, 0.25..=16.0).logarithmic(true).text("Blocks a pixel").max_decimals(2).width(w * 2.0 + 4.0));
                });
                setting(ui, "Heights", |ui| {
                    ui.add(Field::new(&mut h.low, bottom as f32..=(top - 1) as f32).text("Black is Y").max_decimals(0).width(w));
                    ui.add(Field::new(&mut h.high, bottom as f32..=(top - 1) as f32).text("White is Y").max_decimals(0).width(w));
                });
                setting(ui, "Water", |ui| {
                    ui.add(Field::new(&mut h.sea_level, bottom..=top - 1).text("Level").width(w));
                });
                ui.add_space(4.0);
                if ui.link(RichText::new("Choose another…").size(12.0)).clicked() {
                    *action = Some(HomeAction::PickHeightmap);
                }
            });
        });
        ui.add_space(12.0);
        let r = ui.available_rect_before_wrap();
        ui.painter().hline(r.x_range(), r.top(), Stroke::new(1.0, pal.border));
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            let (w, d) = (pick.map.width as f32 * pick.how.scale, pick.map.height as f32 * pick.how.scale);
            ui.label(
                RichText::new(format!("{} × {} pixels  →  {w:.0} × {d:.0} blocks, Y {:.0} to {:.0}", pick.map.width, pick.map.height, pick.how.low, pick.how.high))
                    .size(11.5)
                    .color(pal.weak),
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

    /// Ridges in dots behind `rect`, made again when its size or the
    /// accent colour changes.
    fn paint_backdrop(&mut self, ui: &Ui, rect: Rect) {
        let pal = Palette::of(ui);
        let bg = backdrop_fill(&pal);
        ui.painter().rect_filled(rect, 0, bg);
        let ppp = ui.ctx().pixels_per_point();
        let pitch = ((2.0 * ppp).round() as usize).max(3);
        let cols = ((rect.width() * ppp) as usize / pitch).max(1);
        let rows = ((rect.height() * ppp) as usize / pitch).max(1);
        let key = (cols, rows, pal.accent.to_array(), pal.dark);
        if self.backdrop.as_ref().is_none_or(|b| b.key != key || b.pitch != pitch) {
            let nearest = egui::TextureOptions::NEAREST;
            let (image, twinkles) = ridges(cols, rows, pal.accent, pal.dark, bg);
            let cells = ui.ctx().load_texture("home backdrop", image, nearest);
            let grid = ui.ctx().load_texture(
                "home dots",
                dots(pitch, bg),
                egui::TextureOptions {
                    wrap_mode: egui::TextureWrapMode::Repeat,
                    ..nearest
                },
            );
            self.backdrop = Some(Backdrop {
                key,
                cells,
                grid,
                pitch,
                twinkles,
            });
        }
        if let Some(b) = &self.backdrop {
            let shown = Rect::from_min_size(rect.min, vec2(cols as f32, rows as f32) * pitch as f32 / ppp);
            let painter = ui.painter_at(rect);
            painter.image(b.cells.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            painter.image(b.grid.id(), shown, Rect::from_min_max(pos2(0.0, 0.0), pos2(cols as f32, rows as f32)), Color32::WHITE);
            // Now and then a dot of the ridges lights up: barely, a few at
            // a time.
            let t = ui.input(|i| i.time) as f32;
            let cell = pitch as f32 / ppp;
            let dot = (pitch - (pitch / 3).max(1)) as f32 / ppp;
            for tw in &b.twinkles {
                let a = (0.5 + 0.5 * (t * tw.speed + tw.phase).sin()).powi(30);
                if a < 0.03 {
                    continue;
                }
                let r = Rect::from_min_size(shown.min + vec2(tw.col as f32 * cell, tw.row as f32 * cell), vec2(dot, dot));
                let c = if pal.dark { theme::mix(tw.color, Color32::WHITE, 0.55) } else { theme::mix(tw.color, pal.accent, 0.6) };
                painter.rect_filled(r, 0, Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), (a * 170.0) as u8));
            }
            ui.ctx().request_repaint_after(std::time::Duration::from_millis(66));
        }
    }
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
fn recent_row(ui: &mut Ui, path: &Path) -> Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &stem));
    if response.hovered() {
        ui.painter().rect_filled(rect, 8, theme::mix(sidebar_fill(&pal), pal.text, 0.05));
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let (age, ink) = match modified {
        Some(t) => (ago(t), theme::mix(pal.text, pal.weak, 0.2)),
        None => ("missing".to_owned(), pal.weak),
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

/// A setting of the new map: its name on the left, its controls after.
fn setting(ui: &mut Ui, name: &str, add: impl FnOnce(&mut Ui)) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(6.0, 0.0);
        ui.add_sized([70.0, 24.0], egui::Label::new(RichText::new(name).size(12.5).color(pal.weak)));
        add(ui);
    });
    ui.add_space(8.0);
}

/// The build limits of a new map: the top among Minecraft's own and taller
/// ones, the bottom as deep as it may go.
fn limits(ui: &mut Ui, min_y: &mut i32, max_y: &mut i32) {
    const TOPS: [i32; 4] = [320, 512, 1024, 2032];
    // The most Minecraft allows: 4064 blocks, from −2032 to 2032.
    let (deepest, highest) = massif_core::generate::MAX_LIMITS;
    let full = *min_y == deepest && *max_y == highest;
    let mut on: Vec<bool> = TOPS.iter().map(|t| *max_y == *t && !full).collect();
    on.push(full);
    match button_group(ui, "build limits", &["320", "512", "1024", "2032", "4064"], &on) {
        Some(4) => {
            *min_y = deepest;
            *max_y = highest;
        }
        Some(k) => {
            if full {
                *min_y = massif_core::generate::VANILLA_LIMITS.0;
            }
            *max_y = TOPS[k];
        }
        None => {}
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
    let (rect, response) = ui.allocate_exact_size(vec2(label.size().x + hint.size().x + 34.0, 34.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, text));
    let fill = if response.hovered() { theme::mix(accent, Color32::WHITE, 0.12) } else { accent };
    ui.painter().rect_filled(rect, 9, fill);
    let y = rect.center().y;
    ui.painter().galley(pos2(rect.min.x + 14.0, y - label.size().y / 2.0), label, ink);
    ui.painter().galley(pos2(rect.max.x - 14.0 - hint.size().x, y - hint.size().y / 2.0), hint, ink);
    if response.hovered() {
        ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
    }
    response
}

/// Links in a row, centred: an icon and a name each.
fn link_row(ui: &mut Ui, links: &[(Icon, &str, HomeAction)]) -> Option<HomeAction> {
    let pal = Palette::of(ui);
    let font = FontId::proportional(12.5);
    let gap = 22.0;
    let galleys: Vec<_> = links
        .iter()
        .map(|(icon, name, _)| ui.painter().layout_no_wrap(format!("{}  {name}", theme::glyph(*icon)), font.clone(), pal.weak))
        .collect();
    let total: f32 = galleys.iter().map(|g| g.size().x).sum::<f32>() + gap * (galleys.len() - 1) as f32;
    let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::hover());
    let mut x = row.center().x - total / 2.0;
    let mut chosen = None;
    for (k, ((_, name, act), galley)) in links.iter().zip(galleys).enumerate() {
        let rect = Rect::from_min_size(pos2(x, row.center().y - galley.size().y / 2.0), galley.size());
        let response = ui.interact(rect.expand(3.0), egui::Id::unique(("home link", k)), Sense::click()).on_hover_text(*name);
        response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Link, true, *name));
        let ink = if response.hovered() { pal.accent } else { theme::mix(pal.text, pal.weak, 0.45) };
        ui.painter().galley_with_override_text_color(rect.min, galley, ink);
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if response.clicked() {
            chosen = Some(act.clone());
        }
        x += rect.width() + gap;
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

/// The preview of a new map: its colours and relief from above.
fn preview(ui: &mut Ui, nm: &mut NewMap) {
    if let Some(t) = nm.template {
        template_preview(ui, nm, t);
        return;
    }
    let pal = Palette::of(ui);
    let params = nm.params();
    let key = format!("{params:?}");
    if nm.preview.as_ref().is_none_or(|(k, _)| *k != key) {
        let image = preview_image(&params, &nm.registry, 180);
        let texture = ui.ctx().load_texture("new map preview", image, egui::TextureOptions::LINEAR);
        nm.preview = Some((key, texture));
    }
    let (rect, _) = ui.allocate_exact_size(vec2(PREVIEW, PREVIEW), Sense::hover());
    let shadow = egui::epaint::Shadow {
        offset: [0, 10],
        blur: 34,
        spread: 0,
        color: Color32::from_black_alpha(if pal.dark { 170 } else { 60 }),
    };
    ui.painter().add(shadow.as_shape(rect, 10));
    ui.painter().rect_filled(rect, 10, pal.canvas);
    if let Some((_, texture)) = &nm.preview {
        // Maps that are not square keep their proportions.
        let (w, d) = (nm.width as f32, nm.depth as f32);
        let k = PREVIEW / w.max(d);
        let shown = Rect::from_center_size(rect.center(), vec2(w * k, d * k));
        let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2(w / w.max(d), d / w.max(d)));
        ui.painter().image(texture.id(), shown, uv, Color32::WHITE);
        ui.painter().rect_stroke(shown, 2, Stroke::new(1.0, Color32::from_white_alpha(28)), StrokeKind::Inside);
    }
    let c = rect.center();
    ui.painter().line_segment([c - vec2(5.0, 0.0), c + vec2(5.0, 0.0)], Stroke::new(1.0, Color32::WHITE));
    ui.painter().line_segment([c - vec2(0.0, 5.0), c + vec2(0.0, 5.0)], Stroke::new(1.0, Color32::WHITE));
    ui.add_space(4.0);
    ui.label(RichText::new("The whole map; the cross is its centre.").size(11.0).color(pal.weak));
}

/// The picture of what a template makes, drawn on another thread.
fn template_preview(ui: &mut Ui, nm: &mut NewMap, t: Template) {
    let pal = Palette::of(ui);
    if let Some((drawn, rx)) = &nm.drawing {
        match rx.try_recv() {
            Ok(image) => {
                let texture = ui.ctx().load_texture("template preview", image, egui::TextureOptions::LINEAR);
                nm.template_preview = Some((*drawn, texture));
                nm.drawing = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => nm.drawing = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    let shown = nm.template_preview.as_ref().filter(|p| p.0 == t).map(|p| p.1.id());
    if shown.is_none() && nm.drawing.as_ref().is_none_or(|d| d.0 != t) {
        let (tx, rx) = std::sync::mpsc::channel();
        let reg = nm.registry.clone();
        let sea = nm.params.sea_level as f32;
        let side = nm.width.max(nm.depth) as f64;
        let ctx = ui.ctx().clone();
        let spawned = std::thread::Builder::new().name("massif home template".into()).spawn(move || {
            let extent = massif_core::graph::Extent {
                x0: -side / 2.0,
                z0: -side / 2.0,
                size: side,
                n: 112,
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
    let (rect, _) = ui.allocate_exact_size(vec2(PREVIEW, PREVIEW), Sense::hover());
    ui.painter().rect_filled(rect, 10, pal.canvas);
    match shown {
        Some(id) => {
            ui.painter().image(id, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.painter().rect_stroke(rect, 2, Stroke::new(1.0, Color32::from_white_alpha(28)), StrokeKind::Inside);
        }
        None => {
            ui.put(Rect::from_center_size(rect.center(), vec2(24.0, 24.0)), egui::Spinner::new());
        }
    }
    ui.add_space(4.0);
    ui.label(RichText::new("Made on the Terrain side, from its nodes.").size(11.0).color(pal.weak));
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

/// The ridges behind the home screen, one pixel a dot: layers of mountains
/// from far (high on the screen) to near, their crests bright, their sides
/// fading into the dark, a sun going down behind them in the accent colour.
/// Halftoned with an ordered dither, as an ASCII picture would be.
fn ridges(cols: usize, rows: usize, accent: Color32, dark: bool, bg: Color32) -> (egui::ColorImage, Vec<Twinkle>) {
    const LAYERS: usize = 8;
    let noise = massif_core::noise::Noise::new(0x6d61_7373);
    let aspect = cols as f32 / rows.max(1) as f32;
    let crests: Vec<Vec<f32>> = (0..LAYERS)
        .map(|k| {
            let kf = k as f32;
            let base = 0.53 + 0.06 * kf;
            let amp = 0.075 + 0.02 * kf;
            let freq = 1.5 + 0.3 * kf as f64;
            (0..cols)
                .map(|i| {
                    let u = (i as f64 + 0.5) / cols as f64 * aspect as f64 / 1.7;
                    let r = noise.ridged(u * freq + kf as f64 * 7.31, kf as f64 * 3.7, 5, 2.0, 0.5) as f32;
                    let f = 0.5 + 0.5 * noise.fbm(u * freq * 0.45 + 31.0, kf as f64 * 1.9, 3, 2.0, 0.5) as f32;
                    base - amp * (0.6 * r + 0.55 * f)
                })
                .collect()
        })
        .collect();
    // The sun, right of the middle, just over the far ridges.
    let (gx, gy, gr) = (0.80f32, 0.52f32, 0.34f32);
    const BAYER: [[f32; 4]; 4] = [[0.0, 8.0, 2.0, 10.0], [12.0, 4.0, 14.0, 6.0], [3.0, 11.0, 1.0, 9.0], [15.0, 7.0, 13.0, 5.0]];
    let ink = if dark { Color32::from_rgb(196, 198, 204) } else { Color32::from_rgb(60, 62, 70) };
    let mut pixels = vec![bg; cols * rows];
    let mut twinkles = Vec::new();
    for (j, row) in pixels.chunks_mut(cols).enumerate() {
        let v = (j as f32 + 0.5) / rows as f32;
        for (i, px) in row.iter_mut().enumerate() {
            let u = (i as f32 + 0.5) / cols as f32;
            let d2 = ((u - gx) * aspect).powi(2) + (v - gy).powi(2);
            let glow = (-d2 / (gr * gr)).exp();
            let b = match (0..LAYERS).rev().find(|&k| crests[k][i] <= v) {
                // The sky: only the haze of the sun.
                None => glow * 0.8 * (1.0 - (gy - v).max(0.0) * 1.7).clamp(0.0, 1.0),
                Some(k) => {
                    let near = k as f32 / (LAYERS - 1) as f32;
                    let depth = v - crests[k][i];
                    let crest = (1.0 - depth / (0.004 + 0.004 * near)).clamp(0.0, 1.0);
                    let side = 0.3 * (-depth / (0.028 + 0.03 * near)).exp() + 0.035;
                    crest.max(side) * (0.5 + 0.5 * near) + glow * (0.5 * crest + 0.24)
                }
            };
            if b <= BAYER[j % 4][i % 4] / 16.0 * 0.5 + 0.02 {
                continue;
            }
            let b = b.min(1.0);
            let tint = (glow * 2.2).min(1.0);
            let colour = theme::mix(ink, accent, tint);
            *px = if dark { theme::scale(colour, b) } else { theme::mix(bg, colour, b * 0.85) };
            // A few of the brighter dots twinkle.
            let hash = (i as u32).wrapping_mul(0x9e37_79b1) ^ (j as u32).wrapping_mul(0x85eb_ca77);
            let hash = hash ^ (hash >> 15);
            if b > 0.3 && hash.is_multiple_of(7) && twinkles.len() < 1600 {
                twinkles.push(Twinkle {
                    col: i as u16,
                    row: j as u16,
                    color: colour,
                    phase: (hash >> 8) as f32 / (1u32 << 24) as f32 * std::f32::consts::TAU,
                    speed: 0.45 + 0.9 * ((hash >> 3) & 0xff) as f32 / 255.0,
                });
            }
        }
    }
    (egui::ColorImage::new([cols, rows], pixels), twinkles)
}

/// One dot of the backdrop: the dot clear, the gap around it in the
/// background colour; tiled over the picture of the ridges.
fn dots(pitch: usize, bg: Color32) -> egui::ColorImage {
    let dot = pitch - (pitch / 3).max(1);
    let pixels = (0..pitch * pitch).map(|k| if k % pitch < dot && k / pitch < dot { Color32::TRANSPARENT } else { bg }).collect();
    egui::ColorImage::new([pitch, pitch], pixels)
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
