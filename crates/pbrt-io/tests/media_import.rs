//! Participating media survive import and export (cloud / nanovdb /
//! uniformgrid / rgbgrid). The importer's dependency schema is the only
//! gate widened; the parameter table stays homogeneous-only (typed
//! Inspector rows and GUI creation of grid media are not implemented yet), so grid
//! media remain free parameters.
use pbrt_io::parameter_schema as schema;
use pbrt_io::{ImportOptions, export_project, import_project};
use project::*;
use std::{collections::BTreeMap, fs, path::Path};

fn import(
    source: &Path,
    destination: &Path,
) -> (
    project::ProjectStore,
    project::ProjectSession,
    pbrt_io::ImportReport,
) {
    pbrt_io::import_project_with_report(source, destination, &ImportOptions::default(), || false)
        .unwrap()
}

fn medium_of<'a>(document: &'a ProjectDocument, label: &str) -> &'a Resource {
    document
        .resources
        .values()
        .find(|resource| resource.kind == ResourceKind::Medium && resource.label == label)
        .unwrap_or_else(|| panic!("no medium named {label}"))
}

/// cloud / nanovdb (a fake `.nvdb`; the importer never reads its content) /
/// uniformgrid (4097 density values, so the array takes the `.array` asset
/// path) import into the project and export back with the directives r4
/// needs: the declaration with its declared transform, the shape's
/// `MediumInterface`, and the nvdb asset copied into the bundle.
#[test]
fn grid_media_roundtrip_from_import_to_export() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("grid.nvdb"),
        b"not a real nanovdb payload: import only stages the bytes",
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    let density = std::iter::repeat_n("1", 4097).collect::<Vec<_>>().join(" ");
    fs::write(
        &source,
        format!(
            "Rotate 90 1 0 0\n\
             MakeNamedMedium \"c\" \"string type\" [\"cloud\"] \
             \"spectrum sigma_s\" [400 2 500 3] \"spectrum sigma_a\" [400 0.1 500 0.1] \
             \"float density\" [2]\n\
             MakeNamedMedium \"foo\" \"string type\" [\"nanovdb\"] \
             \"string filename\" [\"grid.nvdb\"] \"string gridname\" [\"density_fog\"] \
             \"string temperaturename\" [\"temperature_fog\"] \"float g\" [0.9]\n\
             Rotate 90 -1 0 0\n\
             MakeNamedMedium \"smoke\" \"string type\" [\"uniformgrid\"] \
             \"float density\" [{density}] \"point3 p0\" [-1 -1 -1] \"point3 p1\" [1 1 1] \
             \"integer nx\" [8] \"integer ny\" [8] \"integer nz\" [8]\n\
             Camera \"perspective\"\n\
             WorldBegin\n\
             Material \"interface\"\n\
             MediumInterface \"c\" \"\"\n\
             Shape \"sphere\" \"float radius\" [1]\n\
             WorldEnd\n"
        ),
    )
    .unwrap();
    let (store, session, report) = import(&source, &temp.path().join("project"));
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("medium is dropped")),
        "the media must import: {:?}",
        report.diagnostics
    );

    // The project model keeps each implementation and the nvdb as an asset.
    let document = session.document();
    let cloud = medium_of(document, "c");
    assert_eq!(cloud.implementation, "cloud");
    let nanovdb = medium_of(document, "foo");
    assert_eq!(nanovdb.implementation, "nanovdb");
    let filename = nanovdb
        .parameters
        .iter()
        .find(|parameter| parameter.name == "filename")
        .unwrap();
    assert!(matches!(&filename.values, ParameterValues::Assets(assets)
            if assets.len() == 1
                && document.assets[&assets[0]].location.ends_with("grid.nvdb")));
    let gridname = nanovdb
        .parameters
        .iter()
        .find(|parameter| parameter.name == "gridname")
        .unwrap();
    assert_eq!(
        &gridname.values,
        &ParameterValues::Strings(vec!["density_fog".into()])
    );
    let smoke = medium_of(document, "smoke");
    assert_eq!(smoke.implementation, "uniformgrid");
    let values = smoke
        .parameters
        .iter()
        .find(|parameter| parameter.name == "density")
        .unwrap();
    assert!(
        matches!(&values.values, ParameterValues::Array { count: 4097, .. }),
        "a 4097-value density becomes a bulk array asset: {:?}",
        values.values
    );

    // Export: declarations with their declared transform, the shape's
    // MediumInterface, the nvdb asset, and the expanded density.
    let export = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&export).unwrap();
    let transform = text.find("Transform [").expect("the declared transform");
    let cloud_at = text.find("MakeNamedMedium \"c\"").expect("the cloud");
    assert!(transform < cloud_at, "{text}");
    assert!(text.contains("\"string type\" [\"cloud\"]"), "{text}");
    let interface = text
        .find("MediumInterface \"c\" \"\"")
        .expect("the shape's medium interface");
    let sphere = text.find("Shape \"sphere\"").expect("the sphere");
    assert!(interface < sphere, "{text}");
    assert!(text.contains("MakeNamedMedium \"foo\""), "{text}");
    assert!(text.contains("\"string type\" [\"nanovdb\"]"), "{text}");
    assert!(
        text.contains("\"string gridname\" [ \"density_fog\" ]")
            && text.contains("\"string temperaturename\" [ \"temperature_fog\" ]"),
        "{text}"
    );
    assert!(text.contains("grid.nvdb"), "{text}");
    let assets = fs::read_dir(temp.path().join("bundle/assets"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    assert!(
        assets.iter().any(|name| name.ends_with("grid.nvdb")),
        "the nvdb asset must be copied into the bundle: {assets:?}"
    );
    assert!(text.contains("MakeNamedMedium \"smoke\""), "{text}");
    assert!(text.contains("\"string type\" [\"uniformgrid\"]"), "{text}");

    // The exported text parses back with the same bulk density and asset.
    let (copy_store, copy) = import_project(
        &export,
        &temp.path().join("copy"),
        &ImportOptions::default(),
        || false,
    )
    .unwrap();
    let copy_smoke = medium_of(copy.document(), "smoke");
    let values = copy_smoke
        .parameters
        .iter()
        .find(|parameter| parameter.name == "density")
        .unwrap();
    assert!(
        matches!(&values.values, ParameterValues::Array { count: 4097, .. }),
        "the exported density round trips: {:?}",
        values.values
    );
    let copy_nanovdb = medium_of(copy.document(), "foo");
    let filename = copy_nanovdb
        .parameters
        .iter()
        .find(|parameter| parameter.name == "filename")
        .unwrap();
    let ParameterValues::Assets(assets) = &filename.values else {
        panic!("the nvdb asset must survive the round trip");
    };
    let asset = assets[0];
    let imported_hash = session
        .document()
        .assets
        .values()
        .find(|a| a.location.ends_with("grid.nvdb"))
        .map(|a| a.sha256.clone())
        .unwrap_or_default();
    let copy_hash = &copy.document().assets[&asset].sha256;
    assert_eq!(&imported_hash, copy_hash, "the same nvdb bytes");
    let _ = copy_store;
}

/// `.nvdb` grid payloads belong with the media:
/// `Media/`, original basename, media type `application/x-nanovdb`). The
/// direct import, the batch re-placement and the legacy-v1 conversion must
/// agree; this is the first two.
#[test]
fn nvdb_assets_land_in_the_media_folder() {
    let temp = tempfile::tempdir().unwrap();
    let bytes = b"not a real nanovdb payload: import only stages the bytes";
    fs::write(temp.path().join("grid.nvdb"), bytes).unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "MakeNamedMedium \"foo\" \"string type\" [\"nanovdb\"] \"string filename\" [\"grid.nvdb\"]\n\
         Camera \"perspective\"\n\
         WorldBegin\n\
         MediumInterface \"foo\" \"\"\n\
         Shape \"sphere\" \"float radius\" [1]\n\
         WorldEnd\n",
    )
    .unwrap();
    let (_store, session, report) = import(&source, &temp.path().join("project"));
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("medium is dropped")),
        "{:?}",
        report.diagnostics
    );
    let document = session.document();
    let asset = document
        .assets
        .values()
        .find(|asset| asset.location.ends_with("grid.nvdb"))
        .expect("the staged nvdb asset");
    assert_eq!(asset.media_type, "application/x-nanovdb");
    assert_eq!(asset.location, "Media/grid.nvdb");

    // The batch import's placement follows the same media type mapping
    // (project/src/import_batch.rs) instead of dropping nvdb files in
    // Other/ with every other unrecognized byte stream.
    let (prepared, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    let target = ProjectDocument::new_empty("Target");
    let placed = prepared.placed_for(&target, None).unwrap();
    let placed_asset = placed
        .document()
        .assets
        .values()
        .find(|asset| asset.location.ends_with("grid.nvdb"))
        .expect("the nvdb in the batch");
    assert_eq!(placed_asset.media_type, "application/x-nanovdb");
    assert_eq!(placed_asset.location, "Media/grid.nvdb");
}

