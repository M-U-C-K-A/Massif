//! View ▸ Minecraft World Around: the land Minecraft makes past the map's
//! edges for the world's seed, drawn faint around the map in the 2D map
//! (and its heights in the 3D View), so that what the game will add can be
//! seen beside what the map has. Worked out by `massif_core::vanilla` at
//! once, a column every few blocks; the game's own land where its server
//! made it (see `game_server`).

use egui::{Color32, ColorImage, TextureHandle};
use massif_core::expand::Surroundings;
use massif_core::vanilla::{Biome, Land, VanillaAround};
use massif_core::{BlockRect, TILE_SIZE, TilePos};
use rayon::prelude::*;
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

/// How far past the map the world is drawn, in blocks.
pub const REACH: i32 = 1024;
/// How faint the world around is drawn.
pub const OPACITY: f32 = 0.6;
/// The most pixels the picture has on a side.
const MOST: i32 = 2048;

/// The picture of the world around the map, and the one being made.
#[derive(Default)]
pub struct Ghost {
    pub on: bool,
    texture: Option<(TextureHandle, [i32; 4])>,
    /// What the picture shows: the seed, the map's tiles, whether the
    /// game's own land was in.
    shown: Option<u64>,
    coming: Option<(u64, Receiver<(ColorImage, [i32; 4])>)>,
}

/// The ground's colour of a biome, as the map's own colours go.
pub fn biome_color(b: Biome) -> [u8; 3] {
    use Biome::*;
    match b {
        DeepFrozenOcean | FrozenOcean => [118, 138, 196],
        DeepColdOcean | ColdOcean => [52, 84, 168],
        DeepOcean | Ocean => [44, 76, 172],
        DeepLukewarmOcean | LukewarmOcean => [48, 104, 184],
        WarmOcean => [56, 128, 196],
        River => [62, 110, 210],
        FrozenRiver => [150, 170, 220],
        Desert | Beach => [219, 207, 163],
        SnowyBeach => [236, 232, 214],
        Badlands | ErodedBadlands | WoodedBadlands => [190, 104, 52],
        SnowyPlains | IceSpikes | SnowySlopes | FrozenPeaks | SnowyTaiga | Grove => [236, 240, 244],
        JaggedPeaks | StonyPeaks | StonyShore | WindsweptGravellyHills => [136, 136, 132],
        WindsweptHills | WindsweptForest => [104, 130, 92],
        Savanna | SavannaPlateau | WindsweptSavanna => [168, 166, 84],
        Jungle | SparseJungle | BambooJungle => [62, 132, 40],
        Swamp => [86, 104, 62],
        MangroveSwamp => [78, 112, 52],
        DarkForest | PaleGarden => [56, 94, 40],
        Taiga | OldGrowthPineTaiga | OldGrowthSpruceTaiga => [74, 108, 76],
        Forest | BirchForest | OldGrowthBirchForest | FlowerForest => [82, 128, 54],
        MushroomFields => [130, 104, 124],
        CherryGrove => [208, 160, 182],
        _ => [110, 158, 70],
    }
}

/// The picture of the land of `around` over `rect`, a pixel every `step`
/// blocks, clear over the tiles of `map`.
pub fn picture(around: &VanillaAround, rect: BlockRect, map: &HashSet<TilePos>, step: i32) -> ColorImage {
    let (w, h) = ((rect.width() / step) as usize, (rect.depth() / step) as usize);
    let column = |px: usize, pz: usize| (rect.x0 + px as i32 * step + step / 2, rect.z0 + pz as i32 * step + step / 2);
    let pixels: Vec<Color32> = (0..h)
        .into_par_iter()
        .flat_map_iter(|pz| {
            (0..w).map(move |px| {
                let (x, z) = column(px, pz);
                if map.contains(&TilePos::containing(x, z)) {
                    return Color32::TRANSPARENT;
                }
                let height = around.height(x, z);
                let wet = around.fluid(x, z).filter(|f| *f > height);
                let base = biome_color(around.biome_at(x, z));
                // Light from the north-west, from the slope across a pixel.
                let east = around.height(x + step, z);
                let south = around.height(x, z + step);
                let shade = (1.0 - ((east - height) + (south - height)) / step as f32 * 0.35).clamp(0.55, 1.35);
                let c = match wet {
                    Some(level) => {
                        let depth = ((level - height) / 40.0).clamp(0.0, 1.0);
                        [(40.0 + 20.0 * (1.0 - depth)) as u8, (70.0 + 50.0 * (1.0 - depth)) as u8, (150.0 + 60.0 * (1.0 - depth)) as u8]
                    }
                    None => base.map(|v| (v as f32 * shade).clamp(0.0, 255.0) as u8),
                };
                Color32::from_rgb(c[0], c[1], c[2])
            })
        })
        .collect();
    ColorImage::new([w, h], pixels)
}

