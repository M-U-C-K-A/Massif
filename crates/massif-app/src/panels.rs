//! The side editors: tools, tool settings, brushes, terrain and biome
//! palettes, levels, world info, and the chooser shown by empty areas.

use crate::blocks::BlockLibrary;
use crate::brushes::{BrushLibrary, BrushSource};
use crate::doc::Document;
use crate::layout::EditorKind;
use crate::map_render::ColumnSampler;
use crate::theme::{self, Icon, Palette, RichTextExt};
use crate::tools::{AnchorMode, SurfaceChoice, Tool, ToolState};
use egui::{Align, Color32, CornerRadius, Rect, RichText, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_core::compose::TileStack;
use massif_core::coords::local_index;
use massif_core::level::LevelKind;
use massif_core::registry::{BiomeId, Registry, TerrainId};
use massif_core::stamps::{Family, Stamp};
use massif_core::tile::NO_FLUID;
use massif_core::{MAX_RADIUS, TilePos};
use std::collections::HashSet;

fn section(ui: &mut Ui, icon: Icon, title: &str) {
    let pal = Palette::of(ui);
    ui.add_space(2.0);
    ui.label(
        RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase()))
            .size(10.5)
            .bold()
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
        egui::FontId::proportional(size * 0.6),
        if selected { pal.accent } else { pal.text },
    );
    response
}

// ------------------------------------------------------------------ tools --

pub fn tools_panel(ui: &mut Ui, tools: &mut ToolState) {
    let pal = Palette::of(ui);
    // A row a theme, set apart: its name, then its tools.
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
    let fill = theme::mix(pal.panel, pal.text, if pal.dark { 0.035 } else { 0.028 });
    for (group, icon, list) in Tool::GROUPS {
        egui::Frame::new()
            .fill(fill)
            .corner_radius(7)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    let short = group.split(' ').next().unwrap_or(group);
                    let (rect, name) = ui.allocate_exact_size(vec2(58.0, 28.0), Sense::hover());
                    ui.painter().text(
                        rect.left_center() + vec2(2.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        format!("{}  {short}", theme::glyph(icon)),
                        egui::FontId::proportional(11.5),
                        pal.weak,
                    );
                    name.on_hover_text(group);
                    for &tool in list {
                        let key = tool.shortcut_text().map(|k| format!("  ({k})")).unwrap_or_default();
                        let tip = format!("{}{key}\n{}", tool.label(), tool.help());
                        if icon_button(ui, tool.icon(), tools.tool == tool, 28.0)
                            .on_hover_text(tip)
                            .clicked()
                        {
                            tools.select(tool);
                        }
                    }
                });
            });
    }
}

// --------------------------------------------------------- tool settings --

/// The tools of the 3D Sculpt editor, along its top: which one, with which
/// brush (custom ones too), how big, how hard.
pub fn sculpt_toolbar(ui: &mut Ui, s: &mut crate::tools::SculptSettings, library: &mut BrushLibrary) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
        ui.spacing_mut().interact_size.y = 22.0;
        for tool in crate::tools::SculptSettings::TOOLS {
            if ui
                .selectable_label(s.tool == tool, theme::with_icon(tool.icon(), tool.label()))
                .on_hover_text(tool.help())
                .clicked()
            {
                s.tool = tool;
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        // The brush: any of the library's, a custom image too.
        let name = library.name(&s.source);
        ui.menu_button(theme::with_icon(Icon::Brush, &name), |ui| {
            ui.set_max_width(330.0);
            let dark = ui.visuals().dark_mode;
            let ctx = ui.ctx().clone();
            let mut lists: Vec<(String, Vec<BrushSource>)> = vec![
                ("Basic".into(), BrushSource::BASIC.to_vec()),
                ("More shapes".into(), BrushSource::MORE.to_vec()),
                ("Stamps".into(), Stamp::ALL.iter().map(|t| BrushSource::Stamp(*t)).collect()),
            ];
            for g in &library.groups {
                lists.push((g.name.clone(), g.entries.iter().map(|e| e.source.clone()).collect()));
            }
            egui::ScrollArea::vertical().max_height(420.0).show(ui, |ui| {
                for (title, list) in lists {
                    if list.is_empty() {
                        continue;
                    }
                    ui.label(RichText::new(title).size(11.0).color(pal.weak));
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                        for source in list {
                            let texture = library.thumbnail(&ctx, &source, dark);
                            let tip = library.name(&source);
                            if brush_button(ui, texture, s.source == source, 40.0).on_hover_text(tip).clicked()
                                && let Some(brush) = library.brush(&source, &s.brush)
                            {
                                s.brush = brush;
                                s.source = source;
                                ui.close();
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        });
        ui.add_space(6.0);
        ui.add(crate::widgets::Field::new(&mut s.brush.radius, 1.0..=64.0).text("Radius").suffix(" blocks").max_decimals(0).width(150.0));
        ui.add(
            crate::widgets::Field::new(&mut s.brush.strength, 0.05..=1.0)
                .text("Strength")
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|t| t.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
                .width(150.0),
        );
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("Left: add · right: dig · Shift+drag: move · Alt+drag: turn · Alt+wheel: size")
                    .size(11.5)
                    .color(pal.weak),
            );
        });
    });
}

/// A row like [`slider_row`] that says what the setting is for.
fn tip_row(ui: &mut Ui, label: &str, tip: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        ui.spacing_mut().interact_size.y = 18.0;
        let label = ui.add_sized([72.0, 18.0], egui::Label::new(label).truncate()).on_hover_text(tip);
        add(ui).on_hover_text(tip).labelled_by(label.id).changed()
    })
    .inner
}

fn slider_row(ui: &mut Ui, label: &str, add: impl FnOnce(&mut Ui) -> egui::Response) -> bool {
    ui.horizontal(|ui| {
        // Slim sliders: the handle's size follows the row height.
        ui.spacing_mut().interact_size.y = 18.0;
        let label = ui.add_sized([72.0, 18.0], egui::Label::new(label).truncate());
        add(ui).labelled_by(label.id).changed()
    })
    .inner
}

