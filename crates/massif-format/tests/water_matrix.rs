//! Every water feature on maps of every kind, checked against what water
//! must do: stay where it is held, have no holes, run only down, and be
//! left as it is by the game once exported.
//!
//! Each case is one step the user takes, as the editor takes it: the River
//! tool from a spring or through points to an end, Edit ▸ Whole map ▸
//! Rivers and Lakes, Flood (a hollow, held, lowered, drained, beyond a
//! hollow, at a level set, inside a selection) and the Water brush. After
//! each, on the whole map:
//!
//! - no water beside open land lower than its surface, and none over air
//!   (`water::check`: spills, floating), no more than before the step;
//! - the export's water ([`massif_core::water::settle_tile`] with the
//!   export's steady water) is left as it is by the game's rules;
//! - a river's water never rises downstream;
//! - undo puts the map back exactly, redo makes it exactly as the step
//!   left it.
//!
//! It runs when asked, in `--release`: `MASSIF_WATER_MATRIX=1` runs every
//! case, `MASSIF_WATER_MATRIX=<word>` those whose name has the word;
//! `MASSIF_WATER_WORLDS=<a.massif;b.massif>` adds real maps;
//! `MASSIF_WATER_KEEP=<folder>` saves each map after its step, to export
//! and check in the game (tools/water_check.py).

use massif_core::coords::{BlockRect, TILE_SIZE, TilePos, local_index};
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::graph::{NodeKind, Reshape};
use massif_core::ops::{self, Flood};
use massif_core::river::{self, Descent, Mouth, RiverSpec};
use massif_core::{Brush, History, NO_FLUID, World};

/// The River tool's settings (`RiverSettings` in the editor).
#[derive(Debug, Clone, Copy)]
struct RiverSettings {
    width: f32,
    depth: f32,
    banks: f32,
    water: bool,
    flow: f32,
    erosion: f32,
    lakes: f32,
}

impl Default for RiverSettings {
    fn default() -> Self {
        Self { width: 12.0, depth: 4.0, banks: 12.0, water: true, flow: 0.5, erosion: 0.3, lakes: 0.5 }
    }
}

impl RiverSettings {
    fn lake_depth(&self) -> f32 {
        if self.lakes <= 0.001 { f32::MAX } else { 1.0 + (1.0 - self.lakes.min(1.0)) * 24.0 }
    }

    fn spec(&self, world: &World) -> RiverSpec {
        let reg = &world.registry;
        let flow = self.flow.clamp(0.0, 1.0);
        RiverSpec {
            width: self.width,
            width_end: self.width * (1.0 + 4.0 * flow),
            depth: self.depth,
            depth_end: self.depth * (1.0 + 1.5 * flow),
            banks: self.banks + self.erosion.clamp(0.0, 1.0) * 4.0 * self.width,
            water: self.water,
            bed: reg.terrain_by_name("Gravel"),
            shore: reg.terrain_by_name("Sand"),
            clay: reg.terrain_by_name("Clay"),
            sea: None,
            biome: reg.biome_by_id("minecraft:river"),
        }
    }
}

/// The top of the ground of a column as a block's y, and its water's level
/// when it is under water.
fn column(world: &World, x: i32, z: i32) -> Option<(i32, Option<i16>)> {
    let tile = world.dims[0].ground().view(TilePos::containing(x, z))?;
    let i = local_index(x, z);
    let top = massif_core::compose::block_top(tile.top.get(i));
    let f = tile.fluid.get(i);
    Some((top, (f != NO_FLUID && f as i32 > top).then_some(f)))
}

/// The ground's height as the editor's probe gives it.
fn height(world: &World, x: i32, z: i32) -> Option<f32> {
    let tile = world.dims[0].ground().view(TilePos::containing(x, z))?;
    Some(tile.top.get(local_index(x, z)))
}

/// Where a river laid out with the River tool runs: as `RiverDraft::plan`
/// works it out, from its anchors (the source, the points it is dragged
/// through, and its end if it has one).
fn lay_out(world: &World, anchors: &[(f32, f32)], end: bool, settings: &RiverSettings) -> (Descent, Vec<usize>) {
    let level = world.dims[0].ground();
    let sea = world.meta.sea_level as f32;
    let mut course = vec![anchors[0]];
    let mut stops = vec![0];
    for w in anchors.windows(2) {
        let leg = river::route(level, w[0], w[1], 400_000);
        course.extend(leg.iter().skip(1));
        stops.push(course.len() - 1);
    }
    let (lakes, mouth) = if end {
        let e = *anchors.last().unwrap();
        let (ex, ez) = (e.0.floor() as i32, e.1.floor() as i32);
        let mouth = match (height(world, ex, ez), column(world, ex, ez)) {
            (None, _) => Mouth::Edge,
            (Some(h), _) if h < sea => Mouth::Sea,
            (_, Some((_, Some(_)))) => Mouth::Water,
            _ => Mouth::Chosen,
        };
        (Vec::new(), Some(mouth))
    } else {
        let last = *anchors.last().unwrap();
        let d = river::descend(level, (last.0.floor() as i32, last.1.floor() as i32), sea, 60_000, 2_000_000, settings.lake_depth());
        course.extend(d.course.iter().skip(1));
        (d.lakes, d.mouth)
    };
    let sea = (mouth == Some(Mouth::Sea)).then_some(sea);
    let width = settings.width * (1.0 + 2.0 * settings.flow.clamp(0.0, 1.0));
    let (course, stops) = river::natural_course(level, &course, width, &stops);
    (Descent { course, lakes, mouth, sea }, stops)
}

