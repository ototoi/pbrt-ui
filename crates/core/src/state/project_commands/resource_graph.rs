//! The upstream graph of one material or texture for the node
//! editor. Nodes are resources, breadth first from the root; an edge says
//! `to`'s `parameter` references `from`. Only upstream: the resources that
//! use the root are where-used relationships, not part of this graph.
//!
//! The graph adds one more node kind: inline spectrum
//! values (rgb / blackbody / named / wavelength samples) a material or
//! texture declares on a spectrum-capable slot are **value nodes** —
//! one per slot, owned by the parameter that holds them, with an edge
//! into the slot like any texture or SpectrumData drop would have. A
//! SpectrumData reference stays the resource node and a texture
//! reference stays a texture node; an undeclared slot grows no node and
//! says its table default in the input row instead.
use super::*;
use pbrt_io::parameter_schema::{self, Role, TextureInput};
use project::{Parameter, ParameterValues, ProjectDocument, Resource, ResourceId, ResourceKind};
use std::collections::{BTreeSet, VecDeque};

use super::spectrum_color::number;

fn directive(kind: ResourceKind) -> Option<&'static str> {
    match kind {
        ResourceKind::Material => Some("MakeNamedMaterial"),
        ResourceKind::Texture => Some("Texture"),
        ResourceKind::Medium | ResourceKind::ObjectDefinition | ResourceKind::SpectrumData => None,
    }
}

/// What may appear in a graph as an upstream node: materials and
/// textures, and the SpectrumData a parameter references — a leaf,
/// since its own parameters hold sample tables, never resources.
fn upstream_node(kind: ResourceKind) -> bool {
    matches!(
        kind,
        ResourceKind::Material | ResourceKind::Texture | ResourceKind::SpectrumData
    )
}

/// the slot's declaration as the resource spells it (an alias stays
/// an alias) — the two fields a drop replaces in one transaction.
fn declared_as<'a>(
    spec: &parameter_schema::ParameterSpec,
    resource: &'a Resource,
) -> Option<&'a Parameter> {
    resource
        .parameters
        .iter()
        .find(|p| spec.name == p.name.as_str() || spec.aliases.contains(&p.name.as_str()))
}

/// the spectrum interpretation classes a table row carries spectrum
/// values in: those are the slots a SpectrumData node
/// may be dropped on — a float lineage never, a spectrum lineage always.
fn takes_spectrum_data(spec: &parameter_schema::ParameterSpec) -> bool {
    spec.value_types.contains(&"spectrum")
}

/// The parameters that take a connection: the table's texture-input and
/// resource rows, then declared references the table does not know.
fn inputs(resource: &Resource, document: &ProjectDocument) -> Vec<Value> {
    let rows = directive(resource.kind)
        .and_then(parameter_schema::schema)
        .and_then(|table| parameter_schema::implementation(table, &resource.implementation))
        .and_then(|found| found.parameters(resource.texture_value_type))
        .unwrap_or_default();
    let mut inputs = rows
        .iter()
        .filter_map(|spec| {
            let accepts = match (spec.texture, spec.role) {
                (Some(TextureInput::Float), _) => "float",
                (Some(_), _) => "spectrum",
                (None, Role::Resource) => "material",
                (None, _) => return None,
            };
            // Table rows are what connect_texture accepts (materials'
            // references are not connected by it yet).
            let connectable = spec.texture.is_some();
            let mut input = json!({"parameter":spec.name,"accepts":accepts,
                                    "connectable":connectable,
                                    "required":spec.required});
            // which slots a SpectrumData node may be dropped on,
            // and the declaration the drop replaces first.
            if takes_spectrum_data(spec) {
                input["spectrum_data"] = json!(true);
                // The input shapes the table allows (the colour
                // dialog's tab rule the slot menu offers by).
                input["value_types"] = json!(spec.value_types);
                // The table's default, spelled: the grey value the
                // consumer shows while no node sits in the slot.
                if let Some(default) = spec.default_parameter()
                    && let Some(default) = default_spell(&default)
                {
                    input["default"] = json!(default);
                }
            }
            if let Some(declared) = declared_as(spec, resource) {
                input["declared_name"] = json!(declared.name);
                input["declared_type"] = json!(declared.value_type);
            }
            Some(input)
        })
        .collect::<Vec<_>>();
    for p in &resource.parameters {
        let ParameterValues::Resources { kind, ids } = &p.values else {
            continue;
        };
        if inputs
            .iter()
            .any(|input| input["parameter"] == p.name.as_str())
        {
            continue;
        }
        let accepts = match kind {
            ResourceKind::Material => "material",
            _ => match ids
                .first()
                .and_then(|id| document.resources.get(id))
                .and_then(|texture| texture.texture_value_type)
            {
                Some(project::TextureValueType::Float) => "float",
                _ => "spectrum",
            },
        };
        let mut input = json!({"parameter":p.name,"accepts":accepts,"connectable":false,
                               "declared_name":p.name,"declared_type":p.value_type});
        // a declared SpectrumData reference is itself a spectrum
        // slot: another SpectrumData may replace it.
        if kind == &ResourceKind::SpectrumData {
            input["spectrum_data"] = json!(true);
        }
        inputs.push(input);
    }
    inputs
}

