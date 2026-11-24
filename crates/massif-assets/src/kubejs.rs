//! The blocks a modpack makes with KubeJS: its startup scripts register
//! them (`StartupEvents.registry('block', e => e.create('marble')…)`, or
//! `onEvent('block.registry', …)` before KubeJS 6), and KubeJS writes their
//! block states and models when the game starts, from what the script says
//! of their textures. Massif does not run the scripts: it reads the calls
//! and writes the same files in memory, for the blocks whose files the
//! `kubejs/assets` folder does not have already.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

/// A block a script makes.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct ScriptBlock {
    /// `kubejs:marble`.
    pub id: String,
    /// KubeJS's type: `basic` (a cube), `slab`, `stairs`, `fence`, `wall`,
    /// `falling`, `cardinal`, `crop`…
    pub kind: String,
    /// `.displayName('Polished Marble')`.
    pub name: Option<String>,
    /// Its textures by face (`all`, `up`, `down`, `north`… `side`), as the
    /// script names them (`kubejs:block/marble`).
    pub textures: BTreeMap<String, String>,
    /// `.model('kubejs:block/marble')`: a model of its own.
    pub model: Option<String>,
}

/// The string literals of a call's arguments, from just after its `(`.
fn literals(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut depth = 0;
    let mut chars = text.char_indices().peekable();
    while let Some((_, c)) = chars.next() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            '\'' | '"' | '`' => {
                let mut s = String::new();
                while let Some((_, d)) = chars.next() {
                    if d == '\\' {
                        if let Some((_, e)) = chars.next() {
                            s.push(e);
                        }
                    } else if d == c {
                        break;
                    } else {
                        s.push(d);
                    }
                }
                if depth == 0 {
                    out.push(s);
                }
            }
            _ => {}
        }
    }
    out
}

/// The script without its comments (`//…` and `/* … */`), strings kept.
fn uncommented(text: &str) -> String {
    let b: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let mut quote: Option<char> = None;
    while i < b.len() {
        let c = b[i];
        if let Some(q) = quote {
            out.push(c);
            if c == '\\' && i + 1 < b.len() {
                out.push(b[i + 1]);
                i += 1;
            } else if c == q {
                quote = None;
            }
        } else if c == '/' && b.get(i + 1) == Some(&'/') {
            while i < b.len() && b[i] != '\n' {
                i += 1;
            }
            continue;
        } else if c == '/' && b.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < b.len() && !(b[i] == '*' && b[i + 1] == '/') {
                i += 1;
            }
            i += 2;
            continue;
        } else {
            if matches!(c, '\'' | '"' | '`') {
                quote = Some(c);
            }
            out.push(c);
        }
        i += 1;
    }
    out
}

/// The parts of a script where blocks are registered: from each registry
/// of blocks to the next registry of anything.
fn block_regions(text: &str) -> Vec<&str> {
    let starts: Vec<(usize, bool)> = {
        let mut v = Vec::new();
        for (at, _) in text.match_indices("StartupEvents.registry(").chain(text.match_indices("onEvent(")) {
            let args = literals(&text[at + text[at..].find('(').unwrap_or(0) + 1..]);
            let first = args.first().map(String::as_str).unwrap_or("");
            v.push((at, matches!(first, "block" | "minecraft:block" | "block.registry" | "blocks")));
        }
        v.sort();
        v
    };
    let mut out = Vec::new();
    for (k, &(at, blocks)) in starts.iter().enumerate() {
        if blocks {
            let end = starts.get(k + 1).map_or(text.len(), |s| s.0);
            out.push(&text[at..end]);
        }
    }
    out
}

