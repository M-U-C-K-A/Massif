//! File ▸ Export ▸ Selection as Schematic…: the selected part of the map
//! as a WorldEdit or Litematica schematic, the blocks the export to
//! Minecraft would write there, with the materials it holds counted before
//! it is written.
//!
//! The blocks are built in the background as soon as the dialog opens, and
//! again when a setting they depend on changes; the file is then made from
//! them (again when its format changes), so that its size is known before
//! it is saved.

use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::widgets::{Field, button_group, row, switch, well};
use egui::{RichText, Ui};
use massif_anvil::Progress;
use massif_anvil::extract::{Bottom, ExtractOptions, Extracted, LARGE_BLOCKS};
use massif_core::selection::Selection;
use massif_core::world::World;
use massif_objects::write::{Format, Volume};
use std::sync::atomic::Ordering;
use std::sync::mpsc::{Receiver, channel};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What the blocks depend on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BlocksKey {
    depth: i32,
    floor: bool,
    objects: bool,
    structures: bool,
    biomes: bool,
}

/// What the file depends on besides its blocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct FileKey {
    blocks: BlocksKey,
    format: Format,
    air: bool,
    corner: bool,
}

/// Work running in its own thread.
struct Work<K, T> {
    key: K,
    progress: Arc<Progress>,
    rx: Receiver<T>,
    started: Instant,
}

/// The blocks built, and how long it took.
type Built = Result<(Arc<Extracted>, Duration), String>;
/// The file made, and how long it took.
type Made = Result<(Arc<Vec<u8>>, Duration), String>;

pub struct SchemForm {
    pub format: Format,
    /// Blocks under the lowest ground of the selection.
    pub depth: i32,
    /// Down to the world's floor instead.
    pub floor: bool,
    pub objects: bool,
    pub structures: bool,
    pub biomes: bool,
    /// Keep the air: pasted, it clears what stands there. Without it, the
    /// empty blocks are structure voids, which Litematica and the game's
    /// structures leave as they are.
    pub air: bool,
    /// The origin at the box's corner rather than the middle of its foot.
    pub corner: bool,
    world: Arc<World>,
    selection: Arc<Selection>,
    dim: usize,
    target: massif_anvil::Target,
    mod_blocks: Arc<std::collections::HashMap<String, Vec<String>>>,
    families: Arc<massif_core::block_families::BlockFamilies>,
    building: Option<Work<BlocksKey, Built>>,
    built: Option<(BlocksKey, Built)>,
    making: Option<Work<FileKey, Made>>,
    made: Option<(FileKey, Made)>,
}

pub enum Choice {
    Keep,
    Close,
    /// Save these bytes as a file of this extension.
    Save(Arc<Vec<u8>>, &'static str),
}

/// The settings kept from one opening to the next.
#[derive(Debug, Clone, Copy)]
pub struct Remembered {
    pub format: Format,
    pub depth: i32,
    pub floor: bool,
    pub objects: bool,
    pub structures: bool,
    pub biomes: bool,
    pub air: bool,
    pub corner: bool,
}

impl Default for Remembered {
    fn default() -> Self {
        Self { format: Format::Sponge3, depth: 8, floor: false, objects: true, structures: true, biomes: true, air: true, corner: false }
    }
}

impl SchemForm {
    #[allow(clippy::too_many_arguments)]
    pub fn new(r: Remembered, world: World, selection: Arc<Selection>, dim: usize, target: massif_anvil::Target, mod_blocks: Arc<std::collections::HashMap<String, Vec<String>>>, families: Arc<massif_core::block_families::BlockFamilies>) -> Self {
        Self {
            format: r.format,
            depth: r.depth,
            floor: r.floor,
            objects: r.objects,
            structures: r.structures,
            biomes: r.biomes,
            air: r.air,
            corner: r.corner,
            world: Arc::new(world),
            selection,
            dim,
            target,
            mod_blocks,
            families,
            building: None,
            built: None,
            making: None,
            made: None,
        }
    }

    pub fn remembered(&self) -> Remembered {
        Remembered { format: self.format, depth: self.depth, floor: self.floor, objects: self.objects, structures: self.structures, biomes: self.biomes, air: self.air, corner: self.corner }
    }

