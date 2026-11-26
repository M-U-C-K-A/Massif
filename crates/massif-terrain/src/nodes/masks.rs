//! Masks: where something is, from 0 to 1 — by height, slope, curvature,
//! facing, light, openness, snow, wetness, distance — and their arithmetic.

use crate::filters::{self, gradient, work_lod};
use crate::kernel::{Ctx, Footprint, Grid, Uniforms};
use crate::noise::Fbm;
use crate::params::Params;
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::space::Space;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&HEIGHT, &SLOPE, &CURVATURE, &FACING, &EXPOSURE, &OCCLUSION, &SNOW, &WETNESS, &DISTANCE, &THRESHOLD, &INVERT, &EXPAND];

const RELIEF: PortSpec = PortSpec::new("height", "Height", PortKind::Height, "The relief to measure.");
const MASK_IN: PortSpec = PortSpec::new("mask", "Mask", PortKind::Mask, "The mask to work on.");

const fn mask_out(help: &'static str) -> PortSpec {
    PortSpec::new("mask", "Mask", PortKind::Mask, help)
}

pub(crate) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    if (b - a).abs() < 1e-6 {
        return if x >= a { 1.0 } else { 0.0 };
    }
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Each sample of the kernel's tile, with its index and its absolute
/// sample coordinates.
fn tile<'a>(ctx: &'a Ctx<'a>) -> impl Iterator<Item = (usize, i32, i32)> + 'a {
    let w = ctx.rect.w as usize;
    (0..ctx.rect.area()).map(move |k| (k, ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32))
}

/// The slope in degrees of `g` at `(x, z)`.
fn slope_deg(g: &Grid, x: i32, z: i32) -> f32 {
    let (dx, dz) = gradient(g, x, z);
    (dx * dx + dz * dz).sqrt().atan().to_degrees()
}

// ---------------------------------------------------------------- height --

pub static HEIGHT: NodeSpec = NodeSpec {
    id: "mask.height",
    label: "Height",
    family: Family::Mask,
    summary: "Where the ground lies between two heights: 1 from From to To, fading over Softness on either side.",
    help: "Picks bands of altitude: the snowy summits (From high, To at the top), the low ground by the sea, a band of forest. Heights are in blocks, as the relief's; Softness fades the edges so that what is painted through it mixes.",
    keywords: &["altitude", "elevation", "band", "select", "range"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 between From and To.")],
    params: &[
        ParamSpec::float("from", "From", Unit::Blocks, 90.0, -2048.0, 4096.0, "The bottom of the band, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("to", "To", Unit::Blocks, 200.0, -2048.0, 4096.0, "The top of the band, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("softness", "Softness", Unit::Blocks, 10.0, 0.0, 1024.0, "How far the edges fade, in blocks.").soft(0.0, 60.0),
    ],
    kernel: Kernel::Point { cpu: height_cpu, gpu: Some(&HEIGHT_GPU) },
    ..NodeSpec::BASE
};

fn band(p: &Params, h: f32) -> f32 {
    let (from, to, soft) = (p.f("from"), p.f("to"), p.f("softness").max(1e-3) * 0.5);
    smoothstep(from - soft, from + soft, h) * (1.0 - smoothstep(to - soft, to + soft, h))
}

fn height_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("the band has its input");
    for (o, &v) in out[0].iter_mut().zip(h) {
        *o = band(ctx.params, v);
    }
}

static HEIGHT_GPU: crate::kernel::GpuKernel = crate::kernel::GpuKernel {
    name: "mask.height",
    source: include_str!("wgsl/height_band.wgsl"),
    uniforms: |ctx, _| {
        let p = ctx.params;
        let soft = p.f("softness").max(1e-3) * 0.5;
        Uniforms::default().f32(p.f("from")).f32(p.f("to")).f32(soft).finish()
    },
};

// ----------------------------------------------------------------- slope --

pub static SLOPE: NodeSpec = NodeSpec {
    id: "mask.slope",
    label: "Slope",
    family: Family::Mask,
    summary: "Where the ground is steep: 0 below From degrees, 1 above To, a smooth step between (and back to 0 above Upper, when set).",
    help: "Picks slopes by their steepness: cliffs for rock, gentle ground for grass and snow (inverted). Upper keeps only a band of slopes (scree between 30° and 40°, say). The slope is measured between neighbouring samples, so a coarse preview shows slopes a little gentler than the final build.",
    keywords: &["steep", "cliff", "gradient", "angle"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 where it is steep.")],
    params: &[
        ParamSpec::float("from", "From", Unit::Degrees, 25.0, 0.0, 90.0, "Slopes gentler than this are 0."),
        ParamSpec::float("to", "To", Unit::Degrees, 45.0, 0.0, 90.0, "Slopes steeper than this are 1."),
        ParamSpec::float("upper", "Upper", Unit::Degrees, 90.0, 0.0, 90.0, "Slopes steeper than this are 0 again (90: none).").section(Section::Advanced),
    ],
    kernel: Kernel::Local { cpu: slope_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: 1, map: None } },
    ..NodeSpec::BASE
};

fn slope_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("slope has its input");
    let p = ctx.params;
    let (from, to, upper) = (p.f("from"), p.f("to"), p.f("upper"));
    let width = (to - from).abs().max(1.0);
    for (k, x, z) in tile(ctx) {
        let deg = slope_deg(h, x, z);
        let rise = smoothstep(from, to, deg);
        out[0][k] = if upper < 90.0 { rise * (1.0 - smoothstep(upper, upper + width, deg)) } else { rise };
    }
}

