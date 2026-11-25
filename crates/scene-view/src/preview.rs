//! Headless preview renderer for `scene-parse` geometry packages.
use protocol::geometry::{Bounds, GeometryManifest};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewMode {
    Solid,
    Normal,
    Wireframe,
    /// pbrt-like preview: pbrt light units, area lights, shadows and GGX
    /// materials.
    Lit,
}

/// What a Lit capture shows instead of the shaded image.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum LitDebug {
    #[default]
    Off,
    /// Diffuse reflectance, no lighting or film exposure.
    Albedo,
    /// The environment light's screen-space AO, 1 = unoccluded.
    Ao,
}

#[cfg(test)]
mod shader_tests {
    fn validates(source: &str, label: &str) {
        let mut frontend = naga::front::wgsl::Frontend::new();
        let module = frontend
            .parse(source)
            .unwrap_or_else(|e| panic!("{label} parse: {e}"));
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::all(),
        )
        .validate(&module)
        .unwrap_or_else(|e| panic!("{label} validation: {e}"));
    }

    #[test]
    fn preview_lit_and_output_wgsl_validate() {
        validates(
            concat!(
                include_str!("preview/vertex.wgsl"),
                include_str!("preview/preview.wgsl")
            ),
            "preview WGSL",
        );
        // The exact source the Lit pipeline compiles.
        validates(super::lit::pipeline::LIT_WGSL, "Lit WGSL");
        // And the binding-array rewrite on devices that prefer individual
        // textures: the rewrite's text matches are fragile, so the version
        // it produces must also validate.
        validates(
            &super::lit::pipeline::shader_source(super::lit::pipeline::LIT_WGSL, Some(64)),
            "Lit WGSL with binding arrays",
        );
        validates(super::lit::pipeline::SHADOW_WGSL, "shadow pass WGSL");
        validates(include_str!("preview/lit/output.wgsl"), "output WGSL");
        validates(
            include_str!("preview/lit/environment_prefilter.wgsl"),
            "environment prefilter WGSL",
        );
        validates(
            include_str!("preview/lit/environment.wgsl"),
            "environment bake WGSL",
        );
        validates(
            include_str!("preview/lit/environment_background.wgsl"),
            "environment background WGSL",
        );
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewerCamera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub fov_y_degrees: f64,
    pub near: f64,
    pub far: f64,
    /// Image mirrored left-right for a right-handed scene; the
    /// projection, picking, gizmo and drags all follow it.
    pub mirror: bool,
}
impl ViewerCamera {
    pub fn frame(bounds: Option<&Bounds>, aspect: f64) -> Self {
        Self::from_protocol(protocol::view::framing_camera(bounds, aspect))
    }
    /// `frame` around a scene's up axis.
    pub fn frame_up(bounds: Option<&Bounds>, aspect: f64, up: protocol::project::UpAxis) -> Self {
        Self::from_protocol(protocol::view::framing_camera_up(bounds, aspect, up))
    }
    pub fn from_manifest(manifest: &GeometryManifest, aspect: f64) -> Self {
        Self::from_manifest_up(manifest, aspect, protocol::project::UpAxis::PosY)
    }
    /// The manifest camera, else a framing around `up`.
    pub fn from_manifest_up(
        manifest: &GeometryManifest,
        aspect: f64,
        up: protocol::project::UpAxis,
    ) -> Self {
        protocol::view::manifest_camera(manifest)
            .map(Self::from_protocol)
            .unwrap_or_else(|| Self::frame_up(manifest.bounds.as_ref(), aspect, up))
    }
    /// Adapt the shared wire camera; the viewport only renders perspective.
    pub fn from_protocol(camera: protocol::view::Camera) -> Self {
        Self {
            eye: camera.eye,
            target: camera.target,
            up: camera.up,
            fov_y_degrees: camera.fov_y_degrees,
            near: camera.near,
            far: camera.far,
            mirror: camera.mirror,
        }
    }
    /// Wire representation of this viewport camera (always perspective).
    pub fn to_protocol(&self) -> protocol::view::Camera {
        protocol::view::Camera {
            eye: self.eye,
            target: self.target,
            up: self.up,
            projection: protocol::view::Projection::Perspective,
            fov_y_degrees: self.fov_y_degrees,
            near: self.near,
            far: self.far,
            mirror: self.mirror,
        }
    }

