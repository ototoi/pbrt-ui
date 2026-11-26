//! Create menu: the presets it lists and where a created node goes
//! Pure helpers make the placement rules testable.
use super::menu::PresetItem;
use serde_json::{Value, json};

pub const OBJECT_INSTANCE_PRESET: uuid::Uuid = uuid::uuid!("a62d8a1c-f165-40f7-adef-c5d3bf027006");

/// The camera perspective preset uses the viewport's current view.
pub const CAMERA_PERSPECTIVE_PRESET: uuid::Uuid =
    uuid::uuid!("266b209c-bbbf-4687-a697-483f7126d8ca");

/// The wire id of the group preset (`presets::GROUP_EMPTY`): an empty
/// Attribute group with nothing placeable inside, so it takes no view.
pub const GROUP_EMPTY_PRESET: uuid::Uuid = uuid::uuid!("19500901-220e-4577-a08a-6fa4173fa347");

/// The wire `view` pair for `create_preset`: the viewport's eye and
/// look-at point, for every preset the core can place — poses the
/// camera with it; places shapes at the target and B3 raises finite
/// and area lights above it. The group and object instance presets create nothing
/// placeable, so they send none. The camera keeps its guard: only a
/// view naming a direction not parallel to the scene's `up` is sent —
/// anything else leaves the camera the scene's default placement, which
/// is what `presets::camera_pose` would refuse core-side. A placement
/// needs a place, not a direction, so a body takes any finite view
/// with the eye off its look-at point — the near-zero guard mirrors
/// `presets::placeable` core-side; a zero distance would scale a
/// shape flat.
pub fn camera_view(
    preset: uuid::Uuid,
    camera: Option<&scene_view::ViewerCamera>,
    up: [f64; 3],
) -> Option<Value> {
    if preset == OBJECT_INSTANCE_PRESET || preset == GROUP_EMPTY_PRESET {
        return None;
    }
    let camera = camera?;
    let finite = |point: [f64; 3]| point.iter().all(|value| value.is_finite());
    let forward: [f64; 3] = std::array::from_fn(|i| camera.target[i] - camera.eye[i]);
    let cross = [
        forward[1] * up[2] - forward[2] * up[1],
        forward[2] * up[0] - forward[0] * up[2],
        forward[0] * up[1] - forward[1] * up[0],
    ];
    let directed = cross.iter().map(|value| value * value).sum::<f64>() > 1e-18;
    let away = forward.iter().map(|value| value * value).sum::<f64>() > 1e-18;
    let usable = if preset == CAMERA_PERSPECTIVE_PRESET {
        finite(camera.eye) && finite(camera.target) && directed
    } else {
        // A zero distance has no scale, and the core's `placeable`
        // check refuses it.
        finite(camera.eye) && finite(camera.target) && away
    };
    usable.then(|| json!({"position":camera.eye, "look_at":camera.target}))
}

/// The Inspector and View menu's one-step camera alignment edit. The
/// project preset helper owns the camera-to-world math shared with creation.
pub fn align_camera_edit(
    node_id: &str,
    coordinate_system: project::CoordinateSystem,
    view: project::presets::PresetView,
) -> Option<serde_json::Value> {
    let pose = project::presets::camera_pose(coordinate_system, Some(view))?;
    Some(serde_json::json!({
        "kind": "set_local_matrix",
        "node_id": node_id,
        "matrix": pose.start,
    }))
}

/// pbrt perspective cameras express fov along the film's shorter axis.
pub fn aligned_camera_fov(view_fov_y_degrees: f64, film_aspect: f64) -> f64 {
    if film_aspect >= 1.0 {
        view_fov_y_degrees
    } else {
        (2.0 * ((view_fov_y_degrees.to_radians() / 2.0).tan() * film_aspect).atan()).to_degrees()
    }
}

