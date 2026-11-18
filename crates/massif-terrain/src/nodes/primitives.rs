//! Primitives: what makes relief from nothing.

use super::height_out;
use crate::kernel::{Ctx, GpuKernel, Uniforms};
use crate::noise::{Fbm as ShapeFbm, Fractal, MAX_OCTAVES, Octave, Style, fractal, voronoi};
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&CONSTANT, &FBM, &ERODED, &CELLS, &GRADIENT, &SHAPE];

pub static CONSTANT: NodeSpec = NodeSpec {
    id: "pattern.constant",
    label: "Constant",
    family: Family::Primitive,
    summary: "The same height everywhere.",
    help: "A flat ground at one height: a start to build on, or a level to combine with another relief.",
    keywords: &["flat", "value", "plane"],
    inputs: &[],
    outputs: &[height_out("The height, everywhere.")],
    params: &[ParamSpec::float("value", "Height", Unit::Blocks, 64.0, -2048.0, 4096.0, "The height of the ground everywhere, in blocks.").soft(-64.0, 320.0)],
    kernel: Kernel::Point { cpu: constant_cpu, gpu: Some(&CONSTANT_GPU) },
    ..NodeSpec::BASE
};

fn constant_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    out[0].fill(ctx.params.f("value"));
}

static CONSTANT_GPU: GpuKernel = GpuKernel {
    name: "pattern.constant",
    source: include_str!("wgsl/constant.wgsl"),
    uniforms: |ctx, _| Uniforms::default().f32(ctx.params.f("value")).finish(),
};

const STYLES: &[Choice] = &[
    Choice { value: "fbm", label: "Rolling", help: "Octaves summed: smooth hills and valleys." },
    Choice { value: "ridged", label: "Ridged", help: "Octaves folded into sharp crests, with finer ridges branching where the coarser ones are high: mountain ranges." },
    Choice { value: "billow", label: "Billowy", help: "Octaves folded the other way: puffy, rounded mounds with creases between them." },
];

pub static FBM: NodeSpec = NodeSpec {
    id: "noise.fbm",
    label: "Noise",
    family: Family::Primitive,
    summary: "Fractal noise: relief at every scale, from its largest features down to fine detail.",
    help: "The base of most landscapes. Scale is the size of the largest features; each octave adds detail half as large (lacunarity) and weaker (roughness). Rolling makes hills, Ridged mountain crests, Billowy rounded mounds. Height is how far the lowest and highest points lie apart, Base where the lowest lies.",
    keywords: &["fbm", "perlin", "fractal", "ridged", "billow", "hills", "random"],
    inputs: &[],
    outputs: &[height_out("The relief, from Base to Base + Height.")],
    params: &[
        ParamSpec::choice("style", "Style", STYLES, "fbm", "How the octaves add up."),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 1000.0, 2.0, 65536.0, "Blocks across the largest features.").soft(16.0, 8000.0),
        ParamSpec::float("height", "Height", Unit::Blocks, 200.0, 0.0, 4096.0, "How far the lowest and the highest points lie apart, in blocks.").soft(0.0, 1000.0).drivable(),
        ParamSpec::float("base", "Base", Unit::Blocks, 40.0, -2048.0, 4096.0, "The height of the lowest points, in blocks.").soft(-64.0, 320.0).drivable(),
        ParamSpec::int("octaves", "Detail", 8, 1, MAX_OCTAVES as i64, "Octaves summed: each adds detail twice as fine.").section(Section::Detail),
        ParamSpec::float("roughness", "Roughness", Unit::Ratio, 0.5, 0.05, 0.95, "How strong each octave is next to the one before: higher, rougher.").section(Section::Detail),
        ParamSpec::float("lacunarity", "Lacunarity", Unit::Ratio, 2.0, 1.2, 4.0, "How much finer each octave is than the one before.").section(Section::Advanced),
        ParamSpec::float("offset_x", "Offset X", Unit::Blocks, 0.0, -1.0e6, 1.0e6, "Moves the noise east, in blocks.").soft(-10000.0, 10000.0).section(Section::Advanced),
        ParamSpec::float("offset_z", "Offset Z", Unit::Blocks, 0.0, -1.0e6, 1.0e6, "Moves the noise south, in blocks.").soft(-10000.0, 10000.0).section(Section::Advanced),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: fbm_cpu, gpu: Some(&FBM_GPU) },
    ..NodeSpec::BASE
};