pub fn tool_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0 - 84.0).clamp(60.0, 220.0);
    let tool = tools.tool;
    ui.horizontal(|ui| {
        ui.label(RichText::new(theme::glyph(tool.icon()).to_string()).size(18.0).color(pal.accent));
        ui.label(RichText::new(tool.label()).bold().size(14.0));
    });
    hint(ui, tool.help());

    section(ui, Icon::SlidersHorizontal, "Options");
    match tool {
        Tool::Raise => {
            slider_row(ui, "Speed", |ui| {
                ui.add(crate::widgets::Field::new(&mut tools.raise_speed, 1.0..=120.0).logarithmic(true).suffix(" b/s"))
            });
        }
        Tool::Flatten | Tool::SetHeight => {
            slider_row(ui, "Height", |ui| {
                ui.add(egui::DragValue::new(&mut tools.target_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
            });
            hint(ui, "Alt-click on the map to pick a height.");
        }
        Tool::Roughen => {
            slider_row(ui, "Amplitude", |ui| ui.add(crate::widgets::Field::new(&mut tools.roughness, 0.5..=40.0).suffix(" blocks")));
        }
        Tool::PaintTerrain => {
            ui.checkbox(&mut tools.pattern.on, "a pattern of terrains")
                .on_hover_text("Several terrains at once, mixed by a noise");
            if tools.pattern.on {
                pattern_settings(ui, tools, doc, blocks);
            } else {
                let def = doc.world.registry.terrain(TerrainId(tools.terrain));
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                    terrain_icon(ui, r, &doc.world.registry, tools.terrain, blocks);
                    ui.label(RichText::new(&def.name).bold());
                });
                hint(
                    ui,
                    "Pick it in Terrain or Blocks; Alt-click picks it on the map.",
                );
            }
        }
        Tool::PaintBiome => {
            let reg = &doc.world.registry;
            let def = reg.biome(BiomeId(tools.biome));
            ui.horizontal(|ui| {
                swatch(ui, theme::rgb(def.color));
                ui.label(RichText::new(&def.name).bold());
            });
            hint(ui, "Pick it in Biomes.");
        }
        Tool::AutoBiome | Tool::AutoPaint => {
            let biomes = tool == Tool::AutoBiome;
            hint(ui, "Hover: see what changes, in colour · drag: paint it.");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui
                    .button(theme::with_icon(Icon::Map, "Whole map"))
                    .on_hover_text(if biomes {
                        "Set the biomes of the whole map (undoable)"
                    } else {
                        "Paint the whole map (undoable)"
                    })
                    .clicked()
                {
                    let changed = if biomes { doc.auto_biome(&tools.auto) } else { doc.auto_paint(&tools.auto) };
                    tools.auto_done = Some((biomes, changed));
                }
                if ui
                    .button(theme::with_icon(Icon::Settings2, "Rules…"))
                    .on_hover_text("The biome of each kind of land, the terrains of each biome")
                    .clicked()
                {
                    tools.open_rules = true;
                }
            });
            if let Some((_, changed)) = tools.auto_done.filter(|(b, _)| *b == biomes) {
                hint(ui, &format!("{changed} columns changed over the whole map."));
            }
        }
        Tool::River => {
            let r = &mut tools.river;
            fn percent(v: &mut f32) -> crate::widgets::Field<'_> {
                crate::widgets::Field::new(v, 0.0..=1.0)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0))
            }
            let mut changed = false;
            changed |= tip_row(ui, "Width", "Width of the water at the source", |ui| ui.add(crate::widgets::Field::new(&mut r.width, 2.0..=200.0).suffix(" blocks").max_decimals(0)));
            changed |= tip_row(ui, "Flow", "How much wider and deeper it grows downstream", |ui| ui.add(percent(&mut r.flow)));
            changed |= tip_row(ui, "Depth", "Depth of the water at the source", |ui| ui.add(crate::widgets::Field::new(&mut r.depth, 1.0..=30.0).suffix(" blocks").max_decimals(0)));
            changed |= tip_row(ui, "Erosion", "How much it wears the land beside its bed", |ui| ui.add(percent(&mut r.erosion)));
            changed |= tip_row(ui, "Banks", "How far its banks slope back up to the land", |ui| ui.add(crate::widgets::Field::new(&mut r.banks, 0.0..=120.0).suffix(" blocks").max_decimals(0)));
            changed |= tip_row(ui, "Lakes", "Which hollows on its way fill up as lakes: none, the deep ones, all", |ui| ui.add(percent(&mut r.lakes)));
            ui.checkbox(&mut r.water, "Fill it with water");
            ui.checkbox(&mut r.biome, "River biome on its water");
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new("Bed"));
                let shown = if r.bed.is_empty() { "As it is".to_owned() } else { r.bed.clone() };
                crate::widgets::combo("river bed").selected_text(shown).width(150.0).show_ui(ui, |ui| {
                    if ui.selectable_label(r.bed.is_empty(), "As it is").clicked() {
                        r.bed.clear();
                    }
                    for t in &doc.world.registry.terrains {
                        if ui.selectable_label(t.name == r.bed, &t.name).clicked() {
                            r.bed = t.name.clone();
                        }
                    }
                });
            });
            let spec = r.spring_spec(&doc.world.registry);
            hint(
                ui,
                &format!(
                    "{:.0} × {:.0} at the source, {:.0} × {:.0} at the end (width × depth).",
                    spec.width, spec.depth, spec.width_end, spec.depth_end
                ),
            );
            ui.add_space(4.0);
            if changed && let Some(draft) = &mut tools.river_draft {
                draft.plan(doc, &tools.river, None);
            }
            match &tools.river_draft {
                None => hint(ui, "Click on land: where it springs. The water finds its way down."),
                Some(draft) => {
                    ui.label(RichText::new(draft.summary()).strong());
                    hint(ui, "Drag the river to guide it · click elsewhere: where it ends · right-click a point: remove it.");
                    ui.horizontal(|ui| {
                        if ui.button(theme::with_icon(Icon::Route, "Dig river")).on_hover_text("Enter").clicked() {
                            tools.river_note = crate::tools::dig_draft(doc, tools);
                        }
                        if ui.button("Cancel").on_hover_text("Esc").clicked() {
                            tools.river_draft = None;
                        }
                    });
                }
            }
            if let Some(note) = &tools.river_note {
                hint(ui, note);
            }
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
                ui.add(crate::widgets::Field::new(&mut tools.slab_thickness, 1.0..=128.0).logarithmic(true).suffix(" blocks"))
            });
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new("Centre"));
                ui.selectable_value(&mut tools.anchor, AnchorMode::AboveGround, "Above ground");
                ui.selectable_value(&mut tools.anchor, AnchorMode::Absolute, "At height");
            });
            match tools.anchor {
                AnchorMode::AboveGround => {
                    slider_row(ui, "Offset", |ui| {
                        ui.add(
                            egui::DragValue::new(&mut tools.slab_offset)
                                .speed(0.5)
                                .suffix(" blocks"),
                        )
                    });
                    hint(ui, "Negative: underground.");
                }
                AnchorMode::Absolute => {
                    slider_row(ui, "Height", |ui| {
                        ui.add(egui::DragValue::new(&mut tools.slab_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
                    });
                }
            }
        }
        Tool::Build | Tool::Dig | Tool::Smooth3d => {
            hint(ui, "Best in the 3D view.");
            let sculpted = doc.dimension().voxels.len();
            if sculpted > 0 {
                hint(ui, &format!("{sculpted} sculpted chunks of 16³ blocks in this dimension."));
            }
        }
        Tool::Eyedropper | Tool::Smooth | Tool::Drain | Tool::EraseSlab | Tool::Pan => {}
    }

    // Which level the tool edits.
    if !matches!(tool, Tool::Pan | Tool::Flood | Tool::Drain | Tool::Eyedropper) && !tool.is_sculpt() {
        section(ui, Icon::Layers, "Level");
        let levels: Vec<(String, LevelKind)> = doc.dimension().levels.iter().map(|l| (l.name.clone(), l.kind)).collect();
        ui.horizontal(|ui| {
            ui.add_sized([72.0, 18.0], egui::Label::new("Edit"));
            crate::widgets::combo("active level")
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
                .on_hover_text("Add greyscale images as brushes");
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
        "Greyscale images: white paints, black doesn't.",
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
        hint(ui, "No custom brushes yet.");
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

/// Size, intensity and rotation of the brush, with a preview, then the
/// options that limit where it acts (WorldPainter's brush options).
pub fn brush_settings(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, reg: &Registry, min_y: i32, max_y: i32, light: f32) {
    let pal = Palette::of(ui);
    let dark = ui.visuals().dark_mode;
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0 - 84.0).clamp(60.0, 260.0);
    let ctx = ui.ctx().clone();
    ui.horizontal(|ui| {
        let texture = library.preview(&ctx, &tools.brush_source, &tools.brush, dark);
        let (rect, _) = ui.allocate_exact_size(vec2(76.0, 76.0), Sense::hover());
        ui.painter().rect_filled(rect, 8, pal.header);
        ui.painter()
            .image(texture, rect.shrink(4.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        ui.vertical(|ui| {
            ui.add_space(6.0);
            ui.label(RichText::new(library.name(&tools.brush_source)).bold().size(14.0));
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
        ui.add(crate::widgets::Field::new(&mut b.radius, 1.0..=MAX_RADIUS).logarithmic(true).suffix(" radius").max_decimals(0))
    });
    slider_row(ui, "Intensity", |ui| {
        ui.add(
            crate::widgets::Field::new(&mut b.strength, 0.01..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    let mut degrees = if b.rotation > 180.0 { b.rotation - 360.0 } else { b.rotation };
    let turned = slider_row(ui, "Rotation", |ui| {
        // No steps: the wheel turns the brush 5° (or 1°) at a time.
        ui.add(crate::widgets::Field::new(&mut degrees, -180.0..=180.0).suffix("°").max_decimals(0))
    });
    if turned {
        b.rotation = degrees.rem_euclid(360.0);
    }
    ui.add_space(2.0);
    hint(
        ui,
        if cfg!(target_os = "macos") {
            "Scroll: size · 1…0: intensity · ⌘+scroll: turn"
        } else {
            "Scroll: size · 1…0: intensity · Ctrl+Shift+scroll: turn"
        },
    );

    // ---- options ---------------------------------------------------------
    let f = &mut tools.filter;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Options").bold());
        if f.is_active() {
            ui.label(RichText::new(theme::glyph(Icon::Funnel).to_string()).color(pal.accent))
                .on_hover_text("A filter is on: the brushes only act where it lets them");
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button("Reset").on_hover_text("Act everywhere again").clicked() {
                    *f = massif_core::filter::Filter::default();
                }
            });
        }
    });

    section(ui, Icon::ArrowUpDown, "Height limits");
    height_graph(ui, f, min_y, max_y);
    ui.horizontal(|ui| {
        let both = f.above.is_some() && f.below.is_some();
        ui.add_enabled_ui(both, |ui| {
            let half = (ui.available_width() - 8.0) / 2.0;
            if ui.add_sized([half, 20.0], egui::Button::selectable(!f.beyond, "Between")).clicked() {
                f.beyond = false;
            }
            if ui.add_sized([half, 20.0], egui::Button::selectable(f.beyond, "Beyond")).clicked() {
                f.beyond = true;
            }
        });
    });
    let caption = match (f.above, f.below) {
        (None, None) => "Applies at every height".to_owned(),
        (Some(a), None) => format!("Applies at {a} and above"),
        (None, Some(b)) => format!("Applies at {b} and below"),
        (Some(a), Some(b)) if f.beyond => format!("Applies below {a} and above {b}"),
        (Some(a), Some(b)) => format!("Applies from {a} to {b}"),
    };
    ui.vertical_centered(|ui| hint(ui, &caption));
    ui.horizontal_wrapped(|ui| {
        let mut above = f.above.is_some();
        if ui.checkbox(&mut above, "at or above").changed() {
            f.above = above.then_some(f.below.map_or(64, |b| b.min(64)));
        }
        let mut v = f.above.unwrap_or(min_y);
        if ui.add_enabled(above, egui::DragValue::new(&mut v).range(min_y..=max_y)).changed() {
            f.above = Some(v);
        }
        ui.add_space(16.0);
        let mut below = f.below.is_some();
        if ui.checkbox(&mut below, "at or below").changed() {
            f.below = below.then_some(f.above.map_or(100, |a| a.max(100)));
        }
        let mut v = f.below.unwrap_or(max_y - 1);
        if ui.add_enabled(below, egui::DragValue::new(&mut v).range(min_y..=max_y)).changed() {
            f.below = Some(v);
        }
    });
    ui.add_enabled(
        f.above.is_some() || f.below.is_some(),
        egui::Checkbox::new(&mut f.fade, format!("fade at the limits ({} blocks)", massif_core::filter::FADE)),
    );

    section(ui, Icon::Layers, "What is on the map");
    match_list(ui, "only on", &mut f.only_on, reg, "only on");
    match_list(ui, "except on", &mut f.except_on, reg, "except on");

    section(ui, Icon::TrendingUp, "Slope");
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.slope_above, "above");
        ui.checkbox(&mut f.slope_below, "below");
        ui.add_enabled(
            f.slope_above || f.slope_below,
            egui::DragValue::new(&mut f.slope).range(0.0..=90.0).speed(0.5).suffix("°"),
        );
    });

    section(ui, Icon::Compass, "Facing");
    ui.horizontal(|ui| {
        facing_dial(ui, f);
        ui.vertical(|ui| {
            ui.checkbox(&mut f.facing_on, "only slopes facing")
                .on_hover_text("Only slopes facing this way.");
            ui.horizontal(|ui| {
                ui.add_enabled(
                    f.facing_on,
                    egui::DragValue::new(&mut f.facing)
                        .range(0.0..=360.0)
                        .speed(1.0)
                        .max_decimals(0)
                        .suffix("°")
                        .custom_formatter(|v, _| format!("{} {:.0}", compass(v as f32), v)),
                );
                ui.label("±");
                ui.add_enabled(f.facing_on, egui::DragValue::new(&mut f.facing_spread).range(5.0..=180.0).speed(1.0).max_decimals(0).suffix("°"));
            });
            ui.horizontal(|ui| {
                if ui
                    .small_button(theme::with_icon(Icon::Sun, "Sun"))
                    .on_hover_text("The slopes the light of the map falls on (R and L turn it)")
                    .clicked()
                {
                    f.facing_on = true;
                    f.facing = light.rem_euclid(360.0);
                }
                if ui
                    .small_button(theme::with_icon(Icon::Moon, "Shade"))
                    .on_hover_text("The slopes turned away from the light of the map")
                    .clicked()
                {
                    f.facing_on = true;
                    f.facing = (light + 180.0).rem_euclid(360.0);
                }
            });
        });
    });
    f.facing = f.facing.rem_euclid(360.0);

    section(ui, Icon::Waves, "Curvature");
    ui.horizontal(|ui| {
        ui.checkbox(&mut f.hollows, "hollows").on_hover_text("Columns lower than the ones around them");
        ui.checkbox(&mut f.bumps, "bumps").on_hover_text("Columns higher than the ones around them");
    });
    let on = f.hollows || f.bumps;
    ui.horizontal(|ui| {
        ui.add_enabled(on, egui::DragValue::new(&mut f.depth).range(0.1..=64.0).speed(0.1).max_decimals(1));
        ui.label("blocks deep");
        ui.add_enabled(on, egui::DragValue::new(&mut f.radius).range(1..=64));
        ui.label("blocks around");
    });
}

/// The pattern of the Paint tool: its shape, its terrains and their
/// weights, and a picture of it.
fn pattern_settings(ui: &mut Ui, tools: &mut ToolState, doc: &Document, blocks: &mut BlockLibrary) {
    use massif_core::pattern::PatternKind;
    let reg = &doc.world.registry;
    let ps = &mut tools.pattern;
    let p = &mut ps.pattern;
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new("Shape"));
        crate::widgets::combo("pattern kind").selected_text(p.kind.label()).width(120.0).show_ui(ui, |ui| {
            for k in PatternKind::ALL {
                ui.selectable_value(&mut p.kind, k, k.label());
            }
        });
    });
    slider_row(ui, "Size", |ui| ui.add(crate::widgets::Field::new(&mut p.scale, 2.0..=600.0).suffix(" blocks").max_decimals(0)));
    if p.kind == PatternKind::Stripes {
        slider_row(ui, "Direction", |ui| ui.add(crate::widgets::Field::new(&mut p.angle, 0.0..=180.0).suffix("°").max_decimals(0)));
    }
    slider_row(ui, "Soft edges", |ui| {
        ui.add(
            crate::widgets::Field::new(&mut p.softness, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new("Seed"));
        ui.add(egui::DragValue::new(&mut p.seed));
        if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text("Another layout").clicked() {
            p.seed = p.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 100_000;
        }
    });
    ui.add_space(4.0);
    // The terrains and their shares.
    let total: f32 = ps.terrains.iter().map(|t| t.1.max(0.0)).sum::<f32>().max(1e-6);
    let mut remove = None;
    for (k, (name, weight)) in ps.terrains.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let id = reg.terrain_by_name(name);
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            if let Some(t) = id {
                terrain_icon(ui, r, reg, t.0, blocks);
            }
            crate::widgets::combo(("pattern terrain", k)).selected_text(name.as_str()).width(110.0).show_ui(ui, |ui| {
                for t in &reg.terrains {
                    if ui.selectable_label(t.name == *name, &t.name).clicked() {
                        *name = t.name.clone();
                    }
                }
            });
            ui.add(egui::DragValue::new(weight).range(0.0..=100.0).speed(0.05).max_decimals(1));
            ui.label(RichText::new(format!("{:.0}%", weight.max(0.0) / total * 100.0)).size(11.0).color(Palette::of(ui).weak));
            if ui.small_button(theme::glyph(Icon::X).to_string()).clicked() {
                remove = Some(k);
            }
        });
    }
    if let Some(k) = remove {
        ps.terrains.remove(k);
    }
    if ui
        .button(theme::with_icon(Icon::Plus, "Add the selected terrain"))
        .on_hover_text("From Terrain or Blocks")
        .clicked()
    {
        ps.terrains.push((reg.terrain(TerrainId(tools.terrain)).name.clone(), 1.0));
    }
    // A picture of the pattern, eight of its features across.
    let key = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        format!("{:?}{:?}", tools.pattern.pattern, tools.pattern.terrains).hash(&mut h);
        reg.terrains.iter().map(|t| t.color).collect::<Vec<_>>().hash(&mut h);
        h.finish()
    };
    let id = egui::Id::unique("pattern picture");
    let cached: Option<(u64, egui::TextureHandle)> = ui.ctx().data(|d| d.get_temp(id));
    let texture = match cached {
        Some((k, t)) if k == key => Some(t),
        _ => tools.pattern.sampler(reg).map(|sampler| {
            let n = 128usize;
            let span = (tools.pattern.pattern.scale * 8.0).max(32.0) / n as f32;
            let rgb: Vec<u8> = (0..n * n)
                .flat_map(|c| {
                    let (x, z) = (((c % n) as f32 * span) as i32, ((c / n) as f32 * span) as i32);
                    sampler.terrain(x, z).map_or([60, 60, 60], |t| reg.terrain(t).color)
                })
                .collect();
            let t = ui.ctx().load_texture("pattern picture", egui::ColorImage::from_rgb([n, n], &rgb), egui::TextureOptions::NEAREST);
            ui.ctx().data_mut(|d| d.insert_temp(id, (key, t.clone())));
            t
        }),
    };
    if let Some(t) = texture {
        ui.add_space(4.0);
        ui.add(egui::Image::new(&t).fit_to_exact_size(vec2(ui.available_width().min(220.0), ui.available_width().min(220.0))).corner_radius(6));
        hint(ui, &format!("{:.0} blocks across", tools.pattern.pattern.scale * 8.0));
    }
}

