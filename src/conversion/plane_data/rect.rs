use super::plane_data::PlaneOutline;
use crate::{conversion::plane_data::PlaneRect, model::base::Vector3};

fn optimize_outline(outline: &PlaneOutline, threthould: f32) -> PlaneOutline {
    let mut optimized = outline.clone();
    let vn = optimized.positions.len() / 3;
    let positions = (0..vn)
        .map(|i| {
            Vector3::new(
                optimized.positions[i * 3],
                optimized.positions[i * 3 + 1],
                optimized.positions[i * 3 + 2],
            )
        })
        .collect::<Vec<Vector3>>();
    let mut new_positions = Vec::new();
    for i in 0..vn {
        let prev = if i == 0 { vn - 1 } else { i - 1 };
        let next = if i == vn - 1 { 0 } else { i + 1 };
        let v_prev = positions[i] - positions[prev];
        let v_next = positions[next] - positions[i];
        if v_prev.length() < 1e-6 || v_next.length() < 1e-6 {
            continue; // Skip duplicate points
        }
        let v_prev = v_prev.normalize();
        let v_next = v_next.normalize();
        let dot = Vector3::dot(&v_prev, &v_next);
        if dot > threthould {
            continue; // Skip nearly collinear points
        }
        new_positions.push(positions[i]);
    }
    optimized.positions.clear();
    for v in new_positions {
        optimized.positions.push(v.x);
        optimized.positions.push(v.y);
        optimized.positions.push(v.z);
    }
    return optimized;
}

pub fn create_plane_rect_from_plane_outline(
    outline: &PlaneOutline,
    threthould: f32,
) -> Option<PlaneRect> {
    //let vn1 = outline.positions.len() / 3;
    let outline = optimize_outline(outline, threthould);
    let vn2 = outline.positions.len() / 3;
    //println!("optimize_outline: {} -> {}", vn1, vn2);
    if vn2 == 4 {
        let vn = vn2;
        let mut center = Vector3::new(0.0, 0.0, 0.0);
        let mut normal = Vector3::new(0.0, 0.0, 0.0);
        let mut positions = Vec::new();
        for i in 0..vn {
            let v = Vector3::new(
                outline.positions[i * 3],
                outline.positions[i * 3 + 1],
                outline.positions[i * 3 + 2],
            );
            positions.push(v);
            center += v;
        }
        center = center * 1.0 / vn as f32;
        for i in 0..vn {
            let v0 = positions[i];
            let v1 = positions[(i + 1) % vn];
            let v2 = positions[(i + 2) % vn];
            let edge1 = v1 - v0;
            let edge2 = v2 - v1;
            normal += Vector3::cross(&edge1, &edge2);
        }
        normal = normal.normalize();
        let edge0 = positions[1] - positions[0];
        let edge1 = positions[2] - positions[1];
        let edge2 = positions[3] - positions[2];
        let edge3 = positions[0] - positions[3];
        let u_dir = (edge0 - edge2) * 0.5;
        let v_dir = (edge1 - edge3) * 0.5;
        let u_axis = u_dir * 0.5;
        let v_axis = v_dir * 0.5;
        let rect = PlaneRect {
            center: [center.x, center.y, center.z],
            normal: [normal.x, normal.y, normal.z],
            u_axis: [u_axis.x, u_axis.y, u_axis.z],
            v_axis: [v_axis.x, v_axis.y, v_axis.z],
        };
        return Some(rect);
    }
    return None;
}
