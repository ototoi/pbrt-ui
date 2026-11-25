//! The Inspector's component add/remove menu rows.
//! Pure so the shapes can be tested without a window: which component rows
//! offer a Remove action, and which Add Component entries an object still
//! takes.
use serde_json::{Value, json};

/// The ⋮ menu's "Remove Component" edit for one component row, or `None`
/// when that row cannot be removed (Transform, the object's own body, rows
/// without a local state to clear).
///
/// Material and medium clear through `clear_local_binding` and only offer it
/// when the object holds its own binding state; an inherited binding is not
/// the object's to remove. An area light is always removable — removing a
/// shared one cancels it for this object — and reverse orientation turns its
/// effect off (cancelling an outer toggle when there is one). The render
/// entities delete their Options node, cameras excepted: they delete
/// from the Hierarchy with their detach cascade.
pub fn remove_edit(component: &Value, node_id: &str) -> Option<Value> {
    let node_id = node_id.to_owned();
    match component["kind"].as_str().unwrap_or_default() {
        "material" if component["local"] == true => Some(json!({
            "kind": "clear_local_binding", "node_id": node_id,
            "material": true, "medium": false,
        })),
        "medium" if component["local"] == true => Some(json!({
            "kind": "clear_local_binding", "node_id": node_id,
            "material": false, "medium": true,
        })),
        "area_light" => Some(json!({
            "kind": "set_area_light", "node_id": node_id, "light": null,
        })),
        // Reverse orientation reads as an on/off switch: removing means
        // turning it off.
        "reverse_orientation" if component["enabled"] == true => Some(json!({
            "kind": "set_reverse_orientation", "node_id": node_id, "enabled": false,
        })),
        "film" | "sampler" | "integrator" | "pixel_filter" | "accelerator" => {
            Some(json!({"kind": "delete_node", "node_id": component["node_id"]}))
        }
        _ => None,
    }
}

/// The ⋮ menu's "Remove Component" edits: `remove_edit`'s one edit, or for
/// the Alpha Mask (the user's 2026-10-29 ruling) one `set_parameter`
/// removal per declared alpha / shadowalpha, sent as one transaction.
pub fn remove_edits(component: &Value, node_id: &str) -> Option<Vec<Value>> {
    if component["kind"] != "alpha_mask" {
        return remove_edit(component, node_id).map(|edit| vec![edit]);
    }
    let target = &component["parameter_target"];
    let edits = component["declared"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            json!({"kind":"set_parameter","target":target,"name":p["name"],
                   "value_type":p["value_type"],"value":null})
        })
        .collect::<Vec<_>>();
    (!edits.is_empty()).then_some(edits)
}

/// `name` (alpha or shadowalpha) declared opaque: r4's alpha default is 1.
fn opaque(target: &Value, name: &str) -> Value {
    json!({"kind": "set_parameter", "target": target, "name": name, "value_type": "float",
           "value": {"name": name, "value_type": "float", "color_space": null,
                     "values": {"kind": "floats", "value": [1.0]}}})
}

/// Disconnecting a shape's alpha mask texture: the row goes back to the
/// opaque 1 in the same transaction, so the mask stays (the user's
/// 2026-10-31 ruling). None for any other row, which disconnects alone.
pub fn alpha_mask_disconnect(target: &Value, name: &str) -> Option<Vec<Value>> {
    (target["kind"] == "node" && matches!(name, "alpha" | "shadowalpha")).then(|| {
        vec![
            json!({"kind": "disconnect_texture", "target": target, "parameter": name}),
            opaque(target, name),
        ]
    })
}

