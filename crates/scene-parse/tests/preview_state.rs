use scene_parse::{
    ParseOptions, document::ReadOptions, document::read_document,
    document::read_document_with_options, parse_scene, state_graph::parse_state_graph,
};
use std::{fs, io::Write};

#[test]
fn preview_draws_match_parser_states_including_transform_scope_restoration() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "LookAt 0 0 -5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nAttributeBegin\nTranslate 2 0 0\nTransformBegin\nScale 2 3 4\nReverseOrientation\nShape \"sphere\"\nTransformEnd\nShape \"disk\"\nAttributeEnd\nShape \"cone\"\n").unwrap();
    let document = read_document(&path).unwrap();
    let graph = parse_state_graph(&document.directives).unwrap();
    let states: Vec<_> = graph
        .nodes
        .iter()
        .filter(|node| document.directives[node.directive].name == "Shape")
        .map(|node| &node.state)
        .collect();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), states.len());
    for ((draw, mesh), state) in preview
        .manifest
        .draws
        .iter()
        .zip(&preview.manifest.meshes)
        .zip(states)
    {
        let origin = mesh.origin_f64;
        let expected = state.start.mul(scene_parse::Mat4::translation(
            origin[0], origin[1], origin[2],
        ));
        for (actual, expected) in draw.world_from_object[0].iter().zip(expected.0) {
            assert!((actual - expected).abs() < 1e-10);
        }
        assert_eq!(draw.reverse_orientation, state.reverse);
    }
    assert!(preview.manifest.draws[1].reverse_orientation);
    assert!(!preview.manifest.draws[2].reverse_orientation);
    let bounds = &preview.manifest.meshes[2].bounds;
    assert!((bounds.min[0] + bounds.max[0]).abs() < 1e-10);
    assert_eq!(bounds.min[2], 0.);
}

#[test]
fn active_sample_transforms_are_interpolated_and_do_not_leak_from_scope() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "WorldBegin\nAttributeBegin\nActiveTransform StartTime\nTranslate 1 0 0\nActiveTransform All\nShape \"sphere\"\nAttributeEnd\nShape \"disk\"\n").unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 2);
    assert_eq!(preview.manifest.draws[0].source.as_ref().unwrap().line, 6);
    assert_eq!(preview.manifest.draws[1].source.as_ref().unwrap().line, 8);
    assert_eq!(preview.manifest.supported_times.len(), 61);
    let matrices = &preview.manifest.draws[0].world_from_object;
    assert_eq!(matrices.len(), 61);
    assert!((matrices[0][3] - matrices[60][3] - 1.0).abs() < 1e-8);
}

#[test]
fn named_coordinates_place_preview_geometry_at_the_restored_transform() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "WorldBegin\nTranslate 4 5 6\nCoordinateSystem \"saved\"\nIdentity\nCoordSysTransform \"saved\"\nShape \"sphere\"\n").unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    let bounds = &preview.manifest.meshes[0].bounds;
    assert_eq!(bounds.center(), [4., 5., 6.]);
}

#[test]
fn gzip_includes_keep_source_locations_and_share_parent_transform_state() {
    let dir = tempfile::tempdir().unwrap();
    // Parsed paths are canonical; macOS temp dirs sit behind the /var symlink.
    let base = protocol::path::canonicalize(dir.path()).unwrap();
    let path = base.join("scene.pbrt");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(b"Shape \"sphere\"\n").unwrap();
    let child = base.join("shape.pbrt.gz");
    fs::write(&child, encoder.finish().unwrap()).unwrap();
    fs::write(
        &path,
        "WorldBegin\nTranslate 2 0 0\nInclude \"shape.pbrt.gz\"\n",
    )
    .unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    let draw = &preview.manifest.draws[0];
    assert_eq!(draw.world_from_object[0][3], 2.);
    assert_eq!(draw.source.as_ref().unwrap().path, child.to_str().unwrap());
    assert_eq!(draw.source.as_ref().unwrap().line, 1);
}

