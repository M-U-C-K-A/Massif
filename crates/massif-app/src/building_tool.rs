//! The Building tool: one building of a village placed from a pack,
//! selected on the map, then moved, turned, taken away, swapped for another
//! of the pack or drawn again. Each change is kept in the village's recipe
//! (its `edits`) and the village built again, as one step to undo; it is
//! made again each time the village is built with the same draw.

use crate::doc::Document;
use crate::structure_tool::{label, recipe_of};
use crate::theme::{self, Icon, Palette};
use crate::tools::{Feedback, ToolState};
use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::structures::{Piece, Structure};
use massif_structures::placed::Recipe;
use massif_structures::village::edits::{BuildingEdit, file_name};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex};

/// A drag: where it began and where the pointer is, in blocks.
type Drag = Option<((f64, f64), (f64, f64))>;

/// The tool's state: the building selected (by its key, which holds when
/// its village is built again), and a move under way.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BuildingState {
    #[serde(skip)]
    pub selected: Option<[i32; 2]>,
    /// A drag: where it began and where the pointer is, in blocks.
    #[serde(skip)]
    pub drag: Arc<Mutex<Drag>>,
}

impl PartialEq for BuildingState {
    fn eq(&self, other: &Self) -> bool {
        self.selected == other.selected
    }
}

/// The village and piece of building `key`.
pub fn find(doc: &Document, key: [i32; 2]) -> Option<(&Structure, &Piece)> {
    doc.dimension().structures.iter().rev().find_map(|s| s.pieces.iter().find(|p| p.key == Some(key)).map(|p| (&**s, p)))
}

/// The building at column `(x, z)`: a piece of a village built from a pack
/// (one that keeps its recipe).
fn hit(doc: &Document, x: i32, z: i32) -> Option<[i32; 2]> {
    doc.dimension().structures.iter().rev().filter(|s| matches!(recipe_of(s), Some(Recipe::Village { .. }))).find_map(|s| s.pieces.iter().find(|p| p.key.is_some() && x >= p.min[0] && x <= p.max[0] && z >= p.min[2] && z <= p.max[2]).and_then(|p| p.key))
}

/// Whether `(x, z)` lies on a village the tool cannot change one building
/// of (the game's own, or one placed before 1.2).
fn other_village(doc: &Document, x: i32, z: i32) -> bool {
    doc.dimension().structures.iter().any(|s| crate::structure_tool::footprint(s).contains(x, z) && s.pieces.iter().all(|p| p.key.is_none()))
}

/// The edit of building `key` in `recipe`, as `change` leaves it: the
/// village's recipe with it.
fn edited(recipe: &Recipe, key: [i32; 2], change: impl FnOnce(&mut BuildingEdit)) -> Option<Recipe> {
    let Recipe::Village { pack, theme, centre, size, params, seed } = recipe else { return None };
    let mut params = params.clone();
    let k = match params.edits.iter().position(|e| e.at == key) {
        Some(k) => k,
        None => {
            params.edits.push(BuildingEdit::new(key));
            params.edits.len() - 1
        }
    };
    change(&mut params.edits[k]);
    if params.edits[k].is_empty() {
        params.edits.remove(k);
    }
    Some(Recipe::Village { pack: pack.clone(), theme: theme.clone(), centre: *centre, size: *size, params, seed: *seed })
}

/// The edit made to building `key` so far.
pub fn edit_of(doc: &Document, key: [i32; 2]) -> Option<BuildingEdit> {
    let (s, _) = find(doc, key)?;
    let Some(Recipe::Village { params, .. }) = recipe_of(s) else { return None };
    params.edits.into_iter().find(|e| e.at == key)
}

/// Change building `key` as `change` says, its village built again (one
/// step to undo). What to say of it.
pub fn change(doc: &Document, tools: &mut ToolState, key: [i32; 2], what: &str, change: impl FnOnce(&mut BuildingEdit)) -> Feedback {
    let (s, _) = find(doc, key)?;
    let recipe = edited(&recipe_of(s)?, key, change)?;
    // At once, not after the settings rest: one change, one step.
    crate::structure_tool::redo(tools, s.id, recipe);
    Some(what.to_owned())
}

