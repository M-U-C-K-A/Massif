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

pub(crate) fn section(ui: &mut Ui, icon: Icon, title: &str) {
    let pal = Palette::of(ui);
    ui.add_space(2.0);
    ui.label(
        RichText::new(format!("{}  {}", theme::glyph(icon), title.to_uppercase()))
            .size(10.5)
            .bold()
            .color(pal.weak),
    );
}

pub(crate) fn hint(ui: &mut Ui, text: &str) {
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
    painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        theme::glyph(icon),
        egui::FontId::proportional(size * 0.6),
        if selected { pal.accent } else { pal.text },
    );
    response
}

/// A tool's button: its own icon (see [`crate::icons`]) on a tile.
fn tool_button(ui: &mut Ui, tool: Tool, selected: bool, size: f32) -> egui::Response {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, crate::i18n::tr_name(tool.label())));
    let visuals = ui.style().interact_selectable(&response, selected);
    let fill = if selected {
        pal.accent_soft
    } else if response.hovered() {
        visuals.bg_fill
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
    let icon = Rect::from_center_size(rect.center(), vec2(size * 0.66, size * 0.66));
    crate::icons::paint(ui.painter(), icon, tool, if selected { pal.accent } else { pal.text });
    response
}

/// A tool to choose in a row of them: its icon and its name, in a pill.
fn tool_chip(ui: &mut Ui, tool: Tool, label: &str, selected: bool) -> egui::Response {
    let pal = Palette::of(ui);
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, if selected { pal.text } else { pal.weak });
    let (rect, response) = ui.allocate_exact_size(vec2(10.0 + 16.0 + 6.0 + galley.size().x + 12.0, 26.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let fill = if selected {
        pal.accent_soft
    } else if response.hovered() {
        theme::mix(pal.panel, pal.text, 0.07)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::PILL, fill);
    let icon = Rect::from_center_size(pos2(rect.left() + 18.0, rect.center().y), vec2(16.0, 16.0));
    crate::icons::paint(ui.painter(), icon, tool, if selected { pal.accent } else { pal.weak });
    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

/// A chip like [`tool_chip`] that switches something on and off, with one
/// of the font's icons.
fn toggle_chip(ui: &mut Ui, icon: Icon, label: &str, on: bool) -> egui::Response {
    let pal = Palette::of(ui);
    let font = egui::FontId::proportional(12.5);
    let galley = ui.painter().layout_no_wrap(label.to_owned(), font, pal.text);
    let (rect, response) = ui.allocate_exact_size(vec2(10.0 + 16.0 + 6.0 + galley.size().x + 12.0, 26.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, on, label));
    let fill = if on {
        pal.accent_soft
    } else if response.hovered() {
        theme::mix(pal.panel, pal.text, 0.07)
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(rect, theme::PILL, fill);
    ui.painter().text(
        pos2(rect.left() + 18.0, rect.center().y),
        egui::Align2::CENTER_CENTER,
        theme::glyph(icon),
        egui::FontId::proportional(14.0),
        if on { pal.accent } else { pal.weak },
    );
    ui.painter().galley(pos2(rect.left() + 32.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }
    response
}

// ------------------------------------------------------------------ tools --

pub fn tools_panel(ui: &mut Ui, tools: &mut ToolState) {
    let pal = Palette::of(ui);
    // A row a theme, set apart: its name, then its tools.
    ui.spacing_mut().item_spacing = vec2(0.0, 4.0);
    let fill = theme::mix(pal.panel, pal.text, if pal.dark { 0.035 } else { 0.028 });
    // In a narrow area the rows keep their tools on one line: their names
    // give way to their icons (the name shows under the pointer).
    let most = Tool::GROUPS.iter().map(|(_, _, list)| list.len()).max().unwrap_or(0) as f32;
    let compact = ui.available_width() - 12.0 < 58.0 + 4.0 + most * 32.0;
    let label_w = if compact { 20.0 } else { 58.0 };
    for (group, icon, list) in Tool::GROUPS {
        egui::Frame::new()
            .fill(fill)
            .corner_radius(theme::GROUP_RADIUS)
            .inner_margin(egui::Margin::symmetric(6, 3))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    let short = group.split(' ').next().unwrap_or(group);
                    let (rect, name) = ui.allocate_exact_size(vec2(label_w, 28.0), Sense::hover());
                    let text = if compact { theme::glyph(icon).to_string() } else { format!("{}  {short}", theme::glyph(icon)) };
                    ui.painter().text(
                        rect.left_center() + vec2(2.0, 0.0),
                        egui::Align2::LEFT_CENTER,
                        text,
                        egui::FontId::proportional(11.5),
                        pal.weak,
                    );
                    name.on_hover_text(group);
                    for &tool in list {
                        let button = tool_button(ui, tool, tools.tool == tool, 28.0);
                        if tools.untried(tool) {
                            // A new tool: a dot of the accent until it is tried.
                            let at = button.rect.right_top() + vec2(-4.5, 4.5);
                            ui.painter().circle(at, 3.0, pal.accent, Stroke::new(1.5, fill));
                        }
                        if crate::cards::tool(button, tool).clicked() {
                            tools.select(tool);
                        }
                    }
                });
            });
    }
}

// --------------------------------------------------------- tool settings --

/// The tools of the 3D Sculpt editor, along its top: which one, with which
/// brush — the one chosen for every tool, a custom one too —, how big, how
/// hard; with Paint, the terrain it paints.
pub fn sculpt_toolbar(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, reg: &Registry) {
    let pal = Palette::of(ui);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 0.0);
        ui.spacing_mut().interact_size.y = 22.0;
        for tool in crate::tools::SculptSettings::TOOLS {
            let label = if tool == Tool::PaintTerrain { tr!("Paint") } else { crate::i18n::tr_name(tool.label()) };
            if crate::cards::tool(tool_chip(ui, tool, label, tools.sculpt.tool == tool), tool).clicked() {
                tools.sculpt.tool = tool;
            }
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        // Walking in the view, as in Minecraft.
        let player = toggle_chip(ui, Icon::PersonStanding, tr!("Player"), tools.sculpt.player).on_hover_text(
            "See and walk the map as in Minecraft with Axiom: the game's blocks, light and sky. The cursor free, \
             the left button looks around, the right one uses the tool (Alt: its inverse), the middle one picks a block, \
             Ctrl turns about a point or moves the view, the wheel sets the flight speed. A click takes the mouse \
             as in the game (Esc gives it back): the right button builds, the left one breaks. W A S D walk, \
             Space jumps (twice: flies), Shift sneaks or goes down, Ctrl sprints",
        );
        if player.clicked() {
            tools.sculpt.player = !tools.sculpt.player;
            crate::sounds::play(crate::sounds::Cue::Toggle);
            // Space would press it again.
            player.surrender_focus();
        }
        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);
        // The brush: the one chosen for every tool, from any of the
        // library's groups, a custom image too.
        let name = library.name(&tools.brush_source);
        ui.menu_button(theme::with_icon(Icon::Brush, &name), |ui| {
            ui.set_max_width(330.0);
            let dark = ui.visuals().dark_mode;
            let ctx = ui.ctx().clone();
            let mut lists: Vec<(String, Vec<BrushSource>)> = vec![
                (tr!("Basic").into(), BrushSource::BASIC.to_vec()),
                (tr!("More shapes").into(), BrushSource::MORE.to_vec()),
                (tr!("Stamps").into(), Stamp::ALL.iter().map(|t| BrushSource::Stamp(*t)).collect()),
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
                            let response = brush_button(ui, texture, tools.brush_source == source, 40.0);
                            let response = if response.hovered() {
                                let shape = library.brush(&source, &tools.brush);
                                crate::cards::brush(response, &tip, &format!("{source:?}"), shape.as_ref())
                            } else {
                                response
                            };
                            if response.clicked() && select_brush(tools, library, source) {
                                ui.close();
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
            });
        });
        // Its size and intensity are those of Brush Settings, as for every
        // tool: said here, set there.
        ui.add_space(6.0);
        let b = tools.sculpt_brush();
        let most = massif_core::voxel::MAX_SCULPT_RADIUS;
        let capped = tools.brush.radius > most;
        let size = if capped { tr!("{most:.0} radius (the most in 3D)", most) } else { tr!("{radius:.0} radius", radius = b.radius) };
        ui.label(RichText::new(tr!("{size} · {value:.0}%", value = b.strength * 100.0, size)).size(12.0).color(if capped { pal.accent } else { pal.weak }))
            .on_hover_text(tr!("Size, intensity and rotation: in Brush Settings, as for every tool (⌘+wheel: size). In 3D a brush sculpts {most:.0} blocks around at most.", most));
        // What Paint paints: the terrain chosen (in Terrain or Blocks).
        if tools.sculpt.tool == Tool::PaintTerrain {
            ui.add_space(6.0);
            let def = reg.terrain(TerrainId(tools.terrain));
            swatch(ui, theme::rgb(def.color));
            let what = if tools.pattern.on { tr!("a noise of {n} blocks", n = tools.pattern.terrains.len()) } else { def.name.clone() };
            ui.label(RichText::new(what).size(12.0)).on_hover_text(tr!("Pick it in Terrain or Blocks"));
        }
        let hint = match tools.sculpt.tool {
            _ if tools.sculpt.player => tr!("The keys and buttons: along the bottom of the view"),
            Tool::PaintTerrain if tools.unpaint == crate::tools::Unpaint::MapTerrain => tr!("Left: paint · right: back to the map's terrain · Shift+drag: move · Alt+drag: turn · ⌘+wheel: size · Alt+wheel: fly to the point"),
            Tool::PaintTerrain => tr!("Left: paint · right: what the height calls for · Shift+drag: move · Alt+drag: turn · ⌘+wheel: size · Alt+wheel: fly to the point"),
            Tool::Smooth3d => tr!("Left: smooth · Shift+drag: move · Alt+drag: turn · ⌘+wheel: size · Alt+wheel: fly to the point"),
            _ => tr!("Left: add · right: dig · Shift+drag: move · Alt+drag: turn · ⌘+wheel: size · Alt+wheel: fly to the point"),
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(egui::Label::new(RichText::new(hint).size(11.5).color(pal.weak)).truncate());
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

/// The ways to paint terrains: One terrain or Noise (Paint terrain), or
/// Gradient (the Gradient brush, which shows the same choice).
pub(crate) fn paint_modes(ui: &mut Ui, tools: &mut ToolState) {
    let gradient = tools.tool == Tool::Gradient;
    let on = [!gradient && !tools.pattern.on, !gradient && tools.pattern.on, gradient];
    if let Some(k) = crate::widgets::button_group(ui, "paint what", &[tr!("One terrain"), tr!("Noise"), tr!("Gradient")], &on) {
        if k == 2 {
            tools.select(Tool::Gradient);
        } else {
            tools.select(Tool::PaintTerrain);
            tools.pattern.on = k == 1;
        }
    }
}

/// Paint terrain's right button: back to the map's own terrain (shown, and
/// chosen here), or to the one the height calls for.
fn unpaint_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    use crate::tools::Unpaint;
    section(ui, Icon::MouseRight, tr!("Right button"));
    let modes = [Unpaint::MapTerrain, Unpaint::ByHeight];
    if let Some(k) = crate::widgets::button_group(ui, "unpaint", &[tr!("The map's terrain"), tr!("By height")], &modes.map(|m| tools.unpaint == m)) {
        tools.unpaint = modes[k];
    }
    ui.add_space(4.0);
    if tools.unpaint == Unpaint::ByHeight {
        hint(ui, tr!("Right button: back to the terrain the height calls for, as a new map is painted: grass, sand by the water, rock and snow up high."));
        return;
    }
    let known = doc.map_terrain();
    let mut chosen = None;
    ui.horizontal(|ui| {
        match known {
            Some(t) => {
                let (r, _) = ui.allocate_exact_size(vec2(26.0, 26.0), Sense::hover());
                terrain_icon(ui, r, &doc.world.registry, t.0, blocks);
                ui.label(RichText::new(crate::i18n::tr_name(&doc.world.registry.terrain(t).name)).bold());
            }
            None => {
                ui.label(RichText::new(tr!("By height, for now")).color(Palette::of(ui).weak));
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            crate::widgets::combo("unpaint terrain").selected_text(tr!("Change")).height(320.0).show_ui(ui, |ui| {
                for (i, def) in doc.world.registry.terrains.iter().enumerate() {
                    let t = TerrainId(i as u16);
                    if ui.selectable_label(known == Some(t), crate::i18n::tr_name(&def.name)).clicked() {
                        chosen = Some(t);
                    }
                }
            })
            .response
            .on_hover_text(tr!("The terrain the map is made of, kept with it"));
        });
    });
    if let Some(t) = chosen {
        doc.world.meta.fill = Some(t);
        doc.mark_changed();
    }
    match known {
        Some(t) => hint(ui, &tr!("Right button: back to {name}, the map's own terrain.", name = crate::i18n::tr_name(&doc.world.registry.terrain(t).name))),
        None => hint(ui, tr!("This map's terrains come from its height: the right button puts back the one the height calls for, until you choose the map's terrain.")),
    }
}

