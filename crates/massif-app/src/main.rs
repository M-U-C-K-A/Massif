//! Massif: a map editor for Minecraft worlds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[macro_use]
mod i18n;
mod inspect;
mod access;
mod analysis;
mod app;
mod autosave;
mod backdrop;
mod block_state;
mod blocks;
mod brushes;
mod cards;
mod caves;
mod collab;
mod devtools;
mod doc;
mod gradient_ui;
mod zones_ui;
mod graphics;
mod finder;
mod graph_editor;
mod help;
mod home;
mod icons;
mod icons_classic;
mod import_mask;
mod jobs;
mod keymap;
mod keys_page;
mod layers;
mod layout;
mod loader;
mod map_render;
mod map_view;
mod mc_import;
mod menu;
mod menubar;
mod notify;
mod object_list;
mod object_window;
mod ores;
mod pack_share;
mod packs;
mod palettes;
mod panels;
mod perf;
mod picker;
mod player;
mod quit;
mod real_terrain;
mod region_export;
mod report;
mod report_view;
mod render_side;
mod references;
mod diorama;
mod reshape;
mod snowfall_ui;
mod reticle;
mod run_script;
mod road_draft;
mod curve_draft;
mod select;
mod settings_backup;
#[cfg(test)]
mod memory_tests;
mod sounds;
mod textures;
mod tiles;
mod smooth_page;
mod startup;
// The Live preview of the new terrain generator on the World side's map;
// wired to the Terrain side when the new panel lands.
#[allow(dead_code)]
mod terrain_live;
mod terrain;
mod structure_tool;
mod terrain_view;
mod theme;
mod tools;
mod tour;
mod tree_sizes;
mod true_color;
mod update;
mod versions;
mod view3d;
mod screens;
mod view_image;
mod widgets;

use std::path::PathBuf;

/// The window icon: Massif's logo.
pub(crate) fn icon() -> egui::IconData {
    let image = image::load_from_memory(include_bytes!("../assets/icon.png"))
        .expect("the icon is a PNG")
        .into_rgba8();
    let (width, height) = image.dimensions();
    egui::IconData {
        rgba: image.into_raw(),
        width,
        height,
    }
}

fn main() -> eframe::Result {
    // Every step of the start written down, panics too (see `startup`).
    startup::begin();
    startup::log_warnings();
    // The scopes reach the performance recorder (idle until it records).
    perf::install();
    let open = std::env::args_os().nth(1).map(PathBuf::from);
    // Files the Finder asks to open, and the system's Quit (the Dock's, a
    // logout's), asked first when the map has changes; on Windows, `.massif`
    // files made Massif's, with their maps as their icons.
    startup::step("listening for the files the system asks to open, and its Quit");
    finder::listen();
    quit::listen();
    // In the registry, and Explorer told (which waits on Explorer): on a
    // thread of its own, the start never held up by it.
    startup::step("making .massif and .massifpack files Massif's (on a thread of its own)");
    let _ = std::thread::Builder::new().name("massif file types".into()).spawn(|| {
        let started = std::time::Instant::now();
        finder::register();
        startup::note(&format!(".massif and .massifpack files made Massif's, in {} ms", started.elapsed().as_millis()));
    });
    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Massif")
        .with_app_id("massif")
        .with_inner_size([1600.0, 1000.0])
        .with_min_inner_size([960.0, 640.0])
        .with_icon(icon());
    if cfg!(target_os = "macos") {
        // The traffic lights sit in Massif's own top bar, like in native apps.
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false);
    }
    // The 3D view keeps the map in texture arrays of 1024 × 1024 pages:
    // ask for as many layers as the GPU has (Apple's hold 2048).
    let mut setup = egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    // What WGPU_BACKEND names, or not a graphics API that failed to draw a
    // first frame on this computer.
    setup.instance_descriptor.backends = startup::backends();
    let base = setup.device_descriptor.clone();
    setup.device_descriptor = std::sync::Arc::new(move |adapter| {
        let mut desc = base(adapter);
        let can = adapter.limits();
        let want = &mut desc.required_limits;
        want.max_texture_array_layers = want.max_texture_array_layers.max(can.max_texture_array_layers.min(2048));
        // The Render side keeps a map's blocks in storage buffers, and the
        // terrain generator's simulations their grids (8192 × 8192 samples,
        // 256 MB): as large as the card binds, up to 2 GiB, 16 a stage.
        want.max_storage_buffer_binding_size = want.max_storage_buffer_binding_size.max(can.max_storage_buffer_binding_size.min(2 << 30));
        want.max_buffer_size = want.max_buffer_size.max(can.max_buffer_size.min(2 << 30));
        want.max_storage_buffers_per_shader_stage = want.max_storage_buffers_per_shader_stage.max(can.max_storage_buffers_per_shader_stage.min(16));
        desc
    });
    let wgpu_options = egui_wgpu::WgpuConfiguration::default();
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
            // A frame the window gives no image for, written down while
            // the start lasts.
            on_surface_status: startup::surface_status(wgpu_options.on_surface_status.clone()),
            ..wgpu_options
        },
        ..Default::default()
    };
    startup::step_in("opening the window and the graphics card (wgpu)", startup::Stage::Graphics);
    let result = eframe::run_native(
        "Massif",
        options,
        Box::new(|cc| {
            if let Some(render) = &cc.wgpu_render_state {
                startup::graphics_ready(render, cc);
            }
            startup::step("making the app");
            finder::wake(&cc.egui_ctx);
            quit::install(&cc.egui_ctx);
            // On macOS, File, Edit, View and Help in the menu bar at the
            // top of the screen too.
            menubar::install(&cc.egui_ctx);
            let app = app::MassifApp::new(cc, open);
            startup::step("app made: drawing the first frame");
            Ok(Box::new(app))
        }),
    );
    match &result {
        Ok(()) => startup::step("closed"),
        Err(error) => startup::failed(error),
    }
    startup::flush();
    result
}
