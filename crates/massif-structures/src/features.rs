//! The game's features that structures place among their pieces: the trees
//! and piles of hay of a village, the patches of sculk of an Ancient City
//! (whose shriekers call the warden). Read from the game's own feature
//! files, grown the way the game grows them, simplified.

use crate::build::{Builder, is_air};
use crate::library::Library;
use crate::nbt;
use crate::processor::int_provider;
use crate::rng::Rng;
use crate::source::Kind;
use serde_json::Value;

/// Grow feature `id` (a placed feature, as pools name them, or a
/// configured one) at `pos`. Whether Massif knows how.
pub fn place(lib: &Library, b: &mut Builder, id: &str, pos: [i32; 3], rng: &mut Rng) -> bool {
    match lib.json(Kind::PlacedFeature, id).or_else(|| lib.json(Kind::Feature, id)) {
        Some(v) => run_placed(lib, b, &v, pos, rng),
        None => false,
    }
}

/// A placed feature (`{"feature": …, "placement": […]}`), or a feature
/// alone, grown at `pos`: where its placement puts it.
fn run_placed(lib: &Library, b: &mut Builder, v: &Value, pos: [i32; 3], rng: &mut Rng) -> bool {
    let (feature, placement) = match v.get("feature") {
        Some(Value::String(id)) => (lib.json(Kind::Feature, id), v.get("placement")),
        Some(inline @ Value::Object(_)) => (Some(inline.clone()), v.get("placement")),
        _ => (Some(v.clone()), None),
    };
    let Some(feature) = feature else { return false };
    let mut known = true;
    for p in placements(lib, b, placement, pos, rng) {
        known &= configured(lib, b, &feature, p, rng);
    }
    known
}

/// Where a placed feature's placement modifiers put it, from `pos`: the
/// common ones (count, chance, offsets, block predicates); the others leave
/// the place as it is.
fn placements(lib: &Library, b: &Builder, list: Option<&Value>, pos: [i32; 3], rng: &mut Rng) -> Vec<[i32; 3]> {
    let mut out = vec![pos];
    for m in list.and_then(|l| l.as_array()).into_iter().flatten() {
        let kind = m.get("type").and_then(|t| t.as_str()).unwrap_or("");
        let sample = |v: Option<&Value>, rng: &mut Rng| {
            let (lo, hi) = int_provider(v);
            rng.between(lo, hi)
        };
        out = match kind.strip_prefix("minecraft:").unwrap_or(kind) {
            "count" => out.iter().flat_map(|&p| std::iter::repeat_n(p, sample(m.get("count"), rng).clamp(0, 256) as usize)).collect(),
            "random_chance" => {
                let c = m.get("chance").and_then(|c| c.as_f64()).unwrap_or(1.0) as f32;
                out.into_iter().filter(|_| rng.float() < c).collect()
            }
            "rarity_filter" => {
                let c = m.get("chance").and_then(|c| c.as_u64()).unwrap_or(1).max(1) as u32;
                out.into_iter().filter(|_| rng.below(c) == 0).collect()
            }
            "offset" | "random_offset" => {
                let (kx, ky, kz) = if kind.ends_with("random_offset") { ("xz_spread", "y_spread", "xz_spread") } else { ("x", "y", "z") };
                out.into_iter().map(|p| [p[0] + sample(m.get(kx), rng), p[1] + sample(m.get(ky), rng), p[2] + sample(m.get(kz), rng)]).collect()
            }
            "block_predicate_filter" => out.into_iter().filter(|&p| test_block(lib, b, m.get("predicate"), p)).collect(),
            _ => out,
        };
    }
    out
}

