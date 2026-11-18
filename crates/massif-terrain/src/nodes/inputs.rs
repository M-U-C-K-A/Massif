//! Inputs: the map as it is, heightmaps, images (climate maps, painted
//! masks, colour maps).

use crate::kernel::{Ctx, Image, MapLayer};
use crate::params::Params;
use crate::registry::{Choice, Family, Kernel, NodeSpec, ParamSpec, PortKind, PortSpec, Reads, Section, Unit};

/// The node types of this family, in the library's order.
pub static NODES: &[&NodeSpec] = &[&MAP, &HEIGHTMAP, &IMAGE];

// ------------------------------------------------------------------ map --

pub static MAP: NodeSpec = NodeSpec {
    id: "input.map",
    label: "Map",
    family: Family::Input,
    summary: "The map as it is: its ground, its water, its terrains and biomes, read again whenever the map changes.",
    help: "Start from the map you painted or imported: run erosion over it, put rivers in its valleys, colour it from its relief, keep its terrains where a mask says. Water is 1 where water lies over the ground, Level the water's surface. The levels the graph writes itself are left out.",
    keywords: &["map", "world", "existing", "current", "ground", "input", "worldpainter"],
    inputs: &[],
    outputs: &[
        PortSpec::new("height", "Height", PortKind::Height, "The ground's height."),
        PortSpec::new("water", "Water", PortKind::Mask, "1 where water lies over the ground."),
        PortSpec::new("level", "Level", PortKind::Height, "The water's surface where there is water, the ground's elsewhere."),
        PortSpec::new("terrain", "Terrain", PortKind::Terrain, "The terrain of each column."),
        PortSpec::new("biome", "Biome", PortKind::Biome, "The biome of each column."),
        PortSpec::new("color", "Colour", PortKind::Color, "The terrains' colours."),
    ],
    params: &[],
    kernel: Kernel::Point { cpu: map_cpu, gpu: None },
    reads: Reads::Map,
    ..NodeSpec::BASE
};

fn map_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let (space, lod, rect) = (ctx.space, ctx.lod, ctx.rect);
    let n = rect.area();
    if !ctx.env.read_map(MapLayer::Height, space, lod, rect, &mut out[0]) {
        // No map: a flat ground at the sea level.
        out[0].fill(space.sea_level);
        out[1].fill(0.0);
        out[2].fill(space.sea_level);
        out[3].fill(0.0);
        out[4].fill(0.0);
        out[5].fill(0.5);
        return;
    }
    if !ctx.env.read_map(MapLayer::Water, space, lod, rect, &mut out[2]) {
        let (ground, rest) = out.split_at_mut(1);
        rest[1].copy_from_slice(&ground[0]);
    }
    if let [ground, water, level, ..] = out {
        for ((w, l), g) in water.iter_mut().zip(level.iter()).zip(ground.iter()) {
            *w = if *l > *g + 0.5 { 1.0 } else { 0.0 };
        }
    }
    if !ctx.env.read_map(MapLayer::Terrain, space, lod, rect, &mut out[3]) {
        out[3].fill(0.0);
    }
    if !ctx.env.read_map(MapLayer::Biome, space, lod, rect, &mut out[4]) {
        out[4].fill(0.0);
    }
    for k in 0..n {
        let c = ctx.env.terrain_color(out[3][k] as u16).unwrap_or([0.4, 0.4, 0.4]);
        for q in 0..3 {
            out[5][q * n + k] = c[q];
        }
    }
}

// --------------------------------------------------------------- images --

const FITS: &[Choice] = &[
    Choice { value: "stretch", label: "Stretch", help: "The image fills the rectangle, stretched if its shape differs." },
    Choice { value: "contain", label: "Contain", help: "The whole image within the rectangle, its shape kept." },
    Choice { value: "cover", label: "Cover", help: "The rectangle filled, the image's shape kept, its edges cut." },
];

