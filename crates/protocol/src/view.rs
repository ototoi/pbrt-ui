//! Shared view DTOs; no GUI or GPU types.
use crate::{
    Revision, SceneId,
    geometry::{Bounds, CameraRecord, GeometryManifest},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Camera {
    pub eye: [f64; 3],
    pub target: [f64; 3],
    pub up: [f64; 3],
    pub projection: Projection,
    pub fov_y_degrees: f64,
    pub near: f64,
    pub far: f64,
    /// Mirror the image left-right, like the `Scale -1 1 1` a right-handed
    /// scene puts before its camera.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mirror: bool,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Projection {
    Perspective,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    Solid,
    Normal,
    Wireframe,
    /// Approximate lit preview: Lambert shading from the scene's light
    /// records plus emissive area lights.
    Lit,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ViewState {
    pub camera: Camera,
    pub size: [u32; 2],
    pub mode: DisplayMode,
    pub time: f64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameKey {
    pub scene_id: SceneId,
    pub scene_revision: Revision,
    pub view_revision: Revision,
}

/// Canonical framing and initial-camera math shared by the desktop viewport,
/// the capture adapter and core validation. Everything stays f64/pure so both
/// viewers and headless hosts derive identical cameras.
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    (0..3).map(|i| a[i] * b[i]).sum()
}
fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    std::array::from_fn(|i| a[i] - b[i])
}
/// True when eye/target/up, field of view and clipping planes form a usable
/// perspective camera; the single predicate behind `view.*` validation.
pub fn camera_is_valid(camera: &Camera) -> bool {
    let forward = sub(camera.target, camera.eye);
    let cross = [
        forward[1] * camera.up[2] - forward[2] * camera.up[1],
        forward[2] * camera.up[0] - forward[0] * camera.up[2],
        forward[0] * camera.up[1] - forward[1] * camera.up[0],
    ];
    camera
        .eye
        .iter()
        .chain(&camera.target)
        .chain(&camera.up)
        .all(|x| x.is_finite())
        && dot(forward, forward) >= 1e-18
        && dot(cross, cross) >= 1e-18
        && camera.fov_y_degrees.is_finite()
        && (0.1..179.).contains(&camera.fov_y_degrees)
        && camera.near.is_finite()
        && camera.far.is_finite()
        && camera.near > 0.
        && camera.far > camera.near
}
/// Camera that fits `bounds` (or the unit sphere) into a 45° view at `aspect`.
pub fn framing_camera(bounds: Option<&Bounds>, aspect: f64) -> Camera {
    framing_camera_up(bounds, aspect, crate::project::UpAxis::PosY)
}
/// `framing_camera` around a scene's up axis: `up` points up
/// on screen and the eye sits above the horizon on the scene's front side
/// (+Z for Y-up, −Y for Z-up), so orbiting turns about the scene's up.
pub fn framing_camera_up(
    bounds: Option<&Bounds>,
    aspect: f64,
    up: crate::project::UpAxis,
) -> Camera {
    use crate::project::UpAxis;
    let (center, radius) = bounds
        .map(|b| (b.center(), b.half_diagonal().max(1e-3)))
        .unwrap_or(([0.; 3], 1.));
    let angle = 45f64.to_radians() / 2.;
    let distance = radius * 1.1 / angle.min((aspect * angle.tan()).atan()).sin();
    // The horizontal side the eye starts on, perpendicular to `up`.
    let front = match up {
        UpAxis::PosY | UpAxis::PosX | UpAxis::NegX => [0., 0., 1.],
        UpAxis::NegY => [0., 0., -1.],
        UpAxis::PosZ => [0., -1., 0.],
        UpAxis::NegZ => [0., 1., 0.],
    };
    let up_vector = up.vector();
    // 25° above the horizon.
    let (sin, cos) = 25f64.to_radians().sin_cos();
    let back: [f64; 3] = std::array::from_fn(|i| front[i] * cos + up_vector[i] * sin);
    Camera {
        eye: std::array::from_fn(|i| center[i] + back[i] * distance),
        target: center,
        up: up_vector,
        projection: Projection::Perspective,
        fov_y_degrees: 45.,
        near: (distance - radius * 1.5).max(0.001),
        far: distance + radius * 2.,
        mirror: false,
    }
}
/// Vertical fov (degrees) a scene camera gives the view. PBRT's `fov` is the
/// angle that maps to ±1 in screen space, and the screen window sets how far
/// the vertical axis extends: an explicit `screenwindow` gives it directly
/// (`(top − bottom) / 2`, so Render View's `[-aspect aspect -1 1]` keeps its
/// vertical fov); otherwise r4 screens `[-frame, frame] × [-1, 1]` when
/// `frame > 1` and `[-1, 1] × [-1/frame, 1/frame]` below, where `frame` is
/// `frameaspectratio` or the film's aspect (`pbrt-r4/src/cameras/perspective.rs`).
/// A portrait film therefore widens to `2·atan(tan(fov/2) / frame)`. Without a
/// window, `frameaspectratio` or `film_resolution` the recorded fov passes
/// through. The viewer rejects 179° and above; a widened angle is clamped to
/// stay just under it so the scene camera's pose is kept.
pub fn camera_fov_y(
    fov_degrees: Option<f64>,
    screenwindow: Option<[f64; 4]>,
    frame_aspect_ratio: Option<f64>,
    film_aspect: Option<f64>,
) -> f64 {
    const CEILING: f64 = 178.9;
    let fov = fov_degrees.unwrap_or(45.);
    let extent = match screenwindow {
        Some([_, _, bottom, top]) if top > bottom => (top - bottom) / 2.,
        _ => match frame_aspect_ratio.or(film_aspect) {
            Some(frame) if frame > 0. && frame < 1. => 1. / frame,
            _ => 1.,
        },
    };
    if extent == 1. {
        return fov;
    }
    (2. * ((fov.to_radians() / 2.).tan() * extent).atan().to_degrees()).min(CEILING)
}
fn camera_record_fov_y(record: &CameraRecord) -> f64 {
    camera_fov_y(
        record.fov_degrees,
        record.screenwindow,
        record.frame_aspect_ratio,
        record
            .film_resolution
            .filter(|[w, h]| *w > 0 && *h > 0)
            .map(|[w, h]| w as f64 / h as f64),
    )
}
/// Scene-camera candidate from a manifest's parsed `Camera "perspective"`:
/// `None` when the manifest has no usable perspective LookAt.
pub fn manifest_camera(manifest: &GeometryManifest) -> Option<Camera> {
    let radius = manifest
        .bounds
        .as_ref()
        .map(Bounds::half_diagonal)
        .unwrap_or(1.)
        .max(1e-3);
    let record = manifest
        .camera
        .as_ref()
        .filter(|c| c.kind == "perspective")?;
    let [eye, target, up] = record.look_at?;
    let distance = dot(sub(eye, target), sub(eye, target)).sqrt();
    // A negative 3×3 determinant is the `Scale -1 1 1` of a right-handed
    // scene's camera, so headless captures mirror like the render.
    let m = &record.camera_from_world;
    let det = m[0] * (m[5] * m[10] - m[6] * m[9]) - m[1] * (m[4] * m[10] - m[6] * m[8])
        + m[2] * (m[4] * m[9] - m[5] * m[8]);
    let candidate = Camera {
        eye,
        target,
        up,
        projection: Projection::Perspective,
        fov_y_degrees: camera_record_fov_y(record),
        near: (distance - radius * 2.).max(0.001),
        far: distance + radius * 2.,
        mirror: det < 0.,
    };
    camera_is_valid(&candidate).then_some(candidate)
}
/// Default ViewState for a freshly loaded geometry manifest: scene camera when
/// usable, framed camera otherwise, mid supported time, solid, 16:9.
pub fn initial_view(manifest: Option<&GeometryManifest>) -> ViewState {
    let camera = manifest
        .and_then(manifest_camera)
        .unwrap_or_else(|| framing_camera(manifest.and_then(|g| g.bounds.as_ref()), 16. / 9.));
    ViewState {
        camera,
        size: [1280, 720],
        mode: DisplayMode::Solid,
        time: manifest
            .filter(|m| !m.supported_times.is_empty())
            .map(|m| m.supported_times[m.supported_times.len() / 2])
            .unwrap_or(0.),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::UpAxis;

    /// Default framing keeps the eye on +Z for a +Y-up scene and turns with
    /// the scene's up axis, so a Z-up scene opens from the side.
    #[test]
    fn framing_follows_the_scene_up_axis() {
        let y = framing_camera(None, 1.);
        assert_eq!(y.up, [0., 1., 0.]);
        assert!(
            y.eye[2] > 0. && y.eye[1] > 0. && y.eye[0] == 0.,
            "{:?}",
            y.eye
        );
        for up in [
            UpAxis::PosX,
            UpAxis::NegX,
            UpAxis::PosY,
            UpAxis::NegY,
            UpAxis::PosZ,
            UpAxis::NegZ,
        ] {
            let camera = framing_camera_up(None, 1., up);
            assert_eq!(camera.up, up.vector());
            assert!(camera_is_valid(&camera), "{up:?}");
            // Above the horizon of the scene's up.
            assert!(
                dot(sub(camera.eye, camera.target), up.vector()) > 0.,
                "{up:?}"
            );
        }
        let mut mirrored = framing_camera(None, 1.);
        assert!(!serde_json::to_string(&mirrored).unwrap().contains("mirror"));
        mirrored.mirror = true;
        let text = serde_json::to_string(&mirrored).unwrap();
        assert!(text.contains(r#""mirror":true"#));
        assert_eq!(serde_json::from_str::<Camera>(&text).unwrap(), mirrored);
        let z = framing_camera_up(None, 1., UpAxis::PosZ);
        assert!(
            z.eye[1] < 0. && z.eye[2] > 0.,
            "Z-up frames from -Y, above: {:?}",
            z.eye
        );
    }

    /// Manifest whose perspective camera sits 8 above the origin looking
    /// down −Z with `fov` over `film_resolution`.
    fn camera_manifest(fov: Option<f64>, film_resolution: Option<[u32; 2]>) -> GeometryManifest {
        GeometryManifest {
            camera: Some(CameraRecord {
                kind: "perspective".into(),
                camera_from_world: [
                    1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 8., 1.,
                ],
                look_at: Some([[0., 0., 8.], [0., 0., 0.], [0., 1., 0.]]),
                fov_degrees: fov,
                screenwindow: None,
                frame_aspect_ratio: None,
                shutter: [0., 1.],
                film_resolution,
                film_exposure: None,
            }),
            ..GeometryManifest::new()
        }
    }

    /// A scene camera's `fov` spans the film's shorter axis (r4
    /// `perspective` puts it on the ±1 screen-window axis), so a portrait
    /// film carries a horizontal angle: the view's vertical angle widens by
    /// the screen extent above the frame axis, 2·atan(h/w · tan(fov/2)).
    #[test]
    fn portrait_film_widens_the_scene_camera_fov() {
        let manifest = camera_manifest(Some(47.), Some([1000, 1400]));
        let fov_y = manifest_camera(&manifest).unwrap().fov_y_degrees;
        // 2·atan(1.4·tan(23.5°)) ≈ 62.66, not the recorded 47.
        assert!((fov_y - 62.6609).abs() < 1e-3, "{fov_y}");
    }

    /// Landscape films screen at [-frame, frame] × [-1, 1], so the record's
    /// fov already spans the vertical axis and must survive unchanged.
    #[test]
    fn landscape_film_keeps_the_recorded_vertical_fov() {
        let manifest = camera_manifest(Some(30.), Some([1280, 720]));
        assert_eq!(manifest_camera(&manifest).unwrap().fov_y_degrees, 30.);
    }

    /// Square films screen at [-1, 1] × [-1, 1]; the fov spans both axes
    /// and passes through unchanged.
    #[test]
    fn square_film_keeps_the_recorded_vertical_fov() {
        let manifest = camera_manifest(Some(40.), Some([1000, 1000]));
        assert_eq!(manifest_camera(&manifest).unwrap().fov_y_degrees, 40.);
    }

    /// Without `film_resolution` the shorter axis is unknown, so the
    /// recorded fov keeps flowing through unchanged.
    #[test]
    fn camera_without_film_resolution_keeps_the_recorded_fov() {
        let manifest = camera_manifest(Some(30.), None);
        assert_eq!(manifest_camera(&manifest).unwrap().fov_y_degrees, 30.);
    }

    /// An explicit screen window sets the vertical extent itself: Render
    /// View writes `[-aspect aspect -1 1]` with a vertical fov, which must
    /// not be widened again on a portrait film.
    #[test]
    fn explicit_screenwindow_keeps_a_vertical_fov() {
        let mut manifest = camera_manifest(Some(45.), Some([1000, 1400]));
        manifest.camera.as_mut().unwrap().screenwindow = Some([-0.714, 0.714, -1., 1.]);
        assert_eq!(manifest_camera(&manifest).unwrap().fov_y_degrees, 45.);
        manifest.camera.as_mut().unwrap().screenwindow = Some([-1., 1., -2., 2.]);
        let fov_y = manifest_camera(&manifest).unwrap().fov_y_degrees;
        assert!((fov_y - 2. * (2. * 22.5f64.to_radians().tan()).atan().to_degrees()).abs() < 1e-9);
    }

    /// `frameaspectratio` replaces the film's aspect in the default window.
    #[test]
    fn frame_aspect_ratio_replaces_the_film_aspect() {
        let mut manifest = camera_manifest(Some(40.), Some([1280, 720]));
        manifest.camera.as_mut().unwrap().frame_aspect_ratio = Some(0.5);
        let fov_y = manifest_camera(&manifest).unwrap().fov_y_degrees;
        let want = 2. * (2. * 20f64.to_radians().tan()).atan().to_degrees();
        assert!((fov_y - want).abs() < 1e-9, "{fov_y}");
    }

    /// A widened angle that would pass the viewer's ceiling keeps the scene
    /// camera (clamped) rather than dropping its pose for a framing camera.
    #[test]
    fn widened_fov_stays_below_the_viewer_ceiling() {
        let manifest = camera_manifest(Some(178.), Some([1000, 3000]));
        let camera = manifest_camera(&manifest).expect("the scene camera survives");
        assert!(camera.fov_y_degrees < 179. && camera.fov_y_degrees > 178.);
    }
}
