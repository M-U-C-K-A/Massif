//! Backups of the open map while it has changes not saved, to restore after
//! Massif closed without saving them (a crash, a forced quit, a power cut).
//!
//! Every few minutes of changes, a copy of the map goes to a folder of the
//! app's own (`Autosave`, next to `startup.log`), never over the map's own
//! file: a `.massif` file and a `.json` of what it is (the map's name, its
//! file, when it was taken). The copy is taken without encoding anything
//! (see [`massif_core::World::copy_unfrozen`]) and written on a thread of
//! its own, so the frames go on as they were.
//!
//! Each run of Massif is a session holding a lock on a file of its own
//! (`<session>.lock`) as long as it runs: the system lets go of it when the
//! process ends, however it ends. A backup whose session holds its lock no
//! more was left by a run that did not close cleanly: the next start offers
//! to restore it. Saving the map, or closing it (quitting, opening another),
//! removes its backup; at most [`MAX_BACKUPS`] are kept, of at most
//! [`MAX_BYTES`] together.

use massif_core::World;
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Backups kept at most, of every session.
pub const MAX_BACKUPS: usize = 5;
/// Their size together at most; the newest backup of each running session
/// is kept whatever its size.
pub const MAX_BYTES: u64 = 4 << 30;

/// The folder of the backups, next to `startup.log`.
pub fn default_dir() -> Option<PathBuf> {
    crate::startup::log_dir().map(|d| d.join("Autosave"))
}

/// What a backup is of, kept beside it as JSON.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Info {
    /// The run of Massif that wrote it.
    pub session: String,
    /// The map in that run (its document's generation).
    pub generation: u64,
    /// The map's name.
    pub name: String,
    /// The map's own file; `None` for a map never saved.
    pub path: Option<PathBuf>,
    /// When it was taken, in seconds since 1970.
    pub taken: u64,
    /// The document's revision it holds.
    pub revision: u64,
}

/// A backup on disk.
#[derive(Debug, Clone, PartialEq)]
pub struct Backup {
    /// The `.massif` file; the `.json` is beside it.
    pub file: PathBuf,
    pub info: Info,
    pub bytes: u64,
}

impl Backup {
    fn info_path(file: &Path) -> PathBuf {
        file.with_extension("json")
    }

    /// Remove its files.
    fn remove(&self) {
        let _ = std::fs::remove_file(&self.file);
        let _ = std::fs::remove_file(Self::info_path(&self.file));
    }

    /// How long ago it was taken, in words: "3 minutes ago".
    pub fn age(&self) -> String {
        let secs = now_secs().saturating_sub(self.info.taken);
        match secs {
            0..60 => tr!("less than a minute ago").into(),
            60..3600 => trn!("{n} minute ago", "{n} minutes ago", n = secs / 60),
            3600..86400 => trn!("{n} hour ago", "{n} hours ago", n = secs / 3600),
            _ => trn!("{n} day ago", "{n} days ago", n = secs / 86400),
        }
    }
}

fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Write `bytes` to `path` through a file beside it, renamed over it once
/// complete.
fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    massif_core::atomic::write(path, bytes, true)
}

/// Every backup in `dir` whose description reads, newest first.
fn list(dir: &Path) -> Vec<Backup> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for e in entries.flatten() {
        let info_path = e.path();
        if info_path.extension().is_none_or(|x| x != "json") {
            continue;
        }
        let file = info_path.with_extension(massif_format::EXTENSION);
        let Ok(bytes) = std::fs::metadata(&file).map(|m| m.len()) else {
            continue;
        };
        if let Some(info) = std::fs::read(&info_path).ok().and_then(|b| serde_json::from_slice::<Info>(&b).ok()) {
            out.push(Backup { file, info, bytes });
        }
    }
    out.sort_by(|a, b| b.info.taken.cmp(&a.info.taken).then(b.info.revision.cmp(&a.info.revision)));
    out
}

/// Whether the run that holds `session`'s lock is still running: its lock
/// file is there and cannot be taken.
fn running(dir: &Path, session: &str) -> bool {
    let Ok(file) = File::open(dir.join(format!("{session}.lock"))) else {
        return false;
    };
    matches!(file.try_lock(), Err(std::fs::TryLockError::WouldBlock))
}

