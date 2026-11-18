//! Mountains: massifs of a kind (alpine, rounded), ranges along a drawn
//! spine, peaks placed by hand.

use super::{Relief, Warp, bounds, finest, samples, smoothstep};
use crate::geom::Curve;
use crate::kernel::Ctx;
use crate::nodes::height_out;
use crate::noise::{Fbm, noise1};
use crate::params::Point;
use crate::registry::{Family, Field, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};

const MASK_OUT: PortSpec = PortSpec::new("mask", "Mask", PortKind::Mask, "1 on the mountains, fading to 0 at their feet.");
const AREA_IN: PortSpec = PortSpec::new("area", "Area", PortKind::Mask, "Where the mountains stand, from 0 to 1 (where Coverage says, when unconnected).").optional();

const fn height(default: f64) -> ParamSpec {
    ParamSpec::float("height", "Height", Unit::Blocks, default, 0.0, 4096.0, "How high the summits rise over Base, in blocks (a Minecraft world stops at 320 unless a data pack raises it).").soft(0.0, 600.0).drivable()
}

const BASE: ParamSpec = ParamSpec::float("base", "Base", Unit::Blocks, 70.0, -2048.0, 4096.0, "The height of the ground at the mountains' feet, in blocks (0 to add them to another relief).").soft(-64.0, 320.0);

const fn sharpness(default: f64) -> ParamSpec {
    ParamSpec::float("sharpness", "Sharpness", Unit::Ratio, default, 0.0, 1.0, "0 gives rounded crests and smooth valleys, 1 knife-edge arêtes.").section(Section::Detail)
}

const fn roughness(default: f64) -> ParamSpec {
    ParamSpec::float("roughness", "Roughness", Unit::Ratio, default, 0.0, 1.0, "How much small relief the slopes carry.").section(Section::Detail)
}

const fn detail(default: f64) -> ParamSpec {
    ParamSpec::float("detail", "Detail size", Unit::Blocks, default, 16.0, 8000.0, "The size of the largest relief on the slopes (ridges, gullies), in blocks.").soft(32.0, 2000.0).section(Section::Detail)
}

const WARP: ParamSpec = ParamSpec::float("warp", "Wander", Unit::Ratio, 0.35, 0.0, 1.0, "How far the mountains' lines wander from straight.").section(Section::Detail);

const fn profile(default: &'static [[f64; 2]]) -> ParamSpec {
    ParamSpec::profile("profile", "Profile", default, "The shape of the flanks, from the foot (left) to the crest (right).").section(Section::Shape)
}

/// Concave feet, steep middles, sharp tops.
const STEEP: &[[f64; 2]] = &[[0.0, 0.0], [0.3, 0.08], [0.65, 0.5], [1.0, 1.0]];
/// Mountains broad to the top: a range's flanks.
const FULL: &[[f64; 2]] = &[[0.0, 0.0], [0.25, 0.14], [0.6, 0.62], [1.0, 1.0]];
/// Convex, worn shoulders.
const WORN: &[[f64; 2]] = &[[0.0, 0.0], [0.3, 0.3], [0.7, 0.82], [1.0, 1.0]];

// --------------------------------------------------------------- massifs --

const MASSIF_HELP: &str = "Mountains over an area: summits about Peak spacing apart joined by ridges, valleys between them, and the relief of Detail size on their slopes. Connect a mask to Area to say where they stand (a painted area, a coastline's land), or let Coverage spread them. Height is the summits' height over Base; the Profile shapes the massif from its edges to its heart. Run erosion after them for valleys carved by water.";

pub static ALPINE: NodeSpec = NodeSpec {
    id: "mountain.alpine",
    label: "Alpine mountains",
    family: Family::Shape,
    summary: "Young, sharp mountains: pyramidal summits, knife-edge ridges and deep valleys, like the Alps or the Himalaya.",
    help: MASSIF_HELP,
    keywords: &["alps", "himalaya", "peaks", "sharp", "high", "massif", "ridged"],
    inputs: &[AREA_IN],
    outputs: &[height_out("The mountains over Base."), MASK_OUT],
    params: &[
        height(220.0),
        BASE,
        ParamSpec::float("peak_spacing", "Peak spacing", Unit::Blocks, 500.0, 64.0, 16000.0, "How far apart the summits stand, in blocks.").soft(200.0, 4000.0),
        ParamSpec::float("coverage", "Coverage", Unit::Ratio, 0.55, 0.0, 1.0, "How much of the terrain the mountains cover when Area is unconnected."),
        profile(STEEP),
        sharpness(0.8),
        roughness(0.55),
        detail(160.0),
        WARP,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: massif_cpu, gpu: None },
    ..NodeSpec::BASE
};

