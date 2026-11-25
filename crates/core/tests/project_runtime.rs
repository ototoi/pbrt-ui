use core::{CoreHandle, CoreHost};
use protocol::commands::Limits;
use protocol::{ErrorCode, Method, Request};
use serde_json::{Value, json};
use std::{
    fs,
    time::{Duration, Instant},
};

#[cfg(unix)]
fn render_host(directory: &std::path::Path) -> CoreHost {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let renderer = directory.join("renderer.py");
    fs::write(
        &renderer,
        r#"#!/usr/bin/python3
from pathlib import Path
import sys, time
program = Path(__file__)
print("argv:", " ".join(sys.argv[1:-1]), flush=True)
print(Path(sys.argv[-1]).read_text(), flush=True)
program.with_suffix('.started').touch()
deadline = time.monotonic() + 10
while not program.with_suffix('.release').exists():
    if time.monotonic() > deadline:
        raise RuntimeError('test did not release renderer')
    time.sleep(0.01)
Path(sys.argv[sys.argv.index('--outfile') + 1]).write_bytes(bytes([0x76,0x2f,0x31,0x01]))
"#,
    )
    .unwrap();
    fs::set_permissions(&renderer, fs::Permissions::from_mode(0o755)).unwrap();
    CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: None,
            renderer: Some(renderer),
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    )
}

#[cfg(unix)]
fn wait_renderer(directory: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !directory.join("renderer.started").exists() {
        assert!(Instant::now() < deadline, "renderer did not start");
        std::thread::sleep(Duration::from_millis(5));
    }
}

/// The renderer's log of one finished render job: its argv line and the
/// scene it read.
#[cfg(unix)]
fn renderer_log(core: &CoreHandle, job: &Value) -> String {
    for id in job["artifacts"].as_array().unwrap() {
        let artifact = call(core, Method::ArtifactGet, json!({"artifact_id":id}));
        if artifact["metadata"]["kind"] == "renderer_log" {
            return fs::read_to_string(artifact["path"].as_str().unwrap()).unwrap();
        }
    }
    panic!("no renderer log: {job}")
}

/// project.render uses the scene's own camera and Film unless a view is
/// asked for (an explicit one, or camera "view" for the shared one); the
/// settings leave samples to the scene's Sampler and threads to the
/// renderer while unset, and threads start unset.
#[cfg(unix)]
#[test]
fn project_render_uses_the_scene_camera_unless_a_view_is_asked_for() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    let source = temp.path().join("source.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\n\
         Film \"rgb\" \"integer xresolution\" [64] \"integer yresolution\" [48]\n\
         WorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = || state(&core)["context"]["project"].clone();
    let render = |extra: Value| {
        let mut params = json!({"project":project(),"expected_settings_revision":
            call(&core, Method::RenderGetSettings, json!({}))["settings_revision"]});
        params
            .as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let job = wait(&core, call(&core, Method::ProjectRender, params));
        assert_eq!(job["state"], "succeeded", "{job}");
        renderer_log(&core, &job)
    };
    let settings = call(&core, Method::RenderGetSettings, json!({}));
    assert_eq!(settings["settings"]["threads"], Value::Null, "{settings}");

    // No camera named: the scene's camera and Film, all cores.
    let scene = render(json!({}));
    let argv = scene.lines().next().unwrap();
    assert!(
        !argv.contains("--spp") && !argv.contains("--nthreads"),
        "{argv}"
    );
    assert!(
        scene.contains("\"integer xresolution\" [ 64 ] \"integer yresolution\" [ 48 ]"),
        "{scene}"
    );
    assert!(!scene.contains("view-scene"), "{scene}");
    let body = |log: &str| log.split_once('\n').unwrap().1.to_owned();
    assert_eq!(body(&render(json!({"camera":"scene"}))), body(&scene));

    // An explicit view renders that view.
    let view = protocol::view::ViewState {
        camera: core::view::frame(None, [32, 24]),
        size: [32, 24],
        mode: protocol::view::DisplayMode::Solid,
        time: 0.0,
    };
    let viewed = render(json!({"view":view}));
    assert!(
        viewed.contains("\"integer xresolution\" [32] \"integer yresolution\" [24]"),
        "{viewed}"
    );
    let refused = core.request(request(
        Method::ProjectRender,
        json!({"project":project(),"expected_settings_revision":"0","camera":"scene","view":view}),
    ));
    assert_eq!(refused.error.unwrap().code, ErrorCode::InvalidRequest);

    // camera "view" needs a shared view: refused with a reason until one is
    // set, then that view renders.
    let refused = core.request(request(
        Method::ProjectRender,
        json!({"project":project(),"expected_settings_revision":"0","camera":"view"}),
    ));
    let error = refused.error.expect("no shared view yet");
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert_eq!(error.details.unwrap()["reason"], "no_shared_view");
    call(
        &core,
        Method::ProjectSetCamera,
        json!({"project":project(),"expected_view_revision":"0","camera":view.camera}),
    );
    let shared = render(json!({"camera":"view"}));
    assert!(shared.contains("LookAt"), "{shared}");
    assert!(
        !shared.contains("\"integer xresolution\" [ 64 ]"),
        "the shared view, not the scene's Film: {shared}"
    );

    // Unset samples follow the scene's Sampler.
    call(
        &core,
        Method::RenderSetSettings,
        json!({"expected_settings_revision":"0",
               "settings":{"samples_per_pixel":null,"threads":4,"max_depth":null}}),
    );
    let argv = render(json!({}));
    let argv = argv.lines().next().unwrap();
    assert!(
        !argv.contains("--spp") && argv.contains("--nthreads 4"),
        "{argv}"
    );
}

#[cfg(unix)]
#[test]
fn project_render_pins_unsaved_snapshot_and_settings_survives_close_and_exports_without_viewer() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nTranslate 1 0 0\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    let roots = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference}),
    );
    let world = roots["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["label"] == "World")
        .unwrap()["node_id"]
        .clone();
    let children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":world}),
    );
    // World-direct transforms wrap into their own group at import,
    // so a state node may sit one level down.
    let state_in = |children: &Value| {
        children["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "state")
            .map(|node| node["node_id"].clone())
    };
    let transform = match state_in(&children) {
        Some(id) => id,
        None => {
            let nested = call(
                &core,
                Method::ProjectListNodes,
                json!({"project":reference,"parent":children["nodes"][0]["node_id"]}),
            );
            state_in(&nested).expect("a transform state")
        }
    };
    let apply = |x| {
        call(
            &core,
            Method::ProjectApply,
            json!({"project":state(&core)["context"]["project"],"edits":[{"kind":"set_transform","node_id":transform,"value":{"kind":"translate","value":[x,0,0]}}]}),
        )
    };
    apply(2);
    let pinned = state(&core)["context"]["project"].clone();
    let render = call(
        &core,
        Method::ProjectRender,
        json!({"project":pinned,"expected_settings_revision":"0"}),
    );
    apply(9);
    call(
        &core,
        Method::RenderSetSettings,
        json!({"expected_settings_revision":"0","settings":{"samples_per_pixel":null,"threads":1,"max_depth":null}}),
    );
    let busy = core.request(request(
        Method::ProjectRender,
        json!({"project":state(&core)["context"]["project"],"expected_settings_revision":"1"}),
    ));
    assert_eq!(busy.error.unwrap().code, ErrorCode::Busy);
    wait_renderer(temp.path());
    fs::remove_file(source).unwrap();
    call(
        &core,
        Method::ProjectClose,
        json!({"expected_current":state(&core)["context"],"discard_unsaved":true}),
    );
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    let job = wait(&core, render);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["project"], pinned);
    assert_eq!(job["scene_id"], Value::Null);
    let mut exr = Value::Null;
    for id in job["artifacts"].as_array().unwrap() {
        let artifact = call(&core, Method::ArtifactGet, json!({"artifact_id":id}));
        assert_eq!(artifact["metadata"]["project"], pinned);
        if artifact["mime"] == "text/plain" {
            let text = fs::read_to_string(artifact["path"].as_str().unwrap()).unwrap();
            assert!(text.contains("Translate 2 0 0"), "{text}");
            assert!(!text.contains("Translate 9 0 0"));
        }
        if artifact["mime"] == "image/x-exr" {
            exr = artifact;
        }
    }
    assert_eq!(
        exr["metadata"]["settings"]["samples_per_pixel"],
        Value::Null
    );
    assert_eq!(
        exr["metadata"]["export"]["project_revision"],
        pinned["revision"]
    );
    let output = temp.path().join("exported.exr");
    let export = call(
        &core,
        Method::ArtifactExport,
        json!({"artifact_id":exr["artifact_id"],"absolute_path":output}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    assert_eq!(fs::read(output).unwrap(), [0x76, 0x2f, 0x31, 0x01]);
}

#[cfg(unix)]
#[test]
fn project_view_render_validates_guards_preserves_tokens_and_cancels_the_process() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "Sampler \"independent\" \"integer seed\" [9007199254740993]\nWorldBegin\nActiveTransform StartTime\nTranslate 1 0 0\nActiveTransform All\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let view = protocol::view::ViewState {
        camera: core::view::frame(None, [32, 24]),
        size: [32, 24],
        mode: protocol::view::DisplayMode::Solid,
        time: 0.5,
    };
    let valid = json!({"project":project,"expected_settings_revision":"0","view":view});
    let mut invalid = valid.clone();
    invalid["expected_settings_revision"] = json!("1");
    assert_eq!(
        core.request(request(Method::ProjectRender, invalid))
            .error
            .unwrap()
            .code,
        ErrorCode::RevisionConflict
    );
    let mut invalid = valid.clone();
    invalid["view"]["size"] = json!([0, 24]);
    assert_eq!(
        core.request(request(Method::ProjectRender, invalid))
            .error
            .unwrap()
            .code,
        ErrorCode::InvalidRequest
    );
    let render = call(&core, Method::ProjectRender, valid);
    wait_renderer(temp.path());
    call(&core, Method::JobCancel, json!({"job_id":render["job_id"]}));
    let job = wait(&core, render);
    assert_eq!(job["state"], "cancelled", "{job}");
    for id in job["artifacts"].as_array().unwrap() {
        let artifact = call(&core, Method::ArtifactGet, json!({"artifact_id":id}));
        assert_ne!(artifact["mime"], "image/x-exr");
        if artifact["mime"] == "text/plain" {
            let text = fs::read_to_string(artifact["path"].as_str().unwrap()).unwrap();
            assert!(text.contains("9007199254740993"), "{text}");
            assert!(text.contains("ActiveTransform StartTime"), "{text}");
            assert!(
                text.contains("\"integer xresolution\" [32] \"integer yresolution\" [24]"),
                "{text}"
            );
            assert!(text.contains("\"float shutteropen\" [0.5]"), "{text}");
        }
    }
}
fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        method,
        params,
    }
}
fn call(core: &CoreHandle, method: Method, params: Value) -> Value {
    let r = core.request(request(method, params));
    assert!(r.error.is_none(), "{:?}", r.error);
    r.result.unwrap()
}
fn state(core: &CoreHandle) -> Value {
    call(core, Method::ProjectGetState, json!({}))
}

#[test]
fn folder_project_capability_is_advertised_after_m4b() {
    let host = CoreHost::start(Limits::default());
    let capabilities = call(&host.handle(), Method::AppGetCapabilities, json!({}));
    assert_eq!(capabilities["project"]["folder_format"], true);
    assert_eq!(capabilities["project"]["viewer_mode"], true);
}

#[test]
fn project_scene_switch_persists_the_default_scene() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));

    let original = state(&core);
    let created = call(
        &core,
        Method::ProjectApply,
        json!({
            "project":original["context"]["project"],
            "edits":[{"kind":"create_scene","location":"Second.scene"}]
        }),
    );
    let second_scene = created["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "scene")
        .unwrap()["id"]
        .clone();
    let save = call(
        &core,
        Method::ProjectSave,
        json!({"project":state(&core)["context"]["project"]}),
    );
    assert_eq!(wait(&core, save)["state"], "succeeded");

    let listed = call(
        &core,
        Method::ProjectListScenes,
        json!({"project":state(&core)["context"]["project"]}),
    );
    assert_eq!(listed["scenes"].as_array().unwrap().len(), 2);
    assert!(
        listed["scenes"].as_array().unwrap().iter().any(|scene| {
            scene["id"] == second_scene && scene["default"] == false && scene["active"] == false
        }),
        "scenes={:?}, second={second_scene}",
        listed["scenes"]
    );

    let switched = call(
        &core,
        Method::ProjectOpenScene,
        json!({"project":state(&core)["context"]["project"],"scene":second_scene}),
    );
    assert_eq!(switched["active_scene"], second_scene);
    assert_eq!(switched["default_scene"], second_scene);
    assert_eq!(switched["dirty"], false);
    assert_eq!(
        switched["context"]["project"]["revision"],
        created["project"]["revision"]
    );

    call(
        &core,
        Method::ProjectClose,
        json!({"expected_current":state(&core)["context"],"discard_unsaved":false}),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let reopened = call(
            &core,
            Method::ProjectOpen,
            json!({"path":temp.path().join("project"),"expected_current":state(&core)["context"]}),
        );
        let result = wait(&core, reopened);
        if result["state"] == "succeeded" {
            break;
        }
        assert!(
            result["error"]["code"] == "project_locked" && Instant::now() < deadline,
            "{result}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(state(&core)["active_scene"], second_scene);
    assert_eq!(state(&core)["default_scene"], second_scene);
}

fn wait(core: &CoreHandle, job: Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let value = call(core, Method::JobGet, json!({"job_id":job["job_id"]}));
        if ["succeeded", "failed", "cancelled"].contains(&value["state"].as_str().unwrap()) {
            return value;
        }
        assert!(Instant::now() < deadline, "job did not terminate: {value}");
        std::thread::sleep(Duration::from_millis(2));
    }
}
fn import(core: &CoreHandle, path: &std::path::Path, destination: &std::path::Path) -> Value {
    let job = call(
        core,
        Method::ProjectImport,
        json!({"source_path":path,"destination":destination,"expected_current":state(core)["context"]}),
    );
    let result = wait(core, job);
    assert_eq!(result["state"], "succeeded", "{result}");
    assert!(result["result"]["report"]["diagnostics"].is_array());
    result
}

fn import_viewer(core: &CoreHandle, path: &std::path::Path) -> Value {
    let job = call(
        core,
        Method::ProjectImport,
        json!({"source_path":path,"expected_current":state(core)["context"]}),
    );
    let result = wait(core, job);
    assert_eq!(result["state"], "succeeded", "{result}");
    assert_eq!(result["result"]["path"], Value::Null);
    assert_eq!(result["result"]["mode"], "viewer");
    result
}

#[test]
fn viewer_import_is_read_only_but_supports_queries_preview_and_export() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("viewer.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();

    import_viewer(&core, &source);
    let before = state(&core);
    let reference = before["context"]["project"].clone();
    assert_eq!(before["mode"], "viewer");
    assert_eq!(before["read_only"], true);
    assert_eq!(before["source_path"], source.to_str().unwrap());
    assert_eq!(before["path"], Value::Null);
    assert_eq!(before["dirty"], false);

    let nodes = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference}),
    );
    assert!(!nodes["nodes"].as_array().unwrap().is_empty());
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference}),
    );
    assert!(contents["entries"].is_array());
    let preview = call(
        &core,
        Method::ProjectPreview,
        json!({"project":reference,"destination":temp.path().join("preview")}),
    );
    assert_eq!(wait(&core, preview)["state"], "succeeded");
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference,"destination":temp.path().join("bundle")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");

    let node = nodes["nodes"][0]["node_id"].clone();
    for (method, params) in [
        (
            Method::ProjectApply,
            json!({"project":reference,"edits":[{"kind":"rename_node","node_id":node,"label":"Changed"}]}),
        ),
        (Method::ProjectUndo, json!({"project":reference})),
        (Method::ProjectRedo, json!({"project":reference})),
    ] {
        let error = core.request(request(method, params)).error.unwrap();
        assert_eq!(error.code, ErrorCode::ReadOnly);
        assert_eq!(
            error.details.unwrap(),
            json!({"mode":"viewer","hint":"project.save with destination"})
        );
    }
    let save = core.request(request(Method::ProjectSave, json!({"project":reference})));
    assert_eq!(save.error.unwrap().code, ErrorCode::InvalidRequest);
    assert_eq!(
        state(&core)["context"]["project"]["revision"],
        reference["revision"]
    );
    assert_eq!(state(&core)["dirty"], false);
}

/// An asset file rewritten outside the application (after Save As, when
/// I/O jobs pin each asset's bytes first) is refused as a changed source
/// naming the file, not as an internal copy error.
#[test]
fn an_asset_rewritten_outside_reports_a_changed_source() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("checker.png"), b"not decoded by import").unwrap();
    let source = temp.path().join("viewer.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTexture \"checker\" \"spectrum\" \"imagemap\" \"string filename\" \"checker.png\"\n\
         Material \"diffuse\" \"texture reflectance\" \"checker\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let destination = temp.path().join("project");
    fs::create_dir(&destination).unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import_viewer(&core, &source);
    let save_as = call(
        &core,
        Method::ProjectSave,
        json!({"project":state(&core)["context"]["project"],"destination":destination}),
    );
    assert_eq!(wait(&core, save_as)["state"], "succeeded");
    fs::write(
        destination.join("Assets/Images/checker.png"),
        b"not decoded by import, but longer now",
    )
    .unwrap();
    // The bytes are pinned when the export is asked for.
    let error = core
        .request(request(
            Method::ProjectExport,
            json!({"project":state(&core)["context"]["project"],"destination":temp.path().join("bundle")}),
        ))
        .error
        .expect("refused");
    assert_eq!(error.code, ErrorCode::SourceChanged, "{error:?}");
    assert_eq!(
        error.details.unwrap()["files"],
        json!(["Assets/Images/checker.png"])
    );
}

#[test]
fn viewer_save_as_switches_to_project_and_preserves_reference_and_scene_ids() {
    let temp = tempfile::tempdir().unwrap();
    // Core reports canonical paths; macOS temp dirs sit behind the /var symlink.
    let root = protocol::path::canonicalize(temp.path()).unwrap();
    let source = root.join("viewer.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let destination = root.join("project");
    fs::create_dir(&destination).unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import_viewer(&core, &source);

    let before = state(&core);
    let reference = before["context"]["project"].clone();
    let default_scene = before["default_scene"].clone();
    let save_as = call(
        &core,
        Method::ProjectSave,
        json!({"project":reference,"destination":destination}),
    );
    let saved = wait(&core, save_as);
    assert_eq!(saved["state"], "succeeded", "{saved}");
    assert_eq!(saved["result"]["mode"], "project");
    assert_eq!(saved["result"]["relocated"], true);
    let project_state = state(&core);
    assert_eq!(project_state["mode"], "project");
    assert_eq!(project_state["read_only"], false);
    assert_eq!(project_state["path"], destination.to_str().unwrap());
    assert_eq!(project_state["dirty"], false);
    assert_eq!(project_state["default_scene"], default_scene);
    assert_eq!(
        project_state["context"]["project"]["project_id"],
        reference["project_id"]
    );
    assert_eq!(
        project_state["context"]["project"]["revision"],
        reference["revision"]
    );
    assert_eq!(
        project_state["context"]["project"]["activation"],
        reference["activation"]
    );

    let nodes = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":project_state["context"]["project"]}),
    );
    let node = nodes["nodes"][0]["node_id"].clone();
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project_state["context"]["project"],"edits":[{"kind":"rename_node","node_id":node,"label":"Editable"}]}),
    );
    let in_place = call(
        &core,
        Method::ProjectSave,
        json!({"project":state(&core)["context"]["project"]}),
    );
    assert_eq!(wait(&core, in_place)["state"], "succeeded");

    let copy = root.join("project-copy");
    let save_as = call(
        &core,
        Method::ProjectSave,
        json!({"project":state(&core)["context"]["project"],"destination":copy}),
    );
    let saved_copy = wait(&core, save_as);
    assert_eq!(saved_copy["state"], "succeeded", "{saved_copy}");
    assert_eq!(saved_copy["result"]["relocated"], true);
    assert_eq!(state(&core)["path"], copy.to_str().unwrap());
    assert_eq!(
        state(&core)["context"]["project"]["activation"],
        reference["activation"]
    );

    let other_host = CoreHost::start(Limits::default());
    let other = other_host.handle();
    let open_old = call(
        &other,
        Method::ProjectOpen,
        json!({"path":destination,"expected_current":state(&other)["context"]}),
    );
    assert_eq!(wait(&other, open_old)["state"], "succeeded");
    assert_eq!(state(&other)["path"], destination.to_str().unwrap());
}

#[test]
fn viewer_save_rejects_invalid_destinations_and_accepts_only_a_folder() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("viewer.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import_viewer(&core, &source);
    let reference = state(&core)["context"]["project"].clone();
    let file = temp.path().join("file");
    fs::write(&file, "not a folder").unwrap();
    let nonempty = temp.path().join("nonempty");
    fs::create_dir(&nonempty).unwrap();
    fs::write(nonempty.join("child"), "x").unwrap();
    for destination in [
        "relative".to_owned(),
        temp.path()
            .join("scene.pbrt")
            .to_string_lossy()
            .into_owned(),
        temp.path()
            .join("scene.pbrt.gz")
            .to_string_lossy()
            .into_owned(),
        file.to_string_lossy().into_owned(),
        nonempty.to_string_lossy().into_owned(),
        temp.path()
            .join("missing-parent/project")
            .to_string_lossy()
            .into_owned(),
    ] {
        let result = core.request(request(
            Method::ProjectSave,
            json!({"project":reference,"destination":destination}),
        ));
        assert_eq!(result.error.unwrap().code, ErrorCode::InvalidRequest);
    }
    assert_eq!(state(&core)["mode"], "viewer");
}

#[test]
fn legacy_format_one_opens_as_viewer_and_saves_as_folder() {
    let temp = tempfile::tempdir().unwrap();
    // Core reports canonical paths; macOS temp dirs sit behind the /var symlink.
    let root = protocol::path::canonicalize(temp.path()).unwrap();
    let legacy = root.join("legacy");
    write_minimal_legacy_project(&legacy);
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let open = call(
        &core,
        Method::ProjectOpen,
        json!({"path":legacy,"expected_current":state(&core)["context"]}),
    );
    let open = wait(&core, open);
    assert_eq!(open["state"], "succeeded", "{open}");
    let before = state(&core);
    let reference = before["context"]["project"].clone();
    assert_eq!(before["mode"], "viewer");
    assert_eq!(before["read_only"], true);
    assert_eq!(before["path"], Value::Null);
    assert_eq!(
        before["source_path"],
        legacy.join("project.pbrtui.json").to_str().unwrap()
    );
    assert_eq!(before["dirty"], false);
    let node = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference}),
    )["nodes"][0]["node_id"]
        .clone();
    let denied = core.request(request(
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"rename_node","node_id":node,"label":"No"}]}),
    ));
    assert_eq!(denied.error.unwrap().code, ErrorCode::ReadOnly);

    let destination = root.join("migrated");
    let save = call(
        &core,
        Method::ProjectSave,
        json!({"project":reference,"destination":destination}),
    );
    assert_eq!(wait(&core, save)["state"], "succeeded");
    assert_eq!(state(&core)["mode"], "project");
    assert_eq!(state(&core)["path"], destination.to_str().unwrap());
}

fn write_minimal_legacy_project(path: &std::path::Path) {
    use serde::Serialize;
    use sha2::Digest;
    use std::collections::BTreeMap;

    #[derive(Serialize)]
    struct Document {
        id: project::ProjectId,
        title: String,
        scene: Scene,
        assets: BTreeMap<project::AssetId, Value>,
        imports: BTreeMap<project::ImportId, Value>,
    }
    #[derive(Serialize)]
    struct Scene {
        options: Vec<project::NodeId>,
        world: project::NodeId,
        nodes: BTreeMap<project::NodeId, Node>,
        resources: BTreeMap<project::ResourceId, Value>,
    }
    #[derive(Serialize)]
    struct Node {
        id: project::NodeId,
        label: String,
        edition: project::Revision,
        origin: Option<project::Origin>,
        data: NodeKind,
    }
    #[derive(Serialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum NodeKind {
        Scope {
            scope: project::ScopeKind,
            children: Vec<project::NodeId>,
        },
    }
    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", sha2::Sha256::digest(bytes))
    }
    fn object_ref(bytes: &[u8]) -> Value {
        json!({"sha256":digest(bytes),"size":bytes.len()})
    }

    fs::create_dir_all(path.join("objects")).unwrap();
    let generation_id = uuid::Uuid::new_v4();
    let generation = path.join("generations").join(generation_id.to_string());
    fs::create_dir_all(&generation).unwrap();
    let id = project::ProjectId::new();
    let world = project::NodeId::new();
    let document = Document {
        id,
        title: "Legacy".into(),
        scene: Scene {
            options: vec![],
            world,
            nodes: BTreeMap::from([(
                world,
                Node {
                    id: world,
                    label: "World".into(),
                    edition: project::Revision::initial(),
                    origin: None,
                    data: NodeKind::Scope {
                        scope: project::ScopeKind::World,
                        children: vec![],
                    },
                },
            )]),
            resources: BTreeMap::new(),
        },
        assets: BTreeMap::new(),
        imports: BTreeMap::new(),
    };
    let mut scene = serde_json::to_vec(&json!({
        "record":"scene", "project_id":id, "title":"Legacy", "options":[], "world":world
    }))
    .unwrap();
    scene.push(b'\n');
    scene.extend(
        serde_json::to_vec(&json!({
            "record":"node",
            "node": {
                "id":world,
                "label":"World",
                "edition":"0",
                "origin":null,
                "data":{"kind":"scope","scope":"world","children":[]}
            }
        }))
        .unwrap(),
    );
    scene.push(b'\n');
    let assets = b"{}";
    let imports = b"{}";
    fs::write(generation.join("scene.jsonl"), &scene).unwrap();
    fs::write(generation.join("assets.json"), assets).unwrap();
    fs::write(generation.join("imports.json"), imports).unwrap();
    let manifest = json!({
        "format":"pbrt-ui-project",
        "format_version":1,
        "project_id":id,
        "saved_revision":"0",
        "head":generation_id,
        "model_sha256":digest(&serde_json::to_vec(&document).unwrap()),
        "files":{
            "assets.json":object_ref(assets),
            "imports.json":object_ref(imports),
            "scene.jsonl":object_ref(&scene),
        }
    });
    fs::write(
        path.join("project.pbrtui.json"),
        serde_json::to_vec(&manifest).unwrap(),
    )
    .unwrap();
}

#[cfg(unix)]
#[test]
fn viewer_capture_remains_available_and_does_not_mark_the_document_dirty() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys
spec = json.load(open(sys.argv[2]))
geometry = json.load(open(spec['geometry_path']))
open(sys.argv[3], 'wb').write(json.dumps({'draws': len(geometry['draws'])}).encode())
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let directory = Arc::new(tempfile::tempdir().unwrap());
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture),
            renderer: None,
            directory,
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("viewer.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import_viewer(&core, &source);
    let result = call(
        &core,
        Method::ProjectCapture,
        json!({"project":state(&core)["context"]["project"]}),
    );
    let job = wait(&core, result);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(state(&core)["mode"], "viewer");
    assert_eq!(state(&core)["dirty"], false);
}

#[cfg(unix)]
#[test]
fn viewer_can_be_rendered() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("viewer.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    import_viewer(&core, &source);
    let result = call(
        &core,
        Method::ProjectRender,
        json!({"project":state(&core)["context"]["project"],"expected_settings_revision":"0"}),
    );
    wait_renderer(temp.path());
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    let job = wait(&core, result);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(
        job["project"]["revision"],
        state(&core)["context"]["project"]["revision"]
    );
    assert_eq!(state(&core)["mode"], "viewer");
    assert_eq!(state(&core)["dirty"], false);
}

