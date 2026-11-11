//! Massif: a map editor for Minecraft worlds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;
mod blocks;
mod brushes;
mod doc;
mod graph_editor;
mod jobs;
mod layout;
mod map_render;
mod map_view;
mod panels;
mod terrain_view;
mod theme;
mod tools;
mod view3d;

use std::path::PathBuf;

/// The window icon: Massif's logo.
fn icon() -> egui::IconData {
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
    let open = std::env::args_os().nth(1).map(PathBuf::from);
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
    let base = setup.device_descriptor.clone();
    setup.device_descriptor = std::sync::Arc::new(move |adapter| {
        let mut desc = base(adapter);
        desc.required_limits.max_texture_array_layers = desc
            .required_limits
            .max_texture_array_layers
            .max(adapter.limits().max_texture_array_layers.min(2048));
        desc
    });
    let options = eframe::NativeOptions {
        viewport,
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: egui_wgpu::WgpuConfiguration {
            wgpu_setup: egui_wgpu::WgpuSetup::CreateNew(setup),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        "Massif",
        options,
        Box::new(|cc| Ok(Box::new(app::MassifApp::new(cc, open)))),
    )
}
