//! The objects of an object layer, as WorldPainter's object layer dialog
//! lists them: a compact list where a click picks one to edit, and
//! Shift+Click, Shift+Arrow or Cmd/Ctrl+Click pick several to edit at
//! once; under it, their settings — how often each stands, where its
//! origin is, and its own rules (WorldPainter's object attributes: turning,
//! mirroring, where it may stand, what it gives way to, its leaves, its
//! height).

use crate::theme::{self, Icon, Palette};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::objects::{Collision, Leaves, Mirroring, ObjectDef, ObjectRules, Turning, Underground};
use massif_core::registry::Registry;

/// The rows of the list picked, and the one a Shift+Click or Shift+Arrow
/// extends from.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Picked {
    pub rows: Vec<usize>,
    pub anchor: usize,
    /// The row the arrows move from: the last clicked or reached.
    pub cursor: usize,
}

impl Picked {
    /// A click on `row`: alone, added or taken out (`toggle`, Cmd/Ctrl),
    /// or the rows from the anchor to it (`range`, Shift).
    pub fn click(&mut self, row: usize, toggle: bool, range: bool) {
        if range {
            let (a, b) = (self.anchor.min(row), self.anchor.max(row));
            self.rows = (a..=b).collect();
        } else if toggle {
            match self.rows.iter().position(|&r| r == row) {
                Some(i) => {
                    self.rows.remove(i);
                }
                None => self.rows.push(row),
            }
            self.anchor = row;
        } else {
            self.rows = vec![row];
            self.anchor = row;
        }
        self.cursor = row;
        self.rows.sort_unstable();
    }

    /// An arrow key: the next row up or down alone, or (`extend`, Shift)
    /// the rows from the anchor to it, in a list of `count`.
    pub fn step(&mut self, up: bool, extend: bool, count: usize) {
        if count == 0 {
            return;
        }
        let next = if up { self.cursor.saturating_sub(1) } else { (self.cursor + 1).min(count - 1) };
        self.cursor = next;
        if extend {
            let (a, b) = (self.anchor.min(next), self.anchor.max(next));
            self.rows = (a..=b).collect();
        } else {
            self.rows = vec![next];
            self.anchor = next;
        }
    }

    /// A click on `row` in a list shown in `order` (rows of the layer, as
    /// sorted and filtered): a range runs along the order shown.
    pub fn click_in(&mut self, order: &[usize], row: usize, toggle: bool, range: bool) {
        let at = |r: usize| order.iter().position(|&o| o == r);
        match (range, at(self.anchor), at(row)) {
            (true, Some(a), Some(b)) => {
                self.rows = order[a.min(b)..=a.max(b)].to_vec();
                self.rows.sort_unstable();
                self.cursor = row;
            }
            _ => self.click(row, toggle, range && !order.contains(&self.anchor)),
        }
    }

    /// An arrow key in a list shown in `order`.
    pub fn step_in(&mut self, order: &[usize], up: bool, extend: bool) {
        if order.is_empty() {
            return;
        }
        let here = order.iter().position(|&o| o == self.cursor).unwrap_or(0);
        let next = order[if up { here.saturating_sub(1) } else { (here + 1).min(order.len() - 1) }];
        if extend {
            self.click_in(order, next, false, true);
        } else {
            self.rows = vec![next];
            self.anchor = next;
        }
        self.cursor = next;
    }

    /// Kept to the rows there are.
    fn clamp(&mut self, count: usize) {
        self.rows.retain(|&r| r < count);
        if self.rows.is_empty() && count > 0 {
            self.rows.push(self.anchor.min(count - 1));
        }
        self.anchor = self.anchor.min(count.saturating_sub(1));
        self.cursor = self.cursor.min(count.saturating_sub(1));
    }
}

const ROW_H: f32 = 26.0;

/// The picked rows of layer `k`'s list, kept between frames.
fn picked_id(k: usize) -> egui::Id {
    egui::Id::unique(("object rows picked", k))
}

/// The objects picked in the list of layer `k`, from another panel.
pub fn picked(ctx: &egui::Context, k: usize) -> Picked {
    ctx.data(|d| d.get_temp::<Picked>(picked_id(k))).unwrap_or_default()
}

pub fn set_picked(ctx: &egui::Context, k: usize, picked: Picked) {
    ctx.data_mut(|d| d.insert_temp(picked_id(k), picked));
}

/// One change asked of the objects picked: to each, with its weight.
type Edit = Box<dyn Fn(&mut ObjectDef, &mut u16)>;

