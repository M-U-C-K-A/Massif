//! Apply: the graph written into the map, everything its Map Output
//! carries, in one step that can be undone. A thread of its own makes the
//! terrain window after window (never the whole map at once), then the
//! geometry; the app writes each window as it comes into one open step of
//! the map's history, then what the graph puts underground (caves, cave
//! biomes, volumes) and its villages (structures and roads), and closes the
//! step. A dialog shows how far it is; Cancel takes back what was written.

use super::panel::TerrainPanel;
use crate::doc::Document;
use crate::theme::{self, Palette};
use crate::widgets;
use massif_core::geometry::{Instances, Splines};
use massif_core::graph::{ClimateTable, Extent, GroundWrite, Outputs, Painting};
use massif_core::{BlockRect, TILE_SIZE, TerrainId, TilePos};
use massif_terrain::compile::Compiled;
use massif_terrain::engine::Engine;
use massif_terrain::kernel::Geo;
use massif_terrain::space::Rect;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError, sync_channel};
use std::sync::Arc;
use std::time::Instant;

/// The side of a window, in blocks (a whole number of tiles).
pub const WINDOW: u32 = 1024;

/// What the thread sends.
enum Msg {
    /// A window of the terrain made: where its samples lie, what they are,
    /// the tiles of the map it writes.
    Window(Extent, Box<Outputs>, Vec<TilePos>),
    /// The Map Output's structures and roads, once every window is sent.
    Geometry(Option<Instances>, Option<Splines>),
    Failed(String),
}

/// Where Apply is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Terrain,
    Underground,
    Villages,
}

/// An Apply under way.
pub struct Applying {
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    made: Arc<AtomicUsize>,
    pub windows: usize,
    pub written: usize,
    pub phase: Phase,
    started: Instant,
    engine: Arc<Engine>,
    compiled: Arc<Compiled>,
    /// The folder pack paths are read from.
    folder: PathBuf,
    revision: u64,
    geometry: Option<(Option<Instances>, Option<Splines>)>,
    /// What Apply has to say once done.
    notes: Vec<String>,
    /// Whether the graph makes heights (sculpted chunks on the old ground
    /// then go).
    heights: bool,
    /// The levels of rock written, by slot, made at the first window.
    levels: Option<Vec<usize>>,
    /// What went underground.
    under: Option<massif_terrain::underground::Applied>,
    /// The caves, cave biomes and volumes being worked out on a thread of
    /// their own, over the map as the ground now is.
    prepared: Option<Receiver<Result<massif_terrain::underground::Prepared, String>>>,
    /// The blocks edited by hand before anything was written, kept through
    /// the Apply (see [`massif_terrain::underground::HandEdits`]).
    kept: Option<massif_terrain::underground::HandEdits>,
}

/// What Apply did, to say.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Done {
    pub windows: usize,
    pub seconds: f64,
    pub caves: massif_terrain::underground::Applied,
    pub villages: usize,
    pub notes: Vec<String>,
}

impl Done {
    /// What Apply did, in a sentence.
    pub fn summary(&self) -> String {
        let mut text = t!("Applied the terrain graph in {s} s", s = format!("{:.1}", self.seconds));
        match self.caves.cave_layers {
            0 => {}
            1 => text.push_str(t!(", a cave layer")),
            n => text.push_str(&t!(", {n} cave layers", n = n)),
        }
        if self.caves.chunks > 0 {
            text.push_str(&t!(", {n} sculpted chunks", n = self.caves.chunks));
        }
        match self.villages {
            0 => {}
            1 => text.push_str(t!(", a village")),
            n => text.push_str(&t!(", {n} villages", n = n)),
        }
        if let Some(first) = self.notes.first() {
            text.push_str(&format!(" · {first}"));
        }
        text
    }
}

