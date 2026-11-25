//! deterministic publication of import jobs. The worker
//! completes off-thread; here a job's `ImportPrepared` output is injected
//! directly so every race in the publish order is reproduced as a sequence.
use super::*;

fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: Uuid::new_v4().to_string(),
        method,
        params,
    }
}

/// An editable project-mode session with one default scene, opened from a
/// synthesized load so no bytes hit the disk unless a test saves.
fn open_editable(core: &mut CoreState, empty: bool) -> Uuid {
    let document = if empty {
        project::ProjectDocument::new_empty("Target")
    } else {
        project::ProjectDocument::new("Target")
    };
    let snapshot = project::ProjectSession::new(document).unwrap().snapshot();
    let store_id = Uuid::new_v4();
    let open = {
        let context = core.projects.context();
        let response = core.handle(request(
            Method::ProjectOpen,
            json!({"path":crate::TEST_PROJECT,"expected_current":context}),
        ));
        assert!(response.error.is_none(), "{:?}", response.error);
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
    };
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id,
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    store_id
}

fn scene_source(name: &str) -> std::path::PathBuf {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.keep().join(name);
    std::fs::write(
        &path,
        "WorldBegin\nShape \"sphere\" \"float radius\" [1]\nWorldEnd\n",
    )
    .unwrap();
    path
}

fn image_source(name: &str) -> std::path::PathBuf {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.keep().join(name);
    image::RgbImage::from_raw(2, 2, vec![0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255])
        .unwrap()
        .save(&path)
        .unwrap();
    path
}

fn ply_source(name: &str) -> std::path::PathBuf {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.keep().join(name);
    std::fs::write(
        &path,
        "ply\nformat ascii 1.0\nelement vertex 4\nproperty float x\nproperty float y\n\
          property float z\nelement face 4\nproperty list uchar int vertex_indices\n\
          end_header\n0 0 0\n1 0 0\n0 1 0\n0 0 1\n3 0 1 2\n3 0 1 3\n3 1 2 3\n3 2 0 3\n",
    )
    .unwrap();
    path
}

fn start_import(
    core: &mut CoreState,
    method: Method,
    source: &std::path::Path,
    folder: Option<&str>,
) -> JobId {
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        method,
        json!({"project":reference,"source_path":source.display().to_string(),"folder":folder}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
}

fn complete_import(core: &mut CoreState, job: JobId, batch: project::PreparedImport) {
    core.project_io_finished(
        job,
        Ok(IoOutput::ImportPrepared {
            batch,
            diagnostics: Vec::new(),
        }),
    );
}

#[test]
fn import_publishes_one_undoable_batch_without_touching_selection_or_view() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let before = core.projects.reference().unwrap();
    let active_before = core
        .projects
        .active
        .as_ref()
        .unwrap()
        .session
        .active_scene();
    let default_before = core
        .projects
        .active
        .as_ref()
        .unwrap()
        .session
        .document()
        .default_scene;
    // The stored shared-view slot must survive publication (project switches
    // reset it; an addition does not), while the shared view itself expires
    // on any content revision exactly like a normal project.apply - see the
    // assertions after the publish below.
    let context = core.projects.context();
    let view = protocol::view::ViewState {
        camera: crate::view::frame(None, [32, 24]),
        size: [32, 24],
        mode: protocol::view::DisplayMode::Solid,
        time: 0.0,
    };
    core.project_view = Some((context, view, 1));

    let source = scene_source("added.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    let identity = batch.document().scenes.keys().copied().collect::<Vec<_>>();
    complete_import(&mut core, job, batch);

    let job = core.job(job).unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    let result = job.result.as_ref().unwrap();
    assert_eq!(result["published"], true);
    assert_eq!(
        result["created"][0]["kind"], "scene",
        "scene imports report their scene: {result}"
    );
    assert!(
        result["created"][0]["location"]
            .as_str()
            .unwrap()
            .starts_with("Scenes/")
    );
    // Still dirty: publication adds to the session, not to the disk store.
    assert!(core.projects.active.as_ref().unwrap().session.is_dirty());
    let after = core.projects.active.as_ref().unwrap();
    assert_eq!(after.session.active_scene(), active_before);
    assert_eq!(after.session.document().default_scene, default_before);
    assert_eq!(after.session.document().scenes.len(), 2);
    // The pinned desktop camera never clears on an addition, but the SHARED
    // view is revision-checked by (content edits make it expire the
    // same way project.apply does, so external captures re-establish it):
    // the publication bumps the revision and leaves the stored slot alone.
    assert!(
        core.project_view.is_some(),
        "adding a scene reset the stored project view"
    );
    assert!(
        core.stored_project_view().is_none(),
        "the shared view must expire on a content revision, as any edit does"
    );

    // Undo returns to the pre-import document, redo works without the input.
    let reference = core.projects.reference().unwrap();
    core.handle(request(Method::ProjectUndo, json!({"project":reference})));
    let sessions = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(sessions.document().scenes.len(), 1);
    let reference = core.projects.reference().unwrap();
    std::fs::remove_file(&source).unwrap();
    core.handle(request(Method::ProjectRedo, json!({"project":reference})));
    let document = &core.projects.active.as_ref().unwrap().session.document();
    assert_eq!(document.scenes.len(), 2);
    assert!(
        document.scenes.contains_key(&identity[0]),
        "redo must reuse the batch's saved ids, not regenerate them"
    );
    let _ = before;
}

#[test]
fn imports_publish_into_sceneless_projects_without_opening_the_scene() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, true);
    let source = scene_source("lonely.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let sessions = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(sessions.document().scenes.len(), 1);
    assert!(sessions.active_scene().is_none());
    assert!(sessions.document().default_scene.is_none());
    assert_eq!(
        core.job(job).unwrap().state,
        JobState::Succeeded,
        "a project without scenes accepts imports"
    );
}

