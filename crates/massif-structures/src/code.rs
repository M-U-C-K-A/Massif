//! The game's structures built by its code rather than from templates: the
//! desert and jungle temples, the witch hut, the stronghold. Massif builds
//! them with code of its own after the game's.

use crate::ground::Ground;
use crate::rng::Rng;
use crate::vanilla::Request;
use massif_core::structures::Structure;
use std::collections::HashMap;

pub fn build(_ground: &dyn Ground, req: &Request, _rng: &mut Rng) -> Result<(Structure, HashMap<(i32, i32), i32>), String> {
    Err(format!("{} is not built yet", req.kind.name))
}
