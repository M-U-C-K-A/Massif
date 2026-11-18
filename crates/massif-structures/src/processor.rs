//! Processors: what the game does to a template's blocks as it places
//! them — rots some, mosses others, swaps a path for planks over water,
//! hides loot in suspicious gravel.

use crate::ground::{Ground, WorldBlock};
use crate::nbt::{self, Compound};
use crate::rng::Rng;
use fastnbt::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// A block on its way into the world.
#[derive(Debug, Clone, PartialEq)]
pub struct BlockInfo {
    pub pos: [i32; 3],
    pub state: Arc<str>,
    pub nbt: Option<Compound>,
}

/// A set of blocks named by a tag (`#minecraft:features_cannot_replace`).
pub type Tag = Arc<HashSet<String>>;

/// The heightmaps the game's processors and structures read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Heightmap {
    /// The first free block above the topmost block that is not air
    /// (water counts).
    WorldSurface,
    /// The first free block above the topmost solid block.
    OceanFloor,
}

impl Heightmap {
    pub fn parse(name: &str) -> Heightmap {
        if name.starts_with("OCEAN_FLOOR") || name.starts_with("MOTION_BLOCKING") { Heightmap::OceanFloor } else { Heightmap::WorldSurface }
    }

    pub fn height(self, ground: &dyn Ground, x: i32, z: i32) -> i32 {
        match self {
            Heightmap::WorldSurface => ground.surface(x, z),
            Heightmap::OceanFloor => ground.floor(x, z),
        }
    }
}

/// What a rule asks of a block.
#[derive(Debug, Clone)]
pub enum Test {
    Always,
    Block(String),
    State(String),
    Tag(Tag),
    RandomBlock(String, f32),
    RandomState(String, f32),
}

impl Test {
    pub fn test(&self, state: &str, rng: &mut Rng) -> bool {
        match self {
            Test::Always => true,
            Test::Block(b) => nbt::block_name(state) == b,
            Test::State(s) => state == s,
            Test::Tag(t) => t.contains(nbt::block_name(state)),
            Test::RandomBlock(b, p) => nbt::block_name(state) == b && rng.float() < *p,
            Test::RandomState(s, p) => state == s && rng.float() < *p,
        }
    }
}

/// What a rule asks of where a block goes.
#[derive(Debug, Clone)]
pub enum PosTest {
    Always,
    /// More likely the farther from the structure's middle (in blocks,
    /// along every axis, or along one).
    Linear { min_chance: f32, max_chance: f32, min_dist: i32, max_dist: i32, axis: Option<usize> },
}

impl PosTest {
    fn test(&self, world: [i32; 3], center: [i32; 3], rng: &mut Rng) -> bool {
        match *self {
            PosTest::Always => true,
            PosTest::Linear { min_chance, max_chance, min_dist, max_dist, axis } => {
                let d = match axis {
                    Some(a) => (world[a] - center[a]).abs(),
                    None => (0..3).map(|a| (world[a] - center[a]).abs()).sum(),
                } as f32;
                let t = if max_dist == min_dist { 1.0 } else { ((d - min_dist as f32) / (max_dist - min_dist) as f32).clamp(0.0, 1.0) };
                rng.float() <= min_chance + (max_chance - min_chance) * t
            }
        }
    }
}

/// What a rule does to the block's data.
#[derive(Debug, Clone)]
pub enum Modifier {
    Passthrough,
    Clear,
    Append(Compound),
    /// Fills it from a loot table when the player opens (or brushes) it.
    Loot(String),
}

#[derive(Debug, Clone)]
pub struct Rule {
    pub input: Test,
    pub location: Test,
    pub position: PosTest,
    pub output: Arc<str>,
    pub modifier: Modifier,
}

