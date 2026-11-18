//! Edit ▸ Import mask: an image painted onto the map, as WorldPainter's
//! Import mask. The image is read once in the background for its range
//! and a picture of it (`massif_core::mask::survey`); the dialog then says
//! what it paints (annotations, a layer, a terrain, a biome, water, the
//! height), where it lands, how its values are read and how it goes with
//! what is painted, with a picture of where it lands; the import runs in
//! the background, as one step to undo. The image may stay on the 2D map
//! as a reference image, where it landed, to see under what it painted.
//!
//! Several images can go in at once, each with what it paints and how,
//! all in the same place, as one step to undo: the heightmap and masks of
//! a region of the map worked on in another tool. Images of a region
//! written by File ▸ Export Region… come back by themselves: the
//! `region.txt` beside them places them, and their names say what they
//! paint (`massif_core::mask::named`).

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, combo, row, switch, well};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::mask::{Columns, ImageInfo, Mapping, MaskSettings, Mode, Named, Placement, Region, Target};
use massif_core::registry::Registry;
use massif_core::tile::ANNOTATION_COLORS;
use std::path::PathBuf;
use std::sync::Arc;

pub(crate) mod window;

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
    AtZero,
    /// As large as the map lets it, in its middle.
    Fit,
    /// Where its `region.txt` says.
    Region,
}

/// One image's own settings, while another one is shown.
#[derive(Clone)]
struct Entry {
    path: PathBuf,
    info: Arc<ImageInfo>,
    kind: Kind,
    by_colour: bool,
    colour: u16,
    layer: usize,
    terrain: u16,
    biome: u16,
    water: i32,
    black: f32,
    white: f32,
    mapping: Mapping,
    mode: Mode,
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
    /// Keep the image on the map as a reference image, at this opacity.
    pub reference: bool,
    pub opacity: f32,
    preview: Option<(egui::TextureHandle, u64)>,
    /// Every image, the one shown (`current`) as it was when last left:
    /// its settings are the form's own while it is shown.
    entries: Vec<Entry>,
    current: usize,
    /// Where the images' `region.txt` places them, if they have one.
    region: Option<Region>,
    /// Blocks over which a heightmap fades into the land around it.
    pub feather: u32,
    /// The map's columns, for what the import makes of them.
    columns: Option<Arc<Columns>>,
    /// Shown as the mask window (`None`: as the window has room for).
    pub(crate) big: Option<bool>,
    studio: window::Studio,
    /// The image alone, as its values are read.
    mask_preview: Option<(egui::TextureHandle, u64)>,
    /// The zone the images came from (its `zone.json`, of this map), and
    /// the index of the version it was exported from, when the map still
    /// has it.
    pub zone: Option<(massif_core::patch::Zone, Option<usize>)>,
    /// Review the images as a contribution against that version, rather
    /// than paint them at once.
    pub review: bool,
}

/// What the dialog wants done.
pub enum Choice {
    Keep,
    Close,
    /// More images to read and add.
    AddImages,
    Import(Vec<(PathBuf, Arc<ImageInfo>, MaskSettings)>),
}

