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

#[test]
fn every_node_of_the_catalogue_runs() {
    for kind in NodeKind::catalogue() {
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
                    assert!(c.data.iter().all(|p| p.iter().all(|x| x.is_finite())), "{}", kind.label());
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
        .filter(|k| matches!(k.family(), Family::Generator | Family::Landform))
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
    let lakes = g.add(NodeKind::Lakes { amount: 1.0 }, [0.0; 2]);
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
    apply(&mut h.edit(&mut world, 0), &ext, &outputs, 1, &palette);
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
