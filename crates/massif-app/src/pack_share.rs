//! Packs shared between users: the library of packs imported from
//! `.massifpack` files (see `massif_core::pack_file`), shown in the Packs
//! panel with their author, and the dialogs to export a pack from the
//! map's layers, to import one whose name is taken, and to remove one.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::{Tool, ToolState};
use egui::{Align, Color32, Rect, RichText, Ui, pos2, vec2};
use massif_core::pack_file::{self, ItemKind, Pack};
use std::path::{Path, PathBuf};

/// A pack of the library.
pub struct Shared {
    /// Its file in the library folder.
    pub file: PathBuf,
    pub pack: Pack,
    preview: Option<egui::TextureHandle>,
}

/// What the Packs panel asks the app to do with the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    Import,
    /// Save a copy of pack `i` to share it.
    Share(usize),
    /// Ask before removing pack `i`.
    Remove(usize),
}

/// The packs imported, kept in a folder of their own.
#[derive(Default)]
pub struct PackLibrary {
    dir: Option<PathBuf>,
    pub packs: Vec<Shared>,
    loaded: bool,
    pub request: Option<Request>,
}

impl std::fmt::Debug for PackLibrary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PackLibrary").field("dir", &self.dir).field("packs", &self.packs.len()).finish()
    }
}

/// Read a pack file, its size checked before it is read.
pub fn read_file(path: &Path) -> Result<Pack, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    if len > pack_file::MAX_FILE_BYTES {
        return Err(pack_file::PackError::TooLarge.to_string());
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Pack::from_bytes(&bytes).map_err(|e| e.to_string())
}

/// Whether a path is a pack file, by its extension.
pub fn is_pack_file(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case(pack_file::EXTENSION))
}

impl PackLibrary {
    /// The library of this installation, in Massif's folder.
    pub fn installed() -> Self {
        Self::at(eframe::storage_dir("massif").map(|d| d.join("packs")))
    }

    pub fn at(dir: Option<PathBuf>) -> Self {
        Self { dir, ..Self::default() }
    }

    /// Read the library folder, the first time. A file that is not a pack
    /// (any more) is left out, and said in the log.
    pub fn ensure_loaded(&mut self) {
        if self.loaded {
            return;
        }
        self.loaded = true;
        let Some(dir) = &self.dir else { return };
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        let mut files: Vec<PathBuf> = entries.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_file() && is_pack_file(p)).collect();
        files.sort();
        for file in files {
            match read_file(&file) {
                Ok(pack) => self.packs.push(Shared { file, pack, preview: None }),
                Err(e) => log::warn!("pack {} left out: {e}", file.display()),
            }
        }
        self.packs.sort_by_key(|s| s.pack.name.to_lowercase());
    }

    /// The pack of that name, whatever its case.
    pub fn find(&self, name: &str) -> Option<usize> {
        self.packs.iter().position(|s| s.pack.name.to_lowercase() == name.to_lowercase())
    }

    /// Add a pack to the library, in place of pack `replace`. It is written
    /// again from what was read, under a file name made from its name: no
    /// name of the file it came from reaches the disk. Returns its place.
    pub fn add(&mut self, pack: Pack, replace: Option<usize>) -> Result<usize, String> {
        let dir = self.dir.clone().ok_or(tr!("Massif has no folder to keep packs in"))?;
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let old = replace.and_then(|i| self.packs.get(i)).map(|s| s.file.clone());
        let stem = pack_file::file_stem(&pack.name);
        let taken = |p: &Path| p.exists() && old.as_deref() != Some(p);
        let file = std::iter::once(dir.join(format!("{stem}.{}", pack_file::EXTENSION)))
            .chain((2..).map(|k| dir.join(format!("{stem} {k}.{}", pack_file::EXTENSION))))
            .find(|p| !taken(p))
            .expect("a free name");
        massif_core::atomic::write(&file, &pack.to_bytes(), true).map_err(|e| massif_core::atomic::describe(&e, &file))?;
        if let Some(i) = replace.filter(|&i| i < self.packs.len()) {
            if self.packs[i].file != file {
                let _ = std::fs::remove_file(&self.packs[i].file);
            }
            self.packs.remove(i);
        }
        let name = pack.name.to_lowercase();
        self.packs.push(Shared { file, pack, preview: None });
        self.packs.sort_by_key(|s| s.pack.name.to_lowercase());
        Ok(self.packs.iter().position(|s| s.pack.name.to_lowercase() == name).unwrap_or(0))
    }

    /// Take pack `i` out of the library, its file deleted. The layers maps
    /// have from it stay in them.
    pub fn remove(&mut self, i: usize) -> Result<Pack, String> {
        let s = self.packs.get(i).ok_or("no such pack")?;
        std::fs::remove_file(&s.file).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }).map_err(|e| format!("{}: {e}", s.file.display()))?;
        Ok(self.packs.remove(i).pack)
    }

    fn preview(&mut self, ctx: &egui::Context, i: usize) -> Option<egui::TextureHandle> {
        let s = self.packs.get_mut(i)?;
        if s.preview.is_none() {
            let png = s.pack.preview.as_ref()?;
            let img = image::load_from_memory_with_format(png, image::ImageFormat::Png).ok()?.to_rgba8();
            let size = [img.width() as usize, img.height() as usize];
            let image = egui::ColorImage::from_rgba_unmultiplied(size, img.as_raw());
            s.preview = Some(ctx.load_texture(format!("pack preview {}", s.file.display()), image, egui::TextureOptions::LINEAR));
        }
        s.preview.clone()
    }
}

