//! Travel in Minecraft terms: how long a distance takes on foot, on a
//! horse, in a boat, with an elytra…, and the Travel range tool, rings
//! around the pointer (or a point clicked) at the distance each way of
//! going covers in a given time, to lay out a map for its players.
//!
//! Distances are straight and flat, as the crow flies over the map: the
//! land's slopes, its water and its trees are not counted, so the rings say
//! how far one gets at best.

use crate::doc::Document;
use crate::theme::{self, Icon, Palette};
use crate::tools::ToolState;
use egui::{Color32, FontId, Pos2, Rect, Stroke, StrokeKind, Ui, vec2};
use massif_core::compose::block_top;
use serde::{Deserialize, Serialize};

/// A Minecraft day, in seconds of real time: 20 minutes.
pub const DAY: f64 = 1200.0;

/// A way of going about the world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Mode {
    Walk,
    Sprint,
    SprintJump,
    Horse,
    Boat,
    BoatIce,
    Minecart,
    Elytra,
}

impl Mode {
    pub const ALL: [Mode; 8] = [Mode::Walk, Mode::Sprint, Mode::SprintJump, Mode::Horse, Mode::Boat, Mode::BoatIce, Mode::Minecart, Mode::Elytra];

    /// Its speed on flat ground, in blocks a second. From the Minecraft
    /// Wiki (Java Edition): walking 4.317, sprinting 5.612, sprint-jumping
    /// about 7.127 ("Sprinting", "Movement"); a fast horse about 9.5 of the
    /// 4.8–14.2 horses have ("Horse"); a boat 8.0 on water and about 40 on
    /// packed ice ("Boat"); a minecart at most 8.0 on powered rails
    /// ("Minecart"); an elytra boosted by rockets about 33.5 in level
    /// flight ("Elytra").
    pub const fn speed(self) -> f64 {
        match self {
            Mode::Walk => 4.317,
            Mode::Sprint => 5.612,
            Mode::SprintJump => 7.127,
            Mode::Horse => 9.5,
            Mode::Boat => 8.0,
            Mode::BoatIce => 40.0,
            Mode::Minecart => 8.0,
            Mode::Elytra => 33.5,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Walk => tr!("Walking"),
            Mode::Sprint => tr!("Sprinting"),
            Mode::SprintJump => tr!("Sprint-jumping"),
            Mode::Horse => tr!("Fast horse"),
            Mode::Boat => tr!("Boat on water"),
            Mode::BoatIce => tr!("Boat on ice"),
            Mode::Minecart => tr!("Minecart"),
            Mode::Elytra => tr!("Elytra with rockets"),
        }
    }

    /// How long `blocks` take this way, in seconds.
    pub fn time(self, blocks: f64) -> f64 {
        blocks / self.speed()
    }
}

/// A span of time, as one reads it: "45 s", "2 min 30 s", "12 min",
/// "1 h 21 min".
pub fn duration(secs: f64) -> String {
    let s = secs.max(0.0).round() as u64;
    if s < 60 {
        tr!("{n} s", n = s)
    } else if s < 600 {
        if s.is_multiple_of(60) { tr!("{n} min", n = s / 60) } else { tr!("{m} min {s} s", m = s / 60, s = s % 60) }
    } else {
        let m = (secs / 60.0).round() as u64;
        if m < 60 { tr!("{n} min", n = m) } else if m.is_multiple_of(60) { tr!("{h} h", h = m / 60) } else { tr!("{h} h {m} min", h = m / 60, m = m % 60) }
    }
}

/// A number of Minecraft days, one decimal under ten: "1 MC day",
/// "2.5 MC days", "14 MC days".
pub fn days(n: f64) -> String {
    let rounded = if n < 10.0 { (n * 10.0).round() / 10.0 } else { n.round() };
    trn!("{n} MC day", "{n} MC days", n = rounded)
}

/// How long `blocks` take one way of going: "1 h 21 min (≈ 4 MC days)",
/// the days told from a day on.
pub fn time_to(mode: Mode, blocks: f64) -> String {
    let secs = mode.time(blocks);
    if secs >= DAY { tr!("{time} (≈ {days})", time = duration(secs), days = days(secs / DAY)) } else { duration(secs) }
}

