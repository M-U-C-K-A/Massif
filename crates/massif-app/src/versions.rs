//! The Versions panel: the map's save points (see
//! [`massif_core::versions`]), newest first, each with its picture, to put
//! back, open on its own, rename, delete or compare with the map.
//!
//! Comparing draws over the 2D map where the land differs: the accent where
//! its height does (the stronger, the more blocks), grey where only its
//! paint does (terrain, biome, ground cover, objects or water).

use crate::theme::{self, Icon, Palette};
use egui::{Color32, RichText, Sense, Ui, vec2};
use massif_core::coords::{TILE_SIZE, local_index};
use massif_core::versions::SavePoint;
use massif_core::{TilePos, World};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::Instant;

/// What the panel asks of the app.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// Save a new point (its name asked first).
    Make,
    Restore(usize),
    /// Open a point as a map of its own: read-only, or as a copy to change.
    Open { index: usize, read_only: bool },
    Rename(usize),
    Delete(usize),
    /// Compare a point with the map, or stop comparing.
    Compare(Option<usize>),
}

/// The longest side of the comparison's picture, in pixels.
const DIFF_SIZE: i32 = 2048;

/// Where the map and a save point differ, a pixel for every `step` × `step`
/// blocks over both maps.
#[derive(Debug, Clone, Default)]
pub struct Diff {
    /// Block coordinates of the picture: west, north, east, south.
    pub rect: [i32; 4],
    pub size: [usize; 2],
    /// Per pixel: 0 the same, 1 painted differently, from 2 a height that
    /// differs by `code - 1` blocks (at most 254).
    pub cells: Vec<u8>,
    /// Tiles that differ, and of their columns those higher now, lower now,
    /// and only painted differently.
    pub tiles: usize,
    pub raised: u64,
    pub lowered: u64,
    pub painted: u64,
    /// The most the land rose and sank, in blocks.
    pub most_up: f32,
    pub most_down: f32,
}

impl Diff {
    pub fn is_empty(&self) -> bool {
        self.raised + self.lowered + self.painted == 0
    }
}

