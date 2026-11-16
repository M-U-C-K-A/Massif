//! Elevation tiles fetched from the web, kept on disk: the Terrain Tiles of
//! Mapzen on AWS (see `massif_core::real`), and a place search.
//!
//! Polite to the servers: a few requests at a time, a User-Agent naming
//! Massif and nothing about the user, every tile asked for once and then
//! read from the disk, a pause after a refusal, and none at all for a
//! while once the network is down. Offline, what is on disk still serves.

use massif_core::real::{DemTile, TileKey, decode_terrarium};
use std::collections::{HashMap, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Requests to the tile server at once.
const AT_ONCE: usize = 4;
/// Threads fetching the tiles the dialog shows.
const UI_WORKERS: usize = 2;
/// Tiles kept in memory for the dialog.
const KEEP: usize = 600;
/// How long to wait before trying the network again once it failed.
const OFFLINE_FOR: Duration = Duration::from_secs(30);
/// The disk cache's size it is trimmed to, oldest tiles first.
const CACHE_BYTES: u64 = 2 << 30;

/// Massif, its version and its page, and nothing about who uses it.
pub fn user_agent() -> String {
    format!("Massif/{} (+https://github.com/M-U-C-K-A/Massif)", env!("CARGO_PKG_VERSION"))
}

/// Where the tiles are kept: the system's folder of caches.
pub fn cache_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("MASSIF_TILE_CACHE") {
        return Some(PathBuf::from(dir));
    }
    let var = |k: &str| std::env::var_os(k).map(PathBuf::from).filter(|p| p.is_absolute());
    let root = if cfg!(windows) {
        var("LOCALAPPDATA").map(|p| p.join("Massif").join("cache"))
    } else if cfg!(target_os = "macos") {
        std::env::home_dir().map(|h| h.join("Library").join("Caches").join("Massif"))
    } else {
        var("XDG_CACHE_HOME").or_else(|| std::env::home_dir().map(|h| h.join(".cache"))).map(|p| p.join("massif"))
    };
    root.map(|r| r.join("terrarium"))
}

enum Slot {
    Ready(Arc<DemTile>),
    Asked,
    /// Could not be had (at that moment).
    Failed(Instant),
}

/// The tiles: in memory, on disk, or from the web.
pub struct TileStore {
    dir: Option<PathBuf>,
    online: bool,
    slots: Mutex<HashMap<TileKey, Slot>>,
    /// Order the tiles in memory came in, to forget the oldest.
    order: Mutex<VecDeque<TileKey>>,
    queue: Mutex<Vec<TileKey>>,
    wake: Condvar,
    workers: OnceLock<()>,
    /// Requests under way, at most [`AT_ONCE`].
    busy: Mutex<usize>,
    free: Condvar,
    offline_since: Mutex<Option<Instant>>,
    ctx: Mutex<Option<egui::Context>>,
    trimmed: AtomicBool,
    /// Tiles read from the web, for tests and the status line.
    pub downloaded: AtomicUsize,
}

impl TileStore {
    pub fn new(dir: Option<PathBuf>, online: bool) -> Self {
        Self {
            dir,
            online,
            slots: Mutex::default(),
            order: Mutex::default(),
            queue: Mutex::default(),
            wake: Condvar::new(),
            workers: OnceLock::new(),
            busy: Mutex::new(0),
            free: Condvar::new(),
            offline_since: Mutex::new(None),
            ctx: Mutex::new(None),
            trimmed: AtomicBool::new(false),
            downloaded: AtomicUsize::new(0),
        }
    }

    /// The app's: the cache folder of the system; tests never go online.
    pub fn global() -> Arc<TileStore> {
        static STORE: OnceLock<Arc<TileStore>> = OnceLock::new();
        STORE
            .get_or_init(|| {
                let online = !cfg!(test) && std::env::var_os("MASSIF_OFFLINE").is_none();
                // Tests read only the tiles they put in memory.
                Arc::new(TileStore::new(if cfg!(test) { None } else { cache_dir() }, online))
            })
            .clone()
    }

