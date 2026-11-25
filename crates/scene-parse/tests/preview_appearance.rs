//! GeometryManifest v2 appearance records: materials, area lights, lights.
use protocol::GEOMETRY_MANIFEST_VERSION;
use protocol::geometry::EmitterShape;
use scene_parse::{ParseOptions, parse_scene};
use std::fs;

fn parse(content: &str) -> scene_parse::PreviewGeometry {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    fs::write(&path, content).unwrap();
    parse_scene(&path, ParseOptions::default()).unwrap()
}

#[test]
fn lights_carry_their_declaration_transform() {
    let geometry = parse(
        "WorldBegin\nAttributeBegin\nTranslate 1 2 3\nLightSource \"point\" \"rgb I\" [1 1 1]\nAttributeEnd\n",
    );
    let m = geometry.manifest.lights[0].world_from_light;
    assert_eq!([m[3], m[7], m[11]], [1., 2., 3.]);
}

#[test]
fn lights_inside_object_definitions_are_skipped() {
    let geometry = parse("WorldBegin\nObjectBegin \"o\"\nLightSource \"point\"\nObjectEnd\n");
    assert!(geometry.manifest.lights.is_empty());
    assert!(
        geometry
            .diagnostics
            .iter()
            .any(|d| d.message.contains("LightSource"))
    );
}

#[test]
fn film_exposure_follows_scale_and_iso() {
    let exposure = |film: &str| {
        parse(&format!("{film}\nLookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n"))
            .manifest.camera.unwrap().film_exposure
    };
    assert_eq!(
        exposure("Film \"rgb\" \"float iso\" 200 \"float scale\" 2"),
        Some(4.)
    );
    assert_eq!(exposure("Film \"rgb\" \"float iso\" -5"), Some(0.));
    assert_eq!(exposure(""), None);
}

#[test]
fn manifest_carries_materials_area_lights_and_light_sources() {
    let preview = parse(
        "LookAt 0 1 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nLightSource \"point\" \"point from\" [0 2 0] \"rgb intensity\" [4 4 4]\nMakeNamedMaterial \"resource_11111111-1111-1111-1111-111111111111\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.8 0.2 0.1]\nAttributeBegin\nNamedMaterial \"resource_11111111-1111-1111-1111-111111111111\"\nAreaLightSource \"diffuse\" \"rgb emission\" [2 2 2]\nShape \"sphere\"\nAttributeEnd\nAttributeBegin\nMaterial \"metal\" \"float eta\" 1.5\nShape \"disk\"\nAttributeEnd\n",
    );
    let manifest = &preview.manifest;
    assert_eq!(manifest.version, GEOMETRY_MANIFEST_VERSION);
    assert!(preview.diagnostics.is_empty());

    assert_eq!(manifest.materials.len(), 2);
    assert_eq!(manifest.materials[0].implementation, "diffuse");
    assert_eq!(
        manifest.materials[0].resource_id.as_ref().unwrap().0,
        "11111111-1111-1111-1111-111111111111"
    );
    assert_eq!(
        manifest.materials[0].name.as_deref(),
        Some("resource_11111111-1111-1111-1111-111111111111")
    );
    assert_eq!(
        manifest.materials[0].parameters[0].float("reflectance"),
        Some(0.8)
    );
    assert_eq!(manifest.materials[1].implementation, "metal");
    assert_eq!(manifest.materials[1].parameters[0].float("eta"), Some(1.5));
    assert_eq!(manifest.materials[1].resource_id, None);

    assert_eq!(manifest.draws.len(), 2);
    assert_eq!(manifest.draws[0].material_id, Some(0));
    assert_eq!(manifest.draws[1].material_id, Some(1));
    let area_light = manifest.draws[0].area_light.as_ref().unwrap();
    assert_eq!(area_light.implementation, "diffuse");
    assert_eq!(area_light.parameters[0].float("emission"), Some(2.0));
    assert!(manifest.draws[1].area_light.is_none());

    assert_eq!(manifest.lights.len(), 1);
    assert_eq!(manifest.lights[0].implementation, "point");
    assert_eq!(manifest.lights[0].parameters[0].name, "from");
    assert_eq!(manifest.lights[0].parameters[0].floats, vec![0., 2., 0.]);
    assert_eq!(manifest.lights[0].parameters[1].floats, vec![4., 4., 4.]);
}

