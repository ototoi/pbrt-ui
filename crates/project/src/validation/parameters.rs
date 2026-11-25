use super::*;

fn transform(operation: &TransformOperation) -> Result<()> {
    let finite = |v: &[f64]| v.iter().all(|n| n.is_finite());
    let valid = match operation {
        TransformOperation::Identity => true,
        TransformOperation::Translate(v) | TransformOperation::Scale(v) => finite(v),
        TransformOperation::LookAt(v) => finite(v) && crate::math::look_at(v).is_some(),
        TransformOperation::Rotate { degrees, axis } => {
            degrees.is_finite() && finite(axis) && axis.iter().any(|v| *v != 0.)
        }
        TransformOperation::Replace(m) | TransformOperation::Concat(m) => {
            finite(m) && m[12..] == [0., 0., 0., 1.]
        }
    };
    if !valid {
        return Err(invalid(
            "transform requires finite affine values and a nonzero rotation axis",
        ));
    }
    Ok(())
}

pub(super) fn resource(doc: &ProjectDocument, id: ResourceId, kind: ResourceKind) -> Result<()> {
    if doc.resources.get(&id).is_none_or(|r| r.kind != kind) {
        return Err(invalid("missing resource or reference kind mismatch"));
    }
    Ok(())
}
pub(super) fn state(doc: &ProjectDocument, operation: &StateOperation) -> Result<()> {
    match operation {
        StateOperation::Transform(t) => transform(t)?,
        StateOperation::TransformTimes(times)
            if !times.iter().all(|t| t.is_finite()) || times[0] > times[1] =>
        {
            return Err(invalid("invalid transform time interval"));
        }
        StateOperation::Material(MaterialBinding::Resource(id)) => {
            resource(doc, *id, ResourceKind::Material)?
        }
        StateOperation::Medium { inside, outside } => {
            for id in inside.iter().chain(outside) {
                resource(doc, *id, ResourceKind::Medium)?;
            }
        }
        StateOperation::ColorSpace(name)
        | StateOperation::DefineCoordinates(name)
        | StateOperation::UseCoordinates(name)
            if name.is_empty() =>
        {
            return Err(invalid("state name is empty"));
        }
        _ => {}
    }
    Ok(())
}

pub(super) fn parameters(doc: &ProjectDocument, params: &[Parameter]) -> Result<()> {
    for p in params {
        if p.name.is_empty() || p.value_type.is_empty() {
            return Err(invalid("parameter name/type is empty"));
        }
        if matches!(&p.color_space,ColorSpaceBinding::Explicit(name) if name.is_empty()) {
            return Err(invalid("empty explicit color space"));
        }
        match &p.values {
            ParameterValues::Floats(values)
                if values
                    .iter()
                    .any(|v| matches!(v,FloatValue::Finite(n) if !n.is_finite())) =>
            {
                return Err(invalid(
                    "nonfinite float requires an explicit tagged representation",
                ));
            }
            ParameterValues::Assets(ids) => {
                for id in ids {
                    if !doc.assets.contains_key(id) {
                        return Err(invalid("missing asset reference"));
                    }
                }
            }
            ParameterValues::Resources { kind, ids } => {
                for id in ids {
                    resource(doc, *id, *kind)?;
                }
            }
            ParameterValues::Array {
                asset,
                encoding,
                count,
            } => {
                let Some(asset) = doc.assets.get(asset) else {
                    return Err(invalid("missing parameter array asset"));
                };
                let stride = match encoding {
                    ArrayEncoding::F64Le | ArrayEncoding::I64Le => 8,
                    ArrayEncoding::BoolU8 => 1,
                };
                if count.checked_mul(stride) != Some(asset.size) {
                    return Err(invalid("parameter array byte count mismatch"));
                }
            }
            _ => {}
        }
        let compatible = match p.value_type.as_str() {
            "integer" => matches!(
                &p.values,
                ParameterValues::Integers(_)
                    | ParameterValues::Array {
                        encoding: ArrayEncoding::I64Le,
                        ..
                    }
            ),
            "bool" => matches!(
                &p.values,
                ParameterValues::Booleans(_)
                    | ParameterValues::Array {
                        encoding: ArrayEncoding::BoolU8,
                        ..
                    }
            ),
            "float" | "rgb" | "point2" | "point3" | "vector2" | "vector3" | "normal3"
 // r4's shorter spellings, accepted by import.
            | "vector" | "normal"
            | "blackbody" => matches!(
                &p.values,
                ParameterValues::Floats(_)
                    | ParameterValues::Array {
                        encoding: ArrayEncoding::F64Le,
                        ..
                    }
            ),
            "texture" => matches!(
                &p.values,
                ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ..
                }
            ),
            "string" => matches!(
                &p.values,
                ParameterValues::Strings(_)
                    | ParameterValues::Assets(_)
                    | ParameterValues::Resources { .. }
            ),
            "spectrum" => matches!(
                &p.values,
                ParameterValues::Floats(_)
                    | ParameterValues::BuiltinSpectra(_)
                    | ParameterValues::Assets(_)
                    | ParameterValues::Array {
                        encoding: ArrayEncoding::F64Le,
                        ..
                    }
                    | ParameterValues::Resources {
                        kind: ResourceKind::SpectrumData,
                        ..
                    }
            ),
            _ => true,
        };
        if !compatible {
            return Err(invalid(
                "parameter declaration type does not match its payload",
            ));
        }
    }
    Ok(())
}