/// S6 (smoke-plume): pbrt-v4 and r4 read the token stream spliced across
/// `Include`, so parameters following an `Include` continue the last
/// directive the included file produced. A MakeNamedMedium's parameter
/// list can continue in the parent file after the Include that pulled in
/// the medium's bulk arrays.
#[test]
fn include_continues_the_last_directive_of_the_included_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("density.pbrt"),
        "MakeNamedMedium \"smoke\" \"string type\" [\"uniformgrid\"] \"float density\" [0 0.5 0.5 1] \"integer nx\" [2]\n",
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "Include \"density.pbrt\" \"spectrum sigma_a\" [400 0.01 500 0.01] \"spectrum sigma_s\" [400 0.9 500 0.9]\n\
         Camera \"perspective\"\n\
         WorldBegin\n\
         MediumInterface \"smoke\" \"\"\n\
         Shape \"sphere\" \"float radius\" [1]\n\
         WorldEnd\n",
    )
    .unwrap();
    let (store, session, report) = import(&source, &temp.path().join("project"));
    assert!(
        !report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("medium is dropped")
            || diagnostic.message.contains("smoke")),
        "{:?}",
        report.diagnostics
    );
    let smoke = medium_of(session.document(), "smoke");
    let names = smoke
        .parameters
        .iter()
        .map(|parameter| parameter.name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["density", "nx", "sigma_a", "sigma_s"]);
    let export = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&export).unwrap();
    let medium = text
        .lines()
        .find(|line| line.starts_with("MakeNamedMedium \"smoke\""))
        .expect("the continued medium");
    assert!(medium.contains("\"float density\""), "{medium}");
    assert!(medium.contains("\"spectrum sigma_a\""), "{medium}");
    assert!(medium.contains("\"spectrum sigma_s\""), "{medium}");
}

