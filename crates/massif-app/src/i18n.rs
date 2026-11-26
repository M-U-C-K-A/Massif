//! The interface in the user's language. Every text the interface shows is
//! written in English in the code, through `tr!` (or `trn!` when it counts
//! something), and that English text is the key: a language is a file of
//! `"English": "translation"` pairs, in `lang/` (built into the app).
//!
//! Nothing here may break the interface. A text the language does not have,
//! a language file that does not read, or a translation that loses or adds a
//! placeholder all show the English text instead; in debug builds each
//! missing text is logged once.
//!
//! A placeholder is a name in braces, `{n} blocks`, filled with the value of
//! that name; `{size:.1}` keeps the number's precision. A text that counts
//! has two English forms, `trn!("{n} block", "{n} blocks", n = count)`, and
//! its translation is either one text or `{"one": …, "other": …}`, chosen by
//! the language's own rule (French counts 0 as one). Russian adds `"few"`
//! (2–4, 22–24…) and `"many"` (5–20, 25…), `"other"` being for fractions.

/// A text of the interface in the language shown. `tr!("Save")` is a
/// `&'static str`; with placeholders, `tr!("Undo {what}", what)` or
/// `tr!("{size:.1} MB", size = mb)`, a `String`.
macro_rules! tr {
    ($s:literal) => {
        $crate::i18n::tr($s)
    };
    ($s:literal, $($name:ident $(= $val:expr)?),+ $(,)?) => {
        $crate::i18n::fill($crate::i18n::tr($s), &[$((stringify!($name), &tr!(@value $name $(= $val)?) as &dyn ::std::fmt::Display)),+])
    };
    (@value $name:ident) => {
        $name
    };
    (@value $name:ident = $val:expr) => {
        $val
    };
}

/// A name kept in English where it is written (a constant, a name saved in
/// the map or the settings) and translated where it is shown, by `tr_name`:
/// `tn!("Painting")` is the literal itself, marked for the language files.
macro_rules! tn {
    ($s:literal) => {
        $s
    };
}

/// A text that counts, in the language shown: `trn!("{n} block", "{n}
/// blocks", n = count)`, more placeholders after the count.
macro_rules! trn {
    ($one:literal, $other:literal, n = $n:expr $(, $name:ident $(= $val:expr)?)* $(,)?) => {{
        let n = $n;
        $crate::i18n::fill($crate::i18n::tr_count($one, $other, n as f64), &[(stringify!(n), &n as &dyn ::std::fmt::Display) $(, (stringify!($name), &tr!(@value $name $(= $val)?) as &dyn ::std::fmt::Display))*])
    }};
}

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fmt::Display;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU8, Ordering};

/// A language the interface can be in, or the system's.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Language {
    /// The system's own language, when Massif has it; English otherwise.
    #[default]
    System,
    English,
    French,
    German,
    Spanish,
    Portuguese,
    Russian,
}

impl Language {
    /// The languages offered, each by its own name, after System.
    pub const ALL: [Language; 6] = [Language::English, Language::French, Language::German, Language::Spanish, Language::Portuguese, Language::Russian];

    /// The language's name in that language: how someone looks for theirs.
    pub fn native_name(self) -> &'static str {
        match self {
            Language::System => tr!("System"),
            Language::English => "English",
            Language::French => "Français",
            Language::German => "Deutsch",
            Language::Spanish => "Español",
            Language::Portuguese => "Português",
            Language::Russian => "Русский",
        }
    }

    /// Its ISO 639-1 code, as language files and systems name it.
    pub fn code(self) -> &'static str {
        match self {
            Language::System | Language::English => "en",
            Language::French => "fr",
            Language::German => "de",
            Language::Spanish => "es",
            Language::Portuguese => "pt",
            Language::Russian => "ru",
        }
    }

    /// The language a code such as `de-AT`, `fr_FR.UTF-8` or `pt` names.
    pub fn from_code(code: &str) -> Option<Language> {
        let lang = code.trim().split(['-', '_', '.', '@']).next()?.to_ascii_lowercase();
        Language::ALL.into_iter().find(|l| l.code() == lang)
    }

    /// Whether Massif offers it: English, and each language with texts.
    pub fn offered(self) -> bool {
        matches!(self, Language::System | Language::English) || catalog(self).is_some_and(|c| c.len() > 0)
    }

    /// The language actually shown: System made one of the others.
    pub fn resolve(self) -> Language {
        match self {
            Language::System => system_language(),
            l => l,
        }
    }

    fn index(self) -> u8 {
        self as u8
    }

    fn from_index(i: u8) -> Language {
        [Language::System, Language::English, Language::French, Language::German, Language::Spanish, Language::Portuguese, Language::Russian].get(i as usize).copied().unwrap_or(Language::English)
    }

    /// The file of the language, built into the app; English has none.
    fn source(self) -> Option<&'static str> {
        match self {
            Language::System | Language::English => None,
            Language::French => Some(include_str!("../lang/fr.json")),
            Language::German => Some(include_str!("../lang/de.json")),
            Language::Spanish => Some(include_str!("../lang/es.json")),
            Language::Portuguese => Some(include_str!("../lang/pt.json")),
            Language::Russian => Some(include_str!("../lang/ru.json")),
        }
    }
}

