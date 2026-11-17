//! Edit ▸ Import mask: an image painted onto the map, as WorldPainter's
//! Import mask. The image is read once in the background for its range
//! and a picture of it (`massif_core::mask::survey`); the dialog then says
//! what it paints (annotations, a layer, a terrain, a biome, water, the
//! height), where it lands, how its values are read and how it goes with
//! what is painted, with a picture of where it lands; the import runs in
//! the background, as one step to undo. The image may stay on the 2D map
//! as a guide to trace over, never exported.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, combo, row, switch, well};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::mask::{ImageInfo, Mapping, MaskSettings, Mode, Placement, Target};
use massif_core::registry::Registry;
use massif_core::tile::ANNOTATION_COLORS;
use std::path::PathBuf;
use std::sync::Arc;

/// What kind of thing the image paints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Annotations,
    Layer,
    Terrain,
    Biome,
    Water,
    Height,
}

impl Kind {
    const ALL: [Kind; 6] = [Kind::Annotations, Kind::Layer, Kind::Terrain, Kind::Biome, Kind::Water, Kind::Height];

    fn label(self) -> &'static str {
        match self {
            Kind::Annotations => tr!("Annotations"),
            Kind::Layer => tr!("Layer"),
            Kind::Terrain => tr!("Terrain"),
            Kind::Biome => tr!("Biome"),
            Kind::Water => tr!("Water"),
            Kind::Height => tr!("Height"),
        }
    }
}

/// A layer the image can paint: its target, its name and colour.
#[derive(Debug, Clone)]
struct LayerChoice {
    target: Target,
    name: String,
    color: [u8; 3],
}

/// Where the image lands, as the buttons set it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Origin {
    /// Its top left corner on x 0, z 0, a block a pixel.
    MapOrigin,
    /// As large as the map lets it, in its middle.
    Fit,
}

/// The dialog's settings and what it shows.
pub struct MaskForm {
    pub path: PathBuf,
    pub info: Arc<ImageInfo>,
    /// The map's picture and the blocks it covers (west, north, east, south).
    map_picture: Option<(egui::ColorImage, [i32; 4])>,
    map_texture: Option<egui::TextureHandle>,
    /// The map's blocks.
    bounds: [i32; 4],
    pub(crate) kind: Kind,
    /// Annotations: each pixel's nearest colour (true), or `colour`.
    by_colour: bool,
    colour: u16,
    layers: Vec<LayerChoice>,
    pub(crate) layer: usize,
    terrains: Vec<(u16, String, [u8; 3])>,
    terrain: u16,
    biomes: Vec<(u16, String, [u8; 3])>,
    biome: u16,
    water: i32,
    black: f32,
    white: f32,
    heights: (i32, i32),
    pub mapping: Mapping,
    pub mode: Mode,
    origin: Option<Origin>,
    x: i32,
    z: i32,
    /// Blocks a pixel.
    scale: f32,
    /// Keep the image on the map to trace over, at this opacity.
    pub guide: bool,
    pub opacity: f32,
    preview: Option<(egui::TextureHandle, u64)>,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    Import(MaskSettings),
}

