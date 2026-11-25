//! Minecraft's real blocks in the editor: read from the player's own
//! installation of the game (see `massif_assets`), drawn with their real
//! textures, and paintable on the map.

use egui::{ColorImage, TextureHandle, TextureOptions};
use massif_assets::instance::Instance;
use massif_assets::{Block, Catalogue, Jar, SourceKind};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// Pixels of the icons (drawn at half that in points, for sharp edges).
const ICON: usize = 48;

type Loaded = (Vec<Jar>, Result<Catalogue, String>, Vec<Instance>);

#[derive(Default)]
pub struct BlockLibrary {
    /// Client jars found on this computer, newest first.
    pub jars: Vec<Jar>,
    pub catalogue: Option<Arc<Catalogue>>,
    pending: Option<Receiver<Loaded>>,
    icons: HashMap<String, TextureHandle>,
    pub error: Option<String>,
    /// The world whose registry has the real colours.
    pub applied: Option<u64>,
    pub search: String,
    /// Show the blocks that are not whole cubes (flowers, torches…).
    pub all: bool,
    /// The instances installed here with mods with blocks not read yet,
    /// whatever the mods, the best read first: each offered in one click.
    pub instances: Vec<Instance>,
    /// What the list of instances is searched for.
    pub instance_search: String,
    /// What reading an instance left to say (the version of the game it
    /// plays is not installed…).
    pub notice: Option<String>,
    /// A block id typed by hand (`mod:block[property=value]`), for a block
    /// nothing in the panel finds.
    pub typed: String,
    /// The sections the panel shows, for the catalogue, search and shapes
    /// they were made for.
    sections: Option<(usize, String, bool, Arc<Vec<Section>>)>,
}

/// The instances installed here with mods with blocks. In tests, only
/// those of the folder MASSIF_INSTANCES_DIR (or MASSIF_CONQUEST_DIR) names,
/// so that no test depends on what the computer running it has installed.
fn installed_instances() -> Vec<Instance> {
    if cfg!(test) {
        let dir = std::env::var_os("MASSIF_INSTANCES_DIR").or_else(|| std::env::var_os("MASSIF_CONQUEST_DIR"));
        return dir.map(|d| massif_assets::instance::with_blocks_in(std::path::Path::new(&d))).unwrap_or_default();
    }
    massif_assets::instance::find_all_with_blocks()
}

/// The namespaces of the mods a catalogue has read.
pub fn read_namespaces(cat: &Catalogue) -> Vec<&str> {
    cat.mods.iter().filter(|m| m.kind == SourceKind::Mod).flat_map(|m| m.namespaces.iter().map(String::as_str)).collect()
}

/// A section of the Blocks panel: a title and its blocks, by their index in
/// the catalogue.
pub struct Section {
    pub title: String,
    pub blocks: Vec<u32>,
}

impl BlockLibrary {
    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    /// The namespaces of the mods whose blocks are read.
    pub fn namespaces(&self) -> Vec<String> {
        self.catalogue.as_deref().map(|c| read_namespaces(c).into_iter().map(str::to_owned).collect()).unwrap_or_default()
    }

    /// The palettes that come with Massif to offer now: the game's, and
    /// those of the mods read (see `massif_core::mod_profiles`).
    pub fn offered_palettes(&self) -> Vec<massif_core::materials::Palette> {
        let read = self.namespaces();
        massif_core::materials::Palette::offered(&read.iter().map(String::as_str).collect::<Vec<_>>())
    }

    /// An instance installed here to read for the blocks of the mods
    /// `needed` (namespaces), when some are not read yet and it has them
    /// all: the best read first.
    pub fn instance_for(&self, needed: &[&str]) -> Option<Instance> {
        let read = self.namespaces();
        let missing: Vec<&str> = needed.iter().copied().filter(|ns| !read.iter().any(|r| r == ns)).collect();
        if missing.is_empty() || self.is_loading() {
            return None;
        }
        self.instances.iter().find(|i| missing.iter().all(|ns| i.namespaces.iter().any(|n| n == ns))).cloned()
    }

