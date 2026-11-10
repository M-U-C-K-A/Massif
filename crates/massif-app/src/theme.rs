//! Look and feel: fonts, icons, and the light / dark palettes.

use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{
    Color32, CornerRadius, FontData, FontFamily, FontId, Stroke, TextStyle, Theme, Visuals, vec2,
};
pub use lucide_icons::Icon;

/// The colours the custom-painted parts of the UI use. Derived from the
/// active egui theme so that both stay in sync.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Behind the areas.
    pub app_bg: Color32,
    /// Area background.
    pub panel: Color32,
    /// Area header and bars.
    pub header: Color32,
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    /// Background of selected items.
    pub accent_soft: Color32,
    pub danger: Color32,
    /// Backdrop of the map and 3D canvases.
    pub canvas: Color32,
    pub sky: Color32,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        dark: false,
        app_bg: Color32::from_rgb(232, 232, 236),
        panel: Color32::from_rgb(250, 250, 251),
        header: Color32::from_rgb(244, 244, 246),
        border: Color32::from_rgb(214, 214, 220),
        text: Color32::from_rgb(29, 29, 31),
        weak: Color32::from_rgb(110, 110, 118),
        accent: Color32::from_rgb(10, 108, 255),
        accent_soft: Color32::from_rgb(218, 232, 255),
        danger: Color32::from_rgb(220, 53, 60),
        canvas: Color32::from_rgb(226, 227, 232),
        sky: Color32::from_rgb(196, 220, 245),
    };

    pub const DARK: Palette = Palette {
        dark: true,
        app_bg: Color32::from_rgb(20, 20, 23),
        panel: Color32::from_rgb(34, 34, 38),
        header: Color32::from_rgb(41, 41, 46),
        border: Color32::from_rgb(58, 58, 65),
        text: Color32::from_rgb(232, 232, 236),
        weak: Color32::from_rgb(150, 150, 160),
        accent: Color32::from_rgb(64, 140, 255),
        accent_soft: Color32::from_rgb(38, 62, 104),
        danger: Color32::from_rgb(240, 84, 88),
        canvas: Color32::from_rgb(26, 26, 30),
        sky: Color32::from_rgb(34, 44, 62),
    };

    pub fn of(ui: &egui::Ui) -> Palette {
        if ui.visuals().dark_mode {
            Self::DARK
        } else {
            Self::LIGHT
        }
    }

    fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        let radius = CornerRadius::same(6);
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(10);
        v.menu_corner_radius = CornerRadius::same(8);
        v.extreme_bg_color = if self.dark {
            Color32::from_rgb(24, 24, 27)
        } else {
            Color32::WHITE
        };
        v.faint_bg_color = self.header;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent_soft;
        v.selection.stroke = Stroke::new(1.0, self.accent);
        v.slider_trailing_fill = true;

        let lift = |c: Color32, by: i16| {
            let f = |x: u8| (x as i16 + by).clamp(0, 255) as u8;
            Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
        };
        let step = if self.dark { 10 } else { -10 };
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = lift(self.header, step * 2);
        w.inactive.weak_bg_fill = lift(self.header, step / 2);
        w.inactive.bg_stroke = Stroke::NONE;
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;

        w.hovered.bg_fill = lift(self.header, step * 3);
        w.hovered.weak_bg_fill = lift(self.header, step * 2);
        w.hovered.bg_stroke = Stroke::new(1.0, lift(self.border, step * 2));
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = self.accent_soft;
        w.active.weak_bg_fill = self.accent_soft;
        w.active.bg_stroke = Stroke::new(1.0, self.accent);
        w.active.fg_stroke = Stroke::new(1.0, self.text);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = lift(self.header, step * 2);
        w.open.weak_bg_fill = lift(self.header, step * 2);
        w.open.bg_stroke = Stroke::new(1.0, self.border);
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        w.open.corner_radius = radius;
        v
    }
}

/// Install fonts, text sizes, spacing and both colour themes.
pub fn install(ctx: &egui::Context) {
    // Lucide ships as an icon font: registered as a fallback, its glyphs can
    // be used inline in any label or button.
    let mut lucide = FontData::from_static(lucide_icons::LUCIDE_FONT_BYTES);
    lucide.tweak.scale = 1.12;
    lucide.tweak.y_offset_factor = 0.06;
    ctx.add_font(FontInsert::new(
        "lucide",
        lucide,
        vec![
            InsertFontFamily {
                family: FontFamily::Proportional,
                priority: FontPriority::Lowest,
            },
            InsertFontFamily {
                family: FontFamily::Monospace,
                priority: FontPriority::Lowest,
            },
        ],
    ));

    ctx.all_styles_mut(|style| {
        style.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(17.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = vec2(6.0, 5.0);
        style.spacing.button_padding = vec2(7.0, 3.0);
        style.spacing.interact_size = vec2(28.0, 22.0);
        style.spacing.slider_width = 120.0;
        style.spacing.menu_margin = egui::Margin::same(6);
        style.interaction.selectable_labels = false;
    });
    ctx.set_visuals_of(Theme::Light, Palette::LIGHT.visuals());
    ctx.set_visuals_of(Theme::Dark, Palette::DARK.visuals());
}

/// The glyph of an icon, to embed in text.
pub fn glyph(icon: Icon) -> char {
    char::from(icon)
}

/// `"<icon>  label"`.
pub fn with_icon(icon: Icon, label: &str) -> String {
    format!("{}  {label}", glyph(icon))
}

/// Multiply the RGB of a colour.
pub fn scale(c: Color32, f: f32) -> Color32 {
    let s = |x: u8| (x as f32 * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgba_premultiplied(s(c.r()), s(c.g()), s(c.b()), c.a())
}

/// Linear blend of two colours, `t` = 0 gives `a`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()))
}

pub fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}
