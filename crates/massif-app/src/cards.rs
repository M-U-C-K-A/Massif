//! Cards: the rich tooltips of the tools and the brushes, as in the user's
//! WorldPainter fork. A card shows a little looping animation of what a tool
//! does — or the land a brush raises, slowly turning — then its name and
//! shortcut, what it does, what the mouse and the keys do with it, and a
//! remark.
//!
//! The animations are worked out from scratch for each frame from the time
//! in their loop, on a little patch of land drawn like the map; the relief of
//! a brush is a shaded mesh rasterised in software. Both only run while their
//! card shows.

use crate::theme::{Palette, RichTextExt};
use crate::tools::Tool;
use egui::{Color32, ColorImage, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use massif_core::Brush;

/// The width of a card's contents and the size of its picture, in points.
const WIDTH: f32 = 276.0;
const PREVIEW: egui::Vec2 = egui::Vec2::new(276.0, 140.0);
/// The length of an animation's loop, in seconds.
const LOOP: f64 = 4.2;

/// What a card shows above its text.
pub enum Preview<'a> {
    Demo(Demo),
    /// The land a brush raises, turning from side to side.
    Relief { key: String, brush: &'a Brush },
}

pub struct Card<'a> {
    pub title: String,
    pub shortcut: Option<String>,
    pub body: &'a str,
    /// A gesture (`Drag`, `Right-click`…) and what it does.
    pub actions: Vec<(&'a str, &'a str)>,
    pub tip: Option<&'a str>,
    pub preview: Option<Preview<'a>>,
}

/// Show `card` as the tooltip of `response`, while it is hovered.
pub fn attach<'a>(response: egui::Response, card: impl FnOnce() -> Card<'a>) -> egui::Response {
    response.on_hover_ui(|ui| show(ui, &card()))
}

/// The card of a tool.
pub fn tool(response: egui::Response, tool: Tool) -> egui::Response {
    attach(response, || tool_card(tool))
}

/// The card of a brush: its land, turning.
pub fn brush(response: egui::Response, name: &str, key: &str, brush: Option<&Brush>) -> egui::Response {
    match brush {
        Some(brush) => attach(response, || Card {
            title: name.to_owned(),
            shortcut: None,
            body: "The picture shows the land this brush raises. When painting, it paints most where the land is highest.",
            actions: vec![
                ("Scroll", "On the map: change its size"),
                (if cfg!(target_os = "macos") { "⌘+scroll" } else { "Ctrl+Shift+scroll" }, "On the map: turn it"),
                ("[  ]", "Turn it by steps"),
            ],
            tip: None,
            preview: Some(Preview::Relief { key: key.to_owned(), brush }),
        }),
        None => response.on_hover_text(name),
    }
}

/// The time in the loop of the card showing now, from 0 to 1: from the
/// start each time a card appears.
fn loop_time(ui: &Ui) -> f64 {
    let id = egui::Id::unique("card loop");
    let now = ui.input(|i| i.time);
    let frame = ui.ctx().cumulative_frame_nr();
    let (start, last) = ui.ctx().data(|d| d.get_temp::<(f64, u64)>(id)).unwrap_or((now, 0));
    let start = if frame > last + 2 { now } else { start };
    ui.ctx().data_mut(|d| d.insert_temp(id, (start, frame)));
    ((now - start) % LOOP) / LOOP
}

/// A texture kept from frame to frame, its image replaced each time.
fn texture(ui: &Ui, name: &str, image: ColorImage, filter: TextureOptions) -> TextureHandle {
    let id = egui::Id::unique(("card texture", name));
    match ui.ctx().data(|d| d.get_temp::<TextureHandle>(id)) {
        Some(mut t) => {
            t.set(image, filter);
            t
        }
        None => {
            let t = ui.ctx().load_texture(format!("card {name}"), image, filter);
            ui.ctx().data_mut(|d| d.insert_temp(id, t.clone()));
            t
        }
    }
}

/// A gesture or a key, in a little box.
fn chip(ui: &mut Ui, text: &str, pal: &Palette) {
    let subtle = crate::theme::mix(pal.panel, pal.text, 0.1);
    egui::Frame::new()
        .fill(subtle)
        .stroke(Stroke::new(1.0, pal.border))
        .corner_radius(3)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(11.5).color(pal.text)));
}

pub(crate) fn show(ui: &mut Ui, card: &Card) {
    let pal = Palette::of(ui);
    ui.set_width(WIDTH);
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(preview) = &card.preview {
        let (rect, _) = ui.allocate_exact_size(PREVIEW, Sense::hover());
        let t = loop_time(ui);
        match preview {
            Preview::Demo(demo) => demo.paint(ui, rect, t),
            Preview::Relief { key, brush } => relief::paint(ui, rect, key, brush, t),
        }
        ui.painter().rect_stroke(rect, 3, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        ui.ctx().request_repaint();
        ui.add_space(6.0);
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(&card.title).bold().size(14.0).color(pal.text));
        if let Some(key) = &card.shortcut {
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| chip(ui, key, &pal));
        }
    });
    ui.label(RichText::new(card.body).size(12.5).color(pal.text));
    if !card.actions.is_empty() {
        ui.add_space(4.0);
        egui::Grid::new(egui::Id::unique(("card actions", &card.title))).num_columns(2).spacing([8.0, 5.0]).show(ui, |ui| {
            for (gesture, what) in &card.actions {
                chip(ui, gesture, &pal);
                ui.label(RichText::new(*what).size(12.5).color(pal.text));
                ui.end_row();
            }
        });
    }
    if let Some(tip) = card.tip {
        ui.add_space(4.0);
        ui.label(RichText::new(tip).size(11.5).color(pal.weak));
    }
}

