//! The tree sizes of the map, as Tree sizes paints them and objects are
//! sorted into: the four of every map (Bushes, Small, Medium and Large
//! trees) and the user's own (Tiny, Forest, Open-grown…), each with its
//! colour over the map. How far apart trees stand is their canopies'
//! business (see `massif_core::objects::Spacing`); each layer sets the
//! density of each size.

use crate::theme::{self, Icon, Palette};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use massif_core::registry::{Registry, TreeSize};

/// The names of the four sizes of every map, translated where shown.
pub const BUILT_IN: [&str; 4] = [tn!("Bushes"), tn!("Small trees"), tn!("Medium trees"), tn!("Large trees")];

/// The name of tree size `n` of `reg` (from 1), in the language shown for
/// the four of every map.
pub fn label(reg: &Registry, n: u16) -> String {
    match reg.tree_size(n) {
        Some(t) => name_of(t),
        None => tr!("Any size").to_owned(),
    }
}

/// A size's name as shown.
pub fn name_of(t: &TreeSize) -> String {
    if BUILT_IN.contains(&t.name.as_str()) { crate::i18n::tr_name(&t.name).to_owned() } else { t.name.clone() }
}

/// The sizes of `reg` that may be painted: by number, those not removed.
pub fn live(reg: &Registry) -> impl Iterator<Item = (u16, &TreeSize)> {
    reg.tree_sizes.iter().enumerate().filter(|(_, t)| !t.removed).map(|(k, t)| (k as u16 + 1, t))
}

/// The tree sizes as swatches `size` points wide in their colours, the one
/// of `picked` ringed, each named beside it when `named`; returns the one
/// clicked.
pub fn swatches(ui: &mut Ui, reg: &Registry, size: f32, picked: Option<u16>, named: bool) -> Option<u16> {
    let pal = Palette::of(ui);
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(size / 4.0, size / 6.0);
        let count = live(reg).count().max(1) as f32;
        for (rank, (n, t)) in live(reg).enumerate() {
            let name = name_of(t);
            let width = if named { size + 8.0 + ui.painter().layout_no_wrap(name.clone(), egui::FontId::proportional(12.0), pal.text).size().x + 8.0 } else { size };
            let (rect, response) = ui.allocate_exact_size(vec2(width, size), Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, picked == Some(n), &name));
            let painter = ui.painter();
            let radius = theme::TILE_RADIUS / 2;
            let swatch = Rect::from_min_size(rect.min, vec2(size, size)).shrink(size / 8.0);
            painter.rect(swatch, radius, theme::rgb(t.color), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            // A tree, as tall as its place among the sizes.
            let h = swatch.height() * (0.4 + 0.5 * (rank as f32 + 1.0) / count);
            let foot = swatch.center_bottom() - vec2(0.0, swatch.height() * 0.12);
            painter.circle_filled(foot - vec2(0.0, h * 0.6), h * 0.35, Color32::from_black_alpha(90));
            painter.line_segment([foot, foot - vec2(0.0, h * 0.4)], Stroke::new(1.5, Color32::from_black_alpha(110)));
            if named {
                painter.text(rect.left_center() + vec2(size + 6.0, 0.0), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(12.0), pal.text);
            }
            if picked == Some(n) {
                painter.rect_stroke(swatch.expand(2.5), radius, Stroke::new(2.0, pal.accent), StrokeKind::Outside);
            }
            let response = if named { response } else { response.on_hover_text(&name) };
            if response.clicked() {
                clicked = Some(n);
            }
        }
    });
    clicked
}

