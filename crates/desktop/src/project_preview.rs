//! Native node bindings and content-based mesh editions for exported previews.
use protocol::{
    Bounds,
    geometry::{DrawRecord, GeometryManifest, LightRecord},
};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

/// World-space bounds of the selected node's placements. `ids` carries the
/// selection plus any descendants; with the lazy hierarchy that set is
/// computed on demand instead of walking a full-tree snapshot.
pub fn selected_bounds(
    manifest: &GeometryManifest,
    bindings: &BTreeMap<u32, String>,
    object_bounds: &BTreeMap<u64, Bounds>,
    time: f64,
    ids: &BTreeSet<String>,
) -> Option<Bounds> {
    let time_index = manifest.supported_times.iter().position(|t| *t == time)?;
    manifest
        .draws
        .iter()
        .filter(|draw| {
            bindings
                .get(&draw.pick_id)
                .is_some_and(|id| ids.contains(id))
        })
        .filter_map(|draw| {
            let bounds = object_bounds.get(&draw.mesh_id)?;
            let matrix = draw.matrix_at(time_index)?;
            Some(placed_bounds(bounds, &matrix))
        })
        .reduce(|a, b| a.union(&b))
}

/// The transform fast path's base —
/// the untouched draws AND lights of the installed preview, the package
/// slot it was read from, and the revision that install was keyed to.
/// Fast answers always apply to THIS copy, never to already-updated
/// draws or lights, so several moves in a row stay exact; a job's
/// install replaces the whole record wholesale (an override install —
/// the fast path's own light move — keeps it.
#[derive(Clone)]
pub struct TransformBase {
    /// The revision `project.preview_transforms` diffs against.
    pub revision: String,
    pub draws: Vec<DrawRecord>,
    /// The base manifest's light records — a moved static
    /// light retargets `world_from_light = delta · base` here. Records
    /// without their pin name nothing and cannot be answered.
    pub lights: Vec<LightRecord>,
    /// The base package's slot directory — a light delta
    /// re-installs from it (blobs, diagnostics) without a preview job.
    pub slot: PathBuf,
}

/// One entity's answer from `project.preview_transforms`: `[start16,
/// end16]`, both row-major like `DrawRecord::world_from_object`, so
/// `delta · draw_matrix` is the new draw matrix.
pub type TransformDelta = [[f64; 16]; 2];

