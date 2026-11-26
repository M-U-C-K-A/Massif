//! The colours annotations are drawn in: WorldPainter's fifteen, numbered
//! 1 to 15 in a tile's annotation plane, and the map's own colours after
//! them (16 to [`MAX`]), each with its opacity on the map.
//!
//! The palette is kept with the map, in its [`crate::Registry`]: a tile
//! holds numbers, the palette what they look like. A colour of the map's
//! own may have a name: it is then an annotation layer of its own (as
//! WorldPainter's custom annotation layers), listed by name in the Layers
//! panel and shown or hidden on its own.

use crate::tile::ANNOTATION_COLORS;
use serde::{Deserialize, Serialize};

/// WorldPainter's colours: 1 to 15.
pub const PRESETS: u16 = 15;
/// The highest number of an annotation: the map's own colours run from 16
/// to here (the map draws a column's annotation in a byte).
pub const MAX: u16 = 255;
/// How opaque a colour is drawn unless set otherwise, in percent: the
/// mark of colour the map always drew.
pub const DEFAULT_OPACITY: u8 = 75;

/// A colour of the map's own (see [`AnnotationPalette::custom`]).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CustomColor {
    pub rgb: [u8; 3],
    /// Percent, 0 to 100.
    #[serde(default = "default_opacity")]
    pub opacity: u8,
    /// Its name, as an annotation layer; empty for a plain colour.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Hidden on the map and in 3D (its marks are kept).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    /// The palette it is sorted into in the Layers panel, as an annotation
    /// layer ("": none, it stays with the annotations).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub palette: String,
    /// Deleted as an annotation layer: no longer listed, offered or drawn
    /// (its number is kept, so that the others keep theirs, and its marks
    /// come back with an undo).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub removed: bool,
}

impl CustomColor {
    pub fn new(rgb: [u8; 3]) -> Self {
        Self { rgb, opacity: DEFAULT_OPACITY, name: String::new(), hidden: false, palette: String::new(), removed: false }
    }
}

fn default_opacity() -> u8 {
    DEFAULT_OPACITY
}

fn default_presets() -> [u8; 15] {
    [DEFAULT_OPACITY; 15]
}

/// What the annotations of a map look like.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct AnnotationPalette {
    /// The opacity of each of the fifteen colours, in percent.
    #[serde(default = "default_presets")]
    pub presets: [u8; 15],
    /// The map's own colours, numbered from 16.
    pub custom: Vec<CustomColor>,
}

impl Default for AnnotationPalette {
    fn default() -> Self {
        Self { presets: default_presets(), custom: Vec::new() }
    }
}

impl AnnotationPalette {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// The colour of annotation `n`, `None` for 0 and for a number with
    /// no colour.
    pub fn color(&self, n: u16) -> Option<[u8; 3]> {
        match n {
            0 => None,
            1..=PRESETS => Some(ANNOTATION_COLORS[n as usize - 1]),
            _ => self.custom.get((n - PRESETS - 1) as usize).map(|c| c.rgb),
        }
    }

    /// The opacity of annotation `n`, in percent (0 where it has none).
    pub fn opacity(&self, n: u16) -> u8 {
        match n {
            0 => 0,
            1..=PRESETS => self.presets[n as usize - 1],
            _ => self.custom.get((n - PRESETS - 1) as usize).map_or(0, |c| c.opacity),
        }
    }

    /// How opaque annotation `n` is drawn: its opacity, 0 when its layer
    /// is hidden or deleted.
    pub fn drawn_opacity(&self, n: u16) -> u8 {
        match n {
            0..=PRESETS => self.opacity(n),
            _ => self.custom.get((n - PRESETS - 1) as usize).map_or(0, |c| if c.hidden || c.removed { 0 } else { c.opacity }),
        }
    }

    /// Whether annotation `n` is an annotation layer sorted into a palette
    /// of the Layers panel: shown or hidden by its own eye (its palette's),
    /// whatever the eye of all the annotations says.
    pub fn in_palette(&self, n: u16) -> bool {
        n > PRESETS && self.custom.get((n - PRESETS - 1) as usize).is_some_and(|c| !c.name.is_empty() && !c.palette.is_empty())
    }