/// The sizes of the map, to edit: each one's colour and name, a size of
/// one's own to add, one's own to remove. Returns whether
/// anything changed.
pub fn editor(ui: &mut Ui, reg: &mut Registry) -> bool {
    let before = reg.tree_sizes.clone();
    let mut remove = None;
    crate::widgets::well(ui, |ui| {
        for (k, t) in reg.tree_sizes.iter_mut().enumerate() {
            if t.removed {
                continue;
            }
            let own = k >= BUILT_IN.len();
            let label = name_of(t);
            crate::widgets::row(ui, "", |ui| {
                if own {
                    let x = ui.add(egui::Button::new(RichText::new(theme::glyph(Icon::X).to_string()).size(12.0)).frame(false));
                    x.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tr!("Remove this size")));
                    if x.on_hover_text(tr!("Remove this size: where it is painted, any size stands")).clicked() {
                        remove = Some(k);
                    }
                }
                // Its name, the four of every map shown in the language of
                // the interface until they are renamed.
                let mut name = label.clone();
                let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width((ui.available_width() - 56.0).max(40.0)));
                r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr!("Name")));
                if r.changed() && !name.trim().is_empty() {
                    t.name = name;
                }
                let mut c = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour over the map")).changed() {
                    t.color = [c.r(), c.g(), c.b()];
                }
            });
        }
    });
    if let Some(k) = remove {
        reg.tree_sizes[k].removed = true;
    }
    ui.add_space(4.0);
    if crate::widgets::button(ui, theme::with_icon(Icon::Plus, tr!("New size"))).on_hover_text(tr!("A size of your own: Tiny, Forest, Open-grown…")).clicked() {
        add(reg);
    }
    reg.tree_sizes != before
}

/// A size of one's own added to the map, named Size (Size 2…): its number,
/// `None` when the map has as many as it can.
pub fn add(reg: &mut Registry) -> Option<u16> {
    if reg.tree_sizes.len() >= massif_core::tile::SIZE_MAX as usize {
        return None;
    }
    let taken: Vec<String> = live(reg).map(|(_, t)| name_of(t)).collect();
    let name = crate::layers::unique_name(tr!("Size"), taken.iter().map(String::as_str));
    // A colour between the greens of the four.
    let hue = reg.tree_sizes.len() as f32 * 0.17;
    let color = [(110.0 + 90.0 * hue.sin()) as u8, (150.0 + 60.0 * (hue * 1.3).cos()) as u8, (90.0 + 70.0 * (hue * 0.7).sin()) as u8];
    reg.tree_sizes.push(TreeSize { name, color, removed: false });
    Some(reg.tree_sizes.len() as u16)
}

/// What [`object_size`] returns for New size…: a size to add and give.
pub const NEW_SIZE: u8 = u8::MAX;