/// A dialog about packs.
pub enum PackDialog {
    Export(Box<ExportForm>),
    /// A pack read whose name is one of the library's.
    Clash { pack: Box<Pack>, existing: usize, rename: String },
    Remove(usize),
}

/// What Export Pack asks.
pub struct ExportForm {
    pub name: String,
    pub author: String,
    pub description: String,
    /// A picture chosen (a PNG); otherwise one is made from what it holds.
    pub picture: Option<Vec<u8>>,
    /// What the map has to put in it: kind, number, name, whether it goes.
    pub choices: Vec<(ItemKind, u16, String, bool)>,
    /// The map's gradients, a choice each.
    gradients: Vec<massif_core::gradient::Gradient>,
    preview: Option<(Vec<u8>, egui::TextureHandle)>,
    pub error: Option<String>,
}

impl ExportForm {
    /// The form for the map's layers: its object layers, and the covers and
    /// terrains that are not those of every new map.
    pub fn new(reg: &massif_core::Registry, author: &str, name: &str) -> Self {
        let vanilla = massif_core::Registry::vanilla();
        let mut choices = Vec::new();
        for (k, l) in reg.object_layers.iter().enumerate() {
            if !l.removed {
                choices.push((ItemKind::Layer, k as u16 + 1, l.name.clone(), true));
            }
        }
        for (k, c) in reg.covers.iter().enumerate() {
            if !c.removed && !vanilla.covers.iter().any(|v| v.name == c.name && v.plants == c.plants) {
                choices.push((ItemKind::Cover, k as u16 + 1, c.name.clone(), true));
            }
        }
        for (k, t) in reg.terrains.iter().enumerate() {
            if !vanilla.terrains.iter().any(|v| v.name == t.name && v.group == t.group) && !t.strata.is_empty() {
                choices.push((ItemKind::Terrain, k as u16, t.name.clone(), true));
            }
        }
        Self { name: name.to_owned(), author: author.to_owned(), description: String::new(), picture: None, choices, gradients: Vec::new(), preview: None, error: None }
    }

    /// The form with the map's gradients to choose from as well.
    pub fn with_gradients(mut self, gradients: &[massif_core::gradient::Gradient]) -> Self {
        for (k, g) in gradients.iter().enumerate() {
            self.choices.push((ItemKind::Gradient, k as u16, g.name.clone(), true));
        }
        self.gradients = gradients.to_vec();
        self
    }

    /// The pack it makes.
    pub fn pack(&self, reg: &massif_core::Registry) -> Pack {
        let pick = |kind| self.choices.iter().filter(|c| c.0 == kind && c.3).map(|c| c.1).collect::<Vec<u16>>();
        let mut content = Pack::gather(reg, &pick(ItemKind::Cover), &pick(ItemKind::Terrain), &pick(ItemKind::Layer));
        let gradients: Vec<_> = pick(ItemKind::Gradient).into_iter().filter_map(|k| self.gradients.get(k as usize).cloned()).collect();
        Pack::gather_gradients(&mut content, reg, &gradients);
        let mut pack = Pack {
            name: pack_file::clean_text(&self.name, pack_file::MAX_NAME, false),
            author: pack_file::clean_text(&self.author, pack_file::MAX_NAME, false),
            description: pack_file::clean_text(&self.description, pack_file::MAX_DESCRIPTION, true),
            made_with: crate::update::RUNNING.to_owned(),
            preview: None,
            content,
        };
        pack.preview = Some(self.picture.clone().unwrap_or_else(|| pack.draw_preview()));
        pack
    }
}

/// What a dialog asked for.
pub enum Outcome {
    Say(String, bool, Option<PathBuf>),
    /// The library changed: show the Packs.
    ShowPacks(String),
}

