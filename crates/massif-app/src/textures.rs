//! The game's own look for the player's view in 3D: the textures of
//! Minecraft's blocks, read from the player's installation of the game (see
//! `massif_assets`), gathered in one array the shaders sample, with the
//! game's clouds.
//!
//! Every face the 3D view draws says what it is made of: a block of the
//! registry, or a block of an object's palette (its *material*, see
//! `view3d::object_bases`). The atlas gives each material the layers of its
//! top, its sides and its bottom; layer 0 is plain white, for what has no
//! texture: such a face keeps its colour.

use massif_assets::{Catalogue, Texture, Tint};
use massif_core::registry::Registry;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};

/// Pixels on a side of a layer: the game's blocks are 16 × 16.
pub const SIDE: usize = 16;
/// Levels of detail of a layer: 16, 8, 4, 2 and 1 pixels across.
pub const MIPS: u32 = 5;
/// Pixels on a side of the clouds' map: one for 12 × 12 blocks, repeated
/// every 3,072 blocks.
pub const CLOUDS: usize = 256;

/// One layer: 16 × 16 pixels, RGBA, in sRGB.
pub type Layer = [u8; SIDE * SIDE * 4];

/// The textures of the blocks a world's faces are made of.
pub struct Atlas {
    /// Layer 0 is plain white: a material without texture keeps its colour.
    pub layers: Vec<Layer>,
    /// For every material: the layers of its top, its sides and its
    /// bottom, and the axis a log lies along (0: y, 1: x, 2: z).
    pub materials: Vec<[u16; 4]>,
    /// The layers of still water and of lava.
    pub water: u16,
    pub lava: u16,
}

impl Atlas {
    /// No textures: every face keeps its colour.
    pub fn plain() -> Self {
        Self {
            layers: vec![[255; SIDE * SIDE * 4]],
            materials: Vec::new(),
            water: 0,
            lava: 0,
        }
    }
}

/// What the faces of `reg` are made of, with the game's textures from
/// `cat`; at most `most` layers (the graphics card's limit), the materials
/// past them keeping their colour.
pub fn atlas(reg: &Registry, cat: &Catalogue, most: usize) -> Atlas {
    let mut out = Atlas::plain();
    let mut index: HashMap<(String, [u8; 3]), u16> = HashMap::new();
    let mut layer = |out: &mut Atlas, key: &str, tint: Tint, kind: Kind| -> u16 {
        let tint = tint.color();
        if let Some(&k) = index.get(&(key.to_owned(), tint)) {
            return k;
        }
        let k = match cat.texture(key) {
            Some(tex) if out.layers.len() < most.min(u16::MAX as usize) => {
                out.layers.push(make_layer(tex, tint, kind));
                (out.layers.len() - 1) as u16
            }
            _ => 0,
        };
        index.insert((key.to_owned(), tint), k);
        k
    };
    let states = reg.blocks.iter().map(|b| b.state.as_str()).chain(reg.objects.iter().flat_map(|o| o.palette.iter().map(String::as_str)));
    let mut materials = Vec::with_capacity(reg.blocks.len());
    for state in states {
        let name = state.split('[').next().unwrap_or(state);
        let mut faces = [0u16; 4];
        if let Some(block) = cat.block(name) {
            let kind = if name.contains("leaves") { Kind::Leaves } else { Kind::Solid };
            let water = name == "minecraft:water";
            let lava = name == "minecraft:lava";
            let mut get = |key: &Option<String>, tint: Tint| key.as_ref().map_or(0, |k| layer(&mut out, k, tint, if water || lava { Kind::Fluid } else { kind }));
            let top = get(&block.top, block.top_tint);
            let side = if water || lava { top } else { get(&block.side, block.side_tint) };
            let bottom = if water || lava { top } else { get(&block.bottom, Tint::None) };
            let axis = match state.split_once("axis=").map(|(_, rest)| rest.as_bytes().first().copied()) {
                Some(Some(b'x')) => 1,
                Some(Some(b'z')) => 2,
                _ => 0,
            };
            faces = [top, side, bottom, axis];
            // A face the model leaves out shows the top's texture.
            if faces[1] == 0 {
                faces[1] = top;
            }
            if faces[2] == 0 {
                faces[2] = faces[1];
            }
        }
        materials.push(faces);
    }
    out.materials = materials;
    out.water = layer(&mut out, "block/water_still", Tint::Water, Kind::Fluid);
    out.lava = layer(&mut out, "block/lava_still", Tint::None, Kind::Fluid);
    out
}

