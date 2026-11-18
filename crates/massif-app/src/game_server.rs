//! The land Minecraft makes for a seed, made by the game itself: its
//! server, from the game installed on this computer (the client jar of a
//! launcher's version, its libraries and its Java), run without a window
//! on a world of that seed, has the chunks around the map generated, and
//! their surface is read back from its region files (see
//! `massif_anvil::read`). The worlds are kept in Massif's cache, one a
//! version and seed, so a seed's land is made once.
//!
//! Running Minecraft's server means accepting Mojang's EULA: the user does,
//! once, in the dialog that asks for the land; nothing starts before.

use massif_core::vanilla::{Land, LandChunk};
use std::collections::HashSet;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Mojang's EULA, which running the server accepts.
pub const EULA: &str = "https://aka.ms/MinecraftEULA";

/// Chunks forced loaded at once: the server keeps them in memory until
/// they are let go.
const BATCH: usize = 1024;

/// A version of the game installed by a launcher, with what its server
/// needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Game {
    pub version: String,
    pub jar: PathBuf,
    /// The jar and its libraries.
    pub classpath: Vec<PathBuf>,
    pub java: PathBuf,
}

/// The newest version installed that makes 1.18's land (the one Massif's
/// own reckoning follows) and that a launcher keeps with its libraries.
pub fn find_game() -> Option<Game> {
    massif_assets::find_jars().into_iter().find_map(|jar| {
        let numbers = massif_assets::release_numbers(&jar.version)?;
        (numbers >= vec![1, 18]).then_some(())?;
        game_of(&jar.version, &jar.path)
    })
}

/// What the server of `version`, whose client jar is `jar` in a launcher's
/// `versions/<version>/` folder, needs.
fn game_of(version: &str, jar: &Path) -> Option<Game> {
    let dir = jar.parent()?;
    let root = dir.parent()?.parent()?;
    let meta: serde_json::Value = serde_json::from_slice(&std::fs::read(dir.join(format!("{version}.json"))).ok()?).ok()?;
    let mut libraries = vec![root.join("libraries")];
    if let Some(home) = std::env::home_dir() {
        libraries.push(home.join("Library/Application Support/minecraft/libraries"));
        libraries.push(home.join(".minecraft/libraries"));
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        libraries.push(PathBuf::from(appdata).join(".minecraft/libraries"));
    }
    let os = if cfg!(target_os = "macos") { "osx" } else if cfg!(windows) { "windows" } else { "linux" };
    let mut classpath = Vec::new();
    for lib in meta.get("libraries")?.as_array()? {
        if let Some(rules) = lib.get("rules").and_then(|r| r.as_array()) {
            let mut allowed = false;
            for rule in rules {
                let applies = rule.get("os").and_then(|o| o.get("name")).and_then(|n| n.as_str()).is_none_or(|n| n == os);
                if applies {
                    allowed = rule.get("action").and_then(|a| a.as_str()) == Some("allow");
                }
            }
            if !allowed {
                continue;
            }
        }
        let Some(path) = lib.pointer("/downloads/artifact/path").and_then(|p| p.as_str()) else { continue };
        if let Some(found) = libraries.iter().map(|l| l.join(path)).find(|p| p.is_file()) {
            classpath.push(found);
        }
    }
    classpath.push(jar.to_path_buf());
    let component = meta.pointer("/javaVersion/component").and_then(|c| c.as_str()).unwrap_or("java-runtime-delta");
    let java = find_java(root, component)?;
    Some(Game { version: version.to_owned(), jar: jar.to_path_buf(), classpath, java })
}

/// The Java the launcher installed for the game (its `runtime` folder),
/// else the system's.
fn find_java(root: &Path, component: &str) -> Option<PathBuf> {
    let mut runtimes = vec![root.join("runtime")];
    if let Some(home) = std::env::home_dir() {
        runtimes.push(home.join("Library/Application Support/minecraft/runtime"));
        runtimes.push(home.join(".minecraft/runtime"));
    }
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        runtimes.push(PathBuf::from(local).join("Packages/Microsoft.4297127D64EC6_8wekyb3d8bbwe/LocalCache/Local/runtime"));
    }
    let exe = if cfg!(windows) { "java.exe" } else { "java" };
    for runtime in runtimes {
        let base = runtime.join(component);
        if let Some(java) = find_file(&base, exe, 8) {
            return Some(java);
        }
    }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(exe)).find(|p| p.is_file())
}

/// A file named `name` in a `bin` folder under `dir`, `depth` folders down
/// at most.
fn find_file(dir: &Path, name: &str, depth: usize) -> Option<PathBuf> {
    let candidate = dir.join("bin").join(name);
    if candidate.is_file() {
        return Some(candidate);
    }
    if depth == 0 {
        return None;
    }
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
    entries.sort();
    entries.into_iter().find_map(|d| find_file(&d, name, depth - 1))
}

/// Where the worlds the server makes are kept: Massif's cache.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MASSIF_GAME_CACHE") {
        return Some(PathBuf::from(dir));
    }
    crate::tiles::cache_dir().and_then(|d| d.parent().map(|p| p.join("worlds")))
}