    /// Find the game's jars and read `jar` (or the newest) on another thread,
    /// with the blocks of the mods' jars `mods` beside its own.
    pub fn load(&mut self, ctx: &egui::Context, jar: Option<PathBuf>, mods: Vec<PathBuf>) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            // The instances installed here, and the game beside each of
            // them and beside the mods chosen: a launcher kept elsewhere
            // than where launchers usually are (CurseForge on another
            // drive or in a folder of its own) is found by its instances.
            let mut instances = installed_instances();
            let mut jars = massif_assets::find_jars();
            jars.extend(mods.iter().flat_map(|m| massif_assets::jars_near(m)));
            jars.extend(instances.iter().flat_map(|i| massif_assets::jars_near(&i.dir.join("mods"))));
            jars.sort_by_key(|j| std::cmp::Reverse(massif_assets::release_numbers(&j.version)));
            jars.dedup_by(|a, b| a.version == b.version);
            let chosen = jar
                .filter(|p| p.is_file())
                .map(|p| {
                    let version = jars
                        .iter()
                        .find(|j| j.path == p)
                        .map(|j| j.version.clone())
                        .or_else(|| massif_assets::jar_version(&p))
                        .unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                    Jar { version, path: p }
                })
                // The version the instance of the mods plays, else the newest.
                .or_else(|| {
                    let wanted = instances.iter().find(|i| mods.iter().any(|m| m.starts_with(&i.dir))).and_then(|i| i.minecraft.clone())?;
                    jars.iter().find(|j| j.version == wanted).cloned()
                })
                .or_else(|| jars.first().cloned());
            let result = match chosen {
                // Its textures decoded in the threads kept for loads, not in
                // the pool the interface draws with (see `in_loading_pool`).
                Some(j) => crate::jobs::in_loading_pool(|| massif_assets::load_with_mods(&j.path, &j.version, &mods)).map_err(|e| format!("{}: {e}", j.path.display())),
                None => Err(tr!("No Minecraft installation was found. Choose the game's jar (versions/<version>/<version>.jar).").to_owned()),
            };
            // The instances installed here with block mods not read yet,
            // to offer.
            let read: Vec<String> = result.as_ref().map(|c| read_namespaces(c).into_iter().map(str::to_owned).collect()).unwrap_or_default();
            instances.retain(|i| i.namespaces.iter().any(|ns| !read.contains(ns)));
            let _ = tx.send((jars, result, instances));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    /// Take the result of a load. Returns whether a catalogue arrived.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        let Ok((jars, result, instances)) = rx.try_recv() else { return false };
        self.pending = None;
        self.jars = jars;
        self.instances = instances;
        match result {
            Ok(cat) => {
                self.catalogue = Some(Arc::new(cat));
                self.error = None;
                self.icons.clear();
                self.sections = None;
                self.applied = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// Read a modpack's instance: its mods with blocks, their lists of
    /// blocks and its resource packs, over the version of the game it plays
    /// when that is installed here (else over the game read now, said in
    /// `notice`). `jar` and `mods` are the settings that remember them.
    pub fn use_instance(&mut self, ctx: &egui::Context, instance: &Instance, jar: &mut Option<PathBuf>, mods: &mut Vec<PathBuf>) {
        // The game beside the instance first: its own launcher's.
        let mut jars = massif_assets::jars_near(&instance.dir.join("mods"));
        jars.extend(if self.jars.is_empty() { massif_assets::find_jars() } else { self.jars.clone() });
        self.notice = None;
        if let Some(version) = &instance.minecraft {
            match jars.iter().find(|j| j.version == *version) {
                Some(j) => *jar = Some(j.path.clone()),
                None if jars.is_empty() && jar.as_ref().is_none_or(|p| !p.is_file()) => {
                    self.notice = Some(tr!("{instance} plays Minecraft {version}, whose jar was not found here. Start {version} once with the launcher, or choose its jar (versions/{version}/{version}.jar) with Choose….", instance = instance.name, version));
                }
                None => {
                    let current = self.catalogue.as_ref().map(|c| c.version.clone()).or_else(|| jars.first().map(|j| j.version.clone())).unwrap_or_default();
                    self.notice = Some(tr!("{instance} plays Minecraft {version}, which is not installed here: its blocks are read with Minecraft {current}'s. Install {version} with the launcher to read the game's blocks of that version.", instance = instance.name, version, current));
                }
            }
        }
        *mods = instance.files();
        if instance.mods.is_empty() {
            self.notice = Some(tr!("{instance} has no mod with blocks.", instance = instance.name));
        }
        self.instances.clear();
        self.load(ctx, jar.clone(), mods.clone());
    }

    /// The panel's sections, made again only when the catalogue, the search
    /// or the shapes shown change: the game's blocks by category, then each
    /// mod's, by its creative tabs when it gives them, else by category.
    pub fn sections(&mut self, needle: &str) -> Arc<Vec<Section>> {
        let Some(cat) = self.catalogue.clone() else { return Arc::default() };
        let key = Arc::as_ptr(&cat) as usize;
        if let Some((k, n, all, sections)) = &self.sections
            && *k == key
            && n == needle
            && *all == self.all
        {
            return sections.clone();
        }
        let sections = Arc::new(make_sections(&cat, needle, self.all));
        self.sections = Some((key, needle.to_owned(), self.all, sections.clone()));
        sections
    }

    /// The texture of a block's top, flat and tinted, made the first time:
    /// to draw ground seen from above.
    pub fn top(&mut self, ctx: &egui::Context, block: &Block) -> Option<egui::TextureId> {
        let cat = self.catalogue.as_ref()?;
        let key = format!("top {}", block.id);
        if let Some(t) = self.icons.get(&key) {
            return Some(t.id());
        }
        let tex = block.top.as_ref().or(block.side.as_ref()).and_then(|k| cat.texture(k))?.tinted(block.top_tint);
        // An animated texture is its frames one under the other: the first.
        let (w, h) = (tex.width as usize, (tex.height.min(tex.width)) as usize);
        if w == 0 || h == 0 || tex.rgba.len() < w * h * 4 {
            return None;
        }
        let image = ColorImage::from_rgba_unmultiplied([w, h], &tex.rgba[..w * h * 4]);
        let texture = ctx.load_texture(format!("block {key}"), image, TextureOptions::NEAREST);
        let id = texture.id();
        self.icons.insert(key, texture);
        Some(id)
    }

    /// The icon of a block, made the first time.
    pub fn icon(&mut self, ctx: &egui::Context, block: &Block) -> Option<egui::TextureId> {
        let cat = self.catalogue.as_ref()?;
        if let Some(t) = self.icons.get(&block.id) {
            return Some(t.id());
        }
        let rgba = if block.full_cube {
            massif_assets::cube_icon(cat, block, ICON)
        } else {
            massif_assets::flat_icon(cat, block, ICON)
        };
        let image = ColorImage::from_rgba_unmultiplied([ICON, ICON], &rgba);
        // Nearest filtering keeps the pixels of the textures crisp.
        let texture = ctx.load_texture(format!("block {}", block.id), image, TextureOptions::NEAREST);
        let id = texture.id();
        self.icons.insert(block.id.clone(), texture);
        Some(id)
    }
}

/// Whether the Blocks panel shows a block before All shapes is on: whole
/// blocks, plants, and the ground's blocks not quite whole (a path, a
/// farmland, a layer of snow), all of which can be painted as terrains or
/// covers.
pub fn shown_at_first(b: &Block) -> bool {
    b.full_cube || matches!(b.category, massif_assets::Category::Plants | massif_assets::Category::Natural)
}

/// The sections of the Blocks panel (see [`BlockLibrary::sections`]).
/// `needle` is the search, lowercase; `all` shows every shape.
fn make_sections(cat: &Catalogue, needle: &str, all: bool) -> Vec<Section> {
    // Whole blocks, plants and ground (terrains and covers) show at first; a
    // search looks through every shape, as All shapes does.
    let all = all || !needle.is_empty();
    use massif_assets::{Category, namespace};
    let matches = |b: &Block, source: Option<&str>| needle.is_empty() || b.name.to_lowercase().contains(needle) || b.id.contains(needle) || source.is_some_and(|s| s.to_lowercase().contains(needle));
    let index: HashMap<&str, u32> = cat.blocks.iter().enumerate().map(|(i, b)| (b.id.as_str(), i as u32)).collect();
    let mods: Vec<&massif_assets::ModInfo> = cat.mods.iter().filter(|m| m.kind == SourceKind::Mod && m.blocks > 0).collect();
    let of_mods: Vec<&str> = mods.iter().flat_map(|m| m.namespaces.iter().map(String::as_str)).filter(|ns| *ns != "minecraft").collect();
    let mut out = Vec::new();
    let by_category = |out: &mut Vec<Section>, source: Option<&str>, filter: &dyn Fn(&Block) -> bool| {
        for category in Category::ALL {
            let blocks: Vec<u32> = cat
                .blocks
                .iter()
                .enumerate()
                .filter(|(_, b)| b.category == category && (all || shown_at_first(b)) && filter(b) && matches(b, source))
                .map(|(i, _)| i as u32)
                .collect();
            if blocks.is_empty() {
                continue;
            }
            let category = crate::i18n::tr_name(category.label()).to_uppercase();
            let title = match source {
                Some(name) => format!("{}  ·  {category}  ·  {}", name.to_uppercase(), blocks.len()),
                None => format!("{category}  ·  {}", blocks.len()),
            };
            out.push(Section { title, blocks });
        }
    };
    by_category(&mut out, None, &|b| !of_mods.contains(&namespace(&b.id)));
    for m in mods {
        let spaces: Vec<&str> = m.namespaces.iter().map(String::as_str).filter(|ns| *ns != "minecraft").collect();
        let of_mod = |b: &Block| spaces.contains(&namespace(&b.id));
        if m.groups.is_empty() {
            by_category(&mut out, Some(&m.name), &of_mod);
            continue;
        }
        // The mod's tabs show its blocks (and a few of the game's) as the
        // game does; the shapes of a block (its slabs, stairs, arches…),
        // which the tabs leave out, follow it when every shape is shown.
        let members: HashMap<&str, &str> = m.groups.iter().flat_map(|g| g.blocks.iter()).map(|id| (id.rsplit(':').next().unwrap_or(id), id.as_str())).collect();
        let listed: std::collections::HashSet<&str> = m.groups.iter().flat_map(|g| g.blocks.iter().map(String::as_str)).collect();
        let mut shapes: HashMap<&str, Vec<u32>> = HashMap::new();
        let mut loose: Vec<u32> = Vec::new();
        for (i, b) in cat.blocks.iter().enumerate() {
            if !of_mod(b) || listed.contains(b.id.as_str()) {
                continue;
            }
            let path = b.id.rsplit(':').next().unwrap_or(&b.id);
            // The longest block of the tabs whose name starts its own.
            let base = path.char_indices().rev().filter(|&(_, c)| c == '_').find_map(|(at, _)| members.get(&path[..at]).copied());
            match base {
                Some(base) => shapes.entry(base).or_default().push(i as u32),
                None => loose.push(i as u32),
            }
        }
        let name = m.name.to_uppercase();
        let mut shown = std::collections::HashSet::new();
        for g in &m.groups {
            let mut blocks = Vec::new();
            for id in &g.blocks {
                let Some(&i) = index.get(id.as_str()) else { continue };
                let b = &cat.blocks[i as usize];
                let listed = shown.insert((i, true));
                if matches(b, Some(&m.name)) || all && shapes.get(id.as_str()).is_some_and(|s| s.iter().any(|&k| matches(&cat.blocks[k as usize], Some(&m.name)))) {
                    blocks.push(i);
                }
                if all && listed && let Some(s) = shapes.get(id.as_str()) {
                    blocks.extend(s.iter().copied().filter(|&k| matches(&cat.blocks[k as usize], Some(&m.name))));
                }
            }
            if !blocks.is_empty() {
                out.push(Section { title: format!("{name}  ·  {}  ·  {}", g.name.to_uppercase(), blocks.len()), blocks });
            }
        }
        if all {
            let blocks: Vec<u32> = loose.into_iter().filter(|&i| matches(&cat.blocks[i as usize], Some(&m.name))).collect();
            if !blocks.is_empty() {
                out.push(Section { title: format!("{name}  ·  {}  ·  {}", crate::i18n::tr_name(Category::Other.label()).to_uppercase(), blocks.len()), blocks });
            }
        }
    }
    out
}

/// The blocks of the mods the catalogue read, by id, with the names of
/// their properties when a list of them was read (empty otherwise): Smooth
/// terrain's partial blocks are found among them.
pub fn mod_block_ids(cat: Option<&Catalogue>) -> std::sync::Arc<HashMap<String, Vec<String>>> {
    let Some(cat) = cat else { return Default::default() };
    // As the mod's lists say, else as its block states name them.
    let props = |id: &str| cat.definitions.get(id).map(|d| d.properties.iter().map(|p| p.name.clone()).collect()).or_else(|| cat.state_properties.get(id).cloned()).unwrap_or_default();
    let mut ids: HashMap<String, Vec<String>> = cat.blocks.iter().filter(|b| massif_assets::namespace(&b.id) != "minecraft").map(|b| (b.id.clone(), props(&b.id))).collect();
    for (ns, names) in &cat.registered {
        if ns != "minecraft" {
            for n in names {
                ids.entry(n.clone()).or_insert_with(|| props(n));
            }
        }
    }
    for id in cat.definitions.keys().filter(|id| massif_assets::namespace(id) != "minecraft") {
        ids.entry(id.clone()).or_insert_with(|| props(id));
    }
    std::sync::Arc::new(ids)
}

/// Whether a block of the game exists, by id, in the newest version known.
pub fn vanilla_exists(id: &str) -> bool {
    static NEWEST: std::sync::OnceLock<massif_anvil::Target> = std::sync::OnceLock::new();
    id.strip_prefix("minecraft:").is_some_and(|short| NEWEST.get_or_init(|| massif_anvil::Target::releases().into_iter().next().unwrap_or_default()).has_block(short))
}

/// The blocks of the mods read, as [`mod_block_ids`] lists them, for the 3D
/// View's slabs: kept up to date by [`note_catalogue`].
/// Blocks of mods by id, with their properties' names.
type ModBlocks = std::sync::Arc<HashMap<String, Vec<String>>>;
static MOD_BLOCKS: std::sync::RwLock<Option<ModBlocks>> = std::sync::RwLock::new(None);
/// The catalogue they were listed from (its address; 0: none).
static NOTED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// List the mods' blocks of a catalogue newly read, once.
pub fn note_catalogue(cat: Option<&std::sync::Arc<Catalogue>>) {
    let at = cat.map_or(0, |c| std::sync::Arc::as_ptr(c) as usize);
    if NOTED.swap(at, std::sync::atomic::Ordering::Relaxed) != at
        && let Ok(mut m) = MOD_BLOCKS.write()
    {
        *m = Some(mod_block_ids(cat.map(|c| &**c)));
        if let Ok(mut f) = FAMILIES.write() {
            *f = Some(block_families(cat.map(|c| &**c)));
        }
    }
}

/// The blocks of the mods read (see [`note_catalogue`]).
pub fn mod_blocks() -> std::sync::Arc<HashMap<String, Vec<String>>> {
    MOD_BLOCKS.read().ok().and_then(|m| m.clone()).unwrap_or_default()
}

/// The shapes the mods' recipes link to full blocks, of a catalogue (see
/// [`massif_core::block_families`]): the names its blocks are given (a mod
/// renames some of the game's) say which links are a block's own.
pub fn block_families(cat: Option<&Catalogue>) -> std::sync::Arc<massif_core::block_families::BlockFamilies> {
    let Some(cat) = cat.filter(|c| !c.stonecutting.is_empty()) else { return Default::default() };
    let names: HashMap<&str, &str> = cat.blocks.iter().map(|b| (b.id.as_str(), b.name.as_str())).collect();
    let name_of = |id: &str| names.get(id).map(|n| (*n).to_owned());
    std::sync::Arc::new(massif_core::block_families::BlockFamilies::from_recipes(cat.stonecutting.iter().map(|(a, b)| (a.as_str(), b.as_str())), &name_of))
}

static FAMILIES: std::sync::RwLock<Option<std::sync::Arc<massif_core::block_families::BlockFamilies>>> = std::sync::RwLock::new(None);

/// Replace the shapes the mods' recipes link (renders of before and
/// after).
#[cfg(test)]
pub fn set_families(f: std::sync::Arc<massif_core::block_families::BlockFamilies>) {
    if let Ok(mut m) = FAMILIES.write() {
        *m = Some(f);
    }
}

/// The shapes the recipes of the mods read link to full blocks (see
/// [`note_catalogue`]).
pub fn families() -> std::sync::Arc<massif_core::block_families::BlockFamilies> {
    FAMILIES.read().ok().and_then(|m| m.clone()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Jack's case, on a PC: no game where the official launcher keeps it,
    /// a Fabric instance of the Modrinth App that plays 1.20.1, whose game
    /// the app keeps under the loader's name. Use on the instance finds
    /// that jar and reads the mod's blocks over it, with no error.
    #[test]
    fn use_finds_the_instances_own_game() {
        let zip = |files: &[(&str, &[u8])]| {
            let parts: Vec<crate::report::Part> = files.iter().map(|(name, bytes)| crate::report::Part { name: (*name).to_owned(), about: "", bytes: bytes.to_vec(), text: false }).collect();
            let mut out = Vec::new();
            crate::report::zip(&mut out, &parts).unwrap();
            out
        };
        let root = std::env::temp_dir().join(format!("massif-use-instance-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let app = root.join("AppData/Roaming/ModrinthApp");
        let jar = app.join("meta/versions/1.20.1-fabric-0.16.10/1.20.1-fabric-0.16.10.jar");
        std::fs::create_dir_all(jar.parent().unwrap()).unwrap();
        std::fs::write(&jar, zip(&[("version.json", br#"{"id":"1.20.1","world_version":3465}"#), ("assets/minecraft/blockstates/stone.json", br#"{"variants":{"":{"model":"minecraft:block/stone"}}}"#)])).unwrap();
        let profile = app.join("profiles/ArdaCraft");
        std::fs::create_dir_all(profile.join("mods")).unwrap();
        std::fs::write(profile.join("profile.json"), br#"{"game_version":"1.20.1","loader":"fabric"}"#).unwrap();
        std::fs::write(profile.join("mods/arda.jar"), zip(&[("fabric.mod.json", br#"{"id":"arda","name":"Arda"}"#), ("assets/arda/blockstates/mallorn_log.json", br#"{"variants":{"":{"model":"arda:block/mallorn_log"}}}"#)])).unwrap();
        let instance = massif_assets::instance::Instance::read(&profile).expect("an instance");
        assert_eq!(instance.minecraft.as_deref(), Some("1.20.1"));
        let ctx = egui::Context::default();
        let mut lib = BlockLibrary::default();
        let (mut chosen, mut mods) = (None, Vec::new());
        lib.use_instance(&ctx, &instance, &mut chosen, &mut mods);
        assert_eq!(chosen.as_deref(), Some(jar.as_path()), "its own game's jar: {:?}", lib.notice);
        let started = std::time::Instant::now();
        while lib.is_loading() {
            lib.poll();
            assert!(started.elapsed().as_secs() < 60);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(lib.error.is_none(), "{:?}", lib.error);
        let cat = lib.catalogue.as_ref().expect("read");
        assert_eq!(cat.version, "1.20.1");
        assert!(cat.block("arda:mallorn_log").is_some() && cat.block("minecraft:stone").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }
}
