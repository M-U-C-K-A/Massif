//! A map, or a part of one, from the relief of a real place: the dialog.
//! Where on the Earth (a map of tiles to drag, a place to search, or
//! coordinates and bounds typed), at what scale, how high; a preview of
//! what the ground will be. The work itself is a job (`Job::real`).

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tiles::{Place, TileStore, parse_coords};
use crate::widgets::{Field, button_group, row, switch, well};
use egui::{Color32, ColorImage, Rect, RichText, Sense, TextureHandle, TextureOptions, Ui, pos2, vec2};
use massif_core::coords::BlockRect;
use massif_core::heightmap::Heightmap;
use massif_core::real::{Area, Bounds, DemTile, Elevation, Fit, Grid, MAX_SCALE, MAX_ZOOM, MIN_SCALE, Samples, Span, TILE_PX, TileKey, Tiles, Vertical, metres_per_px, to_px, from_px};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::Receiver;

/// What the relief goes into.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Target {
    /// A new world.
    NewWorld,
    /// The map open: over `rect` (the selection's bounds, or the map's).
    Map { rect: BlockRect, selected: bool, min_y: i32, max_y: i32 },
}

/// A height file the user downloaded.
#[derive(Clone)]
pub struct FileSource {
    pub path: PathBuf,
    pub map: Arc<Heightmap>,
    /// A TIFF's samples are metres; a picture's run from 0 to 1, from
    /// `low` to `high` metres.
    pub tiff: bool,
    pub metres_per_px: f64,
    pub low: f32,
    pub high: f32,
    pub nodata: Option<f32>,
}

impl FileSource {
    pub fn read(path: PathBuf) -> Result<Self, String> {
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
        if ext == "tif" || ext == "tiff" {
            let t = massif_core::real::read_tiff(&path)?;
            Ok(Self {
                path,
                map: Arc::new(t.map),
                tiff: true,
                metres_per_px: t.metres_per_px.unwrap_or(30.0),
                low: 0.0,
                high: 1.0,
                nodata: t.nodata,
            })
        } else {
            let map = massif_core::heightmap::read(&path)?;
            Ok(Self {
                path,
                map: Arc::new(map),
                tiff: false,
                metres_per_px: 30.0,
                low: 0.0,
                high: 1000.0,
                nodata: None,
            })
        }
    }

    pub fn grid(&self) -> Grid {
        Grid {
            map: self.map.clone(),
            metres_per_px: self.metres_per_px,
            low: self.low,
            high: self.high,
            nodata: self.nodata,
        }
    }
}

/// Where the heights come from.
#[derive(Clone)]
pub enum Source {
    Online,
    File(FileSource),
}

/// What to make, for the job.
#[derive(Clone)]
pub struct RealJob {
    pub name: String,
    pub area: Area,
    pub vertical: Vertical,
    pub source: Source,
    pub target: Target,
}

impl std::fmt::Debug for RealJob {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RealJob").field("name", &self.name).field("area", &self.area).field("target", &self.target).finish_non_exhaustive()
    }
}

/// The settings kept from one opening of the dialog to the next.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Remembered {
    pub area: Area,
    pub vertical: Vertical,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    Make(Box<RealJob>),
}

/// Samples a side of the preview.
const PREVIEW_N: usize = 160;
/// The map of tiles and the preview, in points.
const PICKER: egui::Vec2 = vec2(330.0, 230.0);
const PREVIEW: egui::Vec2 = vec2(330.0, 190.0);
/// Tiles of elevation the preview reads at most.
const PREVIEW_TILES: usize = 16;

struct Picture {
    made_for: String,
    texture: TextureHandle,
    span: Option<Span>,
    fit: Fit,
    complete: bool,
}

pub struct RealForm {
    pub target: Target,
    pub name: String,
    pub area: Area,
    pub vertical: Vertical,
    pub file: Option<FileSource>,
    pub online: bool,
    query: String,
    places: Vec<Place>,
    searching: Option<Receiver<Result<Vec<Place>, String>>>,
    note: Option<String>,
    /// The map of tiles: its zoom, and the metres it was fitted to.
    view_zoom: f64,
    fitted_for: (f64, f64),
    textures: HashMap<TileKey, TextureHandle>,
    picture: Option<Picture>,
    sources: bool,
    store: Arc<TileStore>,
}