/// The list of object layer `k`'s objects and the settings of those
/// picked; returns whether anything changed.
pub fn object_list(ui: &mut Ui, reg: &mut Registry, k: usize) -> bool {
    let pal = Palette::of(ui);
    let count = reg.object_layers[k].objects.len();
    let mut picked = picked(ui.ctx(), k);
    picked.clamp(count);
    if count == 0 {
        set_picked(ui.ctx(), k, picked);
        return false;
    }
    let total: f32 = reg.object_layers[k].objects.iter().map(|o| o.1 as f32).sum::<f32>().max(1.0);

    // The list: a row each, at most ten high, then it scrolls.
    let list_id = egui::Id::unique(("object list", k));
    let focused = ui.memory(|m| m.has_focus(list_id));
    let frame = egui::Frame::new().fill(pal.well()).stroke(Stroke::new(1.0, if focused { pal.accent } else { pal.border })).corner_radius(theme::GROUP_RADIUS).inner_margin(4);
    let mut clicked = None;
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        egui::ScrollArea::vertical().id_salt(("object list scroll", k)).max_height(ROW_H * 10.0 + 4.0).auto_shrink([false, true]).show_rows(ui, ROW_H, count, |ui, range| {
            ui.spacing_mut().item_spacing.y = 0.0;
            for row in range {
                let (index, weight) = reg.object_layers[k].objects[row];
                let Some(def) = reg.objects.get(index as usize) else { continue };
                let on = picked.rows.contains(&row);
                let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
                response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, &def.name));
                let fill = if on {
                    pal.accent_soft
                } else if response.hovered() {
                    pal.header
                } else {
                    Color32::TRANSPARENT
                };
                let painter = ui.painter();
                painter.rect(rect, theme::TILE_RADIUS, fill, if on { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
                let texture = crate::panels::object_texture(ui, index as usize, def);
                let size = texture.size_vec2();
                let r = Rect::from_center_size(rect.left_center() + vec2(14.0, 0.0), vec2(22.0, 22.0));
                let fit = (r.width() / size.x).min(r.height() / size.y);
                painter.image(texture.id(), Rect::from_center_size(r.center(), size * fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                let share = painter.layout_no_wrap(format!("{:.0}%", weight as f32 * 100.0 / total), egui::FontId::proportional(11.0), pal.weak);
                let mut right = share.size().x + 8.0;
                painter.galley(rect.right_center() - vec2(right, share.size().y / 2.0), share, pal.weak);
                // Its own rules, marked.
                if def.rules.is_some() {
                    right += 16.0;
                    painter.text(rect.right_center() - vec2(right - 6.0, 0.0), egui::Align2::CENTER_CENTER, theme::glyph(Icon::SlidersHorizontal), egui::FontId::proportional(11.0), pal.weak);
                }
                let room = rect.width() - 30.0 - right - 8.0;
                let mut job = egui::text::LayoutJob::simple_singleline(def.name.clone(), egui::FontId::proportional(12.5), pal.text);
                job.wrap = egui::text::TextWrapping::truncate_at_width(room.max(10.0));
                let galley = painter.fonts_mut(|f| f.layout_job(job));
                painter.galley(pos2(rect.left() + 30.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
                let response = response.on_hover_text(format!("{} · {} × {} × {}\n{}", def.name, def.size[0], def.size[1], def.size[2], def.source));
                if response.clicked() {
                    clicked = Some(row);
                }
            }
        });
    });
    if let Some(row) = clicked {
        let (toggle, range) = ui.input(|i| (i.modifiers.command, i.modifiers.shift));
        picked.click(row, toggle, range);
        ui.memory_mut(|m| m.request_focus(list_id));
    }
    // Arrows move through the list while it has the keys.
    if ui.memory(|m| m.has_focus(list_id)) {
        let keys = ui.input(|i| (i.key_pressed(egui::Key::ArrowUp), i.key_pressed(egui::Key::ArrowDown), i.modifiers.shift, i.modifiers.command && i.key_pressed(egui::Key::A), i.key_pressed(egui::Key::Escape)));
        if keys.0 || keys.1 {
            picked.step(keys.0, keys.2, count);
        }
        if keys.3 {
            picked.rows = (0..count).collect();
        }
        if keys.4 {
            ui.memory_mut(|m| m.surrender_focus(list_id));
        }
        ui.memory_mut(|m| m.set_focus_lock_filter(list_id, egui::EventFilter { vertical_arrows: true, ..Default::default() }));
    }
    ui.add(egui::Label::new(RichText::new(tr!("Click one to edit it; Shift+Click, Shift+Arrows or {cmd}+Click to edit several at once.", cmd = if cfg!(target_os = "macos") { "⌘" } else { "Ctrl" })).size(11.0).color(pal.weak)));
    set_picked(ui.ctx(), k, picked.clone());

    // The settings of those picked.
    let rows = picked.rows.clone();
    let changed = editor(ui, reg, k, &rows);
    // Taken out of the layer.
    let remove = crate::widgets::button(ui, theme::with_icon(Icon::X, &trn!("Take {n} out of the layer", "Take {n} out of the layer", n = rows.len())))
        .on_hover_text(tr!("Its objects stay in the map's library; they no longer stand with this layer"))
        .clicked();
    let export = crate::widgets::button(ui, theme::with_icon(Icon::FileBox, &trn!("Export {n} as .schem…", "Export {n} as .schem…", n = rows.len())))
        .on_hover_text(tr!("Each as a Sponge schematic of its own, its anchor the origin: to paste with WorldEdit or bring into another map"))
        .clicked();
    let said = egui::Id::unique(("object schematics said", k));
    if export {
        let defs: Vec<&ObjectDef> = rows.iter().filter_map(|&r| reg.object_layers[k].objects.get(r)).filter_map(|o| reg.objects.get(o.0 as usize)).collect();
        if let Some(text) = export_schematics(&defs) {
            ui.ctx().data_mut(|d| d.insert_temp(said, text));
        }
    }
    if let Some(text) = ui.ctx().data(|d| d.get_temp::<String>(said)) {
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(pal.weak)).wrap());
    }
    if remove {
        let layer = &mut reg.object_layers[k];
        for &row in rows.iter().rev() {
            if row < layer.objects.len() {
                layer.objects.remove(row);
            }
        }
        set_picked(ui.ctx(), k, Picked::default());
        return true;
    }
    changed
}