/// A map to test on.
struct Map {
    name: String,
    world: World,
}

fn generated(landform: Landform, seed: i64, tiles: i32) -> Map {
    let world = generate(&GenParams {
        tiles_x: tiles,
        tiles_z: tiles,
        landform,
        seed,
        ..GenParams::default()
    });
    Map { name: format!("{}-{seed}", landform.label().to_lowercase()), world }
}

/// The map's bounds in blocks (x0, z0, x1, z1), exclusive.
fn bounds(world: &World) -> (i32, i32, i32, i32) {
    let (lo, hi) = world.dims[0].bounds().expect("a map");
    (lo.x * TILE_SIZE, lo.z * TILE_SIZE, (hi.x + 1) * TILE_SIZE, (hi.z + 1) * TILE_SIZE)
}

/// Dry columns over the sea, every `every` blocks, the highest first, at
/// least `apart` blocks from each other: where to let rivers go.
fn springs(world: &World, every: i32, apart: f32, n: usize) -> Vec<(i32, i32)> {
    let (x0, z0, x1, z1) = bounds(world);
    let sea = world.meta.sea_level;
    let mut all = Vec::new();
    for z in (z0 + 16..z1 - 16).step_by(every as usize) {
        for x in (x0 + 16..x1 - 16).step_by(every as usize) {
            if let Some((top, None)) = column(world, x, z)
                && top > sea + 6
            {
                all.push((top, x, z));
            }
        }
    }
    all.sort_by(|a, b| b.cmp(a));
    let mut out: Vec<(i32, i32)> = Vec::new();
    for (_, x, z) in all {
        if out.iter().all(|p| ((p.0 - x) as f32).hypot((p.1 - z) as f32) >= apart) {
            out.push((x, z));
            if out.len() == n {
                break;
            }
        }
    }
    out
}

/// Dry columns in hollows: lower than every column 6 blocks around them.
fn hollows(world: &World, n: usize) -> Vec<(i32, i32)> {
    let (x0, z0, x1, z1) = bounds(world);
    let sea = world.meta.sea_level;
    let mut out: Vec<(i32, i32, i32)> = Vec::new();
    for z in (z0 + 8..z1 - 8).step_by(3) {
        for x in (x0 + 8..x1 - 8).step_by(3) {
            let Some((top, None)) = column(world, x, z) else { continue };
            if top <= sea {
                continue;
            }
            let lowest = (-3..=3).all(|dz| (-3..=3).all(|dx| (dx == 0 && dz == 0) || column(world, x + dx, z + dz).is_some_and(|c| c.1.is_none() && c.0 >= top)));
            if !lowest || out.iter().any(|p| (p.0 - x).abs() + (p.1 - z).abs() <= 40) {
                continue;
            }
            // Water poured there stands at least two blocks deep.
            if let Some(Some(rim)) = ops::spill_level_within(&world.dims[0], x, z, 1 << 18)
                && rim as i32 > top + 1
            {
                out.push((x, z, rim as i32 - top));
            }
        }
    }
    out.sort_by_key(|h| -h.2);
    out.into_iter().take(n).map(|h| (h.0, h.1)).collect()
}

/// Columns on a slope: dry, with ground 3 blocks lower within 4 blocks.
fn slopes(world: &World, n: usize) -> Vec<(i32, i32)> {
    let (x0, z0, x1, z1) = bounds(world);
    let sea = world.meta.sea_level;
    let mut out: Vec<(i32, i32)> = Vec::new();
    for z in (z0 + 20..z1 - 20).step_by(17) {
        for x in (x0 + 20..x1 - 20).step_by(13) {
            let Some((top, None)) = column(world, x, z) else { continue };
            if top <= sea + 2 {
                continue;
            }
            let drop = [(4, 0), (-4, 0), (0, 4), (0, -4)].iter().filter_map(|d| column(world, x + d.0, z + d.1)).map(|c| top - c.0).max().unwrap_or(0);
            if (3..12).contains(&drop) && out.iter().all(|p| (p.0 - x).abs() + (p.1 - z).abs() > 120) {
                out.push((x, z));
                if out.len() == n {
                    return out;
                }
            }
        }
    }
    out
}

