//! End-to-end export checks. The exported region files are read back with
//! `fastanvil`, an independent Anvil implementation, and compared block by
//! block with what the world model says each column contains.

use fastanvil::biome::Biome;
use fastanvil::{Chunk, JavaChunk, Region};
use massif_anvil::{Era, ExportError, ExportOptions, Progress, Target, export};
use massif_core::compose::{Materializer, TileStack};
use massif_core::coords::local_index;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::level::LevelKind;
use massif_core::ops::{self, Anchor, Surface};
use massif_core::objects::{ObjectDef, ObjectLayerDef, transform_state};
use massif_core::registry::{BiomeId, CoverDef, TerrainId};
use massif_core::{Brush, BrushShape, Profile, TilePos, World};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

fn hard(radius: f32) -> Brush {
    Brush {
        shape: BrushShape::Circle,
        radius,
        profile: Profile::Constant,
        strength: 1.0,
        rotation: 0.0,
        mask: None,
        filter: None,
    }
}

/// A 256×256 map around the origin with one of everything:
/// a grass plateau at (-40, -40), a sandy basin under water at (40, 40),
/// a floating sand slab at (-40, 40) and a mossy cave at (40, -40).
fn build_world() -> World {
    let mut w = generate(&GenParams {
        tiles_x: 2,
        tiles_z: 2,
        landform: Landform::Hills,
        seed: 7,
        ..GenParams::default()
    });
    // Block for block as the world model says: no ores (see
    // `ores_are_laid_in_the_rock`).
    w.meta.ores.on = false;
    let terrain = |w: &World, name: &str| w.registry.terrain_by_name(name).unwrap();
    let (grass, sand, moss) = (terrain(&w, "Grass"), terrain(&w, "Sand"), terrain(&w, "Moss"));
    let lush = w.registry.biome_by_id("minecraft:lush_caves").unwrap();
    let mut h = History::default();

    h.begin("ground", 0);
    {
        let mut e = h.edit(&mut w, 0);
        ops::set_height(&mut e, &hard(12.0), -40.0, -40.0, 100.0, Surface::Top);
        ops::paint_terrain(&mut e, &hard(12.0), -40.0, -40.0, grass, 1);
        ops::set_height(&mut e, &hard(12.0), 40.0, 40.0, 50.0, Surface::Top);
        ops::paint_terrain(&mut e, &hard(12.0), 40.0, 40.0, sand, 2);
        ops::set_height(&mut e, &hard(12.0), 40.0, -40.0, 80.0, Surface::Top);
        ops::paint_terrain(&mut e, &hard(12.0), 40.0, -40.0, grass, 3);
        ops::set_height(&mut e, &hard(12.0), -40.0, 40.0, 70.0, Surface::Top);
    }
    h.commit();

    h.begin("levels", 0);
    let island = h.edit(&mut w, 0).structure().add_level("Island", LevelKind::Solid);
    let cave = h.edit(&mut w, 0).structure().add_level("Cave", LevelKind::Carve);
    h.commit();

    h.begin("slabs", 0);
    {
        let mut e = h.edit(&mut w, island);
        ops::stamp_slab(&mut e, &hard(8.0), -40.0, 40.0, Anchor::Absolute(150.0), 10.0);
        ops::paint_terrain(&mut e, &hard(20.0), -40.0, 40.0, sand, 4);
    }
    {
        let mut e = h.edit(&mut w, cave);
        ops::stamp_slab(&mut e, &hard(8.0), 40.0, -40.0, Anchor::Ground(-20.0), 8.0);
        ops::paint_terrain(&mut e, &hard(20.0), 40.0, -40.0, moss, 5);
        ops::paint_biome(&mut e, &hard(20.0), 40.0, -40.0, lush, 6);
    }
    h.commit();

    // A round cave sculpted in 3D inside the plateau, and a ball built high
    // over the hills (an overhang all around).
    h.begin("sculpt", 0);
    massif_core::voxel::sculpt(&mut h.edit(&mut w, 0), &hard(6.0), [-40.5, 70.5, -40.5], 1.0, massif_core::voxel::SculptMode::Remove);
    massif_core::voxel::sculpt(&mut h.edit(&mut w, 0), &hard(5.0), [10.5, 150.5, 60.5], 1.0, massif_core::voxel::SculptMode::Add);
    h.commit();
    w
}

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("massif-anvil-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// The folder of the overworld's regions: `region`, or where 26.1 keeps it.
fn region_dir(dir: &Path) -> PathBuf {
    let split = dir.join("dimensions/minecraft/overworld/region");
    if split.is_dir() { split } else { dir.join("region") }
}

/// A chunk as fastanvil reads it: the block states of 26.3 (`id` and
/// `properties`) under the names they had before (`Name` and `Properties`),
/// which fastanvil knows.
fn as_fastanvil_reads(data: &[u8]) -> Vec<u8> {
    use fastnbt::Value;
    let mut root: Value = fastnbt::from_bytes(data).unwrap();
    let Value::Compound(chunk) = &mut root else { panic!("a chunk is a compound") };
    if !matches!(chunk.get("DataVersion"), Some(Value::Int(v)) if *v >= 5006) {
        return data.to_vec();
    }
    if let Some(Value::List(sections)) = chunk.get_mut("sections") {
        for section in sections {
            let Value::Compound(section) = section else { continue };
            let Some(Value::Compound(states)) = section.get_mut("block_states") else { continue };
            let Some(Value::List(palette)) = states.get_mut("palette") else { continue };
            for entry in palette {
                let Value::Compound(entry) = entry else { continue };
                if let Some(id) = entry.remove("id") {
                    entry.insert("Name".into(), id);
                }
                if let Some(props) = entry.remove("properties") {
                    entry.insert("Properties".into(), props);
                }
            }
        }
    }
    fastnbt::to_bytes(&root).unwrap()
}

/// Reads blocks back from an exported world folder.
struct Reader {
    dir: PathBuf,
    chunks: HashMap<(i32, i32), JavaChunk>,
    /// Chunks of older versions are read too.
    any_version: bool,
}

impl Reader {
    fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
            chunks: HashMap::new(),
            any_version: false,
        }
    }

    fn chunk(&mut self, x: i32, z: i32) -> &JavaChunk {
        let (cx, cz) = (x >> 4, z >> 4);
        let dir = self.dir.clone();
        self.chunks.entry((cx, cz)).or_insert_with(|| {
            let path = region_dir(&dir).join(format!("r.{}.{}.mca", cx >> 5, cz >> 5));
            let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let mut region = Region::from_stream(file).unwrap();
            let data = region
                .read_chunk((cx & 31) as usize, (cz & 31) as usize)
                .unwrap()
                .unwrap_or_else(|| panic!("chunk ({cx}, {cz}) is missing"));
            match JavaChunk::from_bytes(&as_fastanvil_reads(&data)).unwrap() {
                chunk @ JavaChunk::Post18(_) => chunk,
                chunk if self.any_version => chunk,
                _ => panic!("chunk ({cx}, {cz}) was not read as a 1.18+ chunk"),
            }
        })
    }

    fn block(&mut self, x: i32, y: i32, z: i32) -> String {
        let chunk = self.chunk(x, z);
        chunk
            .block((x & 15) as usize, y as isize, (z & 15) as usize)
            .unwrap_or_else(|| panic!("no block at ({x}, {y}, {z})"))
            .name()
            .to_owned()
    }

    /// The block with its properties: `minecraft:tall_grass|half=upper`.
    fn state(&mut self, x: i32, y: i32, z: i32) -> String {
        let chunk = self.chunk(x, z);
        chunk.block((x & 15) as usize, y as isize, (z & 15) as usize).unwrap_or_else(|| panic!("no block at ({x}, {y}, {z})")).encoded_description().to_owned()
    }

    fn biome(&mut self, x: i32, y: i32, z: i32) -> Biome {
        let chunk = self.chunk(x, z);
        chunk
            .biome((x & 15) as usize, y as isize, (z & 15) as usize)
            .unwrap_or_else(|| panic!("no biome at ({x}, {y}, {z})"))
    }
}

/// What the world model says column `(x, z)` contains, bottom to top.
fn expected_column(world: &World, x: i32, z: i32) -> Vec<String> {
    let dim = &world.dims[0];
    let reg = &world.registry;
    let mat = Materializer::new(reg, world.meta.seed, dim.min_y);
    let stack = TileStack::new(dim, TilePos::containing(x, z), true).unwrap();
    let cell = local_index(x, z);
    let (mut spans, mut fluids) = (Vec::new(), Vec::new());
    stack.spans(cell, &mut spans);
    stack.fluids(cell, &spans, &mut fluids);
    (dim.min_y..dim.max_y)
        .map(|y| {
            if let Some(span) = spans.iter().find(|s| y >= s.y0 && y < s.y1) {
                let submerged = fluids.iter().any(|f| f.0 == span.y1);
                reg.block(mat.block(span, x, y, z, submerged)).name().to_owned()
            } else if fluids.iter().any(|f| y >= f.0 && y < f.1) {
                "minecraft:water".to_owned()
            } else {
                "minecraft:air".to_owned()
            }
        })
        .collect()
}

#[test]
fn exported_blocks_match_the_world_model() {
    let mut world = build_world();
    let dir = temp_dir("blocks");
    let progress = Progress::default();
    let stats = export(&world.snapshot(), &dir, &ExportOptions::default(), &progress).unwrap();

    // 2×2 tiles around the origin: four regions of 8×8 chunks each.
    assert_eq!((stats.regions, stats.chunks), (4, 256));
    assert_eq!(progress.done.load(Ordering::Relaxed), 4);
    assert_eq!(progress.fraction(), 1.0);
    for name in ["r.-1.-1.mca", "r.0.-1.mca", "r.-1.0.mca", "r.0.0.mca"] {
        assert!(dir.join("region").join(name).is_file(), "{name}");
    }
    assert!(dir.join("level.dat").is_file());

    let mut reader = Reader::new(&dir);
    {
        let chunk = reader.chunk(0, 0);
        assert_eq!(chunk.status(), "full");
        assert_eq!(chunk.y_range(), -64..320);
    }

    // The landmarks, spelled out.
    let b = |r: &mut Reader, x, y, z| r.block(x, y, z);
    // Grass plateau at 100.
    assert_eq!(b(&mut reader, -40, 100, -40), "minecraft:air");
    assert_eq!(b(&mut reader, -40, 99, -40), "minecraft:grass_block");
    assert_eq!(b(&mut reader, -40, 98, -40), "minecraft:dirt");
    assert_eq!(b(&mut reader, -40, 60, -40), "minecraft:stone");
    assert_eq!(b(&mut reader, -40, -30, -40), "minecraft:deepslate");
    assert_eq!(b(&mut reader, -40, -64, -40), "minecraft:bedrock");
    // Sandy basin under 13 blocks of water.
    assert_eq!(b(&mut reader, 40, 49, 40), "minecraft:sand");
    assert_eq!(b(&mut reader, 40, 50, 40), "minecraft:water");
    assert_eq!(b(&mut reader, 40, 62, 40), "minecraft:water");
    assert_eq!(b(&mut reader, 40, 63, 40), "minecraft:air");
    // Floating slab: 145..155, sand on top, nothing but air around it.
    assert_eq!(b(&mut reader, -40, 155, 40), "minecraft:air");
    assert_eq!(b(&mut reader, -40, 154, 40), "minecraft:sand");
    assert_eq!(b(&mut reader, -40, 145, 40), "minecraft:stone");
    assert_eq!(b(&mut reader, -40, 144, 40), "minecraft:air");
    assert_eq!(b(&mut reader, -40, 71, 40), "minecraft:air");
    // Cave 20 blocks under ground level 80: air in 56..64, moss on the floor,
    // the roof and the surface above it intact.
    assert_eq!(b(&mut reader, 40, 79, -40), "minecraft:grass_block");
    assert_eq!(b(&mut reader, 40, 64, -40), "minecraft:stone");
    assert_eq!(b(&mut reader, 40, 63, -40), "minecraft:air");
    assert_eq!(b(&mut reader, 40, 56, -40), "minecraft:air");
    assert_eq!(b(&mut reader, 40, 55, -40), "minecraft:moss_block");
    assert_eq!(b(&mut reader, 40, 54, -40), "minecraft:dirt");

    // The sculpted cave: air inside, rock around it, the plateau's top
    // untouched; the built ball floats over the water.
    assert_eq!(b(&mut reader, -41, 70, -41), "minecraft:air");
    assert_eq!(b(&mut reader, -41, 64, -41), "minecraft:air");
    assert_eq!(b(&mut reader, -41, 78, -41), "minecraft:stone");
    assert_eq!(b(&mut reader, -40, 99, -40), "minecraft:grass_block");
    assert_ne!(b(&mut reader, 10, 150, 60), "minecraft:air");
    assert_eq!(b(&mut reader, 10, 144, 60), "minecraft:air");

    // Biomes: automatic on the ground, painted inside the cave only.
    assert_eq!(reader.biome(-40, 99, -40), Biome::Plains);
    assert_eq!(reader.biome(40, 55, 40), Biome::Ocean);
    assert_eq!(reader.biome(40, 60, -40), Biome::LushCaves);
    assert_eq!(reader.biome(40, 100, -40), Biome::Plains);

    // And 300 arbitrary columns, every block from bedrock to the build limit.
    let mut s = 0x1234_5678u64;
    let mut columns = vec![(-40, -40), (40, 40), (-40, 40), (40, -40), (-128, -128), (127, 127), (0, 0), (-1, -1), (-41, -41), (-36, -43), (10, 60), (13, 62)];
    for _ in 0..300 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        columns.push((((s >> 33) % 256) as i32 - 128, ((s >> 45) % 256) as i32 - 128));
    }
    for (x, z) in columns {
        let expected = expected_column(&world, x, z);
        for (i, want) in expected.iter().enumerate() {
            let y = -64 + i as i32;
            let got = reader.block(x, y, z);
            assert_eq!(&got, want, "block at ({x}, {y}, {z})");
        }
    }

    std::fs::remove_dir_all(&dir).unwrap();
}

