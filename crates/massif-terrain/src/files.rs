//! Files on disk for graphs that read them: images decoded once and kept
//! while they do not change, paths relative to a folder (the world's).

use crate::kernel::{Env, Image};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// An environment with files and no map: what tests and the command line
/// use, and what the app wraps with its map.
#[derive(Debug, Default)]
pub struct DiskEnv {
    root: PathBuf,
    images: Mutex<HashMap<PathBuf, (u64, Arc<Image>)>>,
}

impl DiskEnv {
    /// Relative paths are read from `root`.
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into(), images: Mutex::default() }
    }

    pub fn resolve(&self, path: &str) -> PathBuf {
        let p = Path::new(path);
        if p.is_absolute() { p.to_path_buf() } else { self.root.join(p) }
    }
}

/// A file's stamp: its modification time and length, mixed; 0 when it
/// cannot be read.
pub fn stamp(path: &Path) -> u64 {
    match std::fs::metadata(path) {
        Ok(m) => {
            let t = m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map_or(0, |d| d.as_nanos() as u64);
            crate::util::hash_str(&format!("{t}:{}", m.len())) | 1
        }
        Err(_) => 0,
    }
}

/// An image file decoded: grey images as one channel, others as four
/// (RGBA); integers from 0 to 1, floating-point values as stored. A
/// `.raw` or `.r16` file is a square of little-endian 16-bit samples.
pub fn decode(path: &Path) -> Option<Image> {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if ext == "raw" || ext == "r16" {
        let bytes = std::fs::read(path).ok()?;
        let n = ((bytes.len() / 2) as f64).sqrt() as usize;
        if n == 0 || n * n * 2 != bytes.len() {
            return None;
        }
        let data = bytes.as_chunks::<2>().0.iter().map(|b| u16::from_le_bytes(*b) as f32 / 65535.0).collect();
        return Some(Image { width: n as u32, height: n as u32, channels: 1, data });
    }
    let img = image::open(path).ok()?;
    let (width, height) = (img.width(), img.height());
    use image::DynamicImage::*;
    match img {
        ImageLuma8(_) | ImageLumaA8(_) | ImageLuma16(_) | ImageLumaA16(_) => Some(Image { width, height, channels: 1, data: img.to_luma32f().into_raw() }),
        other => Some(Image { width, height, channels: 4, data: other.to_rgba32f().into_raw() }),
    }
}

impl Env for DiskEnv {
    fn file_stamp(&self, path: &str) -> u64 {
        stamp(&self.resolve(path))
    }

    fn image(&self, path: &str) -> Option<Arc<Image>> {
        let full = self.resolve(path);
        let s = stamp(&full);
        if s == 0 {
            return None;
        }
        if let Some((kept, img)) = self.images.lock().expect("lock").get(&full)
            && *kept == s
        {
            return Some(img.clone());
        }
        let img = Arc::new(decode(&full)?);
        self.images.lock().expect("lock").insert(full, (s, img.clone()));
        Some(img)
    }
}
