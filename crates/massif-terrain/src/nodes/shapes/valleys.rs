//! Cut into a relief along a line: canyons, glacial valleys, faults.

use super::{Warp, bounds, finest, samples, smoothstep};
use crate::geom::{Curve, Segment};
use crate::kernel::Ctx;
use crate::nodes::height_out;
use crate::noise::Fbm;
use crate::params::Point;
use crate::registry::{Family, Field, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};

const HEIGHT_IN: PortSpec = PortSpec::new("height", "Height", PortKind::Height, "The relief to cut into (a ground at Base when unconnected).").optional();
const BASE: ParamSpec = ParamSpec::float("base", "Base", Unit::Blocks, 140.0, -2048.0, 4096.0, "The height of the ground cut into when Height is unconnected, in blocks.").soft(-64.0, 320.0);
const WIDTH_FIELDS: &[Field] = &[Field { name: "width", label: "Width", unit: Unit::Ratio, default: 1.0, min: 0.1, max: 3.0, help: "How wide it is here, as a share of Width." }];
const PATH_FIELDS: &[Field] = &[
    Field { name: "depth", label: "Depth", unit: Unit::Ratio, default: 1.0, min: 0.0, max: 3.0, help: "How deep it is here, as a share of Depth." },
    Field { name: "width", label: "Width", unit: Unit::Ratio, default: 1.0, min: 0.1, max: 3.0, help: "How wide it is here, as a share of Width." },
];

/// A line drawn on the viewer, or the default across the terrain.
fn path_or(p: &crate::params::Params, name: &str) -> Vec<Point> {
    let given = p.points(name);
    if given.len() >= 2 {
        return given.to_vec();
    }
    [(0.1, 0.35), (0.4, 0.5), (0.65, 0.45), (0.92, 0.7)].iter().map(|&(x, z)| Point { x, z, values: vec![1.0, 1.0] }).collect()
}

/// The candidates of a line's nearest point for each 16 × 16 cell of the
/// kernel's tile.
struct Near {
    curve: Curve,
    cells: Vec<Vec<Segment>>,
    nx: usize,
    reach: f64,
    soft: f64,
}

impl Near {
    fn new(ctx: &Ctx, points: &[Point], closed: bool, width: f64, reach: f64, wander: f64) -> Option<Self> {
        let curve = Curve::new(points, ctx.space.size as f64, closed, (width / 8.0).clamp(1.0, 64.0))?;
        let (x0, z0, x1, z1) = bounds(ctx);
        let segs = curve.segments_near(x0, z0, x1, z1, reach + wander);
        let step = ctx.spacing() * 16.0;
        let (nx, nz) = ((ctx.rect.w as usize).div_ceil(16), (ctx.rect.h as usize).div_ceil(16));
        let soft = width * 0.1;
        let cells = curve.cells(&segs, x0, z0, step, nx, nz, reach + wander, 4.0 * soft + 2.0 * wander);
        Some(Self { curve, cells, nx, reach: reach + wander, soft })
    }

    fn at(&self, ctx: &Ctx, k: usize, x: f64, z: f64) -> Option<crate::geom::Near> {
        let w = ctx.rect.w as usize;
        let cell = &self.cells[(k / w / 16) * self.nx + (k % w) / 16];
        self.curve.nearest_soft(cell, x, z, self.reach, self.soft)
    }
}

/// Steps of `steps` ledges and risers over `s` (0–1).
fn stepped(s: f32, steps: f32) -> f32 {
    if steps <= 0.0 {
        return s;
    }
    let t = s * (steps + 1.0);
    let i = t.floor();
    ((i + smoothstep(0.5, 1.0, t - i)) / (steps + 1.0)).min(1.0)
}

// --------------------------------------------------------------- canyon --

