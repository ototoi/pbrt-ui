//! Spectrum Data resources: a typed
//! `wavelengths`/`values` sample table stored as `Spectra/<name>.spectrum`,
//! validated like r4's piecewise-linear spectrum, referenced through
//! `spectrum` parameters and undoable like any other resource.
mod common;
use common::*;
use project::*;

/// A well-formed SpectrumData resource: r4 `PiecewiseLinear` semantics.
fn spectrum(label: &str, wavelengths: &[f64], values: &[f64]) -> Resource {
    Resource {
        id: ResourceId::new(),
        label: label.into(),
        location: format!("Spectra/{label}.spectrum"),
        edition: Revision::initial(),
        kind: ResourceKind::SpectrumData,
        texture_value_type: None,
        implementation: "piecewise".into(),
        parameters: vec![
            Parameter::floats("wavelengths", wavelengths.iter().copied()),
            Parameter::floats("values", values.iter().copied()),
        ],
        definition_scope: None,
        declared_in: None,
        origin: None,
    }
}

fn apply(session: &mut ProjectSession, edits: Vec<Edit>) -> ChangeSet {
    let revision = session.revision();
    session.apply(&revision, &edits).unwrap()
}

fn apply_error(session: &mut ProjectSession, edits: Vec<Edit>) -> String {
    let revision = session.revision();
    session.apply(&revision, &edits).unwrap_err().to_string()
}

fn undo(session: &mut ProjectSession) -> ChangeSet {
    let revision = session.revision();
    session.undo(&revision).unwrap()
}

#[test]
fn spectrum_data_resources_round_trip_through_the_folder_store() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("project");
    let mut document = ProjectDocument::new("Spectrum store");
    let resource = spectrum(
        "Au.k",
        &[298.757050, 302.400421, 306.133759],
        &[1.920375, 1.92, 1.918875],
    );
    let expected_id = resource.id;
    document.resources.insert(resource.id, resource);
    let session = ProjectSession::new(document).unwrap();
    let snapshot = session.snapshot();
    let mut store = FolderStore::create_v3(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    store.save(&snapshot).unwrap();
    drop(store);
    drop(session);

    let (_, reopened) = ProjectStore::open(&path).unwrap();
    assert_eq!(reopened.document(), snapshot.document());
    let resource = &reopened.document().resources[&expected_id];
    assert_eq!(resource.kind, ResourceKind::SpectrumData);
    assert_eq!(resource.implementation, "piecewise");
    assert_eq!(
        resource.parameters,
        vec![
            Parameter::floats("wavelengths", [298.757050, 302.400421, 306.133759]),
            Parameter::floats("values", [1.920375, 1.92, 1.918875]),
        ]
    );
    assert!(
        path.join("Assets").join("Spectra/Au.k.spectrum").is_file(),
        "the resource is stored as Spectra/Au.k.spectrum"
    );
}

#[test]
fn spectrum_data_validation_rejects_broken_samples() {
    fn invalid(
        wavelengths: Option<Vec<f64>>,
        values: Option<Vec<f64>>,
        mutate: impl FnOnce(&mut Resource),
    ) -> String {
        let mut document = ProjectDocument::new("Spectrum validation");
        let mut resource = spectrum("Au.k", &[400., 500., 600.], &[1., 2., 3.]);
        if let Some(wavelengths) = wavelengths {
            resource.parameters[0] = Parameter::floats("wavelengths", wavelengths);
        }
        if let Some(values) = values {
            resource.parameters[1] = Parameter::floats("values", values);
        }
        mutate(&mut resource);
        document.resources.insert(resource.id, resource);
        document.validate().unwrap_err().to_string()
    }

    // A well-formed curve passes.
    let mut document = ProjectDocument::new("Spectrum validation");
    let resource = spectrum("Au.k", &[400., 500., 600.], &[1., 2., 3.]);
    document.resources.insert(resource.id, resource);
    document.validate().unwrap();

    assert!(
        invalid(Some(vec![500., 400., 600.]), None, |_| ()).contains("ascending"),
        "descending wavelengths are rejected"
    );
    assert!(
        invalid(Some(vec![0., 500., 600.]), None, |_| ()).contains("positive"),
        "zero wavelengths are rejected"
    );
    assert!(
        invalid(Some(vec![400.]), Some(vec![1., 2.]), |_| ()).contains("counts"),
        "sample count mismatches are rejected"
    );
    assert!(
        invalid(None, Some(vec![]), |_| ()).contains("counts"),
        "an empty value table is rejected"
    );
    assert!(
        invalid(None, None, |resource| {
            resource.parameters[1] = Parameter {
                value_type: "float".into(),
                name: "values".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Floats(vec![
                    FloatValue::Finite(1.),
                    FloatValue::Nan,
                    FloatValue::Finite(3.),
                ]),
            };
        })
        .contains("finite"),
        "non-finite values are rejected"
    );
    assert!(
        invalid(Some(vec![]), Some(vec![]), |_| ()).contains("no samples"),
        "an empty spectrum is rejected"
    );
    assert!(
        invalid(None, None, |resource| resource.implementation =
            "linear".into())
        .contains("piecewise"),
        "the implementation is fixed"
    );
    assert!(
        invalid(None, None, |resource| {
            resource.parameters.pop();
        })
        .contains("wavelengths and values"),
        "a missing table side is rejected"
    );
    assert!(
        invalid(None, None, |resource| {
            resource.parameters.push(Parameter::floats("scale", [1.]));
        })
        .contains("wavelengths and values"),
        "unknown parameters are rejected"
    );
    assert!(
        !invalid(None, None, |resource| {
            resource.parameters[0] = Parameter {
                value_type: "float".into(),
                name: "wavelengths".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Integers(vec![400, 500, 600]),
            };
        })
        .is_empty(),
        "integer samples are rejected"
    );
    // The sample limit: MAX_SPECTRUM_SAMPLES is the last accepted count.
    let count = MAX_SPECTRUM_SAMPLES + 1;
    let wavelengths = (0..count).map(|n| 400. + n as f64).collect::<Vec<_>>();
    assert!(
        invalid(Some(wavelengths.clone()), Some(vec![1.; count]), |_| ()).contains("limit"),
        "oversized sample tables are rejected"
    );
    let mut document = ProjectDocument::new("Spectrum validation");
    let within_limit = (0..MAX_SPECTRUM_SAMPLES)
        .map(|n| 400. + n as f64)
        .collect::<Vec<_>>();
    let resource = spectrum("Full", &within_limit, &vec![1.; MAX_SPECTRUM_SAMPLES]);
    document.resources.insert(resource.id, resource);
    document.validate().unwrap();
}