/// The folder of the world of `seed` made by `version`.
fn world_dir(version: &str, seed: i64) -> Option<PathBuf> {
    Some(cache_dir()?.join(massif_core::mask::file_safe(version)).join(seed.to_string()))
}

/// How far a making of land has gone.
#[derive(Default)]
pub struct Progress {
    pub chunks: AtomicUsize,
    pub total: AtomicUsize,
    pub cancel: AtomicBool,
    /// What the server is doing, in a few words.
    pub step: Mutex<String>,
}

/// The land of a seed being made in the background.
pub struct Making {
    pub seed: i64,
    pub version: String,
    pub progress: Arc<Progress>,
    pub done: Receiver<Result<Land, String>>,
    pub started: Instant,
}

impl Making {
    pub fn stop(&self) {
        self.progress.cancel.store(true, Ordering::Relaxed);
    }
}

/// The chunks of the land around `rect` (blocks), out to `reach` blocks
/// past it: a ring, the map's own chunks left out.
pub fn ring(rect: massif_core::BlockRect, reach: i32) -> Vec<(i32, i32)> {
    let outer = rect.expand(reach);
    let (cx0, cz0, cx1, cz1) = (outer.x0.div_euclid(16), outer.z0.div_euclid(16), (outer.x1 - 1).div_euclid(16), (outer.z1 - 1).div_euclid(16));
    let (ix0, iz0, ix1, iz1) = ((rect.x0 + 16).div_euclid(16), (rect.z0 + 16).div_euclid(16), (rect.x1 - 17).div_euclid(16), (rect.z1 - 17).div_euclid(16));
    let mut out = Vec::new();
    for cz in cz0..=cz1 {
        for cx in cx0..=cx1 {
            let inside = cx >= ix0 && cx <= ix1 && cz >= iz0 && cz <= iz1;
            if !inside {
                out.push((cx, cz));
            }
        }
    }
    out
}

/// Make the land of `seed` over `chunks` with `game` in the background:
/// what the cache has already is read, the rest generated. The biomes are
/// numbered as `reg` numbers them.
pub fn make(game: Game, seed: i64, chunks: Vec<(i32, i32)>, reg: &massif_core::Registry, ctx: &egui::Context) -> Making {
    let progress = Arc::new(Progress::default());
    let (tx, rx) = channel();
    let p = progress.clone();
    let version = game.version.clone();
    let names: Vec<(String, u16)> = reg.biomes.iter().enumerate().filter_map(|(i, b)| b.id.strip_prefix("minecraft:").map(|n| (n.to_owned(), i as u16))).collect();
    let ctx = ctx.clone();
    let _ = std::thread::Builder::new().name("massif game server".into()).spawn(move || {
        let out = run(&game, seed, &chunks, &names, &p);
        let _ = tx.send(out);
        ctx.request_repaint();
    });
    Making { seed, version, progress, done: rx, started: Instant::now() }
}

fn say(p: &Progress, step: &str) {
    *p.step.lock().unwrap_or_else(|e| e.into_inner()) = step.to_owned();
}

/// The land read from a world's region files.
fn read_land(dir: &Path, seed: i64, names: &[(String, u16)]) -> Land {
    let mut land = Land { seed, ..Land::default() };
    let Ok(surfaces) = massif_anvil::read::world_surfaces(dir) else { return land };
    for s in surfaces {
        let numbers: Vec<u16> = s.biomes.iter().map(|b| names.iter().find(|n| n.0 == *b).map_or(0, |n| n.1)).collect();
        land.chunks.insert((s.x, s.z), LandChunk { ground: s.ground, fluid: s.fluid, biome: s.biome.map(|b| numbers.get(b as usize).copied().unwrap_or(0)) });
    }
    land
}

fn run(game: &Game, seed: i64, chunks: &[(i32, i32)], names: &[(String, u16)], p: &Progress) -> Result<Land, String> {
    let dir = world_dir(&game.version, seed).ok_or_else(|| "no cache folder".to_owned())?;
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    say(p, "reading");
    let land = read_land(&dir.join("world"), seed, names);
    let missing: Vec<(i32, i32)> = chunks.iter().copied().filter(|c| !land.chunks.contains_key(c)).collect();
    p.total.store(chunks.len(), Ordering::Relaxed);
    p.chunks.store(chunks.len() - missing.len(), Ordering::Relaxed);
    if missing.is_empty() {
        return Ok(land);
    }
    std::fs::write(dir.join("eula.txt"), "eula=true\n").map_err(|e| e.to_string())?;
    let port = std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(25599);
    std::fs::write(
        dir.join("server.properties"),
        format!(
            "level-name=world\nlevel-seed={seed}\nonline-mode=false\nserver-ip=127.0.0.1\nserver-port={port}\nmax-players=1\nenable-rcon=false\nenable-query=false\nview-distance=2\nsimulation-distance=2\nmax-tick-time=-1\nspawn-protection=0\nsync-chunk-writes=false\n"
        ),
    )
    .map_err(|e| e.to_string())?;
    say(p, "starting");
    let classpath = std::env::join_paths(&game.classpath).map_err(|e| e.to_string())?;
    let mut command = Command::new(&game.java);
    command.args(["-Xmx3G", "-cp"]).arg(classpath).args(["net.minecraft.server.Main", "--nogui"]).current_dir(&dir).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let mut server = Server::start(command)?;
    let result = (|| {
        server.wait_for("Done (", Duration::from_secs(300), p)?;
        say(p, "generating");
        let mut done = chunks.len() - missing.len();
        for batch in missing.chunks(BATCH) {
            for &(cx, cz) in batch {
                server.send(&format!("forceload add {} {}", cx * 16, cz * 16))?;
            }
            // Each forceload loads its chunk before the next is read: a
            // marker after them comes once they all are.
            server.send("say MASSIF batch")?;
            server.wait_for("MASSIF batch", Duration::from_secs(1800), p)?;
            for &(cx, cz) in batch {
                server.send(&format!("forceload remove {} {}", cx * 16, cz * 16))?;
            }
            done += batch.len();
            p.chunks.store(done, Ordering::Relaxed);
        }
        say(p, "saving");
        server.send("save-all flush")?;
        server.wait_for("Saved the game", Duration::from_secs(300), p)?;
        Ok(())
    })();
    server.stop();
    result?;
    say(p, "reading");
    Ok(read_land(&dir.join("world"), seed, names))
}

