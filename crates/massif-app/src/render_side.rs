//! The Render side: path-traced images of the map. For now, what it will
//! be, until the renderer of Massif 2.0 comes.

use egui::{Rect, RichText, Ui, vec2};

use crate::theme::{self, Icon, Palette, RichTextExt};

/// The Render side, over the whole of the window under its bars.
pub fn ui(ui: &mut Ui) {
    let pal = Palette::of(ui);
    let rect = ui.available_rect_before_wrap();
    ui.painter().rect_filled(rect, theme::CARD_RADIUS, pal.canvas);
    let column = Rect::from_center_size(rect.center(), vec2(420.0_f32.min(rect.width() - 32.0), 260.0));
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(column).layout(egui::Layout::top_down(egui::Align::Center)));
    child.label(RichText::new(theme::glyph(Icon::Aperture).to_string()).size(28.0).color(pal.weak));
    child.add_space(8.0);
    child.horizontal(|ui| {
        // Centred: the title and its tag, side by side.
        let tag = RichText::new(tr!("Beta")).size(11.0).strong().color(pal.accent);
        let title = RichText::new(tr!("Render")).size(16.0).medium();
        let width = ui.fonts_mut(|f| f.layout_no_wrap(tr!("Render").to_owned(), egui::FontId::proportional(16.0), pal.text).size().x + f.layout_no_wrap(tr!("Beta").to_owned(), egui::FontId::proportional(11.0), pal.text).size().x) + ui.spacing().item_spacing.x;
        ui.add_space(((ui.available_width() - width) / 2.0).max(0.0));
        ui.label(title);
        ui.label(tag);
    });
    child.add_space(2.0);
    child.label(RichText::new(tr!("Coming in the Massif 2.0 beta")).size(12.5).color(pal.text));
    child.add_space(8.0);
    child.add(
        egui::Label::new(
            RichText::new(tr!("Photorealistic images of your map, path-traced on the graphics card with the game's textures and resource packs, under a sky with a sun, haze and fog. Then large stills, camera paths and Minecraft worlds opened as they are."))
                .size(12.0)
                .color(pal.weak),
        )
        .halign(egui::Align::Center)
        .wrap(),
    );
}