// ------------------------------------------------------------- the rings --

/// What a ring's time is counted in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum Unit {
    Minutes,
    #[default]
    Days,
}

/// A ring: how far one way of going gets in a time.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Ring {
    pub amount: f32,
    pub unit: Unit,
    pub mode: Mode,
}

impl Ring {
    /// Its time, in seconds.
    pub fn seconds(&self) -> f64 {
        match self.unit {
            Unit::Minutes => self.amount as f64 * 60.0,
            Unit::Days => self.amount as f64 * DAY,
        }
    }

    /// Its radius, in blocks.
    pub fn radius(&self) -> f64 {
        self.seconds() * self.mode.speed()
    }

    /// Its time as set: "5 min", "1 MC day".
    pub fn time_label(&self) -> String {
        match self.unit {
            Unit::Minutes => duration(self.seconds()),
            Unit::Days => days(self.amount as f64),
        }
    }

    /// What the map writes on it: "1 MC day walking · 5.18 km".
    pub fn label(&self) -> String {
        tr!("{time} · {mode} · {distance}", time = self.time_label(), mode = self.mode.name(), distance = crate::measure::length(self.radius()))
    }
}

/// The Travel range tool's settings: its rings, kept with the tools.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Travel {
    pub rings: Vec<Ring>,
    /// The way of going of a ring added.
    pub mode: Mode,
    /// The centre, clicked: the rings stay there until Esc. Never saved.
    #[serde(skip)]
    pub pin: Option<(f32, f32)>,
}

impl Default for Travel {
    fn default() -> Self {
        Travel { rings: vec![Ring { amount: 1.0, unit: Unit::Days, mode: Mode::Walk }, Ring { amount: 2.0, unit: Unit::Days, mode: Mode::Walk }], mode: Mode::Walk, pin: None }
    }
}

/// The most rings the tool draws.
pub const MAX_RINGS: usize = 8;

/// A label in a pill, centred on `at`.
fn pill(painter: &egui::Painter, pal: &Palette, at: Pos2, text: String) {
    let galley = painter.layout_no_wrap(text, FontId::proportional(11.5), pal.text);
    let r = Rect::from_center_size(at, galley.size() + vec2(14.0, 6.0));
    painter.rect(r, theme::PILL, pal.panel.gamma_multiply(0.94), Stroke::new(1.0, pal.border), StrokeKind::Inside);
    painter.galley(r.min + vec2(7.0, 3.0), galley, pal.text);
}

/// The rings, larger first, and their order from the smallest (for their
/// shade: the nearer, the stronger).
fn sorted(rings: &[Ring]) -> Vec<(usize, Ring)> {
    let mut v: Vec<Ring> = rings.iter().copied().filter(|r| r.radius() > 0.0).take(MAX_RINGS).collect();
    v.sort_by(|a, b| a.radius().total_cmp(&b.radius()));
    v.into_iter().enumerate().collect()
}

/// A ring's stroke: the accent, fainter outward, over a dark halo that
/// keeps it on light and dark land alike.
fn strokes(pal: &Palette, k: usize, n: usize) -> (Stroke, Stroke) {
    let fade = 1.0 - 0.45 * k as f32 / n.max(2).saturating_sub(1) as f32;
    (Stroke::new(4.0, Color32::from_black_alpha(110)), Stroke::new(2.0, pal.accent.gamma_multiply(fade)))
}