/// Row-major `a · b`, the convention every preview matrix uses.
pub fn row_major_product(a: [f64; 16], b: [f64; 16]) -> [f64; 16] {
    std::array::from_fn(|i| {
        let (row, column) = (i / 4, i % 4);
        (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum()
    })
}

/// The axis-aligned box of `bounds`' corners carried through a row-major
/// affine transform (the 3×4 part; the last row multiplies the implicit 1).
fn placed_bounds(bounds: &Bounds, matrix: &[f64; 16]) -> Bounds {
    let mut out = Bounds {
        min: [f64::INFINITY; 3],
        max: [f64::NEG_INFINITY; 3],
    };
    for corner in 0..8 {
        let point: [f64; 3] = std::array::from_fn(|axis| {
            if corner & (1 << axis) == 0 {
                bounds.min[axis]
            } else {
                bounds.max[axis]
            }
        });
        for axis in 0..3 {
            let value = matrix[axis * 4] * point[0]
                + matrix[axis * 4 + 1] * point[1]
                + matrix[axis * 4 + 2] * point[2]
                + matrix[axis * 4 + 3];
            out.min[axis] = out.min[axis].min(value);
            out.max[axis] = out.max[axis].max(value);
        }
    }
    out
}

/// The fast answer applied to the base draws: `delta ·
/// draw_matrix` for every sample of a named draw. Core answers only for
/// static nodes, so one delta moves every sample (an instance's animated
/// object body included) exactly; an answer whose two samples differ is
/// not that, and falls back (`None`) to the caller's full rebuild. So does
/// a delta naming no base draw: the base may have omitted that entity's
/// geometry, and only a rebuild can tell. Draws missing from the answer
/// keep their base matrices. Each touched draw's world `bounds` follow the
/// mesh's local bounds (`object_bounds`, the same per-placement boxes the
/// install computes); a mesh without local bounds on hand keeps its base
/// box moved by the delta, which stays conservative.
pub fn apply_transform_deltas(
    base: &[DrawRecord],
    deltas: &BTreeMap<String, TransformDelta>,
    object_bounds: &BTreeMap<u64, Bounds>,
) -> Option<Vec<DrawRecord>> {
    if deltas.values().any(|delta| delta[0] != delta[1]) {
        return None;
    }
    let mut named = std::collections::BTreeSet::new();
    let mut draws = base.to_vec();
    for draw in &mut draws {
        let Some(entity) = draw.entity_id.as_ref() else {
            continue;
        };
        let Some(delta) = deltas.get(entity.as_str()) else {
            continue;
        };
        named.insert(entity.as_str().to_owned());
        let delta = delta[0];
        for matrix in &mut draw.world_from_object {
            *matrix = row_major_product(delta, *matrix);
        }
        if let Some(bounds) = draw.bounds.clone() {
            draw.bounds = match object_bounds.get(&draw.mesh_id) {
                Some(local) => draw
                    .world_from_object
                    .iter()
                    .map(|matrix| placed_bounds(local, matrix))
                    .reduce(|union, placed| union.union(&placed)),
                None => Some(placed_bounds(&bounds, &delta)),
            };
        }
    }
    (named.len() == deltas.len()).then_some(draws)
}

/// Union of every draw's placement bounds — the same give-up rule the
/// renderer's update applies (a draw without bounds leaves the previous
/// scene bounds standing).
pub fn draws_world_bounds(draws: &[DrawRecord]) -> Option<Bounds> {
    let mut bounds = draws.iter().map(|draw| draw.bounds.as_ref());
    let first = bounds.next()??.clone();
    bounds.try_fold(first, |union, next| Some(union.union(next?)))
}

/// The fast answer applied to the whole transform
/// base — every delta id must name a base draw or a base light, else
/// `None`: the base may have omitted that entity's geometry or pin, and
/// only a rebuild can tell. Draws retarget by `apply_transform_deltas`;
/// a named light retargets `world_from_light = delta.start ·
/// base.world_from_light` — the same row-major product, so `delta ·
/// base` is the new matrix. Core answers static entities only, so
/// endpoint pairs that differ retarget nothing, exactly like the
/// draws' rule.
pub fn retarget_transform_base(
    base: &TransformBase,
    deltas: &BTreeMap<String, TransformDelta>,
    object_bounds: &BTreeMap<u64, Bounds>,
) -> Option<(Vec<DrawRecord>, Vec<LightRecord>)> {
    if deltas.values().any(|delta| delta[0] != delta[1]) {
        return None;
    }
    let base_lights: BTreeSet<&str> = base
        .lights
        .iter()
        .filter_map(|light| light.entity_id.as_ref().map(|entity| entity.as_str()))
        .collect();
    let mut draw_deltas = BTreeMap::new();
    let mut light_deltas = BTreeMap::new();
    for (id, delta) in deltas {
        if base_lights.contains(id.as_str()) {
            light_deltas.insert(id.clone(), *delta);
        } else {
            draw_deltas.insert(id.clone(), *delta);
        }
    }
    let draws = apply_transform_deltas(&base.draws, &draw_deltas, object_bounds)?;
    let mut lights = base.lights.clone();
    for light in &mut lights {
        let Some(entity) = light.entity_id.as_ref() else {
            continue;
        };
        let Some(delta) = light_deltas.get(entity.as_str()) else {
            continue;
        };
        light.world_from_light = row_major_product(delta[0], light.world_from_light);
    }
    Some((draws, lights))
}

/// The digest a mesh's content resolves to — the blob's recorded sha256
/// plus its array layout, the same value `MeshEditions` keys its
/// editions by.
pub fn mesh_digest(mesh: &protocol::geometry::MeshRecord, manifest: &GeometryManifest) -> [u8; 32] {
    let mut hash = Sha256::new();
    hash.update(
        manifest
            .blobs
            .get(&mesh.blob)
            .map(|file| file.sha256.as_str())
            .unwrap_or_default()
            .as_bytes(),
    );
    hash.update(mesh.mesh_id.to_le_bytes());
    for array in [
        Some(&mesh.positions),
        mesh.normals.as_ref(),
        mesh.uv.as_ref(),
        Some(&mesh.indices),
    ] {
        hash.update([u8::from(array.is_some())]);
        if let Some(array) = array {
            hash.update(array.offset.to_le_bytes());
            hash.update(array.count.to_le_bytes());
            hash.update(array.stride.to_le_bytes());
        }
    }
    hash.finalize().into()
}

#[derive(Clone, Default)]
pub struct MeshEditions(BTreeMap<u64, ([u8; 32], u64)>);
impl MeshEditions {
    pub fn update(&mut self, manifest: &GeometryManifest) -> Result<BTreeMap<u64, u64>, String> {
        let mut next = BTreeMap::new();
        for mesh in &manifest.meshes {
            if !manifest.blobs.contains_key(&mesh.blob) {
                return Err("missing mesh blob".into());
            }
            // Digest the manifest's recorded blob hash plus layout: the blob
            // sha256 already covers every vertex byte, so re-hashing blobs on
            // each edit becomes a cheap per-mesh string hash — and reading
            // The blob was never needed because the manifest records its presence.
            let digest = mesh_digest(mesh, manifest);
            let edition = match self.0.get(&mesh.mesh_id) {
                Some((old, edition)) if *old == digest => *edition,
                Some((_, edition)) => edition.checked_add(1).ok_or("mesh edition overflow")?,
                None => 1,
            };
            next.insert(mesh.mesh_id, (digest, edition));
        }
        let editions = next
            .iter()
            .map(|(id, (_, edition))| (*id, *edition))
            .collect();
        self.0 = next;
        Ok(editions)
    }

    /// Staged update for the GPU worker: digest `manifest` into a candidate
    /// ledger without touching this one, and return the candidate with the
    /// per-mesh editions the renderer wants. The worker commits the
    /// candidate back (plain assignment) only after
    /// `PreviewRenderer::update_scene` accepts the scene: a failed install
    /// must not prune the digest history of meshes its package omits, or
    /// the next package re-adding such a mesh with changed content
    /// re-enters at first-appearance edition 1 — the same edition the
    /// entry retained on the live scene holds — and update_scene's
    /// edition cache silently reuses the outdated buffers. The old serial pipeline staged the same way (clone, update,
    /// commit after upload).
    pub fn staged_update(
        &self,
        manifest: &GeometryManifest,
    ) -> Result<(Self, BTreeMap<u64, u64>), String> {
        let mut candidate = self.clone();
        let editions = candidate.update(manifest)?;
        Ok((candidate, editions))
    }
}

/// Local-space mesh bounds keyed by the same digest `MeshEditions` uses
/// An unchanged mesh's blob need not be read again to
/// re-derive its bounds. The map stays on the GPU worker; the world-space
/// answer still comes from the draw matrices (`selected_bounds`).
#[derive(Clone, Default)]
pub struct MeshBounds(BTreeMap<[u8; 32], Bounds>);

impl MeshBounds {
    /// `object_bounds` over a digest-keyed cache: blobs whose digest is
    /// cached are not read, misses compute from `blobs` (absent bytes for
    /// an uncached mesh fail like today's reader, so a skipped read can
    /// never pass silently).
    pub fn object_bounds(
        &mut self,
        manifest: &GeometryManifest,
        blobs: &BTreeMap<String, Vec<u8>>,
    ) -> Result<BTreeMap<u64, Bounds>, String> {
        let mut bounds = BTreeMap::new();
        for mesh in &manifest.meshes {
            let digest = mesh_digest(mesh, manifest);
            let computed = if let Some(cached) = self.0.get(&digest) {
                cached.clone()
            } else {
                let positions = protocol::geometry::read_f32x3_array(
                    blobs.get(&mesh.blob).ok_or("missing mesh blob")?,
                    &mesh.positions,
                )?;
                let mut b = Bounds {
                    min: [f64::INFINITY; 3],
                    max: [f64::NEG_INFINITY; 3],
                };
                for p in &positions {
                    for (axis, value) in p.iter().enumerate() {
                        b.min[axis] = b.min[axis].min(*value as f64);
                        b.max[axis] = b.max[axis].max(*value as f64);
                    }
                }
                if positions.is_empty() {
                    // An empty mesh answers nothing, cached or not.
                    continue;
                }
                self.0.insert(digest, b.clone());
                b
            };
            bounds.insert(mesh.mesh_id, computed);
        }
        Ok(bounds)
    }
}

/// The blob names an install must actually read: the
/// union of
/// · geometry blobs whose mesh edition differs between the installed
///   ledger `current` and this update's `staged` (changed, or new),
/// · geometry blobs whose bounds digest `bounds` does not hold yet,
/// · geometry blobs of meshes drawn with an area light (evaluate reads
///   their triangles every refresh),
/// · `texture/` and `light/` blobs the renderer does not already hold as
///   committed, memo-failed or environment-reused identity (the
///   `resident` closure answers that).
pub fn needed_blob_names<F: Fn(&str) -> bool>(
    current: &MeshEditions,
    staged: &MeshEditions,
    bounds: &MeshBounds,
    manifest: &GeometryManifest,
    resident: F,
) -> BTreeSet<String> {
    let emitters: std::collections::BTreeSet<u64> = manifest
        .draws
        .iter()
        .filter(|draw| draw.area_light.is_some())
        .map(|draw| draw.mesh_id)
        .collect();
    let mut names = BTreeSet::new();
    for mesh in &manifest.meshes {
        let changed = staged.0.get(&mesh.mesh_id).is_some_and(|(digest, _)| {
            current
                .0
                .get(&mesh.mesh_id)
                .is_none_or(|(held, _)| held != digest)
        });
        let bounds_missing = !bounds.0.contains_key(&mesh_digest(mesh, manifest));
        if changed || bounds_missing || emitters.contains(&mesh.mesh_id) {
            names.insert(mesh.blob.clone());
        }
    }
    for name in manifest.blobs.keys() {
        if (name.starts_with("texture/") || name.starts_with("light/")) && !resident(name) {
            names.insert(name.clone());
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tree_node_keeps_the_active_flag_for_dimming() {
        let node = tree_node(&serde_json::json!({"node_id":"n","active":false}));
        assert_eq!(node["active"], false);
    }
    // Text-path join primitives under test here; the runtime no longer calls
    // them.
    use app_core::preview_bindings::{bindings, rekey_meshes};
    fn scene(
        source: &str,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        scene_parse::PreviewGeometry,
    ) {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("scene.pbrt");
        std::fs::write(&path, source).unwrap();
        let preview = scene_parse::parse_scene(&path, Default::default()).unwrap();
        (temp, path, preview)
    }

    #[test]
    fn edition_history_survives_a_failed_install_so_re_added_meshes_reupload() {
        // The GPU worker stages each install's ledger and commits it only
        // after update_scene accepts the scene. This pins why: a failed
        // install must not prune the digest history of meshes its package
        // omits, or the next package re-adding one with changed content
        // re-enters at edition 1 and matches the edition of the entry the
        // retained live scene still holds — the renderer then reuses the
        // outdated buffers.
        let (_temp, _sample, base) = scene("WorldBegin\nShape \"sphere\"\n");
        assert_eq!(base.manifest.meshes.len(), 1, "fixture needs one mesh");
        let mesh = base.manifest.meshes[0].clone();
        // update never reads blob bytes; the manifest's presence plus the
        // recorded sha256 is what the digest bookkeeping runs on.
        let manifest = |sha: &str, with_mesh: bool| {
            let mut manifest = base.manifest.clone();
            if with_mesh {
                manifest.blobs.get_mut(&mesh.blob).unwrap().sha256 = sha.into();
            } else {
                manifest.meshes.clear();
                manifest.draws.clear();
            }
            manifest
        };
        let mut ledger = MeshEditions::default();

        // Install A: the mesh first appears at edition 1; the upload
        // succeeds, so the candidate ledger is committed.
        let install_a = manifest("blob-a", true);
        let (committed_a, editions_a) = ledger.staged_update(&install_a).unwrap();
        assert_eq!(editions_a[&mesh.mesh_id], 1);
        ledger = committed_a;

        // Install B omits the mesh. Staging answers a pruned candidate, but
        // the live ledger only changes when the upload commits.
        let install_b = manifest("-", false);
        let (candidate_b, _) = ledger.staged_update(&install_b).unwrap();
        assert!(
            !candidate_b.0.contains_key(&mesh.mesh_id),
            "the staged candidate prunes the omitted mesh"
        );

        // Install C re-adds the mesh with changed vertex bytes. The intact
        // history compares digests, so the edition advances past the 1 the
        // retained live scene holds and update_scene uploads fresh buffers.
        let install_c = manifest("blob-c", true);
        let (_, editions_c) = ledger.staged_update(&install_c).unwrap();
        assert_eq!(
            editions_c[&mesh.mesh_id], 2,
            "changed content must not reuse the live scene's edition"
        );

        // The mechanics the early commit would reintroduce: committing B's
        // candidate drops the history, and the re-add
        // falls back to first-appearance numbering — the collision case.
        ledger = candidate_b;
        let (_, editions_bug) = ledger.staged_update(&install_c).unwrap();
        assert_eq!(
            editions_bug[&mesh.mesh_id], 1,
            "a pruned history re-enters at the live scene's edition"
        );
    }

    #[test]
    fn native_instance_selection_and_scope_bounds_follow_the_displayed_time() {
        let (_temp, path, mut preview) = scene(
            "WorldBegin\nObjectBegin \"ball\"\nShape \"sphere\"\nObjectEnd\nTranslate 2 0 0\nObjectInstance \"ball\"\nShape \"disk\"\n",
        );
        let instance = uuid::Uuid::new_v4().to_string();
        let shape = uuid::Uuid::new_v4().to_string();
        let mut export = serde_json::json!({"node_sources":{}});
        for draw in &preview.manifest.draws {
            let source = draw
                .instance_source
                .as_ref()
                .map(|s| &s.source)
                .or(draw.source.as_ref())
                .unwrap();
            export["node_sources"][source.line.to_string()] =
                serde_json::json!(if draw.instance_source.is_some() {
                    &instance
                } else {
                    &shape
                });
        }
        let ids = bindings(&preview.manifest, &path, &export).unwrap();
        let draw = preview
            .manifest
            .draws
            .iter_mut()
            .find(|d| d.instance_source.is_some())
            .unwrap();
        assert_eq!(ids[&draw.pick_id], instance);
        let first = draw.world_from_object[0];
        let mut last = first;
        last[3] += 10.;
        draw.world_from_object = vec![first, last];
        preview.manifest.supported_times = vec![0., 1.];
        let bounds = MeshBounds::default()
            .object_bounds(&preview.manifest, &preview.blobs)
            .unwrap();
        let scope_ids = BTreeSet::from(["scope".to_owned(), instance.clone()]);
        let first = selected_bounds(&preview.manifest, &ids, &bounds, 0., &scope_ids).unwrap();
        let last = selected_bounds(&preview.manifest, &ids, &bounds, 1., &scope_ids).unwrap();
        assert!((last.center()[0] - first.center()[0] - 10.).abs() < 1e-6);
        assert!(
            selected_bounds(
                &preview.manifest,
                &ids,
                &bounds,
                0.,
                &BTreeSet::from(["unknown".to_owned()])
            )
            .is_none()
        );
    }

    #[test]
    #[ignore = "requires a GPU adapter"]
    fn native_content_editions_reuse_real_gpu_buffers_after_transform_edits() {
        let (_a, _path, a) = scene("WorldBegin\nShape \"sphere\"\n");
        let (_b, _path, b) = scene("WorldBegin\nTranslate 4 0 0\nShape \"sphere\"\n");
        let (_c, _path, c) =
            scene("WorldBegin\nTranslate 4 0 0\nShape \"sphere\" \"float radius\" [2]\n");
        let mut editions = MeshEditions::default();
        let mut renderer = scene_view::PreviewRenderer::new().unwrap();
        renderer
            .update_scene(
                &a.manifest,
                &a.blobs,
                &editions.update(&a.manifest).unwrap(),
            )
            .unwrap();
        let uploads = renderer.stats().mesh_uploads;
        renderer
            .update_scene(
                &b.manifest,
                &b.blobs,
                &editions.update(&b.manifest).unwrap(),
            )
            .unwrap();
        assert_eq!(renderer.stats().mesh_uploads, uploads);
        renderer
            .update_scene(
                &c.manifest,
                &c.blobs,
                &editions.update(&c.manifest).unwrap(),
            )
            .unwrap();
        assert_eq!(renderer.stats().mesh_uploads, uploads + 1);
    }
    #[test]
    fn transforms_reuse_meshes_but_shape_edits_advance_editions() {
        // Use a real parser output to include generated normals and indices.
        let temp =
            std::env::temp_dir().join(format!("pui-mesh-test-{}.pbrt", uuid::Uuid::new_v4()));
        let parse = |translate, radius| {
            std::fs::write(&temp,format!("WorldBegin\nTranslate {translate} 0 0\nShape \"sphere\" \"float radius\" [{radius}]\n")).unwrap();
            scene_parse::parse_scene(&temp, Default::default()).unwrap()
        };
        let mut editions = MeshEditions::default();
        let first = parse(0, 1);
        let a = editions.update(&first.manifest).unwrap();
        let moved = parse(4, 1);
        assert_eq!(a, editions.update(&moved.manifest).unwrap());
        let changed = parse(4, 2);
        let b = editions.update(&changed.manifest).unwrap();
        assert!(a.iter().all(|(id, edition)| b[id] == edition + 1));
        let _ = std::fs::remove_file(temp);
    }

    /// The install reads only the blobs it needs —
    /// geometry blobs of changed or bounds-uncached meshes (and of
    /// area-light emitters), plus image blobs the renderer cannot
    /// resolve without bytes.
    #[test]
    fn an_install_only_reads_the_blobs_it_actually_needs() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("env.png"), b"env map bytes").unwrap();
        let path = temp.path().join("scene.pbrt");
        std::fs::write(
            &path,
            "WorldBegin\nShape \"sphere\"\nShape \"disk\"\nLightSource \"infinite\" \"string mapname\" \"env.png\"\n",
        )
        .unwrap();
        let base = scene_parse::parse_scene(&path, Default::default()).unwrap();
        assert!(base.manifest.meshes.len() >= 2, "fixture needs two meshes");
        assert!(
            base.manifest.blobs.contains_key("light/0"),
            "fixture needs the light map blob"
        );
        let mut editions = MeshEditions::default();
        let mut bounds = MeshBounds::default();

        // First install: no ledger, no bounds cache, nothing resident.
        let (staged, _) = editions.staged_update(&base.manifest).unwrap();
        let first = needed_blob_names(&editions, &staged, &bounds, &base.manifest, |_: &str| false);
        let geometry: std::collections::BTreeSet<String> = base
            .manifest
            .meshes
            .iter()
            .map(|mesh| mesh.blob.clone())
            .collect();
        assert!(geometry.is_subset(&first) && first.contains("light/0"));

        // The install succeeded: both caches are committed.
        editions = staged;
        bounds.object_bounds(&base.manifest, &base.blobs).unwrap();

        // Second install of the same manifest: nothing is read.
        let (staged, _) = editions.staged_update(&base.manifest).unwrap();
        assert!(
            needed_blob_names(&editions, &staged, &bounds, &base.manifest, |_: &str| true)
                .is_empty(),
            "unchanged content with resident images reads nothing"
        );

        // One mesh changed: only its geometry blob is read.
        let mut changed = base.manifest.clone();
        let mesh = &base.manifest.meshes[0];
        changed.blobs.get_mut(&mesh.blob).unwrap().sha256 = "changed".into();
        let (staged, _) = editions.staged_update(&changed).unwrap();
        assert_eq!(
            needed_blob_names(&editions, &staged, &bounds, &changed, |_: &str| true),
            std::collections::BTreeSet::from([mesh.blob.clone()])
        );

        // One texture changed: the renderer did not keep its identity, so
        // exactly that image blob is read — the env map stays unread
        // because the committed environment covers it.
        let mut textured = base.manifest.clone();
        textured.blobs.insert(
            "texture/spectrum/oak".into(),
            protocol::package::PackageFile {
                size: 1,
                sha256: "t".into(),
            },
        );
        let changed_texture = "texture/spectrum/oak";
        let resident = |name: &str| name != changed_texture;
        let (staged, _) = editions.staged_update(&textured).unwrap();
        assert_eq!(
            needed_blob_names(&editions, &staged, &bounds, &textured, resident),
            std::collections::BTreeSet::from([changed_texture.to_owned()]),
            "one changed texture reads exactly its blob"
        );
    }

    /// An emitter's mesh feeds `lit::evaluate`'s per-draw frames every
    /// install, so its geometry blob is read even when the mesh itself did
    /// not change.
    #[test]
    fn an_area_light_emitters_mesh_blob_is_always_read() {
        let (_temp, _path, base) = scene(
            "WorldBegin\nShape \"sphere\"\nAttributeBegin\nAreaLightSource \"diffuse\" \"rgb L\" [1 1 1]\nShape \"disk\"\nAttributeEnd\n",
        );
        let mut editions = MeshEditions::default();
        let mut bounds = MeshBounds::default();
        let (staged, _) = editions.staged_update(&base.manifest).unwrap();
        // The sphere draws without an area light, so once both caches are
        // committed only the emitter's blob is still read.
        editions = staged;
        bounds.object_bounds(&base.manifest, &base.blobs).unwrap();
        let (staged, _) = editions.staged_update(&base.manifest).unwrap();
        let read = needed_blob_names(&editions, &staged, &bounds, &base.manifest, |_: &str| true);
        assert!(
            read.len() == 1,
            "only the emitter draw's blob stays read, got {read:?}"
        );
        let emitter = base
            .manifest
            .draws
            .iter()
            .find(|draw| draw.area_light.is_some())
            .map(|draw| draw.mesh_id)
            .expect("the fixture has an emitter draw");
        let emitter_blob = &base
            .manifest
            .meshes
            .iter()
            .find(|mesh| mesh.mesh_id == emitter)
            .expect("the emitter draw has a mesh")
            .blob;
        assert_eq!(read.iter().next().unwrap(), emitter_blob);
    }

    /// Bounds recomputed from the cache equal a fresh recomputation,
    /// and a bounds-uncached mesh whose bytes are absent fails cleanly
    /// (the "missing mesh blob" error the old reader produced).
    #[test]
    fn bounds_from_the_digest_cache_equal_a_recomputation() {
        let (_temp, _path, base) = scene("WorldBegin\nShape \"sphere\"\nShape \"disk\"\n");
        let mut cache = MeshBounds::default();
        let from_cache = cache.object_bounds(&base.manifest, &base.blobs).unwrap();
        assert_eq!(
            from_cache,
            MeshBounds::default()
                .object_bounds(&base.manifest, &base.blobs)
                .unwrap(),
            "cached bounds equal a fresh computation"
        );
        // The cache answers the same digest without the bytes present.
        let mut empty = base.blobs.clone();
        empty.clear();
        let again = cache.object_bounds(&base.manifest, &empty).unwrap();
        assert_eq!(again, from_cache);
        // A fresh cache with absent bytes fails like today's reader.
        assert_eq!(
            MeshBounds::default()
                .object_bounds(&base.manifest, &empty)
                .unwrap_err(),
            "missing mesh blob"
        );
    }

    /// `MeshEditions::staged_update` checks blob presence
    /// on the manifest, never in the bytes — the install decides reads
    /// before parsing any blob.
    #[test]
    fn staged_update_presence_checks_the_manifest_not_the_blobs() {
        let (_temp, _path, base) = scene("WorldBegin\nShape \"sphere\"\n");
        let mesh = &base.manifest.meshes[0];
        let mut without_blob = base.manifest.clone();
        without_blob.blobs.remove(&mesh.blob);
        match MeshEditions::default().staged_update(&without_blob) {
            Err(error) => assert_eq!(error, "missing mesh blob"),
            Ok(_) => panic!("the manifest must refuse its own absent blob"),
        }
        // Bytes were never read: presence is the manifest's word.
        assert!(
            MeshEditions::default()
                .staged_update(&base.manifest)
                .is_ok()
        );
    }

    #[test]
    fn stable_mesh_editions_survive_structural_node_edits() {
        // Same project across two exports where a node is inserted ahead
        // of untouched shapes: directives shift lines, so only the rekeyed
        // node-stable identity keeps mesh ids and GPU editions.
        fn rekeyed(shapes: &[(&str, &str)]) -> scene_parse::PreviewGeometry {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join("scene.pbrt");
            let mut scene = String::from("WorldBegin\n");
            let mut export = serde_json::json!({ "node_sources": {} });
            let mut line = 2u32;
            for (directive, node) in shapes {
                scene.push_str(directive);
                export["node_sources"][line.to_string()] = serde_json::json!(node);
                line += directive.matches('\n').count() as u32;
            }
            std::fs::write(&path, scene).unwrap();
            let mut preview = scene_parse::parse_scene(&path, Default::default()).unwrap();
            rekey_meshes(&mut preview.manifest, &path, &export).unwrap();
            preview
        }
        let first = rekeyed(&[("Shape \"sphere\"\n", "aa"), ("Shape \"disk\"\n", "bb")]);
        let mut editions = MeshEditions::default();
        let first = editions.update(&first.manifest).unwrap();
        let inserted = rekeyed(&[
            ("Shape \"sphere\" \"float radius\" [2]\n", "cc"),
            ("Shape \"sphere\"\n", "aa"),
            ("Shape \"disk\"\n", "bb"),
        ]);
        let second = editions.update(&inserted.manifest).unwrap();
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 3);
        assert_eq!(first, BTreeMap::from([(0, 1), (1, 1)]));
        assert_eq!(second, BTreeMap::from([(0, 1), (1, 1), (2, 1)]));
    }

    // Apply transform deltas to the base preview.

    fn translation(x: f64, y: f64, z: f64) -> [f64; 16] {
        [1., 0., 0., x, 0., 1., 0., y, 0., 0., 1., z, 0., 0., 0., 1.]
    }

    fn box_bounds(min: [f64; 3], max: [f64; 3]) -> Bounds {
        Bounds { min, max }
    }

    fn delta_draw(mesh_id: u64, entity: Option<&str>, matrices: Vec<[f64; 16]>) -> DrawRecord {
        DrawRecord {
            mesh_id,
            shape_index: 0,
            source: None,
            shape_type: "sphere".into(),
            pick_id: mesh_id as u32,
            entity_id: entity.map(protocol::EntityId::new),
            instance_use_id: None,
            instance_source: None,
            bounds: Some(box_bounds([-1., -1., -1.], [1., 1., 1.])),
            world_from_object: matrices,
            reverse_orientation: false,
            material_id: None,
            interface: false,
            area_light: None,
            emitter_shape: None,
            alpha: None,
        }
    }

    fn deltas(list: &[(&str, TransformDelta)]) -> BTreeMap<String, TransformDelta> {
        list.iter()
            .map(|(id, delta)| ((*id).to_owned(), *delta))
            .collect()
    }

    /// A fast answer retargets `delta · draw_matrix` for every sample of a
    /// named draw (core answers static nodes only, so one delta), and each touched draw's placement box, plus
    /// the manifest-level union, follow the move off the meshes' local
    /// bounds. Draws the answer does not name keep their base matrices.
    #[test]
    fn transform_deltas_retarget_static_and_two_sample_draws() {
        let base = vec![
            delta_draw(1, Some("a"), vec![translation(1., 2., 3.)]),
            delta_draw(
                2,
                Some("b"),
                vec![translation(0., 0., 0.), translation(2., 0., 0.)],
            ),
            delta_draw(3, None, vec![translation(0., 0., 0.)]),
        ];
        let object_bounds = BTreeMap::from([
            (1u64, box_bounds([-1., -1., -1.], [1., 1., 1.])),
            (2u64, box_bounds([0., -1., -1.], [2., 1., 1.])),
        ]);
        let moved = apply_transform_deltas(
            &base,
            &deltas(&[
                ("a", [translation(3., 3., 3.); 2]),
                ("b", [translation(4., 0., 0.); 2]),
            ]),
            &object_bounds,
        )
        .expect("one static delta retargets every sample exactly");
        assert_eq!(moved[0].world_from_object, vec![translation(4., 5., 6.)]);
        assert_eq!(
            moved[1].world_from_object,
            vec![translation(4., 0., 0.), translation(6., 0., 0.)]
        );
        assert_eq!(moved[2], base[2], "unnamed draws keep their base matrices");
        assert_eq!(
            moved[0].bounds,
            Some(box_bounds([3., 4., 5.], [5., 6., 7.])),
            "the static draw's placement box follows its local bounds"
        );
        assert_eq!(
            moved[1].bounds,
            Some(box_bounds([4., -1., -1.], [8., 1., 1.])),
            "a per-time draw's placement box unions both samples"
        );
        assert_eq!(
            draws_world_bounds(&moved),
            Some(box_bounds([-1., -1., -1.], [8., 6., 7.])),
            "the manifest-level union covers every draw"
        );
    }

    /// An empty answer (a rename) keeps the base draws; a delta naming no
    /// base draw falls back, since the base may have omitted that entity's
    /// geometry and only a rebuild can produce it.
    #[test]
    fn a_delta_without_a_base_draw_falls_back() {
        let base = vec![
            delta_draw(1, Some("a"), vec![translation(1., 0., 0.)]),
            delta_draw(3, None, vec![translation(0., 0., 3.)]),
        ];
        let kept = apply_transform_deltas(&base, &BTreeMap::new(), &BTreeMap::new())
            .expect("a rename keeps the scene");
        assert_eq!(kept, base);
        assert!(
            apply_transform_deltas(
                &base,
                &deltas(&[("ghost", [translation(9., 9., 9.); 2])]),
                &BTreeMap::new(),
            )
            .is_none()
        );
    }

    /// Several moves in a row stay exact: every answer applies to
    /// the BASE draws, never to already-updated ones.
    #[test]
    fn repeated_answers_apply_to_the_same_base_draws() {
        let base = vec![delta_draw(1, Some("a"), vec![translation(1., 0., 0.)])];
        let first = apply_transform_deltas(
            &base,
            &deltas(&[("a", [translation(1., 0., 0.); 2])]),
            &BTreeMap::new(),
        )
        .unwrap();
        let second = apply_transform_deltas(
            &base,
            &deltas(&[("a", [translation(2., 0., 0.); 2])]),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(first[0].world_from_object, vec![translation(2., 0., 0.)]);
        // From the base: composing onto the first answer would give 4.
        assert_eq!(second[0].world_from_object, vec![translation(3., 0., 0.)]);
    }

    /// Endpoint deltas that differ belong to an animated node, which core
    /// never answers; the desktop refuses them too instead of guessing the
    /// samples in between.
    #[test]
    fn differing_endpoint_deltas_fall_back_to_a_full_rebuild() {
        let base = vec![delta_draw(
            1,
            Some("a"),
            vec![translation(0., 0., 0.), translation(1., 0., 0.)],
        )];
        assert!(
            apply_transform_deltas(
                &base,
                &deltas(&[("a", [translation(1., 0., 0.), translation(2., 0., 0.)])]),
                &BTreeMap::new(),
            )
            .is_none()
        );
    }

    /// A mesh with no local bounds on hand still moves the draw's
    /// placement box: the base box carried by the draw moves by the delta —
    /// conservative, and exact for a translation.
    #[test]
    fn draws_without_local_bounds_still_move_their_placement_boxes() {
        let mut draw = delta_draw(7, Some("a"), vec![translation(1., 2., 3.)]);
        draw.bounds = Some(box_bounds([0., 1., 2.], [2., 3., 4.]));
        let base = vec![draw];
        let moved = apply_transform_deltas(
            &base,
            &deltas(&[("a", [translation(10., 0., 0.); 2])]),
            &BTreeMap::new(),
        )
        .expect("a missing local bounds entry falls back, never errors");
        assert_eq!(
            moved[0].bounds,
            Some(box_bounds([10., 1., 2.], [12., 3., 4.]))
        );
    }

    // Apply light deltas in the transform fast path.

    fn delta_light(entity: Option<&str>, at: [f64; 16]) -> protocol::geometry::LightRecord {
        protocol::geometry::LightRecord {
            implementation: "point".into(),
            world_from_light: at,
            source: None,
            entity_id: entity.map(protocol::EntityId::new),
            parameters: vec![],
        }
    }

    fn transform_base(base: Vec<DrawRecord>, lights: Vec<LightRecord>) -> TransformBase {
        TransformBase {
            revision: "0".into(),
            draws: base,
            lights,
            slot: std::path::PathBuf::from("/base-slot"),
        }
    }

    /// Row-major 90° rotation about +z, for pinning the product's order.
    fn rotation_z() -> [f64; 16] {
        [
            0., -1., 0., 0., 1., 0., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]
    }

    /// A fast answer naming a base light retargets its `world_from_light`
    /// by `delta.start · base.world_from_light` — the same row-major product
    /// the draws use — while base draws named by the same answer retarget
    /// exactly like `apply_transform_deltas`; draws and lights the answer
    /// does not name keep their base matrices. The product's ORDER is the
    /// point of the rotation: `delta · base` moves the light, `base · delta`
    /// would not.
    #[test]
    fn light_deltas_retarget_world_from_light_alongside_draws() {
        let base = transform_base(
            vec![delta_draw(1, Some("shape"), vec![translation(1., 0., 0.)])],
            vec![
                delta_light(Some("lamp"), translation(1., 2., 3.)),
                delta_light(Some("still"), translation(9., 9., 9.)),
                delta_light(None, translation(8., 8., 8.)),
            ],
        );
        let moved = retarget_transform_base(
            &base,
            &deltas(&[
                ("shape", [translation(3., 0., 0.); 2]),
                ("lamp", [rotation_z(); 2]),
            ]),
            &BTreeMap::new(),
        )
        .expect("a mixed answer retargets draws and lights");
        assert_eq!(moved.0[0].world_from_object, vec![translation(4., 0., 0.)]);
        assert_eq!(
            moved.1[0].world_from_light,
            row_major_product(rotation_z(), translation(1., 2., 3.)),
            "delta · base world_from_light is the new world_from_light"
        );
        // The product applied to the light's origin: the rotated base
        // position, not the base applied to the rotated origin.
        assert_eq!(
            [
                moved.1[0].world_from_light[3],
                moved.1[0].world_from_light[7],
                moved.1[0].world_from_light[11]
            ],
            [-2., 1., 3.],
        );
        assert_eq!(
            moved.1[1].world_from_light,
            translation(9., 9., 9.),
            "unnamed lights keep their base matrices"
        );
        assert_eq!(moved.1[2].world_from_light, translation(8., 8., 8.));
    }

    /// A delta id naming no base draw and no base light falls back to the
    /// full rebuild — the base may have omitted that entity (a light record
    /// without its pin, a draw the package failed to build), and only a
    /// rebuild can tell.
    #[test]
    fn a_delta_naming_neither_a_base_draw_nor_a_base_light_falls_back() {
        let base = transform_base(
            vec![delta_draw(1, Some("a"), vec![translation(1., 0., 0.)])],
            vec![delta_light(Some("lamp"), translation(1., 2., 3.))],
        );
        assert!(
            retarget_transform_base(
                &base,
                &deltas(&[("ghost", [translation(9., 9., 9.); 2])]),
                &BTreeMap::new(),
            )
            .is_none()
        );
        // A moved light whose base record carries no pin names nothing.
        let unpinned = transform_base(vec![], vec![delta_light(None, translation(1., 2., 3.))]);
        assert!(
            retarget_transform_base(
                &unpinned,
                &deltas(&[("lamp", [translation(9., 9., 9.); 2])]),
                &BTreeMap::new(),
            )
            .is_none()
        );
    }

    /// Every answer applies to the BASE lights, never to already-moved
    /// ones, following the same rule as the draws, so several moves in a
    /// row stay exact.
    #[test]
    fn repeated_light_answers_apply_to_the_same_base_lights() {
        let base = transform_base(
            vec![],
            vec![delta_light(Some("lamp"), translation(1., 2., 3.))],
        );
        let first = retarget_transform_base(
            &base,
            &deltas(&[("lamp", [translation(3., 3., 3.); 2])]),
            &BTreeMap::new(),
        )
        .unwrap()
        .1;
        assert_eq!(first[0].world_from_light, translation(4., 5., 6.));
        let second = retarget_transform_base(
            &base,
            &deltas(&[("lamp", [translation(2., 3., 4.); 2])]),
            &BTreeMap::new(),
        )
        .unwrap()
        .1;
        assert_eq!(
            second[0].world_from_light,
            translation(3., 5., 7.),
            "the second move applies to the base, not to the first answer"
        );
    }

    /// Endpoint deltas that differ belong to an animated node, core never
    /// answers them for lights (a light record carries one sample); the
    /// desktop refuses the whole answer instead of guessing.
    #[test]
    fn differing_light_endpoint_deltas_retarget_nothing() {
        let base = transform_base(
            vec![],
            vec![delta_light(Some("lamp"), translation(1., 2., 3.))],
        );
        assert!(
            retarget_transform_base(
                &base,
                &deltas(&[("lamp", [translation(1., 0., 0.), translation(2., 0., 0.)])]),
                &BTreeMap::new(),
            )
            .is_none()
        );
    }
}

/// Merge one page of lazily fetched children into the project tree JSON.
/// `None` means the parent no longer exists (a stale page) and the tree is
/// left untouched.
/// One lazy Hierarchy node from a `view: "scene_graph"` record.
pub fn tree_node(record: &serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "id": record["node_id"],
        "label": record["label"],
        "kind": record["kind"],
        "object": record["object"],
        "active": record["active"],
        "components": record["components"],
        "medium_used": record["medium_used"],
        "medium_bound": record["medium_bound"],
        "medium_sigma_t": record["medium_sigma_t"],
        "render_camera": record["render_camera"],
        "child_count": record["child_count"],
        "loaded": false,
        "children": [],
        "more_offset": serde_json::Value::Null,
    })
}