// ------------------------------------------------------------- curvature --

const CURVATURE_MODES: &[Choice] = &[
    Choice { value: "convex", label: "Ridges", help: "Where the ground bulges: crests, ridges, the edges of cliffs." },
    Choice { value: "concave", label: "Hollows", help: "Where the ground hollows: valleys, gullies, the feet of slopes." },
    Choice { value: "both", label: "Both", help: "0.5 on even ground, toward 1 in hollows and toward 0 on ridges." },
];

pub static CURVATURE: NodeSpec = NodeSpec {
    id: "mask.curvature",
    label: "Curvature",
    family: Family::Mask,
    summary: "Where the ground bulges into ridges or hollows into valleys, at the scale asked for.",
    help: "Compares the relief with its own smoothing over Scale blocks: higher than its surroundings is a ridge, lower a hollow. A small scale finds gullies and crests, a large one valleys and spurs. Sensitivity says how slight a bulge or hollow reaches 1. Ridges take rock and wind, hollows soil, water and snow drifts.",
    keywords: &["convex", "concave", "ridges", "hollows", "valleys", "crests", "gullies"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 on ridges or in hollows, as the mode says.")],
    params: &[
        ParamSpec::choice("mode", "Mode", CURVATURE_MODES, "convex", "Ridges, hollows, or both."),
        ParamSpec::float("scale", "Scale", Unit::Blocks, 32.0, 2.0, 8192.0, "The size of the ridges and hollows looked for, in blocks.").soft(4.0, 512.0),
        ParamSpec::float("sensitivity", "Sensitivity", Unit::Ratio, 0.5, 0.0, 1.0, "How slight a bulge or a hollow is enough to reach 1."),
    ],
    kernel: Kernel::Local { cpu: curvature_cpu, gpu: None, footprint: curvature_footprint },
    ..NodeSpec::BASE
};

/// The Gaussian's deviation in blocks for a scale.
fn sigma_of(scale: f64) -> f64 {
    scale * 0.5
}

fn curvature_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    let s = sigma_of(p.num("scale"));
    let work = work_lod(lod, s, 6.0);
    let reach = (3.0 * s / Space::spacing(work)).ceil() as u32;
    Footprint { lod: work, halo: reach + 2, map: None }
}

/// How far below its smoothing each sample lies (hollows positive), at
/// the level `src` holds.
fn hollowness(src: &Grid, sigma_blocks: f64) -> Grid {
    let blurred = filters::blur(src, sigma_blocks / Space::spacing(src.lod));
    let rect = blurred.rect;
    let mut data = Vec::with_capacity(rect.area());
    for z in rect.z..rect.z1() {
        for x in rect.x..rect.x1() {
            data.push(blurred.at(x, z) - src.at(x, z));
        }
    }
    Grid::new(src.lod, rect, data)
}

