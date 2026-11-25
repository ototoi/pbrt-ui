//! Unix-only: every test drives a shell-script fake renderer.
#![cfg(unix)]
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
outfile = Path(sys.argv[sys.argv.index('--outfile') + 1])
with program.with_suffix('.calls').open('a') as handle:
    handle.write(outfile.parent.name + '\n')
deadline = time.monotonic() + 15
while not program.with_suffix('.release').exists():
    if time.monotonic() > deadline:
        raise RuntimeError('test did not release renderer')
    time.sleep(0.01)
outfile.write_bytes(bytes([0x76,0x2f,0x31,0x01]))
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

fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        method,
        params,
    }
}

fn call(core: &CoreHandle, method: Method, params: Value) -> Value {
    let response = core.request(request(method, params));
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

fn state(core: &CoreHandle) -> Value {
    call(core, Method::ProjectGetState, json!({}))
}

fn ipr_status(core: &CoreHandle) -> Value {
    call(core, Method::AppGetState, json!({}))["ipr"].clone()
}

fn wait(core: &CoreHandle, job: Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let value = call(core, Method::JobGet, json!({"job_id":job["job_id"]}));
        if ["succeeded", "failed", "cancelled"].contains(&value["state"].as_str().unwrap()) {
            return value;
        }
        assert!(Instant::now() < deadline, "job did not terminate: {value}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn import(core: &CoreHandle, path: &std::path::Path, destination: &std::path::Path) {
    let job = call(
        core,
        Method::ProjectImport,
        json!({"source_path":path,"destination":destination,"expected_current":state(core)["context"]}),
    );
    let result = wait(core, job);
    assert_eq!(result["state"], "succeeded", "{result}");
}

fn ipr_jobs(core: &CoreHandle) -> Vec<Value> {
    call(core, Method::JobList, json!({"limit":50}))["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["kind"] == "ipr")
        .cloned()
        .collect()
}

fn wait_ipr_job(core: &CoreHandle, exclude: &[String]) -> Value {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        for job in ipr_jobs(core) {
            let id = job["job_id"].as_str().unwrap().to_string();
            if !exclude.contains(&id) {
                return job;
            }
        }
        assert!(Instant::now() < deadline, "no ipr job appeared");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn calls(directory: &std::path::Path) -> String {
    fs::read_to_string(directory.join("renderer.calls")).unwrap_or_default()
}

fn wait_calls(directory: &std::path::Path, job_id: &str) {
    let deadline = Instant::now() + Duration::from_secs(15);
    while !calls(directory).contains(job_id) {
        assert!(
            Instant::now() < deadline,
            "renderer did not run for {job_id}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn enable_ipr(core: &CoreHandle, debounce_ms: u64) {
    let status = call(
        core,
        Method::RenderIprSet,
        json!({"enabled":true,"settings":{"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":debounce_ms}}),
    );
    assert_eq!(status["enabled"], true);
}

fn set_transform(core: &CoreHandle, x: i64) {
    let reference = state(core)["context"]["project"].clone();
    let roots = call(core, Method::ProjectListNodes, json!({"project":reference}));
    // The scene's roots may open with a Camera entity (it lives in Options
    // but lists under the scene), so find the World scope explicitly.
    let world = roots["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["label"] == "World")
        .expect("a World scope");
    let children = call(
        core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":world["node_id"]}),
    );
    // A world-direct transform import lands in a wrapper group, so the
    // Translate state sits one level down.
    let translate_in = |children: &Value| {
        children["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["label"] == "Translate")
            .map(|node| node["node_id"].clone())
    };
    let transform = match translate_in(&children) {
        Some(id) => id,
        None => {
            let nested = call(
                core,
                Method::ProjectListNodes,
                json!({"project":reference,"parent":children["nodes"][0]["node_id"]}),
            );
            translate_in(&nested).expect("the fixture's Translate state")
        }
    };
    call(
        core,
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"set_transform","node_id":transform,"value":{"kind":"translate","value":[x,0,0]}}]}),
    );
}

#[cfg(unix)]
#[test]
fn edits_inside_the_debounce_window_render_one_preview_with_no_retained_artifacts() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nTranslate 1 0 0\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    assert_eq!(ipr_status(&core)["enabled"], false);
    enable_ipr(&core, 150);
    set_transform(&core, 2);
    set_transform(&core, 3);
    let first = wait_ipr_job(&core, &[]);
    let preview = wait(&core, first);
    assert_eq!(preview["state"], "succeeded", "{preview}");
    assert_eq!(preview["kind"], "ipr");
    assert_eq!(preview["result"]["ipr"], true, "{preview}");
    assert!(
        preview["artifacts"].as_array().unwrap().is_empty(),
        "previews keep no persistent artifacts: {preview}"
    );
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(
        ipr_jobs(&core).len(),
        1,
        "two edits launched one coalesced preview"
    );
    assert_eq!(
        preview["project"]["revision"],
        state(&core)["context"]["project"]["revision"]
    );
    assert_eq!(ipr_status(&core)["active_job_id"], Value::Null);
}

#[cfg(unix)]
#[test]
fn a_final_render_preempts_a_preview_and_the_latest_state_relaunches_after_it() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    // A render camera: since render settings became explicit, ProjectRender
    // refuses a scene that has no camera to render through.
    fs::write(
        &source,
        "Camera \"perspective\" \"float fov\" [45]\nWorldBegin\nTranslate 1 0 0\nShape \"sphere\"\n",
    )
    .unwrap();
    import(&core, &source, &temp.path().join("project"));
    enable_ipr(&core, 100);
    set_transform(&core, 2);
    let first = wait_ipr_job(&core, &[]);
    let preview_id = first["job_id"].as_str().unwrap().to_string();
    wait_calls(temp.path(), &preview_id);
    let reference = state(&core)["context"]["project"].clone();
    let settings_revision =
        call(&core, Method::AppGetState, json!({}))["settings_revision"].clone();
    let final_job = call(
        &core,
        Method::ProjectRender,
        json!({"project":reference,"expected_settings_revision":settings_revision}),
    );
    let preview = wait(&core, first);
    assert_eq!(preview["state"], "cancelled", "{preview}");
    wait_calls(temp.path(), final_job["job_id"].as_str().unwrap());
    set_transform(&core, 4);
    let deadline = Instant::now() + Duration::from_secs(15);
    while ipr_status(&core)["waiting_for_final_render"] != true {
        assert!(
            Instant::now() < deadline,
            "ipr never waited for the final render"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(ipr_jobs(&core).len(), 1, "no relaunch while the final runs");
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    let finished = wait(&core, final_job);
    assert_eq!(finished["state"], "succeeded", "{finished}");
    assert_eq!(finished["kind"], "render");
    assert!(
        !finished["artifacts"].as_array().unwrap().is_empty(),
        "the final render keeps artifacts: {finished}"
    );
    let second = wait_ipr_job(&core, &[preview_id]);
    let relaunch = wait(&core, second);
    assert_eq!(relaunch["state"], "succeeded", "{relaunch}");
    assert_eq!(relaunch["kind"], "ipr");
    assert!(relaunch["artifacts"].as_array().unwrap().is_empty());
    assert_eq!(
        relaunch["project"]["revision"],
        state(&core)["context"]["project"]["revision"],
        "the relaunch renders the latest revision"
    );
    assert_eq!(
        ipr_jobs(&core).len(),
        2,
        "the preempted state relaunched exactly once"
    );
}

#[cfg(unix)]
#[test]
fn switching_ipr_off_cancels_the_running_preview_and_stays_quiet() {
    let temp = tempfile::tempdir().unwrap();
    let host = render_host(temp.path());
    let core = host.handle();
    let source = temp.path().join("source.pbrt");
    fs::write(&source, "WorldBegin\nTranslate 1 0 0\nShape \"sphere\"\n").unwrap();
    import(&core, &source, &temp.path().join("project"));
    enable_ipr(&core, 100);
    set_transform(&core, 2);
    let first = wait_ipr_job(&core, &[]);
    wait_calls(temp.path(), first["job_id"].as_str().unwrap());
    let status = call(
        &core,
        Method::RenderIprSet,
        json!({"enabled":false,"settings":{"samples_per_pixel":8,"threads":1,"max_depth":null,"debounce_ms":100}}),
    );
    assert_eq!(status["enabled"], false);
    let preview = wait(&core, first);
    assert_eq!(preview["state"], "cancelled", "{preview}");
    fs::write(temp.path().join("renderer.release"), "").unwrap();
    set_transform(&core, 3);
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(ipr_jobs(&core).len(), 1, "disabled ipr launches nothing");
    assert_eq!(ipr_status(&core)["active_job_id"], Value::Null);
    assert_eq!(
        core.request(request(Method::RenderIprSubmit, json!({})))
            .error
            .unwrap()
            .code,
        ErrorCode::Unsupported
    );
}
