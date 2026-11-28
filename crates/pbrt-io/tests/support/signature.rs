//! Scene meaning independent of names, UUIDs, file layout and directive order.
//!
//! A signature records what a renderer sees: camera, render settings, and
//! for every shape, light and instance its implementation, parameters,
//! evaluated transforms and the content (not the name) of every resource it
//! uses. Shapes, lights and instances are sorted, so reordering or renaming
//! during export does not change the signature.
use project::*;
use serde_json::{Value, json};
use std::{collections::BTreeMap, io::Read};

pub fn signature(document: &ProjectDocument, store: &ProjectStore) -> Value {
    let view = document
        .default_scene_view()
        .expect("imported scene exists");
    let evaluated = evaluate(&view).expect("imported scene evaluates");
    let mut builder = Builder {
        view,
        document,
        store,
        evaluated: &evaluated,
        definitions: BTreeMap::new(),
    };
    let mut camera = Value::Null;
    let mut settings = vec![];
    let mut options = vec![];
    for id in &view.scene.options {
        builder.options(*id, &mut camera, &mut settings, &mut options);
    }
    let mut world = World::default();
    builder.world(view.scene.world, &mut world);
    // The film's output file is chosen by the render job, not the scene, and
    // export supplies the default rgb film when the source has none.
    for setting in &mut settings {
        if setting["kind"] == "Film" {
            setting["parameters"]
                .as_array_mut()
                .unwrap()
                .retain(|parameter| parameter["name"] != "filename");
        }
    }
    if !settings.iter().any(|setting| setting["kind"] == "Film") {
        settings.push(json!({"kind": "Film", "implementation": "rgb", "parameters": []}));
    }
    settings.sort_by_key(canonical);
    options.sort_by_key(canonical);
    json!({
        "camera": camera,
        "settings": settings,
        "options": options,
        "shapes": sorted(world.shapes),
        "lights": sorted(world.lights),
        "instances": sorted(world.instances),
    })
}

/// Compare two signatures, allowing relative float error. Returns the first
/// differing path.
pub fn difference(left: &Value, right: &Value) -> Option<String> {
    fn walk(left: &Value, right: &Value, path: &str) -> Option<String> {
        match (left, right) {
            (Value::Number(a), Value::Number(b)) => {
                let (a, b) = (a.as_f64().unwrap(), b.as_f64().unwrap());
                let tolerance = 1e-9 * a.abs().max(b.abs()).max(1.0);
                ((a - b).abs() > tolerance).then(|| format!("{path}: {a} != {b}"))
            }
            (Value::Array(a), Value::Array(b)) => {
                if a.len() != b.len() {
                    return Some(format!("{path}: length {} != {}", a.len(), b.len()));
                }
                a.iter()
                    .zip(b)
                    .enumerate()
                    .find_map(|(i, (a, b))| walk(a, b, &format!("{path}[{i}]")))
            }
            (Value::Object(a), Value::Object(b)) => {
                let keys = a
                    .keys()
                    .chain(b.keys())
                    .collect::<std::collections::BTreeSet<_>>();
                keys.into_iter()
                    .find_map(|key| match (a.get(key), b.get(key)) {
                        (Some(a), Some(b)) => walk(a, b, &format!("{path}.{key}")),
                        _ => Some(format!("{path}.{key}: present on one side only")),
                    })
            }
            _ => (left != right).then(|| format!("{path}: {left} != {right}")),
        }
    }
    walk(left, right, "$")
}

#[derive(Default)]
struct World {
    shapes: Vec<Value>,
    lights: Vec<Value>,
    instances: Vec<Value>,
}

struct Builder<'a> {
    view: SceneView<'a>,
    document: &'a ProjectDocument,
    store: &'a ProjectStore,
    evaluated: &'a EvaluatedScene,
    definitions: BTreeMap<ResourceId, Value>,
}

