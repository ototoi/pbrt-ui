use scene_parse::{ParseOptions, parse_scene};
use std::{fs, io::Write as _};

#[test]
fn concatenated_gzip_ply_members_validate_and_preview() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scene.pbrt");
    let asset = dir.path().join("split.ply.gz");
    fs::write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" [\"split.ply.gz\"]\n",
    )
    .unwrap();
    let header = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n";
    let body = b"0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
    let mut compressed = Vec::new();
    for member in [header.as_slice(), body.as_slice()] {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(member).unwrap();
        compressed.extend(encoder.finish().unwrap());
    }
    fs::write(&asset, compressed).unwrap();
    scene_parse::ply::validate_ply_gzip_file(
        &asset,
        scene_parse::ply::PlyValidationLimits::default(),
        || false,
    )
    .unwrap();

    let preview = parse_scene(&source, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 1);
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
}

#[test]
fn uppercase_ply_gzip_suffix_is_previewed() {
    let dir = tempfile::tempdir().unwrap();
    let source = dir.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" [\"tiny.PLY.GZ\"]\n",
    )
    .unwrap();
    let ply = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(ply).unwrap();
    fs::write(dir.path().join("tiny.PLY.GZ"), encoder.finish().unwrap()).unwrap();

    let preview = parse_scene(&source, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 1);
    assert!(preview.diagnostics.is_empty());
}
