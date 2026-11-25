//! Fixed built-in node presets shared by project clients.
use crate::math::{Mat4, inverse};
use crate::{
    CameraPose, CoordinateSystem, EntityKind, FloatValue, Node, NodeKind, Parameter,
    ParameterValues, ResourceId, StateOperation, TransformOperation,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BuiltinPreset {
    pub id: ResourceId,
    pub name: &'static str,
    pub category: &'static str,
    pub label: &'static str,
    pub implementation: &'static str,
}

/// The viewport view a caller wants a new Camera preset to take: where
/// its eye sits and what it looks at. `look_at` is a point, not a
/// direction; the scene's up axis decides the roll.: the same pair
/// places a shape, light or area body — shapes use the look-at point
/// (and scale to a tenth of the eye's distance), while finite and area
/// lights are raised above it by a third of that distance.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PresetView {
    pub position: [f64; 3],
    pub look_at: [f64; 3],
}

/// whether a viewport view can place a preset body: finite eye and
/// look-at, with the eye off the look-at point — a zero distance would
/// scale a shape flat. The camera preset's own refusal
/// (`camera_pose`) instead checks the LookAt direction names an up.
pub(crate) fn placeable(view: PresetView) -> bool {
    let finite = view
        .position
        .iter()
        .chain(view.look_at.iter())
        .all(|value| value.is_finite());
    let delta = [
        view.position[0] - view.look_at[0],
        view.position[1] - view.look_at[1],
        view.position[2] - view.look_at[2],
    ];
    finite && (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]) > 0.
}

pub const GROUP_EMPTY: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("19500901-220e-4577-a08a-6fa4173fa347")),
    name: "group.empty",
    category: "group",
    label: "Group",
    implementation: "attribute",
};
pub const CAMERA_PERSPECTIVE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("266b209c-bbbf-4687-a697-483f7126d8ca")),
    name: "camera.perspective",
    category: "camera",
    label: "Camera",
    implementation: "perspective",
};
pub const SHAPE_SPHERE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("209d7c20-ca6d-4965-a471-405c97e13662")),
    name: "shape.sphere",
    category: "shape",
    label: "Sphere",
    implementation: "sphere",
};
pub const SHAPE_DISK: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("c5ea4a95-1821-4fd7-b569-acf6f0c75910")),
    name: "shape.disk",
    category: "shape",
    label: "Disk",
    implementation: "disk",
};
pub const SHAPE_CYLINDER: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("d90e15f3-5996-4368-ac2e-9945e6856e23")),
    name: "shape.cylinder",
    category: "shape",
    label: "Cylinder",
    implementation: "cylinder",
};
pub const SHAPE_PLANE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("767a280d-35eb-49bb-924a-b7dbb03f7568")),
    name: "shape.plane",
    category: "shape",
    label: "Plane",
    implementation: "bilinearmesh",
};
pub const SHAPE_FLOOR: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("f7e45351-5c8b-4756-a41b-4c1b9f782743")),
    name: "shape.floor",
    category: "shape",
    label: "Floor",
    implementation: "bilinearmesh",
};
pub const SHAPE_CUBE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("7ad28556-48d6-42d7-8bd9-2823cace6ad1")),
    name: "shape.cube",
    category: "shape",
    label: "Cube",
    implementation: "trianglemesh",
};
pub const LIGHT_POINT: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("80bf7ed4-21df-48f8-8715-bb1285ecd7b7")),
    name: "light.point",
    category: "light",
    label: "Point Light",
    implementation: "point",
};
pub const LIGHT_SPOT: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("023850b2-919c-4ea9-9119-1739c6955e55")),
    name: "light.spot",
    category: "light",
    label: "Spot Light",
    implementation: "spot",
};
pub const LIGHT_DISTANT: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("698b29cc-3eb4-493f-b922-0fe61c98d988")),
    name: "light.distant",
    category: "light",
    label: "Distant Light",
    implementation: "distant",
};
pub const LIGHT_INFINITE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("7b826e74-156d-4042-987d-4ec688620d13")),
    name: "light.infinite",
    category: "light",
    label: "Environment Light",
    implementation: "infinite",
};
pub const AREA_QUAD: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("ee016a71-134f-4eeb-8d71-ef1132f3125a")),
    name: "area.quad",
    category: "area",
    label: "Quad Area Light",
    implementation: "plane",
};
pub const AREA_SPHERE: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("53179d38-2337-474d-a861-fe1921021e47")),
    name: "area.sphere",
    category: "area",
    label: "Sphere Area Light",
    implementation: "sphere",
};
pub const INSTANCE_OBJECT: BuiltinPreset = BuiltinPreset {
    id: ResourceId(uuid::uuid!("a62d8a1c-f165-40f7-adef-c5d3bf027006")),
    name: "instance.object",
    category: "instance",
    label: "Object Instance",
    implementation: "object",
};

