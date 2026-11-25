use pbrt_io::{ImportOptions, export_project, import_into, import_project};
use project::*;
use sha2::Digest;
use std::{cell::Cell, fs, io::Write, path::Path};

fn scene(doc: &ProjectDocument) -> &SceneDocument {
    doc.scene(doc.default_scene.unwrap()).unwrap()
}

fn shapes(doc: &ProjectDocument) -> Vec<NodeId> {
    fn visit(id: NodeId, doc: &ProjectDocument, out: &mut Vec<NodeId>) {
        match &scene(doc).nodes[&id].data {
            NodeKind::Scope { children, .. } => {
                for child in children {
                    visit(*child, doc, out);
                }
            }
            NodeKind::Entity {
                entity: EntityKind::Shape,
                ..
            } => out.push(id),
            _ => {}
        }
    }
    let mut out = vec![];
    visit(scene(doc).world, doc, &mut out);
    out
}

#[test]
fn export_uses_the_explicit_camera_even_when_another_camera_is_last_in_options() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "Translate 1 0 0\nCamera \"perspective\"\nTranslate 10 0 0\nCamera \"orthographic\"\nWorldBegin\nCoordSysTransform \"camera\"\nShape \"sphere\"\n",
    );
    let (store, mut session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let scene_id = session.document().default_scene.unwrap();
    let cameras = scene(session.document())
        .options
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                scene(session.document()).nodes[id].data,
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(cameras.len(), 2);
    session
        .apply(
            &session.revision(),
            &[Edit::SetRenderCamera {
                scene: scene_id,
                camera: Some(cameras[0]),
            }],
        )
        .unwrap();
    let output = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("export"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(output).unwrap();
    assert!(text.contains("Camera \"perspective\""), "{text}");
    assert!(text.contains("Camera \"orthographic\""), "{text}");
    // The world's `CoordSysTransform "camera"` attaches to the
    // import-time camera (the last in Options) as a `parent_camera` group,
    // so no coordinate-system directive remains and the group exports with
    // that camera's camera-to-world matrix regardless of the render camera.
    assert!(
        !text.contains("CoordSysTransform"),
        "import-time resets never re-export: {text}"
    );
    assert!(
        text.rfind("Camera \"perspective\"") > text.rfind("Camera \"orthographic\""),
        "the selected camera is replayed after the original options sequence: {text}"
    );
    // The sphere follows the camera it attached to at import: the
    // orthographic one, even though the perspective one renders.
    let ortho_pose = match &scene(session.document()).nodes[&cameras[1]].data {
        NodeKind::Entity {
            pose: Some(pose), ..
        } => *pose,
        other => panic!("the resolved camera carries its pose: {other:?}"),
    };
    let world = text.find("WorldBegin").unwrap();
    let lines: Vec<&str> = text[world..].lines().collect();
    let shape = lines
        .iter()
        .position(|line| line.starts_with("Shape \"sphere\""))
        .expect("the sphere");
    let attribute = (0..shape)
        .rev()
        .find(|&index| lines[index].starts_with("AttributeBegin"))
        .expect("the camera group");
    assert!(
        lines[attribute + 1].starts_with("Transform ["),
        "the group enters with the camera-to-world transform: {text}"
    );
    let numbers: Vec<f64> = lines[attribute + 1]
        .split_once('[')
        .map(|(_, rest)| rest)
        .unwrap_or_default()
        .trim_end_matches(']')
        .split_whitespace()
        .map(|number| number.parse().unwrap())
        .collect();
    assert_eq!(numbers.len(), 16, "{text}");
    // The file spells matrices transposed against the row-major model.
    let mut emitted = [0f64; 16];
    for row in 0..4 {
        for col in 0..4 {
            emitted[row * 4 + col] = numbers[col * 4 + row];
        }
    }
    for (index, (value, expected)) in emitted.iter().zip(ortho_pose.start).enumerate() {
        assert!(
            (value - expected).abs() <= 1e-12 * expected.abs().max(1.0),
            "group transform[{index}]: {value} vs {expected}\n{text}"
        );
    }
}

#[test]
fn selected_camera_replay_restores_medium_color_space_and_preserves_camera_coordinates() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "MakeNamedMedium \"fog_a\" \"string type\" [\"homogeneous\"]\nMakeNamedMedium \"fog_b\" \"string type\" [\"homogeneous\"]\nColorSpace \"ACEScg\"\nMediumInterface \"fog_a\" \"\"\nCamera \"perspective\"\nCoordSysTransform \"camera\"\nColorSpace \"DisplayP3\"\nMediumInterface \"fog_b\" \"\"\nCamera \"orthographic\"\nWorldBegin\nCoordSysTransform \"camera\"\nShape \"sphere\"\n",
    );
    let (store, mut session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let scene_id = session.document().default_scene.unwrap();
    let cameras = scene(session.document())
        .options
        .iter()
        .copied()
        .filter(|id| {
            matches!(
                scene(session.document()).nodes[id].data,
                NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(cameras.len(), 2);
    session
        .apply(
            &session.revision(),
            &[
                Edit::SetRenderCamera {
                    scene: scene_id,
                    camera: Some(cameras[0]),
                },
                Edit::SetParameter {
                    target: ParameterTarget::Node(cameras[0]),
                    name: "lenscolor".into(),
                    value_type: "rgb".into(),
                    value: Some(Parameter {
                        value_type: "rgb".into(),
                        name: "lenscolor".into(),
                        color_space: ColorSpaceBinding::Explicit("ACEScg".into()),
                        values: ParameterValues::Floats(
                            [0.2, 0.4, 0.8]
                                .into_iter()
                                .map(FloatValue::Finite)
                                .collect(),
                        ),
                    }),
                },
            ],
        )
        .unwrap();
    let output = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("export"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&output).unwrap();
    let world = text.find("WorldBegin").unwrap();
    let options = &text[..world];
    let replay = options.rfind("Camera \"perspective\"").unwrap();
    assert!(replay < options.len());
    // Every `CoordSysTransform` is resolved at import; the
    // world's camera-relative group enters as a plain `Transform` and the
    // options-level one is swept with the rest of the options transform run.
    assert!(
        !text.contains("CoordSysTransform"),
        "import-time resets never re-export: {text}"
    );
    let selected_color = options.rfind("ColorSpace \"ACEScg\"").unwrap();
    let selected_medium = options.rfind("MediumInterface \"fog_a\"").unwrap();
    let restore_color = options.rfind("ColorSpace \"DisplayP3\"").unwrap();
    let restore_medium = options.rfind("MediumInterface \"fog_b\"").unwrap();
    assert!(
        selected_color < replay && selected_medium < replay,
        "{options}"
    );
    assert!(
        replay < restore_color && replay < restore_medium,
        "{options}"
    );
    assert!(
        text.contains("Camera \"perspective\" \"rgb lenscolor\""),
        "{text}"
    );

    // The options-level `CoordSysTransform "camera"` between the first
    // camera and the replay must still resolve when the export is read back
    // (PR review verification): the reimport keeps both cameras, their
    // evaluated matrices, and the render-camera selection.
    let (_, reimported) = import_project(
        &output,
        &temp.path().join("again"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    fn camera_matrices(
        view: &project::SceneView,
        evaluated: &project::EvaluatedScene,
    ) -> std::collections::BTreeMap<String, ([f64; 16], [f64; 16])> {
        let mut out = std::collections::BTreeMap::new();
        for id in &view.scene.options {
            if let Some(NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation,
                ..
            }) = view.node(*id).map(|node| &node.data)
            {
                let state = &evaluated.nodes[id];
                out.insert(implementation.clone(), (state.start.0, state.end.0));
            }
        }
        out
    }
    let snapshot = reimported.snapshot();
    let view = snapshot.scene_view().unwrap();
    let evaluated = evaluate(&view).unwrap();
    let original_snapshot = session.snapshot();
    let original_view = original_snapshot.scene_view().unwrap();
    let original_evaluated = evaluate(&original_view).unwrap();
    assert_eq!(
        camera_matrices(&view, &evaluated),
        camera_matrices(&original_view, &original_evaluated),
        "both cameras and their evaluated matrices must survive the round trip"
    );
    assert_eq!(
        camera_matrices(&view, &evaluated).len(),
        2,
        "a camera directive may not duplicate on reimport"
    );
    assert!(
        view.scene.render_camera.is_some(),
        "the render camera selection must survive the round trip"
    );
}

#[test]
fn import_into_writes_assets_and_project_records_to_folder_store() {
    let temp = tempfile::tempdir().unwrap();
    let source_dir = temp.path().join("source");
    fs::create_dir(&source_dir).unwrap();
    let mesh = b"managed mesh data";
    fs::write(source_dir.join("mesh.ply"), mesh).unwrap();
    let source = source_dir.join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" [\"mesh.ply\"]\n",
    );
    let project_path = temp.path().join("project");
    let project_id = ProjectId::new();
    let mut store =
        FolderStore::create(&project_path, project_id, "Imported", Revision::initial()).unwrap();

    let (session, report) =
        import_into(&source, &mut store, &ImportOptions::default(), || false).unwrap();
    assert!(report.diagnostics.is_empty());
    assert_eq!(session.document().id, project_id);
    let asset_id = *session.document().assets.keys().next().unwrap();
    fs::write(project_path.join("Assets/000-notes.txt"), b"unmanaged").unwrap();
    assert_eq!(fs::read(store.asset_path(asset_id).unwrap()).unwrap(), mesh);
    assert!(project_path.join("Imports").is_dir());
    assert!(project_path.join("Assets/Scenes/scene.scene").is_file());
    drop(store);
    assert!(ProjectStore::open(&project_path).is_ok());
}

#[test]
fn gzip_ply_scene_asset_is_classified_as_a_mesh_and_keeps_its_name() {
    use std::io::Write as _;
    let temp = tempfile::tempdir().unwrap();
    let source_dir = temp.path().join("source");
    fs::create_dir(&source_dir).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder
        .write_all(b"ply\nformat ascii 1.0\nelement vertex 0\nend_header\n")
        .unwrap();
    let compressed = encoder.finish().unwrap();
    fs::write(source_dir.join("mesh.ply.gz"), &compressed).unwrap();
    let source = source_dir.join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" [\"mesh.ply.gz\"]\n",
    );
    let project_path = temp.path().join("project");
    let mut store = FolderStore::create(
        &project_path,
        ProjectId::new(),
        "Imported",
        Revision::initial(),
    )
    .unwrap();

    let (session, report) =
        import_into(&source, &mut store, &ImportOptions::default(), || false).unwrap();
    assert!(report.diagnostics.is_empty());
    let asset = session.document().assets.values().next().unwrap();
    assert_eq!(asset.media_type, "application/x-ply");
    assert_eq!(asset.location, "Meshes/mesh.ply.gz");
    assert_eq!(
        fs::read(store.asset_path(asset.id).unwrap()).unwrap(),
        compressed
    );
}
fn write(path: &Path, text: &str) {
    fs::write(path, text).unwrap();
}

fn measured_bsdf() -> Vec<u8> {
    let fields = [
        ("wavelengths", vec![3u64], vec![465f32, 532., 630.]),
        ("spectra", vec![1u64, 1, 3, 1, 1], vec![0.2f32, 0.5, 0.9]),
    ];
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"tensor_file\0");
    bytes.extend([1, 0]);
    bytes.extend((fields.len() as u32).to_le_bytes());
    let mut offsets = Vec::new();
    for (name, shape, _) in &fields {
        bytes.extend((name.len() as u16).to_le_bytes());
        bytes.extend(name.as_bytes());
        bytes.extend((shape.len() as u16).to_le_bytes());
        bytes.push(10);
        offsets.push(bytes.len());
        bytes.extend([0; 8]);
        for dim in shape {
            bytes.extend(dim.to_le_bytes());
        }
    }
    let mut offset = bytes.len() as u64;
    for ((_, _, values), offset_at) in fields.iter().zip(offsets) {
        bytes[offset_at..offset_at + 8].copy_from_slice(&offset.to_le_bytes());
        offset += values.len() as u64 * 4;
    }
    for (_, _, values) in fields {
        for value in values {
            bytes.extend(value.to_le_bytes());
        }
    }
    bytes
}

/// A project's infinite light keeps its map through the preview package, under
/// the blob name the renderer looks up.
#[test]
fn project_previews_carry_infinite_light_maps() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("env.exr"), b"map bytes").unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nLightSource \"infinite\" \"string filename\" \"env.exr\"\nShape \"sphere\"\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let bundle = temp.path().join("bundle");
    fs::create_dir(&bundle).unwrap();
    let snapshot = session.snapshot();
    let assets = pbrt_io::stage_assets(snapshot.document(), &store, &bundle, &|| false).unwrap();
    let program = pbrt_io::program_from_project(&snapshot, &store, &assets, || false).unwrap();
    let preview = scene_parse::preview_from_program(
        program,
        None,
        &bundle,
        Default::default(),
        &mut scene_parse::GeometryCache::new(0),
    )
    .unwrap();
    assert_eq!(preview.blobs["light/0"], b"map bytes");
    let package = temp.path().join("package");
    fs::create_dir(&package).unwrap();
    scene_parse::write_preview(&preview, &package, true).unwrap();
    let read = scene_parse::read_preview_package(&package).unwrap();
    assert_eq!(read.blobs["light/0"], b"map bytes");
}

