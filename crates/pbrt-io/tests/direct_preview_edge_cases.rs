//! Direct-preview edge cases that need a synthesized project document.
//! The emitter's re-lex check has a unit test beside its implementation.
//! Matrix preview parity covers the transform cases; other edge cases
//! are tested beside their implementations.
use pbrt_io::{export_project, export_warnings, program_from_project, stage_assets};
use project::{
    ArrayEncoding, Asset, AssetId, FolderStore, Node, NodeKind, Parameter, ParameterValues,
    ProjectDocument, ProjectSession, ProjectStore, Revision, StateOperation, TransformOperation,
};
use scene_parse::{
    Directive, PreviewProgram, Value, document::ReadOptions, document::read_document_with_options,
};
use sha2::Digest;
use std::{collections::BTreeMap, fs, path::Path};

fn scene_mut(document: &mut ProjectDocument) -> &mut project::SceneDocument {
    let id = document.default_scene.unwrap();
    std::sync::Arc::make_mut(document.scenes.get_mut(&id).unwrap())
}

fn append_world(document: &mut ProjectDocument, node: Node) {
    let scene = scene_mut(document);
    let world = scene.world;
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        panic!("missing World");
    };
    children.push(node.id);
    scene.nodes.insert(node.id, node);
}

fn shape(implementation: &str, parameters: Vec<Parameter>) -> Node {
    Node::new(
        "Shape",
        NodeKind::Entity {
            entity: project::EntityKind::Shape,
            implementation: implementation.into(),
            parameters,
            pose: None,
        },
    )
}

fn finish(
    store: ProjectStore,
    document: ProjectDocument,
) -> (project::ProjectSnapshot, ProjectStore) {
    let session = ProjectSession::new(document).unwrap();
    (session.snapshot(), store)
}

/// Run the text path and the direct path on one snapshot and return both
/// directive streams (the caller asserts the case-specific content).
fn both_paths(
    snapshot: &project::ProjectSnapshot,
    store: &ProjectStore,
    temp: &Path,
) -> (Vec<Directive>, PreviewProgram, BTreeMap<AssetId, String>) {
    let text = export_project(snapshot, store, &temp.join("text"), || false).unwrap();
    let text_document = read_document_with_options(
        &text,
        ReadOptions {
            expand_imports: true,
            ..Default::default()
        },
    )
    .unwrap();
    let bundle = temp.join("direct");
    fs::create_dir_all(&bundle).unwrap();
    let assets = stage_assets(snapshot.document(), store, &bundle, &|| false).unwrap();
    let program = program_from_project(snapshot, store, &assets, || false).unwrap();
    (text_document.directives, program, assets)
}

fn find<'a>(directives: &'a [Directive], name: &str) -> &'a Directive {
    directives
        .iter()
        .find(|directive| directive.name == name)
        .unwrap_or_else(|| panic!("missing {name} directive"))
}

fn parameter<'a>(directive: &'a Directive, name: &str) -> &'a scene_parse::Parameter {
    directive
        .parameters
        .iter()
        .find(|parameter| parameter.name == name)
        .unwrap_or_else(|| panic!("missing {name} parameter on {}", directive.name))
}

/// Parameter equality minus the source location, which differs by path.
fn assert_parameter_content_equal(left: &scene_parse::Parameter, right: &scene_parse::Parameter) {
    assert_eq!(left.value_type, right.value_type);
    assert_eq!(left.name, right.name);
    assert_eq!(left.values, right.values);
    assert_eq!(left.value_tokens, right.value_tokens);
}

