mod common;
use common::*;
use project::presets::PresetView;
use project::*;

fn create(session: &mut ProjectSession, preset: ResourceId, parent: NodeId) -> ChangeSet {
    create_with(session, preset, parent, None)
}

fn create_with(
    session: &mut ProjectSession,
    preset: ResourceId,
    parent: NodeId,
    view: Option<PresetView>,
) -> ChangeSet {
    session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset,
                parent,
                after: None,
                definition: None,
                view,
            }],
        )
        .unwrap()
}

/// A scene whose coordinate system is set and the id of the lone camera
/// node a `camera.perspective` preset created.
fn create_camera(
    up: Axis,
    handedness: Handedness,
    view: Option<PresetView>,
) -> (ProjectSession, CameraPose) {
    let mut document = ProjectDocument::new("Camera preset");
    scene_mut(&mut document).coordinate_system = CoordinateSystem::new(up, handedness);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = create_with(&mut session, presets::CAMERA_PERSPECTIVE.id, world, view);
    let camera = change.created[0];
    let pose = match &scene(session.document()).nodes[&camera].data {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => *pose,
        _ => panic!("the camera preset creates a posed camera"),
    };
    (session, pose)
}

/// Shape body of a preset created in an empty scene.
fn body(preset: &presets::BuiltinPreset) -> Vec<Parameter> {
    let document = ProjectDocument::new("Preset");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, preset.id, world);
    change
        .created
        .iter()
        .find_map(|id| match &scene(session.document()).nodes[id].data {
            NodeKind::Entity {
                entity: EntityKind::Shape,
                parameters,
                ..
            } => Some(parameters.clone()),
            _ => None,
        })
        .expect("preset creates a shape")
}

fn points(parameters: &[Parameter]) -> Vec<[f64; 3]> {
    let values = parameters
        .iter()
        .find(|parameter| parameter.name == "P")
        .map(|parameter| match &parameter.values {
            ParameterValues::Floats(values) => values
                .iter()
                .map(|value| match value {
                    FloatValue::Finite(value) => *value,
                    other => panic!("non-finite point {other:?}"),
                })
                .collect::<Vec<_>>(),
            other => panic!("P is not floats: {other:?}"),
        })
        .expect("shape has P");
    values.chunks(3).map(|p| [p[0], p[1], p[2]]).collect()
}

fn sub(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}
fn dot(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

#[test]
fn builtin_preset_ids_are_fixed() {
    let ids = presets::BUILTIN_PRESETS
        .iter()
        .map(|preset| (preset.id.0.to_string(), preset.name))
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            ("19500901-220e-4577-a08a-6fa4173fa347", "group.empty"),
            ("266b209c-bbbf-4687-a697-483f7126d8ca", "camera.perspective"),
            ("209d7c20-ca6d-4965-a471-405c97e13662", "shape.sphere"),
            ("c5ea4a95-1821-4fd7-b569-acf6f0c75910", "shape.disk"),
            ("d90e15f3-5996-4368-ac2e-9945e6856e23", "shape.cylinder"),
            ("767a280d-35eb-49bb-924a-b7dbb03f7568", "shape.plane"),
            ("f7e45351-5c8b-4756-a41b-4c1b9f782743", "shape.floor"),
            ("7ad28556-48d6-42d7-8bd9-2823cace6ad1", "shape.cube"),
            ("80bf7ed4-21df-48f8-8715-bb1285ecd7b7", "light.point"),
            ("023850b2-919c-4ea9-9119-1739c6955e55", "light.spot"),
            ("698b29cc-3eb4-493f-b922-0fe61c98d988", "light.distant"),
            ("7b826e74-156d-4042-987d-4ec688620d13", "light.infinite"),
            ("ee016a71-134f-4eeb-8d71-ef1132f3125a", "area.quad"),
            ("53179d38-2337-474d-a861-fe1921021e47", "area.sphere"),
            ("a62d8a1c-f165-40f7-adef-c5d3bf027006", "instance.object"),
        ]
        .map(|(id, name)| (id.to_owned(), name))
    );
}

/// Creates `preset` in a scene with the given up axis and returns the
/// world matrix the evaluation gives the created shape.
fn oriented_world(preset: presets::BuiltinPreset, up: Axis) -> math::Mat4 {
    let mut document = ProjectDocument::new("Oriented");
    scene_mut(&mut document).coordinate_system = CoordinateSystem::new(up, Handedness::Left);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, preset.id, world);
    let shape = change
        .created
        .iter()
        .find(|id| {
            matches!(
                &scene(session.document()).nodes[id].data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            )
        })
        .expect("the preset creates a shape");
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    evaluated.nodes[shape].start
}

