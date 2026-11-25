//! Evaluation of the canonical ordered model. No PBRT syntax or GPU types.
use crate::{
    math::{Mat4, inverse},
    *,
};
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq)]
pub struct GraphicsState {
    pub start: Mat4,
    pub end: Mat4,
    pub active: ActiveSamples,
    pub times: [f64; 2],
    pub material: MaterialBinding,
    pub inside_medium: Option<ResourceId>,
    pub outside_medium: Option<ResourceId>,
    pub area_light: Option<NodeId>,
    /// State nodes that set the current material / medium binding, if any.
    pub material_node: Option<NodeId>,
    pub medium_node: Option<NodeId>,
    pub reverse_orientation: bool,
    pub color_space: String,
}
impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            start: Mat4::identity(),
            end: Mat4::identity(),
            active: ActiveSamples::All,
            times: [0., 1.],
            material: MaterialBinding::Default,
            inside_medium: None,
            outside_medium: None,
            area_light: None,
            material_node: None,
            medium_node: None,
            reverse_orientation: false,
            color_space: "srgb".into(),
        }
    }
}
impl GraphicsState {
    pub fn apply(&mut self, operation: &TransformOperation) -> Result<()> {
        let matrix = match operation {
            TransformOperation::Identity => Mat4::identity(),
            TransformOperation::Translate(v) => Mat4::translation(v[0], v[1], v[2]),
            TransformOperation::Scale(v) => Mat4::scale(v[0], v[1], v[2]),
            TransformOperation::LookAt(v) => {
                crate::math::look_at(v).ok_or_else(|| invalid("degenerate LookAt"))?
            }
            TransformOperation::Rotate { degrees, axis } => {
                Mat4::rotation(*degrees, axis[0], axis[1], axis[2])
            }
            TransformOperation::Replace(m) | TransformOperation::Concat(m) => Mat4(*m),
        };
        let replace = matches!(
            operation,
            TransformOperation::Identity | TransformOperation::Replace(_)
        );
        self.set(matrix, matrix, replace)?;
        Ok(())
    }
    pub fn set(&mut self, start: Mat4, end: Mat4, replace: bool) -> Result<()> {
        if self.active != ActiveSamples::End {
            self.start = if replace {
                start
            } else {
                self.start.compose(start)
            };
        }
        if self.active != ActiveSamples::Start {
            self.end = if replace { end } else { self.end.compose(end) };
        }
        if self
            .start
            .0
            .iter()
            .chain(&self.end.0)
            .any(|v| !v.is_finite())
        {
            return Err(invalid("evaluated transform overflow"));
        }
        Ok(())
    }

    /// State captured where a resource was declared. Material declarations do
    /// not capture state (r4 applies them at the point of use); their
    /// parameters use the default color space.
    pub fn at_declaration(resource: &Resource) -> GraphicsState {
        let mut state = GraphicsState::default();
        let Some(declared) = &resource.declared_in else {
            return state;
        };
        if let Some(transform) = &declared.transform {
            state.start = Mat4(transform.start);
            state.end = Mat4(transform.end);
            state.active = transform.active;
            state.times = transform.times;
        }
        if let Some(graphics) = &declared.graphics {
            state.material = graphics.material.clone();
            state.inside_medium = graphics.inside_medium;
            state.outside_medium = graphics.outside_medium;
            state.reverse_orientation = graphics.reverse_orientation;
            state.color_space = graphics.color_space.clone();
        }
        state
    }
}

#[derive(Clone, Debug)]
pub struct EvaluatedScene {
    /// State after each declaration/operation; scopes store their entry state.
    pub nodes: BTreeMap<NodeId, GraphicsState>,
    /// Transform state immediately before each transform or coordinate reset.
    pub transform_before: BTreeMap<NodeId, GraphicsState>,
    /// State captured at each resource's declaration.
    pub resources: BTreeMap<ResourceId, GraphicsState>,
}

