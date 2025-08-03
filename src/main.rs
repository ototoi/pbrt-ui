#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example
#![allow(unsafe_code)]
#![allow(clippy::undocumented_unsafe_blocks)]

use std::sync::Arc;

use eframe::egui;
use eframe::wgpu;
use pbrt_ui::app::PbrtUIApp;

use uuid::Uuid;

fn get_wgpu_options() -> eframe::egui_wgpu::WgpuConfiguration {
    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetup::default();
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(create) = &mut wgpu_setup {
        create.device_descriptor = Arc::new(|adapter| {
            let base_limits = if adapter.get_info().backend == wgpu::Backend::Gl {
                wgpu::Limits::downlevel_webgl2_defaults()
            } else {
                wgpu::Limits::default()
            };
            wgpu::DeviceDescriptor {
                label: Some("egui wgpu device"),
                required_features: wgpu::Features::default() | wgpu::Features::POLYGON_MODE_LINE,
                required_limits: wgpu::Limits {
                    // When using a depth buffer, we have to be able to create a texture
                    // large enough for the entire surface, and we want to support 4k+ displays.
                    max_texture_dimension_2d: 8192,
                    max_uniform_buffer_binding_size: 4_294_967_292, // 4 GB
                    max_storage_buffer_binding_size: 4_294_967_292, // 4 GB
                    ..base_limits
                },
                ..Default::default()
            }
        });
    }
    let wgpu_options = eframe::egui_wgpu::WgpuConfiguration {
        wgpu_setup,
        ..Default::default()
    };
    return wgpu_options;
}

fn main() -> eframe::Result {
    //env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    //let window_size = [1920.0, 1080.0];
    let window_size = [1280.0, 720.0];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(window_size),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: get_wgpu_options(),
        //depth_buffer: 32, // Use a 24-bit depth buffer.
        ..Default::default()
    };
    let uuid = Uuid::new_v4(); // Generate a random UUID for the application.
    println!("Starting PBRT UI with UUID: {}", uuid);

    eframe::run_native(
        "PBRT UI",
        options,
        Box::new(|cc| Ok(Box::new(PbrtUIApp::new(cc)))),
    )
}
