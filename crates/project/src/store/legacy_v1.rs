//! Format 1 (generation store) reader. The old record types are frozen here so
//! the checked-in generation files keep their original JSON shape; the reader
//! and converts them into the new model.
use super::*;
use std::collections::{BTreeMap, BTreeSet};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyDocument {
    id: ProjectId,
    title: String,
    scene: LegacyScene,
    assets: BTreeMap<AssetId, LegacyAsset>,
    imports: BTreeMap<ImportId, ImportRecord>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyScene {
    options: Vec<NodeId>,
    world: NodeId,
    nodes: BTreeMap<NodeId, LegacyNode>,
    resources: BTreeMap<ResourceId, LegacyResource>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyNode {
    id: NodeId,
    label: String,
    edition: Revision,
    origin: Option<Origin>,
    data: LegacyNodeKind,
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
enum LegacyNodeKind {
    Scope {
        scope: ScopeKind,
        children: Vec<NodeId>,
    },
    State {
        operation: StateOperation,
    },
    Entity {
        entity: EntityKind,
        implementation: String,
        parameters: Vec<LegacyParameter>,
    },
    Instance {
        definition: ResourceId,
    },
    ResourceDeclaration {
        resource: ResourceId,
        bind_material: bool,
    },
    Configuration {
        configuration: ConfigurationKind,
        implementation: String,
        parameters: Vec<LegacyParameter>,
    },
    Option {
        name: String,
        value_token: String,
    },
    Retained {
        directive: String,
        raw: Origin,
        reason: String,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyResource {
    id: ResourceId,
    label: String,
    edition: Revision,
    kind: ResourceKind,
    texture_value_type: Option<TextureValueType>,
    implementation: String,
    parameters: Vec<LegacyParameter>,
    definition_scope: Option<NodeId>,
    origin: Option<Origin>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyAsset {
    id: AssetId,
    logical_path: String,
    media_type: String,
    object: ObjectRef,
    origin: Option<Origin>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyParameter {
    value_type: String,
    name: String,
    color_space: ColorSpaceBinding,
    values: LegacyParameterValues,
}

#[derive(Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
enum LegacyParameterValues {
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
        object: ObjectRef,
        encoding: ArrayEncoding,
        count: u64,
    },
}

#[derive(Serialize, Deserialize)]
#[serde(tag = "record", rename_all = "snake_case", deny_unknown_fields)]
enum LegacySceneRecord {
    Scene {
        project_id: ProjectId,
        title: String,
        options: Vec<NodeId>,
        world: NodeId,
    },
    Node {
        node: LegacyNode,
    },
    Resource {
        resource: LegacyResource,
    },
}

pub(super) fn read(generation: &Path, manifest: &Manifest) -> Result<ProjectDocument> {
    let read = |name: &str| -> Result<File> {
        verified_file(
            &generation.join(name),
            &manifest.files[name],
            MAX_METADATA_BYTES,
        )
    };
    let mut scene = BufReader::new(read("scene.jsonl")?);
    let mut header = None;
    let mut nodes = BTreeMap::new();
    let mut resources = BTreeMap::new();
    let mut number = 0usize;
    let mut line = Vec::new();
    loop {
        line.clear();
        let count = Read::by_ref(&mut scene)
            .take(MAX_RECORD_BYTES + 2)
            .read_until(b'\n', &mut line)?;
        if count == 0 {
            break;
        }
        if line.last() == Some(&b'\n') {
            line.pop();
        }
        if line.len() as u64 > MAX_RECORD_BYTES {
            return Err(ProjectError::ResourceLimit("project scene record".into()));
        }
        match serde_json::from_slice::<LegacySceneRecord>(&line)? {
            LegacySceneRecord::Scene {
                project_id,
                title,
                options,
                world,
            } if number == 0 => header = Some((project_id, title, options, world)),
            LegacySceneRecord::Scene { .. } => {
                return Err(invalid("duplicate or misplaced scene header"));
            }
            LegacySceneRecord::Node { node } => {
                if nodes.insert(node.id, node).is_some() {
                    return Err(invalid("duplicate node id"));
                }
            }
            LegacySceneRecord::Resource { resource } => {
                if resources.insert(resource.id, resource).is_some() {
                    return Err(invalid("duplicate resource id"));
                }
            }
        }
        number += 1;
    }
    let (id, title, options, world) = header.ok_or_else(|| invalid("missing scene header"))?;
    if id != manifest.project_id {
        return Err(invalid("manifest/scene project id mismatch"));
    }
    let assets: BTreeMap<AssetId, LegacyAsset> =
        serde_json::from_reader(BufReader::new(read("assets.json")?))?;
    let imports: BTreeMap<ImportId, ImportRecord> =
        serde_json::from_reader(BufReader::new(read("imports.json")?))?;
    let legacy = LegacyDocument {
        id,
        title,
        scene: LegacyScene {
            options,
            world,
            nodes,
            resources,
        },
        assets,
        imports,
    };
    if fingerprint(&legacy)? != manifest.model_sha256 {
        return Err(invalid("project model digest mismatch"));
    }
    convert(legacy)
}

fn warn(imports: &mut BTreeMap<ImportId, ImportRecord>, code: &str, message: String) {
    let Some(import) = imports.values_mut().next() else {
        return;
    };
    import.diagnostics.push(ImportDiagnostic {
        code: code.into(),
        message,
        severity: None,
        origin: None,
    });
}

struct Layout {
    used: BTreeSet<String>,
}
impl Layout {
    fn new() -> Self {
        Self {
            used: BTreeSet::new(),
        }
    }
    fn take(&mut self, folder: &str, stem: &str, extension: &str) -> String {
        let stem = safe_name(stem, fallback_stem(extension));
        let mut candidate = format!("{folder}/{stem}.{extension}");
        let mut number = 1;
        while !self.used.insert(candidate.clone()) {
            candidate = format!("{folder}/{stem}_{number}.{extension}");
            number += 1;
        }
        candidate
    }
}

fn fallback_stem(extension: &str) -> &'static str {
    match extension {
        "material" => "material",
        "texture" => "texture",
        "medium" => "medium",
        "object" => "object",
        "scene" => "scene",
        "array" => "array",
        _ => "file",
    }
}

fn safe_name(name: &str, fallback: &str) -> String {
    let mut safe = String::new();
    for c in name.chars() {
        if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control() {
            safe.push('_');
        } else {
            safe.push(c);
        }
    }
    let safe = safe.trim().trim_end_matches('.').trim().to_owned();
    if safe.is_empty() || safe == "." || safe == ".." {
        fallback.into()
    } else {
        safe
    }
}

fn resource_extension(kind: ResourceKind) -> &'static str {
    match kind {
        ResourceKind::Material => "material",
        ResourceKind::Texture => "texture",
        ResourceKind::Medium => "medium",
        _ => "object",
    }
}

fn asset_media_type(path: &str) -> &'static str {
    match Path::new(path)
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
        // nanovdb grid payloads: medium data, Media/ like a fresh
        // import's placement.
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
        _ => "Textures",
    }
}

fn basename(path: &str) -> &str {
    // The stem, so `take` does not append the extension twice; this is the
    // pbrt-io importer's convention (`import.rs::basename`).
    Path::new(path)
        .file_stem()
        .and_then(|n| n.to_str())
        .unwrap_or("")
}

fn extension_of(path: &str) -> &str {
    Path::new(path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("bin")
}

fn convert(legacy: LegacyDocument) -> Result<ProjectDocument> {
    let title = legacy.title.clone();
    let mut retained = Vec::new();
    for node in legacy.scene.nodes.values() {
        if let LegacyNodeKind::Retained {
            directive, reason, ..
        } = &node.data
        {
            retained.push(format!("{directive}: {reason}"));
        }
    }
    let states = evaluate_legacy(&legacy)?;

    let scene_id = SceneId::new();
    let mut layout = Layout::new();
    let scene_location = layout.take("Scenes", &title, "scene");
    let mut folders = crate::default_folders();

    let mut assets = BTreeMap::new();
    let mut asset_by_object = BTreeMap::<ObjectRef, AssetId>::new();
    for old in legacy.assets.into_values() {
        if asset_by_object.contains_key(&old.object) {
            // The same content is one asset, shared by every reference.
            continue;
        }
        let media_type = asset_media_type(&old.logical_path);
        let location = layout.take(
            asset_folder(media_type),
            basename(&old.logical_path),
            extension_of(&old.logical_path),
        );
        let id = old.id;
        assets.insert(
            id,
            Asset {
                id,
                location,
                media_type: media_type.into(),
                sha256: old.object.sha256.clone(),
                size: old.object.size,
                origin: Some(AssetOrigin {
                    logical_path: old.logical_path.clone(),
                    origin: old.origin,
                }),
            },
        );
        asset_by_object.insert(old.object, id);
    }

    // Object-definition bodies live in `definition_nodes` from now on.
    let mut definition_owned = BTreeSet::new();
    for resource in legacy.scene.resources.values() {
        let Some(root) = resource.definition_scope else {
            continue;
        };
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            if !definition_owned.insert(id) {
                return Err(invalid("definition node owned more than once"));
            }
            let node = legacy
                .scene
                .nodes
                .get(&id)
                .ok_or_else(|| invalid("missing definition node"))?;
            if let LegacyNodeKind::Scope { children, .. } = &node.data {
                stack.extend(children.iter().rev().copied());
            }
        }
    }

    // Unbound declarations and retained directives leave the tree. Bound
    // Material declarations become a `StateOperation::Material` in place.
    let mut removed = BTreeSet::new();
    for (id, node) in &legacy.scene.nodes {
        match &node.data {
            LegacyNodeKind::ResourceDeclaration {
                bind_material: false,
                ..
            }
            | LegacyNodeKind::Retained { .. } => {
                removed.insert(*id);
            }
            _ => {}
        }
    }

    let mut scene_nodes = BTreeMap::new();
    let mut stack = legacy
        .scene
        .options
        .iter()
        .copied()
        .chain([legacy.scene.world])
        .collect::<Vec<_>>();
    while let Some(id) = stack.pop() {
        let old = legacy
            .scene
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("missing scene node"))?;
        let Some(data) = convert_node_data(
            &old.data,
            &removed,
            &mut assets,
            &mut asset_by_object,
            &mut layout,
        )?
        else {
            continue;
        };
        if let LegacyNodeKind::Scope { children, .. } = &old.data {
            stack.extend(children.iter().rev().copied());
        }
        scene_nodes.insert(
            id,
            Node {
                id,
                label: old.label.clone(),
                edition: old.edition.clone(),
                origin: old.origin.clone(),
                active: true,
                data,
            },
        );
    }

    let mut definition_nodes = BTreeMap::new();
    for root in legacy
        .scene
        .resources
        .values()
        .filter_map(|r| r.definition_scope)
    {
        let mut stack = vec![root];
        while let Some(id) = stack.pop() {
            let old = legacy
                .scene
                .nodes
                .get(&id)
                .ok_or_else(|| invalid("missing definition node"))?;
            let data = convert_node_data(
                &old.data,
                &removed,
                &mut assets,
                &mut asset_by_object,
                &mut layout,
            )?
            .ok_or_else(|| invalid("definition root cannot be dropped"))?;
            if let LegacyNodeKind::Scope { children, .. } = &old.data {
                stack.extend(children.iter().rev().copied());
            }
            definition_nodes.insert(
                id,
                Node {
                    id,
                    label: old.label.clone(),
                    edition: old.edition.clone(),
                    origin: old.origin.clone(),
                    active: true,
                    data,
                },
            );
        }
    }
    if scene_nodes.len() + definition_nodes.len() != legacy.scene.nodes.len() {
        return Err(invalid("unreachable nodes in a format 1 project"));
    }

    let options: Vec<NodeId> = legacy
        .scene
        .options
        .iter()
        .filter(|id| scene_nodes.contains_key(id))
        .copied()
        .collect();

    let mut resources = BTreeMap::new();
    for (id, old) in legacy.scene.resources {
        // Format 1 predates spectrum data; a project claiming one is not a
        // legacy project but a corrupt or hand-built file.
        if old.kind == ResourceKind::SpectrumData {
            return Err(invalid("format 1 projects cannot hold spectrum data"));
        }
        let location = layout.take(
            match old.kind {
                ResourceKind::Material => "Materials",
                ResourceKind::Texture => "Textures",
                ResourceKind::Medium => "Media",
                _ => "Objects",
            },
            &old.label,
            resource_extension(old.kind),
        );
        let parameters = old
            .parameters
            .into_iter()
            .map(|p| to_parameter(p, &mut assets, &mut asset_by_object, &mut layout))
            .collect();
        let declared_in = states.get(&id).map(|state| DeclarationState {
            transform: matches!(
                old.kind,
                ResourceKind::Texture | ResourceKind::Medium | ResourceKind::ObjectDefinition
            )
            .then(|| CapturedTransform {
                start: state.start.0,
                end: state.end.0,
                active: state.active,
                times: state.times,
            }),
            graphics: match old.kind {
                ResourceKind::ObjectDefinition => Some(CapturedGraphics {
                    material: state.material.clone(),
                    inside_medium: state.inside_medium,
                    outside_medium: state.outside_medium,
                    reverse_orientation: state.reverse_orientation,
                    color_space: state.color_space.clone(),
                }),
                // Only the declaration color space affects material parameters.
                ResourceKind::Material => Some(CapturedGraphics {
                    material: MaterialBinding::Default,
                    inside_medium: None,
                    outside_medium: None,
                    reverse_orientation: false,
                    color_space: state.color_space.clone(),
                }),
                _ => None,
            },
        });
        resources.insert(
            id,
            Resource {
                id,
                label: old.label,
                location,
                edition: old.edition,
                kind: old.kind,
                texture_value_type: old.texture_value_type,
                implementation: old.implementation,
                parameters,
                definition_scope: old.definition_scope,
                declared_in,
                origin: old.origin,
            },
        );
    }

    let mut imports = legacy.imports;
    for reason in retained {
        warn(
            &mut imports,
            "legacy_retained_dropped",
            format!("dropped a retained directive while reading a format 1 project: {reason}"),
        );
    }

    for location in assets
        .values()
        .map(|a| a.location.as_str())
        .chain(resources.values().map(|r| r.location.as_str()))
    {
        let mut parent = Path::new(location).parent();
        while let Some(p) = parent {
            if p.as_os_str().is_empty() {
                break;
            }
            folders.insert(p.to_string_lossy().into_owned());
            parent = p.parent();
        }
    }

    let render_camera = options.iter().rev().copied().find(|id| {
        matches!(
            scene_nodes.get(id).map(|node| &node.data),
            Some(NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            })
        )
    });
    // Legacy projects predate poses and the coordinate system alike; the
    // Legacy projects use the same coordinate-system inference as fresh
    // imports, with the inferred default applied here.
    let coordinate_system =
        coordinate::infer_cameras(&options, &scene_nodes, render_camera).coordinate_system;
    Ok(ProjectDocument {
        id: legacy.id,
        title,
        default_scene: Some(scene_id),
        scenes: BTreeMap::from([(
            scene_id,
            std::sync::Arc::new(SceneDocument {
                id: scene_id,
                label: legacy.title,
                location: scene_location,
                options,
                render_camera,
                coordinate_system,
                world: legacy.scene.world,
                nodes: scene_nodes,
            }),
        )]),
        resources,
        definition_nodes,
        assets,
        folders,
        imports,
    })
}

