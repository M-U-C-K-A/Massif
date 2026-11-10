//! The brushes, the WorldPainter way: twelve built-in brushes (a circle or a
//! square with one of six profiles), generated terrain stamps, and custom
//! brushes made from greyscale images (white paints, black does not).
//!
//! Custom brushes live in Massif's `brushes` folder; every folder inside it
//! is a group of its own. WorldPainter's brushes folders are read too, so
//! brushes made for WorldPainter show up without copying anything.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};
use massif_core::stamps::Stamp;
use massif_core::{Brush, BrushMask, BrushShape, Profile};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// Largest side of a brush image kept in memory; bigger images are scaled
/// down (a brush is stretched over its diameter anyway).
const MAX_IMAGE_SIDE: u32 = 1024;
/// Image files read as brushes.
const EXTENSIONS: [&str; 3] = ["png", "jpg", "jpeg"];

/// Where a brush comes from. Saved with the tool settings, so that the same
/// brush is picked again at the next start.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BrushSource {
    Symmetric(BrushShape, Profile),
    Stamp(Stamp),
    Image(PathBuf),
}

impl Default for BrushSource {
    fn default() -> Self {
        BrushSource::Symmetric(BrushShape::Circle, Profile::Sine)
    }
}

impl BrushSource {
    /// The six brushes WorldPainter shows first.
    pub const BASIC: [BrushSource; 6] = [
        BrushSource::Symmetric(BrushShape::Circle, Profile::Sine),
        BrushSource::Symmetric(BrushShape::Circle, Profile::Plateau),
        BrushSource::Symmetric(BrushShape::Square, Profile::Constant),
        BrushSource::Symmetric(BrushShape::Circle, Profile::Linear),
        BrushSource::Symmetric(BrushShape::Circle, Profile::Spike),
        BrushSource::Symmetric(BrushShape::Circle, Profile::Dome),
    ];

    /// The other six of the twelve symmetric brushes.
    pub const MORE: [BrushSource; 6] = [
        BrushSource::Symmetric(BrushShape::Circle, Profile::Constant),
        BrushSource::Symmetric(BrushShape::Square, Profile::Sine),
        BrushSource::Symmetric(BrushShape::Square, Profile::Plateau),
        BrushSource::Symmetric(BrushShape::Square, Profile::Linear),
        BrushSource::Symmetric(BrushShape::Square, Profile::Spike),
        BrushSource::Symmetric(BrushShape::Square, Profile::Dome),
    ];
}

/// One brush of the library.
#[derive(Debug, Clone)]
pub struct BrushEntry {
    pub source: BrushSource,
    pub name: String,
    /// The image of a stamp or custom brush.
    pub mask: Option<Arc<BrushMask>>,
}

/// A group of custom brushes: one folder.
#[derive(Debug, Clone)]
pub struct BrushGroup {
    pub name: String,
    pub dir: PathBuf,
    /// Massif's own folders take new brushes; WorldPainter's are only read.
    pub writable: bool,
    pub entries: Vec<BrushEntry>,
}

/// What loading the library produced.
struct Loaded {
    stamps: Vec<BrushEntry>,
    groups: Vec<BrushGroup>,
    errors: Vec<String>,
}

/// Every brush there is, with the thumbnails the panels show.
pub struct BrushLibrary {
    /// Massif's brushes folder (made when a brush is first added).
    pub own_dir: Option<PathBuf>,
    /// Brushes folders of WorldPainter installations: (label, folder).
    pub foreign_dirs: Vec<(String, PathBuf)>,
    pub stamps: Vec<BrushEntry>,
    pub groups: Vec<BrushGroup>,
    pub errors: Vec<String>,
    loaded: bool,
    pending: Option<Receiver<Loaded>>,
    thumbs: HashMap<(BrushSource, bool), TextureHandle>,
    preview: Option<((BrushSource, i32, bool), TextureHandle)>,
}

impl Default for BrushLibrary {
    fn default() -> Self {
        Self::new(None, Vec::new())
    }
}

impl BrushLibrary {
    pub fn new(own_dir: Option<PathBuf>, foreign_dirs: Vec<(String, PathBuf)>) -> Self {
        Self {
            own_dir,
            foreign_dirs,
            stamps: Vec::new(),
            groups: Vec::new(),
            errors: Vec::new(),
            loaded: false,
            pending: None,
            thumbs: HashMap::new(),
            preview: None,
        }
    }

