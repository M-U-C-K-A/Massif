//! The side editors: tools, tool settings, brushes, terrain and biome
//! palettes, levels, world info, and the chooser shown by empty areas.

use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
use crate::layout::EditorKind;
use crate::map_render::ColumnSampler;
use crate::theme::{self, Icon, Palette};
use crate::tools::{AnchorMode, SurfaceChoice, Tool, ToolState};
use egui::{Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::compose::TileStack;
use massif_core::coords::local_index;
use massif_core::level::LevelKind;
use massif_core::registry::{BiomeId, Registry, TerrainId};
use massif_core::tile::NO_FLUID;
use massif_core::stamps::{Family, Stamp};
use massif_core::{MAX_RADIUS, TilePos};
use std::collections::HashSet;

fn section(ui: &mut Ui, icon: Icon, title: &str) {
    let pal = Palette::of(ui);
    ui.add_space(4.0);
    ui.label(
        RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase()))
            .size(10.5)
            .strong()
            .color(pal.weak),
    );
}

fn hint(ui: &mut Ui, text: &str) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(text).size(11.5).color(pal.weak));
}

/// A square icon button.
fn icon_button(ui: &mut Ui, icon: Icon, selected: bool, size: f32) -> egui::Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let visuals = ui.style().interact_selectable(&response, selected);
    let fill = if selected {
        pal.accent_soft
    } else if response.hovered() {
        visuals.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter();
    painter.rect(rect, 6, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        theme::glyph(icon),
        egui::FontId::proportional(size * 0.52),
        if selected { pal.accent } else { pal.text },
    );
    response
}

// ------------------------------------------------------------------ tools --

pub fn tools_panel(ui: &mut Ui, tools: &mut ToolState) {
    for (group, icon, list) in Tool::GROUPS {
        section(ui, icon, group);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for &tool in list {
                let key = tool.shortcut_text().map(|k| format!("  ({k})")).unwrap_or_default();
                let tip = format!("{}{key}\n{}", tool.label(), tool.help());
                if icon_button(ui, tool.icon(), tools.tool == tool, 32.0).on_hover_text(tip).clicked() {
                    tools.select(tool);
                }
            }
        });
    }
}

// --------------------------------------------------------- tool settings --

fn slider_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        let label = ui.add_sized([72.0, 18.0], egui::Label::new(label).truncate());
        add(ui).labelled_by(label.id).changed()
    })
    .inner
}