impl Builder<'_> {
    fn options(
        &mut self,
        id: NodeId,
        camera: &mut Value,
        settings: &mut Vec<Value>,
        options: &mut Vec<Value>,
    ) {
        let node = self.view.node(id).expect("node exists");
        match &node.data {
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation,
                parameters,
                pose,
                ..
            } => {
                let state = &self.evaluated.nodes[&id];
                // A posed camera's placement is its pose (resolved
                // imports); a pre-pose camera (a saved project) keeps the
                // inverse of the transform current at its declaration.
                // Either way the renderer sees the same camera-to-world,
                // snapped at the comparison tolerance so the inverse
                // round trip cannot masquerade as a meaning change.
                *camera = json!({
                    "implementation": implementation,
                    "parameters": self.parameters(parameters, &state.color_space),
                    "transform": camera_placement(pose, state),
                    "inside_medium": self.medium(state.inside_medium),
                    "outside_medium": self.medium(state.outside_medium),
                });
            }
            NodeKind::Configuration {
                configuration,
                implementation,
                parameters,
            } => {
                let state = &self.evaluated.nodes[&id];
                settings.push(json!({
                    "kind": format!("{configuration:?}"),
                    "implementation": implementation,
                    "parameters": self.parameters(parameters, &state.color_space),
                }));
            }
            NodeKind::Option { name, value } => {
                options.push(
                    json!({"name": name.to_ascii_lowercase(), "value": option_meaning(value)}),
                );
            }
            NodeKind::Scope { children, .. } => {
                for child in children {
                    self.options(*child, camera, settings, options);
                }
            }
            _ => {}
        }
    }

    fn world(&mut self, id: NodeId, world: &mut World) {
        let node = self.view.node(id).expect("node exists");
        match &node.data {
            NodeKind::Scope { children, .. } => {
                for child in children {
                    self.world(*child, world);
                }
            }
            NodeKind::Entity {
                entity: EntityKind::Shape,
                ..
            } => world.shapes.push(self.shape(id)),
            NodeKind::Entity {
                entity: EntityKind::Light,
                implementation,
                parameters,
                ..
            } => {
                let state = &self.evaluated.nodes[&id];
                world.lights.push(json!({
                    "implementation": implementation,
                    "parameters": self.parameters(parameters, &state.color_space),
                    "transform": transform(state),
                    "outside_medium": self.medium(state.outside_medium),
                }));
            }
            NodeKind::Instance { definition } => {
                let state = &self.evaluated.nodes[&id];
                world.instances.push(json!({
                    "definition": self.definition(*definition),
                    "transform": transform(state),
                }));
            }
            // Object definitions are reached through their instances only.
            _ => {}
        }
    }

    fn shape(&mut self, id: NodeId) -> Value {
        let NodeKind::Entity {
            implementation,
            parameters,
            ..
        } = &self.view.node(id).expect("node exists").data
        else {
            unreachable!()
        };
        let state = self.evaluated.nodes[&id].clone();
        let area_light = state.area_light.map(|light| {
            let NodeKind::Entity {
                implementation,
                parameters,
                ..
            } = &self.view.node(light).expect("light exists").data
            else {
                unreachable!()
            };
            let light_state = &self.evaluated.nodes[&light];
            json!({
                "implementation": implementation,
                "parameters": self.parameters(parameters, &light_state.color_space),
            })
        });
        json!({
            "implementation": implementation,
            "parameters": self.parameters(parameters, &state.color_space),
            "transform": transform(&state),
            "reverse_orientation": state.reverse_orientation,
            "material": self.material(&state.material),
            "area_light": area_light,
            "inside_medium": self.medium(state.inside_medium),
            "outside_medium": self.medium(state.outside_medium),
        })
    }

    fn definition(&mut self, id: ResourceId) -> Value {
        if let Some(value) = self.definitions.get(&id) {
            return value.clone();
        }
        let scope = self.document.resources[&id]
            .definition_scope
            .expect("object definition scope");
        let mut world = World::default();
        self.world(scope, &mut world);
        let value = json!({
            "shapes": sorted(world.shapes),
            "lights": sorted(world.lights),
            "instances": sorted(world.instances),
        });
        self.definitions.insert(id, value.clone());
        value
    }

    fn material(&mut self, binding: &MaterialBinding) -> Value {
        match binding {
            MaterialBinding::Default => json!("default"),
            MaterialBinding::None => json!("none"),
            MaterialBinding::Resource(id) => self.resource(*id),
        }
    }

    fn medium(&mut self, id: Option<ResourceId>) -> Value {
        id.map_or(Value::Null, |id| self.resource(id))
    }

    /// Resource content: kind, implementation, parameters and, for textures
    /// and media, the transform captured at the declaration.
    fn resource(&mut self, id: ResourceId) -> Value {
        let resource = &self.document.resources[&id];
        if resource.kind == ResourceKind::ObjectDefinition {
            return self.definition(id);
        }
        let state = self.evaluated.resources[&id].clone();
        let captured = matches!(resource.kind, ResourceKind::Texture | ResourceKind::Medium)
            .then(|| transform(&state));
        json!({
            "kind": format!("{:?}", resource.kind),
            "implementation": resource.implementation,
            "texture_value_type": resource.texture_value_type.map(|t| format!("{t:?}")),
            "parameters": self.parameters(&resource.parameters, &state.color_space),
            "declared_transform": captured,
        })
    }

    fn parameters(&mut self, parameters: &[Parameter], color_space: &str) -> Value {
        let mut values = parameters
            .iter()
            .map(|parameter| {
                let colored =
                    parameter.value_type.starts_with("rgb") || parameter.value_type == "spectrum";
                let space = match &parameter.color_space {
                    ColorSpaceBinding::Explicit(space) => space.as_str(),
                    ColorSpaceBinding::Inherit => color_space,
                };
                json!({
                    "name": parameter.name,
                    "type": parameter.value_type,
                    "color_space": colored.then_some(space),
                    "values": self.values(&parameter.values),
                })
            })
            .collect::<Vec<_>>();
        values.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        Value::Array(values)
    }

    fn values(&mut self, values: &ParameterValues) -> Value {
        match values {
            ParameterValues::Floats(values) => Value::Array(values.iter().map(float).collect()),
            ParameterValues::Integers(values) => json!(values),
            ParameterValues::Booleans(values) => json!(values),
            ParameterValues::Strings(values) => json!(values),
            ParameterValues::BuiltinSpectra(values) => json!({"builtin": values}),
            ParameterValues::Assets(ids) => Value::Array(
                ids.iter()
                    .map(|id| json!({"asset": self.document.assets[id].sha256}))
                    .collect(),
            ),
            ParameterValues::Resources { ids, .. } => {
                Value::Array(ids.iter().map(|id| self.resource(*id)).collect())
            }
            ParameterValues::Array {
                asset,
                encoding,
                count,
            } => {
                let mut bytes = vec![];
                let path = self.store.asset_path(*asset).unwrap();
                std::fs::File::open(path)
                    .unwrap()
                    .read_to_end(&mut bytes)
                    .unwrap();
                let values = match encoding {
                    ArrayEncoding::F64Le => bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|b| float_number(f64::from_le_bytes(*b)))
                        .collect::<Vec<_>>(),
                    ArrayEncoding::I64Le => bytes
                        .as_chunks::<8>()
                        .0
                        .iter()
                        .map(|b| json!(i64::from_le_bytes(*b)))
                        .collect(),
                    ArrayEncoding::BoolU8 => bytes.iter().map(|b| json!(*b != 0)).collect(),
                };
                assert_eq!(values.len() as u64, *count);
                Value::Array(values)
            }
        }
    }
}