impl Applying {
    /// Starts writing `graph` (the document's) over `doc`'s map: the step
    /// of the map's history opens, the thread starts.
    pub fn start(engine: Arc<Engine>, graph: &massif_terrain::model::Graph, doc: &mut Document, env: Arc<dyn massif_terrain::kernel::Env>, revision: u64) -> Result<Self, String> {
        let space = massif_terrain::world::space_of(&doc.world, doc.dim, graph);
        let sim = space.size.min(4096);
        let (c, diags) = massif_terrain::compile::compile_in(graph, space, sim, env);
        if let Some(d) = diags.iter().find(|d| d.severity == massif_terrain::compile::Severity::Error) {
            return Err(d.to_string());
        }
        let Some(_) = c.map_output() else { return Err(t!("The graph has no active Map Output to apply").to_owned()) };
        let compiled = Arc::new(c);
        let heights = compiled.output("height").is_some();
        let folder = doc.path.as_ref().and_then(|p| p.parent().map(|d| d.to_path_buf())).unwrap_or_default();
        let cancel = Arc::new(AtomicBool::new(false));
        let made = Arc::new(AtomicUsize::new(0));
        // Windows aligned on the map's tiles, so that each tile is written
        // whole by one window; a terrain off the tiles is one window.
        let n = compiled.space.samples(0);
        let aligned = compiled.space.x0.rem_euclid(TILE_SIZE) == 0 && compiled.space.z0.rem_euclid(TILE_SIZE) == 0;
        let side = if aligned { WINDOW.min(n) } else { n };
        let per_row = n.div_ceil(side);
        let windows = (per_row * per_row) as usize;
        // Two windows waiting at most: memory stays at a few windows.
        let (tx, rx) = sync_channel::<Msg>(2);
        let kept = Some(massif_terrain::underground::HandEdits::capture(&compiled, doc.dimension()));
        doc.history.begin(t!("Apply the terrain graph"), doc.dim);
        let (e, c, stop, count) = (engine.clone(), compiled.clone(), cancel.clone(), made.clone());
        std::thread::Builder::new()
            .name("terrain apply".into())
            .spawn(move || work(e, c, side, per_row, tx, stop, count))
            .map_err(|e| e.to_string())?;
        Ok(Applying { rx, cancel, made, windows, written: 0, phase: Phase::Terrain, started: Instant::now(), engine, compiled, folder, revision, geometry: None, notes: Vec::new(), heights, levels: None, under: None, prepared: None, kept })
    }

    /// Stops it: what was written is taken back.
    pub fn cancel(self, doc: &mut Document) {
        self.cancel.store(true, Ordering::Relaxed);
        doc.history.abandon(&mut doc.world);
    }

    /// How far it is, from 0 to 1.
    pub fn progress(&self) -> f32 {
        let terrain = (self.written as f32 + 0.5 * self.made.load(Ordering::Relaxed).saturating_sub(self.written) as f32) / self.windows.max(1) as f32;
        match self.phase {
            Phase::Terrain => 0.85 * terrain.min(1.0),
            Phase::Underground => 0.88,
            Phase::Villages => 0.96,
        }
    }

    /// Writes what has come, for at most a few hundredths of a second:
    /// `Some` once it is all written (or failed: `Err`).
    pub fn step(&mut self, doc: &mut Document, blocks: &crate::blocks::BlockLibrary, lib: Option<&massif_structures::Library>) -> Option<Result<Done, String>> {
        let t0 = Instant::now();
        while self.phase == Phase::Terrain && t0.elapsed().as_millis() < 40 {
            match self.rx.try_recv() {
                Ok(Msg::Window(extent, outputs, tiles)) => {
                    self.write_window(doc, blocks, &extent, &outputs, &tiles);
                    self.written += 1;
                }
                Ok(Msg::Geometry(s, r)) => {
                    self.geometry = Some((s, r));
                    self.phase = Phase::Underground;
                    // What goes underground, worked out off the interface's
                    // thread on the map as its ground now is; written next.
                    let dim = doc.world.dims[doc.dim].snapshot();
                    let (engine, c, cancel) = (self.engine.clone(), self.compiled.clone(), self.cancel.clone());
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.prepared = Some(rx);
                    let _ = std::thread::Builder::new().name("terrain apply underground".into()).spawn(move || {
                        let stop = move || cancel.load(Ordering::Relaxed);
                        let _ = tx.send(massif_terrain::underground::prepare(&engine, &c, &dim, &stop).map_err(|e| e.to_string()));
                    });
                    // The dialog shows the next phase before it runs.
                    return None;
                }
                Ok(Msg::Failed(e)) => {
                    doc.history.abandon(&mut doc.world);
                    return Some(Err(e));
                }
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => {
                    doc.history.abandon(&mut doc.world);
                    return Some(Err(t!("Apply stopped").to_owned()));
                }
            }
        }
        match self.phase {
            Phase::Terrain => None,
            Phase::Underground => {
                // Caves, cave biomes and volumes, on the ground as written.
                let reg = doc.world.registry.clone();
                let dim = doc.dim;
                let cancel = self.cancel.clone();
                let stop = move || cancel.load(Ordering::Relaxed);
                let prepared = match self.prepared.as_ref().map(|rx| rx.try_recv()) {
                    Some(Err(TryRecvError::Empty)) => return None,
                    Some(Ok(Ok(p))) => Some(p),
                    Some(Ok(Err(e))) => {
                        doc.history.abandon(&mut doc.world);
                        return Some(Err(e));
                    }
                    // No thread: worked out while writing.
                    _ => None,
                };
                self.prepared = None;
                let kept = self.kept.take();
                let under = massif_terrain::underground::apply_full(&self.engine, &self.compiled, &reg, &mut doc.history.edit(&mut doc.world, 0), prepared.as_ref(), kept, &stop);
                match under {
                    Ok(u) => {
                        if u != Default::default() {
                            doc.caves_changed = true;
                        }
                        let _ = dim;
                        self.phase = Phase::Villages;
                        self.under = Some(u);
                        None
                    }
                    Err(e) => {
                        doc.history.abandon(&mut doc.world);
                        Some(Err(e.to_string()))
                    }
                }
            }
            Phase::Villages => {
                let villages = self.write_villages(doc, lib);
                doc.commit();
                doc.clamp_active_level();
                let caves = self.under.take().unwrap_or_default();
                let notes: Vec<String> = std::mem::take(&mut self.notes);
                Some(Ok(Done { windows: self.written, seconds: self.started.elapsed().as_secs_f64(), caves, villages, notes }))
            }
        }
    }