    /// Whether the network failed a moment ago.
    pub fn offline(&self) -> bool {
        !self.online || self.offline_since.lock().expect("not poisoned").is_some_and(|t| t.elapsed() < OFFLINE_FOR)
    }

    /// Put a tile in memory (tests, fixtures).
    #[cfg(test)]
    pub fn insert(&self, key: TileKey, tile: Arc<DemTile>) {
        self.slots.lock().expect("not poisoned").insert(key, Slot::Ready(tile));
        self.order.lock().expect("not poisoned").push_back(key);
    }

    /// The tile when it is in memory; else asked for in the background
    /// (the dialog is drawn again when it comes). `None` meanwhile, or
    /// when it could not be had.
    pub fn get(self: &Arc<Self>, key: TileKey, ctx: &egui::Context) -> Option<Arc<DemTile>> {
        {
            let mut slots = self.slots.lock().expect("not poisoned");
            match slots.get(&key) {
                Some(Slot::Ready(t)) => return Some(t.clone()),
                Some(Slot::Asked) => return None,
                Some(Slot::Failed(when)) if when.elapsed() < OFFLINE_FOR => return None,
                _ => {}
            }
            slots.insert(key, Slot::Asked);
        }
        *self.ctx.lock().expect("not poisoned") = Some(ctx.clone());
        self.queue.lock().expect("not poisoned").push(key);
        self.wake.notify_one();
        self.workers.get_or_init(|| {
            for k in 0..UI_WORKERS {
                let me = self.clone();
                let _ = std::thread::Builder::new().name(format!("massif tiles {k}")).spawn(move || me.work());
            }
        });
        None
    }

    /// Whether a tile was asked for and could not be had.
    pub fn failed(&self, key: TileKey) -> bool {
        matches!(self.slots.lock().expect("not poisoned").get(&key), Some(Slot::Failed(_)))
    }

    /// Forget what is asked for and not begun: the dialog moved on.
    pub fn forget_queue(&self) {
        let mut slots = self.slots.lock().expect("not poisoned");
        for key in self.queue.lock().expect("not poisoned").drain(..) {
            slots.remove(&key);
        }
    }

    fn work(self: Arc<Self>) {
        loop {
            let key = {
                let mut queue = self.queue.lock().expect("not poisoned");
                loop {
                    // The last asked first: what is in view now.
                    if let Some(k) = queue.pop() {
                        break k;
                    }
                    queue = self.wake.wait(queue).expect("not poisoned");
                }
            };
            let slot = match self.load(key) {
                Ok(t) => Slot::Ready(t),
                Err(e) => {
                    log::info!("tile {}/{}/{}: {e}", key.z, key.x, key.y);
                    Slot::Failed(Instant::now())
                }
            };
            {
                let mut slots = self.slots.lock().expect("not poisoned");
                slots.insert(key, slot);
                let mut order = self.order.lock().expect("not poisoned");
                order.push_back(key);
                while order.len() > KEEP {
                    if let Some(old) = order.pop_front() {
                        slots.remove(&old);
                    }
                }
            }
            if let Some(ctx) = self.ctx.lock().expect("not poisoned").as_ref() {
                ctx.request_repaint();
            }
        }
    }