/// A block predicate of the game's, asked at `p`.
fn test_block(lib: &Library, b: &Builder, v: Option<&Value>, p: [i32; 3]) -> bool {
    let Some(v) = v else { return true };
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let at = |v: &Value| -> [i32; 3] {
        let o = v.get("offset").and_then(|o| o.as_array()).map(|a| a.iter().map(|x| x.as_i64().unwrap_or(0) as i32).collect::<Vec<_>>()).unwrap_or_default();
        [p[0] + o.first().copied().unwrap_or(0), p[1] + o.get(1).copied().unwrap_or(0), p[2] + o.get(2).copied().unwrap_or(0)]
    };
    let all = |key: &str| v.get(key).and_then(|l| l.as_array()).map(|l| l.iter().map(|x| test_block(lib, b, Some(x), p)).collect::<Vec<_>>()).unwrap_or_default();
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "all_of" => all("predicates").iter().all(|&t| t),
        "any_of" => all("predicates").iter().any(|&t| t),
        "not" => !test_block(lib, b, v.get("predicate"), p),
        "has_sturdy_face" | "solid" => b.is_sturdy(at(v)),
        "replaceable" => b.is_air(at(v)),
        "matching_blocks" => {
            let q = at(v);
            let name = b.block(q);
            let names: Vec<String> = match v.get("blocks") {
                Some(Value::String(s)) => vec![crate::source::full_id(s)],
                Some(Value::Array(a)) => a.iter().filter_map(|x| x.as_str()).map(crate::source::full_id).collect(),
                _ => Vec::new(),
            };
            names.iter().any(|n| if n == "minecraft:air" { b.is_air(q) } else { nbt::block_name(&name) == n })
        }
        "matching_block_tag" => {
            let q = at(v);
            let tag = v.get("tag").and_then(|t| t.as_str()).unwrap_or("");
            if crate::source::full_id(tag) == "minecraft:air" { b.is_air(q) } else { lib.tag(tag).contains(nbt::block_name(&b.block(q))) }
        }
        _ => true,
    }
}

fn configured(lib: &Library, b: &mut Builder, json: &Value, pos: [i32; 3], rng: &mut Rng) -> bool {
    let kind = json.get("type").and_then(|t| t.as_str()).unwrap_or("");
    // Before 26.3 a feature's settings were under `config`.
    let config = json.get("config").cloned().unwrap_or_else(|| json.clone());
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        "tree" => {
            tree(lib, b, &config, pos, rng);
            true
        }
        "block_pile" => {
            pile(lib, b, &config, pos, rng);
            true
        }
        "random_patch" | "flower" | "no_bonemeal_flower" => {
            patch(lib, b, &config, pos, rng);
            true
        }
        "simple_block" => {
            if b.is_air(pos)
                && let Some(state) = provide(lib, config.get("to_place"), rng)
            {
                b.set(pos, &state, None);
            }
            true
        }
        "block_column" => {
            column(lib, b, &config, pos, rng);
            true
        }
        "sculk_patch" => {
            sculk_patch(lib, b, &config, pos, rng);
            true
        }
        // 26.3 makes a feature of several, one after the other.
        "sequence" | "overlay" => {
            let mut known = true;
            for f in config.get("features").and_then(|f| f.as_array()).into_iter().flatten() {
                known &= run_placed(lib, b, f, pos, rng);
            }
            known
        }
        _ => false,
    }
}

/// A block state from a state provider: one of the game's kinds, a state
/// alone, or (from 26.3) the name of a provider of its own.
fn provide(lib: &Library, v: Option<&Value>, rng: &mut Rng) -> Option<String> {
    let v = v?;
    if let Some(name) = v.as_str() {
        return match lib.json(Kind::StateProvider, name) {
            Some(named) => provide(lib, Some(&named), rng),
            None => Some(nbt::state_of_text(name)),
        };
    }
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let state = |s: Option<&Value>| s.and_then(nbt::state_of_json);
    match kind.strip_prefix("minecraft:").unwrap_or(kind) {
        // A state, as such.
        "" => state(Some(v)),
        "simple_state_provider" | "simple" => state(v.get("state")),
        "rotated_block_provider" | "rotated" => {
            let s = state(v.get("state"))?;
            Some(nbt::set_property(&s, "axis", ["x", "y", "z"][rng.below(3) as usize]))
        }
        "weighted_state_provider" | "weighted" => {
            let entries: Vec<(String, u32)> = v.get("entries")?.as_array()?.iter().filter_map(|e| Some((state(e.get("data"))?, e.get("weight").and_then(|w| w.as_u64()).unwrap_or(1) as u32))).collect();
            let total: u32 = entries.iter().map(|e| e.1).sum();
            let mut r = rng.below(total.max(1));
            entries.into_iter().find(|e| {
                if r < e.1 {
                    true
                } else {
                    r -= e.1;
                    false
                }
            }).map(|e| e.0)
        }
        "noise_threshold_provider" | "noise_provider" | "dual_noise_provider" | "noise_threshold" | "noise" | "dual_noise" => {
            let mut all: Vec<String> = Vec::new();
            for key in ["default_state", "high_states", "low_states", "states"] {
                match v.get(key) {
                    Some(Value::Array(a)) => all.extend(a.iter().filter_map(nbt::state_of_json)),
                    Some(s) => all.extend(nbt::state_of_json(s)),
                    None => {}
                }
            }
            rng.pick(&all).cloned()
        }
        "randomized_int_state_provider" | "randomized_int" => provide(lib, v.get("source"), rng),
        _ => state(v.get("state")),
    }
}

