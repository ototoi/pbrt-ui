use crate::Result;
use crate::{
    source::{Declaration, Sources, resolve},
    *,
};
use project::*;
use scene_parse::Directive;
use serde::Serialize;
mod parameters;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct ImportOptions {
    pub max_source_bytes: u64,
    pub max_expanded_bytes: u64,
    pub max_copied_bytes: u64,
    pub max_include_depth: usize,
    pub max_nodes: usize,
    /// Tokens one directive may use. A medium's bulk density can run to
    /// millions of values (smoke-plume: 9,437,184), so this budget sits far
    /// above the per-file byte limits.
    pub max_directive_tokens: usize,
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    pub diagnostics: Vec<ImportDiagnostic>,
}
impl Default for ImportOptions {
    fn default() -> Self {
        Self {
            max_source_bytes: 128 * 1024 * 1024,
            max_expanded_bytes: 128 * 1024 * 1024,
            max_copied_bytes: 8 * 1024 * 1024 * 1024,
            max_include_depth: 64,
            max_nodes: 250_000,
            max_directive_tokens: 16_000_000,
        }
    }
}
/// Create and publish a new format-3 folder project after import validation.
pub fn import_project(
    source: &Path,
    destination: &Path,
    options: &ImportOptions,
    cancelled: impl Fn() -> bool,
) -> Result<(ProjectStore, ProjectSession)> {
    let (store, session, _) = import_project_with_report(source, destination, options, cancelled)?;
    Ok((store, session))
}

pub fn import_project_with_report(
    source: &Path,
    destination: &Path,
    options: &ImportOptions,
    cancelled: impl Fn() -> bool,
) -> Result<(ProjectStore, ProjectSession, ImportReport)> {
    let destination = absolute_destination(destination)?;
    let parent = destination.parent().unwrap_or(Path::new("."));
    let temporary = tempfile::tempdir_in(parent)?;
    let staging = temporary.path().join("project");
    let title = source
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Imported scene");
    let mut store = FolderStore::create(&staging, ProjectId::new(), title, Revision::initial())?;
    let (session, report) = import_into(source, &mut store, options, &cancelled)?;
    let source_snapshot = session.snapshot();
    let plan = FormatThreePlan::new(&source_snapshot)?;
    let source_store = ProjectStore::Folder(store);
    let (published, receipt) =
        source_store.publish_format_three(&destination, &source_snapshot, &plan, &cancelled)?;
    let mut session = ProjectSession::from_saved_snapshot(plan.snapshot().clone())?;
    session.mark_saved(&receipt)?;
    Ok((published, session, report))
}

/// Import into a caller-provided empty format-2 folder project. Raw source and
/// asset bytes use a temporary object spool during parsing; only imported
/// assets and the format-2 model are written into the project.
pub fn import_into(
    source: &Path,
    store: &mut FolderStore,
    options: &ImportOptions,
    cancelled: impl Fn() -> bool,
) -> Result<(ProjectSession, ImportReport)> {
    check(&cancelled)?;
    if !store.is_empty_for_import()? {
        return Err(invalid("import destination folder project must be empty"));
    }
    let ParsedImport {
        document,
        spool,
        owner: _owner,
        report,
        ..
    } = parse_import(source, store.project_id(), options, &cancelled)?;
    // One batch: a single duplicate scan and baseline walk for every asset
    // the import produced, instead of one full-folder scan per asset.
    store.import_assets(
        document.assets.values(),
        |asset| spool.open_object(&asset.object()),
        &cancelled,
    )?;
    let session = ProjectSession::new(document)?;
    let saved = store.save_cancellable(&session.snapshot(), &cancelled)?;
    let mut session = session;
    session.mark_saved(&saved)?;
    Ok((session, report))
}
struct ParsedImport {
    document: ProjectDocument,
    spool: ObjectStore,
    owner: tempfile::TempDir,
    report: ImportReport,
    copied_bytes: u64,
}

/// Parsing shared by the import path and the test-only unresolved parser:
/// Parse everything up to `builder.finish()`, including the scene location
/// and import record. The spool and cancellation flag are borrowed from the caller;
/// validation, evaluation and export checks stay with the caller too.
struct ParsedDocument<'a, F> {
    document: ProjectDocument,
    sources: Sources<'a, F>,
}

/// The spool directory stays alive for as long as the returned value.
#[allow(clippy::type_complexity)]
fn parse_document<'a, F: Fn() -> bool>(
    source: &Path,
    project_id: ProjectId,
    options: &'a ImportOptions,
    spool: &'a mut ObjectStore,
    cancelled: &'a F,
) -> Result<ParsedDocument<'a, F>> {
    if options.max_source_bytes > 128 * 1024 * 1024
        || options.max_include_depth > 256
        || options.max_nodes > 250_000
    {
        return Err(invalid("import limits exceed native format limits"));
    }
    let mut sources = Sources::new(spool, options, cancelled);
    sources.read(source, &mut vec![], None)?;
    let declarations = std::mem::take(&mut sources.declarations);
    let mut document = ProjectDocument::new(
        source
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Imported scene"),
    );
    document.id = project_id;
    let scene = document
        .default_scene
        .ok_or_else(|| invalid("import produced no scene"))?;
    let scene_location = format!(
        "Scenes/{}.scene",
        project::layout::sanitize(&document.title, "scene")
    );
    let mut layout = project::layout::LocationNamer::new();
    layout.reserve(&scene_location);
    let mut builder = Builder {
        document,
        scene,
        scopes: vec![],
        in_world: false,
        world_seen: false,
        world_ended: false,
        names: BTreeMap::new(),
        sources: &mut sources,
        assets: BTreeMap::new(),
        object_assets: BTreeMap::new(),
        added_assets: Vec::new(),
        spectra: BTreeMap::new(),
        added_spectra: Vec::new(),
        coordinates: BTreeMap::new(),
        state: GraphicsState::default(),
        state_stack: Vec::new(),
        layout,
        skipped: None,
        group_names: Vec::new(),
        pending_materials: Vec::new(),
    };
    for declaration in declarations {
        check(cancelled)?;
        builder.import(declaration)?;
        if builder.node_count() > options.max_nodes {
            return Err(project::ProjectError::ResourceLimit(format!(
                "more than {} nodes (import limit)",
                options.max_nodes
            ))
            .into());
        }
    }
    builder.finish();
    builder.set_scene_location(scene_location);
    let mut document = builder.document;
    document
        .imports
        .insert(sources.record.id, sources.record.clone());
    Ok(ParsedDocument { document, sources })
}