/// connect_texture with an image: the row's lineage picks an imagemap of
/// that image, reused when one exists (whatever its other settings) and
/// created under Textures/ named after the image otherwise. A spectrum
/// imagemap of the same image is not reused for a float row, and an asset
/// that is not an image is refused.
#[test]
fn connecting_an_image_reuses_or_creates_an_imagemap_of_the_row_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    fs::write(temp.path().join("checker.png"), b"not decoded by import").unwrap();
    fs::write(
        temp.path().join("mesh.ply"),
        "ply\nformat ascii 1.0\nelement vertex 3\nproperty float x\nproperty float y\n\
         property float z\nelement face 1\nproperty list uchar int vertex_indices\nend_header\n\
         0 0 0\n1 0 0\n0 1 0\n3 0 1 2\n",
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTexture \"rgbcheck\" \"spectrum\" \"imagemap\" \"string filename\" \"checker.png\"\n\
         Material \"diffuse\"\nShape \"sphere\"\nShape \"plymesh\" \"string filename\" \"mesh.ply\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = || state(&core)["context"]["project"].clone();
    let entries = all_contents(&core, &project());
    let asset_named = |name: &str| {
        entries
            .iter()
            .find(|entry| entry["kind"] == "asset" && entry["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let (image, mesh) = (asset_named("checker.png"), asset_named("mesh.ply"));
    let rgbcheck = entries
        .iter()
        .find(|entry| entry["name"] == "rgbcheck")
        .unwrap()["id"]
        .clone();
    let material = entries
        .iter()
        .find(|entry| entry["resource_kind"] == "material")
        .unwrap()["id"]
        .clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project(),"query":"Shape"}),
    );
    let sphere = hits["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["implementation"] == "sphere" || node["label"] == "Shape")
        .unwrap()["node_id"]
        .clone();
    let shape = json!({"kind":"node","id":sphere});
    let apply = |edit: Value| {
        core.request(request(
            Method::ProjectApply,
            json!({"project":project(),"edits":[edit]}),
        ))
    };
    let connected = |target: &Value, parameter: &str| {
        call(
            &core,
            Method::ProjectGetParameters,
            json!({"project":project(),"target":target}),
        )["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == parameter)
            .unwrap()["values"]["ids"][0]
            .clone()
    };

    // No float imagemap of the image yet: one is created and connected.
    let first =
        apply(json!({"kind":"connect_texture","target":shape,"parameter":"alpha","image":image}));
    assert!(first.error.is_none(), "{:?}", first.error);
    let created = first.result.unwrap()["created"].clone();
    let created = created.as_array().unwrap();
    assert_eq!(created.len(), 1, "{created:?}");
    let mask = created[0]["id"].clone();
    assert_eq!(connected(&shape, "alpha"), mask);
    // The same image on another float row reuses it.
    let second = apply(
        json!({"kind":"connect_texture","target":shape,"parameter":"shadowalpha","image":image}),
    );
    assert!(second.error.is_none(), "{:?}", second.error);
    assert!(
        second.result.unwrap()["created"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(connected(&shape, "shadowalpha"), mask);
    // A spectrum row reuses the scene's spectrum imagemap of the image.
    let target = json!({"kind":"resource","id":material});
    let third = apply(
        json!({"kind":"connect_texture","target":target,"parameter":"reflectance","image":image}),
    );
    assert!(third.error.is_none(), "{:?}", third.error);
    assert!(
        third.result.unwrap()["created"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    assert_eq!(connected(&target, "reflectance"), rgbcheck);

    let refused =
        apply(json!({"kind":"connect_texture","target":shape,"parameter":"alpha","image":mesh}));
    assert_eq!(
        refused.error.unwrap().details.unwrap()["reason"],
        "not_an_image"
    );

    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":project(),"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "succeeded", "{job}");
    let text = fs::read_to_string(job["result"]["scene_path"].as_str().unwrap()).unwrap();
    assert!(
        text.contains("Texture \"checker\" \"float\" \"imagemap\""),
        "{text}"
    );
    assert!(text.contains("\"texture alpha\" [ \"checker\" ]"), "{text}");
}

/// A shape's alpha mask takes a float texture through connect_texture, as
/// a material parameter does: the table decides which node rows take one and
/// of which lineage, and disconnecting removes the declaration.
#[test]
fn a_shape_alpha_connects_and_disconnects_a_float_texture() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = || state(&core)["context"]["project"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project(),"query":"Shape"}),
    );
    let shape = hits["nodes"][0]["node_id"].clone();
    let apply = |edits: Value| {
        core.request(request(
            Method::ProjectApply,
            json!({"project":project(),"edits":edits}),
        ))
    };
    let create = |label: &str, value_type: &str| {
        apply(json!([{"kind":"create_resource","resource_kind":"texture",
            "implementation":"constant","label":label,"texture_value_type":value_type}]))
        .result
        .unwrap()["created"][0]["id"]
            .clone()
    };
    let mask = create("Mask", "float");
    let tint = create("Tint", "spectrum");
    let node = json!({"kind":"node","id":shape});
    let reason = |response: protocol::Response| {
        response.error.expect("refused").details.unwrap()["reason"].clone()
    };

    let connected = apply(json!([{"kind":"connect_texture","target":node,
        "parameter":"alpha","texture":mask}]));
    assert!(connected.error.is_none(), "{:?}", connected.error);
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":project(),"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "succeeded", "{job}");
    let text = fs::read_to_string(job["result"]["scene_path"].as_str().unwrap()).unwrap();
    assert!(text.contains("\"texture alpha\" [ \"Mask\" ]"), "{text}");

    assert_eq!(
        reason(apply(json!([{"kind":"connect_texture","target":node,
            "parameter":"shadowalpha","texture":tint}]))),
        "texture_lineage"
    );
    assert_eq!(
        reason(apply(json!([{"kind":"connect_texture","target":node,
            "parameter":"radius","texture":mask}]))),
        "not_a_texture_input"
    );

    let disconnected = apply(json!([{"kind":"disconnect_texture","target":node,
        "parameter":"alpha"}]));
    assert!(disconnected.error.is_none(), "{:?}", disconnected.error);
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":project(),"target":node}),
    );
    assert!(
        !parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["name"] == "alpha"),
        "{parameters}"
    );
}

/// An imagemap without an image the scene does not use is left out of
/// export and preview, which say so in `warnings`; once the scene uses it,
/// both fail as an unresolved reference naming the texture.
#[test]
fn imagemaps_without_an_image_warn_when_skipped_and_fail_as_unresolved_when_used() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":state(&core)["context"]["project"],"edits":[{"kind":"create_resource",
            "resource_kind":"texture","implementation":"imagemap","label":"Pending",
            "texture_value_type":"spectrum","parameters":[]}]}),
    );
    let pending = applied["created"][0]["id"].clone();
    let run = |method: Method, name: &str| {
        let job = call(
            &core,
            method,
            json!({"project":state(&core)["context"]["project"],"destination":temp.path().join(name)}),
        );
        wait(&core, job)
    };
    for (method, name) in [
        (Method::ProjectExport, "bundle"),
        (Method::ProjectPreview, "preview"),
    ] {
        let job = run(method, name);
        assert_eq!(job["state"], "succeeded", "{job}");
        let warnings = job["result"]["warnings"].as_array().unwrap();
        assert_eq!(warnings.len(), 1, "{job}");
        assert_eq!(warnings[0]["code"], "imagemap_without_image_skipped");
        assert_eq!(warnings[0]["resource_id"], pending);
        assert!(warnings[0]["message"].as_str().unwrap().contains("Pending"));
    }

    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":state(&core)["context"]["project"],"folder":"Materials"}),
    );
    let material = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "resource" && entry["resource_kind"] == "material")
        .unwrap()["id"]
        .clone();
    call(
        &core,
        Method::ProjectApply,
        json!({"project":state(&core)["context"]["project"],"edits":[{"kind":"connect_texture",
            "target":{"kind":"resource","id":material},"parameter":"reflectance","texture":pending}]}),
    );
    for (method, name) in [
        (Method::ProjectExport, "used"),
        (Method::ProjectPreview, "used-preview"),
    ] {
        let job = run(method, name);
        assert_eq!(job["state"], "failed", "{job}");
        assert_eq!(job["error"]["code"], "reference_unresolved", "{job}");
        assert_eq!(job["error"]["target_id"], pending, "{job}");
        assert_eq!(
            job["error"]["details"]["reason"], "imagemap_without_image",
            "{job}"
        );
    }
}

#[test]
fn an_unfinished_mix_stops_export_but_not_the_viewport_preview() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    // New Material ▸ mix leaves `materials` undeclared; nothing uses it.
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":state(&core)["context"]["project"],"edits":[{"kind":"create_resource",
            "resource_kind":"material","implementation":"mix","label":"Blend","parameters":[]}]}),
    );
    let mix = applied["created"][0]["id"].clone();
    let run = |method: Method, name: &str| {
        wait(
            &core,
            call(
                &core,
                method,
                json!({"project":state(&core)["context"]["project"],"destination":temp.path().join(name)}),
            ),
        )
    };
    let job = run(Method::ProjectExport, "bundle");
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["error"]["code"], "invalid_request", "{job}");
    assert_eq!(
        job["error"]["details"]["reason"], "missing_required",
        "{job}"
    );
    let missing = &job["error"]["details"]["missing"][0];
    assert_eq!(missing["id"], mix, "{job}");
    assert_eq!(missing["parameter"], "materials", "{job}");
    assert_eq!(
        job["error"]["message"],
        "material \"Blend\": \"materials\" is required"
    );
    // The viewport keeps updating while the mix is being set up.
    let job = run(Method::ProjectPreview, "preview");
    assert_eq!(job["state"], "succeeded", "{job}");
}

#[test]
fn material_resource_edits_export_and_long_text_previews_cannot_be_mistaken_for_complete_values() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nMaterial \"diffuse\" \"rgb reflectance\" [0.5 0.5 0.5]\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference,"folder":"Materials"}),
    );
    let material = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "resource" && entry["resource_kind"] == "material")
        .unwrap();
    let target = json!({"kind":"resource","id":material["id"]});
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"set_parameter","target":target,"name":"reflectance","value_type":"rgb","value":{"name":"reflectance","value_type":"rgb","values":{"kind":"floats","value":[0.1,0.2,0.3]}}}]}),
    );
    let current = state(&core)["context"]["project"].clone();
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":current,"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "succeeded", "{job}");
    let text = fs::read_to_string(job["result"]["scene_path"].as_str().unwrap()).unwrap();
    assert!(
        text.contains("\"rgb reflectance\" [ 0.1 0.2 0.3 ]"),
        "{text}"
    );
    // an unmanaged string dependency on a material would now be
    // rejected at apply time (not_exportable), so the long-text preview
    // check rides on a plain string parameter that stays exportable.
    let name = "wrap".to_string();
    let value = "界".repeat(300);
    call(
        &core,
        Method::ProjectApply,
        json!({"project":current,"edits":[{"kind":"set_parameter","target":target,"name":name,"value_type":"string","value":{"name":name,"value_type":"string","values":{"kind":"strings","value":[value]}}}]}),
    );
    let preview = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":state(&core)["context"]["project"],"target":target}),
    );
    let parameter = preview["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == name)
        .unwrap();
    assert_eq!(parameter["preview_truncated"], true);
    assert_eq!(parameter["value_count"], "1");
    assert_eq!(
        parameter["values"]["value"][0]
            .as_str()
            .unwrap()
            .chars()
            .count(),
        256
    );
}
#[test]
fn native_project_jobs_editing_replay_save_and_reopen_share_one_state() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let destination = temp.path().join("project");
    fs::write(
        &source,
        "WorldBegin\nTranslate 1 0 0\nShape \"sphere\" \"float radius\" [1]\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let other = host.handle();
    let loaded = import(&core, &source, &destination);
    assert_eq!(loaded["result"]["activated"], true);
    let initial = state(&core);
    assert_eq!(initial["dirty"], false);
    assert_eq!(initial["preview_connected"], false);
    let reference = initial["context"]["project"].clone();
    let root = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference}),
    );
    let world = root["nodes"][0]["node_id"].clone();
    let children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":world}),
    );
    // wraps the world-direct transform: the Translate and the Shape
    // live one level down, in the wrapper group.
    let nested = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":children["nodes"][0]["node_id"]}),
    );
    let transform = nested["nodes"][0]["node_id"].clone();
    let shape = nested["nodes"][1]["node_id"].clone();
    let edits = json!([{"kind":"set_transform","node_id":transform,"value":{"kind":"translate","value":[2,3,4]}},{"kind":"set_parameter","target":{"kind":"node","id":shape},"name":"radius","value_type":"float","value":{"name":"radius","value_type":"float","values":{"kind":"floats","value":[2]}}}]);
    let operation = request(
        Method::ProjectApply,
        json!({"project":reference,"edits":edits}),
    );
    let applied = core.request(operation.clone());
    assert!(applied.error.is_none(), "{:?}", applied.error);
    assert_eq!(other.request(operation.clone()), applied);
    assert_eq!(state(&core)["context"]["project"]["revision"], "1");
    let mut reused = operation.clone();
    reused.params["edits"][0]["value"]["value"] = json!([9, 0, 0]);
    assert_eq!(
        core.request(reused).error.unwrap().code,
        ErrorCode::RequestIdConflict
    );
    let stale = core.request(request(Method::ProjectUndo, json!({"project":reference})));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);
    let changed = state(&core)["context"]["project"].clone();
    call(&core, Method::ProjectUndo, json!({"project":changed}));
    call(
        &core,
        Method::ProjectRedo,
        json!({"project":state(&core)["context"]["project"]}),
    );
    let saved_ref = state(&core)["context"]["project"].clone();
    let save = call(&core, Method::ProjectSave, json!({"project":saved_ref}));
    call(
        &other,
        Method::ProjectApply,
        json!({"project":saved_ref,"edits":[{"kind":"rename_node","node_id":shape,"label":"Newer edit"}]}),
    );
    let saved = wait(&core, save);
    assert_eq!(saved["state"], "succeeded");
    assert_eq!(saved["result"]["saved_revision"], "3");
    assert_eq!(state(&core)["dirty"], true);
    let denied = core.request(request(
        Method::ProjectClose,
        json!({"expected_current":state(&core)["context"]}),
    ));
    assert_eq!(denied.error.unwrap().code, ErrorCode::UnsavedChanges);
    let save = call(
        &core,
        Method::ProjectSave,
        json!({"project":state(&core)["context"]["project"]}),
    );
    assert_eq!(wait(&core, save)["state"], "succeeded");
    let before_close = state(&core)["context"]["project"].clone();
    call(
        &core,
        Method::ProjectClose,
        json!({"expected_current":state(&core)["context"]}),
    );
    let open = call(
        &core,
        Method::ProjectOpen,
        json!({"path":destination,"expected_current":state(&core)["context"]}),
    );
    let opened = wait(&core, open);
    assert_eq!(opened["state"], "succeeded", "{opened}");
    let reopened = state(&core);
    assert_eq!(reopened["dirty"], false);
    assert_eq!(reopened["can_undo"], false);
    assert_eq!(
        reopened["context"]["project"]["project_id"],
        reference["project_id"]
    );
    let stale = core.request(request(
        Method::ProjectGetNode,
        json!({"project":before_close,"node_id":shape}),
    ));
    assert_eq!(stale.error.unwrap().code, ErrorCode::StaleProject);
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reopened["context"]["project"],"node_id":shape}),
    );
    assert_eq!(details["label"], "Newer edit");
    let target = temp.path().join("bundle");
    let job = call(
        &core,
        Method::ProjectExport,
        json!({"project":reopened["context"]["project"],"destination":target}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");
    assert!(
        fs::read_to_string(target.join("scene.pbrt"))
            .unwrap()
            .contains("Translate 2 3 4")
    );
}
#[test]
fn lock_failures_unknown_fields_and_v1_clients_cannot_replace_current_project() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    let destination = temp.path().join("project");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &destination);
    let previous = state(&core);
    let host2 = CoreHost::start(Limits::default());
    let other = host2.handle();
    let open = call(
        &other,
        Method::ProjectOpen,
        json!({"path":destination,"expected_current":state(&other)["context"]}),
    );
    let failed = wait(&other, open);
    assert_eq!(failed["state"], "failed");
    assert_eq!(failed["error"]["code"], "project_locked");
    assert!(state(&other)["context"]["project"].is_null());
    let invalid = core.request(request(
        Method::ProjectApply,
        json!({"project":previous["context"]["project"],"edits":[],"unexpected":true}),
    ));
    assert_eq!(invalid.error.unwrap().code, ErrorCode::InvalidRequest);
    let missing = call(
        &core,
        Method::ProjectOpen,
        json!({"path":temp.path().join("missing"),"expected_current":previous["context"]}),
    );
    assert_eq!(wait(&core, missing)["state"], "failed");
    assert_eq!(state(&core)["context"], previous["context"]);
    let mut old = request(
        Method::AppHello,
        json!({"client_name":"old","supported_protocols":["1"]}),
    );
    old.protocol = "1".into();
    assert_eq!(
        core.request(old).error.unwrap().code,
        ErrorCode::UnsupportedProtocol
    );
}

#[cfg(unix)]
#[test]
fn project_capture_pins_snapshot_and_reports_the_view_used() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys
spec = json.load(open(sys.argv[2]))
geometry = json.load(open(spec['geometry_path']))
open(sys.argv[3], 'wb').write(json.dumps({'draws': len(geometry['draws']), 'size': spec['size'], 'debug_albedo': spec['debug_albedo'], 'debug_ao': spec['debug_ao']}).encode())
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let services = Arc::new(tempfile::tempdir().unwrap());
    let project_folder = temp.path().join("project");
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture),
            renderer: None,
            directory: services.clone(),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let pinned = state(&core)["context"]["project"].clone();
    // The capture adapter is attached, so shared capture APIs report native
    // geometry serviceability honestly.
    assert_eq!(state(&core)["preview_connected"], true);
    let job = call(
        &core,
        Method::ProjectCapture,
        json!({"project":pinned,"debug_albedo":true}),
    );
    let job = wait(&core, job);
    assert_eq!(job["state"], "succeeded", "{job}");
    // Capture consumes the direct package: one published slot with geometry,
    // and no scene text anywhere in the service directory.
    let jobs = project_folder.join("Library/Jobs");
    let packages: Vec<_> = fs::read_dir(&jobs)
        .unwrap()
        .filter_map(|entry| entry.ok())
        // The jobs folders sit beside their owner lock file.
        .filter(|entry| entry.path().is_dir())
        .flat_map(|entry| {
            fs::read_dir(entry.path())
                .unwrap()
                .filter_map(|child| child.ok())
        })
        .filter(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .starts_with("project-preview-")
        })
        .collect();
    assert_eq!(packages.len(), 1, "{packages:?}");
    assert!(packages[0].path().join("geometry.json").is_file());
    for job_dir in fs::read_dir(&jobs).unwrap() {
        let job_dir = job_dir.unwrap().path();
        if !job_dir.is_dir() {
            continue;
        }
        for entry in fs::read_dir(job_dir).unwrap() {
            let path = entry.unwrap().path();
            assert!(!path.join("scene.pbrt").exists(), "{path:?}");
            assert!(!path.join("export.json").exists(), "{path:?}");
        }
    }
    assert_eq!(job["kind"], "capture");
    assert_eq!(job["project"], pinned);
    let artifact = call(
        &core,
        Method::ArtifactGet,
        json!({"artifact_id":job["result"]["artifact_id"]}),
    );
    assert_eq!(artifact["mime"], "image/png");
    assert_eq!(artifact["metadata"]["project"], pinned);
    assert_eq!(artifact["metadata"]["debug_albedo"], true);
    let seen: Value =
        serde_json::from_slice(&fs::read(artifact["path"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(seen["draws"], 1);
    assert_eq!(seen["debug_albedo"], true);
    assert_eq!(seen["debug_ao"], false);
    // AO capture reaches the adapter too; it cannot combine with albedo.
    let job = wait(
        &core,
        call(
            &core,
            Method::ProjectCapture,
            json!({"project":pinned,"debug_ao":true}),
        ),
    );
    assert_eq!(job["state"], "succeeded", "{job}");
    let artifact = call(
        &core,
        Method::ArtifactGet,
        json!({"artifact_id":job["result"]["artifact_id"]}),
    );
    assert_eq!(artifact["metadata"]["debug_ao"], true);
    let seen: Value =
        serde_json::from_slice(&fs::read(artifact["path"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(
        (seen["debug_albedo"].clone(), seen["debug_ao"].clone()),
        (json!(false), json!(true))
    );
    let both = core.request(request(
        Method::ProjectCapture,
        json!({"project":pinned,"debug_albedo":true,"debug_ao":true}),
    ));
    assert_eq!(both.error.unwrap().code, ErrorCode::InvalidRequest);
    let stale = core.request(request(
        Method::ProjectCapture,
        json!({"project":{"project_id":pinned["project_id"],"revision":"999","activation":pinned["activation"]}}),
    ));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);
}

/// Whether a `[f64; 3]` wire point sits at `expected` (the scene camera's
/// eye and target come back through f32 tessellation bounds).
#[cfg(unix)]
fn wire_point_is_near(point: &Value, expected: [f64; 3]) -> bool {
    point.as_array().is_some_and(|values| {
        values
            .iter()
            .zip(expected)
            .all(|(value, want)| (value.as_f64().unwrap() - want).abs() < 1e-9)
    })
}

/// The fake capture adapter used by the capture tests: it records the
/// spec's camera, size, mode and time instead of a PNG so the tests can
/// compare what the adapter actually received.
#[cfg(unix)]
fn recording_capture_adapter(temp: &std::path::Path) -> std::path::PathBuf {
    use std::os::unix::fs::PermissionsExt;
    let capture = temp.join("capture.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys
spec = json.load(open(sys.argv[2]))
open(sys.argv[3], 'wb').write(json.dumps({
    'camera': spec['camera'],
    'size': spec['size'],
    'mode': spec['mode'],
    'time': spec['time'],
}).encode())
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    capture
}

/// The scene the capture tests render: one perspective camera over a
/// sphere with the Film line supplied per case.
#[cfg(unix)]
fn filmed_scene(camera: &str, film: &str) -> String {
    format!("LookAt 0 5 15 0 0 0 0 1 0\n{camera}\n{film}\nWorldBegin\nShape \"sphere\"\n")
}

/// What the adapter recorded for one finished capture job.
#[cfg(unix)]
fn recorded_spec(core: &CoreHandle, job: &Value) -> Value {
    let artifact = call(
        core,
        Method::ArtifactGet,
        json!({"artifact_id":job["result"]["artifact_id"]}),
    );
    assert_eq!(artifact["mime"], "image/png", "{artifact}");
    serde_json::from_slice(&fs::read(artifact["path"].as_str().unwrap()).unwrap()).unwrap()
}

/// A capture with no view at all — and a `camera: "scene"` capture —
/// sizes itself at the scene Film's aspect ratio (`frameaspectratio` when
/// set, `film_resolution` otherwise), long side 1280, rather than
/// initial_view's fixed 1280×720.
#[cfg(unix)]
#[test]
fn capture_sizes_the_scene_camera_at_the_films_aspect_ratio() {
    use std::sync::Arc;
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(recording_capture_adapter(temp.path())),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let cases = [
        // Landscape 16:9 film.
        (
            filmed_scene(
                "Camera \"perspective\" \"float fov\" [45]",
                "Film \"rgb\" \"integer xresolution\" [1920] \"integer yresolution\" [1080]",
            ),
            [1280, 720],
        ),
        // Portrait film: the long side is the height.
        (
            filmed_scene(
                "Camera \"perspective\" \"float fov\" [45]",
                "Film \"rgb\" \"integer xresolution\" [600] \"integer yresolution\" [800]",
            ),
            [960, 1280],
        ),
        // An explicit frameaspectratio wins over the film resolution.
        (
            filmed_scene(
                "Camera \"perspective\" \"float fov\" [45] \"float frameaspectratio\" [2.0]",
                "Film \"rgb\" \"integer xresolution\" [640] \"integer yresolution\" [480]",
            ),
            [1280, 640],
        ),
    ];
    for (index, (scene, expected)) in cases.into_iter().enumerate() {
        let source = temp.path().join(format!("source{index}.pbrt"));
        fs::write(&source, &scene).unwrap();
        import(&core, &source, &temp.path().join(format!("project{index}")));
        let reference = state(&core)["context"]["project"].clone();
        for camera in [Value::Null, json!("scene")] {
            let mut params = json!({"project":reference.clone()});
            params["camera"] = camera.clone();
            let job = wait(&core, call(&core, Method::ProjectCapture, params));
            assert_eq!(job["state"], "succeeded", "{job} (case {index})");
            let view = &job["result"]["view"];
            assert_eq!(
                view["size"],
                json!(expected),
                "case {index}, camera {camera}: {view}"
            );
            // The manifest's scene camera frames it — not the viewer's
            // 16:9 fallback framing.
            assert!(
                wire_point_is_near(&view["camera"]["eye"], [0., 5., 15.]),
                "case {index}, camera {camera}: {view}"
            );
            assert!(
                wire_point_is_near(&view["camera"]["target"], [0., 0., 0.]),
                "case {index}, camera {camera}: {view}"
            );
            // The adapter rendered at the same size, not just the report.
            assert_eq!(recorded_spec(&core, &job)["size"], json!(expected), "{job}");
        }
    }
}

/// With a shared view present, an omitted camera or `"view"` keeps
/// that whole shared view; `"scene"` ignores its camera pose and size but
/// keeps its mode and time.
#[cfg(unix)]
#[test]
fn scene_camera_capture_keeps_the_shared_views_mode_and_time() {
    use std::sync::Arc;
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(recording_capture_adapter(temp.path())),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(
        &source,
        "LookAt 0 5 15 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [1920] \"integer yresolution\" [1080]\n\
         WorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    // A shared view whose pose and size differ from the scene camera's.
    let shared_camera = json!({
        "eye": [1.0, 2.0, 12.0], "target": [0.0, 0.0, 0.0], "up": [0.0, 1.0, 0.0],
        "projection": "perspective", "fov_y_degrees": 40.,
        "near": 0.1, "far": 50.
    });
    call(
        &core,
        Method::ProjectSetCamera,
        json!({"project":reference(),"expected_view_revision":"0","camera":shared_camera}),
    );
    call(
        &core,
        Method::ProjectConfigureView,
        json!({"project":reference(),"expected_view_revision":"1","size":[37,64],"mode":"lit","time":0.25}),
    );
    // Omitted, or "view": exactly today's shared-view behavior.
    for camera in [Value::Null, json!("view")] {
        let mut params = json!({"project":reference()});
        params["camera"] = camera.clone();
        let job = wait(&core, call(&core, Method::ProjectCapture, params));
        assert_eq!(job["state"], "succeeded", "{job} (camera {camera})");
        let view = &job["result"]["view"];
        assert_eq!(view["camera"], shared_camera, "{view}");
        assert_eq!(view["size"], json!([37, 64]), "{view}");
        assert_eq!(view["mode"], "lit", "{view}");
        assert_eq!(view["time"], 0.25, "{view}");
    }
    // "scene": the manifest camera and the Film's aspect ratio, with the
    // shared view's mode and time carried over.
    let job = wait(
        &core,
        call(
            &core,
            Method::ProjectCapture,
            json!({"project":reference(),"camera":"scene"}),
        ),
    );
    assert_eq!(job["state"], "succeeded", "{job}");
    let view = &job["result"]["view"];
    assert!(
        wire_point_is_near(&view["camera"]["eye"], [0., 5., 15.]),
        "{view}"
    );
    assert!(
        wire_point_is_near(&view["camera"]["target"], [0., 0., 0.]),
        "{view}"
    );
    assert_ne!(view["camera"], shared_camera, "{view}");
    assert_eq!(view["size"], json!([1280, 720]), "{view}");
    assert_eq!(view["mode"], "lit", "{view}");
    assert_eq!(view["time"], 0.25, "{view}");
    let seen = recorded_spec(&core, &job);
    assert_eq!(seen["mode"], "lit", "{seen}");
    assert!(
        wire_point_is_near(&seen["camera"]["eye"], [0., 5., 15.]),
        "{seen}"
    );
}

/// An explicit view cannot combine with a camera selection, either
/// value — the same refusal render makes for a scene-camera render.
#[cfg(unix)]
#[test]
fn capture_refuses_an_explicit_view_with_a_camera() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("source.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    let view = protocol::view::ViewState {
        camera: core::view::frame(None, [32, 24]),
        size: [32, 24],
        mode: protocol::view::DisplayMode::Solid,
        time: 0.0,
    };
    for camera in ["scene", "view"] {
        let refused = core.request(request(
            Method::ProjectCapture,
            json!({"project":reference,"camera":camera,"view":view}),
        ));
        assert_eq!(
            refused.error.unwrap().code,
            ErrorCode::InvalidRequest,
            "camera {camera} must not combine with a view"
        );
    }
}

#[cfg(unix)]
#[test]
fn project_pick_maps_the_pixel_to_the_native_shape_node() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture.py");
    // Fake adapter: every pixel hits the first draw's pick id.
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, struct, sys
spec = json.load(open(sys.argv[2]))
geometry = json.load(open(spec['geometry_path']))
pick = geometry['draws'][0]['pick_id']
w, h = spec['size']
open(sys.argv[3], 'wb').write(b'png')
open(sys.argv[3].rsplit('.', 1)[0] + '.pick', 'wb').write(struct.pack('<I', pick) * (w * h))
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let pinned = state(&core)["context"]["project"].clone();
    let job = call(
        &core,
        Method::ProjectPick,
        json!({"project":pinned,"normalized_xy":[0.5,0.5]}),
    );
    let job = wait(&core, job);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["kind"], "pick");
    let node = job["result"]["node_id"].as_str().expect("node id");
    assert_eq!(job["result"]["node"]["kind"], "entity", "{job}");
    assert_eq!(job["result"]["node"]["detail"]["implementation"], "sphere");
    let described = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":pinned,"node_id":node}),
    );
    assert_eq!(described["node_id"], node, "{described}");
    let outside = core.request(request(
        Method::ProjectPick,
        json!({"project":pinned,"normalized_xy":[1.5,0.5]}),
    ));
    assert_eq!(outside.error.unwrap().code, ErrorCode::InvalidRequest);
}

#[cfg(unix)]
#[test]
fn project_apply_with_capture_captures_the_edited_revision() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture.py");
    fs::write(
        &capture,
        "#!/usr/bin/python3\nimport sys\nopen(sys.argv[3], 'wb').write(b'png')\n",
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let start = |capture: Option<std::path::PathBuf>| {
        CoreHost::start_with_services(
            Limits::default(),
            Some(core::services::ServiceConfig {
                gui: false,
                capture,
                renderer: None,
                directory: Arc::new(tempfile::tempdir().unwrap()),
                geometry_cache: Default::default(),
                capture_daemon: Default::default(),
                project_directories: Default::default(),
            }),
        )
    };
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nTranslate 1 0 0\nShape \"sphere\"\n").unwrap();
    let edit = |core: &CoreHandle, capture: Value| {
        let reference = state(core)["context"]["project"].clone();
        let roots = call(core, Method::ProjectListNodes, json!({"project":reference}));
        let children = call(
            core,
            Method::ProjectListNodes,
            json!({"project":reference,"parent":roots["nodes"][0]["node_id"]}),
        );
        // wraps the world-direct transform, so the Translate state
        // sits inside the wrapper group.
        let nested = call(
            core,
            Method::ProjectListNodes,
            json!({"project":reference,"parent":children["nodes"][0]["node_id"]}),
        );
        let transform = nested["nodes"][0]["node_id"].clone();
        core.request(request(
            Method::ProjectApply,
            json!({"project":reference,"capture":capture,"edits":[{"kind":"set_transform","node_id":transform,"value":{"kind":"translate","value":[5,0,0]}}]}),
        ))
    };
    let host = start(Some(capture));
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let response = edit(&core, json!({"view":null}));
    let result = response.result.expect("apply succeeds");
    let edited = state(&core)["context"]["project"].clone();
    assert_eq!(result["project"], edited);
    let job = wait(&core, json!({"job_id":result["capture_job_id"]}));
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["project"], edited);
    assert_ne!(edited["revision"], "0");

    let host = start(None);
    let core = host.handle();
    import(&core, &source, &temp.path().join("project2"));
    let before = state(&core)["context"]["project"].clone();
    let rejected = edit(&core, json!({"view":null}));
    assert_eq!(rejected.error.unwrap().code, ErrorCode::Unsupported);
    assert_eq!(state(&core)["context"]["project"], before);
}

