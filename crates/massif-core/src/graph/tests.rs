use super::*;

fn extent(n: usize) -> Extent {
    Extent {
        x0: -512.0,
        z0: -512.0,
        size: 1024.0,
        n,
    }
}

fn eval_one(g: &Graph, id: NodeId, n: usize) -> Arc<Field> {
    let e = g.evaluate(&extent(n), &[id], true, &AtomicBool::new(false)).unwrap();
    e.field(id, 0).expect("a field").clone()
}

fn colour_of(g: &Graph, id: NodeId, n: usize) -> Arc<ColorField> {
    let e = g.evaluate(&extent(n), &[id], true, &AtomicBool::new(false)).unwrap();
    e.values[&(id, 0)].color().expect("colours").clone()
}

/// Feed every input of `id` with something of the right type.
fn wire_inputs(g: &mut Graph, id: NodeId) {
    let inputs = g.node(id).unwrap().kind.inputs();
    for (k, (_, t)) in inputs.iter().enumerate() {
        let src = match t {
            PortType::Height => g.add(
                NodeKind::Noise {
                    style: NoiseStyle::Ridged,
                    scale: 300.0,
                    octaves: 5,
                    roughness: 0.5,
                    height: 150.0,
                    base: 40.0,
                    seed: 3,
                },
                [0.0; 2],
            ),
            PortType::Mask => g.add(
                NodeKind::Radial {
                    inner: 0.2,
                    outer: 0.8,
                    height: 1.0,
                    base: 0.0,
                },
                [0.0; 2],
            ),
            PortType::Color => g.add(NodeKind::SolidColor { color: [120, 140, 90] }, [0.0; 2]),
        };
        assert!(g.connect(src, 0, id, k as u8));
    }
}

/// The tiles of a world's ground.
fn map_tiles(world: &crate::world::World) -> Vec<TilePos> {
    world.dims[0].ground().positions().collect()
}

/// How the tests write a build to the ground: its own tiles, made none.
fn write(tiles: &[TilePos]) -> GroundWrite<'_> {
    GroundWrite {
        tiles,
        create: false,
        map: None,
        sea: 62,
        blank: TerrainId::ROCK,
    }
}

#[test]
fn every_node_of_the_catalogue_runs() {
    // (The Map node needs a map: it has its own test.)
    for kind in NodeKind::catalogue().into_iter().filter(|k| *k != NodeKind::MapHeight) {
        let mut g = Graph::default();
        let id = g.add(kind.clone(), [0.0; 2]);
        wire_inputs(&mut g, id);
        let e = g.evaluate(&extent(48), &[id], true, &AtomicBool::new(false));
        let e = e.unwrap_or_else(|err| panic!("{}: {err}", kind.label()));
        assert_eq!(e.values.keys().filter(|k| k.0 == id).count(), kind.outputs().len(), "{}", kind.label());
        for (k, (name, t)) in kind.outputs().iter().enumerate() {
            let v = &e.values[&(id, k as u8)];
            match t {
                PortType::Color => {
                    let c = v.color().unwrap_or_else(|| panic!("{} {name} is a colour", kind.label()));
                    assert_eq!(c.data.len(), c.n * c.n, "{}", kind.label());
                }
                PortType::Mask => {
                    let f = v.field().unwrap();
                    let (lo, hi) = f.range();
                    assert!(lo >= -1e-4 && hi <= 1.0 + 1e-4, "{} {name}: {lo}..{hi}", kind.label());
                }
                PortType::Height => {
                    let f = v.field().unwrap();
                    assert!(f.data.iter().all(|x| x.is_finite()), "{} {name}", kind.label());
                }
            }
        }
        assert!(!kind.description().is_empty());
    }
}

#[test]
fn generators_stay_in_their_range_and_are_deterministic() {
    for kind in NodeKind::catalogue()
        .into_iter()
        .filter(|k| matches!(k.family(), Family::Generator | Family::Landform) && *k != NodeKind::MapHeight)
    {
        let mut g = Graph::default();
        let id = g.add(kind.clone(), [0.0; 2]);
        let f = eval_one(&g, id, 64);
        let (lo, hi) = f.range();
        let (base, height) = match kind {
            NodeKind::Noise { base, height, .. }
            | NodeKind::Cells { base, height, .. }
            | NodeKind::Radial { base, height, .. }
            | NodeKind::LinearGradient { base, height, .. }
            | NodeKind::Shape { base, height, .. }
            | NodeKind::Mountain { base, height, .. }
            | NodeKind::MountainRange { base, height, .. }
            | NodeKind::Volcano { base, height, .. }
            | NodeKind::Dunes { base, height, .. }
            | NodeKind::Mesas { base, height, .. }
            | NodeKind::Island { base, height, .. } => (base, height),
            NodeKind::Constant { value } => (value, 0.0),
            NodeKind::Rocks { height, .. } => (-height / 2.0, height),
            NodeKind::Crater { base, depth, rim, peak, .. } => (base - depth, depth + rim + peak),
            NodeKind::CraterField { base, max_radius, depth, .. } => (base - max_radius * depth * 4.0, max_radius * depth * 8.0),
            NodeKind::Canyon { base, depth, .. } => (base - depth - 5.0, depth + 10.0),
            // Their tops: from the lowest island's edge to the highest crag.
            NodeKind::FloatingIslands { altitude, spread, size, .. } => (altitude - spread, 2.0 * spread + size * 1.5),
            // From the trenches to the highest ranges.
            NodeKind::Tectonic { relief, sea, .. } => (sea - 0.9 * relief, 2.2 * relief),
            _ => unreachable!("{}", kind.label()),
        };
        let slack = height.abs() * 0.25 + 1e-3;
        assert!(lo >= base - slack && hi <= base + height + slack, "{}: {lo}..{hi}", kind.label());
        assert_eq!(eval_one(&g, id, 64).data, f.data, "{}", kind.label());
    }
}

#[test]
fn landforms_have_their_shape() {
    let one = |kind: NodeKind| {
        let mut g = Graph::default();
        let id = g.add(kind, [0.0; 2]);
        eval_one(&g, id, 96)
    };
    // A mountain is highest near its middle, low at the edges of the map.
    let m = one(NodeKind::Mountain {
        style: MountainStyle::Alpine,
        x: 0.5,
        z: 0.5,
        radius: 400.0,
        height: 200.0,
        base: 40.0,
        detail: 0.5,
        seed: 1,
    });
    let centre: f32 = (44..52).flat_map(|j| (44..52).map(move |i| (i, j))).map(|(i, j)| m.at(i, j)).sum::<f32>() / 64.0;
    assert!(centre > 120.0 && m.at(1, 1) < 45.0, "mountain: {centre}, edge {}", m.at(1, 1));
    // A crater is a bowl under the plain with a rim above it.
    let c = one(NodeKind::Crater {
        x: 0.5,
        z: 0.5,
        radius: 300.0,
        depth: 60.0,
        rim: 20.0,
        peak: 0.0,
        base: 100.0,
        seed: 1,
    });
    let rim = (0..96).map(|i| c.at(i, 48)).fold(f32::MIN, f32::max);
    assert!(c.at(48, 48) < 50.0 && rim > 110.0, "crater floor {} rim {rim}", c.at(48, 48));
    // A volcano's crater sits below its rim.
    let v = one(NodeKind::Volcano {
        x: 0.5,
        z: 0.5,
        radius: 450.0,
        height: 200.0,
        base: 40.0,
        crater: 0.2,
        depth: 50.0,
        seed: 3,
    });
    let peak = (0..96).map(|i| v.at(i, 48)).fold(f32::MIN, f32::max);
    assert!(v.at(48, 48) < peak - 30.0 && peak > 180.0, "volcano centre {} peak {peak}", v.at(48, 48));
    // A canyon is low along its middle and high on the plateau.
    let k = one(NodeKind::Canyon {
        angle: 0.0,
        width: 200.0,
        depth: 80.0,
        meander: 0.0,
        steps: 4,
        base: 150.0,
        seed: 1,
    });
    assert!(k.at(48, 48) < 80.0 && k.at(5, 48) > 140.0, "canyon floor {} plateau {}", k.at(48, 48), k.at(5, 48));
    // Dunes rise and fall over and over across the wind.
    let d = one(NodeKind::Dunes {
        angle: 90.0,
        wavelength: 100.0,
        height: 20.0,
        base: 60.0,
        sharpness: 0.7,
        seed: 2,
    });
    let row: Vec<f32> = (0..96).map(|i| d.at(i, 48)).collect();
    let crests = row.windows(3).filter(|w| w[1] > w[0] && w[1] >= w[2] && w[1] > 70.0).count();
    assert!(crests >= 5, "{crests} crests");
}

