//! Work that runs off the UI thread: generating, opening, saving, exporting.
//!
//! Every job works on data it owns (a freshly generated world, a file, or a
//! cold snapshot of the document), so the editor stays responsive and keeps
//! editing while a save or an export runs.

use massif_anvil::{ExportOptions, ExportStats, Progress};
use massif_legacy::{ImportProgress, ImportReport};
use massif_core::World;
use massif_core::generate::{GenParams, generate};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Instant;

pub enum Outcome {
    /// A world to open in place of the current one.
    World {
        world: World,
        path: Option<PathBuf>,
        /// What to say besides (a real place's data that could not be had).
        note: Option<String>,
    },
    /// A save point of the map, made on a copy of it (its picture drawn).
    SavePoint(massif_core::versions::SavePoint),
    Saved {
        path: PathBuf,
        revision: u64,
        bytes: u64,
    },
    Exported {
        dir: PathBuf,
        stats: ExportStats,
    },
    /// A backup left by a run that closed without saving, read back (see
    /// `autosave`).
    Recovered {
        world: World,
        backup: crate::autosave::Backup,
    },
    /// A WorldPainter world converted to Massif.
    Imported {
        world: World,
        report: Box<ImportReport>,
        source: PathBuf,
    },
    /// An automatic tool over the whole map, worked out on a copy of it: the
    /// tiles it changed, to put in place in one undoable step.
    Auto {
        biomes: bool,
        tiles: Vec<(massif_core::TilePos, massif_core::level::Blob)>,
        columns: usize,
        /// The document's revision and dimension it was worked out from.
        revision: u64,
        dim: usize,
    },
    /// A pattern of terrains painted over the whole of a level, worked out
    /// on a copy of the map: the tiles it changed, to put in place in one
    /// undoable step.
    Pattern {
        tiles: Vec<(massif_core::TilePos, massif_core::level::Blob)>,
        columns: usize,
        /// The document's revision, dimension and level it was worked out
        /// from.
        revision: u64,
        dim: usize,
        level: usize,
    },
    /// A filter of the graph run over the whole of the ground, worked out
    /// on a copy of the map: the tiles it changed, to put in place in one
    /// undoable step.
    Reshaped {
        label: &'static str,
        tiles: Vec<(massif_core::TilePos, massif_core::level::Blob)>,
        columns: usize,
        /// The document's revision and dimension it was worked out from.
        revision: u64,
        dim: usize,
    },
    /// Ground covers of layer blocks turned into the ground's own layers
    /// (see `massif_core::surface_layers`), to put in place in one step.
    SurfaceLayers {
        grounded: Box<massif_core::surface_layers::Grounded>,
        /// The document's revision and dimension it was worked out from.
        revision: u64,
        dim: usize,
    },
    /// The relief of a real place written into a copy of the map: the
    /// tiles it changed, to put in place in one undoable step.
    RealTerrain {
        tiles: Vec<(massif_core::TilePos, massif_core::level::Blob)>,
        columns: usize,
        /// The document's revision and dimension it was worked out from.
        revision: u64,
        dim: usize,
        note: Option<String>,
    },
    /// The same, the map made another size first: the whole dimension, to
    /// put in place of the map's in one undoable step.
    /// The map's palette switched too (`palette`), in the same step; the
    /// map made another size or not (`resized`).
    RealMap {
        dimension: Box<massif_core::world::Dimension>,
        palette: Option<massif_core::materials::Palette>,
        resized: bool,
        columns: usize,
        revision: u64,
        dim: usize,
        note: Option<String>,
    },
    /// Images to import as masks, each read once through, and the map's
    /// picture with the blocks it covers (when asked for).
    MaskRead {
        images: Vec<(PathBuf, Arc<massif_core::mask::ImageInfo>)>,
        map: Option<(egui::ColorImage, [i32; 4])>,
        /// The map's columns, for the mask window's preview.
        columns: Option<Arc<massif_core::mask::Columns>>,
    },
    /// A region of the map written as images: its folder and files, the
    /// version made of the map for it to come back to (when one was
    /// needed), and whether `zone.json` was written.
    RegionExported {
        dir: PathBuf,
        files: Vec<PathBuf>,
        point: Option<massif_core::versions::SavePoint>,
        zone: bool,
    },
    /// A region of the map written as a `.massif` of its own: where, its
    /// tiles (wide, deep), and the version made of the map for it to come
    /// back to (when one was needed).
    RegionMapWritten {
        path: PathBuf,
        tiles: (i32, i32),
        point: Option<massif_core::versions::SavePoint>,
    },
    /// A contribution read and classified against the map, to review.
    Review(Box<crate::collab::ReviewReady>),
    /// The map published into its shared folder: the version, and the file.
    Published {
        name: String,
        path: PathBuf,
    },
    /// A contribution written: where, how many tiles, how many bytes.
    PatchWritten {
        path: PathBuf,
        tiles: usize,
        bytes: u64,
    },
    /// One of the game's structures assembled on a copy of the map, to
    /// place in one undoable step, where it was clicked (in the place of
    /// structure `replace`, drawn again).
    Structure {
        built: Box<massif_structures::vanilla::Built>,
        at: (i32, i32),
        replace: Option<u32>,
        /// The dimension it was worked out on.
        dim: usize,
        /// How to make it again.
        recipe: massif_structures::placed::Recipe,
    },
    /// A village built on a copy of the map, to place in one undoable
    /// step, in place of `replace` if given.
    Village {
        built: Box<massif_structures::vanilla::Built>,
        dim: usize,
        replace: Option<u32>,
        recipe: massif_structures::placed::Recipe,
    },
    Failed(String),
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind {
    Generate,
    Open,
    Import,
    Save,
    Export,
    Auto,
    SavePoint,
    /// A structure being assembled: editing goes on meanwhile.
    Structure,
}

impl JobKind {
    /// Jobs that replace the document block editing until they finish.
    pub fn blocks_editing(self) -> bool {
        matches!(self, JobKind::Generate | JobKind::Open | JobKind::Import | JobKind::Auto)
    }
}

/// Progress a running job reports, and how to cancel it.
pub trait JobProgress: Send + Sync {
    fn fraction(&self) -> f32;
    fn cancel(&self);
}

impl JobProgress for Progress {
    fn fraction(&self) -> f32 {
        Progress::fraction(self)
    }
    fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl JobProgress for massif_core::auto::Steps {
    fn fraction(&self) -> f32 {
        massif_core::auto::Steps::fraction(self)
    }
    fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl JobProgress for ImportProgress {
    fn fraction(&self) -> f32 {
        ImportProgress::fraction(self)
    }
    fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

/// How far a job of several steps is: tiles fetched, then a pass or two
/// over the map.
#[derive(Default)]
pub struct Phased {
    /// The step under way, of `steps`.
    pub step: std::sync::atomic::AtomicUsize,
    pub steps: std::sync::atomic::AtomicUsize,
    pub fetched: std::sync::atomic::AtomicUsize,
    pub tiles: std::sync::atomic::AtomicUsize,
    pub pass: massif_core::auto::Steps,
}

impl JobProgress for Phased {
    fn fraction(&self) -> f32 {
        use std::sync::atomic::Ordering::Relaxed;
        let steps = self.steps.load(Relaxed).max(1) as f32;
        let step = self.step.load(Relaxed);
        // Fetching weighs as much as the rest together.
        let within = if step == 0 { self.fetched.load(Relaxed) as f32 / self.tiles.load(Relaxed).max(1) as f32 } else { self.pass.fraction() };
        let (start, width) = if step == 0 { (0.0, 0.5) } else { (0.5 + 0.5 * (step - 1) as f32 / (steps - 1.0).max(1.0), 0.5 / (steps - 1.0).max(1.0)) };
        (start + width * within).clamp(0.0, 1.0)
    }
    fn cancel(&self) {
        self.pass.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct Job {
    pub kind: JobKind,
    pub label: String,
    pub started: Instant,
    pub progress: Option<Arc<dyn JobProgress>>,
    /// The map it works on (its generation), for a job whose outcome is
    /// for that map alone: see [`Job::on`].
    pub map: Option<u64>,
    rx: Receiver<Outcome>,
}

impl Job {
    fn spawn(kind: JobKind, label: String, progress: Option<Arc<dyn JobProgress>>, ctx: &egui::Context, work: impl FnOnce() -> Outcome + Send + 'static) -> Job {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        // A system that refuses the thread (memory short, too many of them)
        // drops the work and its sender: the job fails when polled.
        let spawned = std::thread::Builder::new()
            .name(format!("massif {kind:?}"))
            // Reading Java object graphs recurses; give it room.
            .stack_size(64 << 20)
            .spawn(move || {
                // A panic in the work fails the job, its message said; the
                // map open is not touched by a job until it is done.
                let outcome = crate::startup::catch(work).unwrap_or_else(|e| Outcome::Failed(tr!("The task stopped on an error: {e}. The map is as it was.", e)));
                let _ = tx.send(outcome);
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            log::warn!("no thread for “{label}”: {e}");
        }
        Job {
            kind,
            label,
            started: Instant::now(),
            progress,
            map: None,
            rx,
        }
    }

    /// For the map of `generation` alone: its outcome is not put into
    /// another map opened meanwhile.
    pub fn on(mut self, generation: u64) -> Job {
        self.map = Some(generation);
        self
    }

    /// The outcome, once the job is over.
    pub fn poll(&self) -> Option<Outcome> {
        match self.rx.try_recv() {
            Ok(outcome) => Some(outcome),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Outcome::Failed("the worker thread stopped unexpectedly".into())),
        }
    }

    pub fn cancel(&self) {
        if let Some(p) = &self.progress {
            p.cancel();
        }
    }

    pub fn generate(ctx: &egui::Context, params: GenParams) -> Job {
        let label = tr!("Generating “{name}”", name = params.name);
        Job::spawn(JobKind::Generate, label, None, ctx, move || Outcome::World {
            note: None,
            world: generate(&params),
            path: None,
        })
    }

    /// A job whose outcome is ready: what a test makes a task bring.
    #[cfg(test)]
    pub fn ready(ctx: &egui::Context, kind: JobKind, outcome: Outcome) -> Job {
        Job::spawn(kind, String::new(), None, ctx, move || outcome)
    }

    /// Convert a WorldPainter `.world` file.
    pub fn import(ctx: &egui::Context, path: PathBuf) -> Job {
        let progress = Arc::new(ImportProgress::default());
        let label = tr!("Importing {file}", file = display_name(&path));
        let p = progress.clone();
        Job::spawn(JobKind::Import, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            match massif_legacy::import_world(&path, Some(&p)) {
                Ok((world, report)) => Outcome::Imported {
                    world,
                    report: Box::new(report),
                    source: path,
                },
                Err(massif_legacy::ImportError::Cancelled) => Outcome::Cancelled,
                Err(e) => Outcome::Failed(tr!("Could not import {file}: {e}", file = display_name(&path), e)),
            }
        })
    }

    /// Import a Minecraft world.
    pub fn import_minecraft(ctx: &egui::Context, world: massif_legacy::minecraft::McWorld, opts: massif_legacy::minecraft::Options) -> Job {
        let progress = Arc::new(ImportProgress::default());
        let label = tr!("Importing {file}", file = world.name.clone());
        let p = progress.clone();
        Job::spawn(JobKind::Import, label, Some(progress as Arc<dyn JobProgress>), ctx, move || match massif_legacy::minecraft::import(&world, &opts, Some(&p)) {
            Ok((imported, report)) => Outcome::Imported { world: imported, report: Box::new(report), source: world.root.clone() },
            Err(massif_legacy::ImportError::Cancelled) => Outcome::Cancelled,
            Err(e) => Outcome::Failed(tr!("Could not import {file}: {e}", file = world.name.clone(), e)),
        })
    }

    /// Open a Massif world, or import a WorldPainter one.
    pub fn open(ctx: &egui::Context, path: PathBuf) -> Job {
        if is_worldpainter(&path) {
            return Job::import(ctx, path);
        }
        let label = tr!("Opening {file}", file = display_name(&path));
        Job::spawn(JobKind::Open, label, None, ctx, move || match massif_format::load(&path) {
            Ok(world) => Outcome::World { world, path: Some(path), note: None },
            Err(e) => Outcome::Failed(tr!("Could not open {file}: {e}", file = display_name(&path), e)),
        })
    }

    /// Read back a backup left by a run that closed without saving.
    pub fn recover(ctx: &egui::Context, backup: crate::autosave::Backup) -> Job {
        let label = tr!("Restoring “{name}”", name = backup.info.name);
        Job::spawn(JobKind::Open, label, None, ctx, move || match massif_format::load(&backup.file) {
            Ok(world) => Outcome::Recovered { world, backup },
            Err(e) => Outcome::Failed(tr!("Could not restore “{name}”: {e}", name = backup.info.name, e)),
        })
    }

    /// A save point named `name` of a cold copy of the map, its picture
    /// drawn away from the frames.
    /// `other`: the version it merges (its second parent); `author`: who
    /// made it; `branch`: the branch it is made on (empty: the map's line).
    pub fn merge_point(ctx: &egui::Context, snapshot: World, name: String, other: Option<massif_core::versions::Id>, author: String, branch: String) -> Job {
        let label = tr!("Saving the point “{name}”", name);
        Job::spawn(JobKind::SavePoint, label, None, ctx, move || {
            let mut point = massif_core::versions::SavePoint::of(snapshot, name, massif_core::versions::now());
            point.parents.extend(other.filter(|o| !point.parents.contains(o)));
            point.author = author;
            point.branch = branch;
            Outcome::SavePoint(point)
        })
    }

    /// Save a cold snapshot taken at `revision`.
    pub fn save(ctx: &egui::Context, snapshot: World, path: PathBuf, revision: u64) -> Job {
        let label = tr!("Saving {file}", file = display_name(&path));
        Job::spawn(JobKind::Save, label, None, ctx, move || match massif_format::save(&snapshot, &path) {
            Ok(stats) => Outcome::Saved {
                path,
                revision,
                bytes: stats.file_len,
            },
            Err(e) => Outcome::Failed(tr!("Could not save {file}. {why}", file = display_name(&path), why = massif_format::describe(&e, &path))),
        })
    }

    /// Automatic biomes (`biomes`) or paint over the whole of dimension
    /// `dim` of a snapshot of the document at `revision`.
    /// Turn the ground covers `chosen` into the ground's own layers, over
    /// a copy of the map.
    pub fn surface_layers(ctx: &egui::Context, snapshot: World, dim: usize, chosen: Vec<massif_core::surface_layers::SurfaceCover>, revision: u64) -> Job {
        Job::spawn(JobKind::Auto, tr!("Covers of layers made the ground's own").to_owned(), None, ctx, move || {
            let grounded = massif_core::surface_layers::to_ground(&snapshot.dims[dim], &snapshot.meta, &snapshot.registry, &chosen);
            Outcome::SurfaceLayers { grounded: Box::new(grounded), revision, dim }
        })
    }

    pub fn auto(ctx: &egui::Context, mut snapshot: World, dim: usize, rules: massif_core::auto::AutoRules, biomes: bool, revision: u64) -> Job {
        use massif_core::auto::{Reader, Steps, base_terrain, biomes_over_the_map_with, paint_over_the_map_with};
        let progress = Arc::new(Steps::default());
        let label = if biomes { tr!("Automatic biomes over the whole map") } else { tr!("Automatic paint over the whole map") };
        let p = progress.clone();
        Job::spawn(JobKind::Auto, label.to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let base = base_terrain(&snapshot.dims[dim], &snapshot.registry);
            let reader = Reader::new(&rules, &snapshot.registry, snapshot.meta.sea_level).with_base(base);
            let mut history = massif_core::history::History::new(1);
            history.begin(tr!("automatic"), dim);
            let changes = {
                let mut edit = history.edit(&mut snapshot, 0);
                if biomes {
                    biomes_over_the_map_with(&mut edit, &reader, &p)
                } else {
                    paint_over_the_map_with(&mut edit, &reader, &p)
                }
            };
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let level = &mut snapshot.dims[dim].levels[0];
            level.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Auto {
                biomes,
                tiles,
                columns: changes.columns,
                revision,
                dim,
            }
        })
    }

    /// A pattern of terrains over the whole of level `level` of dimension
    /// `dim` of a snapshot of the document at `revision`, where `filter`
    /// lets it.
    pub fn pattern(ctx: &egui::Context, mut snapshot: World, dim: usize, level: usize, settings: crate::tools::PatternSettings, filter: massif_core::filter::Filter, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        Job::spawn(JobKind::Auto, tr!("Noise paint over the whole map").to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let Some(sampler) = settings.sampler(&snapshot.registry) else {
                return Outcome::Failed(tr!("Noise paint: none of its terrains is in this map").to_owned());
            };
            let mut history = massif_core::history::History::new(1);
            history.begin(tr!("noise paint"), dim);
            let changes = massif_core::pattern::paint_over_the_map(&mut history.edit(&mut snapshot, level), &sampler, Some(&filter), &p);
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let edited = &mut snapshot.dims[dim].levels[level];
            edited.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| edited.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Pattern {
                tiles,
                columns: changes.columns,
                revision,
                dim,
                level,
            }
        })
    }

    /// `op` over the ground of the whole of dimension `dim` of a snapshot of
    /// the document at `revision`; `bed`, the terrain of the beds rivers dig.
    pub fn reshape(ctx: &egui::Context, mut snapshot: World, dim: usize, op: massif_core::graph::Reshape, bed: Option<massif_core::registry::TerrainId>, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = crate::i18n::tr_name(op.label());
        Job::spawn(JobKind::Auto, tr!("{label} over the whole map", label), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let sea = snapshot.meta.sea_level;
            let mut history = massif_core::history::History::new(1);
            history.begin(label, dim);
            // Sand in the shallows, wherever the beds are painted.
            let shore = bed.and_then(|_| snapshot.registry.terrain_by_name("Sand"));
            let changes = massif_core::graph::reshape_ground(&mut history.edit(&mut snapshot, 0), &op, sea, bed, shore, &p);
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let level = &mut snapshot.dims[dim].levels[0];
            level.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Reshaped {
                label,
                tiles,
                columns: changes.columns,
                revision,
                dim,
            }
        })
    }

    /// Every column of terrain `from` of the map `snapshot` (its dimension
    /// `dim`, its selection) painted with `to`: one step to undo.
    pub fn replace_terrain(ctx: &egui::Context, mut snapshot: World, dim: usize, from: u16, to: u16, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = tr!("Replace terrain");
        Job::spawn(JobKind::Auto, label.to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let mut history = massif_core::history::History::new(1);
            history.begin(label, dim);
            let changes = massif_core::auto::replace_terrain(&mut history.edit(&mut snapshot, 0), from, to, &p);
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let level = &mut snapshot.dims[dim].levels[0];
            level.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Reshaped { label, tiles, columns: changes.columns, revision, dim }
        })
    }

