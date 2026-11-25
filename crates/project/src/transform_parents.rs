//! Resolve PBRT transform-parent resets into the hierarchy at import.
//!
//! `Identity`, `Transform [m]` and `CoordSysTransform "name"` set the
//! *transform parent* of the directives that follow them. The model keeps
//! them as states, which makes the hierarchy parent and the transform parent
//! disagree after import. This module rewrites a freshly parsed scene so
//! that every node's transform parent is its hierarchy parent, without
//! changing the evaluated meaning.
//!
//! The rewrite has two structural steps, one evaluation to base
//! every judgment on, and one pass over the remaining resets in document
//! order. Identity checks are exact: an approximate match would
//! delete small transforms that only look like the identity.

use crate::math::{Mat4, inverse};
use crate::{
    ActiveSamples, CameraPose, EntityKind, EvaluatedScene, GraphicsState, ImportDiagnostic,
    ImportSeverity, MaterialBinding, Node, NodeId, NodeKind, ProjectDocument, ResourceKind, Result,
    SceneId, ScopeKind, StateOperation, TransformOperation, invalid,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};

/// Rewrite the scene's World tree and object-definition bodies so that no
/// transform-reset state remains. The document must be a freshly parsed
/// import; the Options list is left to later stages. Equivalence
/// with the input is guaranteed by construction and checked against by
/// tests.
pub fn resolve_transform_parents(
    document: &mut ProjectDocument,
    scene: SceneId,
) -> Result<Vec<ImportDiagnostic>> {
    let world = match document.scene(scene) {
        Some(scene) => scene.world,
        None => return Err(invalid("scene is missing")),
    };
    let definitions = definition_roots(document);
    // The conversion needs the END states of Transform scopes, which
    // disappear once the scopes become Attribute scopes. Evaluate the
    // unconverted tree first: the conversion reproduces every stream value,
    // so these states stay valid for the whole conversion.
    let conversion_states = evaluate_snapshot(document, scene)?;
    convert_transform_scopes(document, scene, world, &definitions, &conversion_states)?;
    wrap_world_transform_states(document, scene, world, &definitions)?;
    // Evaluate once for each reset decision.
    let snapshot = evaluate_snapshot(document, scene)?;
    let mut pass = Pass::build(document, scene, world, definitions, snapshot)?;
    pass.run()?;
    Ok(std::mem::take(&mut pass.diagnostics))
}

fn definition_roots(document: &ProjectDocument) -> Vec<NodeId> {
    document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::ObjectDefinition)
        .filter_map(|resource| resource.definition_scope)
        .collect()
}

fn evaluate_snapshot(document: &ProjectDocument, scene: SceneId) -> Result<EvaluatedScene> {
    let view = document.scene_view(scene)?;
    crate::evaluate(&view)
}

// -- node tables and children ---------------------------------------------------

fn node(document: &ProjectDocument, scene: SceneId, id: NodeId) -> Option<&Node> {
    document
        .scene(scene)
        .and_then(|scene| scene.nodes.get(&id))
        .or_else(|| document.definition_nodes.get(&id))
}

fn node_mut(document: &mut ProjectDocument, scene: SceneId, id: NodeId) -> Option<&mut Node> {
    if document
        .scene(scene)
        .is_some_and(|scene| scene.nodes.contains_key(&id))
    {
        let scene = document
            .scenes
            .get_mut(&scene)
            .map(std::sync::Arc::make_mut)
            .expect("scene exists");
        return scene.nodes.get_mut(&id);
    }
    document.definition_nodes.get_mut(&id)
}

fn children(document: &ProjectDocument, scene: SceneId, id: NodeId) -> Vec<NodeId> {
    match node(document, scene, id).map(|node| &node.data) {
        Some(NodeKind::Scope { children: ids, .. }) => ids.clone(),
        _ => Vec::new(),
    }
}

fn set_children(document: &mut ProjectDocument, scene: SceneId, id: NodeId, ids: Vec<NodeId>) {
    if let Some(NodeKind::Scope {
        children: slots, ..
    }) = node_mut(document, scene, id).map(|node| &mut node.data)
    {
        *slots = ids;
    }
}

fn remove_node(document: &mut ProjectDocument, scene: SceneId, id: NodeId) {
    if document
        .scene(scene)
        .is_some_and(|scene| scene.nodes.contains_key(&id))
    {
        document
            .scenes
            .get_mut(&scene)
            .map(std::sync::Arc::make_mut)
            .expect("scene exists")
            .nodes
            .remove(&id);
        return;
    }
    document.definition_nodes.remove(&id);
}

/// Add a freshly created node to the table of the tree it belongs to.
fn register_node(
    document: &mut ProjectDocument,
    scene: SceneId,
    definition_trees: &HashSet<NodeId>,
    tree: NodeId,
    node: Node,
) -> NodeId {
    let id = node.id;
    if definition_trees.contains(&tree) {
        document.definition_nodes.insert(id, node);
    } else {
        document
            .scenes
            .get_mut(&scene)
            .map(std::sync::Arc::make_mut)
            .expect("scene exists")
            .nodes
            .insert(id, node);
    }
    id
}

fn is_transform_state(kind: &NodeKind) -> bool {
    matches!(
        kind,
        NodeKind::State {
            operation: StateOperation::Transform(_)
        }
    )
}

// -- graphics-state materialization -------------------------------------

/// The graphics-state difference between two states, written as fresh state
/// nodes. Only expressible differences are written: PBRT has no directive
/// that clears an area light or resets a material to the default, and an
/// import can never produce those differences.
fn materialize(
    document: &ProjectDocument,
    scene: SceneId,
    target: &GraphicsState,
    base: &GraphicsState,
) -> Result<Vec<Node>> {
    let mut nodes = Vec::new();
    if target.material != base.material {
        match &target.material {
            MaterialBinding::Default => {
                // Nothing re-establishes the default binding once it was
                // overridden, so an import cannot reach this; export could
                // not express it either (emit writes nothing for Default).
                debug_assert!(false, "material reset to the default cannot be written");
            }
            binding => {
                let label = if matches!(binding, MaterialBinding::Resource(_)) {
                    "NamedMaterial"
                } else {
                    "Material"
                };
                nodes.push(Node::new(
                    label,
                    NodeKind::State {
                        operation: StateOperation::Material(binding.clone()),
                    },
                ));
            }
        }
    }
    if target.inside_medium != base.inside_medium || target.outside_medium != base.outside_medium {
        nodes.push(Node::new(
            "MediumInterface",
            NodeKind::State {
                operation: StateOperation::Medium {
                    inside: target.inside_medium,
                    outside: target.outside_medium,
                },
            },
        ));
    }
    if target.color_space != base.color_space {
        nodes.push(Node::new(
            "ColorSpace",
            NodeKind::State {
                operation: StateOperation::ColorSpace(target.color_space.clone()),
            },
        ));
    }
    if target.reverse_orientation != base.reverse_orientation {
        nodes.push(Node::new(
            "ReverseOrientation",
            NodeKind::State {
                operation: StateOperation::ReverseOrientation,
            },
        ));
    }
    if target.area_light != base.area_light {
        match target.area_light.and_then(|id| node(document, scene, id)) {
            Some(existing) => {
                // One input directive lit both sides of the rewrite. As
                // separate nodes, the area light is duplicated so each side
                // can be edited independently.
                let mut duplicate = Node::new(existing.label.clone(), existing.data.clone());
                duplicate.origin = existing.origin.clone();
                nodes.push(duplicate);
            }
            None => {
                debug_assert!(
                    base.area_light.is_none(),
                    "an area light cannot be cleared by a state write"
                );
            }
        }
    }
    Ok(nodes)
}