#[test]
fn bilinearmesh_without_indices_uses_sequential_quad_control_points() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "WorldBegin\nShape \"bilinearmesh\" \"point3 P\" [ 0 0 0 1 0 0 0 1 0 1 1 0 ]\n",
    )
    .unwrap();

    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 1);
    assert_eq!(preview.manifest.meshes[0].indices.count, 8 * 8 * 6);
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn included_ply_resolves_from_scene_root_and_reads_gzip_assets() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("scene.pbrt");
    let nested = dir.path().join("geometry");
    fs::create_dir_all(&nested).unwrap();
    fs::write(&root, "Include \"geometry/mesh.pbrt\"\n").unwrap();
    fs::write(
        nested.join("mesh.pbrt"),
        "Shape \"plymesh\" \"string filename\" [ \"geometry/tiny.ply.gz\" ]\n",
    )
    .unwrap();

    let ply = b"ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n";
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(ply).unwrap();
    fs::write(
        dir.path().join("geometry/tiny.ply.gz"),
        encoder.finish().unwrap(),
    )
    .unwrap();

    let preview = parse_scene(&root, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 1);
    assert_eq!(preview.manifest.meshes[0].indices.count, 3);
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn preview_geometry_budget_keeps_partial_scene_and_reports_skipped_shape() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "Camera \"perspective\"\nShape \"sphere\"\n").unwrap();

    let preview = parse_scene(
        &path,
        ParseOptions {
            max_geometry_bytes: 1,
            ..ParseOptions::default()
        },
    )
    .unwrap();
    assert!(preview.manifest.meshes.is_empty());
    assert!(preview.manifest.camera.is_some());
    assert_eq!(preview.diagnostics[0].code, "preview_geometry_limit");
}

#[test]
fn default_preview_geometry_budget_is_256_mib() {
    assert_eq!(
        ParseOptions::default().max_geometry_bytes,
        256 * 1024 * 1024
    );
}

#[test]
fn ply_asset_budget_reports_failure_without_loading_oversized_data() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        dir.path().join("mesh.ply"),
        "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\nproperty float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n",
    )
    .unwrap();
    fs::write(
        &path,
        "Shape \"plymesh\" \"string filename\" [ \"mesh.ply\" ]\n",
    )
    .unwrap();

    let preview = parse_scene(
        &path,
        ParseOptions {
            max_asset_bytes: 8,
            ..ParseOptions::default()
        },
    )
    .unwrap();
    assert!(preview.manifest.meshes.is_empty());
    assert_eq!(preview.diagnostics[0].code, "geometry_failed");
    assert!(
        preview.diagnostics[0]
            .message
            .contains("preview asset budget")
    );
}

#[test]
fn expanded_gzip_source_budget_rejects_before_parsing() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("large.pbrt.gz");
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&vec![b' '; 1024]).unwrap();
    fs::write(&path, encoder.finish().unwrap()).unwrap();

    let error = match read_document_with_options(
        &path,
        ReadOptions {
            max_expanded_source_bytes: 100,
            ..ReadOptions::default()
        },
    ) {
        Ok(_) => panic!("expanded source over the configured budget should fail"),
        Err(error) => error,
    };
    assert!(
        error.to_string().contains("expanded source")
            && error
                .to_string()
                .contains("exceeds the 100 byte preview source budget")
    );
}

#[test]
fn matrix_camera_pose_is_initialized_without_a_look_at_directive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "Translate 0 0 5\nCamera \"perspective\" \"float fov\" [60]\nFilm \"rgb\" \"integer xresolution\" [640] \"integer yresolution\" [480]\nWorldBegin\nShape \"sphere\"\n").unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    let camera = preview.manifest.camera.unwrap();
    let [eye, target, _] = camera.look_at.unwrap();
    assert_eq!(eye, [0., 0., -5.]);
    assert!(target[2] > eye[2]);
    assert_eq!(camera.fov_degrees, Some(60.));
    assert_eq!(camera.film_resolution, Some([640, 480]));
}

#[test]
fn import_expands_with_callsite_transform_and_include_depth_is_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    let child = dir.path().join("child.pbrt");
    fs::write(&child, "Shape \"sphere\"\n").unwrap();
    fs::write(
        &path,
        "WorldBegin\nTranslate 2 0 0\nImport \"child.pbrt\"\n",
    )
    .unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 1);
    assert_eq!(
        preview.manifest.draws[0].source.as_ref().unwrap().path,
        protocol::path::canonicalize(child)
            .unwrap()
            .to_string_lossy()
    );
    assert_eq!(preview.manifest.draws[0].world_from_object[0][3], 2.0);
    fs::write(&path, "Include \"child.pbrt\"\n").unwrap();
    assert!(
        parse_scene(
            &path,
            ParseOptions {
                include_depth: 0,
                ..Default::default()
            }
        )
        .is_err()
    );
}

