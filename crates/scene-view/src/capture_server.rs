//! Resident capture daemon protocol: one wgpu device and one
//! `PreviewRenderer` serve a newline-delimited JSON request stream so
//! repeated captures skip device creation and re-upload unchanged geometry.
//!
//! Protocol over the injected streams (all messages are single JSON lines):
//! 1. the daemon writes `{"ready":true,"protocol":1}` once after start,
//! 2. the host may send `{"hello":true}`, answered with
//!    `{"ok":true,"protocol":1}`,
//! 3. each request `{"geometry_key":"<sha256>","spec":<CaptureSpec>,"output":"<path>"}`
//!    renders one frame, writes the PNG and its `.pick` sidecar next to it, and
//!    replies with `{"ok":true,"output":...,"pick":...}` or `{"ok":false,"error":...}`,
//! 4. input EOF ends the session.
//!
//! A `geometry_key` identical to the previous request skips blob loading and
//! signals the renderer that the GPU-resident scene is still valid; mesh-level
//! edition reuse happens inside the renderer (see `PreviewRenderer::update_scene`).
use protocol::geometry::GeometryManifest;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{BufRead, Write},
    path::{Component, Path, PathBuf},
};

/// Geometry package of one request: manifest plus blob bytes.
pub struct Geometry {
    pub manifest: GeometryManifest,
    pub blobs: BTreeMap<String, Vec<u8>>,
}

/// One request from the host.
pub struct CaptureRequest {
    /// Content hash of the geometry manifest; equal keys skip re-upload.
    pub geometry_key: String,
    pub spec: protocol::jobs::CaptureSpec,
    /// PNG destination; the pick buffer lands next to it as `.pick`.
    pub output: PathBuf,
}

/// Render one frame. Receives the geometry when it was (re)loaded for this
/// request and `None` when the scene kept from the previous request is still
/// current; returns the frame to persist. Overridable for tests.
pub type Render = Box<
    dyn FnMut(
        &CaptureRequest,
        Option<&Geometry>,
    ) -> Result<(crate::preview::PreviewFrame, Vec<crate::preview::LitNote>), String>,
>;

/// Serve requests from `input`, writing replies to `output`. Returns on input
/// EOF or an unrecoverable stream error.
pub fn serve<R: BufRead, W: Write>(input: R, output: W, mut render: Render) -> Result<(), String> {
    let mut writer = output;
    let mut session = Session {
        key: String::new(),
        loaded: false,
    };
    let input = input;
    let mut line = String::new();
    let mut buffered = std::io::BufReader::new(input);
    line.clear();
    write_line(&mut writer, json!({"ready": true, "protocol": 1}))
        .map_err(|error| format!("write ready line: {error}"))?;
    writer.flush().ok();
    loop {
        line.clear();
        let count = buffered
            .read_line(&mut line)
            .map_err(|error| format!("read request: {error}"))?;
        if count == 0 {
            return Ok(());
        }
        let reply = handle_line(&mut session, &line, &mut render);
        write_line(&mut writer, reply).map_err(|error| format!("write reply: {error}"))?;
        writer.flush().ok();
    }
}

struct Session {
    key: String,
    loaded: bool,
}

fn handle_line(session: &mut Session, line: &str, render: &mut Render) -> Value {
    let request: Value = match serde_json::from_str(line.trim_end()) {
        Ok(request) => request,
        Err(error) => return json!({"ok": false, "error": format!("malformed request: {error}")}),
    };
    if request.get("hello") == Some(&json!(true)) {
        return json!({"ok": true, "protocol": 1});
    }
    let Ok(parsed) = parse_request(&request) else {
        return json!({"ok": false, "error": "invalid capture request"});
    };
    // Reload the geometry package only when its content hash changed.
    let mut reloaded = None;
    if !session.loaded || session.key != parsed.geometry_key {
        match load_geometry(&parsed) {
            Ok(geometry) => {
                session.key = parsed.geometry_key.clone();
                reloaded = Some(geometry);
            }
            Err(error) => return json!({"ok": false, "error": error}),
        }
    }
    let frame = render(&parsed, reloaded.as_ref());
    session.loaded = true;
    match frame.and_then(|(frame, notes)| write_frame(&parsed.output, &frame).map(|_| notes)) {
        Ok(notes) => {
            let mut reply = json!({"ok":true,"output":parsed.output.display().to_string(),"pick":parsed.output.with_extension("pick").display().to_string()});
            if parsed.spec.mode == protocol::view::DisplayMode::Lit
                || parsed.spec.debug_albedo
                || parsed.spec.debug_ao
            {
                reply["lit_notes"] = json!(notes);
            }
            reply
        }
        Err(error) => json!({"ok": false, "error": error}),
    }
}

fn parse_request(request: &Value) -> Result<CaptureRequest, String> {
    let spec: protocol::jobs::CaptureSpec = serde_json::from_value(
        request
            .get("spec")
            .cloned()
            .ok_or("capture request needs a spec")?,
    )
    .map_err(|error| format!("invalid capture spec: {error}"))?;
    Ok(CaptureRequest {
        geometry_key: request
            .get("geometry_key")
            .and_then(Value::as_str)
            .ok_or("capture request needs a geometry key")?
            .to_owned(),
        spec,
        output: PathBuf::from(
            request
                .get("output")
                .and_then(Value::as_str)
                .ok_or("capture request needs an output path")?,
        ),
    })
}