#[test]
fn simulations_fill_dig_and_cover() {
    let mut g = Graph::default();
    // A bowl: lakes fill it flat.
    let bowl = g.add(
        NodeKind::Radial {
            inner: 0.0,
            outer: 0.7,
            height: -60.0,
            base: 100.0,
        },
        [0.0; 2],
    );
    let lakes = g.add(NodeKind::Lakes { amount: 1.0, sea: NO_WATER }, [0.0; 2]);
    g.connect(bowl, 0, lakes, 0);
    let e = g.evaluate(&extent(64), &[lakes], true, &AtomicBool::new(false)).unwrap();
    let filled = e.field(lakes, 0).unwrap();
    let mask = e.field(lakes, 1).unwrap();
    assert!((filled.at(32, 32) - filled.at(20, 32)).abs() < 0.5, "a flat lake");
    assert!(mask.at(32, 32) > 0.9 && mask.at(0, 0) < 0.1);
    // Mountains: rivers run in their valleys, snow on their tops.
    let hills = g.add(
        NodeKind::Noise {
            style: NoiseStyle::Ridged,
            scale: 400.0,
            octaves: 5,
            roughness: 0.5,
            height: 160.0,
            base: 60.0,
            seed: 9,
        },
        [0.0; 2],
    );
    let rivers = g.add(
        NodeKind::Rivers {
            threshold: 0.4,
            depth: 6.0,
            width: 16.0,
            sea: NO_WATER,
            banks: 1.0,
            lakes: 0.5,
        },
        [0.0; 2],
    );
    g.connect(hills, 0, rivers, 0);
    let snow = g.add(
        NodeKind::Snow {
            line: 150.0,
            slope: 45.0,
            depth: 3.0,
            softness: 5.0,
        },
        [0.0; 2],
    );
    g.connect(hills, 0, snow, 0);
    let flow = g.add(NodeKind::Flow { spread: 0.5 }, [0.0; 2]);
    g.connect(hills, 0, flow, 0);
    let e = g.evaluate(&extent(96), &[rivers, snow, flow], true, &AtomicBool::new(false)).unwrap();
    let (before, dug, river) = (e.field(hills, 0).unwrap(), e.field(rivers, 0).unwrap(), e.field(rivers, 1).unwrap());
    let wet = river.data.iter().filter(|v| **v > 0.5).count();
    assert!(wet > 20 && wet < 96 * 96 / 3, "{wet} river samples");
    assert!(dug.data.iter().zip(&before.data).all(|(a, b)| a <= b), "rivers only dig");
    let snowy = e.field(snow, 1).unwrap();
    for c in 0..96 * 96 {
        // (The mask is softened over a sample, some ten blocks here.)
        if before.data[c] < 115.0 {
            assert!(snowy.data[c] < 0.05, "no snow under the line");
        }
    }
    assert!(snowy.range().1 > 0.9);
    // Water gathers in valleys: more flow where it is lower.
    let f = e.field(flow, 0).unwrap();
    let mean = |pick: &dyn Fn(f32) -> bool| {
        let v: Vec<f32> = (0..96 * 96).filter(|&c| pick(before.data[c])).map(|c| f.data[c]).collect();
        v.iter().sum::<f32>() / v.len().max(1) as f32
    };
    let (lo, hi) = before.range();
    let mid = (lo + hi) / 2.0;
    assert!(mean(&|h| h < mid) > mean(&|h| h > mid));
}

#[test]
fn colours_follow_their_gradients_and_masks() {
    let mut g = Graph::default();
    let ramp = g.add(
        NodeKind::LinearGradient {
            angle: 90.0,
            height: 100.0,
            base: 0.0,
        },
        [0.0; 2],
    );
    let grad = g.add(
        NodeKind::Gradient {
            preset: GradientPreset::Grey,
            auto_range: true,
            min: 0.0,
            max: 1.0,
            reverse: false,
            variation: 0.0,
            seed: 1,
        },
        [0.0; 2],
    );
    g.connect(ramp, 0, grad, 0);
    let c = colour_of(&g, grad, 32);
    // Black in the west, white in the east.
    assert!(c.at(0, 16)[0] < 0.01 && c.at(31, 16)[0] > 0.9);
    assert_eq!(color::encode(color::decode(200)), 200);
    // Auto colour: snow above the snow line, not below.
    let auto = g.add(
        NodeKind::AutoColor {
            palette: AutoPalette::Alpine,
            snow_line: 60.0,
            rock_slope: 80.0,
            seed: 1,
        },
        [0.0; 2],
    );
    g.connect(ramp, 0, auto, 0);
    let a = colour_of(&g, auto, 32);
    let east = a.at(31, 16);
    let west = a.at(0, 16);
    assert!(east.iter().all(|v| *v > 0.7), "snow: {east:?}");
    assert!(west[1] > west[2], "green valleys: {west:?}");
    // Mixing by a mask: A where it is 0, B where it is 1.
    let red = g.add(NodeKind::SolidColor { color: [255, 0, 0] }, [0.0; 2]);
    let blue = g.add(NodeKind::SolidColor { color: [0, 0, 255] }, [0.0; 2]);
    let half = g.add(
        NodeKind::Levels {
            in_min: 49.0,
            in_max: 51.0,
            out_min: 0.0,
            out_max: 1.0,
            gamma: 1.0,
        },
        [0.0; 2],
    );
    g.connect(ramp, 0, half, 0);
    assert_eq!(g.output_type(half, 0), PortType::Mask);
    let mix = g.add(NodeKind::MixColors, [0.0; 2]);
    g.connect(red, 0, mix, 0);
    g.connect(blue, 0, mix, 1);
    g.connect(half, 0, mix, 2);
    let m = colour_of(&g, mix, 32);
    assert!(m.at(2, 5)[0] > 0.9 && m.at(29, 5)[2] > 0.9);
    // A colour fed where a field is wanted counts as its brightness.
    let lvl = g.add(NodeKind::Blur { radius: 1.0 }, [0.0; 2]);
    g.connect(grad, 0, lvl, 0);
    let f = eval_one(&g, lvl, 32);
    assert!(f.at(31, 16) > f.at(0, 16));
}

#[test]
fn the_cache_computes_only_what_changed() {
    let mut g = Graph::default();
    let a = g.add(
        NodeKind::Noise {
            style: NoiseStyle::Fbm,
            scale: 300.0,
            octaves: 4,
            roughness: 0.5,
            height: 100.0,
            base: 0.0,
            seed: 1,
        },
        [0.0; 2],
    );
    let b = g.add(NodeKind::Blur { radius: 8.0 }, [0.0; 2]);
    let c = g.add(NodeKind::Terrace { steps: 4, strength: 0.5 }, [0.0; 2]);
    g.connect(a, 0, b, 0);
    g.connect(b, 0, c, 0);
    let mut cache = EvalCache::default();
    let ext = extent(64);
    let no = AtomicBool::new(false);
    let first = g.evaluate_cached(&ext, &[c], &mut cache, &no).unwrap();
    assert_eq!(cache.len(), 3);
    // Change the last node: the first two come from the cache.
    if let Some(n) = g.node_mut(c) {
        n.kind = NodeKind::Terrace { steps: 6, strength: 0.5 };
    }
    let second = g.evaluate_cached(&ext, &[c], &mut cache, &no).unwrap();
    let same = |id: NodeId| Arc::ptr_eq(first.field(id, 0).unwrap(), second.field(id, 0).unwrap());
    assert!(same(a) && same(b) && !same(c));
    // Change the first: everything is computed again.
    if let Some(n) = g.node_mut(a) {
        n.kind = NodeKind::Constant { value: 3.0 };
    }
    let third = g.evaluate_cached(&ext, &[c], &mut cache, &no).unwrap();
    assert!(!Arc::ptr_eq(second.field(b, 0).unwrap(), third.field(b, 0).unwrap()));
    // Another extent starts afresh.
    let other = g.evaluate_cached(&extent(32), &[c], &mut cache, &no).unwrap();
    assert_eq!(other.field(c, 0).unwrap().n, 32);
    // A node missing an input fails alone in a preview.
    let lone = g.add(NodeKind::Blur { radius: 2.0 }, [0.0; 2]);
    let partial = g.evaluate_cached(&ext, &[c, lone], &mut cache, &no).unwrap();
    assert!(partial.failed.contains_key(&lone) && partial.field(c, 0).is_some());
}

