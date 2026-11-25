//! wire tests: scene graph view, components and presets.
use super::*;
use project::{
    ConfigurationKind, EntityKind, Node, NodeKind, Parameter, ProjectDocument, ScopeKind,
    StateOperation, TransformOperation,
};

fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: Uuid::new_v4().to_string(),
        method,
        params,
    }
}

fn entity(label: &str, entity: EntityKind, implementation: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity,
            implementation: implementation.into(),
            parameters: Vec::new(),
            pose: None,
        },
    )
}

/// A camera with an identity pose, as produced by a resolved import.
/// Cameras without a pose cannot be edited.
fn posed_camera(label: &str, implementation: &str) -> Node {
    Node::new(
        label,
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: implementation.into(),
            parameters: Vec::new(),
            pose: Some(project::CameraPose {
                start: project::math::Mat4::identity().0,
                end: project::math::Mat4::identity().0,
            }),
        },
    )
}

fn state(label: &str, operation: StateOperation) -> Node {
    Node::new(label, NodeKind::State { operation })
}

fn group(label: &str, children: &[&Node]) -> Node {
    Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: children.iter().map(|node| node.id).collect(),
            owned_object: None,
            parent_camera: None,
        },
    )
}

fn owned_group(label: &str, children: &[&Node], owner: &Node) -> Node {
    Node::new(
        label,
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: children.iter().map(|node| node.id).collect(),
            owned_object: Some(owner.id),
            parent_camera: None,
        },
    )
}

/// Builds the default scene from option nodes and World children; every node
/// in `nodes` is inserted into the scene table.
fn document(options: &[&Node], world: &[&Node], nodes: Vec<Node>) -> ProjectDocument {
    let mut document = ProjectDocument::new("Graph");
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    scene.options = options.iter().map(|node| node.id).collect();
    scene.render_camera = options
        .iter()
        .rev()
        .find(|node| {
            matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
        })
        .map(|node| node.id);
    let world_id = scene.world;
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world_id).unwrap().data else {
        unreachable!()
    };
    *children = world.iter().map(|node| node.id).collect();
    for node in nodes {
        scene.nodes.insert(node.id, node);
    }
    document.validate().unwrap();
    document
}

fn loaded(document: ProjectDocument) -> CoreState {
    let mut core = CoreState::new(Limits::default());
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    let job: JobId = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.project_io_finished(
        job,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document).unwrap().snapshot(),
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    core
}

fn call(core: &mut CoreState, method: Method, mut params: Value) -> Result<Value, ErrorPayload> {
    params["project"] = json!(core.projects.reference().unwrap());
    let response = core.handle(request(method, params));
    match response.error {
        Some(error) => Err(error),
        None => Ok(response.result.unwrap()),
    }
}

fn ok(core: &mut CoreState, method: Method, params: Value) -> Value {
    let name = format!("{method:?}");
    call(core, method, params).unwrap_or_else(|error| panic!("{name}: {error:?}"))
}

fn graph(core: &mut CoreState, parent: Value) -> Vec<Value> {
    ok(
        core,
        Method::ProjectListNodes,
        json!({"parent":parent,"view":"scene_graph"}),
    )["nodes"]
        .as_array()
        .unwrap()
        .clone()
}

fn ids(nodes: &[Value]) -> Vec<String> {
    nodes
        .iter()
        .map(|node| node["node_id"].as_str().unwrap().to_owned())
        .collect()
}

fn id(node: &Node) -> String {
    node.id.0.to_string()
}

fn component<'a>(node: &'a Value, kind: &str) -> &'a Value {
    node["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == kind)
        .unwrap_or_else(|| panic!("no {kind} component in {node}"))
}

#[test]
fn camera_rows_list_their_attached_groups() {
    // the hair-scene shape — a light group attached to the camera,
    // an owned sphere group attached as well, and a free sphere.
    let mut camera = posed_camera("Camera", "perspective");
    let mut pose = scene_graph_pose();
    pose.start = project::math::Mat4::translation(2., 3., 8.).0;
    pose.end = project::math::Mat4::translation(2., 3., 8.).0;
    if let NodeKind::Entity {
        pose: Some(node_pose),
        ..
    } = &mut camera.data
    {
        *node_pose = pose;
    }
    let light = entity("Fill", EntityKind::Light, "point");
    let attached_light = project::Node::new(
        "Camera lights",
        project::NodeKind::Scope {
            scope: project::ScopeKind::Attribute,
            children: vec![light.id],
            owned_object: None,
            parent_camera: Some(camera.id),
        },
    );
    let owned_sphere = entity("Rig", EntityKind::Shape, "sphere");
    let attached_owned_group = project::Node::new(
        "Rig group",
        project::NodeKind::Scope {
            scope: project::ScopeKind::Attribute,
            children: vec![owned_sphere.id],
            owned_object: Some(owned_sphere.id),
            parent_camera: Some(camera.id),
        },
    );
    let free = entity("Free", EntityKind::Shape, "sphere");
    let built = document(
        &[&camera],
        &[&attached_light, &attached_owned_group, &free],
        vec![
            camera.clone(),
            attached_light.clone(),
            attached_owned_group.clone(),
            owned_sphere.clone(),
            light.clone(),
            free.clone(),
        ],
    );
    let scene = built.default_scene.unwrap().0.to_string();
    let mut core = loaded(built);

    // Scene-root children: the camera and the free sphere — the attached
    // groups moved under the camera row.
    let roots = graph(&mut core, json!(scene));
    assert_eq!(ids(&roots), [id(&camera), id(&free)]);
    let camera_row = &roots[0];
    assert_eq!(camera_row["child_count"], 2, "both attached groups");

    // Listing under the camera: the light group (it is its own row) and
    // the owned wrapper's object (a wrapper is never its own row).
    let children = graph(&mut core, json!(id(&camera)));
    assert_eq!(ids(&children), [id(&attached_light), id(&owned_sphere)]);

    // The attached group's local parents at the camera; the light inside it
    // stays group-relative, and the camera's local is its pose (root).
    let attached_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":attached_light.id}),
    );
    let attached_transform = component(&attached_node, "transform").clone();
    let camera_parent = json!({"camera": id(&camera)});
    assert_eq!(attached_transform["local"]["parent"], camera_parent);
    let light_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":light.id}),
    );
    let light_transform = component(&light_node, "transform").clone();
    assert_eq!(light_transform["local"]["parent"], "group");
    // Light world = camera pose (the group enters at the camera).
    assert_eq!(
        light_transform["world"]["start"],
        json!(project::math::Mat4::translation(2., 3., 8.).0),
        "{light_transform}"
    );
    let camera_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":camera.id}),
    );
    let camera_transform = component(&camera_node, "transform");
    assert_eq!(camera_transform["local"]["parent"], "root");
    assert_eq!(
        camera_transform["world"]["start"],
        json!(project::math::Mat4::translation(2., 3., 8.).0),
        "the camera's world is its pose"
    );
    assert_eq!(
        json!(camera_transform["local"]["matrix"]),
        camera_transform["world"]["start"],
        "the camera's local is its pose (root parent)"
    );

    // Search paths follow the visible rows: through the attached group to
    // the camera, then the Scene root (the lazy-reveal chain).
    let hits = ok(
        &mut core,
        Method::ProjectSearchNodes,
        json!({"query":"Fill","view":"scene_graph"}),
    )["nodes"]
        .as_array()
        .unwrap()
        .clone();
    assert!(hits.len() == 1, "{hits:?}");
    assert_eq!(
        hits[0]["path"],
        json!([id(&attached_light), id(&camera), scene])
    );
    let free_hits = ok(
        &mut core,
        Method::ProjectSearchNodes,
        json!({"query":"Free","view":"scene_graph"}),
    )["nodes"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(free_hits[0]["path"], json!([scene]));
}

