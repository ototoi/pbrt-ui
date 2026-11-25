use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

fn scene(source: &str) -> PreviewGeometry {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pbrt-ui-scene-parse-{unique}"));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.pbrt");
    fs::write(&path, source).unwrap();
    let parsed = parse_scene(&path, ParseOptions::default()).unwrap();
    fs::remove_dir_all(dir).unwrap();
    parsed
}

#[test]
fn trianglemesh_exports_object_space_binary_and_directive_source() {
    let result = scene(
        "LookAt 0 0 -4 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" 50\nTranslate 2 0 0\nShape \"trianglemesh\" \"point P\" [ 0 0 0 1 0 0 0 1 0 ] \"integer indices\" [ 0 1 2 ]",
    );
    assert_eq!(result.manifest.meshes.len(), 1);
    assert_eq!(result.manifest.draws[0].world_from_object[0][3], 2.0);
    assert!(result.manifest.draws[0].source.as_ref().unwrap().line > 0);
    assert_eq!(
        result.manifest.camera.as_ref().unwrap().fov_degrees,
        Some(50.0)
    );
    assert_eq!(result.manifest.meshes[0].indices.count, 3);
}

#[test]
fn sphere_is_tessellated_and_unknown_shape_is_reported() {
    let result = scene("Shape \"sphere\" \"float radius\" 2\nShape \"unknown-preview-shape\"");
    assert_eq!(result.manifest.meshes.len(), 1);
    assert!(result.manifest.meshes[0].positions.count > 100);
    assert_eq!(result.diagnostics[0].code, "unsupported_shape");
}

#[test]
fn geometry_failed_names_only_the_asset_file() {
    let result = scene("Shape \"sphere\"\nShape \"plymesh\" \"string filename\" \"broken.ply\"\n");
    assert_eq!(result.manifest.meshes.len(), 1);
    let failed = result
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == "geometry_failed")
        .expect("the missing ply produces a geometry_failed warning");
    // Preview packages live in temporary staging bundles; the directory is
    // meaningless to a user and would defeat deduplication across the
    // preview that runs after every edit. The warning keeps the file
    // name only.
    assert!(failed.message.contains("broken.ply"), "{}", failed.message);
    assert!(!failed.message.contains('/'), "{}", failed.message);
}

/// Media are project data the preview cannot draw, so every declared
/// medium (by name and type) produces one `media_not_previewed` note.
/// It is info, not a warning: nothing was dropped from the project, so
/// it must not count toward the preview's `unsupported` number.
#[test]
fn media_note_is_reported_once_per_declared_medium() {
    let result = scene(
        "Camera \"perspective\"\n\
         WorldBegin\n\
         MakeNamedMedium \"fog\" \"string type\" [\"cloud\"] \"float density\" [2]\n\
         MakeNamedMedium \"fog\" \"string type\" [\"cloud\"] \"float density\" [3]\n\
         MediumInterface \"fog\" \"\"\n\
         Shape \"sphere\"\n",
    );
    let notes = result
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code == "media_not_previewed")
        .collect::<Vec<_>>();
    assert_eq!(notes.len(), 1, "{:?}", result.diagnostics);
    assert_eq!(notes[0].severity, "info");
    assert!(
        notes[0]
            .message
            .contains("participating media are not drawn in Solid/Lit (MakeNamedMedium \"fog\", type cloud); render with r4 to see them"),
        "{}",
        notes[0].message
    );
    assert!(
        notes[0].path.is_some() && notes[0].line.is_some(),
        "the note points at the declaration: {:?}",
        notes[0]
    );
    assert_eq!(
        result.warning_count(),
        0,
        "an info note about retained data is not an unsupported piece: {:?}",
        result.diagnostics
    );
}