/// The rings over the map around `centre` (in blocks); `to_screen` maps
/// blocks to points.
pub fn draw_map(ui: &Ui, painter: &egui::Painter, to_screen: &dyn Fn(f64, f64) -> Pos2, centre: (f32, f32), travel: &Travel) {
    let pal = Palette::of(ui);
    let (cx, cz) = (centre.0 as f64, centre.1 as f64);
    let c = to_screen(cx, cz);
    let rings = sorted(&travel.rings);
    let clip = painter.clip_rect();
    for &(k, ring) in rings.iter().rev() {
        let r = (to_screen(cx + ring.radius(), cz).x - c.x).abs();
        let (under, line) = strokes(&pal, k, rings.len());
        // Far out of sight, or smaller than a dot: nothing to draw.
        if r < 2.0 || (c.distance(clip.center()) - r).abs() > clip.size().length() {
            continue;
        }
        painter.circle_stroke(c, r, under);
        painter.circle_stroke(c, r, line);
        // Its label at its top, or where the top is out of sight, at the
        // top of what shows of it.
        let top = if c.y - r >= clip.top() + 12.0 { egui::pos2(c.x, c.y - r) } else { egui::pos2(c.x.clamp(clip.left() + 80.0, clip.right() - 80.0), clip.top() + 14.0 + 22.0 * k as f32) };
        pill(painter, &pal, top, ring.label());
    }
    painter.circle(c, 4.0, pal.accent, Stroke::new(2.0, Color32::WHITE));
}

/// The rings in a 3D view, laid on the ground; `to_screen` maps a point of
/// the world to the view, `None` behind the eyes.
pub fn draw_3d(ui: &Ui, painter: &egui::Painter, to_screen: &dyn Fn([f32; 3]) -> Option<Pos2>, doc: &Document, centre: (f32, f32), travel: &Travel) {
    let pal = Palette::of(ui);
    let mut ground = crate::measure::DocGround::new(doc);
    let mut height = |x: f32, z: f32| crate::measure::Ground::col(&mut ground, x.floor() as i32, z.floor() as i32, false).map(|c| block_top(c.height) as f32 + 0.15);
    let rings = sorted(&travel.rings);
    for &(k, ring) in rings.iter().rev() {
        let (under, line) = strokes(&pal, k, rings.len());
        let r = ring.radius() as f32;
        // A point every few blocks, 64 to 720 of them.
        let n = ((r * std::f32::consts::TAU / 4.0) as usize).clamp(64, 720);
        let mut run: Vec<Pos2> = Vec::new();
        let mut label = None;
        let flush = |run: &mut Vec<Pos2>| {
            if run.len() > 1 {
                painter.add(egui::Shape::line(run.clone(), under));
                painter.add(egui::Shape::line(std::mem::take(run), line));
            }
            run.clear();
        };
        for i in 0..=n {
            let a = i as f32 / n as f32 * std::f32::consts::TAU;
            let (x, z) = (centre.0 + r * a.sin(), centre.1 - r * a.cos());
            match height(x, z).and_then(|y| to_screen([x, y, z])) {
                Some(p) => {
                    label.get_or_insert(p);
                    run.push(p);
                }
                None => flush(&mut run),
            }
        }
        flush(&mut run);
        if let Some(p) = label {
            pill(painter, &pal, p - vec2(0.0, 12.0), ring.label());
        }
    }
    if let Some(p) = height(centre.0, centre.1).and_then(|y| to_screen([centre.0, y, centre.1])) {
        painter.circle(p, 4.0, pal.accent, Stroke::new(2.0, Color32::WHITE));
    }
}

/// Where the rings are: the pin, or else the pointer.
pub fn centre(travel: &Travel, pointer: Option<(f32, f32)>) -> Option<(f32, f32)> {
    travel.pin.or(pointer)
}

// ---------------------------------------------------------------- panels --