#[test]
fn project_list_contents_without_a_folder_lists_the_assets_root() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nMakeNamedMaterial \"wall\" \"string type\" \"diffuse\"\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let omitted = call(
        &core,
        Method::ProjectListContents,
        json!({"project":project}),
    );
    let root = call(
        &core,
        Method::ProjectListContents,
        json!({"project":project,"folder":""}),
    );
    assert_eq!(omitted, root);
    assert_eq!(root["folder"], "");
    assert!(root.get("items").is_none(), "{root}");
    assert!(
        root["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "folder" && entry["location"] == "Materials"),
        "{root}"
    );
}

#[test]
fn project_list_contents_rejects_offset_past_the_end() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let response = core.request(request(
        Method::ProjectListContents,
        json!({"project":project,"offset":9999}),
    ));
    assert_eq!(response.error.unwrap().code, ErrorCode::InvalidRequest);
}

#[test]
fn project_search_nodes_reports_hits_with_ancestor_paths_and_pages() {
    let temp = tempfile::tempdir().unwrap();
    let host = core::CoreHost::start(protocol::commands::Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTranslate 1 0 0\nShape \"sphere\"\nShape \"disk\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let roots = call(&core, Method::ProjectListNodes, json!({"project":project}));
    let world = roots["nodes"].as_array().unwrap().last().unwrap().clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"sHaPe"}),
    );
    let nodes = hits["nodes"].as_array().unwrap();
    assert_eq!(hits["total"], 2, "{hits}");
    assert_eq!(nodes[0]["kind"], "entity");
    assert!(nodes[0]["label"].as_str().unwrap().contains("Shape"));
    for node in nodes {
        // wraps world-direct transforms, so the Scene root is the
        // wrapper's ancestor: it stays the last path entry.
        let path = node["path"].as_array().unwrap();
        assert_eq!(path[path.len() - 1], world["node_id"], "{node}");
    }
    let empty = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"brick"}),
    );
    assert_eq!(empty["total"], 0);
    assert!(empty["nodes"].as_array().unwrap().is_empty());
    assert!(empty["next_offset"].is_null());
    let paged = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"shape","limit":1}),
    );
    assert_eq!(paged["total"], 2);
    assert_eq!(paged["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(paged["next_offset"], 1);
    let empty_query = core.request(request(
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"  "}),
    ));
    assert_eq!(empty_query.error.unwrap().code, ErrorCode::InvalidRequest);
    let early = core.request(request(
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"sphere","offset":3}),
    ));
    assert_eq!(early.error.unwrap().code, ErrorCode::InvalidRequest);
}

#[test]
fn project_search_contents_matches_paths_and_builtin_presets_with_stable_pages() {
    let temp = tempfile::tempdir().unwrap();
    let host = core::CoreHost::start(protocol::commands::Limits::default());
    let core = host.handle();
    let source = temp.path().join("SearchScene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\nWorldEnd\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let root = call(
        &core,
        Method::ProjectListContents,
        json!({"project":project,"folder":"Scenes"}),
    );
    let scene_location = root["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "scene")
        .unwrap_or_else(|| panic!("{root}"))["location"]
        .as_str()
        .unwrap()
        .to_owned();

    let by_path = call(
        &core,
        Method::ProjectSearchContents,
        json!({"project":project,"query":scene_location.to_uppercase()}),
    );
    assert_eq!(by_path["project"], project);
    assert_eq!(by_path["query"], scene_location.to_uppercase());
    assert!(
        by_path["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["kind"] == "scene" && entry["location"] == scene_location }),
        "{by_path}"
    );

    let builtin = call(
        &core,
        Method::ProjectSearchContents,
        json!({"project":project,"query":"sPhErE"}),
    );
    assert!(
        builtin["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["builtin"] == true && entry["name"] == "Sphere" }),
        "{builtin}"
    );

    let first = call(
        &core,
        Method::ProjectSearchContents,
        json!({"project":project,"query":"s","limit":1}),
    );
    assert_eq!(first["entries"].as_array().unwrap().len(), 1);
    let next = first["next_offset"].as_u64().unwrap();
    let second = call(
        &core,
        Method::ProjectSearchContents,
        json!({"project":project,"query":"s","limit":1,"offset":next}),
    );
    assert_eq!(first["total"], second["total"]);
    assert_ne!(first["entries"][0], second["entries"][0]);

    let empty = core.request(request(
        Method::ProjectSearchContents,
        json!({"project":project,"query":"  "}),
    ));
    assert_eq!(empty.error.unwrap().code, ErrorCode::InvalidRequest);
    let long = core.request(request(
        Method::ProjectSearchContents,
        json!({"project":project,"query":"x".repeat(257)}),
    ));
    assert_eq!(long.error.unwrap().code, ErrorCode::InvalidRequest);
    let past_end = core.request(request(
        Method::ProjectSearchContents,
        json!({"project":project,"query":"sphere","offset":99}),
    ));
    assert_eq!(past_end.error.unwrap().code, ErrorCode::InvalidRequest);
}

// Unix-only: the fake adapter is a shell script.
#[cfg(unix)]
#[test]
fn project_capture_daemon_serves_repeated_jobs_from_one_process() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture-server.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys, os
count_path = sys.argv[0].rsplit('.', 1)[0] + '.count'
with open(count_path, 'a') as counter:
    counter.write('1')
print(json.dumps({"ready": True, "protocol": 1}), flush=True)
for line in sys.stdin:
    request = json.loads(line)
    if request.get("hello"):
        print(json.dumps({"ok": True, "protocol": 1}), flush=True)
        continue
    width, height = request["spec"]["size"]
    open(request["output"], "wb").write(b"png-bytes")
    open(os.path.splitext(request["output"])[0] + ".pick", "wb").write(b"\x00" * 4 * width * height)
    print(json.dumps({"ok": True, "output": request["output"]}), flush=True)
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture.clone()),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    for _ in 0..2 {
        let job = call(&core, Method::ProjectCapture, json!({"project":reference}));
        let job = wait(&core, job);
        assert_eq!(job["state"], "succeeded", "{job}");
    }
    let count =
        fs::read_to_string(capture.to_string_lossy().trim_end_matches(".py").to_owned() + ".count")
            .unwrap_or_default();
    assert_eq!(
        count.len(),
        1,
        "resident daemon must spawn exactly once: {count}"
    );
}

// Unix-only: the fake adapter is a shell script.
#[cfg(unix)]
#[test]
fn project_capture_cancelled_mid_render_abandons_the_daemon_quickly() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture-server.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys, time
print(json.dumps({"ready": True, "protocol": 1}), flush=True)
for line in sys.stdin:
    time.sleep(30)
    request = json.loads(line)
    open(request["output"], "wb").write(b"png-bytes")
    print(json.dumps({"ok": True, "output": request["output"]}), flush=True)
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture.clone()),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    let started = call(&core, Method::ProjectCapture, json!({"project":reference}));
    // Give the daemon time to accept the request before cancelling.
    std::thread::sleep(Duration::from_millis(300));
    call(
        &core,
        Method::JobCancel,
        json!({"job_id":started["job_id"]}),
    );
    let begin = Instant::now();
    let job = wait(&core, started);
    assert_eq!(job["state"], "cancelled", "{job}");
    assert!(job["result"].is_null(), "{job}");
    assert!(
        begin.elapsed() < Duration::from_secs(10),
        "cancellation must not wait for a stuck adapter"
    );
    // The cancelled session is abandoned; the respawned daemon serves the
    // next request, and cancelling that one works the same way.
    let recovered = call(&core, Method::ProjectCapture, json!({"project":reference}));
    std::thread::sleep(Duration::from_millis(300));
    call(
        &core,
        Method::JobCancel,
        json!({"job_id":recovered["job_id"]}),
    );
    let begin = Instant::now();
    let job = wait(&core, recovered);
    assert_eq!(job["state"], "cancelled", "{job}");
    assert!(
        begin.elapsed() < Duration::from_secs(10),
        "the respawned daemon must honor cancellation too"
    );
}

// Unix-only: the fake adapter is a shell script.
#[cfg(unix)]
#[test]
fn project_view_is_revision_checked_and_feeds_captures_and_ipr() {
    use std::{os::unix::fs::PermissionsExt, sync::Arc};
    let temp = tempfile::tempdir().unwrap();
    let capture = temp.path().join("capture.py");
    fs::write(
        &capture,
        r#"#!/usr/bin/python3
import json, sys
spec = json.load(open(sys.argv[2]))
geometry = json.load(open(spec['geometry_path']))
open(sys.argv[3], 'wb').write(json.dumps({
    'draws': len(geometry['draws']),
    'camera': spec['camera'],
    'frame_key': spec['frame_key'],
}).encode())
width, height = spec['size']
open(sys.argv[3][:-3] + 'pick', 'wb').write(b'\x00' * 4 * width * height)
"#,
    )
    .unwrap();
    fs::set_permissions(&capture, fs::Permissions::from_mode(0o755)).unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture),
            renderer: None,
            directory: Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();

    // Fresh activations have no stored view: captures use the manifest framing
    // revision zero until a client establishes one.
    let view = call(&core, Method::ProjectGetView, json!({"project":reference}));
    assert!(view["view"].is_null(), "{view}");
    assert_eq!(view["view_revision"], "0");

    // Establishing or changing the camera is checked; stale requests conflict.
    let camera = json!({
        "eye": [0.0, 3.0, 12.0], "target": [0.0, 0.0, 0.0], "up": [0.0, 1.0, 0.0],
        "projection": "perspective", "fov_y_degrees": 40.,
        "near": 0.1, "far": 50.
    });
    let set = call(
        &core,
        Method::ProjectSetCamera,
        json!({"project":reference,"expected_view_revision":"0","camera":camera}),
    );
    assert_eq!(set["view_revision"], "1");
    assert_eq!(set["view"]["camera"], camera);
    let stale = core.request(request(
        Method::ProjectSetCamera,
        json!({"project":reference,"expected_view_revision":"0","camera":camera}),
    ));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);

    // Capture without an explicit view pins the shared camera and time.
    let configured = call(
        &core,
        Method::ProjectConfigureView,
        json!({"project":reference,"expected_view_revision":"1","size":[64,64],"time":0.5}),
    );
    assert_eq!(configured["view_revision"], "2");
    let job = call(&core, Method::ProjectCapture, json!({"project":reference}));
    let job = wait(&core, job);
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(job["result"]["view"]["camera"], camera, "{}", job["result"]);
    assert_eq!(job["result"]["view"]["size"], json!([64, 64]));
    assert_eq!(job["result"]["view"]["time"], 0.5);

    // Pick's frame key carries the real view revision now.
    let picked = call(
        &core,
        Method::ProjectPick,
        json!({"project":reference,"normalized_xy":[0.5,0.5]}),
    );
    let picked = wait(&core, picked);
    assert_eq!(picked["state"], "succeeded", "{picked}");
    assert_eq!(picked["result"]["view_revision"], "2", "{picked}");
    let artifact = call(
        &core,
        Method::ArtifactGet,
        json!({"artifact_id":job["result"]["artifact_id"]}),
    );
    let seen: Value =
        serde_json::from_slice(&fs::read(artifact["path"].as_str().unwrap()).unwrap()).unwrap();
    assert_eq!(seen["camera"], camera, "{seen}");
    assert_eq!(seen["frame_key"]["view_revision"], "2");

    // A different activation resets the stored view.
    import(&core, &source, &temp.path().join("project2"));
    let next = state(&core)["context"]["project"].clone();
    assert_ne!(next["project_id"], reference["project_id"]);
    assert_ne!(next["activation"], reference["activation"]);
    let view = call(&core, Method::ProjectGetView, json!({"project":next}));
    assert!(
        view["view"].is_null(),
        "the next activation starts clean: {view}"
    );
    assert_eq!(view["view_revision"], "0");
}

#[test]
fn project_preview_job_publishes_direct_geometry() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 3 2 6 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\nFilm \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\nWorldBegin\nAttributeBegin\nTranslate -1.5 0 0\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\nAttributeBegin\nTranslate 1.5 0 0\nShape \"cone\" \"float radius\" [0.5] \"float height\" [1]\nAttributeEnd\nWorldEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();

    // The direct package: geometry + assets + audit record, no scene.pbrt.
    let slot = temp.path().join("preview-slot");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":slot}),
    );
    let finished = wait(&core, job);
    assert_eq!(finished["state"], "succeeded", "{finished}");
    let result = &finished["result"];
    assert_eq!(result["published"], true);
    assert!(
        result["geometry_path"]
            .as_str()
            .unwrap()
            .ends_with("geometry.json"),
        "{result}"
    );
    assert!(result.get("scene_path").is_none(), "{result}");
    assert!(slot.join("geometry.json").is_file());
    assert!(slot.join("preview.json").is_file());
    assert!(slot.join("assets").is_dir());
    assert!(!slot.join("scene.pbrt").exists());
    let manifest: Value =
        serde_json::from_str(&fs::read_to_string(slot.join("geometry.json")).unwrap()).unwrap();
    assert_eq!(manifest["draws"].as_array().unwrap().len(), 2);
    assert_eq!(result["draws"], json!(2));
    assert_eq!(result["unsupported"], json!(0));
    for draw in manifest["draws"].as_array().unwrap() {
        let entity = draw["entity_id"].as_str().expect("draws carry entity pins");
        let preview: Value =
            serde_json::from_str(&fs::read_to_string(slot.join("preview.json")).unwrap()).unwrap();
        assert!(
            preview["node_sources"]
                .as_object()
                .unwrap()
                .values()
                .any(|n| n.as_str() == Some(entity))
        );
    }
    // The staged temp directory must not linger next to the slot.
    let leftovers = fs::read_dir(temp.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().starts_with(".tmp"))
        .count();
    assert_eq!(leftovers, 0, "staged temp dir lingered");

    // Editing the project means the reference is stale: a preview request
    // for the old revision must not even enqueue.
    let stale = core.request(request(
        Method::ProjectPreview,
        json!({
            "project":{
                "project_id":project["project_id"],
                "activation":project["activation"],
                "revision":"9999"
            },
            "destination":slot
        }),
    ));
    assert_eq!(stale.error.unwrap().code, ErrorCode::RevisionConflict);

    // Publish is exclusive: the slot exists, so a second preview to the
    // same destination fails without clobbering the first bundle.
    let project = state(&core)["context"]["project"].clone();
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":slot}),
    );
    let finished = wait(&core, job);
    assert_eq!(finished["state"], "failed", "{finished}");
    assert!(slot.join("geometry.json").is_file());
}

/// a transform-only edit answers per-entity delta
/// matrices instead of a preview job. The delta times the draw matrix of a
/// real full preview at the base revision equals (within 1e-9) the draw
/// matrix of a real full preview at the new revision.
#[test]
fn preview_transforms_answers_deltas_matching_full_previews() {
    // Row-major 4x4 product, the convention every preview matrix uses.
    fn mul(a: &[f64], b: &[f64]) -> [f64; 16] {
        let mut out = [0.; 16];
        for row in 0..4 {
            for column in 0..4 {
                out[row * 4 + column] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum();
            }
        }
        out
    }
    fn matrix(value: &Value) -> Vec<f64> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_f64().unwrap())
            .collect()
    }

    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nTranslate 1 2 3\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\nWorldEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let base_revision = project["revision"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"translate"}),
    );
    let translate = hits["nodes"][0]["node_id"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"shape"}),
    );
    // An imported entity carries its directive name as the label.
    assert_eq!(hits["total"], json!(1), "{hits}");
    let sphere_shape = hits["nodes"][0]["node_id"].as_str().unwrap().to_owned();
    assert_eq!(hits["nodes"][0]["label"], json!("Shape"), "{hits}");

    // The displayed preview: a real preview job at the base revision, which
    // is what records the preview base the deltas diff against.
    let base_slot = temp.path().join("preview-base");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":base_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    // A move only: one set_transform on the imported Translate node.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"set_transform","node_id":translate,
            "value":{"kind":"translate","value":[4.,5.,6.]}}]}),
    );
    let moved = state(&core)["context"]["project"].clone();
    assert_eq!(moved["revision"], json!("1"));

    let answer = call(
        &core,
        Method::ProjectPreviewTransforms,
        json!({"project":moved,"base_revision":base_revision}),
    );
    assert_eq!(answer["fast"], json!(true), "{answer}");
    assert_eq!(answer["base_revision"], base_revision, "{answer}");
    assert_eq!(answer["revision"], json!("1"), "{answer}");
    let deltas = answer["deltas"].as_object().unwrap();
    assert_eq!(deltas.len(), 1, "only the sphere moved: {answer}");
    let delta = &deltas[&sphere_shape];
    let delta_start = matrix(&delta[0]);
    let delta_end = matrix(&delta[1]);
    assert_eq!(delta_start, delta_end, "the move stays static: {answer}");

    // The referee: a real full preview at the moved revision.
    let moved_slot = temp.path().join("preview-moved");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":moved,"destination":moved_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    let draw_world_matrix = |slot: &std::path::Path| -> Vec<f64> {
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(slot.join("geometry.json")).unwrap()).unwrap();
        let draw = manifest["draws"]
            .as_array()
            .unwrap()
            .iter()
            .find(|draw| draw["entity_id"].as_str() == Some(&sphere_shape))
            .expect("the sphere's draw is pinned by entity_id");
        assert_eq!(draw["world_from_object"].as_array().unwrap().len(), 1);
        matrix(&draw["world_from_object"][0])
    };
    let base_matrix = draw_world_matrix(&base_slot);
    let moved_matrix = draw_world_matrix(&moved_slot);

    let retargeted = mul(&delta_start, &base_matrix);
    for (delta_applied, full_preview) in retargeted.iter().zip(&moved_matrix) {
        assert!(
            (delta_applied - full_preview).abs() < 1e-9,
            "delta · base draw {delta_applied} != moved draw {full_preview}"
        );
    }
}

/// a moved point light stays fast. The answer's
/// delta keys the light node id the base light record pins by
/// `entity_id`, and `delta · the base world_from_light` equals (within
/// 1e-9) the `world_from_light` of a real full preview at the new
/// revision.
#[test]
fn preview_transforms_answers_light_deltas_matching_full_previews() {
    // Row-major 4x4 product, the convention every preview matrix uses.
    fn mul(a: &[f64], b: &[f64]) -> [f64; 16] {
        let mut out = [0.; 16];
        for row in 0..4 {
            for column in 0..4 {
                out[row * 4 + column] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum();
            }
        }
        out
    }
    fn matrix16(value: &Value) -> [f64; 16] {
        std::array::from_fn(|index| value[index].as_f64().unwrap())
    }

    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nTranslate 1 2 3\n\
         LightSource \"point\" \"rgb I\" [10 10 10]\nAttributeEnd\nWorldEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let base_revision = project["revision"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"translate"}),
    );
    assert_eq!(hits["total"], json!(1), "{hits}");
    let translate = hits["nodes"][0]["node_id"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"lightsource"}),
    );
    assert_eq!(hits["total"], json!(1), "{hits}");
    let light_id = hits["nodes"][0]["node_id"].as_str().unwrap().to_owned();

    // The displayed preview: a real preview job at the base revision,
    // which is what records the preview base the deltas diff against.
    let base_slot = temp.path().join("preview-base");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":base_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    // A move only: one set_transform on the light's Translate node.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"set_transform","node_id":translate,
            "value":{"kind":"translate","value":[4.,5.,6.]}}]}),
    );
    let moved = state(&core)["context"]["project"].clone();
    assert_eq!(moved["revision"], json!("1"));

    let answer = call(
        &core,
        Method::ProjectPreviewTransforms,
        json!({"project":moved,"base_revision":base_revision}),
    );
    assert_eq!(answer["fast"], json!(true), "{answer}");
    assert_eq!(answer["base_revision"], base_revision, "{answer}");
    assert_eq!(answer["revision"], json!("1"), "{answer}");
    let deltas = answer["deltas"].as_object().unwrap();
    assert_eq!(deltas.len(), 1, "only the light moved: {answer}");
    let delta = &deltas[&light_id];
    let delta_start = matrix16(&delta[0]);
    let delta_end = matrix16(&delta[1]);
    assert_eq!(delta_start, delta_end, "the move stays static: {answer}");

    // The referee: a real full preview at the moved revision.
    let moved_slot = temp.path().join("preview-moved");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":moved,"destination":moved_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    let light_record = |slot: &std::path::Path| -> Value {
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(slot.join("geometry.json")).unwrap()).unwrap();
        let lights = manifest["lights"].as_array().unwrap();
        assert_eq!(lights.len(), 1, "the scene declares one light: {manifest}");
        lights[0].clone()
    };
    // The light record carries the pin to its light node.
    assert_eq!(light_record(&base_slot)["entity_id"], json!(light_id));
    let base_matrix = matrix16(&light_record(&base_slot)["world_from_light"]);
    let moved_matrix = matrix16(&light_record(&moved_slot)["world_from_light"]);
    assert_eq!(light_record(&moved_slot)["entity_id"], json!(light_id));

    let retargeted = mul(&delta_start, &base_matrix);
    for (delta_applied, full_preview) in retargeted.iter().zip(&moved_matrix) {
        assert!(
            (delta_applied - full_preview).abs() < 1e-9,
            "delta · base world_from_light {delta_applied} \
             != moved world_from_light {full_preview}"
        );
    }
}

