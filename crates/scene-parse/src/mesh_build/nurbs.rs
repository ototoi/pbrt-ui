use super::*;

fn bspline_basis(index: usize, degree: usize, t: f64, knots: &[f64], count: usize) -> f64 {
    if t == knots[count] {
        return if index + 1 == count { 1.0 } else { 0.0 };
    }
    if degree == 0 {
        return if knots[index] <= t && t < knots[index + 1] {
            1.0
        } else {
            0.0
        };
    }
    let left_denominator = knots[index + degree] - knots[index];
    let right_denominator = knots[index + degree + 1] - knots[index + 1];
    let left = if left_denominator > 0.0 {
        (t - knots[index]) / left_denominator * bspline_basis(index, degree - 1, t, knots, count)
    } else {
        0.0
    };
    let right = if right_denominator > 0.0 {
        (knots[index + degree + 1] - t) / right_denominator
            * bspline_basis(index + 1, degree - 1, t, knots, count)
    } else {
        0.0
    };
    left + right
}

/// Samples a rational B-spline surface using bounded per-axis dice counts.
pub fn nurbs_mesh(d: &Directive) -> Result<crate::tessellate::Tessellated, SceneParseError> {
    const MAX_CONTROL_POINTS_PER_AXIS: usize = 256;
    const MAX_DICE: usize = 64;
    const MAX_VERTICES: usize = 1_000_000;
    let params = d.params();
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    let nu = params.int("nu", -1);
    let nv = params.int("nv", -1);
    let uorder = params.int("uorder", -1);
    let vorder = params.int("vorder", -1);
    if nu < 2 || nv < 2 || uorder < 2 || vorder < 2 {
        return Err(fail("nurbs requires nu, nv and orders of at least 2"));
    }
    let (nu, nv, uorder, vorder) = (nu as usize, nv as usize, uorder as usize, vorder as usize);
    if nu > MAX_CONTROL_POINTS_PER_AXIS
        || nv > MAX_CONTROL_POINTS_PER_AXIS
        || uorder > nu
        || vorder > nv
    {
        return Err(fail("nurbs dimensions or orders exceed the preview limits"));
    }
    let uknots = params.floats("uknots");
    let vknots = params.floats("vknots");
    let valid_knots = |knots: &[f64], points: usize, order: usize| {
        knots.len() == points + order
            && knots.iter().all(|value| value.is_finite())
            && knots.windows(2).all(|pair| pair[0] <= pair[1])
            && knots[order - 1] < knots[points]
    };
    if !valid_knots(&uknots, nu, uorder) || !valid_knots(&vknots, nv, vorder) {
        return Err(fail(
            "nurbs knot vectors must be finite, ordered and match the control grid",
        ));
    }
    let homogeneous = params.has("Pw");
    let values = params.floats(if homogeneous { "Pw" } else { "P" });
    let tuple = if homogeneous { 4 } else { 3 };
    let control_count = nu
        .checked_mul(nv)
        .ok_or_else(|| fail("nurbs control grid dimensions overflow"))?;
    if values.len() != control_count * tuple
        || values.iter().any(|value| !value.is_finite())
        || (homogeneous && values.chunks_exact(4).any(|point| point[3] == 0.0))
    {
        return Err(fail(
            "nurbs requires a finite P or Pw control grid matching nu × nv",
        ));
    }
    let diceu = params.int("diceu", 30).clamp(2, MAX_DICE as i64) as usize;
    let dicev = params.int("dicev", 30).clamp(2, MAX_DICE as i64) as usize;
    if diceu
        .checked_mul(dicev)
        .is_none_or(|count| count > MAX_VERTICES)
    {
        return Err(fail("nurbs tessellation exceeds the preview vertex limit"));
    }
    let (ua, ub) = (uknots[uorder - 1], uknots[nu]);
    let (va, vb) = (vknots[vorder - 1], vknots[nv]);
    let u0 = params.float("u0", ua).clamp(ua, ub);
    let u1 = params.float("u1", ub).clamp(ua, ub);
    let v0 = params.float("v0", va).clamp(va, vb);
    let v1 = params.float("v1", vb).clamp(va, vb);
    if !(u0 < u1 && v0 < v1) {
        return Err(fail("nurbs clipping ranges must have positive extent"));
    }
    let mut points = Vec::with_capacity(diceu * dicev);
    let mut uvs = Vec::with_capacity(diceu * dicev);
    let mut basis_u = vec![vec![0.0; nu]; diceu];
    let mut basis_v = vec![vec![0.0; nv]; dicev];
    // The actual knot parameter each dice step evaluates at (r4 keeps these
    // as the surface UVs, unnormalized).
    let (mut ts_u, mut ts_v) = (vec![0.0; diceu], vec![0.0; dicev]);
    for (sample, (basis, t)) in basis_u.iter_mut().zip(&mut ts_u).enumerate() {
        *t = u0 + (u1 - u0) * sample as f64 / (diceu - 1) as f64;
        for (index, weight) in basis.iter_mut().enumerate() {
            *weight = bspline_basis(index, uorder - 1, *t, &uknots, nu);
        }
    }
    for (sample, (basis, t)) in basis_v.iter_mut().zip(&mut ts_v).enumerate() {
        *t = v0 + (v1 - v0) * sample as f64 / (dicev - 1) as f64;
        for (index, weight) in basis.iter_mut().enumerate() {
            *weight = bspline_basis(index, vorder - 1, *t, &vknots, nv);
        }
    }
    for (v_basis, t_v) in basis_v.iter().zip(&ts_v) {
        for (u_basis, t_u) in basis_u.iter().zip(&ts_u) {
            let mut point = [0.0; 3];
            let mut weight_sum = 0.0;
            for (y, v_weight) in v_basis.iter().enumerate() {
                for (x, u_weight) in u_basis.iter().enumerate() {
                    let basis = v_weight * u_weight;
                    let offset = (y * nu + x) * tuple;
                    let weight = if homogeneous { values[offset + 3] } else { 1.0 };
                    for axis in 0..3 {
                        point[axis] += values[offset + axis] * basis;
                    }
                    weight_sum += weight * basis;
                }
            }
            if !weight_sum.is_finite() || weight_sum.abs() <= 1e-12 {
                return Err(fail(
                    "nurbs evaluation produced a zero or invalid homogeneous weight",
                ));
            }
            points.push(point.map(|coordinate| coordinate / weight_sum));
            uvs.push([*t_u as f32, *t_v as f32]);
        }
    }
    let mut indices = Vec::with_capacity((diceu - 1) * (dicev - 1) * 6);
    for y in 0..dicev - 1 {
        for x in 0..diceu - 1 {
            let a = (y * diceu + x) as u32;
            let b = a + 1;
            let c = a + diceu as u32 + 1;
            let e = c - 1;
            indices.extend_from_slice(&[a, b, c, a, c, e]);
        }
    }
    Ok((points, indices, uvs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_build::test_directive;

    /// r4 `shapes/nurbs.rs`: the dicing carries the knot parameter values it
    /// evaluated at — the clipped [u0, u1] and [v0, v1] ranges as-is, not
    /// normalized.
    #[test]
    fn nurbs_uvs_keep_the_knot_parameter_values() {
        let source = "Shape \"nurbs\" \"integer nu\" [2] \"integer nv\" [2] \
\"integer uorder\" [2] \"integer vorder\" [2] \
\"float uknots\" [0 0 1 1] \"float vknots\" [0 0 1 1] \
\"float u0\" [0.25] \"float u1\" [0.75] \
\"float v0\" [0.5] \"float v1\" [1.0] \
\"float diceu\" [5] \"float dicev\" [4] \
\"point3 P\" [0 0 0  1 0 0  0 1 0  1 1 1]";
        let (points, indices, uvs) = nurbs_mesh(&test_directive(source)).unwrap();
        assert_eq!(points.len(), 20);
        assert_eq!(uvs.len(), points.len());
        let (diceu, dicev) = (5usize, 4usize);
        for (index, uv) in uvs.iter().enumerate() {
            let (x, y) = (index % diceu, index / diceu);
            let expected_u = 0.25 + 0.50 * x as f64 / (diceu - 1) as f64;
            let expected_v = 0.50 + 0.50 * y as f64 / (dicev - 1) as f64;
            assert!((uv[0] as f64 - expected_u).abs() < 1e-6, "{uv:?} {index}");
            assert!((uv[1] as f64 - expected_v).abs() < 1e-6, "{uv:?} {index}");
        }
        assert_eq!(indices.chunks_exact(3).count() * 3, indices.len());
    }
}