/// The rules of the automatic biomes: how the land is read, and the biome
/// of each kind of it.
pub fn land_rules(ui: &mut Ui, rules: &mut massif_core::auto::BiomeRules, reg: &Registry) {
    use massif_core::auto::Land;
    let pal = Palette::of(ui);
    section(ui, Icon::Ruler, "Reading the land");
    egui::Grid::new("land thresholds").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
        let blocks = |ui: &mut Ui, label: &str, v: &mut f32, hover: &str| {
            ui.label(label).on_hover_text(hover);
            ui.add(egui::DragValue::new(v).range(0.0..=384.0).speed(0.5).max_decimals(0).suffix(" blocks"));
        };
        blocks(ui, "Deep water from", &mut rules.deep_water, "Water at least this deep is deep water");
        ui.label("Rivers up to").on_hover_text("Water with a bank on both sides within this width is a river");
        ui.add(egui::DragValue::new(&mut rules.river_width).range(4..=64).suffix(" blocks wide"));
        ui.end_row();
        blocks(ui, "Shores up to", &mut rules.shore_height, "Ground at most this high over the water beside it is its shore");
        ui.label("Cliffs from").on_hover_text("Slopes at least this steep are rock, whatever grows on them");
        ui.add(egui::DragValue::new(&mut rules.steep).range(10.0..=85.0).speed(0.5).max_decimals(0).suffix("°"));
        ui.end_row();
        blocks(ui, "Hills from", &mut rules.hills, "Height over the sea where the hills begin");
        blocks(ui, "Mountains from", &mut rules.mountains, "Height over the sea where the mountains begin");
        ui.end_row();
        blocks(ui, "Peaks from", &mut rules.peaks, "Height over the sea where the peaks begin");
        ui.end_row();
    });
    rules.mountains = rules.mountains.max(rules.hills);
    rules.peaks = rules.peaks.max(rules.mountains);
    section(ui, Icon::Trees, "The biome of each kind of land");
    egui::Grid::new("land biomes").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
        for land in Land::ALL {
            ui.label(land.label()).on_hover_text(land.hint());
            let current = rules.biome_of(land).to_owned();
            let name = reg.biome_by_id(&current).map_or(current.as_str(), |b| reg.biome(b).name.as_str()).to_owned();
            let mut picked = None;
            ui.horizontal(|ui| {
                if let Some(b) = reg.biome_by_id(&current) {
                    swatch(ui, theme::rgb(reg.biome(b).color));
                }
                crate::widgets::combo(("land biome", land)).selected_text(name).width(210.0).show_ui(ui, |ui| {
                    for biome in reg.biomes.iter().skip(1) {
                        if ui.selectable_label(biome.id == current, &biome.name).clicked() {
                            picked = Some(biome.id.clone());
                        }
                    }
                });
                ui.label(RichText::new(land.hint()).size(11.0).color(pal.weak));
            });
            if let Some(id) = picked {
                rules.set_biome(land, &id);
            }
            ui.end_row();
        }
    });
}