/// The biome the export writes at block `(x, y, z)`, as the Ores window
/// and the survival check read it.
fn biome_of_block(world: &World, x: i32, y: i32, z: i32) -> u16 {
    let dim = &world.dims[0];
    let auto = massif_core::compose::AutoBiomes::new(&world.registry, world.meta.sea_level, dim.min_y);
    let (sx, sz) = ((x & !3) + 2, (z & !3) + 2);
    let stack = TileStack::new(dim, TilePos::containing(sx, sz), true).unwrap();
    let cell = local_index(sx, sz);
    let (mut spans, mut column) = (Vec::new(), massif_core::compose::BiomeColumn::default());
    stack.spans(cell, &mut spans);
    stack.biome_column(cell, &world.registry, &auto, || spans.last().map(|s| (s.y1, s.terrain)), &mut column);
    column.of_block(y)
}

/// What column `(x, z)` holds with its underground pockets: the world
/// model's blocks, and the pockets' blocks in the rock where the survival
/// check and the cut of the Ores window find them.
fn expected_with_pockets(world: &World, x: i32, z: i32) -> Vec<String> {
    let mut column = expected_column(world, x, z);
    let dim = &world.dims[0];
    let reg = &world.registry;
    let Some(pockets) = massif_core::pockets::Pockets::new(reg, world.meta.seed as u64, dim.min_y) else { return column };
    let mat = Materializer::new(reg, world.meta.seed, dim.min_y);
    let stack = TileStack::new(dim, TilePos::containing(x, z), true).unwrap();
    let cell = local_index(x, z);
    let mut spans = Vec::new();
    stack.spans(cell, &mut spans);
    if let Some(col) = pockets.column(stack.ground().pockets.get(cell), &spans, &mat, x, z) {
        for y in col.low..=col.high {
            if let Some(b) = col.at(x, y, z) {
                column[(y - dim.min_y) as usize] = reg.block(b).name().to_owned();
            }
        }
    }
    column
}

/// What column `(x, z)` holds once the ores are laid: the world model's
/// blocks with its pockets, and the ores where the survival check counts
/// them (over the runs of the columns, as [`massif_core::ores::rock_at`]
/// reads them, in the biomes each ore keeps to).
fn expected_with_ores(world: &World, x: i32, z: i32) -> Vec<String> {
    let mut column = expected_with_pockets(world, x, z);
    let dim = &world.dims[0];
    let reg = &world.registry;
    let mat = Materializer::new(reg, world.meta.seed, dim.min_y);
    let hosts = massif_core::ores::hosts(reg);
    let pockets = massif_core::pockets::Pockets::new(reg, world.meta.seed as u64, dim.min_y);
    let placer = massif_core::ores::Placer::new(&world.meta.ores, world.meta.seed).unwrap();
    let (cx, cz) = (x >> 4, z >> 4);
    let mut runs: HashMap<(i32, i32), (Vec<massif_core::Span>, u16)> = HashMap::new();
    for bz in cz * 16..cz * 16 + 16 {
        for bx in cx * 16..cx * 16 + 16 {
            let stack = TileStack::new(dim, TilePos::containing(bx, bz), true).unwrap();
            let mut spans = Vec::new();
            stack.spans(local_index(bx, bz), &mut spans);
            runs.insert((bx, bz), (spans, stack.ground().pockets.get(local_index(bx, bz))));
        }
    }
    placer.chunk_over(
        cx,
        cz,
        |bx, by, bz| {
            let (spans, n) = &runs[&(bx, bz)];
            let col = pockets.as_ref().and_then(|p| p.column(*n, spans, &mat, bx, bz));
            massif_core::ores::rock_at(spans, &mat, &hosts, col.as_ref(), bx, by, bz, dim.max_y)
        },
        |bx, by, bz| biome_of_block(world, bx, by, bz),
        |b, (_, host)| {
            if (b.x, b.z) == (x, z) {
                let (stone, deep) = massif_core::ores::ore_blocks(world.meta.ores.ores[b.ore as usize].mineral);
                let name = if host == massif_core::ores::Host::Deepslate { deep } else { stone };
                column[(b.y - dim.min_y) as usize] = name.split('[').next().unwrap().to_owned();
            }
        },
    );
    column
}

