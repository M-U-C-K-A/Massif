// The Diorama: the open map from straight above, block by block (see
// diorama.rs). A pixel finds the block under it, the floor seen there (the
// surface, or the caves under a height) from the floors of the tiles in
// view, draws its top with the game's texture and what stands on it, the
// water over it, and lights it by the relief: the slope, the edges of the
// steps around it and the shadows the higher blocks to the north-west
// cast. From afar, or where the floors are not on the card, it reads the
// tile's small image.

struct Params {
    table: vec4<i32>,
    background: vec4<f32>,
    center: vec2<f32>,
    size: vec2<u32>,
    origin: vec2<i32>,
    blocks_per_px: f32,
    turn: u32,
    slice: i32,
    caves: u32,
    min_y: i32,
    near_side: u32,
    far_size: u32,
    far_side: u32,
    water: u32,
    lava: u32,
    relief: f32,
    use_near: u32,
    pad0: u32,
    pad1: u32,
};

@group(0) @binding(0) var<uniform> p: Params;
@group(0) @binding(1) var table: texture_2d<u32>;
@group(0) @binding(2) var near: texture_2d<u32>;
@group(0) @binding(3) var far: texture_2d<f32>;
@group(0) @binding(4) var<storage, read> materials: array<vec4<u32>>;
@group(0) @binding(5) var atlas: texture_2d_array<f32>;
@group(0) @binding(6) var blocks: sampler;
@group(0) @binding(7) var soft: sampler;
@group(0) @binding(8) var image: texture_storage_2d<rgba8unorm, write>;

const NO_FLOOR: u32 = 65535u;
// What the colours are lifted by: the game's daylight, not a dim room.
const EXPOSURE: f32 = 1.25;
const ROCK: vec3<f32> = vec3<f32>(0.0091, 0.0080, 0.0103);

struct Column {
    // The floors of its tile are on the card.
    known: bool,
    // It has a floor where it is looked at.
    has: bool,
    block: u32,
    y: i32,
    over: u32,
    depth: u32,
    lava: bool,
};

fn entry(tile: vec2<i32>) -> u32 {
    let t = tile - p.table.xy;
    if t.x < 0 || t.y < 0 || t.x >= p.table.z || t.y >= p.table.w {
        return 0u;
    }
    return textureLoad(table, t, 0).x;
}

// Block `b` (relative to the origin tile's corner).
fn column(b: vec2<i32>) -> Column {
    var c: Column;
    let tile = b >> vec2<u32>(7u);
    let e = entry(p.origin + tile);
    let slot = e & 0xffffu;
    if slot == 0u {
        return c;
    }
    c.known = true;
    let s = slot - 1u;
    let local = b & vec2<i32>(127);
    let at = vec2<i32>(i32(s % p.near_side) * 128, i32(s / p.near_side) * 128) + local;
    let v = textureLoad(near, at, 0);
    if v.y == NO_FLOOR {
        return c;
    }
    c.has = true;
    c.block = v.x;
    c.y = i32(v.y) + p.min_y;
    c.over = v.z;
    c.depth = v.w & 0xffu;
    c.lava = (v.w >> 8u) != 0u;
    return c;
}

// The top of what is seen of a column: its block, or the water over it.
fn top(c: Column) -> f32 {
    return f32(c.y + 1) + f32(c.depth);
}

fn height_at(b: vec2<i32>, own: f32) -> f32 {
    let c = column(b);
    if !c.has {
        return own;
    }
    return top(c);
}

fn srgb_to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3<f32>(2.4)), c / 12.92, c <= vec3<f32>(0.04045));
}

fn linear_to_srgb(c: vec3<f32>) -> vec3<f32> {
    let k = clamp(c, vec3<f32>(0.0), vec3<f32>(1.0));
    return select(1.055 * pow(k, vec3<f32>(1.0 / 2.4)) - 0.055, k * 12.92, k <= vec3<f32>(0.0031308));
}

fn rgb(w: u32) -> vec3<f32> {
    return srgb_to_linear(vec3<f32>(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u)) / 255.0);
}

fn px4(w: u32) -> vec4<f32> {
    return vec4<f32>(f32(w & 255u), f32((w >> 8u) & 255u), f32((w >> 16u) & 255u), f32(w >> 24u)) / 16.0;
}

// The top of material `m` at `uv` (in the block's square), and how much
// of it is there (alpha).
fn texel(m: u32, uv: vec2<f32>, lod: f32) -> vec4<f32> {
    let mat = materials[m];
    let layer = i32(mat.x & 0xffffu);
    let tint = rgb(mat.y);
    if layer == 0 {
        return vec4<f32>(tint, 1.0);
    }
    let area = px4(mat.w);
    if uv.x < area.x || uv.y < area.y || uv.x > area.z || uv.y > area.w {
        return vec4<f32>(0.0);
    }
    let r = px4(mat.z);
    let t = (uv - area.xy) / max(area.zw - area.xy, vec2<f32>(1e-3));
    let s = textureSampleLevel(atlas, blocks, mix(r.xy, r.zw, t), layer, lod);
    return vec4<f32>(s.rgb * tint, s.a);
}

fn relief(h: f32, w: f32, e: f32, n: f32, s: f32) -> f32 {
    let light = vec3<f32>(-0.55, 0.75, -0.55);
    let d = clamp(vec2<f32>(e - w, s - n) * 0.5, vec2<f32>(-6.0), vec2<f32>(6.0));
    let normal = normalize(vec3<f32>(-d.x, 1.0, -d.y));
    let l = normalize(light);
    let flat = l.y;
    let step = clamp((h - n) + (h - w), -4.0, 4.0) * 0.025;
    return clamp(0.3 + 0.7 * max(dot(normal, l), 0.0) / flat + step, 0.25, 1.35);
}