fn curvature_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("curvature has its input");
    let p = ctx.params;
    let scale = p.num("scale");
    let diff = hollowness(src, sigma_of(scale));
    filters::resample(ctx, &diff, &mut out[0]);
    let full = (scale * 0.1 * (1.0 - p.num("sensitivity")) + 0.5) as f32;
    let mode = p.choice("mode");
    for v in out[0].iter_mut() {
        *v = match mode {
            "concave" => smoothstep(0.0, full, *v),
            "both" => 0.5 + 0.5 * (*v / full).clamp(-1.0, 1.0),
            _ => smoothstep(0.0, full, -*v),
        };
    }
}

// ---------------------------------------------------------------- facing --

pub static FACING: NodeSpec = NodeSpec {
    id: "mask.facing",
    label: "Facing",
    family: Family::Mask,
    summary: "Where slopes face a direction: 1 on slopes turned toward it, fading within Spread degrees, 0 on flat ground.",
    help: "Direction is where the slopes look to (0 east, 90 south, 180 west, 270 north). North-facing slopes keep snow and moss in the northern hemisphere, south-facing ones are dry and sunny; slopes facing the prevailing wind take its rain. Ground flatter than Min slope counts as facing nowhere.",
    keywords: &["aspect", "orientation", "north", "south", "sunny", "shady", "exposure"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 on slopes facing Direction.")],
    params: &[
        ParamSpec::float("direction", "Direction", Unit::Degrees, 270.0, -360.0, 360.0, "Where the slopes look to, in degrees (0 east, 90 south, 270 north)."),
        ParamSpec::float("spread", "Spread", Unit::Degrees, 90.0, 1.0, 180.0, "How far from Direction a slope still counts, in degrees."),
        ParamSpec::float("min_slope", "Min slope", Unit::Degrees, 5.0, 0.0, 45.0, "Ground flatter than this faces nowhere.").section(Section::Advanced),
    ],
    kernel: Kernel::Local { cpu: facing_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: 1, map: None } },
    ..NodeSpec::BASE
};

/// The angular distance between two angles, in degrees (0 to 180).
fn angle_between(a: f32, b: f32) -> f32 {
    let d = (a - b).rem_euclid(360.0);
    d.min(360.0 - d)
}

fn facing_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("facing has its input");
    let p = ctx.params;
    let (dir, spread, min) = (p.f("direction"), p.f("spread"), p.f("min_slope"));
    for (k, x, z) in tile(ctx) {
        let (dx, dz) = gradient(h, x, z);
        let slope = (dx * dx + dz * dz).sqrt().atan().to_degrees();
        // The way the slope looks: downhill.
        let aspect = (-dz).atan2(-dx).to_degrees();
        let towards = 1.0 - smoothstep(0.0, spread, angle_between(aspect, dir));
        out[0][k] = towards * smoothstep(min * 0.5, min.max(0.01), slope);
    }
}

// -------------------------------------------------------------- exposure --

pub static EXPOSURE: NodeSpec = NodeSpec {
    id: "mask.exposure",
    label: "Sunlight",
    family: Family::Mask,
    summary: "How much sunlight the ground takes from a sun at a given direction and height: 1 facing it, 0 turned away.",
    help: "Azimuth is where the sun stands (0 east, 90 south, 180 west), Elevation how high above the horizon. Flat ground takes the sine of the elevation. Sunlit slopes dry out and lose their snow first; shaded ones keep moss, ice and forests that like the cold.",
    keywords: &["sun", "light", "shade", "insolation", "lambert", "exposure"],
    inputs: &[RELIEF],
    outputs: &[mask_out("The share of full sunlight.")],
    params: &[
        ParamSpec::float("azimuth", "Azimuth", Unit::Degrees, 90.0, -360.0, 360.0, "Where the sun stands, in degrees (0 east, 90 south)."),
        ParamSpec::float("elevation", "Elevation", Unit::Degrees, 35.0, 0.0, 90.0, "How high the sun stands above the horizon, in degrees."),
    ],
    kernel: Kernel::Local { cpu: exposure_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: 1, map: None } },
    ..NodeSpec::BASE
};