/// The data version objects are written with: the newest release of 1.21,
/// which WorldEdit and Litematica of every later version read.
const OBJECT_DATA_VERSION: i32 = 4671;

/// The file names of objects, each its own, in a folder: `Oak.schem`,
/// `Oak 2.schem`…
fn file_names(defs: &[&ObjectDef]) -> Vec<String> {
    let mut taken = std::collections::HashSet::new();
    defs.iter()
        .map(|d| {
            let base = massif_core::mask::file_safe(if d.name.trim().is_empty() { "Object" } else { d.name.trim() });
            let mut name = format!("{base}.schem");
            let mut n = 2;
            while !taken.insert(name.to_lowercase()) {
                name = format!("{base} {n}.schem");
                n += 1;
            }
            name
        })
        .collect()
}

/// Write `defs` as schematics: one to a file asked for, several to a
/// folder asked for. What was done, to say; `None` when nothing was asked.
pub fn export_schematics(defs: &[&ObjectDef]) -> Option<String> {
    let names = file_names(defs);
    let paths: Vec<std::path::PathBuf> = match defs {
        [] => return None,
        [_] => vec![rfd::FileDialog::new().set_title(tr!("Save the object as a schematic")).set_file_name(&names[0]).add_filter("Sponge schematic", &["schem"]).save_file()?],
        _ => {
            let dir = rfd::FileDialog::new().set_title(tr!("A folder to write the objects in")).pick_folder()?;
            names.iter().map(|n| dir.join(n)).collect()
        }
    };
    let mut written = 0;
    for (def, path) in defs.iter().zip(&paths) {
        let volume = massif_objects::write::Volume::of_object(def, OBJECT_DATA_VERSION);
        let bytes = match massif_objects::write::write(&volume, massif_objects::write::Format::Sponge3) {
            Ok(b) => b,
            Err(e) => return Some(tr!("{name} could not be written: {e}", name = def.name, e)),
        };
        if let Err(e) = massif_core::atomic::write(path, &bytes, true) {
            return Some(tr!("{name} could not be written. {why}.", name = def.name, why = massif_core::atomic::cause(&e, path)));
        }
        written += 1;
    }
    let place = paths[0].parent().map(|p| p.display().to_string()).unwrap_or_default();
    Some(trn!("{n} schematic written in {place}", "{n} schematics written in {place}", n = written, place))
}

