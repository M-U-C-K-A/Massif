//! WorldPainter's lists of a mod's blocks: the "custom materials" a
//! WorldPainter user drops into its `materials` folder, one line per block
//! with every property it has and what kind of block it is.
//!
//! Conquest Reforged writes one of its own, `conquest_wp.csv`, from the
//! game's registry, and beside it the list of every block the game
//! registers (`blocks-pls.txt`). Massif reads both: the properties say
//! which states of a block are real, so that a map is written with the
//! game's own states, and the list of the registry tells the block files
//! of a jar the game never registers (models left in the jar) from its
//! real blocks.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// One property of a block and the values it takes, in the game's order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Property {
    pub name: String,
    pub values: Vec<String>,
}

impl Property {
    /// From WorldPainter's descriptor: `waterlogged:b`, `layers:i[1-8]`,
    /// `facing:e[north;south;west;east]`.
    pub fn parse(descriptor: &str) -> Option<Self> {
        let (name, kind) = descriptor.trim().split_once(':')?;
        let values = if kind == "b" {
            vec!["true".to_owned(), "false".to_owned()]
        } else if let Some(range) = kind.strip_prefix("i[").and_then(|r| r.strip_suffix(']')) {
            let (lo, hi) = range.split_once('-')?;
            let (lo, hi): (i32, i32) = (lo.trim().parse().ok()?, hi.trim().parse().ok()?);
            if hi < lo || hi - lo > 1024 {
                return None;
            }
            (lo..=hi).map(|v| v.to_string()).collect()
        } else {
            let list = kind.strip_prefix("e[").and_then(|r| r.strip_suffix(']'))?;
            list.split(';').map(|v| v.trim().to_owned()).filter(|v| !v.is_empty()).collect()
        };
        (!name.is_empty() && !values.is_empty()).then(|| Property { name: name.to_owned(), values })
    }
}

/// What a list says of one block.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Definition {
    /// Every property of the block, by name.
    pub properties: Vec<Property>,
    /// How much light it stops, 0 to 15.
    pub opacity: u8,
    /// Walked through: plants, flowers, vines.
    pub insubstantial: bool,
    pub vegetation: bool,
    /// Logs, leaves and the like.
    pub tree: bool,
    /// The light it gives, 0 to 15.
    pub light: u8,
    /// Its colour on the game's maps.
    pub map_color: Option<[u8; 3]>,
}

impl Definition {
    pub fn property(&self, name: &str) -> Option<&Property> {
        self.properties.iter().find(|p| p.name == name)
    }

    /// Whether the list makes it a whole, solid block: one with no
    /// property that stops all light and is not walked through.
    pub fn plain_cube(&self) -> bool {
        self.properties.is_empty() && self.opacity >= 15 && !self.insubstantial && !self.vegetation
    }
}

/// A list of blocks read from a file.
#[derive(Debug, Clone, Default)]
pub struct List {
    /// The blocks and what the list says of them, in its order.
    pub blocks: Vec<(String, Definition)>,
    /// Every block the game registers, from the registry's own list beside
    /// the file, when there is one: of its namespaces, the blocks missing
    /// from it are none of the game's.
    pub registered: Option<HashSet<String>>,
}

impl List {
    /// The namespaces of its blocks, sorted.
    pub fn namespaces(&self) -> Vec<String> {
        let mut out: Vec<String> = self.blocks.iter().map(|(id, _)| crate::namespace(id).to_owned()).collect();
        out.sort();
        out.dedup();
        out
    }
}

/// Whether a file is a list of blocks to read as one (by its extension).
pub fn is_list(path: &Path) -> bool {
    path.extension().is_some_and(|e| e.eq_ignore_ascii_case("csv"))
}

/// The registry's list Conquest Reforged writes beside its WorldPainter
/// list.
pub const REGISTRY_LIST: &str = "blocks-pls.txt";

/// Read a WorldPainter list of blocks (and the registry's list beside it).
pub fn read(path: &Path) -> Result<List, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let mut list = parse(&String::from_utf8_lossy(&bytes))?;
    if let Some(dir) = path.parent() {
        list.registered = read_registry(&dir.join(REGISTRY_LIST));
    }
    Ok(list)
}

/// The registry's list: one namespaced id per line.
pub fn read_registry(path: &Path) -> Option<HashSet<String>> {
    let text = std::fs::read_to_string(path).ok()?;
    let ids: HashSet<String> = text.lines().map(str::trim).filter(|l| l.contains(':') && !l.contains(' ')).map(str::to_owned).collect();
    (!ids.is_empty()).then_some(ids)
}