/// a moved shape that carries an area light stays
/// fast — the emitter the renderer rebinds from the moved draw is the
/// moved area light — and `delta · the base draw matrix` equals (within
/// 1e-9) the draw matrix of a real full preview at the new revision.
#[test]
fn preview_transforms_answers_area_light_shape_deltas_matching_full_previews() {
    fn mul(a: &[f64], b: &[f64]) -> [f64; 16] {
        let mut out = [0.; 16];
        for row in 0..4 {
            for column in 0..4 {
                out[row * 4 + column] = (0..4).map(|k| a[row * 4 + k] * b[k * 4 + column]).sum();
            }
        }
        out
    }
    fn matrix16(value: &Value) -> [f64; 16] {
        std::array::from_fn(|index| value[index].as_f64().unwrap())
    }

    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nAreaLightSource \"diffuse\" \"rgb L\" [4 4 4]\n\
         Translate 1 2 3\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\nWorldEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let base_revision = project["revision"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"translate"}),
    );
    assert_eq!(hits["total"], json!(1), "{hits}");
    let translate = hits["nodes"][0]["node_id"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"arealightsource"}),
    );
    assert_eq!(hits["total"], json!(1), "{hits}");
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"shape"}),
    );
    assert_eq!(hits["total"], json!(1), "{hits}");
    let shape_id = hits["nodes"][0]["node_id"].as_str().unwrap().to_owned();

    let base_slot = temp.path().join("preview-base");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":base_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    // A move only: one set_transform on the shape's Translate node.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"set_transform","node_id":translate,
            "value":{"kind":"translate","value":[4.,5.,6.]}}]}),
    );
    let moved = state(&core)["context"]["project"].clone();
    assert_eq!(moved["revision"], json!("1"));

    let answer = call(
        &core,
        Method::ProjectPreviewTransforms,
        json!({"project":moved,"base_revision":base_revision}),
    );
    assert_eq!(answer["fast"], json!(true), "{answer}");
    let deltas = answer["deltas"].as_object().unwrap();
    // The AreaLightSource node owns no draw to name; only the shape does.
    assert_eq!(deltas.len(), 1, "only the area-light shape moved: {answer}");
    let delta = &deltas[&shape_id];
    let delta_start = matrix16(&delta[0]);
    let delta_end = matrix16(&delta[1]);
    assert_eq!(delta_start, delta_end, "the move stays static: {answer}");

    // The referee: a real full preview at the moved revision.
    let moved_slot = temp.path().join("preview-moved");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":moved,"destination":moved_slot}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");

    let draw_of_area_light_shape = |slot: &std::path::Path| -> Value {
        let manifest: Value =
            serde_json::from_str(&fs::read_to_string(slot.join("geometry.json")).unwrap()).unwrap();
        let draw = manifest["draws"]
            .as_array()
            .unwrap()
            .iter()
            .find(|draw| draw["entity_id"].as_str() == Some(&shape_id))
            .expect("the area-light shape's draw is pinned by entity_id");
        assert!(
            draw["area_light"].is_object(),
            "the shape carries the area light"
        );
        draw.clone()
    };
    let base_matrix = matrix16(&draw_of_area_light_shape(&base_slot)["world_from_object"][0]);
    let moved_matrix = matrix16(&draw_of_area_light_shape(&moved_slot)["world_from_object"][0]);

    let retargeted = mul(&delta_start, &base_matrix);
    for (delta_applied, full_preview) in retargeted.iter().zip(&moved_matrix) {
        assert!(
            (delta_applied - full_preview).abs() < 1e-9,
            "delta · base draw {delta_applied} != moved draw {full_preview}"
        );
    }
}

/// The preview package's `diagnostics.jsonl` warnings — for example, a
/// corrupt PLY skipped with `geometry_failed` — count as unsupported in the
/// job result, and the warning names only the asset file, never the
/// temporary staging bundle it was read from.
#[test]
fn preview_unsupported_counts_the_package_warnings() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    fs::write(temp.path().join("broken.ply"), "not a ply at all").unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "LookAt 3 2 6 0 0 0 0 1 0\n\
         Camera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\n\
         Shape \"plymesh\" \"string filename\" \"broken.ply\"\n\
         Shape \"sphere\" \"float radius\" [1]\n\
         WorldEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let slot = temp.path().join("preview-slot");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":slot}),
    );
    let finished = wait(&core, job);
    assert_eq!(finished["state"], "succeeded", "{finished}");
    let result = &finished["result"];
    assert_eq!(result["draws"], json!(1));
    assert_eq!(result["unsupported"], json!(1), "{result}");
    let log = fs::read_to_string(slot.join("diagnostics.jsonl")).unwrap();
    assert!(log.contains("geometry_failed"), "{log}");
    assert!(log.contains("broken.ply"), "{log}");
    assert!(
        !log.contains("assets/") && !log.contains("bundle"),
        "the warning names the file, not its staging path: {log}"
    );
}

/// The in-place preview flavor — the desktop's own slot,
/// and capture/pick packages — writes exactly the geometry the staged,
/// self-contained bundle writes for external callers, and keeps no
/// assets copy of its own.
#[test]
fn in_place_preview_matches_the_staged_preview_bundle() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("tri.ply"),
        concat!(
            "ply\nformat ascii 1.0\nelement vertex 3\n",
            "property float x\nproperty float y\nproperty float z\n",
            "property float nx\nproperty float ny\nproperty float nz\n",
            "element face 1\nproperty list uchar int vertex_indices\nend_header\n",
            "0 0 0 0 0 1\n1 0 0 0 0 1\n0 1 0 0 0 1\n3 0 1 2\n",
        ),
    )
    .unwrap();
    fs::write(temp.path().join("map.dat"), b"map bytes").unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "LookAt 3 2 6 0 0 0 0 1 0\n",
            "Camera \"perspective\" \"float fov\" [45]\n",
            "Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n",
            "WorldBegin\n",
            "Texture \"env\" \"spectrum\" \"imagemap\" \"string filename\" \"map.dat\"\n",
            "LightSource \"infinite\" \"string mapname\" \"map.dat\"\n",
            "AttributeBegin\n",
            "Material \"diffuse\" \"texture reflectance\" [env]\n",
            "Shape \"plymesh\" \"string filename\" \"tri.ply\"\n",
            "AttributeEnd\n",
            "Shape \"sphere\" \"float radius\" [1]\n",
            "WorldEnd\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let staged = temp.path().join("slot-staged");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":staged}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");
    assert!(
        staged.join("assets").is_dir(),
        "the external bundle keeps its assets copy"
    );
    let in_place = temp.path().join("slot-in-place");
    let job = call(
        &core,
        Method::ProjectPreview,
        json!({"project":project,"destination":in_place,"stage_assets":false}),
    );
    let finished = wait(&core, job);
    assert_eq!(finished["state"], "succeeded", "{finished}");
    assert_eq!(finished["result"]["draws"], json!(2), "{finished}");
    assert!(
        !in_place.join("assets").exists(),
        "the in-place slot stages no assets copy"
    );
    // The texture and light records name their source file, and in-place
    // names the store (here, the just-imported payload lease) where the
    // staged flavor names the bundle copy — the only intended difference
    // in geometry.json.
    // Neutralize exactly those reference texts and compare the rest.
    let neutralize = |mut value: serde_json::Value| {
        for group in ["textures", "lights"] {
            for record in value[group].as_array_mut().into_iter().flatten() {
                for parameter in record["parameters"].as_array_mut().into_iter().flatten() {
                    if parameter["name"] == "filename" || parameter["name"] == "mapname" {
                        parameter["texts"] = json!(["<asset source>"]);
                    }
                }
            }
        }
        value
    };
    let staged_geometry = neutralize(
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(staged.join("geometry.json")).unwrap(),
        )
        .unwrap(),
    );
    let in_place_geometry = neutralize(
        serde_json::from_str::<serde_json::Value>(
            &fs::read_to_string(in_place.join("geometry.json")).unwrap(),
        )
        .unwrap(),
    );
    assert_eq!(
        staged_geometry, in_place_geometry,
        "geometry.json must match between the staged and in-place bundles"
    );
}

/// The second preview of unchanged content hits the
/// tessellation cache and opens no PLY at all — count rides
/// on the shared service cache's stats.
#[test]
fn second_in_place_preview_opens_no_ply() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("tri.ply"),
        concat!(
            "ply\nformat ascii 1.0\nelement vertex 3\n",
            "property float x\nproperty float y\nproperty float z\n",
            "property float nx\nproperty float ny\nproperty float nz\n",
            "element face 1\nproperty list uchar int vertex_indices\nend_header\n",
            "0 0 0 0 0 1\n1 0 0 0 0 1\n0 1 0 0 0 1\n3 0 1 2\n",
        ),
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "LookAt 3 2 6 0 0 0 0 1 0\n",
            "Camera \"perspective\" \"float fov\" [45]\n",
            "Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n",
            "WorldBegin\n",
            "Shape \"plymesh\" \"string filename\" \"tri.ply\"\n",
            "Shape \"sphere\" \"float radius\" [1]\n",
            "WorldEnd\n",
        ),
    )
    .unwrap();
    let geometry_cache: std::sync::Arc<std::sync::Mutex<scene_parse::GeometryCache>> =
        Default::default();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: None,
            renderer: None,
            directory: std::sync::Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: geometry_cache.clone(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    for index in 0..2 {
        let slot = temp.path().join(format!("slot{index}"));
        let job = call(
            &core,
            Method::ProjectPreview,
            json!({"project":project,"destination":slot,"stage_assets":false}),
        );
        let finished = wait(&core, job);
        assert_eq!(finished["state"], "succeeded", "{finished}");
        assert_eq!(finished["result"]["draws"], json!(2), "{finished}");
    }
    let stats = geometry_cache.lock().unwrap().stats();
    assert_eq!(
        stats.asset_reads, 1,
        "only the cold first preview opened the PLY: {stats:?}"
    );
    assert_eq!(stats.misses, 2, "both shapes tessellated cold: {stats:?}");
    assert_eq!(
        stats.hits, 2,
        "the second preview reused both tessellations: {stats:?}"
    );
}

/// Every Contents entry, walking the folders from the Assets root.
fn all_contents(core: &CoreHandle, project: &Value) -> Vec<Value> {
    let mut pending = vec![String::new()];
    let mut entries = Vec::new();
    while let Some(folder) = pending.pop() {
        let page = call(
            core,
            Method::ProjectListContents,
            json!({"project":project,"folder":folder,"limit":1000}),
        );
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else {
                entries.push(entry.clone());
            }
        }
    }
    entries
}

/// An asset whose file left the project folder (moved outside the
/// application) fails an export as a missing file, naming its location,
/// not as an unknown asset id.
#[test]
fn an_export_reports_an_asset_file_missing_from_the_project_folder() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    fs::write(temp.path().join("checker.png"), b"not decoded by import").unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTexture \"checker\" \"spectrum\" \"imagemap\" \"string filename\" \"checker.png\"\n\
         Material \"diffuse\" \"texture reflectance\" \"checker\"\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let asset = all_contents(&core, &project)
        .into_iter()
        .find(|entry| entry["kind"] == "asset")
        .unwrap();
    fs::rename(
        temp.path().join("project/Assets/Images/checker.png"),
        temp.path().join("moved.png"),
    )
    .unwrap();
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":project,"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "failed", "{job}");
    let error = &job["error"];
    assert_eq!(error["code"], "io_error", "{job}");
    assert_eq!(error["target_id"], asset["id"], "{job}");
    assert_eq!(error["details"]["reason"], "asset_file_missing", "{job}");
    assert_eq!(error["details"]["location"], "Images/checker.png", "{job}");
    assert!(
        error["message"]
            .as_str()
            .unwrap()
            .contains("Images/checker.png"),
        "{job}"
    );
}

/// project.get_parameter_values reads any window of a parameter's values,
/// inline or kept in a data asset (above 4096 values), in the parameter
/// preview's value forms.
#[test]
fn parameter_values_read_any_window_inline_or_from_a_data_asset() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    // 1700 points (5100 floats, a data asset) and 40 points (120, inline).
    let points = |count: usize| {
        (0..count * 3)
            .map(|i| format!("{}", i as f64 * 0.5))
            .collect::<Vec<_>>()
            .join(" ")
    };
    let indices = |count: usize| {
        (0..count - 2)
            .flat_map(|i| [0, i + 1, i + 2])
            .map(|i| i.to_string())
            .collect::<Vec<_>>()
            .join(" ")
    };
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        format!(
            "WorldBegin\nShape \"trianglemesh\" \"point3 P\" [{}] \"integer indices\" [{}]\n\
             Shape \"trianglemesh\" \"point3 P\" [{}] \"integer indices\" [{}]\n",
            points(1700),
            indices(1700),
            points(40),
            indices(40)
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"Shape"}),
    );
    // Told apart by their P counts; search order is not part of the test.
    let mut shapes = hits["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| json!({"kind":"node","id":node["node_id"]}))
        .collect::<Vec<_>>();
    shapes.sort_by_key(|target| {
        let page = call(
            &core,
            Method::ProjectGetParameters,
            json!({"project":project,"target":target}),
        );
        page["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "P")
            .unwrap()["value_count"]
            .as_str()
            .unwrap()
            .parse::<usize>()
            .unwrap()
    });
    let (small, large) = (shapes[0].clone(), shapes[1].clone());
    let values_at = |project: &Value, target: &Value, name: &str, offset: usize, limit: usize| {
        call(
            &core,
            Method::ProjectGetParameterValues,
            json!({"project":project,"target":target,"name":name,"offset":offset,"limit":limit}),
        )
    };
    let values = |target: &Value, name: &str, offset: usize, limit: usize| {
        values_at(&project, target, name, offset, limit)
    };
    let finite = |value: f64| json!({"kind":"finite","value":value});

    let page = values(&large, "P", 5000, 200);
    assert_eq!(page["value_type"], "point3");
    assert_eq!(page["total"], "5100");
    assert_eq!(page["values"]["kind"], "floats");
    let window = page["values"]["value"].as_array().unwrap();
    assert_eq!(window.len(), 100, "the window stops at the end");
    assert_eq!(window[0], finite(2500.0));
    assert_eq!(window[99], finite(2549.5));
    assert_eq!(page["next_offset"], Value::Null);
    let first = values(&large, "P", 0, 3);
    assert_eq!(
        first["values"]["value"],
        json!([finite(0.0), finite(0.5), finite(1.0)])
    );
    assert_eq!(first["next_offset"], 3);
    let tail = values(&large, "indices", 5091, 10);
    assert_eq!(tail["total"], "5094");
    assert_eq!(tail["values"]["kind"], "integers");
    assert_eq!(tail["values"]["value"], json!(["0", "1698", "1699"]));

    let inline = values(&small, "P", 30, 10);
    assert_eq!(inline["total"], "120");
    assert_eq!(inline["values"]["value"][0], finite(15.0));
    assert_eq!(inline["next_offset"], 40);

    let apply = |project: &Value, edits: Value| {
        call(
            &core,
            Method::ProjectApply,
            json!({"project":project,"edits":edits}),
        )
    };
    let edited = apply(
        &project,
        json!([{"kind":"set_parameter_values","target":small,"name":"P","offset":30,
            "values":{"kind":"floats","value":[99.0,98.0,97.0]}}]),
    );
    assert_eq!(edited["project"]["revision"], "1");
    let after = values_at(&edited["project"], &small, "P", 30, 3);
    assert_eq!(
        after["values"]["value"],
        json!([finite(99.0), finite(98.0), finite(97.0)])
    );
    let undone = call(
        &core,
        Method::ProjectUndo,
        json!({"project":edited["project"]}),
    );
    assert_eq!(
        values_at(&undone["project"], &small, "P", 30, 3)["values"]["value"],
        json!([finite(15.0), finite(15.5), finite(16.0)])
    );
    let redone = call(
        &core,
        Method::ProjectRedo,
        json!({"project":undone["project"]}),
    );
    assert_eq!(
        values_at(&redone["project"], &small, "P", 30, 3)["values"]["value"],
        json!([finite(99.0), finite(98.0), finite(97.0)])
    );

    // An inline window outside the array, or of another kind, is refused
    // when the job is asked for, not later as a failed job.
    for windows in [
        json!([{"offset":119,"values":{"kind":"floats","value":[1.0,2.0]}}]),
        json!([{"offset":0,"values":{"kind":"integers","value":["1"]}}]),
    ] {
        let refused = core.request(request(
            Method::ProjectSetParameterValues,
            json!({"project":redone["project"],"target":small,"name":"P","windows":windows}),
        ));
        assert_eq!(
            refused.error.expect("refused at submission").code,
            ErrorCode::InvalidRequest,
            "{windows}"
        );
    }

    let refuse = |params: Value| {
        core.request(request(Method::ProjectGetParameterValues, params))
            .error
            .unwrap()
            .code
    };
    assert_eq!(
        refuse(json!({"project":redone["project"],"target":small,"name":"P","offset":121})),
        ErrorCode::InvalidRequest
    );
    assert_eq!(
        refuse(json!({"project":redone["project"],"target":small,"name":"N"})),
        ErrorCode::NotFound
    );
}

#[test]
fn editing_a_data_asset_window_replaces_bytes_and_undo_redo_restores_identity() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let values = (0..5000)
        .map(|n| (n as f64).to_string())
        .collect::<Vec<_>>()
        .join(" ");
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        format!(
            "WorldBegin\nShape \"trianglemesh\" \"float alpha\" [{}]\n",
            values
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let nodes = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"Shape"}),
    );
    let target = json!({"kind":"node","id":nodes["nodes"][0]["node_id"]});
    let parameter = |project: &Value| {
        let page = call(
            &core,
            Method::ProjectGetParameters,
            json!({"project":project,"target":target}),
        );
        page["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "alpha")
            .unwrap()
            .clone()
    };
    let original = parameter(&project);
    assert_eq!(original["values"]["kind"], "array");
    let original_asset = original["values"]["asset"].clone();
    // A data asset is rewritten on the I/O executor: a job, never apply.
    let refused = core.request(request(
        Method::ProjectApply,
        json!({"project":project,
            "edits":[{"kind":"set_parameter_values","target":target,"name":"alpha","offset":2048,
                "values":{"kind":"floats","value":[777.0]}}]}),
    ));
    assert_eq!(
        refused.error.unwrap().details.unwrap()["reason"],
        "data_asset_edit_is_a_job"
    );
    let set_values = |project: &Value, windows: Value| {
        let job = call(
            &core,
            Method::ProjectSetParameterValues,
            json!({"project":project,"target":target,"name":"alpha","windows":windows}),
        );
        let job = wait(&core, job);
        assert_eq!(job["state"], "succeeded", "{job}");
        job["result"].clone()
    };
    let overlapping = core.request(request(
        Method::ProjectSetParameterValues,
        json!({"project":project,"target":target,"name":"alpha","windows":[
            {"offset":10,"values":{"kind":"floats","value":[1.0,2.0]}},
            {"offset":11,"values":{"kind":"floats","value":[3.0]}}]}),
    ));
    assert_eq!(overlapping.error.unwrap().code, ErrorCode::InvalidRequest);
    // A retried request (same id) is the same job, before and after it
    // finishes; the same id with other windows is a conflict.
    let retried = request(
        Method::ProjectSetParameterValues,
        json!({"project":project,"target":target,"name":"alpha",
               "windows":[{"offset":3,"values":{"kind":"floats","value":[3.25]}}]}),
    );
    let accepted = core.request(retried.clone());
    assert!(accepted.error.is_none(), "{:?}", accepted.error);
    assert_eq!(
        core.request(retried.clone()),
        accepted,
        "replayed before it finishes"
    );
    let job = wait(&core, accepted.result.clone().unwrap());
    assert_eq!(job["state"], "succeeded", "{job}");
    assert_eq!(
        core.request(retried.clone()),
        accepted,
        "replayed after it finishes"
    );
    let mut reused = retried.clone();
    reused.params["windows"][0]["offset"] = json!(4);
    assert_eq!(
        core.request(reused).error.unwrap().code,
        ErrorCode::RequestIdConflict
    );
    call(
        &core,
        Method::ProjectUndo,
        json!({"project":state(&core)["context"]["project"]}),
    );
    let project = state(&core)["context"]["project"].clone();

    // Two separate windows land in one transaction.
    let first = set_values(
        &project,
        json!([{"offset":2048,"values":{"kind":"floats","value":[777.0,778.0]}},
               {"offset":10,"values":{"kind":"floats","value":[1.5]}}]),
    );
    assert_eq!(first["created"].as_array().unwrap().len(), 1, "{first}");
    let edited_ref = state(&core)["context"]["project"].clone();
    assert_eq!(first["revision"], edited_ref["revision"]);
    let edited = parameter(&edited_ref);
    let edited_asset = edited["values"]["asset"].clone();
    assert_ne!(edited_asset, original_asset, "replacement is copy-on-write");
    let read = |project: &Value, offset: usize, limit: usize| {
        call(
            &core,
            Method::ProjectGetParameterValues,
            json!({
                "project":project,"target":target,"name":"alpha","offset":offset,"limit":limit
            }),
        )
    };
    assert_eq!(
        read(&edited_ref, 2048, 2)["values"]["value"],
        json!([{"kind":"finite","value":777.0},{"kind":"finite","value":778.0}])
    );
    assert_eq!(
        read(&edited_ref, 9, 3)["values"]["value"],
        json!([{"kind":"finite","value":9.0},{"kind":"finite","value":1.5},
               {"kind":"finite","value":11.0}])
    );
    set_values(
        &edited_ref,
        json!([{"offset":2048,"values":{"kind":"floats","value":[888.0,889.0]}}]),
    );
    let second_ref = state(&core)["context"]["project"].clone();
    assert_eq!(
        read(&second_ref, 2048, 1)["values"]["value"][0]["value"],
        888.0
    );
    let restored = call(&core, Method::ProjectUndo, json!({"project":second_ref}));
    let edited_ref = restored["project"].clone();
    let save = call(&core, Method::ProjectSave, json!({"project":edited_ref}));
    assert_eq!(wait(&core, save)["state"], "succeeded");
    let saved_ref = state(&core)["context"]["project"].clone();
    let undone = call(&core, Method::ProjectUndo, json!({"project":saved_ref}));
    assert_eq!(
        parameter(&undone["project"])["values"]["asset"],
        original_asset
    );
    assert_eq!(
        read(&undone["project"], 2048, 2)["values"]["value"],
        json!([{"kind":"finite","value":2048.0},{"kind":"finite","value":2049.0}])
    );
    let redone = call(
        &core,
        Method::ProjectRedo,
        json!({"project":undone["project"]}),
    );
    assert_eq!(
        parameter(&redone["project"])["values"]["asset"],
        edited_asset
    );
    assert_eq!(
        read(&redone["project"], 2048, 2)["values"]["value"],
        json!([{"kind":"finite","value":777.0},{"kind":"finite","value":778.0}])
    );

    // The data file gone from the project folder fails the job as I/O.
    let asset = call(
        &core,
        Method::ProjectGetAsset,
        json!({"project":redone["project"],"asset_id":edited_asset}),
    );
    fs::remove_file(asset["path"].as_str().unwrap()).unwrap();
    let job = call(
        &core,
        Method::ProjectSetParameterValues,
        json!({"project":redone["project"],"target":target,"name":"alpha",
               "windows":[{"offset":0,"values":{"kind":"floats","value":[5.0]}}]}),
    );
    let job = wait(&core, job);
    assert_eq!(job["state"], "failed", "{job}");
    assert_eq!(job["error"]["code"], "io_error", "{job}");
    // Named like any missing asset file, not as a bare OS error.
    assert_eq!(
        job["error"]["details"]["reason"], "asset_file_missing",
        "{job}"
    );
    assert_eq!(job["error"]["target_id"], edited_asset, "{job}");
    let location = asset["location"].as_str().unwrap();
    assert!(
        job["error"]["message"].as_str().unwrap().contains(location),
        "{job}"
    );
}

#[test]
fn project_get_asset_reports_file_facts_and_users() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let bytes = b"not decoded by import";
    fs::write(temp.path().join("checker.png"), bytes).unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTexture \"checker\" \"spectrum\" \"imagemap\" \"string filename\" \"checker.png\"\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let asset = all_contents(&core, &project)
        .into_iter()
        .find(|entry| entry["kind"] == "asset")
        .unwrap();
    assert_eq!(asset["location"], "Images/checker.png");
    let sidecar: Value = serde_json::from_slice(
        &fs::read(
            temp.path()
                .join("project/Assets")
                .join(format!("{}.meta", asset["location"].as_str().unwrap())),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(sidecar["id"], asset["id"]);
    let details = call(
        &core,
        Method::ProjectGetAsset,
        json!({"project":project,"asset_id":sidecar["id"]}),
    );
    assert_eq!(details["location"], asset["location"]);
    assert_eq!(details["name"], "checker.png");
    assert_eq!(details["size"], bytes.len().to_string());
    assert_eq!(details["sha256"].as_str().unwrap().len(), 64, "{details}");
    assert_eq!(details["reference_count"], 1, "{details}");
    let path = std::path::PathBuf::from(details["path"].as_str().unwrap());
    assert_eq!(std::fs::read(&path).unwrap(), bytes, "{details}");
    let missing = core.request(request(
        Method::ProjectGetAsset,
        json!({"project":project,"asset_id":uuid::Uuid::new_v4()}),
    ));
    assert_eq!(missing.error.unwrap().code, ErrorCode::NotFound);
}

#[test]
fn set_instance_definition_repoints_an_imported_instance() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nObjectBegin \"a\"\nShape \"sphere\"\nObjectEnd\nObjectBegin \"b\"\nShape \"disk\"\nObjectEnd\nObjectInstance \"a\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let definitions = all_contents(&core, &project)
        .into_iter()
        .filter(|entry| entry["resource_kind"] == "object_definition")
        .collect::<Vec<_>>();
    let b = definitions
        .iter()
        .find(|entry| entry["name"] == "b")
        .unwrap();
    let root = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":project,"view":"scene_graph"}),
    )["nodes"][0]["node_id"]
        .clone();
    let objects = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":project,"parent":root,"view":"scene_graph"}),
    );
    let instance = objects["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["object"] == "instance")
        .unwrap()["node_id"]
        .clone();
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"set_instance_definition","node_id":instance,"definition":b["id"]}]}),
    );
    let project = state(&core)["context"]["project"].clone();
    let node = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":project,"node_id":instance}),
    );
    assert_eq!(node["data"]["definition"], b["id"], "{node}");
    let component = node["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == "object_instance")
        .unwrap();
    assert_eq!(component["definition"], b["id"]);
}

#[test]
fn node_edits_duplicate_move_and_delete_scene_graph_objects() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nAttributeBegin\nShape \"sphere\"\nAttributeEnd\nAttributeBegin\nShape \"disk\"\nShape \"cylinder\"\nAttributeEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let objects = |core: &CoreHandle, parent: &Value| {
        let project = state(core)["context"]["project"].clone();
        call(
            core,
            Method::ProjectListNodes,
            json!({"project":project,"parent":parent,"view":"scene_graph"}),
        )["nodes"]
            .as_array()
            .unwrap()
            .clone()
    };
    let root = {
        let project = state(&core)["context"]["project"].clone();
        call(
            &core,
            Method::ProjectListNodes,
            json!({"project":project,"view":"scene_graph"}),
        )["nodes"][0]["node_id"]
            .clone()
    };
    let groups = objects(&core, &root);
    assert_eq!(groups.len(), 2);
    let sphere = objects(&core, &groups[0]["node_id"])[0]["node_id"].clone();
    let apply = |core: &CoreHandle, edit: Value| {
        let project = state(core)["context"]["project"].clone();
        call(
            core,
            Method::ProjectApply,
            json!({"project":project,"edits":[edit]}),
        )
    };
    // Imported groups have no ownership metadata: duplicate only the sphere,
    // even when it was the group's only object.
    let duplicated = apply(&core, json!({"kind":"duplicate_node","node_id":sphere}));
    let created = duplicated["created"].as_array().unwrap();
    assert_eq!(created.len(), 1, "{duplicated}");
    let root_objects = objects(&core, &root);
    assert_eq!(root_objects.len(), 2);
    let siblings = objects(&core, &groups[0]["node_id"]);
    assert_eq!(siblings.len(), 2);
    assert_eq!(siblings[1]["node_id"], created[0]["id"]);
    // The disk shares its group with the cylinder: it moves alone.
    let disk = objects(&core, &groups[1]["node_id"])[0]["node_id"].clone();
    apply(
        &core,
        json!({"kind":"move_node","node_id":disk,"parent":root,"after":null}),
    );
    assert_eq!(objects(&core, &root).last().unwrap()["node_id"], disk);
    apply(&core, json!({"kind":"delete_node","node_id":disk}));
    assert_eq!(objects(&core, &root).len(), 2);
    let refused = core.request(request(
        Method::ProjectApply,
        json!({"project":state(&core)["context"]["project"],"edits":[{"kind":"delete_node","node_id":root}]}),
    ));
    assert_eq!(refused.error.unwrap().code, ErrorCode::InvalidRequest);
}