#[test]
fn instances_and_animated_shapes_are_drawn_at_supported_times() {
    let result = scene(
        "ObjectBegin \"part\"\nShape \"sphere\"\nObjectEnd\nObjectInstance \"part\"\nActiveTransform StartTime\nTranslate 1 0 0\nShape \"sphere\"\nActiveTransform All\nIdentity\nShape \"sphere\"",
    );
    assert_eq!(result.manifest.draws.len(), 3);
    assert!(result.manifest.draws[0].instance_source.is_some());
    assert_eq!(result.manifest.supported_times.len(), 61);
    assert_eq!(result.manifest.draws[1].world_from_object.len(), 61);
    assert!(result.diagnostics.is_empty());
}

#[test]
fn reverse_orientation_is_restored_at_attribute_end() {
    let result = scene(
        "AttributeBegin\nReverseOrientation\nShape \"sphere\"\nAttributeEnd\nShape \"sphere\"",
    );
    assert_eq!(result.manifest.draws.len(), 2);
    assert!(result.manifest.draws[0].reverse_orientation);
    assert!(!result.manifest.draws[1].reverse_orientation);
}

#[test]
fn geometry_cache_reuses_unchanged_shapes_and_notices_edits() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pbrt-ui-geometry-cache-{unique}"));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.pbrt");
    let parse = |source: &str, cache: &mut GeometryCache| {
        fs::write(&path, source).unwrap();
        parse_scene_with_cache(&path, ParseOptions::default(), cache).unwrap()
    };
    let mut cache = GeometryCache::default();
    let first = parse("WorldBegin\nShape \"sphere\"\nShape \"disk\"\n", &mut cache);
    assert_eq!(cache.stats().entries, 2);
    assert_eq!(cache.stats().misses, 2);
    assert_eq!(cache.stats().hits, 0);
    // Transform edits only move placements; the tessellation is reused.
    let moved = parse(
        "WorldBegin\nTranslate 10 0 0\nShape \"sphere\"\nShape \"disk\"\n",
        &mut cache,
    );
    assert_eq!(cache.stats().hits, 2);
    assert_eq!(cache.stats().misses, 2);
    assert_eq!(moved.manifest.meshes.len(), 2);
    assert_eq!(moved.manifest.draws[0].world_from_object[0][3], 10.0);
    assert_eq!(
        moved.manifest.blobs["geometry/00000000.bin"].sha256,
        first.manifest.blobs["geometry/00000000.bin"].sha256,
    );
    // Editing one parameter only invalidates that shape.
    let edited = parse(
        "WorldBegin\nTranslate 10 0 0\nShape \"sphere\" \"float radius\" [2]\nShape \"disk\"\n",
        &mut cache,
    );
    assert_eq!(cache.stats().hits, 3);
    assert_eq!(cache.stats().misses, 3);
    assert_eq!(cache.stats().entries, 3);
    assert_ne!(
        edited.manifest.blobs["geometry/00000000.bin"].sha256,
        first.manifest.blobs["geometry/00000000.bin"].sha256,
    );
    assert_eq!(
        edited.manifest.blobs["geometry/00000001.bin"].sha256,
        first.manifest.blobs["geometry/00000001.bin"].sha256,
    );
    fs::remove_dir_all(dir).unwrap();
}

#[test]
fn geometry_cache_reuses_curve_uv_payload() {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("pbrt-ui-curve-uv-cache-{unique}"));
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("scene.pbrt");
    let source = "WorldBegin\nShape \"curve\" \"point3 P\" [0 0 0 0.2 0.5 0 0.8 0.5 0 1 0 0 1.2 -0.5 0 1.8 -0.5 0 2 0 0] \"string type\" [\"cylinder\"]\n";
    let mut cache = GeometryCache::default();
    fs::write(&path, source).unwrap();
    let first = parse_scene_with_cache(&path, ParseOptions::default(), &mut cache).unwrap();
    let mesh = &first.manifest.meshes[0];
    let first_uvs = read_f32x2_array(
        &first.blobs[&mesh.blob],
        mesh.uv.as_ref().expect("curve UV span"),
    )
    .unwrap();
    assert_eq!(first_uvs.len(), mesh.positions.count as usize);
    assert_eq!(cache.stats().misses, 1);
    fs::write(&path, source).unwrap();
    let second = parse_scene_with_cache(&path, ParseOptions::default(), &mut cache).unwrap();
    let second_mesh = &second.manifest.meshes[0];
    let second_uvs = read_f32x2_array(
        &second.blobs[&second_mesh.blob],
        second_mesh.uv.as_ref().expect("cached curve UV span"),
    )
    .unwrap();
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(first_uvs, second_uvs);
    assert_eq!(
        second.manifest.blobs[&second_mesh.blob].sha256,
        first.manifest.blobs[&mesh.blob].sha256
    );
    fs::remove_dir_all(dir).unwrap();
}