/// A processor.
#[derive(Debug, Clone)]
pub enum Processor {
    /// Leaves these blocks out.
    Ignore(Vec<String>),
    /// Leaves each block out but a share `integrity` of them (of those in
    /// `rottable` only, when given).
    Rot { integrity: f32, rottable: Option<Tag> },
    /// Each block as high above the land as it was above the template's
    /// floor (the land's height plus `offset`): a path that follows the
    /// land.
    Gravity { heightmap: Heightmap, offset: i32 },
    Rule(Vec<Rule>),
    /// Leaves the blocks of the world in the tag as they are.
    Protected(Tag),
    /// What `delegate` would change, on at most `limit` blocks drawn among
    /// those it would.
    Capped { limit: (i32, i32), delegate: Box<Processor> },
    /// A jigsaw block becomes the block it leaves once joined.
    JigsawReplacement,
    /// Blocks of stone become their mossy or cracked kin (ruined portals).
    Age { mossiness: f32 },
    /// Overworld stone becomes the Nether's blackstone.
    Blackstone,
    /// A block that is not whole, where the world has lava, leaves it.
    LavaSubmerged,
    /// One Massif does not know: leaves the blocks as they are.
    Unknown(String),
}

/// What processors read while they work.
pub struct Ctx<'a> {
    pub ground: &'a dyn Ground,
    pub seed: u64,
    /// The middle of the structure's bottom, which some rules measure from.
    pub center: [i32; 3],
}

impl Processor {
    /// The processors the game's list `json` (a `processor_list` file, or
    /// its `processors` array) holds; `tag` finds a tag's blocks.
    pub fn parse_list(json: &serde_json::Value, tag: &dyn Fn(&str) -> Tag) -> Vec<Processor> {
        let list = match json {
            serde_json::Value::Array(a) => a.as_slice(),
            serde_json::Value::Object(o) => o.get("processors").and_then(|p| p.as_array()).map_or(&[][..], |a| a.as_slice()),
            _ => &[],
        };
        list.iter().map(|p| Processor::parse(p, tag)).collect()
    }

    pub fn parse(json: &serde_json::Value, tag: &dyn Fn(&str) -> Tag) -> Processor {
        let kind = json.get("processor_type").and_then(|t| t.as_str()).unwrap_or("");
        let kind = kind.strip_prefix("minecraft:").unwrap_or(kind);
        let f = |k: &str, d: f32| json.get(k).and_then(|v| v.as_f64()).map_or(d, |v| v as f32);
        match kind {
            "block_ignore" => Processor::Ignore(json.get("blocks").and_then(|b| b.as_array()).map(|b| b.iter().filter_map(nbt::state_of_json).collect()).unwrap_or_default()),
            "block_rot" => Processor::Rot { integrity: f("integrity", 1.0), rottable: blocks_or_tag(json.get("rottable_blocks"), tag) },
            "gravity" => Processor::Gravity {
                heightmap: Heightmap::parse(json.get("heightmap").and_then(|h| h.as_str()).unwrap_or("WORLD_SURFACE_WG")),
                offset: json.get("offset").and_then(|o| o.as_i64()).unwrap_or(0) as i32,
            },
            "rule" => Processor::Rule(json.get("rules").and_then(|r| r.as_array()).map(|r| r.iter().filter_map(|r| rule(r, tag)).collect()).unwrap_or_default()),
            "protected_blocks" => Processor::Protected(blocks_or_tag(json.get("value"), tag).unwrap_or_default()),
            "capped" => Processor::Capped {
                limit: int_provider(json.get("limit")),
                delegate: Box::new(json.get("delegate").map_or(Processor::Unknown("capped without a delegate".into()), |d| Processor::parse(d, tag))),
            },
            "jigsaw_replacement" => Processor::JigsawReplacement,
            "block_age" => Processor::Age { mossiness: f("mossiness", 0.5) },
            "blackstone_replace" => Processor::Blackstone,
            "lava_submerged_block" => Processor::LavaSubmerged,
            "nop" => Processor::Ignore(Vec::new()),
            _ => Processor::Unknown(kind.to_owned()),
        }
    }