    pub fn orbit(&mut self, yaw_radians: f64, pitch_radians: f64) -> Result<(), String> {
        if !yaw_radians.is_finite() || !pitch_radians.is_finite() {
            return Err("orbit delta must be finite".into());
        }
        let offset = sub(self.eye, self.target);
        let distance = dot(offset, offset).sqrt();
        if !distance.is_finite() || distance <= 1e-9 {
            return Err("camera eye and target coincide".into());
        }
        let world_up = norm(self.up).ok_or("camera up is invalid")?;
        let mut direction = norm(offset).unwrap();
        // A mirrored image turns the other way on screen; keep drags natural.
        let yaw_radians = if self.mirror {
            -yaw_radians
        } else {
            yaw_radians
        };
        let (sy, cy) = yaw_radians.sin_cos();
        direction = rotate_axis(direction, world_up, sy, cy);
        let right = norm(cross(world_up, direction)).ok_or("camera orbit axis is invalid")?;
        // Elevation against the camera's own up (pbrt scenes are often
        // Z-up). Rotating about `right` by `a` lowers it by `a`, so a
        // positive pitch lowers the eye, stopping short of either pole.
        let max_pitch = std::f64::consts::FRAC_PI_2 - 0.01;
        let current = dot(direction, world_up).clamp(-1., 1.).asin();
        let wanted = (current - pitch_radians).clamp(-max_pitch, max_pitch);
        let (sp, cp) = (current - wanted).sin_cos();
        direction = rotate_axis(direction, right, sp, cp);
        self.eye = [
            self.target[0] + direction[0] * distance,
            self.target[1] + direction[1] * distance,
            self.target[2] + direction[2] * distance,
        ];
        self.up = world_up;
        Ok(())
    }
    pub fn pan(&mut self, horizontal: f64, vertical: f64) -> Result<(), String> {
        if !horizontal.is_finite() || !vertical.is_finite() {
            return Err("pan delta must be finite".into());
        }
        let f = norm(sub(self.target, self.eye)).ok_or("camera eye and target coincide")?;
        // Match the projection basis (pbrt's left-handed LookAt): up ×
        // forward is camera-right.
        let right = norm(cross(self.up, f)).ok_or("camera up is invalid")?;
        let up = cross(f, right);
        let right = if self.mirror {
            right.map(|v| -v)
        } else {
            right
        };
        let distance = dot(sub(self.target, self.eye), sub(self.target, self.eye)).sqrt();
        let scale = 2. * distance * (self.fov_y_degrees.to_radians() / 2.).tan();
        let shift = [
            (-horizontal * scale) * right[0] + vertical * scale * up[0],
            (-horizontal * scale) * right[1] + vertical * scale * up[1],
            (-horizontal * scale) * right[2] + vertical * scale * up[2],
        ];
        for a in 0..3 {
            self.eye[a] += shift[a];
            self.target[a] += shift[a];
        }
        Ok(())
    }
    pub fn dolly(&mut self, log_distance_delta: f64) -> Result<(), String> {
        if !log_distance_delta.is_finite() {
            return Err("dolly delta must be finite".into());
        }
        let offset = sub(self.eye, self.target);
        let distance = dot(offset, offset).sqrt();
        if !distance.is_finite() || distance <= 1e-9 {
            return Err("camera eye and target coincide".into());
        }
        let next = (distance * (-log_distance_delta).exp()).clamp(1e-6, 1e15);
        for a in 0..3 {
            self.eye[a] = self.target[a] + offset[a] * next / distance;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct PreviewFrame {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
    pub pick_ids: Vec<u32>,
    pub backend: String,
}
impl PreviewFrame {
    /// Read the top-left-origin pick buffer using normalized image coordinates.
    pub fn pick_at(&self, normalized_xy: [f64; 2]) -> Result<u32, String> {
        let [u, v] = normalized_xy;
        if !u.is_finite()
            || !v.is_finite()
            || !(0.0..=1.0).contains(&u)
            || !(0.0..=1.0).contains(&v)
        {
            return Err("pick coordinates must be finite and in [0,1]".into());
        }
        let x = ((u * self.width as f64).floor() as u32).min(self.width - 1);
        let y = ((v * self.height as f64).floor() as u32).min(self.height - 1);
        Ok(self.pick_ids[(y * self.width + x) as usize])
    }
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn norm(a: [f64; 3]) -> Option<[f64; 3]> {
    let l = dot(a, a).sqrt();
    (l.is_finite() && l > 1e-12).then(|| [a[0] / l, a[1] / l, a[2] / l])
}
fn rotate_axis(v: [f64; 3], axis: [f64; 3], sin: f64, cos: f64) -> [f64; 3] {
    let c = cross(axis, v);
    let d = dot(axis, v) * (1. - cos);
    [
        v[0] * cos + c[0] * sin + axis[0] * d,
        v[1] * cos + c[1] * sin + axis[1] * d,
        v[2] * cos + c[2] * sin + axis[2] * d,
    ]
}
pub mod gizmo;
pub mod light_gizmo;
mod lit;
pub use lit::LitNote;
pub use lit::blackbody_rgb;
mod mesh;
mod renderer;
mod targets;
pub use renderer::{
    MAX_PREVIEW_PIXELS, PreviewRenderer, PreviewRendererStats, RenderedLitTextures,
    SceneUpdateTimings,
};

/// One-shot capture. Interactive callers retain a PreviewRenderer and upload
/// the scene once, so camera, mode and placement changes reuse mesh buffers.
pub fn render_preview(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
    camera: ViewerCamera,
    width: u32,
    height: u32,
    mode: PreviewMode,
) -> Result<PreviewFrame, String> {
    let time = manifest
        .supported_times
        .get(manifest.supported_times.len() / 2)
        .copied()
        .unwrap_or(0.0);
    render_preview_at_time(manifest, blobs, camera, width, height, mode, time)
}

pub fn render_preview_at_time(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
    camera: ViewerCamera,
    width: u32,
    height: u32,
    mode: PreviewMode,
    time: f64,
) -> Result<PreviewFrame, String> {
    render_preview_at_time_with_options(
        manifest,
        blobs,
        camera,
        width,
        height,
        mode,
        time,
        LitDebug::Off,
    )
}

pub fn render_preview_at_time_with_options(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
    camera: ViewerCamera,
    width: u32,
    height: u32,
    mode: PreviewMode,
    time: f64,
    debug: LitDebug,
) -> Result<PreviewFrame, String> {
    let mut renderer = PreviewRenderer::new()?;
    renderer.upload_scene(manifest, blobs)?;
    renderer.render_with_options(camera, width, height, mode, time, debug)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A mirrored (right-handed) view shows the world flipped, so
    /// pan and orbit turn the other way in world space and the scene still
    /// follows the mouse on screen.
    #[test]
    fn mirrored_cameras_reverse_horizontal_drags_in_world_space() {
        let plain = ViewerCamera {
            eye: [0., 0., 10.],
            target: [0.; 3],
            up: [0., 1., 0.],
            fov_y_degrees: 45.,
            near: 0.1,
            far: 100.,
            mirror: false,
        };
        let mirrored = ViewerCamera {
            mirror: true,
            ..plain
        };
        let (mut a, mut b) = (plain, mirrored);
        a.pan(0.1, 0.).unwrap();
        b.pan(0.1, 0.).unwrap();
        assert!(a.eye[0] * b.eye[0] < 0., "{:?} {:?}", a.eye, b.eye);
        let (mut a, mut b) = (plain, mirrored);
        a.orbit(0.3, 0.).unwrap();
        b.orbit(0.3, 0.).unwrap();
        assert!(a.eye[0] * b.eye[0] < 0., "{:?} {:?}", a.eye, b.eye);
        // Vertical drags are not mirrored.
        let (mut a, mut b) = (plain, mirrored);
        a.pan(0., 0.1).unwrap();
        b.pan(0., 0.1).unwrap();
        assert_eq!(a.eye, b.eye);
        // The flag rides the wire camera both ways.
        assert_eq!(
            ViewerCamera::from_protocol(mirrored.to_protocol()),
            mirrored
        );
    }

    #[test]
    fn camera_orbit_pan_and_dolly_preserve_the_pivot_contract() {
        let mut camera = ViewerCamera {
            eye: [0., 0., 10.],
            target: [0.; 3],
            up: [0., 1., 0.],
            fov_y_degrees: 45.,
            near: 0.1,
            far: 100.,
            mirror: false,
        };
        camera.orbit(0.4, 0.2).unwrap();
        assert!(
            (dot(
                sub(camera.eye, camera.target),
                sub(camera.eye, camera.target)
            )
            .sqrt()
                - 10.)
                .abs()
                < 1e-9
        );
        assert_ne!(camera.eye, [0., 0., 10.]);
        let eye = camera.eye;
        let target = camera.target;
        camera.pan(0.1, -0.05).unwrap();
        for axis in 0..3 {
            assert!(
                (camera.eye[axis] - eye[axis] - (camera.target[axis] - target[axis])).abs() < 1e-9
            )
        }
        let before = dot(
            sub(camera.eye, camera.target),
            sub(camera.eye, camera.target),
        )
        .sqrt();
        camera.dolly(0.2).unwrap();
        let after = dot(
            sub(camera.eye, camera.target),
            sub(camera.eye, camera.target),
        )
        .sqrt();
        assert!(after < before);
    }

    /// Pitch is measured against the camera's own up axis, so a Z-up scene
    /// (common in pbrt) tilts by exactly the requested angle and stops just
    /// short of either pole, like a Y-up one. A positive pitch lowers the
    /// eye.
    #[test]
    fn orbit_pitch_follows_the_camera_up_axis() {
        for (up, eye) in [([0., 1., 0.], [0., 2., 10.]), ([0., 0., 1.], [0., 10., 2.])] {
            let mut camera = ViewerCamera {
                eye,
                target: [0.; 3],
                up,
                fov_y_degrees: 45.,
                near: 0.1,
                far: 100.,
                mirror: false,
            };
            let elevation = |camera: &ViewerCamera| {
                let offset = norm(sub(camera.eye, camera.target)).unwrap();
                dot(offset, up).asin()
            };
            let before = elevation(&camera);
            camera.orbit(0., -0.3).unwrap();
            assert!(
                (elevation(&camera) - before - 0.3).abs() < 1e-9,
                "up {up:?}: {} -> {}",
                before,
                elevation(&camera)
            );
            let pole = std::f64::consts::FRAC_PI_2 - 0.01;
            camera.orbit(0., -3.).unwrap();
            assert!((elevation(&camera) - pole).abs() < 1e-9, "up {up:?}");
            camera.orbit(0., 6.).unwrap();
            assert!((elevation(&camera) + pole).abs() < 1e-9, "up {up:?}");
        }
    }

    #[test]
    fn normalized_pick_maps_top_left_coordinates_and_rejects_outside_values() {
        let frame = PreviewFrame {
            width: 2,
            height: 2,
            rgba: vec![0; 16],
            pick_ids: vec![1, 2, 3, 4],
            backend: "test".into(),
        };
        assert_eq!(frame.pick_at([0., 0.]).unwrap(), 1);
        assert_eq!(frame.pick_at([1., 1.]).unwrap(), 4);
        assert!(frame.pick_at([-0.1, 0.5]).is_err());
    }

    #[test]
    fn viewport_cameras_agree_with_the_shared_wire_math() {
        // The viewport camera delegates to protocol::view so desktop and the
        // headless capture adapter frame the same geometry identically.
        fn close(a: &ViewerCamera, b: &ViewerCamera) {
            for axis in 0..3 {
                assert!((a.eye[axis] - b.eye[axis]).abs() < 1e-12);
                assert!((a.target[axis] - b.target[axis]).abs() < 1e-12);
                assert!((a.up[axis] - b.up[axis]).abs() < 1e-12);
            }
            assert!((a.fov_y_degrees - b.fov_y_degrees).abs() < 1e-12);
            assert!((a.near - b.near).abs() < 1e-12);
            assert!((a.far - b.far).abs() < 1e-12);
        }
        let bounds = protocol::Bounds {
            min: [0.; 3],
            max: [2., 2., 2.],
        };
        let aspect = 1280. / 720.;
        let framed = ViewerCamera::frame(Some(&bounds), aspect);
        let protocol_framed =
            ViewerCamera::from_protocol(protocol::view::framing_camera(Some(&bounds), aspect));
        close(&framed, &protocol_framed);
        // Without a scene camera the manifest default is the framed camera.
        let mut manifest = GeometryManifest::default();
        manifest.bounds = Some(bounds);
        close(
            &ViewerCamera::from_manifest(&manifest, aspect),
            &ViewerCamera::frame(manifest.bounds.as_ref(), aspect),
        );
        assert!(protocol::view::manifest_camera(&manifest).is_none());
        // A scene LookAt takes precedence, with identical near/far extension.
        manifest.camera = Some(protocol::geometry::CameraRecord {
            kind: "perspective".into(),
            camera_from_world: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 8., 1.,
            ],
            look_at: Some([[0., 0., 8.], [0., 0., 0.], [0., 1., 0.]]),
            fov_degrees: Some(30.),
            shutter: [0., 1.],
            screenwindow: None,
            frame_aspect_ratio: None,
            film_resolution: None,
            film_exposure: None,
        });
        let scene = ViewerCamera::from_manifest(&manifest, aspect);
        let protocol_scene =
            ViewerCamera::from_protocol(protocol::view::manifest_camera(&manifest).unwrap());
        close(&scene, &protocol_scene);
        assert_eq!(scene.fov_y_degrees, 30.);
        // The default ViewState matches core::view::initial's contract.
        let initial = protocol::view::initial_view(Some(&manifest));
        close(&scene, &ViewerCamera::from_protocol(initial.camera));
        assert!(!scene.mirror);
        // A camera with a mirror (negative determinant) keeps it, so a
        // headless capture of a right-handed scene matches the render.
        let record = manifest.camera.as_mut().unwrap();
        record.camera_from_world[0] = -1.;
        assert!(ViewerCamera::from_manifest(&manifest, aspect).mirror);
        assert_eq!(initial.size, [1280, 720]);
        assert_eq!(initial.mode, protocol::view::DisplayMode::Solid);
    }
}