fn exposure_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("sunlight has its input");
    let p = ctx.params;
    let (az, el) = (p.f("azimuth").to_radians(), p.f("elevation").to_radians());
    let sun = [el.cos() * az.cos(), el.sin(), el.cos() * az.sin()];
    for (k, x, z) in tile(ctx) {
        let (dx, dz) = gradient(h, x, z);
        let l = (dx * dx + 1.0 + dz * dz).sqrt();
        out[0][k] = ((-dx * sun[0] + sun[1] - dz * sun[2]) / l).max(0.0);
    }
}

// ------------------------------------------------------------- occlusion --

pub static OCCLUSION: NodeSpec = NodeSpec {
    id: "mask.occlusion",
    label: "Openness",
    family: Family::Mask,
    summary: "How open to the sky the ground is: 1 on summits and plains, lower in gorges, gullies and at the feet of cliffs.",
    help: "Looks around each place in Directions directions up to Radius blocks, and takes how high the horizon rises. Narrow valleys, crevices and the feet of walls are dark; the dark is where moss, shade plants and old snow keep. Wide radii are worked at a coarser level.",
    keywords: &["ambient occlusion", "sky", "shade", "crevices", "gorges", "openness"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 open to the sky, lower where the horizon rises.")],
    params: &[
        ParamSpec::float("radius", "Radius", Unit::Blocks, 64.0, 2.0, 4096.0, "How far around each place the horizon is looked for, in blocks.").soft(4.0, 512.0),
        ParamSpec::int("directions", "Directions", 8, 4, 32, "How many directions are looked in.").section(Section::Advanced),
    ],
    kernel: Kernel::Local { cpu: occlusion_cpu, gpu: None, footprint: occlusion_footprint },
    ..NodeSpec::BASE
};

fn occlusion_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    let r = p.num("radius");
    let work = work_lod(lod, r, 24.0);
    Footprint { lod: work, halo: (r / Space::spacing(work)).ceil() as u32 + 3, map: None }
}

fn occlusion_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("openness has its input");
    let p = ctx.params;
    let spacing = Space::spacing(src.lod) as f32;
    let steps = (p.num("radius") / Space::spacing(src.lod)).ceil().max(1.0) as i32;
    let dirs = p.int("directions").max(1) as usize;
    let rays: Vec<(f32, f32)> = (0..dirs).map(|d| (d as f32 * std::f32::consts::TAU / dirs as f32).sin_cos()).map(|(s, c)| (c, s)).collect();
    // Worked over the samples the tile covers at the working level, then
    // resampled.
    let rect = ctx.rect.at_lod(ctx.lod, src.lod).expand(2);
    let mut data = Vec::with_capacity(rect.area());
    for z in rect.z..rect.z1() {
        for x in rect.x..rect.x1() {
            let h0 = src.at(x, z);
            let mut open = 0.0f32;
            for &(cx, cz) in &rays {
                let mut horizon = 0.0f32;
                for s in 1..=steps {
                    let (u, v) = (x as f64 + (cx * s as f32) as f64, z as f64 + (cz * s as f32) as f64);
                    let rise = (src.bilinear(u, v) - h0) / (s as f32 * spacing);
                    horizon = horizon.max(rise);
                }
                // The share of the sky's half above the horizon.
                open += 1.0 - horizon.atan() / std::f32::consts::FRAC_PI_2;
            }
            data.push(open / dirs as f32);
        }
    }
    filters::resample(ctx, &Grid::new(src.lod, rect, data), &mut out[0]);
}

// ------------------------------------------------------------------ snow --

