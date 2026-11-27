//! The Render Viewport's minimap: the whole map small in a corner, the
//! camera on it with what it sees, the Timeline's path; a click there puts
//! the camera over that place.

use massif_core::World;
use massif_core::coords::{TILE_SIZE, TilePos, local_index};
use std::collections::HashMap;

/// The map drawn small: its pixels, RGBA, `size` across, and the blocks it
/// covers (west, north, east, south edges).
pub struct Picture {
    pub size: [usize; 2],
    pub rgba: Vec<u8>,
    pub blocks: [f64; 4],
}

/// The map of `world`'s dimension `dim` at most `most` pixels on its long
/// side: each pixel the terrain's colour at a column, shaded by the slope
/// toward the north-west light, water blue by its depth, nothing where
/// there is no land.
pub fn draw(world: &World, dim: usize, most: usize) -> Option<Picture> {
    let d = world.dims.get(dim)?;
    let (lo, hi) = d.ground().bounds()?;
    let (x0, z0) = (lo.x * TILE_SIZE, lo.z * TILE_SIZE);
    let (w_blocks, h_blocks) = ((hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE);
    let scale = (w_blocks.max(h_blocks) as f64 / most as f64).max(1.0);
    let (w, h) = (((w_blocks as f64 / scale) as usize).max(1), ((h_blocks as f64 / scale) as usize).max(1));
    let reg = &world.registry;
    let mut stacks: HashMap<TilePos, Option<massif_core::compose::TileStack>> = HashMap::new();
    let mut sampler = crate::map_render::ColumnSampler::default();
    let mut heights = vec![f32::NAN; w * h];
    let mut colours = vec![[0u8; 4]; w * h];
    for py in 0..h {
        for px in 0..w {
            let bx = x0 + ((px as f64 + 0.5) * scale) as i32;
            let bz = z0 + ((py as f64 + 0.5) * scale) as i32;
            let t = TilePos { x: bx.div_euclid(TILE_SIZE), z: bz.div_euclid(TILE_SIZE) };
            let stack = stacks.entry(t).or_insert_with(|| massif_core::compose::TileStack::from_above(d, t, false));
            let Some(stack) = stack else { continue };
            let s = sampler.sample(stack, local_index(bx, bz));
            if s.height.is_nan() {
                continue;
            }
            heights[py * w + px] = s.height;
            let c = reg.terrains.get(s.terrain as usize).map_or([120, 140, 90], |t| t.color);
            colours[py * w + px] = if s.fluid != massif_core::NO_FLUID && s.fluid as f32 > s.height {
                let depth = ((s.fluid as f32 - s.height) / 24.0).clamp(0.0, 1.0);
                let deep = [24.0, 58.0, 110.0];
                let shallow = [62.0, 130.0, 180.0];
                let m = |k: usize| (shallow[k] + (deep[k] - shallow[k]) * depth) as u8;
                [m(0), m(1), m(2), 255]
            } else {
                [c[0], c[1], c[2], 255]
            };
        }
    }
    // The slopes facing the light lighter, the others darker.
    let mut rgba = Vec::with_capacity(w * h * 4);
    for py in 0..h {
        for px in 0..w {
            let k = py * w + px;
            let c = colours[k];
            let here = heights[k];
            let up = if px > 0 && py > 0 { heights[k - w - 1] } else { here };
            let shade = if here.is_nan() || up.is_nan() { 1.0 } else { (1.0 + (here - up) as f64 / scale * 0.35).clamp(0.6, 1.35) };
            rgba.extend_from_slice(&[(c[0] as f64 * shade).min(255.0) as u8, (c[1] as f64 * shade).min(255.0) as u8, (c[2] as f64 * shade).min(255.0) as u8, c[3]]);
        }
    }
    Some(Picture { size: [w, h], rgba, blocks: [x0 as f64, z0 as f64, (x0 + w_blocks) as f64, (z0 + h_blocks) as f64] })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The test map drawn small: the size asked on its long side, land and
    /// water both there, nothing outside.
    #[test]
    fn the_map_is_drawn_small() {
        let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 4, tiles_z: 2, landform: massif_core::generate::Landform::Islands, ..Default::default() });
        let p = draw(&world, 0, 128).unwrap();
        assert_eq!(p.size, [128, 64]);
        assert_eq!(p.blocks[2] - p.blocks[0], 4.0 * TILE_SIZE as f64);
        let px: Vec<&[u8]> = p.rgba.chunks(4).collect();
        assert!(px.iter().any(|c| c[2] > c[1] && c[3] == 255), "water");
        assert!(px.iter().any(|c| c[1] > c[2] && c[3] == 255), "land");
    }
}
