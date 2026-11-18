//! The scene the rays go through: for every chunk, the blocks a ray can
//! reach, in a sparse tree the graphics card walks.
//!
//! A block whose six neighbours are opaque cubes can never be seen: a ray
//! stops at one of them first. Leaving those out keeps only the surface of
//! the land (its top, its cliffs, its caves), so a map takes memory for
//! its area, not its volume: the stone under the ground is empty to the
//! rays, which never get there.
//!
//! Each section of 16³ blocks is a two-level *64-tree*: its 64 bricks of
//! 4³ blocks, and their 64 blocks. A node is a 64-bit mask of the children
//! it has and where they start; a child's place is the count of bits below
//! its own (so only the children there are take room). A brick whose
//! blocks are all one material stores that material, and nothing else.
//!
//! Words of a chunk (all `u32`):
//!
//! * `entries`: one per section from the bottom of the scene: 0, nothing
//!   to see; `UNIFORM | material`, the whole section that material;
//!   otherwise 1 + where its root node starts in `tree`.
//! * `tree`: a root node `[mask low, mask high, first brick]`, then its
//!   bricks `[mask low, mask high, data]`, 3 words each; `data` is
//!   `UNIFORM | material`, or where the brick's materials start in
//!   `voxels` (one `u16` per block it has, in the order of the mask).

use crate::chunk::{Chunk, Section, VOLUME};
use crate::materials::{AIR, Class};

/// The flag of an entry or a brick holding one material throughout.
pub const UNIFORM: u32 = 0x8000_0000;

/// A chunk ready for the graphics card.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct GpuChunk {
    pub x: i32,
    pub z: i32,
    /// One above the highest block a ray can reach, from the bottom of the
    /// scene; 0, none.
    pub top: u32,
    pub entries: Vec<u32>,
    pub tree: Vec<u32>,
    pub voxels: Vec<u16>,
    /// The blocks that give light, seen or not (a torch is not drawn yet):
    /// `x | z << 4 | y << 8` in the chunk (y from the bottom of the scene),
    /// and their material.
    pub lights: Vec<[u32; 2]>,
}

impl GpuChunk {
    /// Bytes it takes on the graphics card, its record included.
    pub fn bytes(&self) -> usize {
        (4 + self.entries.len() + self.tree.len()) * 4 + self.voxels.len().div_ceil(2) * 4
    }
}

/// Chunks side by side, `width` × `depth` of them from chunk `(x0, z0)`:
/// a tile of the map or a region of a world. Their faces toward chunks
/// outside are all kept: what lies there is not known.
pub struct Patch {
    pub x0: i32,
    pub z0: i32,
    pub width: usize,
    pub depth: usize,
    pub sections: usize,
    /// By `z * width + x`.
    pub chunks: Vec<Option<Chunk>>,
    /// What is known of the blocks just outside the patch, on its west,
    /// east, north and south sides: 1 where they are opaque, by
    /// `along * height + y` (`along`: z on the west and east, x on the
    /// north and south). Without it, a side's faces are all kept.
    pub edges: [Option<Vec<u8>>; 4],
}

impl Patch {
    pub fn new(x0: i32, z0: i32, width: usize, depth: usize, sections: usize) -> Self {
        Self { x0, z0, width, depth, sections, chunks: vec![None; width * depth], edges: [None, None, None, None] }
    }

    /// Put a chunk in its place (one outside the patch is left out).
    pub fn put(&mut self, chunk: Chunk) {
        let (x, z) = (chunk.x - self.x0, chunk.z - self.z0);
        if (0..self.width as i32).contains(&x) && (0..self.depth as i32).contains(&z) {
            self.chunks[z as usize * self.width + x as usize] = Some(chunk);
        }
    }

    fn section(&self, cx: i32, cz: i32, s: i32) -> Option<&Section> {
        if cx < 0 || cz < 0 || cx >= self.width as i32 || cz >= self.depth as i32 || s < 0 || s >= self.sections as i32 {
            return None;
        }
        self.chunks[cz as usize * self.width + cx as usize].as_ref().map(|c| &c.sections[s as usize])
    }

