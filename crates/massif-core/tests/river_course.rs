//! A river laid out by hand keeps to the way it was given: through every
//! point dragged, to the end clicked, what is shown being what is dug.

use massif_core::coords::{TilePos, local_index};
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::level::Level;
use massif_core::river::{GUIDE_REACH, natural_course, route, route_near, smooth_course};
use massif_core::tile::{NO_FLUID, Tile};
use massif_core::world::{Dimension, DimensionKind, World, WorldMeta};

/// The distance from `p` to the polyline `line`.
fn off(p: (f32, f32), line: &[(f32, f32)]) -> f32 {
    if line.len() == 1 {
        return (p.0 - line[0].0).hypot(p.1 - line[0].1);
    }
    let mut best = f32::MAX;
    for w in line.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (dx, dz) = (b.0 - a.0, b.1 - a.1);
        let l2 = dx * dx + dz * dz;
        let t = if l2 > 0.0 { (((p.0 - a.0) * dx + (p.1 - a.1) * dz) / l2).clamp(0.0, 1.0) } else { 0.0 };
        best = best.min((p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dz));
    }
    best
}

/// The max and mean distance of the points of `a` from the polyline `b`.
fn stray(a: &[(f32, f32)], b: &[(f32, f32)]) -> (f32, f32) {
    let d: Vec<f32> = a.iter().map(|&p| off(p, b)).collect();
    (d.iter().copied().fold(0.0, f32::max), d.iter().sum::<f32>() / d.len().max(1) as f32)
}

/// Pairs of segments of `line` that cross, not neighbours.
fn crossings(line: &[(f32, f32)]) -> usize {
    let cross = |a: (f32, f32), b: (f32, f32), c: (f32, f32), d: (f32, f32)| {
        let o = |p: (f32, f32), q: (f32, f32), r: (f32, f32)| (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0);
        let (d1, d2, d3, d4) = (o(c, d, a), o(c, d, b), o(a, b, c), o(a, b, d));
        d1 * d2 < 0.0 && d3 * d4 < 0.0
    };
    let n = line.len();
    let mut count = 0;
    for i in 0..n.saturating_sub(1) {
        for j in i + 2..n - 1 {
            if cross(line[i], line[i + 1], line[j], line[j + 1]) {
                count += 1;
            }
        }
    }
    count
}

/// A course laid out as the River tool lays it, each leg dragged first
/// (worked out coarsely) then let go (finely, along the coarse way): the
/// coarse way shown while dragged, the way found, the course made natural
/// (what is shown, then dug).
struct Laid {
    coarse: Vec<(f32, f32)>,
    found: Vec<(f32, f32)>,
    shown: Vec<(f32, f32)>,
    stops: Vec<usize>,
}

fn lay_out(level: &Level, anchors: &[(f32, f32)], width: f32) -> Laid {
    lay_out_as(level, anchors, width, true)
}

/// [`lay_out`], each leg dragged first (`dragged`) or clicked at once.
fn lay_out_as(level: &Level, anchors: &[(f32, f32)], width: f32, dragged: bool) -> Laid {
    let mut found = vec![anchors[0]];
    let mut coarse = vec![anchors[0]];
    let mut stops = vec![0];
    for w in anchors.windows(2) {
        let guide = route(level, w[0], w[1], 40_000);
        let leg = route_near(level, w[0], w[1], 400_000, if dragged { &guide } else { &[] });
        coarse.extend(guide.iter().skip(1));
        found.extend(leg.iter().skip(1));
        stops.push(found.len() - 1);
    }
    let (shown, stops) = natural_course(level, &found, width, &stops);
    Laid { coarse, found, shown, stops }
}

/// Spurs at the points dragged: near each, coming to it, the course gets
/// nearer it at every point, and leaving it, farther. The worst
/// backtrack, in blocks.
fn spur(laid: &Laid) -> f32 {
    let mut worst = 0.0f32;
    for &a in &laid.stops[1..laid.stops.len() - 1] {
        let p = laid.shown[a];
        let d = |k: usize| (laid.shown[k].0 - p.0).hypot(laid.shown[k].1 - p.1);
        let mut k = a;
        while k > 0 && d(k - 1) < 12.0 {
            worst = worst.max(d(k) - d(k - 1));
            k -= 1;
        }
        let mut k = a;
        while k + 1 < laid.shown.len() && d(k + 1) < 12.0 {
            worst = worst.max(d(k) - d(k + 1));
            k += 1;
        }
    }
    worst
}