/// With ores on, as a new map has them: every block of 200 columns is the
/// world model's or the ore the survival check counts there; ores are
/// found at their heights, never at the top of a column.
#[test]
fn ores_are_laid_in_the_rock() {
    let mut world = build_world();
    world.meta.ores = Default::default();
    let dir = temp_dir("ores");
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut reader = Reader::new(&dir);
    let mut s = 0x9e37_79b9u64;
    let (mut ores, mut deep_diamonds) = (0, 0);
    for _ in 0..200 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let (x, z) = (((s >> 33) % 256) as i32 - 128, ((s >> 45) % 256) as i32 - 128);
        let expected = expected_with_ores(&world, x, z);
        for (i, want) in expected.iter().enumerate() {
            let y = -64 + i as i32;
            let got = reader.block(x, y, z);
            assert_eq!(&got, want, "block at ({x}, {y}, {z})");
            if got.ends_with("_ore") {
                ores += 1;
                assert_ne!(reader.block(x, y + 1, z), "minecraft:air", "an ore in the open at ({x}, {y}, {z})");
                deep_diamonds += (got == "minecraft:deepslate_diamond_ore") as u32;
            }
        }
    }
    assert!(ores > 200 && deep_diamonds > 0, "{ores} ores, {deep_diamonds} diamonds in deepslate");
    std::fs::remove_dir_all(&dir).unwrap();

    // Before deepslate (1.16): plain ores where deepslate ones would be.
    let dir = temp_dir("ores-old");
    let options = ExportOptions { target: Target::release("1.16.5").unwrap(), ..ExportOptions::default() };
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut reader = Reader::new(&dir);
    reader.any_version = true;
    let names: std::collections::HashSet<String> = (-128..128).step_by(7).flat_map(|x| (-128..128).step_by(7).map(move |z| (x, z))).flat_map(|(x, z)| (0..60).map(move |y| (x, y, z))).map(|(x, y, z)| reader.block(x, y, z)).collect();
    assert!(names.iter().any(|n| n.ends_with("_ore")), "{names:?}");
    assert!(!names.iter().any(|n| n.contains("deepslate")), "{names:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Ores kept to some biomes, or out of some, are laid only where the
/// biome written there lets them in, and where the Ores window and the
/// survival check expect them; a choice that lets every biome in writes
/// the same bytes as none.
/// A region file's bytes but for its table of the times its chunks were
/// written: two exports a second apart differ there.
fn region_bytes(path: &std::path::Path) -> Vec<u8> {
    let mut bytes = std::fs::read(path).unwrap();
    bytes[4096..8192].fill(0);
    bytes
}

#[test]
fn ores_keep_to_their_biomes() {
    let mut world = build_world();
    world.meta.ores = Default::default();
    let desert = world.registry.biome_by_id("minecraft:desert").unwrap();
    // Desert painted over the west of the map, the rest on automatic.
    let mut h = History::default();
    h.begin("desert", 0);
    ops::paint_biome(&mut h.edit(&mut world, 0), &hard(110.0), -128.0, 0.0, desert, 1);
    h.commit();
    let ore = |w: &World, m: massif_core::analysis::Mineral| w.meta.ores.ores.iter().position(|o| o.mineral == m).unwrap();
    let (diamond, coal) = (ore(&world, massif_core::analysis::Mineral::Diamond), ore(&world, massif_core::analysis::Mineral::Coal));

    let plain = temp_dir("ores-biomes-plain");
    export(&world.snapshot(), &plain, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut open = world.clone();
    open.meta.ores.ores[diamond].biomes.except = Some(Vec::new());
    let open_dir = temp_dir("ores-biomes-open");
    export(&open.snapshot(), &open_dir, &ExportOptions::default(), &Progress::default()).unwrap();
    for entry in std::fs::read_dir(plain.join("region")).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(region_bytes(&plain.join("region").join(&name)) == region_bytes(&open_dir.join("region").join(&name)), "{name:?} differs");
    }
    std::fs::remove_dir_all(&open_dir).unwrap();
    std::fs::remove_dir_all(&plain).unwrap();

    world.meta.ores.ores[diamond].biomes.only = Some(vec![desert.0]);
    world.meta.ores.ores[coal].biomes.except = Some(vec![desert.0]);
    let dir = temp_dir("ores-biomes");
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut reader = Reader::new(&dir);
    let mut s = 0x2545_f491u64;
    let mut found = HashMap::new();
    for _ in 0..240 {
        s = s.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let (x, z) = (((s >> 33) % 256) as i32 - 128, ((s >> 45) % 256) as i32 - 128);
        let expected = expected_with_ores(&world, x, z);
        for (i, want) in expected.iter().enumerate() {
            let y = -64 + i as i32;
            let got = reader.block(x, y, z);
            assert_eq!(&got, want, "block at ({x}, {y}, {z})");
            let in_desert = reader.biome(x, y, z) == Biome::Desert;
            assert_eq!(in_desert, biome_of_block(&world, x, y, z) == desert.0, "the biome at ({x}, {y}, {z})");
            if got.ends_with("diamond_ore") {
                assert!(in_desert, "a diamond out of the desert at ({x}, {y}, {z})");
            }
            if got.ends_with("coal_ore") {
                assert!(!in_desert, "coal in the desert at ({x}, {y}, {z})");
            }
            if got.ends_with("_ore") {
                *found.entry((got, in_desert)).or_insert(0) += 1;
            }
        }
    }
    let count = |name: &str, desert: bool| found.iter().filter(|((n, d), _)| n.ends_with(name) && *d == desert).map(|(_, c)| *c).sum::<u32>();
    assert!(count("diamond_ore", true) > 0 && count("coal_ore", false) > 0, "{found:?}");
    assert!(count("iron_ore", true) > 0 && count("iron_ore", false) > 0, "iron everywhere: {found:?}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The same world, written for versions of every era: each is read back
/// as that era's chunks, with the blocks the version has.
#[test]
fn every_era_gets_its_own_chunks() {
    let mut world = build_world();
    let snapshot = world.snapshot();
    for name in ["26.3", "1.21.4", "1.19.2", "1.17.1", "1.16.5", "1.15.2", "1.14.4", "1.12.2", "1.8.9"] {
        let target = Target::release(name).unwrap();
        let dir = temp_dir(&format!("era-{name}"));
        let options = ExportOptions {
            target: target.clone(),
            ..ExportOptions::default()
        };
        let stats = export(&snapshot, &dir, &options, &Progress::default()).unwrap();
        assert_eq!((stats.regions, stats.chunks), (4, 256), "{name}");
        let mut reader = Reader::new(&dir);
        reader.any_version = true;
        let era = target.era();
        {
            let chunk = reader.chunk(0, 0);
            match (era, chunk) {
                (Era::Modern, JavaChunk::Post18(_)) => {
                    assert_eq!(chunk.status(), target.full_status(), "{name}");
                    assert_eq!(chunk.y_range(), -64..320, "{name}");
                }
                // Sections of air are left out: the range is what is there.
                (Era::Flat14 | Era::Flat15 | Era::Flat16, JavaChunk::Pre18(_)) => assert_eq!(chunk.y_range().start, 0, "{name}"),
                (Era::Numbered, JavaChunk::Pre13(_)) => {}
                _ => panic!("{name}: the chunk was not read as its era's"),
            }
        }
        let mut b = |x, y, z| reader.block(x, y, z);
        // The grass plateau at 100, down to the bedrock: at −64 from 1.18,
        // at 0 before.
        // (Before 1.13 the block of grass was plainly called grass.)
        let grass = if era == Era::Numbered { "minecraft:grass" } else { "minecraft:grass_block" };
        assert_eq!(b(-40, 100, -40), "minecraft:air", "{name}");
        assert_eq!(b(-40, 99, -40), grass, "{name}");
        assert_eq!(b(-40, 98, -40), "minecraft:dirt", "{name}");
        assert_eq!(b(-40, 60, -40), "minecraft:stone", "{name}");
        if era == Era::Modern {
            assert_eq!(b(-40, -30, -40), "minecraft:deepslate", "{name}");
            assert_eq!(b(-40, -64, -40), "minecraft:bedrock", "{name}");
        } else {
            assert_eq!(b(-40, 0, -40), "minecraft:bedrock", "{name}");
            // Just above it, deepslate where the version has it (1.17),
            // stone before.
            let rock = b(-40, 1, -40);
            assert!(rock == "minecraft:stone" || (target.data_version >= 2724 && rock == "minecraft:deepslate"), "{name}: {rock}");
        }
        // The sandy basin under its water, the floating slab over the void.
        assert_eq!(b(40, 49, 40), "minecraft:sand", "{name}");
        assert_eq!(b(40, 62, 40), "minecraft:water", "{name}");
        assert_eq!(b(40, 63, 40), "minecraft:air", "{name}");
        assert_eq!(b(-40, 154, 40), "minecraft:sand", "{name}");
        assert_eq!(b(-40, 145, 40), "minecraft:stone", "{name}");
        assert_eq!(b(-40, 144, 40), "minecraft:air", "{name}");
        // Moss came with 1.17: grass stands in for it before.
        let moss = if target.data_version >= 2724 { "minecraft:moss_block" } else { grass };
        assert_eq!(b(40, 55, -40), moss, "{name}");
        assert_eq!(reader.biome(-40, 99, -40), Biome::Plains, "{name}");
        assert_eq!(reader.biome(40, 55, 40), Biome::Ocean, "{name}");

        // level.dat says which version it is for, and how the world goes on.
        let mut raw = Vec::new();
        std::io::Read::read_to_end(&mut flate2::read::GzDecoder::new(std::fs::File::open(dir.join("level.dat")).unwrap()), &mut raw).unwrap();
        let level: fastnbt::Value = fastnbt::from_bytes(&raw).unwrap();
        let fastnbt::Value::Compound(root) = &level else { panic!("{name}: level.dat") };
        let fastnbt::Value::Compound(data) = &root["Data"] else { panic!("{name}: Data") };
        if target.data_version > 0 {
            // Every version's own.
            assert_eq!(data["DataVersion"], fastnbt::Value::Int(target.data_version), "{name}");
        } else {
            assert!(!data.contains_key("DataVersion"), "{name}: versions before 1.9 have no data version");
        }
        match era {
            // From 26.1, in a file of its own.
            Era::Modern if target.splits_level() => {
                assert!(!data.contains_key("WorldGenSettings"), "{name}");
                assert!(dir.join("data/minecraft/world_gen_settings.dat").is_file(), "{name}");
                assert!(!dir.join("region").exists(), "{name}");
            }
            Era::Modern | Era::Flat16 => assert!(data.contains_key("WorldGenSettings"), "{name}"),
            _ => assert_eq!(data["generatorName"], fastnbt::Value::String("default".into()), "{name}"),
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

#[test]
fn test_exports_datapacks_and_settings() {
    let mut world = build_world();
    let dir = temp_dir("test-area");
    // A data pack folder to copy in.
    let pack = temp_dir("pack");
    std::fs::create_dir_all(pack.join("data")).unwrap();
    std::fs::write(pack.join("pack.mcmeta"), r#"{"pack":{"pack_format":9,"description":"test"}}"#).unwrap();
    let options = ExportOptions {
        area: Some((TilePos::new(0, 0), TilePos::new(0, 0))),
        datapacks: vec![pack.clone()],
        spawn: Some([5, 90, 6]),
        difficulty: massif_anvil::Difficulty::Peaceful,
        border: massif_anvil::WorldBorder {
            center: [64.0, 64.0],
            size: 256.0,
        },
        ..ExportOptions::default()
    };
    let stats = export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    // One tile: 8 × 8 chunks in one region.
    assert_eq!((stats.regions, stats.chunks), (1, 64));
    assert!(dir.join("region/r.0.0.mca").is_file());
    assert!(!dir.join("region/r.-1.-1.mca").exists());
    let name = pack.file_name().unwrap();
    assert!(dir.join("datapacks").join(name).join("pack.mcmeta").is_file());
    let raw = {
        use std::io::Read;
        let bytes = std::fs::read(dir.join("level.dat")).unwrap();
        let mut raw = Vec::new();
        flate2::read::GzDecoder::new(&bytes[..]).read_to_end(&mut raw).unwrap();
        raw
    };
    let root: HashMap<String, fastnbt::Value> = fastnbt::from_bytes(&raw).unwrap();
    let fastnbt::Value::Compound(data) = &root["Data"] else { panic!() };
    assert_eq!(data["SpawnY"], fastnbt::Value::Int(90));
    assert_eq!(data["Difficulty"], fastnbt::Value::Byte(0));
    assert_eq!(data["BorderSize"], fastnbt::Value::Double(256.0));
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&pack).unwrap();
}

#[test]
fn export_refuses_to_clobber_and_can_be_cancelled() {
    let mut world = build_world();
    let snapshot = world.snapshot();
    let dir = temp_dir("guard");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("precious.txt"), "do not delete").unwrap();

    let options = ExportOptions::default();
    let err = export(&snapshot, &dir, &options, &Progress::default()).unwrap_err();
    assert!(matches!(err, ExportError::Exists(_)), "{err}");
    assert!(!dir.join("region").exists());

    // Overwriting replaces the regions but leaves unrelated files alone.
    let overwrite = ExportOptions {
        overwrite: true,
        ..options.clone()
    };
    std::fs::create_dir_all(dir.join("region")).unwrap();
    std::fs::write(dir.join("region/r.9.9.mca"), "stale").unwrap();
    export(&snapshot, &dir, &overwrite, &Progress::default()).unwrap();
    assert!(dir.join("precious.txt").is_file());
    assert!(!dir.join("region/r.9.9.mca").exists());
    assert!(dir.join("region/r.0.0.mca").is_file());

    let progress = Progress::default();
    progress.cancel.store(true, Ordering::Relaxed);
    let err = export(&snapshot, &dir, &overwrite, &progress).unwrap_err();
    assert!(matches!(err, ExportError::Cancelled), "{err}");

    std::fs::remove_dir_all(&dir).unwrap();
}

/// A map taller than the game's limits: its sections go as high as it does,
/// and a data pack gives the world that height.
/// The world of [`build_world`] with covers painted: tall grass all over
/// the plateau, and under the floating slab where it cannot grow; grass
/// and seagrass in the basin; sugar cane 1 to 3 high on the slab's sand;
/// grass over the cave.
fn covered_world() -> World {
    let mut w = build_world();
    let reg = &mut w.registry;
    let (grass, seagrass) = (reg.cover_by_name("Grass").unwrap(), reg.cover_by_name("Seagrass").unwrap());
    let tall = reg.add_cover(CoverDef::new("Tall grass only", [0; 3], &[("minecraft:tall_grass", 1)], 1.0));
    let mut cane = CoverDef::new("Cane", [0; 3], &[("minecraft:sugar_cane", 1)], 1.0);
    cane.height = [1, 3];
    cane.soil_only = false;
    let cane = reg.add_cover(cane);
    let mut h = History::default();
    h.begin("covers", 0);
    {
        let mut e = h.edit(&mut w, 0);
        ops::paint_cover(&mut e, &hard(6.0), -40.0, -40.0, tall, 1);
        ops::paint_cover(&mut e, &hard(6.0), -40.0, 40.0, tall, 2);
        ops::paint_cover(&mut e, &hard(6.0), 40.0, 40.0, seagrass, 3);
        ops::paint_cover(&mut e, &hard(2.0), 44.0, 40.0, grass, 4);
        ops::paint_cover(&mut e, &hard(5.0), 40.0, -40.0, grass, 6);
    }
    ops::paint_cover(&mut h.edit(&mut w, 1), &hard(6.0), -40.0, 40.0, cane, 5);
    h.commit();
    w
}

/// Underground pockets lie in the rock under the ground's strata, between
/// their heights, about as much of it as their share. With ores on, every
/// block of the columns painted is the one the survival check and the cut
/// of the Ores window count there: the pockets' blocks, and the ores in
/// the rock, also in pockets of rock an ore takes (andesite), never in
/// gravel.
#[test]
fn underground_pockets_lie_in_the_rock() {
    let mut world = build_world();
    let reg = &mut world.registry;
    let (gravel, andesite) = (reg.intern_block("minecraft:gravel", [136, 126, 118]), reg.intern_block("minecraft:andesite", [136, 136, 136]));
    let loose = reg.add_pocket(massif_core::pockets::PocketDef::new("Gravel", [0; 3], &[(gravel, 1)], 0.3, 4.0, 40, 90));
    let rock = reg.add_pocket(massif_core::pockets::PocketDef::new("Andesite", [0; 3], &[(andesite, 1)], 0.5, 6.0, -64, 90));
    let mut h = History::default();
    h.begin("pockets", 0);
    ops::paint_pockets(&mut h.edit(&mut world, 0), &hard(8.0), -40.0, -40.0, loose, 1);
    ops::paint_pockets(&mut h.edit(&mut world, 0), &hard(8.0), 40.0, 40.0, rock, 2);
    h.commit();
    let dir = temp_dir("pockets");
    let options = ExportOptions { target: Target::release("1.21.11").unwrap(), ..ExportOptions::default() };
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    let (mut pockets, mut stone) = (0, 0);
    for x in -45..=-35 {
        for z in -45..=-35 {
            for y in 30..100 {
                match r.block(x, y, z).as_str() {
                    "minecraft:gravel" => {
                        assert!((40..=90).contains(&y), "gravel at {y}, out of its heights");
                        pockets += 1;
                    }
                    "minecraft:stone" if (40..=90).contains(&y) => stone += 1,
                    _ => {}
                }
            }
            // The strata of the ground stay: grass on top.
            assert_eq!(r.block(x, 99, z), "minecraft:grass_block");
        }
    }
    // About its share: the window is small for blobs of four blocks (the
    // share over a large volume is checked in massif-core).
    let share = pockets as f32 / (pockets + stone) as f32;
    assert!((0.12..0.48).contains(&share), "{pockets} gravel, {stone} stone: {share}");
    // Outside what is painted, the rock is whole.
    assert!((40..90).all(|y| r.block(-40, y, -55) != "minecraft:gravel"));
    std::fs::remove_dir_all(&dir).unwrap();

    // With ores on: block for block as the survival check counts them.
    world.meta.ores = Default::default();
    let dir = temp_dir("pockets-ores");
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    let mut in_andesite = 0;
    for (x0, z0) in [(-40, -40), (40, 40)] {
        for x in x0 - 6..=x0 + 6 {
            for z in z0 - 6..=z0 + 6 {
                let (bare, expected) = (expected_with_pockets(&world, x, z), expected_with_ores(&world, x, z));
                for (i, want) in expected.iter().enumerate() {
                    let y = -64 + i as i32;
                    let got = r.block(x, y, z);
                    assert_eq!(&got, want, "block at ({x}, {y}, {z})");
                    if got.ends_with("_ore") {
                        assert_ne!(bare[i], "minecraft:gravel", "an ore in gravel at ({x}, {y}, {z})");
                        in_andesite += (bare[i] == "minecraft:andesite") as u32;
                    }
                }
            }
        }
    }
    assert!(in_andesite > 0, "ores take pockets of andesite too");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A sub-canopy layer grows in the air the base layer leaves: painted over
/// the same blocks as oaks, hazels change none of the oaks' blocks, and
/// stand in the air between them. A tree size painted chooses the objects
/// written: small oaks where small trees are marked.
#[test]
fn sub_canopy_grows_under_the_canopy() {
    let pole = |name: &str, h: u16, state: &str| ObjectDef::new(name, [1, h, 1], vec!["minecraft:air".into(), state.into()], vec![1; h as usize]);
    let bush = ObjectDef::new("Hazel", [3, 2, 3], vec!["minecraft:air".into(), "minecraft:azalea_leaves[distance=1,persistent=true,waterlogged=false]".into()], vec![1; 18]);
    let mut world = build_world();
    let first = world.registry.objects.len() as u32;
    world.registry.objects.extend([pole("Oak", 10, "minecraft:oak_log[axis=y]"), pole("Small oak", 4, "minecraft:birch_log[axis=y]"), bush]);
    let mut oaks = ObjectLayerDef::new("Oaks", &[first, first + 1], &world.registry);
    (oaks.soil_only, oaks.density, oaks.spacing, oaks.rotate, oaks.mirror) = (false, 1.0, 4, false, false);
    let oaks = world.registry.add_object_layer(oaks);
    let mut hazel = ObjectLayerDef::new("Hazel", &[first + 2], &world.registry);
    (hazel.soil_only, hazel.density, hazel.spacing, hazel.understory) = (false, 1.0, 3, true);
    let hazel = world.registry.add_object_layer(hazel);
    let mut only_oaks = world.clone();
    let paint = |w: &mut World, under: bool| {
        let mut h = History::default();
        h.begin("forest", 0);
        {
            let mut e = h.edit(w, 0);
            ops::paint_objects(&mut e, &hard(10.0), -40.0, -40.0, oaks, 1);
            if under {
                ops::paint_understory(&mut e, &hard(10.0), -40.0, -40.0, hazel, 2);
            }
        }
        h.commit();
    };
    paint(&mut only_oaks, false);
    paint(&mut world, true);
    let export_to = |w: &mut World, name: &str| {
        let dir = temp_dir(name);
        let options = ExportOptions { target: Target::release("1.21.11").unwrap(), ..ExportOptions::default() };
        export(&w.snapshot(), &dir, &options, &Progress::default()).unwrap();
        dir
    };
    let (a, b) = (export_to(&mut only_oaks, "canopy-oaks"), export_to(&mut world, "canopy-hazel"));
    let (mut ra, mut rb) = (Reader::new(&a), Reader::new(&b));
    let (mut logs, mut leaves) = (0, 0);
    for x in -50..=-30 {
        for z in -50..=-30 {
            for y in 100..112 {
                let (was, is) = (ra.block(x, y, z), rb.block(x, y, z));
                if was != "minecraft:air" {
                    assert_eq!(is, was, "the oaks' block at ({x}, {y}, {z})");
                    logs += 1;
                } else if is == "minecraft:azalea_leaves" {
                    leaves += 1;
                }
            }
        }
    }
    assert!(logs > 100 && leaves > 100, "{logs} blocks of oaks, {leaves} of hazels");
    std::fs::remove_dir_all(&a).unwrap();
    std::fs::remove_dir_all(&b).unwrap();

    // Small trees marked: the small oak only (4 high, birch logs here).
    let mut h = History::default();
    h.begin("sizes", 0);
    ops::paint_tree_size(&mut h.edit(&mut world, 0), &hard(12.0), -40.0, -40.0, massif_core::tile::SIZE_BUSHES, 3);
    h.commit();
    let c = export_to(&mut world, "canopy-small");
    let mut r = Reader::new(&c);
    let (mut small, mut tall) = (0, 0);
    for x in -48..=-32 {
        for z in -48..=-32 {
            if r.block(x, 100, z) == "minecraft:birch_log" {
                small += 1;
            }
            if r.block(x, 100, z) == "minecraft:oak_log" {
                tall += 1;
            }
        }
    }
    assert!(small > 5 && tall == 0, "{small} small oaks, {tall} tall ones");
    std::fs::remove_dir_all(&c).unwrap();
}

/// Frost and Lava come with every new map: unpainted, they change nothing,
/// and the map exports the same bytes as one made before them, which has
/// neither. Annotations are never exported.
#[test]
fn unpainted_frost_and_annotations_leave_the_export_unchanged() {
    let mut world = build_world();
    let (grass, seagrass) = (world.registry.cover_by_name("Grass").unwrap(), world.registry.cover_by_name("Seagrass").unwrap());
    let mut h = History::default();
    h.begin("covers", 0);
    {
        let mut e = h.edit(&mut world, 0);
        ops::paint_cover(&mut e, &hard(6.0), -40.0, -40.0, grass, 1);
        ops::paint_cover(&mut e, &hard(6.0), 40.0, 40.0, seagrass, 2);
    }
    h.commit();
    let mut before = world.clone();
    before.registry.covers.retain(|c| !c.frost && !c.lava);
    assert_eq!(before.registry.covers.len() + 2, world.registry.covers.len());
    let plain = temp_dir("frost-unpainted-before");
    export(&before.snapshot(), &plain, &ExportOptions::default(), &Progress::default()).unwrap();
    h.begin("marks", 0);
    ops::paint_annotation(&mut h.edit(&mut world, 0), &hard(30.0), -40.0, -40.0, 14, 3);
    ops::paint_annotation(&mut h.edit(&mut world, 0), &hard(30.0), 40.0, 40.0, 4, 4);
    h.commit();
    let dir = temp_dir("frost-unpainted-marked");
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut regions = 0;
    for entry in std::fs::read_dir(plain.join("region")).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(region_bytes(&plain.join("region").join(&name)) == region_bytes(&dir.join("region").join(&name)), "{name:?} differs");
        regions += 1;
    }
    assert!(regions > 0);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&plain).unwrap();
}

/// Frost, as WorldPainter's: snow on the ground, and the top of the water
/// frozen, in the game as on the map; and Lava, the water turned to lava.
#[test]
fn frost_lays_snow_and_freezes_the_water() {
    let mut world = build_world();
    let frost = world.registry.cover_by_name("Frost").unwrap();
    assert!(world.registry.cover(frost).unwrap().frost);
    let lava = world.registry.cover_by_name("Lava").unwrap();
    assert!(world.registry.cover(lava).unwrap().lava);
    let mut h = History::default();
    h.begin("frost", 0);
    {
        let mut e = h.edit(&mut world, 0);
        ops::paint_cover(&mut e, &hard(6.0), -40.0, -40.0, frost, 1);
        ops::paint_cover(&mut e, &hard(6.0), 40.0, 40.0, frost, 2);
        ops::paint_cover(&mut e, &hard(1.5), 31.0, 40.0, lava, 3);
    }
    h.commit();
    let dir = temp_dir("frost");
    let options = ExportOptions {
        target: Target::release("1.21.11").unwrap(),
        ..ExportOptions::default()
    };
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    // Snow on the plateau.
    assert_eq!(r.block(-40, 99, -40), "minecraft:grass_block");
    assert_eq!(r.state(-40, 100, -40), "minecraft:snow|layers=1");
    // The basin's water frozen at its top only, the sand under it bare.
    for x in 37..=43 {
        for z in 37..=43 {
            assert_eq!(r.block(x, 49, z), "minecraft:sand");
            assert_eq!(r.block(x, 50, z), "minecraft:water", "at ({x}, 50, {z})");
            assert_eq!(r.block(x, 61, z), "minecraft:water");
            assert_eq!(r.block(x, 62, z), "minecraft:ice", "at ({x}, 62, {z})");
            assert_eq!(r.block(x, 63, z), "minecraft:air");
        }
    }
    // Beyond the brush, the water is open.
    assert_eq!(r.block(40, 62, 50), "minecraft:water");
    // Under the lava, the whole of the water is lava, the sand under it.
    assert_eq!(r.block(31, 49, 40), "minecraft:sand");
    for y in 50..63 {
        assert_eq!(r.block(31, y, 40), "minecraft:lava", "at (31, {y}, 40)");
    }
    assert_eq!(r.block(31, 63, 40), "minecraft:air");
    std::fs::remove_dir_all(&dir).unwrap();

    // Before 1.13, ice is block 79.
    let dir = temp_dir("frost-old");
    let old = ExportOptions {
        target: Target::release("1.12.2").unwrap(),
        ..ExportOptions::default()
    };
    export(&world.snapshot(), &dir, &old, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    r.any_version = true;
    assert_eq!(r.block(40, 62, 40), "minecraft:ice");
    assert_eq!(r.block(40, 61, 40), "minecraft:water");
    assert_eq!(r.block(-40, 100, -40), "minecraft:snow");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn covers_grow_where_they_are_painted() {
    let mut world = covered_world();
    let dir = temp_dir("covers");
    let options = ExportOptions {
        target: Target::release("1.21.11").unwrap(),
        ..ExportOptions::default()
    };
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    // Tall grass on the plateau, in two halves.
    assert_eq!(r.block(-40, 99, -40), "minecraft:grass_block");
    assert_eq!(r.state(-40, 100, -40), "minecraft:tall_grass|half=lower");
    assert_eq!(r.state(-40, 101, -40), "minecraft:tall_grass|half=upper");
    assert_eq!(r.block(-40, 102, -40), "minecraft:air");
    // Not under the slab, whose top is the column's: cane grows there, 1
    // to 3 high, on sand.
    assert_eq!(r.block(-40, 71, 40), "minecraft:air");
    let mut heights = std::collections::BTreeSet::new();
    for x in -43..=-37 {
        for z in 37..=43 {
            assert_eq!(r.block(x, 154, z), "minecraft:sand");
            let h = (155..160).take_while(|&y| r.block(x, y, z) == "minecraft:sugar_cane").count();
            assert!((1..=3).contains(&h), "cane {h} high at ({x}, {z})");
            heights.insert(h);
        }
    }
    assert_eq!(heights.len(), 3, "{heights:?}");
    // Seagrass grows in the basin, under the water; grass does not.
    let mut seagrass = 0;
    for x in 36..=44 {
        for z in 36..=44 {
            assert_eq!(r.block(x, 49, z), "minecraft:sand");
            match r.block(x, 50, z).as_str() {
                "minecraft:seagrass" => seagrass += 1,
                "minecraft:tall_seagrass" => {
                    seagrass += 1;
                    assert_eq!(r.state(x, 51, z), "minecraft:tall_seagrass|half=upper");
                }
                other => assert_eq!(other, "minecraft:water", "at ({x}, 50, {z})"),
            }
        }
    }
    assert!(seagrass > 8, "{seagrass} seagrass");
    // Grass over the cave: about half the columns, short grass mostly.
    let over_cave: Vec<String> = (37..=43).flat_map(|x| (-43..=-37).map(move |z| (x, z))).map(|(x, z)| r.block(x, 80, z)).collect();
    let short = over_cave.iter().filter(|b| *b == "minecraft:short_grass").count();
    let bare = over_cave.iter().filter(|b| *b == "minecraft:air").count();
    assert!(short > 10 && bare > 10 && short + bare + 12 > over_cave.len(), "{over_cave:?}");
    // Nothing else changed: the plateau around and the cave below.
    assert_eq!(r.block(-48, 100, -40), "minecraft:air");
    assert_eq!(r.block(40, 55, -40), "minecraft:moss_block");
    assert_eq!(r.block(40, 56, -40), "minecraft:air");
    std::fs::remove_dir_all(&dir).unwrap();

    // Versions without some of the plants: short grass of old was grass,
    // and before 1.13 plants were numbered, tall ones as double plants
    // (and the reader calls block 31, grass the plant, tall grass).
    for (name, tall, short) in [("1.19.2", "minecraft:tall_grass", "minecraft:grass"), ("1.12.2", "minecraft:double_plant", "minecraft:tall_grass")] {
        let target = Target::release(name).unwrap();
        let dir = temp_dir(&format!("covers-{name}"));
        let options = ExportOptions {
            target,
            ..ExportOptions::default()
        };
        export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
        let mut r = Reader::new(&dir);
        r.any_version = true;
        assert_eq!(r.block(-40, 100, -40), tall, "{name}");
        assert_eq!(r.block(-40, 101, -40), tall, "{name}");
        let over_cave = (37..=43).flat_map(|x| (-43..=-37).map(move |z| (x, z))).filter(|&(x, z)| r.block(x, 80, z) == short).count();
        assert!(over_cave > 10, "{name}: {over_cave} of {short}");
        assert_eq!(r.block(-40, 155, 40), if name == "1.12.2" { "minecraft:reeds" } else { "minecraft:sugar_cane" }, "{name}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A little oak: a trunk 3 high on a block of dirt, leaves around its
/// top, a vine on the east of its trunk.
fn little_oak() -> ObjectDef {
    let (w, h, l) = (5usize, 6usize, 5usize);
    let palette = vec![
        "minecraft:air".to_owned(),
        "minecraft:dirt".to_owned(),
        "minecraft:oak_log[axis=x]".to_owned(),
        "minecraft:oak_leaves[distance=1,persistent=true,waterlogged=false]".to_owned(),
        "minecraft:vine[east=false,north=false,south=false,up=false,west=true]".to_owned(),
    ];
    let mut blocks = vec![0u16; w * h * l];
    let at = |x: usize, y: usize, z: usize| x + z * w + y * w * l;
    blocks[at(2, 0, 2)] = 1;
    for y in 1..4 {
        blocks[at(2, y, 2)] = 2;
    }
    for y in 3..6 {
        for z in 1..4 {
            for x in 0..5 {
                if blocks[at(x, y, z)] == 0 {
                    blocks[at(x, y, z)] = 3;
                }
            }
        }
    }
    blocks[at(3, 2, 2)] = 4;
    ObjectDef::new("Little oak", [w as u16, h as u16, l as u16], palette, blocks)
}

/// `minecraft:vine[east=true,…]` and `minecraft:vine|east=true,…` (as the
/// reader gives it, leaving out whether a block is waterlogged) name the
/// same block.
fn same_state(want: &str, got: &str) -> bool {
    let parse = |s: &str, sep: char| {
        let (name, props) = s.split_once(sep).unwrap_or((s, ""));
        let mut props: Vec<String> = props.trim_end_matches(']').split(',').filter(|p| !p.is_empty() && !p.starts_with("waterlogged=")).map(str::to_owned).collect();
        props.sort();
        (name.to_owned(), props)
    };
    parse(want, '[') == parse(got, '|')
}

/// Objects of a layer painted across the border of two tiles stand whole,
/// turned and mirrored, in every chunk they reach.
#[test]
fn objects_stand_across_chunks_and_tiles() {
    let mut w = generate(&GenParams {
        tiles_x: 4,
        tiles_z: 2,
        landform: Landform::Hills,
        seed: 7,
        ..GenParams::default()
    });
    let k = w.registry.objects.len() as u32;
    w.registry.objects.push(little_oak());
    let mut layer = ObjectLayerDef::new("Oaks", &[k], &w.registry);
    layer.spacing = 12;
    layer.density = 1.0;
    layer.soil_only = false;
    let n = w.registry.add_object_layer(layer);
    let mut h = History::default();
    h.begin("oaks", 0);
    {
        let mut e = h.edit(&mut w, 0);
        ops::set_height(&mut e, &hard(40.0), 128.0, -40.0, 90.0, Surface::Top);
        ops::paint_objects(&mut e, &hard(30.0), 128.0, -40.0, n, 1);
    }
    h.commit();
    let snapshot = w.snapshot();
    let def = &w.registry.objects[k as usize];
    let dim = &w.dims[0];
    let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
    let mut placements = Vec::new();
    for pos in [TilePos::new(0, -1), TilePos::new(1, -1)] {
        let stack = TileStack::new(dim, pos, true).unwrap();
        placements.extend(massif_core::objects::tile_placements(&w.registry, &mat, w.meta.seed as u64, false, &stack, pos));
    }
    assert!(placements.len() >= 12, "{} oaks", placements.len());
    // Some across the two tiles, some across two chunks, every way round.
    assert!(placements.iter().any(|p| p.bounds(def).x0 < 128 && p.bounds(def).x1 > 128), "{placements:?}");
    assert!(placements.iter().any(|p| {
        let b = p.bounds(def);
        b.x0.div_euclid(16) != (b.x1 - 1).div_euclid(16) && (b.x0 >= 128 || b.x1 <= 128)
    }));
    assert!(placements.iter().any(|p| p.mirror) && placements.iter().any(|p| p.turn % 2 == 1));

    let dir = temp_dir("objects");
    let options = ExportOptions {
        target: Target::release("1.21.11").unwrap(),
        ..ExportOptions::default()
    };
    export(&snapshot, &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    for p in &placements {
        assert_eq!(p.y, 90, "{p:?}");
        for by in 0..6 {
            for bz in 0..5 {
                for bx in 0..5 {
                    let k = def.at(bx, by, bz);
                    let (x, y, z) = p.world(def, bx as i32, by as i32, bz as i32);
                    let got = r.state(x, y, z);
                    if by == 0 {
                        // The dirt goes into the ground, which it leaves as
                        // it is.
                        assert_eq!(r.block(x, y, z), "minecraft:grass_block", "{p:?}");
                        continue;
                    }
                    let want = if k == 0 { "minecraft:air".to_owned() } else { transform_state(&def.palette[k as usize], p.turn, p.mirror) };
                    assert!(same_state(&want, &got), "{p:?} block {bx} {by} {bz} at ({x}, {y}, {z}): want {want}, got {got}");
                }
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();

    // Before 1.13, numbered: the crowns are there. (The reader cannot
    // name logs lying along x or z before 1.13: axis bits 4 and 8, as the
    // game writes them, are beyond what it expects.)
    let dir = temp_dir("objects-1.12");
    let options = ExportOptions {
        target: Target::release("1.12.2").unwrap(),
        ..ExportOptions::default()
    };
    export(&snapshot, &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    r.any_version = true;
    for p in &placements {
        let (x, y, z) = p.world(def, 2, 5, 2);
        assert_eq!(r.block(x, y, z), "minecraft:oak_leaves", "{p:?}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn tall_maps_get_their_build_limits() {
    let mut world = generate(&GenParams {
        tiles_x: 1,
        tiles_z: 1,
        landform: Landform::Mountains,
        relief: 1200.0,
        min_y: -64,
        max_y: 1024,
        seed: 3,
        ..GenParams::default()
    });
    assert_eq!((world.dims[0].min_y, world.dims[0].max_y), (-64, 1024));
    let tallest = world.dims[0].ground().view(TilePos::new(0, 0)).map(|t| (0..128 * 128).map(|i| t.top.get(i)).fold(0.0f32, f32::max)).unwrap();
    assert!(tallest > 400.0, "mountains past 320: {tallest}");
    world.meta.spawn = [0, 200, 0];
    let dir = temp_dir("tall");
    let options = ExportOptions {
        target: Target::release("1.21.11").unwrap(),
        ..ExportOptions::default()
    };
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let json = std::fs::read_to_string(dir.join("datapacks/massif_build_limits/data/minecraft/dimension_type/overworld.json")).unwrap();
    assert!(json.contains("\"min_y\": -64") && json.contains("\"height\": 1088"), "{json}");
    assert!(dir.join("datapacks/massif_build_limits/pack.mcmeta").exists());
    // The highest column is written up there.
    let mut reader = Reader::new(&dir);
    let top = (-64..1024).rev().find(|&y| (0..128).step_by(4).any(|x| (0..128).step_by(4).any(|z| reader.block(x, y, z) != "minecraft:air")));
    assert!(top.is_some_and(|y| y > 320), "blocks above 320: {top:?}");
    // The game's own limits need no pack.
    let mut plain = generate(&GenParams {
        tiles_x: 1,
        tiles_z: 1,
        ..GenParams::default()
    });
    let dir2 = temp_dir("plain");
    export(&plain.snapshot(), &dir2, &options, &Progress::default()).unwrap();
    assert!(!dir2.join("datapacks/massif_build_limits").exists());
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// The overworld's type in the height pack is each version's own, its
/// limits changed: 26.1 wants `has_ender_dragon_fight` (without it the
/// game would not load the pack, and the world would open cut at 320),
/// 1.21.11 has the sky, fog, music and beds in attributes, 1.21.6 has the
/// clouds' height. Every one is JSON with the limits asked.
#[test]
fn the_height_pack_has_each_versions_overworld() {
    use massif_anvil::export::overworld_type;
    for (name, wants, lacks) in [
        ("1.18.2", &["ultrawarm", "effects", "bed_works"][..], &["monster_spawn_light_level", "cloud_height", "attributes"][..]),
        ("1.20.4", &["monster_spawn_light_level", "effects"][..], &["cloud_height", "attributes"][..]),
        ("1.21.8", &["cloud_height", "effects"][..], &["attributes"][..]),
        ("1.21.11", &["attributes", "timelines"][..], &["has_ender_dragon_fight", "effects", "ultrawarm"][..]),
        ("26.1", &["attributes", "has_ender_dragon_fight", "default_clock"][..], &["effects"][..]),
        ("26.2", &["has_ender_dragon_fight"][..], &[][..]),
        ("26.3", &["has_ender_dragon_fight", "minecraft:gameplay/straw_bed_rule"][..], &[][..]),
    ] {
        let json = overworld_type(&Target::release(name).unwrap(), -128, 1152);
        let value: serde_json::Value = serde_json::from_str(&json).unwrap_or_else(|e| panic!("{name}: {e}\n{json}"));
        assert_eq!((value["min_y"].as_i64(), value["height"].as_i64(), value["logical_height"].as_i64()), (Some(-128), Some(1152), Some(1152)), "{name}");
        for key in wants {
            assert!(json.contains(&format!("\"{key}\"")), "{name} wants {key}");
        }
        for key in lacks {
            assert!(!json.contains(&format!("\"{key}\"")), "{name} has no {key}");
        }
    }
}

#[test]
fn unusual_worlds_are_handled() {
    // An empty map has nothing to export.
    let empty = World::new(Default::default());
    let dir = temp_dir("empty");
    let err = export(&empty, &dir, &ExportOptions::default(), &Progress::default()).unwrap_err();
    assert!(matches!(err, ExportError::Empty), "{err}");

    // A terrain pointing at a block that does not exist is reported, not
    // exported as garbage.
    let mut world = build_world();
    world.registry.terrains[1].strata[0].blocks[0].0 = massif_core::BlockId(9999);
    let err = export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default())
        .unwrap_err();
    assert!(matches!(err, ExportError::Unsupported(_)), "{err}");

    // Unknown terrain and biome ids in tiles fall back instead of panicking.
    let mut world = build_world();
    let mut h = History::default();
    h.begin("junk", 0);
    {
        let mut e = h.edit(&mut world, 0);
        ops::paint_terrain(&mut e, &hard(6.0), 0.0, 0.0, TerrainId(60_000), 1);
        ops::paint_biome(&mut e, &hard(6.0), 0.0, 0.0, BiomeId(60_000), 1);
    }
    h.commit();
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut reader = Reader::new(&dir);
    let top = (-64..320).rev().find(|&y| reader.block(0, y, 0) != "minecraft:air").unwrap();
    assert!(reader.block(0, top, 0).starts_with("minecraft:"));
    assert_eq!(reader.biome(0, top, 0), Biome::Plains);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A map dressed from the packs: cherry trees of the tree pack, a mix of
/// rock in beds and a preset of flowers, exported for today's game and for
/// 1.19.2, which has no cherry trees: there they are birches, their axis
/// and their leaves' persistence kept.
#[test]
fn packs_are_exported() {
    use massif_core::packs::{MATERIAL_MIXES, PLANT_PRESETS};
    use massif_core::treepack::TreeKind;
    let mut w = generate(&GenParams {
        tiles_x: 4,
        tiles_z: 2,
        landform: Landform::Hills,
        seed: 7,
        ..GenParams::default()
    });
    let trees = TreeKind::Cherry.install(&mut w.registry);
    let beds = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut w.registry);
    let flowers = PLANT_PRESETS.iter().find(|p| p.name == "Tulip field").unwrap().install(&mut w.registry);
    let mut h = History::default();
    h.begin("packs", 0);
    {
        let mut e = h.edit(&mut w, 0);
        ops::set_height(&mut e, &hard(60.0), 128.0, -40.0, 90.0, Surface::Top);
        ops::paint_objects(&mut e, &hard(30.0), 128.0, -40.0, trees, 1);
        ops::paint_cover(&mut e, &hard(30.0), 128.0, -40.0, flowers, 2);
        ops::paint_terrain(&mut e, &hard(12.0), 180.0, -40.0, beds, 3);
    }
    h.commit();
    let snapshot = w.snapshot();
    let dim = &w.dims[0];
    let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
    let mut placements = Vec::new();
    for pos in [TilePos::new(0, -1), TilePos::new(1, -1)] {
        let stack = TileStack::new(dim, pos, true).unwrap();
        placements.extend(massif_core::objects::tile_placements(&w.registry, &mat, w.meta.seed as u64, false, &stack, pos));
    }
    assert!(placements.len() >= 4, "{} trees", placements.len());
    let rock: Vec<String> = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().blocks.iter().map(|b| format!("minecraft:{}", b.0)).collect();
    for (version, log, leaves) in [("1.21.11", "minecraft:cherry_log", "minecraft:cherry_leaves"), ("1.19.2", "minecraft:birch_log", "minecraft:birch_leaves")] {
        let dir = temp_dir(&format!("packs-{version}"));
        let options = ExportOptions {
            target: Target::release(version).unwrap(),
            ..ExportOptions::default()
        };
        export(&snapshot, &dir, &options, &Progress::default()).unwrap();
        let mut r = Reader::new(&dir);
        let mut leaves_seen = 0;
        for p in &placements {
            let def = &w.registry.objects[p.object as usize];
            let [ax, _, az] = def.anchor.map(|a| a as i32);
            let (x, y, z) = p.world(def, ax, 0, az);
            let foot = r.state(x, y, z);
            assert!(foot.starts_with(log) && foot.contains("axis=y"), "{version}: {foot}");
            for by in 0..def.size[1] as usize {
                for bz in 0..def.size[2] as usize {
                    for bx in 0..def.size[0] as usize {
                        let k = def.at(bx, by, bz);
                        if k != 0 && def.palette[k as usize].contains("_leaves") {
                            let (x, y, z) = p.world(def, bx as i32, by as i32, bz as i32);
                            let got = r.state(x, y, z);
                            if got.starts_with(leaves) {
                                assert!(got.contains("persistent=true"), "{version}: {got}");
                                leaves_seen += 1;
                            }
                        }
                    }
                }
            }
        }
        assert!(leaves_seen > 100, "{version}: {leaves_seen} leaves");
        // The beds: every block down the column is of the mix.
        for y in 70..90 {
            let b = r.block(180, y, -40);
            assert!(rock.contains(&b), "{version}: {b} at {y}");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// A selection changes nothing in an export unless it is asked to keep to
/// it; asked, only the tiles the selection holds are written.
#[test]
fn an_export_can_keep_to_the_selection() {
    use massif_core::selection::{Selection, Shape};
    let mut world = build_world();
    let plain = temp_dir("sel-plain");
    export(&world.snapshot(), &plain, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut h = History::default();
    let shape = Selection::shape(&Shape::Rect(massif_core::BlockRect::new(-60, 10, 60, 60))).feather(6);
    assert!(h.select(&mut world, 0, "Select", Some(shape)));
    let ignored = temp_dir("sel-ignored");
    export(&world.snapshot(), &ignored, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut names: Vec<_> = std::fs::read_dir(plain.join("region")).unwrap().map(|e| e.unwrap().file_name()).collect();
    names.sort();
    assert!(!names.is_empty());
    for name in &names {
        assert!(region_bytes(&plain.join("region").join(name)) == region_bytes(&ignored.join("region").join(name)), "{name:?}");
    }
    let only = temp_dir("sel-only");
    let options = ExportOptions {
        selection_only: true,
        ..ExportOptions::default()
    };
    let stats = export(&world.snapshot(), &only, &options, &Progress::default()).unwrap();
    // The rectangle lies across tiles (-1, 0) and (0, 0): two tiles of
    // 8 × 8 chunks, of the map's four.
    assert_eq!(stats.chunks, 2 * 64);
    for dir in [plain, ignored, only] {
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// Caves carved by the generator are exported as air, their lakes as lava
/// in a world of water: every block of every chunk they changed.
#[test]
fn generated_caves_are_exported() {
    use massif_core::caves::{CaveSettings, carve};
    use massif_core::voxel::Fluid;
    let mut world = build_world();
    // A seed whose four tiles hold a cavern (they are rarer than tunnels).
    let carved = (1..200)
        .map(|seed| {
            let settings = CaveSettings { seed, density: 1.0, openings: 0.3, low: -50, high: 60, lake: Some(Fluid::Lava), lake_level: -10, ..CaveSettings::for_dimension(&world.dims[0], world.meta.sea_level) };
            carve(&world.dims[0], &settings, &massif_core::auto::Steps::default())
        })
        .find(|c| c.flooded > 0)
        .expect("a seed with a cavern");
    assert!(carved.blocks > 1000 && carved.flooded > 0, "{} carved, {} flooded", carved.blocks, carved.flooded);
    let mut h = History::default();
    h.begin("caves", 0);
    h.edit(&mut world, 0).set_voxels(carved.chunks.clone());
    h.commit();
    let dir = temp_dir("caves");
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut reader = Reader::new(&dir);
    let (mut air, mut lava) = (0, 0);
    for (pos, chunk) in &carved.chunks {
        let (ox, oy, oz) = pos.origin();
        for ly in 0..16 {
            for lz in 0..16 {
                for lx in 0..16 {
                    let (x, y, z) = (ox + lx, oy + ly, oz + lz);
                    if chunk.solid(lx, ly, lz) {
                        continue;
                    }
                    let got = reader.block(x, y, z);
                    match chunk.fluid(lx, ly, lz) {
                        Some(Fluid::Lava) => {
                            assert_eq!(got, "minecraft:lava", "({x}, {y}, {z})");
                            lava += 1;
                        }
                        _ => {
                            assert!(got == "minecraft:air" || got == "minecraft:water" || got == "minecraft:cave_air", "({x}, {y}, {z}): {got}");
                            air += 1;
                        }
                    }
                }
            }
        }
    }
    assert_eq!(lava, carved.flooded);
    assert!(air > 0);
    // And the whole of a few carved columns, block by block, as the model
    // composes them (lakes aside).
    for (pos, _) in carved.chunks.iter().step_by(7).take(20) {
        let (x, _, z) = pos.origin();
        let expected = expected_column(&world, x + 5, z + 9);
        for (i, want) in expected.iter().enumerate() {
            let y = -64 + i as i32;
            let got = reader.block(x + 5, y, z + 9);
            if got != "minecraft:lava" {
                assert_eq!(&got, want, "block at ({}, {y}, {})", x + 5, z + 9);
            }
        }
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A map painted with a pack someone shared exports the very same blocks
/// as one painted with the layers the pack was made of: cherry trees, a mix
/// of rock in beds and a preset of flowers, written to a .massifpack,
/// read back, installed into a new map and painted the same way.
#[test]
fn a_shared_pack_exports_as_the_original() {
    use massif_core::pack_file::Pack;
    use massif_core::packs::{MATERIAL_MIXES, PLANT_PRESETS};
    use massif_core::treepack::TreeKind;
    let params = GenParams {
        tiles_x: 4,
        tiles_z: 2,
        landform: Landform::Hills,
        seed: 11,
        ..GenParams::default()
    };
    let paint = |w: &mut World, trees: u16, beds: TerrainId, flowers: u16| {
        let mut h = History::default();
        h.begin("pack", 0);
        {
            let mut e = h.edit(w, 0);
            ops::paint_objects(&mut e, &hard(40.0), 128.0, -40.0, trees, 1);
            ops::paint_cover(&mut e, &hard(40.0), 200.0, -40.0, flowers, 2);
            ops::paint_terrain(&mut e, &hard(30.0), 330.0, -40.0, beds, 3);
        }
        h.commit();
    };
    let mut original = generate(&params);
    let trees = TreeKind::Cherry.install(&mut original.registry);
    let beds = MATERIAL_MIXES.iter().find(|m| m.name == "Grey cliff strata").unwrap().install(&mut original.registry);
    let flowers = PLANT_PRESETS.iter().find(|p| p.name == "Tulip field").unwrap().install(&mut original.registry);
    paint(&mut original, trees, beds, flowers);

    let content = Pack::gather(&original.registry, &[flowers], &[beds.0], &[trees]);
    let pack = Pack { name: "Shared".into(), author: "Tester".into(), description: String::new(), made_with: "0.6.0".into(), preview: None, content };
    let pack = Pack::from_bytes(&pack.to_bytes()).unwrap();
    let mut shared = generate(&params);
    let trees = pack.install_layer(0, &mut shared.registry);
    let beds = pack.install_terrain(0, &mut shared.registry);
    let flowers = pack.install_cover(0, &mut shared.registry);
    paint(&mut shared, trees, beds, flowers);

    let options = ExportOptions {
        target: Target::release("1.21.11").unwrap(),
        ..ExportOptions::default()
    };
    let (a, b) = (temp_dir("pack-original"), temp_dir("pack-shared"));
    export(&original.snapshot(), &a, &options, &Progress::default()).unwrap();
    export(&shared.snapshot(), &b, &options, &Progress::default()).unwrap();
    let regions = |dir: &Path| {
        let mut files: Vec<PathBuf> = std::fs::read_dir(dir.join("region")).unwrap().map(|e| e.unwrap().path()).collect();
        files.sort();
        files
    };
    let (ra, rb) = (regions(&a), regions(&b));
    assert!(!ra.is_empty());
    assert_eq!(ra.iter().map(|p| p.file_name()).collect::<Vec<_>>(), rb.iter().map(|p| p.file_name()).collect::<Vec<_>>());
    for (x, y) in ra.iter().zip(&rb) {
        assert!(region_bytes(x) == region_bytes(y), "{} differs", x.display());
    }
    // And the pack's things are in it: cherry logs where the trees stand.
    let mut r = Reader::new(&b);
    let dim = &shared.dims[0];
    let mat = Materializer::new(&shared.registry, shared.meta.seed, dim.min_y);
    let mut placements = Vec::new();
    for pos in [TilePos::new(0, -1), TilePos::new(1, -1)] {
        let stack = TileStack::new(dim, pos, true).unwrap();
        placements.extend(massif_core::objects::tile_placements(&shared.registry, &mat, shared.meta.seed as u64, false, &stack, pos));
    }
    let logs = placements
        .iter()
        .filter(|p| {
            let def = &shared.registry.objects[p.object as usize];
            let [ax, _, az] = def.anchor.map(|v| v as i32);
            let (x, y, z) = p.world(def, ax, 0, az);
            r.state(x, y, z).starts_with("minecraft:cherry_log")
        })
        .count();
    assert!(logs >= 3, "{logs} of {} trees", placements.len());
    std::fs::remove_dir_all(&a).unwrap();
    std::fs::remove_dir_all(&b).unwrap();
}

/// Save points are not exported: the map with points of earlier states of
/// it, one of them restored and undone, exports the same bytes as without.
#[test]
fn save_points_leave_the_export_unchanged() {
    use massif_core::versions::SavePoint;
    let mut world = build_world();
    let plain = temp_dir("points-plain");
    export(&world.snapshot(), &plain, &ExportOptions::default(), &Progress::default()).unwrap();

    let mut with = world.clone();
    let mut h = History::default();
    let point = SavePoint::new(&mut with, "before", 1);
    with.points.push(point);
    h.begin("raise", 0);
    ops::raise(&mut h.edit(&mut with, 0), &hard(20.0), 0.0, 0.0, 9.0, Surface::Top);
    h.commit();
    let point = SavePoint::new(&mut with, "after", 2);
    with.points.push(point);
    h.undo(&mut with);
    let then = with.points[1].world.clone();
    h.restore_point(&mut with, &then, "Restore");
    h.undo(&mut with);
    assert_eq!(with.points.len(), 2);
    let dir = temp_dir("points-with");
    export(&with.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    let mut regions = 0;
    for entry in std::fs::read_dir(plain.join("region")).unwrap() {
        let name = entry.unwrap().file_name();
        assert!(region_bytes(&plain.join("region").join(&name)) == region_bytes(&dir.join("region").join(&name)), "{name:?} differs");
        regions += 1;
    }
    assert!(regions > 0);
    assert_eq!(std::fs::read_dir(dir.join("region")).unwrap().count(), regions);
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::remove_dir_all(&plain).unwrap();
}

/// A map of things later versions added or renamed: ground of mud (1.19),
/// tuff bricks (1.21), pale moss (1.21.4) and sulfur (26.2); plants under
/// grass's old name, pink petals (1.20), leaf litter (1.21.5) and golden
/// dandelions (26.1); posts of chain (renamed iron chain in 1.21.9), cherry
/// planks and poplar logs (26.3); cherry groves and pale gardens.
fn newer_world() -> World {
    let mut w = generate(&GenParams {
        tiles_x: 2,
        tiles_z: 2,
        landform: Landform::Hills,
        seed: 7,
        ..GenParams::default()
    });
    w.meta.ores.on = false;
    let reg = &mut w.registry;
    let ground: Vec<_> = ["minecraft:mud", "minecraft:tuff_bricks", "minecraft:pale_moss_block", "minecraft:sulfur"].iter().map(|b| (reg.intern_block(b, [90; 3]), 1)).collect();
    let mut terrain = reg.terrains[reg.terrain_by_name("Grass").unwrap().index()].clone();
    terrain.name = "Newer ground".into();
    terrain.strata = vec![massif_core::registry::Stratum { blocks: ground, depth: (3, 3), mix: Default::default() }];
    terrain.submerged_top = None;
    reg.terrains.push(terrain);
    let terrain = TerrainId((reg.terrains.len() - 1) as _);
    let mut plants = CoverDef::new(
        "Newer plants",
        [0; 3],
        &[("minecraft:grass", 1), ("minecraft:pink_petals[facing=north,flower_amount=2]", 1), ("minecraft:leaf_litter[facing=east,segment_amount=1]", 1), ("minecraft:golden_dandelion", 1)],
        1.0,
    );
    plants.soil_only = false;
    let plants = reg.add_cover(plants);
    let k = reg.objects.len() as u32;
    let palette = ["minecraft:air", "minecraft:chain[axis=y,waterlogged=false]", "minecraft:cherry_planks", "minecraft:poplar_log[axis=y]"].map(str::to_owned).to_vec();
    reg.objects.push(ObjectDef::new("Posts", [1, 3, 1], palette, vec![2, 1, 3]));
    let mut layer = ObjectLayerDef::new("Posts", &[k], reg);
    layer.spacing = 6;
    layer.density = 1.0;
    layer.soil_only = false;
    let posts = reg.add_object_layer(layer);
    let (cherry, pale) = (reg.biome_by_id("minecraft:cherry_grove").unwrap(), reg.biome_by_id("minecraft:pale_garden").unwrap());
    let mut h = History::default();
    h.begin("newer", 0);
    {
        let mut e = h.edit(&mut w, 0);
        ops::set_height(&mut e, &hard(200.0), 0.0, 0.0, 90.0, Surface::Top);
        ops::paint_terrain(&mut e, &hard(200.0), 0.0, 0.0, terrain, 1);
        ops::paint_cover(&mut e, &hard(200.0), 0.0, 0.0, plants, 2);
        ops::paint_objects(&mut e, &hard(40.0), 0.0, 0.0, posts, 3);
        ops::paint_biome(&mut e, &hard(40.0), -64.0, -64.0, cherry, 4);
        ops::paint_biome(&mut e, &hard(40.0), 64.0, 64.0, pale, 5);
    }
    h.commit();
    w
}

/// Every chunk's NBT as fastnbt reads it, every region read whole.
fn chunk_values(dir: &Path) -> Vec<HashMap<String, fastnbt::Value>> {
    let mut chunks = Vec::new();
    for entry in std::fs::read_dir(region_dir(dir)).unwrap() {
        let mut region = Region::from_stream(std::fs::File::open(entry.unwrap().path()).unwrap()).unwrap();
        for c in region.iter() {
            let c = c.unwrap();
            // An independent reader takes it as a chunk of today's shape.
            assert!(matches!(JavaChunk::from_bytes(&as_fastanvil_reads(&c.data)).unwrap(), JavaChunk::Post18(_)));
            let fastnbt::Value::Compound(root) = fastnbt::from_bytes(&c.data).unwrap() else { panic!("a chunk is a compound") };
            chunks.push(root);
        }
    }
    chunks
}

/// From 1.18.2 to 26.3, each version gets chunks as it writes them itself:
/// its own data version and status, the keys it reads, and only blocks and
/// biomes it has, under the names it gives them, so the game loads them
/// without upgrading anything.
#[test]
fn chunks_are_each_versions_own() {
    let mut world = newer_world();
    let snapshot = world.snapshot();
    let versions = ["1.18.2", "1.19.2", "1.19.4", "1.20.1", "1.20.4", "1.20.6", "1.21.1", "1.21.4", "1.21.5", "1.21.8", "1.21.11", "26.1.2", "26.2", "26.3"];
    for name in versions {
        let target = Target::release(name).unwrap();
        let dir = temp_dir(&format!("native-{name}"));
        let options = ExportOptions { target: target.clone(), ..ExportOptions::default() };
        export(&snapshot, &dir, &options, &Progress::default()).unwrap();
        let chunks = chunk_values(&dir);
        assert_eq!(chunks.len(), 256, "{name}");
        let mut blocks: HashMap<String, String> = HashMap::new();
        let mut biomes = std::collections::BTreeSet::new();
        for chunk in &chunks {
            let mut keys: Vec<&str> = chunk.keys().map(String::as_str).collect();
            keys.sort();
            assert_eq!(
                keys,
                ["DataVersion", "Heightmaps", "InhabitedTime", "LastUpdate", "Status", "block_entities", "block_ticks", "fluid_ticks", "isLightOn", "sections", "structures", "xPos", "yPos", "zPos"],
                "{name}"
            );
            assert_eq!(chunk["DataVersion"], fastnbt::Value::Int(target.data_version), "{name}");
            assert_eq!(chunk["Status"], fastnbt::Value::String(target.full_status().into()), "{name}");
            let fastnbt::Value::List(sections) = &chunk["sections"] else { panic!("{name}: sections") };
            for section in sections {
                let fastnbt::Value::Compound(section) = section else { panic!() };
                let (fastnbt::Value::Compound(states), fastnbt::Value::Compound(biome)) = (&section["block_states"], &section["biomes"]) else { panic!() };
                let fastnbt::Value::List(palette) = &states["palette"] else { panic!() };
                for entry in palette {
                    let fastnbt::Value::Compound(entry) = entry else { panic!() };
                    let (name_key, properties_key) = target.state_keys();
                    assert!(!entry.keys().any(|k| k != name_key && k != properties_key), "{name}: {entry:?}");
                    let fastnbt::Value::String(id) = &entry[name_key] else { panic!() };
                    let props = match entry.get(properties_key) {
                        Some(fastnbt::Value::Compound(p)) => {
                            let mut p: Vec<String> = p.iter().map(|(k, v)| format!("{k}={}", match v { fastnbt::Value::String(s) => s.as_str(), _ => panic!() })).collect();
                            p.sort();
                            p.join(",")
                        }
                        _ => String::new(),
                    };
                    blocks.insert(id.clone(), props);
                }
                let fastnbt::Value::List(palette) = &biome["palette"] else { panic!() };
                for id in palette {
                    let fastnbt::Value::String(id) = id else { panic!() };
                    biomes.insert(id.clone());
                }
            }
        }
        // Nothing the version lacks.
        for id in blocks.keys() {
            let short = id.strip_prefix("minecraft:").unwrap_or_else(|| panic!("{name}: {id}"));
            assert!(target.has_block(short), "{name} has no {id}");
        }
        for id in &biomes {
            assert_eq!(target.biome(id), id, "{name} has no {id}");
        }
        // Each thing as the version knows it.
        let has = |id: &str| blocks.contains_key(&format!("minecraft:{id}"));
        let at_least = |since: &str| target.data_version >= Target::release(since).unwrap().data_version;
        assert_eq!(has("mud"), at_least("1.19"), "{name}");
        assert_eq!(has("coarse_dirt"), !at_least("1.19"), "{name}: mud before 1.19");
        assert_eq!(has("tuff_bricks"), at_least("1.21"), "{name}");
        assert_eq!(has("pale_moss_block"), at_least("1.21.4"), "{name}");
        assert_eq!(has("moss_block"), !at_least("1.21.4"), "{name}: pale moss before 1.21.4");
        assert_eq!(has("sulfur"), at_least("26.2"), "{name}");
        assert!(has("stone"), "{name}");
        assert_eq!(has("short_grass"), at_least("1.20.3"), "{name}");
        assert_eq!(has("grass"), !at_least("1.20.3"), "{name}");
        assert_eq!(has("pink_petals"), at_least("1.20"), "{name}");
        assert_eq!(has("leaf_litter"), at_least("1.21.5"), "{name}");
        assert_eq!(has("golden_dandelion"), at_least("26.1"), "{name}");
        assert_eq!(has("dandelion"), !at_least("26.1"), "{name}");
        let chain = if at_least("1.21.9") { "iron_chain" } else { "chain" };
        assert_eq!(blocks.get(&format!("minecraft:{chain}")).map(String::as_str), Some("axis=y,waterlogged=false"), "{name}: {chain}");
        assert_eq!(has("cherry_planks"), at_least("1.20"), "{name}");
        assert_eq!(has("birch_planks"), !at_least("1.20"), "{name}: cherry before 1.20");
        assert_eq!(has("poplar_log"), at_least("26.3"), "{name}");
        if !at_least("26.3") {
            assert_eq!(blocks.get("minecraft:birch_log").map(String::as_str), Some("axis=y"), "{name}: poplar before 26.3");
        }
        assert_eq!(biomes.contains("minecraft:cherry_grove"), at_least("1.20"), "{name}");
        assert_eq!(biomes.contains("minecraft:meadow"), !at_least("1.20"), "{name}");
        assert_eq!(biomes.contains("minecraft:pale_garden"), at_least("1.21.4"), "{name}");
        assert_eq!(biomes.contains("minecraft:dark_forest"), !at_least("1.21.4"), "{name}");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// The biome ids written in the palettes of a chunk's sections.
fn biome_ids(dir: &Path, x: i32, z: i32) -> std::collections::BTreeSet<String> {
    use fastnbt::Value;
    let (cx, cz) = (x >> 4, z >> 4);
    let file = std::fs::File::open(region_dir(dir).join(format!("r.{}.{}.mca", cx >> 5, cz >> 5))).unwrap();
    let data = Region::from_stream(file).unwrap().read_chunk((cx & 31) as usize, (cz & 31) as usize).unwrap().unwrap();
    let root: Value = fastnbt::from_bytes(&data).unwrap();
    let Value::Compound(chunk) = root else { panic!("a chunk is a compound") };
    let Some(Value::List(sections)) = chunk.get("sections") else { panic!("no sections") };
    let mut out = std::collections::BTreeSet::new();
    for section in sections {
        let Value::Compound(section) = section else { continue };
        let Some(Value::Compound(biomes)) = section.get("biomes") else { continue };
        let Some(Value::List(palette)) = biomes.get("palette") else { continue };
        out.extend(palette.iter().filter_map(|v| if let Value::String(s) = v { Some(s.clone()) } else { None }));
    }
    out
}

/// A mod's biomes, and a biome typed by hand, go out by their ids into the
/// chunks' biome palettes, as the game reads them with the mod installed:
/// painted, and from a terrain whose biome its mod's profile names, where
/// the biome is left on automatic.
#[test]
fn modded_biomes_are_exported_by_their_ids() {
    let mut world = build_world();
    let reg = &mut world.registry;
    let bayou = reg.intern_biome(massif_core::registry::BiomeDef { group: "Regions Unexplored".into(), grass: Some([90, 110, 60]), ..massif_core::registry::BiomeDef::of_id("regions_unexplored:bayou") });
    let typed = reg.intern_biome(massif_core::registry::BiomeDef::of_id("mypack:misty_glade"));
    assert_eq!(reg.intern_biome(massif_core::registry::BiomeDef::of_id("mypack:misty_glade")), typed, "added once");
    // A terrain calling for a mod's biome, painted on the plateau.
    let grass = reg.terrain_by_name("Grass").unwrap();
    let mut fen = reg.terrain(grass).clone();
    fen.name = "Fen".into();
    fen.biome_hint = Some(reg.intern_biome(massif_core::registry::BiomeDef::of_id("regions_unexplored:fen")));
    reg.terrains.push(fen);
    let fen = TerrainId((reg.terrains.len() - 1) as u16);
    let mut h = History::default();
    h.begin("biomes", 0);
    {
        let mut e = h.edit(&mut world, 0);
        ops::paint_biome(&mut e, &hard(10.0), 100.0, 100.0, bayou, 1);
        ops::paint_biome(&mut e, &hard(10.0), -100.0, 100.0, typed, 2);
        ops::paint_terrain(&mut e, &hard(6.0), -40.0, -40.0, fen, 3);
    }
    h.commit();
    let dir = temp_dir("modded-biomes");
    export(&world.snapshot(), &dir, &ExportOptions::default(), &Progress::default()).unwrap();
    assert!(biome_ids(&dir, 100, 100).contains("regions_unexplored:bayou"), "{:?}", biome_ids(&dir, 100, 100));
    assert!(biome_ids(&dir, -100, 100).contains("mypack:misty_glade"), "{:?}", biome_ids(&dir, -100, 100));
    assert!(biome_ids(&dir, -40, -40).contains("regions_unexplored:fen"), "{:?}", biome_ids(&dir, -40, -40));
    assert!(!biome_ids(&dir, 0, 0).iter().any(|b| !b.starts_with("minecraft:")), "the rest of the map keeps the game's");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// The palettes of an export's chunks: each state as `name[k=v,…]`, its
/// properties sorted (fastanvil leaves some out).
fn palette_set(dir: &Path) -> std::collections::BTreeSet<String> {
    let mut states = std::collections::BTreeSet::new();
    for chunk in chunk_values(dir) {
        let Some(fastnbt::Value::List(sections)) = chunk.get("sections") else { continue };
        for section in sections {
            let fastnbt::Value::Compound(section) = section else { continue };
            let Some(fastnbt::Value::Compound(bs)) = section.get("block_states") else { continue };
            let Some(fastnbt::Value::List(palette)) = bs.get("palette") else { continue };
            for entry in palette {
                let fastnbt::Value::Compound(entry) = entry else { continue };
                let Some(fastnbt::Value::String(name)) = entry.get("Name") else { continue };
                let mut props: Vec<String> = match entry.get("Properties") {
                    Some(fastnbt::Value::Compound(p)) => p.iter().map(|(k, v)| format!("{k}={}", match v { fastnbt::Value::String(s) => s.as_str(), _ => "?" })).collect(),
                    _ => Vec::new(),
                };
                props.sort();
                states.insert(format!("{name}[{}]", props.join(",")));
            }
        }
    }
    states
}

/// Smooth terrain: the top of the land in partial blocks of the block at
/// the surface, by its precise height and its steps — slabs and stairs of
/// the game's stone, a mod's layers for the game's grass, a mod's rock's
/// whole range — waterlogged in water, and nothing for a block without
/// them; a slab chosen by hand with the brush, whatever the block.
#[test]
fn smooth_terrain_follows_the_painted_terrain() {
    use massif_core::registry::{Mix, Stratum, TerrainDef, TerrainId};
    use massif_core::shapes;
    let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, landform: Landform::Hills, seed: 3, ..GenParams::default() });
    world.meta.ores.on = false;
    world.meta.smooth.on = true;
    let stone = world.registry.terrain_by_name("Stone").unwrap();
    let grass = world.registry.terrain_by_name("Grass").unwrap();
    let glow = {
        let reg = &mut world.registry;
        let b = reg.intern_block("minecraft:glowstone", [200, 180, 90]);
        reg.terrains.push(TerrainDef { name: "Glow".into(), group: "Test".into(), color: [200, 180, 90], strata: vec![Stratum { blocks: vec![(b, 1)], depth: (3, 5), mix: Mix::Grain }], submerged_top: None, biome_hint: None });
        TerrainId((reg.terrains.len() - 1) as u16)
    };
    let rock = {
        let reg = &mut world.registry;
        let b = reg.intern_block("conquest:gray_limestone", [128, 128, 128]);
        reg.terrains.push(TerrainDef { name: "Gray limestone".into(), group: "Conquest Reforged".into(), color: [128, 128, 128], strata: vec![Stratum { blocks: vec![(b, 1)], depth: (3, 5), mix: Mix::Grain }], submerged_top: None, biome_hint: None });
        TerrainId((reg.terrains.len() - 1) as u16)
    };
    let positions: Vec<TilePos> = world.dims[0].ground().positions().collect();
    for pos in positions {
        let (ox, oz) = pos.origin();
        let tile = world.dims[0].ground_mut().tile_mut(pos).unwrap();
        for i in 0..massif_core::TILE_AREA {
            let (x, z) = (ox + (i % 128) as i32, oz + (i / 128) as i32);
            // A step up east of x = 0, a notch north of (-1, -5); a pool
            // west of x = -30 level with the land; west of x = -60, a
            // slope whose precise height is 63.6 (five eighths over the
            // top block's bottom at 63).
            let high = x >= 0 || (x, z) == (-1, -6);
            let pool = (-60..-30).contains(&x);
            let h = if high { 65.0 } else if pool { 63.0 } else if x < -60 { 63.6 } else { 64.0 };
            tile.top.set(i, h);
            tile.fluid.set(i, if pool { 64 } else { massif_core::NO_FLUID });
            let t = match z {
                ..0 => stone,
                0..20 => rock,
                20..40 => grass,
                _ => glow,
            };
            tile.terrain.set(i, t.0);
            tile.cover.set(i, 0);
            tile.objects.set(i, 0);
        }
    }
    let props = |p: &str| p.split(',').map(String::from).collect::<Vec<_>>();
    let mods: std::collections::HashMap<String, Vec<String>> = [
        ("conquest:gray_limestone", ""),
        ("conquest:gray_limestone_slab", "layers,type,waterlogged"),
        ("conquest:gray_limestone_stairs", "facing,half,shape,waterlogged"),
        ("conquest:gray_limestone_vertical_slab", "extension_toggle,facing,layer,waterlogged"),
        ("conquest:gray_limestone_corner_slab", "facing,type,waterlogged"),
        ("conquest:grass_block_layer", "layers"),
    ]
    .into_iter()
    .map(|(k, v)| (k.to_owned(), if v.is_empty() { Vec::new() } else { props(v) }))
    .collect();
    let options = ExportOptions { target: Target::release("1.20.1").unwrap(), mod_blocks: std::sync::Arc::new(mods), ..ExportOptions::default() };
    let dir = temp_dir("smooth");
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    let has = |s: &str, parts: &[&str]| parts.iter().all(|p| s.contains(p));
    // Stone at the foot of the step: a stair facing it.
    let s = r.state(-1, 64, -10);
    assert!(has(&s, &["minecraft:stone_stairs", "facing=east", "half=bottom", "shape=straight"]), "{s}");
    // In the inner corner, steps up north and east: an inner stair.
    let s = r.state(-1, 64, -5);
    assert!(has(&s, &["minecraft:stone_stairs", "facing=north", "shape=inner_right"]), "{s}");
    // On top of the step and away from it: nothing.
    assert_eq!(r.block(0, 65, -10), "minecraft:air");
    assert_eq!(r.block(-5, 64, -10), "minecraft:air");
    // Five eighths of stone: a slab in place of the top block.
    assert_eq!(r.block(-70, 63, -10), "minecraft:stone_slab");
    assert_eq!(r.block(-70, 62, -10), "minecraft:stone");
    // At the pool's edge, under water: a stair, waterlogged (see below).
    assert_eq!(r.block(-31, 63, -10), "minecraft:stone_stairs");
    // The game's grass, with the mod's layers: five layers in its place.
    let s = r.state(-70, 63, 25);
    assert!(has(&s, &["conquest:grass_block_layer", "layers=5"]), "{s}");
    assert_eq!(r.block(-1, 64, 25), "conquest:grass_block_layer", "half a block of layers at the step's foot");
    // The mod's rock: its slab by eighths, its stairs.
    let s = r.state(-70, 63, 5);
    assert!(has(&s, &["conquest:gray_limestone_slab", "layers=5", "type=bottom"]), "{s}");
    assert_eq!(r.block(-1, 64, 5), "conquest:gray_limestone_stairs");
    // Glowstone has none: whole blocks.
    assert_eq!(r.block(-70, 63, 50), "minecraft:glowstone");
    assert_eq!(r.block(-1, 64, 50), "minecraft:air");
    let states = palette_set(&dir);
    for want in ["minecraft:stone_stairs[facing=west,half=bottom,shape=straight,waterlogged=true]", "minecraft:stone_slab[type=bottom,waterlogged=false]", "conquest:grass_block_layer[layers=5]"] {
        assert!(states.contains(want), "{want} in {:?}", states.iter().filter(|s| s.contains("slab") || s.contains("stairs") || s.contains("layer")).collect::<Vec<_>>());
    }
    std::fs::remove_dir_all(&dir).unwrap();

    // The map's switch off: nothing; the brush on here and a slab chosen
    // by hand: that slab at the step, whatever the block.
    world.meta.smooth.on = false;
    let oak = world.registry.intern_block("minecraft:oak_slab", [160, 130, 80]);
    let mut h = History::default();
    h.begin("oak", 0);
    ops::paint_slabs(&mut h.edit(&mut world, 0), &hard(30.0), 0.0, 25.0, shapes::value(Some(oak), false), 1);
    h.commit();
    let dir = temp_dir("smooth-oak");
    export(&world.snapshot(), &dir, &options, &Progress::default()).unwrap();
    let mut r = Reader::new(&dir);
    assert_eq!(r.block(-1, 64, 25), "minecraft:oak_stairs", "the chosen slab's stairs at the step");
    assert_eq!(r.block(-1, 64, -10), "minecraft:air", "elsewhere, the map is not smoothed");
    assert_eq!(r.block(-70, 63, -10), "minecraft:stone");
    std::fs::remove_dir_all(&dir).unwrap();
}
