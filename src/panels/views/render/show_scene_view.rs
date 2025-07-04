use super::scene_view::RenderMode;
use super::scene_view::get_render_items;
use super::scene_view::{GizmoRenderItem, MeshRenderItem, RenderItem};
use crate::model::base::Quaternion;
use crate::model::base::Vector3;
use crate::model::scene::Node;
use crate::renderers::gl::RenderGizmo;
use crate::renderers::gl::RenderUniformValue;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;
use eframe::egui_glow;
use eframe::glow::HasContext;
use egui::Vec2;
use egui_glow::glow;
//use egui::Vec2;

use crate::model::base::Matrix4x4;
use crate::model::base::Property;
use crate::model::scene::CameraComponent;
use crate::model::scene::FilmComponent;

use crate::model::scene::TransformComponent; // Import LightRenderGizmo

fn render_mesh(
    gl: &glow::Context,
    w2c: &Matrix4x4,
    c2c: &Matrix4x4,
    item: &MeshRenderItem,
    mode: RenderMode,
) {
    unsafe {
        let material = &item.material;
        let program = &item.material.program;
        let program_handle = program.handle;

        let local_to_world = item.local_to_world;

        //gl.depth_mask(true);
        gl.use_program(Some(program_handle));
        gl.enable_vertex_attrib_array(0);

        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("local_to_world")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &local_to_world.m,
        );

        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("world_to_camera")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &w2c.m,
        );

        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("camera_to_clip")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &c2c.m,
        );

        for (key, value) in material.uniform_values.iter() {
            match value {
                RenderUniformValue::Vec4(v) => {
                    if let Some(location) = program.uniform_locations.get(key) {
                        let location = glow::NativeUniformLocation(*location);
                        gl.uniform_4_f32(Some(&location), v[0], v[1], v[2], v[3]);
                    }
                }
                _ => {}
            }
        }

        gl.bind_vertex_array(Some(item.mesh.vao));
        gl.draw_elements(glow::TRIANGLES, item.mesh.count, glow::UNSIGNED_INT, 0);
        gl.bind_vertex_array(None);

        gl.use_program(None);
    }
}

fn render_gizmo(
    gl: &glow::Context,
    w2c: &Matrix4x4,
    c2c: &Matrix4x4,
    item: &GizmoRenderItem,
    mode: RenderMode,
) {
    unsafe {
        let material = &item.material;
        let program = &item.material.program;
        let program_handle = program.handle;

        let local_to_world = item.local_to_world;

        //gl.depth_mask(false);
        gl.use_program(Some(program_handle));
        gl.enable_vertex_attrib_array(0);

        //let loc = gl.get_uniform_location(items[0].program.handle, "world_to_camera").unwrap().0;
        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("local_to_world")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &local_to_world.m,
        );

        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("world_to_camera")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &w2c.m,
        );

        gl.uniform_matrix_4_f32_slice(
            program
                .uniform_locations
                .get("camera_to_clip")
                .map(|&loc| glow::NativeUniformLocation(loc))
                .as_ref(),
            false,
            &c2c.m,
        );

        for (key, value) in material.uniform_values.iter() {
            match value {
                RenderUniformValue::Vec4(v) => {
                    if let Some(location) = program.uniform_locations.get(key) {
                        let location = glow::NativeUniformLocation(*location);
                        gl.uniform_4_f32(Some(&location), v[0], v[1], v[2], v[3]);
                    }
                }
                _ => {}
            }
        }

        match item.gizmo.as_ref() {
            RenderGizmo::Light(gizmo) => {
                for line in &gizmo.lines {
                    gl.bind_vertex_array(Some(line.vao));
                    gl.draw_elements(glow::LINE_STRIP, line.count, glow::UNSIGNED_INT, 0);
                }
                gl.bind_vertex_array(None);
            }
        }

        gl.use_program(None);
    }
}

