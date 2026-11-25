use crate::*;
use project::*;

/// What a rejected `{directive} "{implementation}"` declaration costs the
/// user, so the skip reason reads as a sentence instead of naming the
/// internal schema check.
pub(crate) fn unsupported_declaration_reason(directive: &str, implementation: &str) -> String {
    format!(
        "{directive} \"{implementation}\" is not supported by the importer \
         (the {} is dropped)",
        dropped_noun(directive)
    )
}

/// The same reason for one parameter the schema does not manage: `kind` is
/// the parameter's role ("string", "file" or "resource"), `name` its own.
pub(crate) fn unsupported_parameter_reason(
    directive: &str,
    implementation: &str,
    name: &str,
    kind: &str,
) -> String {
    format!(
        "{directive} \"{implementation}\" uses the \"{name}\" {kind} parameter, \
         which the importer does not support (the {} is dropped)",
        dropped_noun(directive)
    )
}

/// What the importer drops with the directive: the thing the Hierarchy
/// would show.
fn dropped_noun(directive: &str) -> &'static str {
    match directive {
        "Shape" => "shape",
        "LightSource" | "AreaLightSource" => "light",
        "Camera" => "camera",
        "Material" | "MakeNamedMaterial" => "material",
        "Texture" => "texture",
        "MakeNamedMedium" => "medium",
        "Film" => "film",
        "Sampler" => "sampler",
        "Integrator" => "integrator",
        "PixelFilter" => "pixel filter",
        "Accelerator" => "accelerator",
        _ => "directive",
    }
}

/// The implementations the export schema supports for one directive, in
/// the order editors list them (selector candidates
/// fall back to these where the parameter table has no rows). The area
/// light's "" is the explicit "no area light".
pub(crate) fn implementations(directive: &str) -> &'static [&'static str] {
    match directive {
        "Shape" => &[
            "sphere",
            "cylinder",
            "disk",
            "trianglemesh",
            "plymesh",
            "bilinearmesh",
            "curve",
            "curves",
            "loopsubdiv",
            "cone",
            "paraboloid",
            "hyperboloid",
            "nurbs",
        ],
        "Material" | "MakeNamedMaterial" => &[
            "diffuse",
            "diffusetransmission",
            "dielectric",
            "thindielectric",
            "conductor",
            "coateddiffuse",
            "coatedconductor",
            "hair",
            "mix",
            "subsurface",
            "measured",
        ],
        "Texture" => &[
            "constant",
            "imagemap",
            "scale",
            "mix",
            "directionmix",
            "checkerboard",
            "dots",
            "fbm",
            "wrinkled",
            "windy",
            "marble",
            "bilerp",
        ],
        // The five media the exporter can round trip; r4 implements all
        // five (medium.rs). Grid media stay free parameters (no parameter
        // table), so the Inspector and GUI candidates do not widen.
        "MakeNamedMedium" => &["homogeneous", "uniformgrid", "rgbgrid", "cloud", "nanovdb"],
        "Camera" => &["perspective", "orthographic", "realistic", "spherical"],
        "LightSource" => &[
            "point",
            "distant",
            "spot",
            "infinite",
            "goniometric",
            "projection",
        ],
        "AreaLightSource" => &["diffuse", ""],
        "Film" => &["rgb", "gbuffer", "spectral"],
        "Sampler" => &[
            "halton",
            "sobol",
            "zsobol",
            "paddedsobol",
            "independent",
            "stratified",
            "pmj02bn",
        ],
        "Integrator" => &[
            "path",
            "volpath",
            "simplepath",
            "simplevolpath",
            "ao",
            "randomwalk",
            "bdpt",
            "mlt",
            "sppm",
            "lightpath",
        ],
        "PixelFilter" => &["box", "gaussian", "mitchell", "sinc", "triangle"],
        "Accelerator" => &["bvh", "kdtree"],
        _ => &[],
    }
}

pub(crate) fn supports(directive: &str, implementation: &str) -> bool {
    implementations(directive).contains(&implementation)
}