pub const BUILTIN_PRESETS: &[BuiltinPreset] = &[
    GROUP_EMPTY,
    CAMERA_PERSPECTIVE,
    SHAPE_SPHERE,
    SHAPE_DISK,
    SHAPE_CYLINDER,
    SHAPE_PLANE,
    SHAPE_FLOOR,
    SHAPE_CUBE,
    LIGHT_POINT,
    LIGHT_SPOT,
    LIGHT_DISTANT,
    LIGHT_INFINITE,
    AREA_QUAD,
    AREA_SPHERE,
    INSTANCE_OBJECT,
];

pub fn builtin(id: ResourceId) -> Option<&'static BuiltinPreset> {
    BUILTIN_PRESETS.iter().find(|preset| preset.id == id)
}

/// What `CreatePreset` builds, for listings: the camera preset lands in
/// the Scene's Options as a bare Camera; every other body sits in its own
/// group — the group, then each body node as `<entity>:<implementation>`
/// (or `instance`).
pub fn creates(preset: &BuiltinPreset) -> Vec<String> {
    if preset.id == CAMERA_PERSPECTIVE.id {
        return vec!["camera:perspective".to_owned()];
    }
    let mut created = vec!["attribute".to_owned()];
    if preset.id == INSTANCE_OBJECT.id {
        created.push("instance".into());
        return created;
    }
    created.extend(
        make_body(preset, CoordinateSystem::default(), None)
            .iter()
            .filter_map(|node| match &node.data {
                NodeKind::Entity {
                    entity,
                    implementation,
                    ..
                } => {
                    let entity = match entity {
                        EntityKind::Shape => "shape",
                        EntityKind::Light => "light",
                        EntityKind::AreaLight => "area_light",
                        EntityKind::Camera => "camera",
                    };
                    Some(format!("{entity}:{implementation}"))
                }
                _ => None,
            }),
    );
    created
}

