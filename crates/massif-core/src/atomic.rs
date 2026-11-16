//! Writing a file so that the one it replaces is never lost.
//!
//! The new content goes to a temporary file in the same folder (so that the
//! rename stays on one disk), is flushed and, when asked, synced to the
//! disk, then renamed over the old file. Whatever fails on the way (a full
//! disk, a folder that cannot be written, a file another program holds),
//! the old file is left as it was and the temporary one is removed.
//!
//! On Windows a file that OneDrive, an antivirus or a backup tool has open
//! for a moment refuses to be replaced: the rename is tried again for a
//! little while before giving up.

use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

/// A temporary file's name beside `path`, its own to this process and
/// this write: `.map.massif.massif-tmp-1234-7`.
pub fn temp_path(path: &Path) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "file".into());
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    path.with_file_name(format!(".{name}.massif-tmp-{}-{n}", std::process::id()))
}

/// Whether `path` is a temporary file [`write_with`] leaves only when the
/// process is killed while writing.
pub fn is_temp(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with('.') && n.contains(".massif-tmp-"))
}

/// Write `path` through `write`, atomically. With `durable`, the data is
/// synced to the disk before the rename and the folder after it, so that
/// a power cut leaves the old file or the new one, never an empty one.
pub fn write_with<T>(path: &Path, durable: bool, write: impl FnOnce(&mut dyn Write) -> io::Result<T>) -> io::Result<T> {
    let tmp = temp_path(path);
    let result = (|| {
        let file = std::fs::File::create(&tmp)?;
        let mut out = BufWriter::with_capacity(1 << 20, file);
        let value = write(&mut out)?;
        let file = out.into_inner().map_err(|e| e.into_error())?;
        if durable {
            file.sync_all()?;
        }
        drop(file);
        replace(&tmp, path)?;
        Ok(value)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    } else if durable {
        sync_dir(path);
    }
    result
}

/// [`write_with`] for bytes in memory.
pub fn write(path: &Path, bytes: &[u8], durable: bool) -> io::Result<()> {
    write_with(path, durable, |out| out.write_all(bytes))
}

/// Rename `from` over `to`, again for a few seconds while another program
/// holds `to` (Windows only: elsewhere a rename never waits on a reader).
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    let mut wait = std::time::Duration::from_millis(50);
    let mut waited = std::time::Duration::ZERO;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if held_by_another(&e) && waited < std::time::Duration::from_secs(3) => {
                std::thread::sleep(wait);
                waited += wait;
                wait = (wait * 2).min(std::time::Duration::from_millis(800));
            }
            other => return other,
        }
    }
}

