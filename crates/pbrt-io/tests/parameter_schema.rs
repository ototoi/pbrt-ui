//! The per-implementation parameter table must describe
//! implementations the export schema supports, and parameters built from the
//! table's defaults must pass document validation and export — the table is
//! information, never a new restriction, but its defaults must be honest.
use pbrt_io::check_exportable;
use pbrt_io::parameter_schema as schema;
use project::*;
use std::sync::Arc;

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
        ParameterValues::Floats(
            values
                .iter()
                .map(|value| FloatValue::Finite(*value))
                .collect(),
        ),
    )
}

fn entity_node(entity: EntityKind, implementation: &str, parameters: Vec<Parameter>) -> Node {
    let directive = match entity {
        EntityKind::Shape => "Shape",
        EntityKind::Light => "LightSource",
        EntityKind::AreaLight => "AreaLightSource",
        EntityKind::Camera => "Camera",
    };
    Node::new(
        directive,
        NodeKind::Entity {
            entity,
            implementation: implementation.into(),
            parameters,
            pose: None,
        },
    )
}

/// Append a node under the scene's World scope, the way imports place
/// declarations ("unreachable scene nodes" otherwise fails validation).
fn append_world(document: &mut ProjectDocument, node: Node) {
    let scene = Arc::make_mut(
        document
            .scenes
            .get_mut(&document.default_scene.unwrap())
            .unwrap(),
    );
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&scene.world).unwrap().data
    else {
        panic!("missing World");
    };
    children.push(node.id);
    scene.nodes.insert(node.id, node);
}

fn resource(
    kind: ResourceKind,
    implementation: &str,
    parameters: Vec<Parameter>,
    texture_value_type: Option<TextureValueType>,
) -> Resource {
    let (folder, extension) = match kind {
        ResourceKind::Material => ("Materials", "material"),
        ResourceKind::Texture => ("Textures", "texture"),
        ResourceKind::Medium => ("Media", "medium"),
        ResourceKind::ObjectDefinition => ("Objects", "object"),
        ResourceKind::SpectrumData => ("Spectra", "spectrum"),
    };
    let label = format!(
        "{implementation}-{}",
        texture_value_type
            .map(|value| format!("{value:?}").to_lowercase())
            .unwrap_or_else(|| "x".into())
    );
    Resource {
        id: ResourceId::new(),
        label: label.clone(),
        location: format!("{folder}/{label}.{extension}"),
        edition: Revision::initial(),
        kind,
        texture_value_type,
        implementation: implementation.into(),
        parameters,
        definition_scope: None,
        declared_in: None,
        origin: None,
    }
}

/// The PBRT directive each kind of table is addressed by.
const DIRECTIVES: [&str; 7] = [
    "Shape",
    "Material",
    "MakeNamedMaterial",
    "LightSource",
    "AreaLightSource",
    "Texture",
    "MakeNamedMedium",
];

/// Parameters the dependency rules of today's export cannot carry as inline
/// constants: `aamode` / `preset` / `emissionfilename` are plain strings the
/// export schema does not whitelist yet (their defaults are real in r4; the
/// table records them, the export rules catch up later), and file parameters
/// with an empty-string default mean "absent" — a real file lands as a
/// managed asset reference, never as this placeholder.
fn not_yet_exportable(spec: &schema::ParameterSpec) -> bool {
    matches!(spec.name, "emissionfilename")
        || spec.default == Some(schema::DefaultValue::Strings(&[""]))
}

