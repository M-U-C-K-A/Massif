//! Edit ▸ Ores: the ores the export lays in the rock, in a window.
//!
//! Across the top, every ore with what the map will hold of it, and the
//! map small, to pick where to cut it. On the left, the map cut through
//! there: its rock, its water and the ores exactly as the export will lay
//! them, and over it a candle for each band of each ore — the wick from the
//! lowest height a vein starts at to the highest, the body where veins are
//! the most common, its width the size of a vein. Drag a candle's ends, its
//! body or its side; the wheel over its body sets how many veins a chunk
//! gets. On the right, the ore expected at each height of the map, in
//! blocks, on a log scale, or as a share of the ground. Below, the details
//! of the ore picked: the biomes it keeps to, and its bands in numbers, the
//! band picked or pointed at in the cut lit there.
//!
//! Delete (or Backspace) removes the ore picked, ⌘Z (Ctrl+Z) brings back
//! what was removed; the + at the end of the ores adds one back.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{self, Part};
use egui::{Align, Align2, Color32, ColorImage, FontId, Key, KeyboardShortcut, Layout, Modifiers, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, TextureHandle, TextureOptions, Ui, pos2, vec2};
use massif_core::World;
use massif_core::analysis::{Mineral, thousands};
use massif_core::compose::{AutoBiomes, BiomeColumn, Materializer, Span, TileStack, hash3};
use massif_core::ores::{Band, BiomeChoice, Host, OreRule, OreSettings, Placer, Preset, RockProfile, rock_at};
use massif_core::registry::{BiomeId, Registry};
use massif_core::{TILE_SIZE, TilePos};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};

/// The window's width, the cut's share of it, the room between parts.
const WIDTH: f32 = 1140.0;
const CUT_W: f32 = 690.0;
const GAP: f32 = 18.0;
/// The gutter of the height labels, and the cards' header strips.
const AXIS: f32 = 38.0;
const STRIP: f32 = 38.0;
/// Room under a plot for the amounts' labels.
const FOOT: f32 = 24.0;
/// The width of the tags at the left of the details' rows.
const TAG: f32 = 58.0;

/// How the chart reads amounts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scale {
    Blocks,
    Log,
    Share,
}

/// What the window wants done.
pub enum Choice {
    Keep,
    /// Close, keeping these settings.
    Close(OreSettings),
}

/// A part of a candle that drags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Grip {
    Min,
    Max,
    PeakLo,
    PeakHi,
    Body,
    Size,
}

struct Drag {
    ore: usize,
    band: usize,
    grip: Grip,
    from: Pos2,
    band0: Band,
}

/// Something removed, and where it was.
enum Removed {
    Ore(usize, OreRule),
    /// Of the ore of this mineral, band `.1`.
    Band(Mineral, usize, Band),
}

/// The map cut through along a row, as images.
struct Cut {
    /// The row, the first block across, how many, the heights shown.
    key: (i32, i32, usize, i32, i32),
    /// The runs of each column of the cut.
    runs: Vec<Vec<Span>>,
    /// The biomes the export writes over each column of the cut.
    biomes: Vec<BiomeColumn>,
    terrain: TextureHandle,
    /// The ores over it, for the settings of this key, and how many blocks
    /// of each ore it shows.
    ores: Option<(u64, TextureHandle, Vec<usize>)>,
}

/// The expected ore at each height, for the settings of `key` (and
/// whether the map's rock was read).
struct Series {
    key: (u64, bool),
    /// By ore of the settings: blocks a chunk of rock at each height from
    /// `y0`, smoothed over a vein's spread.
    per_chunk: Vec<Vec<f64>>,
    /// By ore: the rock at each height from `y0` in the biomes it keeps to,
    /// in columns (`None` until the map's rock is read).
    rock: Option<Vec<Vec<f64>>>,
}

pub struct OresView {
    pub settings: OreSettings,
    world: Arc<World>,
    dim: usize,
    selected: usize,
    band: usize,
    scale: Scale,
    /// The row cut through, and the block at the left of the cut.
    cut_z: i32,
    view_x: f64,
    cut: Option<Cut>,
    series: Option<Series>,
    rock: Option<RockProfile>,
    rock_rx: Option<Receiver<(Option<RockProfile>, Option<ColorImage>)>>,
    cancel: Arc<AtomicBool>,
    picture: Option<TextureHandle>,
    drag: Option<Drag>,
    /// Panning the cut: where the pointer was.
    pan: Option<f32>,
    /// The map's blocks: west, north, east, south (the last two past it).
    area: [i32; 4],
    /// Where the cut was last drawn (for tests).
    last_plot: Rect,
    /// What the map holds of each ore, for the settings of the key and
    /// whether its rock was read.
    totals: Option<(u64, bool, Vec<f64>)>,
    /// The blocks across the cut shows: the first, how many.
    shown: (i32, usize),
    /// The ores and bands removed, the last last: ⌘Z puts them back,
    /// whatever was changed since.
    undo: Vec<Removed>,
    /// The ores removed while the window is open, as they were: the + adds
    /// them back so.
    removed: Vec<OreRule>,
    /// The band of the picked ore whose row the pointer is over in the
    /// details (its candle lit in the cut), and the candle the pointer is
    /// over in the cut (its row lit in the details).
    row_hover: Option<usize>,
    cut_hover: Option<(usize, usize)>,
}

impl Drop for OresView {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

impl OresView {
    /// The window over a cold copy of the map, cut through its middle.
    pub fn new(ctx: &egui::Context, world: World, dim: usize) -> Self {
        let area = world.dims.get(dim).and_then(|d| d.bounds()).map_or([0, 0, TILE_SIZE, TILE_SIZE], |(lo, hi)| {
            [lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE]
        });
        let settings = world.meta.ores.clone();
        let world = Arc::new(world);
        let cancel = Arc::new(AtomicBool::new(false));
        let (tx, rx) = channel();
        let (w, c, ctx2) = (world.clone(), cancel.clone(), ctx.clone());
        std::thread::Builder::new()
            .name("massif ores".into())
            .spawn(move || {
                let rock = massif_core::ores::rock_profile(&w, dim, &c);
                let picture = massif_core::thumbnail::render(&w, 192).map(|img| ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()));
                let _ = tx.send((rock, picture));
                ctx2.request_repaint();
            })
            .expect("a thread for the ores");
        let selected = settings.ores.iter().position(|o| o.mineral == Mineral::Diamond).unwrap_or(0);
        Self {
            settings,
            dim,
            selected,
            band: 0,
            scale: Scale::Blocks,
            cut_z: (area[1] + area[3]) / 2,
            // Set to the middle of the map once the cut's width is known.
            view_x: f64::NAN,
            cut: None,
            series: None,
            rock: None,
            rock_rx: Some(rx),
            cancel,
            picture: None,
            drag: None,
            pan: None,
            area,
            world,
            last_plot: Rect::NOTHING,
            totals: None,
            shown: (0, 256),
            undo: Vec::new(),
            removed: Vec::new(),
            row_hover: None,
            cut_hover: None,
        }
    }

    /// Where a grip of band `band` of ore `ore` was last drawn (for tests).
    #[cfg(test)]
    pub fn grip(&self, ore: usize, band: usize, top: bool) -> Pos2 {
        let plot = self.last_plot;
        let (y0, y1) = self.heights();
        let scale = plot.height() / (y1 - y0) as f32;
        let candles = self.candles(plot, |y| plot.bottom() - (y - y0 as f32) * scale);
        let c = candles.iter().find(|c| c.ore == ore && c.band == band).expect("the candle");
        pos2(c.x, if top { c.top } else { c.bottom })
    }

    /// Whether the map's rock has been read (for tests).
    #[cfg(test)]
    pub fn is_ready(&self) -> bool {
        self.rock_rx.is_none() && self.cut.as_ref().is_some_and(|c| c.ores.is_some())
    }

    /// Pick ore `k` (for tests).
    #[cfg(test)]
    pub fn select(&mut self, k: usize) {
        self.selected = k;
        self.band = 0;
    }

    /// The ore picked and its band picked (for tests).
    #[cfg(test)]
    pub fn picked(&self) -> (usize, usize) {
        (self.selected, self.band)
    }

    /// Pick band `band` of ore `ore` (for tests).
    #[cfg(test)]
    pub fn select_band(&mut self, ore: usize, band: usize) {
        self.selected = ore;
        self.band = band;
    }

    /// The blocks of ore `k` the cut shows, as the export lays them (for tests).
    #[cfg(test)]
    pub fn in_cut(&self, k: usize) -> usize {
        self.cut.as_ref().and_then(|c| c.ores.as_ref()).and_then(|o| o.2.get(k).copied()).unwrap_or(0)
    }

    /// The blocks of ore `k` the map is expected to hold (for tests).
    #[cfg(test)]
    pub fn total(&self, k: usize) -> f64 {
        self.total_on_map(k)
    }

    /// The biomes whose rock the map has, the one with the most first (for tests).
    #[cfg(test)]
    pub fn biomes_by_rock(&self) -> Vec<u16> {
        let rock = self.rock.as_ref().expect("the map's rock");
        let sum = |b: usize| rock.by_biome[b].iter().sum::<f64>();
        let mut list: Vec<usize> = (0..rock.by_biome.len()).filter(|&b| sum(b) > 0.0).collect();
        list.sort_by(|&a, &b| sum(b).total_cmp(&sum(a)));
        list.into_iter().map(|b| b as u16).collect()
    }

    /// The candle pointed at in the cut, and the band whose row is pointed
    /// at in the details (for tests).
    #[cfg(test)]
    pub fn lit(&self) -> (Option<(usize, usize)>, Option<usize>) {
        (self.cut_hover, self.row_hover)
    }

