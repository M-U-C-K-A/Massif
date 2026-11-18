//! The background of the home screen, one of four styles:
//! - **Ridges**: ranges of mountains under a glow of the accent, dithered
//!   into dots, drawn on the processor into a texture of a cell every few
//!   points (see [`frame`]);
//! - **Waves**: smooth bands of colour rising from one side and melting into
//!   each other, a deep colour beyond them;
//! - **Pixel**: the same waves in large square pixels;
//! - **Prism**: columns of colour, crisp side by side, each a gradient,
//!   their tops drawing a shape that moves.
//!
//! The last three are meshes whose corners are coloured here and blended
//! by the graphics card (see [`paint`]): a few thousand colours a frame.
//! Whatever the style, its light is held toward the page's colour where the
//! home screen's words are, so that they stay legible; in high contrast the
//! colours turn grey and calm.

use crate::theme::{Icon, Palette};
use egui::{Color32, Pos2, Rect, pos2, vec2};
use serde::{Deserialize, Serialize};

/// Frames a second of the background while it moves.
pub const FPS: f64 = 12.0;

/// What the background shows.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Style {
    /// Ranges of mountains seen from afar, under a glow of the accent.
    #[default]
    Ridges,
    /// Bands of colour that rise and melt into each other.
    Waves,
    /// The waves in large square pixels.
    Pixel,
    /// Columns of colour drawing a shape.
    Prism,
}

impl Style {
    pub const ALL: [Style; 4] = [Style::Ridges, Style::Waves, Style::Pixel, Style::Prism];

    pub fn label(self) -> &'static str {
        match self {
            Style::Ridges => tn!("Ridges"),
            Style::Waves => tn!("Waves"),
            Style::Pixel => tn!("Pixel"),
            Style::Prism => tn!("Prism"),
        }
    }
}

/// Where the bands rise toward, or the columns grow.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Direction {
    #[default]
    Up,
    Right,
    Down,
    Left,
}

impl Direction {
    pub const ALL: [Direction; 4] = [Direction::Up, Direction::Right, Direction::Down, Direction::Left];

    pub fn label(self) -> &'static str {
        match self {
            Direction::Up => tn!("Upward"),
            Direction::Right => tn!("To the right"),
            Direction::Down => tn!("Downward"),
            Direction::Left => tn!("To the left"),
        }
    }

    /// Its arrow.
    pub fn icon(self) -> Icon {
        match self {
            Direction::Up => Icon::ArrowUp,
            Direction::Right => Icon::ArrowRight,
            Direction::Down => Icon::ArrowDown,
            Direction::Left => Icon::ArrowLeft,
        }
    }
}

/// The shape the prism's columns draw.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Shape {
    /// An arch, high in the middle.
    #[default]
    Crest,
    /// Low in the middle, high at the sides.
    Valley,
    /// A swell rolling across.
    Tide,
    /// Rising from one side to the other.
    Slant,
    /// Round, as a lens.
    Lens,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Crest, Shape::Valley, Shape::Tide, Shape::Slant, Shape::Lens];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Crest => tn!("Crest"),
            Shape::Valley => tn!("Valley"),
            Shape::Tide => tn!("Tide"),
            Shape::Slant => tn!("Slant"),
            Shape::Lens => tn!("Lens"),
        }
    }

    /// How tall a column at `a` (0–1 across) stands, 0–1.
    fn profile(self, a: f32) -> f32 {
        let d = a - 0.5;
        match self {
            Shape::Crest => 0.08 + 0.92 * (-(d / 0.2).powi(2)).exp(),
            Shape::Valley => 0.25 + 0.75 * (1.0 - (-(d / 0.26).powi(2)).exp()),
            Shape::Tide => 0.5 + 0.38 * (std::f32::consts::TAU * (a * 1.2 + 0.1)).sin(),
            Shape::Slant => 0.12 + 0.82 * a,
            Shape::Lens => (1.0 - (d / 0.42).powi(2)).max(0.0).sqrt() * 0.95 + 0.05,
        }
    }
}

/// How the prism's columns move.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum Motion {
    /// Swells running out from the middle.
    #[default]
    Expand,
    /// Swells running in toward the middle.
    Gather,
    /// All of it rising and falling together.
    Breathe,
}

impl Motion {
    pub const ALL: [Motion; 3] = [Motion::Expand, Motion::Gather, Motion::Breathe];

    pub fn label(self) -> &'static str {
        match self {
            Motion::Expand => tn!("Expand"),
            Motion::Gather => tn!("Gather"),
            Motion::Breathe => tn!("Breathe"),
        }
    }
}

/// The colours of the waves and the pixels.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum WavesPalette {
    /// The accent of the settings, on the theme's page.
    Theme,
    #[default]
    Flamingo,
    Lagoon,
    Ember,
    Glacier,
    Ink,
}

impl WavesPalette {
    pub const ALL: [WavesPalette; 6] = [WavesPalette::Theme, WavesPalette::Flamingo, WavesPalette::Lagoon, WavesPalette::Ember, WavesPalette::Glacier, WavesPalette::Ink];

    pub fn label(self) -> &'static str {
        match self {
            WavesPalette::Theme => tn!("Theme"),
            WavesPalette::Flamingo => tn!("Flamingo"),
            WavesPalette::Lagoon => tn!("Lagoon"),
            WavesPalette::Ember => tn!("Ember"),
            WavesPalette::Glacier => tn!("Glacier"),
            WavesPalette::Ink => tn!("Ink"),
        }
    }

    /// Its six colours, deep to light; none for the theme's.
    fn colours(self) -> Option<[u32; 6]> {
        match self {
            WavesPalette::Theme => None,
            WavesPalette::Flamingo => Some([0x140741, 0x391b99, 0xb02a9a, 0xf3528d, 0xf7a3c3, 0xfff0f3]),
            WavesPalette::Lagoon => Some([0x04152b, 0x0b3c5d, 0x1c7c8c, 0x39b5a4, 0x9be3c7, 0xeffbf5]),
            WavesPalette::Ember => Some([0x1a0710, 0x4e2337, 0xa3342c, 0xe8763a, 0xf4b26b, 0xfbe8d2]),
            WavesPalette::Glacier => Some([0x0a1626, 0x1d3a5f, 0x3e6e9e, 0x7fa9d6, 0xc3ddf2, 0xf2f8fd]),
            WavesPalette::Ink => Some([0x0b0b0e, 0x23232b, 0x44444f, 0x7a7a86, 0xbdbdc6, 0xf3f3f6]),
        }
    }

    /// Its colours as a swatch shows them, top to ground, for `pal`.
    pub fn swatch(self, pal: &Palette, page: Color32) -> Vec<Color32> {
        waves_ramp(self, pal, rgb_f(page)).into_iter().map(to_color).collect()
    }
}

/// The colours of the prism.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default, Serialize, Deserialize)]
pub enum PrismPalette {
    /// The accent of the settings, on the theme's page.
    Theme,
    #[default]
    Ember,
    Ultraviolet,
    Petrol,
    Citron,
    BlueHour,
}

impl PrismPalette {
    pub const ALL: [PrismPalette; 6] = [PrismPalette::Theme, PrismPalette::Ember, PrismPalette::Ultraviolet, PrismPalette::Petrol, PrismPalette::Citron, PrismPalette::BlueHour];

    pub fn label(self) -> &'static str {
        match self {
            PrismPalette::Theme => tn!("Theme"),
            PrismPalette::Ember => tn!("Ember"),
            PrismPalette::Ultraviolet => tn!("Ultraviolet"),
            PrismPalette::Petrol => tn!("Petrol"),
            PrismPalette::Citron => tn!("Citron"),
            PrismPalette::BlueHour => tn!("Blue hour"),
        }
    }

    /// Its colours: the ground behind the columns, then a column's from its
    /// top to its foot (dark, middle, hot, light); none for the theme's.
    fn colours(self) -> Option<[u32; 5]> {
        match self {
            PrismPalette::Theme => None,
            PrismPalette::Ember => Some([0x4e2337, 0x7a2c3c, 0xd9663f, 0xf2b36e, 0xf8e3c6]),
            PrismPalette::Ultraviolet => Some([0x1b1240, 0x3a2a8c, 0x7a4ce0, 0xc88cf5, 0xf3e6ff]),
            PrismPalette::Petrol => Some([0x0e2a33, 0x134e5e, 0x1f8a8a, 0x7fd1c3, 0xe6f7f2]),
            PrismPalette::Citron => Some([0x1f2a12, 0x3e5a1e, 0x8fb339, 0xd8e36a, 0xfaf8d8]),
            PrismPalette::BlueHour => Some([0x0d1530, 0x22346e, 0x4c64b8, 0xa4b6ea, 0xeef2ff]),
        }
    }

    /// Its colours as a swatch shows them, for `pal`.
    pub fn swatch(self, pal: &Palette, page: Color32) -> Vec<Color32> {
        prism_ramp(self, pal, rgb_f(page)).iter().copied().map(to_color).collect()
    }
}