pub fn merge_project_children(
    tree: &mut serde_json::Value,
    parent: &str,
    offset: u64,
    records: &[serde_json::Value],
    next_offset: Option<u64>,
) -> Option<()> {
    use serde_json::Value;
    fn find<'a>(entries: &'a mut [Value], parent: &str) -> Option<&'a mut Value> {
        for entry in entries.iter_mut() {
            if entry["id"].as_str() == Some(parent) {
                return Some(entry);
            }
            if let Some(children) = entry
                .as_object_mut()
                .and_then(|object| object.get_mut("children"))
                .and_then(Value::as_array_mut)
            {
                if let Some(found) = find(children, parent) {
                    return Some(found);
                }
            }
        }
        None
    }
    let entries = tree.as_object_mut()?.get_mut("nodes")?.as_array_mut()?;
    let node = find(entries, parent)?;
    let object = node.as_object_mut()?;
    // A revision refresh replaces page 0. Children that still exist keep their
    // already loaded subtree so nested branches do not
    // collapse while their own refresh is still queued.
    let previous: std::collections::HashMap<String, Value> = if offset == 0 {
        object["children"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|child| Some((child["id"].as_str()?.to_owned(), child.clone())))
            .collect()
    } else {
        std::collections::HashMap::new()
    };
    let mut converted: Vec<Value> = records.iter().map(tree_node).collect();
    if offset == 0 {
        for child in &mut converted {
            let Some(old) = child["id"].as_str().and_then(|id| previous.get(id)) else {
                continue;
            };
            if old["loaded"] != true {
                continue;
            }
            for key in ["children", "more_offset", "loaded", "stale"] {
                if let Some(value) = old.get(key) {
                    child[key] = value.clone();
                }
            }
        }
        object.insert("children".into(), Value::Array(converted));
        object.remove("stale");
    } else {
        object
            .get_mut("children")?
            .as_array_mut()?
            .extend(converted);
    }
    object.insert("loaded".into(), serde_json::json!(true));
    object.insert(
        "more_offset".into(),
        next_offset
            .map(|offset| serde_json::json!(offset))
            .unwrap_or(Value::Null),
    );
    Some(())
}