    /// Remove ore `k`, as Delete does: undone by ⌘Z, brought back as it was
    /// by the +.
    fn remove_ore(&mut self, k: usize) {
        if k >= self.settings.ores.len() {
            return;
        }
        let rule = self.settings.ores.remove(k);
        self.removed.retain(|r| r.mineral != rule.mineral);
        self.removed.push(rule.clone());
        self.undo.push(Removed::Ore(k, rule));
        // The ore after it is picked in its place, or the one before at the end.
        self.selected = k.min(self.settings.ores.len().saturating_sub(1));
        self.band = 0;
        self.drag = None;
        crate::sounds::play(crate::sounds::Cue::Tap);
    }

    /// Remove band `j` of the picked ore: undone by ⌘Z.
    fn remove_band(&mut self, j: usize) {
        let Some(rule) = self.settings.ores.get_mut(self.selected) else { return };
        if j >= rule.bands.len() {
            return;
        }
        let band = rule.bands.remove(j);
        self.undo.push(Removed::Band(rule.mineral, j, band));
        self.band = self.band.min(rule.bands.len().saturating_sub(1));
        self.drag = None;
    }

    /// Add ore `mineral` back among the others: as it was when removed
    /// here, else as the preset for this world's heights has it.
    fn add_ore(&mut self, mineral: Mineral) {
        let rule = match self.removed.iter().rposition(|r| r.mineral == mineral) {
            Some(k) => self.removed.remove(k),
            None => {
                let floor = self.world.dims[self.dim].min_y;
                match Preset::for_floor(floor).rule(mineral).or_else(|| Preset::Modern.rule(mineral)) {
                    Some(rule) => rule,
                    None => return,
                }
            }
        };
        self.selected = self.settings.insert(rule);
        self.band = 0;
    }

    /// Put back what the last removal took, where it was, and pick it. A
    /// removal that no longer applies (the ore added back with the +, or a
    /// band whose ore is gone) is passed over.
    fn undo(&mut self) -> bool {
        while let Some(last) = self.undo.pop() {
            match last {
                Removed::Ore(k, rule) => {
                    if self.settings.ores.iter().any(|r| r.mineral == rule.mineral) {
                        continue;
                    }
                    self.removed.retain(|r| r.mineral != rule.mineral);
                    let k = k.min(self.settings.ores.len());
                    self.settings.ores.insert(k, rule);
                    (self.selected, self.band) = (k, 0);
                }
                Removed::Band(mineral, j, band) => {
                    let Some(k) = self.settings.ores.iter().position(|r| r.mineral == mineral) else { continue };
                    let bands = &mut self.settings.ores[k].bands;
                    let j = j.min(bands.len());
                    bands.insert(j, band);
                    (self.selected, self.band) = (k, j);
                }
            }
            self.drag = None;
            crate::sounds::play(crate::sounds::Cue::Tap);
            return true;
        }
        false
    }

