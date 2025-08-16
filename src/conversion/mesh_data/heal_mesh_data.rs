use super::mesh_data::MeshData;
use crate::model::base::Vector2;
use crate::model::base::Vector3;
use crate::render::wgpu::mesh;

use std::collections::HashMap;

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

            let dp02 = p0 - p2;
            let dp12 = p1 - p2;
            let normal = Vector3::cross(&dp02, &dp12);

            //let edge1 = p1 - p0;
            //let edge2 = p2 - p0;
            //let normal = Vector3::cross(&edge1, &edge2);

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
        /*
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
        */
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
/*
#[inline]
fn coordinate_system(v1: &Vector3) -> (Vector3, Vector3) {
    let v1 = [v1.x, v1.y, v1.z];
    let mut axis1 = 0;
    if v1[1].abs() > v1[axis1].abs() {
        axis1 = 1;
    }
    if v1[2].abs() > v1[axis1].abs() {
        axis1 = 2;
    }
    let mut axis2 = (axis1 + 1) % 3;
    let mut axis3 = (axis1 + 2) % 3;
    if v1[axis2].abs() > v1[axis3].abs() {
        // Swap axis2 and axis3 if axis2 is larger
        std::mem::swap(&mut axis2, &mut axis3);
    }
    //let axis2 = if axis1 == 0 { 1 } else { 0 };
    //let axis3 = (axis1 + 2) % 3;
    let mut v2 = [0.0; 3];
    v2[axis2] = 1.0;//f32::signum(v1[axis2]);
    let v2 = Vector3::new(
        v2[0],
        v2[1],
        v2[2],
    );
    let v1 = Vector3::new(v1[0], v1[1], v1[2]);
    let v3 = Vector3::cross(&v1, &v2).normalize();
    let v2 = Vector3::cross(&v3, &v1).normalize();
    return (v2, v3);
}
*/

fn difference_of_products_f32(a: f32, b: f32, c: f32, d: f32) -> f32 {
    //X =  a * b - cd
    //Y = -c * d + cd
    //Z = X + Y = a * b - c * d
    return a * b - c * d;
}

fn difference_of_products_v3(a: f32, b: Vector3, c: f32, d: Vector3) -> Vector3 {
    //X =  a * b - cd
    //Y = -c * d + cd
    //Z = X + Y = a * b - c * d
    return a * b - c * d;
}