/// The rules of the automatic paint: the terrains of each biome.
pub fn paint_rules(ui: &mut Ui, rules: &mut massif_core::auto::PaintRules, reg: &Registry) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.label("Cliffs from").on_hover_text("Slopes at least this steep get the cliffs' terrain");
        ui.add(egui::DragValue::new(&mut rules.steep).range(10.0..=85.0).speed(0.5).max_decimals(0).suffix("°"));
        ui.add_space(12.0);
        ui.checkbox(&mut rules.read_the_land, "where the biome is automatic, paint as the land is")
            .on_hover_text("Unset biomes count as automatic");
    });
    ui.add_space(4.0);
    egui::Grid::new("biome paint").num_columns(4).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
        for title in ["Biome", "Ground", "Cliffs", "Under water"] {
            ui.label(RichText::new(title).size(11.5).color(pal.weak));
        }
        ui.end_row();
        for biome in reg.biomes.iter().skip(1) {
            ui.horizontal(|ui| {
                swatch(ui, theme::rgb(biome.color));
                ui.label(&biome.name);
            });
            let paint = rules.of(&biome.id).cloned();
            let mut changed: Option<(usize, String)> = None;
            for k in 0..3 {
                let current = paint.as_ref().map_or("", |p| [p.ground.as_str(), p.cliffs.as_str(), p.underwater.as_str()][k]).to_owned();
                let shown = if current.is_empty() { "— as it is".to_owned() } else { current.clone() };
                crate::widgets::combo(("biome paint", &biome.id, k)).selected_text(shown).width(128.0).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_empty(), "— as it is").clicked() {
                        changed = Some((k, String::new()));
                    }
                    for terrain in &reg.terrains {
                        if ui.selectable_label(terrain.name == current, &terrain.name).clicked() {
                            changed = Some((k, terrain.name.clone()));
                        }
                    }
                });
            }
            if let Some((k, name)) = changed {
                let p = rules.of_mut(&biome.id);
                match k {
                    0 => p.ground = name,
                    1 => p.cliffs = name,
                    _ => p.underwater = name,
                }
            }
            ui.end_row();
        }
    });
}

