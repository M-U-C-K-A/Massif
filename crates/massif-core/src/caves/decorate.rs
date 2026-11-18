//! The decoration of caves: what grows and forms on their floors, walls
//! and ceilings, by the cave biome painted there (see
//! [`super::CaveBiomes`]), as the game's own cave biomes have it.
//!
//! - Lush caves: moss over the floor and the ceiling, carpets of moss,
//!   grass, azaleas, cave vines hanging with glow berries, spore blossoms,
//!   rooted dirt with hanging roots, and pools of water lined with clay
//!   where big and small dripleaves stand.
//! - Dripstone caves: dripstone blocks in patches, stalactites hanging and
//!   stalagmites standing, of pointed dripstone of the right thickness
//!   from base to tip (merged where they meet).
//! - The deep dark: sculk over the floor, walls and ceiling, sculk veins
//!   clinging to the rock around, sculk sensors, shriekers and catalysts.
//! - Other caves: glow lichen here and there on walls and ceilings.
//!
//! What a block's surroundings are decides what it gets: a floor (rock
//! under the air), a ceiling (rock over it) or a wall, how flat a floor is
//! (the rock under the blocks around), how much air there is below a
//! ceiling or above a floor. Patches follow a noise, single things a hash
//! of the block, so that a tile is decorated the same every time, the
//! same whatever part of the map is exported. A tile decorates its own
//! blocks only, reading a column around it for the faces at its edges.

use super::noise;
use super::seal::Seals;
use crate::compose::TileStack;
use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::registry::Registry;
use crate::world::Dimension;
use std::collections::HashMap;

/// The thickness of a block of pointed dripstone, from its base to its tip.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Thickness {
    Base,
    Middle,
    Frustum,
    Tip,
    /// The tip of a stalactite meeting the tip of a stalagmite.
    TipMerge,
}

impl Thickness {
    pub const ALL: [Thickness; 5] = [Thickness::Base, Thickness::Middle, Thickness::Frustum, Thickness::Tip, Thickness::TipMerge];

    pub const fn name(self) -> &'static str {
        match self {
            Thickness::Base => "base",
            Thickness::Middle => "middle",
            Thickness::Frustum => "frustum",
            Thickness::Tip => "tip",
            Thickness::TipMerge => "tip_merge",
        }
    }
}

/// The faces of a block a lichen or a vein clings to: bits of
/// [`FACES`].
pub type Faces = u8;
/// The face names of the game's multiface blocks, by bit.
pub const FACES: [&str; 6] = ["down", "up", "north", "south", "west", "east"];
/// The neighbour behind each face, `(dx, dy, dz)`.
const FACE_DIRS: [(i32, i32, i32); 6] = [(0, -1, 0), (0, 1, 0), (0, 0, -1), (0, 0, 1), (-1, 0, 0), (1, 0, 0)];

/// What decorates a block.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Deco {
    // Rock made of something else.
    MossBlock,
    Clay,
    RootedDirt,
    DripstoneBlock,
    Sculk,
    /// The water of a pool in the floor.
    PoolWater,
    // In the air.
    MossCarpet,
    ShortGrass,
    Azalea,
    FloweringAzalea,
    SporeBlossom,
    HangingRoots,
    /// A cave vine's last block, and those above it.
    CaveVines { berries: bool },
    CaveVinesPlant { berries: bool },
    /// The leaf of a big dripleaf, and its stem under it; facing 0 to 3
    /// (north, east, south, west).
    BigDripleaf { facing: u8 },
    BigDripleafStem { facing: u8 },
    SmallDripleaf { upper: bool, facing: u8 },
    PointedDripstone { up: bool, thickness: Thickness },
    GlowLichen { faces: Faces },
    SculkVein { faces: Faces },
    SculkSensor,
    SculkShrieker,
    SculkCatalyst,
}

