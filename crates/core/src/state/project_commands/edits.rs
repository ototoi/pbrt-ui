use super::*;
use pbrt_io::implementation_change::{ImplementationChange, implementation_change};
use project::{ColorSpaceBinding, Edit, FloatValue, Parameter, ParameterValues as V};
pub(super) fn target(value: dto::ParameterTarget) -> project::ParameterTarget {
    match value {
        dto::ParameterTarget::Node(id) => project::ParameterTarget::Node(project::NodeId(id)),
        dto::ParameterTarget::Resource(id) => {
            project::ParameterTarget::Resource(project::ResourceId(id))
        }
    }
}
fn resource_kind(value: dto::ResourceKind) -> project::ResourceKind {
    match value {
        dto::ResourceKind::Material => project::ResourceKind::Material,
        dto::ResourceKind::Texture => project::ResourceKind::Texture,
        dto::ResourceKind::Medium => project::ResourceKind::Medium,
        dto::ResourceKind::ObjectDefinition => project::ResourceKind::ObjectDefinition,
        dto::ResourceKind::SpectrumData => project::ResourceKind::SpectrumData,
    }
}
fn configuration_kind(value: dto::ConfigurationKind) -> project::ConfigurationKind {
    match value {
        dto::ConfigurationKind::Film => project::ConfigurationKind::Film,
        dto::ConfigurationKind::Sampler => project::ConfigurationKind::Sampler,
        dto::ConfigurationKind::Integrator => project::ConfigurationKind::Integrator,
        dto::ConfigurationKind::PixelFilter => project::ConfigurationKind::PixelFilter,
        dto::ConfigurationKind::Accelerator => project::ConfigurationKind::Accelerator,
    }
}
pub(super) fn values(value: dto::ParameterValues) -> Result<V, ErrorPayload> {
    let (count, value) = match value {
        dto::ParameterValues::Floats(values) => {
            if values.iter().any(|v| !v.is_finite()) {
                return Err(invalid("float edits require finite values"));
            }
            (
                values.len(),
                V::Floats(values.into_iter().map(FloatValue::Finite).collect()),
            )
        }
        dto::ParameterValues::Integers(values) => {
            let parsed = values
                .iter()
                .map(|v| {
                    let n = v
                        .parse::<i64>()
                        .map_err(|_| invalid("integer edit requires signed i64 decimal strings"))?;
                    if n.to_string() != *v {
                        return Err(invalid("integer strings must be canonical"));
                    }
                    Ok(n)
                })
                .collect::<Result<Vec<_>, ErrorPayload>>()?;
            (values.len(), V::Integers(parsed))
        }
        dto::ParameterValues::Booleans(values) => (values.len(), V::Booleans(values)),
        dto::ParameterValues::Strings(values) => {
            if values.iter().any(|v| v.len() > 4096) {
                return Err(invalid("edited strings exceed 4096 bytes"));
            }
            (values.len(), V::Strings(values))
        }
        dto::ParameterValues::BuiltinSpectra(values) => {
            if values.iter().any(|v| v.len() > 4096) {
                return Err(invalid("spectrum names exceed 4096 bytes"));
            }
            (values.len(), V::BuiltinSpectra(values))
        }
        dto::ParameterValues::Assets(values) => (
            values.len(),
            V::Assets(values.into_iter().map(project::AssetId).collect()),
        ),
        dto::ParameterValues::Resources { kind, ids } => (
            ids.len(),
            V::Resources {
                kind: resource_kind(kind),
                ids: ids.into_iter().map(project::ResourceId).collect(),
            },
        ),
    };
    if count > 4096 {
        return Err(invalid(
            "parameter edits are limited to 4096 values; bulk objects remain read-only",
        ));
    }
    Ok(value)
}

/// What one `project.set_parameter_values` job does. Inline arrays are
/// plain window edits; a data asset gets a patched copy staged on the I/O
/// executor (`patches`: byte offset, replacement), placed as `new_id` at
/// `new_location`, and the old asset is deleted unless another parameter
/// shares it.
// Built once per array edit, so the larger variant costs nothing worth a box at every match.
#[allow(clippy::large_enum_variant)]
pub(crate) enum ArrayEditPlan {
    Inline {
        target: project::ParameterTarget,
        edits: Vec<Edit>,
    },
    DataAsset {
        old: project::Asset,
        patches: Vec<(u64, Vec<u8>)>,
        target: project::ParameterTarget,
        name: String,
        new_id: project::AssetId,
        new_location: String,
        shared: bool,
    },
}

/// The parameter `name` of `target` as `document` holds it.
fn array_parameter<'a>(
    document: &'a project::ProjectDocument,
    active_scene: Option<project::SceneId>,
    target: &dto::ParameterTarget,
    name: &str,
) -> Result<&'a Parameter, ErrorPayload> {
    let parameters = match target {
        dto::ParameterTarget::Node(id) => {
            let node = document
                .scene_view(active_scene)
                .ok()
                .and_then(|view| view.node(project::NodeId(*id)))
                .or_else(|| document.definition_nodes.get(&project::NodeId(*id)))
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "node not found"))?;
            match &node.data {
                project::NodeKind::Entity { parameters, .. }
                | project::NodeKind::Configuration { parameters, .. } => parameters,
                _ => return Err(invalid("node does not declare parameters")),
            }
        }
        dto::ParameterTarget::Resource(id) => {
            &document
                .resources
                .get(&project::ResourceId(*id))
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "resource not found"))?
                .parameters
        }
    };
    find_parameter(parameters, name)
}