#[test]
fn identical_material_definitions_share_one_record() {
    let preview = parse(
        "WorldBegin\nMakeNamedMaterial \"shared\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nAttributeBegin\nNamedMaterial \"shared\"\nShape \"sphere\"\nAttributeEnd\nAttributeBegin\nNamedMaterial \"shared\"\nShape \"disk\"\nAttributeEnd\n",
    );
    assert_eq!(preview.manifest.materials.len(), 1);
    assert_eq!(
        preview.manifest.materials[0].name.as_deref(),
        Some("shared")
    );
    assert_eq!(preview.manifest.draws[0].material_id, Some(0));
    assert_eq!(preview.manifest.draws[1].material_id, Some(0));
    assert!(preview.manifest.materials[0].resource_id.is_none());
}

#[test]
fn named_material_defined_after_its_use_still_resolves() {
    let preview = parse(
        "WorldBegin\nAttributeBegin\nNamedMaterial \"later\"\nShape \"sphere\"\nAttributeEnd\nMakeNamedMaterial \"later\" \"string type\" \"metal\" \"float eta\" 2.0\n",
    );
    assert_eq!(preview.manifest.materials.len(), 1);
    assert_eq!(preview.manifest.materials[0].implementation, "metal");
    assert_eq!(preview.manifest.draws[0].material_id, Some(0));
    assert!(preview.diagnostics.is_empty());
}

#[test]
fn unresolved_named_material_reports_a_diagnostic_without_a_material_id() {
    let preview = parse(
        "WorldBegin\nAttributeBegin\nNamedMaterial \"missing\"\nShape \"sphere\"\nAttributeEnd\n",
    );
    assert_eq!(preview.manifest.draws[0].material_id, None);
    assert!(preview.manifest.materials.is_empty());
    assert_eq!(preview.diagnostics[0].code, "named_material_unresolved");
    assert_eq!(preview.diagnostics[0].line, Some(4));
}

#[test]
fn only_an_explicit_interface_material_marks_the_draw_invisible() {
    let preview = parse(
        "WorldBegin\nShape \"sphere\"\nMaterial \"interface\"\nShape \"sphere\"\nMaterial \"\"\nShape \"sphere\"\n",
    );
    assert_eq!(preview.manifest.draws.len(), 3);
    assert!(!preview.manifest.draws[0].interface);
    assert!(preview.manifest.draws[1].interface);
    assert!(preview.manifest.draws[2].interface);
    assert_eq!(preview.manifest.draws[0].material_id, None);
    assert_eq!(preview.manifest.draws[1].material_id, None);
    assert_eq!(preview.manifest.draws[2].material_id, None);
}

#[test]
fn area_light_without_a_preceding_shape_is_reported() {
    let preview = parse(
        "AttributeBegin\nAreaLightSource \"diffuse\" \"rgb emission\" [1 1 1]\nAttributeEnd\n",
    );
    assert!(preview.manifest.draws.is_empty());
    assert_eq!(preview.diagnostics[0].code, "area_light_without_shape");
}

#[test]
fn canonical_area_light_declaration_attaches_to_the_following_shape() {
    let preview = parse(
        "WorldBegin\nAreaLightSource \"diffuse\" \"rgb L\" [2 1 0.5]\nShape \"trianglemesh\" \"point3 P\" [-1 -1 0 1 -1 0 1 1 0] \"integer indices\" [0 1 2]\n",
    );
    assert_eq!(preview.manifest.draws.len(), 1);
    let area = preview.manifest.draws[0].area_light.as_ref().unwrap();
    assert_eq!(area.implementation, "diffuse");
    assert_eq!(area.parameters[0].floats, vec![2., 1., 0.5]);
}

