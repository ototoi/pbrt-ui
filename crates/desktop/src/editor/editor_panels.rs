//! Dedicated resource editors.
//!
//! Contents double-click, `Open in Editor` and the Inspector header look the
//! editor up here; a kind with no registered editor keeps `Open in Editor`
//! disabled. Each registered kind, including ArrayEditor, reuses one Dock tab.
//! These editor tabs are never saved in the layout.
use serde_json::Value;

use super::image_display;

/// The Dock `panel_name` every editor tab uses; layout saves drop it.
pub const PANEL_NAME: &str = "ResourceEditor";

/// The registered editors.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum EditorKind {
    /// Read-only view of an image asset. Registered for PNG for now, not
    /// because of the decoder — the desktop's `image` build also decodes
    /// EXR, HDR, JPEG, TGA and more through gpui-kit's feature integration
    /// (PFM excepted) — but because the registration is scoped to PNG;
    /// widens it.
    ImageViewer,
    /// A material's upstream graph with the selected node's
    /// parameters. The Texture Editor is the same node editor.
    MaterialEditor,
    TextureEditor,
    /// A read-only 3D view of a PLY asset.
    MeshViewer,
    ArrayEditor,
    /// A Spectrum Data resource's curve, approximate colours and
    /// sample table.
    SpectrumEditor,
}

impl EditorKind {
    pub fn name(self) -> &'static str {
        match self {
            Self::ImageViewer => "Image Viewer",
            Self::MaterialEditor => "Material Editor",
            Self::TextureEditor => "Texture Editor",
            Self::MeshViewer => "Mesh Viewer",
            Self::ArrayEditor => "Array Editor",
            Self::SpectrumEditor => "Spectrum Data Editor",
        }
    }

    /// The node editor rather than an asset view.
    pub fn is_graph(self) -> bool {
        matches!(self, Self::MaterialEditor | Self::TextureEditor)
    }
}

/// Window/editor registry key; each editor kind owns one panel.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum EditorIdentity {
    Kind(EditorKind),
}

pub fn identity(kind: EditorKind, _entry: &Value) -> EditorIdentity {
    EditorIdentity::Kind(kind)
}

pub fn response_is_current(
    latest: &std::collections::HashMap<EditorIdentity, u64>,
    identity: &EditorIdentity,
    request_id: u64,
    response_project: &Value,
    current_project: &Value,
) -> bool {
    response_project == current_project && latest.get(identity) == Some(&request_id)
}

pub fn should_show_editor_on_response(
    editor_existed_before_reply: bool,
    panel_is_open: bool,
) -> bool {
    !editor_existed_before_reply || panel_is_open
}

pub fn project_scope(project: &Value) -> String {
    format!("{}:{}", project["project_id"], project["activation"])
}

/// The editor registered for a Contents entry: viewable images, PLY
/// meshes, and the project's own materials and textures. Media have none
/// yet.
pub fn editor_for(entry: &Value) -> Option<EditorKind> {
    if entry["kind"] == "parameter" && entry["editor_kind"] == "array" {
        return Some(EditorKind::ArrayEditor);
    }
    if entry["kind"] == "asset" && entry["media_type"] == "application/x-ply" {
        return Some(EditorKind::MeshViewer);
    }
    if entry["kind"] == "resource" && entry["builtin"] != true {
        return match entry["resource_kind"].as_str() {
            Some("material") => Some(EditorKind::MaterialEditor),
            Some("texture") => Some(EditorKind::TextureEditor),
            Some("spectrum_data") => Some(EditorKind::SpectrumEditor),
            _ => None,
        };
    }
    (entry["kind"] == "asset"
        && entry["media_type"]
            .as_str()
            .is_some_and(|media_type| image_display::VIEWER_MEDIA_TYPES.contains(&media_type)))
    .then_some(EditorKind::ImageViewer)
}

/// A tab title: `<editor>: <item name>`.
pub fn title(kind: EditorKind, entry: &Value) -> String {
    match entry["name"].as_str().filter(|name| !name.is_empty()) {
        Some(name) => format!("{}: {name}", kind.name()),
        None => kind.name().to_owned(),
    }
}

