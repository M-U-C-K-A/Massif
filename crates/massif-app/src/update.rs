//! New versions: once a day at most, off the main thread, Massif asks
//! GitHub which release of Massif is the latest, and says so when it is
//! newer than the one running. It sends nothing about the user (the request
//! carries Massif's version as its user agent, and no more), stays silent
//! when offline or refused, and never downloads or installs anything: the
//! release page opens in the browser.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The repository the releases are published from.
pub const REPOSITORY: &str = "M-U-C-K-A/Massif";
/// GitHub's answer for the latest release (neither a draft nor a
/// pre-release).
const LATEST: &str = "https://api.github.com/repos/M-U-C-K-A/Massif/releases/latest";
/// How long an answer is kept before GitHub is asked again.
pub const EVERY: Duration = Duration::from_secs(24 * 60 * 60);
/// The version running.
pub const RUNNING: &str = env!("CARGO_PKG_VERSION");

/// A version, `1.2.3` (a `v` before it and a `-beta` after it allowed: a
/// pre-release comes before its release).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    pub numbers: [u64; 3],
    /// No suffix (`true`) sorts after any pre-release of the same numbers.
    released: bool,
}

impl Version {
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        let text = text.strip_prefix(['v', 'V']).unwrap_or(text);
        let (core, pre) = match text.find(['-', '+']) {
            Some(k) => (&text[..k], text[k..].starts_with('-')),
            None => (text, false),
        };
        let mut numbers = [0u64; 3];
        let mut parts = core.split('.');
        for (k, n) in numbers.iter_mut().enumerate() {
            match parts.next() {
                Some(p) => *n = p.parse().ok()?,
                // "1.2" is 1.2.0; "1" alone is not a version.
                None if k == 2 => {}
                None => return None,
            }
        }
        if parts.next().is_some() {
            return None;
        }
        Some(Self { numbers, released: !pre })
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let [a, b, c] = self.numbers;
        write!(f, "{a}.{b}.{c}")
    }
}

/// A release, as GitHub describes it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    pub name: Option<String>,
    /// Its notes, in Markdown.
    #[serde(default)]
    pub body: Option<String>,
    /// Its page on GitHub.
    pub html_url: String,
}

impl Release {
    pub fn version(&self) -> Option<Version> {
        Version::parse(&self.tag_name)
    }

    /// Whether it is newer than `running`.
    pub fn is_newer_than(&self, running: &str) -> bool {
        match (self.version(), Version::parse(running)) {
            (Some(new), Some(old)) => new > old,
            _ => false,
        }
    }

    /// The page to open: GitHub's page of this release, and nothing else
    /// whatever the answer said.
    pub fn page(&self) -> String {
        let prefix = format!("https://github.com/{REPOSITORY}/releases/");
        if self.html_url.starts_with(&prefix) { self.html_url.clone() } else { format!("{prefix}latest") }
    }

    /// What is new since `running`: the notes' sections `## New in X.Y.Z`
    /// of the versions after it, newest first as the notes have them; the
    /// whole notes when they are not in sections.
    pub fn notes_since(&self, running: &str) -> String {
        let body = self.body.as_deref().unwrap_or("").replace("\r\n", "\n");
        let old = Version::parse(running);
        let mut out = String::new();
        let (mut sections, mut keep) = (false, false);
        for line in body.lines() {
            if let Some(title) = line.strip_prefix("## ") {
                sections = true;
                let version = title.split_whitespace().find_map(Version::parse);
                keep = match (version, &old) {
                    (Some(v), Some(old)) => v > *old,
                    _ => false,
                };
            }
            if keep {
                out.push_str(line);
                out.push('\n');
            }
        }
        if sections && !out.trim().is_empty() { out } else { body }
    }
}

/// The last answer, kept in `update.json` next to `startup.log`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Cache {
    /// When GitHub was last asked and answered, in seconds since 1970.
    pub checked: u64,
    pub release: Release,
}

pub fn default_cache() -> Option<PathBuf> {
    crate::startup::log_dir().map(|d| d.join("update.json"))
}

pub fn now_secs() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Cache {
    pub fn read(path: &Path) -> Option<Self> {
        serde_json::from_slice(&std::fs::read(path).ok()?).ok()
    }

    fn write(&self, path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = path.with_extension("json.tmp");
        if let Ok(bytes) = serde_json::to_vec_pretty(self)
            && std::fs::write(&tmp, bytes).is_ok()
        {
            let _ = std::fs::rename(&tmp, path);
        }
    }

    /// Whether it is recent enough at `now` not to ask again (a clock set
    /// back makes it old).
    pub fn fresh(&self, now: u64) -> bool {
        now >= self.checked && now - self.checked < EVERY.as_secs()
    }
}

/// How GitHub is reached: the URL asked, the body answered, or why not.
pub type Fetch = fn(&str) -> Result<String, String>;