/// The remaining analytic shapes carry pbrt's UVs end to end:
/// the manifest's mesh records hold UV spans that match the vertex count.
/// loopsubdiv and a "uv"-less trianglemesh stay without (out of scope).
#[test]
fn analytic_shapes_carry_uvs_and_uvless_meshes_do_not() {
    let source = "WorldBegin\n\
Shape \"paraboloid\"\n\
Shape \"hyperboloid\" \"point3 p1\" [0 0 0] \"point3 p2\" [1 1 1]\n\
Shape \"bilinearmesh\" \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 0] \"integer indices\" [0 1 2 3]\n\
Shape \"heightfield\" \"integer nu\" [2] \"integer nv\" [2] \"float Pz\" [0 1 1 0]\n\
Shape \"nurbs\" \"integer nu\" [2] \"integer nv\" [2] \"integer uorder\" [2] \"integer vorder\" [2] \
\"float uknots\" [0 0 1 1] \"float vknots\" [0 0 1 1] \"point3 P\" [0 0 0 1 0 0 0 1 0 1 1 0]\n\
Shape \"loopsubdiv\" \"integer nlevels\" [2] \"point3 P\" [0 0 0 1 0 0 0 1 0] \"integer indices\" [0 1 2]\n\
Shape \"trianglemesh\" \"point3 P\" [0 0 0 1 0 0 0 1 0] \"integer indices\" [0 1 2]\n";
    let result = scene(source);
    assert_eq!(result.manifest.meshes.len(), 7, "{:?}", result.diagnostics);
    for (index, mesh) in result.manifest.meshes.iter().enumerate().take(5) {
        let uv = mesh
            .uv
            .as_ref()
            .unwrap_or_else(|| panic!("shape {index} has no uv span"));
        assert_eq!(uv.count, mesh.positions.count);
        let uvs = read_f32x2_array(&result.blobs[&mesh.blob], uv).unwrap();
        assert_eq!(uvs.len(), mesh.positions.count as usize);
    }
    for (index, mesh) in result.manifest.meshes.iter().enumerate().skip(5) {
        assert!(mesh.uv.is_none(), "shape {index} must stay uv-less");
    }
    assert!(result.diagnostics.is_empty(), "{:?}", result.diagnostics);
}

/// Identical `texcoord2 uv` value tokens hash to the same cache key,
/// so an unchanged uv parameter keeps the tessellation alive across parses.
#[test]
fn texcoord_uv_value_tokens_share_the_cache_key() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    let parse = |source: &str, cache: &mut GeometryCache| {
        fs::write(&path, source).unwrap();
        parse_scene_with_cache(&path, ParseOptions::default(), cache).unwrap()
    };
    let mut cache = GeometryCache::default();
    let uv = "\"float radius\" [1] \"texcoord2 uv\" [ 0 0 1 0 0 1 ]";
    parse(&format!("WorldBegin\nShape \"sphere\" {uv}\n"), &mut cache);
    assert_eq!(cache.stats().misses, 1);
    parse(&format!("WorldBegin\nShape \"sphere\" {uv}\n"), &mut cache);
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(cache.stats().misses, 1);
}