    /// The names of the processors Massif does not know, in `list`.
    pub fn unknown(list: &[Processor]) -> Vec<String> {
        let mut out = Vec::new();
        for p in list {
            match p {
                Processor::Unknown(k) => out.push(k.clone()),
                Processor::Capped { delegate, .. } => out.extend(Processor::unknown(std::slice::from_ref(delegate))),
                _ => {}
            }
        }
        out
    }

    /// The block `current` as this processor leaves it, `None` to leave it
    /// out. `local` is where it was in the template.
    pub fn process(&self, ctx: &Ctx, local: [i32; 3], current: BlockInfo) -> Option<BlockInfo> {
        match self {
            Processor::Ignore(blocks) => (!blocks.iter().any(|b| matches(b, &current.state))).then_some(current),
            Processor::Rot { integrity, rottable } => {
                if rottable.as_ref().is_some_and(|t| !t.contains(nbt::block_name(&current.state))) {
                    return Some(current);
                }
                let mut rng = Rng::at(ctx.seed ^ 0x0072_6f74, current.pos);
                (rng.float() <= *integrity).then_some(current)
            }
            Processor::Gravity { heightmap, offset } => {
                let [x, _, z] = current.pos;
                let y = heightmap.height(ctx.ground, x, z) + offset + local[1];
                Some(BlockInfo { pos: [x, y, z], ..current })
            }
            Processor::Rule(rules) => {
                let mut rng = Rng::at(ctx.seed ^ 0x7275_6c65, current.pos);
                let here = ctx.ground.block(current.pos[0], current.pos[1], current.pos[2]);
                let here = here.state();
                for r in rules {
                    if r.input.test(&current.state, &mut rng) && r.location.test(here, &mut rng) && r.position.test(current.pos, ctx.center, &mut rng) {
                        let nbt = match &r.modifier {
                            Modifier::Passthrough => current.nbt.clone(),
                            Modifier::Clear => Some(Compound::new()),
                            Modifier::Append(data) => {
                                let mut n = current.nbt.clone().unwrap_or_default();
                                n.extend(data.iter().map(|(k, v)| (k.clone(), v.clone())));
                                Some(n)
                            }
                            Modifier::Loot(table) => {
                                let mut n = current.nbt.clone().unwrap_or_default();
                                n.insert("LootTable".into(), Value::String(table.clone()));
                                n.insert("LootTableSeed".into(), Value::Long(rng.next_u64() as i64));
                                Some(n)
                            }
                        };
                        return Some(BlockInfo { pos: current.pos, state: r.output.clone(), nbt });
                    }
                }
                Some(current)
            }
            Processor::Protected(tag) => match ctx.ground.block(current.pos[0], current.pos[1], current.pos[2]) {
                WorldBlock::Solid(s) if tag.contains(nbt::block_name(&s)) => None,
                _ => Some(current),
            },
            // Capped works on the whole template once its blocks are
            // processed (see [`finalize`]).
            Processor::Capped { .. } => Some(current),
            Processor::JigsawReplacement => {
                if nbt::block_name(&current.state) != "minecraft:jigsaw" {
                    return Some(current);
                }
                let final_state = current.nbt.as_ref().and_then(|n| nbt::string(n, "final_state")).unwrap_or("minecraft:air");
                let state = nbt::state_of_text(final_state);
                (nbt::block_name(&state) != "minecraft:structure_void").then(|| BlockInfo { pos: current.pos, state: state.into(), nbt: None })
            }
            Processor::Age { mossiness } => {
                let mut rng = Rng::at(ctx.seed ^ 0x0061_6765, current.pos);
                let name = nbt::block_name(&current.state);
                let out = match name {
                    "minecraft:stone_bricks" | "minecraft:stone" | "minecraft:chiseled_stone_bricks" if rng.float() < 0.5 => {
                        let pick = ["minecraft:cracked_stone_bricks", "minecraft:mossy_stone_bricks", "minecraft:stone_bricks"];
                        Some(if rng.float() < *mossiness { pick[1] } else { pick[rng.below(3) as usize] }.to_owned())
                    }
                    "minecraft:stone_brick_stairs" if rng.float() < *mossiness * 0.5 => Some(current.state.replace("minecraft:stone_brick_stairs", "minecraft:mossy_stone_brick_stairs")),
                    "minecraft:stone_brick_slab" if rng.float() < *mossiness * 0.5 => Some(current.state.replace("minecraft:stone_brick_slab", "minecraft:mossy_stone_brick_slab")),
                    "minecraft:obsidian" if rng.float() < 0.15 => Some("minecraft:crying_obsidian".to_owned()),
                    _ => None,
                };
                Some(match out {
                    Some(s) => BlockInfo { state: s.into(), ..current },
                    None => current,
                })
            }
            Processor::Blackstone => {
                let name = nbt::block_name(&current.state);
                let to = match name {
                    "minecraft:cobblestone" => "minecraft:blackstone",
                    "minecraft:mossy_cobblestone" => "minecraft:blackstone",
                    "minecraft:stone" => "minecraft:polished_blackstone",
                    "minecraft:stone_bricks" => "minecraft:polished_blackstone_bricks",
                    "minecraft:mossy_stone_bricks" => "minecraft:polished_blackstone_bricks",
                    "minecraft:cracked_stone_bricks" => "minecraft:cracked_polished_blackstone_bricks",
                    "minecraft:chiseled_stone_bricks" => "minecraft:chiseled_polished_blackstone",
                    "minecraft:stone_brick_stairs" | "minecraft:mossy_stone_brick_stairs" | "minecraft:cobblestone_stairs" => "minecraft:polished_blackstone_brick_stairs",
                    "minecraft:stone_brick_slab" | "minecraft:mossy_stone_brick_slab" | "minecraft:cobblestone_slab" => "minecraft:polished_blackstone_brick_slab",
                    "minecraft:stone_brick_wall" | "minecraft:mossy_stone_brick_wall" | "minecraft:cobblestone_wall" => "minecraft:polished_blackstone_brick_wall",
                    "minecraft:iron_bars" => "minecraft:chain",
                    _ => return Some(current),
                };
                let state = current.state.replacen(name, to, 1);
                Some(BlockInfo { state: state.into(), ..current })
            }
            Processor::LavaSubmerged => {
                let lava = matches!(ctx.ground.block(current.pos[0], current.pos[1], current.pos[2]), WorldBlock::Lava);
                if lava && !is_whole(&current.state) {
                    Some(BlockInfo { state: "minecraft:lava[level=0]".into(), nbt: None, ..current })
                } else {
                    Some(current)
                }
            }
            Processor::Unknown(_) => Some(current),
        }
    }
}

