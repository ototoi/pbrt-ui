//! Legacy spd Asset references convert to SpectrumData resources on open
//! Referenced `Spectra/*.spd` assets become
//! shared resources, saving prunes the now-unreferenced files, unreadable
//! input stays behind with a warning, and reopening is stable. The
//! project's format stays 2: this is a read-time transform, not a format.
use pbrt_io::import_project;
use project::*;
use std::fs;

/// Build the pre-migration shape the only way it existed: a project whose
/// spd files were imported as `Spectra/<stem>.spd` text/plain assets (here
/// through a `string filename` dependency, which import still stages that
/// way), with conductor materials referencing them through `spectrum k`
/// Asset parameters. One material per entry of `requests`; the same file
/// twice means the same asset referenced by two materials. `format` picks
/// the folder format the legacy project lands in — imports publish v3
/// today, but deployed legacy projects are format 2 as well.
enum Format {
    Two,
    Three,
}

/// The fixture writes the edited session through whichever store the
/// format's own flow allows: v3 through `ProjectStore::save`, v2 through
/// the folder store it created (folders only turn read-only wrapped in
/// `ProjectStore`).
enum StoreHandle {
    Project(project::ProjectStore),
    Folder(project::FolderStore),
}

impl StoreHandle {
    fn save(&mut self, snapshot: &ProjectSnapshot) {
        match self {
            StoreHandle::Project(store) => store.save(snapshot).unwrap(),
            StoreHandle::Folder(store) => store.save(snapshot).unwrap(),
        };
    }
}

fn legacy_project(home: &tempfile::TempDir, requests: &[(&str, &str)]) -> std::path::PathBuf {
    legacy_project_in_format(home, requests, Format::Three)
}

fn legacy_project_in_format(
    home: &tempfile::TempDir,
    requests: &[(&str, &str)],
    format: Format,
) -> std::path::PathBuf {
    let mut distinct = Vec::<(&str, &str)>::new();
    for (name, text) in requests {
        if !distinct.iter().any(|(known, _)| known == name) {
            distinct.push((name, text));
        }
    }
    for (name, text) in &distinct {
        fs::write(home.path().join(name), text).unwrap();
    }
    let mut scene = String::from("WorldBegin\n");
    for (name, _) in &distinct {
        scene.push_str(&format!(
            "Shape \"plymesh\" \"string filename\" [\"{name}\"]\n"
        ));
    }
    fs::write(home.path().join("scene.pbrt"), scene).unwrap();
    let project_path = home.path().join("project");
    let (mut store, mut session) = match format {
        Format::Three => {
            let (store, session) = import_project(
                &home.path().join("scene.pbrt"),
                &project_path,
                &Default::default(),
                || false,
            )
            .unwrap();
            (StoreHandle::Project(store), session)
        }
        Format::Two => {
            // The pre-v3 publisher: an empty format-2 folder store filled
            // directly by the importer. A v2 folder is read-only to
            // ProjectStore::save — its edits persist by publishing as v3 —
            // but FolderStore itself writes the initial project.
            let mut folder = project::FolderStore::create(
                &project_path,
                ProjectId::new(),
                "Legacy gold",
                Revision::initial(),
            )
            .unwrap();
            let (session, _) = pbrt_io::import_into(
                &home.path().join("scene.pbrt"),
                &mut folder,
                &Default::default(),
                || false,
            )
            .unwrap();
            (StoreHandle::Folder(folder), session)
        }
    };
    let mut material_index = 0;
    let mut edits = Vec::new();
    // The plymesh was only the delivery vehicle: with the spectrum
    // parameters about to own the files, its references go away.
    for scene in session.document().scenes.values() {
        for (id, node) in &scene.nodes {
            if matches!(
                &node.data,
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    implementation,
                    ..
                } if implementation == "plymesh"
            ) {
                edits.push(Edit::DeleteNode { node: *id });
            }
        }
    }
    for (name, _) in requests {
        material_index += 1;
        let location = format!("Spectra/{name}");
        let asset = session
            .document()
            .assets
            .values()
            .find(|asset| asset.location == location)
            .unwrap_or_else(|| panic!("missing {location}"))
            .id;
        edits.push(Edit::CreateResource {
            id: None,
            resource_kind: ResourceKind::Material,
            implementation: "conductor".into(),
            label: format!("gold {material_index}"),
            folder: None,
            texture_value_type: None,
            parameters: vec![Parameter {
                value_type: "spectrum".into(),
                name: "k".into(),
                color_space: ColorSpaceBinding::Inherit,
                values: ParameterValues::Assets(vec![asset]),
            }],
        });
    }
    let revision = session.revision();
    session.apply(&revision, &edits).unwrap();
    store.save(&session.snapshot());
    drop(store);
    drop(session);
    project_path
}

