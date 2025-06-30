use super::mesh_data::MeshData;
use crate::models::scene::shape::Shape;

fn create_disk_plate(
    radius: f32,
    innerradius: f32,
    height: f32,
    flip: bool,
    udiv: i32,
) -> MeshData {
    let vdiv = 1;
    let mut indices: Vec<i32> = Vec::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();

    let n = if flip {
        [0.0, 0.0, -1.0]
    } else {
        [0.0, 0.0, 1.0]
    };
    for iu in 0..=udiv {
        for iv in 0..=vdiv {
            let u = iu as f32 / udiv as f32;
            let v = iv as f32 / vdiv as f32;

            let theta = u * std::f32::consts::PI * 2.0;
            let l = innerradius + (radius - innerradius) * v;

            let x = theta.sin() * l;
            let y = theta.cos() * l;
            let z = height;

            positions.push(x);
            positions.push(y);
            positions.push(z);

            normals.push(n[0]);
            normals.push(n[1]);
            normals.push(n[2]);

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
    return mesh_data;
}

fn create_disk_cylinder(radius: f32, height: f32, flip: bool, vv: f32, udiv: i32) -> MeshData {
    let vdiv = 1;
    let mut indices: Vec<i32> = Vec::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();

    for iu in 0..=udiv {
        for iv in 0..=vdiv {
            let u = iu as f32 / udiv as f32;
            let v = iv as f32 / vdiv as f32;

            let theta = u * std::f32::consts::PI * 2.0;
            let x = theta.sin();
            let y = theta.cos();
            let z = v * height;

            positions.push(x * radius);
            positions.push(y * radius);
            positions.push(z);

            let n = if flip { [-x, -y, 0.0] } else { [x, y, 0.0] };

            normals.push(n[0]);
            normals.push(n[1]);
            normals.push(n[2]);

            uvs.push(u);
            uvs.push(vv);
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
    return mesh_data;
}

fn merge_mesh_data(meshes: &[MeshData]) -> MeshData {
    let mut indices: Vec<i32> = Vec::new();
    let mut positions: Vec<f32> = Vec::new();
    let mut normals: Vec<f32> = Vec::new();
    let mut uvs: Vec<f32> = Vec::new();
    let mut index_offset = 0;
    for shape in meshes {
        indices.extend(shape.indices.iter().map(|i| i + index_offset));
        positions.extend(shape.positions.iter());
        normals.extend(shape.normals.iter());
        uvs.extend(shape.uvs.iter());
        index_offset += shape.positions.len() as i32 / 3;
    }
    let mesh_data = MeshData {
        indices,
        positions,
        normals,
        uvs,
        tangents: vec![],
    };
    return mesh_data;
}

pub fn create_mesh_data_from_disk(shape: &Shape) -> Option<MeshData> {
    let mesh_type = shape.get_type();
    assert!(mesh_type == "disk", "Mesh type is not disk");
    let radius = shape
        .as_property_map()
        .find_one_float("radius")
        .unwrap_or(1.0);
    let height = shape
        .as_property_map()
        .find_one_float("height")
        .unwrap_or(0.0);
    let innerradius = shape
        .as_property_map()
        .find_one_float("innerradius")
        .unwrap_or(0.0);
    let udiv = shape.as_property_map().find_one_int("udiv").unwrap_or(32);
    //let vdiv = shape.as_property_map().find_one_int("vdiv").unwrap_or(4);

    if height == 0.0 {
        return Some(create_disk_plate(radius, innerradius, 0.0, false, udiv));
    } else {
        let mut meshes = Vec::new();
        let top = create_disk_plate(radius, innerradius, height, false, udiv);
        let bottom = create_disk_plate(radius, innerradius, 0.0, true, udiv);
        meshes.push(top);
        meshes.push(bottom);
        let outside = create_disk_cylinder(radius, height, false, 1.0, udiv);
        meshes.push(outside);
        if innerradius > 0.0 {
            let inside = create_disk_cylinder(innerradius, height, true, 0.0, udiv);
            meshes.push(inside);
        }
        let mesh_data = merge_mesh_data(&meshes);
        return Some(mesh_data);
    }
}