#[test]
fn measured_asset_is_staged_for_project_preview_and_preview_color_never_enters_export() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("sample.bsdf"), measured_bsdf()).unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nMakeNamedMaterial \"measured\" \"string type\" \"measured\" \"string filename\" \"sample.bsdf\"\nNamedMaterial \"measured\"\nShape \"sphere\"\n",
    );
    let project_path = temp.path().join("project");
    let (store, session) =
        import_project(&source, &project_path, &ImportOptions::default(), || false).unwrap();
    let snapshot = session.snapshot();
    let measured_resource = snapshot
        .document()
        .resources
        .values()
        .find(|r| r.implementation == "measured")
        .unwrap();
    assert!(
        measured_resource
            .parameters
            .iter()
            .any(|p| p.name == "filename")
    );
    assert!(
        !measured_resource
            .parameters
            .iter()
            .any(|p| p.name == "reflectance")
    );

    let bundle = temp.path().join("preview-bundle");
    fs::create_dir(&bundle).unwrap();
    let assets = pbrt_io::stage_snapshot_assets(&snapshot, &store, &bundle, &|| false).unwrap();
    assert_eq!(assets.len(), 1);
    let program = pbrt_io::program_from_project(&snapshot, &store, &assets, || false).unwrap();
    let preview = scene_parse::preview_from_program(
        program,
        None,
        &bundle,
        Default::default(),
        &mut scene_parse::GeometryCache::new(0),
    )
    .unwrap();
    let record = preview
        .manifest
        .materials
        .iter()
        .find(|m| m.implementation == "measured")
        .unwrap();
    let reflectance = record
        .parameters
        .iter()
        .find(|p| p.name == "reflectance")
        .unwrap();
    assert_eq!(reflectance.value_type, "rgb");
    for (actual, expected) in reflectance.floats.iter().zip([0.9, 0.5, 0.2]) {
        assert!((actual - expected).abs() < 1e-6);
    }

    let exported =
        export_project(&snapshot, &store, &temp.path().join("exported"), || false).unwrap();
    let exported_text = fs::read_to_string(&exported).unwrap();
    assert!(exported_text.contains("filename"));
    assert!(!exported_text.contains("reflectance"), "{exported_text}");
    let exported_asset = fs::read_dir(exported.parent().unwrap().join("assets"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "bsdf")
        })
        .unwrap();
    assert_eq!(fs::read(exported_asset).unwrap(), measured_bsdf());
}

