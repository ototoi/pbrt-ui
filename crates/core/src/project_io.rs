//! Serial file-I/O executor. Store locks live here; editing sessions live in core.
use crate::preview_timing;
use crate::state::project_commands::{IoCommand, IoOutput, IoTask, Work};
use project::{FolderStore, ProjectId, ProjectSession, ProjectStore, Revision};
use protocol::{ErrorCode, ErrorPayload, JobId, project::ProjectMode};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::mpsc::{Receiver, Sender},
    time::Instant,
};
use uuid::Uuid;

pub(crate) enum Completion {
    Started(JobId),
    Finished(JobId, Result<IoOutput, ErrorPayload>),
    DefaultSceneFailed {
        store_id: Uuid,
        scene: Option<project::SceneId>,
        error: ErrorPayload,
    },
}
pub(crate) fn run(commands: Receiver<IoCommand>, done: Sender<Completion>) {
    let mut stores = HashMap::<Uuid, ProjectStore>::new();
    let mut staging = HashMap::<Uuid, std::sync::Arc<tempfile::TempDir>>::new();
    while let Ok(command) = commands.recv() {
        let task = match command {
            IoCommand::Release(id) => {
                stores.remove(&id);
                staging.remove(&id);
                continue;
            }
            IoCommand::SetDefaultScene { store_id, scene } => {
                let result = stores
                    .get_mut(&store_id)
                    .ok_or_else(|| {
                        ErrorPayload::new(
                            ErrorCode::StaleProject,
                            "project store has been released",
                        )
                    })
                    .and_then(|store| {
                        store
                            .set_default_scene(scene)
                            .map(|_| ())
                            .map_err(project_error)
                    });
                if let Err(error) = result {
                    let _ = done.send(Completion::DefaultSceneFailed {
                        store_id,
                        scene,
                        error,
                    });
                }
                continue;
            }
            IoCommand::Run(task) => task,
        };
        let id = task.control.job_id;
        let _ = done.send(Completion::Started(id));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            execute(task, &mut stores, &mut staging)
        }))
        .unwrap_or_else(|_| {
            Err(ErrorPayload::new(
                ErrorCode::WorkerCrashed,
                "project I/O adapter panicked",
            ))
        });
        let _ = done.send(Completion::Finished(id, result));
    }
    drop(stores);
    drop(staging);
}
/// Where a folder store keeps asset files.
fn asset_root(store: &ProjectStore) -> Option<String> {
    match store {
        ProjectStore::Folder(folder) => folder
            .directory()
            .join("Assets")
            .to_str()
            .map(str::to_owned),
        ProjectStore::Legacy(_) => None,
    }
}
fn execute(
    task: IoTask,
    stores: &mut HashMap<Uuid, ProjectStore>,
    staging: &mut HashMap<Uuid, std::sync::Arc<tempfile::TempDir>>,
) -> Result<IoOutput, ErrorPayload> {
    let cancelled = || task.control.is_cancelled();
    if cancelled() {
        return Err(ErrorPayload::new(
            ErrorCode::Cancelled,
            "project job cancelled before launch",
        ));
    }
    match task.work {
        Work::ArrayEdit {
            store_id,
            snapshot,
            data,
        } => {
            let Some(data) = data else {
                return Ok(IoOutput::ArrayPrepared { lease: None });
            };
            let store = stores.get(&store_id).ok_or_else(|| {
                ErrorPayload::new(ErrorCode::StaleProject, "project store has been released")
            })?;
            let staging_base = store.directory().join("Library/Staging");
            // The source gone (from the folder or the history's staging) is
            // the same missing asset file as anywhere else; other staging
            // failures are not about the asset and pass through.
            let missing = |error: project::ProjectError| match &error {
                project::ProjectError::AssetFileMissing(_) => {
                    asset_missing(data.asset, &data.location)
                }
                _ => project_error(error),
            };
            let source = snapshot.asset_path(store, data.asset).map_err(missing)?;
            if !source.is_file() {
                return Err(asset_missing(data.asset, &data.location));
            }
            let mut staging =
                project::ImportStaging::new_in(&staging_base).map_err(project_error)?;
            let lease = staging
                .stage_patched_cancellable(
                    data.new_id,
                    &source,
                    data.size,
                    &data.sha256,
                    &data.patches,
                    cancelled,
                )
                // Bytes unlike the record: the project file changed outside
                // the application, named as the folder names it.
                .map_err(|error| match error {
                    project::ProjectError::Io(ref io)
                        if io.kind() == std::io::ErrorKind::InvalidData =>
                    {
                        project_error(project::ProjectError::SourceChanged(format!(
                            "Assets/{}",
                            data.location
                        )))
                    }
                    error => project_error(error),
                })?;
            Ok(IoOutput::ArrayPrepared { lease: Some(lease) })
        }
        Work::AddImport {
            source,
            scene,
            staging_base,
        } => {
            let (batch, diagnostics) = if scene {
                let (batch, report) = pbrt_io::prepare_scene_in(
                    Path::new(&source),
                    &Default::default(),
                    &cancelled,
                    &staging_base,
                )
                .map_err(interchange_error)?;
                crate::import_prepare::validate_batch(&batch, &cancelled).map_err(project_error)?;
                (batch, report.diagnostics)
            } else {
                (
                    crate::import_prepare::resource_in(
                        Path::new(&source),
                        &staging_base,
                        &cancelled,
                    )
                    .map_err(project_error)?,
                    Vec::new(),
                )
            };
            Ok(IoOutput::ImportPrepared { batch, diagnostics })
        }
        Work::Import {
            source,
            destination,
        } => {
            let (store, session, report, path, source_path, published, mode, temp) =
                if let Some(destination) = destination {
                    let (store, session, report) = pbrt_io::import_project_with_report(
                        Path::new(&source),
                        Path::new(&destination),
                        &Default::default(),
                        &cancelled,
                    )
                    .map_err(interchange_error)?;
                    let path = store
                        .directory()
                        .to_str()
                        .ok_or_else(|| {
                            ErrorPayload::new(
                                ErrorCode::UnsupportedPathEncoding,
                                "project path is not UTF-8",
                            )
                        })?
                        .to_owned();
                    (
                        store,
                        session,
                        report,
                        Some(path),
                        None,
                        true,
                        ProjectMode::Project,
                        None,
                    )
                } else {
                    let cache = project::cache_directory().map_err(|error| {
                        ErrorPayload::new(ErrorCode::IoError, error.to_string())
                    })?;
                    let temporary = project::viewer_tempdir_in(&cache).map_err(|error| {
                        ErrorPayload::new(ErrorCode::IoError, error.to_string())
                    })?;
                    let title = Path::new(&source)
                        .file_stem()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Imported scene");
                    let mut folder = FolderStore::create(
                        &temporary.path().join("project"),
                        ProjectId::new(),
                        title,
                        Revision::initial(),
                    )
                    .map_err(project_error)?;
                    let (session, report) = pbrt_io::import_into(
                        Path::new(&source),
                        &mut folder,
                        &Default::default(),
                        &cancelled,
                    )
                    .map_err(interchange_error)?;
                    (
                        ProjectStore::Folder(folder),
                        session,
                        report,
                        None,
                        Some(source),
                        false,
                        ProjectMode::Viewer,
                        Some(temporary),
                    )
                };
            let id = Uuid::new_v4();
            let snapshot = session.snapshot();
            let asset_root = asset_root(&store);
            let unregistered = store
                .unregistered_files()
                .map_err(project_error)?
                .into_iter()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect();
            stores.insert(id, store);
            let staging_owner = temp.map(std::sync::Arc::new);
            if let Some(owner) = &staging_owner {
                staging.insert(id, owner.clone());
            }
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: id,
                path,
                source_path,
                unregistered,
                published,
                report: Some(report),
                mode,
                asset_root,
                staging_owner,
            })
        }
        Work::Open { path } => {
            let (store, session) = ProjectStore::open(Path::new(&path)).map_err(project_error)?;
            let id = Uuid::new_v4();
            let store_path = store
                .directory()
                .to_str()
                .ok_or_else(|| {
                    ErrorPayload::new(
                        ErrorCode::UnsupportedPathEncoding,
                        "project path is not UTF-8",
                    )
                })?
                .to_owned();
            let snapshot = session.snapshot();
            let mode = if store.is_read_only() {
                ProjectMode::Viewer
            } else {
                ProjectMode::Project
            };
            let path = (mode == ProjectMode::Project).then_some(store_path.clone());
            let asset_root = asset_root(&store);
            let unregistered = store
                .unregistered_files()
                .map_err(project_error)?
                .into_iter()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect();
            stores.insert(id, store);
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: id,
                path,
                source_path: (mode == ProjectMode::Viewer).then(|| {
                    Path::new(&store_path)
                        .join("project.pbrtui.json")
                        .to_string_lossy()
                        .into_owned()
                }),
                unregistered,
                published: false,
                report: None,
                mode,
                asset_root,
                staging_owner: None,
            })
        }
        Work::Create {
            path,
            title,
            coordinate_system,
        } => {
            // the destination must be new or an empty folder.
            let destination = Path::new(&path);
            let empty = match std::fs::read_dir(destination) {
                Ok(mut entries) => entries.next().is_none(),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => true,
                Err(error) => return Err(ErrorPayload::new(ErrorCode::IoError, error.to_string())),
            };
            if !empty {
                return Err(ErrorPayload::new(
                    ErrorCode::InvalidRequest,
                    "project.create needs a new or empty folder",
                ));
            }
            // The initial scene goes through the same edit machinery the
            // Create menu uses, then round-trips through a save.
            let document = project::presets::initial_document(&title, coordinate_system)
                .map_err(project_error)?;
            let mut session = ProjectSession::new(document).map_err(project_error)?;
            let mut store = project::FolderStore::create_v3(
                destination,
                session.document().id,
                &title,
                session.revision(),
            )
            .map_err(project_error)?;
            let receipt = store.save(&session.snapshot()).map_err(project_error)?;
            session.mark_saved(&receipt).map_err(project_error)?;
            let id = Uuid::new_v4();
            let store = project::ProjectStore::Folder(store);
            let store_path = store
                .directory()
                .to_str()
                .ok_or_else(|| {
                    ErrorPayload::new(
                        ErrorCode::UnsupportedPathEncoding,
                        "project path is not UTF-8",
                    )
                })?
                .to_owned();
            let snapshot = session.snapshot();
            let asset_root = asset_root(&store);
            let unregistered = store
                .unregistered_files()
                .map_err(project_error)?
                .into_iter()
                .map(|path| path.to_string_lossy().replace('\\', "/"))
                .collect();
            stores.insert(id, store);
            // The folder is written, so a cancellation after this point
            // reports the project as published like a persistent import.
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: id,
                path: Some(store_path),
                source_path: None,
                unregistered,
                published: true,
                report: None,
                mode: ProjectMode::Project,
                asset_root,
                staging_owner: None,
            })
        }
        Work::Save {
            store_id,
            snapshot,
            destination,
            retained,
        } => {
            if let Some(destination) = destination {
                let store = stores.get(&store_id).ok_or_else(|| {
                    ErrorPayload::new(ErrorCode::StaleProject, "project store has been released")
                })?;
                let (saved, receipt) = store
                    .save_as_cancellable(Path::new(&destination), &snapshot, cancelled)
                    .map_err(project_error)?;
                let registered = saved.read_document().map_err(project_error)?;
                let path = saved
                    .directory()
                    .to_str()
                    .ok_or_else(|| {
                        ErrorPayload::new(
                            ErrorCode::UnsupportedPathEncoding,
                            "project path is not UTF-8",
                        )
                    })?
                    .to_owned();
                let relocated = project::RelocatedPayloads::prepare(
                    &retained,
                    &saved.directory().join("Assets"),
                )
                .ok();
                let new_store_id = Uuid::new_v4();
                stores.insert(new_store_id, saved);
                Ok(IoOutput::SavedAs {
                    receipt,
                    registered,
                    store_id: new_store_id,
                    path,
                    relocated,
                })
            } else {
                let store = stores.get_mut(&store_id).ok_or_else(|| {
                    ErrorPayload::new(ErrorCode::StaleProject, "project store has been released")
                })?;
                let receipt = store
                    .save_cancellable(&snapshot, cancelled)
                    .map_err(project_error)?;
                let registered = store.read_document().map_err(project_error)?;
                Ok(IoOutput::Saved {
                    receipt,
                    registered,
                })
            }
        }
        Work::Export {
            store_id,
            snapshot,
            destination,
        } => {
            let store = stores.get(&store_id).ok_or_else(|| {
                ErrorPayload::new(ErrorCode::StaleProject, "project store has been released")
            })?;
            let path =
                pbrt_io::export_project(&snapshot, store, Path::new(&destination), cancelled)
                    .map_err(interchange_error)?;
            Ok(IoOutput::Exported {
                path: path.to_string_lossy().into_owned(),
                warnings: pbrt_io::export_warnings(&snapshot).map_err(interchange_error)?,
            })
        }
        Work::Preview {
            store_id,
            snapshot,
            destination,
            store: content_store,
            geometry_cache,
            stage_assets,
        } => {
            // Per-stage records are opt-in through PBRT_UI_PREVIEW_TIMING.
            // keyed by job id only. `total` spans the whole job, including
            // the manifest write and the atomic publish.
            let total = Instant::now();
            let job_id = task.control.job_id;
            let store = stores.get(&store_id).ok_or_else(|| {
                ErrorPayload::new(ErrorCode::StaleProject, "project store has been released")
            })?;
            let destination = PathBuf::from(&destination);
            let parent = destination
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from("."));
            std::fs::create_dir_all(&parent)
                .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            let temporary = crate::services::private_tempdir_in(&parent)
                .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            let staged = temporary.path().join("bundle");
            // The published slot keeps this mode, and it holds asset links.
            crate::services::create_private_dir_all(&staged)
                .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            // Same seam the text export uses, composed either of a
            // staged, verified `assets/` copy (the external callers'
            // self-contained bundle) or of the assets'
            // real paths in the project store, read in place with no
            // copy, no sha256 and no fsync along the way.
            let document = snapshot.document();
            let content_store = content_store.map(PathBuf::from);
            let stored_blobs = if let Some(content_store) = &content_store {
                crate::services::create_private_dir_all(content_store)
                    .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
                std::fs::read_dir(content_store)
                    .ok()
                    .into_iter()
                    .flatten()
                    .filter_map(Result::ok)
                    .filter_map(|entry| {
                        let name = entry.file_name().into_string().ok()?;
                        let metadata = entry.metadata().ok()?;
                        let size = metadata.is_file().then_some(metadata.len())?;
                        (name.len() == 64 && name.bytes().all(|byte| byte.is_ascii_hexdigit()))
                            .then_some((name, size))
                    })
                    .collect()
            } else {
                std::collections::BTreeMap::new()
            };
            let started = Instant::now();
            let assets = if stage_assets {
                pbrt_io::stage_snapshot_assets(&snapshot, store, &staged, &cancelled)
                    .map_err(interchange_error)?
            } else {
                pbrt_io::asset_store_paths(&snapshot, store).map_err(interchange_error)?
            };
            // The model's recorded SHA-256 per program filename keys the tessellation cache without reading the file and
            // answers the imagemap / image-light blob records. A staged
            // copy was hash-verified during the copy itself.
            let asset_sha256 = document
                .assets
                .iter()
                .filter_map(|(id, asset)| {
                    assets
                        .get(id)
                        .map(|filename| (filename.clone(), asset.sha256.clone()))
                })
                .collect();
            preview_timing::record("stage_assets", started, job_id);
            let started = Instant::now();
            let program = pbrt_io::program_from_project(&snapshot, store, &assets, cancelled)
                .map_err(interchange_error)?;
            preview_timing::record("program", started, job_id);
            let origins = program.origins.clone();
            let mut local_cache;
            let mut shared_cache;
            let cache: &mut scene_parse::GeometryCache = match &geometry_cache {
                Some(shared) => {
                    shared_cache = shared.lock().map_err(|_| {
                        ErrorPayload::new(ErrorCode::Busy, "geometry cache lock poisoned")
                    })?;
                    &mut shared_cache
                }
                None => {
                    local_cache = scene_parse::GeometryCache::default();
                    &mut local_cache
                }
            };
            // This job holds the cache lock for the whole geometry stage, so
            // the counter deltas below are exactly this job's hits/misses.
            let cache_before = cache.stats();
            let started = Instant::now();
            let preview = scene_parse::preview_from_program(
                program,
                Some(&origins),
                &staged,
                scene_parse::ParseOptions {
                    asset_sha256,
                    stored_blobs,
                    ..Default::default()
                },
                cache,
            )
            .map_err(|error| ErrorPayload::new(ErrorCode::ParseFailed, error.to_string()))?;
            let cache_after = cache.stats();
            preview_timing::record_geometry_cache(
                "geometry",
                started,
                job_id,
                cache_after.hits - cache_before.hits,
                cache_after.misses - cache_before.misses,
                cache_after.asset_reads - cache_before.asset_reads,
            );
            let started = Instant::now();
            // The slot is disposable: atomic publish, no
            // fsync on the way.
            scene_parse::export::write_preview_with_store(
                &preview,
                &staged,
                false,
                content_store.as_deref(),
            )
            .map_err(|error| ErrorPayload::new(ErrorCode::ParseFailed, error.to_string()))?;
            // Audit record for tests; consumers read only the package
            // (geometry.json + blobs) and the manifest's entity pins.
            let node_sources: serde_json::Map<String, serde_json::Value> = origins
                .iter()
                .map(|(line, entity)| {
                    (
                        line.to_string(),
                        serde_json::Value::String(entity.as_str().to_owned()),
                    )
                })
                .collect();
            let manifest = serde_json::json!({
                "kind": "program",
                "node_sources": node_sources,
                "project": document.id,
                "revision": snapshot.revision(),
            });
            let manifest_file = staged.join("preview.json");
            std::fs::write(
                &manifest_file,
                serde_json::to_vec_pretty(&manifest)
                    .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?,
            )
            .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            // `write` spans write_preview plus this manifest file; publish
            // stays separate so `total` matches the job's whole duration.
            preview_timing::record("write", started, job_id);
            // Last cancellation checkpoint, same seam as the text export.
            if cancelled() {
                return Err(ErrorPayload::new(
                    ErrorCode::Cancelled,
                    "project preview cancelled before publish",
                ));
            }
            pbrt_io::publish(&staged, &destination).map_err(interchange_error)?;
            if let Some(content_store) = &content_store {
                let prune = Instant::now();
                let referenced: std::collections::BTreeSet<String> = preview
                    .manifest
                    .blobs
                    .values()
                    .map(|file| file.sha256.clone())
                    .collect();
                prune_preview_store(content_store, &referenced);
                preview_timing::record("prune", prune, job_id);
            }
            preview_timing::record("total", total, job_id);
            Ok(IoOutput::Previewed {
                path: destination.to_string_lossy().into_owned(),
                draws: preview.manifest.draws.len(),
                // The package's warnings (geometry_failed and friends) join
                // the manifest's own unsupported list in the one number the
                // wire already carries.
                unsupported: preview.manifest.unsupported.len() + preview.warning_count(),
                warnings: pbrt_io::export_warnings(&snapshot).map_err(interchange_error)?,
            })
        }
    }
}