/// The language the system's interface is in, among those Massif has.
pub fn system_language() -> Language {
    static SYSTEM: OnceLock<Language> = OnceLock::new();
    // The tests find widgets by their English names, whatever the machine.
    if cfg!(test) {
        return Language::English;
    }
    *SYSTEM.get_or_init(|| os_languages().iter().find_map(|c| Language::from_code(c)).filter(|l| l.offered()).unwrap_or(Language::English))
}

/// The system's preferred languages, first first.
#[cfg(target_os = "macos")]
fn os_languages() -> Vec<String> {
    objc2_foundation::NSLocale::preferredLanguages().iter().map(|s| s.to_string()).collect()
}

#[cfg(windows)]
fn os_languages() -> Vec<String> {
    // SAFETY: takes nothing and returns the user's interface language id.
    let id = unsafe { windows_sys::Win32::Globalization::GetUserDefaultUILanguage() };
    // The primary language is the low 10 bits of the id.
    let code = match id & 0x3ff {
        0x07 => "de",
        0x0c => "fr",
        0x0a => "es",
        0x16 => "pt",
        0x19 => "ru",
        _ => "en",
    };
    vec![code.to_owned()]
}

#[cfg(not(any(target_os = "macos", windows)))]
fn os_languages() -> Vec<String> {
    let mut codes = Vec::new();
    if let Ok(list) = std::env::var("LANGUAGE") {
        codes.extend(list.split(':').filter(|s| !s.is_empty()).map(str::to_owned));
    }
    for var in ["LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Ok(v) = std::env::var(var)
            && !v.is_empty()
        {
            codes.push(v);
        }
    }
    codes
}

// ------------------------------------------------------------ the choice --

/// The language chosen in the settings (System until then).
static CHOSEN: AtomicU8 = AtomicU8::new(0);

#[cfg(test)]
thread_local! {
    /// A test's own language, so that tests running side by side in other
    /// languages leave each other alone.
    static TEST_CHOSEN: std::cell::Cell<Option<Language>> = const { std::cell::Cell::new(None) };
}

/// Show the interface in this language from now on (the next frame).
pub fn set_language(language: Language) {
    #[cfg(test)]
    TEST_CHOSEN.with(|c| c.set(Some(language)));
    #[cfg(not(test))]
    CHOSEN.store(language.index(), Ordering::Relaxed);
}

/// The language chosen, maybe System.
#[allow(dead_code)]
pub fn chosen() -> Language {
    #[cfg(test)]
    if let Some(l) = TEST_CHOSEN.with(|c| c.get()) {
        return l;
    }
    // MASSIF_SNAPSHOT_LANG=de: the renders of the tests in that language
    // (the tests that find widgets by their English names then fail).
    #[cfg(test)]
    if let Some(l) = std::env::var("MASSIF_SNAPSHOT_LANG").ok().and_then(|c| Language::from_code(&c)) {
        return l;
    }
    Language::from_index(CHOSEN.load(Ordering::Relaxed))
}

/// The language shown.
pub fn current() -> Language {
    chosen().resolve()
}

// ------------------------------------------------------------ catalogs --

