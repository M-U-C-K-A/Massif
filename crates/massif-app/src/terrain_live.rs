//! Live mode on the World side: the tiles the terrain generator makes while
//! its graph is edited, lit and drawn over the map without writing it (the
//! map view draws [`LiveOverlay::pictures`]), and the view they are made
//! for ([`LiveOverlay::follow`]).

use egui::{ColorImage, Context, Rect, TextureHandle, TextureId, TextureOptions, pos2};
use massif_terrain::engine::TileData;
use massif_terrain::kernel::Grid;
use massif_terrain::live::{Live, Ready};
use massif_terrain::space::{Rect as Samples, Space, TILE, TileKey};
use std::collections::HashMap;
use std::sync::Arc;

/// Pictures kept at most: the view, the overview and some around.
const MAX_PICTURES: usize = 384;

#[derive(Default)]
struct Tile {
    revision: u64,
    height: Option<Arc<TileData>>,
    albedo: Option<Arc<TileData>>,
    picture: Option<TextureHandle>,
    used: u64,
}

/// The Live preview's pictures, tile by tile, the finest over the coarser;
/// a tile of an older version stays until the new one replaces it, so that
/// nothing flickers.
#[derive(Default)]
pub struct LiveOverlay {
    tiles: HashMap<TileKey, Tile>,
    tick: u64,
}

impl LiveOverlay {
    /// Takes the tiles Live made (outputs: 0 the height, 1 the albedo when
    /// `has_albedo`) and lights them into pictures. Whether any came.
    pub fn take(&mut self, ctx: &Context, ready: Vec<Ready>, sea: f32, has_albedo: bool) -> bool {
        if ready.is_empty() {
            return false;
        }
        self.tick += 1;
        let mut changed = Vec::new();
        for r in ready {
            let t = self.tiles.entry(r.key).or_default();
            if r.revision != t.revision {
                t.revision = r.revision;
                t.height = None;
                t.albedo = None;
            }
            match r.output {
                0 => t.height = Some(r.tile),
                _ => t.albedo = Some(r.tile),
            }
            t.used = self.tick;
            changed.push(r.key);
        }
        changed.sort_unstable();
        changed.dedup();
        for key in changed {
            let t = self.tiles.get_mut(&key).expect("just added");
            let (Some(h), albedo) = (&t.height, &t.albedo) else { continue };
            if has_albedo && albedo.is_none() {
                continue;
            }
            let rect = Samples::tile(key.x, key.z);
            let Some(heights) = h.values() else { continue };
            let grid = Grid::new(key.lod, rect, heights.to_vec());
            let colours = albedo.as_ref().and_then(|a| a.values()).map(|v| Grid::new(key.lod, rect, v.to_vec()));
            let rgb = massif_terrain::preview::shade_with(&grid, sea, colours.as_ref());
            let image = ColorImage::from_rgb([TILE as usize, TILE as usize], &rgb.into_iter().flatten().collect::<Vec<u8>>());
            match &mut t.picture {
                Some(p) => p.set(image, TextureOptions::LINEAR),
                None => t.picture = Some(ctx.load_texture(format!("live {} {} {}", key.lod, key.x, key.z), image, TextureOptions::LINEAR)),
            }
        }
        // The pictures least recently made go first.
        if self.tiles.len() > MAX_PICTURES {
            let mut ages: Vec<(u64, TileKey)> = self.tiles.iter().map(|(k, t)| (t.used, *k)).collect();
            ages.sort_unstable();
            for (_, k) in ages.into_iter().take(self.tiles.len() - MAX_PICTURES) {
                self.tiles.remove(&k);
            }
        }
        true
    }