/// A tree: its trunk, then its leaves as its foliage placer lays them
/// (blob, spruce, pine, acacia; the others as a blob).
fn tree(lib: &Library, b: &mut Builder, config: &Value, pos: [i32; 3], rng: &mut Rng) {
    let Some(log) = provide(lib, config.get("trunk_provider"), rng) else { return };
    let Some(leaves) = provide(lib, config.get("foliage_provider"), rng) else { return };
    let below = [pos[0], pos[1] - 1, pos[2]];
    if !b.is_sturdy(below) || !b.is_air(pos) {
        return;
    }
    let trunk = config.get("trunk_placer").cloned().unwrap_or(Value::Null);
    let g = |k: &str| trunk.get(k).and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let height = g("base_height").max(1) + rng.between(0, g("height_rand_a")) + rng.between(0, g("height_rand_b"));
    let trunk_kind = trunk.get("type").and_then(|t| t.as_str()).unwrap_or("");
    let foliage = config.get("foliage_placer").cloned().unwrap_or(Value::Null);
    let foliage_kind = foliage.get("type").and_then(|t| t.as_str()).unwrap_or("").trim_start_matches("minecraft:").to_owned();
    let radius = int_provider(foliage.get("radius"));
    let radius = rng.between(radius.0, radius.1).max(0);
    let mut logs: Vec<[i32; 3]> = Vec::new();
    // Grass under the trunk becomes dirt, as under any tree.
    if b.block(below).starts_with("minecraft:grass_block") {
        b.set(below, "minecraft:dirt", None);
    }
    let mut top = [pos[0], pos[1] + height - 1, pos[2]];
    if trunk_kind.ends_with("forking_trunk_placer") {
        // Up, then leaning a few blocks one way near the top.
        let lean = crate::geom::Dir::HORIZONTAL[rng.below(4) as usize].step();
        let bend_at = height - rng.between(1, 3);
        let mut p = pos;
        for y in 0..height {
            if y >= bend_at {
                p = [p[0] + lean[0], p[1], p[2] + lean[2]];
            }
            logs.push([p[0], pos[1] + y, p[2]]);
        }
        top = *logs.last().expect("a trunk");
    } else {
        for y in 0..height {
            logs.push([pos[0], pos[1] + y, pos[2]]);
        }
    }
    let mut leaf_spots: Vec<[i32; 3]> = Vec::new();
    let disc = |center: [i32; 3], r: i32, corners: bool, rng: &mut Rng, out: &mut Vec<[i32; 3]>| {
        for dz in -r..=r {
            for dx in -r..=r {
                let corner = dx.abs() == r && dz.abs() == r && r > 0;
                if corner && (!corners || rng.chance(0.5)) {
                    continue;
                }
                out.push([center[0] + dx, center[1], center[2] + dz]);
            }
        }
    };
    match foliage_kind.as_str() {
        "spruce_foliage_placer" | "mega_pine_foliage_placer" => {
            // A cone of rings from wide at the bottom to a point.
            let layers = (height - 1).clamp(3, 8);
            let mut r = 0;
            for i in 0..=layers {
                let y = top[1] + 1 - i;
                disc([top[0], y, top[2]], r, false, rng, &mut leaf_spots);
                r = if r >= radius.max(1) { 1 } else { r + 1 };
            }
        }
        "pine_foliage_placer" => {
            for i in 0..4 {
                let r = if i == 0 { 0 } else if i % 2 == 1 { 1 } else { 2.min(radius + 1) };
                disc([top[0], top[1] + 1 - i, top[2]], r, false, rng, &mut leaf_spots);
            }
        }
        "acacia_foliage_placer" => {
            disc(top, radius + 1, false, rng, &mut leaf_spots);
            disc([top[0], top[1] + 1, top[2]], radius.max(1), false, rng, &mut leaf_spots);
        }
        _ => {
            // A blob: two wide layers under two narrow ones.
            for i in 0..4 {
                let r = if i < 2 { radius.max(1) } else { (radius - 1).max(1) };
                disc([top[0], top[1] - 2 + i, top[2]], if i == 3 { r.min(1) } else { r }, true, rng, &mut leaf_spots);
            }
        }
    }
    for &p in &logs {
        if b.is_air(p) || b.get(p).is_some_and(|s| s.contains("leaves")) {
            b.set(p, &log, None);
        }
    }
    for p in leaf_spots {
        if !b.is_air(p) {
            continue;
        }
        // How far it is from the trunk, so that the game does not let the
        // leaves decay.
        let d = logs.iter().map(|l| (l[0] - p[0]).abs() + (l[1] - p[1]).abs() + (l[2] - p[2]).abs()).min().unwrap_or(7).clamp(1, 7);
        let state = nbt::set_property(&leaves, "distance", &d.to_string());
        let state = if d >= 7 { nbt::set_property(&state, "persistent", "true") } else { state };
        b.set(p, &state, None);
    }
}