/// Minimal values for parameters the table marks required and gives no
/// constant default for. `mix` material's required `materials` reference is
/// built in the test body (it names two materials of the same document).
fn minimal_required(
    directive: &str,
    implementation: &str,
    assets: &[AssetId],
    materials: &[ResourceId],
) -> Vec<Parameter> {
    match (directive, implementation) {
        ("Shape", "trianglemesh") => vec![
            parameter(
                "indices",
                "integer",
                ParameterValues::Integers(vec![0, 3, 1]),
            ),
            floats("P", "point3", &[0., 0., 0., 1., 0., 0., 0., 1., 0.]),
        ],
        ("Shape", "bilinearmesh") => vec![floats(
            "P",
            "point3",
            &[0., 0., 0., 1., 0., 0., 1., 1., 0., 0., 1., 0.],
        )],
        ("Shape", "plymesh") => vec![parameter(
            "filename",
            "string",
            ParameterValues::Assets(assets.to_vec()),
        )],
        ("Material" | "MakeNamedMaterial", "measured") => vec![parameter(
            "filename",
            "string",
            ParameterValues::Assets(assets.to_vec()),
        )],
        ("Texture", "scale") => vec![floats("tex", "float", &[1.])],
        ("Texture", "imagemap") => vec![parameter(
            "filename",
            "string",
            ParameterValues::Assets(assets.to_vec()),
        )],
        ("Material", "mix") | ("MakeNamedMaterial", "mix") => vec![parameter(
            "materials",
            "string",
            ParameterValues::Resources {
                kind: ResourceKind::Material,
                ids: materials.to_vec(),
            },
        )],
        _ => Vec::new(),
    }
}

fn supplied_by_default_or_minima(spec: &schema::ParameterSpec, minima: &[Parameter]) -> bool {
    spec.default_parameter().is_some() || minima.iter().any(|found| found.name == spec.name)
}

#[test]
fn every_covered_implementation_exports_from_its_defaults() {
    let mut checked = 0;
    for directive in DIRECTIVES {
        let implementations =
            schema::schema(directive).unwrap_or_else(|| panic!("no table for {directive}"));
        for implementation in implementations {
            let texture_types = if directive == "Texture" {
                vec![
                    Some(TextureValueType::Float),
                    Some(TextureValueType::Spectrum),
                ]
            } else {
                vec![None]
            };
            for texture_value_type in texture_types {
                let Some(parameters) = implementation.parameters(texture_value_type) else {
                    continue;
                };
                let name_check = minimal_required(directive, implementation.name, &[], &[]);
                for spec in parameters {
                    if spec.required {
                        assert!(
                            supplied_by_default_or_minima(spec, &name_check),
                            "{} {} {:?} {spec:?} is required but nothing supplies it",
                            directive,
                            implementation.name,
                            texture_value_type,
                        );
                    }
                }
                let mut document = ProjectDocument::new("Schema");
                let mut node_ids = Vec::new();
                let mut resource_ids = Vec::new();
                // A managed asset for the required file parameters.
                let mut assets = Vec::new();
                if matches!(
                    (directive, implementation.name),
                    ("Shape", "plymesh")
                        | ("Texture", "imagemap")
                        | ("Material" | "MakeNamedMaterial", "measured")
                ) {
                    let asset = Asset {
                        id: AssetId::new(),
                        location: format!("Other/{}.bin", implementation.name),
                        media_type: "application/octet-stream".into(),
                        sha256: "0".repeat(64),
                        size: 1,
                        origin: None,
                    };
                    assets.push(asset.id);
                    document.assets.insert(asset.id, asset);
                }
                // Two materials for mix's required reference list.
                let mut materials = Vec::new();
                if matches!(directive, "Material" | "MakeNamedMaterial")
                    && implementation.name == "mix"
                {
                    for index in 0..2 {
                        let diffuse = resource(
                            ResourceKind::Material,
                            "diffuse",
                            vec![floats("reflectance", "rgb", &[0.5, 0.5, 0.5])],
                            None,
                        );
                        // Distinct items need distinct locations.
                        let mut unique = diffuse.clone();
                        unique.label = format!("diffuse-{index}");
                        unique.location = format!("Materials/diffuse-{index}.material");
                        document.resources.insert(unique.id, unique.clone());
                        materials.push(unique.id);
                    }
                }
                let mut params: Vec<Parameter> = parameters
                    .iter()
                    .filter(|spec| !not_yet_exportable(spec))
                    .filter_map(|spec| spec.default_parameter())
                    .chain(minimal_required(
                        directive,
                        implementation.name,
                        &assets,
                        &materials,
                    ))
                    .collect();
                params.sort_by(|a, b| a.name.cmp(&b.name));
                params.dedup_by(|a, b| a.name == b.name);

                match directive {
                    "Shape" | "LightSource" | "AreaLightSource" => {
                        let entity = match directive {
                            "Shape" => EntityKind::Shape,
                            "LightSource" => EntityKind::Light,
                            _ => EntityKind::AreaLight,
                        };
                        let node = entity_node(entity, implementation.name, params);
                        append_world(&mut document, node.clone());
                        node_ids.push(node.id);
                    }
                    "Texture" => {
                        let item = resource(
                            ResourceKind::Texture,
                            implementation.name,
                            params,
                            texture_value_type,
                        );
                        document.resources.insert(item.id, item.clone());
                        resource_ids.push(item.id);
                    }
                    "MakeNamedMedium" => {
                        let item =
                            resource(ResourceKind::Medium, implementation.name, params, None);
                        document.resources.insert(item.id, item.clone());
                        resource_ids.push(item.id);
                    }
                    "Material" | "MakeNamedMaterial" => {
                        let item =
                            resource(ResourceKind::Material, implementation.name, params, None);
                        document.resources.insert(item.id, item.clone());
                        resource_ids.push(item.id);
                    }
                    _ => unreachable!(),
                }
                document.validate().unwrap_or_else(|error| {
                    panic!(
                        "{} {} {:?} defaults do not validate: {error}",
                        directive, implementation.name, texture_value_type
                    )
                });
                check_exportable(&document, &node_ids, &resource_ids).unwrap_or_else(|error| {
                    panic!(
                        "{} {} {:?} defaults do not export: {error}",
                        directive, implementation.name, texture_value_type
                    )
                });
                checked += 1;
            }
        }
    }
    assert!(
        checked > 40,
        "suspiciously little covered: {checked} (shapes 6 + materials 7 + lights 5 + textures 23 + medium 1)"
    );
}