/// Where dimension `dim` of `now` differs from that of `then`. Both must be
/// cold (see [`World::snapshot`]).
pub fn diff(now: &World, then: &World, dim: usize) -> Diff {
    use rayon::prelude::*;
    let (Some(a), Some(b)) = (now.dims.get(dim), then.dims.get(dim)) else {
        return Diff::default();
    };
    let bounds = [a.bounds(), b.bounds()].into_iter().flatten().reduce(|(lo, hi), (l, h)| {
        (TilePos::new(lo.x.min(l.x), lo.z.min(l.z)), TilePos::new(hi.x.max(h.x), hi.z.max(h.z)))
    });
    let Some((lo, hi)) = bounds else {
        return Diff::default();
    };
    let (x0, z0) = (lo.x * TILE_SIZE, lo.z * TILE_SIZE);
    let (w, d) = ((hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE);
    let mut step = 1;
    while w.max(d) / step > DIFF_SIZE {
        step *= 2;
    }
    let size = [(w / step).max(1) as usize, (d / step).max(1) as usize];
    let tiles = SavePoint::changed_tiles(now, then, dim);
    struct Part {
        cells: Vec<(usize, u8)>,
        raised: u64,
        lowered: u64,
        painted: u64,
        up: f32,
        down: f32,
    }
    let parts: Vec<Part> = tiles
        .par_iter()
        .map(|&t| {
            let sa = massif_core::compose::TileStack::new(a, t, false);
            let sb = massif_core::compose::TileStack::new(b, t, false);
            let (mut ca, mut cb) = (crate::map_render::ColumnSampler::default(), crate::map_render::ColumnSampler::default());
            // The tile's pixels, at most 128 × 128.
            let n = (TILE_SIZE / step).max(1) as usize;
            let mut local = vec![0u8; n * n];
            let mut part = Part {
                cells: Vec::new(),
                raised: 0,
                lowered: 0,
                painted: 0,
                up: 0.0,
                down: 0.0,
            };
            for j in 0..TILE_SIZE {
                for i in 0..TILE_SIZE {
                    let cell = local_index(t.x * TILE_SIZE + i, t.z * TILE_SIZE + j);
                    let pa = sa.as_ref().map_or(crate::map_render::Sample::EMPTY, |s| ca.sample(s, cell));
                    let pb = sb.as_ref().map_or(crate::map_render::Sample::EMPTY, |s| cb.sample(s, cell));
                    let code = match (pa.height.is_nan(), pb.height.is_nan()) {
                        (true, true) => 0,
                        (false, false) => {
                            let dh = pa.height - pb.height;
                            if dh.abs() >= 0.5 {
                                if dh > 0.0 {
                                    part.raised += 1;
                                    part.up = part.up.max(dh);
                                } else {
                                    part.lowered += 1;
                                    part.down = part.down.max(-dh);
                                }
                                (dh.abs().round() as u32).clamp(1, 254) as u8 + 1
                            } else if (pa.terrain, pa.biome, pa.cover, pa.objects, pa.fluid) != (pb.terrain, pb.biome, pb.cover, pb.objects, pb.fluid) {
                                part.painted += 1;
                                1
                            } else {
                                0
                            }
                        }
                        // Land on one side only: as different as can be.
                        (nan_now, _) => {
                            if nan_now {
                                part.lowered += 1;
                            } else {
                                part.raised += 1;
                            }
                            255
                        }
                    };
                    let k = (j as usize * n / TILE_SIZE as usize) * n + i as usize * n / TILE_SIZE as usize;
                    local[k] = local[k].max(code);
                }
            }
            let (px, pz) = (((t.x * TILE_SIZE - x0) / step) as usize, ((t.z * TILE_SIZE - z0) / step) as usize);
            for (k, &code) in local.iter().enumerate() {
                let (x, z) = (px + k % n, pz + k / n);
                if code > 0 && x < size[0] && z < size[1] {
                    part.cells.push((z * size[0] + x, code));
                }
            }
            part
        })
        .collect();
    let mut out = Diff {
        rect: [x0, z0, x0 + size[0] as i32 * step, z0 + size[1] as i32 * step],
        size,
        cells: vec![0; size[0] * size[1]],
        tiles: tiles.len(),
        ..Diff::default()
    };
    for part in parts {
        for (k, code) in part.cells {
            out.cells[k] = out.cells[k].max(code);
        }
        out.raised += part.raised;
        out.lowered += part.lowered;
        out.painted += part.painted;
        out.most_up = out.most_up.max(part.up);
        out.most_down = out.most_down.max(part.down);
    }
    out
}

/// The comparison of the map with a save point, worked out away from the
/// frames and again a moment after the map changes.
pub struct Compare {
    /// The point compared: its index, and its date and name to know it.
    pub index: usize,
    key: (u64, String),
    /// The revision, generation and dimension of the map it was worked out
    /// at.
    at: (u64, u64, usize),
    running: Option<Receiver<Diff>>,
    started: Option<Instant>,
    pub diff: Option<Arc<Diff>>,
    texture: Option<(egui::TextureHandle, [i32; 4], Color32)>,
}

/// The panel's state: the points' pictures, the comparison.
#[derive(Default)]
pub struct VersionsUi {
    thumbs: HashMap<usize, egui::TextureHandle>,
    pub compare: Option<Compare>,
}

/// A point's date, said as the panel says it: how long ago within a week,
/// then its day.
pub fn when(created: u64, now: u64) -> String {
    let secs = now.saturating_sub(created);
    match secs {
        0..60 => tr!("Just now").into(),
        60..3600 => trn!("{n} minute ago", "{n} minutes ago", n = secs / 60),
        3600..86400 => trn!("{n} hour ago", "{n} hours ago", n = secs / 3600),
        86400..604800 => trn!("{n} day ago", "{n} days ago", n = secs / 86400),
        _ => day(created),
    }
}

/// "12 Nov 2026" (UTC).
fn day(secs: u64) -> String {
    // Days since 1970 to a civil date (Howard Hinnant's algorithm).
    let z = (secs / 86400) as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + (m <= 2) as i64;
    const MONTHS: [&str; 12] = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
    format!("{d} {} {y}", MONTHS[(m - 1) as usize])
}

impl VersionsUi {
    /// Compare point `index` of the map with it (`None`: stop comparing).
    pub fn compare_with(&mut self, points: &[SavePoint], index: Option<usize>) {
        self.compare = index.and_then(|i| points.get(i).map(|p| (i, p))).map(|(index, p)| Compare {
            index,
            key: (p.created, p.name.clone()),
            at: (u64::MAX, 0, 0),
            running: None,
            started: None,
            diff: None,
            texture: None,
        });
    }

    /// Keep the comparison up to date with the map: stopped if its point is
    /// gone, worked out again a moment after the map changed. Returns the
    /// picture to draw over the map, and its block coordinates.
    pub fn overlay(&mut self, ctx: &egui::Context, doc: &crate::doc::Document) -> Option<(egui::TextureHandle, [i32; 4])> {
        let compare = self.compare.as_mut()?;
        // The point, wherever it now is in the list.
        let Some(index) = doc.world.points.iter().position(|p| (p.created, &p.name) == (compare.key.0, &compare.key.1)) else {
            self.compare = None;
            return None;
        };
        compare.index = index;
        if let Some(rx) = &compare.running {
            match rx.try_recv() {
                Ok(diff) => {
                    compare.diff = Some(Arc::new(diff));
                    compare.texture = None;
                    compare.running = None;
                }
                Err(TryRecvError::Disconnected) => compare.running = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        let now = (doc.revision, doc.generation, doc.dim);
        let due = compare.started.is_none_or(|t| t.elapsed().as_secs_f32() > 1.0);
        if compare.at != now && compare.running.is_none() && !doc.history.is_recording() {
            if due {
                let (map, then, dim) = (doc.world.copy_unfrozen(), doc.world.points[index].world.clone(), doc.dim);
                let (tx, rx) = std::sync::mpsc::channel();
                let ctx = ctx.clone();
                let spawned = std::thread::Builder::new().name("massif compare".into()).spawn(move || {
                    let mut map = map;
                    map.points.clear();
                    let _ = tx.send(diff(&map.snapshot(), &then, dim));
                    ctx.request_repaint();
                });
                if spawned.is_ok() {
                    compare.running = Some(rx);
                    compare.started = Some(Instant::now());
                    compare.at = now;
                }
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }
        let diff = compare.diff.clone()?;
        let visuals = ctx.global_style().visuals.clone();
        let dark = visuals.dark_mode;
        // The accent of the settings, as the rest of the window has it.
        let pal = Palette::themed(dark, theme::is_high(&visuals));
        if compare.texture.as_ref().is_none_or(|t| t.2 != pal.accent) {
            let paint = if dark { Color32::from_gray(200) } else { Color32::from_gray(60) };
            let pixels: Vec<Color32> = diff
                .cells
                .iter()
                .map(|&c| match c {
                    0 => Color32::TRANSPARENT,
                    1 => paint.gamma_multiply(0.45),
                    // A block's difference shows; eight or more, fully.
                    h => pal.accent.gamma_multiply(0.3 + 0.45 * ((h - 1) as f32 / 8.0).min(1.0)),
                })
                .collect();
            let image = egui::ColorImage::new(diff.size, pixels);
            compare.texture = Some((ctx.load_texture("save point comparison", image, egui::TextureOptions::NEAREST), diff.rect, pal.accent));
        }
        compare.texture.as_ref().map(|t| (t.0.clone(), t.1))
    }

    fn thumbnail(&mut self, ctx: &egui::Context, point: &SavePoint) -> Option<egui::TextureHandle> {
        let jpeg = point.thumbnail.as_ref()?;
        let key = jpeg.as_ptr() as usize;
        if let Some(t) = self.thumbs.get(&key) {
            return Some(t.clone());
        }
        let img = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).ok()?.to_rgba8();
        let image = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
        let texture = ctx.load_texture(format!("save point {}", point.name), image, egui::TextureOptions::LINEAR);
        self.thumbs.insert(key, texture.clone());
        Some(texture)
    }

    /// The panel. `busy`: a point is being saved.
    pub fn panel(&mut self, ui: &mut Ui, doc: &crate::doc::Document, busy: bool) -> Option<Action> {
        let pal = Palette::of(ui);
        let mut action = None;
        let points = &doc.world.points;
        // Pictures of points gone are let go.
        if self.thumbs.len() > points.len() {
            let alive: Vec<usize> = points.iter().filter_map(|p| p.thumbnail.as_ref().map(|j| j.as_ptr() as usize)).collect();
            self.thumbs.retain(|k, _| alive.contains(k));
        }
        ui.horizontal(|ui| {
            let label = if busy { tr!("Saving a point…") } else { tr!("Save point…") };
            if ui.add_enabled(!busy && doc.read_only.is_none(), theme::primary_button(ui, format!("{}  {label}", theme::glyph(Icon::BookmarkPlus)))).clicked() {
                action = Some(Action::Make);
            }
        });
        ui.add_space(4.0);
        if let Some(name) = &doc.read_only {
            crate::panels::hint(ui, &tr!("This is “{name}”, open read-only: Save As… makes it a map of its own to change.", name));
            return action;
        }
        if points.is_empty() {
            crate::panels::hint(ui, tr!("No save point yet. Save one before a big change, to come back to it or compare."));
            return action;
        }
        crate::panels::hint(ui, tr!("Kept in the map's file. A point takes only the tiles changed since."));
        ui.add_space(2.0);
        let now = massif_core::versions::now();
        let comparing = self.compare.as_ref().map(|c| c.index);
        for index in (0..points.len()).rev() {
            let point = &points[index];
            let thumb = self.thumbnail(ui.ctx(), point);
            let active = comparing == Some(index);
            let frame = egui::Frame::new()
                .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
                .stroke(egui::Stroke::new(1.0, if active { pal.accent } else { pal.border }))
                .corner_radius(theme::TILE_RADIUS)
                .inner_margin(egui::Margin::same(6));
            frame.show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    // The picture, at most 72 × 48.
                    let size = thumb.as_ref().map_or(vec2(72.0, 48.0), |t| {
                        let s = t.size_vec2();
                        let k = (72.0 / s.x).min(48.0 / s.y);
                        s * k
                    });
                    let (rect, _) = ui.allocate_exact_size(vec2(72.0, 48.0), Sense::hover());
                    ui.painter().rect_filled(rect, theme::TILE_RADIUS / 2, pal.canvas);
                    if let Some(t) = &thumb {
                        let r = egui::Rect::from_center_size(rect.center(), size);
                        ui.painter().image(t.id(), r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), Color32::WHITE);
                    }
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.horizontal(|ui| {
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.menu_button(theme::glyph(Icon::Ellipsis).to_string(), |ui| {
                                    if ui.button(tr!("{glyph}  Open as a copy", glyph = theme::glyph(Icon::Copy))).clicked() {
                                        action = Some(Action::Open { index, read_only: false });
                                    }
                                    if ui.button(tr!("{glyph}  Open read-only", glyph = theme::glyph(Icon::Lock))).clicked() {
                                        action = Some(Action::Open { index, read_only: true });
                                    }
                                    ui.separator();
                                    if ui.button(tr!("{glyph}  Rename…", glyph = theme::glyph(Icon::Pencil))).clicked() {
                                        action = Some(Action::Rename(index));
                                    }
                                    if ui.button(tr!("{glyph}  Delete…", glyph = theme::glyph(Icon::Trash2))).clicked() {
                                        action = Some(Action::Delete(index));
                                    }
                                });
                                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                                    ui.add(egui::Label::new(RichText::new(&point.name).size(13.0).strong()).truncate());
                                });
                            });
                        });
                        ui.label(RichText::new(when(point.created, now)).size(11.0).color(pal.weak));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            if ui.small_button(tr!("Restore")).on_hover_text(tr!("Put the map back as it was then, as one step to undo")).clicked() {
                                action = Some(Action::Restore(index));
                            }
                            let label = if active { tr!("Stop") } else { tr!("Compare") };
                            let r = ui.small_button(label);
                            let r = if active { r.on_hover_text(tr!("Stop comparing")) } else { r.on_hover_text(tr!("Show on the 2D map where the map differs from this point")) };
                            if r.clicked() {
                                action = Some(Action::Compare((!active).then_some(index)));
                            }
                        });
                    });
                });
                if active {
                    compare_legend(ui, self.compare.as_ref().and_then(|c| c.diff.as_deref()));
                }
            });
            ui.add_space(3.0);
        }
        action
    }
}

