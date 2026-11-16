//! Look and feel: fonts, icons, and the light / dark palettes.
//!
//! The style follows current product design: neutral near-black (or
//! near-white) surfaces, cards with hairline borders and generous corners,
//! wells a shade darker for what is grouped, pill-shaped buttons, slim
//! sliders, neutral fills for what is selected, one accent used sparingly,
//! and the system's own typeface (SF Pro on macOS) — or one of those built
//! in, chosen in the settings — with real weights.

use egui::epaint::Shadow;
use egui::{
    Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, RichText, Stroke, TextStyle, Theme, Visuals, vec2,
};
pub use lucide_icons::Icon;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, OnceLock};

// Accessibility (Settings ▸ Accessibility) lives in the context's style,
// not in globals: high contrast in its visuals (a selection outlined twice
// as thick), reduced motion in its animations (none).

/// Whether `ctx` is in high contrast: stronger text, lines and edges.
pub fn high_contrast(ctx: &egui::Context) -> bool {
    is_high(&ctx.style_of(Theme::Light).visuals)
}

pub fn is_high(v: &Visuals) -> bool {
    v.selection.stroke.width > 1.5
}

/// Whether movement is kept to what is needed in `ctx`: views stop where
/// they are let go, pictures stay still.
pub fn reduce_motion(ctx: &egui::Context) -> bool {
    ctx.style_of(Theme::Light).animation_time == 0.0
}

/// Set high contrast and reduced motion: both palettes, and egui's own
/// animations (none while motion is reduced).
pub fn set_access(ctx: &egui::Context, high_contrast: bool, reduce_motion: bool) {
    ctx.set_visuals_of(Theme::Light, Palette::themed(false, high_contrast).visuals());
    ctx.set_visuals_of(Theme::Dark, Palette::themed(true, high_contrast).visuals());
    let time = if reduce_motion { 0.0 } else { egui::Style::default().animation_time };
    ctx.all_styles_mut(|style| style.animation_time = time);
}

