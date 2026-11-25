use super::*;

pub fn vertex_normals(points: &[[f64; 3]], indices: &[u32]) -> Vec<[f32; 3]> {
    let mut normals = vec![[0.; 3]; points.len()];
    for tri in indices.chunks_exact(3) {
        let (a, b, c) = (
            points[tri[0] as usize],
            points[tri[1] as usize],
            points[tri[2] as usize],
        );
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let v = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let n = [
            u[1] * v[2] - u[2] * v[1],
            u[2] * v[0] - u[0] * v[2],
            u[0] * v[1] - u[1] * v[0],
        ];
        for i in tri {
            for axis in 0..3 {
                normals[*i as usize][axis] += n[axis] as f32;
            }
        }
    }
    for n in &mut normals {
        let len = (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if len > 0. && len.is_finite() {
            for v in n {
                *v /= len
            }
        }
    }
    normals
}
/// Trianglemesh tessellation carrying optional texcoords for the lit preview.
pub type Tessellated = (Vec<[f64; 3]>, Vec<u32>, Option<Vec<[f32; 2]>>);
pub fn triangle_mesh(d: &Directive) -> Result<Tessellated, SceneParseError> {
    let params = d.params();
    let raw_points = params.floats("P");
    let raw_indices = params.floats("indices");
    if raw_points.is_empty()
        || raw_points.len() % 3 != 0
        || raw_indices.is_empty()
        || raw_indices.len() % 3 != 0
        || raw_indices
            .iter()
            .any(|v| !v.is_finite() || v.fract() != 0.0)
    {
        return Err(SceneParseError::syntax(
            &d.path,
            d.line,
            d.column,
            "trianglemesh requires point P and triangle indices",
        ));
    }
    let p = raw_points
        .chunks_exact(3)
        .map(|v| [v[0], v[1], v[2]])
        .collect::<Vec<_>>();
    let ix = raw_indices
        .into_iter()
        .map(|v| v as i64)
        .collect::<Vec<_>>();
    if ix.iter().any(|i| *i < 0 || *i as usize >= p.len()) {
        return Err(SceneParseError::syntax(
            &d.path,
            d.line,
            d.column,
            "trianglemesh index is outside point P",
        ));
    }
    // Texcoords ride along when declared; count must match point P.
    let raw_uv = params.floats("uv");
    let uvs = if raw_uv.is_empty() {
        None
    } else {
        if raw_uv.len() != p.len() * 2 {
            return Err(SceneParseError::syntax(
                &d.path,
                d.line,
                d.column,
                "trianglemesh uv count does not match point P",
            ));
        }
        Some(
            raw_uv
                .chunks_exact(2)
                .map(|pair| [pair[0] as f32, pair[1] as f32])
                .collect(),
        )
    };
    Ok((p, ix.into_iter().map(|i| i as u32).collect(), uvs))
}
