use super::super::mesh::Mesh;
use super::mesh_data::MeshData;

pub fn create_mesh_data_from_sphere(mesh: &Mesh) -> Option<MeshData> {
    let mesh_type = mesh.get_type();
    //assert!(mesh_type == "sphere", "Mesh type is not sphere");
    let radius = mesh
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);
    let udiv = mesh.as_property_map().find_one_int("udiv").unwrap_or(32);
    let vdiv = mesh.as_property_map().find_one_int("vdiv").unwrap_or(16);

    let mut indices: Vec<i32> = Vec::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    for iu in 0..=udiv {
        for iv in 0..=vdiv {
            let u = iu as f32 / udiv as f32;
            let v = iv as f32 / vdiv as f32;

            let theta = u * std::f32::consts::PI * 2.0;
            let phi = v * std::f32::consts::PI;

            let x = theta.sin() * phi.sin();
            let y = theta.cos() * phi.sin();
            let z = phi.cos();

            positions.push(radius * x);
            positions.push(radius * y);
            positions.push(radius * z);

            normals.push(x);
            normals.push(y);
            normals.push(z);

            uvs.push(u);
            uvs.push(v);
        }
    }

    for iu in 0..udiv {
        for iv in 0..vdiv {
            let ix0 = iu * (vdiv + 1);
            let ix1 = (iu + 1) * (vdiv + 1);

            let i0 = ix0 + iv;
            let i1 = ix1 + iv;
            let i2 = ix1 + (iv + 1);
            let i3 = ix0 + (iv + 1);

            indices.push(i0 as i32);
            indices.push(i1 as i32);
            indices.push(i2 as i32);
            indices.push(i0 as i32);
            indices.push(i2 as i32);
            indices.push(i3 as i32);
        }
    }

    let mesh_data = MeshData {
        positions,
        normals,
        uvs,
        indices,
        tangents: vec![],
    };
    return Some(mesh_data);
}
