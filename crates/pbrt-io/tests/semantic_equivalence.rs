//! Export must preserve scene meaning, not text: importing the exported scene
//! yields the same signature as the original import, and every fixture's
//! signature is pinned so model refactors cannot silently change what an
//! import means. Set UPDATE_SEMANTIC_SNAPSHOTS=1 to rewrite the snapshots
//! after an intended change.
mod support;

use pbrt_io::{ImportOptions, export_project, import_project};
use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
};
use support::signature::{difference, signature};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

fn snapshot_path(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/semantic")
        .join(format!("{name}.json"))
}

/// Import `source`, export it, import the export, and return both signatures.
fn round_trip(source: &Path, home: &Path) -> (serde_json::Value, serde_json::Value) {
    let (store, session) = import_project(
        source,
        &home.join("original"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap_or_else(|error| panic!("import {}: {error}", source.display()));
    let original = signature(session.document(), &store);
    let exported = export_project(&session.snapshot(), &store, &home.join("bundle"), || false)
        .unwrap_or_else(|error| panic!("export {}: {error}", source.display()));
    let (store, session) = import_project(
        &exported,
        &home.join("reimported"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap_or_else(|error| panic!("re-import export of {}: {error}", source.display()));
    (original, signature(session.document(), &store))
}

fn check(name: &str, source: &Path) {
    let home = tempfile::tempdir().unwrap();
    let (original, reimported) = round_trip(source, home.path());
    if let Some(path) = difference(&original, &reimported) {
        panic!("{name}: export changed the scene meaning at {path}");
    }
    let snapshot = snapshot_path(name);
    let text = serde_json::to_string_pretty(&original).unwrap() + "\n";
    if std::env::var_os("UPDATE_SEMANTIC_SNAPSHOTS").is_some() {
        std::fs::create_dir_all(snapshot.parent().unwrap()).unwrap();
        std::fs::write(&snapshot, text).unwrap();
        return;
    }
    let pinned: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&snapshot).unwrap_or_else(|_| {
            panic!(
                "missing {}; run with UPDATE_SEMANTIC_SNAPSHOTS=1 to create it",
                snapshot.display()
            )
        }))
        .unwrap();
    if let Some(path) = difference(&pinned, &original) {
        panic!("{name}: import meaning differs from the pinned snapshot at {path}");
    }
}

/// A grid mesh whose point and index arrays exceed the bulk-array threshold,
/// so the round trip covers out-of-line arrays (and, later, PLY extraction).
fn large_mesh_scene(path: &Path) {
    const N: usize = 40;
    let mut points = String::new();
    for z in 0..=N {
        for x in 0..=N {
            let height = ((x * 7 + z * 3) % 11) as f64 * 0.01;
            write!(points, "{} {height} {} ", x as f64 * 0.25, z as f64 * 0.25).unwrap();
        }
    }
    let mut indices = String::new();
    for z in 0..N {
        for x in 0..N {
            let a = z * (N + 1) + x;
            let (b, c, d) = (a + 1, a + N + 1, a + N + 2);
            write!(indices, "{a} {c} {b} {b} {c} {d} ").unwrap();
        }
    }
    let scene = format!(
        "LookAt 5 5 15 5 0 5 0 1 0\nCamera \"perspective\"\nWorldBegin\nMaterial \"diffuse\"\nShape \"trianglemesh\" \"point3 P\" [{points}] \"integer indices\" [{indices}]\n"
    );
    std::fs::write(path, scene).unwrap();
}

#[test]
fn test_export_preserves_scene_meaning_for_each_fixture() {
    for name in [
        "minimal",
        "editor-preview",
        "instances-preview",
        "matrix-preview",
        "render-preview",
        "semantic-features",
    ] {
        check(name, &fixture(&format!("{name}.pbrt")));
    }
}

#[test]
fn test_export_preserves_meaning_of_large_out_of_line_arrays() {
    let home = tempfile::tempdir().unwrap();
    let source = home.path().join("large-mesh.pbrt");
    large_mesh_scene(&source);
    check("large-mesh", &source);
}