/// Turn building `key` a quarter turn clockwise.
pub fn turn(doc: &Document, tools: &mut ToolState, key: [i32; 2]) -> Feedback {
    change(doc, tools, key, tr!("Turning it…"), |e| e.turn = (e.turn + 1) % 4)
}

/// Take building `key` away.
pub fn remove(doc: &Document, tools: &mut ToolState, key: [i32; 2]) -> Feedback {
    tools.building.selected = None;
    change(doc, tools, key, tr!("Taking it away…"), |e| e.removed = true)
}

/// The pointer pressed on the map at `at`: the building there selected,
/// ready to be dragged.
pub fn press(doc: &Document, tools: &mut ToolState, at: (f64, f64)) -> Feedback {
    let (x, z) = (at.0.floor() as i32, at.1.floor() as i32);
    match hit(doc, x, z) {
        Some(key) => {
            let first = tools.building.selected != Some(key);
            tools.building.selected = Some(key);
            *tools.building.drag.lock().expect("drag") = Some((at, at));
            first.then(|| {
                let kind = find(doc, key).map(|(_, p)| crate::structure_tool::type_name(&p.template, 1)).unwrap_or_default();
                tr!("{kind} selected: drag it to move it, R turns it, Delete takes it away", kind = capitalised(&kind))
            })
        }
        None => {
            tools.building.selected = None;
            other_village(doc, x, z).then(|| tr!("Only the buildings of a village built from a pack, placed with this version, can be changed one by one").to_owned())
        }
    }
}

fn capitalised(s: &str) -> String {
    let mut c = s.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The pointer, held, at `at`.
pub fn drag(tools: &ToolState, at: (f64, f64)) {
    if let Some(d) = tools.building.drag.lock().expect("drag").as_mut() {
        d.1 = at;
    }
}

/// Whether a drag is under way.
pub fn dragging(tools: &ToolState) -> bool {
    tools.building.drag.lock().expect("drag").is_some()
}

/// The pointer let go: the building moved where it was dragged.
pub fn release(doc: &Document, tools: &mut ToolState) -> Feedback {
    let (from, to) = tools.building.drag.lock().expect("drag").take()?;
    let key = tools.building.selected?;
    let (dx, dz) = ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32);
    if dx == 0 && dz == 0 {
        return None;
    }
    change(doc, tools, key, tr!("Moving it…"), |e| e.offset = [e.offset[0] + dx, e.offset[1] + dz])
}