fn parse_import(
    source: &Path,
    project_id: ProjectId,
    options: &ImportOptions,
    cancelled: &impl Fn() -> bool,
) -> Result<ParsedImport> {
    let temporary = tempfile::tempdir()?;
    let spool_path = temporary.path().join("spool");
    let mut spool = ObjectStore::create(&spool_path)?;
    let ParsedDocument {
        mut document,
        mut sources,
    } = parse_document(source, project_id, options, &mut spool, &cancelled)?;
    // The PBRT relationship ends at import: resolve transform resets right
    // after the parse, before validation and evaluation, so no model
    // consumer sees `Identity`/`Replace`/coordinate-system reuse.
    let scene = document
        .default_scene
        .ok_or_else(|| invalid("import produced no scene"))?;
    let resolution = project::resolve_transform_parents(&mut document, scene)?;
    // Fold a light's `from` into its own transform run, so the
    // wire-frame and the Move gizmo sit on the same point. The fold only
    // inserts nodes, so the budget below re-counts after it.
    project::fold_light_from_transforms(&mut document, scene)?;
    // Resolution adds nodes (moved groups and explicit attribute states), so
    // the budget holds a second time over the resolved model.
    let resolved_nodes = document.scenes[&scene].nodes.len() + document.definition_nodes.len();
    if resolved_nodes > options.max_nodes {
        return Err(project::ProjectError::ResourceLimit(format!(
            "more than {} nodes (import limit)",
            options.max_nodes
        ))
        .into());
    }
    if let Some(record) = document.imports.get_mut(&sources.record.id) {
        record.diagnostics.extend(resolution.iter().cloned());
    }
    sources.record.diagnostics.extend(resolution);
    // Estimate the scene's display coordinate system from its
    // cameras — the render camera decides — and report it as an info
    // diagnostic. Cameras that disagree with the choice are named. A
    // scene without cameras stays silent: it keeps the +Y/left default
    // and there is no source line the diagnostic could point at.
    let scene_document = document
        .scenes
        .get(&scene)
        .ok_or_else(|| invalid("import produced no scene"))?;
    let inference = project::coordinate::infer_scene(scene_document);
    let origin_camera = inference.camera.or(scene_document.render_camera);
    if let Some(camera) = origin_camera {
        let mut message = format!("coordinate system: {}", inference.coordinate_system);
        if inference.coordinate_system.handedness == project::Handedness::Right {
            message.push_str(" (camera has a mirror)");
        }
        if inference.camera.is_none() {
            message.push_str(" (defaults; the scene has no camera with a usable pose)");
        }
        match inference.conflicts.as_slice() {
            [] => {}
            [(_, estimate)] => {
                message.push_str(&format!("; another camera suggests {estimate}"));
            }
            many => message.push_str(&format!("; {} other cameras disagree", many.len())),
        }
        let origin = scene_document
            .nodes
            .get(&camera)
            .and_then(|node| node.origin.clone());
        let diagnostic = project::ImportDiagnostic {
            code: "scene_coordinate_system".into(),
            message,
            severity: Some(project::ImportSeverity::Info),
            origin,
        };
        if let Some(record) = document.imports.get_mut(&sources.record.id) {
            record.diagnostics.push(diagnostic.clone());
        }
        sources.record.diagnostics.push(diagnostic);
    }
    std::sync::Arc::make_mut(document.scenes.get_mut(&scene).expect("scene exists"))
        .coordinate_system = inference.coordinate_system;
    document.validate()?;
    let view = document.scene_view(document.default_scene)?;
    evaluate(&view)?;
    crate::schema::exportable(&document)?;
    sources.verify_inputs()?;
    let copied_bytes = sources.copied_bytes();
    let report = ImportReport {
        diagnostics: sources.record.diagnostics.clone(),
    };
    drop(sources);
    Ok(ParsedImport {
        document,
        spool,
        owner: temporary,
        report,
        copied_bytes,
    })
}

/// Parse PBRT into a project document without resolving transform parents or
/// running the export checks. Test-only: the import path resolves through
/// `project::resolve_transform_parents`; tests drive the resolution by hand
/// and compare the unresolved and resolved models.
#[doc(hidden)]
pub fn parse_unresolved(source: &Path, options: &ImportOptions) -> Result<ProjectDocument> {
    let cancelled = || false;
    let temporary = tempfile::tempdir()?;
    let spool_path = temporary.path().join("spool");
    let mut spool = ObjectStore::create(&spool_path)?;
    let ParsedDocument { document, .. } =
        parse_document(source, ProjectId::new(), options, &mut spool, &cancelled)?;
    Ok(document)
}

/// Prepare a new scene and its resources without saving or changing a project.
/// Decoder validation of image/PLY leases belongs to the worker before publish.
pub fn prepare_scene(
    source: &Path,
    options: &ImportOptions,
    cancelled: impl Fn() -> bool,
) -> Result<(PreparedImport, ImportReport)> {
    let base = project::cache_directory()?;
    prepare_scene_in(source, options, cancelled, &base)
}

