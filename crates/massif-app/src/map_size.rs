//! Edit ▸ Map Size…: the map made larger or smaller by whole tiles of 128
//! blocks, on any side, or tile by tile with clicks on its picture (as
//! WorldPainter's Add/Remove Tiles); the tiles added filled with the land
//! continued past the edges, sea, flat land or generated land, eased out of
//! the edge (see `massif_core::expand`). One step to undo.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, row, well};
use egui::{Color32, Rect, RichText, Sense, Stroke, TextureHandle, Ui, pos2, vec2};
use massif_core::TilePos;
use massif_core::expand::{Fill, FillSettings, Sides};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// How long the settings rest before the preview is worked out again.
const SETTLE: Duration = Duration::from_millis(300);

/// A picture of the map and the blocks it shows.
type Picture = (egui::ColorImage, [i32; 4]);

/// What fills the tiles added, as the dialog offers it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillKind {
    Continue,
    Sea,
    Flat,
    Generated,
    /// The world Minecraft makes around the map for a seed.
    Minecraft,
}

/// The most tiles a side of the map may have: 32,768 blocks, as Minecraft
/// maps Massif makes elsewhere.
const MAX_SIDE: i32 = 256;

/// The dialog's settings.
pub struct SizeForm {
    /// The map's tiles now.
    tiles: HashSet<TilePos>,
    pub sides: Sides,
    /// Tiles clicked: in the new map if they were not, out if they were.
    toggled: HashSet<TilePos>,
    pub fill: FillKind,
    pub sea_depth: i32,
    pub flat_height: i32,
    pub ease: i32,
    /// The seed of the Minecraft world, as typed.
    pub seed_text: String,
    /// The land of that seed, made once for the seed typed.
    around: Option<Arc<massif_core::vanilla::VanillaAround>>,
    /// Blocks of the band all around.
    band: i32,
    /// What a drag over the picture does: add tiles (`true`) or take them
    /// away, decided by the first tile it touches.
    painting: Option<bool>,
    /// The map as it is (one dimension), what the preview is made from.
    source: Arc<massif_core::World>,
    /// The water level, fluid, seed and build limits of the map.
    base: FillSettings,
    /// The picture shown: the map as it is, then as it will be.
    picture: Option<(TextureHandle, [i32; 4])>,
    /// The picture being made, for the settings of that key, and how to
    /// stop it.
    coming: Option<(u64, Receiver<Option<Picture>>, Arc<AtomicBool>)>,
    /// The settings the picture shows, and when they last changed.
    shown: u64,
    changed: (u64, Instant),
}

pub enum Choice {
    Keep,
    Close,
    Apply,
}

impl SizeForm {
    /// The dialog for the dimension `dim` of `world`, its picture drawn in
    /// the background.
    pub fn new(world: &massif_core::World, dim: usize) -> Self {
        let d = &world.dims[dim];
        let tiles: HashSet<TilePos> = d.ground().positions().collect();
        let meta = &world.meta;
        let base = FillSettings { fill: Fill::Continue, ease: 96.0, sea_level: meta.sea_level, lava: meta.lava, seed: meta.seed, min_y: d.min_y, max_y: d.max_y };
        let source = Arc::new(massif_core::World { meta: meta.clone(), registry: world.registry.clone(), dims: vec![d.copy_unfrozen()], points: Vec::new() });
        let mut form = Self {
            tiles,
            sides: Sides::default(),
            toggled: HashSet::new(),
            fill: FillKind::Continue,
            sea_depth: 12,
            flat_height: meta.sea_level + 2,
            ease: 96,
            seed_text: meta.seed.to_string(),
            around: None,
            band: 96,
            painting: None,
            source,
            base,
            picture: None,
            coming: None,
            shown: 0,
            changed: (0, Instant::now()),
        };
        let key = form.key();
        form.changed = (key, Instant::now() - SETTLE);
        form
    }

