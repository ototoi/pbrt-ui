use super::mesh_data::MeshData;
use crate::model::scene::Shape;

pub fn create_mesh_data_from_trianglemesh(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    assert!(mesh_type == "trianglemesh", "Mesh type is not trianglemesh");
    if let Some(indices) = shape.get_indices() {
        if let Some(positions) = shape.get_positions() {
            let indices = indices.to_vec();
            let positions = positions.to_vec();
            let normals: Vec<f32> = Vec::new();
            let mut uvs: Vec<f32> = Vec::new();
            let s = Vec::new();
            if let Some(v) = shape.get_uvs() {
                uvs = v.to_vec();
            } else {
                // If no UVs are provided, create a default UV mapping
                for _i in 0..positions.len() / 3 {
                    uvs.push(0.0);
                    uvs.push(0.0);
                }
            }

            let mesh_data = MeshData {
                indices: indices,
                positions: positions,
                tangents: s,
                normals: normals,
                uvs: uvs,
            };
            return Some(mesh_data);
        }
    }
    return None;
}