    fn key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}", self.settings).hash(&mut h);
        h.finish()
    }

    /// The heights shown: the floor of the world up to a little over its
    /// highest ground (the game's 320 until the map is read).
    fn heights(&self) -> (i32, i32) {
        let d = &self.world.dims[self.dim];
        let top = match &self.rock {
            Some(r) => r.solid.iter().rposition(|&s| s > 0.0).map_or(d.max_y, |k| r.min_y + k as i32 + 40),
            None => 320,
        };
        let top = top.clamp((d.min_y + 64).min(d.max_y), d.max_y);
        (d.min_y, ((top + 15).div_euclid(16) * 16).min(d.max_y))
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let ctx = ui.ctx().clone();
        if let Some(rx) = &self.rock_rx
            && let Ok((rock, picture)) = rx.try_recv()
        {
            self.rock = rock;
            self.picture = picture.map(|img| ctx.load_texture("ores map", img, TextureOptions::LINEAR));
            self.rock_rx = None;
            self.cut = None;
        } else if self.rock_rx.is_some() {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }
        // Delete (Backspace on a Mac keyboard) removes the ore picked, ⌘Z
        // puts back what was removed; neither while a number is typed (the
        // field has its own) or a menu is open.
        if !ctx.text_edit_focused() && !egui::Popup::is_any_open(&ctx) {
            let (delete, undo) = ui.input_mut(|i| {
                let delete = i.consume_key(Modifiers::NONE, Key::Delete) || i.consume_key(Modifiers::NONE, Key::Backspace);
                (delete, i.consume_shortcut(&KeyboardShortcut::new(Modifiers::COMMAND, Key::Z)))
            });
            if delete && self.drag.is_none() {
                self.remove_ore(self.selected);
            }
            if undo {
                self.undo();
            }
        }
        self.selected = self.selected.min(self.settings.ores.len().saturating_sub(1));
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        let room = ctx.content_rect().height() - 150.0;
        let plot_h = (room - 400.0).clamp(220.0, 400.0);
        ui.set_width(WIDTH);
        self.header(ui, &pal, &mut choice);
        ui.add_space(14.0);
        self.strip(ui, &pal);
        ui.add_space(14.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = GAP;
            let (cut_rect, _) = ui.allocate_exact_size(vec2(CUT_W, STRIP + plot_h + FOOT), Sense::hover());
            let (chart_rect, _) = ui.allocate_exact_size(vec2(WIDTH - CUT_W - GAP, STRIP + plot_h + FOOT), Sense::hover());
            self.cut_card(ui, &pal, cut_rect);
            self.chart_card(ui, &pal, chart_rect);
        });
        ui.add_space(14.0);
        self.inspector(ui, &pal);
        if matches!(choice, Choice::Keep) {
            choice
        } else {
            Choice::Close(self.settings.clone())
        }
    }

    // ------------------------------------------------------------ top --

    fn header(&mut self, ui: &mut Ui, pal: &Palette, choice: &mut Choice) {
        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(theme::glyph(Icon::Pickaxe).to_string()).size(18.0).color(pal.accent));
                    ui.label(RichText::new("Ores").size(19.0).bold());
                });
                ui.add_space(3.0);
                let preset = self.settings.preset().map_or("Custom", |p| p.label());
                let per_chunk: f64 = self.settings.ores.iter().map(|o| self.settings.per_chunk(o)).sum();
                let words = if self.settings.on {
                    format!("Laid in the rock by the export · {preset} · about {} ore blocks a chunk of rock", thousands(per_chunk.round() as u64))
                } else {
                    "Off: the export lays no ore, and Minecraft adds none to the map's finished chunks".to_owned()
                };
                ui.label(RichText::new(words).size(12.5).color(pal.weak));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.x = 10.0;
                let done = egui::Button::new(RichText::new(theme::with_icon(Icon::Check, "Done")).size(13.0).medium())
                    .fill(if pal.dark { theme::mix(pal.panel, pal.text, 0.04) } else { Color32::WHITE })
                    .stroke(Stroke::new(1.0, pal.control_border()))
                    .corner_radius(theme::PILL)
                    .min_size(vec2(0.0, 32.0));
                if ui.add(done).clicked() {
                    *choice = Choice::Close(OreSettings::default());
                }
                ui.add_space(4.0);
                widgets::switch(ui, &mut self.settings.on).on_hover_text("Whether the export lays ores in the rock");
                ui.label(RichText::new("Lay ores").size(12.5));
                ui.add_space(8.0);
                ui.add_enabled(
                    self.settings.on,
                    widgets::Field::new(&mut self.settings.richness, 0.0..=4.0).max_decimals(2).suffix("×").text("Richness").width(150.0),
                )
                .on_hover_text("Every band's veins, times this");
                let label = self.settings.preset().map_or("Custom", |p| p.label());
                widgets::combo("ore preset").selected_text(label).width(150.0).show_ui(ui, |ui| {
                    for p in Preset::ALL {
                        if ui.selectable_label(self.settings.preset() == Some(p), p.label()).clicked() {
                            let (on, richness) = (self.settings.on, self.settings.richness);
                            self.settings = p.settings();
                            self.settings.on = on;
                            self.settings.richness = richness;
                            self.band = 0;
                        }
                    }
                });
            });
        });
    }

    /// Every ore with what the map holds of it, and the map to pick the cut.
    fn strip(&mut self, ui: &mut Ui, pal: &Palette) {
        let (rect, _) = ui.allocate_exact_size(vec2(WIDTH, 64.0), Sense::hover());
        let p = ui.painter().clone();
        p.rect(rect, theme::CARD_RADIUS, card_fill(pal), Stroke::new(1.0, pal.border), StrokeKind::Inside);
        // The map, small: a click picks the row cut through.
        let map = Rect::from_min_size(rect.min + vec2(8.0, 8.0), vec2(48.0, 48.0));
        let [ax0, az0, ax1, az1] = self.area;
        let (w, d) = ((ax1 - ax0).max(1) as f32, (az1 - az0).max(1) as f32);
        let side = w.max(d);
        let inner = Rect::from_center_size(map.center(), vec2(48.0 * w / side, 48.0 * d / side));
        match &self.picture {
            Some(t) => {
                egui::Image::new(t).corner_radius(6).paint_at(ui, inner);
            }
            None => {
                p.rect_filled(inner, 6, pal.well());
            }
        }
        let r = ui.interact(map, egui::Id::unique("ores cut picker"), Sense::click_and_drag()).on_hover_text("Where to cut the map: click or drag");
        if (r.clicked() || r.dragged())
            && let Some(at) = r.interact_pointer_pos()
        {
            let u = ((at.x - inner.left()) / inner.width()).clamp(0.0, 1.0);
            let v = ((at.y - inner.top()) / inner.height()).clamp(0.0, 1.0);
            self.cut_z = az0 + (v * (az1 - az0) as f32) as i32;
            let shown = self.shown.1 as f64;
            self.view_x = ax0 as f64 + u as f64 * (ax1 - ax0) as f64 - shown / 2.0;
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
        }
        let ly = inner.top() + (self.cut_z - az0) as f32 / (az1 - az0).max(1) as f32 * inner.height();
        p.hline(inner.x_range(), ly, Stroke::new(1.5, pal.accent));
        {
            let (first, across) = self.shown;
            let x0 = inner.left() + (first - ax0) as f32 / (ax1 - ax0).max(1) as f32 * inner.width();
            let x1 = x0 + across as f32 / (ax1 - ax0).max(1) as f32 * inner.width();
            p.rect_stroke(Rect::from_min_max(pos2(x0.max(inner.left()), ly - 3.0), pos2(x1.min(inner.right()), ly + 3.0)), 1, Stroke::new(1.0, Color32::WHITE), StrokeKind::Middle);
        }
        p.rect_stroke(inner, 6, Stroke::new(1.0, pal.border), StrokeKind::Inside);

        // The ores, a cell each: a click picks one.
        let key = (self.key(), self.rock.is_some());
        if self.totals.as_ref().is_none_or(|t| (t.0, t.1) != key) {
            let totals = (0..self.settings.ores.len()).map(|k| self.total_on_map(k)).collect();
            self.totals = Some((key.0, key.1, totals));
        }
        let totals = self.totals.as_ref().map(|t| t.2.clone()).unwrap_or_default();
        let n = self.settings.ores.len().max(1);
        // The ores taken out, to add back at the end.
        let missing = self.settings.missing();
        let add_w = if missing.is_empty() { 0.0 } else { 46.0 };
        let cells = Rect::from_min_max(pos2(map.right() + 14.0, rect.top()), pos2(rect.right() - add_w, rect.bottom()));
        let cw = cells.width() / n as f32;
        if self.settings.ores.is_empty() {
            p.text(pos2(cells.left() + 12.0, cells.center().y), Align2::LEFT_CENTER, "No ore: the export lays none. Add one with +", FontId::proportional(12.5), pal.weak);
        }
        let reg = &self.world.registry;
        for k in 0..self.settings.ores.len() {
            let rule = &self.settings.ores[k];
            let cell = Rect::from_min_size(pos2(cells.left() + k as f32 * cw, rect.top()), vec2(cw, rect.height()));
            let r = ui.interact(cell, egui::Id::unique(("ores cell", k)), Sense::click());
            let on = self.settings.on && rule.on;
            let color = theme::rgb(rule.mineral.color());
            if k == self.selected {
                p.rect_filled(cell.shrink2(vec2(3.0, 6.0)), theme::TILE_RADIUS, selected_fill(pal));
                p.hline(cell.shrink2(vec2(14.0, 0.0)).x_range(), cell.bottom() - 6.5, Stroke::new(2.0, color));
            } else if r.hovered() {
                p.rect_filled(cell.shrink2(vec2(3.0, 6.0)), theme::TILE_RADIUS, hover_fill(pal));
            }
            if k > 0 {
                p.vline(cell.left(), cell.y_range().shrink(14.0), Stroke::new(1.0, pal.border));
            }
            let sq = Rect::from_min_size(pos2(cell.left() + 12.0, cell.top() + 15.0), vec2(8.0, 8.0));
            p.rect_filled(sq, 2, if on { color } else { color.gamma_multiply(0.35) });
            let name = p.text(pos2(sq.right() + 6.0, sq.center().y), Align2::LEFT_CENTER, rule.mineral.name(), FontId::proportional(11.5), pal.weak);
            // Kept to some biomes: a funnel, as a brush's filter shows.
            if !rule.biomes.is_everywhere() {
                p.text(pos2(name.right() + 5.0, sq.center().y), Align2::LEFT_CENTER, theme::glyph(Icon::Funnel).to_string(), FontId::proportional(10.5), pal.accent);
            }
            let total = totals.get(k).copied().unwrap_or(0.0);
            let (value, unit) = if on { short(total) } else { ("off".to_owned(), "") };
            let g = p.layout_no_wrap(value, FontId::proportional(17.0), if on { pal.text } else { pal.weak });
            let at = pos2(cell.left() + 12.0, cell.top() + 30.0);
            let gw = g.size().x;
            p.galley(at, g, pal.text);
            if !unit.is_empty() {
                p.text(pos2(at.x + gw + 3.0, at.y + 14.0), Align2::LEFT_BOTTOM, unit, FontId::proportional(11.0), pal.weak);
            }
            if r.clicked() && k != self.selected {
                self.selected = k;
                self.band = 0;
                crate::sounds::play(crate::sounds::Cue::Select);
            }
            if r.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            let mut tip = format!("{}: for {}. About {} blocks a chunk of rock.", rule.mineral.name(), rule.mineral.use_for(), thousands(self.settings.per_chunk(rule).round() as u64));
            if let Some(words) = biome_words(&rule.biomes, reg) {
                tip = format!("{tip} {words}.");
            }
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, k == self.selected, rule.mineral.name()));
            r.on_hover_text(tip);
        }

        // The +: an ore taken out, back among the others.
        if !missing.is_empty() {
            let cell = Rect::from_min_max(pos2(cells.right(), rect.top()), rect.max);
            let r = ui.interact(cell, egui::Id::unique("ores add"), Sense::click());
            if r.hovered() || egui::Popup::is_id_open(ui.ctx(), egui::Popup::default_response_id(&r)) {
                p.rect_filled(cell.shrink2(vec2(5.0, 6.0)), theme::TILE_RADIUS, hover_fill(pal));
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            if !self.settings.ores.is_empty() {
                p.vline(cell.left(), cell.y_range().shrink(14.0), Stroke::new(1.0, pal.border));
            }
            p.text(cell.center(), Align2::CENTER_CENTER, theme::glyph(Icon::Plus).to_string(), FontId::proportional(16.0), pal.text);
            r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, "Add an ore"));
            let r = r.on_hover_text("Add an ore taken out");
            let mut add = None;
            egui::Popup::menu(&r).show(|ui| {
                // Each ore with its colour, in the room left of its name.
                ui.spacing_mut().button_padding.x = 24.0;
                for &m in &missing {
                    let button = ui.button(m.name());
                    let sq = Rect::from_center_size(pos2(button.rect.left() + 12.0, button.rect.center().y), vec2(8.0, 8.0));
                    ui.painter().rect_filled(sq, 2, theme::rgb(m.color()));
                    if button.clicked() {
                        add = Some(m);
                    }
                }
            });
            if let Some(m) = add {
                self.add_ore(m);
                crate::sounds::play(crate::sounds::Cue::Select);
            }
        }
    }

    /// The ore blocks the map is expected to hold of ore `k`, in the
    /// biomes it keeps to.
    fn total_on_map(&self, k: usize) -> f64 {
        let (y0, y1) = (self.world.dims[self.dim].min_y, self.world.dims[self.dim].max_y);
        let columns = self.rock.as_ref().map_or(((self.area[2] - self.area[0]) * (self.area[3] - self.area[1])) as f64, |r| r.columns as f64);
        let one = series(&self.settings, y0, y1, (0, false));
        let (Some(per_chunk), Some(rule)) = (one.per_chunk.get(k), self.settings.ores.get(k)) else { return 0.0 };
        per_chunk.iter().enumerate().map(|(i, &e)| e * self.rock.as_ref().map_or(columns, |r| r.rock_in(&rule.biomes, y0 + i as i32)) / 256.0).sum()
    }

    // ------------------------------------------------------------ cut --

    fn cut_card(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect) {
        let p = ui.painter().clone();
        p.rect(rect, theme::CARD_RADIUS, card_fill(pal), Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
        let plot = Rect::from_min_max(pos2(rect.left() + AXIS, strip.bottom() + 8.0), pos2(rect.right() - 10.0, rect.bottom() - FOOT + 4.0));
        self.last_plot = plot;
        let (y0, y1) = self.heights();
        let scale = plot.height() / (y1 - y0) as f32;
        let across = (plot.width() / scale).floor().max(16.0) as usize;
        let [ax0, _, ax1, _] = self.area;
        if self.view_x.is_nan() {
            self.view_x = (ax0 + ax1) as f64 / 2.0 - across as f64 / 2.0;
        }
        self.view_x = self.view_x.clamp(ax0 as f64, (ax1 as f64 - across as f64).max(ax0 as f64));
        let x0 = self.view_x.floor() as i32;
        title(&p, pal, strip, Icon::Layers, &format!("cut at z {} · x {} to {}", self.cut_z, x0, x0 + across as i32));
        p.hline(strip.x_range(), strip.bottom(), Stroke::new(1.0, pal.border));
        p.text(pos2(strip.right() - 14.0, strip.center().y), Align2::RIGHT_CENTER, "drag the candles · wheel on a body: veins", FontId::proportional(11.0), pal.weak);

        // The images, made again when the cut or the settings change.
        // Made twice as wide as shown and kept while the view stays in it:
        // panning redraws it only now and then.
        self.shown = (x0, across);
        let fits = |c: &Cut| (c.key.0, c.key.3, c.key.4) == (self.cut_z, y0, y1) && c.key.1 <= x0 && x0 + across as i32 <= c.key.1 + c.key.2 as i32;
        if self.cut.as_ref().is_none_or(|c| !fits(c)) {
            let start = (x0 - across as i32 / 2).max(ax0);
            let width = (2 * across).min((ax1 - start).max(across as i32) as usize);
            self.cut = Some(self.make_cut(ui.ctx(), (self.cut_z, start, width, y0, y1)));
        }
        let settings_key = self.key();
        if let Some(cut) = &mut self.cut
            && cut.ores.as_ref().is_none_or(|o| o.0 != settings_key)
        {
            let (image, counts) = ores_image(&self.world, self.dim, &self.settings, cut);
            cut.ores = Some((settings_key, ui.ctx().load_texture("ores cut ores", image, TextureOptions::NEAREST), counts));
        }

        // Sky, ground, ores.
        let painter = p.with_clip_rect(plot);
        let sky_top = if pal.dark { Color32::from_rgb(16, 24, 40) } else { Color32::from_rgb(140, 182, 236) };
        let sky_low = if pal.dark { Color32::from_rgb(34, 46, 66) } else { Color32::from_rgb(196, 220, 246) };
        gradient(&painter, plot, sky_top, sky_low);
        let shown = Rect::from_min_size(plot.min, vec2(across as f32 * scale, plot.height()));
        if let Some(cut) = &self.cut {
            let w = cut.key.2 as f32;
            let u0 = (x0 - cut.key.1) as f32 / w;
            let uv = Rect::from_min_max(pos2(u0, 0.0), pos2(u0 + across as f32 / w, 1.0));
            painter.image(cut.terrain.id(), shown, uv, Color32::WHITE);
            if let Some((_, t, _)) = &cut.ores {
                painter.image(t.id(), shown, uv, if self.settings.on { Color32::WHITE } else { Color32::from_white_alpha(60) });
            }
        }
        let to_y = |y: f32| plot.bottom() - (y - y0 as f32) * scale;
        // Heights: a hairline every 16 blocks, a label every 32.
        let step = if scale < 0.6 { 64 } else { 16 };
        let mut y = (y0 + step - 1).div_euclid(step) * step;
        while y < y1 {
            let sy = to_y(y as f32);
            painter.hline(plot.x_range(), sy, Stroke::new(1.0, Color32::from_white_alpha(if y % (step * 2) == 0 { 22 } else { 10 })));
            if y % (step * 2) == 0 {
                p.text(pos2(plot.left() - 8.0, sy), Align2::RIGHT_CENTER, y.to_string(), FontId::monospace(10.5), pal.weak);
            }
            y += step;
        }
        let sea = self.world.meta.sea_level;
        if (y0..y1).contains(&sea) {
            dashed_h(&painter, plot.x_range(), to_y(sea as f32), Stroke::new(1.0, Color32::from_rgb(96, 160, 250).gamma_multiply(0.8)));
            painter.text(pos2(plot.left() + 6.0, to_y(sea as f32) - 2.0), Align2::LEFT_BOTTOM, format!("sea {sea}"), FontId::monospace(10.0), Color32::from_rgb(140, 190, 255));
        }

        // Panning, unless a candle is held.
        let id = egui::Id::unique("ores cut");
        let r = ui.interact(plot, id, Sense::click_and_drag());
        let pointer = r.hover_pos();
        let candles = self.candles(plot, to_y);
        let hit = pointer.and_then(|at| self.hit(&candles, at));
        // Where the button went down: a drag starts once the pointer has
        // moved a little, the candle is the one pressed.
        if r.drag_started()
            && let Some(at) = ui.input(|i| i.pointer.press_origin()).or(r.interact_pointer_pos())
        {
            match self.hit(&candles, at) {
                Some((ore, band, grip)) => {
                    self.selected = ore;
                    self.band = band;
                    self.drag = Some(Drag { ore, band, grip, from: at, band0: self.settings.ores[ore].bands[band] });
                }
                None => self.pan = Some(at.x),
            }
        }
        if r.dragged()
            && let Some(at) = r.interact_pointer_pos()
        {
            if let Some(d) = &self.drag {
                let dy = ((d.from.y - at.y) / scale).round() as i32;
                let at_y = (y0 as f32 + (plot.bottom() - at.y) / scale).round() as i32;
                let mut b = d.band0;
                match d.grip {
                    Grip::Min => b.min = at_y.min(b.peak[0]),
                    Grip::Max => b.max = at_y.max(b.peak[1]),
                    Grip::PeakLo => b.peak[0] = at_y.clamp(b.min, b.peak[1]),
                    Grip::PeakHi => b.peak[1] = at_y.clamp(b.peak[0], b.max),
                    Grip::Body => {
                        b.min += dy;
                        b.max += dy;
                        b.peak[0] += dy;
                        b.peak[1] += dy;
                    }
                    Grip::Size => b.size = (d.band0.size as f32 + (at.x - d.from.x) / 1.2).round().clamp(1.0, massif_core::ores::MAX_SIZE as f32) as u16,
                }
                b.tidy();
                let (ore, band) = (d.ore, d.band);
                self.settings.ores[ore].bands[band] = b;
            } else if let Some(last) = self.pan {
                self.view_x -= ((at.x - last) / scale) as f64;
                self.pan = Some(at.x);
            }
        }
        if r.drag_stopped() {
            self.drag = None;
            self.pan = None;
        }
        // The candle held or pointed at: its row is lit in the details.
        self.cut_hover = self.drag.as_ref().map(|d| (d.ore, d.band)).or(hit.map(|h| (h.0, h.1)));
        // The wheel over a body: veins a chunk.
        if let Some((ore, band, Grip::Body)) = hit {
            let wheel = ui.input(|i| i.smooth_scroll_delta.y);
            if wheel.abs() > 0.5 {
                let b = &mut self.settings.ores[ore].bands[band];
                b.veins = (b.veins * 1.0025f32.powf(wheel)).clamp(0.05, massif_core::ores::MAX_VEINS);
                b.veins = (b.veins * 20.0).round() / 20.0;
            }
        }
        match (&self.drag, hit, self.pan) {
            (Some(d), ..) => ui.ctx().set_cursor_icon(if d.grip == Grip::Size { egui::CursorIcon::ResizeHorizontal } else { egui::CursorIcon::ResizeVertical }),
            (None, Some((_, _, Grip::Size)), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal),
            (None, Some((_, _, Grip::Body)), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            (None, Some(_), _) => ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical),
            (None, None, Some(_)) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing),
            (None, None, None) if r.hovered() => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            _ => {}
        }

        // The candles; the one of a row pointed at in the details lit too.
        let candles = self.candles(plot, to_y);
        for c in &candles {
            let lit = hit.is_some_and(|h| (h.0, h.1) == (c.ore, c.band)) || (c.ore == self.selected && self.row_hover == Some(c.band));
            self.draw_candle(&painter, pal, c, plot, lit);
        }
        // Names over the slots.
        let n = self.settings.ores.len().max(1);
        let slot = plot.width() / n as f32;
        for (k, rule) in self.settings.ores.iter().enumerate() {
            let x = plot.left() + slot * (k as f32 + 0.5);
            let ink = match (k == self.selected, pal.dark) {
                (true, _) => pal.text,
                (false, true) => Color32::from_white_alpha(170),
                (false, false) => pal.weak,
            };
            painter.text(pos2(x, plot.top() + 6.0), Align2::CENTER_TOP, rule.mineral.name().to_uppercase(), FontId::monospace(9.5), ink);
        }
        if let Some(d) = &self.drag {
            let b = self.settings.ores[d.ore].bands[d.band];
            let words = match d.grip {
                Grip::Min => format!("from y {}", b.min),
                Grip::Max => format!("up to y {}", b.max),
                Grip::PeakLo | Grip::PeakHi => format!("most from y {} to {}", b.peak[0], b.peak[1]),
                Grip::Body => format!("y {} to {}", b.min, b.max),
                Grip::Size => format!("{} blocks a vein", b.size),
            };
            if let Some(at) = pointer {
                bubble(&p, pal, at + vec2(14.0, -18.0), &words);
            }
        } else if let Some((ore, band, _)) = hit
            && let Some(at) = pointer
        {
            let b = self.settings.ores[ore].bands[band];
            let words = format!(
                "{} · y {} to {}, most at {} · {} veins a chunk · {} blocks a vein",
                self.settings.ores[ore].mineral.name(),
                b.min,
                b.max,
                if b.peak[0] == b.peak[1] { b.peak[0].to_string() } else { format!("{} to {}", b.peak[0], b.peak[1]) },
                trim(b.veins as f64),
                b.size
            );
            bubble(&p, pal, at + vec2(14.0, -18.0), &words);
        }
        if !self.settings.on {
            painter.rect_filled(plot, 0, Color32::from_black_alpha(90));
            painter.text(plot.center(), Align2::CENTER_CENTER, "Ores are off: the export lays none", FontId::proportional(14.0), Color32::WHITE);
        }
        p.rect_stroke(plot, 0, Stroke::new(1.0, pal.border), StrokeKind::Outside);
        // Across: blocks from the left of the cut.
        p.text(pos2(plot.left(), rect.bottom() - 11.0), Align2::LEFT_CENTER, format!("x {x0}"), FontId::monospace(10.5), pal.weak);
        p.text(pos2(plot.right(), rect.bottom() - 11.0), Align2::RIGHT_CENTER, format!("x {}", x0 + across as i32), FontId::monospace(10.5), pal.weak);
    }

    /// The candles of every band, where they stand over `plot`.
    fn candles(&self, plot: Rect, to_y: impl Fn(f32) -> f32) -> Vec<Candle> {
        let n = self.settings.ores.len().max(1);
        let slot = plot.width() / n as f32;
        let mut out = Vec::new();
        for (k, rule) in self.settings.ores.iter().enumerate() {
            let bands = rule.bands.len();
            for (j, b) in rule.bands.iter().enumerate() {
                let w = 9.0 + b.size as f32 * 0.32;
                let spread = (bands as f32 - 1.0) / 2.0;
                let x = plot.left() + slot * (k as f32 + 0.5) + (j as f32 - spread) * (slot / bands.max(1) as f32).min(w + 10.0);
                let clamp = |y: f32| y.clamp(plot.top() + 16.0, plot.bottom() - 2.0);
                out.push(Candle {
                    ore: k,
                    band: j,
                    x,
                    w,
                    top: clamp(to_y(b.max as f32 + 1.0)),
                    bottom: clamp(to_y(b.min as f32)),
                    body_top: clamp(to_y(b.peak[1] as f32 + 1.0)),
                    body_bottom: clamp(to_y(b.peak[0] as f32)),
                    past_top: to_y(b.max as f32 + 1.0) < plot.top() + 16.0,
                    past_bottom: to_y(b.min as f32) > plot.bottom() - 2.0,
                    color: theme::rgb(rule.mineral.color()),
                    on: rule.on && self.settings.on,
                });
            }
        }
        // The ore picked last: on top.
        out.sort_by_key(|c| c.ore == self.selected);
        out
    }

    /// The grip of a candle under the pointer, the picked ore's first.
    fn hit(&self, candles: &[Candle], at: Pos2) -> Option<(usize, usize, Grip)> {
        let mut best: Option<(f32, (usize, usize, Grip))> = None;
        for c in candles.iter().rev() {
            let body = c.body();
            let near = |p: Pos2| p.distance(at);
            let picked = c.ore == self.selected;
            let mut grips = vec![
                (near(pos2(c.x, c.top)), Grip::Max),
                (near(pos2(c.x, c.bottom)), Grip::Min),
                (near(pos2(body.right(), body.center().y)), Grip::Size),
            ];
            if body.height() >= 10.0 {
                grips.push(((at.y - body.top()).abs().max((at.x - c.x).abs() - c.w / 2.0), Grip::PeakHi));
                grips.push(((at.y - body.bottom()).abs().max((at.x - c.x).abs() - c.w / 2.0), Grip::PeakLo));
            } else {
                grips.push(((at.y - body.center().y).abs().max((at.x - c.x).abs() - c.w / 2.0), Grip::PeakHi));
            }
            for (d, grip) in grips {
                let reach = if picked { 8.0 } else { 6.0 };
                if d <= reach && best.is_none_or(|b| d < b.0) {
                    best = Some((d, (c.ore, c.band, grip)));
                }
            }
            if best.is_none() && body.expand2(vec2(2.0, 0.0)).contains(at) {
                best = Some((0.0, (c.ore, c.band, Grip::Body)));
            }
        }
        if let Some((_, (ore, band, grip))) = best {
            return Some((ore, band, grip));
        }
        // Along a wick: the body moves.
        candles.iter().rev().find(|c| (at.x - c.x).abs() <= 4.0 && at.y >= c.top && at.y <= c.bottom).map(|c| (c.ore, c.band, Grip::Body))
    }

    /// A band's candle; `lit` when it is pointed at, in the cut or by its
    /// row in the details.
    fn draw_candle(&self, painter: &egui::Painter, pal: &Palette, c: &Candle, plot: Rect, lit: bool) {
        let picked = c.ore == self.selected;
        let alpha = match (c.on, picked) {
            (false, _) => 0.3,
            (true, true) => 1.0,
            (true, false) => 0.62,
        };
        let color = c.color.gamma_multiply(alpha);
        let shadow = Stroke::new(4.0, Color32::from_black_alpha((110.0 * alpha) as u8));
        painter.line_segment([pos2(c.x, c.top), pos2(c.x, c.bottom)], shadow);
        painter.line_segment([pos2(c.x, c.top), pos2(c.x, c.bottom)], Stroke::new(2.0, color));
        // The body lets the cut show through, its edge in the ore's colour.
        let body = c.body();
        painter.rect_filled(body.expand(1.5), 3, Color32::from_black_alpha((90.0 * alpha) as u8));
        painter.rect(body, 3, c.color.gamma_multiply(alpha * if picked { 0.78 } else { 0.5 }), Stroke::new(1.2, color), StrokeKind::Inside);
        painter.rect_stroke(body.shrink(1.2), 2, Stroke::new(1.0, Color32::from_white_alpha((70.0 * alpha) as u8)), StrokeKind::Inside);
        if picked && self.band == c.band && c.on {
            painter.rect_stroke(body.expand(3.0), 5, Stroke::new(1.0, pal.accent), StrokeKind::Outside);
        } else if lit && c.on {
            painter.rect_stroke(body.expand(3.0), 5, Stroke::new(1.0, Color32::from_white_alpha(210)), StrokeKind::Outside);
        }
        // The grips of the picked ore, or of the one pointed at.
        if (picked || lit) && c.on {
            for (p, arrow) in [(pos2(c.x, c.top), c.past_top), (pos2(c.x, c.bottom), c.past_bottom)] {
                painter.circle(p, 4.5, Color32::WHITE, Stroke::new(1.5, c.color));
                if arrow {
                    let dir = if p.y < plot.center().y { -1.0 } else { 1.0 };
                    painter.line_segment([p + vec2(-3.0, dir * 7.0), p + vec2(0.0, dir * 10.0)], Stroke::new(1.5, Color32::WHITE));
                    painter.line_segment([p + vec2(3.0, dir * 7.0), p + vec2(0.0, dir * 10.0)], Stroke::new(1.5, Color32::WHITE));
                }
            }
            painter.hline(body.x_range().expand(2.0), body.top(), Stroke::new(2.0, Color32::WHITE));
            painter.hline(body.x_range().expand(2.0), body.bottom(), Stroke::new(2.0, Color32::WHITE));
            let side = pos2(body.right(), body.center().y);
            painter.rect(Rect::from_center_size(side, vec2(5.0, 12.0)), 2, Color32::WHITE, Stroke::new(1.0, c.color), StrokeKind::Inside);
        }
    }

    /// The map along the cut: its blocks, its fluid, its sky left clear.
    fn make_cut(&self, ctx: &egui::Context, key: (i32, i32, usize, i32, i32)) -> Cut {
        let (z, x0, across, y0, y1) = key;
        let world = &self.world;
        let dim = &world.dims[self.dim];
        let reg = &world.registry;
        let mat = Materializer::new(reg, world.meta.seed, dim.min_y);
        let h = (y1 - y0).max(1) as usize;
        let mut pixels = vec![Color32::TRANSPARENT; across * h];
        let mut runs = Vec::with_capacity(across);
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        let mut stacks: Vec<(TilePos, Option<TileStack>)> = Vec::new();
        let fluid = if world.meta.lava { Color32::from_rgba_unmultiplied(214, 96, 18, 235) } else { Color32::from_rgba_unmultiplied(54, 104, 214, 180) };
        let auto = AutoBiomes::new(reg, world.meta.sea_level, dim.min_y);
        let mut biomes: Vec<BiomeColumn> = Vec::with_capacity(across);
        let mut biome = BiomeColumn::default();
        for i in 0..across {
            let x = x0 + i as i32;
            let pos = TilePos::containing(x, z);
            let k = match stacks.iter().position(|(p, _)| *p == pos) {
                Some(k) => k,
                None => {
                    stacks.push((pos, TileStack::new(dim, pos, true)));
                    stacks.len() - 1
                }
            };
            let Some(stack) = &stacks[k].1 else {
                runs.push(Vec::new());
                biomes.push(BiomeColumn::default());
                continue;
            };
            // The biomes, as the export takes them in the middle of each 4 × 4.
            if i == 0 || x & 3 == 0 {
                let middle = massif_core::coords::local_index((x & !3) + 2, (z & !3) + 2);
                let top = || {
                    stack.spans(middle, &mut spans);
                    spans.last().map(|s| (s.y1, s.terrain))
                };
                stack.biome_column(middle, reg, &auto, top, &mut biome);
            }
            biomes.push(biome.clone());
            let cell = massif_core::coords::local_index(x, z);
            stack.spans(cell, &mut spans);
            stack.fluids(cell, &spans, &mut fluids);
            for s in &spans {
                let submerged = fluids.iter().any(|f| f.0 == s.y1);
                let recipe = mat.recipe_depth(s.terrain, x, z);
                let split = if recipe >= (s.y1 - s.y0) as u32 { s.y0 } else { s.y1 - recipe as i32 };
                for y in s.y0.max(y0)..s.y1.min(y1) {
                    let b = if y < split { mat.substrate(x, y, z) } else { mat.block(s, x, y, z, submerged) };
                    let c = reg.block(b).color;
                    let grain = 0.9 + 0.13 * (hash3(x, y, z, 77) & 15) as f32 / 15.0;
                    // The cut face a touch darker under the surface.
                    let depth = if s.y1 - y > 3 { 0.8 } else { 1.0 };
                    let f = grain * depth;
                    pixels[(y1 - 1 - y) as usize * across + i] = Color32::from_rgb((c[0] as f32 * f).min(255.0) as u8, (c[1] as f32 * f).min(255.0) as u8, (c[2] as f32 * f).min(255.0) as u8);
                }
            }
            for &(f0, f1) in &fluids {
                for y in f0.max(y0)..f1.min(y1) {
                    pixels[(y1 - 1 - y) as usize * across + i] = fluid;
                }
            }
            runs.push(spans.clone());
        }
        let image = ColorImage::new([across, h], pixels);
        Cut { key, runs, biomes, terrain: ctx.load_texture("ores cut", image, TextureOptions::NEAREST), ores: None }
    }

    // ---------------------------------------------------------- chart --

    fn chart_card(&mut self, ui: &mut Ui, pal: &Palette, rect: Rect) {
        let p = ui.painter().clone();
        p.rect(rect, theme::CARD_RADIUS, card_fill(pal), Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let strip = Rect::from_min_size(rect.min, vec2(rect.width(), STRIP));
        title(&p, pal, strip, Icon::ChartLine, "ore at each height");
        p.hline(strip.x_range(), strip.bottom(), Stroke::new(1.0, pal.border));
        // Blocks | Log | Share, the one shown underlined.
        let mut x = strip.right() - 12.0;
        for (scale, word) in [(Scale::Share, "Share"), (Scale::Log, "Log"), (Scale::Blocks, "Blocks")] {
            let g = p.layout_no_wrap(word.to_owned(), FontId::proportional(12.0), pal.text);
            let r = Rect::from_min_max(pos2(x - g.size().x - 8.0, strip.top()), pos2(x + 2.0, strip.bottom()));
            let resp = ui.interact(r, egui::Id::unique(("ores scale", word)), Sense::click());
            let on = self.scale == scale;
            p.galley(pos2(r.left() + 4.0, r.center().y - g.size().y / 2.0), g, if on { pal.text } else { pal.weak });
            if on {
                p.hline(r.x_range().shrink(3.0), strip.bottom() - 1.0, Stroke::new(1.5, pal.text));
            }
            if resp.hovered() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
            }
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, word));
            if resp.clicked() {
                self.scale = scale;
            }
            x = r.left() - 4.0;
        }

        let plot = Rect::from_min_max(pos2(rect.left() + 14.0, strip.bottom() + 8.0), pos2(rect.right() - 16.0, rect.bottom() - FOOT + 4.0));
        let (y0, y1) = self.heights();
        let key = (self.key(), self.rock.is_some());
        if self.series.as_ref().is_none_or(|s| s.key != key) {
            let mut made = series(&self.settings, y0, y1, key);
            made.rock = self.rock.as_ref().map(|r| self.settings.ores.iter().map(|rule| (y0..y1).map(|y| r.rock_in(&rule.biomes, y)).collect()).collect());
            self.series = Some(made);
        }
        let Some(series) = &self.series else { return };
        let columns = self.rock.as_ref().map_or(((self.area[2] - self.area[0]) * (self.area[3] - self.area[1])) as f64, |r| r.columns as f64);
        // Ore `k`'s blocks at height `y`: in the rock of the biomes it keeps to.
        let value = |k: usize, per_chunk: f64, y: i32| -> f64 {
            let rock = series.rock.as_ref().and_then(|r| r.get(k)?.get((y - y0) as usize).copied()).unwrap_or(columns);
            let blocks = per_chunk * rock / 256.0;
            match self.scale {
                Scale::Blocks => blocks,
                Scale::Log => (1.0 + blocks).log10(),
                Scale::Share => {
                    let solid = self.rock.as_ref().map_or(columns, |r| r.solid_at(y));
                    if solid > 0.0 { blocks / solid * 100.0 } else { 0.0 }
                }
            }
        };
        let values: Vec<Vec<f64>> = series.per_chunk.iter().enumerate().map(|(k, v)| v.iter().enumerate().map(|(i, &e)| value(k, e, y0 + i as i32)).collect()).collect();
        let top = values.iter().flatten().cloned().fold(0.0, f64::max);
        let (ticks, max) = nice(top, self.scale);
        let to_x = |v: f64| plot.left() + (v / max).clamp(0.0, 1.0) as f32 * plot.width();
        let scale_y = plot.height() / (y1 - y0) as f32;
        let to_y = |y: f32| plot.bottom() - (y - y0 as f32) * scale_y;
        let painter = p.with_clip_rect(plot.expand(2.0));
        // Grid: amounts dashed, heights faint (as the cut's).
        for &t in &ticks {
            let x = to_x(t);
            dashed_v(&painter, x, plot.y_range(), Stroke::new(1.0, pal.border));
            p.text(pos2(x, rect.bottom() - 11.0), Align2::CENTER_CENTER, tick_label(t, self.scale), FontId::monospace(10.0), pal.weak);
        }
        let step = if scale_y < 0.6 { 64 } else { 16 };
        let mut y = (y0 + step - 1).div_euclid(step) * step;
        while y < y1 {
            if y % (step * 2) == 0 {
                painter.hline(plot.x_range(), to_y(y as f32), Stroke::new(1.0, pal.border.gamma_multiply(0.6)));
            }
            y += step;
        }
        let sea = self.world.meta.sea_level;
        if (y0..y1).contains(&sea) {
            dashed_h(&painter, plot.x_range(), to_y(sea as f32), Stroke::new(1.0, Color32::from_rgb(96, 160, 250).gamma_multiply(0.6)));
        }
        // Each ore: an area fading toward the axis, a line with a glow; the
        // picked one last, brightest.
        let mut order: Vec<usize> = (0..values.len()).filter(|&k| self.settings.ores[k].on).collect();
        order.sort_by_key(|&k| k == self.selected);
        for k in order {
            let color = theme::rgb(self.settings.ores[k].mineral.color());
            let picked = k == self.selected;
            let points: Vec<Pos2> = values[k].iter().enumerate().map(|(i, &v)| pos2(to_x(v), to_y(y0 as f32 + i as f32 + 0.5))).collect();
            if points.len() < 2 {
                continue;
            }
            let mut mesh = egui::Mesh::default();
            let (edge, axis) = (color.gamma_multiply(if picked { 0.30 } else { 0.14 }), color.gamma_multiply(0.0));
            for (i, &pt) in points.iter().enumerate() {
                mesh.colored_vertex(pos2(plot.left(), pt.y), axis);
                mesh.colored_vertex(pt, edge);
                if i > 0 {
                    let a = (i as u32 - 1) * 2;
                    mesh.add_triangle(a, a + 1, a + 2);
                    mesh.add_triangle(a + 1, a + 3, a + 2);
                }
            }
            painter.add(egui::Shape::mesh(mesh));
            let width = if picked { 1.8 } else { 1.2 };
            if picked {
                painter.add(egui::Shape::line(points.clone(), Stroke::new(7.0, color.gamma_multiply(0.10))));
                painter.add(egui::Shape::line(points.clone(), Stroke::new(3.5, color.gamma_multiply(0.22))));
            }
            painter.add(egui::Shape::line(points, Stroke::new(width, color.gamma_multiply(if picked { 1.0 } else { 0.75 }))));
        }
        // Pointed at: a height, each ore's amount there.
        let r = ui.interact(plot, egui::Id::unique("ores chart"), Sense::hover());
        if let Some(at) = r.hover_pos() {
            let y = (y0 as f32 + (plot.bottom() - at.y) / scale_y).floor() as i32;
            if (y0..y1).contains(&y) {
                let sy = to_y(y as f32 + 0.5);
                dashed_h(&painter, plot.x_range(), sy, Stroke::new(1.0, pal.weak));
                let mut rows: Vec<(usize, f64)> = (0..values.len()).filter(|&k| self.settings.ores[k].on).map(|k| (k, values[k][(y - y0) as usize])).collect();
                rows.sort_by(|a, b| b.1.total_cmp(&a.1));
                for &(k, v) in &rows {
                    if v > 0.0 {
                        painter.circle(pos2(to_x(v), sy), 3.5, theme::rgb(self.settings.ores[k].mineral.color()), Stroke::new(1.5, card_fill(pal)));
                    }
                }
                tooltip(&p, pal, (plot, at), y, &rows, (&self.settings, self.selected), self.scale);
            }
        }
    }

    // ------------------------------------------------------ inspector --

    fn inspector(&mut self, ui: &mut Ui, pal: &Palette) {
        let world = self.world.clone();
        let reg = &world.registry;
        let selected = self.selected;
        // The band whose candle the pointer is over in the cut.
        let lit = self.cut_hover.and_then(|(ore, band)| (ore == selected).then_some(band));
        let (mut remove_ore, mut remove_band, mut row_hover) = (false, None, None);
        let card = egui::Frame::new()
            .fill(card_fill(pal))
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(theme::CARD_RADIUS)
            .inner_margin(egui::Margin { left: 18, right: 16, top: 12, bottom: 12 })
            .show(ui, |ui| {
                ui.set_width(WIDTH - 34.0);
                let Some(rule) = self.settings.ores.get_mut(selected) else {
                    ui.label(RichText::new("No ore: the export lays none. Add one with the + at the end of the ores.").size(12.5).color(pal.weak));
                    return None;
                };
                let mineral = rule.mineral;
                ui.horizontal(|ui| {
                    let (sq, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
                    ui.painter().rect_filled(sq, 2, theme::rgb(mineral.color()));
                    ui.label(RichText::new(mineral.name()).size(14.0).medium());
                    ui.label(RichText::new(format!("for {}", mineral.use_for())).size(12.0).color(pal.weak));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.spacing_mut().item_spacing.x = 8.0;
                        let add = egui::Button::new(RichText::new(theme::with_icon(Icon::Plus, "Add a band")).size(12.5))
                            .fill(pal.control())
                            .stroke(Stroke::new(1.0, pal.control_border()))
                            .corner_radius(theme::PILL)
                            .min_size(vec2(0.0, 26.0));
                        if ui.add_enabled(rule.bands.len() < 4, add).clicked() {
                            let next = rule.bands.last().copied().unwrap_or(Band::new(0, [32, 32], 64, 4.0, 8));
                            rule.bands.push(next);
                        }
                        widgets::switch(ui, &mut rule.on).on_hover_text("Whether the export lays this ore");
                        ui.label(RichText::new("Laid").size(12.5));
                        ui.add_space(6.0);
                        let (key, undo) = if cfg!(target_os = "macos") { ("⌫", "⌘Z") } else { ("Delete", "Ctrl+Z") };
                        let remove = egui::Button::new(RichText::new(theme::with_icon(Icon::Trash2, "Remove")).size(12.5).color(pal.weak)).frame(false);
                        if ui.add(remove).on_hover_text(format!("Take {} out of the ores ({key}); {undo} or the + brings it back", mineral.name())).clicked() {
                            remove_ore = true;
                        }
                    });
                });
                ui.add_space(6.0);
                // The biomes it keeps to: only in some, never in others.
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.add_sized(vec2(TAG, 30.0), egui::Label::new(RichText::new("Biomes").size(12.0).color(pal.weak)));
                    let half = ((ui.available_width() - 10.0) / 2.0).floor();
                    biome_list(ui, pal, half, ("Only in", "none yet: laid nowhere"), &mut rule.biomes.only, reg, "ore only in");
                    biome_list(ui, pal, half, ("Except in", "none yet"), &mut rule.biomes.except, reg, "ore except in");
                });
                ui.add_space(4.0);
                for (j, band) in rule.bands.iter_mut().enumerate() {
                    let picked = j == self.band;
                    // Its row lit: picked, or its candle pointed at, or under the pointer.
                    let behind = ui.painter().add(egui::Shape::Noop);
                    let row = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 10.0;
                        let words = RichText::new(format!("Band {}", j + 1)).size(12.0).color(if picked { pal.text } else { pal.weak });
                        if tag(ui, if picked { words.medium() } else { words }).clicked() {
                            self.band = j;
                        }
                        let (mut lo, mut p0, mut p1, mut hi) = (band.min, band.peak[0], band.peak[1], band.max);
                        let (changed, _) = widgets::input_group(
                            ui,
                            &format!("ore band {j}"),
                            &mut [
                                Part::Text("From y"),
                                Part::Int(&mut lo, -2048..=2048, 0.5),
                                Part::Text("most from"),
                                Part::Int(&mut p0, -2048..=2048, 0.5),
                                Part::Text("to"),
                                Part::Int(&mut p1, -2048..=2048, 0.5),
                                Part::Text("up to"),
                                Part::Int(&mut hi, -2048..=2048, 0.5),
                            ],
                        );
                        if changed {
                            (band.min, band.peak, band.max) = (lo, [p0, p1], hi);
                            band.tidy();
                            self.band = j;
                        }
                        ui.add(widgets::Field::new(&mut band.veins, 0.0..=60.0).max_decimals(2).text("Veins a chunk").width(170.0));
                        ui.add(widgets::Field::new(&mut band.size, 1..=massif_core::ores::MAX_SIZE).text("Blocks a vein").width(150.0));
                        let x = ui.add(egui::Button::new(RichText::new(theme::glyph(Icon::X).to_string()).size(13.0).color(pal.weak)).frame(false)).on_hover_text("Remove this band");
                        if x.clicked() {
                            remove_band = Some(j);
                        }
                    });
                    let rect = Rect::from_min_max(pos2(ui.min_rect().left() - 8.0, row.response.rect.top() - 2.0), pos2(ui.max_rect().right() + 6.0, row.response.rect.bottom() + 2.0));
                    let pointed = ui.rect_contains_pointer(rect);
                    if pointed {
                        row_hover = Some(j);
                    }
                    // Picked: ringed in the accent, as its candle is in the cut.
                    let shape = if picked {
                        Some(egui::Shape::Rect(egui::epaint::RectShape::new(rect, theme::TILE_RADIUS, selected_fill(pal), Stroke::new(1.0, pal.accent.gamma_multiply(0.7)), StrokeKind::Inside)))
                    } else if pointed || lit == Some(j) {
                        Some(egui::Shape::rect_filled(rect, theme::TILE_RADIUS, hover_fill(pal)))
                    } else {
                        None
                    };
                    if let Some(shape) = shape {
                        ui.painter().set(behind, shape);
                    }
                }
                if rule.bands.is_empty() {
                    ui.label(RichText::new("No band: this ore is laid nowhere.").size(12.0).color(pal.weak));
                }
                Some(theme::rgb(mineral.color()))
            });
        // The ore the details are of: its colour down their left side, as
        // under it in the strip.
        if let Some(color) = card.inner {
            let r = card.response.rect;
            ui.painter().rect_filled(Rect::from_min_max(pos2(r.left() + 7.0, r.top() + 14.0), pos2(r.left() + 10.0, r.bottom() - 14.0)), 2, color);
        }
        if remove_ore {
            self.remove_ore(selected);
        }
        if let Some(j) = remove_band {
            self.remove_band(j);
        }
        if row_hover != self.row_hover {
            // The cut, drawn before, lights the candle on the next frame.
            self.row_hover = row_hover;
            ui.ctx().request_repaint();
        }
    }
}

