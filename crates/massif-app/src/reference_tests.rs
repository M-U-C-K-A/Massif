//! Reference images on the 2D map: the whole app, at 1× (and 2× with
//! MASSIF_SNAPSHOT_PPP=2); with MASSIF_SNAPSHOT_DIR, renders of each.

use super::tests::{drag_on, harness_with};
use super::*;
use egui_kittest::Harness;
use massif_core::reference::ReferenceImage;

fn ppp() -> f32 {
    std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

fn snapshot_dir() -> Option<PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from)
}

fn save(h: &mut Harness<'_, MassifApp>, name: &str) {
    if let Some(dir) = snapshot_dir() {
        let _ = std::fs::create_dir_all(&dir);
        h.render().expect("rendered").save(dir.join(format!("{name}@{}x.png", ppp()))).unwrap();
    }
}

fn map_id(h: &Harness<'_, MassifApp>) -> AreaId {
    h.state().active_map.or_else(|| h.state().maps.keys().next().copied()).expect("a map view")
}

/// A plan to trace over: a grid of streets on paper, a river across it.
fn plan_picture(path: &std::path::Path) {
    let (w, hh) = (600u32, 400u32);
    let img = image::RgbaImage::from_fn(w, hh, |x, y| {
        let river = ((x as f32 * 0.02).sin() * 40.0 + 200.0 - y as f32).abs() < 14.0;
        let street = x % 60 < 5 || y % 50 < 5;
        if river {
            image::Rgba([40, 110, 220, 255])
        } else if street {
            image::Rgba([200, 40, 40, 255])
        } else {
            image::Rgba([245, 240, 225, 255])
        }
    });
    img.save(path).unwrap();
}

/// A picture laid under the map and another over it, placed by their
/// handles: the map shows faded over the one under it; dragging the image
/// moves it and paints nothing; it is saved with the map.
#[test]
fn reference_images_lie_over_and_under_the_map_and_move_by_their_handles() {
    let dir = std::env::temp_dir().join(format!("massif-references-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let plan = dir.join("plan.png");
    plan_picture(&plan);
    let mut h = harness_with(ThemeChoice::Dark, ppp());
    h.run_steps(5);
    let id = map_id(&h);
    {
        let view = h.state_mut().maps.get_mut(&id).unwrap();
        view.center = [0.0, 0.0];
        view.zoom = 1.0;
    }
    h.run_steps(5);
    let before = h.state_mut().doc.world.dims[0].snapshot();
    h.state_mut().add_references(std::slice::from_ref(&plan), Some((0.0, 0.0)));
    h.run_steps(5);
    {
        let r = &h.state().doc.world.meta.references[0];
        assert_eq!(r.size, [600, 400], "read, its size taken");
        assert_eq!((r.x, r.z, r.scale), (0.0, 0.0, 1.0), "a block a pixel where it was put: smaller than the map");
    }
    // Under the map: the relief fades over it.
    {
        let r = &mut h.state_mut().doc.world.meta.references[0];
        r.above = false;
        r.opacity = 0.75;
    }
    h.state_mut().references.placing = false;
    h.run_steps(20);
    save(&mut h, "reference-under");
    let image = h.render().expect("rendered");
    let p = h.state().maps[&id].screen_of(-270.0, -170.0);
    let px = image.get_pixel((p.x * ppp()) as u32, (p.y * ppp()) as u32).0;
    assert!(px[0] > 70, "the red street shows through the faded map: {px:?}");
    // Over the map, with its handles.
    {
        let r = &mut h.state_mut().doc.world.meta.references[0];
        r.above = true;
        r.opacity = 0.6;
        r.rotation = 12.0;
    }
    h.state_mut().references.placing = true;
    h.run_steps(10);
    save(&mut h, "reference-over");
    // Dragged by its middle: it moves, the land does not change.
    h.state_mut().tools.select(Tool::Raise);
    let from = h.state().maps[&id].screen_of(10.0, 10.0);
    let zoom = h.state().maps[&id].zoom;
    drag_on(&mut h, from, from + egui::vec2(80.0, 40.0));
    let r = h.state().doc.world.meta.references[0].clone();
    assert!((r.x - 80.0 / zoom as f64).abs() < 2.0 && (r.z - 40.0 / zoom as f64).abs() < 2.0, "moved by the drag: {} {}", r.x, r.z);
    let after = h.state().doc.world.dims[0].ground();
    assert!(after.positions().all(|p| after.view(p).unwrap().top == before.ground().view(p).unwrap().top), "the drag raised nothing");
    // A corner pulled out scales it.
    let corner = r.at(1.0, 1.0);
    let c = h.state().maps[&id].screen_of(corner.0, corner.1);
    drag_on(&mut h, c, c + egui::vec2(60.0, 40.0));
    assert!(h.state().doc.world.meta.references[0].scale > 1.05, "scaled by its corner");
    h.state_mut().doc.world.meta.references[0].locked = true;
    // Kept with the map.
    let file = dir.join("traced.massif");
    massif_format::save(&h.state().doc.world, &file).unwrap();
    let back = massif_format::load(&file).unwrap();
    // As it was, to the last bit or so of its floats (JSON).
    let (a, b) = (&back.meta.references[0], &h.state().doc.world.meta.references[0]);
    assert!((a.x - b.x).abs() < 1e-9 && (a.z - b.z).abs() < 1e-9);
    assert_eq!(ReferenceImage { x: b.x, z: b.z, ..a.clone() }, *b);
    let _ = std::fs::remove_dir_all(&dir);
}
