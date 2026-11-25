use protocol::project::*;
use serde_json::json;
#[test]
fn project_guards_and_edits_are_strict_and_counters_are_decimal_strings() {
    let id = uuid::Uuid::new_v4();
    let guard = json!({"project_id":id,"revision":"9007199254740993","activation":"7"});
    let dto: ProjectRef = serde_json::from_value(guard.clone()).unwrap();
    assert_eq!(dto.revision.as_str(), "9007199254740993");
    let mut bad = guard.clone();
    bad["revision"] = json!(3);
    assert!(serde_json::from_value::<ProjectRef>(bad).is_err());
    let mut bad = guard.clone();
    bad["extra"] = json!(true);
    assert!(serde_json::from_value::<ProjectRef>(bad).is_err());
    assert!(serde_json::from_value::<ApplyProjectParams>(json!({"edits":[]})).is_err());
    let edit = json!({"project":guard,"edits":[{"kind":"set_transform","node_id":id,"value":{"kind":"translate","value":[1,2,3],"extra":true}}]});
    assert!(serde_json::from_value::<ApplyProjectParams>(edit).is_err());
    assert!(
        serde_json::from_value::<ParameterValues>(
            json!({"kind":"integers","value":[9007199254740993u64]})
        )
        .is_err()
    );
    let values: ParameterValues =
        serde_json::from_value(json!({"kind":"integers","value":["9007199254740993"]})).unwrap();
    assert!(matches!(values,ParameterValues::Integers(v) if v[0]=="9007199254740993"));
}

#[test]
fn spectrum_data_round_trips_as_a_resource_kind() {
    let id = uuid::Uuid::new_v4();
    let values: ParameterValues = serde_json::from_value(json!({
        "kind":"resources","value":{"kind":"spectrum_data","ids":[id]}
    }))
    .unwrap();
    let ParameterValues::Resources {
        kind: ResourceKind::SpectrumData,
        ids,
    } = values
    else {
        panic!("spectrum_data must deserialize as a resources kind");
    };
    assert_eq!(ids, vec![id]);
    assert_eq!(
        serde_json::to_value(ParameterValues::Resources {
            kind: ResourceKind::SpectrumData,
            ids: vec![id],
        })
        .unwrap(),
        json!({"kind":"resources","value":{"kind":"spectrum_data","ids":[id]}})
    );
    let edit: ProjectEdit = serde_json::from_value(json!({
        "kind":"create_resource","resource_kind":"spectrum_data",
        "implementation":"piecewise","label":"Gold k"
    }))
    .unwrap();
    assert!(matches!(
        edit,
        ProjectEdit::CreateResource {
            resource_kind: ResourceKind::SpectrumData,
            ..
        }
    ));
}

#[test]
fn create_resource_round_trips_with_optional_id() {
    let id = uuid::Uuid::new_v4();
    for value in [
        json!({"kind":"create_resource","resource_kind":"material","implementation":"diffuse","label":"diffuse"}),
        json!({"kind":"create_resource","id":id,"resource_kind":"material","implementation":"diffuse","label":"diffuse"}),
    ] {
        let edit: ProjectEdit = serde_json::from_value(value.clone()).unwrap();
        let encoded = serde_json::to_value(edit).unwrap();
        if value.get("id").is_some() {
            assert_eq!(encoded["id"], json!(id));
        } else {
            assert!(encoded.get("id").is_none());
        }
    }
}

#[test]
fn project_render_requires_snapshot_and_settings_guards_and_rejects_viewer_fields() {
    let value = json!({"project":{"project_id":uuid::Uuid::new_v4(),"revision":"2","activation":"1"},"expected_settings_revision":"3"});
    let params: RenderProjectParams = serde_json::from_value(value.clone()).unwrap();
    assert!(params.view.is_none());
    let mut invalid = value.clone();
    invalid["scene_id"] = json!(uuid::Uuid::new_v4());
    assert!(serde_json::from_value::<RenderProjectParams>(invalid).is_err());
    let mut invalid = value;
    invalid
        .as_object_mut()
        .unwrap()
        .remove("expected_settings_revision");
    assert!(serde_json::from_value::<RenderProjectParams>(invalid).is_err());
}

#[test]
fn make_object_definition_edit_round_trips() {
    let value = json!({"kind":"make_object_definition","node_id":uuid::Uuid::new_v4()});
    let parsed: ProjectEdit = serde_json::from_value(value.clone()).unwrap();
    assert!(matches!(
        parsed,
        ProjectEdit::MakeObjectDefinition { label: None, .. }
    ));
    assert_eq!(serde_json::to_value(parsed).unwrap(), value);
}
