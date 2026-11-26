//! The viewer: the node viewed (or the active Map Output) lit as a render,
//! from above or in 3D, in clay or in its colours, a mask laid over the
//! relief it comes from; points and lines of the selected node edited on
//! it. Coarse first, then sharp.

use super::eval::Shown;
use super::panel::{Event, TerrainPanel};
use super::shade::Surface;
use super::sockets;
use crate::theme::{self, Icon, Palette};
use crate::widgets;
use egui::{Align2, Color32, CursorIcon, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2};
use massif_terrain::params::Value;
use massif_terrain::registry::{ParamKind, PortKind};

const BAR_H: f32 = 36.0;
/// The width the viewer's bar keeps on its right for Apply, Live and the
/// options.
const RIGHT_W: f32 = 236.0;

/// Where the picture goes in the viewer's card.
pub fn picture_rect(card: Rect) -> Rect {
    Rect::from_min_max(pos2(card.left() + 8.0, card.top() + BAR_H), pos2(card.right() - 8.0, card.bottom() - 8.0))
}

pub fn ui(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    // A height map saved: said once.
    if let Some(rx) = &p.heightmap {
        match rx.try_recv() {
            Ok(said) => {
                p.heightmap = None;
                let now = ui.input(|i| i.time);
                p.say(said.unwrap_or_else(|e| e), now);
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ui.ctx().request_repaint_after(std::time::Duration::from_millis(100)),
            Err(std::sync::mpsc::TryRecvError::Disconnected) => p.heightmap = None,
        }
    }
    let card = ui.max_rect();
    // Docked, the area's header names it: the commands from the left.
    let left = if p.docked {
        theme::PAD - 4.0
    } else {
        super::panel::title(ui, card, t!("Viewer"));
        74.0
    };
    let bar = Rect::from_min_max(pos2(card.left() + left, card.top() + 4.0), pos2(card.right() - theme::PAD, card.top() + BAR_H - 2.0));
    let mut bar_ui = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    controls(p, &mut bar_ui);
    let pic = picture_rect(card);
    p.look.back = { let c = pal.well(); [c.r(), c.g(), c.b()] };
    let painter = ui.painter_at(pic);
    painter.rect_filled(pic, theme::TILE_RADIUS, pal.well());
    if p.look.three_d {
        three_d(p, ui, pic, &pal);
        return;
    }

    // 2D: the window over the terrain keeps the picture's shape.
    let aspect = pic.width() / pic.height().max(1.0);
    let (ax, az) = if aspect >= 1.0 { (aspect, 1.0) } else { (1.0, 1.0 / aspect) };
    let span = p.look.span;
    p.look.window = [p.look.center[0] - span * ax / 2.0, p.look.center[1] - span * az / 2.0, p.look.center[0] + span * ax / 2.0, p.look.center[1] + span * az / 2.0];

    let resp = ui.interact(pic, ui.scope_id().with("terrain viewer"), Sense::click_and_drag());
    let shift = ui.input(|i| i.modifiers.shift);
    let editing_points = !p.look.three_d && editable(p).is_some();
    if resp.dragged() && (shift || !editing_points || !ui.ctx().is_being_dragged(ui.scope_id().with("viewer point"))) {
        let d = resp.drag_delta();
        if shift {
            // The sun turned, as in the Render workspace.
            p.look.sun[0] = (p.look.sun[0] + d.x * 0.5).rem_euclid(360.0);
            p.look.sun[1] = (p.look.sun[1] - d.y * 0.25).clamp(4.0, 85.0);
        } else if p.look.three_d {
            p.look.yaw = (p.look.yaw - d.x * 0.4).rem_euclid(360.0);
            p.look.pitch = (p.look.pitch + d.y * 0.3).clamp(8.0, 85.0);
        } else if !ui.ctx().dragged_id().is_some_and(|i| i != resp.id) {
            let per = span / pic.height().min(pic.width()).max(1.0);
            p.look.center[0] -= d.x * per;
            p.look.center[1] -= d.y * per;
        }
    }
    if resp.hovered() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y + (i.zoom_delta() - 1.0) * 200.0);
        if scroll != 0.0 {
            let f = (-scroll * 0.002).exp();
            if p.look.three_d {
                p.look.distance = (p.look.distance * f).clamp(0.05, 4.0);
            } else {
                // Zoom around the pointer.
                if let Some(q) = resp.hover_pos() {
                    let w = p.look.window;
                    let at = [w[0] + (w[2] - w[0]) * (q.x - pic.left()) / pic.width(), w[1] + (w[3] - w[1]) * (q.y - pic.top()) / pic.height()];
                    let new = (span * f).clamp(0.02, 2.5);
                    let k = new / span;
                    p.look.center = [at[0] + (p.look.center[0] - at[0]) * k, at[1] + (p.look.center[1] - at[1]) * k];
                    p.look.span = new;
                }
            }
        }
        if shift {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        } else if resp.dragged() {
            ui.ctx().set_cursor_icon(CursorIcon::Grabbing);
        }
    }
    if resp.double_clicked() && !editing_points {
        p.look.center = [0.5, 0.5];
        p.look.span = 1.0;
    }

    match &p.viewer {
        Some((tex, picture)) => {
            // The picture as made (the viewer may have changed size since).
            let size = vec2(picture.size[0] as f32, picture.size[1] as f32);
            let scale = (pic.width() / size.x).min(pic.height() / size.y);
            let r = Rect::from_center_size(pic.center(), size * scale);
            egui::Image::new((tex.id(), r.size())).corner_radius(theme::TILE_RADIUS).paint_at(ui, r);
            legend(ui, &painter, pic, picture, &pal);
            if !picture.sharp {
                chip(&painter, pic.right_top() + vec2(-10.0, 10.0), Align2::RIGHT_TOP, t!("refining…"));
            }
        }
        None => {
            let text = if p.graph.nodes.is_empty() {
                t!("Add nodes to see the terrain they make.")
            } else if p.previews_idle() {
                t!("Connect a node to the Map Output's Height to see the terrain.")
            } else {
                t!("Computing…")
            };
            painter.text(pic.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), pal.weak);
        }
    }
    points_overlay(p, ui, pic, &pal);
    sun_badge(p, ui, &painter, pic);
}