/// The Add Component ▸ rows (label, edit) for one Inspector target — the
/// components the object can still take. Shapes keep Material and Medium
/// selector rows; Groups hide those bindings and expose only Area Light here.
///
/// - shape: Area Light and Reverse Orientation, until the object holds its
///   own (an inherited area light or outer toggle is still addable — adding
///   overrides them), and Alpha Mask until alpha or shadowalpha is declared;
/// - group: Area Light;
/// - the Scene root: the render entities the Options lack, each
///   creating its kind's default implementation; the existence map rides
///   the render settings box and stays what the menu reads, Film
///   included (the active Film is a Scene root component);
/// - lights, cameras, instances add nothing today (the Inspector still
///   shows the button and says so in its menu).
pub fn addable(node: &Value) -> Vec<(&'static str, Value)> {
    let node_id = node["node_id"].as_str().unwrap_or_default().to_owned();
    let components = node["components"].as_array().cloned().unwrap_or_default();
    let local = |kind: &str| {
        components
            .iter()
            .find(|component| component["kind"] == kind)
            .is_some_and(|component| component["local"] == true)
    };
    let entity = node["data"]["entity"].as_str();
    let mut rows = Vec::new();
    if node["data"]["kind"] == "scene" {
        // The menu offers Film, Sampler, Integrator, Pixel Filter, and Accelerator —
        // each only while the scene holds none.
        if let Some(scene) = components
            .iter()
            .find(|component| component["kind"] == "render_settings")
            // An older core without the existence map: offer nothing
            // rather than guess duplicates.
            .filter(|scene| scene["configurations"].is_object())
        {
            for (configuration, label) in [
                ("film", "Film"),
                ("sampler", "Sampler"),
                ("integrator", "Integrator"),
                ("pixel_filter", "Pixel Filter"),
                ("accelerator", "Accelerator"),
            ] {
                if scene["configurations"][configuration] != true {
                    rows.push((
                        label,
                        json!({"kind": "create_configuration",
                               "scene": scene["scene_id"], "configuration": configuration}),
                    ));
                }
            }
        }
        return rows;
    }
    // Only a shape emits; a group's imported area light stays editable.
    if entity == Some("shape") && !local("area_light") {
        // Omitted parameters use the unit-radiance default and the
        // table's constant defaults.
        rows.push((
            "Area Light",
            json!({"kind": "set_area_light", "node_id": node_id,
                   "light": {"implementation": "diffuse"}}),
        ));
    }
    // Adding reverse orientation while its effect is already on would be a
    // no-op: offer it only while the switch reads off and the object holds
    // no own toggle.
    let reverse_on = node["components"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|component| component["kind"] == "reverse_orientation")
        .is_some_and(|component| component["enabled"] == true);
    if entity == Some("shape") && !reverse_on && !local("reverse_orientation") {
        rows.push((
            "Reverse Orientation",
            json!({"kind": "set_reverse_orientation", "node_id": node_id, "enabled": true}),
        ));
    }
    // The Alpha Mask starts opaque.
    let masked = components.iter().any(|c| c["kind"] == "alpha_mask");
    if entity == Some("shape") && !masked {
        rows.push((
            "Alpha Mask",
            opaque(&json!({"kind": "node", "id": node_id}), "alpha"),
        ));
    }
    rows
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(components: Vec<Value>) -> Value {
        json!({
            "node_id": "6f0d8a4e-1111-4222-8333-444455556666",
            "data": {"kind": "entity", "entity": "shape"},
            "components": components,
        })
    }

    fn group(components: Vec<Value>) -> Value {
        json!({
            "node_id": "6f0d8a4e-1111-4222-8333-444455556666",
            "data": {"kind": "scope", "scope": "attribute"},
            "components": components,
        })
    }

    fn area_light(local: bool, id: &str) -> Value {
        json!({"kind": "area_light", "local": local, "node_id": id})
    }

    #[test]
    fn removable_rows_report_their_edits() {
        let node = "6f0d8a4e-1111-4222-8333-444455556666";
        // A local material binding clears; an inherited one is not offered.
        let local = json!({"kind":"material","local":true});
        let edit = remove_edit(&local, node).unwrap();
        assert_eq!(
            edit,
            json!({"kind":"clear_local_binding","node_id":node,"material":true,"medium":false})
        );
        let inherited = json!({"kind":"material","local":false});
        assert!(remove_edit(&inherited, node).is_none());
        let local_medium = json!({"kind":"medium","local":true});
        let edit = remove_edit(&local_medium, node).unwrap();
        assert_eq!(
            edit,
            json!({"kind":"clear_local_binding","node_id":node,"material":false,"medium":true})
        );
        // An area light is always removable — shared or own.
        for local in [true, false] {
            let edit = remove_edit(&area_light(local, "head-light"), node).unwrap();
            assert_eq!(
                edit,
                json!({"kind":"set_area_light","node_id":node,"light":null})
            );
        }
        // Reverse removal is "turn it off": offered while the effect is on
        // — local or inherited — never while it reads off, even with a
        // local cancelling toggle sitting in the group.
        let edit = remove_edit(
            &json!({"kind":"reverse_orientation","enabled":true,"local":true}),
            node,
        )
        .unwrap();
        assert_eq!(
            edit,
            json!({"kind":"set_reverse_orientation","node_id":node,"enabled":false})
        );
        let edit = remove_edit(
            &json!({"kind":"reverse_orientation","enabled":true,"local":false}),
            node,
        )
        .unwrap();
        assert_eq!(
            edit,
            json!({"kind":"set_reverse_orientation","node_id":node,"enabled":false})
        );
        assert!(
            remove_edit(
                &json!({"kind":"reverse_orientation","enabled":false,"local":true}),
                node
            )
            .is_none()
        );
        // Bodies and transforms never take the action.
        assert!(remove_edit(&json!({"kind":"transform"}), node).is_none());
        assert!(remove_edit(&json!({"kind":"shape"}), node).is_none());
        assert!(remove_edit(&json!({"kind":"object_instance"}), node).is_none());
    }

    /// Disconnecting a mask row's texture keeps the mask: the row goes
    /// back to the opaque 1 in the same transaction. Other rows have no
    /// such rule.
    #[test]
    fn disconnecting_an_alpha_mask_texture_restores_opaque() {
        let target = json!({"kind":"node","id":"n"});
        for name in ["alpha", "shadowalpha"] {
            assert_eq!(
                alpha_mask_disconnect(&target, name).unwrap(),
                [
                    json!({"kind":"disconnect_texture","target":target,"parameter":name}),
                    json!({"kind":"set_parameter","target":target,"name":name,"value_type":"float",
                           "value":{"name":name,"value_type":"float","color_space":null,
                                    "values":{"kind":"floats","value":[1.0]}}}),
                ]
            );
        }
        assert!(alpha_mask_disconnect(&target, "radius").is_none());
        let material = json!({"kind":"resource","id":"m"});
        assert!(alpha_mask_disconnect(&material, "alpha").is_none());
    }

    #[test]
    fn the_alpha_mask_adds_alpha_and_removes_every_declaration() {
        let node = "6f0d8a4e-1111-4222-8333-444455556666";
        let target = json!({"kind":"node","id":node});
        // Without a mask, shapes offer one: alpha declared opaque.
        let rows = addable(&shape(vec![]));
        let mask = rows.iter().find(|row| row.0 == "Alpha Mask").unwrap();
        assert_eq!(
            mask.1,
            json!({"kind":"set_parameter","target":target,"name":"alpha","value_type":"float",
                   "value":{"name":"alpha","value_type":"float","color_space":null,
                            "values":{"kind":"floats","value":[1.0]}}})
        );
        // With one, it is not addable again; removing it clears both in
        // one transaction.
        let component = json!({"kind":"alpha_mask","parameter_target":target,"declared":[
            {"name":"alpha","value_type":"texture"},{"name":"shadowalpha","value_type":"float"}]});
        let rows = addable(&shape(vec![component.clone()]));
        assert!(rows.iter().all(|row| row.0 != "Alpha Mask"), "{rows:?}");
        assert_eq!(
            remove_edits(&component, node).unwrap(),
            [
                json!({"kind":"set_parameter","target":target,"name":"alpha",
                       "value_type":"texture","value":null}),
                json!({"kind":"set_parameter","target":target,"name":"shadowalpha",
                       "value_type":"float","value":null}),
            ]
        );
        // Other rows keep their single edit.
        assert_eq!(
            remove_edits(&json!({"kind":"material","local":true}), node)
                .unwrap()
                .len(),
            1
        );
        assert!(remove_edits(&json!({"kind":"transform"}), node).is_none());
        // Groups and lights take no mask.
        assert!(
            addable(&group(vec![]))
                .iter()
                .all(|row| row.0 != "Alpha Mask")
        );
    }

    #[test]
    fn shapes_add_area_light_and_reverse_until_their_own_exist() {
        // Nothing yet: both, in that order.
        let rows = addable(&shape(vec![]));
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].0, "Area Light");
        assert_eq!(
            rows[0].1,
            json!({"kind":"set_area_light","node_id":"6f0d8a4e-1111-4222-8333-444455556666",
                   "light":{"implementation":"diffuse"}})
        );
        assert_eq!(rows[1].0, "Reverse Orientation");
        // An inherited area light still offers its own.
        let rows = addable(&shape(vec![area_light(false, "outer")]));
        assert_eq!(rows.len(), 3);
        // The object's own head light is not addable again, and reverse
        // orientation adds only while the switch reads off.
        let rows = addable(&shape(vec![
            area_light(true, "own"),
            json!({"kind":"reverse_orientation","enabled":true,"local":true}),
            json!({"kind":"alpha_mask","declared":[]}),
        ]));
        assert!(rows.is_empty(), "{rows:?}");
        let rows = addable(&shape(vec![
            json!({"kind":"reverse_orientation","enabled":true,"local":false}),
        ]));
        assert!(
            rows.iter().all(|row| row.0 != "Reverse Orientation"),
            "an inherited-on reverse is already on: {rows:?}"
        );
        // Groups add no area light: only a shape emits.
        let rows = addable(&group(vec![json!({"kind":"transform"})]));
        assert!(rows.is_empty(), "{rows:?}");
        // Lights, cameras and instances add nothing; a bare scene falls to
        // the render-entities rule below.
        for data in [
            json!({"kind":"entity","entity":"light"}),
            json!({"kind":"entity","entity":"camera"}),
            json!({"kind":"instance"}),
            // The World node, for completeness: not an add target either.
            json!({"kind":"scope","scope":"world"}),
        ] {
            let node = json!({"node_id":"n","data":data,"components":[]});
            assert!(addable(&node).is_empty(), "{data}");
        }
    }

    fn scene(components: Vec<Value>) -> Value {
        json!({
            "node_id": "6f0d8a4e-1111-4222-8333-444455556666",
            "data": {"kind": "scene"},
            "components": components,
        })
    }

    fn render_settings(configurations: Value) -> Value {
        json!({
            "kind":"render_settings",
            "scene_id":"scene-1",
            "camera":null,
            "cameras":[],
            "configurations":configurations,
        })
    }

    /// The Scene root's Add Component menu offers the render entities the
    /// Options lack, each creating its kind's default implementation; a
    /// report without the existence map adds nothing (the wire predates
    /// it), and the rows go each to its own scene.
    #[test]
    fn a_scene_root_adds_the_render_entities_it_lacks() {
        let none = json!({
            "film":false,"sampler":false,"integrator":false,
            "pixel_filter":false,"accelerator":false,
        });
        let rows = addable(&scene(vec![render_settings(none.clone())]));
        assert_eq!(
            rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            [
                "Film",
                "Sampler",
                "Integrator",
                "Pixel Filter",
                "Accelerator"
            ]
        );
        assert_eq!(
            rows[0].1,
            json!({"kind":"create_configuration","scene":"scene-1","configuration":"film"})
        );
        assert_eq!(
            rows[4].1,
            json!({"kind":"create_configuration","scene":"scene-1","configuration":"accelerator"})
        );
        // Each row carries its own scene, so a single-choice menu stays
        // correct.
        assert_ne!(rows[1].1, rows[2].1);
        // Present entities are not offered again — the Film counts like
        // one on the Scene root.
        let initial = json!({"film":true,"sampler":true,"integrator":true,
                             "pixel_filter":false,"accelerator":false});
        let rows = addable(&scene(vec![render_settings(initial)]));
        assert_eq!(
            rows.iter().map(|row| row.0).collect::<Vec<_>>(),
            ["Pixel Filter", "Accelerator"]
        );
        // A report without the existence map adds nothing.
        let rows = addable(&scene(vec![
            json!({"kind":"render_settings","scene_id":"s"}),
        ]));
        assert!(rows.is_empty(), "{rows:?}");
        // So does one without a render settings box.
        assert!(addable(&scene(vec![json!({"kind":"scene"})])).is_empty());
    }

    /// Render entity components are removed through the same wire
    /// delete_node that deletes options cameras; world-attached components
    /// never ride this rule.
    #[test]
    fn render_entity_components_remove_by_delete_node() {
        for (kind, label) in [
            ("film", "Film"),
            ("sampler", "Sampler"),
            ("integrator", "Integrator"),
            ("pixel_filter", "Pixel Filter"),
            ("accelerator", "Accelerator"),
        ] {
            let component = json!({"kind":kind, "node_id":"render-1",
                                   "implementation":label});
            let scene = "6f0d8a4e-1111-4222-8333-444455556666";
            assert_eq!(
                remove_edit(&component, scene),
                Some(json!({"kind":"delete_node","node_id":"render-1"})),
                "{kind}"
            );
        }
        // The camera component keeps refusing: cameras delete from the
        // Hierarchy, with their detach cascade.
        let camera = json!({"kind":"camera", "node_id":"cam"});
        assert!(remove_edit(&camera, "scene").is_none());
    }
}
