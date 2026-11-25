//! The open map for other programs of this computer (the MCP server a model
//! edits with): one connection a request, a line of JSON each way, on a
//! port of the loopback address alone. The port, and a token a request must
//! carry, are in `bridge/<process id>.json` beside the log, which only the
//! user's own programs read. `MASSIF_BRIDGE=0` turns it off.
//!
//! Maps cross as `.massif` files, written and read on the connection's
//! thread; the app's only hands over a copy of the map, and takes one back
//! as one step to undo (see `MassifApp::poll_bridge`).

use massif_core::World;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, Sender, SyncSender, channel, sync_channel};
use std::time::Duration;

/// What is asked of the app.
pub enum Ask {
    Status,
    /// A copy of the map.
    Map,
    /// Take this map, made from the one of `generation` and `revision`.
    Apply { world: Box<World>, label: String, generation: u64, revision: u64 },
    Undo,
    Redo,
    /// Save to the map's own file.
    Save,
}

pub struct Answer {
    pub status: Value,
    pub map: Option<Box<World>>,
}

pub struct Request {
    pub ask: Ask,
    pub reply: SyncSender<Result<Answer, String>>,
}

/// Listening, until dropped.
pub struct Bridge {
    pub requests: Receiver<Request>,
    port: u16,
    token: String,
    file: Option<PathBuf>,
}

/// The longest the app may take to answer: it may be writing a file.
const PATIENCE: Duration = Duration::from_secs(60);
const LONGEST_LINE: u64 = 1 << 16;

impl Bridge {
    /// Listen, and say where in the app's folder.
    pub fn start(ctx: &egui::Context) -> Option<Bridge> {
        if std::env::var_os("MASSIF_BRIDGE").is_some_and(|v| v == "0") {
            return None;
        }
        let ctx = ctx.clone();
        let mut bridge = match Self::listen(move || ctx.request_repaint()) {
            Ok(b) => b,
            Err(e) => {
                crate::startup::step(&format!("other programs cannot reach the map: {e}"));
                return None;
            }
        };
        let dir = crate::startup::log_dir()?.join("bridge");
        let file = dir.join(format!("{}.json", std::process::id()));
        let said = json!({ "port": bridge.port, "token": bridge.token, "pid": std::process::id(), "version": env!("CARGO_PKG_VERSION") }).to_string();
        if let Err(e) = write_private(&dir, &file, &said) {
            crate::startup::step(&format!("other programs cannot reach the map: {e}"));
            return None;
        }
        bridge.file = Some(file);
        Some(bridge)
    }

    fn listen(wake: impl Fn() + Clone + Send + 'static) -> std::io::Result<Bridge> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let port = listener.local_addr()?.port();
        let token = token();
        let (tx, requests) = channel();
        let expected = token.clone();
        std::thread::Builder::new().name("bridge".into()).spawn(move || {
            for stream in listener.incoming().flatten() {
                let (tx, expected, wake) = (tx.clone(), expected.clone(), wake.clone());
                let _ = std::thread::Builder::new().name("bridge request".into()).spawn(move || serve(stream, &expected, &tx, &wake));
            }
        })?;
        Ok(Bridge { requests, port, token, file: None })
    }
}

impl Drop for Bridge {
    fn drop(&mut self) {
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
    }
}

/// `text` in `file`, for the user alone (on Windows, their local app data
/// is theirs already).
fn write_private(dir: &Path, file: &Path, text: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(file)?.write_all(text.as_bytes())
}

/// 256 bits that cannot be guessed: the keys of the standard library's
/// hashers come from the system's generator.
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    (0..4u64)
        .map(|k| {
            let mut h = std::collections::hash_map::RandomState::new().build_hasher();
            h.write_u64(k);
            format!("{:016x}", h.finish())
        })
        .collect()
}

fn serve(stream: TcpStream, token: &str, tx: &Sender<Request>, wake: &dyn Fn()) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut line = String::new();
    if BufReader::new((&stream).take(LONGEST_LINE)).read_line(&mut line).is_err() {
        return;
    }
    let answer = match answer(&line, token, tx, wake) {
        Ok(v) => json!({ "ok": true, "status": v }),
        Err(e) => json!({ "ok": false, "error": e }),
    };
    let _ = writeln!(&stream, "{answer}");
}

