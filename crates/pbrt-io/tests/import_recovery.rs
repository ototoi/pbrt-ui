//! Import skips what it cannot interpret, records a warning for each skipped
//! piece, and keeps the rest of the scene.
use pbrt_io::{ImportOptions, import_project};
use project::*;
use std::{fs, path::Path};

struct Imported {
    session: ProjectSession,
    diagnostics: Vec<ImportDiagnostic>,
    _home: tempfile::TempDir,
}

fn import_text(text: &str) -> Imported {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("scene.pbrt"), text).unwrap();
    import_in(home)
}

fn import_in(home: tempfile::TempDir) -> Imported {
    let (_, session) = import_project(
        &home.path().join("scene.pbrt"),
        &home.path().join("project"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let diagnostics = session
        .document()
        .imports
        .values()
        .flat_map(|import| import.diagnostics.clone())
        .collect();
    Imported {
        session,
        diagnostics,
        _home: home,
    }
}

fn scene(doc: &ProjectDocument) -> &SceneDocument {
    doc.scene(doc.default_scene.unwrap()).unwrap()
}

impl Imported {
    fn document(&self) -> &ProjectDocument {
        self.session.document()
    }
    /// Shape implementations reachable from the world, in order.
    fn shapes(&self) -> Vec<String> {
        self.entities(EntityKind::Shape)
            .into_iter()
            .map(|(_, implementation)| implementation)
            .collect()
    }
    fn entities(&self, kind: EntityKind) -> Vec<(NodeId, String)> {
        fn visit(
            doc: &ProjectDocument,
            id: NodeId,
            kind: EntityKind,
            out: &mut Vec<(NodeId, String)>,
        ) {
            let view = doc.default_scene_view().unwrap();
            let Some(node) = view.node(id) else {
                return;
            };
            match &node.data {
                NodeKind::Scope { children, .. } => {
                    for child in children {
                        visit(doc, *child, kind, out);
                    }
                }
                NodeKind::Entity {
                    entity,
                    implementation,
                    ..
                } if *entity == kind => out.push((id, implementation.clone())),
                _ => {}
            }
        }
        let mut out = vec![];
        let doc = self.document();
        let scene = scene(doc);
        for id in scene.options.iter().chain([&scene.world]) {
            visit(doc, *id, kind, &mut out);
        }
        out
    }
    fn has(&self, code: &str, text: &str) -> bool {
        self.diagnostics
            .iter()
            .any(|d| d.code == code && d.message.contains(text))
    }
}

#[test]
fn test_import_skips_unknown_directives_with_warnings() {
    let imported = import_text(
        "Option \"msereferenceimage\" \"outside.exr\"\nOption \"bool disablepixeljitter\" true\nShape \"sphere\"\nReverseOrientation\nWorldBegin\nTransformTimes 1 2\nObjectBegin \"o\"\nObjectInstance \"o\"\nObjectEnd\nCamera \"perspective\"\nUnknownDirective \"x\"\nAttribute \"shape\" \"float radius\" [2]\nShape \"plymesh\" \"string filename\" [\"missing.ply\"]\nShape \"teapot\"\nCoordSysTransform \"nowhere\"\nNamedMaterial \"later\"\nMakeNamedMaterial \"later\" \"string type\" [\"diffuse\"]\nInclude \"missing.pbrt\"\nShape \"disk\"\n",
    );
    assert_eq!(imported.shapes(), ["disk"]);
    assert!(imported.entities(EntityKind::Camera).is_empty());
    for (code, text) in [
        ("unsupported", "Option"),
        ("invalid", "Shape is only allowed after WorldBegin"),
        ("invalid", "Camera is only allowed before WorldBegin"),
        ("unsupported", "UnknownDirective"),
        ("unsupported", "Attribute"),
        ("invalid", "missing.ply"),
        ("unsupported", "teapot"),
        ("invalid", "unknown coordinate system nowhere"),
        ("unresolved_include", "missing.pbrt"),
        (
            "invalid",
            "TransformTimes is only allowed before WorldBegin",
        ),
        (
            "invalid",
            "ReverseOrientation is only allowed after WorldBegin",
        ),
        ("unsupported", "ObjectInstance inside an object definition"),
    ] {
        assert!(imported.has(code, text), "no {code} warning about {text}");
    }
    // The typed pbrt-v4 Option form is accepted as the bare r4 name.
    let doc = imported.document();
    let scene = scene(doc);
    assert!(scene.options.iter().any(|id| matches!(
        &scene.nodes[id].data,
        NodeKind::Option { name, .. } if name == "disablepixeljitter"
    )));
    // The material the reference names is still imported - pbrt-v4 binds
    // NamedMaterial late, so a definition after the reference binds it
    // without a warning.
    assert!(
        doc.resources
            .values()
            .any(|r| r.label == "later" && r.kind == ResourceKind::Material)
    );
    // Every skipped piece points back at its source line.
    for diagnostic in &imported.diagnostics {
        assert!(diagnostic.origin.as_ref().is_some_and(|o| o.span.line > 0));
    }
}

#[test]
fn test_skipped_declarations_leave_no_copied_assets_behind() {
    let home = tempfile::tempdir().unwrap();
    fs::write(home.path().join("albedo.exr"), b"texture bytes").unwrap();
    fs::write(
        home.path().join("scene.pbrt"),
        "WorldBegin\nTexture \"x\" \"spectrum\" \"imagemap\" \"string filename\" [\"albedo.exr\"] \"string surprise\" [\"value\"]\nShape \"sphere\"\n",
    )
    .unwrap();
    let imported = import_in(home);
    assert!(imported.has("unsupported", "surprise"));
    assert!(imported.document().assets.is_empty());
    assert!(imported.document().resources.is_empty());
    assert_eq!(imported.shapes(), ["sphere"]);
}

#[test]
fn test_import_resyncs_after_malformed_directive() {
    let imported = import_text(
        "WorldBegin\nShape 42 \"sphere\"\nShape \"cylinder\"\nTranslate 1 0\nShape \"disk\"\n] 7\nShape \"cone\"\n",
    );
    assert_eq!(imported.shapes(), ["cylinder", "disk", "cone"]);
    let syntax = imported
        .diagnostics
        .iter()
        .filter(|d| d.code == "syntax")
        .map(|d| d.origin.as_ref().unwrap().span.line)
        .collect::<Vec<_>>();
    assert_eq!(syntax, [2, 4, 6]);
}

#[test]
fn test_import_closes_unbalanced_scopes_with_warnings() {
    let imported = import_text(
        "WorldBegin\nAttributeEnd\nAttributeBegin\nTranslate 1 0 0\nShape \"sphere\"\nTransformEnd\n",
    );
    assert_eq!(imported.shapes(), ["sphere"]);
    assert!(imported.has("invalid", "AttributeEnd"));
    assert!(imported.has("invalid", "TransformEnd"));
    assert!(imported.has("unbalanced_scope", "Attribute"));

    let at_world_end = import_text("WorldBegin\nAttributeBegin\nShape \"sphere\"\nWorldEnd\n");
    assert_eq!(at_world_end.shapes(), ["sphere"]);
    assert!(at_world_end.has("unbalanced_scope", "WorldEnd"));

    let no_world = import_text("Camera \"perspective\"\n");
    assert!(no_world.has("missing_world", "WorldBegin"));
    assert!(no_world.shapes().is_empty());
}

#[test]
fn test_skipped_scope_discards_its_contents() {
    let imported = import_text(
        "WorldBegin\nObjectBegin \"a\"\nShape \"sphere\"\nObjectEnd\nObjectBegin \"a\"\nShape \"disk\"\nAttributeBegin\nShape \"cone\"\nAttributeEnd\nObjectEnd\nObjectInstance \"a\"\nShape \"cylinder\"\n",
    );
    // r4 keeps the first definition; the second one is skipped whole, so its
    // shapes do not leak into the world.
    assert!(imported.has("invalid", "duplicate object name"));
    assert!(imported.has("invalid", "5 directive(s)"));
    let doc = imported.document();
    let definition = doc
        .resources
        .values()
        .find(|r| r.kind == ResourceKind::ObjectDefinition)
        .unwrap();
    let NodeKind::Scope { children, .. } =
        &doc.definition_nodes[&definition.definition_scope.unwrap()].data
    else {
        unreachable!()
    };
    assert_eq!(children.len(), 1);
    // Shapes of a definition are reached through its instances, not the world.
    assert_eq!(imported.shapes(), ["cylinder"]);
}

#[test]
fn test_redefined_named_resources_follow_r4_last_wins() {
    let imported = import_text(
        "WorldBegin\nMakeNamedMaterial \"m\" \"string type\" [\"diffuse\"]\nNamedMaterial \"m\"\nShape \"sphere\"\nMakeNamedMaterial \"m\" \"string type\" [\"conductor\"]\nNamedMaterial \"m\"\nShape \"disk\"\nTexture \"t\" \"float\" \"constant\"\nTexture \"t\" \"spectrum\" \"constant\"\nMaterial \"diffuse\" \"texture reflectance\" [\"t\"]\nShape \"cone\"\n",
    );
    assert_eq!(imported.shapes(), ["sphere", "disk", "cone"]);
    let doc = imported.document();
    let view = doc.default_scene_view().unwrap();
    let evaluated = evaluate(&view).unwrap();
    let material = |index: usize| {
        let id = imported.entities(EntityKind::Shape)[index].0;
        match &evaluated.nodes[&id].material {
            MaterialBinding::Resource(id) => doc.resources[id].implementation.clone(),
            other => format!("{other:?}"),
        }
    };
    assert_eq!(material(0), "diffuse");
    assert_eq!(material(1), "conductor");
    // r4 keeps float and spectrum textures apart, so both exist and a
    // reference by the shared name is ambiguous.
    assert_eq!(
        doc.resources
            .values()
            .filter(|r| r.kind == ResourceKind::Texture)
            .count(),
        2
    );
    assert!(imported.has("unsupported", "both float and spectrum"));
    assert_eq!(material(2), "conductor");
    assert!(
        !imported
            .diagnostics
            .iter()
            .any(|d| d.message.contains("MakeNamedMaterial"))
    );
}

#[test]
fn test_include_cycles_are_skipped_with_a_warning() {
    let home = tempfile::tempdir().unwrap();
    fs::write(
        home.path().join("scene.pbrt"),
        "WorldBegin\nInclude \"child.pbrt\"\nShape \"disk\"\n",
    )
    .unwrap();
    fs::write(
        Path::new(home.path()).join("child.pbrt"),
        "Shape \"sphere\"\nInclude \"scene.pbrt\"\n",
    )
    .unwrap();
    let imported = import_in(home);
    assert_eq!(imported.shapes(), ["sphere", "disk"]);
    assert!(imported.has("unresolved_include", "include cycle"));
}

#[test]
fn texture_color_and_rgb_value_types_import_as_spectrum() {
    let imported = import_text(concat!(
        "WorldBegin\n",
        "Texture \"c\" \"color\" \"constant\"\n",
        "Texture \"g\" \"rgb\" \"constant\"\n",
        "Material \"diffuse\" \"texture reflectance\" [\"c\"]\n",
        "Shape \"sphere\"\n",
    ));
    assert_eq!(imported.shapes(), ["sphere"]);
    let doc = imported.document();
    let textures = doc
        .resources
        .values()
        .filter(|r| r.kind == ResourceKind::Texture)
        .collect::<Vec<_>>();
    assert_eq!(
        textures.len(),
        2,
        "both textures import instead of skipping"
    );
    for label in ["c", "g"] {
        let texture = textures
            .iter()
            .find(|r| r.label == label)
            .unwrap_or_else(|| panic!("missing texture {label}"));
        assert_eq!(texture.texture_value_type, Some(TextureValueType::Spectrum));
    }
    assert!(!imported.has("unsupported", "texture value type"));
    // Export normalizes the value type to "spectrum", which r4 reads the
    // same way (r4's parser: `"color" | "rgb" | "spectrum"`).
    let store =
        ProjectStore::Folder(FolderStore::open(&imported._home.path().join("project")).unwrap());
    let path = pbrt_io::export_project(
        &imported.session.snapshot(),
        &store,
        &imported._home.path().join("export"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(path).unwrap();
    assert_eq!(
        text.matches("\"spectrum\" \"constant\"").count(),
        2,
        "exported textures use the spectrum value type"
    );
}

#[test]
fn r4s_vector_and_normal_type_spellings_import_like_vector3_and_normal3() {
    // r4 accepts "vector" / "normal" as aliases of "vector3" / "normal3",
    // and the parameter table's defaults use the r4 spelling — files
    // exported from table values must re-import instead of dropping the
    // whole directive with a parameter-type diagnostic (FX4 review finding).
    let imported = import_text(
        "WorldBegin\n\
         Texture \"t\" \"float\" \"directionmix\" \"float tex1\" [0] \"float tex2\" [1] \
         \"vector dir\" [0 1 0]\n\
         Material \"diffuse\" \"texture reflectance\" \"t\"\n\
         Shape \"sphere\" \"float radius\" [1]\n",
    );
    assert!(
        !imported
            .diagnostics
            .iter()
            .any(|d| d.message.contains("parameter type vector")),
        "the vector spelling must import: {:?}",
        imported.diagnostics
    );
    let document = imported.session.document();
    let directionmix = document
        .resources
        .values()
        .find(|resource| resource.implementation == "directionmix")
        .expect("the directionmix texture must survive import");
    let dir = directionmix
        .parameters
        .iter()
        .find(|p| p.name == "dir")
        .expect("dir parameter");
    // The declared r4 spelling is preserved, not normalized away.
    assert_eq!(dir.value_type, "vector");
    let ParameterValues::Floats(values) = &dir.values else {
        panic!("vector values parse as floats");
    };
    assert_eq!(values.len(), 3);
}