#[test]
fn image_imports_create_only_the_image_asset_in_the_requested_folder() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, true);
    let source = image_source("wood.png");
    let staging = tempfile::tempdir().unwrap();
    let job = start_import(
        &mut core,
        Method::ProjectImportResource,
        &source,
        Some("Images"),
    );
    let batch = crate::import_prepare::resource_in(&source, staging.path(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let result = core.job(job).unwrap().result.as_ref().unwrap().clone();
    let created = result["created"].as_array().unwrap();
    assert_eq!(created.len(), 1, "{result}");
    assert_eq!(created[0]["kind"], "asset");
    assert_eq!(created[0]["location"], "Images/wood.png");
    let document = &core.projects.active.as_ref().unwrap().session.document();
    assert!(document.resources.is_empty(), "no texture is created");
    assert_eq!(document.assets.len(), 1);
}

/// Folders have no fixed role, so a single resource import names its
/// destination; omitting it dies at request time without a job.
#[test]
fn resource_imports_without_a_folder_are_rejected_without_a_job() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, true);
    let reference = core.projects.reference().unwrap();
    let source = image_source("wood.png");
    let response = core.handle(request(
        Method::ProjectImportResource,
        json!({"project":reference,"source_path":source.display().to_string()}),
    ));
    assert_eq!(response.error.unwrap().code, ErrorCode::InvalidRequest);
    assert!(
        !core
            .jobs
            .iter()
            .any(|record| record.job.kind == "project.import_resource")
    );
}