    /// Snow laid over the map `snapshot` (its dimension `dim`) as `fall`
    /// says, in terrain `snow`: one step to undo.
    pub fn snowfall(ctx: &egui::Context, mut snapshot: World, dim: usize, fall: massif_core::graph::Snowfall, snow: massif_core::registry::TerrainId, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = tr!("Snowfall");
        Job::spawn(JobKind::Auto, tr!("Snowfall over the whole map").to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let Some(found) = massif_core::graph::snow_over(&snapshot.dims[dim], &fall, massif_core::graph::SNOW_MAX, &p.cancel) else {
                return Outcome::Cancelled;
            };
            let covers = massif_core::graph::snow_covers(&snapshot.registry);
            let mut history = massif_core::history::History::new(1);
            history.begin(label, dim);
            let changes = massif_core::graph::lay_snow(&mut history.edit(&mut snapshot, 0), &found, &fall, snow, &covers, &p);
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let level = &mut snapshot.dims[dim].levels[0];
            level.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Reshaped {
                label,
                tiles,
                columns: changes.columns,
                revision,
                dim,
            }
        })
    }

    /// Read the images at `paths` once through for Import mask, and draw
    /// the picture of the map `snapshot`, when given, to show where they
    /// land.
    pub fn read_mask(ctx: &egui::Context, paths: Vec<PathBuf>, snapshot: Option<World>, dim: usize) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = match paths.as_slice() {
            [one] => tr!("Reading “{name}”", name = one.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned())),
            _ => tr!("Reading {n} images", n = paths.len()),
        };
        Job::spawn(JobKind::Auto, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let (read, map, columns) = std::thread::scope(|s| {
                let columns = s.spawn(|| {
                    let d = snapshot.as_ref()?.dims.get(dim)?;
                    let (lo, hi) = d.bounds()?;
                    let t = massif_core::TILE_SIZE;
                    Some(Arc::new(massif_core::mask::columns(d.ground(), [lo.x * t, lo.z * t, (hi.x + 1) * t, (hi.z + 1) * t], 512)))
                });
                let picture = s.spawn(|| {
                    let snapshot = snapshot.as_ref()?;
                    let (lo, hi) = snapshot.dims.first()?.bounds()?;
                    let t = massif_core::TILE_SIZE;
                    let img = massif_core::thumbnail::render(snapshot, 384)?;
                    Some((egui::ColorImage::from_rgb([img.width() as usize, img.height() as usize], img.as_raw()), [lo.x * t, lo.z * t, (hi.x + 1) * t, (hi.z + 1) * t]))
                });
                let mut images = Vec::new();
                for path in &paths {
                    match massif_core::mask::survey(path, &p) {
                        Ok(info) => images.push((path.clone(), Arc::new(info))),
                        Err(e) => {
                            let name = path.file_name().map_or_else(String::new, |n| n.to_string_lossy().into_owned());
                            return (Err(format!("{name}: {e}")), None, None);
                        }
                    }
                }
                (Ok(images), picture.join().ok().flatten(), columns.join().ok().flatten())
            });
            match read {
                Ok(images) => Outcome::MaskRead { images, map, columns },
                Err(_) if p.cancel.load(std::sync::atomic::Ordering::Relaxed) => Outcome::Cancelled,
                Err(e) => Outcome::Failed(tr!("That image cannot be read: {e}", e)),
            }
        })
    }

    /// The images painted onto dimension `dim` of a snapshot of the document
    /// at `revision`, each as its settings say, as one step to undo.
    pub fn import_mask(ctx: &egui::Context, mut snapshot: World, dim: usize, images: Vec<(PathBuf, Arc<massif_core::mask::ImageInfo>, massif_core::mask::MaskSettings)>, locks: massif_core::registry::Locks, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = if images.len() > 1 { tr!("Importing {n} masks", n = images.len()) } else { tr!("Importing the mask").to_owned() };
        Job::spawn(JobKind::Auto, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let label = tr!("Import mask");
            let mut history = massif_core::history::History::new(1);
            history.locks = locks;
            history.begin(label, dim);
            let changes = match massif_core::mask::apply_all(&mut history.edit(&mut snapshot, 0), &images, &p) {
                Ok(c) => c,
                Err(e) => return Outcome::Failed(tr!("Import mask: {e}", e)),
            };
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let level = &mut snapshot.dims[dim].levels[0];
            level.encode_blobs(&changes.tiles);
            let tiles = changes.tiles.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
            Outcome::Reshaped {
                label,
                tiles,
                columns: changes.columns,
                revision,
                dim,
            }
        })
    }

    /// The part `rect` of dimension `dim` of a snapshot written into the
    /// folder `dir` as images (see `massif_core::mask::export_region`).
    /// With `review`, the version the map stands on (if any) and the name of
    /// one to make when the map changed since: the region is written with a
    /// `zone.json` naming that version, to come back as a contribution.
    pub fn export_region(ctx: &egui::Context, mut snapshot: World, dim: usize, rect: massif_core::BlockRect, dir: PathBuf, parts: massif_core::mask::RegionParts, review: Option<(Option<massif_core::versions::SavePoint>, String)>) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        Job::spawn(JobKind::Export, tr!("Exporting the region").to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let files = match massif_core::mask::export_region(&snapshot.dims[dim], &snapshot.registry, rect, &dir, &parts, &p) {
                Ok(files) => files,
                Err(_) if p.cancel.load(std::sync::atomic::Ordering::Relaxed) => return Outcome::Cancelled,
                Err(e) => return Outcome::Failed(tr!("The region could not be exported: {e}", e)),
            };
            let Some((head, name)) = review else {
                return Outcome::RegionExported { dir, files, point: None, zone: false };
            };
            // The version the zone comes back to: the one the map stands on
            // if it is as it was then, a new one otherwise.
            let (base, point) = match head {
                Some(h) if massif_core::versions::unchanged_since(&snapshot, &h) => (h, None),
                _ => {
                    let copy = massif_core::versions::SavePoint::copy_of(&mut snapshot);
                    let point = massif_core::versions::SavePoint::of(copy, name, massif_core::versions::now());
                    (point.clone(), Some(point))
                }
            };
            let zone = massif_core::patch::Zone {
                format: massif_core::patch::Zone::FORMAT.into(),
                map: snapshot.map_id(),
                map_name: snapshot.meta.name.clone(),
                base: massif_core::patch::VersionRef { id: base.id, name: base.name.clone() },
                dim,
                rect: [rect.x0, rect.z0, rect.x1, rect.z1],
                created: massif_core::versions::now(),
            };
            match zone.write(&dir) {
                Ok(()) => Outcome::RegionExported { dir, files, point, zone: true },
                Err(e) => Outcome::Failed(tr!("The region was exported, but its zone.json could not be written: {e}", e)),
            }
        })
    }

    /// The images of a zone brought back from another tool, painted onto
    /// the version it was exported from (`base`) as their settings say, and
    /// what they change classified against the map as it is (`map`, a
    /// snapshot at `revision`).
    #[allow(clippy::too_many_arguments)]
    pub fn review_zone(ctx: &egui::Context, base: massif_core::versions::SavePoint, map: World, dim: usize, images: Vec<(PathBuf, Arc<massif_core::mask::ImageInfo>, massif_core::mask::MaskSettings)>, locks: massif_core::registry::Locks, zone: massif_core::patch::Zone, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        Job::spawn(JobKind::Auto, tr!("Comparing the zone with “{name}”", name = base.name), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let mut theirs = (*base.world).clone();
            theirs.meta.map_id = Some(zone.map);
            if let Some(d) = theirs.dims.get_mut(dim) {
                d.selection = None;
            } else {
                return Outcome::Failed(tr!("The zone's dimension is not in that version").to_owned());
            }
            let mut history = massif_core::history::History::new(1);
            history.locks = locks;
            history.begin(tr!("Import mask"), dim);
            let changes = match massif_core::mask::apply_all(&mut history.edit(&mut theirs, 0), &images, &p) {
                Ok(c) => c,
                Err(e) => return Outcome::Failed(tr!("Import mask: {e}", e)),
            };
            drop(history);
            if changes.cancelled {
                return Outcome::Cancelled;
            }
            let theirs = theirs.snapshot();
            let source = images.first().and_then(|(path, _, _)| path.parent()).and_then(|d| d.file_name()).map_or_else(String::new, |d| d.to_string_lossy().into_owned());
            let mut patch = match massif_core::patch::Patch::since(&base, &theirs, massif_core::patch::Kind::External, "", "", massif_core::versions::now()) {
                Ok(patch) => patch,
                Err(e) => return Outcome::Failed(e.to_string()),
            };
            patch.manifest.zone = Some(zone.rect);
            // A tile no one touched comes back through a 16-bit heightmap a
            // hair off: not a change.
            patch.drop_noise(&base.world, 0.05);
            Outcome::Review(Box::new(crate::collab::ReviewReady::of(patch, &map, source, true, revision)))
        })
    }

    /// The map (`snapshot`) published as `version` into its shared folder.
    pub fn publish(ctx: &egui::Context, mut depot: massif_format::depot::FolderDepot, snapshot: World, version: massif_core::versions::SavePoint) -> Job {
        use massif_format::depot::Depot;
        Job::spawn(JobKind::Save, tr!("Publishing the map").to_owned(), None, ctx, move || match depot.publish(&snapshot, &version) {
            Ok(()) => Outcome::Published {
                name: version.name,
                path: depot.published_map().unwrap_or_else(|| depot.root().to_path_buf()),
            },
            Err(e) => Outcome::Failed(tr!("The map could not be published: {e}", e)),
        })
    }

    /// The contribution of the map (`snapshot`) since version `base`, put
    /// into its shared folder.
    pub fn send_patch(ctx: &egui::Context, base: massif_core::versions::SavePoint, snapshot: World, depot: massif_format::depot::FolderDepot, author: String, message: String) -> Job {
        use massif_format::depot::Depot;
        Job::spawn(JobKind::Export, tr!("Sending the contribution").to_owned(), None, ctx, move || {
            let patch = match massif_core::patch::Patch::since(&base, &snapshot, massif_core::patch::Kind::Contribution, &author, &message, massif_core::versions::now()) {
                Ok(p) => p,
                Err(e) => return Outcome::Failed(e.to_string()),
            };
            match depot.send(&patch) {
                Ok(path) => {
                    let bytes = std::fs::metadata(&path).map_or(0, |m| m.len());
                    Outcome::PatchWritten { path, tiles: patch.tiles(), bytes }
                }
                Err(e) => Outcome::Failed(tr!("The contribution could not be put in the shared folder: {e}", e)),
            }
        })
    }

    /// A region of the map (`snapshot`, cold, with its identifier) written
    /// as a `.massif` of its own at `path`: the tiles around `rect` and
    /// `border` more, with the map's versions cut to them. The version it
    /// comes back to is the one the map stands on (`head`) if the map is as
    /// it was then, a new one named `name` otherwise.
    #[allow(clippy::too_many_arguments)]
    pub fn export_region_map(ctx: &egui::Context, mut snapshot: World, dim: usize, rect: massif_core::BlockRect, border: i32, path: PathBuf, head: Option<massif_core::versions::SavePoint>, name: String) -> Job {
        let label = tr!("Writing {file}", file = display_name(&path));
        Job::spawn(JobKind::Export, label, None, ctx, move || {
            let point = match head {
                Some(h) if massif_core::versions::unchanged_since(&snapshot, &h) => None,
                _ => {
                    let copy = massif_core::versions::SavePoint::copy_of(&mut snapshot);
                    let point = massif_core::versions::SavePoint::of(copy, name, massif_core::versions::now());
                    snapshot.meta.head = Some(point.id);
                    snapshot.points.push(point.clone());
                    Some(point)
                }
            };
            let region = massif_core::region_map::RegionOf::around(dim, rect, border);
            let mut part = massif_core::region_map::crop(&snapshot, region);
            part.meta.name = format!("{} ({}, {})", snapshot.meta.name, rect.x0, rect.z0);
            match massif_format::save(&part, &path) {
                Ok(_) => Outcome::RegionMapWritten { path, tiles: region.size(), point },
                Err(e) => Outcome::Failed(tr!("Could not save {file}. {why}", file = display_name(&path), why = massif_format::describe(&e, &path))),
            }
        })
    }

    /// Another `.massif` (a region of the map, a copy of it, or another
    /// map) read and what it changes classified against the map as it is
    /// (`map`, a snapshot at `revision`, with its identifier).
    pub fn merge_map(ctx: &egui::Context, path: PathBuf, map: World, revision: u64) -> Job {
        let label = tr!("Reading “{name}”", name = display_name(&path));
        Job::spawn(JobKind::Auto, label, None, ctx, move || {
            let theirs = match massif_format::load(&path) {
                Ok(w) => w,
                Err(e) => return Outcome::Failed(tr!("{file} cannot be read: {e}", file = display_name(&path), e)),
            };
            let mut from = match massif_core::region_map::patch_from_map(&map, &theirs, massif_core::versions::now()) {
                Ok(f) => f,
                Err(e) => return Outcome::Failed(e.to_string()),
            };
            // Signed by nobody: the file's name says whose work it is.
            if from.patch.manifest.author.is_empty() {
                from.patch.manifest.author = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            }
            Outcome::Review(Box::new(crate::collab::ReviewReady::of(from.patch, &map, display_name(&path), from.common, revision)))
        })
    }

    /// A `.massifpatch` file read and classified against the map as it is
    /// (`map`, a snapshot at `revision`).
    pub fn review_patch(ctx: &egui::Context, path: PathBuf, map: World, revision: u64) -> Job {
        let label = tr!("Reading “{name}”", name = display_name(&path));
        Job::spawn(JobKind::Auto, label, None, ctx, move || {
            let patch = match massif_format::load_patch(&path) {
                Ok(p) => p,
                Err(massif_format::FormatError::NotPatch) => return Outcome::Failed(tr!("{file} is a map, not a contribution: open it with File ▸ Open…", file = display_name(&path))),
                Err(e) => return Outcome::Failed(tr!("{file} cannot be read: {e}", file = display_name(&path), e)),
            };
            if map.meta.map_id != Some(patch.manifest.map) {
                return Outcome::Failed(tr!("{file} is a contribution to another map, “{name}”: open that map to review it", file = display_name(&path), name = patch.manifest.map_name));
            }
            let known = map.points.iter().any(|p| p.id == patch.manifest.base.id);
            Outcome::Review(Box::new(crate::collab::ReviewReady::of(patch, &map, display_name(&path), known, revision)))
        })
    }

    /// The contribution of the map (`snapshot`) since version `base`,
    /// written to `path`.
    pub fn write_patch(ctx: &egui::Context, base: massif_core::versions::SavePoint, snapshot: World, path: PathBuf, author: String, message: String) -> Job {
        let label = tr!("Writing {file}", file = display_name(&path));
        Job::spawn(JobKind::Export, label, None, ctx, move || {
            let patch = match massif_core::patch::Patch::since(&base, &snapshot, massif_core::patch::Kind::Contribution, &author, &message, massif_core::versions::now()) {
                Ok(p) => p,
                Err(e) => return Outcome::Failed(e.to_string()),
            };
            match massif_format::save_patch(&patch, &path) {
                Ok(bytes) => Outcome::PatchWritten { path, tiles: patch.tiles(), bytes },
                Err(e) => Outcome::Failed(tr!("Could not save {file}. {why}", file = display_name(&path), why = massif_format::describe(&e, &path))),
            }
        })
    }

    /// The structure `req` asks for, assembled on dimension `dim` of a
    /// snapshot of the document from the game's files `lib`.
    #[allow(clippy::too_many_arguments)]
    pub fn structure(ctx: &egui::Context, snapshot: World, dim: usize, lib: Arc<massif_structures::Library>, req: massif_structures::vanilla::Request, replace: Option<u32>, old_ground: Vec<(i32, i32, f32)>, recipe: massif_structures::placed::Recipe) -> Job {
        let label = tr!("Assembling {name}", name = crate::i18n::tr_name(req.kind.name));
        Job::spawn(JobKind::Structure, label, None, ctx, move || {
            let map = massif_structures::ground::MapGround::new(&snapshot, dim);
            // The land the one replaced levelled, as it was before.
            let ground = massif_structures::vanilla::Adapted { inner: &map, heights: old_ground.iter().map(|&(x, z, h)| ((x, z), h.round() as i32)).collect() };
            match massif_structures::vanilla::build(&lib, &ground, &req) {
                Ok(built) => Outcome::Structure { built: Box::new(built), at: (req.at[0], req.at[2]), replace, dim, recipe },
                Err(e) => Outcome::Failed(tr!("{name} could not be placed: {e}", name = crate::i18n::tr_name(req.kind.name), e)),
            }
        })
    }

    /// A village of `theme` filling `selection` on dimension `dim` of a
    /// snapshot of the document (`lib`: the game's files, for its own
    /// themes).
    #[allow(clippy::too_many_arguments)]
    #[allow(clippy::too_many_arguments)]
    pub fn village(ctx: &egui::Context, snapshot: World, dim: usize, lib: Option<Arc<massif_structures::Library>>, theme: massif_structures::village::Theme, job: crate::structure_tool::VillageJob, old_ground: Vec<(i32, i32, f32)>, id: u32) -> Job {
        Job::spawn(JobKind::Structure, tr!("Building the village").to_owned(), None, ctx, move || {
            let massif_structures::placed::Recipe::Village { params, seed, .. } = job.recipe.clone() else { return Outcome::Failed("not a village".into()) };
            let Some(rect) = job.recipe.area() else { return Outcome::Failed("not a village".into()) };
            let mask = job.mask.clone();
            let inside = move |x: i32, z: i32| mask.as_ref().is_none_or(|m| m.get(x, z) >= 128);
            let map = massif_structures::ground::MapGround::new(&snapshot, dim);
            let ground = massif_structures::vanilla::Adapted { inner: &map, heights: old_ground.iter().map(|&(x, z, h)| ((x, z), h.round() as i32)).collect() };
            let req = massif_structures::village::Request { theme: &theme, rect, inside: &inside, seed, params, id };
            match massif_structures::village::build(lib.as_deref(), &ground, &req) {
                Ok(built) => Outcome::Village { built: Box::new(built), dim, replace: job.replace, recipe: job.recipe },
                Err(e) => Outcome::Failed(tr!("The village could not be built: {e}", e)),
            }
        })
    }

    /// The relief of a real place: a new world (`snapshot` `None`), or
    /// written into dimension `dim` of a snapshot of the document at
    /// `revision`, painted with `rules`.
    pub fn real(ctx: &egui::Context, store: Arc<crate::tiles::TileStore>, job: crate::real_terrain::RealJob, snapshot: Option<World>, dim: usize, rules: massif_core::auto::AutoRules, revision: u64) -> Job {
        use massif_core::real::{Elevation, Samples, Tiles, metres_per_px, spacing};
        use std::sync::atomic::Ordering::Relaxed;
        let progress = Arc::new(Phased::default());
        let p = progress.clone();
        let online = matches!(job.source, crate::real_terrain::Source::Online);
        let label = match (&snapshot, online) {
            (None, true) => tr!("Downloading the relief of “{name}”", name = job.name),
            (None, false) => tr!("Making “{name}” from the file", name = job.name),
            (Some(_), _) => tr!("Relief of a real place").to_owned(),
        };
        let kind = if snapshot.is_some() { JobKind::Auto } else { JobKind::Generate };
        Job::spawn(kind, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            // A new world is whole tiles, each read from data of its own.
            let area = if snapshot.is_none() { job.area.whole_tiles() } else { job.area };
            p.steps.store(if snapshot.is_some() { 3 } else { 2 }, Relaxed);
            let mut fetched = None;
            let mut square = None;
            let (source, data_metres): (Box<dyn Elevation>, f64) = match &job.source {
                crate::real_terrain::Source::Online => {
                    let z = area.zoom(massif_core::real::MAX_TILES);
                    let keys = area.tiles(z);
                    p.tiles.store(keys.len(), Relaxed);
                    match store.fetch(&keys, &p.fetched, &p.pass.cancel) {
                        Ok(got) => {
                            let failed = got.failed();
                            let tiles = Tiles::patched(z, got.tiles, &got.coarser);
                            fetched = Some((keys.len(), failed, got.lost, got.why, tiles.missing_px()));
                            square = Some(tiles.square_px() as f64 * metres_per_px(area.lat, z));
                            (Box::new(tiles), metres_per_px(area.lat, z))
                        }
                        Err(e) => return Outcome::Failed(tr!("Relief of a real place: {e}", e)),
                    }
                }
                crate::real_terrain::Source::File(f) => {
                    p.tiles.store(1, Relaxed);
                    p.fetched.store(1, Relaxed);
                    (Box::new(f.grid()), f.metres_per_px)
                }
            };
            if p.pass.cancel.load(Relaxed) {
                return Outcome::Cancelled;
            }
            p.step.store(1, Relaxed);
            let mut samples = Samples::read(&area, source.as_ref(), spacing(&area, data_metres));
            drop(source);
            // Gaps in the data given the heights around them, and the coasts
            // shaped.
            // The map's water at the real height the user gave it, over the
            // sea's: a lake's (the Great Lakes' 176 m).
            let water = (job.vertical.datum > crate::real_terrain::WATER_DATUM).then_some(job.vertical.datum);
            let prepared = samples.prepare_at(area.blocks_per_metre, square, water);
            let note = real_note(&job.name, fetched, &prepared, samples.metres.len());
            let Some(span) = samples.span() else {
                return Outcome::Failed(tr!("Relief of a real place: no heights over this area").to_owned());
            };
            let fit = job.vertical.fit(span, area.blocks_per_metre);
            let sea = job.vertical.sea_level;
            // The sea's biomes by the place's latitude.
            let rules = massif_core::real::rules_at(&rules, area.lat);
            let painted_land = snapshot.is_none() || job.repaint;
            let paint = |world: &mut World, history: &mut massif_core::history::History| {
                let base = massif_core::auto::base_terrain(&world.dims[dim], &world.registry);
                let reader = massif_core::auto::Reader::new(&rules, &world.registry, sea).with_base(base);
                let mut painted = massif_core::auto::paint_over_the_map_with(&mut history.edit(world, 0), &reader, &p.pass);
                // A land painted from the place gets its biomes too: the sea's
                // ocean, its lakes river, its beaches beach.
                if painted_land && !painted.cancelled {
                    let biomes = massif_core::auto::biomes_over_the_map_with(&mut history.edit(world, 0), &reader, &p.pass);
                    painted.cancelled = biomes.cancelled;
                    painted.tiles.extend(biomes.tiles);
                }
                painted
            };
            match snapshot {
                None => {
                    let (lo, hi) = (job.vertical.low, job.vertical.high);
                    let mut world = massif_core::real::new_world(&job.name, &samples, &fit, &area, sea, lo, hi);
                    // Its terrains of the palette's blocks, as a new map's.
                    massif_core::materials::apply(&mut world.registry, &job.palette);
                    world.meta.palette = job.palette.clone();
                    drop(samples);
                    p.step.store(1, Relaxed);
                    let mut history = massif_core::history::History::new(1);
                    history.begin(tr!("automatic"), 0);
                    let painted = paint(&mut world, &mut history);
                    drop(history);
                    if painted.cancelled {
                        return Outcome::Cancelled;
                    }
                    Outcome::World { world, path: None, note }
                }
                Some(mut snapshot) => {
                    let rect = match (job.target, job.resize) {
                        (crate::real_terrain::Target::Map { .. }, Some(rect)) => rect,
                        (crate::real_terrain::Target::Map { rect, .. }, None) => rect,
                        (crate::real_terrain::Target::NewWorld, _) => unreachable!("a snapshot is of a map"),
                    };
                    // Another palette: the map's terrains made of its blocks
                    // before the ground is written and painted.
                    let palette = (job.palette != snapshot.meta.palette).then(|| job.palette.clone());
                    if let Some(p) = &palette {
                        massif_core::materials::apply(&mut snapshot.registry, p);
                        snapshot.meta.palette = p.clone();
                    }
                    if job.resize.is_some() {
                        let ground = fit.sea.floor();
                        snapshot.dims[dim].resize(rect, || massif_core::tile::Tile::ground(ground, 0, massif_core::tile::NO_FLUID));
                    }
                    let repaint = job.repaint.then(|| massif_core::generate::Theme::new(&snapshot.registry));
                    let mut history = massif_core::history::History::new(1);
                    history.begin(tr!("real terrain"), dim);
                    let written = massif_core::real::write_ground(&mut history.edit(&mut snapshot, 0), &samples, &fit, rect, repaint.as_ref().map(|t| (&area, *t)), &p.pass);
                    drop(samples);
                    if written.cancelled {
                        return Outcome::Cancelled;
                    }
                    p.step.store(2, Relaxed);
                    let painted = paint(&mut snapshot, &mut history);
                    drop(history);
                    if painted.cancelled {
                        return Outcome::Cancelled;
                    }
                    if job.resize.is_some() || palette.is_some() {
                        return Outcome::RealMap {
                            dimension: Box::new(snapshot.dims[dim].snapshot()),
                            palette,
                            resized: job.resize.is_some(),
                            columns: written.columns,
                            revision,
                            dim,
                            note,
                        };
                    }
                    let mut changed = written.tiles;
                    changed.extend(painted.tiles);
                    changed.sort_unstable_by_key(|p| (p.z, p.x));
                    changed.dedup();
                    let level = &mut snapshot.dims[dim].levels[0];
                    level.encode_blobs(&changed);
                    let tiles = changed.iter().filter_map(|&pos| level.blob(pos).map(|b| (pos, b))).collect();
                    Outcome::RealTerrain {
                        tiles,
                        columns: written.columns,
                        revision,
                        dim,
                        note,
                    }
                }
            }
        })
    }

    pub fn export(ctx: &egui::Context, snapshot: World, dir: PathBuf, options: ExportOptions) -> Job {
        let progress = Arc::new(Progress::default());
        let label = tr!("Exporting to {file}", file = display_name(&dir));
        let p = progress.clone();
        Job::spawn(JobKind::Export, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            match massif_anvil::export(&snapshot, &dir, &options, &p) {
                Ok(stats) => Outcome::Exported { dir, stats },
                Err(massif_anvil::ExportError::Cancelled) => Outcome::Cancelled,
                Err(massif_anvil::ExportError::Io(e)) => Outcome::Failed(tr!("Export failed. {why}.", why = massif_core::atomic::cause(&e, &dir))),
                Err(e) => Outcome::Failed(tr!("Export failed: {e}", e)),
            }
        })
    }
}