fn samples(resource: &Resource) -> (Vec<f64>, Vec<f64>) {
    let inline = |name: &str| {
        resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .values
            .clone()
    };
    let numbers = |values: ParameterValues| {
        let ParameterValues::Floats(values) = values else {
            panic!("expected inline floats");
        };
        values
            .into_iter()
            .map(|value| match value {
                FloatValue::Finite(n) => n,
                other => panic!("non-finite {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    (numbers(inline("wavelengths")), numbers(inline("values")))
}

const AU_K: &str = "298.757050 1.920375\n302.400421 1.920000\n306.133759 1.918875\n";
const AU_ETA: &str = "298.757050 1.068421\n302.400421 1.070088\n306.133759 1.072401\n";

#[test]
fn open_converts_referenced_spd_assets_and_save_prunes_them() {
    let home = tempfile::tempdir().unwrap();
    // Two materials share Au.k.spd; Au.eta.spd is referenced once.
    let project_path = legacy_project(
        &home,
        &[
            ("Au.k.spd", AU_K),
            ("Au.k.spd", AU_K),
            ("Au.eta.spd", AU_ETA),
        ],
    );

    // The fixture imports as format 3 (imports publish v3); the migration
    // is a read-time transform, so the format on disk cannot move.
    let manifest_before: serde_json::Value =
        serde_json::from_slice(&fs::read(project_path.join("project.pbrtui.json")).unwrap())
            .unwrap();

    // Open: the references become SpectrumData resources; in-memory only —
    // the disk still holds the old shape until the user saves.
    let (mut store, session) = ProjectStore::open(&project_path).unwrap();
    let document = session.document();
    let spectra: Vec<&Resource> = document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::SpectrumData)
        .collect();
    assert_eq!(
        spectra.len(),
        2,
        "one resource per distinct file: {spectra:?}"
    );
    let k = spectra
        .iter()
        .copied()
        .find(|resource| resource.label == "Au.k")
        .expect("label from the file stem");
    let eta = spectra
        .iter()
        .copied()
        .find(|resource| resource.label == "Au.eta")
        .expect("the eta file also becomes a resource");
    assert_eq!(k.location, "Spectra/Au.k.spectrum");
    assert_eq!(eta.location, "Spectra/Au.eta.spectrum");
    assert_eq!(k.implementation, "piecewise");
    let (wavelengths, values) = samples(k);
    assert_eq!(wavelengths, [298.757050, 302.400421, 306.133759]);
    assert_eq!(values, [1.920375, 1.920000, 1.918875]);
    let mut referencing = Vec::new();
    for resource in document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::Material)
    {
        let ParameterValues::Resources {
            kind: ResourceKind::SpectrumData,
            ids,
        } = &resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == "k")
            .expect("the parameter survives")
            .values
        else {
            panic!("material {} was not converted", resource.label);
        };
        referencing.push(ids[0]);
    }
    referencing.sort_by_key(|id| (*id == eta.id) as u8);
    assert_eq!(
        referencing,
        vec![k.id, k.id, eta.id],
        "both Au.k materials share ONE resource; Au.eta has its own"
    );
    assert!(
        document.assets.is_empty(),
        "the converted assets left the document"
    );
    assert!(project_path.join("Assets/Spectra/Au.k.spd").is_file());

    // Save: the spd assets' files and metas are pruned; the format stays 2.
    store.save(&session.snapshot()).unwrap();
    assert!(!project_path.join("Assets/Spectra/Au.k.spd").exists());
    assert!(!project_path.join("Assets/Spectra/Au.k.spd.meta").is_file());
    assert!(project_path.join("Assets/Spectra/Au.k.spectrum").is_file());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(project_path.join("project.pbrtui.json")).unwrap())
            .unwrap();
    assert_eq!(
        manifest["format"], manifest_before["format"],
        "the migration is a read-time transform, not a format change"
    );
    drop(store);
    drop(session);

    // Reopen: what was converted stays converted, with the same samples.
    let (store, reopened) = ProjectStore::open(&project_path).unwrap();
    assert_eq!(
        reopened
            .document()
            .resources
            .values()
            .filter(|resource| resource.kind == ResourceKind::SpectrumData)
            .count(),
        2
    );
    assert!(reopened.document().assets.is_empty());
    let reopened_k = reopened
        .document()
        .resources
        .values()
        .find(|resource| {
            resource.kind == ResourceKind::SpectrumData
                && resource.location == "Spectra/Au.k.spectrum"
        })
        .unwrap();
    assert_eq!(samples(reopened_k).0, wavelengths);
    drop(store);
    drop(reopened);
    // Opening the pruned folder again is the same operation (idempotent).
    let (_, again) = ProjectStore::open(&project_path).unwrap();
    assert_eq!(
        again
            .document()
            .resources
            .values()
            .filter(|resource| resource.kind == ResourceKind::SpectrumData)
            .count(),
        2
    );
    assert!(
        again
            .document()
            .assets
            .values()
            .all(|asset| !asset.location.ends_with(".spd"))
    );
}