/// A translation: one text, or one by count.
#[derive(Debug, Clone, Copy)]
enum Entry {
    One(&'static str),
    /// `few` and `many` are Russian's: the other languages leave them out,
    /// and `other` stands in where a form is missing.
    Plural { one: &'static str, few: Option<&'static str>, many: Option<&'static str>, other: &'static str },
}

/// A language's texts, by their English text.
#[derive(Debug, Default)]
pub struct Catalog {
    entries: HashMap<&'static str, Entry>,
}

impl Catalog {
    /// Reads a language file. Whatever does not read is left out (the whole
    /// file when it is not a JSON object), with a warning, never a panic: the
    /// English text shows instead.
    pub fn parse(name: &str, source: &str) -> Catalog {
        let value: serde_json::Value = match serde_json::from_str(source) {
            Ok(v) => v,
            Err(e) => {
                log::warn!("language file {name} does not read ({e}); English instead");
                return Catalog::default();
            }
        };
        let Some(object) = value.as_object() else {
            log::warn!("language file {name} is not an object of texts; English instead");
            return Catalog::default();
        };
        let leak = |s: &str| -> &'static str { Box::leak(s.to_owned().into_boxed_str()) };
        let mut entries = HashMap::with_capacity(object.len());
        for (key, value) in object {
            // Keys starting with "//" are notes to translators.
            if key.starts_with("//") {
                continue;
            }
            let entry = match value {
                serde_json::Value::String(s) if !s.is_empty() => Entry::One(leak(s)),
                serde_json::Value::Object(forms) => match (forms.get("one").and_then(|v| v.as_str()), forms.get("other").and_then(|v| v.as_str())) {
                    (Some(one), Some(other)) => {
                        let form = |name: &str| forms.get(name).and_then(|v| v.as_str()).filter(|s| !s.is_empty()).map(leak);
                        Entry::Plural { one: leak(one), few: form("few"), many: form("many"), other: leak(other) }
                    }
                    (None, Some(other)) => Entry::One(leak(other)),
                    _ => continue,
                },
                _ => continue,
            };
            let wanted = placeholders(key);
            let keeps = match entry {
                Entry::One(s) => placeholders(s) == wanted,
                // The singular may leave the count out ("un bloc").
                Entry::Plural { one, few, many, other } => placeholders(other) == wanted && [Some(one), few, many].into_iter().flatten().all(|f| placeholders(f).iter().all(|p| wanted.contains(p))),
            };
            if !keeps {
                log::warn!("language file {name}: the translation of {key:?} changes its placeholders; English instead");
                continue;
            }
            entries.insert(leak(key), entry);
        }
        Catalog { entries }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[allow(dead_code)]
    pub fn contains(&self, key: &str) -> bool {
        self.entries.contains_key(key)
    }

    #[allow(dead_code)]
    pub fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.keys().copied()
    }

    fn get(&self, key: &str) -> Option<Entry> {
        self.entries.get(key).copied()
    }
}

/// The catalog of a language, read the first time it is asked for.
fn catalog(language: Language) -> Option<&'static Catalog> {
    static CATALOGS: [OnceLock<Catalog>; 7] = [const { OnceLock::new() }; 7];
    let source = language.source()?;
    Some(CATALOGS[language.index() as usize].get_or_init(|| Catalog::parse(language.code(), source)))
}

/// The catalog of a language, for the tests and the coverage report.
#[allow(dead_code)]
pub fn catalog_of(language: Language) -> Option<&'static Catalog> {
    catalog(language)
}

#[cfg(debug_assertions)]
fn missing(language: Language, key: &str) {
    use std::collections::HashSet;
    use std::sync::Mutex;
    static SEEN: Mutex<Option<HashSet<(u8, String)>>> = Mutex::new(None);
    let Ok(mut seen) = SEEN.lock() else { return };
    if seen.get_or_insert_with(HashSet::new).insert((language.index(), key.to_owned())) {
        log::debug!("no {} text for {key:?}; English instead", language.code());
    }
}

#[cfg(not(debug_assertions))]
fn missing(_: Language, _: &str) {}

fn lookup(key: &str) -> Option<Entry> {
    let language = current();
    let catalog = catalog(language)?;
    let entry = catalog.get(key);
    if entry.is_none() {
        missing(language, key);
    }
    entry
}

/// The text in the language shown: its translation, or the English text.
pub fn tr(english: &'static str) -> &'static str {
    match lookup(english) {
        Some(Entry::One(s)) | Some(Entry::Plural { other: s, .. }) => s,
        None => english,
    }
}

/// A name that comes from data (a terrain, a layer, a biome): its
/// translation when the language has one, the name itself otherwise.
pub fn tr_name(name: &str) -> &str {
    if current() == Language::English {
        return name;
    }
    match catalog(current()).and_then(|c| c.get(name)) {
        Some(Entry::One(s)) | Some(Entry::Plural { other: s, .. }) => s,
        None => name,
    }
}

/// The form a count takes in a language, as CLDR names them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Form {
    One,
    Few,
    Many,
    Other,
}

