//! Scene graph view and Inspector components (
//! appendix D). Derived from the ordered model on each query; nothing here is
//! stored.
use super::*;
use project::transform_trs;
use project::{
    ConfigurationKind, EntityKind, EvaluatedScene, GraphicsState, MaterialBinding, NodeId,
    NodeKind, ParameterValues, SceneView, ScopeKind, StateOperation,
};
use std::collections::BTreeMap;

/// A matrix's TRS for the wire, or why it has none.
fn trs_json(matrix: &project::math::Mat4) -> (Value, Value) {
    match transform_trs::decompose(matrix) {
        Ok(trs) => (
            json!({
                "translation":trs.translation,
                "rotation_euler_degrees":trs.rotation_euler_degrees,
                "rotation_order":"zxy",
                "scale":trs.scale,
            }),
            Value::Null,
        ),
        Err(reason) => (Value::Null, json!(reason)),
    }
}

/// Endpoint values for an animated local transform. The matrix remains null
/// only when that endpoint cannot be recovered; a valid matrix can still be
/// shown even when its TRS decomposition fails.
fn local_sample(
    matrix: Result<project::math::Mat4, transform_trs::LocalError>,
    parent: &Value,
) -> Value {
    match matrix {
        Ok(matrix) => {
            let (trs, trs_reason) = trs_json(&matrix);
            json!({
                "matrix":matrix.0,
                "matrix_reason":null,
                "trs":trs,
                "trs_reason":trs_reason,
                "parent":parent,
            })
        }
        Err(reason) => json!({
            "matrix":null,
            "matrix_reason":reason,
            "trs":null,
            "trs_reason":null,
            "parent":parent,
        }),
    }
}

/// Object kind shown in the scene graph, or `None` for directive-only nodes
/// (state, configuration, option, area light).
pub(super) fn object_kind(kind: &NodeKind) -> Option<&'static str> {
    match kind {
        NodeKind::Scope { scope, .. } => Some(match scope {
            // World is a PBRT state boundary, not a visible object. Its
            // object children are projected directly under the Scene root.
            ScopeKind::World => return None,
            ScopeKind::Attribute | ScopeKind::Transform => "group",
            ScopeKind::ObjectDefinition => "definition",
        }),
        NodeKind::Entity { entity, .. } => match entity {
            EntityKind::Shape => Some("shape"),
            EntityKind::Light => Some("light"),
            EntityKind::Camera => Some("camera"),
            EntityKind::AreaLight => None,
        },
        NodeKind::Instance { .. } => Some("instance"),
        NodeKind::State { .. } | NodeKind::Configuration { .. } | NodeKind::Option { .. } => None,
    }
}

// Physical scope scans must stop at owned wrappers too. Projection handles
// their visibility independently in projected_children.
fn is_object(view: &SceneView, id: NodeId) -> bool {
    view.node(id)
        .is_some_and(|node| object_kind(&node.data).is_some())
}

fn is_owned_scope(view: &SceneView, id: NodeId) -> bool {
    matches!(
        view.node(id).map(|node| &node.data),
        Some(NodeKind::Scope {
            owned_object: Some(_),
            ..
        })
    )
}

fn owned_object(view: &SceneView, id: NodeId) -> Option<NodeId> {
    match view.node(id).map(|node| &node.data) {
        Some(NodeKind::Scope {
            scope: ScopeKind::Attribute,
            owned_object,
            ..
        }) => *owned_object,
        _ => None,
    }
}

/// Object kind for rows exposed by the scene graph. Internally owned scopes
/// remain real scopes but are not independent visible hierarchy items.
pub(super) fn visible_object_kind(view: &SceneView, id: NodeId) -> Option<&'static str> {
    (!is_owned_scope(view, id))
        .then(|| view.node(id).and_then(|node| object_kind(&node.data)))
        .flatten()
}

/// The visible owner row associated with an object or its internal component.
pub(super) fn visible_owner(view: &SceneView, id: NodeId) -> Option<NodeId> {
    if visible_object_kind(view, id).is_some() {
        return Some(id);
    }
    let parent = parents(view).get(&id).copied()?;
    owned_object(view, parent)
}

/// The camera a scope is attached to: None for every other node.
fn parent_camera_of(view: &SceneView, id: NodeId) -> Option<NodeId> {
    match view.node(id).map(|node| &node.data) {
        Some(NodeKind::Scope { parent_camera, .. }) => *parent_camera,
        _ => None,
    }
}

