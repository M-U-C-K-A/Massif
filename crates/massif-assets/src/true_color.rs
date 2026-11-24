//! The colours the game gives its blocks, shared by every view that shows a
//! map as it will look in the game (the 2D map's True Colour, the 3D views,
//! the Diorama, the Render side).
//!
//! A block's top shows its texture; some textures are grey and the game
//! tints them by the biome they stand in: grass, leaves, vines and water.
//! The tint comes from a colour map read at the biome's temperature and
//! downfall (`colormap/grass.png`, `colormap/foliage.png`), unless the
//! biome names its own colour. Resource packs replace these maps, and
//! packs made for OptiFine (or MCPatcher before it) add maps of their own
//! for any block (`optifine/colormap/*.properties` and its png), read by
//! temperature and downfall (`format=vanilla`), by biome and height
//! (`format=grid`) or as one colour (`format=fixed`).
//!
//! [`Rules`] holds what the game and the packs say, in the packs' order (the
//! last wins, as in the game); [`Rules::tint`] gives the tint of a block in
//! a biome at a height.

use crate::zip;
use std::collections::HashMap;

/// A colour map: 256 × 256 pixels (or a grid, see [`Format`]), RGB.
#[derive(Debug, Clone, PartialEq)]
pub struct Map {
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<[u8; 3]>,
}

impl Map {
    pub fn from_png(bytes: &[u8]) -> Option<Map> {
        let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?.to_rgba8();
        let (width, height) = img.dimensions();
        (width > 0 && height > 0).then(|| Map { width, height, rgb: img.pixels().map(|p| [p[0], p[1], p[2]]).collect() })
    }

    fn at(&self, x: u32, y: u32) -> [u8; 3] {
        self.rgb[(y.min(self.height - 1) * self.width + x.min(self.width - 1)) as usize]
    }

    /// The colour at a temperature and a downfall, as the game reads its
    /// maps: the downfall is scaled by the temperature, so that the map's
    /// lower left half is used.
    pub fn climate(&self, temperature: f32, downfall: f32) -> [u8; 3] {
        let t = temperature.clamp(0.0, 1.0);
        let d = downfall.clamp(0.0, 1.0) * t;
        let x = ((1.0 - t) * (self.width - 1) as f32) as u32;
        let y = ((1.0 - d) * (self.height - 1) as f32) as u32;
        self.at(x, y)
    }
}

/// How a custom colour map is read.
#[derive(Debug, Clone, PartialEq)]
pub enum Format {
    /// By temperature and downfall, as the game's own maps.
    Vanilla,
    /// One column a biome (by its number, see [`grid_column`]), one row a
    /// block of height from `y_offset`.
    Grid { y_offset: i32 },
    /// One colour everywhere.
    Fixed([u8; 3]),
}

/// A colour map a resource pack gives some blocks.
#[derive(Debug, Clone, PartialEq)]
pub struct Custom {
    pub format: Format,
    pub map: Option<Map>,
    /// Where it was read: `optifine/colormap/custom/leaves.properties`.
    pub source: String,
}

impl Custom {
    fn color(&self, biome: &Climate, y: i32) -> Option<[u8; 3]> {
        match (&self.format, &self.map) {
            (Format::Fixed(c), _) => Some(*c),
            (Format::Vanilla, Some(m)) => Some(m.climate(biome.temperature, biome.downfall)),
            (Format::Grid { y_offset }, Some(m)) => {
                let col = grid_column_any(&biome.id).unwrap_or(1);
                Some(m.at(col.min(m.width - 1), (y - y_offset).clamp(0, m.height as i32 - 1) as u32))
            }
            _ => None,
        }
    }
}

/// What the game knows of a biome to colour it.
#[derive(Debug, Clone, PartialEq)]
pub struct Climate {
    /// `minecraft:plains`.
    pub id: String,
    pub temperature: f32,
    pub downfall: f32,
    /// The colours the biome names itself, before the maps' (`grass_color`,
    /// `foliage_color`, `dry_foliage_color`; `water_color` is always named,
    /// the game's own water by default).
    pub grass: Option<[u8; 3]>,
    pub foliage: Option<[u8; 3]>,
    /// `dry_foliage_color` (leaf litter, since 1.21.5).
    pub dry_foliage: Option<[u8; 3]>,
    pub water: [u8; 3],
    /// `grass_color_modifier`: `swamp`, `dark_forest`, or none.
    pub modifier: Modifier,
}