/// A mesh imported but not yet saved keeps its bytes in the staged
/// payload; its Inspector statistics must resolve from there, not from
/// the not-yet-written file on disk.
#[test]
fn unsaved_meshes_report_statistics_from_their_staged_payload() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, true);
    let source = ply_source("toy.ply");
    let staging = tempfile::tempdir().unwrap();
    let job = start_import(
        &mut core,
        Method::ProjectImportResource,
        &source,
        Some("Images"),
    );
    let batch = crate::import_prepare::resource_in(&source, staging.path(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let result = core.job(job).unwrap().result.as_ref().unwrap().clone();
    let asset = result["created"][0]["id"].as_str().unwrap().to_owned();
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        Method::ProjectGetAsset,
        json!({"project":reference,"asset_id":asset}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    let mesh = &response.result.unwrap()["mesh"];
    assert_eq!(
        mesh["vertices"], 4,
        "unsaved imports keep mesh statistics readable: {mesh}"
    );
    assert_eq!(mesh["faces"], 4);
}

#[test]
fn an_edit_between_request_and_publication_makes_the_import_stale() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let reference = core.projects.reference().unwrap();
    let node = *core
        .projects
        .active
        .as_ref()
        .unwrap()
        .session
        .document()
        .scenes
        .values()
        .next()
        .unwrap()
        .nodes
        .keys()
        .next()
        .unwrap();
    let source = scene_source("stale.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    // A concurrent edit lands before the worker finishes.
    let applied = core.handle(request(
        Method::ProjectApply,
        json!({"project":reference,"edits":[
            {"kind":"rename_node","node_id":node,"label":"Renamed"}]}),
    ));
    assert!(applied.error.is_none(), "{:?}", applied.error);
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let details = core
        .job(job)
        .unwrap()
        .error
        .clone()
        .unwrap()
        .details
        .unwrap();
    // Machine-readable, wire-shaped reason: the serialized error code name.
    assert_eq!(details["published"], false);
    assert_eq!(details["reason"], "revision_conflict", "{details}");
    let sessions = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(
        sessions.document().scenes.len(),
        1,
        "stale imports add nothing"
    );
    assert!(
        !sessions.can_redo(),
        "stale imports leave no history residue"
    );
}

#[test]
fn cancellation_before_publication_keeps_the_session_untouched() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let source = scene_source("doomed.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    core.handle(request(Method::JobCancel, json!({"job_id":job})));
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let error = core.job(job).unwrap().error.clone().unwrap();
    assert_eq!(error.code, ErrorCode::Cancelled);
    let details = error.details.unwrap();
    assert_eq!(details["published"], false);
    assert_eq!(details["reason"], "cancelled", "{details}");
    let sessions = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(sessions.document().scenes.len(), 1);
    assert!(!sessions.is_dirty());
    assert!(!sessions.can_undo() && !sessions.can_redo());
}

#[test]
fn imports_fail_with_a_null_result_and_stable_reasons() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let source = scene_source("ghost.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    core.project_io_finished(
        job,
        Err(ErrorPayload::new(ErrorCode::IoError, "the source vanished")),
    );
    let record = core.job(job).unwrap();
    assert_eq!(record.state, JobState::Failed);
    assert!(record.result.is_none(), "failed jobs have a null result");
    assert_eq!(
        record.error.clone().unwrap().details.unwrap()["reason"],
        "io_error"
    );
    let sessions = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(sessions.document().scenes.len(), 1);
}

#[test]
fn imports_are_stale_after_save_as_relocates_the_store() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    // The worker finishes after the project moved to its new store: the
    // AddImport pending keeps the store id captured at request time.
    let reference = core.projects.reference().unwrap();
    let source = scene_source("relocated.pbrt");
    let import = start_import(&mut core, Method::ProjectImportScene, &source, None);
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("published");
    let save = {
        let response = core.handle(request(
            Method::ProjectSave,
            json!({"project":reference,"destination":destination.display().to_string()}),
        ));
        assert!(response.error.is_none(), "{:?}", response.error);
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
    };
    let snapshot = core.projects.active.as_ref().unwrap().session.snapshot();
    let mut folder = project::FolderStore::create_v3(
        &destination,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    let receipt = folder.save(&snapshot).unwrap();
    core.project_io_finished(
        save,
        Ok(IoOutput::SavedAs {
            receipt,
            registered: snapshot.document().clone(),
            store_id: Uuid::new_v4(),
            path: destination.display().to_string(),
            relocated: None,
        }),
    );
    assert_eq!(core.job(save).unwrap().state, JobState::Succeeded);

    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, import, batch);
    let error = core.job(import).unwrap().error.clone().unwrap();
    assert_eq!(error.code, ErrorCode::StaleProject);
    let details = error.details.unwrap();
    assert_eq!(details["published"], false);
    assert_eq!(details["reason"], "stale_project", "{details}");
    assert_eq!(
        core.projects
            .active
            .as_ref()
            .unwrap()
            .session
            .document()
            .scenes
            .len(),
        1
    );
}