pub fn tool_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    let pal = Palette::of(ui);
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0 - 84.0).clamp(60.0, 220.0);
    let tool = tools.tool;
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(tool.icon()).to_string()).size(18.0).color(pal.accent));
        ui.label(RichText::new(tool.label()).strong().size(14.0));
    });
    hint(ui, tool.help());

    if tool.uses_brush() {
        let b = &tools.brush;
        hint(
            ui,
            &format!(
                "Brush: radius {}, intensity {:.0}% — change it in Brush Settings, pick another in Brushes.",
                b.radius.round(),
                b.strength * 100.0
            ),
        );
    }

    section(ui, Icon::SlidersHorizontal, "Options");
    match tool {
        Tool::Raise => {
            slider_row(ui, "Speed", |ui| {
                ui.add(egui::Slider::new(&mut tools.raise_speed, 1.0..=120.0).logarithmic(true).suffix(" b/s"))
            });
        }
        Tool::Flatten | Tool::SetHeight => {
            slider_row(ui, "Height", |ui| {
                ui.add(egui::DragValue::new(&mut tools.target_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
            });
            hint(ui, "Alt-click on the map to pick a height.");
        }
        Tool::Roughen => {
            slider_row(ui, "Amplitude", |ui| ui.add(egui::Slider::new(&mut tools.roughness, 0.5..=40.0).suffix(" blocks")));
        }
        Tool::PaintTerrain => {
            let def = doc.world.registry.terrain(TerrainId(tools.terrain));
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                let (top, side) = terrain_colors(&doc.world.registry, tools.terrain);
                cube_icon(ui.painter(), r, top, side);
                ui.label(RichText::new(&def.name).strong());
            });
            hint(ui, "Choose terrains in the Terrain panel; Alt-click picks one from the map.");
        }
        Tool::PaintBiome => {
            let reg = &doc.world.registry;
            let def = reg.biome(BiomeId(tools.biome));
            ui.horizontal(|ui| {
                swatch(ui, theme::rgb(def.color));
                ui.label(RichText::new(&def.name).strong());
            });
            hint(ui, "Choose biomes in the Biomes panel. Automatic lets the exporter decide from the terrain and the water.");
        }
        Tool::Flood | Tool::Water => {
            slider_row(ui, "Water level", |ui| {
                ui.add(egui::DragValue::new(&mut tools.water_level).range(doc.dimension().min_y as i16..=doc.dimension().max_y as i16))
            });
            ui.horizontal(|ui| {
                ui.add_space(76.0);
                if ui.small_button(format!("Sea level ({})", doc.world.meta.sea_level)).clicked() {
                    tools.water_level = doc.world.meta.sea_level as i16;
                }
            });
        }
        Tool::StampSlab => {
            slider_row(ui, "Thickness", |ui| {
                ui.add(egui::Slider::new(&mut tools.slab_thickness, 1.0..=128.0).logarithmic(true).suffix(" blocks"))
            });
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new("Centre"));
                ui.selectable_value(&mut tools.anchor, AnchorMode::AboveGround, "Above ground");
                ui.selectable_value(&mut tools.anchor, AnchorMode::Absolute, "At height");
            });
            match tools.anchor {
                AnchorMode::AboveGround => {
                    slider_row(ui, "Offset", |ui| ui.add(egui::DragValue::new(&mut tools.slab_offset).speed(0.5).suffix(" blocks")));
                    hint(ui, "Negative offsets put the slab underground: on a Carve level that makes a cave.");
                }
                AnchorMode::Absolute => {
                    slider_row(ui, "Height", |ui| {
                        ui.add(egui::DragValue::new(&mut tools.slab_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
                    });
                }
            }
        }
        Tool::Build | Tool::Dig | Tool::Smooth3d => {
            hint(
                ui,
                "Works best in the 3D view: the brush is a ball at the point under the cursor. There the middle button (or Alt + drag) turns the view, Shift + middle button pans.",
            );
            hint(ui, "On the map it sculpts at the surface. Size, intensity and profile come from Brush Settings.");
            let sculpted = doc.dimension().voxels.len();
            if sculpted > 0 {
                hint(ui, &format!("{sculpted} sculpted chunks of 16³ blocks in this dimension."));
            }
        }
        Tool::Eyedropper => hint(ui, "Picks terrain, biome and height, then returns to the previous tool."),
        Tool::Smooth | Tool::Drain | Tool::EraseSlab => hint(ui, "No other option: the brush is all there is."),
        Tool::Pan => hint(ui, "Two-finger scroll or the middle button pans with any tool; pinch or the wheel zooms."),
    }

    // Which level the tool edits.
    if !matches!(tool, Tool::Pan | Tool::Flood | Tool::Drain | Tool::Eyedropper) && !tool.is_sculpt() {
        section(ui, Icon::Layers, "Level");
        let levels: Vec<(String, LevelKind)> = doc.dimension().levels.iter().map(|l| (l.name.clone(), l.kind)).collect();
        ui.horizontal(|ui| {
            ui.add_sized([72.0, 18.0], egui::Label::new("Edit"));
            egui::ComboBox::from_id_salt("active level")
                .selected_text(level_label(&levels[doc.active_level.min(levels.len() - 1)]))
                .show_ui(ui, |ui| {
                    for (i, l) in levels.iter().enumerate() {
                        ui.selectable_value(&mut doc.active_level, i, level_label(l));
                    }
                });
        });
        if doc.active_kind() != LevelKind::Ground && !matches!(tool, Tool::StampSlab | Tool::EraseSlab) {
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new("Surface"));
                ui.selectable_value(&mut tools.surface, SurfaceChoice::Top, "Top");
                ui.selectable_value(&mut tools.surface, SurfaceChoice::Bottom, "Bottom");
            });
        }
        if tool.needs_slab_level() && doc.active_kind() == LevelKind::Ground {
            ui.label(RichText::new("This tool edits a Solid or Carve level.").color(pal.danger).size(12.0));
            ui.horizontal(|ui| {
                if ui.button(format!("{} Add solid level", theme::glyph(Icon::Plus))).clicked() {
                    doc.add_level(LevelKind::Solid);
                }
                if ui.button(format!("{} Add carve level", theme::glyph(Icon::Plus))).clicked() {
                    doc.add_level(LevelKind::Carve);
                }
            });
        }
    }
}

// --------------------------------------------------------------- brushes --

