//! Geometry that terrain graphs make and pass along beside their rasters:
//! points (where things stand), splines (roads, rivers, walls), footprints
//! (the ground things take) and instances (what stands there: a building
//! of a pack, a structure of the game, an object).
//!
//! All in block coordinates of the world (x east, y up, z south), plain
//! data that compares and serialises, so that a graph caches them by their
//! content and a test can pin them. `kind` names what an item is in the
//! words of the node that made it (`"centre"`, `"main"`, `"tower"`,
//! `"small"`…); `attrs` carries the numbers a later node may want, by name.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Named numbers an item carries.
pub type Attrs = BTreeMap<String, f64>;

/// A side of a square, the way a door or a road faces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dir {
    North,
    East,
    South,
    West,
}

impl Dir {
    /// Quarter turns clockwise from south, the way schematics face their
    /// entrance (south: 0, west: 1, north: 2, east: 3).
    pub const fn turns_from_south(self) -> u8 {
        match self {
            Dir::South => 0,
            Dir::West => 1,
            Dir::North => 2,
            Dir::East => 3,
        }
    }

    /// One block that way, as `(dx, dz)`.
    pub const fn step(self) -> (i32, i32) {
        match self {
            Dir::North => (0, -1),
            Dir::East => (1, 0),
            Dir::South => (0, 1),
            Dir::West => (-1, 0),
        }
    }
}

/// Where something stands.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub pos: [f64; 3],
    /// Turned about the vertical, in degrees clockwise seen from above.
    #[serde(default)]
    pub turn: f32,
    #[serde(default = "one")]
    pub scale: f32,
    /// A number of its own, steady from one evaluation to the next (a seed
    /// for what grows there, a key to match it again).
    #[serde(default)]
    pub id: u64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: Attrs,
}

fn one() -> f32 {
    1.0
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Points {
    pub items: Vec<Point>,
}

/// A line through points: a road, a river, a wall.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spline {
    /// Its points in order, at least two (`y` may be left at the ground's
    /// height by the node that made it).
    pub points: Vec<[f64; 3]>,
    /// The last point joins the first.
    #[serde(default)]
    pub closed: bool,
    /// Across, in blocks.
    #[serde(default = "one")]
    pub width: f32,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: Attrs,
}

impl Spline {
    /// Its length along the points, in blocks (on the ground's plane).
    pub fn length(&self) -> f64 {
        let n = self.points.len();
        let segs = if self.closed { n } else { n.saturating_sub(1) };
        (0..segs)
            .map(|i| {
                let (a, b) = (self.points[i], self.points[(i + 1) % n]);
                ((b[0] - a[0]).powi(2) + (b[2] - a[2]).powi(2)).sqrt()
            })
            .sum()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Splines {
    pub lines: Vec<Spline>,
}

/// The ground something takes, a rectangle of columns.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Footprint {
    /// The first column, west and north.
    pub min: [i32; 2],
    /// Past the last column, east and south.
    pub max: [i32; 2],
    /// The side it opens onto (its road).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub facing: Option<Dir>,
    /// The column of its entrance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub door: Option<[i32; 2]>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: Attrs,
}

impl Footprint {
    pub fn contains(&self, x: i32, z: i32) -> bool {
        (self.min[0]..self.max[0]).contains(&x) && (self.min[1]..self.max[1]).contains(&z)
    }

    pub fn overlaps(&self, o: &Footprint) -> bool {
        self.min[0] < o.max[0] && o.min[0] < self.max[0] && self.min[1] < o.max[1] && o.min[1] < self.max[1]
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Footprints {
    pub items: Vec<Footprint>,
}

/// Something to build at a place: what, where, turned how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Instance {
    /// What it is: `pack:<pack>/<theme>/<path>` (a schematic of a pack),
    /// `structure:<namespace:name>` (a structure of the game),
    /// `object:<name>` (an object of the world's library), `file:<path>`.
    pub asset: String,
    /// The block its schematic's corner (west, bottom, north) lands on,
    /// once turned.
    pub origin: [i32; 3],
    /// Quarter turns clockwise.
    #[serde(default)]
    pub turn: u8,
    /// Mirrored west to east before it turns.
    #[serde(default)]
    pub mirror: bool,
    /// The blocks it takes, min inclusive and max exclusive.
    pub bbox: ([i32; 3], [i32; 3]),
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attrs: Attrs,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Instances {
    pub items: Vec<Instance>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_reads_and_writes() {
        let p = Points { items: vec![Point { pos: [1.0, 64.0, 2.0], turn: 0.0, scale: 1.0, id: 7, kind: "centre".into(), attrs: Attrs::new() }] };
        let text = serde_json::to_string(&p).unwrap();
        assert_eq!(text, r#"{"items":[{"pos":[1.0,64.0,2.0],"turn":0.0,"scale":1.0,"id":7,"kind":"centre"}]}"#);
        assert_eq!(serde_json::from_str::<Points>(&text).unwrap(), p);
        let s: Spline = serde_json::from_str(r#"{"points":[[0,0,0],[3,0,4]],"kind":"main"}"#).unwrap();
        assert_eq!((s.width, s.length()), (1.0, 5.0));
        let a = Footprint { min: [0, 0], max: [8, 8], facing: Some(Dir::South), door: Some([4, 7]), kind: "small".into(), attrs: Attrs::new() };
        let b = Footprint { min: [8, 0], max: [12, 8], ..a.clone() };
        assert!(a.contains(7, 7) && !a.contains(8, 0) && !a.overlaps(&b));
        assert_eq!(serde_json::to_string(&Dir::West).unwrap(), r#""west""#);
        assert_eq!(Dir::East.turns_from_south(), 3);
    }
}
