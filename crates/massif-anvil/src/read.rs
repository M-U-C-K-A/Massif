//! The surface of chunks Minecraft generated, read back from its region
//! files: the height of the ground and of the water over it, and the
//! biome there, column by column. What the land around a map is in the
//! game (see `massif_core::expand`), for the seed a server made it with.

use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io;
use std::path::Path;

/// The surface of one chunk: 16 × 16 columns, `z * 16 + x`.
#[derive(Debug, Clone)]
pub struct ChunkSurface {
    pub x: i32,
    pub z: i32,
    /// Surface elevation of the ground: one stands at this height (the
    /// highest block is one under it). Trees and water left out.
    pub ground: [i16; 256],
    /// The surface of the water or lava over the ground, where there is.
    pub fluid: [i16; 256],
    /// The biome of the column at its surface, as an index into
    /// [`ChunkSurface::biomes`].
    pub biome: [u8; 256],
    /// The names of the biomes, without `minecraft:`.
    pub biomes: Vec<String>,
}

#[derive(Deserialize)]
struct Chunk {
    #[serde(rename = "xPos")]
    x: i32,
    #[serde(rename = "zPos")]
    z: i32,
    #[serde(rename = "Status")]
    status: String,
    #[serde(rename = "yPos", default)]
    y_pos: Option<i32>,
    #[serde(rename = "Heightmaps")]
    heightmaps: Heightmaps,
    sections: Vec<Section>,
}

#[derive(Deserialize)]
#[allow(non_snake_case)]
struct Heightmaps {
    OCEAN_FLOOR: Option<fastnbt::LongArray>,
    MOTION_BLOCKING_NO_LEAVES: Option<fastnbt::LongArray>,
}

#[derive(Deserialize)]
struct Section {
    #[serde(rename = "Y")]
    y: i8,
    biomes: Option<Biomes>,
}

#[derive(Deserialize)]
struct Biomes {
    palette: Vec<String>,
    data: Option<fastnbt::LongArray>,
}

/// The 256 values of a heightmap: 9 bits each, never across two longs.
fn heightmap(longs: &[i64], bits: usize) -> [i16; 256] {
    let per = 64 / bits;
    let mask = (1u64 << bits) - 1;
    std::array::from_fn(|i| longs.get(i / per).map_or(0, |l| ((*l as u64 >> ((i % per) * bits)) & mask) as i16))
}

/// The biome of a section at quarter `(qx, qy, qz)`, as an index into its
/// palette.
fn section_biome(b: &Biomes, qx: usize, qy: usize, qz: usize) -> usize {
    let Some(data) = &b.data else { return 0 };
    let bits = (usize::BITS - (b.palette.len() - 1).leading_zeros()).max(1) as usize;
    let per = 64 / bits;
    let i = (qy * 4 + qz) * 4 + qx;
    data.get(i / per).map_or(0, |l| ((*l as u64 >> ((i % per) * bits)) & ((1u64 << bits) - 1)) as usize)
}

/// The surface of a chunk's NBT, if it is a whole chunk (fully generated).
pub fn chunk_surface(nbt: &[u8], min_y: i32) -> Option<ChunkSurface> {
    let chunk: Chunk = fastnbt::from_bytes(nbt).ok()?;
    if !chunk.status.ends_with("full") {
        return None;
    }
    let min_y = chunk.y_pos.map_or(min_y, |y| y * 16);
    let ground = heightmap(chunk.heightmaps.OCEAN_FLOOR.as_deref()?, 9).map(|v| (v as i32 + min_y) as i16);
    let top = heightmap(chunk.heightmaps.MOTION_BLOCKING_NO_LEAVES.as_deref()?, 9).map(|v| (v as i32 + min_y) as i16);
    let fluid: [i16; 256] = std::array::from_fn(|i| if top[i] > ground[i] { top[i] } else { massif_core::NO_FLUID });
    let mut names: Vec<String> = Vec::new();
    let mut index: HashMap<(i8, usize), u8> = HashMap::new();
    let sections: HashMap<i8, &Section> = chunk.sections.iter().map(|s| (s.y, s)).collect();
    let biome: [u8; 256] = std::array::from_fn(|i| {
        let (x, z) = (i % 16, i / 16);
        // The biome of the block one stands in, at the ground's surface.
        let y = ground[i] as i32;
        let Some(section) = sections.get(&(y.div_euclid(16) as i8)).or_else(|| sections.get(&((y - 1).div_euclid(16) as i8))) else { return 0 };
        let Some(b) = &section.biomes else { return 0 };
        let k = section_biome(b, x / 4, (y.rem_euclid(16) / 4) as usize, z / 4);
        *index.entry((section.y, k)).or_insert_with(|| {
            let name = b.palette.get(k).map_or("plains", |n| n.strip_prefix("minecraft:").unwrap_or(n)).to_owned();
            names.iter().position(|n| *n == name).unwrap_or_else(|| {
                names.push(name);
                names.len() - 1
            }) as u8
        })
    });
    Some(ChunkSurface { x: chunk.x, z: chunk.z, ground, fluid, biome, biomes: names })
}

