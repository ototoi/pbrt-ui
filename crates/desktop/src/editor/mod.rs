//! Editor-owned UI state and views. Runtime/process setup lives in application.
mod array_editor;
mod color_dialog;
mod colour_wheel;
mod component_menu;
mod components;
mod contents_thumbnails;
pub(crate) mod contents_tree;
mod create_menu;
pub(crate) use create_menu::align_camera_edits_for_type;
pub(crate) mod dialogs;
pub(crate) mod document;
mod edit_feedback;
mod gizmo;
pub(crate) use gizmo::GIZMO_FORM;
mod editor_panels;
mod graph_layout;
mod graph_selection;
mod hierarchy;
mod image_display;
mod implementation_menu;
mod jobs;
mod layout;
/// The selected light's wire-frame target, from the same Inspector
/// details the transform gizmo reads.
pub(crate) mod light_gizmo;
mod menu;
mod mesh_viewer;
mod node_editor;
mod node_editor_create;
mod node_edits;
mod panels;
mod parameter_field;
mod parameter_form;
pub(crate) mod parameter_rows;
mod parameter_values;
mod path_picker;
mod project_actions;
mod project_contents_search;
mod prompt;
mod recent;
mod render_settings;
mod render_start;
mod resource_editor;
mod resource_menu;
mod session;
mod spectrum_editor;
mod start_screen;
mod status;
mod toolbar;
mod transform_form;
mod workspace;
use crate::backend::Backend;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::{dock::DockArea, slider::SliderState};
use gpui_kit::*;
use protocol::project::ProjectState;
use scene_view::PreviewMode;
use std::{cell::Cell, path::PathBuf, rc::Rc, sync::Arc};
actions!(
    pbrt_editor,
    [
        FrameSelected,
        FrameAll,
        CancelGizmo,
        MoveTool,
        RotateTool,
        ScaleTool,
        NoTool,
        ToggleGizmoSpace
    ]
);