/// The value a declared parameter holds, as the value node names it:
/// the kind the node editor draws (`value_type`) and its summary label.
/// `None` for anything that is not an inline spectrum value — a texture
/// or SpectrumData reference, a material reference, or a non-finite
/// float the document should never have accepted.
fn inline_spectrum(declared: &Parameter) -> Option<(&'static str, String)> {
    let finite = |values: &[project::FloatValue]| -> Option<Vec<f64>> {
        values
            .iter()
            .map(|value| match value {
                project::FloatValue::Finite(number) if number.is_finite() => Ok(*number),
                _ => Err(()),
            })
            .collect::<Result<_, _>>()
            .ok()
    };
    match (&declared.values, declared.value_type.as_str()) {
        // A builtin name: named spectra keep their table spelling.
        (ParameterValues::BuiltinSpectra(names), _) => names
            .first()
            .map(|name| ("named", format!("Named: {name}"))),
        (ParameterValues::Floats(values), "blackbody") => {
            let kelvin = finite(values)?.first().copied()?;
            Some(("blackbody", format!("Blackbody {} K", number(kelvin))))
        }
        // λ v pairs, the shape an inline sample table takes.
        (ParameterValues::Floats(values), "spectrum") => {
            let values = finite(values)?;
            let pairs = values.len();
            (pairs > 0 && pairs % 2 == 0).then_some(("samples", format!("Samples ({})", pairs / 2)))
        }
        // An rgb triple.
        (ParameterValues::Floats(values), "rgb") => {
            finite(values)?;
            (!values.is_empty()).then_some(("rgb", "RGB".to_owned()))
        }
        // A scalar float the spectrum slot also accepts (dielectric's
        // eta): a number of its own, never an RGB — the Inspector page
        // the node opens shows the number field for it.
        (ParameterValues::Floats(values), "float") => {
            let values = finite(values)?;
            match values.as_slice() {
                [] => None,
                [single] => Some(("number", format!("Float {}", number(*single)))),
                rest => Some(("number", format!("Floats ({})", rest.len()))),
            }
        }
        // A legacy spd asset the migration could not read: still the
        // slot's inline value, a file behind it.
        (ParameterValues::Assets(_), "spectrum") => Some(("samples", "spectrum file".into())),
        _ => None,
    }
}

/// The table's default, spelled the slot can show it grey: floats plain,
/// builtin names as they are. `None` for the defaults r4 resolves at
/// creation (illuminants, depends, presets) and non-finite floats.
fn default_spell(default: &Parameter) -> Option<String> {
    match &default.values {
        ParameterValues::Floats(values) => {
            let values = values
                .iter()
                .map(|value| match value {
                    project::FloatValue::Finite(nr) if nr.is_finite() => Ok(number(*nr)),
                    _ => Err(()),
                })
                .collect::<Result<Vec<_>, _>>()
                .ok()?;
            Some(values.join(" "))
        }
        ParameterValues::Strings(names) | ParameterValues::BuiltinSpectra(names) => {
            Some(names.join(" "))
        }
        _ => None,
    }
}

/// The inline spectrum values one material or texture holds: one value
/// node per spectrum-capable slot, each with its edge into the slot's
/// row (the table's own spelling, so the edge lands where the consumer
/// looks the row up). The node's id and `parameter` name the declared
/// spelling — the ids must name it too. Only materials and textures
/// have tables; SpectrumData leaves carry sample tables of their own.
fn value_nodes(resource: &Resource, owner: ResourceId) -> Vec<(Value, Value)> {
    directive(resource.kind)
        .and_then(parameter_schema::schema)
        .and_then(|table| parameter_schema::implementation(table, &resource.implementation))
        .and_then(|found| found.parameters(resource.texture_value_type))
        .into_iter()
        .flatten()
        .filter(|spec| takes_spectrum_data(spec))
        .filter_map(|spec| declared_as(spec, resource).map(|declared| (spec, declared)))
        .filter_map(|(spec, declared)| {
            let (value_type, summary) = inline_spectrum(declared)?;
            let id = format!("{owner}:{}", declared.name);
            Some((
                json!({"kind":"spectrum_value","id":id,
                       "owner":owner,"parameter":declared.name,
                       "value_type":value_type,
                       "label":summary,"summary":summary,
                       "inputs":[]}),
                json!({"from":id,"to":owner,"parameter":spec.name}),
            ))
        })
        .collect()
}