#[test]
fn plane_and_floor_normals_follow_the_scene_up_axis() {
    for up in Axis::ALL {
        let up_vector = up.vector();
        if up == Axis::PosY {
            continue;
        }
        for preset in [
            presets::SHAPE_PLANE,
            presets::SHAPE_FLOOR,
            presets::AREA_QUAD,
        ] {
            let world = oriented_world(preset, up);
            let normal = world.transform_point([0., 1., 0.]);
            assert!(
                (normal[0] - up_vector[0]).abs() < 1e-9
                    && (normal[1] - up_vector[1]).abs() < 1e-9
                    && (normal[2] - up_vector[2]).abs() < 1e-9,
                "{} with {up:?} faces {normal:?}",
                preset.name
            );
        }
    }
    // +Y-up scenes keep the unrotated patch, unchanged behavior.
    let plain = oriented_world(presets::SHAPE_PLANE, Axis::PosY);
    let normal = plain.transform_point([0., 1., 0.]);
    assert!((normal[0]).abs() < 1e-9 && (normal[1] - 1.).abs() < 1e-9 && (normal[2]).abs() < 1e-9);
}

#[test]
fn disks_and_cylinders_stand_upright_in_every_scene() {
    for up in Axis::ALL {
        let up_vector = up.vector();
        for preset in [presets::SHAPE_DISK, presets::SHAPE_CYLINDER] {
            let world = oriented_world(preset, up);
            let axis = world.transform_point([0., 0., 1.]);
            assert!(
                (axis[0] - up_vector[0]).abs() < 1e-9
                    && (axis[1] - up_vector[1]).abs() < 1e-9
                    && (axis[2] - up_vector[2]).abs() < 1e-9,
                "{} with {up:?} puts its axis at {axis:?}",
                preset.name
            );
        }
    }
}

#[test]
fn spot_and_distant_lights_point_down() {
    for up in Axis::ALL {
        let down = up.vector().map(|value| -value);
        let mut document = ProjectDocument::new("Lights");
        scene_mut(&mut document).coordinate_system = CoordinateSystem::new(up, Handedness::Left);
        let world = scene(&document).world;
        let mut session = ProjectSession::new(document).unwrap();
        for preset in [presets::LIGHT_SPOT, presets::LIGHT_DISTANT] {
            let change = create(&mut session, preset.id, world);
            let light = change
                .created
                .iter()
                .find(|id| {
                    matches!(
                        &scene(session.document()).nodes[id].data,
                        NodeKind::Entity {
                            entity: EntityKind::Light,
                            ..
                        }
                    )
                })
                .expect("the preset creates a light");
            let NodeKind::Entity { parameters, .. } = &scene(session.document()).nodes[light].data
            else {
                unreachable!()
            };
            let value = |name: &str| {
                parameters
                    .iter()
                    .find(|parameter| parameter.name == name)
                    .and_then(|parameter| match &parameter.values {
                        ParameterValues::Floats(values) => Some(
                            values
                                .iter()
                                .map(|value| match value {
                                    FloatValue::Finite(value) => *value,
                                    other => panic!("{other:?}"),
                                })
                                .collect::<Vec<_>>(),
                        ),
                        _ => None,
                    })
                    .unwrap_or_else(|| panic!("{} has no {name}", preset.name))
            };
            assert_eq!(value("from"), vec![0., 0., 0.], "{} from", preset.name);
            let to = value("to");
            assert!(
                (to[0] - down[0]).abs() < 1e-9
                    && (to[1] - down[1]).abs() < 1e-9
                    && (to[2] - down[2]).abs() < 1e-9,
                "{} with {up:?} points at {to:?}",
                preset.name
            );
        }
    }
}

#[test]
fn the_floor_preset_spans_ten_units() {
    let corners = points(&body(&presets::SHAPE_FLOOR));
    assert_eq!(
        corners,
        [[-5., 0., -5.], [-5., 0., 5.], [5., 0., -5.], [5., 0., 5.]],
        "floor corners are p00, p10, p01, p11, ten by ten and centered"
    );
    for (u, v) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
        let dpdu = (0..3)
            .map(|i| {
                (1. - v) * (corners[1][i] - corners[0][i]) + v * (corners[3][i] - corners[2][i])
            })
            .collect::<Vec<_>>();
        let dpdv = (0..3)
            .map(|i| {
                (1. - u) * (corners[2][i] - corners[0][i]) + u * (corners[3][i] - corners[1][i])
            })
            .collect::<Vec<_>>();
        let normal = cross([dpdu[0], dpdu[1], dpdu[2]], [dpdv[0], dpdv[1], dpdv[2]]);
        assert!(
            normal[1] > 0. && normal[0].abs() < 1e-12 && normal[2].abs() < 1e-12,
            "the pbrt normal faces +Y for either handedness: {normal:?}"
        );
    }
}

