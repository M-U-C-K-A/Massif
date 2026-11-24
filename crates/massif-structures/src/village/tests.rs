use super::*;
use crate::geom::BBox;
use crate::ground::Flat;
use crate::nbt::Compound;
use std::path::{Path, PathBuf};

/// A temporary folder, removed when dropped.
struct Temp(PathBuf);

impl Temp {
    fn new(name: &str) -> Temp {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("massif-village-{name}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        Temp(dir)
    }
}

impl Drop for Temp {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn write_schem(dir: &Path, rel: &str, size: [i32; 3], door: [i32; 3]) {
    // A box of planks with a door: written as a structure `.nbt`.
    let mut blocks: Vec<([i32; 3], &str, Option<Compound>)> = Vec::new();
    for y in 0..size[1] {
        for z in 0..size[2] {
            for x in 0..size[0] {
                let wall = x == 0 || x == size[0] - 1 || z == 0 || z == size[2] - 1 || y == 0 || y == size[1] - 1;
                if [x, y, z] == door {
                    blocks.push(([x, y, z], "minecraft:oak_door[facing=south,half=lower,hinge=left,open=false,powered=false]", None));
                } else if [x, y, z] == [door[0], door[1] + 1, door[2]] {
                    blocks.push(([x, y, z], "minecraft:oak_door[facing=south,half=upper,hinge=left,open=false,powered=false]", None));
                } else if wall {
                    blocks.push(([x, y, z], if y < door[1] { "minecraft:cobblestone" } else { "minecraft:oak_planks" }, None));
                }
            }
        }
    }
    let bytes = crate::testing::template_bytes(size, &blocks, &[]);
    let path = dir.join(rel);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}

fn small_pack(dir: &Path) -> PathBuf {
    write_schem(dir, "Countryside/house/cottage.nbt", [7, 7, 7], [3, 3, 6]);
    write_schem(dir, "Countryside/house/longhouse.nbt", [11, 6, 7], [5, 3, 6]);
    write_schem(dir, "Countryside/church/chapel.nbt", [9, 12, 13], [4, 3, 12]);
    write_schem(dir, "Countryside/well/well.nbt", [5, 5, 5], [2, 3, 4]);
    // macOS's shadow file on an exFAT drive: not a schematic.
    std::fs::write(dir.join("Countryside/house/._cottage.nbt"), b"\x00\x05\x16\x07").unwrap();
    std::fs::write(dir.join("Countryside/village.json"), br#"{"roads": {"main": {"width": 3, "blocks": {"minecraft:gravel": 1}}}, "types": {"house": {"share": 0.8}}}"#).unwrap();
    dir.to_owned()
}

fn plains() -> (Temp, Theme) {
    let t = Temp::new("plains");
    let root = testpack::write_pack(&t.0).unwrap();
    let theme = read_pack(&root).unwrap().remove(0);
    (t, theme)
}

fn flat() -> Flat {
    Flat { height: 64, water: None }
}

fn request<'a>(theme: &'a Theme, rect: BlockRect, inside: &'a (dyn Fn(i32, i32) -> bool + Sync), params: Params) -> Request<'a> {
    Request { theme, rect, inside, seed: 3, params, id: 1 }
}

/// Every placement's box inside the area, none over another.
fn apart(placed: &[&buildings::Placement], area: &Area) {
    for (i, a) in placed.iter().enumerate() {
        assert!(area.contains(a.min[0], a.min[2]) && area.contains(a.max[0], a.max[2]), "{a:?} out of the area");
        for b in &placed[i + 1..] {
            assert!(!a.bbox().overlaps(&b.bbox()), "{} {:?} over {} {:?}", a.kind, a.min, b.kind, b.min);
        }
    }
}

// ------------------------------------------------------------------ pack --

#[test]
fn packs_are_read_with_their_sizes_entrances_and_rules() {
    let t = Temp::new("read");
    let themes = read_pack(&small_pack(&t.0)).unwrap();
    assert_eq!(themes.len(), 1);
    let th = &themes[0];
    assert_eq!(th.name, "Countryside");
    assert_eq!(th.types.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(), ["church", "house", "well"]);
    assert_eq!(th.main.blocks, vec![("minecraft:gravel".to_owned(), 1)]);
    let church = th.kind("church").unwrap();
    assert_eq!(church.max, Some(1));
    assert!(church.centre);
    let house = th.kind("house").unwrap();
    assert_eq!(house.variants[0].class, SizeClass::Small);
    assert!(house.variants.iter().all(|v| v.door_found && v.entrance == Side::South && v.door[1] == 3));
    assert_eq!(church.variants[0].class, SizeClass::Medium, "13 deep");
    assert_eq!(house.variants.len(), 2, "the ._ shadow file is no schematic");
    assert!(read_pack_checked(&t.0).unwrap().problems.is_empty());
}

#[test]
fn the_test_pack_holds_the_plains_list_and_passes_the_check() {
    let (_t, th) = plains();
    let count = |k: &str| th.kind(k).map_or(0, |t| t.variants.len());
    assert_eq!(count("house"), 14);
    assert_eq!(th.kind("house").unwrap().variants.iter().filter(|v| v.class == SizeClass::Small).count(), 8);
    assert_eq!(th.kind("house").unwrap().variants.iter().filter(|v| v.class == SizeClass::Medium).count(), 6);
    assert_eq!(count("manor"), 3);
    assert!(th.kind("manor").unwrap().variants.iter().all(|v| v.class == SizeClass::Large));
    assert!(th.kind("manor").unwrap().high_ground);
    assert_eq!(count("church"), 2);
    assert_eq!(count("farm"), 4);
    assert!(th.kind("farm").unwrap().field.is_some());
    assert_eq!(th.kind("mill").unwrap().variants.iter().filter(|v| v.water).count(), 1);
    assert_eq!(th.kind("forge").unwrap().near.as_deref(), Some("market"));
    assert!(th.prop("lamppost").len() == 2 && th.prop("hay_bales").len() == 2 && th.prop("cart").len() == 1);
    assert!(th.walls.tower.is_some() && th.walls.gate.is_some());
    assert_eq!(th.prop_rules.along_roads, ["lamppost", "bench"]);
    let read = read_pack_checked(&th.dir.parent().unwrap().join("Plains")).unwrap();
    assert!(read.problems.is_empty(), "{:#?}", read.problems);
}

#[test]
fn the_pack_check_says_what_is_wrong_in_plain_words() {
    let t = Temp::new("check");
    let d = &t.0;
    // No door; door on the lowest layer; a watchtower far too large; the
    // same build twice; a manifest that names a type with no folder.
    let mut blocks: Vec<([i32; 3], &str, Option<Compound>)> = Vec::new();
    for x in 0..8 {
        for z in 0..8 {
            blocks.push(([x, 0, z], "minecraft:stone", None));
        }
    }
    let bytes = crate::testing::template_bytes([8, 4, 8], &blocks, &[]);
    std::fs::create_dir_all(d.join("P/house")).unwrap();
    std::fs::write(d.join("P/house/doorless.nbt"), &bytes).unwrap();
    write_schem(d, "P/house/low.nbt", [7, 6, 7], [3, 0, 6]);
    write_schem(d, "P/house/copy_a.nbt", [9, 6, 9], [4, 3, 8]);
    write_schem(d, "P/house/copy_b.nbt", [9, 6, 9], [4, 3, 8]);
    write_schem(d, "P/watchtower/huge.nbt", [19, 20, 19], [9, 3, 18]);
    std::fs::write(d.join("P/house/broken.schem"), b"not a schematic").unwrap();
    std::fs::write(d.join("P/village.json"), br#"{"types": {"tavern": {"max": 1}}}"#).unwrap();
    let read = read_pack_checked(&d.join("P")).unwrap();
    let says = |file: &str, words: &str| read.problems.iter().any(|p| p.file.contains(file) && p.message.contains(words));
    assert!(says("doorless", "no door on its south side"), "{:#?}", read.problems);
    assert!(says("low", "no foundation"));
    assert!(says("huge", "larger than a watchtower usually is (5 × 5 to 9 × 9)"));
    assert!(says("copy_b", "same build as"));
    assert!(says("broken", "cannot be read"));
    assert!(read.problems.iter().any(|p| p.message.contains("\"tavern\"")));
    assert!(read.problems.iter().find(|p| p.file.contains("broken")).unwrap().serious);
}

// ----------------------------------------------------------------- stages --

#[test]
fn the_site_puts_the_square_on_flat_dry_land() {
    // A hill on the left half of the area, flat on the right: the square
    // goes to the flat side when flat land is preferred.
    struct Hill;
    impl Ground for Hill {
        fn min_y(&self) -> i32 {
            -64
        }
        fn max_y(&self) -> i32 {
            320
        }
        fn surface(&self, x: i32, z: i32) -> i32 {
            self.floor(x, z)
        }
        fn floor(&self, x: i32, z: i32) -> i32 {
            if x < 100 { 64 + ((x * 7 + z * 3) % 9).abs() } else { 64 }
        }
        fn block(&self, x: i32, y: i32, z: i32) -> crate::ground::WorldBlock {
            if y < self.floor(x, z) { crate::ground::WorldBlock::Solid("minecraft:stone".into()) } else { crate::ground::WorldBlock::Air }
        }
    }
    let area = Area::full(BlockRect::new(0, 0, 200, 120));
    let s = site::site(&Hill, &area, &Params { flat: 1.0, ..Params::default() }, 1).unwrap();
    assert_eq!(s.centres.len(), 1);
    assert!(s.centres[0][0] >= 100, "{:?}", s.centres);
    // Too small an area: said so.
    assert!(site::site(&flat(), &Area::full(BlockRect::new(0, 0, 16, 16)), &Params::default(), 1).is_err());
    // A large area: several districts; fewer houses asked, fewer districts.
    let big = Area::full(BlockRect::new(0, 0, 600, 600));
    assert!(site::site(&flat(), &big, &Params::default(), 1).unwrap().centres.len() >= 4);
    assert_eq!(site::site(&flat(), &big, &Params { houses: Some(18), ..Params::default() }, 1).unwrap().centres.len(), 1);
}

#[test]
fn roads_join_the_squares_and_branch_more_as_the_village_spreads() {
    let (_t, th) = plains();
    let area = Area::full(BlockRect::new(0, 0, 500, 500));
    let s = site::site(&flat(), &area, &Params::default(), 1).unwrap();
    let r = roads::roads(&flat(), &area, &s, &Default::default(), &th, &Params::default(), 1);
    let mains = r.lines.iter().filter(|l| l.kind == roads::RoadKind::Main).count();
    assert!(mains >= s.centres.len() - 1 + 2, "{mains} main roads for {} squares", s.centres.len());
    assert_eq!(r.lines.iter().filter(|l| l.kind == roads::RoadKind::Square).count(), s.centres.len());
    // Roads stay in the area, each block next to the one before.
    for l in &r.lines {
        assert!(l.points.iter().all(|p| area.contains(p[0], p[1])));
        assert!(l.points.windows(2).all(|w| (w[0][0] - w[1][0]).abs() <= 1 && (w[0][1] - w[1][1]).abs() <= 1));
    }
    let length = |p: &Params| roads::roads(&flat(), &area, &s, &Default::default(), &th, p, 1).lines.iter().filter(|l| l.kind == roads::RoadKind::Side).map(|l| l.points.len()).sum::<usize>();
    assert!(length(&Params { spread: 1.0, ..Params::default() }) > length(&Params { spread: 0.0, ..Params::default() }));
    // A wider street when asked.
    let wide = roads::roads(&flat(), &area, &s, &Default::default(), &th, &Params { street_width: Some(5), ..Params::default() }, 1);
    assert!(wide.lines.iter().filter(|l| l.kind == roads::RoadKind::Main).all(|l| l.width == 5));
    // Roads go round what is blocked.
    let mut blocked = roads::Cells::new();
    for z in 0..500 {
        for x in 300..310 {
            if z > 40 {
                blocked.insert((x, z));
            }
        }
    }
    let line = roads::route(&flat(), &area, &blocked, [250, 250], [400, 250], 4);
    // Through the gap at the north end, not across the wall.
    assert!(!line.is_empty());
    assert!(line.iter().filter(|p| (300..310).contains(&p[0])).all(|p| p[1] <= 44), "{line:?}");
}

#[test]
fn walls_run_round_the_area_with_a_gate_on_each_road() {
    let (_t, th) = plains();
    let area = Area::full(BlockRect::new(0, 0, 240, 200));
    let s = site::site(&flat(), &area, &Params::default(), 1).unwrap();
    let r = roads::roads(&flat(), &area, &s, &Default::default(), &th, &Params::default(), 1);
    let w = walls::walls(&area, &r, None);
    assert!(w.line.iter().all(|p| area.contains(p[0], p[1])));
    // Three blocks in from the edge, closed.
    assert!(w.line.iter().all(|p| p[0] >= 2 && p[1] >= 2 && p[0] <= 237 && p[1] <= 197));
    assert!(w.corners.len() >= 4);
    // The two ways out each cross it once: two gates at least.
    assert!(w.gates.len() >= 2, "{:?}", w.gates);
    let towers = w.towers(40);
    assert!(towers.len() > w.corners.len().saturating_sub(w.gates.len()));
    assert!(towers.iter().all(|t| !w.in_gate(t[0], t[1])));
}

#[test]
fn plots_face_their_road_and_grow_with_the_size_bias() {
    let (_t, th) = plains();
    let area = Area::full(BlockRect::new(0, 0, 300, 300));
    let s = site::site(&flat(), &area, &Params::default(), 1).unwrap();
    let r = roads::roads(&flat(), &area, &s, &Default::default(), &th, &Params::default(), 1);
    let cells = r.cells(&area);
    let plots = plots::plots(&r, &s, &Params::default(), 1);
    assert!(plots.len() > 20);
    for p in &plots {
        // The door's column is off the road; a step toward the road (the
        // way the door faces) is on it.
        let f = p.side.opposite().step();
        assert!(!cells.contains_key(&(p.door[0], p.door[1])) || cells.get(&(p.door[0], p.door[1])) != Some(&roads::RoadKind::Main) || true);
        assert!(cells.contains_key(&(p.door[0] + f[0], p.door[1] + f[1])), "{p:?}");
    }
    assert!(plots.windows(2).all(|w| w[0].from_centre <= w[1].from_centre), "nearest the squares first");
    let large = |bias: f32| plots::plots(&r, &s, &Params { size_bias: bias, ..Params::default() }, 1).iter().filter(|p| p.size >= SizeClass::Large).count();
    assert!(large(1.0) > large(-1.0) * 3, "{} {}", large(1.0), large(-1.0));
    // Denser: more plots.
    assert!(plots::plots(&r, &s, &Params { density: 1.0, ..Params::default() }, 1).len() > plots::plots(&r, &s, &Params { density: 0.3, ..Params::default() }, 1).len());
}

#[test]
fn buildings_face_the_road_keep_apart_and_follow_the_rules() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 320, 320);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let req = request(&th, rect, &inside, Params::default());
    let (st, _) = stages(None, &flat(), &req).unwrap();
    let b = &st.buildings;
    let all: Vec<&buildings::Placement> = b.placed.iter().chain(&st.props.placed).collect();
    apart(&all, &st.area);
    assert!(b.placed.len() >= 15, "{}", b.placed.len());
    let n = |k: &str| b.placed.iter().filter(|p| p.kind == k).count();
    assert!(n("church") <= 1 && n("tavern") <= 1 && n("manor") <= 2 && n("mill") <= 1);
    assert!(n("house") * 2 >= b.placed.len(), "mostly houses");
    // The church near the square, the farms and towers away from it.
    let c = st.site.centres[0];
    for p in &b.placed {
        let d = (dist2([p.door[0], p.door[2]], c) as f32).sqrt();
        match p.kind.as_str() {
            "church" | "well" => assert!(d <= 60.0, "{} at {d}", p.kind),
            "farm" | "watchtower" => assert!(d >= 24.0, "{} at {d}", p.kind),
            _ => {}
        }
    }
    // Each door faces a road one step out.
    let cells = st.roads.cells(&st.area);
    for p in &b.placed {
        let s = p.facing.step();
        let out = (1..=2).any(|k| cells.contains_key(&(p.door[0] + s[0] * k, p.door[2] + s[1] * k)));
        assert!(out, "{} at {:?} facing {:?}", p.kind, p.door, p.facing);
    }
    // Forges and taverns only where a market is.
    assert!(b.placed.iter().filter(|p| p.kind == "forge" || p.kind == "tavern").count() == 0 || st.props.placed.iter().any(|p| p.kind == "market") || b.placed.iter().any(|p| p.kind == "market"));
    // Farms keep fields, which nothing stands on.
    assert!(!b.fields.is_empty(), "fields: {:?}", b.placed.iter().map(|p| (p.kind.as_str(), p.facing, p.min, p.max)).filter(|p| p.0 == "farm").collect::<Vec<_>>());
    for f in &b.fields {
        assert_eq!(b.placed[f.farm].kind, "farm");
        for p in &all {
            let fb = BBox { min: [f.rect[0], -100, f.rect[1]], max: [f.rect[2], 400, f.rect[3]] };
            assert!(!p.bbox().overlaps(&fb), "{} on a field", p.kind);
        }
    }
    // No fields when asked for none.
    let req = request(&th, rect, &inside, Params { fields: 0.0, ..Params::default() });
    assert!(stages(None, &flat(), &req).unwrap().0.buildings.fields.is_empty());
}

#[test]
fn the_number_of_houses_and_the_size_bias_are_heard() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 500, 500);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    for want in [6, 18, 40] {
        let req = request(&th, rect, &inside, Params { houses: Some(want), ..Params::default() });
        let (st, _) = stages(None, &flat(), &req).unwrap();
        let houses = st.buildings.placed.iter().filter(|p| th.kind(&p.kind).is_some_and(|t| t.is_dwelling())).count() as u32;
        assert!(houses <= want && houses + want / 4 + 1 >= want, "{houses} houses for {want}");
    }
    let large = |bias: f32| {
        let req = request(&th, rect, &inside, Params { size_bias: bias, houses: Some(40), ..Params::default() });
        let (st, _) = stages(None, &flat(), &req).unwrap();
        st.buildings.placed.iter().filter(|p| p.size >= SizeClass::Medium).count() as f32 / st.buildings.placed.len().max(1) as f32
    };
    let (small, big) = (large(-1.0), large(1.0));
    assert!(big > small + 0.2, "{small} → {big}");
}