impl Deco {
    /// Every decoration, for the export to give each its block state.
    pub fn all() -> Vec<Deco> {
        let mut out = vec![
            Deco::MossBlock,
            Deco::Clay,
            Deco::RootedDirt,
            Deco::DripstoneBlock,
            Deco::Sculk,
            Deco::PoolWater,
            Deco::MossCarpet,
            Deco::ShortGrass,
            Deco::Azalea,
            Deco::FloweringAzalea,
            Deco::SporeBlossom,
            Deco::HangingRoots,
            Deco::SculkSensor,
            Deco::SculkShrieker,
            Deco::SculkCatalyst,
        ];
        for berries in [false, true] {
            out.push(Deco::CaveVines { berries });
            out.push(Deco::CaveVinesPlant { berries });
        }
        for facing in 0..4 {
            out.push(Deco::BigDripleaf { facing });
            out.push(Deco::BigDripleafStem { facing });
            out.push(Deco::SmallDripleaf { upper: false, facing });
            out.push(Deco::SmallDripleaf { upper: true, facing });
        }
        for up in [false, true] {
            for thickness in Thickness::ALL {
                out.push(Deco::PointedDripstone { up, thickness });
            }
        }
        for faces in 1..64u8 {
            out.push(Deco::GlowLichen { faces });
            out.push(Deco::SculkVein { faces });
        }
        out
    }

    /// The block state of it, as the game names it.
    pub fn state(self) -> String {
        const FACING: [&str; 4] = ["north", "east", "south", "west"];
        let faces = |f: Faces| {
            let mut s: Vec<String> = FACES.iter().enumerate().map(|(i, n)| format!("{n}={}", f & (1 << i) != 0)).collect();
            s.push("waterlogged=false".into());
            s.join(",")
        };
        match self {
            Deco::MossBlock => "minecraft:moss_block".into(),
            Deco::Clay => "minecraft:clay".into(),
            Deco::RootedDirt => "minecraft:rooted_dirt".into(),
            Deco::DripstoneBlock => "minecraft:dripstone_block".into(),
            Deco::Sculk => "minecraft:sculk".into(),
            Deco::PoolWater => "minecraft:water[level=0]".into(),
            Deco::MossCarpet => "minecraft:moss_carpet".into(),
            Deco::ShortGrass => "minecraft:short_grass".into(),
            Deco::Azalea => "minecraft:azalea".into(),
            Deco::FloweringAzalea => "minecraft:flowering_azalea".into(),
            Deco::SporeBlossom => "minecraft:spore_blossom".into(),
            Deco::HangingRoots => "minecraft:hanging_roots[waterlogged=false]".into(),
            Deco::CaveVines { berries } => format!("minecraft:cave_vines[age=25,berries={berries}]"),
            Deco::CaveVinesPlant { berries } => format!("minecraft:cave_vines_plant[berries={berries}]"),
            Deco::BigDripleaf { facing } => format!("minecraft:big_dripleaf[facing={},tilt=none,waterlogged=false]", FACING[facing as usize]),
            Deco::BigDripleafStem { facing } => format!("minecraft:big_dripleaf_stem[facing={},waterlogged=false]", FACING[facing as usize]),
            Deco::SmallDripleaf { upper, facing } => format!("minecraft:small_dripleaf[facing={},half={},waterlogged=false]", FACING[facing as usize], if upper { "upper" } else { "lower" }),
            Deco::PointedDripstone { up, thickness } => format!("minecraft:pointed_dripstone[thickness={},vertical_direction={},waterlogged=false]", thickness.name(), if up { "up" } else { "down" }),
            Deco::GlowLichen { faces: f } => format!("minecraft:glow_lichen[{}]", faces(f)),
            Deco::SculkVein { faces: f } => format!("minecraft:sculk_vein[{}]", faces(f)),
            Deco::SculkSensor => "minecraft:sculk_sensor[power=0,sculk_sensor_phase=inactive,waterlogged=false]".into(),
            Deco::SculkShrieker => "minecraft:sculk_shrieker[can_summon=true,shrieking=false,waterlogged=false]".into(),
            Deco::SculkCatalyst => "minecraft:sculk_catalyst[bloom=false]".into(),
        }
    }