/// A band's candle, where it stands.
struct Candle {
    ore: usize,
    band: usize,
    x: f32,
    w: f32,
    top: f32,
    bottom: f32,
    body_top: f32,
    body_bottom: f32,
    past_top: bool,
    past_bottom: bool,
    color: Color32,
    on: bool,
}

impl Candle {
    fn body(&self) -> Rect {
        let (mut t, mut b) = (self.body_top, self.body_bottom);
        if b - t < 4.0 {
            let c = (t + b) / 2.0;
            (t, b) = (c - 2.0, c + 2.0);
        }
        Rect::from_min_max(pos2(self.x - self.w / 2.0, t), pos2(self.x + self.w / 2.0, b))
    }
}

/// The ores of the cut, as the export lays them: a picture over the cut,
/// and how many blocks of each ore it shows.
fn ores_image(world: &World, dim: usize, settings: &OreSettings, cut: &Cut) -> (ColorImage, Vec<usize>) {
    let (z, x0, across, y0, y1) = cut.key;
    let h = (y1 - y0).max(1) as usize;
    let mut pixels = vec![Color32::TRANSPARENT; across * h];
    let mut counts = vec![0; settings.ores.len()];
    let d = &world.dims[dim];
    let reg = &world.registry;
    let mat = Materializer::new(reg, world.meta.seed, d.min_y);
    let hosts = massif_core::ores::hosts(reg);
    if let Some(placer) = Placer::new(settings, world.meta.seed) {
        let colors: Vec<Color32> = settings.ores.iter().map(|o| theme::rgb(o.mineral.color())).collect();
        let first = x0.div_euclid(16);
        let count = (x0 + across as i32 - 1).div_euclid(16) - first + 1;
        for chunk in placer.chunks(first, z.div_euclid(16), count, 1, (y0, y1)) {
            placer.lay(
                &chunk,
                |bx, by, bz| {
                    if bz != z || bx < x0 || bx >= x0 + across as i32 || by < y0 || by >= y1 {
                        return None;
                    }
                    rock_at(&cut.runs[(bx - x0) as usize], &mat, &hosts, bx, by, bz, d.max_y)
                },
                |bx, by, _| cut.biomes[(bx - x0) as usize].of_block(by),
                |b, (_, host): (massif_core::BlockId, Host)| {
                    let c = colors[b.ore as usize];
                    let c = if host == Host::Deepslate { theme::mix(c, Color32::BLACK, 0.18) } else { c };
                    pixels[(y1 - 1 - b.y) as usize * across + (b.x - x0) as usize] = c;
                    counts[b.ore as usize] += 1;
                },
            );
        }
    }
    (ColorImage::new([across, h], pixels), counts)
}