/// The dialog, if any. Returns what to say; leaves `dialog` as it is
/// while it stays open. `author` is the name remembered for exports.
pub fn dialog(ctx: &egui::Context, dialog: &mut Option<PackDialog>, library: &mut PackLibrary, reg: &massif_core::Registry, author: &mut String) -> Option<Outcome> {
    let mut out = None;
    let mut close = false;
    match dialog.as_mut()? {
        PackDialog::Export(form) => {
            let response = crate::app::modal(ctx, "export pack").show(ctx, |ui| export_ui(ui, form, reg, &mut close, &mut out));
            if response.should_close() {
                close = true;
            }
            if let Some(Outcome::Say(_, false, _)) = &out {
                author.clone_from(&form.author);
            }
        }
        PackDialog::Clash { pack, existing, rename } => {
            let mut action = None;
            let response = crate::app::modal(ctx, "pack clash").show(ctx, |ui| {
                ui.set_width(420.0);
                let pal = Palette::of(ui);
                let theirs = library.packs.get(*existing).map(|s| s.pack.author.clone()).unwrap_or_default();
                title(ui, Icon::Package, &tr!("“{name}” is in your packs already", name = pack.name));
                let by = |a: &str| if a.is_empty() { String::new() } else { tr!(" by {a}", a) };
                ui.add(egui::Label::new(RichText::new(tr!("Your packs have one of that name{by}. The one you import is{by2}. Replace it, or keep both under another name.", by = by(&theirs), by2 = by(&pack.author))).color(pal.weak)).wrap());
                ui.add_space(10.0);
                let free = !rename.trim().is_empty() && library.find(rename.trim()).is_none();
                crate::widgets::well(ui, |ui| {
                    crate::widgets::row(ui, tr!("Name"), |ui| {
                        ui.add(egui::TextEdit::singleline(rename).hint_text(tr!("Another name")).desired_width(f32::INFINITY).char_limit(pack_file::MAX_NAME));
                    });
                    if !free {
                        ui.label(RichText::new(tr!("Choose a name your packs do not have.")).size(11.5).color(pal.weak));
                    }
                });
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(free, theme::primary_button(ui, tr!("Keep both"))).clicked() {
                        action = Some(true);
                    }
                    if ui.button(tr!("Replace")).clicked() {
                        action = Some(false);
                    }
                    if ui.button(tr!("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if response.should_close() {
                close = true;
            }
            if let Some(keep_both) = action {
                let mut pack = (**pack).clone();
                let replace = if keep_both {
                    pack.name = pack_file::clean_text(rename, pack_file::MAX_NAME, false);
                    None
                } else {
                    Some(*existing)
                };
                out = Some(added(library.add(pack.clone(), replace), &pack));
                close = true;
            }
        }
        PackDialog::Remove(i) => {
            let i = *i;
            let Some(name) = library.packs.get(i).map(|s| s.pack.name.clone()) else {
                *dialog = None;
                return None;
            };
            let mut remove = false;
            let response = crate::app::modal(ctx, "remove pack").show(ctx, |ui| {
                ui.set_width(400.0);
                let pal = Palette::of(ui);
                title(ui, Icon::Trash2, &tr!("Remove “{name}” from your packs?", name));
                ui.add(egui::Label::new(RichText::new(tr!("Its file is deleted from your packs. Maps keep the layers they took from it.")).color(pal.weak)).wrap());
                ui.add_space(12.0);
                ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new(tr!("Remove")).color(Color32::WHITE)).fill(pal.danger)).clicked() {
                        remove = true;
                    }
                    if ui.button(tr!("Cancel")).clicked() {
                        close = true;
                    }
                });
            });
            if response.should_close() {
                close = true;
            }
            if remove {
                out = Some(match library.remove(i) {
                    Ok(p) => Outcome::Say(tr!("Removed the pack “{name}”", name = p.name), false, None),
                    Err(e) => Outcome::Say(tr!("Could not remove the pack: {e}", e), true, None),
                });
                close = true;
            }
        }
    }
    if close {
        *dialog = None;
    }
    out
}

/// What to say once a pack is added.
pub fn added(result: Result<usize, String>, pack: &Pack) -> Outcome {
    match result {
        Ok(_) => {
            let by = if pack.author.is_empty() { String::new() } else { tr!(" by {author}", author = pack.author) };
            Outcome::ShowPacks(tr!("Added the pack “{name}”{by} to your packs: {things}", name = pack.name, things = things(pack.len()), by))
        }
        Err(e) => Outcome::Say(tr!("Could not add the pack: {e}", e), true, None),
    }
}

fn things(n: usize) -> String {
    trn!("{n} thing", "{n} things", n = n)
}

fn title(ui: &mut Ui, icon: Icon, text: &str) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(icon).to_string()).size(17.0).color(pal.accent));
        ui.add(egui::Label::new(RichText::new(text).bold().size(15.0)).truncate());
    });
    ui.add_space(2.0);
}