#[test]
fn thin_hair_and_subsurface_materials_round_trip_representative_parameters() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nMakeNamedMaterial \"thin\" \"string type\" \"thindielectric\" \"float eta\" [1.33]\nMakeNamedMaterial \"hair\" \"string type\" \"hair\" \"float eta\" [1.7]\nMakeNamedMaterial \"skin\" \"string type\" \"subsurface\" \"float scale\" [0.8]\nNamedMaterial \"thin\"\nShape \"sphere\"\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("export"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(exported).unwrap();
    assert!(
        text.contains("\"string type\" [\"thindielectric\"] \"float eta\" [ 1.33 ]"),
        "{text}"
    );
    assert!(
        text.contains("\"string type\" [\"hair\"] \"float eta\" [ 1.7 ]"),
        "{text}"
    );
    assert!(
        text.contains("\"string type\" [\"subsurface\"] \"float scale\" [ 0.8 ]"),
        "{text}"
    );
}

/// pbrt resolves a mix's material names as it reads, and resources export by
/// location: "a_mix" would otherwise come before its "b_child".
#[test]
fn export_defines_mix_children_before_the_mix() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nMakeNamedMaterial \"c_leaf\" \"string type\" \"diffuse\"\nMakeNamedMaterial \"b_child\" \"string type\" \"mix\" \"string materials\" [\"c_leaf\" \"c_leaf\"]\nMakeNamedMaterial \"a_mix\" \"string type\" \"mix\" \"string materials\" [\"b_child\" \"c_leaf\"]\nNamedMaterial \"a_mix\"\nShape \"sphere\"\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let output = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(output).unwrap();
    let at = |name: &str| text.find(&format!("MakeNamedMaterial \"{name}\"")).unwrap();
    assert!(
        at("c_leaf") < at("b_child") && at("b_child") < at("a_mix"),
        "{text}"
    );
}

