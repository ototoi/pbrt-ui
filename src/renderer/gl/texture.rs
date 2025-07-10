use crate::model::scene::Texture;

use std::sync::Arc;
use uuid::Uuid;

use image::imageops::*;
use image::{DynamicImage, GenericImageView};

//use eframe::egui;
use eframe::{
    egui_glow,
    glow::{HasContext, PixelUnpackData},
};
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

fn get_wrap_mode(mode: &str) -> u32 {
    match mode {
        "clamp" => glow::CLAMP_TO_EDGE,
        "repeat" => glow::REPEAT,
        _ => glow::CLAMP_TO_EDGE, // Default to clamp to edge
    }
}

impl RenderTexture {
    pub fn from_image(
        gl: &Arc<glow::Context>,
        texture: &Texture,
        image: &DynamicImage,
    ) -> Option<Self> {
        let id = texture.get_id();
        let edition = texture.get_edition();
        let wrap_mode = get_wrap_mode(&texture.get_wrap());
        let (width, height) = image.dimensions();
        let image = image.to_rgba8();
        let image = flip_vertical(&image);
        unsafe {
            let texture = gl.create_texture().ok()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(texture));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA as i32,
                width as i32,
                height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                PixelUnpackData::Slice(Some(image.as_raw().as_slice())),
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );

            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, wrap_mode as i32);
            gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, wrap_mode as i32);
            gl.bind_texture(glow::TEXTURE_2D, None);
            return Some(Self {
                id,
                edition: edition.to_string(),
                texture,
                width,
                height,
                gl: gl.clone(),
            });
        }
    }

    pub fn get_id(&self) -> Uuid {
        self.id
    }
    pub fn get_edition(&self) -> String {
        self.edition.clone()
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
