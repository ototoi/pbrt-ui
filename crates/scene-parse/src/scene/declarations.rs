use super::*;

pub(super) struct Declarations {
    pub(super) shapes: Vec<Shape>,
    pub(super) diagnostics: Vec<Diagnostic>,
    pub(super) camera: Option<CameraRecord>,
    pub(super) instances: Vec<InstanceUse>,
    pub(super) lights: Vec<LightRecord>,
    /// `Texture` declarations; imagemap bytes ride in geometry blobs under
    /// `texture/{value_type}/{name}` (filled in by the geometry builder).
    pub(super) textures: Vec<TextureRecord>,
    pub(super) film_resolution: Option<[u32; 2]>,
    pub(super) film_exposure: Option<f64>,
    /// `MakeNamedMaterial` definitions by name, for materials a bound one
    /// refers to (a `mix`'s children).
    pub(super) named_materials: std::collections::BTreeMap<String, MaterialRecord>,
}
pub(super) fn collect(
    document: crate::document::Document,
    entity_of: Option<&BTreeMap<u32, protocol::EntityId>>,
) -> Result<Declarations, SceneParseError> {
    let graph = parse_state_graph(&document.directives)?;
    let directives = document.directives;
    // Named material definitions are collected in a first pass so a
    // `NamedMaterial` binding is resolved regardless of where its
    // `MakeNamedMaterial` definition appears in the document.
    let mut named_materials = std::collections::BTreeMap::<String, MaterialRecord>::new();
    for d in &directives {
        if d.name == "MakeNamedMaterial" {
            let Some(name) = d.positional_text(0) else {
                continue;
            };
            named_materials.insert(name.to_owned(), named_material_record(d, name));
        }
    }
    let mut shapes: Vec<Shape> = Vec::new();
    let mut diagnostics = Vec::new();
    let mut camera = None;
    let mut definition = None;
    let mut local_materials = std::collections::BTreeMap::<usize, MaterialRecord>::new();
    let mut lights = Vec::new();
    let mut named_textures = std::collections::BTreeMap::<(String, String), usize>::new();
    let mut textures = Vec::<TextureRecord>::new();
    // One media_not_previewed note per declared medium: the preview
    // cannot draw participating media, but the project keeps them.
    let mut media_seen = std::collections::BTreeSet::<(String, String)>::new();
    // AreaLightSource records by directive index; the state graph scopes
    // which one (if any) each Shape is under, as pbrt's graphics state does.
    let mut area_lights =
        std::collections::BTreeMap::<usize, (AreaLightRecord, String, u32, bool)>::new();
    let mut definitions = std::collections::BTreeSet::new();
    let mut instances = Vec::<InstanceUse>::new();
    let mut film_resolution = None;
    let mut film_exposure = None;
    for (d, node) in directives.into_iter().zip(graph.nodes) {
        let state = node.state;
        match d.name.as_str() {
            "ObjectBegin" => {
                let name = d.positional_text(0).unwrap_or("").to_owned();
                definitions.insert(name.clone());
                definition = Some(name);
            }
            "ObjectEnd" => definition = None,
            "ObjectInstance" => instances.push(InstanceUse {
                name: d.positional_text(0).unwrap_or("").into(),
                source: SourceLocation {
                    path: d.path.clone(),
                    line: d.line,
                    column: d.column,
                },
                start_transform: state.start,
                end_transform: state.end,
                times: state.times,
            }),
            "Film" => {
                let params = d.params();
                let has_scale = d
                    .parameters
                    .iter()
                    .any(|parameter| parameter.name == "scale");
                let has_iso = d.parameters.iter().any(|parameter| parameter.name == "iso");
                let scale = params.float("scale", 1.);
                let iso = params.float("iso", 100.);
                if has_scale || has_iso {
                    if scale.is_finite() && iso.is_finite() {
                        film_exposure = Some(scale * iso.max(0.) / 100.);
                    }
                }
                let width = params.float("xresolution", 1280.);
                let height = params.float("yresolution", 720.);
                if width.is_finite()
                    && height.is_finite()
                    && width >= 1.
                    && height >= 1.
                    && width <= u32::MAX as f64
                    && height <= u32::MAX as f64
                {
                    film_resolution = Some([width as u32, height as u32]);
                }
            }
            "Camera" => {
                if let Some(matrix) = state.camera_to_world() {
                    let eye = matrix.transform_point([0., 0., 0.]);
                    let target = matrix.transform_point([0., 0., 1.]);
                    let up_point = matrix.transform_point([0., 1., 0.]);
                    let up = [
                        up_point[0] - eye[0],
                        up_point[1] - eye[1],
                        up_point[2] - eye[2],
                    ];
                    let params = d.params();
                    camera = Some(CameraRecord {
                        kind: d.positional_text(0).unwrap_or("perspective").into(),
                        camera_from_world: matrix.0,
                        look_at: Some([eye, target, up]),
                        fov_degrees: Some(params.float("fov", 45.)),
                        screenwindow: <[f64; 4]>::try_from(params.floats("screenwindow"))
                            .ok()
                            .filter(|w| w.iter().all(|v| v.is_finite())),
                        frame_aspect_ratio: Some(params.float("frameaspectratio", 0.))
                            .filter(|f| f.is_finite() && *f > 0.),
                        shutter: [
                            params.float("shutteropen", 0.),
                            params.float("shutterclose", 1.),
                        ],
                        film_resolution: None,
                        film_exposure: None,
                    });
                }
            }
            "Material" if !matches!(d.positional_text(0), None | Some("interface")) => {
                local_materials.insert(node.directive, inline_material_record(&d));
            }
            "AreaLightSource" if !d.positional_text(0).is_none_or(str::is_empty) => {
                let record = AreaLightRecord {
                    implementation: d.positional_text(0).unwrap_or("").to_owned(),
                    source: Some(SourceLocation {
                        path: d.path.clone(),
                        line: d.line,
                        column: d.column,
                    }),
                    parameters: preview_parameters(&d.parameters).0,
                };
                area_lights.insert(node.directive, (record, d.path.clone(), d.line, false));
            }
            "Texture" => {
                // pbrt keeps float and spectrum texture names in separate
                // namespaces; the declaration CTM is part of each record.
                if let Some(name) = d.positional_text(0) {
                    let value_type = d.positional_text(1).unwrap_or("spectrum").to_owned();
                    let implementation = d.positional_text(2).unwrap_or_default().to_owned();
                    let record = TextureRecord {
                        name: name.to_owned(),
                        value_type: value_type.clone(),
                        implementation,
                        world_from_texture: state.start.0,
                        source: Some(source_of(&d)),
                        parameters: preview_parameters(&d.parameters).0,
                    };
                    let key = (value_type, name.to_owned());
                    match named_textures.get(&key) {
                        Some(slot) => textures[*slot] = record,
                        None => {
                            named_textures.insert(key, textures.len());
                            textures.push(record);
                        }
                    }
                }
            }
            "LightSource" => {
                if definition.is_some() {
                    diagnostics.push(
                        Diagnostic::warning(
                            "light_inside_object_definition",
                            "LightSource inside ObjectBegin is ignored",
                        )
                        .at(&d.path, d.line),
                    );
                    continue;
                }
                let implementation = d.positional_text(0).unwrap_or("").to_owned();
                if implementation.is_empty() {
                    continue;
                }
                lights.push(LightRecord {
                    implementation,
                    world_from_light: state.start.0,
                    // The origin table pins the record
                    // to the light node that emitted it — the same join the
                    // draws resolve their `entity_id` by.
                    entity_id: entity_of.and_then(|table| table.get(&d.line).cloned()),
                    source: Some(SourceLocation {
                        path: d.path.clone(),
                        line: d.line,
                        column: d.column,
                    }),
                    parameters: preview_parameters(&d.parameters).0,
                });
            }
            "MakeNamedMedium" => {
                let name = d.positional_text(0).unwrap_or("").to_owned();
                let medium_type = d
                    .parameters
                    .iter()
                    .find(|parameter| parameter.matches("type"))
                    .and_then(|parameter| {
                        parameter
                            .values
                            .iter()
                            .find_map(|value| value.as_text().map(str::to_owned))
                    })
                    .unwrap_or_default();
                if media_seen.insert((name.clone(), medium_type.clone())) {
                    diagnostics.push(
                        Diagnostic::info(
                            "media_not_previewed",
                            format!(
                                "participating media are not drawn in Solid/Lit \
                                 (MakeNamedMedium \"{name}\", type {medium_type}); \
                                 render with r4 to see them"
                            ),
                        )
                        .at(&d.path, d.line),
                    );
                }
            }
            "Shape" => {
                if d.parameters.iter().any(|p| p.matches("shadowalpha")) {
                    diagnostics.push(
                        Diagnostic::warning(
                            "shadowalpha_unsupported",
                            "the preview ignores shadowalpha; shadows use alpha",
                        )
                        .at(&d.path, d.line),
                    );
                }
                let alpha = preview_parameters(&d.parameters)
                    .0
                    .into_iter()
                    .find(|p| p.matches("alpha"));
                let material = resolve_material(
                    &state.material,
                    &named_materials,
                    &local_materials,
                    &mut diagnostics,
                    &d,
                );
                shapes.push(Shape {
                    d,
                    start_transform: state.start,
                    end_transform: state.end,
                    times: state.times,
                    reverse: state.reverse,
                    index: shapes.len(),
                    definition: definition.clone(),
                    material,
                    interface_material: state.interface_material,
                    alpha,
                    area_light: state.area_light.and_then(|index| {
                        area_lights.get_mut(&index).map(|entry| {
                            entry.3 = true;
                            entry.0.clone()
                        })
                    }),
                });
            }
            _ => {}
        }
    }
    // An AreaLightSource no Shape follows in its scope lights nothing.
    for (_, path, line, used) in area_lights.values() {
        if !used {
            diagnostics.push(
                Diagnostic::warning(
                    "area_light_without_shape",
                    "AreaLightSource is not followed by a Shape in its scope",
                )
                .at(path, *line),
            );
        }
    }
    for instance in &instances {
        let issue = if !definitions.contains(&instance.name) {
            Some((
                "instance_definition_unresolved",
                "ObjectInstance names an unknown definition",
            ))
        } else {
            None
        };
        if let Some((code, message)) = issue {
            diagnostics.push(
                Diagnostic::warning(code, message).at(&instance.source.path, instance.source.line),
            );
        }
    }
    Ok(Declarations {
        shapes,
        diagnostics,
        camera,
        instances,
        lights,
        textures,
        film_resolution,
        film_exposure,
        named_materials,
    })
}

