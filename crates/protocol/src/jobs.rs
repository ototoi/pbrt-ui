//! Capture, render and artifact contracts.
use crate::{
    ArtifactId, Revision, SceneId,
    view::{Camera, FrameKey},
};
use serde::{Deserialize, Serialize};
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderSettings {
    /// Deprecated compatibility field; final renders require None because
    /// the scene's Sampler controls image quality.
    pub samples_per_pixel: Option<u32>,
    /// None: the renderer decides (all cores).
    pub threads: Option<u32>,
    /// Use the renderer's GPU backend for final renders.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gpu: bool,
    /// Deprecated compatibility field; final renders require None because
    /// the scene's Integrator controls maximum depth.
    pub max_depth: Option<u32>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetRenderSettingsParams {
    pub settings: RenderSettings,
    pub expected_settings_revision: Revision,
}
/// Interactive progressive rendering (IPR) settings. IPR keeps its own low
/// sample budget so long interactive sessions do not consume the final-render
/// settings or their revision.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IprSettings {
    pub samples_per_pixel: u32,
    /// None: the renderer decides (all cores).
    pub threads: Option<u32>,
    pub max_depth: Option<u32>,
    /// Quiet period after the last trigger before an IPR job launches; edits
    /// within the window coalesce into a single job for the latest state.
    pub debounce_ms: u64,
}
impl Default for IprSettings {
    fn default() -> Self {
        Self {
            samples_per_pixel: 32,
            threads: None,
            max_depth: None,
            debounce_ms: 400,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IprSetParams {
    pub enabled: bool,
    pub settings: IprSettings,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IprSubmitParams {
    /// Present when a native project is active; must match its current
    /// activation and revision. Absent in read-only viewer mode, where core
    /// renders its own current view.
    #[serde(default)]
    pub project: Option<crate::project::ProjectRef>,
    /// Current camera/view parameters pushed by the client. In viewer mode
    /// core uses its own view instead.
    #[serde(default)]
    pub view: Option<crate::view::ViewState>,
    #[serde(default)]
    pub scene: Option<SceneId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IprStatus {
    pub enabled: bool,
    pub settings: IprSettings,
    pub active_job_id: Option<crate::JobId>,
    /// True when launch is waiting for an active final render to finish.
    pub waiting_for_final_render: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactParams {
    pub artifact_id: ArtifactId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportParams {
    pub artifact_id: ArtifactId,
    pub absolute_path: String,
    #[serde(default)]
    pub overwrite: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Artifact {
    pub artifact_id: ArtifactId,
    pub path: String,
    pub mime: String,
    pub size: u64,
    pub sha256: String,
    pub metadata: serde_json::Value,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureSpec {
    pub geometry_path: String,
    pub camera: Camera,
    pub size: [u32; 2],
    pub mode: crate::view::DisplayMode,
    /// Output linear diffuse reflectance without lighting or film exposure.
    #[serde(default)]
    pub debug_albedo: bool,
    /// Output the environment light's screen-space AO, 1 = white.
    #[serde(default)]
    pub debug_ao: bool,
    #[serde(default)]
    pub time: f64,
    pub frame_key: FrameKey,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn render_settings_gpu_defaults_false_and_omits_false() {
        let legacy: RenderSettings = serde_json::from_value(serde_json::json!({
            "samples_per_pixel": null, "threads": null, "max_depth": null
        }))
        .unwrap();
        assert!(!legacy.gpu);
        assert!(serde_json::to_value(&legacy).unwrap().get("gpu").is_none());
        let enabled: RenderSettings = serde_json::from_value(serde_json::json!({
            "samples_per_pixel": null, "threads": null, "gpu": true, "max_depth": null
        }))
        .unwrap();
        assert_eq!(serde_json::to_value(enabled).unwrap()["gpu"], true);
    }

    #[test]
    fn capture_spec_keeps_old_payloads_compatible_and_reads_debug_albedo() {
        let base = serde_json::json!({
            "geometry_path": "geometry.json",
            "camera": {
                "eye": [0.0, 0.0, 5.0],
                "target": [0.0, 0.0, 0.0],
                "up": [0.0, 1.0, 0.0],
                "projection": "perspective",
                "fov_y_degrees": 45.0,
                "near": 0.1,
                "far": 100.0
            },
            "size": [64, 64],
            "mode": "lit",
            "time": 0.0,
            "frame_key": {
                "scene_id": "00000000-0000-0000-0000-000000000001",
                "scene_revision": "0",
                "view_revision": "0"
            }
        });
        let legacy: CaptureSpec = serde_json::from_value(base.clone()).unwrap();
        assert!(!legacy.debug_albedo && !legacy.debug_ao);
        let mut ao = base.clone();
        ao["debug_ao"] = serde_json::Value::Bool(true);
        assert!(serde_json::from_value::<CaptureSpec>(ao).unwrap().debug_ao);
        let mut debug = base;
        debug["debug_albedo"] = serde_json::Value::Bool(true);
        assert!(
            serde_json::from_value::<CaptureSpec>(debug)
                .unwrap()
                .debug_albedo
        );
    }
}
