//! QA S1 regression: headless capture must honour the spec camera's `mirror`
//! flag. Both adapter paths — the one-shot `--spec` run and the resident
//! `--server` daemon — build the renderer camera from the request, so the
//! `mirror: true` capture must be the exact horizontal flip of the
//! `mirror: false` capture, color and pick buffers alike. The scene is
//! asymmetric left-right (one quad off to the side), so a dropped `mirror`
//! cannot hide behind symmetry.

use protocol::jobs::CaptureSpec;
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

const CAPTURE_BIN: &str = env!("CARGO_BIN_EXE_pbrt-ui-capture");
const SIZE: [u32; 2] = [96, 64];
/// Both daemon requests carry this key, so the second one reuses the kept
/// GPU scene and only its camera may differ.
const GEOMETRY_KEY: &str = "s1-mirror-regression";
/// One quad spanning world x 0.5..1.5: asymmetric left-right.
const ASYMMETRIC_SCENE: &str = "WorldBegin\nTranslate 1.0 0 0\nShape \"trianglemesh\" \"point3 P\" [-0.5 -0.5 0 0.5 -0.5 0 0.5 0.5 0 -0.5 0.5 0] \"integer indices\" [0 1 2 0 2 3]\n";

/// Parse the asymmetric scene and publish it as the geometry package the
/// capture adapter reads (`geometry.json` plus blobs), exactly like core's
/// preview job. Returns the manifest path and the time sample to render.
fn write_geometry_package(root: &Path) -> (PathBuf, f64) {
    let scene = root.join("asymmetric.pbrt");
    std::fs::write(&scene, ASYMMETRIC_SCENE).unwrap();
    let preview = scene_parse::parse_scene(&scene, scene_parse::ParseOptions::default()).unwrap();
    assert!(preview.diagnostics.is_empty());
    assert!(preview.manifest.unsupported.is_empty());
    assert_eq!(preview.manifest.draws.len(), 1, "the quad must tessellate");
    let time = *preview
        .manifest
        .supported_times
        .first()
        .expect("a static scene exports at least one time");
    let package = root.join("geometry");
    scene_parse::write_preview(&preview, &package, true).unwrap();
    (package.join("geometry.json"), time)
}

/// The capture spec as the wire DTO, parsed through `CaptureSpec` so the
/// file the adapter reads is byte-compatible what core's capture jobs write.
fn capture_spec(geometry_path: &Path, mirror: bool, time: f64) -> CaptureSpec {
    let value = json!({
        "geometry_path": geometry_path.display().to_string(),
        "camera": {
            "eye": [0., 0., 5.0], "target": [0., 0., 0.0], "up": [0., 1., 0.0],
            "projection": "perspective", "fov_y_degrees": 45.0,
            "near": 0.1, "far": 100.0, "mirror": mirror
        },
        "size": SIZE, "mode": "solid", "time": time,
        "frame_key": {
            "scene_id": "00000000-0000-0000-0000-000000000001",
            "scene_revision": "1", "view_revision": "1"
        }
    });
    serde_json::from_value(value).unwrap()
}

