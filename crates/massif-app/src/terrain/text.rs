//! The panel's texts in the user's language. The panel keeps its own
//! catalogues, `lang/<code>.json` beside this file, in the app's format
//! (`"English": "translation"`): the node set of 1.3 is new, and keeping
//! its words apart lets it grow without touching the app's files while
//! sessions work side by side. A text a catalogue lacks falls back on the
//! app's catalogue, then on the English.

use crate::i18n::{self, Language};
use std::collections::HashMap;
use std::sync::OnceLock;

/// A text of the panel in the language shown: `t!("Viewer")`, or with
/// placeholders `t!("{n} points", n = 3)`.
macro_rules! t {
    ($s:literal) => {
        $crate::terrain::text::t($s)
    };
    ($s:literal, $($name:ident = $val:expr),+ $(,)?) => {
        $crate::i18n::fill($crate::terrain::text::t($s), &[$((stringify!($name), &$val as &dyn ::std::fmt::Display)),+])
    };
}

type Catalog = HashMap<String, &'static str>;

fn source(language: Language) -> Option<&'static str> {
    match language {
        Language::French => Some(include_str!("lang/fr.json")),
        Language::German => Some(include_str!("lang/de.json")),
        Language::Spanish => Some(include_str!("lang/es.json")),
        Language::Portuguese => Some(include_str!("lang/pt.json")),
        _ => None,
    }
}

fn catalog(language: Language) -> Option<&'static Catalog> {
    static CATALOGS: OnceLock<HashMap<u8, Catalog>> = OnceLock::new();
    let all = CATALOGS.get_or_init(|| {
        let mut all = HashMap::new();
        for (k, l) in Language::ALL.into_iter().enumerate() {
            let Some(text) = source(l) else { continue };
            let parsed: HashMap<String, String> = serde_json::from_str(text).unwrap_or_default();
            all.insert(k as u8, parsed.into_iter().filter(|(_, v)| !v.is_empty()).map(|(k, v)| (k, &*Box::leak(v.into_boxed_str()))).collect());
        }
        all
    });
    let k = Language::ALL.iter().position(|l| *l == language)?;
    all.get(&(k as u8))
}

/// The text in the language shown.
pub fn t(english: &'static str) -> &'static str {
    let language = i18n::current();
    if language == Language::English {
        return english;
    }
    match catalog(language).and_then(|c| c.get(english)) {
        Some(s) => s,
        None => i18n::tr(english),
    }
}

/// A name from the registry (a node's, a setting's, a family's): its
/// translation when there is one.
pub fn name(english: &'static str) -> &'static str {
    t(english)
}

/// A name a document or a composite gives (not known when building):
/// its translation when a catalogue has one, else as it is.
pub fn name_owned(english: &str) -> String {
    let language = i18n::current();
    if language == Language::English {
        return english.to_owned();
    }
    match catalog(language).and_then(|c| c.get(english)) {
        Some(s) => (*s).to_owned(),
        None => english.to_owned(),
    }
}

/// Every key of every catalogue, for the tests.
#[cfg(test)]
pub fn keys(language: Language) -> Vec<String> {
    catalog(language).map(|c| c.keys().cloned().collect()).unwrap_or_default()
}
