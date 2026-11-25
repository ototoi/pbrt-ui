//! The node editor's selection and what the Inspector shows for it:
//! pure decisions the click handlers and the
//! Inspector panel execute — which target a clicked node stands for,
//! which node a node editor highlights, and which parameter row a
//! value node's page edits.
use super::parameter_rows::{self, Row};
use serde_json::Value;

/// What a node editor click asks the Inspector to show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InspectorTarget {
    /// A resource node selects like a Contents click; `id` is the
    /// resource (the Contents entry itself is session state).
    Contents(String),
    /// A value node opens the inline colour page of its owner's
    /// parameter, writing the owner's declaration as the dialog does.
    Value { owner: String, parameter: String },
}

/// What clicking `node` drives the Inspector to: material, texture and
/// SpectrumData nodes select like a Contents click of the resource;
/// a value node opens its owner's parameter. Anything else — an
/// unknown kind, a malformed node — selects nothing.
pub fn click_target(node: &Value) -> Option<InspectorTarget> {
    match node["kind"].as_str() {
        Some("material" | "texture" | "spectrum_data") => {
            Some(InspectorTarget::Contents(node["id"].as_str()?.to_owned()))
        }
        Some("spectrum_value") => Some(InspectorTarget::Value {
            owner: node["owner"].as_str()?.to_owned(),
            parameter: node["parameter"].as_str()?.to_owned(),
        }),
        _ => None,
    }
}

/// A value node's id: the owner's resource id, colon, the declared
/// parameter name — the spelling the core graph emits it under.
pub fn value_node_id(owner: &str, parameter: &str) -> String {
    format!("{owner}:{parameter}")
}

/// What the node editor asks the resource details of, for one selected
/// node against the graph it has: a resource node's own id, a value
/// node's owner — and, once the selected node is gone from the graph
/// (its own Disconnect or replacement consumed it, the page's Data tab
/// turned the value into a SpectrumData reference, an undo removed it),
/// the owner the Inspector's value page still names for this root, so
/// that page keeps its data across the very edits it sends.
pub fn details_target(
    root: &str,
    selected: &str,
    graph: &Value,
    graph_value: Option<(&str, &str, &str)>,
) -> String {
    match graph["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|node| node["id"].as_str() == Some(selected))
    {
        Some(node) if node["kind"] == "spectrum_value" => {
            node["owner"].as_str().unwrap_or(selected).to_owned()
        }
        // A resource node: itself.
        Some(_) => selected.to_owned(),
        // The selected node is gone; the value page still names it for
        // this root — its owner is what the page reads.
        None => match graph_value {
            Some((value_root, owner, parameter))
                if value_root == root
                    && (selected == value_node_id(owner, parameter) || selected == owner) =>
            {
                owner.to_owned()
            }
            _ => selected.to_owned(),
        },
    }
}

/// What the Inspector's value page does with the editor it has
/// mounted, on one preparation pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValuePageAction {
    /// Build an editor for a target that has none yet.
    Mount,
    /// The mounted editor is the target's: refresh it with the fresh
    /// row (the same owner and parameter, a new revision).
    Refresh,
    /// The mounted editor is the target's but its owner's page details
    /// are still in flight: keep it as it is — its drafts live until
    /// the fresh row arrives.
    Keep,
    /// No value target, or the target moved: let the editor go.
    Discard,
}