const fn placement(section: Section) -> [ParamSpec; 6] {
    [
        ParamSpec::float("x", "X", Unit::Fraction, 0.0, -4.0, 4.0, "Where the image's west edge lies, as a share of the terrain.").soft(-0.5, 1.5).section(section),
        ParamSpec::float("z", "Z", Unit::Fraction, 0.0, -4.0, 4.0, "Where the image's north edge lies, as a share of the terrain.").soft(-0.5, 1.5).section(section),
        ParamSpec::float("width", "Width", Unit::Fraction, 1.0, 0.001, 16.0, "How wide the image is, as a share of the terrain.").soft(0.01, 2.0).section(section),
        ParamSpec::float("depth", "Depth", Unit::Fraction, 1.0, 0.001, 16.0, "How deep (north to south) the image is, as a share of the terrain.").soft(0.01, 2.0).section(section),
        ParamSpec::float("rotation", "Rotation", Unit::Degrees, 0.0, -180.0, 180.0, "How far the image is turned around its middle, in degrees.").section(section),
        ParamSpec::choice("fit", "Fit", FITS, "stretch", "How the image's shape meets the rectangle's.").section(section),
    ]
}

/// Where a world position (blocks) falls on an image laid over the
/// rectangle the settings say: its pixel coordinates (continuous, pixel
/// centres on whole numbers), and how far inside the rectangle it is, in
/// blocks (negative outside).
struct Laid {
    x0: f64,
    z0: f64,
    w: f64,
    d: f64,
    cx: f64,
    cz: f64,
    sin: f64,
    cos: f64,
    pw: f64,
    ph: f64,
}

impl Laid {
    fn new(p: &Params, size: f64, img: &Image) -> Self {
        let (mut x0, mut z0, mut w, mut d) = (p.num("x") * size, p.num("z") * size, p.num("width") * size, p.num("depth") * size);
        let (pw, ph) = (img.width as f64, img.height as f64);
        let aspect = pw / ph.max(1.0);
        let fit = p.choice("fit");
        if fit != "stretch" {
            let (cx, cz) = (x0 + w * 0.5, z0 + d * 0.5);
            let wider = w / d > aspect;
            let (nw, nd) = if (fit == "contain") == wider { (d * aspect, d) } else { (w, w / aspect) };
            (w, d) = (nw, nd);
            (x0, z0) = (cx - w * 0.5, cz - d * 0.5);
        }
        let (sin, cos) = (-p.num("rotation").to_radians()).sin_cos();
        Self { x0, z0, w, d, cx: x0 + w * 0.5, cz: z0 + d * 0.5, sin, cos, pw, ph }
    }

    fn at(&self, x: f64, z: f64) -> (f64, f64, f64) {
        let (dx, dz) = (x - self.cx, z - self.cz);
        let (rx, rz) = (self.cx + self.cos * dx - self.sin * dz, self.cz + self.sin * dx + self.cos * dz);
        let (u, v) = ((rx - self.x0) / self.w, (rz - self.z0) / self.d);
        let inside = (rx - self.x0).min(self.x0 + self.w - rx).min(rz - self.z0).min(self.z0 + self.d - rz);
        (u * self.pw - 0.5, v * self.ph - 0.5, inside)
    }
}

/// Channel `c` of `img` at pixel coordinates `(u, v)`, by clamped
/// Catmull-Rom; tiled, or held at the edges.
fn sample(img: &Image, c: usize, u: f64, v: f64, tile: bool) -> f32 {
    let (w, h) = (img.width as i64, img.height as i64);
    let ch = img.channels as usize;
    let px = |x: i64, z: i64| {
        let (x, z) = if tile { (x.rem_euclid(w), z.rem_euclid(h)) } else { (x.clamp(0, w - 1), z.clamp(0, h - 1)) };
        img.data[(z * w + x) as usize * ch + c.min(ch - 1)]
    };
    let (fx, fz) = (u.floor(), v.floor());
    let (x, z) = (fx as i64, fz as i64);
    let (tx, tz) = ((u - fx) as f32, (v - fz) as f32);
    let wt = |t: f32| {
        let (t2, t3) = (t * t, t * t * t);
        [-0.5 * t3 + t2 - 0.5 * t, 1.5 * t3 - 2.5 * t2 + 1.0, -1.5 * t3 + 2.0 * t2 + 0.5 * t, 0.5 * t3 - 0.5 * t2]
    };
    let (wx, wz) = (wt(tx), wt(tz));
    let mut sum = 0.0;
    for (b, wzb) in wz.iter().enumerate() {
        let row = z - 1 + b as i64;
        sum += wzb * (wx[0] * px(x - 1, row) + wx[1] * px(x, row) + wx[2] * px(x + 1, row) + wx[3] * px(x + 2, row));
    }
    let corners = [px(x, z), px(x + 1, z), px(x, z + 1), px(x + 1, z + 1)];
    let lo = corners.iter().copied().fold(f32::MAX, f32::min);
    let hi = corners.iter().copied().fold(f32::MIN, f32::max);
    sum.clamp(lo, hi)
}