#[test]
fn oversized_parameter_values_are_clipped_and_flagged() {
    let values = vec!["0.5"; 70].join(" ");
    let preview = parse(&format!(
        "WorldBegin\nAttributeBegin\nMaterial \"diffuse\" \"rgb reflectance\" [{values}]\nShape \"sphere\"\nAttributeEnd\n",
    ));
    let parameter = &preview.manifest.materials[0].parameters[0];
    assert_eq!(parameter.floats.len(), 64);
    assert!(parameter.truncated);
    assert!(preview.manifest.draws[0].material_id.is_some());
}

#[test]
fn manifest_carries_texture_records_and_imagemap_payloads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("scene.pbrt");
    // Two solid halves: red left, green right; the lit preview samples it.
    let mut image = image::RgbImage::new(2, 1);
    image.put_pixel(0, 0, image::Rgb([255, 0, 0]));
    image.put_pixel(1, 0, image::Rgb([0, 255, 0]));
    image.save(dir.path().join("strip.png")).unwrap();
    fs::write(
        &path,
        "WorldBegin\nTexture \"strip\" \"spectrum\" \"imagemap\" \"string filename\" \"strip.png\"\nMakeNamedMaterial \"m\" \"string type\" \"diffuse\" \"texture reflectance\" [strip]\nNamedMaterial \"m\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    let manifest = &preview.manifest;
    assert!(preview.diagnostics.is_empty());
    assert_eq!(manifest.textures.len(), 1);
    assert_eq!(manifest.textures[0].name, "strip");
    assert_eq!(manifest.textures[0].value_type, "spectrum");
    assert_eq!(manifest.textures[0].implementation, "imagemap");
    assert!(manifest.textures[0].parameters[0].matches("filename"));
    // The decoded payload is keyed by type as float and spectrum names are
    // separate in pbrt.
    let entry = manifest.blobs.get("texture/spectrum/strip").unwrap();
    assert_eq!(
        entry.size,
        fs::metadata(dir.path().join("strip.png")).unwrap().len()
    );
    assert!(preview.blobs.contains_key("texture/spectrum/strip"));
    // The draw's material keeps the texture-name binding in its texts.
    assert_eq!(manifest.draws[0].material_id, Some(0));
    let binding = manifest.materials[0]
        .parameters
        .iter()
        .find(|parameter| parameter.value_type == "texture")
        .unwrap();
    assert_eq!(binding.name, "reflectance");
    assert_eq!(binding.texts, vec!["strip".to_owned()]);

    // Constant textures ride without a payload; missing imagaps warn.
    fs::write(
        &path,
        "WorldBegin\nTexture \"gone\" \"spectrum\" \"imagemap\" \"string filename\" \"missing.png\"\n",
    )
    .unwrap();
    let preview = parse_scene(&path, ParseOptions::default()).unwrap();
    assert!(preview.manifest.textures.iter().any(|t| t.name == "gone"));
    assert!(!preview.blobs.contains_key("texture/gone"));
    assert!(
        preview
            .diagnostics
            .iter()
            .any(|d| d.code == "texture_failed")
    );
}

#[test]
fn textures_keep_type_namespace_declaration_transform_and_shape_alpha() {
    let geometry = parse(
        "WorldBegin\nTexture \"same\" \"float\" \"constant\" \"float value\" 0.25\nAttributeBegin\nTranslate 7 8 9\nTexture \"same\" \"spectrum\" \"constant\" \"rgb value\" [0.1 0.2 0.3]\nShape \"sphere\" \"texture alpha\" [same]\nAttributeEnd\n",
    );
    assert_eq!(geometry.manifest.textures.len(), 2);
    let float = geometry
        .manifest
        .textures
        .iter()
        .find(|t| t.value_type == "float")
        .unwrap();
    let spectrum = geometry
        .manifest
        .textures
        .iter()
        .find(|t| t.value_type == "spectrum")
        .unwrap();
    assert_eq!(
        [
            float.world_from_texture[3],
            float.world_from_texture[7],
            float.world_from_texture[11]
        ],
        [0.0; 3]
    );
    assert_eq!(
        [
            spectrum.world_from_texture[3],
            spectrum.world_from_texture[7],
            spectrum.world_from_texture[11]
        ],
        [7.0, 8.0, 9.0]
    );
    assert_eq!(
        geometry.manifest.draws[0].alpha.as_ref().unwrap().name,
        "alpha"
    );
    assert_eq!(
        geometry.manifest.draws[0].alpha.as_ref().unwrap().texts,
        ["same"]
    );
}

