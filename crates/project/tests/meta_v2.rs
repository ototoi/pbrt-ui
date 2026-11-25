use project::meta_v2::MetaV2;
use serde_json::{Value, json};

/// An image is an ordinary Asset sidecar; Texture settings live elsewhere.
fn image() -> Value {
    json!({
        "format": "pbrt-ui.meta/2",
        "kind": "asset",
        "id": "20000000-0000-0000-0000-000000000002",
        "media_type": "image/png", "sha256": "a".repeat(64),
        "size": 42, "origin": null
    })
}

fn decode(value: &Value) -> project::Result<MetaV2> {
    MetaV2::decode(&serde_json::to_vec(value).unwrap())
}

#[test]
fn an_image_sidecar_roundtrips_as_an_asset() {
    let fixture = image();
    let decoded = decode(&fixture).unwrap();
    assert_eq!(serde_json::to_value(decoded).unwrap(), fixture);
}

#[test]
fn rejects_unknown_versions_kinds_and_fields() {
    for (pointer, value) in [
        ("/format", json!("pbrt-ui.meta/3")),
        ("/kind", json!("future_kind")),
        ("/kind", json!("image_texture")),
    ] {
        let mut fixture = image();
        *fixture.pointer_mut(pointer).unwrap() = value;
        assert!(decode(&fixture).is_err(), "{pointer}");
    }
    let mut fixture = image();
    fixture
        .as_object_mut()
        .unwrap()
        .insert("unexpected".into(), json!(true));
    assert!(decode(&fixture).is_err());
}

#[test]
fn rejects_invalid_asset_identity_hash_and_size() {
    for (pointer, value) in [
        ("/id", json!("00000000-0000-0000-0000-000000000000")),
        ("/sha256", json!("A".repeat(64))),
        ("/size", json!(4_u64 * 1024 * 1024 * 1024 + 1)),
        ("/media_type", json!("")),
    ] {
        let mut fixture = image();
        *fixture.pointer_mut(pointer).unwrap() = value;
        assert!(decode(&fixture).is_err(), "{pointer}");
    }
}

#[test]
fn scene_resource_and_regular_asset_have_distinct_field_sets() {
    for kind in ["scene", "resource", "asset"] {
        let mut fixture = json!({"format": "pbrt-ui.meta/2", "kind": kind,
            "id": "10000000-0000-0000-0000-000000000001"});
        if kind == "asset" {
            fixture["media_type"] = json!("application/octet-stream");
            fixture["sha256"] = json!("0".repeat(64));
            fixture["size"] = json!(0);
            fixture["origin"] = Value::Null;
        }
        decode(&fixture).unwrap();
        fixture["texture"] = json!({"texture_value_type": "spectrum"});
        assert!(decode(&fixture).is_err());
    }
}