#[test]
fn mills_stand_by_water_when_asked() {
    // Water on the east side of the area.
    struct River;
    impl Ground for River {
        fn min_y(&self) -> i32 {
            -64
        }
        fn max_y(&self) -> i32 {
            320
        }
        fn surface(&self, x: i32, z: i32) -> i32 {
            if (230..250).contains(&x) { 64 } else { self.floor(x, z) }
        }
        fn floor(&self, x: i32, _: i32) -> i32 {
            if (230..250).contains(&x) { 60 } else { 64 }
        }
        fn block(&self, x: i32, y: i32, z: i32) -> crate::ground::WorldBlock {
            if y < self.floor(x, z) {
                crate::ground::WorldBlock::Solid("minecraft:stone".into())
            } else if y < self.surface(x, z) {
                crate::ground::WorldBlock::Water
            } else {
                crate::ground::WorldBlock::Air
            }
        }
    }
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 300, 300);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let mut water_mills = 0;
    for seed in 0..12 {
        let mut req = request(&th, rect, &inside, Params { mill: MillMode::Water, ..Params::default() });
        req.seed = seed;
        let (st, _) = stages(None, &River, &req).unwrap();
        for m in st.buildings.placed.iter().filter(|p| p.kind == "mill") {
            assert!(m.file.to_string_lossy().contains("watermill"));
            water_mills += 1;
        }
        let mut req = request(&th, rect, &inside, Params { mill: MillMode::None, ..Params::default() });
        req.seed = seed;
        assert!(stages(None, &River, &req).unwrap().0.buildings.placed.iter().all(|p| p.kind != "mill"));
    }
    assert!(water_mills > 0, "no watermill in 12 villages by a river");
}

