//! A stroke that leaves part of the map looking as it was says why: a
//! ground cover of blocks lying over the terrain painted (as a
//! WorldPainter map's custom ground cover of sand does), a locked layer,
//! the brush's filter, a level over the one painted; the Info panel says it
//! of the column under the cursor. With MASSIF_SNAPSHOT_DIR, the word on
//! the map and the Info panel are rendered at 1× and 2×.

use super::tests::harness_with;
use super::*;
use egui_kittest::Harness;
use massif_core::coords::local_index;
use massif_core::generate::{GenParams, Landform, generate};
use massif_core::registry::CoverDef;

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(|d| {
        let dir = PathBuf::from(d).join("paint-blocked");
        std::fs::create_dir_all(&dir).unwrap();
        dir
    })
}

fn densities() -> Vec<f32> {
    if snapshot_dir().is_some() { vec![1.0, 2.0] } else { vec![1.0] }
}

/// A map of hills with a ground cover of sand blocks, as a WorldPainter
/// custom ground cover is imported, laid over `rect`; its number.
fn sand_covered(rect: massif_core::BlockRect) -> (massif_core::World, u16) {
    let mut world = generate(&GenParams { tiles_x: 8, tiles_z: 8, landform: Landform::Hills, ..GenParams::default() });
    let mut def = CoverDef::new("Sand", [219, 207, 163], &[("minecraft:sand", 1)], 1.0);
    def.soil_only = false;
    def.opacity = 1.0;
    assert!(def.is_laid());
    let sand = world.registry.add_cover(def);
    let ground = world.dims[0].ground_mut();
    for z in rect.z0..rect.z1 {
        for x in rect.x0..rect.x1 {
            ground.tile_mut(massif_core::TilePos::containing(x, z)).unwrap().cover.set(local_index(x, z), sand);
        }
    }
    (world, sand)
}

/// A stroke from `a` to `b` with the tools given; what it says at its end.
fn stroke(doc: &mut crate::doc::Document, tools: &crate::tools::ToolState, a: (f32, f32), b: (f32, f32)) -> Option<String> {
    let (s, _) = crate::tools::begin_stroke(doc, tools, a, false, false);
    let mut s = s.unwrap();
    for k in 0..=20 {
        let t = k as f32 / 20.0;
        crate::tools::continue_stroke(doc, tools, &mut s, (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t), 0.02);
    }
    crate::tools::end_stroke(doc, s)
}

fn paint_tools(terrain: &str, doc: &crate::doc::Document) -> crate::tools::ToolState {
    let mut tools = crate::tools::ToolState::default();
    tools.select(Tool::PaintTerrain);
    tools.terrain = doc.world.registry.terrain_by_name(terrain).unwrap().0;
    tools.brush.radius = 8.0;
    tools.brush.strength = 1.0;
    tools
}

/// Painting a terrain across a cover of sand: the terrain is painted under
/// it, and the stroke says the cover lies over that part of it.
#[test]
fn a_cover_over_the_terrain_is_named() {
    let (world, _) = sand_covered(massif_core::BlockRect::new(200, 180, 240, 220));
    let mut doc = crate::doc::Document::new(world, None, 1);
    let tools = paint_tools("Grass", &doc);
    let grass = tools.terrain;
    let said = stroke(&mut doc, &tools, (150.0, 200.0), (290.0, 200.0)).expect("said");
    assert!(said.contains("Sand") && said.contains("ground cover"), "{said}");
    let pct: i32 = said.split('%').next().unwrap().parse().unwrap();
    assert!((15..=45).contains(&pct), "{said}");
    // The terrain under the sand is painted all the same.
    let t = doc.dimension().ground().view(massif_core::TilePos::containing(220, 200)).unwrap().terrain.get(local_index(220, 200));
    assert_eq!(t, grass);
    // Away from it, nothing to say.
    assert_eq!(stroke(&mut doc, &tools, (100.0, 300.0), (180.0, 300.0)), None);
    // Hidden, the cover no longer stands in the way on the map.
    doc.world.registry.covers.last_mut().unwrap().hidden = true;
    assert_eq!(stroke(&mut doc, &tools, (150.0, 200.0), (290.0, 200.0)), None);
}

/// A cover of plants only tints the terrain: nothing said.
#[test]
fn plants_are_not_in_the_way() {
    let (mut world, sand) = sand_covered(massif_core::BlockRect::new(200, 180, 240, 220));
    world.registry.covers[sand as usize - 1] = CoverDef::new("Flowers", [200, 60, 60], &[("minecraft:poppy", 1)], 0.5);
    let mut doc = crate::doc::Document::new(world, None, 1);
    let tools = paint_tools("Grass", &doc);
    assert_eq!(stroke(&mut doc, &tools, (150.0, 200.0), (290.0, 200.0)), None);
}