/// pbrt keeps an AreaLightSource in the graphics state: every Shape after it
/// in the same attribute scope emits, and nothing outside the scope does.
#[test]
fn area_light_applies_to_every_following_shape_in_its_scope() {
    let preview = parse(
        "WorldBegin\nAttributeBegin\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"sphere\"\nShape \"disk\"\nAttributeEnd\nShape \"sphere\"\n",
    );
    let emits: Vec<bool> = preview
        .manifest
        .draws
        .iter()
        .map(|draw| draw.area_light.is_some())
        .collect();
    assert_eq!(emits, [true, true, false]);
}

#[test]
fn full_sphere_and_disk_emitters_carry_their_shape() {
    let p = parse(
        "WorldBegin\nAttributeBegin\nAreaLightSource \"diffuse\"\nShape \"sphere\" \"float radius\" 2\nShape \"disk\" \"float radius\" 3 \"float height\" 1\nShape \"sphere\" \"float zmax\" 0.5\nAttributeEnd\nShape \"sphere\"\n",
    );
    let shapes: Vec<_> = p
        .manifest
        .draws
        .iter()
        .map(|d| d.emitter_shape.clone())
        .collect();
    assert_eq!(
        shapes,
        [
            Some(EmitterShape::Sphere { radius: 2. }),
            Some(EmitterShape::Disk {
                radius: 3.,
                height: 1.
            }),
            None,
            None,
        ]
    );
}

/// An AreaLightSource after a Shape does not light that Shape.
#[test]
fn area_light_after_a_shape_does_not_light_it() {
    let preview = parse(
        "WorldBegin\nAttributeBegin\nShape \"sphere\"\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nAttributeEnd\n",
    );
    assert!(preview.manifest.draws[0].area_light.is_none());
    assert_eq!(preview.diagnostics[0].code, "area_light_without_shape");
}

/// A `mix` names its children; they must ride in the manifest even when no
/// shape binds them directly, or the preview cannot blend them.
#[test]
fn mix_children_ride_in_the_manifest() {
    let preview = parse(
        "WorldBegin\nMakeNamedMaterial \"red\" \"string type\" \"diffuse\" \"rgb reflectance\" [1 0 0]\nMakeNamedMaterial \"blue\" \"string type\" \"diffuse\" \"rgb reflectance\" [0 0 1]\nMakeNamedMaterial \"both\" \"string type\" \"mix\" \"string materials\" [\"red\" \"blue\"]\nNamedMaterial \"both\"\nShape \"sphere\"\n",
    );
    let names: Vec<_> = preview
        .manifest
        .materials
        .iter()
        .map(|m| m.name.clone().unwrap_or_default())
        .collect();
    assert_eq!(names, ["both", "red", "blue"]);
    let bound = preview.manifest.draws[0].material_id.unwrap() as usize;
    assert_eq!(preview.manifest.materials[bound].implementation, "mix");
}

/// r4 keeps a separate mask for shadow rays; the preview does not, and says so
/// rather than quietly merging it into alpha.
#[test]
fn shadowalpha_is_reported_and_not_applied() {
    let preview =
        parse("WorldBegin\nShape \"sphere\" \"float alpha\" [0.5] \"float shadowalpha\" [0.25]\n");
    assert!(
        preview
            .diagnostics
            .iter()
            .any(|d| d.code == "shadowalpha_unsupported"),
        "{:?}",
        preview.diagnostics
    );
    assert_eq!(
        preview.manifest.draws[0].alpha.as_ref().unwrap().floats,
        [0.5]
    );
}
