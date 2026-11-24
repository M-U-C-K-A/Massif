//! Combined layers, as WorldPainter's: one layer that paints several at
//! once — a terrain, a biome, a ground cover, a tree layer and one under
//! the trees, a tree size, underground pockets — each over its share of
//! the columns the brush paints. A combined layer has no blocks of its
//! own: each of its parts is written into its slot of the blocks, as that
//! layer's own tool writes it, so a block still holds one layer of each
//! slot, and editing a combined layer changes what it paints next, not
//! what it painted.

use crate::registry::{Registry, TerrainId};
use serde::{Deserialize, Serialize};

/// A slot of the blocks a combined layer paints into.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Part {
    Terrain,
    Biome,
    Cover,
    Trees,
    /// The sub-canopy, under the trees.
    Under,
    Size,
    Pockets,
}

impl Part {
    /// Every part, in the order the editor shows them and the brush paints
    /// them.
    pub const ALL: [Part; 7] = [Part::Terrain, Part::Biome, Part::Cover, Part::Trees, Part::Under, Part::Size, Part::Pockets];
}

/// One layer a combined layer paints: of slot `part`, `layer` by its
/// number there (a terrain's or a biome's id, a cover's, an object
/// layer's or a pocket layer's number, a size from 1 to 4), over `share`
/// of the columns painted (0 to 1).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Component {
    pub part: Part,
    pub layer: u16,
    pub share: f32,
}

/// A combined layer: its name, its colour in the lists, and what it
/// paints, one layer of each slot it names.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CombinedDef {
    pub name: String,
    pub color: [u8; 3],
    /// In the order of [`Part::ALL`], one of each part at most.
    #[serde(default)]
    pub parts: Vec<Component>,
    /// Removed: no longer offered.
    #[serde(default)]
    pub removed: bool,
    /// Its place among the combined layers in the Layers panel.
    #[serde(default, skip_serializing_if = "crate::registry::is_zero")]
    pub order: u32,
    /// The palette it is sorted into in the Layers panel ("": none, it
    /// stays in its section).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub palette: String,
}

impl CombinedDef {
    /// A combined layer that paints nothing yet.
    pub fn new(name: &str, color: [u8; 3]) -> Self {
        Self { name: name.to_owned(), color, parts: Vec::new(), removed: false, order: 0, palette: String::new() }
    }

    /// The layer it paints in slot `part`, and over what share.
    pub fn get(&self, part: Part) -> Option<(u16, f32)> {
        self.parts.iter().find(|c| c.part == part).map(|c| (c.layer, c.share))
    }

    /// Paint `layer` over `share` in slot `part`, or (`None`) nothing there.
    pub fn set(&mut self, part: Part, what: Option<(u16, f32)>) {
        self.parts.retain(|c| c.part != part);
        if let Some((layer, share)) = what {
            self.parts.push(Component { part, layer, share: share.clamp(0.0, 1.0) });
            self.parts.sort_by_key(|c| Part::ALL.iter().position(|p| *p == c.part));
        }
    }

    /// What it paints now in a world of registry `reg`: its parts whose
    /// layer is there still, a tree layer's in the slot the layer is in
    /// (base or sub-canopy) whatever part named it.
    pub fn to_paint(&self, reg: &Registry) -> Vec<Component> {
        let mut out: Vec<Component> = Vec::new();
        for c in &self.parts {
            let part = match c.part {
                Part::Terrain if (c.layer as usize) < reg.terrains.len() => Part::Terrain,
                Part::Biome if (c.layer as usize) < reg.biomes.len() => Part::Biome,
                Part::Cover if reg.covers.get((c.layer as usize).wrapping_sub(1)).is_some_and(|d| !d.removed) => Part::Cover,
                Part::Trees | Part::Under => match reg.object_layer(c.layer) {
                    Some(l) if l.understory => Part::Under,
                    Some(_) => Part::Trees,
                    None => continue,
                },
                Part::Size if (1..=crate::tile::SIZE_LARGE).contains(&c.layer) => Part::Size,
                Part::Pockets if reg.pocket(c.layer).is_some() => Part::Pockets,
                _ => continue,
            };
            if !out.iter().any(|o| o.part == part) {
                out.push(Component { part, ..*c });
            }
        }
        out
    }

    /// The parts that rub out what it painted: each slot it names emptied
    /// (the biome back to automatic) but the terrain, which the brush puts
    /// back as Paint terrain's right button does.
    pub fn to_rub_out(&self, reg: &Registry) -> Vec<Component> {
        self.to_paint(reg).into_iter().filter(|c| c.part != Part::Terrain).map(|c| Component { layer: 0, share: 1.0, ..c }).collect()
    }

    /// The terrain it paints, if any.
    pub fn terrain(&self) -> Option<TerrainId> {
        self.get(Part::Terrain).map(|(t, _)| TerrainId(t))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One layer a slot, in the editor's order; a part set again takes the
    /// place of the one there.
    #[test]
    fn one_layer_a_slot() {
        let mut def = CombinedDef::new("Forest", [40, 120, 40]);
        def.set(Part::Cover, Some((3, 0.5)));
        def.set(Part::Terrain, Some((2, 1.0)));
        def.set(Part::Cover, Some((4, 1.5)));
        assert_eq!(def.parts, [Component { part: Part::Terrain, layer: 2, share: 1.0 }, Component { part: Part::Cover, layer: 4, share: 1.0 }]);
        def.set(Part::Terrain, None);
        assert_eq!(def.get(Part::Terrain), None);
        assert_eq!(def.get(Part::Cover), Some((4, 1.0)));
    }

    /// What it paints follows the registry: a removed layer is left out, a
    /// tree layer goes to the slot it is in now.
    #[test]
    fn it_paints_the_layers_that_are_there() {
        let mut reg = Registry::vanilla();
        let mut hazel = crate::objects::ObjectLayerDef::new("Hazel", &[], &reg);
        hazel.understory = true;
        let hazel = reg.add_object_layer(hazel);
        let mut def = CombinedDef::new("Forest", [40, 120, 40]);
        def.set(Part::Cover, Some((1, 0.5)));
        def.set(Part::Trees, Some((hazel, 1.0)));
        def.set(Part::Size, Some((9, 1.0)));
        def.set(Part::Pockets, Some((1, 1.0)));
        let parts = def.to_paint(&reg);
        assert_eq!(parts.iter().map(|c| (c.part, c.layer)).collect::<Vec<_>>(), [(Part::Cover, 1), (Part::Under, hazel)], "no size 9, no pocket layer");
        reg.covers[0].removed = true;
        assert_eq!(def.to_paint(&reg).len(), 1);
        assert_eq!(def.to_rub_out(&reg), [Component { part: Part::Under, layer: 0, share: 1.0 }]);
    }
}
