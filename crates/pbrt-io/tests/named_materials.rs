//! pbrt-v4 resolves `NamedMaterial` bindings after the whole file
//! is parsed, so a material defined later in the same import — directly or
//! through a later `Include` — still binds (contemporary-bathroom's light
//! materials work this way). A name that is never defined stays a warning
//! and its binding never reaches the project.
use pbrt_io::{ImportOptions, export_project, import_project};
use project::*;
use std::fs;

/// Import one scene (plus any extra files beside it) and return the
/// document and its import diagnostics.
fn import_text(source: &str, files: &[(&str, &str)]) -> (ProjectDocument, Vec<ImportDiagnostic>) {
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
    let diagnostics = session
        .document()
        .imports
        .values()
        .flat_map(|import| import.diagnostics.clone())
        .collect();
    (session.document().clone(), diagnostics)
}

/// The state nodes a scene's first world group holds, with the material
/// resource labels their bindings name.
fn group_state_materials(document: &ProjectDocument, group: usize) -> Vec<Option<String>> {
    let scene = document
        .scene(document.default_scene.expect("a default scene"))
        .unwrap();
    let view = document.default_scene_view().unwrap();
    let NodeKind::Scope { children, .. } = &view.node(scene.world).unwrap().data else {
        panic!("the world is a scope");
    };
    let NodeKind::Scope { children, .. } = &view.node(children[group]).unwrap().data else {
        panic!("the group is a scope");
    };
    children
        .iter()
        .filter_map(|id| match &view.node(*id).unwrap().data {
            NodeKind::State {
                operation: StateOperation::Material(MaterialBinding::Resource(id)),
            } => Some(Some(document.resources[id].label.clone())),
            NodeKind::State { .. } => Some(None),
            _ => None,
        })
        .collect()
}

#[test]
fn a_named_material_defined_later_in_the_same_file_binds() {
    let (document, diagnostics) = import_text(
        "WorldBegin\n\
         AttributeBegin\n\
         NamedMaterial \"light\"\n\
         AreaLightSource \"diffuse\" \"float scale\" [10]\n\
         Shape \"sphere\" \"float radius\" [0.25]\n\
         AttributeEnd\n\
         MakeNamedMaterial \"light\" \"string type\" [\"diffuse\"] \
           \"rgb reflectance\" [0.9 0.9 0.85]\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        group_state_materials(&document, 0),
        [Some("light".to_owned())],
        "the forward NamedMaterial binds the material the file defines later"
    );
    assert!(
        diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("\"light\"")),
        "no warning about a material that resolves: {diagnostics:#?}"
    );
}

#[test]
fn a_named_material_from_a_later_include_binds() {
    let (document, diagnostics) = import_text(
        "WorldBegin\n\
         AttributeBegin\n\
         NamedMaterial \"panel\"\n\
         Shape \"disk\" \"float radius\" [1]\n\
         AttributeEnd\n\
         Include \"materials.pbrt\"\n\
         WorldEnd\n",
        &[(
            "materials.pbrt",
            "MakeNamedMaterial \"panel\" \"string type\" [\"coateddiffuse\"]\n",
        )],
    );
    assert_eq!(
        group_state_materials(&document, 0),
        [Some("panel".to_owned())],
        "the material from the later Include binds the earlier reference"
    );
    assert!(
        diagnostics.is_empty(),
        "a resolvable forward reference is no warning: {diagnostics:#?}"
    );
}

#[test]
fn a_never_defined_named_material_stays_a_warning_and_drops_its_binding() {
    let (document, diagnostics) = import_text(
        "WorldBegin\n\
         AttributeBegin\n\
         NamedMaterial \"ghost\"\n\
         Shape \"sphere\" \"float radius\" [1]\n\
         AttributeEnd\n\
         WorldEnd\n",
        &[],
    );
    assert_eq!(
        group_state_materials(&document, 0),
        Vec::<Option<String>>::new(),
        "the sphere keeps the ambient material; the phantom binding node is gone"
    );
    let warning = diagnostics
        .iter()
        .find(|diagnostic| diagnostic.message.contains("\"ghost\""))
        .expect("the unresolved reference stays a warning");
    assert_eq!(
        (warning.code.as_str(), warning.severity),
        ("unsupported", Some(ImportSeverity::Warning)),
        "{warning:#?}"
    );
    assert!(
        warning
            .message
            .contains("refers to \"ghost\", which is not defined earlier (or was skipped)"),
        "{}",
        warning.message
    );
}

/// The export defines every named material before the world graph uses it
/// (all resources are emitted right after WorldBegin), so a scene that
/// only imports because of a late binding exports a file r4 accepts.
#[test]
fn a_resolved_forward_reference_exports_with_the_material_defined_first() {
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("scene.pbrt"),
        "WorldBegin\n\
         AttributeBegin\n\
         NamedMaterial \"light\"\n\
         Shape \"sphere\" \"float radius\" [0.25]\n\
         AttributeEnd\n\
         MakeNamedMaterial \"light\" \"string type\" [\"diffuse\"]\n\
         WorldEnd\n",
    )
    .unwrap();
    let (store, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let destination = home.path().join("bundle");
    export_project(&session.snapshot(), &store, &destination, || false).unwrap();
    let text = fs::read_to_string(destination.join("scene.pbrt")).unwrap();
    let definition = text.find("MakeNamedMaterial \"light\"").expect(&text);
    // The newline keeps this off the Make… definition line itself.
    let use_site = text
        .find("\nNamedMaterial \"light\"")
        .unwrap_or_else(|| panic!("no binding line:\n{text}"));
    assert!(
        definition < use_site,
        "the exported file defines the material before using it:\n{text}"
    );
}