fn cave_light(depth: f32) -> f32 {
    return clamp(1.0 - depth / 120.0, 0.45, 1.0);
}

fn far_colour(pos: vec2<f32>) -> vec4<f32> {
    let tile = vec2<i32>(floor(pos / 128.0));
    let e = entry(p.origin + tile) >> 16u;
    if e == 0u {
        return vec4<f32>(0.0);
    }
    let s = e - 1u;
    let size = f32(p.far_size);
    let local = (pos - vec2<f32>(tile) * 128.0) / 128.0 * size;
    let lod = clamp(log2(max(p.blocks_per_px * size / 128.0, 1e-4)), 0.0, log2(size));
    // Kept inside the tile's slot at that level.
    let half = 0.5 * exp2(floor(lod));
    let inside = clamp(local, vec2<f32>(half), vec2<f32>(size - half));
    let total = f32(p.far_side) * size;
    let uv = (vec2<f32>(f32(s % p.far_side), f32(s / p.far_side)) * size + inside) / total;
    return textureSampleLevel(far, soft, uv, lod);
}

@compute @workgroup_size(8, 8)
fn main(@builtin(global_invocation_id) id: vec3<u32>) {
    if id.x >= p.size.x || id.y >= p.size.y {
        return;
    }
    let d = (vec2<f32>(id.xy) + 0.5 - vec2<f32>(p.size) * 0.5) * p.blocks_per_px;
    var w = d;
    switch p.turn {
        case 1u: { w = vec2<f32>(-d.y, d.x); }
        case 2u: { w = -d; }
        case 3u: { w = vec2<f32>(d.y, -d.x); }
        default: {}
    }
    let pos = p.center + w;
    var colour = p.background.rgb;
    let b = vec2<i32>(floor(pos));
    var c: Column;
    if p.use_near != 0u {
        c = column(b);
    }
    if c.known {
        if !c.has {
            colour = select(p.background.rgb, ROCK, p.caves != 0u);
        } else {
            let uv = fract(pos);
            let lod = log2(max(16.0 * p.blocks_per_px, 1e-4));
            var g = texel(c.block, uv, lod);
            if g.a < 0.5 {
                // The holes of leaves and glass: their colour, in shade.
                let m = materials[c.block];
                g = vec4<f32>(textureSampleLevel(atlas, blocks, vec2<f32>(0.5), i32(m.x & 0xffffu), 4.0).rgb * rgb(m.y) * 0.55, 1.0);
            }
            var col = g.rgb;
            if c.depth > 0u {
                let fluid = select(p.water, p.lava, c.lava);
                let wt = texel(fluid, uv, lod).rgb;
                // The floor seen through the water, tinted by it, then the
                // water over it, thicker the deeper.
                let a = select(clamp(0.55 + f32(c.depth) * 0.08, 0.55, 0.95), 1.0, c.lava);
                col = mix(col * wt * 2.5, wt, a);
            }
            if c.over != 0u {
                let o = texel(c.over, uv, lod);
                col = mix(col, o.rgb, o.a);
            }
            // The relief: the slope, the edges of the steps around, and the
            // shadow of what stands higher to the north-west.
            let h = top(c);
            let hw = height_at(b + vec2<i32>(-1, 0), h);
            let he = height_at(b + vec2<i32>(1, 0), h);
            let hn = height_at(b + vec2<i32>(0, -1), h);
            let hs = height_at(b + vec2<i32>(0, 1), h);
            var k = relief(h, hw, he, hn, hs);
            let f = fract(pos);
            let edge = 0.3;
            var ao = 1.0;
            ao *= 1.0 - 0.16 * clamp((hw - h) / 3.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, edge, f.x));
            ao *= 1.0 - 0.16 * clamp((he - h) / 3.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, edge, 1.0 - f.x));
            ao *= 1.0 - 0.16 * clamp((hn - h) / 3.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, edge, f.y));
            ao *= 1.0 - 0.16 * clamp((hs - h) / 3.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, edge, 1.0 - f.y));
            // A rim of light along the edges of a step down to the north-west.
            let rim = 0.05 * clamp((h - hw) / 2.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, 0.12, f.x)) + 0.05 * clamp((h - hn) / 2.0, 0.0, 1.0) * (1.0 - smoothstep(0.0, 0.12, f.y));
            var shade = 1.0;
            for (var i = 1; i <= 10; i++) {
                let q = pos - vec2<f32>(f32(i) * 0.7);
                let hq = height_at(vec2<i32>(floor(q)), 0.0);
                if hq > h + f32(i) * 0.95 {
                    shade = 0.68;
                    break;
                }
            }
            k = k * ao * shade + rim;
            k = mix(1.0, k, p.relief);
            if p.caves != 0u {
                k *= cave_light(f32(p.slice - 1 - c.y));
            }
            colour = col * k * EXPOSURE;
        }
    } else {
        let f = far_colour(pos);
        if f.a > 0.5 {
            colour = f.rgb;
        } else if p.caves != 0u && entry(p.origin + vec2<i32>(floor(pos / 128.0))) != 0u {
            colour = ROCK;
        }
    }
    textureStore(image, vec2<i32>(id.xy), vec4<f32>(linear_to_srgb(colour), 1.0));
}