/// The blocks a startup script makes.
pub fn parse_script(text: &str) -> Vec<ScriptBlock> {
    let text = uncommented(text);
    let mut out = Vec::new();
    for region in block_regions(&text) {
        let creates: Vec<usize> = region.match_indices(".create(").map(|(i, _)| i).collect();
        for (k, &at) in creates.iter().enumerate() {
            let chain = &region[at..creates.get(k + 1).copied().unwrap_or(region.len())];
            let args = literals(&chain[".create(".len()..]);
            let Some(raw) = args.first().filter(|s| !s.trim().is_empty()) else { continue };
            let id = if raw.contains(':') { raw.trim().to_owned() } else { format!("kubejs:{}", raw.trim()) };
            let mut block = ScriptBlock { id, kind: args.get(1).cloned().unwrap_or_else(|| "basic".to_owned()), ..Default::default() };
            // The calls chained on it.
            for (i, _) in chain.match_indices('.').skip(1) {
                let rest = &chain[i + 1..];
                let name: String = rest.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                let after = rest[name.len()..].trim_start();
                let Some(after) = after.strip_prefix('(') else { continue };
                let a = literals(after);
                match (name.as_str(), a.as_slice()) {
                    ("displayName", [n, ..]) => block.name = Some(n.clone()),
                    ("textureAll", [t, ..]) => {
                        block.textures.insert("all".into(), t.clone());
                    }
                    ("texture" | "textureSide", [side, t, ..]) => {
                        let side = match side.to_ascii_lowercase().as_str() {
                            "top" => "up".to_owned(),
                            "bottom" => "down".to_owned(),
                            "sides" => "side".to_owned(),
                            s => s.to_owned(),
                        };
                        block.textures.insert(side, t.clone());
                    }
                    ("model", [m, ..]) => block.model = Some(m.clone()),
                    _ => {}
                }
            }
            out.push(block);
        }
    }
    out
}

/// `kubejs:marble` → `kubejs:block/marble`: the texture KubeJS gives a
/// block whose script names none.
fn default_texture(id: &str) -> String {
    let (ns, path) = id.split_once(':').unwrap_or(("kubejs", id));
    format!("{ns}:block/{path}")
}

