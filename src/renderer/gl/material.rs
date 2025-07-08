use super::program::RenderProgram;
use std::collections::HashMap;
use std::sync::Arc;

use uuid::Uuid;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub enum RenderUniformValue {
    Float(f32),
    Vec4([f32; 4]),
    Mat4([f32; 16]),
    Int(i32),
    Bool(bool),
    Texture(glow::Texture),
}

#[derive(Debug, Clone)]
pub struct RenderMaterial {
    pub id: Uuid,
    pub edition: String,
    pub program: Arc<RenderProgram>,
    pub uniform_values: Vec<(String, RenderUniformValue)>, //key, value
    pub gl: Arc<glow::Context>,
}

impl RenderMaterial {
    pub fn get_id(&self) -> Uuid {
        self.id
    }

    pub fn get_edition(&self) -> String {
        self.edition.clone()
    }
}

impl Drop for RenderMaterial {
    fn drop(&mut self) {
        //
    }
}
