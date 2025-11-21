use include_dir::{include_dir, Dir};

/// Embedded shaders directory
pub static SHADERS: Dir = include_dir!("$CARGO_MANIFEST_DIR/src/render/wgpu/shaders");
