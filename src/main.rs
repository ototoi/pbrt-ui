#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example
#![allow(unsafe_code)]
#![allow(clippy::undocumented_unsafe_blocks)]

use std::sync::Arc;

use eframe::egui;
use eframe::wgpu;
use pbrt_ui::app::PbrtUIApp;
use pbrt_ui::render::wgpu::copy_shaders;

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
                required_features: wgpu::Features::default()
                    | wgpu::Features::POLYGON_MODE_LINE
                    | wgpu::Features::all_webgpu_mask(),
                required_limits: wgpu::Limits {
                    // When using a depth buffer, we have to be able to create a texture
                    // large enough for the entire surface, and we want to support 4k+ displays.
                    max_texture_dimension_2d: 8192,
                    max_uniform_buffer_binding_size: 65536, // 64 KB
                    max_storage_buffer_binding_size: 2147483647, // 2 GB
                    max_buffer_size: 4294967292,            // 4 GB
                    min_uniform_buffer_offset_alignment: 256, // 256 bytes
                    min_storage_buffer_offset_alignment: 256, // 256 bytes
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

    // Copy shader files to cache directory
    match pbrt_ui::render::wgpu::copy_shaders::copy_shaders_to_cache() {
        Ok(path) => {
            println!("Shaders copied to cache: {:?}", path);
        }
        Err(e) => {
            eprintln!("Failed to copy shaders to cache: {}", e);
            // Continue execution even if shader copy fails
        }
    }

    let window_size = [1920.0, 1080.0];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(window_size),
        renderer: eframe::Renderer::Wgpu,
        wgpu_options: get_wgpu_options(),
        depth_buffer: 32, // Use a 32-bit depth buffer for better precision.
        ..Default::default()
    };
    eframe::run_native(
        "PBRT UI",
        options,
        Box::new(|cc| Ok(Box::new(PbrtUIApp::new(cc)))),
    )
}