    /// The block entity the game keeps for it, if any.
    pub fn entity(self) -> Option<&'static str> {
        match self {
            Deco::SculkSensor => Some("minecraft:sculk_sensor"),
            Deco::SculkShrieker => Some("minecraft:sculk_shrieker"),
            Deco::SculkCatalyst => Some("minecraft:sculk_catalyst"),
            _ => None,
        }
    }

    /// Whether it is solid rock of another kind (it takes the place of the
    /// rock of a floor, wall or ceiling).
    pub fn is_rock(self) -> bool {
        matches!(self, Deco::MossBlock | Deco::Clay | Deco::RootedDirt | Deco::DripstoneBlock | Deco::Sculk)
    }
}

/// The cave biomes that have decorations of their own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Plain,
    Lush,
    Dripstone,
    DeepDark,
}

/// The decorations of a tile, by column.
#[derive(Debug, Default, Clone)]
pub struct Decorations {
    /// `(cell, y, deco)`, sorted by cell and height.
    items: Vec<(u16, i16, Deco)>,
}

impl Decorations {
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    /// The decorations of column `cell`, as `(y, deco)`.
    pub fn column(&self, cell: usize) -> impl Iterator<Item = (i32, Deco)> + '_ {
        let at = self.items.partition_point(|b| (b.0 as usize) < cell);
        self.items[at..].iter().take_while(move |b| b.0 as usize == cell).map(|b| (b.1 as i32, b.2))
    }

    /// How many of each decoration.
    pub fn counts(&self) -> HashMap<Deco, usize> {
        let mut out = HashMap::new();
        for &(_, _, d) in &self.items {
            *out.entry(d).or_insert(0) += 1;
        }
        out
    }
}

/// The blocks of the tile and of a column around it, from `y0` up: solid
/// or not, fluid or not, and the top of each column.
struct Grid {
    y0: i32,
    h: usize,
    solid: Vec<bool>,
    wet: Vec<bool>,
    tops: Vec<i32>,
}

const SIDE: usize = TILE_SIZE as usize + 2;

impl Grid {
    #[inline]
    fn index(&self, i: i32, y: i32, j: i32) -> Option<usize> {
        if !(-1..=TILE_SIZE).contains(&i) || !(-1..=TILE_SIZE).contains(&j) || y < self.y0 || y >= self.y0 + self.h as i32 {
            return None;
        }
        Some((((j + 1) as usize * SIDE) + (i + 1) as usize) * self.h + (y - self.y0) as usize)
    }

    /// Solid, as far as is known (outside the grid: yes).
    #[inline]
    fn solid(&self, i: i32, y: i32, j: i32) -> bool {
        self.index(i, y, j).is_none_or(|k| self.solid[k])
    }

    /// Air closed in under the top of its column: a cave's, a room's.
    #[inline]
    fn cave_air(&self, i: i32, y: i32, j: i32) -> bool {
        self.index(i, y, j).is_some_and(|k| !self.solid[k] && !self.wet[k] && y < self.tops[(j + 1) as usize * SIDE + (i + 1) as usize])
    }
}

/// A number in `[0, 1)` for block `(x, y, z)` and `salt`.
#[inline]
fn chance(x: i32, y: i32, z: i32, salt: u64) -> f32 {
    (noise::hash3(salt, x, y, z) >> 40) as f32 / (1u64 << 24) as f32
}