// --: Transform scopes become Attribute scopes ---------------------------------

/// The state a Transform scope's stream leaves behind when it closes: the
/// entry transform endures, the graphics state of its last directive leaks.
fn transform_scope_end_state(
    document: &ProjectDocument,
    scene: SceneId,
    snapshot: &EvaluatedScene,
    scope: NodeId,
) -> GraphicsState {
    let entry = snapshot
        .nodes
        .get(&scope)
        .cloned()
        .expect("the scope's entry state is evaluated");
    // Walk down the rightmost path while it ends in another Transform
    // scope: only those leak their own tail's graphics state further. A
    // state node ends the stream, and an Attribute scope restores its entry,
    // so in both cases the stored state of the endpoint is the leak.
    let mut current = scope;
    let endpoint = loop {
        match children(document, scene, current).last().copied() {
            None => break None,
            Some(last) => match node(document, scene, last).map(|node| &node.data) {
                Some(NodeKind::Scope {
                    scope: ScopeKind::Transform,
                    ..
                }) => current = last,
                _ => break Some(last),
            },
        }
    };
    let leaked = match endpoint {
        Some(endpoint) => snapshot
            .nodes
            .get(&endpoint)
            .cloned()
            .expect("the endpoint state is evaluated"),
        None => snapshot
            .nodes
            .get(&current)
            .cloned()
            .expect("the deepest scope's entry state is evaluated"),
    };
    GraphicsState {
        start: entry.start,
        end: entry.end,
        active: entry.active,
        times: entry.times,
        material: leaked.material,
        material_node: leaked.material_node,
        inside_medium: leaked.inside_medium,
        outside_medium: leaked.outside_medium,
        medium_node: leaked.medium_node,
        area_light: leaked.area_light,
        reverse_orientation: leaked.reverse_orientation,
        color_space: leaked.color_space,
    }
}

fn convert_transform_scopes(
    document: &mut ProjectDocument,
    scene: SceneId,
    world: NodeId,
    definitions: &[NodeId],
    snapshot: &EvaluatedScene,
) -> Result<()> {
    let definition_trees: HashSet<NodeId> = definitions.iter().copied().collect();
    // Every Transform scope with its parent, innermost first: a scope's
    // writes are then in place around the enclosing scope when its own
    // difference is taken beside it.
    let mut found: Vec<(NodeId, NodeId, NodeId, usize)> = Vec::new();
    for tree in std::iter::once(world).chain(definitions.iter().copied()) {
        let mut stack: Vec<(NodeId, Option<NodeId>, usize)> = vec![(tree, None, 0)];
        while let Some((id, parent, depth)) = stack.pop() {
            if matches!(
                node(document, scene, id).map(|node| &node.data),
                Some(NodeKind::Scope {
                    scope: ScopeKind::Transform,
                    ..
                })
            ) && let Some(parent) = parent
            {
                found.push((id, parent, tree, depth));
            }
            for child in children(document, scene, id).into_iter().rev() {
                stack.push((child, Some(id), depth + 1));
            }
        }
    }
    found.sort_by_key(|(_, _, _, depth)| std::cmp::Reverse(*depth));
    // Every scope's end state comes from the original tree, before any
    // re-kinding or materialization write exists: the walk then still sees
    // the Transform kinds it descends through, and its endpoints are nodes
    // the snapshot evaluated.
    let ends: Vec<GraphicsState> = found
        .iter()
        .map(|(scope, _, _, _)| transform_scope_end_state(document, scene, snapshot, *scope))
        .collect();
    for (scope, _, _, _) in &found {
        if let Some(target) = node_mut(document, scene, *scope)
            && let NodeKind::Scope { scope, .. } = &mut target.data
        {
            *scope = ScopeKind::Attribute;
        }
    }
    for ((scope, parent, tree, _), end) in found.into_iter().zip(ends) {
        let entry = snapshot
            .nodes
            .get(&scope)
            .cloned()
            .expect("the scope's entry state is evaluated");
        let writes = materialize(document, scene, &end, &entry)?;
        for (offset, write) in writes.into_iter().enumerate() {
            let id = write.id;
            register_node(document, scene, &definition_trees, tree, write);
            let mut kids = children(document, scene, parent);
            let position = kids
                .iter()
                .position(|id| *id == scope)
                .expect("the scope is its parent's child");
            kids.insert(position + 1 + offset, id);
            set_children(document, scene, parent, kids);
        }
    }
    Ok(())
}

// --: wrap root-level transform states -----------------------------------------

fn wrap_world_transform_states(
    document: &mut ProjectDocument,
    scene: SceneId,
    world: NodeId,
    definitions: &[NodeId],
) -> Result<()> {
    let definition_trees: HashSet<NodeId> = definitions.iter().copied().collect();
    for root in std::iter::once(&world).chain(definitions.iter()) {
        let kids = children(document, scene, *root);
        if kids.is_empty() {
            continue;
        }
        let Some(first) = kids.iter().position(|id| {
            node(document, scene, *id).is_some_and(|node| is_transform_state(&node.data))
        }) else {
            continue;
        };
        // The wrapper takes everything from the first transform state to the
        // end; it has nothing after it, so this changes no stream value.
        let wrapper = Node::new(
            "World Transform",
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: kids[first..].to_vec(),
                owned_object: None,
                parent_camera: None,
            },
        );
        let id = register_node(document, scene, &definition_trees, *root, wrapper);
        let mut head = kids[..first].to_vec();
        head.push(id);
        set_children(document, scene, *root, head);
    }
    Ok(())
}

// --: the reset pass -------------------------------------------------------

struct Pass<'a> {
    document: &'a mut ProjectDocument,
    scene: SceneId,
    world: NodeId,
    definitions: Vec<NodeId>,
    /// The graphics state before each reset, captured once before any
    /// mutation. Rewrites reproduce the evaluated stream, so the captured
    /// values stay valid for later resets too.
    before: BTreeMap<NodeId, GraphicsState>,
    /// The current parent of every node.
    parent: BTreeMap<NodeId, NodeId>,
    resets: Vec<NodeId>,
    snapshot: EvaluatedScene,
    used_names: BTreeSet<String>,
    /// Binding of every resolved coordinate use, in document order.
    use_binding: BTreeMap<NodeId, NodeId>,
    /// The group created per used coordinate-system definition.
    groups: BTreeMap<NodeId, NodeId>,
    /// The Options list at the start of the pass; a definition's group
    /// removes it from the live list, and later uses still need to know.
    initial_options: BTreeSet<NodeId>,
    /// The stream state a coordinate group's children start from.
    group_children_start: BTreeMap<NodeId, GraphicsState>,
    /// Entry state of the groups created by this pass; created groups are
    /// Attribute scopes, so their exit state is this state.
    created_entry: BTreeMap<NodeId, GraphicsState>,
    /// The last group placed for each anchor, keeping placements in document
    /// order when several tails land after the same ancestor.
    last_after: BTreeMap<NodeId, NodeId>,
    diagnostics: Vec<ImportDiagnostic>,
}