/// A server running, its log read line by line.
struct Server {
    child: Child,
    lines: Receiver<String>,
}

impl Server {
    fn start(mut command: Command) -> Result<Self, String> {
        let mut child = command.spawn().map_err(|e| format!("Java could not start: {e}"))?;
        let out = child.stdout.take().ok_or("no output")?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    break;
                }
            }
        });
        Ok(Self { child, lines: rx })
    }

    fn send(&mut self, command: &str) -> Result<(), String> {
        let stdin = self.child.stdin.as_mut().ok_or("no input")?;
        writeln!(stdin, "{command}").and_then(|_| stdin.flush()).map_err(|_| "the server stopped".to_owned())
    }

    fn wait_for(&mut self, text: &str, most: Duration, p: &Progress) -> Result<(), String> {
        let end = Instant::now() + most;
        let mut last = String::new();
        while Instant::now() < end {
            if p.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            match self.lines.recv_timeout(Duration::from_millis(200)) {
                Ok(line) => {
                    if line.contains(text) {
                        return Ok(());
                    }
                    if line.contains("ERROR") || line.contains("Exception") {
                        last = line;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return Err(if last.is_empty() { "the server stopped".into() } else { format!("the server stopped: {last}") }),
            }
        }
        Err("the server took too long".into())
    }

    fn stop(&mut self) {
        let _ = self.send("stop");
        let end = Instant::now() + Duration::from_secs(60);
        while Instant::now() < end {
            if let Ok(Some(_)) = self.child.try_wait() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        let _ = self.child.kill();
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
    }
}

/// The chunks of `chunks` the land has.
pub fn known(land: &Land, chunks: &[(i32, i32)]) -> usize {
    let set: HashSet<&(i32, i32)> = land.chunks.keys().collect();
    chunks.iter().filter(|c| set.contains(c)).count()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_ring_leaves_the_map_out() {
        let rect = massif_core::BlockRect::new(0, 0, 256, 256);
        let ring = ring(rect, 64);
        assert!(ring.contains(&(-1, -1)) && ring.contains(&(-4, 0)) && ring.contains(&(19, 19)));
        assert!(!ring.contains(&(8, 8)) && !ring.contains(&(-5, 0)) && !ring.contains(&(20, 0)));
        // The map's own edge chunks are in: the band starts on them.
        assert!(ring.contains(&(0, 0)) && ring.contains(&(15, 7)));
        assert_eq!(ring.len(), 24 * 24 - 14 * 14);
    }

    /// The server of the game installed makes a seed's land
    /// (`MASSIF_GAME_SERVER=1`; it accepts Mojang's EULA).
    #[test]
    #[ignore]
    fn the_game_makes_the_land() {
        if std::env::var_os("MASSIF_GAME_SERVER").is_none() {
            return;
        }
        let game = find_game().expect("a game installed");
        eprintln!("{} with {}", game.version, game.java.display());
        let reg = massif_core::Registry::vanilla();
        let names: Vec<(String, u16)> = reg.biomes.iter().enumerate().filter_map(|(i, b)| b.id.strip_prefix("minecraft:").map(|n| (n.to_owned(), i as u16))).collect();
        let chunks = ring(massif_core::BlockRect::new(0, 0, 256, 256), 64);
        let p = Progress::default();
        let t = Instant::now();
        let land = run(&game, 4242, &chunks, &names, &p).unwrap();
        eprintln!("{} chunks in {:.1} s", land.chunks.len(), t.elapsed().as_secs_f32());
        assert_eq!(known(&land, &chunks), chunks.len());
        let t = Instant::now();
        let again = run(&game, 4242, &chunks, &names, &p).unwrap();
        assert_eq!(known(&again, &chunks), chunks.len());
        eprintln!("from the cache in {:.2} s", t.elapsed().as_secs_f32());
    }
}
