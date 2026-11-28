//! GPU worker thread: owns the PreviewRenderer so the coordinator loop
//! never runs the heavy preview preparation inline.
//!
//! Requests are processed strictly FIFO: install, renders, and gizmo
//! updates share one mailbox, every request carries the preview key it was
//! issued for, and the worker drops requests whose key no longer matches
//! the installed scene. Results flow back over one channel; the
//! coordinator publishes frames and keeps the pick state.
// The frame's channel swap runs on the worker thread; nothing here needs
// the interim allow anymore.
use super::frame::outline_selection;
use super::*;
use crate::project_preview::{MeshBounds, MeshEditions};
use protocol::geometry::{DrawRecord, GeometryManifest};

/// What the coordinator loop hands to the GPU thread.
pub(super) enum GpuRequest {
    /// Read the geometry package and install it into the renderer.
    Install {
        key: String,
        job_id: String,
        /// The package's slot directory (`geometry.json`'s parent).
        slot: PathBuf,
        /// Install this manifest instead of reading
        /// `geometry.json` from the slot — the transform fast path's light
        /// move, which retargets the installed package's draws and lights
        /// without a job. The slot still answers the blobs and diagnostics;
        /// the install's marker makes the coordinator keep the transform
        /// base it was computed from.
        manifest: Option<Box<GeometryManifest>>,
    },
    /// Drop the installed scene (activation change or preview teardown).
    Clear,
    /// New placements for the installed scene (gizmo preview). `key` pins
    /// the request to the scene the draws were computed against (design
    /// I1); `adopt` additionally advances the installed key to a newer
    /// revision the same geometry now represents, which avoids stale geometry.
    /// transform fast path installs delta-retargeted draws without a new
    /// package). `None` keeps the installed key (the gizmo's own updates).
    UpdateDraws {
        key: String,
        draws: Vec<DrawRecord>,
        adopt: Option<String>,
    },
    Render {
        id: u64,
        key: String,
        camera: ViewerCamera,
        size: [u32; 2],
        mode: PreviewMode,
        time: f64,
        highlight: BTreeSet<u32>,
    },
    Shutdown,
}

/// Where in the install pipeline a package failed, so the coordinator can
/// keep the existing two user-facing message texts: parse errors before
/// the renderer is touched, the renderer's own errors after.
pub(super) enum InstallStage {
    Parse,
    Upload,
}

/// What [`GpuResult::Installed`] delivers: everything the old inline
/// success branch installed, minus the heavy work itself. The coordinator
/// applies it to its own bookkeeping exactly where it used to.
pub(super) struct InstalledData {
    pub key: String,
    /// Becomes `project_geometry`: draws for gizmo/orbit gates, cameras,
    /// bounds for FrameAll/FrameSelection, supported times.
    pub manifest: GeometryManifest,
    /// pick_id -> entity node id; becomes `project_pick_nodes`.
    pub bindings: BTreeMap<u32, String>,
    /// mesh_id -> world bounds; becomes `project_object_bounds`.
    pub bounds: BTreeMap<u64, protocol::Bounds>,
    pub lit_notes: Vec<scene_view::preview::LitNote>,
    /// The package's `diagnostics.jsonl` records; its warnings reach the
    /// Console and the status line's unsupported count, its info
    /// notes (S6 media) reach the Console only.
    pub diagnostics: Vec<scene_parse::Diagnostic>,
    /// The slot this package was read from — the echo
    /// of the request's slot; the transform base re-installs from it on
    /// the next light move.
    pub slot: PathBuf,
    /// True when the request carried its manifest —
    /// the fast path's light move re-installing the base package
    /// retargeted. That manifest is the pre-move base with deltas
    /// applied, so `apply_installed` must keep the base it was
    /// computed from; everything else applies normally.
    pub keeps_transform_base: bool,
}

