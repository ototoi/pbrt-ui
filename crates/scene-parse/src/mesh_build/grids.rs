use super::*;

/// Tessellates bilinear quad patches to a bounded regular grid for viewport use.
/// Four control points are ordered as p00, p10, p01, p11.
pub fn bilinear_mesh(d: &Directive) -> Result<crate::tessellate::Tessellated, SceneParseError> {
    const SUBDIVISIONS: usize = 8;
    const MAX_PATCHES: usize = 16_384;
    let params = d.params();
    let raw_points = params.floats("P");
    let raw_indices = params.floats("indices");
    let raw_indices =
        if raw_indices.is_empty() && raw_points.len() >= 12 && raw_points.len() % 12 == 0 {
            (0..raw_points.len() / 3)
                .map(|index| index as f64)
                .collect()
        } else {
            raw_indices
        };
    let invalid = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    if raw_points.is_empty()
        || raw_points.len() % 3 != 0
        || raw_points.iter().any(|value| !value.is_finite())
        || raw_indices.is_empty()
        || raw_indices.len() % 4 != 0
        || raw_indices.len() / 4 > MAX_PATCHES
        || raw_indices.iter().any(|value| {
            !value.is_finite()
                || value.fract() != 0.0
                || *value < 0.0
                || *value >= (raw_points.len() / 3) as f64
        })
    {
        return Err(invalid(
            "bilinearmesh requires finite point P and valid quad indices",
        ));
    }
    let controls = raw_points
        .chunks_exact(3)
        .map(|point| [point[0], point[1], point[2]])
        .collect::<Vec<_>>();
    // r4 `bilinearmesh.rs`: per control point; "uv" wins over the legacy "st".
    let mut control_uvs = params.floats("uv");
    if control_uvs.is_empty() {
        control_uvs = params.floats("st");
    }
    if !control_uvs.is_empty() && control_uvs.len() != controls.len() * 2 {
        return Err(invalid(
            "bilinearmesh uv must be empty or have the same length as point P",
        ));
    }
    let control_uvs = control_uvs
        .chunks_exact(2)
        .map(|pair| [pair[0] as f32, pair[1] as f32])
        .collect::<Vec<_>>();
    let mut points = Vec::with_capacity(raw_indices.len() / 4 * (SUBDIVISIONS + 1).pow(2));
    let mut uvs = Vec::with_capacity(raw_indices.len() / 4 * (SUBDIVISIONS + 1).pow(2));
    let mut triangles = Vec::with_capacity(raw_indices.len() / 4 * SUBDIVISIONS.pow(2) * 6);
    let interpolate = |corners: &[[f64; 3]], corner_uvs: Option<&[[f32; 2]]>, u, v| {
        let point = std::array::from_fn(|axis| {
            (1.0 - u) * (1.0 - v) * corners[0][axis]
                + u * (1.0 - v) * corners[1][axis]
                + (1.0 - u) * v * corners[2][axis]
                + u * v * corners[3][axis]
        });
        let uv = corner_uvs.map_or([u as f32, v as f32], |corners| {
            [
                ((1.0 - u) * (1.0 - v) * corners[0][0] as f64
                    + u * (1.0 - v) * corners[1][0] as f64
                    + (1.0 - u) * v * corners[2][0] as f64
                    + u * v * corners[3][0] as f64) as f32,
                ((1.0 - u) * (1.0 - v) * corners[0][1] as f64
                    + u * (1.0 - v) * corners[1][1] as f64
                    + (1.0 - u) * v * corners[2][1] as f64
                    + u * v * corners[3][1] as f64) as f32,
            ]
        });
        (point, uv)
    };
    for face in raw_indices.chunks_exact(4) {
        let corners = face
            .iter()
            .map(|index| controls[*index as usize])
            .collect::<Vec<_>>();
        let corner_uvs = (!control_uvs.is_empty()).then(|| {
            face.iter()
                .map(|index| control_uvs[*index as usize])
                .collect::<Vec<_>>()
        });
        let base = u32::try_from(points.len())
            .map_err(|_| invalid("bilinearmesh preview exceeds index range"))?;
        for v in 0..=SUBDIVISIONS {
            for u in 0..=SUBDIVISIONS {
                let (u, v) = (
                    u as f64 / SUBDIVISIONS as f64,
                    v as f64 / SUBDIVISIONS as f64,
                );
                let (point, uv) = interpolate(&corners, corner_uvs.as_deref(), u, v);
                points.push(point);
                uvs.push(uv);
            }
        }
        let side = (SUBDIVISIONS + 1) as u32;
        for y in 0..SUBDIVISIONS as u32 {
            for x in 0..SUBDIVISIONS as u32 {
                let a = base + y * side + x;
                let b = a + 1;
                let c = a + side + 1;
                let d = a + side;
                triangles.extend_from_slice(&[a, b, c, a, c, d]);
            }
        }
    }
    Ok((points, triangles, uvs))
}

