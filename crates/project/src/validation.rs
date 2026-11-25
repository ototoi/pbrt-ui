use crate::*;
use std::collections::{BTreeMap, BTreeSet};

const MAX_NODES: usize = 250_000;
const MAX_DEPTH: usize = 256;

/// `Assets/`-relative placement. Full grammar checks live in the folder store
///; the model only rejects absolute, empty and traversing paths.
fn location(value: &str) -> Result<()> {
    if value.is_empty()
        || value.contains('\\')
        || value.starts_with('/')
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == ".." || part.contains(':'))
    {
        return Err(invalid(
            "location must be a relative path without traversal",
        ));
    }
    Ok(())
}

fn normalize_location(value: &str) -> String {
    value.to_lowercase()
}

pub(crate) fn validate(doc: &ProjectDocument) -> Result<()> {
    if doc.title.trim().is_empty() {
        return Err(invalid("project title is empty"));
    }
    if let Some(scene) = doc.default_scene
        && !doc.scenes.contains_key(&scene)
    {
        return Err(invalid("default scene does not exist"));
    }
    let scene_nodes = doc.scenes.values().map(|s| s.nodes.len()).sum::<usize>();
    if scene_nodes + doc.definition_nodes.len() > MAX_NODES
        || doc.resources.len() > MAX_NODES
        || doc.assets.len() > MAX_NODES
    {
        return Err(ProjectError::ResourceLimit("project record count".into()));
    }
    validate_imports(doc)?;

    let mut ids = BTreeSet::new();
    let mut uuid_ids = BTreeSet::from([doc.id.0]);
    let mut locations = BTreeMap::<String, String>::new();
    for (id, scene) in &doc.scenes {
        if id != &scene.id {
            return Err(invalid("scene id does not match map key"));
        }
        if !uuid_ids.insert(id.0) {
            return Err(invalid("project UUID is repeated across records"));
        }
        if scene.label.trim().is_empty() {
            return Err(invalid("scene label is empty"));
        }
        location(&scene.location)?;
        if let Some(other) = locations.insert(normalize_location(&scene.location), "scene".into()) {
            return Err(invalid(format!(
                "duplicate item location (scene and {other})"
            )));
        }
        for node in scene.nodes.keys() {
            if !ids.insert(*node) {
                return Err(invalid("node id is repeated across scenes or definitions"));
            }
            if !uuid_ids.insert(node.0) {
                return Err(invalid("project UUID is repeated across records"));
            }
        }
    }
    for id in doc.definition_nodes.keys() {
        if !ids.insert(*id) {
            return Err(invalid("node id is repeated across scenes or definitions"));
        }
        if !uuid_ids.insert(id.0) {
            return Err(invalid("project UUID is repeated across records"));
        }
    }

    for (id, asset) in &doc.assets {
        if id != &asset.id {
            return Err(invalid("asset id does not match map key"));
        }
        if !uuid_ids.insert(id.0) {
            return Err(invalid("project UUID is repeated across records"));
        }
        location(&asset.location)?;
        if let Some(other) = locations.insert(normalize_location(&asset.location), "asset".into()) {
            return Err(invalid(format!(
                "duplicate item location (asset and {other})"
            )));
        }
        if asset.media_type.trim().is_empty() {
            return Err(invalid("asset media type is empty"));
        }
        if asset.sha256.len() != 64
            || !asset
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(invalid("invalid object SHA-256"));
        }
        if let Some(origin) = &asset.origin {
            if origin.logical_path.trim().is_empty() {
                return Err(invalid("asset origin path is empty"));
            }
            if let Some(origin) = &origin.origin {
                validate_origin(doc, origin)?;
            }
        }
    }

    for (id, resource) in &doc.resources {
        if id != &resource.id {
            return Err(invalid("resource id does not match map key"));
        }
        if !uuid_ids.insert(id.0) {
            return Err(invalid("project UUID is repeated across records"));
        }
        revision(&resource.edition)?;
        if resource.label.trim().is_empty() {
            return Err(invalid("resource label is empty"));
        }
        location(&resource.location)?;
        if let Some(other) =
            locations.insert(normalize_location(&resource.location), "resource".into())
        {
            return Err(invalid(format!(
                "duplicate item location (resource and {other})"
            )));
        }
        if let Some(origin) = &resource.origin {
            validate_origin(doc, origin)?;
        }
        parameters(doc, &resource.parameters)?;
        if (resource.kind == ResourceKind::Texture) != resource.texture_value_type.is_some() {
            return Err(invalid("texture value type is required only for textures"));
        }
        if resource.kind == ResourceKind::ObjectDefinition {
            let scope = resource
                .definition_scope
                .and_then(|id| doc.definition_nodes.get(&id));
            if !matches!(
                scope.map(|n| &n.data),
                Some(NodeKind::Scope {
                    scope: ScopeKind::ObjectDefinition,
                    ..
                })
            ) {
                return Err(invalid(
                    "object definition requires its own definition scope",
                ));
            }
            if !resource.parameters.is_empty() {
                return Err(invalid("object definition parameters belong to its body"));
            }
        } else if resource.definition_scope.is_some() || resource.implementation.is_empty() {
            return Err(invalid(
                "invalid resource implementation or definition scope",
            ));
        }
        if resource.kind == ResourceKind::SpectrumData {
            spectrum_data(resource)?;
        }
        if let Some(declared) = &resource.declared_in {
            if resource.kind == ResourceKind::Material && declared.transform.is_some() {
                return Err(invalid("material declarations do not capture transforms"));
            }
            if let Some(transform) = &declared.transform {
                if !transform
                    .start
                    .iter()
                    .chain(&transform.end)
                    .all(|v| v.is_finite())
                    || !transform.times.iter().all(|t| t.is_finite())
                {
                    return Err(invalid("captured declaration transform is not finite"));
                }
            }
            if let Some(graphics) = &declared.graphics {
                state(doc, &StateOperation::Material(graphics.material.clone()))?;
                state(
                    doc,
                    &StateOperation::Medium {
                        inside: graphics.inside_medium,
                        outside: graphics.outside_medium,
                    },
                )?;
                if graphics.color_space.is_empty() {
                    return Err(invalid("captured color space is empty"));
                }
            }
            if resource.kind == ResourceKind::Texture && declared.graphics.is_some() {
                return Err(invalid("texture captures only its transform"));
            }
        }
    }

    for (id, import) in &doc.imports {
        if id != &import.id || !uuid_ids.insert(id.0) {
            return Err(invalid("project UUID is repeated or mismatched"));
        }
        for source in import.sources.keys() {
            if !uuid_ids.insert(source.0) {
                return Err(invalid("project UUID is repeated across records"));
            }
        }
        for occurrence in import.occurrences.keys() {
            if !uuid_ids.insert(occurrence.0) {
                return Err(invalid("project UUID is repeated across records"));
            }
        }
    }

    for (id, node) in &doc.definition_nodes {
        if id != &node.id {
            return Err(invalid("node id does not match map key"));
        }
        validate_node(doc, node)?;
    }

    for scene in doc.scenes.values() {
        validate_scene(doc, scene)?;
    }
    validate_definitions(doc)?;

    let explicit_folders = doc
        .folders
        .iter()
        .map(|folder| normalize_location(folder))
        .collect::<BTreeSet<_>>();
    let mut item_parent_counts = BTreeMap::<String, usize>::new();
    for (path, _) in &locations {
        if let Some((parent, _)) = path.rsplit_once('/') {
            *item_parent_counts.entry(parent.to_owned()).or_default() += 1;
        }
    }
    for path in locations.keys() {
        if let Some((parent, _)) = path.rsplit_once('/')
            && !explicit_folders.contains(parent)
            && item_parent_counts.get(parent).copied().unwrap_or_default() < 2
        {
            return Err(invalid("item parent folder does not exist"));
        }
    }
    let mut folder_locations = BTreeMap::new();
    for folder in &doc.folders {
        location(folder)?;
        let normalized = normalize_location(folder);
        if folder_locations.insert(normalized.clone(), ()).is_some() {
            return Err(invalid("duplicate folder location"));
        }
        if locations.contains_key(&normalized) {
            return Err(invalid("folder location collides with project item"));
        }
        let mut parent = std::path::Path::new(folder).parent();
        while let Some(p) = parent {
            if p.as_os_str().is_empty() {
                break;
            }
            let parent_key = normalize_location(&p.to_string_lossy());
            if !explicit_folders.contains(&parent_key)
                && !item_parent_counts.contains_key(&parent_key)
            {
                return Err(invalid("folder parent does not exist"));
            }
            parent = p.parent();
        }
    }

    validate_resource_cycles(doc)?;
    objects(doc)?;
    Ok(())
}