pub fn tool_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    ui.spacing_mut().slider_width = (ui.available_width() - 76.0 - 84.0).clamp(60.0, 220.0);
    let tool = tools.tool;
    ui.horizontal(|ui| {
        crate::icons::show(ui, tool, 20.0, pal.accent);
        ui.label(RichText::new(crate::i18n::tr_name(tool.label())).bold().size(14.0));
    });
    hint(ui, tool.help());

    section(ui, Icon::SlidersHorizontal, tr!("Options"));
    match tool {
        Tool::Raise => {
            slider_row(ui, tr!("Speed"), |ui| {
                ui.add(crate::widgets::Field::new(&mut tools.raise_speed, 1.0..=120.0).logarithmic(true).suffix(" b/s"))
            });
        }
        Tool::Flatten | Tool::SetHeight => {
            slider_row(ui, tr!("Height"), |ui| {
                crate::widgets::drag(ui, egui::DragValue::new(&mut tools.target_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
            });
            hint(ui, tr!("Alt-click on the map to pick a height."));
            if tool == Tool::Flatten {
                use massif_core::ops::FlattenMode;
                ui.add_space(4.0);
                let modes = [FlattenMode::Both, FlattenMode::RaiseOnly, FlattenMode::LowerOnly];
                let on = modes.map(|m| tools.flatten_mode == m);
                if let Some(k) = crate::widgets::button_group(ui, "flatten mode", &[tr!("Both ways"), tr!("Only raise"), tr!("Only lower")], &on) {
                    tools.flatten_mode = modes[k];
                }
                ui.add_space(4.0);
                crate::widgets::checkbox(ui, &mut tools.flatten_theme, tr!("Apply theme"));
                hint(ui, tr!("Repaints the land it flattens with the terrain its new height calls for."));
            }
        }
        Tool::Road => crate::road_draft::settings(ui, tools, doc),
        Tool::Roughen => {
            slider_row(ui, tr!("Amplitude"), |ui| ui.add(crate::widgets::Field::new(&mut tools.roughness, 0.5..=40.0).suffix(tr!(" blocks"))));
        }
        Tool::PaintTerrain => {
            paint_modes(ui, tools);
            ui.add_space(4.0);
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
                    tr!("Pick it in Terrain or Blocks; Alt-click picks it on the map."),
                );
            }
            unpaint_settings(ui, tools, doc, blocks);
        }
        Tool::Gradient => crate::gradient_ui::tool_settings(ui, tools, doc),
        Tool::Cover => cover_settings(ui, tools, doc, blocks),
        Tool::Objects => object_settings(ui, tools, doc, blocks),
        Tool::Annotate => annotation_settings(ui, tools),
        Tool::TreeSize => tree_size_settings(ui, tools),
        Tool::Pockets => pocket_settings(ui, tools, doc, blocks),
        Tool::Combined => combined_settings(ui, tools, doc),
        Tool::PaintBiome => {
            let reg = &doc.world.registry;
            let def = reg.biome(BiomeId(tools.biome));
            ui.horizontal(|ui| {
                swatch(ui, theme::rgb(def.color));
                ui.label(RichText::new(&def.name).bold());
            });
            hint(ui, tr!("Pick it in Biomes."));
        }
        Tool::AutoBiome | Tool::AutoPaint => {
            let biomes = tool == Tool::AutoBiome;
            hint(ui, tr!("Hover: what changes shows in its new colour, what already is as the land calls for is hatched · drag: paint it."));
            if biomes {
                hint(ui, tr!("Biomes show on the map in its Biome mode."));
            }
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                if ui
                    .button(theme::with_icon(Icon::Map, tr!("Whole map")))
                    .on_hover_text(if biomes {
                        tr!("Set the biomes of the whole map (undoable)")
                    } else {
                        tr!("Paint the whole map (undoable)")
                    })
                    .clicked()
                {
                    tools.run_auto = Some(biomes);
                }
                if ui
                    .button(theme::with_icon(Icon::Settings2, tr!("Rules…")))
                    .on_hover_text(tr!("The biome of each kind of land, the terrains of each biome"))
                    .clicked()
                {
                    tools.open_rules = true;
                }
            });
            if let Some((_, changed)) = tools.auto_done.filter(|(b, _)| *b == biomes) {
                hint(ui, &tr!("{changed} columns changed over the whole map.", changed));
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
            changed |= tip_row(ui, tr!("Width"), tr!("Width of the water at the source"), |ui| ui.add(crate::widgets::Field::new(&mut r.width, 2.0..=200.0).suffix(tr!(" blocks")).max_decimals(0)));
            changed |= tip_row(ui, tr!("Flow"), tr!("How much wider and deeper it grows downstream"), |ui| ui.add(percent(&mut r.flow)));
            changed |= tip_row(ui, tr!("Depth"), tr!("Depth of the water at the source"), |ui| ui.add(crate::widgets::Field::new(&mut r.depth, 1.0..=30.0).suffix(tr!(" blocks")).max_decimals(0)));
            changed |= tip_row(ui, tr!("Erosion"), tr!("How much it wears the land beside its bed"), |ui| ui.add(percent(&mut r.erosion)));
            changed |= tip_row(ui, tr!("Banks"), tr!("How far its banks slope back up to the land"), |ui| ui.add(crate::widgets::Field::new(&mut r.banks, 0.0..=120.0).suffix(tr!(" blocks")).max_decimals(0)));
            changed |= tip_row(ui, tr!("Lakes"), tr!("Which hollows on its way fill up as lakes: none, the deep ones, all"), |ui| ui.add(percent(&mut r.lakes)));
            crate::widgets::checkbox(ui, &mut r.water, tr!("Fill it with water"));
            crate::widgets::checkbox(ui, &mut r.biome, tr!("River biome on its water"));
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Bed")));
                let shown = if r.bed.is_empty() { tr!("As it is").to_owned() } else { r.bed.clone() };
                crate::widgets::combo("river bed").selected_text(shown).width(150.0).show_ui(ui, |ui| {
                    if ui.selectable_label(r.bed.is_empty(), tr!("As it is")).clicked() {
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
                &tr!("{width:.0} × {depth:.0} at the source, {width_end:.0} × {depth_end:.0} at the end (width × depth).", width = spec.width, depth = spec.depth, width_end = spec.width_end, depth_end = spec.depth_end),
            );
            ui.add_space(4.0);
            if changed && let Some(draft) = &mut tools.river_draft {
                draft.plan(doc, &tools.river, None);
            }
            match &tools.river_draft {
                None => hint(ui, tr!("Click on land: where it springs. The water finds its way down.")),
                Some(draft) => {
                    ui.label(RichText::new(draft.summary()).strong());
                    hint(ui, tr!("Drag the river to guide it · click elsewhere: where it ends · right-click a point: remove it."));
                    ui.horizontal(|ui| {
                        if ui.button(theme::with_icon(Icon::Route, tr!("Dig river"))).on_hover_text(tr!("Enter")).clicked() {
                            tools.river_note = crate::tools::dig_draft(doc, tools);
                        }
                        if ui.button(tr!("Cancel")).on_hover_text(tr!("Esc")).clicked() {
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
            let (lo, hi) = (doc.dimension().min_y as i16, doc.dimension().max_y as i16);
            let sea = doc.world.meta.sea_level as i16;
            crate::widgets::checkbox(ui, &mut tools.water_from_click, tr!("From the click"));
            if tools.water_from_click {
                hint(
                    ui,
                    if tool == Tool::Flood {
                        tr!("Fills the hollow you click up to where it spills, and sets the level below to it.")
                    } else {
                        tr!("Lays water as high as where each stroke starts: the water's surface there, or a block over the ground.")
                    },
                );
            }
            ui.add_space(4.0);
            // The level set, for when the click does not give it.
            ui.add_enabled_ui(!tools.water_from_click, |ui| {
                slider_row(ui, tr!("Water level"), |ui| crate::widgets::drag(ui, egui::DragValue::new(&mut tools.water_level).range(lo..=hi)));
                ui.horizontal(|ui| {
                    ui.add_space(76.0);
                    if ui.small_button(tr!("Sea level ({sea_level})", sea_level = sea)).clicked() {
                        tools.water_level = sea;
                    }
                });
            });
            ui.add_space(4.0);
            crate::widgets::checkbox(ui, &mut tools.water_limit_on, tr!("Limit the water's height"));
            if tools.water_limit_on {
                slider_row(ui, tr!("Highest"), |ui| crate::widgets::drag(ui, egui::DragValue::new(&mut tools.water_limit).range(lo..=hi)));
                ui.horizontal(|ui| {
                    ui.add_space(76.0);
                    if ui.small_button(tr!("Sea level ({sea_level})", sea_level = sea)).clicked() {
                        tools.water_limit = sea;
                    }
                });
                hint(ui, tr!("The water tools never lay water above this height."));
            }
        }
        Tool::StampSlab => {
            slider_row(ui, tr!("Thickness"), |ui| {
                ui.add(crate::widgets::Field::new(&mut tools.slab_thickness, 1.0..=128.0).logarithmic(true).suffix(tr!(" blocks")))
            });
            ui.horizontal(|ui| {
                ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Centre")));
                ui.selectable_value(&mut tools.anchor, AnchorMode::AboveGround, tr!("Above ground"));
                ui.selectable_value(&mut tools.anchor, AnchorMode::Absolute, tr!("At height"));
            });
            match tools.anchor {
                AnchorMode::AboveGround => {
                    slider_row(ui, tr!("Offset"), |ui| {
                        crate::widgets::drag(ui, egui::DragValue::new(&mut tools.slab_offset)
                                .speed(0.5)
                                .suffix(tr!(" blocks")),
                        )
                    });
                    hint(ui, tr!("Negative: underground."));
                }
                AnchorMode::Absolute => {
                    slider_row(ui, tr!("Height"), |ui| {
                        crate::widgets::drag(ui, egui::DragValue::new(&mut tools.slab_height).speed(0.5).range(doc.dimension().min_y..=doc.dimension().max_y))
                    });
                }
            }
        }
        Tool::Build | Tool::Dig | Tool::Smooth3d => {
            hint(ui, tr!("Best in the 3D view."));
            let sculpted = doc.dimension().voxels.len();
            if sculpted > 0 {
                hint(ui, &tr!("{sculpted} sculpted chunks of 16³ blocks in this dimension.", sculpted));
            }
        }
        Tool::SelectRect | Tool::SelectEllipse | Tool::SelectLasso | Tool::SelectBrush => crate::select::settings(ui, tools, doc, blocks),
        Tool::Eyedropper | Tool::Smooth | Tool::Drain | Tool::EraseSlab | Tool::Pan => {}
    }

    // Which level the tool edits.
    if !matches!(tool, Tool::Pan | Tool::Flood | Tool::Drain | Tool::Eyedropper) && !tool.is_sculpt() && !tool.is_select() {
        section(ui, Icon::Layers, tr!("Level"));
        let levels: Vec<(String, LevelKind)> = doc.dimension().levels.iter().map(|l| (crate::i18n::tr_name(&l.name).to_owned(), l.kind)).collect();
        ui.horizontal(|ui| {
            ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Edit")));
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
                ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Surface")));
                ui.selectable_value(&mut tools.surface, SurfaceChoice::Top, tr!("Top"));
                ui.selectable_value(&mut tools.surface, SurfaceChoice::Bottom, tr!("Bottom"));
            });
        }
        if tool.needs_slab_level() && doc.active_kind() == LevelKind::Ground {
            ui.label(RichText::new(tr!("This tool edits a Solid or Carve level.")).color(pal.danger).size(12.0));
            ui.horizontal(|ui| {
                if ui.button(tr!("{glyph} Add solid level", glyph = theme::glyph(Icon::Plus))).clicked() {
                    doc.add_level(LevelKind::Solid);
                }
                if ui.button(tr!("{glyph} Add carve level", glyph = theme::glyph(Icon::Plus))).clicked() {
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
    painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.5, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
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
            let response = brush_button(ui, texture, selected, 44.0);
            let response = if response.hovered() {
                let shape = library.brush(source, &tools.brush);
                crate::cards::brush(response, name, &format!("{source:?}"), shape.as_ref())
            } else {
                response
            };
            if response.clicked() {
                select_brush(tools, library, source.clone());
            }
        }
        if let Some(add) = add {
            let r = icon_button(ui, Icon::Plus, false, 44.0)
                .on_hover_text(tr!("Add greyscale images as brushes"));
            *add |= r.clicked();
        }
    });
}

/// Ask for images and copy them into a group (Massif's own folder when
/// `group` is `None`).
fn add_brushes(library: &mut BrushLibrary, group: Option<&std::path::Path>, ctx: &egui::Context) -> Option<String> {
    let files = rfd::FileDialog::new()
        .set_title(tr!("Add brushes"))
        .add_filter(tr!("Images"), &["png", "jpg", "jpeg"])
        .pick_files()?;
    Some(match library.add_images(group, &files) {
        Ok(n) => {
            library.load_in_background(ctx);
            trn!("Added {n} brush", "Added {n} brushes", n = n)
        }
        Err(e) => tr!("Could not add the brushes: {e}", e),
    })
}

/// The built-in brushes, the terrain stamps and the commands for your own.
pub fn brushes_panel(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, new_group: &mut Option<String>) -> Option<String> {
    let mut feedback = None;
    let named = |library: &BrushLibrary, list: &[BrushSource]| -> Vec<(BrushSource, String)> {
        list.iter().map(|s| (s.clone(), library.name(s))).collect()
    };
    section(ui, Icon::Circle, tr!("Basic"));
    let basic = named(library, &BrushSource::BASIC);
    brush_grid(ui, tools, library, &basic, None);
    section(ui, Icon::Shapes, tr!("More shapes"));
    let more = named(library, &BrushSource::MORE);
    brush_grid(ui, tools, library, &more, None);
    for family in Family::ALL {
        section(ui, Icon::MountainSnow, crate::i18n::tr_name(family.label()));
        let stamps: Vec<(BrushSource, String)> = Stamp::ALL
            .iter()
            .filter(|s| s.family() == family)
            .map(|s| (BrushSource::Stamp(*s), crate::i18n::tr_name(s.label()).to_owned()))
            .collect();
        brush_grid(ui, tools, library, &stamps, None);
    }

    section(ui, Icon::Images, tr!("Your brushes"));
    ui.horizontal_wrapped(|ui| {
        if ui.button(theme::with_icon(Icon::Plus, tr!("Add brushes…"))).clicked() {
            feedback = add_brushes(library, None, ui.ctx());
        }
        if ui.button(theme::with_icon(Icon::FolderPlus, tr!("New group…"))).clicked() {
            *new_group = Some(String::new());
        }
        if ui.button(theme::with_icon(Icon::FolderOpen, tr!("Open the folder"))).clicked()
            && let Err(e) = library.open_folder()
        {
            feedback = Some(tr!("Could not open the brushes folder: {e}", e));
        }
        if ui
            .add_enabled(!library.is_loading(), egui::Button::new(theme::with_icon(Icon::RefreshCw, tr!("Reload"))))
            .clicked()
        {
            library.load_in_background(ui.ctx());
        }
    });
    if let Some(name) = new_group {
        let mut done = None;
        ui.horizontal(|ui| {
            let edit = ui.add(egui::TextEdit::singleline(name).hint_text(tr!("Group name")).desired_width(140.0));
            if !edit.has_focus() && name.is_empty() {
                edit.request_focus();
            }
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if ui.button(tr!("Create")).clicked() || enter {
                done = Some(true);
            }
            if ui.button(tr!("Cancel")).clicked() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                done = Some(false);
            }
        });
        match done {
            Some(true) => {
                feedback = Some(match library.new_group(name) {
                    Ok(_) => {
                        library.load_in_background(ui.ctx());
                        tr!("Made the group “{trim}”", trim = name.trim())
                    }
                    Err(e) => tr!("Could not make the group: {e}", e),
                });
                *new_group = None;
            }
            Some(false) => *new_group = None,
            None => {}
        }
    }
    hint(
        ui,
        tr!("Greyscale images: white paints, black doesn't."),
    );
    feedback
}

/// The groups of brushes made from images.
pub fn custom_brushes_panel(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary) -> Option<String> {
    let mut feedback = None;
    if !library.is_loaded() {
        ui.horizontal(|ui| {
            crate::loader::loader(ui, 16.0);
            hint(ui, tr!("Loading the brushes…"));
        });
        return None;
    }
    if library.groups.is_empty() {
        hint(ui, tr!("No custom brushes yet."));
        if ui.button(theme::with_icon(Icon::Plus, tr!("Add brushes…"))).clicked() {
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
        section(ui, if writable { Icon::Folder } else { Icon::FolderOpen }, &tr!("{name}  ·  {n}", n = entries.len(), name));
        let mut add = false;
        brush_grid(ui, tools, library, &entries, writable.then_some(&mut add));
        if add {
            feedback = add_brushes(library, Some(&dir), ui.ctx());
        }
    }
    if !library.errors.is_empty() {
        let pal = Palette::of(ui);
        ui.label(RichText::new(tr!("{n} image(s) could not be read", n = library.errors.len())).color(pal.danger).size(11.5))
            .on_hover_text(library.errors.join("\n"));
    }
    feedback
}

/// Size, intensity and rotation of the brush, with a preview, then the
/// options that limit where it acts (WorldPainter's brush options).
#[allow(clippy::too_many_arguments)]
pub fn brush_settings(ui: &mut Ui, tools: &mut ToolState, library: &mut BrushLibrary, reg: &Registry, min_y: i32, max_y: i32, light: f32, heights: Option<&crate::doc::Heights>, sea: i32) {
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
                BrushSource::Symmetric(..) => tr!("Built-in brush"),
                BrushSource::Stamp(_) => tr!("Terrain stamp"),
                BrushSource::Image(_) => tr!("Custom brush"),
            };
            hint(ui, kind);
            let r = tools.brush.radius.round();
            hint(ui, &tr!("{value} blocks across", value = 2.0 * r + 1.0));
        });
    });
    ui.add_space(4.0);
    let b = &mut tools.brush;
    slider_row(ui, tr!("Size"), |ui| {
        ui.add(crate::widgets::Field::new(&mut b.radius, 1.0..=MAX_RADIUS).logarithmic(true).suffix(tr!(" radius")).max_decimals(0))
    });
    slider_row(ui, tr!("Intensity"), |ui| {
        ui.add(
            crate::widgets::Field::new(&mut b.strength, 0.01..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    let mut degrees = if b.rotation > 180.0 { b.rotation - 360.0 } else { b.rotation };
    let turned = slider_row(ui, tr!("Rotation"), |ui| {
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
            tr!("Scroll: size · 1…0: intensity · ⌘+scroll: turn")
        } else {
            tr!("Scroll: size · 1…0: intensity · Ctrl+Shift+scroll: turn")
        },
    );
    crate::widgets::checkbox(ui, &mut tools.show_strength, tr!("Strength on the map"))
        .on_hover_text(tr!("Under the pointer, lighter where the brush presses harder. Its lines at a quarter, a half and three quarters of its strength show either way."));

    // ---- options ---------------------------------------------------------
    let f = &mut tools.filter;
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label(RichText::new(tr!("Options")).bold());
        if f.is_active() {
            ui.label(RichText::new(theme::glyph(Icon::Funnel).to_string()).color(pal.accent))
                .on_hover_text(tr!("A filter is on: the brushes only act where it lets them"));
            ui.with_layout(egui::Layout::right_to_left(Align::Center), |ui| {
                if ui.small_button(tr!("Reset")).on_hover_text(tr!("Act everywhere again")).clicked() {
                    *f = massif_core::filter::Filter::default();
                }
            });
        }
    });

    section(ui, Icon::ArrowUpDown, tr!("Height limits"));
    height_limits(ui, f, heights, min_y, max_y, sea);

    section(ui, Icon::Layers, tr!("What is on the map"));
    match_list(ui, "only on", &mut f.only_on, reg, "only on");
    match_list(ui, "except on", &mut f.except_on, reg, "except on");

    section(ui, Icon::TrendingUp, tr!("Slope"));
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut f.slope_above, tr!("above"));
        crate::widgets::checkbox(ui, &mut f.slope_below, tr!("below"));
        crate::widgets::drag_enabled(ui, f.slope_above || f.slope_below, egui::DragValue::new(&mut f.slope).range(0.0..=90.0).speed(0.5).suffix("°"),
        );
    });

    section(ui, Icon::Compass, tr!("Facing"));
    ui.horizontal(|ui| {
        facing_dial(ui, f);
        ui.vertical(|ui| {
            crate::widgets::checkbox(ui, &mut f.facing_on, tr!("only slopes facing"))
                .on_hover_text(tr!("Only slopes facing this way."));
            ui.horizontal(|ui| {
                crate::widgets::drag_enabled(ui, f.facing_on, egui::DragValue::new(&mut f.facing)
                        .range(0.0..=360.0)
                        .speed(1.0)
                        .max_decimals(0)
                        .suffix("°")
                        .custom_formatter(|v, _| format!("{} {:.0}", compass(v as f32), v)),
                );
                ui.label("±");
                crate::widgets::drag_enabled(ui, f.facing_on, egui::DragValue::new(&mut f.facing_spread).range(5.0..=180.0).speed(1.0).max_decimals(0).suffix("°"));
            });
            ui.horizontal(|ui| {
                if ui
                    .small_button(theme::with_icon(Icon::Sun, tr!("Sun")))
                    .on_hover_text(tr!("The slopes the light of the map falls on (R and L turn it)"))
                    .clicked()
                {
                    f.facing_on = true;
                    f.facing = light.rem_euclid(360.0);
                }
                if ui
                    .small_button(theme::with_icon(Icon::Moon, tr!("Shade")))
                    .on_hover_text(tr!("The slopes turned away from the light of the map"))
                    .clicked()
                {
                    f.facing_on = true;
                    f.facing = (light + 180.0).rem_euclid(360.0);
                }
            });
        });
    });
    f.facing = f.facing.rem_euclid(360.0);

    section(ui, Icon::Waves, tr!("Curvature"));
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut f.hollows, tr!("hollows")).on_hover_text(tr!("Columns lower than the ones around them"));
        crate::widgets::checkbox(ui, &mut f.bumps, tr!("bumps")).on_hover_text(tr!("Columns higher than the ones around them"));
    });
    let on = f.hollows || f.bumps;
    ui.horizontal(|ui| {
        crate::widgets::drag_enabled(ui, on, egui::DragValue::new(&mut f.depth).range(0.1..=64.0).speed(0.1).max_decimals(1));
        ui.label(tr!("blocks deep"));
        crate::widgets::drag_enabled(ui, on, egui::DragValue::new(&mut f.radius).range(1..=64));
        ui.label(tr!("blocks around"));
    });
}

/// The pattern of the Paint tool: its shape, its terrains and their
/// weights, and a picture of it.
/// Whether a file holds an object Massif reads (by its extension).
pub fn is_object_file(path: &std::path::Path) -> bool {
    path.extension().and_then(|e| e.to_str()).is_some_and(|e| massif_objects::EXTENSIONS.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Read objects from files into the world: into layer `into`, or a new
/// layer named after them, which becomes the one painted. The game's
/// colours are given to their blocks when its blocks are loaded. Returns
/// what was read, to say.
pub fn import_objects(doc: &mut Document, tools: &mut ToolState, files: &[std::path::PathBuf], into: Option<u16>, catalogue: Option<&massif_assets::Catalogue>) -> String {
    let reg = &mut doc.world.registry;
    let mut added = Vec::new();
    let mut failed = Vec::new();
    let mut formats: Vec<&str> = Vec::new();
    let mut left_out = 0;
    for path in files {
        match massif_objects::read(path) {
            Ok(mut imported) => {
                let o = &mut imported.object;
                o.colors = o
                    .palette
                    .iter()
                    .map(|state| match catalogue.and_then(|c| c.block(state)) {
                        // Trunks show their bark; the rest, its top.
                        Some(b) if state.contains("log") || state.contains("wood") || state.contains("stem") => b.side_color,
                        Some(b) => b.color,
                        None => massif_core::objects::guess_color(state),
                    })
                    .collect();
                if !formats.contains(&imported.format) {
                    formats.push(imported.format);
                }
                left_out += imported.left_out;
                reg.objects.push(imported.object);
                added.push((reg.objects.len() - 1) as u32);
            }
            Err(e) => failed.push(format!("{}: {e}", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())),
        }
    }
    if !added.is_empty() {
        match into.and_then(|n| n.checked_sub(1)).and_then(|k| reg.object_layers.get_mut(k as usize)).filter(|l| !l.removed) {
            Some(layer) => layer.objects.extend(added.iter().map(|&k| (k, 1))),
            None => {
                let name = if added.len() == 1 { reg.objects[added[0] as usize].name.clone() } else { tr!("{name} and {value} more", name = reg.objects[added[0] as usize].name, value = added.len() - 1) };
                let layer = massif_core::objects::ObjectLayerDef::new(&name, &added, reg);
                tools.object_layer = reg.add_object_layer(layer);
            }
        }
        doc.mark_changed();
        if tools.tool != Tool::Objects {
            tools.select(Tool::Objects);
        }
    }
    let mut note = match added.len() {
        0 => String::new(),
        1 => tr!("Read 1 object ({join})", join = formats.join(", ")),
        n => tr!("Read {n} objects ({join})", join = formats.join(", "), n),
    };
    if left_out > 0 {
        note.push_str(&tr!(" · {left_out} blocks of unknown numbers left out", left_out));
    }
    if !failed.is_empty() {
        if !note.is_empty() {
            note.push_str(" · ");
        }
        note.push_str(&tr!("Could not read {join}", join = failed.join("; ")));
    }
    tools.objects_note = Some(note.clone());
    note
}

/// A picture of an object: its blocks seen from a corner, the nearest
/// drawn last.
pub fn object_picture(def: &massif_core::objects::ObjectDef) -> egui::ColorImage {
    let [w, h, l] = def.size.map(|s| s as usize);
    // Very large objects: seen from above.
    if w * h * l > 1 << 21 {
        let tops = def.top_view();
        let pixels = tops.iter().map(|&(k, y)| if k == 0 { Color32::TRANSPARENT } else { theme::scale(theme::rgb(def.color(k)), 0.7 + 0.3 * y as f32 / h.max(1) as f32) }).collect();
        return egui::ColorImage::new([w, l], pixels);
    }
    let (iw, ih) = ((w + l) * 2 + 2, w + l + 2 * h + 2);
    let mut pixels = vec![Color32::TRANSPARENT; iw * ih];
    let mut put = |x: usize, y: usize, c: Color32| {
        if x < iw && y < ih {
            pixels[y * iw + x] = c;
        }
    };
    for d in 0..w + l {
        for y in 0..h {
            for x in d.saturating_sub(l - 1)..=d.min(w - 1) {
                let z = d - x;
                let k = def.at(x, y, z);
                if k == 0 {
                    continue;
                }
                let c = theme::rgb(def.color(k));
                let sx = (x + l - 1 - z) * 2;
                let sy = x + z + 2 * (h - 1 - y);
                // Top lit, the sides darker.
                for (dx, dy, f) in [(0, 0, 1.12), (1, 0, 1.12), (2, 0, 1.12), (3, 0, 1.12), (0, 1, 1.12), (1, 1, 1.12), (2, 1, 1.12), (3, 1, 1.12), (0, 2, 0.82), (1, 2, 0.82), (2, 2, 0.64), (3, 2, 0.64), (0, 3, 0.82), (1, 3, 0.82), (2, 3, 0.64), (3, 3, 0.64)] {
                    put(sx + dx, sy + dy, theme::scale(c, f));
                }
            }
        }
    }
    egui::ColorImage::new([iw, ih], pixels)
}

/// The picture of object `k`, made once.
fn object_texture(ui: &Ui, k: usize, def: &massif_core::objects::ObjectDef) -> egui::TextureHandle {
    let id = egui::Id::unique(("object picture", k));
    let key = (def.name.clone(), def.size, def.blocks.len(), def.colors.len());
    if let Some((cached, t)) = ui.ctx().data(|d| d.get_temp::<((String, [u16; 3], usize, usize), egui::TextureHandle)>(id))
        && cached == key
    {
        return t;
    }
    let t = ui.ctx().load_texture(format!("object {k}"), object_picture(def), egui::TextureOptions::NEAREST);
    ui.ctx().data_mut(|d| d.insert_temp(id, (key, t.clone())));
    t
}

/// The object layers of the world, and the one painted: its objects and
/// how they stand.
fn object_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let before = (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len());
    let pick = |title: &str| {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(tr!("Schematics"), &massif_objects::EXTENSIONS)
            .pick_files()
            .unwrap_or_default()
    };
    if ui
        .button(theme::with_icon(Icon::FolderOpen, tr!("Import objects…")))
        .on_hover_text(tr!("Sponge .schem, MCEdit .schematic, Axiom .bp, Litematica .litematic, structure .nbt"))
        .clicked()
    {
        let files = pick(tr!("Import objects"));
        if !files.is_empty() {
            import_objects(doc, tools, &files, None, blocks.catalogue.as_deref());
        }
    }
    hint(ui, tr!("Trees, rocks, ruins from .schem, .schematic, .bp, .litematic or .nbt files — or drop them on the window. Every layer of the map: the Layers panel."));
    if let Some(note) = &tools.objects_note {
        ui.label(RichText::new(note).size(11.5).color(if note.contains("Could not") { pal.danger } else { pal.weak }));
    }

    // The layers to choose from.
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, layer) in doc.world.registry.object_layers.iter().enumerate() {
        if layer.removed {
            continue;
        }
        let n = k as u16 + 1;
        let selected = tools.object_layer == n;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let fill = if selected {
            pal.accent_soft
        } else if response.hovered() {
            pal.header
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
        let dot = Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(11.0, 11.0));
        painter.rect(dot, 3, theme::rgb(layer.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
        painter.text(rect.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, &layer.name, egui::FontId::proportional(12.5), pal.text);
        let count = layer.objects.len();
        painter.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, trn!("{n} object", "{n} objects", n = count), egui::FontId::proportional(11.0), pal.weak);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &layer.name));
        if response.clicked() {
            picked = Some(n);
        }
    }
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(n) = picked {
        tools.object_layer = n;
    }

    object_layer_editor(ui, tools, doc, blocks);
    if (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len()) != before {
        doc.mark_changed();
    }
}