    /// The class of the block at `(x, y, z)` from the patch's corner and the
    /// bottom of the scene: under the scene, opaque (its floor is never
    /// seen); over it, beside the patch and in chunks missing, air.
    fn class_at(&self, classes: &[Class], x: i32, y: i32, z: i32) -> Class {
        if y < 0 {
            return Class::Opaque;
        }
        let (w, d, h) = (self.width as i32 * 16, self.depth as i32 * 16, self.sections as i32 * 16);
        if y < h && (x < 0 || z < 0 || x >= w || z >= d) {
            let (side, along) = match (x < 0, x >= w, z < 0) {
                (true, _, _) => (0, z),
                (_, true, _) => (1, z),
                (_, _, true) => (2, x),
                _ => (3, x),
            };
            let opaque = self.edges[side].as_ref().is_some_and(|e| e.get((along * h + y) as usize).is_some_and(|&o| o != 0));
            return if opaque { Class::Opaque } else { Class::Air };
        }
        match self.section(x >> 4, z >> 4, y >> 4) {
            Some(s) => class(classes, s.get((x & 15) as usize, (y & 15) as usize, (z & 15) as usize)),
            None => Class::Air,
        }
    }

    /// Build every chunk of the patch; `lights`: the light each material
    /// gives (the game's levels).
    pub fn build(&self, classes: &[Class], lights: &[u8]) -> Vec<GpuChunk> {
        // Whether each section is opaque throughout, to leave the buried
        // ones out without looking at their blocks.
        let solid: Vec<Vec<bool>> = self.chunks.iter().map(|c| c.as_ref().map_or_else(Vec::new, |c| c.sections.iter().map(|s| all_opaque(classes, s)).collect())).collect();
        let solid_at = |cx: i32, cz: i32, s: i32| -> bool {
            if s < 0 {
                return true;
            }
            if cx < 0 || cz < 0 || cx >= self.width as i32 || cz >= self.depth as i32 || s >= self.sections as i32 {
                return false;
            }
            solid[cz as usize * self.width + cx as usize].get(s as usize).copied().unwrap_or(false)
        };
        let mut out = Vec::new();
        let mut scratch = Scratch::default();
        for cz in 0..self.depth as i32 {
            for cx in 0..self.width as i32 {
                let Some(chunk) = &self.chunks[cz as usize * self.width + cx as usize] else { continue };
                let mut gpu = GpuChunk { x: chunk.x, z: chunk.z, top: 0, entries: vec![0; self.sections], tree: Vec::new(), voxels: Vec::new(), lights: Vec::new() };
                for s in 0..self.sections as i32 {
                    let section = &chunk.sections[s as usize];
                    if *section == Section::EMPTY {
                        continue;
                    }
                    let buried = solid_at(cx, cz, s) && solid_at(cx, cz, s - 1) && solid_at(cx, cz, s + 1) && solid_at(cx - 1, cz, s) && solid_at(cx + 1, cz, s) && solid_at(cx, cz - 1, s) && solid_at(cx, cz + 1, s);
                    if buried {
                        continue;
                    }
                    if let Some(high) = self.build_section(classes, lights, cx, cz, s, section, &mut gpu, &mut scratch) {
                        gpu.top = gpu.top.max((s * 16 + high + 1) as u32);
                    }
                }
                out.push(gpu);
            }
        }
        out
    }