/// The waves' settings.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Waves {
    pub palette: WavesPalette,
    /// How broad the swells are: 1 as made, from 0.5 to 2.
    pub scale: f32,
    /// How soft the seams between the bands are, 0–1.
    pub melt: f32,
    /// How fast they move: 1 as made, from 0 to 2.
    pub speed: f32,
    pub direction: Direction,
}

impl Default for Waves {
    fn default() -> Self {
        Self { palette: WavesPalette::Flamingo, scale: 1.0, melt: 0.5, speed: 1.0, direction: Direction::Up }
    }
}

/// The pixels' settings.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Pixels {
    pub palette: WavesPalette,
    /// Pixels across the background, from 12 to 96.
    pub across: f32,
    pub speed: f32,
}

impl Default for Pixels {
    fn default() -> Self {
        Self { palette: WavesPalette::Flamingo, across: 36.0, speed: 1.0 }
    }
}

/// The prism's settings.
#[derive(Clone, Copy, PartialEq, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Prism {
    pub palette: PrismPalette,
    pub shape: Shape,
    pub direction: Direction,
    pub motion: Motion,
    pub speed: f32,
}

impl Default for Prism {
    fn default() -> Self {
        Self { palette: PrismPalette::Ember, shape: Shape::Crest, direction: Direction::Up, motion: Motion::Expand, speed: 1.0 }
    }
}

/// The background as the settings have it: its style, and each style's
/// own settings, kept when another is shown.
#[derive(Clone, Copy, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Background {
    pub style: Style,
    pub waves: Waves,
    pub pixel: Pixels,
    pub prism: Prism,
}

// ------------------------------------------------------------- colours --

/// The relative luminance of `c` (0–1 channels), as contrast ratios take it.
pub fn luminance(c: [f32; 3]) -> f32 {
    let lin = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    0.2126 * lin(c[0]) + 0.7152 * lin(c[1]) + 0.0722 * lin(c[2])
}

/// `c` as red, green and blue from 0 to 1.
pub fn rgb_f(c: Color32) -> [f32; 3] {
    [c.r(), c.g(), c.b()].map(|v| v as f32 / 255.0)
}

fn hex(v: u32) -> [f32; 3] {
    [(v >> 16) & 0xff, (v >> 8) & 0xff, v & 0xff].map(|c| c as f32 / 255.0)
}

fn to_color(c: [f32; 3]) -> Color32 {
    let [r, g, b] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    Color32::from_rgb(r, g, b)
}

fn lerp3(a: [f32; 3], b: [f32; 3], t: f32) -> [f32; 3] {
    [0, 1, 2].map(|i| a[i] + (b[i] - a[i]) * t)
}

/// `c` with its hue turned by `degrees`, a little less saturated.
fn hue_shift(c: [f32; 3], degrees: f32) -> [f32; 3] {
    let [r, g, b] = c;
    let (max, min) = (r.max(g).max(b), r.min(g).min(b));
    let d = max - min;
    if d <= 1e-5 {
        return c;
    }
    let h = if max == r { ((g - b) / d).rem_euclid(6.0) } else if max == g { (b - r) / d + 2.0 } else { (r - g) / d + 4.0 };
    let h = (h * 60.0 + degrees).rem_euclid(360.0) / 60.0;
    let (v, s) = (max, 0.85 * d / max);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let m = v - c;
    let [r, g, b] = match h as u32 {
        0 => [c, x, 0.0],
        1 => [x, c, 0.0],
        2 => [0.0, c, x],
        3 => [0.0, x, c],
        4 => [x, 0.0, c],
        _ => [c, 0.0, x],
    };
    [r + m, g + m, b + m]
}

/// In high contrast, colours turn to greys of their own lightness, held
/// near the page's colour: calm.
fn calm(c: [f32; 3], page: [f32; 3]) -> [f32; 3] {
    let l = luminance(c).powf(1.0 / 2.2);
    lerp3(page, [l; 3], 0.45)
}

/// The six colours of the waves, from the top to the ground: in the same
/// order on both pages, deep at the top and light at the foot. On the light
/// page they are softened toward it — the top a pale tint, no colour
/// darker than a pastel, the ground not blown out to white — for a calm
/// picture behind dark words.
fn waves_ramp(palette: WavesPalette, pal: &Palette, page: [f32; 3]) -> [[f32; 3]; 6] {
    let mut ramp = match palette.colours() {
        Some(c) => {
            let c = c.map(hex);
            if pal.dark {
                c
            } else {
                [lerp3(page, c[1], 0.22), lerp3(page, c[1], 0.32), lerp3(page, c[2], 0.42), lerp3(page, c[3], 0.5), lerp3(page, c[4], 0.52), lerp3(page, c[4], 0.36)]
            }
        }
        None => {
            let a = rgb_f(pal.accent);
            if pal.dark {
                let far = [1.0, 0.97, 0.94];
                [lerp3(page, a, 0.1), lerp3(page, hue_shift(a, -30.0), 0.32), lerp3(page, hue_shift(a, -14.0), 0.62), a, lerp3(a, far, 0.4), lerp3(a, far, 0.72)]
            } else {
                let (cool, warm) = (hue_shift(a, -30.0), hue_shift(a, -14.0));
                [lerp3(page, cool, 0.2), lerp3(page, cool, 0.3), lerp3(page, warm, 0.4), lerp3(page, a, 0.5), lerp3(page, a, 0.42), lerp3(page, a, 0.28)]
            }
        }
    };
    if pal.high {
        ramp = ramp.map(|c| calm(c, page));
    }
    ramp
}

/// The colours of the prism: the ground behind the columns, then a
/// column's from its top to its foot.
fn prism_ramp(palette: PrismPalette, pal: &Palette, page: [f32; 3]) -> [[f32; 3]; 5] {
    let mut ramp = match palette.colours() {
        Some(c) => {
            let c = c.map(hex);
            // On the light page, a pale ground, and the columns the other way
            // up: deep at their foot, light at their tips.
            if pal.dark { c } else { [lerp3(page, c[4], 0.55), c[3], c[2], c[1], c[0]] }
        }
        None => {
            let a = rgb_f(pal.accent);
            if pal.dark {
                [lerp3(page, a, 0.08), lerp3(page, hue_shift(a, -30.0), 0.4), lerp3(page, hue_shift(a, -12.0), 0.75), a, lerp3(a, [1.0, 0.97, 0.94], 0.6)]
            } else {
                [lerp3(page, a, 0.1), lerp3(page, a, 0.45), lerp3(page, hue_shift(a, -12.0), 0.8), a, lerp3(hue_shift(a, -30.0), [0.12, 0.08, 0.1], 0.35)]
            }
        }
    };
    if pal.high {
        ramp = ramp.map(|c| calm(c, page));
    }
    ramp
}