fn scene_graph_pose() -> project::CameraPose {
    project::CameraPose {
        start: project::math::Mat4::identity().0,
        end: project::math::Mat4::identity().0,
    }
}

/// A camera's local is its pose and edits refuse the static API:
/// a posed animated camera reports per-sample poses; a camera without a
/// pose (an older import) is read-only with the older-import reason.
#[test]
fn camera_poses_report_local_samples_and_older_import_reasons() {
    let times = state("TransformTimes", StateOperation::TransformTimes([2., 5.]));
    let mut animated = posed_camera("Animated Camera", "perspective");
    let start = project::math::Mat4::translation(0., 0., 1.).0;
    let end = project::math::Mat4::translation(0., 4., 0.).0;
    if let NodeKind::Entity {
        pose: Some(node_pose),
        ..
    } = &mut animated.data
    {
        *node_pose = project::CameraPose { start, end };
    }
    let mut core = loaded(document(
        &[&times, &animated],
        &[],
        vec![times.clone(), animated.clone()],
    ));
    let details = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":animated.id}),
    );
    let transform = component(&details, "transform").clone();
    assert_eq!(transform["local"]["parent"], "root");
    assert_eq!(transform["local"]["matrix"], Value::Null);
    assert_eq!(transform["local"]["matrix_reason"], "animated");
    assert_eq!(transform["local"]["samples"]["times"], json!([2., 5.]));
    assert_eq!(
        transform["local"]["samples"]["start"]["matrix"],
        json!(start)
    );
    assert_eq!(transform["local"]["samples"]["end"]["matrix"], json!(end));
    assert_eq!(transform["editable"], false);

    let rejected = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_matrix","node_id":animated.id,
            "matrix":project::math::Mat4::identity().0}]}),
    );
    assert!(
        rejected
            .as_ref()
            .err()
            .is_some_and(|error| error.message.contains("animated"))
    );

    // A camera without a pose is an older import: read-only with the
    // older-import reason, edits refused by the session.
    let legacy = entity("Legacy Camera", EntityKind::Camera, "perspective");
    let legacy_look = state(
        "LookAt",
        StateOperation::Transform(TransformOperation::LookAt([
            0., 1., 5., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let mut core = loaded(document(
        &[&legacy_look, &legacy],
        &[],
        vec![legacy_look.clone(), legacy.clone()],
    ));
    let details = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":legacy.id}),
    );
    let transform = component(&details, "transform").clone();
    assert_eq!(transform["editable"], false);
    assert_eq!(
        transform["read_only_reason"],
        "The camera has no pose (an older import; re-import to edit it)"
    );
    // The declared state still reports the world placement.
    let eye = serde_json::from_value::<Vec<f64>>(transform["local"]["trs"]["translation"].clone())
        .unwrap();
    assert!(
        eye.iter()
            .zip([0., 1., 5.])
            .all(|(a, b)| (a - b).abs() < 1e-9)
    );
    let rejected = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_matrix","node_id":legacy.id,
            "matrix":project::math::Mat4::identity().0}]}),
    );
    assert!(
        rejected
            .as_ref()
            .err()
            .is_some_and(|error| error.message.contains("older import"))
    );
}

#[test]
fn scene_graph_view_hides_state_and_attaches_area_lights() {
    let camera = entity("Camera", EntityKind::Camera, "perspective");
    let film = Node::new(
        "Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "rgb".into(),
            parameters: Vec::new(),
        },
    );
    let placement = state(
        "Placement",
        StateOperation::Transform(TransformOperation::Translate([0., 2.8, 0.])),
    );
    let area = entity("Emitter", EntityKind::AreaLight, "diffuse");
    let ceiling = entity("Ceiling", EntityKind::Shape, "sphere");
    let room = group("Room", &[&area, &ceiling]);
    let sky = entity("Sky", EntityKind::Light, "infinite");
    let document = document(
        &[&camera, &film],
        &[&placement, &room, &sky],
        vec![
            camera.clone(),
            film.clone(),
            placement.clone(),
            area.clone(),
            ceiling.clone(),
            room.clone(),
            sky.clone(),
        ],
    );
    let scene = document.default_scene.unwrap().0.to_string();
    let world = document
        .default_scene_view()
        .unwrap()
        .scene
        .world
        .0
        .to_string();
    let mut core = loaded(document);

    let roots = graph(&mut core, Value::Null);
    assert_eq!(ids(&roots), [scene.clone()]);
    assert_eq!(roots[0]["object"], "scene");
    assert_eq!(roots[0]["child_count"], 3);
    let scene_children = graph(&mut core, json!(scene));
    assert_eq!(ids(&scene_children), [id(&camera), id(&room), id(&sky)]);
    assert_eq!(scene_children[1]["object"], "group");
    assert_eq!(scene_children[1]["child_count"], 1);
    assert_eq!(
        scene_children[1]["components"],
        json!(["transform", "area_light"])
    );
    let room_children = graph(&mut core, json!(id(&room)));
    assert_eq!(ids(&room_children), [id(&ceiling)]);
    assert_eq!(
        room_children[0]["components"],
        json!([
            "transform",
            "shape",
            "area_light",
            "reverse_orientation",
            "material",
            "medium"
        ])
    );
    assert_eq!(
        scene_children[2]["components"],
        json!(["transform", "light", "medium"])
    );

    // Directives stay the default view and still list the state nodes.
    let directives = ok(&mut core, Method::ProjectListNodes, json!({"parent":world}));
    assert_eq!(directives["total"], 3);

    let hits = |core: &mut CoreState, query: &str, view: &str| {
        ok(
            core,
            Method::ProjectSearchNodes,
            json!({"query":query,"view":view}),
        )["nodes"]
            .as_array()
            .unwrap()
            .clone()
    };
    assert!(hits(&mut core, "Placement", "scene_graph").is_empty());
    assert!(hits(&mut core, "Emitter", "scene_graph").is_empty());
    assert_eq!(hits(&mut core, "Placement", "directives").len(), 1);
    let ceiling_hit = hits(&mut core, "Ceiling", "scene_graph");
    assert_eq!(ceiling_hit[0]["path"], json!([id(&room), scene]));

    let shape = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":ceiling.id}),
    );
    let area_light = component(&shape, "area_light");
    assert_eq!(area_light["node_id"], id(&area));
    assert_eq!(area_light["implementation"], "diffuse");
    // Visible hierarchy path omits the hidden World scope.
    assert_eq!(shape["path"], json!([id(&room), scene]));
    assert_eq!(
        hits(&mut core, "Ceiling", "directives")[0]["path"],
        json!([id(&room), world])
    );
}