#[test]
fn the_camera_preset_estimates_the_scene_coordinate_system_back() {
    for up in Axis::ALL {
        for handedness in Handedness::ALL {
            let (_session, pose) = create_camera(up, handedness, None);
            assert_eq!(
                CoordinateSystem::estimate(&pose.start),
                Some(CoordinateSystem::new(up, handedness)),
                "{up:?} {handedness:?}"
            );
        }
    }
}

#[test]
fn the_default_camera_placement_overlooks_the_origin() {
    let (_session, pose) = create_camera(Axis::PosY, Handedness::Left, None);
    let matrix = math::Mat4(pose.start);
    let origin = matrix.transform_point([0., 0., 0.]);
    assert!(
        (origin[0]).abs() < 1e-9 && (origin[1] - 2.).abs() < 1e-9 && (origin[2] - 6.).abs() < 1e-9,
        "the camera sits at (0, 2, 6), the side Frame/Focus view from: {origin:?}"
    );
    let tip = matrix.transform_point([0., 0., 1.]);
    let forward = sub(tip, origin);
    let look = sub([0., 0.5, 0.], [0., 2., 6.]);
    let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    let cosine = (forward[0] * look[0] + forward[1] * look[1] + forward[2] * look[2])
        / (length(forward) * length(look));
    assert!(
        (cosine - 1.).abs() < 1e-9,
        "looks at (0, 0.5, 0): {forward:?}"
    );
}

#[test]
fn the_camera_preset_takes_the_viewport_view() {
    let view = PresetView {
        position: [3., 4., 5.],
        look_at: [-1., 0., 2.],
    };
    for (up, handedness) in [
        (Axis::PosZ, Handedness::Right),
        (Axis::PosY, Handedness::Left),
    ] {
        let (_session, pose) = create_camera(up, handedness, Some(view));
        let matrix = math::Mat4(pose.start);
        let origin = matrix.transform_point([0., 0., 0.]);
        assert!(
            (origin[0] - view.position[0]).abs() < 1e-9
                && (origin[1] - view.position[1]).abs() < 1e-9
                && (origin[2] - view.position[2]).abs() < 1e-9,
            "{up:?}: the camera sits at the view position: {origin:?}"
        );
        let tip = matrix.transform_point([0., 0., 1.]);
        let forward = sub(tip, origin);
        let look = [
            view.look_at[0] - view.position[0],
            view.look_at[1] - view.position[1],
            view.look_at[2] - view.position[2],
        ];
        let length = |v: [f64; 3]| (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        let cosine = (forward[0] * look[0] + forward[1] * look[1] + forward[2] * look[2])
            / (length(forward) * length(look));
        assert!((cosine - 1.).abs() < 1e-9, "{up:?}: forward {forward:?}");
    }
}

#[test]
fn a_degenerate_camera_view_is_refused() {
    let document = ProjectDocument::new("Degenerate");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::CAMERA_PERSPECTIVE.id,
                parent: world,
                after: None,
                definition: None,
                view: Some(PresetView {
                    position: [0., 0., 0.],
                    look_at: [0., 0., 0.],
                }),
            }],
        )
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("degenerate"),
        "the refusal explains itself: {error}"
    );
}

#[test]
fn the_camera_becomes_the_render_camera_only_when_none_is_set() {
    let document = ProjectDocument::new("Render camera");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, presets::CAMERA_PERSPECTIVE.id, world);
    let camera = change.created[0];
    let target = scene(session.document());
    assert!(target.options.contains(&camera), "lands in the Options");
    assert_eq!(
        target.render_camera,
        Some(camera),
        "no camera rendered before"
    );

    // A scene that already renders from a camera keeps its choice.
    let mut document = ProjectDocument::new("Render camera");
    scene_mut(&mut document).coordinate_system =
        CoordinateSystem::new(Axis::PosZ, Handedness::Left);
    let existing = Node::new(
        "Existing",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: Vec::new(),
            pose: Some(CameraPose {
                start: math::Mat4::identity().0,
                end: math::Mat4::identity().0,
            }),
        },
    );
    let existing_id = existing.id;
    let world = scene(&document).world;
    {
        let target = scene_mut(&mut document);
        target.options.push(existing_id);
        target.nodes.insert(existing_id, existing);
        target.render_camera = Some(existing_id);
    }
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, presets::CAMERA_PERSPECTIVE.id, world);
    assert_eq!(
        scene(session.document()).render_camera,
        Some(existing_id),
        "the existing render camera is kept"
    );
    assert_eq!(change.created.len(), 1, "only the Camera is created");
    assert_ne!(change.created[0], existing_id);
}

