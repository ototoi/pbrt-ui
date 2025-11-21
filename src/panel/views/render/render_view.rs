use super::image_data::ImageData;
use super::render_history::RenderHistory;

use eframe::egui;
use std::sync::Arc;

#[inline]
pub fn gamma_correct(value: f32) -> f32 {
    if value <= 0.0031308 {
        return 12.92 * value;
    } else {
        return 1.055 * f32::powf(value, 1.0 / 2.4) - 0.055;
    }
}
/*
#[inline]
pub fn inverse_gamma_correct(value: f32) -> f32 {
    if value <= 0.04045 {
        return value * 1.0 / 12.92;
    } else {
        return f32::powf((value + 0.055) * 1.0 / 1.055, 2.4);
    }
}
*/

#[inline]
fn to_byte(a: f32) -> u8 {
    (gamma_correct(a) * 255.0).clamp(0.0, 255.0) as u8
}

fn create_image_delta(render_image: &ImageData) -> egui::epaint::ImageDelta {
    let width = render_image.width as usize;
    let height = render_image.height as usize;
    let mut pixels: Vec<egui::Color32> = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let index = y * width + x;
            let r = render_image.data[3 * index + 0];
            let g = render_image.data[3 * index + 1];
            let b = render_image.data[3 * index + 2];
            let pixel = egui::Color32::from_rgb(to_byte(r), to_byte(g), to_byte(b));
            pixels.push(pixel);
        }
    }
    let image = egui::ColorImage {
        size: [width, height],
        source_size: egui::Vec2::new(width as f32, height as f32),
        pixels,
    };
    let image = egui::ImageData::Color(Arc::new(image));
    let options = egui::TextureOptions::LINEAR;
    let delta = egui::epaint::ImageDelta {
        image,
        options,
        pos: None,
    };
    return delta;
}

fn show_render_view(ui: &mut egui::Ui, history: &mut RenderHistory) {
    let available_rect = ui.available_rect_before_wrap();
    let available_size = available_rect.size();
    if let Some(image) = history.get_image_data() {
        let mut image = image.lock().unwrap();
        let render_size = egui::vec2(image.width as f32, image.height as f32);
        let scale_x = available_size.x / render_size.x;
        let scale_y = available_size.y / render_size.y;
        let scale = scale_x.min(scale_y);
        let scaled_size = egui::vec2(render_size.x * scale, render_size.y * scale);
        let scaled_rect = egui::Rect::from_min_size(
            available_rect.min + (available_size - scaled_size) / 2.0,
            scaled_size,
        );
        {
            let tex_manager = ui.ctx().tex_manager().clone();
            let mut tex_manager = tex_manager.write();
            if history.texture_id.is_none() {
                let image_size = [render_size.x as usize, render_size.y as usize];
                let color_image = egui::ColorImage::filled(image_size, egui::Color32::BLACK);
                let image_data = egui::ImageData::Color(Arc::new(color_image));
                let options = egui::TextureOptions::LINEAR;
                history.texture_id =
                    Some(tex_manager.alloc("render_image".to_string(), image_data, options));
            }

            if let Some(texture_id) = history.texture_id {
                if !image.tiles.is_empty() {
                    image.tiles.clear();
                    let image_delta = create_image_delta(&image);
                    tex_manager.set(texture_id, image_delta);
                }

                ui.painter().image(
                    texture_id,
                    scaled_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
        }

        ui.painter().rect_stroke(
            scaled_rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );
    }
}

pub struct RenderView {}

impl RenderView {
    pub fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        Self {}
    }

    pub fn show(&mut self, ui: &mut egui::Ui, history: &mut RenderHistory) {
        show_render_view(ui, history);
    }
}