pub static CANYON: NodeSpec = NodeSpec {
    id: "valley.canyon",
    label: "Canyon",
    family: Family::Shape,
    summary: "A canyon cut into the relief: along a line drawn on the terrain, or a network of them winding across it, with a flat floor and stepped walls.",
    help: "Draw the Path on the viewer for one canyon (each point can make it deeper or wider there), or leave it empty for a network of canyons, Scale apart, winding across the terrain with side canyons (Branches). Width is the canyon's width at the rim, Floor the share of it that is flat bottom, Steps how many ledges the rock beds make in its walls. Feed it a plateau (a Mesas node, a Constant) through Height.",
    keywords: &["canyon", "gorge", "ravine", "grand canyon", "river", "cut", "valley", "slot"],
    inputs: &[HEIGHT_IN],
    outputs: &[height_out("The relief with the canyon cut into it."), PortSpec::new("mask", "Mask", PortKind::Mask, "1 in the canyon's floor, fading up its walls.")],
    params: &[
        ParamSpec::curve("path", "Path", PATH_FIELDS, false, "The canyon's course, drawn on the viewer; a network when not drawn."),
        ParamSpec::float("depth", "Depth", Unit::Blocks, 55.0, 0.0, 2048.0, "How deep the canyon is, in blocks.").soft(0.0, 200.0).drivable(),
        ParamSpec::float("width", "Width", Unit::Blocks, 260.0, 4.0, 16000.0, "How wide the canyon is at its rim, in blocks.").soft(10.0, 2000.0),
        ParamSpec::float("floor", "Floor", Unit::Ratio, 0.22, 0.0, 0.95, "The share of the width that is flat floor.").section(Section::Shape),
        ParamSpec::int("steps", "Steps", 3, 0, 16, "How many ledges the walls make.").section(Section::Shape),
        ParamSpec::float("meander", "Meander", Unit::Ratio, 0.4, 0.0, 1.0, "How much the canyon winds.").section(Section::Shape),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 1600.0, 64.0, 32000.0, "How far apart the canyons of a network are, in blocks.").soft(200.0, 8000.0).section(Section::Shape),
        ParamSpec::float("branches", "Branches", Unit::Ratio, 0.4, 0.0, 1.0, "How many side canyons a network has.").section(Section::Shape),
        BASE,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: canyon_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn canyon_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let width = p.num("width");
    let floor = p.f("floor");
    let steps = p.int("steps") as f32;
    let wander = p.num("meander") * width * 1.5;
    let warp = Warp::new(width * 4.0, wander, seed);
    let drawn = p.points("path").len() >= 2;
    let near = if drawn { Near::new(ctx, p.points("path"), false, width, width * 3.0 * 0.5, wander) } else { None };
    let scale = p.num("scale");
    let network = Fbm::down_to(scale, finest(ctx).max(scale / 8.0), 0.5, seed ^ 0x6a);
    let side = Fbm::down_to(scale * 0.4, finest(ctx).max(scale / 16.0), 0.5, seed ^ 0x6b);
    let branches = p.f("branches");
    for (k, x, z) in samples(ctx) {
        let ground = inputs[0].map_or(base, |h| h[k]);
        let (wx, wz) = warp.at(x, z);
        // Distance to the canyon's axis, in widths, and how deep it is.
        let (u, deep, wide) = if drawn {
            match near.as_ref().and_then(|n| n.at(ctx, k, wx, wz).map(|m| (m, n.curve.length()))) {
                // Shallower towards its ends.
                Some((n, length)) => {
                    let ends = smoothstep(0.0, 0.12, (n.along / length) as f32).min(smoothstep(1.0, 0.88, (n.along / length) as f32));
                    ((n.dist / (width * n.values[1] * 0.5)) as f32, n.values[0] as f32 * ends, n.values[1] as f32)
                }
                None => (f32::MAX, 0.0, 1.0),
            }
        } else {
            // The network: where a smooth noise crosses zero, at the
            // distance its value and slope give; side canyons narrower and
            // shallower.
            let main = (network.zero_distance(wx, wz) as f64 / (width * 0.5)) as f32;
            let branch = if branches > 0.0 { (side.zero_distance(wx, wz) as f64 / (width * 0.35 * branches as f64)) as f32 } else { f32::MAX };
            if main <= branch { (main, 1.0, 1.0) } else { (branch, 0.6, 0.7) }
        };
        let _ = wide;
        let carve = if u < floor {
            1.0
        } else if u < 1.0 {
            1.0 - stepped((u - floor) / (1.0 - floor), steps)
        } else {
            0.0
        };
        out[0][k] = ground - ctx.num_at("depth", k) * deep * carve;
        out[1][k] = carve;
    }
}