pub(crate) fn make_body(
    preset: &BuiltinPreset,
    coordinate_system: CoordinateSystem,
    view: Option<PresetView>,
) -> Vec<Node> {
    if preset.name == "instance.object" || preset.name == "group.empty" {
        return Vec::new();
    }
    let (entity, implementation, mut parameters) = match preset.name {
        "camera.perspective" => (
            EntityKind::Camera,
            "perspective",
            vec![Parameter::floats("fov", [40.])],
        ),
        "shape.sphere" | "area.sphere" => (
            EntityKind::Shape,
            "sphere",
            vec![Parameter::floats("radius", [1.])],
        ),
        "shape.disk" => (
            EntityKind::Shape,
            "disk",
            vec![Parameter::floats("radius", [1.])],
        ),
        "shape.cylinder" => (
            EntityKind::Shape,
            "cylinder",
            vec![
                Parameter::floats("radius", [1.]),
                Parameter::floats("zmin", [-1.]),
                Parameter::floats("zmax", [1.]),
            ],
        ),
        // Bilinear patch corners p00, p10, p01, p11 with dp/du × dp/dv
        // up: the object-space normal is +Y and a rotation aligns it with
        // the scene's up axis.
        "shape.floor" => (EntityKind::Shape, "bilinearmesh", plane_points(5.)),
        "shape.plane" | "area.quad" => (EntityKind::Shape, "bilinearmesh", plane_points(0.5)),
        "shape.cube" => (EntityKind::Shape, "trianglemesh", cube_parameters()),
        "light.point" => (EntityKind::Light, "point", vec![rgb("I", [1., 1., 1.])]),
        "light.spot" => (
            EntityKind::Light,
            "spot",
            vec![
                rgb("I", [1., 1., 1.]),
                floats("from", "point3", [0., 0., 0.]),
                // down, along −up.
                floats(
                    "to",
                    "point3",
                    coordinate_system.up_vector().map(|value| -value),
                ),
                Parameter::floats("coneangle", [30.]),
                Parameter::floats("conedelta", [5.]),
            ],
        ),
        "light.distant" => (
            EntityKind::Light,
            "distant",
            vec![
                rgb("L", [1., 1., 1.]),
                floats("from", "point3", [0., 0., 0.]),
                floats(
                    "to",
                    "point3",
                    coordinate_system.up_vector().map(|value| -value),
                ),
            ],
        ),
        "light.infinite" => (
            EntityKind::Light,
            "infinite",
            vec![rgb("L", [0.5, 0.5, 0.5])],
        ),
        _ => return Vec::new(),
    };
    if let Some(view) = view
        && matches!(preset.name, "light.spot" | "light.distant")
    {
        let position = raised_light_position(view, coordinate_system.up_vector());
        if let Some(to) = parameters
            .iter_mut()
            .find(|parameter| parameter.name == "to")
        {
            to.values = ParameterValues::Floats(
                view.look_at
                    .iter()
                    .zip(position)
                    .map(|(target, origin)| FloatValue::Finite(target - origin))
                    .collect(),
            );
        }
    }
    // the oriented parts turn by a rotation in the preset group; the
    // camera carries its pose directly and spot/distant aim by from/to.
    let orientation = match preset.name {
        "shape.floor" | "shape.plane" => {
            align_rotation([0., 1., 0.], coordinate_system.up_vector())
        }
        "area.quad" => {
            let up = coordinate_system.up_vector();
            let direction = if view.is_some() {
                up.map(|axis| -axis)
            } else {
                up
            };
            align_rotation([0., 1., 0.], direction)
        }
        "shape.disk" | "shape.cylinder" => {
            align_rotation([0., 0., 1.], coordinate_system.up_vector())
        }
        _ => None,
    };
    let mut nodes = Vec::new();
    // with a viewport view the body opens with its placement, at
    // the start of the preset's own scope. `placeable` — called by the
    // session — guarantees the eye is finite and off the look-at point.
    // The camera preset uses the view for its pose instead.
    if preset.name != "camera.perspective"
        && let Some(view) = view
    {
        let tenth = view_scale(view);
        let placement = if matches!(
            preset.name,
            "light.point" | "light.spot" | "light.distant" | "area.quad" | "area.sphere"
        ) {
            raised_light_position(view, coordinate_system.up_vector())
        } else {
            view.look_at
        };
        nodes.push(Node::new(
            "Translate",
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate(placement)),
            },
        ));
        // Shapes and area-light bodies size to the view. Point and directional
        // lights carry their own reach, and the camera takes its pose above.
        if matches!(preset.category, "shape" | "area") {
            nodes.push(Node::new(
                "Scale",
                NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Scale([
                        tenth, tenth, tenth,
                    ])),
                },
            ));
        }
    }
    if let Some(rotation) = orientation {
        nodes.push(Node::new(
            "Rotation",
            NodeKind::State {
                operation: StateOperation::Transform(rotation),
            },
        ));
    }
    if preset.category == "area" {
        // Without a view the body keeps its unit size and no distance is known.
        let scale = if view.is_some() {
            area_light_scale(preset.name)
        } else {
            1.
        };
        nodes.push(Node::new(
            "Diffuse Area Light",
            NodeKind::Entity {
                entity: EntityKind::AreaLight,
                implementation: "diffuse".into(),
                parameters: vec![rgb("L", [1., 1., 1.]), Parameter::floats("scale", [scale])],
                pose: None,
            },
        ));
    }
    let pose = if preset.name == "camera.perspective" {
        Some(camera_pose(coordinate_system, view).expect("the session validated the placement"))
    } else {
        None
    };
    nodes.push(Node::new(
        preset.label,
        NodeKind::Entity {
            entity,
            implementation: implementation.into(),
            parameters,
            pose,
        },
    ));
    nodes
}