    /// The library of this installation: Massif's brushes folder and those
    /// of WorldPainter.
    pub fn installed() -> Self {
        let own = eframe::storage_dir("massif").map(|d| d.join("brushes"));
        Self::new(own, worldpainter_dirs())
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    /// Load (or reload) the stamps and the custom brushes on another thread.
    pub fn load_in_background(&mut self, ctx: &egui::Context) {
        let (tx, rx) = channel();
        let (own, foreign) = (self.own_dir.clone(), self.foreign_dirs.clone());
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(load(own.as_deref(), &foreign));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    /// Load everything now (tests).
    #[cfg(test)]
    pub fn load_now(&mut self) {
        let loaded = load(self.own_dir.as_deref(), &self.foreign_dirs);
        self.install(loaded);
    }

    /// Take the result of a background load. Returns whether it arrived.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        match rx.try_recv() {
            Ok(loaded) => {
                self.pending = None;
                self.install(loaded);
                true
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                self.pending = None;
                false
            }
        }
    }

    fn install(&mut self, loaded: Loaded) {
        self.stamps = loaded.stamps;
        self.groups = loaded.groups;
        self.errors = loaded.errors;
        self.loaded = true;
        // Thumbnails of images that changed would be stale.
        self.thumbs.retain(|(s, _), _| !matches!(s, BrushSource::Image(_)));
        self.preview = None;
    }

    /// The image of a stamp or custom brush, once loaded.
    pub fn mask(&self, source: &BrushSource) -> Option<Arc<BrushMask>> {
        match source {
            BrushSource::Symmetric(..) => None,
            BrushSource::Stamp(stamp) => self
                .stamps
                .iter()
                .find(|e| e.source == *source)
                .and_then(|e| e.mask.clone())
                .or_else(|| Some(Arc::new(stamp.render()))),
            BrushSource::Image(_) => self
                .groups
                .iter()
                .flat_map(|g| &g.entries)
                .find(|e| e.source == *source)
                .and_then(|e| e.mask.clone()),
        }
    }

    /// The brush `source` stands for, keeping the size, intensity and
    /// rotation of `current`. `None` while a custom brush is not loaded.
    pub fn brush(&self, source: &BrushSource, current: &Brush) -> Option<Brush> {
        let mut brush = current.clone();
        match source {
            BrushSource::Symmetric(shape, profile) => {
                brush.shape = *shape;
                brush.profile = *profile;
                brush.mask = None;
            }
            _ => brush.mask = Some(self.mask(source)?),
        }
        Some(brush)
    }

    /// The name of a brush, for tooltips and the settings panel.
    pub fn name(&self, source: &BrushSource) -> String {
        match source {
            BrushSource::Symmetric(shape, profile) => Brush::symmetric(*shape, *profile, 1.0).name(),
            BrushSource::Stamp(stamp) => stamp.label().to_owned(),
            BrushSource::Image(path) => image_name(path),
        }
    }

    /// The thumbnail of a brush: a small shaded relief.
    pub fn thumbnail(&mut self, ctx: &egui::Context, source: &BrushSource, dark: bool) -> Option<egui::TextureId> {
        let key = (source.clone(), dark);
        if let Some(t) = self.thumbs.get(&key) {
            return Some(t.id());
        }
        if matches!(source, BrushSource::Stamp(_)) && !self.loaded {
            return None; // generated with the library, on another thread
        }
        let brush = self.brush(source, &Brush::default())?;
        let image = relief(&brush, 64, dark);
        let texture = ctx.load_texture(format!("brush {source:?} {dark}"), image, TextureOptions::LINEAR);
        let id = texture.id();
        self.thumbs.insert(key, texture);
        Some(id)
    }

    /// A larger relief of the current brush, turned as it paints.
    pub fn preview(&mut self, ctx: &egui::Context, source: &BrushSource, brush: &Brush, dark: bool) -> egui::TextureId {
        let key = (source.clone(), brush.rotation.round() as i32, dark);
        if let Some((k, t)) = &self.preview
            && *k == key
        {
            return t.id();
        }
        let image = relief(brush, 160, dark);
        let texture = ctx.load_texture("brush preview", image, TextureOptions::LINEAR);
        let id = texture.id();
        self.preview = Some((key, texture));
        id
    }