/// Prepare a scene and stage copied assets below an explicit project or cache base.
pub fn prepare_scene_in(
    source: &Path,
    options: &ImportOptions,
    cancelled: impl Fn() -> bool,
    staging_base: &Path,
) -> Result<(PreparedImport, ImportReport)> {
    let ParsedImport {
        document,
        spool,
        owner: _owner,
        report,
        copied_bytes,
    } = parse_import(source, ProjectId::new(), options, &cancelled)?;
    if report
        .diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == "missing_world")
    {
        return Err(invalid("scene import requires WorldBegin"));
    }
    let snapshot = ProjectSession::new(document)?.snapshot();
    let plan = FormatThreePlan::new(&snapshot)?;
    let mut document = plan.snapshot().document().clone();
    document.default_scene = None;
    let mut staging = ImportStaging::with_copied_bytes_in(staging_base, copied_bytes)?;
    let mut payloads = Vec::new();
    for asset in document.assets.values() {
        check(&cancelled)?;
        payloads.push(staging.stage_file_cancellable(
            asset,
            &spool.object_path(&asset.object())?,
            &cancelled,
        )?);
    }
    Ok((PreparedImport::new(document, payloads)?, report))
}

pub(crate) fn absolute_destination(path: &Path) -> Result<PathBuf> {
    let name = path
        .file_name()
        .ok_or_else(|| invalid("destination requires a new directory name"))?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = protocol::path::canonicalize(parent)?.join(name);
    if destination.symlink_metadata().is_ok() {
        return Err(invalid("destination already exists"));
    }
    Ok(destination)
}
struct Builder<'a, 'b, F> {
    document: ProjectDocument,
    scene: SceneId,
    scopes: Vec<(ScopeKind, NodeId)>,
    in_world: bool,
    world_seen: bool,
    world_ended: bool,
    names: BTreeMap<(u8, String), ResourceId>,
    sources: &'a mut Sources<'b, F>,
    assets: BTreeMap<PathBuf, AssetId>,
    /// Content-addressed identity: the same bytes share one asset.
    object_assets: BTreeMap<ObjectRef, AssetId>,
    /// Assets added by the declaration being imported, removed if it is skipped.
    added_assets: Vec<AssetId>,
    /// spd files imported as SpectrumData resources, one shared
    /// resource per distinct file within one import.
    spectra: BTreeMap<PathBuf, ResourceId>,
    /// SpectrumData resources added by the declaration being imported,
    /// removed if it is skipped.
    added_spectra: Vec<ResourceId>,
    /// Coordinate systems defined so far, with their transforms.
    coordinates: BTreeMap<String, (project::math::Mat4, project::math::Mat4)>,
    /// Graphics state at the current declaration. Mirrors the evaluator.
    state: GraphicsState,
    state_stack: Vec<GraphicsState>,
    layout: project::layout::LocationNamer,
    /// A skipped scope whose contents are being discarded until it closes.
    skipped: Option<SkippedScope>,
    /// One naming frame per open scope, pushed and popped alongside
    /// `scopes` to keep group names aligned with open scopes.
    group_names: Vec<ScopeNaming>,
    /// `NamedMaterial` bindings whose material was not yet defined at the
    /// directive (pbrt-v4 binds named materials after the whole parse):
    /// node, referenced name, source location and origin. Resolved in
    /// `finish`.
    pending_materials: Vec<(NodeId, String, String, Origin)>,
}