impl MaskForm {
    /// The dialog for the image at `path`, surveyed as `info`, onto a map of
    /// `bounds` (blocks), its picture `map`.
    pub fn new(path: PathBuf, info: Arc<ImageInfo>, reg: &Registry, bounds: [i32; 4], heights: (i32, i32), sea: i32, map: Option<(egui::ColorImage, [i32; 4])>) -> Self {
        let mut layers = Vec::new();
        for (k, c) in reg.covers.iter().enumerate().filter(|(_, c)| !c.removed) {
            layers.push(LayerChoice { target: Target::Cover(k as u16 + 1), name: c.name.clone(), color: c.color });
        }
        for (k, l) in reg.object_layers.iter().enumerate().filter(|(_, l)| !l.removed) {
            layers.push(LayerChoice { target: Target::Trees(k as u16 + 1), name: l.name.clone(), color: l.color });
            layers.push(LayerChoice { target: Target::Understory(k as u16 + 1), name: tr!("{name}, under the trees", name = l.name), color: l.color });
        }
        for (k, p) in reg.pockets.iter().enumerate().filter(|(_, p)| !p.removed) {
            layers.push(LayerChoice { target: Target::Pockets(k as u16 + 1), name: tr!("Pockets: {name}", name = p.name), color: p.color });
        }
        for (n, name) in [(1u16, tr!("Tree size: bushes")), (2, tr!("Tree size: small")), (3, tr!("Tree size: medium")), (4, tr!("Tree size: large"))] {
            layers.push(LayerChoice { target: Target::TreeSize(n), name: name.to_owned(), color: [120, 160, 90] });
        }
        let terrains: Vec<(u16, String, [u8; 3])> = reg.terrains.iter().enumerate().map(|(k, t)| (k as u16, t.name.clone(), t.color)).collect();
        let biomes: Vec<(u16, String, [u8; 3])> = reg.biomes.iter().enumerate().skip(1).map(|(k, b)| (k as u16, b.name.clone(), b.color)).collect();
        let terrain = reg.terrain_by_name("Grass").map_or(0, |t| t.0);
        let biome = biomes.first().map_or(1, |b| b.0);
        // A colour image is a map of annotations, a grey one of strength.
        let kind = if info.colour { Kind::Annotations } else { Kind::Layer };
        let mapping = Mapping {
            full_range: !info.header.float,
            threshold: if info.header.bits <= 8 && !info.header.float { Some(0.5) } else { None },
            ..Mapping::default()
        };
        let mut form = Self {
            path,
            info,
            map_picture: map,
            map_texture: None,
            bounds,
            kind,
            by_colour: true,
            colour: 14,
            layers,
            layer: 0,
            terrains,
            terrain,
            biomes,
            biome,
            water: sea,
            black: heights.0.max(0) as f32,
            white: (heights.0.max(0) + 128).min(heights.1) as f32,
            heights,
            mapping,
            mode: Mode::Replace,
            origin: None,
            x: 0,
            z: 0,
            scale: 1.0,
            guide: false,
            opacity: 0.5,
            preview: None,
        };
        // An image of the map's size lands on it; another one at the origin.
        let (w, h) = (form.info.header.width as i32, form.info.header.height as i32);
        if (w, h) == (bounds[2] - bounds[0], bounds[3] - bounds[1]) {
            form.set_origin(Origin::Fit);
        } else {
            form.set_origin(Origin::MapOrigin);
        }
        form
    }

    fn set_origin(&mut self, origin: Origin) {
        self.origin = Some(origin);
        let (w, h) = (self.info.header.width as f64, self.info.header.height as f64);
        match origin {
            Origin::MapOrigin => {
                (self.x, self.z, self.scale) = (0, 0, 1.0);
            }
            Origin::Fit => {
                let [x0, z0, x1, z1] = self.bounds;
                let (mw, mh) = ((x1 - x0) as f64, (z1 - z0) as f64);
                let s = (mw / w).min(mh / h);
                self.scale = s as f32;
                self.x = x0 + ((mw - w * s) / 2.0).round() as i32;
                self.z = z0 + ((mh - h * s) / 2.0).round() as i32;
            }
        }
    }

    /// The target the settings make.
    pub fn target(&self) -> Target {
        match self.kind {
            Kind::Annotations => Target::Annotations((!self.by_colour || !self.info.colour).then_some(self.colour)),
            Kind::Layer => self.layers.get(self.layer).map_or(Target::TreeSize(1), |l| l.target),
            Kind::Terrain => Target::Terrain(self.terrain),
            Kind::Biome => Target::Biome(self.biome),
            Kind::Water => Target::Water(self.water.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16),
            Kind::Height => Target::Height { black: self.black, white: self.white },
        }
    }

    pub fn placement(&self) -> Placement {
        Placement { x: self.x, z: self.z, scale: self.scale }
    }

    pub fn settings(&self) -> MaskSettings {
        MaskSettings {
            target: self.target(),
            mapping: self.mapping,
            mode: self.mode,
            placement: self.placement(),
        }
    }

    /// The colour the target shows in.
    fn tint(&self) -> [u8; 3] {
        match self.target() {
            Target::Annotations(Some(c)) => ANNOTATION_COLORS[(c.clamp(1, 15) - 1) as usize],
            Target::Terrain(t) => self.terrains.iter().find(|x| x.0 == t).map_or([200; 3], |x| x.2),
            Target::Biome(b) => self.biomes.iter().find(|x| x.0 == b).map_or([200; 3], |x| x.2),
            Target::Water(_) => [64, 120, 230],
            Target::Height { .. } => [255; 3],
            _ => self.layers.get(self.layer).map_or([120, 200, 90], |l| l.color),
        }
    }