/// A pile of blocks: hay bales, pumpkins, melons, ice.
fn pile(lib: &Library, b: &mut Builder, config: &Value, pos: [i32; 3], rng: &mut Rng) {
    let (rx, rz) = (2 + rng.below(2) as i32, 2 + rng.below(2) as i32);
    for y in 0..2 {
        for dz in -rz..=rz {
            for dx in -rx..=rx {
                let p = [pos[0] + dx, pos[1] + y, pos[2] + dz];
                let d2 = (dx * dx + dz * dz) as f32;
                let keep = d2 <= rng.float() * 10.0 - rng.float() * 6.0 || rng.float() < 0.031;
                if keep
                    && b.is_air(p)
                    && b.is_sturdy([p[0], p[1] - 1, p[2]])
                    && let Some(state) = provide(lib, config.get("state_provider"), rng)
                {
                    b.set(p, &state, None);
                }
            }
        }
    }
}

/// Patches of plants (flowers, grass, berry bushes, cacti): a number of
/// tries spread around `pos`, each set where there is room.
fn patch(lib: &Library, b: &mut Builder, config: &Value, pos: [i32; 3], rng: &mut Rng) {
    let tries = config.get("tries").and_then(|t| t.as_u64()).unwrap_or(32).min(256);
    let xz = config.get("xz_spread").and_then(|t| t.as_i64()).unwrap_or(7) as i32;
    let ys = config.get("y_spread").and_then(|t| t.as_i64()).unwrap_or(3) as i32;
    // A placed feature, or one named: grown as it says, at each try.
    let inner = match config.get("feature") {
        Some(Value::String(id)) => serde_json::json!({ "feature": id }),
        Some(f) => f.clone(),
        None => return,
    };
    for _ in 0..tries {
        let p = [pos[0] + rng.between(-xz, xz) - rng.between(-xz, xz) / 2, pos[1] + rng.between(-ys, ys) / 2, pos[2] + rng.between(-xz, xz) - rng.between(-xz, xz) / 2];
        // On the ground: in the air, over a whole block.
        if !b.is_air(p) || !b.is_sturdy([p[0], p[1] - 1, p[2]]) {
            continue;
        }
        let ground = b.block([p[0], p[1] - 1, p[2]]);
        let cactus = inner.to_string().contains("cactus");
        if cactus && !ground.contains("sand") || !cactus && !(ground.contains("grass_block") || ground.contains("dirt") || ground.contains("podzol") || ground.contains("moss")) {
            continue;
        }
        run_placed(lib, b, &inner, p, rng);
    }
}

/// A column of blocks (a cactus).
fn column(lib: &Library, b: &mut Builder, config: &Value, pos: [i32; 3], rng: &mut Rng) {
    let mut y = pos[1];
    for layer in config.get("layers").and_then(|l| l.as_array()).into_iter().flatten() {
        let h = match layer.get("height") {
            Some(Value::Number(n)) => n.as_i64().unwrap_or(1) as i32,
            Some(other) => {
                let (lo, hi) = int_provider(Some(other));
                rng.between(lo, hi.max(lo))
            }
            None => 1,
        };
        for _ in 0..h {
            let p = [pos[0], y, pos[2]];
            if !b.is_air(p) {
                return;
            }
            if let Some(state) = provide(lib, layer.get("provider"), rng) {
                b.set(p, &state, None);
            }
            y += 1;
        }
    }
}

