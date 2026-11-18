//! The water tools driven as a user drives them: mouse events at screen
//! points over the map of the whole app, at 1× and 2× (a Retina screen)
//! and at several zooms. The River tool's points land where clicked, a
//! click adds one and moves none, a drag moves only the point held, the
//! right button takes one away; Flood puts water wherever it is clicked,
//! with the settings a new user has and with a level set under the land.
//! With MASSIF_SNAPSHOT_DIR, each sequence is drawn as a strip of images.

use super::tests::{click_on, drag_on, harness_with};
use super::*;
use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use massif_core::TILE_SIZE;
use massif_core::generate::{GenParams, Landform, generate};

/// The whole app on a hilly map with sand by its lakes, at `ppp` pixels a
/// point, `tool` in hand and every setting as a new user has it.
fn hills(ppp: f32, tool: Tool) -> Harness<'static, MassifApp> {
    let mut h = harness_with(ThemeChoice::Dark, ppp);
    h.run_steps(3);
    {
        let app = h.state_mut();
        let world = generate(&GenParams {
            tiles_x: 8,
            tiles_z: 8,
            landform: Landform::Hills,
            ..GenParams::default()
        });
        app.doc = crate::doc::Document::new(world, None, 9);
        app.map_cache.invalidate_all();
        app.view3d.invalidate_all();
        app.tools = crate::tools::ToolState::default();
        app.tools.select(tool);
        app.tools.river.width = 8.0;
    }
    h.run_steps(5);
    h
}

fn map<'a>(h: &'a Harness<'_, MassifApp>) -> &'a crate::map_view::MapView {
    h.state().maps.values().next().expect("a map")
}

/// The map seen at `zoom` (pixels a block), its middle `shift` blocks
/// off the middle of the view.
fn view(h: &mut Harness<'_, MassifApp>, zoom: f32, shift: (f64, f64)) {
    let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
    let middle = (((lo.x + hi.x + 1) * TILE_SIZE) as f64 / 2.0, ((lo.z + hi.z + 1) * TILE_SIZE) as f64 / 2.0);
    let m = h.state_mut().maps.values_mut().next().unwrap();
    m.zoom = zoom;
    m.center = [middle.0 + shift.0, middle.1 + shift.1];
    h.run_steps(3);
}

/// Where to click: the map's canvas over the land, away from its edges,
/// the bar of the river and the messages at its foot.
fn area(h: &Harness<'_, MassifApp>) -> Rect {
    let m = map(h);
    let canvas = m.canvas();
    let (lo, hi) = h.state().doc.dimension().bounds().unwrap();
    let a = m.screen_of((lo.x * TILE_SIZE) as f64, (lo.z * TILE_SIZE) as f64);
    let b = m.screen_of(((hi.x + 1) * TILE_SIZE) as f64, ((hi.z + 1) * TILE_SIZE) as f64);
    let r = Rect::from_min_max(canvas.min + vec2(50.0, 50.0), canvas.max - vec2(50.0, 120.0)).intersect(Rect::from_two_pos(a, b).shrink(16.0));
    assert!(r.width() > 200.0 && r.height() > 150.0, "room to click: {r:?}");
    r
}

/// The block under screen point `p`.
fn block_at(h: &Harness<'_, MassifApp>, p: Pos2) -> (i32, i32) {
    let (x, z) = map(h).world_at(p);
    (x.floor() as i32, z.floor() as i32)
}

fn dry(h: &Harness<'_, MassifApp>, p: Pos2) -> bool {
    let (x, z) = block_at(h, p);
    let app = h.state();
    crate::tools::probe(&app.doc, x, z).is_some_and(|q| q.height > app.doc.world.meta.sea_level as f32 && massif_core::ops::water_at(app.doc.dimension(), x, z).is_none())
}

fn anchors(h: &Harness<'_, MassifApp>) -> Vec<(f32, f32)> {
    h.state().tools.river_draft.as_ref().map(|d| d.layout.anchors()).unwrap_or_default()
}