/// What each tool's card says, and its animation.
pub fn tool_card(tool: Tool) -> Card<'static> {
    let title = tool.label().to_owned();
    let shortcut = tool.shortcut_text();
    let card = |body: &'static str, actions: Vec<(&'static str, &'static str)>, tip: Option<&'static str>, demo: Option<Demo>| Card {
        title: title.clone(),
        shortcut: shortcut.clone(),
        body,
        actions,
        tip,
        preview: demo.map(Preview::Demo),
    };
    match tool {
        Tool::Raise => card(
            "Raises or lowers the land under the brush.",
            vec![("Drag", "Raise the land"), ("Right-drag", "Lower the land")],
            Some("The brush sets the shape, its intensity the speed."),
            Some(Demo::Raise),
        ),
        Tool::Smooth => card("Softens rough or jagged land without changing its overall shape.", vec![("Drag", "Smooth the land")], None, Some(Demo::Smooth)),
        Tool::Flatten => card(
            "Levels the land to a single height.",
            vec![("Drag", "Level to the height where you started")],
            Some("For building sites, fields and roads."),
            Some(Demo::Flatten),
        ),
        Tool::SetHeight => card(
            "Raises or digs the land to the height you set.",
            vec![("Drag", "Bring the land to the height"), ("Alt+click", "Pick the height on the map")],
            None,
            Some(Demo::SetHeight),
        ),
        Tool::Roughen => card(
            "Roughens the land: rocks, ruts and bumps.",
            vec![("Drag", "Roughen the land"), ("Right-drag", "Make it smoother")],
            None,
            Some(Demo::Roughen),
        ),
        Tool::PaintTerrain => card(
            "Paints a terrain: the blocks on top of the ground.",
            vec![("Drag", "Paint"), ("Right-drag", "Back to what the height calls for")],
            Some("Pick it in Terrain or Blocks, or Alt-click it on the map. A pattern mixes several."),
            Some(Demo::Paint),
        ),
        Tool::PaintBiome => card(
            "Paints a biome: the colours of grass and water, the weather, what lives there.",
            vec![("Drag", "Paint"), ("Right-drag", "Back to automatic")],
            None,
            Some(Demo::Biome),
        ),
        Tool::Cover => card(
            "Grows grass, flowers, snow — or any blocks — over the ground.",
            vec![("Drag", "Grow it"), ("Right-drag", "Take it off")],
            Some("Each plant grows where the world's seed puts it, on soil if the cover asks."),
            Some(Demo::Cover),
        ),
        Tool::Objects => card(
            "Stands objects — trees, rocks, ruins — where you paint.",
            vec![("Drag", "Paint where they stand"), ("Right-drag", "Take them off")],
            Some("Import .schem, .schematic, .bp, .litematic or .nbt files in its settings."),
            Some(Demo::Objects),
        ),
        Tool::AutoPaint => card(
            "Paints what the land calls for: rock on cliffs, sand on shores, grass elsewhere.",
            vec![("Hover", "See what would change"), ("Drag", "Paint it")],
            Some("Rules… sets the terrains of each biome."),
            Some(Demo::AutoPaint),
        ),
        Tool::AutoBiome => card(
            "Sets the biome the land calls for: beaches, forests, peaks…",
            vec![("Hover", "See what would change"), ("Drag", "Set it")],
            Some("Rules… sets the biome of each kind of land."),
            Some(Demo::AutoBiome),
        ),
        Tool::Eyedropper => card(
            "Picks the terrain, biome, height and water under the cursor.",
            vec![("Click", "Pick what is here")],
            None,
            Some(Demo::Eyedropper),
        ),
        Tool::Flood => card(
            "Fills low ground with water as far as it is enclosed: lakes and seas.",
            vec![("Click", "Flood up to the water level"), ("Right-click", "Drain it")],
            None,
            Some(Demo::Flood),
        ),
        Tool::Drain => card(
            "Drains a body of water, as far as it reaches.",
            vec![("Click", "Drain it"), ("Right-click", "Fill it")],
            None,
            Some(Demo::Drain),
        ),
        Tool::Water => card(
            "Sets the water under the brush to the water level.",
            vec![("Drag", "Lay water"), ("Right-drag", "Take it off")],
            None,
            Some(Demo::Water),
        ),
        Tool::River => card(
            "Lays out a river from its source: the water finds its way down the valleys.",
            vec![
                ("Click", "Its source, then where it ends"),
                ("Drag", "Guide it through a point"),
                ("Right-click", "Take a point off"),
                ("Enter", "Dig it"),
            ],
            Some("Its width, depth and banks are in its settings."),
            Some(Demo::River),
        ),
        Tool::StampSlab => card(
            "Adds rock to the level you edit: floating islands, arches, overhangs.",
            vec![("Drag", "Add rock"), ("Right-drag", "Erase it")],
            Some("Choose its thickness and how high it floats in the settings."),
            Some(Demo::Slab),
        ),
        Tool::EraseSlab => card("Erases rock of the level you edit.", vec![("Drag", "Erase"), ("Right-drag", "Add rock")], None, Some(Demo::EraseSlab)),
        Tool::Build | Tool::Dig => card(
            "Adds matter in 3D where you point, laid along the slope.",
            vec![("Drag", "Add matter"), ("Right-drag", "Dig"), ("Shift+drag", "Move the view"), ("Alt+scroll", "Brush size")],
            None,
            Some(Demo::Raise),
        ),
        Tool::Smooth3d => card("Rounds off in 3D: edges, hollows, overhangs.", vec![("Drag", "Smooth")], None, Some(Demo::Smooth)),
        Tool::Pan => card("Moves the map.", vec![("Drag", "Move the map"), ("Middle-drag", "Move it with any tool"), ("Space+drag", "The same")], None, Some(Demo::Pan)),
    }
}

// ------------------------------------------------------------ animations --

/// The size of the patch of land of an animation, in cells.
const W: usize = 128;
const H: usize = 64;
/// Dabs along a stroke.
const SAMPLES: usize = 48;
/// The action takes this share of the loop; then its result is held.
const ACTION: f64 = 0.78;

/// A little looping animation of what a tool does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Demo {
    Raise,
    Smooth,
    Flatten,
    SetHeight,
    Roughen,
    Paint,
    Biome,
    Cover,
    Objects,
    AutoPaint,
    AutoBiome,
    Eyedropper,
    Flood,
    Drain,
    Water,
    River,
    Slab,
    EraseSlab,
    Pan,
}