#[test]
fn links_refuse_loops_and_missing_inputs_are_reported() {
    let mut g = Graph::default();
    let a = g.add(NodeKind::Constant { value: 1.0 }, [0.0; 2]);
    let b = g.add(NodeKind::Blur { radius: 4.0 }, [0.0; 2]);
    let c = g.add(NodeKind::Blur { radius: 4.0 }, [0.0; 2]);
    assert!(g.connect(a, 0, b, 0));
    assert!(g.connect(b, 0, c, 0));
    assert!(!g.connect(c, 0, b, 0), "a loop");
    assert!(g.depends_on(c, a));
    let lone = g.add(NodeKind::Invert, [0.0; 2]);
    let err = g.evaluate(&extent(16), &[lone], false, &AtomicBool::new(false)).unwrap_err();
    assert_eq!(err, GraphError::MissingInput("Invert".into()));
    // Reconnecting an input replaces the old link.
    let d = g.add(NodeKind::Constant { value: 5.0 }, [0.0; 2]);
    assert!(g.connect(d, 0, c, 0));
    assert_eq!(g.input_of(c, 0), Some((d, 0)));
    g.remove(d);
    assert_eq!(g.input_of(c, 0), None);
}

#[test]
fn combiners_selectors_and_modifiers() {
    let mut g = Graph::default();
    let a = g.add(NodeKind::Constant { value: 10.0 }, [0.0; 2]);
    let b = g.add(NodeKind::Constant { value: 4.0 }, [0.0; 2]);
    for (op, want) in [
        (CombineOp::Add, 14.0),
        (CombineOp::Subtract, 6.0),
        (CombineOp::Multiply, 40.0),
        (CombineOp::Max, 10.0),
        (CombineOp::Min, 4.0),
        (CombineOp::Average, 7.0),
        (CombineOp::Difference, 6.0),
    ] {
        let c = g.add(NodeKind::Combine { op, weight: 1.0 }, [0.0; 2]);
        g.connect(a, 0, c, 0);
        g.connect(b, 0, c, 1);
        assert_eq!(eval_one(&g, c, 8).data[0], want, "{op:?}");
    }
    let lerp = g.add(
        NodeKind::Combine {
            op: CombineOp::Lerp,
            weight: 0.25,
        },
        [0.0; 2],
    );
    g.connect(a, 0, lerp, 0);
    g.connect(b, 0, lerp, 1);
    assert_eq!(eval_one(&g, lerp, 8).data[0], 8.5);
    let sel = g.add(
        NodeKind::SelectHeight {
            min: 5.0,
            max: 20.0,
            falloff: 1.0,
        },
        [0.0; 2],
    );
    g.connect(a, 0, sel, 0);
    assert_eq!(eval_one(&g, sel, 8).data[0], 1.0);
    g.connect(b, 0, sel, 0);
    assert_eq!(eval_one(&g, sel, 8).data[0], 0.0);
    let levels = g.add(
        NodeKind::Levels {
            in_min: 0.0,
            in_max: 20.0,
            out_min: 100.0,
            out_max: 200.0,
            gamma: 1.0,
        },
        [0.0; 2],
    );
    g.connect(a, 0, levels, 0);
    assert_eq!(eval_one(&g, levels, 8).data[0], 150.0);
    // A slope: steep where a radial gradient falls.
    let radial = g.add(
        NodeKind::Radial {
            inner: 0.2,
            outer: 0.6,
            height: 300.0,
            base: 0.0,
        },
        [0.0; 2],
    );
    let steep = g.add(
        NodeKind::SelectSlope {
            min: 20.0,
            max: 90.0,
            falloff: 1.0,
        },
        [0.0; 2],
    );
    g.connect(radial, 0, steep, 0);
    let f = eval_one(&g, steep, 64);
    assert_eq!(f.at(32, 32), 0.0, "flat top");
    assert_eq!(f.at(0, 0), 0.0, "flat plain");
    assert!(f.at(32 + 13, 32) > 0.9, "the flank");
    // Normalize stretches to the range asked; Invert height flips it.
    let norm = g.add(NodeKind::Normalize { min: 10.0, max: 20.0 }, [0.0; 2]);
    g.connect(radial, 0, norm, 0);
    let (lo, hi) = eval_one(&g, norm, 32).range();
    assert!((lo - 10.0).abs() < 1e-3 && (hi - 20.0).abs() < 1e-3);
    let flip = g.add(NodeKind::InvertHeight, [0.0; 2]);
    g.connect(radial, 0, flip, 0);
    let fl = eval_one(&g, flip, 32);
    assert!(fl.at(16, 16) < fl.at(0, 0));
    // Facing: on a ramp rising to the east, slopes face west.
    let ramp = g.add(
        NodeKind::LinearGradient {
            angle: 90.0,
            height: 300.0,
            base: 0.0,
        },
        [0.0; 2],
    );
    let west = g.add(NodeKind::Facing { angle: 270.0, spread: 40.0 }, [0.0; 2]);
    let east = g.add(NodeKind::Facing { angle: 90.0, spread: 40.0 }, [0.0; 2]);
    g.connect(ramp, 0, west, 0);
    g.connect(ramp, 0, east, 0);
    assert!(eval_one(&g, west, 16).at(8, 8) > 0.9);
    assert!(eval_one(&g, east, 16).at(8, 8) < 0.1);
}

#[test]
fn erosion_carves_and_settles_and_thermal_limits_slopes() {
    let mut g = Graph::default();
    let hills = g.add(
        NodeKind::Noise {
            style: NoiseStyle::Ridged,
            scale: 400.0,
            octaves: 5,
            roughness: 0.5,
            height: 120.0,
            base: 40.0,
            seed: 9,
        },
        [0.0; 2],
    );
    let eroded = g.add(
        NodeKind::Erosion {
            rain: 2.0,
            erosion: 0.3,
            deposition: 0.3,
            capacity: 4.0,
            evaporation: 0.02,
            radius: 12.0,
            seed: 1,
        },
        [0.0; 2],
    );
    g.connect(hills, 0, eroded, 0);
    let e = g.evaluate(&extent(128), &[eroded], true, &AtomicBool::new(false)).unwrap();
    let before = e.field(hills, 0).unwrap();
    let after = e.field(eroded, 0).unwrap();
    let flow = e.field(eroded, 1).unwrap();
    let deposits = e.field(eroded, 2).unwrap();
    let wear = e.field(eroded, 3).unwrap();
    let changed = before.data.iter().zip(&after.data).filter(|(a, b)| (*a - *b).abs() > 0.05).count();
    assert!(changed > 128 * 128 / 4, "erosion changed only {changed} samples");
    // Peaks wear down: the highest point is lower.
    assert!(after.range().1 < before.range().1);
    let (flo, fhi) = flow.range();
    assert!(flo >= 0.0 && fhi <= 1.0 && fhi > 0.9);
    assert!(deposits.range().1 > 0.5);
    assert!(wear.range().1 > 0.5);
    // Water gathers in valleys: more flow where it is lower.
    let mean = |pick: &dyn Fn(f32) -> bool| {
        let v: Vec<f32> = (0..128 * 128).filter(|&c| pick(after.data[c])).map(|c| flow.data[c]).collect();
        v.iter().sum::<f32>() / v.len().max(1) as f32
    };
    let (lo, hi) = after.range();
    let mid = (lo + hi) / 2.0;
    assert!(mean(&|h| h < mid) > mean(&|h| h > mid));

    // Thermal erosion: a cliff becomes a slope no steeper than the talus.
    let step = g.add(
        NodeKind::Radial {
            inner: 0.5,
            outer: 0.505,
            height: 100.0,
            base: 0.0,
        },
        [0.0; 2],
    );
    let slid = g.add(
        NodeKind::Thermal {
            talus: 45.0,
            iterations: 400,
            strength: 1.0,
        },
        [0.0; 2],
    );
    g.connect(step, 0, slid, 0);
    let f = eval_one(&g, slid, 64);
    let spacing = extent(64).spacing();
    let steepest = (1..64)
        .map(|i| (f.at(i, 32) - f.at(i - 1, 32)).abs() / spacing)
        .fold(0.0f32, f32::max);
    assert!(steepest < 1.6, "steepest slope {steepest}");
}

