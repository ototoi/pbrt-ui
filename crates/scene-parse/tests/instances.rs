use scene_parse::{ParseOptions, parse_scene};
use std::fs;
fn preview(source: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, source).unwrap();
    parse_scene(&path, ParseOptions::default()).unwrap()
}
#[test]
fn multiple_placements_share_meshes_and_compose_definition_and_use_transforms() {
    let result = preview(
        "WorldBegin\nObjectBegin \"pair\"\nTranslate 1 0 0\nShape \"sphere\"\nTranslate 2 0 0\nShape \"disk\"\nObjectEnd\nAttributeBegin\nTranslate 10 0 0\nObjectInstance \"pair\"\nAttributeEnd\nTranslate -10 0 0\nObjectInstance \"pair\"\n",
    );
    assert_eq!(result.manifest.meshes.len(), 2);
    assert_eq!(result.blobs.len(), 2);
    assert_eq!(result.manifest.draws.len(), 4);
    let sphere: Vec<_> = result
        .manifest
        .draws
        .iter()
        .filter(|d| d.shape_type == "sphere")
        .collect();
    assert_eq!(sphere[0].mesh_id, sphere[1].mesh_id);
    assert_ne!(sphere[0].pick_id, sphere[1].pick_id);
    assert_eq!(sphere[0].bounds.as_ref().unwrap().center(), [11., 0., 0.]);
    assert_eq!(sphere[1].bounds.as_ref().unwrap().center(), [-9., 0., 0.]);
    assert_eq!(sphere[0].source.as_ref().unwrap().line, 4);
    assert_eq!(sphere[0].instance_source.as_ref().unwrap().source.line, 10);
    assert_eq!(sphere[1].instance_source.as_ref().unwrap().source.line, 13);
}
#[test]
fn definition_orientation_is_retained_and_use_orientation_does_not_override_it() {
    let result = preview(
        "WorldBegin\nObjectBegin \"part\"\nReverseOrientation\nShape \"sphere\"\nObjectEnd\nReverseOrientation\nObjectInstance \"part\"\n",
    );
    assert!(result.manifest.draws[0].reverse_orientation);
}
#[test]
fn forward_uses_resolve_and_unused_definitions_do_not_appear_on_their_own() {
    let result = preview(
        "WorldBegin\nObjectInstance \"part\"\nObjectBegin \"part\"\nShape \"sphere\"\nObjectEnd\nObjectBegin \"unused\"\nShape \"cone\"\nObjectEnd\n",
    );
    assert_eq!(result.manifest.meshes.len(), 1);
    assert_eq!(result.manifest.draws.len(), 1);
    assert_eq!(
        result.manifest.draws[0]
            .instance_source
            .as_ref()
            .unwrap()
            .name,
        "part"
    );
}
#[test]
fn unresolved_uses_are_diagnosed_and_animated_uses_receive_time_samples() {
    let result = preview(
        "WorldBegin\nObjectBegin \"part\"\nShape \"sphere\"\nObjectEnd\nObjectInstance \"missing\"\nActiveTransform StartTime\nTranslate 1 0 0\nActiveTransform All\nObjectInstance \"part\"\n",
    );
    assert_eq!(result.manifest.draws.len(), 1);
    assert_eq!(result.manifest.draws[0].world_from_object.len(), 61);
    assert!(
        result
            .diagnostics
            .iter()
            .any(|d| d.code == "instance_definition_unresolved")
    );
    assert!(
        !result
            .diagnostics
            .iter()
            .any(|d| d.code == "animated_instance_unsupported")
    );
}
#[test]
fn duplicate_definitions_and_nested_uses_are_rejected() {
    for source in [
        "ObjectBegin \"part\"\nObjectEnd\nObjectBegin \"part\"\nObjectEnd\n",
        "ObjectBegin \"part\"\nObjectInstance \"part\"\nObjectEnd\n",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("invalid.pbrt");
        fs::write(&path, source).unwrap();
        assert!(parse_scene(&path, ParseOptions::default()).is_err());
    }
}
