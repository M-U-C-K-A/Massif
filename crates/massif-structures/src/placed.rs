//! How a structure placed by hand can be made again: its recipe, kept with
//! it in the map (`Structure::recipe`), which the Structures tool reads to
//! move it, resize it or draw it again with another seed.

use crate::village::Params;
use massif_core::coords::BlockRect;
use serde::{Deserialize, Serialize};

/// What was placed, where, and how.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "what", rename_all = "snake_case")]
pub enum Recipe {
    /// A village of a theme (`pack` empty for the game's own) over a
    /// rectangle round `centre`.
    Village { pack: String, theme: String, centre: [i32; 2], size: [i32; 2], params: Params, seed: u64 },
    /// One of the game's structures (`kind`: Massif's key of it,
    /// `ancient_city`), at a column (and a height for those placed at one).
    Game { kind: String, at: [i32; 3], turn: Option<u8>, seed: u64, flatten: bool },
}

impl Recipe {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(text: &str) -> Option<Recipe> {
        serde_json::from_str(text).ok()
    }

    /// A village's rectangle.
    pub fn area(&self) -> Option<BlockRect> {
        match self {
            Recipe::Village { centre, size, .. } => {
                let (w, h) = (size[0].max(16), size[1].max(16));
                Some(BlockRect::new(centre[0] - w / 2, centre[1] - h / 2, centre[0] - w / 2 + w, centre[1] - h / 2 + h))
            }
            Recipe::Game { .. } => None,
        }
    }

    pub fn seed(&self) -> u64 {
        match self {
            Recipe::Village { seed, .. } | Recipe::Game { seed, .. } => *seed,
        }
    }

    /// The same, drawn with `seed`.
    pub fn with_seed(&self, s: u64) -> Recipe {
        let mut r = self.clone();
        match &mut r {
            Recipe::Village { seed, .. } | Recipe::Game { seed, .. } => *seed = s,
        }
        r
    }

    /// A village over rectangle `r`: as many houses as `r` holds at its
    /// density, unless its houses were set by hand.
    pub fn over(&self, r: BlockRect) -> Recipe {
        let mut out = self.clone();
        if let Recipe::Village { centre, size, params, .. } = &mut out {
            *centre = [(r.x0 + r.x1) / 2, (r.z0 + r.z1) / 2];
            *size = [r.x1 - r.x0, r.z1 - r.z0];
            if !params.houses_by_hand {
                params.houses = Some(crate::village::houses_for(r, params.density));
            }
        }
        out
    }

    /// The same, moved by `(dx, dz)`.
    pub fn moved(&self, dx: i32, dz: i32) -> Recipe {
        let mut r = self.clone();
        match &mut r {
            Recipe::Village { centre, .. } => *centre = [centre[0] + dx, centre[1] + dz],
            Recipe::Game { at, .. } => *at = [at[0] + dx, at[1], at[2] + dz],
        }
        r
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recipes_read_back_and_move() {
        let r = Recipe::Village { pack: String::new(), theme: "Plains".into(), centre: [10, -20], size: [160, 120], params: Params { houses: Some(12), ..Params::default() }, seed: 3 };
        assert_eq!(Recipe::from_json(&r.to_json()), Some(r.clone()));
        assert_eq!(r.area(), Some(BlockRect::new(-70, -80, 90, 40)));
        assert_eq!(r.moved(5, 5).area(), Some(BlockRect::new(-65, -75, 95, 45)));
        assert_eq!(r.with_seed(9).seed(), 9);
        let g = Recipe::Game { kind: "stronghold".into(), at: [0, -20, 0], turn: None, seed: 1, flatten: false };
        assert_eq!(Recipe::from_json(&g.to_json()), Some(g));
    }

    /// Resizing a village keeps its density: twice the land, about twice
    /// the houses; a denser one holds more on the same land; houses typed
    /// by hand stay as typed.
    #[test]
    fn houses_follow_the_area_at_the_same_density() {
        let r = Recipe::Village { pack: String::new(), theme: "Plains".into(), centre: [0, 0], size: [192, 192], params: Params { density: 0.6, ..Params::default() }, seed: 1 };
        let houses = |r: &Recipe| match r {
            Recipe::Village { params, .. } => params.houses.unwrap(),
            Recipe::Game { .. } => 0,
        };
        let small = houses(&r.over(BlockRect::new(0, 0, 192, 192)));
        let big = houses(&r.over(BlockRect::new(0, 0, 384, 192)));
        assert!((small * 2).abs_diff(big) <= 1, "{small} houses, then {big} over twice the land");
        let dense = match &r {
            Recipe::Village { pack, theme, centre, size, params, seed } => Recipe::Village { pack: pack.clone(), theme: theme.clone(), centre: *centre, size: *size, params: Params { density: 0.85, ..params.clone() }, seed: *seed },
            g => g.clone(),
        };
        assert!(houses(&dense.over(BlockRect::new(0, 0, 192, 192))) > small, "denser, more houses");
        let typed = match &r {
            Recipe::Village { pack, theme, centre, size, params, seed } => Recipe::Village { pack: pack.clone(), theme: theme.clone(), centre: *centre, size: *size, params: Params { houses: Some(7), houses_by_hand: true, ..params.clone() }, seed: *seed },
            g => g.clone(),
        };
        assert_eq!(houses(&typed.over(BlockRect::new(0, 0, 384, 384))), 7);
    }
}
