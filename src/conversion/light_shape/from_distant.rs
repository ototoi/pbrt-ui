use super::light_shape::LightShape;
use crate::model::base::{PropertyMap, Vector3};

fn create_circle_points(axis: usize, div: usize) -> Vec<Vector3> {
    let mut points = Vec::new();
    let xx = ((axis + 1) % 3) as usize;
    let yy = ((axis + 2) % 3) as usize;
    let zz = axis as usize;
    for i in 0..=div {
        let angle = (i as f32 / div as f32) * std::f32::consts::PI * 2.0;
        let x = angle.cos();
        let y = angle.sin();
        let mut point = [0.0; 3];
        point[zz] = 0.0;
        point[xx] = x;
        point[yy] = y;
        points.push(Vector3::new(point[0], point[1], point[2]));
    }
    return points;
}

pub fn create_light_shape_from_distant(props: &PropertyMap) -> Option<LightShape> {
    let mut from = props.get_floats("from");
    if from.len() != 3 {
        from = vec![0.0, 0.0, 0.0];
    }
    let mut to = props.get_floats("to");
    if to.len() != 3 {
        to = vec![0.0, 0.0, 1.0];
    }
    let from = Vector3::new(from[0], from[1], from[2]);
    let to = Vector3::new(to[0], to[1], to[2]);
    //let dir = to - from;
    let mut lines = vec![];
    lines.push(vec![from, to]);
    lines.push(create_circle_points(2, 16));//todo:align direction of the circle with the direction of the light
    let shape = LightShape {
        lines: lines,
    };
    return Some(shape);
}
