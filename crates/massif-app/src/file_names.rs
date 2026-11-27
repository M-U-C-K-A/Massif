//! The names exports are offered under (Settings ▸ Export ▸ File names): a
//! template of tokens — `{name}` (the map's), `{kind}` (World, Render,
//! Height map…), `{date}`, `{time}`, `{timestamp}`, `{counter}`, `{size}`,
//! `{version}` — made a name every system takes, and free in the folder
//! the save dialog opens in (`-2`, `-3`… after it otherwise).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// How a date is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum DateFormat {
    /// 2026-11-27.
    #[default]
    Iso,
    /// 27-11-2026.
    European,
    /// 11-27-2026.
    Us,
}

/// How the time is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Clock {
    /// 14.05.
    #[default]
    H24,
    /// 2.05 PM.
    H12,
}

/// The settings of the names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Naming {
    pub template: String,
    pub date: DateFormat,
    pub clock: Clock,
    /// The exports named so far, for `{counter}`.
    pub counter: u32,
    /// The folder the last export went to: the next dialogs open there.
    pub folder: Option<PathBuf>,
}

/// The template offered first: the map's name, then what is exported.
pub const DEFAULT_TEMPLATE: &str = "{name} {kind}";

/// Templates offered ready.
pub const PRESETS: [&str; 5] = ["{name} {kind}", "{name}", "{name} {kind} {date}", "{name} {date} {time}", "{kind} {timestamp}"];

/// The tokens a template can hold.
pub const TOKENS: [&str; 8] = ["{name}", "{kind}", "{date}", "{time}", "{timestamp}", "{counter}", "{size}", "{version}"];

impl Default for Naming {
    fn default() -> Self {
        Self { template: DEFAULT_TEMPLATE.to_owned(), date: DateFormat::Iso, clock: Clock::H24, counter: 0, folder: None }
    }
}

/// A moment, as the user's clock says.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Moment {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: u32,
}

impl Moment {
    pub fn now() -> Self {
        let (year, month, day, hour, minute, second) = crate::report::local_now();
        Self { year, month, day, hour, minute, second }
    }
}

/// What an export is: what fills the tokens.
#[derive(Debug, Clone, Copy)]
pub struct Export<'a> {
    pub name: &'a str,
    pub kind: &'a str,
    /// Its size in pixels or blocks, when it has one.
    pub size: Option<[u32; 2]>,
}

impl Naming {
    /// The name (without its extension) the template gives `export` at
    /// `when`, the `counter`-th export.
    pub fn format(&self, export: &Export, when: Moment, counter: u32) -> String {
        let Moment { year, month, day, hour, minute, second } = when;
        let date = match self.date {
            DateFormat::Iso => format!("{year:04}-{month:02}-{day:02}"),
            DateFormat::European => format!("{day:02}-{month:02}-{year:04}"),
            DateFormat::Us => format!("{month:02}-{day:02}-{year:04}"),
        };
        let time = match self.clock {
            Clock::H24 => format!("{hour:02}.{minute:02}"),
            Clock::H12 => format!("{}.{minute:02} {}", if hour % 12 == 0 { 12 } else { hour % 12 }, if hour < 12 { "AM" } else { "PM" }),
        };
        let mut out = String::new();
        let mut rest = self.template.as_str();
        while let Some(open) = rest.find('{') {
            out.push_str(&rest[..open]);
            let after = &rest[open..];
            let Some(close) = after.find('}') else {
                out.push_str(after);
                rest = "";
                break;
            };
            let value = match &after[..=close] {
                "{name}" => export.name.to_owned(),
                "{kind}" => export.kind.to_owned(),
                "{date}" => date.clone(),
                "{time}" => time.clone(),
                "{timestamp}" => format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}"),
                "{counter}" => format!("{counter:03}"),
                "{size}" => export.size.map_or_else(String::new, |[w, h]| format!("{w}x{h}")),
                "{version}" => env!("CARGO_PKG_VERSION").to_owned(),
                other => other.to_owned(),
            };
            out.push_str(&value);
            rest = &after[close + 1..];
        }
        out.push_str(rest);
        let name = sanitise(&out);
        if name.is_empty() { sanitise(export.kind) } else { name }
    }
}

