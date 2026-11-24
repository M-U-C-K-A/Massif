//! The colours annotations are drawn in: WorldPainter's fifteen, numbered
//! 1 to 15 in a tile's annotation plane, and the map's own colours after
//! them (16 to [`MAX`]), each with its opacity on the map.
//!
//! The palette is kept with the map, in its [`crate::Registry`]: a tile
//! holds numbers, the palette what they look like.

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CustomColor {
    pub rgb: [u8; 3],
    /// Percent, 0 to 100.
    #[serde(default = "default_opacity")]
    pub opacity: u8,
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
        if let Some(k) = self.custom.iter().position(|c| c.rgb == rgb) {
            return Some(k as u16 + PRESETS + 1);
        }
        let n = self.custom.len() as u16 + PRESETS + 1;
        if n > MAX {
            return None;
        }
        self.custom.push(CustomColor { rgb, opacity: DEFAULT_OPACITY });
        Some(n)
    }

    /// Every number with a colour: the fifteen, then the map's own.
    pub fn numbers(&self) -> impl Iterator<Item = u16> + '_ {
        1..=PRESETS + self.custom.len() as u16
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
        let back: AnnotationPalette = serde_json::from_str(&serde_json::to_string(&AnnotationPalette { custom: vec![CustomColor { rgb: [1, 2, 3], opacity: 40 }], ..Default::default() }).unwrap()).unwrap();
        assert_eq!(back.color(16), Some([1, 2, 3]));
    }
}
