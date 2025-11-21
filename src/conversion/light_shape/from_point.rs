use super::light_shape::LightShape;
use crate::model::base::Vector3;
use crate::model::scene::Light;

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

pub fn create_light_shape_from_point(light: &Light) -> Option<LightShape> {
    let axis_x = vec![Vector3::new(-1.0, 0.0, 0.0), Vector3::new(1.0, 0.0, 0.0)];
    let axis_y = vec![Vector3::new(0.0, -1.0, 0.0), Vector3::new(0.0, 1.0, 0.0)];
    let axis_z = vec![Vector3::new(0.0, 0.0, -1.0), Vector3::new(0.0, 0.0, 1.0)];

    let circle_x = create_circle_points(0, 16);
    let circle_y = create_circle_points(1, 16);
    let circle_z = create_circle_points(2, 16);

    let mut lines = Vec::new();
    lines.push(axis_x);
    lines.push(axis_y);
    lines.push(axis_z);
    lines.push(circle_x);
    lines.push(circle_y);
    lines.push(circle_z);

    let light_shape = LightShape { lines };
    return Some(light_shape);
}