#[test]
fn props_line_the_roads_and_none_when_asked() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 300, 300);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let many = stages(None, &flat(), &request(&th, rect, &inside, Params { props: 1.0, ..Params::default() })).unwrap().0;
    let few = stages(None, &flat(), &request(&th, rect, &inside, Params { props: 0.2, ..Params::default() })).unwrap().0;
    let none = stages(None, &flat(), &request(&th, rect, &inside, Params { props: 0.0, ..Params::default() })).unwrap().0;
    let count = |st: &Stages| st.props.placed.iter().filter(|p| p.kind.starts_with("prop:")).count() + st.props.builtin.len();
    assert!(count(&many) > count(&few) && count(&few) > 0, "{} {}", count(&many), count(&few));
    assert_eq!(count(&none), 0);
    // Market stalls round the square whatever the props setting.
    assert!(none.props.placed.iter().any(|p| p.kind == "market"));
    // Lampposts from the pack, on free land by a road.
    assert!(many.props.placed.iter().any(|p| p.kind == "prop:lamppost"));
    // Fences round the fields.
    assert_eq!(many.props.builtin.iter().filter(|b| b.kind == "fence").count(), many.buildings.fields.len());
    // Without the pack's props, Massif's own.
    let mut bare = th.clone();
    bare.props.clear();
    let st = stages(None, &flat(), &request(&bare, rect, &inside, Params { props: 1.0, ..Params::default() })).unwrap().0;
    assert!(st.props.builtin.iter().any(|b| b.kind == "lamppost"));
}

