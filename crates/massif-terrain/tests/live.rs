//! Live: the view first, nearest its centre; versions replacing each other;
//! quick simulations, then exact ones once the graph rests; the whole
//! terrain in the Whole mode.

use massif_terrain::compile::compile;
use massif_terrain::engine::{Engine, never};
use massif_terrain::kernel::NoEnv;
use massif_terrain::live::{Live, Mode, Phase, Ready, Settings, view_lod};
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space, TileKey};
use std::collections::HashSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

fn hills(height: f64) -> Graph {
    Graph::from_json(&format!(
        r#"{{ "nodes": {{
        "hills": {{ "type": "noise.fbm", "params": {{ "scale": 400, "height": {height} }} }},
        "out": {{ "type": "output.map", "inputs": {{ "height": "hills" }} }} }} }}"#
    ))
    .unwrap()
}

fn worn() -> Graph {
    Graph::from_json(
        r#"{ "nodes": {
        "hills": { "type": "noise.fbm", "params": { "scale": 300, "height": 200 } },
        "worn": { "type": "erosion.thermal", "inputs": { "height": "hills" }, "params": { "angle": 25, "iterations": 20 } },
        "out": { "type": "output.map", "inputs": { "height": "worn" } } } }"#,
    )
    .unwrap()
}

fn live(settings: Settings) -> Live {
    Live::new(Arc::new(Engine::new(Default::default())), settings)
}

fn idle(live: &Live) -> Vec<Ready> {
    assert!(live.wait_idle(Duration::from_secs(120)), "Live never rested: {:?}", live.progress());
    live.tiles_ready()
}

/// The tile as an engine of its own makes it, from the graph compiled with
/// simulations at `sim_res`.
fn expected(graph: &Graph, size: u32, sim_res: u32, key: TileKey) -> Vec<f32> {
    let (c, _) = compile(graph, Space::new(size), sim_res);
    let e = Engine::new(Default::default());
    let t = e.tiles(&c, c.output("height").unwrap(), key.lod, &[(key.x, key.z)], &never).unwrap();
    t[0].values().unwrap().to_vec()
}

#[test]
fn levels_of_a_view() {
    assert_eq!(view_lod(0.25), 0);
    assert_eq!(view_lod(1.0), 0);
    assert_eq!(view_lod(1.9), 0);
    assert_eq!(view_lod(2.0), 1);
    assert_eq!(view_lod(16.4), 4);
}

#[test]
fn the_view_comes_first_nearest_its_centre() {
    let live = live(Settings::default());
    let repaints = Arc::new(AtomicUsize::new(0));
    let r = repaints.clone();
    live.on_ready(move || {
        r.fetch_add(1, Ordering::Relaxed);
    });
    // 1024 × 512 blocks at a block a pixel: tiles 4 to 7 by 4 to 5 at level 0.
    live.set_view(Rect::new(1024, 1024, 1024, 512), 1.0);
    let graph = hills(100.0);
    assert!(live.set_graph(&graph, Space::new(4096), Arc::new(NoEnv), 7).is_empty());
    let tiles = idle(&live);
    assert!(repaints.load(Ordering::Relaxed) > 0);
    let visible: Vec<TileKey> = tiles.iter().filter(|t| t.phase == Phase::Visible).map(|t| t.key).collect();
    assert_eq!(visible.len(), 8);
    let central: HashSet<TileKey> = [(5, 4), (6, 4), (5, 5), (6, 5)].iter().map(|&(x, z)| TileKey::new(0, x, z)).collect();
    assert_eq!(visible[..4].iter().copied().collect::<HashSet<_>>(), central);
    // Then the overview: 512 samples a side, level 3, its four tiles.
    let overview: HashSet<TileKey> = tiles.iter().filter(|t| t.phase == Phase::Overview).map(|t| t.key).collect();
    assert_eq!(overview, (0..4).map(|k| TileKey::new(3, k % 2, k / 2)).collect());
    assert!(tiles[..8].iter().all(|t| t.phase == Phase::Visible), "the view before the overview");
    assert!(tiles.iter().all(|t| t.revision == 7 && t.exact && t.output == 0));
    // No rest of the terrain in this mode.
    assert!(tiles.iter().all(|t| t.phase != Phase::Rest));
    // The same samples as an engine of its own.
    let t = &tiles[0];
    assert_eq!(t.tile.values().unwrap(), &expected(&graph, 4096, 2048, t.key)[..]);
    let p = live.progress();
    assert!(p.idle && p.error.is_none() && p.revision == 7);
    let (first, visible, finished) = (p.first_visible.unwrap(), p.visible.unwrap(), p.finished.unwrap());
    assert!(first <= visible && visible <= finished, "{p:?}");
    // Moving the view makes what it shows next; what was made stays.
    live.set_view(Rect::new(0, 0, 512, 512), 1.0);
    let more = idle(&live);
    let keys: HashSet<TileKey> = more.iter().map(|t| t.key).collect();
    assert_eq!(keys, (0..4).map(|k| TileKey::new(0, k % 2, k / 2)).collect());
}

