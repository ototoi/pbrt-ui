//! Persistent snapshots of the PBRT graphics-state machine.
//! Parent links describe state derivation and scope restoration, not entity kinds.
use crate::{Directive, Mat4, SceneParseError, Value as ParsedValue};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug)]
pub struct GraphicsState {
    pub start: Mat4,
    pub end: Mat4,
    pub active: u8,
    pub times: [f64; 2],
    pub reverse: bool,
    pub color_space: String,
    /// Declared binding; local material and area-light values refer to directive ordinals.
    pub material: Value,
    /// Explicit `Material "interface"` / `Material ""`, distinct from the
    /// default diffuse binding used when no Material directive is present.
    pub interface_material: bool,
    pub media: [String; 2],
    pub area_light: Option<usize>,
}
impl Default for GraphicsState {
    fn default() -> Self {
        Self {
            start: Mat4::identity(),
            end: Mat4::identity(),
            active: 3,
            times: [0., 1.],
            reverse: false,
            color_space: "srgb".into(),
            material: json!({"kind":"default"}),
            interface_material: false,
            media: [String::new(), String::new()],
            area_light: None,
        }
    }
}
impl GraphicsState {
    pub fn camera_to_world(&self) -> Option<Mat4> {
        inverse(self.start)
    }
    pub fn transform(&self) -> Value {
        if self.start == self.end {
            json!({"kind":"static","world_from_object":self.start.0})
        } else {
            json!({"kind":"animated","start":self.start.0,"end":self.end.0,"start_time":self.times[0],"end_time":self.times[1]})
        }
    }
    pub fn metadata(&self) -> Value {
        json!({"transform":self.transform(),"active_transform":self.active,"reverse_orientation":self.reverse,
            "color_space":self.color_space,"material_binding":self.material,"inside_medium":self.media[0],
            "outside_medium":self.media[1],"area_light_directive":self.area_light})
    }
    fn apply(&mut self, matrix: Mat4, replace: bool) {
        for (bit, target) in [(1, &mut self.start), (2, &mut self.end)] {
            if self.active & bit != 0 {
                *target = if replace { matrix } else { *target * matrix };
            }
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeRole {
    Scope,
    State,
    Entity,
    None,
}
#[derive(Clone, Debug)]
pub struct StateNode {
    /// Index into Document::directives, retaining the exact source occurrence.
    pub directive: usize,
    pub parent: Option<usize>,
    pub role: NodeRole,
    /// Snapshot after this directive; never mutated by subsequent directives.
    pub state: GraphicsState,
}
#[derive(Clone, Debug)]
pub struct StateGraph {
    pub nodes: Vec<StateNode>,
}

pub fn parse_state_graph(directives: &[Directive]) -> Result<StateGraph, SceneParseError> {
    let mut state = GraphicsState::default();
    let mut parent = None;
    let mut scopes = Vec::new();
    let mut coordinates = BTreeMap::new();
    let mut in_object = false;
    let mut object_names = BTreeSet::new();
    let mut graph = StateGraph {
        nodes: Vec::with_capacity(directives.len()),
    };
    for (index, d) in directives.iter().enumerate() {
        let fail = |message: &str| SceneParseError::syntax(&d.path, d.line, d.column, message);
        let text = d.positional_text(0).unwrap_or("");
        let numbers = d
            .positional
            .iter()
            .filter_map(ParsedValue::as_float)
            .collect::<Vec<_>>();
        let require = |count| {
            if numbers.len() == count && numbers.iter().all(|n| n.is_finite()) {
                Ok(())
            } else {
                Err(fail("invalid numeric transform operands"))
            }
        };
        let mut role = NodeRole::Entity;
        let mut node_parent = parent;
        match d.name.as_str() {
            "WorldBegin" => {
                if !scopes.is_empty() {
                    return Err(fail("WorldBegin inside an open scope"));
                }
                state.start = Mat4::identity();
                state.end = Mat4::identity();
                state.active = 3;
                coordinates.insert("world".into(), (state.start, state.end));
                node_parent = None;
                role = NodeRole::Scope;
            }
            "WorldEnd" => {
                if !scopes.is_empty() {
                    return Err(fail("WorldEnd inside an open scope"));
                }
                parent = None;
                role = NodeRole::None;
            }
            "AttributeBegin" | "TransformBegin" | "ObjectBegin" => {
                if d.name == "ObjectBegin" {
                    if in_object {
                        return Err(fail("nested object definitions"));
                    }
                    in_object = true;
                    if !object_names.insert(text.to_owned()) {
                        return Err(fail("duplicate object definition"));
                    }
                }
                scopes.push((d.name.as_str(), state.clone(), parent));
                role = if d.name == "ObjectBegin" {
                    NodeRole::Entity
                } else {
                    NodeRole::Scope
                };
            }
            "AttributeEnd" | "TransformEnd" | "ObjectEnd" => {
                let (kind, previous, previous_parent) =
                    scopes.pop().ok_or_else(|| fail("unmatched scope end"))?;
                let expected = match d.name.as_str() {
                    "AttributeEnd" => "AttributeBegin",
                    "TransformEnd" => "TransformBegin",
                    _ => "ObjectBegin",
                };
                if kind != expected {
                    return Err(fail("mismatched scope end"));
                }
                if kind == "TransformBegin" {
                    state.start = previous.start;
                    state.end = previous.end;
                    state.active = previous.active;
                } else {
                    state = previous;
                }
                if d.name == "ObjectEnd" {
                    in_object = false;
                }
                parent = previous_parent;
                node_parent = previous_parent;
                // TransformEnd can retain material/media/orientation changes made
                // inside the scope, so its restored state is a new visible node.
                role = if d.name == "TransformEnd" {
                    NodeRole::State
                } else {
                    NodeRole::None
                };
            }
            "Identity" => {
                state.apply(Mat4::identity(), true);
                role = NodeRole::State;
            }
            "Translate" | "Scale" => {
                require(3)?;
                state.apply(
                    if d.name == "Translate" {
                        Mat4::translation(numbers[0], numbers[1], numbers[2])
                    } else {
                        Mat4::scale(numbers[0], numbers[1], numbers[2])
                    },
                    false,
                );
                role = NodeRole::State;
            }
            "Rotate" => {
                require(4)?;
                state.apply(
                    Mat4::rotation(numbers[0], numbers[1], numbers[2], numbers[3]),
                    false,
                );
                role = NodeRole::State;
            }
            "Transform" | "ConcatTransform" => {
                require(16)?;
                let mut matrix = [0.; 16];
                for row in 0..4 {
                    for col in 0..4 {
                        matrix[row * 4 + col] = numbers[col * 4 + row];
                    }
                }
                state.apply(Mat4(matrix), d.name == "Transform");
                role = NodeRole::State;
            }
            "LookAt" => {
                require(9)?;
                state.apply(
                    look_at(&numbers).ok_or_else(|| fail("degenerate LookAt"))?,
                    false,
                );
                role = NodeRole::State;
            }
            "ActiveTransform" => {
                state.active = match text {
                    "StartTime" => 1,
                    "EndTime" => 2,
                    "All" => 3,
                    _ => return Err(fail("invalid ActiveTransform selector")),
                };
                role = NodeRole::State;
            }
            "TransformTimes" => {
                require(2)?;
                state.times = [numbers[0], numbers[1]];
                role = NodeRole::State;
            }
            "CoordinateSystem" => {
                coordinates.insert(text.to_owned(), (state.start, state.end));
                role = NodeRole::State;
            }
            "Camera" => {
                let start =
                    inverse(state.start).ok_or_else(|| fail("singular camera transform"))?;
                let end = inverse(state.end).ok_or_else(|| fail("singular camera transform"))?;
                coordinates.insert("camera".into(), (start, end));
            }
            "CoordSysTransform" => {
                let (start, end) = coordinates
                    .get(text)
                    .ok_or_else(|| fail("unknown coordinate system"))?;
                // Apply the named transforms only to the active time samples.
                if state.active & 1 != 0 {
                    state.start = *start;
                }
                if state.active & 2 != 0 {
                    state.end = *end;
                }
                role = NodeRole::State;
            }
            "ReverseOrientation" => {
                state.reverse = !state.reverse;
                role = NodeRole::State;
            }
            "ColorSpace" => {
                state.color_space = text.into();
                role = NodeRole::State;
            }
            "NamedMaterial" => {
                state.material = json!({"kind":"named","name":text});
                state.interface_material = false;
                role = NodeRole::State;
            }
            "Material" => {
                state.interface_material = matches!(text, "" | "interface");
                state.material = if state.interface_material {
                    json!({"kind":"none"})
                } else {
                    json!({"kind":"directive","directive":index})
                };
            }
            "MediumInterface" => {
                state.media = [text.into(), d.positional_text(1).unwrap_or("").into()];
                role = NodeRole::State;
            }
            "AreaLightSource" => {
                state.area_light = if text.is_empty() { None } else { Some(index) };
            }
            "ObjectInstance" if in_object => {
                return Err(fail("ObjectInstance inside an object definition"));
            }
            "Include" | "Import" => role = NodeRole::None,
            _ => {}
        }
        graph.nodes.push(StateNode {
            directive: index,
            parent: node_parent,
            role,
            state: state.clone(),
        });
        if matches!(role, NodeRole::Scope | NodeRole::State)
            || matches!(
                d.name.as_str(),
                "ObjectBegin" | "Material" | "AreaLightSource"
            )
        {
            parent = Some(index);
        }
    }
    if !scopes.is_empty() {
        let d = directives.last().unwrap();
        return Err(SceneParseError::syntax(
            &d.path,
            d.line,
            d.column,
            "unclosed scene scope",
        ));
    }
    Ok(graph)
}
fn look_at(v: &[f64]) -> Option<Mat4> {
    fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        (n > 0. && n.is_finite()).then(|| v.map(|x| x / n))
    }
    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    let forward = normalize([v[3] - v[0], v[4] - v[1], v[5] - v[2]])?;
    let right = normalize(cross(normalize([v[6], v[7], v[8]])?, forward))?;
    let up = cross(forward, right);
    let mut matrix = Mat4::identity();
    for (row, axis) in [right, up, forward].into_iter().enumerate() {
        for col in 0..3 {
            matrix.0[row * 4 + col] = axis[col];
        }
        matrix.0[row * 4 + 3] = -(0..3).map(|col| axis[col] * v[col]).sum::<f64>();
    }
    Some(matrix)
}

fn inverse(matrix: Mat4) -> Option<Mat4> {
    let mut rows = [[0.; 8]; 4];
    for row in 0..4 {
        rows[row][..4].copy_from_slice(&matrix.0[row * 4..row * 4 + 4]);
        rows[row][row + 4] = 1.;
    }
    for column in 0..4 {
        let pivot =
            (column..4).max_by(|&a, &b| rows[a][column].abs().total_cmp(&rows[b][column].abs()))?;
        rows.swap(column, pivot);
        let divisor = rows[column][column];
        if divisor == 0. || !divisor.is_finite() {
            return None;
        }
        for value in &mut rows[column] {
            *value /= divisor;
        }
        let pivot_row = rows[column];
        for row in 0..4 {
            if row == column {
                continue;
            }
            let factor = rows[row][column];
            for k in 0..8 {
                rows[row][k] -= factor * pivot_row[k];
            }
        }
    }
    let mut result = [0.; 16];
    for row in 0..4 {
        result[row * 4..row * 4 + 4].copy_from_slice(&rows[row][4..]);
    }
    result.iter().all(|v| v.is_finite()).then_some(Mat4(result))
}