fn style(ctx: &Ctx) -> Style {
    match ctx.params.choice("style") {
        "ridged" => Style::Ridged,
        "billow" => Style::Billow,
        _ => Style::Fbm,
    }
}

fn octaves(ctx: &Ctx) -> Vec<Octave> {
    let p = ctx.params;
    let f = Fractal {
        scale: p.num("scale"),
        octaves: p.int("octaves") as u32,
        lacunarity: p.num("lacunarity"),
        gain: p.num("roughness"),
        offset: (p.num("offset_x"), p.num("offset_z")),
        seed: ctx.seed,
    };
    f.octaves(ctx.world(ctx.rect.x, ctx.rect.z), ctx.spacing())
}

fn fbm_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let octs = octaves(ctx);
    let style = style(ctx);
    let w = ctx.rect.w as usize;
    for (j, row) in out[0].chunks_mut(w).enumerate() {
        for (i, v) in row.iter_mut().enumerate() {
            let k = j * w + i;
            *v = ctx.num_at("base", k) + ctx.num_at("height", k) * fractal(&octs, style, i as f32, j as f32);
        }
    }
}

static FBM_GPU: GpuKernel = GpuKernel {
    name: "noise.fbm",
    source: include_str!("wgsl/fbm.wgsl"),
    uniforms: |ctx, driven| {
        let octs = octaves(ctx);
        let mut u = Uniforms::default();
        u.u32(style(ctx) as u32).u32(octs.len() as u32).f32(ctx.params.f("height")).f32(ctx.params.f("base"));
        u.u32(driven[0] as u32).u32(driven[1] as u32);
        for k in 0..MAX_OCTAVES {
            octs.get(k).copied().unwrap_or_default().write(&mut u);
        }
        u.finish()
    },
};

// ------------------------------------------------- primitives on the CPU --

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Each sample of the kernel's tile: index and position in blocks.
fn each<'a>(ctx: &'a Ctx<'a>) -> impl Iterator<Item = (usize, f64, f64)> + 'a {
    let w = ctx.rect.w as usize;
    (0..ctx.rect.area()).map(move |k| {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        (k, x, z)
    })
}

