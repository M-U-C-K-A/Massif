//! Structure templates: the `.nbt` files a structure block saves, which the
//! game's structures are built from.

use crate::geom::{BBox, Dir, place};
use crate::nbt::{self, Compound};
use fastnbt::Value;

/// A block of a template.
#[derive(Debug, Clone)]
pub struct TBlock {
    pub pos: [i32; 3],
    /// Its state, an index into each palette.
    pub state: u32,
    /// Its block entity's data (a chest's loot table, a jigsaw's target).
    pub nbt: Option<Compound>,
}

/// An entity of a template.
#[derive(Debug, Clone)]
pub struct TEntity {
    /// Where it stands, in blocks from the template's corner.
    pub pos: [f64; 3],
    pub nbt: Compound,
}

/// How a jigsaw block lets the pieces it joins turn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Joint {
    /// Turned any way about the axis it faces along.
    Rollable,
    /// Their tops facing the same way.
    Aligned,
}

/// A jigsaw block: where a piece joins another.
#[derive(Debug, Clone)]
pub struct Jigsaw {
    pub pos: [i32; 3],
    /// The side it faces (where the joined piece goes) and the side its
    /// top faces.
    pub front: Dir,
    pub top: Dir,
    /// Its own name, the name of the jigsaw it joins, and the pool that
    /// piece is drawn from.
    pub name: String,
    pub target: String,
    pub pool: String,
    pub joint: Joint,
    /// The block it leaves once joined.
    pub final_state: String,
    /// Which jigsaws are joined first: the higher the sooner.
    pub placement_priority: i32,
    pub selection_priority: i32,
}

impl Jigsaw {
    /// As it stands in a template moved to `origin` and turned `turn`
    /// quarter turns.
    pub fn placed(&self, origin: [i32; 3], turn: u8) -> Jigsaw {
        Jigsaw { pos: place(self.pos, origin, turn), front: self.front.turned(turn), top: self.top.turned(turn), ..self.clone() }
    }

    /// Whether `other` can join this one: facing each other, their tops
    /// alike unless the joint lets them roll, and `other` named as this
    /// one's target.
    pub fn can_attach(&self, other: &Jigsaw) -> bool {
        self.front == other.front.opposite() && (self.joint == Joint::Rollable || self.top == other.top) && crate::source::full_id(&self.target) == crate::source::full_id(&other.name)
    }

    /// The block in front of it: where the piece it joins has its jigsaw.
    pub fn attached(&self) -> [i32; 3] {
        let s = self.front.step();
        [self.pos[0] + s[0], self.pos[1] + s[1], self.pos[2] + s[2]]
    }
}

/// A template.
#[derive(Debug, Clone)]
pub struct Template {
    pub size: [i32; 3],
    /// One palette, or several of the same blocks in other materials (the
    /// game draws one for each shipwreck).
    pub palettes: Vec<Vec<String>>,
    /// Sorted by height, then x, then z.
    pub blocks: Vec<TBlock>,
    pub entities: Vec<TEntity>,
    pub jigsaws: Vec<Jigsaw>,
    pub data_version: i32,
}

/// The largest template read: the game's are at most 48 blocks a side.
const MAX_SIDE: i32 = 512;

impl Template {
    pub fn parse(bytes: &[u8]) -> Result<Template, String> {
        let root = match massif_objects::parse_nbt(bytes).map_err(|e| e.to_string())? {
            Value::Compound(c) => c,
            _ => return Err("no compound at the root".into()),
        };
        Self::from_nbt(&root)
    }

    pub fn from_nbt(root: &Compound) -> Result<Template, String> {
        let size = nbt::triple(root.get("size")).ok_or("no size")?;
        if size.iter().any(|&s| !(0..=MAX_SIDE).contains(&s)) {
            return Err(format!("a size of {size:?}"));
        }
        let palettes: Vec<Vec<String>> = match root.get("palettes") {
            Some(Value::List(ps)) if !ps.is_empty() => ps
                .iter()
                .map(|p| match p {
                    Value::List(p) => p.iter().map(nbt::state_of).collect(),
                    _ => Vec::new(),
                })
                .collect(),
            _ => vec![nbt::list(root, "palette").iter().map(nbt::state_of).collect()],
        };
        let n = palettes[0].len();
        let mut blocks = Vec::new();
        let mut jigsaws = Vec::new();
        for b in nbt::list(root, "blocks") {
            let Value::Compound(b) = b else { continue };
            let (Some(pos), Some(state)) = (nbt::triple(b.get("pos")), nbt::int(b.get("state"))) else { continue };
            if state < 0 || state as usize >= n {
                continue;
            }
            let data = nbt::compound(b, "nbt").cloned();
            let name = nbt::block_name(&palettes[0][state as usize]);
            if name == "minecraft:jigsaw"
                && let Some(j) = jigsaw(pos, &palettes[0][state as usize], data.as_ref())
            {
                jigsaws.push(j);
            }
            blocks.push(TBlock { pos, state: state as u32, nbt: data });
        }
        blocks.sort_by_key(|b| (b.pos[1], b.pos[0], b.pos[2]));
        let entities = nbt::list(root, "entities")
            .iter()
            .filter_map(|e| {
                let Value::Compound(e) = e else { return None };
                let pos = nbt::triple_f(e.get("pos"))?;
                let data = nbt::compound(e, "nbt")?.clone();
                Some(TEntity { pos, nbt: data })
            })
            .collect();
        Ok(Template {
            size,
            palettes,
            blocks,
            entities,
            jigsaws,
            data_version: nbt::int(root.get("DataVersion")).unwrap_or(0) as i32,
        })
    }