/// The point of the compass nearest a direction in degrees.
fn compass(degrees: f32) -> &'static str {
    const POINTS: [&str; 8] = ["N", "NE", "E", "SE", "S", "SW", "W", "NW"];
    POINTS[((degrees.rem_euclid(360.0) + 22.5) / 45.0) as usize % 8]
}

/// A compass to point the facing test: north up, as on the map. Dragging
/// turns it; the wedge is the spread it accepts.
fn facing_dial(ui: &mut Ui, f: &mut massif_core::filter::Filter) {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(64.0, 64.0), Sense::click_and_drag());
    let (centre, radius) = (rect.center(), 26.0);
    if let Some(p) = response.interact_pointer_pos().filter(|_| response.dragged() || response.clicked()) {
        let d = p - centre;
        if d.length() > 3.0 {
            // Up is north, clockwise to the east.
            f.facing = d.x.atan2(-d.y).to_degrees().rem_euclid(360.0).round();
            f.facing_on = true;
        }
    }
    let painter = ui.painter_at(rect);
    let on = f.facing_on;
    let ink = if on { pal.accent } else { pal.weak };
    painter.circle(centre, radius, pal.header, Stroke::new(1.0, pal.border));
    let at = |degrees: f32, r: f32| {
        let a = degrees.to_radians();
        centre + vec2(a.sin(), -a.cos()) * r
    };
    if on {
        // The wedge of directions let through.
        let spread = f.facing_spread.clamp(1.0, 180.0);
        let steps = (spread / 6.0).ceil().max(2.0) as i32;
        let mut points = vec![centre];
        for k in -steps..=steps {
            points.push(at(f.facing + spread * k as f32 / steps as f32, radius - 1.0));
        }
        if spread >= 179.0 {
            painter.circle_filled(centre, radius - 1.0, ink.gamma_multiply(0.28));
        } else {
            // A fan of triangles: the wedge may be wider than half a turn.
            for pair in points[1..].windows(2) {
                painter.add(egui::Shape::convex_polygon(vec![centre, pair[0], pair[1]], ink.gamma_multiply(0.28), Stroke::NONE));
            }
        }
    }
    for (degrees, name) in [(0.0, "N"), (90.0, "E"), (180.0, "S"), (270.0, "W")] {
        painter.text(at(degrees, radius - 7.0), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(8.5), pal.weak);
    }
    painter.line_segment([centre, at(f.facing, radius - 2.0)], Stroke::new(if on { 2.0 } else { 1.0 }, ink));
    painter.circle_filled(at(f.facing, radius - 2.0), 3.0, ink);
    painter.circle_filled(centre, 2.0, ink);
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text("Drag to turn (north up)");
}

