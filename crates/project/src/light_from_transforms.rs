//! Fold a `LightSource` point/spot/distant light's `from` into
//! the light's own local transform, at import.
//!
//! pbrt-v4 composes the parameters into the transform current at the
//! `LightSource` directive (src/pbrt/lights.cpp): a point light renders
//! from `renderFromLight · Translate(from)`, a spot the same translation
//! with the LookAt-like frame toward `to`, and a distant light only the
//! direction `to - from`. A `from` off the origin therefore renders,
//! but the light's drawn wire-frame sits at `from` while the transform
//! gizmo, the Inspector and the local-transform editor all live at the
//! node's own origin.
//!
//! This pass — run by the import right after
//! [`crate::transform_parents::resolve_transform_parents`] — puts the
//! translation where the model's editor already reads it: the light's
//! dedicated group, the same run an interactive `set_local_transform`
//! rewrites, gains a trailing `Translate(from)`, so the node's local
//! transform becomes `local · Translate(from)`; the parameters
//! re-centre on the origin (`from` back to the default — the model
//! stores only declared rows, so the row is dropped — and `to` moved to
//! `to - from` for the spot and distant lights that read it). The
//! evaluated rendering is identical: pbrt builds the same frame either
//! way.
//!
//! Lights whose evaluated transform is animated are left unchanged —
//! the same refusal `set_local_transform` gives them, so a folded light
//! is always gizmo-editable — and so are lights inside object
//! definitions, which are not scene nodes. Already-imported projects are
//! not migrated: the pass runs at parse time.

use crate::session::{ensure_dedicated_group, insert_child_node};
use crate::{
    ActiveSamples, EntityKind, EvaluatedScene, FloatValue, GraphicsState, Node, NodeId, NodeKind,
    Parameter, ProjectDocument, Result, SceneId, StateOperation, TransformOperation, evaluate,
    invalid,
};
use std::collections::BTreeMap;

/// pbrt-v4's `to` default (lights.cpp): spot and distant read `to`,
/// default (0,0,1).
const TO: [f64; 3] = [0., 0., 1.];

/// One light to fold: the node, its declared `from`, and the `to` the
/// parameters must carry (already moved); `None` for the point light,
/// which reads no `to`.
struct Fold {
    light: NodeId,
    from: [f64; 3],
    to: Option<[f64; 3]>,
}

/// Rewrite every foldable light in the scene. The document must
/// be a freshly resolved import; the pass only inserts nodes, so the
/// import's node budget is re-checked after it by the caller.
pub fn fold_light_from_transforms(document: &mut ProjectDocument, scene: SceneId) -> Result<()> {
    let Some(world) = document.scene(scene).map(|scene| scene.world) else {
        return Err(invalid("scene is missing"));
    };
    // One evaluation answers every animated question: the light's own
    // samples, start against end, exactly what `set_local_transform`
    // refuses — nothing the gizmo could not edit gets folded.
    let view = document.scene_view(scene)?;
    let evaluated = evaluate(&view)?;
    let folds = folds(document, scene, &evaluated);
    let mut changed = BTreeMap::new();
    let mut created = Vec::new();
    for fold in folds {
        // The model's editor reads the group's `local` transform through
        // `ensure_dedicated_group`, which wraps `set_local_transform`.
        // Appending the state before the light ends the run in
        // `Translate(from)`, so the light's world transform becomes
        // `world · Translate(from)`.
        let group = ensure_dedicated_group(
            document,
            scene,
            world,
            fold.light,
            &mut changed,
            &mut created,
        )?;
        let at = match &document
            .scene(scene)
            .and_then(|scene| scene.nodes.get(&group))
            .expect("the dedicated group exists")
            .data
        {
            NodeKind::Scope { children, .. } => children
                .iter()
                .position(|child| *child == fold.light)
                .unwrap_or(children.len()),
            _ => return Err(invalid("the dedicated group is not a scope")),
        };
        insert_child_node(
            document,
            scene,
            group,
            at,
            Node::new(
                "Translate",
                NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Translate(fold.from)),
                },
            ),
            &mut created,
            &mut changed,
        )?;
        recenter_parameters(document, scene, &fold)?;
    }
    Ok(())
}

/// Whether the light's own samples diverge — the same refusal
/// `set_local_transform` gives an animated local (either sample changed,
/// or a selector still narrows what a new state would write).
fn animated(state: &GraphicsState) -> bool {
    state.start != state.end || state.active != ActiveSamples::All
}

