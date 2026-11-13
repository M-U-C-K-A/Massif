//! Height maps: greyscale images of a relief, to and from other tools
//! (Gaea, World Machine, WorldPainter, image editors).
//!
//! Read: PNG or JPEG, 8 or 16 bits a channel (colour is made grey), or raw
//! square grids of little-endian 16-bit (`.r16`, `.raw`) or 32-bit float
//! (`.r32`) samples. Written: 16-bit greyscale PNG. Values are fractions,
//! 0 for black and 1 for white; what heights they stand for is the
//! caller's.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::SystemTime;

/// A height map: `width` × `height` samples, row by row, 0 to 1.
#[derive(Debug, Clone, PartialEq)]
pub struct Heightmap {
    pub width: usize,
    pub height: usize,
    pub data: Vec<f32>,
}

impl Heightmap {
    /// The value at fractional pixel `(u, v)`, bilinear, clamped to the
    /// edges.
    pub fn sample(&self, u: f32, v: f32) -> f32 {
        let (w, h) = (self.width, self.height);
        let (u, v) = (u.clamp(0.0, (w - 1) as f32), v.clamp(0.0, (h - 1) as f32));
        let (i, j) = (u.floor() as usize, v.floor() as usize);
        let (i1, j1) = ((i + 1).min(w - 1), (j + 1).min(h - 1));
        let (fu, fv) = (u - i as f32, v - j as f32);
        let at = |i: usize, j: usize| self.data[j * w + i];
        let top = at(i, j) + (at(i1, j) - at(i, j)) * fu;
        let bottom = at(i, j1) + (at(i1, j1) - at(i, j1)) * fu;
        top + (bottom - top) * fv
    }
}

/// Read a height map from a file.
pub fn read(path: &Path) -> Result<Heightmap, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_ascii_lowercase();
    let raw = |bytes: usize| -> Result<Heightmap, String> {
        let data = std::fs::read(path).map_err(|e| e.to_string())?;
        let n = data.len() / bytes;
        let side = (n as f64).sqrt().round() as usize;
        if side < 2 || side * side * bytes != data.len() {
            return Err(format!("a raw height map must be square: {} bytes is not one", data.len()));
        }
        let values = data
            .chunks_exact(bytes)
            .map(|c| if bytes == 2 { u16::from_le_bytes([c[0], c[1]]) as f32 / 65535.0 } else { f32::from_le_bytes([c[0], c[1], c[2], c[3]]) })
            .collect();
        Ok(Heightmap {
            width: side,
            height: side,
            data: values,
        })
    };
    match ext.as_str() {
        "r16" | "raw" => raw(2),
        "r32" => raw(4),
        _ => {
            let img = image::open(path).map_err(|e| e.to_string())?.into_luma16();
            let (w, h) = img.dimensions();
            Ok(Heightmap {
                width: w as usize,
                height: h as usize,
                data: img.into_raw().into_iter().map(|v| v as f32 / 65535.0).collect(),
            })
        }
    }
}

/// [`read`], kept while the file stays as it is: graphs read their height
/// maps at every preview.
pub fn cached(path: &Path) -> Result<Arc<Heightmap>, String> {
    type Cache = Mutex<HashMap<PathBuf, (Option<SystemTime>, Arc<Heightmap>)>>;
    static CACHE: OnceLock<Cache> = OnceLock::new();
    let modified = std::fs::metadata(path).and_then(|m| m.modified()).ok();
    let cache = CACHE.get_or_init(Default::default);
    if let Some((when, map)) = cache.lock().expect("not poisoned").get(path)
        && *when == modified
    {
        return Ok(map.clone());
    }
    let map = Arc::new(read(path)?);
    cache.lock().expect("not poisoned").insert(path.to_path_buf(), (modified, map.clone()));
    Ok(map)
}

/// Write `samples` (`width` × `height`, 0 to 1) as a 16-bit greyscale PNG.
pub fn write_png16(path: &Path, width: usize, height: usize, samples: &[f32]) -> Result<(), String> {
    let pixels: Vec<u16> = samples.iter().map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16).collect();
    let img = image::ImageBuffer::<image::Luma<u16>, Vec<u16>>::from_raw(width as u32, height as u32, pixels).ok_or("the image does not have its size")?;
    img.save_with_format(path, image::ImageFormat::Png).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn height_maps_go_out_and_back() {
        let dir = std::env::temp_dir().join(format!("massif-heightmap-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let (w, h) = (40, 30);
        let samples: Vec<f32> = (0..w * h).map(|k| (k % w) as f32 / (w - 1) as f32 * 0.5 + (k / w) as f32 / (h - 1) as f32 * 0.5).collect();
        let png = dir.join("relief.png");
        write_png16(&png, w, h, &samples).unwrap();
        let back = read(&png).unwrap();
        assert_eq!((back.width, back.height), (w, h));
        assert!(back.data.iter().zip(&samples).all(|(a, b)| (a - b).abs() < 1.0 / 60000.0), "16 bits kept");
        assert!((back.sample(19.5, 14.5) - 0.5).abs() < 0.02);
        // Raw 16-bit, square.
        let r16 = dir.join("relief.r16");
        let bytes: Vec<u8> = (0..16 * 16).flat_map(|k| ((k * 256) as u16).to_le_bytes()).collect();
        std::fs::write(&r16, bytes).unwrap();
        let raw = read(&r16).unwrap();
        assert_eq!((raw.width, raw.height), (16, 16));
        assert!((raw.data[255] - 255.0 * 256.0 / 65535.0).abs() < 1e-5);
        assert!(cached(&png).is_ok() && Arc::ptr_eq(&cached(&png).unwrap(), &cached(&png).unwrap()));
        let _ = std::fs::remove_dir_all(dir);
    }
}