/// A stylised cross-section of a landscape, with the height band the filter
/// lets through. The limits can be dragged.
fn height_graph(ui: &mut Ui, f: &mut massif_core::filter::Filter, min_y: i32, max_y: i32) {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 120.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4, pal.header);
    let gutter = 34.0;
    let plot = Rect::from_min_max(pos2(rect.left() + gutter, rect.top() + 4.0), pos2(rect.right() - 4.0, rect.bottom() - 4.0));
    let (lo, hi) = (min_y as f32, max_y as f32);
    let to_y = |h: f32| plot.bottom() - (h - lo) / (hi - lo).max(1.0) * plot.height();
    let from_y = |y: f32| lo + (plot.bottom() - y) / plot.height().max(1.0) * (hi - lo);
    // Axis labels.
    for h in [min_y, 0, 100, 200, max_y - 1] {
        if h < min_y || h >= max_y {
            continue;
        }
        let y = to_y(h as f32);
        painter.text(pos2(rect.left() + gutter - 4.0, y), egui::Align2::RIGHT_CENTER, h.to_string(), egui::FontId::proportional(9.5), pal.weak);
        painter.hline(plot.x_range(), y, Stroke::new(0.5, pal.border));
    }
    // The landscape: valleys, hills, mountains.
    let profile = |t: f32| {
        let base = 70.0 + 30.0 * (t * 7.0).sin() + 25.0 * (t * 13.0 + 1.0).sin();
        let valley = -90.0 * (-((t - 0.3) * 7.0).powi(2)).exp();
        let mountains = 160.0 * (-((t - 0.78) * 4.5).powi(2)).exp();
        (base + valley + mountains).clamp(lo + 4.0, hi - 4.0)
    };
    let n = 96;
    let points: Vec<egui::Pos2> = (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32;
            pos2(plot.left() + t * plot.width(), to_y(profile(t)))
        })
        .collect();
    // Fill under the profile column by column (it is not convex).
    for w in points.windows(2) {
        let quad = vec![w[0], w[1], pos2(w[1].x, plot.bottom()), pos2(w[0].x, plot.bottom())];
        painter.add(egui::Shape::convex_polygon(quad, pal.weak.gamma_multiply(0.35), Stroke::NONE));
    }
    // The band.
    let band_lo = f.above.map_or(lo, |a| a as f32);
    let band_hi = f.below.map_or(hi, |b| b as f32);
    let shade = pal.accent.gamma_multiply(0.12);
    if f.above.is_some() && f.below.is_some() && f.beyond {
        painter.rect_filled(Rect::from_min_max(pos2(plot.left(), to_y(band_hi)), pos2(plot.right(), plot.top())), 0, shade);
        painter.rect_filled(Rect::from_min_max(pos2(plot.left(), plot.bottom()), pos2(plot.right(), to_y(band_lo))), 0, shade);
    } else if f.above.is_some() || f.below.is_some() {
        painter.rect_filled(Rect::from_min_max(pos2(plot.left(), to_y(band_hi)), pos2(plot.right(), to_y(band_lo))), 0, shade);
    }
    // The surface, in blue where the brush acts.
    for (i, w) in points.windows(2).enumerate() {
        let t = (i as f32 + 0.5) / n as f32;
        let weight = f.height_weight(profile(t));
        let color = if weight > 0.0 { pal.accent.gamma_multiply(0.3 + 0.7 * weight) } else { pal.text.gamma_multiply(0.5) };
        painter.line_segment([w[0], w[1]], Stroke::new(if weight > 0.0 { 2.5 } else { 1.2 }, color));
    }
    // The limits, dashed, with their values.
    let limit = |painter: &egui::Painter, h: i32, label: String| {
        let y = to_y(h as f32);
        painter.add(egui::Shape::dashed_line(&[pos2(plot.left(), y), pos2(plot.right(), y)], Stroke::new(1.2, pal.accent), 5.0, 3.0));
        let galley = painter.layout_no_wrap(label, egui::FontId::proportional(10.0), pal.accent);
        let at = pos2(plot.right() - galley.size().x - 4.0, (y - galley.size().y - 2.0).max(plot.top()));
        painter.rect_filled(Rect::from_min_size(at - vec2(2.0, 1.0), galley.size() + vec2(4.0, 2.0)), 2, pal.panel);
        painter.galley(at, galley, pal.accent);
    };
    if let Some(a) = f.above {
        limit(&painter, a, format!("≥ {a}"));
    }
    if let Some(b) = f.below {
        limit(&painter, b, format!("≤ {b}"));
    }
    // Dragging moves the nearest limit.
    let id = response.id.with("dragging");
    if response.drag_started()
        && let Some(p) = response.interact_pointer_pos()
    {
        let dist = |h: Option<i32>| h.map(|h| (to_y(h as f32) - p.y).abs());
        let which = match (dist(f.above), dist(f.below)) {
            (Some(a), Some(b)) => Some(if a <= b { 0u8 } else { 1 }),
            (Some(_), None) => Some(0),
            (None, Some(_)) => Some(1),
            (None, None) => None,
        };
        ui.data_mut(|d| d.insert_temp(id, which));
    }
    if response.dragged()
        && let Some(p) = response.interact_pointer_pos()
    {
        let which: Option<u8> = ui.data(|d| d.get_temp(id)).flatten();
        let h = from_y(p.y).round().clamp(lo, hi - 1.0) as i32;
        match which {
            Some(0) => f.above = Some(f.below.map_or(h, |b| h.min(b))),
            Some(1) => f.below = Some(f.above.map_or(h, |a| h.max(a))),
            _ => {}
        }
    }
    if response.hovered() && (f.above.is_some() || f.below.is_some()) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }
}

