//! NDJSON wire envelope: Request / Response / Event.
//!
//! One UTF-8 JSON object per line, 8 MiB line cap. `result` and `error` are
//! mutually exclusive. Unknown methods/fields/enums are rejected at parse time.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::ErrorPayload;

/// Wire protocol version negotiated in `app.hello`.
pub const PROTOCOL_VERSION: &str = "2";

/// Maximum accepted size of a single NDJSON line, in bytes.
pub const MAX_LINE_BYTES: usize = 8 * 1024 * 1024;

/// Known v2 methods. Unknown method strings fail to deserialize (rejected).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Method {
    #[serde(rename = "app.hello")]
    AppHello,
    #[serde(rename = "app.get_state")]
    AppGetState,
    #[serde(rename = "app.get_capabilities")]
    AppGetCapabilities,
    #[serde(rename = "app.quit")]
    AppQuit,
    #[serde(rename = "project.get_state")]
    ProjectGetState,
    #[serde(rename = "project.import")]
    ProjectImport,
    #[serde(rename = "project.import_scene")]
    ProjectImportScene,
    #[serde(rename = "project.import_resource")]
    ProjectImportResource,
    #[serde(rename = "project.open")]
    ProjectOpen,
    #[serde(rename = "project.create")]
    ProjectCreate,
    #[serde(rename = "project.close")]
    ProjectClose,
    #[serde(rename = "project.save")]
    ProjectSave,
    #[serde(rename = "project.export")]
    ProjectExport,
    #[serde(rename = "project.preview")]
    ProjectPreview,
    /// per-entity transform deltas for a transform-only
    /// edit, instead of rebuilding the preview package.
    #[serde(rename = "project.preview_transforms")]
    ProjectPreviewTransforms,
    #[serde(rename = "project.render")]
    ProjectRender,
    #[serde(rename = "project.capture")]
    ProjectCapture,
    #[serde(rename = "project.pick")]
    ProjectPick,
    #[serde(rename = "project.list_nodes")]
    ProjectListNodes,
    #[serde(rename = "project.list_contents")]
    ProjectListContents,
    /// where one project item is used.
    #[serde(rename = "project.list_references")]
    ProjectListReferences,
    #[serde(rename = "project.list_scenes")]
    ProjectListScenes,
    #[serde(rename = "project.open_scene")]
    ProjectOpenScene,
    #[serde(rename = "project.close_scene")]
    ProjectCloseScene,
    #[serde(rename = "project.list_presets")]
    ProjectListPresets,
    #[serde(rename = "project.get_view")]
    ProjectGetView,
    #[serde(rename = "project.set_camera")]
    ProjectSetCamera,
    #[serde(rename = "project.configure_view")]
    ProjectConfigureView,
    #[serde(rename = "project.search_nodes")]
    ProjectSearchNodes,
    #[serde(rename = "project.search_contents")]
    ProjectSearchContents,
    #[serde(rename = "project.get_node")]
    ProjectGetNode,
    #[serde(rename = "project.get_resource")]
    ProjectGetResource,
    #[serde(rename = "project.get_asset")]
    ProjectGetAsset,
    #[serde(rename = "project.get_cache")]
    ProjectGetCache,
    #[serde(rename = "project.put_cache")]
    ProjectPutCache,
    #[serde(rename = "project.get_parameters")]
    ProjectGetParameters,
    #[serde(rename = "project.get_parameter_values")]
    ProjectGetParameterValues,
    #[serde(rename = "project.set_parameter_values")]
    ProjectSetParameterValues,
    #[serde(rename = "project.get_parameter_schema")]
    ProjectGetParameterSchema,
    #[serde(rename = "project.plan_implementation")]
    ProjectPlanImplementation,
    #[serde(rename = "project.get_resource_graph")]
    ProjectGetResourceGraph,
    #[serde(rename = "project.list_spectra")]
    ProjectListSpectra,
    #[serde(rename = "project.spectrum_color")]
    ProjectSpectrumColor,
    #[serde(rename = "project.preview_texture")]
    ProjectPreviewTexture,
    #[serde(rename = "project.preview_material")]
    ProjectPreviewMaterial,
    #[serde(rename = "project.preview_asset")]
    ProjectPreviewAsset,
    #[serde(rename = "project.apply")]
    ProjectApply,
    #[serde(rename = "project.undo")]
    ProjectUndo,
    #[serde(rename = "project.redo")]
    ProjectRedo,
    #[serde(rename = "render.get_settings")]
    RenderGetSettings,
    #[serde(rename = "render.set_settings")]
    RenderSetSettings,
    #[serde(rename = "render.ipr_set")]
    RenderIprSet,
    #[serde(rename = "render.ipr_submit")]
    RenderIprSubmit,
    #[serde(rename = "job.get")]
    JobGet,
    #[serde(rename = "job.list")]
    JobList,
    #[serde(rename = "job.cancel")]
    JobCancel,
    #[serde(rename = "events.read")]
    EventsRead,
    #[serde(rename = "artifact.get")]
    ArtifactGet,
    #[serde(rename = "artifact.export")]
    ArtifactExport,
    #[serde(rename = "workspace.reset_layout")]
    WorkspaceResetLayout,
}