    fn blocks_key(&self) -> BlocksKey {
        BlocksKey { depth: self.depth, floor: self.floor, objects: self.objects, structures: self.structures, biomes: self.biomes }
    }

    fn file_key(&self) -> FileKey {
        FileKey { blocks: self.blocks_key(), format: self.format, air: self.air, corner: self.corner }
    }

    /// Stop what runs in the background: the dialog is closed.
    pub fn stop(&mut self) {
        for p in [self.building.as_ref().map(|w| &w.progress), self.making.as_ref().map(|w| &w.progress)].into_iter().flatten() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Start what the settings call for, take in what has finished.
    fn run(&mut self, ctx: &egui::Context) {
        let key = self.blocks_key();
        if let Some(w) = &self.building {
            let got = match w.rx.try_recv() {
                Ok(built) => Some(built),
                // The thread ended without a word: it failed.
                Err(std::sync::mpsc::TryRecvError::Disconnected) if !w.progress.cancel.load(Ordering::Relaxed) => Some(Err(tr!("The blocks could not be built: see the log.").to_owned())),
                Err(_) => None,
            };
            if let Some(built) = got {
                let key = w.key;
                self.building = None;
                self.built = Some((key, built));
            }
        }
        if self.built.as_ref().is_none_or(|b| b.0 != key) && self.building.as_ref().is_none_or(|w| w.key != key) {
            if let Some(w) = self.building.take() {
                w.progress.cancel.store(true, Ordering::Relaxed);
            }
            let options = ExtractOptions {
                dimension: self.dim,
                target: self.target.clone(),
                mod_blocks: self.mod_blocks.clone(),
                families: self.families.clone(),
                bottom: if key.floor { Bottom::Floor } else { Bottom::BelowGround(key.depth) },
                objects: key.objects,
                structures: key.structures,
                biomes: key.biomes,
            };
            let progress = Arc::new(Progress::default());
            let (tx, rx) = channel();
            let (world, selection, p, ctx2) = (self.world.clone(), self.selection.clone(), progress.clone(), ctx.clone());
            let _ = std::thread::Builder::new().name("massif schematic".into()).spawn(move || {
                let started = Instant::now();
                // In the builders' threads, not the interface's.
                let result = massif_render::builder_pool().install(|| massif_anvil::extract::extract(&world, &selection, &options, &p));
                let built = match result {
                    Ok(e) => Ok((Arc::new(e), started.elapsed())),
                    Err(massif_anvil::ExportError::Cancelled) => return,
                    Err(massif_anvil::ExportError::Empty) => Err(tr!("Nothing selected stands on the map.").to_owned()),
                    Err(e) => Err(e.to_string()),
                };
                let _ = tx.send(built);
                ctx2.request_repaint();
            });
            self.building = Some(Work { key, progress, rx, started: Instant::now() });
        }
        // The file, once the blocks are in.
        let fkey = self.file_key();
        if let Some(w) = &self.making {
            let got = match w.rx.try_recv() {
                Ok(made) => Some(made),
                Err(std::sync::mpsc::TryRecvError::Disconnected) if !w.progress.cancel.load(Ordering::Relaxed) => Some(Err(tr!("The file could not be made: see the log.").to_owned())),
                Err(_) => None,
            };
            if let Some(made) = got {
                let key = w.key;
                self.making = None;
                self.made = Some((key, made));
            }
        }
        let Some((bkey, Ok((extracted, _)))) = &self.built else { return };
        if *bkey != key || self.made.as_ref().is_some_and(|m| m.0 == fkey) || self.making.as_ref().is_some_and(|w| w.key == fkey) {
            return;
        }
        if let Some(w) = self.making.take() {
            w.progress.cancel.store(true, Ordering::Relaxed);
        }
        let (extracted, progress, ctx2) = (extracted.clone(), Arc::new(Progress::default()), ctx.clone());
        let (tx, rx) = channel();
        let p = progress.clone();
        let _ = std::thread::Builder::new().name("massif schematic file".into()).spawn(move || {
            let started = Instant::now();
            let volume = volume_for(&extracted, fkey);
            let made = massif_objects::write::write(&volume, fkey.format).map(|b| (Arc::new(b), started.elapsed())).map_err(|e| e.to_string());
            if !p.cancel.load(Ordering::Relaxed) {
                let _ = tx.send(made);
                ctx2.request_repaint();
            }
        });
        self.making = Some(Work { key: fkey, progress, rx, started: Instant::now() });
    }

    pub fn ui(&mut self, ui: &mut Ui, blocks: &mut crate::blocks::BlockLibrary) -> Choice {
        self.run(ui.ctx());
        let pal = Palette::of(ui);
        ui.set_width(480.0);
        let top = ui.min_rect().top();
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::glyph(Icon::Box).to_string()).size(17.0).color(pal.accent));
            ui.label(RichText::new(tr!("Export Selection as Schematic")).size(16.0).bold());
        });
        let key = egui::Id::unique("schematic export buttons");
        let buttons = ui.data(|d| d.get_temp::<f32>(key)).unwrap_or(44.0);
        let room = (crate::app::modal_room(ui.ctx()) - (ui.min_rect().bottom() - top) - buttons).max(160.0);
        egui::ScrollArea::vertical().id_salt("schematic export").max_height(room).auto_shrink([false, true]).show(ui, |ui| self.settings(ui, blocks));
        let from = ui.min_rect().bottom();
        ui.add_space(12.0);
        let mut choice = Choice::Keep;
        let ready = self.ready();
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add_enabled(ready.is_some(), theme::primary_button(ui, tr!("Export…"))).clicked()
                && let Some(bytes) = ready
            {
                choice = Choice::Save(bytes, self.format.extension());
            }
            if ui.button(tr!("Cancel")).clicked() {
                choice = Choice::Close;
            }
            let text = self.as_text();
            if ui.add_enabled(text.is_some(), egui::Button::new(theme::with_icon(Icon::Copy, tr!("Copy as text")))).on_hover_text(tr!("The size and the materials, a line each, to paste in a message or a note")).clicked()
                && let Some(text) = text
            {
                ui.ctx().copy_text(text);
            }
        });
        ui.data_mut(|d| d.insert_temp(key, ui.min_rect().bottom() - from));
        // The progress moves while something is built.
        if (self.building.is_some() || self.making.is_some()) && !theme::reduce_motion(ui.ctx()) {
            ui.ctx().request_repaint_after(Duration::from_millis(200));
        }
        choice
    }

    /// The file, when it is made for the settings as they are.
    pub(crate) fn ready(&self) -> Option<Arc<Vec<u8>>> {
        match &self.made {
            Some((k, Ok((bytes, _)))) if *k == self.file_key() => Some(bytes.clone()),
            _ => None,
        }
    }

    fn extracted(&self) -> Option<&Arc<Extracted>> {
        match &self.built {
            Some((k, Ok((e, _)))) if *k == self.blocks_key() => Some(e),
            _ => None,
        }
    }

    fn settings(&mut self, ui: &mut Ui, blocks: &mut crate::blocks::BlockLibrary) {
        let pal = Palette::of(ui);
        let note = |ui: &mut Ui, text: &str| {
            ui.add(egui::Label::new(RichText::new(text).size(11.5).color(pal.weak)).wrap());
        };
        note(ui, tr!("The selected part of the map as a schematic, with the blocks the export to Minecraft writes there: its ground, water, caves, plants, trees and structures, to paste with WorldEdit or build from with Litematica."));
        ui.add_space(10.0);
        let formats = [Format::Sponge3, Format::Sponge2, Format::Litematic];
        if let Some(i) = button_group(ui, "schematic format", &[tr!("WorldEdit .schem"), tr!("WorldEdit 7.2 .schem"), tr!("Litematica")], &formats.map(|f| f == self.format)) {
            self.format = formats[i];
        }
        note(
            ui,
            match self.format {
                Format::Sponge3 => tr!("Sponge schematic, version 3: WorldEdit 7.3 and later, FAWE, Axiom, Massif."),
                Format::Sponge2 => tr!("Sponge schematic, version 2: WorldEdit 7.0 to 7.2 and older FAWE. Biomes by the column."),
                Format::Litematic => tr!("Litematica schematic: shown in the world as a guide to build from, with its list of materials. No biomes."),
            },
        );
        ui.add_space(10.0);
        well(ui, |ui| {
            row(ui, tr!("Down to"), |ui| {
                if let Some(i) = button_group(ui, "schematic bottom", &[tr!("Under the ground"), tr!("The world's floor")], &[!self.floor, self.floor]) {
                    self.floor = i == 1;
                }
            });
            if !self.floor {
                row(ui, tr!("Under the lowest ground"), |ui| {
                    ui.add(Field::new(&mut self.depth, 0..=4064).suffix(tr!(" blocks")).width(110.0));
                });
            }
            row(ui, tr!("Trees and objects"), |ui| {
                switch(ui, &mut self.objects);
            });
            row(ui, tr!("Structures"), |ui| {
                switch(ui, &mut self.structures);
            });
            row(ui, tr!("Biomes"), |ui| {
                switch(ui, &mut self.biomes);
            });
            row(ui, tr!("Keep the air"), |ui| {
                switch(ui, &mut self.air);
            });
            row(ui, tr!("Origin"), |ui| {
                if let Some(i) = button_group(ui, "schematic origin", &[tr!("Middle of the foot"), tr!("Corner")], &[!self.corner, self.corner]) {
                    self.corner = i == 1;
                }
            });
        });
        note(
            ui,
            if self.air {
                tr!("Pasted, the air clears what stands there; WorldEdit's //paste -a leaves it out.")
            } else {
                tr!("The empty blocks are structure voids: Litematica and Massif leave what stands there as it is. WorldEdit places them as blocks: keep the air for it, and paste with -a.")
            },
        );
        ui.add_space(10.0);

        // What it holds.
        if let Some(w) = &self.building {
            ui.horizontal(|ui| {
                ui.add(egui::ProgressBar::new(w.progress.fraction()).desired_width(ui.available_width() - 90.0).show_percentage());
                if ui.button(tr!("Stop")).clicked() {
                    w.progress.cancel.store(true, Ordering::Relaxed);
                }
            });
            note(ui, &tr!("Building the blocks of the selection… {s} s", s = format!("{:.1}", w.started.elapsed().as_secs_f32())));
            if w.progress.cancel.load(Ordering::Relaxed) {
                // Stopped: shown as such until a setting changes.
                let key = w.key;
                self.building = None;
                self.built = Some((key, Err(tr!("Stopped. Change a setting to build it again.").to_owned())));
            }
            return;
        }
        if let Some((_, Err(e))) = &self.built {
            ui.label(RichText::new(e).color(pal.danger).size(12.0));
            return;
        }
        let Some(e) = self.extracted().cloned() else { return };
        let v = &e.volume;
        let [w, h, l] = v.size;
        let count = v.count();
        let took = match &self.built {
            Some((_, Ok((_, t)))) => t.as_secs_f32(),
            _ => 0.0,
        };
        well(ui, |ui| {
            row(ui, tr!("Size"), |ui| {
                ui.label(RichText::new(format!("{w} × {h} × {l}")).strong());
            });
            row(ui, tr!("Heights"), |ui| {
                ui.label(tr!("y {a} to {b}", a = e.corner[1], b = e.corner[1] + h as i32 - 1));
            });
            row(ui, tr!("Blocks"), |ui| {
                ui.label(tr!("{n} of {v}", n = crate::select::thousands(count as u64), v = crate::select::thousands(v.volume() as u64)));
            });
            row(ui, tr!("File"), |ui| match &self.made {
                Some((k, Ok((bytes, _)))) if *k == self.file_key() => {
                    ui.label(size_text(bytes.len()));
                }
                Some((k, Err(e))) if *k == self.file_key() => {
                    ui.label(RichText::new(e).color(pal.danger));
                }
                _ => {
                    ui.spinner();
                }
            });
        });
        note(ui, &tr!("Built in {s} s, from x {x}, y {y}, z {z}.", s = format!("{took:.1}"), x = e.corner[0], y = e.corner[1], z = e.corner[2]));
        if v.volume() > LARGE_BLOCKS {
            ui.label(RichText::new(tr!("A large box: past 16 million blocks, WorldEdit and Litematica are slow to paste it, and Massif does not import it as an object. Select less, or raise its bottom.")).color(pal.danger).size(11.5));
        }
        ui.add_space(10.0);
        ui.label(RichText::new(tr!("Materials")).size(11.5).color(pal.weak).medium());
        ui.add_space(4.0);
        let materials = v.materials();
        let cat = blocks.catalogue.clone();
        let ctx = ui.ctx().clone();
        well(ui, |ui| {
            egui::ScrollArea::vertical().id_salt("schematic materials").max_height(220.0).auto_shrink([false, true]).show_rows(ui, 22.0, materials.len(), |ui, range| {
                for (name, n) in &materials[range] {
                    ui.horizontal(|ui| {
                        ui.set_height(22.0);
                        let (r, _) = ui.allocate_exact_size(egui::vec2(18.0, 18.0), egui::Sense::hover());
                        if let Some(cat) = &cat
                            && let Some(block) = cat.block(name)
                            && let Some(texture) = blocks.icon(&ctx, block)
                        {
                            ui.painter().image(texture, r, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
                        }
                        ui.label(short(name));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.label(RichText::new(crate::select::thousands(*n as u64)).monospace());
                        });
                    });
                }
            });
        });
        if !e.objects.is_empty() {
            ui.add_space(10.0);
            ui.label(RichText::new(tr!("Trees and objects")).size(11.5).color(pal.weak).medium());
            ui.add_space(4.0);
            well(ui, |ui| {
                for o in &e.objects {
                    row(ui, &format!("{} · {}", o.layer, o.object), |ui| {
                        ui.label(RichText::new(crate::select::thousands(o.count as u64)).monospace());
                    });
                }
            });
        }
        if !v.block_entities.is_empty() {
            note(ui, &tr!("{n} chests, signs and other blocks keep what they hold.", n = v.block_entities.len()));
        }
    }

    /// The size and the materials as text.
    pub(crate) fn as_text(&self) -> Option<String> {
        let e = self.extracted()?;
        let v = &e.volume;
        let [w, h, l] = v.size;
        let mut out = format!("{} — {w} × {h} × {l}, {} blocks\n", v.name, v.count());
        for (name, n) in v.materials() {
            out += &format!("{n}\t{}\n", short(&name));
        }
        if !e.objects.is_empty() {
            out += "\n";
            for o in &e.objects {
                out += &format!("{}\t{} · {}\n", o.count, o.layer, o.object);
            }
        }
        Some(out)
    }
}

