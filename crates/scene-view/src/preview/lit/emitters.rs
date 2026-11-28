use super::lights::{LightColor, light_color};
use super::{DISK_SIDES, GpuLight, LIGHT_POLYGON, LIGHT_SPHERE, MAX_EMITTER_TRIANGLES};
use protocol::geometry::{AreaLightRecord, DrawRecord, EmitterShape, MeshRecord, PreviewParameter};
use std::collections::BTreeMap;
use std::f64::consts::PI;

/// Object-space emitter geometry cached per mesh.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct EmitterGeometry {
    /// Mesh-local triangles. Mesh positions are stored relative to
    /// `MeshRecord::origin_f64`, and draw matrices expect that frame.
    pub triangles: Vec<[[f64; 3]; 3]>,
    /// The full sphere or disk the mesh tessellates, in shape space.
    pub shape: Option<EmitterShape>,
    /// `MeshRecord::origin_f64`: the shape-space point at mesh-local zero.
    pub origin: [f64; 3],
}

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum WorldEmitter {
    Polygons(Vec<Vec<[f64; 3]>>),
    Sphere { center: [f64; 3], radius: f64 },
}

#[derive(Clone, Debug, Default)]
pub(crate) struct FrameEmitters {
    pub lights: Vec<GpuLight>,
    /// World-space polygon vertices, kept in f64 until the eye-relative
    /// upload so distant emitters keep their shape.
    pub vertices: Vec<[f64; 3]>,
    /// World-space position per light (centroid or sphere center).
    pub positions: Vec<[f64; 3]>,
    pub draw_of_light: Vec<usize>,
    pub over_budget: bool,
    /// Per light: why its `L`/`emission` could only be approximated, for
    /// the LitNotes `rebind` keeps once.
    pub light_spectrum_notes: Vec<String>,
    /// Per draw with an emitter: the radiance its own surface shows —
    /// resolved colour × `scale`, power-matched, before the over-budget
    /// sphere correction reshapes only the contribution. The
    /// `KIND_EMISSIVE` appearance palette and `rebind`'s matching both
    /// read this, so an emitter's face can never disagree with the light
    /// it emits.
    pub surface_of_draw: BTreeMap<usize, [f64; 3]>,
}