#[test]
fn export_binds_direct_shapes_and_instance_placements_to_native_nodes() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.pbrt");
    write(
        &source,
        "WorldBegin\nObjectBegin \"ball\"\nShape \"sphere\"\nObjectEnd\nTranslate 2 0 0\nObjectInstance \"ball\"\nShape \"disk\"\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let output = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(output.with_file_name("export.json")).unwrap()).unwrap();
    let preview = scene_parse::parse_scene(&output, Default::default()).unwrap();
    assert_eq!(preview.manifest.draws.len(), 2);
    for draw in &preview.manifest.draws {
        let location = draw
            .instance_source
            .as_ref()
            .map(|s| &s.source)
            .or(draw.source.as_ref())
            .unwrap();
        let id: NodeId =
            serde_json::from_value(manifest["node_sources"][location.line.to_string()].clone())
                .unwrap();
        let node = &scene(session.document()).nodes[&id];
        if draw.instance_source.is_some() {
            assert!(matches!(node.data, NodeKind::Instance { .. }));
        } else {
            assert!(matches!(
                node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                }
            ));
        }
    }
}

#[test]
fn repeated_gzip_includes_and_assets_become_independent_of_original_paths() {
    let temp = tempfile::tempdir().unwrap();
    let original = temp.path().join("original");
    fs::create_dir(&original).unwrap();
    let child = "Texture \"albedo\" \"spectrum\" \"imagemap\" \"string filename\" [\"albedo.exr\"]\nMaterial \"diffuse\" \"texture reflectance\" [\"albedo\"]\nShape \"sphere\" \"float radius\" [1]\n";
    // One texture declaration outside repeated includes, because texture redefinition is rejected.
    let root = "Camera \"perspective\"\nWorldBegin\nTexture \"albedo\" \"spectrum\" \"imagemap\" \"string filename\" [\"albedo.exr\"]\nInclude \"child.pbrt.gz\"\nTranslate 3 0 0\nInclude \"child.pbrt.gz\"\n";
    let child = &child[child.find("Material").unwrap()..];
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    gz.write_all(child.as_bytes()).unwrap();
    let compressed = gz.finish().unwrap();
    fs::write(original.join("child.pbrt.gz"), &compressed).unwrap();
    write(&original.join("root.pbrt"), root);
    fs::write(original.join("albedo.exr"), b"managed texture bytes").unwrap();
    let path = temp.path().join("project");
    let (mut store, mut session) = import_project(
        &original.join("root.pbrt"),
        &path,
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let ids = shapes(session.document());
    assert_eq!(ids.len(), 2);
    let import = session.document().imports.values().next().unwrap();
    assert_eq!(import.sources.len(), 2);
    assert_eq!(import.occurrences.len(), 3);
    let first = scene(session.document()).nodes[&ids[0]]
        .origin
        .clone()
        .unwrap();
    let second = scene(session.document()).nodes[&ids[1]]
        .origin
        .clone()
        .unwrap();
    assert_ne!(first.occurrence, second.occurrence);
    assert_eq!(
        &child.as_bytes()
            [first.span.offset as usize..(first.span.offset + first.span.length) as usize],
        b"Shape \"sphere\" \"float radius\" [1]"
    );
    let source = &import.sources[&import.occurrences[&first.occurrence].source];
    assert_eq!(source.original.size, compressed.len() as u64);
    assert_eq!(
        source.original.sha256,
        format!("{:x}", sha2::Sha256::digest(&compressed))
    );
    let view = session.document().default_scene_view().unwrap();
    let evaluated = evaluate(&view).unwrap();
    assert_eq!(evaluated.nodes[&ids[1]].start.0[3], 3.);
    session
        .apply(
            &session.revision(),
            &[Edit::SetParameter {
                target: ParameterTarget::Node(ids[0]),
                name: "radius".into(),
                value_type: "float".into(),
                value: Some(Parameter::floats("radius", [2.])),
            }],
        )
        .unwrap();
    store.save(&session.snapshot()).unwrap();
    drop(store);
    fs::rename(&original, temp.path().join("removed-original")).unwrap();
    let (mut store, session) = ProjectStore::open(&path).unwrap();
    assert_eq!(shapes(session.document()), ids);
    store.save(&session.snapshot()).unwrap();
    store.save(&session.snapshot()).unwrap();
    let asset_id = *session.document().assets.keys().next().unwrap();
    let asset_meta: serde_json::Value = serde_json::from_slice(
        &fs::read(format!(
            "{}.meta",
            store.asset_path(asset_id).unwrap().display()
        ))
        .unwrap(),
    )
    .unwrap();
    assert!(if asset_meta["kind"] == "image_texture" {
        asset_meta["image"]["origin"].is_object()
    } else {
        asset_meta["origin"].is_object()
    });
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&exported).unwrap();
    assert!(!text.contains("Include"));
    assert!(!text.contains(original.to_str().unwrap()));
    let (_, copy) = import_project(
        &exported,
        &temp.path().join("copy"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let copied = shapes(copy.document());
    assert_eq!(copied.len(), 2);
    let radius = |doc: &ProjectDocument, id| {
        let NodeKind::Entity { parameters, .. } = &scene(doc).nodes[&id].data else {
            unreachable!()
        };
        parameters[0].clone()
    };
    assert_eq!(
        radius(copy.document(), copied[0]),
        Parameter::floats("radius", [2.])
    );
    assert_eq!(
        radius(copy.document(), copied[1]),
        Parameter::floats("radius", [1.])
    );
    assert_eq!(
        copy.document().assets.values().next().unwrap().object(),
        session.document().assets.values().next().unwrap().object()
    );
}

#[test]
fn exported_assets_are_independent_copies_of_folder_assets() {
    let temp = tempfile::tempdir().unwrap();
    let source_dir = temp.path().join("source");
    fs::create_dir(&source_dir).unwrap();
    let original = b"asset before export";
    fs::write(source_dir.join("mesh.ply"), original).unwrap();
    let source = source_dir.join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" [\"mesh.ply\"]\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let asset_id = *session.document().assets.keys().next().unwrap();
    let source_asset = store.asset_path(asset_id).unwrap();
    let output = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let mut assets = fs::read_dir(output.parent().unwrap().join("assets")).unwrap();
    let exported_asset = assets.next().unwrap().unwrap().path();
    assert!(assets.next().is_none());

    fs::write(&source_asset, b"changed project asset").unwrap();
    assert_eq!(fs::read(&exported_asset).unwrap(), original);
    fs::write(&exported_asset, b"changed exported asset").unwrap();
    assert_eq!(fs::read(&source_asset).unwrap(), b"changed project asset");
}

#[test]
fn transform_scope_animation_coordinates_and_camera_match_the_existing_parser() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "LookAt 0 0 -5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nCoordSysTransform \"camera\"\nCoordinateSystem \"saved\"\nIdentity\nTransformBegin\nTranslate 2 0 0\nMaterial \"diffuse\" \"rgb reflectance\" [.2 .3 .4]\nReverseOrientation\nShape \"sphere\"\nTransformEnd\nShape \"sphere\"\nAttributeBegin\nActiveTransform EndTime\nTranslate 0 4 0\nActiveTransform All\nShape \"sphere\"\nAttributeEnd\nCoordSysTransform \"saved\"\nShape \"sphere\"\n",
    );
    let document = scene_parse::document::read_document(&source).unwrap();
    let old = scene_parse::state_graph::parse_state_graph(&document.directives).unwrap();
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let view = session.document().default_scene_view().unwrap();
    let native = evaluate(&view).unwrap();
    for node in scene(session.document()).nodes.values() {
        let Some(origin) = &node.origin else { continue };
        if !matches!(node.data, NodeKind::Entity { .. } | NodeKind::State { .. }) {
            continue;
        }
        let previous = old
            .nodes
            .iter()
            .find(|n| document.directives[n.directive].line == origin.span.line)
            .unwrap();
        if let NodeKind::Entity {
            entity: EntityKind::Camera,
            pose: Some(pose),
            ..
        } = &node.data
        {
            // The camera's placement lives in its pose (the inverse
            // of the transform current at its directive); the options
            // transform run that produced it no longer exists in the model,
            // so the state line comparison gives way to the pose relation.
            assert_eq!(
                pose.start,
                math::inverse(math::Mat4(previous.state.start.0))
                    .expect("the parser's camera transform is invertible")
                    .0,
                "{} holds the inverse of its directive-time transform (line {})",
                node.label,
                origin.span.line
            );
            assert_eq!(
                pose.end,
                math::inverse(math::Mat4(previous.state.end.0))
                    .expect("the parser's camera transform is invertible")
                    .0,
                "{} holds the inverse of its directive-time end sample (line {})",
                node.label,
                origin.span.line
            );
            continue;
        }
        let state = &native.nodes[&node.id];
        assert_eq!(
            state.start.0, previous.state.start.0,
            "{} at line {}",
            node.label, origin.span.line
        );
        assert_eq!(
            state.end.0, previous.state.end.0,
            "{} at line {}",
            node.label, origin.span.line
        );
        assert_eq!(
            state.reverse_orientation, previous.state.reverse,
            "{} at line {}",
            node.label, origin.span.line
        );
    }
    // The resolution moves r4 into its coordinate system's group, so the
    // pre-order shifts; both sides keep it, so compare by pre-order and
    // single out the post-scope shape by its source line.
    let ids = shapes(session.document());
    let line_of = |id: NodeId| {
        session
            .document()
            .scene(session.document().default_scene.unwrap())
            .unwrap()
            .nodes[&id]
            .origin
            .as_ref()
            .expect("an imported shape keeps its origin")
            .span
            .line
    };
    let after_scope = *ids
        .iter()
        .find(|id| line_of(**id) == 13)
        .expect("the shape after the converted scope");
    assert_eq!(native.nodes[&after_scope].start, math::Mat4::identity());
    assert!(matches!(
        native.nodes[&after_scope].material,
        MaterialBinding::Resource(_)
    ));
    assert!(native.nodes[&after_scope].reverse_orientation);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let (_, copy) = import_project(
        &exported,
        &temp.path().join("copy"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let copy_view = copy.document().default_scene_view().unwrap();
    let reevaluated = evaluate(&copy_view).unwrap();
    // The exported text holds no resets, so the re-imported pre-order keeps
    // the resolution's shape order (the moved r4 first).
    let copies = shapes(copy.document());
    assert_eq!(ids.len(), copies.len());
    for (left, right) in ids.iter().zip(copies) {
        assert_eq!(native.nodes[left].start, reevaluated.nodes[&right].start);
        assert_eq!(native.nodes[left].end, reevaluated.nodes[&right].end);
    }
}

#[test]
fn exact_integers_parameter_omission_duplicates_and_option_quoting_are_preserved() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "Option \"rendercoordsys\" world\nSampler \"independent\" \"integer seed\" [9007199254740993 -9223372036854775808 9223372036854775807]\nWorldBegin\nShape \"sphere\" \"float radius\" [2] \"float radius\" [3]\n",
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let document = session.document();
    let sampler = scene(document)
        .options
        .iter()
        .find_map(|id| match &scene(document).nodes[id].data {
            NodeKind::Configuration { parameters, .. } => Some(parameters),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        sampler[0].values,
        ParameterValues::Integers(vec![9007199254740993, i64::MIN, i64::MAX])
    );
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(exported).unwrap();
    assert!(text.contains("9007199254740993"));
    assert!(text.contains("Option \"rendercoordsys\" \"world\""));
    let id = shapes(session.document())[0];
    let NodeKind::Entity { parameters, .. } = &scene(session.document()).nodes[&id].data else {
        unreachable!()
    };
    assert_eq!(parameters.len(), 2);
}

#[test]
fn import_failures_and_cancellation_never_publish_or_replace_a_project() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let dest = temp.path().join("project");
    // Uninterpretable input is skipped with warnings (tests/import_recovery.rs);
    // cancellation, budgets and an existing destination still fail.
    write(&source, "WorldBegin\nShape \"sphere\"\n");
    let count = Cell::new(0);
    let result = import_project(&source, &dest, &ImportOptions::default(), || {
        count.set(count.get() + 1);
        count.get() > 5
    });
    assert!(matches!(
        result,
        Err(pbrt_io::Error::Project(ProjectError::Cancelled))
    ));
    assert!(!dest.exists());
    let options = ImportOptions {
        max_source_bytes: 4,
        ..Default::default()
    };
    assert!(import_project(&source, &dest, &options, || false).is_err());
    assert!(!dest.exists());
    fs::create_dir(&dest).unwrap();
    write(&dest.join("keep"), "original");
    assert!(import_project(&source, &dest, &ImportOptions::default(), || false).is_err());
    assert_eq!(fs::read_to_string(dest.join("keep")).unwrap(), "original");
}

#[test]
fn cancellation_during_final_save_as_does_not_publish_the_imported_project() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(&source, "WorldBegin\nShape \"sphere\"\n");

    let successful_calls = Cell::new(0);
    import_project(
        &source,
        &temp.path().join("successful"),
        &ImportOptions::default(),
        || {
            successful_calls.set(successful_calls.get() + 1);
            false
        },
    )
    .unwrap();
    let cancel_at = successful_calls.get();

    let calls = Cell::new(0);
    let destination = temp.path().join("cancelled-final-save");
    let result = import_project(&source, &destination, &ImportOptions::default(), || {
        calls.set(calls.get() + 1);
        calls.get() == cancel_at
    });
    assert!(matches!(
        result,
        Err(pbrt_io::Error::Project(ProjectError::Cancelled))
    ));
    assert!(!destination.exists());
}