/// A block's name without the game's namespace.
fn short(name: &str) -> &str {
    name.strip_prefix("minecraft:").unwrap_or(name)
}

fn size_text(bytes: usize) -> String {
    if bytes < 1 << 20 {
        format!("{} KB", bytes.div_ceil(1024))
    } else {
        format!("{:.1} MB", bytes as f64 / (1 << 20) as f64)
    }
}

/// The volume written for these settings: air or structure voids, the
/// origin where asked.
fn volume_for(e: &Extracted, key: FileKey) -> Volume {
    let mut v = e.volume.clone();
    if !key.air {
        for s in &mut v.palette {
            if s == "minecraft:air" {
                *s = "minecraft:structure_void".to_owned();
            }
        }
    }
    if key.corner {
        v.origin = [0, 0, 0];
    }
    if key.format == Format::Litematic {
        v.biomes = None;
    }
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_file_follows_its_settings() {
        let e = Extracted {
            volume: Volume { name: "Hill".into(), size: [2, 1, 1], palette: vec!["minecraft:air".into(), "minecraft:stone".into()], blocks: vec![0, 1], origin: [1, 0, 0], ..Default::default() },
            corner: [0, 60, 0],
            objects: Vec::new(),
            lowest_ground: 64,
        };
        let key = FileKey { blocks: BlocksKey { depth: 8, floor: false, objects: true, structures: true, biomes: true }, format: Format::Litematic, air: false, corner: true };
        let v = volume_for(&e, key);
        assert_eq!(v.palette[0], "minecraft:structure_void");
        assert_eq!(v.origin, [0, 0, 0]);
        assert_eq!(size_text(1500), "2 KB");
        assert_eq!(size_text(3 << 20), "3.0 MB");
    }
}