fn smooth_step(edge0: f64, edge1: f64, x: f64) -> f64 {
    let t = ((x - edge0) / (edge1 - edge0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The path of a brush stroke: a gentle wave across the patch.
fn sweep(s: f64) -> (f64, f64) {
    (16.0 + (W as f64 - 32.0) * s, H as f64 / 2.0 + (s * std::f64::consts::PI * 2.2).sin() * 11.0)
}

fn hash(i: usize) -> f64 {
    let mut h = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
    h = (h ^ (h >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 27;
    (h >> 40) as f64 / (1u64 << 24) as f64
}

const GRASS: u8 = 0;
const SAND: u8 = 1;
const STONE: u8 = 2;
const COLOURS: [[u8; 3]; 3] = [[0x5f, 0x9e, 0x3a], [0xe2, 0xd3, 0x9a], [0x8a, 0x8a, 0x8a]];
const ACCENT: Color32 = Color32::from_rgb(0xf9, 0x73, 0x16);

/// The land of an animation: a height and a terrain per cell, water, and
/// what lies over it.
struct Scene {
    heights: Vec<f32>,
    terrain: Vec<u8>,
    water: f32,
    lava: bool,
    /// Cells where the water was taken off, or laid.
    dry: Option<Vec<bool>>,
    wet: Option<Vec<bool>>,
    /// A colour laid over cells (biomes), and spots of colour (plants).
    tint: Vec<Option<[u8; 3]>>,
    spots: Vec<Option<[u8; 3]>>,
    /// Rock of a level over the land: how much of it a cell has (0 to 1).
    slab: Option<Vec<f32>>,
    /// Points clicked, trees standing (x, y, how grown), the swatch of a
    /// picked terrain, the river laid out.
    markers: Vec<(f64, f64)>,
    trees: Vec<(f64, f64, f64)>,
    swatch: Option<u8>,
}

impl Scene {
    fn new() -> Self {
        let mut heights = vec![0.0f32; W * H];
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f64, y as f64);
                heights[y * W + x] = (64.0 + 1.5 * (fx * 0.11 + 1.0).sin() * (fy * 0.15).cos() + (fx * 0.05 + fy * 0.07).sin() * 1.5) as f32;
            }
        }
        Self {
            heights,
            terrain: vec![GRASS; W * H],
            water: f32::NEG_INFINITY,
            lava: false,
            dry: None,
            wet: None,
            tint: vec![None; W * H],
            spots: vec![None; W * H],
            slab: None,
            markers: Vec::new(),
            trees: Vec::new(),
            swatch: None,
        }
    }

    fn near(cx: f64, cy: f64, radius: f64, mut f: impl FnMut(usize, f64)) {
        let (x1, x2) = ((cx - radius).floor().max(0.0) as usize, ((cx + radius).ceil() as usize).min(W - 1));
        let (y1, y2) = ((cy - radius).floor().max(0.0) as usize, ((cy + radius).ceil() as usize).min(H - 1));
        for y in y1..=y2 {
            for x in x1..=x2 {
                let d = ((x as f64 - cx).powi(2) + (y as f64 - cy).powi(2)).sqrt();
                if d <= radius {
                    f(y * W + x, d);
                }
            }
        }
    }

    fn bump(&mut self, cx: f64, cy: f64, radius: f64, amount: f64) {
        let h = &mut self.heights;
        Self::near(cx, cy, radius * 1.6, |i, d| h[i] += (amount * (-d * d / (radius * radius * 0.5)).exp()) as f32);
    }

    fn plateau(weight: &mut [f32], cx: f64, cy: f64, radius: f64) {
        Self::near(cx, cy, radius, |i, d| weight[i] = weight[i].max(smooth_step(radius, radius * 0.6, d) as f32));
    }

    fn add_hills(&mut self, amplitude: f64) {
        for y in 0..H {
            for x in 0..W {
                let (fx, fy) = (x as f64, y as f64);
                self.heights[y * W + x] += (amplitude * (0.5 + 0.5 * (fx * 0.13 + 0.7).sin() * (fy * 0.19 + 0.3).sin()) + amplitude * 0.4 * (fx * 0.31 + fy * 0.23).sin()) as f32;
            }
        }
    }

    fn add_basin(&mut self) {
        for y in 0..H {
            for x in 0..W {
                let d = (((x as f64 - W as f64 / 2.0) / 46.0).powi(2) + ((y as f64 - H as f64 / 2.0) / 22.0).powi(2)).sqrt();
                self.heights[y * W + x] = (54.0 + 16.0 * smooth_step(0.2, 1.1, d) + (x as f64 * 0.3 + y as f64 * 0.2).sin()) as f32;
            }
        }
    }

    fn blurred(&self, radius: isize) -> Vec<f32> {
        let mut out = vec![0.0; W * H];
        for y in 0..H as isize {
            for x in 0..W as isize {
                let (mut sum, mut count) = (0.0, 0.0);
                for dy in -radius..=radius {
                    for dx in -radius..=radius {
                        let (nx, ny) = (x + dx, y + dy);
                        if nx >= 0 && ny >= 0 && nx < W as isize && ny < H as isize {
                            sum += self.heights[ny as usize * W + nx as usize];
                            count += 1.0;
                        }
                    }
                }
                out[y as usize * W + x as usize] = sum / count;
            }
        }
        out
    }

    /// The land as the map draws it: lit, its contours, water.
    fn render(&self) -> ColorImage {
        let mut pixels = Vec::with_capacity(W * H);
        let mix = |a: [f32; 3], b: [u8; 3], t: f32| [0, 1, 2].map(|k| a[k] + (b[k] as f32 - a[k]) * t);
        for y in 0..H {
            for x in 0..W {
                let i = y * W + x;
                let h = self.heights[i];
                let at = |x: usize, y: usize| self.heights[y * W + x];
                let (west, east) = (at(x.saturating_sub(1), y), at((x + 1).min(W - 1), y));
                let (north, south) = (at(x, y.saturating_sub(1)), at(x, (y + 1).min(H - 1)));
                let mut shade = (1.0 + (east - west + south - north) * 0.09).clamp(0.45, 1.5);
                let mut c = COLOURS[self.terrain[i] as usize].map(|v| v as f32);
                if let Some(t) = self.tint[i] {
                    c = mix(c, t, 0.45);
                }
                if let Some(s) = self.spots[i] {
                    c = s.map(|v| v as f32);
                }
                // A level's rock over the land, with its shadow beside it.
                if let Some(slab) = &self.slab {
                    let k = slab[i];
                    let shadow = slab[y.saturating_sub(2) * W + x.saturating_sub(2)];
                    if k > 0.0 {
                        c = mix(c, [176, 172, 164], k);
                        shade = 1.0 + 0.25 * k;
                    } else if shadow > 0.0 {
                        shade *= 1.0 - 0.45 * shadow;
                    }
                }
                let flooded = self.water > h && self.dry.as_ref().is_none_or(|d| !d[i]) && self.wet.as_ref().is_none_or(|w| w[i]);
                if flooded {
                    let depth = ((self.water - h) / 10.0).min(1.0);
                    c = if self.lava { mix(c, [0xe8, 0x60, 0x1c], 0.95) } else { mix(c, [0x2f, 0x6f, 0xd8], 0.55 + 0.4 * depth) };
                    shade = 1.0;
                }
                let mut rgb = c.map(|v| (v * shade).min(255.0));
                // Contour lines, as on the map.
                let low = west.min(east).min(north.min(south));
                if !flooded && (h / 4.0).floor() > (low / 4.0).floor() {
                    rgb = rgb.map(|v| v * 0.75);
                }
                pixels.push(Color32::from_rgb(rgb[0] as u8, rgb[1] as u8, rgb[2] as u8));
            }
        }
        ColorImage::new([W, H], pixels)
    }
}

impl Demo {
    /// Apply the tool up to a point of its action. Returns the cursor —
    /// where, its radius in cells, whether it clicks — if one shows.
    fn act(self, s: &mut Scene, progress: f64) -> Option<(f64, f64, f64, bool)> {
        let steps = (progress * SAMPLES as f64).round() as usize;
        let stroke = |radius: f64| {
            let (x, y) = sweep(progress);
            Some((x, y, radius, false))
        };
        match self {
            Demo::Raise => {
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    s.bump(x, y, 9.0, 2.4);
                }
                stroke(9.0)
            }
            Demo::Flatten | Demo::SetHeight => {
                s.add_hills(9.0);
                let mut weight = vec![0.0; W * H];
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    Scene::plateau(&mut weight, x, y, 10.0);
                }
                let target = if self == Demo::Flatten { 66.0 } else { 74.0 };
                for (h, w) in s.heights.iter_mut().zip(&weight) {
                    *h += (target - *h) * w;
                }
                stroke(10.0)
            }
            Demo::Smooth | Demo::Roughen => {
                if self == Demo::Smooth {
                    for (i, h) in s.heights.iter_mut().enumerate() {
                        *h += ((hash(i) - 0.5) * 4.0 + (hash(i / 3 + 7919) - 0.5) * 4.0) as f32;
                    }
                }
                let mut weight = vec![0.0; W * H];
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    Scene::plateau(&mut weight, x, y, 11.0);
                }
                let blurred = s.blurred(3);
                for (i, h) in s.heights.iter_mut().enumerate() {
                    let target = if self == Demo::Smooth { blurred[i] } else { *h + ((hash(i) - 0.5) * 6.0 + (hash(i / 2 + 31) - 0.5) * 5.0) as f32 };
                    *h += (target - *h) * weight[i];
                }
                stroke(11.0)
            }
            Demo::Paint => {
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    let t = &mut s.terrain;
                    Scene::near(x, y, 7.0, |i, _| t[i] = SAND);
                }
                stroke(7.0)
            }
            Demo::Biome => {
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    let t = &mut s.tint;
                    Scene::near(x, y, 9.0, |i, _| t[i] = Some([214, 190, 96]));
                }
                stroke(9.0)
            }
            Demo::Cover => {
                const FLOWERS: [[u8; 3]; 5] = [[236, 206, 52], [200, 44, 38], [238, 238, 230], [72, 112, 222], [178, 112, 212]];
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    let spots = &mut s.spots;
                    Scene::near(x, y, 9.0, |i, _| {
                        if hash(i * 7 + 3) < 0.3 {
                            spots[i] = Some(FLOWERS[(hash(i * 13) * 5.0) as usize % 5]);
                        }
                    });
                }
                stroke(9.0)
            }
            Demo::Objects => {
                // Trees stand on a grid of squares, as they grow where painted.
                s.add_hills(3.0);
                let painted: Vec<(f64, f64)> = (0..steps).map(|k| sweep(k as f64 / SAMPLES as f64)).collect();
                for gy in 0..H / 9 + 1 {
                    for gx in 0..W / 9 + 1 {
                        let cell = gy * 31 + gx;
                        let (x, y) = (gx as f64 * 9.0 + 2.0 + hash(cell) * 5.0, gy as f64 * 9.0 + 2.0 + hash(cell + 999) * 5.0);
                        if let Some(k) = painted.iter().position(|&(px, py)| (px - x).hypot(py - y) < 10.0) {
                            let age = (steps - k) as f64 / 8.0;
                            s.trees.push((x, y, smooth_step(0.0, 1.0, age)));
                        }
                    }
                }
                stroke(10.0)
            }
            Demo::AutoPaint | Demo::AutoBiome => {
                s.add_basin();
                s.add_hills(4.0);
                s.water = 61.0;
                let mut done = vec![false; W * H];
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    Scene::near(x, y, 10.0, |i, _| done[i] = true);
                }
                for (i, _) in done.iter().enumerate().filter(|d| *d.1) {
                    let (x, y) = (i % W, i / W);
                    let h = s.heights[i];
                    let slope = (s.heights[y * W + (x + 1).min(W - 1)] - s.heights[y * W + x.saturating_sub(1)]).abs()
                        + (s.heights[(y + 1).min(H - 1) * W + x] - s.heights[y.saturating_sub(1) * W + x]).abs();
                    if self == Demo::AutoPaint {
                        s.terrain[i] = if h < s.water + 2.0 { SAND } else if slope > 3.2 { STONE } else { GRASS };
                    } else {
                        s.tint[i] = Some(if h < s.water + 2.0 { [236, 214, 140] } else if h > 70.0 { [240, 244, 248] } else { [40, 120, 50] });
                    }
                }
                stroke(10.0)
            }
            Demo::Eyedropper => {
                for y in 18..42 {
                    for x in 70..104 {
                        if ((x as f64 - 87.0) / 17.0).hypot((y as f64 - 30.0) / 12.0) < 1.0 {
                            s.terrain[y * W + x] = SAND;
                        }
                    }
                }
                s.swatch = Some(if progress > 0.6 { SAND } else { GRASS });
                let m = smooth_step(0.0, 0.55, progress);
                Some((20.0 + (87.0 - 20.0) * m, 44.0 + (30.0 - 44.0) * m, 2.0, progress > 0.55 && progress < 0.7))
            }
            Demo::Flood | Demo::Drain => {
                s.add_basin();
                let rise = smooth_step(0.15, 1.0, progress);
                s.water = (54.0 + 12.0 * if self == Demo::Flood { rise } else { 1.0 - rise }) as f32;
                (progress < 0.3).then_some((64.0, 30.0, 2.0, true))
            }
            Demo::Water => {
                s.add_basin();
                s.water = 66.0;
                let mut wet = vec![false; W * H];
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    Scene::near(x, y, 9.0, |i, _| wet[i] = true);
                }
                s.wet = Some(wet);
                stroke(9.0)
            }
            Demo::River => {
                s.add_hills(7.0);
                let points = [(10.0, 14.0), (44.0, 36.0), (80.0, 24.0), (118.0, 46.0)];
                let clicked = ((progress / 0.12).floor() as usize + 1).min(points.len());
                s.markers = points[..clicked].to_vec();
                let dig = smooth_step(0.5, 0.95, progress);
                if dig > 0.0 {
                    let total: f64 = points.windows(2).map(|p| (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1)).sum();
                    let mut left = total * dig;
                    for p in points.windows(2) {
                        let len = (p[1].0 - p[0].0).hypot(p[1].1 - p[0].1);
                        let part = (left / len).min(1.0);
                        let mut t = 0.0;
                        while t <= part && left > 0.0 {
                            let (x, y) = (p[0].0 + (p[1].0 - p[0].0) * t, p[0].1 + (p[1].1 - p[0].1) * t);
                            let width = 2.0 + 2.5 * (x / W as f64);
                            let (h, wet) = (&mut s.heights, s.wet.get_or_insert_with(|| vec![false; W * H]));
                            Scene::near(x, y, width + 3.0, |i, d| {
                                h[i] = h[i].min(60.0 + (d / (width + 3.0) * 8.0) as f32);
                                if d <= width {
                                    wet[i] = true;
                                }
                            });
                            t += 0.5 / len;
                        }
                        left -= len;
                    }
                    s.water = 63.0;
                }
                let last = points[clicked - 1];
                (progress < 0.5).then_some((last.0, last.1, 2.0, (progress / 0.12) % 1.0 < 0.3))
            }
            Demo::Slab | Demo::EraseSlab => {
                s.add_hills(4.0);
                let mut slab = vec![0.0f32; W * H];
                if self == Demo::EraseSlab {
                    for y in 10..54 {
                        for x in 14..114 {
                            slab[y * W + x] = 1.0;
                        }
                    }
                }
                for k in 0..steps {
                    let (x, y) = sweep(k as f64 / SAMPLES as f64);
                    let value = if self == Demo::Slab { 1.0 } else { 0.0 };
                    Scene::near(x, y, 9.0, |i, _| slab[i] = value);
                }
                s.slab = Some(slab);
                stroke(9.0)
            }
            Demo::Pan => {
                // The land slides under the hand.
                let shift = 30.0 * smooth_step(0.1, 0.9, progress);
                let mut moved = Scene::new();
                moved.add_hills(8.0);
                for y in 0..H {
                    for x in 0..W {
                        let from = (x as f64 + shift) as usize;
                        s.heights[y * W + x] = moved.heights[y * W + from.min(W - 1)];
                        if (from as f64 * 0.05 + y as f64 * 0.09).sin() > 0.85 {
                            s.terrain[y * W + x] = SAND;
                        }
                    }
                }
                Some((84.0 - shift, 32.0, 2.0, progress < 0.92))
            }
        }
    }

    /// Draw the frame at `t` (0 to 1 in the loop) into `rect`.
    pub fn paint(self, ui: &Ui, rect: Rect, t: f64) {
        let mut scene = Scene::new();
        let progress = (t / ACTION).min(1.0);
        let cursor = self.act(&mut scene, progress);
        let image = scene.render();
        let texture = texture(ui, "demo", image, TextureOptions::LINEAR);
        let painter = ui.painter_at(rect);
        painter.image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        let (sx, sy) = (rect.width() / W as f32, rect.height() / H as f32);
        let at = |x: f64, y: f64| pos2(rect.left() + x as f32 * sx, rect.top() + y as f32 * sy);
        // Trees from above: a shadow, a round crown.
        for &(x, y, grown) in &scene.trees {
            let r = (3.2 * grown) as f32 * sx;
            if r > 0.5 {
                painter.circle_filled(at(x, y) + vec2(1.5, 1.5), r, Color32::from_black_alpha(90));
                painter.circle_filled(at(x, y), r, Color32::from_rgb(52, 104, 40));
                painter.circle_filled(at(x, y) - vec2(r * 0.25, r * 0.25), r * 0.55, Color32::from_rgb(72, 132, 52));
            }
        }
        for &(x, y) in &scene.markers {
            painter.circle(at(x, y), 4.0, ACCENT, Stroke::new(1.5, Color32::WHITE));
        }
        if let Some(sw) = scene.swatch {
            let r = Rect::from_min_size(pos2(rect.right() - 34.0, rect.top() + 8.0), vec2(26.0, 26.0));
            painter.rect_filled(r, 8, Color32::from_black_alpha(120));
            let c = COLOURS[sw as usize];
            painter.rect_filled(r.shrink(3.0), 6, Color32::from_rgb(c[0], c[1], c[2]));
        }
        if let Some((x, y, radius, clicking)) = cursor
            && t < ACTION + 0.08
        {
            draw_cursor(&painter, at(x, y), radius as f32 * sx, clicking);
        }
        // Fade in at the start of the loop, out at its end.
        let fade = smooth_step(0.06, 0.0, t).max(smooth_step(0.94, 1.0, t));
        if fade > 0.0 {
            painter.rect_filled(rect, 0, Color32::from_rgba_unmultiplied(0x0f, 0x11, 0x15, (fade * 255.0).round() as u8));
        }
    }
}