/// An Include with trailing parameters and no directive to continue
/// (the included file produced nothing) still skips with
/// `unresolved_include`, naming the reason; the rest of the scene
/// imports. A missing target keeps reporting the missing path, not the
/// parameter shape.
#[test]
fn include_parameters_without_a_directive_to_continue_skip_the_include() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("empty.pbrt"), "# nothing to declare\n").unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "Include \"empty.pbrt\" \"float x\" [1]\n\
         Camera \"perspective\"\n\
         WorldBegin\n\
         Include \"missing.pbrt\" \"float y\" [2]\n\
         Shape \"sphere\"\n\
         WorldEnd\n",
    )
    .unwrap();
    let (_store, session, report) = import(&source, &temp.path().join("project"));
    assert_eq!(
        report
            .diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code == "unresolved_include")
            .count(),
        2,
        "{:?}",
        report.diagnostics
    );
    assert!(
        report.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("Include parameters with no directive to continue")),
        "{:?}",
        report.diagnostics
    );
    assert!(
        report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("missing.pbrt")),
        "{:?}",
        report.diagnostics
    );
    let document = session.document();
    let sphere = document
        .scenes
        .values()
        .flat_map(|scene| scene.nodes.values())
        .find(|node| matches!(&node.data, NodeKind::Entity { implementation, .. } if implementation == "sphere"))
        .expect("the scene after the skipped includes");
    assert!(matches!(
        &sphere.data,
        NodeKind::Entity {
            parameters,
            ..
        } if parameters.iter().all(|parameter| parameter.name != "x" && parameter.name != "y")
    ));
}

/// The preview program must not expand a medium's bulk arrays — a
/// grid density can hold millions of values and the preview never draws
/// media. MakeNamedMedium reaches the program with its name and type
/// only, while the text export keeps everything.
#[test]
fn preview_programs_emit_media_names_without_their_bulk_arrays() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let density = std::iter::repeat_n("1", 4097).collect::<Vec<_>>().join(" ");
    fs::write(
        &source,
        format!(
            "MakeNamedMedium \"smoke\" \"string type\" [\"uniformgrid\"] \"float density\" [{density}]\n\
             Camera \"perspective\"\n\
             WorldBegin\n\
             MediumInterface \"smoke\" \"\"\n\
             Shape \"sphere\"\n\
             WorldEnd\n"
        ),
    )
    .unwrap();
    let (store, session, _) = import(&source, &temp.path().join("project"));
    let program =
        pbrt_io::program_from_project(&session.snapshot(), &store, &BTreeMap::new(), || false)
            .unwrap();
    let medium = program
        .directives
        .iter()
        .find(|directive| {
            directive.name == "MakeNamedMedium" && directive.positional_text(0) == Some("smoke")
        })
        .expect("the medium reaches the preview program by name");
    assert_eq!(
        medium.parameters,
        vec![scene_parse::Parameter {
            value_type: "string".into(),
            name: "type".into(),
            values: vec![scene_parse::Value::Text("uniformgrid".into())],
            value_tokens: vec!["\"uniformgrid\"".into()],
            line: 0,
            column: 0,
        }],
        "only the packed type parameter: {:?}",
        medium.parameters
    );
    assert!(
        !medium
            .parameters
            .iter()
            .any(|parameter| parameter.name == "density"),
        "the bulk density must not expand into the program"
    );
    // The text export keeps the full medium (r4 renders from it).
    let export = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&export).unwrap();
    assert!(text.contains("\"float density\""), "{text}");
    assert!(text.contains("\"string type\" [\"uniformgrid\"]"), "{text}");
}

