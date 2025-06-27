use std::sync::Arc;
use uuid::Uuid;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderLines {
    pub id: Uuid,
    pub postions: glow::Buffer,
    pub indices: glow::Buffer,
    pub count: i32,
    pub vao: glow::VertexArray,
}

impl RenderLines {
    pub fn destroy(&self, gl: &Arc<glow::Context>) {
        unsafe {
            gl.delete_buffer(self.postions);
            gl.delete_buffer(self.indices);
            gl.delete_vertex_array(self.vao);
        }
    }
}