/// The brush's circle and an arrow pointer, with a dot while it clicks.
fn draw_cursor(painter: &egui::Painter, p: Pos2, radius: f32, clicking: bool) {
    if radius > 3.0 {
        painter.circle_stroke(p, radius, Stroke::new(1.5, Color32::from_white_alpha(210)));
        painter.circle_stroke(p, radius + 1.5, Stroke::new(1.0, Color32::from_black_alpha(90)));
    }
    if clicking {
        painter.circle_filled(p, 7.0, Color32::from_rgba_unmultiplied(ACCENT.r(), ACCENT.g(), ACCENT.b(), 160));
    }
    let arrow: Vec<Pos2> = [(0.0, 0.0), (0.0, 15.0), (4.0, 11.0), (7.0, 17.0), (9.0, 16.0), (6.0, 10.0), (11.0, 10.0)].iter().map(|&(x, y)| p + vec2(x, y)).collect();
    painter.add(Shape::convex_polygon(vec![arrow[0], arrow[1], arrow[2], arrow[5], arrow[6]], Color32::WHITE, Stroke::NONE));
    painter.add(Shape::convex_polygon(vec![arrow[2], arrow[3], arrow[4], arrow[5]], Color32::WHITE, Stroke::NONE));
    painter.add(Shape::closed_line(arrow, Stroke::new(1.0, Color32::BLACK)));
}

