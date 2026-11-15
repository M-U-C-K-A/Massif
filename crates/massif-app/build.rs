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

/// Write `backdrops.rs` into OUT_DIR: every JPEG and PNG of
/// `assets/backdrops`, by name, its bytes built into the app.
fn backdrops() {
    println!("cargo:rerun-if-changed=assets/backdrops");
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets/backdrops");
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok().map(|e| e.path()))
                .filter(|p| p.extension().and_then(|e| e.to_str()).is_some_and(|e| matches!(e.to_ascii_lowercase().as_str(), "jpg" | "jpeg" | "png")))
                .collect()
        })
        .unwrap_or_default();
    files.sort();
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
