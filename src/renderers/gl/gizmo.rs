use super::line::RenderLine;
use crate::models::scene::Light;

use std::sync::Arc;
use uuid::Uuid;

use eframe::egui_glow;
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct LightRenderGizmo {
    pub id: Uuid,
    pub edition: String,
    pub lines: Vec<Arc<RenderLine>>,
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

impl RenderGizmo {
    pub fn from_light_shape(gl: &Arc<glow::Context>, light: &Light) -> Option<Self> {
        let gl = gl.clone();
        let id = light.get_id();
        let edition = light
            .as_property_map()
            .find_one_string("edition")
            .unwrap_or("".to_string());
        if let Some(light_shape) = crate::models::scene::create_light_shape(light) {
            let mut lines = Vec::new();
            for line in light_shape.lines {
                let line: Vec<f32> = line.iter().flat_map(|v| vec![v.x, v.y, v.z]).collect();
                if let Some(line) = RenderLine::from_positions(&gl, &line) {
                    lines.push(Arc::new(line));
                }
            }
            return Some(RenderGizmo::Light(LightRenderGizmo {
                id,
                edition,
                lines,
                gl,
            }));
        }
        return None;
    }
}