impl Climate {
    /// Plains: what a block is tinted with when nothing says where it is.
    pub fn plains() -> Climate {
        Climate { id: "minecraft:plains".into(), temperature: 0.8, downfall: 0.4, grass: None, foliage: None, dry_foliage: None, water: WATER, modifier: Modifier::None }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Modifier {
    #[default]
    None,
    Swamp,
    DarkForest,
}

/// The game's water, in a biome that names no other.
pub const WATER: [u8; 3] = [0x3f, 0x76, 0xe4];
/// What the game gives spruce and birch leaves wherever they are.
pub const SPRUCE: [u8; 3] = [0x61, 0x99, 0x61];
pub const BIRCH: [u8; 3] = [0x80, 0xa7, 0x55];

/// Which of the game's tints a block takes on its tinted faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Grass,
    Foliage,
    /// Leaf litter (1.21.5): the biome's dry foliage.
    DryFoliage,
    Water,
    Spruce,
    Birch,
    /// A colour of its own whatever the biome (lily pads, stems…).
    Fixed([u8; 3]),
}

/// What the game and the resource packs say of the colours of blocks.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Rules {
    pub grass: Option<Map>,
    pub foliage: Option<Map>,
    pub dry_foliage: Option<Map>,
    /// OptiFine's maps for the game's own tints (`optifine/colormap/water.png`,
    /// `pine.png`, `birch.png`, `swampgrass.png`, `swampfoliage.png`), by name.
    pub special: HashMap<String, Custom>,
    /// OptiFine's maps for blocks, by block id (`minecraft:oak_leaves`).
    pub blocks: HashMap<String, Custom>,
    /// The packs whose colours were read, in order.
    pub packs: Vec<String>,
}

/// A temperature and downfall's colour in a map, or `default`.
fn climate_of(map: Option<&Map>, c: &Climate, default: [u8; 3]) -> [u8; 3] {
    map.map_or(default, |m| m.climate(c.temperature, c.downfall))
}

impl Rules {
    /// The colour of grass in a biome.
    pub fn grass(&self, c: &Climate, y: i32) -> [u8; 3] {
        if c.modifier == Modifier::Swamp
            && let Some(v) = self.special.get("swampgrass").and_then(|m| m.color(c, y))
        {
            return v;
        }
        let base = c.grass.unwrap_or_else(|| climate_of(self.grass.as_ref(), c, GRASS));
        match c.modifier {
            Modifier::Swamp => [0x6a, 0x70, 0x39],
            Modifier::DarkForest => {
                let v = (u32::from_be_bytes([0, base[0], base[1], base[2]]) & 0xfefefe) + 0x28340a;
                let v = v >> 1;
                [(v >> 16) as u8, (v >> 8) as u8, v as u8]
            }
            Modifier::None => base,
        }
    }

    /// The colour of leaves in a biome.
    pub fn foliage(&self, c: &Climate, y: i32) -> [u8; 3] {
        if c.modifier == Modifier::Swamp
            && let Some(v) = self.special.get("swampfoliage").and_then(|m| m.color(c, y))
        {
            return v;
        }
        c.foliage.unwrap_or_else(|| climate_of(self.foliage.as_ref(), c, FOLIAGE))
    }

    pub fn dry_foliage(&self, c: &Climate) -> [u8; 3] {
        c.dry_foliage.unwrap_or_else(|| climate_of(self.dry_foliage.as_ref(), c, [0xa0, 0x80, 0x50]))
    }

    pub fn water(&self, c: &Climate, y: i32) -> [u8; 3] {
        // A pack's map of water replaces the biome's colour only for the
        // game's own blue, as OptiFine does.
        match self.special.get("water").and_then(|m| m.color(c, y)) {
            Some(v) if c.water == WATER => v,
            _ => c.water,
        }
    }

