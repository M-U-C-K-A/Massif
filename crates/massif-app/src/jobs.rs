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
    Saved {
        path: PathBuf,
        revision: u64,
        bytes: u64,
    },
    Exported {
        dir: PathBuf,
        stats: ExportStats,
    },
    /// A WorldPainter world converted to Massif.
    Imported {
        world: World,
        report: Box<ImportReport>,
        source: PathBuf,
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
}

impl JobKind {
    /// Jobs that replace the document block editing until they finish.
    pub fn blocks_editing(self) -> bool {
        matches!(self, JobKind::Generate | JobKind::Open | JobKind::Import)
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

impl JobProgress for ImportProgress {
    fn fraction(&self) -> f32 {
        ImportProgress::fraction(self)
    }
    fn cancel(&self) {
        self.cancel.store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

pub struct Job {
    pub kind: JobKind,
    pub label: String,
    pub started: Instant,
    pub progress: Option<Arc<dyn JobProgress>>,
    rx: Receiver<Outcome>,
}

impl Job {
    fn spawn(kind: JobKind, label: String, progress: Option<Arc<dyn JobProgress>>, ctx: &egui::Context, work: impl FnOnce() -> Outcome + Send + 'static) -> Job {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name(format!("massif {kind:?}"))
            // Reading Java object graphs recurses; give it room.
            .stack_size(64 << 20)
            .spawn(move || {
                let outcome = work();
                let _ = tx.send(outcome);
                ctx.request_repaint();
            })
            .expect("spawning a worker thread");
        Job {
            kind,
            label,
            started: Instant::now(),
            progress,
            rx,
        }
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
        let label = format!("Generating “{}”", params.name);
        Job::spawn(JobKind::Generate, label, None, ctx, move || Outcome::World {
            world: generate(&params),
            path: None,
        })
    }

    /// Convert a WorldPainter `.world` file.
    pub fn import(ctx: &egui::Context, path: PathBuf) -> Job {
        let progress = Arc::new(ImportProgress::default());
        let label = format!("Importing {}", display_name(&path));
        let p = progress.clone();
        Job::spawn(JobKind::Import, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            match massif_legacy::import_world(&path, Some(&p)) {
                Ok((world, report)) => Outcome::Imported {
                    world,
                    report: Box::new(report),
                    source: path,
                },
                Err(massif_legacy::ImportError::Cancelled) => Outcome::Cancelled,
                Err(e) => Outcome::Failed(format!("Could not import {}: {e}", display_name(&path))),
            }
        })
    }

    /// Open a Massif world, or import a WorldPainter one.
    pub fn open(ctx: &egui::Context, path: PathBuf) -> Job {
        if is_worldpainter(&path) {
            return Job::import(ctx, path);
        }
        let label = format!("Opening {}", display_name(&path));
        Job::spawn(JobKind::Open, label, None, ctx, move || match massif_format::load(&path) {
            Ok(world) => Outcome::World { world, path: Some(path) },
            Err(e) => Outcome::Failed(format!("Could not open {}: {e}", display_name(&path))),
        })
    }

    /// Save a cold snapshot taken at `revision`.
    pub fn save(ctx: &egui::Context, snapshot: World, path: PathBuf, revision: u64) -> Job {
        let label = format!("Saving {}", display_name(&path));
        Job::spawn(JobKind::Save, label, None, ctx, move || match massif_format::save(&snapshot, &path) {
            Ok(stats) => Outcome::Saved {
                path,
                revision,
                bytes: stats.file_len,
            },
            Err(e) => Outcome::Failed(format!("Could not save {}: {e}", display_name(&path))),
        })
    }

    pub fn export(ctx: &egui::Context, snapshot: World, dir: PathBuf, options: ExportOptions) -> Job {
        let progress = Arc::new(Progress::default());
        let label = format!("Exporting to {}", display_name(&dir));
        let p = progress.clone();
        Job::spawn(JobKind::Export, label, Some(progress as Arc<dyn JobProgress>), ctx, move || {
            match massif_anvil::export(&snapshot, &dir, &options, &p) {
                Ok(stats) => Outcome::Exported { dir, stats },
                Err(massif_anvil::ExportError::Cancelled) => Outcome::Cancelled,
                Err(e) => Outcome::Failed(format!("Export failed: {e}")),
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