/// The settings of the object layer picked (`tools.object_layer`), as Tool
/// Settings and the layer's own dialog show them.
pub fn object_layer_editor(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let before = (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len());
    let pick = |title: &str| {
        rfd::FileDialog::new()
            .set_title(title)
            .add_filter(tr!("Schematics"), &massif_objects::EXTENSIONS)
            .pick_files()
            .unwrap_or_default()
    };
    // The selected layer.
    let k = tools.object_layer.checked_sub(1).map(|k| k as usize);
    if k.and_then(|k| doc.world.registry.object_layers.get(k)).is_none_or(|l| l.removed) {
        if doc.world.registry.object_layers.iter().all(|l| l.removed) {
            hint(ui, tr!("No objects yet: import some to paint them."));
        }
        return;
    }
    let k = k.expect("checked above");
    let reg = &mut doc.world.registry;
    let classes = reg.object_layers[k].size_classes(reg);
    let objects = std::mem::take(&mut reg.objects);
    let layer = &mut reg.object_layers[k];
    ui.add_space(4.0);
    section(ui, Icon::TreePine, &layer.name.clone());
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Name")));
        ui.add(egui::TextEdit::singleline(&mut layer.name).desired_width((ui.available_width() - 40.0).max(60.0)));
        let mut c = Color32::from_rgb(layer.color[0], layer.color[1], layer.color[2]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour in the list")).changed() {
            layer.color = [c.r(), c.g(), c.b()];
        }
    });
    tip_row(ui, tr!("Spacing"), tr!("One object at most in each square of this side"), |ui| {
        ui.add(crate::widgets::Field::new(&mut layer.spacing, 2..=256).suffix(tr!(" blocks")).max_decimals(0))
    });
    tip_row(ui, tr!("Density"), tr!("Share of those squares where one stands"), |ui| {
        ui.add(
            crate::widgets::Field::new(&mut layer.density, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    tip_row(ui, tr!("Sink"), tr!("How many blocks into the ground they go (rocks bedded in)"), |ui| {
        ui.add(crate::widgets::Field::new(&mut layer.sink, -16..=32).suffix(tr!(" blocks")).max_decimals(0))
    });
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut layer.rotate, tr!("Turn at random")).on_hover_text(tr!("A quarter turn picked for each"));
        crate::widgets::checkbox(ui, &mut layer.mirror, tr!("Mirror")).on_hover_text(tr!("Half of them mirrored"));
    });
    ui.horizontal(|ui| {
        crate::widgets::checkbox(ui, &mut layer.soil_only, tr!("Only on soil")).on_hover_text(tr!("Grass, dirt, podzol, moss, mud…"));
        crate::widgets::checkbox(ui, &mut layer.underwater, tr!("Under water")).on_hover_text(tr!("Coral, wrecks: in the water rather than the air"));
    });
    crate::widgets::checkbox(ui, &mut layer.replace, tr!("Replace the ground")).on_hover_text(tr!("Their blocks take the place of the ground they reach; otherwise only of air, water and plants"));
    // Base or sub-canopy.
    let mut role = None;
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Canopy")));
        if let Some(c) = crate::widgets::button_group(ui, "canopy", &[tr!("Base"), tr!("Sub-canopy")], &[!layer.understory, layer.understory])
            && (c == 1) != layer.understory
        {
            role = Some(c == 1);
        }
    });
    hint(
        ui,
        if layer.understory {
            tr!("Grows under the canopy of the base layer painted on the same blocks (hazel, holly, bushes), in the air its trees leave. A block has one sub-canopy layer: painting another takes its place.")
        } else {
            tr!("A block has one base tree layer: painting another over it takes its place. A sub-canopy layer shares its blocks.")
        },
    );

    section(ui, Icon::Boxes, &tr!("Objects · {n}", n = layer.objects.len()));
    let total: f32 = layer.objects.iter().map(|o| o.1 as f32).sum::<f32>().max(1.0);
    let mut remove = None;
    for (row, (index, weight)) in layer.objects.iter_mut().enumerate() {
        let Some(def) = objects.get(*index as usize) else { continue };
        ui.horizontal(|ui| {
            let texture = object_texture(ui, *index as usize, def);
            let (r, _) = ui.allocate_exact_size(vec2(44.0, 44.0), Sense::hover());
            let size = texture.size_vec2();
            let fit = (r.width() / size.x).min(r.height() / size.y);
            ui.painter().image(texture.id(), Rect::from_center_size(r.center(), size * fit), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.add(egui::Label::new(RichText::new(&def.name).strong()).truncate()).on_hover_text(&def.source);
                ui.label(RichText::new(tr!("{value} × {value2} × {value3} · {n} blocks", value = def.size[0], value2 = def.size[1], value3 = def.size[2], n = def.count())).size(11.0).color(pal.weak));
                ui.label(RichText::new(size_name(classes.get(row).copied().unwrap_or(1) as u16)).size(11.0).color(pal.weak)).on_hover_text(tr!("Its size, where Tree sizes are painted: a quarter of the layer's objects each, by height"));
                ui.horizontal(|ui| {
                    crate::widgets::drag(ui, egui::DragValue::new(weight).range(0..=1000).speed(0.2)).on_hover_text(tr!("Weight: how often it stands against the others. 0: only where Tree sizes ask for its size"));
                    ui.label(RichText::new(format!("{:.0}%", *weight as f32 * 100.0 / total)).size(11.0).color(pal.weak));
                    if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Take it out of the layer")).clicked() {
                        remove = Some(row);
                    }
                });
            });
        });
    }
    if let Some(row) = remove {
        layer.objects.remove(row);
    }
    let add = ui.button(theme::with_icon(Icon::Plus, tr!("Add objects…"))).on_hover_text(tr!("Read more files into this layer")).clicked();
    if layer.objects.is_empty() {
        hint(ui, tr!("Nothing stands yet: add objects."));
    }
    ui.add_space(6.0);
    let removed = ui
        .button(RichText::new(theme::with_icon(Icon::Trash2, tr!("Remove this layer"))).color(pal.danger))
        .on_hover_text(tr!("Nothing stands where it was painted any more"))
        .clicked();
    if removed {
        layer.removed = true;
    }
    reg.objects = objects;
    if removed {
        tools.object_layer = reg.object_layers.iter().position(|l| !l.removed).map_or(0, |k| k as u16 + 1);
    }
    // What is painted of it moves to the sub-canopy, or back to the base,
    // as one step to undo.
    if let Some(under) = role {
        reg.object_layers[k].understory = under;
        let (n, levels) = (k as u16 + 1, doc.dimension().levels.len());
        doc.history.begin(if under { tr!("Sub-canopy layer") } else { tr!("Base tree layer") }, doc.dim);
        for level in 0..levels {
            massif_core::ops::move_object_layer(&mut doc.history.edit(&mut doc.world, level), n, under);
        }
        doc.history.commit();
    }
    if add {
        let files = pick(tr!("Add objects"));
        if !files.is_empty() {
            import_objects(doc, tools, &files, Some(k as u16 + 1), blocks.catalogue.as_deref());
        }
    }
    if (doc.world.registry.object_layers.clone(), doc.world.registry.objects.len()) != before {
        doc.mark_changed();
    }
}

/// Plants offered when the game's blocks are not loaded.
const COMMON_PLANTS: [&str; 24] = [
    "minecraft:short_grass",
    "minecraft:tall_grass",
    "minecraft:fern",
    "minecraft:large_fern",
    "minecraft:dandelion",
    "minecraft:poppy",
    "minecraft:blue_orchid",
    "minecraft:allium",
    "minecraft:azure_bluet",
    "minecraft:red_tulip",
    "minecraft:oxeye_daisy",
    "minecraft:cornflower",
    "minecraft:lily_of_the_valley",
    "minecraft:sunflower",
    "minecraft:lilac",
    "minecraft:rose_bush",
    "minecraft:peony",
    "minecraft:dead_bush",
    "minecraft:brown_mushroom",
    "minecraft:red_mushroom",
    "minecraft:sweet_berry_bush",
    "minecraft:moss_carpet",
    "minecraft:snow[layers=1]",
    "minecraft:seagrass",
];

/// `minecraft:oxeye_daisy[x=y]` → `Oxeye daisy`.
pub fn plant_name(state: &str) -> String {
    let name = state.split('[').next().unwrap_or(state);
    let short = name.strip_prefix("minecraft:").unwrap_or(name).replace('_', " ");
    let mut c = short.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// The name of tree size `n` (1 to 4).
pub fn size_name(n: u16) -> &'static str {
    match n {
        1 => tr!("Bushes"),
        2 => tr!("Small trees"),
        3 => tr!("Medium trees"),
        _ => tr!("Large trees"),
    }
}

/// The four tree sizes as swatches `size` points wide in their colours, the
/// one of `picked` ringed, each named beside it when `named`; returns the
/// one clicked.
pub fn size_swatches(ui: &mut Ui, size: f32, picked: Option<u16>, named: bool) -> Option<u16> {
    let pal = Palette::of(ui);
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(size / 4.0, size / 6.0);
        for (k, color) in crate::map_render::SIZE_COLORS.iter().enumerate() {
            let n = k as u16 + 1;
            let name = size_name(n);
            let width = if named { size + 8.0 + ui.painter().layout_no_wrap(name.to_owned(), egui::FontId::proportional(12.0), pal.text).size().x + 8.0 } else { size };
            let (rect, response) = ui.allocate_exact_size(vec2(width, size), Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, picked == Some(n), name));
            let painter = ui.painter();
            let radius = theme::TILE_RADIUS / 2;
            let swatch = Rect::from_min_size(rect.min, vec2(size, size)).shrink(size / 8.0);
            painter.rect(swatch, radius, theme::rgb(*color), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            // A tree, as tall as the size.
            let h = swatch.height() * (0.35 + 0.15 * n as f32);
            let foot = swatch.center_bottom() - vec2(0.0, swatch.height() * 0.12);
            painter.circle_filled(foot - vec2(0.0, h * 0.6), h * 0.35, Color32::from_black_alpha(90));
            painter.line_segment([foot, foot - vec2(0.0, h * 0.4)], Stroke::new(1.5, Color32::from_black_alpha(110)));
            if named {
                painter.text(rect.left_center() + vec2(size + 6.0, 0.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(12.0), pal.text);
            }
            if picked == Some(n) {
                painter.rect_stroke(rect, radius, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
            } else if response.hovered() {
                painter.rect_stroke(rect, radius, Stroke::new(1.0, pal.text), StrokeKind::Inside);
            }
            let response = if named { response } else { response.on_hover_text(name) };
            if response.clicked() {
                clicked = Some(n);
            }
        }
    });
    clicked
}

/// The tree sizes to paint: a click picks one.
fn tree_size_settings(ui: &mut Ui, tools: &mut ToolState) {
    if let Some(n) = size_swatches(ui, 26.0, Some(tools.tree_size), true) {
        tools.tree_size = n;
    }
    hint(ui, tr!("Paint the sizes first, then a species over them (one object layer, such as Oaks): where Small trees is marked, only its small oaks stand. A layer's objects are split by height into four sizes, a quarter each; where it has none of a size, the nearest one."));
    ui.add_space(4.0);
    crate::widgets::well(ui, |ui| {
        crate::widgets::row(ui, tr!("Show over the map with every tool"), |ui| {
            crate::widgets::switch(ui, &mut tools.show_sizes);
        });
    });
}

/// The name of annotation colour `n` (1 to 15).
pub fn annotation_name(n: u16) -> &'static str {
    match n {
        1 => tr!("White"),
        2 => tr!("Orange"),
        3 => tr!("Magenta"),
        4 => tr!("Light blue"),
        5 => tr!("Yellow"),
        6 => tr!("Lime"),
        7 => tr!("Pink"),
        8 => tr!("Light grey"),
        9 => tr!("Cyan"),
        10 => tr!("Purple"),
        11 => tr!("Blue"),
        12 => tr!("Brown"),
        13 => tr!("Green"),
        14 => tr!("Red"),
        _ => tr!("Black"),
    }
}

/// The fifteen colours of annotations as swatches `size` points wide, the
/// one of `picked` ringed; returns the one clicked.
pub fn annotation_swatches(ui: &mut Ui, size: f32, picked: Option<u16>) -> Option<u16> {
    let pal = Palette::of(ui);
    let mut clicked = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(size / 6.0, size / 6.0);
        for (k, color) in massif_core::tile::ANNOTATION_COLORS.iter().enumerate() {
            let n = k as u16 + 1;
            let name = annotation_name(n);
            let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::click());
            response.widget_info(|| egui::WidgetInfo::selected(egui::Role::RadioButton, true, picked == Some(n), name));
            let painter = ui.painter();
            let radius = theme::TILE_RADIUS / 2;
            // A hairline around each, so that black shows on a dark panel
            // and white on a light one.
            painter.rect(rect.shrink(size / 8.0), radius, theme::rgb(*color), Stroke::new(1.0, pal.border), StrokeKind::Inside);
            if picked == Some(n) {
                painter.rect_stroke(rect, radius, Stroke::new(2.0, pal.accent), StrokeKind::Inside);
            } else if response.hovered() {
                painter.rect_stroke(rect, radius, Stroke::new(1.0, pal.text), StrokeKind::Inside);
            }
            if response.on_hover_text(name).clicked() {
                clicked = Some(n);
            }
        }
    });
    clicked
}

/// The colours to annotate with: a click picks one.
fn annotation_settings(ui: &mut Ui, tools: &mut ToolState) {
    if let Some(n) = annotation_swatches(ui, 24.0, Some(tools.annotation)) {
        tools.annotation = n;
    }
    hint(ui, tr!("Shown on the map, as in WorldPainter, and never exported. The pen button in the map's header hides them."));
}

/// What a cover grows, in a line.
pub fn describe_cover(def: &massif_core::registry::CoverDef) -> String {
    if def.lava && def.plants.is_empty() {
        return tr!("The water where it is painted is lava").to_owned();
    }
    let total: u32 = def.plants.iter().map(|p| p.1 as u32).sum::<u32>().max(1);
    let plants: Vec<String> = def.plants.iter().map(|(b, w)| format!("{} {:.0}%", plant_name(b), *w as f32 * 100.0 / total as f32)).collect();
    let where_ = match (def.underwater, def.soil_only) {
        (true, _) => tr!(" · under water"),
        (false, true) => tr!(" · on soil"),
        _ => "",
    };
    tr!("{value:.0}% of the ground: {join}{where_}", value = def.density * 100.0, join = plants.join(", "), where_)
}

/// The cover of a block of the game: one that grows it alone, made if
/// there is none yet. Returns its number.
pub fn block_cover(reg: &mut Registry, block: &massif_assets::Block) -> u16 {
    // A block two high (a mod's tall plant) is kept as its lower half: the
    // export writes both.
    let state = if block.tall { format!("{}[half=lower]", block.id) } else { block.id.clone() };
    if let Some(k) = reg.covers.iter().position(|c| !c.removed && c.plants.len() == 1 && c.plants[0].0 == state) {
        return k as u16 + 1;
    }
    let plant = block.category == massif_assets::Category::Plants && !block.id.contains("carpet");
    // A plant shows its sides (a cross of them), tinted as the game tints
    // it, rather than what stands for its top.
    let color = if block.full_cube { block.color } else { block.side_color };
    let mut def = massif_core::registry::CoverDef::new(&block.name, color, &[(&state, 1)], if plant { 0.35 } else { 1.0 });
    def.soil_only = plant;
    def.underwater = ["seagrass", "kelp", "coral", "sea_pickle"].iter().any(|w| block.id.contains(w));
    if def.underwater {
        def.soil_only = false;
    }
    reg.add_cover(def)
}

/// A new cover of short grass, named after how many there are. Returns
/// its number.
pub fn new_cover(reg: &mut Registry) -> u16 {
    let count = reg.covers.iter().filter(|c| !c.removed).count();
    let name = crate::layers::unique_name(&tr!("Cover {value}", value = count + 1), reg.covers.iter().filter(|c| !c.removed).map(|c| c.name.as_str()));
    reg.add_cover(massif_core::registry::CoverDef::new(&name, [120, 170, 90], &[("minecraft:short_grass", 1)], 0.5))
}

/// A new layer of underground pockets, of gravel to start with, in the rock
/// of the dimension; returns its number.
pub fn new_pockets(reg: &mut Registry, dim: &massif_core::world::Dimension) -> u16 {
    let count = reg.pockets.iter().filter(|p| !p.removed).count();
    let name = crate::layers::unique_name(&tr!("Pockets {value}", value = count + 1), reg.pockets.iter().filter(|p| !p.removed).map(|p| p.name.as_str()));
    let gravel = reg.intern_block("minecraft:gravel", massif_core::objects::guess_color("minecraft:gravel"));
    reg.add_pocket(massif_core::pockets::PocketDef::new(&name, [140, 132, 124], &[(gravel, 1)], 0.15, 5.0, dim.min_y + 5, 50))
}

/// Blocks offered for pockets when the game's blocks are not loaded.
const COMMON_POCKETS: [&str; 12] = [
    "minecraft:gravel",
    "minecraft:dirt",
    "minecraft:coarse_dirt",
    "minecraft:clay",
    "minecraft:sand",
    "minecraft:andesite",
    "minecraft:granite",
    "minecraft:diorite",
    "minecraft:tuff",
    "minecraft:calcite",
    "minecraft:deepslate",
    "minecraft:magma_block",
];