/// Results the worker sends back. Render results go to both the frames
/// slot (published by the coordinator) and the coordinator's own pick
/// state, so `Command::Pick`'s frame-id match keeps working.
pub(super) enum GpuResult {
    Installed(Box<InstalledData>),
    InstallFailed {
        key: String,
        stage: InstallStage,
        message: String,
    },
    RenderedOk {
        id: u64,
        key: String,
        frame: PreviewFrame,
        camera: ViewerCamera,
    },
    RenderFailed {
        message: String,
    },
    /// The render above this one ran the deferred lit texture stage
    /// — the scene's Lit notes, evaluate's and the stage's own, for the
    /// Console. Sends only when the stage ran; the notes wait for the first
    /// switch to Lit just like the textures do.
    LitTextures {
        key: String,
        notes: Vec<scene_view::preview::LitNote>,
    },
}

/// Handle for retiring the worker thread when the display loop exits.
pub(super) struct GpuHandle {
    pub requests: mpsc::Sender<GpuRequest>,
    join: std::thread::JoinHandle<()>,
}

/// Spawn the worker thread. The renderer is created lazily on the first
/// successful package parse, so environments without a Vulkan adapter
/// (plain `cargo test`) never touch one.
pub(super) fn spawn() -> (GpuHandle, mpsc::Receiver<GpuResult>) {
    let (requests, requests_rx) = mpsc::channel::<GpuRequest>();
    let (results, results_rx) = mpsc::channel::<GpuResult>();
    let join = thread::Builder::new()
        .name("preview-gpu".to_owned())
        .spawn(move || worker(requests_rx, results))
        .expect("spawn preview gpu thread");
    (GpuHandle { requests, join }, results_rx)
}

impl GpuHandle {
    /// Send `Shutdown` and join the worker thread.
    pub(super) fn stop(self) {
        let _ = self.requests.send(GpuRequest::Shutdown);
        let _ = self.join.join();
    }
}

#[cfg(test)]
impl GpuHandle {
    /// A handle wired to channels the test owns; its join target is a
    /// finished no-op thread.
    pub(super) fn for_tests(requests: mpsc::Sender<GpuRequest>) -> Self {
        Self {
            requests,
            join: thread::Builder::new()
                .name("preview-gpu-test".to_owned())
                .spawn(|| {})
                .expect("test thread"),
        }
    }

    /// A headless handle for coordinator-side fixtures: sends land in a
    /// dropped channel and fail silently — no worker, no adapter.
    pub(super) fn orphan() -> Self {
        Self::for_tests(mpsc::channel().0)
    }
}

/// A receiver for fixtures whose results are all injected by the test.
#[cfg(test)]
pub(super) fn orphan_rx() -> mpsc::Receiver<GpuResult> {
    mpsc::channel().1
}