#[test]
fn project_cache_round_trips_through_the_wire() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = state(&core)["context"]["project"].clone();
    let item = "6868d5ba-99e5-4b94-9c38-64d7ab1ba18c";
    // put_cache (base64 of "png bytes"): core owns the write, under Library/Cache.
    let put = call(
        &core,
        Method::ProjectPutCache,
        json!({"project":reference,"item":item,"key":"sha256-key-1","kind":"thumbnail",
               "media_type":"image/png","data_base64":"cG5nIGJ5dGVz"}),
    );
    let path = put["path"].as_str().unwrap().to_owned();
    assert!(
        path.replace('\\', "/")
            .ends_with(&format!("Library/Cache/Thumbnails/{item}/sha256-key-1")),
        "{path}"
    );
    assert_eq!(fs::read(&path).unwrap(), b"png bytes");
    // get_cache hits with the path and the media type, misses other keys.
    let hit = call(
        &core,
        Method::ProjectGetCache,
        json!({"project":reference,"item":item,"key":"sha256-key-1"}),
    );
    assert_eq!(hit["hit"], true);
    assert_eq!(hit["path"], path.as_str());
    assert_eq!(hit["media_type"], "image/png");
    let miss = call(
        &core,
        Method::ProjectGetCache,
        json!({"project":reference,"item":item,"key":"sha256-key-2"}),
    );
    assert_eq!(miss["hit"], false);
    // Keys stay file names: no traversal, no separators.
    for key in ["../escape", "a/b", ""] {
        let error = core.request(request(
            Method::ProjectPutCache,
            json!({"project":reference,"item":item,"key":key,"kind":"thumbnail",
                   "media_type":"image/png","data_base64":"cG5nIGJ5dGVz"}),
        ));
        assert_eq!(
            error.error.unwrap().code,
            ErrorCode::InvalidRequest,
            "{key}"
        );
    }
    // Viewer staging is a cache writer too: the
    // staged folder store serves the cache, and it dies with the staging.
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import_viewer(&core, &source);
    let reference = state(&core)["context"]["project"].clone();
    let put = call(
        &core,
        Method::ProjectPutCache,
        json!({"project":reference,"item":item,"key":"sha256-key-1","kind":"preview",
               "media_type":"image/png","data_base64":"cG5nIGJ5dGVz"}),
    );
    assert!(
        put["path"]
            .as_str()
            .unwrap()
            .replace('\\', "/")
            .ends_with(&format!("Library/Cache/Previews/{item}/sha256-key-1")),
        "{put}"
    );
    let hit = call(
        &core,
        Method::ProjectGetCache,
        json!({"project":reference,"item":item,"key":"sha256-key-1"}),
    );
    assert_eq!(hit["hit"], true);
    assert_eq!(hit["media_type"], "image/png");
}

#[test]
fn parameter_schema_is_served_without_an_open_project() {
    // The schema is static information: no project at all.
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let call_schema = |directive: &str, implementation: &str, texture_value_type: Value| {
        call(
            &core,
            Method::ProjectGetParameterSchema,
            json!({"directive":directive,"implementation":implementation,
                   "texture_value_type":texture_value_type}),
        )
    };
    let reply = call_schema("Shape", "sphere", Value::Null);
    assert_eq!(reply["directive"], "Shape");
    assert_eq!(reply["implementation"], "sphere");
    assert!(reply["texture_value_type"].is_null());
    let named = |reply: &Value, name: &str| {
        reply["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {reply}"))
            .clone()
    };
    let radius = named(&reply, "radius");
    assert_eq!(
        radius["default"],
        json!({"value_type":"float","values":[1.0]})
    );
    assert_eq!(radius["required"], false);
    let alpha = named(&reply, "alpha");
    assert_eq!(alpha["texture"], json!({"kind":"float"}));
    assert_eq!(alpha["role"], "texture_input");
    let zmin = named(&reply, "zmin");
    assert!(zmin["default"].is_null());
    assert_eq!(
        zmin["default_depends"],
        json!({"parameter":"radius","operation":"negated"})
    );
    let implementations: Vec<&str> = reply["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|name| name.as_str())
        .collect();
    assert!(implementations.contains(&"trianglemesh"));
    // plymesh's required filename is a file role.
    let plymesh = call_schema("Shape", "plymesh", Value::Null);
    assert_eq!(named(&plymesh, "filename")["role"], "file");
    assert_eq!(named(&plymesh, "filename")["required"], true);

    // Texture tables are per value type (doc): mix defaults differ and
    // marble has no float table, but the implementation list still shows it.
    let mix_float = call_schema("Texture", "mix", json!("float"));
    let mix_spectrum = call_schema("Texture", "mix", json!("spectrum"));
    assert_eq!(named(&mix_float, "tex1")["default"]["values"], json!([0.0]));
    assert_eq!(
        named(&mix_spectrum, "tex1")["default"]["values"],
        // A spectrum scalar renders as an rgb triple.
        json!([1.0, 1.0, 1.0])
    );
    assert!(
        mix_float["implementations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| *name == json!("marble"))
    );
    let marble_float = call_schema("Texture", "marble", json!("float"));
    assert!(marble_float["parameters"].is_null());
    let marble_spectrum = call_schema("Texture", "marble", json!("spectrum"));
    assert!(marble_spectrum["parameters"].as_array().is_some());

    // The r4 aliases of the area light resolve to the canonical name.
    let area = call_schema("AreaLightSource", "", Value::Null);
    assert_eq!(area["implementation"], "diffuse");
    assert_eq!(
        named(&area, "twosided")["default"],
        json!({"value_type":"bool","values":[false]})
    );

    // Implementations without a table stay legal to use: parameters null.
    let cone = call_schema("Shape", "cone", Value::Null);
    assert!(cone["parameters"].is_null());
    assert!(
        !cone["implementations"]
            .as_array()
            .unwrap()
            .iter()
            .any(|name| *name == json!("cone"))
    );

    // 's options-block tables (r4 transcription, first edition): the
    // rgb film's rows carry r4's constants, and its output name stays a
    // plain string the user types.
    let film = call_schema("Film", "rgb", Value::Null);
    assert_eq!(
        named(&film, "filename")["default"],
        json!({"value_type":"string","values":["pbrt.exr"]})
    );
    assert_eq!(named(&film, "filename")["role"], "value");
    assert_eq!(
        named(&film, "xresolution")["default"],
        json!({"value_type":"integer","values":[1280]})
    );
    assert_eq!(
        named(&film, "yresolution")["default"],
        json!({"value_type":"integer","values":[720]})
    );
    assert!(named(&film, "pixelbounds")["default"].is_null());
    assert!(named(&film, "cropwindow")["default"].is_null());
    assert_eq!(named(&film, "sensor")["choices"][0], json!("cie1931"));
    let list = |reply: &Value| -> Vec<String> {
        reply["implementations"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|name| name.as_str())
            .map(str::to_owned)
            .collect()
    };
    // Only the table's implementations are offered.
    assert_eq!(list(&film), ["rgb"]);
    // The camera's shared rows plus perspective's fov, with r4's defaults.
    let camera = call_schema("Camera", "perspective", Value::Null);
    assert_eq!(
        named(&camera, "fov")["default"],
        json!({"value_type":"float","values":[90.0]})
    );
    assert_eq!(
        named(&camera, "focaldistance")["default"],
        json!({"value_type":"float","values":[1e6]})
    );
    // r4 derives frameaspectratio from the film and screenwindow from it.
    assert!(named(&camera, "frameaspectratio")["default"].is_null());
    assert_eq!(named(&camera, "frameaspectratio")["default_kind"], "none");
    assert!(named(&camera, "screenwindow")["default"].is_null());
    assert_eq!(list(&camera), ["perspective", "orthographic"]);
    let orthographic = call_schema("Camera", "orthographic", Value::Null);
    assert!(
        orthographic["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .all(|row| row["name"] != json!("fov"))
    );

    // The sampler's randomization is a choice; its seed follows the
    // render's global seed, so no constant default ships.
    let sampler = call_schema("Sampler", "zsobol", Value::Null);
    assert_eq!(
        named(&sampler, "pixelsamples")["default"],
        json!({"value_type":"integer","values":[16]})
    );
    assert_eq!(
        named(&sampler, "randomization")["default"],
        json!({"value_type":"string","values":["fastowen"]})
    );
    assert_eq!(
        named(&sampler, "randomization")["choices"]
            .as_array()
            .unwrap()
            .len(),
        4
    );
    assert!(named(&sampler, "seed")["default"].is_null());

    // The integrator's lightsampler alias resolves to one row with r4's
    // strategies as choices.
    let integrator = call_schema("Integrator", "volpath", Value::Null);
    assert_eq!(
        named(&integrator, "maxdepth")["default"],
        json!({"value_type":"integer","values":[5]})
    );
    assert_eq!(
        named(&integrator, "lightsamplestrategy")["aliases"],
        json!(["lightsampler"])
    );
    assert_eq!(
        named(&integrator, "lightsamplestrategy")["choices"],
        json!(["uniform", "power", "exhaustive", "bvh", "spatial"])
    );

    // The pixel filter and the accelerator, with r4's divergent bvh
    // default (hlbvh, where pbrt-v4 reads sah).
    let filter = call_schema("PixelFilter", "gaussian", Value::Null);
    assert_eq!(
        named(&filter, "sigma")["default"],
        json!({"value_type":"float","values":[0.5]})
    );
    let accelerator = call_schema("Accelerator", "bvh", Value::Null);
    assert_eq!(
        named(&accelerator, "splitmethod")["default"],
        json!({"value_type":"string","values":["hlbvh"]})
    );
    assert_eq!(list(&accelerator), ["bvh", "kdtree"]);

    // Implementations outside the new tables stay legal to use with no
    // rows (gbuffer and realistic are a later edition's).
    let gbuffer = call_schema("Film", "gbuffer", Value::Null);
    assert!(gbuffer["parameters"].is_null());
    assert_eq!(gbuffer["implementation"], "gbuffer");
    let realistic = call_schema("Camera", "realistic", Value::Null);
    assert!(realistic["parameters"].is_null());

    // Unknown directives, foreign texture types and unbounded names are out.
    let error = core.request(request(
        Method::ProjectGetParameterSchema,
        json!({"directive":"Nonsense","implementation":"x"}),
    ));
    assert_eq!(error.error.unwrap().code, ErrorCode::InvalidRequest);
    let error = core.request(request(
        Method::ProjectGetParameterSchema,
        json!({"directive":"Shape","implementation":"sphere","texture_value_type":"float"}),
    ));
    assert_eq!(error.error.unwrap().code, ErrorCode::InvalidRequest);
    let error = core.request(request(
        Method::ProjectGetParameterSchema,
        json!({"directive":"Shape","implementation":""}),
    ));
    assert_eq!(error.error.unwrap().code, ErrorCode::InvalidRequest);
    let error = core.request(request(
        Method::ProjectGetParameterSchema,
        json!({"directive":"Shape","implementation":"0".repeat(129)}),
    ));
    assert_eq!(error.error.unwrap().code, ErrorCode::InvalidRequest);
    drop(host);
}

#[test]
fn create_resource_fills_the_table_defaults_and_reports_the_id() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    // Fresh per call: every apply bumps the revision.
    let reference = || state(&core)["context"]["project"].clone();
    // A: omitted parameters fill every constant default in the schema table
    // that the dependency rules can export, passing the apply-time check.
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"material",
             "implementation":"diffuse","label":"Copper"}]}),
    );
    assert_eq!(applied["created"][0]["kind"], "resource");
    let copper = applied["created"][0]["id"].as_str().unwrap().to_owned();
    let resource = call(
        &core,
        Method::ProjectGetResource,
        json!({"project":reference(),"resource_id":copper}),
    );
    assert_eq!(resource["label"], "Copper");
    // Placement went through the shared rules.
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Materials"}),
    );
    let entry = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == copper)
        .expect("the created material shows in its folder");
    assert_eq!(entry["location"], "Materials/Copper.material");
    // The declared default is the table's.
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":{"kind":"resource","id":copper}}),
    );
    let reflectance = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "reflectance")
        .expect("the table's default is declared");
    assert_eq!(reflectance["values"]["kind"], "floats");
    let reflectance_value = reflectance["values"]["value"].as_array().unwrap();
    assert_eq!(reflectance_value.len(), 3);
    assert!(
        reflectance_value
            .iter()
            .all(|item| item["value"] == json!(0.5)),
        "{reflectance_value:?}"
    );
    // Plain strings the export whitelist accepts (mapping, aamode) come in;
    // an empty one (the medium preset) means "absent" and stays away.
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"texture",
             "implementation":"checkerboard","label":"Checker",
             "texture_value_type":"float"}]}),
    );
    let checker = applied["created"][0]["id"].as_str().unwrap().to_owned();
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":{"kind":"resource","id":checker}}),
    );
    let names: Vec<&str> = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert!(names.contains(&"aamode"), "{names:?}");
    assert!(names.contains(&"mapping"), "{names:?}");
    assert!(names.contains(&"dimension"), "{names:?}");
    // A texture without a value type never applies.
    let error = core.request(request(
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"texture",
             "implementation":"constant","label":"No type"}]}),
    ));
    assert!(error.error.is_some());
    // Object definitions come with their empty scope, and undo removes
    // them; redo reports the creation again.
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"object_definition",
             "implementation":"","label":"Bush"}]}),
    );
    // The scope node creation is reported alongside the resource.
    let bush = applied["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "resource")
        .expect("creation reports the resource")["id"]
        .as_str()
        .unwrap()
        .to_owned();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Objects"}),
    );
    let entry = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == bush)
        .expect("the object definition shows in its folder");
    assert_eq!(entry["location"], "Objects/Bush.object");
    let undo = call(&core, Method::ProjectUndo, json!({"project":reference()}));
    assert_eq!(undo["resources"], json!([bush]));
    let missing = core.request(request(
        Method::ProjectGetResource,
        json!({"project":reference(),"resource_id":bush}),
    ));
    assert_eq!(missing.error.unwrap().code, ErrorCode::NotFound);
    // Redo brings it back under the same id (the undo/redo response lists
    // touched resources; the re-report of created items lives on apply).
    let redo = call(&core, Method::ProjectRedo, json!({"project":reference()}));
    assert_eq!(redo["resources"], json!([bush]));
    let again = call(
        &core,
        Method::ProjectGetResource,
        json!({"project":reference(),"resource_id":bush}),
    );
    assert_eq!(again["label"], "Bush");
    // Created items stay exportable even though nothing references them.
    // The omitted defaults went through the apply-time
    // check, a real export must also pass with them written out.
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    // Unreferenced resources are still exported, in the bundle's
    // scene: the material with the table's declared default, the texture,
    // and the object definition scope.
    let scene = std::fs::read_to_string(temp.path().join("bundle/scene.pbrt"))
        .expect("an exported scene file");
    assert!(
        scene.contains(
            "MakeNamedMaterial \"Copper\" \"string type\" [\"diffuse\"] \"rgb reflectance\""
        ),
        "{scene}"
    );
    assert!(scene.contains("[ 0.5 0.5 0.5 ]"), "{scene}");
    assert!(scene.contains("ObjectBegin \"Bush\""), "{scene}");
    assert!(scene.contains("ObjectEnd"), "{scene}");
    drop(host);
}

#[test]
fn create_resource_fills_spectrum_data_defaults_and_refuses_implementation_change() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"spectrum_data",
             "implementation":"piecewise","label":"Gold k"}]}),
    );
    assert_eq!(applied["created"][0]["kind"], "resource");
    let gold = applied["created"][0]["id"].as_str().unwrap().to_owned();
    let resource = call(
        &core,
        Method::ProjectGetResource,
        json!({"project":reference(),"resource_id":gold}),
    );
    assert_eq!(resource["label"], "Gold k");
    // Placement uses the folder and extension rules for the resource kind.
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Spectra"}),
    );
    let entry = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["id"] == gold)
        .expect("the created spectrum data shows in its folder");
    assert_eq!(entry["location"], "Spectra/Gold k.spectrum");
    // Omitted parameters are the flat unit curve: neutral for previews.
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":{"kind":"resource","id":gold}}),
    );
    let find = |name: &str| {
        parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
            .unwrap_or_else(|| panic!("missing parameter {name}: {parameters}"))
            .clone()
    };
    assert_eq!(
        find("wavelengths")["values"]["value"],
        json!([{"kind":"finite","value":400.},{"kind":"finite","value":700.}])
    );
    assert_eq!(
        find("values")["values"]["value"],
        json!([{"kind":"finite","value":1.},{"kind":"finite","value":1.}])
    );
    // A spectrum curve has no implementation table to change.
    let error = core.request(request(
        Method::ProjectPlanImplementation,
        json!({"project":reference(),"target":{"kind":"resource","id":gold},
               "implementation":"x"}),
    ));
    let payload = error.error.expect("spectrum data has no implementation");
    assert!(
        payload
            .message
            .contains("spectrum data has no implementation"),
        "{payload:?}"
    );
    drop(host);
}

// ----------------------------------------------------------------- // project.spectrum_color: one parameter's spectrum-ish value resolved to
// the swatch rgb through the same `spectrum` crate Lit resolves with,
// plus the human `source` string a tooltip shows.

fn spectrum_fixture(name: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/spectra")
        .join(name)
}

/// One Contents entry by name, as the wire returns it.
fn list_entry(core: &CoreHandle, reference: &dyn Fn() -> Value, folder: &str, name: &str) -> Value {
    let contents = call(
        core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":folder}),
    );
    contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == name)
        .unwrap_or_else(|| panic!("missing {name} in {folder}: {contents}"))
        .clone()
}

/// The λ/v floats of an spd file, exactly as the imported resource holds
/// them.
fn spd_pairs(path: &std::path::Path) -> Vec<f64> {
    fs::read_to_string(path)
        .unwrap()
        .split_whitespace()
        .map(|token| token.parse::<f64>().unwrap())
        .collect()
}

/// The r4 named-metal F0 tables (scene-view's generated
/// `named_metals.rs`), the reference the conductor role must land
/// within (2e-3, the 10 nm integration against the tables' 1 nm one).
#[rustfmt::skip]
const AU_F0: [f64; 3] = [1.02188775, 0.782443963, 0.34342196];
#[rustfmt::skip]
const AU_F0_IN_COAT: [f64; 3] = [1.0098443, 0.734506672, 0.234874706];

/// One conductor-role answer, with the fixtures staged.
fn conductor_answer(
    core: &CoreHandle,
    reference: &dyn Fn() -> Value,
    target: &Value,
    parameter: &str,
) -> Value {
    call(
        core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":target,
               "parameter":parameter,"role":"conductor"}),
    )
}

#[test]
fn spectrum_color_conductor_role_answers_the_metal_colour() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let temp = tempfile::tempdir().unwrap();
    for name in ["Au.eta.spd", "Au.k.spd"] {
        fs::copy(spectrum_fixture(name), temp.path().join(name)).unwrap();
    }
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"Au.eta.spd\"] \"spectrum k\" [\"Au.k.spd\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let gold = list_entry(&core, &reference, "Materials", "gold");
    let target = json!({"kind":"resource","id":gold["id"]});
    // However the row asks — eta, k or reflectance — the conductor role
    // answers the metal's normal-incidence F0, within the tables' grid.
    for parameter in ["eta", "k", "reflectance"] {
        let answer = conductor_answer(&core, &reference, &target, parameter);
        for (got, want) in answer["rgb"].as_array().unwrap().iter().zip(AU_F0) {
            assert!(
                (got.as_f64().unwrap() - want).abs() < 2e-3,
                "{parameter}: {answer}"
            );
        }
        assert_eq!(answer["role"], "conductor");
    }
    // The source spells both sides, the curve resources by their labels.
    let answer = conductor_answer(&core, &reference, &target, "eta");
    assert_eq!(
        answer["source"],
        "eta: SpectrumData: Au.eta · k: SpectrumData: Au.k"
    );

    // A coated conductor under the default interface index (1.5): the
    // coat table's lower F0, from either prefixed parameter.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"coated\" \"string type\" [\"coatedconductor\"] ",
            "\"spectrum conductor.eta\" [\"metal-Au-eta\"] ",
            "\"spectrum conductor.k\" [\"metal-Au-k\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project2"));
    let reference = || state(&core)["context"]["project"].clone();
    let coated = list_entry(&core, &reference, "Materials", "coated");
    let target = json!({"kind":"resource","id":coated["id"]});
    for parameter in ["conductor.eta", "conductor.k", "reflectance"] {
        let answer = conductor_answer(&core, &reference, &target, parameter);
        for (got, want) in answer["rgb"].as_array().unwrap().iter().zip(AU_F0_IN_COAT) {
            assert!(
                (got.as_f64().unwrap() - want).abs() < 2e-3,
                "{parameter}: {answer}"
            );
        }
        assert_eq!(
            answer["source"],
            "conductor.eta: metal-Au-eta · conductor.k: metal-Au-k"
        );
    }

    // A spectral interface.eta reads at one wavelength: a pairs coat of
    // index 2 divides the metal's eta and k by 2, not by the pair list's
    // first number.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"coated\" \"string type\" [\"coatedconductor\"] ",
            "\"spectrum interface.eta\" [400 2 700 2] ",
            "\"spectrum conductor.eta\" [\"metal-Au-eta\"] ",
            "\"spectrum conductor.k\" [\"metal-Au-k\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project3"));
    let reference = || state(&core)["context"]["project"].clone();
    let coated = list_entry(&core, &reference, "Materials", "coated");
    let target = json!({"kind":"resource","id":coated["id"]});
    let answer = conductor_answer(&core, &reference, &target, "conductor.k");
    let expected = spectrum::conductor_f0(
        spectrum::Value::Named("metal-Au-eta"),
        spectrum::Value::Named("metal-Au-k"),
        2.,
    )
    .unwrap();
    for (got, want) in answer["rgb"].as_array().unwrap().iter().zip(expected) {
        assert!((got.as_f64().unwrap() - want).abs() < 1e-9, "{answer}");
    }
    drop(host);
}

#[test]
fn spectrum_color_conductor_role_fills_defaults_and_warns_like_r4() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");

    // Only eta declared: r4 fills k with copper, and the source says so.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"metal-Au-eta\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let gold = list_entry(&core, &reference, "Materials", "gold");
    let target = json!({"kind":"resource","id":gold["id"]});
    let answer = conductor_answer(&core, &reference, &target, "eta");
    let expected = spectrum::conductor_f0(
        spectrum::Value::Named("metal-Au-eta"),
        spectrum::Value::Named("metal-Cu-k"),
        1.,
    )
    .unwrap();
    for (got, want) in answer["rgb"].as_array().unwrap().iter().zip(expected) {
        assert!((got.as_f64().unwrap() - want).abs() < 1e-9, "{answer}");
    }
    assert_eq!(answer["source"], "eta: metal-Au-eta · k: default (Cu)");

    // Only the reflectance declared: the Metal colour is its colour.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"rgb reflectance\" [0.8 0.6 0.2]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project2"));
    let reference = || state(&core)["context"]["project"].clone();
    let gold = list_entry(&core, &reference, "Materials", "gold");
    let target = json!({"kind":"resource","id":gold["id"]});
    let answer = conductor_answer(&core, &reference, &target, "reflectance");
    assert_eq!(answer["rgb"], json!([0.8, 0.6, 0.2]), "{answer}");
    assert_eq!(answer["source"], "reflectance: rgb 0.8 0.6 0.2");

    // r4 refuses a reflectance beside eta/k: no Metal colour, the
    // warning on the row, grey.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"rgb reflectance\" [0.8 0.6 0.2] \"spectrum k\" [\"metal-Au-k\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project3"));
    let reference = || state(&core)["context"]["project"].clone();
    let gold = list_entry(&core, &reference, "Materials", "gold");
    let target = json!({"kind":"resource","id":gold["id"]});
    let answer = conductor_answer(&core, &reference, &target, "eta");
    assert_eq!(answer["rgb"], json!([0.5, 0.5, 0.5]));
    assert_eq!(
        answer["source"],
        "r4 rejects reflectance together with eta/k"
    );

    // eta wired to a texture cannot be evaluated: grey, saying so.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"bumps\" \"spectrum\" \"constant\" \"rgb value\" [0.3 0.3 0.3]\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"texture eta\" [\"bumps\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project4"));
    let reference = || state(&core)["context"]["project"].clone();
    let gold = list_entry(&core, &reference, "Materials", "gold");
    let target = json!({"kind":"resource","id":gold["id"]});
    let answer = conductor_answer(&core, &reference, &target, "eta");
    assert_eq!(answer["rgb"], json!([0.5, 0.5, 0.5]));
    assert_eq!(
        answer["source"],
        "eta: texture (not evaluated) · k: default (Cu)"
    );

    // Not a conductor: the conductor role refuses the material.
    fs::write(
        &source,
        "WorldBegin\nMakeNamedMaterial \"wall\" \"string type\" [\"diffuse\"]\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project5"));
    let reference = || state(&core)["context"]["project"].clone();
    let wall = list_entry(&core, &reference, "Materials", "wall");
    let error = core
        .request(request(
            Method::ProjectSpectrumColor,
            json!({"project":reference(),
                   "target":{"kind":"resource","id":wall["id"]},
                   "parameter":"reflectance","role":"conductor"}),
        ))
        .error
        .expect("a diffuse material has no Metal colour");
    assert_eq!(error.code, protocol::error::ErrorCode::InvalidRequest);
    // A node target is not a metal either.
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"Shape"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    let error = core
        .request(request(
            Method::ProjectSpectrumColor,
            json!({"project":reference(),
                   "target":{"kind":"node","id":node},
                   "parameter":"reflectance","role":"conductor"}),
        ))
        .error
        .expect("a node is not a material");
    assert_eq!(error.code, protocol::error::ErrorCode::InvalidRequest);
    // The metal-row names are the only parameters the role answers.
    let error = core
        .request(request(
            Method::ProjectSpectrumColor,
            json!({"project":reference(),
                   "target":{"kind":"resource","id":wall["id"]},
                   "parameter":"roughness","role":"conductor"}),
        ))
        .error
        .expect("the conductor role answers only the metal rows");
    assert_eq!(error.code, protocol::error::ErrorCode::InvalidRequest);
    drop(host);
}

/// a non-metal curve on eta — a glass name reads its
/// channel samples (the rgb way), and a name with neither is answered
/// grey with a source saying it is no refractive index, never an error.
#[test]
fn spectrum_color_conductor_reads_glass_and_greys_out_illuminants() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");

    // A dielectric's glass name on a conductor's eta: the channel
    // samples with r4's copper k, not "the declared eta/k curves could
    // not be read".
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"glass-metal\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"glass-BK7\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let glass = list_entry(&core, &reference, "Materials", "glass-metal");
    let target = json!({"kind":"resource","id":glass["id"]});
    let answer = conductor_answer(&core, &reference, &target, "eta");
    let expected = spectrum::conductor_f0(
        spectrum::Value::Named("glass-BK7"),
        spectrum::Value::Named("metal-Cu-k"),
        1.,
    )
    .unwrap();
    for (got, want) in answer["rgb"].as_array().unwrap().iter().zip(expected) {
        assert!((got.as_f64().unwrap() - want).abs() < 1e-9, "{answer}");
    }
    assert_eq!(answer["source"], "eta: glass-BK7 · k: default (Cu)");
    assert_eq!(answer["role"], "conductor");

    // A standard illuminant is not a refractive index: the neutral grey
    // with a source that says so, not an error.
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"warm\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"stdillum-D65\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project2"));
    let reference = || state(&core)["context"]["project"].clone();
    let warm = list_entry(&core, &reference, "Materials", "warm");
    let target = json!({"kind":"resource","id":warm["id"]});
    let answer = conductor_answer(&core, &reference, &target, "k");
    assert_eq!(answer["rgb"], json!([0.5, 0.5, 0.5]), "{answer}");
    assert_eq!(answer["role"], "conductor");
    assert_eq!(
        answer["source"],
        "eta: stdillum-D65 is not a refractive index (not evaluated) · k: default (Cu)"
    );
    drop(host);
}