fn media_type(path: &Path) -> &'static str {
    if path
        .file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.to_ascii_lowercase().ends_with(".ply.gz"))
    {
        return "application/x-ply";
    }
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png") => "image/png",
        Some("jpg" | "jpeg") => "image/jpeg",
        Some("exr") => "image/x-exr",
        Some("hdr") => "image/vnd.radiance",
        Some("tga") => "image/x-tga",
        Some("pfm") => "image/x-portable-floatmap",
        Some("ply") => "application/x-ply",
        Some("spd") => "text/plain",
        Some("array") => "application/x-pbrt-ui-array",
        // nanovdb grid payloads: medium data, Media/ in the folder
        // doc's layout.
        Some("nvdb") => "application/x-nanovdb",
        _ => "application/octet-stream",
    }
}
fn asset_folder(media_type: &str) -> &'static str {
    match media_type {
        "application/x-ply" => "Meshes",
        "text/plain" => "Spectra",
        "application/x-pbrt-ui-array" => "Data",
        "application/x-nanovdb" => "Media",
        "application/octet-stream" => "Other",
        _ => "Images",
    }
}
fn basename(path: &Path) -> String {
    path.file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("asset")
        .into()
}
fn extension(path: &Path) -> &str {
    path.extension().and_then(|e| e.to_str()).unwrap_or("bin")
}
struct SkippedScope {
    depth: usize,
    directives: usize,
    diagnostic: ImportDiagnostic,
}
/// Names recordable for a still-unnamed group: the first of each
/// kind set directly inside one scope, kept while that scope is the
/// innermost open one.
#[derive(Default)]
struct ScopeNaming {
    material: Option<String>,
    instance: Option<String>,
    plymesh: Option<String>,
}
/// The name a "Group" scope can take from one declaration its own scope
/// holds, read off the raw directive before the import decides whether
/// the declaration survives.
enum GroupNaming {
    Material(String),
    Instance(String),
    Plymesh(String),
}
fn group_naming_candidate(directive: &Directive) -> Option<GroupNaming> {
    let text = directive.positional_text(0).unwrap_or("");
    match directive.name.as_str() {
        "NamedMaterial" if !text.is_empty() => Some(GroupNaming::Material(text.to_owned())),
        "ObjectInstance" if !text.is_empty() => Some(GroupNaming::Instance(text.to_owned())),
        "Shape" if text == "plymesh" => Some(GroupNaming::Plymesh(ply_file_stem(
            &directive.params().string("filename", ""),
        )?)),
        _ => None,
    }
}
/// `mesh_00001` from `geometry/mesh_00001.ply`; `dragon` from
/// `dragon.ply.gz`.
fn ply_file_stem(filename: &str) -> Option<String> {
    let name = Path::new(filename).file_name()?.to_str()?;
    let name = name.strip_suffix(".gz").unwrap_or(name);
    let stem = Path::new(name).file_stem()?.to_str()?;
    (!stem.is_empty()).then(|| stem.to_owned())
}
/// Name-table keys. r4 keeps float and spectrum textures in separate tables,
/// so the same name may denote one of each.
fn kind_key(kind: ResourceKind, texture: Option<TextureValueType>) -> u8 {
    match (kind, texture) {
        (ResourceKind::Material, _) => 0,
        (ResourceKind::Texture, Some(TextureValueType::Spectrum)) => 4,
        (ResourceKind::Texture, _) => 1,
        (ResourceKind::Medium, _) => 2,
        (ResourceKind::ObjectDefinition, _) => 3,
        (ResourceKind::SpectrumData, _) => 5,
    }
}
fn opens_scope(name: &str) -> bool {
    matches!(name, "AttributeBegin" | "TransformBegin" | "ObjectBegin")
}
fn closes_scope(name: &str) -> bool {
    matches!(name, "AttributeEnd" | "TransformEnd" | "ObjectEnd")
}
fn world_only(name: &str) -> bool {
    matches!(
        name,
        "Shape"
            | "LightSource"
            | "AreaLightSource"
            | "Material"
            | "MakeNamedMaterial"
            | "NamedMaterial"
            | "Texture"
            | "ObjectInstance"
            | "ReverseOrientation"
    )
}
fn options_only(name: &str) -> bool {
    matches!(
        name,
        "Camera"
            | "Film"
            | "Sampler"
            | "Integrator"
            | "PixelFilter"
            | "Accelerator"
            | "Option"
            | "TransformTimes"
    )
}
impl<F: Fn() -> bool> Builder<'_, '_, F> {
    fn lookup(&self, kind: ResourceKind, name: &str) -> Result<ResourceId> {
        let found = if kind == ResourceKind::Texture {
            let float = self.names.get(&(1, name.into()));
            let spectrum = self.names.get(&(4, name.into()));
            if float.is_some() && spectrum.is_some() {
                return Err(unsupported(format!(
                    "texture {name} is defined as both float and spectrum"
                )));
            }
            float.or(spectrum)
        } else {
            self.names.get(&(kind_key(kind, None), name.into()))
        };
        found.copied().ok_or_else(|| {
            unsupported(format!(
                "refers to \"{name}\", which is not defined earlier (or was skipped)"
            ))
        })
    }
    fn warn(&mut self, code: &str, message: String, origin: Option<Origin>) {
        self.sources
            .record
            .diagnostics
            .push(ImportDiagnostic::warning(code, message, origin));
    }
    /// Import one declaration. Input that cannot be interpreted is skipped
    /// with a warning; only cancellation, I/O and budget errors stop the import.
    fn import(&mut self, declaration: Declaration) -> Result<()> {
        let name = declaration.directive.name.clone();
        if let Some(skipped) = &mut self.skipped {
            skipped.directives += 1;
            if opens_scope(&name) {
                skipped.depth += 1;
            } else if closes_scope(&name) {
                skipped.depth -= 1;
                if skipped.depth == 0 {
                    self.flush_skipped();
                }
            }
            return Ok(());
        }
        let origin = declaration.origin.clone();
        let location = format!(
            "{}:{}",
            declaration.directive.path, declaration.directive.line
        );
        let naming = group_naming_candidate(&declaration.directive);
        self.added_assets.clear();
        self.added_spectra.clear();
        let state = self.state.clone();
        let error = match self.declaration(declaration) {
            Ok(()) => {
                self.name_open_scope(naming);
                return Ok(());
            }
            Err(Error::Unsupported(message)) => ("unsupported", message),
            Err(Error::Invalid(message)) => ("invalid", message),
            Err(error) => return Err(error),
        };
        self.state = state;
        for id in std::mem::take(&mut self.added_assets) {
            self.document.assets.remove(&id);
            self.assets.retain(|_, asset| *asset != id);
            self.object_assets.retain(|_, asset| *asset != id);
        }
        for id in std::mem::take(&mut self.added_spectra) {
            self.document.resources.remove(&id);
            self.spectra.retain(|_, resource| *resource != id);
        }
        let diagnostic = ImportDiagnostic::warning(
            error.0,
            format!("skipped {name} at {location}: {}", error.1),
            Some(origin),
        );
        if opens_scope(&name) {
            self.skipped = Some(SkippedScope {
                depth: 1,
                directives: 0,
                diagnostic,
            });
        } else {
            self.sources.record.diagnostics.push(diagnostic);
        }
        Ok(())
    }
    fn flush_skipped(&mut self) {
        if let Some(mut skipped) = self.skipped.take() {
            skipped.diagnostic.message += &format!(
                " (with the {} directive(s) of its scope)",
                skipped.directives
            );
            self.sources.record.diagnostics.push(skipped.diagnostic);
        }
    }
    /// Record the first name-able declaration set directly inside the
    /// innermost open scope; the first of each kind wins.
    fn name_open_scope(&mut self, naming: Option<GroupNaming>) {
        let (Some(naming), Some(frame)) = (naming, self.group_names.last_mut()) else {
            return;
        };
        match naming {
            GroupNaming::Material(name) => frame.material.get_or_insert(name),
            GroupNaming::Instance(name) => frame.instance.get_or_insert(name),
            GroupNaming::Plymesh(name) => frame.plymesh.get_or_insert(name),
        };
    }
    /// Name a closing scope whose label is still the default "Group":
    /// the first NamedMaterial set directly inside it, else the first
    /// ObjectInstance, else the first plymesh's file stem. Object
    /// definitions always arrive named and never take this route.
    fn name_closing_scope(&mut self, kind: ScopeKind, id: NodeId) {
        let Some(frame) = self.group_names.pop() else {
            return;
        };
        if kind == ScopeKind::ObjectDefinition {
            return;
        }
        let candidate = frame.material.or(frame.instance).or(frame.plymesh);
        if let (Some(name), Some(node)) = (candidate, self.node_mut(id))
            && node.label == "Group"
        {
            node.label = name;
        }
    }
    /// Bind the NamedMaterial directives whose material was only defined
    /// later in the file (pbrt-v4 binds named materials after the whole
    /// parse): every placeholder takes the definition the finished import
    /// ended with. A name that was never defined drops its binding with
    /// the warning the early lookup produced.
    fn resolve_pending_materials(&mut self) {
        let mut unresolved = std::collections::HashSet::new();
        for (node, name, location, origin) in std::mem::take(&mut self.pending_materials) {
            let defined = self
                .names
                .get(&(kind_key(ResourceKind::Material, None), name.clone()))
                .copied();
            if let Some(id) = defined {
                if let Some(record) = self.node_mut(node) {
                    record.data = NodeKind::State {
                        operation: StateOperation::Material(MaterialBinding::Resource(id)),
                    };
                }
            } else {
                unresolved.insert(node);
                self.warn(
                    "unsupported",
                    format!(
                        "skipped NamedMaterial at {location}: refers to \"{name}\", \
                         which is not defined earlier (or was skipped)"
                    ),
                    Some(origin),
                );
            }
        }
        if !unresolved.is_empty() {
            self.remove_bindings(&unresolved);
        }
    }
    /// Take the placeholder bindings whose material never materialized out
    /// of their parent scopes' children and the node table, in one pass
    /// however many there are.
    fn remove_bindings(&mut self, nodes: &std::collections::HashSet<NodeId>) {
        for scope in self.document.definition_nodes.values_mut() {
            if let NodeKind::Scope { children, .. } = &mut scope.data {
                children.retain(|child| !nodes.contains(child));
            }
        }
        let scene = std::sync::Arc::make_mut(
            self.document
                .scenes
                .get_mut(&self.scene)
                .expect("import scene exists"),
        );
        for scope in scene.nodes.values_mut() {
            if let NodeKind::Scope { children, .. } = &mut scope.data {
                children.retain(|child| !nodes.contains(child));
            }
        }
        for node in nodes {
            self.document.definition_nodes.remove(node);
            scene.nodes.remove(node);
        }
    }
    /// Close what the input left open, so a truncated scene still imports.
    fn finish(&mut self) {
        self.resolve_pending_materials();
        self.flush_skipped();
        while let Some((scope, id)) = self.scopes.pop() {
            let origin = self.node(id).and_then(|node| node.origin.clone());
            self.warn(
                "unbalanced_scope",
                format!("closed {scope:?} scope left open at the end of the input"),
                origin,
            );
            self.name_closing_scope(scope, id);
        }
        if !self.world_seen {
            self.warn(
                "missing_world",
                "the input has no WorldBegin; the world is empty".into(),
                None,
            );
        }
        let scene = &mut self.document.scenes.get_mut(&self.scene).unwrap();
        let scene = std::sync::Arc::make_mut(scene);
        if scene.render_camera.is_none() {
            scene.render_camera = scene.options.iter().rev().copied().find(|id| {
                matches!(
                    scene.nodes.get(id).map(|node| &node.data),
                    Some(NodeKind::Entity {
                        entity: EntityKind::Camera,
                        ..
                    })
                )
            });
        }
    }
    fn node_count(&self) -> usize {
        self.document.scenes[&self.scene].nodes.len() + self.document.definition_nodes.len()
    }
    fn set_scene_location(&mut self, location: String) {
        let scene = std::sync::Arc::make_mut(
            self.document
                .scenes
                .get_mut(&self.scene)
                .expect("import scene exists"),
        );
        scene.location = location;
    }
    fn node(&self, id: NodeId) -> Option<&Node> {
        self.document
            .scenes
            .get(&self.scene)
            .and_then(|scene| scene.nodes.get(&id))
            .or_else(|| self.document.definition_nodes.get(&id))
    }
    fn node_mut(&mut self, id: NodeId) -> Option<&mut Node> {
        if self
            .document
            .scenes
            .get(&self.scene)
            .is_some_and(|scene| scene.nodes.contains_key(&id))
        {
            let scene = std::sync::Arc::make_mut(
                self.document
                    .scenes
                    .get_mut(&self.scene)
                    .expect("import scene exists"),
            );
            return scene.nodes.get_mut(&id);
        }
        self.document.definition_nodes.get_mut(&id)
    }
    fn append(&mut self, node: Node) {
        let parent = self.scopes.last().map(|(_, id)| *id).or(self
            .in_world
            .then_some(self.document.scenes[&self.scene].world));
        if let Some(parent) = parent {
            let NodeKind::Scope { children, .. } =
                &mut self.node_mut(parent).expect("parent node exists").data
            else {
                unreachable!()
            };
            children.push(node.id);
        } else {
            std::sync::Arc::make_mut(
                self.document
                    .scenes
                    .get_mut(&self.scene)
                    .expect("import scene exists"),
            )
            .options
            .push(node.id);
        }
        if self.document.definition_nodes.contains_key(&node.id)
            || !self.document.scenes[&self.scene]
                .nodes
                .contains_key(&node.id)
                && self
                    .scopes
                    .iter()
                    .any(|(scope, _)| *scope == ScopeKind::ObjectDefinition)
        {
            self.document.definition_nodes.insert(node.id, node);
        } else {
            std::sync::Arc::make_mut(
                self.document
                    .scenes
                    .get_mut(&self.scene)
                    .expect("import scene exists"),
            )
            .nodes
            .insert(node.id, node);
        }
    }
    fn resource(&mut self, resource: Resource, bind: bool) {
        let id = resource.id;
        self.names.insert(
            (
                kind_key(resource.kind, resource.texture_value_type),
                resource.label.clone(),
            ),
            id,
        );
        let label = resource.label.clone();
        let origin = resource.origin.clone();
        self.document.resources.insert(id, resource);
        if bind {
            let mut node = Node::new(
                label,
                NodeKind::State {
                    operation: StateOperation::Material(MaterialBinding::Resource(id)),
                },
            );
            node.origin = origin;
            self.append(node);
            // The binding node is emitted through the tree; keep the import's
            // state mirror in step so later declarations (notably ObjectBegin)
            // capture the material that is actually active.
            self.state.material = MaterialBinding::Resource(id);
        }
    }
    fn capture_transform(&self) -> CapturedTransform {
        CapturedTransform {
            start: self.state.start.0,
            end: self.state.end.0,
            active: self.state.active,
            times: self.state.times,
        }
    }
    fn apply_state(&mut self, operation: &StateOperation) -> Result<()> {
        match operation {
            StateOperation::Transform(transform) => self.state.apply(transform)?,
            StateOperation::ActiveTransform(active) => self.state.active = *active,
            StateOperation::TransformTimes(times) => self.state.times = *times,
            StateOperation::Material(binding) => self.state.material = binding.clone(),
            StateOperation::Medium { inside, outside } => {
                self.state.inside_medium = *inside;
                self.state.outside_medium = *outside;
            }
            StateOperation::ReverseOrientation => {
                self.state.reverse_orientation = !self.state.reverse_orientation;
            }
            StateOperation::ColorSpace(name) => self.state.color_space = name.clone(),
            StateOperation::DefineCoordinates(name) => {
                self.coordinates
                    .insert(name.clone(), (self.state.start, self.state.end));
            }
            StateOperation::UseCoordinates(name) => {
                let (start, end) = *self
                    .coordinates
                    .get(name)
                    .ok_or_else(|| invalid("unknown coordinate system"))?;
                self.state.set(start, end, true)?;
            }
        }
        Ok(())
    }
    fn capture_graphics(&self) -> CapturedGraphics {
        CapturedGraphics {
            material: self.state.material.clone(),
            inside_medium: self.state.inside_medium,
            outside_medium: self.state.outside_medium,
            reverse_orientation: self.state.reverse_orientation,
            color_space: self.state.color_space.clone(),
        }
    }
    /// Only the color space matters for material parameter parsing.
    fn capture_color_space(&self) -> CapturedGraphics {
        CapturedGraphics {
            material: MaterialBinding::Default,
            inside_medium: None,
            outside_medium: None,
            reverse_orientation: false,
            color_space: self.state.color_space.clone(),
        }
    }
    fn declaration(&mut self, declaration: Declaration) -> Result<()> {
        let Declaration {
            directive: d,
            origin,
            search_dirs,
        } = declaration;
        if matches!(
            d.name.as_str(),
            "WorldBegin"
                | "WorldEnd"
                | "AttributeBegin"
                | "AttributeEnd"
                | "TransformBegin"
                | "TransformEnd"
                | "ObjectBegin"
                | "ObjectEnd"
        ) && !d.parameters.is_empty()
        {
            return Err(unsupported("parameters on a scope delimiter"));
        }
        let text = d.positional_text(0).unwrap_or("").to_owned();
        let numbers = || -> Result<Vec<f64>> {
            d.positional
                .iter()
                .map(|v| {
                    v.as_float()
                        .filter(|n| n.is_finite())
                        .ok_or_else(|| invalid("nonfinite transform"))
                })
                .collect()
        };
        let state = |op| NodeKind::State { operation: op };
        if self.in_world && options_only(&d.name) {
            return Err(invalid(format!(
                "{} is only allowed before WorldBegin",
                d.name
            )));
        }
        if !self.in_world && !self.world_ended && world_only(&d.name) {
            return Err(invalid(format!(
                "{} is only allowed after WorldBegin",
                d.name
            )));
        }
        let in_definition = self
            .scopes
            .iter()
            .any(|(scope, _)| *scope == ScopeKind::ObjectDefinition);
        if in_definition && matches!(d.name.as_str(), "ObjectBegin" | "ObjectInstance") {
            return Err(unsupported(format!(
                "{} inside an object definition",
                d.name
            )));
        }
        let data = match d.name.as_str() {
            "WorldBegin" => {
                if self.world_seen || !self.scopes.is_empty() || !d.parameters.is_empty() {
                    return Err(invalid("invalid WorldBegin"));
                }
                self.world_seen = true;
                self.in_world = true;
                self.state.start = project::math::Mat4::identity();
                self.state.end = project::math::Mat4::identity();
                self.state.active = ActiveSamples::All;
                self.coordinates
                    .insert("world".into(), (self.state.start, self.state.end));
                let world = self.document.scenes[&self.scene].world;
                self.node_mut(world).unwrap().origin = Some(origin);
                return Ok(());
            }
            "WorldEnd" => {
                if !self.in_world {
                    return Err(invalid("WorldEnd without WorldBegin"));
                }
                while let Some((scope, id)) = self.scopes.pop() {
                    let origin = self.node(id).and_then(|node| node.origin.clone());
                    self.warn(
                        "unbalanced_scope",
                        format!("closed {scope:?} scope left open at WorldEnd"),
                        origin,
                    );
                    self.name_closing_scope(scope, id);
                }
                self.state_stack.clear();
                self.in_world = false;
                self.world_ended = true;
                return Ok(());
            }
            _ if self.world_ended => return Err(invalid("directive after WorldEnd")),
            "AttributeBegin" | "TransformBegin" | "ObjectBegin" => {
                if !self.in_world {
                    return Err(invalid("scope outside World"));
                }
                let scope = match d.name.as_str() {
                    "AttributeBegin" => ScopeKind::Attribute,
                    "TransformBegin" => ScopeKind::Transform,
                    _ => ScopeKind::ObjectDefinition,
                };
                if self.scopes.len() >= 255 {
                    return Err(project::ProjectError::ResourceLimit("scope depth".into()).into());
                }
                // ObjectBegin names its scope; AttributeBegin/TransformBegin
                // carry no name and take Create ▸ Group's label.
                let mut node = Node::new(
                    if text.is_empty() {
                        "Group".to_owned()
                    } else {
                        text.clone()
                    },
                    NodeKind::Scope {
                        scope,
                        children: vec![],
                        owned_object: None,
                        parent_camera: None,
                    },
                );
                node.origin = Some(origin.clone());
                let id = node.id;
                if scope == ScopeKind::ObjectDefinition {
                    if text.is_empty() || self.names.contains_key(&(3, text.clone())) {
                        // r4 keeps the first definition of a name.
                        return Err(invalid("missing or duplicate object name"));
                    }
                    let location = self.layout.take("Objects", &text, "object");
                    self.document.definition_nodes.insert(id, node);
                    self.resource(
                        Resource {
                            id: ResourceId::new(),
                            label: text,
                            location,
                            edition: Revision::initial(),
                            kind: ResourceKind::ObjectDefinition,
                            implementation: String::new(),
                            parameters: vec![],
                            texture_value_type: None,
                            definition_scope: Some(id),
                            declared_in: Some(DeclarationState {
                                transform: Some(self.capture_transform()),
                                graphics: Some(self.capture_graphics()),
                            }),
                            origin: Some(origin.clone()),
                        },
                        false,
                    );
                } else {
                    self.append(node);
                }
                self.state_stack.push(self.state.clone());
                self.scopes.push((scope, id));
                self.group_names.push(ScopeNaming::default());
                return Ok(());
            }
            "AttributeEnd" | "TransformEnd" | "ObjectEnd" => {
                let expected = match d.name.as_str() {
                    "AttributeEnd" => ScopeKind::Attribute,
                    "TransformEnd" => ScopeKind::Transform,
                    _ => ScopeKind::ObjectDefinition,
                };
                if self
                    .scopes
                    .last()
                    .is_none_or(|(scope, _)| *scope != expected)
                {
                    return Err(invalid("scope end does not match an open scope"));
                }
                let (_, id) = self.scopes.pop().expect("the scope matched");
                self.name_closing_scope(expected, id);
                if let Some(previous) = self.state_stack.pop() {
                    self.state = previous;
                }
                return Ok(());
            }
            "Identity" => state(StateOperation::Transform(TransformOperation::Identity)),
            "Translate" | "Scale" => {
                let n = numbers()?;
                let v = [n[0], n[1], n[2]];
                state(StateOperation::Transform(if d.name == "Translate" {
                    TransformOperation::Translate(v)
                } else {
                    TransformOperation::Scale(v)
                }))
            }
            "Rotate" => {
                let n = numbers()?;
                state(StateOperation::Transform(TransformOperation::Rotate {
                    degrees: n[0],
                    axis: [n[1], n[2], n[3]],
                }))
            }
            "Transform" | "ConcatTransform" => {
                let n = numbers()?;
                let m = std::array::from_fn(|i| n[(i % 4) * 4 + i / 4]);
                state(StateOperation::Transform(if d.name == "Transform" {
                    TransformOperation::Replace(m)
                } else {
                    TransformOperation::Concat(m)
                }))
            }
            "LookAt" => {
                let n: [f64; 9] = numbers()?.try_into().map_err(|_| invalid("LookAt arity"))?;
                if project::math::look_at(&n).is_none() {
                    return Err(invalid("degenerate LookAt"));
                }
                state(StateOperation::Transform(TransformOperation::LookAt(n)))
            }
            "ActiveTransform" => state(StateOperation::ActiveTransform(match text.as_str() {
                "StartTime" => ActiveSamples::Start,
                "EndTime" => ActiveSamples::End,
                "All" => ActiveSamples::All,
                _ => return Err(invalid("unknown ActiveTransform selector")),
            })),
            "TransformTimes" => {
                let n = numbers()?;
                state(StateOperation::TransformTimes([n[0], n[1]]))
            }
            "ReverseOrientation" => state(StateOperation::ReverseOrientation),
            "ColorSpace" => state(StateOperation::ColorSpace(text)),
            "CoordinateSystem" => state(StateOperation::DefineCoordinates(text)),
            "CoordSysTransform" => {
                if !self.coordinates.contains_key(&text) {
                    return Err(invalid(format!("unknown coordinate system {text}")));
                }
                state(StateOperation::UseCoordinates(text))
            }
            "NamedMaterial" => {
                match self
                    .names
                    .get(&(kind_key(ResourceKind::Material, None), text.clone()))
                    .copied()
                {
                    Some(id) => state(StateOperation::Material(MaterialBinding::Resource(id))),
                    None => {
                        // pbrt-v4 resolves NamedMaterial after the whole
                        // file is parsed, so a material defined later in the
                        // same import still binds (contemporary-bathroom's
                        // "light" arrives through a later Include). Emit the
                        // binding now with a placeholder and patch it in
                        // `finish`; a name that is never defined drops there
                        // with the very warning the early lookup gave. The
                        // state mirror deliberately keeps the current
                        // material: only an ObjectBegin in this scope would
                        // capture graphics before the patch, and nothing
                        // binds named materials that late in object
                        // definitions.
                        let mut node = Node::new(
                            d.name.clone(),
                            NodeKind::State {
                                operation: StateOperation::Material(MaterialBinding::Default),
                            },
                        );
                        node.origin = Some(origin.clone());
                        let id = node.id;
                        self.append(node);
                        let location = format!("{}:{}", d.path, d.line);
                        self.pending_materials
                            .push((id, text.clone(), location, origin));
                        return Ok(());
                    }
                }
            }
            "MediumInterface" => {
                // pbrt-v4 binds both sides with one name (parser.cpp): a
                // missing second argument means inside = outside.
                let inside = if text.is_empty() {
                    None
                } else {
                    Some(self.lookup(ResourceKind::Medium, &text)?)
                };
                let outside = match d.positional_text(1) {
                    Some(outside) if !outside.is_empty() => {
                        Some(self.lookup(ResourceKind::Medium, outside)?)
                    }
                    Some(_) => None,
                    None => inside,
                };
                state(StateOperation::Medium { inside, outside })
            }
            "Material" if matches!(text.as_str(), "" | "interface") => {
                if !d.parameters.is_empty() {
                    return Err(unsupported("interface material parameters"));
                }
                state(StateOperation::Material(MaterialBinding::None))
            }
            "Material" | "MakeNamedMaterial" | "MakeNamedMedium" | "Texture" => {
                let kind = match d.name.as_str() {
                    "Texture" => ResourceKind::Texture,
                    "MakeNamedMedium" => ResourceKind::Medium,
                    _ => ResourceKind::Material,
                };
                let implementation = if d.name == "Texture" {
                    d.positional_text(2).unwrap().to_owned()
                } else if d.name == "Material" {
                    text.clone()
                } else {
                    let types = d
                        .parameters
                        .iter()
                        .filter(|p| p.name == "type")
                        .collect::<Vec<_>>();
                    if types.len() != 1
                        || types[0].value_type != "string"
                        || types[0].values.len() != 1
                    {
                        return Err(invalid("named resource requires one string type"));
                    }
                    types[0].values[0]
                        .as_text()
                        .ok_or_else(|| invalid("resource type must be a string"))?
                        .into()
                };
                let texture_type = if kind == ResourceKind::Texture {
                    Some(match d.positional_text(1).unwrap() {
                        "float" => TextureValueType::Float,
                        // r4 accepts `color` and `rgb` as spectrum textures
                        // (parse_target_impl.rs); export writes `spectrum`.
                        "spectrum" | "color" | "rgb" => TextureValueType::Spectrum,
                        _ => return Err(unsupported("texture value type")),
                    })
                } else {
                    None
                };
                let mut params = self.parameters(&d, &search_dirs, &origin, &implementation)?;
                if matches!(d.name.as_str(), "MakeNamedMaterial" | "MakeNamedMedium") {
                    params.retain(|p| p.name != "type");
                }
                crate::schema::declaration(&d.name, &implementation, &params)?;
                let location = self.layout.take(
                    match kind {
                        ResourceKind::Material => "Materials",
                        ResourceKind::Texture => "Textures",
                        ResourceKind::Medium => "Media",
                        // PBRT has no spectrum declaration; spectrum data
                        // resources come from file parameters, not directives.
                        ResourceKind::SpectrumData | ResourceKind::ObjectDefinition => {
                            unreachable!()
                        }
                    },
                    &text,
                    match kind {
                        ResourceKind::Material => "material",
                        ResourceKind::Texture => "texture",
                        ResourceKind::Medium => "medium",
                        ResourceKind::SpectrumData | ResourceKind::ObjectDefinition => {
                            unreachable!()
                        }
                    },
                );
                // r4 parses material parameters in the color space current at
                // the declaration, so a material keeps that color space (and
                // nothing else) for export and evaluation.
                let declared_in = if kind == ResourceKind::Material {
                    (self.state.color_space != "srgb").then(|| DeclarationState {
                        transform: None,
                        graphics: Some(self.capture_color_space()),
                    })
                } else {
                    Some(DeclarationState {
                        transform: Some(self.capture_transform()),
                        graphics: None,
                    })
                };
                self.resource(
                    Resource {
                        id: ResourceId::new(),
                        label: text,
                        location,
                        edition: Revision::initial(),
                        kind,
                        implementation,
                        parameters: params,
                        texture_value_type: texture_type,
                        definition_scope: None,
                        declared_in,
                        origin: Some(origin.clone()),
                    },
                    d.name == "Material",
                );
                return Ok(());
            }
            "ObjectInstance" => NodeKind::Instance {
                definition: self.lookup(ResourceKind::ObjectDefinition, &text)?,
            },
            "Camera" | "Shape" | "LightSource" | "AreaLightSource" => {
                let parameters = self.parameters(&d, &search_dirs, &origin, &text)?;
                crate::schema::declaration(&d.name, &text, &parameters)?;
                NodeKind::Entity {
                    entity: match d.name.as_str() {
                        "Camera" => EntityKind::Camera,
                        "Shape" => EntityKind::Shape,
                        "LightSource" => EntityKind::Light,
                        _ => EntityKind::AreaLight,
                    },
                    implementation: text,
                    parameters,
                    pose: None,
                }
            }
            "Film" | "Sampler" | "Integrator" | "PixelFilter" | "Accelerator" => {
                let parameters = self.parameters(&d, &search_dirs, &origin, &text)?;
                crate::schema::declaration(&d.name, &text, &parameters)?;
                NodeKind::Configuration {
                    configuration: match d.name.as_str() {
                        "Film" => ConfigurationKind::Film,
                        "Sampler" => ConfigurationKind::Sampler,
                        "Integrator" => ConfigurationKind::Integrator,
                        "PixelFilter" => ConfigurationKind::PixelFilter,
                        _ => ConfigurationKind::Accelerator,
                    },
                    implementation: text,
                    parameters,
                }
            }
            "Option" => {
                if d.positional_tokens.len() != 2 || !d.parameters.is_empty() {
                    return Err(unsupported("Option arity"));
                }
                // pbrt-v4 writes `Option "bool name" value`; r4 reads the bare name.
                let name = match text.split_once(char::is_whitespace) {
                    Some(("bool" | "integer" | "float" | "string", name)) => name.trim().to_owned(),
                    _ => text,
                };
                let value = crate::schema::parse_option(&name, &d.positional_tokens[1])?;
                NodeKind::Option { name, value }
            }
            _ => return Err(unsupported(format!("{} at {}:{}", d.name, d.path, d.line))),
        };
        if !matches!(
            &data,
            NodeKind::Entity { .. } | NodeKind::Configuration { .. }
        ) && !d.parameters.is_empty()
        {
            return Err(unsupported(format!("parameters on {}", d.name)));
        }
        let mut node = Node::new(d.name, data);
        node.origin = Some(origin);
        if matches!(
            &node.data,
            NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            }
        ) {
            // Mirrors the evaluator: the camera coordinate system is the
            // inverse of the transform current at the Camera declaration.
            let start = project::math::inverse(self.state.start)
                .ok_or_else(|| invalid("singular camera transform"))?;
            let end = project::math::inverse(self.state.end)
                .ok_or_else(|| invalid("singular camera transform"))?;
            self.coordinates.insert("camera".into(), (start, end));
        }
        if let NodeKind::State { operation } = &node.data {
            self.apply_state(operation)?;
        }
        self.append(node);
        Ok(())
    }
}