/// An image's brightness at a pixel (its one channel for a grey image).
fn luminance(img: &Image, u: f64, v: f64, tile: bool) -> f32 {
    if img.channels < 3 {
        return sample(img, 0, u, v, tile);
    }
    0.2126 * sample(img, 0, u, v, tile) + 0.7152 * sample(img, 1, u, v, tile) + 0.0722 * sample(img, 2, u, v, tile)
}

const HEIGHTMAP_PARAMS: [ParamSpec; 12] = {
    let [x, z, width, depth, rotation, fit] = placement(Section::Shape);
    [
        ParamSpec::file("file", "File", &["png", "tif", "tiff", "jpg", "jpeg", "bmp", "raw", "r16"], "The heightmap: an image (16-bit greys keep their precision) or a square RAW of 16-bit samples."),
        ParamSpec::float("low", "Low", Unit::Blocks, 40.0, -2048.0, 4096.0, "The height black stands for, in blocks.").soft(-64.0, 320.0),
        ParamSpec::float("high", "High", Unit::Blocks, 200.0, -2048.0, 4096.0, "The height white stands for, in blocks.").soft(-64.0, 320.0),
        x,
        z,
        width,
        depth,
        rotation,
        fit,
        ParamSpec::float("edge", "Edge fade", Unit::Blocks, 0.0, 0.0, 16000.0, "How far inside its edges the image's mask fades out, in blocks.").soft(0.0, 1000.0).section(Section::Detail),
        ParamSpec::flag("tile", "Tile", false, "Repeat the image beyond its rectangle.").section(Section::Detail),
        ParamSpec::flag("invert", "Invert", false, "Read white as low and black as high.").section(Section::Detail),
    ]
};

pub static HEIGHTMAP: NodeSpec = NodeSpec {
    id: "input.heightmap",
    label: "Heightmap",
    family: Family::Input,
    summary: "A heightmap image turned into relief: black at Low, white at High, laid over a rectangle of the terrain.",
    help: "Choose the file (PNG, TIFF, JPEG, BMP or a square RAW of 16-bit samples; 16-bit greys keep their precision), then where it lies: X, Z, Width and Depth as shares of the terrain, Rotation, and how its shape fits. Outside it, the relief is Low (or the image repeats when Tile is on); the Mask output says where the image lies, fading over Edge fade, to blend several heightmaps or a heightmap into a generated relief.",
    keywords: &["heightmap", "dem", "image", "import", "png", "tiff", "raw", "elevation", "file"],
    inputs: &[],
    outputs: &[PortSpec::new("height", "Height", PortKind::Height, "The relief the image gives."), PortSpec::new("mask", "Mask", PortKind::Mask, "1 where the image lies, fading at its edges.")],
    params: &HEIGHTMAP_PARAMS,
    kernel: Kernel::Point { cpu: heightmap_cpu, gpu: None },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn heightmap_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let (low, high) = (p.f("low"), p.f("high"));
    let Some(img) = ctx.env.image(p.text("file")) else {
        out[0].fill(low);
        out[1].fill(0.0);
        return;
    };
    let laid = Laid::new(p, ctx.space.size as f64, &img);
    let (tile, invert, edge) = (p.flag("tile"), p.flag("invert"), p.num("edge"));
    let w = ctx.rect.w as usize;
    let (heights, masks) = out.split_at_mut(1);
    for (k, (h, m)) in heights[0].iter_mut().zip(masks[0].iter_mut()).enumerate() {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let (u, v, inside) = laid.at(x, z);
        let mask = if tile { 1.0 } else if edge > 0.0 { (inside / edge).clamp(0.0, 1.0) as f32 } else if inside >= 0.0 { 1.0 } else { 0.0 };
        let value = if tile || inside >= 0.0 { luminance(&img, u, v, tile) } else { 0.0 };
        let value = if invert { 1.0 - value } else { value };
        *h = if tile || inside >= 0.0 { low + (high - low) * value } else { low };
        *m = mask;
    }
}

const CHANNELS: &[Choice] = &[
    Choice { value: "luminance", label: "Brightness", help: "The image's brightness." },
    Choice { value: "red", label: "Red", help: "The red channel." },
    Choice { value: "green", label: "Green", help: "The green channel." },
    Choice { value: "blue", label: "Blue", help: "The blue channel." },
    Choice { value: "alpha", label: "Alpha", help: "The alpha (opacity) channel." },
];