    /// One window of the terrain into the open step: its ground, rock,
    /// biomes, the old sculpted chunks over it gone.
    fn write_window(&mut self, doc: &mut Document, blocks: &crate::blocks::BlockLibrary, extent: &Extent, outputs: &Outputs, tiles: &[TilePos]) {
        let palette = match &outputs.colors {
            Some((colors, set)) => crate::graph_editor::block_palette(doc, blocks, *set, colors),
            None => Vec::new(),
        };
        if self.levels.is_none() {
            self.levels = Some(crate::graph_editor::layer_levels(doc, &outputs.layers, false));
        }
        let seed = doc.world.meta.seed as u64;
        let sea = doc.world.meta.sea_level;
        let grass = doc.world.registry.terrain_by_name("Grass").unwrap_or(TerrainId::ROCK);
        let climate = ClimateTable::new(&doc.world.registry);
        let paint = Painting { palette: &palette, climate: Some(&climate) };
        let write = GroundWrite { tiles, create: true, map: None, sea, blank: grass };
        {
            let edit = &mut doc.history.edit(&mut doc.world, 0);
            massif_core::graph::apply(edit, extent, outputs, seed, &paint, &write);
            if self.heights {
                edit.clear_voxels(tiles);
            }
            if outputs.biomes.is_some() {
                massif_core::graph::apply_biomes(edit, extent, outputs, &climate, tiles);
            }
        }
        let levels = self.levels.clone().unwrap_or_default();
        for (layer, index) in outputs.layers.iter().zip(&levels) {
            let layer_palette = match &layer.colors {
                Some((colors, set)) => crate::graph_editor::block_palette(doc, blocks, *set, colors),
                None => Vec::new(),
            };
            let edit = &mut doc.history.edit(&mut doc.world, *index);
            massif_core::graph::apply_layer(edit, extent, layer, seed, &layer_palette, grass, tiles);
        }
    }

    /// The graph's villages: laid on the ground as written, the graph's
    /// earlier ones replaced, those placed by hand kept. Gives how many.
    fn write_villages(&mut self, doc: &mut Document, lib: Option<&massif_structures::Library>) -> usize {
        let Some((Some(structures), roads)) = self.geometry.take() else {
            // No structures: those the graph laid before go.
            let edit = &mut doc.history.edit(&mut doc.world, 0);
            massif_structures::village::graph::apply_graph(edit, &[]);
            return 0;
        };
        let roads = roads.unwrap_or_default();
        let packs: Vec<PathBuf> = self
            .compiled
            .nodes
            .iter()
            .filter(|n| n.spec.id.starts_with("village."))
            .filter_map(|n| n.spec.param("pack").map(|_| n.params.text("pack").to_owned()))
            .filter(|p| !p.is_empty())
            .map(|p| {
                let p = PathBuf::from(p);
                if p.is_relative() { self.folder.join(p) } else { p }
            })
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect();
        let (themes, unread) = massif_structures::village::graph::themes_of(&packs);
        self.notes.extend(unread);
        let first = massif_core::structures::next_id(&doc.dimension().structures);
        let seed = doc.world.meta.seed as u64;
        let laid = {
            let ground = massif_structures::ground::MapGround::new(&doc.world, doc.dim);
            massif_structures::village::graph::lay_graph(lib, &massif_structures::village::lay::DiskPlans::default(), &ground, &structures, &roads, &themes, seed, first, self.revision)
        };
        match laid {
            Ok((built, notes)) => {
                self.notes.extend(notes);
                let edit = &mut doc.history.edit(&mut doc.world, 0);
                massif_structures::village::graph::apply_graph(edit, &built);
                built.len()
            }
            Err(e) => {
                self.notes.push(e);
                0
            }
        }
    }
}

