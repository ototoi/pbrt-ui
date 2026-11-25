use crate::{CoordinateSystem, Result, Revision};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

macro_rules! ids {
    ($($name:ident),+) => { $(
        #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub uuid::Uuid);
        impl $name { pub fn new() -> Self { Self(uuid::Uuid::new_v4()) } }
        impl Default for $name { fn default() -> Self { Self::new() } }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { self.0.fmt(f) }
        }
    )+ };
}
ids!(
    ProjectId,
    SceneId,
    NodeId,
    ResourceId,
    AssetId,
    ImportId,
    SourceId,
    OccurrenceId
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum ItemId {
    Scene(SceneId),
    Resource(ResourceId),
    Asset(AssetId),
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ObjectRef {
    pub sha256: String,
    pub size: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceSpan {
    pub offset: u64,
    pub length: u64,
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Origin {
    pub import: ImportId,
    pub occurrence: OccurrenceId,
    pub span: SourceSpan,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportedSource {
    /// Provenance only; never used as the project's active input path.
    pub original_path: String,
    pub original: ObjectRef,
    pub expanded: Option<ObjectRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceOccurrence {
    pub source: SourceId,
    pub parent: Option<OccurrenceId>,
    /// Span in the parent's expanded source, not this occurrence's source.
    pub callsite: Option<SourceSpan>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportRecord {
    pub id: ImportId,
    pub importer_version: String,
    pub dialect: ImportDialect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub renderer_revision: Option<String>,
    pub diagnostics: Vec<ImportDiagnostic>,
    pub root: OccurrenceId,
    pub sources: BTreeMap<SourceId, ImportedSource>,
    pub occurrences: BTreeMap<OccurrenceId, SourceOccurrence>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportDialect {
    PbrtR4,
    PbrtV4,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImportDiagnostic {
    pub code: String,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<ImportSeverity>,
    pub origin: Option<Origin>,
}

impl ImportDiagnostic {
    /// The common kind of import diagnostic: something was skipped or
    /// reported, and the project still imports. New records always carry
    /// `Warning`; older records keep no severity and count as warnings too.
    pub fn warning(
        code: impl Into<String>,
        message: impl Into<String>,
        origin: Option<Origin>,
    ) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            severity: Some(ImportSeverity::Warning),
            origin,
        }
    }
}

/// How much attention a diagnostic deserves. Older records carry no
/// severity and count as warnings where the UI distinguishes them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImportSeverity {
    Info,
    Warning,
}

/// Where an asset came from during import. `logical_path` is the path the
/// source file was read under; `origin` keeps the source span for diagnostics.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AssetOrigin {
    pub logical_path: String,
    pub origin: Option<Origin>,
}

/// An external file stored by the project. `location` only says where the file
/// lives (relative to `Assets/`); references are always by `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Asset {
    pub id: AssetId,
    pub location: String,
    pub media_type: String,
    pub sha256: String,
    pub size: u64,
    pub origin: Option<AssetOrigin>,
}
impl Asset {
    /// Content-addressed handle used by the store's object API.
    pub fn object(&self) -> ObjectRef {
        ObjectRef {
            sha256: self.sha256.clone(),
            size: self.size,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FloatValue {
    Finite(f64),
    PositiveInfinity,
    NegativeInfinity,
    Nan,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ResourceKind {
    Material,
    Texture,
    Medium,
    ObjectDefinition,
    SpectrumData,
}

/// The maximum number of sample pairs a SpectrumData resource can hold is
/// an upper bound in the low thousands. Larger tables are
/// malformed or hostile input, so import skips the file with a diagnostic.
pub const MAX_SPECTRUM_SAMPLES: usize = 8192;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TextureValueType {
    Float,
    Spectrum,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArrayEncoding {
    F64Le,
    I64Le,
    BoolU8,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ParameterValues {
    Floats(Vec<FloatValue>),
    Integers(Vec<i64>),
    Booleans(Vec<bool>),
    Strings(Vec<String>),
    BuiltinSpectra(Vec<String>),
    Assets(Vec<AssetId>),
    Resources {
        kind: ResourceKind,
        ids: Vec<ResourceId>,
    },
    Array {
        asset: AssetId,
        encoding: ArrayEncoding,
        count: u64,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum ColorSpaceBinding {
    #[default]
    Inherit,
    Explicit(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parameter {
    /// The declared type is preserved, including point/vector/spectrum forms.
    pub value_type: String,
    pub name: String,
    pub color_space: ColorSpaceBinding,
    pub values: ParameterValues,
}
impl Parameter {
    pub fn floats(name: impl Into<String>, values: impl IntoIterator<Item = f64>) -> Self {
        Self {
            value_type: "float".into(),
            name: name.into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Floats(values.into_iter().map(FloatValue::Finite).collect()),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScopeKind {
    World,
    Attribute,
    Transform,
    ObjectDefinition,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntityKind {
    Shape,
    Light,
    AreaLight,
    Camera,
}

/// A camera's pose in its scene: camera-to-world matrices for the two
/// transform samples (row-major, like every model matrix). Cameras carry this
/// as a value so editing does not depend on the Options transform stream.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CameraPose {
    pub start: [f64; 16],
    pub end: [f64; 16],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigurationKind {
    Film,
    Sampler,
    Integrator,
    PixelFilter,
    Accelerator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActiveSamples {
    All,
    Start,
    End,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum TransformOperation {
    Identity,
    Translate([f64; 3]),
    Scale([f64; 3]),
    LookAt([f64; 9]),
    Rotate {
        degrees: f64,
        axis: [f64; 3],
    },
    /// Row-major f64; PBRT lexical matrix order is handled by the importer.
    Replace([f64; 16]),
    Concat([f64; 16]),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum MaterialBinding {
    Default,
    None,
    Resource(ResourceId),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum StateOperation {
    Transform(TransformOperation),
    ActiveTransform(ActiveSamples),
    TransformTimes([f64; 2]),
    Material(MaterialBinding),
    Medium {
        inside: Option<ResourceId>,
        outside: Option<ResourceId>,
    },
    ReverseOrientation,
    ColorSpace(String),
    DefineCoordinates(String),
    UseCoordinates(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NodeKind {
    Scope {
        scope: ScopeKind,
        children: Vec<NodeId>,
        /// The sole visible object internally owned by this Attribute scope.
        /// When present, the scope remains in the PBRT directive stream but is
        /// transparent in the editor's scene hierarchy.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        owned_object: Option<NodeId>,
        /// For a World-level group: the camera its transform-parent attaches
        /// to (the "camera" coordinate system). The camera itself lives in
        /// Options and cannot own children, so the group references it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_camera: Option<NodeId>,
    },
    State {
        operation: StateOperation,
    },
    Entity {
        entity: EntityKind,
        implementation: String,
        parameters: Vec<Parameter>,
        /// Camera-only: the camera-to-world matrices at the two transform
        /// samples. Every freshly imported camera carries one; older saved
        /// projects may not.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        pose: Option<CameraPose>,
    },
    Instance {
        definition: ResourceId,
    },
    Configuration {
        configuration: ConfigurationKind,
        implementation: String,
        parameters: Vec<Parameter>,
    },
    /// Option holds one typed lexical value (no original token is kept).
    Option {
        name: String,
        value: OptionValue,
    },
}

/// Typed value of a PBRT `Option` directive. Import converts the single lexical
/// token with `schema::option`'s type table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OptionValue {
    Bool(bool),
    Integer(i64),
    Float(FloatValue),
    String(String),
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Node {
    pub id: NodeId,
    pub label: String,
    pub edition: Revision,
    pub origin: Option<Origin>,
    #[serde(default = "node_active_by_default", skip_serializing_if = "is_true")]
    pub active: bool,
    pub data: NodeKind,
}

fn node_active_by_default() -> bool {
    true
}

fn is_true(value: &bool) -> bool {
    *value
}

impl Node {
    pub fn new(label: impl Into<String>, data: NodeKind) -> Self {
        Self {
            id: NodeId::new(),
            label: label.into(),
            edition: Revision::initial(),
            origin: None,
            active: true,
            data,
        }
    }
}

/// Transform and graphics state captured where a resource was declared.
/// Textures and media only need `transform`; object definitions need both.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclarationState {
    pub transform: Option<CapturedTransform>,
    pub graphics: Option<CapturedGraphics>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedTransform {
    /// Row-major f64, start/end samples of the declaration's CTM.
    pub start: [f64; 16],
    pub end: [f64; 16],
    pub active: ActiveSamples,
    pub times: [f64; 2],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CapturedGraphics {
    pub material: MaterialBinding,
    pub inside_medium: Option<ResourceId>,
    pub outside_medium: Option<ResourceId>,
    pub reverse_orientation: bool,
    pub color_space: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Resource {
    pub id: ResourceId,
    pub label: String,
    /// Placement relative to `Assets/`; never used for references.
    pub location: String,
    pub edition: Revision,
    pub kind: ResourceKind,
    pub texture_value_type: Option<TextureValueType>,
    pub implementation: String,
    pub parameters: Vec<Parameter>,
    pub definition_scope: Option<NodeId>,
    pub declared_in: Option<DeclarationState>,
    pub origin: Option<Origin>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SceneDocument {
    pub id: SceneId,
    pub label: String,
    /// Placement relative to `Assets/`; never used for references.
    pub location: String,
    pub options: Vec<NodeId>,
    /// Camera explicitly selected for Render Scene. `None` is a deliberate
    /// unset value; it is independent of PBRT option order.
    #[serde(default)]
    pub render_camera: Option<NodeId>,
    /// Display and authoring metadata: the up axis the scene is
    /// authored around and whether its content came from a right-handed
    /// source. Never changes node matrices; never written to a PBRT export.
    #[serde(default)]
    pub coordinate_system: CoordinateSystem,
    pub world: NodeId,
    pub nodes: BTreeMap<NodeId, Node>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectDocument {
    pub id: ProjectId,
    pub title: String,
    pub default_scene: Option<SceneId>,
    pub scenes: BTreeMap<SceneId, Arc<SceneDocument>>,
    pub resources: BTreeMap<ResourceId, Resource>,
    /// Nodes inside ObjectDefinition scopes. They share the NodeId space with
    /// every scene; validation rejects duplicates.
    pub definition_nodes: BTreeMap<NodeId, Node>,
    pub assets: BTreeMap<AssetId, Asset>,
    /// Empty folders are kept explicitly.
    pub folders: BTreeSet<String>,
    pub imports: BTreeMap<ImportId, ImportRecord>,
}

/// One scene of a project, plus the document-wide tables it references.
#[derive(Clone, Copy, Debug)]
pub struct SceneView<'a> {
    pub document: &'a ProjectDocument,
    pub scene: &'a SceneDocument,
}
impl<'a> SceneView<'a> {
    /// Scene nodes first, then object-definition nodes. Ids are unique, so the
    /// order cannot change the result; it only makes the lookup scope explicit.
    pub fn node(&self, id: NodeId) -> Option<&'a Node> {
        self.scene
            .nodes
            .get(&id)
            .or_else(|| self.document.definition_nodes.get(&id))
    }
    pub fn resource(&self, id: ResourceId) -> Option<&'a Resource> {
        self.document.resources.get(&id)
    }
}

fn default_scene(title: &str) -> SceneDocument {
    let world = Node::new(
        "World",
        NodeKind::Scope {
            scope: ScopeKind::World,
            children: Vec::new(),
            owned_object: None,
            parent_camera: None,
        },
    );
    SceneDocument {
        id: SceneId::new(),
        label: title.into(),
        location: format!("Scenes/{title}.scene"),
        options: Vec::new(),
        render_camera: None,
        coordinate_system: CoordinateSystem::default(),
        world: world.id,
        nodes: BTreeMap::from([(world.id, world)]),
    }
}

pub(crate) fn default_folders() -> BTreeSet<String> {
    [
        "Scenes",
        "Materials",
        "Textures",
        "Images",
        "Media",
        "Objects",
        "Meshes",
        "Spectra",
        "Other",
        "Data",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

impl ProjectDocument {
    pub fn new(title: impl Into<String>) -> Self {
        let title = title.into();
        let scene = default_scene(&title);
        let default_scene = scene.id;
        Self {
            id: ProjectId::new(),
            title,
            default_scene: Some(default_scene),
            scenes: BTreeMap::from([(default_scene, Arc::new(scene))]),
            resources: BTreeMap::new(),
            definition_nodes: BTreeMap::new(),
            assets: BTreeMap::new(),
            folders: default_folders(),
            imports: BTreeMap::new(),
        }
    }
    pub fn new_empty(title: impl Into<String>) -> Self {
        let mut document = Self::new(title);
        document.scenes.clear();
        document.default_scene = None;
        document
    }
    pub fn scene(&self, id: impl Into<Option<SceneId>>) -> Option<&SceneDocument> {
        self.scenes.get(&id.into()?).map(|s| s.as_ref())
    }
    pub fn scene_view(&self, id: impl Into<Option<SceneId>>) -> Result<SceneView<'_>> {
        let id = id.into().ok_or_else(|| crate::invalid("no active scene"))?;
        let scene = self
            .scene(id)
            .ok_or_else(|| crate::invalid("missing scene"))?;
        Ok(SceneView {
            document: self,
            scene,
        })
    }
    pub fn default_scene_view(&self) -> Result<SceneView<'_>> {
        self.scene_view(self.default_scene)
    }
    pub fn validate(&self) -> Result<()> {
        crate::validation::validate(self)
    }
    pub fn object_refs(&self) -> Result<Vec<ObjectRef>> {
        crate::validation::objects(self)
    }
    pub fn reference_index(&self) -> crate::ReferenceIndex {
        crate::ReferenceIndex::build(self)
    }
}
