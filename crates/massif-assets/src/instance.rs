//! The instances of modpacks installed on this computer (CurseForge,
//! Prism, MultiMC, Modrinth, the official launcher's own folder): their
//! version of the game, the mods with blocks in them, the resource packs
//! the player turned on, and the lists of blocks made for WorldPainter.
//!
//! Reading an instance is what makes a modpack's blocks one step away: a
//! block mod's instance gives its jar, its resource packs and the list of
//! blocks the mod writes for WorldPainter (`<mod>_wp.csv`) at once, and
//! the blocks its KubeJS scripts make (`kubejs/assets`). Every
//! instance with a mod with blocks is found the same way, whatever the mod.

use crate::materials;
use crate::zip::Archive;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// A modpack's instance, as read from its folder.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Instance {
    /// The game's folder: the one with `mods` in it.
    pub dir: PathBuf,
    /// Its name, as its launcher shows it.
    pub name: String,
    /// The version of the game it plays (`1.20.1`).
    pub minecraft: Option<String>,
    /// The jars of its mods that have blocks, by name, then its `kubejs`
    /// folder when its scripts make blocks.
    pub mods: Vec<PathBuf>,
    /// Those mods as a player knows them, by name: see [`InstanceMod`].
    pub mod_list: Vec<InstanceMod>,
    /// The loader it plays with (`Forge`, `NeoForge`, `Fabric`, `Quilt`),
    /// from its launcher's files, else from what its mods are made for.
    pub loader: Option<String>,
    /// The namespaces of their blocks, sorted.
    pub namespaces: Vec<String>,
    /// The resource packs it has turned on, the lowest first.
    pub packs: Vec<PathBuf>,
    /// Lists of blocks made for WorldPainter, in the instance or in
    /// WorldPainter's folder, for the namespaces of its mods.
    pub lists: Vec<PathBuf>,
    /// What defines biomes without blocks: its mods with biomes only, and
    /// the data packs every world of the instance gets (`datapacks`,
    /// `global_packs`, `openloader`), zips or folders.
    pub datapacks: Vec<PathBuf>,
}

/// A mod with blocks of an instance.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct InstanceMod {
    /// Its name, as its jar says it, else as its file's name gives it
    /// (`Actually Additions`).
    pub name: String,
    /// Its id, as its jar says it (`actuallyadditions`).
    pub id: Option<String>,
    /// Its jar.
    pub jar: PathBuf,
    /// The namespaces of its blocks, sorted.
    pub namespaces: Vec<String>,
    /// How many blocks it has: its block states files.
    pub blocks: usize,
}

impl InstanceMod {
    /// Its jar's file name.
    pub fn file_name(&self) -> String {
        self.jar.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// Whether `query` (lowercase) is in its name, id, namespaces or file.
    pub fn matches(&self, query: &str) -> bool {
        query.is_empty()
            || self.name.to_lowercase().contains(query)
            || self.id.as_deref().is_some_and(|i| i.to_lowercase().contains(query))
            || self.namespaces.iter().any(|n| n.contains(query))
            || self.file_name().to_lowercase().contains(query)
    }
}

impl Instance {
    /// Everything to read, in order: the mods, their lists, then the packs.
    pub fn files(&self) -> Vec<PathBuf> {
        self.mods.iter().chain(&self.lists).chain(&self.packs).chain(&self.datapacks).cloned().collect()
    }

