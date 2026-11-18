//! Which files a script may touch: those under its own folder, those the
//! user picked (a file parameter, a path given on the command line, a
//! folder allowed with `--allow`), and, to export a map, Minecraft's saves.
//! Everything else is refused with a message saying how to allow it.
//! Scripts have no network at all: QuickJS has none, and none is given.

use std::path::{Component, Path, PathBuf};

#[derive(Debug, Clone, Default)]
pub struct Access {
    /// Where relative paths start.
    pub base: PathBuf,
    /// Folders under which anything may be read and written.
    pub roots: Vec<PathBuf>,
    /// Single files that may be read and written.
    pub files: Vec<PathBuf>,
    /// Minecraft's saves, where maps may be exported.
    pub saves: Option<PathBuf>,
}

/// `path` made absolute and plain (no `.` or `..`), its existing part
/// resolved through links, so that `a/../../b` or a link cannot leave a
/// folder unnoticed.
pub fn resolve(path: &Path) -> PathBuf {
    let mut plain = PathBuf::new();
    for c in path.components() {
        match c {
            Component::ParentDir => {
                plain.pop();
            }
            Component::CurDir => {}
            other => plain.push(other.as_os_str()),
        }
    }
    // The longest existing ancestor, through its links; the rest as is.
    let mut existing = plain.clone();
    let mut rest = Vec::new();
    loop {
        if let Ok(real) = std::fs::canonicalize(&existing) {
            let mut out = real;
            for part in rest.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (existing.file_name().map(|n| n.to_owned()), existing.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return plain,
        }
    }
}

impl Access {
    /// Allow `path`: a folder and all under it, or one file.
    pub fn allow(&mut self, path: &Path) {
        let real = resolve(&self.absolute(path));
        if real.is_dir() {
            self.roots.push(real);
        } else {
            self.files.push(real);
        }
    }

    fn absolute(&self, path: &Path) -> PathBuf {
        if path.is_absolute() { path.to_path_buf() } else { self.base.join(path) }
    }

    /// The real path `name` stands for, if the script may use it; `export`
    /// lets Minecraft's saves in.
    pub fn check(&self, name: &str, export: bool) -> Result<PathBuf, String> {
        if name.is_empty() {
            return Err("Source file not specified".into());
        }
        let path = resolve(&self.absolute(Path::new(name)));
        let under = |root: &PathBuf| path.starts_with(root);
        if self.roots.iter().any(under) || self.files.contains(&path) || (export && self.saves.as_ref().is_some_and(|s| path.starts_with(resolve(s)))) {
            return Ok(path);
        }
        Err(format!(
            "Massif scripts may only use files in the script's folder or ones you picked: {} is outside them. Copy it next to the script, give it as a file parameter or an argument, or allow its folder (massif-cli script --allow <folder>)",
            path.display()
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_the_script_folder_and_picked_files() {
        let dir = std::env::temp_dir().join(format!("massif-access-{}", std::process::id()));
        let scripts = dir.join("scripts");
        std::fs::create_dir_all(scripts.join("masks")).unwrap();
        std::fs::write(dir.join("picked.png"), b"x").unwrap();
        let mut a = Access { base: scripts.clone(), ..Default::default() };
        a.allow(&scripts);
        assert!(a.check("masks/a.png", false).is_ok());
        assert!(a.check("new/out.massif", false).is_ok(), "a file to write under the folder");
        assert!(a.check("../picked.png", false).is_err());
        assert!(a.check("masks/../../picked.png", false).is_err());
        assert!(a.check("/etc/passwd", false).is_err());
        a.allow(&dir.join("picked.png"));
        assert!(a.check("../picked.png", false).is_ok());
        assert!(a.check("../other.png", false).is_err());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/etc", scripts.join("etc")).unwrap();
            assert!(a.check("etc/passwd", false).is_err(), "a link out of the folder");
        }
        std::fs::remove_dir_all(&dir).ok();
    }
}