/// The kind a reset resolves to, with everything its move needs.
#[derive(Clone)]
enum ResolvedKind {
    Identity,
    Replace,
    Coordinate {
        mode: CoordinateMode,
        /// The coordinate's value per sample.
        value: Box<(Mat4, Mat4)>,
        diagnostic: Option<String>,
    },
}

#[derive(Clone, Copy)]
enum CoordinateMode {
    /// The reset's group attaches to the scene's camera node.
    Camera { camera: NodeId },
    /// The reset's container becomes a child of the definition's group.
    Group { group: NodeId },
    /// The coordinate is baked into matrices, with a diagnostic.
    Bake,
}

impl ResolvedKind {
    /// Whether a fresh (or moved) container must carry the attachment, and so
    /// the root scope cannot absorb the reset in place.
    fn needs_container(&self) -> bool {
        matches!(
            self,
            ResolvedKind::Coordinate {
                mode: CoordinateMode::Camera { .. } | CoordinateMode::Group { .. },
                ..
            }
        )
    }

    /// The camera a resolved kind attaches its container to, if any.
    fn parent_camera(&self) -> Option<NodeId> {
        match self {
            ResolvedKind::Coordinate {
                mode: CoordinateMode::Camera { camera },
                ..
            } => Some(*camera),
            _ => None,
        }
    }

    /// The samples the reset does not replace keep their captured value; the
    /// replaced ones start from an identity base (the reset behaves as a
    /// concat from there) or from the baked coordinate value.
    fn targets(&self, before: &GraphicsState) -> (Mat4, Mat4) {
        let identity = Mat4::identity();
        let (start_affected, end_affected) = affected_samples(before.active);
        let value = match self {
            ResolvedKind::Coordinate { value, .. } => **value,
            _ => (identity, identity),
        };
        (
            if start_affected {
                value.0
            } else {
                before.start
            },
            if end_affected { value.1 } else { before.end },
        )
    }
}

/// Where a reset's container lands: in the gap, at the reset's own position
/// under a root scope, under a coordinate group, or nowhere at all (the
/// whole-scope rewrite of a root).
#[derive(Clone, Copy)]
enum Placement {
    Stay,
    AfterAnchor { anchor: NodeId },
    RootChild { root: NodeId, index: usize },
    GroupChild { group: NodeId },
}

/// The kinds this pass resolves: `Identity`, `Transform [m]`, and every
/// `CoordSysTransform` in the scene's World tree and definitions.
fn is_reset_kind(kind: &NodeKind) -> Option<ResolvedKind> {
    match kind {
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Identity),
        } => Some(ResolvedKind::Identity),
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Replace(_)),
        } => Some(ResolvedKind::Replace),
        NodeKind::State {
            operation: StateOperation::UseCoordinates(_),
        } => Some(ResolvedKind::Coordinate {
            mode: CoordinateMode::Bake,
            value: Box::new((Mat4::identity(), Mat4::identity())),
            diagnostic: None,
        }),
        _ => None,
    }
}