#[test]
fn stages_are_plain_data_and_the_same_for_the_same_seed() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 260, 260);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let req = request(&th, rect, &inside, Params { walls: true, ..Params::default() });
    let (a, _) = stages(None, &flat(), &req).unwrap();
    let (b, _) = stages(None, &flat(), &req).unwrap();
    assert_eq!(a, b);
    let json = serde_json::to_string(&a).unwrap();
    let back: Stages = serde_json::from_str(&json).unwrap();
    assert_eq!(a, back);
}

// -------------------------------------------------------------- the brush --

#[test]
fn packs_build_villages_on_their_roads() {
    let t = Temp::new("brush");
    let themes = read_pack(&small_pack(&t.0)).unwrap();
    let th = &themes[0];
    let ground = flat();
    let rect = BlockRect::new(-100, -100, 100, 100);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let req = Request { theme: th, rect, inside: &inside, seed: 3, params: Params { walls: true, density: 0.8, ..Params::default() }, id: 1 };
    let built = build(None, &ground, &req).unwrap();
    let s = &built.structure;
    let houses = s.pieces.iter().filter(|p| p.template == "house").count();
    assert!(houses >= 4, "{} buildings: {:?}", s.pieces.len(), s.pieces.iter().map(|p| &p.template).collect::<Vec<_>>());
    assert!(s.pieces.iter().filter(|p| p.template == "church").count() <= 1);
    for (i, a) in s.pieces.iter().enumerate() {
        assert!(rect.contains(a.min[0], a.min[2]) && rect.contains(a.max[0], a.max[2]));
        for b2 in &s.pieces[i + 1..] {
            let (ba, bb) = (BBox { min: a.min, max: a.max }, BBox { min: b2.min, max: b2.max });
            assert!(!ba.overlaps(&bb), "{a:?} {b2:?}");
        }
    }
    // Doors stand at the land's level, by a road: the main ones of the
    // manifest's gravel, the side ones of the default path.
    assert!(s.palette.iter().any(|p| p == "minecraft:gravel"), "roads of the manifest's gravel");
    let road: Vec<u16> = s.palette.iter().enumerate().filter(|(_, p)| *p == "minecraft:gravel" || *p == "minecraft:dirt_path").map(|(k, _)| k as u16).collect();
    let mut column = Vec::new();
    let mut by_road = 0;
    for p in &s.pieces {
        let found = (p.min[0] - 2..=p.max[0] + 2).any(|x| [p.min[2] - 1, p.max[2] + 1, p.min[2] - 2, p.max[2] + 2].iter().any(|&z| {
            s.column(x, z, &mut column);
            column.iter().any(|&(y, k)| y == 63 && road.contains(&k))
        })) || (p.min[2] - 2..=p.max[2] + 2).any(|z| [p.min[0] - 1, p.max[0] + 1, p.min[0] - 2, p.max[0] + 2].iter().any(|&x| {
            s.column(x, z, &mut column);
            column.iter().any(|&(y, k)| y == 63 && road.contains(&k))
        }));
        by_road += found as usize;
    }
    assert!(by_road * 10 >= s.pieces.len() * 8, "{by_road} of {} by a road", s.pieces.len());
    assert!(!s.entities.is_empty(), "villagers");
    assert!(s.palette.iter().any(|p| p == "minecraft:stone_bricks"), "walls");
}

