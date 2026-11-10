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
  massif-cli export <world.massif> <folder> [--void] [--survival] [--overwrite]
  massif-cli import <old.world> <out.massif>

  --size      map size in tiles of 128 blocks (default 8x8 = 1024x1024 blocks)
  --landform  flat | plains | hills | mountains | islands (default hills)
  --void      nothing generates around the exported map (default: vanilla terrain)
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
    let options = ExportOptions {
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
    let [path, dir] = args.positional(["<world.massif>", "<folder>"])?;
    let t = Instant::now();
    let world = massif_format::load(&path).map_err(|e| e.to_string())?;
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