// --------------------------------------------------------------- reliefs --

mod relief {
    use super::*;

    const GRID: usize = 128;
    const VERTICES: usize = GRID + 1;
    const BACKGROUND: [u8; 3] = [0x18, 0x18, 0x1b];
    const RELIEF: f32 = 0.42;
    const BASE: f32 = 0.16;
    const TILT: f32 = 0.5;
    const LIGHT: [f64; 3] = [-0.25, 0.55, 0.8];
    const DEPTH_PER_GROUND: f32 = 0.866;
    const DEPTH_PER_HEIGHT: f32 = 0.577;
    const CONTOURS: f32 = 9.0;
    const LINE_WIDTH: f32 = 0.9;
    const SURFACE: [f32; 3] = [240.0, 239.0, 234.0];
    const SIDE: [f32; 3] = [143.0, 138.0, 128.0];

    /// The land of a brush, worked out once: its heights (0 to 1), the
    /// light each point gets, and how far it reaches up and down on screen.
    #[derive(Clone)]
    pub struct Land {
        heights: Vec<f32>,
        lights: Vec<f32>,
        up: f32,
        down: f32,
    }

    fn blur(values: &[f32], passes: usize) -> Vec<f32> {
        let mut from = values.to_vec();
        let mut to = vec![0.0; from.len()];
        for _ in 0..passes {
            for y in 0..VERTICES {
                for x in 0..VERTICES {
                    let mut sum = 0.0;
                    for dy in -1i32..=1 {
                        let row = (y as i32 + dy).clamp(0, GRID as i32) as usize * VERTICES;
                        for dx in -1i32..=1 {
                            sum += from[row + (x as i32 + dx).clamp(0, GRID as i32) as usize];
                        }
                    }
                    to[y * VERTICES + x] = sum / 9.0;
                }
            }
            std::mem::swap(&mut from, &mut to);
        }
        from
    }

