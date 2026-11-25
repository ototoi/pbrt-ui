//! Retain already loaded hierarchy branches when a project revision changes.
//!
//! The backend sends a fresh roots-only snapshot on every edit. Keeping the
//! previously loaded children avoids collapsing the tree; each retained node
//! is marked `stale` and refreshed by the Hierarchy one page at a time.
use serde_json::Value;
use std::collections::HashSet;

/// Copy loaded branches from `previous` into the fresh snapshot.
///
/// Only nodes present in both trees are retained, and only when the fresh
/// snapshot has not loaded them yet. The retained subtree keeps the new node's
/// label and child count; stale marks that its children predate the revision.
pub fn retain_loaded(fresh: &mut Value, previous: &Value) -> bool {
    let mut old = std::collections::HashMap::new();
    index(
        previous["nodes"]
            .as_array()
            .map(Vec::as_slice)
            .unwrap_or(&[]),
        &mut old,
    );
    let mut changed = false;
    if let Some(nodes) = fresh["nodes"].as_array_mut() {
        retain_nodes(nodes, &old, &mut changed);
    }
    changed
}

fn index<'a>(nodes: &'a [Value], out: &mut std::collections::HashMap<&'a str, &'a Value>) {
    for node in nodes {
        if let Some(id) = node["id"].as_str() {
            out.insert(id, node);
        }
        if let Some(children) = node["children"].as_array() {
            index(children, out);
        }
    }
}

fn retain_nodes(
    nodes: &mut [Value],
    old: &std::collections::HashMap<&str, &Value>,
    changed: &mut bool,
) {
    for node in nodes {
        let Some(id) = node["id"].as_str() else {
            continue;
        };
        if node["loaded"] != true {
            if let Some(previous) = old.get(id).filter(|previous| previous["loaded"] == true) {
                let object = node.as_object_mut().unwrap();
                for key in ["children", "more_offset"] {
                    if let Some(value) = previous.get(key) {
                        object.insert(key.into(), value.clone());
                    }
                }
                object.insert("loaded".into(), Value::Bool(true));
                if let Some(children) = object.get_mut("children") {
                    mark_stale(children);
                }
                object.insert("stale".into(), Value::Bool(true));
                *changed = true;
            }
        } else if let Some(children) = node
            .as_object_mut()
            .and_then(|object| object.get_mut("children"))
            .and_then(Value::as_array_mut)
        {
            // A freshly loaded node (the Scene root arrives with its
            // children) still keeps the branches loaded below it.
            retain_nodes(children, old, changed);
        }
    }
}

fn mark_stale(nodes: &mut Value) {
    if let Some(nodes) = nodes.as_array_mut() {
        for node in nodes {
            if let Some(object) = node.as_object_mut() {
                object.insert("stale".into(), Value::Bool(true));
                if let Some(children) = object.get_mut("children") {
                    mark_stale(children);
                }
            }
        }
    }
}

/// Whether this node's loaded children came from an older revision.
pub fn is_stale(tree: &Value, id: &str) -> bool {
    find(tree, id)
        .map(|node| node["stale"] == true)
        .unwrap_or(false)
}

fn find<'a>(tree: &'a Value, id: &str) -> Option<&'a Value> {
    fn walk<'a>(nodes: &'a [Value], id: &str) -> Option<&'a Value> {
        for node in nodes {
            if node["id"].as_str() == Some(id) {
                return Some(node);
            }
            if let Some(found) = walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                id,
            ) {
                return Some(found);
            }
        }
        None
    }
    walk(
        tree["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        id,
    )
}

/// A branch whose children on screen are missing or from an older revision:
/// stale, or expanded before its first page arrived (a reveal or a rebuild
/// can open a branch without the Hierarchy's expand event).
fn needs_refresh(node: &Value) -> bool {
    node["stale"] == true
        || (node["loaded"] != true && node["child_count"].as_u64().unwrap_or(0) > 0)
}

/// The next expanded stale or unloaded branch to refresh, at most one per sync.
pub fn next_stale_refresh(
    tree: &Value,
    expansion: &HashSet<String>,
    requested: &HashSet<String>,
) -> Option<String> {
    fn walk(
        nodes: &[Value],
        expansion: &HashSet<String>,
        requested: &HashSet<String>,
    ) -> Option<String> {
        for node in nodes {
            let id = node["id"].as_str()?;
            // A definition root's row is keyed by its resource, not its scope.
            let expanded = expansion.contains(&format!("p:{id}"))
                || node["resource_id"]
                    .as_str()
                    .is_some_and(|resource| expansion.contains(&format!("d:{resource}")));
            if needs_refresh(node) && expanded && !requested.contains(id) {
                return Some(id.to_owned());
            }
            if let Some(found) = walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                expansion,
                requested,
            ) {
                return Some(found);
            }
        }
        None
    }
    walk(
        tree["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        expansion,
        requested,
    )
}

