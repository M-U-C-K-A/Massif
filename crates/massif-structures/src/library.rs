//! The game's structure data read from the player's jars, each file read
//! once: templates, template pools, processor lists and block tags.

use crate::pool::{Element, Pool, Projection};
use crate::processor::{Processor, Tag};
use crate::source::{Kind, Source, full_id, paths};
use crate::template::Template;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::{Arc, Mutex};

pub struct Library {
    source: Source,
    templates: Mutex<HashMap<String, Option<Arc<Template>>>>,
    pools: Mutex<HashMap<String, Option<Arc<Pool>>>>,
    processors: Mutex<HashMap<String, Arc<Vec<Processor>>>>,
    tags: Mutex<HashMap<String, Tag>>,
    /// What Massif met and does not know (a processor, a feature, a kind
    /// of pool element), to tell the player.
    unknown: Mutex<BTreeSet<String>>,
    /// The game's version, as its jar names it (`1.21.11`).
    pub version: String,
    /// The number the game stamps on what it saves (its `version.json`).
    data_version: i32,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Library {
    pub fn new(source: Source, version: &str) -> Self {
        Self {
            source,
            templates: Mutex::default(),
            pools: Mutex::default(),
            processors: Mutex::default(),
            tags: Mutex::default(),
            unknown: Mutex::default(),
            version: version.to_owned(),
            data_version: 0,
        }
        .with_data_version()
    }

    fn with_data_version(mut self) -> Self {
        self.data_version = self
            .source
            .read("version.json")
            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
            .and_then(|j| j.get("world_version")?.as_i64())
            .unwrap_or(0) as i32;
        self
    }

    /// The number the game stamps on what it saves: what the structures
    /// read from it are.
    pub fn data_version(&self) -> i32 {
        self.data_version
    }

    /// The game's jar and the mods' jars.
    pub fn open(jar: &std::path::Path, version: &str, mods: &[std::path::PathBuf]) -> Result<Self, massif_assets::zip::ZipError> {
        Ok(Self::new(Source::open(jar, mods)?, version))
    }

    pub fn source(&self) -> &Source {
        &self.source
    }

    fn read(&self, kind: Kind, id: &str) -> Option<Vec<u8>> {
        paths(kind, id).iter().find_map(|p| self.source.read(p))
    }

    pub fn json(&self, kind: Kind, id: &str) -> Option<serde_json::Value> {
        serde_json::from_slice(&self.read(kind, id)?).ok()
    }

    /// Note something Massif does not know.
    pub fn unknown(&self, what: impl Into<String>) {
        lock(&self.unknown).insert(what.into());
    }

    /// What Massif met and does not know, so far.
    pub fn unknowns(&self) -> Vec<String> {
        lock(&self.unknown).iter().cloned().collect()
    }

    pub fn template(&self, id: &str) -> Option<Arc<Template>> {
        let id = full_id(id);
        if let Some(t) = lock(&self.templates).get(&id) {
            return t.clone();
        }
        let t = self.read(Kind::Template, &id).and_then(|b| Template::parse(&b).ok()).map(Arc::new);
        if t.is_none() {
            self.unknown(format!("template {id} (missing or damaged)"));
        }
        lock(&self.templates).insert(id, t.clone());
        t
    }

    pub fn pool(&self, id: &str) -> Option<Arc<Pool>> {
        let id = full_id(id);
        if let Some(p) = lock(&self.pools).get(&id) {
            return p.clone();
        }
        let p = if id == "minecraft:empty" {
            Some(Arc::new(Pool { name: id.clone(), fallback: id.clone(), elements: Vec::new() }))
        } else {
            self.json(Kind::Pool, &id).map(|j| Arc::new(self.parse_pool(&id, &j)))
        };
        lock(&self.pools).insert(id, p.clone());
        p
    }

    fn parse_pool(&self, id: &str, json: &serde_json::Value) -> Pool {
        let fallback = json.get("fallback").and_then(|f| f.as_str()).map_or_else(|| "minecraft:empty".to_owned(), full_id);
        let elements = json
            .get("elements")
            .and_then(|e| e.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|e| {
                        let weight = e.get("weight").and_then(|w| w.as_u64()).unwrap_or(1).min(150) as u32;
                        Some((Arc::new(self.parse_element(e.get("element")?)), weight))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Pool { name: id.to_owned(), fallback, elements }
    }

    fn parse_element(&self, json: &serde_json::Value) -> Element {
        let kind = json.get("element_type").and_then(|t| t.as_str()).unwrap_or("");
        let projection = match json.get("projection").and_then(|p| p.as_str()) {
            Some("terrain_matching") => Projection::TerrainMatching,
            _ => Projection::Rigid,
        };
        match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "single_pool_element" | "legacy_single_pool_element" => {
                let location = json.get("location").and_then(|l| l.as_str()).map(full_id).unwrap_or_default();
                let (processors, name) = match json.get("processors") {
                    Some(serde_json::Value::String(s)) => (self.processor_list(s), full_id(s)),
                    Some(inline) => (Arc::new(Processor::parse_list(inline, &|t| self.tag(t))), String::new()),
                    None => (Arc::new(Vec::new()), String::new()),
                };
                for u in Processor::unknown(&processors) {
                    self.unknown(format!("processor {u}"));
                }
                Element::Single { location, processors, processors_name: name, projection, legacy: kind.ends_with("legacy_single_pool_element") }
            }
            "list_pool_element" => Element::List {
                elements: json.get("elements").and_then(|e| e.as_array()).map(|e| e.iter().map(|e| self.parse_element(e)).collect()).unwrap_or_default(),
                projection,
            },
            "feature_pool_element" => Element::Feature { feature: json.get("feature").and_then(|f| f.as_str()).map(full_id).unwrap_or_default(), projection },
            "empty_pool_element" => Element::Empty,
            other => {
                self.unknown(format!("pool element {other}"));
                Element::Empty
            }
        }
    }

    pub fn processor_list(&self, id: &str) -> Arc<Vec<Processor>> {
        let id = full_id(id);
        if let Some(p) = lock(&self.processors).get(&id) {
            return p.clone();
        }
        let list = match self.json(Kind::Processors, &id) {
            Some(j) => Processor::parse_list(&j, &|t| self.tag(t)),
            None => {
                if id != "minecraft:empty" {
                    self.unknown(format!("processor list {id} (missing)"));
                }
                Vec::new()
            }
        };
        let list = Arc::new(list);
        lock(&self.processors).insert(id, list.clone());
        list
    }

    /// The blocks of a block tag (`#minecraft:logs`), the tags it names
    /// opened.
    pub fn tag(&self, id: &str) -> Tag {
        let id = full_id(id);
        if let Some(t) = lock(&self.tags).get(&id) {
            return t.clone();
        }
        let mut out = HashSet::new();
        let mut seen = HashSet::new();
        let mut todo = vec![id.clone()];
        while let Some(t) = todo.pop() {
            if !seen.insert(t.clone()) {
                continue;
            }
            let Some(json) = self.json(Kind::BlockTag, &t) else { continue };
            for v in json.get("values").and_then(|v| v.as_array()).into_iter().flatten() {
                // An entry is a name, or `{"id": …, "required": false}`.
                let Some(name) = v.as_str().or_else(|| v.get("id").and_then(|i| i.as_str())) else { continue };
                match name.strip_prefix('#') {
                    Some(inner) => todo.push(full_id(inner)),
                    None => {
                        out.insert(full_id(name));
                    }
                }
            }
        }
        let tag: Tag = Arc::new(out);
        lock(&self.tags).insert(id, tag.clone());
        tag
    }
}
