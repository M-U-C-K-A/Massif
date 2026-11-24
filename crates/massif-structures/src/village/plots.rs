//! Stage 4, the plots: places along both sides of the roads, each where its
//! door would be, the side of the road it is on, the size it would like.

use super::pack::SizeClass;
use super::roads::{RoadKind, Roads};
use super::site::Site;
use super::{Params, Side, dist2};
use crate::rng::Rng;
use serde::{Deserialize, Serialize};

/// A plot by a road.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Plot {
    /// The column in front of the door, just off the road.
    pub door: [i32; 2],
    /// The side of the road the plot is on: its building's door faces the
    /// other way, to the road.
    pub side: Side,
    /// The size of building it would like: more large ones as
    /// `params.size_bias` grows, and away from the squares.
    pub size: SizeClass,
    /// Its district (the index of its square).
    pub district: u16,
    /// How far from its square, in blocks.
    pub from_centre: f32,
    /// By a main road.
    pub main: bool,
}

/// The weight of each size class at `bias` (−1 small … +1 large): what
/// the plots and the choice of variants draw from.
pub fn class_weight(class: SizeClass, bias: f32) -> f32 {
    let base = match class {
        SizeClass::Small => 0.5,
        SizeClass::Medium => 0.35,
        SizeClass::Large => 0.12,
        SizeClass::Huge => 0.03,
    };
    base * (bias.clamp(-1.0, 1.0) * class.rank() * 1.6).exp()
}

/// Stage 4: a plot every 9–15 blocks (fewer as `params.density` falls)
/// along each side of every road, nearest the squares first.
pub fn plots(roads: &Roads, site: &Site, params: &Params, seed: u64) -> Vec<Plot> {
    let mut rng = Rng::new(seed ^ 0x706c_6f74);
    let density = params.density.clamp(0.2, 1.0);
    let mut out = Vec::new();
    for r in roads.lines.iter().filter(|r| r.kind != RoadKind::Square) {
        let line = &r.points;
        let w = r.width;
        let mut k = 2usize;
        while k + 1 < line.len() {
            let (p, q) = (line[k], line[k + 1]);
            let along_x = (q[0] - p[0]).abs() >= (q[1] - p[1]).abs();
            for side in Side::ALL {
                if along_x == matches!(side, Side::North | Side::South) {
                    let s = side.step();
                    // Just past the road's edge on that side.
                    let edge = (w - 1) / 2 + 1 + (side == Side::South || side == Side::East) as i32 * ((w + 1) % 2);
                    let door = [p[0] + s[0] * edge, p[1] + s[1] * edge];
                    let (district, c) = site.centres.iter().enumerate().min_by_key(|(_, c)| dist2(door, **c)).map(|(i, c)| (i, *c)).unwrap_or((0, door));
                    let from_centre = (dist2(door, c) as f32).sqrt();
                    // Larger away from the square, as the bias asks.
                    let out_there = (from_centre / site.reach.max(1) as f32).min(1.5) - 0.5;
                    let bias = (params.size_bias + out_there * 0.4).clamp(-1.0, 1.0);
                    let total: f32 = SizeClass::ALL.iter().map(|c| class_weight(*c, bias)).sum();
                    let mut pick = rng.float() * total;
                    let size = *SizeClass::ALL.iter().find(|c| {
                        let w = class_weight(**c, bias);
                        if pick < w {
                            true
                        } else {
                            pick -= w;
                            false
                        }
                    }).unwrap_or(&SizeClass::Small);
                    out.push(Plot { door, side, size, district: district as u16, from_centre, main: r.kind == RoadKind::Main });
                }
            }
            k += ((rng.between(9, 15) as f32) / density) as usize;
        }
    }
    // The middles first: the buildings that keep to them take their places
    // before the others fill the streets.
    out.sort_by(|a, b| a.from_centre.total_cmp(&b.from_centre));
    out
}
