//! Serial display driver and explicit phase ordering.
use super::*;
pub(super) fn run(
    core: CoreHandle,
    rx: mpsc::Receiver<Command>,
    tx: mpsc::Sender<Update>,
    frames: Arc<Mutex<Option<FrameUpdate>>>,
    thumbnail_mailbox: Arc<Mutex<ThumbnailMailbox>>,
    initial: Option<Startup>,
) {
    let (gpu, gpu_rx) = gpu::spawn();
    // Take this process's preview-slot owner lock before any preview
    // request can create a slot, and keep the guard in the display state
    // for the process lifetime. The startup sweep of slots orphaned by
    // desktops that never exited runs once on its own thread: startup
    // does not wait for the GBs such a sweep may delete.
    let temp_dir = std::env::temp_dir();
    let pid = std::process::id();
    let (owner, own_pid_stale) = preview_sweep::take_owner_lock(&temp_dir, pid);
    if own_pid_stale {
        preview_sweep::remove_own_slots(&temp_dir, pid);
    }
    let mut display = Display {
        core,
        tx,
        frames,
        thumbnail_mailbox,
        stopping: false,
        gpu,
        gpu_rx,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        preview_ready: false,
        camera: ViewerCamera::frame(None, 16. / 9.),
        scene_camera: None,
        view_revision: None,
        mode: PreviewMode::Solid,
        size: [1280, 720],
        dirty: false,
        frame_id: 0u64,
        last_frame: None,
        last_state: Value::Null,
        last_jobs: Value::Null,
        job_ages: Default::default(),
        last_artifact: None,
        last_settings: Value::Null,
        last_project: Value::Null,
        selected_project_node: None,
        project_items: None,
        project_preview_key: None,
        project_camera_owner: None,
        project_preview_job: None,
        import_jobs: Vec::new(),
        foreground_jobs: Vec::new(),
        foreground_preview_pending: false,
        last_foreground_jobs: Value::Null,
        save_job: None,
        export_jobs: Vec::new(),
        preserve_camera: None,
        project_preview_cleanup: Vec::new(),
        preview_slot_owner: owner,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::<u32, String>::new(),
        project_tree_snapshot: Value::Null,
        project_object_bounds: BTreeMap::new(),
        project_time: 0.0,
        ipr_due: None,
        view_due: None,
        highlight: BTreeSet::new(),
        highlight_key: None,
        logged_lit_notes: Vec::new(),
        logged_preview_warnings: Vec::new(),
        thumbnail_project: Value::Null,
        thumbnail_generation: 0,
        thumbnail_candidates: Default::default(),
        thumbnail_flight: None,
        preview_queue: Default::default(),
        preview_flight: None,
    };
    thread::spawn(move || preview_sweep::sweep_orphaned_slots(&temp_dir, pid));
    if let Some(startup) = initial {
        let started = match startup {
            Startup::Scene(path) => import_viewer(&display.core, path),
            Startup::Project(path) => open_project(&display.core, path),
        };
        match started {
            Ok(Some(job_id)) => {
                display.import_jobs.push(job_id.clone());
                display
                    .foreground_jobs
                    .push((job_id, "Open scene".into(), 0));
            }
            Ok(None) => {}
            Err(error) => {
                let _ = display.tx.send(Update::Status(error));
            }
        }
    }
    loop {
        let first = match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(command) => Some(command),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        display.handle_commands(first.into_iter().chain(rx.try_iter()));
        if display.stopping {
            break;
        }
        display.sync_state();
        display.poll_import_report();
        display.poll_save_job();
        display.poll_export_warnings();
        display.poll_project_preview();
        display.poll_cleanup();
        display.poll_results();
        display.poll_gpu();
        display.issue_gpu();
        display.poll_ipr_submit();
        display.poll_view_sync();
        display.poll_thumbnail();
        display.poll_preview();
        display.start_thumbnail();
        display.start_preview();
    }
    display.cleanup();
    display.gpu.stop();
}
