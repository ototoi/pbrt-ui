//! creating resources through an edit — placement
//! by the shared rules, the validation shapes of every kind, undo that
//! removes what creation added, and `created_items` reporting the new ids.
use project::*;

fn apply(session: &mut ProjectSession, edits: Vec<Edit>) -> ChangeSet {
    let revision = session.revision();
    session.apply(&revision, &edits).unwrap()
}

fn undo(session: &mut ProjectSession) -> ChangeSet {
    let revision = session.revision();
    session.undo(&revision).unwrap()
}

fn redo(session: &mut ProjectSession) -> ChangeSet {
    let revision = session.revision();
    session.redo(&revision).unwrap()
}

fn apply_error(session: &mut ProjectSession, edits: Vec<Edit>) -> String {
    let revision = session.revision();
    session.apply(&revision, &edits).unwrap_err().to_string()
}

fn material(label: &str, parameters: Vec<Parameter>) -> Edit {
    Edit::CreateResource {
        id: None,
        resource_kind: ResourceKind::Material,
        implementation: "diffuse".into(),
        label: label.into(),
        folder: None,
        texture_value_type: None,
        parameters,
    }
}

#[test]
fn created_resources_place_number_and_undo() {
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    let change = apply(
        &mut session,
        vec![material(
            "Copper",
            vec![Parameter::floats("reflectance", [0.5, 0.5, 0.5])],
        )],
    );
    let [ItemId::Resource(id)] = change.created_items[..] else {
        panic!(
            "creation must report the resource id: {:?}",
            change.created_items
        );
    };
    assert_eq!(change.resources, vec![id]);
    let document = session.document();
    let resource = &document.resources[&id];
    // rules: the kind's folder, sanitized label, matching extension.
    assert_eq!(resource.location, "Materials/Copper.material");
    assert_eq!(resource.label, "Copper");
    assert_eq!(resource.implementation, "diffuse");
    assert_eq!(
        resource.parameters,
        vec![Parameter::floats("reflectance", [0.5, 0.5, 0.5])]
    );
    document.validate().unwrap();

    // Same label: append `_N`, checking case-insensitively.
    let second = apply(&mut session, vec![material("copper", vec![])]);
    let [ItemId::Resource(second_id)] = second.created_items[..] else {
        panic!("second creation must report its id");
    };
    assert_eq!(
        session.document().resources[&second_id].location,
        "Materials/copper_1.material"
    );

    // Undo in creation order removes them (and the numbering rewinds).
    for id in [second_id, id] {
        let undone = undo(&mut session);
        assert_eq!(
            undone.resources,
            vec![id],
            "undo reports the removed resource"
        );
    }
    assert!(session.document().resources.is_empty());
    // Redo brings the first one back under its original id.
    let redone = redo(&mut session);
    assert_eq!(
        redone.created_items,
        vec![ItemId::Resource(id)],
        "redo re-reports the creation"
    );
    assert_eq!(
        redone.created_items,
        vec![ItemId::Resource(id)],
        "redo re-reports the creation"
    );
    assert_eq!(
        session.document().resources[&id].location,
        "Materials/Copper.material"
    );
}

#[test]
fn textures_pick_a_value_type_others_reject_one() {
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    let created = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Texture,
            implementation: "constant".into(),
            label: "Gloss".into(),
            folder: None,
            texture_value_type: Some(TextureValueType::Spectrum),
            parameters: vec![],
        }],
    );
    let [ItemId::Resource(id)] = created.created_items[..] else {
        panic!("texture creation must report its id");
    };
    let texture = &session.document().resources[&id];
    assert_eq!(texture.location, "Textures/Gloss.texture");
    assert_eq!(texture.texture_value_type, Some(TextureValueType::Spectrum));
    // A texture without a value type is invalid; a medium with one too.
    let error = apply_error(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Texture,
            implementation: "constant".into(),
            label: "No type".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![],
        }],
    );
    assert!(error.contains("texture value type"), "{error}");
    let error = apply_error(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Medium,
            implementation: "homogeneous".into(),
            label: "Fog".into(),
            folder: None,
            texture_value_type: Some(TextureValueType::Float),
            parameters: vec![],
        }],
    );
    assert!(error.contains("texture value type"), "{error}");
}