/// The settings of rows `rows` of layer `k`: of one, or of several at
/// once, what they share shown and a change made to all of them.
pub fn editor(ui: &mut Ui, reg: &mut Registry, k: usize, rows: &[usize]) -> bool {
    let pal = Palette::of(ui);
    let layer = &reg.object_layers[k];
    let picked: Vec<(&ObjectDef, u16)> = rows.iter().filter_map(|&r| layer.objects.get(r)).filter_map(|&(i, w)| reg.objects.get(i as usize).map(|d| (d, w))).collect();
    let Some(&(first, first_weight)) = picked.first() else { return false };
    let several = picked.len() > 1;
    let mixed = |f: &dyn Fn(&ObjectDef, u16) -> String| picked.iter().any(|(d, w)| f(d, *w) != f(first, first_weight));
    let name_of = |label: &str, differ: bool| if differ { tr!("{label} · mixed", label) } else { label.to_owned() };
    let mut edits: Vec<Edit> = Vec::new();
    let mut new_size = false;

    ui.add_space(6.0);
    if several {
        ui.label(RichText::new(trn!("{n} object", "{n} objects", n = picked.len())).strong());
    } else {
        ui.horizontal(|ui| {
            let index = layer.objects[rows[0]].0 as usize;
            let texture = crate::panels::object_texture(ui, index, first);
            let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
            let size = texture.size_vec2();
            let fit = (r.width() / size.x).min(r.height() / size.y);
            ui.painter().image(texture.id(), Rect::from_center_size(r.center(), size * fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                let mut name = first.name.clone();
                if ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Name")).desired_width(ui.available_width())).changed() {
                    edits.push(Box::new(move |d, _| d.name = name.clone()));
                }
                ui.label(RichText::new(tr!("{value} × {value2} × {value3} · {n} blocks", value = first.size[0], value2 = first.size[1], value3 = first.size[2], n = first.count())).size(11.0).color(pal.weak));
                ui.add(egui::Label::new(RichText::new(&first.source).size(11.0).color(pal.weak)).truncate());
            });
        });
    }
    ui.add_space(4.0);
    crate::widgets::well(ui, |ui| {
        // How often it stands, against the others.
        let mut weight = first_weight;
        crate::widgets::row(ui, &name_of(tr!("Frequency"), mixed(&|_, w| w.to_string())), |ui| {
            if ui.add(crate::widgets::Field::new(&mut weight, 0..=1000).logarithmic(true).width(120.0).max_decimals(0)).on_hover_text(tr!("How often it stands against the layer's other objects. 0: only where Tree sizes ask for its size")).changed() {
                edits.push(Box::new(move |_, w| *w = weight));
            }
        });
        // Its tree size, by hand or guessed from its height.
        let guess = layer.size_classes(reg).get(rows[0]).copied().unwrap_or(1);
        crate::widgets::row(ui, &name_of(tr!("Tree size"), mixed(&|d, _| d.tree_size.to_string())), |ui| {
            match crate::tree_sizes::object_size(ui, reg, ("object tree size", k), first.tree_size, guess) {
                Some(crate::tree_sizes::NEW_SIZE) => new_size = true,
                Some(size) => edits.push(Box::new(move |d, _| d.tree_size = size)),
                None => {}
            }
        });
        // Its canopy's radius: measured, or set by hand.
        let measured = first.measured_canopy();
        let mut canopy = if first.canopy > 0.0 { first.canopy } else { 0.0 };
        crate::widgets::row(ui, &name_of(tr!("Canopy"), mixed(&|d, _| d.canopy.to_bits().to_string())), |ui| {
            let field = crate::widgets::Field::new(&mut canopy, 0.0..=32.0)
                .width(120.0)
                .custom_formatter(move |v, _| if v <= 0.0 { tr!("Auto ({r:.1})", r = measured) } else { tr!("{v:.1} blocks", v) })
                .custom_parser(|s| s.trim().trim_end_matches(|c: char| !c.is_ascii_digit() && c != '.').trim().parse::<f64>().ok().or(Some(0.0)));
            if ui.add(field).on_hover_text(tr!("The radius of its crown: two trees spaced by canopy stand as far apart as their radii added. Auto: measured from its leaves")).changed() {
                edits.push(Box::new(move |d, _| d.canopy = canopy.max(0.0)));
            }
        });
        // Its origin: the block of it set on the spot over the ground.
        if !several {
            let mut anchor = first.anchor.map(|a| a as i32);
            let size = first.size;
            crate::widgets::row(ui, tr!("Origin"), |ui| {
                if ui.add(egui::Button::new(tr!("Auto")).small()).on_hover_text(tr!("The foot of its trunk, or the middle of its base, found from its blocks")).clicked() {
                    edits.push(Box::new(|d, _| d.anchor = d.find_anchor()));
                }
                let mut moved = false;
                for (axis, name) in [(2usize, "Z"), (1, "Y"), (0, "X")] {
                    moved |= ui.add(crate::widgets::Field::new(&mut anchor[axis], 0..=(size[axis] as i32 - 1)).width(46.0).max_decimals(0)).on_hover_text(name).changed();
                    ui.label(RichText::new(name).size(11.0).color(pal.weak));
                }
                if moved {
                    edits.push(Box::new(move |d, _| d.anchor = std::array::from_fn(|a| anchor[a].clamp(0, d.size[a] as i32 - 1) as u16)));
                }
            });
        }
        // Its own rules, or its layer's.
        let own = first.rules.is_some();
        let mut on = own;
        crate::widgets::row(ui, &name_of(tr!("Own rules"), mixed(&|d, _| d.rules.is_some().to_string())), |ui| {
            if crate::widgets::switch(ui, &mut on).on_hover_text(tr!("WorldPainter's object attributes: where it may stand, how it is turned, what it gives way to. Off: as its layer says")).changed() {
                let of_layer = ObjectRules::of_layer(&reg.object_layers[k]);
                edits.push(Box::new(move |d, _| d.rules = on.then(|| d.rules.clone().unwrap_or_else(|| Box::new(of_layer.clone())))));
            }
        });
    });
    let rules = first.rules.as_deref().cloned();
    if let Some(r) = rules {
        let differs = |f: &dyn Fn(&ObjectRules) -> String| mixed(&|d, _| d.rules.as_deref().map_or(String::new(), f));
        // A change to one field of the rules, made to every object picked
        // that has its own.
        fn set(edits: &mut Vec<Edit>, f: impl Fn(&mut ObjectRules) + 'static) {
            edits.push(Box::new(move |d, _| {
                if let Some(r) = d.rules.as_deref_mut() {
                    f(r);
                }
            }));
        }
        ui.add_space(6.0);
        ui.label(RichText::new(tr!("How it stands")).size(11.5).strong().color(pal.weak));
        crate::widgets::well(ui, |ui| {
            let turn = match r.turning {
                Turning::Random => 0,
                Turning::Fixed(t) => 1 + t as usize,
            };
            crate::widgets::row(ui, &name_of(tr!("Turned"), differs(&|r| format!("{:?}", r.turning))), |ui| {
                let labels = [tr!("At random"), tr!("Not turned"), "90°", "180°", "270°"];
                let mut pick = turn;
                crate::widgets::combo(("object turning", k))
                    .selected_text(labels[turn])
                    .show_ui(ui, |ui| {
                        for (i, l) in labels.iter().enumerate() {
                            ui.selectable_value(&mut pick, i, *l);
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr!("Turned")));
                if pick != turn {
                    set(&mut edits, move |r| r.turning = if pick == 0 { Turning::Random } else { Turning::Fixed(pick as u8 - 1) });
                }
            });
            crate::widgets::row(ui, &name_of(tr!("Mirrored"), differs(&|r| format!("{:?}", r.mirroring))), |ui| {
                let all = [Mirroring::Random, Mirroring::Never, Mirroring::Always];
                if let Some(i) = crate::widgets::button_group(ui, "object mirroring", &[tr!("At random"), tr!("Never"), tr!("Always")], &all.map(|m| m == r.mirroring)) {
                    let m = all[i];
                    set(&mut edits, move |r| r.mirroring = m);
                }
            });
            crate::widgets::row(ui, &name_of(tr!("Height"), differs(&|r| r.fixed_height.to_string())), |ui| {
                if let Some(i) = crate::widgets::button_group(ui, "object height", &[tr!("Over the ground"), tr!("Fixed")], &[!r.fixed_height, r.fixed_height]) {
                    set(&mut edits, move |r| r.fixed_height = i == 1);
                }
            });
            let mut offset = r.offset;
            crate::widgets::row(ui, &name_of(if r.fixed_height { tr!("At height") } else { tr!("Raised by") }, differs(&|r| r.offset.to_string())), |ui| {
                if ui.add(crate::widgets::Field::new(&mut offset, -64..=320).width(120.0).suffix(tr!(" blocks")).max_decimals(0)).on_hover_text(tr!("Blocks over the ground its origin stands (under it when below 0); at a fixed height, that height")).changed() {
                    set(&mut edits, move |r| r.offset = offset);
                }
            });
            let mut variation = r.variation;
            crate::widgets::row(ui, &name_of(tr!("Varied by"), differs(&|r| r.variation.to_string())), |ui| {
                if ui.add(crate::widgets::Field::new(&mut variation, 0..=64).width(120.0).suffix(tr!(" blocks")).max_decimals(0)).on_hover_text(tr!("Up to this many blocks higher or lower, at random, about as often up as down")).changed() {
                    set(&mut edits, move |r| r.variation = variation);
                }
            });
        });
        ui.add_space(6.0);
        ui.label(RichText::new(tr!("Where it may stand")).size(11.5).strong().color(pal.weak));
        crate::widgets::well(ui, |ui| {
            // Each switch and the field of the rules it sets.
            fn flag(r: &mut ObjectRules, i: usize) -> &mut bool {
                match i {
                    0 => &mut r.on_land,
                    1 => &mut r.on_air,
                    2 => &mut r.under_water,
                    3 => &mut r.on_water,
                    4 => &mut r.collide_with_floor,
                    5 => &mut r.under_lava,
                    6 => &mut r.on_lava,
                    _ => &mut r.extend_foundation,
                }
            }
            let flags: [(&str, &str); 8] = [
                (tr!("On land"), tr!("On dry ground")),
                (tr!("On air"), tr!("With nothing solid under it: at a fixed height over the ground, it floats rather than being left out")),
                (tr!("Under water"), tr!("On the bottom of water")),
                (tr!("On water"), tr!("Floating on the top of water")),
                (tr!("Collides with the floor"), tr!("Standing on water, its blocks under it stop at the floor rather than going through it")),
                (tr!("Under lava"), tr!("On the bottom of lava, in a map of lava")),
                (tr!("On lava"), tr!("Floating on the top of lava")),
                (tr!("Extends its foundation"), tr!("Its lowest blocks go on down to the ground where it would float")),
            ];
            for (i, (label, tip)) in flags.into_iter().enumerate() {
                let mut value = *flag(&mut r.clone(), i);
                crate::widgets::row(ui, &name_of(label, differs(&|r| flag(&mut r.clone(), i).to_string())), |ui| {
                    if crate::widgets::switch(ui, &mut value).on_hover_text(tip).changed() {
                        set(&mut edits, move |r| *flag(r, i) = value);
                    }
                });
            }
        });
        ui.add_space(6.0);
        ui.label(RichText::new(tr!("What it gives way to")).size(11.5).strong().color(pal.weak));
        crate::widgets::well(ui, |ui| {
            crate::widgets::row(ui, &name_of(tr!("Collides with"), differs(&|r| format!("{:?}", r.collision))), |ui| {
                let all = [Collision::All, Collision::Solid, Collision::Nothing];
                if let Some(i) = crate::widgets::button_group_named(ui, "object collision", &[tr!("All"), tr!("Solid"), tr!("None")], &[tr!("Over the ground, it only takes the place of air"), tr!("It takes the place of air, water and plants"), tr!("It takes the place of anything")], &all.map(|c| c == r.collision)) {
                    let c = all[i];
                    set(&mut edits, move |r| r.collision = c);
                }
            });
            crate::widgets::row(ui, &name_of(tr!("Replaces underground"), differs(&|r| format!("{:?}", r.underground))), |ui| {
                let all = [Underground::All, Underground::Solid, Underground::None];
                if let Some(i) = crate::widgets::button_group_named(ui, "object underground", &[tr!("All"), tr!("Solid"), tr!("None")], &[tr!("All its blocks in the ground replace the ground"), tr!("Its solid blocks replace the ground; the others only go where there is air"), tr!("Its blocks only go where there is air")], &all.map(|u| u == r.underground)) {
                    let u = all[i];
                    set(&mut edits, move |r| r.underground = u);
                }
            });
            crate::widgets::row(ui, &name_of(tr!("Leaves"), differs(&|r| format!("{:?}", r.leaves))), |ui| {
                let all = [Leaves::AsTheyAre, Leaves::Decay, Leaves::Persistent];
                let labels = [tr!("As they are"), tr!("Decay"), tr!("Never decay")];
                let now = all.iter().position(|l| *l == r.leaves).unwrap_or(0);
                let mut pick = now;
                crate::widgets::combo(("object leaves", k))
                    .selected_text(labels[now])
                    .show_ui(ui, |ui| {
                        for (i, l) in labels.iter().enumerate() {
                            ui.selectable_value(&mut pick, i, *l);
                        }
                    })
                    .response
                    .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, tr!("Leaves")));
                if pick != now {
                    let l = all[pick];
                    set(&mut edits, move |r| r.leaves = l);
                }
            });
            crate::widgets::row(ui, &name_of(tr!("Waterlogged"), differs(&|r| r.manage_waterlogged.to_string())), |ui| {
                if let Some(i) = crate::widgets::button_group_named(ui, "object waterlogging", &[tr!("By the water"), tr!("As they are")], &[tr!("Its blocks that can hold water do where they stand in water, and not out of it"), tr!("As the object has them")], &[r.manage_waterlogged, !r.manage_waterlogged]) {
                    set(&mut edits, move |r| r.manage_waterlogged = i == 0);
                }
            });
            let mut connect = r.connect_blocks;
            crate::widgets::row(ui, &name_of(tr!("Joins fences and panes"), differs(&|r| r.connect_blocks.to_string())), |ui| {
                if crate::widgets::switch(ui, &mut connect).on_hover_text(tr!("Its fences, panes, bars and walls join their neighbours in it")).changed() {
                    set(&mut edits, move |r| r.connect_blocks = connect);
                }
            });
            let mut air = r.replace_with_air.clone().unwrap_or_default();
            crate::widgets::row(ui, &name_of(tr!("Placed as air"), differs(&|r| r.replace_with_air.clone().unwrap_or_default())), |ui| {
                if ui.add(egui::TextEdit::singleline(&mut air).hint_text("minecraft:glass").desired_width(130.0)).on_hover_text(tr!("A block of the object placed as air rather than itself: to carve a hollow in the ground")).changed() {
                    let air = air.trim().to_owned();
                    set(&mut edits, move |r| r.replace_with_air = (!air.is_empty()).then(|| air.clone()));
                }
            });
        });
    } else if !several {
        crate::panels::hint(ui, tr!("It stands as its layer says. Own rules gives it WorldPainter's: where it may stand, its turn, what it gives way to."));
    }
    // New size…: a size of one's own added to the map, theirs.
    if new_size && let Some(n) = crate::tree_sizes::add(reg) {
        edits.push(Box::new(move |d, _| d.tree_size = n as u8));
    }
    if edits.is_empty() {
        return false;
    }
    apply(reg, k, rows, &edits);
    true
}

