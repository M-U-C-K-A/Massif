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
        return Err("terrain needs a command: schema, catalog, validate, patch or render".into());
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
        other => Err(format!("unknown terrain command \"{other}\": schema, catalog, validate, patch or render")),
    }
}