pub(crate) fn input_file(directive: &str, implementation: &str, parameter: &str) -> bool {
    match parameter {
        "normalmap" => matches!(directive, "Material" | "MakeNamedMaterial"),
        "lensfile" => directive == "Camera" && implementation == "realistic",
        "filename" => {
            (directive == "Shape" && implementation == "plymesh")
                || (directive == "Texture" && implementation == "imagemap")
                || (directive == "LightSource"
                    && matches!(implementation, "infinite" | "goniometric" | "projection"))
                || (matches!(directive, "Material" | "MakeNamedMaterial")
                    && implementation == "measured")
                || (directive == "AreaLightSource" && implementation == "diffuse")
                // nanovdb's grid payload: an input file like any image.
                || (directive == "MakeNamedMedium" && implementation == "nanovdb")
        }
        _ => false,
    }
}
pub(crate) fn plain_string(directive: &str, parameter: &str) -> bool {
    matches!(
        parameter,
        "type"
            | "mapping"
            | "wrap"
            | "swrap"
            | "twrap"
            | "filter"
            | "encoding"
            // Names, never paths: checkerboard's antialiasing and the
            // homogeneous medium's scattering preset.
            | "aamode"
            | "preset"
            | "sensor"
            | "randomization"
            | "splitmethod"
            | "lightsamplestrategy"
            | "coordinatesystem"
            | "mode"
    ) || (directive == "Film" && parameter == "filename")
        // nanovdb names a grid within its file; MakeNamedMedium only,
        // because the name is only meaningful there.
        || (directive == "MakeNamedMedium"
            && matches!(parameter, "gridname" | "temperaturename"))
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OptionType {
    Bool,
    Integer,
    Float,
    String,
}

pub(crate) fn option_type(name: &str) -> Option<OptionType> {
    match name.to_ascii_lowercase().as_str() {
        "seed" => Some(OptionType::Integer),
        "rendercoordsys" => Some(OptionType::String),
        "disablepixeljitter"
        | "disablewavelengthjitter"
        | "disabletexturefiltering"
        | "forcediffuse"
        | "pixelstats" => Some(OptionType::Bool),
        "displacementedgescale" => Some(OptionType::Float),
        _ => None,
    }
}

/// Convert one lexical Option token using the dependency schema's type table.
pub(crate) fn parse_option(name: &str, token: &str) -> crate::Result<OptionValue> {
    let kind = option_type(name)
        .ok_or_else(|| crate::unsupported(format!("Option dependency schema: {name}")))?;
    let text = token.trim();
    let value = match kind {
        OptionType::Bool => match text {
            "true" => OptionValue::Bool(true),
            "false" => OptionValue::Bool(false),
            _ => return Err(crate::invalid(format!("invalid Option {name}: {token}"))),
        },
        OptionType::Integer => OptionValue::Integer(
            text.parse::<i64>()
                .map_err(|_| crate::invalid(format!("invalid Option {name}: {token}")))?,
        ),
        OptionType::Float => {
            let value = text
                .parse::<f64>()
                .map_err(|_| crate::invalid(format!("invalid Option {name}: {token}")))?;
            if !value.is_finite() {
                return Err(crate::invalid(format!("invalid Option {name}: {token}")));
            }
            OptionValue::Float(FloatValue::Finite(value))
        }
        OptionType::String => OptionValue::String(unquote(text)?),
    };
    option(name, &value)?;
    Ok(value)
}

fn unquote(token: &str) -> crate::Result<String> {
    if token.len() < 2 || !token.starts_with('"') || !token.ends_with('"') {
        // Bare enum spellings (`Option "rendercoordsys" camera`) are strings too.
        return Ok(token.to_owned());
    }
    let text = &token[1..token.len() - 1];
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
            None => return Err(crate::invalid("dangling Option escape")),
        }
    }
    Ok(out)
}

