use super::light_shape::LightShape;
use crate::conversion::mesh_data::create_mesh_data;
use crate::conversion::plane_data::create_outline_from_plane_mesh;
use crate::conversion::plane_data::create_plane_meshes_from_mesh;
use crate::model::base::Vector3;
use crate::model::scene::Light;
use crate::model::scene::Shape;

pub fn create_light_shape_from_mesh_area(_light: &Light, shape: &Shape) -> Option<LightShape> {
    if let Some(mesh) = create_mesh_data(shape) {
        let planes = create_plane_meshes_from_mesh(&mesh, 0.9);
        if !planes.is_empty() {
            let mut outlines = Vec::new();
            for plane in planes.iter() {
                if let Some(outline) = create_outline_from_plane_mesh(plane) {
                    let positions = &outline.positions;
                    let num_vertices = positions.len() / 3;
                    let mut lines = Vec::new();
                    for i in 0..num_vertices {
                        let v = Vector3::new(
                            positions[i * 3],
                            positions[i * 3 + 1],
                            positions[i * 3 + 2],
                        );
                        lines.push(v);
                    }
                    {
                        let i = 0;
                        let v = Vector3::new(
                            positions[i * 3],
                            positions[i * 3 + 1],
                            positions[i * 3 + 2],
                        );
                        lines.push(v);
                    }
                    //println!("outline: {:?}", lines);
                    outlines.push(lines);
                }
            }

            if !outlines.is_empty() {
                let light_shape = LightShape { lines: outlines };
                return Some(light_shape);
            }
        }
    }
    return None;
}

pub fn create_light_shape_from_area(light: &Light, shape: &Shape) -> Option<LightShape> {
    let shape_type = shape.get_type();
    match shape_type.as_str() {
        "trianglemesh" | "plymesh" => {
            return create_light_shape_from_mesh_area(light, shape);
        }
        _ => {
            return None;
        }
    }
    return None;
}