/// The sun's bearing, at the top left, and how to turn it.
fn sun_badge(p: &TerrainPanel, ui: &Ui, painter: &egui::Painter, pic: Rect) {
    let sun_at = pic.left_top() + vec2(22.0, 22.0);
    painter.circle(sun_at, 12.0, Color32::from_black_alpha(90), Stroke::NONE);
    let a = p.look.sun[0].to_radians();
    let reach = 9.0 * (1.0 - p.look.sun[1] / 100.0);
    let s = sun_at + vec2(a.sin(), -a.cos()) * reach;
    painter.circle_filled(s, 3.2, Color32::from_rgb(255, 214, 120));
    let sun_hit = ui.interact(Rect::from_center_size(sun_at, vec2(26.0, 26.0)), ui.scope_id().with("sun"), Sense::hover());
    sun_hit.on_hover_text(t!("Sun: {bearing}° bearing, {height}° high. Shift + drag on the picture turns it.", bearing = format!("{:.0}", p.look.sun[0]), height = format!("{:.0}", p.look.sun[1])));
}

/// The viewer in 3D: the relief as the graph's viewport draws it (sky,
/// soft light, clouds, sea, sides), turned and lit with the pointer; its
/// render settings, clouds, sea, framing and saving at its foot.
fn three_d(p: &mut TerrainPanel, ui: &mut Ui, pic: Rect, pal: &Palette) {
    let painter = ui.painter_at(pic);
    // The viewer's look drives the view: its sun, its relief, its backdrop.
    let v = &mut p.view3d;
    v.render.sun_azimuth = p.look.sun[0];
    v.render.sun_elevation = p.look.sun[1];
    let auto = p.viewer3d.as_ref().map_or(1.0, |v| v.auto);
    v.render.exaggeration = auto * p.look.relief;
    v.sea_level = p.space.sea_level;
    v.backdrop = if p.look.sky { None } else { Some(BACKDROP) };
    if p.viewer3d.is_some() && v.has_data() {
        let mut child = ui.new_child(egui::UiBuilder::new().max_rect(pic).layout(egui::Layout::top_down(egui::Align::Min)));
        child.set_clip_rect(pic.intersect(ui.clip_rect()));
        v.ui_with(&mut child, p.gpu_view.as_ref());
    } else {
        let text = if p.graph.nodes.is_empty() {
            t!("Add nodes to see the terrain they make.")
        } else if p.previews_idle() {
            t!("Connect a node to the Map Output's Height to see the terrain.")
        } else {
            t!("Computing…")
        };
        painter.text(pic.center(), Align2::CENTER_CENTER, text, FontId::proportional(13.0), pal.weak);
    }
    // What the view turned is the viewer's.
    p.look.sun = [p.view3d.render.sun_azimuth, p.view3d.render.sun_elevation.clamp(4.0, 85.0)];
    if let Some(picture) = &p.viewer3d {
        legend(ui, &painter, Rect::from_min_max(pic.min, pos2(pic.max.x, pic.max.y - 44.0)), picture, pal);
        if !picture.sharp {
            chip(&painter, pic.right_top() + vec2(-10.0, 10.0), Align2::RIGHT_TOP, t!("refining…"));
        }
    }
    sun_badge(p, ui, &painter, pic);
    // The view's commands, at its foot, as in the graph's viewport.
    let bar = Rect::from_min_max(pos2(pic.left() + 10.0, pic.bottom() - 40.0), pos2(pic.right() - 10.0, pic.bottom() - 8.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(bar).layout(egui::Layout::left_to_right(egui::Align::Center)));
    egui::Frame::new().fill(pal.panel.gamma_multiply(0.94)).stroke(Stroke::new(1.0, pal.border)).corner_radius(theme::PILL).inner_margin(egui::Margin::symmetric(5, 4)).show(&mut child, |ui| {
        ui.horizontal(|ui| {
            ui.spacing_mut().interact_size.y = 22.0;
            ui.spacing_mut().item_spacing.x = 2.0;
            {
                let w = &mut ui.visuals_mut().widgets;
                w.inactive.weak_bg_fill = Color32::TRANSPARENT;
                for state in [&mut w.inactive, &mut w.hovered, &mut w.active] {
                    state.bg_stroke = Stroke::NONE;
                }
            }
            ui.toggle_value(&mut p.render_open, theme::with_icon(Icon::Sun, t!("Render"))).on_hover_text(t!("Sun, clouds, haze, sea and exposure of the view"));
            ui.toggle_value(&mut p.view3d.render.clouds, theme::with_icon(Icon::Cloud, t!("Clouds")));
            ui.toggle_value(&mut p.view3d.render.sea, theme::with_icon(Icon::Waves, t!("Sea"))).on_hover_text(t!("Water at the world's sea level ({sea_level})", sea_level = format!("{:.0}", p.space.sea_level)));
            if ui.button(theme::glyph(Icon::Focus).to_string()).on_hover_text(t!("Frame the relief")).clicked() {
                p.view3d.camera = Default::default();
            }
            let [w, h] = IMAGE_SIZE;
            if ui.button(theme::glyph(Icon::Camera).to_string()).on_hover_text(t!("Save the view as an image, at full quality ({w} × {h})", w = w, h = h)).clicked() {
                save_image(p);
            }
            match &p.saved {
                Some(Ok(name)) => {
                    ui.label(egui::RichText::new(t!("Saved {name}", name = name)).size(11.5).color(pal.weak));
                }
                Some(Err(e)) => {
                    ui.label(egui::RichText::new(e).size(11.5).color(pal.danger));
                }
                None => {}
            }
        });
    });
    if p.render_open {
        egui::Area::new(ui.scope_id().with("generator render settings")).order(egui::Order::Foreground).fixed_pos(pos2(pic.left() + 10.0, pic.top() + 44.0)).show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).inner_margin(egui::Margin::same(14)).show(ui, |ui| {
                ui.set_width(330.0);
                egui::ScrollArea::vertical().max_height((pic.height() - 100.0).max(160.0)).show(ui, |ui| {
                    p.view3d.settings_ui(ui);
                });
            });
        });
        p.look.sun = [p.view3d.render.sun_azimuth, p.view3d.render.sun_elevation.clamp(4.0, 85.0)];
        p.look.relief = (p.view3d.render.exaggeration / auto).clamp(0.25, 4.0);
    }
}