#[test]
fn the_table_is_transcribed_from_r4_not_invented() {
    let sphere = schema::implementation(schema::schema("Shape").unwrap(), "sphere").unwrap();
    let radius = sphere
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "radius")
        .unwrap();
    assert_eq!(
        radius.default_parameter().unwrap().values,
        ParameterValues::Floats(vec![FloatValue::Finite(1.)])
    );
    let zmin = sphere
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "zmin")
        .unwrap();
    assert_eq!(
        zmin.default,
        Some(schema::DefaultValue::Depends {
            parameter: "radius",
            operation: schema::DependsOperation::Negated,
        })
    );

    let diffuse =
        schema::implementation(schema::schema("MakeNamedMaterial").unwrap(), "diffuse").unwrap();
    let reflectance = diffuse
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "reflectance")
        .unwrap();
    assert_eq!(
        reflectance.texture,
        Some(schema::TextureInput::SpectrumAlbedo)
    );
    assert_eq!(
        reflectance.default_parameter().unwrap().values,
        // A single scalar for a spectrum parameter becomes an rgb triple.
        ParameterValues::Floats(vec![
            FloatValue::Finite(0.5),
            FloatValue::Finite(0.5),
            FloatValue::Finite(0.5),
        ])
    );
    // displacement / normalmap ride with every material but mix.
    assert!(
        diffuse
            .parameters(None)
            .unwrap()
            .iter()
            .any(|p| p.name == "displacement")
    );

    let conductor =
        schema::implementation(schema::schema("Material").unwrap(), "conductor").unwrap();
    let eta = conductor
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "eta")
        .unwrap();
    assert_eq!(
        eta.default_parameter().unwrap().values,
        ParameterValues::BuiltinSpectra(vec!["metal-Cu-eta".into()])
    );
    assert_eq!(eta.texture, Some(schema::TextureInput::SpectrumUnbounded));
    let expected: &[&[&str]] = &[&["eta", "k"], &["reflectance"]];
    assert_eq!(conductor.exclusive, expected);

    let mix = schema::implementation(schema::schema("Material").unwrap(), "mix").unwrap();
    assert!(
        !mix.parameters(None)
            .unwrap()
            .iter()
            .any(|p| p.name == "displacement"),
        "mix has no displacement/normalmap"
    );
    let materials = mix
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "materials")
        .unwrap();
    assert!(materials.required);
    assert_eq!(materials.role, schema::Role::Resource);

    let tri = schema::implementation(schema::schema("Shape").unwrap(), "trianglemesh").unwrap();
    let twosided = tri
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "twosided")
        .unwrap();
    assert_eq!(
        twosided.default_parameter().unwrap().values,
        ParameterValues::Booleans(vec![true])
    );
    let uv = tri
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "uv")
        .unwrap();
    assert!(uv.aliases.contains(&"st"));

    // Every uroughness / vroughness entry accepts float
    // textures even though their defaults follow roughness.
    for implementation in ["conductor", "dielectric", "coateddiffuse"] {
        let table =
            schema::implementation(schema::schema("Material").unwrap(), implementation).unwrap();
        let uroughness = table
            .parameters(None)
            .unwrap()
            .iter()
            .find(|p| p.name == "uroughness")
            .unwrap();
        assert_eq!(uroughness.texture, Some(schema::TextureInput::Float));
        assert_eq!(uroughness.role, schema::Role::TextureInput);
    }
    let sphere_table = schema::implementation(schema::schema("Shape").unwrap(), "sphere").unwrap();
    assert_eq!(
        sphere_table
            .parameters(None)
            .unwrap()
            .iter()
            .find(|p| p.name == "zmax")
            .unwrap()
            .texture,
        None,
        "shape scalars do not take textures"
    );

    // The medium prefers explicit sigma over the preset.
    let homogeneous =
        schema::implementation(schema::schema("MakeNamedMedium").unwrap(), "homogeneous").unwrap();
    let sigma_a = homogeneous
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "sigma_a")
        .unwrap();
    assert_eq!(
        sigma_a.default,
        Some(schema::DefaultValue::PresetElse(&[1.]))
    );
}

