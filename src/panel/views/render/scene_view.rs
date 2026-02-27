use super::camera_controller::CameraController;
use super::camera_controller::OrbitCameraController;
use super::fps_counter::FpsCounter;
use crate::model::base::Matrix4x4;
use crate::model::base::Property;
use crate::model::scene::CameraComponent;
use crate::model::scene::FilmComponent;
use crate::model::scene::Node;
use crate::model::scene::TransformComponent;
use crate::render::LightingRenderer;
use crate::render::RenderMode;
use crate::render::SolidRenderer;
use crate::render::WireRenderer;
use crate::render::wgpu::camera::RenderCamera;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use egui::Vec2;

pub struct SceneView {
    wireframe: Option<WireRenderer>,
    solid: Option<SolidRenderer>,
    shaded: Option<LightingRenderer>,
    fps_counter: FpsCounter,
    camera_controller: Box<dyn CameraController>,
}

impl SceneView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let wireframe = WireRenderer::new(cc);
        let solid = SolidRenderer::new(cc);
        let shaded = LightingRenderer::new(cc);
        Self {
            wireframe,
            solid,
            shaded,
            fps_counter: FpsCounter::new(),
            camera_controller: Box::new(OrbitCameraController),
        }
    }

    pub fn set_camera_controller(&mut self, controller: Box<dyn CameraController>) {
        self.camera_controller = controller;
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        node: &Arc<RwLock<Node>>,
        render_mode: RenderMode,
        is_playing: bool,
    ) {
        let available_rect = ui.available_rect_before_wrap();
        let available_size = available_rect.size();
        //let camera_props = CameraProperties::get_instance().get_entries("perspective");

        let mut znear = 1.0f32;
        let mut zfar = 10000.0f32;
        let mut fov = 90.0f32.to_radians();
        let mut w2c = Matrix4x4::identity();
        let mut render_size = Vec2::new(1280.0, 720.0);
        {
            let root_node = node.clone();
            if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
                let camera_node = camera_node.read().unwrap();
                if let Some(t) = camera_node.get_component::<TransformComponent>() {
                    let local_to_world = t.get_local_matrix();
                    w2c = local_to_world.inverse().unwrap();
                }
                if let Some(camera) = camera_node.get_component::<CameraComponent>() {
                    if let Some(prop) = camera.props.get("fov")
                        && let Property::Floats(f) = prop
                        && !f.is_empty()
                    {
                        fov = f[0].to_radians();
                    }
                    if let Some(prop) = camera.props.get("znear")
                        && let Property::Floats(f) = prop
                        && !f.is_empty()
                        && f[0] > 0.0
                    {
                        znear = f[0];
                    }
                    if let Some(prop) = camera.props.get("zfar")
                        && let Property::Floats(f) = prop
                        && !f.is_empty()
                        && f[0] > znear
                    {
                        zfar = f[0];
                    }
                }
                if let Some(film) = camera_node.get_component::<FilmComponent>() {
                    let width = film
                        .props
                        .find_one_int("integer xresolution")
                        .unwrap_or(1280);
                    let height = film
                        .props
                        .find_one_int("integer yresolution")
                        .unwrap_or(720);
                    render_size = Vec2::new(width as f32, height as f32);
                }
            }
        }

        let scale_x = available_size.x / render_size.x;
        let scale_y = available_size.y / render_size.y;
        let scale = scale_x.min(scale_y);
        let scaled_size = Vec2::new(render_size.x * scale, render_size.y * scale);
        let scaled_rect = egui::Rect::from_min_size(
            available_rect.min + (available_size - scaled_size) / 2.0,
            scaled_size,
        );

        let vertical_fov = if scaled_size.x < scaled_size.y {
            // portrait mode
            let k = scaled_size.x / (fov / 2.0).tan(); //tan = y / x
            2.0 * f32::atan2(available_size.y, k)
        } else {
            // landscape mode
            let k = scaled_size.y / (fov / 2.0).tan(); //tan = y / x
            2.0 * f32::atan2(available_size.y, k)
        };
        let fov = vertical_fov;

        let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());
        if is_playing {
            self.camera_controller.react_response(&response, node);
        }

        let aspect = rect.width() / rect.height();
        let c2c = Matrix4x4::OPENGL_TO_WGPU_CLIP * Matrix4x4::perspective(fov, aspect, znear, zfar);
        let render_camera = RenderCamera::from_perspective(
            glam::Mat4::from(&w2c),
            glam::Mat4::from(&c2c),
            znear,
            zfar,
            fov,
            aspect,
        );


        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);

        //let render_mode = RenderMode::Solid;
        match render_mode {
            RenderMode::Wire => {
                if let Some(renderer) = &mut self.wireframe {
                    renderer.render(ui, rect, node, &render_camera);
                }
            }
            RenderMode::Solid => {
                if let Some(renderer) = &mut self.solid {
                    renderer.render(ui, rect, node, &render_camera);
                }
            }
            RenderMode::Lighting => {
                if let Some(renderer) = &mut self.shaded {
                    renderer.render(ui, rect, node, &render_camera);
                }
            }
            _ => {
                ui.label("Unsupported render mode");
            }
        }
        ui.painter().rect_stroke(
            scaled_rect,
            0.0,
            egui::Stroke::new(1.0, egui::Color32::WHITE),
            egui::StrokeKind::Inside,
        );

        let show_fps = true; //TODO: add option to show/hide fps
        if show_fps {
            self.fps_counter.update();
            let fps = self.fps_counter.get_fps();
            ui.painter().text(
                rect.right_top() + egui::vec2(-5.0, 5.0),
                egui::Align2::RIGHT_TOP,
                format!("FPS: {:.1}", fps),
                egui::FontId::monospace(16.0),
                egui::Color32::GREEN,
            );
        }

        ui.painter().text(
            rect.left_bottom() + egui::vec2(8.0, -8.0),
            egui::Align2::LEFT_BOTTOM,
            "LMB Orbit | RMB Pan | MMB/Wheel Dolly",
            egui::FontId::monospace(12.0),
            egui::Color32::LIGHT_GRAY,
        );
    }
}
