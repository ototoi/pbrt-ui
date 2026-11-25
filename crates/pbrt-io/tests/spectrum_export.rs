//! The text export writes one `spds/<name>.spd` per referenced SpectrumData
//! resource and points its parameters at that path; importing an exported
//! bundle reproduces the same samples.
use pbrt_io::{export_project, import_project};
use project::*;
use std::{fs, path::Path};

enum FileSource {
    Text(String),
    /// The real killeroos spd file of this name, copied in.
    Fixture,
}

fn fixture(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/spectra")
        .join(name)
}

fn import_scene(home: &tempfile::TempDir, scene: &str, files: &[(&str, FileSource)]) {
    for (name, source) in files {
        match source {
            FileSource::Text(text) => fs::write(home.path().join(name), text).unwrap(),
            FileSource::Fixture => {
                fs::copy(fixture(name), home.path().join(name)).unwrap();
            }
        }
    }
    fs::write(home.path().join("scene.pbrt"), scene).unwrap();
}

fn import_at(source: &Path, home: &tempfile::TempDir) -> (ProjectStore, ProjectSession) {
    import_project(
        &home.path().join(source),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap()
}

/// The samples a file holds, parsed using the plain wavelength-value form.
fn file_samples(path: &Path) -> (Vec<f64>, Vec<f64>) {
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

/// An inline-table resource's samples.
fn resource_samples(resource: &Resource) -> (Vec<f64>, Vec<f64>) {
    let inline = |name: &str| {
        let values = resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
            .values
            .clone();
        let ParameterValues::Floats(values) = values else {
            panic!("expected inline floats in {name}")
        };
        values
            .into_iter()
            .map(|value| match value {
                FloatValue::Finite(n) => n,
                other => panic!("non-finite {other:?}"),
            })
            .collect::<Vec<_>>()
    };
    (inline("wavelengths"), inline("values"))
}

const GOLD_SCENE: &str = "WorldBegin\n\
     MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] \
     \"spectrum eta\" [\"Au.eta.spd\"] \"spectrum k\" [\"Au.k.spd\"]\n\
     NamedMaterial \"gold\"\n\
     Shape \"sphere\"\n";

#[test]
fn export_writes_one_spd_per_resource_and_quotes_the_bundle_path() {
    let home = tempfile::tempdir().unwrap();
    import_scene(
        &home,
        GOLD_SCENE,
        &[
            ("Au.eta.spd", FileSource::Fixture),
            ("Au.k.spd", FileSource::Fixture),
        ],
    );
    let (store, session) = import_at(Path::new("scene.pbrt"), &home);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let bundle = exported.parent().unwrap();
    let text = fs::read_to_string(bundle.join("scene.pbrt")).unwrap();
    assert!(
        text.contains("\"spectrum eta\" [ \"spds/Au.eta.spd\" ]"),
        "the parameter points at the bundle file: {text}"
    );
    assert!(
        text.contains("\"spectrum k\" [ \"spds/Au.k.spd\" ]"),
        "{text}"
    );
    // The files hold the resource's samples, pairwise per line.
    let exported_eta = bundle.join("spds").join("Au.eta.spd");
    let exported_k = bundle.join("spds").join("Au.k.spd");
    assert!(exported_eta.is_file(), "spds/Au.eta.spd exists");
    assert!(exported_k.is_file(), "spds/Au.k.spd exists");
    for (exported, fixture_name) in [(exported_eta, "Au.eta.spd"), (exported_k, "Au.k.spd")] {
        let (wavelengths, values) = file_samples(&exported);
        let (expected_wavelengths, expected_values) = file_samples(&fixture(fixture_name));
        assert_eq!(wavelengths, expected_wavelengths);
        assert_eq!(values, expected_values);
    }
}

#[test]
fn exported_bundles_reimport_with_the_same_samples() {
    let home = tempfile::tempdir().unwrap();
    import_scene(
        &home,
        GOLD_SCENE,
        &[
            ("Au.eta.spd", FileSource::Fixture),
            ("Au.k.spd", FileSource::Fixture),
        ],
    );
    let (store, session) = import_at(Path::new("scene.pbrt"), &home);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    // The exported bundle is a full input: reimport its own scene.pbrt.
    let (_, reopened) = import_project(
        &exported,
        &home.path().join("reproject"),
        &Default::default(),
        || false,
    )
    .unwrap();
    let document = reopened.document();
    let spectra: Vec<&Resource> = document
        .resources
        .values()
        .filter(|resource| resource.kind == ResourceKind::SpectrumData)
        .collect();
    assert_eq!(spectra.len(), 2, "the two files import back as resources");
    for resource in spectra {
        let (wavelengths, values) = resource_samples(resource);
        let matches_fixture = ["Au.eta", "Au.k"].iter().any(|name| {
            let (expected_wavelengths, expected_values) =
                file_samples(&fixture(&format!("{name}.spd")));
            wavelengths == expected_wavelengths && values == expected_values
        });
        assert!(
            matches_fixture,
            "samples {wavelengths:?}/{values:?} trace back to a fixture file"
        );
    }
}

#[test]
fn colliding_spectrum_names_number_like_other_dependencies() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("g1")).unwrap();
    fs::create_dir_all(home.path().join("g2")).unwrap();
    import_scene(
        &home,
        "WorldBegin\n\
         MakeNamedMaterial \"m\" \"string type\" [\"conductor\"] \"spectrum k\" [\"g1/gold.spd\"]\n\
         NamedMaterial \"m\"\n\
         Shape \"sphere\"\n\
         LightSource \"distant\" \"spectrum L\" [\"g2/gold.spd\"]\n",
        &[
            ("g1/gold.spd", FileSource::Text("400 1\n500 2\n".into())),
            ("g2/gold.spd", FileSource::Text("600 3\n700 4\n".into())),
        ],
    );
    // The two sources share a stem, so their labels collide: `gold`, `gold_1`.
    let (store, session) = import_at(Path::new("scene.pbrt"), &home);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let bundle = exported.parent().unwrap();
    let text = fs::read_to_string(bundle.join("scene.pbrt")).unwrap();
    assert!(
        text.contains("\"spectrum k\" [ \"spds/gold.spd\" ]"),
        "first by location order: {text}"
    );
    assert!(
        text.contains("\"spectrum L\" [ \"spds/gold_1.spd\" ]"),
        "the collision numbers with _1: {text}"
    );
    let (wavelengths, values) = file_samples(&bundle.join("spds/gold.spd"));
    assert_eq!((wavelengths, values), (vec![400., 500.], vec![1., 2.]));
    let (wavelengths, values) = file_samples(&bundle.join("spds/gold_1.spd"));
    assert_eq!((wavelengths, values), (vec![600., 700.], vec![3., 4.]));
}

