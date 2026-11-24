//! Geometry: points scattered over the terrain, and points drawn back as a
//! mask. The first nodes of the geometry family, and the pattern the
//! village, road and forest nodes follow: a [`Kernel::Geometry`] node makes
//! its points, splines, footprints or instances whole; a raster node reads
//! them through [`Ctx::geo`].

use crate::kernel::{Ctx, Geo, Grid};
use crate::noise::hash2;
use crate::registry::{Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Section, Unit};
use massif_core::geometry::{Attrs, Point, Points};

pub const NODES: &[&NodeSpec] = &[&SCATTER, &POINTS_MASK];

pub static SCATTER: NodeSpec = NodeSpec {
    id: "points.scatter",
    label: "Scatter",
    family: Family::Geometry,
    summary: "Points spread over the terrain, as many as a density mask asks, never closer than a spacing.",
    help: "One point at most in each square of Spacing blocks, at a place of its own inside it: kept where a random draw falls under the density there. Trees, rocks, ruins or the places villages may start: give it the mask of where they may stand, and the relief for the points' heights.",
    keywords: &["scatter", "points", "distribute", "spread", "poisson", "place", "instances"],
    inputs: &[
        PortSpec::new("density", "Density", PortKind::Mask, "How likely a point is at each place, from 0 to 1.").or("1 everywhere"),
        PortSpec::new("height", "Height", PortKind::Height, "The relief the points stand on: their height.").or("the sea level"),
    ],
    outputs: &[PortSpec::new("points", "Points", PortKind::Points, "The points, each standing on the relief, turned at random.")],
    params: &[
        ParamSpec::float("spacing", "Spacing", Unit::Blocks, 64.0, 2.0, 16384.0, "The size of the squares the points are drawn in: no two points are much closer than half of it.").soft(4.0, 1024.0),
        ParamSpec::float("jitter", "Jitter", Unit::Ratio, 0.8, 0.0, 1.0, "How far a point wanders inside its square: 0 lines them up on a grid.").section(Section::Shape),
        ParamSpec::seed(),
    ],
    kernel: Kernel::Geometry { cpu: scatter },
    ..NodeSpec::BASE
};

fn scatter(ctx: &Ctx, rasters: &[Option<&Grid>], cancel: &(dyn Fn() -> bool + Sync)) -> Result<Vec<Geo>, String> {
    let p = ctx.params;
    let spacing = p.num("spacing").max(1.0);
    let jitter = p.num("jitter").clamp(0.0, 1.0);
    let size = ctx.space.size as f64;
    let cells = (size / spacing).ceil() as u32;
    if cells as u64 * cells as u64 > 4_000_000 {
        return Err(format!("{} points to draw: give a larger spacing", cells as u64 * cells as u64));
    }
    let step = ctx.spacing();
    let sample = |g: Option<&Grid>, bx: f64, bz: f64, or: f32| g.map_or(or, |g| g.bilinear(bx / step, bz / step));
    let mut items = Vec::new();
    for j in 0..cells {
        if cancel() {
            return Err("cancelled".into());
        }
        for i in 0..cells {
            let h = hash2(i, j, ctx.seed);
            let u = |k: u32| (hash2(h, k, ctx.seed) as f64) / u32::MAX as f64;
            let bx = (i as f64 + 0.5 + (u(1) - 0.5) * jitter) * spacing;
            let bz = (j as f64 + 0.5 + (u(2) - 0.5) * jitter) * spacing;
            if bx >= size || bz >= size {
                continue;
            }
            if u(3) as f32 >= sample(rasters[0], bx, bz, 1.0) {
                continue;
            }
            let y = sample(rasters[1], bx, bz, ctx.space.sea_level);
            let pos = [ctx.space.x0 as f64 + bx, y as f64, ctx.space.z0 as f64 + bz];
            items.push(Point { pos, turn: (u(4) * 360.0) as f32, scale: 1.0, id: ((j as u64) << 32) | i as u64, kind: String::new(), attrs: Attrs::new() });
        }
    }
    Ok(vec![Geo::Points(Points { items })])
}

pub static POINTS_MASK: NodeSpec = NodeSpec {
    id: "points.mask",
    label: "Points to mask",
    family: Family::Geometry,
    summary: "A mask of discs round points: 1 within Radius of one, fading to 0 over Softness.",
    help: "Where points stand, as a mask: keep the forest off the places a village takes, raise a mound under each ruin, paint the ground round each tree.",
    keywords: &["points", "mask", "disc", "rasterize", "around", "footprint"],
    inputs: &[PortSpec::new("points", "Points", PortKind::Points, "The points.")],
    outputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "1 near the points.")],
    params: &[
        ParamSpec::float("radius", "Radius", Unit::Blocks, 8.0, 0.0, 4096.0, "How far round each point the mask is 1, in blocks (times the point's scale).").soft(0.0, 128.0),
        ParamSpec::float("softness", "Softness", Unit::Blocks, 4.0, 0.0, 4096.0, "How far beyond Radius it fades to 0, in blocks.").soft(0.0, 64.0),
    ],
    kernel: Kernel::Point { cpu: points_mask, gpu: None },
    ..NodeSpec::BASE
};

fn points_mask(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let Some(points) = ctx.geo(0).and_then(Geo::points) else { return };
    let radius = ctx.params.num("radius");
    let soft = ctx.params.num("softness").max(1e-6);
    let step = ctx.spacing();
    let r = ctx.rect;
    let (x0, z0) = (ctx.space.x0 as f64 + r.x as f64 * step, ctx.space.z0 as f64 + r.z as f64 * step);
    let (x1, z1) = (x0 + r.w as f64 * step, z0 + r.h as f64 * step);
    let o = &mut out[0];
    for p in &points.items {
        let reach = radius * p.scale as f64 + soft;
        let [px, _, pz] = p.pos;
        if px + reach < x0 || px - reach > x1 || pz + reach < z0 || pz - reach > z1 {
            continue;
        }
        let i0 = (((px - reach - x0) / step).floor().max(0.0)) as usize;
        let i1 = (((px + reach - x0) / step).ceil().min(r.w as f64)) as usize;
        let j0 = (((pz - reach - z0) / step).floor().max(0.0)) as usize;
        let j1 = (((pz + reach - z0) / step).ceil().min(r.h as f64)) as usize;
        for j in j0..j1 {
            for i in i0..i1 {
                let (bx, bz) = (x0 + i as f64 * step, z0 + j as f64 * step);
                let d = ((bx - px).powi(2) + (bz - pz).powi(2)).sqrt();
                let v = (1.0 - (d - radius * p.scale as f64) / soft).clamp(0.0, 1.0) as f32;
                let s = &mut o[j * r.w as usize + i];
                *s = s.max(v);
            }
        }
    }
}