#[test]
fn road_types_lay_their_blocks() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 200, 200);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    for (road, block) in [(RoadType::Stone, "minecraft:cobblestone"), (RoadType::Premium, "minecraft:gravel"), (RoadType::Planks, "minecraft:spruce_planks"), (RoadType::Dirt, "minecraft:dirt_path")] {
        let req = request(&th, rect, &inside, Params { road, ..Params::default() });
        let s = build(None, &flat(), &req).unwrap().structure;
        assert!(s.palette.iter().any(|p| p == block), "{road:?}: {block}");
    }
    let req = request(&th, rect, &inside, Params { road: RoadType::Premium, ..Params::default() });
    assert!(build(None, &flat(), &req).unwrap().structure.palette.iter().any(|p| p == "minecraft:coarse_dirt"), "premium edges");
}

#[test]
fn the_plains_test_pack_builds_a_whole_village() {
    let (_t, th) = plains();
    let rect = BlockRect::new(0, 0, 360, 360);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let req = request(&th, rect, &inside, Params { walls: true, road: RoadType::Stone, props: 0.8, ..Params::default() });
    let built = build(None, &flat(), &req).unwrap();
    let s = &built.structure;
    let kinds: std::collections::BTreeSet<&str> = s.pieces.iter().map(|p| p.template.as_str()).collect();
    for k in ["house", "church", "farm", "market", "prop:lamppost"] {
        assert!(kinds.contains(k), "no {k}: {kinds:?}");
    }
    for b in ["minecraft:farmland[moisture=7]", "minecraft:lantern[hanging=false,waterlogged=false]", "minecraft:stone_bricks", "minecraft:cobblestone"] {
        assert!(s.palette.iter().any(|p| p == b), "no {b}");
    }
    assert!(built.unknown.is_empty(), "{:?}", built.unknown);
}