/// `None` means the node is dropped (retained or an unbound declaration).
fn convert_node_data(
    data: &LegacyNodeKind,
    removed: &BTreeSet<NodeId>,
    assets: &mut BTreeMap<AssetId, Asset>,
    object_assets: &mut BTreeMap<ObjectRef, AssetId>,
    layout: &mut Layout,
) -> Result<Option<NodeKind>> {
    Ok(Some(match data {
        LegacyNodeKind::Scope { scope, children } => NodeKind::Scope {
            scope: *scope,
            children: children
                .iter()
                .copied()
                .filter(|id| !removed.contains(id))
                .collect(),
            owned_object: None,
            parent_camera: None,
        },
        LegacyNodeKind::State { operation } => NodeKind::State {
            operation: operation.clone(),
        },
        LegacyNodeKind::Entity {
            entity,
            implementation,
            parameters,
        } => NodeKind::Entity {
            entity: *entity,
            implementation: implementation.clone(),
            parameters: parameters
                .iter()
                .map(|p| to_parameter_ref(p, assets, object_assets, layout))
                .collect::<Result<_>>()?,
            pose: None,
        },
        LegacyNodeKind::Instance { definition } => NodeKind::Instance {
            definition: *definition,
        },
        LegacyNodeKind::ResourceDeclaration {
            resource,
            bind_material,
        } => {
            if *bind_material {
                NodeKind::State {
                    operation: StateOperation::Material(MaterialBinding::Resource(*resource)),
                }
            } else {
                return Ok(None);
            }
        }
        LegacyNodeKind::Configuration {
            configuration,
            implementation,
            parameters,
        } => NodeKind::Configuration {
            configuration: *configuration,
            implementation: implementation.clone(),
            parameters: parameters
                .iter()
                .map(|p| to_parameter_ref(p, assets, object_assets, layout))
                .collect::<Result<_>>()?,
        },
        LegacyNodeKind::Option { name, value_token } => NodeKind::Option {
            name: name.clone(),
            value: option_value(value_token)
                .ok_or_else(|| invalid("legacy Option value cannot be typed"))?,
        },
        LegacyNodeKind::Retained { .. } => return Ok(None),
    }))
}