/// The decorations of the tile at `pos` (see the module). `seals`: the
/// blocks the export makes rock there.
pub fn decorate_tile(dim: &Dimension, reg: &Registry, pos: TilePos, hidden: bool, seals: &Seals) -> Decorations {
    let Some(stack) = TileStack::new(dim, pos, hidden) else { return Decorations::default() };
    let ground = stack.ground();
    let painted = ground.cave_biomes.used();
    // The biomes that decorate, by number.
    let kind_of = |b: u16| match reg.biomes.get(b as usize).map(|d| d.id.as_str()) {
        Some("minecraft:lush_caves") => Kind::Lush,
        Some("minecraft:dripstone_caves") => Kind::Dripstone,
        Some("minecraft:deep_dark") => Kind::DeepDark,
        _ => Kind::Plain,
    };
    let lichen = stack.caves().is_some();
    if !lichen && painted.iter().all(|&b| kind_of(b) == Kind::Plain) {
        return Decorations::default();
    }
    let cave_biomes = ground.cave_biomes.clone();
    let seed = dim.caves.defs().first().map_or(0, |d| d.seed as u64) ^ 0xDEC0;

    // The heights where the tile has air under its tops.
    let n = TILE_SIZE;
    let (ox, oz) = pos.origin();
    let mut spans = Vec::new();
    let mut fluids = Vec::new();
    let (mut lo, mut hi) = (i32::MAX, i32::MIN);
    for cell in 0..crate::coords::TILE_AREA {
        stack.spans(cell, &mut spans);
        for w in spans.windows(2) {
            lo = lo.min(w[0].y1);
            hi = hi.max(w[1].y0);
        }
    }
    if lo >= hi {
        return Decorations::default();
    }
    let (y0, y1) = (lo - 1, hi + 2);
    let h = (y1 - y0) as usize;
    let mut grid = Grid { y0, h, solid: vec![true; SIDE * SIDE * h], wet: vec![false; SIDE * SIDE * h], tops: vec![dim.max_y; SIDE * SIDE] };
    let mut stacks: HashMap<TilePos, Option<TileStack>> = HashMap::new();
    stacks.insert(pos, Some(stack));
    for j in -1..=n {
        for i in -1..=n {
            let (x, z) = (ox + i, oz + j);
            let p = TilePos::containing(x, z);
            let Some(stack) = stacks.entry(p).or_insert_with(|| TileStack::new(dim, p, hidden)) else { continue };
            let cell = local_index(x, z);
            if !stack.ground().present(cell) {
                continue;
            }
            stack.spans(cell, &mut spans);
            stack.fluids(cell, &spans, &mut fluids);
            let c = (j + 1) as usize * SIDE + (i + 1) as usize;
            grid.tops[c] = spans.last().map_or(dim.min_y, |s| s.y1);
            for y in y0..y1 {
                let k = c * h + (y - y0) as usize;
                grid.solid[k] = spans.iter().any(|s| s.y0 <= y && y < s.y1);
                grid.wet[k] = !grid.solid[k] && fluids.iter().any(|f| f.0 <= y && y < f.1);
            }
            if p == pos {
                for y in seals.column(cell) {
                    if let Some(k) = grid.index(i, y, j) {
                        grid.solid[k] = true;
                        grid.wet[k] = false;
                    }
                }
            }
        }
    }

    let mut out: HashMap<(u16, i16), Deco> = HashMap::new();
    // Nothing in the structures, which bring their own.
    let structures = super::kept_dry(dim, pos);
    let put = |i: i32, y: i32, j: i32, d: Deco, out: &mut HashMap<(u16, i16), Deco>| {
        if (0..n).contains(&i) && (0..n).contains(&j) && !super::in_boxes(&structures, ox + i, y, oz + j, 0) {
            out.entry((local_index(ox + i, oz + j) as u16, y as i16)).or_insert(d);
        }
    };
    // Stalactites and stalagmites, by column, to merge where they meet.
    for j in 0..n {
        for i in 0..n {
            let (x, z) = (ox + i, oz + j);
            let mut y = y0 + 1;
            while y < y1 - 1 {
                if !grid.cave_air(i, y, j) {
                    y += 1;
                    continue;
                }
                // A run of air from `y` up to `top` (exclusive).
                let mut top = y;
                while top < y1 && grid.cave_air(i, top, j) {
                    top += 1;
                }
                let floor = grid.solid(i, y - 1, j);
                let ceiling = grid.solid(i, top, j);
                decorate_run(&grid, &cave_biomes, &kind_of, lichen, seed, (i, j), (x, z), (y, top), (floor, ceiling), &mut |ii, yy, jj, d| put(ii, yy, jj, d, &mut out));
                y = top;
            }
        }
    }
    let mut items: Vec<(u16, i16, Deco)> = out.into_iter().map(|((c, y), d)| (c, y, d)).collect();
    items.sort_unstable_by_key(|t| (t.0, t.1));
    Decorations { items }
}

