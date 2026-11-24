//! Volumes: fields over the world's blocks, made brick by brick (see
//! [`Kernel::Volume`]). Negative inside matter, positive in the air, the
//! distance to the surface in blocks where it can be: the field caves,
//! arches and overhangs are cut and added in.
//!
//! The Map Output takes two: `volume.carve`, cut out of the ground to air,
//! and `volume.add`, filled with rock; Apply writes them into the map's
//! sculpted blocks ([`crate::underground`]). What cave layers carve is
//! kept; a volume is for what they cannot do: a tunnel along a drawn line,
//! one great hall, a network of tunnels in a band of rock, an arch.

use crate::kernel::{BRICK, Brick, Ctx, Geo, Grid};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use massif_core::caves::noise;

pub const NODES: &[&NodeSpec] = &[&FROM_HEIGHT, &SPHERE, &BOX, &TUBE, &NOISE, &CAVE_NOISE, &COMBINE, &SHAPE, &LIMIT];

/// Far from anything: where a volume holds nothing.
pub const EMPTY: f32 = 1.0e4;

/// Each block of brick `out` from its world coordinates (its middle is at
/// `+ 0.5`), `x` fastest then `z` then `y`.
#[inline]
fn each(out: &mut Brick, mut f: impl FnMut(i32, i32, i32) -> f32) {
    let [x0, y0, z0] = out.origin;
    let mut n = 0;
    for j in 0..BRICK {
        for k in 0..BRICK {
            for i in 0..BRICK {
                out.data[n] = f(x0 + i, y0 + j, z0 + k);
                n += 1;
            }
        }
    }
}

/// The column under block `(x, z)` of a raster input read under a brick
/// (level 0, from the terrain's corner).
#[inline]
fn column(ctx: &Ctx, g: Option<&Grid>, x: i32, z: i32, or: f32) -> f32 {
    g.map_or(or, |g| g.at(x - ctx.space.x0, z - ctx.space.z0))
}

const fn pos(name: &'static str, label: &'static str, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Fraction, 0.5, -1.0, 2.0, help).soft(0.0, 1.0)
}

const fn y(name: &'static str, label: &'static str, default: f64, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Blocks, default, -2048.0, 4096.0, help).soft(-64.0, 320.0)
}

const fn blocks(name: &'static str, label: &'static str, default: f64, max: f64, help: &'static str) -> ParamSpec {
    ParamSpec::float(name, label, Unit::Blocks, default, 0.0, 4096.0, help).soft(0.0, max)
}

pub static FROM_HEIGHT: NodeSpec = NodeSpec {
    id: "volume.from_height",
    label: "Relief to volume",
    family: Family::Volume,
    summary: "The ground of a relief as a volume: matter under its heights, air above.",
    help: "Where volumes start: caves are cut out of it, arches and overhangs added to it, and Volume to relief reads its top back. Its value is the height above the ground in blocks, negative under it.",
    keywords: &["volume", "density", "sdf", "3d", "solid", "voxel"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief.")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative under the ground, the height above it elsewhere.")],
    params: &[],
    kernel: Kernel::Volume { cpu: from_height, halo: 0 },
    ..NodeSpec::BASE
};

fn from_height(ctx: &Ctx, origin: [i32; 3], rasters: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let [x0, y0, z0] = origin;
    let b = &mut out[0];
    for k in 0..BRICK {
        for i in 0..BRICK {
            // Sample (x, z) of level 0 is the block from the terrain's corner.
            let h = rasters[0].map_or(ctx.space.sea_level, |g| g.at(x0 + i - ctx.space.x0, z0 + k - ctx.space.z0));
            for j in 0..BRICK {
                // The block's middle against the ground's height.
                b.data[((j * BRICK + k) * BRICK + i) as usize] = (y0 + j) as f32 + 0.5 - h;
            }
        }
    }
}