#[test]
fn exporting_a_snapshot_is_deterministic_and_cancellation_leaves_no_bundle() {
    let temp = tempfile::tempdir().unwrap();
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/instances-preview.pbrt");
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let a = export_project(&session.snapshot(), &store, &temp.path().join("a"), || {
        false
    })
    .unwrap();
    let b = export_project(&session.snapshot(), &store, &temp.path().join("b"), || {
        false
    })
    .unwrap();
    assert_eq!(fs::read(&a).unwrap(), fs::read(b).unwrap());
    let preview = scene_parse::parse_scene(&a, Default::default()).unwrap();
    assert!(!preview.manifest.draws.is_empty());
    assert!(!preview.blobs.is_empty());
    // The streaming hash recorded in export.json must equal the digest of
    // the scene file actually published to disk.
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(a.with_file_name("export.json")).unwrap()).unwrap();
    use sha2::Digest;
    assert_eq!(
        manifest["scene_sha256"],
        format!("{:x}", sha2::Sha256::digest(&fs::read(&a).unwrap())),
        "scene_sha256 must match the published scene.pbrt"
    );
    let cancelled = temp.path().join("cancelled");
    assert!(matches!(
        export_project(&session.snapshot(), &store, &cancelled, || true),
        Err(pbrt_io::Error::Project(ProjectError::Cancelled))
    ));
    assert!(!cancelled.exists());
}

