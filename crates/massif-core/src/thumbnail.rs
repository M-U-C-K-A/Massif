//! A small picture of a map, kept in its file so that the Finder (and the
//! home screen) can show the map instead of a blank page, as WorldPainter's
//! fork does for `.world` files: the land in its terrains' colours, lit
//! from the north-west, its water and lava, on a dark background where the
//! map has no tiles.

use crate::compose::TileStack;
use crate::coords::{TILE_SIZE, TilePos, local_index};
use crate::tile::NO_FLUID;
use crate::world::World;
use rayon::prelude::*;

/// One pixel's sample: its place in the picture, the height of the land,
/// its colour and the depth of the water over it (NaN height: no land).
type Sample = (usize, f32, [f32; 3], f32);

/// The longest side of the picture, in pixels.
pub const MAX_SIZE: u32 = 512;
const BACKGROUND: [u8; 3] = [24, 24, 27];

/// The picture of the first dimension of `world` (the surface), at most
/// `max` pixels on its longest side; `None` for an empty map.
pub fn render(world: &World, max: u32) -> Option<image::RgbImage> {
    let dim = world.dims.first()?;
    let (lo, hi) = dim.bounds()?;
    let (x0, z0) = (lo.x as i64 * TILE_SIZE as i64, lo.z as i64 * TILE_SIZE as i64);
    let (w, d) = ((hi.x - lo.x + 1) as i64 * TILE_SIZE as i64, (hi.z - lo.z + 1) as i64 * TILE_SIZE as i64);
    let step = (w.max(d) as f64 / max.max(1) as f64).max(1.0);
    let (pw, ph) = (((w as f64 / step).round() as usize).max(1), ((d as f64 / step).round() as usize).max(1));
    let column = |p: usize, origin: i64| (origin as f64 + (p as f64 + 0.5) * step).floor() as i32;
    let reg = &world.registry;
    let sea = world.meta.sea_level as f32;

    // Heights, colours and water, one sample per pixel: a row of tiles at a
    // time, so that each tile is read once.
    let rows: Vec<Vec<usize>> = {
        let n = (hi.z - lo.z + 1) as usize;
        let mut rows: Vec<Vec<usize>> = vec![Vec::new(); n];
        for py in 0..ph {
            let tz = column(py, z0).div_euclid(TILE_SIZE) - lo.z;
            rows[tz.clamp(0, n as i32 - 1) as usize].push(py);
        }
        rows
    };
    let samples: Vec<Vec<Sample>> = rows
        .par_iter()
        .map(|pys| {
            let mut out = Vec::with_capacity(pys.len() * pw);
            let mut stacks: Vec<(TilePos, Option<TileStack>)> = Vec::new();
            let mut spans = Vec::new();
            for &py in pys {
                let z = column(py, z0);
                for px in 0..pw {
                    let x = column(px, x0);
                    let pos = TilePos::containing(x, z);
                    let k = match stacks.iter().position(|(p, _)| *p == pos) {
                        Some(k) => k,
                        None => {
                            stacks.push((pos, TileStack::new(dim, pos, false)));
                            stacks.len() - 1
                        }
                    };
                    let Some(stack) = &stacks[k].1 else {
                        out.push((py * pw + px, f32::NAN, [0.0; 3], 0.0));
                        continue;
                    };
                    let cell = local_index(x, z);
                    let Some((top, terrain)) = stack.surface(cell, &mut spans) else {
                        out.push((py * pw + px, f32::NAN, [0.0; 3], 0.0));
                        continue;
                    };
                    let c = reg.terrain(terrain).color;
                    let fluid = stack.ground().fluid.get(cell);
                    let depth = if fluid != NO_FLUID { (fluid as f32 - top).max(0.0) } else { 0.0 };
                    out.push((py * pw + px, top, [c[0] as f32, c[1] as f32, c[2] as f32], depth));
                }
            }
            out
        })
        .collect();
    let mut height = vec![f32::NAN; pw * ph];
    let mut color = vec![[0.0f32; 3]; pw * ph];
    let mut water = vec![0.0f32; pw * ph];
    for row in samples {
        for (i, h, c, depth) in row {
            height[i] = h;
            color[i] = c;
            water[i] = depth;
        }
    }

    // Lit from the north-west, 45° up, a little steeper the farther apart
    // the samples are.
    let light = [-0.5f32, std::f32::consts::FRAC_1_SQRT_2, -0.5];
    let exaggeration = 1.0 + 0.3 * (step as f32).log2().max(0.0);
    let mut img = image::RgbImage::new(pw as u32, ph as u32);
    for py in 0..ph {
        for px in 0..pw {
            let i = py * pw + px;
            let h = height[i];
            if h.is_nan() {
                img.put_pixel(px as u32, py as u32, image::Rgb(BACKGROUND));
                continue;
            }
            let at = |x: usize, y: usize| {
                let v = height[y * pw + x];
                if v.is_nan() { h } else { v }
            };
            let dx = (at((px + 1).min(pw - 1), py) - at(px.saturating_sub(1), py)) / (2.0 * step as f32) * exaggeration;
            let dz = (at(px, (py + 1).min(ph - 1)) - at(px, py.saturating_sub(1))) / (2.0 * step as f32) * exaggeration;
            let dot = (-dx * light[0] + light[1] - dz * light[2]) / (dx * dx + 1.0 + dz * dz).sqrt();
            let shade = (0.3 + 0.7 * dot / light[1]).clamp(0.3, 1.32);
            let mut c = color[i].map(|v| v * shade);
            let depth = water[i];
            if depth > 0.0 {
                let (target, k) = if world.meta.lava {
                    ([238.0, 112.0, 22.0], 0.9)
                } else {
                    ([48.0, 96.0, 200.0], 0.5 + 0.42 * (depth / 18.0).min(1.0))
                };
                c = [0, 1, 2].map(|k2| c[k2] + (target[k2] - c[k2]) * k);
            } else if h < sea - 0.5 && !world.meta.lava {
                // Dry ground below the sea: a little darker.
                c = c.map(|v| v * 0.92);
            }
            img.put_pixel(px as u32, py as u32, image::Rgb(c.map(|v| v.round().clamp(0.0, 255.0) as u8)));
        }
    }
    Some(img)
}