#[test]
fn the_starter_graph_builds_and_applies_with_real_block_colours() {
    use crate::generate::{GenParams, Landform, generate};
    use crate::history::History;
    let mut world = generate(&GenParams {
        tiles_x: 4,
        tiles_z: 4,
        landform: Landform::Flat,
        ..GenParams::default()
    });
    let reg = &world.registry;
    let (rock, snow, gravel) = (
        TerrainId::ROCK.0,
        reg.terrain_by_name("Snow").unwrap().0,
        reg.terrain_by_name("Gravel").unwrap().0,
    );
    let grass = reg.terrain_by_name("Grass").unwrap();
    let sand = reg.terrain_by_name("Sand").unwrap();
    let g = Graph::starter(grass.0, rock, snow, gravel);
    let rect = BlockRect::new(-256, -256, 256, 256);
    let ext = Extent::covering(rect, 128);
    let outputs = g.build(&ext, &AtomicBool::new(false)).unwrap();
    assert!(outputs.heights.is_some());
    assert_eq!(outputs.terrains.len(), 2);
    assert!(outputs.colors.is_some());
    // The colours become the closest of two terrains: green to grass.
    let palette = [(grass, [96, 140, 60]), (sand, [220, 200, 150])];
    let mut h = History::default();
    h.begin("graph", 0);
    let tiles = map_tiles(&world);
    apply(&mut h.edit(&mut world, 0), &ext, &outputs, 1, &Painting::colors(&palette), &write(&tiles));
    h.commit();
    let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
    let heights: Vec<f32> = (0..128 * 128).map(|i| tile.top.get(i)).collect();
    let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    assert!(hi - lo > 20.0, "the flat world got relief: {lo}..{hi}");
    let grassy = (0..128 * 128).filter(|&i| tile.terrain.get(i) == grass.0).count();
    assert!(grassy > 1000, "{grassy} grass columns");
    // Undo puts the flat world back.
    h.undo(&mut world);
    let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
    assert_eq!(tile.top.get(0), 64.0);
    // A graph survives serialisation (it is saved with the world).
    let json = serde_json::to_string(&g).unwrap();
    assert_eq!(serde_json::from_str::<Graph>(&json).unwrap(), g);
    // Cancelling stops a build.
    assert_eq!(g.build(&ext, &AtomicBool::new(true)).unwrap_err(), GraphError::Cancelled);
}

#[test]
fn the_nearest_colour_table_picks_the_closest() {
    let colors = [[0, 0, 0], [255, 255, 255], [200, 30, 30], [30, 160, 40]];
    let t = Nearest::new(&colors);
    assert_eq!(t.get([10.0, 10.0, 10.0]), 0);
    assert_eq!(t.get([240.0, 250.0, 245.0]), 1);
    assert_eq!(t.get([190.0, 50.0, 40.0]), 2);
    assert_eq!(t.get([60.0, 150.0, 60.0]), 3);
}

/// How long the starter graph takes, node by node, at several resolutions.
/// Runs with MASSIF_GRAPH_TIMING (a list of resolutions, e.g. "1024,4096"),
/// and with MASSIF_GRAPH_TEMPLATE, on a template rather than the starter.
#[test]
fn timing_of_the_starter_graph() {
    let Ok(list) = std::env::var("MASSIF_GRAPH_TIMING") else { return };
    let g = benchmark_graph();
    for n in list.split(',').filter_map(|v| v.trim().parse::<usize>().ok()) {
        let ext = Extent {
            x0: -4096.0,
            z0: -4096.0,
            size: 8192.0,
            n,
        };
        let ids: Vec<NodeId> = g.nodes.iter().map(|n| n.id).collect();
        let started = std::time::Instant::now();
        let e = g.evaluate(&ext, &ids, false, &AtomicBool::new(false)).unwrap();
        let total = started.elapsed().as_secs_f32();
        let mut t: Vec<(String, f32)> = e.timings.iter().map(|(id, ms)| (g.node(*id).unwrap().kind.label().to_owned(), *ms)).collect();
        t.sort_by(|a, b| b.1.total_cmp(&a.1));
        eprintln!("{n} x {n}: {total:.2} s; slowest: {:?}", &t[..t.len().min(8)]);
    }
}

/// The graph the timing tests run: the starter, or with
/// MASSIF_GRAPH_TEMPLATE the template whose name holds that word.
fn benchmark_graph() -> Graph {
    match std::env::var("MASSIF_GRAPH_TEMPLATE") {
        Ok(name) => {
            let t = Template::ALL.into_iter().find(|t| t.label().to_lowercase().contains(&name.to_lowercase())).expect("a template of that name");
            eprintln!("template: {}", t.label());
            t.graph(&crate::registry::Registry::vanilla(), 62.0, 16384.0)
        }
        Err(_) => Graph::starter(1, 2, 3, 4),
    }
}

/// A whole build of the starter graph, as Convert does it. Runs with
/// MASSIF_GRAPH_BUILD (the resolution, e.g. 16384); with MASSIF_GRAPH_APPLY
/// too, the build is written to a world of as many blocks per side, and
/// with MASSIF_GRAPH_TEMPLATE, of a template rather than the starter.
#[test]
fn build_of_the_starter_graph() {
    let Some(n) = std::env::var("MASSIF_GRAPH_BUILD").ok().and_then(|v| v.parse::<usize>().ok()) else { return };
    let g = benchmark_graph();
    let ext = Extent {
        x0: -8192.0,
        z0: -8192.0,
        size: 16384.0,
        n,
    };
    // With MASSIF_GRAPH_APPLY, written to a world of one block per sample.
    if std::env::var_os("MASSIF_GRAPH_APPLY").is_none() {
        let started = std::time::Instant::now();
        let out = g.build(&ext, &AtomicBool::new(false)).unwrap();
        eprintln!("build at {n} x {n}: {:.1} s, {} layers over the ground", started.elapsed().as_secs_f32(), out.layers.len());
        assert_eq!(out.heights.as_ref().unwrap().n, n);
        return;
    }
    use crate::generate::{GenParams, Landform, generate};
    use crate::history::History;
    let tiles = (n / crate::coords::TILE_SIZE as usize) as i32;
    let mut world = generate(&GenParams {
        tiles_x: tiles,
        tiles_z: tiles,
        landform: Landform::Flat,
        ..GenParams::default()
    });
    let rect = match world.dims[0].bounds() {
        Some((lo, hi)) => crate::coords::BlockRect::new(
            lo.x * crate::coords::TILE_SIZE,
            lo.z * crate::coords::TILE_SIZE,
            (hi.x + 1) * crate::coords::TILE_SIZE,
            (hi.z + 1) * crate::coords::TILE_SIZE,
        ),
        None => unreachable!("the world has tiles"),
    };
    let ext = Extent::covering(rect, n);
    let started = std::time::Instant::now();
    let out = g.build(&ext, &AtomicBool::new(false)).unwrap();
    eprintln!("build at {n} x {n}: {:.1} s", started.elapsed().as_secs_f32());
    let reg = &world.registry;
    let palette = [
        (reg.terrain_by_name("Grass").unwrap(), [96, 140, 60]),
        (reg.terrain_by_name("Sand").unwrap(), [220, 200, 150]),
        (TerrainId::ROCK, [120, 120, 120]),
    ];
    let started = std::time::Instant::now();
    let mut h = History::default();
    h.begin("graph", 0);
    let tiles = map_tiles(&world);
    apply(&mut h.edit(&mut world, 0), &ext, &out, 1, &Painting::colors(&palette), &write(&tiles));
    eprintln!("written to the map: {:.1} s", started.elapsed().as_secs_f32());
}