fn prune_preview_store(store: &Path, referenced: &std::collections::BTreeSet<String>) {
    if let Ok(entries) = std::fs::read_dir(store) {
        for entry in entries.flatten() {
            // Only this writer's blobs and temp files: the caller picks the
            // directory, so anything else in it is not ours to delete.
            if let Some(name) = entry.file_name().to_str()
                && (name.starts_with(".preview-blob-")
                    || (name.len() == 64
                        && name.bytes().all(|byte| byte.is_ascii_hexdigit())
                        && !referenced.contains(name)))
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}

pub(crate) fn project_error(error: project::ProjectError) -> ErrorPayload {
    use project::ProjectError as E;
    let code = match &error {
        E::RevisionConflict { .. } => ErrorCode::RevisionConflict,
        E::Locked => ErrorCode::ProjectLocked,
        E::UnsupportedVersion(_) | E::SaveUnavailable => ErrorCode::Unsupported,
        E::Cancelled => ErrorCode::Cancelled,
        E::SourceChanged(_) => ErrorCode::SourceChanged,
        E::ResourceLimit(_) => ErrorCode::ResourceExhausted,
        E::Invalid(_) | E::ResourceCycle => ErrorCode::InvalidRequest,
        E::Io(_) | E::Json(_) | E::Durability { .. } | E::AssetFileMissing(_) => ErrorCode::IoError,
    };
    let mut result = ErrorPayload::new(code, error.to_string());
    if matches!(&error, E::Invalid(message) if message == "no active scene") {
        result.details = Some(serde_json::json!({"reason":"no_active_scene"}));
    }
    if let E::Durability { revision, .. } = &error {
        result.details = Some(serde_json::json!({"published":true,"revision":revision}));
    }
    // An edit may close a reference loop, regardless of which edit it was.
    if let E::ResourceCycle = &error {
        result.details = Some(serde_json::json!({"reason":"resource_cycle"}));
    }
    if let E::SourceChanged(path) = &error {
        result.details = Some(serde_json::json!({"files":[path]}));
    }
    result
}
/// An asset whose file is gone (moved or deleted outside the application).
fn asset_missing(asset: project::AssetId, location: &str) -> ErrorPayload {
    let mut payload = ErrorPayload::new(
        ErrorCode::IoError,
        format!("asset file is missing from the project folder: {location}"),
    )
    .with_details(serde_json::json!({
        "reason":"asset_file_missing","asset_id":asset,"location":location,
    }));
    payload.target_id = Some(asset.to_string());
    payload
}
fn interchange_error(error: pbrt_io::Error) -> ErrorPayload {
    match error {
        pbrt_io::Error::Project(error) => project_error(error),
        pbrt_io::Error::Unsupported(message) => ErrorPayload::new(ErrorCode::Unsupported, message),
        pbrt_io::Error::Invalid(message) => ErrorPayload::new(ErrorCode::ParseFailed, message),
        pbrt_io::Error::MissingRequired { message, missing } => {
            ErrorPayload::new(ErrorCode::InvalidRequest, message)
                .with_details(serde_json::json!({"reason":"missing_required","missing":missing}))
        }
        pbrt_io::Error::AssetMissing { asset, location } => asset_missing(asset, &location),
        // The texture's image reference is what fails to resolve.
        error @ pbrt_io::Error::ImagemapWithoutImage { resource, .. } => {
            let mut payload = ErrorPayload::new(ErrorCode::ReferenceUnresolved, error.to_string())
                .with_details(serde_json::json!({
                    "reason":"imagemap_without_image","resource_id":resource,
                }));
            payload.target_id = Some(resource.to_string());
            payload
        }
        pbrt_io::Error::SourceChanged(path) => ErrorPayload::new(
            ErrorCode::SourceChanged,
            format!("input changed during import: {path}"),
        ),
        pbrt_io::Error::Parse(error) => {
            ErrorPayload::new(ErrorCode::ParseFailed, error.to_string())
        }
        pbrt_io::Error::Io(error) => ErrorPayload::new(ErrorCode::IoError, error.to_string()),
        pbrt_io::Error::Published { directory, source } => {
            ErrorPayload::new(ErrorCode::IoError, source.to_string())
                .with_details(serde_json::json!({"published":true,"path":directory}))
        }
    }
}

#[cfg(test)]
mod preview_store_tests {
    use super::prune_preview_store;
    use std::{collections::BTreeSet, fs};

    #[test]
    fn pruning_unreferenced_store_entries_keeps_published_hardlinks() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("store");
        let bundle = dir.path().join("published");
        fs::create_dir_all(&store).unwrap();
        fs::create_dir_all(&bundle).unwrap();
        let used = "a".repeat(64);
        let old = "b".repeat(64);
        fs::write(store.join(&used), b"live").unwrap();
        fs::write(store.join(&old), b"old").unwrap();
        fs::write(store.join(".preview-blob-x1"), b"tmp").unwrap();
        fs::write(store.join("unrelated.cache"), b"keep").unwrap();
        fs::write(store.join(".hidden"), b"keep").unwrap();
        fs::hard_link(store.join(&used), bundle.join("blob")).unwrap();
        prune_preview_store(&store, &BTreeSet::from([used.clone()]));
        assert!(store.join(&used).is_file());
        assert!(!store.join(&old).exists());
        assert!(!store.join(".preview-blob-x1").exists());
        assert!(
            store.join("unrelated.cache").is_file(),
            "files the writer does not own stay"
        );
        assert!(store.join(".hidden").is_file());
        assert_eq!(fs::read(bundle.join("blob")).unwrap(), b"live");
    }
}