    /// Read an instance from its folder (or the folder of its game, or its
    /// `mods` folder). `None` when it has no mods folder.
    pub fn read(dir: &Path) -> Option<Instance> {
        let dir = game_dir(dir)?;
        let mut instance = Instance {
            name: name_of(&dir),
            minecraft: minecraft_of(&dir),
            dir: dir.clone(),
            ..Default::default()
        };
        let mut namespaces = Vec::new();
        let mut loaders: Vec<&'static str> = Vec::new();
        for jar in files_in(&dir.join("mods"), "jar") {
            let Ok(archive) = Archive::open(&jar) else { continue };
            let spaces = namespaces_in(archive.names());
            if spaces.is_empty() && has_biomes(archive.names()) {
                instance.datapacks.push(jar);
                continue;
            }
            let meta = crate::mod_meta(&archive);
            loaders.extend(meta.loader);
            if !spaces.is_empty() {
                let blocks = archive.names().filter(|n| is_blockstate(n)).count();
                instance.mod_list.push(InstanceMod { name: crate::display_mod_name(&meta, &jar), id: meta.id, jar: jar.clone(), namespaces: spaces.clone(), blocks });
                namespaces.extend(spaces);
                instance.mods.push(jar);
            }
        }
        // KubeJS's folder: the blocks its scripts make, with their states,
        // models and textures, as a mod's (after the mods, whose textures
        // its models may use).
        let kubejs = dir.join("kubejs");
        if let Some(archive) = crate::kubejs::archive(&kubejs) {
            let spaces = namespaces_in(archive.names());
            if !spaces.is_empty() {
                let blocks = archive.names().filter(|n| is_blockstate(n)).count();
                instance.mod_list.push(InstanceMod { name: "KubeJS".to_owned(), id: Some("kubejs".to_owned()), jar: kubejs.clone(), namespaces: spaces.clone(), blocks });
                namespaces.extend(spaces);
                instance.mods.push(kubejs);
            }
        }
        instance.mod_list.sort_by_cached_key(|m| (m.name.to_lowercase(), m.file_name()));
        instance.loader = loader_of(&dir).or_else(|| most_common(&loaders).map(str::to_owned));
        namespaces.sort();
        namespaces.dedup();
        instance.namespaces = namespaces.clone();
        instance.packs = packs_on(&dir);
        instance.datapacks.extend(datapacks_in(&dir));
        // The lists of the instance itself, then WorldPainter's own for the
        // namespaces left.
        let mut covered: Vec<String> = Vec::new();
        let lists = |path: PathBuf, instance: &mut Instance, covered: &mut Vec<String>| {
            if let Some(space) = list_namespace(&path)
                && namespaces.contains(&space)
                && !covered.contains(&space)
            {
                covered.push(space);
                instance.lists.push(path);
            }
        };
        for path in materials::lists_in(&dir) {
            lists(path, &mut instance, &mut covered);
        }
        for folder in materials::worldpainter_folders() {
            for path in materials::lists_in(&folder) {
                lists(path, &mut instance, &mut covered);
            }
        }
        Some(instance)
    }

    /// Whether one of its mods is the one named (`create`): by its jar's
    /// name.
    pub fn has_mod(&self, name: &str) -> bool {
        let name = name.to_lowercase();
        self.mods.iter().any(|m| m.file_name().is_some_and(|f| f.to_string_lossy().to_lowercase().contains(&name)))
    }
}

/// The folder with `mods` in it: the one given, or the `.minecraft` or
/// `minecraft` folder of a Prism or MultiMC instance, or the parent of a
/// `mods` folder.
fn game_dir(dir: &Path) -> Option<PathBuf> {
    if dir.file_name().is_some_and(|n| n == "mods") {
        return dir.parent().map(Path::to_path_buf);
    }
    [dir.to_path_buf(), dir.join(".minecraft"), dir.join("minecraft")].into_iter().find(|d| d.join("mods").is_dir())
}

fn json(path: &Path) -> Option<Value> {
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// The instance's name: from its launcher's files, else its folder's.
fn name_of(dir: &Path) -> String {
    let parent = dir.parent().filter(|_| dir.file_name().is_some_and(|n| n == ".minecraft" || n == "minecraft"));
    let root = parent.unwrap_or(dir);
    if let Some(name) = json(&root.join("minecraftinstance.json")).and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned)) {
        return name;
    }
    if let Ok(cfg) = std::fs::read_to_string(root.join("instance.cfg"))
        && let Some(name) = cfg.lines().find_map(|l| l.strip_prefix("name="))
    {
        return name.trim().to_owned();
    }
    if let Some(name) = json(&root.join("profile.json")).and_then(|v| v.get("name").and_then(Value::as_str).map(str::to_owned)) {
        return name;
    }
    root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The loader a launcher's id names (`forge-47.2.0`, `net.fabricmc.fabric-loader`).
fn loader_named(id: &str) -> Option<&'static str> {
    let id = id.to_ascii_lowercase();
    [("neoforge", "NeoForge"), ("neoforged", "NeoForge"), ("forge", "Forge"), ("fabric", "Fabric"), ("quilt", "Quilt")].into_iter().find(|(k, _)| id.contains(k)).map(|(_, v)| v)
}