/// Read the geometry package a request points at (manifest and blob bytes),
/// with the same path-safety checks the one-shot adapter uses.
pub fn load_geometry(request: &CaptureRequest) -> Result<Geometry, String> {
    let manifest_path = PathBuf::from(&request.spec.geometry_path)
        .canonicalize()
        .map_err(|error| format!("geometry path: {error}"))?;
    let directory = manifest_path
        .parent()
        .ok_or("geometry manifest has no parent")?;
    let manifest: GeometryManifest = serde_json::from_slice(
        &std::fs::read(&manifest_path).map_err(|error| format!("geometry manifest: {error}"))?,
    )
    .map_err(|error| format!("geometry manifest: {error}"))?;
    let mut blobs = BTreeMap::new();
    for name in manifest.blobs.keys() {
        let rel = Path::new(name);
        if rel.components().any(|c| !matches!(c, Component::Normal(_))) {
            return Err(format!("unsafe blob path {name}"));
        }
        let path = directory
            .join(rel)
            .canonicalize()
            .map_err(|error| format!("blob {name}: {error}"))?;
        if !path.starts_with(directory) {
            return Err(format!("blob escapes geometry directory: {name}"));
        }
        blobs.insert(
            name.clone(),
            std::fs::read(&path).map_err(|error| format!("blob {name}: {error}"))?,
        );
    }
    Ok(Geometry { manifest, blobs })
}

fn write_frame(output: &Path, frame: &crate::preview::PreviewFrame) -> Result<(), String> {
    let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.rgba.clone())
        .ok_or("invalid RGBA frame size")?;
    if let Some(parent) = output.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("create output directory: {error}"))?;
    }
    image
        .save(output)
        .map_err(|error| format!("write {}: {error}", output.display()))?;
    std::fs::write(
        output.with_extension("pick"),
        frame
            .pick_ids
            .iter()
            .flat_map(|id| id.to_le_bytes())
            .collect::<Vec<_>>(),
    )
    .map_err(|error| format!("write pick buffer: {error}"))?;
    Ok(())
}

fn write_line<W: Write>(writer: &mut W, value: Value) -> std::io::Result<()> {
    let mut encoded =
        serde_json::to_string(&value).map_err(|error| std::io::Error::other(error.to_string()))?;
    encoded.push('\n');
    writer.write_all(encoded.as_bytes())
}

