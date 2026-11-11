//! Minecraft's real blocks in the editor: read from the player's own
//! installation of the game (see `massif_assets`), drawn with their real
//! textures, and paintable on the map.

use egui::{ColorImage, TextureHandle, TextureOptions};
use massif_assets::{Block, Catalogue, Jar};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// Pixels of the icons (drawn at half that in points, for sharp edges).
const ICON: usize = 48;

type Loaded = (Vec<Jar>, Result<Catalogue, String>);

#[derive(Default)]
pub struct BlockLibrary {
    /// Client jars found on this computer, newest first.
    pub jars: Vec<Jar>,
    pub catalogue: Option<Arc<Catalogue>>,
    pending: Option<Receiver<Loaded>>,
    icons: HashMap<String, TextureHandle>,
    pub error: Option<String>,
    /// The world whose registry has the real colours.
    pub applied: Option<u64>,
    pub search: String,
    /// Show the blocks that are not whole cubes (flowers, torches…).
    pub all: bool,
}

impl BlockLibrary {
    pub fn is_loading(&self) -> bool {
        self.pending.is_some()
    }

    /// Find the game's jars and read `jar` (or the newest) on another thread.
    pub fn load(&mut self, ctx: &egui::Context, jar: Option<PathBuf>) {
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let jars = massif_assets::find_jars();
            let chosen = jar
                .filter(|p| p.is_file())
                .map(|p| {
                    let version = jars
                        .iter()
                        .find(|j| j.path == p)
                        .map(|j| j.version.clone())
                        .unwrap_or_else(|| p.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default());
                    Jar { version, path: p }
                })
                .or_else(|| jars.first().cloned());
            let result = match chosen {
                Some(j) => massif_assets::load(&j.path, &j.version).map_err(|e| format!("{}: {e}", j.path.display())),
                None => Err("No Minecraft installation was found. Choose the game's jar (versions/<version>/<version>.jar).".to_owned()),
            };
            let _ = tx.send((jars, result));
            ctx.request_repaint();
        });
        self.pending = Some(rx);
    }

    /// Take the result of a load. Returns whether a catalogue arrived.
    pub fn poll(&mut self) -> bool {
        let Some(rx) = &self.pending else { return false };
        let Ok((jars, result)) = rx.try_recv() else { return false };
        self.pending = None;
        self.jars = jars;
        match result {
            Ok(cat) => {
                self.catalogue = Some(Arc::new(cat));
                self.error = None;
                self.icons.clear();
                self.applied = None;
                true
            }
            Err(e) => {
                self.error = Some(e);
                false
            }
        }
    }

    /// The icon of a block, made the first time.
    pub fn icon(&mut self, ctx: &egui::Context, block: &Block) -> Option<egui::TextureId> {
        let cat = self.catalogue.as_ref()?;
        if let Some(t) = self.icons.get(&block.id) {
            return Some(t.id());
        }
        let rgba = if block.full_cube {
            massif_assets::cube_icon(cat, block, ICON)
        } else {
            massif_assets::flat_icon(cat, block, ICON)
        };
        let image = ColorImage::from_rgba_unmultiplied([ICON, ICON], &rgba);
        // Nearest filtering keeps the pixels of the textures crisp.
        let texture = ctx.load_texture(format!("block {}", block.id), image, TextureOptions::NEAREST);
        let id = texture.id();
        self.icons.insert(block.id.clone(), texture);
        Some(id)
    }
}