/// The loader an instance plays with, from its launcher's files.
fn loader_of(dir: &Path) -> Option<String> {
    let parent = dir.parent().filter(|_| dir.file_name().is_some_and(|n| n == ".minecraft" || n == "minecraft"));
    let root = parent.unwrap_or(dir);
    let found = json(&root.join("manifest.json"))
        .and_then(|v| v.pointer("/minecraft/modLoaders/0/id").and_then(Value::as_str).and_then(loader_named))
        .or_else(|| json(&root.join("minecraftinstance.json")).and_then(|v| v.pointer("/baseModLoader/name").and_then(Value::as_str).and_then(loader_named)))
        .or_else(|| {
            json(&root.join("mmc-pack.json")).and_then(|v| v.get("components")?.as_array()?.iter().filter_map(|c| c.get("uid").and_then(Value::as_str)).filter(|u| *u != "net.minecraft").find_map(loader_named))
        })
        .or_else(|| json(&root.join("profile.json")).and_then(|v| v.get("loader").and_then(Value::as_str).and_then(loader_named)));
    found.map(str::to_owned)
}

/// The value most often in `of`, the first of those as often.
fn most_common<'a>(of: &[&'a str]) -> Option<&'a str> {
    let mut best: Option<(&str, usize)> = None;
    for v in of {
        let n = of.iter().filter(|o| *o == v).count();
        if best.is_none_or(|(_, b)| n > b) {
            best = Some((v, n));
        }
    }
    best.map(|(v, _)| v)
}

/// The version of the game an instance plays, from its launcher's files.
fn minecraft_of(dir: &Path) -> Option<String> {
    let parent = dir.parent().filter(|_| dir.file_name().is_some_and(|n| n == ".minecraft" || n == "minecraft"));
    let root = parent.unwrap_or(dir);
    let found = json(&root.join("manifest.json"))
        .and_then(|v| v.pointer("/minecraft/version").and_then(Value::as_str).map(str::to_owned))
        .or_else(|| json(&root.join("minecraftinstance.json")).and_then(|v| v.get("gameVersion").and_then(Value::as_str).map(str::to_owned)))
        .or_else(|| {
            json(&root.join("mmc-pack.json")).and_then(|v| {
                v.get("components")?
                    .as_array()?
                    .iter()
                    .find(|c| c.get("uid").and_then(Value::as_str) == Some("net.minecraft"))?
                    .get("version")?
                    .as_str()
                    .map(str::to_owned)
            })
        })
        .or_else(|| json(&root.join("profile.json")).and_then(|v| v.get("game_version").and_then(Value::as_str).map(str::to_owned)));
    found.filter(|v| crate::release_numbers(v).is_some())
}

/// The files of a folder with an extension, by name, leaving out the
/// `._` files macOS writes on other disks.
fn files_in(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case(ext)) && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("._")))
        .collect();
    out.sort();
    out
}

/// The namespaces whose block states a jar has (`assets/<ns>/blockstates/`),
/// sorted; reading only its directory.
pub fn block_namespaces(jar: &Path) -> Vec<String> {
    let Ok(names) = Archive::list(jar) else { return Vec::new() };
    namespaces_in(names.iter().map(String::as_str))
}

/// The namespace of a jar's entry when it is a block states file
/// (`assets/<ns>/blockstates/<block>.json`).
fn blockstate_namespace(name: &str) -> Option<&str> {
    let rest = name.strip_prefix("assets/")?;
    let (ns, rest) = rest.split_once('/')?;
    (rest.starts_with("blockstates/") && rest.ends_with(".json")).then_some(ns)
}

