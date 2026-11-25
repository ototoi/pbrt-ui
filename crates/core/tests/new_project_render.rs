//! Render check for the initial scene created by `project.create`: the
//! preset's Floor faces upward in both supported convention pairs — the
//! Y-up left-handed default and a Z-up right-handed world — so pbrt-r4
//! lights it from above against the environment background instead of
//! rendering a black, back-facing floor. The test also checks that the
//! initial scene's Film sets the resolution to 640×360 rather than using
//! pbrt-r4's fallback.
//!
//! Machine-bound (a real renderer), so the test is ignored unless the
//! pbrt-r4 binary is around: pass `PBRT_R4=<path>`, or place it at `../pbrt-r4` beside this repository.
use core::{CoreHandle, CoreHost};
use image::{GenericImageView, ImageBuffer};
use protocol::commands::Limits;
use protocol::{Method, Request};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::Command as Process;
use std::time::{Duration, Instant};

fn renderer() -> Option<PathBuf> {
    let path = std::env::var_os("PBRT_R4")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../pbrt-r4/target/release/pbrt-r4")
        });
    path.is_file().then_some(path)
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

fn wait(core: &CoreHandle, job: Value) -> Value {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let value = call(core, Method::JobGet, json!({"job_id":job["job_id"]}));
        if ["succeeded", "failed", "cancelled"].contains(&value["state"].as_str().unwrap()) {
            return value;
        }
        assert!(Instant::now() < deadline, "job did not terminate: {value}");
        std::thread::sleep(Duration::from_millis(2));
    }
}

/// The mean 8-bit brightness of a row band, across the inner 80% of
/// columns: what the sky and the floor read as in the render.
fn band_mean(image: &ImageBuffer<image::Rgb<u8>, Vec<u8>>, first_row: u32, last_row: u32) -> f64 {
    let (width, height) = image.dimensions();
    assert!(last_row <= height && first_row < last_row);
    let (first, last) = (width / 10, width * 9 / 10);
    let mut sum = 0u64;
    let mut count = 0u64;
    for (_, _, pixel) in image
        .view(first, first_row, last - first, last_row - first_row)
        .pixels()
    {
        let [r, g, b] = pixel.0;
        sum += (r as u64 + g as u64 + b as u64) / 3;
        count += 1;
    }
    sum as f64 / count.max(1) as f64
}

#[test]
#[ignore = "requires the pbrt-r4 binary (PBRT_R4=<path> or../pbrt-r4/target/release/pbrt-r4)"]
fn project_create_floors_render_lit_from_above_in_r4() {
    let Some(renderer) = renderer() else {
        eprintln!("skipped: no pbrt-r4 binary; set PBRT_R4");
        return;
    };
    // 's two convention pairs: the new-project default and the
    // mirrored world.
    for (up, handedness) in [("+Y", "left"), ("+Z", "right")] {
        let temp = tempfile::tempdir().unwrap();
        let host = CoreHost::start(Limits::default());
        let core = host.handle();
        let destination = temp.path().join("Fresh");
        let job = call(
            &core,
            Method::ProjectCreate,
            json!({
                "path":destination,
                "title":"Fresh",
                "coordinate_system":{"up":up,"handedness":handedness},
                "expected_current":state(&core)["context"],
            }),
        );
        let created = wait(&core, job);
        assert_eq!(created["state"], "succeeded", "{created}");
        let reference = state(&core)["context"]["project"].clone();

        let export = call(
            &core,
            Method::ProjectExport,
            json!({"project":reference,"destination":temp.path().join("bundle")}),
        );
        let exported = wait(&core, export);
        assert_eq!(exported["state"], "succeeded", "{exported}");
        let scene_path = PathBuf::from(exported["result"]["scene_path"].as_str().unwrap());

        let png = temp.path().join("fresh.png");
        let finished = Process::new(&renderer)
            .arg(&scene_path)
            .arg("--outfile")
            .arg(&png)
            .arg("--spp")
            .arg("1")
            .arg("--quiet")
            .output()
            .expect("pbrt-r4 runs");
        assert!(
            finished.status.success(),
            "r4 exit {:?}: {}",
            finished.status.code(),
            String::from_utf8_lossy(&finished.stderr)
        );
        let rendered = image::open(&png)
            .expect("r4 wrote the PNG the outfile names")
            .to_rgb8();
        // the initial scene's own Film decides the resolution —
        // 640×360, not r4's fallback default.
        assert_eq!(rendered.dimensions(), (640, 360));
        let (_, height) = rendered.dimensions();
        // The camera looks down across the Floor, so the top of the
        // frame is the 0.5 environment sky and the bottom third is the
        // lit diffuse Floor: bright against black, dimmer than the sky.
        let sky = band_mean(&rendered, 0, height / 8);
        let floor = band_mean(&rendered, 2 * height / 3, height * 19 / 20);
        eprintln!("{up}/{handedness}: floor {floor:.1}, sky {sky:.1}, {png:?}");
        assert!(
            floor > 40.,
            "the floor must render lit from above, not back-face black: {floor:.1}"
        );
        assert!(
            floor + 25. < sky,
            "a lit diffuse floor stays dimmer than the environment behind it: {floor:.1} vs {sky:.1}"
        );
    }
}

