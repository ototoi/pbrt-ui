//! Texture reference lineage checks against the same parameter tables used
//! by connection planning. Every scene and definition participates.
use crate::parameter_schema::{self, TextureInput};
use project::*;
use serde_json::{Value, json};

pub fn texture_type_mismatches(document: &ProjectDocument) -> Vec<Value> {
    let mut errors = Vec::new();
    for resource in document.resources.values() {
        let directive = match resource.kind {
            ResourceKind::Texture => "Texture",
            ResourceKind::Material => "MakeNamedMaterial",
            ResourceKind::Medium => "MakeNamedMedium",
            ResourceKind::ObjectDefinition | ResourceKind::SpectrumData => continue,
        };
        if resource.kind == ResourceKind::Texture
            && parameter_schema::schema(directive)
                .and_then(|table| parameter_schema::implementation(table, &resource.implementation))
                .and_then(|implementation| implementation.parameters(resource.texture_value_type))
                .is_none()
        {
            errors
                .push(json!({"resource_id":resource.id,"reason":"unsupported_texture_value_type"}));
        }
        check(
            document,
            directive,
            &resource.implementation,
            resource.texture_value_type,
            &resource.parameters,
            json!({"kind":"resource","id":resource.id}),
            &mut errors,
        );
    }
    for node in document
        .scenes
        .values()
        .flat_map(|scene| scene.nodes.values())
        .chain(document.definition_nodes.values())
    {
        let (directive, implementation, parameters) = match &node.data {
            NodeKind::Entity {
                entity,
                implementation,
                parameters,
                ..
            } => (
                match entity {
                    EntityKind::Shape => "Shape",
                    EntityKind::Light => "LightSource",
                    EntityKind::AreaLight => "AreaLightSource",
                    EntityKind::Camera => "Camera",
                },
                implementation,
                parameters,
            ),
            NodeKind::Configuration {
                configuration,
                implementation,
                parameters,
            } => (
                match configuration {
                    ConfigurationKind::Film => "Film",
                    ConfigurationKind::Sampler => "Sampler",
                    ConfigurationKind::Integrator => "Integrator",
                    ConfigurationKind::PixelFilter => "PixelFilter",
                    ConfigurationKind::Accelerator => "Accelerator",
                },
                implementation,
                parameters,
            ),
            _ => continue,
        };
        check(
            document,
            directive,
            implementation,
            None,
            parameters,
            json!({"kind":"node","id":node.id}),
            &mut errors,
        );
    }
    errors
}

fn check(
    document: &ProjectDocument,
    directive: &str,
    implementation: &str,
    lineage: Option<TextureValueType>,
    parameters: &[Parameter],
    owner: Value,
    errors: &mut Vec<Value>,
) {
    let table = parameter_schema::schema(directive)
        .and_then(|table| parameter_schema::implementation(table, implementation))
        .and_then(|implementation| implementation.parameters(lineage));
    for parameter in parameters {
        if directive == "Texture" && !matches!(parameter.values, ParameterValues::Resources { .. })
        {
            if let Some(spec) = table.and_then(|rows| {
                rows.iter().find(|row| {
                    row.name == parameter.name || row.aliases.contains(&parameter.name.as_str())
                })
            }) {
                if !spec.value_types.contains(&parameter.value_type.as_str()) {
                    errors.push(json!({"reference_from":owner,"parameter":parameter.name,"expected":spec.value_types,"actual":parameter.value_type,"reason":"literal_type_mismatch"}));
                }
            }
        }
        let ParameterValues::Resources {
            kind: ResourceKind::Texture,
            ids,
        } = &parameter.values
        else {
            continue;
        };
        let input = table
            .and_then(|rows| {
                rows.iter().find(|row| {
                    row.name == parameter.name || row.aliases.contains(&parameter.name.as_str())
                })
            })
            .and_then(|row| row.texture);
        let expected = input.map(|input| match input {
            TextureInput::Float => TextureValueType::Float,
            _ => TextureValueType::Spectrum,
        });
        for id in ids {
            let actual = document
                .resources
                .get(id)
                .and_then(|resource| resource.texture_value_type);
            if expected.is_none() || actual != expected {
                errors.push(json!({"texture_id":id,"reference_from":owner,"parameter":parameter.name,"expected":expected,"actual":actual}));
            }
        }
    }
}