/// The tool's options, in Tool Settings: its rings, each a time, a unit
/// and a way of going.
pub fn settings(ui: &mut Ui, tools: &mut ToolState) {
    use crate::widgets::{Field, button_group, combo, well};
    let travel = &mut tools.travel;
    let mut remove = None;
    well(ui, |ui| {
        for (i, ring) in travel.rings.iter_mut().enumerate() {
            // Its time and unit, its radius, its remove button; under them
            // its way of going.
            if i > 0 {
                ui.add_space(4.0);
            }
            ui.horizontal(|ui| {
                let max = if ring.unit == Unit::Days { 100.0 } else { 600.0 };
                ui.add(Field::new(&mut ring.amount, 0.1..=max).max_decimals(1).width(44.0));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.add(egui::Button::new(theme::glyph(Icon::X).to_string()).frame(false)).on_hover_text(tr!("Remove this ring")).clicked() {
                        remove = Some(i);
                    }
                    ui.label(egui::RichText::new(crate::measure::length(ring.radius())).size(11.0).color(Palette::of(ui).weak));
                });
            });
            let units = [Unit::Minutes, Unit::Days];
            if let Some(k) = button_group(ui, &format!("travel unit {i}"), &[tr!("min"), tr!("MC days")], &units.map(|u| ring.unit == u)) {
                ring.unit = units[k];
            }
            combo(("travel mode", i)).width((ui.available_width() - 30.0).max(40.0)).selected_text(ring.mode.name()).show_ui(ui, |ui| {
                for m in Mode::ALL {
                    ui.selectable_value(&mut ring.mode, m, m.name());
                }
            });
        }
        if travel.rings.is_empty() {
            ui.label(egui::RichText::new(tr!("No ring")).color(Palette::of(ui).weak));
        }
    });
    if let Some(i) = remove {
        travel.rings.remove(i);
    }
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(travel.rings.len() < MAX_RINGS, egui::Button::new(theme::with_icon(Icon::Plus, tr!("Add")))).on_hover_text(tr!("Add a ring")).clicked() {
            let last = travel.rings.last().copied();
            travel.rings.push(last.unwrap_or(Ring { amount: 5.0, unit: Unit::Minutes, mode: travel.mode }));
        }
        if ui.add_enabled(travel.pin.is_some(), egui::Button::new(theme::with_icon(Icon::X, tr!("Unpin")))).clicked() {
            travel.pin = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_read_as_one_says_them() {
        assert_eq!(duration(0.4), "0 s");
        assert_eq!(duration(45.0), "45 s");
        assert_eq!(duration(150.0), "2 min 30 s");
        assert_eq!(duration(300.0), "5 min");
        assert_eq!(duration(12.0 * 60.0 + 20.0), "12 min");
        assert_eq!(duration(3600.0), "1 h");
        assert_eq!(duration(81.0 * 60.0), "1 h 21 min");
        assert_eq!(days(1.0), "1 MC day");
        assert_eq!(days(2.5), "2.5 MC days");
        assert_eq!(days(4.04), "4 MC days");
        assert_eq!(days(14.4), "14 MC days");
    }

    /// 21 km walked: 81 minutes, about four Minecraft days; a short way
    /// tells no days.
    #[test]
    fn travel_times() {
        let km21 = 21_060.0;
        assert_eq!(time_to(Mode::Walk, km21), "1 h 21 min (≈ 4.1 MC days)");
        assert_eq!(time_to(Mode::Elytra, km21), "10 min");
        assert_eq!(time_to(Mode::Sprint, 100.0), "18 s");
        assert_eq!(time_to(Mode::Boat, 8.0 * 1200.0), "20 min (≈ 1 MC day)");
        for m in Mode::ALL {
            assert!(m.speed() > 4.0 && m.speed() < 50.0, "{m:?}");
        }
    }

    /// A ring's radius is its time by its speed: a Minecraft day's walk
    /// is 1,200 s × 4.317 b/s.
    #[test]
    fn ring_radii() {
        let day = Ring { amount: 1.0, unit: Unit::Days, mode: Mode::Walk };
        assert!((day.radius() - 5180.4).abs() < 1e-6, "{}", day.radius());
        let elytra = Ring { amount: 3.0, unit: Unit::Minutes, mode: Mode::Elytra };
        assert!((elytra.radius() - 6030.0).abs() < 1e-6);
        assert_eq!(day.label(), "1 MC day · Walking · 5.18 km");
        assert_eq!(Ring { amount: 5.0, unit: Unit::Minutes, mode: Mode::Walk }.label(), "5 min · Walking · 1.30 km");
        let t = Travel::default();
        assert_eq!(t.rings.len(), 2);
        let s = sorted(&[elytra, day, Ring { amount: 0.0, ..day }]);
        assert_eq!(s.iter().map(|(k, r)| (*k, r.mode)).collect::<Vec<_>>(), vec![(0, Mode::Walk), (1, Mode::Elytra)], "smaller first, none empty");
    }
}