/// with a viewport view, a shape preset's scope opens with the
/// placement states — a Translate to the view's look-at point and a
/// uniform Scale of a tenth of the eye's distance to it — ahead of the
/// preset's own rotation, so the shape lands where the user is
/// looking, sized to stay in frame.
#[test]
fn a_shape_preset_with_a_view_opens_with_the_placement_states() {
    let view = PresetView {
        position: [10., 2., 3.],
        look_at: [4., 0.5, -2.],
    };
    let distance: f64 = (6.0f64 * 6.0 + 1.5 * 1.5 + 5.0 * 5.0).sqrt();
    let document = ProjectDocument::new("Placed shape");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    for preset in [presets::SHAPE_CUBE, presets::SHAPE_DISK] {
        let change = create_with(&mut session, preset.id, world, Some(view));
        let NodeKind::Scope { children, .. } =
            &scene(session.document()).nodes[&change.created[0]].data
        else {
            panic!("{}: the group leads the changes", preset.name);
        };
        assert_eq!(
            children.as_slice(),
            &change.created[1..],
            "{}: the scope holds its members in creation order",
            preset.name
        );
        assert_eq!(
            scene(session.document()).nodes[&change.created[1]].data,
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate(view.look_at)),
            },
            "{}: the Translate opens the preset's own scope",
            preset.name
        );
        assert_eq!(
            scene(session.document()).nodes[&change.created[2]].data,
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Scale(
                    [distance / 10.; 3]
                )),
            },
            "{}: the Scale follows the Translate, a tenth of the eye's distance",
            preset.name
        );
        if preset.id == presets::SHAPE_DISK.id {
            assert!(
                matches!(
                    &scene(session.document()).nodes[&change.created[3]].data,
                    NodeKind::State {
                        operation: StateOperation::Transform(TransformOperation::Rotate { .. })
                    }
                ),
                "{}: the preset's own rotation keeps its place after the placement",
                preset.name
            );
        }
        assert!(
            matches!(
                &scene(session.document()).nodes[change.created.last().unwrap()].data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            ),
            "{}: the shape still closes the members",
            preset.name
        );
        // The evaluated body sits at the look-at point, sized by the view.
        let shape = *change.created.last().unwrap();
        let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
        let origin = evaluated.nodes[&shape].start.transform_point([0., 0., 0.]);
        assert!(
            (origin[0] - view.look_at[0]).abs() < 1e-9
                && (origin[1] - view.look_at[1]).abs() < 1e-9
                && (origin[2] - view.look_at[2]).abs() < 1e-9,
            "{} sits at {origin:?}, not the look-at point",
            preset.name
        );
        if preset.id == presets::SHAPE_CUBE.id {
            let edge = evaluated.nodes[&shape].start.transform_point([1., 0., 0.]);
            let span = ((edge[0] - origin[0]) * (edge[0] - origin[0])
                + (edge[1] - origin[1]) * (edge[1] - origin[1])
                + (edge[2] - origin[2]) * (edge[2] - origin[2]))
                .sqrt();
            assert!(
                (span - distance / 10.).abs() < 1e-9,
                "the unit edge scales to a tenth of the eye's distance: {span}"
            );
        }
    }
}