#[test]
fn spectrum_file_names_number_case_insensitively() {
    let home = tempfile::tempdir().unwrap();
    fs::create_dir_all(home.path().join("g1")).unwrap();
    fs::create_dir_all(home.path().join("g2")).unwrap();
    import_scene(
        &home,
        "WorldBegin\n\
         MakeNamedMaterial \"m\" \"string type\" [\"conductor\"] \"spectrum k\" [\"g1/Gold.spd\"]\n\
         NamedMaterial \"m\"\n\
         Shape \"sphere\"\n\
         LightSource \"distant\" \"spectrum L\" [\"g2/gold.spd\"]\n",
        &[
            ("g1/Gold.spd", FileSource::Text("400 1\n".into())),
            ("g2/gold.spd", FileSource::Text("600 3\n".into())),
        ],
    );
    // `Gold` and `gold` collide the moment a filesystem stops distinguishing
    // case: the bundle numbers them case-insensitively, before one file can
    // silently overwrite the other on such a disk.
    let (store, session) = import_at(Path::new("scene.pbrt"), &home);
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let bundle = exported.parent().unwrap();
    let text = fs::read_to_string(bundle.join("scene.pbrt")).unwrap();
    assert!(
        text.contains("\"spectrum k\" [ \"spds/Gold.spd\" ]"),
        "{text}"
    );
    assert!(
        text.contains("\"spectrum L\" [ \"spds/gold_1.spd\" ]"),
        "the case-insensitive collision numbers with _1: {text}"
    );
    assert!(bundle.join("spds/Gold.spd").is_file());
    assert!(bundle.join("spds/gold_1.spd").is_file());
    let (wavelengths, values) = file_samples(&bundle.join("spds/Gold.spd"));
    assert_eq!((wavelengths, values), (vec![400.], vec![1.]));
    let (wavelengths, values) = file_samples(&bundle.join("spds/gold_1.spd"));
    assert_eq!((wavelengths, values), (vec![600.], vec![3.]));
}