fn worker(requests: mpsc::Receiver<GpuRequest>, results: mpsc::Sender<GpuResult>) {
    let mut renderer: Option<PreviewRenderer> = None;
    let mut editions = MeshEditions::default();
    let mut mesh_bounds = MeshBounds::default();
    let mut installed_key: Option<String> = None;
    for request in requests {
        match request {
            GpuRequest::Shutdown => break,
            GpuRequest::Clear => {
                if let Some(renderer) = renderer.as_mut() {
                    renderer.clear_scene();
                }
                editions = MeshEditions::default();
                mesh_bounds = MeshBounds::default();
                installed_key = None;
            }
            GpuRequest::UpdateDraws { key, draws, adopt } => {
                // A gizmo delta computed against the previous geometry must
                // never land on the newly installed scene.
                if installed_key.as_deref() == Some(key.as_str())
                    && let Some(renderer) = renderer.as_mut()
                {
                    let _ = renderer.update_draws(&draws);
                    if let Some(adopted) = adopt.as_deref() {
                        installed_key = Some(adopted.to_owned());
                    }
                }
            }
            GpuRequest::Install {
                key,
                job_id,
                slot,
                manifest,
            } => {
                match install(
                    &mut renderer,
                    &mut editions,
                    &mut mesh_bounds,
                    &key,
                    &job_id,
                    &slot,
                    manifest,
                ) {
                    Ok(data) => {
                        installed_key = Some(key);
                        let _ = results.send(GpuResult::Installed(Box::new(data)));
                    }
                    Err((stage, message)) => {
                        let _ = results.send(GpuResult::InstallFailed {
                            key,
                            stage,
                            message,
                        });
                    }
                }
            }
            GpuRequest::Render {
                id,
                key,
                camera,
                size,
                mode,
                time,
                highlight,
            } => {
                if installed_key.as_deref() != Some(key.as_str()) {
                    continue;
                }
                let Some(renderer) = renderer.as_mut() else {
                    continue;
                };
                let started = Instant::now();
                match renderer.render(camera, size[0], size[1], mode, time) {
                    Ok(frame) => {
                        // Design order: render → outline → rgb-a conversion,
                        // then the frame_render record covers the whole
                        // worker-side finish.
                        let frame = finish_frame(frame, &highlight);
                        preview_timing::record("frame_render", started, None, &key, Some(id));
                        // A render that ran the deferred lit
                        // texture stage reports it as the same
                        // `lit_textures` stage the install used to print —
                        // the install's line read zero — and the notes the
                        // stage folded
                        // reach the Console with the same switch to Lit.
                        if let Some(stage) = renderer.take_rendered_lit_textures() {
                            preview_timing::record_duration(
                                "lit_textures",
                                stage.timings.lit_textures,
                                None,
                                &key,
                                Some(id),
                            );
                            if !stage.notes.is_empty() {
                                let _ = results.send(GpuResult::LitTextures {
                                    key: key.clone(),
                                    notes: stage.notes,
                                });
                            }
                        }
                        let _ = results.send(GpuResult::RenderedOk {
                            id,
                            key,
                            frame,
                            camera,
                        });
                    }
                    Err(message) => {
                        let _ = results.send(GpuResult::RenderFailed { message });
                    }
                }
            }
        }
    }
}