/// What the processors of `list` that work on the whole template do, once
/// every block went through [`Processor::process`]: `processed[i]` came
/// from the template's block whose place there was `locals[i]`.
pub fn finalize(list: &[Processor], ctx: &Ctx, piece: [i32; 3], locals: &[[i32; 3]], processed: &mut [BlockInfo]) {
    for p in list {
        let Processor::Capped { limit, delegate } = p else { continue };
        if processed.is_empty() || limit.1 <= 0 {
            continue;
        }
        let mut rng = Rng::at(ctx.seed ^ 0x6361_7070, piece);
        let cap = (rng.between(limit.0, limit.1).max(0) as usize).min(processed.len());
        let mut order: Vec<usize> = (0..processed.len()).collect();
        rng.shuffle(&mut order);
        let mut done = 0;
        for i in order {
            if done >= cap {
                break;
            }
            if let Some(out) = delegate.process(ctx, locals[i], processed[i].clone())
                && out != processed[i]
            {
                processed[i] = out;
                done += 1;
            }
        }
    }
}

/// Whether `pattern` (a block, or a block state) names `state`.
fn matches(pattern: &str, state: &str) -> bool {
    if pattern.contains('[') { pattern == state } else { pattern == nbt::block_name(state) }
}

/// Whether a block fills its whole cube: a guess from its name, for the
/// blocks structures leave in lava.
fn is_whole(state: &str) -> bool {
    let name = nbt::block_name(state);
    !["_stairs", "_slab", "_wall", "_fence", "_bars", "chain", "_button", "lantern", "torch", "_door", "_trapdoor", "_pane", "_carpet", "_sign", "ladder", "vine"].iter().any(|w| name.contains(w))
}

