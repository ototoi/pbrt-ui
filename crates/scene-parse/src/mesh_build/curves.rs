use super::*;
use crate::tessellate::Tessellated;

/// Builds a bounded preview mesh for a cubic/quadratic Bezier or uniform
/// B-spline curve. Curves are approximated by swept rings or a flat strip.
pub fn curve_mesh(d: &Directive) -> Result<Tessellated, SceneParseError> {
    const STEPS: usize = 24;
    const RADIAL: usize = 8;
    const MAX_SEGMENTS: usize = 256;
    let params = d.params();
    let raw_points = params.floats("P");
    let degree = params.int("degree", 3);
    let basis = params.string("basis", "bezier");
    let kind = params.string("type", "flat");
    let fail = |reason| SceneParseError::syntax(&d.path, d.line, d.column, reason);
    if !matches!(degree, 2 | 3)
        || !matches!(basis.as_str(), "bezier" | "bspline")
        || !matches!(kind.as_str(), "flat" | "ribbon" | "cylinder")
        || raw_points.is_empty()
        || !raw_points.len().is_multiple_of(3)
        || raw_points.iter().any(|value| !value.is_finite())
    {
        return Err(fail(
            "curve requires finite control points, a supported basis, degree and type",
        ));
    }
    let controls = raw_points
        .as_chunks::<3>()
        .0
        .iter()
        .map(|point| [point[0], point[1], point[2]])
        .collect::<Vec<_>>();
    let degree = degree as usize;
    let segments = match basis.as_str() {
        "bezier" if controls.len() > degree && (controls.len() - 1) % degree == 0 => {
            (controls.len() - 1) / degree
        }
        "bspline" if controls.len() > degree => controls.len() - degree,
        _ => {
            return Err(fail(
                "curve control point count does not match its basis and degree",
            ));
        }
    };
    if segments == 0 || segments > MAX_SEGMENTS {
        return Err(fail("curve exceeds the preview segment limit"));
    }
    let width = params.float("width", 1.0);
    let width0 = params.float("width0", width);
    let width1 = params.float("width1", width);
    if !width0.is_finite() || !width1.is_finite() || width0 <= 0.0 || width1 <= 0.0 {
        return Err(fail("curve widths must be finite and positive"));
    }
    let normals = params.floats("N");
    let normals = if kind == "ribbon" {
        if normals.len() != (segments + 1) * 3 || normals.iter().any(|value| !value.is_finite()) {
            return Err(fail(
                "ribbon curves require one finite normal per segment endpoint",
            ));
        }
        Some(
            normals
                .as_chunks::<3>()
                .0
                .iter()
                .map(|n| [n[0], n[1], n[2]])
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let mut points = Vec::new();
    let mut indices = Vec::new();
    let mut uvs = Vec::new();
    let sides = if kind == "cylinder" { RADIAL } else { 1 };
    let ring_vertices = sides + 1;
    for segment in 0..segments {
        let base = segment * degree;
        let bezier = match (basis.as_str(), degree) {
            ("bezier", 3) => [
                controls[base],
                controls[base + 1],
                controls[base + 2],
                controls[base + 3],
            ],
            ("bezier", 2) => {
                let a = controls[base];
                let b = controls[base + 1];
                let c = controls[base + 2];
                [a, lerp3(a, b, 2.0 / 3.0), lerp3(c, b, 2.0 / 3.0), c]
            }
            ("bspline", 3) => {
                let p0 = controls[base];
                let p1 = controls[base + 1];
                let p2 = controls[base + 2];
                let p3 = controls[base + 3];
                [
                    std::array::from_fn(|axis| (p0[axis] + 4.0 * p1[axis] + p2[axis]) / 6.0),
                    lerp3(p1, p2, 1.0 / 3.0),
                    lerp3(p1, p2, 2.0 / 3.0),
                    std::array::from_fn(|axis| (p1[axis] + 4.0 * p2[axis] + p3[axis]) / 6.0),
                ]
            }
            ("bspline", 2) => {
                let a = controls[base];
                let b = controls[base + 1];
                let c = controls[base + 2];
                let start = lerp3(a, b, 0.5);
                let end = lerp3(c, b, 0.5);
                [
                    start,
                    lerp3(start, b, 2.0 / 3.0),
                    lerp3(end, b, 2.0 / 3.0),
                    end,
                ]
            }
            _ => unreachable!(),
        };
        let start = points.len() as u32;
        for step in 0..=STEPS {
            let t = step as f64 / STEPS as f64;
            let u = ((segment as f64 + t) / segments as f64) as f32;
            let s = 1.0 - t;
            let p: [f64; 3] = std::array::from_fn(|axis| {
                s * s * s * bezier[0][axis]
                    + 3.0 * s * s * t * bezier[1][axis]
                    + 3.0 * s * t * t * bezier[2][axis]
                    + t * t * t * bezier[3][axis]
            });
            let tangent = std::array::from_fn(|axis| {
                3.0 * s * s * (bezier[1][axis] - bezier[0][axis])
                    + 6.0 * s * t * (bezier[2][axis] - bezier[1][axis])
                    + 3.0 * t * t * (bezier[3][axis] - bezier[2][axis])
            });
            let tangent =
                normalize3(tangent).ok_or_else(|| fail("curve contains a degenerate tangent"))?;
            let width_t = width0 * (1.0 - (segment as f64 + t) / segments as f64)
                + width1 * ((segment as f64 + t) / segments as f64);
            let normal = if let Some(normals) = &normals {
                normalize3(lerp3(normals[segment], normals[segment + 1], t))
                    .ok_or_else(|| fail("ribbon normal is degenerate"))?
            } else {
                let axis = if tangent[0].abs() <= tangent[1].abs()
                    && tangent[0].abs() <= tangent[2].abs()
                {
                    [1., 0., 0.]
                } else if tangent[1].abs() <= tangent[2].abs() {
                    [0., 1., 0.]
                } else {
                    [0., 0., 1.]
                };
                normalize3(cross3(tangent, axis))
                    .ok_or_else(|| fail("curve frame is degenerate"))?
            };
            let binormal = normalize3(cross3(tangent, normal))
                .ok_or_else(|| fail("curve frame is degenerate"))?;
            if kind == "cylinder" {
                for side in 0..=RADIAL {
                    let angle = std::f64::consts::TAU * side as f64 / RADIAL as f64;
                    let offset: [f64; 3] = std::array::from_fn(|axis| {
                        width_t * 0.5 * (normal[axis] * angle.cos() + binormal[axis] * angle.sin())
                    });
                    points.push(std::array::from_fn(|axis| p[axis] + offset[axis]));
                    uvs.push([u, side as f32 / RADIAL as f32]);
                }
            } else {
                let across = if kind == "ribbon" {
                    normalize3(cross3(normal, tangent))
                        .ok_or_else(|| fail("ribbon direction is degenerate"))?
                } else {
                    normal
                };
                points.push(std::array::from_fn(|axis| {
                    p[axis] - across[axis] * width_t * 0.5
                }));
                uvs.push([u, 0.]);
                points.push(std::array::from_fn(|axis| {
                    p[axis] + across[axis] * width_t * 0.5
                }));
                uvs.push([u, 1.]);
            }
        }
        for step in 0..STEPS as u32 {
            for side in 0..sides as u32 {
                let a = start + step * ring_vertices as u32 + side;
                let b = a + 1;
                let c = a + ring_vertices as u32;
                let e = c + 1;
                indices.extend_from_slice(&[a, c, b, b, c, e]);
            }
        }
    }
    Ok((points, indices, uvs))
}
fn lerp3(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    std::array::from_fn(|i| a[i] * (1.0 - t) + b[i] * t)
}
fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize3(v: [f64; 3]) -> Option<[f64; 3]> {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    (length > 1e-12 && length.is_finite()).then(|| v.map(|value| value / length))
}

#[cfg(test)]
mod tests {
    use super::curve_mesh;
    use crate::{lexer::Lexer, params::DirectiveReader};

    const STEPS: usize = 24;
    const RADIAL: usize = 8;

    fn curve(kind: &str) -> crate::params::Directive {
        let normals = if kind == "ribbon" {
            " \"point3 N\" [0 0 1 0 0 1 0 0 1]"
        } else {
            ""
        };
        let source = format!(
            "Shape \"curve\" \"point3 P\" [0 0 0 0.2 0.5 0 0.8 0.5 0 1 0 0 1.2 -0.5 0 1.8 -0.5 0 2 0 0] \"string type\" [\"{kind}\"] \"float width\" [0.2]{normals}"
        );
        let mut lexer = Lexer::new("curve-test.pbrt", &source);
        let mut reader = DirectiveReader::new(&mut lexer);
        reader.read_directive().unwrap().unwrap()
    }

    #[test]
    fn curve_uvs_cover_two_bezier_segments_and_each_supported_type() {
        for kind in ["flat", "ribbon", "cylinder"] {
            let (points, indices, uvs) = curve_mesh(&curve(kind)).unwrap();
            let ring = if kind == "cylinder" { RADIAL + 1 } else { 2 };
            assert_eq!(uvs.len(), points.len(), "{kind}");
            assert_eq!(points.len(), 2 * (STEPS + 1) * ring, "{kind}");
            assert_eq!(
                indices.len(),
                2 * STEPS * if kind == "cylinder" { RADIAL } else { 1 } * 6,
                "{kind}"
            );
            assert!(
                uvs.iter()
                    .flatten()
                    .all(|uv| uv.is_finite() && (0. ..=1.).contains(uv))
            );

            for segment in 0..2 {
                let base = segment * (STEPS + 1) * ring;
                let start = uvs[base][0];
                let boundary = uvs[base + STEPS * ring][0];
                assert!((start - segment as f32 / 2.).abs() < 1e-6, "{kind} {start}");
                assert!((boundary - (segment + 1) as f32 / 2.).abs() < 1e-6);
                for step in 0..=STEPS {
                    let u = uvs[base + step * ring][0];
                    let expected = (segment as f32 + step as f32 / STEPS as f32) / 2.;
                    assert!(
                        (u - expected).abs() < 1e-6,
                        "{kind} {segment} {step}: {u} != {expected}"
                    );
                    if kind == "cylinder" {
                        for side in 0..=RADIAL {
                            assert_eq!(
                                uvs[base + step * ring + side][1],
                                side as f32 / RADIAL as f32
                            );
                        }
                        let seam_a = points[base + step * ring];
                        let seam_b = points[base + step * ring + RADIAL];
                        assert!(seam_a.iter().zip(seam_b).all(|(a, b)| (a - b).abs() < 1e-9));
                    } else {
                        assert_eq!(uvs[base + step * ring][1], 0.);
                        assert_eq!(uvs[base + step * ring + 1][1], 1.);
                    }
                }
            }
            // Each Bezier segment duplicates its shared endpoint and maps the
            // first endpoint of the next segment to the same continuous u.
            assert_eq!(uvs[STEPS * ring][0], uvs[(STEPS + 1) * ring][0]);
        }
    }
}