    /// Its box moved to `origin` and turned.
    pub fn bbox(&self, origin: [i32; 3], turn: u8) -> BBox {
        BBox::of_template(self.size, origin, turn)
    }

    /// Its jigsaws as they stand moved to `origin` and turned, in the
    /// template's order.
    pub fn jigsaws_at(&self, origin: [i32; 3], turn: u8) -> Vec<Jigsaw> {
        self.jigsaws.iter().map(|j| j.placed(origin, turn)).collect()
    }
}

/// A jigsaw block's data: its orientation from its state (`orientation`,
/// or `facing` before 1.16), its names and pool from its block entity.
fn jigsaw(pos: [i32; 3], state: &str, data: Option<&Compound>) -> Option<Jigsaw> {
    let (front, top) = match nbt::property(state, "orientation") {
        Some(o) => {
            let (f, t) = o.split_once('_')?;
            (Dir::parse(f)?, Dir::parse(t)?)
        }
        None => {
            let f = Dir::parse(nbt::property(state, "facing")?)?;
            (f, if f.is_horizontal() { Dir::Up } else { Dir::North })
        }
    };
    let data = data?;
    let text = |k: &str| nbt::string(data, k).unwrap_or("").to_owned();
    let joint = match nbt::string(data, "joint") {
        Some("rollable") => Joint::Rollable,
        Some("aligned") => Joint::Aligned,
        _ if front.is_horizontal() => Joint::Aligned,
        _ => Joint::Rollable,
    };
    // Before 1.16 the name was `attachement_type` and the target the same.
    let name = nbt::string(data, "name").or_else(|| nbt::string(data, "attachement_type")).unwrap_or("minecraft:empty").to_owned();
    let target = nbt::string(data, "target").map_or_else(|| name.clone(), str::to_owned);
    Some(Jigsaw {
        pos,
        front,
        top,
        name,
        target,
        pool: nbt::string(data, "pool").or_else(|| nbt::string(data, "target_pool")).unwrap_or("minecraft:empty").to_owned(),
        joint,
        final_state: { let f = text("final_state"); if f.is_empty() { "minecraft:air".to_owned() } else { f } },
        placement_priority: nbt::int(data.get("placement_priority")).unwrap_or(0) as i32,
        selection_priority: nbt::int(data.get("selection_priority")).unwrap_or(0) as i32,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::testing::{jig, template_bytes};

    #[test]
    fn templates_and_their_jigsaws_are_read() {
        let bytes = template_bytes(
            [3, 2, 4],
            &[
                ([0, 0, 0], "minecraft:stone", None),
                ([1, 0, 3], "minecraft:jigsaw[orientation=south_up]", jig("a:door", "a:street", "a:streets", "minecraft:oak_planks")),
                ([2, 1, 1], "minecraft:chest[facing=north,type=single,waterlogged=false]", None),
            ],
            &[([1.5, 1.0, 1.5], Compound::new())],
        );
        let t = Template::parse(&bytes).unwrap();
        assert_eq!(t.size, [3, 2, 4]);
        assert_eq!(t.blocks.len(), 3);
        assert_eq!(t.entities.len(), 1);
        let j = &t.jigsaws[0];
        assert_eq!((j.front, j.top, j.joint), (Dir::South, Dir::Up, Joint::Aligned));
        assert_eq!((j.name.as_str(), j.target.as_str(), j.pool.as_str(), j.final_state.as_str()), ("a:door", "a:street", "a:streets", "minecraft:oak_planks"));
        // Turned a quarter, the jigsaw on the south side faces west.
        let p = j.placed([0, 64, 0], 1);
        assert_eq!((p.pos, p.front), ([-3, 64, 1], Dir::West));
        assert_eq!(p.attached(), [-4, 64, 1]);
        let other = Jigsaw { front: Dir::East, name: "a:street".into(), target: "a:door".into(), ..p.clone() };
        assert!(p.can_attach(&other));
        assert!(!p.can_attach(&Jigsaw { front: Dir::West, ..other.clone() }));
        assert!(!p.can_attach(&Jigsaw { name: "a:road".into(), ..other }));
        assert!(Template::parse(b"nonsense").is_err());
    }
}