/// Keep at most [`MAX_BACKUPS`] backups of at most [`MAX_BYTES`]: the
/// oldest go first, but never `keep` nor the newest backup of a session
/// still running.
fn prune(dir: &Path, keep: &Path) {
    let mut seen_running = std::collections::HashSet::new();
    let (mut count, mut bytes) = (0usize, 0u64);
    for b in list(dir) {
        let newest_of_running = !seen_running.contains(&b.info.session) && running(dir, &b.info.session);
        if newest_of_running {
            seen_running.insert(b.info.session.clone());
        }
        let protected = b.file == keep || newest_of_running;
        if !protected && (count >= MAX_BACKUPS || bytes + b.bytes > MAX_BYTES) {
            log::info!("autosave: removing the old backup {}", b.file.display());
            b.remove();
            continue;
        }
        count += 1;
        bytes += b.bytes;
    }
}

/// Write a backup of `copy` (see [`massif_core::World::copy_unfrozen`]) to
/// `dir`, described by `info`; then keep the backups within bounds.
pub fn write(dir: &Path, mut copy: World, info: Info) -> Result<Backup, String> {
    let snapshot = copy.snapshot();
    drop(copy);
    let stem = format!("{}-{}-{}", info.session, info.generation, info.revision);
    let file = dir.join(stem).with_extension(massif_format::EXTENSION);
    let stats = massif_format::save(&snapshot, &file).map_err(|e| e.to_string())?;
    let json = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
    if let Err(e) = write_atomic(&Backup::info_path(&file), &json) {
        let _ = std::fs::remove_file(&file);
        return Err(e.to_string());
    }
    prune(dir, &file);
    Ok(Backup { file, info, bytes: stats.file_len })
}

/// The threads backups are written on: two, given way to by the system
/// before the editor's (and rayon's, which the frames wait for at times),
/// so that writing a backup of a huge map leaves the frames as they were.
fn pool() -> &'static rayon::ThreadPool {
    static POOL: std::sync::OnceLock<rayon::ThreadPool> = std::sync::OnceLock::new();
    POOL.get_or_init(|| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(2)
            .thread_name(|k| format!("massif autosave {k}"))
            .start_handler(|_| lower_priority())
            .build()
            .expect("the autosave's threads")
    })
}

/// Let this thread give way to the others.
fn lower_priority() {
    #[cfg(target_os = "macos")]
    unsafe {
        libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
    }
    #[cfg(target_os = "linux")]
    unsafe {
        // On Linux, a thread's own niceness (its id as the "process").
        libc::setpriority(libc::PRIO_PROCESS, libc::gettid() as libc::id_t, 10);
    }
    #[cfg(windows)]
    unsafe {
        use windows_sys::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
        SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL);
    }
}

/// Write a backup on the autosave's threads (see [`write`]).
pub fn write_in_background(dir: PathBuf, copy: World, info: Info) -> Receiver<Result<Backup, String>> {
    let (tx, rx) = channel();
    pool().spawn(move || {
        let _ = tx.send(pool().install(|| write(&dir, copy, info)));
    });
    rx
}

/// A backup being written.
struct Running {
    rx: Receiver<Result<Backup, String>>,
    started: Instant,
}

/// The backups of this run of Massif.
pub struct Autosave {
    /// Where they go; `None`: nowhere (no folder for the app, or the tests).
    dir: Option<PathBuf>,
    session: String,
    /// Held as long as Massif runs (see the module's notes).
    lock: Option<File>,
    /// The open map's backup, the newest written.
    current: Option<Backup>,
    /// The map the backups and the clock are for (its document's
    /// generation).
    generation: u64,
    /// The revision backed up or saved last: nothing to back up while the
    /// document is at it.
    covered: u64,
    /// The revision saved last: a backup older than it is of no use.
    saved: u64,
    /// Since when the map has had changes not backed up.
    since: Option<Instant>,
    running: Option<Running>,
    /// Backups left by runs that did not close cleanly, newest first, to
    /// offer at the start.
    pub found: Vec<Backup>,
    /// How long the last backup took to write, and the copy before it on
    /// the editor's thread.
    pub last: Option<(Duration, Duration)>,
    /// Keep the open map's backup when this run closes (after an error).
    keep: bool,
}