/// The size of an object, by hand or guessed: a choice of every size of
/// the map, and Auto (the guess, named), and New size… ([`NEW_SIZE`]).
/// Returns the size picked (0: Auto) when it changed.
pub fn object_size(ui: &mut Ui, reg: &Registry, id: impl egui::AsIdSalt, now: u8, guess: u8) -> Option<u8> {
    let text = |n: u8| if n == 0 { tr!("Auto ({size})", size = label(reg, guess as u16)) } else { label(reg, n as u16) };
    let mut pick = now;
    let r = crate::widgets::combo(id).selected_text(text(now)).show_ui(ui, |ui| {
        ui.selectable_value(&mut pick, 0, text(0));
        for (n, _) in live(reg) {
            ui.selectable_value(&mut pick, n as u8, label(reg, n));
        }
        if reg.tree_sizes.len() < massif_core::tile::SIZE_MAX as usize {
            ui.separator();
            ui.selectable_value(&mut pick, NEW_SIZE, theme::with_icon(Icon::Plus, tr!("New size…"))).on_hover_text(tr!("A size of your own, given to it: rename it with the Tree sizes tool"));
        }
    });
    r.response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr!("Tree size")));
    (pick != now).then_some(pick)
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn draw(ui: &mut Ui, reg: &mut Registry) {
        theme::install(ui.ctx());
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            swatches(ui, reg, 26.0, Some(5), true);
            ui.add_space(6.0);
            editor(ui, reg);
        });
    }

    fn registry() -> Registry {
        let mut reg = Registry::vanilla();
        reg.tree_sizes.push(TreeSize { name: "Forest".into(), color: [90, 120, 60], removed: false });
        reg.tree_sizes.push(TreeSize { name: "Open-grown".into(), color: [150, 170, 70], removed: false });
        reg
    }

    /// A size of one's own is added, named and removed; the four of
    /// every map stay; the map saves its sizes only when they are not the
    /// four as they come.
    #[test]
    fn sizes_are_added_and_removed() {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 600.0)).build_ui_state(draw, Registry::vanilla());
        h.run_steps(2);
        h.get_by_label_contains("New size").click();
        h.run_steps(2);
        assert_eq!(h.state().tree_sizes.len(), 5);
        assert_eq!(h.state().tree_sizes[4].name, "Size");
        let json = serde_json::to_string(&Registry::vanilla()).unwrap();
        assert!(!json.contains("tree_sizes"), "the four as they come are not written");
        let reg = h.state_mut();
        reg.tree_sizes[4].color = [1, 2, 3];
        let back: Registry = serde_json::from_str(&serde_json::to_string(reg).unwrap()).unwrap();
        assert_eq!(back.tree_sizes, reg.tree_sizes);
        h.run_steps(2);
        h.get_by_label("Size");
        assert_eq!(label(h.state(), 5), "Size");
        assert_eq!(label(h.state(), 2), "Small trees");
        assert_eq!(label(h.state(), 0), "Any size");
        // Removed: no longer offered, its number kept.
        h.get_by_label("Remove this size").click();
        h.run_steps(2);
        assert!(h.state().tree_sizes[4].removed && h.state().tree_size(5).is_none());
        assert_eq!(live(h.state()).count(), 4);
    }

    /// The sizes and their editor, with two of one's own: offscreen in both
    /// themes. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn tree_sizes_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp: f32 = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        for dark in [true, false] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 380.0))
                .with_pixels_per_point(ppp)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, reg: &mut Registry| {
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        draw(ui, reg);
                    },
                    registry(),
                );
            h.run_steps(3);
            let suffix = if ppp > 1.5 { "2x" } else { "1x" };
            h.render().unwrap().save(dir.join(format!("tree-sizes-{}-{suffix}.png", if dark { "dark" } else { "light" }))).unwrap();
        }
    }

    /// A forest of narrow and wide trees mixed (Forest size), the land
    /// seen from above, each crown a disc: on a grid of 14 as before (left),
    /// spaced by their canopies (right). Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn forest_before_after_render() {
        use massif_core::compose::{Materializer, TileStack};
        use massif_core::objects::{ObjectDef, ObjectLayerDef, Spacing};
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp: f32 = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        let crowned = |name: &str, r: i32| {
            let side = (2 * r + 1) as usize;
            let mut blocks = vec![0u16; side * 4 * side];
            for y in 0..3 {
                blocks[r as usize + r as usize * side + y * side * side] = 1;
            }
            for z in 0..side {
                for x in 0..side {
                    let (dx, dz) = (x as i32 - r, z as i32 - r);
                    if dx * dx + dz * dz <= r * r {
                        blocks[x + z * side + 3 * side * side] = 2;
                    }
                }
            }
            ObjectDef::new(name, [side as u16, 4, side as u16], vec!["minecraft:air".into(), "minecraft:oak_log[axis=y]".into(), "minecraft:oak_leaves".into()], blocks)
        };
        let mut w = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 2, tiles_z: 2, seed: 5, ..Default::default() });
        for (name, r) in [("Narrow", 1), ("Slim", 2), ("Wide", 5), ("Broad", 6)] {
            w.registry.objects.push(crowned(name, r));
        }
        let mut forest = ObjectLayerDef::new("Forest", &[0, 1, 2, 3], &w.registry);
        (forest.soil_only, forest.density, forest.spacing) = (false, 1.0, 14);
        let n = w.registry.add_object_layer(forest);
        let flat = massif_core::Brush { radius: 128.0, shape: massif_core::BrushShape::Square, profile: massif_core::Profile::Constant, strength: 1.0, ..Default::default() };
        let mut h = massif_core::history::History::default();
        h.begin("forest", 0);
        massif_core::ops::set_height(&mut h.edit(&mut w, 0), &flat, 0.0, 0.0, 80.0, massif_core::ops::Surface::Top);
        massif_core::ops::paint_objects(&mut h.edit(&mut w, 0), &flat, 0.0, 0.0, n, 1);
        h.commit();
        let placed = |w: &massif_core::World| {
            let dim = &w.dims[0];
            let mat = Materializer::new(&w.registry, w.meta.seed, dim.min_y);
            let mut all = Vec::new();
            for pos in [massif_core::TilePos::new(-1, -1), massif_core::TilePos::new(0, -1), massif_core::TilePos::new(-1, 0), massif_core::TilePos::new(0, 0)] {
                all.extend(massif_core::objects::tile_placements(&w.registry, &mat, 1, false, &TileStack::new(dim, pos, true).unwrap(), pos));
            }
            all.into_iter().map(|p| (p.x, p.z, w.registry.objects[p.object as usize].canopy_radius(), p.object >= 2)).collect::<Vec<_>>()
        };
        let before = placed(&w);
        w.registry.object_layers[0].spacing_mode = Spacing::Canopy;
        let after = placed(&w);
        for dark in [true, false] {
            let (before, after) = (before.clone(), after.clone());
            let mut h = egui_kittest::Harness::builder().with_size(vec2(620.0, 340.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui(move |ui| {
                theme::install(ui.ctx());
                ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                let pal = Palette::of(ui);
                egui::Frame::new().fill(pal.panel).inner_margin(10).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        for (title, trees) in [("On a grid of 14 (before)", &before), ("By canopy (after)", &after)] {
                            ui.vertical(|ui| {
                                ui.label(RichText::new(title).strong());
                                let (rect, _) = ui.allocate_exact_size(vec2(290.0, 290.0), Sense::hover());
                                let painter = ui.painter_at(rect);
                                painter.rect_filled(rect, 4, Color32::from_rgb(112, 92, 60));
                                // A square of 128 blocks, 2.25 points a block.
                                let k = rect.width() / 128.0;
                                let to = |x: f32, z: f32| rect.min + vec2((x + 64.0) * k, (z + 64.0) * k);
                                for &(x, z, r, wide) in trees.iter() {
                                    let c = to(x as f32 + 0.5, z as f32 + 0.5);
                                    let fill = if wide { Color32::from_rgb(40, 104, 52) } else { Color32::from_rgb(116, 170, 74) };
                                    painter.circle(c, r * k, fill, Stroke::new(1.0, Color32::from_black_alpha(90)));
                                    painter.circle_filled(c, 1.2, Color32::from_rgb(70, 46, 24));
                                }
                            });
                        }
                    });
                });
            });
            h.run_steps(2);
            let suffix = if ppp > 1.5 { "2x" } else { "1x" };
            h.render().unwrap().save(dir.join(format!("forest-before-after-{}-{suffix}.png", if dark { "dark" } else { "light" }))).unwrap();
        }
    }

    /// An object layer's settings, spaced by canopy, with the density of each
    /// size (the four of every map, and Forest): offscreen in both themes.
    #[test]
    fn size_density_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp: f32 = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        struct S {
            tools: crate::tools::ToolState,
            doc: crate::doc::Document,
            blocks: crate::blocks::BlockLibrary,
        }
        for dark in [true, false] {
            let world = massif_core::generate::generate(&massif_core::generate::GenParams { tiles_x: 1, tiles_z: 1, ..Default::default() });
            let mut doc = crate::doc::Document::new(world, None, 1);
            let reg = &mut doc.world.registry;
            reg.tree_sizes.push(TreeSize { name: "Forest".into(), color: [90, 120, 60], removed: false });
            reg.objects.push(massif_core::objects::ObjectDef::new("Oak", [1, 2, 1], vec!["minecraft:air".into(), "minecraft:oak_log".into()], vec![1, 1]));
            let mut layer = massif_core::objects::ObjectLayerDef::new("Mixed forest", &[0], reg);
            layer.spacing_mode = massif_core::objects::Spacing::Canopy;
            layer.size_density = vec![0.4, 1.0, 1.0, 0.6, 0.8];
            reg.add_object_layer(layer);
            let tools = crate::tools::ToolState { object_layer: 1, ..Default::default() };
            let state = S { tools, doc, blocks: crate::blocks::BlockLibrary::default() };
            let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 640.0)).with_pixels_per_point(ppp).with_accessibility_check(false).wgpu().build_ui_state(
                move |ui, s: &mut S| {
                    theme::install(ui.ctx());
                    ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                    let pal = Palette::of(ui);
                    egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        crate::panels::object_layer_editor(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
                    });
                },
                state,
            );
            h.run_steps(3);
            let suffix = if ppp > 1.5 { "2x" } else { "1x" };
            h.render().unwrap().save(dir.join(format!("size-density-{}-{suffix}.png", if dark { "dark" } else { "light" }))).unwrap();
        }
    }
}
