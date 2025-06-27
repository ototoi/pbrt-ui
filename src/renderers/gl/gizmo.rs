use super::lines::RenderLines;

use std::sync::Arc;
use uuid::Uuid;

use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct LightRenderGizmo {
    pub id: Uuid,
    pub edition: String,
    pub lines: Vec<Arc<RenderLines>>,
    pub gl: Arc<glow::Context>,
}

#[derive(Debug, Clone)]
pub enum RenderGizmo {
    Light(LightRenderGizmo),
}

impl RenderGizmo {
    pub fn get_id(&self) -> Uuid {
        match self {
            RenderGizmo::Light(gizmo) => gizmo.id,
        }
    }
}

impl Drop for RenderGizmo {
    fn drop(&mut self) {
        match self {
            RenderGizmo::Light(gizmo) => {
                for line in &gizmo.lines {
                    line.destroy(&gizmo.gl);
                }
            }
        }
    }
}
