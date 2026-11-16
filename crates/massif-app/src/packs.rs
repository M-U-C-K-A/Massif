//! The Packs panel: ready-made things to paint with, as WorldPainter
//! Modernised's Packs panel has them — the sixteen trees of the tree pack,
//! presets of flowers, plants and water plants, and mixes of blocks for the
//! ground, rock and cliffs. A click adds one to the map (an object layer, a
//! ground cover, a terrain), or finds it there if it was added before, and
//! picks it to paint.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::{Tool, ToolState};
use egui::{Color32, Rect, RichText, Sense, Stroke, StrokeKind, Ui, vec2};
use massif_core::packs::{Lay, MATERIAL_MIXES, MIX_FAMILIES, MaterialMix, PLANT_FAMILIES, PLANT_PRESETS, PlantPreset, WATER};
use massif_core::treepack::TreeKind;

/// One thing of a pack.
#[derive(Clone, Copy)]
pub enum Item {
    Tree(TreeKind),
    Plants(&'static PlantPreset),
    Mix(&'static MaterialMix),
}

impl Item {
    pub fn name(&self) -> &'static str {
        match self {
            Item::Tree(k) => k.name(),
            Item::Plants(p) => p.name,
            Item::Mix(m) => m.name,
        }
    }

    fn color(&self) -> [u8; 3] {
        match self {
            Item::Tree(k) => k.color(),
            Item::Plants(p) => p.color(),
            Item::Mix(m) => m.color(),
        }
    }

    fn tip(&self) -> String {
        match self {
            Item::Tree(k) => tr!("{about}. Eight trees, an object layer.", about = crate::i18n::tr_name(k.about())),
            Item::Plants(p) => tr!("{about}. A ground cover.", about = p.about),
            Item::Mix(m) => tr!("{describe}. A terrain.", describe = m.describe()),
        }
    }

    /// Whether the map has it already.
    fn installed(&self, reg: &massif_core::Registry) -> bool {
        match self {
            Item::Tree(k) => reg.object_layers.iter().any(|l| !l.removed && l.name == k.name()),
            Item::Plants(p) => reg.cover_by_name(p.name).is_some(),
            Item::Mix(m) => reg.terrains.iter().any(|t| t.group == m.family && t.name == m.name),
        }
    }

    /// Whether it is what the tool in hand paints.
    fn painted(&self, tools: &ToolState, reg: &massif_core::Registry) -> bool {
        match self {
            Item::Tree(k) => tools.tool == Tool::Objects && reg.object_layer(tools.object_layer).is_some_and(|l| l.name == k.name()),
            Item::Plants(p) => tools.tool == Tool::Cover && reg.cover(tools.cover).is_some_and(|c| c.name == p.name),
            Item::Mix(m) => tools.tool == Tool::PaintTerrain && !tools.pattern.on && reg.terrains.get(tools.terrain as usize).is_some_and(|t| t.group == m.family && t.name == m.name),
        }
    }
}

/// Every item, by section.
pub fn sections() -> Vec<(&'static str, Vec<Item>)> {
    let mut out = vec![(tr!("Trees"), TreeKind::ALL.iter().map(|&k| Item::Tree(k)).collect())];
    for family in PLANT_FAMILIES {
        out.push((family, PLANT_PRESETS.iter().filter(|p| p.family == family).map(Item::Plants).collect()));
    }
    for family in MIX_FAMILIES {
        out.push((family, MATERIAL_MIXES.iter().filter(|m| m.family == family).map(Item::Mix).collect()));
    }
    out
}

/// Add `item` to the map (or find it there) and pick it to paint. Returns
/// what to say.
pub fn apply(item: Item, tools: &mut ToolState, doc: &mut Document, catalogue: Option<&massif_assets::Catalogue>) -> String {
    let had = item.installed(&doc.world.registry);
    let reg = &mut doc.world.registry;
    match item {
        Item::Tree(kind) => {
            let first = reg.objects.len();
            tools.object_layer = kind.install(reg);
            // The game's colours for their blocks, as for imported objects.
            for o in &mut reg.objects[first..] {
                o.colors = o
                    .palette
                    .iter()
                    .map(|state| match catalogue.and_then(|c| c.block(state)) {
                        Some(b) if state.contains("log") => b.side_color,
                        Some(b) => b.color,
                        None => massif_core::objects::guess_color(state),
                    })
                    .collect();
            }
            tools.select(Tool::Objects);
        }
        Item::Plants(p) => {
            tools.cover = p.install(reg);
            tools.select(Tool::Cover);
        }
        Item::Mix(m) => {
            tools.terrain = m.install(reg).0;
            if let Some(cat) = catalogue {
                massif_assets::apply_colors(reg, cat);
            }
            tools.pattern.on = false;
            tools.select(Tool::PaintTerrain);
        }
    }
    if had {
        tr!("Painting with {name}", name = crate::i18n::tr_name(item.name()))
    } else {
        doc.mark_changed();
        tr!("Added {name} to the map's layers: paint it", name = crate::i18n::tr_name(item.name()))
    }
}

/// The Packs panel, with its search field. Returns what to say.
pub fn packs_panel(ui: &mut Ui, tools: &mut ToolState, doc: &mut Document, catalogue: Option<&massif_assets::Catalogue>, search: &mut String, library: &mut crate::pack_share::PackLibrary) -> Option<String> {
    let pal = Palette::of(ui);
    ui.label(RichText::new(tr!("Ready-made trees, plants and mixes of blocks. Click one to add it to the map and paint it.")).size(11.5).color(pal.weak));
    ui.add(egui::TextEdit::singleline(search).hint_text(tr!("{glyph}  Search packs", glyph = theme::glyph(Icon::Search))).desired_width(f32::INFINITY));
    let needle = search.trim().to_lowercase();
    let said = crate::pack_share::library_ui(ui, library, &needle, tools, doc, catalogue);
    let mut picked = None;
    for (title, items) in sections() {
        let shown: Vec<Item> = if needle.is_empty() || title.to_lowercase().contains(&needle) { items } else { items.into_iter().filter(|i| i.name().to_lowercase().contains(&needle) || crate::i18n::tr_name(i.name()).to_lowercase().contains(&needle)).collect() };
        if shown.is_empty() {
            continue;
        }
        ui.add_space(6.0);
        ui.label(RichText::new(format!("{}  {}", theme::glyph(section_icon(title)), title.to_uppercase())).size(10.5).strong().color(pal.weak));
        ui.add_space(2.0);
        let gap = 4.0;
        let width = ui.available_width();
        let columns = ((width + gap) / 150.0).floor().max(1.0) as usize;
        let tile_w = (width - gap * (columns - 1) as f32) / columns as f32;
        for row in shown.chunks(columns) {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = gap;
                for item in row {
                    if tile(ui, item, tile_w, tools, doc, catalogue) {
                        picked = Some(*item);
                    }
                }
            });
            ui.add_space(gap - ui.spacing().item_spacing.y);
        }
    }
    if !needle.is_empty() && sections().iter().all(|(t, items)| !t.to_lowercase().contains(&needle) && items.iter().all(|i| !(i.name().to_lowercase().contains(&needle) || crate::i18n::tr_name(i.name()).to_lowercase().contains(&needle)))) {
        ui.label(RichText::new(tr!("Nothing of that name among Massif's packs.")).size(11.5).color(pal.weak));
    }
    picked.map(|item| apply(item, tools, doc, catalogue)).or(said)
}