impl RealForm {
    pub fn new(target: Target, sea_level: i32, kept: Option<Remembered>, store: Arc<TileStore>) -> Self {
        let mut area = kept.map_or(Area { lat: 45.8326, lon: 6.8652, blocks_per_metre: 0.5, width: 2048, depth: 2048 }, |k| k.area);
        let mut vertical = kept.map_or(Vertical { datum: 0.0, sea_level, exaggeration: 1.0, low: -64, high: 320, fit: true }, |k| k.vertical);
        if let Target::Map { rect, min_y, max_y, .. } = target {
            area.width = rect.width().max(1);
            area.depth = rect.depth().max(1);
            vertical.sea_level = sea_level;
            vertical.low = vertical.low.clamp(min_y, max_y - 2);
            vertical.high = vertical.high.clamp(vertical.low + 2, max_y);
        }
        Self {
            target,
            name: String::new(),
            area,
            vertical,
            file: None,
            online: true,
            query: String::new(),
            places: Vec::new(),
            searching: None,
            note: None,
            view_zoom: 10.0,
            fitted_for: (0.0, 0.0),
            textures: HashMap::new(),
            picture: None,
            sources: false,
            store,
        }
    }

    pub fn remembered(&self) -> Remembered {
        Remembered { area: self.area, vertical: self.vertical }
    }

    fn new_world(&self) -> bool {
        self.target == Target::NewWorld
    }