/// The contrast ratio of two colours, as the WCAG has it: 1 (none) to 21.
pub fn contrast(a: Color32, b: Color32) -> f32 {
    let lum = |c: Color32| {
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
    };
    let (la, lb) = (lum(a), lum(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `c`, darkened (on a light `back`) or lightened (on a dark one) until it
/// stands out from `back` at least `ratio` to 1.
pub fn legible(c: Color32, back: Color32, ratio: f32) -> Color32 {
    let toward = if contrast(back, Color32::WHITE) > contrast(back, Color32::BLACK) { Color32::WHITE } else { Color32::BLACK };
    let mut t = 0.0;
    let mut out = c;
    while contrast(out, back) < ratio && t < 1.0 {
        t += 0.05;
        out = mix(c, toward, t);
    }
    out
}

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
    let high = high_contrast(ctx);
    ctx.set_visuals_of(Theme::Light, Palette::themed(false, high).visuals());
    ctx.set_visuals_of(Theme::Dark, Palette::themed(true, high).visuals());
}

/// Corner radii, in points: of an area (a card), of a dialog, of a popover
/// or menu, of a group of settings inside one, of a tile or small chip —
/// and of controls, which are pills.
pub const CARD_RADIUS: u8 = 12;
pub const WINDOW_RADIUS: u8 = 16;
pub const POPUP_RADIUS: u8 = 12;
pub const GROUP_RADIUS: u8 = 10;
pub const TILE_RADIUS: u8 = 8;
pub const PILL: u8 = 255;

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
    /// High contrast (Settings ▸ Accessibility).
    pub high: bool,
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
        high: false,
        app_bg: Color32::from_rgb(228, 228, 231),
        panel: Color32::from_rgb(250, 250, 251),
        header: Color32::from_rgb(240, 240, 242),
        border: Color32::from_rgb(222, 222, 226),
        text: Color32::from_rgb(23, 23, 25),
        weak: Color32::from_rgb(113, 113, 120),
        accent: Color32::from_rgb(37, 99, 235),
        accent_soft: Color32::from_rgb(234, 234, 237),
        danger: Color32::from_rgb(220, 38, 38),
        success: Color32::from_rgb(22, 163, 74),
        canvas: Color32::from_rgb(234, 234, 237),
    };

    // Neutral greys, as the references: no tint.
    pub const DARK: Palette = Palette {
        dark: true,
        high: false,
        app_bg: Color32::from_rgb(9, 9, 10),
        panel: Color32::from_rgb(20, 20, 21),
        header: Color32::from_rgb(29, 29, 31),
        border: Color32::from_rgb(40, 40, 43),
        text: Color32::from_rgb(237, 237, 239),
        weak: Color32::from_rgb(141, 141, 147),
        accent: Color32::from_rgb(77, 144, 254),
        accent_soft: Color32::from_rgb(44, 44, 47),
        danger: Color32::from_rgb(244, 84, 84),
        success: Color32::from_rgb(52, 199, 120),
        canvas: Color32::from_rgb(14, 14, 15),
    };

    /// High contrast, light: black text, muted text still past 7 to 1 on
    /// the cards, lines that stand out (3 to 1 at least), white cards on a
    /// darker ground.
    pub const LIGHT_HIGH: Palette = Palette {
        dark: false,
        high: true,
        app_bg: Color32::from_rgb(204, 204, 210),
        panel: Color32::from_rgb(255, 255, 255),
        header: Color32::from_rgb(236, 236, 240),
        border: Color32::from_rgb(124, 124, 132),
        text: Color32::from_rgb(6, 6, 8),
        weak: Color32::from_rgb(70, 70, 78),
        accent: Color32::from_rgb(37, 99, 235),
        accent_soft: Color32::from_rgb(212, 212, 220),
        danger: Color32::from_rgb(178, 20, 20),
        success: Color32::from_rgb(10, 118, 50),
        canvas: Color32::from_rgb(226, 226, 231),
    };

    /// High contrast, dark: white text on black, the same ratios.
    pub const DARK_HIGH: Palette = Palette {
        dark: true,
        high: true,
        app_bg: Color32::from_rgb(0, 0, 0),
        panel: Color32::from_rgb(10, 10, 11),
        header: Color32::from_rgb(30, 30, 33),
        border: Color32::from_rgb(122, 122, 130),
        text: Color32::from_rgb(255, 255, 255),
        weak: Color32::from_rgb(198, 198, 206),
        accent: Color32::from_rgb(77, 144, 254),
        accent_soft: Color32::from_rgb(66, 66, 72),
        danger: Color32::from_rgb(255, 118, 118),
        success: Color32::from_rgb(90, 226, 150),
        canvas: Color32::from_rgb(4, 4, 5),
    };

    pub fn of(ui: &egui::Ui) -> Palette {
        Self::themed(ui.visuals().dark_mode, is_high(ui.visuals()))
    }

    /// The dark or light palette, in high contrast or not, with the accent
    /// of the settings (in high contrast, darkened or lightened until it
    /// reads on the cards).
    pub fn themed(dark: bool, high: bool) -> Palette {
        let base = match (dark, high) {
            (true, false) => Self::DARK,
            (true, true) => Self::DARK_HIGH,
            (false, false) => Self::LIGHT,
            (false, true) => Self::LIGHT_HIGH,
        };
        let accent = if high { legible(accent(), base.panel, 4.5) } else { accent() };
        Palette { accent, ..base }
    }

    /// The dark palette, with the accent of the settings.
    pub fn dark() -> Palette {
        Self::themed(true, false)
    }

    /// The light palette, with the accent of the settings.
    pub fn light() -> Palette {
        Self::themed(false, false)
    }

    /// Fill of a control at rest.
    pub fn control(&self) -> Color32 {
        match (self.dark, self.high) {
            (true, false) => Color32::from_rgb(33, 33, 35),
            (true, true) => Color32::from_rgb(22, 22, 24),
            (false, _) => Color32::from_rgb(255, 255, 255),
        }
    }

    /// Fill of a well: what groups settings, a shade under the card.
    pub fn well(&self) -> Color32 {
        match (self.dark, self.high) {
            (true, false) => Color32::from_rgb(14, 14, 15),
            (true, true) => Color32::from_rgb(0, 0, 0),
            (false, false) => Color32::from_rgb(244, 244, 246),
            (false, true) => Color32::from_rgb(240, 240, 244),
        }
    }

    /// The hairline around a control: a little lighter than its fill (in
    /// high contrast, a line that stands out).
    pub fn control_border(&self) -> Color32 {
        match (self.dark, self.high) {
            (true, false) => Color32::from_rgb(50, 50, 53),
            (true, true) => Color32::from_rgb(150, 150, 158),
            (false, false) => Color32::from_rgb(214, 214, 219),
            (false, true) => Color32::from_rgb(98, 98, 106),
        }
    }

    /// The ink that reads on the accent: dark on a light one.
    pub fn on_accent(&self) -> Color32 {
        let a = self.accent;
        let luma = 0.299 * a.r() as f32 + 0.587 * a.g() as f32 + 0.114 * a.b() as f32;
        if luma > 165.0 { Color32::from_rgb(16, 16, 18) } else { Color32::WHITE }
    }

    fn visuals(&self) -> Visuals {
        let mut v = if self.dark {
            Visuals::dark()
        } else {
            Visuals::light()
        };
        // Buttons, fields and tabs are pills; what only frames is rounded.
        let radius = CornerRadius::same(PILL);
        v.panel_fill = self.panel;
        v.window_fill = self.panel;
        v.window_stroke = Stroke::new(1.0, self.border);
        v.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
        v.menu_corner_radius = CornerRadius::same(POPUP_RADIUS);
        let shadow = |blur: u8, alpha: u8| Shadow {
            offset: [0, 8],
            blur,
            spread: 0,
            color: Color32::from_black_alpha(alpha),
        };
        v.window_shadow = shadow(32, if self.dark { 150 } else { 40 });
        v.popup_shadow = shadow(20, if self.dark { 120 } else { 30 });
        v.extreme_bg_color = if self.dark { self.well() } else { Color32::WHITE };
        v.text_edit_bg_color = Some(v.extreme_bg_color);
        v.faint_bg_color = self.header;
        v.code_bg_color = self.header;
        v.hyperlink_color = self.accent;
        v.warn_fg_color = Color32::from_rgb(245, 166, 35);
        v.error_fg_color = self.danger;
        // Selected things are lifted, in the text colour, as in the
        // references: the accent is kept for focus, sliders and links.
        v.selection.bg_fill = self.accent_soft;
        v.selection.stroke = Stroke::new(if self.high { 2.0 } else { 1.0 }, self.text);
        if self.high {
            // Muted text in its own colour, not faded; what cannot be used
            // still legible.
            v.weak_text_color = Some(self.weak);
            v.disabled_alpha = v.disabled_alpha.max(0.62);
        }
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
        w.noninteractive.corner_radius = CornerRadius::same(GROUP_RADIUS);

        w.inactive.bg_fill = lift(control, 8 * up);
        w.inactive.weak_bg_fill = control;
        w.inactive.bg_stroke = Stroke::new(1.0, self.control_border());
        w.inactive.fg_stroke = Stroke::new(1.0, self.text);
        w.inactive.corner_radius = radius;
        w.inactive.expansion = 0.0;

        w.hovered.bg_fill = lift(control, 14 * up);
        w.hovered.weak_bg_fill = lift(control, 8 * up);
        w.hovered.bg_stroke = Stroke::new(1.0, lift(self.control_border(), 12 * up));
        w.hovered.fg_stroke = Stroke::new(1.0, self.text);
        w.hovered.corner_radius = radius;
        w.hovered.expansion = 0.0;

        w.active.bg_fill = lift(control, 20 * up);
        w.active.weak_bg_fill = lift(control, 14 * up);
        w.active.bg_stroke = Stroke::new(1.0, lift(self.control_border(), 22 * up));
        w.active.fg_stroke = Stroke::new(1.0, self.text);
        w.active.corner_radius = radius;
        w.active.expansion = 0.0;

        w.open.bg_fill = lift(control, 10 * up);
        w.open.weak_bg_fill = lift(control, 10 * up);
        w.open.bg_stroke = Stroke::new(1.0, self.control_border());
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

/// The system's interface typeface and its monospaced one, when they can be
/// read: SF Pro and SF Mono on macOS, Segoe UI and Consolas on Windows.
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

/// The system's interface font and its monospaced one, if read.
type Faces = (Option<&'static [u8]>, Option<&'static [u8]>);

/// The system's fonts, read once and kept for as long as Massif runs.
fn system_faces() -> Faces {
    static FACES: OnceLock<Faces> = OnceLock::new();
    *FACES.get_or_init(|| {
        let keep = |bytes: Option<Vec<u8>>| bytes.map(|b| &*Box::leak(b.into_boxed_slice()));
        let (sans, mono) = system_fonts();
        (keep(sans), keep(mono))
    })
}

/// The typefaces the interface can be set in: the system's own, or one of
/// those built into Massif — variable fonts under the SIL Open Font
/// License, each with real weights (assets/fonts).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum Typeface {
    #[default]
    System,
    Inter,
    Geist,
    IbmPlexSans,
    Manrope,
}

impl Typeface {
    pub const ALL: [Typeface; 5] = [Typeface::System, Typeface::Inter, Typeface::Geist, Typeface::IbmPlexSans, Typeface::Manrope];

    pub fn name(self) -> &'static str {
        match self {
            Typeface::System => "System",
            Typeface::Inter => "Inter",
            Typeface::Geist => "Geist",
            Typeface::IbmPlexSans => "IBM Plex Sans",
            Typeface::Manrope => "Manrope",
        }
    }

    /// What it is like, in a few words.
    pub fn about(self) -> &'static str {
        match self {
            Typeface::System => {
                if cfg!(target_os = "macos") {
                    "SF Pro, as the rest of macOS"
                } else if cfg!(target_os = "windows") {
                    "Segoe UI, as the rest of Windows"
                } else {
                    "The system's own, or Inter"
                }
            }
            Typeface::Inter => "Drawn for screens: neutral, very clear when small",
            Typeface::Geist => "A crisp, geometric grotesque",
            Typeface::IbmPlexSans => "Engineered, with a technical character",
            Typeface::Manrope => "Open and rounded, a little warmer",
        }
    }

    /// The font file: the system's, or Inter where it cannot be read.
    fn bytes(self) -> &'static [u8] {
        const INTER: &[u8] = include_bytes!("../assets/fonts/Inter.ttf");
        match self {
            Typeface::System => system_faces().0.unwrap_or(INTER),
            Typeface::Inter => INTER,
            Typeface::Geist => include_bytes!("../assets/fonts/Geist.ttf"),
            Typeface::IbmPlexSans => include_bytes!("../assets/fonts/IBMPlexSans.ttf"),
            Typeface::Manrope => include_bytes!("../assets/fonts/Manrope.ttf"),
        }
    }

    /// Its name among the fonts.
    fn key(self) -> &'static str {
        match self {
            Typeface::System => "system",
            Typeface::Inter => "Inter",
            Typeface::Geist => "Geist",
            Typeface::IbmPlexSans => "IBM Plex Sans",
            Typeface::Manrope => "Manrope",
        }
    }

    /// A family of its own, whichever is chosen: to show it in the settings.
    pub fn family(self) -> FontFamily {
        FontFamily::Name(format!("typeface {}", self.key()).into())
    }
}