fn render_wireframe(
    gl: &glow::Context,
    w2c: &Matrix4x4,
    c2c: &Matrix4x4,
    items: &[Arc<RenderItem>],
) {
    unsafe {
        gl.polygon_mode(glow::FRONT_AND_BACK, glow::LINE);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }
    for item in items.iter() {
        let item = item.as_ref();
        match item {
            RenderItem::Mesh(item) => {
                render_mesh(gl, w2c, c2c, item, RenderMode::Wireframe);
            }
            _ => {}
        }
    }
    //unsafe {
    //    gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
    //}
    for item in items.iter() {
        let item = item.as_ref();
        match item {
            RenderItem::Gizmo(item) => {
                render_gizmo(gl, w2c, c2c, item, RenderMode::Wireframe);
            }
            _ => {}
        }
    }
    unsafe {
        gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
    }
}

fn get_position(m: &Matrix4x4) -> [f32; 3] {
    [
        m.m[12], // x //4 * 3 + 0
        m.m[13], // y //4 * 3 + 1
        m.m[14], // z //4 * 3 + 2
    ]
}

fn sort_render_items(items: &[Arc<RenderItem>], w2c: &Matrix4x4) -> Vec<Arc<RenderItem>> {
    let mut index_depth = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            let item = item.as_ref();
            match item {
                RenderItem::Mesh(item) => (i, get_position(&item.local_to_world)),
                RenderItem::Gizmo(item) => (i, get_position(&item.local_to_world)),
            }
        })
        .map(|(i, pos)| {
            let pos = w2c.transform_point(&Vector3::new(pos[0], pos[1], pos[2]));
            (i, pos.z) // z coordinate in clip space
        })
        .collect::<Vec<_>>();
    index_depth.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));

    let sorted_items: Vec<Arc<RenderItem>> = index_depth
        .iter()
        .rev()
        .map(|(i, _)| items[*i].clone())
        .collect();
    return sorted_items;
}

fn render_solid(gl: &glow::Context, w2c: &Matrix4x4, c2c: &Matrix4x4, items: &[Arc<RenderItem>]) {
    let items = sort_render_items(items, w2c);
    unsafe {
        gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
        gl.enable(glow::DEPTH_TEST);
        gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);
    }

    for item in items.iter() {
        let item = item.as_ref();
        match item {
            RenderItem::Mesh(item) => {
                render_mesh(gl, w2c, c2c, item, RenderMode::Solid);
            }
            RenderItem::Gizmo(item) => {
                render_gizmo(gl, w2c, c2c, item, RenderMode::Solid);
            }
            _ => {}
        }
    }

    unsafe {
        gl.disable(glow::DEPTH_TEST);
        gl.polygon_mode(glow::FRONT_AND_BACK, glow::FILL);
    }
}

fn render(
    gl: &glow::Context,
    w2c: &Matrix4x4,
    c2c: &Matrix4x4,
    items: &[Arc<RenderItem>],
    mode: RenderMode,
) {
    match mode {
        RenderMode::Wireframe => {
            render_wireframe(gl, w2c, c2c, items);
        }
        RenderMode::Solid => {
            render_solid(gl, w2c, c2c, items);
        }
        RenderMode::Lighting => {
            //todo!("Implement lighting rendering");
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
    render_mode: RenderMode,
    is_playing: bool,
) {
    let available_rect = ui.available_rect_before_wrap();
    let available_size = available_rect.size();

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
            /*
            ui.painter()
                .rect_filled(scaled_rect, 0.0, egui::Color32::from_rgb(0, 0, 128));
            ui.painter().rect_stroke(
                scaled_rect,
                0.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::StrokeKind::Inside,
            );
            */

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

        let (rect, response) = ui.allocate_exact_size(available_size, egui::Sense::drag());
        if is_playing {
            react_response(&response, node);
        }

        let aspect = rect.width() / rect.height();
        let c2c = Matrix4x4::perspective(fov, aspect, 0.1, 1000.0);

        let render_items = get_render_items(gl, node, render_mode);

        if render_items.len() > 0 {
            let gl = gl.clone();
            let cb = egui_glow::CallbackFn::new(move |_info, _painter| {
                if render_items.len() > 0 {
                    render(&gl, &w2c, &c2c, &render_items, render_mode);
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