fn smooth(a: f32, b: f32, x: f32) -> f32 {
    let t = ((x - a) / (b - a)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

// --------------------------------------------------------- legibility --

/// Holds the light of the smooth styles toward the page's colour where the
/// words are: a colour may be at most (dark theme) or at least (light) so
/// luminous that muted words keep 3.2:1 on it (4.6:1 in high contrast) — a
/// scrim only as strong as the colours under it ask, fading out softly and
/// roundly around the words, so that it reads as a haze, not a card.
pub struct Guard {
    dark: bool,
    page: [f32; 3],
    page_l: f32,
    limit: f32,
    /// The words' column, in points, and how far its hold fades out
    /// around it: to its sides, above it and below it.
    column: Option<Rect>,
    soft: [f32; 3],
    /// A channel (in 1024ths) to its light.
    to_light: Vec<f32>,
}

impl Guard {
    pub fn new(pal: &Palette, page: Color32, muted: Color32, column: Option<Rect>, area: Rect) -> Self {
        let page = rgb_f(page);
        let muted = luminance(rgb_f(muted));
        let ratio = if pal.high { 4.6 } else { 3.2 };
        let limit = if pal.dark { (muted + 0.05) / ratio - 0.05 } else { ratio * (muted + 0.05) - 0.05 };
        let to_light = (0..=1024)
            .map(|j| {
                let v = j as f32 / 1024.0;
                if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            })
            .collect();
        // Below the words, where the colours are, it fades out sooner.
        let soft = [0.22 * area.width(), 0.2 * area.height(), 0.1 * area.height()];
        Self { dark: pal.dark, page, page_l: luminance(page), limit, column, soft, to_light }
    }

    /// How much the words are there at `p`, 0–1: all of it over their
    /// column, less and less away from it, round at its corners.
    fn weight(&self, p: Pos2) -> f32 {
        let Some(c) = self.column else { return 0.0 };
        let dx = (c.left() - p.x).max(p.x - c.right()).max(0.0) / self.soft[0];
        let dy = if p.y < c.top() { (c.top() - p.y) / self.soft[1] } else { (p.y - c.bottom()).max(0.0) / self.soft[2] };
        let d = (dx * dx + dy * dy).sqrt().min(1.0);
        1.0 - d * d * (3.0 - 2.0 * d)
    }

    #[inline]
    fn luminance(&self, c: [f32; 3]) -> f32 {
        let at = |v: f32| self.to_light[(v.clamp(0.0, 1.0) * 1024.0) as usize];
        0.2126 * at(c[0]) + 0.7152 * at(c[1]) + 0.0722 * at(c[2])
    }

    /// Colour `c` at `p`, held.
    fn hold(&self, c: [f32; 3], p: Pos2) -> [f32; 3] {
        let m = self.weight(p);
        if m <= 0.0 {
            return c;
        }
        let allowed = if self.dark { 1.0 + (self.limit - 1.0) * m } else { self.limit * m };
        let lc = self.luminance(c);
        let (room, far) = if self.dark { (allowed - self.page_l, lc - self.page_l) } else { (self.page_l - allowed, self.page_l - lc) };
        if room <= 0.0 {
            return self.page;
        }
        // The knee where it starts gives way from all the room (no change,
        // where the words are far) to 60 % of it, so that no edge shows.
        let knee = room * (1.0 - 0.4 * m);
        if far <= knee || room - knee <= 1e-5 {
            return c;
        }
        let wanted = knee + (room - knee) * (1.0 - (-(far - knee) / (room - knee)).exp());
        // Drawn toward the page's colour until its luminance is the one
        // wanted (luminance grows with the mix: a few halvings find it).
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..8 {
            let mid = (lo + hi) / 2.0;
            let l = self.luminance(lerp3(self.page, c, mid));
            let gone = if self.dark { l - self.page_l } else { self.page_l - l };
            if gone > wanted { hi = mid } else { lo = mid }
        }
        lerp3(self.page, c, lo)
    }
}

// --------------------------------------------------------------- smooth --

/// A point of a style's plane: across (in heights of the area) and along
/// the direction it rises toward (0 at the foot, 1 at the far side).
fn axes(direction: Direction, u: f32, v: f32, aspect: f32) -> (f32, f32) {
    match direction {
        Direction::Up => (u, 1.0 - v),
        Direction::Down => (u, v),
        Direction::Right => (v, u / aspect),
        Direction::Left => (v, 1.0 - u / aspect),
    }
}

/// How the waves are drawn: how broad their swells are, how soft their
/// seams, how tall their swells, and the side they rise from.
#[derive(Clone, Copy)]
struct Shaping {
    scale: f32,
    melt: f32,
    swell: f32,
    direction: Direction,
}

impl Shaping {
    /// For `pal`: calmer on the light page, the seams softer and the swells
    /// lower.
    fn new(pal: &Palette, scale: f32, melt: f32, direction: Direction) -> Self {
        if pal.dark { Self { scale, melt, swell: 1.0, direction } } else { Self { scale, melt: melt.max(0.75), swell: 0.7, direction } }
    }
}

/// Where the waves' five seams stand at `a` across (in heights of the
/// area), `t` seconds in, from the foot: each swelling (by `swell`) and
/// rolling.
fn waves_seams(scale: f32, swell: f32, a: f32, t: f32) -> [f32; 5] {
    use std::f32::consts::TAU;
    const BASES: [f32; 5] = [0.10, 0.21, 0.32, 0.44, 0.58];
    let scale = scale.clamp(0.5, 2.0);
    let freq = 0.5 / scale;
    let amp = 0.09 * scale.sqrt() * swell;
    std::array::from_fn(|k| {
        let kf = k as f32;
        let wave = 0.65 * (TAU * (freq * a + 0.13 * kf) + 0.12 * t).sin() + 0.35 * (TAU * (2.3 * freq * a - 0.29 * kf) - 0.08 * t + kf).sin();
        BASES[k] + amp * wave * (1.0 + 0.25 * kf)
    })
}

/// The waves' colour `s` along their direction (0 at the foot) where their
/// seams stand at `seams`: the ground colour at the foot, above each seam
/// the next colour of `ramp`, softened by `melt` into each other.
fn waves_colour(ramp: &[[f32; 3]; 6], seams: &[f32; 5], melt: f32, s: f32) -> [f32; 3] {
    let melt = 0.012 + 0.11 * melt.clamp(0.0, 1.0);
    let mut c = ramp[5];
    for (k, seam) in seams.iter().enumerate() {
        c = lerp3(c, ramp[4 - k], smooth(seam - melt, seam + melt, s));
    }
    c
}

/// The waves' colour at (`u`, `v`) (in heights of the area, from the top
/// left), `t` seconds in.
fn waves_at(ramp: &[[f32; 3]; 6], shaping: &Shaping, (u, v): (f32, f32), aspect: f32, t: f32) -> [f32; 3] {
    let (a, s) = axes(shaping.direction, u, v, aspect);
    waves_colour(ramp, &waves_seams(shaping.scale, shaping.swell, a, t), shaping.melt, s)
}

/// Columns across the prism.
const COLUMNS: usize = 44;

/// How tall the prism's column `i` stands (0–1 of the area along its
/// direction), `t` seconds in.
fn prism_height(p: &Prism, i: usize, t: f32) -> f32 {
    use std::f32::consts::TAU;
    let a = (i as f32 + 0.5) / COLUMNS as f32;
    let from_middle = (a - 0.5).abs() * 2.0;
    let swell = match p.motion {
        Motion::Expand => (TAU * (0.07 * t - from_middle)).sin(),
        Motion::Gather => (TAU * (0.07 * t + from_middle)).sin(),
        Motion::Breathe => (TAU * 0.06 * t).sin(),
    };
    0.66 * p.shape.profile(a) * (0.88 + 0.12 * swell)
}

/// A quad of `mesh`, its corners `p` and colours `c` (top left, top right,
/// bottom right, bottom left).
fn quad(mesh: &mut egui::Mesh, p: [Pos2; 4], c: [Color32; 4]) {
    let base = mesh.vertices.len() as u32;
    for (p, c) in p.into_iter().zip(c) {
        mesh.vertices.push(egui::epaint::Vertex { pos: p, uv: egui::epaint::WHITE_UV, color: c });
    }
    mesh.indices.extend_from_slice(&[base, base + 1, base + 2, base, base + 2, base + 3]);
}

/// The background in a smooth style (waves, pixels, prism) over `rect`,
/// `t` seconds in, held by `guard` where the words are: meshes, to add to a
/// painter. `None` for the ridges, drawn by [`frame`].
pub fn meshes(background: &Background, pal: &Palette, page: Color32, guard: &Guard, rect: Rect, t: f32) -> Option<Vec<egui::Mesh>> {
    let page_f = rgb_f(page);
    let (w, h) = (rect.width().max(1.0), rect.height().max(1.0));
    let aspect = w / h;
    let point = |u: f32, v: f32| pos2(rect.left() + u * h, rect.top() + v * h);
    let mut mesh = egui::Mesh::default();
    match background.style {
        Style::Ridges => return None,
        Style::Waves => {
            let b = &background.waves;
            let ramp = waves_ramp(b.palette, pal, page_f);
            let shaping = Shaping::new(pal, b.scale, b.melt, b.direction);
            let t = t * b.speed.clamp(0.0, 2.0);
            // A grid fine enough for the sharpest seams; the card blends
            // between its corners.
            let ny = (64.0 + 96.0 * (1.0 - shaping.melt.clamp(0.0, 1.0))) as usize;
            let nx = ((ny as f32 * aspect).ceil() as usize).max(2);
            // The seams depend on one axis only: worked out once along it.
            let vertical = matches!(b.direction, Direction::Up | Direction::Down);
            let seams: Vec<[f32; 5]> = if vertical {
                (0..=nx).map(|i| waves_seams(shaping.scale, shaping.swell, i as f32 / nx as f32 * aspect, t)).collect()
            } else {
                (0..=ny).map(|j| waves_seams(shaping.scale, shaping.swell, j as f32 / ny as f32, t)).collect()
            };
            let colours: Vec<Color32> = (0..=ny)
                .flat_map(|j| (0..=nx).map(move |i| (i, j)))
                .map(|(i, j)| {
                    let (u, v) = (i as f32 / nx as f32 * aspect, j as f32 / ny as f32);
                    let (_, along) = axes(b.direction, u, v, aspect);
                    let c = waves_colour(&ramp, &seams[if vertical { i } else { j }], shaping.melt, along);
                    to_color(guard.hold(c, point(u, v)))
                })
                .collect();
            for j in 0..=ny {
                for i in 0..=nx {
                    mesh.vertices.push(egui::epaint::Vertex { pos: point(i as f32 / nx as f32 * aspect, j as f32 / ny as f32), uv: egui::epaint::WHITE_UV, color: colours[j * (nx + 1) + i] });
                }
            }
            let at = |i: usize, j: usize| (j * (nx + 1) + i) as u32;
            for j in 0..ny {
                for i in 0..nx {
                    mesh.indices.extend_from_slice(&[at(i, j), at(i + 1, j), at(i + 1, j + 1), at(i, j), at(i + 1, j + 1), at(i, j + 1)]);
                }
            }
        }
        Style::Pixel => {
            let b = &background.pixel;
            let ramp = waves_ramp(b.palette, pal, page_f);
            let pixel_shaping = Shaping::new(pal, 1.0, 0.6, Direction::Up);
            let t = t * b.speed.clamp(0.0, 2.0);
            let across = b.across.clamp(12.0, 96.0).round();
            let side = w / across;
            let rows = (h / side).ceil() as usize;
            for j in 0..rows {
                // From the bottom up, as the bands rise.
                let y1 = rect.bottom() - j as f32 * side;
                let y0 = y1 - side;
                for i in 0..across as usize {
                    let x0 = rect.left() + i as f32 * side;
                    let centre = pos2(x0 + side / 2.0, (y0 + y1) / 2.0);
                    let (u, v) = ((centre.x - rect.left()) / h, (centre.y - rect.top()) / h);
                    let c = to_color(guard.hold(waves_at(&ramp, &pixel_shaping, (u, v), aspect, t), centre));
                    quad(&mut mesh, [pos2(x0, y0), pos2(x0 + side, y0), pos2(x0 + side, y1), pos2(x0, y1)], [c; 4]);
                }
            }
        }
        Style::Prism => {
            let b = &background.prism;
            let ramp = prism_ramp(b.palette, pal, page_f);
            let t = t * b.speed.clamp(0.0, 2.0);
            // Where a point of the plane (across, along) is on the screen.
            let to_screen = |a: f32, s: f32| match b.direction {
                Direction::Up => pos2(rect.left() + a * w, rect.bottom() - s * h),
                Direction::Down => pos2(rect.left() + a * w, rect.top() + s * h),
                Direction::Right => pos2(rect.left() + s * w, rect.top() + a * h),
                Direction::Left => pos2(rect.right() - s * w, rect.top() + a * h),
            };
            let colour = |c: [f32; 3], a: f32, s: f32| to_color(guard.hold(c, to_screen(a, s)));
            // Along a column: its foot, the stops of its gradient, its top,
            // then the ground beyond it, in steps the hold can follow.
            for i in 0..COLUMNS {
                let (a0, a1) = (i as f32 / COLUMNS as f32, (i + 1) as f32 / COLUMNS as f32);
                let top = prism_height(b, i, t);
                // Facets: each column a little lighter or darker.
                let facet = 1.0 + 0.06 * (i as f32 * 1.7).sin();
                let shade = |c: [f32; 3]| c.map(|v| (v * facet).min(1.0));
                let stops = [(0.0, shade(ramp[4])), (0.3, shade(ramp[3])), (0.62, shade(ramp[2])), (1.0, shade(ramp[1]))];
                let mut along: Vec<(f32, [f32; 3])> = stops.iter().map(|&(k, c)| (k * top, c)).collect();
                // Its top edge, crisp: the ground starts there.
                let ground = ramp[0];
                let steps = 8;
                let ground_from = along.len();
                for k in 0..=steps {
                    along.push((top + (1.0 - top) * k as f32 / steps as f32, ground));
                }
                for k in 0..along.len() - 1 {
                    if k + 1 == ground_from {
                        continue;
                    }
                    let ((s0, c0), (s1, c1)) = (along[k], along[k + 1]);
                    // Several rows in a long stop, so that the hold follows.
                    let rows = (((s1 - s0) * 12.0).ceil() as usize).max(1);
                    for r in 0..rows {
                        let (f0, f1) = (r as f32 / rows as f32, (r + 1) as f32 / rows as f32);
                        let (sa, sb) = (s0 + (s1 - s0) * f0, s0 + (s1 - s0) * f1);
                        let (ca, cb) = (lerp3(c0, c1, f0), lerp3(c0, c1, f1));
                        quad(&mut mesh, [to_screen(a0, sa), to_screen(a1, sa), to_screen(a1, sb), to_screen(a0, sb)], [colour(ca, a0, sa), colour(ca, a1, sa), colour(cb, a1, sb), colour(cb, a0, sb)]);
                    }
                }
            }
        }
    }
    // A mesh holds at most 65 536 corners: split it.
    let mut out = Vec::new();
    if mesh.vertices.len() <= u16::MAX as usize {
        out.push(mesh);
    } else {
        out.extend(mesh.split_to_u16().into_iter().map(|m| egui::Mesh { indices: m.indices.into_iter().map(u32::from).collect(), vertices: m.vertices, texture_id: m.texture_id }));
    }
    Some(out)
}

/// The colour a smooth style shows at `p` in `rect`, `t` seconds in, held
/// by `guard`: what its mesh's corners are given (the pixels' at their
/// middle), for its preview and the tests of its legibility.
pub fn colour_at(background: &Background, pal: &Palette, page: Color32, guard: &Guard, rect: Rect, p: Pos2, t: f32) -> [f32; 3] {
    let page_f = rgb_f(page);
    let (w, h) = (rect.width(), rect.height());
    let aspect = w / h;
    let (u, v) = ((p.x - rect.left()) / h, (p.y - rect.top()) / h);
    let c = match background.style {
        Style::Waves => {
            let b = &background.waves;
            waves_at(&waves_ramp(b.palette, pal, page_f), &Shaping::new(pal, b.scale, b.melt, b.direction), (u, v), aspect, t * b.speed)
        }
        Style::Pixel => {
            let b = &background.pixel;
            // The middle of the pixel `p` is in, from the bottom left.
            let side = w / b.across.clamp(12.0, 96.0).round();
            let i = ((p.x - rect.left()) / side).floor();
            let j = ((rect.bottom() - p.y) / side).floor();
            let centre = pos2(rect.left() + (i + 0.5) * side, rect.bottom() - (j + 0.5) * side);
            let (u, v) = ((centre.x - rect.left()) / h, (centre.y - rect.top()) / h);
            let c = waves_at(&waves_ramp(b.palette, pal, page_f), &Shaping::new(pal, 1.0, 0.6, Direction::Up), (u, v), aspect, t * b.speed);
            return guard.hold(c, centre);
        }
        Style::Prism => {
            let b = &background.prism;
            let ramp = prism_ramp(b.palette, pal, page_f);
            let (a, s) = match b.direction {
                Direction::Up => ((p.x - rect.left()) / w, (rect.bottom() - p.y) / h),
                Direction::Down => ((p.x - rect.left()) / w, (p.y - rect.top()) / h),
                Direction::Right => ((p.y - rect.top()) / h, (p.x - rect.left()) / w),
                Direction::Left => ((p.y - rect.top()) / h, (rect.right() - p.x) / w),
            };
            let i = ((a * COLUMNS as f32) as usize).min(COLUMNS - 1);
            let top = prism_height(b, i, t * b.speed);
            if s >= top {
                ramp[0]
            } else {
                let k = s / top;
                let facet = 1.0 + 0.06 * (i as f32 * 1.7).sin();
                let c = if k < 0.3 { lerp3(ramp[4], ramp[3], k / 0.3) } else if k < 0.62 { lerp3(ramp[3], ramp[2], (k - 0.3) / 0.32) } else { lerp3(ramp[2], ramp[1], (k - 0.62) / 0.38) };
                c.map(|v| (v * facet).min(1.0))
            }
        }
        Style::Ridges => page_f,
    };
    guard.hold(c, p)
}

/// A picture of a smooth style, `cols` × `rows` samples over `rect` (for a
/// preview, blended by the card).
fn smooth_image(background: &Background, pal: &Palette, page: Color32, rect: Rect, cols: usize, rows: usize, t: f32) -> egui::ColorImage {
    let guard = Guard::new(pal, page, page, None, rect);
    let (cw, ch) = (rect.width() / cols as f32, rect.height() / rows as f32);
    let pixels = (0..rows)
        .flat_map(|y| (0..cols).map(move |x| (x, y)))
        .map(|(x, y)| to_color(colour_at(background, pal, page, &guard, rect, pos2(rect.left() + (x as f32 + 0.5) * cw, rect.top() + (y as f32 + 0.5) * ch), t)))
        .collect();
    egui::ColorImage::new([cols, rows], pixels)
}

// --------------------------------------------------------------- ridges --

/// How much of the art shows through the halo the home screen lays around
/// letters, at most: its rings of the page's colour cover at least half of
/// what is next to a letter, more where they overlap; taken as 35 %.
pub const HALO: f32 = 0.35;

/// The luminance the ridges may have under muted text (`muted`, its
/// luminance) on a page whose grey is `page` (0–1): at most in the dark
/// theme, at least in the light one, for the text to keep 3:1 on them seen
/// through its halo — 3.35 rather than 3, as dots dithered between levels
/// average a little further from the page than the colour they stand for.
fn art_limit(muted: f32, page: f32, dark: bool) -> f32 {
    let to_srgb = |l: f32| if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 };
    let to_light = |v: f32| if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) };
    let seen = if dark { (muted + 0.05) / 3.35 - 0.05 } else { 3.35 * (muted + 0.05) - 0.05 };
    // What is seen is the art blended with the page, in gamma space as the
    // interface is drawn: the art itself may go this far.
    let art = ((to_srgb(seen.clamp(0.0, 1.0)) - (1.0 - HALO) * page) / HALO).clamp(0.0, 1.0);
    to_light(art)
}