/// The pocket layers of the world, and the one painted.
fn pocket_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let before = doc.world.registry.pockets.clone();
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, p) in doc.world.registry.pockets.iter().enumerate() {
        if p.removed {
            continue;
        }
        let n = k as u16 + 1;
        let selected = tools.pocket_layer == n;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let fill = if selected {
            pal.accent_soft
        } else if response.hovered() {
            pal.header
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
        let dot = Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(11.0, 11.0));
        painter.rect(dot, 3, theme::rgb(p.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
        painter.text(rect.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, &p.name, egui::FontId::proportional(12.5), pal.text);
        painter.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{:.0}%", p.share * 100.0), egui::FontId::proportional(11.0), pal.weak);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &p.name));
        if response.clicked() {
            picked = Some(n);
        }
    }
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(n) = picked {
        tools.pocket_layer = n;
    }
    if ui.button(theme::with_icon(Icon::Plus, tr!("New pockets"))).on_hover_text(tr!("Pockets of your own: any blocks, in the rock under the ground")).clicked() {
        let dim = &doc.world.dims[doc.dim];
        tools.pocket_layer = new_pockets(&mut doc.world.registry, dim);
    }
    pocket_editor(ui, tools, doc, blocks);
    if doc.world.registry.pockets != before {
        doc.mark_changed();
    }
}

/// The settings of the pocket layer picked (`tools.pocket_layer`), as Tool
/// Settings and the layer's own dialog show them.
pub fn pocket_editor(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let ctx = ui.ctx().clone();
    let before = doc.world.registry.pockets.clone();
    let catalogue = blocks.catalogue.clone();
    let (floor, ceiling) = (doc.dimension().min_y, doc.dimension().max_y - 1);
    let reg = &mut doc.world.registry;
    let Some(k) = tools.pocket_layer.checked_sub(1).map(usize::from).filter(|&k| reg.pockets.get(k).is_some_and(|d| !d.removed)) else {
        if reg.pockets.iter().all(|p| p.removed) {
            hint(ui, tr!("No pockets yet: make a layer of them to paint."));
        }
        return;
    };
    // Its blocks' states, read before the layer is taken to be edited.
    let states: Vec<String> = reg.pockets[k].blocks.iter().map(|(b, _)| reg.block(*b).state.clone()).collect();
    let def = &mut reg.pockets[k];
    ui.add_space(4.0);
    section(ui, Icon::Gem, &def.name.clone());
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Name")));
        ui.add(egui::TextEdit::singleline(&mut def.name).desired_width((ui.available_width() - 40.0).max(60.0)));
        let mut c = Color32::from_rgb(def.color[0], def.color[1], def.color[2]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour over the map while it is painted")).changed() {
            def.color = [c.r(), c.g(), c.b()];
        }
    });
    tip_row(ui, tr!("Share"), tr!("How much of the rock the pockets take where the layer is painted"), |ui| {
        ui.add(
            crate::widgets::Field::new(&mut def.share, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    tip_row(ui, tr!("Size"), tr!("About how many blocks across a pocket is"), |ui| ui.add(crate::widgets::Field::new(&mut def.size, 1.0..=64.0).suffix(tr!(" blocks"))));
    tip_row(ui, tr!("Highest"), tr!("The highest a pocket goes; never into the strata of the ground"), |ui| crate::widgets::drag(ui, egui::DragValue::new(&mut def.high).range(floor..=ceiling)));
    tip_row(ui, tr!("Lowest"), tr!("The lowest a pocket goes"), |ui| crate::widgets::drag(ui, egui::DragValue::new(&mut def.low).range(floor..=ceiling)));
    if def.low > def.high {
        std::mem::swap(&mut def.low, &mut def.high);
    }

    // What the pockets are made of, and how much of each.
    section(ui, Icon::Boxes, tr!("Made of"));
    let total: f32 = def.blocks.iter().map(|b| b.1 as f32).sum::<f32>().max(1.0);
    let mut remove = None;
    egui::Grid::new("pocket blocks").num_columns(4).spacing([6.0, 3.0]).show(ui, |ui| {
        for (k, ((_, weight), state)) in def.blocks.iter_mut().zip(&states).enumerate() {
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            match catalogue.as_ref().and_then(|cat| cat.block(state)).and_then(|b| blocks.icon(&ctx, b)) {
                Some(t) => {
                    ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    ui.painter().rect_filled(r.shrink(3.0), 3, theme::rgb(massif_core::objects::guess_color(state)));
                }
            }
            ui.label(plant_name(state)).on_hover_text(state.as_str());
            ui.horizontal(|ui| {
                crate::widgets::drag(ui, egui::DragValue::new(weight).range(1..=1000).speed(0.2));
                ui.label(RichText::new(format!("{:.0}%", *weight as f32 * 100.0 / total)).size(11.0).color(pal.weak));
            });
            if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    if let Some(k) = remove
        && def.blocks.len() > 1
    {
        def.blocks.remove(k);
    }
    let search_id = egui::Id::unique("pocket block search");
    let mut added = None;
    ui.menu_button(theme::with_icon(Icon::Plus, tr!("Add a block")), |ui| {
        ui.set_max_width(300.0);
        let mut search: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
        let hint_text = tr!("{glyph}  Search, or type a block id", glyph = theme::glyph(Icon::Search));
        ui.add(egui::TextEdit::singleline(&mut search).hint_text(hint_text).desired_width(280.0)).request_focus();
        let needle = search.trim().to_lowercase();
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            let mut shown = 0;
            match &catalogue {
                Some(cat) => {
                    for b in cat.blocks.iter().filter(|b| b.full_cube) {
                        if !needle.is_empty() && !b.name.to_lowercase().contains(&needle) && !b.id.contains(&needle) {
                            continue;
                        }
                        shown += 1;
                        if shown > 200 {
                            break;
                        }
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            if let Some(t) = blocks.icon(&ctx, b) {
                                ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            if ui.selectable_label(false, &b.name).on_hover_text(&b.id).clicked() {
                                added = Some(b.id.clone());
                            }
                        });
                    }
                }
                None => {
                    for id in COMMON_POCKETS {
                        if needle.is_empty() || id.contains(&needle) {
                            shown += 1;
                            if ui.selectable_label(false, plant_name(id)).on_hover_text(id).clicked() {
                                added = Some(id.to_owned());
                            }
                        }
                    }
                }
            }
            if needle.contains(':') || (shown == 0 && !needle.is_empty()) {
                let id = if needle.contains(':') { needle.clone() } else { format!("minecraft:{}", needle.replace(' ', "_")) };
                if ui.button(tr!("Add {id}", id)).clicked() {
                    added = Some(id);
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(search_id, search));
        if added.is_some() {
            ui.close();
        }
    });
    hint(ui, tr!("In the rock under the ground's strata, where the layer is painted: blobs of these blocks, as WorldPainter's underground pockets. Shown over the map while the tool is in hand."));
    ui.add_space(6.0);
    if ui.button(RichText::new(theme::with_icon(Icon::Trash2, tr!("Remove this layer"))).color(pal.danger)).on_hover_text(tr!("No pocket where it was painted any more")).clicked() {
        def.removed = true;
        tools.pocket_layer = reg.pockets.iter().position(|p| !p.removed).map_or(0, |k| k as u16 + 1);
    }
    // A block added: one of the registry's, as a terrain's.
    if let Some(id) = added {
        let color = catalogue.as_ref().and_then(|cat| cat.block(&id)).map_or_else(|| massif_core::objects::guess_color(&id), |b| b.color);
        let b = reg.intern_block(&id, color);
        if !reg.pockets[k].blocks.iter().any(|x| x.0 == b) {
            reg.pockets[k].blocks.push((b, 1));
        }
    }
    if reg.pockets != before {
        doc.mark_changed();
    }
}

/// A new combined layer, painting nothing yet: its number.
pub fn new_combined(reg: &mut Registry) -> u16 {
    let count = reg.combined.iter().filter(|c| !c.removed).count();
    let name = crate::layers::unique_name(&tr!("Combined {value}", value = count + 1), reg.combined.iter().filter(|c| !c.removed).map(|c| c.name.as_str()));
    reg.add_combined(massif_core::combined::CombinedDef::new(&name, [96, 132, 62]))
}

/// The name of a slot a combined layer paints into, short, as
/// `docs/layers.md` names the slots.
pub fn part_name(part: massif_core::combined::Part) -> &'static str {
    use massif_core::combined::Part;
    match part {
        Part::Terrain => tr!("Terrain"),
        Part::Biome => tr!("Biome"),
        Part::Cover => tr!("Cover"),
        Part::Trees => tr!("Trees"),
        Part::Under => tr!("Sub-canopy"),
        Part::Size => tr!("Tree size"),
        Part::Pockets => tr!("Pockets"),
    }
}

/// The layers of slot `part` a combined layer may paint, by number, with
/// their names.
pub fn part_choices(reg: &Registry, part: massif_core::combined::Part) -> Vec<(u16, String)> {
    use massif_core::combined::Part;
    let live = |removed: bool| !removed;
    match part {
        Part::Terrain => reg.terrains.iter().enumerate().map(|(k, t)| (k as u16, t.name.clone())).collect(),
        Part::Biome => reg.biomes.iter().enumerate().skip(1).map(|(k, b)| (k as u16, b.name.clone())).collect(),
        Part::Cover => reg.covers.iter().enumerate().filter(|(_, c)| live(c.removed)).map(|(k, c)| (k as u16 + 1, c.name.clone())).collect(),
        Part::Trees | Part::Under => reg.object_layers.iter().enumerate().filter(|(_, l)| live(l.removed) && l.understory == (part == Part::Under)).map(|(k, l)| (k as u16 + 1, l.name.clone())).collect(),
        Part::Size => (1..=4).map(|n| (n, size_name(n).to_owned())).collect(),
        Part::Pockets => reg.pockets.iter().enumerate().filter(|(_, p)| live(p.removed)).map(|(k, p)| (k as u16 + 1, p.name.clone())).collect(),
    }
}

/// What combined layer `def` paints, in words: "Terrain: Grass · Ground
/// cover: Ferns 50%".
pub fn describe_combined(reg: &Registry, def: &massif_core::combined::CombinedDef) -> String {
    let parts: Vec<String> = def
        .parts
        .iter()
        .map(|c| {
            let name = part_choices(reg, c.part).into_iter().find(|(n, _)| *n == c.layer).map_or_else(|| tr!("(removed)").to_owned(), |(_, name)| name);
            if c.share < 1.0 { format!("{}: {name} {:.0}%", part_name(c.part), c.share * 100.0) } else { format!("{}: {name}", part_name(c.part)) }
        })
        .collect();
    if parts.is_empty() { tr!("Nothing yet: choose what it paints in its settings.").to_owned() } else { parts.join(" · ") }
}

/// The combined layers of the world, and the one painted.
fn combined_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    let pal = Palette::of(ui);
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, c) in doc.world.registry.combined.iter().enumerate() {
        if c.removed {
            continue;
        }
        let n = k as u16 + 1;
        let selected = tools.combined == n;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let fill = if selected {
            pal.accent_soft
        } else if response.hovered() {
            pal.header
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
        let dot = Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(11.0, 11.0));
        painter.rect(dot, 3, theme::rgb(c.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
        painter.text(rect.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, &c.name, egui::FontId::proportional(12.5), pal.text);
        painter.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, trn!("{n} layer", "{n} layers", n = c.parts.len()), egui::FontId::proportional(11.0), pal.weak);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &c.name));
        if response.clicked() {
            picked = Some(n);
        }
    }
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(n) = picked {
        tools.combined = n;
    }
    if ui.button(theme::with_icon(Icon::Plus, tr!("New combined layer"))).on_hover_text(tr!("Several layers painted at once: a terrain, a biome, a cover, trees, pockets")).clicked() {
        tools.combined = new_combined(&mut doc.world.registry);
    }
    combined_editor(ui, tools, doc);
}

/// The settings of the combined layer picked (`tools.combined`), as Tool
/// Settings and the layer's own dialog show them: its name and colour,
/// and the layer of each slot it paints, over its share.
pub fn combined_editor(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document) {
    use massif_core::combined::Part;
    // The width of a row's share.
    const SHARE_W: f32 = 60.0;
    let pal = Palette::of(ui);
    let reg = &doc.world.registry;
    let Some(k) = tools.combined.checked_sub(1).map(usize::from).filter(|&k| reg.combined.get(k).is_some_and(|c| !c.removed)) else {
        if reg.combined.iter().all(|c| c.removed) {
            hint(ui, tr!("No combined layers yet: make one to paint several layers at once."));
        }
        return;
    };
    // The layers each slot offers, read before the layer is taken to be
    // edited.
    let choices: Vec<Vec<(u16, String)>> = Part::ALL.iter().map(|&p| part_choices(reg, p)).collect();
    let before = reg.combined[k].clone();
    let def = &mut doc.world.registry.combined[k];
    ui.add_space(4.0);
    section(ui, Icon::Combine, &def.name.clone());
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Name")));
        ui.add(egui::TextEdit::singleline(&mut def.name).desired_width((ui.available_width() - 40.0).max(60.0)));
        let mut c = Color32::from_rgb(def.color[0], def.color[1], def.color[2]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour in the lists")).changed() {
            def.color = [c.r(), c.g(), c.b()];
        }
    });
    ui.add_space(4.0);
    section(ui, Icon::Layers, tr!("It paints"));
    // A slot's layer and its share, from the right: the share (or the room
    // for it, so that the layers line up), then the layer, `width` wide.
    let mut controls = |ui: &mut Ui, part: Part, choices: &[(u16, String)], width: f32| {
        let mut current = def.get(part);
        match &mut current {
            Some((_, share)) => {
                let field = crate::widgets::Field::new(share, 0.0..=1.0)
                    .width(SHARE_W)
                    .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                    .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0));
                if ui.add(field).on_hover_text(tr!("The share of the columns painted it takes")).changed() {
                    def.set(part, current);
                }
            }
            None => ui.add_space(SHARE_W + ui.spacing().item_spacing.x),
        }
        let shown = current.and_then(|(n, _)| choices.iter().find(|(c, _)| *c == n)).map_or(tr!("None"), |(_, name)| name.as_str());
        let mut chosen = None;
        crate::widgets::combo(("combined part", part as usize)).selected_text(shown).width(width).height(320.0).truncate().show_ui(ui, |ui| {
            if ui.selectable_label(current.is_none(), tr!("None")).clicked() {
                chosen = Some(None);
            }
            for (n, name) in choices {
                if ui.selectable_label(current.is_some_and(|c| c.0 == *n), name).clicked() {
                    chosen = Some(Some((*n, current.map_or(1.0, |c| c.1))));
                }
            }
        });
        if let Some(what) = chosen {
            def.set(part, what);
        }
    };
    // Each slot on a row, its name on the left; in a panel too narrow for
    // the longest name beside them, its name over them.
    let spacing = ui.spacing().item_spacing.x;
    let room = ui.available_width() - 24.0 - SHARE_W - 2.0 * spacing;
    let longest = Part::ALL.iter().map(|&p| ui.painter().layout_no_wrap(part_name(p).to_owned(), egui::FontId::proportional(12.5), pal.text).size().x).fold(0.0, f32::max);
    let beside = (room - longest - spacing).min(124.0);
    crate::widgets::well(ui, |ui| {
        for (part, choices) in Part::ALL.into_iter().zip(&choices) {
            if beside >= 96.0 {
                crate::widgets::row(ui, part_name(part), |ui| controls(ui, part, choices, beside));
            } else {
                ui.add_space(6.0);
                ui.label(RichText::new(part_name(part)).size(12.5));
                ui.add_space(2.0);
                ui.allocate_ui_with_layout(vec2(ui.available_width(), 28.0), egui::Layout::right_to_left(Align::Center), |ui| controls(ui, part, choices, room));
                ui.add_space(4.0);
            }
        }
    });
    hint(ui, tr!("Each layer goes to its slot of the blocks, over its share of the columns painted, as its own tool paints it: a block keeps one layer of each slot. Changing the combined layer changes what it paints next, not what it painted. Right button: rub out what it paints."));
    ui.add_space(6.0);
    if ui.button(RichText::new(theme::with_icon(Icon::Trash2, tr!("Remove this layer"))).color(pal.danger)).on_hover_text(tr!("What it painted stays")).clicked() {
        def.removed = true;
        tools.combined = doc.world.registry.combined.iter().position(|c| !c.removed).map_or(0, |k| k as u16 + 1);
    }
    if doc.world.registry.combined.get(k) != Some(&before) {
        doc.mark_changed();
    }
}

/// The covers of the world, and the one painted: what it grows, how
/// thick, where.
fn cover_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let before = doc.world.registry.covers.clone();

    // The covers to choose from.
    let mut picked = None;
    ui.spacing_mut().item_spacing.y = 2.0;
    for (k, c) in doc.world.registry.covers.iter().enumerate() {
        if c.removed {
            continue;
        }
        let n = k as u16 + 1;
        let selected = tools.cover == n;
        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::click());
        let fill = if selected {
            pal.accent_soft
        } else if response.hovered() {
            pal.header
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter();
        painter.rect(rect, theme::TILE_RADIUS, fill, if selected { Stroke::new(1.0, pal.accent) } else { Stroke::NONE }, StrokeKind::Inside);
        let dot = Rect::from_center_size(rect.left_center() + vec2(13.0, 0.0), vec2(11.0, 11.0));
        painter.rect(dot, 3, theme::rgb(c.color), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
        painter.text(rect.left_center() + vec2(26.0, 0.0), egui::Align2::LEFT_CENTER, &c.name, egui::FontId::proportional(12.5), pal.text);
        painter.text(rect.right_center() - vec2(8.0, 0.0), egui::Align2::RIGHT_CENTER, format!("{:.0}%", c.density * 100.0), egui::FontId::proportional(11.0), pal.weak);
        response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &c.name));
        if response.on_hover_text(describe_cover(c)).clicked() {
            picked = Some(n);
        }
    }
    ui.spacing_mut().item_spacing.y = 4.0;
    if let Some(n) = picked {
        tools.cover = n;
    }
    if ui.button(theme::with_icon(Icon::Plus, tr!("New cover"))).on_hover_text(tr!("A cover of your own: any blocks, any mix")).clicked() {
        tools.cover = new_cover(&mut doc.world.registry);
    }
    hint(ui, tr!("Non-cube blocks clicked in Blocks become covers too. Every cover and object layer of the map: the Layers panel."));

    cover_editor(ui, tools, doc, blocks);
    if doc.world.registry.covers != before {
        doc.mark_changed();
    }
}

/// The settings of the cover picked (`tools.cover`), as Tool Settings and
/// the layer's own dialog show them.
pub fn cover_editor(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    let pal = Palette::of(ui);
    let ctx = ui.ctx().clone();
    let before = doc.world.registry.covers.clone();
    // The selected cover.
    let catalogue = blocks.catalogue.clone();
    let Some(def) = tools.cover.checked_sub(1).and_then(|k| doc.world.registry.covers.get_mut(k as usize)).filter(|d| !d.removed) else {
        return;
    };
    ui.add_space(4.0);
    section(ui, Icon::Sprout, &def.name.clone());
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Name")));
        ui.add(egui::TextEdit::singleline(&mut def.name).desired_width((ui.available_width() - 40.0).max(60.0)));
        let mut c = Color32::from_rgb(def.color[0], def.color[1], def.color[2]);
        if egui::color_picker::color_edit_button_srgba(ui, &mut c, egui::color_picker::Alpha::Opaque).on_hover_text(tr!("Its colour on the map")).changed() {
            def.color = [c.r(), c.g(), c.b()];
        }
    });
    tip_row(ui, tr!("Density"), tr!("Share of the painted ground where something grows"), |ui| {
        ui.add(
            crate::widgets::Field::new(&mut def.density, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    ui.horizontal(|ui| {
        let tip = tr!("How many blocks high it grows (cane, cactus: 1 to 3). Tall plants are always two.");
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Height"))).on_hover_text(tip);
        let (mut lo, mut hi) = (def.height[0] as i32, def.height[1] as i32);
        crate::widgets::drag(ui, egui::DragValue::new(&mut lo).range(1..=16)).on_hover_text(tip);
        ui.label(RichText::new(tr!("to")).color(pal.weak));
        crate::widgets::drag(ui, egui::DragValue::new(&mut hi).range(1..=16)).on_hover_text(tip);
        ui.label(RichText::new(tr!("blocks")).color(pal.weak));
        def.height = [lo as u8, hi.max(lo) as u8];
    });
    crate::widgets::checkbox(ui, &mut def.soil_only, tr!("Only on soil")).on_hover_text(tr!("Grass, dirt, podzol, moss, mud… as plants need in the game"));
    crate::widgets::checkbox(ui, &mut def.underwater, tr!("Under water")).on_hover_text(tr!("Grows in the water (seagrass, coral) rather than in the air"));
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("The water")));
        let chosen = [!def.frost && !def.lava, def.frost, def.lava];
        if let Some(k) = crate::widgets::button_group(ui, "cover water", &[tr!("As it is"), tr!("Frozen"), tr!("Lava")], &chosen) {
            (def.frost, def.lava) = (k == 1, k == 2);
        }
    })
    .response
    .on_hover_text(tr!("Where the cover is painted, the top of the water turns to ice (WorldPainter's Frost), or all of it to lava (its Lava layer)"));

    // What it grows, and how much of each.
    section(ui, Icon::Flower2, tr!("Grows"));
    let total: f32 = def.plants.iter().map(|p| p.1 as f32).sum::<f32>().max(1.0);
    let mut remove = None;
    let color = def.color;
    egui::Grid::new("cover plants").num_columns(5).spacing([6.0, 3.0]).show(ui, |ui| {
        for (k, (state, weight)) in def.plants.iter_mut().enumerate() {
            let (r, _) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            let icon = catalogue.as_ref().and_then(|cat| cat.block(state)).and_then(|b| blocks.icon(&ctx, b));
            match icon {
                Some(t) => {
                    ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                }
                None => {
                    let c = massif_core::registry::plant_color(state).unwrap_or(color);
                    ui.painter().rect(r.shrink(5.0), 3, theme::rgb(c), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
                }
            }
            let known = catalogue.as_ref().is_none_or(|cat| cat.block(state).is_some());
            let tip = if known { state.clone() } else { tr!("{state}\nNot a block of Minecraft {map_or}", map_or = catalogue.as_ref().map_or("", |c| c.version.as_str()), state) };
            ui.allocate_ui_with_layout(vec2(118.0, 20.0), egui::Layout::left_to_right(Align::Center), |ui| {
                ui.set_width(118.0);
                ui.add(egui::Label::new(RichText::new(plant_name(state)).color(if known { pal.text } else { pal.danger })).truncate()).on_hover_text(tip);
            });
            crate::widgets::drag(ui, egui::DragValue::new(weight).range(1..=1000).speed(0.2)).on_hover_text(tr!("Weight: how often it grows against the others"));
            ui.label(RichText::new(format!("{:.0}%", *weight as f32 * 100.0 / total)).size(11.0).color(pal.weak));
            if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Take it out")).clicked() {
                remove = Some(k);
            }
            ui.end_row();
        }
    });
    if let Some(k) = remove {
        def.plants.remove(k);
    }
    // Add a plant: one of the game's blocks, or typed.
    let search_id = egui::Id::unique("cover plant search");
    let mut added = None;
    ui.menu_button(theme::with_icon(Icon::Plus, tr!("Add a block")), |ui| {
        ui.set_max_width(300.0);
        let mut search: String = ui.ctx().data(|d| d.get_temp(search_id)).unwrap_or_default();
        let hint_text = tr!("{glyph}  Search, or type a block id", glyph = theme::glyph(Icon::Search));
        let edit = ui.add(egui::TextEdit::singleline(&mut search).hint_text(hint_text).desired_width(280.0));
        edit.request_focus();
        let needle = search.trim().to_lowercase();
        egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
            let mut shown = 0;
            match &catalogue {
                Some(cat) => {
                    // Plants first, then every other block that is not a cube.
                    let mut list: Vec<&massif_assets::Block> = cat.blocks.iter().filter(|b| !b.full_cube || b.category == massif_assets::Category::Plants).collect();
                    list.sort_by_key(|b| (b.category != massif_assets::Category::Plants, b.name.clone()));
                    for b in list {
                        if !needle.is_empty() && !b.name.to_lowercase().contains(&needle) && !b.id.contains(&needle) {
                            continue;
                        }
                        shown += 1;
                        if shown > 200 {
                            break;
                        }
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                            if let Some(t) = blocks.icon(&ctx, b) {
                                ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                            }
                            if ui.selectable_label(false, &b.name).on_hover_text(&b.id).clicked() {
                                added = Some(b.id.clone());
                            }
                        });
                    }
                }
                None => {
                    for id in COMMON_PLANTS {
                        if needle.is_empty() || id.contains(&needle) {
                            shown += 1;
                            if ui.selectable_label(false, plant_name(id)).on_hover_text(id).clicked() {
                                added = Some(id.to_owned());
                            }
                        }
                    }
                }
            }
            // Any block by its id.
            if needle.contains(':') || (shown == 0 && !needle.is_empty()) {
                let id = if needle.contains(':') { needle.clone() } else { format!("minecraft:{}", needle.replace(' ', "_")) };
                if ui.button(tr!("Add {id}", id)).clicked() {
                    added = Some(id);
                }
            }
        });
        ui.ctx().data_mut(|d| d.insert_temp(search_id, search));
        if added.is_some() {
            ui.close();
        }
    });
    if let Some(id) = added {
        match def.plants.iter_mut().find(|p| p.0 == id) {
            Some(p) => p.1 = p.1.saturating_add(1),
            None => def.plants.push((id, 1)),
        }
        ui.ctx().data_mut(|d| d.remove::<String>(search_id));
    }
    if def.plants.is_empty() {
        hint(ui, tr!("Nothing grows yet: add a block."));
    }
    ui.add_space(6.0);
    if ui
        .button(RichText::new(theme::with_icon(Icon::Trash2, tr!("Remove this cover"))).color(pal.danger))
        .on_hover_text(tr!("Nothing grows where it was painted any more"))
        .clicked()
    {
        def.removed = true;
        tools.cover = doc.world.registry.covers.iter().position(|c| !c.removed).map_or(0, |k| k as u16 + 1);
    }
    if doc.world.registry.covers != before {
        doc.mark_changed();
    }
}