fn validate_node(doc: &ProjectDocument, node: &Node) -> Result<()> {
    revision(&node.edition)?;
    if let Some(origin) = &node.origin {
        validate_origin(doc, origin)?;
    }
    match &node.data {
        NodeKind::State { operation } => state(doc, operation)?,
        NodeKind::Entity {
            entity: EntityKind::AreaLight,
            implementation,
            parameters: params,
            ..
        } if implementation.is_empty() => {
            if !params.is_empty() {
                return Err(invalid("cleared area light cannot have parameters"));
            }
        }
        NodeKind::Entity {
            entity,
            implementation,
            parameters: params,
            pose,
        } => {
            if implementation.is_empty() {
                return Err(invalid("empty entity/configuration implementation"));
            }
            if let Some(pose) = pose {
                // a pose is a camera's own value; no other entity kind
                // has a camera-to-world matrix.
                if *entity != EntityKind::Camera {
                    return Err(invalid("only a camera can hold a pose"));
                }
                if !(pose.start.iter().chain(&pose.end).all(|v| v.is_finite())) {
                    return Err(invalid("camera pose is not finite"));
                }
            }
            parameters(doc, params)?;
        }
        NodeKind::Configuration {
            implementation,
            parameters: params,
            ..
        } => {
            if implementation.is_empty() {
                return Err(invalid("empty entity/configuration implementation"));
            }
            parameters(doc, params)?;
        }
        NodeKind::Instance { definition } => {
            resource(doc, *definition, ResourceKind::ObjectDefinition)?
        }
        NodeKind::Option { name, value } => {
            if name.is_empty() {
                return Err(invalid("Option requires a name"));
            }
            if matches!(value, OptionValue::String(text) if text.is_empty()) {
                return Err(invalid("Option string value is empty"));
            }
            if matches!(value, OptionValue::Float(FloatValue::Finite(n)) if !n.is_finite()) {
                return Err(invalid("Option float value is not finite"));
            }
        }
        NodeKind::Scope { .. } => {}
    }
    Ok(())
}

