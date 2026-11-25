//! Changing an implementation name: parameters are always kept; the table
//! identifies the ones the new implementation does not know and the
//! ones the new implementation could no longer export.
use pbrt_io::implementation_change::{
    ImplementationChange, Rejection, candidates, implementation_change,
};
use project::*;

fn parameter(name: &str, value_type: &str, values: ParameterValues) -> Parameter {
    Parameter {
        name: name.into(),
        value_type: value_type.into(),
        color_space: ColorSpaceBinding::Inherit,
        values,
    }
}

fn floats(name: &str, value_type: &str, values: &[f64]) -> Parameter {
    parameter(
        name,
        value_type,
        ParameterValues::Floats(values.iter().map(|v| FloatValue::Finite(*v)).collect()),
    )
}

fn texture(name: &str) -> Parameter {
    parameter(
        name,
        "texture",
        ParameterValues::Resources {
            kind: ResourceKind::Texture,
            ids: vec![ResourceId::new()],
        },
    )
}

fn file(name: &str) -> Parameter {
    parameter(
        name,
        "string",
        ParameterValues::Assets(vec![AssetId::new()]),
    )
}

fn change(
    directive: &str,
    current: &str,
    texture_value_type: Option<TextureValueType>,
    parameters: &[Parameter],
    requested: &str,
) -> ImplementationChange {
    let texture_types = std::collections::HashMap::new();
    implementation_change(
        directive,
        current,
        texture_value_type,
        &texture_types,
        parameters,
        requested,
    )
    .expect("accepted")
    .expect("a change")
}

#[test]
fn matching_name_and_type_is_kept_silently_and_the_rest_is_reported() {
    let parameters = [
        floats("reflectance", "rgb", &[0.2, 0.3, 0.4]),
        floats("roughness", "float", &[0.1]),
        // Same name, a type the new row does not accept.
        floats("eta", "point3", &[1., 2., 3.]),
        floats("mystery", "float", &[1.]),
    ];
    let plan = change(
        "MakeNamedMaterial",
        "coateddiffuse",
        None,
        &parameters,
        "conductor",
    );
    assert_eq!(plan.implementation, "conductor");
    assert_eq!(plan.kept_unknown, ["eta", "mystery"]);
    assert!(plan.breaks_reference.is_empty());
}

#[test]
fn texture_bound_parameters_need_a_row_that_accepts_textures() {
    let parameters = [texture("reflectance"), texture("eta")];
    // conductor's eta takes a spectrum texture; dielectric's eta is constant.
    let plan = change("Material", "conductor", None, &parameters, "dielectric");
    assert_eq!(plan.kept_unknown, ["reflectance", "eta"]);
    let plan = change("Material", "diffuse", None, &parameters[..1], "conductor");
    assert!(plan.kept_unknown.is_empty());
}

#[test]
fn conductor_spectrum_eta_texture_is_dropped_when_changing_to_hair() {
    let texture_id = ResourceId::new();
    let parameters = [parameter(
        "eta",
        "texture",
        ParameterValues::Resources {
            kind: ResourceKind::Texture,
            ids: vec![texture_id],
        },
    )];
    let texture_types = [(texture_id, TextureValueType::Spectrum)]
        .into_iter()
        .collect();
    let plan = implementation_change(
        "Material",
        "conductor",
        None,
        &texture_types,
        &parameters,
        "hair",
    )
    .unwrap()
    .unwrap();
    assert_eq!(plan.breaks_reference, ["eta"]);
    assert!(plan.kept_unknown.is_empty());
}

#[test]
fn aliases_normalize_and_the_same_implementation_is_a_no_op() {
    assert_eq!(
        implementation_change(
            "Material",
            "diffuse",
            None,
            &std::collections::HashMap::new(),
            &[],
            "diffuse"
        ),
        Ok(None)
    );
    let plan = change(
        "Shape",
        "sphere",
        None,
        &[floats("st", "point2", &[0., 0.])],
        "trianglemesh",
    );
    // `st` is trianglemesh's alias of uv.
    assert!(plan.kept_unknown.is_empty());
}

#[test]
fn an_implementation_without_a_table_keeps_and_reports_everything() {
    // bdpt stays exportable but the first edition's table does not cover
    // it, so every declared parameter is kept as unknown.
    let parameters = [
        floats("maxdepth", "integer", &[5.]),
        floats("regularize", "bool", &[0.]),
    ];
    let plan = change("Integrator", "path", None, &parameters, "bdpt");
    assert_eq!(plan.implementation, "bdpt");
    assert_eq!(plan.kept_unknown, ["maxdepth", "regularize"]);
    let plan = change(
        "Camera",
        "perspective",
        None,
        &[floats("fov", "float", &[45.])],
        "orthographic",
    );
    assert_eq!(plan.kept_unknown, ["fov"]);
}

