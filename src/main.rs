#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example
#![allow(unsafe_code)]
#![allow(clippy::undocumented_unsafe_blocks)]

use eframe::egui;
use pbrt_ui::app::PbrtUIApp;

use uuid::Uuid;

fn main() -> eframe::Result {
    //env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    //let window_size = [1920.0, 1080.0];
    let window_size = [1280.0, 720.0];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(window_size),
        renderer: eframe::Renderer::Glow,
        depth_buffer: 32, // Use a 24-bit depth buffer.
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