/// One scene's roots and owned nodes. This also rejects cycles and nodes that
/// no root reaches.
fn validate_scene(doc: &ProjectDocument, scene: &SceneDocument) -> Result<()> {
    for (id, node) in &scene.nodes {
        if id != &node.id {
            return Err(invalid("node id does not match map key"));
        }
    }
    if !matches!(
        scene.nodes.get(&scene.world).map(|n| &n.data),
        Some(NodeKind::Scope {
            scope: ScopeKind::World,
            ..
        })
    ) {
        return Err(invalid("scene world must reference a World scope"));
    }
    if let Some(camera) = scene.render_camera {
        if !scene.options.contains(&camera)
            || !matches!(
                scene.nodes.get(&camera).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            )
        {
            return Err(invalid(
                "render camera must reference a Camera in the same scene",
            ));
        }
    }
    let mut seen = BTreeSet::new();
    let mut owned_objects = BTreeSet::new();
    let mut stack = vec![(scene.world, 0usize, false, false, false)];
    stack.extend(scene.options.iter().map(|id| (*id, 0, true, false, false)));
    while let Some((id, depth, options, in_definition, world_direct)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(ProjectError::ResourceLimit("project scope depth".into()));
        }
        if !seen.insert(id) {
            return Err(invalid("node has multiple owners or forms a cycle"));
        }
        let node = scene
            .nodes
            .get(&id)
            .ok_or_else(|| invalid("missing child node"))?;
        validate_node(doc, node)?;
        match &node.data {
            NodeKind::Scope {
                scope,
                children,
                owned_object,
                parent_camera,
            } => {
                if options {
                    return Err(invalid("graphics scopes require World"));
                }
                if *scope == ScopeKind::World && id != scene.world {
                    return Err(invalid("nested World scope"));
                }
                if *scope == ScopeKind::ObjectDefinition {
                    return Err(invalid(
                        "object definition scopes belong to definition_nodes",
                    ));
                }
                if let Some(camera) = *parent_camera {
                    // a camera-attached group sits at the World level and
                    // points at a Camera of the same scene's Options; the
                    // camera itself cannot own children.
                    if *scope != ScopeKind::Attribute {
                        return Err(invalid("parent camera is valid only on an Attribute scope"));
                    }
                    if !world_direct {
                        return Err(invalid(
                            "a parent camera group must be a direct child of World",
                        ));
                    }
                    if !scene.options.contains(&camera)
                        || !matches!(
                            scene.nodes.get(&camera).map(|node| &node.data),
                            Some(NodeKind::Entity {
                                entity: EntityKind::Camera,
                                ..
                            })
                        )
                    {
                        return Err(invalid(
                            "parent camera must reference a Camera in the scene options",
                        ));
                    }
                }
                validate_owned_object(
                    *scope,
                    *owned_object,
                    children,
                    &scene.nodes,
                    &mut owned_objects,
                )?;
                let parent_is_world = id == scene.world;
                stack.extend(
                    children
                        .iter()
                        .rev()
                        .map(move |id| (*id, depth + 1, options, in_definition, parent_is_world)),
                );
            }
            NodeKind::State {
                operation: StateOperation::TransformTimes(_),
            } if !options => {
                return Err(invalid("TransformTimes must be in Options"));
            }
            NodeKind::State {
                operation: StateOperation::Material(_) | StateOperation::ReverseOrientation,
            } if options => {
                return Err(invalid("world graphics state in Options"));
            }
            NodeKind::Configuration { .. } | NodeKind::Option { .. } if !options => {
                return Err(invalid("render configuration must be in Options"));
            }
            NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            } if !options => return Err(invalid("camera must be in Options")),
            NodeKind::Entity {
                entity: EntityKind::Shape | EntityKind::Light | EntityKind::AreaLight,
                ..
            }
            | NodeKind::Instance { .. }
                if options =>
            {
                return Err(invalid("world entity in Options"));
            }
            NodeKind::Instance { .. } if in_definition => {
                return Err(invalid(
                    "nested instances are not supported in object definitions",
                ));
            }
            _ => {}
        }
    }
    if seen.len() != scene.nodes.len() {
        return Err(invalid("unreachable scene nodes"));
    }
    Ok(())
}