    /// Copy images into a group's folder (Massif's own folder when `group`
    /// is `None`), under names not taken yet. Returns how many were added.
    pub fn add_images(&self, group: Option<&Path>, files: &[PathBuf]) -> std::io::Result<usize> {
        let dir = match group {
            Some(dir) => dir.to_path_buf(),
            None => self.own_dir.clone().ok_or_else(|| std::io::Error::other("no folder for brushes"))?,
        };
        std::fs::create_dir_all(&dir)?;
        let mut added = 0;
        for file in files.iter().filter(|f| is_image(f)) {
            let Some(name) = file.file_name() else { continue };
            let target = free_name(&dir, Path::new(name));
            std::fs::copy(file, &target)?;
            added += 1;
        }
        Ok(added)
    }

    /// Make a new group: a folder in Massif's brushes folder.
    pub fn new_group(&self, name: &str) -> std::io::Result<PathBuf> {
        let name = name.trim();
        if name.is_empty() || name.contains(['/', '\\', ':']) || name.starts_with('.') {
            return Err(std::io::Error::other("a group name cannot be empty or contain / \\ :"));
        }
        let own = self.own_dir.clone().ok_or_else(|| std::io::Error::other("no folder for brushes"))?;
        let dir = own.join(name);
        if dir.exists() {
            return Err(std::io::Error::other(format!("there is already a group called “{name}”")));
        }
        std::fs::create_dir_all(&dir)?;
        Ok(dir)
    }

    /// Show Massif's brushes folder in the file manager, making it first.
    pub fn open_folder(&self) -> std::io::Result<()> {
        let dir = self.own_dir.clone().ok_or_else(|| std::io::Error::other("no folder for brushes"))?;
        std::fs::create_dir_all(&dir)?;
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else if cfg!(target_os = "windows") {
            "explorer"
        } else {
            "xdg-open"
        };
        std::process::Command::new(opener).arg(&dir).spawn().map(|_| ())
    }
}

/// The brushes folders of WorldPainter (the modernised fork first).
fn worldpainter_dirs() -> Vec<(String, PathBuf)> {
    let mut bases = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            let support = PathBuf::from(home).join("Library/Application Support");
            bases.push(("WorldPainter".to_owned(), support.join("WorldPainter [MODERNISED]")));
            bases.push(("WorldPainter (original)".to_owned(), support.join("WorldPainter")));
        }
    } else if cfg!(target_os = "windows") {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            let appdata = PathBuf::from(appdata);
            bases.push(("WorldPainter".to_owned(), appdata.join("WorldPainter [MODERNISED]")));
            bases.push(("WorldPainter (original)".to_owned(), appdata.join("WorldPainter")));
        }
    } else if let Some(home) = std::env::var_os("HOME") {
        let home = PathBuf::from(home);
        bases.push(("WorldPainter".to_owned(), home.join(".worldpainter [modernised]")));
        bases.push(("WorldPainter (original)".to_owned(), home.join(".worldpainter")));
    }
    bases
        .into_iter()
        .map(|(label, base)| (label, base.join("brushes")))
        .filter(|(_, dir)| dir.is_dir())
        .collect()
}

fn is_image(path: &Path) -> bool {
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
    ext.is_some_and(|e| EXTENSIONS.contains(&e.as_str()))
        && !path.file_name().is_some_and(|n| n.eq_ignore_ascii_case("icon.png"))
}