impl<'a> Pass<'a> {
    fn build(
        document: &'a mut ProjectDocument,
        scene: SceneId,
        world: NodeId,
        definitions: Vec<NodeId>,
        snapshot: EvaluatedScene,
    ) -> Result<Pass<'a>> {
        let mut parent = BTreeMap::new();
        let mut before = BTreeMap::new();
        let mut resets = Vec::new();
        let mut used_names = BTreeSet::new();
        // Coordinate tracking, in stream order: options first, then the World
        // preorder, then the definition bodies. The evaluator runs the
        // definition bodies after the World, in resource order, so a use's
        // binding is the last definition of that name anywhere earlier in
        // that order.
        let mut coordinate_defs: BTreeMap<String, NodeId> = BTreeMap::new();
        let mut use_binding = BTreeMap::new();
        let options = document.scene(scene).expect("scene exists").options.clone();
        for id in &options {
            if let Some(state) = node(document, scene, *id).map(|node| &node.data) {
                collect_used_coordinate_name(state, &mut used_names);
                define_coordinate_name(*id, state, &mut coordinate_defs);
            }
        }
        // A true preorder stack: a node's children run before its later
        // siblings, exactly like the PBRT stream, so resets are collected in
        // document order, and a use binds the definitions its stream
        // position already has.
        let mut stack: Vec<NodeId> = definitions
            .iter()
            .copied()
            .rev()
            .chain(std::iter::once(world))
            .collect();
        // The last child visited per scope: the entry state of that node is
        // the state the next sibling starts from (attribute scopes restore
        // their entry; state nodes store their post-application state).
        let mut last_child: BTreeMap<NodeId, NodeId> = BTreeMap::new();
        while let Some(id) = stack.pop() {
            let owning = parent.get(&id).copied().unwrap_or(id);
            let kind = &node(document, scene, id)
                .expect("every visited node exists")
                .data;
            if is_reset_kind(kind).is_some() {
                resets.push(id);
                let before_node = last_child.get(&owning).copied().unwrap_or(owning);
                let state = snapshot
                    .nodes
                    .get(&before_node)
                    .cloned()
                    .expect("the state before a reset is evaluated");
                before.insert(id, state);
                if let NodeKind::State {
                    operation: StateOperation::UseCoordinates(name),
                } = kind
                    && let Some(binding) = coordinate_defs.get(name).copied()
                {
                    use_binding.insert(id, binding);
                }
            }
            collect_used_coordinate_name(kind, &mut used_names);
            define_coordinate_name(id, kind, &mut coordinate_defs);
            last_child.insert(owning, id);
            if let NodeKind::Scope { children: kids, .. } = kind {
                for child in kids.iter().rev() {
                    parent.insert(*child, id);
                    stack.push(*child);
                }
            }
        }
        let initial_options = document
            .scene(scene)
            .expect("scene exists")
            .options
            .iter()
            .copied()
            .collect();
        Ok(Pass {
            document,
            scene,
            world,
            definitions,
            before,
            parent,
            resets,
            snapshot,
            used_names,
            use_binding,
            groups: BTreeMap::new(),
            initial_options,
            group_children_start: BTreeMap::new(),
            created_entry: BTreeMap::new(),
            last_after: BTreeMap::new(),
            diagnostics: Vec::new(),
        })
    }

    fn run(&mut self) -> Result<()> {
        let resets = std::mem::take(&mut self.resets);
        for reset in resets {
            self.process(reset)?;
        }
        self.assign_camera_poses()?;
        self.sweep_options_and_coordinates()?;
        Ok(())
    }

    /// every options camera carries its pose as a value, so the Options
    /// transform stream can go. Pose values come from the single evaluation
    /// the pass is based on.
    fn assign_camera_poses(&mut self) -> Result<()> {
        let options = self.options();
        for id in options {
            let is_camera = matches!(
                node(self.document, self.scene, id).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            );
            if !is_camera {
                continue;
            }
            let state = self
                .snapshot
                .nodes
                .get(&id)
                .cloned()
                .expect("the camera state is evaluated");
            let start = inverse(state.start).ok_or_else(|| invalid("singular camera transform"))?;
            let end = inverse(state.end).ok_or_else(|| invalid("singular camera transform"))?;
            let Some(slot) = node_mut(self.document, self.scene, id) else {
                continue;
            };
            let NodeKind::Entity { pose, .. } = &mut slot.data else {
                continue;
            };
            *pose = Some(CameraPose {
                start: start.0,
                end: end.0,
            });
        }
        Ok(())
    }

    fn options(&self) -> Vec<NodeId> {
        self.document
            .scene(self.scene)
            .expect("scene exists")
            .options
            .clone()
    }

    /// The sweep for Options, plus the coordinate
    /// definition state nodes that never became a group: used ones were
    /// replaced at their position, and unused ones simply go.
    fn sweep_options_and_coordinates(&mut self) -> Result<()> {
        for id in self.options() {
            // An options node deleted by an earlier stage of the pass leaves
            // a stale list slot; it goes with the rest.
            let gone = match node(self.document, self.scene, id).map(|node| &node.data) {
                None => true,
                Some(NodeKind::State {
                    operation:
                        StateOperation::Transform(_)
                        | StateOperation::ActiveTransform(_)
                        | StateOperation::DefineCoordinates(_)
                        | StateOperation::UseCoordinates(_),
                }) => true,
                Some(_) => false,
            };
            if !gone {
                continue;
            }
            let scene = std::sync::Arc::make_mut(
                self.document
                    .scenes
                    .get_mut(&self.scene)
                    .expect("scene exists"),
            );
            scene.options.retain(|other| *other != id);
            remove_node(self.document, self.scene, id);
            self.parent.remove(&id);
        }
        let mut leftovers = self
            .world_tree()
            .into_iter()
            .filter(|id| {
                matches!(
                    node(self.document, self.scene, *id).map(|node| &node.data),
                    Some(NodeKind::State {
                        operation: StateOperation::DefineCoordinates(_)
                            | StateOperation::UseCoordinates(_),
                    })
                )
            })
            .collect::<Vec<_>>();
        // Include the options, so a stray Option-level use cannot survive.
        leftovers.extend(self.options().into_iter().filter(|id| {
            matches!(
                node(self.document, self.scene, *id).map(|node| &node.data),
                Some(NodeKind::State {
                    operation: StateOperation::UseCoordinates(_),
                })
            )
        }));
        for id in leftovers {
            if let Some(scope) = self.parent.get(&id).copied() {
                let mut kids = children(self.document, self.scene, scope);
                kids.retain(|other| *other != id);
                set_children(self.document, self.scene, scope, kids);
            }
            let options = self.options();
            if options.contains(&id) {
                let scene = std::sync::Arc::make_mut(
                    self.document
                        .scenes
                        .get_mut(&self.scene)
                        .expect("scene exists"),
                );
                scene.options.retain(|other| *other != id);
            }
            remove_node(self.document, self.scene, id);
            self.parent.remove(&id);
        }
        Ok(())
    }

    /// Every node of the World tree and the definition bodies, in preorder.
    fn world_tree(&self) -> Vec<NodeId> {
        trees(self.document, self.scene, self.world, &self.definitions)
    }

    fn process(&mut self, reset: NodeId) -> Result<()> {
        let (kind, name) = match node(self.document, self.scene, reset).map(|n| &n.data) {
            Some(NodeKind::State {
                operation: StateOperation::UseCoordinates(name),
            }) => (None, Some(name.clone())),
            other => match other.and_then(is_reset_kind) {
                Some(kind) => (Some(kind), None),
                None => return Ok(()),
            },
        };
        let before = self
            .before
            .get(&reset)
            .cloned()
            .expect("every reset has a captured state");
        let kind = match kind {
            Some(kind @ (ResolvedKind::Identity | ResolvedKind::Replace)) => {
                let identity = Mat4::identity();
                let (start_affected, end_affected) = affected_samples(before.active);
                let in_place = (!start_affected || before.start == identity)
                    && (!end_affected || before.end == identity);
                if in_place {
                    // the reset's parent is already the identity in
                    // every affected sample, so the rewrite changes nothing.
                    self.replace_reset(reset, &kind);
                    return Ok(());
                }
                kind
            }
            Some(kind) => kind,
            None => {
                // a coordinate use never takes the in-place route
                // follows the reference through the group, or bakes it.
                self.coordinate_resolution(reset, name.as_deref().unwrap_or_default(), &before)?
            }
        };
        if let Some(message) = kind.diagnostic_message() {
            self.diagnostics.push(ImportDiagnostic {
                code: "coordinate_system_fixed".into(),
                message,
                // The import applied an automatic fix; nothing was lost.
                severity: Some(ImportSeverity::Info),
                origin: node(self.document, self.scene, reset).and_then(|n| n.origin.clone()),
            });
        }
        self.resolve(reset, kind, before)
    }

    /// decide how a coordinate use attaches. `"camera"` follows the
    /// render camera; a named system follows its defining group; everything
    /// the group cannot express is baked into a matrix with a diagnostic.
    fn coordinate_resolution(
        &mut self,
        reset: NodeId,
        name: &str,
        before: &GraphicsState,
    ) -> Result<ResolvedKind> {
        let use_tree = self.tree_of_node(reset);
        // A user definition of the name active at the use wins, mirroring
        // the evaluator's coordinate map. It seeds "camera" (the render
        // camera) and "world" (the identity) after the Options list, so a
        // definition of those names inside Options never survives to a use.
        let binding = self.use_binding.get(&reset).copied().filter(|&define| {
            !matches!(name, "camera" | "world") || !self.initial_options.contains(&define)
        });
        if let Some(define) = binding {
            let defined = self
                .snapshot
                .nodes
                .get(&define)
                .cloned()
                .expect("the coordinate definition state is evaluated");
            let value = Box::new((defined.start, defined.end));
            // Where the definition's group can live: an Options definition
            // becomes a group directly under the World, which a use
            // inside an object definition cannot reach; a tree definition's
            // group stays in its own tree.
            let define_in_options = self.initial_options.contains(&define);
            // An earlier use already replaced the definition with its group.
            let define_place = self.groups.get(&define).copied().unwrap_or(define);
            let same_tree = if define_in_options {
                use_tree == self.world
            } else {
                self.tree_of_node(define_place) == use_tree
            };
            if !same_tree {
                return Ok(ResolvedKind::Coordinate {
                    mode: CoordinateMode::Bake,
                    value,
                    diagnostic: Some(format!(
                        "coordinate system {name} is used outside its defining tree; \
                         its transform is baked into a matrix"
                    )),
                });
            }
            // under the defining group F, an inherited graphics state
            // the use's position does not carry cannot be written back. An
            // Options definition sits at the fresh World entry, where
            // nothing inherits.
            let inexpressible = !define_in_options
                && ((before.material == MaterialBinding::Default
                    && defined.material != MaterialBinding::Default)
                    || (before.area_light.is_none() && defined.area_light.is_some()));
            if inexpressible {
                return Ok(ResolvedKind::Coordinate {
                    mode: CoordinateMode::Bake,
                    value,
                    diagnostic: Some(format!(
                        "coordinate system {name} inherits a material or area light \
                         that the moved part does not carry; \
                         its transform is baked into a matrix"
                    )),
                });
            }
            let group = self.coordinate_group(define)?;
            return Ok(ResolvedKind::Coordinate {
                mode: CoordinateMode::Group { group },
                value,
                diagnostic: None,
            });
        }
        if name == "world" {
            // The evaluator seeds the world coordinate system fresh before
            // the World stream as the identity, so an unbound use behaves
            // exactly like `Identity`.
            return Ok(ResolvedKind::Coordinate {
                mode: CoordinateMode::Bake,
                value: Box::new((Mat4::identity(), Mat4::identity())),
                diagnostic: None,
            });
        }
        if name == "camera" {
            let camera = self.reference_camera()?;
            let state = self
                .snapshot
                .nodes
                .get(&camera)
                .cloned()
                .expect("the camera state is evaluated");
            let value = Box::new((
                inverse(state.start).ok_or_else(|| invalid("singular camera transform"))?,
                inverse(state.end).ok_or_else(|| invalid("singular camera transform"))?,
            ));
            // the camera attachment needs a World-direct group, which
            // an object definition cannot have; inside a definition the
            // pose is baked into a matrix instead.
            if use_tree == self.world {
                return Ok(ResolvedKind::Coordinate {
                    mode: CoordinateMode::Camera { camera },
                    value,
                    diagnostic: None,
                });
            }
            return Ok(ResolvedKind::Coordinate {
                mode: CoordinateMode::Bake,
                value,
                diagnostic: Some(
                    "the camera coordinate system is used inside an object definition; \
                     its transform is baked into a matrix"
                        .into(),
                ),
            });
        }
        Err(invalid(format!(
            "coordinate system {name} is used without a definition"
        )))
    }

    /// the camera coordinate system follows the importer's last camera.
    /// Evaluation refuses an unbound "camera" use long before this runs, so
    /// the error is defense for editor-built models, not an import outcome.
    fn reference_camera(&self) -> Result<NodeId> {
        let scene = self.document.scene(self.scene).expect("scene exists");
        scene
            .render_camera
            .or_else(|| {
                scene.options.iter().rev().copied().find(|id| {
                    matches!(
                        node(self.document, self.scene, *id).map(|n| &n.data),
                        Some(NodeKind::Entity {
                            entity: EntityKind::Camera,
                            ..
                        })
                    )
                })
            })
            .ok_or_else(|| invalid("a camera coordinate system use needs a camera"))
    }

    /// The group for a used coordinate-system definition: it takes the
    /// definition's place in its tree (so editing the defining group still
    /// moves the group), or sits at the World level when the definition was
    /// an Options directive with its transform written into its head.
    fn coordinate_group(&mut self, define: NodeId) -> Result<NodeId> {
        if let Some(group) = self.groups.get(&define) {
            return Ok(*group);
        }
        let name = match node(self.document, self.scene, define).map(|n| &n.data) {
            Some(NodeKind::State {
                operation: StateOperation::DefineCoordinates(name),
            }) => name.clone(),
            _ => {
                // A "camera" or "world" use never reaches here; bindings
                // always resolve to define nodes.
                return Err(invalid("coordinate definition is missing"));
            }
        };
        let defined = self
            .snapshot
            .nodes
            .get(&define)
            .cloned()
            .expect("the coordinate definition state is evaluated");
        let in_options = self.options().contains(&define);
        let group = Node::new(
            name,
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: Vec::new(),
                owned_object: None,
                parent_camera: None,
            },
        );
        let group_id = group.id;
        // An Options definition's group sits directly under the World;
        // every other one takes its definition's place in that tree.
        let tree = if in_options {
            self.world
        } else {
            self.tree_of_node(define)
        };
        let definition_trees: HashSet<NodeId> = self.definitions.iter().copied().collect();
        register_node(self.document, self.scene, &definition_trees, tree, group);
        let entry = if in_options {
            // an Options definition becomes a World-level group with
            // the definition's transform written into its head, since the
            // World stream starts from the identity.
            let mut kids = children(self.document, self.scene, self.world);
            kids.insert(0, group_id);
            set_children(self.document, self.scene, self.world, kids);
            self.parent.insert(group_id, self.world);
            let world_entry = self
                .snapshot
                .nodes
                .get(&self.world)
                .cloned()
                .unwrap_or_default();
            let head = bridge_nodes(
                &GraphicsState {
                    start: Mat4::identity(),
                    end: Mat4::identity(),
                    active: ActiveSamples::All,
                    ..world_entry.clone()
                },
                (defined.start, defined.end),
                ActiveSamples::All,
            )?;
            let mut slots = Vec::new();
            for op in head {
                let op_id = self.register_child(op, group_id);
                slots.push(op_id);
                self.parent.insert(op_id, group_id);
            }
            if let Some(NodeKind::Scope {
                children: group_slots,
                ..
            }) = node_mut(self.document, self.scene, group_id).map(|n| &mut n.data)
            {
                group_slots.extend(slots);
            }
            GraphicsState {
                start: defined.start,
                end: defined.end,
                active: ActiveSamples::All,
                times: world_entry.times,
                ..world_entry
            }
        } else {
            // Take the definition's place in its own scope, wherever that
            // scope itself sits — edits the defining group, and F
            // must land beside the states the definition saw.
            let scope = self
                .parent
                .get(&define)
                .copied()
                .expect("a tree definition has a parent scope");
            let mut kids = children(self.document, self.scene, scope);
            match kids.iter().position(|id| *id == define) {
                Some(index) => kids[index] = group_id,
                None => kids.push(group_id),
            }
            set_children(self.document, self.scene, scope, kids);
            self.parent.insert(group_id, scope);
            defined
        };
        // The definition node is replaced by the group everywhere: its node
        // record, its scope's children, and its Options list slot.
        if in_options {
            let scene = std::sync::Arc::make_mut(
                self.document
                    .scenes
                    .get_mut(&self.scene)
                    .expect("scene exists"),
            );
            scene.options.retain(|other| *other != define);
        }
        remove_node(self.document, self.scene, define);
        self.parent.remove(&define);
        self.groups.insert(define, group_id);
        self.created_entry.insert(group_id, entry.clone());
        self.group_children_start.insert(group_id, entry);
        Ok(group_id)
    }

    /// The shared move machinery, dispatched by the resolved kind.
    fn resolve(&mut self, reset: NodeId, kind: ResolvedKind, before: GraphicsState) -> Result<()> {
        let scope = self
            .parent
            .get(&reset)
            .copied()
            .expect("the reset is inside a scope");
        let kids = children(self.document, self.scene, scope);
        let index = kids
            .iter()
            .position(|id| *id == reset)
            .expect("the reset is inside the scope");
        let root_scope = self.is_root(scope);
        // The scope itself can absorb the reset when its head carries nothing
        // worth keeping, the kind needs no attachment container, and the root
        // wraps Identity/Replace resets away.
        let whole = !head_blocks(self, scope, index)
            && !(root_scope
                && (kind.needs_container()
                    || matches!(kind, ResolvedKind::Identity | ResolvedKind::Replace)));
        // Where the moved container lands, and the stream state at that gap.
        let (placement, base, bridge_base) = match &kind {
            ResolvedKind::Identity | ResolvedKind::Replace => {
                if root_scope {
                    return Err(invalid("a reset directly under a root must be wrapped"));
                }
                let anchor = self.top_level_ancestor(scope);
                let gap = self.gap_state(anchor);
                (Placement::AfterAnchor { anchor }, gap.clone(), gap)
            }
            ResolvedKind::Coordinate {
                mode: CoordinateMode::Bake,
                ..
            } => {
                if root_scope {
                    let state = if whole {
                        self.root_entry(scope)
                    } else {
                        self.state_before_child(scope, index)
                    };
                    let placement = if whole {
                        Placement::Stay
                    } else {
                        Placement::RootChild { root: scope, index }
                    };
                    (placement, state.clone(), state)
                } else {
                    let anchor = self.top_level_ancestor(scope);
                    let gap = self.gap_state(anchor);
                    let placement = Placement::AfterAnchor { anchor };
                    (placement, gap.clone(), gap)
                }
            }
            ResolvedKind::Coordinate {
                mode: CoordinateMode::Camera { .. },
                value,
                ..
            } => {
                if root_scope {
                    // The camera attachment needs an attribute scope, so the
                    // root's tail becomes one at the reset's own position.
                    let state = self.state_before_child(scope, index);
                    let bridge_base = GraphicsState {
                        start: value.0,
                        end: value.1,
                        ..state.clone()
                    };
                    (
                        Placement::RootChild { root: scope, index },
                        state,
                        bridge_base,
                    )
                } else {
                    let anchor = self.top_level_ancestor(scope);
                    let gap = self.gap_state(anchor);
                    let bridge_base = GraphicsState {
                        start: value.0,
                        end: value.1,
                        ..gap.clone()
                    };
                    (Placement::AfterAnchor { anchor }, gap, bridge_base)
                }
            }
            ResolvedKind::Coordinate {
                mode: CoordinateMode::Group { group, .. },
                ..
            } => {
                let children_start = self
                    .group_children_start
                    .get(group)
                    .cloned()
                    .expect("the group's children state exists");
                // the use's container becomes the defining group's
                // child, wherever the use itself sits.
                let placement = Placement::GroupChild { group: *group };
                let value = match &kind {
                    ResolvedKind::Coordinate { value, .. } => **value,
                    _ => unreachable!(),
                };
                let bridge_base = GraphicsState {
                    start: value.0,
                    end: value.1,
                    ..children_start.clone()
                };
                (placement, children_start, bridge_base)
            }
        };
        if whole {
            self.move_scope_whole(reset, &kind, scope, &before, base, bridge_base, placement)?;
        } else {
            self.move_tail_split(&kind, scope, &before, base, bridge_base, index, placement)?;
        }
        // The writes lifted the reset's position to its targets; the reset
        // itself now behaves as a concat (or disappears).
        self.replace_reset(reset, &kind);
        Ok(())
    }

    /// A root scope's entry: the World entry, or the definition's declared
    /// state.
    fn root_entry(&self, root: NodeId) -> GraphicsState {
        self.snapshot
            .nodes
            .get(&root)
            .cloned()
            .expect("the root's entry state is evaluated")
    }

    /// The stream state at a root's child position: the previous sibling's
    /// exit, or the root's entry.
    fn state_before_child(&self, root: NodeId, index: usize) -> GraphicsState {
        let kids = children(self.document, self.scene, root);
        if index > 0 {
            let previous = kids[index - 1];
            return self
                .created_entry
                .get(&previous)
                .or_else(|| self.snapshot.nodes.get(&previous))
                .cloned()
                .expect("the previous child's state was captured");
        }
        self.root_entry(root)
    }

    /// move S itself, keeping its id, label and owned object, to the
    /// placement's gap, and delete the transform states of its head (the
    /// reset canceled their product, so keeping them would let them apply at
    /// the new position).
    #[allow(clippy::too_many_arguments)]
    fn move_scope_whole(
        &mut self,
        reset: NodeId,
        kind: &ResolvedKind,
        scope: NodeId,
        before: &GraphicsState,
        base: GraphicsState,
        bridge_base: GraphicsState,
        placement: Placement,
    ) -> Result<()> {
        let kids = children(self.document, self.scene, scope);
        let index = kids
            .iter()
            .position(|id| *id == reset)
            .expect("the reset is inside the scope");
        let tail: Vec<NodeId> = kids[index..].to_vec();
        let mut head: Vec<NodeId> = Vec::new();
        for id in &kids[..index] {
            if node(self.document, self.scene, *id)
                .is_some_and(|node| is_transform_state(&node.data))
            {
                remove_node(self.document, self.scene, *id);
                self.parent.remove(id);
            } else {
                head.push(*id);
            }
        }
        // The remaining head states still apply; the simulated state is what
        // the tail inherits beyond the transform writes.
        let head_state = self.simulate_head(&head, base.clone());
        let writes = materialize(self.document, self.scene, before, &head_state)?;
        // The kept head may have switched the selector; the bridge starts there.
        let bridge_from = GraphicsState {
            active: head_state.active,
            ..bridge_base.clone()
        };
        let bridge = self.transform_bridge(&bridge_from, before, kind)?;
        let mut merged = head;
        for write in writes {
            let id = self.register_child(write, scope);
            merged.push(id);
        }
        for op in bridge {
            let id = self.register_child(op, scope);
            merged.push(id);
        }
        merged.extend(tail.iter().copied());
        for id in &merged {
            self.parent.insert(*id, scope);
        }
        set_children(self.document, self.scene, scope, merged);
        self.place(scope, placement);
        if let Some(camera) = kind.parent_camera()
            && let Some(slot) = node_mut(self.document, self.scene, scope)
            && let NodeKind::Scope { parent_camera, .. } = &mut slot.data
        {
            *parent_camera = Some(camera);
        }
        // The scope's entry at its landing — and so the state after it for
        // later resets that anchor here — is the placement's ambient: every
        // scope, camera attachments included, restores it on exit, and the
        // bridge writes inside keep its internal stream identical to the
        // original one.
        self.created_entry.insert(scope, base);
        Ok(())
    }

    /// the tail moves into a new attribute scope N at the
    /// placement's gap, with the inherited state written at N's head.
    #[allow(clippy::too_many_arguments)]
    fn move_tail_split(
        &mut self,
        kind: &ResolvedKind,
        scope: NodeId,
        before: &GraphicsState,
        base: GraphicsState,
        bridge_base: GraphicsState,
        index: usize,
        placement: Placement,
    ) -> Result<()> {
        let kids = children(self.document, self.scene, scope);
        let (head, tail) = kids.split_at(index);
        let (head, tail) = (head.to_vec(), tail.to_vec());
        let label = if self.is_root(scope) {
            // gives the container its scope's label, but the root
            // scopes themselves are not part of the modeled world.
            "Group".to_string()
        } else {
            node(self.document, self.scene, scope)
                .map(|node| node.label.clone())
                .unwrap_or_else(|| "Group".into())
        };
        let owned = match node(self.document, self.scene, scope).map(|node| &node.data) {
            Some(NodeKind::Scope { owned_object, .. }) => *owned_object,
            _ => None,
        };
        // an owned object inside the tail moves with it.
        let owned_moves = owned.is_some_and(|id| tail.contains(&id));
        let writes = materialize(self.document, self.scene, before, &base)?;
        let bridge = self.transform_bridge(&bridge_base, before, kind)?;
        let write_ids = writes
            .into_iter()
            .map(|node| self.register_child(node, scope))
            .collect::<Vec<_>>();
        let bridge_ids = bridge
            .into_iter()
            .map(|node| self.register_child(node, scope))
            .collect::<Vec<_>>();
        let group_children = write_ids
            .iter()
            .copied()
            .chain(bridge_ids.iter().copied())
            .chain(tail.iter().copied())
            .collect::<Vec<_>>();
        // Beyond a moved owned object, a sole object with no scope of its own
        // becomes the new group's owned object, so the hierarchy shows it in
        // the group's place.
        let owned = owned
            .filter(|id| tail.contains(id))
            .or(self.sole_owned_object(&group_children));
        let group = Node::new(
            label,
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: group_children.clone(),
                owned_object: owned,
                parent_camera: kind.parent_camera(),
            },
        );
        let group_id = group.id;
        let tree = match placement {
            Placement::GroupChild { group } => self.tree_of_node(group),
            Placement::RootChild { root, .. } => root,
            _ => self.tree_of_node(scope),
        };
        let definition_trees: HashSet<NodeId> = self.definitions.iter().copied().collect();
        register_node(self.document, self.scene, &definition_trees, tree, group);
        // The tail left the original scope, which keeps its head.
        set_children(self.document, self.scene, scope, head);
        if owned_moves
            && let Some(slot) = node_mut(self.document, self.scene, scope)
            && let NodeKind::Scope { owned_object, .. } = &mut slot.data
        {
            *owned_object = None;
        }
        for id in group_children {
            self.parent.insert(id, group_id);
        }
        // The group's entry at its landing is the placement's ambient, and
        // that is also the state after it for anything anchoring here.
        self.place(group_id, placement);
        self.created_entry.insert(group_id, base);
        Ok(())
    }

    /// Place `id` at the gap assigns: right after the anchor at the
    /// root level, as the last child of a coordinate group,
    /// or at a root-level reset's own position.
    fn place(&mut self, id: NodeId, placement: Placement) {
        match placement {
            Placement::Stay => {}
            Placement::AfterAnchor { anchor } => self.place_after(id, anchor),
            Placement::RootChild { root, index } => {
                // The container takes the reset's own position, absorbing the
                // tail into one attribute scope under the root.
                let mut kids = children(self.document, self.scene, root);
                kids.truncate(index);
                kids.push(id);
                set_children(self.document, self.scene, root, kids);
                self.parent.insert(id, root);
            }
            Placement::GroupChild { group } => {
                if let Some(current) = self.parent.get(&id).copied()
                    && current != group
                {
                    let mut siblings = children(self.document, self.scene, current);
                    siblings.retain(|other| *other != id);
                    set_children(self.document, self.scene, current, siblings);
                }
                let mut kids = children(self.document, self.scene, group);
                kids.push(id);
                set_children(self.document, self.scene, group, kids);
                self.parent.insert(id, group);
            }
        }
    }

    /// The graphics state the remaining head states produce over base.
    fn simulate_head(&self, head: &[NodeId], mut state: GraphicsState) -> GraphicsState {
        for id in head {
            let Some(NodeKind::State { operation }) =
                node(self.document, self.scene, *id).map(|node| &node.data)
            else {
                continue;
            };
            match operation {
                StateOperation::Material(binding) => {
                    state.material = binding.clone();
                    state.material_node = Some(*id);
                }
                StateOperation::Medium { inside, outside } => {
                    state.inside_medium = *inside;
                    state.outside_medium = *outside;
                    state.medium_node = Some(*id);
                }
                StateOperation::ReverseOrientation => {
                    state.reverse_orientation = !state.reverse_orientation;
                }
                StateOperation::ColorSpace(name) => state.color_space = name.clone(),
                StateOperation::ActiveTransform(active) => state.active = *active,
                _ => {}
            }
        }
        state
    }

    ///,: write the samples the reset does not replace, with
    /// explicit `ActiveTransform` selectors, and leave the selector at the
    /// reset's own value.
    fn transform_bridge(
        &self,
        base: &GraphicsState,
        before: &GraphicsState,
        kind: &ResolvedKind,
    ) -> Result<Vec<Node>> {
        bridge_nodes(base, kind.targets(before), before.active)
    }

    /// An in-place replacement for a `Transform [m]` keeps the node and
    /// behave as a concat. Identity and coordinate resets disappear.
    fn replace_reset(&mut self, reset: NodeId, kind: &ResolvedKind) {
        match kind {
            ResolvedKind::Replace => {
                let Some(slot) = node_mut(self.document, self.scene, reset) else {
                    return;
                };
                let matrix = match &slot.data {
                    NodeKind::State {
                        operation: StateOperation::Transform(TransformOperation::Replace(m)),
                    } => *m,
                    _ => return,
                };
                slot.label = "ConcatTransform".into();
                slot.data = NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Concat(matrix)),
                };
            }
            _ => {
                let parent = self
                    .parent
                    .get(&reset)
                    .copied()
                    .expect("the reset has a parent");
                let mut kids = children(self.document, self.scene, parent);
                kids.retain(|id| *id != reset);
                set_children(self.document, self.scene, parent, kids);
                remove_node(self.document, self.scene, reset);
                self.parent.remove(&reset);
            }
        }
    }

    /// The scope that is a direct child of a root and contains `of` (or
    /// `of` itself when it already is one).
    fn top_level_ancestor(&self, of: NodeId) -> NodeId {
        let mut current = of;
        while let Some(parent) = self.parent.get(&current).copied() {
            if self.is_root(parent) {
                return current;
            }
            current = parent;
        }
        current
    }

    fn is_root(&self, id: NodeId) -> bool {
        id == self.world || self.definitions.contains(&id)
    }

    fn tree_of_node(&self, id: NodeId) -> NodeId {
        let mut current = id;
        while !self.is_root(current) {
            current = self
                .parent
                .get(&current)
                .copied()
                .or_else(|| self.definitions.contains(&current).then_some(current))
                .expect("every node belongs to a tree");
        }
        current
    }

    /// The stream state at the gap right after the anchor. Every existing
    /// scope restores its entry state, and the groups this pass creates are
    /// Attribute scopes, so the state after any of them is the state at the
    /// gap they were placed into.
    fn gap_state(&self, anchor: NodeId) -> GraphicsState {
        self.created_entry
            .get(&anchor)
            .or_else(|| self.snapshot.nodes.get(&anchor))
            .cloned()
            .expect("the anchor's stream state was captured")
    }

    fn register_child(&mut self, node: Node, parent: NodeId) -> NodeId {
        let tree = self.tree_of_node(parent);
        let definition_trees: HashSet<NodeId> = self.definitions.iter().copied().collect();
        register_node(self.document, self.scene, &definition_trees, tree, node)
    }

    /// Place `id` at the gap right after the anchor in its root's children.
    fn place_after(&mut self, id: NodeId, anchor: NodeId) {
        let target = self.last_after.get(&anchor).copied().unwrap_or(anchor);
        if target == id {
            // A whole-moved scope can be its own anchor: it already sits at
            // the gap, so only the bookkeeping runs.
            self.last_after.insert(anchor, id);
            self.parent.insert(id, self.tree_of_node(anchor));
            return;
        }
        if let Some(current) = self.parent.get(&id).copied()
            && !self.is_root(current)
        {
            let mut siblings = children(self.document, self.scene, current);
            siblings.retain(|other| *other != id);
            set_children(self.document, self.scene, current, siblings);
        }
        let tree = self.tree_of_node(anchor);
        let mut siblings = children(self.document, self.scene, tree);
        let position = siblings
            .iter()
            .position(|other| *other == target)
            .map(|position| position + 1)
            .unwrap_or(siblings.len());
        siblings.retain(|other| *other != id);
        let position = position.min(siblings.len());
        siblings.insert(position, id);
        set_children(self.document, self.scene, tree, siblings);
        self.parent.insert(id, tree);
        self.last_after.insert(anchor, id);
    }

    fn sole_owned_object(&self, group_children: &[NodeId]) -> Option<NodeId> {
        let mut objects = group_children
            .iter()
            .filter(|id| {
                node(self.document, self.scene, **id)
                    .is_some_and(|node| is_owned_visible_object(&node.data))
            })
            .copied();
        let first = objects.next()?;
        if objects.next().is_some() {
            return None;
        }
        if group_children.iter().any(|id| {
            matches!(
                node(self.document, self.scene, *id).map(|node| &node.data),
                Some(NodeKind::Scope { .. })
            )
        }) {
            return None;
        }
        Some(first)
    }
}