#[test]
fn scene_graph_flattens_only_explicitly_owned_attribute_scopes() {
    let generated_shape = entity("Generated sphere", EntityKind::Shape, "sphere");
    let generated_wrapper = Node::new(
        "Internal generated wrapper",
        NodeKind::Scope {
            scope: ScopeKind::Attribute,
            children: vec![generated_shape.id],
            owned_object: Some(generated_shape.id),
            parent_camera: None,
        },
    );
    let ordinary_shape = entity("Ordinary sphere", EntityKind::Shape, "sphere");
    let ordinary_group = group("User group", &[&ordinary_shape]);
    let document = document(
        &[],
        &[&generated_wrapper, &ordinary_group],
        vec![
            generated_shape.clone(),
            generated_wrapper.clone(),
            ordinary_shape.clone(),
            ordinary_group.clone(),
        ],
    );
    let scene = document.default_scene.unwrap().0;
    let mut core = loaded(document);

    let roots = graph(&mut core, json!(scene));
    assert_eq!(ids(&roots), [id(&generated_shape), id(&ordinary_group)]);
    assert_eq!(roots[1]["object"], "group");
    assert_eq!(roots[1]["child_count"], 1);
    let listing = ok(
        &mut core,
        Method::ProjectListNodes,
        json!({"parent":scene,"view":"scene_graph","limit":1}),
    );
    assert_eq!(listing["total"], 2);
    assert_eq!(listing["next_offset"], 1);

    let generated = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":generated_shape.id}),
    );
    assert_eq!(generated["path"], json!([scene]));
    let wrapper_details = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":generated_wrapper.id}),
    );
    assert_eq!(wrapper_details["path"], json!([scene]));
    assert_eq!(
        wrapper_details["data"]["owned_object"],
        id(&generated_shape)
    );
    let ordinary = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":ordinary_shape.id}),
    );
    assert_eq!(ordinary["path"], json!([id(&ordinary_group), scene]));

    let hidden_wrapper = ok(
        &mut core,
        Method::ProjectSearchNodes,
        json!({"query":"Internal generated wrapper","view":"scene_graph"}),
    );
    assert_eq!(hidden_wrapper["total"], 0);
    assert!(
        call(
            &mut core,
            Method::ProjectListNodes,
            json!({"parent":generated_wrapper.id,"view":"scene_graph"}),
        )
        .is_err()
    );
}

#[test]
fn get_node_reports_components_and_shared_transforms() {
    let camera = entity("Camera", EntityKind::Camera, "perspective");
    let film = Node::new(
        "Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "rgb".into(),
            parameters: Vec::new(),
        },
    );
    let placement = state(
        "Placement",
        StateOperation::Transform(TransformOperation::Translate([0., 2.8, 0.])),
    );
    let first = entity("First", EntityKind::Shape, "sphere");
    let second = entity("Second", EntityKind::Shape, "sphere");
    let lamp = entity("Lamp", EntityKind::Light, "point");
    let mut document = document(
        &[&camera, &film],
        &[&placement, &first, &second, &lamp],
        vec![
            camera.clone(),
            film.clone(),
            placement.clone(),
            first.clone(),
            second.clone(),
            lamp.clone(),
        ],
    );
    let material = project::Resource {
        id: project::ResourceId::new(),
        label: "wall".into(),
        location: "Materials/wall.material".into(),
        edition: project::Revision::initial(),
        kind: project::ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: vec![Parameter::floats("roughness", [0.])],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let material_id = material.id;
    document.resources.insert(material_id, material);
    let scene = document.default_scene.unwrap().0;
    let mut core = loaded(document);

    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":first.id}),
    );
    let kinds = node["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["kind"].clone())
        .collect::<Vec<_>>();
    assert_eq!(
        kinds,
        [
            "transform",
            "shape",
            "reverse_orientation",
            "material",
            "medium",
        ]
        .map(Value::from)
    );
    let transform = component(&node, "transform");
    assert_eq!(
        transform["operations"],
        json!([{"node_id":placement.id,"value":{"kind":"translate","value":[0.0,2.8,0.0]}}])
    );
    // The translation also moves Second and Lamp.
    assert_eq!(transform["shared_with"], 2);
    let world = transform["world"]["start"].as_array().unwrap();
    assert_eq!(world.len(), 16);
    assert!(world.iter().any(|value| value.as_f64() == Some(2.8)));
    assert_eq!(transform["reverse_orientation"], false);
    let binding = component(&node, "material");
    assert_eq!(binding["binding"], json!({"kind":"default"}));
    assert_eq!(binding["binding_node"], Value::Null);
    assert_eq!(binding["local"], false);

    // A local binding wraps Second in its own group and is reported as local.
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_binding","node_id":second.id,
            "material":{"kind":"resource","value":material_id},"medium":null}]}),
    );
    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":second.id}),
    );
    let binding = component(&node, "material");
    assert_eq!(
        binding["binding"],
        json!({"kind":"resource","value":material_id})
    );
    assert!(binding["binding_node"].is_string());
    assert_eq!(binding["local"], true);
    assert_eq!(component(&node, "transform")["shared_with"], 0);
    let references = ok(
        &mut core,
        Method::ProjectListReferences,
        json!({"item":{"kind":"resource","id":material_id}}),
    );
    assert_eq!(references["references"][0]["node"]["id"], id(&second));
    assert_eq!(references["references"][0]["node"]["path"], json!([scene]));

    let root = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(root["data"]["kind"], "scene");
    assert_eq!(component(&root, "render_settings")["camera"], id(&camera));
    let camera_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":camera.id}),
    );
    assert_eq!(
        camera_node["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|component| component["kind"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["transform", "camera", "medium"]
    );
    // Non-object nodes have no components.
    let placement = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":placement.id}),
    );
    assert_eq!(placement["components"], json!([]));
}

#[test]
fn node_active_is_returned_on_scene_graph_rows_and_get_node() {
    let shape = entity("Toggle", EntityKind::Shape, "sphere");
    let document = document(&[], &[&shape], vec![shape.clone()]);
    let scene_id = document.default_scene.unwrap();
    let mut core = loaded(document);
    let row = ok(
        &mut core,
        Method::ProjectListNodes,
        json!({"parent":scene_id,"view":"scene_graph"}),
    );
    assert_eq!(row["nodes"][0]["active"], true);
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_node_active","node_id":shape.id,"active":false}]}),
    );
    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":shape.id}),
    );
    assert_eq!(node["active"], false);
    let row = ok(
        &mut core,
        Method::ProjectListNodes,
        json!({"parent":scene_id,"view":"scene_graph"}),
    );
    assert_eq!(row["nodes"][0]["active"], false);
}

#[test]
fn cameras_are_independently_editable_and_scene_owns_render_film() {
    let first = posed_camera("First Camera", "perspective");
    let second = posed_camera("Second Camera", "orthographic");
    let first_film = Node::new(
        "First Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "rgb".into(),
            parameters: Vec::new(),
        },
    );
    let second_film = Node::new(
        "Second Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "gbuffer".into(),
            parameters: Vec::new(),
        },
    );
    let document = document(
        &[&first, &first_film, &second, &second_film],
        &[],
        vec![
            first.clone(),
            first_film.clone(),
            second.clone(),
            second_film.clone(),
        ],
    );
    let scene = document.default_scene.unwrap();
    let mut core = loaded(document);
    let scene_children = graph(&mut core, json!(scene));
    assert_eq!(ids(&scene_children), [id(&first), id(&second)]);

    let first_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":first.id}),
    );
    assert_eq!(first_node["components"][0]["editable"], true);
    assert_eq!(
        first_node["components"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|component| component["kind"] == "film")
            .count(),
        0
    );

    let second_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":second.id}),
    );
    assert_eq!(second_node["components"][0]["editable"], true);
    // The render camera carries no Film either: the active
    // Film belongs to the Scene root, as it does without a camera.
    assert!(
        second_node["components"]
            .as_array()
            .unwrap()
            .iter()
            .all(|component| component["kind"] != "film"),
        "the render camera reports no film component: {second_node}"
    );
    let root = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(component(&root, "render_settings")["camera"], id(&second));
    let film_component = component(&root, "film");
    assert_eq!(film_component["node_id"], id(&second_film));
    assert_eq!(film_component["implementation"], "gbuffer");

    let result = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_matrix","node_id":first.id,
            "matrix":project::math::Mat4::identity().0}]}),
    );
    assert!(result.is_ok(), "{result:?}");

    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_render_camera","scene":scene,"camera":first.id}]}),
    );
    let selected_first = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(
        component(&selected_first, "render_settings")["camera"],
        id(&first)
    );
    // Switching the render camera moves nothing: the active Film stays
    // on the Scene root, and the newly selected camera has none either.
    assert_eq!(
        component(&selected_first, "film")["node_id"],
        id(&second_film)
    );
    assert!(
        ok(
            &mut core,
            Method::ProjectGetNode,
            json!({"node_id":first.id})
        )["components"]
            .as_array()
            .unwrap()
            .iter()
            .all(|component| component["kind"] != "film"),
        "the newly selected camera reports no film component"
    );
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_render_camera","scene":scene,"camera":null}]}),
    );
    let unset = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(component(&unset, "render_settings")["camera"], Value::Null);
    assert_eq!(component(&unset, "film")["node_id"], id(&second_film));
}

