//! Shapes: landforms with real shape controls — mountains by type, ranges
//! along a drawn spine, peaks placed by hand, massifs, coasts and islands —
//! and, to come, plateaus, faults, canyons, valleys, dunes and karst.
//!
//! Shapes give heights in blocks over a Base, so that they combine as they
//! are (Max sets them side by side, Add raises one on another with Base 0).
//! Positions are fractions of the terrain and sizes in blocks: a graph keeps
//! its layout on a larger terrain, which then holds more of them.

mod coasts;
mod landforms;
mod mountains;
mod valleys;

use crate::kernel::Ctx;
use crate::noise::Fbm;
use crate::registry::NodeSpec;

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[
    &mountains::ALPINE,
    &mountains::ROUNDED,
    &mountains::RANGE,
    &mountains::PEAKS,
    &landforms::VOLCANO,
    &landforms::MESA,
    &valleys::CANYON,
    &valleys::GLACIAL,
    &valleys::FAULT,
    &landforms::DUNES,
    &landforms::KARST,
    &landforms::CRATERS,
    &coasts::ISLAND,
    &coasts::ARCHIPELAGO,
    &coasts::COASTLINE,
];

pub(crate) fn smoothstep(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// The relief laid over a shape: ridged crests mixed with eroded valleys
/// by `sharpness`, from features `detail` blocks wide down to a few blocks.
pub(crate) struct Relief {
    ridged: Fbm,
    eroded: Fbm,
    sharpness: f32,
}

impl Relief {
    /// Octaves down to `finest` blocks: the samples' spacing, so that a
    /// coarse level does not alias what only a finer one can hold.
    pub fn new(detail: f64, roughness: f64, sharpness: f64, finest: f64, seed: u32) -> Self {
        let gain = 0.35 + 0.3 * roughness;
        let finest = finest.max(2.0);
        Self {
            ridged: Fbm::down_to(detail, finest, gain, seed ^ 0x51ed_270b),
            eroded: Fbm::down_to(detail * 1.6, finest, gain, seed ^ 0x2545_f491),
            sharpness: sharpness as f32,
        }
    }

    /// From 0 to 1.
    pub fn at(&self, x: f64, z: f64) -> f32 {
        let r = if self.sharpness > 0.0 { self.ridged.ridged(x, z, self.sharpness) } else { 0.0 };
        let e = if self.sharpness < 1.0 { self.eroded.eroded(x, z, 1.5) } else { 0.0 };
        e + (r - e) * self.sharpness
    }
}

/// Moves positions by a smooth noise: lines wander, nothing is straight.
pub(crate) struct Warp {
    x: Fbm,
    z: Fbm,
    amount: f64,
}

impl Warp {
    pub fn new(scale: f64, amount: f64, seed: u32) -> Self {
        Self { x: Fbm::new(scale, 4, 0.5, seed ^ 0x68e3_1da4), z: Fbm::new(scale, 4, 0.5, seed ^ 0xb529_7a4d), amount }
    }

    pub fn at(&self, x: f64, z: f64) -> (f64, f64) {
        if self.amount <= 0.0 {
            return (x, z);
        }
        (x + self.amount * self.x.at(x, z) as f64, z + self.amount * self.z.at(x, z) as f64)
    }
}

/// Each sample of the kernel's tile, with its index and its position in
/// blocks from the terrain's corner.
pub(crate) fn samples<'a>(ctx: &'a Ctx<'a>) -> impl Iterator<Item = (usize, f64, f64)> + 'a {
    let w = ctx.rect.w as usize;
    (0..ctx.rect.area()).map(move |k| {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        (k, x, z)
    })
}

/// The finest relief a level can hold: twice its samples' spacing.
pub(crate) fn finest(ctx: &Ctx) -> f64 {
    (2.0 * ctx.spacing()).max(3.0)
}

/// The corners of the kernel's tile, in blocks.
pub(crate) fn bounds(ctx: &Ctx) -> (f64, f64, f64, f64) {
    let (x0, z0) = ctx.world(ctx.rect.x, ctx.rect.z);
    let (x1, z1) = ctx.world(ctx.rect.x1(), ctx.rect.z1());
    (x0, z0, x1, z1)
}