impl Autosave {
    /// No backups at all.
    pub fn off() -> Self {
        Self {
            dir: None,
            session: String::new(),
            lock: None,
            current: None,
            generation: 0,
            covered: 0,
            saved: 0,
            since: None,
            running: None,
            found: Vec::new(),
            last: None,
            keep: false,
        }
    }

    /// The backups in `dir`, for a new run: what runs before left there is
    /// in [`Autosave::found`].
    pub fn open(dir: PathBuf) -> Self {
        let mut me = Self::off();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            log::warn!("autosave: no folder for the backups at {}: {e}", dir.display());
            return me;
        }
        let millis = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_millis());
        // Two opened in the same millisecond by one process (the tests) are
        // two sessions all the same.
        static OPENED: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = OPENED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        me.session = if n == 0 { format!("{millis}-{}", std::process::id()) } else { format!("{millis}-{}-{n}", std::process::id()) };
        let lock = File::create(dir.join(format!("{}.lock", me.session))).and_then(|f| f.try_lock().map(|_| f).map_err(std::io::Error::other));
        match lock {
            Ok(f) => me.lock = Some(f),
            Err(e) => {
                log::warn!("autosave: no lock for this run: {e}");
                return me;
            }
        }
        // What runs before left: their backups, their files half written,
        // the locks of runs that are over.
        me.found = list(&dir).into_iter().filter(|b| b.info.session != me.session && !running(&dir, &b.info.session)).collect();
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let path = e.path();
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                // A file half written by a run that is over: another
                // running one may be writing its own right now.
                let stale = || e.metadata().and_then(|m| m.modified()).ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > Duration::from_secs(3600));
                if name.ends_with(".tmp") || massif_core::atomic::is_temp(&path) && stale() {
                    let _ = std::fs::remove_file(&path);
                } else if let Some(session) = name.strip_suffix(".lock")
                    && session != me.session
                    && !running(&dir, session)
                    && !me.found.iter().any(|b| b.info.session == session)
                {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        me.dir = Some(dir);
        me
    }

    /// Whether a backup is being written.
    pub fn writing(&self) -> bool {
        self.running.is_some()
    }

    /// The open map's backup, if one is written.
    #[cfg(test)]
    pub fn current(&self) -> Option<&Backup> {
        self.current.as_ref()
    }

    /// Follow the document, every frame: back it up once it has had changes
    /// for `every`, if `on` and nothing holds it (`busy`: a stroke under
    /// way, a task over the whole map). Returns when to look again.
    pub fn tick(&mut self, doc: &crate::doc::Document, on: bool, every: Duration, busy: bool) -> Option<Duration> {
        self.poll();
        if doc.generation != self.generation {
            // Another map: the last one's backup is of no use any more (its
            // changes were saved, or the user chose not to).
            if self.current.as_ref().is_none_or(|b| b.info.generation != doc.generation) {
                self.drop_current();
            }
            self.generation = doc.generation;
            // Nothing of it backed up yet: a map just opened has no changes,
            // one with changes has them all to back up.
            self.covered = u64::MAX;
            self.saved = 0;
            self.since = None;
        }
        if !doc.is_modified() || doc.revision == self.covered {
            self.since = None;
            return None;
        }
        let since = *self.since.get_or_insert_with(Instant::now);
        let dir = self.dir.clone()?;
        if !on || self.running.is_some() {
            return None;
        }
        let waited = since.elapsed();
        if waited < every {
            return Some(every - waited);
        }
        if busy {
            return Some(Duration::from_secs(1));
        }
        let started = Instant::now();
        let copy = doc.world.copy_unfrozen();
        let copied = started.elapsed();
        let info = Info {
            session: self.session.clone(),
            generation: doc.generation,
            name: doc.world.meta.name.clone(),
            path: doc.path.clone(),
            taken: now_secs(),
            revision: doc.revision,
        };
        let rx = write_in_background(dir, copy, info);
        log::info!("autosave: backing up revision {} of “{}” ({:.1} ms to copy)", doc.revision, doc.world.meta.name, copied.as_secs_f64() * 1e3);
        self.running = Some(Running { rx, started });
        self.last = Some((Duration::ZERO, copied));
        self.covered = doc.revision;
        self.since = None;
        None
    }

    /// Take the backup written, if it is done.
    fn poll(&mut self) {
        let Some(run) = &self.running else {
            return;
        };
        let result = match run.rx.try_recv() {
            Ok(r) => r,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => Err("the thread writing it stopped".into()),
        };
        let took = run.started.elapsed();
        self.running = None;
        match result {
            Ok(b) if b.info.generation == self.generation && b.info.revision > self.saved => {
                log::info!("autosave: backed up “{}” ({} bytes) in {:.2} s", b.info.name, b.bytes, took.as_secs_f64());
                if let Some(old) = self.current.replace(b)
                    && Some(&old.file) != self.current.as_ref().map(|b| &b.file)
                {
                    old.remove();
                }
                if let Some(last) = &mut self.last {
                    last.0 = took;
                }
            }
            // Saved meanwhile, or another map opened.
            Ok(b) => b.remove(),
            Err(e) => log::warn!("autosave: the backup failed: {e}"),
        }
    }

    /// Wait for the backup being written, if any (for the tests).
    #[cfg(test)]
    pub fn finish(&mut self) {
        while self.running.is_some() {
            std::thread::sleep(Duration::from_millis(5));
            self.poll();
        }
    }

    /// Back up `doc` now, on this thread, whatever the timer: after an
    /// error, before anything else goes wrong. `None` when it has no
    /// changes to keep.
    pub fn rescue(&mut self, doc: &crate::doc::Document) -> Result<Option<Backup>, String> {
        let Some(dir) = self.dir.clone() else { return Err("backups are off".into()) };
        if let Some(run) = self.running.take() {
            // The one being written is older: let it finish, then replace it.
            if let Ok(Ok(b)) = run.rx.recv_timeout(Duration::from_secs(30)) {
                b.remove();
            }
        }
        if !doc.is_modified() {
            return Ok(None);
        }
        if let Some(b) = &self.current
            && b.info.generation == doc.generation
            && b.info.revision == doc.revision
        {
            return Ok(Some(b.clone()));
        }
        let info = Info {
            session: self.session.clone(),
            generation: doc.generation,
            name: doc.world.meta.name.clone(),
            path: doc.path.clone(),
            taken: now_secs(),
            revision: doc.revision,
        };
        let backup = write(&dir, doc.world.copy_unfrozen(), info)?;
        if let Some(old) = self.current.replace(backup.clone())
            && old.file != backup.file
        {
            old.remove();
        }
        self.generation = doc.generation;
        self.covered = doc.revision;
        self.since = None;
        Ok(Some(backup))
    }

    /// Keep the open map's backup when this run closes, for the next start
    /// to offer back.
    pub fn keep(&mut self) {
        self.keep = true;
    }

    /// The map `generation` was saved at `revision`: its backup goes.
    pub fn saved(&mut self, generation: u64, revision: u64) {
        if generation != self.generation {
            return;
        }
        self.saved = self.saved.max(revision);
        if self.current.as_ref().is_some_and(|b| b.info.revision <= revision) {
            self.drop_current();
        }
        self.covered = self.covered.max(revision);
    }

    fn drop_current(&mut self) {
        if let Some(b) = self.current.take() {
            b.remove();
        }
    }

    /// Massif closes cleanly: the open map's backup goes, and so does the
    /// lock of this run.
    pub fn close(&mut self) {
        self.poll();
        // One being written is removed once written; a run that ends first
        // leaves its half-written file, removed at the next start.
        if let Some(run) = self.running.take()
            && let Ok(Ok(b)) = run.rx.recv_timeout(Duration::from_secs(10))
        {
            if self.keep {
                self.current = Some(b);
            } else {
                b.remove();
            }
        }
        if !self.keep {
            self.drop_current();
        }
        if let (Some(dir), Some(lock)) = (&self.dir, self.lock.take()) {
            drop(lock);
            let _ = std::fs::remove_file(dir.join(format!("{}.lock", self.session)));
        }
        self.dir = None;
    }

    /// A backup left by another run, restored as map `generation` at
    /// `revision`: it is this run's backup of it from now on, until a
    /// newer one, a save or a clean quit.
    pub fn adopt(&mut self, mut backup: Backup, generation: u64, revision: u64) {
        self.found.retain(|b| b.file != backup.file);
        let Some(dir) = self.dir.clone() else {
            return;
        };
        let old_session = std::mem::replace(&mut backup.info.session, self.session.clone());
        backup.info.generation = generation;
        backup.info.revision = revision;
        if let Ok(json) = serde_json::to_vec_pretty(&backup.info) {
            let _ = write_atomic(&Backup::info_path(&backup.file), &json);
        }
        self.drop_current();
        self.current = Some(backup);
        self.generation = generation;
        self.covered = revision;
        self.saved = 0;
        self.since = None;
        self.forget_session(&dir, &old_session);
    }

    /// A backup left by another run, not wanted.
    pub fn discard(&mut self, backup: &Backup) {
        backup.remove();
        self.found.retain(|b| b.file != backup.file);
        if let Some(dir) = self.dir.clone() {
            self.forget_session(&dir, &backup.info.session);
        }
    }

    /// Remove the lock file of a run that is over once none of its backups
    /// are left.
    fn forget_session(&self, dir: &Path, session: &str) {
        if session != self.session && !list(dir).iter().any(|b| b.info.session == session) {
            let _ = std::fs::remove_file(dir.join(format!("{session}.lock")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::doc::Document;
    use massif_core::generate::{GenParams, generate};

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("massif-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn small_world() -> World {
        generate(&GenParams { tiles_x: 3, tiles_z: 2, ..GenParams::default() })
    }

    /// Paint a few columns of the map, as a brush would.
    fn edit(doc: &mut Document) {
        let level = &mut doc.world.dims[0].levels[0];
        let pos = level.positions().next().unwrap();
        level.tile_mut(pos).unwrap().top.fill(77.0 + doc.revision as f32);
        doc.mark_changed();
    }

    #[test]
    fn a_backup_holds_the_map_exactly() {
        let dir = temp_dir("exact");
        let mut a = Autosave::open(dir.clone());
        let mut doc = Document::new(small_world(), None, 1);
        // Edited and left so: the changed tile is not encoded yet.
        edit(&mut doc);
        assert!(a.tick(&doc, true, Duration::ZERO, false).is_none());
        a.finish();
        let b = a.current().expect("a backup").clone();
        let read = massif_format::load(&b.file).unwrap();
        let mut bytes = Vec::new();
        massif_format::write(&doc.world.snapshot(), &mut bytes).unwrap();
        assert_eq!(std::fs::read(&b.file).unwrap(), bytes, "the backup is the file a save would write");
        assert_eq!(read.meta.name, doc.world.meta.name);
        assert_eq!(b.info.name, doc.world.meta.name);
        a.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A backup keeps the map's save points, and restoring it brings them
    /// back with the map.
    #[test]
    fn a_backup_keeps_the_save_points() {
        let dir = temp_dir("points");
        let mut a = Autosave::open(dir.clone());
        let mut doc = Document::new(small_world(), None, 1);
        let point = massif_core::versions::SavePoint::new(&mut doc.world, "before", 5);
        doc.world.points.push(point);
        edit(&mut doc);
        assert!(a.tick(&doc, true, Duration::ZERO, false).is_none());
        a.finish();
        let b = a.current().expect("a backup").clone();
        let read = massif_format::load(&b.file).unwrap();
        assert_eq!(read.points.len(), 1);
        assert_eq!((read.points[0].name.as_str(), read.points[0].created), ("before", 5));
        // The point is the map before the edit, the map the map after it.
        let pos = doc.world.dims[0].levels[0].positions().next().unwrap();
        let mut now = read.dims[0].levels[0].clone();
        let mut then = read.points[0].world.dims[0].levels[0].clone();
        assert_eq!(now.tile_mut(pos).unwrap().top.get(0), 77.0 + (doc.revision - 1) as f32);
        assert_ne!(then.tile_mut(pos).unwrap().top.get(0), now.tile_mut(pos).unwrap().top.get(0));
        a.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_crash_leaves_a_backup_the_next_start_finds() {
        let dir = temp_dir("crash");
        let mut a = Autosave::open(dir.clone());
        let mut doc = Document::new(small_world(), Some("/maps/Island.massif".into()), 3);
        edit(&mut doc);
        // Not yet: the changes are a moment old.
        assert!(a.tick(&doc, true, Duration::from_secs(300), false).is_some());
        assert!(!a.writing());
        // Nor in the middle of a stroke.
        a.since = Some(Instant::now() - Duration::from_secs(301));
        assert!(a.tick(&doc, true, Duration::from_secs(300), true).is_some());
        assert!(!a.writing());
        a.tick(&doc, true, Duration::from_secs(300), false);
        a.finish();
        let written = a.current().unwrap().clone();
        // Another run, while this one runs: nothing to offer.
        assert!(Autosave::open(dir.clone()).found.is_empty());
        // This one ends without closing (its lock let go, as when a process
        // dies).
        drop(a);
        let mut next = Autosave::open(dir.clone());
        assert_eq!(next.found.len(), 1);
        let found = next.found[0].clone();
        assert_eq!(found.file, written.file);
        assert_eq!(found.info.path.as_deref(), Some(Path::new("/maps/Island.massif")));
        assert_eq!(found.info.name, doc.world.meta.name);
        // Restored: it is the new run's own.
        let world = massif_format::load(&found.file).unwrap();
        let mut restored = Document::new(world, found.info.path.clone(), 1);
        restored.mark_changed();
        next.adopt(found, 1, restored.revision);
        assert!(next.found.is_empty());
        assert_eq!(next.current().unwrap().info.session, next.session);
        drop(next);
        // Ended without closing again: offered again.
        let mut third = Autosave::open(dir.clone());
        assert_eq!(third.found.len(), 1);
        let found = third.found[0].clone();
        third.discard(&found);
        assert!(!found.file.exists());
        third.close();
        assert!(Autosave::open(dir.clone()).found.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn saving_or_closing_removes_the_backup() {
        let dir = temp_dir("save");
        let mut a = Autosave::open(dir.clone());
        let mut doc = Document::new(small_world(), None, 1);
        edit(&mut doc);
        a.tick(&doc, true, Duration::ZERO, false);
        a.finish();
        let first = a.current().unwrap().clone();
        assert!(first.file.exists());
        // Saved: gone, and nothing more to back up.
        doc.mark_saved(doc.revision, dir.join("saved.massif"));
        a.saved(doc.generation, doc.revision);
        assert!(!first.file.exists() && a.current().is_none());
        assert!(a.tick(&doc, true, Duration::ZERO, false).is_none() && !a.writing());
        // Changed again, backed up again; a newer backup replaces the older.
        edit(&mut doc);
        a.tick(&doc, true, Duration::ZERO, false);
        a.finish();
        let second = a.current().unwrap().clone();
        edit(&mut doc);
        a.tick(&doc, true, Duration::ZERO, false);
        a.finish();
        assert!(!second.file.exists() && a.current().unwrap().file.exists());
        // Off: nothing written.
        edit(&mut doc);
        a.tick(&doc, false, Duration::ZERO, false);
        assert!(!a.writing());
        // A clean quit: gone, the lock with it.
        let third = a.current().unwrap().clone();
        a.close();
        assert!(!third.file.exists());
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
        assert!(left.is_empty(), "left behind: {left:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn another_map_drops_the_last_ones_backup() {
        let dir = temp_dir("another");
        let mut a = Autosave::open(dir.clone());
        let mut doc = Document::new(small_world(), None, 1);
        edit(&mut doc);
        a.tick(&doc, true, Duration::ZERO, false);
        a.finish();
        let b = a.current().unwrap().clone();
        let doc = Document::new(small_world(), None, 2);
        a.tick(&doc, true, Duration::ZERO, false);
        assert!(!b.file.exists() && a.current().is_none());
        a.close();
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cost of a backup of a large map to the editor's thread
    /// (MASSIF_AUTOSAVE_WORLD=<a large .massif>, in release): the copy it
    /// takes there, against the snapshot a save takes, with many tiles
    /// changed; how long the backup takes to write; and a frame's worth of
    /// work on the editor's thread while it does.
    #[test]
    fn a_backup_of_a_huge_map() {
        let Some(path) = std::env::var_os("MASSIF_AUTOSAVE_WORLD").map(PathBuf::from) else {
            return;
        };
        let ms = |d: Duration| d.as_secs_f64() * 1e3;
        let mut world = massif_format::load(&path).unwrap();
        let positions: Vec<_> = world.dims[0].levels[0].positions().collect();
        println!("{} tiles", positions.len());
        let dirty = |world: &mut World, n: usize, k: f32| {
            for p in positions.iter().take(n) {
                world.dims[0].levels[0].tile_mut(*p).unwrap().top.set(0, 100.0 + k);
            }
        };
        for n in [64usize, 512, crate::doc::MAX_HOT_TILES] {
            let (mut copy_best, mut snap_best) = (Duration::MAX, Duration::MAX);
            for k in 0..3 {
                dirty(&mut world, n, k as f32);
                let t = Instant::now();
                let copy = world.copy_unfrozen();
                copy_best = copy_best.min(t.elapsed());
                drop(copy);
                let t = Instant::now();
                let snap = world.snapshot();
                snap_best = snap_best.min(t.elapsed());
                drop(snap);
            }
            println!("{n} tiles changed: the copy {:.1} ms, a save's snapshot {:.1} ms (best of 3)", ms(copy_best), ms(snap_best));
        }
        // A frame's worth of work on this thread: reading 64 tiles.
        let frame = |world: &World| {
            let t = Instant::now();
            let level = &world.dims[0].levels[0];
            let mut sum = 0.0;
            for p in positions.iter().rev().take(64) {
                sum += level.view(*p).unwrap().top.get(0);
            }
            std::hint::black_box(sum);
            t.elapsed()
        };
        let mut quiet = Vec::new();
        for _ in 0..40 {
            quiet.push(frame(&world));
        }
        let dir = temp_dir("huge");
        std::fs::create_dir_all(&dir).unwrap();
        dirty(&mut world, crate::doc::MAX_HOT_TILES, 9.0);
        let info = Info { session: "huge".into(), generation: 1, name: "huge".into(), path: None, taken: now_secs(), revision: 1 };
        let copy = world.copy_unfrozen();
        let started = Instant::now();
        let writer = write_in_background(dir.clone(), copy, info);
        let mut busy = Vec::new();
        let b = loop {
            match writer.try_recv() {
                Ok(b) => break b.unwrap(),
                Err(_) => busy.push(frame(&world)),
            }
        };
        let wrote = started.elapsed();
        let median = |v: &mut Vec<Duration>| {
            v.sort();
            v[v.len() / 2]
        };
        let worst = |v: &Vec<Duration>| v.iter().max().copied().unwrap_or_default();
        println!("written in {:.2} s ({} MB)", wrote.as_secs_f64(), b.bytes >> 20);
        println!("a frame's work alone: median {:.2} ms, worst {:.2} ms", ms(median(&mut quiet)), ms(worst(&quiet)));
        println!("while the backup is written: median {:.2} ms, worst {:.2} ms ({} frames)", ms(median(&mut busy)), ms(worst(&busy)), busy.len());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_are_kept_within_bounds() {
        let dir = temp_dir("bounds");
        std::fs::create_dir_all(&dir).unwrap();
        // Backups of runs that are over, one a minute.
        let world = small_world();
        for k in 0..8u64 {
            let info = Info { session: format!("old{k}"), generation: 1, name: format!("Map {k}"), path: None, taken: 1000 + 60 * k, revision: 1 };
            write(&dir, world.copy_unfrozen(), info).unwrap();
        }
        let left = list(&dir);
        assert_eq!(left.len(), MAX_BACKUPS);
        assert_eq!(left.iter().map(|b| b.info.name.as_str()).collect::<Vec<_>>(), ["Map 7", "Map 6", "Map 5", "Map 4", "Map 3"], "the newest kept");
        let a = Autosave::open(dir.clone());
        assert_eq!(a.found.len(), MAX_BACKUPS);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
