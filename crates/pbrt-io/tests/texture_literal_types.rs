use project::*;

#[test]
fn changing_constant_lineage_rejects_incompatible_literals_in_both_directions() {
    for (original, destination, value_type, values) in [
        (
            TextureValueType::Spectrum,
            TextureValueType::Float,
            "rgb",
            vec![0.2, 0.3, 0.4],
        ),
        (
            TextureValueType::Float,
            TextureValueType::Spectrum,
            "float",
            vec![0.5],
        ),
    ] {
        let mut document = ProjectDocument::new_empty("Literal types");
        let id = ResourceId::new();
        let mut parameter = Parameter::floats("value", values);
        parameter.value_type = value_type.into();
        document.resources.insert(
            id,
            Resource {
                id,
                label: "Constant".into(),
                location: "Constant.texture".into(),
                edition: Revision::initial(),
                kind: ResourceKind::Texture,
                texture_value_type: Some(original),
                implementation: "constant".into(),
                parameters: vec![parameter],
                definition_scope: None,
                declared_in: None,
                origin: None,
            },
        );
        assert!(pbrt_io::texture_type_mismatches(&document).is_empty());
        document.resources.get_mut(&id).unwrap().texture_value_type = Some(destination);
        let errors = pbrt_io::texture_type_mismatches(&document);
        assert!(
            errors
                .iter()
                .any(|error| error["reason"] == "literal_type_mismatch"),
            "{errors:?}"
        );
    }
}
