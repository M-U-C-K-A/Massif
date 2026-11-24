//! Connected textures: a face of a block shows a tile chosen by its
//! neighbours (glass without seams, paving with borders round its edge) or
//! by its place (a pattern repeated over several blocks, tiles picked at
//! random), as OptiFine's CTM (`optifine/ctm/**/*.properties`, read by
//! Continuity too) and Fusion (`"fusion"` in a texture's `.png.mcmeta`)
//! draw them.
//!
//! Every way of connecting is brought to a few: [`Kind::Compact`], five
//! tiles from which any face is pieced together a quarter at a time (a
//! quarter is alone, joined across, joined up and down, inside, or an
//! inner corner), as OptiFine's `ctm_compact` draws them; a full CTM of 47
//! tiles gives its five. A row (`horizontal`), a column (`vertical`), a
//! pattern repeated (`repeat`, Fusion's `continuous`) and tiles at random
//! (`random`) keep their own.

use crate::Texture;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum Kind {
    /// Alone, joined left and right, joined up and down, inside, inner
    /// corner: OptiFine's `ctm_compact` order.
    Compact([Arc<Texture>; 5]),
    /// Alone, left end (joined right), middle, right end (joined left).
    Horizontal([Arc<Texture>; 4]),
    /// Alone, bottom end (joined up), middle, top end (joined down).
    Vertical([Arc<Texture>; 4]),
    /// A pattern `width` × `height` blocks, row by row.
    Repeat { width: u32, height: u32, tiles: Vec<Arc<Texture>> },
    /// Tiles at random by place, with their weights.
    Random(Vec<(Arc<Texture>, u32)>),
}

/// Faces a connected texture applies to.
pub const UP: u8 = 1;
pub const DOWN: u8 = 2;
pub const SIDES: u8 = 4;
pub const ALL: u8 = UP | DOWN | SIDES;

#[derive(Debug, Clone, PartialEq)]
pub struct Connected {
    pub kind: Kind,
    /// [`UP`], [`DOWN`], [`SIDES`].
    pub faces: u8,
    /// Where it was read (`optifine/ctm/glass/glass.properties`).
    pub source: String,
}

/// What lies around a face, in the face's own frame (up is the face's top:
/// north for a top face): whether the block there joins it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Around {
    pub left: bool,
    pub right: bool,
    pub up: bool,
    pub down: bool,
    pub up_left: bool,
    pub up_right: bool,
    pub down_left: bool,
    pub down_right: bool,
}

impl Around {
    /// Joined on every side and corner.
    pub const INSIDE: Around = Around { left: true, right: true, up: true, down: true, up_left: true, up_right: true, down_left: true, down_right: true };
}

fn hash(x: i32, y: i32, z: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ (z as u32).wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^ (h >> 15)
}

/// The quarter of a compact tile a quarter of a face takes: 0 alone, 1
/// joined across, 2 joined up and down, 3 inside, 4 inner corner.
fn quarter(side: bool, vertical: bool, corner: bool) -> usize {
    match (side, vertical) {
        (false, false) => 0,
        (true, false) => 1,
        (false, true) => 2,
        (true, true) if corner => 3,
        (true, true) => 4,
    }
}

impl Connected {
    /// The tile a face shows when nothing joins it: what a block alone and
    /// the palette's picture show.
    pub fn alone(&self) -> Arc<Texture> {
        match &self.kind {
            Kind::Compact(t) => t[0].clone(),
            Kind::Horizontal(t) | Kind::Vertical(t) => t[0].clone(),
            Kind::Repeat { tiles, .. } => tiles[0].clone(),
            Kind::Random(t) => t[0].0.clone(),
        }
    }

    /// Whether the tile depends on the neighbours (else only on the place).
    pub fn joins(&self) -> bool {
        matches!(self.kind, Kind::Compact(_) | Kind::Horizontal(_) | Kind::Vertical(_))
    }

