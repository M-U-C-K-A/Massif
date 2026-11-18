//! Gives massif.exe its icon on Windows, and the build its commit and
//! version.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    commit();
    version();
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

/// `MASSIF_VERSION`: the version shown and compared with GitHub's releases.
/// A release built from a tag gives the tag's (`1.0.0-beta.1` for
/// `v1.0.0-beta.1`); else it is Cargo.toml's. A tag of other numbers than
/// Cargo.toml's stops the build: a beta of 1.0.0 is built from 1.0.0's code.
fn version() {
    println!("cargo:rerun-if-env-changed=MASSIF_VERSION");
    let cargo = std::env::var("CARGO_PKG_VERSION").unwrap();
    let version = match std::env::var("MASSIF_VERSION").ok().map(|v| v.trim().trim_start_matches('v').to_owned()).filter(|v| !v.is_empty()) {
        Some(v) => {
            let core = v.split(['-', '+']).next().unwrap_or("");
            assert!(core == cargo, "MASSIF_VERSION is {v} but Cargo.toml says {cargo}: tag the version Cargo.toml names");
            v
        }
        None => cargo,
    };
    println!("cargo:rustc-env=MASSIF_VERSION={version}");
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
