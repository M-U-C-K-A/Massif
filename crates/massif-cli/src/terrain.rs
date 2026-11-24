//! `massif-cli terrain …`: terrain graphs from the command line, for people
//! and for models: the schema and the catalogue, checking a graph, editing
//! it a step at a time, rendering it.

use super::{Args, Res};
use massif_terrain::compile::{Diagnostic, Severity, compile};
use massif_terrain::engine::{Engine, Settings, never};
use massif_terrain::format::{self, Op};
use massif_terrain::model::Graph;
use massif_terrain::space::Space;
use std::path::Path;
use std::time::Instant;

fn read(path: &Path) -> Res<String> {
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}

fn graph(path: &Path) -> Res<Graph> {
    Graph::from_json(&read(path)?).map_err(|e| format!("{}: {e} (massif-cli terrain validate says more)", path.display()))
}

/// Prints what was found, one line each, on the error stream; how many errors.
fn report(diags: &[Diagnostic]) -> usize {
    for d in diags {
        eprintln!("{d}");
    }
    diags.iter().filter(|d| d.severity == Severity::Error).count()
}

pub fn cmd(mut args: Args) -> Res<()> {
    if args.0.is_empty() {
        return Err("terrain needs a command: schema, catalog, validate, patch, render, migrate or build".into());
    }
    let sub = args.0.remove(0);
    match sub.as_str() {
        "schema" => {
            args.positional([])?;
            println!("{}", serde_json::to_string_pretty(&format::schema()).expect("the schema is JSON"));
            Ok(())
        }
        "catalog" | "catalogue" => {
            let json = args.flag("--json");
            args.positional([])?;
            if json {
                println!("{}", serde_json::to_string_pretty(&format::catalog_json()).expect("the catalogue is JSON"));
            } else {
                print!("{}", format::catalog_markdown());
            }
            Ok(())
        }
        "validate" => {
            let [file] = args.positional(["<graph.json>"])?;
            match report(&format::validate(&read(&file)?)) {
                0 => {
                    eprintln!("{}: a terrain graph that can be built", file.display());
                    Ok(())
                }
                n => Err(format!("{n} error{}", if n == 1 { "" } else { "s" })),
            }
        }
        "patch" => {
            let out = args.option("--out")?;
            let layout = args.flag("--layout");
            let [file, edits] = args.positional(["<graph.json>", "<edits.json>"])?;
            let mut g = graph(&file)?;
            let edits: serde_json::Value = serde_json::from_str(&read(&edits)?).map_err(|e| format!("{}: {e}", edits.display()))?;
            let json_patch = edits.as_array().and_then(|a| a.first()).is_some_and(|op| op.get("path").is_some());
            let diags = if json_patch {
                format::json_patch(&mut g, &edits)
            } else {
                let ops: Vec<Op> = serde_json::from_value(edits).map_err(|e| format!("the edits are a list of operations (add, set, link, unlink, remove, rename) or a JSON Patch: {e}"))?;
                format::patch(&mut g, &ops)
            };
            if layout {
                format::lay_out(&mut g);
            }
            let errors = report(&diags);
            match out {
                Some(path) => std::fs::write(&path, g.to_json() + "\n").map_err(|e| format!("{path}: {e}"))?,
                None => println!("{}", g.to_json()),
            }
            match errors {
                0 => Ok(()),
                n => Err(format!("{n} error{}; the graph written has every edit that could be made", if n == 1 { "" } else { "s" })),
            }
        }
        "render" => {
            let pixels: u32 = args.option("--pixels")?.map(|p| p.parse().map_err(|_| "--pixels: a whole number")).transpose()?.unwrap_or(1024);
            let size: Option<u32> = args.option("--size")?.map(|p| p.parse().map_err(|_| "--size: a whole number of blocks")).transpose()?;
            let cpu = args.flag("--cpu");
            let [file, out] = args.positional(["<graph.json>", "<out.png>"])?;
            let mut g = graph(&file)?;
            if size.is_some() {
                g.terrain.size = size;
            }
            let (c, diags) = compile(&g, Space::new(g.terrain.size.unwrap_or(8192)), 2048);
            if report(&diags) > 0 {
                return Err("the graph has errors".into());
            }
            let gpu = if cpu { None } else { massif_terrain::gpu::Gpu::headless() };
            let engine = match gpu {
                Some(gpu) => Engine::with_gpu(gpu, Settings::default()),
                None => Engine::new(Settings::default()),
            };
            let height = c.output("height").ok_or("the graph's Map Output has no Height")?;
            let lod = c.space.lod_for(pixels.max(1));
            let started = Instant::now();
            let whole = c.space.whole(lod);
            let grid = engine.region(&c, height, lod, whole, &never).map_err(|e| e.to_string())?;
            let albedo = c.output("albedo").map(|src| engine.region(&c, src, lod, whole, &never)).transpose().map_err(|e| e.to_string())?;
            let (lo, hi) = grid.range();
            eprintln!("{}² samples at level {lod} on {} in {:.0} ms; heights {lo:.0} to {hi:.0}", grid.rect.w, engine.gpu_name().unwrap_or("the CPU"), started.elapsed().as_secs_f64() * 1000.0);
            let rgb = massif_terrain::preview::shade_with(&grid, c.space.sea_level, albedo.as_ref());
            let img = image::RgbImage::from_raw(grid.rect.w, grid.rect.h, rgb.into_iter().flatten().collect()).expect("one pixel a sample");
            img.save(&out).map_err(|e| format!("{}: {e}", out.display()))
        }
        "stream" => {
            // The whole terrain at one sample a block, window after window,
            // as Apply writes a map too large to hold: how long, and (under
            // /usr/bin/time -l) how much memory.
            let size: Option<u32> = args.option("--size")?.map(|p| p.parse().map_err(|_| "--size: a whole number of blocks")).transpose()?;
            let cache_mb: usize = args.option("--cache")?.map(|p| p.parse().map_err(|_| "--cache: megabytes")).transpose()?.unwrap_or(Settings::default().cache_mb);
            let window: u32 = args.option("--window")?.map(|p| p.parse().map_err(|_| "--window: a whole number of blocks")).transpose()?.unwrap_or(1024);
            let cpu = args.flag("--cpu");
            let [file] = args.positional(["<graph.json>"])?;
            let mut g = graph(&file)?;
            if size.is_some() {
                g.terrain.size = size;
            }
            let (c, diags) = compile(&g, Space::new(g.terrain.size.unwrap_or(8192)), 2048);
            if report(&diags) > 0 {
                return Err("the graph has errors".into());
            }
            let gpu = if cpu { None } else { massif_terrain::gpu::Gpu::headless() };
            let engine = match gpu {
                Some(gpu) => Engine::with_gpu(gpu, Settings { cache_mb }),
                None => Engine::new(Settings { cache_mb }),
            };
            let started = Instant::now();
            let n = c.space.samples(0);
            let windows = n.div_ceil(window);
            let (mut lo, mut hi) = (f32::MAX, f32::MIN);
            for wz in 0..windows {
                for wx in 0..windows {
                    let rect = massif_terrain::space::Rect::new((wx * window) as i32, (wz * window) as i32, window, window);
                    let (_, out) = massif_terrain::world::outputs_in(&engine, &c, 0, rect, &never).map_err(|e| e.to_string())?;
                    if let Some(h) = &out.heights {
                        for v in &h.data {
                            lo = lo.min(*v);
                            hi = hi.max(*v);
                        }
                    }
                }
                eprintln!("row {}/{windows} after {:.1} s, cache {} MB", wz + 1, started.elapsed().as_secs_f64(), engine.cache_bytes() >> 20);
            }
            eprintln!("{n}² blocks in {windows}² windows of {window} on {} in {:.1} s; heights {lo:.0} to {hi:.0}", engine.gpu_name().unwrap_or("the CPU"), started.elapsed().as_secs_f64());
            Ok(())
        }
        "migrate" => {
            let out = args.option("--out")?;
            let [file] = args.positional(["<world.massif>"])?;
            let world = massif_format::load(&file).map_err(|e| e.to_string())?;
            let (g, diags) = massif_terrain::world::graph_of(&world).ok_or("the world has no terrain graph")?;
            report(&diags);
            match out {
                Some(path) => std::fs::write(&path, g.to_json() + "\n").map_err(|e| format!("{path}: {e}"))?,
                None => println!("{}", g.to_json()),
            }
            Ok(())
        }
        "build" => build(args),
        other => Err(format!("unknown terrain command \"{other}\": schema, catalog, validate, patch, render, migrate or build")),
    }
}