/// What the relief of real place `name` lacked and how it was made good,
/// written to the log; returned to be said when tiles could not be had
/// (`fetched`: the tiles asked, failed, lost, why the first failed,
/// pixels without data) or samples had no data.
fn real_note(name: &str, fetched: Option<(usize, usize, usize, Option<String>, usize)>, prepared: &massif_core::real::Prepared, samples: usize) -> Option<String> {
    let c = &prepared.coast;
    let mut log = format!(
        "real place “{name}”: {samples} samples, {} without data filled; coast: data in squares of {:.1} samples, {} samples of a coarse fill mended, {} under the sea ({} with a depth of the data)",
        prepared.missing, c.square, c.coarse, c.sea, c.known
    );
    let mut said = None;
    if let Some((asked, failed, lost, why, px)) = fetched {
        let coarser = failed - lost;
        log += &format!("; {asked} tiles, {failed} failed: {coarser} made from a coarser zoom, {lost} lost; {px} pixels without data");
        if let Some(why) = &why {
            log += &format!(" (first failure: {why})");
        }
        if failed > 0 {
            said = Some(tr!("{n} of {total} tiles of elevation could not be downloaded: {coarser} were made from coarser data, {lost} filled from the land around them", n = failed, total = asked, coarser, lost));
        }
    }
    if said.is_none() && prepared.missing * 100 > samples {
        let share = prepared.missing as f64 * 100.0 / samples as f64;
        said = Some(tr!("{share:.0} % of the area had no elevation data and was filled from the land around it", share));
    }
    crate::startup::note(&log);
    said
}