/// `{root, nodes, edges, truncated}` for `root`, at most `limit` nodes.
/// Edges only join kept nodes (a left-out node gets none, however often it
/// is referenced); a visited set stops reference cycles. The value nodes
/// of the kept materials and textures ride along after them — they are
/// not upstream resources and never set `truncated`.
pub(super) fn upstream(
    document: &ProjectDocument,
    root: ResourceId,
    limit: usize,
) -> Result<Value, ErrorPayload> {
    let first = document
        .resources
        .get(&root)
        .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "resource not found"))?;
    if !matches!(first.kind, ResourceKind::Material | ResourceKind::Texture) {
        return Err(invalid("only materials and textures have a resource graph"));
    }
    let mut kept = vec![root];
    let mut seen = BTreeSet::from([root]);
    let mut queue = VecDeque::from([root]);
    let mut links = Vec::new();
    let mut truncated = false;
    let mut left_out = BTreeSet::new();
    while let Some(id) = queue.pop_front() {
        for p in &document.resources[&id].parameters {
            let ParameterValues::Resources { ids, .. } = &p.values else {
                continue;
            };
            for (index, upstream) in ids.iter().enumerate() {
                let Some(resource) = document.resources.get(upstream) else {
                    continue;
                };
                if !upstream_node(resource.kind) {
                    continue;
                }
                if seen.insert(*upstream) {
                    if kept.len() == limit {
                        truncated = true;
                        left_out.insert(*upstream);
                        continue;
                    }
                    kept.push(*upstream);
                    queue.push_back(*upstream);
                }
                // A left-out node stays out when referenced again.
                if !left_out.contains(upstream) {
                    links.push((
                        *upstream,
                        id,
                        p.name.clone(),
                        (ids.len() > 1).then_some(index),
                    ));
                }
            }
        }
    }
    let nodes = kept
        .iter()
        .map(|id| {
            let resource = &document.resources[id];
            json!({
                "id":id,
                "kind":resource.kind,
                "label":resource.label,
                "implementation":resource.implementation,
                "texture_value_type":resource.texture_value_type,
                "inputs":inputs(resource, document),
            })
        })
        .collect::<Vec<_>>();
    // The value nodes hang after the resources they belong to (the root
    // keeps index 0 the layout relies on), each with its own edge.
    let (value_nodes, value_edges): (Vec<_>, Vec<_>) = kept
        .iter()
        .flat_map(|id| value_nodes(&document.resources[id], *id))
        .collect();
    let nodes = nodes.into_iter().chain(value_nodes).collect::<Vec<_>>();
    let edges = links
        .into_iter()
        .map(|(from, to, parameter, index)| {
            let mut edge = json!({"from":from,"to":to,"parameter":parameter});
            if let Some(index) = index {
                edge["index"] = json!(index);
            }
            edge
        })
        .chain(value_edges)
        .collect::<Vec<_>>();
    Ok(json!({"root":root,"nodes":nodes,"edges":edges,"truncated":truncated}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use project::{ColorSpaceBinding, Parameter, Revision, TextureValueType};

    fn texture(document: &mut ProjectDocument, label: &str) -> ResourceId {
        let resource = Resource {
            id: ResourceId::new(),
            label: label.into(),
            location: format!("Textures/{label}.texture"),
            edition: Revision::initial(),
            kind: ResourceKind::Texture,
            texture_value_type: Some(TextureValueType::Spectrum),
            implementation: "scale".into(),
            parameters: Vec::new(),
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let id = resource.id;
        document.resources.insert(id, resource);
        id
    }

    fn resource(
        document: &mut ProjectDocument,
        label: &str,
        kind: ResourceKind,
        location: &str,
        implementation: &str,
        texture_value_type: Option<TextureValueType>,
        parameters: Vec<Parameter>,
    ) -> ResourceId {
        let resource = Resource {
            id: ResourceId::new(),
            label: label.into(),
            location: location.into(),
            edition: Revision::initial(),
            kind,
            texture_value_type,
            implementation: implementation.into(),
            parameters,
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let id = resource.id;
        document.resources.insert(id, resource);
        id
    }

    /// A SpectrumData resource with the sample tables a document holds
    /// (the graph reads none of them; it is a leaf).
    fn spectrum_data(document: &mut ProjectDocument, label: &str) -> ResourceId {
        resource(
            document,
            label,
            ResourceKind::SpectrumData,
            &format!("Spectra/{label}.spd"),
            "piecewise",
            None,
            Vec::new(),
        )
    }

    /// A conductor material's `k` pointing at `upstream`.
    fn conductor(
        document: &mut ProjectDocument,
        label: &str,
        parameter: &str,
        upstream: ResourceId,
    ) -> ResourceId {
        resource(
            document,
            label,
            ResourceKind::Material,
            &format!("Materials/{label}.material"),
            "conductor",
            None,
            vec![Parameter {
                name: parameter.into(),
                value_type: "spectrum".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::SpectrumData,
                    ids: vec![upstream],
                },
            }],
        )
    }

    fn link(document: &mut ProjectDocument, from: ResourceId, parameter: &str, to: ResourceId) {
        document
            .resources
            .get_mut(&from)
            .unwrap()
            .parameters
            .push(Parameter {
                name: parameter.into(),
                value_type: "texture".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ids: vec![to],
                },
            });
    }

    #[test]
    fn declared_references_the_table_lacks_are_not_connectable() {
        let mut document = ProjectDocument::new("Unknown");
        let a = texture(&mut document, "a");
        let b = texture(&mut document, "b");
        link(&mut document, a, "mystery", b);
        let graph = upstream(&document, a, 256).unwrap();
        let inputs = graph["nodes"][0]["inputs"].as_array().unwrap();
        let input = |name: &str| inputs.iter().find(|i| i["parameter"] == name).unwrap();
        // scale's own operands are table rows; the extra one is not.
        assert_eq!(input("tex")["connectable"], true);
        assert_eq!(input("mystery")["connectable"], false);
    }

    #[test]
    fn a_reference_cycle_stops_at_the_visited_set() {
        // Export could never write this, but the graph must not loop on it.
        let mut document = ProjectDocument::new("Cycle");
        let a = texture(&mut document, "a");
        let b = texture(&mut document, "b");
        link(&mut document, a, "tex", b);
        link(&mut document, b, "tex", a);
        let graph = upstream(&document, a, 256).unwrap();
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 2);
        assert_eq!(graph["truncated"], false);
        // A shared upstream node appears once, with one edge per reference.
        let c = texture(&mut document, "c");
        link(&mut document, a, "scale", c);
        link(&mut document, b, "scale", c);
        let graph = upstream(&document, a, 256).unwrap();
        assert_eq!(graph["nodes"].as_array().unwrap().len(), 3);
        assert_eq!(graph["edges"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn a_left_out_node_gets_no_edges_even_when_referenced_again() {
        // A → B, A → C, B → C with room for two nodes: C is left out when A
        // reaches it, and B's later reference must not add an edge to it.
        let mut document = ProjectDocument::new("Truncated");
        let a = texture(&mut document, "a");
        let b = texture(&mut document, "b");
        let c = texture(&mut document, "c");
        link(&mut document, a, "tex", b);
        link(&mut document, a, "scale", c);
        link(&mut document, b, "tex", c);
        let graph = upstream(&document, a, 2).unwrap();
        assert_eq!(graph["truncated"], true);
        let kept = graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|node| node["id"].clone())
            .collect::<Vec<_>>();
        assert_eq!(kept.len(), 2);
        for edge in graph["edges"].as_array().unwrap() {
            assert!(kept.contains(&edge["from"]), "{edge}");
            assert!(kept.contains(&edge["to"]), "{edge}");
        }
    }

    /// a SpectrumData reference joins the graph as an upstream leaf
    /// node — kind `spectrum_data`, no inputs, no texture value type —
    /// with an edge naming the parameter that holds
    /// `Resources{kind: SpectrumData}`. The consumer's slot rows say
    /// whether a SpectrumData may land in them (`spectrum_data`) and the
    /// declaration they hold as written (`declared_name` / `_type`), so
    /// a drop can replace it in one transaction.
    #[test]
    fn spectrum_data_joins_the_graph_as_an_upstream_leaf() {
        let mut document = ProjectDocument::new("Conductor");
        let au_k = spectrum_data(&mut document, "Au.k");
        let gold = conductor(&mut document, "gold", "k", au_k);
        let graph = upstream(&document, gold, 256).unwrap();
        assert_eq!(
            graph["edges"],
            serde_json::json!([{ "from": au_k, "to": gold, "parameter": "k" }])
        );
        let node = |id: ResourceId| {
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .find(|node| node["id"] == serde_json::json!(id))
                .cloned()
                .unwrap()
        };
        let spectrum = node(au_k);
        assert_eq!(spectrum["kind"], "spectrum_data");
        assert_eq!(spectrum["inputs"], serde_json::json!([]));
        assert_eq!(spectrum["texture_value_type"], serde_json::Value::Null);
        let root = node(gold);
        let inputs = root["inputs"].as_array().unwrap().clone();
        let input = |name: &str| {
            inputs
                .iter()
                .find(|input| input["parameter"] == name)
                .cloned()
                .unwrap()
        };
        // The conductor's spectrum slots take spectrum data, and `k` also
        // names the declaration it holds; float slots never take it.
        assert_eq!(input("k")["spectrum_data"], true);
        assert_eq!(input("k")["declared_name"], "k");
        assert_eq!(input("k")["declared_type"], "spectrum");
        assert_eq!(input("eta")["spectrum_data"], true);
        assert!(input("eta").get("declared_name").is_none(), "unset");
        assert_eq!(input("reflectance")["spectrum_data"], true);
        assert!(
            input("roughness").get("spectrum_data").is_none(),
            "float slot"
        );
        // The same fields ride the declared reference the table lacks.
        let mystery = resource(
            &mut document,
            "mystery",
            ResourceKind::Material,
            "Materials/mystery.material",
            "diffuse",
            None,
            vec![Parameter {
                name: "glow".into(),
                value_type: "spectrum".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::SpectrumData,
                    ids: vec![au_k],
                },
            }],
        );
        let graph = upstream(&document, mystery, 256).unwrap();
        let glow = graph["nodes"][0]["inputs"]
            .as_array()
            .unwrap()
            .iter()
            .find(|input| input["parameter"] == "glow")
            .cloned()
            .unwrap();
        assert_eq!(glow["spectrum_data"], true);
        assert_eq!(glow["declared_name"], "glow");
        assert_eq!(glow["declared_type"], "spectrum");
    }

    /// A slot declared under an alias reports the spelling it was declared
    /// with — the removal edit must name it, and the new declaration
    /// replaces it rather than adding a second operand.
    #[test]
    fn alias_declared_slots_report_the_declared_spelling() {
        let mut document = ProjectDocument::new("Aliased");
        let checks = texture(&mut document, "checks");
        let tint = resource(
            &mut document,
            "tint",
            ResourceKind::Texture,
            "Textures/tint.texture",
            "scale",
            Some(TextureValueType::Spectrum),
            vec![Parameter {
                name: "tex1".into(),
                value_type: "texture".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Texture,
                    ids: vec![checks],
                },
            }],
        );
        let graph = upstream(&document, tint, 256).unwrap();
        let inputs = graph["nodes"][0]["inputs"].as_array().unwrap().clone();
        let input = |name: &str| {
            inputs
                .iter()
                .find(|input| input["parameter"] == name)
                .cloned()
                .unwrap()
        };
        // The table's own requiredness rides the row too: scale's `tex`
        // operand is required (no default to fall back on), its `scale`
        // is not — the value node's menu reads this and keeps Disconnect
        // off a required operand.
        assert_eq!(input("tex")["required"], true);
        assert_eq!(input("scale")["required"], false);
        // The table's `tex` row says its alias-declared value…
        assert_eq!(input("tex")["spectrum_data"], true);
        assert_eq!(input("tex")["declared_name"], "tex1");
        assert_eq!(input("tex")["declared_type"], "texture");
        // …and the declared reference the table does not know repeats it.
        assert_eq!(input("tex1")["declared_name"], "tex1");
        assert_eq!(input("tex1")["declared_type"], "texture");
        assert!(
            input("tex1").get("spectrum_data").is_none(),
            "a texture ref"
        );
    }

    /// still only materials and textures have a graph of their own —
    /// SpectrumData is a leaf, never a root.
    #[test]
    fn a_spectrum_data_root_is_refused() {
        let mut document = ProjectDocument::new("Refused");
        let au_k = spectrum_data(&mut document, "Au.k");
        let error = upstream(&document, au_k, 256).unwrap_err();
        assert_eq!(
            error.message,
            "only materials and textures have a resource graph"
        );
    }

    /// a parameter referencing a SpectrumData the document does not
    /// hold (deleted, or a hand-edited folder) is skipped like a missing
    /// texture: no node, no edge, no panic.
    #[test]
    fn missing_spectrum_data_references_are_skipped() {
        let mut document = ProjectDocument::new("Missing");
        let ghost = ResourceId::new();
        let gold = conductor(&mut document, "gold", "k", ghost);
        let graph = upstream(&document, gold, 256).unwrap();
        assert_eq!(
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["id"].clone())
                .collect::<Vec<_>>(),
            [serde_json::json!(gold)]
        );
        assert_eq!(graph["edges"], serde_json::json!([]));
    }

    /// A parameter the test helpers spell with plain floats.
    fn floats(name: &str, value_type: &str, values: &[f64]) -> Parameter {
        Parameter {
            name: name.into(),
            value_type: value_type.into(),
            color_space: ColorSpaceBinding::Inherit,
            values: ParameterValues::Floats(
                values
                    .iter()
                    .map(|value| project::FloatValue::Finite(*value))
                    .collect(),
            ),
        }
    }

    /// A value node's id: the owner's id, colon, the declared name.
    fn value_id(owner: ResourceId, parameter: &str) -> String {
        format!("{owner}:{parameter}")
    }

    fn value_node(graph: &Value, owner: ResourceId, parameter: &str) -> Value {
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == serde_json::json!(value_id(owner, parameter)))
            .cloned()
            .unwrap_or_else(|| panic!("no value node for {owner}:{parameter} in {graph}"))
    }

    /// The value nodes of the graph, (parameter, value_type, summary).
    fn value_nodes(graph: &Value) -> Vec<(String, String, String)> {
        graph["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|node| node["kind"] == "spectrum_value")
            .map(|node| {
                (
                    node["parameter"].as_str().unwrap().to_owned(),
                    node["value_type"].as_str().unwrap().to_owned(),
                    node["summary"].as_str().unwrap().to_owned(),
                )
            })
            .collect()
    }

    /// Inline spectrum values: each
    /// declared value of a spectrum-capable slot is a node of its own —
    /// the slot's value, not a resource — with an edge into the slot.
    #[test]
    fn inline_spectrum_values_join_the_graph_as_value_nodes() {
        let mut document = ProjectDocument::new("Gold");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            vec![
                floats("k", "rgb", &[1., 0.84, 0.28]),
                floats("eta", "blackbody", &[6500.]),
            ],
        );
        let graph = upstream(&document, gold, 256).unwrap();
        // The edges of a node's value slots come in the table's row
        // order, not the declaration order — the comparison is sorted.
        let sorted = |edges: &Value| {
            let mut edges = edges.as_array().unwrap().clone();
            edges.sort_by_key(|edge| {
                format!(
                    "{}:{}",
                    edge["parameter"].as_str().unwrap_or_default(),
                    edge["from"].as_str().unwrap_or_default(),
                )
            });
            edges
        };
        assert_eq!(
            sorted(&graph["edges"]),
            sorted(&serde_json::json!([
                { "from": value_id(gold, "k"), "to": gold, "parameter": "k" },
                { "from": value_id(gold, "eta"), "to": gold, "parameter": "eta" },
            ]))
        );
        let k = value_node(&graph, gold, "k");
        assert_eq!(k["kind"], "spectrum_value");
        assert_eq!(k["owner"], serde_json::json!(gold));
        assert_eq!(k["parameter"], "k");
        assert_eq!(k["value_type"], "rgb");
        assert_eq!(k["summary"], "RGB");
        assert_eq!(k["label"], "RGB");
        assert_eq!(k["inputs"], serde_json::json!([]));
        let eta = value_node(&graph, gold, "eta");
        assert_eq!(eta["value_type"], "blackbody");
        assert_eq!(eta["summary"], "Blackbody 6500 K");
        // A slot declared as a texture or a SpectrumData keeps its
        // resource node: only inline values become value nodes.
        assert_eq!(value_nodes(&graph).len(), 2);
    }

    /// Every inline shape names its own kind and summary: a builtin name,
    /// wavelength/value pairs, a legacy spd asset — and a scalar a
    /// spectrum slot declares as a plain float, which is the grey triple
    /// the colour box shows. Non-finite floats spell no node at all.
    #[test]
    fn every_inline_value_shape_gets_its_own_kind_and_summary() {
        let mut document = ProjectDocument::new("Catalogue");
        let named = resource(
            &mut document,
            "named",
            ResourceKind::Material,
            "Materials/named.material",
            "conductor",
            None,
            vec![Parameter {
                name: "reflectance".into(),
                value_type: "spectrum".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::BuiltinSpectra(vec!["stdillum-D65".into()]),
            }],
        );
        let graph = upstream(&document, named, 256).unwrap();
        assert_eq!(
            value_nodes(&graph),
            vec![(
                "reflectance".into(),
                "named".into(),
                "Named: stdillum-D65".into()
            )]
        );
        let samples = resource(
            &mut document,
            "samples",
            ResourceKind::Material,
            "Materials/samples.material",
            "conductor",
            None,
            vec![
                floats("eta", "spectrum", &[400., 0.5, 500., 0.7]),
                floats("k", "float", &[1.5]),
            ],
        );
        let graph = upstream(&document, samples, 256).unwrap();
        assert_eq!(
            value_nodes(&graph),
            vec![
                ("eta".into(), "samples".into(), "Samples (2)".into()),
                ("k".into(), "number".into(), "Float 1.5".into()),
            ]
        );
        // A plain float a spectrum slot also accepts (dielectric's
        // eta) is a number of its own, never a colour: the page the
        // node's Inspector opens shows the number fields, and the
        // label must not call it RGB.
        let glass = resource(
            &mut document,
            "glass",
            ResourceKind::Material,
            "Materials/glass.material",
            "dielectric",
            None,
            vec![
                floats("eta", "float", &[1.5]),
                floats("roughness", "float", &[0.1]),
            ],
        );
        let graph = upstream(&document, glass, 256).unwrap();
        assert_eq!(
            value_nodes(&graph),
            vec![("eta".into(), "number".into(), "Float 1.5".into())],
            "roughness takes no spectrum values, so no node for it"
        );
        // Several floats count themselves: no single scalar to show.
        let bulk = resource(
            &mut document,
            "bulk",
            ResourceKind::Material,
            "Materials/bulk.material",
            "dielectric",
            None,
            vec![floats("eta", "float", &[1., 2., 3.])],
        );
        let graph = upstream(&document, bulk, 256).unwrap();
        assert_eq!(
            value_nodes(&graph),
            vec![("eta".into(), "number".into(), "Floats (3)".into())]
        );
        // A legacy spd asset the migration could not read is still an
        // inline value: a samples node that names a file behind it.
        let asset = project::AssetId::new();
        let legacy = resource(
            &mut document,
            "legacy",
            ResourceKind::Material,
            "Materials/legacy.material",
            "conductor",
            None,
            vec![Parameter {
                name: "reflectance".into(),
                value_type: "spectrum".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Assets(vec![asset]),
            }],
        );
        let graph = upstream(&document, legacy, 256).unwrap();
        assert_eq!(
            value_nodes(&graph),
            vec![(
                "reflectance".into(),
                "samples".into(),
                "spectrum file".into()
            )]
        );
        // Non-finite floats spell nothing: the slot keeps its default.
        let broken = resource(
            &mut document,
            "broken",
            ResourceKind::Material,
            "Materials/broken.material",
            "conductor",
            None,
            vec![Parameter {
                name: "k".into(),
                value_type: "rgb".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Floats(vec![
                    project::FloatValue::Finite(1.),
                    project::FloatValue::Nan,
                    project::FloatValue::Finite(0.5),
                ]),
            }],
        );
        let graph = upstream(&document, broken, 256).unwrap();
        assert!(value_nodes(&graph).is_empty(), "{}", graph);
    }

    /// Undeclared slots have no node: the slot keeps r4's default, the
    /// consumer greys as the input row's `default` says.
    #[test]
    fn undeclared_spectrum_slots_have_no_value_node() {
        let mut document = ProjectDocument::new("Bare");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            Vec::new(),
        );
        let graph = upstream(&document, gold, 256).unwrap();
        assert!(value_nodes(&graph).is_empty(), "{}", graph);
        assert_eq!(graph["edges"], serde_json::json!([]));
    }

    /// A value declared under an alias names the alias (the edits must
    /// spell it that way) while its edge lands on the table's own slot
    /// row, so the consumer finds the slot by the row it knows.
    #[test]
    fn a_value_declared_under_an_alias_names_the_alias_but_lands_on_the_table_slot() {
        let mut document = ProjectDocument::new("Aliased");
        let tint = resource(
            &mut document,
            "tint",
            ResourceKind::Texture,
            "Textures/tint.texture",
            "scale",
            Some(TextureValueType::Spectrum),
            vec![floats("tex1", "rgb", &[0.2, 0.4, 0.8])],
        );
        let graph = upstream(&document, tint, 256).unwrap();
        let node = value_node(&graph, tint, "tex1");
        assert_eq!(node["parameter"], "tex1");
        assert_eq!(node["value_type"], "rgb");
        assert_eq!(
            graph["edges"],
            serde_json::json!([
                { "from": value_id(tint, "tex1"), "to": tint, "parameter": "tex" }
            ])
        );
    }

    /// References stay the nodes they were: a SpectrumData drop target
    /// and a spectrum texture keep their resource nodes and edges, and
    /// neither slot grows a value node.
    #[test]
    fn spectrum_data_and_texture_references_stay_resource_nodes() {
        let mut document = ProjectDocument::new("Referenced");
        let au_k = spectrum_data(&mut document, "Au.k");
        let checks = texture(&mut document, "checks");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            vec![
                Parameter {
                    name: "k".into(),
                    value_type: "spectrum".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: ParameterValues::Resources {
                        kind: ResourceKind::SpectrumData,
                        ids: vec![au_k],
                    },
                },
                Parameter {
                    name: "reflectance".into(),
                    value_type: "texture".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: ParameterValues::Resources {
                        kind: ResourceKind::Texture,
                        ids: vec![checks],
                    },
                },
            ],
        );
        let graph = upstream(&document, gold, 256).unwrap();
        assert!(value_nodes(&graph).is_empty(), "{}", graph);
        let kinds = |graph: &Value| {
            graph["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .map(|node| node["kind"].as_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert!(kinds(&graph).contains(&"spectrum_data".to_owned()));
        assert!(kinds(&graph).contains(&"texture".to_owned()));
        assert_eq!(
            kinds(&graph)
                .iter()
                .filter(|k| *k == "spectrum_value")
                .count(),
            0
        );
    }

    /// Value nodes ride along with the nodes the limit kept — they are
    /// not upstream resources and never set `truncated`.
    #[test]
    fn value_nodes_do_not_consume_the_node_limit() {
        let mut document = ProjectDocument::new("Limited");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            vec![floats("k", "rgb", &[1., 0.84, 0.28])],
        );
        let graph = upstream(&document, gold, 1).unwrap();
        assert_eq!(graph["truncated"], false);
        assert_eq!(value_nodes(&graph).len(), 1);
        assert!(graph["nodes"].as_array().unwrap().len() >= 2);
    }

    /// A spectrum-capable slot row spells the table's default — the grey
    /// value an undeclared slot shows. Float-only rows and rows whose
    /// default r4 resolves at creation stay without one.
    #[test]
    fn spectrum_slot_rows_spell_the_table_default() {
        let mut document = ProjectDocument::new("Defaults");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            vec![floats("k", "rgb", &[1., 0.6, 0.2])],
        );
        let graph = upstream(&document, gold, 256).unwrap();
        let inputs = graph["nodes"][0]["inputs"].as_array().unwrap().clone();
        let input = |name: &str| {
            inputs
                .iter()
                .find(|input| input["parameter"] == name)
                .cloned()
                .unwrap()
        };
        assert_eq!(input("k")["default"], "metal-Cu-k");
        assert_eq!(input("eta")["default"], "metal-Cu-eta");
        // k is declared here — the default still rides the row; the
        // consumer greys it only while no value sits in the slot.
        // A float texture slot and a spectrum row without a constant
        // default name no default.
        assert!(input("roughness").get("default").is_none());
        assert!(input("reflectance").get("default").is_none());
        let light = resource(
            &mut document,
            "light",
            ResourceKind::Texture,
            "Textures/light.texture",
            "constant",
            Some(TextureValueType::Spectrum),
            Vec::new(),
        );
        let _ = light;
    }

    /// A spectrum-capable slot row also lists the input shapes the
    /// table allows (`value_types`) — the colour dialog's tab rule the
    /// slot menu offers by. Float-only rows carry none.
    #[test]
    fn spectrum_slot_rows_list_the_input_shapes_the_schema_allows() {
        let mut document = ProjectDocument::new("Tabs");
        let gold = resource(
            &mut document,
            "gold",
            ResourceKind::Material,
            "Materials/gold.material",
            "conductor",
            None,
            Vec::new(),
        );
        let graph = upstream(&document, gold, 256).unwrap();
        let inputs = graph["nodes"][0]["inputs"].as_array().unwrap().clone();
        let input = |name: &str| {
            inputs
                .iter()
                .find(|input| input["parameter"] == name)
                .cloned()
                .unwrap()
        };
        assert_eq!(
            input("k")["value_types"],
            serde_json::json!(["spectrum", "rgb", "float"])
        );
        assert_eq!(
            input("reflectance")["value_types"],
            serde_json::json!(["spectrum", "rgb"])
        );
        assert!(input("roughness").get("value_types").is_none());
    }

    #[test]
    fn multi_resource_edges_carry_their_parameter_index() {
        let mut document = ProjectDocument::new("Mix slots");
        let first = resource(
            &mut document,
            "first",
            ResourceKind::Material,
            "Materials/first.material",
            "diffuse",
            None,
            Vec::new(),
        );
        let second = resource(
            &mut document,
            "second",
            ResourceKind::Material,
            "Materials/second.material",
            "diffuse",
            None,
            Vec::new(),
        );
        let mix = resource(
            &mut document,
            "mix",
            ResourceKind::Material,
            "Materials/mix.material",
            "mix",
            None,
            vec![Parameter {
                name: "materials".into(),
                value_type: "string".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Material,
                    ids: vec![first, second],
                },
            }],
        );
        let graph = upstream(&document, mix, 256).unwrap();
        let mut edges = graph["edges"].as_array().unwrap().clone();
        edges.sort_by_key(|edge| edge["index"].as_u64());
        assert_eq!(edges[0]["index"], 0);
        assert_eq!(edges[1]["index"], 1);
        assert!(edges.iter().all(|edge| edge["parameter"] == "materials"));

        let single = resource(
            &mut document,
            "single",
            ResourceKind::Material,
            "Materials/single.material",
            "mix",
            None,
            vec![Parameter {
                name: "materials".into(),
                value_type: "string".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::Material,
                    ids: vec![first],
                },
            }],
        );
        let graph = upstream(&document, single, 256).unwrap();
        let edge = graph["edges"].as_array().unwrap().first().unwrap();
        assert!(
            edge.get("index").is_none(),
            "single-resource edges keep their shape: {edge}"
        );
    }
}
