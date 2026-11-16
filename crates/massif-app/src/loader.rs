//! What shows that Massif is working: a small grid of dots lighting in a
//! pattern, in the accent colour, rather than a turning arc.
//!
//! The pattern is chosen in Settings ▸ Appearance. With Reduce motion on, a
//! loader is a still frame of its pattern and asks for no repaint.

use crate::theme::{self, Palette};
use egui::{Color32, Rect, Response, Sense, Ui, pos2, vec2};
use serde::{Deserialize, Serialize};

/// The dots on a side of the grid.
const N: usize = 5;

/// How the dots of a loader light.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Pattern {
    /// A dot running round the edge, a tail after it.
    #[default]
    Orbit,
    /// Rings going out from the middle.
    Ripple,
    /// A column of light sweeping to and fro.
    Scan,
    /// Drops falling down the columns, each at its pace.
    Rain,
    /// Columns rising and falling, as a sound meter's.
    Bars,
    /// A dot winding in from the corner to the middle.
    Spiral,
    /// Waves crossing it from corner to corner.
    Sweep,
    /// Dots lighting here and there.
    Twinkle,
}

impl Pattern {
    pub const ALL: [Pattern; 8] = [Pattern::Orbit, Pattern::Ripple, Pattern::Scan, Pattern::Rain, Pattern::Bars, Pattern::Spiral, Pattern::Sweep, Pattern::Twinkle];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::Orbit => tr!("Orbit"),
            Pattern::Ripple => tr!("Ripple"),
            Pattern::Scan => tr!("Scan"),
            Pattern::Rain => tr!("Rain"),
            Pattern::Bars => tr!("Bars"),
            Pattern::Spiral => tr!("Spiral"),
            Pattern::Sweep => tr!("Sweep"),
            Pattern::Twinkle => tr!("Twinkle"),
        }
    }

    /// How lit the dot at column `x`, row `y` (from the top) is at `t`
    /// seconds: 0 dark to 1 fully lit.
    pub fn light(self, x: usize, y: usize, t: f64) -> f32 {
        let (fx, fy) = (x as f64, y as f64);
        let c = (N - 1) as f64 / 2.0;
        let v = match self {
            Pattern::Orbit => {
                let ring = perimeter();
                match ring.iter().position(|&p| p == (x, y)) {
                    Some(i) => tail(t * 11.0, i, ring.len(), 5.0),
                    // The middle breathes slowly.
                    None if (x, y) == (2, 2) => 0.25 + 0.25 * (t * 3.0).sin(),
                    None => 0.0,
                }
            }
            Pattern::Ripple => {
                let d = ((fx - c).powi(2) + (fy - c).powi(2)).sqrt();
                let w = (d / 3.0 - t * 0.9).rem_euclid(1.0);
                pulse(w, 0.28)
            }
            Pattern::Scan => {
                // A triangle wave over the columns, to and fro.
                let p = (t * 0.9).rem_euclid(2.0);
                let head = if p < 1.0 { p } else { 2.0 - p } * (N - 1) as f64;
                (1.0 - (fx - head).abs() / 1.4).max(0.0).powf(1.4)
            }
            Pattern::Rain => {
                let pace = 1.2 + 0.35 * hash(x, 7);
                let head = ((t * pace + hash(x, 3) * 2.0).rem_euclid(1.6)) * N as f64 - 1.0;
                let k = head - fy;
                if (0.0..3.2).contains(&k) { (1.0 - k / 3.2).powi(2) } else { 0.0 }
            }
            Pattern::Bars => {
                let h = 1.0 + 3.6 * (0.5 + 0.5 * (t * (3.0 + 1.3 * hash(x, 11)) + hash(x, 5) * 6.3).sin());
                let from_bottom = (N - 1 - y) as f64;
                if from_bottom + 1.0 <= h {
                    0.45 + 0.55 * (from_bottom + 1.0) / h
                } else if from_bottom < h {
                    (h - from_bottom) * 0.8
                } else {
                    0.0
                }
            }
            Pattern::Spiral => {
                let path = spiral();
                let i = path.iter().position(|&p| p == (x, y)).unwrap_or(0);
                tail(t * 14.0, i, path.len() + 6, 7.0)
            }
            Pattern::Sweep => {
                let w = ((fx + fy) / (2.0 * (N - 1) as f64) * 0.8 - t * 0.85).rem_euclid(1.0);
                pulse(w, 0.3)
            }
            Pattern::Twinkle => {
                let phase = hash(x, y * 13 + 1);
                let pace = 0.55 + 0.5 * hash(y, x * 17 + 2);
                let s = (std::f64::consts::TAU * (t * pace + phase)).sin().max(0.0);
                s.powi(4)
            }
        };
        v.clamp(0.0, 1.0) as f32
    }

    /// A moment that shows the pattern well, for a still loader.
    fn still_time(self) -> f64 {
        match self {
            Pattern::Orbit => 0.9,
            Pattern::Ripple => 0.5,
            Pattern::Scan => 0.4,
            Pattern::Rain => 0.7,
            Pattern::Bars => 0.6,
            Pattern::Spiral => 1.2,
            Pattern::Sweep => 0.5,
            Pattern::Twinkle => 0.8,
        }
    }
}