/// the view is in world space, so under a transformed parent the
/// placement is converted into the parent's local space — the shape
/// still lands at the look-at point, a tenth of the eye's distance big.
#[test]
fn a_placed_shape_under_a_transformed_parent_lands_at_the_look_at() {
    let view = PresetView {
        position: [10., 2., 3.],
        look_at: [4., 0.5, -2.],
    };
    let distance: f64 = (6.0f64 * 6.0 + 1.5 * 1.5 + 5.0 * 5.0).sqrt();
    let document = ProjectDocument::new("Placed under a parent");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    // An empty group moved and scaled: a transformed parent.
    let parent = create(&mut session, presets::GROUP_EMPTY.id, world).created[0];
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: parent,
                value: LocalTransform::Trs(transform_trs::Trs {
                    translation: [100., 0., 0.],
                    rotation_euler_degrees: [0., 30., 0.],
                    scale: [3.; 3],
                }),
            }],
        )
        .unwrap();
    let change = create_with(&mut session, presets::SHAPE_CUBE.id, parent, Some(view));
    let shape = *change.created.last().unwrap();
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    let origin = evaluated.nodes[&shape].start.transform_point([0., 0., 0.]);
    let edge = evaluated.nodes[&shape].start.transform_point([1., 0., 0.]);
    assert!(
        (0..3).all(|i| (origin[i] - view.look_at[i]).abs() < 1e-9),
        "the cube sits at {origin:?}, not the look-at point"
    );
    let span = (0..3)
        .map(|i| (edge[i] - origin[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    assert!(
        (span - distance / 10.).abs() < 1e-9,
        "the unit edge spans a tenth of the eye's distance in world space: {span}"
    );
}

/// point and directional lights rise without scaling; area-light
/// bodies also rise and scale to one tenth of the view distance.
#[test]
fn light_and_area_presets_with_a_view_use_their_expected_scale() {
    let view = PresetView {
        position: [8., 6., 0.],
        look_at: [1., 1., 1.],
    };
    let document = ProjectDocument::new("Placed light");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let up = [0., 1., 0.];
    let raised = presets::raised_light_position(view, up);
    let delta = std::array::from_fn::<_, 3, _>(|axis| view.position[axis] - view.look_at[axis]);
    let expected_scale = delta.iter().map(|value| value * value).sum::<f64>().sqrt() / 10.;
    for preset in [
        presets::LIGHT_POINT,
        presets::LIGHT_SPOT,
        presets::LIGHT_DISTANT,
        presets::AREA_QUAD,
        presets::AREA_SPHERE,
    ] {
        let change = create_with(&mut session, preset.id, world, Some(view));
        assert_eq!(
            scene(session.document()).nodes[&change.created[1]].data,
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate(raised)),
            },
            "{}: the Translate opens the preset's own scope",
            preset.name
        );
        let scale = change.created[2..].iter().find_map(|member| {
            match &scene(session.document()).nodes[member].data {
                NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Scale(scale)),
                } => Some(*scale),
                _ => None,
            }
        });
        if matches!(preset.name, "area.quad" | "area.sphere") {
            let scale = scale.expect("area-light bodies scale with the view distance");
            assert!(
                scale
                    .iter()
                    .all(|value| (value - expected_scale).abs() < 1e-9)
            );
        } else {
            assert_eq!(scale, None, "{} keeps its own reach", preset.name);
        }
    }
}

