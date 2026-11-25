use scene_parse::{
    document::read_document,
    state_graph::{NodeRole, StateGraph, parse_state_graph},
};
use std::fs;
fn parse(source: &str) -> StateGraph {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("state.pbrt");
    fs::write(&path, source).unwrap();
    parse_state_graph(&read_document(&path).unwrap().directives).unwrap()
}
#[test]
fn transforms_are_persistent_nodes_and_attribute_end_restores_parent() {
    let graph = parse(
        "WorldBegin\nAttributeBegin\nTranslate 2 0 0\nScale 3 1 1\nShape \"sphere\"\nAttributeEnd\nShape \"disk\"\n",
    );
    assert_eq!(graph.nodes[2].role, NodeRole::State);
    assert_eq!(graph.nodes[3].parent, Some(2));
    assert_eq!(graph.nodes[4].parent, Some(3));
    assert_eq!(
        graph.nodes[4].state.start.transform_point([1., 0., 0.]),
        [5., 0., 0.]
    );
    assert_eq!(
        graph.nodes[2].state.start.transform_point([1., 0., 0.]),
        [3., 0., 0.]
    );
    assert_eq!(graph.nodes[6].parent, Some(0));
    assert_eq!(
        graph.nodes[6].state.start.transform_point([1., 0., 0.]),
        [1., 0., 0.]
    );
}
#[test]
fn transform_scope_keeps_attribute_changes_but_attribute_scope_restores_them() {
    let graph = parse(
        "WorldBegin\nTransformBegin\nTranslate 4 0 0\nNamedMaterial \"paint\"\nReverseOrientation\nTransformEnd\nShape \"sphere\"\n",
    );
    let state = &graph.nodes[6].state;
    assert_eq!(graph.nodes[5].role, NodeRole::State);
    assert_eq!(graph.nodes[6].parent, Some(5));
    assert_eq!(state.start.transform_point([0., 0., 0.]), [0., 0., 0.]);
    assert_eq!(state.material["name"], "paint");
    assert!(state.reverse);
    let graph = parse(
        "WorldBegin\nAttributeBegin\nNamedMaterial \"paint\"\nReverseOrientation\nAttributeEnd\nShape \"sphere\"\n",
    );
    assert_eq!(graph.nodes[5].state.material["kind"], "default");
    assert!(!graph.nodes[5].state.reverse);
}
#[test]
fn time_samples_and_named_coordinates_follow_active_transform_selection() {
    let graph = parse(
        "WorldBegin\nActiveTransform StartTime\nTranslate 1 0 0\nActiveTransform EndTime\nTranslate 2 0 0\nCoordinateSystem \"saved\"\nActiveTransform All\nIdentity\nActiveTransform EndTime\nCoordSysTransform \"saved\"\nShape \"sphere\"\n",
    );
    let state = &graph.nodes[10].state;
    assert_eq!(state.start.transform_point([0., 0., 0.]), [0., 0., 0.]);
    assert_eq!(state.end.transform_point([0., 0., 0.]), [2., 0., 0.]);
    assert_eq!(state.transform()["kind"], "animated");
}
#[test]
fn included_directives_keep_occurrence_source_and_inherited_state() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root.pbrt");
    fs::write(dir.path().join("shape.pbrt"), "Shape \"sphere\"\n").unwrap();
    fs::write(&root,"WorldBegin\nAttributeBegin\nTranslate 1 0 0\nInclude \"shape.pbrt\"\nAttributeEnd\nInclude \"shape.pbrt\"\n").unwrap();
    let document = read_document(&root).unwrap();
    let graph = parse_state_graph(&document.directives).unwrap();
    let shapes: Vec<_> = graph
        .nodes
        .iter()
        .filter(|n| document.directives[n.directive].name == "Shape")
        .collect();
    assert_eq!(shapes.len(), 2);
    assert_ne!(shapes[0].directive, shapes[1].directive);
    assert_eq!(
        shapes[0].state.start.transform_point([0., 0., 0.]),
        [1., 0., 0.]
    );
    assert_eq!(
        shapes[1].state.start.transform_point([0., 0., 0.]),
        [0., 0., 0.]
    );
    assert_eq!(document.directives[shapes[0].directive].line, 1);
}
#[test]
fn malformed_scope_is_reported_at_its_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.pbrt");
    fs::write(&path, "WorldBegin\nAttributeBegin\nTransformEnd\n").unwrap();
    let document = read_document(&path).unwrap();
    let error = parse_state_graph(&document.directives)
        .unwrap_err()
        .to_string();
    assert!(error.contains("bad.pbrt:3:"));
    assert!(error.contains("mismatched scope"));
}

#[test]
fn camera_coordinates_survive_world_reset_and_look_at_maps_eye_to_origin() {
    let graph = parse(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nCoordSysTransform \"camera\"\nShape \"sphere\"\n",
    );
    assert_eq!(
        graph.nodes[0].state.start.transform_point([0., 0., 5.]),
        [0., 0., 0.]
    );
    assert_eq!(
        graph.nodes[2].state.start.transform_point([0., 0., 0.]),
        [0., 0., 0.]
    );
    assert_eq!(
        graph.nodes[4].state.start.transform_point([0., 0., 0.]),
        [0., 0., 5.]
    );
}