pub static SNOW: NodeSpec = NodeSpec {
    id: "mask.snow",
    label: "Snow",
    family: Family::Mask,
    summary: "Where snow lies: above a snow line, lower on slopes facing north and in hollows where it drifts, never on slopes too steep to hold it.",
    help: "Line is the height above which snow lies on open ground, Softness how gradually it fades in. North bias lowers the line on north-facing slopes (the shady side), Drift fills hollows, Breakup frays the edge into patches. Max slope keeps cliffs bare. Depth, when above 0, also raises the Height output by the snow's depth.",
    keywords: &["snow", "snowline", "ice", "winter", "glacier", "drift", "cover"],
    inputs: &[RELIEF],
    outputs: &[mask_out("1 under snow."), PortSpec::new("height", "Height", PortKind::Height, "The relief with the snow's depth on it.")],
    params: &[
        ParamSpec::float("line", "Line", Unit::Blocks, 180.0, -2048.0, 4096.0, "The height above which snow lies on open ground, in blocks.").soft(-64.0, 320.0).drivable(),
        ParamSpec::float("softness", "Softness", Unit::Blocks, 20.0, 0.0, 512.0, "How gradually the snow fades in around the line, in blocks.").soft(0.0, 100.0),
        ParamSpec::float("max_slope", "Max slope", Unit::Degrees, 40.0, 0.0, 90.0, "Slopes steeper than this stay bare."),
        ParamSpec::float("north_bias", "North bias", Unit::Blocks, 20.0, 0.0, 512.0, "How much lower the line comes on north-facing slopes, in blocks.").soft(0.0, 100.0).section(Section::Shape),
        ParamSpec::float("drift", "Drift", Unit::Ratio, 0.5, 0.0, 1.0, "How much snow gathers in hollows.").section(Section::Shape),
        ParamSpec::float("breakup", "Breakup", Unit::Ratio, 0.3, 0.0, 1.0, "How much the snow's edge frays into patches.").section(Section::Detail),
        ParamSpec::float("depth", "Depth", Unit::Blocks, 0.0, 0.0, 64.0, "How deep the snow is on the Height output, in blocks.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Local { cpu: snow_cpu, gpu: None, footprint: |_: &Params, lod, _| Footprint { lod, halo: (24.0 / Space::spacing(lod)).ceil() as u32 + 1, map: None } },
    ..NodeSpec::BASE
};

fn snow_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("snow has its input");
    let p = ctx.params;
    let hollow = hollowness(h, 8.0);
    let soft = p.f("softness").max(1e-3) * 0.5;
    let (max_slope, bias, drift, breakup, depth) = (p.f("max_slope"), p.f("north_bias"), p.f("drift"), p.f("breakup"), p.f("depth"));
    let fray = Fbm::new(48.0, 3, 0.5, ctx.seed);
    for (k, x, z) in tile(ctx) {
        let v = h.at(x, z);
        let (dx, dz) = gradient(h, x, z);
        let slope = (dx * dx + dz * dz).sqrt().atan().to_degrees();
        // North-facing: downhill toward −z.
        let len = (dx * dx + dz * dz).sqrt().max(1e-6);
        let north = (dz / len) * smoothstep(2.0, 10.0, slope);
        let (wx, wz) = ctx.world(x, z);
        let effective = v + bias * north.max(0.0) + drift * 2.0 * hollow.at(x, z).max(0.0) + breakup * 25.0 * fray.at(wx, wz);
        let line = ctx.num_at("line", k);
        let m = smoothstep(line - soft, line + soft, effective) * (1.0 - smoothstep(max_slope - 4.0, max_slope + 4.0, slope));
        out[0][k] = m;
        out[1][k] = v + depth * m;
    }
}

// --------------------------------------------------------------- wetness --

pub static WETNESS: NodeSpec = NodeSpec {
    id: "mask.wetness",
    label: "Wetness",
    family: Family::Mask,
    summary: "Where the ground stays wet: where much water gathers and it is flat (with a Flow mask), or in flat hollows.",
    help: "With a Flow mask from erosion connected, this is the topographic wetness index: the more water flows through a place and the flatter it is, the wetter. Without one, flat hollows of Scale blocks count as wet. Wet ground takes moss, mud, reeds, swamps and dense forest.",
    keywords: &["wet", "moisture", "twi", "swamp", "marsh", "soil", "water"],
    inputs: &[RELIEF, PortSpec::new("flow", "Flow", PortKind::Mask, "How much water flows through each place, from erosion (hollows decide when unconnected).").optional()],
    outputs: &[mask_out("1 where the ground stays wet.")],
    params: &[
        ParamSpec::float("scale", "Scale", Unit::Blocks, 32.0, 2.0, 512.0, "The size of the hollows that count, in blocks (without Flow).").soft(4.0, 256.0),
        ParamSpec::float("sensitivity", "Sensitivity", Unit::Ratio, 0.5, 0.0, 1.0, "How little water or hollowing is enough to count as wet."),
    ],
    kernel: Kernel::Local { cpu: wetness_cpu, gpu: None, footprint: wetness_footprint },
    ..NodeSpec::BASE
};

fn wetness_footprint(p: &Params, lod: u8, input: usize) -> Footprint {
    if input == 1 {
        return Footprint { lod, halo: 0, map: None };
    }
    Footprint { lod, halo: (3.0 * sigma_of(p.num("scale")) / Space::spacing(lod)).ceil() as u32 + 1, map: None }
}

fn wetness_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let h = inputs[0].expect("wetness has its relief");
    let p = ctx.params;
    let sensitivity = p.f("sensitivity");
    let hollow = inputs[1].is_none().then(|| hollowness(h, sigma_of(p.num("scale"))));
    let full = (p.f("scale") * 0.05 * (1.0 - sensitivity) + 0.3).max(0.1);
    for (k, x, z) in tile(ctx) {
        let slope = slope_deg(h, x, z).max(1.0);
        out[0][k] = match (inputs[1], &hollow) {
            (Some(flow), _) => {
                let twi = ((flow.at(x, z).max(0.0) * 1000.0 + 1.0) / slope.to_radians().tan()).ln();
                (twi / (11.0 - 5.0 * sensitivity)).clamp(0.0, 1.0)
            }
            (None, Some(hl)) => smoothstep(0.0, full, hl.at(x, z)) * (1.0 - smoothstep(4.0, 20.0, slope)),
            (None, None) => 0.0,
        };
    }
}

