//! Opening a large map: read from the file a part at a time (`load`)
//! against the whole file read first (`read`), best of three.
//!
//! `MASSIF_LOAD_WORLD=<a large .massif> cargo test --release -p massif-format --test load_perf -- --ignored --nocapture`

use std::time::Instant;

#[test]
#[ignore]
fn opening_a_large_map() {
    let Some(path) = std::env::var_os("MASSIF_LOAD_WORLD").map(std::path::PathBuf::from) else {
        return;
    };
    let mut best = [f64::MAX; 2];
    for _ in 0..3 {
        let t = Instant::now();
        drop(massif_format::load(&path).unwrap());
        best[0] = best[0].min(t.elapsed().as_secs_f64());
        let t = Instant::now();
        drop(massif_format::read(&std::fs::read(&path).unwrap()).unwrap());
        best[1] = best[1].min(t.elapsed().as_secs_f64());
    }
    eprintln!("load (a part at a time) {:.2} s, the whole file read first {:.2} s (best of 3)", best[0], best[1]);
}