    /// The tint of a block's tinted faces in a biome at a height: a pack's
    /// map for that block first, then the game's tint of its kind; `None`
    /// for a block the game does not tint.
    pub fn tint(&self, block: &str, kind: Option<Kind>, c: &Climate, y: i32) -> Option<[u8; 3]> {
        if let Some(v) = self.blocks.get(block).and_then(|m| m.color(c, y)) {
            return Some(v);
        }
        Some(match kind? {
            Kind::Grass => self.grass(c, y),
            Kind::Foliage => self.foliage(c, y),
            Kind::DryFoliage => self.dry_foliage(c),
            Kind::Water => self.water(c, y),
            Kind::Spruce => self.special.get("pine").and_then(|m| m.color(c, y)).unwrap_or(SPRUCE),
            Kind::Birch => self.special.get("birch").and_then(|m| m.color(c, y)).unwrap_or(BIRCH),
            Kind::Fixed(v) => v,
        })
    }

    /// Whether a block's colour depends on the height it stands at (a
    /// pack's map by grid for it): then it cannot be cached by biome.
    pub fn by_height(&self, block: &str, kind: Option<Kind>) -> bool {
        let grid = |m: Option<&Custom>| m.is_some_and(|m| matches!(m.format, Format::Grid { .. }));
        grid(self.blocks.get(block))
            || match kind {
                Some(Kind::Spruce) => grid(self.special.get("pine")),
                Some(Kind::Birch) => grid(self.special.get("birch")),
                Some(Kind::Water) => grid(self.special.get("water")),
                Some(Kind::Grass) => grid(self.special.get("swampgrass")),
                Some(Kind::Foliage) => grid(self.special.get("swampfoliage")),
                _ => false,
            }
    }

    /// Read what one archive says (the game's jar first, then each resource
    /// pack or mod in order): its maps replace those read before.
    pub fn read_archive(&mut self, archive: &zip::Archive, name: &str) {
        let files: Vec<(String, Vec<u8>)> = archive
            .names()
            .filter(|n| wanted(n))
            .filter_map(|n| Some((n.to_owned(), archive.read(n)?.ok()?)))
            .collect();
        self.read_files(&files, name);
    }

    /// Read the files of a pack, by their names in it (`assets/…`).
    pub fn read_files(&mut self, files: &[(String, Vec<u8>)], name: &str) {
        let get = |n: &str| files.iter().find(|(f, _)| f == n).map(|(_, b)| b.as_slice());
        let mut any = false;
        for (slot, file) in [(&mut self.grass, "grass"), (&mut self.foliage, "foliage"), (&mut self.dry_foliage, "dry_foliage")] {
            if let Some(m) = get(&format!("assets/minecraft/textures/colormap/{file}.png")).and_then(Map::from_png) {
                *slot = Some(m);
                any = true;
            }
        }
        // OptiFine's palette.format (color.properties) is the format of
        // maps that do not say theirs.
        let mut default_format = "vanilla".to_owned();
        let mut palettes: Vec<(String, String)> = Vec::new();
        for root in ROOTS {
            if let Some(text) = get(&format!("assets/minecraft/{root}/color.properties")).map(String::from_utf8_lossy) {
                for (k, v) in properties(&text) {
                    if k == "palette.format" {
                        default_format = v;
                    } else if let Some(png) = k.strip_prefix("palette.block.") {
                        palettes.push((resolve(png, &format!("assets/minecraft/{root}/"), root), v));
                    }
                }
            }
        }
        for root in ROOTS {
            let dir = format!("assets/minecraft/{root}/colormap/");
            for special in ["water", "pine", "birch", "swampgrass", "swampfoliage"] {
                let png = format!("{dir}{special}.png");
                let props = get(&format!("{dir}{special}.properties")).map(|b| properties(&String::from_utf8_lossy(b))).unwrap_or_default();
                if let Some(map) = get(&png).and_then(Map::from_png) {
                    self.special.insert(special.to_owned(), Custom { format: format_of(&props, &default_format, special == "water" && map.width != 256), map: Some(map), source: png });
                    any = true;
                }
            }
            // Maps of their own for blocks, anywhere under colormap/.
            for (file, bytes) in files.iter().filter(|(f, _)| f.starts_with(&dir) && f.ends_with(".properties")) {
                let props = properties(&String::from_utf8_lossy(bytes));
                let stem = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".properties");
                if ["water", "pine", "birch", "swampgrass", "swampfoliage"].contains(&stem) && file.matches('/').count() == dir.matches('/').count() {
                    continue;
                }
                let folder = &file[..file.rfind('/').map_or(0, |i| i + 1)];
                let source = props.iter().find(|(k, _)| k == "source").map_or_else(|| format!("{folder}{stem}.png"), |(_, v)| resolve(v, folder, root));
                let map = get(&source).and_then(Map::from_png);
                let format = format_of(&props, &default_format, false);
                if map.is_none() && !matches!(format, Format::Fixed(_)) {
                    continue;
                }
                let blocks = props.iter().find(|(k, _)| k == "blocks").map_or_else(|| stem.to_owned(), |(_, v)| v.clone());
                let custom = Custom { format, map, source: file.clone() };
                for b in block_ids(&blocks) {
                    self.blocks.insert(b, custom.clone());
                }
                any = true;
            }
        }
        for (png, blocks) in palettes {
            if let Some(map) = get(&png).and_then(Map::from_png) {
                let custom = Custom { format: format_of(&[], &default_format, false), map: Some(map), source: png };
                for b in block_ids(&blocks) {
                    self.blocks.insert(b, custom.clone());
                }
                any = true;
            }
        }
        if any && !name.is_empty() {
            self.packs.push(name.to_owned());
        }
    }
}