/// Every template builds, within the world's heights, and looks like what
/// it says: islands in a sea, pillars, a river, dunes without water…
#[test]
fn templates_make_their_landscapes() {
    let reg = crate::registry::Registry::vanilla();
    let sea = 62.0;
    let ext = Extent {
        x0: -1024.0,
        z0: -1024.0,
        size: 2048.0,
        n: 192,
    };
    for t in Template::ALL {
        let g = t.graph(&reg, sea, 2048.0);
        assert_eq!(g.reads_the_map(), t.reads_the_map(), "{}", t.label());
        if t.reads_the_map() {
            // It works on a map: on its own, it has nothing to read.
            assert!(g.build(&ext, &AtomicBool::new(false)).is_err());
            continue;
        }
        let out = g.build(&ext, &AtomicBool::new(false)).unwrap_or_else(|e| panic!("{}: {e}", t.label()));
        let h = out.heights.clone().expect("a height output");
        assert!(out.colors.is_some() || out.shown.is_some(), "{}: no colours", t.label());
        let (lo, hi) = h.range();
        assert!(lo > -64.0 && hi < 320.0, "{}: heights {lo}..{hi}", t.label());
        let wet = h.data.iter().filter(|&&v| v < sea).count() as f32 / h.data.len() as f32;
        let steep = ops::slope_degrees(&h, ext.spacing()).data.iter().filter(|&&s| s > 60.0).count() as f32 / h.data.len() as f32;
        eprintln!("{:16} heights {lo:6.1}..{hi:6.1}, {:4.1} % under the sea, {:4.1} % steeper than 60°", t.label(), wet * 100.0, steep * 100.0);
        match t {
            Template::Archipelago => assert!((0.35..0.85).contains(&wet), "islands in a sea: {wet}"),
            Template::VolcanicIsland => assert!(wet > 0.3 && hi > sea + 120.0, "a high island: {wet}, {hi}"),
            Template::KarstTowers => assert!(steep > 0.01 && hi > sea + 100.0, "pillars: {steep}, {hi}"),
            Template::FloatingMountains => {
                assert!(steep > 0.004, "pillars: {steep}");
                assert_eq!(out.layers.len(), 1, "the floating islands");
                for layer in &out.layers {
                    let thick = layer.top.data.iter().zip(&layer.bottom.data).filter(|(t, b)| *t - *b >= 1.0).count();
                    assert!(thick > 0, "{} has rock", layer.name);
                    assert!(layer.top.range().1 < 320.0, "{} under the build limit", layer.name);
                }
                // The islands float clear of the ground.
                let (islands, ground) = (&out.layers[0], &h);
                for c in 0..h.data.len() {
                    if islands.top.data[c] - islands.bottom.data[c] >= 1.0 {
                        assert!(islands.bottom.data[c] >= ground.data[c] + 13.0, "an island touches the ground");
                    }
                }
            }
            Template::RiverValley | Template::DesertCanyon => assert!(wet > 0.001 && wet < 0.2, "a river: {wet}"),
            Template::DuneSea => assert_eq!(wet, 0.0, "no water in the desert"),
            Template::Continents => {
                assert!((0.15..0.85).contains(&wet), "continents and oceans: {wet}");
                assert!(out.biomes.as_ref().is_some_and(|b| b.paint), "its climate");
            }
            Template::Alpine | Template::RollingHills => assert!(wet < 0.5, "mostly land: {wet}"),
            Template::PaintMap | Template::MapRivers => unreachable!(),
        }
    }
}

/// Floating islands float: a rolling top over an underside hanging below,
/// nothing elsewhere, a fair share of the sky covered.
#[test]
fn floating_islands_float() {
    let ext = Extent {
        x0: -1024.0,
        z0: -1024.0,
        size: 2048.0,
        n: 256,
    };
    let (top, bottom, mask) = layers::floating_islands(&ext, 220.0, 20.0, 90.0, 0.6, 1.3, 4);
    let mut covered = 0;
    let mut deepest = 0.0f32;
    for c in 0..ext.n * ext.n {
        assert!(top.data[c] >= bottom.data[c], "the top is never under the bottom");
        let thick = top.data[c] - bottom.data[c];
        if mask.data[c] > 0.5 {
            covered += 1;
            assert!(thick > 0.5, "an island has rock: {thick}");
            assert!(bottom.data[c] > 220.0 - 20.0 - 90.0 * 1.3 * 1.05, "hangs no deeper than it may");
            deepest = deepest.max(thick);
        } else if mask.data[c] == 0.0 {
            assert!(thick < 3.0, "no rock outside the islands: {thick}");
        }
    }
    let share = covered as f32 / (ext.n * ext.n) as f32;
    assert!((0.03..0.6).contains(&share), "{share} of the sky has islands");
    assert!(deepest > 60.0, "the largest hang deep: {deepest}");
}

/// Rims jut out from the top of a cliff over the void at its foot, and
/// nowhere else.
#[test]
fn overhangs_jut_out_over_the_void() {
    let ext = Extent {
        x0: 0.0,
        z0: 0.0,
        size: 256.0,
        n: 256,
    };
    // A plateau 80 blocks above the plain, its edge at x = 128.
    let ground = Field::new(ext.n, (0..ext.n * ext.n).map(|c| if c % ext.n < 128 { 100.0 } else { 20.0 }).collect());
    let (top, bottom) = layers::overhang(&ground, &ext, 12.0, 6.0, 20.0, 0.0, 1);
    let at = |i: usize| (top.data[100 * ext.n + i], bottom.data[100 * ext.n + i]);
    // Just past the edge: a rim at the plateau's height, above the void.
    let (t, b) = at(131);
    assert!((t - 100.0).abs() < 3.0, "the rim is at the cliff's top: {t}");
    assert!(t - b >= 2.0 && b > 23.0, "a rim of rock over the void: {t}..{b}");
    // On the plateau, and far out on the plain: nothing.
    for i in [60, 200] {
        let (t, b) = at(i);
        assert!(t - b < 1.0, "no rim at {i}: {t}..{b}");
    }
}

/// Converted, a layer becomes a Solid level: under an island, the column
/// is ground, then air, then the island's rock.
#[test]
fn layers_become_solid_levels() {
    use crate::compose::TileStack;
    use crate::generate::{GenParams, Landform, generate};
    use crate::history::History;
    let mut world = generate(&GenParams {
        tiles_x: 4,
        tiles_z: 4,
        landform: Landform::Flat,
        ..GenParams::default()
    });
    let mut g = Graph::default();
    let ground = g.add(NodeKind::Constant { value: 40.0 }, [0.0, 0.0]);
    let out = g.add(NodeKind::HeightOutput, [200.0, 0.0]);
    g.connect(ground, 0, out, 0);
    let islands = g.add(
        NodeKind::FloatingIslands {
            altitude: 150.0,
            spread: 0.0,
            size: 60.0,
            coverage: 1.0,
            depth: 1.0,
            seed: 3,
        },
        [0.0, 200.0],
    );
    let layer = g.add(
        NodeKind::LayerOutput {
            slot: 1,
            palette: BlockPalette::Natural,
        },
        [200.0, 200.0],
    );
    g.connect(islands, 0, layer, 0);
    g.connect(islands, 1, layer, 1);
    let rect = crate::coords::BlockRect::new(-256, -256, 256, 256);
    let ext = Extent::covering(rect, 256);
    let built = g.build(&ext, &AtomicBool::new(false)).unwrap();
    assert_eq!(built.layers.len(), 1);
    assert_eq!(built.layers[0].name, "Floating islands (graph 1)");
    let mut h = History::default();
    h.begin("graph", 0);
    let index = h.edit(&mut world, 0).structure().add_level(built.layers[0].name.clone(), crate::level::LevelKind::Solid);
    let tiles = map_tiles(&world);
    apply(&mut h.edit(&mut world, 0), &ext, &built, 1, &Painting::colors(&[]), &write(&tiles));
    apply_layer(&mut h.edit(&mut world, index), &ext, &built.layers[0], 1, &[], TerrainId::ROCK, &tiles);
    h.commit();
    // Some column holds the ground, air, then an island.
    let dim = &world.dims[0];
    let mut spans = Vec::new();
    let mut floating = 0;
    for pos in dim.ground().positions() {
        let stack = TileStack::new(dim, pos, false).unwrap();
        for cell in 0..128 * 128 {
            stack.spans(cell, &mut spans);
            if spans.len() >= 2 && spans[0].y1 <= 41 && spans[1].y0 > 60 {
                floating += 1;
            }
        }
    }
    assert!(floating > 1000, "{floating} columns with an island over the void");
}

/// A world of 2 × 2 tiles at 100, with a bowl ten blocks deep in the
/// middle of the first tile and a ramp across the second.
fn bowl_world() -> crate::world::World {
    use crate::tile::{NO_FLUID, Tile};
    use crate::world::{Dimension, DimensionKind, World, WorldMeta};
    let mut world = World::new(WorldMeta::default());
    let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
    for tz in 0..2 {
        for tx in 0..2 {
            let mut tile = Tile::ground(100.0, 1, 62);
            for z in 0..128i32 {
                for x in 0..128i32 {
                    let i = crate::coords::cell_index(x as usize, z as usize);
                    if (tx, tz) == (0, 0) {
                        let d = (((x - 64) * (x - 64) + (z - 64) * (z - 64)) as f32).sqrt();
                        tile.top.set(i, 100.0 - (10.0 - d * 0.5).max(0.0));
                    } else if (tx, tz) == (1, 0) {
                        tile.top.set(i, 100.0 + x as f32 * 0.25);
                    }
                }
            }
            let _ = NO_FLUID;
            dim.ground_mut().insert(TilePos::new(tx, tz), tile);
        }
    }
    world.dims = vec![dim];
    world
}

