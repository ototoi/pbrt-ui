//! Serial display adapter. Parsing, package I/O and GPU readback stay off GPUI.
use app_core::CoreHandle;
use protocol::{Method, Request};
use scene_view::{PreviewFrame, PreviewMode, PreviewRenderer, ViewerCamera};
use serde_json::{Value, json};
use std::{
    collections::{BTreeMap, BTreeSet, HashMap, VecDeque},
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

/// A preview belongs to both a project revision and the active scene within it.
pub(super) fn project_preview_key(project: &Value, active_scene: &Value) -> String {
    json!([project, active_scene]).to_string()
}

/// The revision a preview key was built from: the key
/// embeds the whole project reference as its first element, so the
/// installed preview's provenance revision is recoverable without extra
/// bookkeeping. `None` for keys that are not that shape.
pub(super) fn preview_key_revision(key: &str) -> Option<String> {
    serde_json::from_str::<Value>(key)
        .ok()?
        .get(0)?
        .get("revision")?
        .as_str()
        .map(str::to_owned)
}

/// What a preview asks for: Inspector and node editor previews.
/// textures, materials) and Contents thumbnails,
/// all through the same wire id fields.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum PreviewKind {
    Texture,
    Material,
    ImageAsset,
}
impl PreviewKind {
    pub(crate) fn method(self) -> Method {
        match self {
            Self::Texture => Method::ProjectPreviewTexture,
            Self::Material => Method::ProjectPreviewMaterial,
            Self::ImageAsset => Method::ProjectPreviewAsset,
        }
    }
    /// The id field of both the request params and the reply.
    pub(crate) fn id_field(self) -> &'static str {
        match self {
            Self::ImageAsset => "asset_id",
            Self::Texture | Self::Material => "resource_id",
        }
    }
    /// The kind a Contents entry previews as, if any: textures and
    /// materials among resources, image files among assets. Everything
    /// else — folders, scenes, unregistered files, PLY and other
    /// non-image assets — stays an icon.
    pub(crate) fn of_entry(entry: &Value) -> Option<Self> {
        match entry["kind"].as_str() {
            Some("resource") => match entry["resource_kind"].as_str() {
                Some("texture") => Some(Self::Texture),
                Some("material") => Some(Self::Material),
                _ => None,
            },
            Some("asset") => entry["media_type"]
                .as_str()
                .filter(|media_type| {
                    protocol::project::IMAGE_PREVIEW_MEDIA_TYPES.contains(media_type)
                })
                .map(|_| Self::ImageAsset),
            _ => None,
        }
    }
}