    fn path(&self, key: TileKey) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(key.z.to_string()).join(key.x.to_string()).join(format!("{}.png", key.y)))
    }

    /// A tile from memory, the disk or the web, waiting for it.
    pub fn load(&self, key: TileKey) -> Result<Arc<DemTile>, String> {
        if let Some(Slot::Ready(t)) = self.slots.lock().expect("not poisoned").get(&key) {
            return Ok(t.clone());
        }
        let path = self.path(key);
        if let Some(p) = &path
            && let Ok(bytes) = std::fs::read(p)
        {
            match decode_terrarium(&bytes) {
                Ok(t) => return Ok(Arc::new(t)),
                Err(_) => {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        if !self.online {
            return Err("offline".into());
        }
        if self.offline() {
            return Err("the network did not answer a moment ago".into());
        }
        let bytes = self.download(key)?;
        let tile = decode_terrarium(&bytes)?;
        if let Some(p) = &path {
            if let Some(dir) = p.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            let tmp = p.with_extension(format!("{}.tmp", std::process::id()));
            if std::fs::write(&tmp, &bytes).is_ok() {
                let _ = std::fs::rename(&tmp, p);
            }
            if let Some(root) = &self.dir
                && !self.trimmed.swap(true, Ordering::Relaxed)
            {
                let root = root.clone();
                let _ = std::thread::Builder::new().name("massif tile cache".into()).spawn(move || trim(&root, CACHE_BYTES));
            }
        }
        Ok(Arc::new(tile))
    }

    fn download(&self, key: TileKey) -> Result<Vec<u8>, String> {
        let _turn = Turn::take(self);
        let mut wait = Duration::from_secs(2);
        for attempt in 0..3 {
            match agent().get(&key.url()).call() {
                Ok(mut answer) => {
                    let bytes = answer.body_mut().with_config().limit(4 << 20).read_to_vec().map_err(|e| e.to_string())?;
                    self.downloaded.fetch_add(1, Ordering::Relaxed);
                    return Ok(bytes);
                }
                // Busy or refusing for now: wait, then once more.
                Err(ureq::Error::StatusCode(code)) if (code == 429 || code >= 500) && attempt < 2 => {
                    std::thread::sleep(wait);
                    wait *= 2;
                }
                Err(ureq::Error::StatusCode(code)) => return Err(format!("the server answered {code}")),
                Err(e) => {
                    *self.offline_since.lock().expect("not poisoned") = Some(Instant::now());
                    return Err(e.to_string());
                }
            }
        }
        Err("the server stayed busy".into())
    }

    /// Every tile of `keys`, waiting for them: `progress` hears how many are
    /// had; stops when `cancel` is set. An error names how many could not be
    /// had, and why the first could not.
    pub fn load_all(&self, keys: &[TileKey], progress: &AtomicUsize, cancel: &AtomicBool) -> Result<HashMap<TileKey, Arc<DemTile>>, String> {
        let next = AtomicUsize::new(0);
        let out = Mutex::new(HashMap::with_capacity(keys.len()));
        let errors = Mutex::new(Vec::new());
        std::thread::scope(|s| {
            for _ in 0..AT_ONCE {
                s.spawn(|| {
                    loop {
                        if cancel.load(Ordering::Relaxed) {
                            return;
                        }
                        let k = next.fetch_add(1, Ordering::Relaxed);
                        let Some(&key) = keys.get(k) else { return };
                        match self.load(key) {
                            Ok(t) => {
                                out.lock().expect("not poisoned").insert(key, t);
                            }
                            Err(e) => errors.lock().expect("not poisoned").push(e),
                        }
                        progress.fetch_add(1, Ordering::Relaxed);
                    }
                });
            }
        });
        let errors = errors.into_inner().expect("not poisoned");
        if let Some(first) = errors.first() {
            let offline = !self.online || self.offline();
            let why = if offline { format!("offline, and not downloaded before ({first})") } else { first.clone() };
            return Err(format!("{} of {} tiles of elevation could not be had: {why}", errors.len(), keys.len()));
        }
        Ok(out.into_inner().expect("not poisoned"))
    }
}

/// A turn among the requests under way.
struct Turn<'a>(&'a TileStore);

impl<'a> Turn<'a> {
    fn take(store: &'a TileStore) -> Self {
        let mut busy = store.busy.lock().expect("not poisoned");
        while *busy >= AT_ONCE {
            busy = store.free.wait(busy).expect("not poisoned");
        }
        *busy += 1;
        Turn(store)
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        *self.0.busy.lock().expect("not poisoned") -= 1;
        self.0.free.notify_one();
    }
}

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| ureq::Agent::config_builder().timeout_global(Some(Duration::from_secs(20))).user_agent(user_agent()).build().into())
}