#[test]
fn bilinearmesh_is_tessellated_as_curved_quad_with_checked_indices() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("bilinear.pbrt");
    std::fs::write(&scene, "WorldBegin\nShape \"bilinearmesh\" \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 1] \"integer indices\" [0 1 2 3]\n").unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert!(
        preview
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "unsupported_shape")
    );
    assert_eq!(preview.manifest.draws.len(), 1);
    let mesh = &preview.manifest.meshes[0];
    assert_eq!(mesh.positions.count, 81);
    assert_eq!(mesh.indices.count, 384);
    let blob = &preview.blobs[&mesh.blob];
    let center_z = f32::from_le_bytes(
        blob[mesh.positions.offset as usize + 40 * mesh.positions.stride as usize + 8
            ..mesh.positions.offset as usize + 40 * mesh.positions.stride as usize + 12]
            .try_into()
            .unwrap(),
    );
    assert!((center_z - 0.25).abs() < 1e-6);

    std::fs::write(&scene, "WorldBegin\nShape \"bilinearmesh\" \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 1] \"integer indices\" [0 1 2 9]\n").unwrap();
    assert!(
        parse_scene(&scene, ParseOptions::default())
            .unwrap()
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "geometry_failed")
    );
}

#[test]
fn curve_and_curves_support_bezier_bspline_and_ribbon_types() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("curves.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"curve\" \"point3 P\" [0 0 0 1 1 0 2 -1 0 3 0 0] \"string type\" [\"cylinder\"] \"float width\" [0.2]\nShape \"curve\" \"point3 P\" [0 0 0 1 1 0 2 -1 0 3 0 0] \"string basis\" [\"bspline\"] \"string type\" [\"flat\"]\nShape \"curves\" \"point3 P\" [0 0 0 1 1 0 2 -1 0 3 0 0] \"point3 N\" [0 0 1 0 0 1] \"string type\" [\"ribbon\"] \"float width\" [0.2]\n",
    ).unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 3);
    assert_eq!(preview.manifest.meshes[0].positions.count, 225);
    assert_eq!(preview.manifest.meshes[0].indices.count, 24 * 8 * 6);
    assert_eq!(preview.manifest.meshes[1].positions.count, 50);
    assert_eq!(preview.manifest.meshes[2].positions.count, 50);
    assert!(
        preview
            .diagnostics
            .iter()
            .all(|diagnostic| diagnostic.code != "unsupported_shape")
    );

    std::fs::write(
        &scene,
        "WorldBegin\nShape \"curve\" \"point3 P\" [0 0 0 1 1 0 2 -1 0 3 0 0] \"string basis\" [\"bezier\"] \"integer degree\" [2]\nShape \"curve\" \"point3 P\" [0 0 0 1 1 0 2 -1 0 3 0 0] \"string type\" [\"ribbon\"]\n",
    ).unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 0);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        2
    );
}

#[test]
fn loopsubdiv_refines_closed_and_boundary_meshes_with_checked_limits() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("loopsubdiv.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"loopsubdiv\" \"point3 P\" [1 1 1 -1 -1 1 -1 1 -1 1 -1 -1] \"integer indices\" [0 2 1 0 1 3 1 2 3 2 0 3] \"integer levels\" [1]\nShape \"loopsubdiv\" \"point3 P\" [0 0 0 1 0 0 0 1 0] \"integer indices\" [0 1 2] \"integer levels\" [1]\nShape \"loopsubdiv\" \"point3 P\" [0 0 0 1 0 0 0 1 0] \"integer indices\" [0 1 2] \"integer levels\" [6]\n",
    ).unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.meshes.len(), 2);
    assert_eq!(preview.manifest.meshes[0].positions.count, 10);
    assert_eq!(preview.manifest.meshes[0].indices.count, 48);
    assert_eq!(preview.manifest.meshes[1].positions.count, 6);
    assert_eq!(preview.manifest.meshes[1].indices.count, 12);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        1
    );
}