fn to_parameter(
    p: LegacyParameter,
    assets: &mut BTreeMap<AssetId, Asset>,
    object_assets: &mut BTreeMap<ObjectRef, AssetId>,
    layout: &mut Layout,
) -> Parameter {
    Parameter {
        value_type: p.value_type,
        name: p.name,
        color_space: p.color_space,
        values: match p.values {
            LegacyParameterValues::Floats(v) => ParameterValues::Floats(v),
            LegacyParameterValues::Integers(v) => ParameterValues::Integers(v),
            LegacyParameterValues::Booleans(v) => ParameterValues::Booleans(v),
            LegacyParameterValues::Strings(v) => ParameterValues::Strings(v),
            LegacyParameterValues::BuiltinSpectra(v) => ParameterValues::BuiltinSpectra(v),
            LegacyParameterValues::Assets(v) => ParameterValues::Assets(v),
            LegacyParameterValues::Resources { kind, ids } => {
                ParameterValues::Resources { kind, ids }
            }
            LegacyParameterValues::Array {
                object,
                encoding,
                count,
            } => ParameterValues::Array {
                asset: array_asset(object, assets, object_assets, layout),
                encoding,
                count,
            },
        },
    }
}

fn to_parameter_ref(
    p: &LegacyParameter,
    assets: &mut BTreeMap<AssetId, Asset>,
    object_assets: &mut BTreeMap<ObjectRef, AssetId>,
    layout: &mut Layout,
) -> Result<Parameter> {
    let clone = LegacyParameter {
        value_type: p.value_type.clone(),
        name: p.name.clone(),
        color_space: p.color_space.clone(),
        values: match &p.values {
            LegacyParameterValues::Floats(v) => LegacyParameterValues::Floats(v.clone()),
            LegacyParameterValues::Integers(v) => LegacyParameterValues::Integers(v.clone()),
            LegacyParameterValues::Booleans(v) => LegacyParameterValues::Booleans(v.clone()),
            LegacyParameterValues::Strings(v) => LegacyParameterValues::Strings(v.clone()),
            LegacyParameterValues::BuiltinSpectra(v) => {
                LegacyParameterValues::BuiltinSpectra(v.clone())
            }
            LegacyParameterValues::Assets(v) => LegacyParameterValues::Assets(v.clone()),
            LegacyParameterValues::Resources { kind, ids } => LegacyParameterValues::Resources {
                kind: *kind,
                ids: ids.clone(),
            },
            LegacyParameterValues::Array {
                object,
                encoding,
                count,
            } => LegacyParameterValues::Array {
                object: object.clone(),
                encoding: *encoding,
                count: *count,
            },
        },
    };
    validate_array_parameters(&clone)?;
    Ok(to_parameter(clone, assets, object_assets, layout))
}

