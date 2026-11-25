//! spd file spectrum parameters import as shared SpectrumData resources
//! one resource per distinct file within an
//! import, `spectrum` parameters referencing them, malformed or oversized
//! files skipping the declaration with a diagnostic — the importer's
//! existing skip-and-warn policy.
use pbrt_io::import_project_with_report;
use project::*;
use std::{fs, path::Path};

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/spectra")
        .join(name)
}

enum FileSource {
    /// File contents written as given.
    Text(String),
    /// The real killeroos spd file of this name, copied in.
    Fixture,
}

struct Imported {
    session: ProjectSession,
    diagnostics: Vec<ImportDiagnostic>,
    _home: tempfile::TempDir,
}

fn import_scene(scene: &str, files: &[(&str, FileSource)]) -> Imported {
    let home = tempfile::tempdir().unwrap();
    for (name, source) in files {
        match source {
            FileSource::Text(text) => fs::write(home.path().join(name), text).unwrap(),
            FileSource::Fixture => {
                fs::copy(fixture(name), home.path().join(name)).unwrap();
            }
        }
    }
    fs::write(home.path().join("scene.pbrt"), scene).unwrap();
    let (_, session, report) = import_project_with_report(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap();
    Imported {
        session,
        diagnostics: report.diagnostics,
        _home: home,
    }
}

impl Imported {
    fn document(&self) -> &ProjectDocument {
        self.session.document()
    }
    fn spectra(&self) -> Vec<&Resource> {
        self.document()
            .resources
            .values()
            .filter(|resource| resource.kind == ResourceKind::SpectrumData)
            .collect()
    }
    fn parameter(&self, id: ResourceId, name: &str) -> &Parameter {
        self.document()
            .resources
            .get(&id)
            .unwrap_or_else(|| panic!("missing resource {id}"))
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("missing parameter {name}"))
    }
    fn has_diagnostic_about(&self, text: &str) -> bool {
        self.diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains(text))
    }
}

/// The samples a file holds, read back in wavelength-value form, so the
/// assertions follow the fixture bytes rather than a copied representation.
fn expected_samples(path: &Path) -> (Vec<f64>, Vec<f64>) {
    let text = fs::read_to_string(path).unwrap();
    let numbers = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.contains('#'))
        .flat_map(str::split_ascii_whitespace)
        .map(|token| token.parse::<f64>().unwrap())
        .collect::<Vec<_>>();
    (
        numbers.chunks_exact(2).map(|pair| pair[0]).collect(),
        numbers.chunks_exact(2).map(|pair| pair[1]).collect(),
    )
}

fn floats(parameter: &Parameter) -> Vec<f64> {
    let ParameterValues::Floats(values) = &parameter.values else {
        panic!("parameter does not hold inline floats: {parameter:?}")
    };
    values
        .iter()
        .map(|value| match value {
            FloatValue::Finite(n) => *n,
            other => panic!("non-finite sample {other:?}"),
        })
        .collect()
}