fn answer(line: &str, token: &str, tx: &Sender<Request>, wake: &dyn Fn()) -> Result<Value, String> {
    let request: Value = serde_json::from_str(line).map_err(|_| "a line of JSON is expected")?;
    if request["token"].as_str() != Some(token) {
        return Err("the token is not this Massif's".into());
    }
    let file = || -> Result<PathBuf, String> {
        let p = PathBuf::from(request["file"].as_str().ok_or("file is needed")?);
        let map = p.extension().is_some_and(|e| e.eq_ignore_ascii_case(massif_format::EXTENSION));
        if p.is_absolute() && map { Ok(p) } else { Err("file: an absolute path to a .massif".into()) }
    };
    let mut to = None;
    let ask = match request["cmd"].as_str().unwrap_or_default() {
        "status" => Ask::Status,
        "map" => {
            let p = file()?;
            if p.exists() {
                return Err(format!("{} exists already", p.display()));
            }
            to = Some(p);
            Ask::Map
        }
        "apply" => {
            let number = |k: &str| request[k].as_u64().ok_or_else(|| format!("{k} is needed"));
            let (generation, revision) = (number("generation")?, number("revision")?);
            let label: String = request["label"].as_str().unwrap_or("Claude").chars().take(80).collect();
            let world = massif_format::load(&file()?).map_err(|e| e.to_string())?;
            Ask::Apply { world: Box::new(world), label, generation, revision }
        }
        "undo" => Ask::Undo,
        "redo" => Ask::Redo,
        "save" => Ask::Save,
        other => return Err(format!("unknown command \"{other}\": status, map, apply, undo, redo or save")),
    };
    let (reply, answered) = sync_channel(1);
    tx.send(Request { ask, reply }).map_err(|_| "Massif is closing")?;
    wake();
    let got = answered.recv_timeout(PATIENCE).map_err(|_| "Massif did not answer: is its window minimised or busy?")??;
    if let (Some(path), Some(mut map)) = (to, got.map) {
        massif_format::save(&map.snapshot(), &path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(got.status)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask(port: u16, line: &str) -> Value {
        let mut s = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
        writeln!(s, "{line}").unwrap();
        let mut back = String::new();
        BufReader::new(s).read_line(&mut back).unwrap();
        serde_json::from_str(&back).unwrap()
    }

    /// Without the token nothing reaches the app; with it, the app answers.
    #[test]
    fn a_request_carries_the_token() {
        let bridge = Bridge::listen(|| {}).unwrap();
        for line in ["nonsense", r#"{"cmd":"status"}"#, r#"{"cmd":"status","token":"x"}"#] {
            assert_eq!(ask(bridge.port, line)["ok"], false, "{line}");
        }
        assert!(bridge.requests.try_recv().is_err(), "nothing came through");
        let (port, token) = (bridge.port, bridge.token.clone());
        assert_eq!(token.len(), 64);
        let client = std::thread::spawn(move || ask(port, &json!({ "cmd": "status", "token": token }).to_string()));
        let request = bridge.requests.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(matches!(request.ask, Ask::Status));
        request.reply.send(Ok(Answer { status: json!({ "name": "Map" }), map: None })).unwrap_or_else(|_| panic!("nobody waits"));
        assert_eq!(client.join().unwrap()["status"]["name"], "Map");
    }

    /// Files are maps named in full; a map is never written over a file.
    #[test]
    fn files_are_maps_named_in_full() {
        let bridge = Bridge::listen(|| {}).unwrap();
        let there = std::env::temp_dir().join(format!("massif-bridge-{}.massif", std::process::id()));
        std::fs::write(&there, b"kept").unwrap();
        for file in ["map.massif".to_owned(), std::env::temp_dir().join("a.txt").display().to_string(), there.display().to_string()] {
            let said = ask(bridge.port, &json!({ "cmd": "map", "token": bridge.token, "file": file }).to_string());
            assert_eq!(said["ok"], false, "{file}: {said}");
        }
        assert_eq!(std::fs::read(&there).unwrap(), b"kept");
        std::fs::remove_file(&there).unwrap();
    }
}