pub enum Command {
    Settings(protocol::jobs::RenderSettings, protocol::Revision),
    ResetLayout,
    Render(bool),
    CancelRender,
    /// Jobs panel: cancel one job by id.
    CancelJob(String),
    Export(protocol::ArtifactId, PathBuf),
    Mode(PreviewMode),
    Time(f64),
    FrameAll,
    FrameSelection,
    ViewSceneCamera,
    /// The Scene viewport's size in pixels.
    Resize([u32; 2]),
    Orbit(f64, f64),
    Pan(f64, f64),
    Dolly(f64),
    /// While a gizmo drags, draw the selection moved by this world
    /// matrix; `None` puts it back.
    GizmoPreview(Option<[f64; 16]>),
    Pick {
        frame_id: u64,
        xy: [f64; 2],
    },
    Project(Method, Value),
    /// Re-import the Viewer's source and keep the current camera.
    ReloadViewer(String),
    ProjectSelect {
        project: Value,
        node_id: String,
    },
    /// The node editor selected a value node, so the Inspector shows the
    /// value's own page: a scene node selected before — a Hierarchy or a
    /// viewport pick — retires, so the next revision's per-selection
    /// refresh emits nothing and the Inspector stays on the value page.
    /// Emits no update of its own.
    ProjectDeselect,
    ProjectExpand {
        project: Value,
        node_id: String,
        offset: u64,
    },
    ProjectSearch {
        project: Value,
        query: String,
        offset: usize,
        limit: usize,
    },
    ProjectSearchContents {
        project: Value,
        query: String,
        generation: u64,
        offset: usize,
        limit: usize,
    },
    ProjectContents {
        project: Value,
        folder: String,
        offset: usize,
        limit: usize,
    },
    /// Look up an asset's file for an editor tab.
    OpenEditor {
        project: Value,
        entry: Value,
        request_id: u64,
    },
    /// The static creation menu summary from `project.get_parameter_schema`
    /// Which implementations exist per kind and texture value
    /// type. Fetched once at startup — the table describes r4, not a project.
    GetResourceSchemas,
    /// Which of the items open in editor tabs no longer exist.
    CheckItems {
        project: Value,
        ids: Vec<String>,
    },
    /// Show a Contents entry in the Inspector.
    InspectContents {
        project: Value,
        entry: Value,
    },
    /// A node editor's upstream graph.
    ResourceGraph {
        project: Value,
        resource_id: String,
    },
    /// A texture's or material's preview PNG, queued
    /// so the display thread never waits for it; the reply arrives as
    /// `Update::Preview`.
    Preview {
        project: Value,
        id: String,
        kind: PreviewKind,
    },
    /// Replace the one queued Contents thumbnail candidate.
    ThumbnailsVisible {
        project: Value,
        generation: u64,
        candidates: Vec<(String, PreviewKind)>,
    },
    /// Release the single preview/decode slot after the UI has accepted or rejected a reply.
    ThumbnailDone(String),
    /// The array values review: one window of a parameter's
    /// values, for an open Inspector
    /// array row's table.
    GetParameterValues {
        project: Value,
        target: Value,
        name: String,
        offset: usize,
        limit: usize,
        /// The array row's form id, to route the answer.
        form: String,
        /// The group page the window starts at.
        group_page: usize,
    },
    /// One SpectrumData resource's whole sample tables, fetched in
    /// windows on this thread, answered whole as `Update::SpectrumTable`.
    GetSpectrumTable {
        project: Value,
        resource_id: String,
    },
    /// A SpectrumData resource's approximate colour for one role
    /// as `Update::SpectrumSwatch`.
    SpectrumSwatch {
        project: Value,
        resource_id: String,
        role: String,
    },
    /// One parameter row's approximate colour
    /// for the row's role — None lets the
    /// core derive it from the schema — as
    /// `Update::ParameterSpectrumColour`, the
    /// Inspector colour box's read, asked at most once per visible row
    /// per revision.
    ParameterSpectrumColour {
        project: Value,
        target: Value,
        parameter: String,
        /// The role the row asks for (`albedo`, `chromaticity`,
        /// `conductor`…), None to keep the core's schema-derived choice.
        role: Option<String>,
    },
    /// Where one item is used
    ///.
    ListReferences {
        project: Value,
        item: (String, String),
    },
    /// One graph node's Inspector-style details.
    ResourceDetails {
        project: Value,
        resource_id: String,
    },
    /// `project.plan_implementation` for an Inspector Implementation
    /// row; the result comes back as `Update::ImplementationPlan`.
    PlanImplementation {
        project: Value,
        target: Value,
        implementation: String,
        /// The Inspector row that asked, for `Update::EditSettled`.
        form: String,
    },
    /// One `project.apply`, then what the window wants done with its result.
    Apply {
        project: Value,
        edits: Vec<Value>,
        follow: Follow,
        /// The Inspector field that sent it, for `Update::EditSettled`.
        form: Option<String>,
    },
    /// Read the active Film and camera parameters, then align pose and fov
    /// with one project.apply transaction.
    AlignCamera {
        project: Value,
        scene_id: String,
        camera_id: String,
        coordinate_system: project::CoordinateSystem,
        view: project::presets::PresetView,
        view_fov_y_degrees: f64,
        viewport_aspect: f64,
    },
    /// A job sent from a form. Its acceptance or
    /// refusal comes back as `Update::EditSettled`; the job itself is
    /// followed from the job list.
    Submit {
        method: Method,
        params: Value,
        form: String,
    },
    /// Jobs that must run one after another: each step is sent with the
    /// project reference current at that moment, after the previous job
    /// published (two writes against one revision would make the second
    /// stale). One acceptance or refusal comes back for the whole form.
    SubmitSequence {
        method: Method,
        steps: Vec<Value>,
        form: String,
    },
    Stop,
}
impl Command {
    /// The form a command answers to, if any.
    pub fn form(&self) -> Option<&str> {
        match self {
            Self::Submit { form, .. }
            | Self::SubmitSequence { form, .. }
            | Self::PlanImplementation { form, .. } => Some(form),
            Self::Apply { form, .. } => form.as_deref(),
            _ => None,
        }
    }
}
/// What follows a successful `Command::Apply`.
#[derive(Clone, Debug, PartialEq)]
pub enum Follow {
    Nothing,
    /// New / Duplicate Scene: open the created scene.
    OpenScene,
    /// Create menu: select the created object and reveal it under `parent`.
    Reveal {
        parent: String,
    },
    /// Unpack: select the edit's first created node (the plain group that
    /// replaced the instance) and reveal it under `parent`.
    RevealFirstCreated {
        parent: String,
    },
    /// Contents ▸ New: select the created resource and enter
    /// rename. `folder` is where the edit aimed the creation.
    SelectResource {
        folder: String,
    },
    /// A node editor background create uses the same
    /// selection, but the editor that asked takes the created id and
    /// places the node where the click landed, so no rename prompt
    /// follows.
    SelectResourceInEditor {
        folder: String,
        /// The session's key for the asking editor and click.
        ask: u64,
    },
}
pub enum Update {
    LayoutReset,
    /// `app.quit` was accepted; the window closes without a dialog.
    Quit,
    Jobs(Value),
    ForegroundJobs(Value),
    Settings(Value),
    Ipr(Value),
    RenderImage {
        project: Value,
        artifact_id: Option<protocol::ArtifactId>,
        width: u32,
        height: u32,
        rgba: Vec<u8>,
        /// The received-pixel fraction when the image is a running
        /// render's partial snapshot, keyed by the job that published it;
        /// the status line shows it only while that job still runs.
        received: Option<(String, f64)>,
    },
    Mode(PreviewMode),
    Time(f64),
    Log(String),
    Status(String),
    RenderRequestFailed,
    Project(protocol::project::ProjectState),
    RenderCamera(Option<String>),
    /// Lazy hierarchy: children of one node, one page at a time.
    ProjectChildren {
        parent: String,
        offset: u64,
        nodes: Vec<Value>,
        next_offset: Option<u64>,
    },
    ProjectTree(Value),
    /// Label search over the whole project; `page` is the core response.
    ProjectSearch {
        query: String,
        offset: usize,
        page: Value,
    },
    ProjectSearchContents {
        project: Value,
        query: String,
        generation: u64,
        offset: usize,
        page: Value,
        error: Option<String>,
    },
    /// One Contents folder page. Null marks a failed request.
    ProjectContents {
        project: Value,
        folder: String,
        offset: usize,
        page: Value,
    },
    /// `project.list_scenes` for the current revision.
    ProjectScenes(Value),
    /// `project.list_presets` for the current activation.
    ProjectPresets(Value),
    /// A created object to select and reveal: (object, its wrapper group,
    /// the parent it was added under).
    Created {
        node: String,
        wrapper: Option<String>,
        parent: String,
    },
    /// A node picked in the Scene viewport, to reveal in the Hierarchy:
    ///.
    RevealPicked(String, Vec<String>),
    /// The `GetResourceSchemas` summary: implementable rows per kind and
    /// texture value type.
    ResourceSchemas(Value),
    /// A created resource to select in Contents. `project`
    /// is the revision that holds it: this arrives before the state sync's
    /// `Update::Project`, so the window's own reference is still the one
    /// before the creation. `editor_ask` is None for the Contents ▸ New
    /// flow; a node-editor ask names its key so
    /// the id reaches the editor that asked instead.
    CreatedResource {
        project: Value,
        entry: Value,
        editor_ask: Option<u64>,
    },
    /// A node-editor creation whose apply failed: its ask settles.
    EditorCreateFailed(u64),
    ProjectInspector(Value),
    /// The backend's scene node selection is retired. It follows any
    /// `ProjectInspector` answer still queued from before the deselect,
    /// so a node that answer restored is cleared again.
    ProjectDeselected,
    /// In-place save finished; the window runs any pending action on success.
    SaveSettled {
        ok: bool,
        error: Option<String>,
    },
    /// An editor tab can open: the entry and its file on disk.
    EditorReady {
        project: Value,
        request_id: u64,
        entry: Value,
        path: Result<Option<String>, protocol::ErrorPayload>,
    },
    /// Items open in editor tabs that the current revision no longer has.
    ItemsMissing {
        project: Value,
        ids: Vec<String>,
    },
    /// Where one item is used (the response of
    /// project.list_references), for `item`'s Inspector view.
    ListReferences {
        project: Value,
        item: (String, String),
        list: Value,
    },
    /// A texture's or material's preview file — its cache path and the
    /// content key it was written under — or why there is none, read at
    /// `project`.
    Preview {
        project: Value,
        id: String,
        result: Result<(String, String), protocol::ErrorPayload>,
    },
    /// A Contents thumbnail's cache file for textures, materials and
    /// image assets), or why there is none, read at `project`.
    Thumbnail {
        project: Value,
        id: String,
        kind: PreviewKind,
        generation: u64,
        request_id: String,
        result: Result<(String, String), protocol::ErrorPayload>,
    },
    /// One window of one parameter's values for the Inspector array row
    /// `form`, read at `project`; a failed read shows as `{error}`.
    ParameterValues {
        project: Value,
        form: String,
        group_page: usize,
        page: Result<Value, String>,
    },
    /// A SpectrumData resource's whole sample tables
    /// Read at the project revision.
    /// `project`.
    SpectrumTable {
        project: Value,
        resource_id: String,
        table: Result<Value, String>,
    },
    /// A SpectrumData resource's approximate colour for one role
    /// Read at the project revision.
    SpectrumSwatch {
        project: Value,
        resource_id: String,
        role: String,
        answer: Result<Value, String>,
    },
    /// One parameter row's approximate colour (`{rgb, source,
    /// role}`, or `{error}`), read at `project` for the role the row
    /// asked — the answer the Inspector colour box caches per visible
    /// row per revision, keyed by `parameter#role`.
    ParameterSpectrumColour {
        project: Value,
        target: Value,
        parameter: String,
        role: Option<String>,
        answer: Result<Value, String>,
    },
    /// `project.get_resource_graph` for a node editor.
    ResourceGraph {
        project: Value,
        resource_id: String,
        graph: Value,
    },
    /// A graph node's details, shaped like `project_item_details`, read at
    /// `project`.
    ResourceDetails {
        project: Value,
        resource_id: String,
        details: Value,
    },
    /// A `Command::PlanImplementation` result: the window confirms (when
    /// parameters are removed or unknown) and applies `plan["edits"]`.
    ImplementationPlan {
        project: Value,
        form: String,
        plan: Value,
    },
    /// The outcome of an Inspector field's edit.
    EditSettled {
        form: String,
        error: Option<String>,
    },
    /// A request without `discard_unsaved` hit the core's guard.
    UnsavedChanges,
    /// Import diagnostics are warnings only; the status bar counts them.
    ImportWarnings(usize),
    PreviewUnavailable(Option<String>),
    ProjectPreview {
        key: String,
        times: Vec<f64>,
        time: f64,
    },
}
/// What the window opens at startup.
pub enum Startup {
    Scene(PathBuf),
    Project(PathBuf),
}
pub struct Backend {
    pub commands: mpsc::SyncSender<Command>,
    pub updates: mpsc::Receiver<Update>,
    latest_frame: Arc<Mutex<Option<FrameUpdate>>>,
    thumbnail_mailbox: Arc<Mutex<ThumbnailMailbox>>,
    thread: Option<thread::JoinHandle<()>>,
}
pub struct FrameUpdate {
    scene_key: String,
    pub id: u64,
    pub frame: PreviewFrame,
    /// The camera and size the frame was drawn with, for the gizmo.
    pub camera: ViewerCamera,
}
impl Backend {
    pub fn start(core: CoreHandle, initial: Option<Startup>) -> Self {
        let (commands, rx) = mpsc::sync_channel(64);
        let (tx, updates) = mpsc::channel();
        let latest_frame = Arc::new(Mutex::new(None));
        let thumbnail_mailbox = Arc::new(Mutex::new(ThumbnailMailbox::default()));
        let frames = latest_frame.clone();
        let mailbox = thumbnail_mailbox.clone();
        let thread = thread::spawn(move || run(core, rx, tx, frames, mailbox, initial));
        Self {
            commands,
            updates,
            latest_frame,
            thumbnail_mailbox,
            thread: Some(thread),
        }
    }