/// The form a count takes in the language shown.
fn form_of(language: Language, n: f64) -> Form {
    match language {
        // French: 0 and 1 (and 1.5) are singular.
        Language::French if n.abs() < 2.0 => Form::One,
        // Russian: 1, 21, 31… one; 2–4, 22–24… few; 0, 5–20, 25… many;
        // fractions other.
        Language::Russian if n.fract() == 0.0 => {
            let i = n.abs() as u64;
            match (i % 10, i % 100) {
                (1, m) if m != 11 => Form::One,
                (2..=4, m) if !(12..=14).contains(&m) => Form::Few,
                _ => Form::Many,
            }
        }
        Language::Russian => Form::Other,
        _ if n == 1.0 => Form::One,
        _ => Form::Other,
    }
}

/// Whether a count takes the singular in the language shown.
#[cfg(test)]
fn is_one(language: Language, n: f64) -> bool {
    form_of(language, n) == Form::One
}

/// The text of a count in the language shown.
pub fn tr_count(one: &'static str, other: &'static str, n: f64) -> &'static str {
    let language = current();
    match lookup(other) {
        Some(Entry::Plural { one, few, many, other }) => match form_of(language, n) {
            Form::One => one,
            Form::Few => few.unwrap_or(other),
            Form::Many => many.unwrap_or(other),
            Form::Other => other,
        },
        Some(Entry::One(s)) => s,
        None => {
            if n == 1.0 {
                one
            } else {
                other
            }
        }
    }
}

/// The names of a text's placeholders, sorted, without their formats.
pub fn placeholders(text: &str) -> Vec<&str> {
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let inside = &after[..close];
        let name = inside.split(':').next().unwrap_or("");
        if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
            names.push(name);
        }
        rest = &after[close + 1..];
    }
    names.sort_unstable();
    names.dedup();
    names
}