/// Run one one-shot `--spec` capture.
fn run_spec_capture(spec: &CaptureSpec, spec_path: &Path, output: &Path) {
    std::fs::write(spec_path, serde_json::to_vec(spec).unwrap()).unwrap();
    let result = Command::new(CAPTURE_BIN)
        .arg("--spec")
        .arg(spec_path)
        .arg(output)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "one-shot capture failed: {}\nstdout: {}\nstderr: {}",
        result.status,
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

/// The pick sidecar both capture paths write: one little-endian u32 per pixel.
fn pick_ids(png: &Path) -> Vec<u32> {
    let bytes = std::fs::read(png.with_extension("pick")).unwrap();
    bytes
        .as_chunks::<4>()
        .0
        .iter()
        .map(|value| u32::from_le_bytes(*value))
        .collect()
}

/// The `mirror: true` capture must equal the horizontally
/// flipped `mirror: false` capture exactly (color and pick), the two must
/// differ, and the quad must land on opposite screen halves.
fn assert_flip_contract(unmirrored: &Path, mirrored: &Path) {
    let plain = image::open(unmirrored).unwrap().to_rgba8();
    let flipped = image::open(mirrored).unwrap().to_rgba8();
    assert!(
        plain.as_raw() != flipped.as_raw(),
        "mirror: true must repaint the image, not repeat mirror: false pixels"
    );
    let (width, height) = (plain.width() as usize, plain.height() as usize);
    assert_eq!([width as u32, height as u32], SIZE);
    for y in 0..height {
        for x in 0..width {
            assert_eq!(
                plain.get_pixel((width - 1 - x) as u32, y as u32),
                flipped.get_pixel(x as u32, y as u32),
                "mirror:true pixel ({x},{y}) must equal the flipped mirror:false capture"
            );
        }
    }
    let plain_pick = pick_ids(unmirrored);
    let flipped_pick = pick_ids(mirrored);
    assert_eq!(plain_pick.len(), width * height);
    assert_eq!(flipped_pick.len(), width * height);
    for y in 0..height {
        for x in 0..width {
            assert_eq!(
                plain_pick[y * width + (width - 1 - x)],
                flipped_pick[y * width + x],
                "mirror:true pick ({x},{y}) must equal the flipped mirror:false capture"
            );
        }
    }
    let covered_centre = |ids: &[u32]| {
        let (mut sum, mut count) = (0., 0usize);
        for (index, id) in ids.iter().enumerate() {
            if *id != 0 {
                sum += (index % width) as f64;
                count += 1;
            }
        }
        assert!(count > 0, "the quad must appear in the capture");
        sum / count as f64
    };
    let (plain_centre, mirrored_centre) =
        (covered_centre(&plain_pick), covered_centre(&flipped_pick));
    let middle = width as f64 / 2.;
    assert!(
        (plain_centre - middle) * (mirrored_centre - middle) < 0.,
        "the quad must sit on opposite screen halves: {plain_centre} vs {mirrored_centre}"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn spec_capture_renders_mirror_true_as_the_horizontal_flip_of_mirror_false() {
    let root = tempfile::tempdir().unwrap();
    let (geometry_path, time) = write_geometry_package(root.path());
    let unmirrored = root.path().join("spec-false.png");
    let mirrored = root.path().join("spec-true.png");
    run_spec_capture(
        &capture_spec(&geometry_path, false, time),
        &root.path().join("spec-false.json"),
        &unmirrored,
    );
    run_spec_capture(
        &capture_spec(&geometry_path, true, time),
        &root.path().join("spec-true.json"),
        &mirrored,
    );
    assert_flip_contract(&unmirrored, &mirrored);
}

/// A cleanup-friendly one-request helper for the resident daemon: write the
/// request line, read the reply, and require `ok`.
fn daemon_request(
    stdin: &mut std::process::ChildStdin,
    replies: &mut BufReader<std::process::ChildStdout>,
    spec: &CaptureSpec,
    output: &Path,
) {
    let request = json!({
        "geometry_key": GEOMETRY_KEY,
        "spec": serde_json::to_value(spec).unwrap(),
        "output": output.display().to_string(),
    });
    stdin
        .write_all(serde_json::to_string(&request).unwrap().as_bytes())
        .unwrap();
    stdin.write_all(b"\n").unwrap();
    stdin.flush().unwrap();
    let mut reply = String::new();
    replies.read_line(&mut reply).unwrap();
    let reply: Value = serde_json::from_str(reply.trim_end()).unwrap();
    assert_eq!(reply["ok"], json!(true), "daemon reply: {reply}");
}

#[test]
#[ignore = "requires a GPU adapter"]
fn server_capture_keeps_geometry_but_honours_mirror_per_request() {
    let root = tempfile::tempdir().unwrap();
    let (geometry_path, time) = write_geometry_package(root.path());
    let mut child = Command::new(CAPTURE_BIN)
        .arg("--server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut replies = BufReader::new(child.stdout.take().unwrap());
    let mut ready = String::new();
    replies.read_line(&mut ready).unwrap();
    let ready: Value = serde_json::from_str(ready.trim_end()).unwrap();
    assert_eq!(ready["ready"], json!(true), "daemon ready line: {ready}");

    let unmirrored = root.path().join("daemon-false.png");
    let mirrored = root.path().join("daemon-true.png");
    daemon_request(
        &mut stdin,
        &mut replies,
        &capture_spec(&geometry_path, false, time),
        &unmirrored,
    );
    // Same geometry key: the daemon keeps its GPU-resident scene, so the
    // second frame differs only through the request's own camera.mirror.
    daemon_request(
        &mut stdin,
        &mut replies,
        &capture_spec(&geometry_path, true, time),
        &mirrored,
    );
    drop(stdin);
    let status = child.wait().unwrap();
    assert!(status.success(), "daemon exited with {status}");
    assert_flip_contract(&unmirrored, &mirrored);
}