/// A brush thumbnail button.
fn brush_button(ui: &mut Ui, texture: Option<egui::TextureId>, selected: bool, size: f32) -> egui::Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    let fill = if selected {
        pal.accent_soft
    } else if response.hovered() {
        ui.visuals().widgets.hovered.weak_bg_fill
    } else {
        pal.header
    };
    let painter = ui.painter();
    painter.rect(rect, 6, fill, if selected { Stroke::new(1.5, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
    match texture {
        Some(tex) => {
            painter.image(tex, rect.shrink(4.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        }
        None => {
            painter.text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(14.0), pal.weak);
        }
    }
    response
}

/// Pick a brush. Returns false while a custom brush is not loaded.
pub fn select_brush(tools: &mut ToolState, library: &BrushLibrary, source: BrushSource) -> bool {
    match library.brush(&source, &tools.brush) {
        Some(brush) => {
            tools.brush = brush;
            tools.brush_source = source;
            true
        }
        None => false,
    }
}

fn brush_grid(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, brushes: &[(BrushSource, String)], add: Option<&mut bool>) {
    let dark = ui.visuals().dark_mode;
    let ctx = ui.ctx().clone();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        for (source, name) in brushes {
            let texture = library.thumbnail(&ctx, source, dark);
            let selected = tools.brush_source == *source;
            if brush_button(ui, texture, selected, 44.0).on_hover_text(name).clicked() {
                select_brush(tools, library, source.clone());
            }
        }
        if let Some(add) = add {
            let r = icon_button(ui, Icon::Plus, false, 44.0)
                .on_hover_text("Add brushes made from images (greyscale: white paints, black doesn't) to this group");
            *add |= r.clicked();
        }
    });
}

/// Ask for images and copy them into a group (Massif's own folder when
/// `group` is `None`).
fn add_brushes(library: &mut BrushLibrary, group: Option<&std::path::Path>, ctx: &egui::Context) -> Option<String> {
    let files = rfd::FileDialog::new()
        .set_title("Add brushes")
        .add_filter("Images", &["png", "jpg", "jpeg"])
        .pick_files()?;
    Some(match library.add_images(group, &files) {
        Ok(n) => {
            library.load_in_background(ctx);
            format!("Added {n} brush{}", if n == 1 { "" } else { "es" })
        }
        Err(e) => format!("Could not add the brushes: {e}"),
    })
}

/// The built-in brushes, the terrain stamps and the commands for your own.
pub fn brushes_panel(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, new_group: &mut Option<String>) -> Option<String> {
    let mut feedback = None;
    let named = |library: &BrushLibrary, list: &[BrushSource]| -> Vec<(BrushSource, String)> {
        list.iter().map(|s| (s.clone(), library.name(s))).collect()
    };
    section(ui, Icon::Circle, "Basic");
    let basic = named(library, &BrushSource::BASIC);
    brush_grid(ui, tools, library, &basic, None);
    section(ui, Icon::Shapes, "More shapes");
    let more = named(library, &BrushSource::MORE);
    brush_grid(ui, tools, library, &more, None);
    for family in Family::ALL {
        section(ui, Icon::MountainSnow, family.label());
        let stamps: Vec<(BrushSource, String)> = Stamp::ALL
            .iter()
            .filter(|s| s.family() == family)
            .map(|s| (BrushSource::Stamp(*s), s.label().to_owned()))
            .collect();
        brush_grid(ui, tools, library, &stamps, None);
    }

    section(ui, Icon::Images, "Your brushes");
    ui.horizontal_wrapped(|ui| {
        if ui.button(theme::with_icon(Icon::Plus, "Add brushes…")).clicked() {
            feedback = add_brushes(library, None, ui.ctx());
        }
        if ui.button(theme::with_icon(Icon::FolderPlus, "New group…")).clicked() {
            *new_group = Some(String::new());
        }
        if ui.button(theme::with_icon(Icon::FolderOpen, "Open the folder")).clicked()
            && let Err(e) = library.open_folder()
        {
            feedback = Some(format!("Could not open the brushes folder: {e}"));
        }
        if ui
            .add_enabled(!library.is_loading(), egui::Button::new(theme::with_icon(Icon::RefreshCw, "Reload")))
            .clicked()
        {
            library.load_in_background(ui.ctx());
        }
    });
    if let Some(name) = new_group {
        let mut done = None;
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(name).hint_text("Group name").desired_width(140.0));
            if !edit.has_focus() && name.is_empty() {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button("Create").clicked() || enter {
                done = Some(true);
            }
            if ui.button("Cancel").clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        match done {
            Some(true) => {
                feedback = Some(match library.new_group(name) {
                    Ok(_) => {
                        library.load_in_background(ui.ctx());
                        format!("Made the group “{}”", name.trim())
                    }
                    Err(e) => format!("Could not make the group: {e}"),
                });
                *new_group = None;
            }
            Some(false) => *new_group = None,
            None => {}
        }
    }
    hint(
        ui,
        "Brushes are greyscale images: white paints, black doesn't. Each folder in the brushes folder is a group of its own, in Custom Brushes; WorldPainter's brushes are there too.",
    );
    feedback
}

/// The groups of brushes made from images.
pub fn custom_brushes_panel(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary) -> Option<String> {
    let mut feedback = None;
    if !library.is_loaded() {
        ui.horizontal(|ui| {
            ui.spinner();
            hint(ui, "Loading the brushes…");
        });
        return None;
    }
    if library.groups.is_empty() {
        hint(ui, "No custom brushes yet. A brush is a greyscale image: white paints, black doesn't.");
        if ui.button(theme::with_icon(Icon::Plus, "Add brushes…")).clicked() {
            feedback = add_brushes(library, None, ui.ctx());
        }
        return feedback;
    }
    type Group = (String, std::path::PathBuf, bool, Vec<(BrushSource, String)>);
    let groups: Vec<Group> = library
        .groups
        .iter()
        .map(|g| {
            let entries = g.entries.iter().map(|e| (e.source.clone(), e.name.clone())).collect();
            (g.name.clone(), g.dir.clone(), g.writable, entries)
        })
        .collect();
    for (name, dir, writable, entries) in groups {
        section(ui, if writable { Icon::Folder } else { Icon::FolderOpen }, &format!("{name}  ·  {}", entries.len()));
        let mut add = false;
        brush_grid(ui, tools, library, &entries, writable.then_some(&mut add));
        if add {
            feedback = add_brushes(library, Some(&dir), ui.ctx());
        }
    }
    if !library.errors.is_empty() {
        let pal = Palette::of(ui);
        ui.label(RichText::new(format!("{} image(s) could not be read", library.errors.len())).color(pal.danger).size(11.5))
            .on_hover_text(library.errors.join("\n"));
    }
    feedback
}

/// Size, intensity and rotation of the brush, with a preview.
pub fn brush_settings(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary) {
    let pal = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0 - 84.0).clamp(60.0, 240.0);
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        let texture = library.preview(&ctx, &tools.brush_source, &tools.brush, dark);
        let (rect, _) = ui.allocate_exact_size(vec2(76.0, 76.0), Sense::hover());
        ui.painter().rect_filled(rect, 8, pal.header);
        ui.painter()
            .image(texture, rect.shrink(4.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(library.name(&tools.brush_source)).strong().size(14.0));
            let kind = match tools.brush_source {
                BrushSource::Symmetric(..) => "Built-in brush",
                BrushSource::Stamp(_) => "Terrain stamp",
                BrushSource::Image(_) => "Custom brush",
            };
            hint(ui, kind);
            let r = tools.brush.radius.round();
            hint(ui, &format!("{} blocks across", 2.0 * r + 1.0));
        });
    });
    ui.add_space(4.0);
    let b = &mut tools.brush;
    slider_row(ui, "Size", |ui| {
        ui.add(egui::Slider::new(&mut b.radius, 1.0..=MAX_RADIUS).logarithmic(true).suffix(" radius").max_decimals(0))
    });
    slider_row(ui, "Intensity", |ui| {
        ui.add(
            egui::Slider::new(&mut b.strength, 0.01..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    let mut degrees = if b.rotation > 180.0 { b.rotation - 360.0 } else { b.rotation };
    let turned = slider_row(ui, "Rotation", |ui| {
        ui.add(egui::Slider::new(&mut degrees, -180.0..=180.0).step_by(15.0).suffix("°").max_decimals(0))
    });
    if turned {
        b.rotation = degrees.rem_euclid(360.0);
    }
    ui.add_space(2.0);
    hint(
        ui,
        "Scroll on the map or − + for the size (with Shift: one block at a time), 1…0 for the intensity, [ ] to turn the brush. The right button inverts the tool.",
    );
}

fn level_label((name, kind): &(String, LevelKind)) -> String {
    format!("{}  {name}", theme::glyph(level_icon(*kind)))
}

pub fn level_icon(kind: LevelKind) -> Icon {
    match kind {
        LevelKind::Ground => Icon::Mountain,
        LevelKind::Solid => Icon::Cuboid,
        LevelKind::Carve => Icon::Pickaxe,
    }
}

// ---------------------------------------------------------------- terrain --

/// Top and side colour of a terrain, from the blocks of its recipe.
pub fn terrain_colors(reg: &Registry, terrain: u16) -> (Color32, Color32) {
    let def = reg.terrain(TerrainId(terrain));
    let block = |i: usize| def.strata.get(i).map(|s| theme::rgb(reg.block(s.blocks[0].0).color));
    let top = block(0).unwrap_or_else(|| theme::rgb(def.color));
    (top, block(1).unwrap_or(top))
}

/// A small isometric block.
pub fn cube_icon(painter: &egui::Painter, rect: Rect, top: Color32, side: Color32) {
    let w = (rect.width() * 0.46).min(rect.height() / 2.1);
    let h = w * 0.5;
    let c = rect.center().x;
    let y0 = rect.center().y - (2.0 * h + w * 1.05) / 2.0;
    let height = w * 1.05;
    let t = pos2(c, y0);
    let r = pos2(c + w, y0 + h);
    let b = pos2(c, y0 + 2.0 * h);
    let l = pos2(c - w, y0 + h);
    let down = vec2(0.0, height);
    let poly = |pts: Vec<egui::Pos2>, color: Color32| {
        painter.add(egui::Shape::convex_polygon(pts, color, Stroke::NONE));
    };
    poly(vec![l, b, b + down, l + down], theme::scale(side, 0.84));
    poly(vec![b, r, r + down, b + down], theme::scale(side, 0.66));
    if side != top {
        // The top block's colour runs a little down the sides (grass, podzol…).
        let band = vec2(0.0, height * 0.22);
        poly(vec![l, b, b + band, l + band], theme::scale(top, 0.8));
        poly(vec![b, r, r + band, b + band], theme::scale(top, 0.62));
    }
    poly(vec![t, r, b, l], top);
    let edge = Stroke::new(0.6, Color32::from_black_alpha(70));
    painter.add(egui::Shape::closed_line(vec![t, r, r + down, b + down, l + down, l], edge));
}

fn swatch(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().rect(r, 3, color, Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
}

/// Returns true when the solo setting changed.
pub fn terrain_panel(ui: &mut Ui, tools: &mut ToolState, reg: &Registry, solo: &mut bool, search: &mut String) -> bool {
    let pal = Palette::of(ui);
    hint(ui, "The block type of the ground surface: grass, sand, stone, snow…");
    let solo_changed = ui
        .checkbox(solo, "Solo")
        .on_hover_text("Fade every other terrain on the map")
        .changed();
    ui.add(egui::TextEdit::singleline(search).hint_text(format!("{}  Search terrain", theme::glyph(Icon::Search))).desired_width(f32::INFINITY));
    let needle = search.to_lowercase();
    let mut groups: Vec<&str> = Vec::new();
    for t in &reg.terrains {
        if !groups.contains(&t.group.as_str()) {
            groups.push(&t.group);
        }
    }
    let mut picked = None;
    for group in groups {
        let members: Vec<(usize, &massif_core::registry::TerrainDef)> = reg
            .terrains
            .iter()
            .enumerate()
            .filter(|(_, t)| t.group == group && (needle.is_empty() || t.name.to_lowercase().contains(&needle)))
            .collect();
        if members.is_empty() {
            continue;
        }
        ui.add_space(4.0);
        ui.label(RichText::new(group.to_uppercase()).size(10.5).strong().color(pal.weak));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for (i, def) in members {
                let selected = tools.terrain as usize == i;
                let (rect, response) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                let painter = ui.painter();
                let fill = if selected {
                    pal.accent_soft
                } else if response.hovered() {
                    pal.header
                } else {
                    Color32::TRANSPARENT
                };
                let stroke = if selected { Stroke::new(1.5, pal.accent) } else { Stroke::new(1.0, pal.border) };
                painter.rect(rect, 6, fill, stroke, StrokeKind::Inside);
                let (top, side) = terrain_colors(reg, i as u16);
                cube_icon(painter, rect.shrink(6.0), top, side);
                let tip = describe_terrain(reg, i as u16);
                if response.on_hover_text(tip).clicked() {
                    picked = Some(i as u16);
                }
                let _ = &def;
            }
        });
    }
    if let Some(t) = picked {
        tools.terrain = t;
        if tools.tool != Tool::PaintTerrain {
            tools.select(Tool::PaintTerrain);
        }
    }
    ui.add_space(6.0);
    ui.separator();
    ui.label(RichText::new(&reg.terrain(TerrainId(tools.terrain)).name).strong());
    hint(ui, &describe_terrain(reg, tools.terrain));
    solo_changed || (*solo && picked.is_some())
}

fn describe_terrain(reg: &Registry, terrain: u16) -> String {
    let def = reg.terrain(TerrainId(terrain));
    let mut parts: Vec<String> = def
        .strata
        .iter()
        .map(|s| {
            let names: Vec<&str> = s
                .blocks
                .iter()
                .map(|(b, _)| reg.block(*b).name().trim_start_matches("minecraft:"))
                .collect();
            let depth = if s.depth.0 == u16::MAX {
                "all the way down".to_owned()
            } else if s.depth.0 == s.depth.1 {
                format!("{}", s.depth.0)
            } else {
                format!("{}–{}", s.depth.0, s.depth.1)
            };
            format!("{} ({depth})", names.join(" / "))
        })
        .collect();
    if parts.is_empty() {
        parts.push("natural stone".into());
    }
    format!("{}: {}", def.name, parts.join(", then "))
}

// ----------------------------------------------------------------- biomes --

pub fn biome_panel(ui: &mut Ui, tools: &mut ToolState, reg: &Registry, search: &mut String) {
    hint(ui, "Painted biomes override the automatic choice (from terrain and water depth).");
    ui.add(egui::TextEdit::singleline(search).hint_text(format!("{}  Search biomes", theme::glyph(Icon::Search))).desired_width(f32::INFINITY));
    let needle = search.to_lowercase();
    for (i, b) in reg.biomes.iter().enumerate() {
        if !needle.is_empty() && !b.name.to_lowercase().contains(&needle) && !b.id.contains(&needle) {
            continue;
        }
        ui.horizontal(|ui| {
            swatch(ui, theme::rgb(b.color));
            let r = ui.selectable_label(tools.biome as usize == i, &b.name);
            if r.on_hover_text(if b.id.is_empty() { "Let the exporter decide" } else { &b.id }).clicked() {
                tools.biome = i as u16;
                tools.select(Tool::PaintBiome);
            }
        });
    }
}

// ----------------------------------------------------------------- levels --

/// What the levels panel changed.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct LevelsChange {
    /// The composed terrain may look different: redraw everything.
    pub composition: bool,
}

pub fn levels_panel(ui: &mut Ui, doc: &mut Document, rename: &mut Option<(usize, String)>) -> LevelsChange {
    let pal = Palette::of(ui);
    let mut change = LevelsChange::default();
    hint(
        ui,
        "The multi-level surface: the ground, then Solid levels that add matter (overhangs, arches, \
         floating islands) and Carve levels that remove it (caves, tunnels, pits). Higher in the list wins.",
    );
    ui.horizontal(|ui| {
        if ui.button(format!("{} Solid", theme::glyph(Icon::Plus))).on_hover_text("Add a level that adds matter").clicked() {
            doc.add_level(LevelKind::Solid);
        }
        if ui.button(format!("{} Carve", theme::glyph(Icon::Plus))).on_hover_text("Add a level that removes matter").clicked() {
            doc.add_level(LevelKind::Carve);
        }
    });
    ui.add_space(4.0);

    let n = doc.dimension().levels.len();
    let mut action: Option<(usize, &'static str)> = None;
    // Top of the stack first, like layers in an image editor.
    for index in (0..n).rev() {
        let (name, kind, visible, tiles) = {
            let l = &doc.dimension().levels[index];
            (l.name.clone(), l.kind, l.visible, l.len())
        };
        let active = doc.active_level == index;
        let frame = egui::Frame::new()
            .fill(if active { pal.accent_soft } else { Color32::TRANSPARENT })
            .stroke(Stroke::new(1.0, if active { pal.accent } else { pal.border }))
            .corner_radius(CornerRadius::same(6))
            .inner_margin(egui::Margin::symmetric(6, 3));
        frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                let eye = if visible { Icon::Eye } else { Icon::EyeOff };
                if index > 0
                    && ui
                        .add(egui::Button::new(theme::glyph(eye).to_string()).frame(false))
                        .on_hover_text(if visible { "Hide this level" } else { "Show this level" })
                        .clicked()
                {
                    action = Some((index, "toggle"));
                }
                if index == 0 {
                    ui.add_space(18.0);
                }
                ui.label(RichText::new(theme::glyph(level_icon(kind)).to_string()).color(pal.accent));
                match rename {
                    Some((i, text)) if *i == index => {
                        let r = ui.add(egui::TextEdit::singleline(text).desired_width(110.0));
                        if r.lost_focus() {
                            if ui.input(|i| !i.key_pressed(egui::Key::Escape)) && !text.trim().is_empty() {
                                action = Some((index, "rename"));
                            } else {
                                *rename = None;
                            }
                        } else {
                            r.request_focus();
                        }
                    }
                    _ => {
                        let label = egui::Label::new(RichText::new(&name).strong().color(if visible { pal.text } else { pal.weak }))
                            .sense(Sense::click())
                            .truncate();
                        let r = ui.add(label).on_hover_text(format!("{} level · {tiles} tiles · double-click to rename", kind.label()));
                        if r.clicked() {
                            doc.active_level = index;
                        }
                        if r.double_clicked() {
                            *rename = Some((index, name.clone()));
                        }
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if index > 0 {
                        if ui.small_button(theme::glyph(Icon::Trash2).to_string()).on_hover_text("Delete this level").clicked() {
                            action = Some((index, "delete"));
                        }
                        if index > 1 && ui.small_button("▼").on_hover_text("Move down").clicked() {
                            action = Some((index, "down"));
                        }
                        if index + 1 < n && ui.small_button("▲").on_hover_text("Move up").clicked() {
                            action = Some((index, "up"));
                        }
                    }
                    ui.label(RichText::new(kind.label()).size(10.5).color(pal.weak));
                });
            });
        });
        ui.add_space(2.0);
    }
    if let Some((index, what)) = action {
        match what {
            "toggle" => {
                let level = &mut doc.dimension_mut().levels[index];
                level.visible = !level.visible;
                doc.mark_changed();
                change.composition = true;
            }
            "rename" => {
                if let Some((_, text)) = rename.take() {
                    doc.rename_level(index, text.trim().to_owned());
                }
            }
            "delete" => {
                doc.remove_level(index);
                change.composition = true;
            }
            "up" => {
                doc.move_level(index, 1);
                change.composition = true;
            }
            "down" => {
                doc.move_level(index, -1);
                change.composition = true;
            }
            _ => {}
        }
    }
    change
}