/// A patch of sculk, as the game spreads one in the deep dark: sculk over
/// the stone and the city's deepslate around `pos`, veins on the faces it
/// reaches, a catalyst half the time, sensors, and shriekers that may
/// summon the warden (`can_summon=true`, as only the world's own are).
fn sculk_patch(lib: &Library, b: &mut Builder, config: &Value, pos: [i32; 3], rng: &mut Rng) {
    let replaceable = lib.tag("minecraft:sculk_replaceable_world_gen");
    let can_take = |state: &str| replaceable.contains(nbt::block_name(state)) || state.starts_with("minecraft:deepslate") || state.starts_with("minecraft:stone");
    // Somewhere for it to start: air beside a whole block.
    if !b.is_air(pos) {
        return;
    }
    let charges = config.get("charge_count").and_then(|v| v.as_i64()).unwrap_or(10) as i32 * config.get("amount_per_charge").and_then(|v| v.as_i64()).unwrap_or(32) as i32;
    let rounds = config.get("spread_rounds").and_then(|v| v.as_i64()).unwrap_or(1).max(1) as i32;
    // The blocks it may cover: solid, of the kinds it takes, with air
    // beside them, near the start; nearer first.
    let reach = 4 + rounds;
    let mut spots: Vec<([i32; 3], i32)> = Vec::new();
    for dy in -3..=2 {
        for dz in -reach..=reach {
            for dx in -reach..=reach {
                let p = [pos[0] + dx, pos[1] + dy, pos[2] + dz];
                let d = dx.abs() + dy.abs() + dz.abs();
                if d > reach + 2 {
                    continue;
                }
                let state = b.block(p);
                if !can_take(&state) || !crate::build::is_sturdy(&state) {
                    continue;
                }
                let open = crate::geom::Dir::HORIZONTAL.iter().map(|d| d.step()).chain([[0, 1, 0], [0, -1, 0]]).any(|s| b.is_air([p[0] + s[0], p[1] + s[1], p[2] + s[2]]));
                if open {
                    spots.push((p, d));
                }
            }
        }
    }
    spots.sort_by_key(|s| s.1);
    let budget = (charges / 4).clamp(8, 160) as usize;
    let mut covered: Vec<[i32; 3]> = Vec::new();
    for (p, d) in spots {
        if covered.len() >= budget {
            break;
        }
        // Thinner the farther: a ragged edge.
        if rng.float() < 1.0 - d as f32 / (reach as f32 + 3.0) {
            b.set(p, "minecraft:sculk", None);
            covered.push(p);
        }
    }
    // Veins on the faces of blocks it did not take, around what it took.
    for &p in &covered {
        for s in [[1, 0, 0], [-1, 0, 0], [0, 0, 1], [0, 0, -1], [0, 1, 0]] {
            let q = [p[0] + s[0], p[1] + s[1], p[2] + s[2]];
            if !b.is_air(q) || rng.float() > 0.25 {
                continue;
            }
            // The vein lies on the block below it, if that is not sculk.
            let under = [q[0], q[1] - 1, q[2]];
            if b.is_sturdy(under) && !b.block(under).starts_with("minecraft:sculk") {
                b.set(q, "minecraft:sculk_vein[down=true,east=false,north=false,south=false,up=false,waterlogged=false,west=false]", None);
            }
        }
    }
    // Growths on top of the sculk: sensors, now and then a shrieker.
    let mut growths = 0;
    for &p in &covered {
        let above = [p[0], p[1] + 1, p[2]];
        if growths >= 3 || !b.is_air(above) || rng.float() > 0.03 {
            continue;
        }
        if rng.below(11) == 0 {
            b.set(above, "minecraft:sculk_shrieker[can_summon=true,shrieking=false,waterlogged=false]", None);
        } else {
            b.set(above, "minecraft:sculk_sensor[power=0,sculk_sensor_phase=inactive,waterlogged=false]", Some(sensor()));
        }
        growths += 1;
    }
    // A catalyst at the start, half the time, over a whole block.
    let catalyst = config.get("catalyst_chance").and_then(|v| v.as_f64()).unwrap_or(0.0) as f32;
    if catalyst > 0.0 && rng.float() <= catalyst && b.is_sturdy([pos[0], pos[1] - 1, pos[2]]) {
        b.set(pos, "minecraft:sculk_catalyst[bloom=false]", None);
    }
    // The rare growths: shriekers around the start that call the warden.
    let (lo, hi) = int_provider(config.get("extra_rare_growths"));
    for _ in 0..rng.between(lo, hi) {
        let p = [pos[0] + rng.between(-2, 2), pos[1], pos[2] + rng.between(-2, 2)];
        if b.is_air(p) && b.is_sturdy([p[0], p[1] - 1, p[2]]) {
            b.set(p, "minecraft:sculk_shrieker[can_summon=true,shrieking=false,waterlogged=false]", None);
        }
    }
}

