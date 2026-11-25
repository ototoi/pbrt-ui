//! Real-scene verification: resolving re-parenting resets on import
//! keeps every evaluated placement in the pbrt-v4 scene fixtures
//! (sportscar, ganesha, hair-actual-bsdf, clouds) and leaves no resets
//! behind, and the hair scene's camera light group attaches to the camera.
//! Machine-bound, so the test is ignored by default and runs only against
//! a pbrt-v4-scenes checkout: pass `PBRT_V4_SCENES=<path>`, or keep the
//! checkout, at `../pbrt-v4-scenes` relative to
//! this repository.
mod support;

use pbrt_io::ImportOptions;
use pbrt_io::parse_unresolved;
use project::{NodeId, NodeKind, resolve_transform_parents};
use std::path::{Path, PathBuf};
use std::time::Instant;
use support::transform_parents::{
    Case, assert_documents_equivalent, assert_no_resets, cameras, node, trees, world_children,
};

/// The unresolved/resolved pair for one scene file, with caller-chosen
/// import budgets (these fixtures do not need more than the defaults).
fn resolve_scene(source: &Path, options: ImportOptions) -> Case {
    let before = parse_unresolved(source, &options).expect("the scene parses");
    let scene = before.default_scene.expect("import produced a scene");
    let mut after = before.clone();
    let diagnostics = resolve_transform_parents(&mut after, scene).expect("the scene resolves");
    Case {
        before,
        after,
        scene,
        diagnostics,
    }
}

fn scenes_root() -> Option<PathBuf> {
    let root = std::env::var_os("PBRT_V4_SCENES")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pbrt-v4-scenes"));
    root.is_dir().then_some(root)
}

#[test]
#[ignore = "requires the pbrt-v4-scenes checkout (PBRT_V4_SCENES=<path> or ../pbrt-v4-scenes)"]
fn real_scenes_keep_their_meaning_through_the_resolution() {
    let Some(root) = scenes_root() else {
        eprintln!("skipped: no pbrt-v4-scenes checkout; set PBRT_V4_SCENES");
        return;
    };
    let default = ImportOptions::default();
    for (label, tail, options) in [
        ("sportscar-sky", "sportscar/sportscar-sky.pbrt", &default),
        (
            "sportscar-area-lights",
            "sportscar/sportscar-area-lights.pbrt",
            &default,
        ),
        ("ganesha", "ganesha/ganesha.pbrt", &default),
        ("clouds", "clouds/clouds.pbrt", &default),
    ] {
        let started = Instant::now();
        let case = resolve_scene(&root.join(tail), options.clone());
        assert_documents_equivalent(&case.before, &case.after);
        assert_no_resets(&case);
        let nodes = trees(&case.after, case.scene).len();
        eprintln!(
            "{label}: {nodes} nodes equivalent, no transform-parent resets remaining ({:.3}s)",
            started.elapsed().as_secs_f64()
        );
    }
    // hair-actual-bsdf: the camera pose `Transform`, the camera light
    // group and the camera-following sphere all live in the top-level
    // file; the geometry includes carry the ~4.4M curves the native
    // format's 250k-node ceiling cannot host. Trim only those includes:
    // every re-parenting input of the real scene stays byte-identical.
    let started = Instant::now();
    let original = std::fs::read_to_string(root.join("hair/hair-actual-bsdf.pbrt"))
        .expect("hair-actual-bsdf reads");
    let trimmed: String = original
        .lines()
        .filter(|line| !line.trim_start().starts_with("Include \"geometry/"))
        .collect::<Vec<_>>()
        .join("\n");
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("hair-actual-bsdf.pbrt");
    std::fs::write(&source, &trimmed).unwrap();
    let case = resolve_scene(&source, default);
    assert_documents_equivalent(&case.before, &case.after);
    assert_no_resets(&case);
    assert_hair_camera_group(&case);
    let nodes = trees(&case.after, case.scene).len();
    eprintln!(
        "hair-actual-bsdf (geometry includes trimmed): {nodes} nodes equivalent, \
         no transform-parent resets remaining ({:.3}s)",
        started.elapsed().as_secs_f64()
    );
}

/// The hair scene's light group follows the camera — after
/// resolution a World-level scope names an Options camera in
/// `parent_camera`, which the Camera row lists as its child.
fn assert_hair_camera_group(case: &Case) {
    let document = &case.after;
    let scene = case.scene;
    let options = cameras(document, scene);
    let attached: Vec<NodeId> = world_children(document, scene)
        .into_iter()
        .filter(|id| {
            matches!(
                node(document, scene, *id).data,
                NodeKind::Scope {
                    parent_camera: Some(_),
                    ..
                }
            )
        })
        .collect();
    assert!(
        !attached.is_empty(),
        "hair: the camera light group must attach to the camera"
    );
    for id in &attached {
        let NodeKind::Scope {
            parent_camera: Some(camera),
            ..
        } = node(document, scene, *id).data
        else {
            unreachable!("filtered above");
        };
        assert!(
            options.contains(&camera),
            "hair: the attached group names an Options camera"
        );
    }
}