fn export_ui(ui: &mut Ui, form: &mut ExportForm, reg: &massif_core::Registry, close: &mut bool, out: &mut Option<Outcome>) {
    ui.set_width(460.0);
    let pal = Palette::of(ui);
    title(ui, Icon::Package, tr!("Export a pack"));
    ui.add(egui::Label::new(RichText::new(tr!("A .massifpack file of the map's layers, to share: others import it into their packs.")).color(pal.weak)).wrap());
    ui.add_space(10.0);
    crate::widgets::well(ui, |ui| {
        crate::widgets::row(ui, tr!("Name"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.name).hint_text(tr!("The pack's name")).desired_width(f32::INFINITY).char_limit(pack_file::MAX_NAME));
        });
        crate::widgets::row(ui, tr!("Author"), |ui| {
            ui.add(egui::TextEdit::singleline(&mut form.author).hint_text(tr!("Your name")).desired_width(f32::INFINITY).char_limit(pack_file::MAX_NAME));
        });
    });
    ui.add_space(6.0);
    ui.add(egui::TextEdit::multiline(&mut form.description).hint_text(tr!("A description: what it holds, what it is for")).desired_rows(2).desired_width(f32::INFINITY).char_limit(pack_file::MAX_DESCRIPTION));
    ui.add_space(8.0);
    ui.label(RichText::new(tr!("What it holds")).medium());
    ui.add_space(2.0);
    if form.choices.is_empty() {
        ui.add(egui::Label::new(RichText::new(tr!("The map has no layers of its own yet: add some from the Packs or the Layers panel first.")).color(pal.weak)).wrap());
    } else {
        egui::ScrollArea::vertical().max_height(200.0).show(ui, |ui| {
            crate::widgets::well(ui, |ui| {
                for (kind, _, name, on) in &mut form.choices {
                    let what = match kind {
                        ItemKind::Layer => tr!("Object layer"),
                        ItemKind::Cover => tr!("Ground cover"),
                        ItemKind::Terrain => tr!("Terrain"),
                        ItemKind::Gradient => tr!("Gradient"),
                    };
                    ui.horizontal(|ui| {
                        crate::widgets::checkbox(ui, on, name.as_str());
                        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(what).size(11.5).color(pal.weak));
                        });
                    });
                }
            });
        });
    }
    ui.add_space(8.0);
    let pack = form.pack(reg);
    ui.horizontal(|ui| {
        let png = pack.preview.clone().unwrap_or_default();
        if form.preview.as_ref().is_none_or(|p| p.0 != png)
            && let Ok(img) = image::load_from_memory_with_format(&png, image::ImageFormat::Png)
        {
            let img = img.to_rgba8();
            let image = egui::ColorImage::from_rgba_unmultiplied([img.width() as usize, img.height() as usize], img.as_raw());
            form.preview = Some((png, ui.ctx().load_texture("export pack preview", image, egui::TextureOptions::LINEAR)));
        }
        if let Some((_, tex)) = &form.preview {
            let size = tex.size_vec2();
            let scale = (160.0 / size.x).min(80.0 / size.y);
            ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(theme::TILE_RADIUS));
        }
        ui.vertical(|ui| {
            ui.label(RichText::new(tr!("Picture")).medium());
            ui.label(RichText::new(if form.picture.is_some() { tr!("Chosen") } else { tr!("Made from what it holds") }).size(11.5).color(pal.weak));
            ui.horizontal(|ui| {
                if ui.button(tr!("Choose…")).clicked()
                    && let Some(path) = rfd::FileDialog::new().set_title(tr!("A picture for the pack")).add_filter(tr!("Pictures"), &["png", "jpg", "jpeg"]).pick_file()
                {
                    match std::fs::read(&path).map_err(|e| e.to_string()).and_then(|b| pack_file::picture_from_file(&b).map_err(|e| e.to_string())) {
                        Ok(png) => form.picture = Some(png),
                        Err(e) => form.error = Some(e),
                    }
                }
                if form.picture.is_some() && ui.button(tr!("Reset")).clicked() {
                    form.picture = None;
                }
            });
        });
    });
    if let Some(e) = &form.error {
        ui.add(egui::Label::new(RichText::new(e).color(pal.danger)).wrap());
    }
    ui.add_space(12.0);
    let ready = !pack.name.is_empty() && !pack.is_empty();
    ui.horizontal(|ui| {
        ui.label(RichText::new(things(pack.len())).size(11.5).color(pal.weak));
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.add_enabled(ready, theme::primary_button(ui, tr!("Export…"))).clicked()
                && let Some(path) = rfd::FileDialog::new()
                    .set_title(tr!("Export a pack"))
                    .add_filter(tr!("Massif pack"), &[pack_file::EXTENSION])
                    .set_file_name(format!("{}.{}", pack_file::file_stem(&pack.name), pack_file::EXTENSION))
                    .save_file()
            {
                let path = if is_pack_file(&path) { path } else { path.with_extension(pack_file::EXTENSION) };
                let bytes = pack.to_bytes();
                match massif_core::atomic::write(&path, &bytes, true) {
                    Ok(()) => {
                        *out = Some(Outcome::Say(tr!("Exported the pack “{name}” ({things}, {human})", name = pack.name, things = things(pack.len()), human = crate::panels::human(bytes.len() as u64)), false, Some(path)));
                        *close = true;
                    }
                    Err(e) => form.error = Some(massif_core::atomic::describe(&e, &path)),
                }
            }
            if ui.button(tr!("Cancel")).clicked() {
                *close = true;
            }
        });
    });
}