/// What the checks found after a step.
#[derive(Debug, Default, Clone)]
struct Found {
    spills: i64,
    floating: i64,
    unsettled: u64,
    over_land: u64,
    rises: usize,
    worst_rise: i32,
    undo: bool,
    redo: bool,
    note: String,
}

impl Found {
    fn bad(&self) -> bool {
        self.spills > 0 || self.floating > 0 || self.unsettled > 0 || self.rises > 0 || !self.undo || !self.redo
    }
}

/// Every column's ground and water, to compare a map before and after.
fn state(world: &World) -> Vec<u8> {
    let ground = world.dims[0].ground();
    let mut positions: Vec<TilePos> = ground.positions().collect();
    positions.sort_by_key(|p| (p.x, p.z));
    let mut out = Vec::new();
    for p in positions {
        let mut t = (*ground.view(p).unwrap()).clone();
        t.compact();
        out.extend(t.encode());
    }
    out
}

/// The tiles whose water differs between two maps, and those beside them.
fn wet_tiles(a: &World, b: &World) -> Vec<TilePos> {
    let (ga, gb) = (a.dims[0].ground(), b.dims[0].ground());
    let mut out = std::collections::BTreeSet::new();
    for p in gb.positions() {
        let (Some(ta), Some(tb)) = (ga.view(p), gb.view(p)) else { continue };
        if (0..massif_core::TILE_AREA).any(|i| ta.fluid.get(i) != tb.fluid.get(i) || ta.top.get(i) != tb.top.get(i)) {
            for dz in -1..=1 {
                for dx in -1..=1 {
                    let q = TilePos::new(p.x + dx, p.z + dz);
                    if gb.view(q).is_some() {
                        out.insert((q.x, q.z));
                    }
                }
            }
        }
    }
    out.into_iter().map(|(x, z)| TilePos::new(x, z)).collect()
}

/// A step the user takes, on a map with its history: what it did.
type Step<'a> = &'a mut dyn FnMut(&mut History, &mut World) -> String;
/// The course of a river, read from the map.
type Course<'a> = Option<&'a dyn Fn(&World) -> Vec<(f32, f32)>>;

/// Run `step` on `world` as one step to undo, and check what it left.
fn check_step(world: &mut World, step: Step, course: Course) -> Found {
    use rayon::prelude::*;
    let before_check = massif_core::water::check(&world.dims[0]);
    let before = state(world);
    let cold = world.clone();
    let mut h = History::new(4);
    let started = std::time::Instant::now();
    let note = format!("{} ({:.1} s)", step(&mut h, world), started.elapsed().as_secs_f32());
    h.commit();
    let after = state(world);
    let check = massif_core::water::check(&world.dims[0]);
    let mut found = Found {
        spills: check.spills as i64 - before_check.spills as i64,
        floating: check.floating as i64 - before_check.floating as i64,
        note,
        ..Found::default()
    };
    // The export's water, left as it is by the game's rules (where the
    // step changed something).
    let tiles = wet_tiles(&cold, world);
    let dim = &world.dims[0];
    let settled = tiles.par_iter().map(|&p| massif_core::water::settle_tile(dim, p, true)).collect::<Vec<_>>();
    found.unsettled = settled.iter().map(|s| s.unsettled).sum();
    found.over_land = settled.iter().map(|s| s.over_land).sum();
    if let Some(course) = course {
        // The water along the course never rises downstream.
        let mut last: Option<(i16, usize, (i32, i32))> = None;
        let mut first = Vec::new();
        for (k, p) in course(world).into_iter().enumerate() {
            let (x, z) = (p.0.floor() as i32, p.1.floor() as i32);
            if let Some((_, Some(w))) = column(world, x, z) {
                if let Some((l, at, lp)) = last
                    && w > l
                {
                    found.rises += 1;
                    found.worst_rise = found.worst_rise.max((w - l) as i32);
                    if first.len() < 3 {
                        first.push(format!("{w} at #{k} {x},{z} after {l} at #{at} {},{}", lp.0, lp.1));
                    }
                }
                if last.is_none_or(|l| w < l.0) {
                    last = Some((w, k, (x, z)));
                }
            }
        }
        if !first.is_empty() {
            found.note += &format!(" RISE: {}", first.join("; "));
            if std::env::var_os("MASSIF_WATER_PROFILE").is_some() {
                // The water and the ground along the course, before and after.
                for (k, p) in course(world).into_iter().enumerate() {
                    let (x, z) = (p.0.floor() as i32, p.1.floor() as i32);
                    let (a, b) = (column(&cold, x, z), column(world, x, z));
                    eprintln!("  #{k} {x},{z}: before {a:?} after {b:?}");
                }
            }
        }
    }
    if h.can_undo() {
        h.undo(world);
        found.undo = state(world) == before;
        h.redo(world);
        found.redo = state(world) == after;
    } else {
        found.undo = after == before;
        found.redo = true;
    }
    found
}