    pub fn land(brush: &Brush) -> Land {
        let mut b = brush.clone();
        b.radius = GRID as f32 / 2.0;
        let mut land = vec![0.0f32; VERTICES * VERTICES];
        let mut max = 0.0f32;
        for y in 0..VERTICES {
            for x in 0..VERTICES {
                let v = b.weight(x as f32 - GRID as f32 / 2.0, y as f32 - GRID as f32 / 2.0);
                land[y * VERTICES + x] = v;
                max = max.max(v);
            }
        }
        if max > 0.0 {
            land.iter_mut().for_each(|v| *v /= max);
        }
        // Images have 256 levels only, which would show as terraces.
        let smooth = blur(&land, 2);
        // Lower than its surroundings: darker; higher: brighter.
        let around = blur(&smooth, 10);
        let length = (LIGHT[0] * LIGHT[0] + LIGHT[1] * LIGHT[1] + LIGHT[2] * LIGHT[2]).sqrt();
        let cell = 2.0 / GRID as f64;
        let mut lights = vec![0.0f32; VERTICES * VERTICES];
        for y in 0..VERTICES {
            for x in 0..VERTICES {
                let i = y * VERTICES + x;
                let h = |x: usize, y: usize| smooth[y * VERTICES + x] as f64;
                let su = (h((x + 1).min(GRID), y) - h(x.saturating_sub(1), y)) * RELIEF as f64 / (2.0 * cell);
                let sv = (h(x, (y + 1).min(GRID)) - h(x, y.saturating_sub(1))) * RELIEF as f64 / (2.0 * cell);
                let n = (su * su + sv * sv + 1.0).sqrt();
                let diffuse = ((-su * LIGHT[0] - sv * LIGHT[1] + LIGHT[2]) / (n * length)).max(0.0);
                let hollow = ((smooth[i] - around[i]) as f64 * 2.5).clamp(-0.3, 0.15);
                lights[i] = ((0.3 + 0.7 * diffuse + 0.06 * smooth[i] as f64) * (1.0 + hollow)).clamp(0.0, 1.0) as f32;
            }
        }
        // How far it reaches above and below the middle of the view as it
        // turns: all of it stays in the frame.
        let (mut up, mut down) = (0.0f32, 0.0f32);
        for yaw in [15.0f32, 30.0, 45.0, 60.0, 75.0] {
            let (cos, sin) = (yaw.to_radians().cos(), yaw.to_radians().sin());
            for (i, &h) in smooth.iter().enumerate() {
                let (u, v) = ((i % VERTICES) as f32 * cell as f32 - 1.0, (i / VERTICES) as f32 * cell as f32 - 1.0);
                let depth = (u * sin + v * cos) * TILT;
                up = up.max(h * RELIEF - depth);
                down = down.max(depth + BASE);
            }
        }
        Land { heights: smooth, lights, up, down }
    }