struct Session {
    layout_revision: u64,
    jobs: serde_json::Value,
    foreground_jobs: serde_json::Value,
    foreground_seen: std::collections::HashMap<String, std::time::Instant>,
    foreground_visible: bool,
    settings: serde_json::Value,
    ipr: serde_json::Value,
    render_image: Option<Arc<RenderImage>>,
    render_artifact: Option<protocol::ArtifactId>,
    render_camera_label: Option<String>,
    render_project: serde_json::Value,
    render_mean_luminance: Option<f64>,
    /// How much of the shown image a running render has published,
    /// keyed by the job id that produced the partial snapshot;
    /// None for the final image or an artifact without the field.
    render_received: Option<(String, f64)>,
    render_start: render_start::RenderStart,
    backend: Backend,
    scene_key: String,
    messages: Vec<String>,
    status: String,
    image: Option<Arc<RenderImage>>,
    frame_id: u64,
    image_size: [u32; 2],
    /// The camera the shown frame was drawn with.
    view_camera: Option<scene_view::ViewerCamera>,
    updating: bool,
    /// The retained image belongs to the previous revision and is display-only.
    stale_frame: bool,
    preview_error: Option<String>,
    /// A gizmo preview reset the full queue refused; retried until sent so
    /// an uncommitted move never stays on screen.
    gizmo_reset: bool,
    preview_mode: PreviewMode,
    time_slider: Entity<SliderState>,
    supported_times: Vec<f64>,
    view_time: f64,
    project: ProjectState,
    project_tree: serde_json::Value,
    /// project_id / activation that `project_tree` was loaded for.
    tree_scope: String,
    project_search_query: String,
    project_search: serde_json::Value,
    contents_search_query: String,
    contents_search: serde_json::Value,
    contents_search_project: serde_json::Value,
    contents_search_generation: u64,
    contents_search_error: Option<String>,
    contents_search_in_flight: Option<(u64, usize)>,
    contents_search_settled: Option<(u64, usize)>,
    /// Contents folder tree for the current project revision.
    contents: contents_tree::ContentsTree,
    contents_thumbnails: contents_thumbnails::ContentsThumbnails,
    thumbnail_images:
        std::collections::HashMap<contents_thumbnails::ThumbnailId, (String, Arc<RenderImage>)>,
    pending_thumbnail_drops: Vec<Arc<RenderImage>>,
    thumbnail_slot_active: bool,
    thumbnail_done_pending: Option<String>,
    thumbnail_visible_at: Option<std::time::Instant>,
    /// `project.list_scenes` for the current revision.
    project_scenes: serde_json::Value,
    /// `project.list_presets` for the current activation.
    project_presets: serde_json::Value,
    /// The Contents ▸ New menu rows, from r4's static
    /// parameter table: material / texture per value type / medium.
    resource_schema: serde_json::Value,
    /// Node editor graphs and node details by resource id, as
    /// the backend last answered them, with the project they were read at.
    resource_graphs: std::collections::HashMap<String, (serde_json::Value, serde_json::Value)>,
    resource_details: std::collections::HashMap<String, (serde_json::Value, serde_json::Value)>,
    /// Node positions by editor root; a folder project keeps them in
    /// `.pbrtui/node-positions.json`.
    node_positions: graph_layout::PositionStore,
    /// The position file `node_positions` was loaded from, if any.
    node_positions_file: Option<std::path::PathBuf>,
    /// Counts position loads so open node editors pick up a new layout.
    node_positions_loaded: u64,
    /// Texture and material previews by resource id;
    /// adds image asset previews by asset id (both UUIDs, so the key
    /// spaces cannot collide).
    previews: std::collections::HashMap<String, session::Preview>,
    /// Current page of each dedicated array editor, keyed by parameter.
    values_page: std::collections::HashMap<String, usize>,
    /// `project.get_parameter_values` pages per open array row.
    values_pages: std::collections::HashMap<String, session::ValuesPages>,
    /// The SpectrumData editor's per-resource answers — the whole
    /// sample table and a swatch per role — each asked once per project.
    spectrum_answers: std::collections::HashMap<String, session::SpectrumAnswers>,
    /// Each parameter row's approximate colour by target id and
    /// parameter name (`SpectrumAnswers` again, its swatch slots keyed by
    /// parameter), asked once per visible row per revision.
    parameter_answers: std::collections::HashMap<String, session::SpectrumAnswers>,
    /// Where-used lists per item, read at the
    /// project they were answered at.
    item_references: std::collections::HashMap<String, (serde_json::Value, serde_json::Value)>,
    /// Parents whose child page arrived since the Hierarchy last synced; it
    /// clears their in-flight refresh requests.
    children_arrived: Vec<String>,
    /// A Create-menu object waiting to be revealed in the Hierarchy:
    ///.
    reveal_created: Option<(String, Option<String>, String)>,
    /// A node picked in the Scene viewport waiting to be revealed in the
    /// Hierarchy.
    reveal_picked: Option<(String, Vec<String>)>,
    /// A where-used row's node waiting to be revealed in the Hierarchy.
    /// The
    /// reveal fires once the Hierarchy's tree is that scene's — right
    /// away when the row's scene is already active, after the switch
    /// when the row opened another scene.
    reveal_where_used: Option<(String, String, Vec<String>)>,
    selected_project_node: Option<String>,
    project_inspector: serde_json::Value,
    /// The node editor's selected value node the Inspector follows
    /// — the inline
    /// colour page the Inspector shows until any other selection
    /// moves it.
    graph_value: Option<(String, String, String)>,
    /// The node editors whose background creates are in
    /// flight, by the key their apply's follow carries: the answer or
    /// failure removes exactly its own ask. Moving to another document
    /// clears it.
    node_editor_creates: std::collections::HashMap<u64, node_editor::CreateAsk>,
    node_editor_create_key: u64,
    /// The Contents item whose `InspectContents` is still unanswered: a
    /// revision refresh re-asks it rather than the page it replaces.
    inspect_asked: Option<serde_json::Value>,
    /// Outcome of the last in-place save, consumed by the window.
    save_settled: Option<(bool, Option<String>)>,
    /// Set while an in-place save job is running.
    save_in_flight: bool,
    /// The core refused a request because the project has unsaved changes.
    unsaved_changes: bool,
    /// Warning count from the last import report; reset per activation.
    import_warnings: usize,
    /// Folded Inspector component kinds and where they are kept.
    folded: components::Folded,
    folded_path: Option<PathBuf>,
    /// Show transforms as a 4×4 matrix instead of TRS. Display only.
    transform_matrix: bool,
    /// Messages under Inspector fields: rejections and overwritten typing.
    edit_feedback: edit_feedback::EditFeedback,
    /// The colour dialogs the user has open, by their
    /// parameter form id — the bookkeeping a rebuilt field's popover
    /// re-opens from.
    colour_dialogs: color_dialog::OpenDialogs,
    /// Editor tabs ready to open, and items whose tabs must close; the
    /// window consumes both.
    editors_ready: Vec<(
        serde_json::Value,
        u64,
        serde_json::Value,
        Result<Option<String>, protocol::ErrorPayload>,
    )>,
    items_missing: Vec<(serde_json::Value, Vec<String>)>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum PanelKind {
    Hierarchy,
    Scene,
    Inspector,
    Contents,
    Console,
    Render,
    Jobs,
}
impl PanelKind {
    /// Every panel, in the order `Workspace.panels` holds them.
    const ALL: [PanelKind; 7] = [
        Self::Hierarchy,
        Self::Scene,
        Self::Render,
        Self::Inspector,
        Self::Contents,
        Self::Console,
        Self::Jobs,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Hierarchy => "Hierarchy",
            Self::Scene => "Scene",
            Self::Inspector => "Inspector",
            Self::Contents => "Contents",
            Self::Console => "Console",
            Self::Render => "Render",
            Self::Jobs => "Jobs",
        }
    }
}
struct EditorPanel {
    settings_form: Option<Entity<render_settings::SettingsForm>>,
    /// The inspected node's local-transform form.
    transform_forms: Vec<(String, String, Entity<transform_form::TransformForm>)>,
    /// Read-only side-by-side endpoint fields for the selected AnimatedTransform.
    animated_transform_form: Option<(String, Entity<transform_form::AnimatedTransformForm>)>,
    ///.
    parameter_forms: Vec<(String, String, Entity<parameter_field::ParameterField>)>,
    /// The Inspector header's label field.
    label_input: Option<(
        String,
        String,
        Entity<gpui_kit::component::input::InputState>,
    )>,
    /// The node editor's selected value node: the inline colour editor
    /// of its owner's parameter, keyed by the state it was built from.
    /// The value page's mounted inline colour editor: the owner and
    /// parameter it edits name it — never the revision, and never the
    /// declaration's edition — so the same editor is refreshed, not
    /// rebuilt, across the very edits that bump the project (the
    /// keyboard focus and unsent drafts stay with it).
    value_edit: Option<(String, String, Entity<color_dialog::ColourEdit>)>,
    hierarchy: Option<Entity<hierarchy::Hierarchy>>,
    kind: PanelKind,
    session: Entity<Session>,
    contents_tree_scroll: UniformListScrollHandle,
    contents_list_scroll: UniformListScrollHandle,
    inspector_scroll: ScrollHandle,
    contents_grid_columns: usize,
    contents_search_input: Option<Entity<gpui_kit::component::input::InputState>>,
    contents_search_generation: u64,
    contents_search_more_offset: Option<usize>,
    contents_search_project_key: String,
    focus: FocusHandle,
    viewport: Rc<Cell<Bounds<Pixels>>>,
    /// The Scene preview size last sent to the backend.
    viewport_requested: Cell<[u32; 2]>,
    last_mouse: Option<Point<Pixels>>,
    /// The Scene gizmo's hovered handle, keyed by the node it was
    /// hovered on so a new selection never inherits it, and the
    /// drag in progress.
    gizmo_hover: Option<(String, scene_view::preview::gizmo::Handle)>,
    gizmo_drag: Option<panels::scene::GizmoDrag>,
    /// The selected spot's hovered cone handle (keyed by the node
    /// it was hovered on, so a new selection never inherits it) and the
    /// drag in progress.
    light_hover: Option<(String, scene_view::preview::light_gizmo::SpotHandle)>,
    light_drag: Option<panels::scene::LightDrag>,
    /// The active gizmo tool and its Local / World space.
    gizmo_tool: scene_view::preview::gizmo::Tool,
    gizmo_space: panels::scene::Space,
    /// When the Jobs panel first saw each job, and its final elapsed time.
    job_times: std::collections::HashMap<String, (std::time::Instant, Option<std::time::Duration>)>,
}
struct Workspace {
    layout_revision: u64,
    focus: FocusHandle,
    session: Entity<Session>,
    dock: Entity<DockArea>,
    panels: Vec<Entity<EditorPanel>>,
    layout_save: Option<Task<()>>,
    menu_bar: Entity<AppMenuBar>,
    menu_state: menu::MenuState,
    /// The operation the unsaved-changes dialog is holding back.
    pending: Option<dialogs::PendingAction>,
    /// The last document request, replayed when the core reports unsaved
    /// changes.
    last_attempt: Option<dialogs::PendingAction>,
    /// project_id / activation the window last saw, so `last_attempt` is
    /// dropped once a document switch succeeds.
    scope: String,
    endpoint: String,
    window_title: String,
    /// `recent.json` in the config directory; None when no config directory.
    recent_path: Option<PathBuf>,
    recent: Vec<recent::RecentEntry>,
    /// Mode and path last added to `recent`, so each document is added once.
    recent_key: Option<(recent::RecentKind, PathBuf)>,
    /// Open resource editor tabs.
    editors: Vec<Entity<resource_editor::ResourceEditor>>,
    /// The project revision the editor tabs were last checked against.
    editors_checked: serde_json::Value,
    editor_scope: String,
    editor_revision: serde_json::Value,
    next_editor_request: u64,
    latest_editor_requests: std::collections::HashMap<editor_panels::EditorIdentity, u64>,
}