/// The Film is always a Scene root component — after the render
/// settings and before the Sampler — and never the Camera's, even with a
/// render camera set.
#[test]
fn film_with_a_render_camera_lives_on_the_scene_root() {
    let camera = posed_camera("Camera", "perspective");
    let film = Node::new(
        "Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "rgb".into(),
            parameters: Vec::new(),
        },
    );
    let sampler = Node::new(
        "Sampler",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Sampler,
            implementation: "zsobol".into(),
            parameters: Vec::new(),
        },
    );
    let document = document(
        &[&camera, &film, &sampler],
        &[],
        vec![camera.clone(), film.clone(), sampler.clone()],
    );
    let scene = document.default_scene.unwrap();
    let mut core = loaded(document);
    let kinds = |node: &Value| {
        node["components"]
            .as_array()
            .unwrap()
            .iter()
            .map(|component| component["kind"].clone())
            .collect::<Vec<_>>()
    };
    // The render camera is set, and the active Film still belongs to the
    // Scene root, in the former orphan-Film position.
    let root = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(component(&root, "render_settings")["camera"], id(&camera));
    assert_eq!(component(&root, "film")["node_id"], id(&film));
    assert_eq!(
        kinds(&root),
        ["scene", "render_settings", "film", "sampler"].map(Value::from)
    );
    // The camera reports no film component of its own.
    let camera_node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":camera.id}),
    );
    assert_eq!(
        kinds(&camera_node),
        ["transform", "camera", "medium"].map(Value::from)
    );
}

#[test]
fn film_without_a_camera_remains_editable_on_the_scene_root() {
    let film = Node::new(
        "Film",
        NodeKind::Configuration {
            configuration: ConfigurationKind::Film,
            implementation: "rgb".into(),
            parameters: Vec::new(),
        },
    );
    let document = document(&[&film], &[], vec![film.clone()]);
    let scene = document.default_scene.unwrap();
    let mut core = loaded(document);
    let root = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    // The scene box, render settings, and the camera-less Film.
    assert_eq!(root["components"].as_array().unwrap().len(), 3);
    assert_eq!(
        component(&root, "scene")["scene_id"],
        serde_json::json!(scene)
    );
    assert_eq!(component(&root, "film")["node_id"], id(&film));
    // the render settings box says which render entities the
    // Options hold; Add Component still reads that map with the Film on
    // the Scene root like everything else.
    assert_eq!(
        component(&root, "render_settings")["configurations"],
        serde_json::json!({
            "film": true, "sampler": false, "integrator": false,
            "pixel_filter": false, "accelerator": false,
        })
    );
}

#[test]
fn create_preset_returns_created_ids() {
    let target = entity("Target", EntityKind::Shape, "sphere");
    let document = document(&[], &[&target], vec![target.clone()]);
    let world = document.default_scene_view().unwrap().scene.world;
    let scene = document.default_scene.unwrap();
    let mut core = loaded(document);

    let presets = ok(&mut core, Method::ProjectListPresets, json!({}))["presets"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(presets.len(), 15);
    let names = presets
        .iter()
        .map(|preset| preset["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            "group.empty",
            "camera.perspective",
            "shape.cube",
            "shape.cylinder",
            "shape.disk",
            "shape.floor",
            "shape.plane",
            "shape.sphere",
            "light.distant",
            "light.infinite",
            "light.point",
            "light.spot",
            "area.quad",
            "area.sphere",
            "instance.object",
        ]
    );
    let quad = presets
        .iter()
        .find(|preset| preset["name"] == "area.quad")
        .unwrap();
    assert_eq!(
        quad["creates"],
        json!(["attribute", "area_light:diffuse", "shape:bilinearmesh"])
    );
    assert_eq!(quad["builtin"], true);

    let sphere = project::presets::SHAPE_SPHERE.id.0;
    let mut created = Vec::new();
    for _ in 0..2 {
        let result = ok(
            &mut core,
            Method::ProjectApply,
            json!({"edits":[{"kind":"create_preset","preset":sphere,"parent":scene.0}]}),
        );
        let nodes = result["created"].as_array().unwrap().clone();
        assert_eq!(nodes.len(), 2, "{result}");
        assert!(nodes.iter().all(|node| node["kind"] == "node"));
        created.push(nodes);
    }
    let labels = created
        .iter()
        .map(|nodes| {
            ok(
                &mut core,
                Method::ProjectGetNode,
                json!({"node_id":nodes[0]["id"]}),
            )["label"]
                .clone()
        })
        .collect::<Vec<_>>();
    assert_eq!(labels, [json!("Sphere"), json!("Sphere (1)")]);
    // The created shape has its own group and no transform yet.
    let shape = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":created[0][1]["id"]}),
    );
    let transform = component(&shape, "transform");
    assert_eq!(transform["operations"], json!([]));
    // The preset's dedicated group sits directly under the World, so its
    // local composes against the root.
    assert_eq!(transform["local"]["parent"], "root");

    let error = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"create_preset","preset":sphere,"parent":target.id}]}),
    )
    .unwrap_err();
    assert_eq!(error.details.unwrap()["reason"], "parent_not_group");
    let error = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"create_preset","preset":Uuid::new_v4(),"parent":world.0}]}),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn apply_rejects_edits_that_leave_the_project_non_exportable() {
    let sphere = entity("Sphere", EntityKind::Shape, "sphere");
    let mut doc = document(&[], &[&sphere], vec![sphere.clone()]);
    let resource = project::Resource {
        id: project::ResourceId::new(),
        label: "matte".into(),
        location: "Materials/matte.material".into(),
        edition: project::Revision::initial(),
        kind: project::ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let resource_id = resource.id.0.to_string();
    doc.resources.insert(resource.id, resource);
    let mut core = loaded(doc);
    let before = core.projects.reference().unwrap();
    // A string parameter the export schema does not manage used to apply
    // fine and broke export until the next save/export failed.
    let error = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[
            {"kind":"set_parameter","target":{"kind":"resource","id":resource_id},
             "name":"surprise","value_type":"string",
             "value":{"name":"surprise","value_type":"string","color_space":null,
                      "values":{"kind":"strings","value":["oops"]}}}
        ]}),
    )
    .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.details.clone().unwrap()["reason"], "not_exportable");
    assert!(
        error.message.contains(
            "uses the \"surprise\" string parameter, which the importer does not support"
        ),
        "unexpected error: {error:?}"
    );
    // The real session is untouched: the revision did not move, and later
    // valid edits still apply.
    let after = core.projects.reference().unwrap();
    assert_eq!(before.revision, after.revision);
    assert_eq!(before.activation, after.activation);
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"rename_node","node_id":id(&sphere),"label":"Renamed"}]}),
    );
}

