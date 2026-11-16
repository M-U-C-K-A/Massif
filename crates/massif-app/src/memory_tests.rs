//! The memory a huge map takes through the app: opened, drawn in the map
//! and 3D views, painted over, saved and exported in part.
//!
//! `MASSIF_MEMORY_WORLD=<a large .massif> cargo test -p massif-app a_huge_map_within_its_memory -- --nocapture`
//! (a tester's map is 44,544 × 33,536 blocks: `massif-cli new <out> --size
//! 348x262 --landform mountains`). Prints the memory resident after each
//! step and the most it reached.

use crate::app::MassifApp;
use massif_core::{TILE_SIZE, TilePos};
use std::path::PathBuf;
use std::time::Instant;

/// The memory the process takes now, and the most it took, in MB: its
/// physical footprint, as the Activity Monitor shows it (what is
/// compressed counts; the resident size does not count it).
pub fn resident() -> (f64, f64) {
    #[cfg(target_os = "macos")]
    {
        // task_vm_info, up to ledger_phys_footprint_peak (revision 2).
        #[repr(C)]
        struct VmInfo {
            virtual_size: u64,
            region_count: i32,
            page_size: i32,
            before: [u64; 16],
            phys_footprint: u64,
            min_address: u64,
            max_address: u64,
            footprint_peak: i64,
            rest: [u64; 32],
        }
        unsafe extern "C" {
            fn mach_task_self() -> u32;
            fn task_info(task: u32, flavor: i32, info: *mut VmInfo, count: *mut u32) -> i32;
        }
        let mut info = VmInfo { virtual_size: 0, region_count: 0, page_size: 0, before: [0; 16], phys_footprint: 0, min_address: 0, max_address: 0, footprint_peak: 0, rest: [0; 32] };
        let mut count = (std::mem::size_of::<VmInfo>() / 4) as u32;
        // SAFETY: TASK_VM_INFO (22) fills the struct it is given, up to
        // `count` words.
        let ok = unsafe { task_info(mach_task_self(), 22, &mut info, &mut count) } == 0;
        if ok {
            return (info.phys_footprint as f64 / 1048576.0, info.footprint_peak as f64 / 1048576.0);
        }
    }
    (0.0, 0.0)
}

#[test]
fn a_huge_map_within_its_memory() {
    let Some(path) = std::env::var_os("MASSIF_MEMORY_WORLD").map(PathBuf::from) else {
        return;
    };
    let report = |what: &str, t: Instant| {
        let (now, most) = resident();
        println!("{what:<44} {now:>7.0} MB now, {most:>7.0} MB at most  ({:.1} s)", t.elapsed().as_secs_f64());
    };
    let t = Instant::now();
    report("start", t);
    let world = massif_format::load(&path).unwrap();
    let (lo, hi) = world.dims[0].bounds().unwrap();
    println!("{} × {} tiles, {} × {} blocks", hi.x - lo.x + 1, hi.z - lo.z + 1, (hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE);
    report("opened", t);

    let t = Instant::now();
    let mut h = egui_kittest::Harness::builder()
        .with_size(egui::vec2(1600.0, 1000.0))
        .with_accessibility_check(false)
        .wgpu()
        .build_eframe(move |cc| {
            crate::theme::install(&cc.egui_ctx);
            MassifApp::for_tests(world)
        });
    for _ in 0..40 {
        h.run_steps(5);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    report("shown in the map and 3D views", t);

    // Strokes of the Raise brush, 600 blocks wide, here and there over the
    // whole map: thousands of tiles changed, each stroke a step to undo.
    let t = Instant::now();
    {
        let app = h.state_mut();
        let mut tools = app.tools_for_tests().clone();
        tools.tool = crate::tools::Tool::Raise;
        tools.brush.radius = 300.0;
        let (w, d) = ((hi.x - lo.x + 1) * TILE_SIZE, (hi.z - lo.z + 1) * TILE_SIZE);
        let doc = app.doc_for_tests();
        for k in 0..120 {
            let x = (lo.x * TILE_SIZE) as f32 + ((k * 7919) % w) as f32;
            let z = (lo.z * TILE_SIZE) as f32 + ((k * 104_729) % d) as f32;
            let (Some(mut stroke), _) = crate::tools::begin_stroke(doc, &tools, (x, z), false, false) else { continue };
            for s in 1..20 {
                crate::tools::continue_stroke(doc, &tools, &mut stroke, (x + 40.0 * s as f32, z + 25.0 * s as f32), 0.05);
            }
            crate::tools::end_stroke(doc, stroke);
            doc.commit();
        }
    }
    for _ in 0..20 {
        h.run_steps(5);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    report("painted (120 strokes over the map)", t);

    let dir = std::env::temp_dir().join(format!("massif-memory-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let t = Instant::now();
    let snapshot = h.state_mut().doc_for_tests().world.snapshot();
    report("a save's snapshot taken", t);
    let stats = massif_format::save(&snapshot, &dir.join("huge.massif")).unwrap();
    drop(snapshot);
    report(&format!("saved ({} MB)", stats.file_len >> 20), t);
    let _ = std::fs::remove_file(dir.join("huge.massif"));

    // An export of a part of it (the whole would take tens of GB of disk):
    // the cost the size of the map adds is in its snapshot and context.
    let t = Instant::now();
    let snapshot = h.state_mut().doc_for_tests().world.snapshot();
    let options = massif_anvil::ExportOptions { area: Some((lo, TilePos::new(lo.x + 15, lo.z + 15))), overwrite: true, ..Default::default() };
    let stats = massif_anvil::export(&snapshot, &dir.join("export"), &options, &massif_anvil::Progress::default()).unwrap();
    drop(snapshot);
    report(&format!("exported 16 × 16 tiles ({} MB)", stats.bytes >> 20), t);
    let _ = std::fs::remove_dir_all(&dir);
}
