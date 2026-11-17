//! Command-line companion: create, inspect, verify and export worlds without
//! opening the editor.

use massif_anvil::{ExportOptions, GameMode, Progress, Surroundings};
use massif_core::generate::{GenParams, Landform, generate};
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

const USAGE: &str = "\
massif-cli — Massif worlds from the command line

USAGE
  massif-cli new <out.massif> [--size WxD] [--landform NAME] [--seed N] [--name TEXT]
  massif-cli info <world.massif>
  massif-cli verify <world.massif>
  massif-cli export <world.massif> <folder> [--version V] [--void] [--survival] [--no-ores] [--overwrite]
  massif-cli import <old.world> <out.massif>
  massif-cli probe <world.massif> [<x> <z>…] [--random N]
  massif-cli analyse <world.massif>
  massif-cli water <world.massif>
  massif-cli rivers <world.massif> <out.massif> [--threshold T]
  massif-cli spring <world.massif> <out.massif> <x> <z>…
  massif-cli crop <world.massif> <out.massif> <x0> <z0> <x1> <z1>

  --size      map size in tiles of 128 blocks (default 8x8 = 1024x1024 blocks)
  --landform  flat | plains | hills | mountains | islands (default hills)
  --version   the Minecraft version to export for, 1.7.10 to 26.3 (default 1.18.2,
              which every later version opens)
  --void      nothing generates around the exported map (default: vanilla terrain)
  --no-ores   lay no ores in the rock (default: the map's own, see Edit ▸ Ores)

  probe prints, for each column, what the export writes there, one JSON object a
  line: its top solid block (y and block state), the water over it, its biome —
  what tools/minecraft_check.py asks the game itself to confirm.

  analyse prints the survival check of Edit ▸ Analyse the map as Markdown: every
  block counted as the export writes it, what breaks survival, what is missing.
";

type Res<T> = Result<T, String>;

struct Args(Vec<String>);

impl Args {
    /// Remove `--name value` and return the value.
    fn option(&mut self, name: &str) -> Res<Option<String>> {
        let Some(i) = self.0.iter().position(|a| a == name) else {
            return Ok(None);
        };
        if i + 1 >= self.0.len() {
            return Err(format!("{name} needs a value"));
        }
        self.0.remove(i);
        Ok(Some(self.0.remove(i)))
    }

    /// Remove `--name` and tell whether it was there.
    fn flag(&mut self, name: &str) -> bool {
        match self.0.iter().position(|a| a == name) {
            Some(i) => {
                self.0.remove(i);
                true
            }
            None => false,
        }
    }

    /// The positional arguments, once every option has been taken out.
    fn positional<const N: usize>(self, what: [&str; N]) -> Res<[PathBuf; N]> {
        if let Some(stray) = self.0.iter().find(|a| a.starts_with("--")) {
            return Err(format!("unknown option {stray}"));
        }
        if self.0.len() != N {
            return Err(format!("expected: {}", what.join(" ")));
        }
        let mut it = self.0.into_iter().map(PathBuf::from);
        Ok(std::array::from_fn(|_| it.next().unwrap()))
    }
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

fn cmd_new(mut args: Args) -> Res<()> {
    let mut params = GenParams::default();
    if let Some(size) = args.option("--size")? {
        let (w, d) = size
            .split_once(['x', 'X'])
            .ok_or("--size expects WxD, for example 16x16")?;
        params.tiles_x = w.parse().map_err(|_| "--size: bad width")?;
        params.tiles_z = d.parse().map_err(|_| "--size: bad depth")?;
        if params.tiles_x < 1 || params.tiles_z < 1 || params.tiles_x > 4096 || params.tiles_z > 4096 {
            return Err("--size: each side must be between 1 and 4096 tiles".into());
        }
    }
    if let Some(name) = args.option("--landform")? {
        params.landform = Landform::ALL
            .into_iter()
            .find(|l| l.label().eq_ignore_ascii_case(&name))
            .ok_or_else(|| format!("unknown landform \"{name}\""))?;
    }
    if let Some(seed) = args.option("--seed")? {
        params.seed = seed.parse().map_err(|_| "--seed expects an integer")?;
    }
    if let Some(name) = args.option("--name")? {
        params.name = name;
    }
    let [out] = args.positional(["<out.massif>"])?;

    let t = Instant::now();
    let world = generate(&params);
    let generated = t.elapsed();
    let t = Instant::now();
    let stats = massif_format::save(&world, &out).map_err(|e| e.to_string())?;
    println!(
        "{}: {}×{} blocks, {} tiles, {} ({:.2} bytes per column)",
        out.display(),
        params.tiles_x * 128,
        params.tiles_z * 128,
        stats.tiles,
        human(stats.file_len),
        stats.file_len as f64 / (stats.tiles as f64 * 16384.0),
    );
    println!("generated in {generated:.2?}, saved in {:.2?}", t.elapsed());
    Ok(())
}

fn cmd_info(args: Args) -> Res<()> {
    let [path] = args.positional(["<world.massif>"])?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let info = massif_format::inspect(&bytes).map_err(|e| e.to_string())?;
    println!("{}", path.display());
    println!("  name        {}", info.world.name);
    println!("  written by  {} (format {})", info.generator, info.version);
    println!("  size        {}", human(info.file_len));
    println!("  seed        {}", info.world.seed);
    println!("  spawn       {:?}", info.world.spawn);
    println!(
        "  registry    {} blocks, {} terrains, {} biomes",
        info.blocks, info.terrains, info.biomes
    );
    for dim in &info.dimensions {
        println!("  {} (Y {}..{})", dim.kind.label(), dim.min_y, dim.max_y);
        for level in &dim.levels {
            println!(
                "    {:<7} {:<24} {:>7} tiles  {:>10}",
                level.kind.label(),
                level.name,
                level.tiles,
                human(level.bytes)
            );
        }
        if dim.sculpted_chunks > 0 {
            println!(
                "    {:<7} {:<24} {:>7} chunks {:>10}",
                "3D",
                "Sculpted (16³ chunks)",
                dim.sculpted_chunks,
                human(dim.sculpted_bytes)
            );
        }
    }
    if !info.points.is_empty() {
        println!("  save points {} ({} of tiles of their own): {}", info.points.len(), human(info.point_bytes), info.points.join(", "));
    }
    Ok(())
}

fn cmd_verify(args: Args) -> Res<()> {
    let [path] = args.positional(["<world.massif>"])?;
    let bytes = std::fs::read(&path).map_err(|e| e.to_string())?;
    let t = Instant::now();
    let tiles = massif_format::verify(&bytes).map_err(|e| e.to_string())?;
    println!("{}: {tiles} tiles decoded without error in {:.2?}", path.display(), t.elapsed());
    Ok(())
}

fn cmd_export(mut args: Args) -> Res<()> {
    let target = match args.option("--version")? {
        Some(name) => massif_anvil::Target::release(&name).ok_or_else(|| {
            let known: Vec<&str> = massif_anvil::version::RELEASES.iter().map(|r| r.0).collect();
            format!("unknown Minecraft version {name}; known: {}", known.join(", "))
        })?,
        None => massif_anvil::Target::default(),
    };
    let options = ExportOptions {
        target,
        surroundings: if args.flag("--void") {
            Surroundings::Void
        } else {
            Surroundings::Vanilla
        },
        game_mode: if args.flag("--survival") {
            GameMode::Survival
        } else {
            GameMode::Creative
        },
        overwrite: args.flag("--overwrite"),
        ..ExportOptions::default()
    };
    let no_ores = args.flag("--no-ores");
    let [path, dir] = args.positional(["<world.massif>", "<folder>"])?;
    let t = Instant::now();
    let mut world = massif_format::load(&path).map_err(|e| e.to_string())?;
    if no_ores {
        world.meta.ores.on = false;
    }
    let loaded = t.elapsed();
    let stats = massif_anvil::export(&world, &dir, &options, &Progress::default())
        .map_err(|e| e.to_string())?;
    println!(
        "{}: {} chunks in {} region files, {}",
        dir.display(),
        stats.chunks,
        stats.regions,
        human(stats.bytes)
    );
    println!("loaded in {loaded:.2?}, exported in {:.2?}", stats.elapsed);
    Ok(())
}

fn cmd_import(args: Args) -> Res<()> {
    let [input, out] = args.positional(["<old.world>", "<out.massif>"])?;
    let t = Instant::now();
    let (world, report) = massif_legacy::import_world(&input, None).map_err(|e| e.to_string())?;
    let imported = t.elapsed();
    let stats = massif_format::save(&world, &out).map_err(|e| e.to_string())?;
    println!("{} → {}", input.display(), out.display());
    if let Some(by) = &report.saved_by {
        println!("  saved by     {by}{}", report.platform.as_ref().map(|p| format!(" for {p}")).unwrap_or_default());
    }
    for d in &report.dimensions {
        println!("  imported     {d}");
    }
    for d in &report.skipped_dimensions {
        println!("  skipped      {d}");
    }
    println!("  terrains     {} kinds, {} custom", report.terrains.len(), report.custom_terrains);
    let columns: u64 = report.terrains.iter().map(|t| t.1).sum::<u64>().max(1);
    for (name, n) in report.terrains.iter().take(8) {
        println!("    {name:<20} {:5.1} %", *n as f64 * 100.0 / columns as f64);
    }
    if !report.converted.is_empty() {
        let names: Vec<String> = report.converted.iter().map(|(n, t)| format!("{n} ({t})")).collect();
        println!("  layers       {}", names.join(", "));
    }
    if !report.layers.is_empty() {
        let names: Vec<String> = report.layers.iter().map(|(n, t)| format!("{n} ({t})")).collect();
        println!("  not imported layers: {}", names.join(", "));
    }
    println!(
        "  {} tiles imported in {imported:.2?}, saved {} in {:.2?}",
        report.tiles,
        human(stats.file_len),
        t.elapsed() - imported
    );
    Ok(())
}

fn cmd_probe(mut args: Args) -> Res<()> {
    let random: usize = args.option("--random")?.map(|v| v.parse().map_err(|_| format!("--random needs a number, not {v}"))).transpose()?.unwrap_or(0);
    let mut rest = args.0.into_iter();
    let path = PathBuf::from(rest.next().ok_or("probe needs <world.massif>")?);
    let mut coords: Vec<i32> = rest.map(|a| a.parse::<i32>().map_err(|_| format!("not a block coordinate: {a}"))).collect::<Res<_>>()?;
    if !coords.len().is_multiple_of(2) || (coords.is_empty() && random == 0) {
        return Err("probe needs pairs of block coordinates (<x> <z>), or --random N".into());
    }
    let world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let dim = &world.dims[0];
    // Columns spread over the map's tiles, the same every time.
    let mut tiles: Vec<massif_core::TilePos> = dim.ground().positions().collect();
    tiles.sort_by_key(|p| (p.z, p.x));
    for k in 0..random {
        if tiles.is_empty() {
            break;
        }
        let h = massif_core::compose::hash2(k as i32, 7, 0x9e0b);
        let t = tiles[h as usize % tiles.len()];
        let (ox, oz) = t.origin();
        coords.push(ox + (h >> 8) as i32 % 128);
        coords.push(oz + (h >> 16) as i32 % 128);
    }
    let reg = &world.registry;
    let mat = massif_core::compose::Materializer::new(reg, world.meta.seed, dim.min_y);
    let (mut spans, mut fluids) = (Vec::new(), Vec::new());
    for pair in coords.chunks(2) {
        let (x, z) = (pair[0], pair[1]);
        let pos = massif_core::TilePos::containing(x, z);
        let Some(stack) = massif_core::compose::TileStack::new(dim, pos, false) else {
            println!("{{\"x\":{x},\"z\":{z},\"top\":null}}");
            continue;
        };
        let cell = massif_core::coords::local_index(x, z);
        stack.spans(cell, &mut spans);
        stack.fluids(cell, &spans, &mut fluids);
        let Some(top) = spans.last().copied() else {
            println!("{{\"x\":{x},\"z\":{z},\"top\":null}}");
            continue;
        };
        let y = top.y1 - 1;
        let water = fluids.last().filter(|f| f.0 == top.y1).copied();
        let block = reg.block(mat.block(&top, x, y, z, water.is_some())).state.clone();
        let biome = stack.layers().filter(|(_, _, t)| t.present(cell)).map(|(_, _, t)| t.biome.get(cell)).filter(|b| *b != 0).last();
        let biome = biome.map(|b| format!("\"{}\"", reg.biome(massif_core::registry::BiomeId(b)).id)).unwrap_or_else(|| "null".into());
        let water = water.map(|(a, b)| format!("[{a},{b}]")).unwrap_or_else(|| "null".into());
        println!("{{\"x\":{x},\"z\":{z},\"top\":{y},\"block\":\"{block}\",\"water\":{water},\"biome\":{biome}}}");
    }
    Ok(())
}

fn cmd_analyse(args: Args) -> Res<()> {
    let [path] = args.positional(["<world.massif>"])?;
    let started = Instant::now();
    let world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let loaded = started.elapsed().as_secs_f32();
    let steps = massif_core::auto::Steps::default();
    let analysis = massif_core::analysis::analyse(&world, 0, &steps).ok_or("the map has no ground")?;
    print!("{}", analysis.report());
    eprintln!("read in {loaded:.2} s, analysed in {:.2} s", analysis.seconds);
    Ok(())
}

fn cmd_water(args: Args) -> Res<()> {
    let [path] = args.positional(["<world.massif>"])?;
    let world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let c = massif_core::water::check(&world.dims[0]);
    if std::env::var_os("MASSIF_WATER_WHERE").is_some() {
        let mut seen = 0;
        for p in world.dims[0].ground().positions() {
            massif_core::water::check_tile_with(&world.dims[0], p, &mut |x, y, z, k| {
                if seen < 1_000_000 && !matches!(k, massif_core::water::Problem::Step | massif_core::water::Problem::Fall) {
                    println!("{k:?} at {x} {y} {z}");
                    seen += 1;
                }
            });
        }
    }
    println!("columns with water: {}", c.columns);
    println!("steps (water over lower water): {}", c.steps);
    println!("falls (water over water lower by more than a block): {}", c.falls);
    println!("spills (water beside open land): {}", c.spills);
    println!("floating (air under water): {}", c.floating);
    println!("at the edge of the map: {}", c.edge);
    eprintln!("checked in {:.2} s", started.elapsed().as_secs_f32());
    // Three pictures of a tile's water from above (MASSIF_WATER_PICTURE=
    // "x,z,out.png"): as the map holds it, once the game updates it, and as
    // the export writes it.
    if let Some(spec) = std::env::var_os("MASSIF_WATER_PICTURE") {
        let spec = spec.to_string_lossy().to_string();
        let parts: Vec<&str> = spec.splitn(3, ',').collect();
        let (x, z) = (parts[0].parse::<i32>().map_err(|e| e.to_string())?, parts[1].parse::<i32>().map_err(|e| e.to_string())?);
        let pos = massif_core::TilePos::containing(x, z);
        let dim = &world.dims[0];
        let (base, _) = massif_core::water::grid_around(dim, pos, 8, None).ok_or("no water to settle on that tile")?;
        let mut game = base.clone();
        let mut q = game.restless();
        game.run(&mut q, usize::MAX);
        let mut ours = base.clone();
        ours.steady(usize::MAX);
        const S: u32 = 4;
        let side = 128 * S;
        let mut img = image::RgbImage::new(3 * side + 2 * 8, side);
        for p in img.pixels_mut() {
            *p = image::Rgb([255, 255, 255]);
        }
        for (k, g) in [&base, &game, &ours].iter().enumerate() {
            for j in 0..128 {
                for i in 0..128 {
                    let (bx, bz) = (g.x0 + 8 + i, g.z0 + 8 + j);
                    let mut y = g.y0 + g.ny as i32 - 1;
                    while y > g.y0 && g.get(bx, y, bz) == massif_core::flow::Block::Air {
                        y -= 1;
                    }
                    // Light from the north-west, by the height of the surface.
                    let up = |dx: i32, dz: i32| {
                        let mut t = g.y0 + g.ny as i32 - 1;
                        while t > g.y0 && g.get(bx + dx, t, bz + dz) == massif_core::flow::Block::Air {
                            t -= 1;
                        }
                        t
                    };
                    let shade = (1.0 - 0.12 * (up(-1, -1) - y) as f32).clamp(0.6, 1.3);
                    let c: [f32; 3] = match g.get(bx, y, bz) {
                        massif_core::flow::Block::Source => [40.0, 90.0, 200.0],
                        massif_core::flow::Block::Flowing { falling: true, .. } => [235.0, 240.0, 255.0],
                        massif_core::flow::Block::Flowing { amount, .. } => [60.0 + 20.0 * (8 - amount) as f32, 170.0, 235.0],
                        _ => {
                            let t = ((y - 60) as f32 / 60.0).clamp(0.0, 1.0);
                            [90.0 + 80.0 * t, 130.0 + 40.0 * t, 70.0 + 80.0 * t]
                        }
                    };
                    let c = image::Rgb(c.map(|v| (v * shade).clamp(0.0, 255.0) as u8));
                    for dy in 0..S {
                        for dx in 0..S {
                            img.put_pixel(k as u32 * (side + 8) + i as u32 * S + dx, j as u32 * S + dy, c);
                        }
                    }
                }
            }
        }
        img.save(parts[2]).map_err(|e| e.to_string())?;
        println!("picture of tile {pos:?} in {}", parts[2]);
    }
    if std::env::var_os("MASSIF_WATER_TIME").is_some() {
        use rayon::prelude::*;
        let dim = &world.dims[0];
        let t = Instant::now();
        let planner = massif_core::water::Planner::new(dim);
        let tiles: Vec<_> = dim.ground().positions().filter(|&p| planner.needs(p)).collect();
        eprintln!("planner: {:.2} s, {} tiles of {} need settling", t.elapsed().as_secs_f32(), tiles.len(), dim.ground().positions().count());
        let t = Instant::now();
        let cols: usize = tiles.par_iter().map(|&p| massif_core::water::columns_around(dim, p, 8).len()).sum();
        eprintln!("columns: {:.2} s ({cols})", t.elapsed().as_secs_f32());
        let t = Instant::now();
        let n: usize = tiles.par_iter().map(|&p| massif_core::water::grid_around(dim, p, 8, Some(&planner)).map_or(0, |g| g.0.blocks.len())).sum();
        eprintln!("columns and grids: {:.2} s ({n} blocks)", t.elapsed().as_secs_f32());
        let t = Instant::now();
        let n: usize = tiles.par_iter().map(|&p| massif_core::water::steady_tile(dim, p, Some(&planner)).len()).sum();
        eprintln!("steady: {:.2} s ({n} blocks)", t.elapsed().as_secs_f32());
    }
    if std::env::var_os("MASSIF_WATER_SETTLE").is_some() {
        let started = Instant::now();
        let s = massif_core::water::settle_all(&world.dims[0], std::env::var_os("MASSIF_WATER_STEADY").is_some());
        println!("once updated: {} blocks become water ({} over dry columns), {} change, {} removed, {} tiles unsettled; {} new sources, {} falling", s.added, s.over_land, s.changed, s.removed, s.unsettled, s.sources, s.falling);
        eprintln!("settled in {:.2} s", started.elapsed().as_secs_f32());
    }
    Ok(())
}

/// Edit ▸ Whole map ▸ Rivers with the settings the app starts from.
fn cmd_rivers(mut args: Args) -> Res<()> {
    let threshold: f32 = args.option("--threshold")?.map(|v| v.parse().map_err(|_| format!("--threshold needs a number, not {v}"))).transpose()?.unwrap_or(0.5);
    let [path, out] = args.positional(["<world.massif>", "<out.massif>"])?;
    let mut world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let sea = world.meta.sea_level;
    let bed = world.registry.terrain_by_name("Gravel");
    let shore = world.registry.terrain_by_name("Sand");
    let side = world.dims[0].ground().positions().fold((i32::MAX, i32::MIN), |(a, b), p| (a.min(p.x), b.max(p.x)));
    let f = (((side.1 - side.0 + 1) * 128) as f32 / 2048.0).clamp(0.5, 3.0);
    let op = massif_core::graph::Reshape::Filter(massif_core::graph::NodeKind::Rivers {
        threshold,
        depth: 4.0,
        width: 10.0 * f.clamp(1.0, 2.5),
        sea: sea as f32,
        banks: 1.0,
        lakes: 0.5,
    });
    let started = Instant::now();
    let mut history = massif_core::history::History::new(1);
    history.begin("Rivers", 0);
    let changes = massif_core::graph::reshape_ground(&mut history.edit(&mut world, 0), &op, sea, bed, shore, &massif_core::auto::Steps::default());
    eprintln!("rivers: {} columns in {} tiles, {:.2} s", changes.columns, changes.tiles.len(), started.elapsed().as_secs_f32());
    massif_format::save(&world.snapshot(), &out).map_err(|e| e.to_string())?;
    Ok(())
}

/// The tiles of a map over a rectangle of blocks, as a map of their own.
fn cmd_crop(args: Args) -> Res<()> {
    let mut rest = args.0.into_iter();
    let path = PathBuf::from(rest.next().ok_or("crop needs <world.massif>")?);
    let out = PathBuf::from(rest.next().ok_or("crop needs <out.massif>")?);
    let c: Vec<i32> = rest.map(|a| a.parse::<i32>().map_err(|_| format!("not a block coordinate: {a}"))).collect::<Res<_>>()?;
    let [x0, z0, x1, z1] = c[..] else { return Err("crop needs <x0> <z0> <x1> <z1>".into()) };
    let (a, b) = (massif_core::TilePos::containing(x0.min(x1), z0.min(z1)), massif_core::TilePos::containing(x0.max(x1), z0.max(z1)));
    let mut world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let mut kept = 0;
    for level in &mut world.dims[0].levels {
        let all: Vec<massif_core::TilePos> = level.positions().collect();
        for p in all {
            if p.x < a.x || p.x > b.x || p.z < a.z || p.z > b.z {
                level.remove(p);
            } else {
                kept += 1;
            }
        }
    }
    massif_format::save(&world.snapshot(), &out).map_err(|e| e.to_string())?;
    eprintln!("{kept} tiles kept");
    Ok(())
}

/// The River tool let go at each spring, with its default settings.
fn cmd_spring(args: Args) -> Res<()> {
    let mut rest = args.0.into_iter();
    let path = PathBuf::from(rest.next().ok_or("spring needs <world.massif>")?);
    let out = PathBuf::from(rest.next().ok_or("spring needs <out.massif>")?);
    let coords: Vec<i32> = rest.map(|a| a.parse::<i32>().map_err(|_| format!("not a block coordinate: {a}"))).collect::<Res<_>>()?;
    if coords.is_empty() || !coords.len().is_multiple_of(2) {
        return Err("spring needs pairs of block coordinates".into());
    }
    let mut world = massif_format::load(&path).map_err(|e| e.to_string())?;
    let sea = world.meta.sea_level as f32;
    let reg = world.registry.clone();
    let spec = massif_core::river::RiverSpec {
        width: 12.0,
        width_end: 12.0 * 3.0,
        depth: 4.0,
        depth_end: 4.0 * 1.75,
        banks: 12.0 + 0.3 * 4.0 * 12.0,
        water: true,
        bed: reg.terrain_by_name("Gravel"),
        shore: reg.terrain_by_name("Sand"),
        clay: reg.terrain_by_name("Clay"),
        sea: None,
        biome: reg.biome_by_id("minecraft:river"),
    };
    for pair in coords.chunks(2) {
        let started = Instant::now();
        let mut d = massif_core::river::descend(world.dims[0].ground(), (pair[0], pair[1]), sea, 60_000, 2_000_000, 13.0);
        // As the River tool lays it out.
        d.course = massif_core::river::natural_course(world.dims[0].ground(), &d.course, 12.0 * 2.0, &[]).0;
        let mut history = massif_core::history::History::new(1);
        history.begin("River", 0);
        let r = massif_core::river::dig_descent(&mut history.edit(&mut world, 0), &d, &spec);
        eprintln!("river from {} {}: {:.0} blocks to {:?}, {} lakes, {} columns, {:.2} s", pair[0], pair[1], r.length, d.mouth, d.lakes.len(), r.columns, started.elapsed().as_secs_f32());
        let every = (d.course.len() / 8).max(1);
        eprintln!("  by way of {}", d.course.iter().step_by(every).map(|p| format!("{:.0},{:.0}", p.0, p.1)).collect::<Vec<_>>().join("  "));
    }
    massif_format::save(&world.snapshot(), &out).map_err(|e| e.to_string())?;
    Ok(())
}

fn main() -> ExitCode {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "--help" || a == "-h") {
        print!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    let command = args.remove(0);
    let args = Args(args);
    let result = match command.as_str() {
        "new" => cmd_new(args),
        "info" => cmd_info(args),
        "verify" => cmd_verify(args),
        "export" => cmd_export(args),
        "import" => cmd_import(args),
        "probe" => cmd_probe(args),
        "water" => cmd_water(args),
        "rivers" => cmd_rivers(args),
        "spring" => cmd_spring(args),
        "crop" => cmd_crop(args),
        "analyse" | "analyze" => cmd_analyse(args),
        other => Err(format!("unknown command \"{other}\"\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("error: {message}");
            ExitCode::FAILURE
        }
    }
}
