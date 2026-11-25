//! The raw-file reader splices `Include` trailing
//! parameters onto the included file's last directive, the way pbrt-v4
//! and r4 read the token stream and the project importer does
//! (pbrt-io/src/source.rs). smoke-plume splits a MakeNamedMedium's
//! parameter list across its density include this way.
use scene_parse::document::read_document;

#[test]
fn include_parameters_continue_the_last_directive_of_the_included_file() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("density.pbrt"),
        "MakeNamedMedium \"smoke\" \"string type\" [\"uniformgrid\"] \"integer nx\" [2]\n",
    )
    .unwrap();
    let root = home.path().join("scene.pbrt");
    std::fs::write(
        &root,
        "Include \"density.pbrt\" \"float sigma_a\" [4] \"float sigma_s\" [5]\n\
         WorldBegin\n\
         Shape \"sphere\"\n",
    )
    .unwrap();
    let document = read_document(&root).unwrap();
    let medium = document
        .directives
        .iter()
        .find(|directive| directive.name == "MakeNamedMedium")
        .expect("the medium from the included file");
    let names = medium
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["type", "nx", "sigma_a", "sigma_s"]);
}