/// Ask GitHub, over HTTPS, with Massif's version as the user agent and
/// nothing else about who asks.
pub fn fetch_github(url: &str) -> Result<String, String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(15)))
        .user_agent(format!("Massif/{RUNNING}"))
        .build()
        .into();
    let mut answer = agent.get(url).header("Accept", "application/vnd.github+json").call().map_err(|e| e.to_string())?;
    answer.body_mut().read_to_string().map_err(|e| e.to_string())
}

/// Ask for the latest release, and keep the answer in `cache`. `None`
/// when offline, refused, or the answer is not one.
pub fn check(fetch: Fetch, cache: &Path, now: u64) -> Option<Release> {
    let body = match fetch(LATEST) {
        Ok(body) => body,
        Err(e) => {
            log::info!("no answer about new versions: {e}");
            return None;
        }
    };
    let release: Release = match serde_json::from_str(&body) {
        Ok(r) => r,
        Err(e) => {
            log::info!("an answer about new versions not understood: {e}");
            return None;
        }
    };
    Cache { checked: now, release: release.clone() }.write(cache);
    Some(release)
}

/// The latest release known, and the check under way.
#[derive(Default)]
pub struct Updates {
    /// The latest release, as last heard (from the cache or GitHub).
    pub latest: Option<Release>,
    pending: Option<Receiver<Option<Release>>>,
    /// Whether a check was started (or the cache answered) in this run.
    started: bool,
}