/// Remove editor tabs from a dumped Dock layout so they are not restored.
/// Tab groups keep a valid active index.
pub fn strip_editors(state: &mut Value) {
    match state {
        Value::Object(object) => {
            if let Some(Value::Array(children)) = object.get_mut("children") {
                children.retain(|child| child["panel_name"] != PANEL_NAME);
                let len = children.len();
                if let Some(active) = object
                    .get_mut("info")
                    .and_then(|info| info.get_mut("tabs"))
                    .and_then(|tabs| tabs.get_mut("active_index"))
                {
                    let index = active.as_u64().unwrap_or(0) as usize;
                    *active = Value::from(index.min(len.saturating_sub(1)));
                }
            }
            for value in object.values_mut() {
                strip_editors(value);
            }
        }
        Value::Array(values) => values.iter_mut().for_each(strip_editors),
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn viewable_image_assets_have_an_editor() {
        let png = json!({"kind":"asset","media_type":"image/png","name":"checker.png"});
        assert_eq!(editor_for(&png), Some(EditorKind::ImageViewer));
        assert_eq!(
            title(EditorKind::ImageViewer, &png),
            "Image Viewer: checker.png"
        );
        for media_type in [
            "image/jpeg",
            "image/x-exr",
            "image/vnd.radiance",
            "image/x-tga",
        ] {
            assert_eq!(
                editor_for(&json!({"kind":"asset","media_type":media_type})),
                Some(EditorKind::ImageViewer),
                "{media_type}"
            );
        }
        // PLY meshes open the Mesh Viewer.
        let ply = json!({"kind":"asset","media_type":"application/x-ply","name":"bunny.ply"});
        assert_eq!(editor_for(&ply), Some(EditorKind::MeshViewer));
        assert_eq!(
            title(EditorKind::MeshViewer, &ply),
            "Mesh Viewer: bunny.ply"
        );
        for entry in [
            // The image build has no PFM decoder: keep Open in Editor disabled.
            json!({"kind":"asset","media_type":"image/x-portable-floatmap"}),
            json!({"kind":"resource","resource_kind":"medium"}),
            json!({"kind":"resource","resource_kind":"material","builtin":true}),
        ] {
            assert_eq!(editor_for(&entry), None, "{entry}");
        }
    }

    #[test]
    fn spectrum_data_resources_open_the_spectrum_editor() {
        let entry = json!({"kind":"resource","resource_kind":"spectrum_data",
                           "id":"00000000-0000-0000-0000-0000000000aa",
                           "name":"Au.k"});
        assert_eq!(editor_for(&entry), Some(EditorKind::SpectrumEditor));
        assert_eq!(
            title(EditorKind::SpectrumEditor, &entry),
            "Spectrum Data Editor: Au.k"
        );
        // Builtin entries never carry the id a tab needs.
        assert_eq!(
            editor_for(&json!({"kind":"resource","resource_kind":"spectrum_data","builtin":true})),
            None
        );
    }

    #[test]
    fn materials_and_textures_open_the_node_editor() {
        let wall = json!({"kind":"resource","resource_kind":"material","name":"wall"});
        assert_eq!(editor_for(&wall), Some(EditorKind::MaterialEditor));
        assert_eq!(
            title(EditorKind::MaterialEditor, &wall),
            "Material Editor: wall"
        );
        let checks = json!({"kind":"resource","resource_kind":"texture","name":"checks"});
        assert_eq!(editor_for(&checks), Some(EditorKind::TextureEditor));
        assert!(EditorKind::TextureEditor.is_graph());
        assert!(!EditorKind::ImageViewer.is_graph());
    }

    #[test]
    fn array_parameters_open_their_dedicated_editor_tab() {
        let entry = json!({"kind":"parameter","editor_kind":"array","name":"P"});
        assert_eq!(editor_for(&entry), Some(EditorKind::ArrayEditor));
        assert_eq!(title(EditorKind::ArrayEditor, &entry), "Array Editor: P");
    }

    #[test]
    fn every_editor_kind_has_one_identity_across_targets() {
        let material_a = json!({"kind":"resource","resource_kind":"material","id":"a"});
        let material_b = json!({"kind":"resource","resource_kind":"material","id":"b"});
        let texture = json!({"kind":"resource","resource_kind":"texture","id":"a"});
        assert_eq!(
            identity(EditorKind::MaterialEditor, &material_a),
            identity(EditorKind::MaterialEditor, &material_b)
        );
        assert_ne!(
            identity(EditorKind::MaterialEditor, &material_a),
            identity(EditorKind::TextureEditor, &texture)
        );
        let array_a = json!({"kind":"parameter","id":"array:target:A"});
        let array_b = json!({"kind":"parameter","id":"array:target:B"});
        assert_eq!(
            identity(EditorKind::ArrayEditor, &array_a),
            identity(EditorKind::ArrayEditor, &array_b)
        );
    }

    #[test]
    fn later_target_and_project_responses_make_earlier_requests_stale() {
        let entry = json!({"kind":"resource","resource_kind":"material","id":"a"});
        let identity = identity(EditorKind::MaterialEditor, &entry);
        let project_a = json!({"project_id":"p","activation":"a","revision":"1"});
        let project_b = json!({"project_id":"p","activation":"a","revision":"2"});
        let mut latest = std::collections::HashMap::new();
        latest.insert(identity.clone(), 10);
        assert!(!response_is_current(
            &latest, &identity, 9, &project_a, &project_a
        ));
        assert!(response_is_current(
            &latest, &identity, 10, &project_a, &project_a
        ));
        assert!(!response_is_current(
            &latest, &identity, 10, &project_a, &project_b
        ));
    }

    #[test]
    fn a_closed_editor_stays_hidden_when_its_pending_target_resolves() {
        assert!(should_show_editor_on_response(false, false));
        assert!(should_show_editor_on_response(true, true));
        assert!(!should_show_editor_on_response(true, false));
    }

    #[test]
    fn project_reactivation_changes_array_editor_scope_even_for_the_same_project_id() {
        let opened = json!({"project_id":"p","activation":"a1","revision":"1"});
        let edited = json!({"project_id":"p","activation":"a1","revision":"2"});
        let reopened = json!({"project_id":"p","activation":"a2","revision":"1"});
        assert_eq!(project_scope(&opened), project_scope(&edited));
        assert_ne!(project_scope(&opened), project_scope(&reopened));
    }

    #[test]
    fn editor_tabs_are_not_saved_in_the_layout() {
        let mut state = json!({"version":4,"center":{
        "panel_name":"StackPanel","info":{"stack":{"sizes":[1],"axis":0}},"children":[
            {"panel_name":"TabPanel","info":{"tabs":{"active_index":1}},"children":[
                {"panel_name":"Scene","info":{"panel":null},"children":[]},
                {"panel_name":"ResourceEditor","info":{"panel":null},"children":[]}
            ]}
        ]}});
        strip_editors(&mut state);
        let tabs = &state["center"]["children"][0];
        assert_eq!(tabs["children"].as_array().unwrap().len(), 1);
        assert_eq!(tabs["children"][0]["panel_name"], "Scene");
        assert_eq!(tabs["info"]["tabs"]["active_index"], 0);
    }
}