fn projected_children(view: &SceneView, id: NodeId) -> Vec<NodeId> {
    let Some(NodeKind::Scope { children, .. }) = view.node(id).map(|node| &node.data) else {
        return Vec::new();
    };
    let mut result = Vec::new();
    for child in children {
        // a camera-attached group lists under its Camera row, not here.
        if parent_camera_of(view, *child).is_some() {
            continue;
        }
        if let Some(owner) = owned_object(view, *child) {
            result.push(owner);
        } else if is_object(view, *child) {
            result.push(*child);
        }
    }
    result
}

/// Children of the Scene root: option Cameras, then World content. The World
/// scope remains in the model but is transparent in the scene graph.
pub(super) fn scene_children(view: &SceneView) -> Vec<NodeId> {
    view.scene
        .options
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                view.node(*id).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            )
        })
        .chain(object_children(view, view.scene.world))
        .collect()
}

/// Object children of a scope; other nodes have none. A Camera row owns the
/// World-level groups attached to it.
pub(super) fn object_children(view: &SceneView, id: NodeId) -> Vec<NodeId> {
    if matches!(
        view.node(id).map(|node| &node.data),
        Some(NodeKind::Entity {
            entity: EntityKind::Camera,
            ..
        })
    ) {
        let mut result = Vec::new();
        if let Some(NodeKind::Scope { children, .. }) =
            view.node(view.scene.world).map(|node| &node.data)
        {
            for child in children {
                if parent_camera_of(view, *child) == Some(id) {
                    // An owned wrapper shows its object as the row.
                    result.push(owned_object(view, *child).unwrap_or(*child));
                }
            }
        }
        return result;
    }
    projected_children(view, id)
}

/// Ancestors nearest first in the visible scene graph, ending with SceneId.
/// The physical World scope is omitted; the underlying node IDs are retained.
pub(super) fn path(view: &SceneView, id: NodeId) -> Vec<uuid::Uuid> {
    let parents = visible_parents(view);
    let mut result = Vec::new();
    let mut current = id;
    while let Some(parent) = parents.get(&current).copied() {
        if parent != view.scene.world {
            result.push(parent.0);
        }
        current = parent;
    }
    if view.scene.nodes.contains_key(&id) {
        result.push(view.scene.id.0);
    }
    result
}

/// Parent links in the user-visible hierarchy. Internal owned scopes are
/// flattened while their owned object IDs stay stable for selection/editing.
pub(super) fn visible_parents(view: &SceneView) -> BTreeMap<NodeId, NodeId> {
    let mut parents = BTreeMap::new();
    let mut stack = scene_children(view);
    while let Some(parent) = stack.pop() {
        for child in object_children(view, parent) {
            parents.insert(child, parent);
            stack.push(child);
        }
    }
    parents
}

fn active_film(view: &SceneView) -> Option<(NodeId, String)> {
    view.scene.options.iter().rev().find_map(|id| {
        let node = view.node(*id)?;
        let NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation,
            ..
        } = &node.data
        else {
            return None;
        };
        Some((*id, implementation.clone()))
    })
}

/// Scope parent of every scene and definition node.
pub(super) fn parents(view: &SceneView) -> BTreeMap<NodeId, NodeId> {
    let mut parents = BTreeMap::new();
    for node in view
        .scene
        .nodes
        .values()
        .chain(view.document.definition_nodes.values())
    {
        if let NodeKind::Scope { children, .. } = &node.data {
            for child in children {
                parents.insert(*child, node.id);
            }
        }
    }
    parents
}

fn evaluated(view: &SceneView) -> Result<EvaluatedScene, ErrorPayload> {
    project::evaluate(view).map_err(|error| invalid(error.to_string()))
}

pub(super) struct Components<'a> {
    view: SceneView<'a>,
    evaluated: EvaluatedScene,
    parents: BTreeMap<NodeId, NodeId>,
    /// Where `Assets/` lives on disk: plymesh statistics read the PLY
    /// header. None leaves them null.
    asset_root: Option<&'a str>,
}

impl<'a> Components<'a> {
    pub(super) fn new(view: SceneView<'a>) -> Result<Self, ErrorPayload> {
        Ok(Self {
            evaluated: evaluated(&view)?,
            parents: parents(&view),
            view,
            asset_root: None,
        })
    }

    pub(super) fn with_asset_root(mut self, asset_root: Option<&'a str>) -> Self {
        self.asset_root = asset_root;
        self
    }

