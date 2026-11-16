//! The instances of modpacks installed on this computer (CurseForge,
//! Prism, MultiMC, Modrinth, the official launcher's own folder): their
//! version of the game, the mods with blocks in them, the resource packs
//! the player turned on, and the lists of blocks made for WorldPainter.
//!
//! Reading an instance is what makes a modpack's blocks one step away:
//! Conquest Reforged's instance gives its jar, its resource packs and its
//! `conquest_wp.csv` at once.

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
    /// The jars of its mods that have blocks, by name.
    pub mods: Vec<PathBuf>,
    /// The resource packs it has turned on, the lowest first.
    pub packs: Vec<PathBuf>,
    /// Lists of blocks made for WorldPainter, in the instance or in
    /// WorldPainter's folder, for the namespaces of its mods.
    pub lists: Vec<PathBuf>,
}

impl Instance {
    /// Everything to read, in order: the mods, their lists, then the packs.
    pub fn files(&self) -> Vec<PathBuf> {
        self.mods.iter().chain(&self.lists).chain(&self.packs).cloned().collect()
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
        for jar in files_in(&dir.join("mods"), "jar") {
            let spaces = block_namespaces(&jar);
            if !spaces.is_empty() {
                namespaces.extend(spaces);
                instance.mods.push(jar);
            }
        }
        namespaces.sort();
        namespaces.dedup();
        instance.packs = packs_on(&dir);
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

    /// Whether one of its mods is the one named (`conquest`): by its jar's
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
    let mut out: Vec<String> = names
        .iter()
        .filter_map(|n| {
            let rest = n.strip_prefix("assets/")?;
            let (ns, rest) = rest.split_once('/')?;
            (rest.starts_with("blockstates/") && rest.ends_with(".json")).then(|| ns.to_owned())
        })
        .collect();
    out.sort();
    out.dedup();
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
        for sub in ["PrismLauncher/instances", "ModrinthApp/profiles"] {
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

/// The first instance on this computer with a mod named `name`
/// (`conquest`) among its jars: read only for that one.
pub fn find_with_mod(name: &str) -> Option<Instance> {
    let name = name.to_lowercase();
    find_instance_dirs()
        .into_iter()
        .find(|dir| files_in(&dir.join("mods"), "jar").iter().any(|j| j.file_name().is_some_and(|f| f.to_string_lossy().to_lowercase().contains(&name))))
        .and_then(|dir| Instance::read(&dir))
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
        assert_eq!(instance.files(), [conquest.clone(), dir.join("conquest_wp.csv"), dir.join("resourcepacks/Conquest.zip")]);
        // From its mods folder too.
        assert_eq!(Instance::read(&dir.join("mods")).unwrap().dir, dir);
        assert!(Instance::read(&dir.join("resourcepacks")).is_none());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