fn is_blockstate(name: &str) -> bool {
    blockstate_namespace(name).is_some()
}

/// The namespaces of the block states among a jar's entries.
fn namespaces_in<'a>(names: impl Iterator<Item = &'a str>) -> Vec<String> {
    let mut out: Vec<String> = names.filter_map(|n| blockstate_namespace(n).map(str::to_owned)).collect();
    out.sort();
    out.dedup();
    out
}

/// Whether a jar's entries define biomes (`data/<ns>/worldgen/biome/`).
fn has_biomes<'a>(mut names: impl Iterator<Item = &'a str>) -> bool {
    names.any(|n| n.starts_with("data/") && n.contains("/worldgen/biome/") && n.ends_with(".json"))
}

/// The data packs an instance gives every world, that define biomes: the
/// zips and folders of its `datapacks` folder and of the folders of the
/// mods that load data packs for every world (`global_packs/required_data`
/// and `optional_data`, `openloader/data`).
pub fn datapacks_in(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for sub in ["datapacks", "global_packs/required_data", "global_packs/optional_data", "openloader/data", "config/openloader/data"] {
        let Ok(read) = std::fs::read_dir(dir.join(sub)) else { continue };
        let mut found: Vec<PathBuf> = read
            .flatten()
            .map(|e| e.path())
            .filter(|p| !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')))
            .filter(|p| {
                if p.is_dir() {
                    p.join("data").is_dir() && std::fs::read_dir(p.join("data")).into_iter().flatten().flatten().any(|ns| ns.path().join("worldgen/biome").is_dir())
                } else {
                    p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip") || e.eq_ignore_ascii_case("jar")) && Archive::list(p).is_ok_and(|names| has_biomes(names.iter().map(String::as_str)))
                }
            })
            .collect();
        found.sort();
        out.extend(found);
    }
    out
}

/// The namespace of a list of blocks: that of its first block.
fn list_namespace(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    if !lines.next()?.trim_start_matches('\u{feff}').starts_with("name,") {
        return None;
    }
    let first = lines.find(|l| l.contains(':'))?;
    Some(crate::namespace(first.split(',').next()?.trim_matches('"')).to_owned())
}

/// The resource packs an instance turned on (`resourcePacks` in its
/// `options.txt`, the lowest first), those of its `resourcepacks` folder
/// that are zips.
fn packs_on(dir: &Path) -> Vec<PathBuf> {
    let Ok(options) = std::fs::read_to_string(dir.join("options.txt")) else { return Vec::new() };
    let Some(line) = options.lines().find_map(|l| l.strip_prefix("resourcePacks:")) else { return Vec::new() };
    let Ok(Value::Array(packs)) = serde_json::from_str::<Value>(line.trim()) else { return Vec::new() };
    packs
        .iter()
        .filter_map(Value::as_str)
        .filter_map(|p| p.strip_prefix("file/"))
        .map(|p| dir.join("resourcepacks").join(p))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")))
        .collect()
}

/// The folders launchers keep their instances in, on this computer.
fn instance_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    if let Some(home) = &home {
        for sub in [
            "Documents/curseforge/minecraft/Instances",
            "curseforge/minecraft/Instances",
            "Library/Application Support/PrismLauncher/instances",
            ".local/share/PrismLauncher/instances",
            "Library/Application Support/ModrinthApp/profiles",
            ".local/share/ModrinthApp/profiles",
        ] {
            roots.push(home.join(sub));
        }
    }
    if let Some(appdata) = std::env::var_os("APPDATA").map(PathBuf::from) {
        for sub in ["PrismLauncher/instances", "ModrinthApp/profiles", "com.modrinth.theseus/profiles", "MultiMC/instances", "ATLauncher/instances", "gdlauncher_next/instances", "gdlauncher_carbon/data/instances"] {
            roots.push(appdata.join(sub));
        }
    }
    // Launchers installed on other drives (CurseForge often is).
    for volume in ["/Volumes", "/media", "/mnt"] {
        if let Ok(read) = std::fs::read_dir(volume) {
            for v in read.flatten() {
                for sub in ["minecraft/Instances", "curseforge/minecraft/Instances"] {
                    roots.push(v.path().join(sub));
                }
            }
        }
    }
    roots
}