/// Levels a channel of the ridges is dithered to, on the square root of
/// its distance from the page's colour: finer near it.
const LEVELS: usize = 7;
/// Steps of a channel in the tables that turn it to a level or a light.
const STEPS: usize = 1024;
/// Steps of a light in the table that turns it back to a channel.
const LIGHTS: usize = 4096;

/// The ordered dither of the ridges.
const BAYER: [[u8; 8]; 8] = [
    [0, 32, 8, 40, 2, 34, 10, 42],
    [48, 16, 56, 24, 50, 18, 58, 26],
    [12, 44, 4, 36, 14, 46, 6, 38],
    [60, 28, 52, 20, 62, 30, 54, 22],
    [3, 35, 11, 43, 1, 33, 9, 41],
    [51, 19, 59, 27, 49, 17, 57, 25],
    [15, 47, 7, 39, 13, 45, 5, 37],
    [63, 31, 55, 23, 61, 29, 53, 21],
];

/// How the ridges are coloured for a theme, with the tables their dither
/// and their legibility work from.
pub struct Look {
    dark: bool,
    high: bool,
    bg: [f32; 3],
    bg_px: Color32,
    /// Their glow and crests, and the glow's hottest colour.
    accent: [f32; 3],
    hot: [f32; 3],
    /// What the land goes toward from the page's colour (white in the dark
    /// theme, black in the light one), how much of it far off and near, and
    /// a crest's highlight.
    ink: [f32; 3],
    ink_far: f32,
    ink_near: f32,
    crest: f32,
    /// The luminance a cell may have under the words, at most (dark theme)
    /// or at least (light), for muted text to keep 3:1 through its halo.
    limit: f32,
    // The tables: a channel (in 1024ths) to its place among the levels, a
    // level to its byte; a channel to its light and back.
    place: [[f32; STEPS + 1]; 3],
    level: [[u8; LEVELS + 1]; 3],
    to_light: [f32; STEPS + 1],
    from_light: Vec<f32>,
}