/// Painting a cover over a locked one names the locked layer.
#[test]
fn a_locked_layer_is_named() {
    let (mut world, sand) = sand_covered(massif_core::BlockRect::new(200, 180, 240, 220));
    world.registry.covers[sand as usize - 1].locked = true;
    let mut doc = crate::doc::Document::new(world, None, 1);
    let mut tools = crate::tools::ToolState::default();
    tools.select(Tool::Cover);
    tools.cover = 1;
    tools.brush.radius = 8.0;
    tools.brush.strength = 1.0;
    let said = stroke(&mut doc, &tools, (150.0, 200.0), (290.0, 200.0)).expect("said");
    assert!(said.contains("Sand") && said.contains("locked"), "{said}");
}

/// The brush's filter leaving part of a stroke is said.
#[test]
fn the_filter_is_named() {
    let (world, _) = sand_covered(massif_core::BlockRect::new(0, 0, 1, 1));
    let mut doc = crate::doc::Document::new(world, None, 1);
    let mut tools = paint_tools("Stone", &doc);
    // Only below a height the hills are over in part.
    let h = |x: i32| doc.dimension().ground().view(massif_core::TilePos::containing(x, 300)).unwrap().top.get(local_index(x, 300));
    let mut hs: Vec<f32> = (100..300).map(h).collect();
    hs.sort_by(f32::total_cmp);
    tools.filter.below = Some(hs[hs.len() / 2] as i32);
    let said = stroke(&mut doc, &tools, (100.0, 300.0), (300.0, 300.0)).expect("said");
    assert!(said.contains("filter"), "{said}");
}

/// The Info panel says a terrain painted on the column stays under the
/// cover, with Paint terrain in hand.
#[test]
fn the_info_panel_says_it() {
    let (world, _) = sand_covered(massif_core::BlockRect::new(200, 180, 240, 220));
    let doc = crate::doc::Document::new(world, None, 1);
    let aim = crate::paint_check::Aim::Terrain(None);
    let note = crate::paint_check::note_at(doc.dimension(), &doc.world.registry, 0, aim, 210, 200).expect("said");
    assert!(note.contains("Sand"), "{note}");
    assert_eq!(crate::paint_check::note_at(doc.dimension(), &doc.world.registry, 0, aim, 100, 100), None);
}

fn app(ppp: f32) -> Harness<'static, MassifApp> {
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(3);
    {
        let app = h.state_mut();
        let (world, _) = sand_covered(massif_core::BlockRect::new(200, 180, 260, 240));
        app.doc = crate::doc::Document::new(world, None, 41);
        app.map_cache.invalidate_all();
        app.view3d.invalidate_all();
        app.tools = paint_tools("Grass", &app.doc);
    }
    h.run_steps(5);
    h
}

/// The word at the foot of the window after a stroke across the sand, and
/// the Info panel over it (rendered at 1× and 2×).
#[test]
fn the_word_shows_on_the_map() {
    for ppp in densities() {
        let mut h = app(ppp);
        {
            let m = h.state_mut().maps.values_mut().next().unwrap();
            m.zoom = 2.0;
            m.center = [230.0, 210.0];
        }
        h.run_steps(3);
        let m = h.state().maps.values().next().unwrap();
        let (a, b) = (m.screen_of(150.0, 210.0), m.screen_of(320.0, 215.0));
        super::tests::drag_on(&mut h, a, b);
        h.run_steps(3);
        let said = h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default();
        assert!(said.contains("Sand"), "{said:?}");
        if let Some(dir) = snapshot_dir() {
            h.render().unwrap().save(dir.join(format!("stroke-word-{ppp}x.png"))).unwrap();
        }
        // The Info panel, the cursor over the sand.
        let (doc, reg) = (&h.state().doc, h.state().doc.world.registry.clone());
        let info = crate::inspect::ColumnInfo::read(doc, 230, 210).unwrap();
        let note = crate::paint_check::note_at(doc.dimension(), &reg, 0, crate::paint_check::Aim::Terrain(Some(massif_core::registry::TerrainId(h.state().tools.terrain))), 230, 210);
        assert!(note.is_some());
        if let Some(dir) = snapshot_dir() {
            for dark in [true, false] {
                let (info, reg, note) = (info.clone(), reg.clone(), note.clone());
                let mut p = egui_kittest::Harness::builder().with_size(egui::vec2(272.0, 620.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui(move |ui| {
                    crate::theme::install(ui.ctx());
                    ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                    let pal = crate::theme::Palette::of(ui);
                    egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                        ui.set_width(248.0);
                        crate::inspect::panel(ui, Some(&info), &reg, note.as_deref(), None);
                    });
                });
                p.run_steps(3);
                p.render().unwrap().save(dir.join(format!("info-{}-{ppp}x.png", if dark { "dark" } else { "light" }))).unwrap();
            }
        }
    }
}