    fn camera_read_only_reason(&self, id: NodeId) -> Option<&'static str> {
        if !self.view.scene.options.contains(&id) {
            return Some("Camera is not in Options");
        }
        let Some(node) = self.view.node(id) else {
            return Some("Camera is not in Options");
        };
        let NodeKind::Entity { pose, .. } = &node.data else {
            return Some("Camera is not in Options");
        };
        // a posed camera edits its pose value directly; an animated
        // pose keeps refusing the static edit API. A camera without a pose
        // comes from an older import and is refused like a reset run.
        match pose {
            Some(pose) if pose.start != pose.end => Some("The camera pose is animated"),
            Some(_) => None,
            None => Some("The camera has no pose (an older import; re-import to edit it)"),
        }
    }

    /// Component kinds in Inspector order.
    pub(super) fn summary(&self, id: NodeId) -> Vec<Value> {
        self.detail(id)
            .into_iter()
            .map(|component| component["kind"].clone())
            .collect()
    }

    pub(super) fn detail(&self, id: NodeId) -> Vec<Value> {
        let Some(node) = self.view.node(id) else {
            return Vec::new();
        };
        let Some(kind) = object_kind(&node.data) else {
            return Vec::new();
        };
        let mut transform = self.transform(id, kind == "group");
        if kind == "camera" {
            let reason = self.camera_read_only_reason(id);
            transform["editable"] = json!(reason.is_none());
            if let Some(reason) = reason {
                transform["read_only_reason"] = json!(reason);
            }
        }
        let mut components = vec![transform];
        let state = self.evaluated.nodes.get(&id);
        let target = json!({"kind":"node","id":id});
        match &node.data {
            NodeKind::Entity {
                entity: EntityKind::Shape,
                implementation,
                parameters,
                ..
            } => {
                let mesh_asset = parameters
                    .iter()
                    .find_map(|parameter| match &parameter.values {
                        ParameterValues::Assets(ids) => ids.first().copied(),
                        _ => None,
                    });
                components.push(json!({
                    "kind":"shape","implementation":implementation,"parameter_target":target,
                    "mesh_asset":mesh_asset,
                    "statistics":match (implementation.as_str(), mesh_asset) {
 // a PLY asset's header holds the counts.
                        ("plymesh", Some(asset)) => self
                            .view
                            .document
                            .assets
                            .get(&asset)
                            .zip(self.asset_root)
                            .and_then(|(asset, root)| ply_info_of(asset, root))
                            .unwrap_or(Value::Null),
                        _ => statistics(implementation, parameters),
                    },
                }));
                // The shape's alpha mask: its
                // own component while alpha or shadowalpha is declared, so it
                // can be added and removed like the others.
                let mask = parameters
                    .iter()
                    .filter(|p| matches!(p.name.as_str(), "alpha" | "shadowalpha"))
                    .map(|p| json!({"name":p.name,"value_type":p.value_type}))
                    .collect::<Vec<_>>();
                if !mask.is_empty() {
                    components.push(json!({
                        "kind":"alpha_mask","parameter_target":target,"declared":mask,
                    }));
                }
                if let Some(light) = state
                    .and_then(|state| state.area_light)
                    .and_then(|light| self.view.node(light).map(|node| (light, node)))
                    && let NodeKind::Entity { implementation, .. } = &light.1.data
                {
                    components.push(json!({
                        "kind":"area_light","node_id":light.0,"implementation":implementation,
                        "parameter_target":{"kind":"node","id":light.0},
 // the entity sits in the object's dedicated group.
                        "local":self.local(id, Some(light.0)),
                    }));
                }
                components.push(json!({
                    "kind":"reverse_orientation",
                    "enabled":state.is_some_and(|state| state.reverse_orientation),
                    "local":self.local_toggle(id).is_some(),
                    "node_id":self.local_toggle(id),
                }));
                components.push(self.material(id));
                components.push(self.medium(id));
            }
            NodeKind::Entity {
                entity: EntityKind::Light,
                implementation,
                ..
            } => {
                components.push(
                    json!({"kind":"light","implementation":implementation,"parameter_target":target}),
                );
                components.push(self.medium(id));
            }
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation,
                ..
            } => {
                components.push(
                    json!({"kind":"camera","implementation":implementation,"parameter_target":target}),
                );
                components.push(self.camera_medium(id));
            }
            NodeKind::Instance { definition } => {
                components.push(json!({"kind":"object_instance","definition":definition}));
            }
            // Groups keep their Material / MediumInterface bindings in the
            // document and evaluator, but the Inspector exposes those rows
            // on Shapes only. Area Light remains a visible Group component.
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                ..
            } => {
                let state = self.group_state(&id);
                let light = state.and_then(|state| state.area_light);
                if let Some(light) =
                    light.and_then(|light| self.view.node(light).map(|node| (light, node)))
                {
                    let NodeKind::Entity { implementation, .. } = &light.1.data else {
                        unreachable!()
                    };
                    components.push(json!({
                        "kind":"area_light","node_id":light.0,"implementation":implementation,
                        "parameter_target":{"kind":"node","id":light.0},
                        "local":self.head_contains(&id, Some(light.0)),
                    }));
                }
            }
            _ => {}
        }
        components
    }

    /// The state a group's content sees: its own head components applied on
    /// top of what it inherited. Scopes record their entry state, so the
    /// first child after the head carries the answer.
    fn group_state(&self, group: &NodeId) -> Option<&GraphicsState> {
        let children = self.scope_children(*group)?;
        let skip = children
            .iter()
            .take_while(|child| self.is_component(child))
            .count();
        if skip == 0 {
            // No head components: the entry state is what the group sees.
            return self.evaluated.nodes.get(group);
        }
        if let Some(first) = children.get(skip) {
            return self.evaluated.nodes.get(first);
        }
        // A group of only head components: its last one carries the state
        // they leave behind.
        self.evaluated.nodes.get(&children[skip - 1])
    }

    /// Whether a node belongs to the group's leading component run.
    fn is_component(&self, id: &NodeId) -> bool {
        self.view.node(*id).is_some_and(|node| match &node.data {
            NodeKind::State { operation } => matches!(
                operation,
                StateOperation::Material(_)
                    | StateOperation::Medium { .. }
                    | StateOperation::ReverseOrientation
            ),
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                ..
            } => true,
            _ => false,
        })
    }

    /// Whether `node` is one of the group's own head components.
    fn head_contains(&self, group: &NodeId, node: Option<NodeId>) -> bool {
        node.is_some_and(|node| {
            let Some(children) = self.scope_children(*group) else {
                return false;
            };
            children
                .iter()
                .take_while(|child| self.is_component(child))
                .any(|child| *child == node)
        })
    }

    /// Transform operations in effect for an object: from the start of its
    /// parent scope up to the object, or for a group, from its own start up to
    /// its first object child.
    fn transform(&self, id: NodeId, group: bool) -> Value {
        let is_camera = matches!(
            self.view.node(id).map(|node| &node.data),
            Some(NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            })
        );
        let (container, end) = if is_camera {
            (None, 0)
        } else if group {
            let children = match self.view.node(id).map(|node| &node.data) {
                Some(NodeKind::Scope { children, .. }) => children.as_slice(),
                _ => &[],
            };
            let end = children
                .iter()
                .position(|child| is_object(&self.view, *child))
                .unwrap_or(children.len());
            (Some(id), end)
        } else {
            let parent = self.parents.get(&id).copied();
            let end = parent
                .and_then(|parent| self.scope_children(parent))
                .and_then(|children| children.iter().position(|child| *child == id))
                .unwrap_or(0);
            (parent, end)
        };
        let children = container
            .and_then(|container| self.scope_children(container))
            .unwrap_or(&[]);
        let mut operations = Vec::new();
        let mut last = None;
        let camera_options = is_camera.then_some(self.view.scene.options.as_slice());
        let camera_end = camera_options
            .and_then(|options| options.iter().position(|child| *child == id))
            .unwrap_or(0);
        let operation_nodes: Vec<_> = if is_camera {
            self.view.scene.options[..camera_end].to_vec()
        } else {
            children.iter().take(end).copied().collect()
        };
        for (index, child) in operation_nodes.iter().enumerate() {
            let Some(NodeKind::State { operation }) = self.view.node(*child).map(|node| &node.data)
            else {
                continue;
            };
            let value = match operation {
                StateOperation::Transform(operation) => json!(operation),
                StateOperation::UseCoordinates(_) => json!(operation),
                _ => continue,
            };
            operations.push(json!({"node_id":child,"value":value}));
            last = Some(index);
        }
        let shared_with = if is_camera {
            0
        } else {
            last.map_or(0, |last| {
                children
                    .iter()
                    .skip(last + 1)
                    .filter(|child| **child != id && is_object(&self.view, **child))
                    .count()
            })
        };
        // A group's world matrix is the state after its leading operations.
        let state = if group {
            last.and_then(|last| self.evaluated.nodes.get(&children[last]))
                .or_else(|| self.evaluated.nodes.get(&id))
        } else {
            self.evaluated.nodes.get(&id)
        };
        // A camera's world placement is its pose (camera-to-world);
        // a pose-less camera (an older import) keeps the inverse of the
        // transform current at its declaration.
        let camera_world = is_camera.then(|| {
            let pose = match self.view.node(id).map(|node| &node.data) {
                Some(NodeKind::Entity { pose, .. }) => *pose,
                _ => None,
            };
            match pose {
                Some(pose) => Some((
                    project::math::Mat4(pose.start),
                    project::math::Mat4(pose.end),
                )),
                None => state.and_then(|state| {
                    Some((
                        project::math::inverse(state.start)?,
                        project::math::inverse(state.end)?,
                    ))
                }),
            }
        });
        let pose = |matrix: project::math::Mat4| {
            if is_camera {
                project::math::inverse(matrix)
            } else {
                Some(matrix)
            }
        };
        let camera_trs = |start: project::math::Mat4, end: project::math::Mat4| -> (Value, Value) {
            if start == end {
                trs_json(&start)
            } else {
                (Value::Null, json!("animated"))
            }
        };
        let (world_trs, world_trs_reason) = if let Some(samples) = camera_world {
            match samples {
                Some((start, end)) => camera_trs(start, end),
                None => (Value::Null, json!("singular")),
            }
        } else {
            match state {
                Some(state) if state.start == state.end => match pose(state.start) {
                    Some(matrix) => trs_json(&matrix),
                    None => (Value::Null, json!("singular")),
                },
                Some(_) => (Value::Null, json!("animated")),
                None => (Value::Null, Value::Null),
            }
        };
        json!({
            "kind":"transform",
            "operations":operations,
            "shared_with":shared_with,
            "world":camera_world.map(|samples| match samples {
                Some((start, end)) => json!({"start":start.0,"end":end.0}),
                None => Value::Null,
            }).unwrap_or_else(|| state.and_then(|state| {
                Some(json!({"start":pose(state.start)?.0,"end":pose(state.end)?.0}))
            }).unwrap_or(Value::Null)),
            "reverse_orientation":state.is_some_and(|state| state.reverse_orientation),
            "local":self.local_transform(id, group),
            "world_trs":world_trs,
            "world_trs_reason":world_trs_reason,
        })
    }

    /// The node's local matrix and its TRS: a group's leading run, an
    /// object's dedicated group run (identity without one), or a camera's
    /// pose. `World = ParentWorld × Local` (and `World = Local` for a
    /// `root` parent): `parent` reports which parent the local composes
    /// against — "root" for cameras and World-direct units, "group" inside
    /// a group, `{"camera": id}` for a group attached to a camera.
    fn local_transform(&self, id: NodeId, group: bool) -> Value {
        let node = self.view.node(id).map(|node| &node.data);
        let local = if let Some(NodeKind::Entity {
            entity: EntityKind::Camera,
            ..
        }) = node
        {
            self.camera_pose_local(id)
        } else {
            let scope = if group {
                Some(id)
            } else {
                self.dedicated_group(id)
            };
            match scope {
                // No dedicated group: the object adds nothing to its parent.
                None => Some(Ok(project::math::Mat4::identity())),
                Some(scope) => {
                    let children = self.scope_children(scope).unwrap_or(&[]);
                    let end = if group {
                        children
                            .iter()
                            .position(|child| is_object(&self.view, *child))
                            .unwrap_or(children.len())
                    } else {
                        children.iter().position(|child| *child == id).unwrap_or(0)
                    };
                    let run = &children[..end];
                    let coordinates = run.iter().any(|child| {
                        matches!(
                            self.view.node(*child).map(|node| &node.data),
                            Some(NodeKind::State {
                                operation: StateOperation::DefineCoordinates(_)
                                    | StateOperation::UseCoordinates(_)
                            })
                        )
                    });
                    // A scope's entry in the evaluation is the state it starts
                    // from; the run ends at its last node, or at the object.
                    // A camera-attached scope enters at the camera's pose, so
                    // the same composition yields the camera-relative local.
                    let exit = if group {
                        run.last().and_then(|last| self.evaluated.nodes.get(last))
                    } else {
                        self.evaluated.nodes.get(&id)
                    };
                    let entry = self.evaluated.nodes.get(&scope);
                    entry.map(|entry| {
                        let exit = exit.unwrap_or(entry);
                        transform_trs::local_matrix(entry, exit, coordinates)
                    })
                }
            }
        };
        let parent = self.local_parent(id, group);
        match local {
            None => Value::Null,
            Some(Err(reason)) => {
                let mut local = json!({
                    "matrix":null,"matrix_reason":reason,"trs":null,"trs_reason":null,"parent":parent,
                });
                if reason == transform_trs::LocalError::Animated
                    && let Some(samples) = self.local_transform_samples(id, group)
                {
                    local["samples"] = samples;
                }
                local
            }
            Some(Ok(matrix)) => {
                let (trs, trs_reason) = trs_json(&matrix);
                json!({
                    "matrix":matrix.0,"matrix_reason":null,"trs":trs,"trs_reason":trs_reason,
                    "parent":parent,
                })
            }
        }
    }

    /// Which parent the node's local transform composes against: the camera
    /// a scope is attached to, the root of its tree (the World or a
    /// definition), or the surrounding group.
    fn local_parent(&self, id: NodeId, group: bool) -> Value {
        if matches!(
            self.view.node(id).map(|node| &node.data),
            Some(NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            })
        ) {
            return json!("root");
        }
        let scope = self.dedicated_group(id).filter(|_| !group).unwrap_or(id);
        if let Some(camera) = parent_camera_of(&self.view, scope) {
            return json!({"camera": camera});
        }
        match self.parents.get(&scope) {
            // World-direct units compose against the identity root.
            Some(parent) if *parent == self.view.scene.world => json!("root"),
            Some(_) => json!("group"),
            None => json!("root"),
        }
    }

    /// A camera's local transform is its pose directly, or, for an older
    /// import without a pose, the inverse of the transform
    /// current at its declaration.
    fn camera_pose_local(
        &self,
        id: NodeId,
    ) -> Option<Result<project::math::Mat4, transform_trs::LocalError>> {
        let pose = match self.view.node(id).map(|node| &node.data) {
            Some(NodeKind::Entity { pose, .. }) => *pose,
            _ => None,
        };
        match pose {
            Some(pose) if pose.start == pose.end => Some(Ok(project::math::Mat4(pose.start))),
            Some(_) => Some(Err(transform_trs::LocalError::Animated)),
            None => self.evaluated.nodes.get(&id).map(|state| {
                if state.start != state.end {
                    Err(transform_trs::LocalError::Animated)
                } else {
                    project::math::inverse(state.start).ok_or(transform_trs::LocalError::Singular)
                }
            }),
        }
    }

    /// The two evaluated local endpoints for an animated transform. This is
    /// a read-only view; the static local-transform edit API still refuses an
    /// animated local. Samples use the node's TransformTimes interval.
    fn local_transform_samples(&self, id: NodeId, group: bool) -> Option<Value> {
        let node = self.view.node(id)?;
        if matches!(
            node.data,
            NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            }
        ) {
            let state = self.evaluated.nodes.get(&id)?;
            let samples = match &node.data {
                NodeKind::Entity {
                    pose: Some(pose), ..
                } => (
                    project::math::Mat4(pose.start),
                    project::math::Mat4(pose.end),
                ),
                _ => (
                    project::math::inverse(state.start)
                        .ok_or(transform_trs::LocalError::Singular)
                        .ok()?,
                    project::math::inverse(state.end)
                        .ok_or(transform_trs::LocalError::Singular)
                        .ok()?,
                ),
            };
            let root = json!("root");
            return Some(json!({
                "times":state.times,
                "start":local_sample(Ok(samples.0), &root),
                "end":local_sample(Ok(samples.1), &root),
            }));
        }

        let scope = if group {
            Some(id)
        } else {
            self.dedicated_group(id)
        }?;
        let children = self.scope_children(scope)?;
        let end = if group {
            children
                .iter()
                .position(|child| is_object(&self.view, *child))
                .unwrap_or(children.len())
        } else {
            children.iter().position(|child| *child == id).unwrap_or(0)
        };
        let run = &children[..end];
        let coordinates = run.iter().any(|child| {
            matches!(
                self.view.node(*child).map(|node| &node.data),
                Some(NodeKind::State {
                    operation: StateOperation::DefineCoordinates(_)
                        | StateOperation::UseCoordinates(_),
                })
            )
        });
        let entry = self.evaluated.nodes.get(&scope)?;
        let exit = if group {
            run.last()
                .and_then(|last| self.evaluated.nodes.get(last))
                .unwrap_or(entry)
        } else {
            self.evaluated.nodes.get(&id)?
        };
        let parent = self.local_parent(id, group);
        let sample = |parent_world: project::math::Mat4, world: project::math::Mat4| {
            if coordinates {
                return Err(transform_trs::LocalError::CoordinateSystem);
            }
            let local = project::math::inverse(parent_world)
                .ok_or(transform_trs::LocalError::Singular)?
                .compose(world);
            if transform_trs::approx_eq(&parent_world.compose(local), &world) {
                Ok(local)
            } else {
                Err(transform_trs::LocalError::Singular)
            }
        };
        Some(json!({
            "times":exit.times,
            "start":local_sample(sample(entry.start, exit.start), &parent),
            "end":local_sample(sample(entry.end, exit.end), &parent),
        }))
    }

    /// The object's dedicated group (D.3 rule 1): an Attribute scope that is
    /// not the World and holds the object as its only object child.
    fn dedicated_group(&self, object: NodeId) -> Option<NodeId> {
        let parent = self.parents.get(&object).copied()?;
        (parent != self.view.scene.world && owned_object(&self.view, parent) == Some(object))
            .then_some(parent)
    }

    /// The reverse-orientation toggle of the object's dedicated group, if one
    /// sits in the run immediately before the object.
    fn local_toggle(&self, object: NodeId) -> Option<NodeId> {
        // The same dedicated-group predicate the session applies.
        let parent = self.dedicated_group(object)?;
        let children = self.scope_children(parent)?;
        let index = children.iter().position(|child| *child == object)?;
        children
            .iter()
            .take(index)
            .rev()
            .take_while(|child| {
                self.view
                    .node(**child)
                    .is_some_and(|node| match &node.data {
                        NodeKind::State { operation } => matches!(
                            operation,
                            StateOperation::Material(_)
                                | StateOperation::Medium { .. }
                                | StateOperation::ReverseOrientation
                        ),
                        NodeKind::Entity {
                            entity: EntityKind::AreaLight,
                            ..
                        } => true,
                        _ => false,
                    })
            })
            .copied()
            .find(|child| {
                matches!(
                    self.view.node(*child).map(|node| &node.data),
                    Some(NodeKind::State {
                        operation: StateOperation::ReverseOrientation,
                        ..
                    })
                )
            })
    }

    fn scope_children(&self, id: NodeId) -> Option<&'a [NodeId]> {
        match self.view.node(id).map(|node| &node.data) {
            Some(NodeKind::Scope { children, .. }) => Some(children.as_slice()),
            _ => None,
        }
    }

    /// Whether a binding node sits in the object's dedicated group (D.3 rule 1).
    fn local(&self, object: NodeId, binding: Option<NodeId>) -> bool {
        let Some(parent) = self.parents.get(&object).copied() else {
            return false;
        };
        binding.is_some_and(|binding| self.parents.get(&binding) == Some(&parent))
            && owned_object(&self.view, parent) == Some(object)
    }

    fn material(&self, id: NodeId) -> Value {
        let state = self.evaluated.nodes.get(&id);
        let binding = state.map_or(MaterialBinding::Default, |state| state.material.clone());
        let node = state.and_then(|state| state.material_node);
        json!({"kind":"material","binding":binding,"binding_node":node,"local":self.local(id, node)})
    }

    fn medium(&self, id: NodeId) -> Value {
        let state = self.evaluated.nodes.get(&id);
        let node = state.and_then(|state| state.medium_node);
        json!({
            "kind":"medium",
            "inside":state.and_then(|state| state.inside_medium),
            "outside":state.and_then(|state| state.outside_medium),
            "binding_node":node,
            "local":self.local(id, node),
        })
    }

    /// PBRT cameras use the scene-level MediumInterface's outside binding as
    /// their ray medium. Keep the wire component single-sided; the underlying
    /// state still retains both values for round-tripping.
    fn camera_medium(&self, id: NodeId) -> Value {
        let state = self.evaluated.nodes.get(&id);
        json!({
            "kind":"medium",
            "binding":state.and_then(|state| state.outside_medium),
            "preserved_inside":state.and_then(|state| state.inside_medium),
            "binding_node":state.and_then(|state| state.medium_node),
            "local":self.local(id, state.and_then(|state| state.medium_node)),
        })
    }
}

