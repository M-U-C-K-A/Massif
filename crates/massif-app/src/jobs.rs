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
    World { world: World, path: Option<PathBuf> },
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
    /// Caves carved into a copy of the map: the sculpted chunks they
    /// changed, to put in place in one undoable step.
    Caves {
        chunks: Vec<(massif_core::voxel::ChunkPos, Arc<massif_core::voxel::VoxelChunk>)>,
        blocks: usize,
        flooded: usize,
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
                let outcome = crate::startup::catch(work).unwrap_or_else(|e| Outcome::Failed(format!("The task stopped on an error: {e}. The map is as it was.")));
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
            world: generate(&params),
            path: None,
        })
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

    /// Open a Massif world, or import a WorldPainter one.
    pub fn open(ctx: &egui::Context, path: PathBuf) -> Job {
        if is_worldpainter(&path) {
            return Job::import(ctx, path);
        }
        let label = tr!("Opening {file}", file = display_name(&path));
        Job::spawn(JobKind::Open, label, None, ctx, move || match massif_format::load(&path) {
            Ok(world) => Outcome::World { world, path: Some(path) },
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
    pub fn save_point(ctx: &egui::Context, snapshot: World, name: String) -> Job {
        let label = tr!("Saving the point “{name}”", name);
        Job::spawn(JobKind::SavePoint, label, None, ctx, move || {
            Outcome::SavePoint(massif_core::versions::SavePoint::of(snapshot, name, massif_core::versions::now()))
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
            let changes = massif_core::graph::reshape_ground(&mut history.edit(&mut snapshot, 0), &op, sea, bed, &p);
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

    /// Caves carved into dimension `dim` of a snapshot of the document at
    /// `revision` (over its selection when it has one).
    pub fn caves(ctx: &egui::Context, snapshot: World, dim: usize, settings: massif_core::caves::CaveSettings, revision: u64) -> Job {
        use massif_core::auto::Steps;
        let progress = Arc::new(Steps::default());
        let p = progress.clone();
        let label = if snapshot.dims[dim].selection.is_some() { tr!("Carving caves in the selection") } else { tr!("Carving caves over the whole map") };
        Job::spawn(JobKind::Auto, label.to_owned(), Some(progress as Arc<dyn JobProgress>), ctx, move || {
            let carved = massif_core::caves::carve(&snapshot.dims[dim], &settings, &p);
            if carved.cancelled {
                return Outcome::Cancelled;
            }
            Outcome::Caves {
                chunks: carved.chunks,
                blocks: carved.blocks,
                flooded: carved.flooded,
                revision,
                dim,
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
            let area = job.area;
            p.steps.store(if snapshot.is_some() { 3 } else { 2 }, Relaxed);
            let (source, data_metres): (Box<dyn Elevation>, f64) = match &job.source {
                crate::real_terrain::Source::Online => {
                    let z = area.zoom(massif_core::real::MAX_TILES);
                    let keys = area.tiles(z);
                    p.tiles.store(keys.len(), Relaxed);
                    match store.load_all(&keys, &p.fetched, &p.pass.cancel) {
                        Ok(tiles) => (Box::new(Tiles { z, tiles }), metres_per_px(area.lat, z)),
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
            let samples = Samples::read(&area, source.as_ref(), spacing(&area, data_metres));
            drop(source);
            let Some(span) = samples.span() else {
                return Outcome::Failed(tr!("Relief of a real place: no heights over this area").to_owned());
            };
            let fit = job.vertical.fit(span, area.blocks_per_metre);
            let sea = job.vertical.sea_level;
            let paint = |world: &mut World, history: &mut massif_core::history::History| {
                let base = massif_core::auto::base_terrain(&world.dims[dim], &world.registry);
                let reader = massif_core::auto::Reader::new(&rules, &world.registry, sea).with_base(base);
                massif_core::auto::paint_over_the_map_with(&mut history.edit(world, 0), &reader, &p.pass)
            };
            match snapshot {
                None => {
                    let (lo, hi) = (job.vertical.low, job.vertical.high);
                    let mut world = massif_core::real::new_world(&job.name, &samples, &fit, sea, lo, hi);
                    drop(samples);
                    p.step.store(1, Relaxed);
                    let mut history = massif_core::history::History::new(1);
                    history.begin(tr!("automatic"), 0);
                    let painted = paint(&mut world, &mut history);
                    drop(history);
                    if painted.cancelled {
                        return Outcome::Cancelled;
                    }
                    Outcome::World { world, path: None }
                }
                Some(mut snapshot) => {
                    let rect = match job.target {
                        crate::real_terrain::Target::Map { rect, .. } => rect,
                        crate::real_terrain::Target::NewWorld => unreachable!("a snapshot is of a map"),
                    };
                    let mut history = massif_core::history::History::new(1);
                    history.begin(tr!("real terrain"), dim);
                    let written = massif_core::real::write_ground(&mut history.edit(&mut snapshot, 0), &samples, &fit, rect, &p.pass);
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
        rayon::ThreadPoolBuilder::new().num_threads(threads).thread_name(|k| format!("massif loads {k}")).build().ok()
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
        let Outcome::World { mut world, path: None } = wait(&job) else {
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
        let Outcome::World { world: back, path: Some(p) } = wait(&job) else {
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