/// Mesh editions for the renderer: content digests of each mesh's blob, so
/// equal content keeps its GPU buffers across requests. The whole blob is
/// hashed — sampling its head would let same-length edits reuse stale GPU
/// buffers.
pub fn mesh_editions(
    manifest: &GeometryManifest,
    blobs: &BTreeMap<String, Vec<u8>>,
) -> Result<BTreeMap<u64, u64>, String> {
    let mut editions = BTreeMap::new();
    for record in &manifest.meshes {
        let blob = blobs
            .get(&record.blob)
            .ok_or_else(|| format!("missing blob {}", record.blob))?;
        let mut hash = sha2::Sha256::new();
        use sha2::Digest;
        hash.update(record.mesh_id.to_le_bytes());
        hash.update((blob.len() as u64).to_le_bytes());
        hash.update(blob);
        editions.insert(
            record.mesh_id,
            u64::from_be_bytes(hash.finalize()[..8].try_into().unwrap()),
        );
    }
    Ok(editions)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::preview::PreviewFrame;

    fn frame() -> PreviewFrame {
        PreviewFrame {
            width: 1,
            height: 1,
            rgba: vec![1, 2, 3, 255],
            pick_ids: vec![7],
            backend: "test".into(),
        }
    }

    fn spec_json(geometry_path: &str) -> Value {
        json!({
            "geometry_key": "key",
            "output": "/tmp/pbrt-ui-capture-server-test.png",
            "spec": {
                "geometry_path": geometry_path,
                "camera": {
                    "eye": [0.0, 0.0, 8.0], "target": [0.0, 0.0, 0.0], "up": [0.0, 1.0, 0.0],
                    "projection": "perspective", "fov_y_degrees": 45.,
                    "near": 0.1, "far": 100.
                },
                "size": [16, 16], "mode": "solid", "time": 0.,
                "frame_key": {
                    "scene_id": "00000000-0000-0000-0000-000000000001",
                    "scene_revision": "1", "view_revision": "1"
                }
            }
        })
    }

    #[test]
    fn test_server_skips_geometry_reload_until_the_key_changes() {
        // Track reload notifications the renderer receives without a GPU.
        let reloaded = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
        let logs = reloaded.clone();
        let render: Render = Box::new(move |request, geometry| {
            logs.borrow_mut().push(geometry.is_some());
            let _ = request;
            Ok((frame(), vec![]))
        });
        let temp = tempfile::tempdir().unwrap();
        let geometry_path = temp.path().join("geometry.json");
        // The two requests share one output path only for simplicity of the
        // script; frame writes go to the tempdir.
        let output = temp.path().join("frame.png");
        let manifest = GeometryManifest::default();
        std::fs::write(&geometry_path, serde_json::to_vec(&manifest).unwrap()).unwrap();
        let mut request = spec_json(geometry_path.to_str().unwrap());
        request["output"] = json!(output.display().to_string());
        let line = format!("{}\n", serde_json::to_string(&request).unwrap());
        let requests = format!("{line}{line}");
        let mut writer = Vec::new();
        serve(
            std::io::Cursor::new(requests.as_bytes()),
            &mut writer,
            render,
        )
        .unwrap();
        let responses = String::from_utf8_lossy(&writer).into_owned();
        let lines: Vec<&str> = responses.lines().collect();
        assert_eq!(lines.len(), 3, "{responses}");
        assert!(lines[0].contains("\"ready\""));
        assert!(lines[1].contains("\"ok\":true"));
        assert!(lines[2].contains("\"ok\":true"));
        assert_eq!(
            *reloaded.borrow(),
            vec![true, false],
            "second request keeps the scene"
        );
        let pick = std::fs::read(output.with_extension("pick")).unwrap();
        assert_eq!(pick.as_slice(), 7u32.to_le_bytes());
    }

    #[test]
    fn test_mesh_editions_hash_the_whole_blob() {
        use protocol::geometry::{ArrayRef, MeshRecord};
        // Same mesh id and length, editing bytes beyond any head sample must
        // advance the edition or the daemon would reuse a stale GPU buffer.
        let mut manifest = GeometryManifest::default();
        manifest.meshes.push(MeshRecord {
            mesh_id: 0,
            blob: "geometry/0.bin".into(),
            positions: ArrayRef {
                offset: 0,
                count: 1,
                stride: 12,
            },
            normals: None,
            uv: None,
            indices: ArrayRef {
                offset: 0,
                count: 1,
                stride: 4,
            },
            origin_f64: [0.; 3],
            bounds: protocol::Bounds {
                min: [0.; 3],
                max: [0.; 3],
            },
        });
        let before = vec![0u8; 8192];
        let mut after = before.clone();
        after[8191] = 9;
        let first = mesh_editions(
            &manifest,
            &BTreeMap::from([("geometry/0.bin".to_owned(), before)]),
        )
        .unwrap();
        let second = mesh_editions(
            &manifest,
            &BTreeMap::from([("geometry/0.bin".to_owned(), after)]),
        )
        .unwrap();
        assert_ne!(
            first[&0], second[&0],
            "late-blob edits must change the edition"
        );
    }

    #[test]
    fn test_server_answers_handshake_and_reports_errors() {
        let render: Render = Box::new(|_, _| Ok((frame(), vec![])));
        let requests = "{\"hello\":true}\nnot json at all\n";
        let mut writer = Vec::new();
        serve(
            std::io::Cursor::new(requests.as_bytes()),
            &mut writer,
            render,
        )
        .unwrap();
        let responses = String::from_utf8(writer).unwrap();
        let lines: Vec<&str> = responses.lines().collect();
        assert!(lines[0].contains("\"ready\""));
        assert!(lines[1].contains("\"ok\":true"));
        assert!(lines[2].contains("\"ok\":false"));
    }

    #[test]
    fn capture_replies_carry_lit_notes_in_lit_or_debug_albedo_mode() {
        // Regression contract is checked through handle_line's successful reply.
        let temp = tempfile::tempdir().unwrap();
        let geometry_path = temp.path().join("geometry.json");
        std::fs::write(
            &geometry_path,
            serde_json::to_vec(&GeometryManifest::default()).unwrap(),
        )
        .unwrap();
        let mut request = spec_json(geometry_path.to_str().unwrap());
        request["spec"]["mode"] = json!("lit");
        request["output"] = json!(temp.path().join("frame.png").display().to_string());
        let render: Render = Box::new(|_, _| {
            Ok((
                frame(),
                vec![crate::preview::LitNote {
                    reason: "hair: shown as gray diffuse".into(),
                    material: Some("hair".into()),
                    draws: 3,
                }],
            ))
        });
        let mut session = Session {
            key: String::new(),
            loaded: false,
        };
        let mut render = render;
        let reply = handle_line(
            &mut session,
            &serde_json::to_string(&request).unwrap(),
            &mut render,
        );
        assert_eq!(reply["lit_notes"][0]["draws"], 3, "{reply}");
        request["spec"]["mode"] = json!("solid");
        request["spec"]["debug_albedo"] = json!(true);
        request["output"] = json!(temp.path().join("solid.png").display().to_string());
        let mut session = Session {
            key: String::new(),
            loaded: false,
        };
        let reply = handle_line(
            &mut session,
            &serde_json::to_string(&request).unwrap(),
            &mut render,
        );
        assert_eq!(reply["lit_notes"][0]["draws"], 3, "{reply}");
    }
}