fn section_icon(title: &str) -> Icon {
    match title {
        "Trees" => Icon::TreePine,
        massif_core::packs::FLOWERS => Icon::Flower2,
        WATER => Icon::Waves,
        massif_core::packs::PLANTS => Icon::Sprout,
        _ => Icon::Blend,
    }
}

/// An item's tile: its picture and its name; ringed when it is painted,
/// a dot when the map has it. Returns whether it was clicked.
fn tile(ui: &mut Ui, item: &Item, width: f32, tools: &ToolState, doc: &Document, catalogue: Option<&massif_assets::Catalogue>) -> bool {
    let reg = &doc.world.registry;
    let installed = item.installed(reg);
    let tip = format!("{}{}", item.tip(), if installed { tr!("\nOn the map already: a click picks it.") } else { "" });
    tile_ui(ui, width, crate::i18n::tr_name(item.name()), item.painted(tools, reg), installed, &tip, |painter, rect| picture(painter, rect, item, catalogue))
}

/// A tile of the Packs panel, its picture drawn by `draw`. Returns
/// whether it was clicked.
pub fn tile_ui(ui: &mut Ui, width: f32, name: &str, painted: bool, installed: bool, tip: &str, draw: impl FnOnce(&egui::Painter, Rect)) -> bool {
    let pal = Palette::of(ui);
    let (rect, response) = ui.allocate_exact_size(vec2(width, 32.0), Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, painted, name));
    let fill = if painted {
        pal.accent_soft
    } else if response.hovered() {
        pal.header
    } else {
        Color32::TRANSPARENT
    };
    let painter = ui.painter().clone();
    painter.rect(rect, theme::TILE_RADIUS, fill, Stroke::new(1.0, if painted { pal.accent } else { pal.border }), StrokeKind::Inside);
    let pic = Rect::from_min_size(rect.left_top() + vec2(5.0, 5.0), vec2(22.0, 22.0));
    draw(&painter, pic);
    let text_w = rect.width() - 38.0 - if installed { 12.0 } else { 0.0 };
    let mut job = egui::text::LayoutJob::simple_singleline(name.to_owned(), egui::FontId::proportional(12.0), pal.text);
    job.wrap = egui::text::TextWrapping::truncate_at_width(text_w.max(10.0));
    let galley = painter.fonts_mut(|f| f.layout_job(job));
    painter.galley(egui::pos2(pic.right() + 7.0, rect.center().y - galley.size().y / 2.0), galley, pal.text);
    if installed {
        painter.circle_filled(egui::pos2(rect.right() - 9.0, rect.center().y), 2.5, pal.weak);
    }
    response.on_hover_text(tip).clicked()
}