/// Every definition node is owned by exactly one object definition, rooted at
/// the resource's `definition_scope`.
fn validate_definitions(doc: &ProjectDocument) -> Result<()> {
    let mut seen = BTreeSet::new();
    let mut owned_objects = BTreeSet::new();
    let mut roots = BTreeSet::new();
    for resource in doc.resources.values() {
        let Some(root) = resource.definition_scope else {
            continue;
        };
        if !roots.insert(root) {
            return Err(invalid(
                "definition scope belongs to more than one resource",
            ));
        }
        if !matches!(
            doc.definition_nodes.get(&root).map(|n| &n.data),
            Some(NodeKind::Scope {
                scope: ScopeKind::ObjectDefinition,
                ..
            })
        ) {
            return Err(invalid(
                "definition scope must be an ObjectDefinition scope",
            ));
        }
        let mut stack = vec![(root, 0usize)];
        while let Some((id, depth)) = stack.pop() {
            if depth > MAX_DEPTH {
                return Err(ProjectError::ResourceLimit("project scope depth".into()));
            }
            if !seen.insert(id) {
                return Err(invalid(
                    "definition node has multiple owners or forms a cycle",
                ));
            }
            let node = doc
                .definition_nodes
                .get(&id)
                .ok_or_else(|| invalid("missing definition node"))?;
            match &node.data {
                NodeKind::Scope {
                    scope: ScopeKind::ObjectDefinition,
                    ..
                } if id != root => {
                    return Err(invalid("nested object definition scope"));
                }
                NodeKind::Scope {
                    scope,
                    children,
                    owned_object,
                    parent_camera,
                } => {
                    if *scope == ScopeKind::World {
                        return Err(invalid("nested World scope"));
                    }
                    if parent_camera.is_some() {
                        // only a World-level group can attach to a
                        // camera; a definition tree has no World.
                        return Err(invalid(
                            "a camera-attached group must be a direct child of World",
                        ));
                    }
                    validate_owned_object(
                        *scope,
                        *owned_object,
                        children,
                        &doc.definition_nodes,
                        &mut owned_objects,
                    )?;
                    stack.extend(children.iter().rev().map(|child| (*child, depth + 1)));
                }
                NodeKind::State {
                    operation: StateOperation::TransformTimes(_),
                } => return Err(invalid("TransformTimes must be in Options")),
                NodeKind::Configuration { .. } | NodeKind::Option { .. } => {
                    return Err(invalid("render configuration must be in Options"));
                }
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                } => return Err(invalid("camera must be in Options")),
                NodeKind::Instance { .. } => {
                    return Err(invalid(
                        "nested instances are not supported in object definitions",
                    ));
                }
                _ => {}
            }
        }
    }
    if seen.len() != doc.definition_nodes.len() {
        return Err(invalid("unreachable definition nodes"));
    }
    Ok(())
}