#[cfg(test)]
mod merge_tests {
    use super::*;

    fn record(node_id: &str) -> serde_json::Value {
        serde_json::json!({"node_id":node_id,"label":"Shape","kind":"entity","child_count":0})
    }

    #[test]
    fn test_merge_project_children_pages_branches_and_ignores_stale_parents() {
        let mut tree = serde_json::json!({"nodes":[
            {"id":"world","label":"World","kind":"scope","child_count":3,
             "loaded":false,"children":[],"more_offset":null}
        ]});
        let first = vec![record("shape-1"), record("shape-2")];
        assert!(merge_project_children(&mut tree, "world", 0, &first, Some(2)).is_some());
        assert_eq!(tree["nodes"][0]["loaded"], true);
        assert_eq!(tree["nodes"][0]["children"].as_array().unwrap().len(), 2);
        assert_eq!(tree["nodes"][0]["children"][0]["loaded"], false);
        assert_eq!(tree["nodes"][0]["more_offset"], 2);
        let second = vec![record("shape-3")];
        assert!(merge_project_children(&mut tree, "world", 2, &second, None).is_some());
        assert_eq!(tree["nodes"][0]["children"].as_array().unwrap().len(), 3);
        assert!(tree["nodes"][0]["more_offset"].is_null());
        // Offset 0 replaces: a refresh must not duplicate children.
        assert!(merge_project_children(&mut tree, "world", 0, &first, Some(2)).is_some());
        assert_eq!(tree["nodes"][0]["children"].as_array().unwrap().len(), 2);
        // Stale pages for a removed parent are dropped untouched.
        assert!(merge_project_children(&mut tree, "gone", 0, &first, None).is_none());
    }