// -------------------------------------------------------------- distance --

pub static DISTANCE: NodeSpec = NodeSpec {
    id: "mask.distance",
    label: "Distance",
    family: Family::Mask,
    summary: "How far each place lies from where a mask is: 1 on the mask, fading to 0 Reach blocks away, and the distance itself in blocks.",
    help: "Feed it where something is — the sea, rivers, a road, a painted area — and get a band around it: beaches and reeds by the water, a margin along a cliff, forests thinning away from rivers. Threshold says where the mask counts as there. Worked over the whole terrain at the simulations' resolution.",
    keywords: &["distance", "proximity", "near", "far", "buffer", "margin", "shore", "band"],
    inputs: &[MASK_IN],
    outputs: &[mask_out("1 on the mask, 0 Reach blocks from it."), PortSpec::new("distance", "Distance", PortKind::Height, "The distance to the mask, in blocks.")],
    params: &[
        ParamSpec::float("threshold", "Threshold", Unit::Ratio, 0.5, 0.0, 1.0, "Where the mask counts as there."),
        ParamSpec::float("reach", "Reach", Unit::Blocks, 200.0, 1.0, 65536.0, "How far from the mask the first output fades to 0, in blocks.").soft(4.0, 2000.0),
    ],
    kernel: Kernel::Global { cpu: distance_cpu, gpu: None, carry: &[None, None] },
    ..NodeSpec::BASE
};

fn distance_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], _: &(dyn Fn() -> bool + Sync)) -> Option<Vec<Vec<f32>>> {
    let m = inputs[0].expect("distance has its mask");
    let p = ctx.params;
    let t = p.f("threshold");
    let (n_x, n_z) = (m.rect.w as usize, m.rect.h as usize);
    let spacing = ctx.spacing() as f32;
    let d = filters::distance(n_x, n_z, |k| m.data[k] >= t);
    let reach = p.f("reach");
    let far = reach * 4.0;
    let blocks: Vec<f32> = d.iter().map(|v| if *v == f32::MAX { far } else { (v * spacing).min(far) }).collect();
    let near = blocks.iter().map(|b| 1.0 - (b / reach).clamp(0.0, 1.0)).collect();
    Some(vec![near, blocks])
}

