//! Live's latencies on a 16k terrain, on the graphics card when there is
//! one: `cargo test --release -p massif-terrain --test perf_live -- --ignored
//! --nocapture` (`MASSIF_LIVE_CPU=1` for the CPU alone).

use massif_terrain::engine::Engine;
use massif_terrain::gpu::Gpu;
use massif_terrain::kernel::NoEnv;
use massif_terrain::live::{Live, Mode, Settings};
use massif_terrain::model::Graph;
use massif_terrain::space::{Rect, Space};
use std::sync::Arc;
use std::time::{Duration, Instant};

const SIZE: u32 = 16384;

fn engine() -> Arc<Engine> {
    let gpu = if std::env::var("MASSIF_LIVE_CPU").is_ok_and(|v| !v.is_empty()) { None } else { Gpu::headless() };
    Arc::new(match gpu {
        Some(g) => Engine::with_gpu(g, Default::default()),
        None => Engine::new(Default::default()),
    })
}

/// A chain of point nodes, the base's height being the setting dragged.
fn points(height: f64) -> Graph {
    Graph::from_json(&format!(
        r#"{{ "nodes": {{
        "base": {{ "type": "noise.fbm", "params": {{ "scale": 3000, "height": {height} }} }},
        "detail": {{ "type": "noise.fbm", "params": {{ "scale": 150, "height": 12, "seed": 7 }} }},
        "mix": {{ "type": "combine.blend", "inputs": {{ "a": "base", "b": "detail" }} }},
        "lv": {{ "type": "adjust.levels", "inputs": {{ "in": "mix" }}, "params": {{ "in_high": 320, "out_high": 300 }} }},
        "out": {{ "type": "output.map", "inputs": {{ "height": "lv" }} }} }} }}"#
    ))
    .unwrap()
}

/// Mountains worn by thermal erosion, its angle being the setting dragged.
fn worn(angle: f64) -> Graph {
    Graph::from_json(&format!(
        r#"{{ "nodes": {{
        "hills": {{ "type": "noise.fbm", "params": {{ "scale": 2500, "height": 400 }} }},
        "worn": {{ "type": "erosion.thermal", "inputs": {{ "height": "hills" }}, "params": {{ "angle": {angle}, "iterations": 50 }} }},
        "out": {{ "type": "output.map", "inputs": {{ "height": "worn" }} }} }} }}"#
    ))
    .unwrap()
}

fn ms(d: Option<Duration>) -> f64 {
    d.map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0)
}

fn best_median(mut v: Vec<f64>) -> (f64, f64) {
    v.sort_by(f64::total_cmp);
    (v[0], v[v.len() / 2])
}

/// Sets versions one after the other, each made in full; gives each one's
/// time to its first tile in view, to the view complete, and to the end
/// (the graphics card done with it, for tiles left on the card).
fn versions(live: &Live, graph: impl Fn(usize) -> Graph, n: usize) -> (Vec<f64>, Vec<f64>, Vec<f64>) {
    let (mut first, mut visible, mut finished) = (Vec::new(), Vec::new(), Vec::new());
    for k in 0..n {
        let started = Instant::now();
        live.set_graph(&graph(k), Space::new(SIZE), Arc::new(NoEnv), k as u64 + 100);
        // Compiling counts: Live's times start once it is compiled.
        let compiled = Some(started.elapsed());
        assert!(live.wait_idle(Duration::from_secs(600)));
        if let Some(g) = live.engine().gpu() {
            g.wait();
        }
        let all = started.elapsed();
        let p = live.progress();
        assert!(p.error.is_none(), "{p:?}");
        live.tiles_ready();
        let plus = |d: Option<Duration>| d.zip(compiled).map(|(d, c)| d + c);
        first.push(ms(plus(p.first_visible)));
        visible.push(ms(plus(p.visible)));
        finished.push(ms(Some(all)));
    }
    (first, visible, finished)
}