/// Drop requests whose page arrived. A later snapshot of the same revision
/// may mark the branch stale again, and it must then be asked for again
/// rather than wait for an answer that already came.
pub fn answer_refresh_requests(arrived: &[String], requested: &mut HashSet<String>) {
    for parent in arrived {
        requested.remove(parent);
    }
}

/// Drop requests whose node disappeared or whose branch is fresh again.
pub fn prune_refresh_requests(tree: &Value, requested: &mut HashSet<String>) {
    requested.retain(|id| find(tree, id).is_some_and(needs_refresh));
}

/// Resolve a Hierarchy row id to the project node whose page it controls.
/// Definition rows use the resource id for selection, while paging uses the
/// definition scope node id.
pub fn refresh_node_for_row(tree: &Value, row_id: &str) -> Option<String> {
    if let Some(node_id) = row_id.strip_prefix("p:") {
        return find(tree, node_id).map(|_| node_id.to_owned());
    }
    let resource_id = row_id.strip_prefix("d:")?;
    fn walk(nodes: &[Value], resource_id: &str) -> Option<String> {
        for node in nodes {
            if node["kind"] == "definition_root"
                && node["resource_id"].as_str() == Some(resource_id)
            {
                return node["id"].as_str().map(str::to_owned);
            }
            if let Some(found) = walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                resource_id,
            ) {
                return Some(found);
            }
        }
        None
    }
    walk(tree["nodes"].as_array()?, resource_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn node(id: &str, loaded: bool, children: Value) -> Value {
        json!({
            "id": id, "label": id, "kind": "scope", "child_count": 1,
            "loaded": loaded, "children": children, "more_offset": null
        })
    }

    #[test]
    fn retained_branches_come_from_the_previous_revision_and_are_stale() {
        let previous = json!({"nodes":[node(
            "world",
            true,
            json!([node("attribute", true, json!([node("shape", false, json!([]))]))])
        )]});
        let mut fresh = json!({"nodes":[node("world", false, json!([]))]});
        assert!(retain_loaded(&mut fresh, &previous));
        assert_eq!(fresh["nodes"][0]["loaded"], true);
        assert_eq!(fresh["nodes"][0]["stale"], true);
        assert_eq!(fresh["nodes"][0]["children"][0]["id"], "attribute");
        assert_eq!(fresh["nodes"][0]["children"][0]["stale"], true);
        assert_eq!(
            fresh["nodes"][0]["children"][0]["children"][0]["id"],
            "shape"
        );
    }

    #[test]
    fn retention_keeps_new_metadata_and_ignores_missing_or_new_nodes() {
        let previous = json!({"nodes":[
            node("world", true, json!([node("gone", true, json!([]))])),
            node("old-only", true, json!([]))
        ]});
        let mut fresh = json!({"nodes":[
            json!({"id":"world","label":"World 2","kind":"scope","child_count":0,
                "loaded":false,"children":[],"more_offset":null}),
            node("added", false, json!([]))
        ]});
        assert!(retain_loaded(&mut fresh, &previous));
        assert_eq!(fresh["nodes"][0]["label"], "World 2");
        assert_eq!(fresh["nodes"][0]["child_count"], 0);
        assert_eq!(fresh["nodes"][0]["children"][0]["id"], "gone");
        assert!(
            fresh["nodes"]
                .as_array()
                .unwrap()
                .iter()
                .all(|n| n["id"] != "old-only")
        );
        assert_eq!(fresh["nodes"][1]["loaded"], false);
        assert!(fresh["nodes"][1].get("stale").is_none());
    }

    #[test]
    fn fresh_loaded_nodes_are_not_overwritten() {
        let previous =
            json!({"nodes":[node("world", true, json!([node("old", true, json!([]))]))]});
        let mut fresh =
            json!({"nodes":[node("world", true, json!([node("new", false, json!([]))]))]});
        assert!(!retain_loaded(&mut fresh, &previous));
        assert_eq!(fresh["nodes"][0]["children"][0]["id"], "new");
    }

    #[test]
    fn only_expanded_unrequested_stale_nodes_are_refreshed_one_at_a_time() {
        let tree = json!({"nodes":[
            json!({"id":"world","label":"world","kind":"scope","child_count":1,
                "loaded":true,"children":[
                    json!({"id":"attribute","label":"attribute","kind":"scope","child_count":0,
                        "loaded":true,"children":[],"more_offset":null,"stale":true})
                ],"more_offset":null,"stale":true}),
            json!({"id":"camera","label":"camera","kind":"scope","child_count":0,
                "loaded":true,"children":[],"more_offset":null,"stale":true})
        ]});
        let mut expansion: HashSet<String> = ["p:world", "p:camera"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        let mut requested = HashSet::new();
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &requested),
            Some("world".into())
        );
        requested.insert("world".into());
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &requested),
            Some("camera".into())
        );
        expansion.remove("p:camera");
        requested.insert("camera".into());
        assert_eq!(next_stale_refresh(&tree, &expansion, &requested), None);
        // A collapsed branch refreshes when opened again.
        expansion.insert("p:attribute".into());
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &requested),
            Some("attribute".into())
        );
    }

    #[test]
    fn branches_below_a_freshly_loaded_root_are_retained() {
        let previous = json!({"nodes":[node("scene", true, json!([
            node("world", true, json!([node("room", false, json!([]))]))
        ]))]});
        let mut fresh =
            json!({"nodes":[node("scene", true, json!([node("world", false, json!([]))]))]});
        assert!(retain_loaded(&mut fresh, &previous));
        let world = &fresh["nodes"][0]["children"][0];
        assert_eq!(world["loaded"], true);
        assert_eq!(world["stale"], true);
        assert_eq!(world["children"][0]["id"], "room");
    }

    #[test]
    fn expanded_branches_without_their_first_page_are_fetched() {
        let tree = json!({"nodes":[node("world", true, json!([node("room", false, json!([]))]))]});
        let expansion = HashSet::from(["p:room".to_owned()]);
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &HashSet::new()).as_deref(),
            Some("room")
        );
        let mut requested = HashSet::from(["room".to_owned()]);
        assert_eq!(next_stale_refresh(&tree, &expansion, &requested), None);
        prune_refresh_requests(&tree, &mut requested);
        assert!(requested.contains("room"), "still waiting for its page");
        let loaded = json!({"nodes":[node("world", true, json!([node("room", true, json!([]))]))]});
        prune_refresh_requests(&loaded, &mut requested);
        assert!(requested.is_empty());
    }

    #[test]
    fn an_open_definition_root_is_refreshed_by_its_resource_row() {
        let mut root = node("scope", false, json!([]));
        root["resource_id"] = json!("resource");
        let tree = json!({"nodes":[{"id":"@object-definitions","children":[root]}]});
        let expansion = HashSet::from(["d:resource".to_owned()]);
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &HashSet::new()).as_deref(),
            Some("scope")
        );
        assert_eq!(
            next_stale_refresh(&tree, &HashSet::new(), &HashSet::new()),
            None
        );
    }

    #[test]
    fn refresh_requests_are_pruned_when_a_branch_becomes_fresh_or_disappears() {
        let mut tree = json!({"nodes":[
            node("world", true, json!([])),
            json!({"id":"camera","label":"Camera","kind":"scope","child_count":1,
                "loaded":true,"children":[],"more_offset":null,"stale":true})
        ]});
        let mut requested: HashSet<String> = ["world", "camera", "gone"]
            .into_iter()
            .map(str::to_owned)
            .collect();
        prune_refresh_requests(&tree, &mut requested);
        assert_eq!(
            requested,
            ["camera"].into_iter().map(str::to_owned).collect()
        );
        tree["nodes"][1].as_object_mut().unwrap().remove("stale");
        prune_refresh_requests(&tree, &mut requested);
        assert!(requested.is_empty());
        assert!(is_stale(&tree, "world") == false);
    }

    #[test]
    fn a_branch_restaled_after_its_page_arrived_is_asked_for_again() {
        let mut root = node("scope", true, json!([]));
        root["stale"] = json!(true);
        let tree = json!({"nodes":[root]});
        let expansion = HashSet::from(["p:scope".to_owned()]);
        let mut requested = HashSet::from(["scope".to_owned()]);
        prune_refresh_requests(&tree, &mut requested);
        assert_eq!(next_stale_refresh(&tree, &expansion, &requested), None);
        answer_refresh_requests(&["scope".to_owned()], &mut requested);
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &requested).as_deref(),
            Some("scope")
        );
    }

    #[test]
    fn definition_row_resolves_to_scope_for_refresh_cleanup() {
        let tree = json!({"nodes":[{"id":"@object-definitions","children":[{
            "id":"scope","resource_id":"resource","kind":"definition_root",
            "loaded":true,"stale":true,"child_count":1,"children":[]
        }]}]});
        assert_eq!(
            refresh_node_for_row(&tree, "d:resource").as_deref(),
            Some("scope")
        );
        assert_eq!(
            refresh_node_for_row(&tree, "p:scope").as_deref(),
            Some("scope")
        );
        assert_eq!(refresh_node_for_row(&tree, "d:missing"), None);
        let expansion = HashSet::from(["d:resource".to_owned()]);
        let mut requested = HashSet::from(["scope".to_owned()]);
        assert_eq!(next_stale_refresh(&tree, &expansion, &requested), None);
        requested.remove(&refresh_node_for_row(&tree, "d:resource").unwrap());
        assert_eq!(
            next_stale_refresh(&tree, &expansion, &requested).as_deref(),
            Some("scope")
        );
    }
}
