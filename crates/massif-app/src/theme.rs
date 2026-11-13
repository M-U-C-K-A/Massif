//! Look and feel: fonts, icons, and the light / dark palettes.
//!
//! The style follows current desktop tools: near-black (or near-white)
//! backgrounds, cards with hairline borders and generous corners, neutral
//! fills for what is selected, one accent used sparingly, pill-shaped
//! controls, and the system's own typeface (SF Pro on macOS) with real
//! weights.

use egui::epaint::Shadow;
use egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use egui::{
    Color32, CornerRadius, FontData, FontFamily, FontId, RichText, Stroke, TextStyle, Theme, Visuals, vec2,
};
pub use lucide_icons::Icon;
use std::sync::atomic::{AtomicU32, Ordering};

/// The accent colours offered in the settings: the colour of what is
/// chosen, of the main buttons, of the art of the home screen.
pub const ACCENTS: [(&str, [u8; 3]); 8] = [
    ("Ember", [255, 112, 40]),
    ("Ocean", [77, 144, 254]),
    ("Moss", [72, 190, 110]),
    ("Glacier", [56, 196, 204]),
    ("Amethyst", [158, 112, 255]),
    ("Rose", [244, 92, 132]),
    ("Gold", [232, 182, 64]),
    ("Silver", [196, 200, 210]),
];
pub const DEFAULT_ACCENT: [u8; 3] = ACCENTS[0].1;

const fn pack(c: [u8; 3]) -> u32 {
    ((c[0] as u32) << 16) | ((c[1] as u32) << 8) | c[2] as u32
}

static ACCENT: AtomicU32 = AtomicU32::new(pack(DEFAULT_ACCENT));

/// The accent colour chosen in the settings.
pub fn accent() -> Color32 {
    let v = ACCENT.load(Ordering::Relaxed);
    Color32::from_rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
}

/// Use `c` as the accent colour, everywhere.
pub fn set_accent(ctx: &egui::Context, c: [u8; 3]) {
    ACCENT.store(pack(c), Ordering::Relaxed);
    ctx.set_visuals_of(Theme::Light, Palette::light().visuals());
    ctx.set_visuals_of(Theme::Dark, Palette::dark().visuals());
}

/// The `wght` axis of variable fonts.
pub const WGHT: u32 = u32::from_be_bytes(*b"wght");
/// Weights used in the interface.
pub const MEDIUM: f32 = 510.0;
pub const SEMIBOLD: f32 = 600.0;

/// The colours the custom-painted parts of the UI use. Derived from the
/// active egui theme so that both stay in sync.
#[derive(Debug, Clone, Copy)]
pub struct Palette {
    pub dark: bool,
    /// Behind the areas.
    pub app_bg: Color32,
    /// Area (card) background.
    pub panel: Color32,
    /// Raised surfaces inside a card: headers of tables, chips, tiles.
    pub header: Color32,
    /// Hairlines.
    pub border: Color32,
    pub text: Color32,
    pub weak: Color32,
    pub accent: Color32,
    /// Background of selected items: a neutral lift, not a colour.
    pub accent_soft: Color32,
    pub danger: Color32,
    pub success: Color32,
    /// Backdrop of the map and 3D canvases.
    pub canvas: Color32,
}

impl Palette {
    pub const LIGHT: Palette = Palette {
        dark: false,
        app_bg: Color32::from_rgb(222, 223, 227),
        panel: Color32::from_rgb(248, 248, 250),
        header: Color32::from_rgb(237, 238, 241),
        border: Color32::from_rgb(212, 213, 219),
        text: Color32::from_rgb(24, 24, 27),
        weak: Color32::from_rgb(113, 113, 122),
        accent: Color32::from_rgb(37, 99, 235),
        accent_soft: Color32::from_rgb(236, 236, 240),
        danger: Color32::from_rgb(220, 38, 38),
        success: Color32::from_rgb(22, 163, 74),
        canvas: Color32::from_rgb(228, 229, 233),
    };