fn report(what: &str, (first, visible, finished): (Vec<f64>, Vec<f64>, Vec<f64>)) {
    let (f, v, e) = (best_median(first), best_median(visible), best_median(finished));
    println!("{what:<58} first tile {:>7.1} ms (median {:>7.1}), view {:>7.1} ms ({:>7.1}), all {:>8.1} ms ({:>8.1})", f.0, f.1, v.0, v.1, e.0, e.1);
}

#[test]
#[ignore]
fn live_on_a_16k_terrain() {
    let engine = engine();
    println!("Live on {} — terrain of {SIZE} blocks", engine.gpu_name().unwrap_or("the CPU"));
    // Tiles read back to the CPU: the time to their samples, complete.
    let base = Settings { rest: Duration::from_millis(300), ..Default::default() };

    // The whole terrain in a 1600 × 1000 view: 16.4 blocks a pixel, level 4.
    let whole_view = (Rect::new(-4096, 0, 26214, 16384), 16.4);
    // A close view at 2 blocks a pixel: level 1, 3200 × 2000 blocks.
    let close_view = (Rect::new(6592, 7192, 3200, 2000), 2.0);
    // Each case's versions of its own, none found in the cache.
    for (case, (name, view)) in [("whole terrain in view (level 4)", whole_view), ("close view (level 1)", close_view)].into_iter().enumerate() {
        let live = Live::new(engine.clone(), Settings { rest: Duration::ZERO, ..base.clone() });
        live.set_view(view.0, view.1);
        report(&format!("points, {name}"), versions(&live, |k| points(250.0 + case as f64 + 10.0 * k as f64), 6));
    }
    if engine.gpu().is_some() {
        // Left on the card for a view drawing from it: the times are those
        // of the work given to the card, which finishes it before drawing.
        let live = Live::new(engine.clone(), Settings { rest: Duration::ZERO, on_gpu: true, ..base.clone() });
        live.set_view(close_view.0, close_view.1);
        report("points, close view, tiles left on the card", versions(&live, |k| points(252.0 + 10.0 * k as f64), 6));
    }

    // A slider dragged: a version every 33 ms, none waited for.
    let live = Live::new(engine.clone(), base.clone());
    live.set_view(close_view.0, close_view.1);
    let mut firsts = Vec::new();
    for k in 0..30 {
        let started = Instant::now();
        live.set_graph(&points(400.0 + 3.0 * k as f64), Space::new(SIZE), Arc::new(NoEnv), k);
        while started.elapsed() < Duration::from_millis(33) {
            std::thread::sleep(Duration::from_millis(1));
        }
        let p = live.progress();
        if p.revision == k
            && let Some(f) = p.first_visible
        {
            firsts.push(ms(Some(f)));
        }
    }
    assert!(live.wait_idle(Duration::from_secs(600)));
    let shown = firsts.len();
    let (b, m) = if firsts.is_empty() { (f64::NAN, f64::NAN) } else { best_median(firsts) };
    println!("points, slider dragged (a version every 33 ms, close view)    {shown}/30 versions shown within 33 ms, first tile {b:.1} ms (median {m:.1})");

    // The rest of the terrain at the close view's level (8192 samples a side).
    let live = Live::new(engine.clone(), Settings { mode: Mode::Whole, rest: Duration::ZERO, ..base.clone() });
    live.set_view(close_view.0, close_view.1);
    report("points, whole mode: the terrain at level 1 (1024 tiles)", versions(&live, |k| points(303.0 + 10.0 * k as f64), 2));

    // Erosion: quick at 1024 samples, then exact at 2048 once rested.
    let live = Live::new(engine.clone(), Settings { rest: Duration::ZERO, ..base.clone() });
    live.set_view(whole_view.0, whole_view.1);
    report("thermal erosion, whole terrain (quick 1024, exact 2048)", versions(&live, |k| worn(30.0 + k as f64), 3));
    let c = engine.counters.by_node.lock().unwrap().get("worn").copied();
    println!("  erosion's tiles and time: {c:?}; simulations: {} on the card, {} in all", engine.counters.gpu_globals.load(std::sync::atomic::Ordering::Relaxed), engine.counters.globals.load(std::sync::atomic::Ordering::Relaxed));
}