pub static SPHERE: NodeSpec = NodeSpec {
    id: "volume.sphere",
    label: "Sphere",
    family: Family::Volume,
    summary: "A ball, or one round each point: a hall to carve, a boulder to add.",
    help: "A ball of Radius blocks at a place of the terrain and a height; given points, one at each instead (its radius times the point's scale, Depth blocks under it): chambers where Scatter put them. Its value is the distance to its surface, negative inside.",
    keywords: &["sphere", "ball", "chamber", "hall", "room", "cavern", "boulder", "sdf"],
    inputs: &[PortSpec::new("points", "Points", PortKind::Points, "Where the balls are.").or("one ball, at the settings' place")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative inside the balls.")],
    params: &[
        pos("x", "X", "Where it is, west to east across the terrain."),
        pos("z", "Z", "Where it is, north to south across the terrain."),
        y("y", "Height", 20.0, "The height of its middle, in blocks."),
        blocks("radius", "Radius", 16.0, 128.0, "In blocks."),
        ParamSpec::float("squash", "Squash", Unit::Ratio, 1.0, 0.1, 4.0, "Its height against its width: under 1, a flat hall.").section(Section::Shape),
        blocks("depth", "Depth", 0.0, 256.0, "Given points, how far under them the balls' middles lie.").section(Section::Shape),
    ],
    kernel: Kernel::Volume { cpu: sphere, halo: 0 },
    ..NodeSpec::BASE
};

/// Balls to draw: middle and radius.
fn balls(ctx: &Ctx) -> Vec<([f32; 3], f32)> {
    let p = ctx.params;
    let r = p.f("radius").max(0.5);
    match ctx.geo(0).and_then(Geo::points) {
        Some(points) => points.items.iter().map(|q| ([q.pos[0] as f32, q.pos[1] as f32 - p.f("depth"), q.pos[2] as f32], r * q.scale.max(0.01))).collect(),
        None => {
            let s = ctx.space.size as f32;
            vec![([ctx.space.x0 as f32 + p.f("x") * s, p.f("y"), ctx.space.z0 as f32 + p.f("z") * s], r)]
        }
    }
}

fn sphere(ctx: &Ctx, origin: [i32; 3], _: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let squash = ctx.params.f("squash").max(0.1);
    let lo = origin.map(|v| v as f32);
    let hi = origin.map(|v| (v + BRICK) as f32);
    // Only the balls reaching the brick.
    let near: Vec<([f32; 3], f32)> = balls(ctx)
        .into_iter()
        .filter(|(c, r)| {
            let reach = [*r + 1.0, *r * squash + 1.0, *r + 1.0];
            (0..3).all(|a| c[a] + reach[a] >= lo[a] && c[a] - reach[a] <= hi[a])
        })
        .collect();
    if near.is_empty() {
        out[0].data.fill(EMPTY);
        return;
    }
    each(&mut out[0], |x, y, z| {
        let q = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
        near.iter()
            .map(|(c, r)| {
                let d = [q[0] - c[0], (q[1] - c[1]) / squash, q[2] - c[2]];
                // Squashed, the distance is scaled by the smaller axis.
                ((d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - r) * squash.min(1.0)
            })
            .fold(EMPTY, f32::min)
    });
}

pub static BOX: NodeSpec = NodeSpec {
    id: "volume.box",
    label: "Box",
    family: Family::Volume,
    summary: "A box with rounded edges: a room, a gallery, a block of rock.",
    help: "A box of Width × Height × Length blocks at a place of the terrain and a height, turned about the vertical and rounded at its edges. Its value is the distance to its surface, negative inside.",
    keywords: &["box", "cube", "room", "gallery", "block", "rectangle", "sdf"],
    inputs: &[],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative inside the box.")],
    params: &[
        pos("x", "X", "Where its middle is, west to east across the terrain."),
        pos("z", "Z", "Where its middle is, north to south across the terrain."),
        y("y", "Height", 20.0, "The height of its middle, in blocks."),
        blocks("width", "Width", 32.0, 256.0, "West to east, in blocks."),
        blocks("height", "Height", 16.0, 256.0, "Bottom to top, in blocks."),
        blocks("length", "Length", 32.0, 256.0, "North to south, in blocks."),
        ParamSpec::float("turn", "Turn", Unit::Degrees, 0.0, -180.0, 180.0, "Turned about the vertical.").section(Section::Shape),
        blocks("rounding", "Rounding", 2.0, 32.0, "The radius of its rounded edges.").section(Section::Shape),
    ],
    kernel: Kernel::Volume { cpu: box_cpu, halo: 0 },
    ..NodeSpec::BASE
};

fn box_cpu(ctx: &Ctx, _: [i32; 3], _: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let p = ctx.params;
    let s = ctx.space.size as f32;
    let c = [ctx.space.x0 as f32 + p.f("x") * s, p.f("y"), ctx.space.z0 as f32 + p.f("z") * s];
    let r = p.f("rounding").max(0.0);
    let half = [p.f("width") / 2.0, p.f("height") / 2.0, p.f("length") / 2.0].map(|h| h.max(0.5));
    let inner = half.map(|h| (h - r).max(0.0));
    let (sin, cos) = p.f("turn").to_radians().sin_cos();
    // Out of reach of the brick: nothing.
    let reach = half[0].hypot(half[2]) + 1.0;
    let o = out[0].origin.map(|v| v as f32);
    if c[0] + reach < o[0] || c[0] - reach > o[0] + BRICK as f32 || c[2] + reach < o[2] || c[2] - reach > o[2] + BRICK as f32 || c[1] + half[1] + 1.0 < o[1] || c[1] - half[1] - 1.0 > o[1] + BRICK as f32 {
        out[0].data.fill(EMPTY);
        return;
    }
    each(&mut out[0], |x, y, z| {
        let (dx, dy, dz) = (x as f32 + 0.5 - c[0], y as f32 + 0.5 - c[1], z as f32 + 0.5 - c[2]);
        let q = [(dx * cos + dz * sin).abs() - inner[0], dy.abs() - inner[1], (-dx * sin + dz * cos).abs() - inner[2]];
        let outside = q.map(|v| v.max(0.0));
        (outside[0] * outside[0] + outside[1] * outside[1] + outside[2] * outside[2]).sqrt() + q[0].max(q[1]).max(q[2]).min(0.0) - r
    });
}

pub static TUBE: NodeSpec = NodeSpec {
    id: "volume.tube",
    label: "Tube",
    family: Family::Volume,
    summary: "A tunnel along lines: a cave that follows a drawn path, a lava tube, a mine.",
    help: "A round tunnel of Radius blocks along each line (its width, when Radius is 0), Depth blocks under the line's points, wider and narrower along its length by Variation. Draw the path, or give the splines of a river or a road to dig under. Its value is the distance to the tunnel's wall, negative inside.",
    keywords: &["tube", "tunnel", "spline", "path", "worm", "cave", "lava tube", "mine", "sdf"],
    inputs: &[PortSpec::new("splines", "Splines", PortKind::Splines, "The lines to follow.").or("the line drawn in Path")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative inside the tunnels.")],
    params: &[
        ParamSpec::curve("path", "Path", &[], false, "The line the tunnel follows, when no splines are given (x and z across the terrain)."),
        y("y", "Height", 20.0, "The height of the drawn path."),
        blocks("radius", "Radius", 4.0, 32.0, "In blocks; 0: half each line's width."),
        blocks("depth", "Depth", 0.0, 256.0, "How far under the lines' points the tunnel runs."),
        ParamSpec::float("variation", "Variation", Unit::Ratio, 0.3, 0.0, 1.0, "How much wider and narrower it gets along its length.").section(Section::Shape),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Volume { cpu: tube, halo: 0 },
    ..NodeSpec::BASE
};

/// Segments of the tunnels: ends and radius.
fn segments(ctx: &Ctx) -> Vec<([f32; 3], [f32; 3], f32)> {
    let p = ctx.params;
    let depth = p.f("depth");
    let mut segs = Vec::new();
    let mut add = |pts: &[[f32; 3]], closed: bool, r: f32| {
        let n = pts.len();
        let count = if closed && n > 2 { n } else { n.saturating_sub(1) };
        for i in 0..count {
            segs.push((pts[i], pts[(i + 1) % n], r.max(0.5)));
        }
    };
    match ctx.geo(0).and_then(Geo::splines) {
        Some(lines) => {
            for l in &lines.lines {
                let r = if p.f("radius") > 0.0 { p.f("radius") } else { l.width / 2.0 };
                let pts: Vec<[f32; 3]> = l.points.iter().map(|q| [q[0] as f32, q[1] as f32 - depth, q[2] as f32]).collect();
                add(&pts, l.closed, r);
            }
        }
        None => {
            let s = ctx.space.size as f32;
            let pts: Vec<[f32; 3]> = p.points("path").iter().map(|q| [ctx.space.x0 as f32 + q.x as f32 * s, p.f("y") - depth, ctx.space.z0 as f32 + q.z as f32 * s]).collect();
            add(&pts, false, p.f("radius").max(0.5));
        }
    }
    segs
}

fn tube(ctx: &Ctx, origin: [i32; 3], _: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let var = ctx.params.f("variation").clamp(0.0, 1.0);
    let seed = ctx.seed as u64;
    let lo = origin.map(|v| v as f32);
    let near: Vec<_> = segments(ctx)
        .into_iter()
        .filter(|(a, b, r)| {
            let reach = r * (1.0 + var) + 1.0;
            (0..3).all(|k| a[k].min(b[k]) - reach <= lo[k] + BRICK as f32 && a[k].max(b[k]) + reach >= lo[k])
        })
        .collect();
    if near.is_empty() {
        out[0].data.fill(EMPTY);
        return;
    }
    each(&mut out[0], |x, y, z| {
        let q = [x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5];
        let swell = 1.0 + var * noise::gradient(seed ^ 0x70BE, q[0] / 24.0, q[1] / 24.0, q[2] / 24.0) * 1.5;
        near.iter()
            .map(|(a, b, r)| {
                let ab = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
                let aq = [q[0] - a[0], q[1] - a[1], q[2] - a[2]];
                let len2 = (ab[0] * ab[0] + ab[1] * ab[1] + ab[2] * ab[2]).max(1e-6);
                let t = ((aq[0] * ab[0] + aq[1] * ab[1] + aq[2] * ab[2]) / len2).clamp(0.0, 1.0);
                let d = [aq[0] - ab[0] * t, aq[1] - ab[1] * t, aq[2] - ab[2] * t];
                (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt() - r * swell
            })
            .fold(EMPTY, f32::min)
    });
}

pub static NOISE: NodeSpec = NodeSpec {
    id: "volume.noise",
    label: "Noise volume",
    family: Family::Volume,
    summary: "Blobs of 3D noise taking a share of space: pockets, sponge-like rock, floating islands.",
    help: "3D gradient noise of three octaves, cut so that the volume takes Share of space, in blobs about Scale blocks across (flatter as Squash falls). Limit it to a band of heights or a mask with Limit volume, carve it from the ground or add it above.",
    keywords: &["noise", "3d", "blobs", "perlin", "pockets", "sponge", "islands", "volume"],
    inputs: &[],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative inside the blobs, about in blocks.")],
    params: &[
        ParamSpec::float("share", "Share", Unit::Ratio, 0.2, 0.0, 1.0, "The share of space the blobs take."),
        blocks("scale", "Scale", 48.0, 256.0, "About how large a blob is, in blocks."),
        ParamSpec::float("squash", "Squash", Unit::Ratio, 0.6, 0.1, 4.0, "Their height against their width.").section(Section::Shape),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Volume { cpu: noise_cpu, halo: 0 },
    ..NodeSpec::BASE
};

fn noise_cpu(ctx: &Ctx, _: [i32; 3], _: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let p = ctx.params;
    let share = p.f("share").clamp(0.0, 1.0);
    let s = p.f("scale").max(1.0);
    let sy = s * p.f("squash").max(0.1);
    let seed = ctx.seed as u64 ^ 0x000B_10B5;
    let cut = noise::cheese_quantiles().quantile(share);
    // The fbm climbs about 1.6 a cell: back to blocks.
    let to_blocks = s / 1.6;
    each(&mut out[0], |x, y, z| (noise::fbm(seed, (x as f32 + 0.5) / s, (y as f32 + 0.5) / sy, (z as f32 + 0.5) / s, 3) - cut) * to_blocks);
}

pub static CAVE_NOISE: NodeSpec = NodeSpec {
    id: "volume.caves",
    label: "Cave noise",
    family: Family::Volume,
    summary: "The game's noise caves as a volume — chambers with pillars, winding tunnels, noodles — between two heights, where a mask says.",
    help: "The noise the game carves its caves from since 1.18, as a volume to shape further: cheese chambers with pillars, spaghetti tunnels and noodles, each with its density and size, fading out toward Lowest and Highest and where the mask falls. Unlike a Cave layer, it has no roof, aquifers or decoration: combine it with other volumes (keep it under a relief with Combine ▸ Intersect, carve it along a band of strata with a mask), then carve it through the Map Output's Volume carve.",
    keywords: &["caves", "cheese", "spaghetti", "noodles", "tunnels", "network", "noise", "volume"],
    inputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "Where the caves go, from 0 to 1.").or("1 everywhere")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Negative in the caves, about in blocks.")],
    params: &[
        ParamSpec::float("cheese", "Chambers", Unit::Ratio, 0.5, 0.0, 1.0, "How much of the rock the chambers take."),
        ParamSpec::float("spaghetti", "Tunnels", Unit::Ratio, 0.5, 0.0, 1.0, "How many winding tunnels."),
        ParamSpec::float("noodles", "Noodles", Unit::Ratio, 0.3, 0.0, 1.0, "How many thin tunnels."),
        y("low", "Lowest", -56.0, "The lowest height the caves reach."),
        y("high", "Highest", 48.0, "The highest height they reach."),
        ParamSpec::float("size", "Size", Unit::Ratio, 1.0, 0.5, 2.0, "How large they are (1: as the game carves them).").section(Section::Shape),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Volume { cpu: cave_noise, halo: 0 },
    ..NodeSpec::BASE
};

fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

fn cave_noise(ctx: &Ctx, origin: [i32; 3], rasters: &[Option<&Grid>], _: &[Option<&Brick>], out: &mut [Brick]) {
    let p = ctx.params;
    let (lo, hi) = (p.f("low"), p.f("high"));
    if hi <= lo || origin[1] as f32 > hi + 1.0 || (origin[1] + BRICK) as f32 <= lo - 1.0 {
        out[0].data.fill(EMPTY);
        return;
    }
    let s = p.f("size").clamp(0.5, 2.0);
    let seed = ctx.seed as u64;
    let (cheese, spaghetti, noodles) = (p.f("cheese") * 0.14, p.f("spaghetti") * 0.05, p.f("noodles") * 0.03);
    let pillar = noise::signed_quantiles().quantile(0.92);
    let ridge = noise::ridge_quantiles();
    let fade = 10.0f32.min((hi - lo) / 2.0);
    each(&mut out[0], |x, y, z| {
        let w = smoothstep(0.15, 0.6, column(ctx, rasters[0], x, z, 1.0));
        let (fx, fy, fz) = (x as f32 + 0.5, y as f32 + 0.5, z as f32 + 0.5);
        let thin = if fy < lo || fy > hi { 0.0 } else { smoothstep(0.0, 1.0, ((fy - lo) / fade).min((hi - fy) / fade)) };
        let m = w * thin;
        if m <= 0.0 {
            return EMPTY;
        }
        let mut d = EMPTY;
        let share = cheese * m;
        if share > 0.0 {
            let n = noise::fbm(seed ^ 0xC4EE, fx / (56.0 * s), fy / (34.0 * s), fz / (56.0 * s), 3);
            let mut c = n - noise::cheese_quantiles().quantile(share);
            let q = noise::gradient(seed ^ 0x9111, fx / (6.0 * s), fy / (36.0 * s), fz / (6.0 * s));
            c = c.max((q - pillar) * 0.5);
            d = d.min(c * 34.0 * s / 1.6);
        }
        let share = spaghetti * m;
        if share > 0.0 {
            let swell = 0.7 + 0.3 * (noise::gradient(seed ^ 0x5E11, fx / 150.0, fy / 100.0, fz / 150.0) + 1.0);
            let wd = ridge.quantile(share.sqrt()) * swell;
            let a = noise::gradient(seed ^ 0x5A, fx / (80.0 * s), fy / (52.0 * s), fz / (80.0 * s));
            let b = noise::gradient(seed ^ 0x5B, fx / (80.0 * s), fy / (52.0 * s), fz / (80.0 * s));
            d = d.min((a.abs().max(b.abs()) - wd) * 52.0 * s / 1.6);
        }
        let share = noodles * m;
        if share > 0.0 {
            let wd = ridge.quantile(share.sqrt());
            let a = noise::gradient(seed ^ 0x40A, fx / (30.0 * s), fy / (26.0 * s), fz / (30.0 * s));
            let b = noise::gradient(seed ^ 0x40B, fx / (30.0 * s), fy / (26.0 * s), fz / (30.0 * s));
            d = d.min((a.abs().max(b.abs()) - wd) * 26.0 * s / 1.6);
        }
        d
    });
}

const OPS: &[Choice] = &[
    Choice { value: "union", label: "Union", help: "Both: what is inside either." },
    Choice { value: "subtract", label: "Subtract", help: "A without B: B cut out of A." },
    Choice { value: "intersect", label: "Intersect", help: "Only what is inside both." },
];

pub static COMBINE: NodeSpec = NodeSpec {
    id: "volume.combine",
    label: "Combine volumes",
    family: Family::Volume,
    summary: "Two volumes as one: their union, A without B, or what they share, with a smooth seam if asked.",
    help: "The booleans of solids. Union joins caves into one network; Subtract cuts B out of A (a cave out of the ground, a doorway out of a wall); Intersect keeps the part of A inside B (cave noise kept under a relief). Smoothness rounds where they meet, over that many blocks.",
    keywords: &["combine", "boolean", "union", "subtract", "difference", "intersect", "csg", "smooth", "volume"],
    inputs: &[PortSpec::new("a", "A", PortKind::Density, "A volume.").or("nothing"), PortSpec::new("b", "B", PortKind::Density, "Another.").or("nothing")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "The two combined.")],
    params: &[ParamSpec::choice("op", "Operation", OPS, "union", "How they combine."), blocks("smooth", "Smoothness", 0.0, 32.0, "Over how many blocks the seam is rounded.")],
    kernel: Kernel::Volume { cpu: combine, halo: 0 },
    ..NodeSpec::BASE
};

/// The smooth minimum of `a` and `b` over `k` blocks.
#[inline]
pub fn smin(a: f32, b: f32, k: f32) -> f32 {
    if k <= 0.0 {
        return a.min(b);
    }
    let h = (k - (a - b).abs()).max(0.0) / k;
    a.min(b) - h * h * k * 0.25
}

fn combine(ctx: &Ctx, _: [i32; 3], _: &[Option<&Grid>], volumes: &[Option<&Brick>], out: &mut [Brick]) {
    let k = ctx.params.f("smooth").max(0.0);
    let op = ctx.params.choice("op");
    let n = out[0].data.len();
    let at = |v: Option<&Brick>, i: usize| v.map_or(EMPTY, |b| b.data[i]);
    for i in 0..n {
        let (a, b) = (at(volumes[0], i), at(volumes[1], i));
        out[0].data[i] = match op {
            "subtract" => -smin(-a, b, k),
            "intersect" => -smin(-a, -b, k),
            _ => smin(a, b, k),
        };
    }
}

pub static SHAPE: NodeSpec = NodeSpec {
    id: "volume.shape",
    label: "Shape volume",
    family: Family::Volume,
    summary: "A volume grown or shrunk, and roughened: rock walls that are not smooth.",
    help: "Grow pushes the surface out by that many blocks (negative: in); Roughness moves it in and out by up to that many blocks with 3D noise of Roughness scale, so that carved halls and tunnels get the uneven walls of real caves.",
    keywords: &["grow", "shrink", "offset", "rough", "displace", "erode", "noise", "volume"],
    inputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "The volume.")],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "Grown and roughened.")],
    params: &[
        ParamSpec::float("grow", "Grow", Unit::Blocks, 0.0, -64.0, 64.0, "Blocks the surface moves out (in when negative)."),
        blocks("roughness", "Roughness", 2.0, 16.0, "How far the noise moves the surface, in blocks."),
        blocks("scale", "Roughness scale", 8.0, 64.0, "How large the bumps are, in blocks.").section(Section::Shape),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Volume { cpu: shape, halo: 0 },
    ..NodeSpec::BASE
};

fn shape(ctx: &Ctx, _: [i32; 3], _: &[Option<&Grid>], volumes: &[Option<&Brick>], out: &mut [Brick]) {
    let Some(v) = volumes[0] else {
        out[0].data.fill(EMPTY);
        return;
    };
    let p = ctx.params;
    let (grow, rough, s) = (p.f("grow"), p.f("roughness"), p.f("scale").max(1.0));
    let seed = ctx.seed as u64 ^ 0x5A4E;
    let mut n = 0;
    each(&mut out[0], |x, y, z| {
        let d = v.data[n];
        n += 1;
        // Far from the surface, the noise changes nothing worth working out.
        if rough <= 0.0 || d.abs() > rough * 2.0 + grow.abs() + 4.0 {
            return d - grow;
        }
        d - grow - rough * noise::fbm(seed, (x as f32 + 0.5) / s, (y as f32 + 0.5) / s, (z as f32 + 0.5) / s, 2)
    });
}

pub static LIMIT: NodeSpec = NodeSpec {
    id: "volume.limit",
    label: "Limit volume",
    family: Family::Volume,
    summary: "A volume kept between two heights, or under the ground, and where a mask says.",
    help: "Keeps a volume between Lowest and Highest and where the mask is over one half, fading it out over Fade blocks; with Under the ground and a relief, also at least Cover blocks under its surface, so that caves never break through it.",
    keywords: &["limit", "mask", "band", "depth", "clip", "range", "volume"],
    inputs: &[
        PortSpec::new("volume", "Volume", PortKind::Density, "The volume."),
        PortSpec::new("mask", "Mask", PortKind::Mask, "Where it is kept.").or("everywhere"),
        PortSpec::new("height", "Height", PortKind::Height, "The relief, for Under the ground.").or("none"),
    ],
    outputs: &[PortSpec::new("volume", "Volume", PortKind::Density, "The volume, kept where asked.")],
    params: &[
        y("low", "Lowest", -64.0, "The lowest height it reaches."),
        y("high", "Highest", 320.0, "The highest height it reaches."),
        blocks("fade", "Fade", 4.0, 32.0, "Over how many blocks it thins out at its limits."),
        ParamSpec::flag("under", "Under the ground", true, "Keeps it under the relief given."),
        blocks("cover", "Cover", 4.0, 64.0, "Blocks of ground kept over it, under the relief."),
    ],
    kernel: Kernel::Volume { cpu: limit, halo: 0 },
    ..NodeSpec::BASE
};

fn limit(ctx: &Ctx, _: [i32; 3], rasters: &[Option<&Grid>], volumes: &[Option<&Brick>], out: &mut [Brick]) {
    let Some(v) = volumes[0] else {
        out[0].data.fill(EMPTY);
        return;
    };
    let p = ctx.params;
    let (lo, hi, fade, cover) = (p.f("low"), p.f("high"), p.f("fade").max(0.0), p.f("cover"));
    let under = p.flag("under") && rasters[2].is_some();
    let mut n = 0;
    each(&mut out[0], |x, y, z| {
        let d = v.data[n];
        n += 1;
        let fy = y as f32 + 0.5;
        // The distance outside the allowed room: positive outside.
        let mut room = (lo - fy).max(fy - hi);
        let m = column(ctx, rasters[1], x, z, 1.0);
        room = room.max((0.5 - m) * 2.0 * (fade + 1.0));
        if under {
            room = room.max(fy - (column(ctx, rasters[2], x, z, 0.0) - cover));
        }
        // Intersect, rounded over the fade.
        -smin(-d, -(room + fade), fade)
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::params::Params;
    use crate::space::Space;

    fn run(spec: &'static NodeSpec, set: &[(&str, f64)], origin: [i32; 3], volumes: &[Option<&Brick>]) -> Brick {
        let mut params = Params::defaults(spec, "n");
        for (k, v) in set {
            params.set_num(k, *v);
        }
        let space = Space::new(256);
        let env = crate::kernel::NoEnv;
        let ctx = Ctx { space: &space, lod: 0, rect: crate::space::Rect::new(0, 0, 16, 16), seed: 7, params: &params, driven: &[], reads: &[], env: &env, geos: &[] };
        let Kernel::Volume { cpu, .. } = spec.kernel else { unreachable!() };
        let mut out = vec![Brick::filled(origin, 0.0)];
        let rasters = vec![None; spec.inputs.len()];
        cpu(&ctx, origin, &rasters, volumes, &mut out);
        out.remove(0)
    }

    #[test]
    fn a_sphere_is_its_distance() {
        let b = run(&SPHERE, &[("x", 0.0625), ("z", 0.0625), ("y", 8.0), ("radius", 4.0)], [0, 0, 0], &[]);
        // Its middle (16, 8, 16) lies at the brick's far corner: block 15 is
        // half a block and a little from it.
        assert!(b.at(15, 7, 15) < -3.0);
        assert!((b.at(15, 7, 11) - (4.5f32.hypot(0.5).hypot(0.5) - 4.0)).abs() < 1e-4);
        assert!(b.at(0, 0, 0) > 0.0);
        // Far away: nothing, without working anything out.
        let far = run(&SPHERE, &[("radius", 4.0)], [0, 0, 0], &[]);
        assert!(far.data.iter().all(|&v| v == EMPTY));
    }

    #[test]
    fn booleans_follow_the_solids() {
        let a = Brick::filled([0, 0, 0], -1.0);
        let b = Brick::filled([0, 0, 0], -2.0);
        let get = |op: &str| {
            let given = [("op".to_owned(), serde_json::json!(op))].into_iter().collect();
            let p = Params::decode(&COMBINE, "n", &given, "n", &mut Vec::new());
            let space = Space::new(256);
            let env = crate::kernel::NoEnv;
            let ctx = Ctx { space: &space, lod: 0, rect: crate::space::Rect::new(0, 0, 16, 16), seed: 0, params: &p, driven: &[], reads: &[], env: &env, geos: &[] };
            let mut out = vec![Brick::filled([0, 0, 0], 0.0)];
            combine(&ctx, [0, 0, 0], &[None, None], &[Some(&a), Some(&b)], &mut out);
            out[0].data[0]
        };
        assert_eq!(get("union"), -2.0);
        assert_eq!(get("intersect"), -1.0);
        assert_eq!(get("subtract"), 2.0);
    }

    #[test]
    fn cave_noise_takes_about_its_share_within_its_heights() {
        let mut inside = 0;
        let mut total = 0;
        for bx in 0..4 {
            for bz in 0..4 {
                let b = run(&CAVE_NOISE, &[("low", -64.0), ("high", 64.0)], [bx * 16, 0, bz * 16], &[]);
                inside += b.data.iter().filter(|&&v| v < 0.0).count();
                total += b.data.len();
            }
        }
        let share = inside as f32 / total as f32;
        assert!(share > 0.01 && share < 0.3, "{share}");
        let above = run(&CAVE_NOISE, &[("low", -64.0), ("high", 64.0)], [0, 96, 0], &[]);
        assert!(above.data.iter().all(|&v| v == EMPTY));
    }

    #[test]
    fn noise_takes_its_share() {
        let mut inside = 0;
        let mut total = 0;
        for bx in 0..6 {
            for by in 0..3 {
                let b = run(&NOISE, &[("share", 0.3), ("scale", 16.0)], [bx * 16, by * 16, 0], &[]);
                inside += b.data.iter().filter(|&&v| v < 0.0).count();
                total += b.data.len();
            }
        }
        let share = inside as f32 / total as f32;
        assert!((share - 0.3).abs() < 0.12, "{share}");
    }
}