fn transform(state: &GraphicsState) -> Value {
    json!({
        "start": state.start.0.iter().map(|v| float_number(*v)).collect::<Vec<_>>(),
        "end": state.end.0.iter().map(|v| float_number(*v)).collect::<Vec<_>>(),
    })
}

/// The camera's world placement: its `pose` when it carries one, else the
/// inverse of the transform current at its declaration (the pre-pose
/// project format). Components are snapped at a relative tolerance of 1e-12
/// so the inverse round trip (recording the pose and re-emitting its
/// inverse) compares equal instead of drifting at the last ulp.
fn camera_placement(pose: &Option<project::CameraPose>, state: &GraphicsState) -> Value {
    let samples = match pose {
        Some(pose) => (pose.start, pose.end),
        None => (
            project::math::inverse(state.start)
                .expect("the declaration transform is invertible")
                .0,
            project::math::inverse(state.end)
                .expect("the declaration transform is invertible")
                .0,
        ),
    };
    let snap = |samples: &[f64; 16]| {
        samples
            .iter()
            .map(|&value| {
                let step = 1e-12 * value.abs().max(1.0);
                float_number((value / step).round() * step)
            })
            .collect::<Vec<_>>()
    };
    json!({
        "start": snap(&samples.0),
        "end": snap(&samples.1),
    })
}

fn float(value: &FloatValue) -> Value {
    match value {
        FloatValue::Finite(value) => float_number(*value),
        other => json!(format!("{other:?}")),
    }
}

/// -0.0 and 0.0 render identically; keep one spelling.
fn float_number(value: f64) -> Value {
    json!(if value == 0.0 { 0.0 } else { value })
}

/// Option values compare by meaning, not by their lexical spelling.
fn option_meaning(value: &OptionValue) -> Value {
    match value {
        OptionValue::Bool(value) => json!(value),
        OptionValue::Integer(value) => json!(value),
        OptionValue::Float(value) => float(value),
        OptionValue::String(value) => json!(value),
    }
}

/// JSON text with object keys in sorted order, so sorting by it does not
/// depend on whether serde_json preserves insertion order (a feature some
/// workspace dependencies enable).
fn canonical(value: &Value) -> String {
    match value {
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by(|a, b| a.0.cmp(b.0));
            let body = entries
                .into_iter()
                .map(|(key, value)| format!("{}:{}", Value::from(key.as_str()), canonical(value)))
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
        Value::Array(values) => {
            format!(
                "[{}]",
                values.iter().map(canonical).collect::<Vec<_>>().join(",")
            )
        }
        other => other.to_string(),
    }
}

fn sorted(mut values: Vec<Value>) -> Vec<Value> {
    values.sort_by_key(canonical);
    values
}