/// A terrain of the map, or a whole block of the game to make one of.
pub(crate) enum Pick {
    Terrain(String),
    Block(massif_assets::Block),
}

/// The name of the terrain picked, made from its block the first time.
pub(crate) fn picked_terrain(pick: Pick, doc: &mut Document) -> String {
    match pick {
        Pick::Terrain(name) => name,
        Pick::Block(block) => {
            let id = massif_assets::block_terrain(&mut doc.world.registry, &block);
            doc.mark_changed();
            doc.world.registry.terrain(id).name.clone()
        }
    }
}

/// A list to pick from, with a search field: the terrains of the map, then
/// the game's whole blocks that are not terrains yet.
pub(crate) fn ground_picker(ui: &mut Ui, id: egui::Id, reg: &Registry, blocks: &mut BlockLibrary, current: Option<&str>) -> Option<Pick> {
    let pal = Palette::of(ui);
    let ctx = ui.ctx().clone();
    ui.set_max_width(300.0);
    let mut search: String = ctx.data(|d| d.get_temp(id)).unwrap_or_default();
    let edit = ui.add(
        egui::TextEdit::singleline(&mut search)
            .hint_text(tr!("{glyph}  Search terrains and blocks", glyph = theme::glyph(Icon::Search)))
            .desired_width(ui.available_width().min(280.0)),
    );
    edit.request_focus();
    let needle = search.trim().to_lowercase();
    let matches = |name: &str, id: &str| needle.is_empty() || name.to_lowercase().contains(&needle) || id.contains(&needle);
    let mut picked = None;
    egui::ScrollArea::vertical().max_height(300.0).show(ui, |ui| {
        let entry = |ui: &mut Ui, selected: bool, name: &str, tip: &str, icon: &mut dyn FnMut(&mut Ui, Rect)| {
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), Sense::hover());
                icon(ui, r);
                ui.selectable_label(selected, name).on_hover_text(tip).clicked()
            })
            .inner
        };
        ui.label(RichText::new(tr!("IN THIS MAP")).size(10.5).bold().color(pal.weak));
        for (k, t) in reg.terrains.iter().enumerate() {
            if matches(&t.name, &t.group.to_lowercase()) && entry(ui, current == Some(t.name.as_str()), &t.name, &t.group, &mut |ui, r| terrain_icon(ui, r, reg, k as u16, blocks)) {
                picked = Some(Pick::Terrain(t.name.clone()));
            }
        }
        if let Some(cat) = blocks.catalogue.clone() {
            ui.add_space(4.0);
            ui.label(RichText::new(tr!("BLOCKS OF MINECRAFT {version}", version = cat.version)).size(10.5).bold().color(pal.weak));
            // Searching shows every match; otherwise the first ones.
            let limit = if needle.is_empty() { 120 } else { usize::MAX };
            let list = cat.blocks.iter().filter(|b| b.full_cube && matches(&b.name, &b.id) && reg.terrain_by_name(&b.name).is_none());
            for b in list.take(limit) {
                let texture = blocks.icon(&ctx, b);
                let icon = &mut |ui: &mut Ui, r: Rect| {
                    if let Some(t) = texture {
                        ui.painter().image(t, r, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                    }
                };
                if entry(ui, false, &b.name, &b.id, icon) {
                    picked = Some(Pick::Block(b.clone()));
                }
            }
        } else {
            hint(ui, tr!("Every block of the game shows here once Blocks has read it."));
        }
    });
    ctx.data_mut(|d| d.insert_temp(id, search));
    if picked.is_some() {
        ctx.data_mut(|d| d.remove::<String>(id));
        ui.close();
    }
    picked
}

/// Patterns to start from, those saved, and saving this one.
fn pattern_presets(ui: &mut Ui, ps: &mut crate::tools::PatternSettings) {
    use crate::tools::PatternPreset;
    let current = ps.preset_name();
    let (mut load, mut forget) = (None, None);
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Preset")));
        let width = (ui.available_width() - 40.0).clamp(90.0, 190.0);
        crate::widgets::combo("noise preset").selected_text(current.clone().unwrap_or_else(|| tr!("Custom").to_owned())).width(width).show_ui(ui, |ui| {
            for p in PatternPreset::built_in() {
                if ui.selectable_label(current.as_deref() == Some(p.name.as_str()), &p.name).clicked() {
                    load = Some(p);
                }
            }
            if !ps.saved.is_empty() {
                ui.separator();
                for (k, p) in ps.saved.iter().enumerate() {
                    ui.horizontal(|ui| {
                        if ui.selectable_label(current.as_deref() == Some(p.name.as_str()), &p.name).clicked() {
                            load = Some(p.clone());
                        }
                        if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Forget it")).clicked() {
                            forget = Some(k);
                        }
                    });
                }
            }
        });
        let name_id = egui::Id::unique("noise preset name");
        ui.menu_button(theme::glyph(Icon::Save).to_string(), |ui| {
            ui.set_min_width(220.0);
            ui.label(RichText::new(tr!("Save this pattern as")).size(12.0));
            let mut name: String = ui.ctx().data(|d| d.get_temp(name_id)).unwrap_or_else(|| current.clone().unwrap_or_default());
            let edit = ui.add(egui::TextEdit::singleline(&mut name).hint_text(tr!("Its name")).desired_width(200.0));
            let enter = edit.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            let taken = ps.saved.iter().any(|p| p.name.eq_ignore_ascii_case(name.trim()));
            let label = if taken { tr!("Replace it") } else { tr!("Save") };
            if (ui.add_enabled(!name.trim().is_empty(), egui::Button::new(label)).clicked() || enter) && !name.trim().is_empty() {
                ps.save(&name);
                ui.ctx().data_mut(|d| d.remove::<String>(name_id));
                ui.close();
            } else {
                ui.ctx().data_mut(|d| d.insert_temp(name_id, name));
            }
        })
        .response
        .on_hover_text(tr!("Save this pattern, to paint with it again"));
    });
    if let Some(p) = load {
        ps.load(&p);
    }
    if let Some(k) = forget {
        ps.saved.remove(k);
    }
}

/// Massif's noise painter: the noise, the blocks it lays out and their
/// shares, pictures of the result, and painting it over the whole map.
fn pattern_settings(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary) {
    use massif_core::noise::Basis;
    use massif_core::pattern::{MAX_OCTAVES, MAX_STRETCH, PatternKind};
    let pal = Palette::of(ui);
    hint(ui, tr!("Blocks mixed by a noise, each over its share of the ground. Neighbours in the list meet on the ground."));
    ui.add_space(2.0);
    pattern_presets(ui, &mut tools.pattern);

    // ---- the noise -------------------------------------------------------
    section(ui, Icon::Waves, tr!("Noise"));
    let p = &mut tools.pattern.pattern;
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Shape"))).on_hover_text(tr!("Patches, thin veins, blobs with creases, cells, or wavering stripes"));
        crate::widgets::combo("pattern kind").selected_text(crate::i18n::tr_name(p.kind.label())).width(120.0).show_ui(ui, |ui| {
            for k in PatternKind::ALL {
                ui.selectable_value(&mut p.kind, k, crate::i18n::tr_name(k.label()));
            }
        });
    });
    let fractal = p.kind.is_fractal();
    ui.add_enabled_ui(fractal, |ui| {
        ui.horizontal(|ui| {
            ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Noise"))).on_hover_text(tr!("Simplex: round shapes · Perlin: a little squarer · Value: soft, blocky blobs"));
            crate::widgets::combo("pattern noise").selected_text(crate::i18n::tr_name(p.noise.label())).width(120.0).show_ui(ui, |ui| {
                for b in Basis::ALL {
                    ui.selectable_value(&mut p.noise, b, crate::i18n::tr_name(b.label()));
                }
            });
        });
    });
    tip_row(ui, tr!("Size"), tr!("The size of the largest shapes"), |ui| {
        ui.add(crate::widgets::Field::new(&mut p.scale, 2.0..=600.0).logarithmic(true).suffix(tr!(" blocks")).max_decimals(0))
    });
    ui.add_enabled_ui(fractal, |ui| {
        tip_row(ui, tr!("Octaves"), tr!("Layers of finer and finer detail over the largest shapes"), |ui| {
            ui.add(crate::widgets::Field::new(&mut p.octaves, 1..=MAX_OCTAVES))
        });
        tip_row(ui, tr!("Lacunarity"), tr!("How much finer each layer of detail is than the one before"), |ui| {
            ui.add(crate::widgets::Field::new(&mut p.lacunarity, 1.0..=4.0).max_decimals(2))
        });
        tip_row(ui, tr!("Gain"), tr!("How much each layer of detail counts: low, smooth edges; high, ragged ones"), |ui| {
            ui.add(crate::widgets::Field::new(&mut p.gain, 0.0..=1.0).max_decimals(2))
        });
    });
    tip_row(ui, tr!("Stretch"), tr!("Draws the shapes out along the direction: streaks, bands"), |ui| {
        ui.add(crate::widgets::Field::new(&mut p.stretch, 1.0..=MAX_STRETCH).logarithmic(true).suffix("×").max_decimals(1))
    });
    ui.add_enabled_ui(p.stretch > 1.01 || p.kind == PatternKind::Stripes, |ui| {
        tip_row(ui, tr!("Direction"), tr!("The way the shapes run: 0° east–west, 90° north–south"), |ui| {
            ui.add(crate::widgets::Field::new(&mut p.angle, 0.0..=180.0).suffix("°").max_decimals(0))
        });
    });
    tip_row(ui, tr!("Soft edges"), tr!("Blocks of each side scattered into the other along the edges"), |ui| {
        ui.add(
            crate::widgets::Field::new(&mut p.softness, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
        )
    });
    ui.horizontal(|ui| {
        ui.add_sized([72.0, 18.0], egui::Label::new(tr!("Seed"))).on_hover_text(tr!("Another layout of the same pattern"));
        crate::widgets::drag(ui, egui::DragValue::new(&mut p.seed));
        if ui.small_button(theme::glyph(Icon::Dices).to_string()).on_hover_text(tr!("Another layout")).clicked() {
            p.seed = p.seed.wrapping_mul(1_103_515_245).wrapping_add(12_345) % 1_000_000_000;
        }
    });

    // ---- the blocks, in their order, with their shares -----------------
    section(ui, Icon::Blocks, tr!("Blocks"));
    let ps = &mut tools.pattern;
    let (mut moved, mut remove, mut replace) = (None, None, None);
    {
        let reg = &doc.world.registry;
        let name_w = (ui.available_width() * 0.36).clamp(64.0, 120.0);
        for k in 0..ps.terrains.len() {
            let name = ps.terrains[k].0.clone();
            let terrain = reg.terrain_by_name(&name);
            let row = ui
                .horizontal(|ui| {
                    // The icon is the handle to move a block up or down.
                    let drag_id = egui::Id::unique(("noise block", k));
                    ui.dnd_drag_source(drag_id, k, |ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), Sense::hover());
                        match terrain {
                            Some(t) => terrain_icon(ui, r, reg, t.0, blocks),
                            None => {
                                ui.painter().rect_stroke(r.shrink(3.0), 4, Stroke::new(1.2, pal.danger), StrokeKind::Inside);
                            }
                        }
                    })
                    .response
                    .on_hover_text(tr!("Drag it up or down the list: neighbours in the list meet on the ground"));
                    // Its name: click to put another block in its place.
                    let ink = if terrain.is_some() { pal.text } else { pal.danger };
                    let label = ui
                        .allocate_ui_with_layout(vec2(name_w, 22.0), egui::Layout::left_to_right(Align::Center), |ui| {
                            ui.set_width(name_w);
                            ui.add(egui::Label::new(RichText::new(&name).color(ink)).truncate().sense(Sense::click()))
                        })
                        .inner
                        .on_hover_cursor(egui::CursorIcon::PointingHand)
                        .on_hover_text(if terrain.is_some() { tr!("Change it") } else { tr!("Not a terrain of this map: change it") });
                    egui::Popup::menu(&label).show(|ui| {
                        if let Some(pick) = ground_picker(ui, egui::Id::unique(("noise block search", k)), reg, blocks, Some(&name)) {
                            replace = Some((k, pick));
                        }
                    });
                    let mut share = ps.share(k);
                    let field = ui.add(
                        crate::widgets::Field::new(&mut share, 0.0..=1.0)
                            .width((ui.available_width() - 26.0).max(60.0))
                            .custom_formatter(|v, _| format!("{:.0}%", v * 100.0))
                            .custom_parser(|s| s.trim_end_matches('%').trim().parse::<f64>().ok().map(|v| v / 100.0)),
                    );
                    if field.on_hover_text(tr!("Its share of the ground: the others share the rest")).changed() {
                        ps.set_share(k, share);
                    }
                    if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false).small()).on_hover_text(tr!("Take it out")).clicked() {
                        remove = Some(k);
                    }
                })
                .response;
            // A block dragged over a row goes there: a line shows where.
            if let Some(from) = row.dnd_hover_payload::<usize>()
                && *from != k
            {
                let y = if *from > k { row.rect.top() - 1.0 } else { row.rect.bottom() + 1.0 };
                ui.painter().hline(row.rect.x_range(), y, Stroke::new(2.0, pal.accent));
            }
            if let Some(from) = row.dnd_release_payload::<usize>() {
                moved = Some((*from, k));
            }
        }
    }
    if let Some((from, to)) = moved {
        ps.reorder(from, to);
    }
    if let Some(k) = remove {
        ps.remove(k);
    }
    if let Some((k, pick)) = replace {
        let name = picked_terrain(pick, doc);
        let ps = &mut tools.pattern;
        match ps.terrains.iter().position(|t| t.0.eq_ignore_ascii_case(&name)) {
            // Already in the list elsewhere: the two swap places.
            Some(j) if j != k => ps.terrains.swap(j, k),
            Some(_) => {}
            None => ps.terrains[k].0 = name,
        }
    }
    let mut added = None;
    ui.horizontal(|ui| {
        ui.menu_button(theme::with_icon(Icon::Plus, tr!("Add a block")), |ui| {
            if let Some(pick) = ground_picker(ui, egui::Id::unique("noise block add"), &doc.world.registry, blocks, None) {
                added = Some(pick);
            }
        });
        let selected = doc.world.registry.terrain(TerrainId(tools.terrain)).name.clone();
        if !tools.pattern.terrains.iter().any(|t| t.0 == selected)
            && ui.button(theme::with_icon(Icon::Pipette, &selected)).on_hover_text(tr!("Add the terrain chosen in Terrain or Blocks")).clicked()
        {
            added = Some(Pick::Terrain(selected));
        }
    });
    if let Some(pick) = added {
        let name = picked_terrain(pick, doc);
        tools.pattern.add(&name);
    }
    if tools.pattern.terrains.is_empty() {
        hint(ui, tr!("No block yet: add one."));
    }

    // ---- what it looks like ----------------------------------------------
    pattern_pictures(ui, tools, doc);

    // ---- over the whole map ----------------------------------------------
    ui.add_space(6.0);
    let ready = tools.pattern.sampler(&doc.world.registry).is_some();
    if ui
        .add_enabled(ready, egui::Button::new(theme::with_icon(Icon::Map, tr!("Paint the whole map"))))
        .on_hover_text(tr!("This noise over every column of the active level, in the background; one step to undo"))
        .clicked()
    {
        tools.pattern.paint_map = true;
    }
    hint(
        ui,
        if tools.filter.is_active() {
            tr!("Only where the options of Brush Settings let the brush act.")
        } else {
            tr!("Every column of the active level: Brush Settings ▸ Options limit it (only on grass, a height band…).")
        },
    );
    if let Some(n) = tools.pattern.done {
        hint(ui, &tr!("{n} columns changed over the whole map.", n));
    }
}

