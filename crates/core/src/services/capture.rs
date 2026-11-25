//! Capture/pick adapter requests and image/pick decoding.
use super::*;

/// Carry a Lit capture's `lit_notes` from the capture daemon's reply into
/// the job result.
fn copy_lit_notes(reply: &Value, result: &mut Value) {
    if let Some(notes) = reply.get("lit_notes") {
        result["lit_notes"] = notes.clone();
    }
}

/// The long side of a scene-camera capture's size: big enough to
/// read, small enough to raster quickly.
const CAPTURE_LONG_SIDE: u32 = 1280;

/// A scene-camera capture's size at the film's aspect ratio: the
/// manifest camera record's `frame_aspect_ratio` when set, else its
/// `film_resolution`, with the long side at `CAPTURE_LONG_SIDE` and the
/// ratio kept after rounding (each side at least one pixel). `None` when
/// the record names no usable aspect — the caller keeps the 1280×720
/// default of `initial_view`.
fn film_capture_size(record: Option<&protocol::geometry::CameraRecord>) -> Option<[u32; 2]> {
    let record = record?;
    let aspect = match record.frame_aspect_ratio {
        Some(aspect) if aspect.is_finite() && aspect > 0. => aspect,
        _ => record
            .film_resolution
            .filter(|[width, height]| *width > 0 && *height > 0)
            .map(|[width, height]| width as f64 / height as f64)?,
    };
    Some(if aspect >= 1. {
        [
            CAPTURE_LONG_SIDE,
            (CAPTURE_LONG_SIDE as f64 / aspect).round().max(1.) as u32,
        ]
    } else {
        [
            (CAPTURE_LONG_SIDE as f64 * aspect).round().max(1.) as u32,
            CAPTURE_LONG_SIDE,
        ]
    })
}

/// The view a scene-camera capture resolves to: the scene camera —
/// or the framed stand-in `initial_view` picks without one — at the
/// film's aspect ratio, with the shared view's mode and time where one
/// exists and the initial defaults otherwise. Decided here, so
/// `initial_view`'s own 16:9 default stays untouched for the desktop.
fn scene_capture_view(
    manifest: &protocol::geometry::GeometryManifest,
    shared: Option<&protocol::view::ViewState>,
) -> protocol::view::ViewState {
    let mut view = crate::view::initial(Some(manifest));
    if let Some(size) = film_capture_size(manifest.camera.as_ref()) {
        view.size = size;
    }
    if let Some(shared) = shared {
        view.mode = shared.mode;
        view.time = shared.time;
    }
    view
}

/// Read the direct-preview package a `Work::Preview` job published. Capture
/// and pick never parse scene text or an export manifest.
pub(super) fn read_project_preview(
    root: &Path,
) -> Result<scene_parse::PreviewGeometry, ErrorPayload> {
    scene_parse::read_preview_package(root)
        .map_err(|error| ErrorPayload::new(ErrorCode::OutputInvalid, error.to_string()))
}

/// Run one capture through the resident daemon when possible; spawn, break
/// or timeout fall back to the historical one-shot `--spec` adapter run.
pub(super) fn run_capture(
    work: &ServiceTask,
    directory: &Path,
    spec: &CaptureSpec,
    output: &Path,
) -> Result<Option<Value>, ErrorPayload> {
    if work.task.is_cancelled() {
        return Err(ErrorPayload::new(ErrorCode::Busy, "job cancelled"));
    }
    let geometry_key = fs::read(&spec.geometry_path)
        .map(|bytes| format!("{:x}", sha2::Sha256::digest(&bytes)))
        .map_err(fail)?;
    {
        let mut daemon = work
            .config
            .capture_daemon
            .lock()
            .map_err(|_| fail("capture daemon lock poisoned"))?;
        if daemon.is_none() {
            if let Some(program) = work.config.capture.as_ref() {
                match capture_daemon::CaptureDaemon::spawn(program) {
                    Ok(session) => *daemon = Some(session),
                    Err(_) => *daemon = None,
                }
            }
        }
        if let Some(session) = daemon.as_mut() {
            match session.capture(&work.task, &geometry_key, spec, output) {
                Ok(reply) => return Ok(reply.get("lit_notes").cloned()),
                Err(error) => {
                    *daemon = None;
                    // A cancelled request must not repaint through the
                    // one-shot fallback; let the job end cancelled.
                    if error.code == ErrorCode::Busy {
                        return Err(error);
                    }
                }
            }
        }
    }
    // One-shot fallback: write the spec and run the adapter process once.
    let spec_path = directory.join("capture.json");
    fs::write(&spec_path, serde_json::to_vec(spec).map_err(fail)?).map_err(fail)?;
    let mut command =
        Command::new(work.config.capture.as_ref().ok_or_else(|| {
            ErrorPayload::new(ErrorCode::Unsupported, "capture adapter unavailable")
        })?);
    command.arg("--spec").arg(&spec_path).arg(output);
    let log = directory.join("capture.log");
    run_process(command, &work.task, &log)?;
    Ok(None)
}

