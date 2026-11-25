//! Clearing a local binding must keep the project exportable and previewable
//! The clear deletes the binding state in the
//! dedicated group instead of pinning `Material(Default)` (which the emitter
//! rejects) or an empty `MediumInterface "" ""` that cancels an outer medium.
use pbrt_io::{export_project, program_from_project, stage_assets};
use project::*;
use scene_parse::{
    GeometryCache, ParseOptions, document::ReadOptions, document::read_document_with_options,
    preview_from_program,
};
use std::fs;

fn material_resource(document: &mut ProjectDocument, label: &str) -> ResourceId {
    let resource = Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Materials/{label}.material"),
        edition: Revision::initial(),
        kind: ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let id = resource.id;
    document.resources.insert(id, resource);
    id
}

fn medium_resource(document: &mut ProjectDocument) -> ResourceId {
    let resource = Resource {
        id: ResourceId::new(),
        label: "fog".into(),
        location: "Media/fog.medium".into(),
        edition: Revision::initial(),
        kind: ResourceKind::Medium,
        texture_value_type: None,
        implementation: "homogeneous".into(),
        parameters: Vec::new(),
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let id = resource.id;
    document.resources.insert(id, resource);
    id
}

#[test]
fn clearing_local_bindings_keeps_the_project_exportable_and_previewable() {
    let temp = tempfile::tempdir().unwrap();
    let mut document = ProjectDocument::new("Bindings");
    let outer = material_resource(&mut document, "outer");
    let inner = material_resource(&mut document, "inner");
    let fog = medium_resource(&mut document);
    let shape = Node::new(
        "Sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![Parameter::floats("radius", [1.])],
            pose: None,
        },
    );
    let shape_id = shape.id;
    let state = Node::new(
        "Outer material",
        NodeKind::State {
            operation: StateOperation::Material(MaterialBinding::Resource(outer)),
        },
    );
    let state_id = state.id;
    let world = document.default_scene_view().unwrap().scene.world;
    {
        let scene = std::sync::Arc::make_mut(
            document
                .scenes
                .get_mut(&document.default_scene.unwrap())
                .unwrap(),
        );
        scene.nodes.insert(shape_id, shape);
        scene.nodes.insert(state_id, state);
        let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
        else {
            unreachable!();
        };
        children.push(state_id);
        children.push(shape_id);
    }
    let store = ProjectStore::Folder(
        FolderStore::create(
            &temp.path().join("project"),
            document.id,
            &document.title,
            Revision::initial(),
        )
        .unwrap(),
    );
    let mut session = ProjectSession::new(document).unwrap();
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape_id,
                material: Some(MaterialBinding::Resource(inner)),
                medium: Some(MediumInterface {
                    inside: Some(fog),
                    outside: None,
                }),
            }],
        )
        .unwrap();
    // With the local binding in place, both the outer and the inner material
    // binding states appear in the world.
    let snapshot = session.snapshot();
    let path = export_project(&snapshot, &store, &temp.path().join("bound"), || false).unwrap();
    let exported = read_document_with_options(&path, ReadOptions::default()).unwrap();
    let count = |name: &str| {
        exported
            .directives
            .iter()
            .filter(|directive| directive.name == name)
            .count()
    };
    assert_eq!(count("NamedMaterial"), 2);
    assert_eq!(count("MediumInterface"), 1);
    // Clearing must not leave a Material(Default) behind: the export still
    // succeeds and the outer binding applies again.
    session
        .apply(
            &session.revision(),
            &[Edit::SetLocalBinding {
                node: shape_id,
                material: Some(MaterialBinding::Default),
                medium: Some(MediumInterface::default()),
            }],
        )
        .unwrap();
    let snapshot = session.snapshot();
    let path = export_project(&snapshot, &store, &temp.path().join("cleared"), || false)
        .unwrap_or_else(|error| panic!("cleared export failed: {error}"));
    let text = fs::read_to_string(&path).unwrap();
    assert!(!text.contains("MediumInterface \"\" \"\""));
    assert_eq!(
        evaluate(&snapshot.scene_view().unwrap()).unwrap().nodes[&shape_id].material,
        MaterialBinding::Resource(outer)
    );
    let exported = read_document_with_options(&path, ReadOptions::default()).unwrap();
    let count = |name: &str| {
        exported
            .directives
            .iter()
            .filter(|directive| directive.name == name)
            .count()
    };
    assert_eq!(count("NamedMaterial"), 1);
    assert_eq!(count("MediumInterface"), 0);
    // The cleared project also previews (the same emitter feeds direct preview).
    let bundle = temp.path().join("direct");
    fs::create_dir_all(&bundle).unwrap();
    let assets = stage_assets(snapshot.document(), &store, &bundle, &|| false).unwrap();
    let program = program_from_project(&snapshot, &store, &assets, || false).unwrap();
    let origins = program.origins.clone();
    let preview = preview_from_program(
        program,
        Some(&origins),
        &bundle,
        ParseOptions::default(),
        &mut GeometryCache::new(0),
    )
    .unwrap();
    assert!(
        preview.manifest.unsupported.is_empty(),
        "{:?}",
        preview.manifest.unsupported
    );
    assert_eq!(preview.manifest.draws.len(), 1);
}