/// The rows of a CSV file: fields split on commas outside quotes, `""` a
/// quote inside them.
fn rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' => quoted = !quoted,
            ',' if !quoted => row.push(std::mem::take(&mut field)),
            '\r' if !quoted => {}
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                if row.iter().any(|f| !f.is_empty()) {
                    rows.push(std::mem::take(&mut row));
                } else {
                    row.clear();
                }
            }
            c => field.push(c),
        }
    }
    row.push(field);
    if row.iter().any(|f| !f.is_empty()) {
        rows.push(row);
    }
    rows
}

/// A colour of the list: Conquest writes the game's map colours as
/// decimal numbers (`7368816`, stone's grey), WorldPainter its own as eight
/// hexadecimal digits with the alpha first (`ffa85a32`).
fn color(text: &str) -> Option<[u8; 3]> {
    let text = text.trim();
    let rgb = if text.len() == 8 || text.chars().any(|c| c.is_ascii_alphabetic()) {
        u32::from_str_radix(text, 16).ok()? & 0xff_ffff
    } else {
        let v: u32 = text.parse().ok()?;
        if v > 0xff_ffff {
            return None;
        }
        v
    };
    // The game's "no colour" (air, glass, plants that let the map through).
    (rgb != 0).then_some([(rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8])
}

/// Read the text of a WorldPainter list of blocks.
pub fn parse(text: &str) -> Result<List, String> {
    let mut rows = rows(text.trim_start_matches('\u{feff}')).into_iter();
    let header = rows.next().ok_or("the file is empty")?;
    let column = |name: &str| header.iter().position(|h| h.trim() == name);
    let name_col = column("name").ok_or("this is not a WorldPainter list of blocks: it has no \"name\" column")?;
    let (props, opacity, insubstantial, vegetation, tree, light, colour) =
        (column("properties"), column("opacity"), column("insubstantial"), column("vegetation"), column("treeRelated"), column("blockLight"), column("colour"));
    let mut blocks = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        let get = |c: Option<usize>| c.and_then(|c| row.get(c)).map(|s| s.trim()).unwrap_or("");
        let name = get(Some(name_col));
        // A row with no namespace, or one given twice, is not read.
        if !name.contains(':') || !seen.insert(name.to_owned()) {
            continue;
        }
        let flag = |c| get(c).eq_ignore_ascii_case("true");
        let number = |c| get(c).parse::<u8>().ok().map(|v| v.min(15));
        let properties = get(props).split(',').filter(|p| !p.trim().is_empty()).filter_map(Property::parse).collect();
        blocks.push((
            name.to_owned(),
            Definition {
                properties,
                opacity: number(opacity).unwrap_or(15),
                insubstantial: flag(insubstantial),
                vegetation: flag(vegetation),
                tree: flag(tree),
                light: number(light).unwrap_or(0),
                map_color: color(get(colour)),
            },
        ));
    }
    if blocks.is_empty() {
        return Err("the list has no blocks".to_owned());
    }
    Ok(List { blocks, registered: None })
}

/// What is wrong with a block state, by what a list says of its block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateProblem {
    /// The game registers no such block.
    UnknownBlock(String),
    /// The block has no such property.
    UnknownProperty { block: String, property: String },
    /// The property takes other values.
    BadValue { block: String, property: String, value: String },
}

impl std::fmt::Display for StateProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StateProblem::UnknownBlock(b) => write!(f, "{b} is not a block of the game"),
            StateProblem::UnknownProperty { block, property } => write!(f, "{block} has no property {property}"),
            StateProblem::BadValue { block, property, value } => write!(f, "{block}: {property} cannot be {value}"),
        }
    }
}

/// Check a state (`conquest:old_bricks_slab[type=top,layers=4]`) against
/// the definition of its block. A property left out takes the game's
/// default, so a state may give fewer than all.
pub fn check(state: &str, definitions: &HashMap<String, Definition>) -> Result<(), StateProblem> {
    let (name, props) = match state.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (state, ""),
    };
    let Some(def) = definitions.get(name) else { return Err(StateProblem::UnknownBlock(name.to_owned())) };
    for (k, v) in props.split(',').filter_map(|kv| kv.split_once('=')) {
        let (k, v) = (k.trim(), v.trim());
        match def.property(k) {
            None => return Err(StateProblem::UnknownProperty { block: name.to_owned(), property: k.to_owned() }),
            Some(p) if !p.values.iter().any(|x| x == v) => return Err(StateProblem::BadValue { block: name.to_owned(), property: k.to_owned(), value: v.to_owned() }),
            Some(_) => {}
        }
    }
    Ok(())
}