/// A list of blocks (`[…]`), a tag (`#minecraft:…`) or a block, as a set
/// of block names.
fn blocks_or_tag(v: Option<&serde_json::Value>, tag: &dyn Fn(&str) -> Tag) -> Option<Tag> {
    match v? {
        serde_json::Value::String(s) if s.starts_with('#') => Some(tag(s)),
        serde_json::Value::String(s) => Some(Arc::new(std::iter::once(crate::source::full_id(s)).collect())),
        serde_json::Value::Array(a) => Some(Arc::new(a.iter().filter_map(|b| b.as_str()).map(crate::source::full_id).collect())),
        _ => None,
    }
}

/// A whole number the game draws (`3`, or `{"type": "uniform", …}`), as
/// its least and greatest.
pub fn int_provider(v: Option<&serde_json::Value>) -> (i32, i32) {
    match v {
        Some(serde_json::Value::Number(n)) => {
            let n = n.as_i64().unwrap_or(0) as i32;
            (n, n)
        }
        Some(serde_json::Value::Object(o)) => {
            let lo = o.get("min_inclusive").or_else(|| o.get("value").and_then(|v| v.get("min_inclusive"))).and_then(|v| v.as_i64());
            let hi = o.get("max_inclusive").or_else(|| o.get("value").and_then(|v| v.get("max_inclusive"))).and_then(|v| v.as_i64());
            match (lo, hi, o.get("value").and_then(|v| v.as_i64())) {
                (Some(lo), Some(hi), _) => (lo as i32, hi as i32),
                (_, _, Some(v)) => (v as i32, v as i32),
                _ => (0, 0),
            }
        }
        _ => (0, 0),
    }
}

fn test(v: Option<&serde_json::Value>, tag: &dyn Fn(&str) -> Tag) -> Test {
    let Some(v) = v else { return Test::Always };
    let kind = v.get("predicate_type").and_then(|t| t.as_str()).unwrap_or("");
    let p = v.get("probability").and_then(|p| p.as_f64()).unwrap_or(1.0) as f32;
    let block = || v.get("block").and_then(|b| b.as_str()).map(crate::source::full_id).unwrap_or_default();
    let state = || v.get("block_state").and_then(nbt::state_of_json).unwrap_or_default();
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "block_match" => Test::Block(block()),
        "blockstate_match" => Test::State(state()),
        "tag_match" => Test::Tag(tag(v.get("tag").and_then(|t| t.as_str()).unwrap_or(""))),
        "random_block_match" => Test::RandomBlock(block(), p),
        "random_blockstate_match" => Test::RandomState(state(), p),
        _ => Test::Always,
    }
}

fn rule(v: &serde_json::Value, tag: &dyn Fn(&str) -> Tag) -> Option<Rule> {
    let output: Arc<str> = nbt::state_of_json(v.get("output_state")?)?.into();
    let position = match v.get("position_predicate") {
        Some(p) => {
            let kind = p.get("predicate_type").and_then(|t| t.as_str()).unwrap_or("");
            let g = |k: &str| p.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0);
            let axis = p.get("axis").and_then(|a| a.as_str()).map(|a| match a {
                "x" => 0,
                "y" => 1,
                _ => 2,
            });
            match kind.strip_prefix("minecraft:").unwrap_or(kind) {
                "linear_pos" | "axis_aligned_linear_pos" => PosTest::Linear {
                    min_chance: g("min_chance") as f32,
                    max_chance: g("max_chance") as f32,
                    min_dist: g("min_dist") as i32,
                    max_dist: g("max_dist") as i32,
                    axis: if kind.ends_with("axis_aligned_linear_pos") { Some(axis.unwrap_or(1)) } else { None },
                },
                _ => PosTest::Always,
            }
        }
        None => PosTest::Always,
    };
    let modifier = match v.get("block_entity_modifier") {
        Some(m) => {
            let kind = m.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match kind.strip_prefix("minecraft:").unwrap_or(kind) {
                "append_loot" => Modifier::Loot(m.get("loot_table").and_then(|t| t.as_str()).map(crate::source::full_id).unwrap_or_default()),
                "clear" => Modifier::Clear,
                "append_static" => Modifier::Append(json_compound(m.get("data"))),
                _ => Modifier::Passthrough,
            }
        }
        // Before 1.20 a rule could give the output its NBT outright.
        None => match v.get("output_nbt") {
            Some(n) => Modifier::Append(json_compound(Some(n))),
            None => Modifier::Passthrough,
        },
    };
    Some(Rule { input: test(v.get("input_predicate"), tag), location: test(v.get("location_predicate"), tag), position, output, modifier })
}