/// Node parameters were never written with arrays by the format-1 writer; keep
/// that invariant explicit instead of silently losing data.
fn validate_array_parameters(p: &LegacyParameter) -> Result<()> {
    if matches!(p.values, LegacyParameterValues::Array { .. }) {
        return Err(invalid("legacy node parameter holds an array"));
    }
    Ok(())
}

fn array_asset(
    object: ObjectRef,
    assets: &mut BTreeMap<AssetId, Asset>,
    object_assets: &mut BTreeMap<ObjectRef, AssetId>,
    layout: &mut Layout,
) -> AssetId {
    if let Some(id) = object_assets.get(&object) {
        return *id;
    }
    let id = AssetId::new();
    let location = layout.take("Data", &format!("array_{}", &object.sha256[..8]), "array");
    assets.insert(
        id,
        Asset {
            id,
            location,
            media_type: "application/x-pbrt-ui-array".into(),
            sha256: object.sha256.clone(),
            size: object.size,
            origin: None,
        },
    );
    object_assets.insert(object, id);
    id
}

fn option_value(token: &str) -> Option<OptionValue> {
    let text = token.trim();
    if text.len() >= 2 && text.starts_with('"') && text.ends_with('"') {
        return Some(OptionValue::String(unescape(&text[1..text.len() - 1])));
    }
    match text {
        "true" => return Some(OptionValue::Bool(true)),
        "false" => return Some(OptionValue::Bool(false)),
        _ => {}
    }
    if let Ok(value) = text.parse::<i64>() {
        return Some(OptionValue::Integer(value));
    }
    if let Ok(value) = text.parse::<f64>() {
        return Some(OptionValue::Float(if value.is_nan() {
            FloatValue::Nan
        } else if value == f64::INFINITY {
            FloatValue::PositiveInfinity
        } else if value == f64::NEG_INFINITY {
            FloatValue::NegativeInfinity
        } else {
            FloatValue::Finite(value)
        }));
    }
    if text.is_empty() {
        None
    } else {
        Some(OptionValue::String(text.into()))
    }
}

