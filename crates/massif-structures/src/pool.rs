//! Template pools: the pieces a jigsaw draws the next piece from.

use crate::geom::{BBox, Dir};
use crate::library::Library;
use crate::processor::Processor;
use crate::rng::Rng;
use crate::template::{Jigsaw, Joint};
use std::sync::Arc;

/// Whether a piece is placed as it is, or block by block on the land.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    Rigid,
    /// Each block as high above the land as it was above the template's
    /// floor: paths and roads.
    TerrainMatching,
}

/// What a pool holds.
#[derive(Debug, Clone)]
pub enum Element {
    /// A template, with its processors.
    Single {
        location: String,
        processors: Arc<Vec<Processor>>,
        /// The name of their list, if they have one.
        processors_name: String,
        projection: Projection,
        /// The kind used before 1.17 (villages, outposts, bastions): its air
        /// leaves the world as it is rather than carving it.
        legacy: bool,
    },
    /// Several placed together, the first's jigsaws joining the rest.
    List { elements: Vec<Element>, projection: Projection },
    /// A feature of the game (a tree, a pile of hay, a patch of sculk) at a
    /// spot, as it would grow there.
    Feature { feature: String, projection: Projection },
    /// Nothing: a jigsaw left as it is.
    Empty,
}

impl Element {
    pub fn projection(&self) -> Projection {
        match self {
            Element::Single { projection, .. } | Element::List { projection, .. } | Element::Feature { projection, .. } => *projection,
            Element::Empty => Projection::Rigid,
        }
    }

    pub fn is_empty(&self) -> bool {
        matches!(self, Element::Empty)
    }

    /// How far above the bottom of its box its floor is.
    pub fn ground_level_delta(&self) -> i32 {
        match self {
            Element::Empty => 0,
            _ => 1,
        }
    }

    /// Its box with its corner at `origin`, turned.
    pub fn bbox(&self, lib: &Library, origin: [i32; 3], turn: u8) -> Option<BBox> {
        match self {
            Element::Single { location, .. } => Some(lib.template(location)?.bbox(origin, turn)),
            Element::List { elements, .. } => elements.iter().filter_map(|e| e.bbox(lib, origin, turn)).reduce(|a, b| a.union(&b)),
            // As the game has it: a box of two blocks a side.
            Element::Feature { .. } => Some(BBox { min: origin, max: [origin[0] + 1, origin[1] + 1, origin[2] + 1] }),
            Element::Empty => None,
        }
    }

    /// Its jigsaws with its corner at `origin`, turned, in the order the
    /// game tries them: shuffled, then those of higher selection priority
    /// first.
    pub fn jigsaws(&self, lib: &Library, origin: [i32; 3], turn: u8, rng: &mut Rng) -> Vec<Jigsaw> {
        let mut list = match self {
            Element::Single { location, .. } => lib.template(location).map(|t| t.jigsaws_at(origin, turn)).unwrap_or_default(),
            Element::List { elements, .. } => elements.first().map(|e| e.jigsaws(lib, origin, turn, rng)).unwrap_or_default(),
            Element::Feature { .. } => vec![Jigsaw {
                pos: origin,
                front: Dir::Down,
                top: Dir::South,
                name: "minecraft:bottom".into(),
                target: "minecraft:empty".into(),
                pool: "minecraft:empty".into(),
                joint: Joint::Rollable,
                final_state: "minecraft:air".into(),
                placement_priority: 0,
                selection_priority: 0,
            }],
            Element::Empty => Vec::new(),
        };
        if !matches!(self, Element::List { .. }) {
            rng.shuffle(&mut list);
            list.sort_by_key(|j| std::cmp::Reverse(j.selection_priority));
        }
        list
    }

    /// The template it places, if one.
    pub fn location(&self) -> Option<&str> {
        match self {
            Element::Single { location, .. } => Some(location),
            Element::List { elements, .. } => elements.iter().find_map(|e| e.location()),
            _ => None,
        }
    }
}

/// A pool: its elements with their weights, and the pool drawn from once
/// the structure is as deep as it goes.
#[derive(Debug, Clone)]
pub struct Pool {
    pub name: String,
    pub fallback: String,
    pub elements: Vec<(Arc<Element>, u32)>,
}

impl Pool {
    /// Each element as many times as its weight, shuffled: the order the
    /// game tries them in.
    pub fn shuffled(&self, rng: &mut Rng) -> Vec<Arc<Element>> {
        let mut out: Vec<Arc<Element>> = self.elements.iter().flat_map(|(e, w)| std::iter::repeat_n(e.clone(), *w as usize)).collect();
        rng.shuffle(&mut out);
        out
    }

    /// An element drawn by weight.
    pub fn random(&self, rng: &mut Rng) -> Option<Arc<Element>> {
        let total: u32 = self.elements.iter().map(|e| e.1).sum();
        if total == 0 {
            return None;
        }
        let mut r = rng.below(total);
        for (e, w) in &self.elements {
            if r < *w {
                return Some(e.clone());
            }
            r -= w;
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.elements.iter().all(|e| e.1 == 0)
    }
}