/// Spot and distant lights' local from/to reach from the raised
/// placement to the view target.
#[test]
fn spot_and_distant_lights_aim_at_the_view_target() {
    let view = PresetView {
        position: [5., 4., -1.],
        look_at: [0., 0., 0.],
    };
    let document = ProjectDocument::new("Aiming");
    let world = scene(&document).world;
    let mut placed = ProjectSession::new(document.clone()).unwrap();
    let raised = presets::raised_light_position(view, CoordinateSystem::default().up_vector());
    for preset in [presets::LIGHT_SPOT, presets::LIGHT_DISTANT] {
        let change = create_with(&mut placed, preset.id, world, Some(view));
        let node = change
            .created
            .iter()
            .find(|id| {
                matches!(
                    &scene(placed.document()).nodes[id].data,
                    NodeKind::Entity {
                        entity: EntityKind::Light,
                        ..
                    }
                )
            })
            .unwrap();
        let NodeKind::Entity { parameters, .. } = &scene(placed.document()).nodes[node].data else {
            unreachable!()
        };
        let target = parameters.iter().find(|p| p.name == "to").unwrap();
        let ParameterValues::Floats(values) = &target.values else {
            unreachable!()
        };
        let target = values
            .iter()
            .map(|v| match v {
                FloatValue::Finite(v) => *v,
                _ => panic!("finite target"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            target,
            view.look_at
                .iter()
                .zip(raised)
                .map(|(at, from)| at - from)
                .collect::<Vec<_>>()
        );
    }
}

/// without a view nothing changes — the body stays at the origin,
/// unit sized, no placement states.
#[test]
fn a_preset_without_a_view_keeps_the_origin_unit_body() {
    let document = ProjectDocument::new("Unplaced");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, presets::SHAPE_CUBE.id, world);
    assert_eq!(change.created.len(), 2, "only the group and the shape");
    let NodeKind::Scope { children, .. } =
        &scene(session.document()).nodes[&change.created[0]].data
    else {
        panic!("the preset group");
    };
    assert_eq!(
        children.as_slice(),
        &change.created[1..],
        "no placement states join the preset's scope"
    );
}

/// a view whose eye sits on its look-at point has no distance to
/// size a body with — the same refusal the camera preset's pose gives.
#[test]
fn a_degenerate_view_is_refused_for_a_placed_body() {
    let document = ProjectDocument::new("Degenerate placement");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    let error = session
        .apply(
            &session.revision(),
            &[Edit::CreatePreset {
                preset: presets::SHAPE_SPHERE.id,
                parent: world,
                after: None,
                definition: None,
                view: Some(PresetView {
                    position: [1., 2., 3.],
                    look_at: [1., 2., 3.],
                }),
            }],
        )
        .unwrap_err();
    assert!(
        error.to_string().to_lowercase().contains("degenerate"),
        "the refusal explains itself: {error}"
    );
}

/// the group and object-instance presets still take no view —
/// they create nothing placeable; the camera keeps its existing meaning.
#[test]
fn view_is_still_refused_for_the_group_and_instance_presets() {
    let view = PresetView {
        position: [0., 1., 0.],
        look_at: [0., 0., 0.],
    };
    let document = ProjectDocument::new("View scope");
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    for preset in [presets::GROUP_EMPTY, presets::INSTANCE_OBJECT] {
        let error = session
            .apply(
                &session.revision(),
                &[Edit::CreatePreset {
                    preset: preset.id,
                    parent: world,
                    after: None,
                    definition: None,
                    view: Some(view),
                }],
            )
            .unwrap_err();
        assert!(
            error.to_string().contains("view "),
            "the refusal names the view: {error}"
        );
    }
}

#[test]
fn preset_listings_cover_the_camera_and_floor() {
    assert_eq!(
        presets::creates(&presets::CAMERA_PERSPECTIVE),
        ["camera:perspective"]
    );
    assert_eq!(
        presets::creates(&presets::SHAPE_FLOOR),
        ["attribute", "shape:bilinearmesh"]
    );
    assert_eq!(
        presets::creates(&presets::SHAPE_PLANE),
        ["attribute", "shape:bilinearmesh"]
    );
}

#[test]
fn the_initial_document_carries_camera_floor_and_environment() {
    for (up, handedness) in [
        (Axis::PosY, Handedness::Left),
        (Axis::PosZ, Handedness::Right),
    ] {
        let coordinate_system = CoordinateSystem::new(up, handedness);
        let document =
            presets::initial_document("Fresh", coordinate_system).expect("a fresh project");
        let scene = scene(&document);
        assert_eq!(scene.coordinate_system, coordinate_system);
        assert_eq!(document.title, "Fresh");
        assert_eq!(scene.location, "Scenes/Fresh.scene");
        // The camera renders and looks like a scene authored in that
        // coordinate system.
        let camera = scene.render_camera.expect("a render camera");
        assert!(scene.options.contains(&camera));
        let NodeKind::Entity {
            pose: Some(pose), ..
        } = &scene.nodes[&camera].data
        else {
            panic!("the initial camera is posed");
        };
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(coordinate_system)
        );
        // a Film, a Sampler and an Integrator ride in Options with
        // quality values — exactly one of those three kinds,
        // nothing else.
        let configurations = |kind: ConfigurationKind| -> Vec<(&str, &[Parameter])> {
            scene
                .options
                .iter()
                .filter_map(|id| scene.nodes.get(id))
                .filter_map(|node| match &node.data {
                    NodeKind::Configuration {
                        configuration,
                        implementation,
                        parameters,
                    } if *configuration == kind => {
                        Some((implementation.as_str(), parameters.as_slice()))
                    }
                    _ => None,
                })
                .collect()
        };
        for kind in [
            ConfigurationKind::Film,
            ConfigurationKind::Sampler,
            ConfigurationKind::Integrator,
            ConfigurationKind::PixelFilter,
            ConfigurationKind::Accelerator,
        ] {
            let expected = usize::from(matches!(
                kind,
                ConfigurationKind::Film
                    | ConfigurationKind::Sampler
                    | ConfigurationKind::Integrator
            ));
            assert_eq!(
                configurations(kind).len(),
                expected,
                "{kind:?}: only Film, Sampler and Integrator, one each"
            );
        }
        let film = configurations(ConfigurationKind::Film);
        assert_eq!(film[0].0, "rgb");
        assert_eq!(
            film[0].1,
            &[
                text("filename", &["Fresh.exr"]),
                integers("xresolution", &[640]),
                integers("yresolution", &[360]),
            ][..]
        );
        let sampler = configurations(ConfigurationKind::Sampler);
        assert_eq!(sampler[0].0, "zsobol");
        assert_eq!(sampler[0].1, &[integers("pixelsamples", &[64])][..]);
        let integrator = configurations(ConfigurationKind::Integrator);
        assert_eq!(integrator[0].0, "volpath");
        assert_eq!(integrator[0].1, &[integers("maxdepth", &[5])][..]);
        // The world holds a Floor and an Environment Light.
        let implementations: Vec<String> = scene
            .nodes
            .values()
            .filter_map(|node| match &node.data {
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    implementation,
                    ..
                } => Some(implementation.clone()),
                _ => None,
            })
            .collect();
        assert!(
            implementations.iter().any(|name| name == "bilinearmesh"),
            "the floor is a bilinearmesh: {implementations:?}"
        );
        let worlds_light = scene.nodes.values().any(|node| {
            matches!(
                &node.data,
                NodeKind::Entity {
                    entity: EntityKind::Light,
                    implementation,
                    ..
                } if implementation == "infinite"
            )
        });
        assert!(worlds_light, "the environment light is there");
        document.validate().unwrap();
        // It round-trips through a save.
        let mut session = ProjectSession::new(document.clone()).unwrap();
        let receipt = FolderStore::create_v3(
            &tempfile::tempdir().unwrap().path().join("fresh"),
            document.id,
            &document.title,
            session.revision(),
        )
        .unwrap()
        .save(&session.snapshot())
        .unwrap();
        session.mark_saved(&receipt).unwrap();
        assert!(!session.is_dirty());
    }
}