/// Make `edits` to rows `rows` of layer `k`. An object another layer also
/// stands is copied first, so that the change is this layer's alone.
pub fn apply(reg: &mut Registry, k: usize, rows: &[usize], edits: &[Edit]) {
    for &row in rows {
        let Some(&(index, _)) = reg.object_layers[k].objects.get(row) else { continue };
        let shared = reg.object_layers.iter().enumerate().any(|(j, l)| !l.removed && l.objects.iter().enumerate().any(|(r, o)| o.0 == index && (j != k || r != row)));
        let index = if shared {
            let copy = reg.objects[index as usize].clone();
            reg.objects.push(copy);
            let new = reg.objects.len() as u32 - 1;
            reg.object_layers[k].objects[row].0 = new;
            new
        } else {
            index
        };
        let def = &mut reg.objects[index as usize];
        let weight = &mut reg.object_layers[k].objects[row].1;
        for edit in edits {
            edit(def, weight);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn rows_are_picked_as_in_a_file_list() {
        let mut p = Picked::default();
        p.click(2, false, false);
        assert_eq!(p.rows, [2]);
        p.click(5, false, true);
        assert_eq!(p.rows, [2, 3, 4, 5], "Shift+Click: from the anchor");
        p.click(8, true, false);
        assert_eq!(p.rows, [2, 3, 4, 5, 8], "Cmd+Click adds");
        p.click(3, true, false);
        assert_eq!(p.rows, [2, 4, 5, 8], "and takes out");
        p.click(6, false, false);
        p.step(false, true, 10);
        p.step(false, true, 10);
        assert_eq!(p.rows, [6, 7, 8], "Shift+Down extends");
        p.step(true, true, 10);
        assert_eq!(p.rows, [6, 7]);
        p.step(true, false, 10);
        assert_eq!(p.rows, [6], "an arrow alone moves");
        p.step(false, false, 7);
        p.step(false, false, 7);
        assert_eq!(p.rows, [6], "kept in the list");
    }

    /// A map of one layer of six poles, and a second layer that shares the
    /// first pole.
    fn registry() -> Registry {
        let mut reg = Registry::vanilla();
        for h in 1..=6u16 {
            reg.objects.push(ObjectDef::new(&format!("Pole {h}"), [1, h, 1], vec!["minecraft:air".into(), "minecraft:oak_log[axis=y]".into()], vec![1; h as usize]));
        }
        let layer = massif_core::objects::ObjectLayerDef::new("Poles", &[0, 1, 2, 3, 4, 5], &reg);
        reg.add_object_layer(layer);
        let other = massif_core::objects::ObjectLayerDef::new("Other", &[0], &reg);
        reg.add_object_layer(other);
        reg
    }

    fn draw(ui: &mut Ui, reg: &mut Registry) {
        theme::install(ui.ctx());
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            egui::ScrollArea::vertical().show(ui, |ui| {
                object_list(ui, reg, 0);
            });
        });
    }

    /// Clicks pick rows, Shift and Cmd/Ctrl pick several, and a change in
    /// the editor goes to every one picked; a pole the other layer also
    /// stands is copied first, so that the other layer keeps it as it was.
    #[test]
    fn several_objects_are_edited_at_once() {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 1400.0)).build_ui_state(draw, registry());
        h.run_steps(2);
        h.get_by_label("Pole 1").click();
        h.run_steps(2);
        h.get_by_label("Pole 3").click_modifiers(egui::Modifiers::SHIFT);
        h.run_steps(2);
        h.get_by_label("Pole 6").click_modifiers(egui::Modifiers::COMMAND);
        h.run_steps(2);
        assert_eq!(picked(&h.ctx, 0).rows, [0, 1, 2, 5]);
        h.get_by_label("4 objects");
        // Own rules for all four, then the frequency and one rule.
        h.get_by_label("Own rules");
        h.query_all_by_role(egui::accesskit::Role::Switch).next().expect("the Own rules switch").click();
        h.run_steps(2);
        let reg = h.state_mut();
        apply(reg, 0, &[0, 1, 2, 5], &[Box::new(|_, w| *w = 7), Box::new(|d, _| d.rules.as_deref_mut().unwrap().offset = 2)]);
        let reg = h.state();
        let layer = &reg.object_layers[0];
        for row in [0, 1, 2, 5] {
            let def = &reg.objects[layer.objects[row].0 as usize];
            assert_eq!(layer.objects[row].1, 7);
            assert_eq!(def.rules.as_deref().map(|r| r.offset), Some(2), "{}", def.name);
        }
        for row in [3, 4] {
            assert!(reg.objects[layer.objects[row].0 as usize].rules.is_none() && layer.objects[row].1 == 1);
        }
        // Pole 1 was copied: the other layer's is untouched.
        assert_ne!(layer.objects[0].0, 0);
        assert!(reg.objects[0].rules.is_none() && reg.object_layers[1].objects[0] == (0, 1));
        h.run_steps(2);
        // Mixed values are said to be.
        h.get_by_label("Pole 4").click_modifiers(egui::Modifiers::COMMAND);
        h.run_steps(2);
        h.get_by_label_contains("Frequency · mixed");
        // Taken out of the layer.
        h.get_by_label_contains("Take 5 out of the layer").click();
        h.run_steps(2);
        assert_eq!(h.state().object_layers[0].objects.len(), 1);
    }

    /// The list with three of six picked, and their rules: offscreen in
    /// both themes. Needs a GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn object_list_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp: f32 = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
        for dark in [true, false] {
            let mut reg = registry();
            for k in [1usize, 2, 3] {
                reg.objects[k].rules = Some(Box::new(ObjectRules { offset: 1, leaves: Leaves::Persistent, ..ObjectRules::default() }));
            }
            reg.objects[3].rules.as_deref_mut().unwrap().offset = 2;
            reg.object_layers[0].objects[2].1 = 3;
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 1250.0))
                .with_pixels_per_point(ppp)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, reg: &mut Registry| {
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        draw(ui, reg);
                    },
                    reg,
                );
            h.run_steps(2);
            set_picked(&h.ctx, 0, Picked { rows: vec![1, 2, 3], anchor: 1, cursor: 3 });
            h.run_steps(3);
            let suffix = if ppp > 1.5 { "2x" } else { "1x" };
            h.render().unwrap().save(dir.join(format!("object-list-{}-{suffix}.png", if dark { "dark" } else { "light" }))).unwrap();
        }
    }
}