/// pbrt-v4 binds both sides with one name — `MediumInterface "fog"`
/// is inside = outside = fog. The importer must not read the missing
/// second argument as an empty (absent) outside medium.
#[test]
fn a_one_argument_medium_interface_binds_inside_and_outside() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "MakeNamedMedium \"fog\" \"string type\" [\"homogeneous\"] \"rgb sigma_s\" [.1 .2 .3]\n\
         MediumInterface \"fog\"\n\
         Camera \"perspective\"\n\
         WorldBegin\n\
         Shape \"sphere\" \"float radius\" [1]\n\
         WorldEnd\n",
    )
    .unwrap();
    let (store, session, report) = import(&source, &temp.path().join("project"));
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("medium is dropped")),
        "{:?}",
        report.diagnostics
    );
    let document = session.document();
    let fog = medium_of(document, "fog").id;
    let view = document.default_scene_view().unwrap();
    let states = evaluate(&view).unwrap();
    let scene = document.scene(document.default_scene.unwrap()).unwrap();
    let sphere = shapes_under(document, scene.world)
        .into_iter()
        .find(|id| states.nodes.contains_key(id))
        .expect("the sphere under the medium");
    let state = &states.nodes[&sphere];
    assert_eq!(state.inside_medium, Some(fog), "inside binds fog");
    assert_eq!(state.outside_medium, Some(fog), "outside binds fog too");
    let export = export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let text = fs::read_to_string(&export).unwrap();
    assert!(text.contains("MediumInterface \"fog\" \"fog\""), "{text}");
}

/// Every shape node under the world scope, depth first.
fn shapes_under(document: &ProjectDocument, scope: NodeId) -> Vec<NodeId> {
    let mut out = Vec::new();
    let mut stack = vec![scope];
    while let Some(id) = stack.pop() {
        if let Some(node) = document
            .scenes
            .values()
            .find_map(|scene| scene.nodes.get(&id))
            .or_else(|| document.definition_nodes.get(&id))
        {
            match &node.data {
                NodeKind::Scope { children, .. } => stack.extend(children.iter().rev()),
                NodeKind::Entity {
                    entity: EntityKind::Shape,
                    ..
                } => out.push(id),
                _ => {}
            }
        }
    }
    out
}

/// The S6 scope is retention and export only: the parameter table (typed
/// Inspector rows, GUI creation candidates) keeps its single homogeneous
/// row, so grid media stay free parameters in the Inspector.
#[test]
fn medium_parameter_table_stays_homogeneous_only() {
    let table = schema::schema("MakeNamedMedium").unwrap();
    let names = table
        .iter()
        .map(|implementation| implementation.name)
        .collect::<Vec<_>>();
    assert_eq!(names, ["homogeneous"]);
    for grid in ["uniformgrid", "rgbgrid", "cloud", "nanovdb"] {
        assert!(
            schema::implementation(table, grid).is_none(),
            "{grid} must not gain a typed parameter table yet"
        );
    }
}

/// PR #333 review: a file named in parameters that continue past an
/// `Include` resolves from the parent file that wrote it, not from the
/// included file's directory, even when both directories hold that name.
#[test]
fn continued_parameters_resolve_files_from_the_parent_file() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("sub")).unwrap();
    fs::write(temp.path().join("grid.nvdb"), b"parent grid").unwrap();
    fs::write(temp.path().join("sub/grid.nvdb"), b"sub grid").unwrap();
    fs::write(
        temp.path().join("sub/medium.pbrt"),
        "MakeNamedMedium \"foo\" \"string type\" [\"nanovdb\"]\n",
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "Include \"sub/medium.pbrt\" \"string filename\" [\"grid.nvdb\"]\n\
         Camera \"perspective\"\n\
         WorldBegin\n\
         MediumInterface \"foo\" \"\"\n\
         Shape \"sphere\"\n\
         WorldEnd\n",
    )
    .unwrap();
    let (store, session, report) = import(&source, &temp.path().join("project"));
    assert!(
        !report.diagnostics.iter().any(|d| d.message.contains("foo")),
        "{:?}",
        report.diagnostics
    );
    export_project(
        &session.snapshot(),
        &store,
        &temp.path().join("bundle"),
        || false,
    )
    .unwrap();
    let copied = fs::read_dir(temp.path().join("bundle/assets"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with("grid.nvdb"))
        .expect("the nvdb asset in the bundle");
    assert_eq!(fs::read(copied).unwrap(), b"parent grid");
}