    /// The tree of one section into its chunk; the highest block kept in
    /// it, if any.
    #[allow(clippy::too_many_arguments)]
    fn build_section(&self, classes: &[Class], lights: &[u8], cx: i32, cz: i32, s: i32, section: &Section, gpu: &mut GpuChunk, scratch: &mut Scratch) -> Option<i32> {
        // The classes of the section with a border of one block around it
        // (its faces' neighbours only: no ray goes diagonally).
        const P: usize = 18;
        let pad = &mut scratch.pad;
        let mats = &mut scratch.mats;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let m = section.get(x, y, z);
                    mats[(y * 16 + z) * 16 + x] = m;
                    pad[((y + 1) * P + z + 1) * P + x + 1] = class(classes, m) as u8;
                }
            }
        }
        let (bx, by, bz) = (cx * 16, s * 16, cz * 16);
        for a in 0..16i32 {
            for b in 0..16i32 {
                let at = |x: i32, y: i32, z: i32| self.class_at(classes, bx + x, by + y, bz + z) as u8;
                let i = |x: i32, y: i32, z: i32| (((y + 1) as usize * P + (z + 1) as usize) * P) + (x + 1) as usize;
                pad[i(-1, a, b)] = at(-1, a, b);
                pad[i(16, a, b)] = at(16, a, b);
                pad[i(a, -1, b)] = at(a, -1, b);
                pad[i(a, 16, b)] = at(a, 16, b);
                pad[i(a, b, -1)] = at(a, b, -1);
                pad[i(a, b, 16)] = at(a, b, 16);
            }
        }
        // Which blocks a ray can reach.
        let mut high = None;
        let kept = &mut scratch.kept;
        for y in 0..16 {
            for z in 0..16 {
                for x in 0..16 {
                    let p = ((y + 1) * P + z + 1) * P + x + 1;
                    let c = from_u8(pad[p]);
                    let show = c != Class::Air && [p - 1, p + 1, p - P, p + P, p - P * P, p + P * P].iter().any(|&n| c.shows_to(from_u8(pad[n])));
                    kept[(y * 16 + z) * 16 + x] = show;
                    if show {
                        high = Some(y as i32);
                    }
                    // A light seen, or one not drawn (a torch).
                    let m = mats[(y * 16 + z) * 16 + x];
                    if lights.get(m as usize).is_some_and(|&l| l > 0) && (show || c == Class::Air) {
                        gpu.lights.push([x as u32 | (z as u32) << 4 | ((s as u32) * 16 + y as u32) << 8, m as u32]);
                    }
                }
            }
        }
        high?;
        // The bricks, in the order of their bits: x + 4 z + 16 y.
        let mut root = 0u64;
        let mut bricks: Vec<[u32; 3]> = Vec::new();
        let mut all_full_of = None::<Option<u16>>;
        let start_voxels = gpu.voxels.len();
        for bi in 0..64usize {
            let (bx, bz, by) = (bi & 3, (bi >> 2) & 3, bi >> 4);
            let mut mask = 0u64;
            let mut first = None;
            let mut same = true;
            let mark = gpu.voxels.len();
            for vi in 0..64usize {
                let (x, z, y) = (bx * 4 + (vi & 3), bz * 4 + ((vi >> 2) & 3), by * 4 + (vi >> 4));
                let k = (y * 16 + z) * 16 + x;
                if kept[k] {
                    mask |= 1 << vi;
                    let m = mats[k];
                    same &= first.is_none_or(|f| f == m);
                    first.get_or_insert(m);
                    gpu.voxels.push(m);
                }
            }
            // A section is one material when every brick is full of it.
            all_full_of = match (all_full_of, mask == u64::MAX && same) {
                (None, true) => Some(first),
                (Some(Some(f)), true) if first == Some(f) => Some(Some(f)),
                _ => Some(None),
            };
            if mask == 0 {
                continue;
            }
            root |= 1 << bi;
            let data = if same {
                gpu.voxels.truncate(mark);
                UNIFORM | first.unwrap_or(AIR) as u32
            } else {
                mark as u32
            };
            bricks.push([mask as u32, (mask >> 32) as u32, data]);
        }
        if let Some(Some(m)) = all_full_of {
            gpu.voxels.truncate(start_voxels);
            gpu.entries[s as usize] = UNIFORM | m as u32;
            return high;
        }
        let at = gpu.tree.len() as u32;
        gpu.tree.extend_from_slice(&[root as u32, (root >> 32) as u32, at + 3]);
        for brick in bricks {
            gpu.tree.extend_from_slice(&brick);
        }
        gpu.entries[s as usize] = at + 1;
        high
    }
}

#[derive(Clone)]
struct Scratch {
    pad: Vec<u8>,
    mats: Vec<u16>,
    kept: Vec<bool>,
}

impl Default for Scratch {
    fn default() -> Self {
        Self { pad: vec![0; 18 * 18 * 18], mats: vec![0; VOLUME], kept: vec![false; VOLUME] }
    }
}

fn class(classes: &[Class], m: u16) -> Class {
    classes.get(m as usize).copied().unwrap_or(Class::Air)
}