const GRASS: [u8; 3] = [0x91, 0xbd, 0x59];
const FOLIAGE: [u8; 3] = [0x77, 0xab, 0x2f];

/// Where OptiFine and MCPatcher keep their files.
const ROOTS: [&str; 2] = ["optifine", "mcpatcher"];

/// Whether a file of an archive says something of colours.
pub fn wanted(name: &str) -> bool {
    name.starts_with("assets/minecraft/textures/colormap/") && name.ends_with(".png")
        || ROOTS.iter().any(|r| {
            let p = format!("assets/minecraft/{r}/");
            name.starts_with(&p) && (name.contains("/colormap/") && (name.ends_with(".png") || name.ends_with(".properties")) || name == format!("{p}color.properties"))
        })
}

/// A path in a properties file: `./x.png` beside it, `~/x.png` from the
/// root of OptiFine's folder, `minecraft:optifine/x.png` or `x.png` beside it.
fn resolve(path: &str, folder: &str, root: &str) -> String {
    let path = path.trim();
    let png = if path.ends_with(".png") { path.to_owned() } else { format!("{path}.png") };
    if let Some(rest) = png.strip_prefix("./") {
        format!("{folder}{rest}")
    } else if let Some(rest) = png.strip_prefix("~/") {
        format!("assets/minecraft/{root}/{rest}")
    } else if let Some((ns, rest)) = png.split_once(':') {
        format!("assets/{ns}/{rest}")
    } else if png.starts_with("assets/") {
        png
    } else if png.contains('/') {
        format!("assets/minecraft/{png}")
    } else {
        format!("{folder}{png}")
    }
}

fn format_of(props: &[(String, String)], default: &str, grid_water: bool) -> Format {
    let get = |k: &str| props.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
    match get("format").unwrap_or(if grid_water { "grid" } else { default }) {
        "fixed" => Format::Fixed(get("color").and_then(|c| u32::from_str_radix(c.trim_start_matches('#'), 16).ok()).map_or([255; 3], |v| [(v >> 16) as u8, (v >> 8) as u8, v as u8])),
        "grid" => Format::Grid { y_offset: get("yOffset").and_then(|v| v.parse().ok()).unwrap_or(0) },
        _ => Format::Vanilla,
    }
}

/// `key=value` lines, `#` comments, `\` line continuations.
pub fn properties(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut line = String::new();
    for raw in text.lines() {
        let raw = raw.trim();
        if line.is_empty() && (raw.starts_with('#') || raw.starts_with('!')) {
            continue;
        }
        if let Some(cont) = raw.strip_suffix('\\') {
            line.push_str(cont);
            line.push(' ');
            continue;
        }
        line.push_str(raw);
        if let Some((k, v)) = line.split_once(['=', ':']).filter(|(k, _)| !k.trim().is_empty()) {
            out.push((k.trim().to_owned(), v.trim().to_owned()));
        }
        line.clear();
    }
    out
}