/// The library's part of the Packs panel: each pack, its picture, its
/// author, its things to paint. Returns what to say.
pub fn library_ui(ui: &mut Ui, library: &mut PackLibrary, needle: &str, tools: &mut ToolState, doc: &mut Document, catalogue: Option<&massif_assets::Catalogue>) -> Option<String> {
    library.ensure_loaded();
    let pal = Palette::of(ui);
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        // The button first, the title in the room it leaves.
        ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
            if ui.small_button(theme::with_icon(Icon::Import, tr!("Import…"))).on_hover_text(tr!("Add a .massifpack file someone shared")).clicked() {
                library.request = Some(Request::Import);
            }
            ui.with_layout(egui::Layout::left_to_right(Align::Center), |ui| {
                ui.add(egui::Label::new(RichText::new(tr!("{glyph}  YOUR PACKS", glyph = theme::glyph(Icon::Package))).size(10.5).strong().color(pal.weak)).truncate());
            });
        });
    });
    if library.packs.is_empty() {
        ui.add(egui::Label::new(RichText::new(tr!("Packs others share are .massifpack files: import one here, from File ▸ Import Pack…, or drop it on the window. File ▸ Export Pack… makes one of the map's layers.")).size(11.5).color(pal.weak)).wrap());
        return None;
    }
    let mut picked = None;
    for p in 0..library.packs.len() {
        let pack = &library.packs[p].pack;
        let matches_pack = needle.is_empty() || pack.name.to_lowercase().contains(needle) || pack.author.to_lowercase().contains(needle);
        let items: Vec<(ItemKind, usize, String)> = pack.items().filter(|(_, _, n)| matches_pack || n.to_lowercase().contains(needle)).map(|(k, i, n)| (k, i, n.to_owned())).collect();
        if items.is_empty() {
            continue;
        }
        ui.add_space(6.0);
        let preview = library.preview(ui.ctx(), p);
        let pack = &library.packs[p].pack;
        let header = ui.horizontal(|ui| {
            if let Some(tex) = &preview {
                let size = tex.size_vec2();
                let scale = (48.0 / size.x).min(24.0 / size.y);
                ui.add(egui::Image::new(tex).fit_to_exact_size(size * scale).corner_radius(theme::TILE_RADIUS));
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.add(egui::Label::new(RichText::new(&pack.name).size(12.5).medium()).truncate());
                let by = if pack.author.is_empty() { tr!("Unknown author").to_owned() } else { tr!("by {author}", author = pack.author) };
                ui.add(egui::Label::new(RichText::new(tr!("{by} · {things}", things = things(pack.len()), by)).size(11.0).color(pal.weak)).truncate());
            });
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                ui.menu_button(theme::glyph(Icon::Ellipsis).to_string(), |ui| {
                    if ui.button(theme::with_icon(Icon::FileDown, tr!("Export a copy…"))).clicked() {
                        library.request = Some(Request::Share(p));
                        ui.close();
                    }
                    if ui.button(theme::with_icon(Icon::Trash2, tr!("Remove…"))).clicked() {
                        library.request = Some(Request::Remove(p));
                        ui.close();
                    }
                });
            });
        });
        let pack = &library.packs[p].pack;
        if !pack.description.is_empty() {
            header.response.on_hover_text(&pack.description);
        }
        ui.add_space(2.0);
        let gap = 4.0;
        let width = ui.available_width();
        let columns = ((width + gap) / 150.0).floor().max(1.0) as usize;
        let tile_w = (width - gap * (columns - 1) as f32) / columns as f32;
        for row in items.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for (kind, i, name) in row {
                    let reg = &doc.world.registry;
                    let found = pack.find(*kind, *i, reg);
                    let painted = found.is_some_and(|n| match kind {
                        ItemKind::Layer => tools.tool == Tool::Objects && tools.object_layer == n,
                        ItemKind::Cover => tools.tool == Tool::Cover && tools.cover == n,
                        ItemKind::Terrain => tools.tool == Tool::PaintTerrain && !tools.pattern.on && tools.terrain == n,
                        ItemKind::Gradient => tools.tool == Tool::Gradient && tools.gradient.gradient.name == *name,
                    });
                    let what = match kind {
                        ItemKind::Layer => tr!("An object layer."),
                        ItemKind::Cover => tr!("A ground cover."),
                        ItemKind::Terrain => tr!("A terrain."),
                        ItemKind::Gradient => tr!("A gradient for the Gradient brush."),
                    };
                    let tip = tr!("{what} From “{name}”.{value}", name = pack.name, value = if found.is_some() { tr!("\nOn the map already: a click picks it.") } else { "" }, what);
                    let clicked = crate::packs::tile_ui(ui, tile_w, name, painted, found.is_some(), &tip, |painter, rect| picture(painter, rect, pack, *kind, *i, catalogue));
                    if clicked {
                        picked = Some((p, *kind, *i));
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }
    let (p, kind, i) = picked?;
    Some(apply(&library.packs[p].pack, kind, i, tools, doc, catalogue))
}

