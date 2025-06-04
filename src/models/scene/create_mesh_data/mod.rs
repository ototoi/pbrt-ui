mod from_cone;
mod from_cylinder;
mod from_disk;
mod from_loopsubdiv;
mod from_ply;
mod from_sphere;
mod mesh_data;

use super::mesh::Mesh;
use from_cone::create_mesh_data_from_cone;
use from_cylinder::create_mesh_data_from_cylinder;
use from_disk::create_mesh_data_from_disk;
use from_loopsubdiv::create_mesh_data_from_loopsubdiv;
use from_ply::create_mesh_data_from_ply;
use from_sphere::create_mesh_data_from_sphere;
pub use mesh_data::MeshData;

pub fn create_mesh_data(mesh: &Mesh) -> Option<MeshData> {
    let mesh_type = mesh.get_type();
    match mesh_type.as_str() {
        "trianglemesh" => {
            // Handle triangle mesh
            // You can implement the logic for triangle mesh here
            return None;
        }
        "plymesh" => {
            return create_mesh_data_from_ply(mesh);
        }
        "sphere" => {
            return create_mesh_data_from_sphere(mesh);
        }
        "disk" => {
            return create_mesh_data_from_disk(mesh);
        }
        "cone" => {
            return create_mesh_data_from_cone(mesh);
        }
        "cylinder" => {
            return create_mesh_data_from_cylinder(mesh);
        }
        "paraboloid" => {
            // Handle paraboloid mesh
            // You can implement the logic for paraboloid mesh here
            return create_mesh_data_from_sphere(mesh);
        }
        "hyperboloid" => {
            // Handle hyperboloid mesh
            // You can implement the logic for hyperboloid mesh here
            return create_mesh_data_from_sphere(mesh);
        }
        "loopsubdiv" => {
            return create_mesh_data_from_loopsubdiv(mesh);
        }
        _ => {
            println!("Unknown mesh type: {}", mesh_type);
        }
    }
    return None;
}