/// Keep the cache under `max` bytes, the tiles read longest ago gone first.
fn trim(root: &Path, max: u64) {
    let mut files = Vec::new();
    let mut dirs = vec![root.to_path_buf()];
    while let Some(d) = dirs.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(meta) = e.metadata() else { continue };
            if meta.is_dir() {
                dirs.push(e.path());
            } else {
                let when = meta.accessed().or_else(|_| meta.modified()).unwrap_or(std::time::UNIX_EPOCH);
                files.push((when, meta.len(), e.path()));
            }
        }
    }
    let mut total: u64 = files.iter().map(|f| f.1).sum();
    if total <= max {
        return;
    }
    files.sort();
    for (_, len, path) in files {
        if total <= max * 9 / 10 {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
        }
    }
}

/// A place found by name.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub lat: f64,
    pub lon: f64,
}

/// Latitude and longitude typed as two numbers ("45.83, 6.86",
/// "45.83 6.86", "45.83N 6.86E").
pub fn parse_coords(text: &str) -> Option<(f64, f64)> {
    let parts: Vec<&str> = text.split(|c: char| c == ',' || c == ';' || c.is_whitespace()).filter(|p| !p.is_empty()).collect();
    if parts.len() != 2 {
        return None;
    }
    let read = |p: &str, neg: char, pos: char| -> Option<f64> {
        let p = p.trim_end_matches('°');
        let (num, sign) = match p.chars().last()?.to_ascii_uppercase() {
            c if c == neg => (&p[..p.len() - 1], -1.0),
            c if c == pos => (&p[..p.len() - 1], 1.0),
            _ => (p, 1.0),
        };
        num.trim_end_matches('°').parse::<f64>().ok().map(|v| v * sign)
    };
    let (lat, lon) = (read(parts[0], 'S', 'N')?, read(parts[1], 'W', 'E')?);
    ((-90.0..=90.0).contains(&lat) && (-180.0..=180.0).contains(&lon)).then_some((lat, lon))
}

/// `text` for a URL's query.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for b in text.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Places by name, from OpenStreetMap's Nominatim: asked only when the
/// user searches, a second apart at least, as its policy asks.
pub fn search(query: &str) -> Result<Vec<Place>, String> {
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    {
        let mut last = LAST.lock().expect("not poisoned");
        if let Some(t) = *last
            && t.elapsed() < Duration::from_secs(1)
        {
            std::thread::sleep(Duration::from_secs(1) - t.elapsed());
        }
        *last = Some(Instant::now());
    }
    let url = format!("https://nominatim.openstreetmap.org/search?format=jsonv2&limit=6&q={}", encode(query));
    let body = agent().get(&url).call().map_err(|e| e.to_string())?.body_mut().read_to_string().map_err(|e| e.to_string())?;
    parse_places(&body)
}

