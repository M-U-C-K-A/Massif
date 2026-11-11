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

/// The window icon: a snowy mountain on a blue tile, drawn procedurally.
fn icon() -> egui::IconData {
    const N: usize = 128;
    let mut rgba = vec![0u8; N * N * 4];
    let ridge = |x: f32| -> f32 {
        // Two peaks, in units of the icon size (0 = top).
        let a = 0.30 + (x - 0.38).abs() * 1.25;
        let b = 0.45 + (x - 0.70).abs() * 1.1;
        a.min(b)
    };
    for y in 0..N {
        for x in 0..N {
            let (fx, fy) = ((x as f32 + 0.5) / N as f32, (y as f32 + 0.5) / N as f32);
            // Rounded square mask.
            let (dx, dy) = ((fx - 0.5).abs() - 0.32, (fy - 0.5).abs() - 0.32);
            let outside = (dx.max(0.0).powi(2) + dy.max(0.0).powi(2)).sqrt() - 0.14;
            let alpha = (0.5 - outside * N as f32).clamp(0.0, 1.0);
            let sky = [
                (40.0 + 40.0 * fy) as u8,
                (110.0 + 60.0 * fy) as u8,
                (235.0 - 20.0 * fy) as u8,
            ];
            let r = ridge(fx);
            let color = if fy > r && fy < 0.86 {
                if fy < r + 0.09 {
                    [244, 248, 252]
                } else if fx < 0.5 {
                    [74, 140, 80]
                } else {
                    [58, 118, 70]
                }
            } else if fy >= 0.86 {
                [52, 104, 210]
            } else {
                sky
            };
            let i = (y * N + x) * 4;
            rgba[i..i + 3].copy_from_slice(&color);
            rgba[i + 3] = (alpha * 255.0) as u8;
        }
    }
    egui::IconData {
        rgba,
        width: N as u32,
        height: N as u32,
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
