use super::*;

/// r4 `shapes/paraboloid.rs`: u = phi / phi_max, v = (z - zmin) / (zmax - zmin).
pub fn paraboloid_mesh(d: &Directive) -> Result<crate::tessellate::Tessellated, SceneParseError> {
    const ANGULAR_STEPS: usize = 64;
    // 32 rows keep the chordal sagitta near r4's exact surface under a
    // preview pixel at grazing angles (the sphere tessellates 32 rows too).
    const VERTICAL_STEPS: usize = 32;
    let params = d.params();
    let radius = params.float("radius", 1.0);
    let mut z_min = params.float("zmin", 0.0);
    let mut z_max = params.float("zmax", 1.0);
    let phi = params.float("phimax", 360.0).clamp(0.0, 360.0);
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    if z_min > z_max {
        std::mem::swap(&mut z_min, &mut z_max);
    }
    if !radius.is_finite()
        || radius <= 0.0
        || !z_min.is_finite()
        || !z_max.is_finite()
        || z_max <= 0.0
        || z_min >= z_max
        || !phi.is_finite()
        || phi <= 0.0
    {
        return Err(fail(
            "paraboloid requires a positive radius, positive zmax and non-empty finite clipping bounds",
        ));
    }
    z_min = z_min.max(0.0);
    if z_min >= z_max {
        return Err(fail(
            "paraboloid clipping interval does not meet the surface",
        ));
    }
    let angular_steps = ((ANGULAR_STEPS as f64 * phi / 360.0).ceil() as usize).max(1);
    let mut points = Vec::with_capacity((VERTICAL_STEPS + 1) * (angular_steps + 1));
    let mut uvs = Vec::with_capacity((VERTICAL_STEPS + 1) * (angular_steps + 1));
    for row in 0..=VERTICAL_STEPS {
        let z = z_min + (z_max - z_min) * row as f64 / VERTICAL_STEPS as f64;
        let ring = radius * (z / z_max).sqrt();
        for column in 0..=angular_steps {
            let angle =
                std::f64::consts::TAU * (phi / 360.0) * column as f64 / angular_steps as f64;
            points.push([ring * angle.cos(), ring * angle.sin(), z]);
            // phi / phi_max at this vertex: the phi scale cancels, so
            // u runs 0..1 across whatever angular range `phimax` sweeps.
            uvs.push([
                column as f32 / angular_steps as f32,
                row as f32 / VERTICAL_STEPS as f32,
            ]);
        }
    }
    let mut indices = Vec::with_capacity(VERTICAL_STEPS * angular_steps * 6);
    for row in 0..VERTICAL_STEPS as u32 {
        for column in 0..angular_steps as u32 {
            let a = row * (angular_steps as u32 + 1) + column;
            let b = a + 1;
            let c = a + angular_steps as u32 + 1;
            let e = c + 1;
            if z_min == 0.0 && row == 0 {
                indices.extend_from_slice(&[b, e, c]);
            } else {
                indices.extend_from_slice(&[a, b, c, b, e, c]);
            }
        }
    }
    Ok((points, indices, uvs))
}