#[test]
fn spd_references_become_shared_spectrum_data_resources() {
    let imported = import_scene(
        "WorldBegin\n\
         MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] \
         \"spectrum eta\" [\"Au.eta.spd\"] \"spectrum k\" [\"Au.k.spd\"]\n\
         NamedMaterial \"gold\"\n\
         Shape \"sphere\"\n\
         LightSource \"distant\" \"spectrum L\" [\"Au.eta.spd\"]\n",
        &[
            ("Au.eta.spd", FileSource::Fixture),
            ("Au.k.spd", FileSource::Fixture),
        ],
    );
    let document = imported.document();
    let spectra = imported.spectra();
    assert_eq!(
        spectra.len(),
        2,
        "one resource per distinct file: {spectra:?} — diagnostics {:?}",
        imported.diagnostics
    );
    let by_location = |location: &str| {
        spectra
            .iter()
            .copied()
            .find(|resource| resource.location == location)
            .unwrap_or_else(|| panic!("missing {location}"))
    };
    let eta = by_location("Spectra/Au.eta.spectrum");
    let k = by_location("Spectra/Au.k.spectrum");
    assert_eq!(eta.label, "Au.eta");
    assert_eq!(eta.implementation, "piecewise");
    assert_eq!(k.label, "Au.k");
    assert_eq!(k.implementation, "piecewise");
    // The samples are the file's, verbatim.
    let (wavelengths, values) = expected_samples(&fixture("Au.k.spd"));
    assert_eq!(floats(imported.parameter(k.id, "wavelengths")), wavelengths);
    assert_eq!(floats(imported.parameter(k.id, "values")), values);
    let (wavelengths, values) = expected_samples(&fixture("Au.eta.spd"));
    assert_eq!(
        floats(imported.parameter(eta.id, "wavelengths")),
        wavelengths
    );
    assert_eq!(floats(imported.parameter(eta.id, "values")), values);
    // The schema expects floats in the given order.
    assert_eq!(imported.parameter(k.id, "wavelengths").value_type, "float");
    assert_eq!(imported.parameter(k.id, "values").value_type, "float");

    // The parameters reference the resources, spectrum-typed; the light's
    // L and the material's eta share ONE resource for the one file.
    let gold = document
        .resources
        .values()
        .find(|resource| resource.label == "gold")
        .expect("the named material imports");
    let expect_reference = |parameter: &Parameter, id| {
        assert_eq!(parameter.value_type, "spectrum");
        assert_eq!(
            parameter.values,
            ParameterValues::Resources {
                kind: ResourceKind::SpectrumData,
                ids: vec![id],
            }
        );
    };
    expect_reference(
        gold.parameters.iter().find(|p| p.name == "eta").unwrap(),
        eta.id,
    );
    expect_reference(
        gold.parameters.iter().find(|p| p.name == "k").unwrap(),
        k.id,
    );
    let light = document
        .scenes
        .values()
        .flat_map(|scene| scene.nodes.values())
        .find_map(|node| match &node.data {
            NodeKind::Entity {
                entity: EntityKind::Light,
                parameters,
                ..
            } => Some(parameters),
            _ => None,
        })
        .expect("the distant light imports");
    expect_reference(light.iter().find(|p| p.name == "L").unwrap(), eta.id);
    // The spd files are resources now, not assets.
    assert!(
        document.assets.is_empty(),
        "no spd asset remains: {:?}",
        document.assets
    );
    assert!(
        !imported.has_diagnostic_about("Au."),
        "the real files import silently: {:?}",
        imported.diagnostics
    );
}

#[test]
fn inline_spectrum_pairs_builtin_names_and_rgb_stay_as_they_were() {
    let imported = import_scene(
        "WorldBegin\n\
         MakeNamedMaterial \"m\" \"string type\" [\"conductor\"] \
         \"spectrum eta\" [\"metal-Au-eta\"] \"spectrum k\" [400 1 500 2]\n\
         Shape \"sphere\" \"rgb reflectance\" [.2 .2 .2]\n",
        &[],
    );
    let document = imported.document();
    assert!(
        imported.spectra().is_empty(),
        "inline pairs and builtin names do not become resources"
    );
    let material = document
        .resources
        .values()
        .find(|resource| resource.label == "m")
        .expect("the material imports");
    assert_eq!(
        material
            .parameters
            .iter()
            .find(|p| p.name == "eta")
            .unwrap()
            .values,
        ParameterValues::BuiltinSpectra(vec!["metal-Au-eta".into()])
    );
    assert_eq!(
        floats(material.parameters.iter().find(|p| p.name == "k").unwrap()),
        [400., 1., 500., 2.],
        "inline spectrum pairs stay inline floats"
    );
}

#[test]
fn broken_spd_files_keep_the_declaration_on_its_asset_reference() {
    let broken: &[(&str, &str)] = &[
        ("400 abc\n", "non-numeric"),
        ("400 1 500\n", "odd"),
        ("500 1 400 2\n", "descending"),
        ("0 1 400 2\n", "positive"),
        ("400 nan\n", "finite"),
        ("# only a comment\n", "no samples"),
    ];
    for (contents, reason) in broken {
        let contents = contents.to_string();
        let imported = import_scene(
            "WorldBegin\n\
             MakeNamedMaterial \"m\" \"string type\" [\"diffuse\"] \
             \"spectrum eta\" [\"Au.eta.spd\"]\n\
             Shape \"sphere\"\n",
            &[("Au.eta.spd", FileSource::Text(contents))],
        );
        // The file cannot become a curve: the declaration keeps importing,
        // its parameter stays on the legacy asset form, and the report says
        // why — the same policy the open-time migration applies.
        assert!(
            imported
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.contains("Au.eta.spd")
                    && diagnostic.message.contains(reason)),
            "the broken file is reported with its reason: {:?}",
            imported.diagnostics
        );
        assert!(
            imported.spectra().is_empty(),
            "a broken file must not leave a resource behind"
        );
        let document = imported.document();
        assert_eq!(
            document.assets.len(),
            1,
            "the file stays staged as the legacy asset"
        );
        assert_eq!(
            document.assets.values().next().unwrap().location,
            "Spectra/Au.eta.spd"
        );
        let material = document
            .resources
            .values()
            .find(|resource| resource.label == "m")
            .expect("the damaged material still imports");
        let parameter = material
            .parameters
            .iter()
            .find(|parameter| parameter.name == "eta")
            .expect("the parameter survives");
        assert_eq!(parameter.value_type, "spectrum");
        assert!(matches!(parameter.values, ParameterValues::Assets(_)));
        let shapes = document
            .scenes
            .values()
            .flat_map(|scene| scene.nodes.values())
            .filter(|node| {
                matches!(
                    &node.data,
                    NodeKind::Entity {
                        entity: EntityKind::Shape,
                        ..
                    }
                )
            })
            .count();
        assert_eq!(shapes, 1, "the whole scene keeps importing");
    }
    // A referenced file that is not there stays a dependency failure: the
    // declaration skips, the scene completes.
    let missing = import_scene(
        "WorldBegin\n\
         MakeNamedMaterial \"m\" \"string type\" [\"diffuse\"] \
         \"spectrum eta\" [\"gone.spd\"]\n\
         Shape \"sphere\"\n",
        &[],
    );
    assert!(
        missing
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("gone.spd")
                && diagnostic.message.contains("unresolved dependency")),
        "a missing spd is a dependency failure: {:?}",
        missing.diagnostics
    );
    assert!(missing.spectra().is_empty());
    assert!(
        missing
            .document()
            .resources
            .values()
            .all(|resource| resource.label != "m"),
        "a declaration whose file does not exist skips entirely"
    );
}