/// Rivers laid out as a user lays them: a source, an end, three points
/// dragged off the straight way between.
fn layouts(level: &Level) -> Vec<Vec<(f32, f32)>> {
    let on = |p: (f32, f32)| level.view(TilePos::containing(p.0 as i32, p.1 as i32)).is_some();
    let mut out = Vec::new();
    for (s, e) in [((-400.0, -400.0), (400.0, 380.0)), ((-420.0, 300.0), (380.0, -350.0)), ((0.0, -450.0), (20.0, 450.0)), ((-450.0, 0.0), (450.0, 40.0)), ((-300.0, -100.0), (250.0, 120.0))] {
        let (s, e): ((f32, f32), (f32, f32)) = (s, e);
        if !on(s) || !on(e) {
            continue;
        }
        let (dx, dz) = (e.0 - s.0, e.1 - s.1);
        let l = dx.hypot(dz);
        let n = (-dz / l, dx / l);
        let mut a = vec![s];
        a.extend([(0.25, 70.0), (0.5, -90.0), (0.75, 60.0)].iter().map(|&(t, o)| (s.0 + dx * t + n.0 * o, s.1 + dz * t + n.1 * o)));
        a.push(e);
        out.push(a);
    }
    out
}

/// Checks rivers laid out over `level` against the way they were given,
/// and says how far they strayed.
fn check(name: &str, level: &Level, width: f32) {
    let layouts = layouts(level);
    assert!(!layouts.is_empty());
    for anchors in layouts {
        let laid = lay_out(level, &anchors, width);
        // What gets dug: the curve through the course, as `dig_descent`
        // hands it to `carve_river`.
        let curve = smooth_course(&laid.shown);
        let natural = stray(&laid.shown, &laid.found);
        let dug = {
            let (a, b) = (stray(&curve, &laid.shown), stray(&laid.shown, &curve));
            (a.0.max(b.0), a.1)
        };
        let fine = stray(&laid.found, &laid.coarse);
        let anchor = anchors.iter().map(|&a| off(a, &curve)).fold(0.0, f32::max);
        let last = *anchors.last().unwrap();
        let end = curve.last().map_or(f32::MAX, |p| (p.0 - last.0).hypot(p.1 - last.1));
        eprintln!(
            "{name}: shown from found max {:.1} mean {:.1} · dug from shown max {:.1} mean {:.2} · let go from dragged max {:.1} · points off dug {:.1} · end {:.1}",
            natural.0, natural.1, dug.0, dug.1, fine.0, anchor, end
        );
        assert!(natural.0 <= 0.5 * width + 0.5, "{name}: the course strays {:.1} from the way found", natural.0);
        assert!(dug.0 <= 3.0_f32.min(width), "{name}: what is dug strays {:.1} from what is shown", dug.0);
        assert!(fine.0 <= GUIDE_REACH + 2.0, "{name}: the way let go strays {:.1} from the way dragged", fine.0);
        assert!(anchor <= 1.0, "{name}: a point dragged lies {anchor:.1} from what is dug");
        assert!(end <= 1.0, "{name}: it ends {end:.1} from the end clicked");
        assert!(spur(&laid) <= 0.5, "{name}: a spur of {:.1} at a point dragged", spur(&laid));
        assert_eq!(crossings(&laid.shown), 0, "{name}: the course crosses itself");
        assert_eq!(crossings(&curve), 0, "{name}: what is dug crosses itself");
    }
}

/// New World's hills (the user's map), and mountains, 1,024 blocks across.
#[test]
fn a_laid_out_river_keeps_to_its_way() {
    for landform in [Landform::Hills, Landform::Mountains] {
        let world = generate(&GenParams {
            tiles_x: 8,
            tiles_z: 8,
            landform,
            ..GenParams::default()
        });
        check(&format!("{landform:?}"), world.dims[0].ground(), 24.0);
        check(&format!("{landform:?}, narrow"), world.dims[0].ground(), 8.0);
    }
}

