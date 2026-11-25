//! A block's state, edited as WorldPainter's block picker does: each of
//! its properties (`layers`, `facing`, `half`, `waterlogged`, `age`…) with
//! the values its states file or a list of blocks allows, the game's
//! default when left alone, and Random where a cover may vary it.

use crate::theme::Palette;
use egui::{RichText, Ui};
use massif_assets::materials::Property;

/// `minecraft:snow[layers=3]` → (`minecraft:snow`, [(layers, 3)]).
pub fn parse(state: &str) -> (String, Vec<(String, String)>) {
    let (id, props) = match state.split_once('[') {
        Some((id, rest)) => (id, rest.trim_end_matches(']')),
        None => (state, ""),
    };
    let props = props.split(',').filter_map(|kv| kv.split_once('=')).map(|(k, v)| (k.trim().to_owned(), v.trim().to_owned())).filter(|(k, _)| !k.is_empty()).collect();
    (id.trim().to_owned(), props)
}

/// The state of `id` with `props`: `id[k=v,k2=v2]`, or `id` alone.
pub fn format(id: &str, props: &[(String, String)]) -> String {
    if props.is_empty() {
        return id.to_owned();
    }
    let inner: Vec<String> = props.iter().map(|(k, v)| format!("{k}={v}")).collect();
    format!("{id}[{}]", inner.join(","))
}

/// Whether a cover may grow a property at random: which way it faces or
/// turns, how old it is, how many it holds.
pub fn may_vary(name: &str) -> bool {
    matches!(name, "facing" | "rotation" | "age" | "axis" | "flower_amount" | "pickles" | "segment_amount" | "eggs" | "candles" | "leaves" | "stage")
}

/// What the editor asks of the cover.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// The state is now this one.
    State(String),
    /// One row for each value of this property, the others kept.
    Random(String),
}

/// The states of a row grown at random by `property`: one for each of
/// its `values`.
pub fn spread(state: &str, property: &str, values: &[String]) -> Vec<String> {
    let (id, props) = parse(state);
    values
        .iter()
        .map(|v| {
            let mut p: Vec<(String, String)> = props.iter().filter(|(k, _)| k != property).cloned().collect();
            p.push((property.to_owned(), v.clone()));
            p.sort_by_key(|(k, _)| props.iter().position(|(k2, _)| k2 == k).unwrap_or(usize::MAX));
            format(&id, &p)
        })
        .collect()
}

/// The editor of `state`, its block's `properties` known: each one a
/// choice of Default (left out: the game's) and its values — a segmented
/// control for a few, a list for more —, Random where it may vary; the
/// state it makes, under them.
pub fn editor(ui: &mut Ui, id_salt: impl std::hash::Hash + std::fmt::Debug, state: &str, properties: &[Property]) -> Option<Change> {
    let pal = Palette::of(ui);
    let (id, mut props) = parse(state);
    let mut change = None;
    let salt = egui::Id::unique(&id_salt);
    if properties.is_empty() {
        crate::panels::hint(ui, tr!("This block has no properties the game's files or the lists of blocks name."));
    }
    crate::widgets::well(ui, |ui| {
        for p in properties {
            let now = props.iter().find(|(k, _)| *k == p.name).map(|(_, v)| v.clone());
            let mut pick: Option<Option<String>> = None;
            crate::widgets::row(ui, &p.name, |ui| {
                if may_vary(&p.name) && p.values.len() > 1 && ui.small_button(tr!("Random")).on_hover_text(tr!("One row for each value, sharing the weight: the cover grows them mixed")).clicked() {
                    change = Some(Change::Random(p.name.clone()));
                }
                let numeric = p.values.len() > 1 && p.values.iter().all(|v| v.parse::<i64>().is_ok());
                if p.values.len() <= 3 && !numeric || (numeric && p.values.len() <= 2) {
                    let mut labels = vec![tr!("Default")];
                    labels.extend(p.values.iter().map(String::as_str));
                    let mut on = vec![now.is_none()];
                    on.extend(p.values.iter().map(|v| now.as_deref() == Some(v.as_str())));
                    if let Some(k) = crate::widgets::button_group(ui, &format!("{salt:?} {}", p.name), &labels, &on) {
                        pick = Some(if k == 0 { None } else { Some(p.values[k - 1].clone()) });
                    }
                } else {
                    let text = now.clone().unwrap_or_else(|| tr!("Default").to_owned());
                    crate::widgets::combo(salt.with(&p.name)).selected_text(text).width(120.0).height(260.0).show_ui(ui, |ui| {
                        if ui.selectable_label(now.is_none(), tr!("Default")).clicked() {
                            pick = Some(None);
                        }
                        for v in &p.values {
                            if ui.selectable_label(now.as_deref() == Some(v.as_str()), v).clicked() {
                                pick = Some(Some(v.clone()));
                            }
                        }
                    });
                }
            });
            if let Some(v) = pick {
                match (v, props.iter().position(|(k, _)| *k == p.name)) {
                    (None, Some(i)) => {
                        props.remove(i);
                    }
                    (Some(v), Some(i)) => props[i].1 = v,
                    (Some(v), None) => props.push((p.name.clone(), v)),
                    (None, None) => {}
                }
                change = Some(Change::State(format(&id, &props)));
            }
        }
    });
    let shown = match &change {
        Some(Change::State(s)) => s.clone(),
        _ => state.to_owned(),
    };
    ui.add_space(2.0);
    ui.add(egui::Label::new(RichText::new(shown).monospace().size(11.0).color(pal.weak)).wrap());
    change
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn states_are_parsed_formatted_and_spread() {
        let (id, props) = parse("conquest:muddy_autumnal_forest_floor_layer[layers=8,waterlogged=false]");
        assert_eq!(id, "conquest:muddy_autumnal_forest_floor_layer");
        assert_eq!(props, vec![("layers".into(), "8".into()), ("waterlogged".into(), "false".into())]);
        assert_eq!(format(&id, &props), "conquest:muddy_autumnal_forest_floor_layer[layers=8,waterlogged=false]");
        assert_eq!(format("minecraft:poppy", &[]), "minecraft:poppy");
        let values: Vec<String> = ["north", "south"].map(String::from).to_vec();
        assert_eq!(spread("minecraft:pink_petals[flower_amount=2,facing=east]", "facing", &values), ["minecraft:pink_petals[flower_amount=2,facing=north]", "minecraft:pink_petals[flower_amount=2,facing=south]"]);
        assert_eq!(spread("minecraft:pink_petals", "facing", &values)[1], "minecraft:pink_petals[facing=south]");
    }
}