/// The view a camera node looks through, the inverse of `align_camera_edits`:
/// `camera_to_world` is the node's row-major world matrix (camera space looks
/// down +Z with +Y up; a right-handed scene's `Scale -1 1 1` only flips X), and
/// `camera_fov` the camera's `fov` along the film's shorter axis. Returns eye,
/// target `distance` ahead, up and the vertical fov.
pub fn view_from_camera(
    camera_to_world: &[f64; 16],
    camera_fov: f64,
    film_aspect: f64,
    distance: f64,
) -> Option<([f64; 3], [f64; 3], [f64; 3], f64)> {
    let m = camera_to_world;
    let unit = |v: [f64; 3]| {
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
        (length.is_finite() && length > 1e-12).then(|| v.map(|c| c / length))
    };
    let eye = [m[3], m[7], m[11]];
    let forward = unit([m[2], m[6], m[10]])?;
    let up = unit([m[1], m[5], m[9]])?;
    let target = std::array::from_fn(|i| eye[i] + forward[i] * distance);
    let vertical = if film_aspect >= 1.0 {
        camera_fov
    } else {
        (2.0 * ((camera_fov.to_radians() / 2.0).tan() / film_aspect).atan()).to_degrees()
    };
    Some((eye, target, up, vertical))
}

/// Pose and framing edits for one camera transaction. `camera_parameters`
/// comes from the same ProjectGetParameters read used by the Inspector.
pub fn align_camera_edits(
    node_id: &str,
    coordinate_system: project::CoordinateSystem,
    view: project::presets::PresetView,
    view_fov_y_degrees: f64,
    film_aspect: f64,
    camera_parameters: &Value,
) -> Option<Vec<Value>> {
    let pose = project::presets::camera_pose(coordinate_system, Some(view))?;
    let parameters = camera_parameters["parameters"].as_array()?;
    let half = parameters
        .iter()
        .any(|parameter| parameter["name"] == "halffov");
    let (name, value) = if half {
        (
            "halffov",
            aligned_camera_fov(view_fov_y_degrees, film_aspect) / 2.0,
        )
    } else {
        ("fov", aligned_camera_fov(view_fov_y_degrees, film_aspect))
    };
    Some(vec![
        json!({"kind":"set_local_matrix","node_id":node_id,"matrix":pose.start}),
        json!({"kind":"set_parameter","target":{"kind":"node","id":node_id},
              "name":name,"value_type":"float","value":{"name":name,"value_type":"float","color_space":null,
                       "values":{"kind":"floats","value":[value]}}}),
    ])
}

pub fn align_camera_edits_for_type(
    implementation: &str,
    node_id: &str,
    coordinate_system: project::CoordinateSystem,
    view: project::presets::PresetView,
    view_fov_y_degrees: f64,
    film_aspect: f64,
    camera_parameters: &Value,
) -> Option<Vec<Value>> {
    if implementation == "perspective" {
        align_camera_edits(
            node_id,
            coordinate_system,
            view,
            view_fov_y_degrees,
            film_aspect,
            camera_parameters,
        )
    } else {
        Some(vec![align_camera_edit(node_id, coordinate_system, view)?])
    }
}

/// Menu rows from `project.list_presets`, keeping the core's order. Rows
/// without a valid UUID are skipped; labels are never used to find a preset.
pub fn presets(list: &Value) -> Vec<PresetItem> {
    list["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|preset| {
            Some(PresetItem {
                id: uuid::Uuid::parse_str(preset["id"].as_str()?).ok()?,
                label: preset["label"].as_str()?.to_owned(),
                category: preset["category"].as_str().unwrap_or("").to_owned(),
            })
        })
        .collect()
}

/// One row of a Create menu: a preset at the top level, or a submenu of
/// presets.
#[derive(Debug, PartialEq, Eq)]
pub enum CreateEntry<'a> {
    Item(&'a PresetItem),
    Submenu(&'static str, Vec<&'a PresetItem>),
}