/// How a texture's clear pixels are drawn.
#[derive(Clone, Copy, PartialEq)]
enum Kind {
    /// A whole block: what shows through is the block's own colour,
    /// darkened (glass, a few ores of mods).
    Solid,
    /// Leaves drawn whole, as the game's fast graphics do: dark between the
    /// leaves.
    Leaves,
    /// Water and lava keep their transparency.
    Fluid,
}

/// A texture as a layer: 16 × 16 (others are resampled), tinted, its clear
/// pixels filled but for fluids.
fn make_layer(tex: &Texture, tint: [u8; 3], kind: Kind) -> Layer {
    let (w, h) = (tex.width.max(1) as usize, tex.height.max(1) as usize);
    let mut out = [0u8; SIDE * SIDE * 4];
    // The mean of the visible pixels, tinted: what clear pixels become.
    let (mut sum, mut n) = ([0u32; 3], 0u32);
    for p in tex.rgba.as_chunks::<4>().0 {
        if p[3] > 16 {
            for k in 0..3 {
                sum[k] += p[k] as u32;
            }
            n += 1;
        }
    }
    let mean: [u32; 3] = std::array::from_fn(|k| sum[k] / n.max(1) * tint[k] as u32 / 255);
    let under = match kind {
        Kind::Leaves => mean.map(|v| v * 2 / 5),
        _ => mean.map(|v| v * 3 / 5),
    };
    for y in 0..SIDE {
        for x in 0..SIDE {
            let (sx, sy) = (x * w / SIDE, y * h / SIDE);
            let i = (sy * w + sx) * 4;
            let p = tex.rgba.get(i..i + 4).unwrap_or(&[255, 0, 255, 255]);
            let o = (y * SIDE + x) * 4;
            let a = p[3] as u32;
            for k in 0..3 {
                let c = p[k] as u32 * tint[k] as u32 / 255;
                out[o + k] = match kind {
                    Kind::Fluid => c,
                    // Over what shows through, as much as the pixel covers.
                    _ => ((c * a + under[k] * (255 - a)) / 255) as u8 as u32,
                } as u8;
            }
            out[o + 3] = if kind == Kind::Fluid { p[3] } else { 255 };
        }
    }
    out
}

/// The levels of detail of a layer, from 16 pixels across down to 1: each
/// pixel the mean of four, in linear light.
pub fn mip_chain(layer: &Layer) -> Vec<Vec<u8>> {
    let to_lin = |c: u8| (c as f32 / 255.0).powf(2.2);
    let to_srgb = |c: f32| (c.max(0.0).powf(1.0 / 2.2) * 255.0).round().clamp(0.0, 255.0) as u8;
    let mut levels = vec![layer.to_vec()];
    let mut side = SIDE;
    while side > 1 {
        let prev = levels.last().expect("one level at least");
        let half = side / 2;
        let mut next = vec![0u8; half * half * 4];
        for y in 0..half {
            for x in 0..half {
                let px = |dx: usize, dy: usize| ((2 * y + dy) * side + 2 * x + dx) * 4;
                let quad = [px(0, 0), px(1, 0), px(0, 1), px(1, 1)];
                let o = (y * half + x) * 4;
                for k in 0..3 {
                    next[o + k] = to_srgb(quad.iter().map(|&i| to_lin(prev[i + k])).sum::<f32>() / 4.0);
                }
                next[o + 3] = (quad.iter().map(|&i| prev[i + 3] as u32).sum::<u32>() / 4) as u8;
            }
        }
        levels.push(next);
        side = half;
    }
    levels
}