/// The 3D view's backdrop under Dark.
const BACKDROP: [u8; 3] = [20, 20, 21];
/// Pixels of an image of the 3D view saved.
const IMAGE_SIZE: [u32; 2] = [1920, 1080];

/// Renders the 3D view at full quality and saves it where the user says.
fn save_image(p: &mut TerrainPanel) {
    let name = format!("{}.png", if p.script.title.is_empty() { "Terrain" } else { &p.script.title });
    let Some(path) = rfd::FileDialog::new().set_file_name(name).add_filter(t!("PNG image"), &["png"]).save_file() else {
        return;
    };
    let Some((w, h, rgba)) = p.view3d.render_image(IMAGE_SIZE) else {
        p.saved = Some(Err(t!("Nothing to render yet").to_owned()));
        return;
    };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut png = Vec::new();
    let encoded = image::write_buffer_with_format(&mut std::io::Cursor::new(&mut png), &rgba, w, h, image::ColorType::Rgba8, image::ImageFormat::Png);
    p.saved = Some(match encoded.map_err(|e| e.to_string()).and_then(|()| massif_core::atomic::write(&path, &png, true).map_err(|e| massif_core::atomic::describe(&e, &path))) {
        Ok(()) => Ok(name),
        Err(e) => Err(t!("The image could not be saved: {e}", e = e)),
    });
}