#[test]
fn object_definitions_reject_everything_but_a_label() {
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    let base = |implementation: &str, texture_value_type, parameters| Edit::CreateResource {
        id: None,
        resource_kind: ResourceKind::ObjectDefinition,
        implementation: implementation.into(),
        label: "Bush".into(),
        folder: None,
        texture_value_type,
        parameters,
    };
    // A value type is for textures only.
    let error = apply_error(
        &mut session,
        vec![base("", Some(TextureValueType::Float), Vec::new())],
    );
    assert!(error.contains("texture value type"), "{error}");
    // An implementation or parameters do not belong on a definition.
    let error = apply_error(&mut session, vec![base("diffuse", None, Vec::new())]);
    assert!(error.contains("implementation"), "{error}");
    let error = apply_error(
        &mut session,
        vec![base(
            "",
            None,
            vec![Parameter::floats("reflectance", [0.5, 0.5, 0.5])],
        )],
    );
    assert!(error.contains("parameter"), "{error}");
    // The bare shape still applies.
    apply(&mut session, vec![base("", None, Vec::new())]);
}

#[test]
fn object_definitions_come_with_an_empty_scope() {
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    let created = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::ObjectDefinition,
            implementation: String::new(),
            label: "Bush".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![],
        }],
    );
    let [ItemId::Resource(id)] = created.created_items[..] else {
        panic!("object definition creation must report its id");
    };
    let document = session.document();
    let resource = &document.resources[&id];
    assert_eq!(resource.location, "Objects/Bush.object");
    assert!(resource.implementation.is_empty());
    assert!(resource.parameters.is_empty());
    let scope_id = resource.definition_scope.expect("scope link");
    let NodeKind::Scope {
        scope: ScopeKind::ObjectDefinition,
        children,
        ..
    } = &document.definition_nodes[&scope_id].data
    else {
        panic!("creation must add an ObjectDefinition scope");
    };
    assert!(children.is_empty());
    document.validate().unwrap();
    // Undo takes the scope with the resource.
    undo(&mut session);
    let document = session.document();
    assert!(document.resources.is_empty());
    assert!(!document.definition_nodes.contains_key(&scope_id));
}

#[test]
fn folders_must_exist_and_default_by_kind() {
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    let created = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Medium,
            implementation: "homogeneous".into(),
            label: "Fog".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![],
        }],
    );
    let [ItemId::Resource(id)] = created.created_items[..] else {
        panic!("medium creation must report its id");
    };
    assert_eq!(
        session.document().resources[&id].location,
        "Media/Fog.medium"
    );
    // Explicit folders work when they exist...
    apply(
        &mut session,
        vec![
            Edit::CreateFolder {
                path: "Media/Thick".into(),
            },
            Edit::CreateResource {
                id: None,
                resource_kind: ResourceKind::Medium,
                implementation: "homogeneous".into(),
                label: "Haze".into(),
                folder: Some("Media/Thick".into()),
                texture_value_type: None,
                parameters: vec![],
            },
        ],
    );
    //...and fail the edit when they do not.
    let error = apply_error(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Material,
            implementation: "diffuse".into(),
            label: "Lost".into(),
            folder: Some("Nowhere".into()),
            texture_value_type: None,
            parameters: vec![],
        }],
    );
    assert!(error.contains("folder"), "{error}");
    session.document().validate().unwrap();
}

#[test]
fn created_resources_export_through_their_defaults() {
    // The parameters supplied by a creator must keep the item exportable.
    // Empty materials were already supported; creation writes the table's
    // constants.
    let mut session = ProjectSession::new(ProjectDocument::new("G7")).unwrap();
    apply(
        &mut session,
        vec![material(
            "Diffuse",
            vec![Parameter::floats("reflectance", [0.5, 0.5, 0.5])],
        )],
    );
    let document = session.document().clone();
    document.validate().unwrap();
}