pub fn evaluate(view: &SceneView) -> Result<EvaluatedScene> {
    view.document.validate()?;
    let mut evaluator = Evaluator {
        view: *view,
        result: EvaluatedScene {
            nodes: BTreeMap::new(),
            transform_before: BTreeMap::new(),
            resources: BTreeMap::new(),
        },
        coordinates: BTreeMap::new(),
    };
    let mut state = GraphicsState::default();
    for id in &view.scene.options {
        if let NodeKind::State {
            operation: StateOperation::TransformTimes(times),
        } = evaluator.node_kind(*id)?
        {
            state.times = *times;
        }
    }
    for id in &view.scene.options {
        evaluator.node(*id, &mut state)?;
    }
    if let Some(camera) = view.scene.render_camera {
        let state = evaluator
            .result
            .nodes
            .get(&camera)
            .ok_or_else(|| invalid("render camera is missing from scene options"))?;
        // a camera with a pose decides the coordinate system itself;
        // without one (saved projects) the Options state's inverse applies.
        let (start, end) = match camera_pose(view, camera) {
            Some(pose) => (Mat4(pose.start), Mat4(pose.end)),
            None => (
                inverse(state.start).ok_or_else(|| invalid("singular camera transform"))?,
                inverse(state.end).ok_or_else(|| invalid("singular camera transform"))?,
            ),
        };
        evaluator.coordinates.insert("camera".into(), (start, end));
    } else {
        evaluator.coordinates.remove("camera");
    }
    state.start = Mat4::identity();
    state.end = Mat4::identity();
    state.active = ActiveSamples::All;
    evaluator
        .coordinates
        .insert("world".into(), (state.start, state.end));
    evaluator.node(view.scene.world, &mut state)?;
    // Object definitions are not part of the world tree any more: evaluate
    // each body from the state captured at its declaration so resource
    // parameters and preview emission see the right color space.
    for resource in view.document.resources.values() {
        let declared = GraphicsState::at_declaration(resource);
        evaluator
            .result
            .resources
            .insert(resource.id, declared.clone());
        if let Some(scope) = resource.definition_scope {
            let mut state = declared;
            evaluator.node(scope, &mut state)?;
        }
    }
    Ok(evaluator.result)
}
struct Evaluator<'a> {
    view: SceneView<'a>,
    result: EvaluatedScene,
    coordinates: BTreeMap<String, (Mat4, Mat4)>,
}
impl Evaluator<'_> {
    fn node_kind(&self, id: NodeId) -> Result<&NodeKind> {
        self.view
            .node(id)
            .map(|node| &node.data)
            .ok_or_else(|| invalid("missing node"))
    }

    fn node(&mut self, id: NodeId, state: &mut GraphicsState) -> Result<()> {
        let view = self.view;
        match view
            .node(id)
            .map(|node| &node.data)
            .ok_or_else(|| invalid("missing node"))?
        {
            NodeKind::Scope {
                scope,
                children,
                parent_camera,
                ..
            } => {
                // a camera-attached group replaces the transform on
                // entry, on both samples — and, like every attribute scope,
                // restores what came before it on exit, so later siblings
                // keep their ambient. The camera lives in Options, so the
                // group references it instead of parenting.
                let previous = state.clone();
                if let Some(camera) = parent_camera {
                    let (start, end) = self.referenced_camera_to_world(*camera)?;
                    state.start = start;
                    state.end = end;
                }
                self.result.nodes.insert(id, state.clone());
                for child in children {
                    self.node(*child, state)?;
                }
                match scope {
                    ScopeKind::Attribute | ScopeKind::ObjectDefinition => *state = previous,
                    ScopeKind::Transform => {
                        state.start = previous.start;
                        state.end = previous.end;
                        state.active = previous.active;
                    }
                    ScopeKind::World => {}
                }
                return Ok(());
            }
            NodeKind::State { operation } => match operation {
                StateOperation::Transform(op) => {
                    self.result.transform_before.insert(id, state.clone());
                    state.apply(op)?;
                }
                StateOperation::ActiveTransform(active) => state.active = *active,
                StateOperation::TransformTimes(_) => {}
                StateOperation::Material(binding) => {
                    state.material = binding.clone();
                    state.material_node = Some(id);
                }
                StateOperation::Medium { inside, outside } => {
                    state.inside_medium = *inside;
                    state.outside_medium = *outside;
                    state.medium_node = Some(id);
                }
                StateOperation::ReverseOrientation => {
                    state.reverse_orientation = !state.reverse_orientation
                }
                StateOperation::ColorSpace(name) => state.color_space = name.clone(),
                StateOperation::DefineCoordinates(name) => {
                    self.coordinates
                        .insert(name.clone(), (state.start, state.end));
                }
                StateOperation::UseCoordinates(name) => {
                    self.result.transform_before.insert(id, state.clone());
                    let (start, end) = self
                        .coordinates
                        .get(name)
                        .ok_or_else(|| invalid("unknown coordinate system"))?;
                    state.set(*start, *end, true)?;
                }
            },
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                implementation,
                ..
            } => {
                state.area_light = (!implementation.is_empty()).then_some(id);
            }
            NodeKind::Entity {
                entity: EntityKind::Camera,
                ..
            } => {
                let (start, end) = self.camera_to_world(id, state)?;
                self.coordinates.insert("camera".into(), (start, end));
            }
            _ => {}
        }
        self.result.nodes.insert(id, state.clone());
        Ok(())
    }

    /// The camera's camera-to-world matrices at the two samples: a pose if
    /// the camera carries one, otherwise the inverse of the transform
    /// current at its declaration.
    fn camera_to_world(&self, camera: NodeId, state: &GraphicsState) -> Result<(Mat4, Mat4)> {
        if let Some(pose) = camera_pose(&self.view, camera) {
            return Ok((Mat4(pose.start), Mat4(pose.end)));
        }
        let start = inverse(state.start).ok_or_else(|| invalid("singular camera transform"))?;
        let end = inverse(state.end).ok_or_else(|| invalid("singular camera transform"))?;
        Ok((start, end))
    }

    /// A referenced camera's camera-to-world matrices: its pose, or the
    /// inverse of its evaluated Options state when it carries no pose.
    fn referenced_camera_to_world(&self, camera: NodeId) -> Result<(Mat4, Mat4)> {
        if let Some(pose) = camera_pose(&self.view, camera) {
            return Ok((Mat4(pose.start), Mat4(pose.end)));
        }
        let state = self
            .result
            .nodes
            .get(&camera)
            .ok_or_else(|| invalid("camera is missing from scene options"))?;
        let start = inverse(state.start).ok_or_else(|| invalid("singular camera transform"))?;
        let end = inverse(state.end).ok_or_else(|| invalid("singular camera transform"))?;
        Ok((start, end))
    }
}