#[test]
fn a_declared_alpha_mask_is_its_own_component() {
    // The user's 2026-10-29 : alpha / shadowalpha form an Alpha Mask
    // component while either is declared; the desktop adds and removes it
    // with set_parameter.
    let mut masked = entity("Masked", EntityKind::Shape, "sphere");
    let NodeKind::Entity { parameters, .. } = &mut masked.data else {
        unreachable!()
    };
    parameters.push(Parameter::floats("radius", [1.]));
    parameters.push(Parameter::floats("shadowalpha", [0.5]));
    let plain = entity("Plain", EntityKind::Shape, "sphere");
    let document = document(&[], &[&masked, &plain], vec![masked.clone(), plain.clone()]);
    let mut core = loaded(document);
    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":masked.id}),
    );
    let mask = component(&node, "alpha_mask");
    assert_eq!(
        mask["parameter_target"],
        json!({"kind":"node","id":masked.id})
    );
    assert_eq!(
        mask["declared"],
        json!([{"name":"shadowalpha","value_type":"float"}])
    );
    let kinds = node["components"]
        .as_array()
        .unwrap()
        .iter()
        .map(|component| component["kind"].clone())
        .collect::<Vec<_>>();
    assert_eq!(kinds[1], "shape");
    assert_eq!(kinds[2], "alpha_mask", "right after the shape body");
    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":plain.id}),
    );
    assert!(
        node["components"]
            .as_array()
            .unwrap()
            .iter()
            .all(|component| component["kind"] != "alpha_mask")
    );
}