static TYPEFACE: AtomicU32 = AtomicU32::new(0);

/// The typeface the interface is set in.
pub fn typeface() -> Typeface {
    Typeface::ALL[TYPEFACE.load(Ordering::Relaxed) as usize % Typeface::ALL.len()]
}

/// Set the interface in `face`: it comes first, the system's typeface and
/// egui's after it for what it lacks, Lucide's icons last; numbers and code
/// stay in the system's monospaced font. Every typeface also has a family
/// of its own, for the settings to show each in itself.
pub fn set_typeface(ctx: &egui::Context, face: Typeface) {
    TYPEFACE.store(Typeface::ALL.iter().position(|&f| f == face).unwrap_or(0) as u32, Ordering::Relaxed);
    let mut fonts = FontDefinitions::default();
    let fallback = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    for f in Typeface::ALL {
        fonts.font_data.insert(f.key().to_owned(), Arc::new(FontData::from_static(f.bytes())));
    }
    if let Some(mono) = system_faces().1 {
        fonts.font_data.insert("system mono".to_owned(), Arc::new(FontData::from_static(mono)));
    }
    // Lucide ships as an icon font, its glyphs used inline in any label or
    // button: after the typefaces, which have none where they are, and
    // before egui's own fonts, whose few icons sit where some of its are.
    let mut lucide = FontData::from_static(lucide_icons::LUCIDE_FONT_BYTES);
    lucide.tweak.scale = 1.08;
    lucide.tweak.y_offset_factor = 0.07;
    fonts.font_data.insert("lucide".to_owned(), Arc::new(lucide));

    let family = |first: Typeface| -> Vec<String> {
        let mut names = vec![first.key().to_owned()];
        if first != Typeface::System {
            names.push(Typeface::System.key().to_owned());
        }
        names.push("lucide".to_owned());
        names.extend(fallback.iter().cloned());
        names
    };
    fonts.families.insert(FontFamily::Proportional, family(face));
    for f in Typeface::ALL {
        fonts.families.insert(f.family(), family(f));
    }
    let mono = fonts.families.entry(FontFamily::Monospace).or_default();
    if system_faces().1.is_some() {
        mono.insert(0, "system mono".to_owned());
    }
    mono.insert(1.min(mono.len()), "lucide".to_owned());
    ctx.set_fonts(fonts);
}