    /// The picture of what the image paints, coloured as the target.
    fn preview_texture(&mut self, ctx: &egui::Context) -> egui::TextureId {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}{:?}{:?}", self.target(), self.mapping, self.tint()).hash(&mut h);
        let key = h.finish();
        if let Some((tex, k)) = &self.preview
            && *k == key
        {
            return tex.id();
        }
        let p = &self.info.preview;
        let target = self.target();
        let tint = self.tint();
        let pixels: Vec<Color32> = (0..p.width * p.height)
            .map(|k| {
                let (v, c) = massif_core::mask::preview_strength(&self.info, &self.mapping, target, k);
                if v.is_nan() {
                    return Color32::TRANSPARENT;
                }
                if let Target::Height { .. } = target {
                    let g = (v * 255.0) as u8;
                    return Color32::from_rgba_unmultiplied(g, g, g, 230);
                }
                let rgb = if c > 0 { ANNOTATION_COLORS[(c - 1) as usize] } else { tint };
                let a = (v.clamp(0.0, 1.0) * 230.0) as u8;
                Color32::from_rgba_unmultiplied(rgb[0], rgb[1], rgb[2], a)
            })
            .collect();
        let image = egui::ColorImage::new([p.width, p.height], pixels);
        let tex = ctx.load_texture("import mask preview", image, egui::TextureOptions::NEAREST);
        let id = tex.id();
        self.preview = Some((tex, key));
        id
    }

    pub fn ui(&mut self, ui: &mut Ui) -> Choice {
        let pal = Palette::of(ui);
        let mut choice = Choice::Keep;
        ui.set_width(if ui.ctx().content_rect().width() >= 1000.0 { 896.0 } else { 560.0 });
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::ImageUp).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Import Mask")).size(16.0).bold());
        });
        let h = &self.info.header;
        let name = self.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
        let what = if self.info.colour {
            tr!("colour")
        } else if h.float {
            tr!("grey, floating point")
        } else {
            tr!("grey")
        };
        let about = tr!("{name}: {w} × {h} pixels, {bits}-bit {what} {format}", name, w = h.width, h = h.height, bits = h.bits, what, format = h.format);
        ui.add(egui::Label::new(RichText::new(about).size(12.0).color(pal.weak)).wrap());
        ui.add_space(10.0);

        // Two columns where the window has room: what and how on the left,
        // where on the right; else one under the other.
        let wide = ui.ctx().content_rect().width() >= 1000.0;
        let max_height = ui.ctx().content_rect().height() * if wide { 0.72 } else { 0.62 };
        egui::ScrollArea::vertical().max_height(max_height).show(ui, |ui| {
            if wide {
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(440.0);
                        self.target_ui(ui);
                        ui.add_space(10.0);
                        self.values_ui(ui);
                    });
                    ui.add_space(16.0);
                    ui.vertical(|ui| {
                        ui.set_width(440.0);
                        self.placement_ui(ui);
                        ui.add_space(10.0);
                        self.guide_ui(ui);
                    });
                });
            } else {
                self.target_ui(ui);
                ui.add_space(10.0);
                self.placement_ui(ui);
                ui.add_space(10.0);
                self.values_ui(ui);
                ui.add_space(10.0);
                self.guide_ui(ui);
            }
        });

        ui.add_space(14.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let ok = !matches!(self.kind, Kind::Layer) || !self.layers.is_empty();
            if ui.add_enabled(ok, theme::primary_button(ui, tr!("Import"))).clicked() {
                choice = Choice::Import(self.settings());
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
        });
        choice
    }

    fn guide_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("Keep")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, tr!("Show the image on the map as a guide"), |ui| {
                switch(ui, &mut self.guide);
            });
            if self.guide {
                row(ui, tr!("Opacity"), |ui| {
                    let mut percent = (self.opacity * 100.0).round();
                    if ui.add(Field::new(&mut percent, 5.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                        self.opacity = percent / 100.0;
                    }
                });
            }
        });
        ui.add(egui::Label::new(RichText::new(tr!("The guide is only shown on the 2D map, to trace over: it is never saved in the map nor exported.")).size(11.5).color(pal.weak)).wrap());
    }

    fn target_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("What it paints")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let labels: Vec<&str> = Kind::ALL.iter().map(|k| k.label()).collect();
        let on: Vec<bool> = Kind::ALL.iter().map(|&k| k == self.kind).collect();
        if let Some(i) = button_group(ui, "mask kind", &labels, &on) {
            self.kind = Kind::ALL[i];
            // Heights are read whole; the rest on or off from the middle.
            if self.kind == Kind::Height {
                self.mapping.black_is_none = false;
            }
        }
        ui.add_space(6.0);
        well(ui, |ui| match self.kind {
            Kind::Annotations => {
                if self.info.colour {
                    row(ui, tr!("Colours"), |ui| {
                        let labels = [tr!("Nearest of the 15"), tr!("One colour")];
                        if let Some(i) = button_group(ui, "mask colours", &labels, &[self.by_colour, !self.by_colour]) {
                            self.by_colour = i == 0;
                        }
                    });
                }
                if !self.info.colour || !self.by_colour {
                    row(ui, tr!("Colour"), |ui| {
                        for k in (1..=15u16).rev() {
                            let c = ANNOTATION_COLORS[(k - 1) as usize];
                            let (rect, resp) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::click());
                            ui.painter().rect_filled(rect.shrink(1.0), theme::TILE_RADIUS as f32 / 2.0, Color32::from_rgb(c[0], c[1], c[2]));
                            if k == self.colour {
                                ui.painter().rect_stroke(rect, theme::TILE_RADIUS as f32 / 2.0, Stroke::new(1.5, pal.accent), StrokeKind::Outside);
                            }
                            if resp.clicked() {
                                self.colour = k;
                            }
                        }
                    });
                }
            }
            Kind::Layer => {
                row(ui, tr!("Layer"), |ui| {
                    if self.layers.is_empty() {
                        ui.label(RichText::new(tr!("The map has no layers")).color(pal.weak));
                        return;
                    }
                    let current = self.layers.get(self.layer).map_or_else(String::new, |l| l.name.clone());
                    combo("mask layer").selected_text(current).width(260.0).height(320.0).show_ui(ui, |ui| {
                        for (k, l) in self.layers.iter().enumerate() {
                            ui.selectable_value(&mut self.layer, k, &l.name);
                        }
                    });
                });
            }
            Kind::Terrain => {
                row(ui, tr!("Terrain"), |ui| {
                    let current = self.terrains.iter().find(|t| t.0 == self.terrain).map_or_else(String::new, |t| crate::i18n::tr_name(&t.1).to_owned());
                    combo("mask terrain").selected_text(current).width(260.0).height(320.0).show_ui(ui, |ui| {
                        for (id, name, _) in &self.terrains {
                            ui.selectable_value(&mut self.terrain, *id, crate::i18n::tr_name(name));
                        }
                    });
                });
            }
            Kind::Biome => {
                row(ui, tr!("Biome"), |ui| {
                    let current = self.biomes.iter().find(|b| b.0 == self.biome).map_or_else(String::new, |b| crate::i18n::tr_name(&b.1).to_owned());
                    combo("mask biome").selected_text(current).width(260.0).height(320.0).show_ui(ui, |ui| {
                        for (id, name, _) in &self.biomes {
                            ui.selectable_value(&mut self.biome, *id, crate::i18n::tr_name(name));
                        }
                    });
                });
            }
            Kind::Water => {
                row(ui, tr!("Water surface"), |ui| {
                    ui.add(Field::new(&mut self.water, (self.heights.0 + 1)..=(self.heights.1 - 1)).suffix(" y"));
                });
            }
            Kind::Height => {
                let (lo, hi) = (self.heights.0 as f32, self.heights.1 as f32);
                row(ui, tr!("Height of white"), |ui| {
                    ui.add(Field::new(&mut self.white, lo..=hi).max_decimals(1).suffix(" y"));
                });
                row(ui, tr!("Height of black"), |ui| {
                    ui.add(Field::new(&mut self.black, lo..=hi).max_decimals(1).suffix(" y"));
                });
            }
        });
        if self.kind == Kind::Height {
            let note = match self.mode {
                Mode::Add => tr!("Add raises the ground by the height the image gives: set black to 0 to leave it where the image is black."),
                _ => tr!("Black and white give these heights, the greys those between: the scale is their difference, the offset the height of black."),
            };
            ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
        }
    }

    fn placement_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("Where it lands")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let labels = [tr!("At the world's origin"), tr!("Fit to the map")];
        let on = [self.origin == Some(Origin::MapOrigin), self.origin == Some(Origin::Fit)];
        if let Some(i) = button_group(ui, "mask origin", &labels, &on) {
            self.set_origin([Origin::MapOrigin, Origin::Fit][i]);
        }
        ui.add_space(6.0);
        well(ui, |ui| {
            row(ui, tr!("Scale"), |ui| {
                let mut percent = self.scale * 100.0;
                if ui.add(Field::new(&mut percent, 1.0..=10_000.0).max_decimals(1).suffix(" %").logarithmic(true)).changed() {
                    self.scale = (percent / 100.0).max(0.01);
                    self.origin = None;
                }
            });
            row(ui, tr!("Offset"), |ui| {
                if ui.add(Field::new(&mut self.z, -30_000_000..=30_000_000).text("z ")).changed() {
                    self.origin = None;
                }
                if ui.add(Field::new(&mut self.x, -30_000_000..=30_000_000).text("x ")).changed() {
                    self.origin = None;
                }
            });
        });
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        let [x0, z0, x1, z1] = self.bounds;
        let size = tr!("It covers {w} × {d} blocks, from x {x}, z {z}; the map is {mw} × {md}.", w = r.width(), d = r.depth(), x = r.x0, z = r.z0, mw = x1 - x0, md = z1 - z0);
        ui.add(egui::Label::new(RichText::new(size).size(11.5).color(pal.weak)).wrap());
        ui.add_space(6.0);
        self.picture(ui, r);
    }

    /// The map, and the image where it lands.
    fn picture(&mut self, ui: &mut Ui, r: massif_core::BlockRect) {
        let pal = Palette::of(ui);
        let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::GROUP_RADIUS, pal.well());
        painter.rect_stroke(rect, theme::GROUP_RADIUS, Stroke::new(1.0, pal.border), StrokeKind::Inside);
        let [x0, z0, x1, z1] = self.bounds;
        // What is seen: the map and the image, with a margin.
        let (wx0, wz0) = (x0.min(r.x0) as f32, z0.min(r.z0) as f32);
        let (wx1, wz1) = (x1.max(r.x1) as f32, z1.max(r.z1) as f32);
        let inner = rect.shrink(12.0);
        let s = (inner.width() / (wx1 - wx0).max(1.0)).min(inner.height() / (wz1 - wz0).max(1.0));
        let origin = inner.center() - vec2((wx1 - wx0) * s, (wz1 - wz0) * s) / 2.0;
        let at = |x: f32, z: f32| pos2(origin.x + (x - wx0) * s, origin.y + (z - wz0) * s);
        let map = Rect::from_min_max(at(x0 as f32, z0 as f32), at(x1 as f32, z1 as f32));
        if let Some((image, blocks)) = self.map_picture.take() {
            self.map_texture = Some(ui.ctx().load_texture("import mask map", image, egui::TextureOptions::LINEAR));
            self.map_picture = None;
            let _ = blocks;
        }
        match &self.map_texture {
            Some(tex) => {
                painter.image(tex.id(), map, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            }
            None => {
                painter.rect_filled(map, 0.0, theme::scale(pal.border, 0.6));
            }
        }
        painter.rect_stroke(map, 0.0, Stroke::new(1.0, theme::scale(pal.border, 0.85)), StrokeKind::Outside);
        let img = Rect::from_min_max(at(r.x0 as f32, r.z0 as f32), at(r.x1 as f32, r.z1 as f32));
        let tex = self.preview_texture(ui.ctx());
        painter.image(tex, img, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        painter.rect_stroke(img, 0.0, Stroke::new(1.5, pal.accent), StrokeKind::Outside);
    }

    fn values_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("How its values are read")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let colours = self.kind == Kind::Annotations && self.info.colour && self.by_colour;
        well(ui, |ui| {
            if self.kind != Kind::Height && !colours {
                row(ui, tr!("Values"), |ui| {
                    let labels = [tr!("On from a threshold"), tr!("Strength, scattered")];
                    let on = [self.mapping.threshold.is_some(), self.mapping.threshold.is_none()];
                    if let Some(i) = button_group(ui, "mask values", &labels, &on) {
                        self.mapping.threshold = (i == 0).then_some(0.5);
                    }
                });
                if let Some(t) = &mut self.mapping.threshold {
                    row(ui, tr!("Threshold"), |ui| {
                        let mut percent = (*t * 100.0).round();
                        if ui.add(Field::new(&mut percent, 1.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                            *t = percent / 100.0;
                        }
                    });
                }
            }
            if !colours {
                row(ui, tr!("Range"), |ui| {
                    let labels = [tr!("Black to white"), tr!("The image's lowest to highest")];
                    if let Some(i) = button_group(ui, "mask range", &labels, &[self.mapping.full_range, !self.mapping.full_range]) {
                        self.mapping.full_range = i == 0;
                    }
                });
                row(ui, tr!("Invert"), |ui| {
                    switch(ui, &mut self.mapping.invert);
                });
            }
            row(ui, tr!("Black leaves the map as it is"), |ui| {
                switch(ui, &mut self.mapping.black_is_none);
            });
            row(ui, tr!("With what is there"), |ui| {
                let labels = [tr!("Replace"), tr!("Add"), tr!("Max"), tr!("Min")];
                let on: Vec<bool> = Mode::ALL.iter().map(|&m| m == self.mode).collect();
                if let Some(i) = button_group(ui, "mask mode", &labels, &on) {
                    self.mode = Mode::ALL[i];
                }
            });
        });
        let note = match (self.kind, self.mode) {
            (Kind::Height, Mode::Replace) => tr!("Replace: the ground takes the image's height."),
            (Kind::Height, Mode::Add) => tr!("Add: the image's height is added to the ground's."),
            (Kind::Height, Mode::Max) => tr!("Max: the ground is raised to the image's height, never lowered."),
            (Kind::Height, Mode::Min) => tr!("Min: the ground is lowered to the image's height, never raised."),
            (Kind::Terrain, Mode::Replace) => tr!("Replace: the terrain goes where the image is on; elsewhere the terrain stays, as a column always has one."),
            (_, Mode::Replace) => tr!("Replace: where the image lies, it alone decides; what was painted where it is off goes."),
            (_, Mode::Add | Mode::Max) => tr!("Add and Max: painted where the image is on, and kept where it was painted already."),
            (_, Mode::Min) => tr!("Min: kept only where it was painted already and the image is on."),
        };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
        if self.kind != Kind::Height && !colours && self.mapping.threshold.is_none() {
            ui.add(egui::Label::new(RichText::new(tr!("Strength: a grey of 30 % paints about 30 % of the columns, scattered as a spray does.")).size(11.5).color(pal.weak)).wrap());
        }
    }
}

