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
            let path = dir.join(format!("region/r.{}.{}.mca", cx >> 5, cz >> 5));
            let file = std::fs::File::open(&path).unwrap_or_else(|e| panic!("{path:?}: {e}"));
            let mut region = Region::from_stream(file).unwrap();
            let data = region
                .read_chunk((cx & 31) as usize, (cz & 31) as usize)
                .unwrap()
                .unwrap_or_else(|| panic!("chunk ({cx}, {cz}) is missing"));
            match JavaChunk::from_bytes(&data).unwrap() {
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
            // From 1.18.2 on, level.dat is 1.18.2's: later versions bring
            // it up to date themselves.
            assert_eq!(data["DataVersion"], fastnbt::Value::Int(target.data_version.min(2975)), "{name}");
        } else {
            assert!(!data.contains_key("DataVersion"), "{name}: versions before 1.9 have no data version");
        }
        match era {
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