/// What the comparison found, under its point.
fn compare_legend(ui: &mut Ui, diff: Option<&Diff>) {
    let pal = Palette::of(ui);
    ui.add_space(4.0);
    let Some(diff) = diff else {
        ui.label(RichText::new(tr!("Comparing…")).size(11.0).color(pal.weak));
        return;
    };
    if diff.is_empty() {
        ui.label(RichText::new(tr!("The map is the same as this point.")).size(11.0).color(pal.weak));
        return;
    }
    let swatch = |ui: &mut Ui, color: Color32, text: String| {
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
            ui.painter().rect_filled(r, 2, color);
            ui.label(RichText::new(text).size(11.0).color(pal.weak));
        });
    };
    let blocks = |n: f32| trn!("{n} block", "{n} blocks", n = n.round() as i64);
    if diff.raised + diff.lowered > 0 {
        let mut parts = Vec::new();
        if diff.raised > 0 {
            parts.push(tr!("{n} columns higher now (up to {blocks})", n = count(diff.raised), blocks = blocks(diff.most_up)));
        }
        if diff.lowered > 0 {
            parts.push(tr!("{n} lower (up to {blocks})", n = count(diff.lowered), blocks = blocks(diff.most_down)));
        }
        swatch(ui, pal.accent, parts.join(", "));
    }
    if diff.painted > 0 {
        let grey = if pal.dark { Color32::from_gray(200) } else { Color32::from_gray(60) };
        swatch(ui, grey.gamma_multiply(0.45), tr!("{n} columns painted differently", n = count(diff.painted)));
    }
    ui.label(RichText::new(tr!("Over {n} tiles, drawn on the 2D map.", n = count(diff.tiles as u64))).size(11.0).color(pal.weak));
}

/// 12,345.
fn count(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (k, c) in s.chars().enumerate() {
        if k > 0 && (s.len() - k).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_as_people_say_them() {
        assert_eq!(when(1000, 1030), "Just now");
        assert_eq!(when(1000, 1000 + 3 * 3600 + 5), "3 hours ago");
        assert_eq!(when(1000, 1000 + 86400), "1 day ago");
        assert_eq!(when(1_763_290_000, 1_763_290_000 + 30 * 86400), "16 Nov 2025");
        assert_eq!(count(1_234_567), "1,234,567");
    }
}
