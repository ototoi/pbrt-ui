use super::mesh_data::MeshData;
use crate::model::base::Vector3;

fn remove_microfaces(mesh_data: &mut MeshData) {
    // Remove microfaces by checking if the indices are valid
    let mut new_indices = Vec::new();
    for i in 0..mesh_data.indices.len() / 3 {
        let idx0 = mesh_data.indices[3 * i + 0] as usize;
        let idx1 = mesh_data.indices[3 * i + 1] as usize;
        let idx2 = mesh_data.indices[3 * i + 2] as usize;

        let p0 = Vector3::new(
            mesh_data.positions[3 * idx0 + 0],
            mesh_data.positions[3 * idx0 + 1],
            mesh_data.positions[3 * idx0 + 2],
        );
        let p1 = Vector3::new(
            mesh_data.positions[3 * idx1 + 0],
            mesh_data.positions[3 * idx1 + 1],
            mesh_data.positions[3 * idx1 + 2],
        );
        let p2 = Vector3::new(
            mesh_data.positions[3 * idx2 + 0],
            mesh_data.positions[3 * idx2 + 1],
            mesh_data.positions[3 * idx2 + 2],
        );
        let edge1 = p1 - p0;
        let edge2 = p2 - p0;
        let normal = Vector3::cross(&edge1, &edge2);
        if normal.length() > 0.0 {
            // Only add triangles with a valid normal
            new_indices.push(idx0 as i32);
            new_indices.push(idx1 as i32);
            new_indices.push(idx2 as i32);
        } else {
            // println!("Warning: Triangle with vertices ({}, {}, {}) has zero area, skipping", idx0, idx1, idx2);
        }
    }
    if new_indices.is_empty() {
        // If no valid triangles, create a default triangle
        new_indices.push(0);
        new_indices.push(1);
        new_indices.push(2);
    }
    mesh_data.indices = new_indices;
}

fn heal_normals(mesh_data: &mut MeshData) {
    let num_vertices = mesh_data.positions.len() / 3;
    if mesh_data.normals.len() != num_vertices * 3 {
        let mut normals = vec![Vector3::zero(); num_vertices];
        for i in 0..mesh_data.indices.len() / 3 {
            let idx0 = mesh_data.indices[3 * i + 0] as usize;
            let idx1 = mesh_data.indices[3 * i + 1] as usize;
            let idx2 = mesh_data.indices[3 * i + 2] as usize;

            let p0 = Vector3::new(
                mesh_data.positions[3 * idx0 + 0],
                mesh_data.positions[3 * idx0 + 1],
                mesh_data.positions[3 * idx0 + 2],
            );
            let p1 = Vector3::new(
                mesh_data.positions[3 * idx1 + 0],
                mesh_data.positions[3 * idx1 + 1],
                mesh_data.positions[3 * idx1 + 2],
            );
            let p2 = Vector3::new(
                mesh_data.positions[3 * idx2 + 0],
                mesh_data.positions[3 * idx2 + 1],
                mesh_data.positions[3 * idx2 + 2],
            );
            let edge1 = p1 - p0;
            let edge2 = p2 - p0;
            let normal = Vector3::cross(&edge1, &edge2);

            if normal.length() == 0.0 {
                // If the normal is zero, skip this triangle
                // println!("Warning: Triangle with vertices ({}, {}, {}) has zero area, skipping normal calculation", idx0, idx1, idx2);
                continue;
            }

            normals[idx0] += normal;
            normals[idx1] += normal;
            normals[idx2] += normal;
        }
        mesh_data.normals.resize(num_vertices * 3, 0.0);
        for i in 0..num_vertices {
            let idx = 3 * i;
            if normals[i].length() == 0.0 {
                // If the normal is zero, set it to a default value
                normals[i] = Vector3::new(0.0, 0.0, 1.0);
                //println!("Warning: Normal for vertex {} is zero, setting to default (0.0, 1.0, 0.0)", i);
            }
            let normal = normals[i].normalize();
            mesh_data.normals[idx + 0] = normal.x;
            mesh_data.normals[idx + 1] = normal.y;
            mesh_data.normals[idx + 2] = normal.z;
        }
    }
}

fn heal_uvs(mesh_data: &mut MeshData) {
    // Ensure UVs are valid, if not, set them to default values
    let num_vertices = mesh_data.positions.len() / 3;
    if mesh_data.uvs.len() != num_vertices * 2 {
        mesh_data.uvs.resize(num_vertices * 2, 0.0);
        for i in 0..num_vertices {
            let idx = 2 * i;
            mesh_data.uvs[idx + 0] = -1.0; // Default U
            mesh_data.uvs[idx + 1] = -1.0; // Default V
        }
        for i in 0..mesh_data.indices.len() / 3 {
            let idx0 = mesh_data.indices[3 * i + 0] as usize;
            let idx1 = mesh_data.indices[3 * i + 1] as usize;
            let idx2 = mesh_data.indices[3 * i + 2] as usize;

            // Set UVs for the vertices of the triangle
            if mesh_data.uvs[2 * idx0 + 0] < 0.0
                && mesh_data.uvs[2 * idx0 + 1] < 0.0
                && mesh_data.uvs[2 * idx1 + 0] < 0.0
                && mesh_data.uvs[2 * idx1 + 1] < 0.0
                && mesh_data.uvs[2 * idx2 + 0] < 0.0
                && mesh_data.uvs[2 * idx2 + 1] < 0.0
            {
                // Assign default UVs for the triangle vertices
                mesh_data.uvs[2 * idx0 + 0] = 0.0; // U for vertex 0
                mesh_data.uvs[2 * idx0 + 1] = 0.0; // V for vertex 0
                mesh_data.uvs[2 * idx1 + 0] = 1.0; // U for vertex 1
                mesh_data.uvs[2 * idx1 + 1] = 0.0; // V for vertex 1
                mesh_data.uvs[2 * idx2 + 0] = 1.0; // U for vertex 2
                mesh_data.uvs[2 * idx2 + 1] = 1.0; // V for vertex 2
            }
        }
        for i in 0..num_vertices {
            let idx = 2 * i;
            if mesh_data.uvs[idx + 0] < 0.0 {
                mesh_data.uvs[idx + 0] = 0.0; // Default U
            }
            if mesh_data.uvs[idx + 1] < 0.0 {
                mesh_data.uvs[idx + 1] = 0.0; // Default V
            }
        }
    }
}

pub fn heal_mesh_data(mesh_data: &mut MeshData) {
    // Ensure positions are not empty
    assert!(!mesh_data.positions.is_empty(), "Positions cannot be empty");
    assert!(!mesh_data.indices.is_empty(), "Indices cannot be empty");

    remove_microfaces(mesh_data);
    heal_normals(mesh_data);
    heal_uvs(mesh_data);
}