/// What the matrix runs: `None` the few cases of the ordinary tests,
/// `Some("")` all of them, `Some(word)` those whose name has the word.
fn wanted() -> Option<String> {
    std::env::var("MASSIF_WATER_MATRIX").ok().map(|v| if v == "1" { String::new() } else { v })
}

struct Report {
    rows: Vec<(String, Found)>,
}

impl Report {
    fn add(&mut self, name: String, f: Found) {
        eprintln!(
            "{:<60} {:>5} {:>4} {:>4} {:>6} {:>4}{} {}{} {}",
            name,
            f.spills,
            f.floating,
            f.unsettled,
            f.over_land,
            f.rises,
            if f.rises > 0 { format!(" (+{})", f.worst_rise) } else { String::new() },
            if f.undo { "" } else { "UNDO " },
            if f.redo { "" } else { "REDO " },
            f.note
        );
        self.rows.push((name, f));
    }
}

fn keep(world: &World, name: &str) {
    if let Some(dir) = std::env::var_os("MASSIF_WATER_KEEP") {
        let path = std::path::Path::new(&dir).join(format!("{}.massif", name.replace([' ', '/', ':', ','], "_")));
        massif_format::save(&world.clone().snapshot(), &path).expect("the map saves");
    }
}

fn maps() -> Vec<Map> {
    let mut out = Vec::new();
    for (landform, seed) in [(Landform::Flat, 1), (Landform::Plains, 1), (Landform::Hills, 1), (Landform::Hills, 7), (Landform::Mountains, 1), (Landform::Mountains, 3), (Landform::Islands, 1), (Landform::Islands, 5)] {
        out.push(generated(landform, seed, 4));
    }
    for path in std::env::var("MASSIF_WATER_WORLDS").unwrap_or_default().split(';').filter(|p| !p.is_empty()) {
        let world = massif_format::load(std::path::Path::new(path)).expect("the map loads");
        let name = std::path::Path::new(path).file_stem().unwrap().to_string_lossy().replace(' ', "_");
        out.push(Map { name, world });
    }
    out
}