/// `project.apply`'s `set_parameter_values`: inline arrays only. Rewriting
/// a data asset reads and writes its whole file, which belongs on the I/O
/// executor (`project.set_parameter_values`), never in apply.
pub(super) fn plan_inline_values(
    wire: &dto::ProjectEdit,
    document: &project::ProjectDocument,
    active_scene: Option<project::SceneId>,
) -> Result<Vec<Edit>, ErrorPayload> {
    let dto::ProjectEdit::SetParameterValues { target, name, .. } = wire else {
        return Err(invalid("expected set_parameter_values edit"));
    };
    if matches!(
        array_parameter(document, active_scene, target, name)?.values,
        V::Array { .. }
    ) {
        return Err(
            invalid("data asset values change through project.set_parameter_values")
                .with_details(json!({"reason":"data_asset_edit_is_a_job"})),
        );
    }
    Ok(vec![convert(wire.clone())?])
}

/// a `project.set_parameter_values` job against the pinned document:
/// every window is checked here (non-empty, at most 4096 values, inside the
/// array, disjoint), so the job only copies bytes.
pub(super) fn plan_array_edit(
    document: &project::ProjectDocument,
    active_scene: Option<project::SceneId>,
    p: &dto::SetParameterValuesParams,
) -> Result<ArrayEditPlan, ErrorPayload> {
    if p.windows.is_empty() || p.windows.len() > 4096 {
        return Err(invalid("give between 1 and 4096 value windows"));
    }
    let parameter = array_parameter(document, active_scene, &p.target, &p.name)?;
    let mut windows = p
        .windows
        .iter()
        .map(|window| Ok((window.offset, values(window.values.clone())?)))
        .collect::<Result<Vec<_>, ErrorPayload>>()?;
    windows.sort_by_key(|(offset, _)| *offset);
    let mut end = 0usize;
    for (offset, window) in &windows {
        let length = value_count(window);
        if length == 0 || length > 4096 {
            return Err(invalid("each window holds between 1 and 4096 values"));
        }
        if *offset < end {
            return Err(invalid("value windows overlap"));
        }
        end = offset
            .checked_add(length)
            .ok_or_else(|| invalid("value window overflows"))?;
    }
    let V::Array {
        asset: old_id,
        encoding,
        count,
    } = &parameter.values
    else {
        // Inline: checked here too, so a bad window is refused now rather
        // than failing the job later.
        if matches!(parameter.values, V::Resources { .. }) {
            return Err(invalid("resource lists are not editable as array values"));
        }
        if end > value_count(&parameter.values) {
            return Err(invalid("value window is outside the existing array"));
        }
        if windows.iter().any(|(_, window)| {
            std::mem::discriminant(window) != std::mem::discriminant(&parameter.values)
        }) {
            return Err(invalid(
                "replacement values do not match the parameter array type",
            ));
        }
        return Ok(ArrayEditPlan::Inline {
            target: target(p.target.clone()),
            edits: windows
                .into_iter()
                .map(|(offset, values)| Edit::SetParameterValues {
                    target: target(p.target.clone()),
                    name: p.name.clone(),
                    offset,
                    values,
                })
                .collect(),
        });
    };
    if end as u64 > *count {
        return Err(invalid("value window is outside the existing data asset"));
    }
    let old = document
        .assets
        .get(old_id)
        .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "array data asset not found"))?;
    let width = match encoding {
        project::ArrayEncoding::F64Le | project::ArrayEncoding::I64Le => 8u64,
        project::ArrayEncoding::BoolU8 => 1u64,
    };
    if count.checked_mul(width) != Some(old.size) {
        return Err(invalid("data asset size does not match its array"));
    }
    let patches = windows
        .iter()
        .map(|(offset, window)| Ok((*offset as u64 * width, encode_window(*encoding, window)?)))
        .collect::<Result<Vec<_>, ErrorPayload>>()?;
    let mut namer = project::layout::LocationNamer::new();
    for location in document
        .assets
        .values()
        .map(|asset| &asset.location)
        .chain(
            document
                .resources
                .values()
                .map(|resource| &resource.location),
        )
        .chain(document.scenes.values().map(|scene| &scene.location))
        .chain(document.folders.iter())
    {
        namer.reserve(location);
    }
    let old_path = std::path::Path::new(&old.location);
    let part = |value: Option<&std::ffi::OsStr>, fallback: &'static str| {
        value
            .and_then(|value| value.to_str())
            .unwrap_or(fallback)
            .to_owned()
    };
    let new_location = namer.take(
        &part(old_path.parent().map(|p| p.as_os_str()), ""),
        &part(old_path.file_stem(), "data"),
        &part(old_path.extension(), "bin"),
    );
    Ok(ArrayEditPlan::DataAsset {
        old: old.clone(),
        patches,
        target: target(p.target.clone()),
        name: p.name.clone(),
        new_id: project::AssetId::new(),
        new_location,
        shared: document
            .reference_index()
            .count(project::ItemId::Asset(*old_id))
            > 1,
    })
}

/// A node target is found in the active scene when the job publishes, so a
/// scene switch since submission (which keeps the revision) is refused with
/// a reason rather than failing on a missing node. Resources do not depend
/// on the scene.
pub(crate) fn scene_still_holds(
    target: &project::ParameterTarget,
    submitted: Option<project::SceneId>,
    current: Option<project::SceneId>,
) -> Result<(), ErrorPayload> {
    if matches!(target, project::ParameterTarget::Node(_)) && submitted != current {
        return Err(ErrorPayload::new(
            ErrorCode::StaleProject,
            "the active scene changed while the array edit ran",
        )
        .with_details(json!({"reason":"scene_changed"})));
    }
    Ok(())
}