/// A PLY asset's header counts and attributes as the wire shows them
/// (`project.get_asset`'s `mesh`, plymesh statistics), or None when it
/// cannot be read. Only the start of the file is read, and once per
/// content: an asset's bytes never change under the same SHA-256, and
/// `get_asset` runs on every revision for open editor tabs.
pub(super) fn ply_info_of(asset: &project::Asset, asset_root: &str) -> Option<Value> {
    let path = std::path::Path::new(asset_root).join(&asset.location);
    ply_info_at(asset, &path)
}

/// Header statistics for a PLY at an explicit path - the assets folder,
/// or the staged payload an unsaved import still keeps its bytes in.
pub(super) fn ply_info_at(asset: &project::Asset, path: &std::path::Path) -> Option<Value> {
    static SEEN: std::sync::OnceLock<std::sync::Mutex<BTreeMap<String, Value>>> =
        std::sync::OnceLock::new();
    let seen = SEEN.get_or_init(Default::default);
    if let Some(info) = seen
        .lock()
        .ok()
        .and_then(|seen| seen.get(&asset.sha256).cloned())
    {
        return Some(info);
    }
    let info = scene_parse::ply::ply_info_from_file(path).ok()?;
    let info = json!({"vertices":info.vertices,"faces":info.faces,
                      "normals":info.normals,"uvs":info.uvs,"format":info.format});
    if let Ok(mut seen) = seen.lock() {
        seen.insert(asset.sha256.clone(), info.clone());
    }
    Some(info)
}