/// The clouds' map, a byte a pixel (255: a cloud): the game's own, or one
/// made alike where the game is not installed.
pub fn clouds(cat: Option<&Catalogue>) -> Vec<u8> {
    if let Some(t) = cat.and_then(|c| c.texture("environment/clouds"))
        && t.width as usize == CLOUDS
        && t.height as usize == CLOUDS
    {
        return t.rgba.as_chunks::<4>().0.iter().map(|p| if p[3] > 127 { 255 } else { 0 }).collect();
    }
    // Blobs of a value noise that wraps around, as broken up as the game's.
    let lattice = |cells: usize, seed: u32| -> Vec<f32> {
        (0..cells * cells)
            .map(|i| {
                let mut h = (i as u32).wrapping_mul(0x9e37_79b9) ^ seed;
                h = (h ^ (h >> 15)).wrapping_mul(0x2c1b_3c6d);
                h ^= h >> 12;
                (h & 0xffff) as f32 / 65535.0
            })
            .collect()
    };
    let octave = |values: &[f32], cells: usize, x: usize, y: usize| {
        let s = CLOUDS as f32 / cells as f32;
        let (fx, fy) = (x as f32 / s, y as f32 / s);
        let (x0, y0) = (fx.floor() as usize % cells, fy.floor() as usize % cells);
        let (x1, y1) = ((x0 + 1) % cells, (y0 + 1) % cells);
        let smooth = |t: f32| t * t * (3.0 - 2.0 * t);
        let (tx, ty) = (smooth(fx.fract()), smooth(fy.fract()));
        let at = |i: usize, j: usize| values[j * cells + i];
        let top = at(x0, y0) + (at(x1, y0) - at(x0, y0)) * tx;
        let bottom = at(x0, y1) + (at(x1, y1) - at(x0, y1)) * tx;
        top + (bottom - top) * ty
    };
    let (coarse, fine) = (lattice(16, 0x51ed_270b), lattice(64, 0x2f2a_9d13));
    (0..CLOUDS * CLOUDS)
        .map(|i| {
            let (x, y) = (i % CLOUDS, i / CLOUDS);
            let v = 0.75 * octave(&coarse, 16, x, y) + 0.25 * octave(&fine, 64, x, y);
            if v > 0.58 { 255 } else { 0 }
        })
        .collect()
}

/// What the faces of a registry may be made of: its blocks and its
/// objects' palettes. When it changes, the atlas is made again (and the
/// numbering of objects' materials may have moved).
pub fn materials_key(reg: &Registry) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for b in &reg.blocks {
        b.state.hash(&mut h);
    }
    for o in &reg.objects {
        o.palette.hash(&mut h);
    }
    h.finish()
}

/// The shaders' side: the atlas, the materials' layers and the clouds, and
/// how the game lights, fogs and marks what it draws. Shared by the tracer
/// and the meshes, after `GLOBALS_WGSL`.
pub const LOOK_WGSL: &str = r#"
@group(2) @binding(0) var atlas: texture_2d_array<f32>;
@group(2) @binding(1) var atlas_s: sampler;
// The top, side and bottom layers of every material, and its axis.
@group(2) @binding(2) var mats: texture_2d<u32>;
// The clouds' map: one texel for 12 × 12 blocks.
@group(2) @binding(3) var cloud_map: texture_2d<f32>;

// The rock under the soil: stone, deepslate under y 0.
const ROCK: u32 = 65535u;

/// The layer of the texture on a face of material `m0` turned `n`, at
/// height `y`.
fn face_layer(m0: u32, n: vec3<f32>, y: f32) -> u32 {
    var m = m0;
    if (m == ROCK) {
        m = u32(select(g.rock.x, g.rock.y, y < 0.0));
    }
    if (g.look.x < 0.5 || m == 0u || f32(m) >= g.look.w) {
        return 0u;
    }
    let l = textureLoad(mats, vec2<i32>(i32(m & 1023u), i32(m >> 10u)), 0);
    // A log lying along x or z shows its rings at its ends.
    var end = abs(n.y) > 0.5;
    if (l.w == 1u) {
        end = abs(n.x) > 0.5;
    } else if (l.w == 2u) {
        end = abs(n.z) > 0.5;
    }
    if (!end) {
        return l.y;
    }
    if (l.w == 0u && n.y < -0.5) {
        return l.z;
    }
    return l.x;
}

/// Where on its face a point is, as the game lays its textures: tops with
/// x across and z down, sides upright.
fn face_uv(p: vec3<f32>, n: vec3<f32>) -> vec2<f32> {
    if (abs(n.y) > 0.5) {
        return fract(p.xz);
    }
    if (abs(n.x) > 0.5) {
        return vec2<f32>(fract(-p.z * n.x), 1.0 - fract(p.y));
    }
    return vec2<f32>(fract(p.x * n.z), 1.0 - fract(p.y));
}

/// The level of detail of a texture seen `t` blocks away on a face at
/// `cosv` to the eye: a texel a pixel at most.
fn tex_lod(t: f32, cosv: f32) -> f32 {
    let texels = 16.0 * t * g.cam_k.z / max(cosv, 0.35);
    return clamp(log2(max(texels, 1.0e-4)), 0.0, 4.0);
}