/// A list of terrains, biomes, water or land the filter matches, as chips,
/// with a menu to add more.
fn match_list(ui: &mut Ui, label: &str, list: &mut Option<Vec<massif_core::filter::Match>>, reg: &Registry, id: &str) {
    use massif_core::filter::Match;
    let pal = Palette::of(ui);
    let mut on = list.is_some();
    if ui.checkbox(&mut on, label).changed() {
        *list = on.then(Vec::new);
    }
    let Some(items) = list else { return };
    let name = |m: &Match| match *m {
        Match::Terrain(t) => reg.terrain(TerrainId(t)).name.clone(),
        Match::Biome(b) => reg.biome(BiomeId(b)).name.clone(),
        Match::Water => "Water".to_owned(),
        Match::Land => "Dry land".to_owned(),
    };
    ui.horizontal_wrapped(|ui| {
        ui.add_space(20.0);
        let mut remove = None;
        for (i, m) in items.iter().enumerate() {
            let text = format!("{}  {}", name(m), theme::glyph(Icon::X));
            if ui.add(egui::Button::new(RichText::new(text).size(11.5)).corner_radius(10)).on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            items.remove(i);
        }
        ui.menu_button(theme::with_icon(Icon::Plus, "Add"), |ui| {
            ui.set_max_height(360.0);
            let add = |m: Match, items: &mut Vec<Match>| {
                if !items.contains(&m) {
                    items.push(m);
                }
            };
            if ui.button("Water").clicked() {
                add(Match::Water, items);
            }
            if ui.button("Dry land").clicked() {
                add(Match::Land, items);
            }
            ui.menu_button("Terrains", |ui| {
                egui::ScrollArea::vertical().id_salt((id, "terrains")).max_height(320.0).show(ui, |ui| {
                    for (i, t) in reg.terrains.iter().enumerate() {
                        if ui.button(&t.name).clicked() {
                            add(Match::Terrain(i as u16), items);
                        }
                    }
                });
            });
            ui.menu_button("Biomes", |ui| {
                egui::ScrollArea::vertical().id_salt((id, "biomes")).max_height(320.0).show(ui, |ui| {
                    for (i, b) in reg.biomes.iter().enumerate().skip(1) {
                        if ui.button(&b.name).clicked() {
                            add(Match::Biome(i as u16), items);
                        }
                    }
                });
            });
        });
    });
    if items.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(20.0);
            ui.label(RichText::new("Add what the brush should act on").size(11.0).color(pal.weak));
        });
    }
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