    /// The face of a block at `(x, y, z)` with `around` it, its texture in
    /// the face's frame at `(u, v)` (the block's place along the face's
    /// across and up, for patterns).
    pub fn face(&self, around: Around, u: i32, v: i32, at: (i32, i32, i32)) -> Arc<Texture> {
        match &self.kind {
            Kind::Compact(t) => {
                let q = [
                    quarter(around.left, around.up, around.up_left),
                    quarter(around.right, around.up, around.up_right),
                    quarter(around.left, around.down, around.down_left),
                    quarter(around.right, around.down, around.down_right),
                ];
                if q.iter().all(|&k| k == q[0]) && q[0] != 4 {
                    return t[q[0]].clone();
                }
                Arc::new(pieced(t, q))
            }
            Kind::Horizontal(t) => t[match (around.left, around.right) {
                (false, false) => 0,
                (false, true) => 1,
                (true, true) => 2,
                (true, false) => 3,
            }]
            .clone(),
            Kind::Vertical(t) => t[match (around.down, around.up) {
                (false, false) => 0,
                (false, true) => 1,
                (true, true) => 2,
                (true, false) => 3,
            }]
            .clone(),
            Kind::Repeat { width, height, tiles } => {
                let (w, h) = (*width as i32, *height as i32);
                let k = (v.rem_euclid(h) * w + u.rem_euclid(w)) as usize;
                tiles[k.min(tiles.len() - 1)].clone()
            }
            Kind::Random(t) => {
                let total: u32 = t.iter().map(|x| x.1.max(1)).sum();
                let mut r = hash(at.0, at.1, at.2) % total.max(1);
                for (tex, w) in t {
                    if r < (*w).max(1) {
                        return tex.clone();
                    }
                    r -= (*w).max(1);
                }
                t[0].0.clone()
            }
        }
    }
}

/// A face pieced from the quarters of compact tiles: `q` names the tile of
/// its top left, top right, bottom left and bottom right quarters. An
/// inner corner's quarter (4) is the inside one with the corner's own
/// quarter of the corner tile.
fn pieced(t: &[Arc<Texture>; 5], q: [usize; 4]) -> Texture {
    let (w, h) = (t[0].width, t[0].height);
    let mut rgba = vec![0u8; (w * h * 4) as usize];
    for y in 0..h {
        for x in 0..w {
            let k = (y >= h / 2) as usize * 2 + (x >= w / 2) as usize;
            let tile = &t[q[k]];
            // Tiles of another size are read at their own scale.
            let (sx, sy) = (x * tile.width / w, y * tile.height / h);
            let from = ((sy * tile.width + sx) * 4) as usize;
            let to = ((y * w + x) * 4) as usize;
            if let Some(p) = tile.rgba.get(from..from + 4) {
                rgba[to..to + 4].copy_from_slice(p);
            }
        }
    }
    Texture { width: w, height: h, rgba }
}

/// The tiles of a sheet `columns` × `rows`, row by row.
pub fn slice(sheet: &Texture, columns: u32, rows: u32) -> Vec<Arc<Texture>> {
    let (tw, th) = (sheet.width / columns.max(1), sheet.height / rows.max(1));
    if tw == 0 || th == 0 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for r in 0..rows {
        for c in 0..columns {
            let mut rgba = Vec::with_capacity((tw * th * 4) as usize);
            for y in 0..th {
                let from = (((r * th + y) * sheet.width + c * tw) * 4) as usize;
                rgba.extend_from_slice(&sheet.rgba[from..from + (tw * 4) as usize]);
            }
            out.push(Arc::new(Texture { width: tw, height: th, rgba }));
        }
    }
    out
}

/// A full CTM's five compact tiles, of its 47 (OptiFine's template, 12 a
/// row): alone (0), joined across (2), joined up and down (24), inside
/// (26), and inside for the inner corners too (the template's corner
/// tiles differ only by a notch at the corner).
pub const FULL_TO_COMPACT: [usize; 5] = [0, 2, 24, 26, 26];

/// A texture of Fusion's, from its `.png.mcmeta`: its sheet's way of
/// connecting, if Massif draws it.
pub fn fusion(meta: &Value, sheet: &Texture, source: &str) -> Option<Connected> {
    let f = meta.get("fusion")?;
    let count = |key: &str| f.get(key).and_then(Value::as_u64).map_or(1, |v| v.clamp(1, 256) as u32);
    let kind = match f.get("type").and_then(Value::as_str)? {
        "continuous" => {
            let (columns, rows) = (count("columns"), count("rows"));
            let tiles = slice(sheet, columns, rows);
            (tiles.len() > 1).then_some(Kind::Repeat { width: columns, height: rows, tiles })?
        }
        "random" => {
            let tiles = slice(sheet, count("columns"), count("rows"));
            (tiles.len() > 1).then(|| Kind::Random(tiles.into_iter().map(|t| (t, 1)).collect()))?
        }
        "connecting" => match f.get("layout").and_then(Value::as_str).unwrap_or("full") {
            "horizontal" => Kind::Horizontal(slice(sheet, 4, 1).try_into().ok()?),
            "vertical" => Kind::Vertical(slice(sheet, 1, 4).try_into().ok()?),
            "compact" | "pieced" => Kind::Compact(slice(sheet, 5, 1).try_into().ok()?),
            _ => return None,
        },
        _ => return None,
    };
    Some(Connected { kind, faces: ALL, source: source.to_owned() })
}