/// The default implementation for each configuration kind is what an Add
/// Component entry creates and what the initial scene uses.
#[test]
fn default_implementations_pin_the_planned_kinds() {
    assert_eq!(
        presets::default_implementation(ConfigurationKind::Film),
        "rgb"
    );
    assert_eq!(
        presets::default_implementation(ConfigurationKind::Sampler),
        "zsobol"
    );
    assert_eq!(
        presets::default_implementation(ConfigurationKind::Integrator),
        "volpath"
    );
    assert_eq!(
        presets::default_implementation(ConfigurationKind::PixelFilter),
        "gaussian"
    );
    assert_eq!(
        presets::default_implementation(ConfigurationKind::Accelerator),
        "bvh"
    );
}

/// The film's output name follows the title, made safe for a file name;
/// a title with nothing nameable falls back to the export's own
/// "render.exr".
#[test]
fn the_films_output_name_follows_a_sanitized_title() {
    for (title, expected) in [
        ("Fresh", "Fresh.exr"),
        ("My Project", "My Project.exr"),
        ("黑色: 甲/乙?", "黑色_ 甲_乙.exr"),
        ("日本語 床", "日本語 床.exr"),
        ("...", "render.exr"),
        ("/", "render.exr"),
        ("a/b", "a_b.exr"),
    ] {
        assert_eq!(presets::exr_name(title), expected, "{title}");
    }
    // The scene file follows the same stem, so any title makes a project.
    for (title, expected) in [
        ("a: b/..", "Scenes/a_ b.scene"),
        ("...", "Scenes/Scene.scene"),
    ] {
        let document = presets::initial_document(title, Default::default()).expect(title);
        let scene = document.scene(document.default_scene.unwrap()).unwrap();
        assert_eq!(scene.location, expected, "{title}");
    }
}

#[test]
fn plane_presets_face_positive_y_everywhere() {
    for preset in [presets::SHAPE_PLANE, presets::AREA_QUAD] {
        // PBRT bilinear patch corners are p00, p10, p01, p11 and the surface
        // normal is cross(dp/du, dp/dv); check it at every corner so a folded
        // patch (normal flipping across the surface) also fails.
        let p = points(&body(&preset));
        assert_eq!(p.len(), 4, "{}", preset.name);
        for (u, v) in [(0., 0.), (1., 0.), (0., 1.), (1., 1.)] {
            let dpdu = (0..3)
                .map(|i| (1. - v) * (p[1][i] - p[0][i]) + v * (p[3][i] - p[2][i]))
                .collect::<Vec<_>>();
            let dpdv = (0..3)
                .map(|i| (1. - u) * (p[2][i] - p[0][i]) + u * (p[3][i] - p[1][i]))
                .collect::<Vec<_>>();
            let normal = cross([dpdu[0], dpdu[1], dpdu[2]], [dpdv[0], dpdv[1], dpdv[2]]);
            assert!(
                normal[1] > 0. && normal[0].abs() < 1e-12 && normal[2].abs() < 1e-12,
                "{} normal at ({u}, {v}) is {normal:?}",
                preset.name
            );
        }
    }
}