/// The value page's lifecycle for its mounted editor (the Inspector
/// executes this): `mounted` is the (owner, parameter) of the editor
/// on screen, `target` the selected value node's, and `ready` whether
/// the owner's page details answered at the live project. The page
/// names its editor by owner and parameter — never by the revision or
/// the declaration's edition — because every successful edit bumps the
/// revision: a revision-keyed comparison would rebuild the editor
/// behind the user's typing, dropping the keyboard focus and every
/// unsent draft in the other fields. The root the selection came from
/// is dropped on purpose: the editor writes the owner's parameter,
/// which root's graph proposed it does not change the page. While the
/// fresh details are in flight (the same edit put the old ones out of
/// date), the editor stays mounted for the same target; only a target
/// that moved, or no target at all, lets it go.
pub fn value_page_action(
    target: Option<(&str, &str)>,
    ready: bool,
    mounted: Option<(&str, &str)>,
) -> ValuePageAction {
    match (target, mounted) {
        (None, _) => ValuePageAction::Discard,
        (Some(_), current @ Some(_)) if !ready => {
            // No fresh row to build or refresh from — keep the mounted
            // editor only when it belongs to this target.
            match current {
                mounted if mounted == target => ValuePageAction::Keep,
                _ => ValuePageAction::Discard,
            }
        }
        (Some(_), mounted) if mounted == target => ValuePageAction::Refresh,
        (Some(_), _) => match ready {
            true => ValuePageAction::Mount,
            false => ValuePageAction::Discard,
        },
    }
}

/// Whether the session holds a live scene-node selection that a value
/// node's selection must retire: a
/// Hierarchy or viewport pick selected the node before, and it has to
/// go at both ends — the session's own marking (the Hierarchy row
/// follows) and the backend's, whose selected-scene-node refresh on
/// the next revision would re-emit `Update::ProjectInspector` and the
/// session handler would move the Inspector off the value page it is
/// showing.
pub fn value_selection_retires_node(selected_node: Option<&str>) -> bool {
    selected_node.is_some()
}

/// The resource node that took the place of the value node the
/// Inspector's colour page names, once that value node is gone: a menu
/// replacement ("New checkerboard") or a drop put a texture or a
/// SpectrumData on the same slot. The page would otherwise keep showing a
/// parameter that is no longer a spectrum value, so the node editor
/// selects the successor and the Inspector shows it. `None` while the
/// value node is still in the graph, when the page names another root,
/// or when nothing resource-like is on the slot (a plain disconnect: the
/// page then shows the slot's default).
pub fn value_successor(
    root: &str,
    graph: &Value,
    graph_value: Option<(&str, &str, &str)>,
) -> Option<String> {
    let (value_root, owner, parameter) = graph_value?;
    if value_root != root {
        return None;
    }
    let nodes = graph["nodes"].as_array()?;
    let value_id = value_node_id(owner, parameter);
    if nodes
        .iter()
        .any(|node| node["id"].as_str() == Some(value_id.as_str()))
    {
        return None;
    }
    let from = graph["edges"].as_array()?.iter().find_map(|edge| {
        (edge["to"].as_str() == Some(owner) && edge["parameter"].as_str() == Some(parameter))
            .then(|| edge["from"].as_str())
            .flatten()
    })?;
    nodes
        .iter()
        .find(|node| node["id"].as_str() == Some(from))
        .filter(|node| matches!(node["kind"].as_str(), Some("texture" | "spectrum_data")))
        .map(|_| from.to_owned())
}