    /// The heights the ground may take, `low..high`.
    fn limits(&self) -> (i32, i32) {
        match self.target {
            Target::NewWorld => massif_core::generate::MAX_LIMITS,
            Target::Map { min_y, max_y, .. } => (min_y, max_y),
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(720.0);
        self.take_search();
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Earth).to_string()).size(17.0).color(pal.accent));
            let title = if self.new_world() { "A new world from a real place" } else { "Relief of a real place" };
            ui.label(RichText::new(title).size(16.0).bold());
        });
        ui.add(egui::Label::new(RichText::new("The heights of a place on the Earth, ocean floors included, at the scale chosen: the ground, the sea and the automatic paint.").size(12.0).color(pal.weak)).wrap());
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(350.0);
                // As tall as the window lets the dialog be.
                let tall = (ui.ctx().content_rect().height() - 330.0).clamp(300.0, 640.0);
                egui::ScrollArea::vertical().max_height(tall).auto_shrink([false, true]).show(ui, |ui| self.settings(ui));
            });
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.set_width(PICKER.x);
                if self.online {
                    ui.label(RichText::new("Where").size(11.5).color(pal.weak).medium());
                    ui.add_space(2.0);
                    self.picker(ui);
                    let hint = if self.store.offline() { "Offline: only the places downloaded before show." } else { "Drag to move, scroll to zoom." };
                    ui.label(RichText::new(hint).size(11.0).color(pal.weak));
                    ui.add_space(6.0);
                }
                ui.label(RichText::new("The ground it makes").size(11.5).color(pal.weak).medium());
                ui.add_space(2.0);
                self.preview(ui);
            });
        });
        ui.add_space(10.0);
        self.credits(ui);
        ui.add_space(6.0);
        let (w, d) = (self.area.width, self.area.depth);
        let over = match self.target {
            Target::NewWorld => format!("A world of {w} × {d} blocks, centred on the origin, its terrains painted by the automatic paint."),
            Target::Map { selected: true, .. } => format!("Into the selection, by its weight ({w} × {d} blocks): its heights, its sea and the automatic paint; one step to undo."),
            Target::Map { .. } => format!("Over the whole map ({w} × {d} blocks): its heights, its sea and the automatic paint; one step to undo."),
        };
        ui.label(RichText::new(over).size(11.5).color(pal.weak));
        ui.add_space(10.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ready = self.online || self.file.is_some();
            let label = if self.new_world() { "Make the world" } else { "Apply" };
            if ui.add_enabled(ready, theme::primary_button(ui, label)).clicked() {
                choice = Choice::Make(Box::new(self.job()));
            }
            if ui.button("Cancel").clicked() {
                choice = Choice::Close;
            }
        });
        if !matches!(choice, Choice::Keep) {
            self.store.forget_queue();
        }
        choice
    }

    pub fn job(&self) -> RealJob {
        let name = if self.name.trim().is_empty() { "Real place".to_owned() } else { self.name.trim().to_owned() };
        RealJob {
            name,
            area: self.area,
            vertical: self.vertical,
            source: match (&self.file, self.online) {
                (Some(f), false) => Source::File(f.clone()),
                _ => Source::Online,
            },
            target: self.target,
        }
    }

    fn settings(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let head = |ui: &mut Ui, text: &str| {
            ui.label(RichText::new(text).size(11.5).color(pal.weak).medium());
            ui.add_space(4.0);
        };
        head(ui, "Heights from");
        well(ui, |ui| {
            row(ui, "Source", |ui| {
                if let Some(k) = button_group(ui, "real source", &["Terrain Tiles", "A file"], &[self.online, !self.online]) {
                    self.online = k == 0;
                    self.picture = None;
                }
            });
            if !self.online {
                self.file_rows(ui);
            }
        });
        ui.add_space(10.0);
        if self.online {
            head(ui, "Place");
            well(ui, |ui| self.place_rows(ui));
            ui.add_space(10.0);
        }
        head(ui, "Size and scale");
        well(ui, |ui| self.scale_rows(ui));
        ui.add_space(10.0);
        head(ui, "Heights");
        well(ui, |ui| self.height_rows(ui));
    }

    fn file_rows(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        row(ui, "File", |ui| {
            let name = self.file.as_ref().map_or("None".to_owned(), |f| crate::jobs::display_name(&f.path));
            if ui.button(theme::with_icon(Icon::FolderOpen, "Choose…")).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title("A file of heights")
                    .add_filter("Heights", &["tif", "tiff", "png", "r16", "r32", "raw"])
                    .pick_file()
            {
                match FileSource::read(path) {
                    Ok(f) => {
                        self.file = Some(f);
                        self.note = None;
                        self.picture = None;
                        if self.new_world() {
                            self.fit_file();
                        }
                    }
                    Err(e) => self.note = Some(format!("Could not read the file: {e}")),
                }
            }
            ui.label(RichText::new(name).size(12.0).color(pal.weak));
        });
        let new_world = self.new_world();
        if let Some(f) = &mut self.file {
            row(ui, "Metres a pixel", |ui| {
                ui.add(Field::new(&mut f.metres_per_px, 0.1..=10_000.0).logarithmic(true).max_decimals(2).suffix(" m"));
            });
            if !f.tiff {
                row(ui, "Black is", |ui| {
                    ui.add(Field::new(&mut f.low, -11_000.0..=9_000.0).max_decimals(0).suffix(" m"));
                });
                row(ui, "White is", |ui| {
                    ui.add(Field::new(&mut f.high, -11_000.0..=9_000.0).max_decimals(0).suffix(" m"));
                });
            }
            let (e, s) = (f.map.width as f64 * f.metres_per_px, f.map.height as f64 * f.metres_per_px);
            ui.label(RichText::new(format!("{} × {} pixels, {} × {}", f.map.width, f.map.height, distance(e), distance(s))).size(11.0).color(pal.weak));
            if new_world && ui.small_button("Size the world to the file").clicked() {
                self.fit_file();
            }
        }
        if let Some(note) = &self.note {
            ui.label(RichText::new(note).size(11.5).color(pal.danger));
        }
    }

    /// A new world as large as the file at the scale.
    fn fit_file(&mut self) {
        if let Some(f) = &self.file {
            let (e, s) = (f.map.width as f64 * f.metres_per_px, f.map.height as f64 * f.metres_per_px);
            self.area.width = (e * self.area.blocks_per_metre).round().clamp(128.0, 16_384.0) as i32;
            self.area.depth = (s * self.area.blocks_per_metre).round().clamp(128.0, 16_384.0) as i32;
        }
    }

    fn place_rows(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(&mut self.query).hint_text("A place, or latitude, longitude").desired_width(ui.available_width() - 34.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let busy = self.searching.is_some();
            if busy {
                ui.add(egui::Spinner::new().size(14.0));
            } else if (ui.small_button(theme::glyph(Icon::Search).to_string()).on_hover_text("Search").clicked() || enter) && !self.query.trim().is_empty() {
                self.search();
            }
        });
        if !self.places.is_empty() {
            let mut picked = None;
            for (k, p) in self.places.iter().enumerate() {
                let short: String = p.name.chars().take(60).collect();
                if ui.add(egui::Button::new(RichText::new(short).size(12.0)).frame(false)).on_hover_text(&p.name).clicked() {
                    picked = Some(k);
                }
            }
            if let Some(k) = picked {
                let p = self.places.remove(k);
                self.go_to(p.lat, p.lon);
                if self.name.is_empty() {
                    self.name = p.name.split(',').next().unwrap_or("").trim().to_owned();
                }
                self.places.clear();
            }
        }
        if let Some(note) = &self.note {
            ui.label(RichText::new(note).size(11.5).color(pal.weak));
        }
        row(ui, "Latitude", |ui| {
            ui.add(Field::new(&mut self.area.lat, -85.0..=85.0).max_decimals(5).suffix("°"));
        });
        row(ui, "Longitude", |ui| {
            ui.add(Field::new(&mut self.area.lon, -180.0..=180.0).max_decimals(5).suffix("°"));
        });
        // The bounds, to read or to type: the area follows them.
        let b = self.area.bounds();
        let mut nb = b;
        row(ui, "North, south", |ui| {
            ui.add(Field::new(&mut nb.south, -85.0..=85.0).max_decimals(4).width(76.0).suffix("°"));
            ui.add(Field::new(&mut nb.north, -85.0..=85.0).max_decimals(4).width(76.0).suffix("°"));
        });
        row(ui, "West, east", |ui| {
            ui.add(Field::new(&mut nb.east, -180.0..=180.0).max_decimals(4).width(76.0).suffix("°"));
            ui.add(Field::new(&mut nb.west, -180.0..=180.0).max_decimals(4).width(76.0).suffix("°"));
        });
        if nb != b && nb.north > nb.south {
            self.set_bounds(nb);
        }
    }

    fn set_bounds(&mut self, b: Bounds) {
        let keep_size = !self.new_world();
        let mut area = self.area.fit_bounds(b, keep_size);
        area.width = area.width.clamp(128, 16_384);
        area.depth = area.depth.clamp(128, 16_384);
        self.area = area;
    }

    fn go_to(&mut self, lat: f64, lon: f64) {
        self.area.lat = lat.clamp(-85.0, 85.0);
        self.area.lon = lon;
        self.picture = None;
    }

    fn search(&mut self) {
        if let Some((lat, lon)) = parse_coords(&self.query) {
            self.go_to(lat, lon);
            self.note = None;
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let query = self.query.trim().to_owned();
        self.searching = Some(rx);
        self.note = None;
        let _ = std::thread::Builder::new().name("massif place search".into()).spawn(move || {
            let _ = tx.send(crate::tiles::search(&query));
        });
    }

    fn take_search(&mut self) {
        let Some(rx) = &self.searching else { return };
        match rx.try_recv() {
            Ok(Ok(places)) => {
                self.searching = None;
                self.note = places.is_empty().then(|| "No place of that name.".to_owned());
                if places.len() == 1 {
                    let p = &places[0];
                    self.go_to(p.lat, p.lon);
                    if self.name.is_empty() {
                        self.name = p.name.split(',').next().unwrap_or("").trim().to_owned();
                    }
                } else {
                    self.places = places;
                }
            }
            Ok(Err(e)) => {
                self.searching = None;
                self.note = Some(format!("The search did not answer ({e}): type the latitude and longitude instead."));
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
            Err(_) => self.searching = None,
        }
    }

    fn scale_rows(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        if self.new_world() {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Name").size(12.5));
                ui.add(egui::TextEdit::singleline(&mut self.name).hint_text("Real place").desired_width(f32::INFINITY));
            });
            row(ui, "Width", |ui| {
                ui.add(Field::new(&mut self.area.width, 128..=16_384).suffix(" blocks"));
            });
            row(ui, "Depth", |ui| {
                ui.add(Field::new(&mut self.area.depth, 128..=16_384).suffix(" blocks"));
            });
        }
        row(ui, "Blocks a metre", |ui| {
            ui.add(Field::new(&mut self.area.blocks_per_metre, MIN_SCALE..=MAX_SCALE).logarithmic(true).max_decimals(3));
        });
        let block = 1.0 / self.area.blocks_per_metre;
        let (e, s) = self.area.metres();
        ui.label(RichText::new(format!("A block is {}; the map covers {} × {}.", distance(block), distance(e), distance(s))).size(11.0).color(pal.weak));
    }

    fn height_rows(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let (lo, hi) = self.limits();
        let v = &mut self.vertical;
        row(ui, "Exaggeration", |ui| {
            ui.add(Field::new(&mut v.exaggeration, 0.1..=10.0).max_decimals(2).suffix("×")).on_hover_text("How many times as high the relief stands as at the scale across");
        });
        row(ui, "Sea level", |ui| {
            ui.add(Field::new(&mut v.sea_level, v.low + 1..=v.high - 1).suffix(" y"));
        });
        row(ui, "Real height there", |ui| {
            ui.add(Field::new(&mut v.datum, -500.0..=6_000.0).max_decimals(0).suffix(" m")).on_hover_text("The real height that stands at the sea level: 0 for the sea's; higher for a place inland, its valleys then under water below it");
        });
        row(ui, "Range", |ui| {
            let mc = massif_core::generate::VANILLA_LIMITS;
            let (a, b) = match self.target {
                Target::NewWorld => (("Minecraft", mc), ("Tall", (-64, 1024))),
                Target::Map { .. } => (("Minecraft", (mc.0.max(lo), mc.1.min(hi))), ("The map's", (lo, hi))),
            };
            let on = [(v.low, v.high) == a.1, (v.low, v.high) == b.1];
            if let Some(k) = button_group(ui, "real range", &[a.0, b.0], &on) {
                (v.low, v.high) = if k == 0 { a.1 } else { b.1 };
            }
        });
        row(ui, "Lowest", |ui| {
            ui.add(Field::new(&mut v.low, lo..=hi - 2).suffix(" y"));
        });
        row(ui, "Highest", |ui| {
            ui.add(Field::new(&mut v.high, lo + 2..=hi).suffix(" y"));
        });
        row(ui, "Squeeze into the range", |ui| {
            switch(ui, &mut v.fit).on_hover_text("Heights past the range brought into it, above and under the sea apart, rather than cut off");
        });
        if v.high < v.low + 2 {
            v.high = v.low + 2;
        }
        v.sea_level = v.sea_level.clamp(v.low + 1, v.high - 1);
        if let Some(p) = &self.picture
            && let Some(span) = p.span
        {
            let (a, b) = (p.fit.height(span.low), p.fit.height(span.high));
            let cut = !v.fit && (span.high - v.datum) * self.area.blocks_per_metre as f32 * v.exaggeration > (v.high - 1 - v.sea_level) as f32;
            let tail = if cut { ", cut at the top" } else { "" };
            ui.label(RichText::new(format!("From {:.0} m to {:.0} m: y {:.0} to {:.0}{tail}.", span.low, span.high, a, b)).size(11.0).color(pal.weak));
        }
    }

    fn credits(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            let lead = if self.online { "Elevation: Terrain Tiles by Mapzen on AWS Open Data, from SRTM, GMTED2010, ETOPO1, 3DEP, EU-DEM and others; place search: Nominatim, © OpenStreetMap contributors." } else { "Heights from your file." };
            ui.label(RichText::new(lead).size(11.0).color(pal.weak));
            if self.online && ui.add(egui::Button::new(RichText::new(if self.sources { "Hide the sources" } else { "All the sources" }).size(11.0).color(pal.accent)).frame(false)).clicked() {
                self.sources = !self.sources;
            }
        });
        if self.sources && self.online {
            egui::ScrollArea::vertical().id_salt("real credits").max_height(90.0).show(ui, |ui| {
                for line in massif_core::real::ATTRIBUTION {
                    ui.add(egui::Label::new(RichText::new(format!("• {line}")).size(10.5).color(pal.weak)).wrap());
                }
            });
        }
    }

    /// The map of tiles, the area outlined on it.
    fn picker(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let metres = self.area.metres();
        if metres != self.fitted_for {
            // The area about half the width of the map.
            let wanted = metres.0.max(metres.1 * PICKER.x as f64 / PICKER.y as f64) / (0.55 * PICKER.x as f64);
            let at0 = metres_per_px(self.area.lat, 0);
            self.view_zoom = (at0 / wanted.max(1e-3)).log2().clamp(1.0, MAX_ZOOM as f64 + 0.99);
            self.fitted_for = metres;
        }
        let (rect, response) = ui.allocate_exact_size(PICKER, Sense::click_and_drag());
        let z = (self.view_zoom.floor() as u8).min(MAX_ZOOM);
        let k = 2f64.powf(self.view_zoom - z as f64);
        let (cx, cy) = to_px(self.area.lat, self.area.lon, z);
        if response.dragged() {
            let d = response.drag_delta();
            let (lat, lon) = from_px(cx - d.x as f64 / k, cy - d.y as f64 / k, z);
            self.area.lat = lat.clamp(-85.0, 85.0);
            self.area.lon = (lon + 180.0).rem_euclid(360.0) - 180.0;
        }
        if response.hovered() {
            let scroll = ui.input(|i| i.smooth_scroll_delta.y + i.zoom_delta().log2() * 200.0);
            if scroll != 0.0 {
                self.view_zoom = (self.view_zoom + scroll as f64 / 200.0).clamp(1.0, MAX_ZOOM as f64 + 0.99);
                self.fitted_for = metres;
            }
        }
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::TILE_RADIUS, pal.well());
        let (cx, cy) = to_px(self.area.lat, self.area.lon, z);
        let screen = |px: f64, py: f64| pos2(rect.center().x + ((px - cx) * k) as f32, rect.center().y + ((py - cy) * k) as f32);
        let half = (rect.width() as f64 / 2.0 / k, rect.height() as f64 / 2.0 / k);
        let t = TILE_PX as f64;
        let side = 1i64 << z;
        let ctx = ui.ctx().clone();
        let water = water_colour(&pal);
        if self.textures.len() > 300 {
            self.textures.clear();
        }
        for ty in ((cy - half.1) / t).floor() as i64..=((cy + half.1) / t).floor() as i64 {
            if ty < 0 || ty >= side {
                continue;
            }
            for tx in ((cx - half.0) / t).floor() as i64..=((cx + half.0) / t).floor() as i64 {
                let key = TileKey { z, x: tx.rem_euclid(side) as u32, y: ty as u32 };
                let at = Rect::from_min_max(screen(tx as f64 * t, ty as f64 * t), screen((tx + 1) as f64 * t, (ty + 1) as f64 * t));
                let texture = match self.textures.get(&key) {
                    Some(tex) => Some(tex.clone()),
                    None => self.store.get(key, &ctx).map(|tile| {
                        let tex = ctx.load_texture(format!("real tile {}/{}/{}", key.z, key.x, key.y), tile_picture(&tile, water), TextureOptions::LINEAR);
                        self.textures.insert(key, tex.clone());
                        tex
                    }),
                };
                if let Some(tex) = texture {
                    painter.image(tex.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
            }
        }
        // The area: its corners on the map.
        let (w, d) = metres;
        let corner = |e: f64, s: f64| {
            let (lat, lon) = self.area.lat_lon(e, s);
            let (mut px, py) = to_px(lat, lon, z);
            let n = massif_core::real::world_px(z);
            if px - cx > n / 2.0 {
                px -= n;
            } else if cx - px > n / 2.0 {
                px += n;
            }
            screen(px, py)
        };
        let outline = vec![corner(-w / 2.0, -d / 2.0), corner(w / 2.0, -d / 2.0), corner(w / 2.0, d / 2.0), corner(-w / 2.0, d / 2.0)];
        painter.add(egui::Shape::convex_polygon(outline.clone(), pal.accent.gamma_multiply(0.12), egui::Stroke::NONE));
        painter.add(egui::Shape::closed_line(outline, egui::Stroke::new(1.5, pal.accent)));
        painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    }

    /// What the ground will be, small, made again when the settings or the
    /// tiles at hand change.
    fn preview(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let ctx = ui.ctx().clone();
        let (rect, _) = ui.allocate_exact_size(PREVIEW, Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::TILE_RADIUS, pal.well());
        // The picture keeps the area's shape, in the box.
        let (w, d) = (self.area.width as f32, self.area.depth as f32);
        let s = (rect.width() / w).min(rect.height() / d);
        let at = Rect::from_center_size(rect.center(), vec2(w * s, d * s));
        let made = self.make_picture(&ctx, &pal);
        match (&self.picture, made) {
            (Some(p), _) => {
                painter.image(p.texture.id(), at, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if !p.complete {
                    ui.put(Rect::from_center_size(rect.right_top() + vec2(-14.0, 14.0), vec2(14.0, 14.0)), egui::Spinner::new().size(14.0));
                }
            }
            (None, Some(why)) => {
                painter.text(rect.center(), egui::Align2::CENTER_CENTER, why, egui::FontId::proportional(11.5), pal.weak);
            }
            (None, None) => {
                ui.put(Rect::from_center_size(rect.center(), vec2(16.0, 16.0)), egui::Spinner::new().size(16.0));
            }
        }
        painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, pal.border), egui::StrokeKind::Inside);
    }

    /// The picture of the ground, made again when needed; why there is
    /// none, when there is none.
    fn make_picture(&mut self, ctx: &egui::Context, pal: &Palette) -> Option<String> {
        let a = self.area;
        let n = PREVIEW_N.max(2);
        let spacing = a.width.max(a.depth) as f64 / n as f64;
        let (source, complete): (Box<dyn Elevation>, bool) = if self.online {
            // Pixels about the preview's samples, a few tiles at most.
            let (e, _) = a.metres();
            let fine = (0..=MAX_ZOOM).find(|&z| metres_per_px(a.lat, z) <= e / n as f64).unwrap_or(MAX_ZOOM);
            let z = fine.min(a.zoom(PREVIEW_TILES));
            let keys = a.tiles(z);
            let mut tiles = HashMap::new();
            let mut failed = 0;
            for key in &keys {
                match self.store.get(*key, ctx) {
                    Some(t) => {
                        tiles.insert(*key, t);
                    }
                    None => failed += self.store.failed(*key) as usize,
                }
            }
            if tiles.is_empty() {
                return (failed > 0).then(|| if self.store.offline() { "Offline, and this place\nwas not downloaded before".to_owned() } else { "The elevation could not be had".to_owned() });
            }
            let complete = tiles.len() == keys.len();
            (Box::new(Tiles { z, tiles }) as Box<dyn Elevation>, complete)
        } else {
            match &self.file {
                Some(f) => (Box::new(f.grid()) as Box<dyn Elevation>, true),
                None => return Some("Choose a file of heights".to_owned()),
            }
        };
        let file_key = self.file.as_ref().map(|f| (f.path.clone(), f.metres_per_px, f.low, f.high));
        let key = format!("{a:?}{:?}{}{file_key:?}{complete}{}", self.vertical, self.online, pal.dark);
        if self.picture.as_ref().is_some_and(|p| p.made_for == key) {
            return None;
        }
        let samples = Samples::read(&a, source.as_ref(), spacing);
        let span = samples.span();
        let fit = self.vertical.fit(span.unwrap_or(Span { low: 0.0, high: 0.0 }), a.blocks_per_metre);
        let image = ground_picture(&samples, &fit, water_colour(pal));
        let texture = ctx.load_texture("real preview", image, TextureOptions::LINEAR);
        self.picture = Some(Picture { made_for: key, texture, span, fit, complete });
        None
    }

    /// Whether the preview is there (tests).
    #[cfg(test)]
    pub fn has_picture(&self) -> bool {
        self.picture.as_ref().is_some_and(|p| p.complete)
    }
}