/// Pictures of a pattern: the ground it paints and its noise, side by side,
/// eight of its shapes across, and how its values spread between its
/// blocks.
fn pattern_pictures(ui: &mut Ui, tools: &ToolState, doc: &Document) {
    let pal = Palette::of(ui);
    let reg = &doc.world.registry;
    let Some(sampler) = (|| {
        // Made again only when the pattern or the colours change.
        let key = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            format!("{:?}{:?}", tools.pattern.pattern, tools.pattern.terrains).hash(&mut h);
            reg.terrains.iter().map(|t| t.color).collect::<Vec<_>>().hash(&mut h);
            h.finish()
        };
        let id = egui::Id::unique("pattern pictures");
        type Pictures = (u64, egui::TextureHandle, egui::TextureHandle, massif_core::pattern::Spread, Vec<Color32>);
        let cached: Option<Pictures> = ui.ctx().data(|d| d.get_temp(id));
        if let Some(c) = cached.filter(|c| c.0 == key) {
            return Some(c);
        }
        let sampler = tools.pattern.sampler(reg)?;
        let n = 128usize;
        let span = (tools.pattern.pattern.scale * 8.0).max(32.0) / n as f32;
        let at = |c: usize| (((c % n) as f32 * span) as i32, ((c / n) as f32 * span) as i32);
        let ground: Vec<u8> = (0..n * n)
            .flat_map(|c| {
                let (x, z) = at(c);
                sampler.terrain(x, z).map_or([60, 60, 60], |t| reg.terrain(t).color)
            })
            .collect();
        let noise: Vec<u8> = (0..n * n)
            .flat_map(|c| {
                let (x, z) = at(c);
                let v = (sampler.rank(x, z) * 255.0) as u8;
                [v, v, v]
            })
            .collect();
        let load = |name: &str, rgb: &[u8]| ui.ctx().load_texture(name, egui::ColorImage::from_rgb([n, n], rgb), egui::TextureOptions::NEAREST);
        let colours = sampler.cumulative().iter().map(|(t, _)| theme::rgb(reg.terrain(*t).color)).collect();
        let pictures: Pictures = (key, load("pattern ground", &ground), load("pattern noise", &noise), sampler.spread(64), colours);
        ui.ctx().data_mut(|d| d.insert_temp(id, pictures.clone()));
        Some(pictures)
    })() else {
        return;
    };
    let (_, ground, noise, spread, colours) = sampler;
    ui.add_space(6.0);
    let side = ((ui.available_width() - 8.0) / 2.0).clamp(60.0, 150.0);
    ui.horizontal(|ui| {
        for (texture, caption, tip) in [(&ground, tr!("Ground"), tr!("What it paints")), (&noise, tr!("Noise"), tr!("Its noise, dark to light: the blocks follow it in the order of the list"))] {
            ui.vertical(|ui| {
                ui.add(egui::Image::new(texture).fit_to_exact_size(vec2(side, side)).corner_radius(6)).on_hover_text(tip);
                ui.label(RichText::new(caption).size(11.0).color(pal.weak));
            });
        }
    });
    // The spread of its values, each step in the colour of its block.
    let width = 2.0 * side + ui.spacing().item_spacing.x;
    let (rect, response) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 4, pal.header);
    let bins = spread.density.len();
    for (k, d) in spread.density.iter().enumerate() {
        let (x0, x1) = (rect.left() + rect.width() * k as f32 / bins as f32, rect.left() + rect.width() * (k + 1) as f32 / bins as f32);
        let middle = (k as f32 + 0.5) / bins as f32;
        let band = spread.limits.iter().position(|l| middle < *l).unwrap_or(spread.limits.len().saturating_sub(1));
        let colour = colours.get(band).copied().unwrap_or(pal.weak);
        let top = rect.bottom() - (rect.height() - 3.0) * d;
        painter.rect_filled(Rect::from_min_max(pos2(x0, top), pos2(x1 + 0.5, rect.bottom())), 0, colour);
    }
    for l in &spread.limits[..spread.limits.len().saturating_sub(1)] {
        let x = rect.left() + rect.width() * l;
        painter.vline(x, rect.y_range(), Stroke::new(1.0, pal.text.gamma_multiply(0.55)));
    }
    response.on_hover_text(tr!("How often each value of the noise comes, low to high, in the colour of the block it gives"));
    hint(ui, &tr!("{value:.0} blocks across", value = tools.pattern.pattern.scale * 8.0));
}