/// The blocks a `blocks=` list names: ids with or without their namespace,
/// their states left out (`oak_leaves:persistent=true` is `oak_leaves`).
fn block_ids(list: &str) -> Vec<String> {
    list.split_whitespace()
        .filter_map(|b| {
            let parts: Vec<&str> = b.split(':').collect();
            // `ns:name`, `name`, `name:prop=v`, `ns:name:prop=v`.
            let (ns, name) = match parts.as_slice() {
                [n] => ("minecraft", *n),
                [a, b, ..] if !b.contains('=') => (*a, *b),
                [n, ..] => ("minecraft", *n),
                [] => return None,
            };
            (!name.is_empty() && !name.contains('=')).then(|| format!("{ns}:{name}"))
        })
        .collect()
}

/// The column of a biome in a map by grid: OptiFine's biome numbers, the
/// game's old numeric ids, by today's names and those of before 1.18.
pub fn grid_column(id: &str) -> Option<u32> {
    let n = id.strip_prefix("minecraft:")?;
    GRID.iter().find(|(names, _)| names.split('|').any(|x| x == n)).map(|&(_, k)| k)
}

const GRID: [(&str, u32); 83] = [
    ("ocean", 0),
    ("plains", 1),
    ("desert", 2),
    ("windswept_hills|mountains|extreme_hills", 3),
    ("forest", 4),
    ("taiga", 5),
    ("swamp|swampland", 6),
    ("river", 7),
    ("nether_wastes|nether|hell", 8),
    ("the_end|sky", 9),
    ("frozen_ocean", 10),
    ("frozen_river", 11),
    ("snowy_plains|snowy_tundra|ice_plains", 12),
    ("snowy_mountains|ice_mountains", 13),
    ("mushroom_fields|mushroom_island", 14),
    ("mushroom_field_shore", 15),
    ("beach|beaches", 16),
    ("desert_hills", 17),
    ("wooded_hills|forest_hills", 18),
    ("taiga_hills", 19),
    ("mountain_edge", 20),
    ("jungle", 21),
    ("jungle_hills", 22),
    ("sparse_jungle|jungle_edge", 23),
    ("deep_ocean", 24),
    ("stony_shore|stone_shore|stone_beach", 25),
    ("snowy_beach|cold_beach", 26),
    ("birch_forest", 27),
    ("birch_forest_hills", 28),
    ("dark_forest|roofed_forest", 29),
    ("snowy_taiga|taiga_cold", 30),
    ("snowy_taiga_hills", 31),
    ("old_growth_pine_taiga|giant_tree_taiga|redwood_taiga", 32),
    ("giant_tree_taiga_hills", 33),
    ("windswept_forest|wooded_mountains", 34),
    ("savanna", 35),
    ("savanna_plateau", 36),
    ("badlands|mesa", 37),
    ("wooded_badlands|wooded_badlands_plateau", 38),
    ("badlands_plateau", 39),
    ("small_end_islands", 40),
    ("end_midlands", 41),
    ("end_highlands", 42),
    ("end_barrens", 43),
    ("warm_ocean", 44),
    ("lukewarm_ocean", 45),
    ("cold_ocean", 46),
    ("deep_warm_ocean", 47),
    ("deep_lukewarm_ocean", 48),
    ("deep_cold_ocean", 49),
    ("deep_frozen_ocean", 50),
    ("the_void|void", 127),
    ("sunflower_plains", 129),
    ("desert_lakes", 130),
    ("windswept_gravelly_hills|gravelly_mountains", 131),
    ("flower_forest", 132),
    ("taiga_mountains", 133),
    ("swamp_hills", 134),
    ("ice_spikes", 140),
    ("modified_jungle", 149),
    ("modified_jungle_edge", 151),
    ("old_growth_birch_forest|tall_birch_forest", 155),
    ("tall_birch_hills", 156),
    ("dark_forest_hills", 157),
    ("snowy_taiga_mountains", 158),
    ("old_growth_spruce_taiga|giant_spruce_taiga", 160),
    ("giant_spruce_taiga_hills", 161),
    ("modified_gravelly_mountains", 162),
    ("windswept_savanna|shattered_savanna", 163),
    ("shattered_savanna_plateau", 164),
    ("eroded_badlands", 165),
    ("modified_wooded_badlands_plateau", 166),
    ("modified_badlands_plateau", 167),
    ("bamboo_jungle", 168),
    ("bamboo_jungle_hills", 169),
    ("soul_sand_valley", 170),
    ("crimson_forest", 171),
    ("warped_forest", 172),
    ("basalt_deltas", 173),
    ("dripstone_caves", 174),
    ("lush_caves", 175),
    ("meadow", 177),
    ("grove", 178),
];

