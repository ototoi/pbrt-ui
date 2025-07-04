use super::mesh_data::MeshData;
use crate::model::scene::Shape;

pub fn create_mesh_data_from_trianglemesh(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    assert!(mesh_type == "trianglemesh", "Mesh type is not trianglemesh");
    if let Some(indices) = shape.get_indices() {
        if let Some(positions) = shape.get_positions() {
            let normals: Vec<f32> = Vec::new();
            let uvs: Vec<f32> = Vec::new();
            let s = Vec::new();

            let mesh_data = MeshData {
                indices: indices.to_vec(),
                positions: positions.to_vec(),
                tangents: s,
                normals: normals,
                uvs: uvs,
            };
            return Some(mesh_data);
        }
    }
    return None;
}