/// The foldable lights in node order: a scene light — never one inside an
/// object definition — implemented as pbrt's point, spot or distant
/// light, whose declared `from` is off the origin and whose evaluated
/// transform is static.
fn folds(document: &ProjectDocument, scene: SceneId, evaluated: &EvaluatedScene) -> Vec<Fold> {
    document
        .scene(scene)
        .into_iter()
        .flat_map(|scene| scene.nodes.values())
        .filter_map(|node| {
            let NodeKind::Entity {
                entity: EntityKind::Light,
                implementation,
                parameters,
                ..
            } = &node.data
            else {
                return None;
            };
            if !matches!(implementation.as_str(), "point" | "spot" | "distant") {
                return None;
            }
            // Unverified animation or an animated transform stays as it
            // is.
            let static_samples = evaluated
                .nodes
                .get(&node.id)
                .is_some_and(|state| !animated(state));
            if !static_samples {
                return None;
            }
            let from = declared_point3(parameters, "from")?;
            if from == [0., 0., 0.] {
                return None;
            }
            // The lights that read `to` carry `to - from`; a missing row
            // starts from the same default the renderer uses.
            let to = match implementation.as_str() {
                "spot" | "distant" => {
                    let to = declared_point3(parameters, "to").unwrap_or(TO);
                    Some([0, 1, 2].map(|axis| difference(to[axis], from[axis])))
                }
                _ => None,
            };
            Some(Fold {
                light: node.id,
                from,
                to,
            })
        })
        .collect()
}

/// `to - from`, rounded to 12 significant digits of the operands: the
/// source's decimals are not exact binary values, so the raw difference
/// would read `-0.014730000000000132` where the file says 16.786858 and
/// 16.801588.
fn difference(to: f64, from: f64) -> f64 {
    let difference = to - from;
    let scale = to.abs().max(from.abs());
    if scale == 0. || !difference.is_finite() {
        return difference;
    }
    let digits = 11 - scale.log10().floor() as i32;
    // Dividing by an exact power of ten (<= 1e22) gives the nearest
    // double; outside that range the raw difference is kept.
    if !(1..=22).contains(&digits) {
        return difference;
    }
    let factor = 10f64.powi(digits);
    (difference * factor).round() / factor
}

/// The first three floats of a declared row, `None` for a row that is
/// absent, not floats, or not finite — a light with such a `from` or
/// `to` keeps what it declared.
fn declared_point3(parameters: &[Parameter], name: &str) -> Option<[f64; 3]> {
    let row = parameters
        .iter()
        .find(|parameter| parameter.name.eq_ignore_ascii_case(name))?;
    let crate::ParameterValues::Floats(values) = &row.values else {
        return None;
    };
    let numbers = values
        .iter()
        .map(|value| match value {
            FloatValue::Finite(value) => Some(*value),
            _ => None,
        })
        .collect::<Option<Vec<f64>>>()?;
    match numbers[..] {
        [x, y, z, ..] => Some([x, y, z]),
        _ => None,
    }
}

/// Re-centre the folded light's parameters on the origin: `from` is its
/// default again, so the declared row folds away, and the lights that
/// read `to` carry `to - from` — a row the source never declared becomes
/// declared, under the renderer's own canonical spelling.
fn recenter_parameters(document: &mut ProjectDocument, scene: SceneId, fold: &Fold) -> Result<()> {
    let scene_document = document
        .scenes
        .get_mut(&scene)
        .ok_or_else(|| invalid("scene is missing"))?;
    let node = std::sync::Arc::make_mut(scene_document)
        .nodes
        .get_mut(&fold.light)
        .ok_or_else(|| invalid("missing folded light"))?;
    let NodeKind::Entity { parameters, .. } = &mut node.data else {
        return Err(invalid("the folded node is not a light"));
    };
    parameters.retain(|parameter| !parameter.name.eq_ignore_ascii_case("from"));
    if let Some([x, y, z]) = fold.to {
        let values = crate::ParameterValues::Floats(vec![
            FloatValue::Finite(x),
            FloatValue::Finite(y),
            FloatValue::Finite(z),
        ]);
        match parameters
            .iter_mut()
            .find(|parameter| parameter.name.eq_ignore_ascii_case("to"))
        {
            // The declared spelling stays; only the floats move.
            Some(row) => row.values = values,
            None => parameters.push(Parameter {
                value_type: "point3".into(),
                name: "to".into(),
                color_space: Default::default(),
                values,
            }),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn difference_drops_the_binary_noise_of_decimal_inputs() {
        assert_eq!(super::difference(16.786858, 16.801588), -0.01473);
        assert_eq!(super::difference(1., -3.), 4.);
        assert_eq!(super::difference(0., 0.), 0.);
        // A tiny scale is never rounded away.
        assert_eq!(super::difference(3e-30, 1e-30), 3e-30 - 1e-30);
    }
}
