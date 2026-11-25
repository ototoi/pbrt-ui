//! Interpretation of `project view`'s JSON argument and its `--mode`,
//! `--size` and `--time` flags into the camera for `project.set_camera`
//! and the size/mode/time for `project.configure_view`.
// Every small parse here returns the crate's `ClientError`, whose
// `Remote` variant is as large as the rest of the crate's
// API already makes it; this lint would flag each helper like it flags
// the client's own methods.
#![allow(clippy::result_large_err)]

use crate::ClientError;
use protocol::view::{Camera, DisplayMode};
use serde_json::Value;

/// What `pbrt-ui-ctl project view [JSON] [flags]` establishes in the shared
/// native view; `None` fields leave the stored value untouched.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ViewUpdate {
    pub camera: Option<Camera>,
    pub size: Option<[u32; 2]>,
    pub mode: Option<DisplayMode>,
    pub time: Option<f64>,
}

impl ViewUpdate {
    /// True when `project.configure_view` has nothing to set.
    pub fn nothing_to_configure(&self) -> bool {
        self.size.is_none() && self.mode.is_none() && self.time.is_none()
    }
}

/// Interpret the subcommand's optional JSON argument (a [`Camera`], a view
/// state carrying its `camera` plus optional `size`/`mode`/`time`, or a
/// `project.get_view` result, digging `view` then `camera`) and merge it
/// with the `--mode`, `--size` and `--time` flags, which win.
pub fn interpret_view_update(
    json: Option<&Value>,
    mode: Option<&str>,
    size: Option<&str>,
    time: Option<&str>,
) -> Result<ViewUpdate, ClientError> {
    let mut update = ViewUpdate::default();
    if let Some(json) = json {
        // A project.get_view result wraps the view state in `view`.
        let mut value = json;
        if let Some(view) = value.get("view") {
            value = view;
        }
        // Null afterwards is a get_view result with no stored view.
        if !value.is_null() {
            match value.get("camera") {
                // A view state carries its camera plus the fields
                // configure_view edits.
                Some(camera) => {
                    update.camera = Some(parse_camera(camera)?);
                    update.size = json_size(value.get("size"))?;
                    update.mode = json_mode(value.get("mode"))?;
                    update.time = json_time(value.get("time"))?;
                }
                // Otherwise the JSON is the camera itself.
                None => update.camera = Some(parse_camera(value)?),
            }
        }
    }
    if let Some(mode) = mode {
        update.mode = Some(parse_mode(mode)?);
    }
    if let Some(size) = size {
        update.size = Some(parse_size(size)?);
    }
    if let Some(time) = time {
        update.time = Some(parse_time(time)?);
    }
    Ok(update)
}

fn invalid(message: &str) -> ClientError {
    ClientError::Invalid(message.into())
}

/// A camera, a view state or a project.get_view result: what failed to read
/// as a camera names the three accepted shapes.
fn not_a_camera() -> ClientError {
    invalid("project view JSON must be a camera, a view state or a project.get_view result")
}

fn parse_camera(value: &Value) -> Result<Camera, ClientError> {
    serde_json::from_value(value.clone()).map_err(|_| not_a_camera())
}

fn parse_mode(mode: &str) -> Result<DisplayMode, ClientError> {
    match mode {
        "solid" => Ok(DisplayMode::Solid),
        "normal" => Ok(DisplayMode::Normal),
        "wireframe" => Ok(DisplayMode::Wireframe),
        "lit" => Ok(DisplayMode::Lit),
        _ => Err(invalid(
            "mode must be one of solid, normal, wireframe or lit",
        )),
    }
}

/// `--size WxH`: two positive integers, e.g. `976x1280`.
fn parse_size(size: &str) -> Result<[u32; 2], ClientError> {
    const MESSAGE: &str = "--size must be WIDTHxHEIGHT with two positive integers, e.g. 976x1280";
    let (width, height) = size.split_once('x').ok_or_else(|| invalid(MESSAGE))?;
    let dimension = |text: &str| {
        text.parse::<u32>()
            .ok()
            .filter(|n| *n > 0)
            .ok_or_else(|| invalid(MESSAGE))
    };
    Ok([dimension(width)?, dimension(height)?])
}