/// The thread: each window's terrain, then the geometry.
fn work(engine: Arc<Engine>, c: Arc<Compiled>, side: u32, per_row: u32, tx: SyncSender<Msg>, stop: Arc<AtomicBool>, made: Arc<AtomicUsize>) {
    let cancel = || stop.load(Ordering::Relaxed);
    let n = c.space.samples(0);
    for wz in 0..per_row {
        for wx in 0..per_row {
            if cancel() {
                return;
            }
            // The last windows are moved back inside the terrain (square
            // windows, overlapping their neighbours rather than past the
            // edge); each tile is written by the window it starts in.
            let x = (wx * side).min(n - side);
            let z = (wz * side).min(n - side);
            let rect = Rect::new(x as i32, z as i32, side, side);
            let out = match massif_terrain::world::outputs_in(&engine, &c, 0, rect, &cancel) {
                Ok((extent, out)) => (extent, out),
                Err(e) => {
                    let _ = tx.send(Msg::Failed(e.to_string()));
                    return;
                }
            };
            let own = BlockRect::new(c.space.x0 + (wx * side) as i32, c.space.z0 + (wz * side) as i32, c.space.x0 + ((wx + 1) * side).min(n) as i32, c.space.z0 + ((wz + 1) * side).min(n) as i32);
            let tiles: Vec<TilePos> = own.tiles().filter(|t| {
                let (bx, bz) = (t.x * TILE_SIZE, t.z * TILE_SIZE);
                bx >= own.x0 && bz >= own.z0 && bx < own.x1 && bz < own.z1
            }).collect();
            made.fetch_add(1, Ordering::Relaxed);
            if tx.send(Msg::Window(out.0, Box::new(out.1), tiles)).is_err() {
                return;
            }
        }
    }
    // The Map Output's geometry, made whole.
    let geo = |name: &str| -> Result<Option<Geo>, String> {
        match c.output(name) {
            Some(src) => engine.geometry(&c, src, &cancel).map(|g| Some((*g).clone())).map_err(|e| e.to_string()),
            None => Ok(None),
        }
    };
    let structures = match geo("structures") {
        Ok(Some(Geo::Instances(i))) => Some(i),
        Ok(_) => None,
        Err(e) => {
            let _ = tx.send(Msg::Failed(e));
            return;
        }
    };
    let roads = match geo("roads") {
        Ok(Some(Geo::Splines(s))) => Some(s),
        Ok(_) => None,
        Err(e) => {
            let _ = tx.send(Msg::Failed(e));
            return;
        }
    };
    let _ = tx.send(Msg::Geometry(structures, roads));
}

/// The dialog over the window while Apply runs: how far, Cancel.
pub fn dialog(a: &Applying, ctx: &egui::Context) -> bool {
    let mut cancel = false;
    crate::app::modal(ctx, "terrain apply").show(ctx, |ui| {
        let pal = Palette::of(ui);
        ui.set_width(380.0);
        ui.label(egui::RichText::new(t!("Applying the terrain graph")).size(15.0).strong());
        ui.add_space(4.0);
        let what = match a.phase {
            Phase::Terrain => t!("The terrain, window {k} of {n}", k = (a.written + 1).min(a.windows), n = a.windows),
            Phase::Underground => t!("Caves, cave biomes and volumes").to_owned(),
            Phase::Villages => t!("Villages and roads").to_owned(),
        };
        ui.label(egui::RichText::new(what).size(12.0).color(pal.weak));
        ui.add_space(10.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 6.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, theme::PILL, pal.well());
        let mut done = rect;
        done.set_width(rect.width() * a.progress().clamp(0.0, 1.0));
        ui.painter().rect_filled(done, theme::PILL, pal.accent);
        ui.add_space(8.0);
        ui.label(egui::RichText::new(t!("{s} s · one step you can undo", s = format!("{:.0}", a.started.elapsed().as_secs_f64()))).size(11.0).color(pal.weak));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if widgets::button(ui, t!("Cancel")).clicked() {
                cancel = true;
            }
        });
    });
    cancel
}

impl TerrainPanel {
    /// What Apply has to do this frame, from the app: started when asked,
    /// written as it comes, said once done. Whether the map changed.
    pub fn tick_apply(&mut self, ctx: &egui::Context, doc: &mut Document, blocks: &crate::blocks::BlockLibrary, lib: Option<&massif_structures::Library>) -> Option<Result<Done, String>> {
        if self.apply_asked && self.applying.is_none() {
            self.apply_asked = false;
            let graph = self.document();
            let engine = self.engine().cloned()?;
            match Applying::start(engine, &graph, doc, self.env.clone(), self.revision) {
                Ok(a) => self.applying = Some(a),
                Err(e) => return Some(Err(e)),
            }
        }
        let a = self.applying.as_mut()?;
        if dialog(a, ctx) {
            let a = self.applying.take().expect("applying");
            a.cancel(doc);
            return Some(Err(t!("Apply cancelled: the map is as it was").to_owned()));
        }
        let out = a.step(doc, blocks, lib);
        // Keeps going while the thread works.
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        if out.is_some() {
            self.applying = None;
        }
        out
    }
}
