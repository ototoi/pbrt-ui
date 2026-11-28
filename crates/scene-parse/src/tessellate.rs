//! Display-only tessellation routines. Inputs are already validated scene values.
//! UVs follow pbrt's parameterization: u = phi / phi_max, v along the shape.

pub type Tessellated = (Vec<[f64; 3]>, Vec<u32>, Vec<[f32; 2]>);

pub fn sphere(radius: f64, z_min: f64, z_max: f64, phi_max: f64, segments: u32) -> Tessellated {
    let n = segments.max(3);
    let rows = (n / 2).max(2);
    let (z0, z1) = (z_min.max(-radius), z_max.min(radius));
    let t0 = (z0 / radius).clamp(-1., 1.).acos();
    let t1 = (z1 / radius).clamp(-1., 1.).acos();
    let mut p = Vec::new();
    let mut uv = Vec::new();
    for j in 0..=rows {
        let theta = t0 + (t1 - t0) * j as f64 / rows as f64;
        for i in 0..=n {
            let phi = phi_max.to_radians() * i as f64 / n as f64;
            p.push([
                radius * theta.sin() * phi.cos(),
                radius * theta.sin() * phi.sin(),
                radius * theta.cos(),
            ]);
            uv.push([i as f32 / n as f32, j as f32 / rows as f32]);
        }
    }
    let mut ix = Vec::new();
    for j in 0..rows {
        for i in 0..n {
            let a = j * (n + 1) + i;
            let b = a + n + 1;
            // Theta rows run from the lower z bound to the upper z bound,
            // which reverses the usual theta/phi parameterization winding.
            ix.extend([a, a + 1, b, a + 1, b + 1, b]);
        }
    }
    (p, ix, uv)
}
pub fn disk(radius: f64, height: f64, phi_max: f64, segments: u32) -> Tessellated {
    let n = segments.max(3);
    let mut p = Vec::new();
    let mut uv = Vec::new();
    // Rim at v = 0. Each wedge has its own center at v = 1, so that u does not
    // run around the whole circle across one wedge.
    for i in 0..=n {
        let a = phi_max.to_radians() * i as f64 / n as f64;
        p.push([radius * a.cos(), radius * a.sin(), height]);
        uv.push([i as f32 / n as f32, 0.]);
    }
    let mut ix = Vec::new();
    for i in 0..n {
        ix.extend([p.len() as u32, i, i + 1]);
        p.push([0., 0., height]);
        uv.push([(i as f32 + 0.5) / n as f32, 1.]);
    }
    (p, ix, uv)
}

pub fn cylinder(radius: f64, z_min: f64, z_max: f64, phi_max: f64, segments: u32) -> Tessellated {
    let n = segments.max(3);
    let mut p = Vec::with_capacity(((n + 1) * 2) as usize);
    let mut uv = Vec::with_capacity(p.capacity());
    for (v, z) in [(0., z_min), (1., z_max)] {
        for i in 0..=n {
            let a = phi_max.to_radians() * i as f64 / n as f64;
            p.push([radius * a.cos(), radius * a.sin(), z]);
            uv.push([i as f32 / n as f32, v]);
        }
    }
    let mut ix = Vec::with_capacity((n * 6) as usize);
    for i in 0..n {
        let a = i;
        let b = n + 1 + i;
        ix.extend([a, b, a + 1, a + 1, b, b + 1]);
    }
    (p, ix, uv)
}

pub fn cone(radius: f64, height: f64, phi_max: f64, segments: u32) -> Tessellated {
    let n = segments.max(3);
    let mut p = Vec::with_capacity(((n + 1) * 2) as usize);
    let mut uv = Vec::with_capacity(p.capacity());
    for (v, z, r) in [(0., 0., radius), (1., height, 0.)] {
        for i in 0..=n {
            let a = phi_max.to_radians() * i as f64 / n as f64;
            p.push([r * a.cos(), r * a.sin(), z]);
            uv.push([i as f32 / n as f32, v]);
        }
    }
    let mut ix = Vec::with_capacity((n * 6) as usize);
    for i in 0..n {
        let a = i;
        let b = n + 1 + i;
        ix.extend([a, b, a + 1, a + 1, b, b + 1]);
    }
    (p, ix, uv)
}