#[test]
fn a_save_snapshot_taken_before_publication_stays_dirty_about_the_import() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    // Save is accepted, the import publishes, and save completes last with
    // the pre-import snapshot. The import stays dirty; next save persists it.
    let pre_import = core.projects.active.as_ref().unwrap().session.snapshot();
    let source = scene_source("ordered.pbrt");
    let import = start_import(&mut core, Method::ProjectImportScene, &source, None);
    let reference = core.projects.reference().unwrap();
    let save = {
        let response = core.handle(request(Method::ProjectSave, json!({"project":reference})));
        assert!(response.error.is_none(), "{:?}", response.error);
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
    };
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, import, batch);
    assert_eq!(core.job(import).unwrap().state, JobState::Succeeded);
    assert!(
        core.projects
            .active
            .as_ref()
            .unwrap()
            .session
            .document()
            .scenes
            .len()
            == 2
    );
    // The worker's receipt is for the snapshot it received at acceptance.
    let directory = tempfile::tempdir();
    let _ = directory;
    let snapdir = tempfile::tempdir().unwrap();
    let root = snapdir.path().join("saved");
    let mut folder = project::FolderStore::create_v3(
        &root,
        pre_import.document().id,
        &pre_import.document().title,
        pre_import.revision().clone(),
    )
    .unwrap();
    let receipt = folder.save(&pre_import).unwrap();
    core.project_io_finished(
        save,
        Ok(IoOutput::Saved {
            receipt,
            registered: pre_import.document().clone(),
        }),
    );
    assert_eq!(core.job(save).unwrap().state, JobState::Succeeded);
    let active = core.projects.active.as_ref().unwrap();
    assert_eq!(active.session.document().scenes.len(), 2);
    assert!(
        active.session.is_dirty(),
        "the published import was accepted after the save snapshot"
    );
}

