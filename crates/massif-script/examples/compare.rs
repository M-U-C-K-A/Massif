//! Compare two maps column by column: what a script made in WorldPainter
//! (a `.world`, imported as Massif imports it) against what it made in
//! Massif (a `.massif`), plane by plane, by names (terrains, biomes, layers
//! are numbered differently in each map).
//!
//! `cargo run --release -p massif-script --example compare -- <a> <b> [planes…]`

use massif_core::World;
use massif_core::coords::TILE_AREA;
use std::collections::BTreeMap;
use std::path::Path;

fn load(path: &Path) -> World {
    if path.extension().is_some_and(|e| e == "massif") {
        massif_format::load(path).expect("a .massif")
    } else {
        massif_legacy::import_world(path, None).expect("a .world").0
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (a, b) = (load(Path::new(&args[0])), load(Path::new(&args[1])));
    let only: Vec<&str> = args[2..].iter().map(String::as_str).collect();
    let want = |p: &str| only.is_empty() || only.contains(&p);
    let (da, db) = (&a.dims[0], &b.dims[0]);
    let mut positions: Vec<_> = da.ground().positions().chain(db.ground().positions()).collect();
    positions.sort_unstable_by_key(|p| (p.z, p.x));
    positions.dedup();
    let mut diff: BTreeMap<&str, (u64, Vec<String>)> = BTreeMap::new();
    let mut columns = 0u64;
    fn set(diff: &mut BTreeMap<&'static str, (u64, Vec<String>)>, plane: &'static str, x: i32, z: i32, va: String, vb: String) {
        let e = diff.entry(plane).or_default();
        e.0 += 1;
        if e.1.len() < 6 {
            e.1.push(format!("({x}, {z}): {va} / {vb}"));
        }
    }
    let name = |w: &World, kind: &str, n: u16| -> String {
        let r = &w.registry;
        match kind {
            "terrain" => r.terrains.get(n as usize).map_or(format!("#{n}"), |t| t.name.clone()),
            "biome" => if n == 0 { "auto".into() } else { r.biomes.get(n as usize).map_or(format!("#{n}"), |b| b.id.clone()) },
            "cover" => if n == 0 { "-".into() } else { r.cover(n).map_or(format!("#{n}"), |c| c.name.clone()) },
            "trees" | "understory" => if n == 0 { "-".into() } else { r.object_layer(n).map_or(format!("#{n}"), |c| c.name.clone()) },
            _ => n.to_string(),
        }
    };
    for pos in positions {
        let (ta, tb) = (da.ground().view(pos), db.ground().view(pos));
        let (ox, oz) = pos.origin();
        match (&ta, &tb) {
            (Some(ta), Some(tb)) => {
                for i in 0..TILE_AREA {
                    columns += 1;
                    let (x, z) = (ox + (i & 127) as i32, oz + (i >> 7) as i32);
                    if want("height") && (ta.top.get(i) - tb.top.get(i)).abs() > 0.01 {
                        set(&mut diff, "height", x, z, ta.top.get(i).to_string(), tb.top.get(i).to_string());
                    }
                    let fa = ta.fluid.get(i).max(ta.top.get(i).round() as i16);
                    let fb = tb.fluid.get(i).max(tb.top.get(i).round() as i16);
                    if want("water") && fa != fb {
                        set(&mut diff, "water", x, z, ta.fluid.get(i).to_string(), tb.fluid.get(i).to_string());
                    }
                    for (plane, pa, pb) in [
                        ("terrain", ta.terrain.get(i), tb.terrain.get(i)),
                        ("biome", ta.biome.get(i), tb.biome.get(i)),
                        ("cover", ta.cover.get(i), tb.cover.get(i)),
                        ("trees", ta.objects.get(i), tb.objects.get(i)),
                        ("understory", ta.understory.get(i), tb.understory.get(i)),
                        ("annotations", ta.annotations.get(i), tb.annotations.get(i)),
                        ("pockets", ta.pockets.get(i), tb.pockets.get(i)),
                    ] {
                        if !want(plane) {
                            continue;
                        }
                        let (na, nb) = (name(&a, plane, pa), name(&b, plane, pb));
                        if na != nb {
                            set(&mut diff, plane, x, z, na, nb);
                        }
                    }
                }
            }
            _ => {
                let e = diff.entry("tiles").or_default();
                e.0 += 1;
                if e.1.len() < 6 {
                    e.1.push(format!("tile {pos:?} only in {}", if ta.is_some() { "the first" } else { "the second" }));
                }
            }
        }
    }
    println!("{columns} columns compared");
    if diff.is_empty() {
        println!("identical");
    }
    for (plane, (n, examples)) in diff {
        println!("{plane}: {n} columns differ ({:.3} %)", n as f64 * 100.0 / columns.max(1) as f64);
        for e in examples {
            println!("    {e}");
        }
    }
}