/// `name` made a file name Windows, macOS and Linux all take: no `< > : "
/// / \ | ? *` nor control characters (each a `-`), single spaces, no dot
/// or space at its ends, not a name Windows keeps for devices.
pub fn sanitise(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for c in name.chars() {
        let c = if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '-' } else { c };
        if c == ' ' && (out.is_empty() || out.ends_with(' ')) {
            continue;
        }
        out.push(c);
    }
    let mut out = out.trim_matches(|c: char| c == ' ' || c == '.').to_owned();
    let stem = out.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL") || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if reserved {
        out.push('_');
    }
    // Some 200 bytes at most, on a character's edge.
    if out.len() > 200 {
        let mut end = 200;
        while !out.is_char_boundary(end) {
            end -= 1;
        }
        out.truncate(end);
    }
    out
}

/// `stem.ext` in `folder`, or `stem-2.ext`, `stem-3.ext`… the first that
/// is not there.
pub fn free_name(folder: Option<&Path>, stem: &str, ext: &str) -> String {
    let name = |k: u32| if k < 2 { format!("{stem}.{ext}") } else { format!("{stem}-{k}.{ext}") };
    let Some(folder) = folder else { return name(1) };
    (1..10_000).map(name).find(|n| !folder.join(n).exists()).unwrap_or_else(|| name(1))
}

/// The settings as they are, for the dialogs all over the app.
static CURRENT: Mutex<Option<Naming>> = Mutex::new(None);

/// Use `naming` from now on (at start and when the settings change).
pub fn set(naming: &Naming) {
    if let Ok(mut c) = CURRENT.lock() {
        *c = Some(naming.clone());
    }
}

/// Take the template and formats of `naming`, keeping the counter and the
/// folder the exports left.
pub fn configure(naming: &Naming) {
    if let Ok(mut c) = CURRENT.lock() {
        let n = c.get_or_insert_with(|| naming.clone());
        n.template.clone_from(&naming.template);
        n.date = naming.date;
        n.clock = naming.clock;
    }
}

/// The settings as the exports left them (their counter and folder), to
/// save.
pub fn current() -> Naming {
    CURRENT.lock().ok().and_then(|c| c.clone()).unwrap_or_default()
}

/// A save dialog for `export` as a `.ext` file, opened in the last export's
/// folder with the name the settings give it there.
pub fn dialog(export: Export, ext: &str) -> rfd::FileDialog {
    let naming = current();
    let stem = naming.format(&export, Moment::now(), naming.counter + 1);
    let folder = naming.folder.filter(|f| f.is_dir());
    let mut d = rfd::FileDialog::new().set_file_name(free_name(folder.as_deref(), &stem, ext));
    if let Some(f) = folder {
        d = d.set_directory(f);
    }
    d
}

/// The name the settings give `export` now, its extension `ext` (for
/// exports that pick no file, as an animation's frames).
pub fn stem(export: Export) -> String {
    let naming = current();
    naming.format(&export, Moment::now(), naming.counter + 1)
}