fn heal_tangents(mesh_data: &mut MeshData) {
    assert!(!mesh_data.positions.is_empty(), "Positions cannot be empty");
    assert!(!mesh_data.indices.is_empty(), "Indices cannot be empty");
    assert!(!mesh_data.normals.is_empty(), "Normals cannot be empty");
    // Ensure tangents are valid, if not, set them to default values
    let num_vertices = mesh_data.positions.len() / 3;
    if mesh_data.tangents.len() != num_vertices * 3 {
        if mesh_data.uvs.len() != num_vertices * 2 {
            //if true {
            // Calculate tangents based on normals
            let mut tangents = vec![Vector3::zero(); num_vertices];
            for i in 0..num_vertices {
                let n = Vector3::new(
                    mesh_data.normals[3 * i + 0],
                    mesh_data.normals[3 * i + 1],
                    mesh_data.normals[3 * i + 2],
                );
                let (tangent, _bitangent) = coordinate_system(&-n);
                tangents[i] = tangent;
            }
            mesh_data.tangents.resize(num_vertices * 3, 0.0);
            for i in 0..num_vertices {
                if tangents[i].length() == 0.0 {
                    // If the tangent is zero, set it to a default value
                    tangents[i] = Vector3::new(1.0, 0.0, 0.0);
                    // println!("Warning: Tangent for vertex {} is zero, setting to default (1.0, 0.0, 0.0)", i);
                } else {
                    tangents[i] = tangents[i].normalize();
                }
                let idx = 3 * i;
                mesh_data.tangents[idx + 0] = tangents[i].x;
                mesh_data.tangents[idx + 1] = tangents[i].y;
                mesh_data.tangents[idx + 2] = tangents[i].z;
            }
        } else {
            // Calculate tangents based on positions and UVs
            let num_faces = mesh_data.indices.len() / 3;
            let mut face_tangents = vec![Vector3::zero(); num_faces];
            let mut ref_indices = vec![Vec::new(); num_vertices];
            for i in 0..num_faces {
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

                let uv0 = Vector2::new(mesh_data.uvs[2 * idx0 + 0], mesh_data.uvs[2 * idx0 + 1]);
                let uv1 = Vector2::new(mesh_data.uvs[2 * idx1 + 0], mesh_data.uvs[2 * idx1 + 1]);
                let uv2 = Vector2::new(mesh_data.uvs[2 * idx2 + 0], mesh_data.uvs[2 * idx2 + 1]);

                let duv02 = uv0 - uv2;
                let duv12 = uv1 - uv2;

                let dp02 = p0 - p2;
                let dp12 = p1 - p2;
                let determinant = difference_of_products_f32(duv02.x, duv12.y, duv02.y, duv12.x);
                let degenerate_uv = f32::abs(determinant) < 1e-8;
                if !degenerate_uv {
                    let invdet = 1.0 / determinant;
                    let dpdu = difference_of_products_v3(duv12.y, dp02, duv02.y, dp12) * invdet;
                    let dpdv = difference_of_products_v3(duv02.x, dp12, duv12.x, dp02) * invdet;
                    let nn = Vector3::cross(&dpdu, &dpdv);
                    if nn.length_squared() > 0.0 {
                        let tangent = dpdu;
                        face_tangents[i] = tangent;
                        ref_indices[idx0].push(i);
                        ref_indices[idx1].push(i);
                        ref_indices[idx2].push(i);
                    }
                }
            }

            let mut should_split = false;
            let mut vertex_tangents = vec![None; num_vertices];
            {
                for i in 0..num_vertices {
                    let indices = &ref_indices[i];
                    if indices.len() < 2 {
                        continue; // No need to check if there are less than 2 faces
                    }
                    let mut vertex_should_split = false;
                    for j in 0..indices.len() {
                        for k in j + 1..indices.len() {
                            let j_tangent = face_tangents[indices[j]];
                            let k_tangent = face_tangents[indices[k]];
                            if Vector3::dot(&j_tangent, &k_tangent) <= 0.25 {
                                vertex_should_split = true;
                                should_split = true;
                                break;
                            }
                        }
                    }
                    if vertex_should_split {
                        vertex_tangents[i] = None;
                    } else {
                        let mut tangent = Vector3::zero();
                        for &face_idx in indices {
                            tangent += face_tangents[face_idx];
                        }
                        tangent = tangent.normalize();
                        if tangent.length() == 0.0 {
                            should_split = true;
                            vertex_tangents[i] = None;
                        } else {
                            vertex_tangents[i] = Some(tangent);
                        }
                    }
                }
            }
            if should_split {
                let mut indices_map = HashMap::new();
                let mut tangents = vec![Vector3::zero(); num_vertices];
                for i in 0..num_faces {
                    let idx0 = mesh_data.indices[3 * i] as usize;
                    let idx1 = mesh_data.indices[3 * i + 1] as usize;
                    let idx2 = mesh_data.indices[3 * i + 2] as usize;
                    if let Some(t) = vertex_tangents[idx0] {
                        tangents[idx0] = t;
                    } else {
                        let new_idx = tangents.len();
                        tangents.push(face_tangents[i]);
                        indices_map.insert(3 * i, new_idx);
                    }
                    if let Some(t) = vertex_tangents[idx1] {
                        tangents[idx1] = t;
                    } else {
                        let new_idx = tangents.len();
                        tangents.push(face_tangents[i]);
                        indices_map.insert(3 * i + 1, new_idx);
                    }
                    if let Some(t) = vertex_tangents[idx2] {
                        tangents[idx2] = t;
                    } else {
                        let new_idx = tangents.len();
                        tangents.push(face_tangents[i]);
                        indices_map.insert(3 * i + 2, new_idx);
                    }
                }
                mesh_data.tangents.resize(tangents.len() * 3, 0.0);
                for i in 0..tangents.len() {
                    if tangents[i].length() == 0.0 {
                        // If the tangent is zero, set it to a default value
                        tangents[i] = Vector3::new(1.0, 0.0, 0.0);
                        // println!("Warning: Tangent for vertex {} is zero, setting to default (1.0, 0.0, 0.0)", i);
                    } else {
                        tangents[i] = tangents[i].normalize();
                    }
                    let idx = i * 3;
                    mesh_data.tangents[idx + 0] = tangents[i].x;
                    mesh_data.tangents[idx + 1] = tangents[i].y;
                    mesh_data.tangents[idx + 2] = tangents[i].z;
                }
                let mut new_positions = vec![0.0; tangents.len() * 3];
                let mut new_normals = vec![0.0; tangents.len() * 3];
                let mut new_uvs = if !mesh_data.uvs.is_empty() {
                    Some(vec![0.0; tangents.len() * 2])
                } else {
                    None
                };
                for i in 0..num_vertices {
                    let idx = 3 * i;
                    new_positions[idx + 0] = mesh_data.positions[idx + 0];
                    new_positions[idx + 1] = mesh_data.positions[idx + 1];
                    new_positions[idx + 2] = mesh_data.positions[idx + 2];
                    new_normals[idx + 0] = mesh_data.normals[idx + 0];
                    new_normals[idx + 1] = mesh_data.normals[idx + 1];
                    new_normals[idx + 2] = mesh_data.normals[idx + 2];
                    if let Some(ref mut uvs) = new_uvs {
                        uvs[2 * i + 0] = mesh_data.uvs[2 * i + 0];
                        uvs[2 * i + 1] = mesh_data.uvs[2 * i + 1];
                    }
                }
                for i in 0..num_faces {
                    let idx0 = mesh_data.indices[3 * i + 0] as usize;
                    let idx1 = mesh_data.indices[3 * i + 1] as usize;
                    let idx2 = mesh_data.indices[3 * i + 2] as usize;
                    if let Some(new_idx) = indices_map.get(&(3 * i)) {
                        new_positions[3 * new_idx + 0] = mesh_data.positions[3 * idx0 + 0];
                        new_positions[3 * new_idx + 1] = mesh_data.positions[3 * idx0 + 1];
                        new_positions[3 * new_idx + 2] = mesh_data.positions[3 * idx0 + 2];
                        new_normals[3 * new_idx + 0] = mesh_data.normals[3 * idx0 + 0];
                        new_normals[3 * new_idx + 1] = mesh_data.normals[3 * idx0 + 1];
                        new_normals[3 * new_idx + 2] = mesh_data.normals[3 * idx0 + 2];
                        if let Some(ref mut uvs) = new_uvs {
                            uvs[2 * new_idx + 0] = mesh_data.uvs[2 * idx0 + 0];
                            uvs[2 * new_idx + 1] = mesh_data.uvs[2 * idx0 + 1];
                        }
                        mesh_data.indices[3 * i + 0] = *new_idx as i32;
                    }
                    if let Some(new_idx) = indices_map.get(&(3 * i + 1)) {
                        new_positions[3 * new_idx + 0] = mesh_data.positions[3 * idx1 + 0];
                        new_positions[3 * new_idx + 1] = mesh_data.positions[3 * idx1 + 1];
                        new_positions[3 * new_idx + 2] = mesh_data.positions[3 * idx1 + 2];
                        new_normals[3 * new_idx + 0] = mesh_data.normals[3 * idx1 + 0];
                        new_normals[3 * new_idx + 1] = mesh_data.normals[3 * idx1 + 1];
                        new_normals[3 * new_idx + 2] = mesh_data.normals[3 * idx1 + 2];
                        if let Some(ref mut uvs) = new_uvs {
                            uvs[2 * new_idx + 0] = mesh_data.uvs[2 * idx1 + 0];
                            uvs[2 * new_idx + 1] = mesh_data.uvs[2 * idx1 + 1];
                        }
                        mesh_data.indices[3 * i + 1] = *new_idx as i32;
                    }
                    if let Some(new_idx) = indices_map.get(&(3 * i + 2)) {
                        new_positions[3 * new_idx + 0] = mesh_data.positions[3 * idx2 + 0];
                        new_positions[3 * new_idx + 1] = mesh_data.positions[3 * idx2 + 1];
                        new_positions[3 * new_idx + 2] = mesh_data.positions[3 * idx2 + 2];
                        new_normals[3 * new_idx + 0] = mesh_data.normals[3 * idx2 + 0];
                        new_normals[3 * new_idx + 1] = mesh_data.normals[3 * idx2 + 1];
                        new_normals[3 * new_idx + 2] = mesh_data.normals[3 * idx2 + 2];
                        if let Some(ref mut uvs) = new_uvs {
                            uvs[2 * new_idx + 0] = mesh_data.uvs[2 * idx2 + 0];
                            uvs[2 * new_idx + 1] = mesh_data.uvs[2 * idx2 + 1];
                        }
                        mesh_data.indices[3 * i + 2] = *new_idx as i32;
                    }
                }

                mesh_data.positions = new_positions;
                mesh_data.normals = new_normals;
                if let Some(ref uvs) = new_uvs {
                    mesh_data.uvs = uvs.clone();
                }
            } else {
                let mut tangents = vec![Vector3::zero(); num_vertices];
                for i in 0..num_faces {
                    let idx0 = mesh_data.indices[3 * i + 0] as usize;
                    let idx1 = mesh_data.indices[3 * i + 1] as usize;
                    let idx2 = mesh_data.indices[3 * i + 2] as usize;
                    let tangent = face_tangents[i];
                    tangents[idx0] += tangent;
                    tangents[idx1] += tangent;
                    tangents[idx2] += tangent;
                }
                mesh_data.tangents.resize(num_vertices * 3, 0.0);
                for i in 0..num_vertices {
                    if tangents[i].length() == 0.0 {
                        // If the tangent is zero, set it to a default value
                        tangents[i] = Vector3::new(1.0, 0.0, 0.0);
                        // println!("Warning: Tangent for vertex {} is zero, setting to default (1.0, 0.0, 0.0)", i);
                    } else {
                        tangents[i] = tangents[i].normalize();
                    }
                    let idx = 3 * i;
                    mesh_data.tangents[idx + 0] = tangents[i].x;
                    mesh_data.tangents[idx + 1] = tangents[i].y;
                    mesh_data.tangents[idx + 2] = tangents[i].z;
                }
            }
        }

        for i in 0..num_vertices {
            let idx = 3 * i;
            let x = mesh_data.tangents[idx + 0];
            let y = mesh_data.tangents[idx + 1];
            let z = mesh_data.tangents[idx + 2];
            let length = f32::sqrt(x * x + y * y + z * z);
            if length < 1e-8 {
                mesh_data.tangents[idx + 0] = 1.0;
                mesh_data.tangents[idx + 1] = 0.0;
                mesh_data.tangents[idx + 2] = 0.0;
            } else {
                mesh_data.tangents[idx + 0] = x / length;
                mesh_data.tangents[idx + 1] = y / length;
                mesh_data.tangents[idx + 2] = z / length;
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
    heal_tangents(mesh_data);
    heal_uvs(mesh_data);
}