/// The screen points of the river's course.
fn course_on_screen(h: &Harness<'_, MassifApp>) -> Vec<Pos2> {
    let m = map(h);
    h.state().tools.river_draft.as_ref().map_or(Vec::new(), |d| d.plan.course.iter().map(|c| m.screen_of(c.0 as f64, c.1 as f64)).collect())
}

/// Whether a click at `p` would fall on the river drawn (its course, a
/// handle, the ring at its mouth), with some room.
fn on_river(h: &Harness<'_, MassifApp>, p: Pos2) -> bool {
    let app = h.state();
    let m = map(h);
    let reach = (app.tools.river.width * (1.0 + 2.0 * app.tools.river.flow) * m.zoom / 2.0).max(7.0) + 6.0;
    let course = course_on_screen(h);
    course.iter().any(|c| c.distance(p) <= reach) || anchors(h).iter().any(|a| m.screen_of(a.0 as f64, a.1 as f64).distance(p) < 40.0)
}

/// `p`, or the nearest point around it that is on dry land (for a source)
/// and off the river drawn.
fn free_spot(h: &Harness<'_, MassifApp>, p: Pos2, land: bool, area: Rect) -> Pos2 {
    for r in 0..40 {
        for k in 0..12 {
            let a = k as f32 * std::f32::consts::TAU / 12.0;
            let q = area.clamp(p + vec2(a.cos(), a.sin()) * (r as f32 * 9.0));
            if (!land || dry(h, q)) && !on_river(h, q) {
                return q;
            }
        }
    }
    panic!("no free spot near {p:?}");
}

/// The window drawn, cut to `r` (in points).
fn shot(h: &mut Harness<'_, MassifApp>, r: Rect) -> image::RgbaImage {
    let image = h.render().unwrap();
    let ppp = h.ctx.pixels_per_point();
    let (w, hh) = (image.width() as f32, image.height() as f32);
    let (x0, y0) = ((r.min.x * ppp).clamp(0.0, w - 1.0), (r.min.y * ppp).clamp(0.0, hh - 1.0));
    let (x1, y1) = ((r.max.x * ppp).clamp(x0 + 1.0, w), (r.max.y * ppp).clamp(y0 + 1.0, hh));
    image::imageops::crop_imm(&image, x0 as u32, y0 as u32, (x1 - x0) as u32, (y1 - y0) as u32).to_image()
}

/// Frames side by side, each `height` pixels high, with a caption strip
/// left to the frames themselves (the strips are for the eye).
fn save_strip(frames: &[image::RgbaImage], height: u32, path: &std::path::Path) {
    let scaled: Vec<image::RgbaImage> = frames
        .iter()
        .map(|f| {
            let w = (f.width() as f32 * height as f32 / f.height() as f32).round().max(1.0) as u32;
            image::imageops::resize(f, w, height, image::imageops::FilterType::Triangle)
        })
        .collect();
    let gap = 10;
    let width = scaled.iter().map(|f| f.width() + gap).sum::<u32>() + gap;
    let mut strip = image::RgbaImage::from_pixel(width, height + 2 * gap, image::Rgba([24, 24, 27, 255]));
    let mut x = gap;
    for f in &scaled {
        image::imageops::overlay(&mut strip, f, x as i64, gap as i64);
        x += f.width() + gap;
    }
    strip.save(path).unwrap();
}

/// Frames in rows of `columns`, each `width` pixels wide, read left to
/// right and top to bottom.
fn save_grid(frames: &[image::RgbaImage], columns: usize, width: u32, path: &std::path::Path) {
    let scaled: Vec<image::RgbaImage> = frames
        .iter()
        .map(|f| {
            let h = (f.height() as f32 * width as f32 / f.width() as f32).round().max(1.0) as u32;
            image::imageops::resize(f, width, h, image::imageops::FilterType::Triangle)
        })
        .collect();
    let gap = 12;
    let row_h = scaled.iter().map(|f| f.height()).max().unwrap_or(1);
    let rows = scaled.len().div_ceil(columns) as u32;
    let mut grid = image::RgbaImage::from_pixel(columns as u32 * (width + gap) + gap, rows * (row_h + gap) + gap, image::Rgba([24, 24, 27, 255]));
    for (k, f) in scaled.iter().enumerate() {
        let (c, r) = ((k % columns) as u32, (k / columns) as u32);
        image::imageops::overlay(&mut grid, f, (gap + c * (width + gap)) as i64, (gap + r * (row_h + gap)) as i64);
    }
    grid.save(path).unwrap();
}