/// A project without a Film node gets the same injected default on both
/// paths, including the packed trailing filename parameter.
#[test]
fn film_default_is_injected_identically_without_a_film_node() {
    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("No film");
    append_world(
        &mut document,
        Node::new(
            "Placement",
            NodeKind::State {
                operation: StateOperation::Transform(TransformOperation::Translate([1., 0., 0.])),
            },
        ),
    );
    append_world(&mut document, shape("sphere", vec![]));
    let store = ProjectStore::Folder(
        FolderStore::create(
            &temp.path().join("project"),
            document.id,
            &document.title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let (snapshot, store) = finish(store, document);

    let (text, program, _) = both_paths(&snapshot, &store, temp.path());
    let text_film = find(&text, "Film");
    let direct_film = find(&program.directives, "Film");
    assert_eq!(text_film.positional_tokens, vec!["\"rgb\""]);
    assert_eq!(text_film.positional_tokens, direct_film.positional_tokens);
    assert_eq!(text_film.positional, direct_film.positional);
    assert_eq!(
        text_film
            .parameters
            .iter()
            .map(|p| (p.value_type.as_str(), p.name.as_str()))
            .collect::<Vec<_>>(),
        vec![("string", "filename")]
    );
    assert_eq!(
        text_film.parameters[0].values,
        vec![Value::Text("render.exr".into())]
    );
    assert_eq!(text_film.parameters[0].value_tokens, vec!["\"render.exr\""]);
    assert_parameter_content_equal(&text_film.parameters[0], &direct_film.parameters[0]);

    // The injected Film is the first emitted directive, before WorldBegin.
    assert_eq!(text[0].name, "Film");
    assert_eq!(program.directives[0].name, "Film");
}

/// Asset values use the shared `assets/{id}-{basename}` staged name and
/// both paths emit that same string.
#[test]
fn asset_names_are_the_staged_asset_relative_paths() {
    let temp = tempfile::tempdir().unwrap();
    let asset_id = AssetId::new();
    let bytes = b"ply\ndummy bytes for the asset staging seam\n";
    let mut document = ProjectDocument::new("Asset");
    document.assets.insert(
        asset_id,
        Asset {
            id: asset_id,
            location: "Meshes/bunny (low).ply".into(),
            media_type: "application/octet-stream".into(),
            sha256: format!("{:x}", sha2::Sha256::digest(bytes)),
            size: bytes.len() as u64,
            origin: None,
        },
    );
    let asset = document.assets[&asset_id].clone();
    let mut folder = FolderStore::create(
        &temp.path().join("project"),
        document.id,
        &document.title,
        Revision::initial(),
    )
    .unwrap();
    folder.import_asset(&asset, &bytes[..], || false).unwrap();
    let store = ProjectStore::Folder(folder);
    append_world(
        &mut document,
        shape(
            "plymesh",
            vec![Parameter {
                value_type: "string".into(),
                name: "filename".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: ParameterValues::Assets(vec![asset_id]),
            }],
        ),
    );
    let (snapshot, store) = finish(store, document);

    let (text, program, assets) = both_paths(&snapshot, &store, temp.path());
    let staged = format!("assets/{asset_id}-bunny__low_.ply");
    assert_eq!(assets[&asset_id], staged);
    assert!(
        temp.path().join("direct").join(&staged).is_file(),
        "staged asset is missing"
    );
    let text_value = parameter(find(&text, "Shape"), "filename");
    let direct_value = parameter(find(&program.directives, "Shape"), "filename");
    assert_eq!(text_value.values, direct_value.values);
    assert_eq!(text_value.value_tokens, direct_value.value_tokens);
    assert_eq!(
        text_value.values,
        vec![Value::Text(staged)],
        "the asset value must be the staged relative path"
    );
}

/// A 100k-element array is emitted element-for-element, with the same
/// values and value tokens on both paths (all elements compared, not a hash).
#[test]
fn large_arrays_preserve_every_element_and_token() {
    let temp = tempfile::tempdir().unwrap();
    const COUNT: usize = 100_000;
    let mut bytes = Vec::with_capacity(COUNT * 8);
    let expected: Vec<f64> = (0..COUNT)
        .map(|index| (index as f64) * 0.25 - 1234.5)
        .collect();
    for value in &expected {
        bytes.extend_from_slice(&value.to_le_bytes());
    }
    let asset_id = AssetId::new();
    let mut document = ProjectDocument::new("Large array");
    document.assets.insert(
        asset_id,
        Asset {
            id: asset_id,
            location: "Data/weights.array".into(),
            media_type: "application/octet-stream".into(),
            sha256: format!("{:x}", sha2::Sha256::digest(&bytes)),
            size: bytes.len() as u64,
            origin: None,
        },
    );
    let asset = document.assets[&asset_id].clone();
    let mut folder = FolderStore::create(
        &temp.path().join("project"),
        document.id,
        &document.title,
        Revision::initial(),
    )
    .unwrap();
    folder.import_asset(&asset, &bytes[..], || false).unwrap();
    let store = ProjectStore::Folder(folder);
    append_world(
        &mut document,
        shape(
            "trianglemesh",
            vec![Parameter {
                value_type: "float".into(),
                name: "weights".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: ParameterValues::Array {
                    asset: asset_id,
                    encoding: ArrayEncoding::F64Le,
                    count: COUNT as u64,
                },
            }],
        ),
    );
    let (snapshot, store) = finish(store, document);

    let (text, program, _) = both_paths(&snapshot, &store, temp.path());
    let text_value = parameter(find(&text, "Shape"), "weights");
    let direct_value = parameter(find(&program.directives, "Shape"), "weights");
    assert_eq!(text_value.values.len(), COUNT);
    assert_eq!(direct_value.values.len(), COUNT);
    assert_eq!(text_value.value_tokens.len(), COUNT);
    assert_eq!(direct_value.value_tokens.len(), COUNT);
    for (index, (text, direct)) in text_value
        .values
        .iter()
        .zip(&direct_value.values)
        .enumerate()
    {
        assert_eq!(
            text.as_float().map(f64::to_bits),
            Some(expected[index].to_bits()),
            "value {index} differs"
        );
        assert_eq!(
            direct.as_float().map(f64::to_bits),
            Some(expected[index].to_bits()),
            "direct value {index} differs"
        );
        assert_eq!(
            text_value.value_tokens[index],
            direct_value.value_tokens[index]
        );
    }
}

/// A New-menu imagemap starts without an image. While nothing the export
/// writes refers to it (a node of another scene does not count), both paths
/// leave it out and report it as a warning; once this scene uses it, both
/// refuse it as an unresolved image naming the texture.
#[test]
fn an_imagemap_without_an_image_is_skipped_until_this_scene_uses_it() {
    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("Unset image");
    let id = project::ResourceId::new();
    document.resources.insert(
        id,
        project::Resource {
            id,
            label: "Pending".into(),
            location: "Textures/Pending.texture".into(),
            edition: Revision::initial(),
            kind: project::ResourceKind::Texture,
            texture_value_type: Some(project::TextureValueType::Float),
            implementation: "imagemap".into(),
            parameters: vec![],
            definition_scope: None,
            declared_in: None,
            origin: None,
        },
    );
    append_world(&mut document, shape("sphere", vec![]));
    let masked = || {
        shape(
            "sphere",
            vec![Parameter {
                name: "alpha".into(),
                value_type: "texture".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: project::ResourceKind::Texture,
                    ids: vec![id],
                },
            }],
        )
    };
    // A second scene uses the texture; this scene's export does not write it.
    let other = {
        let world = Node::new(
            "World",
            NodeKind::Scope {
                scope: project::ScopeKind::World,
                children: Vec::new(),
                owned_object: None,
                parent_camera: None,
            },
        );
        let mask = masked();
        let mut world = world;
        if let NodeKind::Scope { children, .. } = &mut world.data {
            children.push(mask.id);
        }
        project::SceneDocument {
            id: project::SceneId::new(),
            label: "Other".into(),
            location: "Scenes/Other.scene".into(),
            options: Vec::new(),
            world: world.id,
            render_camera: None,
            coordinate_system: Default::default(),
            nodes: BTreeMap::from([(world.id, world), (mask.id, mask)]),
        }
    };
    document.scenes.insert(other.id, std::sync::Arc::new(other));
    let (project_id, title) = (document.id, document.title.clone());
    let store = |name: &str| {
        ProjectStore::Folder(
            FolderStore::create(
                &temp.path().join(name),
                project_id,
                &title,
                Revision::initial(),
            )
            .unwrap(),
        )
    };
    let (snapshot, unused_store) = finish(store("unused"), document.clone());
    let (text, program, _) = both_paths(&snapshot, &unused_store, temp.path());
    assert!(
        !text.iter().any(|d| d.name == "Texture"),
        "an imagemap without an image this scene does not use is not exported"
    );
    assert!(!program.directives.iter().any(|d| d.name == "Texture"));
    let warnings = export_warnings(&snapshot).unwrap();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert_eq!(warnings[0].code, "imagemap_without_image_skipped");
    assert_eq!(warnings[0].resource, id);
    assert!(warnings[0].message.contains("Pending"), "{warnings:?}");

    append_world(&mut document, masked());
    let (snapshot, store) = finish(store("used-project"), document);
    for error in [
        export_warnings(&snapshot).unwrap_err(),
        export_project(&snapshot, &store, &temp.path().join("used"), || false).unwrap_err(),
        program_from_project(&snapshot, &store, &BTreeMap::new(), || false).unwrap_err(),
    ] {
        let message = error.to_string();
        assert!(
            matches!(&error, pbrt_io::Error::ImagemapWithoutImage { resource, .. } if *resource == id),
            "{error:?}"
        );
        assert!(
            message.contains("Pending") && message.contains("no image"),
            "{message}"
        );
    }
}