/// G10 /: the transform component's local matrix and TRS. Each case
/// checks `ParentWorld × Local = World` against the evaluated world, and the
/// reasons reported where a local matrix or a TRS does not exist.
#[test]
fn transform_components_report_the_local_matrix_and_trs() {
    use project::{ActiveSamples, math::Mat4, transform_trs};
    let t = |v: [f64; 3]| {
        state(
            "t",
            StateOperation::Transform(TransformOperation::Translate(v)),
        )
    };
    let rotate = |degrees: f64, axis: [f64; 3]| {
        state(
            "r",
            StateOperation::Transform(TransformOperation::Rotate { degrees, axis }),
        )
    };
    let scale = |v: [f64; 3]| state("s", StateOperation::Transform(TransformOperation::Scale(v)));
    let shape = |label: &str| entity(label, EntityKind::Shape, "sphere");
    // Parent P moves and turns; D is the mirrored child's dedicated group.
    let (p_t, p_r) = (t([5., 0., 0.]), rotate(30., [0., 1., 0.]));
    let (d_t, d_r, d_s, a) = (
        t([1., 2., 3.]),
        rotate(45., [1., 0., 0.]),
        scale([1., -2., 1.]),
        shape("A"),
    );
    let d = owned_group("D", &[&d_t, &d_r, &d_s, &a], &a);
    let sibling = shape("Sibling");
    // E replaces the CTM (PBRT Identity) under the moved parent.
    let (e_i, e_t, e) = (
        state("i", StateOperation::Transform(TransformOperation::Identity)),
        t([0., 1., 0.]),
        shape("E"),
    );
    let e_group = owned_group("EG", &[&e_i, &e_t, &e], &e);
    let p = group("P", &[&p_t, &p_r, &d, &sibling, &e_group]);
    let mut shear = Mat4::identity().0;
    shear[1] = 0.5;
    let (f_c, f) = (
        state(
            "c",
            StateOperation::Transform(TransformOperation::Concat(shear)),
        ),
        shape("F"),
    );
    let f_group = owned_group("FG", &[&f_c, &f], &f);
    let define = state("define", StateOperation::DefineCoordinates("mark".into()));
    let (h_u, h) = (
        state("use", StateOperation::UseCoordinates("mark".into())),
        shape("H"),
    );
    let h_group = owned_group("HG", &[&h_u, &h], &h);
    let (k_a, k_t, k_b, k) = (
        state("end", StateOperation::ActiveTransform(ActiveSamples::End)),
        t([1., 0., 0.]),
        state("all", StateOperation::ActiveTransform(ActiveSamples::All)),
        shape("K"),
    );
    let k_group = owned_group("KG", &[&k_a, &k_t, &k_b, &k], &k);
    let (z_s, z) = (scale([0., 1., 1.]), shape("Z"));
    let z_group = owned_group("ZG", &[&z_s, &z], &z);
    let look = state(
        "look",
        StateOperation::Transform(TransformOperation::LookAt([
            0., 1., 5., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let camera = entity("Camera", EntityKind::Camera, "perspective");
    let nodes: Vec<Node> = [
        &p_t, &p_r, &d_t, &d_r, &d_s, &a, &d, &sibling, &e_i, &e_t, &e, &e_group, &p, &f_c, &f,
        &f_group, &define, &h_u, &h, &h_group, &k_a, &k_t, &k_b, &k, &k_group, &z_s, &z, &z_group,
        &look, &camera,
    ]
    .into_iter()
    .cloned()
    .collect();
    let mut core = loaded(document(
        &[&look, &camera],
        &[&p, &f_group, &define, &h_group, &k_group, &z_group],
        nodes,
    ));
    let transform = |core: &mut CoreState, node: &Node| {
        let node = ok(core, Method::ProjectGetNode, json!({"node_id":node.id}));
        component(&node, "transform").clone()
    };
    let matrix = |value: &Value| Mat4(std::array::from_fn(|i| value[i].as_f64().unwrap()));
    let world = |value: &Value| matrix(&value["world"]["start"]);
    let close = |a: &[f64], b: &[f64]| a.iter().zip(b).all(|(a, b)| (a - b).abs() < 1e-9);

    let parent = transform(&mut core, &p);
    let local = &parent["local"];
    let trs: transform_trs::Trs = transform_trs::Trs {
        translation: serde_json::from_value(local["trs"]["translation"].clone()).unwrap(),
        rotation_euler_degrees: serde_json::from_value(
            local["trs"]["rotation_euler_degrees"].clone(),
        )
        .unwrap(),
        scale: serde_json::from_value(local["trs"]["scale"].clone()).unwrap(),
    };
    assert!(close(&trs.translation, &[5., 0., 0.]), "{local}");
    assert!(
        close(&trs.rotation_euler_degrees, &[0., 30., 0.]),
        "{local}"
    );
    assert!(close(&trs.scale, &[1., 1., 1.]), "{local}");
    assert_eq!(local["trs"]["rotation_order"], "zxy");
    let parent_world = world(&parent);

    // The mirrored child: its local is its own run, and P × L = W.
    for node in [&a, &d] {
        let found = transform(&mut core, node);
        let l = matrix(&found["local"]["matrix"]);
        assert!(found["local"]["samples"].is_null(), "{found}");
        let expected = Mat4::translation(1., 2., 3.)
            .compose(Mat4::rotation(45., 1., 0., 0.))
            .compose(Mat4::scale(1., -2., 1.));
        assert!(transform_trs::approx_eq(&l, &expected), "{found}");
        assert!(
            transform_trs::approx_eq(&parent_world.compose(l), &world(&found)),
            "{found}"
        );
        let scale = &found["local"]["trs"]["scale"];
        assert!(
            scale[0].as_f64().unwrap() < 0.,
            "the mirror goes into X: {found}"
        );
    }
    // Without a dedicated group the object adds nothing.
    let found = transform(&mut core, &sibling);
    assert!(close(
        &serde_json::from_value::<Vec<f64>>(found["local"]["matrix"].clone()).unwrap(),
        &Mat4::identity().0
    ));
    assert!(transform_trs::approx_eq(&world(&found), &parent_world));
    // An `Identity` under a moved parent is an older import's shape:
    // the local stays group-relative and `P × Local = World` still holds.
    let found = transform(&mut core, &e);
    assert_eq!(found["local"]["parent"], "group", "{found}");
    let l = matrix(&found["local"]["matrix"]);
    assert!(
        transform_trs::approx_eq(&parent_world.compose(l), &world(&found)),
        "{found}"
    );
    assert!(transform_trs::approx_eq(
        &world(&found),
        &Mat4::translation(0., 1., 0.)
    ));
    assert!(found["local"]["trs"].is_object(), "{found}");
    assert_eq!(transform(&mut core, &d)["local"]["parent"], "group");

    let found = transform(&mut core, &f);
    assert!(found["local"]["matrix"].is_array());
    assert_eq!(found["local"]["trs"], Value::Null);
    assert_eq!(found["local"]["trs_reason"], "shear");
    assert_eq!(found["world_trs_reason"], "shear");
    let found = transform(&mut core, &h);
    assert_eq!(found["local"]["matrix"], Value::Null);
    assert_eq!(found["local"]["matrix_reason"], "coordinate_system");
    let found = transform(&mut core, &k);
    assert_eq!(found["local"]["matrix_reason"], "animated");
    assert_eq!(found["world_trs_reason"], "animated");
    let found = transform(&mut core, &z);
    assert_eq!(found["local"]["trs_reason"], "singular");

    // A camera shows its camera-to-world transform: the eye's position.
    let found = transform(&mut core, &camera);
    let position = &found["local"]["trs"]["translation"];
    assert!(
        close(
            &serde_json::from_value::<Vec<f64>>(position.clone()).unwrap(),
            &[0., 1., 5.]
        ),
        "{found}"
    );
}

#[test]
fn animated_transform_samples_are_local_and_use_endpoint_specific_roots() {
    use project::{ActiveSamples, math::Mat4};
    let times = state("TransformTimes", StateOperation::TransformTimes([0.2, 0.8]));
    let look = state(
        "LookAt",
        StateOperation::Transform(TransformOperation::LookAt([
            0., 0., 12., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let camera = entity("Camera", EntityKind::Camera, "perspective");

    // Both the parent and child's Transform samples move. The local values
    // remain -1 and +1 instead of displaying the child's world positions.
    let parent_start = state(
        "Parent Start",
        StateOperation::ActiveTransform(ActiveSamples::Start),
    );
    let parent_start_move = state(
        "Parent Start Move",
        StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
    );
    let parent_end = state(
        "Parent End",
        StateOperation::ActiveTransform(ActiveSamples::End),
    );
    let parent_end_move = state(
        "Parent End Move",
        StateOperation::Transform(TransformOperation::Translate([8., 0., 0.])),
    );
    let parent_all = state(
        "Parent All",
        StateOperation::ActiveTransform(ActiveSamples::All),
    );
    let child_start = state(
        "Child Start",
        StateOperation::ActiveTransform(ActiveSamples::Start),
    );
    let child_start_move = state(
        "Child Start Move",
        StateOperation::Transform(TransformOperation::Translate([-1., 0., 0.])),
    );
    let child_end = state(
        "Child End",
        StateOperation::ActiveTransform(ActiveSamples::End),
    );
    let child_end_move = state(
        "Child End Move",
        StateOperation::Transform(TransformOperation::Translate([1., 0., 0.])),
    );
    let child_all = state(
        "Child All",
        StateOperation::ActiveTransform(ActiveSamples::All),
    );
    let child_shape = entity("Child", EntityKind::Shape, "sphere");
    let child_group = owned_group(
        "Child Group",
        &[
            &child_start,
            &child_start_move,
            &child_end,
            &child_end_move,
            &child_all,
            &child_shape,
        ],
        &child_shape,
    );
    let parent = group(
        "Parent",
        &[
            &parent_start,
            &parent_start_move,
            &parent_end,
            &parent_end_move,
            &parent_all,
            &child_group,
        ],
    );

    // A mixed run is relative at Start but replaces the CTM at End. Only the
    // End sample gets root semantics.
    let root_start = state(
        "Root Start",
        StateOperation::ActiveTransform(ActiveSamples::Start),
    );
    let root_start_move = state(
        "Root Start Move",
        StateOperation::Transform(TransformOperation::Translate([1., 0., 0.])),
    );
    let root_end = state(
        "Root End",
        StateOperation::ActiveTransform(ActiveSamples::End),
    );
    let root_end_replace = state(
        "Root End Replace",
        StateOperation::Transform(TransformOperation::Replace(
            Mat4::translation(30., 0., 0.).0,
        )),
    );
    let root_all = state(
        "Root All",
        StateOperation::ActiveTransform(ActiveSamples::All),
    );
    let root_shape = entity("Root Reset", EntityKind::Shape, "sphere");
    let root_group = owned_group(
        "Mixed Reset Group",
        &[
            &root_start,
            &root_start_move,
            &root_end,
            &root_end_replace,
            &root_all,
            &root_shape,
        ],
        &root_shape,
    );
    let world_parent = group("World Parent", &[&parent, &root_group]);

    let nodes = [
        &times,
        &look,
        &camera,
        &parent_start,
        &parent_start_move,
        &parent_end,
        &parent_end_move,
        &parent_all,
        &child_start,
        &child_start_move,
        &child_end,
        &child_end_move,
        &child_all,
        &child_shape,
        &child_group,
        &parent,
        &root_start,
        &root_start_move,
        &root_end,
        &root_end_replace,
        &root_all,
        &root_shape,
        &root_group,
        &world_parent,
    ]
    .into_iter()
    .cloned()
    .collect();
    let mut core = loaded(document(&[&times, &look, &camera], &[&world_parent], nodes));
    let transform = |core: &mut CoreState, node: &Node| {
        let details = ok(core, Method::ProjectGetNode, json!({"node_id":node.id}));
        component(&details, "transform").clone()
    };
    let child = transform(&mut core, &child_shape);
    assert_eq!(child["local"]["matrix"], Value::Null);
    assert_eq!(child["local"]["matrix_reason"], "animated");
    assert_eq!(child["local"]["samples"]["times"], json!([0.2, 0.8]));
    assert_eq!(
        child["local"]["samples"]["start"]["trs"]["translation"][0],
        -1.
    );
    assert_eq!(
        child["local"]["samples"]["end"]["trs"]["translation"][0],
        1.
    );
    assert_eq!(
        child["local"]["samples"]["start"]["matrix_reason"],
        Value::Null
    );
    assert_eq!(
        child["local"]["samples"]["end"]["matrix_reason"],
        Value::Null
    );

    let root = transform(&mut core, &root_shape);
    assert_eq!(root["local"]["matrix_reason"], "animated");
    assert_eq!(
        root["local"]["samples"]["start"]["trs"]["translation"][0],
        1.
    );
    assert_eq!(
        root["local"]["samples"]["end"]["trs"]["translation"][0],
        30.
    );
    // the run's `Transform` reset belongs to an older import; both
    // endpoints read through the ordinary group relation.
    assert_eq!(root["local"]["samples"]["start"]["parent"], "group");
    assert_eq!(root["local"]["samples"]["end"]["parent"], "group");

    let rejected = call(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_trs","node_id":child_shape.id,
            "translation":[0.0,0.0,0.0],"rotation_euler_degrees":[0.0,0.0,0.0],
            "scale":[1.0,1.0,1.0]}]}),
    );
    assert!(rejected.is_err(), "animated local edits remain unsupported");
}

#[test]
fn animated_camera_shear_and_singular_parent_samples_are_reported_per_endpoint() {
    use project::{ActiveSamples, math::Mat4};
    let times = state("TransformTimes", StateOperation::TransformTimes([2., 5.]));
    let look_start = state(
        "Camera Start",
        StateOperation::ActiveTransform(ActiveSamples::Start),
    );
    let look_start_pose = state(
        "Camera Start Pose",
        StateOperation::Transform(TransformOperation::LookAt([
            0., 0., 5., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let look_end = state(
        "Camera End",
        StateOperation::ActiveTransform(ActiveSamples::End),
    );
    let look_end_pose = state(
        "Camera End Pose",
        StateOperation::Transform(TransformOperation::LookAt([
            3., 1., 7., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let camera_all = state(
        "Camera All",
        StateOperation::ActiveTransform(ActiveSamples::All),
    );
    let camera = entity("Animated Camera", EntityKind::Camera, "perspective");

    let shear_group = {
        let mut shear = Mat4::identity().0;
        shear[1] = 0.5;
        let start = state(
            "Shear Start",
            StateOperation::ActiveTransform(ActiveSamples::Start),
        );
        let start_shear = state(
            "Start Shear",
            StateOperation::Transform(TransformOperation::Concat(shear)),
        );
        let end = state(
            "Shear End",
            StateOperation::ActiveTransform(ActiveSamples::End),
        );
        let end_move = state(
            "End Move",
            StateOperation::Transform(TransformOperation::Translate([2., 0., 0.])),
        );
        let all = state(
            "Shear All",
            StateOperation::ActiveTransform(ActiveSamples::All),
        );
        let shape = entity("Shear Animated", EntityKind::Shape, "sphere");
        let group = owned_group(
            "Shear Group",
            &[&start, &start_shear, &end, &end_move, &all, &shape],
            &shape,
        );
        (start, start_shear, end, end_move, all, shape, group)
    };

    let singular_parent = {
        let start = state(
            "Singular Start",
            StateOperation::ActiveTransform(ActiveSamples::Start),
        );
        let start_scale = state(
            "Singular Scale",
            StateOperation::Transform(TransformOperation::Scale([0., 1., 1.])),
        );
        let end = state(
            "Singular End",
            StateOperation::ActiveTransform(ActiveSamples::End),
        );
        let end_move = state(
            "Parent End Move",
            StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
        );
        let all = state(
            "Singular All",
            StateOperation::ActiveTransform(ActiveSamples::All),
        );
        let child_start = state(
            "Local Start",
            StateOperation::ActiveTransform(ActiveSamples::Start),
        );
        let child_start_move = state(
            "Local Start Move",
            StateOperation::Transform(TransformOperation::Translate([2., 0., 0.])),
        );
        let child_end = state(
            "Local End",
            StateOperation::ActiveTransform(ActiveSamples::End),
        );
        let child_end_move = state(
            "Local End Move",
            StateOperation::Transform(TransformOperation::Translate([3., 0., 0.])),
        );
        let child_all = state(
            "Local All",
            StateOperation::ActiveTransform(ActiveSamples::All),
        );
        let shape = entity("Singular Child", EntityKind::Shape, "sphere");
        let child = owned_group(
            "Singular Child Group",
            &[
                &child_start,
                &child_start_move,
                &child_end,
                &child_end_move,
                &child_all,
                &shape,
            ],
            &shape,
        );
        let parent = group(
            "Singular Parent",
            &[&start, &start_scale, &end, &end_move, &all, &child],
        );
        (
            start,
            start_scale,
            end,
            end_move,
            all,
            child_start,
            child_start_move,
            child_end,
            child_end_move,
            child_all,
            shape,
            child,
            parent,
        )
    };

    let nodes = [
        &times,
        &look_start,
        &look_start_pose,
        &look_end,
        &look_end_pose,
        &camera_all,
        &camera,
        &shear_group.0,
        &shear_group.1,
        &shear_group.2,
        &shear_group.3,
        &shear_group.4,
        &shear_group.5,
        &shear_group.6,
        &singular_parent.0,
        &singular_parent.1,
        &singular_parent.2,
        &singular_parent.3,
        &singular_parent.4,
        &singular_parent.5,
        &singular_parent.6,
        &singular_parent.7,
        &singular_parent.8,
        &singular_parent.9,
        &singular_parent.10,
        &singular_parent.11,
        &singular_parent.12,
    ]
    .into_iter()
    .cloned()
    .collect();
    let mut core = loaded(document(
        &[
            &times,
            &look_start,
            &look_start_pose,
            &look_end,
            &look_end_pose,
            &camera_all,
            &camera,
        ],
        &[&shear_group.6, &singular_parent.12],
        nodes,
    ));
    let transform = |core: &mut CoreState, node: &Node| {
        let details = ok(core, Method::ProjectGetNode, json!({"node_id":node.id}));
        component(&details, "transform").clone()
    };
    let camera_view = transform(&mut core, &camera);
    let samples = &camera_view["local"]["samples"];
    assert_eq!(camera_view["local"]["matrix_reason"], "animated");
    assert_eq!(samples["times"], json!([2., 5.]));
    assert_eq!(samples["start"]["trs"]["translation"], json!([0., 0., 5.]));
    assert_eq!(samples["end"]["trs"]["translation"], json!([3., 1., 7.]));

    let shear = transform(&mut core, &shear_group.5);
    assert_eq!(shear["local"]["matrix_reason"], "animated");
    assert_eq!(shear["local"]["samples"]["start"]["trs"], Value::Null);
    assert_eq!(shear["local"]["samples"]["start"]["trs_reason"], "shear");
    assert!(shear["local"]["samples"]["start"]["matrix"].is_array());
    assert!(shear["local"]["samples"]["end"]["trs"].is_object());

    let singular = transform(&mut core, &singular_parent.10);
    assert_eq!(singular["local"]["matrix_reason"], "animated");
    assert_eq!(singular["local"]["samples"]["start"]["matrix"], Value::Null);
    assert_eq!(
        singular["local"]["samples"]["start"]["matrix_reason"],
        "singular"
    );
    assert!(singular["local"]["samples"]["end"]["matrix"].is_array());
    assert_eq!(
        singular["local"]["samples"]["end"]["trs"]["translation"][0],
        3.
    );
}

/// A preset adds no PBRT `Identity`, so under a moved parent it inherits the
/// parent's transform and its local is the identity.
#[test]
fn presets_inherit_their_parents_transform() {
    use project::math::Mat4;
    let moved = state(
        "move",
        StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
    );
    let parent = group("Parent", &[&moved]);
    let mut core = loaded(document(
        &[],
        &[&parent],
        vec![moved.clone(), parent.clone()],
    ));
    let result = ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"create_preset","preset":project::presets::SHAPE_SPHERE.id.0,"parent":parent.id}]}),
    );
    let group_id = result["created"][0]["id"].clone();
    let node = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":group_id}),
    );
    let transform = component(&node, "transform");
    let world: Vec<f64> = serde_json::from_value(transform["world"]["start"].clone()).unwrap();
    assert_eq!(world, Mat4::translation(5., 0., 0.).0, "{transform}");
    assert_eq!(transform["local"]["parent"], "group");
    assert_eq!(transform["local"]["matrix"], json!(Mat4::identity().0));
}

/// `set_local_trs` / `set_local_matrix` over the wire; Camera edits
/// expose camera-to-world pose and write the inverse view matrix to Options.
#[test]
fn set_local_trs_and_matrix_round_trip_through_get_node() {
    use project::math::Mat4;
    let moved = state(
        "move",
        StateOperation::Transform(TransformOperation::Translate([5., 0., 0.])),
    );
    let (a, b) = (
        entity("A", EntityKind::Shape, "sphere"),
        entity("B", EntityKind::Shape, "sphere"),
    );
    let parent = group("Parent", &[&moved, &a, &b]);
    let look = state(
        "look",
        StateOperation::Transform(TransformOperation::LookAt([
            0., 1., 5., 0., 0., 0., 0., 1., 0.,
        ])),
    );
    let camera = posed_camera("Camera", "perspective");
    let mut core = loaded(document(
        &[&look, &camera],
        &[&parent],
        vec![
            moved.clone(),
            a.clone(),
            b.clone(),
            parent.clone(),
            look.clone(),
            camera.clone(),
        ],
    ));
    let local = |core: &mut CoreState, id| {
        let node = ok(core, Method::ProjectGetNode, json!({"node_id":id}));
        component(&node, "transform").clone()
    };
    let b_before = local(&mut core, b.id)["world"].clone();
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_trs","node_id":a.id,
            "translation":[1.0,2.0,3.0],"rotation_euler_degrees":[10.0,20.0,30.0],
            "scale":[1.0,2.0,1.0]}]}),
    );
    let found = local(&mut core, a.id);
    assert_eq!(found["local"]["parent"], "group");
    let trs = &found["local"]["trs"];
    let close = |value: &Value, expected: [f64; 3]| {
        (0..3).all(|i| (value[i].as_f64().unwrap() - expected[i]).abs() < 1e-9)
    };
    assert!(close(&trs["translation"], [1., 2., 3.]), "{found}");
    assert!(
        close(&trs["rotation_euler_degrees"], [10., 20., 30.]),
        "{found}"
    );
    assert!(close(&trs["scale"], [1., 2., 1.]), "{found}");
    assert_eq!(local(&mut core, b.id)["world"], b_before);

    let mut shear = Mat4::identity().0;
    shear[1] = 0.5;
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_matrix","node_id":a.id,"matrix":shear}]}),
    );
    let found = local(&mut core, a.id);
    assert_eq!(found["local"]["matrix"], json!(shear));
    assert_eq!(found["local"]["trs_reason"], "shear");

    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_local_trs","node_id":camera.id,
            "translation":[3.0,2.0,1.0],"rotation_euler_degrees":[10.0,20.0,30.0],
            "scale":[1.0,1.0,1.0]}]}),
    );
    let camera_transform = local(&mut core, camera.id);
    let camera_position: [f64; 3] =
        serde_json::from_value(camera_transform["local"]["trs"]["translation"].clone()).unwrap();
    let camera_rotation: [f64; 3] =
        serde_json::from_value(camera_transform["local"]["trs"]["rotation_euler_degrees"].clone())
            .unwrap();
    assert!(
        camera_position
            .iter()
            .zip([3., 2., 1.])
            .all(|(a, b)| (a - b).abs() < 1e-9)
    );
    assert!(
        camera_rotation
            .iter()
            .zip([10., 20., 30.])
            .all(|(a, b)| (a - b).abs() < 1e-8)
    );
    assert_eq!(camera_transform["local"]["parent"], "root");
    assert_eq!(local(&mut core, b.id)["world"], b_before);
}