pub static ERODED: NodeSpec = NodeSpec {
    id: "noise.eroded",
    label: "Eroded noise",
    family: Family::Primitive,
    summary: "Fractal noise whose fine detail fades on slopes and gathers on heights: smooth flanks, branching valleys, the look of an eroded relief without simulating it.",
    help: "Each octave is weakened where the coarser ones slope (after Iñigo Quilez), so slopes stay smooth and detail stays on crests and in hollows. Erosion is how strongly. A quick base for hills and old mountains, or a mask that follows the relief's structure.",
    keywords: &["erosion", "derivative", "iq", "hills", "valleys", "fbm"],
    inputs: &[],
    outputs: &[height_out("The relief, from Base to Base + Height.")],
    params: &[
        ParamSpec::float("scale", "Scale", Unit::Blocks, 1200.0, 2.0, 65536.0, "Blocks across the largest features.").soft(16.0, 8000.0),
        ParamSpec::float("height", "Height", Unit::Blocks, 120.0, 0.0, 4096.0, "How far the lowest and the highest points lie apart, in blocks.").soft(0.0, 600.0).drivable(),
        ParamSpec::float("base", "Base", Unit::Blocks, 62.0, -2048.0, 4096.0, "The height of the lowest points, in blocks.").soft(-64.0, 320.0).drivable(),
        ParamSpec::float("erosion", "Erosion", Unit::Ratio, 1.5, 0.0, 6.0, "How strongly slopes smooth the finer octaves."),
        ParamSpec::float("roughness", "Roughness", Unit::Ratio, 0.5, 0.05, 0.95, "How strong each octave is next to the one before.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: eroded_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn eroded_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let finest = (2.0 * ctx.spacing()).max(2.0);
    let f = ShapeFbm::down_to(p.num("scale"), finest, p.num("roughness"), ctx.seed);
    let strength = p.f("erosion");
    for (k, x, z) in each(ctx) {
        out[0][k] = ctx.num_at("base", k) + ctx.num_at("height", k) * f.eroded(x, z, strength);
    }
}

const CELL_MODES: &[Choice] = &[
    Choice { value: "f1", label: "Distance", help: "Distance to the nearest point: round pits around each point, high between them." },
    Choice { value: "f2", label: "Second distance", help: "Distance to the second nearest point: soft, pillowy cells." },
    Choice { value: "edges", label: "Edges", help: "How far from the edge between two cells: ridges along the edges (cracks when inverted)." },
    Choice { value: "cells", label: "Cells", help: "One value per cell: a mosaic of flat pieces." },
];

pub static CELLS: NodeSpec = NodeSpec {
    id: "noise.cells",
    label: "Cells",
    family: Family::Primitive,
    summary: "Cellular (Voronoi) noise: points scattered one per cell, and a value from where a sample lies among them — pits, pillows, ridges along the edges, or a mosaic.",
    help: "Scale is how far apart the points are, Jitter how far they stray from a grid. Distance makes round pits around the points, Edges ridges between cells (dried mud, crazed rock), Cells a mosaic of flat pieces (terraces, fields). Height and Base place the result in blocks; for a mask, 1 and 0.",
    keywords: &["voronoi", "worley", "cellular", "cracks", "mosaic", "cells", "pits"],
    inputs: &[],
    outputs: &[height_out("The cells, from Base to Base + Height.")],
    params: &[
        ParamSpec::choice("mode", "Mode", CELL_MODES, "edges", "What value a sample takes."),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 300.0, 2.0, 65536.0, "How far apart the points are, in blocks.").soft(8.0, 4000.0),
        ParamSpec::float("jitter", "Jitter", Unit::Ratio, 0.9, 0.0, 1.0, "How far the points stray from a grid."),
        ParamSpec::float("height", "Height", Unit::Blocks, 1.0, -4096.0, 4096.0, "How far the lowest and the highest values lie apart (1 for a mask).").soft(0.0, 200.0).drivable(),
        ParamSpec::float("base", "Base", Unit::Blocks, 0.0, -2048.0, 4096.0, "The lowest value (0 for a mask).").soft(-64.0, 320.0).drivable(),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: cells_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn cells_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let scale = p.num("scale");
    let jitter = p.num("jitter");
    let mode = p.choice("mode");
    for (k, x, z) in each(ctx) {
        let (f1, f2, id) = voronoi(x / scale, z / scale, jitter, ctx.seed);
        let v = match mode {
            "f1" => (f1 / 0.9).min(1.0),
            "f2" => (f2 / 1.3).min(1.0),
            "cells" => (id & 0xffff) as f32 / 65535.0,
            _ => 1.0 - ((f2 - f1) / 0.6).min(1.0),
        };
        out[0][k] = ctx.num_at("base", k) + ctx.num_at("height", k) * v;
    }
}

const GRADIENTS: &[Choice] = &[
    Choice { value: "linear", label: "Linear", help: "From the first point to the second, straight." },
    Choice { value: "radial", label: "Radial", help: "Out from the first point, reaching 1 at the second's distance." },
];

pub static GRADIENT: NodeSpec = NodeSpec {
    id: "pattern.gradient",
    label: "Gradient",
    family: Family::Primitive,
    summary: "A value rising from one point to another, straight or around the first: tilt a relief, fade a mask towards a coast, raise a dome.",
    help: "Place the two ends on the viewer: the value is 0 at the first and 1 at the second (Radial: 0 at the first, 1 at the second's distance from it), shaped by the Profile; Height and Base place it in blocks. Multiply a relief by it to make it fade out, or add it to tilt land towards the sea.",
    keywords: &["gradient", "ramp", "tilt", "slope", "radial", "fade", "falloff"],
    inputs: &[],
    outputs: &[PortSpec::new("value", "Value", PortKind::Mask, "From Base at the first point to Base + Height at the second.")],
    params: &[
        ParamSpec::choice("kind", "Kind", GRADIENTS, "linear", "Straight, or around the first point."),
        ParamSpec::points("ends", "Ends", &[], 2, "Where the value is 0, then where it is 1, placed on the viewer."),
        ParamSpec::profile("profile", "Profile", &[[0.0, 0.0], [1.0, 1.0]], "How the value rises from the first point (left) to the second (right)."),
        ParamSpec::float("height", "Height", Unit::Blocks, 1.0, -4096.0, 4096.0, "The value at the second point, over Base (1 for a mask).").soft(-200.0, 200.0),
        ParamSpec::float("base", "Base", Unit::Blocks, 0.0, -2048.0, 4096.0, "The value at the first point (0 for a mask).").soft(-64.0, 320.0),
    ],
    kernel: Kernel::Point { cpu: gradient_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn gradient_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let ends = p.points("ends");
    let (a, b) = match ends {
        [a, b, ..] => ((a.x * size, a.z * size), (b.x * size, b.z * size)),
        [a] => ((a.x * size, a.z * size), (size, a.z * size)),
        [] => ((0.0, 0.5 * size), (size, 0.5 * size)),
    };
    let (dx, dz) = (b.0 - a.0, b.1 - a.1);
    let l2 = (dx * dx + dz * dz).max(1e-9);
    let radial = p.choice("kind") == "radial";
    let (height, base) = (p.f("height"), p.f("base"));
    for (k, x, z) in each(ctx) {
        let t = if radial { ((x - a.0).hypot(z - a.1) / l2.sqrt()).clamp(0.0, 1.0) } else { (((x - a.0) * dx + (z - a.1) * dz) / l2).clamp(0.0, 1.0) };
        out[0][k] = base + height * p.profile("profile", t) as f32;
    }
}

const SHAPES: &[Choice] = &[
    Choice { value: "circle", label: "Circle", help: "A disc." },
    Choice { value: "square", label: "Square", help: "A square, turned by Angle." },
    Choice { value: "polygon", label: "Polygon", help: "A regular polygon of Sides sides." },
    Choice { value: "star", label: "Star", help: "A star of Sides points." },
];

pub static SHAPE: NodeSpec = NodeSpec {
    id: "pattern.shape",
    label: "Shape",
    family: Family::Primitive,
    summary: "A geometric shape as a mask: a disc, a square, a polygon or a star, its edge as soft as wanted.",
    help: "Place its centre on the viewer; Radius is its size, Softness how far its edge fades (0 sharp), Angle turns it. Use it to keep something within an area, to raise a platform, to cut a crater-like hollow.",
    keywords: &["circle", "disc", "square", "polygon", "star", "mask", "stencil", "area"],
    inputs: &[],
    outputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "1 inside the shape, 0 outside.")],
    params: &[
        ParamSpec::choice("shape", "Shape", SHAPES, "circle", "The shape."),
        ParamSpec::points("center", "Centre", &[], 1, "Where the shape's centre is, placed on the viewer."),
        ParamSpec::float("radius", "Radius", Unit::Blocks, 1000.0, 1.0, 65536.0, "How far the shape reaches from its centre, in blocks.").soft(10.0, 8000.0),
        ParamSpec::float("softness", "Softness", Unit::Blocks, 200.0, 0.0, 16000.0, "How far the edge fades, in blocks.").soft(0.0, 2000.0),
        ParamSpec::int("sides", "Sides", 6, 3, 32, "How many sides (or points) a polygon (or a star) has.").section(Section::Shape),
        ParamSpec::float("angle", "Angle", Unit::Degrees, 0.0, -180.0, 180.0, "How far the shape is turned, in degrees.").section(Section::Shape),
    ],
    kernel: Kernel::Point { cpu: shape_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn shape_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let (cx, cz) = p.points("center").first().map_or((0.5 * size, 0.5 * size), |c| (c.x * size, c.z * size));
    let r = p.num("radius");
    let soft = p.num("softness").max(1e-3);
    let shape = p.choice("shape");
    let sides = p.int("sides") as f64;
    let turn = p.num("angle").to_radians();
    for (k, x, z) in each(ctx) {
        let (dx, dz) = (x - cx, z - cz);
        let (s, c) = turn.sin_cos();
        let (u, v) = (c * dx + s * dz, -s * dx + c * dz);
        // How far inside the edge, in blocks.
        let inside = match shape {
            "square" => r - u.abs().max(v.abs()),
            "polygon" | "star" => {
                let sector = std::f64::consts::TAU / sides;
                let a = v.atan2(u).rem_euclid(sector) - sector * 0.5;
                let d = u.hypot(v);
                if shape == "polygon" {
                    r * (sector * 0.5).cos() - d * a.cos()
                } else {
                    // Points at full radius, notches at half.
                    let reach = r * (0.5 + 0.5 * (a.abs() / (sector * 0.5)).min(1.0).powf(1.5).mul_add(-1.0, 1.0));
                    reach - d
                }
            }
            _ => r - dx.hypot(dz),
        };
        out[0][k] = smooth(-(soft as f32) * 0.5, soft as f32 * 0.5, inside as f32);
    }
}