// ------------------------------------------------------------ glacial --

pub static GLACIAL: NodeSpec = NodeSpec {
    id: "valley.glacial",
    label: "Glacial valley",
    family: Family::Shape,
    summary: "A U-shaped valley a glacier carved, along a line drawn on the terrain: a wide flat floor gently descending between steep walls.",
    help: "Draw the Path on the viewer (each point can make it wider there). Floor height is the height of its flat floor where the path starts, Drop how much lower it lies at the end; Width is the valley's width from rim to rim, Floor the share of it that is flat, Steepness how sheer its walls are. Cut it into mountains through Height for a valley like Yosemite or Lauterbrunnen; a floor under the sea level makes a fjord.",
    keywords: &["glacial", "glacier", "u-valley", "trough", "fjord", "yosemite", "valley"],
    inputs: &[HEIGHT_IN],
    outputs: &[height_out("The relief with the valley carved."), PortSpec::new("mask", "Mask", PortKind::Mask, "1 on the valley's floor, fading up its walls.")],
    params: &[
        ParamSpec::curve("path", "Path", WIDTH_FIELDS, false, "The valley's course, drawn on the viewer."),
        ParamSpec::float("floor_height", "Floor height", Unit::Blocks, 75.0, -2048.0, 4096.0, "The height of the valley's flat floor where the path starts, in blocks.").soft(-64.0, 320.0).drivable(),
        ParamSpec::float("drop", "Drop", Unit::Blocks, 20.0, -2048.0, 2048.0, "How much lower the floor lies at the path's end than at its start, in blocks.").soft(-50.0, 150.0),
        ParamSpec::float("width", "Width", Unit::Blocks, 700.0, 16.0, 16000.0, "How wide the valley is from rim to rim, in blocks.").soft(50.0, 4000.0),
        ParamSpec::float("floor", "Floor", Unit::Ratio, 0.35, 0.0, 0.95, "The share of the width that is flat floor.").section(Section::Shape),
        ParamSpec::float("steepness", "Steepness", Unit::Ratio, 0.75, 0.0, 1.0, "How sheer the walls are.").section(Section::Shape),
        ParamSpec::float("meander", "Meander", Unit::Ratio, 0.2, 0.0, 1.0, "How much the valley winds.").section(Section::Shape),
        BASE,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: glacial_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn glacial_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let base = p.f("base");
    let width = p.num("width");
    let floor = p.f("floor");
    let power = 1.5 + 4.0 * p.f("steepness");
    let wander = p.num("meander") * width;
    let warp = Warp::new(width * 5.0, wander, ctx.seed);
    let path: Vec<Point> = path_or(p, "path").into_iter().map(|mut pt| {
        // The default path carries depth and width; this one width only.
        if pt.values.len() == 2 {
            pt.values.remove(0);
        }
        pt
    }).collect();
    let widest = path.iter().map(|pt| pt.value(0)).fold(1.0f64, f64::max);
    let near = Near::new(ctx, &path, false, width, width * widest * 0.5, wander);
    let drop = p.f("drop");
    for (k, x, z) in samples(ctx) {
        let ground = inputs[0].map_or(base, |h| h[k]);
        let (wx, wz) = warp.at(x, z);
        let (carve, level) = match near.as_ref().and_then(|n| n.at(ctx, k, wx, wz).map(|m| (m, n.curve.length()))) {
            Some((n, length)) => {
                let u = (n.dist / (width * n.values[0] * 0.5)) as f32;
                let s = ((u - floor) / (1.0 - floor)).clamp(0.0, 1.0);
                let c = (1.0 - s.powf(power)) * if u < 1.0 { 1.0 } else { 0.0 };
                (c, ctx.num_at("floor_height", k) - drop * (n.along / length) as f32)
            }
            None => (0.0, 0.0),
        };
        // From the ground down to the floor, never raising what it cuts.
        out[0][k] = ground.min(ground + (level - ground) * carve);
        out[1][k] = carve.clamp(0.0, 1.0);
    }
}

// ---------------------------------------------------------------- fault --

pub static FAULT: NodeSpec = NodeSpec {
    id: "fault.scarp",
    label: "Fault",
    family: Family::Shape,
    summary: "A fault along a line drawn on the terrain: the ground on one side lifted, a scarp between the two.",
    help: "Draw the fault's line on the viewer. Throw is how much higher the left side (of the way the line runs) stands; a negative throw lifts the right side. Width is the scarp's width, Ragged how broken its face is. Feed it the relief to break through Height.",
    keywords: &["fault", "scarp", "escarpment", "rift", "cliff", "step", "tectonic"],
    inputs: &[HEIGHT_IN],
    outputs: &[height_out("The relief with the fault."), PortSpec::new("mask", "Mask", PortKind::Mask, "1 on the scarp's face.")],
    params: &[
        ParamSpec::curve("path", "Line", &[], false, "The fault's line, drawn on the viewer."),
        ParamSpec::float("throw", "Throw", Unit::Blocks, 30.0, -1024.0, 1024.0, "How much higher the left side stands, in blocks (negative: the right side).").soft(-100.0, 100.0).drivable(),
        ParamSpec::float("width", "Width", Unit::Blocks, 40.0, 1.0, 4000.0, "How wide the scarp is, in blocks.").soft(2.0, 300.0),
        ParamSpec::float("ragged", "Ragged", Unit::Ratio, 0.4, 0.0, 1.0, "How broken the scarp's face and line are.").section(Section::Shape),
        BASE,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: fault_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn fault_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let base = p.f("base");
    let width = p.num("width");
    let ragged = p.num("ragged");
    let wander = ragged * width * 3.0;
    let warp = Warp::new(width * 8.0, wander, ctx.seed);
    let path = path_or(p, "path");
    let size = ctx.space.size as f64;
    // Near the line, its exact course; far from it only the side counts,
    // which a coarse copy of the line gives.
    let near = Near::new(ctx, &path, false, width.max(16.0), width * 4.0, wander);
    let coarse = Curve::new(&path, size, false, size / 64.0);
    let all = coarse.as_ref().map(|c| c.segments_near(-size, -size, 2.0 * size, 2.0 * size, 0.0)).unwrap_or_default();
    for (k, x, z) in samples(ctx) {
        let ground = inputs[0].map_or(base, |h| h[k]);
        let (wx, wz) = warp.at(x, z);
        let (step, face) = match near.as_ref().and_then(|n| n.at(ctx, k, wx, wz)) {
            Some(n) => {
                let signed = (n.side * n.dist / (width * 0.5)) as f32;
                (smoothstep(-1.0, 1.0, signed), 1.0 - signed.abs().min(1.0))
            }
            None => match coarse.as_ref().and_then(|c| c.nearest(&all, wx, wz, f64::MAX)) {
                Some(n) if n.side > 0.0 => (1.0, 0.0),
                _ => (0.0, 0.0),
            },
        };
        out[0][k] = ground + ctx.num_at("throw", k) * step;
        out[1][k] = face;
    }
}