/// The ore blocks expected a chunk of rock at each height from `y0`, by
/// ore, smoothed over the spread of a vein.
fn series(settings: &OreSettings, y0: i32, y1: i32, key: (u64, bool)) -> Series {
    let n = (y1 - y0).max(1) as usize;
    let per_chunk = settings
        .ores
        .iter()
        .map(|rule| {
            let mut raw = vec![0.0f64; n];
            if settings.on && rule.on {
                for b in &rule.bands {
                    let mut b = *b;
                    b.tidy();
                    let total: f64 = (b.min..=b.max).map(|y| b.weight(y)).sum();
                    if total <= 0.0 {
                        continue;
                    }
                    let k = b.veins as f64 * b.size as f64 * settings.richness.max(0.0) as f64 / total;
                    for (i, v) in raw.iter_mut().enumerate() {
                        *v += k * b.weight(y0 + i as i32);
                    }
                }
            }
            // A vein's blocks lie around where it starts.
            (0..n).map(|i| {
                let (a, b) = (i.saturating_sub(2), (i + 3).min(n));
                raw[a..b].iter().sum::<f64>() / (b - a) as f64
            }).collect()
        })
        .collect();
    Series { key, per_chunk, rock: None }
}

/// Round ticks for amounts up to `top`, and the top of the axis.
fn nice(top: f64, scale: Scale) -> (Vec<f64>, f64) {
    if top <= 0.0 {
        return (vec![0.0], 1.0);
    }
    if scale == Scale::Log {
        let max = top.ceil().max(1.0);
        return ((0..=max as i32).map(|v| v as f64).collect(), max);
    }
    let raw = top / 4.0;
    let mag = 10f64.powf(raw.log10().floor());
    let step = [1.0, 2.0, 2.5, 5.0, 10.0].iter().map(|m| m * mag).find(|&s| s >= raw).unwrap_or(10.0 * mag);
    let count = (top / step).ceil() as i32;
    ((0..=count).map(|k| k as f64 * step).collect(), count as f64 * step)
}