#[test]
fn changed_sources_are_detected_after_copy_and_unicode_asset_paths_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let dest = temp.path().join("project");
    write(&source, "WorldBegin\nShape \"sphere\"\n");
    let count = Cell::new(0);
    let result = import_project(&source, &dest, &ImportOptions::default(), || {
        count.set(count.get() + 1);
        if count.get() == 5 {
            write(&source, "WorldBegin\nShape \"cylinder\"\n");
        }
        false
    });
    assert!(result.is_err());
    assert!(!dest.exists());
    write(
        &source,
        "WorldBegin\nTexture \"模様\" \"spectrum\" \"imagemap\" \"string filename\" [\"模様.exr\"]\nMaterial \"diffuse\" \"texture reflectance\" [\"模様\"]\nShape \"sphere\"\n",
    );
    fs::write(temp.path().join("模様.exr"), b"texture").unwrap();
    let (store, session) =
        import_project(&source, &dest, &ImportOptions::default(), || false).unwrap();
    assert!(
        session
            .document()
            .assets
            .values()
            .next()
            .unwrap()
            .origin
            .as_ref()
            .unwrap()
            .logical_path
            .ends_with("模様.exr")
    );
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    assert!(exported.exists());
}

#[test]
fn large_arrays_builtin_spectra_and_medium_bindings_roundtrip() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let points = std::iter::repeat_n("0", 6000).collect::<Vec<_>>().join(" ");
    write(
        &source,
        &format!(
            "MakeNamedMedium \"fog\" \"string type\" [\"homogeneous\"] \"rgb sigma_s\" [.1 .2 .3]\nMediumInterface \"fog\" \"\"\nCamera \"perspective\"\nTransformTimes 2 3\nWorldBegin\nMaterial \"conductor\" \"spectrum eta\" [\"metal-Cu-eta\"]\nShape \"trianglemesh\" \"point3 P\" [{points}] \"integer indices\" [0 1 2]\n"
        ),
    );
    let (store, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let id = shapes(session.document())[0];
    let NodeKind::Entity { parameters, .. } = &scene(session.document()).nodes[&id].data else {
        unreachable!()
    };
    assert!(matches!(
        parameters[0].values,
        ParameterValues::Array { count: 6000, .. }
    ));
    let view = session.document().default_scene_view().unwrap();
    let states = evaluate(&view).unwrap();
    assert_eq!(states.nodes[&id].times, [2., 3.]);
    assert!(states.nodes[&id].inside_medium.is_some());
    let camera = view
        .scene
        .options
        .iter()
        .find(|id| {
            matches!(
                view.node(**id).map(|node| &node.data),
                Some(NodeKind::Entity {
                    entity: EntityKind::Camera,
                    ..
                })
            )
        })
        .copied()
        .unwrap();
    let camera_index = view
        .scene
        .options
        .iter()
        .position(|id| *id == camera)
        .unwrap();
    let camera_medium = view.scene.options[..camera_index]
        .iter()
        .rev()
        .find_map(|id| match view.node(*id).map(|node| &node.data) {
            Some(NodeKind::State {
                operation: StateOperation::Medium { inside, outside },
            }) => Some((*inside, *outside)),
            _ => None,
        })
        .unwrap();
    assert!(camera_medium.0.is_some());
    assert_eq!(camera_medium.1, None);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let (_, copy) = import_project(
        &exported,
        &temp.path().join("copy"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    assert_eq!(
        copy.document()
            .object_refs()
            .unwrap()
            .iter()
            .filter(|o| o.size == 48000)
            .count(),
        1
    );
}

#[test]
fn unknown_dependencies_added_by_editing_block_export() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(&source, "WorldBegin\nShape \"sphere\"\n");
    let (store, mut session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let id = shapes(session.document())[0];
    session
        .apply(
            &session.revision(),
            &[Edit::SetParameter {
                target: ParameterTarget::Node(id),
                name: "unknown_input".into(),
                value_type: "string".into(),
                value: Some(Parameter {
                    value_type: "string".into(),
                    name: "unknown_input".into(),
                    color_space: ColorSpaceBinding::Inherit,
                    values: ParameterValues::Strings(vec!["/original/file.exr".into()]),
                }),
            }],
        )
        .unwrap();
    let bundle = temp.path().join("bundle");
    assert!(matches!(
        export_project(&session.snapshot(), &store, &bundle, || false),
        Err(pbrt_io::Error::Unsupported(_))
    ));
    assert!(!bundle.exists());
}

#[test]
fn expansion_budget_failures_stop_the_import() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(&source, "WorldBegin\nShape \"sphere\"\n");
    let options = ImportOptions {
        max_expanded_bytes: 4,
        ..Default::default()
    };
    assert!(import_project(&source, &temp.path().join("project"), &options, || false).is_err());
}

/// Unnamed AttributeBegin/TransformBegin scopes import as "Group", the
/// label Create ▸ Group gives, rather than the directive's name.
#[test]
fn imported_scopes_are_labeled_group() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    write(
        &source,
        "WorldBegin\nAttributeBegin\nShape \"sphere\"\nAttributeEnd\nTransformBegin\nShape \"disk\"\nTransformEnd\n",
    );
    let (_, session) = import_project(
        &source,
        &temp.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let scene = scene(session.document());
    let NodeKind::Scope { children, .. } = &scene.nodes[&scene.world].data else {
        panic!("world is a scope")
    };
    let labels: Vec<_> = children
        .iter()
        .map(|id| &scene.nodes[id])
        .filter(|node| matches!(node.data, NodeKind::Scope { .. }))
        .map(|node| node.label.as_str())
        .collect();
    assert_eq!(labels, ["Group", "Group"]);
}