/// The picture of `world` as a JPEG image, as files keep it; `None` for an
/// empty map.
pub fn jpeg(world: &World) -> Option<Vec<u8>> {
    let img = render(world, MAX_SIZE)?;
    let mut out = Vec::new();
    image::codecs::jpeg::JpegEncoder::new_with_quality(&mut out, 85).encode_image(&img).ok()?;
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::generate::{GenParams, Landform, generate};

    #[test]
    fn a_map_has_its_picture() {
        let world = generate(&GenParams {
            tiles_x: 6,
            tiles_z: 3,
            landform: Landform::Islands,
            seed: 3,
            ..GenParams::default()
        });
        let img = render(&world, 256).unwrap();
        // As wide as the map is wide, against its depth.
        assert_eq!((img.width(), img.height()), (256, 128));
        // Water and land: blue and green both show.
        let blue = img.pixels().filter(|p| p[2] as i32 > p[0] as i32 + 40 && p[2] > p[1]).count();
        let green = img.pixels().filter(|p| p[1] > p[0] && p[1] > p[2]).count();
        assert!(blue > 1000 && green > 1000, "blue {blue}, green {green}");
        let jpeg = jpeg(&world).unwrap();
        assert!(jpeg.starts_with(&[0xff, 0xd8]) && jpeg.len() < 200_000, "{} bytes", jpeg.len());
        // An empty map has none.
        assert!(render(&World::new(Default::default()), 256).is_none());
    }
}