/// A distance, in metres or kilometres.
fn distance(m: f64) -> String {
    if m >= 10_000.0 {
        format!("{:.0} km", m / 1000.0)
    } else if m >= 1000.0 {
        format!("{:.1} km", m / 1000.0)
    } else if m >= 10.0 {
        format!("{m:.0} m")
    } else {
        format!("{m:.2} m")
    }
}

/// Water in the pictures: they are maps, drawn as the 2D map draws water,
/// a little darker in the dark theme.
fn water_colour(pal: &Palette) -> Color32 {
    if pal.dark { Color32::from_rgb(38, 78, 132) } else { Color32::from_rgb(58, 110, 178) }
}

/// A grey for a height `t` (0 to 1), lit by `shade`; under the sea, the
/// water's colour over it.
fn shaded(t: f32, shade: f32, wet: Option<f32>, water: Color32) -> Color32 {
    let g = ((46.0 + 168.0 * t.clamp(0.0, 1.0)) * shade).clamp(0.0, 255.0);
    let grey = Color32::from_rgb(g as u8, (g * 1.01).min(255.0) as u8, (g * 1.04).min(255.0) as u8);
    match wet {
        Some(depth) => grey.lerp_to_gamma(water, 0.45 + 0.35 * depth.clamp(0.0, 1.0)),
        None => grey,
    }
}

