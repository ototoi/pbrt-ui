use super::scene_view::RenderMode;
use super::scene_view::get_render_items;
use super::scene_view::{MeshRenderItem, RenderItem};
use crate::models::base::Quaternion;
use crate::models::scene::Node;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::HasContext;
use egui::Vec2;
use egui_glow::glow;
//use egui::Vec2;

use crate::models::base::Matrix4x4;
use crate::models::base::Property;
use crate::models::scene::CameraComponent;
use crate::models::scene::FilmComponent;
use crate::models::scene::TransformComponent;

fn render_mesh(gl: &glow::Context, w2c: &Matrix4x4, c2c: &Matrix4x4, item: &MeshRenderItem) {
    unsafe {
        let program = &item.material.program;
        let program_handle = program.handle;

        let local_to_world = item.local_to_world;

        gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
        gl.use_program(Some(program_handle));
        gl.enable_vertex_attrib_array(0);

        //let loc = gl.get_uniform_location(items[0].program.handle, "world_to_camera").unwrap().0;
        gl.uniform_matrix_4_f32_slice(
            gl.get_uniform_location(program_handle, "world_to_camera")
                .as_ref(),
            false,
            &w2c.m,
        );

        gl.uniform_matrix_4_f32_slice(
            gl.get_uniform_location(program_handle, "camera_to_clip")
                .as_ref(),
            false,
            &c2c.m,
        );

        gl.uniform_matrix_4_f32_slice(
            gl.get_uniform_location(program_handle, "local_to_world")
                .as_ref(),
            false,
            &local_to_world.m,
        );

        gl.bind_vertex_array(Some(item.mesh.vao));
        gl.draw_elements(glow::TRIANGLES, item.mesh.count, glow::UNSIGNED_INT, 0);
        gl.bind_vertex_array(None);

        gl.use_program(None);
        gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
    }
}

fn render(gl: &glow::Context, w2c: &Matrix4x4, c2c: &Matrix4x4, items: &[RenderItem]) {
    for item in items {
        match item {
            RenderItem::Mesh(item) => {
                render_mesh(gl, w2c, c2c, item);
            }
            RenderItem::Gizmo(item) => {
                // For light gizmos, we might not need to bind a VAO or EBO
                // but we can still set the local_to_world matrix
            }
            RenderItem::Manipulator(item) => {
                // For manipulator gizmos, we might not need to bind a VAO or EBO
                // but we can still set the local_to_world matrix
            }
        }
    }
    //todo!("Implement render");
}

pub fn react_response(response: &egui::Response, root_node: &Arc<RwLock<Node>>) {
    if response.dragged_by(egui::PointerButton::Primary) {
        if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(&root_node) {
            let mut camera_node = camera_node.write().unwrap();
            if let Some(component) = camera_node.get_component_mut::<TransformComponent>() {
                let rotation_y = -response.drag_motion().x * 0.01;
                let rotation_x = -response.drag_motion().y * 0.01;
                {
                    let (t, r, s) = component.get_local_trs(); //local_to_world
                    //let rotation = Quaternion::from_angle_axis(rotation_y, &Vector3::new(1.0, 0.0, 0.0)) * Quaternion::from_angle_axis(rotation_x, &Vector3::new(1.0, 0.0, 0.0));
                    let rotation =
                        Quaternion::from_euler_angles(rotation_x, rotation_y, 0.0).normalize();
                    let new_rotation = r * rotation;

                    component.set_local_trs(t, new_rotation, s);
                }
            }
        }
    }
}

pub fn show_scene_view(
    ui: &mut egui::Ui,
    gl: &Arc<glow::Context>,
    node: &Arc<RwLock<Node>>,
    is_playing: bool,
) {
    let available_rect = ui.available_rect_before_wrap();
    let available_size = available_rect.size();

    let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());
    if is_playing {
        react_response(&response, node);
    }

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

        {
            let scale_x = available_size.x / render_size.x;
            let scale_y = available_size.y / render_size.y;
            let scale = scale_x.min(scale_y);
            let scaled_size = Vec2::new(render_size.x * scale, render_size.y * scale);
            let scaled_rect = egui::Rect::from_min_size(
                available_rect.min + (available_size - scaled_size) / 2.0,
                scaled_size,
            );
            ui.painter()
                .rect_filled(scaled_rect, 0.0, egui::Color32::from_rgb(0, 0, 128));
            ui.painter().rect_stroke(
                scaled_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
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
            fov = vertical_fov;
        }

        let aspect = rect.width() / rect.height();
        let c2c = Matrix4x4::perspective(fov, aspect, 0.1, 1000.0);

        let render_items = get_render_items(gl, node, RenderMode::Wireframe); //todo

        if render_items.len() > 0 {
            let gl = gl.clone();
            let cb = egui_glow::CallbackFn::new(move |_info, _painter| {
                if render_items.len() > 0 {
                    render(&gl, &w2c, &c2c, &render_items);
                }
            });

            let callback = egui::PaintCallback {
                rect,
                callback: Arc::new(cb),
            };
            ui.painter().add(callback);
        }
    }
}
