//! Volumes: fields over the world's blocks, made brick by brick (see
//! [`Kernel::Volume`]). Negative inside matter, positive in the air, the
//! distance to the surface in blocks where it can be: the field caves,
//! arches and overhangs are cut and added in.

use crate::kernel::{BRICK, Brick, Ctx, Grid};
use crate::registry::{Family, Kernel, NodeSpec, PortKind, PortSpec};

pub const NODES: &[&NodeSpec] = &[&FROM_HEIGHT];

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