impl Updates {
    /// Read the cache; if it is older than a day, ask GitHub again on a
    /// thread of its own, and wake the window when it answers. What the
    /// cache held stands meanwhile, and if no answer comes.
    pub fn start(&mut self, ctx: &egui::Context, cache: PathBuf, fetch: Fetch) {
        if self.started {
            return;
        }
        self.started = true;
        let now = now_secs();
        let cached = Cache::read(&cache);
        let fresh = cached.as_ref().is_some_and(|c| c.fresh(now));
        self.latest = cached.map(|c| c.release);
        if fresh {
            return;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new().name("massif new versions".into()).spawn(move || {
            let _ = tx.send(check(fetch, &cache, now));
            ctx.request_repaint();
        });
        if spawned.is_ok() {
            self.pending = Some(rx);
        }
    }

    /// Take the answer, once it came.
    pub fn poll(&mut self) {
        let Some(rx) = &self.pending else { return };
        match rx.try_recv() {
            Ok(answer) => {
                if answer.is_some() {
                    self.latest = answer;
                }
                self.pending = None;
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => self.pending = None,
        }
    }

    /// The release to tell of: newer than `running`, and not the one
    /// skipped.
    pub fn offer(&self, running: &str, skipped: Option<&str>) -> Option<&Release> {
        self.latest.as_ref().filter(|r| r.is_newer_than(running) && skipped.is_none_or(|s| Version::parse(s) != r.version()))
    }

    /// Whether the answer is still awaited.
    pub fn waiting(&self) -> bool {
        self.pending.is_some()
    }

    #[cfg(test)]
    pub fn known(release: Release) -> Self {
        Self { latest: Some(release), pending: None, started: true }
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// A release as GitHub answers it, cut down.
    pub const FIXTURE: &str = r#"{
        "url": "https://api.github.com/repos/M-U-C-K-A/Massif/releases/1",
        "html_url": "https://github.com/M-U-C-K-A/Massif/releases/tag/v9.1.0",
        "tag_name": "v9.1.0",
        "name": "Massif 9.1.0",
        "draft": false,
        "prerelease": false,
        "body": "Massif is a map editor for Minecraft Java worlds.\r\n\r\n## New in 9.1.0\r\n\r\n**Rivers that find their way.** Rivers now run downhill on their own, around hills, into lakes and out to the sea.\r\n\r\n**Faster export.** A map of 2,116 tiles is written in **two** seconds.\r\n\r\n## New in 9.0.0\r\n\r\n**Caves under the map.** The `Caves` layer digs tunnels and caverns.\r\n\r\n## New in 0.5.2\r\n\r\n**Old news.** Already known.\r\n",
        "assets": []
    }"#;

    pub fn fixture() -> Release {
        serde_json::from_str(FIXTURE).unwrap()
    }

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("massif-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn versions_compare() {
        let v = |s| Version::parse(s).unwrap_or_else(|| panic!("{s}"));
        assert_eq!(v("v0.6.0").numbers, [0, 6, 0]);
        assert_eq!(v("0.6").numbers, [0, 6, 0]);
        assert!(v("v0.10.0") > v("0.9.9"), "numbers, not text");
        assert!(v("1.0.0") > v("0.99.99"));
        assert!(v("0.6.1") > v("0.6.0"));
        assert!(v("0.6.0") > v("0.6.0-beta.2"), "a pre-release comes before its release");
        assert!(v("0.6.0-beta") > v("0.5.9"));
        assert_eq!(v("V0.6.0"), v("0.6.0"));
        assert_eq!(v("0.6.0+build"), v("0.6.0"));
        for bad in ["", "v", "1", "a.b.c", "1.2.3.4", "1..2", "latest"] {
            assert!(Version::parse(bad).is_none(), "{bad:?}");
        }
        let r = fixture();
        assert!(r.is_newer_than("0.6.0") && r.is_newer_than("9.0.9"));
        assert!(!r.is_newer_than("9.1.0") && !r.is_newer_than("10.0.0"), "the same or older is not news");
        assert!(!r.is_newer_than("not a version"));
    }

    #[test]
    fn notes_keep_what_is_new() {
        let r = fixture();
        let since = r.notes_since("0.6.0");
        assert!(since.starts_with("## New in 9.1.0") && since.contains("## New in 9.0.0"), "{since}");
        assert!(!since.contains("Old news") && !since.contains("map editor for"), "{since}");
        assert!(!since.contains('\r'));
        let since = r.notes_since("9.0.0");
        assert!(since.contains("Rivers") && !since.contains("Caves"), "{since}");
        // Notes not in sections are shown whole.
        let plain = Release { body: Some("Fixes a crash.".into()), ..r.clone() };
        assert_eq!(plain.notes_since("0.6.0"), "Fixes a crash.");
        assert_eq!(Release { body: None, ..r.clone() }.notes_since("0.6.0"), "");
        // Only GitHub's page of the release opens.
        assert_eq!(r.page(), "https://github.com/M-U-C-K-A/Massif/releases/tag/v9.1.0");
        let odd = Release { html_url: "https://example.com/x".into(), ..r };
        assert_eq!(odd.page(), "https://github.com/M-U-C-K-A/Massif/releases/latest");
    }

    #[test]
    fn the_answer_is_cached_a_day() {
        let d = dir("cache");
        let cache = d.join("update.json");
        let now = 1_800_000_000;
        let got = check(|url| if url == LATEST { Ok(FIXTURE.to_owned()) } else { Err(url.to_owned()) }, &cache, now);
        assert_eq!(got, Some(fixture()));
        let kept = Cache::read(&cache).expect("kept");
        assert_eq!(kept, Cache { checked: now, release: fixture() });
        assert!(kept.fresh(now) && kept.fresh(now + EVERY.as_secs() - 1));
        assert!(!kept.fresh(now + EVERY.as_secs()), "asked again after a day");
        assert!(!kept.fresh(now - 10), "a clock set back asks again");
        // A fresh cache answers alone: GitHub is not asked.
        let ctx = egui::Context::default();
        let mut u = Updates::default();
        let mut cache_now = kept.clone();
        cache_now.checked = now_secs();
        cache_now.write(&cache);
        u.start(&ctx, cache.clone(), |_| panic!("GitHub asked though the cache is fresh"));
        assert!(!u.waiting());
        assert_eq!(u.offer("0.6.0", None), Some(&fixture()));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn silent_when_offline_or_refused() {
        let d = dir("offline");
        let cache = d.join("update.json");
        for fetch in [
            (|_: &str| Err("dns error: failed to lookup address".to_owned())) as Fetch,
            |_| Err("http status: 403".to_owned()),
            |_| Ok("<html>rate limited</html>".to_owned()),
            |_| Ok(r#"{"message": "Not Found"}"#.to_owned()),
        ] {
            assert_eq!(check(fetch, &cache, 1), None);
            assert!(!cache.exists(), "nothing kept from no answer");
            let ctx = egui::Context::default();
            let mut u = Updates::default();
            u.start(&ctx, cache.clone(), fetch);
            for _ in 0..500 {
                u.poll();
                if !u.waiting() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
            assert!(!u.waiting() && u.latest.is_none() && u.offer("0.6.0", None).is_none());
        }
        // Offline with an old answer kept: the old answer stands.
        Cache { checked: 1, release: fixture() }.write(&cache);
        let ctx = egui::Context::default();
        let mut u = Updates::default();
        u.start(&ctx, cache.clone(), |_| Err("offline".to_owned()));
        while u.waiting() {
            u.poll();
            std::thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(u.offer("0.6.0", None).map(|r| r.tag_name.as_str()), Some("v9.1.0"));
        assert_eq!(Cache::read(&cache).unwrap().checked, 1, "kept as it was");
        let _ = std::fs::remove_dir_all(&d);
    }

    /// GitHub itself, over the network: `cargo test -p massif-app live_github -- --ignored`.
    #[test]
    #[ignore]
    fn live_github() {
        let d = dir("live");
        let r = check(fetch_github, &d.join("update.json"), now_secs()).expect("GitHub answers");
        println!("latest: {} {}", r.tag_name, r.page());
        assert!(r.version().is_some());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn a_skipped_version_is_not_offered() {
        let u = Updates::known(fixture());
        assert!(u.offer("0.6.0", None).is_some());
        assert!(u.offer("0.6.0", Some("9.1.0")).is_none());
        assert!(u.offer("0.6.0", Some("v9.1.0")).is_none());
        assert!(u.offer("0.6.0", Some("9.0.0")).is_some(), "a newer one than the one skipped is offered");
        assert!(u.offer("9.1.0", None).is_none());
    }
}