impl MaskForm {
    /// The dialog for the image at `path`, surveyed as `info`, onto a map of
    /// `bounds` (blocks), its picture `map`.
    pub fn new(images: Vec<(PathBuf, Arc<ImageInfo>)>, reg: &Registry, bounds: [i32; 4], heights: (i32, i32), sea: i32, map: Option<(egui::ColorImage, [i32; 4])>, columns: Option<Arc<Columns>>) -> Self {
        let mut images = images.into_iter();
        let (path, info) = images.next().expect("an image at least");
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
            reference: true,
            opacity: 0.5,
            preview: None,
            entries: Vec::new(),
            current: 0,
            region: None,
            feather: 16,
            columns,
            big: None,
            studio: window::Studio::default(),
            mask_preview: None,
            zone: None,
            review: false,
        };
        let first = form.stash();
        let first = form.guessed(first, reg);
        form.load(first.clone());
        form.entries.push(first);
        // An image of the map's size lands on it; another one at the origin.
        let (w, h) = (form.info.header.width as i32, form.info.header.height as i32);
        if form.region.is_some() {
            form.set_origin(Origin::Region);
        } else if (w, h) == (bounds[2] - bounds[0], bounds[3] - bounds[1]) {
            form.set_origin(Origin::Fit);
        } else {
            form.set_origin(Origin::AtZero);
        }
        form.add(images.collect(), reg);
        form
    }

    /// The form's settings as an entry.
    fn stash(&self) -> Entry {
        Entry {
            path: self.path.clone(),
            info: self.info.clone(),
            kind: self.kind,
            by_colour: self.by_colour,
            colour: self.colour,
            layer: self.layer,
            terrain: self.terrain,
            biome: self.biome,
            water: self.water,
            black: self.black,
            white: self.white,
            mapping: self.mapping,
            mode: self.mode,
        }
    }

    fn load(&mut self, e: Entry) {
        self.path = e.path;
        self.info = e.info;
        self.kind = e.kind;
        self.by_colour = e.by_colour;
        self.colour = e.colour;
        self.layer = e.layer;
        self.terrain = e.terrain;
        self.biome = e.biome;
        self.water = e.water;
        self.black = e.black;
        self.white = e.white;
        self.mapping = e.mapping;
        self.mode = e.mode;
        self.preview = None;
        self.mask_preview = None;
        self.studio.histogram = None;
        self.studio.colours = None;
    }

    /// `e` with what its name and its `region.txt` say: what it paints,
    /// and the heights of its black and white.
    fn guessed(&mut self, mut e: Entry, reg: &Registry) -> Entry {
        use massif_core::mask::file_safe;
        let region = Region::beside(&e.path);
        if self.region.is_none() {
            self.region = region;
        }
        let same = |a: &str, b: &str| file_safe(a).to_lowercase() == *b;
        let layer = |target: &dyn Fn(Target) -> bool| self.layers.iter().position(|l| target(l.target));
        match massif_core::mask::named(&e.path) {
            Some(Named::Height) => e.kind = Kind::Height,
            Some(Named::Annotations) => e.kind = Kind::Annotations,
            Some(Named::Water) => e.kind = Kind::Water,
            Some(Named::Terrain(name)) => {
                if let Some(k) = reg.terrains.iter().position(|t| same(&t.name, &name)) {
                    (e.kind, e.terrain) = (Kind::Terrain, k as u16);
                }
            }
            Some(Named::Biome(name)) => {
                if let Some(k) = reg.biomes.iter().position(|b| same(&b.name, &name)) {
                    (e.kind, e.biome) = (Kind::Biome, k as u16);
                }
            }
            Some(named) => {
                let found = match &named {
                    Named::Cover(n) => layer(&|t| matches!(t, Target::Cover(k) if reg.cover(k).is_some_and(|c| same(&c.name, n)))),
                    Named::Trees(n) => layer(&|t| matches!(t, Target::Trees(k) if reg.object_layer(k).is_some_and(|c| same(&c.name, n)))),
                    Named::Understory(n) => layer(&|t| matches!(t, Target::Understory(k) if reg.object_layer(k).is_some_and(|c| same(&c.name, n)))),
                    Named::Pockets(n) => layer(&|t| matches!(t, Target::Pockets(k) if reg.pocket(k).is_some_and(|c| same(&c.name, n)))),
                    Named::TreeSize(n) => layer(&|t| t == Target::TreeSize(*n)),
                    _ => None,
                };
                if let Some(k) = found {
                    (e.kind, e.layer) = (Kind::Layer, k);
                }
            }
            None => {}
        }
        // What a region wrote is read back as it was written: masks white
        // where they are, the heights from black to white.
        if region.is_some() || massif_core::mask::named(&e.path).is_some() {
            e.mapping = Mapping { full_range: true, invert: false, black_is_none: false, threshold: Some(0.5), ..Mapping::default() };
            if e.kind == Kind::Annotations {
                e.mapping.black_is_none = true;
            }
        }
        if e.kind == Kind::Height {
            e.mapping.black_is_none = false;
            e.mapping.threshold = None;
            if let Some(r) = region {
                (e.black, e.white) = (r.black, r.white);
            }
        }
        e
    }

    /// Add images, each with what its name says it paints.
    pub fn add(&mut self, images: Vec<(PathBuf, Arc<ImageInfo>)>, reg: &Registry) {
        let had_region = self.region.is_some();
        let base = self.stash();
        for (path, info) in images {
            let mut e = base.clone();
            e.kind = if info.colour { Kind::Annotations } else { Kind::Layer };
            e.mapping = Mapping {
                full_range: !info.header.float,
                threshold: if info.header.bits <= 8 && !info.header.float { Some(0.5) } else { None },
                ..Mapping::default()
            };
            (e.path, e.info) = (path, info);
            let e = self.guessed(e, reg);
            self.entries.push(e);
        }
        if !had_region && self.region.is_some() {
            self.set_origin(Origin::Region);
        }
    }

    /// Show image `k`.
    fn show(&mut self, k: usize) {
        if k == self.current || k >= self.entries.len() {
            return;
        }
        self.entries[self.current] = self.stash();
        self.current = k;
        self.load(self.entries[k].clone());
    }

    /// Every image with its settings, the one shown as the form has it.
    pub fn all(&self) -> Vec<(PathBuf, Arc<ImageInfo>, MaskSettings)> {
        let mut entries = self.entries.clone();
        entries[self.current] = self.stash();
        entries
            .into_iter()
            .map(|e| {
                let mut form = MaskSettings {
                    target: self.target_of(&e),
                    mapping: e.mapping,
                    mode: e.mode,
                    placement: self.placement(),
                    feather: self.feather,
                };
                if !matches!(form.target, Target::Height { .. }) {
                    form.feather = 0;
                }
                (e.path, e.info, form)
            })
            .collect()
    }


    fn set_origin(&mut self, origin: Origin) {
        self.origin = Some(origin);
        let (w, h) = (self.info.header.width as f64, self.info.header.height as f64);
        match origin {
            Origin::Region => match self.region {
                Some(r) => {
                    let p = r.placement(self.info.header.width);
                    (self.x, self.z, self.scale) = (p.x, p.z, p.scale);
                }
                None => self.origin = None,
            },
            Origin::AtZero => {
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
        self.target_of(&self.stash())
    }

    fn target_of(&self, e: &Entry) -> Target {
        match e.kind {
            Kind::Annotations => Target::Annotations((!e.by_colour || !e.info.colour).then_some(e.colour)),
            Kind::Layer => self.layers.get(e.layer).map_or(Target::TreeSize(1), |l| l.target),
            Kind::Terrain => Target::Terrain(e.terrain),
            Kind::Biome => Target::Biome(e.biome),
            Kind::Water => Target::Water(e.water.clamp(i16::MIN as i32 + 1, i16::MAX as i32) as i16),
            Kind::Height => Target::Height { black: e.black, white: e.white },
        }
    }

    pub fn placement(&self) -> Placement {
        Placement { x: self.x, z: self.z, scale: self.scale }
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
        let mut choice = Choice::Keep;
        // The mask window where the window has room for it.
        let room = ui.ctx().content_rect();
        let big = *self.big.get_or_insert(room.width() >= 1100.0 && room.height() >= 640.0);
        if big {
            self.window_ui(ui, &mut choice);
            return choice;
        }
        let pal = Palette::of(ui);
        ui.set_width(if room.width() >= 1000.0 { 896.0 } else { 560.0 });
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::ImageUp).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Import Mask")).size(16.0).bold());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(theme::with_icon(Icon::Maximize2, tr!("Mask window"))).on_hover_text(tr!("The map large, the mask over it: before and after, levels, legend")).clicked() {
                    self.big = Some(true);
                }
            });
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
        ui.add_space(8.0);
        self.images_ui(ui, &mut choice);

        // Two columns where the window has room: what and how on the left,
        // where on the right; else one under the other.
        let wide = room.width() >= 1000.0;
        let max_height = room.height() * if wide { 0.72 } else { 0.62 };
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
                        self.reference_ui(ui);
                    });
                });
            } else {
                self.target_ui(ui);
                ui.add_space(10.0);
                self.placement_ui(ui);
                ui.add_space(10.0);
                self.values_ui(ui);
                ui.add_space(10.0);
                self.reference_ui(ui);
            }
        });

        ui.add_space(14.0);
        self.zone_ui(ui);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            self.buttons(ui, &mut choice);
        });
        choice
    }

    /// The images, one shown at a time: all go in, in the same place, as
    /// one step to undo; and Add images….
    fn images_ui(&mut self, ui: &mut Ui, choice: &mut Choice) {
        let mut remove = None;
        ui.horizontal_wrapped(|ui| {
            if self.entries.len() > 1 {
                let names: Vec<String> = self
                    .entries
                    .iter()
                    .enumerate()
                    .map(|(k, e)| {
                        let path = if k == self.current { &self.path } else { &e.path };
                        let name = path.file_stem().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                        if name.chars().count() > 22 { format!("{}…", name.chars().take(21).collect::<String>()) } else { name }
                    })
                    .collect();
                let labels: Vec<&str> = names.iter().map(String::as_str).collect();
                let on: Vec<bool> = (0..names.len()).map(|k| k == self.current).collect();
                if let Some(k) = button_group(ui, "mask images", &labels, &on) {
                    self.show(k);
                }
                if ui.button(theme::glyph(Icon::X).to_string()).on_hover_text(tr!("Leave this image out")).clicked() {
                    remove = Some(self.current);
                }
            }
            if ui.button(theme::with_icon(Icon::Plus, tr!("Add images…"))).clicked() {
                *choice = Choice::AddImages;
            }
        });
        if let Some(k) = remove {
            self.entries.remove(k);
            self.current = k.min(self.entries.len() - 1);
            self.load(self.entries[self.current].clone());
        }
        ui.add_space(10.0);
    }

    /// For the images of a zone of this map exported to come back: review
    /// them as a contribution against the version it left from.
    fn zone_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        let Some((zone, base)) = &self.zone else { return };
        well(ui, |ui| {
            row(ui, tr!("Review it as a contribution"), |ui| {
                if base.is_some() {
                    switch(ui, &mut self.review);
                } else {
                    self.review = false;
                    crate::collab::status_pill(ui, tr!("Version gone"), pal.weak);
                }
            });
        });
        let note = match (base, self.review) {
            (Some(_), true) => tr!("The images are painted onto “{base}”, the version the zone was exported from, and what they change is shown on the map to accept or reject: nothing changes until you apply it.", base = zone.base.name),
            (Some(_), false) => tr!("The images are painted onto the map at once, as one step to undo.").to_owned(),
            (None, _) => tr!("“{base}”, the version the zone was exported from, is no longer in the map: the images are painted onto the map at once.", base = zone.base.name),
        };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
        ui.add_space(10.0);
    }

    /// Import and Cancel, from the right.
    fn buttons(&mut self, ui: &mut Ui, choice: &mut Choice) {
        let ok = !matches!(self.kind, Kind::Layer) || !self.layers.is_empty();
        let label = if self.review {
            tr!("Review…").to_owned()
        } else if self.entries.len() > 1 {
            tr!("Import {n} images", n = self.entries.len())
        } else {
            tr!("Import").to_owned()
        };
        if ui.add_enabled(ok, theme::primary_button(ui, label)).clicked() {
            *choice = Choice::Import(self.all());
        }
        if ui.button(tr!("Cancel")).clicked() {
            *choice = Choice::Close;
        }
    }

    fn reference_ui(&mut self, ui: &mut Ui) {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("Keep")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        well(ui, |ui| {
            row(ui, tr!("Keep the image as a reference image"), |ui| {
                switch(ui, &mut self.reference);
            });
            if self.reference {
                row(ui, tr!("Opacity"), |ui| {
                    let mut percent = (self.opacity * 100.0).round();
                    if ui.add(Field::new(&mut percent, 5.0..=100.0).max_decimals(0).suffix(" %")).changed() {
                        self.opacity = percent / 100.0;
                    }
                });
            }
        });
        let note = if self.entries.len() > 1 {
            tr!("The image shown stays on the 2D map where it lands, to see under what it painted: kept with the map, never exported. The map's Reference images button shows, moves or removes it.")
        } else {
            tr!("The image stays on the 2D map where it lands, to see under what it painted: kept with the map, never exported. The map's Reference images button shows, moves or removes it.")
        };
        ui.add(egui::Label::new(RichText::new(note).size(11.5).color(pal.weak)).wrap());
    }

    /// The image shown as a reference image, exactly where it lands.
    pub fn reference_image(&self) -> massif_core::reference::ReferenceImage {
        let (w, h) = (self.info.header.width, self.info.header.height);
        let p = self.placement();
        massif_core::reference::ReferenceImage {
            path: self.path.to_string_lossy().into_owned(),
            size: [w as u32, h as u32],
            x: p.x as f64 + w as f64 * p.scale as f64 / 2.0,
            z: p.z as f64 + h as f64 * p.scale as f64 / 2.0,
            scale: p.scale,
            rotation: 0.0,
            opacity: self.opacity,
            ..Default::default()
        }
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
        let r = self.placement_fields(ui);
        ui.add_space(6.0);
        self.picture(ui, r);
        ui.add(egui::Label::new(RichText::new(tr!("Drag the image on the map to move it; the arrow keys move it a block, with Shift 16.")).size(11.5).color(pal.weak)).wrap());
    }

    /// Where it lands: the origins, the scale and the offset, and what it
    /// covers. The blocks it covers.
    fn placement_fields(&mut self, ui: &mut Ui) -> massif_core::BlockRect {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("Where it lands")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let mut origins = vec![(Origin::AtZero, tr!("At the world's origin")), (Origin::Fit, tr!("Fit to the map"))];
        if self.region.is_some() {
            origins.push((Origin::Region, tr!("Where region.txt says")));
        }
        let labels: Vec<&str> = origins.iter().map(|o| o.1).collect();
        let on: Vec<bool> = origins.iter().map(|o| self.origin == Some(o.0)).collect();
        if let Some(i) = button_group(ui, "mask origin", &labels, &on) {
            self.set_origin(origins[i].0);
        }
        ui.add_space(6.0);
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        let [x0, z0, x1, z1] = self.bounds;
        well(ui, |ui| {
            row(ui, tr!("Scale"), |ui| {
                // Around the scale that fits the map and a block a pixel,
                // spread in ratios; any scale typed in.
                let (lo, hi) = scale_range(self.fit_scale(), self.scale);
                let mut percent = self.scale * 100.0;
                if ui.add(Field::new(&mut percent, lo * 100.0..=hi * 100.0).max_decimals(1).suffix(" %").logarithmic(true).clamping(egui::SliderClamping::Never)).changed() {
                    self.set_scale(percent / 100.0);
                }
            });
            row(ui, tr!("Offset"), |ui| {
                // From where the image just touches the map's west (north)
                // edge to where it touches its east (south) one: a block a
                // step, any offset typed in.
                let width = ((ui.available_width() - 64.0) / 2.0 - ui.spacing().item_spacing.x).max(90.0);
                let (lo, hi) = offset_range(z0, z1, r.depth(), self.z);
                if ui.add(Field::new(&mut self.z, lo..=hi).text("z ").width(width).clamping(egui::SliderClamping::Never)).changed() {
                    self.origin = None;
                }
                let (lo, hi) = offset_range(x0, x1, r.width(), self.x);
                if ui.add(Field::new(&mut self.x, lo..=hi).text("x ").width(width).clamping(egui::SliderClamping::Never)).changed() {
                    self.origin = None;
                }
            });
            if self.entries.iter().enumerate().any(|(k, e)| if k == self.current { self.kind == Kind::Height } else { e.kind == Kind::Height }) {
                row(ui, tr!("Blend the heights' edges over"), |ui| {
                    ui.add(Field::new(&mut self.feather, 0..=512).suffix(tr!(" blocks")));
                });
            }
        });
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        let size = tr!("It covers {w} × {d} blocks, from x {x}, z {z}; the map is {mw} × {md}.", w = r.width(), d = r.depth(), x = r.x0, z = r.z0, mw = x1 - x0, md = z1 - z0);
        ui.add(egui::Label::new(RichText::new(size).size(11.5).color(pal.weak)).wrap());
        r
    }

    /// The scale at which the image fits the map.
    fn fit_scale(&self) -> f32 {
        let [x0, z0, x1, z1] = self.bounds;
        let (w, h) = (self.info.header.width.max(1) as f64, self.info.header.height.max(1) as f64);
        (((x1 - x0) as f64 / w).min((z1 - z0) as f64 / h)) as f32
    }

    /// Scale the image about its middle, which stays where it is.
    pub(crate) fn set_scale(&mut self, scale: f32) {
        let scale = scale.clamp(0.01, 1000.0);
        let (w, h) = (self.info.header.width as f64, self.info.header.height as f64);
        let (cx, cz) = (self.x as f64 + w * self.scale as f64 / 2.0, self.z as f64 + h * self.scale as f64 / 2.0);
        self.scale = scale;
        self.x = (cx - w * scale as f64 / 2.0).round() as i32;
        self.z = (cz - h * scale as f64 / 2.0).round() as i32;
        self.origin = None;
    }

    /// Move the image by whole blocks.
    pub(crate) fn nudge(&mut self, dx: i32, dz: i32) {
        if (dx, dz) != (0, 0) {
            self.x = self.x.saturating_add(dx);
            self.z = self.z.saturating_add(dz);
            self.origin = None;
        }
    }

    /// The map, and the image where it lands: dragged, it moves; the arrow
    /// keys move it a block (Shift: 16) while the picture has the focus or
    /// the pointer.
    fn picture(&mut self, ui: &mut Ui, r: massif_core::BlockRect) {
        let pal = Palette::of(ui);
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 220.0), Sense::click_and_drag());
        let id = response.id;
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, theme::GROUP_RADIUS, pal.well());
        let focused = response.has_focus();
        painter.rect_stroke(rect, theme::GROUP_RADIUS, Stroke::new(1.0, if focused { pal.accent } else { pal.border }), StrokeKind::Inside);
        let [x0, z0, x1, z1] = self.bounds;
        // What is seen: the map and the image, with a margin; held still
        // while the image is dragged, so that it follows the pointer.
        let fresh = [x0.min(r.x0) as f32, z0.min(r.z0) as f32, x1.max(r.x1) as f32, z1.max(r.z1) as f32];
        let held: Option<[f32; 4]> = ui.data(|d| d.get_temp(id.with("view")));
        let [wx0, wz0, wx1, wz1] = if response.dragged() { held.unwrap_or(fresh) } else { fresh };
        let inner = rect.shrink(12.0);
        let s = (inner.width() / (wx1 - wx0).max(1.0)).min(inner.height() / (wz1 - wz0).max(1.0));
        let origin = inner.center() - vec2((wx1 - wx0) * s, (wz1 - wz0) * s) / 2.0;
        let at = |x: f32, z: f32| pos2(origin.x + (x - wx0) * s, origin.y + (z - wz0) * s);
        if response.drag_started() {
            ui.data_mut(|d| {
                d.insert_temp(id.with("view"), [wx0, wz0, wx1, wz1]);
                d.insert_temp(id.with("start"), (self.x, self.z, egui::Vec2::ZERO));
            });
            response.request_focus();
        }
        if response.clicked() {
            response.request_focus();
        }
        if response.dragged()
            && let Some((sx, sz, moved)) = ui.data(|d| d.get_temp::<(i32, i32, egui::Vec2)>(id.with("start")))
        {
            let moved = moved + response.drag_delta();
            ui.data_mut(|d| d.insert_temp(id.with("start"), (sx, sz, moved)));
            let (dx, dz) = drag_blocks(moved, s);
            self.nudge(sx + dx - self.x, sz + dz - self.z);
        }
        if response.drag_stopped() {
            ui.data_mut(|d| {
                d.remove::<[f32; 4]>(id.with("view"));
                d.remove::<(i32, i32, egui::Vec2)>(id.with("start"));
            });
        }
        if focused || response.hovered() {
            let (dx, dz) = ui.input(|i| {
                let step = if i.modifiers.shift { 16 } else { 1 };
                let k = |key| i.key_pressed(key) as i32 * step;
                (k(egui::Key::ArrowRight) - k(egui::Key::ArrowLeft), k(egui::Key::ArrowDown) - k(egui::Key::ArrowUp))
            });
            self.nudge(dx, dz);
        }
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(if response.dragged() { egui::CursorIcon::Grabbing } else { egui::CursorIcon::Grab });
        }
        let r = self.placement().rect(self.info.header.width, self.info.header.height);
        let map = Rect::from_min_max(at(x0 as f32, z0 as f32), at(x1 as f32, z1 as f32));
        if let Some((image, _)) = self.map_picture.take() {
            self.map_texture = Some(ui.ctx().load_texture("import mask map", image, egui::TextureOptions::LINEAR));
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

/// The offsets along one axis at which an image `size` blocks across still
/// meets a map from `lo` to `hi`: from where its far edge touches the map's
/// near edge to where its near edge touches the far one, with a margin of
/// a twentieth of the map (16 blocks at least); widened to `at`, the offset
/// it has, wherever that is.
pub(crate) fn offset_range(lo: i32, hi: i32, size: i32, at: i32) -> (i32, i32) {
    let margin = ((hi - lo) / 20).max(16);
    ((lo - size - margin).min(at), (hi + margin).max(at))
}

/// The scales the slider spans: from an eighth of the scale that fits the
/// map, or a block a pixel if smaller, to four times it, or 100 % if
/// larger; widened to `at`, the scale it has.
pub(crate) fn scale_range(fit: f32, at: f32) -> (f32, f32) {
    let lo = (fit / 8.0).clamp(0.01, 1.0);
    let hi = (fit * 4.0).clamp(1.0, 100.0);
    (lo.min(at), hi.max(at))
}

/// The whole blocks a drag of `moved` points makes on a picture of `s`
/// points a block.
pub(crate) fn drag_blocks(moved: egui::Vec2, s: f32) -> (i32, i32) {
    let s = s.max(1e-6);
    ((moved.x / s).round() as i32, (moved.y / s).round() as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offsets_span_the_map() {
        // A map of 1,024 blocks from -512, an image of 820 across.
        let (lo, hi) = offset_range(-512, 512, 820, 0);
        assert_eq!(lo, -512 - 820 - 51, "its east edge just past the map's west one");
        assert_eq!(hi, 512 + 51, "its west edge just past the map's east one");
        // An image placed far away keeps its offset in the range.
        assert_eq!(offset_range(-512, 512, 820, 40_000), (lo, 40_000));
        assert_eq!(offset_range(-512, 512, 820, -40_000), (-40_000, hi));
        // A small map keeps a margin of 16 blocks.
        assert_eq!(offset_range(0, 64, 10, 0), (-26, 80));
    }

    #[test]
    fn the_scales_span_fit_and_a_block_a_pixel() {
        let (lo, hi) = scale_range(0.25, 0.41);
        assert!(lo <= 0.25 / 8.0 + 1e-6 && hi >= 1.0, "{lo} {hi}");
        let (lo, hi) = scale_range(16.0, 16.0);
        assert!(lo <= 1.0 && (hi - 64.0).abs() < 1e-4, "{lo} {hi}");
        assert_eq!(scale_range(1.0, 300.0).1, 300.0, "a scale typed in stays in the range");
    }

    #[test]
    fn a_drag_moves_by_whole_blocks() {
        // 0.2 points a block: 10 points are 50 blocks.
        assert_eq!(drag_blocks(vec2(10.0, -3.0), 0.2), (50, -15));
        // 4 points a block: 1 point is nothing yet, 3 are a block.
        assert_eq!(drag_blocks(vec2(1.0, 3.0), 4.0), (0, 1));
    }

    /// A form for an image of `w` × `h` pixels onto a map of 1,024 blocks
    /// from -512.
    pub(crate) fn form(w: usize, h: usize) -> MaskForm {
        use massif_core::mask::{Header, Preview};
        let header = Header { width: w, height: h, bits: 8, colour: false, alpha: false, float: false, format: "PNG", lossy: false };
        let preview = Preview { width: 1, height: 1, grey: vec![255.0], rgb: vec![[255; 3]] };
        let info = ImageInfo { header, colour: false, transparent: false, low: 0.0, high: 255.0, white: 255.0, preview: preview.clone(), guide: preview };
        MaskForm::new(vec![(PathBuf::from("mask.png"), Arc::new(info))], &Registry::vanilla(), [-512, -512, 512, 512], (-64, 320), 62, None, None)
    }

    #[test]
    fn scaling_keeps_the_middle_and_nudging_moves_by_blocks() {
        let mut f = form(1000, 800);
        assert_eq!((f.x, f.z, f.scale), (0, 0, 1.0), "an image of another size lands at the origin");
        f.set_scale(0.5);
        let r = f.placement().rect(1000, 800);
        assert_eq!((r.x0, r.z0, r.width(), r.depth()), (250, 200, 500, 400), "about its middle");
        f.nudge(1, -16);
        assert_eq!((f.x, f.z), (251, 184));
        assert!(f.origin.is_none());
        f.set_origin(Origin::Fit);
        let r = f.placement().rect(1000, 800);
        assert_eq!((r.x0, r.width()), (-512, 1024), "fit to the map");
    }
}