/// Add item `(kind, i)` of `pack` to the map (or find it there) and pick
/// it to paint. Returns what to say.
pub fn apply(pack: &Pack, kind: ItemKind, i: usize, tools: &mut ToolState, doc: &mut Document, catalogue: Option<&massif_assets::Catalogue>) -> String {
    let had = pack.installed(kind, i, &doc.world.registry);
    let reg = &mut doc.world.registry;
    let n = pack.install(kind, i, reg);
    match kind {
        ItemKind::Layer => {
            tools.object_layer = n;
            tools.select(Tool::Objects);
        }
        ItemKind::Cover => {
            tools.cover = n;
            tools.select(Tool::Cover);
        }
        ItemKind::Terrain => {
            tools.terrain = n;
            if let Some(cat) = catalogue {
                massif_assets::apply_colors(reg, cat);
            }
            tools.pattern.on = false;
            tools.select(Tool::PaintTerrain);
        }
        ItemKind::Gradient => {
            tools.gradient.gradient = pack.install_gradient(i, reg);
            if let Some(cat) = catalogue {
                massif_assets::apply_colors(reg, cat);
            }
            let kept = doc.world.meta.gradients.contains(&tools.gradient.gradient);
            tools.gradient.keep_in(doc);
            tools.select(Tool::Gradient);
            let name = tools.gradient.gradient.name.clone();
            return if had && kept {
                tr!("Painting with {name}", name)
            } else {
                doc.mark_changed();
                tr!("Added the gradient {name} to the map: paint with it", name)
            };
        }
    }
    let name = match kind {
        ItemKind::Layer => reg.object_layer(n).map(|l| l.name.clone()),
        ItemKind::Cover => reg.cover(n).map(|c| c.name.clone()),
        ItemKind::Terrain => reg.terrains.get(n as usize).map(|t| t.name.clone()),
        ItemKind::Gradient => None,
    }
    .unwrap_or_default();
    if had {
        tr!("Painting with {name}", name)
    } else {
        doc.mark_changed();
        tr!("Added {name} to the map's layers: paint it", name)
    }
}