/// A placeable shape or area-light body is one tenth of its view distance.
fn view_scale(view: PresetView) -> f64 {
    let delta = [
        view.position[0] - view.look_at[0],
        view.position[1] - view.look_at[1],
        view.position[2] - view.look_at[2],
    ];
    (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt() / 10.
}

/// `scale` that gives irradiance π (the default Environment Light's) at the
/// look-at point. The body is unit sized (quad side 1, sphere radius 1),
/// scaled to d/10 by `view_scale` and raised d/3 above the target by
/// `raised_light_position`, so h²/size² = 100/9: quad E ≈ L·s²/h², sphere
/// E ≈ π·L·r²/h².
fn area_light_scale(preset: &str) -> f64 {
    match preset {
        "area.quad" => 100. / 9. * std::f64::consts::PI,
        "area.sphere" => 100. / 9.,
        _ => 1.,
    }
}

/// The document a `project.create` starts from: one scene
/// with the chosen coordinate system, its render Camera, a Floor, an
/// Environment Light and the render trio — Film, Sampler, Integrator with
/// quality values — built through the same edits the Create
/// menu uses.
pub fn initial_document(
    title: &str,
    coordinate_system: CoordinateSystem,
) -> Result<crate::ProjectDocument, crate::ProjectError> {
    let mut document = crate::ProjectDocument::new(title);
    // The scene file is named after the title, which may hold `:` or `/`.
    for scene in document.scenes.values_mut() {
        let stem = file_stem(title).unwrap_or_else(|| "Scene".to_owned());
        std::sync::Arc::make_mut(scene).location = format!("Scenes/{stem}.scene");
    }
    let mut session = crate::ProjectSession::new(document)?;
    let document = session.document();
    let scene = document
        .default_scene
        .ok_or_else(|| crate::invalid("a fresh project's scene is missing"))?;
    let world = document
        .scene(scene)
        .ok_or_else(|| crate::invalid("missing scene"))?
        .world;
    let revision = session.revision();
    session.apply(
        &revision,
        &[
            crate::Edit::SetSceneCoordinateSystem {
                scene,
                up: coordinate_system.up,
                handedness: coordinate_system.handedness,
            },
            crate::Edit::CreatePreset {
                preset: SHAPE_FLOOR.id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            },
            crate::Edit::CreatePreset {
                preset: LIGHT_INFINITE.id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            },
            crate::Edit::CreatePreset {
                preset: CAMERA_PERSPECTIVE.id,
                parent: world,
                after: None,
                definition: None,
                view: None,
            },
            // the render trio in PBRT file order, with quality values ahead of pbrt-r4's defaults.
            crate::Edit::CreateConfiguration {
                scene,
                configuration: crate::ConfigurationKind::Film,
                implementation: None,
                parameters: vec![
                    text("filename", &[exr_name(title).as_str()]),
                    integers("xresolution", &[640]),
                    integers("yresolution", &[360]),
                ],
            },
            crate::Edit::CreateConfiguration {
                scene,
                configuration: crate::ConfigurationKind::Sampler,
                implementation: None,
                parameters: vec![integers("pixelsamples", &[64])],
            },
            crate::Edit::CreateConfiguration {
                scene,
                configuration: crate::ConfigurationKind::Integrator,
                implementation: None,
                parameters: vec![integers("maxdepth", &[5])],
            },
        ],
    )?;
    Ok(session.document().clone())
}

/// What a `CreateConfiguration` edit lands on when its caller picks no
/// implementation: the kind's first-edition default.
pub fn default_implementation(configuration: crate::ConfigurationKind) -> &'static str {
    match configuration {
        crate::ConfigurationKind::Film => "rgb",
        crate::ConfigurationKind::Sampler => "zsobol",
        crate::ConfigurationKind::Integrator => "volpath",
        crate::ConfigurationKind::PixelFilter => "gaussian",
        crate::ConfigurationKind::Accelerator => "bvh",
    }
}

