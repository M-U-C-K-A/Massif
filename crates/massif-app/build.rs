//! Gives massif.exe its icon on Windows, and lists the pictures behind the
//! home screen (`assets/backdrops`) so that every one there is built in.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    backdrops();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("assets/icon.ico");
        res.set("ProductName", "Massif");
        res.set("FileDescription", "Massif, a map editor for Minecraft worlds");
        if let Err(e) = res.compile() {
            println!("cargo:warning=no icon for massif.exe: {e}");
        }
    }
}

/// The endings of the names of a scene's layers.
const LAYERS: [&str; 4] = ["_ciel", "_sky", "_relief", "_land"];

/// Write `backdrops.rs` into OUT_DIR: every JPEG and PNG of
/// `assets/backdrops` but the layers of a scene, by name, its bytes built
/// into the app.
fn backdrops() {
    println!("cargo:rerun-if-changed=assets/backdrops");
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/backdrops");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png")))
                // The layers of a scene (its sky, its relief cut out) are
                // not pictures to show alone; the whole scene is.
                .filter(|p| p.file_stem().and_then(|s| s.to_str()).is_none_or(|s| !LAYERS.iter().any(|l| s.to_ascii_lowercase().ends_with(l))))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
    // The same picture under two names is built in once.
    let mut seen = std::collections::HashSet::new();
    files.retain(|f| {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        std::fs::read(f).unwrap_or_default().hash(&mut h);
        seen.insert(h.finish())
    });
    let mut code = String::from("/// The pictures of `assets/backdrops`: name, bytes.\npub const BACKDROPS: &[(&str, &[u8])] = &[\n");
    for f in &files {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = f.file_stem().and_then(|s| s.to_str()).unwrap_or("backdrop");
        code.push_str(&format!("    ({name:?}, include_bytes!({:?})),\n", f.display().to_string()));
    }
    code.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("backdrops.rs");
    std::fs::write(out, code).expect("writing backdrops.rs");
}