#[test]
fn spectrum_color_chromaticity_role_normalizes_the_colour_free_of_magnitude() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let temp = tempfile::tempdir().unwrap();
    fs::copy(
        spectrum_fixture("Au.eta.spd"),
        temp.path().join("Au.eta.spd"),
    )
    .unwrap();
    fs::copy(spectrum_fixture("Au.k.spd"), temp.path().join("Au.k.spd")).unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"Au.eta.spd\"] \"spectrum k\" [\"Au.k.spd\"]\n",
            "LightSource \"distant\" \"spectrum L\" [\"Au.k.spd\"]\n",
        ),
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let au_k = list_entry(&core, &reference, "Spectra", "Au.k");
    // A SpectrumData asked directly shows its chromaticity by default:
    // the curve's colour, magnitude-free — not the illuminant read.
    let au_k_pairs = spd_pairs(&spectrum_fixture("Au.k.spd"));
    let answer = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),
               "target":{"kind":"resource","id":au_k["id"]},
               "parameter":"values"}),
    );
    assert_eq!(
        answer["rgb"],
        json!(spectrum::chromaticity(spectrum::Value::Pairs(&au_k_pairs)).unwrap())
    );
    assert_eq!(answer["role"], "chromaticity");
    // An explicit chromaticity role on a light's L shows the same
    // colour without the brightness.
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"LightSource"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    let answer = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),
               "target":{"kind":"node","id":node},
               "parameter":"L","role":"chromaticity"}),
    );
    assert_eq!(
        answer["rgb"],
        json!(spectrum::chromaticity(spectrum::Value::Pairs(&au_k_pairs)).unwrap())
    );
    assert_eq!(answer["role"], "chromaticity");
    drop(host);
}

#[test]
fn spectrum_color_resolves_blackbody_to_its_rgb_and_a_human_source() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nLightSource \"point\" \"blackbody I\" [6500]\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"LightSource"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    assert!(!node.is_null(), "{hits:?}");
    let color = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"node","id":node},"parameter":"I"}),
    );
    assert_eq!(color["rgb"], json!(spectrum::blackbody_rgb(6500.)));
    assert_eq!(color["source"], "blackbody 6500 K");
    assert_eq!(color["role"], "illuminant");
    drop(host);
}

#[test]
fn spectrum_color_resolves_inline_rgb_and_sample_pairs() {
    // One scene per light, since node search finds them by directive.
    let rgb = {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("scene.pbrt");
        fs::write(
            &source,
            "WorldBegin\nLightSource \"point\" \"rgb I\" [0.25 0.5 0.75]\n",
        )
        .unwrap();
        let host = CoreHost::start(Limits::default());
        let core = host.handle();
        import(&core, &source, &temp.path().join("project"));
        let reference = || state(&core)["context"]["project"].clone();
        let hits = call(
            &core,
            Method::ProjectSearchNodes,
            json!({"project":reference(),"query":"LightSource"}),
        );
        let node = &hits["nodes"][0]["node_id"];
        assert!(!node.is_null(), "{hits:?}");
        let rgb = call(
            &core,
            Method::ProjectSpectrumColor,
            json!({"project":reference(),"target":{"kind":"node","id":node},
                   "parameter":"I"}),
        );
        drop(host);
        rgb
    };
    assert_eq!(rgb["rgb"], json!([0.25, 0.5, 0.75]));
    assert_eq!(rgb["source"], "rgb 0.25 0.5 0.75");
    assert_eq!(rgb["role"], "illuminant");
    // Inline λ v pairs: the illuminant read Lit shades with, source
    // naming the form.
    let pairs = {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("scene.pbrt");
        fs::write(
            &source,
            "WorldBegin\nLightSource \"distant\" \"spectrum L\" [400 0.5 700 0.5]\n",
        )
        .unwrap();
        let host = CoreHost::start(Limits::default());
        let core = host.handle();
        import(&core, &source, &temp.path().join("project"));
        let reference = || state(&core)["context"]["project"].clone();
        let hits = call(
            &core,
            Method::ProjectSearchNodes,
            json!({"project":reference(),"query":"LightSource"}),
        );
        let node = &hits["nodes"][0]["node_id"];
        assert!(!node.is_null(), "{hits:?}");
        let pairs = call(
            &core,
            Method::ProjectSpectrumColor,
            json!({"project":reference(),"target":{"kind":"node","id":node},
                   "parameter":"L"}),
        );
        drop(host);
        pairs
    };
    assert_eq!(pairs["rgb"], json!([0.5, 0.5, 0.5]));
    assert_eq!(pairs["source"], "spectrum samples (2 pairs)");
    assert_eq!(pairs["role"], "illuminant");
}

#[test]
fn spectrum_color_resolves_spectrum_data_references_and_their_resources() {
    let temp = tempfile::tempdir().unwrap();
    fs::copy(
        spectrum_fixture("Au.eta.spd"),
        temp.path().join("Au.eta.spd"),
    )
    .unwrap();
    fs::copy(spectrum_fixture("Au.k.spd"), temp.path().join("Au.k.spd")).unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"Au.eta.spd\"] \"spectrum k\" [\"Au.k.spd\"]\n",
            "NamedMaterial \"gold\"\n",
            "Shape \"sphere\"\n",
            "LightSource \"distant\" \"spectrum L\" [\"Au.eta.spd\"]\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    // The material resource in Materials/, and the shared curve resources
    // in Spectra/.
    let entry_in = |folder: &str, name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":folder}),
        );
        let entry = contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} in {folder}: {contents}"))
            .clone();
        assert_eq!(entry["kind"], "resource", "{entry}");
        entry
    };
    let gold = entry_in("Materials", "gold");
    let gold_k = entry_in("Spectra", "Au.k");
    let gold_eta = entry_in("Spectra", "Au.eta");

    let target = |entry: &Value| json!({"kind":"resource","id":entry["id"]});
    let color = |target: Value, parameter: &str, role: Option<&str>| {
        call(
            &core,
            Method::ProjectSpectrumColor,
            json!({"project":reference(),"target":target,"parameter":parameter,"role":role}),
        )
    };
    // A conductor's k is unbounded: the CIE integration, not the channel
    // read, and the source names the referenced curve.
    let au_k = spd_pairs(&spectrum_fixture("Au.k.spd"));
    let expected = spectrum::pairs_rgb(&au_k, spectrum::Role::Unbounded).unwrap();
    let k = color(target(&gold), "k", None);
    assert_eq!(k["rgb"], json!(expected));
    assert_eq!(k["source"], "SpectrumData: Au.k");
    assert_eq!(k["role"], "unbounded");
    // The same curve asked directly as a resource shows its chromaticity
    // by default and switches
    // to the CIE integration when the role says albedo.
    let channel = spectrum::pairs_rgb(&au_k, spectrum::Role::Illuminant).unwrap();
    let cie = spectrum::pairs_rgb(&au_k, spectrum::Role::Albedo).unwrap();
    assert_ne!(json!(channel), json!(cie), "Au.k is not flat");
    let resource_default = color(target(&gold_k), "values", None);
    assert_eq!(
        resource_default["rgb"],
        json!(spectrum::chromaticity(spectrum::Value::Pairs(&au_k)).unwrap())
    );
    assert_eq!(resource_default["source"], "SpectrumData: Au.k");
    assert_eq!(resource_default["role"], "chromaticity");
    let resource_albedo = color(target(&gold_k), "wavelengths", Some("albedo"));
    assert_eq!(resource_albedo["rgb"], json!(cie));
    assert_eq!(resource_albedo["role"], "albedo");
    // A light's L pointing at the same file resolves like Lit shades it:
    // the illuminant read, source naming the curve resource.
    let eta_pairs = spd_pairs(&spectrum_fixture("Au.eta.spd"));
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"LightSource"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    let light_color = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"node","id":node},"parameter":"L"}),
    );
    assert_eq!(
        light_color["rgb"],
        json!(spectrum::pairs_rgb(&eta_pairs, spectrum::Role::Illuminant).unwrap())
    );
    assert_eq!(light_color["source"], "SpectrumData: Au.eta");
    assert_eq!(light_color["role"], "illuminant");
    let _ = gold_eta;
    drop(host);
}

/// the resource graph joins SpectrumData resources as upstream leaf
/// nodes — a conductor's spd-wired `k` gives a `spectrum_data` node with
/// an edge into the slot, and the slot says it takes spectrum data; the
/// declaration's removal edit drops the node again and one Undo puts it
/// back.
#[test]
fn resource_graph_joins_spectrum_data_upstream_as_leaf_nodes() {
    let temp = tempfile::tempdir().unwrap();
    fs::copy(spectrum_fixture("Au.k.spd"), temp.path().join("Au.k.spd")).unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"gold\" \"string type\" [\"conductor\"] ",
            "\"spectrum k\" [\"Au.k.spd\"]\n",
            "NamedMaterial \"gold\"\n",
            "Shape \"sphere\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let id_in = |folder: &str, name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":folder}),
        );
        contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("missing {name} in {folder}: {contents}"))["id"]
            .clone()
    };
    let gold = id_in("Materials", "gold");
    let curve = id_in("Spectra", "Au.k");
    let graph = |resource_id: Value| {
        call(
            &core,
            Method::ProjectGetResourceGraph,
            json!({"project":reference(),"resource_id":resource_id}),
        )
    };
    let reply = graph(gold.clone());
    let ids = reply["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].clone())
        .collect::<Vec<_>>();
    assert_eq!(ids, [gold.clone(), curve.clone()], "{reply}");
    let node = &reply["nodes"][1];
    assert_eq!(node["kind"], "spectrum_data", "{reply}");
    assert_eq!(node["texture_value_type"], serde_json::Value::Null);
    assert_eq!(node["inputs"], json!([]));
    assert_eq!(node["label"], "Au.k");
    assert_eq!(node["implementation"], "piecewise");
    assert_eq!(
        reply["edges"],
        json!([{"from":curve,"to":gold,"parameter":"k"}])
    );
    let k = reply["nodes"][0]["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["parameter"] == "k")
        .unwrap_or_else(|| panic!("no k input: {reply}"));
    assert_eq!(k["accepts"], "spectrum");
    assert_eq!(k["spectrum_data"], true);
    assert_eq!(k["connectable"], true);
    // The declaration as written — the removal edit a drop (or the
    // colour dialog) must name, and the type it replaces first.
    assert_eq!(k["declared_name"], "k");
    assert_eq!(k["declared_type"], "spectrum");
    // The removal edit the colour dialog's disconnect sends — the
    // declaration drops, and with it the node and the edge.
    let target = json!({"kind":"resource","id":gold});
    let apply = |edits: Value| {
        call(
            &core,
            Method::ProjectApply,
            json!({"project":reference(),"edits":edits}),
        )
    };
    apply(json!([{"kind":"set_parameter","target":target,
                  "name":"k","value_type":"spectrum","value":null}]));
    let reply = graph(gold.clone());
    assert_eq!(reply["nodes"].as_array().unwrap().len(), 1, "{reply}");
    assert_eq!(reply["edges"].as_array().unwrap().len(), 0, "{reply}");
    // One Undo: the curve node and its edge are back.
    call(&core, Method::ProjectUndo, json!({"project":reference()}));
    let reply = graph(gold.clone());
    assert_eq!(
        reply["edges"],
        json!([{"from":curve,"to":gold,"parameter":"k"}]),
        "{reply}"
    );
    // Only materials and textures have a graph: a curve is a leaf, never
    // a root.
    let error = core
        .request(request(
            Method::ProjectGetResourceGraph,
            json!({"project":reference(),"resource_id":curve}),
        ))
        .error
        .expect("a spectrum data has no graph");
    assert!(
        error.message.contains("only materials and textures"),
        "{error:?}"
    );
    drop(host);
}

#[test]
fn spectrum_color_resolves_builtin_names_or_degrades_white_like_lit() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "MakeNamedMaterial \"metal\" \"string type\" [\"conductor\"] ",
            "\"spectrum eta\" [\"metal-Au-eta\"]\n",
            "LightSource \"point\" \"spectrum I\" [\"stdillum-D65\"]\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Materials"}),
    );
    let metal = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["name"] == "metal")
        .unwrap()
        .clone();
    // A builtin table name resolves to the r4 values and says its name.
    let eta = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"resource","id":metal["id"]},
               "parameter":"eta"}),
    );
    assert_eq!(eta["rgb"], json!(spectrum::named("metal-Au-eta").unwrap()));
    assert_eq!(eta["source"], "metal-Au-eta");
    assert_eq!(eta["role"], "unbounded");
    // A name the table has no values for degrades to white, like Lit,
    // and still names itself.
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"LightSource"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    let degenerate = call(
        &core,
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"node","id":node},"parameter":"I"}),
    );
    assert_eq!(degenerate["rgb"], json!([1., 1., 1.]));
    assert_eq!(degenerate["source"], "stdillum-D65");
    drop(host);
}

#[test]
fn spectrum_color_rejects_unknown_roles_parameters_and_targets() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nLightSource \"point\" \"blackbody I\" [6500]\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let hits = call(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":reference(),"query":"LightSource"}),
    );
    let node = &hits["nodes"][0]["node_id"];
    let expect_error = |method: Method, params: Value| {
        let response = core.request(request(method, params));
        response.error.expect("the request must fail")
    };
    let bogus_role = expect_error(
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"node","id":node},
               "parameter":"I","role":"cheerful"}),
    );
    assert_eq!(bogus_role.code, protocol::error::ErrorCode::InvalidRequest);
    let missing = expect_error(
        Method::ProjectSpectrumColor,
        json!({"project":reference(),"target":{"kind":"node","id":node},
               "parameter":"nope"}),
    );
    assert_eq!(missing.code, protocol::error::ErrorCode::NotFound);
    let missing_target = expect_error(
        Method::ProjectSpectrumColor,
        json!({"project":reference(),
               "target":{"kind":"node","id":"00000000-0000-0000-0000-000000000123"},
               "parameter":"I"}),
    );
    assert_eq!(missing_target.code, protocol::error::ErrorCode::NotFound);
    drop(host);
}

/// Review issue 1's end-to-end pin: a `.array`-backed
/// SpectrumData table — every table the import bulk-converts holds more
/// than 4096 values — rewrites through the chunked windows the editor
/// now sends, and one whole-table window would break the wire's
/// per-window limit. The seam between the two windows must land joined.
#[test]
fn chunked_windows_rewrite_a_data_asset_backed_spectrum_table() {
    let temp = tempfile::tempdir().unwrap();
    let pairs = (0..5000)
        .map(|index| format!("{} {}", 360 + index, 1.5))
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(
        temp.path().join("big.spd"),
        format!("# big curve\n{pairs}\n"),
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nLightSource \"distant\" \"spectrum L\" [\"big.spd\"]\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Spectra"}),
    );
    let entry = contents["entries"]
        .as_array()
        .and_then(|entries| entries.iter().find(|entry| entry["name"] == "big").cloned())
        .unwrap_or_else(|| panic!("the imported spectrum resource: {contents}"));
    assert_eq!(entry["kind"], "resource");
    let target = json!({"kind":"resource","id":entry["id"]});
    // The bulk threshold passed: both tables ride data assets.
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":target}),
    );
    let kind_of = |name: &str| {
        parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == name)
            .unwrap_or_else(|| panic!("the {name} table: {parameters}"))["values"]["kind"]
            .clone()
    };
    assert_eq!(kind_of("wavelengths"), json!("array"), "{parameters}");
    assert_eq!(kind_of("values"), json!("array"), "{parameters}");
    // The editor's whole-table rewrite: two contiguous windows, each
    // within the 4096-value per-window limit.
    let rewritten: Vec<f64> = (0..5000).map(|index| index as f64 * 0.25).collect();
    let windows = json!([
        {"offset":0,"values":{"kind":"floats","value":&rewritten[0..4096]}},
        {"offset":4096,"values":{"kind":"floats","value":&rewritten[4096..]}},
    ]);
    let job = call(
        &core,
        Method::ProjectSetParameterValues,
        json!({"project":reference(),"target":target,"name":"values","windows":windows}),
    );
    let done = wait(&core, job);
    assert_eq!(done["state"], "succeeded", "{done}");
    // The seam: a window read across offset 4096 holds chunk-0's tail
    // then chunk-1's head, with the length unchanged.
    let page = call(
        &core,
        Method::ProjectGetParameterValues,
        json!({"project":reference(),"target":target,"name":"values",
               "offset":4090,"limit":12}),
    );
    assert_eq!(page["total"], json!(5000.to_string()), "{page}");
    for (position, value) in page["values"]["value"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        let index = 4090 + position;
        assert_eq!(value["kind"], "finite", "{value}");
        assert_eq!(value["value"], json!(rewritten[index]), "{page}");
    }
    drop(host);
}

#[test]
fn component_edits_manage_area_light_and_reverse_orientation() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let roots = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference()}),
    );
    let children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(),"parent":roots["nodes"][0]["node_id"]}),
    );
    let shape = children["nodes"][0]["node_id"].clone();

    // Adding an area light with omitted parameters declares its initial values:
    // L [1 1 1] plus the table's constant defaults.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_area_light","node_id":shape,
             "light":{"implementation":"diffuse"}}]}),
    );
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":shape}),
    );
    let light_id = details["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == "area_light")
        .expect("the shape shows an area light component")["node_id"]
        .clone();
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":{"kind":"node","id":light_id}}),
    )["parameters"]
        .as_array()
        .unwrap()
        .clone();
    let light = parameters
        .iter()
        .find(|p| p["name"] == "L")
        .expect("the area light declares L [1 1 1]");
    assert_eq!(light["value_type"], "rgb");
    assert_eq!(light["values"]["kind"], "floats");
    assert!(
        light["values"]["value"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["value"] == json!(1.0)),
        "{}",
        light["values"]
    );
    assert!(
        parameters.iter().any(|p| p["name"] == "twosided"),
        "the table's constant defaults are declared"
    );

    // The shared-with- export path stays green with the component.
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    let scene = std::fs::read_to_string(temp.path().join("bundle/scene.pbrt")).unwrap();
    assert!(scene.contains("AreaLightSource \"diffuse\""), "{scene}");
    assert!(scene.contains("\"rgb L\" [ 1 1 1 ]"), "{scene}");

    // Removing it unwraps the shape again, and undo brings it back.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_area_light","node_id":shape,"light":null}]}),
    );
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle2")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    let scene = std::fs::read_to_string(temp.path().join("bundle2/scene.pbrt")).unwrap();
    assert!(!scene.contains("AreaLightSource"), "{scene}");
    let world_children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(),"parent":roots["nodes"][0]["node_id"]}),
    );
    assert_eq!(
        (
            world_children["nodes"].as_array().unwrap().len(),
            world_children["nodes"][0]["node_id"].clone()
        ),
        (1, shape.clone()),
        "the dedicated group unwrapped: {world_children}"
    );
    let undone = call(&core, Method::ProjectUndo, json!({"project":reference()}));
    assert!(!undone["nodes"].as_array().unwrap().is_empty());
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":shape}),
    );
    assert!(
        details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "area_light"),
        "undo restores the component"
    );

    // Reverse orientation toggles through the dedicated group; a request
    // that clears no binding is rejected up front.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_reverse_orientation","node_id":shape,"enabled":true}]}),
    );
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle3")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    let scene = std::fs::read_to_string(temp.path().join("bundle3/scene.pbrt")).unwrap();
    assert!(scene.contains("ReverseOrientation"), "{scene}");
    let error = core.request(request(
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"clear_local_binding","node_id":shape,"material":false,"medium":false}]}),
    ));
    assert_eq!(error.error.unwrap().code, ErrorCode::InvalidRequest);
    drop(host);
}

#[test]
fn group_and_camera_component_edits_bind_through_the_wire() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "Camera \"perspective\"\nWorldBegin\nAttributeBegin\nAttributeBegin\nShape \"sphere\"\nAttributeEnd\nAttributeEnd\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    // The scene-graph view roots at the scene: its children are the
    // options' cameras and World content. The World scope stays hidden.
    let nodes = |parent: Value| {
        call(
            &core,
            Method::ProjectListNodes,
            json!({"project":reference(),"parent":parent,"view":"scene_graph"}),
        )
    };
    let scene_children = nodes(state(&core)["active_scene"].clone());
    let world = state(&core)["active_scene"].clone();
    let camera = scene_children["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["object"] == "camera")
        .expect("the camera")["node_id"]
        .clone();
    let group = nodes(world.clone())["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["object"] == "group")
        .expect("the outer AttributeBegin group")["node_id"]
        .clone();
    let inner_group = nodes(group.clone())["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["object"] == "group")
        .expect("the nested AttributeBegin group")["node_id"]
        .clone();
    let shape = nodes(inner_group.clone())["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["object"] == "shape")
        .expect("the nested shape")["node_id"]
        .clone();

    // A material resource to bind with.
    let created = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"material",
             "implementation":"diffuse","label":"Copper"}]}),
    );
    let material = created["created"][0]["id"].clone();
    let created = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"medium",
             "implementation":"homogeneous","label":"Group Fog"}]}),
    );
    let group_medium = created["created"][0]["id"].clone();

    // Group bindings remain local state in the document/evaluator, but
    // Material and Medium rows are exposed only by the Shape components.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_local_binding","node_id":group,
             "material":{"kind":"resource","value":material},
             "medium":{"inside":group_medium,"outside":null}}]}),
    );
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":group}),
    );
    let rows = details["components"].as_array().unwrap().clone();
    assert!(
        rows.iter().all(|component| {
            !matches!(component["kind"].as_str(), Some("material" | "medium"))
        }),
        "Group hides local binding rows: {rows:?}"
    );
    let inherited_group = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":inner_group}),
    );
    assert!(
        inherited_group["components"]
            .as_array()
            .unwrap()
            .iter()
            .all(|component| {
                !matches!(component["kind"].as_str(), Some("material" | "medium"))
            }),
        "Group hides inherited binding rows: {}",
        inherited_group["components"]
    );
    let shape_details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":shape}),
    );
    assert!(
        shape_details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "material"
                && component["binding"]["value"] == material
                && component["local"] == false),
        "the shape reports the group's binding as inherited"
    );
    assert!(
        shape_details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| {
                component["kind"] == "medium"
                    && component["inside"] == group_medium
                    && component["local"] == false
            }),
        "the shape reports the group's medium binding as inherited"
    );

    // The group's area light: set at the head, read back, removed.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_area_light","node_id":group,
             "light":{"implementation":"diffuse"}}]}),
    );
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":group}),
    );
    assert!(
        details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "area_light" && component["local"] == true),
        "the group reads its own head light"
    );

    // The camera's distinct medium sits in the options list and exports with it.
    let created = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"create_resource","resource_kind":"medium",
             "implementation":"homogeneous","label":"Camera Fog"}]}),
    );
    let medium = created["created"][0]["id"].clone();
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_local_binding","node_id":camera,
             "material":null,
             "medium":{"inside":null,"outside":medium}}]}),
    );
    let camera_node = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":camera}),
    );
    let medium_component = camera_node["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == "medium")
        .unwrap();
    assert_eq!(medium_component["binding"], medium);
    assert_eq!(medium_component["preserved_inside"], Value::Null);
    assert!(medium_component.get("outside").is_none());
    assert!(medium_component.get("inside").is_none());
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle")}),
    );
    assert_eq!(wait(&core, export)["state"], "succeeded");
    // The camera's medium writes before it, and the group's local material
    // and medium remain represented in PBRT directives after hiding those UI rows.
    let scene = std::fs::read_to_string(temp.path().join("bundle/scene.pbrt")).unwrap();
    let medium_at = scene
        .find("MediumInterface")
        .expect("the camera medium exports");
    let camera_at = scene
        .find("Camera \"perspective\"")
        .expect("the camera exports");
    assert!(medium_at < camera_at, "{scene}");
    assert_eq!(scene.matches("MediumInterface").count(), 2, "{scene}");
    let area_at = scene
        .find("AreaLightSource")
        .expect("the group light exports");
    assert!(
        // The binding line, not the MakeNamedMaterial definition above it.
        scene
            .find("\nNamedMaterial \"Copper\"")
            .is_some_and(|at| at > area_at),
        "the group's material binding remains after its light at the head: {scene}"
    );

    // The reverse row reads on the shape, and groups never get one.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[
            {"kind":"set_reverse_orientation","node_id":shape,"enabled":true}]}),
    );
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":shape}),
    );
    assert!(
        details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "reverse_orientation"
                && component["enabled"] == true
                && component["local"] == true),
        "the shape reports its local toggle"
    );
    let details = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":group}),
    );
    assert!(
        !details["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "reverse_orientation"),
        "groups take no reverse orientation row"
    );
    drop(host);
}