pub(crate) fn emitter_geometry(
    mesh: &MeshRecord,
    shape: Option<&EmitterShape>,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> Option<EmitterGeometry> {
    let blob = blobs.get(&mesh.blob)?;
    let positions = protocol::geometry::read_f32x3_array(blob, &mesh.positions).ok()?;
    let indices = protocol::geometry::read_indices(blob, &mesh.indices).ok()?;
    let triangles = indices
        .as_chunks::<3>()
        .0
        .iter()
        .map(|tri| [tri[0], tri[1], tri[2]].map(|index| positions[index as usize].map(f64::from)))
        .collect();
    Some(EmitterGeometry {
        triangles,
        shape: shape.cloned(),
        origin: mesh.origin_f64,
    })
}

pub(crate) fn world_emitter(
    geometry: &EmitterGeometry,
    matrix: [f64; 16],
    reverse: bool,
) -> WorldEmitter {
    let transform = |p: [f64; 3]| -> [f64; 3] {
        [
            matrix[0] * p[0] + matrix[1] * p[1] + matrix[2] * p[2] + matrix[3],
            matrix[4] * p[0] + matrix[5] * p[1] + matrix[6] * p[2] + matrix[7],
            matrix[8] * p[0] + matrix[9] * p[1] + matrix[10] * p[2] + matrix[11],
        ]
    };
    let flip = reverse ^ (determinant(matrix) < 0.);
    // Shape-space points: move into the mesh-local frame first.
    let shape_point = |p: [f64; 3]| transform(sub(p, geometry.origin));
    match (&geometry.shape, similarity_scale(matrix)) {
        (Some(EmitterShape::Sphere { radius }), Some(scale)) => WorldEmitter::Sphere {
            center: shape_point([0.; 3]),
            radius: radius * scale,
        },
        (Some(EmitterShape::Disk { radius, height }), Some(_)) => {
            // Equal area to the disk: the inscribed 16-gon is 2.6% smaller.
            let vertex_radius = radius * (PI / (8. * (PI / 8.).sin())).sqrt();
            let mut polygon = (0..DISK_SIDES)
                .map(|i| {
                    let angle = 2. * PI * i as f64 / DISK_SIDES as f64;
                    shape_point([
                        vertex_radius * angle.cos(),
                        vertex_radius * angle.sin(),
                        *height,
                    ])
                })
                .collect::<Vec<_>>();
            if flip {
                polygon.reverse();
            }
            WorldEmitter::Polygons(vec![polygon])
        }
        // Triangle meshes, and spheres or disks under a non-similar transform.
        _ => WorldEmitter::Polygons(
            geometry
                .triangles
                .iter()
                .map(|triangle| {
                    let mut polygon = triangle.iter().copied().map(transform).collect::<Vec<_>>();
                    if flip {
                        polygon.reverse();
                    }
                    polygon
                })
                .collect(),
        ),
    }
}

pub(crate) fn frame_emitters(
    draws: &[DrawRecord],
    geometry: &BTreeMap<u64, EmitterGeometry>,
    time_index: usize,
) -> FrameEmitters {
    let mut frame = FrameEmitters::default();
    let mut used_triangles = 0usize;
    for (draw_index, draw) in draws.iter().enumerate() {
        let (Some(area), Some(geometry), Some(matrix)) = (
            draw.area_light.as_ref(),
            geometry.get(&draw.mesh_id),
            draw.matrix_at(time_index),
        ) else {
            continue;
        };
        let world = world_emitter(geometry, matrix, draw.reverse_orientation);
        let twosided = bool_param(&area.parameters, "twosided");
        let (area_world, center) = world_area_center(&world);
        // Triangles this emitter takes from the budget; a disk's 16-gon is
        // drawn as a fan of 14.
        let tri_count = match &world {
            WorldEmitter::Polygons(polys) => polys
                .iter()
                .map(|polygon| polygon.len().saturating_sub(2))
                .sum(),
            WorldEmitter::Sphere { .. } => 0,
        };
        let fallback = tri_count > 0 && used_triangles + tri_count > MAX_EMITTER_TRIANGLES;
        frame.over_budget |= fallback;
        if !fallback {
            used_triangles += tri_count;
        }
        match if fallback {
            WorldEmitter::Sphere {
                center,
                radius: bounds_radius(&world, center),
            }
        } else {
            world
        } {
            WorldEmitter::Sphere { center, radius } => {
                let area_equivalent = if fallback {
                    area_world
                } else {
                    4. * PI * radius * radius
                };
                let surface =
                    surface_radiance(area, Some(area_equivalent), &mut frame.light_spectrum_notes);
                frame.surface_of_draw.insert(draw_index, surface);
                // The stand-in sphere radiates the mesh's power:
                // π·L'·4πR² = π·L·A; its own face keeps the radiance above.
                let rgb = if fallback {
                    let new_area = 4. * PI * radius * radius;
                    surface.map(|v| v * area_world * (if twosided { 2. } else { 1. }) / new_area)
                } else {
                    surface
                };
                frame.lights.push(GpuLight {
                    position: [center[0] as f32, center[1] as f32, center[2] as f32, 0.],
                    // direction.w carries the radius, as for polygons'
                    // equivalent radius (common.wgsl's Light).
                    direction: [0., 0., 0., radius as f32],
                    color: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 0.],
                    kind: [LIGHT_SPHERE, twosided as u32, 0, 0],
                });
                frame.positions.push(center);
                frame.draw_of_light.push(draw_index);
            }
            WorldEmitter::Polygons(polygons) => {
                let light_index = frame.lights.len();
                let start = frame.vertices.len() as u32;
                let mut count = 0u32;
                for polygon in &polygons {
                    if polygon.len() == 3 {
                        append_triangle(
                            &mut frame.vertices,
                            &mut count,
                            polygon[0],
                            polygon[1],
                            polygon[2],
                        );
                    } else if polygon.len() > 3 {
                        for i in 1..polygon.len() - 1 {
                            append_triangle(
                                &mut frame.vertices,
                                &mut count,
                                polygon[0],
                                polygon[i],
                                polygon[i + 1],
                            );
                        }
                    }
                }
                let centroid = polygons
                    .iter()
                    .flatten()
                    .fold([0.; 3], |mut sum, p| {
                        for axis in 0..3 {
                            sum[axis] += p[axis];
                        }
                        sum
                    })
                    // Mean of the vertices summed (16 for a disk, not the
                    // 42 its fan triangulation uploads).
                    .map(|v| v / polygons.iter().map(Vec::len).sum::<usize>().max(1) as f64);
                let area_value = polygons.iter().map(|poly| polygon_area(poly)).sum::<f64>();
                let surface =
                    surface_radiance(area, Some(area_value), &mut frame.light_spectrum_notes);
                frame.surface_of_draw.insert(draw_index, surface);
                let rgb = surface;
                frame.lights.push(GpuLight {
                    position: [
                        centroid[0] as f32,
                        centroid[1] as f32,
                        centroid[2] as f32,
                        0.,
                    ],
                    direction: [0., 0., 1., (area_value / PI).sqrt() as f32],
                    color: [rgb[0] as f32, rgb[1] as f32, rgb[2] as f32, 0.],
                    kind: [LIGHT_POLYGON, twosided as u32, start, count],
                });
                frame.positions.push(centroid);
                frame.draw_of_light.push(draw_index);
                let _ = light_index;
            }
        }
    }
    frame
}