/// What an OptiFine CTM file names: the textures (`matchTiles`) and the
/// blocks (`matchBlocks`) it applies to, and how it connects.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub tiles: Vec<String>,
    pub blocks: Vec<String>,
    pub connected: Connected,
}

/// `0-3 5 glass_top <default>` → the tiles' files, beside the properties
/// file (`<folder>/0.png`…), or `None` for `<default>` and `<skip>`.
fn tile_files(list: &str, folder: &str) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for t in list.split_whitespace() {
        if t.starts_with('<') {
            out.push(None);
        } else if let Some((a, b)) = t.split_once('-').filter(|(a, b)| a.parse::<u32>().is_ok() && b.parse::<u32>().is_ok()) {
            let (a, b): (u32, u32) = (a.parse().unwrap_or(0), b.parse().unwrap_or(0));
            for k in a..=b.min(a + 255) {
                out.push(Some(format!("{folder}{k}.png")));
            }
        } else {
            let name = if t.ends_with(".png") { t.to_owned() } else { format!("{t}.png") };
            out.push(Some(match name.split_once(':') {
                Some((ns, rest)) => format!("assets/{ns}/{rest}"),
                None if name.contains('/') => format!("assets/minecraft/{name}"),
                None => format!("{folder}{name}"),
            }));
        }
    }
    out
}

/// A texture's key as the catalogue names it (`minecraft:block/stone`
/// for `stone`, `block/stone`, `textures/block/stone.png`).
pub fn texture_key(name: &str) -> String {
    let (ns, path) = name.split_once(':').unwrap_or(("minecraft", name));
    let path = path.trim_start_matches("textures/").trim_end_matches(".png");
    let path = if path.contains('/') { path.to_owned() } else { format!("block/{path}") };
    if ns == "minecraft" { path } else { format!("{ns}:{path}") }
}

/// An OptiFine CTM file read, its tiles' pictures given by `png`.
pub fn optifine(file: &str, text: &str, png: &dyn Fn(&str) -> Option<Texture>) -> Option<Rule> {
    let props = crate::true_color::properties(text);
    let get = |k: &str| props.iter().find(|(p, _)| p == k).map(|(_, v)| v.as_str());
    let folder = &file[..file.rfind('/').map_or(0, |i| i + 1)];
    let stem = file.rsplit('/').next().unwrap_or(file).trim_end_matches(".properties");
    let tiles: Vec<Option<Arc<Texture>>> = tile_files(get("tiles").unwrap_or(""), folder).into_iter().map(|f| f.and_then(|f| png(&f)).map(Arc::new)).collect();
    let all: Option<Vec<Arc<Texture>>> = tiles.iter().cloned().collect();
    let pick = |ks: &[usize]| -> Option<Vec<Arc<Texture>>> { ks.iter().map(|&k| tiles.get(k).cloned().flatten()).collect() };
    let kind = match get("method").unwrap_or("ctm") {
        "ctm" | "glass" => Kind::Compact(pick(&FULL_TO_COMPACT)?.try_into().ok()?),
        "ctm_compact" => Kind::Compact(pick(&[0, 2, 3, 1, 4])?.try_into().ok()?),
        "horizontal" | "bookshelf" => Kind::Horizontal(pick(&[3, 0, 1, 2])?.try_into().ok()?),
        "vertical" => Kind::Vertical(pick(&[3, 0, 1, 2])?.try_into().ok()?),
        "repeat" | "pattern" => {
            let (w, h) = (get("width").and_then(|v| v.parse().ok()).unwrap_or(1u32), get("height").and_then(|v| v.parse().ok()).unwrap_or(1u32));
            let tiles = all?;
            (tiles.len() as u32 >= w * h && w * h > 0).then_some(Kind::Repeat { width: w, height: h, tiles })?
        }
        "random" => {
            let tiles = all?;
            let weights: Vec<u32> = get("weights").map(|w| w.split_whitespace().filter_map(|x| x.parse().ok()).collect()).unwrap_or_default();
            Kind::Random(tiles.into_iter().enumerate().map(|(k, t)| (t, weights.get(k).copied().unwrap_or(1))).collect())
        }
        _ => return None,
    };
    let faces = match get("faces") {
        None => ALL,
        Some(f) => f.split_whitespace().fold(0, |m, x| {
            m | match x {
                "top" => UP,
                "bottom" => DOWN,
                "sides" | "north" | "south" | "east" | "west" => SIDES,
                "all" => ALL,
                _ => 0,
            }
        }),
    };
    let names = |k: &str| get(k).map(|v| v.split_whitespace().map(str::to_owned).collect::<Vec<_>>()).unwrap_or_default();
    let mut blocks: Vec<String> = names("matchBlocks")
        .into_iter()
        .filter_map(|b| {
            let parts: Vec<&str> = b.split(':').collect();
            match parts.as_slice() {
                [n] => Some(format!("minecraft:{n}")),
                [a, n, ..] if !n.contains('=') => Some(format!("{a}:{n}")),
                [n, ..] => Some(format!("minecraft:{n}")),
                [] => None,
            }
        })
        .collect();
    let mut tiles_named: Vec<String> = names("matchTiles").iter().map(|t| texture_key(t)).collect();
    // Neither: the file's name says which, `block_<id>` a block, else a
    // texture.
    if blocks.is_empty() && tiles_named.is_empty() {
        match stem.strip_prefix("block_") {
            Some(b) => blocks.push(format!("minecraft:{b}")),
            None => tiles_named.push(texture_key(stem)),
        }
    }
    Some(Rule { tiles: tiles_named, blocks, connected: Connected { kind, faces, source: file.to_owned() } })
}