/// A shared item's picture: a terrain as a few of its blocks, the others
/// on a swatch of their colour with their icon.
fn picture(painter: &egui::Painter, rect: Rect, pack: &Pack, kind: ItemKind, i: usize, catalogue: Option<&massif_assets::Catalogue>) {
    let color_of = |state: &str, fallback: [u8; 3]| catalogue.and_then(|cat| cat.block(state)).map_or(fallback, |b| b.color);
    match kind {
        ItemKind::Terrain => {
            let t = &pack.content.terrains[i];
            let s = &t.strata[0];
            let total: u32 = s.blocks.iter().map(|b| b.weight as u32).sum::<u32>().max(1);
            let n = 3;
            let cell = rect.width() / n as f32;
            for j in 0..n {
                for k in 0..n {
                    let mut r = (massif_core::compose::hash2(k + j * n, 0, 0x7061_636b) % total) as i32;
                    let b = s.blocks.iter().find(|b| {
                        r -= b.weight as i32;
                        r < 0
                    });
                    let color = b.map_or(t.color, |b| color_of(&b.state, b.color));
                    painter.rect_filled(Rect::from_min_size(rect.min + vec2(k as f32 * cell, j as f32 * cell), vec2(cell, cell)), 0.0, theme::rgb(color));
                }
            }
            painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, Color32::from_black_alpha(70)), egui::StrokeKind::Inside);
        }
        ItemKind::Cover => {
            let c = &pack.content.covers[i];
            crate::packs::swatch(painter, rect, c.color, if c.underwater { Icon::Waves } else { Icon::Sprout });
        }
        ItemKind::Layer => {
            let l = &pack.content.object_layers[i];
            crate::packs::swatch(painter, rect, l.color, Icon::TreeDeciduous);
        }
        ItemKind::Gradient => {
            // Its bands across, each its width.
            let g = &pack.content.gradients[i];
            let vanilla = massif_core::Registry::vanilla();
            let total = g.total().max(1.0);
            let mut x = rect.min.x;
            for (k, b) in g.bands.iter().enumerate() {
                let c = pack.content.terrains.iter().find(|t| t.name == b.terrain).map(|t| t.color).or_else(|| vanilla.terrain_by_name(&b.terrain).map(|t| vanilla.terrain(t).color)).unwrap_or([128; 3]);
                let w = (b.width + g.transitions.get(k).map_or(0.0, |t| t.width)) / total * rect.width();
                painter.rect_filled(Rect::from_min_max(pos2(x, rect.min.y), pos2((x + w).min(rect.max.x), rect.max.y)), 0.0, theme::rgb(c));
                x += w;
            }
            painter.rect_stroke(rect, theme::TILE_RADIUS, egui::Stroke::new(1.0, Color32::from_black_alpha(70)), egui::StrokeKind::Inside);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use massif_core::generate::{GenParams, generate};
    use massif_core::packs::{MATERIAL_MIXES, PLANT_PRESETS};
    use massif_core::treepack::TreeKind;

    fn temp(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("massif-packs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    /// A pack of firs, a tulip field and grey cliffs, made from a map.
    fn sample(name: &str, author: &str) -> Pack {
        let mut reg = massif_core::Registry::vanilla();
        let layer = TreeKind::Fir.install(&mut reg);
        let cover = PLANT_PRESETS.iter().find(|p| p.name == "Tulip field").unwrap().install(&mut reg);
        let terrain = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut reg).0;
        let mut form = ExportForm::new(&reg, author, name);
        form.description = "Firs, tulips and grey cliffs, for northern valleys.".into();
        assert_eq!(form.choices.iter().map(|c| (c.0, c.1)).collect::<Vec<_>>(), [(ItemKind::Layer, layer), (ItemKind::Cover, cover), (ItemKind::Terrain, terrain)]);
        form.pack(&reg)
    }

    #[test]
    fn the_library_adds_reloads_and_removes_packs() {
        let dir = temp("library");
        let mut lib = PackLibrary::at(Some(dir.clone()));
        lib.ensure_loaded();
        assert!(lib.packs.is_empty());
        // A file that is not a pack, among them, is left out.
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("broken.massifpack"), b"PK not really").unwrap();
        let pack = sample("../../Northern woods", "Nedgar");
        let i = lib.add(pack.clone(), None).unwrap();
        let file = lib.packs[i].file.clone();
        assert_eq!(file.parent(), Some(dir.as_path()), "kept in the library folder");
        assert_eq!(file.file_name().unwrap(), "Northern woods.massifpack");
        let mut again = PackLibrary::at(Some(dir.clone()));
        again.ensure_loaded();
        assert_eq!(again.packs.len(), 1);
        assert_eq!(again.packs[0].pack, pack);
        assert_eq!(again.find("../../NORTHERN WOODS"), Some(0));
        // Replaced: one file still.
        let newer = Pack { author: "Someone".into(), ..pack.clone() };
        let i = again.add(newer, Some(0)).unwrap();
        assert_eq!(again.packs.len(), 1);
        assert_eq!(again.packs[i].pack.author, "Someone");
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 2, "the pack and the broken file");
        again.remove(0).unwrap();
        assert!(again.packs.is_empty() && !file.exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct State {
        dialog: Option<PackDialog>,
        library: PackLibrary,
        reg: massif_core::Registry,
        author: String,
        said: Vec<String>,
    }

    fn dialogs(ui: &mut Ui, s: &mut State) {
        theme::install(ui.ctx());
        if let Some(out) = dialog(ui.ctx(), &mut s.dialog, &mut s.library, &s.reg, &mut s.author) {
            s.said.push(match out {
                Outcome::Say(t, _, _) | Outcome::ShowPacks(t) => t,
            });
        }
    }

    /// A pack whose name the library has: kept both under another name,
    /// or put in the place of the first; then removed.
    #[test]
    fn a_name_taken_asks_to_rename_or_replace() {
        let dir = temp("clash");
        let mut library = PackLibrary::at(Some(dir.clone()));
        library.ensure_loaded();
        library.add(sample("Northern woods", "Nedgar"), None).unwrap();
        let clash = |author: &str| PackDialog::Clash { pack: Box::new(sample("Northern woods", author)), existing: 0, rename: "Northern woods 2".into() };
        let state = State { dialog: Some(clash("Someone")), library, reg: massif_core::Registry::vanilla(), author: String::new(), said: Vec::new() };
        let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(dialogs, state);
        h.run_steps(2);
        h.get_by_label("Keep both").click();
        h.run_steps(2);
        let s = h.state();
        assert!(s.dialog.is_none());
        let names: Vec<&str> = s.library.packs.iter().map(|p| p.pack.name.as_str()).collect();
        assert_eq!(names, ["Northern woods", "Northern woods 2"]);
        assert_eq!(s.library.packs[1].pack.author, "Someone");
        h.state_mut().dialog = Some(clash("Third"));
        h.run_steps(2);
        h.get_by_label("Replace").click();
        h.run_steps(2);
        let s = h.state();
        assert_eq!(s.library.packs.len(), 2);
        assert_eq!(s.library.packs[0].pack.author, "Third");
        assert!(s.said.last().unwrap().starts_with("Added the pack “Northern woods” by Third"));
        h.state_mut().dialog = Some(PackDialog::Remove(1));
        h.run_steps(2);
        h.get_by_label("Remove").click();
        h.run_steps(2);
        assert_eq!(h.state().library.packs.len(), 1);
        assert_eq!(h.state().said.last().unwrap(), "Removed the pack “Northern woods 2”");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    struct Panel {
        tools: ToolState,
        doc: Document,
        search: String,
        library: PackLibrary,
    }

    fn panel(ui: &mut Ui, s: &mut Panel) {
        theme::install(ui.ctx());
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            crate::packs::packs_panel(ui, &mut s.tools, &mut s.doc, None, &mut s.search, &mut s.library);
        });
    }

    fn panel_state(dir: &Path) -> Panel {
        let mut library = PackLibrary::at(Some(dir.to_path_buf()));
        library.ensure_loaded();
        library.add(sample("Northern woods", "Nedgar"), None).unwrap();
        let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        Panel { tools: ToolState::default(), doc: Document::new(world, None, 1), search: String::new(), library }
    }

    /// The panel shows the pack with its author; a click on one of its
    /// things adds it to the map once and picks it; the search finds the
    /// pack by its author.
    #[test]
    fn shared_packs_are_shown_and_painted() {
        let dir = temp("panel");
        let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 3000.0)).build_ui_state(panel, panel_state(&dir));
        h.run_steps(2);
        h.get_by_label("Northern woods");
        h.get_by_label("by Nedgar · 3 things");
        let layers = h.state().doc.world.registry.object_layers.len();
        // "Firs" is a tile of Massif's trees too: the first is the pack's.
        for _ in 0..2 {
            h.get_all_by_label("Firs").next().unwrap().click();
            h.run_steps(2);
        }
        let s = h.state();
        assert_eq!(s.doc.world.registry.object_layers.len(), layers + 1);
        assert_eq!(s.tools.tool, Tool::Objects);
        assert_eq!(s.doc.world.registry.object_layer(s.tools.object_layer).unwrap().name, "Firs");
        h.get_all_by_label("Grey cliff strata").next().unwrap().click();
        h.run_steps(2);
        assert_eq!(h.state().tools.tool, Tool::PaintTerrain);
        h.state_mut().search = "nedgar".into();
        h.run_steps(2);
        assert!(h.query_by_label("Tulip field").is_some() && h.query_by_label("Badlands strata").is_none());
        // The panel's menu asks the app to remove it.
        h.state_mut().search.clear();
        h.run_steps(2);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The panel with a pack and the dialogs, offscreen in both themes.
    /// Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn pack_dialogs_render() {
        let Some(out) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) else {
            return;
        };
        let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
        let dir = temp("render");
        for dark in [true, false] {
            let theme = if dark { egui::Theme::Dark } else { egui::Theme::Light };
            let suffix = if dark { "dark" } else { "light" };
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 900.0))
                .with_pixels_per_point(ppp)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut Panel| {
                        ui.ctx().set_theme(theme);
                        panel(ui, s);
                    },
                    panel_state(&dir),
                );
            h.run_steps(4);
            h.render().unwrap().save(out.join(format!("packs-shared-{suffix}.png"))).unwrap();
            let _ = std::fs::remove_dir_all(&dir);

            let mut reg = massif_core::Registry::vanilla();
            TreeKind::Larch.install(&mut reg);
            TreeKind::Cherry.install(&mut reg);
            PLANT_PRESETS[2].install(&mut reg);
            MATERIAL_MIXES[5].install(&mut reg);
            let mut form = ExportForm::new(&reg, "Nedgar", "Spring valley");
            form.description = "Larches, cherry trees and tulips.".into();
            form.choices[1].3 = false;
            let mut library = PackLibrary::at(Some(dir.clone()));
            library.ensure_loaded();
            library.add(sample("Northern woods", "Nedgar"), None).unwrap();
            let dialogs_of = [
                ("export", PackDialog::Export(Box::new(form))),
                ("clash", PackDialog::Clash { pack: Box::new(sample("Northern woods", "Someone")), existing: 0, rename: "Northern woods 2".into() }),
                ("remove", PackDialog::Remove(0)),
            ];
            let mut state = State { dialog: None, library, reg, author: String::new(), said: Vec::new() };
            for (name, d) in dialogs_of {
                state.dialog = Some(d);
                let mut h = egui_kittest::Harness::builder()
                    .with_size(vec2(620.0, 640.0))
                    .with_pixels_per_point(ppp)
                    .with_accessibility_check(false)
                    .wgpu()
                    .build_ui_state(
                        move |ui, s: &mut State| {
                            ui.ctx().set_theme(theme);
                            dialogs(ui, s);
                        },
                        state,
                    );
                h.run_steps(4);
                h.render().unwrap().save(out.join(format!("pack-{name}-{suffix}.png"))).unwrap();
                state = h.into_state();
            }
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
}