fn chip(painter: &egui::Painter, at: Pos2, align: Align2, text: &str) {
    let g = painter.layout_no_wrap(text.to_owned(), FontId::proportional(11.0), Color32::WHITE);
    let r = align.anchor_size(at, g.size() + vec2(14.0, 6.0));
    painter.rect_filled(r, theme::PILL, Color32::from_black_alpha(150));
    painter.galley(r.min + vec2(7.0, 3.0), g, Color32::WHITE);
}

/// What the picture shows, its kind and range, at its bottom left.
fn legend(_ui: &Ui, painter: &egui::Painter, pic: Rect, picture: &super::eval::Picture, _pal: &Palette) {
    let Some(kind) = picture.kind else { return };
    let range = match kind {
        PortKind::Height => format!("{} · {:.0} – {:.0} b", sockets::kind_label(kind), picture.range.0, picture.range.1),
        PortKind::Mask => format!("{} · {:.2} – {:.2}", sockets::kind_label(kind), picture.range.0, picture.range.1),
        _ => sockets::kind_label(kind).to_owned(),
    };
    let text = format!("{range} · {:.0} ms", picture.ms);
    let at = pic.left_bottom() + vec2(10.0, -10.0);
    let g = painter.layout_no_wrap(text, FontId::proportional(11.0), Color32::WHITE);
    let r = Align2::LEFT_BOTTOM.anchor_size(at, g.size() + vec2(26.0, 6.0));
    painter.rect_filled(r, theme::PILL, Color32::from_black_alpha(150));
    painter.circle_filled(r.left_center() + vec2(10.0, 0.0), 4.0, sockets::color(kind));
    painter.galley(r.min + vec2(19.0, 3.0), g, Color32::WHITE);
    let _ = StrokeKind::Inside;
}

/// The viewer's resolutions: as its pixels need, or as many samples a
/// side.
pub const RESOLUTIONS: [u32; 5] = [0, 1024, 2048, 4096, 8192];

