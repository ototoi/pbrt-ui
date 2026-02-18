use crate::model::base::Quaternion;
use crate::model::base::Vector3;
use crate::model::scene::CameraComponent;
use crate::model::scene::CoordinateSystemComponent;
use crate::model::scene::Node;
use crate::model::scene::TransformComponent;

use std::sync::Arc;
use std::sync::RwLock;

use eframe::egui;

pub trait CameraController {
    fn react_response(&mut self, response: &egui::Response, root_node: &Arc<RwLock<Node>>);
}

pub struct OrbitCameraController;

impl Default for OrbitCameraController {
    fn default() -> Self {
        Self
    }
}

const ORBIT_SPEED: f32 = 0.01;
const PAN_SPEED: f32 = 0.0025;
const DOLLY_DRAG_SPEED: f32 = 0.01;
const DOLLY_WHEEL_SPEED: f32 = 0.0015;

fn get_camera_axes(
    root_node: &Arc<RwLock<Node>>,
    rotation: &Quaternion,
) -> (Vector3, Vector3, Vector3) {
    let m = rotation.to_matrix();
    let mut up = m.transform_vector(&Vector3::new(0.0, 1.0, 0.0)).normalize();
    let mut right = m.transform_vector(&Vector3::new(1.0, 0.0, 0.0)).normalize();
    let mut forward = m.transform_vector(&Vector3::new(0.0, 0.0, 1.0)).normalize();

    let root_node = root_node.read().unwrap();
    if let Some(cs) = root_node.get_component::<CoordinateSystemComponent>() {
        up = cs.get_up_vector().normalize();
        forward = Vector3::cross(&right, &up).normalize(); //xy->z
        right = Vector3::cross(&up, &forward).normalize(); //yz->x
    }
    (up, right, forward)
}

impl CameraController for OrbitCameraController {
    fn react_response(&mut self, response: &egui::Response, root_node: &Arc<RwLock<Node>>) {
        if let Some(camera_node) = Node::find_node_by_component::<CameraComponent>(root_node) {
            let mut camera_node = camera_node.write().unwrap();
            if let Some(component) = camera_node.get_component_mut::<TransformComponent>() {
                let mut is_changed = false;
                let drag = response.drag_motion();
                let (mut t, mut r, s) = component.get_local_trs();
                let distance_scale = f32::max(t.length(), 1.0);

                if response.dragged_by(egui::PointerButton::Primary) {
                    let rotation_y = drag.x * ORBIT_SPEED;
                    let rotation_x = drag.y * ORBIT_SPEED;
                    let (up, right, _forward) = get_camera_axes(root_node, &r);

                    let rotation_y = if s.x < 0.0 { -rotation_y } else { rotation_y };
                    let rotation_x = if s.y < 0.0 { -rotation_x } else { rotation_x };

                    let rot_y = Quaternion::from_angle_axis(rotation_y, &up);
                    let rot_x = Quaternion::from_angle_axis(rotation_x, &right);
                    r = (rot_x * rot_y * r).normalize();
                    is_changed = true;
                }

                if response.dragged_by(egui::PointerButton::Secondary) {
                    let (up, right, _forward) = get_camera_axes(root_node, &r);
                    let tx = -drag.x * PAN_SPEED * distance_scale;
                    let ty = drag.y * PAN_SPEED * distance_scale;
                    t += right * tx + up * ty;
                    is_changed = true;
                }

                if response.dragged_by(egui::PointerButton::Middle) {
                    let (_up, _right, forward) = get_camera_axes(root_node, &r);
                    let tz = drag.y * DOLLY_DRAG_SPEED * distance_scale;
                    t += forward * tz;
                    is_changed = true;
                }

                if response.hovered() {
                    let scroll = response.ctx.input(|i| i.raw_scroll_delta.y);
                    if scroll != 0.0 {
                        let (_up, _right, forward) = get_camera_axes(root_node, &r);
                        let tz = -scroll * DOLLY_WHEEL_SPEED * distance_scale;
                        t += forward * tz;
                        is_changed = true;
                    }
                }

                if is_changed {
                    component.set_local_trs(t, r, s);
                }
            }
        }
    }
}