/// The block states, models and names KubeJS would write for these blocks,
/// as files of its `kubejs/assets` folder.
pub fn generated_files(blocks: &[ScriptBlock], has: &dyn Fn(&str) -> bool) -> Vec<(String, Vec<u8>)> {
    let mut out = Vec::new();
    let mut names: HashMap<String, serde_json::Map<String, serde_json::Value>> = HashMap::new();
    for b in blocks {
        let (ns, path) = b.id.split_once(':').unwrap_or(("kubejs", &b.id));
        if let Some(n) = &b.name {
            names.entry(ns.to_owned()).or_default().insert(format!("block.{ns}.{}", path.replace('/', ".")), serde_json::Value::String(n.clone()));
        }
        let state_file = format!("assets/{ns}/blockstates/{path}.json");
        if has(&state_file) {
            continue;
        }
        let model = |suffix: &str| format!("{ns}:block/{path}{suffix}");
        let all = b.textures.get("all").cloned().unwrap_or_else(|| default_texture(&b.id));
        let tex = |face: &str| -> String {
            b.textures
                .get(face)
                .or_else(|| if ["north", "south", "east", "west"].contains(&face) { b.textures.get("side") } else { None })
                .cloned()
                .unwrap_or_else(|| all.clone())
        };
        let mut models: Vec<(String, serde_json::Value)> = Vec::new();
        let state = match b.kind.as_str() {
            "slab" => {
                let t = serde_json::json!({"bottom": tex("down"), "top": tex("up"), "side": tex("north")});
                models.push((model(""), serde_json::json!({"parent": "minecraft:block/slab", "textures": t})));
                models.push((model("_top"), serde_json::json!({"parent": "minecraft:block/slab_top", "textures": t})));
                models.push((model("_double"), serde_json::json!({"parent": "minecraft:block/cube_bottom_top", "textures": t})));
                serde_json::json!({"variants": {"type=bottom": {"model": model("")}, "type=top": {"model": model("_top")}, "type=double": {"model": model("_double")}}})
            }
            "stairs" => {
                let t = serde_json::json!({"bottom": tex("down"), "top": tex("up"), "side": tex("north")});
                models.push((model(""), serde_json::json!({"parent": "minecraft:block/stairs", "textures": t})));
                serde_json::json!({"variants": {"": {"model": model("")}}})
            }
            "fence" => {
                models.push((model(""), serde_json::json!({"parent": "minecraft:block/fence_post", "textures": {"texture": all.clone()}})));
                serde_json::json!({"variants": {"": {"model": model("")}}})
            }
            "wall" => {
                models.push((model(""), serde_json::json!({"parent": "minecraft:block/template_wall_post", "textures": {"wall": all.clone()}})));
                serde_json::json!({"variants": {"": {"model": model("")}}})
            }
            "crop" | "cross" | "plant" => {
                models.push((model(""), serde_json::json!({"parent": "minecraft:block/cross", "textures": {"cross": all.clone()}})));
                serde_json::json!({"variants": {"": {"model": model("")}}})
            }
            _ => {
                if let Some(m) = &b.model {
                    serde_json::json!({"variants": {"": {"model": m}}})
                } else {
                    let faces = ["up", "down", "north", "south", "east", "west", "side"];
                    let own = faces.iter().any(|f| b.textures.contains_key(*f));
                    let m = if own {
                        serde_json::json!({"parent": "minecraft:block/cube", "textures": {
                            "particle": tex("north"), "up": tex("up"), "down": tex("down"),
                            "north": tex("north"), "south": tex("south"), "east": tex("east"), "west": tex("west")
                        }})
                    } else {
                        serde_json::json!({"parent": "minecraft:block/cube_all", "textures": {"all": all.clone()}})
                    };
                    models.push((model(""), m));
                    serde_json::json!({"variants": {"": {"model": model("")}}})
                }
            }
        };
        out.push((state_file, serde_json::to_vec(&state).unwrap_or_default()));
        for (m, json) in models {
            let (mns, mpath) = m.split_once(':').unwrap_or(("kubejs", &m));
            let file = format!("assets/{mns}/models/{mpath}.json");
            if !has(&file) {
                out.push((file, serde_json::to_vec(&json).unwrap_or_default()));
            }
        }
    }
    for (ns, map) in names {
        let file = format!("assets/{ns}/lang/en_us.json");
        out.push((file, serde_json::to_vec(&serde_json::Value::Object(map)).unwrap_or_default()));
    }
    out
}

/// The blocks the startup scripts of a `kubejs` folder make, every script
/// read.
pub fn script_blocks(kubejs: &Path) -> Vec<ScriptBlock> {
    let mut out = Vec::new();
    let mut stack = vec![kubejs.join("startup_scripts")];
    let mut files = Vec::new();
    while let Some(d) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&d) else { continue };
        for e in read.flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().is_some_and(|x| x.eq_ignore_ascii_case("js")) {
                files.push(p);
            }
        }
    }
    files.sort();
    for f in files {
        if let Ok(text) = std::fs::read_to_string(&f) {
            out.extend(parse_script(&text));
        }
    }
    out
}