/// The node a node editor highlights: the value node the Inspector's
/// colour page names while it belongs to this graph (the owner node
/// once an undo removed the value), else the Inspector's Contents item
/// when that item is a node of this graph. A selection anywhere else
/// leaves the editor without a highlight.
pub fn highlighted(
    graph: &Value,
    root: &str,
    graph_value: Option<(&str, &str, &str)>,
    inspector: &Value,
) -> Option<String> {
    let present = |graph: &Value, id: &str| {
        graph["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|node| node["id"].as_str() == Some(id))
    };
    if let Some((value_root, owner, parameter)) = graph_value
        && value_root == root
    {
        let id = value_node_id(owner, parameter);
        if present(graph, &id) {
            return Some(id);
        }
        // The value's owner is what the page edits; an undo may
        // have removed the value node while the page stays.
        if present(graph, owner) {
            return Some(owner.to_owned());
        }
    }
    let item = inspector["item"]["id"].as_str()?;
    present(graph, item).then(|| item.to_owned())
}

/// The parameter row a value node's page edits: the owner's parameter
/// row by the declared spelling the value node carries, declared or not
/// (an undo may have removed the declaration while the page stays the
/// slot's editor). A declaration that left names an alias no row
/// spells anymore — resolve it through the table's aliases so the page
/// follows the slot, not the spelling of a moment ago.
pub fn value_row(owner_details: &Value, parameter: &str) -> Option<Row> {
    let rows = parameter_rows::rows(&owner_details["parameters"]);
    if let Some(row) = rows.iter().find(|row| row.name == parameter) {
        return Some(row.clone());
    }
    let schema = owner_details["parameters"]["schema"].as_array()?;
    let name = schema.iter().find_map(|spec| {
        let spelled = |field: &str| spec[field].as_str() == Some(parameter);
        let aliased = spec["aliases"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|alias| alias.as_str() == Some(parameter));
        (spelled("name") || aliased)
            .then(|| spec["name"].as_str().map(str::to_owned))
            .flatten()
    })?;
    rows.into_iter().find(|row| row.name == name)
}

/// The default an empty slot shows grey, when the row carries one and
/// nothing sits in the slot: no declaration the row reports and no
/// edge arriving — an inline value, a texture or a SpectrumData would
/// spell declared_name; the edge covers anything else.
pub fn empty_slot_default(graph: &Value, owner: &str, input: &Value) -> Option<String> {
    let parameter = input["parameter"].as_str()?;
    let occupied = input.get("declared_name").is_some()
        || graph["edges"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|edge| edge["to"] == owner && edge["parameter"] == parameter);
    (!occupied)
        .then(|| input["default"].as_str().map(str::to_owned))
        .flatten()
}

/// The slot's declaration the value node's menu needs: the name and
/// value type as declared, and the table's own `required` flag. None
/// when the graph lost the row or the declaration. `required` says the
/// operand has no r4 default to fall back on — the menu keeps
/// Disconnect off such a slot.
pub fn slot_declaration(
    graph: &Value,
    owner: &str,
    parameter: &str,
) -> Option<(String, String, bool)> {
    let owner = graph["nodes"].as_array()?.iter().find(|node| {
        node["id"].as_str() == Some(owner)
            && matches!(node["kind"].as_str(), Some("material" | "texture"))
    })?;
    let input = owner["inputs"].as_array()?.iter().find(|input| {
        input["declared_name"].as_str() == Some(parameter)
            || input["parameter"].as_str() == Some(parameter)
    })?;
    Some((
        input["declared_name"]
            .as_str()
            .unwrap_or(parameter)
            .to_owned(),
        input["declared_type"].as_str()?.to_owned(),
        input["required"] == true,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(kind: &str, id: &str) -> Value {
        json!({"kind":kind,"id":id})
    }

    #[test]
    fn resource_nodes_select_like_contents_and_a_value_node_opens_its_parameter() {
        let id = "00000000-0000-0000-0000-0000000000aa";
        assert_eq!(
            click_target(&node("material", id)),
            Some(InspectorTarget::Contents(id.to_owned()))
        );
        assert_eq!(
            click_target(&node("texture", "t1")),
            Some(InspectorTarget::Contents("t1".to_owned()))
        );
        assert_eq!(
            click_target(&node("spectrum_data", "sd1")),
            Some(InspectorTarget::Contents("sd1".to_owned()))
        );
        assert_eq!(
            click_target(&json!({"kind":"spectrum_value","id":format!("{id}:k"),
                                 "owner":id,"parameter":"k","value_type":"rgb"})),
            Some(InspectorTarget::Value {
                owner: id.to_owned(),
                parameter: "k".to_owned()
            })
        );
        assert_eq!(click_target(&json!({})), None);
        // A value node without its owner or parameter is malformed: it
        // selects nothing rather than guessing.
        assert_eq!(
            click_target(&json!({"kind":"spectrum_value","id":"x:k"})),
            None
        );
    }

    #[test]
    fn value_node_ids_splice_owner_and_parameter() {
        assert_eq!(value_node_id("m", "k"), "m:k");
        assert_eq!(
            value_node_id("00000000-0000-0000-0000-0000000000aa", "conductor.k"),
            "00000000-0000-0000-0000-0000000000aa:conductor.k"
        );
    }

    #[test]
    fn the_highlight_follows_the_inspector_into_the_graph() {
        let graph = json!({
            "nodes":[node("material","m"),node("texture","t"),
                     {"kind":"spectrum_value","id":"m:k","owner":"m","parameter":"k"}],
        });
        // No target anywhere: no highlight.
        assert_eq!(highlighted(&graph, "m", None, &json!({})), None);
        // A value page this graph owns highlights its node…
        assert_eq!(
            highlighted(&graph, "m", Some(("m", "m", "k")), &json!({})),
            Some("m:k".to_owned())
        );
        // …another graph's value page does not.
        assert_eq!(
            highlighted(&graph, "m", Some(("other", "o", "k")), &json!({})),
            None
        );
        // An undo that dropped the value highlights the owner.
        let without_value = json!({"nodes":[node("material","m"),node("texture","t")]});
        assert_eq!(
            highlighted(&without_value, "m", Some(("m", "m", "k")), &json!({})),
            Some("m".to_owned())
        );
        // The Inspector's Contents item highlights its own node.
        assert_eq!(
            highlighted(&graph, "m", None, &json!({"item":{"id":"t"}})),
            Some("t".to_owned())
        );
        // An item that is not a node of this graph highlights nothing.
        assert_eq!(
            highlighted(&graph, "m", None, &json!({"item":{"id":"elsewhere"}})),
            None
        );
        // A scene node the Inspector picked
        // highlights nothing.
        assert_eq!(
            highlighted(&graph, "m", None, &json!({"node":{"node_id":"n1"}})),
            None
        );
    }

    #[test]
    fn the_value_page_names_the_row_by_its_declared_spelling() {
        let details = json!({
            "parameters":{
                "parameters":[
                    {"name":"k","value_type":"rgb",
                     "values":{"kind":"floats","value":[1., 0.8, 0.3]}},
                ],
                "schema":[
                    {"name":"k","aliases":[],"value_types":["spectrum","rgb","float"],
                     "default":null,"default_kind":"none","default_depends":null,
                     "texture":{"kind":"spectrum_unbounded"},"role":"texture_input",
                     "required":false,"group":null,"choices":[],"suggestions":[],"when":[]},
                ],
            },
        });
        let row = value_row(&details, "k").expect("the slot's row");
        assert_eq!(row.name, "k");
        assert!(row.declared, "the declaration the value node stands for");
        // The row a ParametersField would build — the colour page's
        // dialog re-uses it.
        assert!(row.known);
        assert!(
            row.value_types
                .iter()
                .any(|value_type| value_type == "spectrum")
        );
        assert!(value_row(&details, "eta").is_none());
        assert!(value_row(&json!({}), "k").is_none());
    }

    #[test]
    fn the_value_page_finds_an_aliased_row_after_its_declaration_left() {
        // The slot was declared under the alias and the value node
        // still names it; after the declaration's removal the page
        // reads the fresh details, where the unset row is spelled by
        // the table's own name — resolve through the schema's aliases.
        let aliased = json!({
            "parameters":{
                "parameters":[],
                "schema":[
                    {"name":"tex","aliases":["tex1"],
                     "value_types":["spectrum","rgb"],
                     "default":null,"default_kind":"none","default_depends":null,
                     "texture":{"kind":"spectrum_albedo"},"role":"texture_input",
                     "required":true,"group":null,"choices":[],"suggestions":[],"when":[]},
                ],
            },
        });
        let row = value_row(&aliased, "tex1").expect("the slot's row through its alias");
        assert_eq!(row.name, "tex");
        assert!(row.required);
        assert!(
            !row.declared,
            "the declaration left; the page stays its editor"
        );
        // A name neither a row nor an alias still finds nothing.
        assert!(value_row(&aliased, "tex2").is_none());
        assert!(value_row(&json!({}), "tex1").is_none());
    }

    #[test]
    fn a_vanished_value_node_keeps_asking_its_owner_page() {
        let graph = json!({"nodes":[node("material","m")]});
        let value = json!({
            "kind":"spectrum_value",
            "id":"m:k",
            "owner":"m",
            "parameter":"k",
            "value_type":"rgb",
        });
        let with_value = json!({"nodes":[node("material","m"), value]});
        // A value node present: its owner's page.
        assert_eq!(
            details_target("m", "m:k", &with_value, None),
            "m",
            "the value node's owner"
        );
        // A resource node: itself — even while a value page shows.
        assert_eq!(details_target("m", "m", &graph, Some(("m", "m", "k"))), "m");
        // The selected value node is gone — consumed by its own menu,
        // or the page's Data tab turned the value into a reference:
        // the owner keeps its page when the value page still names it.
        assert_eq!(
            details_target("m", "m:k", &graph, Some(("m", "m", "k"))),
            "m",
            "the fallback the Inspector's page needs"
        );
        // Another root's value page does not redirect an id, and an
        // unknown id passes through untouched.
        assert_eq!(
            details_target("m", "m:k", &graph, Some(("other", "o", "k"))),
            "m:k"
        );
        assert_eq!(
            details_target("m", "gone", &json!({"nodes":[]}), Some(("m", "m", "k"))),
            "gone"
        );
    }

    #[test]
    fn empty_slots_show_the_table_default_until_a_value_sits_in_them() {
        let conductor_k = json!({"parameter":"k","accepts":"spectrum","connectable":true,
                                  "spectrum_data":true,"default":"metal-Cu-k",
                                  "value_types":["spectrum","rgb","float"]});
        let material = json!({"id":"m","kind":"material","inputs":[conductor_k]});
        let graph = json!({"nodes":[material], "edges":[]});
        // Nothing declared, nothing wired: the default shows grey.
        assert_eq!(
            empty_slot_default(&graph, "m", &graph["nodes"][0]["inputs"][0]),
            Some("metal-Cu-k".to_owned())
        );
        // An edge in the slot — the value node, a texture, a
        // SpectrumData — replaces the grey default.
        let with_value = json!({"nodes":[material],
            "edges":[{"from":"m:k","to":"m","parameter":"k"}]});
        assert_eq!(
            empty_slot_default(&with_value, "m", &with_value["nodes"][0]["inputs"][0]),
            None
        );
        // A declaration the row reports does too.
        let declared = json!({"parameter":"k","accepts":"spectrum","connectable":true,
                              "spectrum_data":true,"default":"metal-Cu-k",
                              "declared_name":"k","declared_type":"rgb"});
        let graph = json!({"nodes":[json!({"id":"m","kind":"material","inputs":[declared]})],
                           "edges":[]});
        assert_eq!(
            empty_slot_default(&graph, "m", &graph["nodes"][0]["inputs"][0]),
            None
        );
        // A slot without a spelled default shows nothing, declared or
        // not.
        let bare = json!({"parameter":"reflectance","accepts":"spectrum","connectable":true,
                          "spectrum_data":true});
        let graph = json!({"nodes":[json!({"id":"m","kind":"material","inputs":[bare]})],
                           "edges":[]});
        assert_eq!(
            empty_slot_default(&graph, "m", &graph["nodes"][0]["inputs"][0]),
            None
        );
    }

    #[test]
    fn the_value_node_menu_names_the_declaration_the_slot_holds() {
        let inputs = json!([
            {"parameter":"k","accepts":"spectrum","connectable":true,"spectrum_data":true,
             "declared_name":"k","declared_type":"rgb"},
            {"parameter":"tex","aliases":null,"accepts":"spectrum","connectable":true,
             "spectrum_data":true,"required":true,
             "declared_name":"tex1","declared_type":"spectrum"},
        ]);
        let graph = json!({"nodes":[json!({"id":"m","kind":"material","inputs":inputs})]});
        assert_eq!(
            slot_declaration(&graph, "m", "k"),
            Some(("k".to_owned(), "rgb".to_owned(), false))
        );
        // The value declared under the alias spells the alias out — and
        // the table's required flag rides with it, so the menu can keep
        // Disconnect off an operand with no default.
        assert_eq!(
            slot_declaration(&graph, "m", "tex1"),
            Some(("tex1".to_owned(), "spectrum".to_owned(), true))
        );
        assert_eq!(slot_declaration(&graph, "m", "reflectance"), None);
        // A SpectrumData node or a texture is no owner of value menus.
        let leaf = json!({"nodes":[json!({"id":"sd","kind":"spectrum_data","inputs":[]})]});
        assert_eq!(slot_declaration(&leaf, "sd", "k"), None);
        assert_eq!(slot_declaration(&json!({}), "m", "k"), None);
    }

    #[test]
    fn the_value_page_keeps_one_editor_per_target_across_revisions() {
        use ValuePageAction::{Discard, Keep, Mount, Refresh};
        let target = Some(("m", "k"));
        // A pass whose details answered at the live project refreshes
        // the target's editor: the revision rides inside the row, not
        // in the editor's identity — the page after its own edit keeps
        // the editor, its focus and its unsent drafts.
        assert_eq!(
            value_page_action(target, true, Some(("m", "k"))),
            Refresh,
            "the same owner and parameter refreshes, whatever the revision"
        );
        // While those details are in flight (the own edit bumped the
        // project the old ones were read at), the editor stays mounted.
        assert_eq!(
            value_page_action(target, false, Some(("m", "k"))),
            Keep,
            "a pending refresh keeps the editor and its drafts"
        );
        // Another target mounts its own editor — ready or not, the
        // previous target's editor does not survive the move.
        assert_eq!(
            value_page_action(Some(("m", "reflectance")), true, Some(("m", "k"))),
            Mount
        );
        assert_eq!(
            value_page_action(Some(("o", "tex1")), false, Some(("m", "k"))),
            Discard,
            "a moved target discards the old editor until it answers"
        );
        // A first click: mount when the details are there, nothing
        // while they travel.
        assert_eq!(value_page_action(target, true, None), Mount);
        assert_eq!(value_page_action(target, false, None), Discard);
        // No value node selected: the page lets the editor go, ready
        // or not.
        assert_eq!(value_page_action(None, true, Some(("m", "k"))), Discard);
        assert_eq!(value_page_action(None, false, None), Discard);
    }

    #[test]
    fn a_value_selection_retires_a_live_scene_node_selection() {
        // Only a live scene-node selection — a Hierarchy pick or a
        // viewport pick, either way one the session still holds — needs
        // retiring, and it retires at both ends.
        assert!(value_selection_retires_node(Some("node-7")));
        assert!(!value_selection_retires_node(None));
    }

    /// A value node replaced by a texture (or a SpectrumData) hands the
    /// Inspector to that node; it stays put while the value node exists,
    /// for another root, and after a plain disconnect.
    #[test]
    fn a_replaced_value_node_hands_the_inspector_to_its_successor() {
        use super::value_successor;
        let replaced = serde_json::json!({
            "nodes":[{"id":"m","kind":"material"},{"id":"t","kind":"texture"}],
            "edges":[{"from":"t","to":"m","parameter":"reflectance"}]});
        assert_eq!(
            value_successor("m", &replaced, Some(("m", "m", "reflectance"))),
            Some("t".to_owned())
        );
        assert_eq!(
            value_successor("other", &replaced, Some(("m", "m", "reflectance"))),
            None
        );
        assert_eq!(value_successor("m", &replaced, None), None);
        let still_there = serde_json::json!({
            "nodes":[{"id":"m","kind":"material"},{"id":"m:reflectance","kind":"spectrum_value"}],
            "edges":[{"from":"m:reflectance","to":"m","parameter":"reflectance"}]});
        assert_eq!(
            value_successor("m", &still_there, Some(("m", "m", "reflectance"))),
            None
        );
        let disconnected = serde_json::json!({"nodes":[{"id":"m","kind":"material"}],"edges":[]});
        assert_eq!(
            value_successor("m", &disconnected, Some(("m", "m", "reflectance"))),
            None
        );
    }
}
