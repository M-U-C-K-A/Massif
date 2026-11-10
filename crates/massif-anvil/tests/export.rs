//! End-to-end export checks. The exported region files are read back with
//! `fastanvil`, an independent Anvil implementation, and compared block by
//! block with what the world model says each column contains.

use fastanvil::biome::Biome;
use fastanvil::{Chunk, JavaChunk, Region};
use massif_anvil::{ExportError, ExportOptions, Progress, export};
use massif_core::compose::{Materializer, TileStack};
use massif_core::coords::local_index;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::level::LevelKind;
use massif_core::ops::{self, Anchor, Surface};
use massif_core::registry::{BiomeId, TerrainId};
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

/// Reads blocks back from an exported world folder.
struct Reader {
    dir: PathBuf,
    chunks: HashMap<(i32, i32), JavaChunk>,
}

impl Reader {
    fn new(dir: &Path) -> Self {
        Self {
            dir: dir.to_owned(),
            chunks: HashMap::new(),
        }
    }

    fn chunk(&mut self, x: i32, z: i32) -> &JavaChunk {
        let (cx, cz) = (x >> 4, z >> 4);
        let dir = self.dir.clone();
        self.chunks.entry((cx, cz)).or_insert_with(|| {
            let path = dir.join(format!("region/r.{}.{}.mca", cx >> 5, cz >> 5));
            let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let mut region = Region::from_stream(file).unwrap();
            let data = region
                .read_chunk((cx & 31) as usize, (cz & 31) as usize)
                .unwrap()
                .unwrap_or_else(|| panic!("chunk ({cx}, {cz}) is missing"));
            match JavaChunk::from_bytes(&data).unwrap() {
                chunk @ JavaChunk::Post18(_) => chunk,
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