#[cfg(test)]
mod tests {
    use super::{cone, cylinder, disk, sphere};

    /// Each shape's uv at a vertex is pbrt's formula at that position.
    #[test]
    fn uvs_follow_pbrt_parameterization() {
        let u_of = |p: [f64; 3], uv: [f32; 2]| {
            let phi = p[1].atan2(p[0]).rem_euclid(std::f64::consts::TAU);
            let u = phi / std::f64::consts::TAU;
            assert!(
                (u - uv[0] as f64).abs() < 1e-4 || p[0].hypot(p[1]) < 1e-9 || uv[0] == 1.,
                "{p:?} {uv:?}"
            );
        };
        let (points, _, uvs) = sphere(2., -1., 2., 360., 16);
        assert_eq!(points.len(), uvs.len());
        let theta_min = (-0.5f64).acos();
        let theta_max = 1f64.acos();
        for (p, uv) in points.iter().zip(&uvs) {
            u_of(*p, *uv);
            let v = ((p[2] / 2.).clamp(-1., 1.).acos() - theta_min) / (theta_max - theta_min);
            assert!((v - uv[1] as f64).abs() < 1e-4, "{p:?} {uv:?}");
        }
        let (points, _, uvs) = cylinder(1., -1., 3., 360., 16);
        assert_eq!(points.len(), uvs.len());
        for (p, uv) in points.iter().zip(&uvs) {
            u_of(*p, *uv);
            assert_eq!((p[2] + 1.) / 4., uv[1] as f64);
        }
        let (points, _, uvs) = cone(1., 2., 360., 16);
        assert_eq!(points.len(), uvs.len());
        for (p, uv) in points.iter().zip(&uvs) {
            u_of(*p, *uv);
            assert_eq!(p[2] / 2., uv[1] as f64);
        }
        let (points, indices, uvs) = disk(2., 0., 360., 16);
        assert_eq!(points.len(), uvs.len());
        for (p, uv) in points.iter().zip(&uvs) {
            u_of(*p, *uv);
            // hypot is not correctly rounded; libms differ by an ulp.
            assert!((1. - p[0].hypot(p[1]) / 2. - uv[1] as f64).abs() < 1e-12);
        }
        assert_eq!(indices.len(), 48);
        // No wedge's center carries a u away from its rim edge.
        for wedge in indices.chunks(3) {
            let span = (uvs[wedge[2] as usize][0] - uvs[wedge[1] as usize][0]).abs();
            assert!((uvs[wedge[0] as usize][0] - uvs[wedge[1] as usize][0]).abs() <= span);
        }
    }

    #[test]
    fn sphere_triangles_face_outward() {
        let (points, indices, _) = sphere(1., -1., 1., 360., 16);
        let mut checked = 0;
        for triangle in indices.as_chunks::<3>().0.iter() {
            let a = points[triangle[0] as usize];
            let b = points[triangle[1] as usize];
            let c = points[triangle[2] as usize];
            let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
            let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
            let normal = [
                u[1] * v[2] - u[2] * v[1],
                u[2] * v[0] - u[0] * v[2],
                u[0] * v[1] - u[1] * v[0],
            ];
            let center = [
                (a[0] + b[0] + c[0]) / 3.,
                (a[1] + b[1] + c[1]) / 3.,
                (a[2] + b[2] + c[2]) / 3.,
            ];
            let area_squared = normal.iter().map(|n| n * n).sum::<f64>();
            if area_squared < 1e-20 {
                continue;
            }
            checked += 1;
            assert!(normal.iter().zip(center).map(|(n, p)| n * p).sum::<f64>() > 0.);
        }
        assert!(checked > 0);
    }
}