/// A graph reads the map through its Map node: block for block at a block
/// per sample, again when the map changes, and not at all without a map.
#[test]
fn the_map_feeds_a_graph() {
    let mut world = bowl_world();
    let mut g = Graph::default();
    let map = g.add(NodeKind::MapHeight, [0.0, 0.0]);
    let out = g.add(NodeKind::HeightOutput, [200.0, 0.0]);
    g.connect(map, 0, out, 0);
    assert!(g.reads_the_map());
    let ext = Extent::covering(crate::coords::BlockRect::new(0, 0, 256, 256), 256);
    let cancel = AtomicBool::new(false);
    assert!(matches!(g.build(&ext, &cancel), Err(GraphError::MissingInput(name)) if name == "Map"));
    let source = GroundMap::new(world.dims[0].snapshot(), 1, 0.0, vec![[10, 20, 30], [40, 50, 60]], Vec::new());
    let built = g.build_on(Some(&source), &ext, &cancel, &Progress::default()).unwrap();
    let h = built.heights.unwrap();
    assert_eq!(h.at(64, 64), 90.0, "the bottom of the bowl");
    assert_eq!(h.at(10, 10), 100.0);
    assert_eq!(h.at(128 + 40, 20), 110.0, "the ramp of the next tile");
    assert_eq!(h.at(40, 200), 100.0);
    // Coarser, the same map; and off the map, the fallback.
    let wide = Extent::covering(crate::coords::BlockRect::new(-256, -256, 512, 512), 96);
    let read = source.sample(&wide);
    let (far, water) = (read.heights, read.water);
    assert_eq!(read.colors.data[40 * 96 + 40], [40, 50, 60], "the colour of terrain 1");
    assert_eq!(far.at(0, 0), 0.0, "off the map");
    // Sample (40, 40) is the block at (68, 68): four from the bowl's middle.
    assert!((far.at(40, 40) - 92.83).abs() < 0.01, "the bowl, at 8 blocks a sample: {}", far.at(40, 40));
    assert!(water.data.iter().all(|w| *w == 0.0), "no water: all of it is over the sea");
    // The cache knows when the map changed.
    let mut cache = EvalCache::default();
    let ids = [map];
    let first = g.evaluate_cached_on(Some(&source), &ext, &ids, &mut cache, &cancel).unwrap();
    assert_eq!(first.field(map, 0).unwrap().at(64, 64), 90.0);
    world.dims[0].ground_mut().tile_mut(TilePos::new(0, 0)).unwrap().top.set(crate::coords::cell_index(64, 64), 50.0);
    let stale = GroundMap::new(world.dims[0].snapshot(), 1, 0.0, Vec::new(), Vec::new());
    let same = g.evaluate_cached_on(Some(&stale), &ext, &ids, &mut cache, &cancel).unwrap();
    assert_eq!(same.field(map, 0).unwrap().at(64, 64), 90.0, "the same revision: not read again");
    let fresh = GroundMap::new(world.dims[0].snapshot(), 2, 0.0, Vec::new(), Vec::new());
    let again = g.evaluate_cached_on(Some(&fresh), &ext, &ids, &mut cache, &cancel).unwrap();
    assert_eq!(again.field(map, 0).unwrap().at(64, 64), 50.0);
}

/// The Map node reads the map as it shows from the sky: the rock a level
/// adds, the pit another carves, what was sculpted — not the levels the
/// graph writes itself. Converting a graph that paints it leaves the rock
/// where it is and paints the surfaces that show: the tower's top, the
/// pit's floor, the ground around.
#[test]
fn the_map_is_read_in_3d() {
    use crate::compose::TileStack;
    use crate::history::History;
    use crate::voxel::{ChunkPos, densities_at};
    let mut world = bowl_world();
    let at = |x: usize, z: usize| crate::coords::cell_index(x, z);
    let everywhere = crate::coords::BlockRect::new(i32::MIN, i32::MIN, i32::MAX, i32::MAX);
    let mut h = History::default();
    h.begin("levels", 0);
    let (tower, pit, dug) = {
        let mut edit = h.edit(&mut world, 0);
        let dim = edit.structure();
        (
            dim.add_level("Tower", LevelKind::Solid),
            dim.add_level("Pit", LevelKind::Carve),
            dim.add_level(Overlay::Dug.level_name(), Overlay::Dug.kind()),
        )
    };
    // A tower from 99 to 140 over x, z in 20..30; a pit from 92 up over
    // x in 100..110; and an old dig of the graph's at (10, 10).
    for (level, x0, (y0, y1)) in [(tower, 20, (99.0, 140.0)), (pit, 100, (92.0, 101.0)), (dug, 8, (80.0, 101.0))] {
        h.edit(&mut world, level).par_tiles([TilePos::new(0, 0)], everywhere, true, |_, _, tile| {
            for z in 20..30 {
                for x in x0..x0 + 10 {
                    let z = if level == dug { z - 12 } else { z };
                    tile.top.set(at(x, z), y1);
                    tile.bottom.as_mut().unwrap().set(at(x, z), y0);
                    tile.terrain.set(at(x, z), 1);
                }
            }
        });
    }
    h.commit();
    // A column sculpted up to y = 110 at (56, 104), where the ground is at 100.
    let pos = ChunkPos { x: 3, y: 6, z: 6 };
    let mut chunk = densities_at(&world.dims[0], pos);
    for y in 0..14 {
        chunk.set(8, y, 8, 255);
    }
    world.dims[0].voxels.insert(pos, Arc::new(chunk));

    let own = Graph::default().own_levels();
    let ext = Extent::covering(crate::coords::BlockRect::new(0, 0, 256, 256), 256);
    let read = sample_map(&world.dims[0], &ext, 0.0, &[[0; 3], [9; 3]], &own);
    let hgt = &read.heights;
    assert_eq!(hgt.at(25, 25), 140.0, "the top of the tower");
    assert_eq!(read.colors.data[25 * 256 + 25], [9; 3], "the tower's terrain");
    assert_eq!(hgt.at(105, 25), 92.0, "the floor of the pit");
    assert!(hgt.at(56, 104) > 108.0, "the sculpted column: {}", hgt.at(56, 104));
    assert_eq!(hgt.at(12, 12), 100.0, "the graph's own dig is not the map's");
    assert_eq!(hgt.at(40, 40), 100.0);

    // A graph that paints the whole map with terrain 2, heights untouched.
    let mut g = Graph::default();
    let map = g.add(NodeKind::MapHeight, [0.0, 0.0]);
    let all = g.add(NodeKind::SelectHeight { min: -1000.0, max: 1000.0, falloff: 0.0 }, [200.0, 100.0]);
    let height = g.add(NodeKind::HeightOutput, [400.0, 0.0]);
    let paint = g.add(NodeKind::TerrainOutput { terrain: 2 }, [400.0, 100.0]);
    g.connect(map, 0, height, 0);
    g.connect(map, 0, all, 0);
    g.connect(all, 0, paint, 0);
    let source = GroundMap::new(world.dims[0].snapshot(), 1, 0.0, Vec::new(), own.clone());
    let built = g.build_on(Some(&source), &ext, &AtomicBool::new(false), &Progress::default()).unwrap();
    h.begin("graph", 0);
    let tiles = map_tiles(&world);
    let raised = h.edit(&mut world, 0).structure().add_level(Overlay::Raised.level_name(), Overlay::Raised.kind());
    for (index, side) in [(dug, Overlay::Dug), (raised, Overlay::Raised)] {
        apply_overlay(&mut h.edit(&mut world, index), side, &ext, &built, 1, &Painting::colors(&[]), &own, &tiles);
    }
    for index in [0, tower, pit] {
        paint_surface(&mut h.edit(&mut world, index), &ext, &built, 1, &Painting::colors(&[]), &tiles);
    }
    h.commit();
    let dim = &world.dims[0];
    let level_tile = |k: usize| dim.levels[k].view(TilePos::new(0, 0)).unwrap();
    assert_eq!(level_tile(tower).top.get(at(25, 25)), 140.0, "the tower stands as it was");
    assert_eq!(level_tile(tower).terrain.get(at(25, 25)), 2, "its top painted");
    assert_eq!(level_tile(0).terrain.get(at(25, 25)), 1, "the ground under it not");
    assert_eq!(level_tile(pit).terrain.get(at(105, 25)), 2, "the pit's floor painted");
    assert_eq!(level_tile(0).terrain.get(at(40, 40)), 2, "the ground around");
    assert!(dim.levels[dug].view(TilePos::new(0, 0)).is_none_or(|t| !t.present(at(12, 12))), "the old dig is gone: the graph digs nothing");
    // Every surface that shows is the graph's terrain.
    let stack = TileStack::new(dim, TilePos::new(0, 0), false).unwrap();
    let mut spans = Vec::new();
    for (x, z) in [(25, 25), (105, 25), (40, 40), (12, 12), (200 - 128, 30)] {
        stack.spans(at(x, z), &mut spans);
        assert_eq!(spans.last().unwrap().terrain.0, 2, "the surface at ({x}, {z})");
    }
}