    pub const DARK: Palette = Palette {
        dark: true,
        app_bg: Color32::from_rgb(11, 12, 14),
        panel: Color32::from_rgb(24, 25, 29),
        header: Color32::from_rgb(30, 31, 36),
        border: Color32::from_rgb(43, 45, 52),
        text: Color32::from_rgb(232, 233, 237),
        weak: Color32::from_rgb(142, 145, 155),
        accent: Color32::from_rgb(77, 144, 254),
        accent_soft: Color32::from_rgb(44, 46, 54),
        danger: Color32::from_rgb(244, 84, 84),
        success: Color32::from_rgb(52, 199, 120),
        canvas: Color32::from_rgb(17, 18, 21),
    };

    pub fn of(ui: &egui::Ui) -> Palette {
        if ui.visuals().dark_mode { Self::dark() } else { Self::light() }
    }

    /// The dark palette, with the accent of the settings.
    pub fn dark() -> Palette {
        Palette {
            accent: accent(),
            ..Self::DARK
        }
    }

    /// The light palette, with the accent of the settings.
    pub fn light() -> Palette {
        Palette {
            accent: accent(),
            ..Self::LIGHT
        }
    }

    /// Fill of a control at rest.
    pub fn control(&self) -> Color32 {
        if self.dark {
            Color32::from_rgb(35, 37, 43)
        } else {
            Color32::from_rgb(255, 255, 255)
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
        v.window_corner_radius = CornerRadius::same(14);
        v.menu_corner_radius = CornerRadius::same(10);
        let shadow = |blur: u8, alpha: u8| Shadow {
            offset: [0, 8],
            blur,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        };
        v.window_shadow = shadow(32, if self.dark { 150 } else { 40 });
        v.popup_shadow = shadow(20, if self.dark { 120 } else { 30 });
        v.extreme_bg_color = if self.dark {
            Color32::from_rgb(14, 14, 16)
        } else {
            Color32::from_rgb(250, 250, 251)
        };
        v.text_edit_bg_color = Some(v.extreme_bg_color);
        v.faint_bg_color = self.header;
        v.code_bg_color = self.header;
        v.hyperlink_color = self.accent;
        v.warn_fg_color = Color32::from_rgb(245, 166, 35);
        v.error_fg_color = self.danger;
        // Selected things are lifted, in the text colour, as in the
        // references: the accent is kept for focus, sliders and links.
        v.selection.bg_fill = self.accent_soft;
        v.selection.stroke = Stroke::new(1.0, self.text);
        v.slider_trailing_fill = true;
        v.handle_shape = egui::style::HandleShape::Circle;
        v.striped = false;
        v.indent_has_left_vline = false;
        v.button_frame = true;
        v.collapsing_header_frame = false;

        let lift = |c: Color32, by: i16| {
            let f = |x: u8| (x as i16 + by).clamp(0, 255) as u8;
            Color32::from_rgb(f(c.r()), f(c.g()), f(c.b()))
        };
        let up = if self.dark { 1 } else { -1 };
        let control = self.control();
        let w = &mut v.widgets;
        w.noninteractive.bg_fill = self.panel;
        w.noninteractive.weak_bg_fill = self.panel;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.border);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.text);
        w.noninteractive.corner_radius = radius;

        w.inactive.bg_fill = lift(control, 8 * up);
        w.inactive.weak_bg_fill = control;
        w.inactive.bg_stroke = Stroke::new(1.0, self.border);
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = lift(control, 14 * up);
        w.hovered.weak_bg_fill = lift(control, 8 * up);
        w.hovered.bg_stroke = Stroke::new(1.0, lift(self.border, 14 * up));
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = lift(control, 20 * up);
        w.active.weak_bg_fill = lift(control, 14 * up);
        w.active.bg_stroke = Stroke::new(1.0, lift(self.border, 24 * up));
        w.active.fg_stroke = Stroke::new(1.0, self.text);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = lift(control, 10 * up);
        w.open.weak_bg_fill = lift(control, 10 * up);
        w.open.bg_stroke = Stroke::new(1.0, self.border);
        w.open.fg_stroke = Stroke::new(1.0, self.text);
        w.open.corner_radius = radius;
        v
    }
}

