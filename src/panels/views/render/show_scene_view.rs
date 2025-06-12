use super::image_data::ImageData;
use super::render_history::RenderHistory;
use crate::models::scene::Node;
use crate::renderers::gl;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::HasContext;
use egui::Vec2;
use egui_glow::glow;
//use egui::Vec2;

use crate::controllers::AppController;
use crate::models::base::Matrix4x4;
use crate::models::base::Property;
use crate::models::base::PropertyMap;
use crate::models::base::Quaternion;
use crate::models::base::Vector3;
use crate::models::scene::CameraComponent;
use crate::models::scene::Component;
use crate::models::scene::CoordinateSystemComponent;
use crate::models::scene::FilmComponent;
use crate::models::scene::Material;
use crate::models::scene::MaterialComponent;
use crate::models::scene::Mesh;
use crate::models::scene::MeshComponent;

use crate::models::scene::ShapeComponent;
use crate::models::scene::SubdivComponent;
use crate::models::scene::TransformComponent;
use crate::renderers::gl::RenderMesh;
use crate::renderers::gl::RenderProgram;
use crate::renderers::gl::ResourceComponent;
use crate::renderers::gl::ResourceManager;


/* 
fn get_render_items(gl: &Arc<glow::Context>, node: &Arc<RwLock<Node>>) -> Vec<RenderMesh> {
    let mut render_items = Vec::new();
    let root_node = node.clone();
    if let Some(mesh_node) = Node::find_node_by_component::<MeshComponent>(&root_node) {
        let mesh_node = mesh_node.read().unwrap();
        if let Some(t) = mesh_node.get_component::<TransformComponent>() {
            let local_to_world = t.get_local_matrix();
            for mesh in mesh_node.get_components::<MeshComponent>() {
                if let Some(mesh) = mesh.read().unwrap().mesh.as_ref() {
                    render_items.push(RenderMesh::new(gl, mesh, local_to_world));
                }
            }
        }
    }
    render_items
}
*/



pub fn show_scene_view(
    gl: &Arc<glow::Context>,
    ui: &mut egui::Ui,
    history: &mut RenderHistory,
    node: &Arc<RwLock<Node>>,
) {
    let available_rect = ui.available_rect_before_wrap();
    let available_size = available_rect.size();

    let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());

    //let render_items =

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
    }
}