pub static ROUNDED: NodeSpec = NodeSpec {
    id: "mountain.rounded",
    label: "Rounded mountains",
    family: Family::Shape,
    summary: "Old, worn mountains: rounded summits, broad shoulders and gentle valleys, like the Appalachians or the Vosges.",
    help: MASSIF_HELP,
    keywords: &["old", "worn", "hills", "appalachian", "vosges", "round", "massif", "highlands"],
    inputs: &[AREA_IN],
    outputs: &[height_out("The mountains over Base."), MASK_OUT],
    params: &[
        height(120.0),
        BASE,
        ParamSpec::float("peak_spacing", "Peak spacing", Unit::Blocks, 800.0, 64.0, 16000.0, "How far apart the summits stand, in blocks.").soft(200.0, 6000.0),
        ParamSpec::float("coverage", "Coverage", Unit::Ratio, 0.6, 0.0, 1.0, "How much of the terrain the mountains cover when Area is unconnected."),
        profile(WORN),
        sharpness(0.12),
        roughness(0.4),
        detail(260.0),
        WARP,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: massif_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn massif_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let spacing = p.num("peak_spacing");
    let sharp = p.f("sharpness");
    let rounded = p.spec().id == "mountain.rounded";
    let warp = Warp::new(spacing * 2.0, p.num("warp") * spacing * 0.5, seed);
    let spread = Fbm::new(spacing * 5.0, 4, 0.5, seed ^ 0x1b87_3593);
    // The ridges joining the summits: a few octaves from Peak spacing down.
    let ridges = Fbm::down_to(spacing * 1.5, (spacing / 8.0).max(finest(ctx)), 0.5, seed ^ 0xcc9e_2d51);
    let summits = Fbm::new(spacing * 0.7, 2, 0.5, seed ^ 0x85eb_ca6b);
    let relief = Relief::new(p.num("detail"), p.num("roughness"), p.num("sharpness"), finest(ctx), seed);
    let roughness = p.f("roughness");
    let threshold = 0.75 - 0.5 * p.f("coverage");
    let base = p.f("base");
    for (k, x, z) in samples(ctx) {
        let (wx, wz) = warp.at(x, z);
        let m = match inputs[0] {
            Some(area) => area[k].clamp(0.0, 1.0),
            None => smoothstep(threshold - 0.12, threshold + 0.12, 0.5 + 0.5 * spread.at(wx, wz)),
        };
        out[1][k] = m;
        if m <= 0.0 {
            out[0][k] = base;
            continue;
        }
        let mass = p.profile("profile", m as f64) as f32;
        let (main, floor) = if rounded {
            (ridges.eroded(wx, wz, 0.8).powf(0.8), 0.3)
        } else {
            // Broad mountains with straight flanks, their crests sharper the
            // finer they are, and summits standing out along them.
            let r = ridges.mountains(wx, wz, sharp);
            let e = ridges.eroded(wx, wz, 1.5);
            let peaks = 0.75 + 0.25 * summits.at(wx, wz);
            ((e + (r - e) * (0.4 + 0.6 * sharp)) * peaks, 0.12)
        };
        let shape = mass * (floor + (1.0 - floor) * main);
        let d = relief.at(wx, wz);
        out[0][k] = base + ctx.num_at("height", k) * (shape * (1.0 + roughness * 0.35 * (d - 0.5) * main)).max(0.0);
    }
}

// ---------------------------------------------------------------- ranges --

const SPINE_FIELDS: &[Field] = &[
    Field { name: "height", label: "Height", unit: Unit::Ratio, default: 1.0, min: 0.0, max: 3.0, help: "How high the range is at this point, as a share of Height." },
    Field { name: "width", label: "Width", unit: Unit::Ratio, default: 1.0, min: 0.1, max: 3.0, help: "How wide the range is at this point, as a share of Width." },
];

/// The spine a new range takes before one is drawn: across the terrain.
fn default_spine() -> Vec<Point> {
    [(0.12, 0.68), (0.4, 0.42), (0.62, 0.38), (0.88, 0.22)].iter().map(|&(x, z)| Point { x, z, values: vec![1.0, 1.0] }).collect()
}

pub static RANGE: NodeSpec = NodeSpec {
    id: "mountain.range",
    label: "Mountain range",
    family: Family::Shape,
    summary: "A range of mountains along a line drawn on the terrain: a crest of summits and saddles, with side ridges running down to its feet.",
    help: "Draw the Spine on the viewer: the crest follows it, summits about Peak spacing apart with saddles between (Saddles: how deep), and side ridges (Spurs) run down the flanks every Spur spacing, valleys between them. Width is the range's width from foot to foot; each point of the spine can make it higher or wider there. Asymmetry makes one side steeper (positive: the left of the way the spine runs). Combine with a coastline or a plain with Max, or Add with Base 0.",
    keywords: &["ridge", "chain", "spine", "cordillera", "sierra", "curve", "line", "drawn"],
    inputs: &[],
    outputs: &[height_out("The range over Base."), MASK_OUT],
    params: &[
        ParamSpec::curve("spine", "Spine", SPINE_FIELDS, false, "The line the crest follows, drawn on the viewer; each point can make the range higher or wider there."),
        height(230.0),
        BASE,
        ParamSpec::float("width", "Width", Unit::Blocks, 1800.0, 64.0, 32000.0, "How wide the range is from foot to foot, in blocks.").soft(300.0, 10000.0),
        profile(FULL),
        ParamSpec::float("peak_spacing", "Peak spacing", Unit::Blocks, 450.0, 32.0, 16000.0, "How far apart the summits stand along the crest, in blocks.").soft(100.0, 4000.0).section(Section::Shape),
        ParamSpec::float("saddles", "Saddles", Unit::Ratio, 0.45, 0.0, 1.0, "How deep the crest dips between summits.").section(Section::Shape),
        ParamSpec::float("spurs", "Spurs", Unit::Ratio, 0.6, 0.0, 1.0, "How strongly side ridges run down the flanks, valleys between them.").section(Section::Shape),
        ParamSpec::float("spur_spacing", "Spur spacing", Unit::Blocks, 260.0, 32.0, 8000.0, "How far apart the side ridges are, in blocks.").soft(100.0, 2000.0).section(Section::Shape),
        ParamSpec::float("asymmetry", "Asymmetry", Unit::Ratio, 0.0, -1.0, 1.0, "Makes one flank wider and gentler, the other steeper: positive widens the left of the way the spine runs.").section(Section::Shape),
        sharpness(0.75),
        roughness(0.5),
        detail(150.0),
        WARP,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: range_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn range_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let width = p.num("width");
    let given = p.points("spine");
    let spine = if given.len() >= 2 { given.to_vec() } else { default_spine() };
    let widest = spine.iter().map(|pt| pt.value(1)).fold(1.0f64, f64::max);
    let asym = p.num("asymmetry");
    let wander = p.num("warp") * width * 0.2;
    let reach = width * widest * 0.5 * (1.0 + asym.abs() * 0.6) + wander * 1.5 + 1.0;
    let Some(curve) = Curve::new(&spine, ctx.space.size as f64, false, (width / 32.0).clamp(1.0, 96.0)) else {
        out[0].fill(base);
        out[1].fill(0.0);
        return;
    };
    let (x0, z0, x1, z1) = bounds(ctx);
    let segs = curve.segments_near(x0, z0, x1, z1, reach);
    if segs.is_empty() {
        out[0].fill(base);
        out[1].fill(0.0);
        return;
    }
    // Candidate segments by cells of 16 × 16 samples (warped positions
    // stray by up to `wander`).
    const CELL: usize = 16;
    let step = ctx.spacing() * CELL as f64;
    let (nx, nz) = ((ctx.rect.w as usize).div_ceil(CELL), (ctx.rect.h as usize).div_ceil(CELL));
    let soft = width * 0.05;
    let cells = curve.cells(&segs, x0, z0, step, nx, nz, reach + wander, 4.0 * soft + 2.0 * wander);
    let warp = Warp::new(width * 0.8, wander, seed);
    let relief = Relief::new(p.num("detail"), p.num("roughness"), p.num("sharpness"), finest(ctx), seed);
    let (peak, saddles) = (p.num("peak_spacing"), p.f("saddles"));
    let summits = Fbm::new(peak, 2, 0.4, seed ^ 0x9e37_79b9);
    // The ridge network the crest and flanks are made of, in the world's
    // own space: nothing in it jumps where the spine bends.
    let sharp = p.f("sharpness");
    let net = Fbm::down_to(peak * 1.5, (peak / 8.0).max(finest(ctx)), 0.5, seed ^ 0xcc9e_2d51);
    let (spur_spacing, spurs) = (p.num("spur_spacing"), p.f("spurs"));
    let spur_bend = Fbm::new(spur_spacing * 2.0, 3, 0.5, seed ^ 0x7f4a_7c15);
    let roughness = p.f("roughness");
    for (k, x, z) in samples(ctx) {
        let (wx, wz) = warp.at(x, z);
        let (i, j) = (k % ctx.rect.w as usize / CELL, k / ctx.rect.w as usize / CELL);
        let Some(near) = curve.nearest_soft(&cells[j * nx + i], wx, wz, reach, soft) else {
            out[0][k] = base;
            out[1][k] = 0.0;
            continue;
        };
        let half = width * near.values[1] * 0.5 * (1.0 + asym * near.side * 0.6);
        let u = (1.0 - near.dist / half.max(1.0)) as f32;
        if u <= 0.0 {
            out[0][k] = base;
            out[1][k] = 0.0;
            continue;
        }
        let cross = p.profile("profile", u as f64) as f32;
        // Side ridges down the flanks, bending as they go, valleys between.
        let q = near.along / spur_spacing + 0.45 * spur_bend.at(wx, wz) as f64;
        let ridge = (1.0 - noise1(q, seed ^ 0x3c6e).abs()).powf(2.0);
        let flank = 4.0 * u * (1.0 - u);
        let carved = cross * (1.0 - spurs * flank.powf(0.7) * (1.0 - ridge));
        // The crest: the network pulled up along the spine, summits and saddles.
        let r = net.mountains(wx, wz, sharp);
        let main = r + (1.0 - r) * u.powi(3) * 0.3;
        let crest = 1.0 - saddles * (1.0 - (0.5 + 0.5 * summits.at(wx, wz)).powf(1.5));
        let d = relief.at(wx, wz);
        let shape = carved * (0.3 + 0.7 * main) * crest * (1.0 + roughness * 0.35 * (d - 0.5) * u);
        out[0][k] = base + ctx.num_at("height", k) * near.values[0] as f32 * shape.max(0.0);
        out[1][k] = cross;
    }
}

// ----------------------------------------------------------------- peaks --

const PEAK_FIELDS: &[Field] = &[
    Field { name: "height", label: "Height", unit: Unit::Blocks, default: 240.0, min: 0.0, max: 4096.0, help: "The summit's height over Base, in blocks." },
    Field { name: "radius", label: "Radius", unit: Unit::Blocks, default: 700.0, min: 32.0, max: 32000.0, help: "How far its flanks reach, in blocks." },
];

fn default_peaks() -> Vec<Point> {
    vec![Point { x: 0.5, z: 0.5, values: vec![240.0, 700.0] }]
}

pub static PEAKS: NodeSpec = NodeSpec {
    id: "mountain.peaks",
    label: "Peaks",
    family: Family::Shape,
    summary: "Summits placed by hand, each with its height and reach: horns with ridges (arêtes) running down from the top.",
    help: "Place the peaks on the viewer; each has its own height and radius. Arêtes is how many main ridges run down from each summit (3 or 4 for a horn like the Matterhorn), Arête strength how deep the faces between them are cut. Where peaks meet, the higher one wins, with a saddle between. Add them to a range or a massif for the mountain everyone must recognise.",
    keywords: &["summit", "horn", "matterhorn", "point", "placed", "hand", "single", "mountain"],
    inputs: &[],
    outputs: &[height_out("The peaks over Base."), MASK_OUT],
    params: &[
        ParamSpec::points("peaks", "Peaks", PEAK_FIELDS, 64, "The summits, placed on the viewer, each with its height and radius."),
        BASE,
        profile(FULL),
        ParamSpec::int("aretes", "Arêtes", 4, 2, 8, "How many main ridges run down from each summit.").section(Section::Shape),
        ParamSpec::float("arete_strength", "Arête strength", Unit::Ratio, 0.7, 0.0, 1.0, "How deep the faces between the ridges are hollowed: 0 a round cone, 1 sharp ridges over deep cirques.").section(Section::Shape),
        sharpness(0.8),
        roughness(0.5),
        detail(120.0),
        WARP,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: peaks_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn peaks_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let size = ctx.space.size as f64;
    let given = p.points("peaks");
    let peaks = if given.is_empty() { default_peaks() } else { given.to_vec() };
    let wander = p.num("warp") * 0.12;
    let (x0, z0, x1, z1) = bounds(ctx);
    // The peaks reaching this tile.
    let near: Vec<(f64, f64, f32, f64, u32)> = peaks
        .iter()
        .enumerate()
        .map(|(i, pt)| (pt.x * size, pt.z * size, pt.value(0) as f32, pt.value(1), crate::util::mix32(seed ^ i as u32)))
        .filter(|&(px, pz, _, r, _)| {
            let reach = r * (1.0 + wander * 2.0);
            px + reach >= x0 && px - reach <= x1 && pz + reach >= z0 && pz - reach <= z1
        })
        .collect();
    if near.is_empty() {
        out[0].fill(base);
        out[1].fill(0.0);
        return;
    }
    let ridges = p.int("aretes").max(2) as usize;
    let strength = p.f("arete_strength");
    let relief = Relief::new(p.num("detail"), p.num("roughness"), p.num("sharpness"), finest(ctx), seed);
    let roughness = p.f("roughness");
    let warps: Vec<Warp> = near.iter().map(|&(_, _, _, r, s)| Warp::new(r * 0.6, wander * r, s)).collect();
    // Each peak's ridges (arêtes): their direction and length, turned and
    // spread at random.
    let arms: Vec<Vec<(f32, f32, f32)>> = near
        .iter()
        .map(|&(_, _, _, _, s)| {
            let turn = (s & 0xffff) as f32 / 65536.0 * std::f32::consts::TAU;
            (0..ridges)
                .map(|k| {
                    let r = |salt: u32| (crate::util::mix32(s ^ salt ^ (k as u32).wrapping_mul(0x2c1b_3c6d)) & 0xffff) as f32 / 65536.0;
                    let angle = turn + k as f32 * std::f32::consts::TAU / ridges as f32 + (r(1) - 0.5) * 0.6;
                    let (sin, cos) = angle.sin_cos();
                    (cos, sin, 0.95 + 0.3 * r(2))
                })
                .collect()
        })
        .collect();
    for (k, x, z) in samples(ctx) {
        let (mut best, mut mask) = (0.0f32, 0.0f32);
        for (n, &(px, pz, h, r, s)) in near.iter().enumerate() {
            let (wx, wz) = warps[n].at(x, z);
            let (dx, dz) = ((wx - px) as f32 / r as f32, (wz - pz) as f32 / r as f32);
            let dist = dx.hypot(dz);
            if dist >= 1.3 {
                continue;
            }
            // The summit: a cone, lower where the arêtes are strong, so that
            // the faces between them are hollowed into cirques.
            let mut v = p.profile("profile", (1.0 - dist).max(0.0) as f64) as f32 * (1.0 - 0.25 * strength);
            for &(cx, cz, len) in &arms[n] {
                // Along the ridge, and across it; ridges broaden downhill
                // and wander as they go.
                let t = dx * cx + dz * cz;
                if t < 0.0 || t >= len {
                    continue;
                }
                let across = (dx * cz - dz * cx) + 0.12 * t * noise1(t as f64 * 3.0, s) ;
                let width = 0.1 + 0.45 * t / len;
                let along = p.profile("profile", (1.0 - t / len) as f64) as f32;
                let ridge = along * (-(across / width).powi(2)).exp();
                v = v.max(ridge);
            }
            let cross = v.min(1.0);
            // The higher peak wins where they meet, with a saddle between.
            best = best.max(h * cross);
            mask = mask.max(cross);
        }
        if best <= 0.0 {
            out[0][k] = base;
            out[1][k] = 0.0;
            continue;
        }
        let d = relief.at(x, z);
        out[0][k] = base + best * (1.0 + roughness * 0.3 * (d - 0.5) * mask);
        out[1][k] = mask;
    }
}