fn emission(area: &AreaLightRecord) -> LightColor {
    light_color(&area.parameters, &["L", "emission"], [1.; 3])
}

/// One emitter's surface radiance — the brightness both its own face and
/// the light it emits carry: the resolved `L`/`emission` colour ×
/// `scale`, power-matched over `area_world` when `power` is given, the
/// same recalculation r4's DiffuseAreaLight applies to the one radiance
/// that shades its emission and its look. `area_world` is None when no
/// geometry exists for the draw: no area means no power to match against
/// (and `frame_emitters` skips that draw's contribution too), so the
/// surface keeps the scale-only radiance. Spectrum approximations carry
/// to `spectrum_notes` for `rebind` to keep once.
pub(super) fn surface_radiance(
    area: &AreaLightRecord,
    area_world: Option<f64>,
    spectrum_notes: &mut Vec<String>,
) -> [f64; 3] {
    let resolved = emission(area);
    if let Some(reason) = resolved.degraded {
        spectrum_notes.push(reason);
    }
    let scale = scalar(&area.parameters, "scale", 1.);
    let power = scalar(&area.parameters, "power", 0.);
    let factor = match area_world {
        Some(area_world) if power > 0. => {
            power
                / (PI
                    * (if bool_param(&area.parameters, "twosided") {
                        2.
                    } else {
                        1.
                    })
                    * area_world.max(1e-20))
        }
        _ => 1.,
    };
    resolved.rgb.map(|v| v * scale * factor)
}
fn append_triangle(
    vertices: &mut Vec<[f64; 3]>,
    count: &mut u32,
    a: [f64; 3],
    b: [f64; 3],
    c: [f64; 3],
) {
    for point in [a, b, c] {
        vertices.push(point);
        *count += 1;
    }
}
fn scalar(p: &[PreviewParameter], n: &str, d: f64) -> f64 {
    p.iter()
        .find(|v| v.matches(n))
        .and_then(|v| v.floats.first())
        .copied()
        .unwrap_or(d)
}
fn bool_param(p: &[PreviewParameter], n: &str) -> bool {
    p.iter().find(|v| v.matches(n)).is_some_and(|v| {
        v.floats.first().is_some_and(|x| *x != 0.)
            || v.texts
                .first()
                .is_some_and(|x| x.eq_ignore_ascii_case("true"))
    })
}
/// The uniform scale of a similarity transform: columns of equal length
/// and pairwise orthogonal. Anything else (non-uniform scale, shear) turns
/// a sphere into an ellipsoid.
fn similarity_scale(m: [f64; 16]) -> Option<f64> {
    let columns = [0, 1, 2].map(|c| [m[c], m[4 + c], m[8 + c]]);
    let lengths = columns.map(length);
    let mean = (lengths[0] + lengths[1] + lengths[2]) / 3.;
    let equal = lengths.iter().all(|v| (v - mean).abs() <= mean * 1e-6);
    let orthogonal = [(0, 1), (1, 2), (0, 2)]
        .iter()
        .all(|&(a, b)| dot(columns[a], columns[b]).abs() <= mean * mean * 1e-6);
    (mean > 0. && equal && orthogonal).then_some(mean)
}
fn determinant(m: [f64; 16]) -> f64 {
    m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
        + m[2] * (m[4] * m[9] - m[5] * m[8])
}
#[cfg(test)]
fn identity() -> [f64; 16] {
    [
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
    ]
}
fn polygon_area(p: &[[f64; 3]]) -> f64 {
    if p.len() < 3 {
        return 0.;
    }
    let origin = p[0];
    (1..p.len() - 1)
        .map(|i| length(cross(sub(p[i], origin), sub(p[i + 1], origin))) * 0.5)
        .sum()
}
fn world_area_center(world: &WorldEmitter) -> (f64, [f64; 3]) {
    match world {
        WorldEmitter::Polygons(p) => (
            p.iter().map(|v| polygon_area(v)).sum(),
            p.iter()
                .flatten()
                .fold([0.; 3], |mut a, p| {
                    for i in 0..3 {
                        a[i] += p[i];
                    }
                    a
                })
                .map(|v| v / p.iter().map(Vec::len).sum::<usize>().max(1) as f64),
        ),
        WorldEmitter::Sphere { center, radius } => (4. * PI * radius * radius, *center),
    }
}
fn bounds_radius(world: &WorldEmitter, center: [f64; 3]) -> f64 {
    match world {
        WorldEmitter::Sphere { radius, .. } => *radius,
        WorldEmitter::Polygons(p) => p
            .iter()
            .flatten()
            .map(|v| length(sub(*v, center)))
            .fold(0., f64::max),
    }
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn length(a: [f64; 3]) -> f64 {
    (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt()
}
#[cfg(test)]
fn scale(x: f64, y: f64, z: f64) -> [f64; 16] {
    [x, 0., 0., 0., 0., y, 0., 0., 0., 0., z, 0., 0., 0., 0., 1.]
}
#[cfg(test)]
mod stage2_tests {
    use super::*;
    fn draw(mesh_id: u64, power: f64) -> DrawRecord {
        DrawRecord {
            mesh_id,
            shape_index: mesh_id as usize,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: mesh_id as u32,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![identity()],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: Some(AreaLightRecord {
                implementation: "diffuse".into(),
                source: None,
                parameters: vec![PreviewParameter {
                    name: "L".into(),
                    value_type: "rgb".into(),
                    floats: vec![power; 3],
                    texts: vec![],
                    truncated: false,
                }],
            }),
            emitter_shape: None,
            alpha: None,
        }
    }
    fn sphere_geometry() -> EmitterGeometry {
        EmitterGeometry {
            triangles: vec![[[0., 0., -1.], [1., 0., 0.], [0., 1., 0.]]],
            shape: Some(EmitterShape::Sphere { radius: 1. }),
            origin: [0.; 3],
        }
    }
    #[test]
    fn world_emitter_keeps_triangle_vertices_and_winding() {
        let g = EmitterGeometry {
            triangles: vec![[[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.]]],
            shape: None,
            origin: [0.; 3],
        };
        let m = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 2., 0., 0., 0., 1.,
        ];
        let WorldEmitter::Polygons(p) = world_emitter(&g, m, false) else {
            panic!()
        };
        assert_eq!(p[0][0][2], 2.);
        assert!(cross(sub(p[0][1], p[0][0]), sub(p[0][2], p[0][0]))[2] > 0.);
    }
    #[test]
    fn disk_geometry_is_a_sixteen_vertex_area_preserving_polygon() {
        let g = EmitterGeometry {
            triangles: vec![],
            shape: Some(EmitterShape::Disk {
                radius: 1.,
                height: 0.,
            }),
            origin: [0.; 3],
        };
        let WorldEmitter::Polygons(p) = world_emitter(&g, identity(), false) else {
            panic!()
        };
        assert_eq!(p[0].len(), 16);
        assert!((polygon_area(&p[0]) - PI).abs() < 1e-9);
    }
    #[test]
    fn sphere_geometry_remains_analytic_under_uniform_scale() {
        let g = sphere_geometry();
        assert!(
            matches!(world_emitter(&g,scale(2.,2.,2.),false),WorldEmitter::Sphere{radius,..} if (radius-2.).abs()<1e-9)
        );
    }
    #[test]
    fn nonuniformly_scaled_spheres_fall_back_to_polygons() {
        let g = sphere_geometry();
        assert!(matches!(
            world_emitter(&g, scale(1., 2., 1.), false),
            WorldEmitter::Polygons(_)
        ));
    }
    #[test]
    fn emitter_triangle_budget_uses_power_matched_spheres() {
        let t = vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]; 40];
        let g = EmitterGeometry {
            triangles: t,
            shape: None,
            origin: [0.; 3],
        };
        let ds = (0..3).map(|i| draw(i, 1.)).collect::<Vec<_>>();
        let gm = (0..3).map(|i| (i, g.clone())).collect();
        let f = frame_emitters(&ds, &gm, 0);
        assert_eq!(f.lights[0].kind[0], LIGHT_POLYGON);
        assert_eq!(f.lights[1].kind[0], LIGHT_SPHERE);
        assert_eq!(f.lights[2].kind[0], LIGHT_SPHERE);
        // The stand-in sphere radiates the mesh's power: π·L'·4πR² = π·L·A.
        let area = 40. * 0.5;
        for light in &f.lights[1..] {
            let r = light.direction[3] as f64;
            let radiated = PI * light.color[0] as f64 * 4. * PI * r * r;
            assert!((radiated / (PI * area) - 1.).abs() < 1e-5, "{radiated}");
        }
    }
    /// Analytic shapes live in shape space; the mesh-local frame is offset by
    /// the mesh origin (a sphere's first vertex is its lower pole).
    #[test]
    fn analytic_shapes_account_for_the_mesh_origin() {
        let mut g = sphere_geometry();
        g.origin = [0., 0., -1.];
        let m = [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 1., 0., 0., 0., 1.,
        ]; // world = local + (0,0,1)·… as the draw composes it
        let WorldEmitter::Sphere { center, .. } = world_emitter(&g, m, false) else {
            panic!()
        };
        assert_eq!(center, [0., 0., 2.]);
    }
    #[test]
    fn non_similar_disks_use_their_mesh_triangles() {
        let g = EmitterGeometry {
            triangles: vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]],
            shape: Some(EmitterShape::Disk {
                radius: 1.,
                height: 0.,
            }),
            origin: [0.; 3],
        };
        let WorldEmitter::Polygons(p) = world_emitter(&g, scale(1., 2., 1.), false) else {
            panic!()
        };
        assert_eq!(p.len(), 1);
        assert_eq!(p[0][2], [0., 2., 0.]);
    }
    #[test]
    fn polygon_entries_index_disjoint_vertex_ranges() {
        let t = vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]; 2];
        let d = vec![draw(0, 1.)];
        let g = BTreeMap::from([(
            0,
            EmitterGeometry {
                triangles: t,
                shape: None,
                origin: [0.; 3],
            },
        )]);
        let f = frame_emitters(&d, &g, 0);
        assert_eq!(f.lights[0].kind[2], 0);
        assert_eq!(f.lights[0].kind[3], 6);
        assert_eq!(f.vertices.len(), 6);
    }
    /// A disk's light sits at its center, the mean of its 16 vertices, not
    /// that sum over the 42 vertices of its fan (review #264).
    #[test]
    fn translated_disk_lights_sit_at_the_disk_center() {
        let g = EmitterGeometry {
            triangles: vec![],
            shape: Some(EmitterShape::Disk {
                radius: 1.,
                height: 0.,
            }),
            origin: [0.; 3],
        };
        let mut d = draw(0, 1.);
        d.world_from_object = vec![[
            1., 0., 0., 5., 0., 1., 0., 0., 0., 0., 1., 2., 0., 0., 0., 1.,
        ]];
        let f = frame_emitters(&[d], &BTreeMap::from([(0, g)]), 0);
        let p = f.positions[0];
        assert!(
            (p[0] - 5.).abs() < 1e-9 && p[1].abs() < 1e-9 && (p[2] - 2.).abs() < 1e-9,
            "{p:?}"
        );
    }
    /// Equal-length but non-orthogonal columns shear a sphere into an
    /// ellipsoid: not a similarity (review #264).
    #[test]
    fn sheared_spheres_fall_back_to_triangles() {
        let sheared = [
            1., 0.6, 0., 0., 0., 0.8, 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ];
        assert!(matches!(
            world_emitter(&sphere_geometry(), sheared, false),
            WorldEmitter::Polygons(_)
        ));
    }
    /// Emitter vertices stay f64 until the eye-relative upload: a 2-unit
    /// quad at x = 1e8 keeps distinct x coordinates (review #264).
    #[test]
    fn distant_emitters_keep_their_vertices_in_f64() {
        let g = EmitterGeometry {
            triangles: vec![[[-1., -1., 0.], [1., -1., 0.], [1., 1., 0.]]],
            shape: None,
            origin: [0.; 3],
        };
        let mut d = draw(0, 1.);
        d.world_from_object = vec![[
            1., 0., 0., 1e8, 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]];
        let f = frame_emitters(&[d], &BTreeMap::from([(0, g)]), 0);
        assert_eq!(f.vertices[1][0] - f.vertices[0][0], 2.);
    }
    fn param(name: &str, kind: &str, floats: &[f64], texts: &[&str]) -> PreviewParameter {
        PreviewParameter {
            name: name.into(),
            value_type: kind.into(),
            floats: floats.to_vec(),
            texts: texts.iter().map(|t| (*t).into()).collect(),
            truncated: false,
        }
    }
    fn light_draw(parameters: Vec<PreviewParameter>) -> DrawRecord {
        DrawRecord {
            mesh_id: 0,
            shape_index: 0,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: 0,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![identity()],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: Some(AreaLightRecord {
                implementation: "diffuse".into(),
                source: None,
                parameters,
            }),
            emitter_shape: None,
            alpha: None,
        }
    }
    fn triangle_geometry() -> EmitterGeometry {
        EmitterGeometry {
            triangles: vec![[[0., 0., 0.], [1., 0., 0.], [0., 1., 0.]]],
            shape: None,
            origin: [0.; 3],
        }
    }
    fn frame_of(parameters: Vec<PreviewParameter>) -> FrameEmitters {
        frame_emitters(
            &[light_draw(parameters)],
            &BTreeMap::from([(0, triangle_geometry())]),
            0,
        )
    }
    fn assert_light_color(frame: &FrameEmitters, want: [f64; 3]) {
        assert_eq!(frame.lights.len(), 1);
        let got = [
            frame.lights[0].color[0] as f64,
            frame.lights[0].color[1] as f64,
            frame.lights[0].color[2] as f64,
        ];
        for channel in 0..3 {
            assert!(
                (got[channel] - want[channel]).abs() < 1e-4,
                "{got:?} != {want:?}"
            );
        }
    }
    /// A `blackbody L` parameter is a temperature, not radiance floats: the
    /// light keeps `blackbody_rgb`'s luminance-1 colour instead of a
    /// radiance of thousands.
    #[test]
    fn area_blackbody_light_carries_the_normalized_blackbody_color() {
        let frame = frame_of(vec![param("L", "blackbody", &[5500.], &[])]);
        assert_light_color(&frame, crate::preview::lit::blackbody_rgb(5500.));
    }
    /// `scale` multiplies the resolved colour in `frame_emitters`, so the
    /// resolution itself must not bake the kelvin in as radiance.
    #[test]
    fn area_blackbody_light_multiplies_the_blackbody_color_by_scale() {
        let frame = frame_of(vec![
            param("L", "blackbody", &[5500.], &[]),
            param("scale", "float", &[0.1], &[]),
        ]);
        assert_light_color(
            &frame,
            crate::preview::lit::blackbody_rgb(5500.).map(|v| v * 0.1),
        );
    }
    #[test]
    fn area_rgb_emission_floats_stay_as_they_were() {
        let frame = frame_of(vec![param("L", "rgb", &[0.25, 0.5, 0.75], &[])]);
        assert_light_color(&frame, [0.25, 0.5, 0.75]);
    }
    /// A spectrum given as λ v pairs reads at the channel wavelengths, as
    /// the material spectra do — not as three RGB floats.
    #[test]
    fn area_inline_spectrum_reads_the_channel_wavelengths() {
        let frame = frame_of(vec![param("L", "spectrum", &[400., 0.25, 500., 0.5], &[])]);
        // 630 nm: held at 0.5; 532 nm: held at 0.5; 465 nm: 0.25 + 0.65·0.25.
        assert_light_color(&frame, [0.5, 0.5, 0.4125]);
    }
    /// Named spectra resolve through the scene-parse table (the wiring
    /// r4-matches; glass values only prove the lookup here).
    #[test]
    fn area_named_spectrum_resolves_through_the_scene_parse_table() {
        let frame = frame_of(vec![param("L", "spectrum", &[], &["glass-BK7"])]);
        assert_light_color(&frame, [1.5151856, 1.5194908, 1.5240455]);
    }

    /// The frame records each emitter's surface radiance — the value the
    /// KIND_EMISSIVE palette reads — through the same resolution that
    /// shades the light it emits.
    #[test]
    fn frame_records_the_surface_radiance_per_draw() {
        let frame = frame_of(vec![
            param("L", "rgb", &[1.; 3], &[]),
            param("scale", "float", &[0.1], &[]),
        ]);
        assert_eq!(frame.lights.len(), 1);
        let surface = frame
            .surface_of_draw
            .get(&0)
            .copied()
            .expect("draw 0 radiance recorded");
        assert_eq!(surface, [0.1; 3]);
        assert!((frame.lights[0].color[0] as f64 - 0.1).abs() < 1e-6);
    }

    /// With `power`, the surface radiance is power-matched over the
    /// emitter's own area — r4's DiffuseAreaLight renders its one
    /// recalculation as both the emission and the visible face: the
    /// 0.5 m² triangle turns L 2 × scale 3 × power π into 2·3·π/(π·0.5).
    #[test]
    fn powered_emitter_surfaces_match_the_radiance_they_emit() {
        let frame = frame_of(vec![
            param("L", "rgb", &[2.; 3], &[]),
            param("scale", "float", &[3.], &[]),
            param("power", "float", &[PI], &[]),
        ]);
        assert_eq!(frame.lights.len(), 1);
        let surface = frame
            .surface_of_draw
            .get(&0)
            .copied()
            .expect("draw 0 radiance recorded");
        assert_eq!(surface, [12.; 3]);
        assert_eq!(
            [
                frame.lights[0].color[0] as f64,
                frame.lights[0].color[1] as f64,
                frame.lights[0].color[2] as f64
            ],
            [12.; 3]
        );
    }

    /// Without geometry there is no area to match a `power` against (and
    /// no contribution either), so the surface keeps the scale-only
    /// radiance.
    #[test]
    fn without_geometry_power_stays_out_of_the_surface_radiance() {
        let area = AreaLightRecord {
            implementation: "diffuse".into(),
            source: None,
            parameters: vec![
                param("L", "rgb", &[2.; 3], &[]),
                param("scale", "float", &[3.], &[]),
                param("power", "float", &[PI], &[]),
            ],
        };
        let mut notes = Vec::new();
        let radiance = surface_radiance(&area, None, &mut notes);
        assert_eq!(radiance, [6.; 3]);
        assert!(notes.is_empty(), "{notes:?}");
    }

    /// Spectrum approximations keep flowing to the frame's notes through
    /// the one shared resolution.
    #[test]
    fn surface_radiance_carries_spectrum_approximations_to_the_notes() {
        let area = AreaLightRecord {
            implementation: "diffuse".into(),
            source: None,
            parameters: vec![param("L", "spectrum", &[], &["spd/luminaire.spd"])],
        };
        let mut notes = Vec::new();
        let radiance = surface_radiance(&area, None, &mut notes);
        assert_eq!(radiance, [1.; 3]);
        assert_eq!(
            notes,
            vec!["light spectrum spd/luminaire.spd: approximated as white".to_string()]
        );
    }
}