/// The initial film's output file: the project title made nameable as a
/// file name, or the export's own `render.exr` when nothing of the title
/// can ride one.
pub fn exr_name(title: &str) -> String {
    file_stem(title).map_or_else(|| "render.exr".to_owned(), |name| format!("{name}.exr"))
}

/// The title as a file stem, or `None` when nothing nameable is left;
/// shared by the film's output, the scene file and the desktop's
/// project subfolder.
pub fn file_stem(title: &str) -> Option<String> {
    let mut name = String::with_capacity(title.len());
    for character in title.chars() {
        if filename_safe(character) {
            name.push(character);
        } else if !name.ends_with('_') {
            name.push('_');
        }
    }
    let name = name
        .trim()
        .trim_matches(|character| matches!(character, '_' | '.' | ' '));
    name.chars()
        .any(|character| character.is_alphanumeric())
        .then(|| name.to_owned())
}

/// The characters a file name can keep: letters, digits, spaces and the
/// connectors that need no escaping anywhere pbrt runs.
fn filename_safe(character: char) -> bool {
    character.is_alphanumeric() || matches!(character, '-' | '_' | '.' | ' ')
}

/// A fresh perspective camera's pose: from the caller's viewport
/// view, or the default placement (on the scene's front side, a
/// little above the origin, looking at it), rolled with the scene's up axis and carrying
/// the camera-space mirror a right-handed scene expects — an added
/// camera looks like the scene's existing cameras. `None` when the
/// placement is degenerate.
pub fn camera_pose(
    coordinate_system: CoordinateSystem,
    view: Option<PresetView>,
) -> Option<CameraPose> {
    let up = coordinate_system.up_vector();
    let (position, look_at) = match view {
        Some(view) => (view.position, view.look_at),
        None => {
            // The scene's front side, the same one `framing_camera_up`
            // views from (+Z for Y-up, −Y for Z-up), so the render camera
            // and Frame/Focus agree on where the front is.
            let front = match coordinate_system.up {
                crate::Axis::PosY | crate::Axis::PosX | crate::Axis::NegX => crate::Axis::PosZ,
                crate::Axis::NegY => crate::Axis::NegZ,
                crate::Axis::PosZ => crate::Axis::NegY,
                crate::Axis::NegZ => crate::Axis::PosY,
            };
            let forward = front.vector().map(|value| -value);
            let look_at = up.map(|value| value * 0.5);
            let position = [
                look_at[0] + up[0] * 1.5 - forward[0] * 6.,
                look_at[1] + up[1] * 1.5 - forward[1] * 6.,
                look_at[2] + up[2] * 1.5 - forward[2] * 6.,
            ];
            (position, look_at)
        }
    };
    let args = [
        position[0],
        position[1],
        position[2],
        look_at[0],
        look_at[1],
        look_at[2],
        up[0],
        up[1],
        up[2],
    ];
    let world_to_camera = crate::math::look_at(&args)?;
    let mut camera_to_world = inverse(world_to_camera)?;
    if coordinate_system.handedness == crate::Handedness::Right {
        camera_to_world = camera_to_world.compose(Mat4::scale(-1., 1., 1.));
    }
    Some(CameraPose {
        start: camera_to_world.0,
        end: camera_to_world.0,
    })
}

/// A light is raised above the viewport target by one third of its viewing
/// distance. Its preset group remains the placement seam for the body.
pub fn raised_light_position(view: PresetView, up: [f64; 3]) -> [f64; 3] {
    let delta = [
        view.position[0] - view.look_at[0],
        view.position[1] - view.look_at[1],
        view.position[2] - view.look_at[2],
    ];
    let offset = (delta[0] * delta[0] + delta[1] * delta[1] + delta[2] * delta[2]).sqrt() / 3.;
    std::array::from_fn(|axis| view.look_at[axis] + up[axis] * offset)
}