/// Newer biomes after the old numbers ran out, in OptiFine's order.
const GRID_NEWER: [(&str, u32); 8] = [("snowy_slopes", 179), ("frozen_peaks", 180), ("jagged_peaks", 181), ("stony_peaks", 182), ("deep_dark", 183), ("mangrove_swamp", 184), ("cherry_grove", 185), ("pale_garden", 186)];

/// [`grid_column`] for the newest biomes too.
pub fn grid_column_any(id: &str) -> Option<u32> {
    grid_column(id).or_else(|| {
        let n = id.strip_prefix("minecraft:")?;
        GRID_NEWER.iter().find(|(x, _)| *x == n).map(|&(_, k)| k)
    })
}

/// The game's tint kind of a block, by its id (see `tint_of` for the
/// catalogue's): `None` for a block the game does not tint.
pub fn kind_of(id: &str, tinted: bool) -> Option<Kind> {
    let n = id.rsplit(':').next().unwrap_or(id);
    match n {
        "spruce_leaves" => Some(Kind::Spruce),
        "birch_leaves" => Some(Kind::Birch),
        "cherry_leaves" | "pale_oak_leaves" => None,
        "leaf_litter" => Some(Kind::DryFoliage),
        "lily_pad" => Some(Kind::Fixed([0x20, 0x80, 0x30])),
        _ if n == "water" || n == "bubble_column" => Some(Kind::Water),
        _ if !tinted => None,
        _ if n.contains("water") => Some(Kind::Water),
        _ if n.contains("leaves") || n == "vine" || n.ends_with("_vine") && !n.contains("cave") => Some(Kind::Foliage),
        _ if n.contains("stem") => Some(Kind::Fixed([0xe0, 0xc7, 0x1c])),
        "redstone_wire" => Some(Kind::Fixed([0xd0, 0x10, 0x10])),
        _ => Some(Kind::Grass),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The game's grass map, as near as a test can have it without the
    /// jar: the corners of the real one, blended (enough to check the
    /// reading of temperature and downfall).
    fn gradient() -> Map {
        let mut rgb = Vec::new();
        for y in 0..256u32 {
            for x in 0..256u32 {
                rgb.push([x as u8, y as u8, 7]);
            }
        }
        Map { width: 256, height: 256, rgb }
    }

    fn climate(id: &str, t: f32, d: f32) -> Climate {
        Climate { id: id.into(), temperature: t, downfall: d, grass: None, foliage: None, dry_foliage: None, water: WATER, modifier: Modifier::None }
    }

    #[test]
    fn maps_are_read_by_temperature_and_downfall_as_the_game_reads_them() {
        let m = gradient();
        // Plains (0.8, 0.4): x = 0.2 × 255, y = (1 − 0.32) × 255, cut
        // down as the game's (int) cast does (0.8f makes 0.19999999).
        assert_eq!(m.climate(0.8, 0.4), [50, 173, 7]);
        // Desert (2.0, 0.0): clamped hot, dry.
        assert_eq!(m.climate(2.0, 0.0), [0, 255, 7]);
        // Snowy plains (0.0, 0.5): cold: the downfall counts for nothing.
        assert_eq!(m.climate(0.0, 0.5), [255, 255, 7]);
        // Jungle (0.95, 0.9).
        assert_eq!(m.climate(0.95, 0.9), [12, 36, 7]);
    }

    #[test]
    fn the_games_biome_tints_without_a_pack() {
        let r = Rules::default();
        let plains = Climate::plains();
        assert_eq!(r.grass(&plains, 64), GRASS);
        assert_eq!(r.foliage(&plains, 64), FOLIAGE);
        // Badlands name their grass and foliage.
        let badlands = Climate { grass: Some([0x90, 0x81, 0x4d]), foliage: Some([0x9e, 0x81, 0x4d]), ..climate("minecraft:badlands", 2.0, 0.0) };
        assert_eq!(r.tint("minecraft:grass_block", Some(Kind::Grass), &badlands, 64), Some([0x90, 0x81, 0x4d]));
        assert_eq!(r.tint("minecraft:oak_leaves", Some(Kind::Foliage), &badlands, 64), Some([0x9e, 0x81, 0x4d]));
        // Swamps' grass, whatever the map; dark forests' mixed with dark green.
        let swamp = Climate { modifier: Modifier::Swamp, ..climate("minecraft:swamp", 0.8, 0.9) };
        assert_eq!(r.grass(&swamp, 64), [0x6a, 0x70, 0x39]);
        let dark = Climate { modifier: Modifier::DarkForest, ..climate("minecraft:dark_forest", 0.7, 0.8) };
        let base = GRASS;
        let v = ((u32::from_be_bytes([0, base[0], base[1], base[2]]) & 0xfefefe) + 0x28340a) >> 1;
        assert_eq!(r.grass(&dark, 64), [(v >> 16) as u8, (v >> 8) as u8, v as u8]);
        // Spruce and birch keep their own colours; water is the biome's.
        assert_eq!(r.tint("minecraft:spruce_leaves", kind_of("minecraft:spruce_leaves", true), &plains, 64), Some(SPRUCE));
        assert_eq!(r.tint("minecraft:birch_leaves", kind_of("minecraft:birch_leaves", true), &plains, 64), Some(BIRCH));
        let warm = Climate { water: [0x43, 0xd5, 0xee], ..climate("minecraft:warm_ocean", 0.5, 0.5) };
        assert_eq!(r.tint("minecraft:water", kind_of("minecraft:water", true), &warm, 62), Some([0x43, 0xd5, 0xee]));
        assert_eq!(r.tint("minecraft:stone", kind_of("minecraft:stone", false), &plains, 64), None);
    }

    #[test]
    fn a_packs_vanilla_maps_replace_the_games() {
        let mut r = Rules::default();
        let mut png = Vec::new();
        image::RgbaImage::from_pixel(256, 256, image::Rgba([200, 30, 40, 255])).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        r.read_files(&[("assets/minecraft/textures/colormap/grass.png".into(), png)], "Red Grass");
        assert_eq!(r.grass(&Climate::plains(), 64), [200, 30, 40]);
        assert_eq!(r.foliage(&Climate::plains(), 64), FOLIAGE, "only grass changed");
        assert_eq!(r.packs, ["Red Grass"]);
    }

    fn png(w: u32, h: u32, f: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
        let img = image::RgbaImage::from_fn(w, h, |x, y| {
            let c = f(x, y);
            image::Rgba([c[0], c[1], c[2], 255])
        });
        let mut out = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    /// A pack as OptiFine reads one: custom maps for blocks by grid and
    /// vanilla format, a fixed colour, special maps, palette entries.
    #[test]
    fn optifine_colormaps() {
        let files = vec![
            // By grid: column = biome number, row = height.
            ("assets/minecraft/optifine/colormap/custom/leaves.properties".to_owned(), b"# oak and jungle leaves\nformat=grid\nblocks=oak_leaves minecraft:jungle_leaves:persistent=true\nsource=./leaves_grid.png\nyOffset=0\n".to_vec()),
            ("assets/minecraft/optifine/colormap/custom/leaves_grid.png".to_owned(), png(256, 256, |x, y| [x as u8, y as u8, 99])),
            // A file named after its block, its png beside it.
            ("assets/minecraft/optifine/colormap/custom/vine.properties".to_owned(), b"format=vanilla\n".to_vec()),
            ("assets/minecraft/optifine/colormap/custom/vine.png".to_owned(), png(256, 256, |_, _| [1, 2, 3])),
            ("assets/minecraft/optifine/colormap/sugar.properties".to_owned(), b"format=fixed\nblocks=sugar_cane\ncolor=ff8000\n".to_vec()),
            ("assets/minecraft/optifine/colormap/pine.png".to_owned(), png(256, 256, |_, _| [10, 90, 20])),
            ("assets/minecraft/optifine/color.properties".to_owned(), b"palette.format=vanilla\npalette.block.~/colormap/ferns.png=fern large_fern \\\n  conquest:wild_fern\n".to_vec()),
            ("assets/minecraft/optifine/colormap/ferns.png".to_owned(), png(256, 256, |_, _| [40, 50, 60])),
        ];
        let mut r = Rules::default();
        r.read_files(&files, "Jack's Pack");
        let forest = climate("minecraft:forest", 0.7, 0.8);
        // forest is column 4.
        assert_eq!(r.tint("minecraft:oak_leaves", Some(Kind::Foliage), &forest, 70), Some([4, 70, 99]));
        assert_eq!(r.tint("minecraft:jungle_leaves", Some(Kind::Foliage), &forest, 100), Some([4, 100, 99]));
        assert!(r.by_height("minecraft:oak_leaves", Some(Kind::Foliage)));
        assert!(!r.by_height("minecraft:vine", Some(Kind::Foliage)));
        assert_eq!(r.tint("minecraft:vine", Some(Kind::Foliage), &forest, 70), Some([1, 2, 3]));
        assert_eq!(r.tint("minecraft:sugar_cane", Some(Kind::Grass), &forest, 70), Some([255, 128, 0]));
        assert_eq!(r.tint("minecraft:spruce_leaves", Some(Kind::Spruce), &forest, 70), Some([10, 90, 20]));
        assert_eq!(r.tint("minecraft:fern", Some(Kind::Grass), &forest, 70), Some([40, 50, 60]));
        assert_eq!(r.tint("conquest:wild_fern", Some(Kind::Grass), &forest, 70), Some([40, 50, 60]));
        // What the pack leaves alone keeps the game's colours.
        assert_eq!(r.tint("minecraft:grass_block", Some(Kind::Grass), &forest, 70), Some(Rules::default().grass(&forest, 70)));
        assert_eq!(r.packs, ["Jack's Pack"]);
        // Heights below the grid's first row read its first row.
        assert_eq!(r.tint("minecraft:oak_leaves", Some(Kind::Foliage), &forest, -40), Some([4, 0, 99]));
    }

    /// The game's own colour maps and biomes, read from the newest jar on
    /// this computer (skipped without one): the tints the game gives grass
    /// and leaves, as the wiki lists them.
    #[test]
    fn biome_tints_match_the_game() {
        let Some(jar) = crate::find_jars().into_iter().find(|j| crate::release_numbers(&j.version).is_some_and(|n| n >= vec![1, 19])) else {
            eprintln!("no game jar: skipped");
            return;
        };
        let cat = crate::load(&jar.path, &jar.version).unwrap();
        let check = |id: &str, grass: u32, foliage: u32| {
            let b = cat.biome(id).unwrap_or_else(|| panic!("{id}"));
            let hex = |c: [u8; 3]| (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32;
            assert_eq!(hex(cat.colors.grass(&b.climate, 64)), grass, "{id} grass {:06x}", hex(b.grass));
            assert_eq!(hex(cat.colors.foliage(&b.climate, 64)), foliage, "{id} foliage");
            assert_eq!(b.grass, cat.colors.grass(&b.climate, 64));
        };
        check("minecraft:plains", 0x91bd59, 0x77ab2f);
        check("minecraft:forest", 0x79c05a, 0x59ae30);
        check("minecraft:jungle", 0x59c93c, 0x30bb0b);
        check("minecraft:desert", 0xbfb755, 0xaea42a);
        check("minecraft:snowy_plains", 0x80b497, 0x60a17b);
        check("minecraft:swamp", 0x6a7039, 0x6a7039);
        check("minecraft:badlands", 0x90814d, 0x9e814d);
        // Water: warm oceans name theirs.
        assert_eq!(cat.biome("minecraft:warm_ocean").unwrap().water, [0x43, 0xd5, 0xee]);
        assert_eq!(cat.biome("minecraft:plains").unwrap().water, WATER);
        // Grass's top is grey before its tint.
        let (top, kind) = cat.true_top("minecraft:grass_block").unwrap();
        assert_eq!(kind, Some(Kind::Grass));
        assert!(top[0].abs_diff(top[1]) < 8 && top[1].abs_diff(top[2]) < 8, "{top:?}");
        assert_eq!(cat.true_top("minecraft:spruce_leaves").unwrap().1, Some(Kind::Spruce));
        assert_eq!(cat.true_top("minecraft:stone").unwrap().1, None);
    }

    #[test]
    fn old_and_new_biome_names_find_their_columns() {
        assert_eq!(grid_column("minecraft:snowy_plains"), Some(12));
        assert_eq!(grid_column("minecraft:snowy_tundra"), Some(12));
        assert_eq!(grid_column_any("minecraft:cherry_grove"), Some(185));
        assert_eq!(grid_column("biomesoplenty:bog"), None);
    }
}