/// Install fonts, text sizes, spacing and both colour themes.
pub fn install(ctx: &egui::Context) {
    set_typeface(ctx, typeface());

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
        style.spacing.button_padding = vec2(12.0, 4.0);
        style.spacing.interact_size = vec2(28.0, 26.0);
        style.spacing.slider_width = 120.0;
        style.spacing.slider_rail_height = 4.0;
        style.spacing.combo_height = 320.0;
        style.spacing.menu_margin = egui::Margin::same(8);
        style.spacing.window_margin = egui::Margin::same(20);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// High contrast is what it says, in both themes and with every accent:
    /// text past 15 to 1 on the cards, muted text past 7 (WCAG AAA), lines
    /// and controls' edges past 3, the accent past 4.5 — and more than the
    /// standard palettes everywhere.
    #[test]
    fn high_contrast_reaches_its_ratios() {
        for dark in [false, true] {
            let (std, high) = (Palette::themed(dark, false), Palette::themed(dark, true));
            assert!(high.high && !std.high);
            let name = if dark { "dark" } else { "light" };
            assert!(contrast(high.text, high.panel) >= 15.0, "{name}: text {}", contrast(high.text, high.panel));
            assert!(contrast(high.weak, high.panel) >= 7.0, "{name}: muted text {}", contrast(high.weak, high.panel));
            assert!(contrast(high.weak, high.well()) >= 7.0, "{name}: muted text in a well {}", contrast(high.weak, high.well()));
            assert!(contrast(high.border, high.panel) >= 3.0, "{name}: lines {}", contrast(high.border, high.panel));
            assert!(contrast(high.control_border(), high.control()) >= 3.0, "{name}: controls' edges {}", contrast(high.control_border(), high.control()));
            assert!(contrast(high.weak, high.panel) > contrast(std.weak, std.panel));
            assert!(contrast(high.border, high.panel) > contrast(std.border, std.panel));
            for (accent_name, c) in ACCENTS {
                let a = legible(rgb(c), high.panel, 4.5);
                assert!(contrast(a, high.panel) >= 4.5, "{name}: {accent_name} {}", contrast(a, high.panel));
            }
        }
        // WCAG's own examples.
        assert!((contrast(Color32::BLACK, Color32::WHITE) - 21.0).abs() < 0.01);
        assert!((contrast(Color32::from_rgb(118, 118, 118), Color32::WHITE) - 4.54).abs() < 0.02);
    }

    /// High contrast and reduced motion live in a context's style: set in
    /// one, not in another.
    #[test]
    fn access_is_per_context() {
        let (a, b) = (egui::Context::default(), egui::Context::default());
        install(&a);
        install(&b);
        set_access(&a, true, true);
        assert!(high_contrast(&a) && reduce_motion(&a));
        assert!(!high_contrast(&b) && !reduce_motion(&b));
        // Setting the accent keeps high contrast.
        set_accent(&a, accent().to_array()[..3].try_into().unwrap());
        assert!(high_contrast(&a));
        set_access(&a, false, false);
        assert!(!high_contrast(&a) && !reduce_motion(&a));
    }

    /// Every typeface reads, and has the weights the interface asks for.
    #[test]
    fn the_typefaces_have_real_weights() {
        for face in Typeface::ALL {
            let axes = FontData::from_static(face.bytes()).variation_axes();
            let weight = axes.iter().find(|a| a.tag == "wght");
            let Some(weight) = weight else {
                // The system's typeface may be a static one, off macOS.
                assert!(face == Typeface::System && !cfg!(target_os = "macos"), "{} has no weight axis", face.name());
                continue;
            };
            assert!(weight.range.min <= 400.0 && weight.range.max >= SEMIBOLD, "{}: weights {:?}", face.name(), weight.range);
        }
    }

    /// Lucide's icons show whichever typeface is chosen: none puts glyphs
    /// of its own where they are (Inter and Manrope had some).
    #[test]
    fn icons_show_in_every_typeface() {
        let ctx = egui::Context::default();
        set_typeface(&ctx, Typeface::System);
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
        for (face, family) in Typeface::ALL.map(|f| (f, f.family())).into_iter().chain([(Typeface::System, FontFamily::Proportional)]) {
            let chars = ctx.fonts_mut(|f| f.characters(&family).clone());
            let icons = chars.iter().filter(|(_, names)| names.iter().any(|n| n == "lucide") && names[0] != "lucide");
            let hidden: Vec<u32> = icons.map(|(c, _)| *c as u32).filter(|c| (0xE000..=0xF8FF).contains(c)).collect();
            assert!(hidden.is_empty(), "{} hides {} icons, from U+{:04X}", face.name(), hidden.len(), hidden.first().copied().unwrap_or(0));
        }
    }
}