    /// How the tiles added are filled.
    pub fn settings(&self) -> FillSettings {
        FillSettings { fill: self.fill(), ease: self.ease as f32, seed: self.seed(), ..self.base }
    }

    /// The seed typed (Minecraft's own reading of it).
    pub fn seed(&self) -> i64 {
        if self.fill == FillKind::Minecraft { crate::home::parse_seed(&self.seed_text) } else { self.base.seed }
    }

    /// The world the tiles added ease into, for the Minecraft world.
    pub fn around(&mut self) -> Option<Arc<massif_core::vanilla::VanillaAround>> {
        if self.fill != FillKind::Minecraft {
            return None;
        }
        let seed = self.seed();
        if self.around.as_ref().is_none_or(|a| a.vanilla().seed != seed) {
            self.around = Some(Arc::new(massif_core::vanilla::VanillaAround::new(seed, &self.source.registry, None)));
        }
        self.around.clone()
    }

    /// A key of the settings: the same settings, the same key.
    fn key(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        let mut tiles: Vec<TilePos> = self.target().into_iter().collect();
        tiles.sort_unstable_by_key(|p| (p.z, p.x));
        for p in tiles {
            (p.x, p.z).hash(&mut h);
        }
        format!("{:?}", self.settings()).hash(&mut h);
        h.finish()
    }