/// One install: parse the package, compute the coordinator's metadata, and
/// upload into the renderer. Moved from the old inline
/// `poll_project_preview` success branch; every timing stage keeps its
/// name so scratch measurements stay comparable.
/// orders the reads: manifest first, the staging and the
/// blob decision on the manifest alone, then only the needed blobs off
/// the disk.
/// `manifest_override` carries the fast path's retargeted
/// manifest instead of reading `geometry.json`; the slot still answers
/// the blobs and diagnostics, and the result is marked so the
/// coordinator keeps its transform base.
fn install(
    renderer: &mut Option<PreviewRenderer>,
    editions: &mut MeshEditions,
    mesh_bounds: &mut MeshBounds,
    key: &str,
    job_id: &str,
    slot: &std::path::Path,
    manifest_override: Option<Box<GeometryManifest>>,
) -> Result<InstalledData, (InstallStage, String)> {
    let started = Instant::now();
    let keeps_transform_base = manifest_override.is_some();
    let manifest = match manifest_override {
        Some(manifest) => *manifest,
        None => scene_parse::read_preview_manifest(slot)
            .map_err(|error| (InstallStage::Parse, error.to_string()))?,
    };
    let (staged_editions, mesh_editions) = editions
        .staged_update(&manifest)
        .map_err(|error| (InstallStage::Parse, error))?;
    preview_timing::record("mesh_editions", started, Some(job_id), key, None);
    let started = Instant::now();
    // The renderer's committed identities decide which image blobs an
    // update can proceed without; everything else is read.
    let needed = crate::project_preview::needed_blob_names(
        editions,
        &staged_editions,
        mesh_bounds,
        &manifest,
        |name| {
            renderer
                .as_ref()
                .is_some_and(|renderer| renderer.resident_blob(name, &manifest))
        },
    );
    let blobs = scene_parse::read_preview_blobs(slot, &manifest, &needed)
        .map_err(|error| (InstallStage::Parse, error.to_string()))?;
    let diagnostics = scene_parse::read_preview_diagnostics(slot)
        .map_err(|error| (InstallStage::Parse, error.to_string()))?;
    preview_timing::record("package_read", started, Some(job_id), key, None);
    let bindings: BTreeMap<u32, String> = manifest
        .draws
        .iter()
        .filter_map(|draw| {
            draw.entity_id
                .as_ref()
                .map(|entity| (draw.pick_id, entity.as_str().to_owned()))
        })
        .collect();
    let started = Instant::now();
    let bounds = mesh_bounds
        .object_bounds(&manifest, &blobs)
        .map_err(|error| (InstallStage::Parse, error))?;
    preview_timing::record("bounds", started, Some(job_id), key, None);
    if renderer.is_none() {
        let started = Instant::now();
        *renderer = Some(PreviewRenderer::new().map_err(|error| (InstallStage::Upload, error))?);
        preview_timing::record("renderer_setup", started, Some(job_id), key, None);
    }
    let renderer = renderer.as_mut().expect("the renderer was just ensured");
    let started = Instant::now();
    renderer
        .update_scene(&manifest, &blobs, &mesh_editions)
        .map_err(|error| (InstallStage::Upload, error))?;
    preview_timing::record("gpu_scene_update", started, Some(job_id), key, None);
    // The split of gpu_scene_update: the renderer reports each stage's
    // share of the line above, in the order the stages ran. The
    // lit texture stage defers to the first Lit render, so its line here
    // measures zero; the render that runs the stage prints the real
    // `lit_textures` line.
    let timings = renderer.last_update_timings();
    preview_timing::record_duration("gpu_meshes", timings.mesh, Some(job_id), key, None);
    preview_timing::record_duration(
        "lit_evaluate",
        timings.lit_evaluate,
        Some(job_id),
        key,
        None,
    );
    preview_timing::record_duration(
        "lit_textures",
        timings.lit_textures,
        Some(job_id),
        key,
        None,
    );
    preview_timing::record_duration("environment", timings.environment, Some(job_id), key, None);
    // The upload survived, so its digest history survives with it: commit
    // the staged ledger here. An earlier commit lets a failed install prune
    // the record of meshes it omits, and a later re-add with changed
    // content would then reuse the live scene's edition 1 — rendering the
    // outdated buffers.
    *editions = staged_editions;
    // The deferred texture stage has not run, so these are the
    // evaluate notes; the texture folds join on the first Lit render,
    // through the render result's LitTextures message.
    let lit_notes = renderer.lit_notes().to_vec();
    Ok(InstalledData {
        key: key.to_owned(),
        manifest,
        bindings,
        bounds,
        lit_notes,
        diagnostics,
        slot: slot.to_owned(),
        keeps_transform_base,
    })
}

/// WGPU reads back BGRA while the image crate wants rgba; swap in place.
pub(super) fn swap_bgra_to_rgba(rgba: &mut [u8]) {
    for pixel in rgba.as_chunks_mut::<4>().0 {
        pixel.swap(0, 2);
    }
}

/// Outline the selection and convert the readback in one place, so the
/// frame the coordinator publishes paints as-is and the UI thread keeps
/// none of the old per-frame conversion. The image is detached from the GPUI
/// thread). The order matches the old serial pipeline exactly — the
/// outline is painted on the BGRA readback, then the channels swap — so
/// the on-screen outline color stays byte-identical to before.
fn finish_frame(mut frame: PreviewFrame, highlight: &BTreeSet<u32>) -> PreviewFrame {
    outline_selection(
        &mut frame.rgba,
        &frame.pick_ids,
        frame.width as usize,
        highlight,
    );
    swap_bgra_to_rgba(&mut frame.rgba);
    frame
}