/// A light at the head of a run of `len` places, at `head` (wrapping), a
/// tail `length` long after it: how lit place `i` is.
fn tail(head: f64, i: usize, len: usize, length: f64) -> f64 {
    let k = (head - i as f64).rem_euclid(len as f64);
    if k < length { (1.0 - k / length).powf(1.5) } else { 0.0 }
}

/// A band of light at the start of each turn of `w` (0 to 1), `width` wide.
fn pulse(w: f64, width: f64) -> f64 {
    if w < width { (std::f64::consts::PI * w / width).sin() } else { 0.0 }
}

/// A number from 0 to 1 that looks random, the same for the same `a`, `b`.
fn hash(a: usize, b: usize) -> f64 {
    let mut h = (a as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15) ^ (b as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F);
    h ^= h >> 29;
    h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
    h ^= h >> 32;
    (h % 10_000) as f64 / 10_000.0
}

/// The dots of the edge, clockwise from the top left.
fn perimeter() -> Vec<(usize, usize)> {
    let m = N - 1;
    let mut v: Vec<(usize, usize)> = (0..m).map(|x| (x, 0)).collect();
    v.extend((0..m).map(|y| (m, y)));
    v.extend((0..m).map(|k| (m - k, m)));
    v.extend((0..m).map(|k| (0, m - k)));
    v
}

/// Every dot, winding clockwise from the top left to the middle.
fn spiral() -> Vec<(usize, usize)> {
    let mut v = Vec::with_capacity(N * N);
    let (mut lo, mut hi) = (0usize, N - 1);
    while lo <= hi {
        for x in lo..=hi {
            v.push((x, lo));
        }
        for y in lo + 1..=hi {
            v.push((hi, y));
        }
        if lo < hi {
            for x in (lo..hi).rev() {
                v.push((x, hi));
            }
            for y in (lo + 1..hi).rev() {
                v.push((lo, y));
            }
        }
        lo += 1;
        if hi == 0 {
            break;
        }
        hi -= 1;
    }
    v
}

fn key() -> egui::Id {
    egui::Id::unique("massif loader pattern")
}

/// The pattern the loaders of `ctx` show.
pub fn pattern(ctx: &egui::Context) -> Pattern {
    ctx.data(|d| d.get_temp(key())).unwrap_or_default()
}

pub fn set_pattern(ctx: &egui::Context, p: Pattern) {
    ctx.data_mut(|d| d.insert_temp(key(), p));
}

/// A loader `size` points square, in the pattern of the settings.
pub fn loader(ui: &mut Ui, size: f32) -> Response {
    let p = pattern(ui.ctx());
    loader_of(ui, size, p)
}

/// A loader `size` points square, in pattern `p`.
pub fn loader_of(ui: &mut Ui, size: f32, p: Pattern) -> Response {
    let (rect, response) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::ProgressIndicator, true, tr!("Working")));
    paint(ui, rect, p);
    response
}

/// A loader drawn over `rect` (a square in its middle), in pattern `p`.
pub fn paint(ui: &Ui, rect: Rect, p: Pattern) {
    if !ui.is_rect_visible(rect) {
        return;
    }
    let pal = Palette::of(ui);
    let still = theme::reduce_motion(ui.ctx());
    let t = if still { p.still_time() } else { ui.input(|i| i.time) };
    if !still {
        ui.ctx().request_repaint();
    }
    let side = rect.width().min(rect.height());
    let cell = side / N as f32;
    let r = (cell * 0.31).max(0.8);
    let origin = rect.center() - vec2(side, side) / 2.0;
    let dark = dim(&pal);
    let painter = ui.painter();
    for y in 0..N {
        for x in 0..N {
            let l = p.light(x, y, t);
            let c = pos2(origin.x + (x as f32 + 0.5) * cell, origin.y + (y as f32 + 0.5) * cell);
            painter.circle_filled(c, r, theme::mix(dark, pal.accent, l));
        }
    }
}

/// The colour of a dot not lit: a faint grey over what is under it.
fn dim(pal: &Palette) -> Color32 {
    theme::mix(pal.panel, pal.text, if pal.dark { 0.16 } else { 0.13 })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each pattern lights some dots and leaves some dark, now and then,
    /// and moves: the dots differ from one moment to the next.
    #[test]
    fn every_pattern_moves() {
        for p in Pattern::ALL {
            let frame = |t: f64| -> Vec<f32> { (0..N * N).map(|k| p.light(k % N, k / N, t)).collect() };
            let mut lit = 0.0f32;
            for k in 0..40 {
                let f = frame(k as f64 * 0.1);
                lit = lit.max(f.iter().copied().fold(0.0, f32::max));
                assert!(f.iter().all(|v| (0.0..=1.0).contains(v)), "{p:?} out of range");
            }
            assert!(lit > 0.8, "{p:?} never lights fully");
            assert_ne!(frame(0.0), frame(0.15), "{p:?} does not move");
            assert!(frame(p.still_time()).iter().any(|v| *v > 0.5), "{p:?} is dark when still");
        }
    }

    #[test]
    fn the_spiral_and_the_edge_cover_their_dots() {
        let s = spiral();
        assert_eq!(s.len(), N * N);
        assert_eq!(s.last(), Some(&(2, 2)));
        let mut sorted = s.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), N * N);
        assert_eq!(perimeter().len(), 4 * (N - 1));
    }
}