fn controls(p: &mut TerrainPanel, ui: &mut Ui) {
    let pal = Palette::of(ui);
    if let Some(k) = widgets::button_group(ui, "terrain viewer mode", &[t!("2D"), t!("3D")], &[!p.look.three_d, p.look.three_d]) {
        p.look.three_d = k == 1;
    }
    ui.add_space(4.0);
    let colour = p.look.surface == Surface::Colour;
    if let Some(k) = widgets::button_group(ui, "terrain viewer surface", &[t!("Clay"), t!("Colours")], &[!colour, colour]) {
        p.look.surface = if k == 0 { Surface::Clay } else { Surface::Colour };
    }
    ui.add_space(8.0);
    // What is viewed, in the room the commands on the right leave (none
    // when the viewer is too narrow: the node stays selected in the graph).
    let room = ui.available_width() - RIGHT_W;
    if room > 90.0 {
        ui.allocate_ui_with_layout(egui::vec2(room, ui.available_height()), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_max_width(room);
            // What is viewed: a node, and which of its outputs.
            let crumbs = p.crumbs();
            let viewed_here = p.viewed.clone().filter(|v| v.0 == crumbs).map(|v| v.1);
            let shown = viewed_here.clone().or_else(|| if p.viewed.is_none() && crumbs.is_empty() { p.active_output().map(|node| Shown { node, port: None }) } else { None });
            if let Some(shown) = shown.filter(|s| p.graph.nodes.contains_key(&s.node)) {
                let shape = p.shape(&shown.node);
                let mut options: Vec<(Option<String>, String)> = Vec::new();
                if let Some(shape) = &shape {
                    if shape.is_output() {
                        options.push((None, t!("Map").to_owned()));
                        for name in p.graph.nodes[&shown.node].inputs.keys() {
                            let label = shape.inputs.iter().find(|i| i.name == *name).map_or_else(|| name.trim_start_matches("mask.").to_owned(), |i| i.label.clone());
                            options.push((Some(name.clone()), label));
                        }
                    } else {
                        for (k, o) in shape.outputs.iter().enumerate() {
                            options.push((if k == 0 { None } else { Some(o.name.clone()) }, o.label.clone()));
                        }
                    }
                }
                let current = options.iter().find(|o| o.0 == shown.port).map_or(String::new(), |o| o.1.clone());
                let title = shape.map_or(shown.node.clone(), |s| super::canvas::node_title(p, &shown.node, &s));
                ui.label(egui::RichText::new(t!("Viewing")).size(12.0).color(pal.weak));
                ui.add(egui::Label::new(egui::RichText::new(&title).size(12.0)).truncate());
                if options.len() > 1 {
                    let mut port = shown.port.clone();
                    widgets::combo("terrain viewer output").selected_text(current).show_ui(ui, |ui| {
                        for (k, label) in &options {
                            ui.selectable_value(&mut port, k.clone(), label);
                        }
                    });
                    if port != shown.port {
                        p.viewed = Some((crumbs.clone(), Shown { node: shown.node.clone(), port }));
                    }
                }
            } else if let Some((_, v)) = &p.viewed {
                ui.label(egui::RichText::new(t!("Viewing")).size(12.0).color(pal.weak));
                ui.add(egui::Label::new(egui::RichText::new(&v.node).size(12.0)).truncate());
            }
            if p.viewed.is_some() && ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false)).on_hover_text(t!("Back to the map output")).clicked() {
                p.viewed = None;
            }
        });
    }
    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
        if ui.add(theme::primary_button(ui, t!("Apply to map"))).on_hover_text(t!("Write the graph's result into the map, in one step you can undo")).clicked() {
            p.events.push(Event::Apply);
            p.apply_asked = true;
        }
        ui.add_space(6.0);
        let mut live = p.live_on;
        if widgets::switch(ui, &mut live).on_hover_text(t!("Show the graph's result on the World map as it is edited, without writing it")).changed() {
            p.live_on = live;
            if let Some(l) = &p.live {
                if live {
                    l.set_graph(&p.document(), p.space, p.env.clone(), p.revision);
                } else {
                    l.clear();
                }
            }
        }
        ui.label(egui::RichText::new(t!("Live")).size(12.0));
        ui.add_space(8.0);
        let r = ui.add(egui::Button::new(egui::RichText::new(theme::glyph(Icon::SlidersHorizontal).to_string()).size(13.0)).frame(false));
        egui::Popup::menu(&r).show(|ui| {
            ui.set_min_width(220.0);
            widgets::well(ui, |ui| {
                widgets::row(ui, t!("Relief"), |ui| {
                    let mut v = p.look.relief as f64;
                    if ui.add(widgets::Field::new(&mut v, 0.25..=4.0).suffix("×").width(110.0)).changed() {
                        p.look.relief = v as f32;
                    }
                });
                widgets::row(ui, t!("Contour lines"), |ui| {
                    widgets::switch(ui, &mut p.look.contours);
                });
                widgets::row(ui, t!("Mask opacity"), |ui| {
                    let mut v = p.look.overlay as f64 * 100.0;
                    if ui.add(widgets::Field::new(&mut v, 0.0..=100.0).suffix(" %").max_decimals(0).width(110.0)).changed() {
                        p.look.overlay = (v / 100.0) as f32;
                    }
                });
                widgets::row(ui, t!("Sun height"), |ui| {
                    let mut v = p.look.sun[1] as f64;
                    if ui.add(widgets::Field::new(&mut v, 4.0..=85.0).suffix("°").max_decimals(0).width(110.0)).changed() {
                        p.look.sun[1] = v as f32;
                    }
                });
                widgets::row(ui, t!("Background"), |ui| {
                    if let Some(k) = widgets::button_group(ui, "terrain viewer background", &[t!("Dark"), t!("Sky")], &[!p.look.sky, p.look.sky]) {
                        p.look.sky = k == 1;
                    }
                });
                widgets::row(ui, t!("Resolution"), |ui| {
                    let label = |r: u32| if r == 0 { t!("Auto").to_owned() } else { r.to_string() };
                    widgets::combo("terrain viewer resolution").width(110.0).selected_text(label(p.look.resolution)).show_ui(ui, |ui| {
                        for r in RESOLUTIONS {
                            ui.selectable_value(&mut p.look.resolution, r, label(r));
                        }
                    });
                });
                if p.look.resolution >= 4096 {
                    ui.add_space(2.0);
                    let mb = ((p.look.resolution as u64).pow(2) * 4 * 5) >> 20;
                    ui.add(egui::Label::new(egui::RichText::new(t!("{n} samples a side: about {mb} MB and several seconds a picture", n = p.look.resolution, mb = mb)).size(11.0).color(Palette::of(ui).weak)).wrap());
                    ui.add_space(4.0);
                }
                widgets::row(ui, t!("Thumbnails"), |ui| {
                    let colour = p.thumb_surface == Surface::Colour;
                    if let Some(k) = widgets::button_group(ui, "terrain thumbs surface", &[t!("Clay"), t!("Colours")], &[!colour, colour]) {
                        p.thumb_surface = if k == 0 { Surface::Clay } else { Surface::Colour };
                    }
                });
            });
            ui.add_space(8.0);
            widgets::well(ui, |ui| {
                widgets::row(ui, t!("Apply over the map"), |ui| {
                    widgets::switch(ui, &mut p.apply_over).on_hover_text(t!("On: what the graph digs and raises goes into levels of their own, which you can hide or delete, and the map's ground stays as it was. Off: the graph replaces the ground."));
                });
                widgets::row(ui, t!("Height map"), |ui| {
                    let busy = p.heightmap.is_some();
                    let label = if busy { t!("Saving…") } else { t!("Save…") };
                    if ui.add_enabled(!busy, egui::Button::new(label)).on_hover_text(t!("Save the terrain at a block a sample as a 16-bit PNG height map, without writing it into the map")).clicked()
                        && let Some(path) = rfd::FileDialog::new().set_file_name(t!("terrain height map.png")).add_filter(t!("PNG image"), &["png"]).save_file()
                    {
                        p.save_heightmap(path);
                        ui.close();
                    }
                });
            });
        });
    });
}