/// Bold-ish text, with real weights where the typeface has them.
pub trait RichTextExt {
    /// Semi-bold: titles and values.
    fn bold(self) -> RichText;
    /// Medium: labels that should stand out a little.
    fn medium(self) -> RichText;
}

impl RichTextExt for RichText {
    fn bold(self) -> RichText {
        self.strong().variation(WGHT, SEMIBOLD)
    }

    fn medium(self) -> RichText {
        self.variation(WGHT, MEDIUM)
    }
}

/// The main action of a dialog or panel: a filled pill in the text colour.
pub fn primary_button(ui: &egui::Ui, text: impl Into<String>) -> egui::Button<'static> {
    let pal = Palette::of(ui);
    let fg = if pal.dark { Color32::from_rgb(12, 12, 14) } else { Color32::WHITE };
    egui::Button::new(RichText::new(text.into()).color(fg).variation(WGHT, SEMIBOLD))
        .fill(pal.text)
        .stroke(Stroke::NONE)
        .corner_radius(CornerRadius::same(255))
        .min_size(vec2(0.0, 28.0))
}

/// The system's interface typeface, when it can be read: SF Pro on macOS.
fn system_fonts() -> (Option<Vec<u8>>, Option<Vec<u8>>) {
    let read = |paths: &[&str]| paths.iter().find_map(|p| std::fs::read(p).ok());
    if cfg!(target_os = "macos") {
        (
            read(&["/System/Library/Fonts/SFNS.ttf"]),
            read(&["/System/Library/Fonts/SFNSMono.ttf"]),
        )
    } else if cfg!(target_os = "windows") {
        (read(&["C:\\Windows\\Fonts\\segoeui.ttf"]), read(&["C:\\Windows\\Fonts\\consola.ttf"]))
    } else {
        (
            read(&["/usr/share/fonts/truetype/inter/Inter-Regular.ttf", "/usr/share/fonts/opentype/inter/Inter-Regular.otf"]),
            None,
        )
    }
}

/// Install fonts, text sizes, spacing and both colour themes.
pub fn install(ctx: &egui::Context) {
    let (sans, mono) = system_fonts();
    if let Some(bytes) = sans {
        ctx.add_font(FontInsert::new(
            "system",
            FontData::from_owned(bytes),
            vec![InsertFontFamily {
                family: FontFamily::Proportional,
                priority: FontPriority::Highest,
            }],
        ));
    }
    if let Some(bytes) = mono {
        ctx.add_font(FontInsert::new(
            "system mono",
            FontData::from_owned(bytes),
            vec![InsertFontFamily {
                family: FontFamily::Monospace,
                priority: FontPriority::Highest,
            }],
        ));
    }
    // Lucide ships as an icon font: registered as a fallback, its glyphs can
    // be used inline in any label or button.
    let mut lucide = FontData::from_static(lucide_icons::LUCIDE_FONT_BYTES);
    lucide.tweak.scale = 1.08;
    lucide.tweak.y_offset_factor = 0.07;
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
            (TextStyle::Heading, FontId::new(18.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        style.spacing.item_spacing = vec2(8.0, 6.0);
        style.spacing.button_padding = vec2(10.0, 4.0);
        style.spacing.interact_size = vec2(28.0, 24.0);
        style.spacing.slider_width = 120.0;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.combo_height = 320.0;
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.window_margin = egui::Margin::same(16);
        style.spacing.icon_width = 15.0;
        style.spacing.icon_width_inner = 9.0;
        style.spacing.icon_spacing = 6.0;
        style.interaction.selectable_labels = false;
    });
    ctx.set_visuals_of(Theme::Light, Palette::light().visuals());
    ctx.set_visuals_of(Theme::Dark, Palette::dark().visuals());
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