/// The selected building on the map, outlined with its door; while it is
/// dragged, where it will go; the one under the pointer, faintly.
pub fn draw(ctx: &egui::Context, painter: &egui::Painter, screen: &dyn Fn((f32, f32)) -> Pos2, hover: Option<(i32, i32)>, doc: &Document, tools: &ToolState) {
    let pal = {
        let v = ctx.global_style().visuals.clone();
        Palette::themed(v.dark_mode, theme::is_high(&v))
    };
    let rect_of = |p: &Piece, dx: i32, dz: i32| -> Rect { Rect::from_two_pos(screen(((p.min[0] + dx) as f32, (p.min[2] + dz) as f32)), screen(((p.max[0] + 1 + dx) as f32, (p.max[2] + 1 + dz) as f32))) };
    // The buildings that can be picked, faintly, and the one under the
    // pointer.
    let hovered = hover.and_then(|(x, z)| hit(doc, x, z));
    for s in doc.dimension().structures.iter().filter(|s| matches!(recipe_of(s), Some(Recipe::Village { .. }))) {
        for p in s.pieces.iter().filter(|p| p.key.is_some()) {
            let r = rect_of(p, 0, 0);
            let on = p.key == hovered && p.key != tools.building.selected;
            painter.rect_stroke(r, 0.0, Stroke::new(if on { 1.5 } else { 1.0 }, Color32::from_white_alpha(if on { 220 } else { 90 })), StrokeKind::Inside);
        }
    }
    let Some(key) = tools.building.selected else { return };
    let Some((_, p)) = find(doc, key) else { return };
    let r = rect_of(p, 0, 0);
    painter.rect_filled(r, 0.0, pal.accent.gamma_multiply(0.12));
    painter.rect_stroke(r, 0.0, Stroke::new(1.5, Color32::from_black_alpha(120)), StrokeKind::Outside);
    painter.rect_stroke(r, 0.0, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
    // Handles at its corners, as a selected thing has.
    for c in [r.left_top(), r.right_top(), r.right_bottom(), r.left_bottom()] {
        let h = Rect::from_center_size(c, vec2(7.0, 7.0));
        painter.rect_filled(h, theme::TILE_RADIUS / 4, pal.accent);
        painter.rect_stroke(h, theme::TILE_RADIUS / 4, Stroke::new(1.0, Color32::from_black_alpha(160)), StrokeKind::Outside);
    }
    let name = crate::structure_tool::type_name(&p.template, 1);
    label(painter, pos2(r.left(), r.bottom() + 4.0), &capitalised(&name));
    if let Some((from, to)) = *tools.building.drag.lock().expect("drag") {
        let (dx, dz) = ((to.0 - from.0).round() as i32, (to.1 - from.1).round() as i32);
        if dx != 0 || dz != 0 {
            let moved = rect_of(p, dx, dz);
            painter.rect_filled(moved, 0.0, pal.accent.gamma_multiply(0.18));
            painter.rect_stroke(moved, 0.0, Stroke::new(1.5, pal.accent), StrokeKind::Inside);
        }
    }
}

/// The colour of a block seen from above, for a building's picture.
fn tint(id: &str) -> Color32 {
    let rgb = Color32::from_rgb;
    let id = id.split('[').next().unwrap_or(id);
    let has = |w: &str| id.contains(w);
    if has("glass") {
        rgb(170, 200, 214)
    } else if has("leaves") || has("grass") || has("moss") || has("vine") {
        rgb(84, 126, 52)
    } else if has("hay") {
        rgb(196, 164, 52)
    } else if has("water") {
        rgb(64, 96, 190)
    } else if has("lantern") || has("torch") || has("glowstone") {
        rgb(236, 190, 90)
    } else if has("brick") && !has("stone") {
        rgb(150, 80, 64)
    } else if has("terracotta") {
        rgb(160, 96, 70)
    } else if has("wool") || has("white") || has("calcite") || has("quartz") {
        rgb(222, 220, 212)
    } else if has("dark_oak") {
        rgb(72, 50, 30)
    } else if has("spruce") {
        rgb(104, 76, 46)
    } else if has("birch") {
        rgb(196, 178, 124)
    } else if has("acacia") {
        rgb(170, 92, 50)
    } else if has("jungle") {
        rgb(160, 114, 78)
    } else if has("mangrove") || has("crimson") {
        rgb(120, 52, 48)
    } else if has("cherry") {
        rgb(222, 170, 164)
    } else if has("oak") || has("planks") || has("log") || has("wood") || has("barrel") || has("chest") || has("crafting") || has("ladder") || has("door") || has("fence") {
        rgb(162, 128, 78)
    } else if has("sandstone") || has("sand") {
        rgb(216, 202, 150)
    } else if has("deepslate") || has("blackstone") || has("basalt") {
        rgb(72, 72, 78)
    } else if has("mossy") {
        rgb(106, 122, 92)
    } else if has("stone") || has("cobble") || has("andesite") || has("tuff") || has("gravel") {
        rgb(128, 128, 128)
    } else if has("dirt") || has("path") || has("mud") || has("farmland") {
        rgb(122, 88, 56)
    } else {
        rgb(140, 140, 140)
    }
}

/// A picture of the schematic at `path` seen from above (its highest block
/// in each column), made once and kept: none if it cannot be read.
fn thumbnail(ctx: &egui::Context, path: &std::path::Path) -> Option<egui::TextureHandle> {
    let id = egui::Id::unique(("building thumbnail", path));
    if let Some(t) = ctx.data(|d| d.get_temp::<Option<egui::TextureHandle>>(id)) {
        return t;
    }
    let made = massif_structures::village::pack::plan_of(path).ok().map(|plan| {
        let (w, d) = (plan.size[0].clamp(1, 256) as usize, plan.size[2].clamp(1, 256) as usize);
        let mut top: Vec<(i32, Color32)> = vec![(i32::MIN, Color32::TRANSPARENT); w * d];
        for (p, state, _) in &plan.blocks {
            if state.ends_with(":air") || state.contains(":air[") || state.contains("structure_void") {
                continue;
            }
            let (x, z) = (p[0] as usize, p[2] as usize);
            if x < w && z < d && p[1] > top[z * w + x].0 {
                top[z * w + x] = (p[1], tint(state));
            }
        }
        // Lighter as it rises: the roof stands out from the walls.
        let lo = top.iter().filter(|t| t.0 > i32::MIN).map(|t| t.0).min().unwrap_or(0);
        let hi = top.iter().map(|t| t.0).max().unwrap_or(0).max(lo + 1);
        let pixels: Vec<Color32> = top.iter().map(|&(y, c)| if y == i32::MIN { Color32::TRANSPARENT } else { theme::mix(c.gamma_multiply(0.7), c, (y - lo) as f32 / (hi - lo) as f32) }).collect();
        ctx.load_texture(format!("building {}", path.display()), egui::ColorImage { size: [w, d], source_size: vec2(w as f32, d as f32), pixels }, egui::TextureOptions::NEAREST)
    });
    ctx.data_mut(|data| data.insert_temp(id, made.clone()));
    made
}

/// The picture of a building, fitted in `rect`.
fn picture(ui: &Ui, rect: Rect, path: &std::path::Path) {
    let pal = Palette::of(ui);
    ui.painter().rect_filled(rect, theme::TILE_RADIUS / 2, theme::mix(pal.panel, pal.text, 0.06));
    if let Some(t) = thumbnail(ui.ctx(), path) {
        let [w, h] = t.size().map(|v| v as f32);
        let s = (rect.width() / w).min(rect.height() / h);
        let r = Rect::from_center_size(rect.center(), vec2(w * s, h * s));
        ui.painter().image(t.id(), r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
}

/// The settings of the tool: the building selected, what can be done to it,
/// and the pack's buildings to put in its place.
pub fn settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use crate::panels::{hint, section};
    use crate::widgets::button;
    let Some(key) = tools.building.selected.filter(|k| find(doc, *k).is_some()) else {
        tools.building.selected = None;
        section(ui, Icon::SquareMousePointer, tr!("Building"));
        hint(ui, tr!("Click a building of a village placed from a pack to select it. Drag it to move it; R turns it; Delete takes it away."));
        let edited = doc.dimension().structures.iter().filter_map(|s| recipe_of(s)).map(|r| if let Recipe::Village { params, .. } = r { params.edits.len() } else { 0 }).sum::<usize>();
        if edited > 0 {
            hint(ui, &tr!("{n} buildings changed by hand on this map.", n = edited));
        }
        return;
    };
    let Some((s, p)) = find(doc, key) else { return };
    let (s, p) = (s.clone(), p.clone());
    let Some(recipe @ Recipe::Village { .. }) = recipe_of(&s) else { return };
    let Recipe::Village { pack, theme: theme_name, .. } = &recipe else { return };
    let theme = tools.village.theme_named(pack, theme_name);
    let edit = edit_of(doc, key).unwrap_or_else(|| BuildingEdit::new(key));
    section(ui, Icon::SquareMousePointer, tr!("Selected"));
    let kind = capitalised(&crate::structure_tool::type_name(&p.template, 1));
    // Its file, as the village has it now.
    let (bw, bd, bh) = (p.max[0] - p.min[0] + 1, p.max[2] - p.min[2] + 1, p.max[1] - p.min[1] + 1);
    let fits = |v: &&massif_structures::village::pack::Variant| v.size[1] == bh && if p.turn % 2 == 0 { v.size[0] == bw && v.size[2] == bd } else { v.size[2] == bw && v.size[0] == bd };
    let file = theme.as_ref().and_then(|t| {
        let ty = t.kind(&p.template)?;
        let v = match &edit.swap {
            Some((_, name)) => ty.variants.iter().find(|v| file_name(v) == *name),
            None => ty.variants.iter().find(fits),
        }?;
        Some((v.file.clone(), file_name(v)))
    });
    ui.horizontal(|ui| {
        if let Some((path, _)) = &file {
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), egui::Sense::hover());
            picture(ui, r, path);
        }
        ui.vertical(|ui| {
            ui.label(egui::RichText::new(&kind).strong());
            let mut line = format!("{} × {} × {}", p.max[0] - p.min[0] + 1, p.max[2] - p.min[2] + 1, p.max[1] - p.min[1] + 1);
            if let Some((_, name)) = &file {
                line = format!("{name} · {line}");
            }
            ui.label(egui::RichText::new(line).small().weak());
        });
    });
    let mut note: Feedback = None;
    ui.horizontal_wrapped(|ui| {
        if button(ui, theme::with_icon(Icon::RotateCw, tr!("Turn"))).on_hover_text(tr!("A quarter turn about its middle (R)")).clicked() {
            note = turn(doc, tools, key);
        }
        if button(ui, theme::with_icon(Icon::Dices, tr!("Draw it again"))).on_hover_text(tr!("Another building of its type, in its place")).clicked() {
            note = change(doc, tools, key, tr!("Drawing it again…"), |e| {
                e.swap = None;
                e.draw += 1;
            });
        }
        if button(ui, theme::with_icon(Icon::Trash2, tr!("Take it away"))).clicked() {
            note = remove(doc, tools, key);
        }
        if !edit.is_empty() && button(ui, theme::with_icon(Icon::Undo2, tr!("As drawn"))).on_hover_text(tr!("Back as the village's draw placed it")).clicked() {
            note = change(doc, tools, key, tr!("Putting it back…"), |e| *e = BuildingEdit::new(key));
        }
    });
    // The pack's buildings to put in its place, type by type.
    if let Some(theme) = &theme {
        ui.label(egui::RichText::new(tr!("Another building")).small().weak());
        let current = edit.swap.clone().or_else(|| file.as_ref().map(|(_, n)| (p.template.clone(), n.clone())));
        let shown = current.as_ref().map_or_else(|| tr!("Pick one").to_owned(), |(_, n)| n.clone());
        crate::widgets::combo("building swap").selected_text(shown).width((ui.available_width() * 0.9).min(240.0)).height(460.0).show_ui(ui, |ui| {
            ui.set_min_width(220.0);
            for t in theme.types.iter().filter(|t| !t.variants.is_empty()) {
                ui.label(egui::RichText::new(capitalised(&crate::structure_tool::type_name(&t.name, 2))).small().weak());
                for v in &t.variants {
                    let name = file_name(v);
                    let on = current.as_ref().is_some_and(|(k, n)| *k == t.name && *n == name);
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 40.0), egui::Sense::click());
                    let pal = Palette::of(ui);
                    if on || response.hovered() {
                        ui.painter().rect_filled(rect, theme::TILE_RADIUS, if on { pal.accent_soft } else { pal.control() });
                    }
                    picture(ui, Rect::from_min_size(rect.min + vec2(4.0, 4.0), vec2(32.0, 32.0)), &v.file);
                    ui.painter().text(rect.left_center() + vec2(44.0, -7.0), egui::Align2::LEFT_CENTER, &name, egui::FontId::proportional(12.5), pal.text);
                    ui.painter().text(rect.left_center() + vec2(44.0, 8.0), egui::Align2::LEFT_CENTER, format!("{} × {} × {}", v.size[0], v.size[2], v.size[1]), egui::FontId::proportional(10.5), pal.weak);
                    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, on, &name));
                    if response.clicked() && !on {
                        let (kind, name) = (t.name.clone(), name.clone());
                        note = change(doc, tools, key, tr!("Putting another in its place…"), |e| {
                            e.swap = Some((kind, name));
                            e.draw = 0;
                        });
                    }
                }
            }
        });
    } else {
        hint(ui, tr!("Its pack is no longer added: add it again to swap this building for another."));
    }
    hint(ui, tr!("Drag it to move it: a path leads from its door to the nearest road. R turns it, Delete takes it away. Its village keeps each change when it is built again with the same draw."));
    if let Some(n) = note {
        tools.structure.rt.say(n);
    }
}
