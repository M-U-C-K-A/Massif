//! Grow trees: the dialog opened from the object layers, its picture of
//! the tree, and the trees added to a new layer and to the one picked.
//! With MASSIF_SNAPSHOT_DIR, renders of the dialog for a few kinds.

use super::tests::harness_with;
use super::*;
use egui_kittest::kittest::Queryable;

fn ppp() -> f32 {
    std::env::var("MASSIF_SNAPSHOT_PPP").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0)
}

#[test]
fn grown_trees_go_to_a_new_layer_and_to_the_one_picked() {
    let mut h = harness_with(ThemeChoice::Dark, ppp());
    h.run_steps(3);
    let layers = h.state().doc.world.registry.object_layers.len();
    let objects = h.state().doc.world.registry.objects.len();
    h.state_mut().tools.grow_trees = true;
    h.run_steps(5);
    assert!(matches!(h.state().dialog, Some(Dialog::GrowTrees(_))), "the dialog opens");
    if let Some(dir) = std::env::var_os("MASSIF_SNAPSHOT_DIR").map(PathBuf::from) {
        let _ = std::fs::create_dir_all(&dir);
        for preset in [massif_core::treegen::TreePreset::Oak, massif_core::treegen::TreePreset::Willow, massif_core::treegen::TreePreset::Palm] {
            if let Some(Dialog::GrowTrees(f)) = &mut h.state_mut().dialog {
                f.preset = preset;
                f.params = preset.params();
            }
            h.run_steps(4);
            let name = preset.name().to_lowercase().replace(' ', "-");
            h.render().expect("rendered").save(dir.join(format!("grow-trees-{name}@{}x.png", ppp()))).unwrap();
        }
    }
    // The dialog fades in before it takes clicks.
    h.run_steps(20);
    h.get_by_label_contains("to a new layer").click();
    h.run_steps(4);
    assert!(h.state().dialog.is_none(), "closed once added");
    let reg = &h.state().doc.world.registry;
    assert_eq!(reg.object_layers.len(), layers + 1, "a new layer");
    // Six trees and six smaller ones for Tree sizes.
    assert_eq!(reg.objects.len(), objects + 12);
    let layer = reg.object_layers.last().unwrap();
    assert_eq!(layer.objects.iter().filter(|o| o.1 > 0).count(), 6);
    assert!(reg.objects[objects..].iter().all(|o| o.source == massif_core::treegen::SOURCE));
    assert_eq!(h.state().tools.object_layer as usize, reg.object_layers.len(), "the new layer is picked");
    // Again, into the layer picked.
    h.state_mut().tools.grow_trees = true;
    h.run_steps(20);
    h.get_by_label_contains("Add to ").click();
    h.run_steps(4);
    let reg = &h.state().doc.world.registry;
    assert_eq!(reg.object_layers.len(), layers + 1, "no other layer");
    assert_eq!(reg.object_layers.last().unwrap().objects.len(), 24);
}
