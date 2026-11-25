use super::*;
use std::path::PathBuf;

/// Resolve the PLY asset a `plymesh` directive names, without reading it.
pub(super) fn resolve_ply_asset(directive: &crate::params::Directive, root_dir: &Path) -> PathBuf {
    let filename = directive.params().string("filename", "");
    let source_dir = Path::new(&directive.path)
        .parent()
        .unwrap_or(Path::new("."));
    let asset = Path::new(&filename);
    let candidates = if asset.is_absolute() {
        vec![asset.to_path_buf()]
    } else {
        vec![source_dir.join(asset), root_dir.join(asset)]
    };
    candidates
        .iter()
        .find(|candidate| candidate.is_file())
        .cloned()
        .unwrap_or_else(|| candidates[0].clone())
}

fn with_uvs(
    uvs: &mut Option<Vec<[f32; 2]>>,
    (points, indices, shape_uvs): tessellate::Tessellated,
) -> (Vec<[f64; 3]>, Vec<u32>) {
    *uvs = Some(shape_uvs);
    (points, indices)
}

pub(super) fn build(
    shape: &Shape,
    kind: &str,
    root_dir: &Path,
    options: &ParseOptions,
    ply_asset: Option<&[u8]>,
    uvs: &mut Option<Vec<[f32; 2]>>,
) -> Option<Result<(Vec<[f64; 3]>, Vec<u32>), SceneParseError>> {
    Some(match kind {
        "trianglemesh" => match mesh_build::triangle_mesh(&shape.d) {
            Ok(tessellated) => {
                *uvs = tessellated.2;
                Ok((tessellated.0, tessellated.1))
            }
            Err(error) => Err(error),
        },
        "bilinearmesh" => {
            mesh_build::bilinear_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated))
        }
        "curve" | "curves" => {
            mesh_build::curve_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated))
        }
        "loopsubdiv" => mesh_build::loop_subdiv(&shape.d),
        "paraboloid" => {
            mesh_build::paraboloid_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated))
        }
        "hyperboloid" => {
            mesh_build::hyperboloid_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated))
        }
        "heightfield" => {
            mesh_build::heightfield_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated))
        }
        "nurbs" => mesh_build::nurbs_mesh(&shape.d).map(|tessellated| with_uvs(uvs, tessellated)),
        "plymesh" => match ply_asset {
            Some(bytes) => crate::ply::read_ply(&shape.d.params().string("filename", ""), bytes),
            None => read_ply_asset(
                &resolve_ply_asset(&shape.d, root_dir),
                options.max_asset_bytes,
            ),
        }
        .map(|(points, indices, ply_uvs)| {
            *uvs = ply_uvs;
            (points, indices)
        }),
        "sphere" => {
            let p = shape.d.params();
            let r = p.float("radius", 1.);
            let (z0, z1) = (p.float("zmin", -r), p.float("zmax", r));
            let phi = p.float("phimax", 360.);
            if !r.is_finite() || r <= 0. || z0 >= z1 {
                Err(SceneParseError::syntax(
                    &shape.d.path,
                    shape.d.line,
                    shape.d.column,
                    "invalid sphere bounds",
                ))
            } else {
                Ok(with_uvs(
                    uvs,
                    tessellate::sphere(r, z0, z1, phi, options.angular_segments),
                ))
            }
        }
        "cylinder" => {
            let p = shape.d.params();
            let (r, z0, z1, phi) = (
                p.float("radius", 1.),
                p.float("zmin", 0.),
                p.float("zmax", 1.),
                p.float("phimax", 360.),
            );
            if r <= 0. || z0 >= z1 || !r.is_finite() {
                Err(SceneParseError::syntax(
                    &shape.d.path,
                    shape.d.line,
                    shape.d.column,
                    "invalid cylinder bounds",
                ))
            } else {
                Ok(with_uvs(
                    uvs,
                    tessellate::cylinder(r, z0, z1, phi, options.angular_segments),
                ))
            }
        }
        "disk" => {
            let p = shape.d.params();
            let (r, h, phi) = (
                p.float("radius", 1.),
                p.float("height", 0.),
                p.float("phimax", 360.),
            );
            if r <= 0. || !r.is_finite() {
                Err(SceneParseError::syntax(
                    &shape.d.path,
                    shape.d.line,
                    shape.d.column,
                    "invalid disk radius",
                ))
            } else {
                Ok(with_uvs(
                    uvs,
                    tessellate::disk(r, h, phi, options.angular_segments),
                ))
            }
        }
        "cone" => {
            let p = shape.d.params();
            let (r, h, phi) = (
                p.float("radius", 1.),
                p.float("height", 1.),
                p.float("phimax", 360.),
            );
            if r <= 0. || h <= 0. || !r.is_finite() || !h.is_finite() {
                Err(SceneParseError::syntax(
                    &shape.d.path,
                    shape.d.line,
                    shape.d.column,
                    "invalid cone bounds",
                ))
            } else {
                Ok(with_uvs(
                    uvs,
                    tessellate::cone(r, h, phi, options.angular_segments),
                ))
            }
        }
        _ => return None,
    })
}