pub(crate) fn run(
    backend: Backend,
    config: Option<PathBuf>,
    endpoint: String,
    mut on_quit: impl FnMut() + 'static,
) {
    gpui_kit::application()
        .with_assets(crate::editor_assets::EditorAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            menu::bind_keys(cx);
            cx.on_app_quit(move |_| {
                on_quit();
                async {}
            })
            .detach();
            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let options = WindowOptions {
                titlebar: Some(TitlebarOptions {
                    title: Some("pbrt-ui".into()),
                    ..Default::default()
                }),
                window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
                    None,
                    size(px(1440.), px(960.)),
                    cx,
                ))),
                icon: window_icon(),
                // Wayland matches the window to a `pbrt-ui.desktop` entry by this id.
                app_id: Some("pbrt-ui".into()),
                ..Default::default()
            };
            let menu_bar = AppMenuBar::new(cx);
            gpui_kit::open_window(options, cx, move |window, cx| {
                let menu_bar = menu_bar.clone();
                let endpoint = endpoint.clone();
                cx.new(|cx| Workspace::new(backend, config, endpoint, menu_bar, window, cx))
            })
            .expect("failed to open editor window");
            cx.activate(true);
        });
}

/// The window icon (X11 shows it; Wayland and macOS take theirs elsewhere).
fn window_icon() -> Option<Arc<image::RgbaImage>> {
    let png = include_bytes!("../../assets/pbrt-ui.png");
    image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .ok()
        .map(|icon| Arc::new(icon.into_rgba8()))
}

#[cfg(test)]
mod window_icon_tests {
    #[test]
    fn the_embedded_window_icon_decodes() {
        let icon = super::window_icon().expect("the bundled PNG decodes");
        assert_eq!(icon.dimensions(), (256, 256));
    }
}