/// The first Points or Curve setting of the selected node.
fn editable(p: &TerrainPanel) -> Option<(String, &'static massif_terrain::registry::ParamSpec, usize)> {
    let id = p.selected.last()?;
    let spec = p.spec(id)?;
    spec.params.iter().enumerate().find(|(_, q)| matches!(q.kind, ParamKind::Points { .. } | ParamKind::Curve { .. })).map(|(k, q)| (id.clone(), q, k))
}

/// The selected node's points (or its line) over the 2D picture: dragged
/// to move them, double-click to add one, right-click to remove one.
fn points_overlay(p: &mut TerrainPanel, ui: &mut Ui, pic: Rect, pal: &Palette) {
    let Some((id, q, k)) = editable(p) else { return };
    let Some(params) = super::props::decoded(p, &id) else { return };
    let Value::Points(mut pts) = params.values()[k].clone() else { return };
    let w = p.look.window;
    // The picture is drawn to fit the card; the window maps onto it.
    let picture = p.viewer.as_ref().map(|v| vec2(v.1.size[0] as f32, v.1.size[1] as f32)).unwrap_or(pic.size());
    let scale = (pic.width() / picture.x).min(pic.height() / picture.y);
    let shown = Rect::from_center_size(pic.center(), picture * scale);
    let to_screen = |x: f64, z: f64| pos2(shown.left() + ((x as f32 - w[0]) / (w[2] - w[0])) * shown.width(), shown.top() + ((z as f32 - w[1]) / (w[3] - w[1])) * shown.height());
    let to_terrain = |s: Pos2| (((s.x - shown.left()) / shown.width() * (w[2] - w[0]) + w[0]) as f64, ((s.y - shown.top()) / shown.height() * (w[3] - w[1]) + w[1]) as f64);
    let painter = ui.painter_at(pic);
    let curve = matches!(q.kind, ParamKind::Curve { .. });
    let accent = Color32::from_rgb(255, 160, 40);
    if curve && pts.len() >= 2 {
        let line: Vec<Pos2> = pts.iter().map(|pt| to_screen(pt.x, pt.z)).collect();
        let closed = matches!(q.kind, ParamKind::Curve { closed: true, .. });
        let mut all = line.clone();
        if closed {
            all.push(line[0]);
        }
        painter.extend(egui::Shape::dashed_line(&all, Stroke::new(2.0, accent), 8.0, 5.0));
    }
    let mut changed = false;
    let mut remove = None;
    for (j, pt) in pts.iter_mut().enumerate() {
        let c = to_screen(pt.x, pt.z);
        let r = ui.interact(Rect::from_center_size(c, vec2(18.0, 18.0)), ui.scope_id().with(("viewer point", j)), Sense::click_and_drag());
        painter.circle(c, if r.hovered() || r.dragged() { 7.0 } else { 5.5 }, accent, Stroke::new(1.5, Color32::WHITE));
        if r.dragged()
            && let Some(q) = r.interact_pointer_pos()
        {
            let (x, z) = to_terrain(q);
            pt.x = x.clamp(0.0, 1.0);
            pt.z = z.clamp(0.0, 1.0);
            changed = true;
        }
        if r.secondary_clicked() {
            remove = Some(j);
        }
        if r.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Grab);
        }
    }
    if let Some(j) = remove {
        pts.remove(j);
        changed = true;
    }
    let max = match q.kind {
        ParamKind::Points { max, .. } => max,
        _ => 64,
    };
    let bg = ui.interact(pic, ui.scope_id().with("terrain viewer"), Sense::click_and_drag());
    if bg.double_clicked()
        && pts.len() < max
        && let Some(s) = bg.interact_pointer_pos()
    {
        let (x, z) = to_terrain(s);
        let fields = match q.kind {
            ParamKind::Points { fields, .. } | ParamKind::Curve { fields, .. } => fields,
            _ => &[],
        };
        let values = pts.last().map(|l| l.values.clone()).unwrap_or_else(|| fields.iter().map(|f| f.default).collect());
        pts.push(massif_terrain::params::Point { x: x.clamp(0.0, 1.0), z: z.clamp(0.0, 1.0), values });
        changed = true;
    }
    chip(&painter, pic.center_top() + vec2(0.0, 10.0), Align2::CENTER_TOP, &t!("{name}: drag the points, double-click to add one, right-click to remove one", name = super::text::name(q.label)));
    if changed {
        let j = super::props::encode(q, &Value::Points(pts));
        p.edit().set(&id, q.name, j);
    }
    let _ = pal;
}
