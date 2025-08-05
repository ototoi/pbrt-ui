use crate::model::base::Matrix4x4;
use crate::model::base::Property;
use crate::model::base::Quaternion;
use crate::model::base::Vector3;
use crate::model::scene::CameraComponent;
use crate::model::scene::FilmComponent;
use crate::model::scene::Node;
use crate::model::scene::TransformComponent;
use crate::render::RenderMode;
use crate::render::ShadedRenderer;
use crate::render::SolidRenderer;
use crate::render::WireRenderer;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use egui::Vec2;

pub fn react_response(response: &egui::Response, root_node: &Arc<RwLock<Node>>) {
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
            let mut camera_node = camera_node.write().unwrap();
            if let Some(component) = camera_node.get_component_mut::<TransformComponent>() {
                let rotation_y = response.drag_motion().x * 0.01;
                let rotation_x = response.drag_motion().y * 0.01;
                {
                    let (t, r, s) = component.get_local_trs();
                    let m = r.to_matrix();
                    let upper = m.transform_vector(&Vector3::new(0.0, 1.0, 0.0)).normalize();
                    let right = m.transform_vector(&Vector3::new(1.0, 0.0, 0.0)).normalize();

                    //let upper = Vector3::new(0.0, 1.0, 0.0);
                    //let right = Vector3::new(1.0, 0.0, 0.0);

                    let rotation_y = if s.x < 0.0 { -rotation_y } else { rotation_y };
                    let rotation_x = if s.y < 0.0 { -rotation_x } else { rotation_x };

                    let rot_y = Quaternion::from_angle_axis(rotation_y, &upper);
                    let rot_x = Quaternion::from_angle_axis(rotation_x, &right);
                    let new_rotation = rot_x * rot_y * r;
                    component.set_local_trs(t, new_rotation, s);
                }
            }
        }
    }
}

pub struct SceneView {
    wireframe: Option<WireRenderer>,
    solid: Option<SolidRenderer>,
    shaded: Option<ShadedRenderer>,
}

impl SceneView {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let wireframe = WireRenderer::new(cc);
        let solid = SolidRenderer::new(cc);
        let shaded = ShadedRenderer::new(cc);
        Self {
            wireframe,
            solid,
            shaded,
        }
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

        let mut znear = 0.01f32;
        let mut zfar = 1000.0f32;
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
                    if let Some(prop) = camera.props.get("fov") {
                        if let Property::Floats(f) = prop {
                            if f.len() > 0 {
                                fov = f[0].to_radians();
                            }
                        }
                    }
                    if let Some(prop) = camera.props.get("znear") {
                        if let Property::Floats(f) = prop {
                            if f.len() > 0 && f[0] > 0.0 {
                                znear = f[0];
                            }
                        }
                    }
                    if let Some(prop) = camera.props.get("zfar") {
                        if let Property::Floats(f) = prop {
                            if f.len() > 0 && f[0] > znear {
                                zfar = f[0];
                            }
                        }
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
            react_response(&response, node);
        }

        let aspect = rect.width() / rect.height();
        let c2c = Matrix4x4::perspective(fov, aspect, znear, zfar);

        ui.painter().rect_filled(rect, 0.0, egui::Color32::BLACK);

        //let render_mode = RenderMode::Solid;
        match render_mode {
            RenderMode::Wire => {
                if let Some(renderer) = &mut self.wireframe {
                    renderer.render(ui, rect, node, &w2c, &c2c);
                }
            }
            RenderMode::Solid => {
                if let Some(renderer) = &mut self.solid {
                    renderer.render(ui, rect, node, &w2c, &c2c);
                }
            }
            RenderMode::Shaded => {
                if let Some(renderer) = &mut self.shaded {
                    renderer.render(ui, rect, node, &w2c, &c2c);
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
    }
}