impl ResolvedKind {
    fn diagnostic_message(&self) -> Option<String> {
        match self {
            ResolvedKind::Coordinate { diagnostic, .. } => diagnostic.clone(),
            _ => None,
        }
    }
}

/// Write the samples a reset does not replace, using explicit
/// `ActiveTransform` selectors, and leave the selector at the reset's own
/// value.,
fn bridge_nodes(
    base: &GraphicsState,
    targets: (Mat4, Mat4),
    target_active: ActiveSamples,
) -> Result<Vec<Node>> {
    let op = |base: Mat4, target: Mat4| -> Result<Option<Mat4>> {
        if target == base {
            return Ok(None);
        }
        if base == Mat4::identity() {
            return Ok(Some(target));
        }
        let inverted =
            inverse(base).ok_or_else(|| invalid("a singular transform cannot be rebased"))?;
        Ok(Some(inverted.compose(target)))
    };
    let start_op = op(base.start, targets.0)?;
    let end_op = op(base.end, targets.1)?;
    let mut nodes = Vec::new();
    let mut current = base.active;
    match (start_op, end_op) {
        (Some(start), Some(end)) if start == end && current == ActiveSamples::All => {
            // One shared write covers both samples.
            nodes.push(concat(start));
        }
        _ => {
            for (sample, matrix) in [
                (ActiveSamples::Start, start_op),
                (ActiveSamples::End, end_op),
            ] {
                if let Some(matrix) = matrix {
                    nodes.push(active_transform(sample));
                    nodes.push(concat(matrix));
                    current = sample;
                }
            }
        }
    }
    if current != target_active {
        nodes.push(active_transform(target_active));
    }
    Ok(nodes)
}