    /// Keep just the latest completed frame; never accumulate full RGBA images
    /// when GPUI is slower than the renderer. Wait for its Scene update first.
    pub fn take_frame(&self, scene_key: &str) -> Option<FrameUpdate> {
        let mut frame = self.latest_frame.lock().ok()?;
        if frame.as_ref()?.scene_key != scene_key {
            return None;
        }
        frame.take()
    }

    /// Publish the latest visible Contents set without consuming the bounded
    /// display command queue. The display thread checks it immediately before
    /// submitting an unstarted preview.
    pub fn update_thumbnail_visibility(
        &self,
        project: Value,
        generation: u64,
        visible: Vec<(String, PreviewKind)>,
    ) {
        if let Ok(mut mailbox) = self.thumbnail_mailbox.lock() {
            mailbox.project = project;
            mailbox.generation = generation;
            mailbox.visible = visible.into_iter().take(256).collect();
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

mod camera_preservation;
mod cleanup;
mod commands;
mod core_client;
mod display;
mod frame;
mod gpu;
mod ipr;
mod job_log;
mod preview_sweep;
pub(crate) mod preview_timing;
mod previews;
mod project_preview;
mod results;
mod state_sync;
mod thumbnails;
use core_client::*;
use display::run;

/// One retired preview job. `queued_request` re-issues the replacement that
/// this job was cancelled for, once the job goes terminal — the replacement
/// must wait, because a job past its last cancellation checkpoint can still
/// publish into the shared slot.
struct PreviewCleanup {
    job_id: String,
    directory: String,
    /// of the superseding preview job, issued only
    /// after cleanup confirmed the slot is free.
    queued_request: Option<(Value, String)>,
}

struct ThumbnailFlight {
    request_id: String,
    project: Value,
    id: String,
    kind: PreviewKind,
    generation: u64,
    receiver: Option<mpsc::Receiver<protocol::Response>>,
}

/// One queued Inspector/node-editor preview in flight. Unlike thumbnails
/// there is no display-side ack: the Update that carries the reply also
/// clears the slot, and the session decodes on its own background
/// executor.
struct PreviewFlight {
    request_id: String,
    project: Value,
    id: String,
    kind: PreviewKind,
    receiver: Option<mpsc::Receiver<protocol::Response>>,
}

#[derive(Default)]
pub(super) struct ThumbnailMailbox {
    pub(super) visible: Vec<(String, PreviewKind)>,
    pub(super) project: Value,
    pub(super) generation: u64,
}

/// Mutable display state has one owner on the serial display thread.
struct Display {
    core: CoreHandle,
    tx: mpsc::Sender<Update>,
    frames: Arc<Mutex<Option<FrameUpdate>>>,
    thumbnail_mailbox: Arc<Mutex<ThumbnailMailbox>>,
    stopping: bool,
    gpu: gpu::GpuHandle,
    gpu_rx: mpsc::Receiver<gpu::GpuResult>,
    /// A render request is with the GPU worker; the next dirty flag only
    /// reissues after its result arrives. `Command::Pick` gates on this.
    render_in_flight: bool,
    /// One install runs on the worker; a newer package waits in the slot.
    install_in_flight: bool,
    /// Latest-wins install slot: plus the
    /// optional manifest override that replaces the slot's
    /// `geometry.json` for install — carried by the fast path's light
    /// move.
    queued_install: Option<(
        String,
        String,
        PathBuf,
        Option<Box<protocol::geometry::GeometryManifest>>,
    )>,
    preview_ready: bool,
    camera: ViewerCamera,
    scene_camera: Option<ViewerCamera>,
    /// The wire camera this display last wrote to, or adopted from, the shared
    /// view. A shared camera that differs from it was set by another client.
    synced_camera: Option<Value>,
    /// When the idle, read-only look for an external camera runs next.
    view_watch_due: Option<Instant>,
    mode: PreviewMode,
    size: [u32; 2],
    dirty: bool,
    frame_id: u64,
    last_frame: Option<PreviewFrame>,
    last_state: Value,
    last_jobs: Value,
    /// First sight of each unfinished job, for the elapsed time in the
    /// Console's job-end lines; the core reports no timestamps.
    job_ages: HashMap<String, Instant>,
    last_artifact: Option<Value>,
    last_settings: Value,
    last_project: Value,
    selected_project_node: Option<String>,
    /// `project_items` for one project revision.
    project_items: Option<(Value, Value)>,
    project_preview_key: Option<String>,
    project_camera_owner: Option<String>,
    project_preview_job: Option<(String, String)>,
    /// Import/open jobs whose reports still owe the Console a poll. A
    /// second import while one runs must keep both: each settled job
    /// delivers its own diagnostics and status.
    import_jobs: Vec<String>,
    /// User initiated operations followed by exact ID, independent of project activation.
    foreground_jobs: Vec<(String, String, u8)>,
    foreground_preview_pending: bool,
    last_foreground_jobs: Value,
    /// In-place save job whose outcome the window waits for.
    save_job: Option<String>,
    /// Export jobs whose warnings still owe the Console a poll.
    export_jobs: Vec<String>,
    /// A Viewer reload's camera intent, bounded to its exact result activation.
    preserve_camera: Option<camera_preservation::CameraPreservation>,
    /// Retired preview jobs. A queued replacement rides along until the
    /// retired job goes terminal, so a cancelled publisher can never keep
    /// the replacement's slot occupied.
    project_preview_cleanup: Vec<PreviewCleanup>,

    /// This process's preview-slot owner lock, taken at startup and
    /// held until the display thread
    /// ends; the startup sweep clears orphaned slots of dead desktops.
    /// Never read: the field's purpose is the guard's lifetime.
    #[allow(dead_code)]
    preview_slot_owner: Option<preview_sweep::PreviewSlotOwner>,

    project_geometry: Option<protocol::geometry::GeometryManifest>,
    /// The transform fast path's base — the untouched
    /// draws of the installed preview, and the revision that install was
    /// keyed to. A fast answer always applies to THIS copy, never to
    /// already-updated draws, so several moves in a row stay exact. Every
    /// install replaces the pair wholesale; absent when the installed key
    /// names no revision.
    project_preview_base: Option<crate::project_preview::TransformBase>,
    project_pick_nodes: BTreeMap<u32, String>,
    project_tree_snapshot: Value,
    project_object_bounds: BTreeMap<u64, protocol::Bounds>,
    project_time: f64,
    /// Camera change queued a preview submit; fires once after the move settles.
    ipr_due: Option<Instant>,
    /// Debounced push of the live GUI view onto the project's shared native
    /// view so external captures see what the desktop shows.
    view_due: Option<Instant>,
    /// Pick ids of the selected node and its descendants, outlined in the
    /// preview, and the they were computed for.
    highlight: BTreeSet<u32>,
    highlight_key: Option<(Option<String>, Option<String>)>,
    /// Lit notes already sent to the Console, so re-uploads after edits do
    /// not repeat them.
    logged_lit_notes: Vec<String>,
    /// Preview package warnings already sent to the Console, so the
    /// preview that runs after every edit does not repeat them.
    logged_preview_warnings: Vec<String>,
    thumbnail_project: Value,
    thumbnail_generation: u64,
    thumbnail_candidates: VecDeque<(String, PreviewKind)>,
    thumbnail_flight: Option<ThumbnailFlight>,
    /// Inspector and node editor preview requests, one in flight and
    /// the rest queued.
    preview_queue: VecDeque<(Value, String, PreviewKind)>,
    preview_flight: Option<PreviewFlight>,
    /// The dragged gizmo's world delta on the selection's draws. It
    /// stays after a commit until the edited preview replaces the scene.
    gizmo_delta: Option<[f64; 16]>,
}
#[cfg(test)]
mod tests;