#[test]
fn owned_wrappers_stop_group_transform_scans() {
    use project::math::Mat4;
    let first = entity("First", EntityKind::Shape, "sphere");
    let wrapper = owned_group("Internal", &[&first], &first);
    let later_move = state(
        "Later move",
        StateOperation::Transform(TransformOperation::Translate([7., 0., 0.])),
    );
    let second = entity("Second", EntityKind::Shape, "sphere");
    let parent = group("Parent", &[&wrapper, &later_move, &second]);
    let mut core = loaded(document(
        &[],
        &[&parent],
        vec![
            first.clone(),
            wrapper,
            later_move,
            second.clone(),
            parent.clone(),
        ],
    ));
    for (id, expected) in [
        (parent.id, Mat4::identity()),
        (first.id, Mat4::identity()),
        (second.id, Mat4::translation(7., 0., 0.)),
    ] {
        let details = ok(&mut core, Method::ProjectGetNode, json!({"node_id":id}));
        let transform = component(&details, "transform");
        assert_eq!(transform["world"]["start"], json!(expected.0));
        if id == parent.id {
            assert_eq!(transform["local"]["matrix"], json!(Mat4::identity().0));
        }
    }
}

#[test]
fn the_scene_root_reports_and_edits_its_coordinate_system() {
    let document = document(&[], &[], vec![]);
    let scene = document.default_scene.unwrap();
    let mut core = loaded(document);
    let plain = serde_json::json!({"up":"+Y","handedness":"left"});
    let tilted = serde_json::json!({"up":"+Z","handedness":"right"});

    // The scene root row, the node details and the scene list all report
    // the scene's coordinate system.
    let rows = ok(
        &mut core,
        Method::ProjectListNodes,
        json!({"view":"scene_graph"}),
    )["nodes"]
        .as_array()
        .unwrap()
        .clone();
    let root_row = rows
        .iter()
        .find(|row| row["kind"] == "scene")
        .expect("the scene root row is listed");
    assert_eq!(root_row["coordinate_system"], plain);

    let details = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    let scene_component = component(&details, "scene");
    assert_eq!(scene_component["scene_id"], serde_json::json!(scene));
    assert_eq!(scene_component["coordinate_system"], plain);

    let scenes = ok(&mut core, Method::ProjectListScenes, json!({}))["scenes"]
        .as_array()
        .unwrap()
        .clone();
    assert_eq!(scenes[0]["coordinate_system"], plain);

    // The apply edit moves the metadata and undo returns it.
    ok(
        &mut core,
        Method::ProjectApply,
        json!({"edits":[{"kind":"set_scene_coordinate_system","scene":scene,
            "up":"+Z","handedness":"right"}]}),
    );
    let details = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(component(&details, "scene")["coordinate_system"], tilted);
    ok(&mut core, Method::ProjectUndo, json!({}));
    let details = ok(&mut core, Method::ProjectGetNode, json!({"node_id":scene}));
    assert_eq!(component(&details, "scene")["coordinate_system"], plain);

    // An unknown scene cannot be edited.
    assert!(
        call(
            &mut core,
            Method::ProjectApply,
            json!({"edits":[{"kind":"set_scene_coordinate_system","scene":Uuid::new_v4(),
                "up":"-Y","handedness":"left"}]}),
        )
        .is_err()
    );

    // The wire spelling is validated, not passed through.
    assert!(
        call(
            &mut core,
            Method::ProjectApply,
            json!({"edits":[{"kind":"set_scene_coordinate_system","scene":scene,
                "up":"+y","handedness":"left"}]}),
        )
        .is_err()
    );
}

