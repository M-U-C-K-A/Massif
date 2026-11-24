//! Reference images on the 2D map: pictures to trace over, as
//! WorldPainter's overlay. Each is read once in the background and lives on
//! the GPU as a texture with mipmaps, so that a large picture costs nothing
//! a frame whatever the zoom; it is drawn as a quad turned and scaled over
//! or under the shaded relief. Their handles on the map move, scale and turn
//! the one being placed; the button in the map's header lists them, with
//! their settings typed in. They are kept with the map (by their files'
//! paths) and never exported.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, row, well};
use egui::epaint::{Mesh, Vertex};
use egui::{Color32, ColorImage, CursorIcon, Pos2, RichText, Shape, Stroke, TextureFilter, TextureHandle, TextureOptions, Ui, pos2, vec2};
use massif_core::reference::ReferenceImage;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};

/// The largest side a picture keeps on the GPU, in pixels: larger ones are
/// scaled down to it (their placement on the map is the same).
const MAX_SIDE: u32 = 8192;

/// The pictures a reference image may be, by their files' extensions.
pub const IMAGE_EXTENSIONS: [&str; 8] = ["png", "jpg", "jpeg", "webp", "bmp", "tif", "tiff", "gif"];

/// Whether `path` is a picture Massif reads as a reference image.
pub fn is_image(path: &Path) -> bool {
    path.extension().is_some_and(|e| IMAGE_EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// A picture read, scaled to fit the GPU, with its own size.
type Read = Result<(ColorImage, [u32; 2]), String>;

enum Slot {
    Loading(Receiver<Read>),
    Ready(TextureHandle),
    Failed(String),
}

/// What a press on the map holds of the image being placed.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Handle {
    Move,
    /// A corner, by its place on the image (0 or 1 each way).
    Corner(f64, f64),
    Turn,
}

/// The reference images' textures, which one is being placed, and the
/// handle held.
#[derive(Default)]
pub struct References {
    textures: HashMap<String, Slot>,
    /// The image whose settings show and whose handles are on the map.
    pub selected: Option<usize>,
    /// Edit placement, picked in the header's menu: the handles of the
    /// selected image are on the map until Done or Esc.
    pub placing: bool,
    /// The Pan tool is in hand: the handles are on the map, and a press on
    /// an image picks it. Set by the map each frame. Otherwise the images
    /// are passive: presses and strokes go to the tool.
    pub pan: bool,
    grab: Option<(Handle, ReferenceImage, (f64, f64))>,
    /// Images added that take a size and a place once read: fitted to the
    /// map when larger than it, else a block a pixel where they were put.
    pending: Vec<String>,
}

impl References {
    /// Add the picture at `path` to the map's, with its middle at `(x,
    /// z)`: placed once it is read.
    pub fn add(&mut self, list: &mut Vec<ReferenceImage>, path: &Path, x: f64, z: f64) {
        let path = path.to_string_lossy().into_owned();
        list.push(ReferenceImage { path: path.clone(), x, z, ..ReferenceImage::default() });
        self.pending.push(path);
        self.selected = Some(list.len() - 1);
    }

    /// Whether the handles are on the map: with the Pan tool, or while
    /// placing.
    pub fn handles_on(&self) -> bool {
        self.placing || self.pan
    }

    /// Read what has to be, hand over what was read: textures for the
    /// pictures of `list` (their files found beside the map, `map_dir`,
    /// when moved with it). True when an image just added took its place.
    pub fn update(&mut self, ctx: &egui::Context, list: &mut [ReferenceImage], map_dir: Option<&Path>, map: Option<[f64; 4]>) -> bool {
        let mut changed = false;
        for r in list.iter_mut() {
            let slot = self.textures.entry(r.path.clone()).or_insert_with(|| {
                let (tx, rx) = channel();
                let file = locate(&r.path, map_dir);
                let ctx = ctx.clone();
                let work = move || {
                    let _ = tx.send(read(&file));
                    ctx.request_repaint();
                };
                if cfg!(test) {
                    work();
                } else {
                    let _ = std::thread::Builder::new().name("massif reference image".into()).spawn(work);
                }
                Slot::Loading(rx)
            });
            if let Slot::Loading(rx) = slot
                && let Ok(read) = rx.try_recv()
            {
                *slot = match read {
                    Ok((image, size)) => {
                        let options = TextureOptions { mipmap_mode: Some(TextureFilter::Linear), ..TextureOptions::LINEAR };
                        let texture = ctx.load_texture(format!("reference {}", r.path), image, options);
                        // One placed by its corner (a WorldPainter
                        // overlay) takes its place now its size is known.
                        changed |= r.set_size(size);
                        if let Some(k) = self.pending.iter().position(|p| *p == r.path) {
                            self.pending.remove(k);
                            if let Some([x0, z0, x1, z1]) = map
                                && (size[0] as f64 > x1 - x0 || size[1] as f64 > z1 - z0)
                            {
                                r.fit(x0, z0, x1, z1);
                            }
                            changed = true;
                        }
                        Slot::Ready(texture)
                    }
                    Err(e) => Slot::Failed(e),
                };
            }
        }
        changed
    }

    fn texture(&self, path: &str) -> Option<&TextureHandle> {
        match self.textures.get(path) {
            Some(Slot::Ready(t)) => Some(t),
            _ => None,
        }
    }

    /// Why the picture of `path` does not show, if it does not.
    pub fn problem(&self, path: &str) -> Option<&str> {
        match self.textures.get(path) {
            Some(Slot::Failed(e)) => Some(e),
            _ => None,
        }
    }

    /// How strongly the map shows over the images under it: less, the more
    /// opaque the most opaque of them.
    pub fn map_alpha(list: &[ReferenceImage]) -> f32 {
        let under = list.iter().filter(|r| r.shown && !r.above).map(|r| r.opacity.clamp(0.0, 1.0)).fold(0.0f32, f32::max);
        1.0 - under * 0.85
    }

    /// Draw the shown images over (`above`) or under the relief.
    pub fn draw(&self, painter: &egui::Painter, list: &[ReferenceImage], above: bool, to_screen: &dyn Fn(f64, f64) -> Pos2) {
        for r in list.iter().filter(|r| r.shown && r.above == above && r.size[0] > 0) {
            let Some(texture) = self.texture(&r.path) else { continue };
            // Under the relief, the image whole: the relief fades over it.
            let alpha = if above { r.opacity.clamp(0.0, 1.0) } else { 1.0 };
            let color = Color32::WHITE.gamma_multiply(alpha);
            let mut mesh = Mesh::with_texture(texture.id());
            for (k, (x, z)) in r.corners().into_iter().enumerate() {
                let uv = [pos2(0.0, 0.0), pos2(1.0, 0.0), pos2(1.0, 1.0), pos2(0.0, 1.0)][k];
                mesh.vertices.push(Vertex { pos: to_screen(x, z), uv, color });
            }
            mesh.indices.extend_from_slice(&[0, 1, 2, 0, 2, 3]);
            painter.add(Shape::mesh(mesh));
        }
    }

    /// The handles of the image being placed: its outline, its corners, the
    /// knob that turns it.
    pub fn draw_handles(&self, ui: &Ui, painter: &egui::Painter, list: &[ReferenceImage], to_screen: &dyn Fn(f64, f64) -> Pos2) {
        let Some(r) = self.placed(list) else { return };
        let pal = Palette::of(ui);
        let accent = theme::accent();
        let corners = r.corners().map(|(x, z)| to_screen(x, z));
        let stroke = Stroke::new(1.5, accent);
        painter.add(Shape::closed_line(corners.to_vec(), stroke));
        if r.locked {
            return;
        }
        for c in corners {
            let rect = egui::Rect::from_center_size(c, vec2(9.0, 9.0));
            painter.rect_filled(rect, theme::TILE_RADIUS / 3, pal.panel);
            painter.rect_stroke(rect, theme::TILE_RADIUS / 3, stroke, egui::StrokeKind::Middle);
        }
        let (top, knob) = turn_knob(r, to_screen);
        painter.line_segment([top, knob], stroke);
        painter.circle(knob, 5.0, pal.panel, stroke);
    }

    fn placed<'a>(&self, list: &'a [ReferenceImage]) -> Option<&'a ReferenceImage> {
        self.selected.filter(|_| self.handles_on()).and_then(|k| list.get(k)).filter(|r| r.shown && r.size[0] > 0)
    }

    /// The handle under the pointer at `p`, on the image being placed.
    fn handle_at(&self, list: &[ReferenceImage], p: Pos2, to_screen: &dyn Fn(f64, f64) -> Pos2, to_world: &dyn Fn(Pos2) -> (f64, f64)) -> Option<Handle> {
        let r = self.placed(list).filter(|r| !r.locked)?;
        let (_, knob) = turn_knob(r, to_screen);
        if knob.distance(p) <= 8.0 {
            return Some(Handle::Turn);
        }
        for (u, v) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            let (x, z) = r.at(u, v);
            if to_screen(x, z).distance(p) <= 8.0 {
                return Some(Handle::Corner(u, v));
            }
        }
        let (x, z) = to_world(p);
        r.contains(x, z).then_some(Handle::Move)
    }

    /// The pointer on the map: a press on a handle of the image being
    /// placed takes it, a drag moves, scales or turns the image. True while
    /// the pointer is on a handle or holds one (the tools then leave the
    /// press alone); `changed` once the image moved.
    #[allow(clippy::too_many_arguments)]
    pub fn interact(&mut self, ui: &Ui, list: &mut [ReferenceImage], pointer: Option<Pos2>, pressed: bool, down: bool, to_screen: &dyn Fn(f64, f64) -> Pos2, to_world: &dyn Fn(Pos2) -> (f64, f64), changed: &mut bool) -> bool {
        let Some(p) = pointer else {
            self.grab = None;
            return false;
        };
        if !self.handles_on() {
            self.grab = None;
            return false;
        }
        let mut over = self.handle_at(list, p, to_screen, to_world);
        // With the Pan tool, the image under the pointer (the topmost
        // shown and unlocked), when not on the handles of the one selected.
        if over.is_none() && self.pan && self.grab.is_none() {
            let (x, z) = to_world(p);
            if let Some(k) = list.iter().rposition(|r| r.shown && !r.locked && r.size[0] > 0 && r.contains(x, z)) {
                over = Some(Handle::Move);
                if pressed {
                    self.selected = Some(k);
                }
            }
        }
        if pressed && let (Some(h), Some(k)) = (over, self.selected) {
            self.grab = Some((h, list[k].clone(), to_world(p)));
        }
        if !down {
            self.grab = None;
        }
        let held = self.grab.as_ref().map(|g| g.0);
        if let (Some((handle, start, from)), Some(k)) = (&self.grab, self.selected)
            && let Some(r) = list.get_mut(k)
        {
            let (x, z) = to_world(p);
            let mut next = start.clone();
            match handle {
                Handle::Move => {
                    next.x = start.x + x - from.0;
                    next.z = start.z + z - from.1;
                }
                Handle::Corner(u, v) => next.scale_to((*u, *v), x, z),
                Handle::Turn => {
                    // The knob points from the middle along the image's
                    // up: its angle to the pointer is the turn.
                    let a = (x - start.x).atan2(-(z - start.z)).to_degrees() as f32;
                    let mut deg = a.rem_euclid(360.0);
                    if ui.input(|i| i.modifiers.shift) {
                        deg = (deg / 15.0).round() * 15.0 % 360.0;
                    }
                    next.rotation = deg;
                }
            }
            if next != *r {
                *r = next;
                *changed = true;
            }
        }
        match held.or(over) {
            Some(Handle::Move) => ui.ctx().set_cursor_icon(if held.is_some() { CursorIcon::Grabbing } else { CursorIcon::Grab }),
            Some(Handle::Corner(..)) => ui.ctx().set_cursor_icon(CursorIcon::ResizeNwSe),
            Some(Handle::Turn) => ui.ctx().set_cursor_icon(CursorIcon::Alias),
            None => {}
        }
        held.is_some() || over.is_some()
    }

    /// While placing: a pill at the top of the map, Done, which ends it, as
    /// Esc does.
    pub fn done_pill(&mut self, ui: &mut Ui, canvas: egui::Rect) {
        if !self.placing {
            return;
        }
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.placing = false;
            return;
        }
        let pal = Palette::of(ui);
        let text = tr!("Placing the image · Done");
        let font = egui::FontId::proportional(12.5);
        let galley = ui.painter().layout_no_wrap(text.to_owned(), font, pal.text);
        let rect = egui::Rect::from_center_size(pos2(canvas.center().x, canvas.top() + 22.0), galley.size() + vec2(28.0, 12.0));
        let r = ui.interact(rect, egui::Id::unique("reference placing done"), egui::Sense::click());
        r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Done placing the image")));
        let fill = if r.hovered() { theme::mix(pal.panel, pal.text, 0.08) } else { pal.panel };
        ui.painter().rect(rect, rect.height() / 2.0, fill, Stroke::new(1.0, theme::accent()), egui::StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, pal.text);
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if r.clicked() {
            self.placing = false;
        }
    }

    /// The button in the map's header: the images, each with an eye and a
    /// lock, the settings of the one selected, and adding one. True when the
    /// images changed; `add` when a file is to be picked.
    pub fn header(&mut self, ui: &mut Ui, list: &mut Vec<ReferenceImage>, add: &mut bool) -> bool {
        let mut changed = false;
        let icon = theme::glyph(Icon::ImagePlus).to_string();
        let r = ui.menu_button(icon, |ui| {
            changed |= self.menu(ui, list, add);
        });
        r.response.on_hover_text(tr!("Reference images: pictures to trace over, never exported"));
        changed
    }

    fn menu(&mut self, ui: &mut Ui, list: &mut Vec<ReferenceImage>, add: &mut bool) -> bool {
        let pal = Palette::of(ui);
        let mut changed = false;
        ui.set_min_width(300.0);
        ui.label(RichText::new(tr!("Reference images")).medium());
        ui.label(RichText::new(tr!("Pictures to trace over: shown on the map only, never exported. Drop one on the map to add it.")).size(11.0).color(pal.weak));
        ui.add_space(4.0);
        let mut remove = None;
        for (k, r) in list.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                let eye = if r.shown { Icon::Eye } else { Icon::EyeOff };
                if ui.add(egui::Button::new(RichText::new(theme::glyph(eye).to_string()).color(if r.shown { pal.text } else { pal.weak })).frame(false)).on_hover_text(if r.shown { tr!("Hide it") } else { tr!("Show it") }).clicked() {
                    r.shown = !r.shown;
                    changed = true;
                }
                let lock = if r.locked { Icon::Lock } else { Icon::LockOpen };
                if ui.add(egui::Button::new(RichText::new(theme::glyph(lock).to_string()).color(if r.locked { pal.text } else { pal.weak })).frame(false)).on_hover_text(if r.locked { tr!("Unlock it") } else { tr!("Lock it in place") }).clicked() {
                    r.locked = !r.locked;
                    changed = true;
                }
                let name = r.name().to_owned();
                let ink = if self.problem(&r.path).is_some() { pal.danger } else { pal.text };
                if ui.selectable_label(self.selected == Some(k), RichText::new(name).color(ink)).clicked() {
                    self.selected = if self.selected == Some(k) { None } else { Some(k) };
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Take it off the map")).clicked() {
                        remove = Some(k);
                    }
                });
            });
        }
        if let Some(k) = remove {
            list.remove(k);
            self.selected = None;
            self.placing = false;
            changed = true;
        }
        if list.is_empty() {
            ui.label(RichText::new(tr!("None yet.")).size(11.5).color(pal.weak));
        }
        if let Some(r) = self.selected.and_then(|k| list.get_mut(k)) {
            ui.add_space(4.0);
            if let Some(e) = self.problem(&r.path).map(str::to_owned) {
                ui.label(RichText::new(tr!("Not read: {e}", e)).size(11.0).color(pal.danger));
                // Its file moved: found again, it keeps its place.
                if crate::widgets::button(ui, theme::with_icon(Icon::ImagePlus, tr!("Find its file…"))).clicked()
                    && let Some(file) = rfd::FileDialog::new().set_title(tr!("Find its file")).add_filter(tr!("Images"), &IMAGE_EXTENSIONS).pick_file()
                {
                    self.textures.remove(&r.path);
                    r.path = file.to_string_lossy().into_owned();
                    changed = true;
                }
            }
            let before = r.clone();
            well(ui, |ui| {
                row(ui, tr!("Place"), |ui| {
                    if let Some(k) = button_group(ui, "reference place", &[tr!("Over the map"), tr!("Under it")], &[r.above, !r.above]) {
                        r.above = k == 0;
                    }
                });
                row(ui, tr!("Opacity"), |ui| {
                    ui.add(Field::new(&mut r.opacity, 0.05..=1.0).custom_formatter(|v, _| format!("{:.0}%", v * 100.0)).custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)));
                });
                ui.add_enabled_ui(!r.locked, |ui| {
                    row(ui, tr!("Middle"), |ui| {
                        crate::widgets::drag(ui, egui::DragValue::new(&mut r.x).speed(1.0).prefix("x ").max_decimals(1));
                        crate::widgets::drag(ui, egui::DragValue::new(&mut r.z).speed(1.0).prefix("z ").max_decimals(1));
                    });
                    row(ui, tr!("Scale"), |ui| {
                        ui.add(Field::new(&mut r.scale, 0.01..=64.0).logarithmic(true).max_decimals(3).suffix(tr!(" blocks a pixel")));
                    });
                    row(ui, tr!("Rotation"), |ui| {
                        ui.add(Field::new(&mut r.rotation, 0.0..=360.0).max_decimals(1).suffix("°"));
                    });
                });
            });
            if !r.locked && crate::widgets::button(ui, theme::with_icon(Icon::Move, tr!("Edit placement"))).on_hover_text(tr!("Its handles on the map: drag it, a corner, the knob to turn it. Done or Esc ends it; the Pan tool shows them too")).clicked() {
                self.placing = true;
                ui.close();
            }
            if r.size[0] > 0 {
                let (w, d) = (r.size[0] as f32 * r.scale, r.size[1] as f32 * r.scale);
                ui.label(RichText::new(tr!("{px} × {py} pixels: {w:.0} × {d:.0} blocks on the map.", px = r.size[0], py = r.size[1], w, d)).size(11.0).color(pal.weak));
            }
            changed |= *r != before;
        }
        ui.add_space(4.0);
        if crate::widgets::button(ui, theme::with_icon(Icon::ImagePlus, tr!("Add an image…"))).clicked() {
            *add = true;
            ui.close();
        }
        changed
    }
}

