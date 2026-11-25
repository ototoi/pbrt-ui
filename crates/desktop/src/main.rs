//! pbrt-ui desktop entry point.
// No console window beside the GUI on Windows; `main` re-attaches to the
// terminal that started it, if any, so the endpoint line still prints.
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
mod application;
mod backend;
mod editor;
mod editor_assets;
mod mesh_view;
mod project_preview;
mod project_search;
mod project_tree;

fn main() {
    #[cfg(windows)]
    // SAFETY: plain Win32 call; failure is expected.
    unsafe {
        windows_sys::Win32::System::Console::AttachConsole(
            windows_sys::Win32::System::Console::ATTACH_PARENT_PROCESS,
        );
    }
    if let Err(error) = application::run() {
        eprintln!("pbrt-ui: {error}");
        std::process::exit(1);
    }
}