#[test]
fn water_holds_on_every_map() {
    let Some(wanted) = wanted() else { return };
    let only = |name: &str| wanted.is_empty() || name.contains(wanted.as_str());
    let mut report = Report { rows: Vec::new() };
    eprintln!("{:<60} {:>5} {:>4} {:>4} {:>6} {:>4}", "case", "spill", "flt", "unst", "o.land", "rise");
    for map in maps() {
        let sea = map.world.meta.sea_level;
        let settings: Vec<(&str, RiverSettings)> = vec![
            ("default", RiverSettings::default()),
            ("narrow", RiverSettings { width: 4.0, depth: 1.0, banks: 0.0, flow: 0.0, erosion: 0.0, lakes: 0.0, ..RiverSettings::default() }),
            ("wide", RiverSettings { width: 40.0, depth: 10.0, banks: 40.0, flow: 1.0, erosion: 1.0, lakes: 1.0, ..RiverSettings::default() }),
            ("dry", RiverSettings { water: false, ..RiverSettings::default() }),
        ];
        // The River tool let go at a spring.
        for (k, &(x, z)) in springs(&map.world, 32, 150.0, 3).iter().enumerate() {
            for (sname, s) in &settings {
                let name = format!("{} river spring{k} {sname}", map.name);
                if !only(&name) {
                    continue;
                }
                let mut w = map.world.clone();
                let (plan, _) = lay_out(&w, &[(x as f32 + 0.5, z as f32 + 0.5)], false, s);
                let course = plan.course.clone();
                let f = check_step(
                    &mut w,
                    &mut |h, w| {
                        h.begin("River", 0);
                        let spec = s.spec(w);
                        let r = river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                        format!("{:.0} blocks to {:?}, {} lakes, from {:.0} to {:.0}", r.length, plan.mouth, plan.lakes.len(), r.source, r.mouth)
                    },
                    Some(&|_: &World| course.clone()),
                );
                keep(&w, &name);
                report.add(name, f);
            }
        }
        // The River tool through points to an end clicked.
        let ends = springs(&map.world, 48, 200.0, 4);
        let (x0, z0, x1, z1) = bounds(&map.world);
        if ends.len() >= 2 {
            let (a, b) = (ends[0], ends[ends.len() - 1]);
            // An end down in the lowest corner of the map, or by the sea.
            let mut low = (x0 + 24, z0 + 24);
            let mut best = i32::MAX;
            for zz in (z0 + 24..z1 - 24).step_by(16) {
                for xx in (x0 + 24..x1 - 24).step_by(16) {
                    if let Some((t, _)) = column(&map.world, xx, zz)
                        && t < best
                        && ((xx - a.0) as f32).hypot((zz - a.1) as f32) > 150.0
                    {
                        best = t;
                        low = (xx, zz);
                    }
                }
            }
            let mid = ((a.0 + low.0) / 2 + 40, (a.1 + low.1) / 2 - 30);
            // Its name, its points, whether the last is its end.
            type Layout<'a> = (&'a str, Vec<(f32, f32)>, bool);
            let layouts: Vec<Layout> = vec![
                ("to-end", vec![(a.0 as f32, a.1 as f32), (low.0 as f32, low.1 as f32)], true),
                ("through-to-end", vec![(a.0 as f32, a.1 as f32), (mid.0 as f32, mid.1 as f32), (low.0 as f32, low.1 as f32)], true),
                ("through-free", vec![(b.0 as f32, b.1 as f32), (mid.0 as f32, mid.1 as f32)], false),
            ];
            for (lname, anchors, end) in layouts {
                for (sname, s) in &settings {
                    let name = format!("{} river {lname} {sname}", map.name);
                    if !only(&name) {
                        continue;
                    }
                    let mut w = map.world.clone();
                    let (plan, stops) = lay_out(&w, &anchors, end, s);
                    // Through its points, and to its end.
                    let off = |p: (f32, f32)| plan.course.iter().map(|c| (c.0 - p.0).hypot(c.1 - p.1)).fold(f32::MAX, f32::min);
                    let missed: Vec<String> = anchors.iter().map(|&p| off(p)).enumerate().filter(|(_, d)| *d > 2.0).map(|(k, d)| format!("anchor {k} {d:.1} off")).collect();
                    let last = *plan.course.last().unwrap();
                    let end_off = if end { (last.0 - anchors.last().unwrap().0).hypot(last.1 - anchors.last().unwrap().1) } else { 0.0 };
                    let course = plan.course.clone();
                    let mut f = check_step(
                        &mut w,
                        &mut |h, w| {
                            h.begin("River", 0);
                            let spec = s.spec(w);
                            let r = river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                            format!("{:.0} blocks to {:?}, stops {:?}, anchors {:?}", r.length, plan.mouth, stops, anchors)
                        },
                        Some(&|_: &World| course.clone()),
                    );
                    if !missed.is_empty() || end_off > 2.0 {
                        f.note += &format!(" COURSE: {} end {end_off:.1} off", missed.join(", "));
                    }
                    keep(&w, &name);
                    report.add(name, f);
                }
            }
        }
        // A tributary: a river let go at a second spring, once the first is
        // dug; and a branch: a river laid out from the first one's water.
        let two = springs(&map.world, 32, 150.0, 2);
        if two.len() == 2 {
            for (sname, s) in settings.iter().take(2) {
                let mut w = map.world.clone();
                let (first, _) = lay_out(&w, &[(two[0].0 as f32 + 0.5, two[0].1 as f32 + 0.5)], false, s);
                let mut h0 = History::new(1);
                h0.begin("River", 0);
                let spec = s.spec(&w);
                river::dig_descent(&mut h0.edit(&mut w, 0), &first, &spec);
                h0.commit();
                let base = w.clone();
                let name = format!("{} river tributary {sname}", map.name);
                if only(&name) {
                    let (plan, _) = lay_out(&w, &[(two[1].0 as f32 + 0.5, two[1].1 as f32 + 0.5)], false, s);
                    let course = plan.course.clone();
                    let f = check_step(
                        &mut w,
                        &mut |h, w| {
                            h.begin("River", 0);
                            let spec = s.spec(w);
                            let r = river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                            format!("{:.0} blocks to {:?}, from {:.0} to {:.0}", r.length, plan.mouth, r.source, r.mouth)
                        },
                        Some(&|_: &World| course.clone()),
                    );
                    keep(&w, &name);
                    report.add(name, f);
                }
                // From a point a third of the way down the first river, in its
                // water, off to the side.
                let name = format!("{} river branch {sname}", map.name);
                let wet: Vec<(f32, f32)> = first.course.iter().copied().filter(|p| column(&base, p.0.floor() as i32, p.1.floor() as i32).is_some_and(|c| c.1.is_some())).collect();
                if only(&name) && wet.len() > 30 {
                    let mut w = base.clone();
                    let src = wet[wet.len() / 3];
                    let next = wet[wet.len() / 3 + 10];
                    let (dx, dz) = (next.0 - src.0, next.1 - src.1);
                    let len = dx.hypot(dz).max(0.1);
                    let side = (src.0 - dz / len * 60.0, src.1 + dx / len * 60.0);
                    let (plan, _) = lay_out(&w, &[src, side], false, s);
                    let course = plan.course.clone();
                    let f = check_step(
                        &mut w,
                        &mut |h, w| {
                            h.begin("River", 0);
                            let spec = s.spec(w);
                            let r = river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                            format!("{:.0} blocks to {:?}, from {:.0} to {:.0}", r.length, plan.mouth, r.source, r.mouth)
                        },
                        Some(&|_: &World| course.clone()),
                    );
                    keep(&w, &name);
                    report.add(name, f);
                }
            }
        }
        // The land shaped by the water: lowered, smoothed, raised and
        // flattened across the shore of a lake filled in a hollow.
        for (k, &(x, z)) in hollows(&map.world, 2).iter().enumerate() {
            let mut lake = map.world.clone();
            let Some(l) = ops::spill_level(&lake.dims[0], x, z, 16 << 20) else { continue };
            let mut h0 = History::new(1);
            h0.begin("Flood", 0);
            ops::flood_fill(&mut h0.edit(&mut lake, 0), x, z, Flood::Fill(l), 64 << 20);
            h0.commit();
            // The shore east of the hollow.
            let Some(sx) = (x..x + 200).find(|&sx| column(&lake, sx, z).is_some_and(|c| c.1.is_none())) else { continue };
            for tool in ["lower", "smooth", "raise", "flatten"] {
                let name = format!("{} shore{k} {tool}", map.name);
                if !only(&name) {
                    continue;
                }
                let mut w = lake.clone();
                let f = check_step(
                    &mut w,
                    &mut |h, w| {
                        h.begin(tool, 0);
                        let brush = Brush { radius: 6.0, strength: 1.0, ..Brush::default() };
                        for i in -4..=4 {
                            let (cx, cz) = (sx as f32 + 0.5, z as f32 + 0.5 + i as f32 * 3.0);
                            match tool {
                                "lower" => ops::raise(&mut h.edit(w, 0), &brush, cx, cz, -3.0, ops::Surface::Top),
                                "raise" => ops::raise(&mut h.edit(w, 0), &brush, cx, cz, 3.0, ops::Surface::Top),
                                "smooth" => ops::smooth(&mut h.edit(w, 0), &brush, cx, cz, ops::Surface::Top),
                                _ => ops::flatten(&mut h.edit(w, 0), &brush, cx, cz, (l - 3) as f32, ops::Surface::Top),
                            }
                        }
                        format!("across the shore at {sx},{z}, the lake at {l}")
                    },
                    None,
                );
                keep(&w, &name);
                report.add(name, f);
            }
        }
        // Edit ▸ Whole map ▸ Rivers and Lakes.
        let side = (x1 - x0).max(z1 - z0) as f32;
        let f = (side / 2048.0).clamp(0.5, 3.0);
        let whole: Vec<(&str, NodeKind)> = vec![
            ("lakes", NodeKind::Lakes { amount: 0.0, sea: sea as f32 }),
            ("lakes-fill", NodeKind::Lakes { amount: 0.5, sea: sea as f32 }),
            ("rivers", NodeKind::Rivers { threshold: 0.5, depth: 4.0, width: 10.0 * f.clamp(1.0, 2.5), sea: sea as f32, banks: 1.0, lakes: 0.5 }),
            ("rivers-many", NodeKind::Rivers { threshold: 0.2, depth: 2.0, width: 6.0, sea: sea as f32, banks: 0.5, lakes: 0.0 }),
            ("rivers-big", NodeKind::Rivers { threshold: 0.8, depth: 8.0, width: 30.0, sea: sea as f32, banks: 2.0, lakes: 1.0 }),
        ];
        for (wname, node) in whole {
            let name = format!("{} whole {wname}", map.name);
            if !only(&name) {
                continue;
            }
            let mut w = map.world.clone();
            let op = Reshape::Filter(node);
            let f = check_step(
                &mut w,
                &mut |h, w| {
                    h.begin("Whole map", 0);
                    let bed = w.registry.terrain_by_name("Gravel");
                    let shore = w.registry.terrain_by_name("Sand");
                    let c = massif_core::graph::reshape_ground(&mut h.edit(w, 0), &op, sea, bed, shore, &massif_core::auto::Steps::default());
                    format!("{} columns", c.columns)
                },
                None,
            );
            keep(&w, &name);
            report.add(name, f);
        }
        // Flood.
        for (k, &(x, z)) in hollows(&map.world, 3).iter().enumerate() {
            type Case = (&'static str, Box<dyn Fn(&mut History, &mut World) -> String>);
            let cases: Vec<Case> = vec![
                (
                    "hollow",
                    Box::new(move |h: &mut History, w: &mut World| {
                        let level = ops::spill_level(&w.dims[0], x, z, 16 << 20);
                        h.begin("Flood", 0);
                        match level {
                            Some(l) => format!("{} columns to {l}", ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Fill(l), 64 << 20)),
                            None => "not a hollow".into(),
                        }
                    }),
                ),
                (
                    "hollow-raised-3",
                    Box::new(move |h: &mut History, w: &mut World| {
                        // Filled, then held: raised a block at a time (warned
                        // of the rim: no further than it).
                        let rim = ops::spill_level(&w.dims[0], x, z, 16 << 20);
                        h.begin("Flood", 0);
                        let Some(l) = rim else { return "not a hollow".into() };
                        ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Fill(l - 3), 64 << 20);
                        let mut n = 0;
                        for _ in 0..5 {
                            let Some(now) = ops::water_at(&w.dims[0], x, z) else { break };
                            if now + 1 > l {
                                break;
                            }
                            ops::raise_water(&mut h.edit(w, 0), x, z, 64 << 20);
                            n += 1;
                        }
                        format!("raised {n} to {:?} (rim {l})", ops::water_at(&w.dims[0], x, z))
                    }),
                ),
                (
                    "hollow-over-rim-2",
                    Box::new(move |h: &mut History, w: &mut World| {
                        // Flood anyway: raised two blocks past the rim.
                        let rim = ops::spill_level(&w.dims[0], x, z, 16 << 20);
                        h.begin("Flood", 0);
                        let Some(l) = rim else { return "not a hollow".into() };
                        let n = ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Fill(l + 2), 64 << 20);
                        format!("{n} columns to {}", l + 2)
                    }),
                ),
                (
                    "hollow-lowered",
                    Box::new(move |h: &mut History, w: &mut World| {
                        // Filled first (a step of its own), then lowered.
                        let Some(l) = ops::water_at(&w.dims[0], x, z) else { return "not a hollow".into() };
                        h.begin("Lower", 0);
                        let mut at = (x, z);
                        let mut n = 0;
                        for _ in 0..2 {
                            match ops::lower_water(&mut h.edit(w, 0), at.0, at.1, 64 << 20) {
                                Some((_, Some(k))) => {
                                    at = k;
                                    n += 1;
                                }
                                _ => break,
                            }
                        }
                        format!("lowered {n} from {l}")
                    }),
                ),
                (
                    "hollow-drained",
                    Box::new(move |h: &mut History, w: &mut World| {
                        if ops::water_at(&w.dims[0], x, z).is_none() {
                            return "not a hollow".into();
                        }
                        h.begin("Drain", 0);
                        format!("drained {}", ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Drain, 64 << 20))
                    }),
                ),
            ];
            for (fname, case) in cases {
                let name = format!("{} flood{k} {fname}", map.name);
                if !only(&name) {
                    continue;
                }
                let mut w = map.world.clone();
                if fname == "hollow-lowered" || fname == "hollow-drained" {
                    // Filled first, as a step of its own.
                    if let Some(l) = ops::spill_level(&w.dims[0], x, z, 16 << 20) {
                        let mut h0 = History::new(1);
                        h0.begin("Flood", 0);
                        ops::flood_fill(&mut h0.edit(&mut w, 0), x, z, Flood::Fill(l), 64 << 20);
                        h0.commit();
                    }
                }
                let f = check_step(&mut w, &mut |h, w| case(h, w), None);
                keep(&w, &name);
                report.add(name, f);
            }
        }
        for (k, &(x, z)) in slopes(&map.world, 6).iter().enumerate() {
            let top = column(&map.world, x, z).unwrap().0;
            let cases: Vec<(String, i16, Option<BlockRect>)> = vec![
                // Flood anyway on a slope: a block over the spot.
                ("anyway".into(), ops::pour_level(&map.world.dims[0], x, z).unwrap(), None),
                ("anyway-limited".into(), (top - 2) as i16, None),
                ("fixed+5".into(), (top + 5) as i16, None),
                ("sea".into(), sea as i16, None),
                ("anyway-selection".into(), ops::pour_level(&map.world.dims[0], x, z).unwrap(), Some(BlockRect::new(x - 30, z - 30, x + 30, z + 30))),
            ];
            for (fname, level, selection) in cases {
                let name = format!("{} slope{k} flood-{fname}", map.name);
                if !only(&name) {
                    continue;
                }
                let mut w = map.world.clone();
                if let Some(rect) = selection {
                    let mut h0 = History::new(1);
                    h0.select(&mut w, 0, "Select", Some(massif_core::selection::Selection::shape(&massif_core::selection::Shape::Rect(rect))));
                }
                let f = check_step(
                    &mut w,
                    &mut |h, w| {
                        h.begin("Flood", 0);
                        format!("{} columns to {level}", ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Fill(level), 64 << 20))
                    },
                    None,
                );
                keep(&w, &name);
                report.add(name, f);
            }
            // The Water brush across the slope, at each height.
            for (bname, level) in [("start", ops::water_at(&map.world.dims[0], x, z).unwrap_or((top + 1) as i16)), ("sea", sea as i16), ("fixed+3", (top + 3) as i16)] {
                let name = format!("{} slope{k} brush-{bname}", map.name);
                if !only(&name) {
                    continue;
                }
                let mut w = map.world.clone();
                let f = check_step(
                    &mut w,
                    &mut |h, w| {
                        h.begin("Water", 0);
                        let brush = Brush { radius: 10.0, strength: 1.0, ..Brush::default() };
                        for i in 0..12 {
                            ops::set_fluid(&mut h.edit(w, 0), &brush, x as f32 + i as f32 * 3.0, z as f32, level);
                        }
                        format!("to {level}")
                    },
                    None,
                );
                keep(&w, &name);
                report.add(name, f);
            }
        }
        // All of it on one map, for the game: rivers from two springs (one
        // wide and deep), one laid out to the lowest place, the hollows
        // flooded (one past its rim), then the whole map's lakes.
        let name = format!("{} combined", map.name);
        if only(&name) && std::env::var_os("MASSIF_WATER_KEEP").is_some() {
            let mut w = map.world.clone();
            let f = check_step(
                &mut w,
                &mut |h, w| {
                    h.begin("All", 0);
                    let tops = springs(w, 32, 150.0, 3);
                    for (k, &(x, z)) in tops.iter().enumerate().take(2) {
                        let s = if k == 0 { RiverSettings::default() } else { RiverSettings { width: 24.0, depth: 6.0, flow: 1.0, erosion: 0.6, lakes: 1.0, ..RiverSettings::default() } };
                        let (plan, _) = lay_out(w, &[(x as f32 + 0.5, z as f32 + 0.5)], false, &s);
                        let spec = s.spec(w);
                        river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                    }
                    if let Some(&(x, z)) = tops.get(2) {
                        let (x0, z0, x1, z1) = bounds(w);
                        let low = (x0 + 32..x1 - 32).step_by(16).flat_map(|xx| (z0 + 32..z1 - 32).step_by(16).map(move |zz| (xx, zz))).min_by_key(|&(xx, zz)| column(w, xx, zz).map_or(i32::MAX, |c| c.0)).unwrap();
                        let s = RiverSettings::default();
                        let (plan, _) = lay_out(w, &[(x as f32 + 0.5, z as f32 + 0.5), (low.0 as f32, low.1 as f32)], true, &s);
                        let spec = s.spec(w);
                        river::dig_descent(&mut h.edit(w, 0), &plan, &spec);
                    }
                    for (k, &(x, z)) in hollows(w, 3).iter().enumerate() {
                        if let Some(l) = ops::spill_level(&w.dims[0], x, z, 16 << 20) {
                            ops::flood_fill(&mut h.edit(w, 0), x, z, Flood::Fill(if k == 0 { l + 2 } else { l }), 64 << 20);
                        }
                    }
                    let lakes = Reshape::Filter(NodeKind::Lakes { amount: 0.0, sea: sea as f32 });
                    let bed = w.registry.terrain_by_name("Gravel");
                    let shore = w.registry.terrain_by_name("Sand");
                    massif_core::graph::reshape_ground(&mut h.edit(w, 0), &lakes, sea, bed, shore, &massif_core::auto::Steps::default());
                    "all of it".to_owned()
                },
                None,
            );
            keep(&w, &name);
            report.add(name, f);
        }
    }
    let bad: Vec<&(String, Found)> = report.rows.iter().filter(|r| r.1.bad()).collect();
    eprintln!("{} cases, {} with a problem", report.rows.len(), bad.len());
    // Water laid where nothing holds it (a stroke of the Water brush down a
    // slope, a flood kept inside a selection) runs out once exported, as
    // the game would have it: only its spills are allowed.
    let held = |name: &str| !name.contains(" brush-") && !name.contains("-selection");
    let broken: Vec<&&(String, Found)> = bad.iter().filter(|r| r.1.floating > 0 || r.1.unsettled > 0 || r.1.rises > 0 || !r.1.undo || !r.1.redo || (r.1.spills > 0 && held(&r.0))).collect();
    assert!(broken.is_empty(), "{broken:#?}");
}