/// The blocks a picture of the world around `map` covers, and the blocks
/// a pixel.
pub fn extent(map: &HashSet<TilePos>) -> Option<(BlockRect, i32)> {
    let (lo, hi) = massif_core::expand::bounds(map)?;
    let rect = BlockRect::new(lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE).expand(REACH);
    let side = rect.width().max(rect.depth());
    let step = [4, 8, 16, 32, 64].into_iter().find(|s| side / s <= MOST).unwrap_or(64);
    Some((rect, step))
}

impl Ghost {
    /// Keep the picture to the seed and the map: one is made again in the
    /// background when either changed.
    pub fn update(&mut self, ctx: &egui::Context, seed: i64, map: &HashSet<TilePos>, reg: &massif_core::Registry, land: Option<&Arc<Land>>) {
        if !self.on {
            return;
        }
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            seed.hash(&mut h);
            let mut tiles: Vec<(i32, i32)> = map.iter().map(|p| (p.x, p.z)).collect();
            tiles.sort_unstable();
            tiles.hash(&mut h);
            land.filter(|l| l.seed == seed).map(|l| l.chunks.len()).hash(&mut h);
            h.finish()
        };
        if let Some((k, rx)) = &self.coming {
            if let Ok((image, blocks)) = rx.try_recv() {
                self.shown = Some(*k);
                self.texture = Some((ctx.load_texture("minecraft world around", image, egui::TextureOptions::LINEAR), blocks));
                self.coming = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        if self.shown == Some(key) || self.coming.as_ref().is_some_and(|c| c.0 == key) {
            return;
        }
        let Some((rect, step)) = extent(map) else { return };
        let (tx, rx) = channel();
        let around = VanillaAround::new(seed, reg, land.cloned());
        let map = map.clone();
        let repaint = ctx.clone();
        let _ = std::thread::Builder::new().name("massif world around".into()).spawn(move || {
            let image = picture(&around, rect, &map, step);
            let _ = tx.send((image, [rect.x0, rect.z0, rect.x1, rect.z1]));
            repaint.request_repaint();
        });
        self.coming = Some((key, rx));
    }

    /// The picture to draw under the map, its blocks and its opacity.
    pub fn drawn(&self) -> Option<(egui::TextureId, [i32; 4], f32)> {
        if !self.on {
            return None;
        }
        self.texture.as_ref().map(|(t, r)| (t.id(), *r, OPACITY))
    }

    /// Whether the picture is up to date.
    #[cfg(test)]
    pub fn ready(&self) -> bool {
        self.texture.is_some() && self.coming.is_none()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_picture_is_clear_over_the_map() {
        let reg = massif_core::Registry::vanilla();
        let around = VanillaAround::new(12345, &reg, None);
        let map: HashSet<TilePos> = [TilePos::new(0, 0)].into_iter().collect();
        let (rect, step) = extent(&map).unwrap();
        assert_eq!((rect.width(), step), (128 + 2 * REACH, 4));
        let t = std::time::Instant::now();
        let image = picture(&around, rect, &map, step);
        eprintln!("{} × {} pixels in {:.2} s", image.size[0], image.size[1], t.elapsed().as_secs_f32());
        let at = |x: i32, z: i32| image.pixels[((z - rect.z0) / step) as usize * image.size[0] + ((x - rect.x0) / step) as usize];
        assert_eq!(at(64, 64), Color32::TRANSPARENT);
        assert_ne!(at(-300, 64).a(), 0);
    }
}
