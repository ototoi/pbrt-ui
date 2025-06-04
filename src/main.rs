#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")] // hide console window on Windows in release
#![allow(rustdoc::missing_crate_level_docs)] // it's an example
#![allow(unsafe_code)]
#![allow(clippy::undocumented_unsafe_blocks)]

use eframe::egui;
use pbrt_ui::app::PbrtUIApp;

fn main() -> eframe::Result {
    //env_logger::init(); // Log to stderr (if you run with `RUST_LOG=debug`).

    //let window_size = [1920.0, 1080.0];
    let window_size = [1280.0, 720.0];
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default().with_inner_size(window_size),
        renderer: eframe::Renderer::Glow,
        ..Default::default()
    };

    eframe::run_native(
        "PBRT UI",
        options,
        Box::new(|cc| Ok(Box::new(PbrtUIApp::new(cc)))),
    )
}