pub(crate) fn option(name: &str, value: &OptionValue) -> crate::Result<()> {
    let kind = option_type(name)
        .ok_or_else(|| crate::unsupported(format!("Option dependency schema: {name}")))?;
    let valid = match (kind, value) {
        (OptionType::Bool, OptionValue::Bool(_)) => true,
        (OptionType::Integer, OptionValue::Integer(value)) => u32::try_from(*value).is_ok(),
        (OptionType::Float, OptionValue::Float(FloatValue::Finite(value))) => value.is_finite(),
        (OptionType::String, OptionValue::String(value)) => {
            matches!(name.to_ascii_lowercase().as_str(), "rendercoordsys")
                && matches!(value.as_str(), "camera" | "cameraworld" | "world")
        }
        _ => false,
    };
    if valid {
        Ok(())
    } else {
        Err(crate::invalid(format!("invalid Option {name}")))
    }
}
/// One declared parameter's dependency check under `directive
/// implementation` — the per-parameter half of `declaration`, shared with
/// FX7's implementation change, which must know which kept parameters a
/// new implementation could no longer export.
pub(crate) fn parameter(directive: &str, implementation: &str, p: &Parameter) -> crate::Result<()> {
    let file = input_file(directive, implementation, &p.name);
    let material_refs = matches!(directive, "Material" | "MakeNamedMaterial")
        && implementation == "mix"
        && p.name == "materials";
    match &p.values {
        ParameterValues::Strings(_)
            if file || material_refs || !plain_string(directive, &p.name) =>
        {
            return Err(unsupported(unsupported_parameter_reason(
                directive,
                implementation,
                &p.name,
                "string",
            )));
        }
        ParameterValues::Assets(_) if p.value_type != "spectrum" && !file => {
            return Err(unsupported(unsupported_parameter_reason(
                directive,
                implementation,
                &p.name,
                "file",
            )));
        }
        ParameterValues::Resources { kind, .. }
            if !(p.value_type == "texture" && *kind == ResourceKind::Texture
                || p.value_type == "string"
                    && material_refs
                    && *kind == ResourceKind::Material
                || p.value_type == "spectrum" && *kind == ResourceKind::SpectrumData) =>
        {
            return Err(unsupported(unsupported_parameter_reason(
                directive,
                implementation,
                &p.name,
                "resource",
            )));
        }
        ParameterValues::BuiltinSpectra(names)
            if names.iter().any(|name| !crate::spectra::is_builtin(name)) =>
        {
            return Err(unsupported("unknown builtin spectrum"));
        }
        _ => {}
    }
    Ok(())
}
pub(crate) fn declaration(
    directive: &str,
    implementation: &str,
    parameters: &[Parameter],
) -> crate::Result<()> {
    if !supports(directive, implementation) {
        return Err(unsupported(unsupported_declaration_reason(
            directive,
            implementation,
        )));
    }
    for p in parameters {
        parameter(directive, implementation, p)?;
    }
    Ok(())
}
pub(crate) fn exportable(document: &ProjectDocument) -> crate::Result<()> {
    let scene_nodes = document
        .scenes
        .values()
        .flat_map(|scene| scene.nodes.values());
    for node in scene_nodes.chain(document.definition_nodes.values()) {
        check_node_exportable(node)?;
    }
    for r in document.resources.values() {
        check_resource_exportable(r)?;
    }
    Ok(())
}