const IMAGE_PARAMS: [ParamSpec; 13] = {
    let [x, z, width, depth, rotation, fit] = placement(Section::Shape);
    [
        ParamSpec::file("file", "File", &["png", "tif", "tiff", "jpg", "jpeg", "bmp"], "The image: a climate map, a painted mask, a colour map."),
        ParamSpec::choice("channel", "Channel", CHANNELS, "luminance", "Which of the image's values makes the mask."),
        ParamSpec::float("low", "Low", Unit::Ratio, 0.0, -1000.0, 1000.0, "The value read as 0.").soft(0.0, 1.0),
        ParamSpec::float("high", "High", Unit::Ratio, 1.0, -1000.0, 1000.0, "The value read as 1.").soft(0.0, 1.0),
        ParamSpec::flag("invert", "Invert", false, "Turn the mask over."),
        x,
        z,
        width,
        depth,
        rotation,
        fit,
        ParamSpec::flag("tile", "Tile", false, "Repeat the image beyond its rectangle.").section(Section::Detail),
        ParamSpec::float("outside", "Outside", Unit::Ratio, 0.0, 0.0, 1.0, "The mask's value outside the image.").section(Section::Detail),
    ]
};

pub static IMAGE: NodeSpec = NodeSpec {
    id: "input.image",
    label: "Image",
    family: Family::Input,
    summary: "An image read as a mask (one of its channels, over a range) and as colours: a temperature or rainfall map, a painted mask, a satellite colour map.",
    help: "Choose the file and where it lies, as for a heightmap. Channel picks which of its values becomes the mask, Low and High the values read as 0 and 1 (a temperature map stored from −30 to 45 °C reads with Low −30 and High 45 into a temperature mask). The Colour output gives its colours, for the Map Output's Albedo.",
    keywords: &["image", "picture", "climate map", "mask", "painted", "satellite", "colour map", "file", "texture"],
    inputs: &[],
    outputs: &[PortSpec::new("mask", "Mask", PortKind::Mask, "The channel read, from 0 to 1."), PortSpec::new("color", "Colour", PortKind::Color, "The image's colours.")],
    params: &IMAGE_PARAMS,
    kernel: Kernel::Point { cpu: image_cpu, gpu: None },
    reads: Reads::Files,
    ..NodeSpec::BASE
};

fn image_cpu(ctx: &Ctx, _: &[Option<&[f32]>], out: &mut [Vec<f32>]) {
    let p = ctx.params;
    let n = ctx.rect.area();
    let outside = p.f("outside");
    let Some(img) = ctx.env.image(p.text("file")) else {
        out[0].fill(outside);
        out[1].fill(0.5);
        return;
    };
    let laid = Laid::new(p, ctx.space.size as f64, &img);
    let tile = p.flag("tile");
    let (lo, hi) = (p.f("low"), p.f("high"));
    let span = if (hi - lo).abs() < 1e-9 { 1e-9 } else { hi - lo };
    let channel = p.choice("channel");
    let invert = p.flag("invert");
    let grey = img.channels < 3;
    let w = ctx.rect.w as usize;
    for k in 0..n {
        let (x, z) = ctx.world(ctx.rect.x + (k % w) as i32, ctx.rect.z + (k / w) as i32);
        let (u, v, inside) = laid.at(x, z);
        if !tile && inside < 0.0 {
            out[0][k] = outside;
            for q in 0..3 {
                out[1][q * n + k] = 0.5;
            }
            continue;
        }
        let rgb: [f32; 3] = if grey {
            [sample(&img, 0, u, v, tile); 3]
        } else {
            [sample(&img, 0, u, v, tile), sample(&img, 1, u, v, tile), sample(&img, 2, u, v, tile)]
        };
        let value = match (channel, grey) {
            (_, true) | ("luminance", false) => if grey { rgb[0] } else { 0.2126 * rgb[0] + 0.7152 * rgb[1] + 0.0722 * rgb[2] },
            ("red", _) => rgb[0],
            ("green", _) => rgb[1],
            ("blue", _) => rgb[2],
            _ => if img.channels >= 4 { sample(&img, 3, u, v, tile) } else { 1.0 },
        };
        let m = ((value - lo) / span).clamp(0.0, 1.0);
        out[0][k] = if invert { 1.0 - m } else { m };
        // Image files hold sRGB.
        let lin = crate::params::linear(rgb);
        for q in 0..3 {
            out[1][q * n + k] = lin[q];
        }
    }
}
