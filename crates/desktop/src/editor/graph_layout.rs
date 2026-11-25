//! The node editor's automatic layout and persisted node positions. Pure so it can be tested: the graph from
//! `project.get_resource_graph` in, boxes and edge end points out, in graph
//! units the view scales by its zoom.
//!
//! The root sits in the rightmost column; every other node sits one column
//! left of the farthest consumer it feeds,
//! so edges always run left to right. Within a column nodes keep the
//! graph's breadth-first order.
use serde_json::Value;
use std::{
    collections::{BTreeMap, HashMap},
    path::{Path, PathBuf},
};

pub const NODE_WIDTH: f32 = 180.;
pub const HEADER: f32 = 40.;
pub const INPUT_ROW: f32 = 20.;
const PADDING: f32 = 8.;
const COLUMN_GAP: f32 = 70.;
const ROW_GAP: f32 = 24.;
const MARGIN: f32 = 16.;

#[derive(Clone, Debug, PartialEq)]
pub struct NodeBox {
    pub id: String,
    pub x: f32,
    pub y: f32,
    pub height: f32,
    /// The input slots, top to bottom: the table's inputs, then any other
    /// parameter an edge arrives at.
    pub inputs: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Edge {
    pub from: (f32, f32),
    pub to: (f32, f32),
    pub source: String,
    pub target: String,
    pub parameter: String,
    pub index: Option<usize>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub nodes: Vec<NodeBox>,
    pub edges: Vec<Edge>,
    pub min_x: f32,
    pub min_y: f32,
    pub width: f32,
    pub height: f32,
}

const POSITION_FILE: &str = "node-positions.json";
const POSITION_VERSION: u32 = 1;

/// Node positions partitioned by editor root resource.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PositionStore(HashMap<String, HashMap<String, (f32, f32)>>);

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionFile {
    version: u32,
    roots: BTreeMap<String, BTreeMap<String, [f32; 2]>>,
}

pub fn position_file_path(project_folder: &Path) -> PathBuf {
    project_folder.join(".pbrtui").join(POSITION_FILE)
}

impl PositionStore {
    pub fn read(path: &Path) -> Self {
        let Ok(bytes) = std::fs::read(path) else {
            return Self::default();
        };
        let Ok(file) = serde_json::from_slice::<PositionFile>(&bytes) else {
            return Self::default();
        };
        if file.version != POSITION_VERSION {
            return Self::default();
        }
        Self(
            file.roots
                .into_iter()
                .map(|(root, nodes)| {
                    (
                        root,
                        nodes
                            .into_iter()
                            .map(|(node, point)| (node, (point[0], point[1])))
                            .collect(),
                    )
                })
                .collect(),
        )
    }

    pub fn write(&self, path: &Path) -> std::io::Result<()> {
        let parent = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
        std::fs::create_dir_all(parent)?;
        let roots = self
            .0
            .iter()
            .map(|(root, nodes)| {
                (
                    root.clone(),
                    nodes
                        .iter()
                        .map(|(node, (x, y))| (node.clone(), [*x, *y]))
                        .collect(),
                )
            })
            .collect();
        let bytes = serde_json::to_vec_pretty(&PositionFile {
            version: POSITION_VERSION,
            roots,
        })
        .map_err(std::io::Error::other)?;
        let temporary = path.with_extension(format!("{}.tmp", uuid::Uuid::new_v4()));
        std::fs::write(&temporary, bytes)?;
        if let Err(error) = std::fs::rename(&temporary, path) {
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
        Ok(())
    }

    pub fn for_root(&self, root: &str) -> HashMap<String, (f32, f32)> {
        self.0.get(root).cloned().unwrap_or_default()
    }

    pub fn remember(&mut self, root: &str, node: &str, position: (f32, f32)) {
        self.0
            .entry(root.to_owned())
            .or_default()
            .insert(node.to_owned(), position);
    }
}

impl NodeBox {
    /// Where an edge leaves this node: the header's right edge.
    pub fn output(&self) -> (f32, f32) {
        (self.x + NODE_WIDTH, self.y + HEADER / 2.)
    }