#[test]
fn set_implementation_keeps_parameters_reports_and_refuses_broken_references() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "Integrator \"path\" \"integer maxdepth\" 7\n",
            "Camera \"perspective\" \"float fov\" 45\n",
            "WorldBegin\n",
            "MakeNamedMaterial \"a\" \"string type\" \"diffuse\"\n",
            "MakeNamedMaterial \"b\" \"string type\" \"conductor\"\n",
            "MakeNamedMaterial \"mixed\" \"string type\" \"mix\" ",
            "\"string materials\" [ \"a\" \"b\" ] \"float amount\" 0.25\n",
            "Shape \"sphere\" \"float radius\" 2 \"float zmax\" 1\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let roots = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference()}),
    );
    let root = |label: &str| {
        roots["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["label"] == label)
            .unwrap_or_else(|| panic!("no {label} in {roots}"))["node_id"]
            .clone()
    };
    let (integrator, camera, world) = (root("Integrator"), root("Camera"), root("World"));
    let world_children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(),"parent":world}),
    );
    let shape = world_children["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "entity")
        .unwrap()["node_id"]
        .clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Materials"}),
    );
    let mixed = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["location"].as_str().unwrap().contains("mixed"))
        .unwrap_or_else(|| panic!("{contents}"))["id"]
        .clone();
    let node = |id: &Value| json!({"kind":"node","id":id});
    let set = |target: Value, implementation: &str| {
        json!({"kind":"set_implementation","target":target,
               "implementation":implementation,"parameters":"keep_compatible"})
    };
    let apply = |edits: Value| {
        core.request(request(
            Method::ProjectApply,
            json!({"project":reference(),"edits":edits}),
        ))
    };
    let plan = |target: Value, implementation: &str| {
        call(
            &core,
            Method::ProjectPlanImplementation,
            json!({"project":reference(),"target":target,"implementation":implementation}),
        )
    };

    // previews exactly what the apply reports; it edits nothing.
    let before = reference();
    let preview = plan(node(&shape), "disk");
    assert_eq!(preview["implementation"], "disk");
    assert_eq!(preview["changed"], true);
    assert_eq!(preview["kept_unknown"], json!(["zmax"]));
    assert_eq!(preview["breaks_reference"], json!([]));
    assert_eq!(reference(), before);
    let applied = apply(json!([set(node(&shape), "disk")])).result.unwrap();
    assert_eq!(
        applied["kept_unknown"],
        json!([{"target":node(&shape),"parameters":["zmax"]}])
    );
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference(),"target":node(&shape)}),
    );
    let names = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["radius", "zmax"], "every parameter stays");

    // Same implementation: no-op, empty report, no new revision.
    let before = reference();
    let applied = apply(json!([set(node(&shape), "disk")])).result.unwrap();
    assert_eq!(applied["kept_unknown"], json!([]));
    assert_eq!(reference(), before);
    let unchanged = plan(node(&shape), "disk");
    assert_eq!(unchanged["changed"], false);
    assert_eq!(unchanged["edits"], json!([]));

    // A kept material reference diffuse cannot export: refused up front.
    let mixed_target = json!({"kind":"resource","id":mixed});
    assert_eq!(
        plan(mixed_target.clone(), "diffuse")["breaks_reference"],
        json!(["materials"])
    );
    let error = apply(json!([set(mixed_target.clone(), "diffuse")]))
        .error
        .unwrap();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let details = error.details.unwrap();
    assert_eq!(details["reason"], "breaks_reference");
    assert_eq!(details["parameters"], json!(["materials"]));
    // edits remove it first and change in one transaction.
    let planned = plan(mixed_target.clone(), "diffuse");
    assert_eq!(
        planned["edits"],
        json!([
            {"kind":"set_parameter","target":mixed_target,"name":"materials",
             "value_type":"string","value":null},
            set(mixed_target.clone(), "diffuse"),
        ])
    );
    let applied = apply(planned["edits"].clone()).result.unwrap();
    assert_eq!(
        applied["kept_unknown"],
        json!([{"target":mixed_target,"parameters":["amount"]}])
    );

    // Options-list nodes: the configuration keeps a row its new table
    // knows (gave path and volpath the same rows), the camera drops
    // its fov to an implementation without one.
    let applied = apply(json!([
        set(node(&integrator), "volpath"),
        set(node(&camera), "orthographic"),
    ]))
    .result
    .unwrap();
    assert_eq!(
        applied["kept_unknown"],
        json!([{"target":node(&camera),"parameters":["fov"]}]),
        "the integrator's maxdepth is volpath's row too"
    );

    // Refusals carry their reason.
    for (target, implementation, reason) in [
        (node(&integrator), "whitted", "unsupported_implementation"),
        (node(&shape), "", "empty_implementation"),
    ] {
        let error = apply(json!([set(target, implementation)])).error.unwrap();
        assert_eq!(error.details.unwrap()["reason"], reason);
    }

    // Undo walks back one transaction at a time; the export follows.
    let undone = call(&core, Method::ProjectUndo, json!({"project":reference()}));
    assert_eq!(undone["kept_unknown"], json!([]));
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "succeeded", "{job}");
    let text = fs::read_to_string(job["result"]["scene_path"].as_str().unwrap()).unwrap();
    assert!(text.contains("Integrator \"path\""), "{text}");
    assert!(text.contains("Shape \"disk\""), "{text}");
    assert!(
        text.contains("\"mixed\" \"string type\" [\"diffuse\"] \"float amount\""),
        "{text}"
    );
}

#[test]
fn implementation_plan_removes_a_texture_with_the_wrong_lineage() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nTexture \"eta-spectrum\" \"spectrum\" \"constant\" \"spectrum value\" [\"metal-Cu-eta\"]\nMakeNamedMaterial \"metal\" \"string type\" \"conductor\" \"texture eta\" \"eta-spectrum\"\nNamedMaterial \"metal\"\nShape \"sphere\"\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":project,"folder":"Materials"}),
    );
    let metal = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| {
            item["location"]
                .as_str()
                .is_some_and(|path| path.contains("metal"))
        })
        .unwrap()["id"]
        .clone();
    let target = json!({"kind":"resource","id":metal});
    let plan = call(
        &core,
        Method::ProjectPlanImplementation,
        json!({"project":project,"target":target,"implementation":"hair"}),
    );
    assert_eq!(plan["breaks_reference"], json!(["eta"]), "{plan}");
    assert_eq!(
        plan["edits"],
        json!([
            {"kind":"set_parameter","target":target,"name":"eta",
             "value_type":"texture","value":null},
            {"kind":"set_implementation","target":target,"implementation":"hair",
             "parameters":"keep_compatible"}
        ])
    );
}

#[test]
fn parameter_schema_lists_export_schema_candidates_without_a_table() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    // gave the integrator a table; the empty-implementation query
    // still lists candidates, now the table's rows.
    let reply = call(
        &core,
        Method::ProjectGetParameterSchema,
        json!({"directive":"Integrator","implementation":"path"}),
    );
    assert!(reply["parameters"].as_array().is_some(), "{reply}");
    let implementations = reply["implementations"].as_array().unwrap();
    assert!(implementations.contains(&json!("volpath")), "{reply}");
    let reply = call(
        &core,
        Method::ProjectGetParameterSchema,
        json!({"directive":"Material","implementation":"diffuse"}),
    );
    let implementations = reply["implementations"].as_array().unwrap();
    for implementation in ["thindielectric", "hair", "subsurface", "measured"] {
        assert!(implementations.contains(&json!(implementation)), "{reply}");
    }
    let reply = call(
        &core,
        Method::ProjectGetParameterSchema,
        json!({"directive":"AreaLightSource","implementation":"diffuse"}),
    );
    assert!(
        !reply["implementations"]
            .as_array()
            .unwrap()
            .contains(&json!(""))
    );
}

#[test]
fn create_resource_accepts_each_new_material_table() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let project = state(&core)["context"]["project"].clone();
    let edits = ["thindielectric", "hair", "subsurface", "measured"]
        .into_iter()
        .map(|implementation| {
            json!({
                "kind":"create_resource", "resource_kind":"material",
                "implementation":implementation, "label":implementation
            })
        })
        .collect::<Vec<_>>();
    let result = call(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":edits}),
    );
    assert_eq!(result["created"].as_array().unwrap().len(), 4, "{result}");
}

#[test]
fn spectra_are_listed_without_an_open_project() {
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let reply = call(&core, Method::ProjectListSpectra, json!({}));
    let names = reply["spectra"].as_array().unwrap();
    assert!(names.contains(&json!("metal-Cu-eta")), "{reply}");
    assert!(names.contains(&json!("glass-BK7")));
    let sorted = {
        let mut copy = names.clone();
        copy.sort_by(|a, b| a.as_str().cmp(&b.as_str()));
        copy
    };
    assert_eq!(*names, sorted);
}

#[test]
fn resource_graph_walks_upstream_only_and_stops_at_cycles_and_the_limit() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"checks\" \"spectrum\" \"checkerboard\"\n",
            "Texture \"grime\" \"float\" \"fbm\"\n",
            "Texture \"tint\" \"spectrum\" \"mix\" \"texture tex1\" \"checks\" \"texture amount\" \"grime\"\n",
            "MakeNamedMaterial \"wall\" \"string type\" \"diffuse\" \"texture reflectance\" \"tint\"\n",
            "MakeNamedMaterial \"metal\" \"string type\" \"conductor\"\n",
            "MakeNamedMaterial \"blend\" \"string type\" \"mix\" \"string materials\" [ \"wall\" \"metal\" ]\n",
            "NamedMaterial \"blend\"\n",
            "Shape \"sphere\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let id_of = |folder: &str, name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":folder}),
        );
        contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {contents}"))["id"]
            .clone()
    };
    let (wall, tint, checks, grime) = (
        id_of("Materials", "wall"),
        id_of("Textures", "tint"),
        id_of("Textures", "checks"),
        id_of("Textures", "grime"),
    );
    let graph = |resource: &Value, limit: Value| {
        call(
            &core,
            Method::ProjectGetResourceGraph,
            json!({"project":reference(),"resource_id":resource,"limit":limit}),
        )
    };
    let reply = graph(&wall, Value::Null);
    assert_eq!(reply["root"], wall);
    assert_eq!(reply["truncated"], false);
    let ids = reply["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["id"].clone())
        .collect::<Vec<_>>();
    // Breadth first from the root; the mix material that uses wall is
    // downstream and never appears.
    assert_eq!(
        ids,
        [wall.clone(), tint.clone(), checks.clone(), grime.clone()]
    );
    let node = |id: &Value| {
        reply["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["id"] == *id)
            .unwrap()
            .clone()
    };
    let root = node(&wall);
    assert_eq!(root["kind"], "material");
    assert_eq!(root["implementation"], "diffuse");
    let reflectance = root["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["parameter"] == "reflectance")
        .unwrap()
        .clone();
    assert_eq!(reflectance["accepts"], "spectrum");
    let mix = node(&tint);
    assert_eq!(mix["texture_value_type"], "spectrum");
    let amount = mix["inputs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|input| input["parameter"] == "amount")
        .unwrap()
        .clone();
    assert_eq!(amount["accepts"], "float");
    let mut edges = reply["edges"].as_array().unwrap().clone();
    edges.sort_by_key(|edge| edge["parameter"].as_str().unwrap().to_owned());
    assert_eq!(
        edges,
        [
            json!({"from":grime,"to":tint,"parameter":"amount"}),
            json!({"from":tint,"to":wall,"parameter":"reflectance"}),
            json!({"from":checks,"to":tint,"parameter":"tex1"}),
        ]
    );
    // The mix material's upstream holds both materials; its inputs accept
    // materials.
    let blend = id_of("Materials", "blend");
    let reply = graph(&blend, Value::Null);
    let input = &reply["nodes"][0]["inputs"][0];
    assert_eq!(input["parameter"], "materials");
    assert_eq!(input["accepts"], "material");
    assert_eq!(reply["nodes"].as_array().unwrap().len(), 6);
    // The limit truncates: only the root and its first neighbour, and no
    // edge leaves the kept nodes.
    let reply = graph(&wall, json!(2));
    assert_eq!(reply["truncated"], true);
    assert_eq!(reply["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(reply["edges"].as_array().unwrap().len(), 1);
    // Only materials and textures have graphs.
    let error = core
        .request(request(
            Method::ProjectGetResourceGraph,
            json!({"project":reference(),"resource_id":uuid::Uuid::new_v4()}),
        ))
        .error
        .unwrap();
    assert_eq!(error.code, ErrorCode::NotFound);
}

#[test]
fn a_new_mix_accepts_two_materials_and_refuses_a_self_cycle() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let create = |implementation: &str, label: &str| {
        let response = core.request(request(Method::ProjectApply, json!({"project":reference(),"edits":[{
            "kind":"create_resource","resource_kind":"material","implementation":implementation,"label":label
        }]})));
        assert!(
            response.error.is_none(),
            "create {label}: {:?}",
            response.error
        );
        response.result.unwrap()["created"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let first = create("diffuse", "First");
    let second = create("diffuse", "Second");
    let third = create("diffuse", "Third");
    let mix = create("mix", "Blend");
    let target = json!({"kind":"resource","id":mix});
    let set = |ids: Value| {
        let response = core.request(request(
            Method::ProjectApply,
            json!({"project":reference(),"edits":[{
                "kind":"set_parameter","target":target,"name":"materials","value_type":"string",
                "value":{"name":"materials","value_type":"string","color_space":null,
                    "values":{"kind":"resources","value":{"kind":"material","ids":ids}}}
            }]}),
        ));
        assert!(
            response.error.is_none(),
            "set mix materials: {:?}",
            response.error
        );
        response.result.unwrap()
    };
    let created = set(json!([first.clone(), second.clone()]));
    assert!(created["error"].is_null(), "{created}");
    let graph = call(
        &core,
        Method::ProjectGetResourceGraph,
        json!({"project":reference(),"resource_id":mix}),
    );
    let mut edges = graph["edges"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|edge| edge["parameter"] == "materials")
        .cloned()
        .collect::<Vec<_>>();
    edges.sort_by_key(|edge| edge["index"].as_u64());
    assert_eq!(edges.len(), 2, "{graph}");
    assert_eq!(edges[0]["index"], 0);
    assert_eq!(edges[1]["index"], 1);
    let replaced = set(json!([first.clone(), third.clone()]));
    assert!(replaced["error"].is_null(), "{replaced}");
    let graph = call(
        &core,
        Method::ProjectGetResourceGraph,
        json!({"project":reference(),"resource_id":mix}),
    );
    assert!(
        graph["edges"].as_array().unwrap().iter().any(|edge| {
            edge["parameter"] == "materials" && edge["index"] == 1 && edge["from"] == third
        }),
        "replacing one mix operand preserves its position: {graph}"
    );
    let cycle = core.request(request(
        Method::ProjectApply,
        json!({"project":reference(),"edits":[{
            "kind":"set_parameter","target":target,"name":"materials","value_type":"string",
            "value":{"name":"materials","value_type":"string","color_space":null,
                "values":{"kind":"resources","value":{"kind":"material","ids":[mix, second]}}}
        }]}),
    ));
    assert_eq!(
        cycle.error.unwrap().details.unwrap()["reason"],
        "resource_cycle"
    );
}

#[test]
fn textures_connect_disconnect_and_cycles_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"checks\" \"spectrum\" \"checkerboard\"\n",
            "Texture \"grime\" \"float\" \"fbm\"\n",
            "Texture \"tint\" \"spectrum\" \"scale\" \"texture tex\" \"checks\"\n",
            "MakeNamedMaterial \"wall\" \"string type\" \"diffuse\" \"rgb reflectance\" [ 0.2 0.3 0.4 ]\n",
            "NamedMaterial \"wall\"\n",
            "Shape \"sphere\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let entry_of = |folder: &str, name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":folder}),
        );
        contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap_or_else(|| panic!("no {name} in {contents}"))
            .clone()
    };
    let wall = entry_of("Materials", "wall")["id"].clone();
    let checks = entry_of("Textures", "checks");
    // Contents entries now say a texture's value type.
    assert_eq!(checks["texture_value_type"], "spectrum");
    let checks = checks["id"].clone();
    let grime = entry_of("Textures", "grime")["id"].clone();
    let tint = entry_of("Textures", "tint")["id"].clone();
    let resource = |id: &Value| json!({"kind":"resource","id":id});
    let apply = |edits: Value| {
        core.request(request(
            Method::ProjectApply,
            json!({"project":reference(),"edits":edits}),
        ))
    };
    let reason = |edits: Value| {
        let error = apply(edits).error.unwrap();
        error.details.clone().unwrap_or_else(|| panic!("{error:?}"))["reason"].clone()
    };
    let reflectance = || {
        call(
            &core,
            Method::ProjectGetParameters,
            json!({"project":reference(),"target":resource(&wall)}),
        )["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["name"] == "reflectance")
            .cloned()
    };

    // Connect replaces the rgb declaration with the texture reference.
    let applied = apply(json!([{"kind":"connect_texture","target":resource(&wall),
        "parameter":"reflectance","texture":tint}]));
    assert!(applied.error.is_none(), "{:?}", applied.error);
    let declared = reflectance().unwrap();
    assert_eq!(declared["value_type"], "texture");
    assert_eq!(declared["values"]["ids"], json!([tint]));
    // One undo step restores the rgb value.
    call(&core, Method::ProjectUndo, json!({"project":reference()}));
    assert_eq!(reflectance().unwrap()["value_type"], "rgb");

    // Refusals: lineage, a slot that takes no texture, a non-texture.
    assert_eq!(
        reason(json!([{"kind":"connect_texture","target":resource(&wall),
            "parameter":"reflectance","texture":grime}])),
        "texture_lineage"
    );
    assert_eq!(
        reason(json!([{"kind":"connect_texture","target":resource(&wall),
            "parameter":"normalmap","texture":checks}])),
        "not_a_texture_input"
    );
    assert_eq!(
        reason(json!([{"kind":"connect_texture","target":resource(&wall),
            "parameter":"reflectance","texture":wall}])),
        "not_a_texture"
    );

    // Create and connect in one edit: the new texture takes the slot's
    // value type and the table's defaults.
    let applied = apply(json!([{"kind":"connect_texture","target":resource(&wall),
        "parameter":"reflectance","create":{"implementation":"marble","label":"Stone"}}]))
    .result
    .unwrap();
    let created = applied["created"]
        .as_array()
        .unwrap()
        .iter()
        .find(|item| item["kind"] == "resource")
        .unwrap()["id"]
        .clone();
    assert_eq!(reflectance().unwrap()["values"]["ids"], json!([created]));
    let stone = entry_of("Textures", "Stone");
    assert_eq!(stone["texture_value_type"], "spectrum");
    // A spectrum-only implementation (marble) cannot fill a float slot.
    assert_eq!(
        reason(json!([{"kind":"connect_texture","target":resource(&tint),
            "parameter":"scale","create":{"implementation":"marble","label":"Bad"}}])),
        "texture_value_type"
    );

    // Export writes the texture before the material that uses it.
    let export = call(
        &core,
        Method::ProjectExport,
        json!({"project":reference(),"destination":temp.path().join("bundle")}),
    );
    let job = wait(&core, export);
    assert_eq!(job["state"], "succeeded", "{job}");
    let text = fs::read_to_string(job["result"]["scene_path"].as_str().unwrap()).unwrap();
    assert!(
        text.contains("\"texture reflectance\" [ \"Stone\" ]"),
        "{text}"
    );

    // Disconnect removes the declaration: r4's default applies again.
    let applied = apply(
        json!([{"kind":"disconnect_texture","target":resource(&wall),
        "parameter":"reflectance"}]),
    );
    assert!(applied.error.is_none(), "{:?}", applied.error);
    assert!(reflectance().is_none());

    // A required operand stays: scale's `tex` cannot be disconnected.
    assert_eq!(
        reason(json!([{"kind":"disconnect_texture","target":resource(&tint),"parameter":"tex"}])),
        "required_parameter"
    );
    // Re-pointing an existing connection works in place.
    let applied = apply(json!([{"kind":"connect_texture","target":resource(&tint),
        "parameter":"tex","texture":created}]));
    assert!(applied.error.is_none(), "{:?}", applied.error);
    call(&core, Method::ProjectUndo, json!({"project":reference()}));
    // The graph says which slots connect_texture accepts.
    let graph = call(
        &core,
        Method::ProjectGetResourceGraph,
        json!({"project":reference(),"resource_id":tint}),
    );
    let inputs = graph["nodes"][0]["inputs"].as_array().unwrap().clone();
    assert!(
        inputs.iter().all(|input| input["connectable"] == true),
        "{inputs:?}"
    );
    // An empty folder for the created texture is refused up front.
    let error = apply(json!([{"kind":"connect_texture","target":resource(&wall),
        "parameter":"reflectance","create":{"implementation":"marble","label":"X","folder":""}}]))
    .error
    .unwrap();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    assert!(error.message.contains("folder"), "{}", error.message);

    // A cycle is refused whichever edit makes it: checks ← tint already,
    // so checks → tint closes the loop, through connect or set_parameter.
    assert_eq!(
        reason(json!([{"kind":"connect_texture","target":resource(&checks),
            "parameter":"tex1","texture":tint}])),
        "resource_cycle"
    );
    assert_eq!(
        reason(
            json!([{"kind":"set_parameter","target":resource(&checks),"name":"tex1",
            "value_type":"texture","value":{"name":"tex1","value_type":"texture","color_space":null,
            "values":{"kind":"resources","value":{"kind":"texture","ids":[tint]}}}}])
        ),
        "resource_cycle"
    );
}

#[test]
fn ply_assets_report_their_mesh_and_shapes_their_topology() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("tri.ply"),
        concat!(
            "ply\nformat ascii 1.0\nelement vertex 3\n",
            "property float x\nproperty float y\nproperty float z\n",
            "property float nx\nproperty float ny\nproperty float nz\n",
            "element face 1\nproperty list uchar int vertex_indices\nend_header\n",
            "0 0 0 0 0 1\n1 0 0 0 0 1\n0 1 0 0 0 1\n3 0 1 2\n",
        ),
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nShape \"plymesh\" \"string filename\" \"tri.ply\"\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":reference(),"folder":"Meshes"}),
    );
    let ply = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["media_type"] == "application/x-ply")
        .unwrap_or_else(|| panic!("{contents}"))["id"]
        .clone();
    let asset = call(
        &core,
        Method::ProjectGetAsset,
        json!({"project":reference(),"asset_id":ply}),
    );
    assert_eq!(
        asset["mesh"],
        json!({"vertices":3,"faces":1,"normals":true,"uvs":false,"format":"ascii"})
    );
    // The Inspector's Shape box: the topology from the PLY header.
    let roots = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference()}),
    );
    let world = roots["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["label"] == "World")
        .unwrap()["node_id"]
        .clone();
    let children = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(),"parent":world}),
    );
    let shape = children["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "entity")
        .unwrap()["node_id"]
        .clone();
    let node = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference(),"node_id":shape}),
    );
    let body = node["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == "shape")
        .unwrap()
        .clone();
    assert_eq!(body["statistics"]["vertices"], 3);
    assert_eq!(body["statistics"]["faces"], 1);
    assert_eq!(body["statistics"]["normals"], true);
}

#[test]
fn texture_previews_are_cached_by_their_upstream() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"checks\" \"spectrum\" \"checkerboard\" \"float uscale\" 4 \"float vscale\" 4\n",
            "Texture \"tint\" \"spectrum\" \"scale\" \"texture tex\" \"checks\" \"float scale\" 0.5\n",
            "Shape \"sphere\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let id_of = |name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":"Textures"}),
        );
        contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let (checks, tint) = (id_of("checks"), id_of("tint"));
    let preview = |id: &Value, size: Value| {
        call(
            &core,
            Method::ProjectPreviewTexture,
            json!({"project":reference(),"resource_id":id,"size":size}),
        )
    };
    let first = preview(&tint, json!(32));
    assert_eq!(first["cached"], false);
    let path = first["path"].as_str().unwrap().to_owned();
    let image = image::open(&path).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (32, 32));
    // 4×4 checks scaled by 0.5: white cells read linear 0.5 (sRGB 188),
    // the others black.
    let mut values = image.pixels().map(|p| p[0]).collect::<Vec<_>>();
    values.sort();
    values.dedup();
    assert_eq!(values, [0, 188], "{values:?}");
    // The same inputs hit the cache.
    let again = preview(&tint, json!(32));
    assert_eq!(again["cached"], true);
    assert_eq!(again["key"], first["key"]);
    fs::write(&path, b"broken png").unwrap();
    let repaired = preview(&tint, json!(32));
    assert_eq!(repaired["cached"], false);
    assert_eq!(repaired["key"], first["key"]);
    assert_eq!(
        image::open(&path).unwrap().to_rgba8().dimensions(),
        (32, 32)
    );
    // Editing the upstream texture makes a new preview.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[{"kind":"set_parameter",
            "target":{"kind":"resource","id":checks},"name":"uscale","value_type":"float",
            "value":{"name":"uscale","value_type":"float","color_space":null,
                     "values":{"kind":"floats","value":[2.0]}}}]}),
    );
    let edited = preview(&tint, json!(32));
    assert_eq!(edited["cached"], false);
    assert_ne!(edited["key"], first["key"]);
    // Only textures preview.
    let error = core
        .request(request(
            Method::ProjectPreviewTexture,
            json!({"project":reference(),"resource_id":uuid::Uuid::new_v4()}),
        ))
        .error
        .unwrap();
    assert_eq!(error.code, ErrorCode::NotFound);
}

/// B: an image asset's preview — decoded like the desktop Image Viewer shows
/// it (exr and hdr pass Reinhard → sRGB, plain formats keep their stored
/// pixels), fit into a square with a transparent letterbox, and cached by
/// the asset's content.
#[test]
fn asset_previews_fit_images_into_a_transparent_square() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(
        temp.path().join("viewer-ldr.png"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../desktop/fixtures/viewer-ldr.png"
        )),
    )
    .unwrap();
    fs::write(
        temp.path().join("viewer-hdr.exr"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../desktop/fixtures/viewer-hdr.exr"
        )),
    )
    .unwrap();
    fs::write(
        temp.path().join("viewer-huge.png"),
        include_bytes!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../desktop/fixtures/viewer-huge.png"
        )),
    )
    .unwrap();
    fs::write(
        temp.path().join("tri.ply"),
        concat!(
            "ply\nformat ascii 1.0\nelement vertex 3\n",
            "property float x\nproperty float y\nproperty float z\n",
            "property float nx\nproperty float ny\nproperty float nz\n",
            "element face 1\nproperty list uchar int vertex_indices\nend_header\n",
            "0 0 0 0 0 1\n1 0 0 0 0 1\n0 1 0 0 0 1\n3 0 1 2\n",
        ),
    )
    .unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"ldr\" \"spectrum\" \"imagemap\" \"string filename\" \"viewer-ldr.png\"\n",
            "Texture \"hdr\" \"spectrum\" \"imagemap\" \"string filename\" \"viewer-hdr.exr\"\n",
            "Texture \"huge\" \"spectrum\" \"imagemap\" \"string filename\" \"viewer-huge.png\"\n",
            "Shape \"plymesh\" \"string filename\" \"tri.ply\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let asset_named = |name: &str| {
        all_contents(&core, &reference())
            .iter()
            .find(|entry| entry["kind"] == "asset" && entry["name"] == name)
            .unwrap_or_else(|| panic!("no asset named {name}"))["id"]
            .clone()
    };
    let (ldr, hdr, huge, ply) = (
        asset_named("viewer-ldr.png"),
        asset_named("viewer-hdr.exr"),
        asset_named("viewer-huge.png"),
        asset_named("tri.ply"),
    );
    let preview = |id: &Value, size: Value| {
        call(
            &core,
            Method::ProjectPreviewAsset,
            json!({"project":reference(),"asset_id":id,"size":size}),
        )
    };
    let refuse = |id: Value, size: Value, code: ErrorCode, needle: &str| {
        let error = core
            .request(request(
                Method::ProjectPreviewAsset,
                json!({"project":reference(),"asset_id":id,"size":size}),
            ))
            .error
            .unwrap();
        assert_eq!(error.code, code, "{error:?}");
        assert!(error.message.contains(needle), "{error:?}");
    };
    // The png fixture (64×48) inside a 64×64 square: content lands on rows
    // 8..56 with its stored pixels, and the letterbox rows stay transparent.
    let first = preview(&ldr, json!(64));
    assert_eq!(first["cached"], false);
    assert_eq!(first["asset_id"], ldr);
    let path = first["path"].as_str().unwrap().to_owned();
    let image = image::open(&path).unwrap().to_rgba8();
    assert_eq!(image.dimensions(), (64, 64));
    assert_eq!(image.get_pixel(0, 0)[3], 0);
    assert_eq!(image.get_pixel(0, 63)[3], 0);
    assert_eq!(image.get_pixel(0, 8), &image::Rgba([0, 128, 255, 255]));
    assert_eq!(image.get_pixel(63, 8), &image::Rgba([252, 128, 3, 255]));
    // The same inputs hit the cache; a corrupted entry regenerates.
    let again = preview(&ldr, json!(64));
    assert_eq!(again["cached"], true);
    assert_eq!(again["key"], first["key"]);
    fs::write(&path, b"broken png").unwrap();
    let repaired = preview(&ldr, json!(64));
    assert_eq!(repaired["cached"], false);
    assert_eq!(repaired["key"], first["key"]);
    assert_eq!(
        image::open(&path).unwrap().to_rgba8().get_pixel(0, 8),
        &image::Rgba([0, 128, 255, 255])
    );
    // The exr fixture tone maps like the Image Viewer, and different
    // content gets a different key.
    let exr = preview(&hdr, json!(64));
    assert_eq!(exr["cached"], false);
    assert_ne!(exr["key"], first["key"]);
    let exr_image = image::open(exr["path"].as_str().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(exr_image.get_pixel(0, 8), &image::Rgba([0, 124, 231, 255]));
    assert_eq!(
        exr_image.get_pixel(63, 8),
        &image::Rgba([242, 124, 231, 255])
    );
    // Oversized input is refused from the header before decoding, sizes
    // outside 8..=512 and non-image assets never generate, and unknown ids
    // are not found.
    refuse(huge, json!(64), ErrorCode::InvalidRequest, "too large");
    refuse(
        ldr.clone(),
        json!(7),
        ErrorCode::InvalidRequest,
        "preview size",
    );
    refuse(ldr, json!(513), ErrorCode::InvalidRequest, "preview size");
    refuse(
        ply,
        serde_json::Value::Null,
        ErrorCode::InvalidRequest,
        "image assets only",
    );
    refuse(
        json!(uuid::Uuid::new_v4()),
        serde_json::Value::Null,
        ErrorCode::NotFound,
        "asset not found",
    );
}