fn tick_label(v: f64, scale: Scale) -> String {
    match scale {
        Scale::Blocks => {
            let (n, unit) = short(v);
            format!("{n}{unit}")
        }
        Scale::Log => format!("10^{}", v as i32),
        Scale::Share => format!("{}%", trim(v)),
    }
}

/// A number short: `850`, `12k`, `2.1M`, `1.3B`.
fn short(v: f64) -> (String, &'static str) {
    let v = v.max(0.0);
    if v >= 1e9 {
        (trim(v / 1e9), "B")
    } else if v >= 1e6 {
        (trim(v / 1e6), "M")
    } else if v >= 1e4 {
        ((v / 1e3).round().to_string(), "k")
    } else {
        (thousands(v.round() as u64), "")
    }
}

/// `2.0` as `2`, `2.25` as `2.25`, `0.6` as `0.6`.
fn trim(v: f64) -> String {
    let s = if v >= 10.0 { format!("{v:.0}") } else { format!("{v:.2}") };
    if s.contains('.') { s.trim_end_matches('0').trim_end_matches('.').to_owned() } else { s }
}

/// The card behind the parts of the window.
fn card_fill(pal: &Palette) -> Color32 {
    if pal.dark { Color32::from_rgb(13, 13, 14) } else { Color32::WHITE }
}