/// A final render keeps publishing partial previews past the old 32-snapshot
/// cap (about 16 s at 2 fps), and only the latest preview is retained.
/// Needs pbrt-r4 and a built `pbrt-ui-capture` (PBRT_UI_CAPTURE=<path> or
/// target/release/pbrt-ui-capture).
#[test]
#[ignore = "requires the pbrt-r4 binary and target/release/pbrt-ui-capture"]
fn long_render_keeps_publishing_the_latest_preview() {
    let capture = std::env::var_os("PBRT_UI_CAPTURE")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/release/pbrt-ui-capture")
        });
    let (Some(renderer), true) = (renderer(), capture.is_file()) else {
        eprintln!("skipped: need pbrt-r4 and pbrt-ui-capture");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let host = CoreHost::start_with_services(
        Limits::default(),
        Some(core::services::ServiceConfig {
            gui: false,
            capture: Some(capture),
            renderer: Some(renderer),
            directory: std::sync::Arc::new(tempfile::tempdir().unwrap()),
            geometry_cache: Default::default(),
            capture_daemon: Default::default(),
            project_directories: Default::default(),
        }),
    );
    let core = host.handle();
    let job = call(
        &core,
        Method::ProjectCreate,
        json!({"path":temp.path().join("Slow"),"title":"Slow","expected_current":state(&core)["context"]}),
    );
    assert_eq!(wait(&core, job)["state"], "succeeded");
    let reference = state(&core)["context"]["project"].clone();
    // One thread makes the 64 spp initial scene take well over 16 s.
    let settings = call(&core, Method::RenderGetSettings, json!({}));
    call(
        &core,
        Method::RenderSetSettings,
        json!({"settings":{"samples_per_pixel":null,"threads":1,"max_depth":null},
            "expected_settings_revision":settings["settings_revision"]}),
    );
    let settings = call(&core, Method::RenderGetSettings, json!({}));
    let render = call(
        &core,
        Method::ProjectRender,
        json!({"project":reference,"expected_settings_revision":settings["settings_revision"]}),
    );
    let started = Instant::now();
    let mut partials = Vec::new();
    let mut last_change = started;
    while started.elapsed() < Duration::from_secs(24) {
        let job = call(&core, Method::JobGet, json!({"job_id":render["job_id"]}));
        let ids = job["artifacts"].as_array().cloned().unwrap_or_default();
        let partial: Vec<_> = ids
            .iter()
            .filter(|id| {
                // A superseded preview may be gone by the time it is asked for.
                core.request(request(Method::ArtifactGet, json!({"artifact_id":id})))
                    .result
                    .is_some_and(|artifact| artifact["metadata"]["partial"] == true)
            })
            .cloned()
            .collect();
        assert!(partial.len() <= 1, "previews accumulated: {partial:?}");
        if partial
            .first()
            .is_some_and(|id| partials.last() != Some(id))
        {
            partials.push(partial[0].clone());
            last_change = Instant::now();
        }
        assert!(
            job["state"] == "queued" || job["state"] == "running",
            "the render must outlast the test window: {job}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
    call(&core, Method::JobCancel, json!({"job_id":render["job_id"]}));
    let elapsed = last_change.duration_since(started).as_secs_f64();
    eprintln!("{} previews, last change at {elapsed:.1} s", partials.len());
    assert!(
        partials.len() > 32,
        "the 32-snapshot cap is gone: {}",
        partials.len()
    );
    assert!(
        elapsed > 20.,
        "previews kept coming after 16 s: last at {elapsed:.1} s"
    );
}
