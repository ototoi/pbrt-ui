//! Shadow maps for the strongest Lit lights.
//!
//! Every shadow map lives in one `Depth32Float` 2D array. A positional light
//! (point, spot, polygon, sphere) takes six layers, one per cube face in the
//! order +x, -x, +y, -y, +z, -z; a distant light takes one orthographic
//! layer. Depth is linear: distance / far for positional lights, the
//! position along the light for distant ones.
//!
//! Shadow maps do not depend on the camera, so they are built in a fixed
//! frame: world coordinates relative to `ShadowSetup::origin`, the scene
//! bounds' center. The Lit shader moves its eye-relative points into that
//! frame with `View::shadow_origin` (eye - origin).
use super::{
    GpuLight, LIGHT_DISTANT, LIGHT_ENVIRONMENT, LIGHT_GONIOMETRIC, LIGHT_INFINITE, LIGHT_POINT,
    LIGHT_POLYGON, LIGHT_PROJECTION, LIGHT_SPHERE, LIGHT_SPOT,
};
use bytemuck::Zeroable;
use protocol::geometry::Bounds;

/// `ShadowGpu::params.z`: what kind of shadow a light has. Entries start
/// zeroed, which is `SHADOW_NONE`.
#[allow(dead_code)]
pub(crate) const SHADOW_NONE: u32 = 0;
pub(crate) const SHADOW_POSITIONAL: u32 = 1;
pub(crate) const SHADOW_DISTANT: u32 = 2;

/// Per light, indexed like the light list.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowGpu {
    /// Light-space view-projection per layer (only [0] for distant lights),
    /// applied to points in the shadow frame.
    pub matrices: [[[f32; 4]; 4]; 6],
    /// xyz: light position in the shadow frame; w: far (positional) or the
    /// depth range (distant).
    pub position_far: [f32; 4],
    /// x: first layer; y: layer count; z: SHADOW_* kind; w unused.
    pub params: [u32; 4],
    /// x: light radius for PCSS (0 for point, spot and distant); yzw unused.
    pub size: [f32; 4],
}

/// One shadow-map render: a layer and what to draw into it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub(crate) struct ShadowPassGpu {
    pub matrix: [[f32; 4]; 4],
    /// xyz: light position in the shadow frame; w: far.
    pub position_far: [f32; 4],
    /// x: SHADOW_* kind.
    pub kind: [u32; 4],
}

pub(crate) struct ShadowPass {
    pub layer: u32,
    pub uniform: ShadowPassGpu,
    /// The light's own emitter draw, which must not shadow its light.
    pub exclude_draw: Option<usize>,
}

pub(crate) struct ShadowSetup {
    pub origin: [f64; 3],
    pub entries: Vec<ShadowGpu>,
    pub passes: Vec<ShadowPass>,
}

/// Choose the shadowed lights and lay out their layers and matrices.
/// `positions` are the lights' world positions in f64; `draw_of_light[i]` is light i's emitter
/// draw, when it has one.
pub(crate) fn shadow_setup(
    lights: &[GpuLight],
    positions: &[[f64; 3]],
    bounds: Option<&Bounds>,
    draw_of_light: &[Option<usize>],
) -> ShadowSetup {
    let origin = bounds.map_or([0.; 3], Bounds::center);
    let scene_radius = bounds.map_or(0., Bounds::half_diagonal);
    let mut setup = ShadowSetup {
        origin,
        entries: vec![ShadowGpu::zeroed(); lights.len()],
        passes: Vec::new(),
    };
    let mut next_layer = 0u32;
    for index in choose_shadowed(lights, scene_radius) {
        let light = &lights[index];
        let exclude_draw = draw_of_light.get(index).copied().flatten();
        if light.kind[0] == LIGHT_DISTANT {
            // Light travels against `direction` (surface -> light).
            let forward =
                normalize(light.direction.map(f64::from)[..3].try_into().unwrap()).map(|v| -v);
            let radius = (scene_radius * 1.01).max(1e-3);
            let matrix = orthographic(forward, radius);
            let entry = &mut setup.entries[index];
            entry.matrices[0] = matrix;
            entry.position_far = [0., 0., 0., (2. * radius) as f32];
            entry.params = [next_layer, 1, SHADOW_DISTANT, 0];
            setup.passes.push(ShadowPass {
                layer: next_layer,
                uniform: ShadowPassGpu {
                    matrix,
                    position_far: entry.position_far,
                    kind: [SHADOW_DISTANT, 0, 0, 0],
                },
                exclude_draw,
            });
            next_layer += 1;
        } else {
            let position = sub(positions[index], origin);
            let far = bounds.map_or(1., |b| farthest_corner(b, origin, position)) * 1.01;
            let far = far.max(1e-3);
            let entry = &mut setup.entries[index];
            entry.position_far = [
                position[0] as f32,
                position[1] as f32,
                position[2] as f32,
                far as f32,
            ];
            entry.params = [next_layer, 6, SHADOW_POSITIONAL, 0];
            entry.size = [light_radius(light) as f32, 0., 0., 0.];
            for face in 0..6 {
                let matrix = cube_face(position, face, far);
                entry.matrices[face] = matrix;
                setup.passes.push(ShadowPass {
                    layer: next_layer + face as u32,
                    uniform: ShadowPassGpu {
                        matrix,
                        position_far: entry.position_far,
                        kind: [SHADOW_POSITIONAL, 0, 0, 0],
                    },
                    exclude_draw,
                });
            }
            next_layer += 6;
        }
    }
    setup
}