/// Keep only shading-relevant values within the transfer limits. Mixed payloads
/// keep source order in `floats` / `texts`; anything dropped sets `truncated`.
fn preview_parameters(parameters: &[crate::params::Parameter]) -> (Vec<PreviewParameter>, bool) {
    let mut out = Vec::new();
    let mut truncated = false;
    for parameter in parameters {
        if out.len() == MAX_PREVIEW_PARAMETERS {
            truncated = true;
            break;
        }
        let mut floats = Vec::new();
        let mut texts = Vec::new();
        let mut parameter_truncated = false;
        for value in &parameter.values {
            let (float_value, text_value) = match value {
                crate::params::Value::Number(value) => (Some(*value), None),
                crate::params::Value::Text(text) => {
                    (crate::lexer::special_number(text), Some(text.as_str()))
                }
            };
            if let Some(value) = float_value {
                if floats.len() == MAX_PREVIEW_PARAMETER_VALUES || !value.is_finite() {
                    parameter_truncated = true;
                    continue;
                }
                floats.push(value);
                continue;
            }
            if let Some(text) = text_value {
                if texts.len() == MAX_PREVIEW_PARAMETER_VALUES {
                    parameter_truncated = true;
                    continue;
                }
                let clipped: String = text
                    .chars()
                    .take(MAX_PREVIEW_PARAMETER_TEXT_CHARS)
                    .collect();
                parameter_truncated |= clipped.chars().count() < text.chars().count();
                texts.push(clipped);
            }
        }
        truncated |= parameter_truncated;
        out.push(PreviewParameter {
            name: parameter.name.clone(),
            value_type: parameter.value_type.clone(),
            floats,
            texts,
            truncated: parameter_truncated,
        });
    }
    (out, truncated)
}