impl Look {
    /// For the palette `pal` (its theme and accent), the page's colour `page`
    /// and the ink of muted words over the background, `muted`.
    pub fn new(pal: &Palette, page: Color32, muted: Color32) -> Self {
        let dark = pal.dark;
        let bg = rgb_f(page);
        let accent = rgb_f(pal.accent);
        let (ink, ink_far, ink_near, crest, hot) = if dark { ([1.0; 3], 0.17, 0.04, 0.8, [1.0, 0.95, 0.86]) } else { ([0.0; 3], 0.05, 0.15, 0.68, accent.map(|c| c * 0.8)) };
        let limit = art_limit(luminance(rgb_f(muted)), bg[1], dark);
        let place = std::array::from_fn(|i| {
            let b = bg[i];
            std::array::from_fn(|j| {
                let ch = j as f32 / STEPS as f32;
                let ink = if dark { (ch - b) / (1.0 - b) } else { (b - ch) / b };
                ink.clamp(0.0, 1.0).sqrt() * LEVELS as f32
            })
        });
        let level = std::array::from_fn(|i| {
            let b = bg[i];
            std::array::from_fn(|l| {
                let l = l as f32 / LEVELS as f32;
                ((if dark { b + l * l * (1.0 - b) } else { b - l * l * b }) * 255.0).round() as u8
            })
        });
        let to_light = std::array::from_fn(|j| {
            let v = j as f32 / STEPS as f32;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        });
        let from_light = (0..=LIGHTS)
            .map(|j| {
                let l = j as f32 / LIGHTS as f32;
                if l <= 0.003_130_8 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }
            })
            .collect();
        Self { dark, high: pal.high, bg, bg_px: page, accent, hot, ink, ink_far, ink_near, crest, limit, place, level, to_light, from_light }
    }

    #[inline]
    fn light(&self, c: [f32; 3]) -> [f32; 3] {
        let at = |v: f32| self.to_light[(v.clamp(0.0, 1.0) * STEPS as f32) as usize];
        [at(c[0]), at(c[1]), at(c[2])]
    }

    #[inline]
    fn quantize(&self, c: [f32; 3], th: f32) -> Color32 {
        let channel = |i: usize| {
            let at = self.place[i][(c[i].clamp(0.0, 1.0) * STEPS as f32) as usize];
            self.level[i][((at + th).max(0.0) as usize).min(LEVELS)]
        };
        Color32::from_rgb(channel(0), channel(1), channel(2))
    }
}

/// The last pass over every cell of the ridges: its light held down under
/// the words, then the dither.
struct Finish<'a> {
    look: &'a Look,
    mask_x: Vec<f32>,
    mask_y: Vec<f32>,
    /// The page's luminance and its light, channel by channel.
    lb: f32,
    bg_light: [f32; 3],
}

impl<'a> Finish<'a> {
    /// For a frame of `cols` × `rows` cells, the words in `calm` (cells).
    fn new(look: &'a Look, cols: usize, rows: usize, calm: Option<[i32; 4]>) -> Self {
        let (w, h) = (cols.max(1) as f32, rows.max(1) as f32);
        let (mask_x, mask_y) = match calm {
            Some([x0, y0, x1, y1]) => {
                // The whole column, fading out around it.
                let inside = |p: f32, lo: f32, hi: f32, soft: f32| {
                    let out = ((lo - p).max(p - hi).max(0.0) / soft).min(1.0);
                    1.0 - out * out * (3.0 - 2.0 * out)
                };
                ((0..cols).map(|x| inside(x as f32, x0 as f32, x1 as f32, 0.1 * w)).collect(), (0..rows).map(|y| inside(y as f32, y0 as f32, y1 as f32, 0.08 * h)).collect())
            }
            None => (vec![0.0; cols], vec![0.0; rows]),
        };
        let bg_light = look.light(look.bg);
        let lb = 0.2126 * bg_light[0] + 0.7152 * bg_light[1] + 0.0722 * bg_light[2];
        Self { look, mask_x, mask_y, lb, bg_light }
    }