/// Polygon and sphere lights carry their (equivalent) radius in
/// `direction.w`; point and spot lights are points.
fn light_radius(light: &GpuLight) -> f64 {
    match light.kind[0] {
        LIGHT_POLYGON | LIGHT_SPHERE => light.direction[3] as f64,
        _ => 0.,
    }
}

/// Farthest bounds corner from `position`, both relative to `origin`.
fn farthest_corner(bounds: &Bounds, origin: [f64; 3], position: [f64; 3]) -> f64 {
    let mut farthest = 0f64;
    for corner in 0..8 {
        let p = std::array::from_fn(|axis| {
            let value = if corner & (1 << axis) == 0 {
                bounds.min[axis]
            } else {
                bounds.max[axis]
            };
            value - origin[axis]
        });
        farthest = farthest.max(length(sub(p, position)));
    }
    farthest
}

/// Cube face axes: forward and up per face, +x, -x, +y, -y, +z, -z.
const FACES: [([f64; 3], [f64; 3]); 6] = [
    ([1., 0., 0.], [0., 1., 0.]),
    ([-1., 0., 0.], [0., 1., 0.]),
    ([0., 1., 0.], [0., 0., -1.]),
    ([0., -1., 0.], [0., 0., 1.]),
    ([0., 0., 1.], [0., 1., 0.]),
    ([0., 0., -1.], [0., 1., 0.]),
];

/// 90-degree perspective for one cube face. Clip depth maps near..far to
/// 0..1 so geometry is not clipped; the shadow pass writes linear depth.
fn cube_face(position: [f64; 3], face: usize, far: f64) -> [[f32; 4]; 4] {
    let (forward, up) = FACES[face];
    let right = cross(up, forward);
    let near = far * 1e-4;
    let a = far / (far - near);
    let b = -near * a;
    let rows = [
        row(right, position, 1., 0.),
        row(up, position, 1., 0.),
        row(forward, position, a, b),
        row(forward, position, 1., 0.),
    ];
    columns(rows)
}

/// Orthographic view along `forward` covering the sphere of `radius` about
/// the shadow origin; depth 0..1 front to back.
fn orthographic(forward: [f64; 3], radius: f64) -> [[f32; 4]; 4] {
    let helper = if forward[2].abs() > 0.9 {
        [1., 0., 0.]
    } else {
        [0., 0., 1.]
    };
    let right = normalize(cross(helper, forward));
    let up = cross(forward, right);
    let rows = [
        row(right, [0.; 3], 1. / radius, 0.),
        row(up, [0.; 3], 1. / radius, 0.),
        row(forward, [0.; 3], 0.5 / radius, 0.5),
        [0., 0., 0., 1.],
    ];
    columns(rows)
}

/// `scale * dot(axis, p - position) + offset` as a matrix row.
fn row(axis: [f64; 3], position: [f64; 3], scale: f64, offset: f64) -> [f64; 4] {
    [
        scale * axis[0],
        scale * axis[1],
        scale * axis[2],
        offset - scale * dot(axis, position),
    ]
}