/// The rotation (a `Rotate` operation) mapping unit `from` onto unit `to`;
/// `None` when they already agree, and a 180° turn about a perpendicular
/// axis when they oppose. For axis-aligned pairs only, which is every
/// used by these presets.
pub(crate) fn align_rotation(from: [f64; 3], to: [f64; 3]) -> Option<TransformOperation> {
    let near = |a: [f64; 3], b: [f64; 3]| {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9 && (a[2] - b[2]).abs() < 1e-9
    };
    if near(from, to) {
        return None;
    }
    let axis = [
        from[1] * to[2] - from[2] * to[1],
        from[2] * to[0] - from[0] * to[2],
        from[0] * to[1] - from[1] * to[0],
    ];
    let length = (axis[0] * axis[0] + axis[1] * axis[1] + axis[2] * axis[2]).sqrt();
    if length < 1e-9 {
        // Antiparallel: any axis perpendicular to `to` turns it 180°.
        // Pick the coordinate axis `to` leans on least.
        let pick = [(to[0].abs(), 0), (to[1].abs(), 1), (to[2].abs(), 2)]
            .into_iter()
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .unwrap()
            .1;
        let mut axis = [0.; 3];
        axis[pick] = 1.;
        return Some(TransformOperation::Rotate {
            degrees: 180.,
            axis,
        });
    }
    let unit = axis.map(|value| value / length);
    for degrees in [90., 270.] {
        let rotated = Mat4::rotation(degrees, unit[0], unit[1], unit[2]);
        if near(rotated.transform_point(from), to) {
            return Some(TransformOperation::Rotate {
                degrees,
                axis: unit,
            });
        }
    }
    None
}

pub(crate) fn make_instance(definition: crate::ResourceId) -> Node {
    Node::new("Object Instance", NodeKind::Instance { definition })
}

fn rgb(name: &str, values: [f64; 3]) -> Parameter {
    floats(name, "rgb", values)
}

/// A centered XZ-plane bilinear patch (normal +Y by pbrt's dp/du × dp/dv
/// rule) spanning `±extent` — 0.5 for Plane and Area Light, 5 for Floor.
fn plane_points(extent: f64) -> Vec<Parameter> {
    vec![Parameter {
        value_type: "point3".into(),
        name: "P".into(),
        color_space: Default::default(),
        values: ParameterValues::Floats(
            [
                -extent, 0., -extent, -extent, 0., extent, extent, 0., -extent, extent, 0., extent,
            ]
            .into_iter()
            .map(FloatValue::Finite)
            .collect(),
        ),
    }]
}

fn floats(name: &str, value_type: &str, values: impl IntoIterator<Item = f64>) -> Parameter {
    Parameter {
        value_type: value_type.into(),
        name: name.into(),
        color_space: Default::default(),
        values: ParameterValues::Floats(values.into_iter().map(FloatValue::Finite).collect()),
    }
}

/// An `integer` parameter with plain values ('s initial render trio).
fn integers(name: &str, values: &[i64]) -> Parameter {
    Parameter {
        value_type: "integer".into(),
        name: name.into(),
        color_space: Default::default(),
        values: ParameterValues::Integers(values.to_vec()),
    }
}

/// A `string` parameter with plain values (the film's output name).
fn text(name: &str, values: &[&str]) -> Parameter {
    Parameter {
        value_type: "string".into(),
        name: name.into(),
        color_space: Default::default(),
        values: ParameterValues::Strings(values.iter().map(|value| (*value).into()).collect()),
    }
}