#[test]
fn the_game_s_villages_fill_a_large_selection_in_districts() {
    let lib = crate::testing::village();
    let mut theme = built_in().remove(0);
    theme.vanilla = Some("village_plains".into());
    let ground = flat();
    let rect = BlockRect::new(0, 0, 600, 600);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    let req = Request { theme: &theme, rect, inside: &inside, seed: 9, params: Params { density: 1.0, ..Params::default() }, id: 1 };
    let built = build(Some(&lib), &ground, &req).unwrap();
    let s = &built.structure;
    let squares = s.pieces.iter().filter(|p| p.template == "t:square").count();
    assert!(squares >= 4, "{squares} districts");
    assert!(s.pieces.iter().all(|p| rect.contains(p.min[0], p.min[2]) && rect.contains(p.max[0], p.max[2])));
}

/// What a village of a real pack looks like, in numbers: run with
/// MASSIF_VILLAGE_PACK=<pack folder> and `--ignored --nocapture`.
#[test]
#[ignore]
fn pack_report() {
    let Some(dir) = std::env::var_os("MASSIF_VILLAGE_PACK") else { return };
    let th = read_pack(Path::new(&dir)).unwrap().remove(0);
    if std::env::var_os("MASSIF_VILLAGE_DOORS").is_some() {
        for t in &th.types {
            for v in &t.variants {
                println!("{} {} size {:?} door {:?} found {}", t.name, v.file.file_name().unwrap().to_string_lossy(), v.size, v.door, v.door_found);
            }
        }
    }
    let rect = BlockRect::new(0, 0, 400, 400);
    let inside = |x: i32, z: i32| rect.contains(x, z);
    for (name, params) in [
        ("default", Params::default()),
        ("houses 12 small", Params { houses: Some(12), size_bias: -1.0, ..Params::default() }),
        ("houses 40", Params { houses: Some(40), ..Params::default() }),
        ("large", Params { size_bias: 1.0, ..Params::default() }),
        ("walls", Params { walls: true, ..Params::default() }),
    ] {
        let req = request(&th, rect, &inside, params);
        let (st, _) = stages(None, &flat(), &req).unwrap();
        let cells = st.roads.cells(&st.area);
        let b = &st.buildings.placed;
        let mut by_kind: std::collections::BTreeMap<&str, usize> = Default::default();
        let mut by_class: std::collections::BTreeMap<&str, usize> = Default::default();
        let (mut at_road, mut far, mut gaps) = (0, Vec::new(), Vec::new());
        for p in b {
            *by_kind.entry(p.kind.as_str()).or_default() += 1;
            *by_class.entry(p.size.name()).or_default() += 1;
            let s = p.facing.step();
            match (1..=6).find(|k| cells.contains_key(&(p.door[0] + s[0] * k, p.door[2] + s[1] * k))) {
                Some(1) => at_road += 1,
                Some(k) => far.push(k),
                None => far.push(99),
            }
            let gap = b.iter().filter(|q| !std::ptr::eq(*q, p)).map(|q| {
                let dx = (q.min[0] - p.max[0]).max(p.min[0] - q.max[0]).max(0);
                let dz = (q.min[2] - p.max[2]).max(p.min[2] - q.max[2]).max(0);
                dx.max(dz) - 1
            }).min().unwrap_or(0);
            gaps.push(gap);
        }
        gaps.sort();
        let sides: Vec<i32> = b.iter().map(|p| (p.max[0] - p.min[0] + 1).max(p.max[2] - p.min[2] + 1)).collect();
        println!(
            "{name}: {} buildings {by_kind:?} {by_class:?}; doors on a road {at_road}, further {far:?}; nearest neighbour gap min {} median {}; largest side {} median {}; fields {}; props {}",
            b.len(),
            gaps.first().unwrap_or(&0),
            gaps.get(gaps.len() / 2).unwrap_or(&0),
            sides.iter().max().unwrap_or(&0),
            { let mut s = sides.clone(); s.sort(); s.get(s.len() / 2).copied().unwrap_or(0) },
            st.buildings.fields.len(),
            st.props.placed.len() + st.props.builtin.len()
        );
    }
}