/// The edits that publish a staged data asset copy: the new asset, the
/// parameter's reference to it, and the old asset's removal when unshared.
pub(crate) fn data_asset_edits(
    plan: ArrayEditPlan,
    lease: Option<std::sync::Arc<project::PayloadLease>>,
) -> Result<Vec<Edit>, ErrorPayload> {
    let (
        ArrayEditPlan::DataAsset {
            old,
            target,
            name,
            new_id,
            new_location,
            shared,
            ..
        },
        Some(lease),
    ) = (plan, lease)
    else {
        return Err(ErrorPayload::new(
            ErrorCode::OutputInvalid,
            "array edit result does not match its plan",
        ));
    };
    let asset = project::Asset {
        id: new_id,
        location: new_location,
        media_type: old.media_type.clone(),
        sha256: lease.sha256().to_owned(),
        size: lease.size(),
        origin: None,
    };
    let folder = std::path::Path::new(&asset.location)
        .parent()
        .and_then(|path| path.to_str())
        .unwrap_or_default()
        .to_owned();
    let mut document = project::ProjectDocument {
        id: project::ProjectId::new(),
        title: "Array edit".into(),
        default_scene: None,
        scenes: Default::default(),
        resources: Default::default(),
        definition_nodes: Default::default(),
        assets: [(asset.id, asset)].into(),
        folders: Default::default(),
        imports: Default::default(),
    };
    if !folder.is_empty() {
        document.folders.insert(folder);
    }
    let batch = project::PreparedImport::new(document, vec![lease])
        .map_err(crate::project_io::project_error)?;
    let mut edits = vec![
        Edit::ImportBatch { batch },
        Edit::SetAssetReference {
            target,
            parameter: name,
            asset: new_id,
        },
    ];
    if !shared {
        edits.push(Edit::DeleteItem {
            item: project::ItemId::Asset(old.id),
        });
    }
    Ok(edits)
}

fn value_count(values: &V) -> usize {
    match values {
        V::Floats(v) => v.len(),
        V::Integers(v) => v.len(),
        V::Booleans(v) => v.len(),
        V::Strings(v) | V::BuiltinSpectra(v) => v.len(),
        V::Assets(v) => v.len(),
        V::Resources { ids, .. } => ids.len(),
        V::Array { count, .. } => *count as usize,
    }
}

fn find_parameter<'a>(
    parameters: &'a [Parameter],
    name: &str,
) -> Result<&'a Parameter, ErrorPayload> {
    let matches = parameters
        .iter()
        .filter(|parameter| parameter.name == name)
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(invalid("parameter is missing or ambiguous"));
    }
    Ok(matches[0])
}