/// The rules of the automatic biomes: how the land is read, and the biome
/// of each kind of it.
pub fn land_rules(ui: &mut Ui, rules: &mut massif_core::auto::BiomeRules, reg: &Registry) {
    use massif_core::auto::Land;
    let pal = Palette::of(ui);
    section(ui, Icon::Ruler, tr!("Reading the land"));
    egui::Grid::new("land thresholds").num_columns(4).spacing([10.0, 4.0]).show(ui, |ui| {
        let blocks = |ui: &mut Ui, label: &str, v: &mut f32, hover: &str| {
            ui.label(label).on_hover_text(hover);
            crate::widgets::drag(ui, egui::DragValue::new(v).range(0.0..=384.0).speed(0.5).max_decimals(0).suffix(tr!(" blocks")));
        };
        blocks(ui, tr!("Deep water from"), &mut rules.deep_water, tr!("Water at least this deep is deep water"));
        ui.label(tr!("Rivers up to")).on_hover_text(tr!("Water with a bank on both sides within this width is a river"));
        crate::widgets::drag(ui, egui::DragValue::new(&mut rules.river_width).range(4..=64).suffix(tr!(" blocks wide")));
        ui.end_row();
        blocks(ui, tr!("Shores up to"), &mut rules.shore_height, tr!("Ground at most this high over the water beside it is its shore"));
        ui.label(tr!("Cliffs from")).on_hover_text(tr!("Slopes at least this steep are rock, whatever grows on them"));
        crate::widgets::drag(ui, egui::DragValue::new(&mut rules.steep).range(10.0..=85.0).speed(0.5).max_decimals(0).suffix("°"));
        ui.end_row();
        blocks(ui, tr!("Hills from"), &mut rules.hills, tr!("Height over the sea where the hills begin"));
        blocks(ui, tr!("Mountains from"), &mut rules.mountains, tr!("Height over the sea where the mountains begin"));
        ui.end_row();
        blocks(ui, tr!("Peaks from"), &mut rules.peaks, tr!("Height over the sea where the peaks begin"));
        ui.end_row();
    });
    rules.mountains = rules.mountains.max(rules.hills);
    rules.peaks = rules.peaks.max(rules.mountains);
    section(ui, Icon::Trees, tr!("The biome of each kind of land"));
    egui::Grid::new("land biomes").num_columns(2).spacing([14.0, 4.0]).striped(true).show(ui, |ui| {
        for land in Land::ALL {
            ui.label(crate::i18n::tr_name(land.label())).on_hover_text(crate::i18n::tr_name(land.hint()));
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
                ui.label(RichText::new(crate::i18n::tr_name(land.hint())).size(11.0).color(pal.weak));
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
        ui.label(tr!("Cliffs from")).on_hover_text(tr!("Slopes at least this steep get the cliffs' terrain"));
        crate::widgets::drag(ui, egui::DragValue::new(&mut rules.steep).range(10.0..=85.0).speed(0.5).max_decimals(0).suffix("°"));
        ui.add_space(12.0);
        crate::widgets::checkbox(ui, &mut rules.read_the_land, tr!("where the biome is automatic, paint as the land is"))
            .on_hover_text(tr!("Unset biomes count as automatic"));
    });
    ui.add_space(4.0);
    egui::Grid::new("biome paint").num_columns(4).spacing([10.0, 4.0]).striped(true).show(ui, |ui| {
        for title in [tr!("Biome"), tr!("Ground"), tr!("Cliffs"), tr!("Under water")] {
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
                let shown = if current.is_empty() { tr!("— as it is").to_owned() } else { current.clone() };
                crate::widgets::combo(("biome paint", &biome.id, k)).selected_text(shown).width(128.0).show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_empty(), tr!("— as it is")).clicked() {
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
        .on_hover_text(tr!("Drag to turn (north up)"));
}

/// One of the two height limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Limit {
    Above,
    Below,
}

/// The brush's height limits, in one well: the land of the map seen from the
/// side with the limits across it, then the limits turned on and set.
fn height_limits(ui: &mut Ui, f: &mut massif_core::filter::Filter, heights: Option<&crate::doc::Heights>, min_y: i32, max_y: i32, sea: i32) {
    let top = max_y - 1;
    crate::widgets::well(ui, |ui| {
        land_profile(ui, f, heights, min_y, max_y, sea);
        crate::widgets::row(ui, tr!("At or above"), |ui| {
            let mut on = f.above.is_some();
            if crate::widgets::switch(ui, &mut on).changed() {
                // From the sea up, or under the other limit.
                f.above = on.then(|| f.below.map_or(sea, |b| b.min(sea)).clamp(min_y, top));
            }
            ui.add_space(6.0);
            let mut v = f.above.unwrap_or(sea);
            if ui.add_enabled(on, crate::widgets::Field::new(&mut v, min_y..=top).width(64.0)).changed() {
                f.above = Some(f.below.map_or(v, |b| v.min(b)));
            }
        });
        crate::widgets::row(ui, tr!("At or below"), |ui| {
            let mut on = f.below.is_some();
            if crate::widgets::switch(ui, &mut on).changed() {
                // A little over the sea, or over the other limit.
                f.below = on.then(|| f.above.map_or(sea + 24, |a| a.max(sea + 24)).clamp(min_y, top));
            }
            ui.add_space(6.0);
            let mut v = f.below.unwrap_or(sea + 24);
            if ui.add_enabled(on, crate::widgets::Field::new(&mut v, min_y..=top).width(64.0)).changed() {
                f.below = Some(f.above.map_or(v, |a| v.max(a)));
            }
        });
        if f.above.is_some() && f.below.is_some() {
            crate::widgets::row(ui, tr!("Acts"), |ui| {
                if let Some(k) = crate::widgets::button_group(ui, "height band side", &[tr!("Between"), tr!("Outside")], &[!f.beyond, f.beyond]) {
                    f.beyond = k == 1;
                }
            });
        }
        crate::widgets::row(ui, &tr!("Soften the edges ({fade} blocks)", fade = massif_core::filter::FADE), |ui| {
            ui.add_enabled_ui(f.above.is_some() || f.below.is_some(), |ui| crate::widgets::switch(ui, &mut f.fade));
        });
    });
    let caption = match (f.above, f.below) {
        (None, None) => tr!("Acts at every height").to_owned(),
        (Some(a), None) => tr!("Acts at {a} and above", a),
        (None, Some(b)) => tr!("Acts at {b} and below", b),
        (Some(a), Some(b)) if f.beyond => tr!("Acts below {a} and above {b}", a, b),
        (Some(a), Some(b)) => tr!("Acts from {a} to {b}", a, b),
    };
    let share = heights.filter(|h| h.total > 0 && (f.above.is_some() || f.below.is_some())).map(|h| tr!(" · {value:.0} % of the map", value = h.share(f) * 100.0)).unwrap_or_default();
    ui.add_space(2.0);
    hint(ui, &tr!("{caption}{share}", caption, share));
}

/// A convex polygon added to `mesh` as a fan of triangles, each corner
/// coloured by `colour`.
fn fan(mesh: &mut egui::Mesh, corners: &[egui::Pos2], colour: impl Fn(egui::Pos2) -> Color32) {
    if corners.len() < 3 {
        return;
    }
    let i = mesh.vertices.len() as u32;
    for &p in corners {
        mesh.colored_vertex(p, colour(p));
    }
    for k in 1..corners.len() as u32 - 1 {
        mesh.add_triangle(i, i + k, i + k + 1);
    }
}

/// The part of a convex polygon between the heights `top` and `bottom` of
/// the screen (`top` above).
fn clip_between(corners: &[egui::Pos2], top: f32, bottom: f32) -> Vec<egui::Pos2> {
    // Keeping the side of a level where `keep` holds, one edge at a time.
    let clip = |points: Vec<egui::Pos2>, level: f32, keep: &dyn Fn(f32) -> bool| {
        let mut out = Vec::with_capacity(points.len() + 2);
        for (k, &p) in points.iter().enumerate() {
            let q = points[(k + 1) % points.len()];
            if keep(p.y) {
                out.push(p);
            }
            if keep(p.y) != keep(q.y) {
                let t = (level - p.y) / (q.y - p.y);
                out.push(pos2(p.x + (q.x - p.x) * t, level));
            }
        }
        out
    };
    if corners.len() < 3 || top >= bottom {
        return Vec::new();
    }
    let under = clip(corners.to_vec(), top, &|y| y >= top);
    if under.len() < 3 {
        return Vec::new();
    }
    clip(under, bottom, &|y| y <= bottom)
}

/// `values`, spread evenly from one end to the other (NaN where there is
/// none), at `n` points spread the same way: the mean of the values around
/// each point when there are more values than points, else read between
/// the two nearest.
fn resample(values: &[f32], n: usize) -> Vec<f32> {
    let m = values.len();
    if m == 0 {
        return vec![f32::NAN; n];
    }
    (0..n)
        .map(|i| {
            let at = if n > 1 { i as f32 / (n - 1) as f32 } else { 0.5 } * m as f32 - 0.5;
            if m > n {
                let reach = 0.5 * m as f32 / (n - 1).max(1) as f32;
                let first = ((at - reach).ceil().max(0.0) as usize).min(m - 1);
                let last = ((at + reach).floor().max(0.0) as usize).clamp(first, m - 1);
                let (sum, count) = values[first..=last].iter().filter(|v| !v.is_nan()).fold((0.0, 0), |(s, c), v| (s + v, c + 1));
                if count > 0 { sum / count as f32 } else { f32::NAN }
            } else {
                let k = (at.floor().max(0.0) as usize).min(m - 1);
                let (p, q) = (values[k], values[(k + 1).min(m - 1)]);
                let t = (at - k as f32).clamp(0.0, 1.0);
                match (p.is_nan(), q.is_nan()) {
                    (false, false) => p + (q - p) * t,
                    (false, true) => p,
                    (true, false) => q,
                    (true, true) => f32::NAN,
                }
            }
        })
        .collect()
}

/// The map seen from the side, cut from west to east through its highest
/// column: the land in grey, the sea in blue, and in the accent the land at
/// the heights the brush acts on, between the limits' lines. Each limit is a
/// line across, dragged up or down (Shift: finely), its height on the scale
/// at the right; pressed elsewhere, the nearest limit comes to the pointer.
fn land_profile(ui: &mut Ui, f: &mut massif_core::filter::Filter, heights: Option<&crate::doc::Heights>, min_y: i32, max_y: i32, sea: i32) {
    let pal = Palette::of(ui);
    let top = max_y - 1;
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 150.0), Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    let heights = heights.filter(|h| h.total > 0);
    let water = Color32::from_rgb(86, 132, 204);
    let small = egui::FontId::proportional(10.0);

    // The heights shown: the land's and the sea's, with a margin. The limits
    // do not stretch them: one beyond them waits at the edge.
    let (lo, hi) = heights.map_or((sea - 24, sea + 72), |h| (h.lo.min(sea), h.hi().max(sea)));
    let pad = ((hi - lo) / 10).max(4);
    let v0 = (lo - pad).max(min_y) as f32;
    let v1 = ((hi + pad).min(max_y) as f32).max(v0 + 16.0);
    // The scale on the right: as wide as its longest label.
    let sea_label = tr!("sea {sea}", sea);
    let widest = [sea_label.clone(), (v0 as i32).to_string(), (v1 as i32).to_string(), f.above.unwrap_or(0).to_string(), f.below.unwrap_or(0).to_string()]
        .into_iter()
        .map(|s| painter.layout_no_wrap(s, small.clone(), pal.text).size().x)
        .fold(0.0f32, f32::max);
    let plot = Rect::from_min_max(pos2(rect.left(), rect.top() + 14.0), pos2(rect.right() - widest - 18.0, rect.bottom() - 12.0));
    let label_x = plot.right() + 9.0;
    let y_of = |e: f32| plot.bottom() - (e - v0) / (v1 - v0) * plot.height();
    let e_at = |y: f32| v0 + (plot.bottom() - y) / plot.height() * (v1 - v0);
    // A limit's line: under the blocks at `above`, over those at `below`.
    let level = |l: Limit, v: i32| match l {
        Limit::Above => v as f32 - 0.5,
        Limit::Below => v as f32 + 0.5,
    };
    let line_y = |l: Limit, v: i32| y_of(level(l, v).clamp(v0, v1));
    // Where the heights are, for the tests.
    ui.data_mut(|d| d.insert_temp(egui::Id::unique("height profile"), (plot, v0, v1)));
    // The share of the map at or above each height, from the lowest.
    let from_top: Vec<f32> = heights.map_or_else(Vec::new, |h| {
        let mut shares = vec![0.0; h.counts.len() + 1];
        let mut sum = 0u64;
        for k in (0..h.counts.len()).rev() {
            sum += h.counts[k] as u64;
            shares[k] = (sum as f64 / h.total as f64) as f32;
        }
        shares
    });
    let lowest = heights.map_or(0, |h| h.lo);
    let share_from = |y: i32| from_top.get((y - lowest).max(0) as usize).copied().unwrap_or(0.0);

    // ---- dragging a limit ---------------------------------------------------
    let id = response.id.with("limit");
    let pointer = response.interact_pointer_pos().or(response.hover_pos());
    let nearest = |y: f32, f: &massif_core::filter::Filter| {
        [(Limit::Above, f.above), (Limit::Below, f.below)]
            .into_iter()
            .filter_map(|(l, v)| v.map(|v| (l, (line_y(l, v) - y).abs())))
            .min_by(|a, b| a.1.total_cmp(&b.1))
    };
    // The limit held, and the height its line is at, not yet rounded: the
    // one nearest to where the button went down.
    let mut held: Option<(Limit, f32)> = ui.data(|d| d.get_temp(id));
    if (response.drag_started() || response.clicked())
        && let Some(p) = ui.input(|i| i.pointer.press_origin()).or(pointer)
        && let Some((l, _)) = nearest(p.y, f)
    {
        let v = match l {
            Limit::Above => f.above,
            Limit::Below => f.below,
        };
        held = v.map(|v| (l, level(l, v)));
    }
    if (response.dragged() || response.clicked())
        && let (Some(p), Some((l, e))) = (pointer, held)
    {
        let fine = ui.input(|i| i.modifiers.shift) && !response.clicked();
        let e = if fine { e - response.drag_delta().y / plot.height() * (v1 - v0) * 0.1 } else { e_at(p.y) }.clamp(v0, v1);
        match l {
            Limit::Above => f.above = Some(f.below.map_or((e + 0.5).round() as i32, |b| ((e + 0.5).round() as i32).min(b)).clamp(min_y, top)),
            Limit::Below => f.below = Some(f.above.map_or((e - 0.5).round() as i32, |a| ((e - 0.5).round() as i32).max(a)).clamp(min_y, top)),
        }
        held = Some((l, e));
    }
    if response.drag_stopped() || response.clicked() || !(response.dragged() || response.is_pointer_button_down_on()) {
        held = None;
    }
    ui.data_mut(|d| match held {
        Some(h) => {
            d.insert_temp(id, h);
        }
        None => d.remove::<(Limit, f32)>(id),
    });
    let hot = held.map(|h| h.0).or_else(|| pointer.filter(|_| response.hovered()).and_then(|p| nearest(p.y, f)).filter(|(_, d)| *d < 7.0).map(|(l, _)| l));
    if hot.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    // ---- the scale ----------------------------------------------------------
    let grid = theme::mix(pal.well(), pal.border, 0.75);
    let span = v1 - v0;
    let step = [8, 16, 32, 64, 128].into_iter().find(|&s| span / s as f32 <= 6.0).unwrap_or(256);
    let ticks: Vec<(i32, f32)> = (((v0 / step as f32).ceil() as i32 * step)..=(v1 as i32))
        .step_by(step as usize)
        .map(|e| (e, y_of(e as f32)))
        .filter(|&(_, y)| y > plot.top() + 4.0 && y < plot.bottom() - 4.0)
        .collect();
    for &(_, y) in &ticks {
        painter.hline(plot.x_range(), y, Stroke::new(1.0, grid));
    }

    // ---- the land and the sea ---------------------------------------------
    // How much the brush acts at a height (0 to 1), as the filter has it but
    // over unrounded heights: from the limits' lines, softened over FADE
    // blocks across them when the edges are soft. Without limits, nothing is
    // shown in the accent.
    let half = if f.fade { massif_core::filter::FADE / 2.0 } else { 0.0 };
    let lines = [f.above.map(|a| level(Limit::Above, a)), f.below.map(|b| level(Limit::Below, b))];
    let weight = |e: f32| {
        let inside = |d: f32| if half > 0.0 { (d / (2.0 * half) + 0.5).clamp(0.0, 1.0) } else if d >= 0.0 { 1.0 } else { 0.0 };
        match (lines[0].map(|l| inside(e - l)), lines[1].map(|l| inside(l - e))) {
            (None, None) => 0.0,
            (Some(w), None) | (None, Some(w)) => w,
            (Some(a), Some(b)) if f.beyond => (1.0 - a).max(1.0 - b),
            (Some(a), Some(b)) => a.min(b),
        }
    };
    // The slabs of heights between the points where that bends, with the
    // weight at the bottom and at the top of each (taken just inside it, so
    // that a sharp limit is a step).
    let mut bends: Vec<f32> = lines.iter().flatten().flat_map(|&l| [l - half, l + half]).filter(|e| *e > v0 && *e < v1).chain([v0, v1]).collect();
    bends.sort_by(f32::total_cmp);
    bends.dedup();
    let slabs: Vec<(f32, f32, f32, f32)> = bends
        .windows(2)
        .map(|p| {
            let inner = (p[1] - p[0]) * 1e-3;
            (p[0], p[1], weight(p[0] + inner), weight(p[1] - inner))
        })
        .filter(|s| s.2 > 0.0 || s.3 > 0.0)
        .collect();
    let sea_level = sea as f32 - 0.5;
    let sea_y = y_of(sea_level);
    let rock = theme::mix(pal.well(), pal.text, if pal.dark { 0.17 } else { 0.12 });
    let ridge = theme::mix(pal.well(), pal.text, 0.42);
    let lit = theme::mix(pal.well(), pal.accent, 0.85);
    let sea_fill = theme::mix(pal.well(), water, if pal.dark { 0.32 } else { 0.25 });
    match heights {
        Some(h) => {
            // The cut, a point a column of the picture; where it holds no
            // land, the bottom of the picture, under the sea.
            let n = plot.width().round().max(2.0) as usize;
            let ground: Vec<egui::Pos2> = resample(&h.section, n)
                .into_iter()
                .enumerate()
                .map(|(i, e)| pos2(plot.left() + i as f32 / (n - 1) as f32 * plot.width(), y_of(if e.is_nan() { v0 } else { e.clamp(v0, v1) })))
                .collect();
            let mut mesh = egui::Mesh::default();
            let mut outlines: Vec<(Color32, Vec<egui::Pos2>)> = Vec::new();
            let mut surface: Vec<(f32, f32)> = Vec::new();
            for pair in ground.windows(2) {
                let (a, b) = (pair[0], pair[1]);
                let land = [a, b, pos2(b.x, plot.bottom()), pos2(a.x, plot.bottom())];
                fan(&mut mesh, &land, |_| rock);
                // The land at the heights the brush acts on.
                for &(e0, e1, w0, w1) in &slabs {
                    let piece = clip_between(&land, y_of(e1), y_of(e0));
                    fan(&mut mesh, &piece, |p| theme::mix(rock, lit, w0 + (w1 - w0) * ((e_at(p.y) - e0) / (e1 - e0)).clamp(0.0, 1.0)));
                }
                // The sea over the land under its level.
                if a.y > sea_y || b.y > sea_y {
                    let cross = || a.x + (b.x - a.x) * ((sea_y - a.y) / (b.y - a.y));
                    let (x0, x1) = match (a.y > sea_y, b.y > sea_y) {
                        (true, true) => (a.x, b.x),
                        (true, false) => (a.x, cross()),
                        _ => (cross(), b.x),
                    };
                    let shore = |x: f32| pos2(x, a.y + (b.y - a.y) * ((x - a.x) / (b.x - a.x)));
                    fan(&mut mesh, &[pos2(x0, sea_y), pos2(x1, sea_y), shore(x1), shore(x0)], |_| sea_fill);
                    match surface.last_mut() {
                        Some(run) if (run.1 - x0).abs() < 0.01 => run.1 = x1,
                        _ => surface.push((x0, x1)),
                    }
                }
                // Its outline, in the accent where its surface is at a height
                // the brush acts on.
                let line = theme::mix(ridge, pal.accent, weight(e_at((a.y + b.y) / 2.0)));
                match outlines.last_mut() {
                    Some((c, points)) if *c == line => points.push(b),
                    _ => outlines.push((line, vec![a, b])),
                }
            }
            painter.add(egui::Shape::mesh(mesh));
            for (c, points) in outlines {
                painter.add(egui::Shape::line(points, Stroke::new(1.5, c)));
            }
            for (x0, x1) in surface {
                painter.hline(x0..=x1, sea_y, Stroke::new(1.0, water));
            }
        }
        None => {
            painter.rect_filled(Rect::from_min_max(pos2(plot.left(), sea_y), plot.max), 0, sea_fill);
            painter.hline(plot.x_range(), sea_y, Stroke::new(1.0, water));
            painter.text(plot.center(), egui::Align2::CENTER_CENTER, tr!("Reading the heights of the map…"), egui::FontId::proportional(11.0), pal.weak);
        }
    }

    // ---- the limits ---------------------------------------------------------
    // Their lines across, their heights in pills on the scale, kept apart.
    let tag_h = 16.0;
    let mut tags: Vec<(Limit, i32, f32)> = [(Limit::Below, f.below), (Limit::Above, f.above)].into_iter().filter_map(|(l, v)| v.map(|v| (l, v, line_y(l, v)))).collect();
    for t in &mut tags {
        t.2 = t.2.clamp(rect.top() + tag_h / 2.0, rect.bottom() - tag_h / 2.0);
    }
    if let [upper, lower] = &mut tags[..] {
        let gap = tag_h + 2.0;
        if lower.2 - upper.2 < gap {
            let middle = ((upper.2 + lower.2) / 2.0).clamp(rect.top() + (tag_h + gap) / 2.0, rect.bottom() - (tag_h + gap) / 2.0);
            upper.2 = middle - gap / 2.0;
            lower.2 = middle + gap / 2.0;
        }
    }
    for &(l, v, tag_y) in &tags {
        let y = line_y(l, v);
        let lit = hot == Some(l);
        painter.hline(plot.left()..=plot.right() + 4.0, y, Stroke::new(if lit { 2.0 } else { 1.5 }, pal.accent));
        let galley = painter.layout_no_wrap(v.to_string(), small.clone(), pal.on_accent());
        let tag = Rect::from_min_size(pos2(plot.right() + 4.0, tag_y - tag_h / 2.0), vec2(galley.size().x + 10.0, tag_h));
        if lit {
            painter.rect_filled(tag.expand(2.5), theme::PILL, pal.accent.gamma_multiply(0.25));
        }
        painter.rect_filled(tag, theme::PILL, pal.accent);
        painter.galley(pos2(tag.left() + 5.0, tag.center().y - galley.size().y / 2.0), galley, pal.on_accent());
    }
    // The heights of the scale and the sea's, where no limit is.
    let free = |y: f32, room: f32| tags.iter().all(|t| (t.2 - y).abs() >= room);
    let sea_shown = free(sea_y, 14.0);
    if sea_shown {
        painter.text(pos2(label_x, sea_y), egui::Align2::LEFT_CENTER, &sea_label, small.clone(), theme::legible(water, pal.well(), 4.5));
    }
    for &(e, y) in &ticks {
        if free(y, 13.0) && (!sea_shown || (y - sea_y).abs() >= 13.0) {
            painter.text(pos2(label_x, y), egui::Align2::LEFT_CENTER, e.to_string(), small.clone(), pal.weak);
        }
    }

    // ---- under the pointer --------------------------------------------------
    // Its height across the land, and how much of the map is at or above it.
    if held.is_none()
        && hot.is_none()
        && let Some(p) = response.hover_pos().filter(|p| plot.contains(*p))
    {
        painter.add(egui::Shape::dashed_line(&[pos2(plot.left(), p.y), pos2(plot.right(), p.y)], Stroke::new(1.0, theme::mix(pal.well(), pal.text, 0.4)), 3.0, 3.0));
        let y = e_at(p.y).round() as i32;
        let much = heights.map(|_| tr!(" · {value:.0} % of the map at or above it", value = share_from(y) * 100.0)).unwrap_or_default();
        response.on_hover_text_at_pointer(tr!("Height {y}{much}", y, much));
    }
}

/// A list of terrains, biomes, water or land the filter matches, as chips,
/// with a menu to add more.
fn match_list(ui: &mut Ui, label: &str, list: &mut Option<Vec<massif_core::filter::Match>>, reg: &Registry, id: &str) {
    use massif_core::filter::Match;
    let pal = Palette::of(ui);
    let mut on = list.is_some();
    if crate::widgets::checkbox(ui, &mut on, label).changed() {
        *list = on.then(Vec::new);
    }
    let Some(items) = list else { return };
    let name = |m: &Match| match *m {
        Match::Terrain(t) => reg.terrain(TerrainId(t)).name.clone(),
        Match::Biome(b) => reg.biome(BiomeId(b)).name.clone(),
        Match::Water => tr!("Water").to_owned(),
        Match::Land => tr!("Dry land").to_owned(),
    };
    ui.horizontal_wrapped(|ui| {
        ui.add_space(20.0);
        let mut remove = None;
        for (i, m) in items.iter().enumerate() {
            let text = format!("{}  {}", name(m), theme::glyph(Icon::X));
            if ui.add(egui::Button::new(RichText::new(text).size(11.5)).corner_radius(10)).on_hover_text(tr!("Remove")).clicked() {
                remove = Some(i);
            }
        }
        if let Some(i) = remove {
            items.remove(i);
        }
        ui.menu_button(theme::with_icon(Icon::Plus, tr!("Add")), |ui| {
            ui.set_max_height(360.0);
            let add = |m: Match, items: &mut Vec<Match>| {
                if !items.contains(&m) {
                    items.push(m);
                }
            };
            if ui.button(tr!("Water")).clicked() {
                add(Match::Water, items);
            }
            if ui.button(tr!("Dry land")).clicked() {
                add(Match::Land, items);
            }
            ui.menu_button(tr!("Terrains"), |ui| {
                egui::ScrollArea::vertical().id_salt((id, "terrains")).max_height(320.0).show(ui, |ui| {
                    for (i, t) in reg.terrains.iter().enumerate() {
                        if ui.button(&t.name).clicked() {
                            add(Match::Terrain(i as u16), items);
                        }
                    }
                });
            });
            ui.menu_button(tr!("Biomes"), |ui| {
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
            ui.label(RichText::new(tr!("Add what the brush should act on")).size(11.0).color(pal.weak));
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
/// A block id typed by hand, for a block nothing in the panel finds (any
/// mod's, with its properties): painted as a terrain or grown as a cover.
/// A block the catalogue has keeps its textures and colour; another is
/// written as typed, in grey on the map. Returns what to say.
fn typed_block(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, cat: &massif_assets::Catalogue) -> Option<String> {
    let pal = Palette::of(ui);
    let parsed = massif_assets::parse_state(&blocks.typed);
    let mut pick: Option<bool> = None;
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut blocks.typed)
                .hint_text(tr!("Block id: mod:block[property=value]"))
                .desired_width((ui.available_width() - 168.0).max(80.0)),
        )
        .on_hover_text(tr!("Any block of the game or of a mod by its id, with its properties if it has any: for a block the list does not show"));
        let ok = parsed.is_ok();
        if ui.add_enabled(ok, egui::Button::new(tr!("As terrain")).small()).on_hover_text(tr!("Paint it as the ground")).clicked() {
            pick = Some(true);
        }
        if ui.add_enabled(ok, egui::Button::new(tr!("As cover")).small()).on_hover_text(tr!("Grow it over the ground")).clicked() {
            pick = Some(false);
        }
    });
    let state = match parsed {
        Ok(state) => state,
        Err(massif_assets::StateSyntax::Empty) => return None,
        Err(e) => {
            let text = match e {
                massif_assets::StateSyntax::BadId(id) => tr!("{id} is not a block id: letters, digits and _ - . / only, the mod's name before a colon.", id),
                massif_assets::StateSyntax::BadProperty(p) => tr!("{p} is not a property: write [name=value,name=value].", p),
                massif_assets::StateSyntax::Empty => String::new(),
            };
            ui.label(RichText::new(text).size(11.5).color(pal.danger));
            return None;
        }
    };
    let id = state.split('[').next().unwrap_or(&state).to_owned();
    let known = cat.block(&id);
    let problem = cat.check_state(&state).err();
    let note = match (&problem, known) {
        (Some(massif_assets::materials::StateProblem::UnknownBlock(b)), _) => (tr!("{b} is not a block of the game, by its mod's list.", b), pal.danger),
        (Some(massif_assets::materials::StateProblem::UnknownProperty { block, property }), _) => (tr!("{block} has no property {property}.", block, property), pal.danger),
        (Some(massif_assets::materials::StateProblem::BadValue { block, property, value }), _) => (tr!("{block}: {property} cannot be {value}.", block, property, value), pal.danger),
        (None, Some(b)) => (tr!("{name}, of the blocks read.", name = b.name), pal.weak),
        (None, None) => (tr!("Not among the blocks read: written as typed, in grey on the map. The game needs its mod to have it.").to_owned(), pal.weak),
    };
    ui.label(RichText::new(note.0).size(11.5).color(note.1));
    let terrain = pick?;
    doc.mark_changed();
    let reg = &mut doc.world.registry;
    let plain = !state.contains('[');
    let name = known.map_or_else(|| state.clone(), |b| b.name.clone());
    let color = known.map_or([128, 128, 128], |b| if terrain || b.full_cube { b.color } else { b.side_color });
    if terrain {
        let t = match known.filter(|b| plain && b.full_cube) {
            Some(b) => massif_assets::block_terrain(reg, b),
            None => massif_assets::state_terrain(reg, &state, &name, color),
        };
        tools.terrain = t.0;
        if tools.tool != Tool::PaintTerrain {
            tools.select(Tool::PaintTerrain);
        }
        Some(tr!("Painting {name}", name))
    } else {
        tools.cover = match known.filter(|_| plain) {
            Some(b) => block_cover(reg, b),
            None => {
                let existing = reg.covers.iter().position(|c| !c.removed && c.plants.len() == 1 && c.plants[0].0 == state);
                match existing {
                    Some(k) => k as u16 + 1,
                    None => reg.add_cover(massif_core::registry::CoverDef::new(&name, color, &[(&state, 1)], 0.5)),
                }
            }
        };
        if tools.tool != Tool::Cover {
            tools.select(Tool::Cover);
        }
        Some(tr!("Painting {name} as a ground cover", name))
    }
}

pub fn blocks_panel(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, blocks: &mut BlockLibrary, jar: &mut Option<std::path::PathBuf>, mods: &mut Vec<std::path::PathBuf>) -> Option<String> {
    let pal = Palette::of(ui);
    let ctx = ui.ctx().clone();
    let mut feedback = None;

    // Where the blocks come from: a version of the game installed here.
    ui.horizontal(|ui| {
        let current = blocks.catalogue.as_ref().map(|c| (c.version.clone(), c.source.clone()));
        let label = match &current {
            Some((v, _)) => tr!("Minecraft {v}", v),
            None if blocks.is_loading() => tr!("Reading the game…").to_owned(),
            None => tr!("No game found").to_owned(),
        };
        crate::widgets::combo("minecraft version")
            .selected_text(label)
            .show_ui(ui, |ui| {
                for j in blocks.jars.clone() {
                    let selected = current.as_ref().is_some_and(|(_, p)| *p == j.path);
                    if ui.selectable_label(selected, tr!("Minecraft {version}", version = j.version)).on_hover_text(j.path.display().to_string()).clicked() {
                        *jar = Some(j.path.clone());
                        blocks.load(&ctx, jar.clone(), mods.clone());
                    }
                }
                if blocks.jars.is_empty() {
                    ui.label(RichText::new(tr!("No version found in the usual places")).color(pal.weak));
                }
            });
        if ui
            .button(theme::with_icon(Icon::FolderOpen, tr!("Choose…")))
            .on_hover_text(tr!("Another version's client jar"))
            .clicked()
            && let Some(path) = rfd::FileDialog::new().set_title(tr!("Choose the Minecraft jar")).add_filter(tr!("Minecraft"), &["jar"]).pick_file()
        {
            *jar = Some(path);
            blocks.load(&ctx, jar.clone(), mods.clone());
        }
        if blocks.is_loading() {
            crate::loader::loader(ui, 16.0);
        }
    });
    // The mods whose blocks are painted with too, each a chip that takes it
    // out: their jars, the resource packs and the lists of blocks read with
    // them, chosen one by one or as a modpack's instance.
    let mut reload = false;
    let mut instance: Option<massif_assets::instance::Instance> = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
        ui.label(RichText::new(tr!("Mods")).color(pal.weak));
        let read = blocks.catalogue.as_ref().map(|c| c.mods.clone()).unwrap_or_default();
        for path in mods.clone() {
            let info = read.iter().find(|m| m.path == path);
            let name = info.map_or_else(|| path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default(), |m| m.name.clone());
            let tip = match info {
                Some(m) if m.kind == massif_assets::SourceKind::List => trn!("A list of blocks made for WorldPainter: the states of {n} block · {path}", "A list of blocks made for WorldPainter: the states of {n} blocks · {path}", n = m.blocks, path = path.display()),
                Some(m) if m.kind == massif_assets::SourceKind::Pack => tr!("A resource pack: its textures over the mods' · {path}", path = path.display()),
                Some(m) => match &m.minecraft {
                    Some(v) => trn!("{n} block · made for Minecraft {v} · {path}", "{n} blocks · made for Minecraft {v} · {path}", n = m.blocks, v, path = path.display()),
                    None => trn!("{n} block · {path}", "{n} blocks · {path}", n = m.blocks, path = path.display()),
                },
                None => path.display().to_string(),
            };
            let chip = egui::Button::new(RichText::new(format!("{name}  {}", theme::glyph(Icon::X))).size(11.5)).corner_radius(theme::PILL);
            if ui.add(chip).on_hover_text(tip).clicked() {
                mods.retain(|p| *p != path);
                reload = true;
            }
        }
        if ui
            .small_button(theme::with_icon(Icon::Plus, tr!("Add mods…")))
            .on_hover_text(tr!("The jars of mods with blocks (Conquest Reforged, Chipped, Macaw's…), from the mods folder of your instance, the resource packs (.zip) that give them their textures, and the lists of blocks made for WorldPainter (.csv) that give their states: their blocks are painted and exported like the game's"))
            .clicked()
            && let Some(files) = rfd::FileDialog::new().set_title(tr!("Choose mods and resource packs")).add_filter(tr!("Mods, resource packs and block lists"), &["jar", "zip", "csv"]).pick_files()
        {
            for f in files {
                if !mods.contains(&f) {
                    mods.push(f);
                    reload = true;
                }
            }
        }
        if ui
            .small_button(theme::with_icon(Icon::FolderOpen, tr!("Instance…")))
            .on_hover_text(tr!("A modpack's instance (CurseForge, Prism, Modrinth…): its mods with blocks, its resource packs and its lists of blocks at once, over the version of the game it plays"))
            .clicked()
            && let Some(dir) = rfd::FileDialog::new().set_title(tr!("Choose a modpack's instance")).pick_folder()
        {
            match massif_assets::instance::Instance::read(&dir) {
                Some(found) => instance = Some(found),
                None => blocks.notice = Some(tr!("{folder} is not an instance: it has no mods folder.", folder = dir.display()).to_owned()),
            }
        }
    });
    // Conquest Reforged installed here: its blocks in one click, while they
    // are not read; each of its instances (Fabric, Forge…) on a row.
    let read_conquest = blocks.catalogue.as_ref().is_some_and(|c| c.mods.iter().any(|m| m.kind == massif_assets::SourceKind::Mod && m.namespaces.iter().any(|n| n == "conquest")));
    if !blocks.conquest.is_empty() && !read_conquest {
        ui.add_space(4.0);
        egui::Frame::new()
            .fill(pal.well())
            .stroke(Stroke::new(1.0, pal.border))
            .corner_radius(theme::GROUP_RADIUS)
            .inner_margin(egui::Margin::symmetric(10, 8))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new(tr!("Conquest Reforged is installed")).medium().size(12.5));
                for found in blocks.conquest.clone() {
                    ui.add_space(4.0);
                    // A row as high as its text: the instance, then its button.
                    ui.horizontal(|ui| {
                        let detail = match &found.minecraft {
                            Some(v) => tr!("{name} · Minecraft {v}", name = found.name, v),
                            None => found.name.clone(),
                        };
                        let text_width = (ui.available_width() - 64.0).max(80.0);
                        ui.allocate_ui_with_layout(vec2(text_width, 0.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
                            ui.add(egui::Label::new(RichText::new(detail).size(11.5).color(pal.weak)).wrap());
                        });
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.add(theme::primary_button(ui, tr!("Use"))).on_hover_text(tr!("Read its mods, lists of blocks and resource packs from this instance, over the version of the game it plays")).clicked() {
                                instance = Some(found.clone());
                            }
                        });
                    });
                }
            });
    }
    if let Some(found) = instance {
        blocks.use_instance(&ctx, &found, jar, mods);
    } else if reload {
        blocks.notice = None;
        blocks.load(&ctx, jar.clone(), mods.clone());
    }
    if let Some(notice) = &blocks.notice {
        ui.label(RichText::new(notice).color(pal.weak).size(11.5));
    }
    hint(
        ui,
        tr!("Your game's real blocks. Click one to paint with it."),
    );
    if let Some(e) = &blocks.error {
        ui.label(RichText::new(e).color(pal.danger).size(11.5));
    }
    let Some(cat) = blocks.catalogue.clone() else { return feedback };
    for (path, why) in &cat.skipped {
        let file = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        ui.label(RichText::new(tr!("{file} could not be read: {why}", file, why)).color(pal.danger).size(11.5));
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.add(
            egui::TextEdit::singleline(&mut blocks.search)
                .hint_text(tr!("{glyph}  Search {n} blocks", glyph = theme::glyph(Icon::Search), n = cat.blocks.len()))
                .desired_width((ui.available_width() - 110.0).max(80.0)),
        );
        crate::widgets::checkbox(ui, &mut blocks.all, tr!("All shapes"))
            .on_hover_text(tr!("Also non-cube blocks, and every shape of a mod's blocks (slabs, stairs, arches…)"));
    });
    if let Some(said) = typed_block(ui, tools, doc, blocks, &cat) {
        feedback = Some(said);
    }
    let needle = blocks.search.to_lowercase();
    let reg = &doc.world.registry;
    let current = massif_assets::terrain_block(reg, &cat, TerrainId(tools.terrain)).map(|b| b.id.clone());
    let mut picked: Option<&massif_assets::Block> = None;
    let sections = blocks.sections(&needle);
    // A grid per section, only its cells in view drawn: a mod's tabs hold
    // thousands of blocks.
    const CELL: f32 = 36.0;
    const GAP: f32 = 4.0;
    let clip = ui.clip_rect();
    for (k, section) in sections.iter().enumerate() {
        ui.add_space(6.0);
        ui.label(RichText::new(&section.title).size(10.5).bold().color(pal.weak));
        let width = ui.available_width();
        let cols = (((width + GAP) / (CELL + GAP)).floor() as usize).max(1);
        let rows = section.blocks.len().div_ceil(cols);
        let (grid, _) = ui.allocate_exact_size(vec2(width, rows as f32 * (CELL + GAP) - GAP), Sense::hover());
        if !grid.intersects(clip) {
            continue;
        }
        let first_row = (((clip.top() - grid.top()) / (CELL + GAP)).floor().max(0.0)) as usize;
        let last_row = ((((clip.bottom() - grid.top()) / (CELL + GAP)).ceil()) as usize).min(rows);
        for row in first_row..last_row {
            for col in 0..cols {
                let Some(&i) = section.blocks.get(row * cols + col) else { break };
                let block = &cat.blocks[i as usize];
                let rect = Rect::from_min_size(grid.min + vec2(col as f32 * (CELL + GAP), row as f32 * (CELL + GAP)), vec2(CELL, CELL));
                let response = ui.interact(rect, ui.scope_id().with(("block", k, i)), Sense::click());
                let selected = current.as_deref() == Some(block.id.as_str());
                let fill = if selected {
                    pal.accent_soft
                } else if response.hovered() {
                    pal.header
                } else {
                    Color32::TRANSPARENT
                };
                let stroke = if selected { Stroke::new(1.5, pal.accent) } else { Stroke::NONE };
                ui.painter().rect(rect, theme::TILE_RADIUS, fill, stroke, StrokeKind::Inside);
                if let Some(texture) = blocks.icon(&ctx, block) {
                    let tint = if block.full_cube { Color32::WHITE } else { Color32::from_white_alpha(150) };
                    ui.painter()
                        .image(texture, rect.shrink(4.0), Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), tint);
                }
                let tip = format!("{}\n{}{}", block.name, block.id, if block.full_cube { "" } else { tr!("\nNot a whole block: it grows over the ground, as a cover") });
                if response.on_hover_text(tip).clicked() {
                    picked = Some(block);
                }
            }
        }
    }
    if let Some(block) = picked {
        if block.full_cube {
            let terrain = massif_assets::block_terrain(&mut doc.world.registry, block);
            doc.mark_changed();
            tools.terrain = terrain.0;
            if tools.tool != Tool::PaintTerrain {
                tools.select(Tool::PaintTerrain);
            }
            feedback = Some(tr!("Painting {name}", name = block.name));
        } else {
            // Plants and the like grow over the ground: a cover of them.
            tools.cover = block_cover(&mut doc.world.registry, block);
            doc.mark_changed();
            if tools.tool != Tool::Cover {
                tools.select(Tool::Cover);
            }
            feedback = Some(tr!("Painting {name} as a ground cover", name = block.name));
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
    hint(ui, tr!("The block on top of the ground."));
    let solo_changed = crate::widgets::checkbox(ui, solo, tr!("Solo"))
        .on_hover_text(tr!("Fade every other terrain on the map"))
        .changed();
    ui.add(egui::TextEdit::singleline(search).hint_text(tr!("{glyph}  Search terrain", glyph = theme::glyph(Icon::Search))).desired_width(f32::INFINITY));
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
                painter.rect(rect, theme::TILE_RADIUS, fill, stroke, StrokeKind::Inside);
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
                tr!("all the way down").to_owned()
            } else if s.depth.0 == s.depth.1 {
                format!("{}", s.depth.0)
            } else {
                format!("{}–{}", s.depth.0, s.depth.1)
            };
            tr!("{join} ({depth})", join = names.join(" / "), depth)
        })
        .collect();
    if parts.is_empty() {
        parts.push(tr!("natural stone").into());
    }
    format!("{}: {}", def.name, parts.join(", then "))
}