    /// Where an edge arrives at `parameter`'s slot.
    pub fn input(&self, parameter: &str) -> Option<(f32, f32)> {
        let index = self.inputs.iter().position(|name| name == parameter)?;
        Some((
            self.x,
            self.y + HEADER + index as f32 * INPUT_ROW + INPUT_ROW / 2.,
        ))
    }
}

pub fn layout(graph: &Value) -> Layout {
    let nodes = graph["nodes"].as_array().cloned().unwrap_or_default();
    let edges = graph["edges"].as_array().cloned().unwrap_or_default();
    let id = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let index: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(i, node)| (id(&node["id"]), i))
        .collect();
    let links = edges
        .iter()
        .filter_map(|edge| {
            Some((
                *index.get(&id(&edge["from"]))?,
                *index.get(&id(&edge["to"]))?,
                id(&edge["parameter"]),
                edge["index"].as_u64().map(|index| index as usize),
            ))
        })
        .collect::<Vec<_>>();
    // Longest path to the root, by relaxation; bounded by the node count so
    // a reference cycle cannot run away.
    let count = nodes.len();
    let mut column = vec![0usize; count];
    for _ in 0..count {
        let mut changed = false;
        for (from, to, _, _) in &links {
            let next = (column[*to] + 1).min(count.saturating_sub(1));
            // The root stays in the rightmost column even when a
            // reference cycle runs back into it.
            if column[*from] < next && from != to && *from != 0 {
                column[*from] = next;
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let last = column.iter().copied().max().unwrap_or(0);
    let mut bottoms = vec![MARGIN; last + 1];
    let mut boxes = Vec::with_capacity(count);
    for (i, node) in nodes.iter().enumerate() {
        let mut inputs = node["inputs"]
            .as_array()
            .into_iter()
            .flatten()
            .flat_map(|input| {
                let parameter = id(&input["parameter"]);
                if input["accepts"] == "material" {
                    vec![format!("{parameter}#0"), format!("{parameter}#1")]
                } else {
                    vec![parameter]
                }
            })
            .collect::<Vec<_>>();
        for (_, to, parameter, index) in &links {
            let slot = input_key(parameter, *index);
            if *to == i && !inputs.contains(&slot) {
                inputs.push(slot);
            }
        }
        let height = HEADER + inputs.len() as f32 * INPUT_ROW + PADDING;
        let x = MARGIN + (last - column[i]) as f32 * (NODE_WIDTH + COLUMN_GAP);
        let y = bottoms[column[i]];
        bottoms[column[i]] = y + height + ROW_GAP;
        boxes.push(NodeBox {
            id: id(&node["id"]),
            x,
            y,
            height,
            inputs,
        });
    }
    let edges = links
        .iter()
        .filter_map(|(from, to, parameter, index)| {
            let slot = input_key(parameter, *index);
            Some(Edge {
                from: boxes[*from].output(),
                to: boxes[*to].input(&slot)?,
                source: boxes[*from].id.clone(),
                target: boxes[*to].id.clone(),
                parameter: parameter.clone(),
                index: *index,
            })
        })
        .collect();
    Layout {
        width: MARGIN * 2. + (last + 1) as f32 * NODE_WIDTH + last as f32 * COLUMN_GAP,
        height: bottoms
            .iter()
            .copied()
            .fold(MARGIN, f32::max)
            .max(MARGIN * 2.),
        min_x: 0.,
        min_y: 0.,
        nodes: boxes,
        edges,
    }
}

/// Overlay session-local node positions and rebuild edge endpoints and bounds.
pub fn with_positions(mut layout: Layout, positions: &HashMap<String, (f32, f32)>) -> Layout {
    for node in &mut layout.nodes {
        if let Some((x, y)) = positions.get(&node.id) {
            node.x = *x;
            node.y = *y;
        }
    }
    for edge in &mut layout.edges {
        if let Some(source) = layout.nodes.iter().find(|node| node.id == edge.source) {
            edge.from = source.output();
        }
        if let Some(target) = layout.nodes.iter().find(|node| node.id == edge.target)
            && let Some(to) = target.input(&input_key(&edge.parameter, edge.index))
        {
            edge.to = to;
        }
    }
    let min_x = layout.nodes.iter().map(|node| node.x).fold(0., f32::min);
    let min_y = layout.nodes.iter().map(|node| node.y).fold(0., f32::min);
    let max_x = layout
        .nodes
        .iter()
        .map(|node| node.x + NODE_WIDTH)
        .fold(0., f32::max);
    let max_y = layout
        .nodes
        .iter()
        .map(|node| node.y + node.height)
        .fold(0., f32::max);
    layout.min_x = min_x;
    layout.min_y = min_y;
    layout.width = max_x - min_x + MARGIN;
    layout.height = max_y - min_y + MARGIN;
    layout
}

/// Place first-time upstream nodes beside positioned owners, leaving the
/// normal automatic layout intact for nodes without such an owner.
pub fn with_owner_positions(
    mut layout: Layout,
    graph: &Value,
    positions: &HashMap<String, (f32, f32)>,
) -> Layout {
    layout = with_positions(layout, positions);
    let edges = graph["edges"].as_array().cloned().unwrap_or_default();
    let mut owners = layout
        .nodes
        .iter()
        .filter(|node| positions.contains_key(&node.id))
        .map(|node| node.id.clone())
        .collect::<Vec<_>>();
    owners.sort();
    let mut placed = HashMap::<String, usize>::new();
    for owner_id in owners {
        let Some(owner) = layout
            .nodes
            .iter()
            .find(|node| node.id == owner_id)
            .cloned()
        else {
            continue;
        };
        let mut upstream = edges
            .iter()
            .filter(|edge| edge["to"] == owner_id.as_str())
            .collect::<Vec<_>>();
        upstream.sort_by_key(|edge| {
            let key = input_key(
                edge["parameter"].as_str().unwrap_or_default(),
                edge["index"].as_u64().map(|n| n as usize),
            );
            owner
                .inputs
                .iter()
                .position(|input| input == &key)
                .unwrap_or(usize::MAX)
        });
        for edge in upstream {
            let Some(id) = edge["from"].as_str() else {
                continue;
            };
            if positions.contains_key(id) || placed.contains_key(id) {
                continue;
            }
            let Some(node) = layout.nodes.iter().find(|node| node.id == id) else {
                continue;
            };
            let index = owner
                .inputs
                .iter()
                .position(|input| {
                    input
                        == &input_key(
                            edge["parameter"].as_str().unwrap_or_default(),
                            edge["index"].as_u64().map(|n| n as usize),
                        )
                })
                .unwrap_or(placed.len());
            let x = owner.x - NODE_WIDTH - COLUMN_GAP;
            let mut y = owner.y + index as f32 * (node.height + ROW_GAP);
            while layout
                .nodes
                .iter()
                .filter(|other| other.id != id)
                .any(|other| boxes_overlap(x, y, node.height, other))
            {
                y += node.height + ROW_GAP;
            }
            if let Some(node) = layout.nodes.iter_mut().find(|node| node.id == id) {
                node.x = x;
                node.y = y;
                placed.insert(id.to_owned(), index);
            }
        }
    }
    rebuild_geometry(layout)
}

fn boxes_overlap(x: f32, y: f32, height: f32, other: &NodeBox) -> bool {
    x < other.x + NODE_WIDTH
        && x + NODE_WIDTH > other.x
        && y < other.y + other.height + ROW_GAP
        && y + height + ROW_GAP > other.y
}

fn rebuild_geometry(mut layout: Layout) -> Layout {
    for edge in &mut layout.edges {
        if let Some(source) = layout.nodes.iter().find(|node| node.id == edge.source) {
            edge.from = source.output();
        }
        if let Some(target) = layout.nodes.iter().find(|node| node.id == edge.target)
            && let Some(to) = target.input(&input_key(&edge.parameter, edge.index))
        {
            edge.to = to;
        }
    }
    let min_x = layout.nodes.iter().map(|node| node.x).fold(0., f32::min);
    let min_y = layout.nodes.iter().map(|node| node.y).fold(0., f32::min);
    let max_x = layout
        .nodes
        .iter()
        .map(|node| node.x + NODE_WIDTH)
        .fold(0., f32::max);
    let max_y = layout
        .nodes
        .iter()
        .map(|node| node.y + node.height)
        .fold(0., f32::max);
    layout.min_x = min_x;
    layout.min_y = min_y;
    layout.width = max_x - min_x + MARGIN;
    layout.height = max_y - min_y + MARGIN;
    layout
}

pub fn input_key(parameter: &str, index: Option<usize>) -> String {
    index.map_or_else(
        || parameter.to_owned(),
        |index| format!("{parameter}#{index}"),
    )
}

pub fn input_parts(key: &str) -> (&str, Option<usize>) {
    key.rsplit_once('#')
        .and_then(|(parameter, index)| index.parse().ok().map(|index| (parameter, Some(index))))
        .unwrap_or((key, None))
}

pub fn input_label(key: &str) -> String {
    let (parameter, index) = input_parts(key);
    index.map_or_else(
        || parameter.to_owned(),
        |index| format!("{parameter} {}", index + 1),
    )
}

/// Return whether a slot of `target` accepting `accepts` (float /
/// spectrum) may be dropped onto graph node `node`: a texture of that
/// lineage, not the slot's own node, and not a node that already uses
/// `target` through the graph's edges (that drop would close a loop).
/// Core checks again; this keeps the drop targets valid. A
/// spectrum-capable slot (the row says `spectrum_data`) also accepts a
/// SpectrumData node, a float slot never.
pub fn can_connect(
    graph: &Value,
    node: &Value,
    accepts: &str,
    spectrum_data: bool,
    target: &str,
) -> bool {
    if accepts == "material" {
        return node["kind"] == "material"
            && node["id"].as_str() != Some(target)
            && !loop_would_close(graph, node, target);
    }
    if node["kind"] == "spectrum_data" {
        // A SpectrumData node lands on spectrum-capable slots only —
        // a material-taking row never carries the flag, a float slot is
        // refused twice over.
        return spectrum_data && accepts == "spectrum" && !loop_would_close(graph, node, target);
    }
    if node["kind"] != "texture" || node["texture_value_type"] != accepts {
        return false;
    }
    !loop_would_close(graph, node, target)
}

/// Walk from `node` upstream; reaching `target` means `target` is
/// already upstream of `node`, so the new edge would close a loop.
fn loop_would_close(graph: &Value, node: &Value, target: &str) -> bool {
    let edges = graph["edges"].as_array().cloned().unwrap_or_default();
    let mut stack = vec![node["id"].as_str().unwrap_or_default().to_owned()];
    let mut seen = std::collections::HashSet::new();
    while let Some(id) = stack.pop() {
        if id == target {
            return true;
        }
        if seen.insert(id.clone()) {
            stack.extend(
                edges
                    .iter()
                    .filter(|edge| edge["to"] == id.as_str())
                    .filter_map(|edge| edge["from"].as_str().map(str::to_owned)),
            );
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, inputs: &[&str]) -> Value {
        json!({"id":id,"inputs":inputs.iter().map(|p| json!({"parameter":p})).collect::<Vec<_>>()})
    }

    fn edge(from: &str, to: &str, parameter: &str) -> Value {
        json!({"from":from,"to":to,"parameter":parameter})
    }

    #[test]
    fn the_root_is_rightmost_and_feeders_sit_left_of_their_farthest_consumer() {
        // wall ← tint.reflectance; tint ← checks.tex1, grime.amount;
        // grime also feeds wall directly, but sits left of tint.
        let graph = json!({
            "nodes":[node("wall",&["reflectance","displacement"]),node("tint",&["tex1","tex2","amount"]),
                     node("checks",&[]),node("grime",&[])],
            "edges":[edge("tint","wall","reflectance"),edge("checks","tint","tex1"),
                     edge("grime","tint","amount"),edge("grime","wall","displacement")],
        });
        let layout = layout(&graph);
        let x = |id: &str| layout.nodes.iter().find(|n| n.id == id).unwrap().x;
        assert!(x("tint") < x("wall"));
        assert_eq!(x("checks"), x("grime"), "both one column left of tint");
        assert!(x("grime") < x("tint"));
        // Edges run left to right and land on their slots.
        assert_eq!(layout.edges.len(), 4);
        assert!(layout.edges.iter().all(|e| e.from.0 < e.to.0));
        let wall = &layout.nodes[0];
        assert_eq!(
            wall.input("displacement").unwrap().1,
            wall.y + HEADER + INPUT_ROW * 1.5
        );
        // Same column stacks top to bottom in graph order.
        let y = |id: &str| layout.nodes.iter().find(|n| n.id == id).unwrap().y;
        assert!(y("checks") < y("grime"));
        assert!(layout.width >= x("wall") + NODE_WIDTH);
    }

    #[test]
    fn a_slot_drops_onto_textures_of_its_lineage_only() {
        let texture = |id: &str, value_type: &str| json!({"id":id,"kind":"texture","texture_value_type":value_type});
        // wall ← tint ← checks.
        let graph = json!({"edges":[{"from":"tint","to":"wall","parameter":"reflectance"},
                                    {"from":"checks","to":"tint","parameter":"tex1"}]});
        let can = |node: Value, accepts: &str, target: &str| {
            can_connect(&graph, &node, accepts, false, target)
        };
        assert!(can(texture("checks", "spectrum"), "spectrum", "wall"));
        assert!(!can(texture("grime", "float"), "spectrum", "wall"));
        assert!(can(texture("grime", "float"), "float", "tint"));
        // Never onto the slot's own node, nor onto a material.
        assert!(!can(texture("tint", "spectrum"), "spectrum", "tint"));
        assert!(!can(
            json!({"id":"m","kind":"material"}),
            "spectrum",
            "wall"
        ));
        // Onto a node that already uses the slot's node would close a loop:
        // checks' slot onto tint, or onto wall.
        assert!(!can(texture("tint", "spectrum"), "spectrum", "checks"));
    }

    #[test]
    fn mix_material_inputs_get_two_indexed_rows_and_only_accept_other_materials() {
        let graph = json!({
            "nodes":[{"id":"mix","kind":"material","inputs":[{"parameter":"materials","accepts":"material"}]},
                      {"id":"a","kind":"material"},{"id":"b","kind":"material"}],
            "edges":[{"from":"a","to":"mix","parameter":"materials","index":0},
                     {"from":"b","to":"mix","parameter":"materials","index":1}]
        });
        let layout = layout(&graph);
        let mix = &layout.nodes[0];
        assert_eq!(mix.inputs, ["materials#0", "materials#1"]);
        assert_eq!(input_label(&mix.inputs[0]), "materials 1");
        assert_eq!(input_label(&mix.inputs[1]), "materials 2");
        assert_eq!(layout.edges[0].to, mix.input("materials#0").unwrap());
        assert_eq!(layout.edges[1].to, mix.input("materials#1").unwrap());
        assert!(can_connect(
            &graph,
            &graph["nodes"][1],
            "material",
            false,
            "mix"
        ));
        assert!(!can_connect(
            &graph,
            &graph["nodes"][0],
            "material",
            false,
            "mix"
        ));
    }

    /// A spectrum-capable slot (the row says `spectrum_data`) also
    /// accepts a SpectrumData node — a float slot never does, a spectrum
    /// node is still no texture, and the loop veto keeps applying.
    #[test]
    fn a_spectrum_data_node_drops_only_on_spectrum_capable_slots() {
        let texture = |id: &str, value_type: &str| json!({"id":id,"kind":"texture","texture_value_type":value_type});
        let spectrum = |id: &str| json!({"id":id,"kind":"spectrum_data"});
        // wall ← tint ← checks; a spectrum node references nothing.
        let graph = json!({"edges":[{"from":"tint","to":"wall","parameter":"reflectance"},
                                    {"from":"checks","to":"tint","parameter":"tex1"}]});
        let can = |node: Value, accepts: &str, spectrum_data: bool, target: &str| {
            can_connect(&graph, &node, accepts, spectrum_data, target)
        };
        assert!(can(spectrum("sd"), "spectrum", true, "wall"));
        assert!(can(spectrum("sd"), "spectrum", true, "tint"));
        // Never on a float slot, never on a
        // material-accepts slot, never the node itself.
        assert!(!can(spectrum("sd"), "float", false, "tint"));
        assert!(!can(spectrum("sd"), "float", true, "tint"), "float lineage");
        assert!(!can(spectrum("sd"), "material", true, "wall"));
        assert!(!can(spectrum("sd"), "spectrum", true, "sd"));
        // A slot that takes no spectrum data refuses the drop; textures
        // keep their own lineage rule on the very same slot.
        assert!(!can(spectrum("sd"), "spectrum", false, "wall"));
        assert!(can(texture("checks", "spectrum"), "spectrum", true, "wall"));
        assert!(!can(texture("grime", "float"), "spectrum", true, "wall"));
        // A material is still no spectrum node, spectrum flag or not.
        assert!(!can(
            json!({"id":"m","kind":"material"}),
            "spectrum",
            true,
            "wall"
        ));
    }

    #[test]
    fn unknown_slots_are_added_and_cycles_stay_bounded() {
        let graph = json!({
            "nodes":[node("a",&[]),node("b",&[])],
            "edges":[edge("b","a","tex"),edge("a","b","tex")],
        });
        let layout = layout(&graph);
        assert_eq!(layout.nodes[0].inputs, ["tex"]);
        // The root stays rightmost even when a cycle runs through it.
        assert!(layout.nodes[0].x > layout.nodes[1].x, "{layout:?}");
        assert_eq!(layout.nodes.len(), 2);
        assert!(layout.width.is_finite() && layout.height.is_finite());
        assert!(layout.nodes.iter().all(|n| n.x.is_finite()));
        // An empty graph lays out nothing.
        assert_eq!(super::layout(&json!({})).nodes.len(), 0);
    }

    #[test]
    fn moved_nodes_keep_edges_attached_and_expand_frame_bounds() {
        let graph = json!({
            "nodes":[node("material", &["reflectance"]), node("texture", &[])],
            "edges":[edge("texture", "material", "reflectance")],
        });
        let automatic = layout(&graph);
        let mut positions = HashMap::new();
        positions.insert("texture".to_owned(), (-80., 240.));
        let moved = with_positions(automatic.clone(), &positions);
        let texture = moved
            .nodes
            .iter()
            .find(|node| node.id == "texture")
            .unwrap();
        let material = moved
            .nodes
            .iter()
            .find(|node| node.id == "material")
            .unwrap();
        assert_eq!((texture.x, texture.y), (-80., 240.));
        assert_eq!(moved.edges[0].from, texture.output());
        assert_eq!(moved.edges[0].to, material.input("reflectance").unwrap());
        assert!(moved.min_x < 0.);
        assert!(moved.height > automatic.height);
        assert!(moved.width > 0.);
    }

    #[test]
    fn upstream_value_nodes_follow_a_positioned_owner_without_overlapping_cards() {
        let graph = json!({
            "nodes":[node("conductor", &["eta", "k"]),
                {"id":"eta-value","kind":"spectrum_value","inputs":[]},
                {"id":"k-value","kind":"spectrum_value","inputs":[]},
                node("other", &[])],
            "edges":[edge("eta-value", "conductor", "eta"), edge("k-value", "conductor", "k")]
        });
        let mut positions = HashMap::new();
        positions.insert("conductor".to_owned(), (500., 300.));
        let layout = with_owner_positions(layout(&graph), &graph, &positions);
        let conductor = layout
            .nodes
            .iter()
            .find(|node| node.id == "conductor")
            .unwrap();
        let eta = layout
            .nodes
            .iter()
            .find(|node| node.id == "eta-value")
            .unwrap();
        let k = layout
            .nodes
            .iter()
            .find(|node| node.id == "k-value")
            .unwrap();
        assert_eq!((conductor.x, conductor.y), (500., 300.));
        assert_eq!(eta.x, conductor.x - NODE_WIDTH - COLUMN_GAP);
        assert!(eta.y < k.y, "input order is stacked top to bottom");
        for (index, first) in layout.nodes.iter().enumerate() {
            for second in layout.nodes.iter().skip(index + 1) {
                assert!(
                    !boxes_overlap(first.x, first.y, first.height, second),
                    "{} overlaps {}: {first:?}, {second:?}",
                    first.id,
                    second.id
                );
            }
        }
        assert_eq!(layout.edges[0].to, conductor.input("eta").unwrap());
    }

    #[test]
    fn positions_restore_per_root_and_do_not_replace_other_roots() {
        let mut store = PositionStore::default();
        store.remember("material-a", "texture", (12., 34.));
        store.remember("material-b", "texture", (56., 78.));
        assert_eq!(
            store.for_root("material-a").get("texture"),
            Some(&(12., 34.))
        );
        assert_eq!(
            store.for_root("material-b").get("texture"),
            Some(&(56., 78.))
        );
        assert!(store.for_root("material-c").is_empty());
        store.remember("material-a", "texture", (90., 10.));
        assert_eq!(
            store.for_root("material-a").get("texture"),
            Some(&(90., 10.))
        );
    }

    #[test]
    fn node_positions_round_trip_in_the_versioned_project_sidecar() {
        let folder =
            std::env::temp_dir().join(format!("pbrt-ui-node-positions-{}", uuid::Uuid::new_v4()));
        let path = position_file_path(&folder);
        let mut positions = PositionStore::default();
        positions.remember("material-a", "texture", (12.5, -34.0));
        positions.remember("material-b", "value", (56.0, 78.0));

        positions.write(&path).unwrap();
        let json: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(json["version"], 1);
        assert_eq!(
            json["roots"]["material-a"]["texture"],
            serde_json::json!([12.5, -34.0])
        );
        assert_eq!(PositionStore::read(&path), positions);
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn malformed_or_unknown_node_position_sidecars_are_ignored() {
        let folder =
            std::env::temp_dir().join(format!("pbrt-ui-node-positions-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = folder.join("node-positions.json");
        for contents in [b"not json".as_slice(), br#"{"version":2,"roots":{}}"#] {
            std::fs::write(&path, contents).unwrap();
            assert_eq!(PositionStore::read(&path), PositionStore::default());
        }
        std::fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn initial_layout_stacks_many_value_nodes_feeding_one_material_without_overlap() {
        let graph = json!({
            "nodes":[node("material", &["roughness", "eta", "k", "reflectance"]),
                {"id":"roughness-value","kind":"float_value","inputs":[]},
                {"id":"eta-value","kind":"spectrum_value","inputs":[]},
                {"id":"k-value","kind":"spectrum_value","inputs":[]},
                {"id":"reflectance-value","kind":"spectrum_value","inputs":[]}],
            "edges":[edge("roughness-value", "material", "roughness"),
                edge("eta-value", "material", "eta"), edge("k-value", "material", "k"),
                edge("reflectance-value", "material", "reflectance")]
        });
        let layout = layout(&graph);
        assert_eq!(layout.nodes.len(), 5);
        for (index, first) in layout.nodes.iter().enumerate() {
            for second in layout.nodes.iter().skip(index + 1) {
                assert!(
                    !boxes_overlap(first.x, first.y, first.height, second),
                    "{} overlaps {}: {first:?}, {second:?}",
                    first.id,
                    second.id
                );
            }
        }
    }

    /// The value nodes the graph now carries lay out like the Spectrum
    /// leaves: no input rows of their own — the box is its header — and
    /// their edge leaves at the header, arriving at the owner's slot.
    #[test]
    fn value_nodes_lay_out_as_input_free_leaves_feeding_their_slot() {
        let graph = json!({
            "nodes":[
                node("m", &["k", "reflectance"]),
                {"id":"m:k", "kind":"spectrum_value", "inputs":[]},
                node("t", &[]),
            ],
            "edges":[edge("m:k", "m", "k"), edge("t", "m", "reflectance")],
        });
        let laid = layout(&graph);
        let value = laid.nodes.iter().find(|n| n.id == "m:k").unwrap();
        let material = laid.nodes.iter().find(|n| n.id == "m").unwrap();
        assert!(value.x < material.x, "upstream of its owner");
        assert_eq!(value.inputs, Vec::<String>::new());
        assert_eq!(value.height, HEADER + PADDING);
        let value_edge = laid.edges.iter().find(|e| e.source == "m:k").unwrap();
        assert_eq!(value_edge.from, value.output());
        assert_eq!(value_edge.to, material.input("k").unwrap());
    }

    /// A value node is never a drop target: the slot replaces its value
    /// by dropping on the node that gives the new one — textures and
    /// SpectrumData — or through the value node's own menu.
    #[test]
    fn a_value_node_is_no_drop_target() {
        let value = |id: &str| json!({"id":id,"kind":"spectrum_value","owner":"m","parameter":"k"});
        let graph = json!({"nodes":[value("m:k"), node("m", &["k"])]});
        assert!(!can_connect(&graph, &value("m:k"), "spectrum", true, "m"));
        assert!(!can_connect(&graph, &value("m:k"), "float", false, "m"));
    }
}