/// The Create menus' layout, shared by the menu bar and the Hierarchy:
/// shapes go under Shape ▸, lights and area lights under Light ▸, the rest
/// stays at the top level. Keeps the `list_presets` order.
pub fn layout(presets: &[PresetItem]) -> Vec<CreateEntry<'_>> {
    let mut entries: Vec<CreateEntry> = Vec::new();
    for preset in presets {
        let title = match preset.category.as_str() {
            "shape" => "Shape",
            "light" | "area" => "Light",
            _ => {
                entries.push(CreateEntry::Item(preset));
                continue;
            }
        };
        match entries
            .iter_mut()
            .find(|entry| matches!(entry, CreateEntry::Submenu(name, _) if *name == title))
        {
            Some(CreateEntry::Submenu(_, items)) => items.push(preset),
            _ => entries.push(CreateEntry::Submenu(title, vec![preset])),
        }
    }
    entries
}

/// A submenu's presets, each flagged when a separator goes before it (its
/// category differs from the previous one, e.g. area lights after lights).
pub fn with_breaks<'a>(
    items: &'a [&'a PresetItem],
) -> impl Iterator<Item = (bool, &'a PresetItem)> + 'a {
    items.iter().enumerate().map(|(index, preset)| {
        (
            index > 0 && items[index - 1].category != preset.category,
            *preset,
        )
    })
}

/// `parent` gets the new group; `after` is the sibling it follows (None:
/// the parent's last child).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub parent: String,
    pub after: Option<String>,
}

/// Create a shape or group at an ObjectDefinition root using the existing
/// `parent` wire field.
pub fn definition_preset_edit(
    preset: uuid::Uuid,
    parent: uuid::Uuid,
    after: Option<uuid::Uuid>,
    view: Value,
) -> Value {
    let edit = json!({
        "kind":"create_preset",
        "preset":preset,
        "parent":parent,
        "after":after,
        "definition":null,
        "view":view,
    });
    serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
        .expect("definition preset edit must match ProjectEdit");
    edit
}

/// A group selected: its last child. Another object selected: right after it
/// in the same parent. Nothing, the Scene root or the camera selected: the
/// end of the Scene root's virtual children (the core maps that to World).
/// If the placement candidate is one of the Hierarchy's light groups
/// (the ✦ rows — anything inside them inherits their Area Light), climb one
/// level; the new object lands right after that light group, in its parent.
pub fn target(tree: &Value, selected: Option<&str>) -> Option<Target> {
    let roots = tree["nodes"].as_array()?;
    let scene = roots.iter().find(|root| root["object"] == "scene")?;
    let scene_id = scene["id"].as_str()?.to_owned();
    let Some(selected) = selected else {
        return Some(Target {
            parent: scene_id,
            after: None,
        });
    };
    let Some((node, parent)) = find(roots, selected, None) else {
        return Some(Target {
            parent: scene_id,
            after: None,
        });
    };
    // Where the base rule would put the new object: the group that takes
    // it and the sibling it would follow.
    let (mut candidate, mut after) = match node["object"].as_str() {
        Some("group") => (node, None),
        Some("scene" | "camera") | None => {
            return Some(Target {
                parent: scene_id,
                after: None,
            });
        }
        Some(_) => match parent {
            Some(parent) if matches!(parent["object"].as_str(), Some("group" | "scene")) => {
                (parent, Some(selected.to_owned()))
            }
            _ => {
                return Some(Target {
                    parent: scene_id,
                    after: None,
                });
            }
        },
    };
    // Climb while the candidate group carries an Area Light — a
    // preset inside it would light up too. The new placement follows the
    // light group instead. A parent that could not hold children (a
    // camera, or no parent at all) falls back to the Scene root's end,
    // the same default today's rule gives a parent it cannot use.
    while is_area_light(candidate) {
        let Some(parent) = find(roots, candidate["id"].as_str()?, None)
            .and_then(|(_, parent)| parent)
            .filter(|parent| matches!(parent["object"].as_str(), Some("group" | "scene")))
        else {
            return Some(Target {
                parent: scene_id,
                after: None,
            });
        };
        after = Some(candidate["id"].as_str()?.to_owned());
        candidate = parent;
    }
    Some(Target {
        parent: candidate["id"].as_str()?.to_owned(),
        after,
    })
}

/// The Hierarchy's ✦ mark: a group whose `components` carry an Area
/// Light. Scenes never carry it; shapes inside such a group do.
fn is_area_light(node: &Value) -> bool {
    node["components"]
        .as_array()
        .is_some_and(|components| components.iter().any(|component| component == "area_light"))
}