/// A tile of elevation, lit from the north-west: the map of the picker.
fn tile_picture(tile: &DemTile, water: Color32) -> ColorImage {
    let n = TILE_PX;
    let at = |x: usize, y: usize| tile.at(x.min(n - 1), y.min(n - 1));
    let mut pixels = Vec::with_capacity(n * n);
    for y in 0..n {
        for x in 0..n {
            let m = at(x, y);
            let dx = at(x + 1, y) - at(x.saturating_sub(1), y);
            let dy = at(x, y + 1) - at(x, y.saturating_sub(1));
            // Slopes in metres a pixel: lit enough at every zoom.
            let shade = (1.0 + (dx + dy) * 0.004).clamp(0.4, 1.5);
            let t = (m.max(0.0) / 4500.0).sqrt();
            let wet = (m <= 0.0).then(|| -m / 4000.0);
            pixels.push(shaded(t, shade, wet, water));
        }
    }
    ColorImage::new([n, n], pixels)
}

/// The ground the samples make: lit relief in greys, the sea over it.
fn ground_picture(s: &Samples, fit: &Fit, water: Color32) -> ColorImage {
    let (nx, nz) = (s.nx, s.nz);
    let h = |i: usize, j: usize| {
        let m = s.metres[j.min(nz - 1) * nx + i.min(nx - 1)];
        if m.is_finite() { Some(fit.height(m)) } else { None }
    };
    let span = (fit.high - fit.low).max(1.0);
    let mut pixels = Vec::with_capacity(nx * nz);
    for j in 0..nz {
        for i in 0..nx {
            let Some(y) = h(i, j) else {
                pixels.push(Color32::TRANSPARENT);
                continue;
            };
            let dx = h(i + 1, j).unwrap_or(y) - h(i.saturating_sub(1), j).unwrap_or(y);
            let dz = h(i, j + 1).unwrap_or(y) - h(i, j.saturating_sub(1)).unwrap_or(y);
            let shade = (1.0 + (dx + dz) / (2.0 * s.spacing as f32) * 0.9).clamp(0.3, 1.6);
            let wet = (y < fit.sea).then(|| (fit.sea - y) / 40.0);
            pixels.push(shaded((y - fit.low) / span, shade, wet, water));
        }
    }
    ColorImage::new([nx, nz], pixels)
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Tiles of a mountain by the sea around `area`, put in `store`: the
    /// sea to the west, a peak of 3000 m to the east.
    pub fn seed_store(store: &TileStore, area: &Area) {
        for z in 0..=MAX_ZOOM {
            let keys = area.tiles(z);
            if keys.len() > 256 {
                break;
            }
            for key in keys {
                let metres = (0..TILE_PX * TILE_PX)
                    .map(|k| {
                        let (lat, lon) = massif_core::real::from_px(key.x as f64 * 256.0 + (k % TILE_PX) as f64, key.y as f64 * 256.0 + (k / TILE_PX) as f64, z);
                        let (e, s) = area.offset(lat, lon);
                        let (w, _) = area.metres();
                        let r = ((e - w / 4.0).powi(2) + s.powi(2)).sqrt() / (w / 4.0);
                        (3000.0 * (1.0 - r).max(0.0) + e / w * 1600.0 - 300.0) as f32
                    })
                    .collect();
                store.insert(key, Arc::new(DemTile { metres }));
            }
        }
    }

    #[test]
    fn distances_read_well() {
        assert_eq!(distance(0.5), "0.50 m");
        assert_eq!(distance(30.0), "30 m");
        assert_eq!(distance(4096.0), "4.1 km");
        assert_eq!(distance(61_440.0), "61 km");
    }
}
