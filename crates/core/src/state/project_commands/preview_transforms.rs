//! and: per-entity transform deltas without
//! a preview job.
//!
//! `project.preview_transforms` keeps one base: the snapshot of the last
//! successful `project.preview` job of the active activation. When the
//! current document differs from the base only in scene-node labels,
//! editions and `StateOperation::Transform` values, the answer carries the
//! changed nodes' `new × base⁻¹` for both transform samples — the same
//! row-major convention as the draw package's matrices, so `delta · base`
//! is the new matrix and nothing but the displayed matrices has to change.
//! Shapes and ObjectInstances key the draw's `entity_id`; a static Light
//! keys the light record's `entity_id` and retargets its `world_from_light`
//! An AreaLight entity owns no draw — the shapes it lights carry
//! the move — and a Camera move sends nothing to the display, so both
//! answer fast with no delta of their own. Everything else — parameters,
//! resources, definition nodes, an animated node, a singular base
//! matrix, a static⇄animated flip — falls back: `fast:false` with a
//! snake_case reason, and the caller rebuilds the full preview.
use super::*;

/// The preview base of the active project activation: the snapshot of the
/// last successful app preview job. `Projects::advance_activation` clears
/// the slot on every project or scene switch, so a base never crosses an
/// activation boundary; the base is never consulted beyond the activation
/// it was recorded in.
pub(crate) struct PreviewBase {
    pub(crate) activation: u64,
    pub(crate) snapshot: ProjectSnapshot,
}

/// The wire's `{ "fast": false, "reason": … }`; the caller rebuilds the
/// full preview.
fn fallback(reason: &str) -> Value {
    json!({"fast": false, "reason": reason})
}

impl CoreState {
    /// `project.preview_transforms`: synchronous and read-only. The handler
    /// already checked the caller's `project` reference (the current
    /// revision); the answer itself never fails the request.
    pub(crate) fn project_preview_transforms(
        &self,
        p: &dto::ProjectPreviewTransformsParams,
    ) -> Value {
        let started = std::time::Instant::now();
        let answer = self.preview_transforms_answer(p);
        crate::preview_timing::record_method("preview_transforms", started);
        answer
    }

    fn preview_transforms_answer(&self, p: &dto::ProjectPreviewTransformsParams) -> Value {
        let Some(active) = self.projects.active.as_ref() else {
            return fallback("no_base");
        };
        let Some(scene) = active.session.active_scene() else {
            return fallback("no_active_scene");
        };
        let Some(base) = self.projects.preview_base.as_ref() else {
            return fallback("no_base");
        };
        if base.activation != self.projects.activation {
            return fallback("no_base");
        }
        if base.snapshot.scene() != Some(scene) {
            return fallback("scene_changed");
        }
        if base.snapshot.revision() != &p.base_revision {
            return fallback("base_revision_mismatch");
        }
        let current = active.session.document();
        if !transform_only(base.snapshot.document(), current) {
            return fallback("non_transform_change");
        }
        // Evaluate the ORIGINAL base scene and the current one: the probe
        // above is only the validity check, and the deltas come from the
        // real matrices the two documents evaluate to.
        let Ok(base_view) = base.snapshot.scene_view() else {
            return fallback("evaluation_failed");
        };
        let Ok(current_view) = current.scene_view(Some(scene)) else {
            return fallback("evaluation_failed");
        };
        let (Ok(base_evaluated), Ok(new_evaluated)) = (
            project::evaluate(&base_view),
            project::evaluate(&current_view),
        ) else {
            return fallback("evaluation_failed");
        };
        let mut deltas = serde_json::Map::new();
        let mut changed: std::collections::BTreeSet<project::NodeId> =
            base_evaluated.nodes.keys().copied().collect();
        changed.extend(new_evaluated.nodes.keys().copied());
        for id in changed {
            // After the probe only transform state nodes can exist on one
            // side; they name no draws.
            let (Some(before), Some(after)) =
                (base_evaluated.nodes.get(&id), new_evaluated.nodes.get(&id))
            else {
                continue;
            };
            if before.start == after.start && before.end == after.end {
                continue;
            }
            // A node inside an object definition: its draws surface through
            // instance-use entity ids the fast path does not rewrite.
            if current.definition_nodes.contains_key(&id) {
                return fallback("definition_node");
            }
            let Some(node) = current_view.node(id) else {
                continue;
            };
            match &node.data {
                project::NodeKind::Entity {
                    entity: project::EntityKind::Light,
                    ..
                } => {
                    // a static light answers the same delta a
                    // Shape draw does, keyed by the light node id the
                    // preview's light record pins, so `delta ·
                    // world_from_light` is the new `world_from_light`.
                    // An animated light keeps rebuilding: its record
                    // carries one sample, which endpoint pairs cannot
                    // retarget.
                    if let Err(reason) = delta_pair(before, after, id, &mut deltas) {
                        return fallback(reason);
                    }
                }
                project::NodeKind::Entity {
                    entity: project::EntityKind::Shape,
                    ..
                } => {
                    // Area-light shapes included: `lit::rebind`
                    // rebuilds the emitter from the draw, so the delta
                    // applies as it does for any other shape.
                    if let Err(reason) = delta_pair(before, after, id, &mut deltas) {
                        return fallback(reason);
                    }
                }
                project::NodeKind::Instance { .. } => {
                    if let Err(reason) = delta_pair(before, after, id, &mut deltas) {
                        return fallback(reason);
                    }
                }
                // An AreaLight entity owns no draw (the shapes it lights
                // carry the move as Shapes), and a camera move sends
                // nothing to the display — the viewer camera follows its
                // owner, not the manifest. Scopes, state
                // nodes, configurations and options likewise carry the
                // move to their descendants; the descendants name the
                // draws.
                _ => {}
            }
        }
        json!({
            "fast": true,
            "base_revision": base.snapshot.revision(),
            "revision": p.project.revision,
            "deltas": Value::Object(deltas),
        })
    }
}

