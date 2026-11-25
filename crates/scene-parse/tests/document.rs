use scene_parse::document::read_document;
use std::{fs, io::Write};
#[test]
fn include_and_import_keep_directive_sources_and_deduplicate_dependency_files() {
    let dir = tempfile::tempdir().unwrap();
    // Parsed paths are canonical; macOS temp dirs sit behind the /var symlink.
    let base = protocol::path::canonicalize(dir.path()).unwrap();
    fs::write(base.join("child.pbrt"), "# comment\n  Shape \"sphere\"\n").unwrap();
    let root = base.join("root.pbrt");
    fs::write(
        &root,
        "WorldBegin\nInclude \"child.pbrt\"\nImport \"child.pbrt\"\n",
    )
    .unwrap();
    let document = read_document(&root).unwrap();
    assert_eq!(document.sources.len(), 2);
    assert_eq!(document.directives.len(), 3);
    for shape in &document.directives[1..] {
        assert_eq!((shape.line, shape.column), (2, 3));
        assert_eq!(shape.path, base.join("child.pbrt").to_str().unwrap());
    }
}
#[test]
fn dependency_cycles_fail_instead_of_recursing_forever() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("root.pbrt");
    fs::write(&root, "Include \"root.pbrt\"\n").unwrap();
    assert!(
        read_document(&root)
            .err()
            .unwrap()
            .to_string()
            .contains("cycle")
    );
}
#[test]
fn compressed_sources_are_indexed_with_their_actual_source_path() {
    let dir = tempfile::tempdir().unwrap();
    // Parsed paths are canonical; macOS temp dirs sit behind the /var symlink.
    let base = protocol::path::canonicalize(dir.path()).unwrap();
    let root = base.join("root.pbrt.gz");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(b"WorldBegin\nShape \"sphere\"\n")
        .unwrap();
    fs::write(&root, encoder.finish().unwrap()).unwrap();
    let document = read_document(&root).unwrap();
    assert_eq!(document.sources.len(), 1);
    assert_eq!(document.directives[1].line, 2);
    assert_eq!(document.directives[1].path, root.to_str().unwrap());
}
