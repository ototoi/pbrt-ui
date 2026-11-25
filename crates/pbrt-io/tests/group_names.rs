//! An `AttributeBegin`/`TransformBegin` scope whose label is
//! still the default "Group" takes its name from the first thing set
//! directly inside it, in priority order: the first `NamedMaterial`, then
//! the first `ObjectInstance`, then the first plymesh's file stem. A scope
//! with none of those stays "Group"; duplicates are allowed; shapes are
//! not renamed.
use pbrt_io::{ImportOptions, import_project};
use project::*;
use std::fs;

const PLY: &str = "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\n\
                   property float y\nproperty float z\nelement face 1\n\
                   property list uchar int vertex_indices\nend_header\n\
                   0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";

/// Direct children of the world: each group's own label and its direct
/// children's labels, so a test sees the name a scope was given.
fn world_groups(source: &str, files: &[(&str, &str)]) -> Vec<(String, Vec<String>)> {
    let home = tempfile::tempdir().unwrap();
    for (name, text) in files {
        let path = home.path().join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    fs::write(home.path().join("scene.pbrt"), source).unwrap();
    let (_, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let document = session.document();
    let scene = document
        .scene(document.default_scene.expect("a default scene"))
        .unwrap();
    let world = scene.world;
    let view = document.default_scene_view().unwrap();
    let label_of = |id: NodeId| -> String { view.node(id).expect("node exists").label.clone() };
    let labels_of = |ids: &Vec<NodeId>| -> Vec<String> {
        ids.iter()
            .map(|id| view.node(*id).expect("node exists").label.clone())
            .collect()
    };
    let NodeKind::Scope { children, .. } = &view.node(world).expect("world").data else {
        panic!("the world is a scope");
    };
    children
        .iter()
        .map(|id| {
            let NodeKind::Scope { children, .. } = &view.node(*id).expect("child").data else {
                panic!("world children are scopes");
            };
            (label_of(*id), labels_of(children))
        })
        .collect()
}

#[test]
fn a_scope_with_a_named_material_takes_its_name_and_beats_a_later_instance() {
    let groups = world_groups(
        "WorldBegin\n\
         MakeNamedMaterial \"inox_pied\" \"string type\" [\"conductor\"]\n\
         ObjectBegin \"wheel\"\nShape \"sphere\"\nObjectEnd\n\
         AttributeBegin\n\
         ObjectInstance \"wheel\"\n\
         NamedMaterial \"inox_pied\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        groups,
        [(
            "inox_pied".to_owned(),
            vec![
                "ObjectInstance".to_owned(),
                "NamedMaterial".to_owned(),
                "Shape".to_owned()
            ]
        )],
        "NamedMaterial wins over the earlier ObjectInstance, and the shape keeps its label"
    );
}

#[test]
fn a_scope_with_only_an_object_instance_takes_its_name() {
    let groups = world_groups(
        "WorldBegin\n\
         ObjectBegin \"crown\"\nShape \"sphere\"\nObjectEnd\n\
         AttributeBegin\n\
         Translate 1 0 0\n\
         ObjectInstance \"crown\"\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        groups,
        [(
            "crown".to_owned(),
            vec!["Translate".to_owned(), "ObjectInstance".to_owned(),]
        )]
    );
}

#[test]
fn a_scope_with_only_a_plymesh_takes_the_file_stem() {
    let groups = world_groups(
        "WorldBegin\n\
         AttributeBegin\n\
         Shape \"plymesh\" \"string filename\" \"geometry/mesh_00001.ply\"\n\
         Shape \"sphere\"\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[("geometry/mesh_00001.ply", PLY)],
    );
    assert_eq!(
        groups,
        [(
            "mesh_00001".to_owned(),
            vec!["Shape".to_owned(), "Shape".to_owned()]
        )],
        "the group takes the ply file stem; the shapes stay \"Shape\""
    );
}

#[test]
fn a_scope_with_no_usable_names_stays_group() {
    let groups = world_groups(
        "WorldBegin\n\
         AttributeBegin\n\
         Translate 1 0 0\n\
         Shape \"sphere\"\n\
         AttributeEnd\n\
         TransformBegin\n\
         Shape \"disk\"\n\
         TransformEnd\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        groups,
        [
            (
                "Group".to_owned(),
                vec!["Translate".to_owned(), "Shape".to_owned()]
            ),
            ("Group".to_owned(), vec!["Shape".to_owned()],)
        ]
    );
}

#[test]
fn the_first_name_wins_and_nested_scopes_do_not_name_their_parent() {
    let groups = world_groups(
        "WorldBegin\n\
         MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"]\n\
         MakeNamedMaterial \"silver\" \"string type\" [\"conductor\"]\n\
         AttributeBegin\n\
         NamedMaterial \"gold\"\n\
         AttributeBegin\n\
         NamedMaterial \"silver\"\n\
         Shape \"sphere\"\n\
         AttributeEnd\n\
         Shape \"disk\"\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        groups,
        [(
            "gold".to_owned(),
            vec![
                "NamedMaterial".to_owned(),
                "silver".to_owned(),
                "Shape".to_owned(),
            ]
        )],
        "first NamedMaterial wins; the inner scope's material does not name the outer scope"
    );
}

#[test]
fn two_scopes_may_take_the_same_name() {
    let groups = world_groups(
        "WorldBegin\n\
         MakeNamedMaterial \"mat\" \"string type\" [\"conductor\"]\n\
         AttributeBegin\n\
         NamedMaterial \"mat\"\n\
         Shape \"sphere\"\n\
         AttributeEnd\n\
         AttributeBegin\n\
         NamedMaterial \"mat\"\n\
         Shape \"disk\"\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[],
    );
    let names = groups
        .iter()
        .map(|(label, _)| label.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["mat", "mat"], "duplicates are allowed: {groups:?}");
}

#[test]
fn a_scope_left_open_at_world_end_still_takes_its_name() {
    let groups = world_groups(
        "WorldBegin\n\
         MakeNamedMaterial \"panel\" \"string type\" [\"diffuse\"]\n\
         AttributeBegin\n\
         NamedMaterial \"panel\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        groups,
        [(
            "panel".to_owned(),
            vec!["NamedMaterial".to_owned(), "Shape".to_owned()]
        )],
        "the forced close at WorldEnd names the scope like a real one would"
    );
}

#[test]
fn a_scope_left_open_at_the_end_of_the_file_still_takes_its_name() {
    let groups = world_groups(
        "WorldBegin\n\
         MakeNamedMaterial \"panel\" \"string type\" [\"diffuse\"]\n\
         AttributeBegin\n\
         NamedMaterial \"panel\"\n\
         Shape \"sphere\" \"float radius\" [0.5]\n",
        &[],
    );
    assert_eq!(
        groups,
        [(
            "panel".to_owned(),
            vec!["NamedMaterial".to_owned(), "Shape".to_owned()]
        )],
        "the forced close at the end of the file names the scope like a real close"
    );
}
