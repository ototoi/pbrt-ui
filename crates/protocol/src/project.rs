//! Native-project control DTOs. No project storage, parser or renderer types.
use crate::{Revision, SceneId};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Activation distinguishes reopening the same saved project from an old context.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectRef {
    pub project_id: Uuid,
    pub revision: Revision,
    pub activation: Revision,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectContext {
    pub activation: Revision,
    pub project: Option<ProjectRef>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode {
    Viewer,
    Project,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectState {
    pub context: ProjectContext,
    pub path: Option<String>,
    pub mode: Option<ProjectMode>,
    pub source_path: Option<String>,
    pub read_only: bool,
    pub active_scene: Option<SceneId>,
    /// The active scene's up axis: the viewport frames and
    /// orbits around it.
    #[serde(default)]
    pub active_up: Option<UpAxis>,
    /// The active scene's handedness: a right-handed scene's viewport is
    /// mirrored like its cameras.
    #[serde(default)]
    pub active_handedness: Option<Handedness>,
    pub default_scene: Option<SceneId>,
    pub scene_count: u64,
    pub title: Option<String>,
    pub saved_revision: Option<Revision>,
    pub dirty: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub node_count: u64,
    pub resource_count: u64,
    pub asset_count: u64,
    pub switch_job_id: Option<crate::JobId>,
    /// Whether the shared core viewer/capture APIs expose native geometry.
    pub preview_connected: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportProjectParams {
    pub source_path: String,
    #[serde(default)]
    pub destination: Option<String>,
    pub expected_current: ProjectContext,
    #[serde(default)]
    pub discard_unsaved: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenProjectParams {
    pub path: String,
    pub expected_current: ProjectContext,
    #[serde(default)]
    pub discard_unsaved: bool,
}
/// Which up axis and handedness a new project's scenes start with
///; each field falls back to the +Y/left default, as the
/// CLI flags do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinateSystemChoice {
    #[serde(default)]
    pub up: Option<UpAxis>,
    #[serde(default)]
    pub handedness: Option<Handedness>,
}
impl CoordinateSystemChoice {
    pub fn up(self) -> UpAxis {
        self.up.unwrap_or(UpAxis::PosY)
    }
    pub fn handedness(self) -> Handedness {
        self.handedness.unwrap_or(Handedness::Left)
    }
}
/// Create a new editable folder project: the destination must
/// not exist or be an empty directory; it opens like `project.open` with
/// an initial scene (render Camera, Floor, Environment Light).
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateProjectParams {
    pub path: String,
    pub title: String,
    #[serde(default)]
    pub coordinate_system: Option<CoordinateSystemChoice>,
    pub expected_current: ProjectContext,
    #[serde(default)]
    pub discard_unsaved: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CloseProjectParams {
    pub expected_current: ProjectContext,
    #[serde(default)]
    pub discard_unsaved: bool,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectParams {
    pub project: ProjectRef,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SaveProjectParams {
    pub project: ProjectRef,
    #[serde(default)]
    pub destination: Option<String>,
}
/// Read the shared native view of the active project. `view` is null until a
/// client establishes one via `project.set_camera`/`project.configure_view`;
/// while null, captures frame the exported manifest's scene camera.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GetProjectViewParams {
    pub project: ProjectRef,
}
/// Establish or update the shared native view's camera under revision control.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetProjectCameraParams {
    pub project: ProjectRef,
    pub expected_view_revision: Revision,
    pub camera: crate::view::Camera,
}
/// Adjust size, display mode or time of the shared native view; `None` fields
/// keep their current value.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigureProjectViewParams {
    pub project: ProjectRef,
    pub expected_view_revision: Revision,
    pub size: Option<[u32; 2]>,
    pub mode: Option<crate::view::DisplayMode>,
    pub time: Option<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportProjectParams {
    pub project: ProjectRef,
    pub destination: String,
    #[serde(default)]
    pub scene: Option<SceneId>,
}

/// Build the preview geometry package for a snapshot directly (no text
/// export / re-parse); the destination slot receives geometry.json, blobs,
/// assets and preview.json instead of scene.pbrt.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewProjectParams {
    pub project: ProjectRef,
    pub destination: String,
    /// Optional content-addressed blob store used to reuse preview payloads.
    pub store: Option<String>,
    /// `true` by default, so external callers receive a self-contained bundle.
    /// stages a verified `assets/` copy so the slot is a self-contained
    /// bundle; `false` resolves asset files in place from the project
    /// store — for slots whose only reader is the app itself (the
    /// desktop preview, capture and pick).
    #[serde(default = "default_stage_assets")]
    pub stage_assets: bool,
}
/// External callers keep the self-contained bundle.
fn default_stage_assets() -> bool {
    true
}

/// ask what the displayed preview needs after an edit
/// that only moved things. `project` names the current revision like
/// every other project method; `base_revision` is the revision the
/// caller's preview shows. Synchronous and read-only: the result is either
/// per-entity delta matrices (row-major, the same convention as the draw
/// package's `world_from_object`, so `delta · draw_matrix` is the new draw
/// matrix) or `fast:false` with a snake_case reason.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreviewTransformsParams {
    pub project: ProjectRef,
    pub base_revision: Revision,
}
/// Pin an editable snapshot and render its scene camera or an explicit view.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RenderProjectParams {
    pub project: ProjectRef,
    pub expected_settings_revision: Revision,
    pub view: Option<crate::view::ViewState>,
    #[serde(default)]
    pub scene: Option<SceneId>,
    /// Which camera renders when no view is given: the scene's own camera
    /// and Film (the default), or the shared project view.
    #[serde(default)]
    pub camera: Option<RenderCamera>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RenderCamera {
    Scene,
    View,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaptureProjectParams {
    pub project: ProjectRef,
    pub view: Option<crate::view::ViewState>,
    /// Which camera a capture without an explicit `view` uses: `view` (the
    /// default) captures the stored shared view exactly as before; `scene`
    /// captures the scene's own camera at the Film's aspect ratio (long
    /// side 1280, the framed stand-in without a usable scene camera),
    /// keeping only the shared view's mode and time. Refused together
    /// with an explicit `view`, like render's camera.
    #[serde(default)]
    pub camera: Option<RenderCamera>,
    /// Output linear diffuse reflectance without lighting or film exposure.
    #[serde(default)]
    pub debug_albedo: bool,
    /// Output the environment light's screen-space AO (Lit only).
    #[serde(default)]
    pub debug_ao: bool,
    #[serde(default)]
    pub scene: Option<SceneId>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PickProjectParams {
    pub project: ProjectRef,
    pub view: Option<crate::view::ViewState>,
    pub normalized_xy: [f64; 2],
    #[serde(default)]
    pub scene: Option<SceneId>,
}
fn page_limit() -> usize {
    100
}
/// `directives` lists every node in PBRT order; `scene_graph` lists only
/// objects (groups, shapes, lights, cameras, instances) under a Scene root.
/// Scene graph node components are JSON values: `medium` exposes two-sided
/// `inside`/`outside` on shapes and lights, but a camera uses one `binding`
/// for its ray medium (PBRT's outside value) and an
/// opaque `preserved_inside` value. Editing a camera binding preserves that
/// inside value for file round-tripping.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeView {
    #[default]
    Directives,
    SceneGraph,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListProjectNodesParams {
    pub project: ProjectRef,
    pub parent: Option<Uuid>,
    #[serde(default)]
    pub view: NodeView,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}
/// Label search across the whole project tree. Each hit reports the node's
/// ancestor path (immediate parent first, up to but excluding the root list)
/// so UIs can reveal it in a lazily loaded hierarchy.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchProjectNodesParams {
    pub project: ProjectRef,
    pub query: String,
    #[serde(default)]
    pub view: NodeView,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}
/// Search all project Contents rows and Built-in presets by name or path.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchProjectContentsParams {
    pub project: ProjectRef,
    pub query: String,
    #[serde(default = "page_limit")]
    pub limit: usize,
    #[serde(default)]
    pub offset: usize,
}
/// One Contents folder: `""` (or omitted) is `Assets/`, `"@builtin"` the
/// built-in presets.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListProjectContentsParams {
    pub project: ProjectRef,
    #[serde(default)]
    pub folder: Option<String>,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenProjectSceneParams {
    pub project: ProjectRef,
    pub scene: SceneId,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectNodeParams {
    pub project: ProjectRef,
    pub node_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectResourceParams {
    pub project: ProjectRef,
    pub resource_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectAssetParams {
    pub project: ProjectRef,
    pub asset_id: Uuid,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCacheGetParams {
    pub project: ProjectRef,
    /// The item the cache is keyed on.
    pub item: Uuid,
    /// An opaque, file-name-safe key; the server never interprets it.
    pub key: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectCachePutParams {
    pub project: ProjectRef,
    pub item: Uuid,
    pub key: String,
    /// `thumbnail` or `preview`: which cache folder the entry lands in.
    pub kind: String,
    pub media_type: String,
    /// Entry bytes, standard base64, at most 4 MiB decoded.
    pub data_base64: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ParameterTarget {
    Node(Uuid),
    Resource(Uuid),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectParametersParams {
    pub project: ProjectRef,
    pub target: ParameterTarget,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}
/// Replace windows of one array parameter's values (fixed length) as a job:
/// a data asset is rewritten on the I/O executor, an inline array simply
/// applies. All windows land in one transaction.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetParameterValuesParams {
    pub project: ProjectRef,
    pub target: ParameterTarget,
    pub name: String,
    pub windows: Vec<ValueWindow>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueWindow {
    pub offset: usize,
    pub values: ParameterValues,
}
/// One window of one parameter's values, inline or in a data asset.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectParameterValuesParams {
    pub project: ProjectRef,
    pub target: ParameterTarget,
    pub name: String,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "values_limit")]
    pub limit: usize,
}
fn values_limit() -> usize {
    256
}
/// one parameter's current spectrum-ish value resolved to the linear
/// RGB a swatch shows, plus the human-readable `source` naming how it was
/// given ("blackbody 6500 K", a builtin name, "SpectrumData: <label>",
/// rgb triples or sample pairs). `role` overrides the role the
/// target's schema would pick (albedo / unbounded / illuminant); the
/// reply echoes the role it used.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpectrumColorParams {
    pub project: ProjectRef,
    pub target: ParameterTarget,
    pub parameter: String,
    #[serde(default)]
    pub role: Option<String>,
}
/// The texture value type a `Texture` parameter table is addressed by; the
/// same shape the document model uses (`float` / `spectrum`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureValueType {
    Float,
    Spectrum,
}
/// The upstream graph of one material or texture, including the resources it
/// references transitively.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectResourceGraphParams {
    pub project: ProjectRef,
    pub resource_id: Uuid,
    /// At most this many nodes (default 256, 1..=4096); `truncated` says
    /// when more were left out.
    #[serde(default)]
    pub limit: Option<usize>,
}
/// A texture preview image, evaluated by core and kept in the project cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreviewTextureParams {
    pub project: ProjectRef,
    pub resource_id: Uuid,
    /// Square size in pixels (default 128, 8..=512).
    #[serde(default)]
    pub size: Option<u32>,
}
/// An approximate sphere preview of a material, shaded by core and kept in
/// the project cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreviewMaterialParams {
    pub project: ProjectRef,
    pub resource_id: Uuid,
    /// Square size in pixels (default 128, 8..=512).
    #[serde(default)]
    pub size: Option<u32>,
}
/// B: an image asset's preview (fit, centered, transparent letterbox),
/// decoded by core and kept in the project cache.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPreviewAssetParams {
    pub project: ProjectRef,
    pub asset_id: Uuid,
    /// Square size in pixels (default 128, 8..=512).
    #[serde(default)]
    pub size: Option<u32>,
}
/// Media types `project.preview_asset` accepts; the desktop Image Viewer
/// opens exactly these.
pub const IMAGE_PREVIEW_MEDIA_TYPES: [&str; 5] = [
    "image/png",
    "image/jpeg",
    "image/x-exr",
    "image/vnd.radiance",
    "image/x-tga",
];
/// what `set_implementation` with `keep_compatible` would do, without
/// applying it.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectPlanImplementationParams {
    pub project: ProjectRef,
    pub target: ParameterTarget,
    pub implementation: String,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectParameterSchemaParams {
    /// The PBRT directive (Shape, Material, Texture, …).
    pub directive: String,
    /// The implementation, by canonical name or r4 alias (may be empty for
    /// the area light's default name).
    pub implementation: String,
    /// Required for `Texture`: which table, the float or the spectrum one.
    #[serde(default)]
    pub texture_value_type: Option<TextureValueType>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum Transform {
    Identity,
    Translate([f64; 3]),
    Scale([f64; 3]),
    LookAt([f64; 9]),
    Rotate { degrees: f64, axis: [f64; 3] },
    Replace([f64; 16]),
    Concat([f64; 16]),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MaterialBinding {
    Default,
    None,
    Resource(Uuid),
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediumInterface {
    pub inside: Option<Uuid>,
    pub outside: Option<Uuid>,
}
/// the content of an added area light component.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AreaLightContent {
    pub implementation: String,
    /// Omitted: the area light table's declared defaults, plus the
    /// `L [1 1 1]`.
    #[serde(default)]
    pub parameters: Option<Vec<Parameter>>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Material,
    Texture,
    Medium,
    ObjectDefinition,
    SpectrumData,
}

/// Which PBRT configuration directive a render entity is: film, sampler,
/// integrator, pixel filter or accelerator.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationKind {
    Film,
    Sampler,
    Integrator,
    PixelFilter,
    Accelerator,
}

/// A signed scene up axis in its wire spelling (`+X` … `-Z`), matching the
/// project model's words exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum UpAxis {
    #[serde(rename = "+X")]
    PosX,
    #[serde(rename = "-X")]
    NegX,
    #[serde(rename = "+Y")]
    PosY,
    #[serde(rename = "-Y")]
    NegY,
    #[serde(rename = "+Z")]
    PosZ,
    #[serde(rename = "-Z")]
    NegZ,
}
impl UpAxis {
    pub fn vector(self) -> [f64; 3] {
        match self {
            UpAxis::PosX => [1., 0., 0.],
            UpAxis::NegX => [-1., 0., 0.],
            UpAxis::PosY => [0., 1., 0.],
            UpAxis::NegY => [0., -1., 0.],
            UpAxis::PosZ => [0., 0., 1.],
            UpAxis::NegZ => [0., 0., -1.],
        }
    }
}

/// Which convention the scene's world was authored with: pbrt's own
/// left-handed one, or right-handed original content (a mirror in the
/// camera chain).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Left,
    Right,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ParameterValues {
    Floats(Vec<f64>),
    /// Canonical signed i64 decimal strings, preserving values above 2^53.
    Integers(Vec<String>),
    Booleans(Vec<bool>),
    Strings(Vec<String>),
    Assets(Vec<Uuid>),
    BuiltinSpectra(Vec<String>),
    Resources {
        kind: ResourceKind,
        ids: Vec<Uuid>,
    },
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    pub name: String,
    pub value_type: String,
    /// None inherits the declaration's color space.
    pub color_space: Option<String>,
    pub values: ParameterValues,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectEdit {
    /// Add a perspective Camera to the Scene's ordered Options.
    CreateCamera {
        scene: Uuid,
    },
    /// Add one render configuration entity to the Scene's ordered
    /// Options; `implementation` omitted picks the kind's
    /// default (Film rgb, Sampler zsobol, Integrator volpath, PixelFilter
    /// gaussian, Accelerator bvh).
    CreateConfiguration {
        scene: Uuid,
        configuration: ConfigurationKind,
        #[serde(default)]
        implementation: Option<String>,
    },
    /// Set or clear the Scene's explicit Render Scene Camera reference.
    SetRenderCamera {
        scene: Uuid,
        camera: Option<Uuid>,
    },
    /// Set the Scene's up axis and world handedness. It is
    /// metadata for display and creation only: no node moves, and nothing
    /// changes on a PBRT export.
    SetSceneCoordinateSystem {
        scene: Uuid,
        up: UpAxis,
        handedness: Handedness,
    },
    CreateScene {
        location: String,
    },
    DuplicateScene {
        scene: SceneId,
        location: String,
    },
    DeleteScene {
        scene: SceneId,
    },
    MoveItem {
        item: ProjectItemId,
        location: String,
    },
    DeleteItem {
        item: ProjectItemId,
    },
    CreateFolder {
        path: String,
    },
    MoveFolder {
        from: String,
        to: String,
    },
    DeleteFolder {
        path: String,
    },
    RenameNode {
        node_id: Uuid,
        label: String,
    },
    SetNodeActive {
        node_id: Uuid,
        active: bool,
    },
    RenameResource {
        resource_id: Uuid,
        label: String,
    },
    SetTextureValueType {
        resource_id: Uuid,
        value: TextureValueType,
    },
    /// Replaces a Transform state node's operation. `identity`/`replace`
    /// `identity` and `replace` write re-parenting resets, which are only
    /// produced by import; the wire refuses them.
    SetTransform {
        node_id: Uuid,
        value: Transform,
    },
    SetMaterialBinding {
        node_id: Uuid,
        value: MaterialBinding,
    },
    SetParameter {
        target: ParameterTarget,
        name: String,
        value_type: String,
        value: Option<Parameter>,
    },
    /// Replace a fixed-size window of an existing scalar array parameter.
    /// The window must fit in the current array and keep its element count.
    SetParameterValues {
        target: ParameterTarget,
        name: String,
        offset: usize,
        values: ParameterValues,
    },
    /// Adds a built-in preset in its own group; `definition` is required only
    /// for the object instance preset. `view` is the viewport view (an eye
    /// position and a look-at point) the new object is created for: the camera
    /// preset takes its pose from it; a shape, light or area preset is placed
    /// at the view's look-at point, shapes in addition scaled by a tenth of
    /// the view's distance. The group and object instance presets take no
    /// `view`; omitted, every body is created at the origin, unit sized, and
    /// the camera takes the scene's default placement.
    CreatePreset {
        preset: Uuid,
        parent: Uuid,
        #[serde(default)]
        after: Option<Uuid>,
        #[serde(default)]
        definition: Option<Uuid>,
        #[serde(default)]
        view: Option<PresetView>,
    },
    /// create one resource. Omitting `parameters`
    /// fills every constant default of the table that the dependency
    /// rules can export (core-side); an explicit list is used as given.
    CreateResource {
        /// Optional caller-chosen id for edits later in the same apply.
        #[serde(default)]
        #[serde(skip_serializing_if = "Option::is_none")]
        id: Option<Uuid>,
        resource_kind: ResourceKind,
        implementation: String,
        label: String,
        #[serde(default)]
        folder: Option<String>,
        #[serde(default)]
        texture_value_type: Option<TextureValueType>,
        #[serde(default)]
        parameters: Option<Vec<Parameter>>,
    },
    /// Null leaves that binding unchanged; `default` and an empty medium
    /// interface clear the object's local binding.
    SetLocalBinding {
        node_id: Uuid,
        #[serde(default)]
        material: Option<MaterialBinding>,
        #[serde(default)]
        medium: Option<MediumInterface>,
    },
    /// the object's area light component. Null
    /// removes it; omitted `parameters` fill the area light table's declared
    /// defaults plus `L [1 1 1]`.
    SetAreaLight {
        node_id: Uuid,
        light: Option<AreaLightContent>,
    },
    /// remove the object's local material and/or medium binding states.
    ClearLocalBinding {
        node_id: Uuid,
        material: bool,
        medium: bool,
    },
    /// make reverse orientation take `enabled` effect for the object.
    SetReverseOrientation {
        node_id: Uuid,
        enabled: bool,
    },
    /// Make `matrix` (row-major) the node's local transform.
    /// A group's leading run or the object's dedicated group run (created on
    /// demand) is rewritten; a run with PBRT `Identity` / `Transform` keeps
    /// the root as its transform parent. The active Options camera accepts a
    /// camera-to-world pose; earlier cameras are refused because their
    /// transform state is shared with later Options directives.
    SetLocalMatrix {
        node_id: Uuid,
        matrix: [f64; 16],
    },
    /// The same as `set_local_matrix` with
    /// `T · Ry(y) · Rx(x) · Rz(z) · S`, Euler degrees in Z→X→Y order.
    SetLocalTrs {
        node_id: Uuid,
        translation: [f64; 3],
        rotation_euler_degrees: [f64; 3],
        scale: [f64; 3],
    },
    SetAssetReference {
        target: ParameterTarget,
        name: String,
        asset: Uuid,
    },
    /// Points an object instance at another object definition.
    SetInstanceDefinition {
        node_id: Uuid,
        definition: Uuid,
    },
    /// Convert a scene group into an ObjectDefinition and replace it with an instance.
    MakeObjectDefinition {
        node_id: Uuid,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Replace a scene object instance with an editable group containing a copy of its definition.
    UnpackObjectInstance {
        node_id: Uuid,
    },
    /// Removes a scene-graph object (with its dedicated group).
    DeleteNode {
        node_id: Uuid,
    },
    /// Copies a scene-graph object (with its dedicated group) right after itself.
    DuplicateNode {
        node_id: Uuid,
    },
    /// Moves a scene-graph object (with its dedicated group) into a group,
    /// onto the World scope, or onto a Camera: a Camera parent attaches the
    /// unit to that camera, and any other parent clears the attachment.
    MoveNode {
        node_id: Uuid,
        parent: Uuid,
        #[serde(default)]
        after: Option<Uuid>,
    },
    /// Change an entity's, configuration's, or
    /// resource's implementation within its directive. Parameters are kept
    /// by `parameters`; the apply result reports the kept ones the new
    /// implementation's table does not know in `kept_unknown`, and a change
    /// that would keep a reference or file the new implementation cannot
    /// export is refused (`reason: "breaks_reference"`).
    SetImplementation {
        target: ParameterTarget,
        implementation: String,
        parameters: ImplementationParameters,
    },
    /// Wire a texture into a texture-input parameter of a
    /// material or texture, replacing its declaration. `texture` names an
    /// existing texture; `create` makes a new one (the slot's value type,
    /// the table's defaults) in the same edit. Core checks the slot takes
    /// textures (`not_a_texture_input`), the lineage (`texture_lineage`) and
    /// that `texture` is one (`not_a_texture`).
    ConnectTexture {
        target: ParameterTarget,
        parameter: String,
        #[serde(default)]
        texture: Option<Uuid>,
        #[serde(default)]
        create: Option<TextureCreate>,
        /// An image asset: an imagemap of it in the row's lineage, reused
        /// when one exists, created otherwise.
        #[serde(default)]
        image: Option<Uuid>,
    },
    /// Remove a texture connection; r4's default applies again.
    DisconnectTexture {
        target: ParameterTarget,
        parameter: String,
    },
}

#[cfg(test)]
mod active_edit_tests {
    use super::ProjectEdit;
    use serde_json::json;

    #[test]
    fn set_node_active_edit_deserializes_from_its_wire_shape() {
        let edit = json!({
            "kind": "set_node_active",
            "node_id": "6f22aa00-0000-4000-8000-000000000001",
            "active": false
        });
        let parsed: ProjectEdit = serde_json::from_value(edit).unwrap();
        assert!(matches!(
            parsed,
            ProjectEdit::SetNodeActive { active: false, .. }
        ));
    }
}
/// The viewport view passed to `create_preset`: the eye position and
/// the point it looks at. The camera perspective preset poses with
/// both; a shape, light or area preset is placed at the look-at point
/// (a shape scaled by a tenth of the eye's distance to it).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetView {
    pub position: [f64; 3],
    pub look_at: [f64; 3],
}

/// The texture `connect_texture` creates.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TextureCreate {
    pub implementation: String,
    pub label: String,
    #[serde(default)]
    pub folder: Option<String>,
}
/// How `set_implementation` treats the declared parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImplementationParameters {
    /// Keep every parameter; warn about the ones the new table lacks.
    KeepCompatible,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "id",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ProjectItemId {
    Scene(SceneId),
    Resource(Uuid),
    Asset(Uuid),
}
/// where one project item is used. The three
/// reference kinds a row takes: a scene node, an object definition's inner
/// node, or a resource.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListReferencesParams {
    pub project: ProjectRef,
    pub item: ProjectItemId,
    #[serde(default)]
    pub offset: usize,
    #[serde(default = "page_limit")]
    pub limit: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyProjectParams {
    pub project: ProjectRef,
    pub edits: Vec<ProjectEdit>,
    /// Starts a raster capture of the edited revision after the transaction commits.
    #[serde(default)]
    pub capture: Option<ApplyCapture>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApplyCapture {
    pub view: Option<crate::view::ViewState>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AddImportParams {
    pub project: ProjectRef,
    pub source_path: String,
    #[serde(default)]
    pub folder: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::ProjectEdit;

    #[test]
    fn unpack_object_instance_wire_edit_parses() {
        let edit = serde_json::json!({
            "kind": "unpack_object_instance",
            "node_id": "d5b71f0b-e4a0-4c0f-9864-95e1285be1a1"
        });
        serde_json::from_value::<ProjectEdit>(edit).unwrap();
    }
}