/// The surfaces of every whole chunk of a region file.
pub fn region_surfaces(path: &Path, min_y: i32) -> io::Result<Vec<ChunkSurface>> {
    let mut region = fastanvil::Region::from_stream(File::open(path)?).map_err(|e| io::Error::other(format!("{e:?}")))?;
    let mut out = Vec::new();
    for z in 0..32 {
        for x in 0..32 {
            if let Ok(Some(nbt)) = region.read_chunk(x, z)
                && let Some(s) = chunk_surface(&nbt, min_y)
            {
                out.push(s);
            }
        }
    }
    Ok(out)
}

/// The surfaces of every whole chunk of a world's overworld.
pub fn world_surfaces(world: &Path) -> io::Result<Vec<ChunkSurface>> {
    let dir = [world.join("region"), world.join("dimensions/minecraft/overworld/region")].into_iter().find(|d| d.is_dir()).ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no region folder"))?;
    let mut out = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.extension().is_some_and(|e| e == "mca") {
            out.extend(region_surfaces(&path, -64)?);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rough land `massif_core::vanilla` works out against the land a
    /// server made for that seed: `MASSIF_VANILLA_WORLD` (a world folder),
    /// `MASSIF_VANILLA_SEED`.
    #[test]
    #[ignore]
    fn vanilla_against_the_game() {
        let (Some(world), Some(seed)) = (std::env::var_os("MASSIF_VANILLA_WORLD"), std::env::var("MASSIF_VANILLA_SEED").ok()) else { return };
        let seed: i64 = seed.parse().unwrap();
        let v = massif_core::vanilla::Vanilla::new(seed);
        let surfaces = world_surfaces(Path::new(&world)).unwrap();
        let (mut same, mut columns, mut err, mut land_err, mut land) = (0usize, 0usize, Vec::new(), 0f64, 0usize);
        let mut confusions: HashMap<(String, &str), usize> = HashMap::new();
        for s in &surfaces {
            for qz in 0..4 {
                for qx in 0..4 {
                    let i = (qz * 4) * 16 + qx * 4;
                    let (x, z) = (s.x * 16 + qx as i32 * 4, s.z * 16 + qz as i32 * 4);
                    let c = v.climate(x, z);
                    let ours = massif_core::vanilla::biome_of(&c);
                    let theirs = &s.biomes[s.biome[i] as usize];
                    columns += 1;
                    if ours.name() == theirs {
                        same += 1;
                    } else {
                        *confusions.entry((theirs.clone(), ours.name())).or_default() += 1;
                    }
                    let d = (c.surface() - s.ground[i] as f32) as f64;
                    err.push(d.abs());
                    if s.fluid[i] == massif_core::NO_FLUID {
                        land_err += d.abs();
                        land += 1;
                    }
                }
            }
        }
        err.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut c: Vec<_> = confusions.into_iter().collect();
        c.sort_by_key(|e| std::cmp::Reverse(e.1));
        eprintln!(
            "{} chunks: biomes {:.1} % the same over {columns} quarters; height error median {:.1}, 90 % {:.1}, mean on land {:.1}",
            surfaces.len(),
            100.0 * same as f64 / columns as f64,
            err[err.len() / 2],
            err[err.len() * 9 / 10],
            land_err / land.max(1) as f64
        );
        for ((a, b), n) in c.iter().take(12) {
            eprintln!("  game {a} / ours {b}: {n}");
        }
    }
}