#[test]
fn texture_tables_are_per_value_type_and_carry_the_r4_defaults() {
    let mix = schema::implementation(schema::schema("Texture").unwrap(), "mix").unwrap();
    let float = mix.parameters(Some(TextureValueType::Float)).unwrap();
    let spectrum = mix.parameters(Some(TextureValueType::Spectrum)).unwrap();
    let default = |params: &[schema::ParameterSpec], name: &str| {
        params
            .iter()
            .find(|p| p.name == name)
            .unwrap()
            .default_parameter()
            .unwrap()
            .values
    };
    // The r4 divergence: spectrum's tex1 is 1
    // (shown as an rgb triple; a spectrum scalar is a color here).
    assert_eq!(
        default(float, "tex1"),
        ParameterValues::Floats(vec![FloatValue::Finite(0.)])
    );
    assert_eq!(
        default(spectrum, "tex1"),
        ParameterValues::Floats(vec![
            FloatValue::Finite(1.),
            FloatValue::Finite(1.),
            FloatValue::Finite(1.),
        ])
    );
    assert_eq!(
        default(float, "tex2"),
        ParameterValues::Floats(vec![FloatValue::Finite(1.)])
    );
    assert_eq!(
        default(spectrum, "tex2"),
        ParameterValues::Floats(vec![
            FloatValue::Finite(0.),
            FloatValue::Finite(0.),
            FloatValue::Finite(0.),
        ])
    );
    // scale's first operand is r4's "tex" (alias tex1), required.
    let scale = schema::implementation(schema::schema("Texture").unwrap(), "scale").unwrap();
    let tex = scale
        .parameters(Some(TextureValueType::Float))
        .unwrap()
        .iter()
        .find(|p| p.name == "tex")
        .unwrap();
    assert!(tex.required);
    assert!(tex.aliases.contains(&"tex1"));
    // marble is spectrum-only.
    let marble = schema::implementation(schema::schema("Texture").unwrap(), "marble").unwrap();
    assert!(marble.parameters(Some(TextureValueType::Float)).is_none());
    assert!(
        marble
            .parameters(Some(TextureValueType::Spectrum))
            .is_some()
    );
    // 2D mapping rows ride with the mapping users, not windy.
    let imagemap = schema::implementation(schema::schema("Texture").unwrap(), "imagemap").unwrap();
    assert!(
        imagemap
            .parameters(Some(TextureValueType::Float))
            .unwrap()
            .iter()
            .any(|p| p.name == "uscale")
    );
    let windy = schema::implementation(schema::schema("Texture").unwrap(), "windy").unwrap();
    assert_eq!(
        windy
            .parameters(Some(TextureValueType::Float))
            .unwrap()
            .len(),
        0
    );
    // r4 reads inside (1) and outside (0) as pbrt-v4 does.
    let dots = schema::implementation(schema::schema("Texture").unwrap(), "dots").unwrap();
    let dots_float = dots.parameters(Some(TextureValueType::Float)).unwrap();
    assert_eq!(
        dots_float
            .iter()
            .find(|p| p.name == "inside")
            .unwrap()
            .default,
        Some(schema::DefaultValue::Floats(&[1.]))
    );
    assert_eq!(
        dots_float
            .iter()
            .find(|p| p.name == "outside")
            .unwrap()
            .default,
        Some(schema::DefaultValue::Floats(&[0.]))
    );
    // Aliases from the doc survive transcription.
    let spot = schema::implementation(schema::schema("LightSource").unwrap(), "spot").unwrap();
    let conedelta = spot
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "conedelta")
        .unwrap();
    assert!(conedelta.aliases.contains(&"conedeltaangle"));
}