/// Behind what is picked on a card: an ore of the strip, a band of the
/// details, a row of the tooltip.
fn selected_fill(pal: &Palette) -> Color32 {
    if pal.dark { theme::mix(card_fill(pal), pal.text, 0.06) } else { pal.header }
}

/// Behind what the pointer is over on a card.
fn hover_fill(pal: &Palette) -> Color32 {
    if pal.dark { theme::mix(card_fill(pal), pal.text, 0.03) } else { pal.well() }
}

/// The biomes an ore keeps to, in words: `Only in Desert, Badlands`.
fn biome_words(choice: &BiomeChoice, reg: &Registry) -> Option<String> {
    let names = |list: &[u16]| {
        let mut names: Vec<String> = list.iter().take(3).map(|&b| reg.biome(BiomeId(b)).name.clone()).collect();
        if list.len() > 3 {
            names.push(format!("{} more", list.len() - 3));
        }
        names.join(", ")
    };
    let only = choice.only.as_ref().map(|l| if l.is_empty() { "In no biome".to_owned() } else { format!("Only in {}", names(l)) });
    let except = choice.except.as_ref().filter(|l| !l.is_empty()).map(|l| names(l));
    match (only, except) {
        (Some(only), Some(except)) => Some(format!("{only}; not in {except}")),
        (Some(only), None) => Some(only),
        (None, Some(except)) => Some(format!("Not in {except}")),
        (None, None) => None,
    }
}