#[test]
fn material_previews_shade_a_sphere_and_follow_their_textures() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        concat!(
            "WorldBegin\n",
            "Texture \"checks\" \"spectrum\" \"checkerboard\" \"float uscale\" 8 \"float vscale\" 8\n",
            "MakeNamedMaterial \"red\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.8 0.1 0.1]\n",
            "MakeNamedMaterial \"checked\" \"string type\" \"diffuse\" \"texture reflectance\" \"checks\"\n",
            "MakeNamedMaterial \"both\" \"string type\" \"mix\" \"string materials\" [\"red\" \"checked\"]\n",
            "NamedMaterial \"both\"\n",
            "Shape \"sphere\"\n",
        ),
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let id_of = |folder: &str, name: &str| {
        let contents = call(
            &core,
            Method::ProjectListContents,
            json!({"project":reference(),"folder":folder}),
        );
        contents["entries"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
            .unwrap()["id"]
            .clone()
    };
    let (checks, red, both) = (
        id_of("Textures", "checks"),
        id_of("Materials", "red"),
        id_of("Materials", "both"),
    );
    let preview = |id: &Value| {
        call(
            &core,
            Method::ProjectPreviewMaterial,
            json!({"project":reference(),"resource_id":id,"size":32}),
        )
    };
    let first = preview(&red);
    assert_eq!(first["cached"], false);
    let image = image::open(first["path"].as_str().unwrap())
        .unwrap()
        .to_rgba8();
    assert_eq!(image.dimensions(), (32, 32));
    // The centre is the red sphere; the corner is the grey backdrop.
    let centre = image.get_pixel(16, 16);
    assert!(centre[0] > 2 * centre[1], "{centre:?}");
    let corner = image.get_pixel(0, 0);
    assert_eq!(corner[0], corner[1]);
    assert_eq!(preview(&red)["cached"], true);
    // Editing a texture a mix input reads makes a new mix preview.
    let mixed = preview(&both);
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(),"edits":[{"kind":"set_parameter",
            "target":{"kind":"resource","id":checks},"name":"uscale","value_type":"float",
            "value":{"name":"uscale","value_type":"float","color_space":null,
                     "values":{"kind":"floats","value":[2.0]}}}]}),
    );
    let edited = preview(&both);
    assert_eq!(edited["cached"], false);
    assert_ne!(edited["key"], mixed["key"]);
    // Only materials preview here.
    let error = core
        .request(request(
            Method::ProjectPreviewMaterial,
            json!({"project":reference(),"resource_id":checks}),
        ))
        .error
        .unwrap();
    assert_eq!(error.code, ErrorCode::InvalidRequest, "{error:?}");
    assert_eq!(error.message, "not a material");
}

#[test]
fn list_references_reports_where_items_are_used() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "WorldBegin\nAttributeBegin\nShape \"sphere\"\nShape \"sphere\"\nAttributeEnd\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let reference = || state(&core)["context"]["project"].clone();
    let world = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference()}),
    )["nodes"][0]["node_id"]
        .clone();
    let group = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(), "parent": world}),
    )["nodes"][0]["node_id"]
        .clone();

    // One bound material, one mix material with a connected texture.
    let id_of_first = || {
        call(
            &core,
            Method::ProjectApply,
            json!({"project":reference(), "edits":[
                {"kind":"create_resource","resource_kind":"material",
                 "implementation":"conductor","label":"Copper"}]}),
        )["created"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let copper = id_of_first();
    let alloy = {
        let _ = &copper;
        call(
            &core,
            Method::ProjectApply,
            json!({"project":reference(), "edits":[
                {"kind":"create_resource","resource_kind":"material",
                 "implementation":"mix","label":"Alloy"}]}),
        )["created"][0]["id"]
            .as_str()
            .unwrap()
            .to_owned()
    };
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(), "edits":[
            {"kind":"set_local_binding","node_id":group,
             "material":{"kind":"resource","value":copper},"medium":null}]}),
    );
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(), "edits":[
            {"kind":"connect_texture","target":{"kind":"resource","id":alloy},
             "parameter":"amount",
             "create":{"implementation":"scale","label":"Scaled"}}]}),
    )["created"]
        .as_array()
        .unwrap()
        .clone();
    let scaled = applied[0]["id"].as_str().unwrap().to_owned();

    // The texture is used by the mix material's parameter.
    let list = call(
        &core,
        Method::ProjectListReferences,
        json!({
        "project":reference(), "item":{"kind":"resource","id":scaled}}),
    );
    assert_eq!(list["total"], 1);
    let row = &list["references"][0];
    assert_eq!(row["kind"], "resource");
    assert_eq!(row["resource"]["label"], "Alloy");
    assert_eq!(row["via"], "parameter:amount");

    // The material is used twice: once by the group, once more by its first
    // shape directly - owned-scope flattening attaches a shape's local
    // binding to the shape itself, so paging has two pages to page through.
    let first_shape = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference(), "parent":group}),
    )["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "entity")
        .expect("a shape")["node_id"]
        .clone();
    call(
        &core,
        Method::ProjectApply,
        json!({"project":reference(), "edits":[
            {"kind":"set_local_binding","node_id":first_shape,
             "material":{"kind":"resource","value":copper},"medium":null}]}),
    );
    let list = call(
        &core,
        Method::ProjectListReferences,
        json!({
        "project":reference(), "item":{"kind":"resource","id":copper}}),
    );
    assert_eq!(list["total"], 2, "{list:?}");
    assert_eq!(list["references"].as_array().unwrap().len(), 2);
    let rows = list["references"].as_array().unwrap().clone();
    assert!(
        rows.iter().all(|row| row["kind"] == "node"
            && row["via"] == "material"
            && row["scene"]["label"].is_string()),
        "the bindings name their holders, via material: {rows:?}"
    );
    let holders: std::collections::BTreeSet<String> = rows
        .iter()
        .map(|row| row["node"]["object"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        holders,
        std::collections::BTreeSet::from(["group".into(), "shape".into()]),
        "the group and the shape each hold their own binding: {rows:?}"
    );
    assert_ne!(
        rows[0]["node"]["id"], rows[1]["node"]["id"],
        "two distinct holders"
    );
    let page_one = call(
        &core,
        Method::ProjectListReferences,
        json!({
        "project":reference(), "item":{"kind":"resource","id":copper}, "limit":1}),
    );
    assert_eq!(page_one["references"].as_array().unwrap().len(), 1);
    assert_eq!(page_one["next_offset"], 1);
    let page_two = call(
        &core,
        Method::ProjectListReferences,
        json!({
        "project":reference(), "item":{"kind":"resource","id":copper},
        "offset":1, "limit":1}),
    );
    assert_eq!(page_two["references"].as_array().unwrap().len(), 1);
    assert_eq!(page_two["next_offset"], Value::Null);
    assert_ne!(
        page_one["references"][0]["node"]["id"], page_two["references"][0]["node"]["id"],
        "the second page holds the other reference"
    );

    // A scene is never referenced by anything (the model carries no such
    // link), and an item that does not exist is not found.
    let empty = call(
        &core,
        Method::ProjectListReferences,
        json!({
        "project":reference(), "item":{"kind":"scene","id":state(&core)["active_scene"]}}),
    );
    assert_eq!(empty["total"], 0);
    let missing = core.request(request(
        Method::ProjectListReferences,
        json!({"project":reference(), "item":{"kind":"resource","id":uuid::Uuid::new_v4()}}),
    ));
    assert_eq!(missing.error.unwrap().code, ErrorCode::NotFound);

    // The delete refusal names the same rows.
    let refused = core.request(request(
        Method::ProjectApply,
        json!({"project":reference(), "edits":[
            {"kind":"delete_item","item":{"kind":"resource","id":copper}}]}),
    ));
    let error = refused.error.unwrap();
    assert_eq!(error.code, ErrorCode::InvalidRequest);
    let details = error.details.unwrap();
    assert_eq!(details["reason"], "referenced");
    assert_eq!(details["references"]["count"], 2);
    let items = details["references"]["items"].as_array().unwrap().clone();
    assert!(
        items.len() >= 2
            && items.iter().all(|row| row["via"] == "material")
            && items
                .iter()
                .map(|row| row["node"]["object"].as_str().unwrap())
                .collect::<std::collections::BTreeSet<_>>()
                == std::collections::BTreeSet::from(["group", "shape"]),
        "the refusal's rows name their holders: {items:?}"
    );
    drop(host);
}

#[test]
fn format_two_opens_in_compatibility_mode_and_save_as_keeps_shared_images() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("old");
    let pbrt = temp.path().join("old.pbrt");
    fs::write(temp.path().join("image.png"), b"image bytes").unwrap();
    fs::write(&pbrt, "WorldBegin\nTexture \"first\" \"float\" \"imagemap\" \"string filename\" \"image.png\"\nTexture \"second\" \"spectrum\" \"imagemap\" \"string filename\" \"image.png\"\nShape \"sphere\"\n").unwrap();
    let mut old = project::FolderStore::create(
        &source,
        project::ProjectId::new(),
        "Old",
        project::Revision::initial(),
    )
    .unwrap();
    let (session, _) =
        pbrt_io::import_into(&pbrt, &mut old, &Default::default(), || false).unwrap();
    let public_ids = session
        .document()
        .resources
        .keys()
        .map(ToString::to_string)
        .collect::<std::collections::BTreeSet<_>>();
    let original_manifest = fs::read(source.join("project.pbrtui.json")).unwrap();
    drop(old);
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let opened = call(
        &core,
        Method::ProjectOpen,
        json!({"path":source,"expected_current":state(&core)["context"]}),
    );
    assert_eq!(wait(&core, opened)["state"], "succeeded");
    let before = state(&core);
    assert_eq!(before["read_only"], true);
    let saved = call(
        &core,
        Method::ProjectSave,
        json!({"project":before["context"]["project"],"destination":temp.path().join("converted")}),
    );
    let result = wait(&core, saved);
    assert_eq!(result["state"], "succeeded", "{result}");
    let after = state(&core);
    assert_eq!(after["read_only"], false);
    assert_eq!(after["dirty"], false);
    let entries = all_contents(&core, &after["context"]["project"]);
    let textures = entries
        .iter()
        .filter(|entry| entry["resource_kind"] == "texture")
        .collect::<Vec<_>>();
    assert_eq!(textures.len(), 2);
    assert_eq!(
        textures
            .iter()
            .map(|entry| entry["id"].as_str().unwrap().to_owned())
            .collect::<std::collections::BTreeSet<_>>(),
        public_ids
    );
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry["kind"] == "asset")
            .count(),
        1,
        "both textures share the one image"
    );
    assert_eq!(
        fs::read(source.join("project.pbrtui.json")).unwrap(),
        original_manifest
    );
}

#[test]
fn project_create_builds_an_editable_project_with_an_initial_scene() {
    let temp = tempfile::tempdir().unwrap();
    // Core reports canonical paths; macOS temp dirs sit behind the /var symlink.
    let root = protocol::path::canonicalize(temp.path()).unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let destination = root.join("Fresh");
    let job = call(
        &core,
        Method::ProjectCreate,
        json!({
            "path":destination,
            "title":"Fresh",
            "coordinate_system":{"up":"+Z","handedness":"right"},
            "expected_current":state(&core)["context"],
        }),
    );
    let result = wait(&core, job);
    assert_eq!(result["state"], "succeeded", "{result}");
    let created = &result["result"];
    assert_eq!(created["path"], destination.to_str().unwrap());
    assert_eq!(created["mode"], "project");
    assert_eq!(
        created["report"],
        Value::Null,
        "creation is not an import; it carries no report"
    );
    let created_state = state(&core);
    assert_eq!(created_state["mode"], "project");
    assert_eq!(created_state["title"], "Fresh");
    assert_eq!(created_state["path"], destination.to_str().unwrap());
    assert_eq!(created_state["read_only"], false);
    // the viewport reads the active scene's up from the state.
    assert_eq!(created_state["active_up"], "+Z");
    assert_eq!(created_state["active_handedness"], "right");
    let reference = created_state["context"]["project"].clone();

    // The initial scene: one posed render Camera, a Floor and an
    // Environment Light, on the requested coordinate system.
    let scene_root = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference,"node_id":created_state["default_scene"]}),
    );
    assert_eq!(
        scene_root["data"]["coordinate_system"],
        json!({"up":"+Z","handedness":"right"}),
        "the requested coordinate system holds: {scene_root}"
    );
    let world = call(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":created_state["default_scene"],"view":"scene_graph"}),
    )["nodes"]
        .as_array()
        .unwrap()
        .clone();
    let labels: Vec<&str> = world
        .iter()
        .map(|node| node["label"].as_str().unwrap())
        .collect();
    assert!(labels.contains(&"Camera"), "{labels:?}");
    assert!(labels.contains(&"Floor"), "{labels:?}");
    assert!(labels.contains(&"Environment Light"), "{labels:?}");
    let render_settings = scene_root["components"]
        .as_array()
        .unwrap()
        .iter()
        .find(|component| component["kind"] == "render_settings")
        .unwrap()
        .clone();
    assert_eq!(render_settings["cameras"].as_array().unwrap().len(), 1);

    // the render trio is in the Options — all three on the Scene
    // root — with quality values as declared parameters.
    let component = |components: &Value, kind: &str| {
        components
            .as_array()
            .unwrap()
            .iter()
            .find(|component| component["kind"] == kind)
            .unwrap_or_else(|| panic!("no {kind} box: {components}"))
            .clone()
    };
    let parameter_names = |target: &Value| {
        let reply = call(
            &core,
            Method::ProjectGetParameters,
            json!({"project":reference,"target":target}),
        );
        reply["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .map(|parameter| {
                (
                    parameter["name"].as_str().unwrap().to_owned(),
                    parameter["values"].clone(),
                )
            })
            .collect::<Vec<_>>()
    };
    let sampler = component(&scene_root["components"], "sampler");
    assert_eq!(sampler["implementation"], "zsobol");
    assert_eq!(
        parameter_names(&sampler["parameter_target"]),
        [(
            "pixelsamples".to_owned(),
            json!({"kind":"integers","value":["64"]})
        )]
    );
    let integrator = component(&scene_root["components"], "integrator");
    assert_eq!(integrator["implementation"], "volpath");
    assert_eq!(
        parameter_names(&integrator["parameter_target"]),
        [(
            "maxdepth".to_owned(),
            json!({"kind":"integers","value":["5"]})
        )]
    );
    let camera_id = render_settings["camera"].clone();
    let camera = call(
        &core,
        Method::ProjectGetNode,
        json!({"project":reference,"node_id":camera_id}),
    );
    assert!(
        !camera["components"]
            .as_array()
            .unwrap()
            .iter()
            .any(|component| component["kind"] == "film"),
        "the render camera reports no Film (stage U-e): {camera}"
    );
    let film = component(&scene_root["components"], "film");
    assert_eq!(film["implementation"], "rgb");
    assert_eq!(
        parameter_names(&film["parameter_target"]),
        [
            (
                "filename".to_owned(),
                json!({"kind":"strings","value":["Fresh.exr"]})
            ),
            (
                "xresolution".to_owned(),
                json!({"kind":"integers","value":["640"]})
            ),
            (
                "yresolution".to_owned(),
                json!({"kind":"integers","value":["360"]})
            ),
        ]
    );

    // The new project saves and reopens like any folder project.
    call(&core, Method::ProjectSave, json!({"project":reference}));
    // Close and open the same folder again.
    call(
        &core,
        Method::ProjectClose,
        json!({"expected_current":state(&core)["context"],"discard_unsaved":false}),
    );
    let context = state(&core)["context"].clone();
    let job = call(
        &core,
        Method::ProjectOpen,
        json!({"path":destination,"expected_current":context}),
    );
    let result = wait(&core, job);
    assert_eq!(result["state"], "succeeded", "{result}");
    let reopened = state(&core);
    assert_eq!(reopened["mode"], "project");
    assert_eq!(reopened["title"], "Fresh");
    assert_eq!(reopened["dirty"], false);

    // Edits work: the mode is editable, not a viewer.
    let scene_id = reopened["active_scene"].clone();
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":reopened["context"]["project"],
               "edits":[{"kind":"set_scene_coordinate_system","scene":scene_id,"up":"+Y","handedness":"left"}]}),
    );
    assert!(applied["nodes"].is_array(), "apply works: {applied}");
    assert_eq!(state(&core)["active_up"], "+Y");
}

/// a scene without Film, Sampler or Integrator (an older import)
/// takes them through the wire `create_configuration` edit, the created
/// nodes edit and delete like other components, and the export carries the
/// edited values.
#[test]
fn create_configuration_adds_render_entities_that_export_with_edits() {
    let temp = tempfile::tempdir().unwrap();
    let source = temp.path().join("scene.pbrt");
    fs::write(
        &source,
        "Camera \"perspective\" \"float fov\" [30]\nWorldBegin\nShape \"sphere\"\n",
    )
    .unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    import(&core, &source, &temp.path().join("project"));
    let project = || state(&core)["context"]["project"].clone();
    let scene_id = state(&core)["active_scene"].clone();

    // The Scene root reports none of the five render entities — the
    // existence map is what says there is none of each kind.
    let root = |component: &str| {
        call(
            &core,
            Method::ProjectGetNode,
            json!({"project":project(),"node_id":scene_id}),
        )["components"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["kind"] == component)
            .cloned()
            .unwrap()
    };
    assert_eq!(
        root("render_settings")["configurations"],
        json!({"film":false,"sampler":false,"integrator":false,
               "pixel_filter":false,"accelerator":false})
    );

    // The default implementations, halton picked explicitly, in PBRT
    // order after the camera; the reply reports all three nodes.
    let applied = call(
        &core,
        Method::ProjectApply,
        json!({"project":project(),"edits":[
            {"kind":"create_configuration","scene":scene_id,"configuration":"film"},
            {"kind":"create_configuration","scene":scene_id,
             "configuration":"sampler","implementation":"halton"},
            {"kind":"create_configuration","scene":scene_id,"configuration":"integrator"},
        ]}),
    );
    let created = applied["created"].as_array().unwrap().clone();
    assert_eq!(created.len(), 3, "{applied}");
    assert_eq!(
        root("render_settings")["configurations"],
        json!({"film":true,"sampler":true,"integrator":true,
               "pixel_filter":false,"accelerator":false})
    );

    // The created nodes edit like any component: values go in through
    // set_parameter (the created film defaults its name to r4's pbrt.exr
    // until the user writes one).
    let set = |node: usize, name: &str, value_json: Value| {
        let id = created[node]["id"].clone();
        let edit = json!({"kind":"set_parameter",
            "target":{"kind":"node","id":id},"name":name,"value_type":"integer",
            "value":{"name":name,"value_type":"integer","color_space":null,
                     "values":value_json}});
        core.request(request(
            Method::ProjectApply,
            json!({"project":project(),"edits":[edit]}),
        ))
    };
    assert!(
        set(0, "xresolution", json!({"kind":"integers","value":["320"]}))
            .error
            .is_none()
    );
    assert!(
        set(0, "yresolution", json!({"kind":"integers","value":["180"]}))
            .error
            .is_none()
    );
    assert!(
        set(1, "pixelsamples", json!({"kind":"integers","value":["8"]}))
            .error
            .is_none()
    );
    assert!(
        set(2, "maxdepth", json!({"kind":"integers","value":["3"]}))
            .error
            .is_none()
    );
    let film_name = created[0]["id"].clone();
    let edit = json!({"kind":"set_parameter",
        "target":{"kind":"node","id":film_name},"name":"filename","value_type":"string",
        "value":{"name":"filename","value_type":"string","color_space":null,
                 "values":{"kind":"strings","value":["bright.exr"]}}});
    assert!(
        core.request(request(
            Method::ProjectApply,
            json!({"project":project(),"edits":[edit]}),
        ))
        .error
        .is_none()
    );

    // The export carries the created entities with the edited values. The
    // film's output name is redirected to render.exr like every film
    // (export.rs: "original declarations stay in the project") — the
    // declared bright.exr stays in the model.
    let bundle = temp.path().join("bundle");
    let job = call(
        &core,
        Method::ProjectExport,
        json!({"project":project(),"destination":bundle}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");
    let text = fs::read_to_string(bundle.join("scene.pbrt")).unwrap();
    assert!(text.contains("Film \"rgb\""), "{text}");
    assert!(
        text.contains("\"string filename\" [\"render.exr\"]"),
        "{text}"
    );
    assert!(text.contains("\"integer xresolution\" [ 320 ]"), "{text}");
    assert!(text.contains("\"integer yresolution\" [ 180 ]"), "{text}");
    assert!(text.contains("Sampler \"halton\""), "{text}");
    assert!(text.contains("\"integer pixelsamples\" [ 8 ]"), "{text}");
    assert!(text.contains("Integrator \"volpath\""), "{text}");
    assert!(text.contains("\"integer maxdepth\" [ 3 ]"), "{text}");
    let film_parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":project(),"target":{"kind":"node","id":film_name}}),
    );
    assert_eq!(
        film_parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "filename")
            .unwrap()["values"]["value"],
        json!(["bright.exr"]),
        "the declared output name stays in the project"
    );

    // Removing one is the same component delete: undo then restores it.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project(),"edits":[
            {"kind":"delete_node","node_id":created[1]["id"]}]}),
    );
    assert_eq!(
        root("render_settings")["configurations"],
        json!({"film":true,"sampler":false,"integrator":true,
               "pixel_filter":false,"accelerator":false})
    );
    call(&core, Method::ProjectUndo, json!({"project":project()}));
    assert_eq!(
        root("render_settings")["configurations"],
        json!({"film":true,"sampler":true,"integrator":true,
               "pixel_filter":false,"accelerator":false})
    );
    let parameters = call(
        &core,
        Method::ProjectGetParameters,
        json!({"project":project(),"target":{"kind":"node","id":created[1]["id"]}}),
    );
    assert_eq!(
        parameters["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "pixelsamples")
            .unwrap()["values"]["value"],
        json!(["8"]),
        "the restored Sampler keeps its edited value"
    );
    drop(host);
}

#[test]
fn project_create_accepts_only_a_missing_or_empty_directory() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let fresh = || state(&core)["context"].clone();

    // A non-empty directory is refused.
    let occupied = temp.path().join("occupied");
    fs::create_dir_all(&occupied).unwrap();
    fs::write(occupied.join("something.txt"), "not empty").unwrap();
    let job = call(
        &core,
        Method::ProjectCreate,
        json!({"path":occupied,"title":"Refused","expected_current":fresh()}),
    );
    let refused = wait(&core, job);
    assert_eq!(refused["state"], "failed", "{refused}");
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap()
            .to_lowercase()
            .contains("empty"),
        "{refused}"
    );
    assert!(
        !occupied.join("project.pbrtui.json").exists(),
        "nothing is written"
    );

    // An existing empty directory works.
    let empty = temp.path().join("existing-empty");
    fs::create_dir_all(&empty).unwrap();
    let job = call(
        &core,
        Method::ProjectCreate,
        json!({"path":empty,"title":"Seed","expected_current":fresh()}),
    );
    let created = wait(&core, job);
    assert_eq!(created["state"], "succeeded", "{created}");
    assert!(empty.join("project.pbrtui.json").is_file());

    // A second create over the now-populated folder is refused the same way.
    let job = call(
        &core,
        Method::ProjectCreate,
        json!({"path":empty,"title":"Second","expected_current":fresh()}),
    );
    let refused = wait(&core, job);
    assert_eq!(refused["state"], "failed", "{refused}");
}

#[test]
fn project_create_validates_titles_and_the_coordinate_system() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let context = state(&core)["context"].clone();
    let deny = |error: Value, text: &str| {
        let message = error["error"]["message"].as_str().unwrap_or_default();
        assert!(message.to_lowercase().contains(text), "{error}");
    };
    // Request-level refusals, before any job is queued.
    let response = core.request(request(
        Method::ProjectCreate,
        json!({"path":temp.path().join("a"),"title":"","expected_current":context}),
    ));
    assert!(response.error.is_some(), "{response:?}");
    deny(serde_json::to_value(&response).unwrap(), "title");
    let response = core.request(request(
        Method::ProjectCreate,
        json!({"path":temp.path().join("b"),"title":"two/segments","expected_current":context}),
    ));
    assert!(response.error.is_some(), "{response:?}");
    deny(serde_json::to_value(&response).unwrap(), "title");
    // Unknown axis words fail to parse as the wire enum.
    let response = core.request(request(
        Method::ProjectCreate,
        json!({"path":temp.path().join("c"),
               "title":"Words",
               "coordinate_system":{"up":"north","handedness":"left"},
               "expected_current":context}),
    ));
    assert!(response.error.is_some(), "{response:?}");
}

/// The colour dialog's own replace payloads — its
/// cross-type replacement (value_node_menu's SpectrumData item,
/// desktop parameter_rows::spectrum_connect_edits) — apply as ONE
/// transaction: one undo returns the slot's old value, not the
/// intermediate cleared state.
#[test]
fn a_spectrum_value_replacement_undoes_as_one_step() {
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start(Limits::default());
    let core = host.handle();
    let source = temp.path().join("gold.pbrt");
    fs::write(
        &source,
        "WorldBegin\nAttributeBegin\n\
         MakeNamedMaterial \"gold\" \"string type\" \"conductor\" \"rgb k\" [1 0.8 0.3]\n\
         Material \"gold\"\nShape \"sphere\"\nAttributeEnd\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    let project = || state(&core)["context"]["project"].clone();
    let contents = call(
        &core,
        Method::ProjectListContents,
        json!({"project":project(),"folder":"Materials"}),
    );
    let gold = contents["entries"]
        .as_array()
        .unwrap()
        .iter()
        .find(|entry| entry["kind"] == "resource" && entry["name"] == "gold")
        .unwrap()["id"]
        .clone();
    let target = json!({"kind":"resource","id":gold});
    let k = |core: &CoreHandle| -> (Value, Value) {
        let parameters = call(
            core,
            Method::ProjectGetParameters,
            json!({"project":project(),"target":target}),
        )["parameters"]
            .as_array()
            .unwrap()
            .iter()
            .find(|parameter| parameter["name"] == "k")
            .cloned()
            .unwrap();
        (
            parameters["value_type"].clone(),
            parameters["values"].clone(),
        )
    };
    let before = k(&core);
    assert_eq!(before.0, "rgb");
    // The menu's payload: remove the declared rgb, then declare the
    // blackbody — both edits in one apply.
    call(
        &core,
        Method::ProjectApply,
        json!({"project":project(),"edits":[
            {"kind":"set_parameter","target":target,"name":"k","value_type":"rgb","value":null},
            {"kind":"set_parameter","target":target,"name":"k","value_type":"blackbody",
             "value":{"name":"k","value_type":"blackbody",
                      "values":{"kind":"floats","value":[6500.]}}},
        ]}),
    );
    assert_eq!(k(&core).0, "blackbody");
    // One undo: the rgb triple as it was.
    call(&core, Method::ProjectUndo, json!({"project":project()}));
    assert_eq!(k(&core), before);
}