fn texel(layer: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    return textureSampleLevel(atlas, atlas_s, uv, i32(layer), lod);
}

/// The game's shade of a face, in its gamma: tops full, the sides along z
/// 0.8, along x 0.6, undersides 0.5; between them on a smooth slope.
fn game_face(n: vec3<f32>) -> f32 {
    let up = max(n.y, 0.0);
    let down = max(-n.y, 0.0);
    let flat = 1.0 - up - down;
    let side = mix(0.6, 0.8, abs(n.z) / max(abs(n.x) + abs(n.z), 1.0e-4));
    return up + 0.5 * down + side * flat;
}

/// Far away, the game's fog.
fn game_fog(c: vec3<f32>, d: f32) -> vec3<f32> {
    let f = clamp((d - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
    return mix(c, to_linear(g.mist.rgb), f * g.mist.w);
}

/// A face of colour `c` (linear) lit as the game lights it at noon: its
/// shade, the shadow of the corners `ao` (1: none), then the fog.
fn game_light(c: vec3<f32>, n: vec3<f32>, ao: f32, d: f32) -> vec3<f32> {
    return game_fog(c * pow(game_face(n) * ao, 2.2), d);
}

/// The game's daytime sky: its blue overhead, the fog's paler blue at the
/// horizon and below; the square sun in its glow.
fn game_sky(d: vec3<f32>) -> vec3<f32> {
    var c = mix(to_linear(g.horizon.rgb), to_linear(g.sky.rgb), smoothstep(0.0, 0.42, d.y));
    let s = g.sun.xyz;
    let k = dot(d, s);
    if (k > 0.0) {
        let a = normalize(cross(s, vec3<f32>(0.0, 1.0, 0.0)));
        let b = cross(a, s);
        let q = vec2<f32>(dot(d, a), dot(d, b)) / k;
        let m = max(abs(q.x), abs(q.y));
        let glow = 1.0 - smoothstep(0.0, 0.17, m);
        c += vec3<f32>(1.0, 0.92, 0.7) * glow * glow * 0.55;
        if (m < 0.075) {
            c = vec3<f32>(1.0, 1.0, 0.9);
        }
    }
    return c;
}

fn cloud_at(c: vec2<i32>) -> bool {
    let q = ((c % vec2<i32>(256)) + vec2<i32>(256)) % vec2<i32>(256);
    return textureLoad(cloud_map, q, 0).r > 0.5;
}

/// The game's clouds along the ray `o + t d` (from the frame's origin) up to
/// `t_max`: a layer 4 blocks thick at height `g.look.z`, in cells of 12 × 12
/// blocks; their colour and how much they hide (0: none).
fn game_clouds(o: vec3<f32>, d: vec3<f32>, t_max: f32) -> vec4<f32> {
    let h = g.look.z;
    if (h <= 0.0) {
        return vec4<f32>(0.0);
    }
    var t0 = 0.0;
    var t1 = t_max;
    if (abs(d.y) < 1.0e-5) {
        if (o.y < h || o.y > h + 4.0) {
            return vec4<f32>(0.0);
        }
    } else {
        let ta = (h - o.y) / d.y;
        let tb = (h + 4.0 - o.y) / d.y;
        t0 = max(min(ta, tb), 0.0);
        t1 = min(max(ta, tb), t_max);
    }
    if (t1 <= t0) {
        return vec4<f32>(0.0);
    }
    // In cells, in the world's own coordinates.
    let w0 = vec2<f32>(o.x + g.eye.w, o.z + g.cam_k.w) / 12.0;
    var dd = d.xz / 12.0;
    dd = select(dd, vec2<f32>(1.0e-7), abs(dd) < vec2<f32>(1.0e-7));
    let inv = 1.0 / dd;
    let step = vec2<i32>(select(-1, 1, dd.x > 0.0), select(-1, 1, dd.y > 0.0));
    var cell = vec2<i32>(floor(w0 + dd * t0));
    var t = t0;
    // In through the bottom or the top of the layer, or a side.
    var n = vec3<f32>(0.0, select(1.0, -1.0, d.y > 0.0), 0.0);
    for (var i = 0; i < 48; i++) {
        if (cloud_at(cell)) {
            var shade = 1.0;
            if (n.y < -0.5) {
                shade = 0.7;
            } else if (abs(n.x) > 0.5) {
                shade = 0.9;
            } else if (abs(n.z) > 0.5) {
                shade = 0.8;
            }
            let fade = 1.0 - clamp((t - g.fog.x) / max(g.fog.y - g.fog.x, 1.0), 0.0, 1.0);
            return vec4<f32>(vec3<f32>(pow(shade, 2.2)), 0.8 * fade);
        }
        let next = (vec2<f32>(cell + max(step, vec2<i32>(0))) - w0) * inv;
        if (next.x < next.y) {
            t = next.x;
            cell.x += step.x;
            n = vec3<f32>(-f32(step.x), 0.0, 0.0);
        } else {
            t = next.y;
            cell.y += step.y;
            n = vec3<f32>(0.0, 0.0, -f32(step.y));
        }
        if (t >= t1) {
            break;
        }
    }
    return vec4<f32>(0.0);
}

/// Whether the middle of a block `p` (from the frame's origin) is under the
/// brush: a ball, or a box for a square brush.
fn in_brush(p: vec3<f32>) -> bool {
    let v = p - g.brush.xyz;
    let r = g.brush.w;
    if (g.brush_n.w > 1.5) {
        let n = g.brush_n.xyz;
        let a = g.brush_t.xyz;
        let b = cross(n, a);
        return abs(dot(v, a)) <= r && abs(dot(v, b)) <= r && abs(dot(v, n)) <= r;
    }
    return dot(v, v) <= r * r;
}

/// What the player's view marks on a face: the blocks the brush takes, in
/// Axiom's lavender (brighter along the edge of what it takes), and the
/// outline of the block looked at, as the game draws it. Both are laid over
/// the face as the game lays its overlays: in its gamma.
fn game_marks(c: vec3<f32>, world: vec3<f32>, n: vec3<f32>, t: f32) -> vec3<f32> {
    let b = floor(world - n * 0.01);
    var a1 = vec3<f32>(1.0, 0.0, 0.0);
    var a2 = vec3<f32>(0.0, 0.0, 1.0);
    if (abs(n.x) > 0.5) {
        a1 = vec3<f32>(0.0, 0.0, 1.0);
        a2 = vec3<f32>(0.0, 1.0, 0.0);
    } else if (abs(n.z) > 0.5) {
        a2 = vec3<f32>(0.0, 1.0, 0.0);
    }
    let q = vec2<f32>(fract(dot(world, a1)), fract(dot(world, a2)));
    // A pixel, in blocks, there.
    let px = max(t * g.cam_k.z, 0.002);
    var out = pow(c, vec3<f32>(1.0 / 2.2));
    if (g.brush.w > 0.0 && in_brush(b + 0.5)) {
        let m = b + 0.5;
        let w = 2.0 * px;
        let edge = (q.x < w && !in_brush(m - a1)) || (q.x > 1.0 - w && !in_brush(m + a1)) || (q.y < w && !in_brush(m - a2)) || (q.y > 1.0 - w && !in_brush(m + a2));
        out = mix(out, vec3<f32>(0.74, 0.7, 1.0), select(0.2, 0.85, edge));
    }
    if (g.pick.w > 0.5 && all(b == g.pick.xyz)) {
        let e = min(min(q.x, 1.0 - q.x), min(q.y, 1.0 - q.y));
        let k = 1.0 - smoothstep(px * 1.1, px * 2.1, e);
        out *= 1.0 - 0.65 * k;
    }
    return pow(out, vec3<f32>(2.2));
}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mips_average_in_linear_light() {
        let mut layer = [0u8; SIDE * SIDE * 4];
        // Half white, half black, opaque.
        for (i, p) in layer.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let v = if i % 2 == 0 { 255 } else { 0 };
            *p = [v, v, v, 255];
        }
        let chain = mip_chain(&layer);
        assert_eq!(chain.len(), MIPS as usize);
        assert_eq!(chain.iter().map(Vec::len).collect::<Vec<_>>(), [1024, 256, 64, 16, 4]);
        // Half the light: 186 in sRGB, not 128.
        let one = &chain[4];
        assert!((one[0] as i32 - 186).abs() <= 1, "{one:?}");
        assert_eq!(one[3], 255);
    }

    #[test]
    fn clouds_are_patches() {
        let map = clouds(None);
        assert_eq!(map.len(), CLOUDS * CLOUDS);
        let share = map.iter().filter(|&&v| v > 0).count() as f32 / map.len() as f32;
        assert!((0.15..0.5).contains(&share), "{share}");
    }
}