fn find<'a>(
    nodes: &'a [Value],
    id: &str,
    parent: Option<&'a Value>,
) -> Option<(&'a Value, Option<&'a Value>)> {
    for node in nodes {
        if node["id"].as_str() == Some(id) {
            return Some((node, parent));
        }
        if let Some(children) = node["children"].as_array() {
            if let Some(found) = find(children, id, Some(node)) {
                return Some(found);
            }
        }
    }
    None
}

/// Ancestor ids of `id` in the loaded tree, immediate parent first (the
/// shape Hierarchy search hits use for reveal).
pub fn ancestors(tree: &Value, id: &str) -> Vec<String> {
    fn walk(nodes: &[Value], id: &str, path: &mut Vec<String>) -> bool {
        for node in nodes {
            let Some(node_id) = node["id"].as_str() else {
                continue;
            };
            if node_id == id {
                return true;
            }
            path.push(node_id.to_owned());
            if walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                id,
                path,
            ) {
                return true;
            }
            path.pop();
        }
        false
    }
    let mut path = Vec::new();
    let roots = tree["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if walk(roots, id, &mut path) {
        path.reverse();
        path
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tree() -> Value {
        json!({"nodes":[{"id":"scene","object":"scene","children":[
            {"id":"camera","object":"camera","children":[]},
            {"id":"room","object":"group","children":[
                {"id":"floor","object":"shape","children":[]}
            ]},
            {"id":"sky","object":"light","children":[]}
        ]}]})
    }

    #[test]
    fn create_menu_target_is_the_selected_group() {
        assert_eq!(
            target(&tree(), Some("room")),
            Some(Target {
                parent: "room".into(),
                after: None
            })
        );
    }

    #[test]
    fn create_menu_target_follows_a_selected_object() {
        assert_eq!(
            target(&tree(), Some("floor")),
            Some(Target {
                parent: "room".into(),
                after: Some("floor".into())
            })
        );
        assert_eq!(
            target(&tree(), Some("sky")),
            Some(Target {
                parent: "scene".into(),
                after: Some("sky".into())
            })
        );
    }

    #[test]
    fn create_menu_target_defaults_to_the_end_of_scene_children() {
        let scene = Some(Target {
            parent: "scene".into(),
            after: None,
        });
        assert_eq!(target(&tree(), None), scene);
        assert_eq!(target(&tree(), Some("camera")), scene);
        assert_eq!(target(&tree(), Some("scene")), scene);
        assert_eq!(target(&tree(), Some("unknown")), scene);
        assert_eq!(target(&json!({"nodes":[]}), None), None);
    }

    /// The Hierarchy's ✦ rows: a group whose `components` carry an
    /// Area Light scopes that light onto whatever lands inside it.
    fn light_tree() -> Value {
        json!({"nodes":[{"id":"scene","object":"scene","children":[
            {"id":"lamps","object":"group","components":["area_light"],"children":[
                {"id":"panel","object":"shape","children":[]}
            ]},
            {"id":"room","object":"group","components":["material"],"children":[]}
        ]}]})
    }

    #[test]
    fn a_light_group_selected_places_after_it_in_its_parent() {
        assert_eq!(
            target(&light_tree(), Some("lamps")),
            Some(Target {
                parent: "scene".into(),
                after: Some("lamps".into())
            })
        );
    }

    #[test]
    fn a_shape_inside_a_light_group_places_after_the_group() {
        assert_eq!(
            target(&light_tree(), Some("panel")),
            Some(Target {
                parent: "scene".into(),
                after: Some("lamps".into())
            })
        );
    }

    #[test]
    fn a_material_group_still_takes_the_new_object_inside() {
        assert_eq!(
            target(&light_tree(), Some("room")),
            Some(Target {
                parent: "room".into(),
                after: None
            })
        );
    }

    #[test]
    fn create_menu_lists_presets_by_uuid() {
        let list = json!({"presets":[
            {"id":"19500901-220e-4577-a08a-6fa4173fa347","label":"Group","category":"group"},
            {"id":"not-a-uuid","label":"Broken","category":"shape"},
            {"id":"209d7c20-ca6d-4965-a471-405c97e13662","label":"Group","category":"shape"}
        ]});
        let items = presets(&list);
        assert_eq!(items.len(), 2);
        assert_ne!(items[0].id, items[1].id);
        assert_eq!(items[1].category, "shape");
    }

    #[test]
    fn create_menu_groups_shapes_and_lights_into_submenus() {
        let preset = |label: &str, category: &str| PresetItem {
            id: uuid::Uuid::new_v4(),
            label: label.into(),
            category: category.into(),
        };
        let presets = [
            preset("Group", "group"),
            preset("Camera", "camera"),
            preset("Sphere", "shape"),
            preset("Cube", "shape"),
            preset("Point Light", "light"),
            preset("Quad Area Light", "area"),
            preset("Object Instance", "instance"),
        ];
        let entries = layout(&presets);
        let shape = vec![&presets[2], &presets[3]];
        let light = vec![&presets[4], &presets[5]];
        assert_eq!(
            entries,
            [
                CreateEntry::Item(&presets[0]),
                CreateEntry::Item(&presets[1]),
                CreateEntry::Submenu("Shape", shape),
                CreateEntry::Submenu("Light", light.clone()),
                CreateEntry::Item(&presets[6]),
            ]
        );
        let breaks: Vec<bool> = with_breaks(&light).map(|(brk, _)| brk).collect();
        assert_eq!(breaks, [false, true], "area lights follow a separator");
    }

    #[test]
    fn creating_inside_a_definition_uses_the_parent_wire_field() {
        let preset = uuid::Uuid::new_v4();
        let parent = uuid::Uuid::new_v4();
        let after = uuid::Uuid::new_v4();
        let edit = super::definition_preset_edit(preset, parent, Some(after), Value::Null);
        assert_eq!(edit["parent"], parent.to_string());
        assert_eq!(edit["after"], after.to_string());
        assert!(edit["definition"].is_null());
        serde_json::from_value::<protocol::project::ProjectEdit>(edit)
            .expect("definition create edit must match ProjectEdit");
    }

    #[test]
    fn ancestors_are_parent_first() {
        assert_eq!(ancestors(&tree(), "floor"), ["room", "scene"]);
        assert!(ancestors(&tree(), "missing").is_empty());
    }

    fn viewport_camera(eye: [f64; 3], target: [f64; 3]) -> scene_view::ViewerCamera {
        scene_view::ViewerCamera {
            eye,
            target,
            up: [0., 1., 0.],
            fov_y_degrees: 50.,
            near: 0.01,
            far: 100.,
            mirror: false,
        }
    }

    #[test]
    fn aligning_a_camera_uses_the_preset_pose_as_one_matrix_edit() {
        let view = project::presets::PresetView {
            position: [1., 2., 3.],
            look_at: [0., 0.5, 0.],
        };
        let coordinate_system = project::CoordinateSystem::default();
        let edit = align_camera_edit("camera-id", coordinate_system, view).unwrap();
        let pose = project::presets::camera_pose(coordinate_system, Some(view)).unwrap();
        assert_eq!(edit["kind"], "set_local_matrix");
        assert_eq!(edit["node_id"], "camera-id");
        assert_eq!(edit["matrix"], json!(pose.start));
        assert_eq!(pose.start, pose.end);
        let degenerate = project::presets::PresetView {
            position: [0., 10., 0.],
            look_at: [0., 0., 0.],
        };
        assert!(align_camera_edit("camera-id", coordinate_system, degenerate).is_none());
    }

    #[test]
    fn aligned_fov_matches_landscape_portrait_and_square_films() {
        let view_fov = 50.0_f64;
        assert!((aligned_camera_fov(view_fov, 16.0 / 9.0) - view_fov).abs() < 1e-10);
        assert!((aligned_camera_fov(view_fov, 1.0) - view_fov).abs() < 1e-10);
        let expected =
            (2.0 * ((view_fov.to_radians() / 2.0).tan() * 9.0 / 16.0).atan()).to_degrees();
        assert!((aligned_camera_fov(view_fov, 9.0 / 16.0) - expected).abs() < 1e-10);
    }

    #[test]
    fn aligning_a_camera_to_a_view_and_back_round_trips() {
        let eye: [f64; 3] = [1.5, 2.0, -3.0];
        let target: [f64; 3] = [-0.5, 0.25, 4.0];
        let distance = ((0..3).map(|i| (target[i] - eye[i]).powi(2)).sum::<f64>()).sqrt();
        for handedness in ["left", "right"] {
            for (aspect, view_fov) in [(16. / 9., 40.0), (0.6, 55.0)] {
                let system: project::CoordinateSystem =
                    serde_json::from_value(json!({"up":"+Y","handedness":handedness})).unwrap();
                let view = project::presets::PresetView {
                    position: eye,
                    look_at: target,
                };
                let pose = project::presets::camera_pose(system, Some(view)).unwrap();
                let camera_fov = super::aligned_camera_fov(view_fov, aspect);
                let (back_eye, back_target, back_up, back_fov) =
                    super::view_from_camera(&pose.start, camera_fov, aspect, distance).unwrap();
                for i in 0..3 {
                    assert!((back_eye[i] - eye[i]).abs() < 1e-9, "{handedness} eye");
                    assert!(
                        (back_target[i] - target[i]).abs() < 1e-9,
                        "{handedness} target"
                    );
                }
                // Up is +Y projected off the view direction.
                assert!(back_up[1] > 0.9, "{handedness} up {back_up:?}");
                assert!((back_fov - view_fov).abs() < 1e-9, "{handedness} fov");
            }
        }
    }

    #[test]
    fn camera_alignment_sets_fov_or_declared_halffov_in_one_apply_batch() {
        let coordinate_system = project::CoordinateSystem::default();
        let view = project::presets::PresetView {
            position: [1., 2., 3.],
            look_at: [0., 0.5, 0.],
        };
        let fov = align_camera_edits_for_type(
            "perspective",
            "camera-id",
            coordinate_system,
            view,
            45.,
            16. / 9.,
            &json!({"parameters":[]}),
        )
        .unwrap();
        assert_eq!(fov.len(), 2);
        assert_eq!(fov[0]["kind"], "set_local_matrix");
        assert_eq!(fov[1]["name"], "fov");
        let half = align_camera_edits_for_type(
            "perspective",
            "camera-id",
            coordinate_system,
            view,
            45.,
            16. / 9.,
            &json!({"parameters":[{"name":"halffov"}]}),
        )
        .unwrap();
        assert_eq!(half[1]["name"], "halffov");
        assert_eq!(half[1]["value"]["values"]["value"][0], 22.5);
        // The wire must take both edits.
        let uuid = "00000000-0000-4000-8000-000000000001";
        for edit in align_camera_edits_for_type(
            "perspective",
            uuid,
            coordinate_system,
            view,
            45.,
            16. / 9.,
            &json!({"parameters":[]}),
        )
        .unwrap()
        {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .unwrap_or_else(|error| panic!("{edit}: {error}"));
        }
        let other = align_camera_edits_for_type(
            "orthographic",
            "camera-id",
            coordinate_system,
            view,
            45.,
            16. / 9.,
            &json!({"parameters":[]}),
        )
        .unwrap();
        assert_eq!(other.len(), 1);
        assert_eq!(other[0]["kind"], "set_local_matrix");
    }

    /// The camera preset takes the live view as the wire
    /// `create_preset` pair, so the added Camera looks like what the
    /// user sees.
    #[test]
    fn the_camera_preset_carries_the_live_viewport_view() {
        let camera = viewport_camera([1., 2., 3.], [0., 0.5, 0.]);
        assert_eq!(
            camera_view(CAMERA_PERSPECTIVE_PRESET, Some(&camera), [0., 1., 0.]),
            Some(json!({"position":[1., 2., 3.], "look_at":[0., 0.5, 0.]}))
        );
        assert_eq!(
            camera_view(CAMERA_PERSPECTIVE_PRESET, None, [0., 1., 0.]),
            None,
            "without a viewport the camera takes the scene's default placement"
        );
    }

    /// Every placeable preset — the shape, light, and area bodies —
    /// carries the live viewport view too, so the core lands it where
    /// the user is looking. The group and instance presets create
    /// nothing placeable and send none.
    #[test]
    fn placeable_presets_carry_the_live_viewport_view() {
        let camera = viewport_camera([1., 2., 3.], [0., 0.5, 0.]);
        let view = Some(json!({"position":[1., 2., 3.], "look_at":[0., 0.5, 0.]}));
        for preset in [
            uuid::uuid!("209d7c20-ca6d-4965-a471-405c97e13662"), // shape.sphere
            uuid::uuid!("80bf7ed4-21df-48f8-8715-bb1285ecd7b7"), // light.point
            uuid::uuid!("ee016a71-134f-4eeb-8d71-ef1132f3125a"), // area.quad
        ] {
            assert_eq!(
                camera_view(preset, Some(&camera), [0., 1., 0.]),
                view,
                "{preset}"
            );
        }
        for preset in [
            uuid::uuid!("19500901-220e-4577-a08a-6fa4173fa347"), // group.empty
            OBJECT_INSTANCE_PRESET,
        ] {
            assert_eq!(
                camera_view(preset, Some(&camera), [0., 1., 0.]),
                None,
                "{preset}"
            );
        }
    }

    /// Placement needs a position, not only a direction — a top-down view
    /// still places a body at its look-at point, while the camera keeps
    /// the refusal: the core's LookAt would name no roll axis.
    #[test]
    fn a_top_down_viewport_view_places_bodies_but_not_cameras() {
        let top_down = viewport_camera([0., 10., 0.], [0., 0., 0.]);
        let sphere = uuid::uuid!("209d7c20-ca6d-4965-a471-405c97e13662");
        assert_eq!(
            camera_view(sphere, Some(&top_down), [0., 1., 0.]),
            Some(json!({"position":[0., 10., 0.], "look_at":[0., 0., 0.]}))
        );
        assert_eq!(
            camera_view(CAMERA_PERSPECTIVE_PRESET, Some(&top_down), [0., 1., 0.]),
            None
        );
    }

    /// The core refuses degenerate placements, so a view that names no place — the eye
    /// on its look-at point, or no direction at all for the camera — is
    /// left off and the default placement applies.
    #[test]
    fn a_degenerate_viewport_view_is_not_sent() {
        let sphere = uuid::uuid!("209d7c20-ca6d-4965-a471-405c97e13662");
        let degenerate = viewport_camera([0., 0.5, 0.], [0., 0.5, 0.]);
        assert_eq!(
            camera_view(CAMERA_PERSPECTIVE_PRESET, Some(&degenerate), [0., 1., 0.]),
            None
        );
        // A body also needs the eye off its look-at point — a zero
        // distance would scale it flat.
        assert_eq!(camera_view(sphere, Some(&degenerate), [0., 1., 0.]), None);
        let non_finite = scene_view::ViewerCamera {
            eye: [f64::NAN, 0., 0.],
            ..degenerate
        };
        assert_eq!(
            camera_view(CAMERA_PERSPECTIVE_PRESET, Some(&non_finite), [0., 1., 0.]),
            None
        );
        assert_eq!(camera_view(sphere, Some(&non_finite), [0., 1., 0.]), None);
        // Looking straight along the scene's up is degenerate for the
        // core's LookAt; the camera's default placement applies instead.
        let straight_down = viewport_camera([0., 10., 0.], [0., 0., 0.]);
        assert_eq!(
            camera_view(
                CAMERA_PERSPECTIVE_PRESET,
                Some(&straight_down),
                [0., 1., 0.]
            ),
            None
        );
        assert!(
            camera_view(
                CAMERA_PERSPECTIVE_PRESET,
                Some(&straight_down),
                [0., 0., 1.]
            )
            .is_some()
        );
    }
}