/// A camera node's pose, if it carries one.
fn camera_pose(view: &SceneView, camera: NodeId) -> Option<CameraPose> {
    view.node(camera).and_then(|node| match &node.data {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => Some(*pose),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const FLAT: [f64; 16] = [
        1., 0., 0., 1., 0., 1., 0., 2., 0., 0., 1., 3., 0., 0., 0., 1.,
    ];

    fn camera(pose: Option<CameraPose>) -> Node {
        Node::new(
            "Camera",
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation: "perspective".into(),
                parameters: Vec::new(),
                pose,
            },
        )
    }

    fn lamp() -> Node {
        Node::new(
            "Lamp",
            NodeKind::Entity {
                entity: EntityKind::Light,
                implementation: "point".into(),
                parameters: Vec::new(),
                pose: None,
            },
        )
    }

    /// A world-level attribute group, optionally attached to a camera.
    fn group(child: Node, parent_camera: Option<NodeId>) -> Node {
        Node::new(
            "Group",
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: vec![child.id],
                owned_object: None,
                parent_camera,
            },
        )
    }

    fn document(camera: Node, group: Node, child: Node) -> ProjectDocument {
        let mut document = ProjectDocument::new("Pose");
        let scene_id = document.default_scene.unwrap();
        let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
        scene.options.push(camera.id);
        scene.render_camera = Some(camera.id);
        let world = scene.world;
        let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
        else {
            unreachable!()
        };
        children.push(group.id);
        for node in [camera, group, child] {
            scene.nodes.insert(node.id, node);
        }
        document
    }

    fn world_group_of(document: &ProjectDocument) -> NodeId {
        let view = document.default_scene_view().unwrap();
        match &view.scene.nodes[&view.scene.world].data {
            NodeKind::Scope { children, .. } => children[0],
            _ => unreachable!(),
        }
    }

    #[test]
    fn a_pose_camera_decides_the_camera_coordinate_system() {
        // The options resolve to translate ∘ camera; with a pose, the camera
        // coordinate system is the pose itself, not the inverse of that
        // composed stream.
        let pose = CameraPose {
            start: FLAT,
            end: FLAT,
        };
        let camera = camera(Some(pose));
        let camera_id = camera.id;
        let translate = Node::new(
            "Translate",
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate([9., 0., 0.])),
            },
        );
        let translate_id = translate.id;
        let use_node = Node::new(
            "CoordSysTransform",
            NodeKind::State {
                operation: StateOperation::UseCoordinates("camera".into()),
            },
        );
        let use_id = use_node.id;
        let group = Node::new(
            "Group",
            NodeKind::Scope {
                scope: ScopeKind::Attribute,
                children: vec![use_id],
                owned_object: None,
                parent_camera: None,
            },
        );
        let group_id = group.id;
        let mut document = ProjectDocument::new("Pose");
        let scene_id = document.default_scene.unwrap();
        let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
        scene.options = vec![translate_id, camera_id];
        scene.render_camera = Some(camera_id);
        let world = scene.world;
        let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
        else {
            unreachable!()
        };
        children.push(group_id);
        for node in [camera, translate, use_node, group] {
            scene.nodes.insert(node.id, node);
        }
        let view = document.default_scene_view().unwrap();
        let evaluated = evaluate(&view).unwrap();
        let state = &evaluated.nodes[&use_id];
        assert_eq!(
            state.start.0, FLAT,
            "the pose defines the coordinate system"
        );
        assert_eq!(state.end.0, FLAT);
    }

    #[test]
    fn a_parent_camera_group_enters_at_the_camera_pose_on_both_samples() {
        let pose = CameraPose {
            start: [
                1., 0., 0., 1., 0., 1., 0., 0., 0., 0., 1., 5., 0., 0., 0., 1.,
            ],
            end: [
                1., 0., 0., 2., 0., 1., 0., 0., 0., 0., 1., 7., 0., 0., 0., 1.,
            ],
        };
        let camera = camera(Some(pose));
        let group_child = lamp();
        let group = group(group_child.clone(), Some(camera.id));
        let group_child_id = group_child.id;
        let document = document(camera, group, group_child);
        let view = document.default_scene_view().unwrap();
        let evaluated = evaluate(&view).unwrap();
        let group_id = world_group_of(&document);
        let state = &evaluated.nodes[&group_id];
        assert_eq!(state.start.0, pose.start);
        assert_eq!(state.end.0, pose.end);
        assert_eq!(evaluated.nodes[&group_child_id].start.0, pose.start);
        assert_eq!(evaluated.nodes[&group_child_id].end.0, pose.end);
    }

    #[test]
    fn a_parent_camera_group_restores_the_ambient_on_exit() {
        // The attachment replaces the transform on entry; on exit the
        // group is an ordinary attribute scope, so later world siblings keep
        // the ambient they had — not the pose.
        let pose = CameraPose {
            start: [
                1., 0., 0., 1., 0., 1., 0., 0., 0., 0., 1., 5., 0., 0., 0., 1.,
            ],
            end: [
                1., 0., 0., 2., 0., 1., 0., 0., 0., 0., 1., 7., 0., 0., 0., 1.,
            ],
        };
        let camera = camera(Some(pose));
        let group_child = lamp();
        let group = group(group_child.clone(), Some(camera.id));
        let shape = lamp();
        let shape_id = shape.id;
        let document = {
            let mut built = document(camera, group, group_child);
            let scene_id = built.default_scene.unwrap();
            let scene = std::sync::Arc::make_mut(built.scenes.get_mut(&scene_id).unwrap());
            let world = scene.world;
            scene.nodes.insert(shape_id, shape);
            if let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
            {
                children.push(shape_id);
            }
            built
        };
        let view = document.default_scene_view().unwrap();
        let evaluated = evaluate(&view).unwrap();
        let identity = Mat4::identity().0;
        assert_eq!(
            evaluated.nodes[&shape_id].start.0, identity,
            "the world sibling after the group keeps the ambient"
        );
        assert_eq!(evaluated.nodes[&shape_id].end.0, identity);
    }

    #[test]
    fn a_pose_less_reference_falls_back_to_the_options_state() {
        // Saved projects without poses keep working: the group attaches to
        // the inverse of the camera's evaluated Options transform.
        let camera = camera(None);
        let camera_id = camera.id;
        let lamp_node = lamp();
        let group = group(lamp_node.clone(), Some(camera_id));
        let mut document = document(camera, group, lamp_node);
        // A translation before the camera in the Options stream.
        let look_at = Node::new(
            "Translate",
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate([2., 0., 0.])),
            },
        );
        let scene_id = document.default_scene.unwrap();
        let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
        let index = scene
            .options
            .iter()
            .position(|id| *id == camera_id)
            .unwrap();
        scene.options.insert(index, look_at.id);
        scene.nodes.insert(look_at.id, look_at);
        let view = document.default_scene_view().unwrap();
        let evaluated = evaluate(&view).unwrap();
        let group_id = world_group_of(&document);
        let state = &evaluated.nodes[&group_id];
        let camera_state = &evaluated.nodes[&camera_id];
        assert_eq!(state.start, inverse(camera_state.start).unwrap());
        assert_eq!(state.end, inverse(camera_state.end).unwrap());
    }
}