/// With MASSIF_RIVER_DEBUG set to a folder, a picture of a relief and its
/// water there (a PPM: shaded relief, water in blue), to look at.
fn debug_picture(name: &str, h: &Field, water: &Field, spacing: f32) {
    let Some(dir) = std::env::var_os("MASSIF_RIVER_DEBUG") else { return };
    let n = h.n;
    let mut out = format!("P6 {n} {n} 255\n").into_bytes();
    let (lo, hi) = h.range();
    for j in 0..n {
        for i in 0..n {
            let dx = (h.at(i + 1, j) - h.at(i.saturating_sub(1), j)) / (2.0 * spacing);
            let dz = (h.at(i, j + 1) - h.at(i, j.saturating_sub(1))) / (2.0 * spacing);
            let shade = (0.55 + 0.45 * ((-dx * 0.6 + 0.75 + dz * 0.45) / (dx * dx + 1.0 + dz * dz).sqrt())).clamp(0.0, 1.0);
            let t = (h.at(i, j) - lo) / (hi - lo).max(1.0);
            let mut c = [0.35 + 0.45 * t, 0.5 + 0.3 * t, 0.25 + 0.4 * t].map(|v| v * shade);
            let w = water.at(i, j);
            if w != NO_WATER {
                let deep = ((w - h.at(i, j)) / 4.0).clamp(0.3, 1.0);
                c = [0.1, 0.35, 0.75].map(|v| v * (1.2 - 0.4 * deep));
            }
            out.extend(c.map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8));
        }
    }
    std::fs::write(std::path::Path::new(&dir).join(format!("{name}.ppm")), out).unwrap();
}

/// A river follows its valley: laid in the bottom of it, its water a
/// little under the banks and running down with it, its bed dug deep
/// enough to hold the water, its banks sloping down to it.
#[test]
fn rivers_hold_water_that_runs_down() {
    // A valley falling to the south (larger j), its floor along i = 70,
    // offset from the middle of the grid.
    let n = 128;
    let f = Field::new(n, (0..n * n).map(|c| 140.0 - 0.2 * (c / n) as f32 + 0.5 * ((c % n) as f32 - 70.0).abs()).collect());
    let params = rivers::RiverParams {
        threshold: 0.6,
        depth: 5.0,
        width: 8.0,
        banks: 1.0,
        lakes: 0.5,
        sea: NO_WATER,
    };
    let r = rivers::rivers(&f, None, 1.0, params, &AtomicBool::new(false)).unwrap();
    assert!(r.count >= 1, "a river runs");
    let c = |j: usize| j * n + 70;
    for j in [20, 60, 100] {
        assert!(r.mask.data[c(j)] > 0.9, "the river follows the valley floor at {j}: {}", r.mask.data[c(j)]);
        assert!(r.mask.data[c(j) - 12] < 0.1, "and not its sides at {j}");
        // Shallow near its source, deeper as water joins it.
        let depth = r.water.data[c(j)] - r.dug.data[c(j)];
        assert!(depth > 1.0 && depth < 7.0, "water in the bed at {j}: {depth}");
        // Under the banks: the ground eight blocks to the side.
        assert!(r.water.data[c(j)] < r.dug.data[c(j) + 8], "the water stays in its valley at {j}");
        // Nothing raised.
        assert!((0..n).all(|i| r.dug.data[j * n + i] <= f.data[j * n + i] + 1e-4));
    }
    let level = |j: usize| r.water.data[c(j)];
    assert!(level(20) > level(60) + 4.0 && level(60) > level(100) + 4.0, "it runs down");
    // Downstream, it never rises.
    let along: Vec<f32> = (10..118).map(level).filter(|&l| l != NO_WATER).collect();
    assert!(along.len() > 90, "water all along: {}", along.len());
    assert!(along.windows(2).all(|w| w[1] <= w[0] + 1e-3), "it only goes down");
}

/// On a plain barely over the sea — as many maps painted by hand have —
/// the rivers keep their water all the way to the coast: their level may
/// come down to the sea's, but over land the water stays in their beds.
#[test]
fn rivers_hold_water_on_low_plains() {
    let n = 256;
    let noise = crate::noise::Noise::new(9);
    let sea = 62.0;
    // From four blocks over the sea in the north to the coast in the south.
    let f = Field::new(
        n,
        (0..n * n)
            .map(|c| {
                let (i, j) = ((c % n) as f64, (c / n) as f64);
                let land = sea as f64 + 4.0 - 4.5 * j / 230.0 + 1.5 * noise.fbm(i / 50.0, j / 50.0, 4, 2.0, 0.5);
                (if j > 235.0 { sea as f64 - 6.0 } else { land }) as f32
            })
            .collect(),
    );
    let params = rivers::RiverParams {
        threshold: 0.4,
        depth: 3.0,
        width: 8.0,
        banks: 1.0,
        lakes: 0.5,
        sea,
    };
    let r = rivers::rivers(&f, None, 2.0, params, &AtomicBool::new(false)).unwrap();
    let (mut bed, mut wet) = (0, 0);
    for c in 0..n * n {
        // The bed itself, dug more than a block: its rim is at the water's edge.
        if r.mask.data[c] >= 0.99 && f.data[c] > sea && f.data[c] - r.dug.data[c] > 1.0 {
            bed += 1;
            wet += (r.water.data[c] != NO_WATER) as usize;
        }
    }
    assert!(bed > 200, "rivers run: {bed}");
    assert!(wet as f32 > 0.97 * bed as f32, "their beds hold water: {wet} of {bed}");
}

/// With MASSIF_RIVER_BENCH set: how long rivers take on a large build.
#[test]
fn rivers_on_a_large_build() {
    let Some(n) = std::env::var("MASSIF_RIVER_BENCH").ok().and_then(|v| v.parse::<usize>().ok()) else { return };
    let noise = crate::noise::Noise::new(3);
    let started = std::time::Instant::now();
    let f = Field::new(
        n,
        (0..n * n)
            .into_par_iter()
            .map(|c| {
                let (i, j) = ((c % n) as f64 / n as f64, (c / n) as f64 / n as f64);
                (150.0 - 70.0 * j + 60.0 * noise.fbm(i * 8.0, j * 8.0, 7, 2.0, 0.5)) as f32
            })
            .collect(),
    );
    eprintln!("relief {n}²: {:.1} s", started.elapsed().as_secs_f32());
    let started = std::time::Instant::now();
    let params = rivers::RiverParams {
        threshold: 0.5,
        depth: 5.0,
        width: 16.0,
        banks: 1.0,
        lakes: 0.5,
        sea: 62.0,
    };
    let r = rivers::rivers(&f, None, 1.0, params, &AtomicBool::new(false)).unwrap();
    let wet = r.water.data.iter().filter(|&&w| w != NO_WATER).count();
    eprintln!("rivers {n}²: {:.1} s, {} rivers, {wet} samples of water", started.elapsed().as_secs_f32(), r.count);
    // A picture of it, a sample in eight.
    let m = n / 8;
    let pick = |f: &Field| Field::new(m, (0..m * m).map(|c| f.data[(c / m) * 8 * n + (c % m) * 8]).collect());
    debug_picture("rivers-large", &pick(&r.dug), &pick(&r.water), 8.0);
}

