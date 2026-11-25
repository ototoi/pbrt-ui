//! Joins exported preview draws back to native project nodes.
use protocol::geometry::GeometryManifest;
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

pub fn bindings(
    manifest: &GeometryManifest,
    scene_path: &Path,
    export: &Value,
) -> Result<BTreeMap<u32, String>, String> {
    let scene_path = protocol::path::canonicalize(scene_path).map_err(|e| e.to_string())?;
    let sources = export["node_sources"]
        .as_object()
        .ok_or("preview export omitted node bindings")?;
    let mut result = BTreeMap::new();
    for draw in &manifest.draws {
        let source = draw
            .instance_source
            .as_ref()
            .map(|s| &s.source)
            .or(draw.source.as_ref());
        let Some(source) = source else { continue };
        if Path::new(&source.path) != scene_path {
            continue;
        }
        if let Some(node) = sources
            .get(&source.line.to_string())
            .and_then(Value::as_str)
        {
            uuid::Uuid::parse_str(node).map_err(|e| e.to_string())?;
            result.insert(draw.pick_id, node.to_owned());
        }
    }
    Ok(result)
}

/// Assign each mesh a stable id derived from the node that emitted its shape,
/// so unrelated node insertions or deletions do not renumber untouched meshes
/// and GPU mesh editions survive structural edits.
///
/// Meshes whose shape source does not resolve to a node keep a positional
/// fallback key, matching the previous behavior for non-project scenes.
/// Export bundles without `node_sources` are left unchanged.
pub fn rekey_meshes(
    manifest: &mut GeometryManifest,
    scene_path: &Path,
    export: &Value,
) -> Result<(), String> {
    let Some(sources) = export["node_sources"].as_object() else {
        return Ok(());
    };
    let scene_path = protocol::path::canonicalize(scene_path).map_err(|e| e.to_string())?;
    let mut lines = BTreeSet::new();
    for key in sources.keys() {
        if let Ok(line) = key.parse::<u32>() {
            lines.insert(line);
        }
    }
    let node_for_source = |source: Option<&protocol::package::SourceLocation>| -> Option<String> {
        let source = source?;
        if Path::new(&source.path) != scene_path {
            return None;
        }
        // The shape directive starts inside its node's line range; fall back to
        // the nearest node entry at or before the directive for wrapped output.
        let exact = sources
            .get(&source.line.to_string())
            .and_then(Value::as_str);
        exact.map(str::to_owned).or_else(|| {
            let key = lines.range(..=source.line).next_back()?.to_string();
            sources.get(&key).and_then(Value::as_str).map(str::to_owned)
        })
    };
    // shape file order per node, plus the mesh it belongs to.
    let mut groups: BTreeMap<String, BTreeSet<(usize, u64)>> = BTreeMap::new();
    for draw in &manifest.draws {
        let key = node_for_source(draw.source.as_ref())
            .unwrap_or_else(|| format!("!shape:{}", draw.shape_index));
        groups
            .entry(key)
            .or_default()
            .insert((draw.shape_index, draw.mesh_id));
    }
    let mut remap = BTreeMap::new();
    let mut next_id = 0u64;
    for (_key, entries) in groups {
        for (_shape_index, old_id) in entries {
            if remap.insert(old_id, next_id).is_none() {
                next_id += 1;
            }
        }
    }
    for mesh in &mut manifest.meshes {
        let id = remap
            .get(&mesh.mesh_id)
            .copied()
            .ok_or("preview mesh had no drawable binding")?;
        mesh.mesh_id = id;
    }
    manifest.meshes.sort_by_key(|mesh| mesh.mesh_id);
    for draw in &mut manifest.draws {
        let id = remap
            .get(&draw.mesh_id)
            .copied()
            .ok_or("preview draw referenced an unbound mesh")?;
        draw.mesh_id = id;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::geometry::{ArrayRef, Bounds, DrawRecord, MeshRecord};

    fn manifest(scene_path: &Path, mesh_ids: &[u64], shape_lines: &[u32]) -> GeometryManifest {
        let mut result = GeometryManifest {
            supported_times: vec![0.0],
            meshes: Vec::new(),
            draws: Vec::new(),
            ..GeometryManifest::default()
        };
        for (index, id) in mesh_ids.iter().enumerate() {
            result.meshes.push(MeshRecord {
                mesh_id: *id,
                blob: format!("geometry/{index:08}.bin"),
                positions: ArrayRef {
                    offset: 0,
                    count: 1,
                    stride: 12,
                },
                normals: None,
                uv: None,
                indices: ArrayRef {
                    offset: 0,
                    count: 1,
                    stride: 4,
                },
                origin_f64: [0.0; 3],
                bounds: Bounds::empty(),
            });
        }
        for (index, id) in mesh_ids.iter().enumerate() {
            let line = shape_lines[index];
            result.draws.push(DrawRecord {
                mesh_id: *id,
                shape_index: index,
                source: Some(protocol::package::SourceLocation {
                    path: scene_path.to_string_lossy().into_owned(),
                    line,
                    column: 1,
                }),
                shape_type: "sphere".into(),
                pick_id: index as u32 + 1,
                entity_id: None,
                instance_use_id: None,
                instance_source: None,
                bounds: None,
                world_from_object: vec![[0.0; 16]],
                reverse_orientation: false,
                material_id: None,
                interface: false,
                area_light: None,
                emitter_shape: None,
                alpha: None,
            });
        }
        result
    }

    fn export_with(node_lines: &[(&str, u32)]) -> Value {
        let mut node_sources = serde_json::Map::new();
        for (node, line) in node_lines {
            node_sources.insert(line.to_string(), serde_json::json!(node));
        }
        serde_json::json!({ "node_sources": node_sources })
    }

    /// Mesh id that each shape line ended up with after the rekey.
    fn rekey(
        shape_scene: &str,
        shape_lines: &[u32],
        node_lines: &[(&str, u32)],
    ) -> BTreeMap<u32, u64> {
        let temp = tempfile::tempdir().unwrap();
        // rekey_meshes matches canonical paths; macOS temp dirs sit behind /var.
        let path = protocol::path::canonicalize(temp.path())
            .unwrap()
            .join("scene.pbrt");
        std::fs::write(&path, shape_scene).unwrap();
        let mesh_ids: Vec<u64> = (0..shape_lines.len() as u64).collect();
        let mut manifest = manifest(&path, &mesh_ids, shape_lines);
        rekey_meshes(&mut manifest, &path, &export_with(node_lines)).unwrap();
        manifest
            .draws
            .iter()
            .map(|draw| (draw.source.as_ref().unwrap().line, draw.mesh_id))
            .collect()
    }

    #[test]
    fn test_rekey_assigns_node_stable_mesh_ids_that_survive_insertion_and_deletion() {
        // Node "a" (line 3) and node "b" (line 2); ids follow node id order.
        let base = rekey("WorldBegin\nShape\nShape\n", &[2, 3], &[("b", 2), ("a", 3)]);
        assert_eq!(base, BTreeMap::from([(2, 1), (3, 0)]));
        // Node "a" emits a second shape (line 4); untouched meshes keep ids.
        let inserted = rekey(
            "WorldBegin\nShape\nShape\nShape\n",
            &[2, 3, 4],
            &[("b", 2), ("a", 3), ("a", 4)],
        );
        assert_eq!(
            inserted,
            BTreeMap::from([(2, 2), (3, 0), (4, 1)]),
            "insertion renumbers only nodes sorted after the edited one"
        );
        // Deleting the line 3 shape keeps node "b" at its id and node "a" at 0.
        let deleted = rekey("WorldBegin\nShape\nShape\n", &[2, 4], &[("b", 2), ("a", 4)]);
        assert_eq!(deleted, BTreeMap::from([(2, 1), (4, 0)]));
    }

    #[test]
    fn test_rekey_falls_back_to_position_and_keeps_unbound_scenes_untouched() {
        let no_sources = serde_json::json!({});
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("scene.pbrt");
        std::fs::write(&path, "WorldBegin\nShape\nShape\n").unwrap();
        let mut manifest = manifest(&path, &[5, 9], &[1, 1]);
        rekey_meshes(&mut manifest, &path, &no_sources).unwrap();
        assert_eq!(
            manifest
                .meshes
                .iter()
                .map(|mesh| mesh.mesh_id)
                .collect::<Vec<_>>(),
            vec![5, 9]
        );
        // Shapes outside node_sources keep distinct positional fallback keys.
        let unbound = rekey("WorldBegin\nShape\nShape\n", &[2, 3], &[]);
        assert_eq!(unbound, BTreeMap::from([(2, 0), (3, 1)]));
    }
}