/// Decorate the run of air `y..top` of column `(i, j)` of the tile (blocks
/// `(x, z)`), over a floor and under a ceiling if `floor`, `ceiling`.
#[allow(clippy::too_many_arguments)]
fn decorate_run(grid: &Grid, biomes: &super::CaveBiomes, kind_of: &dyn Fn(u16) -> Kind, lichen: bool, seed: u64, (i, j): (i32, i32), (x, z): (i32, i32), (y, top): (i32, i32), (floor, ceiling): (bool, bool), put: &mut dyn FnMut(i32, i32, i32, Deco)) {
    let kind = |yy: i32| kind_of(biomes.get(i, yy, j));
    let gap = top - y;
    // How flat the floor is: the rock under the blocks around.
    let flat = |yy: i32, dy: i32| {
        let mut n = 0;
        for (di, dj) in [(-1, -1), (0, -1), (1, -1), (-1, 0), (1, 0), (-1, 1), (0, 1), (1, 1)] {
            n += usize::from(grid.solid(i + di, yy + dy, j + dj) && !grid.solid(i + di, yy, j + dj));
        }
        n
    };
    let patch = |yy: i32, salt: u64, scale: f32| noise::gradient(seed ^ salt, x as f32 / scale, yy as f32 / scale, z as f32 / scale);
    // ------------------------------------------------------------ floor --
    if floor {
        let k = kind(y);
        let level = flat(y, -1);
        match k {
            Kind::Lush => {
                if patch(y, 0x10, 7.0) > -0.25 {
                    // A pool: the floor's block water, rock all around it.
                    let pool = level >= 7 && patch(y, 0x11, 5.0) > 0.42 && (1..TILE_SIZE - 1).contains(&i) && (1..TILE_SIZE - 1).contains(&j) && [(1, 0), (-1, 0), (0, 1), (0, -1)].iter().all(|&(di, dj)| grid.solid(i + di, y - 1, j + dj)) && grid.solid(i, y - 2, j);
                    put(i, y - 1, j, if pool { Deco::PoolWater } else { Deco::MossBlock });
                    if pool {
                        put(i, y - 2, j, Deco::Clay);
                        for (di, dj) in [(1, 0), (-1, 0), (0, 1), (0, -1)] {
                            put(i + di, y - 1, j + dj, Deco::Clay);
                        }
                    } else {
                        let r = chance(x, y, z, seed ^ 0x12);
                        let near_pool = patch(y, 0x11, 5.0) > 0.3;
                        let facing = (chance(x, y, z, seed ^ 0x13) * 4.0) as u8 % 4;
                        if near_pool && r < 0.12 && gap >= 3 {
                            // A big dripleaf: a stem or two, its leaf on top.
                            let stems = 1 + (chance(x, y, z, seed ^ 0x14) * 2.0) as i32;
                            let stems = stems.min(gap - 2);
                            for s in 0..stems {
                                put(i, y + s, j, Deco::BigDripleafStem { facing });
                            }
                            put(i, y + stems, j, Deco::BigDripleaf { facing });
                        } else if near_pool && r < 0.22 && gap >= 2 {
                            put(i, y, j, Deco::SmallDripleaf { upper: false, facing });
                            put(i, y + 1, j, Deco::SmallDripleaf { upper: true, facing });
                        } else if r < 0.30 {
                            put(i, y, j, Deco::MossCarpet);
                        } else if r < 0.40 {
                            put(i, y, j, Deco::ShortGrass);
                        } else if r < 0.42 && gap >= 2 {
                            put(i, y, j, Deco::Azalea);
                        } else if r < 0.43 && gap >= 2 {
                            put(i, y, j, Deco::FloweringAzalea);
                        }
                    }
                }
            }
            Kind::Dripstone => {
                if patch(y, 0x20, 6.0) > 0.0 {
                    put(i, y - 1, j, Deco::DripstoneBlock);
                }
            }
            Kind::DeepDark => {
                if patch(y, 0x30, 8.0) > -0.35 {
                    put(i, y - 1, j, Deco::Sculk);
                    let r = chance(x, y, z, seed ^ 0x31);
                    if level >= 5 {
                        if r < 0.012 {
                            put(i, y, j, Deco::SculkSensor);
                        } else if r < 0.016 {
                            put(i, y, j, Deco::SculkShrieker);
                        } else if r < 0.018 {
                            put(i, y, j, Deco::SculkCatalyst);
                        }
                    }
                }
            }
            Kind::Plain => {}
        }
    }
    // ---------------------------------------------------------- ceiling --
    if ceiling {
        let yc = top - 1;
        match kind(yc) {
            Kind::Lush => {
                let r = chance(x, yc, z, seed ^ 0x40);
                if patch(yc, 0x41, 7.0) > -0.1 {
                    put(i, top, j, Deco::MossBlock);
                    if r < 0.07 {
                        // Cave vines, down as far as there is room.
                        let length = (1 + (chance(x, yc, z, seed ^ 0x42) * 4.0) as i32).min(gap - 1).max(1);
                        for s in 0..length {
                            let berries = chance(x, yc - s, z, seed ^ 0x43) < 0.35;
                            put(i, yc - s, j, if s + 1 == length { Deco::CaveVines { berries } } else { Deco::CaveVinesPlant { berries } });
                        }
                    } else if r < 0.078 {
                        put(i, yc, j, Deco::SporeBlossom);
                    }
                } else if r < 0.15 {
                    put(i, top, j, Deco::RootedDirt);
                    put(i, yc, j, Deco::HangingRoots);
                }
            }
            Kind::Dripstone => {
                if patch(yc, 0x50, 6.0) > 0.0 {
                    put(i, top, j, Deco::DripstoneBlock);
                }
            }
            Kind::DeepDark => {
                if patch(yc, 0x60, 8.0) > 0.0 {
                    put(i, top, j, Deco::Sculk);
                }
            }
            Kind::Plain => {}
        }
    }
    // Stalactites and stalagmites, merged where they meet.
    if floor && ceiling && gap >= 2 && (kind(y) == Kind::Dripstone || kind(top - 1) == Kind::Dripstone) {
        let hang = (kind(top - 1) == Kind::Dripstone && chance(x, top, z, seed ^ 0x70) < 0.09).then(|| 1 + (chance(x, top, z, seed ^ 0x71) * 6.0) as i32);
        let stand = (kind(y) == Kind::Dripstone && chance(x, y, z, seed ^ 0x72) < 0.06).then(|| 1 + (chance(x, y, z, seed ^ 0x73) * 5.0) as i32);
        let (mut down, mut up) = (hang.unwrap_or(0), stand.unwrap_or(0));
        let merged = down > 0 && up > 0 && down + up >= gap;
        if merged {
            down = gap / 2;
            up = gap - down;
        } else {
            // Leave a block of air between them, and above a stalagmite.
            down = down.min(gap - 1);
            up = up.min(gap - 1 - down.min(gap - 1));
        }
        for (k, t) in column(down, merged).into_iter().enumerate() {
            put(i, top - 1 - k as i32, j, Deco::PointedDripstone { up: false, thickness: t });
        }
        for (k, t) in column(up, merged).into_iter().enumerate() {
            put(i, y + k as i32, j, Deco::PointedDripstone { up: true, thickness: t });
        }
    }
    // ------------------------------------------------------------ walls --
    for yy in y..top {
        let k = kind(yy);
        // The rock beside, by face.
        let mut faces: Faces = 0;
        for (f, &(dx, dy, dz)) in FACE_DIRS.iter().enumerate() {
            if grid.solid(i + dx, yy + dy, j + dz) {
                faces |= 1 << f;
            }
        }
        let sides = faces & 0b11_1100;
        match k {
            Kind::Lush if sides != 0 && patch(yy, 0x80, 6.0) > 0.35 => {
                for (f, &(dx, _, dz)) in FACE_DIRS.iter().enumerate().skip(2) {
                    if faces & (1 << f) != 0 {
                        put(i + dx, yy, j + dz, Deco::MossBlock);
                    }
                }
            }
            Kind::Dripstone if sides != 0 && patch(yy, 0x81, 6.0) > 0.3 => {
                for (f, &(dx, _, dz)) in FACE_DIRS.iter().enumerate().skip(2) {
                    if faces & (1 << f) != 0 {
                        put(i + dx, yy, j + dz, Deco::DripstoneBlock);
                    }
                }
            }
            Kind::DeepDark if faces != 0 => {
                if sides != 0 && patch(yy, 0x82, 8.0) > 0.1 {
                    for (f, &(dx, _, dz)) in FACE_DIRS.iter().enumerate().skip(2) {
                        if faces & (1 << f) != 0 {
                            put(i + dx, yy, j + dz, Deco::Sculk);
                        }
                    }
                } else if patch(yy, 0x83, 5.0) > -0.2 && chance(x, yy, z, seed ^ 0x84) < 0.45 {
                    put(i, yy, j, Deco::SculkVein { faces });
                }
            }
            _ if lichen && faces & !1 != 0 && chance(x, yy, z, seed ^ 0x90) < 0.02 => {
                // Glow lichen on the walls and the ceiling, not the floor.
                put(i, yy, j, Deco::GlowLichen { faces: faces & !1 });
            }
            _ => {}
        }
    }
}

