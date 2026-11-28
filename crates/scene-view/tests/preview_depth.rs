use protocol::geometry::*;
use scene_view::{
    PreviewMode, PreviewRenderer, ViewerCamera, render_preview, render_preview_at_time_with_options,
};
use std::collections::BTreeMap;
static GPU: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn parse(source: &str) -> scene_parse::PreviewGeometry {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap()
}

fn camera() -> ViewerCamera {
    ViewerCamera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    }
}

const TRIANGLE: &str =
    "Shape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 0 1 0] \"integer indices\" [0 1 2]\n";

#[test]
#[ignore = "requires a GPU adapter"]
fn foreground_geometry_wins_depth_and_pick_independently_of_draw_order() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut blob = blob_header().to_vec();
    let positions = ArrayRef {
        offset: append_aligned_array(
            &mut blob,
            &positions_bytes(&[[-1., -1., 0.], [1., -1., 0.], [0., 1., 0.]]),
        ),
        count: 3,
        stride: POSITION_STRIDE,
    };
    let indices = ArrayRef {
        offset: append_aligned_array(&mut blob, &indices_bytes(&[0, 1, 2])),
        count: 3,
        stride: INDEX_STRIDE,
    };
    let mut manifest = GeometryManifest::new();
    manifest.supported_times = vec![0.];
    manifest.meshes.push(MeshRecord {
        mesh_id: 0,
        blob: "triangle.bin".into(),
        positions,
        indices,
        normals: None,
        uv: None,
        origin_f64: [0.; 3],
        bounds: Bounds {
            min: [-1., -1., 0.],
            max: [1., 1., 0.],
        },
    });
    for (id, z) in [(1, 0.), (2, -2.)] {
        manifest.draws.push(DrawRecord {
            mesh_id: 0,
            shape_index: id as usize,
            source: None,
            shape_type: "trianglemesh".into(),
            pick_id: id,
            entity_id: None,
            instance_use_id: None,
            instance_source: None,
            bounds: None,
            world_from_object: vec![[
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., z, 0., 0., 0., 1.,
            ]],
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        });
    }
    let camera = ViewerCamera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let blobs = BTreeMap::from([("triangle.bin".into(), blob)]);
    for _ in 0..2 {
        let frame = render_preview(&manifest, &blobs, camera, 64, 64, PreviewMode::Solid).unwrap();
        assert_eq!(
            frame.pick_at([0.5, 0.5]).unwrap(),
            1,
            "rear surface must not hide foreground"
        );
        manifest.draws.reverse();
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn explicit_interface_material_is_invisible_but_keeps_its_pick_id() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let invisible = parse(
        "WorldBegin\nMaterial \"interface\"\nShape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 0 2 0] \"integer indices\" [0 1 2]\n",
    );
    let empty = parse("WorldBegin\n");
    for mode in [PreviewMode::Solid, PreviewMode::Lit] {
        let frame = render_preview(
            &invisible.manifest,
            &invisible.blobs,
            camera(),
            64,
            64,
            mode,
        )
        .unwrap();
        let background =
            render_preview(&empty.manifest, &empty.blobs, camera(), 64, 64, mode).unwrap();
        assert_eq!(
            frame.rgba, background.rgba,
            "{mode:?} interface is invisible"
        );
        assert_eq!(
            frame.pick_at([0.5, 0.5]).unwrap(),
            1,
            "{mode:?} stays pickable"
        );
        assert_eq!(background.pick_at([0.5, 0.5]).unwrap(), 0);
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn nearer_of_overlapping_interface_surfaces_keeps_the_pick_id() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    // The near surface comes first in scene order, so the far one draws later.
    let scene = parse(
        "WorldBegin\nMaterial \"interface\"\nShape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 0 2 0] \"integer indices\" [0 1 2]\nShape \"trianglemesh\" \"point3 P\" [-2 -2 -2 2 -2 -2 0 2 -2] \"integer indices\" [0 1 2]\n",
    );
    for mode in [PreviewMode::Solid, PreviewMode::Lit] {
        let frame = render_preview(&scene.manifest, &scene.blobs, camera(), 64, 64, mode).unwrap();
        assert_eq!(frame.pick_at([0.5, 0.5]).unwrap(), 1, "{mode:?}");
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn camera_inside_background_sphere_keeps_foreground_visible() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("inside.pbrt");
    std::fs::write(&path,"WorldBegin\nShape \"sphere\" \"float radius\" [100]\nShape \"trianglemesh\" \"point3 P\" [-5 -5 0 5 -5 0 5 5 0 -5 5 0] \"integer indices\" [0 1 2 0 2 3]\n").unwrap();
    let preview = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = ViewerCamera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.001,
        far: 400.,
        mirror: false,
    };
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera,
        64,
        64,
        PreviewMode::Solid,
    )
    .unwrap();
    assert!(
        frame.pick_ids.iter().all(|id| *id == 2),
        "an enclosing sphere must not invent occluders near the camera"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn offscreen_triangle_crossing_camera_plane_does_not_invent_a_visible_surface() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("behind.pbrt");
    std::fs::write(&path,"WorldBegin\nShape \"trianglemesh\" \"point3 P\" [-1 -5 2 1 -5 2 0 -5 -1] \"integer indices\" [0 1 2]\n").unwrap();
    let preview = scene_parse::parse_scene(path, scene_parse::ParseOptions::default()).unwrap();
    let camera = ViewerCamera {
        eye: [0.; 3],
        target: [0., 0., 1.],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera,
        64,
        64,
        PreviewMode::Solid,
    )
    .unwrap();
    assert!(
        frame.pick_ids.iter().all(|id| *id == 0),
        "an entirely offscreen triangle must not cover the viewport"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn mesh_buffers_are_shared_and_survive_camera_mode_and_placement_changes() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(&format!(
        "WorldBegin\nObjectBegin \"triangle\"\n{TRIANGLE}ObjectEnd\nObjectInstance \"triangle\"\nTranslate 3 0 0\nObjectInstance \"triangle\"\n"
    ));
    assert_eq!(preview.manifest.meshes.len(), 1);
    assert_eq!(preview.manifest.draws.len(), 2);
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    let id = preview.manifest.draws[0].pick_id;
    let first = renderer
        .render(camera(), 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(first.pick_at([0.5, 0.5]).unwrap(), id);
    let normal = renderer
        .render(camera(), 64, 64, PreviewMode::Normal, 0.)
        .unwrap();
    assert_eq!(first.pick_ids, normal.pick_ids);
    assert_ne!(first.rgba, normal.rgba);
    let wire = renderer
        .render(camera(), 64, 64, PreviewMode::Wireframe, 0.)
        .unwrap();
    assert!(wire.pick_ids.contains(&id));

    let mut moved_camera = camera();
    moved_camera.eye[0] += 5.;
    moved_camera.target[0] += 5.;
    let before = renderer
        .render(moved_camera, 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(before.pick_at([0.5, 0.5]).unwrap(), 0);
    let mut draws = preview.manifest.draws.clone();
    for draw in &mut draws {
        draw.world_from_object[0][3] += 5.;
    }
    renderer.update_draws(&draws).unwrap();
    let after = renderer
        .render(moved_camera, 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(after.pick_at([0.5, 0.5]).unwrap(), id);

    let mut invalid = draws;
    invalid[0].mesh_id = u64::MAX;
    assert!(renderer.update_draws(&invalid).is_err());
    let retained = renderer
        .render(moved_camera, 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(retained.pick_ids, after.pick_ids);
    let stats = renderer.stats();
    assert_eq!(
        stats.mesh_uploads, 1,
        "instances and view changes must share the same mesh buffers"
    );
    assert_eq!(stats.target_allocations, 1);
    assert_eq!(stats.instance_allocations, 1);
    let resized = renderer
        .render(moved_camera, 93, 71, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(resized.rgba.len(), 93 * 71 * 4);
    assert_eq!(resized.pick_at([0.5, 0.5]).unwrap(), id);
    assert_eq!(renderer.stats().target_allocations, 2);
    assert_eq!(renderer.stats().mesh_upload_bytes, stats.mesh_upload_bytes);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn advancing_a_mesh_edition_rebuilds_only_that_mesh_and_failed_updates_keep_the_cache() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut preview = parse(&format!(
        "WorldBegin\n{TRIANGLE}Translate 3 0 0\n{TRIANGLE}"
    ));
    assert_eq!(preview.manifest.meshes.len(), 2);
    let record = &preview.manifest.meshes[0];
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    let first = renderer
        .render(camera(), 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    let id = preview.manifest.draws[0].pick_id;
    assert_eq!(first.pick_at([0.5, 0.5]).unwrap(), id);

    // A cache hit does not even require the source blob to remain in memory.
    renderer
        .update_scene(&preview.manifest, &BTreeMap::new(), &BTreeMap::new())
        .unwrap();
    let editions = BTreeMap::from([(record.mesh_id, 1)]);
    assert!(
        renderer
            .update_scene(&preview.manifest, &BTreeMap::new(), &editions)
            .is_err()
    );
    assert_eq!(renderer.stats().mesh_uploads, 2);
    let retained = renderer
        .render(camera(), 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(retained.pick_ids, first.pick_ids);

    let blob = preview.blobs.get_mut(&record.blob).unwrap();
    let positions = read_f32x3_array(blob, &record.positions)
        .unwrap()
        .into_iter()
        .map(|[x, y, z]| [x + 5., y, z])
        .collect::<Vec<_>>();
    let bytes = positions_bytes(&positions);
    let offset = record.positions.offset as usize;
    blob[offset..offset + bytes.len()].copy_from_slice(&bytes);
    renderer
        .update_scene(&preview.manifest, &preview.blobs, &editions)
        .unwrap();
    let changed = renderer
        .render(camera(), 64, 64, PreviewMode::Solid, 0.)
        .unwrap();
    assert_eq!(changed.pick_at([0.5, 0.5]).unwrap(), 0);
    let mut moved = camera();
    moved.eye[0] += 5.;
    moved.target[0] += 5.;
    assert_eq!(
        renderer
            .render(moved, 64, 64, PreviewMode::Solid, 0.)
            .unwrap()
            .pick_at([0.5, 0.5])
            .unwrap(),
        id
    );
    assert_eq!(
        renderer.stats().mesh_uploads,
        3,
        "only the mesh whose edition changed is uploaded"
    );

    renderer
        .update_scene(&preview.manifest, &BTreeMap::new(), &editions)
        .unwrap();
    assert_eq!(renderer.stats().mesh_uploads, 3);
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    assert_eq!(
        renderer.stats().mesh_uploads,
        5,
        "a new scene has an independent mesh-id namespace"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn gpu_transforms_keep_large_coordinates_and_animation_samples_precise() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(&format!(
        "WorldBegin\nActiveTransform EndTime\nTranslate 5 0 0\nActiveTransform All\n{TRIANGLE}"
    ));
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    let id = preview.manifest.draws[0].pick_id;
    let start = preview.manifest.supported_times[0];
    let end = *preview.manifest.supported_times.last().unwrap();
    assert_eq!(
        renderer
            .render(camera(), 64, 64, PreviewMode::Solid, start)
            .unwrap()
            .pick_at([0.5, 0.5])
            .unwrap(),
        id
    );
    assert_eq!(
        renderer
            .render(camera(), 64, 64, PreviewMode::Solid, end)
            .unwrap()
            .pick_at([0.5, 0.5])
            .unwrap(),
        0
    );
    let mut far_camera = camera();
    for axis in 0..3 {
        far_camera.eye[axis] += 1e12;
        far_camera.target[axis] += 1e12;
    }
    let mut draws = preview.manifest.draws.clone();
    for matrix in &mut draws[0].world_from_object {
        for axis in 0..3 {
            matrix[axis * 4 + 3] += 1e12;
        }
    }
    renderer.update_draws(&draws).unwrap();
    let distant = renderer
        .render(far_camera, 64, 64, PreviewMode::Solid, start)
        .unwrap();
    let near = renderer
        .render(camera(), 64, 64, PreviewMode::Solid, end)
        .unwrap();
    assert_eq!(distant.pick_at([0.5, 0.5]).unwrap(), id);
    assert_eq!(near.pick_at([0.5, 0.5]).unwrap(), 0);
    assert_eq!(renderer.stats().mesh_uploads, 1);
    assert!(
        renderer
            .render(far_camera, 64, 64, PreviewMode::Solid, end + 1.)
            .is_err()
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn gpu_normal_transform_handles_nonuniform_negative_scale_and_reverse_orientation() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(
        "WorldBegin\nScale 1 2 0.5\nShape \"trianglemesh\" \"point3 P\" [-1 -1 -1 1 -1 -1 0 1 1] \"integer indices\" [0 1 2]\n",
    );
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    let pixel = |frame: &scene_view::PreviewFrame| {
        frame.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 4].to_vec()
    };
    let normal = renderer
        .render(camera(), 64, 64, PreviewMode::Normal, 0.)
        .unwrap();
    let rgb = pixel(&normal);
    assert!(
        (160..175).contains(&rgb[1]) && rgb[2] > 245,
        "inverse-transpose normal expected, got {rgb:?}"
    );
    let mut draws = preview.manifest.draws.clone();
    for component in &mut draws[0].world_from_object[0][0..4] {
        *component = -*component;
    }
    renderer.update_draws(&draws).unwrap();
    let flipped = renderer
        .render(camera(), 64, 64, PreviewMode::Normal, 0.)
        .unwrap();
    assert!(pixel(&flipped)[2] < 45);
    draws[0].reverse_orientation = true;
    renderer.update_draws(&draws).unwrap();
    let reversed = renderer
        .render(camera(), 64, 64, PreviewMode::Normal, 0.)
        .unwrap();
    assert_eq!(pixel(&reversed), rgb);
    assert_eq!(renderer.stats().mesh_uploads, 1);
}

#[test]
#[ignore = "requires a GPU adapter; reports cached rendering timings"]
fn cached_viewport_measures_camera_updates_for_one_hundred_thousand_triangles() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let mut points = Vec::new();
    let mut indices = Vec::new();
    for y in 0..=25 {
        for x in 0..=20 {
            points.extend([x as f64 / 10. - 1., y as f64 / 12.5 - 1., 0.]);
        }
    }
    for y in 0..25 {
        for x in 0..20 {
            let a = y * 21 + x;
            indices.extend([a, a + 1, a + 21, a + 1, a + 22, a + 21]);
        }
    }
    let mut source = format!(
        "WorldBegin\nObjectBegin \"grid\"\nShape \"trianglemesh\" \"point3 P\" [{}] \"integer indices\" [{}]\nObjectEnd\n",
        points
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" "),
        indices
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" ")
    );
    for y in 0..10 {
        for x in 0..10 {
            source.push_str(&format!(
                "AttributeBegin\nTranslate {} {} 0\nObjectInstance \"grid\"\nAttributeEnd\n",
                x * 3,
                y * 3
            ));
        }
    }
    let preview = parse(&source);
    assert_eq!(preview.manifest.draws.len(), 100);
    assert_eq!(preview.manifest.meshes.len(), 1);
    let mut camera = ViewerCamera::frame(preview.manifest.bounds.as_ref(), 1280. / 720.);
    let mut renderer = PreviewRenderer::new().unwrap();
    renderer
        .upload_scene(&preview.manifest, &preview.blobs)
        .unwrap();
    let mut times = Vec::new();
    for frame in 0..65 {
        camera.orbit(0.005, 0.001).unwrap();
        let start = std::time::Instant::now();
        let image = renderer
            .render(camera, 1280, 720, PreviewMode::Solid, 0.)
            .unwrap();
        assert!(image.pick_ids.iter().any(|id| *id != 0));
        if frame >= 5 {
            times.push(start.elapsed().as_secs_f64() * 1000.);
        }
    }
    let mean = times.iter().sum::<f64>() / times.len() as f64;
    times.sort_by(f64::total_cmp);
    println!("adapter: {}", renderer.adapter_description());
    println!(
        "cached viewport: 1280x720, 100000 triangles, 100 placements, 60 measured frames; mean={mean:.2}ms, p95={:.2}ms, mean_fps={:.1}; GPU render + CPU readback, excludes GPUI upload",
        times[times.len() * 95 / 100],
        1000. / mean
    );
    assert_eq!(renderer.stats().mesh_uploads, 1);
    assert_eq!(renderer.stats().target_allocations, 1);
    assert_eq!(renderer.stats().instance_allocations, 1);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_mode_follows_light_intensity_and_area_emission_monotonically() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let sphere = "LightSource \"point\" \"point from\" [0 0 5] \"rgb I\" [INT INT INT]\nWorldBegin\nMakeNamedMaterial \"m\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nNamedMaterial \"m\"\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]\nAttributeBegin\nMaterial \"matte\" \"rgb reflectance\" [0.2 0.2 0.2]\nAttributeBegin\nTranslate 2 0 0\nAreaLightSource \"diffuse\" \"rgb emission\" [4 4 4]\nShape \"sphere\" \"float radius\" [0.25]\nAttributeEnd\nAttributeEnd\n";
    let dim = parse(&sphere.replace("INT", "1"));
    let bright = parse(&sphere.replace("INT", "6"));
    assert_eq!(dim.manifest.lights.len(), 1);
    assert!(
        dim.manifest
            .draws
            .iter()
            .any(|draw| draw.area_light.is_some())
    );
    let camera = ViewerCamera {
        eye: [0., 0., 3.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let dim_frame = render_preview(
        &dim.manifest,
        &dim.blobs,
        camera,
        128,
        128,
        PreviewMode::Lit,
    )
    .unwrap();
    let bright_frame = render_preview(
        &bright.manifest,
        &bright.blobs,
        camera,
        128,
        128,
        PreviewMode::Lit,
    )
    .unwrap();
    let luminance = |frame: &scene_view::PreviewFrame| -> f32 {
        // Average over covered pixels only, ignoring the background.
        let mut sum = 0.;
        let mut count = 0;
        for pixel in frame.rgba.as_chunks::<4>().0 {
            if pixel[3] == 255 {
                sum += pixel[0] as f32 + pixel[1] as f32 + pixel[2] as f32;
                count += 1;
            }
        }
        sum / (3. * count.max(1) as f32)
    };
    let dim_luma = luminance(&dim_frame);
    let bright_luma = luminance(&bright_frame);
    assert!(
        bright_luma > dim_luma,
        "stronger lights must brighten the lit preview: {dim_luma} vs {bright_luma}"
    );
    // The emissive sphere outshines the Lambert one even at dim intensity.
    assert!(dim_luma > 0., "lit mode paints lit geometry: {}", dim_luma);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_mode_samples_imagemap_textures_per_pixel() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    // A textured plane: left half red, right half green, distant light from
    // the camera so both halves are equally lit; per-pixel sampling must show
    // both colors, not a per-vertex smear.
    let mut image = image::RgbImage::new(2, 1);
    image.put_pixel(0, 0, image::Rgb([255, 0, 0]));
    image.put_pixel(1, 0, image::Rgb([0, 255, 0]));
    image.save(dir.path().join("strip.png")).unwrap();
    let path = dir.path().join("scene.pbrt");
    std::fs::write(
        &path,
        "WorldBegin\nTexture \"strip\" \"spectrum\" \"imagemap\" \"string filename\" \"strip.png\"\nMakeNamedMaterial \"m\" \"string type\" \"diffuse\" \"texture reflectance\" [strip]\nNamedMaterial \"m\"\nLightSource \"distant\" \"point from\" [0 0 5] \"point to\" [0 0 0] \"rgb I\" [2 2 2]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]\n",
    )
    .unwrap();
    let preview = scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap();
    assert!(preview.diagnostics.is_empty());
    assert!(preview.blobs.contains_key("texture/spectrum/strip"));
    let camera = ViewerCamera {
        eye: [0., 0., 5.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 45.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera,
        128,
        128,
        PreviewMode::Lit,
    )
    .unwrap();
    // Sample symmetric interior points on the left and right halves.
    let (left, right) = (
        &frame.rgba[128 * 4 * 64 + 48 * 4..128 * 4 * 64 + 48 * 4 + 4],
        &frame.rgba[128 * 4 * 64 + 80 * 4..128 * 4 * 64 + 80 * 4 + 4],
    );
    assert!(
        left[1] > left[0] + 40,
        "pbrt's horizontal camera convention places the green UV half on screen left: {left:?}"
    );
    assert!(
        right[0] > right[1] + 40,
        "pbrt's horizontal camera convention places the red UV half on screen right: {right:?}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_texture_array_samples_the_ninth_image_layer() {
    check_image_layers(10, false);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_shared_image_aliases_do_not_consume_the_fallback_layer_budget() {
    check_image_layers(14, true);
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_fallback_layer_overflow_keeps_the_scaled_image_average() {
    check_image_layers(
        25, // one layer beyond the 256 MiB array budget
        false,
    );
}

fn check_image_layers(count: u8, share_body: bool) {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    // The array-path fixtures' overflow limit: beyond the 24 layers the
    // 256 MiB budget holds, the array folds images to their scaled
    // averages.
    let overflow = count == 25 && !share_body;
    let mut source = String::from("WorldBegin\n");
    for index in 0..count {
        let pixels = if share_body || index == count - 1 {
            vec![image::Rgb([255, 0, 0]), image::Rgb([0, 0, 255])]
        } else {
            vec![image::Rgb([20 + index, 40, 60])]
        };
        let image = image::RgbImage::from_fn(pixels.len() as u32, 1, |x, _| pixels[x as usize]);
        image
            .save(dir.path().join(format!("texture{index}.png")))
            .unwrap();
        source.push_str(&format!(
            "Texture \"t{index:02}\" \"spectrum\" \"imagemap\" \"string filename\" \"texture{index}.png\" \"string encoding\" \"linear\"\n"
        ));
        if overflow && index == count - 1 {
            // Scale belongs to the texture instruction even when its image
            // is replaced by its linear average.
            let line_end = source.len() - 1;
            source.insert_str(line_end, " \"float scale\" 0.5");
        }
        source.push_str(&format!(
            "MakeNamedMaterial \"m{index}\" \"string type\" \"diffuse\" \"texture reflectance\" [t{index:02}]\n"
        ));
        source.push_str(&format!(
            "NamedMaterial \"m{index}\"\nShape \"trianglemesh\" \"point3 P\" [-1 -1 {} 1 -1 {} 1 1 {} -1 1 {}] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]\n",
            index as f32 * 0.01,
            index as f32 * 0.01,
            index as f32 * 0.01,
            index as f32 * 0.01,
        ));
    }
    let path = dir.path().join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    let preview = scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap();
    assert_eq!(preview.manifest.textures.len(), count as usize);
    assert_eq!(
        preview
            .blobs
            .keys()
            .filter(|key| key.starts_with("texture/spectrum/"))
            .count(),
        count as usize
    );
    for prefer_binding_arrays in [true, false] {
        let mut renderer = PreviewRenderer::new_with_texture_arrays(prefer_binding_arrays).unwrap();
        renderer
            .upload_scene(&preview.manifest, &preview.blobs)
            .unwrap();
        let frame = renderer
            .render_with_options(
                camera(),
                128,
                128,
                PreviewMode::Lit,
                0.0,
                scene_view::LitDebug::Albedo,
            )
            .unwrap();
        // The two paths sample the same image beyond the old eight-image limit.
        let pixel = &frame.rgba[(64 * 128 + 48) * 4..(64 * 128 + 48) * 4 + 3];
        if overflow && !prefer_binding_arrays {
            assert!(
                renderer
                    .lit_notes()
                    .iter()
                    .any(|note| note.reason.contains("image array byte budget exceeded"))
            );
            assert!(
                !renderer
                    .lit_notes()
                    .iter()
                    .any(|note| note.reason.contains("missing or invalid imagemap"))
            );
            assert!(
                (pixel[0] as i16 - 137).abs() < 5
                    && pixel[1] < 5
                    && (pixel[2] as i16 - 137).abs() < 5,
                "scaled average was lost: {pixel:?}"
            );
        } else {
            let separation = if overflow { 60 } else { 100 };
            assert!(
                pixel[2] as i16 > pixel[0] as i16 + separation,
                "last image was not sampled (binding arrays preferred={prefer_binding_arrays}): {pixel:?}"
            );
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn debug_albedo_capture_bypasses_lighting_and_film_exposure() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(
        "Film \"rgb\" \"float iso\" 800\nWorldBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.25 0.5 0.75]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"integer indices\" [0 1 2 0 2 3]\n",
    );
    let frame = render_preview_at_time_with_options(
        &preview.manifest,
        &preview.blobs,
        camera(),
        64,
        64,
        PreviewMode::Lit,
        0.0,
        scene_view::LitDebug::Albedo,
    )
    .unwrap();
    let offset = (32 * 64 + 32) * 4;
    let pixel = &frame.rgba[offset..offset + 3];
    let to_srgb = |value: f32| {
        let encoded = if value <= 0.0031308 {
            value * 12.92
        } else {
            1.055 * value.powf(1.0 / 2.4) - 0.055
        };
        (encoded * 255.0).round() as i16
    };
    let expected = [0.25, 0.5, 0.75].map(to_srgb);
    for channel in 0..3 {
        assert!(
            (pixel[channel] as i16 - expected[channel]).abs() <= 2,
            "pixel={pixel:?}, expected={expected:?}"
        );
    }
}

/// Albedo captures keep glass see-through: it has no diffuse color of its own
/// to paint over what is behind.
#[test]
#[ignore = "requires a GPU adapter"]
fn debug_albedo_shows_what_is_behind_glass() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let quad = |z: f64| {
        format!(
            "Shape \"trianglemesh\" \"point3 P\" [-1 -1 {z} 1 -1 {z} 1 1 {z} -1 1 {z}] \"integer indices\" [0 1 2 0 2 3]\n"
        )
    };
    let preview = parse(&format!(
        "WorldBegin\nAttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.8 0.8 0.8]\n{}AttributeEnd\nMaterial \"dielectric\"\n{}",
        quad(0.),
        quad(0.25)
    ));
    let frame = render_preview_at_time_with_options(
        &preview.manifest,
        &preview.blobs,
        camera(),
        64,
        64,
        PreviewMode::Lit,
        0.0,
        scene_view::LitDebug::Albedo,
    )
    .unwrap();
    let offset = (32 * 64 + 32) * 4;
    assert!(
        frame.rgba[offset] > 150,
        "{:?}",
        &frame.rgba[offset..offset + 3]
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn texture_vm_mix_and_scale_feed_diffuse_debug_albedo() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(
        "WorldBegin\nTexture \"red\" \"spectrum\" \"constant\" \"rgb value\" [1 0 0]\nTexture \"blue\" \"spectrum\" \"constant\" \"rgb value\" [0 0 1]\nTexture \"half\" \"spectrum\" \"constant\" \"rgb value\" [0.5 0.5 0.5]\nTexture \"halfblue\" \"spectrum\" \"scale\" \"texture tex1\" [blue] \"texture tex2\" [half]\nTexture \"blend\" \"spectrum\" \"mix\" \"texture tex1\" [red] \"texture tex2\" [halfblue] \"float amount\" [0.5]\nMakeNamedMaterial \"mat\" \"string type\" \"diffuse\" \"texture reflectance\" [blend]\nNamedMaterial \"mat\"\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"texcoord2 uv\" [0 0 1 0 1 1 0 1] \"integer indices\" [0 1 2 0 2 3]\n",
    );
    let frame = render_preview_at_time_with_options(
        &preview.manifest,
        &preview.blobs,
        camera(),
        64,
        64,
        PreviewMode::Lit,
        0.0,
        scene_view::LitDebug::Albedo,
    )
    .unwrap();
    let pixel = &frame.rgba[(32 * 64 + 32) * 4..(32 * 64 + 32) * 4 + 3];
    // 0.5 red mixed with 0.25 blue, encoded to sRGB by the output pass.
    assert!(pixel[0] > 180 && pixel[0] < 195, "pixel={pixel:?}");
    assert!(pixel[2] > 130 && pixel[2] < 145, "pixel={pixel:?}");
    assert!(pixel[1] < 3, "pixel={pixel:?}");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn world_up_appears_in_the_top_half_of_the_viewport_and_pick_buffer() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse("WorldBegin\nTranslate 0 1 0\nShape \"sphere\" \"float radius\" [0.25]\n");
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera(),
        64,
        64,
        PreviewMode::Solid,
    )
    .unwrap();
    assert_ne!(
        frame.pick_at([0.5, 0.26]).unwrap(),
        0,
        "world +Y must appear above the image center"
    );
    assert_eq!(frame.pick_at([0.5, 0.74]).unwrap(), 0);
}

/// A camera inside the scene gets a tiny near plane (protocol::view clamps it
/// to 0.001) while far stays large. Depth must still order two quads half a
/// unit apart at distance 400, or back faces speckle through front ones.
#[test]
#[ignore = "requires a GPU adapter"]
fn depth_keeps_distant_surfaces_ordered_with_a_tiny_near_plane() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview = parse(
        "WorldBegin\nShape \"trianglemesh\" \"point3 P\" [-5 -5 0 5 -5 0 5 5 0 -5 5 0] \"integer indices\" [0 1 2 0 2 3]\nTranslate 0 0 0.5\nShape \"trianglemesh\" \"point3 P\" [-5 -5 0 5 -5 0 5 5 0 -5 5 0] \"integer indices\" [0 1 2 0 2 3]\n",
    );
    let back = preview.manifest.draws[0].pick_id;
    let front = preview.manifest.draws[1].pick_id;
    let camera = ViewerCamera {
        eye: [0., 0., 400.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 2.,
        near: 0.001,
        far: 5000.,
        mirror: false,
    };
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera,
        64,
        64,
        PreviewMode::Solid,
    )
    .unwrap();
    let wrong = frame.pick_ids.iter().filter(|id| **id == back).count();
    assert_eq!(wrong, 0, "{wrong} of 4096 pixels show the back quad");
    assert!(frame.pick_ids.contains(&front));
}

#[test]
#[ignore = "requires a GPU adapter"]
fn lit_imagemap_repeat_filters_across_the_uv_seam() {
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    image::RgbImage::from_fn(2, 1, |x, _| {
        if x == 0 {
            image::Rgb([255u8, 0, 0])
        } else {
            image::Rgb([0, 0, 255])
        }
    })
    .save(dir.path().join("seam.png"))
    .unwrap();
    for wrap in ["repeat", "clamp", "black"] {
        let path = dir.path().join("scene.pbrt");
        std::fs::write(&path, format!(
            "WorldBegin\nTexture \"t\" \"spectrum\" \"imagemap\" \"string filename\" \"seam.png\" \"string encoding\" \"linear\" \"string wrap\" \"{wrap}\"\nMaterial \"diffuse\" \"texture reflectance\" \"t\"\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0 -1 1 0] \"point2 uv\" [0 0.5 0 0.5 0 0.5 0 0.5] \"integer indices\" [0 1 2 0 2 3]\n"
        )).unwrap();
        let preview =
            scene_parse::parse_scene(&path, scene_parse::ParseOptions::default()).unwrap();
        for prefer_binding_arrays in [true, false] {
            let mut renderer =
                PreviewRenderer::new_with_texture_arrays(prefer_binding_arrays).unwrap();
            renderer
                .upload_scene(&preview.manifest, &preview.blobs)
                .unwrap();
            let frame = renderer
                .render_with_options(
                    camera(),
                    32,
                    32,
                    PreviewMode::Lit,
                    0.,
                    scene_view::LitDebug::Albedo,
                )
                .unwrap();
            let pixel = &frame.rgba[(16 * 32 + 16) * 4..(16 * 32 + 16) * 4 + 3];
            if wrap == "repeat" {
                assert!(
                    (pixel[0] as i16 - pixel[2] as i16).abs() < 5 && pixel[0] > 170,
                    "{pixel:?}"
                );
            } else if wrap == "black" {
                assert!(
                    pixel[0] > 180 && pixel[0] < 195 && pixel[2] < 5,
                    "{pixel:?}"
                );
            } else {
                assert!(pixel[0] > 245 && pixel[2] < 5, "{pixel:?}");
            }
        }
    }
}

#[test]
#[ignore = "requires a GPU adapter"]
fn gizmo_projection_lands_on_the_pixel_the_preview_draws() {
    // FX13: a small off-centre sphere seen from an oblique camera at a
    // wide aspect; the gizmo's projected centre must pick it.
    let _guard = GPU.lock().unwrap_or_else(|e| e.into_inner());
    let preview =
        parse("WorldBegin\nTranslate 1.2 0.7 -1\nShape \"sphere\" \"float radius\" [0.08]\n");
    let camera = ViewerCamera {
        eye: [3., 2., 6.],
        target: [0.; 3],
        up: [0., 1., 0.],
        fov_y_degrees: 40.,
        near: 0.1,
        far: 100.,
        mirror: false,
    };
    let (width, height) = (320, 180);
    let frame = render_preview(
        &preview.manifest,
        &preview.blobs,
        camera,
        width,
        height,
        PreviewMode::Solid,
    )
    .unwrap();
    let projector = scene_view::preview::gizmo::Projector::new(&camera, width, height).unwrap();
    let xy = projector.project([1.2, 0.7, -1.]).unwrap();
    let id = preview.manifest.draws[0].pick_id;
    let at = |x: f64, y: f64| {
        frame
            .pick_at([x / width as f64, y / height as f64])
            .unwrap()
    };
    assert_eq!(at(xy[0], xy[1]), id, "projected centre {xy:?}");
    // A mirrored or flipped projection would land well off the sphere.
    assert_eq!(at(width as f64 - xy[0], xy[1]), 0);
    assert_eq!(at(xy[0], height as f64 - xy[1]), 0);
}
