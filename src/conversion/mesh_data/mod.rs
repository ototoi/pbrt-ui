mod from_cone;
mod from_cylinder;
mod from_disk;
mod from_loopsubdiv;
mod from_plymesh;
mod from_sphere;
mod from_trianglemesh;
mod heal_mesh_data;
mod mesh_data;

use crate::model::scene::Shape;
use from_cone::create_mesh_data_from_cone;
use from_cylinder::create_mesh_data_from_cylinder;
use from_disk::create_mesh_data_from_disk;
use from_loopsubdiv::create_mesh_data_from_loopsubdiv;
use from_plymesh::create_mesh_data_from_plymesh;
use from_sphere::create_mesh_data_from_sphere;
use from_trianglemesh::create_mesh_data_from_trianglemesh;
use heal_mesh_data::heal_mesh_data;

pub use mesh_data::MeshData;

fn create_mesh_data_core(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    match mesh_type.as_str() {
        "trianglemesh" => {
            return create_mesh_data_from_trianglemesh(shape);
        }
        "plymesh" => {
            return create_mesh_data_from_plymesh(shape);
        }
        "sphere" => {
            return create_mesh_data_from_sphere(shape);
        }
        "disk" => {
            return create_mesh_data_from_disk(shape);
        }
        "cone" => {
            return create_mesh_data_from_cone(shape);
        }
        "cylinder" => {
            return create_mesh_data_from_cylinder(shape);
        }
        "paraboloid" => {
            // Handle paraboloid shape
            // You can implement the logic for paraboloid shape here
            return create_mesh_data_from_sphere(shape);
        }
        "hyperboloid" => {
            // Handle hyperboloid shape
            // You can implement the logic for hyperboloid shape here
            return create_mesh_data_from_sphere(shape);
        }
        "loopsubdiv" => {
            return create_mesh_data_from_loopsubdiv(shape);
        }
        _ => {
            println!("Unknown shape type: {}", mesh_type);
        }
    }
    return None;
}

pub fn create_mesh_data(shape: &Shape) -> Option<MeshData> {
    if let Some(mut mesh_data) = create_mesh_data_core(shape) {
        heal_mesh_data(&mut mesh_data);
        return Some(mesh_data);
    }
    return None;
}