/// Builds a graph into a world: the world's own graph (1.2's migrated) or
/// one given, over its map, its ground and terrains written as Convert
/// writes them.
fn build(mut args: Args) -> Res<()> {
    let given = args.option("--graph")?;
    let samples: Option<u32> = args.option("--samples")?.map(|p| p.parse().map_err(|_| "--samples: a whole number")).transpose()?;
    let cpu = args.flag("--cpu");
    let [file, out] = args.positional(["<world.massif>", "<out.massif>"])?;
    let mut world = massif_format::load(&file).map_err(|e| e.to_string())?;
    let g = match &given {
        Some(path) => graph(Path::new(path))?,
        None => {
            let (g, diags) = massif_terrain::world::graph_of(&world).ok_or("the world has no terrain graph: give one with --graph")?;
            report(&diags);
            g
        }
    };
    let space = massif_terrain::world::space_of(&world, 0, &g);
    let folder = file.parent().map(Path::to_path_buf).unwrap_or_default();
    let env = std::sync::Arc::new(massif_terrain::world::MapEnv::new(&world, 0, 1, folder));
    let samples = samples.unwrap_or(space.size.min(8192));
    let (c, diags) = massif_terrain::compile::compile_in(&g, space, samples.min(4096), env);
    if report(&diags) > 0 {
        return Err("the graph has errors".into());
    }
    let gpu = if cpu { None } else { massif_terrain::gpu::Gpu::headless() };
    let engine = match gpu {
        Some(gpu) => Engine::with_gpu(gpu, Settings::default()),
        None => Engine::new(Settings::default()),
    };
    let lod = c.space.lod_for(samples.max(1));
    let started = Instant::now();
    let (extent, outputs) = massif_terrain::world::outputs(&engine, &c, lod, &never).map_err(|e| e.to_string())?;
    eprintln!("{}² samples over {} blocks on {} in {:.1} s", extent.n, c.space.size, engine.gpu_name().unwrap_or("the CPU"), started.elapsed().as_secs_f64());
    let reg = &world.registry;
    let palette: Vec<(massif_core::TerrainId, [u8; 3])> = (0..reg.terrains.len()).map(|k| (massif_core::TerrainId(k as u16), reg.terrain(massif_core::TerrainId(k as u16)).color)).collect();
    let climate = massif_core::graph::ClimateTable::new(reg);
    let blank = reg.terrain_by_name("Grass").unwrap_or(massif_core::TerrainId::ROCK);
    let rect = massif_core::BlockRect::new(c.space.x0, c.space.z0, c.space.x0 + c.space.size as i32, c.space.z0 + c.space.size as i32);
    let tiles: Vec<massif_core::TilePos> = rect.tiles().collect();
    let (seed, sea) = (world.meta.seed as u64, world.meta.sea_level);
    let registry = world.registry.clone();
    let mut history = massif_core::History::new(1);
    history.begin("Terrain graph", 0);
    {
        let paint = massif_core::graph::Painting { palette: &palette, climate: Some(&climate) };
        let write = massif_core::graph::GroundWrite { tiles: &tiles, create: true, map: None, sea, blank };
        let edit = &mut history.edit(&mut world, 0);
        massif_core::graph::apply(edit, &extent, &outputs, seed, &paint, &write);
        if outputs.biomes.is_some() {
            massif_core::graph::apply_biomes(edit, &extent, &outputs, &climate, &tiles);
        }
        let started = Instant::now();
        let under = massif_terrain::underground::apply(&engine, &c, &registry, edit, &never).map_err(|e| e.to_string())?;
        if under != Default::default() {
            eprintln!(
                "underground in {:.1} s: {} cave layers over {} columns, {} cells of cave biomes, {} sculpted chunks",
                started.elapsed().as_secs_f64(),
                under.cave_layers,
                under.cave_columns,
                under.biome_cells,
                under.chunks
            );
        }
    }
    if !outputs.layers.is_empty() {
        eprintln!("{} layers of rock are not written by the command line yet: Convert in the app writes them", outputs.layers.len());
    }
    world.meta.terrain_graph = Some(serde_json::to_value(&g).expect("graphs serialise"));
    massif_format::save(&world.snapshot(), &out).map_err(|e| e.to_string())?;
    Ok(())
}