fn parse_places(body: &str) -> Result<Vec<Place>, String> {
    let found: Vec<serde_json::Value> = serde_json::from_str(body).map_err(|e| format!("an answer not understood: {e}"))?;
    Ok(found
        .iter()
        .filter_map(|p| {
            let num = |k: &str| p.get(k).and_then(|v| v.as_str().and_then(|s| s.parse::<f64>().ok()).or_else(|| v.as_f64()));
            Some(Place {
                name: p.get("display_name")?.as_str()?.to_owned(),
                lat: num("lat")?,
                lon: num("lon")?,
            })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::real::{TILE_PX, encode_terrarium};

    #[test]
    fn coordinates_typed() {
        assert_eq!(parse_coords("45.8326, 6.8652"), Some((45.8326, 6.8652)));
        assert_eq!(parse_coords("45.8326 6.8652"), Some((45.8326, 6.8652)));
        assert_eq!(parse_coords("33.9S 18.4E"), Some((-33.9, 18.4)));
        assert_eq!(parse_coords("27.98°N, 86.92°E"), Some((27.98, 86.92)));
        assert_eq!(parse_coords("Mont Blanc"), None);
        assert_eq!(parse_coords("95, 10"), None);
        assert_eq!(encode("Mont Blanc, été"), "Mont+Blanc%2C+%C3%A9t%C3%A9");
    }

    #[test]
    fn places_read_from_an_answer() {
        let body = r#"[{"display_name":"Mont Blanc, France","lat":"45.8326","lon":"6.8652"},{"display_name":"no place"}]"#;
        assert_eq!(parse_places(body).unwrap(), vec![Place { name: "Mont Blanc, France".into(), lat: 45.8326, lon: 6.8652 }]);
        assert!(parse_places("<html>").is_err());
    }

    #[test]
    fn tiles_come_from_the_disk_offline() {
        let dir = std::env::temp_dir().join(format!("massif-tiles-{}", std::process::id()));
        let key = TileKey { z: 10, x: 530, y: 360 };
        let path = dir.join("10").join("530").join("360.png");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let metres: Vec<f32> = (0..TILE_PX * TILE_PX).map(|k| (k % TILE_PX) as f32).collect();
        std::fs::write(&path, encode_terrarium(&metres)).unwrap();
        let store = TileStore::new(Some(dir.clone()), false);
        let tile = store.load(key).unwrap();
        assert_eq!(tile.at(200, 3), 200.0);
        // Another, not on disk: a clean error, no network.
        let (had, cancel) = (AtomicUsize::new(0), AtomicBool::new(false));
        let err = store.load_all(&[key, TileKey { z: 10, x: 531, y: 360 }], &had, &cancel).unwrap_err();
        assert!(err.starts_with("1 of 2 tiles") && err.contains("offline"), "{err}");
        assert_eq!(had.load(Ordering::Relaxed), 2);
        assert_eq!(store.downloaded.load(Ordering::Relaxed), 0);
        // A broken file on disk is dropped.
        std::fs::write(&path, b"broken").unwrap();
        assert!(store.load(key).is_err() && !path.exists());
        let _ = std::fs::remove_dir_all(dir);
    }

    /// The real server, by hand: `cargo test -p massif-app -- --ignored real_tiles_download`.
    #[test]
    #[ignore = "goes online"]
    fn real_tiles_download() {
        use massif_core::real::{Area, Samples, Tiles, metres_per_px};
        let dir = std::env::temp_dir().join(format!("massif-tiles-online-{}", std::process::id()));
        let store = TileStore::new(Some(dir.clone()), true);
        // Mont Blanc, 4808 m, a block every 10 m.
        let area = Area { lat: 45.8326, lon: 6.8652, blocks_per_metre: 0.1, width: 1024, depth: 1024 };
        let z = area.zoom(massif_core::real::MAX_TILES);
        let keys = area.tiles(z);
        let started = Instant::now();
        let tiles = store.load_all(&keys, &AtomicUsize::new(0), &AtomicBool::new(false)).unwrap();
        eprintln!("zoom {z} ({:.1} m a pixel), {} tiles in {:.2} s", metres_per_px(area.lat, z), keys.len(), started.elapsed().as_secs_f32());
        let span = Samples::read(&area, &Tiles { z, tiles }, 1.0).span().unwrap();
        eprintln!("{span:?}");
        assert!((4700.0..4850.0).contains(&span.high), "{span:?}");
        // Again, from the disk, offline.
        let offline = TileStore::new(Some(dir.clone()), false);
        assert!(offline.load_all(&keys, &AtomicUsize::new(0), &AtomicBool::new(false)).is_ok());
        eprintln!("{:?}", search("Mont Blanc").unwrap().first());
        let _ = std::fs::remove_dir_all(dir);
    }
}