#[test]
fn loopsubdiv_keeps_a_bowtie_vertex_in_place() {
    // Two triangles meeting only at vertex 0: it has four boundary
    // neighbours (the fixture's pedestal has six such vertices).
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("bowtie.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"loopsubdiv\" \"point3 P\" [0 0 0 1 0 0 1 1 0 -1 0 0 -1 -1 0] \"integer indices\" [0 1 2 0 3 4] \"integer levels\" [1]\n",
    )
    .unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert!(preview.diagnostics.is_empty(), "{:?}", preview.diagnostics);
    let mesh = &preview.manifest.meshes[0];
    // 5 control points + 6 edge points, 2 triangles split into 8.
    assert_eq!(mesh.positions.count, 11);
    assert_eq!(mesh.indices.count, 24);
    let blob = &preview.blobs[&mesh.blob];
    let at = mesh.positions.offset as usize;
    let first: Vec<f32> = (0..3)
        .map(|axis| f32::from_le_bytes(blob[at + 4 * axis..at + 4 * axis + 4].try_into().unwrap()))
        .collect();
    assert_eq!(first, [0., 0., 0.], "the bowtie vertex does not move");
}

#[test]
fn clipped_paraboloid_generates_open_surface_and_rejects_empty_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("paraboloid.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"paraboloid\" \"float radius\" [2] \"float zmin\" [-1] \"float zmax\" [2] \"float phimax\" [90]\nShape \"paraboloid\" \"float zmin\" [-2] \"float zmax\" [-1]\n",
    ).unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.meshes.len(), 1);
    let mesh = &preview.manifest.meshes[0];
    // 32 profile rows (the sphere's density) over 16 angular columns at
    // phimax 90; row 0 degenerates to the apex fan.
    assert_eq!(mesh.positions.count, 33 * 17);
    assert_eq!(mesh.indices.count, (32 * 16 * 2 - 16) * 3);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        1
    );
    let blob = &preview.blobs[&mesh.blob];
    let apex = mesh.positions.offset as usize + (32 * 17) * mesh.positions.stride as usize;
    let x = f32::from_le_bytes(blob[apex..apex + 4].try_into().unwrap());
    let z = f32::from_le_bytes(blob[apex + 8..apex + 12].try_into().unwrap());
    assert!((x - 2.0).abs() < 1e-6 && (z - 2.0).abs() < 1e-6);
}

#[test]
fn clipped_hyperboloid_uses_profile_points_and_rejects_degenerate_profiles() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("hyperboloid.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"hyperboloid\" \"point3 p1\" [0.5 0 0] \"point3 p2\" [1 0 2] \"float phimax\" [180]\nShape \"hyperboloid\" \"point3 p1\" [0 0 1] \"point3 p2\" [1 1 1]\n",
    )
    .unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.meshes.len(), 1);
    let mesh = &preview.manifest.meshes[0];
    // 32 profile rows over 32 angular columns at phimax 180.
    assert_eq!(mesh.positions.count, 33 * 33);
    assert_eq!(mesh.indices.count, 32 * 32 * 6);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        1
    );
    let blob = &preview.blobs[&mesh.blob];
    let upper = mesh.positions.offset as usize + 32 * 33 * mesh.positions.stride as usize;
    let x = f32::from_le_bytes(blob[upper..upper + 4].try_into().unwrap());
    let z = f32::from_le_bytes(blob[upper + 8..upper + 12].try_into().unwrap());
    assert!((x - 0.5).abs() < 1e-6 && (z - 2.0).abs() < 1e-6);
}

#[test]
fn heightfield_builds_regular_grid_and_checks_dimensions_and_heights() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("heightfield.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"heightfield\" \"integer nu\" [3] \"integer nv\" [2] \"float Pz\" [0 1 2 3 4 5]\nShape \"heightfield\" \"integer nu\" [2] \"integer nv\" [2] \"float Pz\" [0 1 2]\nShape \"heightfield\" \"integer nu\" [2] \"integer nv\" [2] \"float Pz\" [0 nan 2 3]\n",
    )
    .unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.meshes.len(), 1);
    let mesh = &preview.manifest.meshes[0];
    assert_eq!(mesh.positions.count, 6);
    assert_eq!(mesh.indices.count, 12);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        2
    );
}