#[test]
fn get_node_names_the_object_definition_that_holds_a_definition_node() {
    let inner = entity("Inner", EntityKind::Shape, "sphere");
    let nested = group("Nested", &[&inner]);
    let scope = Node::new(
        "Definition",
        NodeKind::Scope {
            scope: ScopeKind::ObjectDefinition,
            children: vec![nested.id],
            owned_object: None,
            parent_camera: None,
        },
    );
    let world_shape = entity("Sphere", EntityKind::Shape, "sphere");
    let mut document = document(&[], &[&world_shape], vec![world_shape.clone()]);
    for node in [inner.clone(), nested.clone(), scope.clone()] {
        document.definition_nodes.insert(node.id, node);
    }
    let definition = project::Resource {
        id: project::ResourceId::new(),
        label: "Object".into(),
        location: "Objects/definition.object".into(),
        edition: project::Revision::initial(),
        kind: project::ResourceKind::ObjectDefinition,
        texture_value_type: None,
        implementation: String::new(),
        parameters: Vec::new(),
        definition_scope: Some(scope.id),
        declared_in: None,
        origin: None,
    };
    let definition_id = definition.id;
    document.resources.insert(definition_id, definition);
    document.validate().unwrap();
    let mut core = loaded(document);

    for node in [&inner, &nested, &scope] {
        let detail = ok(
            &mut core,
            Method::ProjectGetNode,
            json!({"node_id":node.id}),
        );
        assert_eq!(detail["definition"], json!(definition_id), "{}", node.label);
    }
    let detail = ok(
        &mut core,
        Method::ProjectGetNode,
        json!({"node_id":world_shape.id}),
    );
    assert!(detail["definition"].is_null());
}