/// Path and volpath use identical tables, so switching between
/// them keeps every row as known.
#[test]
fn table_implementations_report_matching_rows() {
    let parameters = [
        floats("maxdepth", "integer", &[7.]),
        floats("regularize", "bool", &[1.]),
        parameter(
            "lightsamplestrategy",
            "string",
            ParameterValues::Strings(vec!["uniform".into()]),
        ),
    ];
    let plan = change("Integrator", "path", None, &parameters, "volpath");
    assert_eq!(plan.implementation, "volpath");
    assert!(plan.kept_unknown.is_empty(), "{:?}", plan.kept_unknown);
    let plan = change(
        "Sampler",
        "zsobol",
        None,
        &[floats("pixelsamples", "integer", &[32.])],
        "halton",
    );
    assert!(plan.kept_unknown.is_empty(), "{:?}", plan.kept_unknown);
    // halton rejects fastowen's spelling in r4, but the row exists in both
    // tables; the values are the user's to fix.
    let fastowen = parameter(
        "randomization",
        "string",
        ParameterValues::Strings(vec!["fastowen".into()]),
    );
    let plan = change("Sampler", "zsobol", None, &[fastowen], "halton");
    assert!(plan.kept_unknown.is_empty(), "{:?}", plan.kept_unknown);
    assert!(
        change("Sampler", "zsobol", None, &[], "pmj02bn")
            .kept_unknown
            .is_empty()
    );
}

#[test]
fn references_and_files_the_new_implementation_cannot_export_break() {
    let materials = parameter(
        "materials",
        "string",
        ParameterValues::Resources {
            kind: ResourceKind::Material,
            ids: vec![ResourceId::new(), ResourceId::new()],
        },
    );
    let plan = change(
        "MakeNamedMaterial",
        "mix",
        None,
        &[materials, floats("amount", "float", &[0.5])],
        "diffuse",
    );
    assert_eq!(plan.breaks_reference, ["materials"]);
    assert_eq!(
        plan.kept_unknown,
        ["amount"],
        "broken ones are not repeated"
    );

    let plan = change("Shape", "plymesh", None, &[file("filename")], "sphere");
    assert_eq!(plan.breaks_reference, ["filename"]);
    let plan = change(
        "Texture",
        "imagemap",
        Some(TextureValueType::Spectrum),
        &[file("filename")],
        "constant",
    );
    assert_eq!(plan.breaks_reference, ["filename"]);
    // A file stays a file where the new implementation manages one too.
    let plan = change(
        "LightSource",
        "infinite",
        None,
        &[file("filename")],
        "projection",
    );
    assert!(plan.breaks_reference.is_empty());
}

#[test]
fn refusals() {
    let refuse = |directive, current, texture_value_type, requested| {
        implementation_change(
            directive,
            current,
            texture_value_type,
            &std::collections::HashMap::new(),
            &[],
            requested,
        )
    };
    assert_eq!(
        refuse("Material", "diffuse", None, ""),
        Err(Rejection::Empty)
    );
    assert_eq!(
        refuse("AreaLightSource", "", None, "diffuse"),
        Err(Rejection::Cancelling)
    );
    assert_eq!(
        refuse("AreaLightSource", "diffuse", None, ""),
        Err(Rejection::Empty)
    );
    assert_eq!(
        refuse("Material", "diffuse", None, "plastic"),
        Err(Rejection::Unsupported)
    );
    assert_eq!(
        refuse("Integrator", "path", None, "whitted"),
        Err(Rejection::Unsupported)
    );
    // marble has only a spectrum table: a float texture cannot become one.
    assert_eq!(
        refuse(
            "Texture",
            "constant",
            Some(TextureValueType::Float),
            "marble"
        ),
        Err(Rejection::TextureValueType)
    );
    assert!(
        implementation_change(
            "Texture",
            "constant",
            Some(TextureValueType::Spectrum),
            &std::collections::HashMap::new(),
            &[],
            "marble"
        )
        .is_ok()
    );
}

#[test]
fn candidates_come_from_the_table() {
    assert!(candidates("Material").contains(&"conductor"));
    for implementation in ["thindielectric", "hair", "subsurface", "measured"] {
        assert!(candidates("Material").contains(&implementation));
    }
    // The tables pin the render options' lists (r4's other integrators
    // and the advanced cameras stay for a later edition, still legal to
    // switch to by name).
    assert_eq!(candidates("Integrator"), ["path", "volpath"]);
    assert_eq!(candidates("Camera"), ["perspective", "orthographic"]);
    assert_eq!(candidates("Film"), ["rgb"]);
    assert_eq!(
        candidates("Sampler"),
        [
            "zsobol",
            "halton",
            "sobol",
            "paddedsobol",
            "pmj02bn",
            "stratified",
            "independent"
        ]
    );
    assert_eq!(
        candidates("PixelFilter"),
        ["box", "gaussian", "mitchell", "sinc", "triangle"]
    );
    assert_eq!(candidates("Accelerator"), ["bvh", "kdtree"]);
    assert!(candidates("Camera").contains(&"perspective"));
    assert!(!candidates("AreaLightSource").contains(&""));
    // Directives with neither table nor export support stay empty.
    assert!(candidates("Option").is_empty());
}