/// Converts PBRT's regular height grid into two triangles per cell.
pub fn heightfield_mesh(d: &Directive) -> Result<crate::tessellate::Tessellated, SceneParseError> {
    const MAX_VERTICES: usize = 1_000_000;
    let params = d.params();
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    let nu = params.int("nu", -1);
    let nv = params.int("nv", -1);
    let heights = params.floats("Pz");
    if nu < 2 || nv < 2 {
        return Err(fail("heightfield requires nu and nv of at least 2"));
    }
    let (nu, nv) = (nu as usize, nv as usize);
    let vertex_count = nu
        .checked_mul(nv)
        .ok_or_else(|| fail("heightfield dimensions overflow"))?;
    if vertex_count > MAX_VERTICES
        || heights.len() != vertex_count
        || heights.iter().any(|height| !height.is_finite())
    {
        return Err(fail(
            "heightfield requires a finite Pz grid within the preview size limit",
        ));
    }
    let mut points = Vec::with_capacity(vertex_count);
    let mut uvs = Vec::with_capacity(vertex_count);
    for (index, z) in heights.iter().enumerate() {
        let (x, y) = (index % nu, index / nu);
        // r4 `shapes/heightfield.rs`: uvs[pos] = (x / (nx - 1), y / (ny - 1)).
        uvs.push([x as f32 / (nu - 1) as f32, y as f32 / (nv - 1) as f32]);
        points.push([x as f64 / (nu - 1) as f64, y as f64 / (nv - 1) as f64, *z]);
    }
    let mut indices = Vec::with_capacity((nu - 1) * (nv - 1) * 6);
    for y in 0..nv - 1 {
        for x in 0..nu - 1 {
            let a = (x + y * nu) as u32;
            let b = a + 1;
            let c = a + nu as u32 + 1;
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

    /// r4 `shapes/heightfield.rs`: uvs[pos] = (x / (nx - 1), y / (ny - 1)),
    /// exactly the grid vertex's normalized x and y.
    #[test]
    fn heightfield_uvs_match_the_grid_position() {
        let source = "Shape \"heightfield\" \"integer nu\" [3] \"integer nv\" [4] \
\"float Pz\" [1 2 3 4 5 6 7 8 9 10 11 12]";
        let (points, indices, uvs) = heightfield_mesh(&test_directive(source)).unwrap();
        assert_eq!(points.len(), uvs.len());
        for (point, uv) in points.iter().zip(&uvs) {
            assert!((point[0] - uv[0] as f64).abs() < 1e-6, "{point:?} {uv:?}");
            assert!((point[1] - uv[1] as f64).abs() < 1e-6, "{point:?} {uv:?}");
        }
        assert!((uvs[0][0], uvs[0][1]) == (0., 0.));
        assert!((uvs.last().unwrap()[0] - 1.).abs() < 1e-6);
        assert!((uvs.last().unwrap()[1] - 1.).abs() < 1e-6);
        assert_eq!(indices.chunks_exact(3).count() * 3, indices.len());
    }

    fn bilinear_quad() -> &'static str {
        "Shape \"bilinearmesh\" \"point3 P\" \
[-1 -1 0  1 -1 1  1 1 0  -1 1 -1] \"integer indices\" [0 1 2 3]"
    }

    /// Without "uv", r4 uses the patch's own (u, v) as the texcoords
    /// (`bilinearmesh.rs`: `st = uv` when the mesh has none).
    #[test]
    fn bilinear_mesh_without_uv_keeps_the_patch_parameters() {
        let (points, indices, uvs) = bilinear_mesh(&test_directive(bilinear_quad())).unwrap();
        assert_eq!(points.len(), uvs.len());
        let side = 9; // SUBDIVISIONS + 1, per patch.
        for (index, (point, uv)) in points.iter().zip(&uvs).enumerate() {
            let (x, y) = ((index % side) as f32, (index / side) as f32);
            assert_eq!(*uv, [x / 8., y / 8.], "{point:?}");
        }
        assert_eq!(indices.chunks_exact(3).count() * 3, indices.len());
    }

    /// With "uv", the four corners' texcoords (per control point, corner order
    /// p00, p10, p01, p11) are bilinearly interpolated like the position
    /// (r4 `bilinearmesh.rs`, `interpolate_uv`).
    #[test]
    fn bilinear_mesh_interpolates_corner_uvs_bilinearly() {
        let source = "Shape \"bilinearmesh\" \"point3 P\" \
[-1 -1 0  1 -1 1  1 1 0  -1 1 -1] \"integer indices\" [0 1 2 3] \
\"texcoord2 uv\" [0.1 0.2  0.9 0.3  0.3 0.7  0.8 0.8]";
        let (points, _, uvs) = bilinear_mesh(&test_directive(source)).unwrap();
        assert_eq!(points.len(), uvs.len());
        let corners = [[0.1f32, 0.2], [0.9, 0.3], [0.3, 0.7], [0.8, 0.8]];
        let side = 9;
        for (index, uv) in uvs.iter().enumerate() {
            let (u, v) = ((index % side) as f32 / 8., (index / side) as f32 / 8.);
            let expected = [
                (1. - u) * (1. - v) * corners[0][0]
                    + u * (1. - v) * corners[1][0]
                    + (1. - u) * v * corners[2][0]
                    + u * v * corners[3][0],
                (1. - u) * (1. - v) * corners[0][1]
                    + u * (1. - v) * corners[1][1]
                    + (1. - u) * v * corners[2][1]
                    + u * v * corners[3][1],
            ];
            assert!(
                (uv[0] - expected[0]).abs() < 1e-5 && (uv[1] - expected[1]).abs() < 1e-5,
                "{uv:?} vs {expected:?} at ({u}, {v})"
            );
        }
    }

    /// r4 falls back from "uv" to "st", and rejects a count that does not
    /// match the control points.
    #[test]
    fn bilinear_mesh_uv_falls_back_to_st_and_validates_the_count() {
        let st = "Shape \"bilinearmesh\" \"point3 P\" \
[-1 -1 0  1 -1 1  1 1 0  -1 1 -1] \"integer indices\" [0 1 2 3] \
\"texcoord2 st\" [0 0  1 0  0 1  1 1]";
        let (points, _, uvs) = bilinear_mesh(&test_directive(st)).unwrap();
        assert_eq!(uvs[4], [0.5, 0.0], "st feeds the same corners");
        assert_eq!(points.len(), uvs.len());
        let wrong = "Shape \"bilinearmesh\" \"point3 P\" \
[-1 -1 0  1 -1 1  1 1 0  -1 1 -1] \"integer indices\" [0 1 2 3] \
\"texcoord2 uv\" [0 0  1 0]";
        assert!(bilinear_mesh(&test_directive(wrong)).is_err());
    }
}
