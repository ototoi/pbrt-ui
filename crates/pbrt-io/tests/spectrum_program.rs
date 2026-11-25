//! The direct-preview program expands SpectrumData references into
//! interleaved λ/value `spectrum` parameters, so Lit resolves them with no
//! spd file in sight — while inline pairs keep their shape and the text
//! export keeps writing `spds/` files. Long curves reach the preview
//! transfer's value limit, where the standard `truncated` flag reports it.
use pbrt_io::{export_project, import_project, program_from_project, stage_assets};
use scene_parse::{GeometryCache, preview_from_program};
use std::fs;

const QUAD: &str = "Shape \"trianglemesh\" \"point3 P\" [-2 -2 0 2 -2 0 2 2 0 -2 2 0] \"integer indices\" [0 1 2 0 2 1]\n";

fn import_scene(home: &tempfile::TempDir, scene: &str, files: &[(&str, &str)]) {
    for (name, text) in files {
        fs::write(home.path().join(name), text).unwrap();
    }
    fs::write(home.path().join("scene.pbrt"), scene).unwrap();
}

/// The direct path core uses for previews, minus the publish: staged
/// assets, the program, the preview package.
fn preview_of(
    home: &tempfile::TempDir,
    store: &project::ProjectStore,
    session: &project::ProjectSession,
) -> scene_parse::PreviewGeometry {
    let staged = home.path().join("direct");
    fs::create_dir_all(&staged).unwrap();
    let assets = stage_assets(session.document(), store, &staged, &|| false).unwrap();
    let program = program_from_project(&session.snapshot(), store, &assets, || false).unwrap();
    let origins = program.origins.clone();
    preview_from_program(
        program,
        Some(&origins),
        &staged,
        Default::default(),
        &mut GeometryCache::default(),
    )
    .unwrap()
}

fn area_light_parameter<'a>(
    preview: &'a scene_parse::PreviewGeometry,
    name: &str,
) -> &'a protocol::PreviewParameter {
    preview
        .manifest
        .draws
        .iter()
        .find_map(|draw| draw.area_light.as_ref())
        .and_then(|light| light.parameters.iter().find(|p| p.name == name))
        .unwrap_or_else(|| panic!("the area light carries no {name}"))
}

#[test]
fn the_program_expands_spectrum_references_into_interleaved_pairs() {
    let home = tempfile::tempdir().unwrap();
    import_scene(
        &home,
        &format!(
            "WorldBegin\n\
             MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] \"spectrum k\" [\"x.spd\"]\n\
             AttributeBegin\n\
             AreaLightSource \"diffuse\" \"spectrum L\" [\"x.spd\"]\n\
             {QUAD}AttributeEnd\n"
        ),
        &[("x.spd", "400 1\n500 2\n600 4\n")],
    );
    let (store, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap();

    // The area light's L is the resource's samples, interleaved in the
    // order pbrt reads them — no file path anywhere in the program.
    let preview = preview_of(&home, &store, &session);
    let parameter = area_light_parameter(&preview, "L");
    assert_eq!(parameter.value_type, "spectrum");
    assert_eq!(parameter.floats, [400., 1., 500., 2., 600., 4.]);
    assert!(!parameter.truncated);

    // The same snapshot's text export still points at its spds files.
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&exported).unwrap();
    assert!(text.contains("\"spectrum k\" [ \"spds/x.spd\" ]"), "{text}");
    assert!(text.contains("\"spectrum L\" [ \"spds/x.spd\" ]"), "{text}");
}

#[test]
fn inline_spectrum_pairs_keep_their_form_in_the_program() {
    let home = tempfile::tempdir().unwrap();
    import_scene(
        &home,
        &format!(
            "WorldBegin\n\
             AttributeBegin\n\
             AreaLightSource \"diffuse\" \"spectrum L\" [400 1 500 2]\n\
             {QUAD}AttributeEnd\n"
        ),
        &[],
    );
    let (store, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap();
    let preview = preview_of(&home, &store, &session);
    let parameter = area_light_parameter(&preview, "L");
    assert_eq!(parameter.value_type, "spectrum");
    assert_eq!(parameter.floats, [400., 1., 500., 2.]);
    assert!(!parameter.truncated);
}

#[test]
fn long_curves_decimate_onto_the_visible_grid_within_the_transfer_limit() {
    let home = tempfile::tempdir().unwrap();
    let mut spd = String::new();
    for index in 0..40 {
        spd.push_str(&format!("{} {}\n", 400. + 10. * index as f64, 1.));
    }
    import_scene(
        &home,
        &format!(
            "WorldBegin\n\
             AttributeBegin\n\
             AreaLightSource \"diffuse\" \"spectrum L\" [\"x.spd\"]\n\
             {QUAD}AttributeEnd\n"
        ),
        &[("x.spd", &spd)],
    );
    let (store, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap();
    let preview = preview_of(&home, &store, &session);
    let parameter = area_light_parameter(&preview, "L");
    assert_eq!(
        parameter.floats.len(),
        42,
        "the curve decimates onto the 380–780 nm decade grid, inside the cap"
    );
    assert!(
        parameter.floats.len() <= protocol::MAX_PREVIEW_PARAMETER_VALUES,
        "never past the transfer cap"
    );
    assert!(
        !parameter.truncated,
        "no value is dropped at the cap: the decimated curve fits"
    );
    // Each sample is the 20 nm bin mean of r4's piecewise-linear read:
    // outside the curve's 400–790 nm span reads 0, so the 400 nm bin
    // (390–410) is half covered.
    let expected: Vec<f64> = (0..21)
        .flat_map(|index| {
            let wavelength = 380. + 20. * index as f64;
            let value = match wavelength {
                380. => 0.,
                400. => 0.5,
                _ => 1.,
            };
            [wavelength, value]
        })
        .collect();
    assert_eq!(parameter.floats, expected);

    // The same snapshot's text export still writes every sample, kept
    // quoted at its spds file.
    let exported = export_project(
        &session.snapshot(),
        &store,
        &home.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&exported).unwrap();
    assert!(text.contains("\"spectrum L\" [ \"spds/x.spd\" ]"), "{text}");
}

#[test]
fn decimation_keeps_a_peak_narrower_than_the_grid() {
    // 40 pairs at 516–555 nm, 1 only at 532 nm: no grid point lands on it.
    let home = tempfile::tempdir().unwrap();
    let spd: String = (516..=555)
        .map(|wavelength| format!("{wavelength} {}\n", u8::from(wavelength == 532)))
        .collect();
    import_scene(
        &home,
        &format!(
            "WorldBegin\n\
             AttributeBegin\n\
             AreaLightSource \"diffuse\" \"spectrum L\" [\"x.spd\"]\n\
             {QUAD}AttributeEnd\n"
        ),
        &[("x.spd", &spd)],
    );
    let (store, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &Default::default(),
        || false,
    )
    .unwrap();
    let preview = preview_of(&home, &store, &session);
    let parameter = area_light_parameter(&preview, "L");
    let values: Vec<f64> = parameter.floats.chunks(2).map(|pair| pair[1]).collect();
    // The 1 nm-wide triangle holds 1 nm of energy; the 20 nm bins keep it,
    // all in the 540 nm bin (530–550).
    let energy: f64 = values.iter().map(|value| value * 20.).sum();
    assert!((energy - 1.).abs() < 1e-9, "{values:?}");
    assert_eq!(parameter.floats[16..18], [540., 0.05]);
}