/// A swatch of colour `c` with `icon` on it, light or dark as it reads.
pub fn swatch(painter: &egui::Painter, rect: Rect, c: [u8; 3], icon: Icon) {
    painter.rect(rect, theme::TILE_RADIUS, theme::rgb(c), Stroke::new(1.0, Color32::from_black_alpha(60)), StrokeKind::Inside);
    let light = (c[0] as u32 * 299 + c[1] as u32 * 587 + c[2] as u32 * 114) / 1000 > 150;
    painter.text(rect.center(), egui::Align2::CENTER_CENTER, theme::glyph(icon), egui::FontId::proportional(14.0), if light { Color32::from_rgb(24, 24, 27) } else { Color32::WHITE });
}

/// A tree or a plant on a swatch of its colour; a mix as a few of its
/// blocks.
fn picture(painter: &egui::Painter, rect: Rect, item: &Item, catalogue: Option<&massif_assets::Catalogue>) {
    match item {
        Item::Mix(m) => {
            let n = 3;
            let cell = rect.width() / n as f32;
            let total: u32 = m.blocks.iter().map(|b| b.1 as u32).sum::<u32>().max(1);
            for j in 0..n {
                for i in 0..n {
                    let k = match m.lay {
                        Lay::Beds => j,
                        _ => i + j * n,
                    };
                    let mut r = (massif_core::compose::hash2(k, 0, 0x7061_636b) % total) as i32;
                    let block = m.blocks.iter().find(|b| {
                        r -= b.1 as i32;
                        r < 0
                    });
                    let state = format!("minecraft:{}", block.map_or("stone", |b| b.0));
                    let c = Rect::from_min_size(rect.min + vec2(i as f32 * cell, j as f32 * cell), vec2(cell, cell));
                    let color = catalogue.and_then(|cat| cat.block(&state)).map_or_else(|| massif_core::objects::guess_color(&state), |b| b.color);
                    painter.rect_filled(c, 0.0, theme::rgb(color));
                }
            }
            painter.rect_stroke(rect, theme::TILE_RADIUS, Stroke::new(1.0, Color32::from_black_alpha(70)), StrokeKind::Inside);
        }
        _ => {
            let icon = match item {
                Item::Tree(k) if k.conifer() => Icon::TreePine,
                Item::Tree(TreeKind::Palm) => Icon::TreePalm,
                Item::Tree(TreeKind::Bush | TreeKind::Azalea) => Icon::Shrub,
                Item::Tree(_) => Icon::TreeDeciduous,
                Item::Plants(p) if p.family == WATER => Icon::Waves,
                Item::Plants(p) if p.family == massif_core::packs::FLOWERS => Icon::Flower2,
                _ => Icon::Sprout,
            };
            swatch(painter, rect, item.color(), icon);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;
    use massif_core::generate::{GenParams, generate};

    struct State {
        tools: ToolState,
        doc: Document,
        search: String,
        catalogue: Option<std::sync::Arc<massif_assets::Catalogue>>,
        library: crate::pack_share::PackLibrary,
    }

    fn draw(ui: &mut Ui, s: &mut State) {
        theme::install(ui.ctx());
        let pal = Palette::of(ui);
        egui::Frame::new().fill(pal.panel).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            packs_panel(ui, &mut s.tools, &mut s.doc, s.catalogue.as_deref(), &mut s.search, &mut s.library);
        });
    }

    fn state() -> State {
        let world = generate(&GenParams { tiles_x: 2, tiles_z: 2, ..GenParams::default() });
        State { tools: ToolState::default(), doc: Document::new(world, None, 1), search: String::new(), catalogue: None, library: Default::default() }
    }

    /// A click adds a tree, a plant or a mix to the map once and picks it
    /// to paint; the search keeps what matches.
    #[test]
    fn packs_are_added_once_and_picked() {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(340.0, 3000.0)).build_ui_state(draw, state());
        h.run_steps(2);
        let objects = h.state().doc.world.registry.objects.len();
        for _ in 0..2 {
            h.get_by_label("Firs").click();
            h.run_steps(2);
        }
        let reg = &h.state().doc.world.registry;
        assert_eq!(reg.objects.len(), objects + massif_core::treepack::VARIANTS, "added once");
        assert_eq!(h.state().tools.tool, Tool::Objects);
        assert_eq!(reg.object_layer(h.state().tools.object_layer).unwrap().name, "Firs");

        h.get_by_label("Tulip field").click();
        h.run_steps(2);
        assert_eq!(h.state().tools.tool, Tool::Cover);
        assert_eq!(h.state().doc.world.registry.cover(h.state().tools.cover).unwrap().name, "Tulip field");

        h.get_by_label("Badlands strata").click();
        h.run_steps(2);
        let s = h.state();
        assert_eq!(s.tools.tool, Tool::PaintTerrain);
        assert_eq!(s.doc.world.registry.terrain(massif_core::registry::TerrainId(s.tools.terrain)).name, "Badlands strata");
        assert!(s.doc.revision > 0);

        h.state_mut().search = "spruce".into();
        h.run_steps(2);
        h.get_by_label("Tall spruces");
        assert!(h.query_by_label("Firs").is_none());
        h.state_mut().search = "strata".into();
        h.run_steps(2);
        assert!(h.query_by_label("Badlands strata").is_some() && h.query_by_label("Meadow").is_none());
    }

    /// The panel offscreen in both themes, a few things on the map. Needs a
    /// GPU: runs with MASSIF_SNAPSHOT_DIR.
    #[test]
    fn packs_panel_render() {
        let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(std::path::PathBuf::from) else {
            return;
        };
        let ppp = std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(2.0);
        for dark in [true, false] {
            let mut s = state();
            s.catalogue = massif_assets::find_jars().first().and_then(|jar| massif_assets::load(&jar.path, &jar.version).ok()).map(std::sync::Arc::new);
            apply(Item::Tree(TreeKind::Larch), &mut s.tools, &mut s.doc, None);
            apply(Item::Plants(&PLANT_PRESETS[0]), &mut s.tools, &mut s.doc, None);
            apply(Item::Tree(TreeKind::Fir), &mut s.tools, &mut s.doc, None);
            let mut h = egui_kittest::Harness::builder()
                .with_size(vec2(340.0, 1500.0))
                .with_pixels_per_point(ppp)
                .with_accessibility_check(false)
                .wgpu()
                .build_ui_state(
                    move |ui, s: &mut State| {
                        ui.ctx().set_theme(if dark { egui::Theme::Dark } else { egui::Theme::Light });
                        draw(ui, s);
                    },
                    s,
                );
            h.run_steps(4);
            h.render().unwrap().save(dir.join(if dark { "packs-dark.png" } else { "packs-light.png" })).unwrap();
        }
    }
}