fn cube_parameters() -> Vec<Parameter> {
    let p = [
        -0.5, -0.5, -0.5, 0.5, -0.5, -0.5, 0.5, 0.5, -0.5, -0.5, 0.5, -0.5, -0.5, -0.5, 0.5, 0.5,
        -0.5, 0.5, 0.5, 0.5, 0.5, -0.5, 0.5, 0.5,
    ];
    let indices = [
        0, 2, 1, 0, 3, 2, 4, 5, 6, 4, 6, 7, 0, 1, 5, 0, 5, 4, 1, 2, 6, 1, 6, 5, 2, 3, 7, 2, 7, 6,
        3, 0, 4, 3, 4, 7,
    ];
    vec![
        Parameter {
            value_type: "point3".into(),
            name: "P".into(),
            color_space: Default::default(),
            values: ParameterValues::Floats(p.into_iter().map(FloatValue::Finite).collect()),
        },
        Parameter {
            value_type: "integer".into(),
            name: "indices".into(),
            color_space: Default::default(),
            values: ParameterValues::Integers(indices.into_iter().map(i64::from).collect()),
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn preset_scale(preset: &BuiltinPreset, view: PresetView) -> Option<[f64; 3]> {
        make_body(preset, CoordinateSystem::default(), Some(view))
            .into_iter()
            .find_map(|node| match node.data {
                NodeKind::State {
                    operation: StateOperation::Transform(TransformOperation::Scale(scale)),
                } => Some(scale),
                _ => None,
            })
    }

    #[test]
    fn area_light_shapes_scale_to_one_tenth_of_the_view_distance() {
        let view = PresetView {
            position: [0., 0., 10.],
            look_at: [0., 0., 0.],
        };
        assert_eq!(preset_scale(&AREA_QUAD, view), Some([1., 1., 1.]));
        assert_eq!(preset_scale(&AREA_SPHERE, view), Some([1., 1., 1.]));
        assert_eq!(preset_scale(&SHAPE_SPHERE, view), Some([1., 1., 1.]));
        assert_eq!(preset_scale(&LIGHT_POINT, view), None);
    }

    #[test]
    fn area_light_strength_scales_quad_and_sphere_to_pi_irradiance() {
        let view = PresetView {
            position: [0., 0., 10.],
            look_at: [0., 0., 0.],
        };
        let quad = make_body(&AREA_QUAD, CoordinateSystem::default(), Some(view));
        let sphere = make_body(&AREA_SPHERE, CoordinateSystem::default(), Some(view));
        let scale = |body: &[Node]| {
            body.iter()
                .find_map(|node| match &node.data {
                    NodeKind::Entity {
                        entity: EntityKind::AreaLight,
                        parameters,
                        ..
                    } => parameters
                        .iter()
                        .find(|parameter| parameter.name == "scale")
                        .and_then(|parameter| match &parameter.values {
                            ParameterValues::Floats(values) => match values.as_slice() {
                                [FloatValue::Finite(value)] => Some(*value),
                                _ => None,
                            },
                            _ => None,
                        }),
                    _ => None,
                })
                .unwrap()
        };
        // Irradiance at the look-at point from the body's own placement.
        let placement = |body: &[Node]| {
            let mut height = 0.;
            let mut size = 1.;
            for node in body {
                if let NodeKind::State {
                    operation: StateOperation::Transform(operation),
                } = &node.data
                {
                    match operation {
                        TransformOperation::Translate(p) => {
                            height = (p[0] * p[0] + p[1] * p[1] + p[2] * p[2]).sqrt()
                        }
                        TransformOperation::Scale(s) => size = s[0],
                        _ => {}
                    }
                }
            }
            (height, size)
        };
        let (h, s) = placement(&quad);
        let quad_irradiance = scale(&quad) * s * s / (h * h);
        let (h, r) = placement(&sphere);
        let sphere_irradiance = std::f64::consts::PI * scale(&sphere) * r * r / (h * h);
        assert!((quad_irradiance - std::f64::consts::PI).abs() < 1e-10);
        assert!((sphere_irradiance - std::f64::consts::PI).abs() < 1e-10);
    }

    #[test]
    fn quad_area_light_turns_its_normal_toward_the_view_target() {
        let view = PresetView {
            position: [0., 0., 10.],
            look_at: [0., 0., 0.],
        };
        let body = make_body(&AREA_QUAD, CoordinateSystem::default(), Some(view));
        assert!(body.iter().any(|node| matches!(
            node.data,
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Rotate { .. })
            }
        )));
    }
}
