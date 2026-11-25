use super::{asset_preview, material_preview, texture_preview};
use protocol::{ErrorCode, ErrorPayload, Response};
use serde_json::json;

/// Which preview a `project.preview_*` method generates, and the id field
/// the reply reports it under.
pub(crate) enum PreviewTarget {
    Texture(uuid::Uuid),
    Material(uuid::Uuid),
    Asset(uuid::Uuid),
}

impl PreviewTarget {
    /// The reply's id field: `resource_id` for resources, `asset_id` for
    /// assets.
    fn id_field(&self) -> &'static str {
        match self {
            PreviewTarget::Texture(_) | PreviewTarget::Material(_) => "resource_id",
            PreviewTarget::Asset(_) => "asset_id",
        }
    }
}

/// Immutable input captured by the core driver. Snapshot clones retain staged
/// payload leases without scanning or staging every asset.
pub(crate) struct PreparedPreview {
    pub request_id: String,
    pub project: protocol::project::ProjectRef,
    pub target: PreviewTarget,
    pub size: u32,
    pub snapshot: project::ProjectSnapshot,
    pub asset_root: Option<String>,
    pub cache_root: std::path::PathBuf,
    pub _staging_owner: Option<std::sync::Arc<tempfile::TempDir>>,
}

impl PreparedPreview {
    pub(crate) fn execute(self, stopping: &std::sync::atomic::AtomicBool) -> Response {
        let result = self.execute_inner(stopping);
        match result {
            Ok(value) => Response::ok(&self.request_id, value),
            Err(error) => Response::err(self.request_id, error),
        }
    }

    pub(crate) fn execute_inner(
        &self,
        stopping: &std::sync::atomic::AtomicBool,
    ) -> Result<serde_json::Value, ErrorPayload> {
        ensure_running(stopping)?;
        let document = self.snapshot.document();
        let asset_root = self.asset_root.as_deref();
        let (item, key) = match &self.target {
            PreviewTarget::Texture(id) => (
                id.to_string(),
                texture_preview::key(document, project::ResourceId(*id), asset_root, self.size)
                    .map_err(invalid)?,
            ),
            PreviewTarget::Material(id) => (
                id.to_string(),
                material_preview::key(
                    document,
                    Some(&self.snapshot),
                    project::ResourceId(*id),
                    asset_root,
                    self.size,
                )
                .map_err(invalid)?,
            ),
            PreviewTarget::Asset(id) => {
                let asset = document
                    .assets
                    .get(&project::AssetId(*id))
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "asset not found"))?;
                (id.to_string(), asset_preview::key(asset, self.size))
            }
        };
        ensure_running(stopping)?;
        let cache = project::cache_get(&self.cache_root, &item, &key)
            .map_err(crate::project_io::project_error)?;
        ensure_running(stopping)?;
        let (path, cached) = match cache {
            Some(hit) if valid_png(&hit.path, self.size) => (hit.path, true),
            _ => {
                ensure_running(stopping)?;
                let png = match &self.target {
                    PreviewTarget::Texture(id) => {
                        let tree = texture_preview::tree_from_snapshot(
                            &self.snapshot,
                            project::ResourceId(*id),
                            asset_root,
                        )
                        .map_err(invalid)?;
                        texture_preview::png(&tree, self.size)
                    }
                    PreviewTarget::Material(id) => {
                        let tree = material_preview::tree_from_snapshot(
                            &self.snapshot,
                            project::ResourceId(*id),
                            asset_root,
                        )
                        .map_err(invalid)?;
                        material_preview::png(&tree, self.size)
                    }
                    PreviewTarget::Asset(id) => {
                        let asset =
                            document.assets.get(&project::AssetId(*id)).ok_or_else(|| {
                                ErrorPayload::new(ErrorCode::NotFound, "asset not found")
                            })?;
                        asset_preview::png(&self.snapshot, asset_root, asset, self.size)
                    }
                }
                .map_err(invalid)?;
                ensure_running(stopping)?;
                let path = project::cache_put(
                    &self.cache_root,
                    &item,
                    &key,
                    "preview",
                    "image/png",
                    &png,
                    project::DEFAULT_CACHE_LIMIT,
                )
                .map_err(crate::project_io::project_error)?;
                (path, false)
            }
        };
        let mut value = json!({
            "project": self.project,
            "path": path.to_string_lossy().replace('\\', "/"),
            "key": key,
            "cached": cached,
        });
        value
            .as_object_mut()
            .expect("the reply body is an object")
            .insert(self.target.id_field().into(), json!(item));
        Ok(value)
    }
}

fn ensure_running(stopping: &std::sync::atomic::AtomicBool) -> Result<(), ErrorPayload> {
    if stopping.load(std::sync::atomic::Ordering::Acquire) {
        Err(ErrorPayload::new(
            ErrorCode::IoError,
            "host is shutting down",
        ))
    } else {
        Ok(())
    }
}

fn invalid(message: String) -> ErrorPayload {
    ErrorPayload::new(ErrorCode::InvalidRequest, message)
}

fn valid_png(path: &std::path::Path, size: u32) -> bool {
    let Ok(file) = std::fs::File::open(path) else {
        return false;
    };
    let reader = image::io::Reader::new(std::io::BufReader::new(file));
    let Ok(mut reader) = reader.with_guessed_format() else {
        return false;
    };
    if reader.format() != Some(image::ImageFormat::Png) {
        return false;
    }
    let mut limits = image::io::Limits::default();
    limits.max_image_width = Some(size);
    limits.max_image_height = Some(size);
    limits.max_alloc = Some((size as u64) * (size as u64) * 8);
    reader.limits(limits);
    let Ok(decoded) = reader.decode() else {
        return false;
    };
    decoded.width() == size && decoded.height() == size
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, mpsc};

    #[test]
    fn prepared_preview_keeps_viewer_staging_alive_until_task_drop() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().to_path_buf();
        let owner = Arc::new(temporary);
        let snapshot = project::ProjectSession::new(project::ProjectDocument::new("preview"))
            .unwrap()
            .snapshot();
        let task = PreparedPreview {
            request_id: "preview".into(),
            project: protocol::project::ProjectRef {
                project_id: uuid::Uuid::new_v4(),
                revision: protocol::Revision::from_u64(0),
                activation: protocol::Revision::from_u64(0),
            },
            target: PreviewTarget::Texture(uuid::Uuid::new_v4()),
            size: 32,
            snapshot,
            asset_root: None,
            cache_root: path.join("Library/Cache"),
            _staging_owner: Some(owner.clone()),
        };
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            let _task = task;
            started_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        });
        drop(owner);
        started_rx.recv().unwrap();
        assert!(path.is_dir(), "the running task owns the Viewer staging");
        release_tx.send(()).unwrap();
        worker.join().unwrap();
        assert!(
            !path.exists(),
            "staging is removed after the last owner drops"
        );
    }
}