    /// Cell (`x`, `y`) of colour `c`, dithered.
    #[inline]
    fn pixel(&self, c: [f32; 3], x: usize, y: usize) -> Color32 {
        let look = self.look;
        let th = (BAYER[y % 8][x % 8] as f32 + 0.5) / 64.0;
        let m = self.mask_x[x] * self.mask_y[y];
        if m <= 0.0 {
            return look.quantize(c, th);
        }
        if look.high {
            return look.quantize(lerp3(c, look.bg, m), th);
        }
        // Under the words: the light pressed under (dark theme) or the
        // shade lifted over (light) a luminance, softly — no edge where it
        // starts —, the colour drawn toward the page's in light until it is.
        let allowed = if look.dark { 1.0 + (look.limit - 1.0) * m } else { look.limit * m };
        let cl = look.light(c);
        let lc = 0.2126 * cl[0] + 0.7152 * cl[1] + 0.0722 * cl[2];
        let (room, far) = if look.dark { (allowed - self.lb, lc - self.lb) } else { (self.lb - allowed, self.lb - lc) };
        let knee = room * (1.0 - 0.4 * m);
        let c = if room <= 0.0 {
            look.bg
        } else if far > knee && room - knee > 1e-5 {
            let wanted = knee + (room - knee) * (1.0 - (-(far - knee) / (room - knee)).exp());
            let f = wanted / far;
            [0, 1, 2].map(|i| look.from_light[((self.bg_light[i] + (cl[i] - self.bg_light[i]) * f).clamp(0.0, 1.0) * LIGHTS as f32) as usize])
        } else {
            c
        };
        look.quantize(c, th)
    }
}

fn hash(i: i32, j: i32, seed: u32) -> f32 {
    let mut h = (i as u32).wrapping_mul(0x9e37_79b1) ^ (j as u32).wrapping_mul(0x85eb_ca6b) ^ seed.wrapping_mul(0xc2b2_ae35);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    (h & 0xffff) as f32 / 32767.5 - 1.0
}

/// A smooth noise along a line, from -1 to 1: values at whole numbers,
/// eased between.
fn noise_1d(x: f32, seed: u32) -> f32 {
    let i = x.floor();
    let f = x - i;
    let f = f * f * (3.0 - 2.0 * f);
    hash(i as i32, 0, seed) * (1.0 - f) + hash(i as i32 + 1, 0, seed) * f
}

/// One frame of the ridges, `cols` × `rows` cells, `t` seconds in, their
/// light held down around `calm` (cells, where the words are; none for a
/// preview).
pub fn frame(look: &Look, cols: usize, rows: usize, calm: Option<[i32; 4]>, t: f64) -> egui::ColorImage {
    let (cols, rows) = (cols.max(1), rows.max(1));
    // Kept small: f32 would lose the seconds of a long session.
    let tf = (t % 7200.0) as f32;
    let finish = Finish::new(look, cols, rows, calm);
    ridges(look, &finish, cols, rows, tf)
}