/// Compile-time ownership proof for the worker thread (wgpu 30 handles are
/// Arc-based and Send+Sync; the doc comment on `PreviewRenderer` says one
/// renderer per display thread as a design premise, not a limit).
#[allow(dead_code)]
fn assert_handles_cross_threads() {
    fn assert_send<T: Send>() {}
    assert_send::<PreviewRenderer>();
    assert_send::<MeshEditions>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn swap_bgra_to_rgba_swaps_channels() {
        let mut rgba = [10u8, 11, 12, 255, 1, 2, 3, 255];
        swap_bgra_to_rgba(&mut rgba);
        assert_eq!(rgba, [12, 11, 10, 255, 3, 2, 1, 255]);
    }

    #[test]
    fn shutdown_joins_promptly_with_no_work() {
        let (handle, _results) = spawn();
        let started = std::time::Instant::now();
        handle.stop();
        assert!(
            started.elapsed() < std::time::Duration::from_secs(2),
            "shutdown join lagged: {:?}",
            started.elapsed()
        );
    }

    #[test]
    fn install_reports_parse_failure_for_missing_package() {
        let (handle, results) = spawn();
        handle
            .requests
            .send(GpuRequest::Install {
                key: "k".into(),
                job_id: "j".into(),
                slot: PathBuf::from("/nonexistent-preview-threading-slot"),
                manifest: None,
            })
            .unwrap();
        let result = results
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("a gpu result");
        match result {
            GpuResult::InstallFailed {
                stage: InstallStage::Parse,
                message,
                ..
            } => assert!(!message.is_empty()),
            GpuResult::InstallFailed { .. } => panic!("expected the Parse stage"),
            _ => panic!("expected an InstallFailed result"),
        }
        // The failed install must not have left the thread wedged.
        handle.stop();
    }

    #[test]
    fn stale_update_draws_is_ignored() {
        let (handle, results) = spawn();
        handle
            .requests
            .send(GpuRequest::UpdateDraws {
                key: "stale".into(),
                draws: vec![],
                adopt: None,
            })
            .unwrap();
        handle
            .requests
            .send(GpuRequest::Render {
                id: 1,
                key: "stale".into(),
                camera: ViewerCamera::frame(None, 16. / 9.),
                size: [8, 8],
                mode: PreviewMode::Solid,
                time: 0.,
                highlight: BTreeSet::new(),
            })
            .unwrap();
        assert!(
            results
                .recv_timeout(std::time::Duration::from_millis(100))
                .is_err(),
            "requests with a stale key must be dropped without any result"
        );
        // The worker must still be serving after the stale traffic.
        handle.stop();
    }

    #[test]
    fn outline_paint_survives_the_rgba_swap() {
        // One row of five: unselected, unselected, selected,
        // unselected, unselected. The outline paints both sides of the
        // silhouette; the outer pixels pass through — with their channels
        // swapped, which is the point.
        let frame = PreviewFrame {
            rgba: vec![
                1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255, 1, 2, 3, 255,
            ],
            pick_ids: vec![0, 0, 7, 0, 0],
            width: 5,
            height: 1,
            backend: "headless".into(),
        };
        let frame = finish_frame(frame, &BTreeSet::from([7]));
        // Untouched pixels swap their channels: BGRA [1,2,3] → rgba [3,2,1].
        assert_eq!(&frame.rgba[0..4], &[3, 2, 1, 255]);
        assert_eq!(&frame.rgba[16..20], &[3, 2, 1, 255]);
        // The outline constant paints [255, 160, 40] into the BGRA bytes on
        // both sides of the edge; the channel swap then reorders it, so
        // the displayed outline is rgb — byte-identical to
        // the old serial pipeline.
        assert_eq!(&frame.rgba[4..8], &[40, 160, 255, 255]);
        assert_eq!(&frame.rgba[8..12], &[40, 160, 255, 255]);
        assert_eq!(&frame.rgba[12..16], &[40, 160, 255, 255]);
    }
}