/// Every connected texture an archive's OptiFine files describe.
pub fn read_optifine(archive: &crate::zip::Archive) -> Vec<Rule> {
    let mut files: Vec<String> = archive.names().filter(|n| n.ends_with(".properties") && (n.contains("/optifine/ctm/") || n.contains("/mcpatcher/ctm/"))).map(str::to_owned).collect();
    files.sort();
    let png = |name: &str| -> Option<Texture> {
        let bytes = archive.read(name)?.ok()?;
        let img = image::load_from_memory_with_format(&bytes, image::ImageFormat::Png).ok()?.to_rgba8();
        let (w, h) = img.dimensions();
        let side = w.min(h);
        Some(Texture { width: w, height: side, rgba: image::imageops::crop_imm(&img, 0, 0, w, side).to_image().into_raw() })
    };
    files.iter().filter_map(|f| optifine(f, &String::from_utf8_lossy(&archive.read(f)?.ok()?), &png)).collect()
}

/// The connected textures of the blocks read, by texture key.
pub type Table = HashMap<String, Arc<Connected>>;

#[cfg(test)]
mod tests {
    use super::*;

    fn tile(c: u8) -> Arc<Texture> {
        Arc::new(Texture { width: 4, height: 4, rgba: [c, c, c, 255].repeat(16) })
    }

    fn compact() -> Connected {
        Connected { kind: Kind::Compact([tile(0), tile(1), tile(2), tile(3), tile(4)]), faces: ALL, source: String::new() }
    }

    fn quarters(t: &Texture) -> [u8; 4] {
        let at = |x: u32, y: u32| t.rgba[((y * t.width + x) * 4) as usize];
        [at(0, 0), at(3, 0), at(0, 3), at(3, 3)]
    }

    #[test]
    fn compact_faces_are_pieced_by_their_neighbours() {
        let c = compact();
        assert_eq!(quarters(&c.face(Around::default(), 0, 0, (0, 0, 0))), [0; 4], "alone");
        assert_eq!(quarters(&c.face(Around::INSIDE, 0, 0, (0, 0, 0))), [3; 4], "inside");
        // Joined on the right only: its right half joined across.
        let right = Around { right: true, ..Default::default() };
        assert_eq!(quarters(&c.face(right, 0, 0, (0, 0, 0))), [0, 1, 0, 1]);
        // Joined right and down, without the corner: an inner corner there.
        let corner = Around { right: true, down: true, ..Default::default() };
        assert_eq!(quarters(&c.face(corner, 0, 0, (0, 0, 0))), [0, 1, 2, 4]);
    }