/// An export was saved at `path`: the counter goes on, and the next
/// dialog opens in its folder. Gives the path back.
pub fn saved(path: PathBuf) -> PathBuf {
    if let Ok(mut c) = CURRENT.lock() {
        let n = c.get_or_insert_with(Naming::default);
        n.counter = n.counter.saturating_add(1);
        n.folder = path.parent().map(Path::to_path_buf);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    const WHEN: Moment = Moment { year: 2026, month: 11, day: 7, hour: 14, minute: 5, second: 9 };

    fn named(template: &str, date: DateFormat, clock: Clock) -> String {
        let n = Naming { template: template.into(), date, clock, ..Naming::default() };
        n.format(&Export { name: "Alps", kind: "Render", size: Some([1920, 1080]) }, WHEN, 7)
    }

    #[test]
    fn every_token_is_filled() {
        let t = |s: &str| named(s, DateFormat::Iso, Clock::H24);
        assert_eq!(t("{name}"), "Alps");
        assert_eq!(t("{kind}"), "Render");
        assert_eq!(t("{date}"), "2026-11-07");
        assert_eq!(t("{time}"), "14.05");
        assert_eq!(t("{timestamp}"), "20261107-140509");
        assert_eq!(t("{counter}"), "007");
        assert_eq!(t("{size}"), "1920x1080");
        assert_eq!(t("{version}"), env!("CARGO_PKG_VERSION"));
        assert_eq!(t(DEFAULT_TEMPLATE), "Alps Render");
        assert_eq!(t("{name}_{kind} #{counter} {unknown}"), "Alps_Render #007 {unknown}");
        assert_eq!(t("{name"), "{name");
        // No size: the token goes, and so does its space.
        let n = Naming { template: "{name} {size} {kind}".into(), ..Naming::default() };
        assert_eq!(n.format(&Export { name: "Alps", kind: "World", size: None }, WHEN, 1), "Alps World");
        // Nothing left: the kind.
        let n = Naming { template: "{size}".into(), ..Naming::default() };
        assert_eq!(n.format(&Export { name: "Alps", kind: "World", size: None }, WHEN, 1), "World");
    }

    #[test]
    fn dates_and_times_in_each_format() {
        assert_eq!(named("{date}", DateFormat::Iso, Clock::H24), "2026-11-07");
        assert_eq!(named("{date}", DateFormat::European, Clock::H24), "07-11-2026");
        assert_eq!(named("{date}", DateFormat::Us, Clock::H24), "11-07-2026");
        assert_eq!(named("{time}", DateFormat::Iso, Clock::H12), "2.05 PM");
        let n = Naming { template: "{time}".into(), clock: Clock::H12, ..Naming::default() };
        let at = |hour| n.format(&Export { name: "", kind: "", size: None }, Moment { hour, ..WHEN }, 1);
        assert_eq!((at(0).as_str(), at(12).as_str(), at(23).as_str()), ("12.05 AM", "12.05 PM", "11.05 PM"));
    }

    #[test]
    fn names_are_made_safe() {
        assert_eq!(sanitise("a<b>c:d\"e/f\\g|h?i*j"), "a-b-c-d-e-f-g-h-i-j");
        assert_eq!(sanitise("  two   spaces . "), "two spaces");
        assert_eq!(sanitise("tab\there"), "tab-here");
        assert_eq!(sanitise("..hidden."), "hidden");
        assert_eq!(sanitise("CON"), "CON_");
        assert_eq!(sanitise("com1.tar"), "com1.tar_");
        assert_eq!(sanitise("Console"), "Console");
        assert_eq!(sanitise("Île d'été"), "Île d'été");
        assert!(sanitise(&"é".repeat(300)).len() <= 200);
        let n = Naming { template: "{name}: {kind}".into(), ..Naming::default() };
        assert_eq!(n.format(&Export { name: "A/B", kind: "Render", size: None }, WHEN, 1), "A-B- Render");
    }

    #[test]
    fn a_name_taken_gets_a_number() {
        let dir = std::env::temp_dir().join(format!("massif-file-names-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(free_name(Some(&dir), "Alps", "png"), "Alps.png");
        std::fs::write(dir.join("Alps.png"), b"").unwrap();
        assert_eq!(free_name(Some(&dir), "Alps", "png"), "Alps-2.png");
        std::fs::write(dir.join("Alps-2.png"), b"").unwrap();
        assert_eq!(free_name(Some(&dir), "Alps", "png"), "Alps-3.png");
        assert_eq!(free_name(None, "Alps", "png"), "Alps.png");
        let _ = std::fs::remove_dir_all(dir);
    }
}