/// A client request. `params` is method-specific and validated by core.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    pub protocol: String,
    pub request_id: String,
    pub method: Method,
    #[serde(default)]
    pub params: Value,
}

/// A server response. Exactly one of `result` / `error` is set.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ResponseFields")]
pub struct Response {
    pub protocol: String,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<ErrorPayload>,
}

// Preserve the presence of `result: null`: it is a valid successful response,
// whereas an absent result and an absent error are not a response.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResponseFields {
    protocol: String,
    request_id: String,
    #[serde(default, deserialize_with = "present_result")]
    result: Option<Value>,
    #[serde(default, deserialize_with = "present_result")]
    error: Option<Value>,
}

fn present_result<'de, D: serde::Deserializer<'de>>(de: D) -> Result<Option<Value>, D::Error> {
    Value::deserialize(de).map(Some)
}

impl TryFrom<ResponseFields> for Response {
    type Error = String;

    fn try_from(fields: ResponseFields) -> Result<Self, Self::Error> {
        if fields.result.is_some() == fields.error.is_some() {
            return Err("response must contain exactly one of result or error".into());
        }
        Ok(Self {
            protocol: fields.protocol,
            request_id: fields.request_id,
            result: fields.result,
            error: fields
                .error
                .map(serde_json::from_value)
                .transpose()
                .map_err(|error| error.to_string())?,
        })
    }
}

impl Response {
    pub fn ok(request_id: impl Into<String>, result: Value) -> Self {
        Response {
            protocol: PROTOCOL_VERSION.to_string(),
            request_id: request_id.into(),
            result: Some(result),
            error: None,
        }
    }

    pub fn err(request_id: impl Into<String>, error: ErrorPayload) -> Self {
        Response {
            protocol: PROTOCOL_VERSION.to_string(),
            request_id: request_id.into(),
            result: None,
            error: Some(error),
        }
    }
}

/// A monotonic event emitted by core.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    /// Monotonically increasing sequence, decimal string on the wire.
    pub sequence: String,
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scene_id: Option<crate::ids::SceneId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job_id: Option<crate::ids::JobId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_roundtrip_and_unknown_method_rejected() {
        let line =
            r#"{"protocol":"1","request_id":"req-1","method":"project.set_camera","params":{}}"#;
        let req: Request = serde_json::from_str(line).unwrap();
        assert_eq!(req.method, Method::ProjectSetCamera);

        let bad = r#"{"protocol":"1","request_id":"r","method":"no.such.method","params":{}}"#;
        assert!(serde_json::from_str::<Request>(bad).is_err());
    }

    #[test]
    fn result_and_error_are_exclusive_in_practice() {
        let ok = Response::ok("r1", serde_json::json!({"view_revision": "9"}));
        assert!(ok.result.is_some() && ok.error.is_none());
        let e = Response::err(
            "r1",
            ErrorPayload::new(
                crate::error::ErrorCode::RevisionConflict,
                "view revision changed",
            ),
        );
        assert!(e.result.is_none() && e.error.is_some());
    }
}