// ------------------------------------------------------- threshold, etc. --

pub static THRESHOLD: NodeSpec = NodeSpec {
    id: "mask.threshold",
    label: "Threshold",
    family: Family::Mask,
    summary: "A mask made sharp: 1 above At, 0 below, over Softness.",
    help: "Turns a soft mask (or any field) into a crisp one. Softness 0 gives a hard edge; a little softness keeps it from stair-stepping.",
    keywords: &["threshold", "cutoff", "step", "binary", "sharpen", "contrast"],
    inputs: &[MASK_IN],
    outputs: &[mask_out("1 above At.")],
    params: &[
        ParamSpec::float("at", "At", Unit::Ratio, 0.5, -4096.0, 4096.0, "The value where the mask turns from 0 to 1.").soft(0.0, 1.0),
        ParamSpec::float("softness", "Softness", Unit::Ratio, 0.05, 0.0, 4096.0, "How wide the turn is.").soft(0.0, 0.5),
    ],
    kernel: Kernel::Point { cpu: threshold_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn threshold_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let m = inputs[0].expect("threshold has its input");
    let (at, soft) = (ctx.params.f("at"), ctx.params.f("softness") * 0.5);
    for (o, &v) in out[0].iter_mut().zip(m) {
        *o = smoothstep(at - soft, at + soft, v);
    }
}

pub static INVERT: NodeSpec = NodeSpec {
    id: "mask.invert",
    label: "Invert",
    family: Family::Mask,
    summary: "A mask turned over: 1 where it was 0, 0 where it was 1.",
    help: "Everywhere the mask is not: gentle ground from a slope mask, open ground from a forest mask, land from a sea mask.",
    keywords: &["invert", "not", "reverse", "opposite", "complement"],
    inputs: &[MASK_IN],
    outputs: &[mask_out("1 − the mask.")],
    params: &[],
    kernel: Kernel::Point { cpu: invert_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn invert_cpu(_: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let m = inputs[0].expect("invert has its input");
    for (o, &v) in out[0].iter_mut().zip(m) {
        *o = 1.0 - v;
    }
}

pub static EXPAND: NodeSpec = NodeSpec {
    id: "mask.expand",
    label: "Grow or shrink",
    family: Family::Mask,
    summary: "A mask grown outward by Radius blocks, or shrunk inward with a negative radius.",
    help: "Grow a river mask into its banks, a road into its verges; shrink a land mask away from the shore, a forest away from its edge. Wide radii are worked at a coarser level and come back smooth.",
    keywords: &["dilate", "erode", "grow", "shrink", "expand", "contract", "morphology", "margin"],
    inputs: &[MASK_IN],
    outputs: &[mask_out("The mask grown or shrunk.")],
    params: &[ParamSpec::float("radius", "Radius", Unit::Blocks, 8.0, -4096.0, 4096.0, "How far the mask grows, in blocks (negative: shrinks).").soft(-128.0, 128.0)],
    kernel: Kernel::Local { cpu: expand_cpu, gpu: None, footprint: expand_footprint },
    ..NodeSpec::BASE
};

fn expand_footprint(p: &Params, lod: u8, _: usize) -> Footprint {
    let r = p.num("radius").abs();
    let work = work_lod(lod, r, 16.0);
    Footprint { lod: work, halo: (r / Space::spacing(work)).ceil() as u32 + 2, map: None }
}

fn expand_cpu(ctx: &Ctx, inputs: &[Option<&Grid>], out: &mut [Vec<f32>]) {
    let src = inputs[0].expect("grow has its input");
    let r = ctx.params.num("radius");
    let samples = (r.abs() / Space::spacing(src.lod)).round() as i32;
    let g = filters::extreme(src, samples, r > 0.0);
    filters::resample(ctx, &g, &mut out[0]);
}