fn from_u8(c: u8) -> Class {
    match c {
        1 => Class::Opaque,
        2 => Class::Cutout,
        3 => Class::Water,
        4 => Class::Lava,
        _ => Class::Air,
    }
}

fn all_opaque(classes: &[Class], s: &Section) -> bool {
    match s {
        Section::Uniform(m) => class(classes, *m) == Class::Opaque,
        Section::Mixed(b) => b.iter().all(|&m| class(classes, m) == Class::Opaque),
    }
}

/// A ray through built chunks, on the processor: what the shader does, to
/// test the scene's words against (the material and the face hit, by axis).
#[cfg(test)]
pub fn trace_cpu(chunks: &[GpuChunk], sections: usize, from: [f64; 3], dir: [f64; 3], steps: usize) -> Option<(u16, usize, [i32; 3])> {
    let find = |cx: i32, cz: i32| chunks.iter().find(|c| c.x == cx && c.z == cz);
    let mat_at = |v: [i32; 3]| -> u16 {
        let Some(c) = find(v[0] >> 4, v[2] >> 4) else { return AIR };
        if v[1] < 0 || v[1] >= sections as i32 * 16 {
            return AIR;
        }
        let e = c.entries[(v[1] >> 4) as usize];
        if e == 0 {
            return AIR;
        }
        if e & UNIFORM != 0 {
            return (e & 0xffff) as u16;
        }
        let root = (e - 1) as usize;
        let mask = c.tree[root] as u64 | (c.tree[root + 1] as u64) << 32;
        let b = [(v[0] & 15) >> 2, (v[1] & 15) >> 2, (v[2] & 15) >> 2];
        let bi = (b[0] + b[2] * 4 + b[1] * 16) as u32;
        if mask >> bi & 1 == 0 {
            return AIR;
        }
        let brick = c.tree[root + 2] as usize + (mask & ((1u64 << bi) - 1)).count_ones() as usize * 3;
        let bm = c.tree[brick] as u64 | (c.tree[brick + 1] as u64) << 32;
        let w = [v[0] & 3, v[1] & 3, v[2] & 3];
        let vi = (w[0] + w[2] * 4 + w[1] * 16) as u32;
        if bm >> vi & 1 == 0 {
            return AIR;
        }
        let data = c.tree[brick + 2];
        if data & UNIFORM != 0 { (data & 0xffff) as u16 } else { c.voxels[data as usize + (bm & ((1u64 << vi) - 1)).count_ones() as usize] }
    };
    // A plain voxel walk (Amanatides & Woo).
    let mut v = from.map(|f| f.floor() as i32);
    let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
    let mut next = [0.0; 3];
    let mut delta = [0.0; 3];
    for a in 0..3 {
        delta[a] = if dir[a] == 0.0 { f64::INFINITY } else { (1.0 / dir[a]).abs() };
        let edge = if dir[a] > 0.0 { v[a] as f64 + 1.0 } else { v[a] as f64 };
        next[a] = if dir[a] == 0.0 { f64::INFINITY } else { (edge - from[a]) / dir[a] };
    }
    for _ in 0..steps {
        let a = (0..3).min_by(|&i, &j| next[i].total_cmp(&next[j])).unwrap();
        v[a] += step[a];
        next[a] += delta[a];
        let m = mat_at(v);
        if m != AIR {
            return Some((m, a, v));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::materials::Class::*;

    /// A chunk of stone up to y 40 (scene units), grass on top, a pond of
    /// water in one corner and a tree trunk.
    fn ground(cx: i32, cz: i32) -> Chunk {
        let mut sections = vec![Section::EMPTY; 4];
        for (s, section) in sections.iter_mut().enumerate() {
            let mut b = Box::new([AIR; VOLUME]);
            for y in 0..16 {
                for z in 0..16 {
                    for x in 0..16 {
                        let gy = s * 16 + y;
                        let pond = x < 4 && z < 4 && (38..=40).contains(&gy);
                        b[(y * 16 + z) * 16 + x] = if pond {
                            3
                        } else if gy < 40 {
                            1 + ((x * 7 + z * 3 + gy) % 5 == 0) as u16 * 4
                        } else if gy == 40 {
                            2
                        } else if x == 8 && z == 8 && gy < 46 {
                            6
                        } else {
                            AIR
                        };
                    }
                }
            }
            *section = Section::Mixed(b);
        }
        Chunk { x: cx, z: cz, sections }
    }

    // 0 air, 1 stone, 2 grass, 3 water, 4 unused, 5 ore, 6 log, 7 leaves.
    const CLASSES: [Class; 8] = [Air, Opaque, Opaque, Water, Air, Opaque, Opaque, Cutout];

    #[test]
    fn only_the_surface_is_kept_and_rays_find_it() {
        let mut patch = Patch::new(0, 0, 3, 3, 4);
        for cz in 0..3 {
            for cx in 0..3 {
                patch.put(ground(cx, cz));
            }
        }
        let built = patch.build(&CLASSES, &[0; 8]);
        assert_eq!(built.len(), 9);
        let c = built.iter().find(|c| (c.x, c.z) == (1, 1)).unwrap();
        assert_eq!(c.top, 46, "the trunk's top");
        assert_eq!(&c.entries[..2], &[0, 0], "the buried sections are left out");
        // The surface only: the grass, the pond's surface and bed, the trunk.
        let blocks: usize = c.tree.chunks(3).skip(1).map(|b| (b[0].count_ones() + b[1].count_ones()) as usize).sum();
        assert!(blocks < 16 * 16 + 64, "{blocks} blocks kept");
        // Straight down onto the grass, the trunk and the pond's surface.
        let down = |x: f64, z: f64| trace_cpu(&built, 4, [x, 60.0, z], [0.0, -1.0, 0.0], 200).map(|h| (h.0, h.2[1]));
        assert_eq!(down(28.5, 28.5), Some((2, 40)));
        assert_eq!(down(24.5, 24.5), Some((6, 45)));
        assert_eq!(down(17.5, 17.5), Some((3, 40)));
        // Sideways into the trunk, across the air above the grass.
        assert_eq!(trace_cpu(&built, 4, [28.5, 43.5, 24.5], [-1.0, 0.0, 0.0], 200).map(|h| (h.0, h.2[0], h.1)), Some((6, 24, 0)));
        // From beside the patch (whose neighbours are not known) the stone
        // and its ore show.
        let side = trace_cpu(&built, 4, [-5.5, 20.5, 6.5], [1.0, 0.0, 0.0], 200).unwrap();
        assert!(matches!(side.0, 1 | 5) && side.2[0] == 0);
    }

    #[test]
    fn what_is_known_beside_the_patch_hides_its_sides() {
        let build = |known: bool| {
            let mut patch = Patch::new(0, 0, 1, 1, 4);
            patch.put(ground(0, 0));
            if known {
                // Solid up to y 40 all around, as the chunk itself.
                let edge: Vec<u8> = (0..16 * 64).map(|k| (k % 64 < 40) as u8).collect();
                patch.edges = [Some(edge.clone()), Some(edge.clone()), Some(edge.clone()), Some(edge)];
            }
            let built = patch.build(&CLASSES, &[0; 8]);
            built[0].tree.chunks(3).map(|b| (b[0].count_ones() + b[1].count_ones()) as usize).sum::<usize>()
        };
        let (blind, known) = (build(false), build(true));
        assert!(known * 4 < blind, "{known} blocks kept with the sides known, {blind} without");
    }

    #[test]
    fn a_section_seen_throughout_is_one_word() {
        // Leaves let the light through: every one of them can be seen.
        let mut patch = Patch::new(0, 0, 1, 1, 4);
        patch.put(Chunk { x: 0, z: 0, sections: vec![Section::Uniform(7), Section::EMPTY, Section::EMPTY, Section::EMPTY] });
        let built = patch.build(&CLASSES, &[0; 8]);
        assert_eq!(built[0].entries[0], UNIFORM | 7);
        assert!(built[0].tree.is_empty() && built[0].voxels.is_empty());
        assert_eq!(built[0].top, 16);
        assert_eq!(trace_cpu(&built, 4, [3.5, 70.0, 3.5], [0.0, -1.0, 0.0], 200).map(|h| h.2[1]), Some(15));
    }
}