/// The name of a brush made from an image: its file name without extension.
fn image_name(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `dir/name`, or `dir/name 2`, `dir/name 3`… if taken.
fn free_name(dir: &Path, name: &Path) -> PathBuf {
    let target = dir.join(name);
    if !target.exists() {
        return target;
    }
    let stem = image_name(name);
    let ext = name.extension().map(|e| e.to_string_lossy().into_owned()).unwrap_or_default();
    (2..)
        .map(|i| dir.join(format!("{stem} {i}.{ext}")))
        .find(|p| !p.exists())
        .expect("some name is free")
}

/// Read one image as a brush.
pub fn load_image(path: &Path) -> Result<BrushMask, String> {
    let image = image::open(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let image = if image.width() > MAX_IMAGE_SIDE || image.height() > MAX_IMAGE_SIDE {
        image.resize(MAX_IMAGE_SIDE, MAX_IMAGE_SIDE, image::imageops::FilterType::Triangle)
    } else {
        image
    };
    let rgba = image.to_rgba8();
    Ok(BrushMask::from_rgba(
        image_name(path),
        rgba.width() as usize,
        rgba.height() as usize,
        rgba.as_raw(),
    ))
}

/// Read the brushes of `dir` into `groups`: its images make the group
/// `name`, every folder inside a group of its own, named after the folder
/// followed by `suffix`. Empty groups are kept when `keep_empty`, so that
/// brushes can be added to them.
fn load_group(
    dir: &Path,
    name: String,
    writable: bool,
    keep_empty: bool,
    suffix: &str,
    groups: &mut Vec<BrushGroup>,
    errors: &mut Vec<String>,
) {
    use rayon::prelude::*;
    let Ok(read) = std::fs::read_dir(dir) else { return };
    let mut files = Vec::new();
    let mut subdirs = Vec::new();
    for entry in read.flatten() {
        let path = entry.path();
        if path.file_name().is_some_and(|n| n.to_string_lossy().starts_with('.')) {
            continue;
        }
        if path.is_dir() {
            subdirs.push(path);
        } else if is_image(&path) {
            files.push(path);
        }
    }
    // In the order of their names, so that they keep their places.
    files.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    subdirs.sort_by_key(|p| p.to_string_lossy().to_lowercase());
    let results: Vec<(PathBuf, Result<BrushMask, String>)> = files
        .into_par_iter()
        .map(|p| {
            let r = load_image(&p);
            (p, r)
        })
        .collect();
    let mut entries = Vec::new();
    for (path, result) in results {
        match result {
            Ok(mask) => entries.push(BrushEntry {
                name: mask.name.clone(),
                source: BrushSource::Image(path),
                mask: Some(Arc::new(mask)),
            }),
            Err(e) => errors.push(e),
        }
    }
    if !entries.is_empty() || keep_empty {
        groups.push(BrushGroup {
            name,
            dir: dir.to_path_buf(),
            writable,
            entries,
        });
    }
    for sub in subdirs {
        let folder = sub.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        load_group(&sub, format!("{folder}{suffix}"), writable, writable, suffix, groups, errors);
    }
}

fn load(own: Option<&Path>, foreign: &[(String, PathBuf)]) -> Loaded {
    use rayon::prelude::*;
    let stamps = Stamp::ALL
        .par_iter()
        .map(|&stamp| BrushEntry {
            source: BrushSource::Stamp(stamp),
            name: stamp.label().to_owned(),
            mask: Some(Arc::new(stamp.render())),
        })
        .collect();
    let mut groups = Vec::new();
    let mut errors = Vec::new();
    if let Some(own) = own {
        load_group(own, "Custom Brushes".to_owned(), true, false, "", &mut groups, &mut errors);
    }
    for (label, dir) in foreign {
        load_group(dir, label.clone(), false, false, &format!(" ({label})"), &mut groups, &mut errors);
    }
    Loaded { stamps, groups, errors }
}

/// A brush as a small shaded relief lit from the top left, so that its shape
/// reads as a landform (as WorldPainter draws its brush buttons).
pub fn relief(brush: &Brush, size: usize, dark: bool) -> ColorImage {
    let r = size as f32 / 2.0 - 1.0;
    let b = Brush {
        radius: r,
        ..brush.clone()
    };
    let n = size;
    let mut h = vec![0f32; n * n];
    for y in 0..n {
        for x in 0..n {
            let (dx, dz) = (x as f32 + 0.5 - n as f32 / 2.0, y as f32 + 0.5 - n as f32 / 2.0);
            h[y * n + x] = b.weight(dx, dz);
        }
    }
    let at = |x: isize, y: isize| -> f32 {
        if x < 0 || y < 0 || x >= n as isize || y >= n as isize {
            0.0
        } else {
            h[y as usize * n + x as usize]
        }
    };
    let relief = n as f32 * 0.35;
    let light = {
        let (lx, ly, lz) = (-1.0f32, -1.0f32, 1.3f32);
        let len = (lx * lx + ly * ly + lz * lz).sqrt();
        (lx / len, ly / len, lz / len)
    };
    let (lit, shadow) = if dark {
        ([236.0, 240.0, 248.0], [52.0, 60.0, 78.0])
    } else {
        ([214.0, 222.0, 236.0], [44.0, 52.0, 72.0])
    };
    let mut pixels = Vec::with_capacity(n * n);
    for y in 0..n as isize {
        for x in 0..n as isize {
            let v = at(x, y);
            if v <= 0.004 {
                pixels.push(Color32::TRANSPARENT);
                continue;
            }
            let gx = (at(x + 1, y) - at(x - 1, y)) * 0.5 * relief;
            let gy = (at(x, y + 1) - at(x, y - 1)) * 0.5 * relief;
            let len = (gx * gx + gy * gy + 1.0).sqrt();
            let (nx, ny, nz) = (-gx / len, -gy / len, 1.0 / len);
            let diffuse = (nx * light.0 + ny * light.1 + nz * light.2).clamp(0.0, 1.0);
            let k = (0.15 + 0.85 * diffuse) * (0.55 + 0.45 * v);
            let c = |i: usize| (shadow[i] + (lit[i] - shadow[i]) * k).round() as u8;
            let alpha = ((v / 0.05).min(1.0) * 255.0) as u8;
            pixels.push(Color32::from_rgba_unmultiplied(c(0), c(1), c(2), alpha));
        }
    }
    ColorImage::new([n, n], pixels)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("massif-brushes-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_png(path: &Path, size: u32) {
        let img = image::GrayImage::from_fn(size, size, |x, y| {
            let (dx, dy) = (x as f32 - size as f32 / 2.0, y as f32 - size as f32 / 2.0);
            image::Luma([if dx.hypot(dy) < size as f32 * 0.3 { 255 } else { 0 }])
        });
        img.save(path).unwrap();
    }

    #[test]
    fn custom_brushes_are_read_from_folders_in_groups() {
        let own = scratch("own");
        let wp = scratch("wp");
        write_png(&own.join("B.png"), 32);
        write_png(&own.join("a.png"), 32);
        std::fs::create_dir_all(own.join("Rocks")).unwrap();
        write_png(&own.join("Rocks/Boulder.png"), 16);
        std::fs::create_dir_all(own.join("Empty")).unwrap();
        write_png(&wp.join("Peak.png"), 40);
        std::fs::write(own.join("notes.txt"), "not a brush").unwrap();

        let mut lib = BrushLibrary::new(Some(own.clone()), vec![("WorldPainter".into(), wp.clone())]);
        lib.load_now();
        assert!(lib.errors.is_empty(), "{:?}", lib.errors);
        let names: Vec<&str> = lib.groups.iter().map(|g| g.name.as_str()).collect();
        assert_eq!(names, ["Custom Brushes", "Empty", "Rocks", "WorldPainter"]);
        let own_group = &lib.groups[0];
        assert!(own_group.writable);
        let brushes: Vec<&str> = own_group.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(brushes, ["a", "B"]);
        assert!(!lib.groups[3].writable);
        assert_eq!(lib.stamps.len(), Stamp::ALL.len());

        // A custom brush paints where its image is white.
        let source = BrushSource::Image(own.join("a.png"));
        let brush = lib.brush(&source, &Brush { radius: 50.0, ..Brush::default() }).unwrap();
        assert!(brush.weight(0.0, 0.0) > 0.99);
        assert_eq!(brush.weight(45.0, 0.0), 0.0);
        assert!(brush.outline().is_some());

        // Adding images copies them under free names.
        let added = lib.add_images(Some(&own.join("Rocks")), &[own.join("a.png"), own.join("notes.txt")]).unwrap();
        assert_eq!(added, 1);
        lib.add_images(Some(&own.join("Rocks")), &[own.join("a.png")]).unwrap();
        assert!(own.join("Rocks/a 2.png").exists());
        assert!(lib.new_group("Rocks").is_err());
        assert!(lib.new_group("a/b").is_err());
        lib.new_group("Cliffs").unwrap();
        lib.load_now();
        assert!(lib.groups.iter().any(|g| g.name == "Cliffs"));
        assert_eq!(lib.groups.iter().find(|g| g.name == "Rocks").unwrap().entries.len(), 3);

        let _ = std::fs::remove_dir_all(own);
        let _ = std::fs::remove_dir_all(wp);
    }

    #[test]
    fn symmetric_brushes_and_reliefs() {
        let lib = BrushLibrary::default();
        let current = Brush {
            radius: 77.0,
            strength: 0.3,
            ..Brush::default()
        };
        let b = lib.brush(&BrushSource::BASIC[2], &current).unwrap();
        assert_eq!((b.shape, b.profile, b.radius, b.strength), (BrushShape::Square, Profile::Constant, 77.0, 0.3));
        assert_eq!(lib.name(&BrushSource::BASIC[0]), "Sine Circle");
        // Stamps render on demand even before the library is loaded.
        assert!(lib.brush(&BrushSource::Stamp(Stamp::Volcano), &current).unwrap().mask.is_some());
        // A missing image is no brush.
        assert!(lib.brush(&BrushSource::Image("/nowhere.png".into()), &current).is_none());

        let img = relief(&Brush::default(), 32, true);
        assert_eq!(img.size, [32, 32]);
        assert_eq!(img.pixels[0], Color32::TRANSPARENT); // corner, outside the circle
        assert!(img.pixels[16 * 32 + 16].a() > 0);
    }
}
