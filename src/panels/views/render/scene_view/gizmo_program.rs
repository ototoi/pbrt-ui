use crate::renderers::gl::RenderProgram;

use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

use eframe::egui_glow;
use eframe::glow;

pub fn create_gizmo_program(gl: &Arc<glow::Context>, id: Uuid) -> Option<Arc<RenderProgram>> {
    return None;
}
