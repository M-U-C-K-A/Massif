//! Scripts: WorldPainter's JavaScript scripts run as they are (on QuickJS),
//! and Massif's own scripting API, in Lua (Luau).
//!
//! [`run`] runs a script on a thread of its own caller's choosing: the
//! command line's `massif-cli script`, or the app's File ▸ Run script…,
//! which gives it the open map and takes it back as one step to undo.
//!
//! * `wp`: WorldPainter's `ScriptingContext` and the Java objects its
//!   scripts use (`world`, `dimension`, `org.pepsoft.worldpainter.Terrain`,
//!   layers…), over Massif's map ([`session`], [`apply`], `wp.js`);
//! * Massif's own API, in Lua: maps read and written by regions
//!   ([`region`]), whole maps opened, made, saved, exported and changed
//!   ([`api`]), through thin bindings (`lua.rs`) over the same operations.
//!
//! A script reads and writes only files under its own folder and those the
//! user picked ([`access`]), and has no network.

pub mod access;
pub mod api;
pub mod apply;
pub mod descriptor;
pub mod image;
mod js;
mod lua;
pub mod region;
pub mod session;
#[doc(hidden)]
pub mod testing;
pub mod worlds;

pub use descriptor::{Descriptor, Param, ParamKind, Value as ParamValue};
pub use js::ScriptError;
pub use session::{Line, Progress};

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, Instant};

/// A script: its file and source.
#[derive(Debug, Clone)]
pub struct Script {
    pub path: PathBuf,
    pub source: String,
}

impl Script {
    pub fn load(path: &Path) -> Result<Script, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
        // WorldPainter reads scripts in the platform's charset; most are
        // UTF-8, and the rest Latin-1.
        let source = String::from_utf8(bytes).unwrap_or_else(|e| e.into_bytes().iter().map(|&b| b as char).collect());
        Ok(Script { path: path.to_path_buf(), source })
    }

    pub fn descriptor(&self) -> Descriptor {
        Descriptor::parse(&self.source)
    }

    /// The script's file name, as its stack shows it.
    pub fn file_name(&self) -> String {
        self.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "script.js".into())
    }

    /// The folder the script is in, absolute.
    pub fn folder(&self) -> PathBuf {
        let absolute = std::path::absolute(&self.path).unwrap_or_else(|_| self.path.clone());
        absolute.parent().map(Path::to_path_buf).unwrap_or_default()
    }
}

/// What a run takes besides the script.
pub struct Options {
    /// `arguments` (and `argv` after the script's name).
    pub args: Vec<String>,
    /// `params`, the descriptor's parameters.
    pub params: Vec<(String, ParamValue)>,
    /// Where relative paths start: the script's folder in the app, the
    /// current folder on the command line.
    pub base: PathBuf,
    /// Files and folders the user picked, besides the script's folder.
    pub allow: Vec<PathBuf>,
    /// The map open in Massif, bound as `world`, `dimension` and
    /// `massif.world`, with the index in `dims` of the dimension shown.
    pub world: Option<(massif_core::World, usize)>,
    /// Its file, if it has one.
    pub world_path: Option<PathBuf>,
    pub out: Arc<dyn Fn(Line) + Send + Sync>,
    pub progress: Arc<Progress>,
    pub cancel: Arc<AtomicBool>,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    Done,
    /// The script called `exit(code)`.
    Exit(i32),
    Cancelled,
    Failed(ScriptError),
}

/// What a run gives back.
pub struct Outcome {
    pub ending: Ending,
    /// The map given in [`Options::world`], as the script left it.
    pub world: Option<massif_core::World>,
    /// Whether the script changed it.
    pub changed: bool,
    pub elapsed: Duration,
}

/// Run `script`. The map given is changed in place of a copy: a caller
/// that wants to undo it keeps its own copy (cheap: tiles are shared).
pub fn run(script: &Script, options: Options) -> Outcome {
    let started = Instant::now();
    let io = session::Io { out: options.out.clone(), progress: options.progress.clone(), cancel: options.cancel.clone() };
    let folder = access::resolve(&script.folder());
    let mut access = access::Access { base: options.base.clone(), saves: massif_anvil::export::default_saves_dir(), ..Default::default() };
    access.allow(&folder);
    for p in &options.allow {
        access.allow(p);
    }
    for (_, v) in &options.params {
        if let ParamValue::File(Some(p)) = v {
            access.allow(p);
        }
    }
    let mut session = session::Session::new(io, access);
    let mut open = None;
    if let Some((world, dim)) = options.world {
        let kind = world.dims.get(dim).map_or(0, |d| d.kind as i32);
        let w = session.add_world(world, options.world_path.clone());
        session.open = Some(w);
        open = Some((w, kind));
    }
    let bindings = js::Bindings { args: options.args.clone(), params: options.params.clone(), script_dir: folder.display().to_string(), open };
    let lua = script.path.extension().is_some_and(|e| e.eq_ignore_ascii_case("lua") || e.eq_ignore_ascii_case("luau"));
    let (mut session, end) = if lua { lua::run(session, &script.source, &script.file_name(), &bindings) } else { js::run(session, &script.source, &script.file_name(), &bindings) };
    let mut ending = match end {
        js::End::Done => Ending::Done,
        js::End::Exit(code) => Ending::Exit(code),
        js::End::Cancelled => Ending::Cancelled,
        js::End::Failed(e) => Ending::Failed(e),
    };
    // What waits is applied once the script is done, unless it failed.
    if matches!(ending, Ending::Done | Ending::Exit(_))
        && let Err(e) = session.finish()
    {
        ending = if e == session::CANCELLED {
            Ending::Cancelled
        } else {
            Ending::Failed(ScriptError { line: e.rfind(" of line ").and_then(|k| e[k + 9..].trim_end_matches(')').parse().ok()), message: e, column: None, stack: String::new() })
        };
    }
    let (world, changed) = match session.open.and_then(|w| session.worlds.get_mut(w)).and_then(Option::take) {
        Some(slot) => (Some(slot.world), slot.changed),
        None => (None, false),
    };
    Outcome { ending, world, changed, elapsed: started.elapsed() }
}