    /// How opaque annotation `n` is drawn with the annotations shown
    /// (`all`) or hidden: a layer in a palette as its own eye says, the
    /// others only with all the annotations.
    pub fn shown_opacity(&self, n: u16, all: bool) -> u8 {
        if all || self.in_palette(n) { self.drawn_opacity(n) } else { 0 }
    }

    /// Whether annotation `n` is a deleted annotation layer.
    pub fn is_removed(&self, n: u16) -> bool {
        n > PRESETS && self.custom.get((n - PRESETS - 1) as usize).is_some_and(|c| c.removed)
    }

    /// The annotation layers: the map's own colours that have a name, by
    /// number.
    pub fn layers(&self) -> impl Iterator<Item = (u16, &CustomColor)> + '_ {
        self.custom.iter().enumerate().filter(|(_, c)| !c.name.is_empty() && !c.removed).map(|(k, c)| (k as u16 + PRESETS + 1, c))
    }

    /// Whether `n` is an annotation layer (a colour of the map's own with
    /// a name): one that overlaps the others on a column rather than
    /// taking their place.
    pub fn is_layer(&self, n: u16) -> bool {
        n > PRESETS && self.custom.get((n - PRESETS - 1) as usize).is_some_and(|c| !c.name.is_empty())
    }

    /// The annotation layer `n`, if it is one.
    pub fn layer_mut(&mut self, n: u16) -> Option<&mut CustomColor> {
        n.checked_sub(PRESETS + 1).and_then(|k| self.custom.get_mut(k as usize))
    }

    /// A new annotation layer named `name`, in colour `rgb` (another of the
    /// same colour may exist); `None` when the map holds all it can.
    pub fn add_layer(&mut self, name: &str, rgb: [u8; 3]) -> Option<u16> {
        let n = self.custom.len() as u16 + PRESETS + 1;
        if n > MAX {
            return None;
        }
        self.custom.push(CustomColor { name: name.to_owned(), ..CustomColor::new(rgb) });
        Some(n)
    }

    /// A name for a new annotation layer: `Annotations 2`, `Annotations 3`…
    pub fn free_name(&self, base: &str) -> String {
        let taken = |n: &str| self.custom.iter().any(|c| c.name == n);
        if !taken(base) {
            return base.to_owned();
        }
        (2..).map(|k| format!("{base} {k}")).find(|n| !taken(n)).expect("a free name")
    }

    pub fn set_opacity(&mut self, n: u16, opacity: u8) {
        let opacity = opacity.min(100);
        match n {
            0 => {}
            1..=PRESETS => self.presets[n as usize - 1] = opacity,
            _ => {
                if let Some(c) = self.custom.get_mut((n - PRESETS - 1) as usize) {
                    c.opacity = opacity;
                }
            }
        }
    }

    /// Change the colour of one of the map's own colours (the fifteen
    /// stay WorldPainter's).
    pub fn set_color(&mut self, n: u16, rgb: [u8; 3]) {
        if n > PRESETS
            && let Some(c) = self.custom.get_mut((n - PRESETS - 1) as usize)
        {
            c.rgb = rgb;
        }
    }

    /// The number of colour `rgb`: one of the fifteen or of the map's own
    /// that is exactly it, else a new colour of the map's; `None` when the
    /// map has all the colours it can hold.
    pub fn number_of(&mut self, rgb: [u8; 3]) -> Option<u16> {
        if let Some(k) = ANNOTATION_COLORS.iter().position(|&c| c == rgb) {
            return Some(k as u16 + 1);
        }
        if let Some(k) = self.custom.iter().position(|c| c.rgb == rgb && c.name.is_empty()) {
            return Some(k as u16 + PRESETS + 1);
        }
        let n = self.custom.len() as u16 + PRESETS + 1;
        if n > MAX {
            return None;
        }
        self.custom.push(CustomColor::new(rgb));
        Some(n)
    }

    /// Every number with a colour: the fifteen, then the map's own (but
    /// the annotation layers deleted).
    pub fn numbers(&self) -> impl Iterator<Item = u16> + '_ {
        (1..=PRESETS + self.custom.len() as u16).filter(|&n| !self.is_removed(n))
    }

    /// The nearest colour to `c` (1 to 15, or one of the map's own).
    pub fn nearest(&self, c: [u8; 3]) -> u16 {
        let d = |a: [u8; 3]| -> i32 { (0..3).map(|k| (a[k] as i32 - c[k] as i32).pow(2)).sum() };
        self.numbers().min_by_key(|&n| d(self.color(n).unwrap_or([0; 3]))).unwrap_or(1)
    }

    /// What tells two palettes apart, for the caches of the views.
    pub fn fingerprint(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_then_the_maps_own_colours() {
        let mut p = AnnotationPalette::default();
        assert_eq!(p.color(14), Some(ANNOTATION_COLORS[13]));
        assert_eq!(p.opacity(14), DEFAULT_OPACITY);
        assert_eq!(p.number_of(ANNOTATION_COLORS[2]), Some(3), "a preset is found, not added");
        assert_eq!(p.number_of([1, 2, 3]), Some(16));
        assert_eq!(p.number_of([1, 2, 3]), Some(16), "the same colour twice is one");
        assert_eq!(p.number_of([4, 5, 6]), Some(17));
        assert_eq!(p.color(17), Some([4, 5, 6]));
        assert_eq!(p.color(18), None);
        p.set_opacity(17, 30);
        p.set_opacity(2, 120);
        assert_eq!((p.opacity(17), p.opacity(2)), (30, 100));
        assert_eq!(p.nearest([5, 5, 6]), 17);
        for k in 0..(MAX - 17) {
            assert!(p.number_of([200, (k >> 8) as u8, k as u8]).is_some());
        }
        assert_eq!(p.number_of([9, 9, 9]), None, "full at {MAX}");
    }

    #[test]
    fn an_old_map_reads_with_the_default_palette() {
        let p: AnnotationPalette = serde_json::from_str("{}").unwrap();
        assert!(p.is_default());
        let back: AnnotationPalette = serde_json::from_str(&serde_json::to_string(&AnnotationPalette { custom: vec![CustomColor { opacity: 40, ..CustomColor::new([1, 2, 3]) }], ..Default::default() }).unwrap()).unwrap();
        assert_eq!(back.color(16), Some([1, 2, 3]));
    }

    /// Named colours are annotation layers: listed, hidden on their own,
    /// kept apart from a plain colour of the same rgb, and written with
    /// their name (an old map reads with none).
    #[test]
    fn named_colours_are_annotation_layers() {
        let mut p = AnnotationPalette::default();
        let plain = p.number_of([10, 20, 30]).unwrap();
        let misty = p.add_layer("Misties Biome 8", [10, 20, 30]).unwrap();
        assert_ne!(plain, misty);
        assert_eq!(p.number_of([10, 20, 30]), Some(plain), "the plain colour, not the layer");
        assert_eq!(p.layers().map(|(n, c)| (n, c.name.as_str())).collect::<Vec<_>>(), [(misty, "Misties Biome 8")]);
        p.layer_mut(misty).unwrap().hidden = true;
        assert_eq!((p.drawn_opacity(misty), p.opacity(misty), p.drawn_opacity(plain)), (0, DEFAULT_OPACITY, DEFAULT_OPACITY));
        assert_eq!(p.free_name("Misties Biome 8"), "Misties Biome 8 2");
        let json = serde_json::to_string(&p).unwrap();
        assert!(json.contains("\"name\":\"Misties Biome 8\"") && json.contains("\"hidden\":true"), "{json}");
        let back: AnnotationPalette = serde_json::from_str(&json).unwrap();
        assert_eq!(back, p);
        assert!(!serde_json::to_string(&AnnotationPalette { custom: vec![CustomColor::new([1, 2, 3])], ..Default::default() }).unwrap().contains("name"));
    }
}
