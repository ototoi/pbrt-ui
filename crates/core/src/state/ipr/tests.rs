use super::*;
use crate::services::{Work, WorkResult};
use crate::state::project_commands::IoOutput;
use project::{EntityKind, Node, NodeKind, ProjectDocument, ProjectSession, ProjectSnapshot};

fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: Uuid::new_v4().to_string(),
        method,
        params,
    }
}

fn document() -> (ProjectSnapshot, project::NodeId) {
    let mut document = ProjectDocument::new("Native");
    let shape = Node::new(
        "Sphere",
        NodeKind::Entity {
            entity: EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let id = shape.id;
    let camera = Node::new(
        "Camera",
        NodeKind::Entity {
            entity: EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    scene.options.push(camera.id);
    scene.render_camera = Some(camera.id);
    let world = scene.world;
    let NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data else {
        unreachable!()
    };
    children.push(id);
    scene.nodes.insert(id, shape);
    scene.nodes.insert(camera.id, camera);
    (ProjectSession::new(document).unwrap().snapshot(), id)
}

fn configured_core() -> CoreState {
    let mut core = CoreState::new(Limits::default());
    core.configure_services(crate::services::ServiceConfig {
        gui: false,
        capture: None,
        renderer: Some("/unused-renderer".into()),
        directory: Arc::new(tempfile::tempdir().unwrap()),
        geometry_cache: Default::default(),
        capture_daemon: Default::default(),
        project_directories: Default::default(),
    });
    core
}

fn open_project(core: &mut CoreState) -> (protocol::project::ProjectRef, project::NodeId) {
    let context = core
        .handle(request(Method::ProjectGetState, json!({})))
        .result
        .unwrap()["context"]
        .clone();
    let (snapshot, shape) = document();
    let project = open_snapshot(core, context, snapshot);
    (project, shape)
}

fn open_snapshot(
    core: &mut CoreState,
    context: Value,
    snapshot: ProjectSnapshot,
) -> protocol::project::ProjectRef {
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context,"discard_unsaved":true}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    let id = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.project_io_finished(
        id,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: protocol::project::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    core.projects.reference().unwrap()
}

fn settings(debounce_ms: u64) -> Value {
    json!({"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":debounce_ms})
}

fn ipr_set(core: &mut CoreState, enabled: bool, debounce_ms: u64) -> Value {
    let response = core.handle(request(
        Method::RenderIprSet,
        json!({"enabled":enabled,"settings":settings(debounce_ms)}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn ipr_jobs(core: &CoreState) -> Vec<JobId> {
    core.jobs
        .iter()
        .filter(|record| record.job.kind == IPR_JOB_KIND)
        .map(|record| record.job.job_id)
        .collect()
}

fn tick(core: &mut CoreState, after_ms: u64) {
    core.tick_ipr(Instant::now() + Duration::from_millis(after_ms));
}

fn finish_export(core: &mut CoreState, id: JobId) {
    core.project_io_finished(
        id,
        Ok(IoOutput::Exported {
            path: "/published/scene.pbrt".into(),
            warnings: vec![],
        }),
    );
}

#[test]
fn ipr_requires_a_configured_renderer() {
    let mut core = CoreState::new(Limits::default());
    let response = core.handle(request(
        Method::RenderIprSet,
        json!({"enabled":true,"settings":settings(10)}),
    ));
    assert_eq!(response.error.unwrap().code, ErrorCode::Unsupported);
    assert!(!core.capabilities.ipr);
}

#[test]
fn settings_outside_limits_are_rejected() {
    let mut core = configured_core();
    for settings in [
        json!({"samples_per_pixel":0,"threads":1,"max_depth":null,"debounce_ms":400}),
        json!({"samples_per_pixel":257,"threads":1,"max_depth":null,"debounce_ms":400}),
        json!({"samples_per_pixel":8,"threads":0,"max_depth":null,"debounce_ms":400}),
        json!({"samples_per_pixel":8,"threads":1,"max_depth":0,"debounce_ms":400}),
        json!({"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":9}),
        json!({"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":5001}),
    ] {
        let response = core.handle(request(
            Method::RenderIprSet,
            json!({"enabled":true,"settings":settings}),
        ));
        assert!(response.error.is_some(), "{settings}");
    }
    assert!(!core.ipr.enabled);
}

#[test]
fn enabling_ipr_arms_the_project_and_debounce_delays_one_launch() {
    let mut core = configured_core();
    let (project, _) = open_project(&mut core);
    let status = ipr_set(&mut core, true, 10);
    assert_eq!(status["enabled"], true);
    assert_eq!(status["settings"]["samples_per_pixel"], 8);
    assert_eq!(status["active_job_id"], Value::Null);
    tick(&mut core, 0);
    assert!(
        ipr_jobs(&core).is_empty(),
        "launch before the debounce window"
    );
    tick(&mut core, 50);
    let jobs = ipr_jobs(&core);
    assert_eq!(jobs.len(), 1, "one coalesced launch");
    let job = core.job(jobs[0]).unwrap();
    assert_eq!(job.project.as_ref(), Some(&project));
    assert_eq!(job.state, JobState::Queued);
    assert_eq!(core.take_services().count(), 0, "export stage first");
    finish_export(&mut core, jobs[0]);
    let tasks: Vec<_> = core.take_services().collect();
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks[0].work,
        Work::ProjectRender { ipr: true, .. }
    ));
    tick(&mut core, 100);
    assert_eq!(ipr_jobs(&core).len(), 1, "steady state launches nothing");
}

#[test]
fn ipr_scene_override_is_project_scoped_and_does_not_inherit_another_scenes_view() {
    let mut core = configured_core();
    let (project, _) = open_project(&mut core);
    ipr_set(&mut core, true, 10);
    let created = core.handle(request(
        Method::ProjectApply,
        json!({
            "project":core.projects.reference().unwrap(),
            "edits":[{"kind":"create_scene","location":"Second.scene"}]
        }),
    ));
    assert!(created.error.is_none(), "{:?}", created.error);
    let second_scene = created.result.unwrap()["created"][0]["id"]
        .as_str()
        .unwrap()
        .parse::<Uuid>()
        .unwrap();
    let context = core
        .handle(request(Method::ProjectGetState, json!({})))
        .result
        .unwrap()["context"]
        .clone();
    let view = protocol::view::ViewState {
        camera: crate::view::frame(None, [32, 24]),
        size: [32, 24],
        mode: protocol::view::DisplayMode::Solid,
        time: 0.0,
    };
    core.project_view = Some((serde_json::from_value(context.clone()).unwrap(), view, 1));
    let current_project = core.projects.reference().unwrap();
    let submitted = core.handle(request(
        Method::RenderIprSubmit,
        json!({"project":current_project,"scene":second_scene}),
    ));
    assert!(submitted.error.is_none(), "{:?}", submitted.error);
    assert!(core.ipr.params.as_ref().unwrap().view.is_none());

    let mut next_document = ProjectDocument::new("Next project");
    let initial_scene = next_document.default_scene.unwrap();
    let mut next_scene = next_document
        .scenes
        .remove(&initial_scene)
        .unwrap()
        .as_ref()
        .clone();
    next_scene.id = project::SceneId(second_scene);
    next_document.default_scene = Some(next_scene.id);
    next_document
        .scenes
        .insert(next_scene.id, std::sync::Arc::new(next_scene));
    let next_snapshot = ProjectSession::new(next_document).unwrap().snapshot();
    let context = core
        .handle(request(Method::ProjectGetState, json!({})))
        .result
        .unwrap()["context"]
        .clone();
    let next_project = open_snapshot(&mut core, context, next_snapshot);
    assert_ne!(next_project.project_id, project.project_id);
    assert!(core.ipr.params.as_ref().unwrap().scene.is_none());
    tick(&mut core, 50);
    let jobs = ipr_jobs(&core);
    assert_eq!(
        jobs.len(),
        1,
        "a stale scene override must not block relaunch"
    );
    assert_eq!(core.job(jobs[0]).unwrap().state, JobState::Queued);
}

#[test]
fn edits_inside_the_debounce_window_render_only_the_latest_revision() {
    let mut core = configured_core();
    let (project, shape) = open_project(&mut core);
    ipr_set(&mut core, true, 10);
    for label in ["first", "second"] {
        let response = core.handle(request(
            Method::ProjectApply,
            json!({"project":core.projects.reference().unwrap(),"edits":[{"kind":"rename_node","node_id":shape,"label":label}]}),
        ));
        assert!(response.error.is_none(), "{:?}", response.error);
    }
    tick(&mut core, 50);
    let jobs = ipr_jobs(&core);
    assert_eq!(jobs.len(), 1);
    let latest = core.projects.reference().unwrap();
    assert_ne!(latest.revision, project.revision);
    assert_eq!(core.job(jobs[0]).unwrap().project.as_ref(), Some(&latest));
}

#[test]
fn a_stale_active_ipr_job_is_cancelled_and_the_latest_state_relanches() {
    let mut core = configured_core();
    let (_, shape) = open_project(&mut core);
    ipr_set(&mut core, true, 10);
    tick(&mut core, 50);
    let first = ipr_jobs(&core)[0];
    let response = core.handle(request(
        Method::ProjectApply,
        json!({"project":core.projects.reference().unwrap(),"edits":[{"kind":"rename_node","node_id":shape,"label":"edited"}]}),
    ));
    assert!(response.error.is_none());
    tick(&mut core, 50);
    assert_eq!(core.job(first).unwrap().state, JobState::Cancelling);
    finish_export(&mut core, first);
    assert_eq!(core.job(first).unwrap().state, JobState::Cancelled);
    tick(&mut core, 100);
    let jobs = ipr_jobs(&core);
    assert_eq!(jobs.len(), 2);
    let second = jobs[1];
    assert_ne!(second, first);
    assert_eq!(
        core.job(second).unwrap().project.as_ref(),
        Some(&core.projects.reference().unwrap())
    );
    finish_export(&mut core, second);
    let tasks: Vec<_> = core.take_services().collect();
    assert_eq!(tasks.len(), 1, "the cancelled export never renders");
    assert!(matches!(
        tasks[0].work,
        Work::ProjectRender { ipr: true, .. }
    ));
}

#[test]
fn a_final_render_preempts_ipr_and_the_waiting_launch_runs_once_after_it() {
    let mut core = configured_core();
    let (project, _) = open_project(&mut core);
    ipr_set(&mut core, true, 10);
    tick(&mut core, 50);
    let ipr_job = ipr_jobs(&core)[0];
    let response = core.handle(request(
        Method::ProjectRender,
        json!({"project":project,"expected_settings_revision":"0"}),
    ));
    assert!(response.error.is_none());
    let final_job = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    assert_eq!(core.job(ipr_job).unwrap().state, JobState::Cancelling);
    finish_export(&mut core, ipr_job);
    assert_eq!(core.job(ipr_job).unwrap().state, JobState::Cancelled);
    let status = core
        .handle(request(Method::AppGetState, json!({})))
        .result
        .unwrap()["ipr"]
        .clone();
    assert_eq!(status["waiting_for_final_render"], true);
    tick(&mut core, 50);
    assert_eq!(
        ipr_jobs(&core).len(),
        1,
        "no relaunch while the final render runs"
    );
    finish_export(&mut core, final_job);
    let tasks: Vec<_> = core.take_services().collect();
    assert_eq!(tasks.len(), 1);
    assert!(matches!(
        tasks[0].work,
        Work::ProjectRender { ipr: false, .. }
    ));
    core.service_finished(
        final_job,
        WorkResult {
            result: Ok(json!({})),
            artifacts: vec![],
        },
    );
    tick(&mut core, 100);
    let jobs = ipr_jobs(&core);
    assert_eq!(jobs.len(), 2, "the preempted state relaunches once");
    assert_ne!(jobs[1], ipr_job);
    let status = core
        .handle(request(Method::AppGetState, json!({})))
        .result
        .unwrap()["ipr"]
        .clone();
    assert_eq!(status["waiting_for_final_render"], false);
}

#[test]
fn progressive_frames_replace_the_previous_frame_instead_of_accumulating() {
    let mut core = configured_core();
    open_project(&mut core);
    ipr_set(&mut core, true, 10);
    tick(&mut core, 50);
    let id = ipr_jobs(&core)[0];
    finish_export(&mut core, id);
    core.take_services().for_each(drop);
    let directory = tempfile::tempdir().unwrap();
    let frame = |index: u32| Artifact {
        artifact_id: Uuid::new_v4(),
        path: directory
            .path()
            .join(format!("ipr-frame-{index:05}.png"))
            .to_string_lossy()
            .into_owned(),
        mime: "image/png".into(),
        size: 12 + index as u64,
        sha256: format!("{index:0>64}"),
        metadata: json!({"kind":"ipr_frame","partial":index < 2}),
    };
    std::fs::write(directory.path().join("ipr-frame-00000.png"), "old").unwrap();
    core.service_progress(id, frame(0));
    assert_eq!(core.job(id).unwrap().artifacts.len(), 1);
    std::fs::write(directory.path().join("ipr-frame-00001.png"), "new").unwrap();
    let second = frame(1);
    let second_id = second.artifact_id;
    core.service_progress(id, second);
    assert_eq!(core.job(id).unwrap().artifacts, vec![second_id]);
    assert!(!directory.path().join("ipr-frame-00000.png").exists());
    assert!(directory.path().join("ipr-frame-00001.png").exists());
    core.service_finished(
        id,
        WorkResult {
            result: Ok(json!({"ipr":true})),
            artifacts: vec![],
        },
    );
    assert_eq!(core.ipr.frame, None);
    assert!(!directory.path().join("ipr-frame-00001.png").exists());
    assert!(
        core.job(id).unwrap().artifacts.is_empty(),
        "a finished preview keeps no frame artifacts"
    );
    assert_eq!(
        core.handle(request(Method::AppGetState, json!({})))
            .result
            .unwrap()["ipr"]["active_job_id"],
        Value::Null
    );
}

#[test]
fn disabling_ipr_cancels_every_active_preview_job_and_stays_quiet() {
    let mut core = configured_core();
    open_project(&mut core);
    ipr_set(&mut core, true, 10);
    tick(&mut core, 50);
    let id = ipr_jobs(&core)[0];
    let status = ipr_set(&mut core, false, 10);
    assert_eq!(status["enabled"], false);
    assert_eq!(core.job(id).unwrap().state, JobState::Cancelling);
    assert!(!core.ipr.armed);
    finish_export(&mut core, id);
    tick(&mut core, 100);
    assert_eq!(ipr_jobs(&core).len(), 1);
    assert_eq!(core.take_services().count(), 0);
}

#[test]
fn cancelling_a_preview_job_explicitly_does_not_relaunch() {
    let mut core = configured_core();
    open_project(&mut core);
    ipr_set(&mut core, true, 10);
    tick(&mut core, 50);
    let id = ipr_jobs(&core)[0];
    let response = core.handle(request(Method::JobCancel, json!({"job_id":id})));
    assert!(response.error.is_none());
    assert!(!core.ipr.armed, "the pending relaunch is dropped");
    finish_export(&mut core, id);
    tick(&mut core, 100);
    assert_eq!(ipr_jobs(&core).len(), 1);
}

#[test]
fn submitting_requires_enabled_and_uses_the_current_project() {
    let mut core = configured_core();
    let (project, shape) = open_project(&mut core);
    let response = core.handle(request(Method::RenderIprSubmit, json!({})));
    assert_eq!(response.error.unwrap().code, ErrorCode::Unsupported);
    ipr_set(&mut core, true, 500);
    core.handle(request(
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"rename_node","node_id":shape,"label":"edited"}]}),
    ));
    let stale = core.handle(request(Method::RenderIprSubmit, json!({"project":project})));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);
    let status = core.handle(request(Method::RenderIprSubmit, json!({})));
    assert!(status.error.is_none());
    assert!(core.ipr.armed);
    tick(&mut core, 50);
    assert!(
        ipr_jobs(&core).is_empty(),
        "submit respects the debounce window"
    );
    tick(&mut core, 600);
    let jobs = ipr_jobs(&core);
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        core.job(jobs[0]).unwrap().project.as_ref(),
        Some(&core.projects.reference().unwrap())
    );
}

#[test]
fn enabling_without_a_project_arms_nothing() {
    let mut core = configured_core();
    ipr_set(&mut core, true, 10);
    assert!(!core.ipr.armed, "no project means nothing to render");
    tick(&mut core, 50);
    assert!(ipr_jobs(&core).is_empty());
}