    /// What to draw over the map: each picture, the blocks it covers (west,
    /// north, east, south) and the part of it on the terrain, the coarsest
    /// first so that the finer ones lie on top.
    pub fn pictures(&self, space: &Space) -> Vec<(TextureId, [i32; 4], Rect)> {
        let mut out: Vec<(u8, TextureId, [i32; 4], Rect)> = self
            .tiles
            .iter()
            .filter_map(|(key, t)| {
                let picture = t.picture.as_ref()?;
                let step = 1i64 << key.lod;
                let span = TILE as i64 * step;
                let (x0, z0) = (space.x0 as i64 + key.x as i64 * span, space.z0 as i64 + key.z as i64 * span);
                let (east, south) = (space.x0 as i64 + space.size as i64, space.z0 as i64 + space.size as i64);
                let (x1, z1) = ((x0 + span).min(east), (z0 + span).min(south));
                (x1 > x0 && z1 > z0).then(|| {
                    let uv = Rect::from_min_max(pos2(0.0, 0.0), pos2((x1 - x0) as f32 / span as f32, (z1 - z0) as f32 / span as f32));
                    (key.lod, picture.id(), [x0 as i32, z0 as i32, x1 as i32, z1 as i32], uv)
                })
            })
            .collect();
        out.sort_by(|a, b| b.0.cmp(&a.0));
        out.into_iter().map(|(_, id, r, uv)| (id, r, uv)).collect()
    }

    /// Asks Live for the tiles of the map's view: `visible` the blocks seen
    /// (west, north, east, south), `zoom` the screen points a block.
    pub fn follow(live: &Live, space: &Space, visible: [f64; 4], zoom: f32, pixels_per_point: f32) {
        let x0 = (visible[0] - space.x0 as f64).floor() as i32;
        let z0 = (visible[1] - space.z0 as f64).floor() as i32;
        let w = (visible[2] - visible[0]).ceil().max(1.0) as u32;
        let h = (visible[3] - visible[1]).ceil().max(1.0) as u32;
        let blocks_per_pixel = 1.0 / (zoom as f64 * pixels_per_point as f64).max(1e-6);
        live.set_view(Samples::new(x0, z0, w, h), blocks_per_pixel);
    }

    /// Forgets every picture: another map, Live off.
    pub fn clear(&mut self) {
        self.tiles.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.tiles.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_terrain::engine::Engine;
    use massif_terrain::kernel::NoEnv;
    use massif_terrain::live::Settings;
    use massif_terrain::model::Graph;
    use std::time::Duration;

    #[test]
    fn live_tiles_become_pictures_over_their_blocks() {
        let live = Live::new(Arc::new(Engine::new(Default::default())), Settings { outputs: vec!["height".into(), "albedo".into()], ..Default::default() });
        let graph = Graph::from_json(
            r#"{ "nodes": {
            "hills": { "type": "noise.fbm", "params": { "scale": 300, "height": 80, "base": 60 } },
            "paint": { "type": "color.gradient", "inputs": { "in": "hills" } },
            "out": { "type": "output.map", "inputs": { "height": "hills", "albedo": "paint" } } } }"#,
        )
        .unwrap();
        let space = Space { x0: -512, ..Space::new(1000) };
        // The whole map in a view of 500 points, at one pixel a point.
        LiveOverlay::follow(&live, &space, [-512.0, 0.0, 488.0, 1000.0], 0.5, 1.0);
        live.set_graph(&graph, space, Arc::new(NoEnv), 1);
        assert!(live.wait_idle(Duration::from_secs(60)));
        let ctx = Context::default();
        let mut overlay = LiveOverlay::default();
        assert!(overlay.take(&ctx, live.tiles_ready(), 62.0, true));
        let pictures = overlay.pictures(&space);
        assert!(!pictures.is_empty());
        for (_, r, uv) in &pictures {
            // Within the terrain, which ends at x 488, z 1000.
            assert!(r[0] >= -512 && r[2] <= 488 && r[1] >= 0 && r[3] <= 1000, "{r:?}");
            assert!(uv.max.x <= 1.0 && uv.max.y <= 1.0);
        }
    }
}