/// A publication lands between Save As acceptance and its
/// completion. The worker receipts the acceptance-time snapshot, and the
/// relocated session keeps the import's history and bytes: the import's
/// lease stays in staging, undo/redo still reach the import, the mismatch
/// between the receipt and the current content keeps the project dirty, and
/// the next save writes the bytes into the new folder.
#[test]
fn an_import_published_between_save_as_acceptance_and_completion_moves_with_the_session() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let pre_import = core.projects.active.as_ref().unwrap().session.snapshot();
    let reference = core.projects.reference().unwrap();
    let source = image_source("wood.png");
    let staging = tempfile::tempdir().unwrap();
    let import = start_import(
        &mut core,
        Method::ProjectImportResource,
        &source,
        Some("Images"),
    );
    let directory = tempfile::tempdir().unwrap();
    let destination = directory.path().join("published");
    let save = {
        let response = core.handle(request(
            Method::ProjectSave,
            json!({"project":reference,"destination":destination.display().to_string()}),
        ));
        assert!(response.error.is_none(), "{:?}", response.error);
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
    };
    // Publish inside the Save As window, then finish the save with the
    // acceptance-time snapshot the worker actually received.
    let batch = crate::import_prepare::resource_in(&source, staging.path(), || false).unwrap();
    complete_import(&mut core, import, batch);
    assert_eq!(core.job(import).unwrap().state, JobState::Succeeded);
    let mut store = project::FolderStore::create_v3(
        &destination,
        pre_import.document().id,
        &pre_import.document().title,
        pre_import.revision().clone(),
    )
    .unwrap();
    let receipt = store.save(&pre_import).unwrap();
    core.project_io_finished(
        save,
        Ok(IoOutput::SavedAs {
            receipt,
            registered: pre_import.document().clone(),
            store_id: Uuid::new_v4(),
            path: destination.display().to_string(),
            relocated: None,
        }),
    );
    let finished = core.job(save).unwrap();
    assert_eq!(finished.state, JobState::Succeeded);
    assert_eq!(
        finished.result.as_ref().unwrap()["relocated"],
        true,
        "{:?}",
        finished.result
    );

    let active = core.projects.active.as_ref().unwrap();
    assert_eq!(
        active.asset_root.as_deref(),
        Some(destination.join("Assets").to_str().unwrap())
    );
    assert_eq!(active.session.document().assets.len(), 1);
    assert!(
        active.session.is_dirty(),
        "the receipt predates the publication, so the import is unsaved"
    );
    // The relocated session still holds the published bytes: they stay in
    // staging until a save writes them into the new folder.
    let imported: Vec<_> = active
        .session
        .document()
        .assets
        .keys()
        .copied()
        .filter(|id| !pre_import.document().assets.contains_key(id))
        .collect();
    assert_eq!(imported.len(), 1);
    let snapshot = active.session.snapshot();
    let lease = snapshot
        .payload_by(imported[0])
        .expect("the import's lease survives the relocation");
    assert!(std::fs::metadata(lease.path()).is_ok());

    // The history entry moved too: undo drops the import, redo restores it.
    let reference = core.projects.reference().unwrap();
    let undo = core.handle(request(Method::ProjectUndo, json!({"project":reference})));
    assert!(undo.error.is_none(), "{:?}", undo.error);
    assert_eq!(
        core.projects
            .active
            .as_ref()
            .unwrap()
            .session
            .document()
            .assets
            .len(),
        0
    );
    let reference = core.projects.reference().unwrap();
    let redo = core.handle(request(Method::ProjectRedo, json!({"project":reference})));
    assert!(redo.error.is_none(), "{:?}", redo.error);
    let active = core.projects.active.as_ref().unwrap();
    assert_eq!(active.session.document().assets.len(), 1);
    assert!(active.session.snapshot().payload_by(imported[0]).is_some());

    // The next (in-place) save writes the import into the new folder.
    let location = active.session.document().assets[&imported[0]]
        .location
        .clone();
    assert!(!destination.join("Assets").join(&location).exists());
    let current = active.session.snapshot();
    let reference = core.projects.reference().unwrap();
    let save = {
        let response = core.handle(request(Method::ProjectSave, json!({"project":reference})));
        assert!(response.error.is_none(), "{:?}", response.error);
        serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
    };
    let receipt = store.save(&current).unwrap();
    core.project_io_finished(
        save,
        Ok(IoOutput::Saved {
            receipt,
            registered: current.document().clone(),
        }),
    );
    assert_eq!(core.job(save).unwrap().state, JobState::Succeeded);
    assert!(!core.projects.active.as_ref().unwrap().session.is_dirty());
    assert!(destination.join("Assets").join(&location).is_file());
}

/// Opening or closing a scene while an import job runs still publishes
/// anyway — activation moves there, but the document revision does not, and
/// the publication keeps whatever scene the user switched to.
#[test]
fn scene_switches_during_an_import_still_publish() {
    let source = scene_source("switched.pbrt");
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    // The user closes the active scene while the worker is busy.
    let closed = core.handle(request(
        Method::ProjectCloseScene,
        json!({"project":core.projects.reference().unwrap()}),
    ));
    assert!(closed.error.is_none(), "{:?}", closed.error);
    assert!(
        core.projects
            .active
            .as_ref()
            .unwrap()
            .session
            .active_scene()
            .is_none()
    );
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let record = core.job(job).unwrap();
    assert_eq!(
        record.state,
        JobState::Succeeded,
        "scene switches during an import must not stale: {:?}",
        record.error
    );
    let active = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(active.document().scenes.len(), 2);
    assert_eq!(
        active.active_scene(),
        None,
        "the user's closed scene stays closed"
    );

    // And reopening a scene during a second import publishes too, keeping
    // the scene the user opened.
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    let scene = *core
        .projects
        .active
        .as_ref()
        .unwrap()
        .session
        .document()
        .scenes
        .keys()
        .next()
        .unwrap();
    let opened = core.handle(request(
        Method::ProjectOpenScene,
        json!({"project":core.projects.reference().unwrap(),
               "scene":core.projects.active.as_ref().unwrap().session.document().scenes[&scene].id.0}),
    ));
    assert!(opened.error.is_none(), "{:?}", opened.error);
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let record = core.job(job).unwrap();
    assert_eq!(
        record.state,
        JobState::Succeeded,
        "scene switches during an import must not stale: {:?}",
        record.error
    );
    let active = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(active.document().scenes.len(), 3);
    assert_eq!(active.active_scene(), Some(scene));
}