#[test]
fn oversized_spd_files_stay_assets_with_a_diagnostic() {
    let mut giant = String::new();
    // More pairs than the sample limit; larger than the array threshold so
    // the size gate, not the representation, rejects it.
    for index in 0..=MAX_SPECTRUM_SAMPLES {
        giant.push_str(&format!("{} 1\n", 400. + index as f64));
    }
    let imported = import_scene(
        "WorldBegin\n\
         LightSource \"distant\" \"spectrum L\" [\"giant.spd\"]\n",
        &[("giant.spd", FileSource::Text(giant))],
    );
    assert!(
        imported
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("giant.spd")
                && diagnostic.message.contains("sample")),
        "the oversize is reported: {:?}",
        imported.diagnostics
    );
    assert!(imported.spectra().is_empty());
    let document = imported.document();
    assert_eq!(
        document.assets.len(),
        1,
        "the oversized file stays an asset"
    );
    let light = document
        .scenes
        .values()
        .flat_map(|scene| scene.nodes.values())
        .find_map(|node| match &node.data {
            NodeKind::Entity {
                entity: EntityKind::Light,
                parameters,
                ..
            } => Some(parameters),
            _ => None,
        })
        .expect("the light still imports");
    assert!(
        light
            .iter()
            .find(|parameter| parameter.name == "L")
            .is_some_and(|parameter| matches!(parameter.values, ParameterValues::Assets(_)))
    );
}

#[test]
fn large_spd_tables_ride_the_array_asset_path() {
    // Above the inline float threshold: the importer's bulk storage turns
    // the tables into Data/ .array assets, like any other large parameter.
    let count = MAX_SPECTRUM_SAMPLES;
    let mut text = String::new();
    for index in 0..count {
        text.push_str(&format!("{} {}\n", 400. + index as f64, 1.));
    }
    let imported = import_scene(
        "WorldBegin\n\
         LightSource \"distant\" \"spectrum L\" [\"big.spd\"]\n",
        &[("big.spd", FileSource::Text(text))],
    );
    let spectra = imported.spectra();
    assert_eq!(spectra.len(), 1, "{:?}", imported.diagnostics);
    let resource = spectra[0];
    let array = |name: &str| -> project::Asset {
        let parameter = imported.parameter(resource.id, name);
        assert_eq!(parameter.value_type, "float");
        match &parameter.values {
            ParameterValues::Array {
                asset,
                encoding,
                count,
            } => {
                assert_eq!(*encoding, ArrayEncoding::F64Le);
                assert_eq!(*count as usize, MAX_SPECTRUM_SAMPLES);
                imported.document().assets[asset].clone()
            }
            other => panic!("{name} is not an array: {other:?}"),
        }
    };
    let wavelengths = array("wavelengths");
    let values = array("values");
    assert_eq!(wavelengths.media_type, "application/x-pbrt-ui-array");
    assert_eq!(values.media_type, "application/x-pbrt-ui-array");
    assert!(
        wavelengths.location.starts_with("Data/"),
        "{}",
        wavelengths.location
    );
    assert!(values.location.starts_with("Data/"));
}