/// Tessellates PBRT's hyperboloid parameterization over its clipped angular range.
pub fn hyperboloid_mesh(d: &Directive) -> Result<crate::tessellate::Tessellated, SceneParseError> {
    const ANGULAR_STEPS: usize = 64;
    // 32 rows keep the chordal sagitta near r4's exact surface under a
    // preview pixel at grazing angles (the sphere tessellates 32 rows too).
    const PROFILE_STEPS: usize = 32;
    let params = d.params();
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    let p1_values = params.floats("p1");
    let p2_values = params.floats("p2");
    let p1 = if p1_values.is_empty() {
        [0.0, 0.0, 0.0]
    } else if p1_values.len() == 3 {
        [p1_values[0], p1_values[1], p1_values[2]]
    } else {
        return Err(fail("hyperboloid p1 must contain three coordinates"));
    };
    let p2 = if p2_values.is_empty() {
        [1.0, 1.0, 1.0]
    } else if p2_values.len() == 3 {
        [p2_values[0], p2_values[1], p2_values[2]]
    } else {
        return Err(fail("hyperboloid p2 must contain three coordinates"));
    };
    let phi = params.float("phimax", 360.0).clamp(0.0, 360.0);
    if p1.iter().chain(p2.iter()).any(|value| !value.is_finite())
        || !phi.is_finite()
        || phi <= 0.0
        || (p1[2] == p2[2])
        || (p1[2] == 0.0 && p2[2] == 0.0)
    {
        return Err(fail(
            "hyperboloid requires finite profile points and a non-empty z range",
        ));
    }
    let angular_steps = ((ANGULAR_STEPS as f64 * phi / 360.0).ceil() as usize).max(1);
    let mut points = Vec::with_capacity((PROFILE_STEPS + 1) * (angular_steps + 1));
    let mut uvs = Vec::with_capacity((PROFILE_STEPS + 1) * (angular_steps + 1));
    for row in 0..=PROFILE_STEPS {
        let v = row as f64 / PROFILE_STEPS as f64;
        let x = p1[0] * (1.0 - v) + p2[0] * v;
        let y = p1[1] * (1.0 - v) + p2[1] * v;
        let z = p1[2] * (1.0 - v) + p2[2] * v;
        for column in 0..=angular_steps {
            let angle =
                std::f64::consts::TAU * (phi / 360.0) * column as f64 / angular_steps as f64;
            points.push([
                x * angle.cos() - y * angle.sin(),
                x * angle.sin() + y * angle.cos(),
                z,
            ]);
            uvs.push([column as f32 / angular_steps as f32, v as f32]);
        }
    }
    let mut indices = Vec::with_capacity(PROFILE_STEPS * angular_steps * 6);
    for row in 0..PROFILE_STEPS as u32 {
        for column in 0..angular_steps as u32 {
            let a = row * (angular_steps as u32 + 1) + column;
            let b = a + 1;
            let c = a + angular_steps as u32 + 1;
            let e = c + 1;
            indices.extend_from_slice(&[a, b, c, b, e, c]);
        }
    }
    Ok((points, indices, uvs))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh_build::test_directive;

    /// r4 `shapes/paraboloid.rs`: u = phi / phi_max, v = (z - zmin) / (zmax - zmin)
    /// at each vertex's own parameters, so the seam column carries u = 1.
    #[test]
    fn paraboloid_uvs_follow_pbrt_at_every_vertex() {
        let source = "Shape \"paraboloid\" \"float radius\" [1] \"float zmin\" [0.25] \
\"float zmax\" [4] \"float phimax\" [300]";
        let (points, indices, uvs) = paraboloid_mesh(&test_directive(source)).unwrap();
        assert_eq!(points.len(), uvs.len());
        let (z_min, z_max, phi_max) = (0.25f64, 4f64, std::f64::consts::TAU * 300. / 360.);
        for (point, uv) in points.iter().zip(&uvs) {
            let phi = point[1].atan2(point[0]).rem_euclid(std::f64::consts::TAU);
            let u = phi / phi_max;
            let v = (point[2] - z_min) / (z_max - z_min);
            assert!((u - uv[0] as f64).abs() < 1e-5, "{point:?} {uv:?}");
            assert!((v - uv[1] as f64).abs() < 1e-5, "{point:?} {uv:?}");
        }
        // The seam u = 1 exists as its own column of vertices.
        assert!(uvs.iter().any(|uv| (uv[0] - 1.).abs() < 1e-6));
        assert!(indices.len().is_multiple_of(3));
    }

    /// r4 `shapes/hyperboloid.rs`: `phi = u * phi_max` rotates the profile point
    /// `pr(v) = lerp(p1, p2, v)`, so u is the rotation about that profile
    /// direction — atan2(pr.x·y − x·pr.y, x·pr.x + y·pr.y) — and v is the
    /// profile's own lerp parameter.
    #[test]
    fn hyperboloid_uvs_follow_pbrt_at_every_vertex() {
        let source = "Shape \"hyperboloid\" \"point3 p1\" [0.2 0.1 0] \"point3 p2\" [0.4 0.2 2] \
\"float phimax\" [300]";
        let (points, indices, uvs) = hyperboloid_mesh(&test_directive(source)).unwrap();
        assert_eq!(points.len(), uvs.len());
        let (p1, p2) = ([0.2f64, 0.1, 0.], [0.4f64, 0.2, 2.]);
        let phi_max = std::f64::consts::TAU * 300. / 360.;
        for (point, uv) in points.iter().zip(&uvs) {
            let v = (point[2] - p1[2]) / (p2[2] - p1[2]);
            let pr = [p1[0] + (p2[0] - p1[0]) * v, p1[1] + (p2[1] - p1[1]) * v];
            let rotation = (pr[0] * point[1] - point[0] * pr[1])
                .atan2(point[0] * pr[0] + point[1] * pr[1])
                .rem_euclid(std::f64::consts::TAU);
            let u = rotation / phi_max;
            assert!((u - uv[0] as f64).abs() < 1e-5, "{point:?} {uv:?}");
            assert!((v - uv[1] as f64).abs() < 1e-5, "{point:?} {uv:?}");
        }
        assert!(uvs.iter().any(|uv| (uv[0] - 1.).abs() < 1e-6));
        assert!(indices.len().is_multiple_of(3));
    }

    /// z may run downward, from p1.z down to p2.z: v still spans the profile 0..1
    /// with v = 0 at p1.
    #[test]
    fn hyperboloid_v_runs_from_p1_whichever_way_z_goes() {
        let source = "Shape \"hyperboloid\" \"point3 p1\" [0.2 0.1 2] \"point3 p2\" [0.4 0.2 0]";
        let (points, _, uvs) = hyperboloid_mesh(&test_directive(source)).unwrap();
        let z_of = |wanted: f32| {
            let (point, _) = points
                .iter()
                .zip(&uvs)
                .find(|(_, uv)| (uv[1] - wanted).abs() < 1e-6)
                .unwrap();
            point[2]
        };
        assert!((z_of(0.) - 2.).abs() < 1e-9, "v = 0 sits at p1.z = 2");
        assert!((z_of(1.) - 0.).abs() < 1e-9, "v = 1 sits at p2.z = 0");
    }
}