// ------------------------------------------------------------------- info --

/// Returns true when a world setting that affects the map changed.
pub fn info_panel(ui: &mut Ui, doc: &mut Document, hover: Option<(i32, i32)>, cache: (usize, usize, usize)) -> bool {
    let pal = Palette::of(ui);
    let mut affects_map = false;
    section(ui, Icon::Globe, "World");
    egui::Grid::new("world info").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        let meta = &mut doc.world.meta;
        let mut changed = false;
        ui.label("Name");
        changed |= ui.text_edit_singleline(&mut meta.name).changed();
        ui.end_row();
        ui.label("Seed");
        changed |= ui.add(egui::DragValue::new(&mut meta.seed)).changed();
        ui.end_row();
        ui.label("Sea level");
        if ui.add(egui::DragValue::new(&mut meta.sea_level).range(-64..=319)).changed() {
            changed = true;
            affects_map = true;
        }
        ui.end_row();
        ui.label("Spawn");
        ui.horizontal(|ui| {
            for v in &mut meta.spawn {
                changed |= ui.add(egui::DragValue::new(v)).changed();
            }
        });
        ui.end_row();
        if changed {
            doc.mark_changed();
        }
    });

    section(ui, Icon::Map, "Map");
    let dim = doc.dimension();
    match dim.bounds() {
        Some((lo, hi)) => {
            let (w, d) = (hi.x - lo.x + 1, hi.z - lo.z + 1);
            ui.label(format!("{} × {} blocks ({w} × {d} tiles)", w * 128, d * 128));
            ui.label(RichText::new(format!("x {} … {}, z {} … {}", lo.x * 128, (hi.x + 1) * 128 - 1, lo.z * 128, (hi.z + 1) * 128 - 1)).color(pal.weak));
        }
        None => {
            ui.label("Empty");
        }
    }
    ui.label(RichText::new(format!("Build height {} … {}", dim.min_y, dim.max_y - 1)).color(pal.weak));

    section(ui, Icon::Crosshair, "Under the cursor");
    match hover {
        Some((x, z)) => column_info(ui, doc, x, z),
        None => hint(ui, "Hover the map to inspect a column."),
    }

    section(ui, Icon::Activity, "Memory");
    let mut hot = 0;
    let mut cold = 0;
    egui::Grid::new("memory").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
        for level in &doc.dimension().levels {
            let s = level.stats();
            hot += s.hot_bytes;
            cold += s.cold_bytes;
            ui.label(&level.name);
            ui.label(RichText::new(format!("{} tiles", s.tiles)).color(pal.weak));
            ui.label(RichText::new(format!("{} hot", s.hot_tiles)).color(pal.weak));
            ui.end_row();
        }
    });
    ui.label(format!("Decoded {} · compressed {}", human(hot as u64), human(cold as u64)));
    ui.label(RichText::new(format!("Map cache: {} textures, {} grids, {} summaries", cache.0, cache.1, cache.2)).color(pal.weak));

    section(ui, Icon::Save, "File");
    match &doc.path {
        Some(p) => {
            ui.label(RichText::new(p.display().to_string()).color(pal.weak).size(11.0));
        }
        None => hint(ui, "Not saved yet"),
    }
    if doc.is_modified() {
        ui.label(RichText::new("Unsaved changes").color(pal.accent));
    }
    affects_map
}