/// The game folders of the instances on this computer, each with a mods
/// folder, by name. Only their folders are read, not their jars.
pub fn find_instance_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for root in instance_roots() {
        let Ok(read) = std::fs::read_dir(&root) else { continue };
        for entry in read.flatten() {
            if let Some(dir) = game_dir(&entry.path()) {
                out.push(dir);
            }
        }
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    for game in [home.as_ref().map(|h| h.join("Library/Application Support/minecraft")), home.as_ref().map(|h| h.join(".minecraft")), std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join(".minecraft"))].into_iter().flatten() {
        if game.join("mods").is_dir() {
            out.push(game);
        }
    }
    out.sort();
    out.dedup();
    out
}

/// Whether a game folder's mods have a jar named like `name` (lowercase).
fn has_mod_jar(dir: &Path, name: &str) -> bool {
    files_in(&dir.join("mods"), "jar").iter().any(|j| j.file_name().is_some_and(|f| f.to_string_lossy().to_lowercase().contains(name)))
}

/// The instances of these game folders with a mod named like `name`, read,
/// those with lists of blocks for it first (a mod's own lists say which of
/// its blocks are real and what states they have), then by name.
fn read_with_mod(dirs: impl IntoIterator<Item = PathBuf>, name: &str) -> Vec<Instance> {
    let name = name.to_lowercase();
    let mut out: Vec<Instance> = dirs.into_iter().filter(|dir| has_mod_jar(dir, &name)).filter_map(|dir| Instance::read(&dir)).collect();
    out.sort_by_key(|i| (i.lists.is_empty(), i.name.to_lowercase()));
    out
}

/// Instances read, those with mods with blocks, those with lists of blocks
/// first (a mod's own lists say which of its blocks are real and what
/// states they have), then by name.
fn with_blocks(dirs: impl IntoIterator<Item = PathBuf>) -> Vec<Instance> {
    let mut out: Vec<Instance> = dirs.into_iter().filter_map(|dir| Instance::read(&dir)).filter(|i| !i.mods.is_empty()).collect();
    out.sort_by_key(|i| (i.lists.is_empty(), i.name.to_lowercase()));
    out
}

/// Every instance on this computer with a mod with blocks, whatever the
/// mod: see [`with_blocks`]. Every jar of their mods folders is looked
/// into, its directory only.
pub fn find_all_with_blocks() -> Vec<Instance> {
    with_blocks(find_instance_dirs())
}

/// The instances with a mod with blocks in a folder: the folder itself
/// when it is one, else those it holds (a launcher's folder of instances).
pub fn with_blocks_in(dir: &Path) -> Vec<Instance> {
    if let Some(game) = game_dir(dir) {
        return with_blocks([game]);
    }
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    with_blocks(read.flatten().filter_map(|e| game_dir(&e.path())))
}

/// Every instance on this computer with a mod named `name` (`create`)
/// among its jars: see [`read_with_mod`]. Only those instances' jars are
/// looked into.
pub fn find_all_with_mod(name: &str) -> Vec<Instance> {
    read_with_mod(find_instance_dirs(), name)
}

/// The instances with a mod named `name` in a folder: the folder itself
/// when it is one, else those it holds (a launcher's folder of instances).
pub fn with_mod_in(dir: &Path, name: &str) -> Vec<Instance> {
    if let Some(game) = game_dir(dir) {
        return read_with_mod([game], name);
    }
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    read_with_mod(read.flatten().filter_map(|e| game_dir(&e.path())), name)
}