fn unescape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Frozen copy of the format-1 evaluator: it only needs the graphics state at
/// each resource declaration.
fn evaluate_legacy(document: &LegacyDocument) -> Result<BTreeMap<ResourceId, GraphicsState>> {
    let mut states = BTreeMap::new();
    let mut coordinates = BTreeMap::new();
    let mut state = GraphicsState::default();
    for id in &document.scene.options {
        if let LegacyNodeKind::State {
            operation: StateOperation::TransformTimes(times),
        } = &document.scene.nodes[id].data
        {
            state.times = *times;
        }
    }
    for id in &document.scene.options {
        legacy_node(document, *id, &mut state, &mut states, &mut coordinates)?;
    }
    state.start = crate::math::Mat4::identity();
    state.end = crate::math::Mat4::identity();
    state.active = ActiveSamples::All;
    coordinates.insert("world".into(), (state.start, state.end));
    legacy_node(
        document,
        document.scene.world,
        &mut state,
        &mut states,
        &mut coordinates,
    )?;
    Ok(states)
}

fn legacy_node(
    document: &LegacyDocument,
    id: NodeId,
    state: &mut GraphicsState,
    states: &mut BTreeMap<ResourceId, GraphicsState>,
    coordinates: &mut BTreeMap<String, (crate::math::Mat4, crate::math::Mat4)>,
) -> Result<()> {
    match &document.scene.nodes[&id].data {
        LegacyNodeKind::Scope { scope, children } => {
            let previous = state.clone();
            for child in children {
                legacy_node(document, *child, state, states, coordinates)?;
            }
            match scope {
                ScopeKind::Attribute | ScopeKind::ObjectDefinition => *state = previous,
                ScopeKind::Transform => {
                    state.start = previous.start;
                    state.end = previous.end;
                    state.active = previous.active;
                }
                ScopeKind::World => {}
            }
        }
        LegacyNodeKind::State { operation } => match operation {
            StateOperation::Transform(op) => state.apply(op)?,
            StateOperation::ActiveTransform(active) => state.active = *active,
            StateOperation::TransformTimes(_) => {}
            StateOperation::Material(binding) => state.material = binding.clone(),
            StateOperation::Medium { inside, outside } => {
                state.inside_medium = *inside;
                state.outside_medium = *outside;
            }
            StateOperation::ReverseOrientation => {
                state.reverse_orientation = !state.reverse_orientation
            }
            StateOperation::ColorSpace(name) => state.color_space = name.clone(),
            StateOperation::DefineCoordinates(name) => {
                coordinates.insert(name.clone(), (state.start, state.end));
            }
            StateOperation::UseCoordinates(name) => {
                let (start, end) = coordinates
                    .get(name)
                    .ok_or_else(|| invalid("unknown coordinate system"))?;
                state.set(*start, *end, true)?;
            }
        },
        LegacyNodeKind::ResourceDeclaration {
            resource,
            bind_material,
        } => {
            states.insert(*resource, state.clone());
            if *bind_material {
                state.material = MaterialBinding::Resource(*resource);
            }
            if let Some(scope) = document.scene.resources[resource].definition_scope {
                legacy_node(document, scope, state, states, coordinates)?;
            }
        }
        LegacyNodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            ..
        } => {
            state.area_light = (!implementation.is_empty()).then_some(id);
        }
        LegacyNodeKind::Entity {
            entity: EntityKind::Camera,
            ..
        } => {
            let start = crate::math::inverse(state.start)
                .ok_or_else(|| invalid("singular camera transform"))?;
            let end = crate::math::inverse(state.end)
                .ok_or_else(|| invalid("singular camera transform"))?;
            coordinates.insert("camera".into(), (start, end));
        }
        LegacyNodeKind::Retained { .. } => {}
        _ => {}
    }
    Ok(())
}