fn column_info(ui: &mut Ui, doc: &Document, x: i32, z: i32) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(format!("Column {x}, {z}")).strong());
    let dim = doc.dimension();
    let Some(stack) = TileStack::new(dim, TilePos::containing(x, z), true) else {
        hint(ui, "Outside the map");
        return;
    };
    let cell = local_index(x, z);
    let (mut spans, mut fluids) = (Vec::new(), Vec::new());
    stack.spans(cell, &mut spans);
    stack.fluids(cell, &spans, &mut fluids);
    let reg = &doc.world.registry;
    let mut rows: Vec<(i32, String)> = spans
        .iter()
        .map(|s| (s.y0, format!("{} … {}  {}", s.y0, s.y1 - 1, reg.terrain(s.terrain).name)))
        .collect();
    rows.extend(fluids.iter().map(|&(a, b)| (a, format!("{a} … {}  Water", b - 1))));
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, text) in rows {
        ui.label(RichText::new(text).monospace().size(11.5));
    }
    let mut sampler = ColumnSampler::default();
    let s = sampler.sample(&stack, cell);
    let biome = if s.biome == 0 {
        "automatic".to_owned()
    } else {
        reg.biome(BiomeId(s.biome)).name.clone()
    };
    ui.label(RichText::new(format!("Biome: {biome}")).color(pal.weak));
    let levels: Vec<String> = stack
        .layers()
        .filter(|(_, _, t)| t.present(cell))
        .map(|(i, _, _)| dim.levels[i].name.clone())
        .collect();
    ui.label(RichText::new(format!("Levels: {}", levels.join(", "))).color(pal.weak));
    let _ = NO_FLUID;
}