    struct Raster {
        w: usize,
        h: usize,
        colours: Vec<[f32; 3]>,
        depths: Vec<f32>,
        sx: Vec<f32>,
        sy: Vec<f32>,
        sz: Vec<f32>,
    }

    impl Raster {
        /// A triangle of the surface: its light between its corners, and
        /// contour lines.
        fn triangle(&mut self, a: usize, b: usize, c: usize, land: &Land) {
            let (ax, ay, bx, by, cx, cy) = (self.sx[a], self.sy[a], self.sx[b], self.sy[b], self.sx[c], self.sy[c]);
            let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
            if area <= 0.0 {
                return;
            }
            let inverse = 1.0 / area;
            let (min_x, max_x) = (ax.min(bx).min(cx).floor().max(0.0) as usize, (ax.max(bx).max(cx).ceil() as usize).min(self.w - 1));
            let (min_y, max_y) = (ay.min(by).min(cy).floor().max(0.0) as usize, (ay.max(by).max(cy).ceil() as usize).min(self.h - 1));
            let (az, bz, cz) = (self.sz[a], self.sz[b], self.sz[c]);
            let (al, bl, cl) = (land.lights[a], land.lights[b], land.lights[c]);
            let (ah, bh, ch) = (land.heights[a] * CONTOURS, land.heights[b] * CONTOURS, land.heights[c] * CONTOURS);
            let gx = ((ah - bh) * (cy - ay) - (ah - ch) * (by - ay)) * inverse;
            let gy = ((ah - ch) * (bx - ax) - (ah - bh) * (cx - ax)) * inverse;
            let line = (gx.abs() + gy.abs()) * LINE_WIDTH;
            for y in min_y..=max_y {
                let py = y as f32 + 0.5;
                for x in min_x..=max_x {
                    let px = x as f32 + 0.5;
                    let wa = ((bx - px) * (cy - py) - (by - py) * (cx - px)) * inverse;
                    let wb = ((cx - px) * (ay - py) - (cy - py) * (ax - px)) * inverse;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let o = y * self.w + x;
                    let z = wa * az + wb * bz + wc * cz;
                    if z > self.depths[o] {
                        self.depths[o] = z;
                        let mut light = wa * al + wb * bl + wc * cl;
                        let level = wa * ah + wb * bh + wc * ch;
                        let fraction = level - level.floor();
                        if level > 0.02 && level < CONTOURS - 0.1 && (fraction < line || fraction > 1.0 - line) {
                            light *= 7.0 / 8.0;
                        }
                        self.colours[o] = SURFACE.map(|v| v * light);
                    }
                }
            }
        }

        fn flat(&mut self, a: usize, b: usize, c: usize, light: f32) {
            let (ax, ay, bx, by, cx, cy) = (self.sx[a], self.sy[a], self.sx[b], self.sy[b], self.sx[c], self.sy[c]);
            let area = (bx - ax) * (cy - ay) - (by - ay) * (cx - ax);
            if area == 0.0 {
                return;
            }
            let inverse = 1.0 / area;
            let (min_x, max_x) = (ax.min(bx).min(cx).floor().max(0.0) as usize, (ax.max(bx).max(cx).ceil() as usize).min(self.w - 1));
            let (min_y, max_y) = (ay.min(by).min(cy).floor().max(0.0) as usize, (ay.max(by).max(cy).ceil() as usize).min(self.h - 1));
            let (az, bz, cz) = (self.sz[a], self.sz[b], self.sz[c]);
            let colour = SIDE.map(|v| v * light);
            for y in min_y..=max_y {
                let py = y as f32 + 0.5;
                for x in min_x..=max_x {
                    let px = x as f32 + 0.5;
                    let wa = ((bx - px) * (cy - py) - (by - py) * (cx - px)) * inverse;
                    let wb = ((cx - px) * (ay - py) - (cy - py) * (ax - px)) * inverse;
                    let wc = 1.0 - wa - wb;
                    if wa < 0.0 || wb < 0.0 || wc < 0.0 {
                        continue;
                    }
                    let o = y * self.w + x;
                    let z = wa * az + wb * bz + wc * cz;
                    if z > self.depths[o] {
                        self.depths[o] = z;
                        self.colours[o] = colour;
                    }
                }
            }
        }
    }

