use super::light_shape::LightShape;
use crate::model::base::{PropertyMap, Vector3};

fn create_circle_points(radius: f32, div: usize) -> Vec<Vector3> {
    let mut points = Vec::new();
    for i in 0..=div {
        let angle = (i as f32 / div as f32) * std::f32::consts::PI * 2.0;
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;
        let mut point = [0.0; 3];
        point[2] = 1.0;
        point[0] = x;
        point[1] = y;
        points.push(Vector3::new(point[0], point[1], point[2]));
    }
    return points;
}

fn create_spot_lines(radius: f32, div: usize) -> Vec<Vec<Vector3>> {
    let mut lines = Vec::new();
    for i in 0..div {
        let angle = (i as f32 / div as f32) * std::f32::consts::PI * 2.0;
        let x = angle.cos() * radius;
        let y = angle.sin() * radius;
        let line = vec![Vector3::new(0.0, 0.0, 0.0), Vector3::new(x, y, 1.0)];
        lines.push(line);
    }
    return lines;
}

pub fn create_light_shape_from_spot(props: &PropertyMap) -> Option<LightShape> {
    let coneangle = props.find_one_float("coneangle").unwrap_or(30.0);
    let conedelta = props.find_one_float("conedeltaangle").unwrap_or(5.0);
    //0,0,0 -> 0,0,1
    let coneangle = coneangle * 0.5; //convert to half angle
    let conedelta = conedelta * 0.5; //convert to half angle

    let mut lines = Vec::new();

    let inner_radius = (coneangle - conedelta).max(0.0).to_radians().tan();
    let outer_radius = coneangle.to_radians().tan();

    let outer_circle = create_circle_points(outer_radius, 16);
    lines.push(outer_circle);
    if inner_radius > 0.0 {
        let inner_circle = create_circle_points(inner_radius, 16);
        lines.push(inner_circle);
    }
    let spot_lines = create_spot_lines(outer_radius, 8);
    lines.extend(spot_lines);

    let light_shape = LightShape { lines };
    return Some(light_shape);
}