/// The top middle of `r` and the knob above it that turns it, on screen.
fn turn_knob(r: &ReferenceImage, to_screen: &dyn Fn(f64, f64) -> Pos2) -> (Pos2, Pos2) {
    let top = r.at(0.5, 0.0);
    let mid = r.at(0.5, 0.5);
    let (a, b) = (to_screen(top.0, top.1), to_screen(mid.0, mid.1));
    let up = (a - b).normalized();
    let up = if up.is_finite() { up } else { vec2(0.0, -1.0) };
    (a, a + up * 22.0)
}

/// The file of `path`, or the one of its name beside the map when it was
/// moved with it.
fn locate(path: &str, map_dir: Option<&Path>) -> PathBuf {
    let p = PathBuf::from(path);
    if p.exists() {
        return p;
    }
    // Its name, whichever system wrote the path (a Windows one in a
    // WorldPainter world).
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    match map_dir {
        Some(dir) if !name.is_empty() && dir.join(name).exists() => dir.join(name),
        _ => p,
    }
}

/// Read the picture at `path`, scaled down to fit the GPU.
fn read(path: &Path) -> Read {
    let img = image::ImageReader::open(path).map_err(|e| e.to_string())?.with_guessed_format().map_err(|e| e.to_string())?.decode().map_err(|e| e.to_string())?;
    let size = [img.width(), img.height()];
    // Past what the GPU holds, a smaller copy (its place on the map is the
    // same: it keeps its size in pixels).
    let img = if size[0].max(size[1]) > MAX_SIDE { img.thumbnail(MAX_SIDE, MAX_SIDE) } else { img };
    let rgba = img.to_rgba8();
    let image = ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw());
    Ok((image, size))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every common picture reads, the first frame of a GIF; one past what
    /// the GPU holds is made smaller and keeps its own size.
    #[test]
    fn reference_images_read_every_common_format_and_large_ones() {
        let dir = std::env::temp_dir().join(format!("massif-reference-formats-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let img = image::RgbImage::from_fn(40, 30, |x, y| image::Rgb([(x * 6) as u8, (y * 8) as u8, 90]));
        for ext in IMAGE_EXTENSIONS {
            let path = dir.join(format!("plan.{ext}"));
            match ext {
                // WebP and GIF are written with alpha.
                "webp" | "gif" => image::DynamicImage::ImageRgb8(img.clone()).to_rgba8().save(&path).unwrap(),
                _ => img.save(&path).unwrap(),
            }
            assert!(is_image(&path), "{ext}");
            let (picture, size) = read(&path).unwrap_or_else(|e| panic!("{ext}: {e}"));
            assert_eq!((size, picture.size), ([40, 30], [40, 30]), "{ext}");
        }
        assert!(!is_image(Path::new("map.massif")));
        let big = dir.join("big.png");
        image::GrayImage::from_pixel(9000, 120, image::Luma([200])).save(&big).unwrap();
        let (picture, size) = read(&big).unwrap();
        assert_eq!(size, [9000, 120], "its own size, to place it");
        assert!(picture.size[0] <= MAX_SIDE as usize && picture.size[0] >= 8000, "{:?}", picture.size);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