/// The model's asserted sha256 keys the tessellation
/// cache without opening the PLY, so a cached reparse reads it zero
/// times; only the miss that needs tessellation bytes opens the file,
/// and the read bytes must hash back to the model's claim.
#[test]
fn cached_plymesh_reparses_without_reading_the_ply() {
    let dir = tempfile::tempdir().unwrap();
    let ply = concat!(
        "ply\nformat ascii 1.0\nelement vertex 3\n",
        "property float x\nproperty float y\nproperty float z\n",
        "property float nx\nproperty float ny\nproperty float nz\n",
        "element face 1\nproperty list uchar int vertex_indices\nend_header\n",
        "0 0 0 0 0 1\n1 0 0 0 0 1\n0 1 0 0 0 1\n3 0 1 2\n",
    );
    fs::write(dir.path().join("tri.ply"), ply).unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "WorldBegin\nShape \"plymesh\" \"string filename\" \"tri.ply\"\n",
    )
    .unwrap();
    let options = ParseOptions {
        asset_sha256: [(
            "tri.ply".to_string(),
            format!("{:x}", sha2::Sha256::digest(ply.as_bytes())),
        )]
        .into_iter()
        .collect(),
        ..ParseOptions::default()
    };
    let mut cache = GeometryCache::default();
    let first = parse_scene_with_cache(&path, options.clone(), &mut cache).unwrap();
    assert_eq!(cache.stats().misses, 1);
    assert_eq!(
        cache.stats().asset_reads,
        1,
        "the first parse reads the PLY"
    );
    let second = parse_scene_with_cache(&path, options, &mut cache).unwrap();
    assert_eq!(cache.stats().hits, 1);
    assert_eq!(
        cache.stats().asset_reads,
        1,
        "the cached reparse opens no PLY"
    );
    assert_eq!(
        second.manifest.blobs["geometry/00000000.bin"].sha256,
        first.manifest.blobs["geometry/00000000.bin"].sha256,
    );
}

/// The model's sha256 is only trusted for the cache key;
/// the miss that actually reads the PLY verifies it and fails the way a
/// changed asset fails during export.
#[test]
fn changed_ply_fails_the_parse_with_the_export_asset_error() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("tri.ply"),
        "ply bytes that differ from the model",
    )
    .unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "WorldBegin\nShape \"plymesh\" \"string filename\" \"tri.ply\"\n",
    )
    .unwrap();
    let options = ParseOptions {
        asset_sha256: [(
            "tri.ply".to_string(),
            format!("{:x}", sha2::Sha256::digest(b"not the file on disk")),
        )]
        .into_iter()
        .collect(),
        ..ParseOptions::default()
    };
    let error = parse_scene_with_cache(&path, options, &mut GeometryCache::default()).unwrap_err();
    assert_eq!(error.to_string(), "asset changed during export");
}

/// The blob sha the cache stores once at put time is
/// exactly the digest the manifest would recompute from the cached copy.
#[test]
fn cached_blob_sha256_matches_a_recomputation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, "WorldBegin\nShape \"sphere\"\nShape \"curve\" \"point3 P\" [0 0 0 0.2 0.5 0 0.8 0.5 0 1 0 0 1.2 -0.5 0 1.8 -0.5 0 2 0 0] \"string type\" [\"cylinder\"]\n").unwrap();
    let mut cache = GeometryCache::default();
    parse_scene_with_cache(&path, ParseOptions::default(), &mut cache).unwrap();
    let second = parse_scene_with_cache(&path, ParseOptions::default(), &mut cache).unwrap();
    assert!(
        cache.stats().hits > 0,
        "the second parse must hit the cache"
    );
    for (name, bytes) in &second.blobs {
        assert_eq!(
            second.manifest.blobs[name].sha256,
            format!("{:x}", sha2::Sha256::digest(bytes)),
            "{name} sha must match a recomputation"
        );
    }
}