/// The `new × base⁻¹` pair for one changed Shape, ObjectInstance, or static light.
/// The preview keys a draw by its `entity_id` and a light by its own record
/// ID. Each pair contains the start and end matrices in row-major order. Only static nodes (start == end on both
/// sides) answer: an animated draw is sampled at shutter times between
/// the transform times, and a light record carries one sample — either
/// way endpoint deltas cannot retarget exactly. A non-invertible base
/// matrix is a fallback too.
fn delta_pair(
    before: &project::GraphicsState,
    after: &project::GraphicsState,
    id: project::NodeId,
    deltas: &mut serde_json::Map<String, Value>,
) -> Result<(), &'static str> {
    if before.start != before.end || after.start != after.end {
        return Err("animated");
    }
    let (Some(base_start), Some(base_end)) = (
        project::math::inverse(before.start),
        project::math::inverse(before.end),
    ) else {
        return Err("singular_base_matrix");
    };
    deltas.insert(
        id.to_string(),
        json!([
            after.start.compose(base_start).0,
            after.end.compose(base_end).0,
        ]),
    );
    Ok(())
}

/// The B1 validity probe: with every scene's `State{Transform(_)}` nodes
/// removed (from the node table and from scope children) and labels and
/// editions taken from the current document, the base document must equal
/// the current one in full. Removing rather than matching transform nodes
/// lets the desktop's `set_local_trs`, which rebuilds a group's transform
/// run under new node ids, take the fast path. Anything else that differs
/// (parameters, resources, definition nodes, other nodes, TransformTimes,
/// ActiveTransform, …) is a full rebuild; a camera pose counts as a transform.
fn transform_only(base: &project::ProjectDocument, current: &project::ProjectDocument) -> bool {
    let mut probe = base.clone();
    let mut current = current.clone();
    for document in [&mut probe, &mut current] {
        for scene in document.scenes.values_mut() {
            let scene = std::sync::Arc::make_mut(scene);
            scene.nodes.retain(|_, node| !is_transform(node));
        }
    }
    for document in [&mut probe, &mut current] {
        for scene in document.scenes.values_mut() {
            let scene = std::sync::Arc::make_mut(scene);
            let kept: std::collections::BTreeSet<_> = scene.nodes.keys().copied().collect();
            scene.options.retain(|id| kept.contains(id));
            for node in scene.nodes.values_mut() {
                if let project::NodeKind::Scope { children, .. } = &mut node.data {
                    children.retain(|id| kept.contains(id));
                }
            }
        }
    }
    for (scene_id, scene) in probe.scenes.iter_mut() {
        let Some(current_scene) = current.scenes.get(scene_id) else {
            continue;
        };
        let scene = std::sync::Arc::make_mut(scene);
        for (node_id, node) in scene.nodes.iter_mut() {
            if let Some(current_node) = current_scene.nodes.get(node_id) {
                node.label = current_node.label.clone();
                node.edition = current_node.edition.clone();
                // a camera's pose is its transform (the camera gizmo's
                // `set_local_matrix` rewrites it); dependents of the
                // "camera" coordinate system surface as changed nodes below.
                if let (
                    project::NodeKind::Entity {
                        entity: project::EntityKind::Camera,
                        pose,
                        ..
                    },
                    project::NodeKind::Entity {
                        entity: project::EntityKind::Camera,
                        pose: current_pose,
                        ..
                    },
                ) = (&mut node.data, &current_node.data)
                {
                    *pose = *current_pose;
                }
            }
        }
    }
    probe == current
}

fn is_transform(node: &project::Node) -> bool {
    matches!(
        node.data,
        project::NodeKind::State {
            operation: project::StateOperation::Transform(_),
        }
    )
}