#[test]
fn nurbs_samples_rational_surface_with_checked_knot_vectors() {
    let dir = tempfile::tempdir().unwrap();
    let scene = dir.path().join("nurbs.pbrt");
    std::fs::write(
        &scene,
        "WorldBegin\nShape \"nurbs\" \"integer nu\" [2] \"integer nv\" [2] \"integer uorder\" [2] \"integer vorder\" [2] \"float uknots\" [0 0 1 1] \"float vknots\" [0 0 1 1] \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 0] \"integer diceu\" [3] \"integer dicev\" [3]\nShape \"nurbs\" \"integer nu\" [2] \"integer nv\" [2] \"integer uorder\" [2] \"float uknots\" [0 1 0 1] \"float vknots\" [0 0 1 1] \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 0]\nShape \"nurbs\" \"integer nu\" [2] \"integer nv\" [2] \"integer uorder\" [2] \"integer vorder\" [2] \"float uknots\" [0 0 1 1] \"float vknots\" [0 0 1 1] \"point4 Pw\" [0 0 0 1 1 0 0 1 0 1 0 1 2 2 0 2] \"integer diceu\" [3] \"integer dicev\" [3]\n",
    )
    .unwrap();
    let preview = parse_scene(&scene, ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.meshes.len(), 2);
    let mesh = &preview.manifest.meshes[0];
    assert_eq!(mesh.positions.count, 9);
    assert_eq!(mesh.indices.count, 24);
    assert_eq!(
        preview
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "geometry_failed")
            .count(),
        1
    );
    let rational = &preview.manifest.meshes[1];
    let blob = &preview.blobs[&rational.blob];
    let center = rational.positions.offset as usize + 4 * rational.positions.stride as usize;
    let x = f32::from_le_bytes(blob[center..center + 4].try_into().unwrap());
    let y = f32::from_le_bytes(blob[center + 4..center + 8].try_into().unwrap());
    assert!((x - 0.6).abs() < 1e-6 && (y - 0.6).abs() < 1e-6);
}

/// A Z-up LookAt over a huge floor (the killeroo scenes): navigation keeps
/// the scene's up axis, not the camera's tilted one, and pivots near what
/// the camera looks at instead of a scene-sized distance past it.
#[test]
fn look_at_camera_keeps_the_world_up_and_a_near_pivot() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "LookAt 400 20 30  0 63 -110  0 0 1\nCamera \"perspective\" \"float fov\" [39]\nWorldBegin\n\
         AttributeBegin\nTranslate 0 63 -110\nShape \"sphere\" \"float radius\" [20]\nAttributeEnd\n\
         Shape \"trianglemesh\" \"point3 P\" [-4000 -4000 -140 4000 -4000 -140 4000 4000 -140 -4000 4000 -140]\n\
         \"integer indices\" [0 1 2 0 2 3]\n",
    )
    .unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    let [eye, target, up] = preview.manifest.camera.unwrap().look_at.unwrap();
    for (axis, expected) in up.iter().zip([0., 0., 1.]) {
        assert!((axis - expected).abs() < 1e-9, "up {up:?}");
    }
    let distance = (0..3)
        .map(|i| (target[i] - eye[i]).powi(2))
        .sum::<f64>()
        .sqrt();
    // LookAt's own target is ~424 away; the floor's half-diagonal is ~5660.
    assert!(distance < 1000., "pivot {distance} away");
}

/// `screenwindow` and `frameaspectratio` reach the manifest camera, so a
/// portrait view written by Render View keeps its vertical fov.
#[test]
fn camera_screenwindow_and_frame_aspect_ratio_are_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "Film \"rgb\" \"integer xresolution\" 1000 \"integer yresolution\" 1400\nLookAt 0 0 -5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 45 \"float screenwindow\" [-0.714 0.714 -1 1] \"float frameaspectratio\" 0.714\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    let camera = parse_scene(&path, ParseOptions::default())
        .unwrap()
        .manifest
        .camera
        .unwrap();
    assert_eq!(camera.screenwindow, Some([-0.714, 0.714, -1., 1.]));
    assert_eq!(camera.frame_aspect_ratio, Some(0.714));
    let plain = dir.path().join("plain.pbrt");
    fs::write(
        &plain,
        "LookAt 0 0 -5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    let camera = parse_scene(&plain, ParseOptions::default())
        .unwrap()
        .manifest
        .camera
        .unwrap();
    assert_eq!(
        (camera.screenwindow, camera.frame_aspect_ratio),
        (None, None)
    );
}