#[test]
fn unreadable_spd_assets_stay_as_assets_with_a_diagnostic() {
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project(
        &home,
        &[
            ("junk.spd", "400 abc\n"),
            ("empty.spd", "# a comment only\n"),
            ("Au.k.spd", AU_K),
        ],
    );
    let (mut store, session) = ProjectStore::open(&project_path).unwrap();
    let document = session.document();
    // The readable file converted; the broken ones kept their references —
    // with the parameter type they always had — and say why on their import.
    let spectra = document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::SpectrumData)
        .collect::<Vec<_>>();
    assert_eq!(spectra.len(), 1, "only Au.k converts: {spectra:?}");
    assert_eq!(spectra[0].label, "Au.k");
    let mut kept = document
        .assets
        .values()
        .map(|asset| asset.location.as_str())
        .collect::<Vec<_>>();
    kept.sort();
    assert_eq!(
        kept,
        vec!["Spectra/empty.spd", "Spectra/junk.spd"],
        "each broken file keeps its asset record"
    );
    for material in document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::Material)
    {
        let parameter = material
            .parameters
            .iter()
            .find(|parameter| parameter.name == "k")
            .unwrap();
        match &parameter.values {
            ParameterValues::Assets(_)
                if material.label == "gold 1" || material.label == "gold 2" => {}
            ParameterValues::Resources { .. } if material.label == "gold 3" => {}
            other => panic!("material {}: {other:?}", material.label),
        }
    }
    let diagnostics = document
        .imports
        .values()
        .flat_map(|import| import.diagnostics.clone())
        .collect::<Vec<_>>();
    for (name, reason) in [("junk.spd", "non-numeric"), ("empty.spd", "no samples")] {
        assert!(
            diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains(name)
                    && diagnostic.message.contains(reason)),
            "the unreadable spd says why it stayed: {diagnostics:?}"
        );
    }
    // Saving keeps the kept files in the project.
    store.save(&session.snapshot()).unwrap();
    assert!(project_path.join("Assets/Spectra/junk.spd").is_file());
    assert!(project_path.join("Assets/Spectra/empty.spd").is_file());
    assert!(!project_path.join("Assets/Spectra/Au.k.spd").exists());
    drop(store);
    drop(session);
    // And reopening does not duplicate the warnings.
    let (_, again) = ProjectStore::open(&project_path).unwrap();
    let diagnostics = again
        .document()
        .imports
        .values()
        .flat_map(|import| import.diagnostics.clone())
        .filter(|diagnostic| diagnostic.message.contains(".spd:"))
        .collect::<Vec<_>>();
    assert_eq!(
        diagnostics.len(),
        2,
        "one warning per file after reopen: {diagnostics:?}"
    );
}

