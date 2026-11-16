//! Gives massif.exe its icon on Windows, and lists the pictures behind the
//! home screen (`assets/backdrops`) so that every one there is built in.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    backdrops();
    commit();
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
/// The endings of a scene's picture at dawn, for the light theme; a scene's
/// whole picture may end in `_complet`.
const DAWN: [&str; 3] = ["_aube", "_dawn", "_day"];

/// What pairs a scene's night and its dawn: its name without either ending.
fn scene_key(stem: &str) -> &str {
    DAWN.iter().chain(["_complet"].iter()).find_map(|e| stem.strip_suffix(e)).unwrap_or(stem)
}

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
    let stem = |f: &std::path::PathBuf| f.file_stem().and_then(|s| s.to_str()).unwrap_or("backdrop").to_owned();
    let is_dawn = |s: &str| DAWN.iter().any(|e| s.to_ascii_lowercase().ends_with(e));
    let mut code = String::from("/// A picture of `assets/backdrops`: name, bytes at night, bytes at dawn\n/// (for the light theme) if there is a dawn.\npub type Backdrop = (&'static str, &'static [u8], Option<&'static [u8]>);\n\n/// The pictures of `assets/backdrops`.\npub const BACKDROPS: &[Backdrop] = &[\n");
    for f in files.iter().filter(|f| !is_dawn(&stem(f))) {
        println!("cargo:rerun-if-changed={}", f.display());
        let name = stem(f);
        let dawn = files.iter().find(|d| is_dawn(&stem(d)) && scene_key(&stem(d)) == scene_key(&name));
        let dawn = match dawn {
            Some(d) => format!("Some(include_bytes!({:?}))", d.display().to_string()),
            None => "None".to_owned(),
        };
        code.push_str(&format!("    ({name:?}, include_bytes!({:?}), {dawn}),\n", f.display().to_string()));
    }
    code.push_str("];\n");
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("backdrops.rs");
    std::fs::write(out, code).expect("writing backdrops.rs");
}

/// `MASSIF_COMMIT`: the commit the app is built from, for a problem report.
/// Given by the install script (`MASSIF_COMMIT`) or GitHub's build
/// (`GITHUB_SHA`), else asked of git; empty when none says.
fn commit() {
    println!("cargo:rerun-if-env-changed=MASSIF_COMMIT");
    println!("cargo:rerun-if-env-changed=GITHUB_SHA");
    let git = |args: &[&str]| {
        std::process::Command::new("git").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned()).filter(|s| !s.is_empty())
    };
    let sha = std::env::var("MASSIF_COMMIT").ok().filter(|s| !s.is_empty()).or_else(|| std::env::var("GITHUB_SHA").ok().filter(|s| !s.is_empty())).or_else(|| git(&["rev-parse", "HEAD"]));
    // Built again when the checkout moves to another commit.
    for path in ["HEAD", "logs/HEAD"] {
        if let Some(p) = git(&["rev-parse", "--path-format=absolute", "--git-path", path]) {
            println!("cargo:rerun-if-changed={p}");
        }
    }
    let short: String = sha.unwrap_or_default().chars().take(9).collect();
    println!("cargo:rustc-env=MASSIF_COMMIT={short}");
}