/// The icon of a terrain: its main block with the game's own textures when
/// they are loaded, a drawn cube otherwise.
pub fn terrain_icon(ui: &mut Ui, rect: Rect, reg: &Registry, terrain: u16, blocks: &mut BlockLibrary) {
    let ctx = ui.ctx().clone();
    if let Some(cat) = blocks.catalogue.clone()
        && let Some(block) = massif_assets::terrain_block(reg, &cat, TerrainId(terrain))
        && let Some(texture) = blocks.icon(&ctx, block)
    {
        ui.painter().image(texture, rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
        return;
    }
    let (top, side) = terrain_colors(reg, terrain);
    cube_icon(ui.painter(), rect, top, side);
}

/// Every block of the game, with its real textures: click one to paint it.
pub fn blocks_panel(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, jar: &mut Option<std::path::PathBuf>) -> Option<String> {
    use massif_assets::Category;
    let pal = Palette::of(ui);
    let ctx = ui.ctx().clone();
    let mut feedback = None;

    // Where the blocks come from: a version of the game installed here.
    ui.horizontal(|ui| {
        let current = blocks.catalogue.as_ref().map(|c| (c.version.clone(), c.source.clone()));
        let label = match &current {
            Some((v, _)) => format!("Minecraft {v}"),
            None if blocks.is_loading() => "Reading the game…".to_owned(),
            None => "No game found".to_owned(),
        };
        crate::widgets::combo("minecraft version")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for j in blocks.jars.clone() {
                    let selected = current.as_ref().is_some_and(|(_, p)| *p == j.path);
                    if ui.selectable_label(selected, format!("Minecraft {}", j.version)).on_hover_text(j.path.display().to_string()).clicked() {
                        *jar = Some(j.path.clone());
                        blocks.load(&ctx, jar.clone());
                    }
                }
                if blocks.jars.is_empty() {
                    ui.label(RichText::new("No version found in the usual places").color(pal.weak));
                }
            });
        if ui
            .button(theme::with_icon(Icon::FolderOpen, "Choose…"))
            .on_hover_text("Another version's client jar")
            .clicked()
            && let Some(path) = rfd::FileDialog::new().set_title("Choose the Minecraft jar").add_filter("Minecraft", &["jar"]).pick_file()
        {
            *jar = Some(path);
            blocks.load(&ctx, jar.clone());
        }
        if blocks.is_loading() {
            ui.spinner();
        }
    });
    hint(
        ui,
        "Your game's real blocks. Click one to paint with it.",
    );
    if let Some(e) = &blocks.error {
        ui.label(RichText::new(e).color(pal.danger).size(11.5));
    }
    let Some(cat) = blocks.catalogue.clone() else { return feedback };

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut blocks.search)
                .hint_text(format!("{}  Search {} blocks", theme::glyph(Icon::Search), cat.blocks.len()))
                .desired_width((ui.available_width() - 110.0).max(80.0)),
        );
        ui.checkbox(&mut blocks.all, "All shapes")
            .on_hover_text("Also non-cube blocks");
    });
    let needle = blocks.search.to_lowercase();
    let reg = &doc.world.registry;
    let current = massif_assets::terrain_block(reg, &cat, TerrainId(tools.terrain)).map(|b| b.id.clone());
    let mut picked: Option<&massif_assets::Block> = None;
    for category in Category::ALL {
        let list: Vec<&massif_assets::Block> = cat
            .blocks
            .iter()
            .filter(|b| b.category == category && (blocks.all || b.full_cube))
            .filter(|b| needle.is_empty() || b.name.to_lowercase().contains(&needle) || b.id.contains(&needle))
            .collect();
        if list.is_empty() {
            continue;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{}  ·  {}", category.label().to_uppercase(), list.len())).size(10.5).bold().color(pal.weak));
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
            for block in list {
                let (rect, response) = ui.allocate_exact_size(vec2(36.0, 36.0), Sense::click());
                let selected = current.as_deref() == Some(block.id.as_str());
                let fill = if selected {
                    pal.accent_soft
                } else if response.hovered() {
                    pal.header
                } else {
                    Color32::TRANSPARENT
                };
                let stroke = if selected { Stroke::new(1.5, pal.accent) } else { Stroke::NONE };
                ui.painter().rect(rect, 6, fill, stroke, StrokeKind::Inside);
                if let Some(texture) = blocks.icon(&ctx, block) {
                    let tint = if block.full_cube { Color32::WHITE } else { Color32::from_white_alpha(150) };
                    ui.painter()
                        .image(texture, rect.shrink(4.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                }
                let tip = format!("{}\n{}{}", block.name, block.id, if block.full_cube { "" } else { "\nNot a whole block: it cannot be ground" });
                if response.on_hover_text(tip).clicked() {
                    picked = Some(block);
                }
            }
        });
    }
    if let Some(block) = picked {
        if block.full_cube {
            let terrain = massif_assets::block_terrain(&mut doc.world.registry, block);
            doc.mark_changed();
            tools.terrain = terrain.0;
            if tools.tool != Tool::PaintTerrain {
                tools.select(Tool::PaintTerrain);
            }
            feedback = Some(format!("Painting {}", block.name));
        } else {
            feedback = Some(format!("{} is not a whole block: it cannot be the ground", block.name));
        }
    }
    feedback
}

fn swatch(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(14.0, 14.0), Sense::hover());
    ui.painter().rect(r, 3, color, Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
}

/// Returns true when the solo setting changed.
pub fn terrain_panel(ui: &mut Ui, tools: &mut ToolState, reg: &Registry, solo: &mut bool, search: &mut String, blocks: &mut BlockLibrary) -> bool {
    let pal = Palette::of(ui);
    hint(ui, "The block on top of the ground.");
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
        ui.label(RichText::new(group.to_uppercase()).size(10.5).bold().color(pal.weak));
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
                terrain_icon(ui, rect.shrink(4.0), reg, i as u16, blocks);
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
    ui.label(RichText::new(&reg.terrain(TerrainId(tools.terrain)).name).bold());
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
    hint(ui, "Painted biomes override automatic.");
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
        "Solid adds rock, Carve removes it. Higher wins.",
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
                        let label = egui::Label::new(RichText::new(&name).bold().color(if visible { pal.text } else { pal.weak }))
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
    let (bottom, top) = (doc.dimension().min_y, doc.dimension().max_y);
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
        if ui.add(egui::DragValue::new(&mut meta.sea_level).range(bottom..=top - 1)).changed() {
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
    ui.label(RichText::new(format!("Column {x}, {z}")).bold());
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
        ui.label(RichText::new("What should this area show?").bold().size(15.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new(
                "Drag an area's title to move or swap it.",
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