/// The thicknesses of a column of pointed dripstone `n` long, from its base
/// to its tip (`merged`: its tip meets another's), as the game builds them.
pub fn column(n: i32, merged: bool) -> Vec<Thickness> {
    let mut out = Vec::new();
    if n >= 3 {
        out.push(Thickness::Base);
        for _ in 0..n - 3 {
            out.push(Thickness::Middle);
        }
    }
    if n >= 2 {
        out.push(Thickness::Frustum);
    }
    if n >= 1 {
        out.push(if merged { Thickness::TipMerge } else { Thickness::Tip });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dripstone_thickens_toward_its_base() {
        use Thickness::*;
        assert_eq!(column(1, false), [Tip]);
        assert_eq!(column(2, false), [Frustum, Tip]);
        assert_eq!(column(3, true), [Base, Frustum, TipMerge]);
        assert_eq!(column(5, false), [Base, Middle, Middle, Frustum, Tip]);
        assert!(column(0, false).is_empty());
    }

    #[test]
    fn every_decoration_has_a_state() {
        let all = Deco::all();
        let states: std::collections::HashSet<String> = all.iter().map(|d| d.state()).collect();
        assert_eq!(states.len(), all.len(), "one state each");
        assert!(Deco::GlowLichen { faces: 0b100100 }.state().contains("north=true,south=false,west=false,east=true"));
        assert_eq!(Deco::all().iter().filter(|d| d.entity().is_some()).count(), 3);
    }
}