/// A tag at the left of a row of the details: a word, clickable.
fn tag(ui: &mut Ui, words: RichText) -> egui::Response {
    ui.add(egui::Button::new(words).frame(false).min_size(vec2(TAG, 30.0)))
}

/// One list of biomes of an ore's details, `width` wide: a checkbox to keep
/// to it, a menu that ticks biomes on and off (open until a click
/// elsewhere), and the biomes as chips (a click takes one off).
fn biome_list(ui: &mut Ui, pal: &Palette, width: f32, (label, empty): (&str, &str), list: &mut Option<Vec<u16>>, reg: &Registry, id: &str) {
    use egui::containers::menu::{MenuButton, MenuConfig};
    ui.allocate_ui_with_layout(vec2(width, 30.0), Layout::left_to_right(Align::Center).with_main_wrap(true), |ui| {
        ui.set_width(width);
        ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
        let mut on = list.is_some();
        if widgets::checkbox(ui, &mut on, RichText::new(label).size(12.5)).changed() {
            *list = on.then(Vec::new);
        }
        let Some(items) = list else { return };
        // The menu before the chips and with an id of its own, so that it
        // stays where it is, and open, as biomes are ticked.
        let config = MenuConfig::new().close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside);
        ui.scope_builder(egui::UiBuilder::new().scope_id(egui::Id::unique((id, "add menu"))), |ui| MenuButton::new(RichText::new(theme::with_icon(Icon::Plus, "Add")).size(12.0)).config(config).ui(ui, |ui| {
            ui.set_width(230.0);
            // A search over the names: the game has some sixty biomes.
            let key = egui::Id::unique((id, "search"));
            let mut search = ui.data_mut(|d| d.get_temp::<String>(key)).unwrap_or_default();
            ui.add(egui::TextEdit::singleline(&mut search).hint_text("Search").desired_width(f32::INFINITY));
            ui.data_mut(|d| d.insert_temp(key, search.clone()));
            let search = search.to_lowercase();
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt((id, "biomes")).max_height(320.0).show(ui, |ui| {
                for (i, b) in reg.biomes.iter().enumerate().skip(1) {
                    if !search.is_empty() && !b.name.to_lowercase().contains(&search) {
                        continue;
                    }
                    let i = i as u16;
                    let mut ticked = items.contains(&i);
                    if widgets::checkbox(ui, &mut ticked, &b.name).changed() {
                        if ticked {
                            items.push(i);
                        } else {
                            items.retain(|&x| x != i);
                        }
                    }
                }
            });
        }));
        let mut remove = None;
        for (i, &b) in items.iter().enumerate() {
            let chip = egui::Button::new(RichText::new(format!("{}  {}", reg.biome(BiomeId(b)).name, theme::glyph(Icon::X))).size(11.5)).corner_radius(theme::PILL).min_size(vec2(0.0, 24.0));
            if ui.add(chip).clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            items.remove(i);
        }
        if items.is_empty() {
            ui.label(RichText::new(empty).size(11.5).color(pal.weak));
        }
    });
}

/// A card's title in its strip: an icon, words set as code.
fn title(p: &egui::Painter, pal: &Palette, strip: Rect, icon: Icon, words: &str) {
    p.text(pos2(strip.left() + 14.0, strip.center().y), Align2::LEFT_CENTER, theme::glyph(icon).to_string(), FontId::proportional(13.0), pal.weak);
    p.text(pos2(strip.left() + 34.0, strip.center().y), Align2::LEFT_CENTER, words, FontId::monospace(11.5), pal.weak);
}

fn gradient(p: &egui::Painter, r: Rect, top: Color32, bottom: Color32) {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(r.left_top(), top);
    mesh.colored_vertex(r.right_top(), top);
    mesh.colored_vertex(r.left_bottom(), bottom);
    mesh.colored_vertex(r.right_bottom(), bottom);
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(1, 3, 2);
    p.add(egui::Shape::mesh(mesh));
}

fn dashed_h(p: &egui::Painter, x: egui::Rangef, y: f32, stroke: Stroke) {
    p.extend(egui::Shape::dashed_line(&[pos2(x.min, y), pos2(x.max, y)], stroke, 4.0, 4.0));
}

fn dashed_v(p: &egui::Painter, x: f32, y: egui::Rangef, stroke: Stroke) {
    p.extend(egui::Shape::dashed_line(&[pos2(x, y.min), pos2(x, y.max)], stroke, 3.0, 4.0));
}

/// Words in a small dark bubble.
fn bubble(p: &egui::Painter, pal: &Palette, at: Pos2, words: &str) {
    let g = p.layout_no_wrap(words.to_owned(), FontId::proportional(11.5), pal.text);
    let r = Rect::from_min_size(at, g.size() + vec2(16.0, 10.0));
    let clip = p.clip_rect();
    let r = r.translate(vec2((clip.right() - r.right()).min(0.0), (clip.top() - r.top()).max(0.0)));
    p.rect(r, 7, if pal.dark { Color32::from_rgb(24, 24, 26) } else { Color32::WHITE }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    p.galley(r.min + vec2(8.0, 5.0), g, pal.text);
}

/// The amounts at height `y`, ore by ore, beside the pointer; the ore
/// picked lit.
fn tooltip(p: &egui::Painter, pal: &Palette, (plot, at): (Rect, Pos2), y: i32, rows: &[(usize, f64)], (settings, picked): (&OreSettings, usize), scale: Scale) {
    let shown: Vec<&(usize, f64)> = rows.iter().filter(|r| r.1 > 0.0).take(8).collect();
    let height = 30.0 + shown.len().max(1) as f32 * 19.0;
    let width = 168.0;
    let mut r = Rect::from_min_size(at + vec2(16.0, -height / 2.0), vec2(width, height));
    if r.right() > plot.right() {
        r = r.translate(vec2(-width - 32.0, 0.0));
    }
    let r = r.translate(vec2(0.0, (plot.top() - r.top()).max(0.0) + (plot.bottom() - r.bottom()).min(0.0)));
    p.rect(r.translate(vec2(0.0, 3.0)), 9, Color32::from_black_alpha(60), Stroke::NONE, StrokeKind::Inside);
    p.rect(r, 9, if pal.dark { Color32::from_rgb(22, 22, 24) } else { Color32::WHITE }, Stroke::new(1.0, pal.border), StrokeKind::Inside);
    p.text(r.min + vec2(12.0, 10.0), Align2::LEFT_TOP, format!("y {y}"), FontId::monospace(11.0), pal.weak);
    if shown.is_empty() {
        p.text(r.min + vec2(12.0, 30.0), Align2::LEFT_TOP, "no ore here", FontId::proportional(11.5), pal.weak);
    }
    for (i, &&(k, v)) in shown.iter().enumerate() {
        let row_y = r.top() + 37.0 + i as f32 * 19.0;
        if k == picked {
            p.rect_filled(Rect::from_center_size(pos2(r.center().x, row_y), vec2(r.width() - 10.0, 18.0)), 5, selected_fill(pal));
        }
        let c = theme::rgb(settings.ores[k].mineral.color());
        p.rect_filled(Rect::from_center_size(pos2(r.left() + 16.0, row_y), vec2(7.0, 7.0)), 2, c);
        p.text(pos2(r.left() + 26.0, row_y), Align2::LEFT_CENTER, settings.ores[k].mineral.name(), FontId::proportional(11.5), pal.text);
        let value = match scale {
            Scale::Blocks => {
                let (n, u) = short(v);
                format!("{n}{u}")
            }
            Scale::Log => {
                let (n, u) = short(10f64.powf(v) - 1.0);
                format!("{n}{u}")
            }
            Scale::Share => format!("{} %", trim(v)),
        };
        p.text(pos2(r.right() - 12.0, row_y), Align2::RIGHT_CENTER, value, FontId::monospace(11.0), pal.text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_read_short() {
        assert_eq!(short(850.0), ("850".to_owned(), ""));
        assert_eq!(short(12_400.0), ("12".to_owned(), "k"));
        assert_eq!(short(2_140_000.0), ("2.14".to_owned(), "M"));
        assert_eq!(trim(2.0), "2");
        assert_eq!(trim(0.6), "0.6");
        let (ticks, max) = nice(37.0, Scale::Blocks);
        assert_eq!(max, 40.0);
        assert_eq!(ticks, vec![0.0, 10.0, 20.0, 30.0, 40.0]);
    }

    /// The expected ore follows the bands: none outside them, the most over
    /// their peak.
    #[test]
    fn the_series_follow_the_bands() {
        let settings = OreSettings::default();
        let s = series(&settings, -64, 320, (0, false));
        let diamond = settings.ores.iter().position(|o| o.mineral == Mineral::Diamond).unwrap();
        let d = &s.per_chunk[diamond];
        assert!(d[(-58 + 64) as usize] > d[64] && d[(40 + 64) as usize] == 0.0, "diamonds deep down");
        let coal = settings.ores.iter().position(|o| o.mineral == Mineral::Coal).unwrap();
        assert!(s.per_chunk[coal][(96 + 64) as usize] > s.per_chunk[coal][(10 + 64) as usize]);
    }
}
