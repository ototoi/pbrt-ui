//! subsurface's `"string name"` (a measured scattering preset such as
//! "Skin1") imports with its material and exports back unchanged.
use pbrt_io::{ImportOptions, export_project};
use project::*;
use std::fs;

#[test]
fn subsurface_named_preset_survives_import_and_export() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\n\
         Material \"subsurface\" \"float scale\" [ 10 ] \"string name\" [ \"Skin1\" ] \"float eta\" [ 1.5 ]\n\
         Shape \"sphere\"\n",
    )
    .unwrap();
    let (store, session, report) = pbrt_io::import_project_with_report(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("material is dropped")),
        "{:?}",
        report.diagnostics
    );
    let material = session
        .document()
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::Material)
        .expect("the subsurface material");
    assert_eq!(material.implementation, "subsurface");
    let name = material
        .parameters
        .iter()
        .find(|parameter| parameter.name == "name")
        .unwrap();
    assert_eq!(name.values, ParameterValues::Strings(vec!["Skin1".into()]));

    let export = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(export).unwrap();
    assert!(text.contains("\"string name\" [ \"Skin1\" ]"), "{text}");
}
