use super::light_shape::LightShape;
use crate::models::base::{PropertyMap, Vector3};

pub fn create_light_shape_from_point(_props: &PropertyMap) -> Option<LightShape> {
    let axis_x: Vec<Vector3> = vec![Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)];
    let axis_y = vec![Vector3::new(0.0, -1.0, 0.0), Vector3::new(0.0, 1.0, 0.0)];
    let axis_z = vec![Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, 0.0, 1.0)];

    let lines = vec![axis_x, axis_y, axis_z];
    let light_shape = LightShape { lines };
    return Some(light_shape);
}