    #[test]
    fn refresh_keeps_nested_loaded_branches_drops_removed_children_and_clears_stale() {
        let mut tree = serde_json::json!({"nodes":[
            {"id":"world","label":"World","kind":"scope","child_count":3,
             "loaded":true,"more_offset":null,"stale":true,"children":[
                {"id":"attribute","label":"Old attribute","kind":"scope","child_count":1,
                 "loaded":true,"more_offset":null,"stale":true,"children":[
                    {"id":"shape","label":"Shape","kind":"entity","child_count":0,
                     "loaded":false,"children":[],"more_offset":null,"stale":true}
                 ]},
                {"id":"removed","label":"Removed","kind":"entity","child_count":0,
                 "loaded":false,"children":[],"more_offset":null,"stale":true}
             ]}
        ]});
        let records = vec![
            serde_json::json!({"node_id":"attribute","label":"Attribute","kind":"scope","child_count":1}),
            serde_json::json!({"node_id":"added","label":"Added","kind":"entity","child_count":0}),
        ];
        assert!(merge_project_children(&mut tree, "world", 0, &records, Some(2)).is_some());
        let world = &tree["nodes"][0];
        assert!(world.get("stale").is_none());
        assert_eq!(world["children"].as_array().unwrap().len(), 2);
        assert_eq!(world["children"][0]["id"], "attribute");
        assert_eq!(world["children"][0]["label"], "Attribute");
        // The nested page is retained and stays stale until it refreshes.
        assert_eq!(world["children"][0]["loaded"], true);
        assert_eq!(world["children"][0]["stale"], true);
        assert_eq!(
            world["children"][0]["children"][0]["id"], "shape",
            "loaded grandchildren must survive a parent refresh"
        );
        // Removed children disappear, new ones appear in response order.
        assert_eq!(world["children"][1]["id"], "added");
        assert_eq!(world["more_offset"], 2);
    }
}