fn snapshots() -> Option<std::path::PathBuf> {
    std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from)
}

fn status(h: &Harness<'_, MassifApp>) -> String {
    h.state().status.as_ref().map(|s| s.text.clone()).unwrap_or_default()
}

/// A press and its release within one frame, as a quick click on a slow
/// frame comes.
fn quick_click(h: &mut Harness<'_, MassifApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.step();
    for pressed in [true, false] {
        h.input_mut().events.push(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
    }
    h.step();
    h.run_steps(3);
}

fn key(h: &mut Harness<'_, MassifApp>, key: egui::Key) {
    for pressed in [true, false] {
        h.event(egui::Event::Key { key, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
    }
    h.run_steps(4);
}

/// The River tool, its points laid with the mouse at 1× and 2× and at
/// three zooms, the map moved under them: a click on the land is a point
/// more, where clicked (its block and its handle on screen), the last one;
/// no point already laid moves; held by its handle's edge and let go, a
/// point stays; dragged, the point held and no other moves, by as much as
/// the pointer; a click on the course puts a point between the two it
/// joins; the right button takes one away; a click let go within the frame
/// it was pressed counts; the course runs through every point; Esc drops
/// the river, Enter digs it.
#[test]
fn river_points_land_where_clicked() {
    let dir = snapshots();
    for ppp in [1.0, 2.0] {
        for (zoom, shift) in [(0.6, (0.0, 0.0)), (1.5, (-150.0, 90.0)), (4.0, (70.0, -40.0))] {
            let label = format!("{ppp}x, zoom {zoom}");
            let mut h = hills(ppp, Tool::River);
            view(&mut h, zoom, shift);
            let area = area(&h);
            let canvas = map(&h).canvas();
            let mut frames = Vec::new();
            let draw = dir.is_some() && zoom == 1.5;
            if draw {
                frames.push(shot(&mut h, canvas));
            }
            // The source, then three points, then the end, on a zigzag.
            let mut worst = 0.0f32;
            for (n, f) in [(0.1, 0.2), (0.32, 0.8), (0.52, 0.18), (0.72, 0.82), (0.92, 0.3)].into_iter().enumerate() {
                let p = free_spot(&h, area.lerp_inside(vec2(f.0, f.1)), n == 0, area);
                let before = anchors(&h);
                let clicked = map(&h).world_at(p);
                click_on(&mut h, p, PointerButton::Primary);
                let after = anchors(&h);
                assert_eq!(after.len(), before.len() + 1, "{label}: click {n} adds a point: {before:?} → {after:?}");
                assert_eq!(after[..before.len()], before[..], "{label}: click {n} moved no point");
                let new = *after.last().unwrap();
                let off = (new.0 as f64 - clicked.0).hypot(new.1 as f64 - clicked.1);
                assert!(off < 0.5, "{label}: point {n} where clicked: {new:?}, clicked {clicked:?}");
                let shown = map(&h).screen_of(new.0 as f64, new.1 as f64);
                assert!(shown.distance(p) < 1.0, "{label}: its handle under the pointer: {shown:?} {p:?}");
                worst = worst.max(off as f32);
                // The course shown goes to it.
                let draft = h.state().tools.river_draft.clone().unwrap();
                if n > 0 {
                    assert!(draft.layout.end.is_some(), "{label}: the last point is the end");
                    let last = *draft.plan.course.last().unwrap();
                    assert!((last.0 - new.0).hypot(last.1 - new.1) < 2.0, "{label}: the course ends at the last point: {last:?} {new:?}");
                }
                if draw {
                    frames.push(shot(&mut h, canvas));
                }
            }
            eprintln!("{label}: points within {worst:.3} blocks of the clicks");
            // Through every point.
            let draft = h.state().tools.river_draft.clone().unwrap();
            for (k, a) in draft.layout.anchors().iter().enumerate() {
                let d = draft.plan.course.iter().map(|c| (c.0 - a.0).hypot(c.1 - a.1)).fold(f32::MAX, f32::min);
                assert!(d < 2.0, "{label}: the course passes point {} ({d:.2} blocks off)", k + 1);
            }
            // Held by the edge of its handle and let go: it stays.
            let laid = anchors(&h);
            let second = map(&h).screen_of(laid[1].0 as f64, laid[1].1 as f64);
            click_on(&mut h, second + vec2(5.0, -4.0), PointerButton::Primary);
            assert_eq!(anchors(&h), laid, "{label}: a click on a handle moves nothing");
            // The second point dragged: it alone moves, by the pointer's way.
            let (from, to) = (second + vec2(3.0, 2.0), second + vec2(3.0 + 55.0, 2.0 - 42.0));
            drag_on(&mut h, from, to);
            let moved = anchors(&h);
            let want = map(&h).world_at(second + (to - from));
            assert_eq!(moved.len(), laid.len());
            for (k, (a, b)) in laid.iter().zip(&moved).enumerate() {
                if k == 1 {
                    assert!((b.0 as f64 - want.0).hypot(b.1 as f64 - want.1) < 0.5, "{label}: the point dragged follows the pointer: {b:?} {want:?}");
                } else {
                    assert_eq!(a, b, "{label}: point {} stays while the second is dragged", k + 1);
                }
            }
            if draw {
                frames.push(shot(&mut h, canvas));
            }
            // A click on the course between the third and fourth points: a
            // point between them, where clicked.
            let draft = h.state().tools.river_draft.clone().unwrap();
            let m = map(&h);
            let (s3, s4) = (draft.stops[2], draft.stops[3]);
            let handles: Vec<Pos2> = moved.iter().map(|a| m.screen_of(a.0 as f64, a.1 as f64)).collect();
            let pick = (s3..=s4).map(|i| (i, m.screen_of(draft.plan.course[i].0 as f64, draft.plan.course[i].1 as f64))).filter(|(_, q)| canvas.shrink(60.0).contains(*q) && handles.iter().all(|hd| hd.distance(*q) > 30.0)).nth((s4 - s3) / 3);
            if let Some((_, q)) = pick {
                let clicked = m.world_at(q);
                click_on(&mut h, q, PointerButton::Primary);
                let now = anchors(&h);
                assert_eq!(now.len(), moved.len() + 1, "{label}: a point more on the course");
                assert_eq!(now[..3], moved[..3]);
                assert_eq!(now[4..], moved[3..], "{label}: put between the third and fourth");
                assert!((now[3].0 as f64 - clicked.0).hypot(now[3].1 as f64 - clicked.1) < 0.5, "{label}: where clicked");
                if draw {
                    frames.push(shot(&mut h, canvas));
                }
                // The right button on it: taken away, the others stay.
                let at = map(&h).screen_of(now[3].0 as f64, now[3].1 as f64);
                click_on(&mut h, at, PointerButton::Secondary);
                assert_eq!(anchors(&h), moved, "{label}: the right button takes it away");
            } else {
                eprintln!("{label}: no room on the course between the third and fourth points");
            }
            // The right button on the end: the point before it is the end.
            let last = map(&h).screen_of(moved[4].0 as f64, moved[4].1 as f64);
            click_on(&mut h, last, PointerButton::Secondary);
            assert_eq!(anchors(&h), moved[..4], "{label}: the end taken away");
            assert_eq!(h.state().tools.river_draft.as_ref().unwrap().layout.end, Some(moved[3]));
            if draw {
                frames.push(shot(&mut h, canvas));
            }
            // A click pressed and let go within one frame.
            let p = free_spot(&h, area.lerp_inside(vec2(0.9, 0.75)), false, area);
            let clicked = map(&h).world_at(p);
            quick_click(&mut h, p);
            let now = anchors(&h);
            assert_eq!(now.len(), 5, "{label}: a quick click adds a point");
            assert_eq!(now[..4], moved[..4]);
            assert!((now[4].0 as f64 - clicked.0).hypot(now[4].1 as f64 - clicked.1) < 0.5);
            if draw {
                frames.push(shot(&mut h, canvas));
            }
            if ppp == 1.0 && zoom == 0.6 {
                // Esc drops it.
                key(&mut h, egui::Key::Escape);
                assert!(h.state().tools.river_draft.is_none(), "{label}: Esc drops the river");
            } else {
                // Enter digs it.
                key(&mut h, egui::Key::Enter);
                assert_eq!(h.state().doc.history.undo_label(), Some("River"), "{label}: Enter digs it");
                assert!(h.state().tools.river_draft.is_none());
                if draw {
                    frames.push(shot(&mut h, canvas));
                }
            }
            if let (Some(dir), true) = (&dir, draw) {
                save_grid(&frames, 3, 960, &dir.join(format!("river-points-{ppp}x.png")));
            }
        }
    }
}

/// The bar over a river being laid out takes the pointer: a click on Dig
/// river digs the river as laid out, no point added where it was clicked;
/// a click on Cancel drops it; a drag begun on the bar moves nothing; over
/// a button, the pointer is a hand.
#[test]
fn the_river_bar_takes_its_clicks() {
    let dir = snapshots();
    let mut h = hills(1.0, Tool::River);
    view(&mut h, 1.5, (0.0, 0.0));
    let area = area(&h);
    let canvas = map(&h).canvas();
    let lay = |h: &mut Harness<'_, MassifApp>| {
        for f in [(0.2, 0.3), (0.8, 0.6)] {
            let p = free_spot(h, area.lerp_inside(vec2(f.0, f.1)), true, area);
            click_on(h, p, PointerButton::Primary);
        }
        assert_eq!(anchors(h).len(), 2);
    };
    lay(&mut h);
    let mut frames = vec![shot(&mut h, canvas)];
    let [bar, dig, _] = map(&h).river_bar_rects().expect("the bar over the river");
    // Over Dig river: a hand.
    h.event(egui::Event::PointerMoved(dig.center()));
    h.run_steps(2);
    assert_eq!(h.output().platform_output.cursor_icon, egui::CursorIcon::PointingHand);
    if dir.is_some() {
        frames.push(shot(&mut h, canvas));
    }
    // A drag begun on the bar: nothing moves, nothing is added.
    let laid = anchors(&h);
    drag_on(&mut h, bar.left_center() + vec2(14.0, 0.0), bar.left_center() + vec2(60.0, -120.0));
    assert_eq!(anchors(&h), laid, "a drag from the bar reaches no point");
    // Pressed on Dig river: no point added; let go: dug as laid out.
    let steps = h.state().doc.history.undo_count();
    h.event(egui::Event::PointerMoved(dig.center()));
    h.step();
    h.event(egui::Event::PointerButton { pos: dig.center(), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(anchors(&h), laid, "pressing Dig river adds no point");
    h.event(egui::Event::PointerButton { pos: dig.center(), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(h.state().tools.river_draft.is_none(), "dug");
    assert_eq!(h.state().doc.history.undo_label(), Some("River"));
    assert_eq!(h.state().doc.history.undo_count(), steps + 1, "one step: the river");
    if dir.is_some() {
        frames.push(shot(&mut h, canvas));
    }
    // Cancel drops it, digging nothing, the message of the river just dug
    // still shown (over the bar, it took its clicks).
    lay(&mut h);
    let steps = h.state().doc.history.undo_count();
    // Its width follows what it says: where its Cancel button is now.
    let [_, _, cancel] = map(&h).river_bar_rects().expect("the bar again");
    h.event(egui::Event::PointerMoved(cancel.center()));
    h.run_steps(3);
    let [_, _, cancel] = map(&h).river_bar_rects().expect("the bar again");
    let before = anchors(&h);
    click_on(&mut h, cancel.center(), PointerButton::Primary);
    assert!(h.state().tools.river_draft.is_none(), "dropped: {cancel:?} {:?}, {before:?} → {:?}, {}", map(&h).river_bar_rects(), anchors(&h), status(&h));
    assert_eq!(h.state().doc.history.undo_count(), steps, "nothing dug");
    if let Some(dir) = dir {
        save_grid(&frames, 3, 960, &dir.join("river-bar.png"));
    }
}

/// Flood as a new user finds it, Where I click: on every dry spot of a
/// grid over hilly land, the tag by the cursor says what a click does, and
/// the click puts water (in the hollow under it, or in the one its water
/// runs down into), or, where no hollow holds it, asks how far to flood;
/// never nothing. The right button held lowers the water a block at a
/// time, Shift + right drains it.
#[test]
fn flood_puts_water_wherever_clicked() {
    let dir = snapshots();
    let mut h = hills(1.0, Tool::Flood);
    assert_eq!(h.state().tools.flood_height, crate::tools::WaterHeight::Start, "Where I click by default");
    view(&mut h, 3.0, (0.0, 0.0));
    let area = area(&h);
    let canvas = map(&h).canvas();
    let (mut here, mut ran, mut asked, mut spots) = (0, 0, 0, 0);
    let mut frames = Vec::new();
    for j in 0..5 {
        for i in 0..7 {
            let p = area.lerp_inside(vec2((i as f32 + 0.5) / 7.0, (j as f32 + 0.5) / 5.0));
            if !dry(&h, p) {
                continue;
            }
            spots += 1;
            h.event(egui::Event::PointerMoved(p));
            h.run_steps(3);
            let (tag, target) = map(&h).water_tag_shown().unwrap_or_else(|| panic!("a tag by the cursor at {p:?}"));
            let draw = dir.is_some() && frames.len() < 4 && target.is_some();
            if draw {
                frames.push(shot(&mut h, Rect::from_center_size(p, vec2(720.0, 420.0)).intersect(canvas)));
            }
            h.state_mut().status = None;
            let before = h.state().doc.history.undo_label().map(str::to_owned);
            click_on(&mut h, p, PointerButton::Primary);
            let said = status(&h);
            if matches!(h.state().dialog, Some(Dialog::FloodAsk(_))) {
                assert!(tag.starts_with("Not in a hollow"), "the tag said so: {tag}");
                eprintln!("asked at {:?}", block_at(&h, p));
                asked += 1;
                h.state_mut().dialog = None;
                continue;
            }
            assert_eq!(h.state().doc.history.undo_label(), Some("Flood"), "a click at {p:?} ({tag}) put water: {said} (before: {before:?})");
            let (x, z) = block_at(&h, p);
            let wet = massif_core::ops::water_at(h.state().doc.dimension(), x, z).is_some();
            if wet {
                assert!(said.starts_with("Flooded"), "{said}");
                assert!(tag.starts_with("Fill to y") || tag.starts_with("Fills the hollow"), "{tag}");
                here += 1;
            } else {
                assert!(said.starts_with("The water ran down into a hollow") || said.starts_with("No hollow here: filled the nearest one"), "{said}");
                assert!(tag.starts_with("Runs down into a hollow") || tag.starts_with("No hollow here: fills the nearest one") || tag.starts_with("Fills the hollow"), "{tag}");
                assert!(target.is_some(), "a ring where it goes: {tag}");
                let (tx, tz) = target.unwrap();
                assert!(massif_core::ops::water_at(h.state().doc.dimension(), tx, tz).is_some(), "water where the tag pointed");
                ran += 1;
            }
            if draw {
                frames.push(shot(&mut h, Rect::from_center_size(p, vec2(720.0, 420.0)).intersect(canvas)));
            }
            h.state_mut().doc.undo();
            h.run_steps(2);
        }
    }
    eprintln!("Flood on {spots} dry spots: {here} filled the hollow clicked, {ran} the one below, {asked} asked");
    assert!(spots >= 20 && here + ran + asked == spots);
    assert!(here + ran >= spots * 9 / 10, "water nearly everywhere: {here} + {ran} of {spots}");

    // Held on the water laid, the right button lowers it a block at a time;
    // Shift + right drains it whole: in a pit dug 10 blocks deep.
    let q = (0..400).map(|k| area.center() + vec2((k % 20) as f32 - 10.0, (k / 20) as f32 - 10.0) * 14.0).find(|&q| dry(&h, q)).expect("dry land near the middle");
    let (hx, hz) = block_at(&h, q);
    {
        let app = h.state_mut();
        let brush = massif_core::Brush { shape: massif_core::BrushShape::Circle, radius: 14.0, profile: massif_core::Profile::Constant, strength: 1.0, rotation: 0.0, mask: None, filter: None };
        let dim = app.doc.dim;
        app.doc.history.begin("Dig", dim);
        massif_core::ops::raise(&mut app.doc.history.edit(&mut app.doc.world, 0), &brush, hx as f32 + 0.5, hz as f32 + 0.5, -10.0, massif_core::ops::Surface::Top);
        app.doc.commit();
        app.map_cache.invalidate_all();
    }
    h.run_steps(3);
    h.event(egui::Event::PointerMoved(q));
    h.run_steps(3);
    let tag = map(&h).water_tag_shown();
    h.state_mut().status = None;
    click_on(&mut h, q, PointerButton::Primary);
    let full = massif_core::ops::water_at(h.state().doc.dimension(), hx, hz).unwrap_or_else(|| panic!("water in the pit: {tag:?} → {} ({:?}, ground {})", status(&h), h.state().doc.history.undo_label(), q_height(&h.state().doc, hx, hz)));
    if dir.is_some() {
        frames.push(shot(&mut h, Rect::from_center_size(q, vec2(720.0, 420.0)).intersect(canvas)));
    }
    h.event(egui::Event::PointerMoved(q));
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    // A block at the press, a block more after a beat (6 a second).
    h.run_steps(14);
    h.event(egui::Event::PointerButton { pos: q, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let said = status(&h);
    let after = massif_core::ops::water_at(h.state().doc.dimension(), hx, hz);
    assert!(after.is_none_or(|w| w < full - 1), "held, the right button lowered it more than a block: {full} → {after:?} ({said})");
    assert!(said.starts_with("Water lowered by") || said == "The water is gone", "{said}");
    eprintln!("held on the lake at y {full}: {said}");
    if dir.is_some() {
        frames.push(shot(&mut h, Rect::from_center_size(q, vec2(720.0, 420.0)).intersect(canvas)));
    }
    // Shift + right on the water left: drained whole.
    let wet = |h: &Harness<'_, MassifApp>| (-16..=16).flat_map(|dz| (-16..=16).map(move |dx| (hx + dx, hz + dz))).filter(|&(x, z)| massif_core::ops::water_at(h.state().doc.dimension(), x, z).is_some()).collect::<Vec<_>>();
    let left = wet(&h);
    let (wx, wz) = *left.first().expect("water left in the pit");
    let r = map(&h).screen_of(wx as f64 + 0.5, wz as f64 + 0.5);
    h.event(egui::Event::PointerMoved(r));
    h.step();
    h.event_modifiers(egui::Event::PointerButton { pos: r, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::SHIFT }, Modifiers::SHIFT);
    h.step();
    h.event(egui::Event::PointerButton { pos: r, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    let said = status(&h);
    assert!(said.starts_with("Drained"), "{said}");
    assert!(wet(&h).is_empty(), "Shift + right drained the pit: {said}, {} columns of {} left", wet(&h).len(), left.len());
    if dir.is_some() {
        frames.push(shot(&mut h, Rect::from_center_size(q, vec2(720.0, 420.0)).intersect(canvas)));
    }
    if let Some(dir) = dir {
        save_strip(&frames, 360, &dir.join("flood-where-i-click.png"));
    }
}

fn q_height(doc: &crate::doc::Document, x: i32, z: i32) -> f32 {
    crate::tools::probe(doc, x, z).map_or(f32::MAX, |q| q.height.floor())
}

/// Flood with a level set under the land clicked (Fixed, y 63, on hills
/// over 63): no dead end. The tag says the land is above the level and
/// what the click does instead; the click fills the hollow the water runs
/// into, and says so. Under the level, the level set still rules.
#[test]
fn flood_above_a_fixed_level_fills_the_hollow() {
    let dir = snapshots();
    let mut h = hills(1.0, Tool::Flood);
    {
        let tools = &mut h.state_mut().tools;
        tools.flood_height = crate::tools::WaterHeight::Fixed;
        tools.flood_height_chosen = true;
        tools.water_level = 63;
    }
    view(&mut h, 3.0, (0.0, 0.0));
    let area = area(&h);
    let canvas = map(&h).canvas();
    let mut frames = Vec::new();
    let mut done = 0;
    for k in 0..40 {
        let p = area.lerp_inside(vec2((k % 8) as f32 / 8.0 + 0.06, (k / 8) as f32 / 5.0 + 0.1));
        let (x, z) = block_at(&h, p);
        if !dry(&h, p) || q_height(&h.state().doc, x, z) < 64.0 {
            continue;
        }
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(3);
        let (tag, _) = map(&h).water_tag_shown().expect("a tag");
        if tag.contains("not in a hollow") {
            continue;
        }
        assert!(tag.starts_with("Above y 63: "), "{tag}");
        if dir.is_some() && frames.len() < 2 {
            frames.push(shot(&mut h, Rect::from_center_size(p, vec2(720.0, 420.0)).intersect(canvas)));
        }
        h.state_mut().status = None;
        click_on(&mut h, p, PointerButton::Primary);
        let said = status(&h);
        assert_eq!(h.state().doc.history.undo_label(), Some("Flood"), "{tag} → {said}");
        assert!(said.starts_with("The land is above y 63: "), "{said}");
        if dir.is_some() && frames.len() < 4 {
            frames.push(shot(&mut h, Rect::from_center_size(p, vec2(720.0, 420.0)).intersect(canvas)));
        }
        h.state_mut().doc.undo();
        h.run_steps(2);
        done += 1;
        if done >= 6 {
            break;
        }
    }
    assert!(done >= 3, "spots over y 63 in a hollow or over one: {done}");
    // Under the level set: up to it, as before.
    h.state_mut().tools.water_level = 200;
    let p = (0..40).map(|k| area.lerp_inside(vec2((k % 8) as f32 / 8.0 + 0.06, (k / 8) as f32 / 5.0 + 0.1))).find(|&p| dry(&h, p)).unwrap();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    assert_eq!(map(&h).water_tag_shown().unwrap().0, "Fill to y 200");
    if let Some(dir) = dir {
        save_strip(&frames, 360, &dir.join("flood-fixed-above.png"));
    }
}

/// Settings of an earlier version: Flood's height never picked in Tool
/// Settings comes back to Where I click; picked, it stays.
#[test]
fn flood_comes_back_to_where_i_click() {
    use crate::tools::{ToolState, WaterHeight};
    let mut old: ToolState = serde_json::from_str(r#"{"flood_height": "Fixed", "brush_height": "Fixed", "water_level": 63}"#).unwrap();
    old.settle();
    assert_eq!((old.flood_height, old.brush_height, old.water_level), (WaterHeight::Start, WaterHeight::Fixed, 63));
    let mut chosen: ToolState = serde_json::from_str(r#"{"flood_height": "Sea", "flood_height_chosen": true}"#).unwrap();
    chosen.settle();
    assert_eq!(chosen.flood_height, WaterHeight::Sea);
    let saved: ToolState = serde_json::from_str(&serde_json::to_string(&chosen).unwrap()).unwrap();
    assert!(saved.flood_height_chosen);
}
