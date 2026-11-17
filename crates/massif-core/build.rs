//! Bundles the mod profiles of `assets/mods/<namespace>/` (see its README):
//! each folder found becomes an entry of `mod_profiles.rs` in OUT_DIR, so a
//! mod is added as data, without code.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let root = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/mods");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut dirs: Vec<_> = std::fs::read_dir(&root).map(|r| r.flatten().map(|e| e.path()).filter(|p| p.join("profile.json").is_file()).collect()).unwrap_or_default();
    dirs.sort();
    let mut out = String::from("&[\n");
    for dir in dirs {
        let file = |name: &str| {
            let p = dir.join(name);
            println!("cargo:rerun-if-changed={}", p.display());
            p.is_file().then(|| format!("include_str!({:?})", p.display().to_string()))
        };
        let profile = file("profile.json").unwrap();
        let palette = file("palette.json").map_or("None".to_owned(), |p| format!("Some({p})"));
        writeln!(out, "    ({profile}, {palette}),").unwrap();
    }
    out.push(']');
    std::fs::write(Path::new(&std::env::var("OUT_DIR").unwrap()).join("mod_profiles.rs"), out).unwrap();
}
