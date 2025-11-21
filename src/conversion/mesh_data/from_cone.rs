use super::mesh_data::MeshData;
use crate::model::scene::Shape;

pub fn create_mesh_data_from_cone(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    assert!(mesh_type == "cone", "Mesh type is not cone");
    let radius = shape
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);
    let height = shape
        .as_property_map()
        .find_one_float("height")
        .unwrap_or(1.0);
    let udiv = shape.as_property_map().find_one_int("udiv").unwrap_or(32);
    let vdiv = 1; //shape.as_property_map().find_one_int("vdiv").unwrap_or(16);

    let mut indices: Vec<i32> = Vec::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    for iu in 0..=udiv {
        for iv in 0..=vdiv {
            let u = iu as f32 / udiv as f32;
            let v = iv as f32 / vdiv as f32;

            let theta = u * std::f32::consts::PI * 2.0;
            let r = radius * (1.0 - v);
            let h = height * v;

            let x = theta.sin() * r;
            let y = theta.cos() * r;
            let z = h;

            positions.push(x);
            positions.push(y);
            positions.push(z);

            let length = (x * x + y * y + z * z).sqrt();
            let nx = x / length;
            let ny = y / length;
            let nz = z / length;

            normals.push(nx); //
            normals.push(ny); //
            normals.push(nz); //

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
        indices,
        positions,
        normals,
        uvs,
        tangents: vec![],
    };
    return Some(mesh_data);
}