pub(super) fn execute(
    work: &ServiceTask,
    directory: &PathBuf,
    artifacts: &mut Vec<Artifact>,
) -> Result<Value, ErrorPayload> {
    match &work.work {
        Work::ProjectCapture {
            root,
            project,
            view,
            view_revision,
            pick,
            debug_albedo,
            debug_ao,
            camera,
        } => {
            if work.config.capture.is_none() {
                return Err(ErrorPayload::new(
                    ErrorCode::Unsupported,
                    "capture adapter unavailable",
                ));
            }
            let preview = read_project_preview(Path::new(root))?;
            // Resolve the effective view. `camera: "scene"` frames the
            // scene's own camera at the film's aspect ratio, ignoring any
            // shared camera pose and size, with the shared view's mode and
            // time where one exists; a shared-view capture keeps it whole.
            let view = if *camera == Some(protocol::project::RenderCamera::Scene) {
                scene_capture_view(&preview.manifest, view.as_ref())
            } else {
                view.clone()
                    .unwrap_or_else(|| scene_capture_view(&preview.manifest, None))
            };
            let spec = CaptureSpec {
                // The published slot is already the package the adapter
                // reads, so capture borrows it in place.
                geometry_path: Path::new(root)
                    .join("geometry.json")
                    .to_str()
                    .ok_or_else(|| fail("non UTF-8 geometry path"))?
                    .into(),
                camera: view.camera.clone(),
                size: view.size,
                mode: view.mode,
                debug_albedo: *debug_albedo,
                debug_ao: *debug_ao,
                time: view.time,
                frame_key: protocol::view::FrameKey {
                    scene_id: project.project_id,
                    scene_revision: project.revision.clone(),
                    view_revision: view_revision.clone(),
                },
            };
            let output = directory.join("preview.png");
            let lit_notes = run_capture(work, directory, &spec, &output)?;
            if let Some(xy) = pick {
                let bytes = fs::read(output.with_extension("pick")).map_err(fail)?;
                let x =
                    ((xy[0] * spec.size[0] as f64).floor() as usize).min(spec.size[0] as usize - 1);
                let y =
                    ((xy[1] * spec.size[1] as f64).floor() as usize).min(spec.size[1] as usize - 1);
                let offset = (y * spec.size[0] as usize + x) * 4;
                let pick_id = u32::from_le_bytes(
                    bytes
                        .get(offset..offset + 4)
                        .ok_or_else(|| fail("invalid pick buffer"))?
                        .try_into()
                        .unwrap(),
                );
                // The direct program pinned every draw to its project node;
                // the manifest is the only binding source.
                let node_id = preview
                    .manifest
                    .draws
                    .iter()
                    .find(|draw| draw.pick_id == pick_id)
                    .and_then(|draw| draw.entity_id.as_ref());
                return Ok(json!({
                    "project": project,
                    "view": view,
                    "view_revision": view_revision,
                    "pick_id": pick_id,
                    "node_id": node_id,
                }));
            }
            let unsupported = preview.manifest.unsupported.len() + preview.warning_count();
            let result = artifact(
                &output,
                "image/png",
                json!({"kind":"project_capture","project":project,"camera":spec.camera,"size":spec.size,"mode":spec.mode,"debug_albedo":spec.debug_albedo,"debug_ao":spec.debug_ao,"time":spec.time,"unsupported":unsupported}),
            )?;
            let id = result.artifact_id;
            artifacts.push(result);
            let mut result = json!({
                "artifact_id": id,
                "project": project,
                "view": view,
                "view_revision": view_revision,
            });
            if let Some(notes) = lit_notes {
                copy_lit_notes(&json!({"lit_notes":notes}), &mut result);
            }
            Ok(result)
        }
        _ => unreachable!("capture dispatch"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use protocol::geometry::CameraRecord;

    /// Any CameraRecord shape enough for size computation.
    fn record(frame_aspect_ratio: Option<f64>, film_resolution: Option<[u32; 2]>) -> CameraRecord {
        CameraRecord {
            kind: "perspective".into(),
            camera_from_world: [
                1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
            ],
            look_at: None,
            fov_degrees: None,
            screenwindow: None,
            frame_aspect_ratio,
            shutter: [0., 1.],
            film_resolution,
            film_exposure: None,
        }
    }

    /// A capture size keeps the Film resolution's ratio at a 1280 long
    /// side: landscape keeps the width, portrait the height.
    #[test]
    fn film_capture_size_follows_the_film_resolution() {
        assert_eq!(
            film_capture_size(Some(&record(None, Some([1920, 1080])))),
            Some([1280, 720])
        );
        assert_eq!(
            film_capture_size(Some(&record(None, Some([600, 800])))),
            Some([960, 1280])
        );
    }

    /// An explicit frameaspectratio wins over the film resolution.
    #[test]
    fn film_capture_size_prefers_frame_aspect_ratio() {
        assert_eq!(
            film_capture_size(Some(&record(Some(2.0), Some([640, 480])))),
            Some([1280, 640])
        );
        assert_eq!(
            film_capture_size(Some(&record(Some(0.75), Some([1920, 1080])))),
            Some([960, 1280])
        );
    }

    /// The short side rounds and never drops below one pixel.
    #[test]
    fn film_capture_size_rounds_and_clamps_the_short_side() {
        // 1280/3 = 426.67.
        assert_eq!(
            film_capture_size(Some(&record(Some(3.0), None))),
            Some([1280, 427])
        );
        assert_eq!(
            film_capture_size(Some(&record(None, Some([1, 1])))),
            Some([1280, 1280])
        );
        // Extreme ratios stay a pixel wide, not zero.
        assert_eq!(
            film_capture_size(Some(&record(Some(1e9), None))),
            Some([1280, 1])
        );
        assert_eq!(
            film_capture_size(Some(&record(Some(1e-9), None))),
            Some([1, 1280])
        );
    }

    /// Without a camera record or any aspect the 1280×720 default stands.
    #[test]
    fn film_capture_size_stays_1280_720_without_an_aspect() {
        assert_eq!(film_capture_size(None), None);
        assert_eq!(film_capture_size(Some(&record(None, None))), None);
        // A degenerate resolution contributes no ratio.
        assert_eq!(film_capture_size(Some(&record(None, Some([0, 800])))), None);
    }

    #[test]
    fn capture_jobs_pass_lit_notes_through() {
        let reply = json!({"ok": true, "lit_notes": [
            {"reason": "hair: shown as gray diffuse", "material": "hair", "draws": 3}
        ]});
        let mut result = json!({"artifact_id": "a"});
        copy_lit_notes(&reply, &mut result);
        assert_eq!(result["lit_notes"], reply["lit_notes"]);
        let mut plain = json!({"artifact_id": "a"});
        copy_lit_notes(&json!({"ok": true}), &mut plain);
        assert!(
            plain.get("lit_notes").is_none(),
            "no notes key without notes"
        );
    }
}