/// `--time T`: a finite number.
fn parse_time(time: &str) -> Result<f64, ClientError> {
    time.parse::<f64>()
        .ok()
        .filter(|t| t.is_finite())
        .ok_or_else(|| invalid("--time must be a finite number"))
}

/// A JSON field that is absent or null leaves the stored value untouched.
fn json_field(value: Option<&Value>) -> Option<&Value> {
    value.filter(|value| !value.is_null())
}

fn json_size(value: Option<&Value>) -> Result<Option<[u32; 2]>, ClientError> {
    const MESSAGE: &str = "size must be two positive integers, e.g. [976, 1280]";
    let Some(value) = json_field(value) else {
        return Ok(None);
    };
    let dimensions = value
        .as_array()
        .filter(|dimensions| dimensions.len() == 2)
        .ok_or_else(|| invalid(MESSAGE))?;
    let mut parsed = [0u32; 2];
    for (slot, dimension) in parsed.iter_mut().zip(dimensions) {
        *slot = dimension
            .as_u64()
            .filter(|n| *n > 0)
            .and_then(|n| u32::try_from(n).ok())
            .ok_or_else(|| invalid(MESSAGE))?;
    }
    Ok(Some(parsed))
}

fn json_mode(value: Option<&Value>) -> Result<Option<DisplayMode>, ClientError> {
    let Some(value) = json_field(value) else {
        return Ok(None);
    };
    parse_mode(
        value
            .as_str()
            .ok_or_else(|| invalid("mode must be one of solid, normal, wireframe or lit"))?,
    )
    .map(Some)
}