/// An imagemap or image-light source that is a project
/// asset takes the model's sha256 for its blob record instead of hashing
/// the payload; the bytes still ride along with the package. The fixture's model
/// sha deliberately differs from the file's bytes to pin where the
/// record's sha comes from; a consistent model hashes equal either way.
#[test]
fn imagemap_and_light_blob_shas_come_from_the_model_sha() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("env.dat"), b"map bytes").unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(
        &path,
        "WorldBegin\n\
Texture \"env\" \"spectrum\" \"imagemap\" \"string filename\" \"env.dat\"\n\
LightSource \"infinite\" \"string mapname\" \"env.dat\"\n\
Shape \"sphere\"\n",
    )
    .unwrap();
    let model_sha = format!("{:x}", sha2::Sha256::digest(b"the model's immutable bytes"));
    let options = ParseOptions {
        asset_sha256: [("env.dat".to_string(), model_sha.clone())]
            .into_iter()
            .collect(),
        ..ParseOptions::default()
    };
    let scene = parse_scene(&path, options).unwrap();
    assert_eq!(scene.blobs["texture/spectrum/env"], b"map bytes");
    assert_eq!(scene.blobs["light/0"], b"map bytes");
    assert_eq!(
        scene.manifest.blobs["texture/spectrum/env"].sha256,
        model_sha
    );
    assert_eq!(scene.manifest.blobs["light/0"].sha256, model_sha);
}

/// A map rides along as `light/{index}`, which the renderer finds by the light's
/// place in the manifest. A missing one warns and leaves the scene readable.
#[test]
fn infinite_light_maps_ride_along_as_blobs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("env.exr"), b"map bytes").unwrap();
    std::fs::write(dir.path().join("old.exr"), b"old bytes").unwrap();
    let path = dir.path().join("scene.pbrt");
    std::fs::write(
        &path,
        "WorldBegin\nLightSource \"point\"\nLightSource \"infinite\" \"string filename\" \"env.exr\"\nLightSource \"infinite\" \"string mapname\" \"old.exr\"\nLightSource \"infinite\" \"string filename\" \"gone.exr\"\nLightSource \"infinite\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let scene = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(scene.blobs["light/1"], b"map bytes");
    assert_eq!(scene.blobs["light/2"], b"old bytes");
    assert!(!scene.blobs.contains_key("light/3"));
    assert!(!scene.blobs.contains_key("light/4"));
    assert!(scene.manifest.blobs.contains_key("light/1"));
    assert!(
        scene
            .diagnostics
            .iter()
            .any(|d| d.code == "light_image_failed" && d.message.contains("gone.exr")),
        "{:?}",
        scene.diagnostics
    );
}

/// Goniometric and projection lights carry their images the same way: the
/// `filename` (else the legacy `mapname`) lands as `light/{index}`, named by
/// its kind when the file cannot be read. A projection light without either
/// parses and simply has no blob to carry.
#[test]
fn goniometric_and_projection_maps_ride_along_as_blobs() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("ies.png"), b"gonio bytes").unwrap();
    std::fs::write(dir.path().join("slide.png"), b"slide bytes").unwrap();
    let path = dir.path().join("scene.pbrt");
    std::fs::write(
        &path,
        "WorldBegin\n\
LightSource \"goniometric\" \"string filename\" \"ies.png\" \"rgb I\" [2 2 2]\n\
LightSource \"goniometric\" \"string mapname\" \"ies.png\"\n\
LightSource \"goniometric\" \"string filename\" \"missing.png\"\n\
LightSource \"goniometric\"\n\
LightSource \"projection\" \"string filename\" \"slide.png\" \"float fov\" [60]\n\
LightSource \"projection\" \"string filename\" \"lost.png\"\n\
Shape \"sphere\"\n",
    )
    .unwrap();
    let scene = parse_scene(&path, ParseOptions::default()).unwrap();
    assert_eq!(scene.blobs["light/0"], b"gonio bytes");
    assert_eq!(scene.blobs["light/1"], b"gonio bytes", "mapname fallback");
    assert!(
        !scene.blobs.contains_key("light/2") && !scene.blobs.contains_key("light/3"),
        "unreadable and absent maps carry nothing"
    );
    assert_eq!(scene.blobs["light/4"], b"slide bytes");
    assert!(!scene.blobs.contains_key("light/5"));
    for (kind, file) in [("goniometric", "missing.png"), ("projection", "lost.png")] {
        assert!(
            scene
                .diagnostics
                .iter()
                .any(|d| d.code == "light_image_failed"
                    && d.message.contains(kind)
                    && d.message.contains(file)),
            "{kind} warning must name kind and file: {:?}",
            scene.diagnostics
        );
    }
}