/// Make a rename in `path`'s folder last through a power cut (Unix; on
/// Windows the rename itself is written through).
fn sync_dir(path: &Path) {
    #[cfg(unix)]
    if let Some(dir) = path.parent() {
        let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
        if let Ok(d) = std::fs::File::open(dir) {
            let _ = d.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

/// Whether `e` says another program has the file open (Windows' sharing
/// and lock violations, and the access denied a pending delete gives).
fn held_by_another(e: &io::Error) -> bool {
    cfg!(windows) && matches!(e.raw_os_error(), Some(5 | 32 | 33))
}

/// What went wrong, in words a map maker can act on: the disk is full, the
/// folder cannot be written, another program has the file. Ends by saying
/// the file that was there is unchanged, as [`write_with`] leaves it.
pub fn describe(e: &io::Error, path: &Path) -> String {
    format!("{}; the file already there was not changed.", cause(e, path))
}

/// [`describe`] without its last words, for writes that replace nothing
/// (an export into a new folder).
pub fn cause(e: &io::Error, path: &Path) -> String {
    let file = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.display().to_string());
    let folder = path.parent().map(|p| p.display().to_string()).unwrap_or_default();
    // ENOSPC 28 and EDQUOT 69/122 on Unix; ERROR_DISK_FULL 112 and
    // ERROR_HANDLE_DISK_FULL 39 on Windows.
    let full = e.kind() == io::ErrorKind::StorageFull
        || e.kind() == io::ErrorKind::QuotaExceeded
        || match e.raw_os_error() {
            Some(n) if cfg!(windows) => n == 112 || n == 39,
            Some(n) => n == 28 || n == 69 || n == 122,
            None => false,
        };
    let read_only = e.kind() == io::ErrorKind::ReadOnlyFilesystem || (cfg!(windows) && e.raw_os_error() == Some(19));
    if full {
        format!("There is not enough room on the disk to write {file}. Free some space, or save somewhere else")
    } else if read_only {
        format!("The disk holding {folder} cannot be written to (it is read-only). Save somewhere else")
    } else if cfg!(windows) && matches!(e.raw_os_error(), Some(32 | 33)) {
        format!("{file} is open in another program (OneDrive, an antivirus or a backup tool may be using it). Try again in a moment, or save somewhere else")
    } else if e.kind() == io::ErrorKind::PermissionDenied {
        format!("Massif is not allowed to write in {folder}, or {file} is locked or read-only. Save somewhere else")
    } else if e.kind() == io::ErrorKind::NotFound {
        format!("The folder {folder} is not there any more. Save somewhere else")
    } else {
        format!("{file} could not be written ({e})")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("massif-atomic-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn left_over(d: &Path) -> Vec<String> {
        std::fs::read_dir(d).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect()
    }

    #[test]
    fn a_file_is_replaced_whole() {
        let d = dir("whole");
        let path = d.join("map.massif");
        write(&path, b"old", true).unwrap();
        write(&path, b"new and longer", true).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"new and longer");
        assert_eq!(left_over(&d), ["map.massif"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A writer that fails midway, as on a full disk: the old file stays,
    /// no temporary file is left, and the message says the disk is full.
    #[test]
    fn a_full_disk_leaves_the_old_file() {
        let d = dir("full");
        let path = d.join("map.massif");
        write(&path, b"the map as it was", true).unwrap();
        let full = io::Error::from(io::ErrorKind::StorageFull);
        let err = write_with(&path, true, |out| {
            out.write_all(&[7; 3 << 20])?;
            Err::<(), _>(io::Error::from(io::ErrorKind::StorageFull))
        })
        .unwrap_err();
        assert_eq!(err.kind(), full.kind());
        assert_eq!(std::fs::read(&path).unwrap(), b"the map as it was");
        assert_eq!(left_over(&d), ["map.massif"]);
        assert!(describe(&err, &path).starts_with("There is not enough room on the disk to write map.massif."));
        let enospc = io::Error::from_raw_os_error(if cfg!(windows) { 112 } else { 28 });
        assert!(describe(&enospc, &path).starts_with("There is not enough room"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn a_folder_that_cannot_be_written_leaves_the_old_file() {
        use std::os::unix::fs::PermissionsExt;
        let d = dir("readonly");
        let path = d.join("map.massif");
        write(&path, b"the map as it was", true).unwrap();
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o555)).unwrap();
        let result = write(&path, b"new", true);
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Root may write anywhere: only check when refused.
        if let Err(e) = result {
            assert!(describe(&e, &path).starts_with("Massif is not allowed to write in"), "{}", describe(&e, &path));
            assert_eq!(std::fs::read(&path).unwrap(), b"the map as it was");
            assert_eq!(left_over(&d), ["map.massif"]);
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// The rename fails (a folder stands where the file goes): nothing is
    /// left behind.
    #[test]
    fn a_failed_rename_leaves_nothing_behind() {
        let d = dir("rename");
        let path = d.join("map.massif");
        std::fs::create_dir(&path).unwrap();
        std::fs::write(path.join("inside"), b"x").unwrap();
        assert!(write(&path, b"new", false).is_err());
        assert_eq!(left_over(&d), ["map.massif"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn temporary_files_are_told_apart() {
        let t = temp_path(Path::new("/a/map.massif"));
        assert!(is_temp(&t), "{}", t.display());
        assert_eq!(t.parent(), Some(Path::new("/a")));
        assert!(!is_temp(Path::new("/a/map.massif")));
    }

    /// A real full disk: a 2 MB disk image (macOS only, run by hand:
    /// `cargo test -p massif-core atomic -- --ignored`).
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn a_really_full_disk_leaves_the_old_file() {
        let d = dir("image");
        let image = d.join("tiny.dmg");
        let mount = d.join("mnt");
        let run = |args: &[&str]| assert!(std::process::Command::new("hdiutil").args(args).status().unwrap().success(), "{args:?}");
        run(&["create", "-size", "2m", "-fs", "HFS+", "-volname", "MassifFull", image.to_str().unwrap()]);
        run(&["attach", "-nobrowse", "-mountpoint", mount.to_str().unwrap(), image.to_str().unwrap()]);
        let path = mount.join("map.massif");
        write(&path, &[1; 600 << 10], true).unwrap();
        let err = write(&path, &[2; 1500 << 10], true).unwrap_err();
        let result = (describe(&err, &path), std::fs::read(&path).unwrap(), left_over(&mount));
        run(&["detach", mount.to_str().unwrap()]);
        let (message, kept, files) = result;
        assert!(message.starts_with("There is not enough room"), "{message}");
        assert_eq!(kept, vec![1; 600 << 10]);
        assert!(!files.iter().any(|f| f.contains("massif-tmp")), "{files:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
