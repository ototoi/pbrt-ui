//! Hierarchy delete / duplicate / move: which rows qualify and the edits
//! they send. The core acts on an object together with its dedicated group.
//! Pure so the rules can be tested without a window.
use serde_json::{Value, json};

/// Scene-graph objects that can be deleted, duplicated and dragged.
pub fn movable(object: &str) -> bool {
    matches!(object, "group" | "shape" | "light" | "instance")
}

/// Objects with node-level Duplicate and Delete commands. Cameras are
/// option nodes, so they can be cloned/deleted but cannot be moved into World.
pub fn duplicate_delete(object: &str) -> bool {
    movable(object) || object == "camera"
}

pub fn can_unpack_instance(object: &str, editable: bool) -> bool {
    editable && object == "instance"
}

/// Rows a dragged object can be dropped on. Scene is the visible root for
/// World content; the core maps it back to the model's World scope. A drop
/// on a Camera attaches the unit to that camera (plan: move_node's
/// camera-parent target).
pub fn accepts_children(object: &str) -> bool {
    matches!(object, "group" | "scene" | "camera")
}

pub fn delete_edit(node: &str) -> Value {
    json!({"kind":"delete_node","node_id":node})
}

pub fn duplicate_edit(node: &str) -> Value {
    json!({"kind":"duplicate_node","node_id":node})
}

/// Convert a selected group into an object definition and instance.
pub fn make_object_definition_edit(node: &str) -> Value {
    json!({"kind":"make_object_definition","node_id":node})
}

/// Replace an object instance with an editable group containing its definition.
pub fn unpack_object_instance_edit(node: &str) -> Value {
    json!({"kind":"unpack_object_instance","node_id":node})
}

/// Move to the end of `parent`. Dropping a row on itself sends nothing.
pub fn move_edit(node: &str, parent: &str) -> Option<Value> {
    (node != parent)
        .then(|| json!({"kind":"move_node","node_id":node,"parent":parent,"after":null}))
}

/// The visible parent where a duplicate is revealed. Owned implementation
/// scopes are already absent from the scene tree.
pub fn duplicate_parent(tree: &Value, node: &str) -> Option<String> {
    let ancestors = super::create_menu::ancestors(tree, node);
    ancestors.first().cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree() -> Value {
        json!({"nodes":[{"id":"scene","object":"scene","children":[
            {"id":"camera","object":"camera","children":[]},
            {"id":"sphere","object":"shape","children":[]},
            {"id":"lonely","object":"group","children":[
                {"id":"statue","object":"shape","children":[]}
            ]},
            {"id":"room","object":"group","children":[
                {"id":"floor","object":"shape","children":[]},
                {"id":"wall","object":"shape","children":[]}
            ]}
        ]}]})
    }

    #[test]
    fn node_edits_move_world_objects_and_cameras_take_drops() {
        for object in ["group", "shape", "light", "instance"] {
            assert!(movable(object), "{object}");
        }
        assert!(!movable("camera"));
        assert!(duplicate_delete("camera"));
        assert!(!duplicate_delete("scene"));
        assert!(!duplicate_delete(""));
        assert!(can_unpack_instance("instance", true));
        assert!(!can_unpack_instance("instance", false));
        assert!(!can_unpack_instance("group", true));
        assert!(accepts_children("scene"));
        assert!(accepts_children("group"));
        assert!(accepts_children("camera"));
        assert!(!accepts_children("shape"));
        assert!(!accepts_children("instance"));
        assert!(!accepts_children(""));
    }

    #[test]
    fn node_edits_match_the_wire() {
        assert_eq!(
            delete_edit("n"),
            json!({"kind":"delete_node","node_id":"n"})
        );
        assert_eq!(
            duplicate_edit("n"),
            json!({"kind":"duplicate_node","node_id":"n"})
        );
        assert_eq!(
            move_edit("n", "g"),
            Some(json!({"kind":"move_node","node_id":"n","parent":"g","after":null}))
        );
        assert_eq!(move_edit("g", "g"), None);
        let node = uuid::Uuid::new_v4().to_string();
        let edit = make_object_definition_edit(&node);
        assert_eq!(edit["kind"], "make_object_definition");
        serde_json::from_value::<protocol::project::ProjectEdit>(edit).unwrap();

        let edit = unpack_object_instance_edit(&node);
        assert_eq!(edit["kind"], "unpack_object_instance");
        serde_json::from_value::<protocol::project::ProjectEdit>(edit).unwrap();
    }

    #[test]
    fn a_duplicate_is_revealed_where_the_core_puts_it() {
        assert_eq!(
            duplicate_parent(&tree(), "sphere").as_deref(),
            Some("scene")
        );
        assert_eq!(
            duplicate_parent(&tree(), "statue").as_deref(),
            Some("lonely"),
            "single-child ordinary groups remain visible and own their duplicates"
        );
        assert_eq!(duplicate_parent(&tree(), "floor").as_deref(), Some("room"));
        assert_eq!(duplicate_parent(&tree(), "room").as_deref(), Some("scene"));
        assert_eq!(duplicate_parent(&tree(), "missing"), None);
    }
}
