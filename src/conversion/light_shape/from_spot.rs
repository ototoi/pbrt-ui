use super::light_shape::LightShape;
use crate::model::base::Vector3;
use crate::model::base::Matrix4x4;
use crate::model::scene::Light;

#[inline]
fn coordinate_system(v1: &Vector3) -> (Vector3, Vector3) {
    let v2 = if f32::abs(v1.x) > f32::abs(v1.y) {
        Vector3::new(-v1.z, 0.0, v1.x) / f32::sqrt(v1.x * v1.x + v1.z * v1.z)
    } else {
        Vector3::new(0.0, v1.z, -v1.y) / f32::sqrt(v1.y * v1.y + v1.z * v1.z)
    };
    let v3 = Vector3::cross(v1, &v2).normalize();
    return (v2, v3);
}

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

fn apply_transform(lines: &mut Vec<Vec<Vector3>>, mat: &Matrix4x4) {
    for line in lines.iter_mut() {
        for point in line.iter_mut() {
            *point = mat.transform_point(point);
        }
    }
}

pub fn create_light_shape_from_spot(light: &Light) -> Option<LightShape> {
    let props = light.as_property_map();
    let coneangle = props.find_one_float("coneangle").unwrap_or(30.0);
    let conedelta = props.find_one_float("conedeltaangle").unwrap_or(5.0);
    //0,0,0 -> 0,0,1
    // let coneangle = coneangle * 0.5; //convert to half angle
    // let conedelta = conedelta * 0.5; //convert to half angle

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
    let dir = (to - from).normalize();
    let (du, dv) = coordinate_system(&dir);
    let dir_to_z = Matrix4x4::new(
        du.x, du.y, du.z, 0.0, dv.x, dv.y, dv.z, 0., dir.x, dir.y, dir.z, 0.0, 0.0, 0.0, 0.0, 1.0,
    );
    let mat = Matrix4x4::translate(from.x, from.y, from.z) * Matrix4x4::inverse(&dir_to_z).unwrap();

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

    apply_transform(&mut lines, &mat);

    let light_shape = LightShape { lines };
    return Some(light_shape);
}