#[test]
fn spectrum_data_creation_places_and_undoes() {
    let mut session = ProjectSession::new(ProjectDocument::new("Create")).unwrap();
    let change = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::SpectrumData,
            implementation: "piecewise".into(),
            label: "Au k".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![
                Parameter::floats("wavelengths", [400., 500.]),
                Parameter::floats("values", [1.9, 2.]),
            ],
        }],
    );
    let [ItemId::Resource(id)] = change.created_items[..] else {
        panic!("creation must report the resource id");
    };
    let resource = &session.document().resources[&id];
    assert_eq!(resource.location, "Spectra/Au k.spectrum");
    assert_eq!(resource.kind, ResourceKind::SpectrumData);
    session.document().validate().unwrap();

    let second = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::SpectrumData,
            implementation: "piecewise".into(),
            label: "au K".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![
                Parameter::floats("wavelengths", [400., 500.]),
                Parameter::floats("values", [1.9, 2.]),
            ],
        }],
    );
    let [ItemId::Resource(second_id)] = second.created_items[..] else {
        panic!("second creation must report its id");
    };
    assert_eq!(
        session.document().resources[&second_id].location,
        "Spectra/au K_1.spectrum",
        "numbering numbers case-insensitively"
    );

    undo(&mut session);
    assert!(!session.document().resources.contains_key(&second_id));
    undo(&mut session);
    assert!(!session.document().resources.contains_key(&id));
    let revision = session.revision();
    let redone = session.redo(&revision).unwrap();
    assert_eq!(
        redone.created_items,
        vec![ItemId::Resource(id)],
        "redo re-reports the creation"
    );
    assert_eq!(
        session.document().resources[&id].location,
        "Spectra/Au k.spectrum"
    );
}

#[test]
fn spectrum_data_references_report_through_parameters_and_block_deletion() {
    let (document, _, shape) = native_scene();
    let mut session = ProjectSession::new(document).unwrap();
    let change = apply(
        &mut session,
        vec![Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::SpectrumData,
            implementation: "piecewise".into(),
            label: "Au k".into(),
            folder: None,
            texture_value_type: None,
            parameters: vec![
                Parameter::floats("wavelengths", [400., 500.]),
                Parameter::floats("values", [1.9, 2.]),
            ],
        }],
    );
    let [ItemId::Resource(id)] = change.created_items[..] else {
        panic!("creation must report the resource id");
    };

    apply(
        &mut session,
        vec![Edit::SetParameter {
            target: ParameterTarget::Node(shape),
            name: "k".into(),
            value_type: "spectrum".into(),
            value: Some(Parameter {
                value_type: "spectrum".into(),
                name: "k".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Resources {
                    kind: ResourceKind::SpectrumData,
                    ids: vec![id],
                },
            }),
        }],
    );
    let document = session.document();
    let index = document.reference_index();
    let references = index.references(ItemId::Resource(id));
    let [location] = references else {
        panic!("the parameter must reference the resource: {references:?}")
    };
    assert_eq!(location.node, Some(shape));
    assert_eq!(location.via.wire(), "parameter:k");
    // Deletion is refused while the reference stands, then works after the
    // parameter is undone away.
    let error = apply_error(
        &mut session,
        vec![Edit::DeleteItem {
            item: ItemId::Resource(id),
        }],
    );
    assert!(error.contains("referenced"), "{error}");
    undo(&mut session);
    assert_eq!(
        session
            .document()
            .reference_index()
            .count(ItemId::Resource(id)),
        0
    );
    apply(
        &mut session,
        vec![Edit::DeleteItem {
            item: ItemId::Resource(id),
        }],
    );
    assert!(!session.document().resources.contains_key(&id));
    // Undo brings the resource back; the parameter stayed undone, so the
    // deletion leaving no reference behind is what history records.
    undo(&mut session);
    let document = session.document();
    assert!(document.resources.contains_key(&id));
    assert_eq!(
        document.reference_index().count(ItemId::Resource(id)),
        0,
        "undo restores the deleted resource; its undone parameter stays undone"
    );
    assert!(
        parameters(document, shape).iter().all(|p| p.name != "k"),
        "the undone parameter is not resurrected by the deletion's undo"
    );
}
