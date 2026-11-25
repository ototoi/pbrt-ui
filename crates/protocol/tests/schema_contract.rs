//! The checked-in control-v2 schema is generated, and covers every wire method.
use std::{collections::BTreeSet, path::Path, process::Command};

fn workspace_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
}

fn schema() -> serde_json::Value {
    let path = workspace_root().join("schemas/control-v2.schema.json");
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
#[cfg_attr(windows, ignore = "needs python3; the generator check runs on Linux")]
fn test_control_schema_matches_generator() {
    let script = workspace_root().join("scripts/generate-control-schema.py");
    let output = Command::new("python3")
        .arg(&script)
        .arg("--check")
        .output()
        .expect("python3 is required to verify the generated control schema");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn test_control_schema_covers_every_wire_method() {
    let wire =
        std::fs::read_to_string(workspace_root().join("crates/protocol/src/wire.rs")).unwrap();
    let start = wire.find("pub enum Method {").unwrap();
    let end = start + wire[start..].find("\n}").unwrap();
    let methods = wire[start..end]
        .lines()
        .filter_map(|line| line.trim().strip_prefix("#[serde(rename = \""))
        .map(|rest| rest.trim_end_matches("\")]").to_owned())
        .collect::<BTreeSet<_>>();
    let schema = schema();
    let requests = schema["$defs"]["request"]["oneOf"]
        .as_array()
        .unwrap()
        .iter()
        .map(|variant| {
            variant["properties"]["method"]["const"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect::<BTreeSet<_>>();
    assert!(!methods.is_empty());
    assert_eq!(methods, requests);
    for method in &methods {
        assert!(
            schema["$defs"].get(format!("params.{method}")).is_some(),
            "missing params schema for {method}"
        );
    }
}

#[test]
fn project_item_schemas_use_the_wire_id_field() {
    let schema = schema();
    let edits = &schema["$defs"]["params.project.apply"]["properties"]["edits"]["items"]["oneOf"];
    for edit_kind in ["move_item", "delete_item"] {
        let edit = edits
            .as_array()
            .unwrap()
            .iter()
            .find(|variant| variant["properties"]["kind"]["const"] == edit_kind)
            .unwrap();
        let items = &edit["properties"]["item"]["oneOf"];
        for kind in ["scene", "resource", "asset"] {
            let item = items
                .as_array()
                .unwrap()
                .iter()
                .find(|variant| variant["properties"]["kind"]["const"] == kind)
                .unwrap();
            assert!(item["properties"].get("id").is_some(), "{edit_kind}/{kind}");
            assert!(
                item["properties"].get("value").is_none(),
                "{edit_kind}/{kind}"
            );
            assert!(item["required"].as_array().unwrap().contains(&"id".into()));
        }
    }
}

/// Appendix E.1 edit examples parse as `ProjectEdit` and use only the fields
/// their schema variant declares (and every field it requires).
#[test]
fn appendix_edits_match_the_rust_wire_and_schema() {
    let id = "209d7c20-ca6d-4965-a471-405c97e13662";
    let examples = [
        serde_json::json!({"kind":"create_preset","preset":id,"parent":id,"after":null,"definition":null}),
        serde_json::json!({"kind":"create_preset","preset":id,"parent":id}),
        serde_json::json!({"kind":"set_local_binding","node_id":id,"material":{"kind":"resource","value":id},"medium":null}),
        serde_json::json!({"kind":"set_local_binding","node_id":id,"medium":{"inside":id,"outside":null}}),
        serde_json::json!({"kind":"set_asset_reference","target":{"kind":"node","id":id},"name":"filename","asset":id}),
        serde_json::json!({"kind":"set_instance_definition","node_id":id,"definition":id}),
        serde_json::json!({"kind":"delete_node","node_id":id}),
        serde_json::json!({"kind":"duplicate_node","node_id":id}),
        serde_json::json!({"kind":"move_node","node_id":id,"parent":id,"after":null}),
        serde_json::json!({"kind":"move_node","node_id":id,"parent":id}),
        serde_json::json!({"kind":"set_scene_coordinate_system","scene":id,"up":"+Z","handedness":"right"}),
        serde_json::json!({"kind":"create_configuration","scene":id,"configuration":"film"}),
        serde_json::json!({"kind":"create_configuration","scene":id,"configuration":"sampler","implementation":"halton"}),
    ];
    let schema = schema();
    let edits = schema["$defs"]["params.project.apply"]["properties"]["edits"]["items"]["oneOf"]
        .as_array()
        .unwrap()
        .clone();
    for example in examples {
        serde_json::from_value::<protocol::project::ProjectEdit>(example.clone())
            .unwrap_or_else(|error| panic!("{example}: {error}"));
        let variant = edits
            .iter()
            .find(|variant| variant["properties"]["kind"]["const"] == example["kind"])
            .unwrap_or_else(|| panic!("no schema variant for {example}"));
        let fields = example.as_object().unwrap();
        for field in fields.keys() {
            assert!(
                variant["properties"].get(field).is_some(),
                "{example}: {field}"
            );
        }
        for field in variant["required"].as_array().unwrap() {
            assert!(fields.contains_key(field.as_str().unwrap()), "{example}");
        }
    }
}
