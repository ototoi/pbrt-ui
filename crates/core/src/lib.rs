//! Single-owner application state shared by GUI and socket callers.
pub use protocol as proto;
pub mod preview_bindings;
mod project_io;
pub mod runtime;
pub mod services;
pub mod socket;
pub mod state;
pub mod view;
pub use runtime::{CoreHandle, CoreHost};
pub use state::{CoreState, LoadTask};

pub mod render_progress;

/// An absolute project path for unit tests on every platform.
#[cfg(test)]
pub(crate) const TEST_PROJECT: &str = if cfg!(windows) {
    r"C:\project"
} else {
    "/project"
};

mod import_prepare;
mod preview_timing;
