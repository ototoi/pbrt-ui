//! Changing an item's implementation name.
//!
//! `keep_compatible` never edits parameters: every declared parameter stays.
//! The change reports which of them the new implementation's table does not
//! know (`kept_unknown`, kept with a warning) and which
//! the new implementation could no longer export (`breaks_reference`:
//! resource references, managed files and strings outside the dependency
//! whitelist). Callers reject a change with a non-empty `breaks_reference`
//! — the user removes those parameters first — so the apply-time export
//! check never meets them.
use crate::parameter_schema::{self, ParameterSpec};
use project::{Parameter, TextureValueType};

/// What changing to a new implementation does to the declared parameters.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ImplementationChange {
    /// The requested name, normalized through the table's r4 aliases.
    pub implementation: String,
    /// Kept parameters the new implementation's table has no matching row
    /// for (every parameter when it has no table), in declaration order.
    pub kept_unknown: Vec<String>,
    /// Parameters the new implementation could not export, in declaration
    /// order; not repeated in `kept_unknown`.
    pub breaks_reference: Vec<String>,
}

/// Why an implementation change is refused outright.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rejection {
    /// The requested implementation is empty; removing an area light is
    /// `set_area_light(null)`'s job.
    Empty,
    /// The current item is the explicit "no area light", which
    /// entity), which has no implementation to change.
    Cancelling,
    /// The export schema does not support the requested implementation.
    Unsupported,
    /// The texture's value type has no table for the requested
    /// implementation (float and spectrum textures never cross).
    TextureValueType,
}

impl Rejection {
    pub fn as_str(self) -> &'static str {
        match self {
            Rejection::Empty => "empty_implementation",
            Rejection::Cancelling => "cancelling_entity",
            Rejection::Unsupported => "unsupported_implementation",
            Rejection::TextureValueType => "texture_value_type",
        }
    }
}

/// The implementations an editor offers for `directive`: the table's rows
/// when it has one, else every implementation the export schema supports
/// (never the area light's cancelling "").
pub fn candidates(directive: &str) -> Vec<&'static str> {
    match parameter_schema::schema(directive) {
        Some(table) => table.iter().map(|item| item.name).collect(),
        None => crate::schema::implementations(directive)
            .iter()
            .copied()
            .filter(|name| !name.is_empty())
            .collect(),
    }
}

/// Plan changing `current` to `requested` under `directive` (`Shape`,
/// `LightSource`, `AreaLightSource`, `Camera`, the configuration
/// directives, `MakeNamedMaterial`, `Texture`, `MakeNamedMedium`).
/// `Ok(None)` means no change: the names agree after alias normalization.
pub fn implementation_change(
    directive: &str,
    current: &str,
    texture_value_type: Option<TextureValueType>,
    texture_types: &std::collections::HashMap<project::ResourceId, TextureValueType>,
    parameters: &[Parameter],
    requested: &str,
) -> Result<Option<ImplementationChange>, Rejection> {
    if current.is_empty() {
        return Err(Rejection::Cancelling);
    }
    if requested.is_empty() {
        return Err(Rejection::Empty);
    }
    let table = parameter_schema::schema(directive);
    let resolve = |name: &str| -> (String, Option<&'static [ParameterSpec]>) {
        match table.and_then(|table| parameter_schema::implementation(table, name)) {
            Some(found) => (found.name.to_owned(), found.parameters(texture_value_type)),
            None => (name.to_owned(), None),
        }
    };
    let (implementation, rows) = resolve(requested);
    if !crate::schema::supports(directive, &implementation) {
        return Err(Rejection::Unsupported);
    }
    if texture_value_type.is_some() && rows.is_none() {
        return Err(Rejection::TextureValueType);
    }
    let (current, _) = resolve(current);
    if current == implementation {
        return Ok(None);
    }
    let mut kept_unknown = Vec::new();
    let mut breaks_reference = Vec::new();
    for p in parameters {
        if crate::schema::parameter(directive, &implementation, p).is_err() {
            breaks_reference.push(p.name.clone());
        } else if p.value_type == "texture"
            && rows.is_some_and(|rows| !texture_lineage_matches(p, rows, texture_types))
        {
            // The new implementation can no longer safely export this
            // binding: the same row name may accept a different texture kind.
            breaks_reference.push(p.name.clone());
        } else if !rows.is_some_and(|rows| compatible(p, rows)) {
            kept_unknown.push(p.name.clone());
        }
    }
    Ok(Some(ImplementationChange {
        implementation,
        kept_unknown,
        breaks_reference,
    }))
}

fn row<'a>(rows: &'a [ParameterSpec], name: &str) -> Option<&'a ParameterSpec> {
    rows.iter()
        .find(|spec| spec.name == name || spec.aliases.contains(&name))
}

/// Whether the new table has a row for `p` by name (or alias) and type.
fn compatible(p: &Parameter, rows: &[ParameterSpec]) -> bool {
    row(rows, &p.name).is_some_and(|new| {
        if p.value_type == "texture" {
            new.texture.is_some()
        } else {
            new.value_types.contains(&p.value_type.as_str())
        }
    })
}

fn texture_lineage_matches(
    p: &Parameter,
    rows: &[ParameterSpec],
    texture_types: &std::collections::HashMap<project::ResourceId, TextureValueType>,
) -> bool {
    let Some(new) = row(rows, &p.name) else {
        return true;
    };
    let bound_type = match &p.values {
        project::ParameterValues::Resources {
            kind: project::ResourceKind::Texture,
            ids,
        } if ids.len() == 1 => texture_types.get(&ids[0]),
        _ => None,
    };
    let Some(bound_type) = bound_type else {
        return true;
    };
    matches!(
        (new.texture, Some(bound_type)),
        (
            Some(crate::parameter_schema::TextureInput::Float),
            Some(TextureValueType::Float)
        ) | (
            Some(
                crate::parameter_schema::TextureInput::SpectrumAlbedo
                    | crate::parameter_schema::TextureInput::SpectrumUnbounded
                    | crate::parameter_schema::TextureInput::SpectrumIlluminant,
            ),
            Some(TextureValueType::Spectrum),
        )
    )
}