// ----------------------------------------------------------------- biomes --

pub fn biome_panel(ui: &mut Ui, tools: &mut ToolState, reg: &Registry, search: &mut String) {
    hint(ui, tr!("Painted biomes override automatic."));
    ui.add(egui::TextEdit::singleline(search).hint_text(tr!("{glyph}  Search biomes", glyph = theme::glyph(Icon::Search))).desired_width(f32::INFINITY));
    let needle = search.to_lowercase();
    for (i, b) in reg.biomes.iter().enumerate() {
        if !needle.is_empty() && !b.name.to_lowercase().contains(&needle) && !b.id.contains(&needle) {
            continue;
        }
        ui.horizontal(|ui| {
            swatch(ui, theme::rgb(b.color));
            let r = ui.selectable_label(tools.biome as usize == i, &b.name);
            if r.on_hover_text(if b.id.is_empty() { tr!("Let the exporter decide") } else { &b.id }).clicked() {
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
        tr!("Solid adds rock, Carve removes it. Higher wins."),
    );
    ui.horizontal(|ui| {
        if ui.button(tr!("{glyph} Solid", glyph = theme::glyph(Icon::Plus))).on_hover_text(tr!("Add a level that adds matter")).clicked() {
            doc.add_level(LevelKind::Solid);
        }
        if ui.button(tr!("{glyph} Carve", glyph = theme::glyph(Icon::Plus))).on_hover_text(tr!("Add a level that removes matter")).clicked() {
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
                        .on_hover_text(if visible { tr!("Hide this level") } else { tr!("Show this level") })
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
                        let label = egui::Label::new(RichText::new(crate::i18n::tr_name(&name)).bold().color(if visible { pal.text } else { pal.weak }))
                            .sense(Sense::click())
                            .truncate();
                        let r = ui.add(label).on_hover_text(tr!("{label} level · {tiles} tiles · double-click to rename", label = crate::i18n::tr_name(kind.label()), tiles));
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
                        if ui.small_button(theme::glyph(Icon::Trash2).to_string()).on_hover_text(tr!("Delete this level")).clicked() {
                            action = Some((index, "delete"));
                        }
                        if index > 1 && ui.small_button("▼").on_hover_text(tr!("Move down")).clicked() {
                            action = Some((index, "down"));
                        }
                        if index + 1 < n && ui.small_button("▲").on_hover_text(tr!("Move up")).clicked() {
                            action = Some((index, "up"));
                        }
                    }
                    ui.label(RichText::new(crate::i18n::tr_name(kind.label())).size(10.5).color(pal.weak));
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
    section(ui, Icon::Globe, tr!("World"));
    egui::Grid::new("world info").num_columns(2).spacing([10.0, 4.0]).show(ui, |ui| {
        let meta = &mut doc.world.meta;
        let mut changed = false;
        ui.label(tr!("Name"));
        changed |= ui.text_edit_singleline(&mut meta.name).changed();
        ui.end_row();
        ui.label(tr!("Seed"));
        changed |= crate::widgets::drag(ui, egui::DragValue::new(&mut meta.seed)).changed();
        ui.end_row();
        ui.label(tr!("Sea level"));
        if crate::widgets::drag(ui, egui::DragValue::new(&mut meta.sea_level).range(bottom..=top - 1)).changed() {
            changed = true;
            affects_map = true;
        }
        ui.end_row();
        ui.label(tr!("Spawn"));
        ui.horizontal(|ui| {
            for v in &mut meta.spawn {
                changed |= crate::widgets::drag(ui, egui::DragValue::new(v)).changed();
            }
        });
        ui.end_row();
        if changed {
            doc.mark_changed();
        }
    });

    section(ui, Icon::Map, tr!("Map"));
    let dim = doc.dimension();
    match dim.bounds() {
        Some((lo, hi)) => {
            let (w, d) = (hi.x - lo.x + 1, hi.z - lo.z + 1);
            ui.label(tr!("{value} × {value2} blocks ({w} × {d} tiles)", value = w * 128, value2 = d * 128, w, d));
            ui.label(RichText::new(format!("x {} … {}, z {} … {}", lo.x * 128, (hi.x + 1) * 128 - 1, lo.z * 128, (hi.z + 1) * 128 - 1)).color(pal.weak));
        }
        None => {
            ui.label(tr!("Empty"));
        }
    }
    ui.label(RichText::new(tr!("Build height {min_y} … {value}", min_y = dim.min_y, value = dim.max_y - 1)).color(pal.weak));

    section(ui, Icon::Crosshair, tr!("Under the cursor"));
    match hover {
        Some((x, z)) => column_info(ui, doc, x, z),
        None => hint(ui, tr!("Hover the map to inspect a column.")),
    }

    section(ui, Icon::Activity, tr!("Memory"));
    let mut hot = 0;
    let mut cold = 0;
    egui::Grid::new("memory").num_columns(3).spacing([10.0, 2.0]).show(ui, |ui| {
        for level in &doc.dimension().levels {
            let s = level.stats();
            hot += s.hot_bytes;
            cold += s.cold_bytes;
            ui.label(&level.name);
            ui.label(RichText::new(tr!("{tiles} tiles", tiles = s.tiles)).color(pal.weak));
            ui.label(RichText::new(tr!("{hot_tiles} hot", hot_tiles = s.hot_tiles)).color(pal.weak));
            ui.end_row();
        }
    });
    ui.label(tr!("Decoded {human} · compressed {human2}", human = human(hot as u64), human2 = human(cold as u64)));
    ui.label(RichText::new(tr!("Map cache: {value} textures, {value2} grids, {value3} summaries", value = cache.0, value2 = cache.1, value3 = cache.2)).color(pal.weak));

    section(ui, Icon::Save, tr!("File"));
    match &doc.path {
        Some(p) => {
            ui.label(RichText::new(p.display().to_string()).color(pal.weak).size(11.0));
        }
        None => hint(ui, tr!("Not saved yet")),
    }
    if doc.is_modified() {
        ui.label(RichText::new(tr!("Unsaved changes")).color(pal.accent));
    }
    affects_map
}

fn column_info(ui: &mut Ui, doc: &Document, x: i32, z: i32) {
    let pal = Palette::of(ui);
    ui.label(RichText::new(tr!("Column {x}, {z}", x, z)).bold());
    let dim = doc.dimension();
    let Some(stack) = TileStack::new(dim, TilePos::containing(x, z), true) else {
        hint(ui, tr!("Outside the map"));
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
    rows.extend(fluids.iter().map(|&(a, b)| (a, tr!("{a} … {value}  Water", value = b - 1, a))));
    rows.sort_by_key(|r| std::cmp::Reverse(r.0));
    for (_, text) in rows {
        ui.label(RichText::new(text).monospace().size(11.5));
    }
    let mut sampler = ColumnSampler::default();
    let s = sampler.sample(&stack, cell);
    let biome = if s.biome == 0 {
        tr!("automatic").to_owned()
    } else {
        reg.biome(BiomeId(s.biome)).name.clone()
    };
    ui.label(RichText::new(tr!("Biome: {biome}", biome)).color(pal.weak));
    if let Some(cover) = reg.cover(s.cover) {
        ui.label(RichText::new(tr!("Cover: {name}", name = cover.name)).color(pal.weak));
    }
    let levels: Vec<String> = stack
        .layers()
        .filter(|(_, _, t)| t.present(cell))
        .map(|(i, _, _)| dim.levels[i].name.clone())
        .collect();
    ui.label(RichText::new(tr!("Levels: {join}", join = levels.join(", "))).color(pal.weak));
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

/// The page of an empty area: what it can show, as the menu of an area's
/// header offers it, centred. `shown` are the editors on screen.
pub fn empty_area(ui: &mut Ui, shown: &HashSet<EditorKind>, developer: bool) -> Option<EditorKind> {
    let columns = crate::picker::columns_for(ui.available_width() - 24.0);
    let width = crate::picker::width(columns);
    ui.add_space((ui.available_height() * 0.16).max(12.0));
    ui.vertical_centered(|ui| {
        ui.label(RichText::new(tr!("What should this area show?")).bold().size(15.0));
    });
    ui.add_space(14.0);
    let margin = ((ui.available_width() - width) / 2.0).max(0.0);
    ui.horizontal_top(|ui| {
        ui.add_space(margin);
        ui.vertical(|ui| crate::picker::page(ui, None, shown, developer, columns)).inner
    })
    .inner
}

#[cfg(test)]
mod tests {
    use super::*;
    use massif_core::generate::{GenParams, generate};

    /// What a noise settings panel holds while it is drawn.
    struct Noise {
        tools: ToolState,
        doc: Document,
        blocks: BlockLibrary,
    }

    /// The noise painter's settings, drawn offscreen in both themes, with
    /// the game's blocks when a game is installed. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn noise_settings_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let mut blocks = BlockLibrary::default();
        if let Some(jar) = massif_assets::find_jars().first() {
            blocks.catalogue = massif_assets::load(&jar.path, &jar.version).ok().map(std::sync::Arc::new);
        }
        let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let mut tools = ToolState::default();
        tools.select(Tool::PaintTerrain);
        tools.pattern.on = true;
        let state = Noise { tools, doc: Document::new(world, None, 1), blocks };
        for dark in [true, false] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 1180.0))
                .with_pixels_per_point(2.0)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut Noise| {
                        theme::install(ui.ctx());
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        let pal = Palette::of(ui);
                        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            tool_settings(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
                        });
                    },
                    Noise {
                        tools: state.tools.clone(),
                        doc: Document::new(state.doc.world.clone(), None, 1),
                        blocks: {
                            let mut b = BlockLibrary::default();
                            b.catalogue = state.blocks.catalogue.clone();
                            b
                        },
                    },
                );
            h.run_steps(4);
            let name = if dark { "noise-settings-dark.png" } else { "noise-settings-light.png" };
            h.render().unwrap().save(dir.join(name)).unwrap();
            if dark {
                // The list to add a block from, searched.
                use egui_kittest::kittest::Queryable;
                h.get_by_label_contains("Add a block").click();
                h.run_steps(3);
                h.get_by_role(egui::accesskit::Role::TextInput).type_text("dirt");
                h.run_steps(4);
                h.render().unwrap().save(dir.join("noise-picker.png")).unwrap();
            }
        }
    }

    /// The pockets' settings, drawn offscreen in both themes: two pocket
    /// layers, one picked, of gravel and andesite. Needs a GPU: runs with
    /// MASSIF_SNAPSHOT_DIR.
    #[test]
    fn pocket_settings_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let reg = &mut world.registry;
        let (gravel, andesite, clay) = (reg.intern_block("minecraft:gravel", [136, 126, 118]), reg.intern_block("minecraft:andesite", [136, 136, 136]), reg.intern_block("minecraft:clay", [160, 166, 179]));
        reg.add_pocket(massif_core::pockets::PocketDef::new("Clay lenses", [160, 166, 179], &[(clay, 1)], 0.08, 7.0, 30, 58));
        let gravel = reg.add_pocket(massif_core::pockets::PocketDef::new("Gravel and andesite", [136, 126, 118], &[(gravel, 3), (andesite, 1)], 0.2, 5.0, -40, 50));
        let mut tools = ToolState::default();
        tools.select(Tool::Pockets);
        tools.pocket_layer = gravel;
        for dark in [true, false] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 760.0))
                .with_pixels_per_point(2.0)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut Noise| {
                        theme::install(ui.ctx());
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        let pal = Palette::of(ui);
                        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            tool_settings(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
                        });
                    },
                    Noise { tools: tools.clone(), doc: Document::new(world.clone(), None, 1), blocks: BlockLibrary::default() },
                );
            h.run_steps(4);
            h.render().unwrap().save(dir.join(if dark { "pockets-settings-dark.png" } else { "pockets-settings-light.png" })).unwrap();
        }
    }

    /// New pockets makes a pocket layer of gravel, picks it to paint, and
    /// gives it a name of its own; its share is edited in the settings.
    #[test]
    fn new_pockets_are_made_and_picked() {
        use egui_kittest::kittest::Queryable;
        let world = generate(&GenParams { tiles_x: 1, tiles_z: 1, ..GenParams::default() });
        let mut tools = ToolState::default();
        tools.select(Tool::Pockets);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 900.0)).with_accessibility_check(false).build_ui_state(
            |ui, s: &mut Noise| {
                theme::install(ui.ctx());
                tool_settings(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
            },
            Noise { tools, doc: Document::new(world, None, 1), blocks: BlockLibrary::default() },
        );
        h.run_steps(2);
        h.get_by_label_contains("New pockets").click();
        h.run_steps(3);
        h.get_by_label_contains("New pockets").click();
        h.run_steps(3);
        let s = h.state();
        let names: Vec<&str> = s.doc.world.registry.pockets.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["Pockets 1", "Pockets 2"]);
        assert_eq!(s.tools.pocket_layer, 2, "the new one is picked");
        let reg = &s.doc.world.registry;
        assert_eq!(reg.pockets[1].blocks.iter().map(|&(b, w)| (reg.block(b).state.as_str(), w)).collect::<Vec<_>>(), [("minecraft:gravel", 1)]);
    }

    /// A combined layer's settings, drawn offscreen in both themes: an oak
    /// forest's terrain, biome, cover, trees and size. Needs a GPU: runs
    /// with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn combined_settings_render() {
        use massif_core::combined::{CombinedDef, Part};
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let mut world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        let reg = &mut world.registry;
        let oaks = reg.add_object_layer(massif_core::objects::ObjectLayerDef::new("Big oaks", &[], reg));
        let mut forest = CombinedDef::new("Oak forest", [74, 116, 52]);
        forest.set(Part::Terrain, Some((reg.terrain_by_name("Grass").unwrap().0, 1.0)));
        forest.set(Part::Biome, Some((reg.biome_by_id("minecraft:forest").unwrap().0, 1.0)));
        forest.set(Part::Cover, Some((reg.cover_by_name("Ferns").or(reg.cover_by_name("Grass")).unwrap(), 0.6)));
        forest.set(Part::Trees, Some((oaks, 1.0)));
        forest.set(Part::Size, Some((4, 0.5)));
        reg.add_combined(CombinedDef::new("Snowy sand", [230, 226, 200]));
        let n = reg.add_combined(forest);
        let mut tools = ToolState::default();
        tools.select(Tool::Combined);
        tools.combined = n;
        // Dark and light, then as narrow as Tool Settings in the window.
        for (dark, width) in [(true, 340.0), (false, 340.0), (true, 250.0)] {
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(width, 640.0))
                .with_pixels_per_point(2.0)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut Noise| {
                        theme::install(ui.ctx());
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        let pal = Palette::of(ui);
                        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            tool_settings(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
                        });
                    },
                    Noise { tools: tools.clone(), doc: Document::new(world.clone(), None, 1), blocks: BlockLibrary::default() },
                );
            h.run_steps(4);
            let name = match (dark, width < 300.0) {
                (_, true) => "combined-settings-narrow.png",
                (true, false) => "combined-settings-dark.png",
                (false, false) => "combined-settings-light.png",
            };
            h.render().unwrap().save(dir.join(name)).unwrap();
        }
    }

    /// New combined layer makes one that paints nothing yet, picks it, and
    /// names it apart from the others.
    #[test]
    fn new_combined_layers_are_made_and_picked() {
        use egui_kittest::kittest::Queryable;
        let world = generate(&GenParams { tiles_x: 1, tiles_z: 1, ..GenParams::default() });
        let mut tools = ToolState::default();
        tools.select(Tool::Combined);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 900.0)).with_accessibility_check(false).build_ui_state(
            |ui, s: &mut Noise| {
                theme::install(ui.ctx());
                tool_settings(ui, &mut s.tools, &mut s.doc, &mut s.blocks);
            },
            Noise { tools, doc: Document::new(world, None, 1), blocks: BlockLibrary::default() },
        );
        h.run_steps(2);
        for _ in 0..2 {
            h.get_by_label_contains("New combined layer").click();
            h.run_steps(3);
        }
        let s = h.state();
        let names: Vec<&str> = s.doc.world.registry.combined.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, ["Combined 1", "Combined 2"]);
        assert_eq!(s.tools.combined, 2, "the new one is picked");
        assert!(s.doc.world.registry.combined[1].parts.is_empty());
        h.get_by_label("Terrain");
        h.get_by_label("Pockets");
    }
}
