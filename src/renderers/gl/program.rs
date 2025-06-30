use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderProgram {
    pub id: Uuid,
    pub handle: glow::Program,

    pub uniform_locations: HashMap<String, u32>, //key, location
    pub vertex_locations: HashMap<String, u32>,  //key, location
    pub gl: Arc<glow::Context>,
}

impl Drop for RenderProgram {
    fn drop(&mut self) {
        unsafe {
            self.gl.delete_program(self.handle);
        }
    }
}