/// Valleys and ridges across the way, terraced a block at a time, as on
/// the user's map: the river goes through every point dragged all the same.
#[test]
fn a_laid_out_river_keeps_to_its_way_over_ridges() {
    let mut world = World::new(WorldMeta::default());
    let mut dim = Dimension::new(DimensionKind::Overworld, -64, 320);
    for tz in -4..4 {
        for tx in -4..4 {
            let mut tile = Tile::ground(0.0, 1, NO_FLUID);
            for z in 0..128 {
                for x in 0..128 {
                    let (bx, bz) = (tx * 128 + x, tz * 128 + z);
                    let h = 100.0 + 30.0 * (bx as f32 / 45.0).sin() * (bz as f32 / 70.0).cos() + 12.0 * (bz as f32 / 23.0 + bx as f32 / 61.0).sin();
                    tile.top.set(local_index(bx, bz), h.floor());
                }
            }
            dim.ground_mut().insert(TilePos::new(tx, tz), tile);
        }
    }
    world.dims = vec![dim];
    check("ridges", world.dims[0].ground(), 24.0);
}

/// The spurs found at points dragged (an overshoot past the point and
/// back): over hills and mountains, a point dragged on either side.
#[test]
fn no_spur_at_a_point_dragged() {
    for (landform, seed, anchors) in [
        (Landform::Hills, 7, [(192.0, 0.0), (60.0, 70.0), (-152.0, 200.0)]),
        (Landform::Mountains, 3, [(-240.0, -192.0), (4.0, -210.0), (168.0, -168.0)]),
    ] {
        let world = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform,
            seed,
            ..GenParams::default()
        });
        for dragged in [false, true] {
            let laid = lay_out_as(world.dims[0].ground(), &anchors, 24.0, dragged);
            let s = spur(&laid);
            eprintln!("{landform:?} seed {seed}, dragged {dragged}: spur {s:.1}");
            assert!(s <= 0.5, "{landform:?}: a spur of {s:.1} at the point dragged");
            let a = laid.stops[1];
            assert!((laid.shown[a].0 - anchors[1].0).hypot(laid.shown[a].1 - anchors[1].1) < 0.01, "through the point");
        }
    }
}

/// Where the water goes on by itself past the last point (no end): its
/// course neither loops nor steps back on itself (a spring over
/// mountains, wide, with lakes everywhere).
#[test]
fn a_spring_neither_loops_nor_steps_back() {
    use massif_core::river::descend;
    for (seed, source, through, lake_depth) in [(3, Some((0.0, -240.0)), (4.0, -210.0), 13.0), (1, None, (0.0, 0.0), 1.0)] {
        let world = generate(&GenParams {
            tiles_x: 4,
            tiles_z: 4,
            landform: Landform::Mountains,
            seed,
            ..GenParams::default()
        });
        let level = world.dims[0].ground();
        // No source given: the highest point.
        let start = source.unwrap_or_else(
            || {
                let mut best = ((0.0, 0.0), f32::MIN);
                for z in -250..250 {
                    for x in -250..250 {
                        let h = level.view(TilePos::containing(x, z)).map_or(f32::MIN, |t| t.top.get(local_index(x, z)));
                        if h > best.1 {
                            best = ((x as f32, z as f32), h);
                        }
                    }
                }
                best.0
            },
        );
        let (mut course, mut stops) = (vec![start], vec![0]);
        if source.is_some() {
            course = route(level, start, through, 400_000);
            stops.push(course.len() - 1);
        }
        let last = *course.last().unwrap();
        let tail = descend(level, (last.0.floor() as i32, last.1.floor() as i32), 63.0, 60_000, 2_000_000, lake_depth);
        course.extend(tail.course.iter().skip(1));
        let (shown, _) = natural_course(level, &course, 120.0, &stops);
        let mut pts: Vec<(f32, f32)> = Vec::new();
        for &p in &shown {
            if pts.last().is_none_or(|q: &(f32, f32)| (q.0 - p.0).hypot(q.1 - p.1) >= 1.5) {
                pts.push(p);
            }
        }
        let back = pts.windows(3).filter(|w| {
            let (a, b) = ((w[1].0 - w[0].0, w[1].1 - w[0].1), (w[2].0 - w[1].0, w[2].1 - w[1].1));
            (a.0 * b.0 + a.1 * b.1) / (a.0.hypot(a.1) * b.0.hypot(b.1)) < -0.5
        }).count();
        let jump = shown.windows(2).map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1)).fold(0.0, f32::max);
        eprintln!("seed {seed}: {} points, crossings {}, steps back {back}, longest step {jump:.1}", shown.len(), crossings(&shown));
        assert_eq!(crossings(&shown), 0, "seed {seed}: it crosses itself");
        assert_eq!(back, 0, "seed {seed}: it steps back");
    }
}
