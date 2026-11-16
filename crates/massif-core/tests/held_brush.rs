//! A brush held in place a long time edits the land the same on every tile:
//! the same land half a tile further, under the same brush half a tile
//! further, comes out the same, half a tile further, though the tile borders
//! then cut the brush in other places.

use massif_core::coords::{BlockRect, TilePos, local_index};
use massif_core::filter::Filter;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::history::History;
use massif_core::ops::{self, FlattenMode, Surface};
use massif_core::{Brush, BrushMask, BrushShape, Profile, World};
use std::collections::HashMap;
use std::sync::Arc;

/// Half a tile.
const SHIFT: i32 = 64;
const RADIUS: f32 = 240.0;

/// The heights of the ground, tile by tile.
fn heights(w: &World) -> HashMap<TilePos, Vec<f32>> {
    let ground = w.dims[0].ground();
    ground.positions().map(|p| (p, ground.view(p).expect("listed").top.dense().into_owned())).collect()
}

fn height(h: &HashMap<TilePos, Vec<f32>>, x: i32, z: i32) -> f32 {
    h[&TilePos::containing(x, z)][local_index(x, z)]
}

/// The map, and the same land moved `SHIFT` blocks east and south.
fn worlds() -> (World, World) {
    let a = generate(&GenParams {
        tiles_x: 8,
        tiles_z: 8,
        landform: Landform::Mountains,
        seed: 7,
        ..GenParams::default()
    });
    let ha = heights(&a);
    let mut b = a.clone();
    let ground = b.dims[0].ground_mut();
    let positions: Vec<TilePos> = ground.positions().collect();
    for pos in positions {
        let tile = ground.tile_mut(pos).expect("listed");
        let rect = pos.rect();
        for z in rect.z0..rect.z1 {
            for x in rect.x0..rect.x1 {
                let (sx, sz) = (x - SHIFT, z - SHIFT);
                if ha.contains_key(&TilePos::containing(sx, sz)) {
                    tile.top.set(local_index(x, z), height(&ha, sx, sz));
                }
            }
        }
    }
    (a, b)
}

/// What a tool does at one dab.
type Dab = dyn Fn(&mut massif_core::Edit, &Brush, f32, f32);

/// Hold `dab` at `(cx, cz)` for `frames` frames, as one stroke.
fn hold(w: &mut World, cx: f32, cz: f32, frames: usize, dab: &dyn Fn(&mut massif_core::Edit, f32, f32)) {
    let mut history = History::default();
    history.begin("held", 0);
    let mut edit = history.edit(w, 0);
    for _ in 0..frames {
        dab(&mut edit, cx, cz);
    }
    history.commit();
}

#[test]
fn a_held_brush_edits_every_tile_alike() {
    let sine = Brush { strength: 0.8, ..Brush::symmetric(BrushShape::Circle, Profile::Sine, RADIUS) };
    let image = {
        let (w, h) = (61, 47);
        let data = (0..w * h).map(|i| ((i * 97 + (i / w) * 31) % 256) as u8).collect();
        Brush { mask: Some(Arc::new(BrushMask::new("held", w, h, data))), rotation: 20.0, ..sine.clone() }
    };
    let slopes = Brush { filter: Some(Arc::new(Filter { slope_above: true, slope: 20.0, ..Filter::default() })), ..sine.clone() };
    let hollows = Brush { filter: Some(Arc::new(Filter { hollows: true, depth: 2.0, radius: 8, ..Filter::default() })), ..sine.clone() };
    let above = Brush { filter: Some(Arc::new(Filter { above: Some(90), fade: true, ..Filter::default() })), ..sine.clone() };
    let tools: Vec<(&str, Brush, Box<Dab>)> = vec![
        ("raise", sine.clone(), Box::new(|e, b, x, z| ops::raise(e, b, x, z, 0.4, Surface::Top))),
        ("lower", sine.clone(), Box::new(|e, b, x, z| ops::raise(e, b, x, z, -0.4, Surface::Top))),
        ("smooth", Brush { strength: 0.13, ..sine.clone() }, Box::new(|e, b, x, z| ops::smooth(e, b, x, z, Surface::Top))),
        ("flatten", Brush { strength: 0.13, ..sine.clone() }, Box::new(|e, b, x, z| ops::flatten_with(e, b, x, z, 100.0, Surface::Top, FlattenMode::Both, None))),
        ("flatten down", Brush { strength: 0.13, ..sine.clone() }, Box::new(|e, b, x, z| ops::flatten_with(e, b, x, z, 100.0, Surface::Top, FlattenMode::LowerOnly, None))),
        ("image raise", image, Box::new(|e, b, x, z| ops::raise(e, b, x, z, 0.4, Surface::Top))),
        ("raise on slopes", slopes, Box::new(|e, b, x, z| ops::raise(e, b, x, z, 0.4, Surface::Top))),
        ("raise in hollows", hollows, Box::new(|e, b, x, z| ops::raise(e, b, x, z, 0.4, Surface::Top))),
        ("lower above 90", above, Box::new(|e, b, x, z| ops::raise(e, b, x, z, -0.4, Surface::Top))),
    ];
    let (cx, cz) = (-60.3f32, 37.6f32);
    let (a0, b0) = worlds();
    let before = heights(&a0);
    for (name, brush, dab) in &tools {
        let (mut a, mut b) = (a0.clone(), b0.clone());
        hold(&mut a, cx, cz, 40, &|e, x, z| dab(e, brush, x, z));
        hold(&mut b, cx + SHIFT as f32, cz + SHIFT as f32, 40, &|e, x, z| dab(e, brush, x, z));
        let (ha, hb) = (heights(&a), heights(&b));
        let (mut changed, mut off, mut most) = (0, 0, 0f32);
        let r = BlockRect::around(cx, cz, RADIUS + 2.0);
        for (x, z) in (r.z0..r.z1).flat_map(|z| (r.x0..r.x1).map(move |x| (x, z))) {
            let (ha, hb) = (height(&ha, x, z), height(&hb, x + SHIFT, z + SHIFT));
            off += (ha != hb) as usize;
            most = most.max((ha - hb).abs());
            changed += (ha != height(&before, x, z)) as usize;
        }
        eprintln!("{name}: {changed} columns changed, {off} differ half a tile further, by {most} at most");
        assert!(changed > 100, "{name} changed {changed} columns");
        // Smooth's running sums start where each tile's part of the brush
        // does: a rounding, a 256th of a block now and then.
        if name.as_bytes()[0] == b's' {
            assert!(most <= 1.0 / 256.0 && off * 100 < changed, "{name}: {off} columns differ, by {most} at most");
        } else {
            assert_eq!(off, 0, "{name}: {off} columns differ, by {most} at most");
        }
    }
}