/// Reject required resource references while preserving the existing policy
/// for required values and files (such as unused imagemap warnings).
pub(crate) fn missing_required_resources(document: &ProjectDocument) -> crate::Result<()> {
    use crate::parameter_schema::{self, Role};
    use project::{ParameterValues, ResourceKind};
    let mut missing = Vec::new();
    for resource in document.resources.values() {
        let directive = match resource.kind {
            ResourceKind::Material => "MakeNamedMaterial",
            ResourceKind::Texture => "Texture",
            ResourceKind::Medium => "MakeNamedMedium",
            _ => continue,
        };
        let Some(rows) = parameter_schema::schema(directive)
            .and_then(|table| parameter_schema::implementation(table, &resource.implementation))
            .and_then(|implementation| implementation.parameters(resource.texture_value_type))
        else {
            continue;
        };
        for row in rows
            .iter()
            .filter(|row| row.required && row.role == Role::Resource)
        {
            let present = resource.parameters.iter().any(|parameter| {
                parameter.name == row.name
                    && matches!(&parameter.values,
                    ParameterValues::Resources { ids, .. } if !ids.is_empty())
            });
            if !present {
                let kind = match resource.kind {
                    ResourceKind::Material => "material",
                    ResourceKind::Texture => "texture",
                    _ => "medium",
                };
                missing.push((
                    kind,
                    crate::MissingRequired {
                        id: resource.id,
                        label: resource.label.clone(),
                        parameter: row.name.to_owned(),
                    },
                ));
            }
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    let message = missing
        .iter()
        .map(|(kind, item)| {
            format!(
                "{kind} \"{}\": \"{}\" is required",
                item.label, item.parameter
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    let missing = missing.into_iter().map(|(_, item)| item).collect();
    Err(crate::Error::MissingRequired { message, missing })
}

/// Exportability of one node: the same checks
/// `exportable` applies per node, kept separate so `check_exportable` can
/// run them on a touched subset.
pub(crate) fn check_node_exportable(node: &Node) -> crate::Result<()> {
    match &node.data {
        NodeKind::Entity {
            entity,
            implementation,
            parameters,
            ..
        } => declaration(
            match entity {
                EntityKind::Shape => "Shape",
                EntityKind::Light => "LightSource",
                EntityKind::AreaLight => "AreaLightSource",
                EntityKind::Camera => "Camera",
            },
            implementation,
            parameters,
        )?,
        NodeKind::Configuration {
            configuration,
            implementation,
            parameters,
        } => declaration(
            match configuration {
                ConfigurationKind::Film => "Film",
                ConfigurationKind::Sampler => "Sampler",
                ConfigurationKind::Integrator => "Integrator",
                ConfigurationKind::PixelFilter => "PixelFilter",
                ConfigurationKind::Accelerator => "Accelerator",
            },
            implementation,
            parameters,
        )?,
        NodeKind::Option { name, value } => option(name, value)?,
        _ => {}
    }
    Ok(())
}

/// Exportability of one resource; see `check_node_exportable`.
pub(crate) fn check_resource_exportable(r: &Resource) -> crate::Result<()> {
    // Spectrum data has no PBRT declaration: the samples are written as
    // `spds/<name>.spd` (or inline pairs) at each reference site, so the
    // resource itself declares nothing to check.
    if matches!(
        r.kind,
        ResourceKind::ObjectDefinition | ResourceKind::SpectrumData
    ) {
        return Ok(());
    }
    let directive = match r.kind {
        ResourceKind::Material => "MakeNamedMaterial",
        ResourceKind::Medium => "MakeNamedMedium",
        ResourceKind::Texture => "Texture",
        _ => return Ok(()),
    };
    if matches!(r.kind, ResourceKind::Material | ResourceKind::Medium)
        && r.parameters.iter().any(|p| p.name == "type")
    {
        return Err(invalid("resource type is stored as implementation"));
    }
    declaration(directive, &r.implementation, &r.parameters)
}

/// The light-weight apply-time check: exportability
/// limited to the nodes and resources an edit touched, instead of walking
/// the whole document on every apply. Node ids are unique across the scene
/// tables and `definition_nodes`.
pub fn check_exportable(
    document: &ProjectDocument,
    nodes: &[NodeId],
    resources: &[ResourceId],
) -> crate::Result<()> {
    for id in nodes {
        let node = document
            .scenes
            .values()
            .find_map(|scene| scene.nodes.get(id))
            .or_else(|| document.definition_nodes.get(id))
            .ok_or_else(|| invalid("missing edited node"))?;
        check_node_exportable(node)?;
    }
    for id in resources {
        let resource = document
            .resources
            .get(id)
            .ok_or_else(|| invalid("missing edited resource"))?;
        check_resource_exportable(resource)?;
    }
    Ok(())
}

#[cfg(test)]
mod required_reference_tests {
    use super::missing_required_resources;
    use project::*;
    use std::sync::Arc;

    fn resource(
        label: &str,
        kind: ResourceKind,
        implementation: &str,
        value_type: Option<TextureValueType>,
        parameters: Vec<Parameter>,
    ) -> Resource {
        Resource {
            id: ResourceId::new(),
            label: label.into(),
            location: String::new(),
            edition: Revision::initial(),
            kind,
            texture_value_type: value_type,
            implementation: implementation.into(),
            parameters,
            definition_scope: None,
            declared_in: None,
            origin: None,
        }
    }

    #[test]
    fn checks_only_missing_required_resource_references() {
        let mut document = ProjectDocument::new("preflight");
        let mix = resource("mix", ResourceKind::Material, "mix", None, vec![]);
        let mix_id = mix.id;
        document.resources.insert(mix.id, mix);
        let image = resource(
            "unused image",
            ResourceKind::Texture,
            "imagemap",
            Some(TextureValueType::Spectrum),
            vec![],
        );
        document.resources.insert(image.id, image);
        let mesh = Node::new(
            "mesh",
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation: "plymesh".into(),
                parameters: vec![],
                pose: None,
            },
        );
        Arc::make_mut(
            document
                .scenes
                .get_mut(&document.default_scene.unwrap())
                .unwrap(),
        )
        .nodes
        .insert(mesh.id, mesh);
        let error = missing_required_resources(&document).unwrap_err();
        let crate::Error::MissingRequired { missing, .. } = error else {
            panic!("wrong error")
        };
        assert_eq!(missing.len(), 1);
        assert_eq!(missing[0].id, mix_id);
        assert_eq!(missing[0].parameter, "materials");

        let mix = document.resources.get_mut(&mix_id).unwrap();
        mix.parameters.push(Parameter {
            value_type: "string".into(),
            name: "materials".into(),
            color_space: Default::default(),
            values: ParameterValues::Resources {
                kind: ResourceKind::Material,
                ids: vec![ResourceId::new(), ResourceId::new()],
            },
        });
        missing_required_resources(&document).unwrap();
    }
}
