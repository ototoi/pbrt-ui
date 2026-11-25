//! The content identity of a project is a canonical
//! hash of what a save persists, so dirty compares content, not revisions.
//!
//! In: scenes and their nodes, resources (parameters, implementation,
//! placement), definition nodes, assets (placement, hash, size), folders,
//! imports, the project id and title. Out: the communication revision and
//! the per-record editions that move on every edit, the active/default scene
//! view state, and every cache or job status. Asset origins hash in
//! their persisted form — the resume-origin pointer is dropped at save.
use crate::CoordinateSystem;
use crate::model::*;
use serde::Serialize;
use sha2::{Digest, Sha256};

/// The canonical content of one project, as `ContentId::of` hashes it.
#[derive(Serialize)]
struct Canonical<'a> {
    id: &'a ProjectId,
    title: &'a str,
    scenes: serde_json::Map<String, serde_json::Value>,
    definition_nodes: serde_json::Map<String, serde_json::Value>,
    resources: serde_json::Map<String, serde_json::Value>,
    assets: serde_json::Map<String, serde_json::Value>,
    folders: std::collections::BTreeSet<String>,
    imports: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct CanonicalNode<'a> {
    id: &'a NodeId,
    label: &'a str,
    origin: &'a Option<Origin>,
    data: &'a NodeKind,
}

#[derive(Serialize)]
struct CanonicalScene<'a> {
    id: &'a SceneId,
    label: &'a str,
    location: &'a str,
    options: &'a [NodeId],
    render_camera: Option<NodeId>,
    coordinate_system: CoordinateSystem,
    world: NodeId,
    nodes: serde_json::Map<String, serde_json::Value>,
}

#[derive(Serialize)]
struct CanonicalResource<'a> {
    id: &'a ResourceId,
    label: &'a str,
    location: &'a str,
    kind: &'a ResourceKind,
    texture_value_type: &'a Option<TextureValueType>,
    implementation: &'a str,
    parameters: Vec<Parameter>,
    definition_scope: &'a Option<NodeId>,
    declared_in: &'a Option<DeclarationState>,
    origin: &'a Option<Origin>,
}

#[derive(Serialize)]
struct CanonicalAsset<'a> {
    id: &'a AssetId,
    location: &'a str,
    media_type: &'a str,
    sha256: &'a str,
    size: u64,
    /// The persisted origin: the resume pointer (`origin.origin`) is
    /// dropped at save, so it is not content either.
    origin: Option<CanonicalAssetOrigin<'a>>,
}

#[derive(Serialize)]
struct CanonicalAssetOrigin<'a> {
    logical_path: &'a str,
}

fn map<'a, K, T>(
    entries: impl Iterator<Item = (&'a K, &'a T)>,
    project: impl Fn(&'a T) -> serde_json::Value,
) -> serde_json::Map<String, serde_json::Value>
where
    K: PartialOrd + Ord + Clone + std::fmt::Display + 'a,
    T: 'a,
{
    // BTreeMap iteration is key order, so two equal documents hash the same
    // bytes; nested maps inside the projections keep their own order.
    entries
        .map(|(key, value)| (key.to_string(), project(value)))
        .collect()
}

/// The identity of a project's saveable content: a SHA-256 over the
/// canonical form. Content does not change when revisions or
/// editions move, or when the active/default scene view state changes.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ContentId(String);

impl ContentId {
    /// Hash a document's content. The canonical form mirrors what the
    /// folder store persists; the current branch's assets hash with the
    /// resume-origin pointer dropped, as the save does.
    pub fn of(document: &ProjectDocument) -> Self {
        let canonical = Canonical {
            id: &document.id,
            title: &document.title,
            scenes: map(document.scenes.iter(), |scene| {
                to_value(&CanonicalScene {
                    id: &scene.id,
                    label: &scene.label,
                    location: &scene.location,
                    options: &scene.options,
                    render_camera: scene.render_camera,
                    coordinate_system: scene.coordinate_system,
                    world: scene.world,
                    nodes: map(scene.nodes.iter(), |node| {
                        to_value(&CanonicalNode {
                            id: &node.id,
                            label: &node.label,
                            origin: &node.origin,
                            data: &node.data,
                        })
                    }),
                })
            }),
            definition_nodes: map(document.definition_nodes.iter(), |node| {
                to_value(&CanonicalNode {
                    id: &node.id,
                    label: &node.label,
                    origin: &node.origin,
                    data: &node.data,
                })
            }),
            resources: map(document.resources.iter(), |resource| {
                to_value(&CanonicalResource {
                    id: &resource.id,
                    label: &resource.label,
                    location: &resource.location,
                    kind: &resource.kind,
                    texture_value_type: &resource.texture_value_type,
                    implementation: &resource.implementation,
                    parameters: resource.parameters.clone(),
                    definition_scope: &resource.definition_scope,
                    declared_in: &resource.declared_in,
                    origin: &resource.origin,
                })
            }),
            assets: map(document.assets.iter(), |asset| {
                let origin = asset.origin.as_ref().map(|origin| CanonicalAssetOrigin {
                    logical_path: &origin.logical_path,
                });
                to_value(&CanonicalAsset {
                    id: &asset.id,
                    location: &asset.location,
                    media_type: &asset.media_type,
                    sha256: &asset.sha256,
                    size: asset.size,
                    origin,
                })
            }),
            folders: canonical_folders(&document.folders),
            imports: map(document.imports.iter(), |record| {
                serde_json::to_value(record).unwrap_or_default()
            }),
        };
        let bytes = serde_json::to_vec(&canonical).expect("canonical content serializes");
        let mut hasher = Sha256::new();
        hasher.update(bytes);
        Self(hex(&hasher.finalize()))
    }

    /// The hex digest, for wire receipts and records.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn canonical_folders(
    folders: &std::collections::BTreeSet<String>,
) -> std::collections::BTreeSet<String> {
    let mut canonical = std::collections::BTreeSet::new();
    for folder in folders {
        let mut prefix = String::new();
        for component in folder.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(component);
            canonical.insert(prefix.clone());
        }
    }
    canonical
}

impl std::fmt::Display for ContentId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn to_value<T: Serialize>(value: &T) -> serde_json::Value {
    serde_json::to_value(value).expect("canonical content serializes")
}

fn hex(bytes: &[u8]) -> String {
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        let _ = write!(text, "{byte:02x}");
    }
    text
}