/// Closing the project and reopening it while an import is in
/// flight must stale the publication - the store the job captured is gone.
#[test]
fn closing_and_reopening_the_project_during_an_import_stales() {
    let mut core = CoreState::new(Limits::default());
    open_editable(&mut core, false);
    let source = scene_source("gone.pbrt");
    let job = start_import(&mut core, Method::ProjectImportScene, &source, None);
    // Close the project entirely, then reopen the same document under a
    // fresh store, as the real worker would.
    core.handle(request(
        Method::ProjectClose,
        json!({"expected_current":core.projects.context()}),
    ));
    open_editable(&mut core, false);
    let (batch, _) = pbrt_io::prepare_scene(&source, &Default::default(), || false).unwrap();
    complete_import(&mut core, job, batch);
    let record = core.job(job).unwrap();
    assert_eq!(record.state, JobState::Failed, "pending import must stale");
    let error = record.error.clone().unwrap();
    assert_eq!(error.code, ErrorCode::StaleProject);
    let details = error.details.unwrap();
    assert_eq!(details["published"], false);
    assert_eq!(details["reason"], "stale_project", "{details}");
    assert_eq!(
        core.projects
            .active
            .as_ref()
            .unwrap()
            .session
            .document()
            .scenes
            .len(),
        1,
        "the stale batch added nothing"
    );
}

#[test]
fn imports_reject_read_only_viewers_and_missing_folders_without_a_job() {
    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    let snapshot = project::ProjectSession::new(project::ProjectDocument::new("Viewer"))
        .unwrap()
        .snapshot();
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: None,
            source_path: Some("/scene.pbrt".into()),
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Viewer,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let reference = core.projects.reference().unwrap();
    let rejected = core.handle(request(
        Method::ProjectImportScene,
        json!({"project":reference,"source_path":"/scene.pbrt"}),
    ));
    assert_eq!(rejected.error.unwrap().code, ErrorCode::ReadOnly);

    core.handle(request(
        Method::ProjectClose,
        json!({"expected_current":core.projects.context()}),
    ));
    open_editable(&mut core, false);
    let reference = core.projects.reference().unwrap();
    let source = scene_source("foldered.pbrt");
    let missing = core.handle(request(
        Method::ProjectImportScene,
        json!({"project":reference,"source_path":source.display().to_string(),
               "folder":"No Such Folder"}),
    ));
    assert_eq!(missing.error.unwrap().code, ErrorCode::InvalidRequest);
    // Folder requests are case-strict: "scenes" is not the "Scenes" folder,
    // and the variant dies the same way as a missing folder.
    let variant = core.handle(request(
        Method::ProjectImportScene,
        json!({"project":reference,"source_path":source.display().to_string(),
               "folder":"scenes"}),
    ));
    assert_eq!(variant.error.unwrap().code, ErrorCode::InvalidRequest);
    assert!(
        !core
            .jobs
            .iter()
            .any(|record| record.job.kind == "project.import_scene"),
        "a rejected folder must not leave a job behind"
    );
}

fn pending_open(core: &mut CoreState) -> JobId {
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
}