    /// The land seen from a three-quarter view turning gently from side to
    /// side, at `t` in the loop, `width` × `height` pixels.
    pub fn frame(land: &Land, t: f64, width: usize, height: usize) -> ColorImage {
        let samples = 2;
        let (w, h) = (width * samples, height * samples);
        let feet = VERTICES * VERTICES;
        let mut r = Raster {
            w,
            h,
            colours: vec![BACKGROUND.map(|v| v as f32); w * h],
            depths: vec![f32::NEG_INFINITY; w * h],
            sx: vec![0.0; feet + VERTICES],
            sy: vec![0.0; feet + VERTICES],
            sz: vec![0.0; feet + VERTICES],
        };
        let yaw = (45.0 + 30.0 * (t * std::f64::consts::TAU).sin()).to_radians();
        let (cos, sin) = (yaw.cos() as f32, yaw.sin() as f32);
        let reach = land.up + land.down;
        let scale = (w as f32 / 2.9).min(h as f32 * 0.92 / reach);
        let (cx, cy) = (w as f32 / 2.0, (h as f32 - reach * scale) / 2.0 + land.up * scale);
        let step = 2.0 / GRID as f32;
        for y in 0..VERTICES {
            let v = y as f32 * step - 1.0;
            for x in 0..VERTICES {
                let i = y * VERTICES + x;
                let u = x as f32 * step - 1.0;
                let depth = u * sin + v * cos;
                let height = land.heights[i] * RELIEF;
                r.sx[i] = cx + (u * cos - v * sin) * scale;
                r.sy[i] = cy + (depth * TILT - height) * scale;
                r.sz[i] = depth * DEPTH_PER_GROUND + height * DEPTH_PER_HEIGHT;
            }
        }
        for y in 0..GRID {
            for x in 0..GRID {
                let a = y * VERTICES + x;
                let (b, c, d) = (a + 1, a + VERTICES, a + VERTICES + 1);
                r.triangle(a, b, d, land);
                r.triangle(a, d, c, land);
            }
        }
        // The sides of the block of ground that face the viewer.
        let near_v = if cos > 0.0 { GRID } else { 0 };
        let near_u = if sin > 0.0 { GRID } else { 0 };
        for (along_u, position, light) in [(true, near_v, 0.55 + 0.3 * sin.abs()), (false, near_u, 0.55 + 0.3 * cos.abs())] {
            let mut tops = [0usize; VERTICES];
            for (n, top) in tops.iter_mut().enumerate() {
                let (x, y) = if along_u { (n, position) } else { (position, n) };
                let (i, foot) = (y * VERTICES + x, feet + n);
                let (u, v) = (x as f32 * step - 1.0, y as f32 * step - 1.0);
                let depth = u * sin + v * cos;
                r.sx[foot] = cx + (u * cos - v * sin) * scale;
                r.sy[foot] = cy + (depth * TILT + BASE) * scale;
                r.sz[foot] = depth * DEPTH_PER_GROUND - BASE * DEPTH_PER_HEIGHT;
                *top = i;
            }
            for n in 0..GRID {
                let (a, b, c, d) = (tops[n], tops[n + 1], feet + n, feet + n + 1);
                r.flat(a, b, d, light);
                r.flat(a, d, c, light);
            }
        }
        // Average the samples into the frame.
        let mut pixels = Vec::with_capacity(width * height);
        let count = (samples * samples) as f32;
        for y in 0..height {
            for x in 0..width {
                let mut sum = [0.0f32; 3];
                for sy in 0..samples {
                    for sx in 0..samples {
                        let c = r.colours[(y * samples + sy) * w + x * samples + sx];
                        sum = [sum[0] + c[0], sum[1] + c[1], sum[2] + c[2]];
                    }
                }
                pixels.push(Color32::from_rgb((sum[0] / count) as u8, (sum[1] / count) as u8, (sum[2] / count) as u8));
            }
        }
        ColorImage::new([width, height], pixels)
    }

    /// Draw the land of `brush` (known by `key`) at `t` in the loop.
    pub fn paint(ui: &Ui, rect: Rect, key: &str, brush: &Brush, t: f64) {
        let id = egui::Id::unique("card relief");
        let cached: Option<(String, std::sync::Arc<Land>)> = ui.ctx().data(|d| d.get_temp(id));
        let land = match cached {
            Some((k, land)) if k == key => land,
            _ => {
                let land = std::sync::Arc::new(land(brush));
                ui.ctx().data_mut(|d| d.insert_temp(id, (key.to_owned(), land.clone())));
                land
            }
        };
        let ppp = ui.ctx().pixels_per_point();
        let (w, h) = ((rect.width() * ppp).round() as usize, (rect.height() * ppp).round() as usize);
        let image = frame(&land, t, w.max(8), h.max(8));
        let texture = super::texture(ui, "relief", image, TextureOptions::LINEAR);
        ui.painter().image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }

    #[cfg(test)]
    pub fn frame_of(brush: &Brush, t: f64, w: usize, h: usize) -> ColorImage {
        frame(&land(brush), t, w, h)
    }
}

#[cfg(test)]
pub use relief::frame_of as relief_frame;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_card_with_its_animation() {
        for (_, _, tools) in Tool::GROUPS {
            for &tool in tools {
                let card = tool_card(tool);
                assert!(!card.body.is_empty() && !card.actions.is_empty(), "{tool:?}");
                let Some(Preview::Demo(demo)) = card.preview else { panic!("{tool:?} has no animation") };
                // The land, or what lies on it, changes as the tool works.
                let (mut a, mut b) = (Scene::new(), Scene::new());
                demo.act(&mut a, 0.05);
                demo.act(&mut b, 1.0);
                let look = |s: &Scene| (s.render().pixels, s.swatch, s.markers.len(), s.trees.len());
                assert!(look(&a) != look(&b), "{tool:?}: nothing happens");
            }
        }
    }

    /// Cards drawn as they show: a tool's animation at a few moments of its
    /// loop, a brush's land. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn cards_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let brush = Brush::default();
        for (name, which) in [("card-cover", 0), ("card-river", 1), ("card-brush", 2), ("card-objects", 3)] {
            let brush = brush.clone();
            let mut h = egui_kittest::Harness::builder().with_size(vec2(320.0, 420.0)).with_pixels_per_point(2.0).wgpu().build_ui(move |ui| {
                crate::theme::install(ui.ctx());
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    let card = match which {
                        0 => tool_card(Tool::Cover),
                        1 => tool_card(Tool::River),
                        3 => tool_card(Tool::Objects),
                        _ => Card {
                            title: "Sine Circle".into(),
                            shortcut: None,
                            body: "The picture shows the land this brush raises.",
                            actions: vec![("Scroll", "On the map: change its size")],
                            tip: None,
                            preview: Some(Preview::Relief { key: "sine".into(), brush: &brush }),
                        },
                    };
                    show(ui, &card);
                });
            });
            // Into the loop: the tool at work.
            for _ in 0..140 {
                h.step();
            }
            h.render().unwrap().save(dir.join(format!("{name}.png"))).unwrap();
        }
    }

    #[test]
    fn a_brush_turns_its_land() {
        let brush = Brush::default();
        let (a, b) = (relief_frame(&brush, 0.0, 120, 60), relief_frame(&brush, 0.25, 120, 60));
        assert_eq!(a.size, [120, 60]);
        // The land fills a good part of the frame, lit, and turns.
        let lit = a.pixels.iter().filter(|p| p.r() > 120).count();
        assert!(lit > 120 * 60 / 6, "{lit}");
        assert!(a.pixels != b.pixels);
    }
}