fn collect_used_coordinate_name(kind: &NodeKind, names: &mut BTreeSet<String>) {
    if let NodeKind::State {
        operation: StateOperation::UseCoordinates(name),
    } = kind
    {
        names.insert(name.clone());
    }
}

fn define_coordinate_name(
    id: NodeId,
    kind: &NodeKind,
    coordinate_defs: &mut BTreeMap<String, NodeId>,
) {
    if let NodeKind::State {
        operation: StateOperation::DefineCoordinates(name),
    } = kind
    {
        coordinate_defs.insert(name.clone(), id);
    }
}

/// Every node of the World tree and the definition bodies, in preorder.
fn trees(
    document: &ProjectDocument,
    scene: SceneId,
    world: NodeId,
    definitions: &[NodeId],
) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack: Vec<NodeId> = std::iter::once(world)
        .chain(definitions.iter().copied())
        .collect();
    while let Some(id) = stack.pop() {
        out.push(id);
        stack.extend(children(document, scene, id).into_iter().rev());
    }
    out
}

/// whether a reset's head keeps more than states: an object or scope
/// of its own, or a coordinate-system definition that is used later.
fn head_blocks(pass: &Pass, scope: NodeId, reset_index: usize) -> bool {
    children(pass.document, pass.scene, scope)[..reset_index]
        .iter()
        .any(
            |id| match node(pass.document, pass.scene, *id).map(|node| &node.data) {
                Some(NodeKind::Scope { .. }) => true,
                Some(NodeKind::Instance { .. }) => true,
                Some(NodeKind::Entity { .. }) => true,
                Some(NodeKind::State {
                    operation: StateOperation::DefineCoordinates(name),
                }) => pass.used_names.contains(name),
                _ => false,
            },
        )
}

fn affected_samples(active: ActiveSamples) -> (bool, bool) {
    match active {
        ActiveSamples::All => (true, true),
        ActiveSamples::Start => (true, false),
        ActiveSamples::End => (false, true),
    }
}

fn is_owned_visible_object(kind: &NodeKind) -> bool {
    matches!(
        kind,
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

fn active_transform(sample: ActiveSamples) -> Node {
    Node::new(
        "ActiveTransform",
        NodeKind::State {
            operation: StateOperation::ActiveTransform(sample),
        },
    )
}

fn concat(matrix: Mat4) -> Node {
    Node::new(
        "ConcatTransform",
        NodeKind::State {
            operation: StateOperation::Transform(TransformOperation::Concat(matrix.0)),
        },
    )
}