fn validate_owned_object(
    scope: ScopeKind,
    owner: Option<NodeId>,
    children: &[NodeId],
    nodes: &BTreeMap<NodeId, Node>,
    assigned: &mut BTreeSet<NodeId>,
) -> Result<()> {
    let Some(owner) = owner else {
        return Ok(());
    };
    if scope != ScopeKind::Attribute {
        return Err(invalid("owned object is valid only on an Attribute scope"));
    }
    let object_children = children
        .iter()
        .filter(|id| nodes.get(id).is_some_and(is_owned_visible_object))
        .copied()
        .collect::<Vec<_>>();
    if object_children != [owner] {
        return Err(invalid(
            "owned object must be the Attribute scope's sole visible object child",
        ));
    }
    if !assigned.insert(owner) {
        return Err(invalid("object is owned by multiple Attribute scopes"));
    }
    Ok(())
}

fn is_owned_visible_object(node: &Node) -> bool {
    matches!(
        &node.data,
        NodeKind::Entity {
            entity: EntityKind::Shape | EntityKind::Light | EntityKind::Camera,
            ..
        } | NodeKind::Instance { .. }
            | NodeKind::Scope {
                scope: ScopeKind::Attribute | ScopeKind::Transform,
                owned_object: None,
                ..
            }
    )
}

pub(crate) fn revision(value: &Revision) -> Result<u64> {
    value
        .to_u64()
        .ok_or_else(|| invalid("invalid edition/revision"))
}

/// A SpectrumData resource's invariants:
/// exactly `wavelengths` and `values`, both float tables of equal length,
/// finite samples, positive ascending wavelengths, and at most
/// `MAX_SPECTRUM_SAMPLES` pairs — r4's `PiecewiseLinearSpectrum` input
/// contract. Inline tables are checked completely; `.array`-backed tables
/// can only be counted here (the bytes are content-addressed store data;
/// r4 sorts samples itself when it loads a file).
fn spectrum_data(resource: &Resource) -> Result<()> {
    if resource.implementation != "piecewise" {
        return Err(invalid("spectrum implementation must be piecewise"));
    }
    if resource.parameters.len() != 2 {
        return Err(invalid(
            "spectrum data takes exactly wavelengths and values",
        ));
    }
    let mut wavelengths = None;
    let mut values = None;
    for parameter in &resource.parameters {
        match parameter.name.as_str() {
            "wavelengths" if wavelengths.is_none() => wavelengths = Some(parameter),
            "values" if values.is_none() => values = Some(parameter),
            _ => {
                return Err(invalid(
                    "spectrum data takes exactly wavelengths and values",
                ));
            }
        }
    }
    let (Some(wavelengths), Some(values)) = (wavelengths, values) else {
        return Err(invalid(
            "spectrum data takes exactly wavelengths and values",
        ));
    };
    let count = |parameter: &Parameter| -> Result<usize> {
        if parameter.value_type != "float" {
            return Err(invalid("spectrum parameters must be floats"));
        }
        match &parameter.values {
            ParameterValues::Floats(samples) => {
                if samples
                    .iter()
                    .any(|sample| !matches!(sample, FloatValue::Finite(n) if n.is_finite()))
                {
                    return Err(invalid("spectrum samples must be finite"));
                }
                Ok(samples.len())
            }
            ParameterValues::Array {
                encoding, count, ..
            } => {
                if *encoding != ArrayEncoding::F64Le {
                    return Err(invalid("spectrum parameters must be floats"));
                }
                Ok(*count as usize)
            }
            _ => Err(invalid("spectrum parameters must be floats")),
        }
    };
    let wavelength_count = count(wavelengths)?;
    let value_count = count(values)?;
    if wavelength_count != value_count {
        return Err(invalid("spectrum sample counts differ"));
    }
    if wavelength_count == 0 {
        return Err(invalid("spectrum has no samples"));
    }
    if wavelength_count > MAX_SPECTRUM_SAMPLES {
        return Err(invalid("spectrum sample count exceeds the limit"));
    }
    if let ParameterValues::Floats(samples) = &wavelengths.values {
        for pair in samples.windows(2) {
            if matches!(pair, [FloatValue::Finite(a), FloatValue::Finite(b)] if b < a) {
                return Err(invalid("spectrum wavelengths must be ascending"));
            }
        }
        for sample in samples {
            if matches!(sample, FloatValue::Finite(n) if *n <= 0.) {
                return Err(invalid("spectrum wavelengths must be positive"));
            }
        }
    }
    Ok(())
}

mod imports;
mod objects;
mod parameters;
mod resource_cycles;
use imports::{validate_imports, validate_origin};
pub(crate) use objects::objects;
use parameters::{parameters, resource, state};
use resource_cycles::validate_resource_cycles;
