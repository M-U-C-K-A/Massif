//! Where the game's structures are read from: the player's own client jar,
//! and the jars of their mods, which keep their structures the same way
//! (`data/<namespace>/…`). Nothing of the game ships with Massif.

use massif_assets::zip::Archive;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Files of the game's data, by their path in a jar (`data/minecraft/…`).
pub enum Source {
    /// Jars, the game's first: a mod's file of the same path wins, as a
    /// data pack's does in the game.
    Jars(Vec<(PathBuf, Archive)>),
    /// Files given by path, for the tests.
    Memory(HashMap<String, Vec<u8>>),
}

impl Source {
    /// The game's jar and the mods' jars (any that cannot be read is left
    /// out, unless it is the game's).
    pub fn open(jar: &Path, mods: &[PathBuf]) -> Result<Self, massif_assets::zip::ZipError> {
        let mut jars = vec![(jar.to_owned(), Archive::open(jar)?)];
        for m in mods {
            if let Ok(a) = Archive::open(m) {
                jars.push((m.clone(), a));
            }
        }
        Ok(Source::Jars(jars))
    }

    pub fn read(&self, path: &str) -> Option<Vec<u8>> {
        match self {
            Source::Jars(jars) => jars.iter().rev().find_map(|(_, a)| a.read(path).and_then(Result::ok)),
            Source::Memory(files) => files.get(path).cloned(),
        }
    }

    /// The paths of the files under `prefix`.
    pub fn list(&self, prefix: &str) -> Vec<String> {
        let mut out: Vec<String> = match self {
            Source::Jars(jars) => jars.iter().flat_map(|(_, a)| a.names().filter(|n| n.starts_with(prefix) && !n.ends_with('/')).map(str::to_owned).collect::<Vec<_>>()).collect(),
            Source::Memory(files) => files.keys().filter(|n| n.starts_with(prefix)).cloned().collect(),
        };
        out.sort();
        out.dedup();
        out
    }

    /// The namespaces with data: `minecraft`, and a mod's own.
    pub fn namespaces(&self) -> Vec<String> {
        let mut out: Vec<String> = self.list("data/").iter().filter_map(|p| p.strip_prefix("data/")?.split('/').next().map(str::to_owned)).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// A resource location split: (`minecraft`, `village/plains/town_centers`).
pub fn split_id(id: &str) -> (&str, &str) {
    let id = id.strip_prefix('#').unwrap_or(id);
    match id.split_once(':') {
        Some((ns, path)) => (ns, path),
        None => ("minecraft", id),
    }
}

/// `id` with its namespace.
pub fn full_id(id: &str) -> String {
    let (ns, path) = split_id(id);
    format!("{ns}:{path}")
}

/// The paths a kind of file of `id` may have, the newest first: the game
/// renamed its folders in 1.21 (`structures` became `structure`, `blocks`
/// `block`).
pub fn paths(kind: Kind, id: &str) -> Vec<String> {
    let (ns, path) = split_id(id);
    match kind {
        Kind::Template => vec![format!("data/{ns}/structure/{path}.nbt"), format!("data/{ns}/structures/{path}.nbt")],
        Kind::Pool => vec![format!("data/{ns}/worldgen/template_pool/{path}.json")],
        Kind::Processors => vec![format!("data/{ns}/worldgen/processor_list/{path}.json")],
        Kind::Structure => vec![format!("data/{ns}/worldgen/structure/{path}.json")],
        Kind::BlockTag => vec![format!("data/{ns}/tags/block/{path}.json"), format!("data/{ns}/tags/blocks/{path}.json")],
        Kind::Feature => vec![format!("data/{ns}/worldgen/configured_feature/{path}.json")],
        Kind::PlacedFeature => vec![format!("data/{ns}/worldgen/placed_feature/{path}.json")],
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Template,
    Pool,
    Processors,
    Structure,
    BlockTag,
    Feature,
    PlacedFeature,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_and_paths() {
        assert_eq!(split_id("minecraft:village/plains"), ("minecraft", "village/plains"));
        assert_eq!(split_id("bastion/starts"), ("minecraft", "bastion/starts"));
        assert_eq!(split_id("#minecraft:logs"), ("minecraft", "logs"));
        assert_eq!(full_id("empty"), "minecraft:empty");
        assert_eq!(paths(Kind::Template, "minecraft:igloo/top")[0], "data/minecraft/structure/igloo/top.nbt");
        assert_eq!(paths(Kind::BlockTag, "logs")[1], "data/minecraft/tags/blocks/logs.json");
        let mut files = HashMap::new();
        files.insert("data/minecraft/a.json".to_owned(), b"1".to_vec());
        files.insert("data/towns/b.json".to_owned(), b"2".to_vec());
        let s = Source::Memory(files);
        assert_eq!(s.namespaces(), vec!["minecraft".to_owned(), "towns".to_owned()]);
        assert_eq!(s.read("data/towns/b.json").as_deref(), Some(&b"2"[..]));
    }
}