    #[test]
    fn rows_patterns_and_random_tiles() {
        let h = Connected { kind: Kind::Horizontal([tile(0), tile(1), tile(2), tile(3)]), faces: SIDES, source: String::new() };
        let shade = |t: Arc<Texture>| t.rgba[0];
        assert_eq!(shade(h.face(Around::default(), 0, 0, (0, 0, 0))), 0);
        assert_eq!(shade(h.face(Around { right: true, ..Default::default() }, 0, 0, (0, 0, 0))), 1);
        assert_eq!(shade(h.face(Around { left: true, right: true, ..Default::default() }, 0, 0, (0, 0, 0))), 2);
        assert_eq!(shade(h.face(Around { left: true, ..Default::default() }, 0, 0, (0, 0, 0))), 3);
        let r = Connected { kind: Kind::Repeat { width: 2, height: 2, tiles: vec![tile(10), tile(11), tile(12), tile(13)] }, faces: ALL, source: String::new() };
        assert_eq!([(0, 0), (1, 0), (0, 1), (1, 1), (2, 3)].map(|(u, v)| shade(r.face(Around::default(), u, v, (0, 0, 0)))), [10, 11, 12, 13, 12]);
        let random = Connected { kind: Kind::Random(vec![(tile(20), 1), (tile(21), 3)]), faces: ALL, source: String::new() };
        let picks: Vec<u8> = (0..400).map(|x| shade(random.face(Around::default(), 0, 0, (x, 64, x * 7)))).collect();
        let many = picks.iter().filter(|&&p| p == 21).count();
        assert!((250..350).contains(&many), "weighted 3 to 1: {many}");
        assert_eq!(random.face(Around::default(), 0, 0, (5, 6, 7)), random.face(Around::default(), 0, 0, (5, 6, 7)), "the same place, the same tile");
    }

    #[test]
    fn optifine_files_are_read() {
        let png = |name: &str| -> Option<Texture> {
            let k: u8 = name.rsplit('/').next()?.trim_end_matches(".png").parse().ok()?;
            Some(Texture { width: 4, height: 4, rgba: [k, k, k, 255].repeat(16) })
        };
        let glass = optifine("assets/minecraft/optifine/ctm/glass/glass.properties", "matchBlocks=glass\nmethod=ctm\ntiles=0-46\nconnect=block\n", &png).unwrap();
        assert_eq!(glass.blocks, ["minecraft:glass"]);
        let Kind::Compact(t) = &glass.connected.kind else { panic!("compact") };
        assert_eq!(t.iter().map(|x| x.rgba[0]).collect::<Vec<_>>(), [0, 2, 24, 26, 26]);
        let shelf = optifine("assets/minecraft/optifine/ctm/bookshelf/bookshelf.properties", "matchTiles=bookshelf\nmethod=horizontal\ntiles=0-3\nfaces=sides\n", &png).unwrap();
        assert_eq!(shelf.tiles, ["block/bookshelf"]);
        assert_eq!(shelf.connected.faces, SIDES);
        let Kind::Horizontal(t) = &shelf.connected.kind else { panic!("horizontal") };
        assert_eq!(t.iter().map(|x| x.rgba[0]).collect::<Vec<_>>(), [3, 0, 1, 2], "alone first");
        let stone = optifine("assets/minecraft/optifine/ctm/stone/block_stone.properties", "method=random\ntiles=0-2\nweights=1 2 3\n", &png).unwrap();
        assert_eq!(stone.blocks, ["minecraft:stone"], "named by its file");
        assert!(optifine("x/y.properties", "method=overlay\ntiles=0-16\n", &png).is_none(), "overlays are left out");
        assert_eq!(texture_key("conquest:block/old_bricks"), "conquest:block/old_bricks");
        assert_eq!(texture_key("minecraft:textures/block/glass.png"), "block/glass");
    }

    #[test]
    fn fusion_sheets_connect() {
        let sheet = Texture { width: 80, height: 16, rgba: (0..80 * 16).flat_map(|i| { let c = (i % 80 / 16) as u8; [c, c, c, 255] }).collect() };
        let meta: Value = serde_json::from_str(r#"{"fusion":{"type":"connecting","layout":"pieced"}}"#).unwrap();
        let c = fusion(&meta, &sheet, "x").unwrap();
        assert_eq!(c.alone().rgba[0], 0);
        assert_eq!(c.face(Around::INSIDE, 0, 0, (0, 0, 0)).rgba[0], 3);
        let meta: Value = serde_json::from_str(r#"{"fusion":{"type":"continuous","rows":1,"columns":5}}"#).unwrap();
        let r = fusion(&meta, &sheet, "x").unwrap();
        assert_eq!(r.face(Around::default(), 3, 0, (0, 0, 0)).rgba[0], 3);
    }
}