#[test]
fn the_area_light_accepts_its_r4_implementation_aliases() {
    let table = schema::schema("AreaLightSource").unwrap();
    for name in ["diffuse", "area", ""] {
        assert!(
            schema::implementation(table, name).is_some(),
            "AreaLightSource {name:?} must resolve (canonical name: diffuse)"
        );
    }
    let twosided = schema::implementation(table, "diffuse")
        .unwrap()
        .parameters(None)
        .unwrap()
        .iter()
        .find(|p| p.name == "twosided")
        .unwrap();
    // r4's area light defaults twosided to false (pbrt-v4 says true).
    assert_eq!(
        twosided.default_parameter().unwrap().values,
        ParameterValues::Booleans(vec![false])
    );
}

/// The 2D mapping rows form their own group, and each names the mapping
/// modes that read it (r4's UV / planar mappings; spherical and
/// cylindrical read none). Checkerboard's rows apply only in 2D.
#[test]
fn texture_mapping_rows_form_a_group_with_their_modes() {
    let textures = schema::schema("Texture").unwrap();
    let modes = |spec: &schema::ParameterSpec| {
        spec.when
            .iter()
            .map(|condition| (condition.parameter, condition.values.to_vec()))
            .collect::<Vec<_>>()
    };
    for name in ["imagemap", "checkerboard", "dots", "bilerp"] {
        let implementation = schema::implementation(textures, name).unwrap();
        for value_type in [TextureValueType::Float, TextureValueType::Spectrum] {
            let rows = implementation.parameters(Some(value_type)).unwrap();
            let mapping = rows
                .iter()
                .filter(|spec| spec.group == Some(schema::Group::Mapping))
                .map(|spec| spec.name)
                .collect::<Vec<_>>();
            assert_eq!(
                mapping,
                [
                    "mapping", "uscale", "vscale", "udelta", "vdelta", "v1", "v2"
                ],
                "{name}"
            );
            let dimension = if name == "checkerboard" {
                vec![("dimension", vec!["2"])]
            } else {
                vec![]
            };
            let with = |mut extra: Vec<(&'static str, Vec<&'static str>)>| {
                let mut all = dimension.clone();
                all.append(&mut extra);
                all
            };
            for spec in rows {
                let expected = match spec.name {
                    "mapping" => with(vec![]),
                    "uscale" | "vscale" => with(vec![("mapping", vec!["uv"])]),
                    "udelta" | "vdelta" => with(vec![("mapping", vec!["uv", "planar"])]),
                    "v1" | "v2" => with(vec![("mapping", vec!["planar"])]),
                    _ => {
                        assert_eq!(spec.group, None, "{name}.{}", spec.name);
                        vec![]
                    }
                };
                assert_eq!(modes(spec), expected, "{name}.{}", spec.name);
                if spec.group.is_some() {
                    let choices: &[&str] = if spec.name == "mapping" {
                        &["uv", "spherical", "cylindrical", "planar"]
                    } else {
                        &[]
                    };
                    assert_eq!(spec.choices, choices, "{name}.{}", spec.name);
                }
            }
        }
    }
    for name in ["constant", "fbm", "marble"] {
        let implementation = schema::implementation(textures, name).unwrap();
        let rows = implementation
            .parameters(Some(TextureValueType::Spectrum))
            .unwrap();
        assert!(
            rows.iter()
                .all(|spec| spec.group.is_none() && spec.when.is_empty())
        );
    }
}