/// WGSL matrices are column-major.
fn columns(rows: [[f64; 4]; 4]) -> [[f32; 4]; 4] {
    std::array::from_fn(|column| std::array::from_fn(|r| rows[r][column] as f32))
}

fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn length(v: [f64; 3]) -> f64 {
    dot(v, v).sqrt()
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn normalize(v: [f64; 3]) -> [f64; 3] {
    let l = length(v).max(1e-20);
    [v[0] / l, v[1] / l, v[2] / l]
}

pub(crate) const MAX_SHADOWED: usize = 4;
pub(crate) const SHADOW_SIZE: u32 = 1024;

pub(crate) fn choose_shadowed(lights: &[GpuLight], scene_radius: f64) -> Vec<usize> {
    let mut candidates = lights
        .iter()
        .enumerate()
        .filter_map(|(index, light)| {
            let luminance = 0.2126 * light.color[0] as f64
                + 0.7152 * light.color[1] as f64
                + 0.0722 * light.color[2] as f64;
            let flux = match light.kind[0] {
                LIGHT_POINT => 4.0 * std::f64::consts::PI * luminance,
                LIGHT_SPOT => {
                    let cos_start = light.direction[3] as f64;
                    let cos_end = light.color[3] as f64;
                    2.0 * std::f64::consts::PI * (1.0 - 0.5 * (cos_start + cos_end)) * luminance
                }
                LIGHT_DISTANT if scene_radius > 0.0 => {
                    std::f64::consts::PI * scene_radius * scene_radius * luminance
                }
                LIGHT_POLYGON => {
                    let area = std::f64::consts::PI * (light.direction[3] as f64).powi(2);
                    std::f64::consts::PI
                        * luminance
                        * area
                        * if light.kind[1] & 1 != 0 { 2.0 } else { 1.0 }
                }
                LIGHT_SPHERE => {
                    4.0 * std::f64::consts::PI.powi(2)
                        * (light.direction[3] as f64).powi(2)
                        * luminance
                }
                // r4's phi: goniometric 4pi * I * mean(Y) — mean Y rides
                // direction.w; projection A * mean(lum * cos^3) — likewise,
                // color holding scale * I.
                LIGHT_GONIOMETRIC => {
                    4.0 * std::f64::consts::PI * luminance * light.direction[3] as f64
                }
                LIGHT_PROJECTION => luminance * light.direction[3] as f64,
                LIGHT_DISTANT | LIGHT_INFINITE | LIGHT_ENVIRONMENT => return None,
                _ => return None,
            };
            Some((index, flux))
        })
        .collect::<Vec<_>>();
    candidates.sort_by(|a, b| b.1.total_cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    candidates
        .into_iter()
        .take(MAX_SHADOWED)
        .map(|(index, _)| index)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(intensity: f32) -> GpuLight {
        GpuLight {
            color: [intensity, intensity, intensity, 0.0],
            kind: [LIGHT_POINT, 0, 0, 0],
            ..Default::default()
        }
    }

    #[test]
    fn the_four_strongest_lights_get_shadows_with_index_ties() {
        let lights = (1..=6).map(|value| point(value as f32)).collect::<Vec<_>>();
        assert_eq!(choose_shadowed(&lights, 1.0), [5, 4, 3, 2]);
        let tied = vec![point(1.0); 6];
        assert_eq!(choose_shadowed(&tied, 1.0), [0, 1, 2, 3]);
    }

    /// A goniometric light's flux is 4pi * lum(color) * mean Y (r4's phi for
    /// an rgb intensity); a projection light's is lum(color) * A *
    /// mean(lum * cos^3), both riding direction.w — the shadow chooser
    /// must weigh them against point lights, not skip them.
    #[test]
    fn goniometric_and_projection_fluxes_merge_into_the_ranking() {
        let mut goniometric = point(1.0);
        goniometric.kind[0] = LIGHT_GONIOMETRIC;
        goniometric.direction[3] = 0.5; // flux 2pi, the strongest
        let mut projection = point(1.0);
        projection.kind[0] = LIGHT_PROJECTION;
        projection.direction[3] = 3.0; // flux 3, second
        let weak = point(0.2); // flux 4pi*0.2 = 2.51, last
        let lights = vec![goniometric, projection, weak];
        assert_eq!(choose_shadowed(&lights, 1.0), [0, 1, 2]);
    }

    /// Without the frame means the flux falls back to uniform: goniometric
    /// Y = 1 (flux 4pi exactly like a point light of the same intensity).
    #[test]
    fn mapless_goniometric_ranks_like_a_point_light() {
        let mut goniometric = point(1.0);
        goniometric.kind[0] = LIGHT_GONIOMETRIC;
        goniometric.direction[3] = 1.0;
        assert_eq!(choose_shadowed(&[goniometric, point(0.5)], 1.0), [0, 1]);
    }

    fn apply(m: [[f32; 4]; 4], p: [f64; 3]) -> [f64; 4] {
        std::array::from_fn(|r| (0..3).map(|c| m[c][r] as f64 * p[c]).sum::<f64>() + m[3][r] as f64)
    }

    /// A point straight along each face's axis lands in the middle of that
    /// face, with clip depth inside 0..1.
    #[test]
    fn cube_faces_look_along_their_axes() {
        let light = [1., 2., 3.];
        for (face, (forward, _)) in FACES.iter().enumerate() {
            let p = [
                light[0] + 2. * forward[0],
                light[1] + 2. * forward[1],
                light[2] + 2. * forward[2],
            ];
            let clip = apply(cube_face(light, face, 10.), p);
            assert!(
                (clip[0] / clip[3]).abs() < 1e-6 && (clip[1] / clip[3]).abs() < 1e-6,
                "face {face}"
            );
            let depth = clip[2] / clip[3];
            assert!(depth > 0. && depth < 1., "face {face}: {depth}");
        }
    }

    #[test]
    fn positional_shadows_live_in_the_scene_centered_frame() {
        let bounds = Bounds {
            min: [10., 10., 10.],
            max: [12., 12., 12.],
        };
        let light = point(1.);
        let setup = shadow_setup(&[light], &[[11., 11., 15.]], Some(&bounds), &[None]);
        assert_eq!(setup.origin, [11., 11., 11.]);
        assert_eq!(setup.entries[0].position_far[..3], [0., 0., 4.]);
        assert_eq!(setup.entries[0].params, [0, 6, SHADOW_POSITIONAL, 0]);
        assert_eq!(setup.passes.len(), 6);
        assert_eq!(
            setup.passes.iter().map(|p| p.layer).collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5]
        );
    }

    #[test]
    fn distant_shadows_cover_the_scene_front_to_back() {
        let bounds = Bounds {
            min: [-1.; 3],
            max: [1.; 3],
        };
        let distant = GpuLight {
            color: [1.0; 4],
            direction: [0., 0., 1., 0.], // surface -> light: light shines down -z
            kind: [LIGHT_DISTANT, 0, 0, 0],
            ..Default::default()
        };
        let setup = shadow_setup(&[distant], &[[0.; 3]], Some(&bounds), &[None]);
        let m = setup.entries[0].matrices[0];
        let top = apply(m, [0., 0., 1.]);
        let bottom = apply(m, [0., 0., -1.]);
        assert!(top[2] < bottom[2], "nearer the light is shallower");
        assert!(top[2] > 0. && bottom[2] < 1.);
    }

    #[test]
    fn an_emitter_draw_is_excluded_from_its_own_shadow_passes() {
        let mut light = point(1.);
        light.kind[0] = LIGHT_SPHERE;
        light.direction[3] = 0.5;
        let setup = shadow_setup(
            &[light],
            &[[0.; 3]],
            Some(&Bounds {
                min: [-1.; 3],
                max: [1.; 3],
            }),
            &[Some(7)],
        );
        assert!(setup.passes.iter().all(|p| p.exclude_draw == Some(7)));
        assert_eq!(setup.entries[0].size[0], 0.5);
    }

    #[test]
    fn distant_lights_need_scene_bounds_for_shadows() {
        let distant = GpuLight {
            color: [1.0; 4],
            kind: [LIGHT_DISTANT, 0, 0, 0],
            ..Default::default()
        };
        assert!(choose_shadowed(&[distant], 0.0).is_empty());
        assert_eq!(choose_shadowed(&[distant], 1.0), [0]);
    }
}