/// KubeJS's folder read as a mod's jar: its `assets`, with the files KubeJS
/// would write for the blocks its scripts make (and the names they give
/// them, over the folder's own); `None` when it has neither.
pub fn archive(kubejs: &Path) -> Option<crate::zip::Archive> {
    let blocks = script_blocks(kubejs);
    let folder = crate::zip::Archive::folder(kubejs, &["assets"]).ok();
    if blocks.is_empty() {
        return folder;
    }
    let has = |name: &str| folder.as_ref().is_some_and(|a| a.contains(name));
    let mut files = generated_files(&blocks, &has);
    // The scripts' names join the folder's own language file.
    for (name, bytes) in &mut files {
        if name.ends_with("/lang/en_us.json")
            && let Some(own) = folder.as_ref().and_then(|a| a.read(name)).and_then(Result::ok)
            && let Ok(serde_json::Value::Object(mut map)) = serde_json::from_slice::<serde_json::Value>(&own)
            && let Ok(serde_json::Value::Object(ours)) = serde_json::from_slice::<serde_json::Value>(bytes)
        {
            for (k, v) in ours {
                map.entry(k).or_insert(v);
            }
            *bytes = serde_json::to_vec(&serde_json::Value::Object(map)).unwrap_or_default();
        }
    }
    Some(match folder {
        Some(a) => a.with_files(files),
        None => crate::zip::Archive::of_files(files),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = r#"
// priority: 0
StartupEvents.registry('item', event => {
  event.create('not_a_block')
})

StartupEvents.registry('block', event => {
  event.create('marble').displayName('Marble').hardness(1.5).soundType('stone') // a cube
  event.create('marble_slab', 'slab').textureAll('kubejs:block/marble')
  event.create("mypack:mossy_tiles")
    .displayName("Mossy \"Tiles\"")
    .texture('up', 'mypack:block/mossy_top')
    .texture('side', 'mypack:block/mossy_side')
  /* event.create('commented_out') */
  event.create('pillar', 'cardinal').model('kubejs:block/pillar_model')
})

onEvent('block.registry', e => {
  e.create('old_style').textureAll("kubejs:block/old")
})
"#;

    #[test]
    fn a_startup_scripts_blocks_are_read() {
        let blocks = parse_script(SCRIPT);
        let ids: Vec<&str> = blocks.iter().map(|b| b.id.as_str()).collect();
        assert_eq!(ids, ["kubejs:marble", "kubejs:marble_slab", "mypack:mossy_tiles", "kubejs:pillar", "kubejs:old_style"]);
        assert_eq!(blocks[0].name.as_deref(), Some("Marble"));
        assert_eq!(blocks[0].kind, "basic");
        assert_eq!(blocks[1].kind, "slab");
        assert_eq!(blocks[2].name.as_deref(), Some("Mossy \"Tiles\""));
        assert_eq!(blocks[2].textures.get("up").map(String::as_str), Some("mypack:block/mossy_top"));
        assert_eq!(blocks[2].textures.get("side").map(String::as_str), Some("mypack:block/mossy_side"));
        assert_eq!(blocks[3].model.as_deref(), Some("kubejs:block/pillar_model"));
    }

    #[test]
    fn the_files_kubejs_would_write() {
        let blocks = parse_script(SCRIPT);
        let files: HashMap<String, serde_json::Value> = generated_files(&blocks, &|n| n == "assets/kubejs/blockstates/old_style.json").into_iter().map(|(n, b)| (n, serde_json::from_slice(&b).unwrap())).collect();
        assert_eq!(files["assets/kubejs/models/block/marble.json"]["textures"]["all"], "kubejs:block/marble", "the texture by default");
        assert_eq!(files["assets/kubejs/blockstates/marble_slab.json"]["variants"]["type=top"]["model"], "kubejs:block/marble_slab_top");
        assert_eq!(files["assets/mypack/models/block/mossy_tiles.json"]["textures"]["north"], "mypack:block/mossy_side");
        assert_eq!(files["assets/mypack/models/block/mossy_tiles.json"]["textures"]["down"], "mypack:block/mossy_tiles", "a face not named: the block's texture");
        assert_eq!(files["assets/kubejs/blockstates/pillar.json"]["variants"][""]["model"], "kubejs:block/pillar_model");
        assert!(!files.contains_key("assets/kubejs/blockstates/old_style.json"), "the folder's own file is kept");
        assert_eq!(files["assets/kubejs/lang/en_us.json"]["block.kubejs.marble"], "Marble");
        assert_eq!(files["assets/mypack/lang/en_us.json"]["block.mypack.mossy_tiles"], "Mossy \"Tiles\"");
    }
}