/// Vertex and face counts for mesh shapes whose topology is inline.
fn statistics(implementation: &str, parameters: &[project::Parameter]) -> Value {
    let count = |name: &str| {
        parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .map(|parameter| match &parameter.values {
                ParameterValues::Floats(values) => values.len() as u64,
                ParameterValues::Integers(values) => values.len() as u64,
                ParameterValues::Array { count, .. } => *count,
                _ => 0,
            })
    };
    let Some(points) = count("P") else {
        return Value::Null;
    };
    let vertices = points / 3;
    let faces = match (implementation, count("indices")) {
        ("trianglemesh", Some(indices)) => indices / 3,
        ("trianglemesh", None) if vertices == 3 => 1,
        ("bilinearmesh", Some(indices)) => indices / 4,
        ("bilinearmesh", None) if vertices == 4 => 1,
        _ => return Value::Null,
    };
    json!({"vertices":vertices,"faces":faces})
}

/// Render settings of the Scene root, in Inspector order. The Cameras are
/// projected onto their scene-graph rows; the active Film is a Scene root
/// component rather than a Camera one.
pub(super) fn scene_components(view: &SceneView) -> Vec<Value> {
    let options = &view.scene.options;
    let nodes = || options.iter().filter_map(|id| view.node(*id));
    let mut components = Vec::new();
    let cameras = options
        .iter()
        .filter_map(|id| {
            let node = view.node(*id)?;
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
            .then(|| json!({"id":node.id,"label":node.label}))
        })
        .collect::<Vec<_>>();
    // The Scene box: up/handedness selectors. Metadata only.
    components.push(json!({
        "kind":"scene",
        "scene_id":view.scene.id,
        "coordinate_system":view.scene.coordinate_system,
    }));
    // Which render entities the Options hold: the existence
    // map Add Component still reads, Film included.
    let has = |kind: ConfigurationKind| {
        nodes().any(|node| {
            matches!(
                &node.data,
                NodeKind::Configuration {
                    configuration: found,
                    ..
                } if *found == kind
            )
        })
    };
    components.push(json!({
        "kind":"render_settings",
        "camera":view.scene.render_camera,
        "cameras":cameras,
        "scene_id":view.scene.id,
        "configurations":{
            "film":has(ConfigurationKind::Film),
            "sampler":has(ConfigurationKind::Sampler),
            "integrator":has(ConfigurationKind::Integrator),
            "pixel_filter":has(ConfigurationKind::PixelFilter),
            "accelerator":has(ConfigurationKind::Accelerator),
        },
    }));
    // The active Film — the last Film directive — is a Scene root
    // component whether or not a render camera is set (stage, user
    // never the Camera's.
    if let Some((film, implementation)) = active_film(view) {
        components.push(json!({
            "kind":"film","node_id":film,"implementation":implementation,
            "parameter_target":{"kind":"node","id":film},
        }));
    }
    for (configuration, kind) in [
        (ConfigurationKind::Sampler, "sampler"),
        (ConfigurationKind::Integrator, "integrator"),
        (ConfigurationKind::PixelFilter, "pixel_filter"),
        (ConfigurationKind::Accelerator, "accelerator"),
    ] {
        for node in nodes() {
            if let NodeKind::Configuration {
                configuration: current,
                implementation,
                ..
            } = &node.data
                && *current == configuration
            {
                components.push(json!({
                    "kind":kind,"node_id":node.id,"implementation":implementation,
                    "parameter_target":{"kind":"node","id":node.id},
                }));
            }
        }
    }
    for node in nodes() {
        if let NodeKind::State {
            operation: StateOperation::ColorSpace(name),
        } = &node.data
        {
            components.push(json!({"kind":"color_space","node_id":node.id,"value":name}));
        }
    }
    let values = nodes()
        .filter_map(|node| match &node.data {
            NodeKind::Option { name, value } => {
                Some(json!({"node_id":node.id,"name":name,"value":value}))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    if !values.is_empty() {
        components.push(json!({"kind":"options","values":values}));
    }
    components
}