/// The ridges: six ranges of mountains seen from afar, one in front of the
/// other, under a glow of the accent low on the right, as of a sun gone
/// down. Each range is a band: a jagged crest — white-hot near the glow,
/// grey with sparks of the accent away from it — over land that darkens
/// down from it, the far ranges paler in the haze, warmed where the glow
/// reaches. The sky is the page's colour, but for the glow. The ranges
/// drift, the nearer faster, and breathe; the glow pulses. The crests are
/// worked out once a column, so that a cell costs a few comparisons.
fn ridges(look: &Look, finish: &Finish, cols: usize, rows: usize, t: f32) -> egui::ColorImage {
    use std::f32::consts::TAU;
    /// The ranges, far to near: where each stands (in heights of the area,
    /// from the top) on the left, how much higher it stands on the right,
    /// toward the glow; how far it rises and falls, how fine it is, how
    /// fast it drifts (heights a second: the nearer, the faster).
    const RIDGES: [(f32, f32, f32, f32, f32); 6] = [
        (0.560, 0.000, 0.020, 2.4, 0.0020),
        (0.630, 0.030, 0.045, 1.5, 0.0030),
        (0.715, 0.140, 0.070, 1.1, 0.0042),
        (0.770, 0.080, 0.075, 1.0, 0.0058),
        (0.840, 0.030, 0.070, 0.9, 0.0078),
        (0.915, 0.010, 0.060, 0.8, 0.0100),
    ];
    let (w, h) = (cols as f32, rows as f32);
    let n = RIDGES.len();
    let crests: Vec<Vec<f32>> = RIDGES
        .iter()
        .enumerate()
        .map(|(k, &(base, lift, rise, fine, drift))| {
            let seed = 0x6d61_7373 + k as u32 * 977;
            let breathe = 1.0 + 0.1 * (TAU * t / 41.0 + k as f32 * 1.3).sin();
            (0..cols)
                .map(|x| {
                    let across = x as f32 / w;
                    let u = (x as f32 + 0.5) / h * fine + drift * t * fine;
                    // Octaves: broad swells, then ever finer jags.
                    let mut s = 0.0;
                    let (mut f, mut a) = (1.0, 1.0);
                    for o in 0..7 {
                        s += a * noise_1d(u * f, seed + o);
                        f *= 2.13;
                        a *= 0.58;
                    }
                    // Higher toward the glow, as a range catching the light.
                    let toward = across * across * (3.0 - 2.0 * across);
                    (base - lift * toward - rise * breathe * s) * h
                })
                .collect()
        })
        .collect();
    // The glow: low on the right, wide, its core on the farthest range.
    let pulse = 1.0 + 0.08 * (TAU * t / 13.0).sin();
    let (gx, gy) = (0.9 * w, 0.548 * h);
    // A bright core hugging the horizon, in a wide halo that rises.
    let core_x: Vec<f32> = (0..cols).map(|x| (-((x as f32 - gx) / (0.14 * w)).powi(2)).exp()).collect();
    let halo_x: Vec<f32> = (0..cols).map(|x| (-((x as f32 - gx) / (0.42 * w)).powi(2)).exp()).collect();
    let core_y = |y: f32| (-((y - gy) / if y < gy { 0.09 * h } else { 0.03 * h }).powi(2)).exp() * pulse;
    let halo_y = |y: f32| 0.6 * (-((y - gy) / if y < gy { 0.22 * h } else { 0.05 * h }).powi(2)).exp() * pulse;
    let glow_at = |x: usize, y: f32| 1.25 * core_x[x] * core_y(y) + halo_x[x] * halo_y(y);
    let rows_core: Vec<f32> = (0..rows).map(|y| core_y(y as f32 + 0.5)).collect();
    let rows_halo: Vec<f32> = (0..rows).map(|y| halo_y(y as f32 + 0.5)).collect();
    let bg = look.bg;
    let a = look.accent;
    // The glow's colour by its strength: the page's colour, a deep accent,
    // the accent, its hot core.
    let glow_colour = |i: f32| -> [f32; 3] {
        let i = i.min(1.4);
        // Dark theme: a deep accent, the accent, then near white; light
        // theme: a pale accent, the accent, then a darker one.
        let deep = if look.dark { a.map(|c| c * 0.55) } else { lerp3(bg, a, 0.5) };
        if i < 0.5 {
            let t = i / 0.5;
            lerp3(bg, deep, t * t.sqrt())
        } else if i < 0.95 {
            lerp3(deep, a, (i - 0.5) / 0.45)
        } else {
            lerp3(a, lerp3(a, look.hot, if look.dark { 0.25 } else { 0.7 }), ((i - 1.0) / 0.4).clamp(0.0, 1.0))
        }
    };
    // Worked out once a range, and once a range and column.
    let depth: Vec<f32> = (0..n).map(|k| k as f32 / (n - 1) as f32).collect();
    let base: Vec<f32> = depth.iter().map(|d| look.ink_far + (look.ink_near - look.ink_far) * d.powf(0.3)).collect();
    let grey = |ink: f32| lerp3(bg, look.ink, ink);
    // The accent the land warms to: deep in the dark theme, light in the
    // light one.
    let warm = if look.dark { a.map(|v| v * 0.55) } else { lerp3(bg, a, 0.75) };
    let (band_fall, warm_fall, slope_fall) = (1.0 / (0.012 * h), 1.0 / (0.03 * h), 1.0 / (0.07 * h));
    // The glow where a range's crest stands (it thickens and whitens the
    // crest), and with it the light it spreads over the ranges on its side,
    // less on the near (it warms them).
    let direct: Vec<Vec<f32>> = (0..n).map(|k| (0..cols).map(|x| glow_at(x, crests[k][x]).min(1.0)).collect()).collect();
    let ambient: Vec<f32> = (0..cols).map(|x| (-((x as f32 - gx) / (0.6 * w)).powi(2)).exp()).collect();
    let lit: Vec<Vec<f32>> = (0..n).map(|k| (0..cols).map(|x| (direct[k][x] + 0.6 * ambient[x] * (1.0 - 0.35 * depth[k])).min(1.0)).collect()).collect();
    let mut pixels = Vec::with_capacity(cols * rows);
    // Cells over a crest its bloom reaches.
    const RIM: f32 = 3.0;
    // Above every crest, and where no glow reaches, a row is the page.
    let highest = crests.iter().flat_map(|row| row.iter()).fold(f32::MAX, |a, &b| a.min(b));
    for y in 0..rows {
        let v = y as f32 + 0.5;
        if v < highest - RIM && (look.high || 1.25 * rows_core[y] + rows_halo[y] < 0.004) {
            pixels.extend(std::iter::repeat_n(look.bg_px, cols));
            continue;
        }
        for x in 0..cols {
            let mut c = None;
            // The nearest crest just under this cell, of a range in front:
            // its glow blooms a little above it.
            let (mut rim_d, mut rim_k) = (f32::MAX, 0);
            for k in (0..n).rev() {
                let d = v - crests[k][x];
                if d < 0.0 {
                    if -d < rim_d {
                        (rim_d, rim_k) = (-d, k);
                    }
                    continue;
                }
                if look.high {
                    c = (d < 1.0).then_some(a);
                    break;
                }
                let (depth, lit, direct) = (depth[k], lit[k][x], direct[k][x]);
                // Sparks: whether this cell of a crest is the accent's or
                // the grey's, more of the accent toward the glow.
                let spark = || (BAYER[(x * 3 + k) % 8][(x + y * 5) % 8] as f32 + 0.5) / 64.0 < 0.35 + 0.65 * (1.5 * lit).min(1.0);
                if d < 1.0 + 0.8 * lit {
                    // The crest's top: a highlight, pale with sparks of the
                    // accent away from the glow, white-hot toward it.
                    let hi = grey(look.crest * (1.0 - 0.45 * depth));
                    let hot = lerp3(a, look.hot, ((1.4 * direct - 0.3) * (1.0 - 0.5 * depth)).clamp(0.0, 1.0));
                    c = Some(lerp3(hi, hot, if spark() { 0.5 + 0.5 * lit } else { lit }));
                } else if d < 2.0 + 0.8 * lit + 1.5 * direct {
                    // Under it, the accent: bright toward the glow, sparks
                    // on grey away from it.
                    let accent = a.map(|v| v * (0.55 + 0.45 * lit));
                    c = Some(if spark() || lit > 0.5 { lerp3(grey(0.4), accent, 0.6 + 0.4 * lit) } else { grey(look.crest * 0.5 * (1.0 - 0.4 * depth)) });
                } else {
                    // The land: dense just under the crest, fading to a
                    // faint texture; paler far off; warmed by the glow on
                    // its side (falloffs as ratios, cheaper than
                    // exponentials).
                    let below = d - 2.0;
                    let e = 1.0 / (1.0 + below * band_fall);
                    // Darker down the slope, so that each range stands out
                    // from the one behind it.
                    let slope = 1.0 / (1.0 + below * slope_fall);
                    let ground = grey(base[k] * (0.45 + 0.55 * slope) + 0.16 * e * e * (1.0 - 0.4 * depth));
                    let e = 1.0 / (1.0 + below * warm_fall);
                    let warmth = (1.3 * lit * e * e).min(1.0);
                    c = Some(lerp3(ground, warm, 0.85 * warmth));
                }
                break;
            }
            let c = match c {
                Some(c) => c,
                None if look.high => {
                    pixels.push(look.bg_px);
                    continue;
                }
                None => {
                    let glow = 1.25 * core_x[x] * rows_core[y] + halo_x[x] * rows_halo[y];
                    if glow < 0.004 && rim_d >= RIM {
                        pixels.push(look.bg_px);
                        continue;
                    }
                    glow_colour(glow)
                }
            };
            // The bloom over a crest lit by the glow.
            let c = if rim_d < RIM && !look.high {
                let f = 1.0 - rim_d / RIM;
                lerp3(c, a, (0.6 * lit[rim_k][x] * f * f).min(1.0))
            } else {
                c
            };
            pixels.push(finish.pixel(c, x, y));
        }
    }
    egui::ColorImage::new([cols, rows], pixels)
}

// ------------------------------------------------------------- previews --

