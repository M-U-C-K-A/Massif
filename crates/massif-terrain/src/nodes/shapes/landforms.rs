//! Landforms of their own: volcanoes, mesas and buttes, dunes, karst,
//! craters.

use super::{Relief, Warp, bounds, finest, samples, smoothstep};
use crate::kernel::Ctx;
use crate::nodes::height_out;
use crate::noise::{Fbm, noise1, voronoi};
use crate::params::Point;
use crate::registry::{Choice, Family, Field, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use crate::util::mix32;

const MASK_OUT: PortSpec = PortSpec::new("mask", "Mask", PortKind::Mask, "1 on the landform, 0 around it.");
const BASE: ParamSpec = ParamSpec::float("base", "Base", Unit::Blocks, 70.0, -2048.0, 4096.0, "The height of the ground around it, in blocks (0 to add it to another relief).").soft(-64.0, 320.0);
const ROUGHNESS: ParamSpec = ParamSpec::float("roughness", "Roughness", Unit::Ratio, 0.4, 0.0, 1.0, "How much small relief it carries.").section(Section::Detail);

/// A random number from 0 to 1 for item `i`, draw `k`.
fn rnd(seed: u32, i: u32, k: u32) -> f32 {
    (mix32(seed ^ mix32(i.wrapping_mul(0x9e37_79b9) ^ k.wrapping_mul(0x85eb_ca6b))) & 0xffff) as f32 / 65535.0
}

// -------------------------------------------------------------- volcano --

const VOLCANO_FIELDS: &[Field] = &[
    Field { name: "height", label: "Height", unit: Unit::Blocks, default: 180.0, min: 0.0, max: 4096.0, help: "How high the volcano rises over Base, in blocks." },
    Field { name: "radius", label: "Radius", unit: Unit::Blocks, default: 800.0, min: 32.0, max: 32000.0, help: "How far its flanks reach, in blocks." },
];

const KINDS: &[Choice] = &[
    Choice { value: "strato", label: "Stratovolcano", help: "Steep and concave, a small crater at the top: Fuji, Etna." },
    Choice { value: "shield", label: "Shield", help: "Broad and gentle, its top a wide dome with a pit: Mauna Loa." },
    Choice { value: "caldera", label: "Caldera", help: "A volcano whose top fell in: a wide, flat-floored crater ringed by cliffs: Crater Lake." },
    Choice { value: "cinder", label: "Cinder cone", help: "A small, straight-sided cone with a deep crater." },
];

pub static VOLCANO: NodeSpec = NodeSpec {
    id: "mountain.volcano",
    label: "Volcano",
    family: Family::Shape,
    summary: "Volcanoes placed by hand: steep stratovolcanoes, broad shields, calderas or cinder cones, with a crater, gullies down their flanks and smaller cones on them.",
    help: "Place each volcano on the viewer with its height and radius. The kind sets its profile; Crater is the crater's width as a share of the radius (a caldera's is wider), Crater depth how far below the rim its floor lies. Gullies are the channels old flows cut down the flanks; Cones adds smaller vents on them. On an island, take the Max with the sea floor.",
    keywords: &["volcano", "crater", "caldera", "cone", "lava", "shield", "fuji", "etna"],
    inputs: &[],
    outputs: &[height_out("The volcanoes over Base."), MASK_OUT],
    params: &[
        ParamSpec::points("volcanoes", "Volcanoes", VOLCANO_FIELDS, 16, "The volcanoes, placed on the viewer, each with its height and radius."),
        ParamSpec::choice("kind", "Kind", KINDS, "strato", "The kind of volcano: its profile and its crater."),
        BASE,
        ParamSpec::float("crater", "Crater", Unit::Ratio, 0.1, 0.0, 0.6, "How wide the crater is, as a share of the radius (a caldera's is at least 0.3).").section(Section::Shape),
        ParamSpec::float("crater_depth", "Crater depth", Unit::Blocks, 35.0, 0.0, 1024.0, "How far below the rim the crater's floor lies, in blocks.").soft(0.0, 200.0).section(Section::Shape),
        ParamSpec::float("gullies", "Gullies", Unit::Ratio, 0.5, 0.0, 1.0, "How deeply channels cut the flanks.").section(Section::Shape),
        ParamSpec::int("gully_count", "Gully count", 14, 3, 96, "How many channels run down each volcano.").section(Section::Shape),
        ParamSpec::int("cones", "Cones", 3, 0, 16, "How many smaller cones stand on each volcano's flanks.").section(Section::Shape),
        ROUGHNESS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: volcano_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// A volcano's height (0–1) at `u` from its foot (0) to its centre (1).
fn volcano_profile(kind: &str, u: f32) -> f32 {
    match kind {
        "shield" => 1.0 - (1.0 - u).powf(1.8),
        "cinder" => u,
        // Concave, steepening to the top.
        _ => ((1.9 * u).exp() - 1.0) / (1.9f32.exp() - 1.0),
    }
}

fn volcano_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let size = ctx.space.size as f64;
    let kind = p.choice("kind");
    let given = p.points("volcanoes");
    let list: Vec<Point> = if given.is_empty() { vec![Point { x: 0.5, z: 0.5, values: vec![180.0, 800.0] }] } else { given.to_vec() };
    let crater = match kind {
        "caldera" => p.f("crater").max(0.3),
        "cinder" => p.f("crater").max(0.22),
        _ => p.f("crater"),
    };
    let depth = p.f("crater_depth");
    let (gullies, count) = (p.f("gullies"), p.int("gully_count") as f32);
    let cones = p.int("cones") as u32;
    let roughness = p.f("roughness");
    let (x0, z0, x1, z1) = bounds(ctx);
    let near: Vec<(usize, f64, f64, f32, f64)> = list
        .iter()
        .enumerate()
        .map(|(i, v)| (i, v.x * size, v.z * size, v.value(0) as f32, v.value(1)))
        .filter(|&(_, x, z, _, r)| x + r >= x0 && x - r <= x1 && z + r >= z0 && z - r <= z1)
        .collect();
    let relief = Relief::new(400.0, 0.5, 0.3, finest(ctx), seed);
    for (k, x, z) in samples(ctx) {
        let (mut best, mut mask) = (0.0f32, 0.0f32);
        for &(i, cx, cz, h, r) in &near {
            let s = mix32(seed ^ i as u32);
            let (dx, dz) = ((x - cx) / r, (z - cz) / r);
            let dist = dx.hypot(dz) as f32;
            if dist >= 1.0 {
                continue;
            }
            let u = 1.0 - dist;
            let mut v = volcano_profile(kind, u);
            // The crater: steep inner walls down to a flat floor.
            if crater > 0.0 && dist < crater * 1.15 {
                let rim = volcano_profile(kind, 1.0 - crater);
                let floor = rim - depth / h.max(1.0);
                let inside = smoothstep(crater * 0.55, crater, dist);
                let lip = if dist > crater { volcano_profile(kind, u).min(rim) } else { floor + (rim - floor) * inside };
                v = lip;
            }
            // Channels down the flanks, strongest halfway down.
            let angle = (dz as f32).atan2(dx as f32);
            let g = (1.0 - noise1((angle / std::f32::consts::TAU * count) as f64 + 0.35 * noise1(dist as f64 * 4.0, s) as f64, s).abs()).powf(3.0);
            v -= gullies * 0.12 * g * 4.0 * u * (1.0 - u) * v.max(0.0).sqrt();
            // Smaller cones on the flanks.
            for c in 0..cones {
                let (a, d) = (rnd(s, c, 1) * std::f32::consts::TAU, 0.35 + 0.4 * rnd(s, c, 2));
                let (px, pz) = (d * a.cos(), d * a.sin());
                let cr = 0.05 + 0.06 * rnd(s, c, 3);
                let cd = (dx as f32 - px).hypot(dz as f32 - pz) / cr;
                if cd < 1.0 {
                    let cone = (1.0 - cd) * (0.08 + 0.08 * rnd(s, c, 4)) - if cd < 0.25 { 0.03 * (1.0 - cd / 0.25) } else { 0.0 };
                    v += cone.max(0.0);
                }
            }
            best = best.max(h * v.max(0.0));
            mask = mask.max(u.min(1.0));
        }
        out[0][k] = base + best * (1.0 + roughness * 0.3 * (relief.at(x, z) - 0.5) * mask);
        out[1][k] = mask;
    }
}

// ----------------------------------------------------------------- mesa --

pub static MESA: NodeSpec = NodeSpec {
    id: "plateau.mesa",
    label: "Mesas and buttes",
    family: Family::Shape,
    summary: "Flat-topped tablelands cut by cliffs: mesas, buttes standing alone, stepped walls of rock layers, talus at their feet.",
    help: "Coverage is the share of the ground left high, Scale the size of the mesas: small ones stand alone as buttes. Cliff is how sheer their walls are, Layers how many steps the rock beds make in them, Talus how far the scree spreads at their feet. Dissection cuts canyons into the tablelands. Connect a mask to Area to say where they rise.",
    keywords: &["mesa", "butte", "plateau", "table", "cliff", "desert", "canyonlands", "monument valley", "strata"],
    inputs: &[PortSpec::new("area", "Area", PortKind::Mask, "Where the tablelands rise, from 0 to 1 (Coverage spreads them when unconnected).").optional()],
    outputs: &[height_out("The tablelands over Base."), MASK_OUT],
    params: &[
        ParamSpec::float("height", "Height", Unit::Blocks, 60.0, 0.0, 2048.0, "How high the tops stand over Base, in blocks.").soft(0.0, 200.0).drivable(),
        BASE,
        ParamSpec::float("scale", "Scale", Unit::Blocks, 900.0, 32.0, 32000.0, "The size of the mesas, in blocks.").soft(100.0, 4000.0),
        ParamSpec::float("coverage", "Coverage", Unit::Ratio, 0.45, 0.0, 1.0, "The share of the ground left high."),
        ParamSpec::float("cliff", "Cliff", Unit::Ratio, 0.85, 0.0, 1.0, "How sheer the walls are.").section(Section::Shape),
        ParamSpec::int("layers", "Layers", 3, 0, 16, "How many steps the rock beds make in the walls.").section(Section::Shape),
        ParamSpec::float("talus", "Talus", Unit::Ratio, 0.5, 0.0, 1.0, "How far scree spreads at the feet of the walls.").section(Section::Shape),
        ParamSpec::float("dissection", "Dissection", Unit::Ratio, 0.35, 0.0, 1.0, "How much canyons cut into the tablelands.").section(Section::Shape),
        ROUGHNESS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: mesa_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn mesa_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let scale = p.num("scale");
    let warp = Warp::new(scale * 1.5, scale * 0.35, seed);
    let land = Fbm::down_to(scale, finest(ctx).max(scale / 32.0), 0.5, seed ^ 0x3a);
    let cuts = Fbm::new(scale * 0.8, 2, 0.45, seed ^ 0x5c);
    // Four octaves of fbm spread like a normal of deviation ~0.25.
    let threshold = 0.25 * super::coasts::normal_quantile(1.0 - p.num("coverage")) as f32;
    let cliff = p.f("cliff");
    let edge = 0.015 + 0.12 * (1.0 - cliff);
    let layers = p.int("layers") as f32;
    let talus = p.f("talus");
    let dissection = p.f("dissection");
    let roughness = p.f("roughness");
    let relief = Relief::new(scale * 0.3, 0.5, 0.2, finest(ctx), seed ^ 0x77);
    for (k, x, z) in samples(ctx) {
        let (wx, wz) = warp.at(x, z);
        let mut n = land.at(wx, wz);
        if let Some(area) = inputs[0] {
            n = n * 0.35 + (area[k] - 0.5) * 1.3;
        }
        // Canyons cutting into the tablelands.
        n -= dissection * 0.5 * (1.0 - cuts.at(wx, wz).abs()).powf(18.0);
        let e = ((n - threshold) / edge).clamp(-6.0, 1.0);
        // The walls: steps of rock beds, each a ledge then a riser.
        let wall = if e <= 0.0 {
            0.0
        } else if layers > 0.0 {
            let s = e * (layers + 1.0);
            let step = s.floor();
            (step + smoothstep(0.55, 1.0, s - step)) / (layers + 1.0)
        } else {
            e
        };
        // Scree at the feet: a concave apron below the walls.
        let apron = talus * 0.3 * (1.0 + e / 6.0).clamp(0.0, 1.0).powf(3.0) * if e < 0.0 { 1.0 } else { 0.0 };
        let shape = wall.max(apron);
        let detail = 1.0 + roughness * 0.15 * (relief.at(x, z) - 0.5);
        out[0][k] = base + ctx.num_at("height", k) * shape * detail;
        out[1][k] = e.clamp(0.0, 1.0);
    }
}

// ---------------------------------------------------------------- dunes --

const DUNE_KINDS: &[Choice] = &[
    Choice { value: "transverse", label: "Transverse", help: "Long crests across the wind, a gentle slope up and a steep one down: sand seas." },
    Choice { value: "linear", label: "Linear", help: "Long ridges along the wind (seifs), sharp crests between corridors." },
    Choice { value: "barchan", label: "Barchan", help: "Crescents scattered on hard ground, their horns pointing downwind." },
    Choice { value: "star", label: "Star", help: "Pyramids of sand with arms in three directions, where the wind turns." },
];

pub static DUNES: NodeSpec = NodeSpec {
    id: "land.dunes",
    label: "Dunes",
    family: Family::Shape,
    summary: "Sand shaped by the wind: transverse crests, linear ridges, crescent barchans or star dunes, with ripples over them.",
    help: "Direction is where the wind blows to (0 east, 90 south); Wavelength how far apart the crests are; Height how tall they stand. Irregularity breaks and bends the crests. Connect a mask to Sand to say where there is sand (the dunes fade where there is less). Ripples add the wind's small waves where the samples are fine enough to hold them.",
    keywords: &["dunes", "sand", "desert", "erg", "sahara", "barchan", "seif", "wind"],
    inputs: &[PortSpec::new("sand", "Sand", PortKind::Mask, "Where there is sand, from 0 to 1 (everywhere when unconnected).").optional()],
    outputs: &[height_out("The dunes over Base."), MASK_OUT],
    params: &[
        ParamSpec::choice("kind", "Kind", DUNE_KINDS, "transverse", "The kind of dunes."),
        ParamSpec::float("direction", "Direction", Unit::Degrees, 20.0, -180.0, 180.0, "Where the wind blows to, in degrees from east (90: south)."),
        ParamSpec::float("wavelength", "Wavelength", Unit::Blocks, 140.0, 8.0, 8000.0, "How far apart the crests are, in blocks.").soft(20.0, 800.0),
        ParamSpec::float("height", "Height", Unit::Blocks, 14.0, 0.0, 512.0, "How tall the dunes stand, in blocks.").soft(0.0, 60.0).drivable(),
        BASE,
        ParamSpec::float("irregularity", "Irregularity", Unit::Ratio, 0.5, 0.0, 1.0, "How much the crests bend, break and vary.").section(Section::Shape),
        ParamSpec::float("ripples", "Ripples", Unit::Ratio, 0.3, 0.0, 1.0, "How strong the wind's small waves are.").section(Section::Detail),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: dunes_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// A dune's cross-section at phase `f` (0–1): a long gentle stoss slope,
/// a short steep lee face, rounded at the crest.
fn stoss_lee(f: f32) -> f32 {
    let f = f - f.floor();
    let s = if f < 0.78 { f / 0.78 } else { (1.0 - f) / 0.22 };
    s * s * (3.0 - 2.0 * s)
}

fn dunes_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let lambda = p.num("wavelength");
    let (s, c) = p.num("direction").to_radians().sin_cos();
    let irr = p.f("irregularity");
    let bend = Fbm::new(lambda * 5.0, 4, 0.55, seed ^ 0x11);
    let vary = Fbm::new(lambda * 3.0, 2, 0.5, seed ^ 0x22);
    let kind = p.choice("kind");
    let ripples = p.f("ripples");
    // Ripples a few blocks apart, only where the samples can hold them.
    let ripple_ok = ctx.spacing() <= 2.0;
    for (k, x, z) in samples(ctx) {
        let (along, across) = (x * c + z * s, -x * s + z * c);
        let b = irr as f64 * bend.at(x, z) as f64;
        let v = match kind {
            "linear" => {
                let f = (across / lambda + 0.8 * b) as f32;
                (1.0 - (2.0 * (f - f.floor()) - 1.0).abs()).powf(1.3)
            }
            "star" => {
                // A pyramid of sand on each cell's point, three to five arms
                // reaching out from it, each turned its own way; the
                // neighbouring cells' arms reach in too.
                let (cx, cz) = ((x + b * lambda) / (lambda * 1.6), (z - b * lambda) / (lambda * 1.6));
                let (gx, gz) = (cx.floor() as i64, cz.floor() as i64);
                let mut m = 0.0f32;
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let h = crate::noise::hash2((gx + dx) as u32, (gz + dz) as u32, seed);
                        let px = (gx + dx) as f64 + 0.5 + 0.9 * ((h & 0xffff) as f64 / 65535.0 - 0.5);
                        let pz = (gz + dz) as f64 + 0.5 + 0.9 * ((h >> 16) as f64 / 65535.0 - 0.5);
                        let (ux, uz) = ((cx - px) as f32, (cz - pz) as f32);
                        let r = ux.hypot(uz);
                        if r > 0.9 {
                            continue;
                        }
                        let id = crate::util::mix32(h);
                        let arms = 3 + (id % 3);
                        let turn = (id >> 8) as f32 / 16_777_216.0 * std::f32::consts::TAU;
                        let tall = 0.6 + 0.4 * ((id >> 4) & 0xf) as f32 / 15.0;
                        let mut v = (1.0 - r / 0.45).max(0.0).powf(1.5);
                        for a in 0..arms {
                            let (sa, ca) = (turn + a as f32 * std::f32::consts::TAU / arms as f32).sin_cos();
                            let t = ux * ca + uz * sa;
                            let across = (uz * ca - ux * sa).abs();
                            if t > 0.0 && t < 0.85 {
                                v = v.max((1.0 - t / 0.85) * (1.0 - across / (0.05 + 0.12 * t)).max(0.0));
                            }
                        }
                        m = m.max(v * tall);
                    }
                }
                m
            }
            "barchan" => {
                // Crescents: crests bent downwind at their ends, in patches.
                let (f1, _, id) = voronoi(x / (lambda * 2.5), z / (lambda * 2.5), 0.8, seed);
                let patch = smoothstep(0.55, 0.2, f1);
                let horn = ((across / lambda).sin() as f32).powi(2);
                let f = (along / lambda + 0.5 * horn as f64 + b) as f32 + (id & 0xff) as f32 / 255.0;
                stoss_lee(f) * patch
            }
            _ => {
                // Crests that bend, split and merge: the phase wanders at
                // two scales.
                let f = along / lambda + 1.6 * b + 0.25 * irr as f64 * vary.at(x * 2.0, z * 2.0) as f64;
                stoss_lee(f as f32)
            }
        };
        let height = 0.6 + 0.4 * (0.5 + 0.5 * vary.at(x, z));
        let ripple = if ripple_ok { ripples * 0.04 * ((along / 5.0 + across * 0.05) as f32 * std::f32::consts::TAU).sin() } else { 0.0 };
        let sand = inputs[0].map_or(1.0, |m| m[k].clamp(0.0, 1.0));
        out[0][k] = base + ctx.num_at("height", k) * sand * (v * height + ripple);
        out[1][k] = sand;
    }
}

// ---------------------------------------------------------------- karst --

const KARST_KINDS: &[Choice] = &[
    Choice { value: "towers", label: "Towers", help: "Steep pillars of limestone rising from a plain: Guilin, Ha Long Bay." },
    Choice { value: "cockpit", label: "Cockpit", help: "Crowded conical hills around star-shaped hollows: Jamaica's Cockpit Country." },
    Choice { value: "sinkholes", label: "Sinkholes", help: "A plain pitted with round hollows where the rock dissolved beneath." },
];

pub static KARST: NodeSpec = NodeSpec {
    id: "land.karst",
    label: "Karst",
    family: Family::Shape,
    summary: "Limestone dissolved by water: steep towers on a plain, crowded cones around hollows, or a plain pitted with sinkholes.",
    help: "Spacing is how far apart the towers, cones or sinkholes are, Height how tall the towers (or deep the sinkholes) are. Steepness makes the towers' walls sheer. Coverage is the share of places that hold one. Put it over a coastline for a drowned karst like Ha Long Bay.",
    keywords: &["karst", "towers", "limestone", "guilin", "ha long", "pillars", "sinkholes", "cockpit"],
    inputs: &[],
    outputs: &[height_out("The karst over Base."), MASK_OUT],
    params: &[
        ParamSpec::choice("kind", "Kind", KARST_KINDS, "towers", "The kind of karst."),
        ParamSpec::float("spacing", "Spacing", Unit::Blocks, 160.0, 8.0, 8000.0, "How far apart the towers, cones or sinkholes are, in blocks.").soft(30.0, 1000.0),
        ParamSpec::float("height", "Height", Unit::Blocks, 70.0, 0.0, 1024.0, "How tall the towers are (how deep the sinkholes), in blocks.").soft(0.0, 200.0).drivable(),
        BASE,
        ParamSpec::float("steepness", "Steepness", Unit::Ratio, 0.85, 0.0, 1.0, "How sheer the towers' walls are.").section(Section::Shape),
        ParamSpec::float("coverage", "Coverage", Unit::Ratio, 0.6, 0.0, 1.0, "The share of places holding a tower, a cone or a sinkhole.").section(Section::Shape),
        ROUGHNESS,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: karst_cpu, gpu: None },
    ..NodeSpec::BASE
};

fn karst_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let base = p.f("base");
    let spacing = p.num("spacing");
    let kind = p.choice("kind");
    let steep = p.f("steepness");
    let coverage = p.f("coverage");
    let roughness = p.f("roughness");
    let warp = Warp::new(spacing * 2.0, spacing * 0.25, seed);
    let relief = Relief::new(spacing * 0.5, 0.6, 0.5, finest(ctx), seed ^ 0x99);
    for (k, x, z) in samples(ctx) {
        let (wx, wz) = warp.at(x, z);
        let (f1, f2, id) = voronoi(wx / spacing, wz / spacing, 0.85, seed);
        let present = ((id >> 8) & 0xffff) as f32 / 65535.0 < coverage;
        let tall = 0.55 + 0.45 * (id & 0xff) as f32 / 255.0;
        let (v, mask) = match kind {
            "cockpit" => {
                // Cones around the points, hollows where three cells meet.
                let cone = smoothstep(0.75, 0.0, f1) * if present { 1.0 } else { 0.5 };
                let hollow = smoothstep(0.08, 0.0, f2 - f1) * 0.25;
                ((cone - hollow).max(0.0) * tall, cone)
            }
            "sinkholes" => {
                let r = 0.2 + 0.15 * tall;
                let pit = if present { smoothstep(r, r * 0.3, f1) } else { 0.0 };
                (-pit * (0.4 + 0.6 * tall), pit)
            }
            _ => {
                // Towers on the points of this cell and its neighbours (so
                // that none is cut at a cell's edge): sheer, ragged walls,
                // rounded tops, a scree foot.
                let (cx, cz) = (wx / spacing, wz / spacing);
                let (gx, gz) = (cx.floor() as i64, cz.floor() as i64);
                let ragged = 1.0 + 0.25 * relief.at(x * 1.7, z * 1.7);
                let (mut v, mut m) = (0.0f32, 0.0f32);
                for dz in -1..=1 {
                    for dx in -1..=1 {
                        let h = crate::noise::hash2((gx + dx) as u32, (gz + dz) as u32, seed);
                        if ((h >> 8) & 0xffff) as f32 / 65535.0 >= coverage {
                            continue;
                        }
                        let px = (gx + dx) as f64 + 0.5 + 0.85 * ((h & 0xffff) as f64 / 65535.0 - 0.5);
                        let pz = (gz + dz) as f64 + 0.5 + 0.85 * ((h >> 16) as f64 / 65535.0 - 0.5);
                        let d = ((cx - px).hypot(cz - pz) as f32) / ragged;
                        let t = 0.55 + 0.45 * (h & 0xff) as f32 / 255.0;
                        let r = 0.18 + 0.16 * t;
                        let wall = smoothstep(r, r * (1.0 - 0.85 * steep), d);
                        let foot = smoothstep(r * 1.6, r, d) * 0.12;
                        let top = 1.0 - 0.15 * (d / r).min(1.0).powi(2);
                        v = v.max((wall * top).max(foot) * t);
                        m = m.max(wall);
                    }
                }
                (v, m)
            }
        };
        let d = 1.0 + roughness * 0.4 * (relief.at(x, z) - 0.5) * mask;
        out[0][k] = base + ctx.num_at("height", k) * v * d;
        out[1][k] = if present || kind == "towers" { mask } else { 0.0 };
    }
}

// --------------------------------------------------------------- craters --

const CRATER_FIELDS: &[Field] = &[
    Field { name: "radius", label: "Radius", unit: Unit::Blocks, default: 300.0, min: 4.0, max: 16000.0, help: "The crater's radius, in blocks." },
    Field { name: "depth", label: "Depth", unit: Unit::Blocks, default: 40.0, min: 0.0, max: 2048.0, help: "How deep its floor lies below the ground, in blocks." },
];

pub static CRATERS: NodeSpec = NodeSpec {
    id: "crater.field",
    label: "Craters",
    family: Family::Shape,
    summary: "Impact craters: a bowl with a raised rim and its ejecta, a central peak in the large ones; placed by hand or scattered, many small and few large.",
    help: "Place craters on the viewer (each with its radius and depth) or let Count scatter them, from Smallest to Largest. Connect a relief to Height to dig them into it; otherwise they lie on a ground at Base. Large craters (over Peak size) get a central peak and a flat floor.",
    keywords: &["crater", "impact", "meteor", "moon", "bowl", "pockmarks"],
    inputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief to dig them into (a ground at Base when unconnected).").optional()],
    outputs: &[height_out("The relief with its craters."), MASK_OUT],
    params: &[
        ParamSpec::points("craters", "Craters", CRATER_FIELDS, 64, "Craters placed by hand, each with its radius and depth."),
        ParamSpec::int("count", "Count", 30, 0, 2000, "How many craters are scattered."),
        ParamSpec::float("smallest", "Smallest", Unit::Blocks, 20.0, 4.0, 8000.0, "The radius of the smallest scattered craters, in blocks.").soft(4.0, 400.0),
        ParamSpec::float("largest", "Largest", Unit::Blocks, 400.0, 4.0, 16000.0, "The radius of the largest scattered craters, in blocks.").soft(20.0, 3000.0),
        ParamSpec::float("depth_ratio", "Depth", Unit::Ratio, 0.18, 0.0, 1.0, "How deep the scattered craters are, as a share of their radius.").section(Section::Shape),
        ParamSpec::float("rim", "Rim", Unit::Ratio, 0.35, 0.0, 1.0, "How high the rim stands, as a share of the depth.").section(Section::Shape),
        ParamSpec::float("peak_size", "Peak size", Unit::Blocks, 250.0, 8.0, 16000.0, "Craters wider than this get a central peak and a flat floor, in blocks.").section(Section::Shape),
        BASE,
        ParamSpec::seed(),
    ],
    kernel: Kernel::Point { cpu: craters_cpu, gpu: None },
    ..NodeSpec::BASE
};

/// A crater's change of height at `r` radii from its centre, for a depth
/// of 1: bowl, rim, ejecta; a flat floor and a central peak when `complex`.
fn crater_profile(r: f32, rim: f32, complex: bool) -> f32 {
    if r < 1.0 {
        let bowl = if complex { -(1.0 - smoothstep(0.55, 1.0, r)) + 0.35 * smoothstep(0.18, 0.0, r) } else { r * r - 1.0 };
        bowl + rim * smoothstep(0.7, 1.0, r)
    } else {
        rim * (-(r - 1.0) * 4.0).exp()
    }
}

fn craters_cpu(ctx: &Ctx, inputs: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let seed = ctx.seed;
    let size = ctx.space.size as f64;
    let base = p.f("base");
    let (small, large) = (p.f("smallest"), p.f("largest").max(p.f("smallest")));
    let ratio = p.f("depth_ratio");
    let mut list: Vec<(f64, f64, f32, f32)> = (0..p.int("count").max(0) as u32)
        .map(|i| {
            let t = rnd(seed, i, 1).powf(3.0);
            let r = small + (large - small) * t;
            ((0.02 + 0.96 * rnd(seed, i, 2) as f64) * size, (0.02 + 0.96 * rnd(seed, i, 3) as f64) * size, r, r * ratio)
        })
        .collect();
    list.extend(p.points("craters").iter().map(|c| (c.x * size, c.z * size, c.value(0) as f32, c.value(1) as f32)));
    let (x0, z0, x1, z1) = bounds(ctx);
    let near: Vec<&(f64, f64, f32, f32)> = list.iter().filter(|(x, z, r, _)| {
        let reach = *r as f64 * 2.0;
        x + reach >= x0 && x - reach <= x1 && z + reach >= z0 && z - reach <= z1
    }).collect();
    let rim = p.f("rim");
    let peak = p.f("peak_size");
    for (k, x, z) in samples(ctx) {
        let ground = inputs[0].map_or(base, |h| h[k]);
        let (mut change, mut mask) = (0.0f32, 0.0f32);
        for &&(cx, cz, r, d) in &near {
            let rr = ((x - cx).hypot(z - cz) / r as f64) as f32;
            if rr >= 2.0 {
                continue;
            }
            let c = d * crater_profile(rr, rim, r > peak);
            // The deepest dig wins; rims add up.
            change = if c < 0.0 { change.min(c) } else { change + c * 0.5 };
            mask = mask.max(smoothstep(1.05, 0.8, rr));
        }
        out[0][k] = ground + change;
        out[1][k] = mask;
    }
}
