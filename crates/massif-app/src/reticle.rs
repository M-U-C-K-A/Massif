//! The crosshair at the middle of the player's view (Settings ▸
//! Appearance ▸ Crosshair): its shape, size and opacity, and how it is
//! drawn, white in a soft dark outline so that it reads on snow as on deep
//! water.

use egui::{Color32, Painter, Pos2, Stroke, vec2};
use serde::{Deserialize, Serialize};

/// The shape of the crosshair.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Shape {
    /// A small dot.
    Dot,
    /// Four short strokes around an open middle.
    #[default]
    Cross,
    /// A thin ring around a dot.
    Circle,
    /// A plain plus, thick, as in the game.
    Game,
    /// No crosshair.
    None,
}

impl Shape {
    pub const ALL: [Shape; 5] = [Shape::Dot, Shape::Cross, Shape::Circle, Shape::Game, Shape::None];

    pub fn label(self) -> &'static str {
        match self {
            Shape::Dot => tn!("Dot"),
            Shape::Cross => tn!("Cross"),
            Shape::Circle => tn!("Circle"),
            Shape::Game => tn!("Minecraft"),
            Shape::None => tn!("None"),
        }
    }
}

/// The crosshair as the settings have it.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Reticle {
    pub shape: Shape,
    /// Its width across, in points.
    pub size: f32,
    /// 0 to 1.
    pub opacity: f32,
}

impl Default for Reticle {
    fn default() -> Self {
        Self {
            shape: Shape::Cross,
            size: 14.0,
            opacity: 0.9,
        }
    }
}

/// The sizes offered, in points.
pub const SIZES: std::ops::RangeInclusive<f32> = 6.0..=32.0;

fn key() -> egui::Id {
    egui::Id::unique("massif reticle")
}

/// Draw the crosshair as `reticle` from now on.
pub fn set(ctx: &egui::Context, reticle: Reticle) {
    ctx.data_mut(|d| d.insert_temp(key(), reticle));
}

/// The crosshair as set.
pub fn get(ctx: &egui::Context) -> Reticle {
    ctx.data(|d| d.get_temp(key())).unwrap_or_default()
}

/// Draw `reticle` centred on `c`. Each part twice: a wide, faint dark
/// stroke under a thin white one, so that its edge stays soft and it shows
/// on light ground and on dark.
pub fn paint(painter: &Painter, c: Pos2, reticle: &Reticle) {
    let a = reticle.opacity.clamp(0.0, 1.0);
    if a <= 0.0 || reticle.shape == Shape::None {
        return;
    }
    // Snapped to the pixel grid, the thin strokes stay crisp.
    let ppp = painter.ctx().pixels_per_point();
    let c = Pos2::new((c.x * ppp).round() / ppp, (c.y * ppp).round() / ppp);
    let r = (reticle.size * 0.5).clamp(*SIZES.start() * 0.5, *SIZES.end() * 0.5);
    let ink = Color32::from_white_alpha((255.0 * a) as u8);
    let shade = |k: f32| Color32::from_black_alpha((255.0 * a * k) as u8);
    // The width of the white strokes: thin, a little thicker as it grows.
    let w = (1.0 + r / 12.0).min(2.0);
    match reticle.shape {
        Shape::Dot => {
            let rd = (r * 0.3).max(1.6);
            painter.circle_filled(c, rd + 1.6, shade(0.22));
            painter.circle_filled(c, rd + 0.8, shade(0.35));
            painter.circle_filled(c, rd, ink);
        }
        Shape::Cross => {
            let gap = (r * 0.35).max(2.0);
            let arms = [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)];
            for (width, color) in [(w + 2.6, shade(0.2)), (w + 1.2, shade(0.35)), (w, ink)] {
                for d in arms {
                    painter.line_segment([c + d * gap, c + d * r], Stroke::new(width, color));
                }
            }
            painter.circle_filled(c, w * 0.5 + 0.6, shade(0.35));
            painter.circle_filled(c, w * 0.5, ink);
        }
        Shape::Circle => {
            for (width, color) in [(w + 2.4, shade(0.2)), (w + 1.0, shade(0.35)), (w * 0.9, ink)] {
                painter.circle_stroke(c, r - 1.0, Stroke::new(width, color));
            }
            painter.circle_filled(c, w * 0.6 + 0.8, shade(0.35));
            painter.circle_filled(c, w * 0.6, ink);
        }
        Shape::Game => {
            // Square ends, thick and plain.
            let t = (r / 4.5).max(1.5);
            let h = egui::Rect::from_center_size(c, vec2(2.0 * r, t));
            let v = egui::Rect::from_center_size(c, vec2(t, 2.0 * r));
            for rect in [h, v] {
                painter.rect_filled(rect.expand(1.0), 0, shade(0.3));
            }
            for rect in [h, v] {
                painter.rect_filled(rect, 0, ink);
            }
        }
        Shape::None => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn old_settings_keep_the_default() {
        let r: Reticle = serde_json::from_str("{}").unwrap();
        assert_eq!(r, Reticle::default());
        let r: Reticle = serde_json::from_str(r#"{"shape":"Dot"}"#).unwrap();
        assert_eq!((r.shape, r.size), (Shape::Dot, 14.0));
    }
}