fn source_of(d: &Directive) -> SourceLocation {
    SourceLocation {
        path: d.path.clone(),
        line: d.line,
        column: d.column,
    }
}

fn inline_material_record(d: &Directive) -> MaterialRecord {
    MaterialRecord {
        implementation: d.positional_text(0).unwrap_or("").to_owned(),
        resource_id: None,
        name: None,
        source: Some(source_of(d)),
        parameters: preview_parameters(&d.parameters).0,
    }
}

fn named_material_record(d: &Directive, name: &str) -> MaterialRecord {
    // `MakeNamedMaterial "name" "string type" "impl" ...` carries the
    // implementation in its `type` parameter, which is not a shading value.
    let implementation = d
        .parameters
        .iter()
        .find(|parameter| parameter.matches("type"))
        .and_then(|parameter| parameter.values.iter().find_map(|value| value.as_text()))
        .unwrap_or("")
        .to_owned();
    let parameters = preview_parameters(
        &d.parameters
            .iter()
            .filter(|parameter| !parameter.matches("type"))
            .cloned()
            .collect::<Vec<_>>(),
    )
    .0;
    MaterialRecord {
        implementation,
        resource_id: resource_id_of(name),
        name: Some(name.to_owned()),
        source: Some(source_of(d)),
        parameters,
    }
}

/// Project export names resources `resource_{id}`, so the join is direct.
fn resource_id_of(name: &str) -> Option<protocol::EntityId> {
    name.strip_prefix("resource_")
        .filter(|id| !id.is_empty())
        .map(protocol::EntityId::new)
}

fn resolve_material(
    binding: &serde_json::Value,
    named_materials: &std::collections::BTreeMap<String, MaterialRecord>,
    local_materials: &std::collections::BTreeMap<usize, MaterialRecord>,
    diagnostics: &mut Vec<Diagnostic>,
    d: &Directive,
) -> Option<MaterialRecord> {
    match binding.get("kind").and_then(serde_json::Value::as_str) {
        Some("directive") => binding
            .get("directive")
            .and_then(serde_json::Value::as_u64)
            .and_then(|ordinal| local_materials.get(&(ordinal as usize)))
            .cloned(),
        Some("named") => {
            let name = binding.get("name").and_then(serde_json::Value::as_str)?;
            match named_materials.get(name) {
                Some(record) => Some(record.clone()),
                None => {
                    diagnostics.push(
                        Diagnostic::warning(
                            "named_material_unresolved",
                            format!("NamedMaterial \"{name}\" has no MakeNamedMaterial definition"),
                        )
                        .at(&d.path, d.line),
                    );
                    None
                }
            }
        }
        _ => None,
    }
}