/// On a relief of hills, rivers find their own valleys: they run in the
/// lowest ground around them, join into larger ones that grow wider
/// downstream, and reach the sea; their water is everywhere under the
/// ground at the edge of their beds.
#[test]
fn rivers_find_their_valleys() {
    let n = 256;
    let noise = crate::noise::Noise::new(7);
    // Hills falling to the sea along the south edge.
    let f = Field::new(
        n,
        (0..n * n)
            .map(|c| {
                let (i, j) = ((c % n) as f64, (c / n) as f64);
                (120.0 - 0.28 * j + 22.0 * noise.fbm(i / 60.0, j / 60.0, 5, 2.0, 0.5)) as f32
            })
            .collect(),
    );
    let sea = 62.0;
    let params = rivers::RiverParams {
        threshold: 0.35,
        depth: 4.0,
        width: 10.0,
        banks: 1.0,
        lakes: 1.0,
        sea,
    };
    let r = rivers::rivers(&f, None, 2.0, params, &AtomicBool::new(false)).unwrap();
    debug_picture("rivers-hills", &r.dug, &r.water, 2.0);
    assert!(r.count >= 3, "a network of rivers: {}", r.count);
    // In the bottom of their valleys: river samples are lower than most
    // of the ground around them (where one crosses the rim of a hollow,
    // only the two sides of the pass are higher).
    let (mut checked, mut higher_sum, mut low) = (0, 0, 0);
    for j in (8..n - 8).step_by(3) {
        for i in (8..n - 8).step_by(3) {
            let c = j * n + i;
            if r.mask.data[c] < 0.99 || f.data[c] <= sea + 2.0 {
                continue;
            }
            let higher = [(-6i32, 0i32), (6, 0), (0, -6), (0, 6), (-4, -4), (4, 4), (-4, 4), (4, -4)]
                .iter()
                .filter(|(di, dj)| f.data[(j as i32 + dj) as usize * n + (i as i32 + di) as usize] > f.data[c])
                .count();
            higher_sum += higher;
            low += (higher < 3) as usize;
            checked += 1;
        }
    }
    assert!(checked > 20, "{checked} river samples looked at");
    let mean = higher_sum as f32 / checked as f32;
    assert!(mean > 4.5, "rivers run in valleys: {mean:.1} of 8 around them are higher");
    assert!(low * 8 < checked, "{low} of {checked} river samples are not in a valley");
    // Water held: wherever there is water, the samples next to it off the
    // river's bed are either water too or ground at least as high (along
    // the bed, water runs down).
    let mut leaks = 0;
    for j in 1..n - 1 {
        for i in 1..n - 1 {
            let c = j * n + i;
            let w = r.water.data[c];
            if w == NO_WATER {
                continue;
            }
            for (di, dj) in [(-1i32, 0i32), (1, 0), (0, -1), (0, 1)] {
                let o = (j as i32 + dj) as usize * n + (i as i32 + di) as usize;
                if r.water.data[o] == NO_WATER && r.mask.data[o] < 0.99 && r.dug.data[o] < w - 0.6 && r.dug.data[o] > sea {
                    leaks += 1;
                    if leaks < 12 {
                        eprintln!("leak at ({i}, {j}) water {w:.2} mask {:.2} → ({}, {}) dug {:.2} mask {:.2}", r.mask.data[c], i as i32 + di, j as i32 + dj, r.dug.data[o], r.mask.data[o]);
                    }
                }
            }
        }
    }
    let wet = r.water.data.iter().filter(|&&w| w != NO_WATER).count();
    assert!(wet > 300, "{wet} samples of water");
    assert!(leaks * 50 < wet, "{leaks} leaks for {wet} samples of water");
}

/// Water output: the hollows of the map become lakes at the level they
/// spill at, the rest keeps the sea's level.
#[test]
fn water_output_fills_the_hollows() {
    use crate::history::History;
    let mut world = bowl_world();
    let mut g = Graph::default();
    let map = g.add(NodeKind::MapHeight, [0.0, 0.0]);
    let lakes = g.add(NodeKind::Lakes { amount: 0.0, sea: 62.0 }, [200.0, 0.0]);
    let height = g.add(NodeKind::HeightOutput, [400.0, 0.0]);
    let water = g.add(NodeKind::WaterOutput, [400.0, 100.0]);
    g.connect(map, 0, lakes, 0);
    g.connect(lakes, 0, height, 0);
    g.connect(lakes, 2, water, 0);
    let ext = Extent::covering(crate::coords::BlockRect::new(0, 0, 256, 256), 256);
    let source = GroundMap::new(world.dims[0].snapshot(), 1, 0.0, vec![[10, 20, 30], [40, 50, 60]], Vec::new());
    let built = g.build_on(Some(&source), &ext, &AtomicBool::new(false), &Progress::default()).unwrap();
    assert!(built.water.is_some());
    let mut h = History::default();
    h.begin("graph", 0);
    let tiles = map_tiles(&world);
    apply(&mut h.edit(&mut world, 0), &ext, &built, 1, &Painting::colors(&[]), &write(&tiles));
    h.commit();
    let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
    let at = |x: usize, z: usize| crate::coords::cell_index(x, z);
    assert_eq!(tile.top.get(at(64, 64)), 90.0, "the ground is left as it was");
    assert_eq!(tile.fluid.get(at(64, 64)), 100, "the bowl is full to its rim");
    assert_eq!(tile.fluid.get(at(60, 66)), 100);
    assert_eq!(tile.fluid.get(at(10, 10)), 62, "elsewhere, the sea's level");
    // Undo gives the dry bowl back.
    h.undo(&mut world);
    let tile = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
    assert_eq!(tile.fluid.get(at(64, 64)), 62);
}

/// Converted as an overlay, the map's ground is left alone: a trench is a
/// Carve level holding its water, a mound a Solid level, a lake's water
/// rides a slab of the ground's own top block; and with the levels gone
/// the map is as it was.
#[test]
fn overlays_leave_the_ground_alone() {
    use crate::compose::TileStack;
    use crate::history::History;
    let mut world = bowl_world();
    let before = world.dims[0].snapshot();
    let ext = Extent::covering(crate::coords::BlockRect::new(0, 0, 256, 256), 256);
    let base = sample_map(&world.dims[0], &ext, 0.0, &[], &[]).heights;
    // A trench six blocks deep along x = 20..30 with water two under its
    // banks, a mound five blocks high along x = 200..210, and the bowl
    // full to its rim.
    let n = ext.n;
    let mut heights = base.clone();
    let mut water = Field::new(n, vec![NO_WATER; n * n]);
    for c in 0..n * n {
        let i = c % n;
        if (20..30).contains(&i) {
            heights.data[c] -= 6.0;
            water.data[c] = base.data[c] - 2.0;
        } else if (200..210).contains(&i) {
            heights.data[c] += 5.0;
        } else if base.data[c] < 99.5 {
            water.data[c] = 100.0;
        }
    }
    let outputs = Outputs {
        heights: Some(Arc::new(heights)),
        water: Some(Arc::new(water)),
        ..Outputs::default()
    };
    let mut h = History::default();
    h.begin("graph", 0);
    let (dug, raised) = {
        let mut edit = h.edit(&mut world, 0);
        let dim = edit.structure();
        (dim.add_level(Overlay::Dug.level_name(), Overlay::Dug.kind()), dim.add_level(Overlay::Raised.level_name(), Overlay::Raised.kind()))
    };
    let own = Graph::default().own_levels();
    let tiles = map_tiles(&world);
    apply_overlay(&mut h.edit(&mut world, dug), Overlay::Dug, &ext, &outputs, 1, &Painting::colors(&[]), &own, &tiles);
    apply_overlay(&mut h.edit(&mut world, raised), Overlay::Raised, &ext, &outputs, 1, &Painting::colors(&[]), &own, &tiles);
    h.commit();
    assert_eq!(world.dims[0].levels[dug].name, "Dug (graph 0)");

    let column = |world: &crate::world::World, x: i32, z: i32| {
        let dim = &world.dims[0];
        let stack = TileStack::new(dim, TilePos::containing(x, z), false).unwrap();
        let (mut spans, mut fluids) = (Vec::new(), Vec::new());
        let cell = local_index(x, z);
        stack.spans(cell, &mut spans);
        stack.fluids(cell, &spans, &mut fluids);
        (spans.last().map(|s| s.y1), fluids)
    };
    // The ground itself did not move.
    let ground = world.dims[0].ground().view(TilePos::new(0, 0)).unwrap();
    assert_eq!(ground.top.get(local_index(25, 50)), 100.0);
    assert_eq!(ground.fluid.get(local_index(64, 64)), 62);
    drop(ground);
    // The trench: its floor at 94, water from there to 98.
    assert_eq!(column(&world, 25, 50), (Some(94), vec![(94, 98)]));
    // The mound, on the ramp of the next tile (119.25 there): five higher.
    assert_eq!(column(&world, 205, 50).0, Some(124));
    // The bowl: its bottom where it was, full to its rim.
    assert_eq!(column(&world, 64, 64), (Some(90), vec![(90, 100)]));
    // Elsewhere, nothing.
    assert_eq!(column(&world, 150, 150), (Some(100), vec![]));

    // Without the levels, the map as it was.
    h.begin("delete", 0);
    {
        let mut edit = h.edit(&mut world, 0);
        let dim = edit.structure();
        dim.remove_level(raised);
        dim.remove_level(dug);
    }
    h.commit();
    assert_eq!(column(&world, 25, 50), (Some(100), vec![]));
    assert_eq!(column(&world, 64, 64), (Some(90), vec![]));
    let _ = before;
}