/// The folders where WorldPainter looks for lists of blocks, on this
/// computer: its `materials` folder, for each of its versions.
pub fn worldpainter_folders() -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut roots = Vec::new();
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join("Library/Application Support"));
        // `~/.worldpainter` on Linux.
        roots.push(home);
    }
    if let Some(appdata) = std::env::var_os("APPDATA") {
        roots.push(PathBuf::from(appdata));
    }
    for root in roots {
        let Ok(read) = std::fs::read_dir(&root) else { continue };
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if name.trim_start_matches('.').starts_with("worldpainter") {
                let dir = entry.path().join("materials");
                if dir.is_dir() {
                    out.push(dir);
                }
            }
        }
    }
    out
}

/// The lists of blocks in a folder.
pub fn lists_in(dir: &Path) -> Vec<PathBuf> {
    let Ok(read) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut out: Vec<PathBuf> = read
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_list(p) && p.is_file() && !p.file_name().is_some_and(|n| n.to_string_lossy().starts_with("._")))
        .collect();
    out.sort();
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Lines as Conquest Reforged writes them (`conquest_wp.csv`).
    pub(crate) const CONQUEST: &str = "name,discriminator,properties,opacity,receivesLight,insubstantial,resource,tileEntity,tileEntityId,treeRelated,vegetation,blockLight,natural,watery,colour,horizontal_orientation_schemes,vertical_orientation_scheme
conquest:worn_dark_lime_mortar_masonry,,,15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:worn_dark_lime_mortar_masonry_slab,,\"layers:i[1-8],type:e[top;bottom],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:worn_dark_lime_mortar_masonry_small_arch,,\"direction:e[northsouth;eastwest],waterlogged:b\",15,false,false,false,false,false,false,false,0,true,false,7368816,,
conquest:dead_bush_tall,,\"half:e[upper;lower]\",0,false,true,false,false,false,false,true,0,true,false,31744,,
conquest:candle_lantern,,,0,false,false,false,false,false,false,false,14,true,false,0,,
";

    #[test]
    fn conquest_lists_read() {
        let list = parse(CONQUEST).unwrap();
        assert_eq!(list.blocks.len(), 5);
        let (id, masonry) = &list.blocks[0];
        assert_eq!(id, "conquest:worn_dark_lime_mortar_masonry");
        assert!(masonry.plain_cube());
        assert_eq!(masonry.map_color, Some([0x70, 0x70, 0x70]), "decimal map colours");
        let slab = &list.blocks[1].1;
        assert!(!slab.plain_cube());
        assert_eq!(slab.property("layers").unwrap().values, (1..=8).map(|v| v.to_string()).collect::<Vec<_>>());
        assert_eq!(slab.property("type").unwrap().values, ["top", "bottom"]);
        assert_eq!(slab.property("waterlogged").unwrap().values, ["true", "false"]);
        let bush = &list.blocks[3].1;
        assert!(bush.vegetation && bush.insubstantial && bush.opacity == 0);
        assert_eq!(bush.map_color, Some([0, 0x7c, 0]));
        assert_eq!(list.blocks[4].1.light, 14);
        assert_eq!(list.blocks[4].1.map_color, None, "colour 0 is none");
        assert_eq!(list.namespaces(), ["conquest"]);

        let defs: HashMap<String, Definition> = list.blocks.into_iter().collect();
        assert_eq!(check("conquest:worn_dark_lime_mortar_masonry", &defs), Ok(()));
        assert_eq!(check("conquest:worn_dark_lime_mortar_masonry_slab[layers=4,type=top]", &defs), Ok(()));
        assert!(matches!(check("conquest:worn_dark_lime_mortar_masonry_slab[layers=9]", &defs), Err(StateProblem::BadValue { .. })));
        assert!(matches!(check("conquest:worn_dark_lime_mortar_masonry_slab[facing=north]", &defs), Err(StateProblem::UnknownProperty { .. })));
        assert!(matches!(check("conquest:cheese_cut", &defs), Err(StateProblem::UnknownBlock(_))));
    }

    #[test]
    fn worldpainter_lists_read() {
        // WorldPainter's own colours are ARGB in hexadecimal.
        let text = "name,discriminator,properties,opacity,colour\nminecraft:acacia_button,,\"facing:e[north;south;west;east],powered:b\",0,ffa85a32\n";
        let list = parse(text).unwrap();
        assert_eq!(list.blocks[0].1.map_color, Some([0xa8, 0x5a, 0x32]));
        assert_eq!(list.blocks[0].1.properties.len(), 2);
        assert!(parse("id,colour\nconquest:x,1\n").is_err(), "no name column");
        assert!(parse("").is_err());
    }
}