fn encode_window(encoding: project::ArrayEncoding, values: &V) -> Result<Vec<u8>, ErrorPayload> {
    let mut bytes = Vec::new();
    match (encoding, values) {
        (project::ArrayEncoding::F64Le, V::Floats(values)) => {
            for value in values {
                let number = match value {
                    FloatValue::Finite(value) => *value,
                    _ => return Err(invalid("array float edits require finite values")),
                };
                bytes.extend_from_slice(&number.to_le_bytes());
            }
        }
        (project::ArrayEncoding::I64Le, V::Integers(values)) => {
            for value in values {
                bytes.extend_from_slice(&value.to_le_bytes());
            }
        }
        (project::ArrayEncoding::BoolU8, V::Booleans(values)) => {
            bytes.extend(values.iter().map(|value| u8::from(*value)))
        }
        _ => {
            return Err(invalid(
                "replacement values do not match the data asset encoding",
            ));
        }
    }
    Ok(bytes)
}
pub(super) fn convert(edit: dto::ProjectEdit) -> Result<Edit, ErrorPayload> {
    Ok(match edit {
        dto::ProjectEdit::CreateCamera { scene } => Edit::CreateCamera {
            scene: project::SceneId(scene),
        },
        dto::ProjectEdit::CreateConfiguration {
            scene,
            configuration,
            implementation,
        } => Edit::CreateConfiguration {
            scene: project::SceneId(scene),
            configuration: configuration_kind(configuration),
            implementation: implementation
                .as_ref()
                .map(|name| name.trim().to_owned())
                .filter(|name| !name.is_empty()),
            parameters: Vec::new(),
        },
        dto::ProjectEdit::SetRenderCamera { scene, camera } => Edit::SetRenderCamera {
            scene: project::SceneId(scene),
            camera: camera.map(project::NodeId),
        },
        dto::ProjectEdit::SetSceneCoordinateSystem {
            scene,
            up,
            handedness,
        } => Edit::SetSceneCoordinateSystem {
            scene: project::SceneId(scene),
            up: up_axis(up),
            handedness: wire_handedness(handedness),
        },
        dto::ProjectEdit::CreateScene { location } => Edit::CreateScene { location },
        dto::ProjectEdit::DuplicateScene { scene, location } => Edit::DuplicateScene {
            scene: project::SceneId(scene),
            location,
        },
        dto::ProjectEdit::DeleteScene { scene } => Edit::DeleteScene {
            scene: project::SceneId(scene),
        },
        dto::ProjectEdit::MoveItem { item, location } => Edit::MoveItem {
            item: match item {
                dto::ProjectItemId::Scene(id) => project::ItemId::Scene(project::SceneId(id)),
                dto::ProjectItemId::Resource(id) => {
                    project::ItemId::Resource(project::ResourceId(id))
                }
                dto::ProjectItemId::Asset(id) => project::ItemId::Asset(project::AssetId(id)),
            },
            location,
        },
        dto::ProjectEdit::DeleteItem { item } => Edit::DeleteItem {
            item: match item {
                dto::ProjectItemId::Scene(id) => project::ItemId::Scene(project::SceneId(id)),
                dto::ProjectItemId::Resource(id) => {
                    project::ItemId::Resource(project::ResourceId(id))
                }
                dto::ProjectItemId::Asset(id) => project::ItemId::Asset(project::AssetId(id)),
            },
        },
        dto::ProjectEdit::CreateFolder { path } => Edit::CreateFolder { path },
        dto::ProjectEdit::MoveFolder { from, to } => Edit::MoveFolder { from, to },
        dto::ProjectEdit::DeleteFolder { path } => Edit::DeleteFolder { path },
        dto::ProjectEdit::RenameNode { node_id, label } => {
            label_limit(&label)?;
            Edit::RenameNode {
                node: project::NodeId(node_id),
                label,
            }
        }
        dto::ProjectEdit::SetNodeActive { node_id, active } => Edit::SetNodeActive {
            node: project::NodeId(node_id),
            active,
        },
        dto::ProjectEdit::SetTextureValueType { resource_id, value } => Edit::SetTextureValueType {
            resource: project::ResourceId(resource_id),
            value: match value {
                dto::TextureValueType::Float => project::TextureValueType::Float,
                dto::TextureValueType::Spectrum => project::TextureValueType::Spectrum,
            },
        },
        dto::ProjectEdit::RenameResource { resource_id, label } => {
            label_limit(&label)?;
            Edit::RenameResource {
                resource: project::ResourceId(resource_id),
                label,
            }
        }
        dto::ProjectEdit::SetTransform { node_id, value } => {
            // re-parenting resets (PBRT Identity / Transform) enter the
            // session only through import; the wire cannot write them.
            if matches!(value, dto::Transform::Identity | dto::Transform::Replace(_)) {
                return Err(invalid(
                    "set_transform cannot write identity or replace operations \
                     (re-parenting resets are import-only)",
                ));
            }
            Edit::SetTransform {
                node: project::NodeId(node_id),
                value: match value {
                    dto::Transform::Identity => project::TransformOperation::Identity,
                    dto::Transform::Translate(v) => project::TransformOperation::Translate(v),
                    dto::Transform::Scale(v) => project::TransformOperation::Scale(v),
                    dto::Transform::LookAt(v) => project::TransformOperation::LookAt(v),
                    dto::Transform::Rotate { degrees, axis } => {
                        project::TransformOperation::Rotate { degrees, axis }
                    }
                    dto::Transform::Replace(v) => project::TransformOperation::Replace(v),
                    dto::Transform::Concat(v) => project::TransformOperation::Concat(v),
                },
            }
        }
        dto::ProjectEdit::SetMaterialBinding { node_id, value } => Edit::SetMaterialBinding {
            node: project::NodeId(node_id),
            value: material_binding(value),
        },
        dto::ProjectEdit::DeleteNode { node_id } => Edit::DeleteNode {
            node: project::NodeId(node_id),
        },
        dto::ProjectEdit::DuplicateNode { node_id } => Edit::DuplicateNode {
            node: project::NodeId(node_id),
        },
        dto::ProjectEdit::MoveNode {
            node_id,
            parent,
            after,
        } => Edit::MoveNode {
            node: project::NodeId(node_id),
            parent: project::NodeId(parent),
            after: after.map(project::NodeId),
        },
        dto::ProjectEdit::SetInstanceDefinition {
            node_id,
            definition,
        } => Edit::SetInstanceDefinition {
            node: project::NodeId(node_id),
            definition: project::ResourceId(definition),
        },
        dto::ProjectEdit::MakeObjectDefinition { node_id, label } => {
            if label
                .as_ref()
                .is_some_and(|label| label.trim().is_empty() || label.len() > 256)
            {
                return Err(invalid("object definition label must be 1 to 256 bytes"));
            }
            Edit::MakeObjectDefinition {
                node: project::NodeId(node_id),
                label,
            }
        }
        dto::ProjectEdit::UnpackObjectInstance { node_id } => Edit::UnpackObjectInstance {
            node: project::NodeId(node_id),
        },
        dto::ProjectEdit::SetParameter {
            target: where_,
            name,
            value_type,
            value,
        } => {
            if name.is_empty() || name.len() > 256 || value_type.is_empty() || value_type.len() > 64
            {
                return Err(invalid("invalid parameter name/type length"));
            }
            let value = value
                .map(|p| -> Result<Parameter, ErrorPayload> {
                    let payload = values(p.values)?;
                    if p.color_space.as_ref().is_some_and(|s| s.len() > 128) {
                        return Err(invalid("color space name is too long"));
                    }
                    Ok(Parameter {
                        name: p.name,
                        value_type: p.value_type,
                        color_space: p
                            .color_space
                            .map_or(ColorSpaceBinding::Inherit, ColorSpaceBinding::Explicit),
                        values: payload,
                    })
                })
                .transpose()?;
            Edit::SetParameter {
                target: target(where_),
                name,
                value_type,
                value,
            }
        }
        dto::ProjectEdit::SetParameterValues {
            target: where_,
            name,
            offset,
            values: replacement,
        } => Edit::SetParameterValues {
            target: target(where_),
            name,
            offset,
            values: values(replacement)?,
        },
        dto::ProjectEdit::CreatePreset {
            preset,
            parent,
            after,
            definition,
            view,
        } => Edit::CreatePreset {
            preset: project::ResourceId(preset),
            parent: project::NodeId(parent),
            after: after.map(project::NodeId),
            definition: definition.map(project::ResourceId),
            view: view.map(|view| project::presets::PresetView {
                position: view.position,
                look_at: view.look_at,
            }),
        },
        dto::ProjectEdit::CreateResource {
            id,
            resource_kind: requested_kind,
            implementation,
            label,
            folder,
            texture_value_type,
            parameters,
        } => {
            let kind = resource_kind(requested_kind);
            let texture_value_type = texture_value_type.map(|value| match value {
                dto::TextureValueType::Float => project::TextureValueType::Float,
                dto::TextureValueType::Spectrum => project::TextureValueType::Spectrum,
            });
            if label.trim().is_empty() || label.len() > 256 {
                return Err(invalid("invalid resource label length"));
            }
            if implementation.len() > 128 {
                return Err(invalid("invalid resource implementation length"));
            }
            if let Some(folder) = &folder
                && (folder.is_empty() || folder.len() > 256)
            {
                return Err(invalid("invalid resource folder length"));
            }
            let parameters = match parameters {
                Some(list) => list
                    .into_iter()
                    .map(|p| -> Result<Parameter, ErrorPayload> {
                        if p.name.is_empty()
                            || p.name.len() > 256
                            || p.value_type.is_empty()
                            || p.value_type.len() > 64
                        {
                            return Err(invalid("invalid parameter name/type length"));
                        }
                        let payload = values(p.values)?;
                        if p.color_space.as_ref().is_some_and(|s| s.len() > 128) {
                            return Err(invalid("color space name is too long"));
                        }
                        Ok(Parameter {
                            name: p.name,
                            value_type: p.value_type,
                            color_space: p
                                .color_space
                                .map_or(ColorSpaceBinding::Inherit, ColorSpaceBinding::Explicit),
                            values: payload,
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => {
                    if kind == project::ResourceKind::ObjectDefinition {
                        Vec::new()
                    } else if kind == project::ResourceKind::SpectrumData {
                        // A fresh Spectrum Data resource starts as a flat
                        // unit curve: neutral for the preview's channel reads.
                        vec![
                            project::Parameter::floats("wavelengths", [400., 700.]),
                            project::Parameter::floats("values", [1., 1.]),
                        ]
                    } else {
                        let directive = match kind {
                            project::ResourceKind::Material => "MakeNamedMaterial",
                            project::ResourceKind::Texture => "Texture",
                            project::ResourceKind::Medium => "MakeNamedMedium",
                            project::ResourceKind::ObjectDefinition => {
                                unreachable!("handled above")
                            }
                            project::ResourceKind::SpectrumData => {
                                unreachable!("handled above")
                            }
                        };
                        pbrt_io::parameter_schema::declarable_defaults(
                            directive,
                            &implementation,
                            texture_value_type,
                        )
                    }
                }
            };
            Edit::CreateResource {
                id: id.map(project::ResourceId),
                resource_kind: kind,
                implementation,
                label,
                folder,
                texture_value_type,
                parameters,
            }
        }
        dto::ProjectEdit::SetLocalBinding {
            node_id,
            material,
            medium,
        } => Edit::SetLocalBinding {
            node: project::NodeId(node_id),
            material: material.map(material_binding),
            medium: medium.map(|medium| project::MediumInterface {
                inside: medium.inside.map(project::ResourceId),
                outside: medium.outside.map(project::ResourceId),
            }),
        },
        dto::ProjectEdit::SetAreaLight { node_id, light } => {
            let light = light
                .map(|light| -> Result<project::AreaLightContent, ErrorPayload> {
                    if light.implementation.len() > 128 {
                        return Err(invalid("invalid area light implementation length"));
                    }
                    let implementation = light.implementation.clone();
                    Ok(project::AreaLightContent {
                        implementation,
                        // Omitted parameters take the area light table's
                        // declared defaults plus L [1 1 1].
                        parameters: match light.parameters {
                            Some(list) => list
                                .into_iter()
                                .map(|p| -> Result<Parameter, ErrorPayload> {
                                    if p.name.is_empty()
                                        || p.name.len() > 256
                                        || p.value_type.is_empty()
                                        || p.value_type.len() > 64
                                    {
                                        return Err(invalid("invalid parameter name/type length"));
                                    }
                                    let payload = values(p.values)?;
                                    if p.color_space.as_ref().is_some_and(|s| s.len() > 128) {
                                        return Err(invalid("color space name is too long"));
                                    }
                                    Ok(Parameter {
                                        name: p.name,
                                        value_type: p.value_type,
                                        color_space: p.color_space.map_or(
                                            ColorSpaceBinding::Inherit,
                                            ColorSpaceBinding::Explicit,
                                        ),
                                        values: payload,
                                    })
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                            None => {
                                pbrt_io::parameter_schema::initial_area_light(&light.implementation)
                            }
                        },
                    })
                })
                .transpose()?;
            Edit::SetAreaLight {
                node: project::NodeId(node_id),
                light,
            }
        }
        dto::ProjectEdit::ClearLocalBinding {
            node_id,
            material,
            medium,
        } => {
            if !material && !medium {
                return Err(invalid("clear_local_binding requests nothing"));
            }
            Edit::ClearLocalBinding {
                node: project::NodeId(node_id),
                material,
                medium,
            }
        }
        dto::ProjectEdit::SetReverseOrientation { node_id, enabled } => {
            Edit::SetReverseOrientation {
                node: project::NodeId(node_id),
                enabled,
            }
        }
        dto::ProjectEdit::SetLocalMatrix { node_id, matrix } => Edit::SetLocalTransform {
            node: project::NodeId(node_id),
            value: project::LocalTransform::Matrix(matrix),
        },
        dto::ProjectEdit::SetLocalTrs {
            node_id,
            translation,
            rotation_euler_degrees,
            scale,
        } => Edit::SetLocalTransform {
            node: project::NodeId(node_id),
            value: project::LocalTransform::Trs(project::transform_trs::Trs {
                translation,
                rotation_euler_degrees,
                scale,
            }),
        },
        // Resolved against the document by `plan_implementation` and
        // `plan_connection`.
        dto::ProjectEdit::SetImplementation { .. }
        | dto::ProjectEdit::ConnectTexture { .. }
        | dto::ProjectEdit::DisconnectTexture { .. } => {
            return Err(invalid("this edit needs the transaction's document"));
        }
        dto::ProjectEdit::SetAssetReference {
            target: where_,
            name,
            asset,
        } => {
            if name.is_empty() || name.len() > 256 {
                return Err(invalid("invalid parameter name length"));
            }
            Edit::SetAssetReference {
                target: target(where_),
                parameter: name,
                asset: project::AssetId(asset),
            }
        }
    })
}
/// The edit, change, and reference-break details for one `set_implementation`, as found by `plan_implementation`.
pub(super) struct ImplementationPlan {
    /// The session edit (the current name again when nothing changes).
    pub edit: Edit,
    /// None when the names agree after alias normalization.
    pub change: Option<ImplementationChange>,
    /// Wire `set_parameter` removals for `change.breaks_reference`: with the
    /// `set_implementation` itself they make the edit list that applies the
    /// change in one transaction.
    pub removals: Vec<Value>,
}

/// `set_implementation` on `target` against
/// `document` as the transaction has edited it so far. Shared by
/// `project.apply` and the read-only `project.plan_implementation`, so the
/// desktop's confirmation shows exactly what an
/// apply would do.
pub(super) fn plan_implementation(
    document: &project::ProjectDocument,
    active: Option<project::SceneId>,
    where_: dto::ParameterTarget,
    requested: &str,
) -> Result<ImplementationPlan, ErrorPayload> {
    use project::{ConfigurationKind as C, EntityKind as E, NodeKind, ResourceKind as R};
    if requested.len() > 128 {
        return Err(invalid("invalid implementation length"));
    }
    let not_found = |what| ErrorPayload::new(ErrorCode::NotFound, format!("{what} not found"));
    let (directive, current, texture_value_type, parameters) = match where_ {
        dto::ParameterTarget::Node(id) => {
            let view = document
                .scene_view(active)
                .map_err(|error| invalid(error.to_string()))?;
            match &view
                .node(project::NodeId(id))
                .ok_or_else(|| not_found("node"))?
                .data
            {
                NodeKind::Entity {
                    entity,
                    implementation,
                    parameters,
                    ..
                } => (
                    match entity {
                        E::Shape => "Shape",
                        E::Light => "LightSource",
                        E::AreaLight => "AreaLightSource",
                        E::Camera => "Camera",
                    },
                    implementation,
                    None,
                    parameters,
                ),
                NodeKind::Configuration {
                    configuration,
                    implementation,
                    parameters,
                } => (
                    match configuration {
                        C::Film => "Film",
                        C::Sampler => "Sampler",
                        C::Integrator => "Integrator",
                        C::PixelFilter => "PixelFilter",
                        C::Accelerator => "Accelerator",
                    },
                    implementation,
                    None,
                    parameters,
                ),
                _ => return Err(invalid("node has no implementation")),
            }
        }
        dto::ParameterTarget::Resource(id) => {
            let resource = document
                .resources
                .get(&project::ResourceId(id))
                .ok_or_else(|| not_found("resource"))?;
            let directive = match resource.kind {
                R::Material => "MakeNamedMaterial",
                R::Texture => "Texture",
                R::Medium => "MakeNamedMedium",
                R::ObjectDefinition => {
                    return Err(invalid("object definitions have no implementation"));
                }
                R::SpectrumData => {
                    return Err(invalid("spectrum data has no implementation"));
                }
            };
            (
                directive,
                &resource.implementation,
                resource.texture_value_type,
                &resource.parameters,
            )
        }
    };
    let texture_types = document
        .resources
        .iter()
        .filter_map(|(id, resource)| {
            resource
                .texture_value_type
                .map(|value_type| (*id, value_type))
        })
        .collect();
    let change = implementation_change(
        directive,
        current,
        texture_value_type,
        &texture_types,
        parameters,
        requested,
    )
    .map_err(|rejection| {
        invalid(format!(
            "cannot change {directive} {current:?} to {requested:?}"
        ))
        .with_details(json!({"reason":rejection.as_str()}))
    })?;
    let implementation = change
        .as_ref()
        .map_or_else(|| current.clone(), |change| change.implementation.clone());
    let removals = change
        .iter()
        .flat_map(|change| &change.breaks_reference)
        .filter_map(|name| parameters.iter().find(|p| p.name == *name))
        .map(|p| {
            json!({"kind":"set_parameter","target":where_,"name":p.name,
                   "value_type":p.value_type,"value":null})
        })
        .collect();
    Ok(ImplementationPlan {
        edit: Edit::SetImplementation {
            target: target(where_.clone()),
            implementation,
        },
        change,
        removals,
    })
}
/// Resolve `connect_texture` and `disconnect_texture` into
/// session edits against `document` as the transaction has edited it so
/// far: drop the current declaration (whatever its type), then declare the
/// texture reference — after creating the texture first for `create`, with
/// an id fixed here so validating and applying agree.
pub(super) fn plan_connection(
    document: &project::ProjectDocument,
    active: Option<project::SceneId>,
    edit: &dto::ProjectEdit,
) -> Result<Vec<Edit>, ErrorPayload> {
    use pbrt_io::parameter_schema::{self as schema, TextureInput};
    use project::{ResourceKind as R, TextureValueType as T};
    let (where_, parameter) = match edit {
        dto::ProjectEdit::ConnectTexture {
            target, parameter, ..
        }
        | dto::ProjectEdit::DisconnectTexture { target, parameter } => (target, parameter),
        _ => unreachable!("plan_connection plans connections"),
    };
    let refuse =
        |reason: &str, message: String| invalid(message).with_details(json!({"reason":reason}));
    let not_found = |what| ErrorPayload::new(ErrorCode::NotFound, format!("{what} not found"));
    // Materials and textures, and the scene graph's entities (a shape's
    // alpha mask): the table says which of their rows take a texture.
    let (directive, implementation, texture_value_type, parameters) = match where_ {
        dto::ParameterTarget::Resource(id) => {
            let resource = document
                .resources
                .get(&project::ResourceId(*id))
                .ok_or_else(|| not_found("resource"))?;
            let directive = match resource.kind {
                R::Material => "MakeNamedMaterial",
                R::Texture => "Texture",
                _ => {
                    return Err(refuse(
                        "not_a_texture_input",
                        "only materials and textures take textures".into(),
                    ));
                }
            };
            (
                directive,
                &resource.implementation,
                resource.texture_value_type,
                &resource.parameters,
            )
        }
        dto::ParameterTarget::Node(id) => {
            use project::{EntityKind as E, NodeKind};
            let view = document
                .scene_view(active)
                .map_err(|error| invalid(error.to_string()))?;
            let node = view
                .node(project::NodeId(*id))
                .ok_or_else(|| not_found("node"))?;
            let NodeKind::Entity {
                entity,
                implementation,
                parameters,
                ..
            } = &node.data
            else {
                return Err(refuse(
                    "not_a_texture_input",
                    "only entities among scene graph nodes take textures".into(),
                ));
            };
            let directive = match entity {
                E::Shape => "Shape",
                E::Light => "LightSource",
                E::AreaLight => "AreaLightSource",
                E::Camera => "Camera",
            };
            (directive, implementation, None, parameters)
        }
    };
    let spec = schema::schema(directive)
        .and_then(|table| schema::implementation(table, implementation))
        .and_then(|found| found.parameters(texture_value_type))
        .and_then(|rows| {
            rows.iter()
                .find(|spec| spec.name == parameter || spec.aliases.contains(&parameter.as_str()))
        })
        .filter(|spec| spec.texture.is_some())
        .ok_or_else(|| {
            refuse(
                "not_a_texture_input",
                format!("{directive} {implementation} takes no texture as {parameter}"),
            )
        })?;
    let lineage = match spec.texture {
        Some(TextureInput::Float) => T::Float,
        _ => T::Spectrum,
    };
    // The declaration as written (an alias stays an alias).
    let declared = parameters
        .iter()
        .find(|p| p.name == spec.name || spec.aliases.contains(&p.name.as_str()));
    let remove = |p: &Parameter| Edit::SetParameter {
        target: target(where_.clone()),
        name: p.name.clone(),
        value_type: p.value_type.clone(),
        value: None,
    };
    let (texture, create, image) = match edit {
        // Nothing connected is nothing to do; a required operand stays.
        dto::ProjectEdit::DisconnectTexture { .. } => {
            if spec.required && declared.is_some_and(|p| p.value_type == "texture") {
                return Err(refuse(
                    "required_parameter",
                    format!("{parameter} is required; connect another texture instead"),
                ));
            }
            return Ok(declared
                .filter(|p| p.value_type == "texture")
                .map(remove)
                .into_iter()
                .collect());
        }
        dto::ProjectEdit::ConnectTexture {
            texture,
            create,
            image,
            ..
        } => (texture, create, image),
        _ => unreachable!("plan_connection plans connections"),
    };
    let mut edits = Vec::new();
    let texture = match (texture, create, image) {
        (Some(texture), None, None) => {
            let source = document
                .resources
                .get(&project::ResourceId(*texture))
                .filter(|source| source.kind == R::Texture)
                .ok_or_else(|| refuse("not_a_texture", "connect a texture resource".into()))?;
            if source.texture_value_type != Some(lineage) {
                return Err(refuse(
                    "texture_lineage",
                    format!("{parameter} takes a {lineage:?} texture"),
                ));
            }
            project::ResourceId(*texture)
        }
        (None, Some(create), None) => {
            // The same argument rules as create_resource.
            if create.label.trim().is_empty() || create.label.len() > 256 {
                return Err(invalid("invalid resource label length"));
            }
            if create.implementation.len() > 128 {
                return Err(invalid("invalid resource implementation length"));
            }
            if let Some(folder) = &create.folder
                && (folder.is_empty() || folder.len() > 256)
            {
                return Err(invalid("invalid resource folder length"));
            }
            if schema::schema("Texture")
                .and_then(|table| schema::implementation(table, &create.implementation))
                .and_then(|found| found.parameters(Some(lineage)))
                .is_none()
            {
                return Err(refuse(
                    "texture_value_type",
                    format!("{} has no {lineage:?} texture", create.implementation),
                ));
            }
            let id = project::ResourceId::new();
            edits.push(Edit::CreateResource {
                id: Some(id),
                resource_kind: R::Texture,
                implementation: create.implementation.clone(),
                label: create.label.clone(),
                folder: create.folder.clone(),
                texture_value_type: Some(lineage),
                parameters: schema::declarable_defaults(
                    "Texture",
                    &create.implementation,
                    Some(lineage),
                ),
            });
            id
        }
        (None, None, Some(image)) => {
            let asset = document
                .assets
                .get(&project::AssetId(*image))
                .ok_or_else(|| not_found("asset"))?;
            if !asset.media_type.starts_with("image/") {
                return Err(refuse(
                    "not_an_image",
                    format!("{} is not an image", asset.location),
                ));
            }
            let reads_image =
                |p: &Parameter| p.name == "filename" && p.values == V::Assets(vec![asset.id]);
            // Any imagemap of the image in this lineage, whatever its other
            // settings; the first by location
            // so the choice is stable.
            let reused = document
                .resources
                .values()
                .filter(|r| {
                    r.kind == R::Texture
                        && r.implementation == "imagemap"
                        && r.texture_value_type == Some(lineage)
                        && r.parameters.iter().any(reads_image)
                })
                .min_by(|a, b| a.location.cmp(&b.location).then(a.id.cmp(&b.id)));
            match reused {
                Some(texture) => texture.id,
                None => {
                    let id = project::ResourceId::new();
                    let label = std::path::Path::new(&asset.location)
                        .file_stem()
                        .and_then(|stem| stem.to_str())
                        .unwrap_or("image")
                        .to_owned();
                    let mut parameters =
                        schema::declarable_defaults("Texture", "imagemap", Some(lineage));
                    parameters.push(Parameter {
                        name: "filename".into(),
                        value_type: "string".into(),
                        color_space: ColorSpaceBinding::Inherit,
                        values: V::Assets(vec![asset.id]),
                    });
                    edits.push(Edit::CreateResource {
                        id: Some(id),
                        resource_kind: R::Texture,
                        implementation: "imagemap".into(),
                        label,
                        folder: None,
                        texture_value_type: Some(lineage),
                        parameters,
                    });
                    id
                }
            }
        }
        _ => {
            return Err(invalid(
                "connect_texture takes exactly one of texture, create and image",
            ));
        }
    };
    // A different declared type goes first; a texture is replaced in place.
    if let Some(p) = declared.filter(|p| p.value_type != "texture") {
        edits.push(remove(p));
    }
    let name = declared.map_or_else(|| spec.name.to_owned(), |p| p.name.clone());
    edits.push(Edit::SetParameter {
        target: target(where_.clone()),
        name: name.clone(),
        value_type: "texture".into(),
        value: Some(Parameter {
            name,
            value_type: "texture".into(),
            color_space: ColorSpaceBinding::Inherit,
            values: V::Resources {
                kind: R::Texture,
                ids: vec![texture],
            },
        }),
    });
    Ok(edits)
}

fn material_binding(binding: dto::MaterialBinding) -> project::MaterialBinding {
    match binding {
        dto::MaterialBinding::Default => project::MaterialBinding::Default,
        dto::MaterialBinding::None => project::MaterialBinding::None,
        dto::MaterialBinding::Resource(id) => {
            project::MaterialBinding::Resource(project::ResourceId(id))
        }
    }
}

/// Wire up-axis words are the project model's words.
pub(super) fn up_axis(value: dto::UpAxis) -> project::Axis {
    match value {
        dto::UpAxis::PosX => project::Axis::PosX,
        dto::UpAxis::NegX => project::Axis::NegX,
        dto::UpAxis::PosY => project::Axis::PosY,
        dto::UpAxis::NegY => project::Axis::NegY,
        dto::UpAxis::PosZ => project::Axis::PosZ,
        dto::UpAxis::NegZ => project::Axis::NegZ,
    }
}

pub(super) fn wire_handedness(value: dto::Handedness) -> project::Handedness {
    match value {
        dto::Handedness::Left => project::Handedness::Left,
        dto::Handedness::Right => project::Handedness::Right,
    }
}
fn label_limit(label: &str) -> Result<(), ErrorPayload> {
    if label.len() > 1024 {
        Err(invalid("labels are limited to 1024 UTF-8 bytes"))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod array_edit_tests {
    use super::*;

    /// An array edit queues I/O work like an import: a full queue answers
    /// Busy rather than piling up plans and snapshots.
    #[test]
    fn array_edits_respect_the_project_io_queue_limit() {
        assert!(crate::state::project_commands::queues_project_io(
            &Method::ProjectSetParameterValues
        ));
        assert!(!crate::state::project_commands::queues_project_io(
            &Method::ProjectApply
        ));
    }

    /// A node is found in the active scene when the job publishes; a scene
    /// switch since submission is refused with a reason instead of failing
    /// on a missing node. Resources do not depend on the scene.
    #[test]
    fn a_node_window_edit_needs_the_scene_it_was_planned_in() {
        let (first, second) = (project::SceneId::new(), project::SceneId::new());
        let node = project::ParameterTarget::Node(project::NodeId::new());
        let resource = project::ParameterTarget::Resource(project::ResourceId::new());
        assert!(scene_still_holds(&node, Some(first), Some(first)).is_ok());
        let error = scene_still_holds(&node, Some(first), Some(second)).unwrap_err();
        assert_eq!(error.code, ErrorCode::StaleProject);
        assert_eq!(error.details.unwrap()["reason"], "scene_changed");
        assert!(scene_still_holds(&node, Some(first), None).is_err());
        assert!(scene_still_holds(&resource, Some(first), Some(second)).is_ok());
    }
}
