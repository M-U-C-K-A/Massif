//! Reading and writing the NBT of templates, block entities and entities.

use fastnbt::Value;
use std::collections::HashMap;

pub type Compound = HashMap<String, Value>;

pub fn int(v: Option<&Value>) -> Option<i64> {
    match v? {
        Value::Byte(b) => Some(*b as i64),
        Value::Short(s) => Some(*s as i64),
        Value::Int(i) => Some(*i as i64),
        Value::Long(l) => Some(*l),
        _ => None,
    }
}

pub fn float(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Float(f) => Some(*f as f64),
        Value::Double(d) => Some(*d),
        other => int(Some(other)).map(|i| i as f64),
    }
}

pub fn compound<'a>(c: &'a Compound, key: &str) -> Option<&'a Compound> {
    match c.get(key)? {
        Value::Compound(c) => Some(c),
        _ => None,
    }
}

pub fn list<'a>(c: &'a Compound, key: &str) -> &'a [Value] {
    match c.get(key) {
        Some(Value::List(l)) => l,
        _ => &[],
    }
}

pub fn string<'a>(c: &'a Compound, key: &str) -> Option<&'a str> {
    match c.get(key)? {
        Value::String(s) => Some(s),
        _ => None,
    }
}

/// Three whole numbers: a list of ints, or an int array.
pub fn triple(v: Option<&Value>) -> Option<[i32; 3]> {
    match v? {
        Value::IntArray(a) if a.len() >= 3 => Some([a[0], a[1], a[2]]),
        Value::List(l) if l.len() >= 3 => Some([int(l.first())? as i32, int(l.get(1))? as i32, int(l.get(2))? as i32]),
        _ => None,
    }
}

/// Three numbers: a list of doubles.
pub fn triple_f(v: Option<&Value>) -> Option<[f64; 3]> {
    match v? {
        Value::List(l) if l.len() >= 3 => Some([float(l.first())?, float(l.get(1))?, float(l.get(2))?]),
        _ => None,
    }
}

/// A block state from a palette entry: its `Name` and its `Properties` in
/// the game's order (by name) — `id` and `properties` from 26.3, which
/// renamed them.
pub fn state_of(entry: &Value) -> String {
    let Value::Compound(c) = entry else { return "minecraft:air".to_owned() };
    let name = string(c, "Name").or_else(|| string(c, "id")).unwrap_or("minecraft:air");
    let props: Vec<(String, String)> = compound(c, "Properties")
        .or_else(|| compound(c, "properties"))
        .map(|p| {
            p.iter()
                .map(|(k, v)| {
                    let v = match v {
                        Value::String(s) => s.clone(),
                        other => int(Some(other)).map_or_else(String::new, |n| n.to_string()),
                    };
                    (k.clone(), v)
                })
                .collect()
        })
        .unwrap_or_default();
    with_props(name, props)
}

/// `name[k=v,…]`, its properties sorted.
pub fn with_props(name: &str, mut props: Vec<(String, String)>) -> String {
    let name = if name.contains(':') { name.to_owned() } else { format!("minecraft:{name}") };
    if props.is_empty() {
        return name;
    }
    props.sort();
    let props: Vec<String> = props.into_iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{name}[{}]", props.join(","))
}

/// A block state from JSON (`{"Name": …, "Properties": {…}}`).
pub fn state_of_json(v: &serde_json::Value) -> Option<String> {
    // 26.3 writes a state without properties as its name alone.
    if let Some(text) = v.as_str() {
        return Some(state_of_text(text));
    }
    let name = v.get("Name").or_else(|| v.get("id"))?.as_str()?;
    let props = v
        .get("Properties")
        .or_else(|| v.get("properties"))
        .and_then(|p| p.as_object())
        .map(|p| p.iter().map(|(k, v)| (k.clone(), v.as_str().map_or_else(|| v.to_string(), str::to_owned))).collect())
        .unwrap_or_default();
    Some(with_props(name, props))
}

/// A block state as the game writes one in text (`minecraft:stone_stairs[facing=east]`),
/// with its properties sorted.
pub fn state_of_text(text: &str) -> String {
    let text = text.trim();
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.split(']').next().unwrap_or("")),
        None => (text, ""),
    };
    // Block entity data after the state (`minecraft:chest{…}`) is not a
    // property.
    let name = name.split('{').next().unwrap_or(name);
    let props: Vec<(String, String)> = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).collect();
    with_props(name, props)
}

/// The name of a block state, without its properties.
#[inline]
pub fn block_name(state: &str) -> &str {
    state.split('[').next().unwrap_or(state)
}

/// The value of property `key` of a block state.
pub fn property<'a>(state: &'a str, key: &str) -> Option<&'a str> {
    let (_, rest) = state.split_once('[')?;
    rest.trim_end_matches(']').split(',').filter_map(|kv| kv.split_once('=')).find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// A block state with property `key` set to `value` (added if missing).
pub fn set_property(state: &str, key: &str, value: &str) -> String {
    let (name, rest) = match state.split_once('[') {
        Some((n, r)) => (n, r.trim_end_matches(']')),
        None => (state, ""),
    };
    let mut props: Vec<(String, String)> = rest.split(',').filter_map(|kv| kv.split_once('=')).filter(|(k, _)| *k != key).map(|(k, v)| (k.to_owned(), v.to_owned())).collect();
    props.push((key.to_owned(), value.to_owned()));
    with_props(name, props)
}

/// The bytes of a compound as a file of NBT holds it (unnamed root).
pub fn to_bytes(c: &Compound) -> Vec<u8> {
    fastnbt::to_bytes(&Value::Compound(c.clone())).unwrap_or_else(|_| vec![10, 0, 0, 0])
}

pub fn from_bytes(bytes: &[u8]) -> Option<Compound> {
    match massif_objects::parse_nbt(bytes).ok()? {
        Value::Compound(c) => Some(c),
        _ => None,
    }
}

pub fn doubles(v: [f64; 3]) -> Value {
    Value::List(v.iter().map(|&d| Value::Double(d)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_read_and_changed() {
        assert_eq!(state_of_text("minecraft:stone_stairs[half=bottom,facing=east]"), "minecraft:stone_stairs[facing=east,half=bottom]");
        assert_eq!(state_of_text("air"), "minecraft:air");
        assert_eq!(state_of_text("minecraft:chest[facing=north]{LootTable:\"x\"}"), "minecraft:chest[facing=north]");
        assert_eq!(property("minecraft:jigsaw[orientation=up_north]", "orientation"), Some("up_north"));
        assert_eq!(set_property("minecraft:sculk_shrieker[can_summon=false,shrieking=false,waterlogged=false]", "can_summon", "true"), "minecraft:sculk_shrieker[can_summon=true,shrieking=false,waterlogged=false]");
        assert_eq!(set_property("minecraft:stone", "x", "1"), "minecraft:stone[x=1]");
        let json: serde_json::Value = serde_json::from_str(r#"{"Name":"minecraft:oak_log","Properties":{"axis":"x"}}"#).unwrap();
        assert_eq!(state_of_json(&json).as_deref(), Some("minecraft:oak_log[axis=x]"));
        let mut c = Compound::new();
        c.insert("id".into(), Value::String("minecraft:chest".into()));
        assert_eq!(from_bytes(&to_bytes(&c)).unwrap(), c);
    }
}