#[test]
fn a_new_version_replaces_the_old() {
    let live = live(Settings::default());
    live.set_view(Rect::new(0, 0, 2048, 2048), 4.0);
    let (a, b) = (hills(100.0), hills(160.0));
    live.set_graph(&a, Space::new(2048), Arc::new(NoEnv), 1);
    live.set_graph(&b, Space::new(2048), Arc::new(NoEnv), 2);
    let tiles = idle(&live);
    // Whatever of the first was made came before the second.
    let first_b = tiles.iter().position(|t| t.revision == 2).expect("the second version's tiles");
    assert!(tiles[first_b..].iter().all(|t| t.revision == 2));
    let shown: HashSet<TileKey> = tiles[first_b..].iter().filter(|t| t.phase == Phase::Visible).map(|t| t.key).collect();
    assert_eq!(shown, (0..4).map(|k| TileKey::new(2, k % 2, k / 2)).collect());
    let t = &tiles[first_b];
    assert_eq!(t.tile.values().unwrap(), &expected(&b, 2048, 2048, t.key)[..]);
    // Going back is found in the cache.
    let before = live.engine().counters.tiles();
    live.set_graph(&a, Space::new(2048), Arc::new(NoEnv), 3);
    let back = idle(&live);
    assert!(back.iter().all(|t| t.revision == 3) && !back.is_empty());
    assert!(live.engine().counters.tiles() - before <= back.len() as u64);
}

#[test]
fn simulations_run_quick_then_exact_once_the_graph_rests() {
    let settings = Settings { sim_res: 512, quick_sim_res: 128, rest: Duration::from_millis(150), ..Default::default() };
    let live = live(settings);
    // The whole 2048-block terrain at 4 blocks a pixel: level 2, 2 × 2 tiles.
    live.set_view(Rect::new(0, 0, 2048, 2048), 4.0);
    let graph = worn();
    live.set_graph(&graph, Space::new(2048), Arc::new(NoEnv), 1);
    let tiles = idle(&live);
    let quick: Vec<&Ready> = tiles.iter().filter(|t| t.phase == Phase::Visible && !t.exact).collect();
    let exact: Vec<&Ready> = tiles.iter().filter(|t| t.phase == Phase::Visible && t.exact).collect();
    assert_eq!((quick.len(), exact.len()), (4, 4));
    let last_quick = tiles.iter().rposition(|t| !t.exact).unwrap();
    let first_exact = tiles.iter().position(|t| t.exact).unwrap();
    assert!(last_quick < first_exact, "quick first, then exact");
    for t in &exact {
        assert_eq!(t.tile.values().unwrap(), &expected(&graph, 2048, 512, t.key)[..]);
    }
    let q = quick.iter().find(|q| q.key == exact[0].key).unwrap();
    assert_ne!(q.tile.values().unwrap(), exact[0].tile.values().unwrap(), "simulations at two resolutions differ");
    assert!(live.progress().finished.unwrap() >= Duration::from_millis(150));
}

#[test]
fn the_whole_mode_fills_the_terrain() {
    let settings = Settings { mode: Mode::Whole, whole_res: 1024, rest: Duration::ZERO, ..Default::default() };
    let live = live(settings);
    live.set_view(Rect::new(3584, 3584, 512, 512), 1.0);
    live.set_graph(&hills(80.0), Space::new(4096), Arc::new(NoEnv), 1);
    let tiles = idle(&live);
    // The rest: at most 1024 samples a side, level 2, 4 × 4 tiles, each once,
    // from the view's corner outwards.
    let rest: Vec<TileKey> = tiles.iter().filter(|t| t.phase == Phase::Rest).map(|t| t.key).collect();
    assert_eq!(rest.len(), 16);
    assert_eq!(rest.iter().collect::<HashSet<_>>().len(), 16);
    assert!(rest.iter().all(|k| k.lod == 2));
    assert_eq!(rest[0], TileKey::new(2, 3, 3));
    assert_eq!(*rest.last().unwrap(), TileKey::new(2, 0, 0));
}

#[test]
fn off_and_paused_make_nothing() {
    let live = live(Settings { mode: Mode::Off, ..Default::default() });
    live.set_view(Rect::new(0, 0, 1024, 1024), 2.0);
    live.set_graph(&hills(100.0), Space::new(1024), Arc::new(NoEnv), 1);
    assert!(idle(&live).is_empty());
    live.set_paused(true);
    live.set_settings(Settings::default());
    assert!(idle(&live).is_empty());
    live.set_paused(false);
    assert!(!idle(&live).is_empty());
}

#[test]
fn a_graph_that_cannot_be_made_says_why() {
    let live = live(Settings::default());
    live.set_view(Rect::new(0, 0, 1024, 1024), 2.0);
    let broken = Graph::from_json(
        r#"{ "nodes": {
        "lv": { "type": "adjust.levels" },
        "out": { "type": "output.map", "inputs": { "height": "lv" } } } }"#,
    )
    .unwrap();
    let diags = live.set_graph(&broken, Space::new(1024), Arc::new(NoEnv), 1);
    assert!(!diags.is_empty());
    assert!(idle(&live).is_empty());
    let why = live.progress().error.expect("an error");
    assert!(why.contains("lv"), "{why}");
}