#[test]
fn format_two_legacy_projects_convert_the_same_way() {
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project_in_format(
        &home,
        &[
            ("Au.k.spd", AU_K),
            ("Au.k.spd", AU_K),
            ("Au.eta.spd", AU_ETA),
        ],
        Format::Two,
    );
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(project_path.join("project.pbrtui.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["format"], "pbrt-ui.project/2");

    // Same contract on the older format: one shared resource per distinct
    // file, the converted assets gone from the document. A v2 folder is
    // read-only wrapped in ProjectStore, so saving publishes as v3 —
    // the migration's own work persists through that, pruning the spd
    // files, and the reopened result is stable.
    let (store, session) = ProjectStore::open(&project_path).unwrap();
    assert!(
        store.is_read_only(),
        "a format-2 folder stays read-only until published as v3"
    );
    let document = session.document();
    let spectra: Vec<&Resource> = document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::SpectrumData)
        .collect();
    assert_eq!(spectra.len(), 2, "{:?}", spectra);
    assert!(document.assets.is_empty());
    assert!(project_path.join("Assets/Spectra/Au.k.spd").is_file());
    let destination = home.path().join("published");
    let (published, _) = store
        .save_as_format_three(&destination, &session.snapshot(), || false)
        .unwrap();
    drop(published);
    drop(store);
    drop(session);
    assert!(!destination.join("Assets/Spectra/Au.k.spd").exists());
    assert!(destination.join("Assets/Spectra/Au.k.spectrum").is_file());
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(destination.join("project.pbrtui.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["format"], "pbrt-ui.project/3");
    let (_, reopened) = ProjectStore::open(&destination).unwrap();
    assert_eq!(
        reopened
            .document()
            .resources
            .values()
            .filter(|resource| resource.kind == ResourceKind::SpectrumData)
            .count(),
        2
    );
    assert!(reopened.document().assets.is_empty());
}

#[test]
fn tampered_spd_disk_bytes_never_reach_the_migration_as_truth() {
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project_in_format(&home, &[("Au.k.spd", AU_K)], Format::Three);
    let spd = project_path.join("Assets/Spectra/Au.k.spd");
    fs::write(&spd, "999 4.5\n").unwrap();
    // At rest, tamper detection is the format's own provenance check, not
    // the migration's: a v3 project refuses the whole open when bytes no
    // longer match the sidecar's recorded hash — exactly as for any other
    // asset kind — so the migration never even partially runs.
    let error = match ProjectStore::open(&project_path) {
        Ok(_) => panic!("tampered spd bytes must refuse the v3 open"),
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("asset bytes do not match sidecar"),
        "{error}"
    );
    assert_eq!(fs::read(&spd).unwrap(), b"999 4.5\n");
    assert!(!project_path.join("Assets/Spectra/Au.k.spectrum").is_file());

    // Format 2 has no tamper story at all — its asset records are built
    // from the bytes it finds on disk — so the conversion follows the
    // bytes that are there, like every other v2 read, and publishing as
    // v3 records them honestly under the hash it just computed.
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project_in_format(&home, &[("Au.k.spd", AU_K)], Format::Two);
    fs::write(project_path.join("Assets/Spectra/Au.k.spd"), "999 4.5\n").unwrap();
    let (store, session) = ProjectStore::open(&project_path).unwrap();
    let spectrum = session
        .document()
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::SpectrumData)
        .unwrap();
    let (wavelengths, values) = samples(spectrum);
    assert_eq!(wavelengths, [999.0]);
    assert_eq!(values, [4.5]);
    let destination = home.path().join("published");
    let (published, _) = store
        .save_as_format_three(&destination, &session.snapshot(), || false)
        .unwrap();
    drop(published);
    drop(store);
    drop(session);
    assert!(!destination.join("Assets/Spectra/Au.k.spd").exists());
    let (_, reopened) = ProjectStore::open(&destination).unwrap();
    let spectrum = reopened
        .document()
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::SpectrumData)
        .unwrap();
    assert_eq!(samples(spectrum).0, [999.0]);
}

/// Write a second, minimal, valid import record by hand: multi-import
/// legacy projects are allowed, and their unreadable spd files stay Assets
/// without a warning (the single-import home does not apply) — while the
/// readable ones still convert.
fn add_second_import(project_path: &std::path::Path) {
    let import_id = ImportId::new().0;
    let source = SourceId::new().0;
    let root = OccurrenceId::new().0;
    let record = serde_json::json!({
        "format": "pbrt-ui.import/1",
        "id": import_id,
        "importer_version": "hand-written second import",
        "dialect": "pbrt_r4",
        "renderer_revision": null,
        "diagnostics": [],
        "root": root,
        "sources": {
            source.to_string(): {
                "original_path": "scene.pbrt",
                "original": {"sha256": format!("{:064x}", 0), "size": 10},
                "expanded": null,
            },
        },
        "occurrences": {
            root.to_string(): {
                "source": source.to_string(),
                "parent": null,
                "callsite": null,
            },
        },
    });
    let dir = project_path.join("Imports").join(import_id.to_string());
    fs::create_dir_all(&dir).unwrap();
    fs::write(
        dir.join("import.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
}

#[test]
fn multi_import_projects_still_convert_and_only_drop_the_warning() {
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project(&home, &[("junk.spd", "400 abc\n"), ("Au.k.spd", AU_K)]);
    add_second_import(&project_path);
    let (mut store, session) = ProjectStore::open(&project_path).unwrap();
    let document = session.document();
    assert_eq!(
        document
            .resources
            .values()
            .filter(|resource| resource.kind == ResourceKind::SpectrumData)
            .count(),
        1,
        "the readable file still converts"
    );
    assert_eq!(
        document
            .assets
            .values()
            .map(|asset| asset.location.as_str())
            .collect::<Vec<_>>(),
        vec!["Spectra/junk.spd"],
        "the broken file keeps its asset record"
    );
    assert!(
        document
            .imports
            .values()
            .flat_map(|import| import.diagnostics.clone())
            .all(|diagnostic| !diagnostic.message.contains(".spd:")),
        "with no single import to own the warning, none is written"
    );
    store.save(&session.snapshot()).unwrap();
    drop(store);
    drop(session);
    let (store, reopened) = ProjectStore::open(&project_path).unwrap();
    assert!(
        reopened
            .document()
            .imports
            .values()
            .flat_map(|import| import.diagnostics.clone())
            .all(|diagnostic| !diagnostic.message.contains(".spd:")),
        "reopen stays quiet"
    );
    drop(store);
    drop(reopened);
}

#[test]
fn an_all_broken_multi_import_project_bumps_no_revision() {
    let home = tempfile::tempdir().unwrap();
    let project_path = legacy_project(&home, &[("junk.spd", "400 abc\n")]);
    let saved: serde_json::Value =
        serde_json::from_slice(&fs::read(project_path.join("project.pbrtui.json")).unwrap())
            .unwrap();
    add_second_import(&project_path);
    let (_, session) = ProjectStore::open(&project_path).unwrap();
    assert_eq!(
        saved["saved_revision"],
        session.revision().0,
        "nothing parsed, nothing warned: the migration changed nothing"
    );
}