#[test]
fn cube_preset_triangles_face_outward() {
    let parameters = body(&presets::SHAPE_CUBE);
    let p = points(&parameters);
    let ParameterValues::Integers(indices) = &parameters
        .iter()
        .find(|parameter| parameter.name == "indices")
        .unwrap()
        .values
    else {
        panic!("indices are not integers");
    };
    assert_eq!((p.len(), indices.len()), (8, 36));
    for triangle in indices.chunks(3) {
        let [a, b, c] = [0, 1, 2].map(|i| p[triangle[i] as usize]);
        let center = [0, 1, 2].map(|i| (a[i] + b[i] + c[i]) / 3.);
        assert!(
            dot(cross(sub(b, a), sub(c, a)), center) > 0.,
            "triangle {triangle:?} faces inward"
        );
    }
}

#[test]
fn create_preset_inside_object_definition_stays_in_definition() {
    let mut document = ProjectDocument::new("Definition preset");
    let (_, root) = object_definition(&mut document);
    let mut session = ProjectSession::new(document).unwrap();
    let change = create(&mut session, presets::SHAPE_SPHERE.id, root);
    assert_eq!(change.created.len(), 2);
    for id in &change.created {
        assert!(session.document().definition_nodes.contains_key(id));
        assert!(!scene(session.document()).nodes.contains_key(id));
    }
    let NodeKind::Scope { children, .. } = &session.document().definition_nodes[&root].data else {
        unreachable!();
    };
    assert_eq!(children.last(), change.created.first());
    session.document().validate().unwrap();
    session.undo(&session.revision()).unwrap();
    for id in &change.created {
        assert!(!session.document().definition_nodes.contains_key(id));
    }
}

#[test]
fn clearing_a_local_binding_keeps_a_moved_preset_group() {
    fn material(document: &mut ProjectDocument) -> ResourceId {
        let resource = Resource {
            id: ResourceId::new(),
            label: "matte".into(),
            location: "Materials/matte.material".into(),
            edition: Revision::initial(),
            kind: ResourceKind::Material,
            texture_value_type: None,
            implementation: "matte".into(),
            parameters: Vec::new(),
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let id = resource.id;
        document.resources.insert(id, resource);
        id
    }
    let mut document = ProjectDocument::new("Preset binding");
    let matte = material(&mut document);
    let world = scene(&document).world;
    let mut session = ProjectSession::new(document).unwrap();
    create(&mut session, presets::SHAPE_SPHERE.id, world);
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&world].data else {
        unreachable!();
    };
    let group_id = children[0];
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert_eq!(children.len(), 1, "a preset group holds only its shape");
    let shape_id = children[0];
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalTransform {
                node: shape_id,
                value: LocalTransform::Trs(transform_trs::Trs {
                    translation: [0., 2., 0.],
                    rotation_euler_degrees: [0.; 3],
                    scale: [1.; 3],
                }),
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    let translate_id = children[0];
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape_id,
                material: Some(MaterialBinding::Resource(matte)),
                medium: None,
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert_eq!(children.len(), 3);
    let binding_id = children[1];
    // Clearing removes the binding but keeps the preset group and its transform.
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape_id,
                material: Some(MaterialBinding::Default),
                medium: None,
            }],
        )
        .unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert_eq!(children, &[translate_id, shape_id]);
    assert!(!scene(session.document()).nodes.contains_key(&binding_id));
    let evaluated = evaluate(&session.snapshot().scene_view().unwrap()).unwrap();
    assert_eq!(
        evaluated.nodes[&shape_id].material,
        MaterialBinding::Default
    );
    session.undo(&session.revision()).unwrap();
    let NodeKind::Scope { children, .. } = &scene(session.document()).nodes[&group_id].data else {
        unreachable!();
    };
    assert_eq!(children, &[translate_id, binding_id, shape_id]);
}