/// The background's picture in the settings: one texture, drawn again only
/// when what it shows changes.
#[derive(Default)]
pub struct Preview {
    shown: Option<(PreviewKey, egui::TextureHandle)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
struct PreviewKey {
    cols: usize,
    rows: usize,
    dark: bool,
    high: bool,
    accent: Color32,
    background: Background,
    frame: i64,
}

impl Preview {
    /// The texture of `background` for a picture of `size` cells, at frame
    /// `frame` (of [`FPS`]), on the theme of `pal`, and the part of it to
    /// show: the lower part of a home screen of 16 by 10 as wide as the
    /// picture, where its colours are. The waves are blended by the card,
    /// the other styles drawn in sharp cells.
    pub fn texture(&mut self, ctx: &egui::Context, pal: &Palette, background: &Background, size: [usize; 2], frame: i64) -> (egui::TextureId, Rect) {
        let (page, muted) = (crate::home::backdrop_fill(pal), crate::home::art_weak(pal));
        let [cols, shown] = size;
        let rows = (cols * 10 / 16).max(shown).max(1);
        let uv = Rect::from_min_max(pos2(0.0, 1.0 - shown as f32 / rows as f32), pos2(1.0, 1.0));
        let key = PreviewKey { cols, rows, dark: pal.dark, high: pal.high, accent: pal.accent, background: *background, frame };
        if let Some((k, texture)) = &self.shown
            && *k == key
        {
            return (texture.id(), uv);
        }
        let t = frame as f64 / FPS;
        let (image, options) = if background.style == Style::Ridges {
            (self::frame(&Look::new(pal, page, muted), cols, rows, None, t), egui::TextureOptions::NEAREST)
        } else {
            let rect = Rect::from_min_size(pos2(0.0, 0.0), vec2(cols as f32 * 4.0, rows as f32 * 4.0));
            let options = if background.style == Style::Waves { egui::TextureOptions::LINEAR } else { egui::TextureOptions::NEAREST };
            (smooth_image(background, pal, page, rect, cols, rows, t as f32), options)
        };
        match &mut self.shown {
            Some((k, texture)) => {
                texture.set(image, options);
                *k = key;
                (texture.id(), uv)
            }
            None => {
                let texture = ctx.load_texture("background preview", image, options);
                let id = texture.id();
                self.shown = Some((key, texture));
                (id, uv)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    fn palette(dark: bool, high: bool, accent: Color32) -> Palette {
        let base = Palette::themed(dark, high);
        let accent = if high { theme::legible(accent, base.panel, 4.5) } else { accent };
        Palette { accent, ..base }
    }

    fn page(dark: bool) -> Color32 {
        if dark { Color32::from_rgb(8, 8, 9) } else { Color32::from_rgb(248, 248, 249) }
    }

    fn muted(pal: &Palette) -> Color32 {
        if pal.high { pal.weak } else { theme::mix(pal.weak, pal.text, 0.4) }
    }

    /// Every smooth style, palette, direction, shape and motion leaves the
    /// words of every theme legible straight on it, without their halo:
    /// text past 4.5 to 1 and muted text past 3 to 1 (7 and 4.5 in high
    /// contrast) wherever the words are, at any time — and with every
    /// accent for the theme's palette. And each moves.
    #[test]
    fn every_smooth_background_stays_legible_under_the_words() {
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1333.0, 1000.0));
        let column = Rect::from_min_max(pos2(276.0, 120.0), pos2(1056.0, 780.0));
        let accents: Vec<Color32> = theme::ACCENTS.iter().map(|(_, c)| theme::rgb(*c)).collect();
        let mut cases: Vec<Background> = Vec::new();
        for palette in WavesPalette::ALL {
            for direction in Direction::ALL {
                cases.push(Background { style: Style::Waves, waves: Waves { palette, direction, ..Waves::default() }, ..Background::default() });
            }
            for melt in [0.0, 1.0] {
                cases.push(Background { style: Style::Waves, waves: Waves { palette, melt, scale: 2.0, ..Waves::default() }, ..Background::default() });
            }
            cases.push(Background { style: Style::Pixel, pixel: Pixels { palette, ..Pixels::default() }, ..Background::default() });
        }
        for palette in PrismPalette::ALL {
            for shape in Shape::ALL {
                for direction in Direction::ALL {
                    cases.push(Background { style: Style::Prism, prism: Prism { palette, shape, direction, ..Prism::default() }, ..Background::default() });
                }
            }
        }
        let mut worst = [(f32::MAX, f32::MAX); 4];
        for b in &cases {
            let theme_palette = matches!((b.style, b.waves.palette, b.pixel.palette, b.prism.palette), (Style::Waves, WavesPalette::Theme, _, _) | (Style::Pixel, _, WavesPalette::Theme, _) | (Style::Prism, _, _, PrismPalette::Theme));
            let tried: Vec<Color32> = if theme_palette { accents.clone() } else { vec![accents[0]] };
            for accent in tried {
                for (n, (dark, high)) in [(true, false), (false, false), (true, true), (false, true)].into_iter().enumerate() {
                    let pal = palette(dark, high, accent);
                    let (page, muted) = (page(dark), muted(&pal));
                    let guard = Guard::new(&pal, page, muted, Some(column), area);
                    let mut seen = Vec::new();
                    for t in [0.0, 9.0, 300.0] {
                        let mut row = Vec::new();
                        for y in (column.top() as i32..column.bottom() as i32).step_by(10) {
                            for x in (column.left() as i32..column.right() as i32).step_by(10) {
                                let c = to_color(colour_at(b, &pal, page, &guard, area, pos2(x as f32, y as f32), t));
                                let (text, weak) = (theme::contrast(pal.text, c), theme::contrast(muted, c));
                                worst[n] = (worst[n].0.min(text), worst[n].1.min(weak));
                                assert!(text >= if high { 7.0 } else { 4.5 } && weak >= if high { 4.5 } else { 3.0 }, "{b:?} {accent:?} dark {dark} high {high}: text {text:.1}, muted {weak:.1} at {x}, {y}");
                                row.push(c);
                            }
                        }
                        seen.push(row);
                    }
                    // It moves (beyond the words, where it is not held).
                    let moved = (0..40).any(|k| {
                        let p = pos2(30.0 + 31.0 * k as f32, 40.0 + 23.0 * k as f32);
                        let g = Guard::new(&pal, page, muted, None, area);
                        colour_at(b, &pal, page, &g, area, p, 0.0) != colour_at(b, &pal, page, &g, area, p, 9.0)
                    });
                    assert!(moved || b.style == Style::Pixel && b.pixel.speed == 0.0, "{b:?} moves");
                }
            }
        }
        for (n, name) in ["dark", "light", "dark, high contrast", "light, high contrast"].iter().enumerate() {
            eprintln!("{name}: text {:.1}:1, muted {:.1}:1 at worst under the words", worst[n].0, worst[n].1);
        }
    }

    /// The ridges move and leave the words of every theme legible: text
    /// past 4.5 to 1, muted text past 3 to 1, on any patch of dots where the
    /// words are, seen through the letters' halo, at any time, with any
    /// accent.
    #[test]
    fn the_ridges_move_and_stay_calm_under_the_words() {
        let (cols, rows) = (420, 300);
        let calm = [120, 36, 300, 290];
        let lin = |v: u8| {
            let v = v as f32 / 255.0;
            if v <= 0.04045 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
        };
        let srgb = |l: f32| ((if l <= 0.0031308 { l * 12.92 } else { 1.055 * l.powf(1.0 / 2.4) - 0.055 }) * 255.0).round() as u8;
        for (dark, high) in [(true, false), (false, false), (true, true), (false, true)] {
            for (_, accent) in theme::ACCENTS {
                let pal = palette(dark, high, theme::rgb(accent));
                let look = Look::new(&pal, page(dark), muted(&pal));
                let frames: Vec<egui::ColorImage> = [0.0, 9.0, 600.0].iter().map(|&t| frame(&look, cols, rows, Some(calm), t)).collect();
                assert!(frames.windows(2).all(|w| w[0].pixels != w[1].pixels), "dark {dark} high {high}: it moves");
                for f in &frames {
                    for y in (calm[1]..calm[3] - 4).step_by(2) {
                        for x in (calm[0]..calm[2] - 4).step_by(2) {
                            let mut sum = [0.0f32; 3];
                            for dy in 0..4 {
                                for dx in 0..4 {
                                    let c = f.pixels[(y + dy) as usize * cols + (x + dx) as usize];
                                    for (s, v) in sum.iter_mut().zip([c.r(), c.g(), c.b()]) {
                                        *s += lin(v) / 16.0;
                                    }
                                }
                            }
                            let c = theme::mix(page(dark), Color32::from_rgb(srgb(sum[0]), srgb(sum[1]), srgb(sum[2])), HALO);
                            let (text, weak) = (theme::contrast(pal.text, c), theme::contrast(muted(&pal), c));
                            assert!(text >= if high { 7.0 } else { 4.5 } && weak >= 3.0, "dark {dark} high {high} {accent:?}: text {text:.1}, muted {weak:.1}");
                        }
                    }
                }
            }
        }
    }

    /// What a frame of each style costs (MASSIF_PERF): over the home screen
    /// of a window of 1600 × 1000 points on a Retina screen (the ridges'
    /// cells, the others' meshes, whose blending the graphics card does).
    #[test]
    fn a_frame_of_each_background_is_cheap() {
        if std::env::var_os("MASSIF_PERF").is_none() {
            return;
        }
        let pal = palette(true, false, theme::rgb(theme::DEFAULT_ACCENT));
        let (page, muted) = (page(true), muted(&pal));
        let area = Rect::from_min_size(pos2(0.0, 0.0), vec2(1333.0, 1000.0));
        let column = Rect::from_min_max(pos2(276.0, 120.0), pos2(1056.0, 780.0));
        for style in Style::ALL {
            let b = Background { style, ..Background::default() };
            let guard = Guard::new(&pal, page, muted, Some(column), area);
            let look = Look::new(&pal, page, muted);
            let (mut best, mut all, mut corners) = (f64::MAX, 0.0, 0);
            for k in 0..40 {
                let start = std::time::Instant::now();
                if style == Style::Ridges {
                    std::hint::black_box(frame(&look, 680, 500, Some([170, 60, 510, 490]), k as f64 / FPS));
                } else {
                    let m = meshes(&b, &pal, page, &guard, area, k as f32 / FPS as f32).unwrap();
                    corners = m.iter().map(|m| m.vertices.len()).sum();
                    std::hint::black_box(m);
                }
                let took = start.elapsed().as_secs_f64();
                best = best.min(took);
                all += took;
            }
            eprintln!("a frame of {style:?}: best {:.3} ms, mean {:.3} ms{}", best * 1e3, all / 40.0 * 1e3, if corners > 0 { format!(", {corners} corners") } else { String::new() });
        }
    }

    /// Settings saved before the styles (none of these fields) read as the
    /// ridges; each style's settings come back as they were.
    #[test]
    fn a_background_reads_from_little() {
        let b: Background = serde_json::from_str("{}").unwrap();
        assert_eq!(b, Background::default());
        assert_eq!(b.style, Style::Ridges);
        let b: Background = serde_json::from_str(r#"{"style":"Prism","prism":{"shape":"Lens"}}"#).unwrap();
        assert_eq!((b.style, b.prism.shape, b.prism.palette), (Style::Prism, Shape::Lens, PrismPalette::Ember));
    }
}