    /// The picture of the map as the settings make it, worked out on a
    /// thread of its own, the one before stopped.
    fn preview(&mut self, ctx: &egui::Context) {
        let key = self.key();
        if key != self.changed.0 {
            self.changed = (key, Instant::now());
        }
        if let Some((k, rx, _)) = &self.coming {
            match rx.try_recv() {
                Ok(picture) => {
                    self.shown = *k;
                    self.coming = None;
                    if let Some((image, blocks)) = picture {
                        self.picture = Some((ctx.load_texture("map size picture", image, egui::TextureOptions::LINEAR), blocks));
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {}
                Err(_) => self.coming = None,
            }
        }
        if key == self.shown || self.coming.as_ref().is_some_and(|c| c.0 == key) {
            if self.coming.is_some() {
                ctx.request_repaint_after(Duration::from_millis(60));
            }
            return;
        }
        let rest = self.changed.1.elapsed();
        if rest < SETTLE {
            ctx.request_repaint_after(SETTLE - rest);
            return;
        }
        if let Some((_, _, stop)) = self.coming.take() {
            stop.store(true, Ordering::Relaxed);
        }
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let (source, target, settings, cancel, repaint, around) = (self.source.clone(), self.target(), self.settings(), stop.clone(), ctx.clone(), self.around());
        let unchanged = target == self.tiles;
        let _ = std::thread::Builder::new().name("massif map size preview".into()).spawn(move || {
            let mut dim = source.dims[0].copy_unfrozen();
            if !unchanged {
                let around = around.as_deref().map(|a| a as &dyn massif_core::expand::Surroundings);
                massif_core::expand::apply(&mut dim, &target, &settings, &source.registry, around, &cancel);
            }
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            let t = massif_core::TILE_SIZE;
            let picture = dim.bounds().and_then(|(lo, hi)| {
                let world = massif_core::World { meta: source.meta.clone(), registry: source.registry.clone(), dims: vec![dim], points: Vec::new() };
                let img = massif_core::thumbnail::render(&world, 512)?;
                Some((egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()), [lo.x * t, lo.z * t, (hi.x + 1) * t, (hi.z + 1) * t]))
            });
            let _ = tx.send(picture);
            repaint.request_repaint();
        });
        self.coming = Some((key, rx, stop));
        ctx.request_repaint_after(Duration::from_millis(60));
    }

    /// Whether the picture shows the settings as they are.
    #[cfg(test)]
    pub fn preview_ready(&self) -> bool {
        self.picture.is_some() && self.shown == self.key()
    }

    /// The tiles of the new map.
    pub fn target(&self) -> HashSet<TilePos> {
        let mut out = massif_core::expand::sided(&self.tiles, self.sides);
        for p in &self.toggled {
            if !out.remove(p) {
                out.insert(*p);
            }
        }
        out
    }

    /// What fills the tiles added.
    pub fn fill(&self) -> Fill {
        match self.fill {
            FillKind::Continue => Fill::Continue,
            FillKind::Sea => Fill::Sea { depth: self.sea_depth as f32 },
            FillKind::Flat => Fill::Flat { height: self.flat_height as f32 },
            FillKind::Generated => Fill::Generated,
            FillKind::Minecraft => Fill::World,
        }
    }

    /// Click tile `p`: in the new map or out of it.
    pub fn toggle(&mut self, p: TilePos) {
        if !self.toggled.remove(&p) {
            self.toggled.insert(p);
        }
    }

    fn set(&mut self, p: TilePos, on: bool) {
        if self.target().contains(&p) != on {
            self.toggle(p);
        }
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        self.preview(ui.ctx());
        ui.set_width(520.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Scaling).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Map Size")).size(16.0).bold());
        });
        ui.add(
            egui::Label::new(
                RichText::new(tr!("Add tiles of 128 blocks to any side of the map, or take them away. Click or drag over the picture to add or remove tiles one by one."))
                    .size(12.0)
                    .color(pal.weak),
            )
            .wrap(),
        );
        ui.add_space(10.0);
        self.grid(ui, &pal);
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("Tiles on each side")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, tr!("North and south"), |ui| {
                ui.add(Field::new(&mut self.sides.south, -MAX_SIDE..=MAX_SIDE).text(tr!("south ")));
                ui.add(Field::new(&mut self.sides.north, -MAX_SIDE..=MAX_SIDE).text(tr!("north ")));
            });
            row(ui, tr!("West and east"), |ui| {
                ui.add(Field::new(&mut self.sides.east, -MAX_SIDE..=MAX_SIDE).text(tr!("east ")));
                ui.add(Field::new(&mut self.sides.west, -MAX_SIDE..=MAX_SIDE).text(tr!("west ")));
            });
        });
        ui.label(RichText::new(tr!("A negative number crops that side.")).size(11.5).color(pal.weak));
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(tr!("A band all around")).size(12.0));
            ui.add(Field::new(&mut self.band, 1..=4096).suffix(tr!(" blocks")));
            if ui.button(tr!("Set")).on_hover_text(tr!("Tiles enough on every side for a band that wide, the land eased over it")).clicked() {
                let tiles = (self.band + massif_core::TILE_SIZE - 1) / massif_core::TILE_SIZE;
                self.sides = Sides { west: tiles, east: tiles, north: tiles, south: tiles };
                self.ease = self.band;
                if self.fill == FillKind::Continue {
                    self.fill = FillKind::Generated;
                }
            }
        });
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("The land added")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let kinds = [(FillKind::Continue, tr!("The edges continued")), (FillKind::Sea, tr!("Sea")), (FillKind::Flat, tr!("Flat land")), (FillKind::Generated, tr!("Generated")), (FillKind::Minecraft, tr!("Minecraft world"))];
        let labels: Vec<&str> = kinds.iter().map(|k| k.1).collect();
        let on: Vec<bool> = kinds.iter().map(|k| k.0 == self.fill).collect();
        if let Some(i) = button_group(ui, "map size fill", &labels, &on) {
            self.fill = kinds[i].0;
        }
        ui.add_space(6.0);
        let about = match self.fill {
            FillKind::Continue => tr!("The heights, water, terrains and biomes of the edges, carried on outward and smoothed, with some relief of their own."),
            FillKind::Sea => tr!("The land slopes down from the edges into the sea."),
            FillKind::Flat => tr!("The land eases from the edges to a flat plain."),
            FillKind::Generated => tr!("Hills from the generator of new maps, the edges easing into them."),
            FillKind::Minecraft => tr!("The land Minecraft makes around the map for this seed: its heights, seas and biomes, the edges easing into them. The map takes the seed, so the game carries on from there."),
        };
        ui.add(egui::Label::new(RichText::new(about).size(11.5).color(pal.weak)).wrap());
        if self.fill != FillKind::Continue {
            ui.add_space(4.0);
            well(ui, |ui| {
                match self.fill {
                    FillKind::Sea => row(ui, tr!("Depth"), |ui| {
                        ui.add(Field::new(&mut self.sea_depth, 1..=200).suffix(tr!(" blocks")));
                    }),
                    FillKind::Flat => row(ui, tr!("Height"), |ui| {
                        ui.add(Field::new(&mut self.flat_height, -63..=319));
                    }),
                    FillKind::Minecraft => row(ui, tr!("Seed"), |ui| {
                        ui.add(egui::TextEdit::singleline(&mut self.seed_text).desired_width(180.0));
                    }),
                    _ => {}
                }
                row(ui, tr!("Eased over"), |ui| {
                    ui.add(Field::new(&mut self.ease, 1..=1024).suffix(tr!(" blocks")));
                });
            });
        }
        ui.add_space(8.0);
        let target = self.target();
        let added = target.iter().filter(|p| !self.tiles.contains(p)).count();
        let removed = self.tiles.iter().filter(|p| !target.contains(p)).count();
        let size = massif_core::expand::block_bounds(&target);
        let summary = match size {
            Some(r) => tr!("The map becomes {w} × {d} blocks: {added} tiles added, {removed} taken away. One step to undo.", w = r.width(), d = r.depth(), added, removed),
            None => tr!("No tile would be left.").to_owned(),
        };
        ui.add(egui::Label::new(RichText::new(summary).size(11.5).color(if size.is_some() { pal.weak } else { pal.danger })).wrap());
        let too_large = size.is_some_and(|r| r.width() > MAX_SIDE * massif_core::TILE_SIZE || r.depth() > MAX_SIDE * massif_core::TILE_SIZE);
        if too_large {
            ui.label(RichText::new(tr!("At most 32,768 blocks a side.")).size(11.5).color(pal.danger));
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ok = size.is_some() && !too_large && (added > 0 || removed > 0);
            if ui.add_enabled(ok, theme::primary_button(ui, tr!("Apply"))).clicked() {
                choice = Choice::Apply;
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
            if (!self.sides.is_zero() || !self.toggled.is_empty()) && ui.button(tr!("Reset")).clicked() {
                self.sides = Sides::default();
                self.toggled.clear();
            }
        });
        choice
    }

    /// The map's picture under its tiles: those kept, added and taken away,
    /// with a margin of tiles around to click.
    fn grid(&mut self, ui: &mut Ui, pal: &Palette) {
        let target = self.target();
        let all: HashSet<TilePos> = self.tiles.union(&target).copied().collect();
        let Some((lo, hi)) = massif_core::expand::bounds(&all) else {
            return;
        };
        let margin = 2;
        let (x0, z0, x1, z1) = (lo.x - margin, lo.z - margin, hi.x + margin, hi.z + margin);
        let (w, d) = ((x1 - x0 + 1) as f32, (z1 - z0 + 1) as f32);
        let room = vec2(ui.available_width(), 300.0);
        let cell = (room.x / w).min(room.y / d);
        let size = vec2(cell * w, cell * d);
        let (rect, response) = ui.allocate_exact_size(vec2(room.x, size.y), Sense::click_and_drag());
        let origin = pos2(rect.center().x - size.x / 2.0, rect.top());
        let painter = ui.painter_at(rect);
        let area = Rect::from_min_size(origin, size);
        painter.rect_filled(area, theme::TILE_RADIUS, pal.canvas);
        let tile_rect = |p: TilePos| Rect::from_min_size(origin + vec2((p.x - x0) as f32 * cell, (p.z - z0) as f32 * cell), vec2(cell, cell));
        if let Some((texture, blocks)) = &self.picture {
            let t = massif_core::TILE_SIZE as f32;
            let a = origin + vec2((blocks[0] as f32 / t - x0 as f32) * cell, (blocks[1] as f32 / t - z0 as f32) * cell);
            let b = origin + vec2((blocks[2] as f32 / t - x0 as f32) * cell, (blocks[3] as f32 / t - z0 as f32) * cell);
            painter.image(texture.id(), Rect::from_min_max(a, b), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        // Where the map has no tile, the backdrop shows through the picture.
        for z in z0..=z1 {
            for x in x0..=x1 {
                let p = TilePos::new(x, z);
                let r = tile_rect(p);
                let (was, is) = (self.tiles.contains(&p), target.contains(&p));
                match (was, is) {
                    (true, true) => {}
                    (false, false) => {
                        painter.rect_filled(r, 0.0, pal.canvas);
                        painter.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, pal.border.gamma_multiply(0.5)), egui::StrokeKind::Inside);
                    }
                    (false, true) => {
                        painter.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.12));
                        painter.rect_stroke(r.shrink(0.5), 0.0, Stroke::new(1.0, pal.accent), egui::StrokeKind::Inside);
                    }
                    (true, false) => {
                        painter.rect_filled(r, 0.0, Color32::from_black_alpha(170));
                        let s = Stroke::new(1.0, pal.weak);
                        let r = r.shrink(cell * 0.25);
                        if cell >= 8.0 {
                            painter.line_segment([r.left_top(), r.right_bottom()], s);
                            painter.line_segment([r.right_top(), r.left_bottom()], s);
                        }
                    }
                }
            }
        }
        // The new map's outline.
        if let Some((a, b)) = massif_core::expand::bounds(&target) {
            let r = Rect::from_min_max(tile_rect(a).min, tile_rect(b).max);
            painter.rect_stroke(r, 0.0, Stroke::new(1.5, pal.accent), egui::StrokeKind::Outside);
        }
        let at = |pos: egui::Pos2| {
            let v = (pos - origin) / cell;
            let p = TilePos::new(x0 + v.x.floor() as i32, z0 + v.y.floor() as i32);
            (v.x >= 0.0 && v.y >= 0.0 && p.x <= x1 && p.z <= z1).then_some(p)
        };
        if let Some(p) = response.hover_pos().and_then(at) {
            painter.rect_stroke(tile_rect(p), 0.0, Stroke::new(1.5, pal.text), egui::StrokeKind::Inside);
            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
        }
        if (response.drag_started() || response.clicked())
            && let Some(p) = response.interact_pointer_pos().and_then(at)
        {
            let on = !target.contains(&p);
            self.painting = Some(on);
            self.set(p, on);
        }
        if response.dragged()
            && let (Some(on), Some(p)) = (self.painting, response.interact_pointer_pos().and_then(at))
        {
            self.set(p, on);
        }
        if response.drag_stopped() || response.clicked() {
            self.painting = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form(n: i32) -> SizeForm {
        let mut world = massif_core::World::new(massif_core::WorldMeta::default());
        for z in 0..n {
            for x in 0..n {
                world.dims[0].ground_mut().insert(TilePos::new(x, z), massif_core::Tile::ground(70.0, 0, massif_core::NO_FLUID));
            }
        }
        SizeForm::new(&world, 0)
    }

    #[test]
    fn sides_and_clicks_make_the_new_map() {
        let mut f = form(3);
        assert_eq!(f.target().len(), 9);
        f.sides.east = 1;
        assert_eq!(f.target().len(), 12);
        // A click takes a tile out, a second one puts it back.
        f.toggle(TilePos::new(0, 0));
        assert!(!f.target().contains(&TilePos::new(0, 0)));
        f.toggle(TilePos::new(0, 0));
        assert!(f.target().contains(&TilePos::new(0, 0)));
        // A tile clicked outside the map is added; cropping keeps the clicks.
        f.toggle(TilePos::new(-3, 1));
        f.sides.east = -1;
        let t = f.target();
        assert!(t.contains(&TilePos::new(-3, 1)) && !t.contains(&TilePos::new(2, 0)));
        assert_eq!(t.len(), 7);
    }
}
