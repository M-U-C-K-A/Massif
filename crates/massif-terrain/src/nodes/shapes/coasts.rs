//! Coasts: islands, archipelagos, coastlines — land and sea around the
//! world's sea level.

use super::{Relief, bounds, finest, samples, smoothstep};
use crate::geom::Curve;
use crate::kernel::Ctx;
use crate::nodes::height_out;
use crate::noise::Fbm;
use crate::params::Point;
use crate::registry::{Field, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::util::mix32;

const LAND_OUT: PortSpec = PortSpec::new("land", "Land", PortKind::Mask, "1 on land, 0 at sea, soft across the shore.");
const LAND_PROFILE: &[[f64; 2]] = &[[0.0, 0.0], [0.35, 0.18], [0.75, 0.7], [1.0, 1.0]];

const fn height(default: f64, help: &'static str) -> ParamSpec {
    ParamSpec::float("height", "Height", Unit::Blocks, default, 0.0, 4096.0, help).soft(0.0, 600.0).drivable()
}

const SHELF: ParamSpec = ParamSpec::float("shelf", "Shelf", Unit::Blocks, 400.0, 0.0, 16000.0, "How far the shallow sea reaches from the shore, in blocks.").soft(0.0, 3000.0).section(Section::Shape);
const DEPTH: ParamSpec = ParamSpec::float("depth", "Depth", Unit::Blocks, 40.0, 0.0, 1024.0, "How deep the sea is at the shelf's edge, in blocks (it keeps sinking beyond).").soft(0.0, 200.0).section(Section::Shape);
const BEACH: ParamSpec = ParamSpec::float("beach", "Beach", Unit::Blocks, 40.0, 0.0, 2000.0, "How wide the low, flat shore is, in blocks.").soft(0.0, 300.0).section(Section::Shape);
const ROUGH: ParamSpec = ParamSpec::float("roughness", "Coast roughness", Unit::Ratio, 0.5, 0.0, 1.0, "How ragged the coastline is: capes and bays, small and large.").section(Section::Detail);
const HILLS: ParamSpec = ParamSpec::float("hills", "Hills", Unit::Ratio, 0.35, 0.0, 1.0, "How much rolling relief the land carries.").section(Section::Detail);

/// Height over the sea at `d` blocks from the shore (inland positive) of a
/// land rising to `height` over `rise` blocks; land share.
#[allow(clippy::too_many_arguments)]
fn shore(ctx: &Ctx, d: f64, height: f32, rise: f64, shelf: f64, depth: f32, beach: f64) -> (f32, f32) {
    let land = smoothstep(-6.0, 6.0, d as f32);
    if d >= 0.0 {
        let h = if d < beach {
            1.0 + 2.0 * (d / beach.max(1.0)) as f32
        } else {
            let t = ((d - beach) / (rise - beach).max(1.0)).clamp(0.0, 1.0);
            3.0 + (height - 3.0).max(0.0) * ctx.params.profile("profile", t) as f32
        };
        (h, land)
    } else {
        let s = -d;
        let sea = depth * smoothstep(0.0, shelf.max(1.0) as f32, s as f32) + depth * 1.5 * smoothstep(shelf as f32, (shelf * 4.0 + 1.0) as f32, s as f32);
        (-1.0 - sea, land)
    }
}

// ---------------------------------------------------------------- island --

pub static ISLAND: NodeSpec = NodeSpec {
    id: "coast.island",
    label: "Island",
    family: Family::Shape,
    summary: "An island rising from the sea: a ragged coast, a beach, a shallow shelf around it, land rising inland.",
    help: "Place its centre on the viewer. Radius is its size, Height how high its middle rises over the sea, Coast roughness how ragged the shore is. Shelf and Depth shape the sea around it. Put mountains on it with Max (or Add with their Base at 0) through its Land mask.",
    keywords: &["island", "isle", "sea", "coast", "beach", "atoll"],
    inputs: &[],
    outputs: &[height_out("The island and the sea floor around it."), LAND_OUT],
    params: &[
        ParamSpec::points("center", "Centre", &[], 1, "Where the island's middle is, placed on the viewer."),
        ParamSpec::float("radius", "Radius", Unit::Blocks, 1400.0, 16.0, 32000.0, "How far the island reaches from its middle, in blocks.").soft(100.0, 8000.0),
        height(70.0, "How high the island's middle rises over the sea, in blocks."),
        ParamSpec::float("elongation", "Elongation", Unit::Ratio, 0.3, 0.0, 0.9, "How much longer than wide the island is.").section(Section::Shape),
        ParamSpec::float("angle", "Angle", Unit::Degrees, 30.0, -180.0, 180.0, "Which way its length runs, in degrees from east.").section(Section::Shape),
        ParamSpec::profile("profile", "Profile", LAND_PROFILE, "The shape of the land from the shore (left) to the middle (right).").section(Section::Shape),
        BEACH,
        SHELF,
        DEPTH,
        ROUGH,
        ParamSpec::float("outline_scale", "Outline scale", Unit::Blocks, 600.0, 16.0, 16000.0, "The size of the largest capes and bays, in blocks.").soft(50.0, 4000.0).section(Section::Detail),
        HILLS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: island_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// How far inside an island's shore a point lies (negative outside): an
/// ellipse `radius` long, `elongation` narrower across, turned by `angle`,
/// its shore ragged by `outline` (about ±1) times `wobble` blocks.
fn inside_isle(dx: f64, dz: f64, radius: f64, elongation: f64, angle: f64, outline: f64, wobble: f64) -> f64 {
    isle_base(dx, dz, radius, elongation, angle.sin_cos()) + wobble * outline
}

/// [`inside_isle`] but its outline: the island's ellipse alone, its angle's
/// sine and cosine given.
#[inline]
fn isle_base(dx: f64, dz: f64, radius: f64, elongation: f64, (s, c): (f64, f64)) -> f64 {
    let (u, v) = (c * dx + s * dz, -s * dx + c * dz);
    let dist = u.hypot(v / (1.0 - 0.6 * elongation).max(0.1));
    radius - dist
}

/// The most an outline's noise ([`Fbm::at`]) reaches either way: its
/// octaves are gradient noise of unit gradients, within ±1 (√2 · √2 / 2);
/// twice that, so that rounding never matters.
const OUTLINE_MOST: f64 = 2.0;

fn island_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let sea = ctx.space.sea_level;
    let (cx, cz) = p.points("center").first().map_or((0.5 * size, 0.5 * size), |c| (c.x * size, c.z * size));
    let radius = p.num("radius");
    let outline = Fbm::down_to(p.num("outline_scale"), finest(ctx).max(8.0), 0.55, ctx.seed);
    let wobble = p.num("roughness") * radius * 0.4;
    let (elongation, angle) = (p.num("elongation"), p.num("angle").to_radians());
    let hills = Relief::new(radius * 0.3, 0.5, 0.2, finest(ctx), ctx.seed ^ 0x77);
    let hill = p.f("hills");
    let (shelf, depth, beach) = (p.num("shelf"), p.f("depth"), p.num("beach"));
    for (k, x, z) in samples(ctx) {
        let d = inside_isle(x - cx, z - cz, radius, elongation, angle, outline.at(x, z) as f64, wobble);
        let (h, land) = shore(ctx, d, ctx.num_at("height", k), radius, shelf, depth, beach);
        let relief = if d > beach { 1.0 + hill * (hills.at(x, z) - 0.5) } else { 1.0 };
        out[0][k] = sea + if h > 0.0 { h * relief } else { h };
        out[1][k] = land;
    }
}

// ----------------------------------------------------------- archipelago --

const ISLE_FIELDS: &[Field] = &[
    Field { name: "radius", label: "Radius", unit: Unit::Blocks, default: 500.0, min: 16.0, max: 16000.0, help: "How far this island reaches, in blocks." },
    Field { name: "height", label: "Height", unit: Unit::Blocks, default: 50.0, min: 0.0, max: 2048.0, help: "How high this island rises over the sea, in blocks." },
];

pub static ARCHIPELAGO: NodeSpec = NodeSpec {
    id: "coast.archipelago",
    label: "Archipelago",
    family: Family::Shape,
    summary: "A scatter of islands in the sea, many small and a few large, over the terrain or along a line.",
    help: "Count islands are scattered over the terrain, or along the Chain when one is drawn (an arc of islands), their sizes between Smallest and Largest (most of them small). Place islands by hand as Islands to set each one yourself; they come in addition. Shelf and Depth shape the sea around them, so that close islands share their shallows.",
    keywords: &["islands", "isles", "chain", "arc", "sea", "atolls", "scatter"],
    inputs: &[],
    outputs: &[height_out("The islands and the sea floor around them."), LAND_OUT],
    params: &[
        ParamSpec::int("count", "Count", 14, 0, 400, "How many islands are scattered."),
        ParamSpec::float("smallest", "Smallest", Unit::Blocks, 70.0, 16.0, 16000.0, "The radius of the smallest islands, in blocks.").soft(32.0, 2000.0),
        ParamSpec::float("largest", "Largest", Unit::Blocks, 700.0, 16.0, 32000.0, "The radius of the largest islands, in blocks.").soft(100.0, 6000.0),
        height(60.0, "How high the largest islands rise over the sea, in blocks (smaller ones less)."),
        ParamSpec::curve("chain", "Chain", &[], false, "A line the islands gather along (an arc of islands); over the whole terrain when not drawn.").section(Section::Shape),
        ParamSpec::float("spread", "Spread", Unit::Blocks, 1500.0, 0.0, 32000.0, "How far islands stray from the Chain, in blocks.").soft(0.0, 6000.0).section(Section::Shape),
        ParamSpec::points("islands", "Islands", ISLE_FIELDS, 128, "Islands placed by hand, in addition to the scattered ones.").section(Section::Shape),
        ParamSpec::profile("profile", "Profile", LAND_PROFILE, "The shape of the land from the shore (left) to the middle (right).").section(Section::Shape),
        BEACH,
        SHELF,
        DEPTH,
        ROUGH,
        HILLS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: archipelago_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// An island of an archipelago.
struct Isle {
    x: f64,
    z: f64,
    radius: f64,
    height: f32,
    elongation: f64,
    angle: f64,
    seed: u32,
}

/// The islands of an archipelago.
fn isles(ctx: &Ctx) -> Vec<Isle> {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let rnd = |i: u32, k: u32| mix32(ctx.seed ^ mix32(i.wrapping_mul(0x9e37_79b9) ^ k)) as f64 / u32::MAX as f64;
    let (small, large) = (p.num("smallest"), p.num("largest").max(p.num("smallest")));
    let top = p.f("height");
    let chain: Vec<Point> = p.points("chain").to_vec();
    let curve = (chain.len() >= 2).then(|| Curve::new(&chain, size, false, 8.0)).flatten();
    let spread = p.num("spread");
    let mut out = Vec::new();
    for i in 0..p.int("count").max(0) as u32 {
        let t = rnd(i, 1).powf(2.6);
        let r = small + (large - small) * t;
        let h = top * (0.35 + 0.65 * t as f32);
        let (x, z) = match &curve {
            Some(c) => {
                // Along the chain, strayed from it.
                let at = rnd(i, 2) * c.length();
                let (px, pz, dir) = c.point_at(at);
                let off = (rnd(i, 3) * 2.0 - 1.0) * spread;
                (px - dir[1] * off, pz + dir[0] * off)
            }
            None => (size * (0.06 + 0.88 * rnd(i, 2)), size * (0.06 + 0.88 * rnd(i, 3))),
        };
        out.push(Isle { x, z, radius: r, height: h, elongation: 0.6 * rnd(i, 4), angle: rnd(i, 5) * std::f64::consts::TAU, seed: mix32(ctx.seed ^ i) });
    }
    for (j, pt) in p.points("islands").iter().enumerate() {
        let i = 10_000 + j as u32;
        out.push(Isle { x: pt.x * size, z: pt.z * size, radius: pt.value(0), height: pt.value(1) as f32, elongation: 0.4 * rnd(i, 4), angle: rnd(i, 5) * std::f64::consts::TAU, seed: mix32(ctx.seed ^ i) });
    }
    out
}

fn archipelago_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let sea = ctx.space.sea_level;
    let (shelf, depth, beach) = (p.num("shelf"), p.f("depth"), p.num("beach"));
    let rough = p.num("roughness");
    let (x0, z0, x1, z1) = bounds(ctx);
    let reach = shelf * 4.0;
    let all = isles(ctx);
    let near: Vec<&Isle> = all.iter().filter(|i| i.x + i.radius * 1.5 + reach >= x0 && i.x - i.radius * 1.5 - reach <= x1 && i.z + i.radius * 1.5 + reach >= z0 && i.z - i.radius * 1.5 - reach <= z1).collect();
    // Each island's shore ragged in proportion to its size.
    let outlines: Vec<Fbm> = near.iter().map(|i| Fbm::down_to(i.radius * 0.6, finest(ctx).max(6.0), 0.55, i.seed)).collect();
    let hills = Relief::new(p.num("largest") * 0.3, 0.5, 0.2, finest(ctx), ctx.seed ^ 0x77);
    let hill = p.f("hills");
    let turns: Vec<(f64, f64)> = near.iter().map(|i| i.angle.sin_cos()).collect();
    // At sea an island makes the floor deeper the farther from it (Depth
    // not below 0): an island whose nearest it could be, whatever its
    // outline, makes no more than what is here already cannot change the
    // highest, and its outline's noise, most of the work, is not made.
    let monotone = depth >= 0.0;
    let mut order: Vec<(f64, f64, usize)> = Vec::with_capacity(near.len());
    for (k, x, z) in samples(ctx) {
        // The highest of what each island makes here: land wins over sea,
        // the shallower sea over the deeper, with no seam where they meet.
        // The nearest first, so that the far ones are seen to matter not.
        let (mut h, mut land) = (-1.0 - depth * 2.5, 0.0f32);
        order.clear();
        for (i, isle) in near.iter().enumerate() {
            let base = isle_base(x - isle.x, z - isle.z, isle.radius, isle.elongation, turns[i]);
            let most = base + (rough * isle.radius * 0.45).abs() * OUTLINE_MOST;
            // Out of reach whatever its outline.
            if most < -reach {
                continue;
            }
            order.push((most, base, i));
        }
        if monotone {
            order.sort_unstable_by(|a, b| b.0.total_cmp(&a.0));
        }
        for &(most, base, i) in &order {
            let isle = near[i];
            // A block more than the most it could reach, for rounding.
            if monotone && most + 1.0 < 0.0 {
                let (hb, lb) = shore(ctx, most + 1.0, isle.height, isle.radius, shelf, depth, beach);
                if hb <= h && lb <= land {
                    continue;
                }
            }
            let d = base + rough * isle.radius * 0.45 * outlines[i].at(x, z) as f64;
            if d < -reach {
                continue;
            }
            let (hi, li) = shore(ctx, d, isle.height, isle.radius, shelf, depth, beach);
            h = h.max(hi);
            land = land.max(li);
        }
        let relief = if h > 3.0 { 1.0 + hill * (hills.at(x, z) - 0.5) } else { 1.0 };
        out[0][k] = sea + if h > 0.0 { h * relief } else { h };
        out[1][k] = land;
    }
}

// ------------------------------------------------------------- coastline --

pub static COASTLINE: NodeSpec = NodeSpec {
    id: "coast.coastline",
    label: "Coastline",
    family: Family::Shape,
    summary: "Land and sea across the terrain: continents and seas, a ragged coast with capes and bays, beaches, a shelf, fjords if wanted.",
    help: "Land is the share of the terrain above the sea, Scale the size of the landmasses. Draw an Outline to say where the coast runs (a closed line: land inside), or connect a mask to Shape (an imported or painted land map); otherwise noise decides. Inland is how far the land takes to rise to Height. Fjords cut long, deep inlets into the coast. The Land output is the mask to put mountains, forests and beaches through.",
    keywords: &["continent", "coast", "shore", "sea", "ocean", "land", "fjord", "bay", "cape", "mainland"],
    inputs: &[PortSpec::new("shape", "Shape", PortKind::Mask, "Where the land is, from 0 to 1 (noise or the Outline decide, when unconnected).").optional()],
    outputs: &[height_out("The land and the sea floor."), LAND_OUT],
    params: &[
        ParamSpec::float("land", "Land", Unit::Ratio, 0.5, 0.0, 1.0, "The share of the terrain that is land, when noise decides."),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 5000.0, 64.0, 65536.0, "The size of the landmasses, in blocks.").soft(500.0, 20000.0),
        height(80.0, "How high the land rises inland, in blocks over the sea."),
        ParamSpec::float("inland", "Inland", Unit::Blocks, 1500.0, 16.0, 32000.0, "How far from the shore the land reaches its height, in blocks.").soft(100.0, 8000.0),
        ParamSpec::curve("outline", "Outline", &[], true, "The coast, drawn on the viewer as a closed line: land inside.").section(Section::Shape),
        ParamSpec::profile("profile", "Profile", LAND_PROFILE, "The shape of the land from the shore (left) to Inland (right).").section(Section::Shape),
        BEACH,
        SHELF,
        DEPTH,
        ParamSpec::float("fjords", "Fjords", Unit::Ratio, 0.0, 0.0, 1.0, "How many long, deep inlets are cut into the coast.").section(Section::Shape),
        ParamSpec::float("fjord_size", "Fjord size", Unit::Blocks, 350.0, 16.0, 8000.0, "How wide apart the fjords run, in blocks.").soft(50.0, 2000.0).section(Section::Shape),
        ROUGH,
        HILLS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: coastline_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// The value a standard normal variable stays under with probability `p`
/// (Acklam's rational approximation, good to 1e-9).
pub(super) fn normal_quantile(p: f64) -> f64 {
    const A: [f64; 6] = [-3.969_683_028_665_376e1, 2.209_460_984_245_205e2, -2.759_285_104_469_687e2, 1.383_577_518_672_69e2, -3.066_479_806_614_716e1, 2.506_628_277_459_239];
    const B: [f64; 5] = [-5.447_609_879_822_406e1, 1.615_858_368_580_409e2, -1.556_989_798_598_866e2, 6.680_131_188_771_972e1, -1.328_068_155_288_572e1];
    const C: [f64; 6] = [-7.784_894_002_430_293e-3, -3.223_964_580_411_365e-1, -2.400_758_277_161_838, -2.549_732_539_343_734, 4.374_664_141_464_968, 2.938_163_982_698_783];
    const D: [f64; 4] = [7.784_695_709_041_462e-3, 3.224_671_290_700_398e-1, 2.445_134_137_142_996, 3.754_408_661_907_416];
    let p = p.clamp(1e-12, 1.0 - 1e-12);
    let tail = |q: f64| (((((C[0] * q + C[1]) * q + C[2]) * q + C[3]) * q + C[4]) * q + C[5]) / ((((D[0] * q + D[1]) * q + D[2]) * q + D[3]) * q + 1.0);
    if p < 0.024_25 {
        tail((-2.0 * p.ln()).sqrt())
    } else if p > 1.0 - 0.024_25 {
        -tail((-2.0 * (1.0 - p).ln()).sqrt())
    } else {
        let q = p - 0.5;
        let r = q * q;
        (((((A[0] * r + A[1]) * r + A[2]) * r + A[3]) * r + A[4]) * r + A[5]) * q / (((((B[0] * r + B[1]) * r + B[2]) * r + B[3]) * r + B[4]) * r + 1.0)
    }
}

fn coastline_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let size = ctx.space.size as f64;
    let sea = ctx.space.sea_level;
    let scale = p.num("scale");
    let rough = p.num("roughness");
    let inland = p.num("inland");
    let (shelf, depth, beach) = (p.num("shelf"), p.f("depth"), p.num("beach"));
    let outline: Vec<Point> = p.points("outline").to_vec();
    let curve = (outline.len() >= 3).then(|| Curve::new(&outline, size, true, (scale / 200.0).clamp(1.0, 32.0))).flatten();
    let masses = Fbm::new(scale, 3, 0.5, ctx.seed);
    let ragged = Fbm::down_to(scale * 0.35, finest(ctx).max(6.0), 0.55, ctx.seed ^ 0x2f);
    // fbm of three octaves spreads like a normal of deviation ~0.26 around 0.
    let threshold = 0.26 * normal_quantile(1.0 - p.num("land"));
    let fjords = p.f("fjords");
    let fjord = Fbm::new(p.num("fjord_size") * 3.0, 2, 0.4, ctx.seed ^ 0xf1);
    let hills = Relief::new(inland * 0.4, 0.5, 0.25, finest(ctx), ctx.seed ^ 0x77);
    let hill = p.f("hills");
    let (x0, z0, x1, z1) = bounds(ctx);
    let segs = curve.as_ref().map(|c| c.segments_near(x0, z0, x1, z1, inland + shelf * 4.0 + scale));
    for (k, x, z) in samples(ctx) {
        let detail = ragged.at(x, z) as f64;
        let d = match (inputs[0], &curve) {
            (Some(shape), _) => (shape[k] as f64 - 0.5) * 2.0 * inland + rough * scale * 0.06 * detail,
            (None, Some(c)) => {
                let reach = inland + shelf * 4.0 + scale;
                let dist = c.nearest(segs.as_deref().unwrap_or(&[]), x, z, reach).map_or(reach, |n| n.dist);
                let signed = if c.inside(x, z) { dist } else { -dist };
                signed + rough * scale * 0.06 * detail
            }
            (None, None) => {
                let v = masses.at(x, z) as f64 + rough * 0.35 * detail;
                (v - threshold) * scale * 0.9
            }
        };
        let (mut h, mut land) = shore(ctx, d, ctx.num_at("height", k), inland, shelf, depth, beach);
        if h > 0.0 && d > beach {
            h *= 1.0 + hill * (hills.at(x, z) - 0.5);
        }
        if fjords > 0.0 && d > -shelf {
            // Narrow channels running inland from the coast, deeper seaward.
            // Thin lines a few times Fjord size apart, reaching inland.
            let line = (1.0 - fjord.at(x, z).abs()).powf(60.0);
            let reach = (1.0 - (d / (p.num("fjord_size") * 5.0 * fjords as f64)).clamp(0.0, 1.0)) as f32;
            let cut = fjords.sqrt() * line * reach;
            if cut > 0.0 {
                let floor = -1.0 - depth * 0.8;
                h = h + (floor - h) * smoothstep(0.15, 0.6, cut);
                land *= 1.0 - smoothstep(0.3, 0.45, cut);
            }
        }
        out[0][k] = sea + h;
        out[1][k] = land;
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn quantiles() {
        assert!(super::normal_quantile(0.5).abs() < 1e-9);
        assert!((super::normal_quantile(0.975) - 1.959_964).abs() < 1e-5);
        assert!((super::normal_quantile(0.01) + 2.326_348).abs() < 1e-5);
    }
}