pub fn human(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KiB", "MiB", "GiB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit + 1 < UNITS.len() {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}

// ------------------------------------------------------------ empty area --

/// The chooser of an empty area. `shown` are the editors visible elsewhere.
pub fn empty_area(ui: &mut Ui, shown: &HashSet<EditorKind>) -> Option<EditorKind> {
    let pal = Palette::of(ui);
    let mut picked = None;
    ui.vertical_centered(|ui| {
        ui.add_space((ui.available_height() * 0.28).max(12.0));
        ui.label(RichText::new("What should this area show?").strong().size(15.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Any editor can be shown in any area. Drag an area by its title onto another to move it, \
                 or drop it in the middle to swap the two.",
            )
            .size(12.0)
            .color(pal.weak),
        );
        ui.add_space(14.0);
    });
    let width = ui.available_width();
    let columns = if width > 560.0 { 4 } else if width > 300.0 { 2 } else { 1 };
    ui.columns(columns, |cols| {
        for (n, (group, kinds)) in EditorKind::GROUPS.iter().enumerate() {
            let ui = &mut cols[n % columns];
            ui.label(RichText::new(*group).size(11.5).color(pal.weak));
            for &kind in *kinds {
                ui.horizontal(|ui| {
                    let r = ui.selectable_label(false, format!("{}  {}", theme::glyph(kind.icon()), kind.label()));
                    if shown.contains(&kind) {
                        ui.label(RichText::new(theme::glyph(Icon::Dot).to_string()).size(12.0).color(pal.accent)).on_hover_text("Already shown in another area");
                    }
                    if r.clicked() {
                        picked = Some(kind);
                    }
                });
            }
            ui.add_space(8.0);
        }
    });
    picked
}
