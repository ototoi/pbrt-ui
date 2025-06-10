use crate::controllers::AppController;
use crate::controllers::RenderController;
use crate::controllers::render::RenderState;
use crate::controllers::render::image_receiver::ImageData as RenderImageData;
use crate::models::scene::FilmComponent;
use crate::models::scene::Node;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui::Vec2;

fn get_status_string(state: RenderState) -> String {
    match state {
        RenderState::Ready => "Ready to render.".to_string(),
        RenderState::Saving => "Saving scene to PBRT file...".to_string(),
        RenderState::Rendering => "Rendering scene...".to_string(),
        RenderState::Finishing => "Finishing render...".to_string(),
    }
}

fn get_render_size(node: &Arc<RwLock<Node>>) -> Vec2 {
    if let Some(camera_node) = Node::find_node_by_component::<FilmComponent>(node) {
        let camera_node = camera_node.read().unwrap();
        if let Some(c) = camera_node.get_component::<FilmComponent>() {
            let width = c.props.find_one_int("integer xresolution").unwrap_or(1280);
            let height = c.props.find_one_int("integer yresolution").unwrap_or(720);
            return Vec2::new(width as f32, height as f32);
        }
    }
    Vec2::new(1280.0, 720.0) // Default size if no FilmComponent is found
}

//todo:Should use gamma correction

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

fn create_image_delta(render_image: &RenderImageData) -> egui::epaint::ImageDelta {
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

pub struct RenderPanel {
    app_controller: Arc<RwLock<AppController>>,
    render_controller: RenderController,
    tiles: Vec<(usize, usize, usize, usize)>, // (x0, y0, x1, y1)
    texture_id: Option<egui::TextureId>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenderCommand {
    Render,
    Stop,
}

impl RenderPanel {
    pub fn new<'a>(
        _cc: &'a eframe::CreationContext<'a>,
        controller: &Arc<RwLock<AppController>>,
    ) -> Self {
        let config = controller.read().unwrap().get_config();
        Self {
            app_controller: controller.clone(),
            render_controller: RenderController::new(&config),
            tiles: Vec::new(),
            texture_id: None,
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        let _res = self.render_controller.update();
        let state = self.render_controller.get_state();
        let mut final_image_path = "".to_string();
        let mut render_command: Option<RenderCommand> = None;
        egui::TopBottomPanel::bottom("buttons").show_inside(ui, |ui| {
            ui.add_space(3.0);
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut final_image_path);
                if ui.button("Output Path").on_hover_text("Set the output path for the rendered image").clicked() {
                    log::info!("Output Path button clicked");
                    if let Some(path) = rfd::FileDialog::new()
                        .set_title("Select Output Path")
                        .set_file_name("rendered_image.png")
                        .save_file()
                    {
                        final_image_path = path.to_str().unwrap_or("").to_string();
                        //self.render_controller.set_output_path(final_image_path.clone());
                    }
                }
                match state {
                    RenderState::Ready => {
                        if ui.button("▶ Render").clicked() {
                            log::info!("Render button clicked");
                            render_command = Some(RenderCommand::Render);
                        }
                    }
                    RenderState::Saving => {
                        if ui.add_enabled(false, egui::Button::new("⏹ Stop")).clicked() {
                            log::info!("Stop button clicked");
                            //
                        }
                    }
                    RenderState::Rendering => {
                        if ui.button("⏹ Stop").clicked() {
                            log::info!("Stop button clicked");
                            render_command = Some(RenderCommand::Stop);
                        }
                    }
                    RenderState::Finishing => {
                        if ui.add_enabled(false, egui::Button::new("⏹ Stop")).clicked() {
                            log::info!("Stop button clicked");
                            // Here you can add the logic to render the scene
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.monospace(get_status_string(state));
            });
        });
        {
            if let Some(command) = render_command {
                match command {
                    RenderCommand::Render => {
                        self.tiles.clear();
                        let controller = self.app_controller.read().unwrap();
                        let root_node = controller.get_root_node();
                        {
                            let image_size = get_render_size(&root_node);
                            let image_size = [image_size.x as usize, image_size.y as usize];
                            let color_image =
                                egui::ColorImage::new(image_size, egui::Color32::BLACK);
                            let tex_manager = ui.ctx().tex_manager().clone();
                            let mut tex_manager = tex_manager.write();
                            let image_data = egui::ImageData::Color(Arc::new(color_image));
                            let options = egui::TextureOptions::LINEAR;
                            self.texture_id = Some(tex_manager.alloc(
                                "render_image".to_string(),
                                image_data,
                                options,
                            ));
                        }
                        match self.render_controller.render(&root_node) {
                            Ok(_) => {
                                log::info!("Render started");
                            }
                            Err(e) => {
                                log::error!("Render error: {}", e);
                            }
                        }
                    }
                    RenderCommand::Stop => {
                        log::info!("Stopping render");
                        match self.render_controller.cancel() {
                            Ok(_) => {
                                log::info!("Render cancelled");
                            }
                            Err(e) => {
                                log::error!("Render error: {}", e);
                            }
                        }
                    }
                }
            }
        }

        egui::CentralPanel::default().show_inside(ui, |ui| {
            let available_rect = ui.available_rect_before_wrap();
            let available_size = available_rect.size();
            let render_size = get_render_size(&self.app_controller.read().unwrap().get_root_node());
            let scale_x = available_size.x / render_size.x;
            let scale_y = available_size.y / render_size.y;
            let scale = scale_x.min(scale_y);
            let scaled_size = Vec2::new(render_size.x * scale, render_size.y * scale);
            let scaled_rect = egui::Rect::from_min_size(
                available_rect.min + (available_size - scaled_size) / 2.0,
                scaled_size,
            );

            {
                if let Some(image) = self.render_controller.get_image_data() {
                    let mut image = image.lock().unwrap();
                    if !image.tiles.is_empty() {
                        for tile in &image.tiles {
                            self.tiles.push(*tile);
                        }
                        image.tiles.clear();
                        if let Some(texture_id) = self.texture_id {
                            let image_delta = create_image_delta(&image);
                            let tex_manager = ui.ctx().tex_manager().clone();
                            let mut tex_manager = tex_manager.write();
                            tex_manager.set(texture_id, image_delta);
                        }
                    }
                }
            }

            ui.painter().rect_filled(
                available_rect.clone(),
                0.0,
                egui::Color32::from_rgb(0, 0, 64),
            );
            ui.painter()
                .rect_filled(scaled_rect, 0, egui::Color32::from_rgb(0, 0, 128));

            if let Some(texture_id) = self.texture_id {
                ui.painter().image(
                    texture_id,
                    scaled_rect,
                    egui::Rect::from_min_max(egui::Pos2::ZERO, egui::Pos2::new(1.0, 1.0)),
                    egui::Color32::WHITE,
                );
            }
            /*
            ui.painter().rect_stroke(
                scaled_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
            */
        });
    }
}