fn sensor() -> crate::nbt::Compound {
    let mut c = crate::nbt::Compound::new();
    c.insert("id".into(), fastnbt::Value::String("minecraft:sculk_sensor".into()));
    c
}

/// Whether `state` is a block a feature may grow into.
pub fn is_free(state: &str) -> bool {
    is_air(state)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ground::Flat;
    use crate::source::Source;
    use std::collections::HashMap;

    fn lib() -> Library {
        let mut files: HashMap<String, Vec<u8>> = HashMap::new();
        files.insert(
            "data/minecraft/worldgen/configured_feature/oak.json".into(),
            br#"{"type":"minecraft:tree","config":{"foliage_placer":{"type":"minecraft:blob_foliage_placer","height":3,"offset":0,"radius":2},"foliage_provider":{"type":"minecraft:simple_state_provider","state":{"Name":"minecraft:oak_leaves","Properties":{"distance":"7","persistent":"false","waterlogged":"false"}}},"trunk_placer":{"type":"minecraft:straight_trunk_placer","base_height":4,"height_rand_a":2,"height_rand_b":0},"trunk_provider":{"type":"minecraft:simple_state_provider","state":{"Name":"minecraft:oak_log","Properties":{"axis":"y"}}}}}"#.to_vec(),
        );
        files.insert("data/minecraft/worldgen/placed_feature/oak.json".into(), br#"{"feature":"minecraft:oak","placement":[]}"#.to_vec());
        files.insert(
            "data/minecraft/worldgen/configured_feature/sculk_patch_ancient_city.json".into(),
            br#"{"type":"minecraft:sculk_patch","config":{"amount_per_charge":32,"catalyst_chance":0.5,"charge_count":10,"extra_rare_growths":{"type":"minecraft:uniform","max_inclusive":3,"min_inclusive":1},"growth_rounds":0,"spread_attempts":64,"spread_rounds":1}}"#.to_vec(),
        );
        files.insert("data/minecraft/worldgen/placed_feature/sculk_patch_ancient_city.json".into(), br#"{"feature":"minecraft:sculk_patch_ancient_city","placement":[]}"#.to_vec());
        files.insert("data/minecraft/tags/block/sculk_replaceable_world_gen.json".into(), br#"{"values":["minecraft:stone","minecraft:deepslate"]}"#.to_vec());
        Library::new(Source::Memory(files), "test")
    }

    #[test]
    fn trees_grow_with_leaves_that_stay() {
        let lib = lib();
        let ground = Flat { height: 64, water: None };
        let mut b = Builder::new(&ground, 1);
        assert!(place(&lib, &mut b, "minecraft:oak", [0, 64, 0], &mut Rng::new(2)));
        assert!(b.get([0, 64, 0]).unwrap().starts_with("minecraft:oak_log"));
        let s = b.finish(1, "t", "", true, 0);
        let leaves: Vec<&String> = s.palette.iter().filter(|p| p.contains("leaves")).collect();
        assert!(!leaves.is_empty());
        assert!(leaves.iter().all(|l| !l.contains("distance=7") || l.contains("persistent=true")), "{leaves:?}");
        assert!(!place(&lib, &mut Builder::new(&ground, 1), "minecraft:no_such_tree", [0, 64, 0], &mut Rng::new(2)));
    }

    #[test]
    fn sculk_patches_bring_shriekers_that_summon_the_warden() {
        let lib = lib();
        // A floor of deepslate at y -52, air above.
        let ground = Flat { height: -51, water: None };
        let mut shriekers = 0;
        for seed in 0..20 {
            let mut b = Builder::new(&ground, seed);
            assert!(place(&lib, &mut b, "minecraft:sculk_patch_ancient_city", [0, -51, 0], &mut Rng::new(seed)));
            let s = b.finish(1, "t", "", true, 0);
            assert!(s.palette.iter().any(|p| p == "minecraft:sculk"));
            let mut column = Vec::new();
            for x in -3..=3 {
                for z in -3..=3 {
                    s.column(x, z, &mut column);
                    shriekers += column.iter().filter(|&&(_, k)| s.palette[k as usize].starts_with("minecraft:sculk_shrieker[can_summon=true")).count();
                }
            }
        }
        assert!(shriekers >= 20, "{shriekers} shriekers in 20 patches");
    }
}