/// The instance on this computer with a mod named `name` that reads best:
/// see [`find_all_with_mod`].
pub fn find_with_mod(name: &str) -> Option<Instance> {
    find_all_with_mod(name).into_iter().next()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::zip::tests_support::tiny_zip;

    /// A CurseForge instance as Conquest Reforged's is laid out: its
    /// manifest, a mod with blocks and one without, a pack turned on and
    /// one not, its list of blocks for WorldPainter and a list of another
    /// mod's.
    #[test]
    fn instances_read() {
        let dir = std::env::temp_dir().join(format!("massif-instance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("mods")).unwrap();
        std::fs::create_dir_all(dir.join("resourcepacks")).unwrap();
        std::fs::write(dir.join("manifest.json"), r#"{"minecraft":{"version":"1.20.1","modLoaders":[{"id":"fabric-0.16.11"}]},"name":"Conquest Reforged Modpack"}"#).unwrap();
        std::fs::write(dir.join("minecraftinstance.json"), r#"{"name":"Conquest Reforged Modpack (Fabric)","gameVersion":"1.20.1"}"#).unwrap();
        let conquest = dir.join("mods/ConquestReforged-fabric-1.20.1-1.4.1.4.jar");
        std::fs::write(&conquest, tiny_zip(&[("fabric.mod.json", br#"{"name":"ConquestReforged Fabric"}"#, true), ("assets/conquest/blockstates/old_red_bricks.json", br#"{"variants":{}}"#, true)])).unwrap();
        std::fs::write(dir.join("mods/sodium.jar"), tiny_zip(&[("assets/sodium/textures/block/x.png", b"", false)])).unwrap();
        std::fs::write(dir.join("mods/._ConquestReforged-fabric-1.20.1-1.4.1.4.jar"), b"apple double").unwrap();
        std::fs::write(dir.join("resourcepacks/Conquest.zip"), tiny_zip(&[("pack.mcmeta", b"{}", false)])).unwrap();
        std::fs::write(dir.join("resourcepacks/Off.zip"), tiny_zip(&[("pack.mcmeta", b"{}", false)])).unwrap();
        std::fs::write(dir.join("options.txt"), "version:3465\nresourcePacks:[\"vanilla\",\"fabric\",\"file/Conquest.zip\",\"conquest:rp_crrp\"]\nlang:en_us\n").unwrap();
        std::fs::write(dir.join("conquest_wp.csv"), crate::materials::tests::CONQUEST).unwrap();
        std::fs::write(dir.join("other_wp.csv"), "name,properties\nchipped:x,\n").unwrap();
        let instance = Instance::read(&dir).unwrap();
        assert_eq!(instance.name, "Conquest Reforged Modpack (Fabric)");
        assert_eq!(instance.minecraft.as_deref(), Some("1.20.1"));
        assert_eq!(instance.mods, std::slice::from_ref(&conquest));
        assert_eq!(instance.packs, [dir.join("resourcepacks/Conquest.zip")]);
        assert_eq!(instance.lists, [dir.join("conquest_wp.csv")], "only the list of a mod it has");
        assert!(instance.has_mod("conquest") && !instance.has_mod("chipped"));
        assert_eq!(instance.mod_list, [InstanceMod { name: "Conquest Reforged".into(), id: None, jar: conquest.clone(), namespaces: vec!["conquest".into()], blocks: 1 }]);
        assert_eq!(instance.namespaces, ["conquest"]);
        assert_eq!(instance.loader.as_deref(), Some("Fabric"), "from its manifest");
        assert_eq!(with_blocks_in(&dir).len(), 1);
        assert_eq!(with_blocks_in(dir.parent().unwrap()).iter().filter(|i| i.dir == dir).count(), 1);
        assert_eq!(instance.files(), [conquest.clone(), dir.join("conquest_wp.csv"), dir.join("resourcepacks/Conquest.zip")]);
        // From its mods folder too.
        assert_eq!(Instance::read(&dir.join("mods")).unwrap().dir, dir);
        // Named, or among those of a folder of instances.
        assert_eq!(with_mod_in(&dir, "conquest").len(), 1);
        assert_eq!(with_mod_in(dir.parent().unwrap(), "ConquestReforged").iter().filter(|i| i.dir == dir).count(), 1);
        assert!(with_mod_in(&dir, "chipped").is_empty());
        assert!(Instance::read(&dir.join("resourcepacks")).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