/// r4's closed string sets are choices (pbrt-r4 textures/imagemap.rs,
/// textures/checkerboard.rs, media/medium.rs); the color encoding also
/// takes "gamma <value>", so its names are only suggestions.
#[test]
fn closed_string_rows_list_r4_choices() {
    let textures = schema::schema("Texture").unwrap();
    let row = |implementation: &str, name: &str| {
        schema::implementation(textures, implementation)
            .unwrap()
            .parameters(Some(TextureValueType::Spectrum))
            .unwrap()
            .iter()
            .find(|spec| spec.name == name)
            .unwrap()
            .clone()
    };
    assert_eq!(
        row("checkerboard", "aamode").choices,
        ["closedform", "none"]
    );
    assert_eq!(
        row("imagemap", "filter").choices,
        ["point", "bilinear", "trilinear", "ewa"]
    );
    assert_eq!(
        row("imagemap", "wrap").choices,
        ["repeat", "black", "clamp", "octahedralsphere"]
    );
    let encoding = row("imagemap", "encoding");
    assert!(encoding.choices.is_empty());
    assert_eq!(encoding.suggestions, ["sRGB", "linear", "gamma 2.2"]);
    assert!(row("imagemap", "filter").suggestions.is_empty());

    let homogeneous =
        schema::implementation(schema::schema("MakeNamedMedium").unwrap(), "homogeneous").unwrap();
    let preset = homogeneous
        .parameters(None)
        .unwrap()
        .iter()
        .find(|spec| spec.name == "preset")
        .unwrap();
    assert_eq!(preset.choices.len(), 48, "no preset plus r4's 47 names");
    assert_eq!(preset.choices[0], "");
    assert!(preset.choices.contains(&"Skimmilk"));
    assert_eq!(preset.choices[47], "Pacific Ocean Surface Water");
}

/// Every value a choice or suggestion offers exports, under the row's name
/// and each alias: a dropdown must never declare what export refuses.
#[test]
fn every_offered_string_value_exports() {
    let mut checked = 0;
    for (directive, kind) in [
        ("Texture", ResourceKind::Texture),
        ("MakeNamedMedium", ResourceKind::Medium),
    ] {
        for implementation in schema::schema(directive).unwrap() {
            for value_type in [None, Some(TextureValueType::Spectrum)] {
                let Some(rows) = implementation.parameters(value_type) else {
                    continue;
                };
                for spec in rows {
                    for name in std::iter::once(spec.name).chain(spec.aliases.iter().copied()) {
                        for value in spec.choices.iter().chain(spec.suggestions) {
                            let mut document = ProjectDocument::new("offered");
                            let item = resource(
                                kind,
                                implementation.name,
                                vec![parameter(
                                    name,
                                    "string",
                                    ParameterValues::Strings(vec![value.to_string()]),
                                )],
                                value_type,
                            );
                            document.resources.insert(item.id, item.clone());
                            check_exportable(&document, &[], &[item.id]).unwrap_or_else(|error| {
                                panic!("{} {name} {value:?}: {error}", implementation.name)
                            });
                            checked += 1;
                        }
                    }
                }
            }
        }
    }
    assert!(checked > 60, "{checked}");
}