/// NBT given as JSON in a data file.
fn json_compound(v: Option<&serde_json::Value>) -> Compound {
    fn value(v: &serde_json::Value) -> Value {
        match v {
            serde_json::Value::Bool(b) => Value::Byte(*b as i8),
            serde_json::Value::Number(n) => match n.as_i64() {
                Some(i) if i32::try_from(i).is_ok() => Value::Int(i as i32),
                Some(i) => Value::Long(i),
                None => Value::Double(n.as_f64().unwrap_or(0.0)),
            },
            serde_json::Value::String(s) => Value::String(s.clone()),
            serde_json::Value::Array(a) => Value::List(a.iter().map(value).collect()),
            serde_json::Value::Object(o) => Value::Compound(o.iter().map(|(k, v)| (k.clone(), value(v))).collect()),
            serde_json::Value::Null => Value::Byte(0),
        }
    }
    match v {
        Some(serde_json::Value::Object(o)) => o.iter().map(|(k, v)| (k.clone(), value(v))).collect(),
        _ => Compound::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;

    fn no_tags(_: &str) -> Tag {
        Arc::new(["minecraft:bedrock".to_owned()].into_iter().collect())
    }

    fn info(pos: [i32; 3], state: &str) -> BlockInfo {
        BlockInfo { pos, state: state.into(), nbt: None }
    }

    #[test]
    fn rules_rot_and_gravity() {
        let ground = Flat { height: 64, water: None };
        let ctx = Ctx { ground: &ground, seed: 9, center: [0, 0, 0] };
        let json: serde_json::Value = serde_json::from_str(
            r##"{"processors": [
                {"processor_type": "minecraft:rule", "rules": [
                    {"input_predicate": {"predicate_type": "minecraft:random_block_match", "block": "minecraft:stone_bricks", "probability": 0.3},
                     "location_predicate": {"predicate_type": "minecraft:always_true"},
                     "output_state": {"Name": "minecraft:cracked_stone_bricks"}},
                    {"input_predicate": {"predicate_type": "minecraft:block_match", "block": "minecraft:chest"},
                     "location_predicate": {"predicate_type": "minecraft:always_true"},
                     "output_state": {"Name": "minecraft:chest", "Properties": {"facing": "north"}},
                     "block_entity_modifier": {"type": "minecraft:append_loot", "loot_table": "minecraft:chests/ancient_city"}}
                ]},
                {"processor_type": "minecraft:block_rot", "integrity": 0.5},
                {"processor_type": "minecraft:protected_blocks", "value": "#minecraft:features_cannot_replace"},
                {"processor_type": "minecraft:something_new"}
            ]}"##,
        )
        .unwrap();
        let list = Processor::parse_list(&json, &no_tags);
        assert_eq!(list.len(), 4);
        assert_eq!(Processor::unknown(&list), vec!["something_new".to_owned()]);
        // About 30 % cracked.
        let cracked = (0..1000).filter(|&i| list[0].process(&ctx, [0; 3], info([i, 64, 0], "minecraft:stone_bricks")).unwrap().state.contains("cracked")).count();
        assert!((230..370).contains(&cracked), "{cracked}");
        let chest = list[0].process(&ctx, [0; 3], info([0, 64, 0], "minecraft:chest[facing=south]")).unwrap();
        assert_eq!(&*chest.state, "minecraft:chest[facing=north]");
        assert_eq!(chest.nbt.as_ref().and_then(|n| nbt::string(n, "LootTable")), Some("minecraft:chests/ancient_city"));
        // Half kept, the same each time.
        let kept: Vec<bool> = (0..1000).map(|i| list[1].process(&ctx, [0; 3], info([i, 64, 0], "minecraft:stone")).is_some()).collect();
        let n = kept.iter().filter(|&&k| k).count();
        assert!((420..580).contains(&n), "{n}");
        assert_eq!(kept, (0..1000).map(|i| list[1].process(&ctx, [0; 3], info([i, 64, 0], "minecraft:stone")).is_some()).collect::<Vec<_>>());
        // The land stays where it is bedrock (here: nowhere).
        assert!(list[2].process(&ctx, [0; 3], info([0, 10, 0], "minecraft:stone")).is_some());
        // A path's block one below the land's surface, wherever it was.
        let gravity = Processor::Gravity { heightmap: Heightmap::WorldSurface, offset: -1 };
        assert_eq!(gravity.process(&ctx, [3, 0, 1], info([3, 90, 1], "minecraft:dirt_path")).unwrap().pos, [3, 63, 1]);
        assert_eq!(gravity.process(&ctx, [3, 2, 1], info([3, 90, 1], "minecraft:lantern")).unwrap().pos, [3, 65, 1]);
        // A jigsaw becomes what it leaves; a structure void, nothing.
        let mut j = info([0, 0, 0], "minecraft:jigsaw[orientation=up_north]");
        j.nbt = Some([("final_state".to_owned(), Value::String("minecraft:polished_deepslate".into()))].into_iter().collect());
        assert_eq!(&*Processor::JigsawReplacement.process(&ctx, [0; 3], j.clone()).unwrap().state, "minecraft:polished_deepslate");
        j.nbt = Some([("final_state".to_owned(), Value::String("minecraft:structure_void".into()))].into_iter().collect());
        assert!(Processor::JigsawReplacement.process(&ctx, [0; 3], j).is_none());
    }

    #[test]
    fn capped_changes_at_most_its_limit() {
        let ground = Flat { height: 64, water: None };
        let ctx = Ctx { ground: &ground, seed: 2, center: [0, 0, 0] };
        let json: serde_json::Value = serde_json::from_str(
            r#"{"processor_type": "minecraft:capped", "limit": 4, "delegate": {"processor_type": "minecraft:rule", "rules": [
                {"input_predicate": {"predicate_type": "minecraft:block_match", "block": "minecraft:gravel"},
                 "location_predicate": {"predicate_type": "minecraft:always_true"},
                 "output_state": {"Name": "minecraft:suspicious_gravel", "Properties": {"dusted": "0"}},
                 "block_entity_modifier": {"type": "minecraft:append_loot", "loot_table": "minecraft:archaeology/trail_ruins_common"}}]}}"#,
        )
        .unwrap();
        let capped = Processor::parse(&json, &no_tags);
        let mut blocks: Vec<BlockInfo> = (0..30).map(|i| info([i, 60, 0], if i % 3 == 0 { "minecraft:gravel" } else { "minecraft:mud_bricks" })).collect();
        let locals: Vec<[i32; 3]> = (0..30).map(|i| [i, 0, 0]).collect();
        finalize(std::slice::from_ref(&capped), &ctx, [0, 60, 0], &locals, &mut blocks);
        let sus: Vec<&BlockInfo> = blocks.iter().filter(|b| b.state.starts_with("minecraft:suspicious_gravel")).collect();
        assert_eq!(sus.len(), 4);
        assert!(sus.iter().all(|b| b.nbt.as_ref().is_some_and(|n| n.contains_key("LootTable"))));
    }
}
