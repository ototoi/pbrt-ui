//! Direct-preview parity: for every fixture, the text path (export → read_document →
//! parse_scene_with_cache) and the direct path (program_from_project →
//! preview_from_program) must agree on the directive stream and, modulo
//! path/line normalization and mesh-id stabilization, on the manifest.
use pbrt_io::{ImportOptions, export_project, import_project, program_from_project, stage_assets};
use scene_parse::{
    Directive, GeometryCache, Parameter, ParseOptions, PreviewProgram, Value,
    parse_scene_with_cache, preview_from_program,
};
use serde_json::{Value as Json, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::{env, fs};

fn fixture(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures")
        .join(name)
}

/// Everything except the ignored location fields must match exactly.
fn assert_directives_equal(text: &[Directive], direct: &[Directive]) {
    assert_eq!(
        text.len(),
        direct.len(),
        "directive count differs between the text and direct paths"
    );
    for (index, (left, right)) in text.iter().zip(direct).enumerate() {
        assert_eq!(left.name, right.name, "directive {index} name differs");
        assert_eq!(
            left.positional_tokens, right.positional_tokens,
            "directive {index} ({}) positional lexemes differ",
            left.name
        );
        assert_eq!(
            left.positional, right.positional,
            "directive {index} ({}) classified positionals differ",
            left.name
        );
        assert_parameters_equal(index, &left.parameters, &right.parameters);
    }
}

fn assert_parameters_equal(index: usize, left: &[Parameter], right: &[Parameter]) {
    assert_eq!(
        left.len(),
        right.len(),
        "directive {index} parameter counts differ"
    );
    for (left, right) in left.iter().zip(right) {
        assert_eq!(left.value_type, right.value_type);
        assert_eq!(left.name, right.name);
        assert_eq!(left.values, right.values, "parameter values differ");
        assert_eq!(left.value_tokens, right.value_tokens);
    }
}

/// Node key of one draw for mesh re-keying: the resolved origin node, else
/// the shape ordinal. Mirrors the join the preview bindings perform.
fn draw_node_key(
    node_sources: &BTreeMap<u32, String>,
    lines: &BTreeSet<u32>,
    line: u32,
) -> Option<String> {
    if let Some(node) = node_sources.get(&line) {
        return Some(node.clone());
    }
    let bound = lines.range(..=line).next_back()?;
    node_sources.get(bound).cloned()
}

/// Stabilized mesh ids: meshes renumbered by (shape-node bucket, shape
/// order), the total order `core::preview_bindings::rekey_meshes` produces.
/// The node key comes from the SHAPE directive's origin, which both
/// paths record: the text path via its `node_sources` line join, the direct
/// path via the emitter's origin table.
fn mesh_remap(manifest: &Json, node_of_line: &dyn Fn(u32) -> Option<String>) -> BTreeMap<u64, u64> {
    let mut groups = BTreeMap::<String, BTreeSet<(usize, u64)>>::new();
    for draw in manifest["draws"].as_array().expect("draws") {
        let shape_index = draw["shape_index"].as_u64().expect("shape_index") as usize;
        let mesh_id = draw["mesh_id"].as_u64().expect("mesh_id");
        let key = (|| {
            let line = draw["source"]["line"].as_u64()? as u32;
            node_of_line(line)
        })()
        .unwrap_or_else(|| format!("!shape:{shape_index}"));
        groups
            .entry(key)
            .or_default()
            .insert((shape_index, mesh_id));
    }
    let mut remap = BTreeMap::new();
    let mut next = 0u64;
    for (_key, entries) in groups {
        for (_shape_index, old_id) in entries {
            if remap.insert(old_id, next).is_none() {
                next += 1;
            }
        }
    }
    remap
}

/// Normalize a manifest for P2 comparison: scrub path/line/blob identity and
/// apply the mesh re-keying so ordinal and hash-spelled mesh ids converge.
fn normalize(json: &Json, remap: &BTreeMap<u64, u64>) -> Json {
    let mut out = json.clone();
    let mut blobs = Vec::new();
    for (name, entry) in out["blobs"].as_object().expect("blobs").iter() {
        if name.starts_with("geometry/") {
            blobs.push(entry["sha256"].as_str().expect("blob digest").to_string());
        }
    }
    blobs.sort();
    // Texture blobs carry stable texture names; keep them keyed.
    out["draws"] = json!(
        out["draws"]
            .as_array()
            .expect("draws")
            .iter()
            .map(|draw| {
                let mut draw = draw.clone();
                draw["mesh_id"] = json!(
                    remap
                        .get(&draw["mesh_id"].as_u64().expect("mesh_id"))
                        .copied()
                        .expect("mesh must be remapped")
                );
                draw["source"] = json!(null);
                draw["instance_source"] = json!(null);
                draw["entity_id"] = json!(null);
                draw
            })
            .collect::<Vec<_>>()
    );
    out["meshes"] = json!(
        out["meshes"]
            .as_array()
            .expect("meshes")
            .iter()
            .map(|mesh| {
                let mut mesh = mesh.clone();
                mesh["mesh_id"] = json!(
                    remap
                        .get(&mesh["mesh_id"].as_u64().expect("mesh_id"))
                        .copied()
                        .expect("mesh must be remapped")
                );
                mesh["blob"] = json!(null);
                mesh
            })
            .collect::<Vec<_>>()
    );
    out["geometry_blob_digests"] = json!(blobs);
    // Geometry blob file names embed the pre-stabilization mesh id; texture
    // blob names are stable. Keep the stable ones, drop the rest.
    out["blobs"] = json!(
        out["blobs"]
            .as_object()
            .expect("blobs")
            .iter()
            .filter(|(name, _)| !name.starts_with("geometry/"))
            .map(|(name, entry)| (name.clone(), entry.clone()))
            .collect::<serde_json::Map<String, Json>>()
    );
    // Light source locations name the text file vs the preview path; the
    // direct path also always emits at column 0. Entity pins:
    // only the direct path carries them (the text path parses with no
    // origin table), so they normalize away like the draw pins.
    if let Some(lights) = out["lights"].as_array() {
        out["lights"] = json!(
            lights
                .iter()
                .map(|light| {
                    let mut light = light.clone();
                    light["source"] = json!(null);
                    light["entity_id"] = json!(null);
                    light
                })
                .collect::<Vec<_>>()
        );
    }
    // Material source locations name the text file vs the preview path.
    // (Empty material tables may be omitted entirely by the serializer.)
    out["materials"] = out["materials"].as_array().map_or(json!([]), |materials| {
        json!(
            materials
                .iter()
                .map(|material| {
                    let mut material = material.clone();
                    material["source"] = json!(null);
                    material
                })
                .collect::<Vec<_>>()
        )
    });
    out
}

fn manifest_json(path: &Path) -> Json {
    // The preview geometry is serialized by scene-parse::export::write_preview.
    serde_json::from_str(&fs::read_to_string(path).expect("geometry.json"))
        .expect("geometry.json parses")
}

fn import_fixture(path: &Path, home: &Path) -> (project::ProjectStore, project::ProjectSession) {
    import_project(
        path,
        &home.join("project"),
        &ImportOptions::default(),
        || false,
    )
    .expect("fixture imports")
}

#[test]
fn direct_program_matches_the_text_export_and_its_preview() {
    for name in [
        "minimal.pbrt",
        "editor-preview.pbrt",
        "instances-preview.pbrt",
        "render-preview.pbrt",
        "matrix-preview.pbrt",
    ] {
        let temp = tempfile::tempdir().unwrap();
        let (store, session) = import_fixture(&fixture(name), temp.path());

        // Text path: the historical full pipeline.
        let text_bundle = temp.path().join("text");

        let scene_path = export_project(&session.snapshot(), &store, &text_bundle, || false)
            .expect("text export");
        let text_document = scene_parse::document::read_document_with_options(
            &scene_path,
            scene_parse::document::ReadOptions {
                expand_imports: true,
                ..Default::default()
            },
        )
        .expect("re-read the exported text");
        let mut text_cache = GeometryCache::new(0);
        let text_preview =
            parse_scene_with_cache(&scene_path, ParseOptions::default(), &mut text_cache)
                .expect("text preview");
        let text_manifest_path = {
            text_preview
                .write_to(&temp.path().join("text-preview"))
                .expect("write text preview package");
            temp.path().join("text-preview/geometry.json")
        };
        let export_json: Json = serde_json::from_str(
            &fs::read_to_string(scene_path.parent().unwrap().join("export.json"))
                .expect("export.json"),
        )
        .expect("export.json parses");
        let mut node_sources = BTreeMap::new();
        if let Some(sources) = export_json["node_sources"].as_object() {
            for (line, node) in sources {
                node_sources.insert(
                    line.parse::<u32>().expect("node_sources keys are lines"),
                    node.as_str()
                        .expect("node ids serialize as strings")
                        .to_string(),
                );
            }
        }

        // Direct path: staged assets + shared emitter, no text round trip.
        let direct_root = temp.path().join("direct");
        let direct_bundle = direct_root.join("bundle");
        fs::create_dir_all(&direct_bundle).unwrap();
        let document = session.snapshot();
        let assets = stage_assets(document.document(), &store, &direct_bundle, &|| false)
            .expect("stage assets");
        let program = program_from_project(&session.snapshot(), &store, &assets, || false)
            .expect("program build");
        let origins = program.origins.clone();
        assert_directives_into(&text_document, &program, name);
        let mut direct_cache = GeometryCache::new(0);
        let direct_preview = preview_from_program(
            program,
            Some(&origins),
            &direct_bundle,
            ParseOptions::default(),
            &mut direct_cache,
        )
        .unwrap_or_else(|error| panic!("{name}: direct preview failed: {error}"));
        direct_preview
            .write_to(&temp.path().join("direct-preview"))
            .expect("write direct preview package");
        let direct_manifest_path = temp.path().join("direct-preview/geometry.json");

        // P1 leftovers: text source lines point at scene.pbrt; direct at the
        // preview path. Everything else already matched.
        let text_manifest = manifest_json(&text_manifest_path);
        let direct_manifest = manifest_json(&direct_manifest_path);
        let origins: BTreeMap<u32, String> = origins
            .iter()
            .map(|(line, entity)| (*line, entity.0.clone()))
            .collect();
        assert_manifest_parity(
            &text_manifest,
            &direct_manifest,
            &node_sources,
            &origins,
            name,
        );
    }
}

/// Where the manifest lives inside the preview package differs from text;
/// assert P1 (directives) as its own step for clear failures.
fn assert_directives_into(
    text_document: &scene_parse::document::Document,
    program: &PreviewProgram,
    name: &str,
) {
    assert!(
        program.sources.is_empty(),
        "synthetic programs carry no sources"
    );
    assert_directives_equal(&text_document.directives, &program.directives);
    let _ = name;
}

fn assert_manifest_parity(
    text: &Json,
    direct: &Json,
    node_sources: &BTreeMap<u32, String>,
    origins: &BTreeMap<u32, String>,
    name: &str,
) {
    let lines: BTreeSet<u32> = node_sources.keys().copied().collect();
    let text_remap = mesh_remap(text, &|line| draw_node_key(node_sources, &lines, line));
    let direct_remap = mesh_remap(direct, &|line| origins.get(&line).cloned());
    for draw in direct["draws"].as_array().expect("draws") {
        if let Some(entity) = draw["entity_id"].as_str() {
            assert!(
                node_sources.values().any(|node| node == entity),
                "{name}: direct draw pinned to entity {entity} that the text \
                 node_sources never name"
            );
        }
    }
    // Every light record of a project-emitted program
    // carries the pin to the light node that emitted its directive — the
    // same origin table the draw pins come from.
    for light in direct["lights"].as_array().into_iter().flatten() {
        let entity = light["entity_id"]
            .as_str()
            .unwrap_or_else(|| panic!("{name}: light record carries no entity pin: {light}"));
        assert!(
            origins.values().any(|node| node == entity),
            "{name}: direct light pinned to entity {entity} that the emitter's origins never name"
        );
    }
    let text = normalize(text, &text_remap);
    let direct = normalize(direct, &direct_remap);
    assert_eq!(text, direct, "{name}: manifests diverge (P2)");
}

#[test]
fn classify_uses_the_real_lexer() {
    // Quoted values keep their escapes decoded, inf stays text (with the
    // numeric escape inside Value), numbers parse as numbers.
    assert_eq!(Value::classify("1"), Value::Number(1.0));
    assert_eq!(Value::classify("-2.5"), Value::Number(-2.5));
    assert_eq!(Value::classify("\"a b\""), Value::Text("a b".to_string()));
    assert_eq!(Value::classify("inf"), Value::Text("inf".to_string()));
    assert_eq!(Value::classify("true"), Value::Text("true".to_string()));
    assert_eq!(Value::classify("inf").as_float(), Some(f64::INFINITY));
    assert_eq!(Value::classify("nan").as_float().unwrap().is_nan(), true);
}

/// The 16-valued matrix directives keep the historical column
/// transposition, and animation directives keep the reader's token arity.
/// (The golden export also pins the text bytes; this pins the semantics.)
#[test]
fn matrix_and_animation_directives_keep_their_lexical_shape() {
    let temp = tempfile::tempdir().unwrap();
    let (store, session) = import_fixture(&fixture("matrix-preview.pbrt"), temp.path());
    let bundle = temp.path().join("direct");
    fs::create_dir_all(&bundle).unwrap();
    let assets = stage_assets(session.snapshot().document(), &store, &bundle, &|| false).unwrap();
    let program = program_from_project(&session.snapshot(), &store, &assets, || false).unwrap();

    let find = |name: &str| {
        program
            .directives
            .iter()
            .find(|directive| directive.name == name)
            .unwrap_or_else(|| panic!("missing {name}"))
    };
    // The selected render camera is re-emitted at the end of Options (last
    // Camera wins in PBRT), so its start/end matrices arrive as `Transform`
    // directives before WorldBegin. The pose pathway inverts twice
    // (records the pose, then re-emits its inverse), so the values carry
    // the sanctioned 1e-12 round-trip drift against the fixture's LookAt,
    // still transposed back into the file's row-major spelling.
    let camera_start: Vec<f64> = find("Transform")
        .positional_tokens
        .iter()
        .map(|token| token.parse::<f64>().expect("a numeric camera matrix"))
        .collect();
    let lookat_ctm = [
        -0.8944271909999159,
        -0.12777531299998798,
        -0.42857142857142855,
        0.0,
        0.0,
        0.9583148474999098,
        -0.2857142857142857,
        0.0,
        0.4472135954999579,
        -0.25555062599997597,
        -0.8571428571428571,
        0.0,
        0.0,
        0.0,
        6.999999999999999,
        1.0,
    ];
    assert_eq!(camera_start.len(), 16);
    for (emitted, expected) in camera_start.iter().zip(&lookat_ctm) {
        let scale = f64::abs(*expected).max(1.0);
        assert!(
            (emitted - expected).abs() <= 1e-12 * scale,
            "camera start sample drifted: {emitted} vs {expected}"
        );
    }
    // The sphere's own 16-value `Transform` keeps
    // the row-major spelling of the file: identity with translation in the
    // last row. It is the first Transform of the world, after WorldBegin.
    let world = program
        .directives
        .iter()
        .position(|directive| directive.name == "WorldBegin")
        .expect("missing WorldBegin");
    // The sphere's own 16-value `Transform` resolves
    // in place (the scope entry is the identity on both samples, so the
    // reset's matrix concats) and keeps the row-major spelling of the file,
    // first in the world after WorldBegin.
    let world_transform = program
        .directives
        .iter()
        .skip(world + 1)
        .find(|directive| directive.name == "ConcatTransform")
        .expect("missing the sphere's matrix");
    assert_eq!(
        world_transform.positional_tokens,
        [
            "1", "0", "0", "0", "0", "1", "0", "0", "0", "0", "1", "0", "-1.5", "0", "0.5", "1"
        ]
    );
    // The camera's end transform is the unset (identity) end slot.
    let transforms: Vec<_> = program
        .directives
        .iter()
        .filter(|directive| directive.name == "Transform")
        .collect();
    assert_eq!(
        transforms[1].positional_tokens,
        [
            "1", "0", "0", "0", "0", "1", "0", "0", "0", "0", "1", "0", "0", "0", "0", "1"
        ]
    );
    // The cone's own ConcatTransform comes second, values untouched.
    let concats: Vec<_> = program
        .directives
        .iter()
        .filter(|directive| directive.name == "ConcatTransform")
        .collect();
    assert_eq!(
        concats[1].positional_tokens,
        [
            "1", "0", "0", "0", "0", "1", "0", "0", "0", "0", "1", "0", "2.5", "0", "0.75", "1"
        ]
    );
    // The Options look-at no longer exports; the pose carries the
    // placement and only the inverse-pose Transform remains.
    assert!(
        !program
            .directives
            .iter()
            .any(|directive| directive.name == "LookAt"),
        "the options transform run is resolved away"
    );
    assert_eq!(find("TransformTimes").positional_tokens, ["0", "2"]);
    let active: Vec<_> = program
        .directives
        .iter()
        .filter(|directive| directive.name == "ActiveTransform")
        .map(|directive| directive.positional_tokens.clone())
        .collect();
    // The selected camera's (StartTime, EndTime, All), then the world's own
    // EndTime from the fixture; the options-level StartTime is resolved away
    // and only the camera's own choreography precedes it.
    assert_eq!(
        active,
        vec![
            vec!["StartTime"],
            vec!["EndTime"],
            vec!["All"],
            vec!["EndTime"],
        ]
    );
}