/// Whether a path names a WorldPainter world.
pub fn is_worldpainter(path: &std::path::Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("world"))
}

pub fn display_name(path: &std::path::Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// Reveal a file or folder in the system file manager.
pub fn reveal(path: &std::path::Path) {
    #[cfg(target_os = "macos")]
    let result = std::process::Command::new("open").arg("-R").arg(path).spawn();
    #[cfg(target_os = "windows")]
    let result = std::process::Command::new("explorer").arg("/select,").arg(path).spawn();
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let result = std::process::Command::new("xdg-open")
        .arg(path.parent().unwrap_or(path))
        .spawn();
    if let Err(e) = result {
        log::warn!("could not reveal {}: {e}", path.display());
    }
}

/// Run `load` in the threads kept for what the app loads as it starts (the
/// brushes, the game's textures), rather than in rayon's global pool. The
/// interface waits on that pool whenever it draws with rayon (the picture
/// behind the home screen, the map), and a thread that is not one of the
/// pool's waits for the pool's threads to finish what they are doing before
/// they take its work: a slow load there (a large folder of WorldPainter
/// brushes, on a slow computer) held up the first frame for minutes.
pub fn in_loading_pool<R: Send>(load: impl FnOnce() -> R + Send) -> R {
    static POOL: std::sync::OnceLock<Option<rayon::ThreadPool>> = std::sync::OnceLock::new();
    let pool = POOL.get_or_init(|| {
        // One core left to the interface.
        let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .thread_name(|k| format!("massif loads {k}"))
            // Below the interface's priority: on a computer with few cores
            // the loads never keep the main thread from drawing.
            .start_handler(|_| crate::startup::below_normal_priority())
            .build()
            .ok()
    });
    match pool {
        Some(pool) => pool.install(load),
        None => load(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn wait(job: &Job) -> Outcome {
        let started = Instant::now();
        loop {
            if let Some(outcome) = job.poll() {
                return outcome;
            }
            assert!(started.elapsed() < Duration::from_secs(60), "job timed out");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn generate_save_open_and_export_in_the_background() {
        let ctx = egui::Context::default();
        let dir = std::env::temp_dir().join(format!("massif-jobs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let params = GenParams {
            tiles_x: 2,
            tiles_z: 2,
            name: "Jobs".into(),
            ..GenParams::default()
        };
        let job = Job::generate(&ctx, params);
        assert!(job.kind.blocks_editing());
        let Outcome::World { mut world, path: None, .. } = wait(&job) else {
            panic!("generation failed")
        };
        assert_eq!(world.meta.name, "Jobs");

        let path = dir.join("jobs.massif");
        let job = Job::save(&ctx, world.snapshot(), path.clone(), 7);
        match wait(&job) {
            Outcome::Saved { path: p, revision, bytes } => {
                assert_eq!((p, revision), (path.clone(), 7));
                assert_eq!(bytes, std::fs::metadata(&path).unwrap().len());
            }
            _ => panic!("save failed"),
        }

        let job = Job::open(&ctx, path.clone());
        let Outcome::World { world: back, path: Some(p), .. } = wait(&job) else {
            panic!("open failed")
        };
        assert_eq!(p, path);
        assert_eq!(back.dims[0].ground().len(), 4);

        assert!(is_worldpainter(std::path::Path::new("Old Map.WORLD")));
        assert!(!is_worldpainter(&path));
        let job = Job::open(&ctx, dir.join("missing.world"));
        assert_eq!(job.kind, JobKind::Import);
        assert!(matches!(wait(&job), Outcome::Failed(msg) if msg.contains("missing.world")));

        let job = Job::open(&ctx, dir.join("missing.massif"));
        assert!(matches!(wait(&job), Outcome::Failed(msg) if msg.contains("missing.massif")));

        let target = dir.join("export");
        let job = Job::export(&ctx, world.snapshot(), target.clone(), ExportOptions::default());
        assert!(job.progress.is_some());
        match wait(&job) {
            Outcome::Exported { dir, stats } => {
                assert_eq!(dir, target);
                assert_eq!(stats.chunks, 4 * 64);
            }
            _ => panic!("export failed"),
        }
        assert!(target.join("level.dat").is_file());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