fn json_time(value: Option<&Value>) -> Result<Option<f64>, ClientError> {
    let Some(value) = json_field(value) else {
        return Ok(None);
    };
    value
        .as_f64()
        .filter(|t| t.is_finite())
        .map(Some)
        .ok_or_else(|| invalid("time must be a finite number"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::view::Projection;
    use serde_json::json;

    fn camera() -> Camera {
        Camera {
            eye: [0., 0., 8.],
            target: [0., 0., 0.],
            up: [0., 1., 0.],
            projection: Projection::Perspective,
            fov_y_degrees: 45.,
            near: 0.1,
            far: 100.,
            mirror: false,
        }
    }

    fn camera_json() -> Value {
        json!({
            "eye": [0., 0., 8.],
            "target": [0., 0., 0.],
            "up": [0., 1., 0.],
            "projection": "perspective",
            "fov_y_degrees": 45.,
            "near": 0.1,
            "far": 100.,
        })
    }

    fn view_state_json() -> Value {
        json!({
            "camera": camera_json(),
            "size": [976, 1280],
            "mode": "lit",
            "time": 0.25,
        })
    }

    /// A plain Camera argument sets only the camera, leaving configure_view
    /// with nothing to configure.
    #[test]
    fn camera_json_sets_only_the_camera() {
        let update = interpret_view_update(Some(&camera_json()), None, None, None).unwrap();
        assert_eq!(update.camera, Some(camera()));
        assert_eq!(update.size, None);
        assert_eq!(update.mode, None);
        assert_eq!(update.time, None);
        assert!(update.nothing_to_configure());
    }

    /// A view state argument carries its camera plus size, mode and time.
    #[test]
    fn view_state_json_sets_camera_and_view_fields() {
        let update = interpret_view_update(Some(&view_state_json()), None, None, None).unwrap();
        assert_eq!(update.camera, Some(camera()));
        assert_eq!(update.size, Some([976, 1280]));
        assert_eq!(update.mode, Some(DisplayMode::Lit));
        assert_eq!(update.time, Some(0.25));
        assert!(!update.nothing_to_configure());
    }

    /// The view state's size/mode/time are optional in the CLI's reading.
    #[test]
    fn view_state_json_may_omit_size_mode_and_time() {
        let update =
            interpret_view_update(Some(&json!({"camera": camera_json()})), None, None, None)
                .unwrap();
        assert_eq!(update.camera, Some(camera()));
        assert!(update.nothing_to_configure());
    }

    /// A project.get_view result is dug through its `view` field.
    #[test]
    fn get_view_result_is_dug_for_camera_and_fields() {
        let result = json!({
            "project": {"id": "…"},
            "view": view_state_json(),
            "view_revision": "3",
        });
        let update = interpret_view_update(Some(&result), None, None, None).unwrap();
        assert_eq!(update.camera, Some(camera()));
        assert_eq!(update.size, Some([976, 1280]));
        assert_eq!(update.mode, Some(DisplayMode::Lit));
        assert_eq!(update.time, Some(0.25));
    }

    /// The flags override the matching JSON fields.
    #[test]
    fn flags_override_the_json() {
        let update = interpret_view_update(
            Some(&view_state_json()),
            Some("solid"),
            Some("320x200"),
            Some("9"),
        )
        .unwrap();
        assert_eq!(update.camera, Some(camera()));
        assert_eq!(update.size, Some([320, 200]));
        assert_eq!(update.mode, Some(DisplayMode::Solid));
        assert_eq!(update.time, Some(9.));
    }

    /// Flags without any JSON configure the view without touching the camera.
    #[test]
    fn flags_without_json_leave_the_camera_alone() {
        let update =
            interpret_view_update(None, Some("wireframe"), Some("976x1280"), None).unwrap();
        assert_eq!(update.camera, None);
        assert_eq!(update.size, Some([976, 1280]));
        assert_eq!(update.mode, Some(DisplayMode::Wireframe));
        assert_eq!(update.time, None);
    }

    /// A malformed size is a client error, from a flag or from the JSON.
    #[test]
    fn bad_size_is_a_client_error() {
        for size in [
            "976",
            "976x",
            "976x1280x64",
            "0x1280",
            "976x0",
            "ax1280",
            "976.5x1280",
        ] {
            let error = interpret_view_update(None, None, Some(size), None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("size"), "{size}: {error}");
        }
        for size in [
            json!([976]),
            json!([0, 1280]),
            json!([976, 1280, 3]),
            json!("big"),
            json!([-976, -1280]),
        ] {
            let value = json!({"camera": camera_json(), "size": size});
            let error = interpret_view_update(Some(&value), None, None, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("size"), "{size}: {error}");
        }
    }

    /// An --mode outside the four display modes is a client error, as is a
    /// malformed mode in the JSON.
    #[test]
    fn bad_mode_is_a_client_error() {
        for mode in ["shaded", "Solid", ""] {
            let error = interpret_view_update(None, Some(mode), None, None)
                .unwrap_err()
                .to_string();
            assert!(error.contains("mode"), "{mode}: {error}");
        }
        let value = json!({"camera": camera_json(), "mode": 5});
        let error = interpret_view_update(Some(&value), None, None, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("mode"), "{error}");
    }

    /// --time must be a finite number.
    #[test]
    fn bad_time_is_a_client_error() {
        for time in ["inf", "abc", ""] {
            let error = interpret_view_update(None, None, None, Some(time))
                .unwrap_err()
                .to_string();
            assert!(error.contains("time"), "{time}: {error}");
        }
        let value = json!({"camera": camera_json(), "time": "now"});
        let error = interpret_view_update(Some(&value), None, None, None)
            .unwrap_err()
            .to_string();
        assert!(error.contains("time"), "{error}");
    }

    /// JSON that is neither a camera, a view state nor a get_view result is
    /// a client error.
    #[test]
    fn json_without_camera_or_view_is_rejected() {
        for value in [json!({"foo": 1}), json!(5), json!("camera")] {
            let error = interpret_view_update(Some(&value), None, None, None)
                .unwrap_err()
                .to_string();
            assert!(
                error.contains("camera") || error.contains("view"),
                "{value}: {error}"
            );
        }
    }

    /// A get_view result from before any view exists leaves nothing from the
    /// JSON; flags still apply.
    #[test]
    fn get_view_result_with_null_view_sets_only_the_flags() {
        let result = json!({
            "project": {"id": "…"},
            "view": null,
            "view_revision": "0",
        });
        let update = interpret_view_update(Some(&result), Some("lit"), None, None).unwrap();
        assert_eq!(update.camera, None);
        assert_eq!(update.mode, Some(DisplayMode::Lit));
    }

    /// No JSON and no flags configures nothing (the plain `project view` read).
    #[test]
    fn empty_input_configures_nothing() {
        let update = interpret_view_update(None, None, None, None).unwrap();
        assert_eq!(update.camera, None);
        assert!(update.nothing_to_configure());
    }
}