/// A text with its placeholders filled. A placeholder with no value stays
/// as it is; `{x:.N}` gives a number N decimals.
pub fn fill(template: &str, args: &[(&str, &dyn Display)]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let inside = &after[..close];
        let (name, spec) = inside.split_once(':').unwrap_or((inside, ""));
        match args.iter().find(|(n, _)| *n == name) {
            Some((_, value)) => {
                let precision = spec.strip_prefix('.').and_then(|p| p.parse::<usize>().ok());
                let _ = match precision {
                    Some(p) => write!(out, "{value:.p$}"),
                    None => write!(out, "{value}"),
                };
            }
            None => {
                out.push('{');
                out.push_str(inside);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Names that come from data rather than from `tr!` (terrains, layers,
    /// biomes, nodes), translated where shown through `tr_name`: listed so the
    /// language files may have them.
    const NAMES: &[&str] = &["Wood", "Stone", "Iron", "Diamonds", "Nether", "Ground"];

    /// Every English text of the interface, read from the code: the
    /// literals of `tr!` and `trn!` (the plural form).
    fn english_keys() -> std::collections::BTreeSet<String> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut keys = std::collections::BTreeSet::new();
        // The sources and those of the modules in folders of their own.
        let mut files = Vec::new();
        let mut dirs = vec![dir];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    files.push(path);
                }
            }
        }
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap();
            // This file's own texts are examples, but for one.
            let source = if path.ends_with("i18n.rs") { "tr!(\"System\")".to_owned() } else { source };
            keys.extend(literals_of(&source));
        }
        keys.extend(NAMES.iter().map(|s| s.to_string()));
        keys.extend(core_names());
        keys
    }

    /// The names and descriptions massif-core gives its kinds (nodes,
    /// operations, styles, minerals…), shown through `tr_name`: the literals
    /// of its `label`, `about`, `description`, `use_for`, `hint` and
    /// `describe` functions.
    fn core_names() -> Vec<String> {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).into_iter().flatten().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        walk(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../massif-core/src"), &mut files);
        let mut names = Vec::new();
        for path in files {
            let source = std::fs::read_to_string(&path).unwrap_or_default();
            let source = source.split("#[cfg(test)]\nmod tests").next().unwrap_or("");
            for marker in ["fn label(", "fn name(self)", "fn about(", "fn description(", "fn use_for(", "fn hint(", "fn describe("] {
                let mut rest = source;
                while let Some(at) = rest.find(marker) {
                    let body = &rest[at..];
                    // The function's body: up to the first line closing at
                    // its indent or less.
                    let end = body.find("\n    }\n").or_else(|| body.find("\n}\n")).unwrap_or(body.len());
                    let mut text = &body[..end];
                    while let Some(q) = text.find('"') {
                        match string_literal(&text[q..]) {
                            Some((lit, after)) => {
                                // A format string makes a text no lookup finds whole.
                                if lit.chars().any(|c| c.is_alphabetic()) && !lit.contains("minecraft:") && !lit.contains('{') {
                                    names.push(lit);
                                }
                                text = after;
                            }
                            None => text = &text[q + 1..],
                        }
                    }
                    rest = &rest[at + marker.len()..];
                }
            }
        }
        names
    }

    /// The keys of `tr!("…"` and of `trn!("…", "…"` (its second literal).
    fn literals_of(source: &str) -> Vec<String> {
        let mut keys = Vec::new();
        for (marker, nth) in [("tr!(\"", 0), ("tn!(\"", 0), ("trn!(\"", 1)] {
            let mut rest = source;
            while let Some(at) = rest.find(marker) {
                // `include_str!("…"` is not `tr!("…"`.
                if rest[..at].ends_with(|c: char| c.is_alphanumeric() || c == '_') {
                    rest = &rest[at + marker.len()..];
                    continue;
                }
                let mut text = &rest[at + marker.len() - 1..];
                let mut found = None;
                for _ in 0..=nth {
                    let Some((lit, after)) = string_literal(text) else { break };
                    found = Some(lit);
                    text = after.trim_start().trim_start_matches(',').trim_start();
                }
                if let Some(lit) = found {
                    keys.push(lit);
                }
                rest = &rest[at + marker.len()..];
            }
        }
        keys
    }

    /// The value of the Rust string literal `text` starts with, and what
    /// follows it.
    fn string_literal(text: &str) -> Option<(String, &str)> {
        let body = text.strip_prefix('"')?;
        let mut out = String::new();
        let mut chars = body.char_indices();
        while let Some((i, c)) = chars.next() {
            match c {
                '"' => return Some((out, &body[i + 1..])),
                '\\' => match chars.next()?.1 {
                    'n' => out.push('\n'),
                    't' => out.push('\t'),
                    '\\' => out.push('\\'),
                    '"' => out.push('"'),
                    '\'' => out.push('\''),
                    '\n' => {
                        // A continued line: the next line's indent goes.
                        while chars.clone().next().is_some_and(|(_, c)| c.is_whitespace()) {
                            chars.next();
                        }
                    }
                    'u' => {
                        let rest = &body[i + 2..];
                        let end = rest.find('}')?;
                        out.push(char::from_u32(u32::from_str_radix(&rest[1..end], 16).ok()?)?);
                        for _ in 0..=end {
                            chars.next();
                        }
                    }
                    _ => return None,
                },
                c => out.push(c),
            }
        }
        None
    }

    #[test]
    fn every_translated_text_is_an_english_text_of_the_app() {
        let english = english_keys();
        assert!(english.len() > 500, "the texts of the app are read ({})", english.len());
        for language in Language::ALL.into_iter().skip(1) {
            let catalog = catalog_of(language).unwrap();
            let stale: Vec<_> = catalog.keys().filter(|k| !english.contains(*k)).collect();
            assert!(stale.is_empty(), "{} has texts the app does not have: {stale:#?}", language.code());
        }
    }

    #[test]
    fn every_language_file_reads_whole_and_keeps_the_placeholders() {
        for language in Language::ALL.into_iter().skip(1) {
            let source = language.source().unwrap();
            let raw: serde_json::Map<String, serde_json::Value> = serde_json::from_str(source).unwrap_or_else(|e| panic!("{}.json reads: {e}", language.code()));
            let read = Catalog::parse(language.code(), source);
            let texts = raw.keys().filter(|k| !k.starts_with("//")).count();
            // A text the parse left out lost a placeholder or is empty.
            let dropped: Vec<_> = raw.keys().filter(|k| !k.starts_with("//") && !read.contains(k)).collect();
            assert!(dropped.is_empty(), "{}.json: texts left out (placeholders changed or empty): {dropped:#?}", language.code());
            assert_eq!(read.len(), texts);
        }
    }

    #[test]
    fn share_translated_per_language() {
        let english = english_keys();
        let mut report = format!("{} English texts\n", english.len());
        // MASSIF_I18N_DUMP=<file>: the texts as a JSON list, to translate.
        if let Some(path) = std::env::var_os("MASSIF_I18N_DUMP") {
            std::fs::write(path, serde_json::to_string_pretty(&english).unwrap()).unwrap();
        }
        for language in Language::ALL.into_iter().skip(1) {
            let catalog = catalog_of(language).unwrap();
            let done = english.iter().filter(|k| catalog.contains(k)).count();
            report += &format!("{} {}: {done} ({:.1} %)\n", language.code(), language.native_name(), done as f64 * 100.0 / english.len() as f64);
            if std::env::var_os("MASSIF_I18N_MISSING").is_some_and(|v| v == language.code()) {
                for k in english.iter().filter(|k| !catalog.contains(k)) {
                    report += &format!("  missing: {k:?}\n");
                }
            }
        }
        println!("{report}");
    }

    #[test]
    fn a_missing_text_shows_in_english() {
        set_language(Language::French);
        assert_eq!(tr("A text no language has, to test the fallback"), "A text no language has, to test the fallback");
        assert_eq!(tr!("{n} things nobody translated", n = 3), "3 things nobody translated");
        assert_eq!(tr_count("{n} gizmo nobody has", "{n} gizmos nobody has", 1.0), "{n} gizmo nobody has");
        assert_eq!(tr("Save"), "Enregistrer", "a text French has shows in French");
        set_language(Language::English);
    }

    #[test]
    fn a_broken_language_file_shows_english_without_panicking() {
        for broken in ["", "{", "[1, 2]", "{\"Save\": 3}", "{\"Save\": \"\"}", "{\"Save\": {\"one\": 1}}", "\u{feff}nonsense", "{\"Undo {what}\": \"Annuler\"}"] {
            let catalog = Catalog::parse("broken", broken);
            assert!(catalog.get("Save").is_none() && catalog.get("Undo {what}").is_none(), "{broken:?}");
        }
        // A good text beside a bad one still reads.
        let catalog = Catalog::parse("half", "{\"Save\": \"Enregistrer\", \"Undo {what}\": \"Annuler {quoi}\"}");
        assert!(matches!(catalog.get("Save"), Some(Entry::One("Enregistrer"))));
        assert!(catalog.get("Undo {what}").is_none());
    }

    #[test]
    fn placeholders_are_filled_and_counts_agree() {
        assert_eq!(fill("{n} blocks in {name}", &[("n", &12), ("name", &"Alps")]), "12 blocks in Alps");
        assert_eq!(fill("{size:.1} MB", &[("size", &2.345)]), "2.3 MB");
        assert_eq!(fill("{missing} stays, {unclosed", &[]), "{missing} stays, {unclosed");
        assert_eq!(placeholders("{b} and {a:.2} and {b}"), ["a", "b"]);
        let what = "Raise";
        assert_eq!(tr!("Undo {what}", what), "Undo Raise");
        assert_eq!(trn!("{n} block", "{n} blocks", n = 1), "1 block");
        assert_eq!(trn!("{n} block", "{n} blocks", n = 0), "0 blocks");
        assert!(is_one(Language::French, 0.0) && !is_one(Language::German, 0.0));
        let ru = |n: f64| form_of(Language::Russian, n);
        assert_eq!([1.0, 21.0, 101.0].map(ru), [Form::One; 3]);
        assert_eq!([2.0, 4.0, 22.0, 104.0].map(ru), [Form::Few; 4]);
        assert_eq!([0.0, 5.0, 11.0, 12.0, 14.0, 20.0, 111.0].map(ru), [Form::Many; 7]);
        assert_eq!(ru(1.5), Form::Other);
        let catalog = Catalog::parse("ru", r#"{"{n} blocks": {"one": "{n} блок", "few": "{n} блока", "many": "{n} блоков", "other": "{n} блока"}}"#);
        assert!(matches!(catalog.get("{n} blocks"), Some(Entry::Plural { few: Some("{n} блока"), many: Some("{n} блоков"), .. })));
    }

    #[test]
    fn system_codes_name_their_language() {
        assert_eq!(Language::from_code("de-AT"), Some(Language::German));
        assert_eq!(Language::from_code("fr_FR.UTF-8"), Some(Language::French));
        assert_eq!(Language::from_code("pt-BR"), Some(Language::Portuguese));
        assert_eq!(Language::from_code("ru-RU"), Some(Language::Russian));
        assert_eq!(Language::from_code("ja"), None);
        assert_eq!(Language::from_code(""), None);
    }
}