/// The image kept on the 2D map to trace over: never saved nor exported.
pub struct Guide {
    pub name: String,
    pub texture: egui::TextureHandle,
    /// The blocks it covers: west, north, east, south.
    pub rect: [i32; 4],
    pub opacity: f32,
    pub shown: bool,
}

impl Guide {
    pub fn new(ctx: &egui::Context, form: &MaskForm) -> Self {
        let g = &form.info.guide;
        let pixels: Vec<Color32> = g.grey.iter().zip(&g.rgb).map(|(v, c)| if v.is_nan() { Color32::TRANSPARENT } else { Color32::from_rgb(c[0], c[1], c[2]) }).collect();
        let image = egui::ColorImage::new([g.width, g.height], pixels);
        let texture = ctx.load_texture("import mask guide", image, egui::TextureOptions::LINEAR);
        let r = form.placement().rect(form.info.header.width, form.info.header.height);
        Self {
            name: form.path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
            texture,
            rect: [r.x0, r.z0, r.x1, r.z1],
            opacity: form.opacity,
            shown: true,
        }
    }

    /// Its button in the map's header: show it or not, how strongly, or
    /// let it go. False once let go.
    pub fn header(&mut self, ui: &mut Ui) -> bool {
        let mut keep = true;
        let icon = theme::glyph(Icon::Image).to_string();
        ui.menu_button(icon, |ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new(&self.name).medium());
            ui.add_space(4.0);
            well(ui, |ui| {
                row(ui, tr!("Show the guide"), |ui| {
                    switch(ui, &mut self.shown);
                });
                row(ui, tr!("Opacity"), |ui| {
                    let mut percent = (self.opacity * 100.0).round();
                    if ui.add(Field::new(&mut percent, 5.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                        self.opacity = percent / 100.0;
                    }
                });
            });
            ui.add_space(4.0);
            if ui.button(theme::with_icon(Icon::Trash2, tr!("Remove the guide"))).clicked() {
                keep = false;
                ui.close();
            }
        })
        .response
        .on_hover_text(tr!("The imported image shown on the map to trace over: never exported"));
        keep
    }
}
