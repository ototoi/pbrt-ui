use std::sync::Arc;
use uuid::Uuid;

use image::DynamicImage;

//use eframe::egui;
use eframe::{egui_glow, glow::HasContext};
use egui_glow::glow;

#[derive(Debug, Clone)]
pub struct RenderTexture {
    pub id: Uuid,
    pub edition: String,
    pub texture: glow::Texture, //glow::NativeTexture,
    pub width: u32,
    pub height: u32,
    pub gl: Arc<glow::Context>,
}

impl RenderTexture {
    pub fn from_image(
        gl: &Arc<glow::Context>,
        id: Uuid,
        edition: &str,
        image: &DynamicImage
    ) -> Self {
        //
        todo!("Implement RenderTexture2D::new");
    }
}

impl Drop for RenderTexture {
    fn drop(&mut self) {
        unsafe {
            let gl = &self.gl;
            gl.delete_texture(self.texture);
        }
    }
}
