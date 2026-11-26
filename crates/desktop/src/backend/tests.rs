use super::*;
use image::GenericImageView as _;

#[test]
fn preview_key_changes_with_revision_or_active_scene() {
    let project = json!({
        "project_id":"p",
        "revision":"r1",
        "activation":"a1"
    });
    let same_scene = json!("scene-a");
    let first = project_preview_key(&project, &same_scene);
    assert_eq!(first, project_preview_key(&project, &same_scene));
    assert_ne!(first, project_preview_key(&project, &json!("scene-b")));
    let changed_revision = json!({"project_id":"p","revision":"r2","activation":"a1"});
    assert_ne!(first, project_preview_key(&changed_revision, &same_scene));
}

/// `PreviewKind::of_entry` says which Contents entries preview
/// at all — textures, materials and image assets — and every kind maps to
/// its wire method and the id field of both directions.
#[test]
fn preview_kind_classifies_contents_entries_and_maps_the_wire_identity() {
    let texture = json!({"kind":"resource","resource_kind":"texture","id":"t"});
    let material = json!({"kind":"resource","resource_kind":"material","id":"m"});
    assert_eq!(PreviewKind::of_entry(&texture), Some(PreviewKind::Texture));
    assert_eq!(
        PreviewKind::of_entry(&material),
        Some(PreviewKind::Material)
    );
    for media_type in protocol::project::IMAGE_PREVIEW_MEDIA_TYPES {
        let image = json!({"kind":"asset","media_type":media_type,"id":"a"});
        assert_eq!(
            PreviewKind::of_entry(&image),
            Some(PreviewKind::ImageAsset),
            "{media_type} previews as an image"
        );
    }
    // Everything else stays an icon: other resources, non-image assets,
    // scenes, folders and unregistered files.
    for entry in [
        json!({"kind":"resource","resource_kind":"medium","id":"md"}),
        json!({"kind":"resource","resource_kind":"object_definition","id":"od"}),
        json!({"kind":"asset","media_type":"application/x-ply","id":"ply"}),
        json!({"kind":"asset","id":"no-media-type"}),
        json!({"kind":"scene","id":"s"}),
        json!({"kind":"folder","location":"Images"}),
        json!({"kind":"unregistered","id":"u"}),
        json!({}),
    ] {
        assert_eq!(PreviewKind::of_entry(&entry), None, "{entry}");
    }
    assert_eq!(PreviewKind::Texture.method(), Method::ProjectPreviewTexture);
    assert_eq!(PreviewKind::Texture.id_field(), "resource_id");
    assert_eq!(
        PreviewKind::Material.method(),
        Method::ProjectPreviewMaterial
    );
    assert_eq!(PreviewKind::Material.id_field(), "resource_id");
    assert_eq!(
        PreviewKind::ImageAsset.method(),
        Method::ProjectPreviewAsset
    );
    assert_eq!(PreviewKind::ImageAsset.id_field(), "asset_id");
}

#[test]
fn foreground_job_rows_close_on_every_terminal_state_and_stop_cancel_spam() {
    for state in ["succeeded", "failed", "cancelled"] {
        assert!(
            super::results::foreground_job_row(
                "job-a",
                "Import scene",
                &json!({"state":state,"phase":"done"})
            )
            .is_none()
        );
    }
    let running = super::results::foreground_job_row(
        "job-a",
        "Import scene",
        &json!({"state":"running","phase":"parse"}),
    )
    .unwrap();
    assert_eq!(running["job_id"], "job-a");
    assert_eq!(running["phase"], "parse");
    assert_eq!(running["can_cancel"], true);
    let cancelling = super::results::foreground_job_row(
        "job-a",
        "Import scene",
        &json!({"state":"cancelling","phase":"parse"}),
    )
    .unwrap();
    assert_eq!(cancelling["can_cancel"], false);
}

#[test]
fn only_first_activation_or_active_scene_previews_join_foreground_jobs() {
    assert!(super::state_sync::should_foreground_project_preview(
        true, false
    ));
    assert!(super::state_sync::should_foreground_project_preview(
        false, true
    ));
    assert!(!super::state_sync::should_foreground_project_preview(
        false, false
    ));
}

#[test]
fn preview_slots_use_the_project_library_or_cache_library() {
    let base = tempfile::tempdir().unwrap();
    let project = base.path().join("project");
    let cache = base.path().join("cache");
    let reference = json!({"project_id":"a-b9","activation":12});
    let in_project =
        super::state_sync::preview_slot_path(&reference, Some(&project), Some(&cache)).unwrap();
    assert_eq!(
        in_project.parent().unwrap(),
        project.join("Library/Preview")
    );
    assert!(
        in_project
            .file_name()
            .unwrap()
            .to_string_lossy()
            .contains("-ab9-12")
    );

    let without_folder =
        super::state_sync::preview_slot_path(&reference, None, Some(&cache)).unwrap();
    assert_eq!(
        without_folder.parent().unwrap(),
        cache.join("Library/Preview")
    );
    assert!(super::state_sync::preview_slot_path(&reference, None, None).is_none());
}

#[test]
fn activation_change_retires_the_previous_preview_slot_and_store() {
    let (temp, host, first_project) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let core = host.handle();
    let (mut display, _updates, _gpu_results, _gpu_requests) =
        display_with_gpu_channels(core.clone());
    display.sync_state();
    let first_job = display.project_preview_job.as_ref().unwrap().0.clone();
    assert_eq!(
        wait_job(&core, json!({"job_id":first_job}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    let first_slot = super::state_sync::preview_slot_path(
        &first_project,
        Some(&temp.path().join("project")),
        None,
    )
    .unwrap();
    let first_store = super::preview_sweep::store_path_for_slot(&first_slot);
    assert!(first_slot.is_dir());
    assert!(first_store.is_dir());
    let stored_blob_count = std::fs::read_dir(&first_store).unwrap().count();

    request(
        &core,
        Method::ProjectApply,
        json!({"project":first_project,"edits":[
            {"kind":"create_folder","path":"Materials/Unused"}
        ]}),
    )
    .unwrap();
    display.sync_state();
    let second_edit_job = display.project_preview_job.as_ref().unwrap().0.clone();
    assert_eq!(
        wait_job(&core, json!({"job_id":second_edit_job}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    assert_eq!(
        std::fs::read_dir(&first_store).unwrap().count(),
        stored_blob_count,
        "the second edit reuses the activation's existing store entries"
    );

    let second_source = temp.path().join("second.pbrt");
    std::fs::write(&second_source, "WorldBegin\nShape \"disk\"\nWorldEnd\n").unwrap();
    let current = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let opened = request(
        &core,
        Method::ProjectImport,
        json!({"source_path":second_source,"destination":temp.path().join("second-project"),"expected_current":current["context"],"discard_unsaved":true}),
    )
    .unwrap();
    assert_eq!(wait_job(&core, opened)["state"], "succeeded");
    display.sync_state();
    assert!(!first_slot.exists(), "retired activation slot was removed");
    assert!(
        !first_store.exists(),
        "retired activation store was removed"
    );
    let second_job = display.project_preview_job.as_ref().unwrap().0.clone();
    assert_eq!(
        wait_job(&core, json!({"job_id":second_job}))["state"],
        "succeeded"
    );
}

#[test]
fn a_parked_refresh_on_the_same_slot_keeps_the_store() {
    let (_temp, host, project) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let core = host.handle();
    let (mut display, _updates, _gpu_results, _gpu_requests) =
        display_with_gpu_channels(core.clone());
    display.sync_state();
    let first_job = display.project_preview_job.as_ref().unwrap().0.clone();
    assert_eq!(
        wait_job(&core, json!({"job_id":first_job}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    let slot =
        super::state_sync::preview_slot_path(&project, Some(&_temp.path().join("project")), None)
            .unwrap();
    let store = super::preview_sweep::store_path_for_slot(&slot);
    assert!(std::fs::read_dir(&store).unwrap().count() > 0);

    // Two edits back to back: the second parks behind the first's job.
    for path in ["Materials/A", "Materials/B"] {
        let current = request(&core, Method::ProjectGetState, json!({})).unwrap();
        request(
            &core,
            Method::ProjectApply,
            json!({"project":current["context"]["project"],"edits":[
                {"kind":"create_folder","path":path}
            ]}),
        )
        .unwrap();
        display.sync_state();
    }
    let parked = display.project_preview_cleanup[0].job_id.clone();
    wait_job(&core, json!({"job_id":parked}));
    display.poll_cleanup();
    assert!(
        std::fs::read_dir(&store).unwrap().count() > 0,
        "the same activation's store survives a parked refresh"
    );
    let replacement = display.project_preview_job.as_ref().unwrap().0.clone();
    assert_eq!(
        wait_job(&core, json!({"job_id":replacement}))["state"],
        "succeeded"
    );
}

#[test]
fn reload_camera_is_written_for_its_target_then_discarded_after_preview_cancel_and_external_open() {
    let source = "WorldBegin\nAttributeBegin\nTranslate 0 0 0\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\nWorldEnd\n";
    let (temp, host, initial_project) = native_project(source);
    let core = host.handle();
    let source_path = temp.path().join("scene.pbrt");
    let (mut display, _updates) = bare_display(core.clone());
    display.last_project =
        request(&core, Method::AppGetState, json!({})).unwrap()["project"].clone();

    let mut original_camera = ViewerCamera::frame(None, 16. / 9.);
    original_camera.orbit(0.55, -0.18).unwrap();
    display.camera = original_camera.clone();
    let initial_view = request(
        &core,
        Method::ProjectGetView,
        json!({"project":initial_project}),
    )
    .unwrap();
    request(
        &core,
        Method::ProjectSetCamera,
        json!({
            "project":initial_project,
            "expected_view_revision":initial_view["view_revision"],
            "camera":original_camera.to_protocol(),
        }),
    )
    .unwrap();

    display.handle_commands([Command::ReloadViewer(
        source_path.to_string_lossy().into_owned(),
    )]);
    let reload_id = display.preserve_camera.as_ref().unwrap().job_id.clone();
    wait_job(&core, json!({"job_id":reload_id}));
    display.poll_import_report();
    let reload_target =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let reload_owner = camera_preservation::ProjectOwner::from_ref(&reload_target).unwrap();
    let manifest_camera = ViewerCamera::frame(
        Some(&protocol::Bounds {
            min: [395., 53., 29.],
            max: [399., 57., 31.],
        }),
        16. / 9.,
    );
    assert!(display.choose_camera_for_preview(&reload_owner, manifest_camera.clone()));
    assert_eq!(display.camera.to_protocol(), original_camera.to_protocol());
    let reload_view = request(
        &core,
        Method::ProjectGetView,
        json!({"project":reload_target}),
    )
    .unwrap();
    request(
        &core,
        Method::ProjectSetCamera,
        json!({
            "project":reload_target,
            "expected_view_revision":reload_view["view_revision"],
            "camera":display.camera.to_protocol(),
        }),
    )
    .unwrap();
    let shared = request(
        &core,
        Method::ProjectGetView,
        json!({"project":reload_target}),
    )
    .unwrap();
    assert_eq!(
        shared["view"]["camera"]["eye"],
        json!(original_camera.to_protocol().eye)
    );

    // A successfully imported reload whose preview is cancelled must lose its
    // camera intent before the next, unrelated external project activation.
    display.last_project =
        request(&core, Method::AppGetState, json!({})).unwrap()["project"].clone();
    display.handle_commands([Command::ReloadViewer(
        source_path.to_string_lossy().into_owned(),
    )]);
    let second_reload = display.preserve_camera.as_ref().unwrap().job_id.clone();
    wait_job(&core, json!({"job_id":second_reload}));
    display.poll_import_report();
    let cancelled_target =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let cancelled_owner = camera_preservation::ProjectOwner::from_ref(&cancelled_target).unwrap();
    display.fail_camera_preview_for(&cancelled_owner);
    assert!(display.preserve_camera.is_none());

    let external_source = temp.path().join("external.pbrt");
    std::fs::write(&external_source, source).unwrap();
    let before_external = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let external_job = request(
        &core,
        Method::ProjectImport,
        json!({
            "source_path":external_source,
            "expected_current":before_external["context"],
        }),
    )
    .unwrap();
    wait_job(&core, external_job.clone());
    display.last_project =
        request(&core, Method::AppGetState, json!({})).unwrap()["project"].clone();
    display.reconcile_camera_preservation();
    let external_project = display.last_project["context"]["project"].clone();
    let external_owner = camera_preservation::ProjectOwner::from_ref(&external_project).unwrap();
    assert!(!display.choose_camera_for_preview(&external_owner, manifest_camera.clone()));
    let external_view = request(
        &core,
        Method::ProjectGetView,
        json!({"project":external_project}),
    )
    .unwrap();
    request(
        &core,
        Method::ProjectSetCamera,
        json!({
            "project":external_project,
            "expected_view_revision":external_view["view_revision"],
            "camera":display.camera.to_protocol(),
        }),
    )
    .unwrap();
    let external_shared = request(
        &core,
        Method::ProjectGetView,
        json!({"project":external_project}),
    )
    .unwrap();
    assert_eq!(
        external_shared["view"]["camera"]["eye"],
        json!(manifest_camera.to_protocol().eye)
    );
}

fn native_project(source: &str) -> (tempfile::TempDir, app_core::CoreHost, Value) {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("scene.pbrt");
    std::fs::write(&path, source).unwrap();
    let host = app_core::CoreHost::start(protocol::commands::Limits::default());
    let core = host.handle();
    let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let job=request(&core,Method::ProjectImport,json!({"source_path":path,"destination":temp.path().join("project"),"expected_current":state["context"]})).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = request(&core, Method::JobGet, json!({"job_id":job["job_id"]})).unwrap();
        if state["state"] == "succeeded" {
            break;
        }
        assert_ne!(state["state"], "failed", "{state}");
        assert!(std::time::Instant::now() < deadline, "{state}");
        thread::sleep(Duration::from_millis(5));
    }
    let reference =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    (temp, host, reference)
}

#[test]
fn external_project_switch_allows_create_inspector_edit_and_undo() {
    let (temp, host, original) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\nWorldEnd\n",
    );
    let core = host.handle();
    let (mut display, _updates) = bare_display(core.clone());
    display.sync_state();

    let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    request(
        &core,
        Method::ProjectClose,
        json!({"expected_current":state["context"],"discard_unsaved":true}),
    )
    .unwrap();
    let empty_state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let path = temp.path().join("switched-project");
    let started = request(
        &core,
        Method::ProjectCreate,
        json!({"path":path,"title":"Switched Project",
            "expected_current":empty_state["context"],"discard_unsaved":true}),
    )
    .unwrap();
    assert_eq!(wait_job(&core, started)["state"], "succeeded");
    display.sync_state();
    let switched = display.last_project["context"]["project"].clone();
    assert_ne!(switched["project_id"], original["project_id"]);
    let roots = project_roots(&core, &switched).unwrap();
    let scene_id = roots.iter().find(|node| node["object"] == "scene").unwrap()["node_id"].clone();

    let create = json!({"kind":"create_preset","preset":project::presets::SHAPE_SPHERE.id,
        "parent":scene_id,"after":null,"view":null});
    serde_json::from_value::<protocol::project::ProjectEdit>(create.clone())
        .expect("Create edit must match ProjectEdit");
    display.handle_commands([Command::Apply {
        project: switched.clone(),
        edits: vec![create],
        follow: Follow::Nothing,
        form: None,
    }]);
    let created_state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let created_ref = created_state["context"]["project"].clone();
    let scene_parent = Some(scene_id.as_str().unwrap().to_owned());
    let tree = project_nodes_page(&core, &created_ref, &scene_parent, 0, 1000).unwrap();
    fn find_sphere(nodes: &[Value]) -> Option<String> {
        for node in nodes {
            if node["label"] == "Sphere" && node["kind"] == "entity" {
                return node["node_id"].as_str().map(str::to_owned);
            }
            if let Some(id) = node["children"]
                .as_array()
                .and_then(|children| find_sphere(children))
            {
                return Some(id);
            }
        }
        None
    }
    let sphere = find_sphere(tree["nodes"].as_array().unwrap())
        .unwrap_or_else(|| panic!("Sphere was created: {tree}"));
    let rename = json!({"kind":"rename_node","node_id":sphere,"label":"Edited Sphere"});
    serde_json::from_value::<protocol::project::ProjectEdit>(rename.clone())
        .expect("Inspector edit must match ProjectEdit");
    display.handle_commands([Command::Apply {
        project: created_ref,
        edits: vec![rename],
        follow: Follow::Nothing,
        form: None,
    }]);
    let edited = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let edited_ref = edited["context"]["project"].clone();
    let details = request(
        &core,
        Method::ProjectGetNode,
        json!({"project":edited_ref,"node_id":sphere}),
    )
    .unwrap();
    assert_eq!(details["label"], "Edited Sphere");

    request(&core, Method::ProjectUndo, json!({"project":edited_ref})).unwrap();
    let undone = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let details = request(
        &core,
        Method::ProjectGetNode,
        json!({"project":undone["context"]["project"],"node_id":sphere}),
    )
    .unwrap();
    assert_eq!(details["label"], "Sphere");
}

fn wait_job(core: &app_core::CoreHandle, job: Value) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = request(core, Method::JobGet, json!({"job_id":job["job_id"]})).unwrap();
        if ["succeeded", "failed", "cancelled"]
            .contains(&state["state"].as_str().unwrap_or_default())
        {
            return state;
        }
        assert!(std::time::Instant::now() < deadline, "{state}");
        thread::sleep(Duration::from_millis(5));
    }
}

/// Walk one lazily loaded branch: roots, then children of `parent`.
fn expanded_children(
    core: &app_core::CoreHandle,
    reference: &Value,
    parent: Option<&str>,
) -> Value {
    match parent {
        None => json!(project_roots(core, reference).unwrap()),
        Some(parent) => {
            let page =
                project_nodes_page(core, reference, &Some(parent.to_owned()), 0, 1000).unwrap();
            page["nodes"].clone()
        }
    }
}

#[test]
fn inspector_loads_resource_parameters_across_pages() {
    let parameters = (0..40)
        .map(|n| format!(" \"float custom{n}\" [{n}]"))
        .collect::<String>();
    let (_temp, host, reference) = native_project(&format!(
        "WorldBegin\nMakeNamedMaterial \"wall\" \"string type\" \"diffuse\"{parameters}\nNamedMaterial \"wall\"\nShape \"sphere\"\n"
    ));
    let core = host.handle();
    // Resources are listed by Contents, folder by folder.
    let mut pending = vec![String::new()];
    let mut material = None;
    while let Some(folder) = pending.pop() {
        let page = project_contents_page(&core, &reference, &folder, 0, 256).unwrap();
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else if entry["resource_kind"] == "material" {
                material = Some(entry.clone());
            }
        }
    }
    let material = material.expect("the material is listed in Contents");
    assert_eq!(material["reference_count"], 1, "{material}");
    let details = project_item_details(&core, &reference, &material).unwrap();
    assert_eq!(details["parameter_target"]["kind"], "resource");
    assert_eq!(details["resource"]["implementation"], "diffuse");
    assert_eq!(
        details["parameters"]["parameters"]
            .as_array()
            .unwrap()
            .len(),
        40
    );
    assert!(details["parameters"]["next_offset"].is_null());
    // The page carries the table's rows for the typed fields.
    let schema = details["parameters"]["schema"].as_array().unwrap();
    assert!(schema.iter().any(|row| row["name"] == "reflectance"));
}

#[test]
fn contents_thumbnail_requests_are_async_single_flight_and_use_64px_previews() {
    let (_temp, host, reference) = native_project(
        "WorldBegin\nTexture \"thumb\" \"spectrum\" \"constant\" \"rgb value\" [0.2 0.5 0.8]\nMakeNamedMaterial \"wall\" \"string type\" [\"diffuse\"] \"texture reflectance\" [\"thumb\"]\nNamedMaterial \"wall\"\nShape \"sphere\"\n",
    );
    let core = host.handle();
    let mut pending = vec![String::new()];
    let mut resources = BTreeMap::new();
    while let Some(folder) = pending.pop() {
        let page = project_contents_page(&core, &reference, &folder, 0, 256).unwrap();
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else if entry["kind"] == "resource" {
                resources.insert(
                    entry["resource_kind"].as_str().unwrap().to_owned(),
                    entry["id"].as_str().unwrap().to_owned(),
                );
            }
        }
    }
    let texture = resources.get("texture").unwrap().clone();
    let material = resources.get("material").unwrap().clone();
    let (mut display, rx) = bare_display(core);
    display.sync_state();
    display.thumbnail_project = reference.clone();
    display.thumbnail_generation = 4;

    for (resource_id, kind) in [
        (texture, PreviewKind::Texture),
        (material, PreviewKind::Material),
    ] {
        {
            let mut mailbox = display.thumbnail_mailbox.lock().unwrap();
            mailbox.project = reference.clone();
            mailbox.generation = 4;
            mailbox.visible = vec![(resource_id.clone(), kind)];
        }
        display.thumbnail_candidates = VecDeque::from([(resource_id, kind)]);
        display.start_thumbnail();
        assert!(display.thumbnail_flight.is_some());
        let active_id = display.thumbnail_flight.as_ref().unwrap().id.clone();
        display
            .thumbnail_candidates
            .push_back(("queued-behind-active".into(), PreviewKind::Texture));
        display.start_thumbnail();
        assert_eq!(display.thumbnail_flight.as_ref().unwrap().id, active_id);
        assert!(
            display
                .thumbnail_flight
                .as_ref()
                .unwrap()
                .receiver
                .is_some()
        );
        assert!(display.thumbnail_flight.as_ref().unwrap().request_id.len() > 10);

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let update = loop {
            display.poll_thumbnail();
            let mut found = None;
            for update in rx.try_iter() {
                if let Update::Thumbnail {
                    project,
                    id: replied_id,
                    kind: replied_kind,
                    generation,
                    request_id,
                    result,
                } = update
                {
                    assert_eq!(project, reference);
                    assert_eq!(replied_id, display.thumbnail_flight.as_ref().unwrap().id);
                    assert_eq!(replied_kind, kind);
                    assert_eq!(generation, 4);
                    let (path, key) = result.unwrap();
                    assert!(!key.is_empty());
                    let dimensions = image::ImageReader::open(&path)
                        .unwrap()
                        .decode()
                        .unwrap()
                        .dimensions();
                    assert_eq!(
                        dimensions,
                        (64, 64),
                        "kind={kind:?} resource={replied_id} path={path}"
                    );
                    found = Some(request_id);
                    break;
                }
            }
            if let Some(request_id) = found {
                break request_id;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "thumbnail response timeout"
            );
            thread::sleep(Duration::from_millis(2));
        };
        assert!(
            display
                .thumbnail_flight
                .as_ref()
                .unwrap()
                .receiver
                .is_none()
        );
        display.handle_commands([Command::ThumbnailDone(update)]);
        assert!(display.thumbnail_flight.is_none());
    }
}

/// An image asset's thumbnail reply matches through `asset_id` — a
/// `resource_id` reply never lands, whatever its id string is.
#[test]
fn image_asset_thumbnail_replies_match_through_asset_id() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    let project = json!({"project_id":"p","revision":"1","activation":"a"});
    let install = |display: &mut Display, receiver: mpsc::Receiver<protocol::Response>| {
        display.thumbnail_flight = Some(ThumbnailFlight {
            request_id: "req".into(),
            project: project.clone(),
            id: "asset-x".into(),
            kind: PreviewKind::ImageAsset,
            generation: 3,
            receiver: Some(receiver),
        });
    };

    let (reply, receiver) = mpsc::channel();
    install(&mut display, receiver);
    reply
        .send(protocol::Response::ok(
            "req",
            json!({"project":&project,"asset_id":"asset-x","path":"p.png","key":"k"}),
        ))
        .unwrap();
    display.poll_thumbnail();
    let update = rx
        .try_iter()
        .find_map(|update| match update {
            Update::Thumbnail {
                id, kind, result, ..
            } => Some((id, kind, result)),
            _ => None,
        })
        .expect("the matching reply must land its thumbnail update");
    assert_eq!(update.0, "asset-x");
    assert_eq!(update.1, PreviewKind::ImageAsset);
    let (path, key) = update
        .2
        .expect("a matched reply carries the cache identity");
    assert_eq!(path, "p.png");
    assert_eq!(key, "k");

    let (reply, receiver) = mpsc::channel();
    install(&mut display, receiver);
    reply
        .send(protocol::Response::ok(
            "req",
            json!({"project":&project,"resource_id":"asset-x","path":"p.png","key":"k"}),
        ))
        .unwrap();
    display.poll_thumbnail();
    let update = rx
        .try_iter()
        .find_map(|update| match update {
            Update::Thumbnail {
                id, kind, result, ..
            } => Some((id, kind, result)),
            _ => None,
        })
        .expect("the mismatched reply must answer its thumbnail update");
    assert_eq!(update.0, "asset-x");
    assert_eq!(update.1, PreviewKind::ImageAsset);
    assert_eq!(
        update.2.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );
}

/// The SpectrumData editor's backend reads the resource's whole
/// sample tables in one answer and the swatch colours per role — round
/// trip against a real core.
#[test]
fn spectrum_table_and_swatch_round_trip_through_the_backend() {
    let (_temp, host, project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    // A SpectrumData resource with a non-flat curve, applied directly.
    let applied = core.request(protocol::Request {
        protocol: "2".into(),
        request_id: "spectrum-create".into(),
        method: Method::ProjectApply,
        params: json!({"project":project,"edits":[{"kind":"create_resource",
            "resource_kind":"spectrum_data","implementation":"piecewise","label":"Gold k",
            "folder":"Spectra","parameters":[
                {"name":"wavelengths","value_type":"float","color_space":null,
                 "values":{"kind":"floats","value":[400.,600.]}},
                {"name":"values","value_type":"float","color_space":null,
                 "values":{"kind":"floats","value":[1.,0.25]}}]}]}),
    });
    let resource = applied.result.expect("apply must succeed")["created"][0]["id"]
        .as_str()
        .expect("the created resource id")
        .to_owned();
    // The reference the apply moved to.
    let state = core.request(protocol::Request {
        protocol: "2".into(),
        request_id: "spectrum-state".into(),
        method: Method::ProjectGetState,
        params: json!({}),
    });
    let project = state.result.expect("state")["context"]["project"].clone();

    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    display.handle_commands([
        Command::GetSpectrumTable {
            project: project.clone(),
            resource_id: resource.clone(),
        },
        Command::SpectrumSwatch {
            project: project.clone(),
            resource_id: resource.clone(),
            role: "illuminant".into(),
        },
        Command::SpectrumSwatch {
            project,
            resource_id: resource.clone(),
            role: "albedo".into(),
        },
    ]);
    let table = rx
        .try_iter()
        .find_map(|update| match update {
            Update::SpectrumTable { table, .. } => Some(table),
            _ => None,
        })
        .expect("the sample tables must answer")
        .expect("the table fetch must succeed");
    assert_eq!(table["wavelengths"], json!([400., 600.]));
    assert_eq!(table["values"], json!([1., 0.25]));
    assert_eq!(table["storage"], "floats");

    let pairs = [400., 1., 600., 0.25];
    let swatch = |role: &str| {
        rx.try_iter()
            .find_map(|update| match update {
                Update::SpectrumSwatch {
                    role: answered,
                    answer,
                    ..
                } => (answered == role).then_some(answer),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the {role} swatch must answer"))
            .expect("the swatch must succeed")
    };
    let illuminant = swatch("illuminant");
    assert_eq!(
        illuminant["rgb"],
        json!(spectrum::pairs_rgb(&pairs, spectrum::Role::Illuminant).unwrap())
    );
    assert_eq!(illuminant["source"], "SpectrumData: Gold k");
    let albedo = swatch("albedo");
    assert_eq!(
        albedo["rgb"],
        json!(spectrum::pairs_rgb(&pairs, spectrum::Role::Albedo).unwrap())
    );
    assert_eq!(albedo["source"], "SpectrumData: Gold k");
    drop(host);
}

/// A parameter row's approximate colour — the Inspector's colour
/// box — round-trips against a real core: an rgb reflectance, a
/// SpectrumData reference and a light's blackbody all resolve, the
/// answer naming the source the box's tooltip shows. The command runs on
/// the display thread like every other queued read, so the GPUI render
/// that asks for it never waits for the wire.
#[test]
fn parameter_spectrum_colours_round_trip_through_the_backend() {
    let (_temp, host, reference) = native_project(
        "WorldBegin\nLightSource \"point\" \"blackbody I\" [6500]\nMakeNamedMaterial \"wall\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.25 0.5 0.75]\nNamedMaterial \"wall\"\nShape \"sphere\"\n",
    );
    let core = host.handle();
    // A SpectrumData curve and a conductor material (applied over the
    // wire, through the real planner); k references the curve next.
    let applied = core.request(protocol::Request {
        protocol: "2".into(),
        request_id: "r2b-setup".into(),
        method: Method::ProjectApply,
        params: json!({"project":reference,"edits":[
            {"kind":"create_resource","resource_kind":"spectrum_data",
             "implementation":"piecewise","label":"Gold k","folder":"Spectra",
             "parameters":[
                 {"name":"wavelengths","value_type":"float","color_space":null,
                  "values":{"kind":"floats","value":[400.,600.]}},
                 {"name":"values","value_type":"float","color_space":null,
                  "values":{"kind":"floats","value":[1.,0.25]}}]},
            {"kind":"create_resource","resource_kind":"material","implementation":"conductor",
             "label":"Metal","folder":"Materials","parameters":[]}]}),
    });
    let created = applied.result.expect("the apply must succeed")["created"]
        .as_array()
        .cloned()
        .expect("the created ids");
    assert_eq!(created.len(), 2, "one id for each created resource");
    let spectrum_data = created[0]["id"].clone();
    let conductor = json!({"kind":"resource","id":created[1]["id"]});
    let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let project = state["context"]["project"].clone();
    request(
        &core,
        Method::ProjectApply,
        json!({"project":project,"edits":[
            {"kind":"set_parameter",
             "target":conductor,
             "name":"k","value_type":"spectrum",
             "value":{"name":"k","value_type":"spectrum","color_space":null,
                      "values":{"kind":"resources","value":{"kind":"spectrum_data",
                                                           "ids":[spectrum_data]}}}}]}),
    )
    .unwrap();
    let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let project = state["context"]["project"].clone();
    // The diffuse material from the scene, and the point light's node.
    let mut material = None;
    let mut pending = vec![String::new()];
    while let Some(folder) = pending.pop() {
        let page = project_contents_page(&core, &project, &folder, 0, 256).unwrap();
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else if entry["name"] == "wall" {
                material = Some(json!({"kind":"resource","id":entry["id"]}));
            }
        }
    }
    let material = material.expect("the wall material is listed in Contents");
    let mut light_node = None;
    let mut pending: Vec<Option<String>> = vec![None];
    while light_node.is_none()
        && let Some(parent) = pending.pop()
    {
        for node in expanded_children(&core, &project, parent.as_deref())
            .as_array()
            .cloned()
            .unwrap_or_default()
        {
            if node["object"] == "light" {
                light_node = Some(json!({"kind":"node","id":node["node_id"]}));
                break;
            }
            if let Some(node_id) = node["node_id"].as_str() {
                pending.push(Some(node_id.to_owned()));
            }
        }
    }
    let light_node = light_node.expect("the light is a scene node");

    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    display.handle_commands([
        Command::ParameterSpectrumColour {
            project: project.clone(),
            target: material,
            parameter: "reflectance".into(),
            role: None,
        },
        Command::ParameterSpectrumColour {
            project: project.clone(),
            target: conductor.clone(),
            parameter: "k".into(),
            role: Some("chromaticity".into()),
        },
        Command::ParameterSpectrumColour {
            project,
            target: light_node,
            parameter: "I".into(),
            role: None,
        },
    ]);
    let answer = |parameter: &str| {
        rx.try_iter()
            .find_map(|update| match update {
                Update::ParameterSpectrumColour {
                    parameter: answered,
                    answer,
                    ..
                } => (answered == parameter).then_some(answer),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the {parameter} colour must answer"))
            .expect("the colour must succeed")
    };
    // The rgb reflectance answers its channels, named for the tooltip.
    let rgb = answer("reflectance");
    assert_eq!(rgb["rgb"], json!([0.25, 0.5, 0.75]));
    assert_eq!(rgb["source"], "rgb 0.25 0.5 0.75");
    assert_eq!(rgb["role"], "albedo");
    // The SpectrumData reference answers through the curve — asks
    // for it by the chromaticity role, the curve's colour, magnitude-free
    // — and names the curve.
    let curve = answer("k");
    let pairs = [400., 1., 600., 0.25];
    assert_eq!(
        curve["rgb"],
        json!(spectrum::chromaticity(spectrum::Value::Pairs(&pairs)).unwrap())
    );
    assert_eq!(curve["source"], "SpectrumData: Gold k");
    assert_eq!(curve["role"], "chromaticity");
    // The light's blackbody answers the luminance-normalized colour of
    // its kelvin — the same one an rgb box and Lit would show.
    let blackbody = answer("I");
    for (got, want) in blackbody["rgb"]
        .as_array()
        .unwrap()
        .iter()
        .zip(spectrum::blackbody_rgb(6500.))
    {
        let (got, want) = (got.as_f64().unwrap(), want);
        assert!((got - want).abs() < 1e-9, "{blackbody}: {got} vs {want}");
    }
    assert_eq!(blackbody["source"], "blackbody 6500 K");
    assert_eq!(blackbody["role"], "illuminant");
    drop(host);
}

/// The edits the colour dialog builds — same-type updates and
/// cross-type tab switches — apply through a real core as one
/// transaction each, and one Undo step brings the previous value back.
#[test]
fn colour_dialog_edits_apply_and_undo_as_one_step() {
    let (_temp, host, reference) = native_project(
        "WorldBegin\nMakeNamedMaterial \"wall\" \"string type\" \"diffuse\" \"rgb reflectance\" [0.25 0.5 0.75]\nNamedMaterial \"wall\"\nShape \"sphere\"\n",
    );
    let core = host.handle();
    let mut material = None;
    let mut pending = vec![String::new()];
    while let Some(folder) = pending.pop() {
        let page = project_contents_page(&core, &reference, &folder, 0, 256).unwrap();
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else if entry["name"] == "wall" {
                material = Some(json!({"kind":"resource","id":entry["id"]}));
            }
        }
    }
    let material = material.expect("the wall material");
    // The row the dialog edits, built the way the Inspector builds it:
    // declared parameters plus the schema table.
    let details = project_item_details(&core, &reference, &material).unwrap();
    let row = crate::editor::parameter_rows::rows(&details["parameters"])
        .into_iter()
        .find(|row| row.name == "reflectance")
        .expect("the schema carries reflectance");
    assert_eq!(row.value_type, "rgb");
    let reference =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();

    // A same-type edit.
    let edits = crate::editor::parameter_rows::colour_edits(
        &material,
        &row,
        "rgb",
        json!({"kind":"floats","value":[0.5, 0.25, 0.]}),
    );
    for edit in &edits {
        serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
            .unwrap_or_else(|error| panic!("{error}: {edit}"));
    }
    request(
        &core,
        Method::ProjectApply,
        json!({"project":reference,"edits":edits}),
    )
    .unwrap();

    // The tab switch to blackbody replaces the declaration in one
    // transaction.
    let reference =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let row = {
        let details = project_item_details(&core, &reference, &material).unwrap();
        crate::editor::parameter_rows::rows(&details["parameters"])
            .into_iter()
            .find(|row| row.name == "reflectance")
            .unwrap()
    };
    let edits = crate::editor::parameter_rows::tab_default_edits(
        &material,
        &row,
        crate::editor::parameter_rows::SpectrumTab::Blackbody,
        &json!({}),
        &[],
    );
    assert_eq!(edits.len(), 2, "the removal and the declaration: {edits:?}");
    request(
        &core,
        Method::ProjectApply,
        json!({"project":reference,"edits":edits}),
    )
    .unwrap();

    let reference =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let parameters = request(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference,"target":material,"offset":0,"limit":32}),
    )
    .unwrap();
    let blackbody = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == "reflectance")
        .unwrap();
    assert_eq!(blackbody["value_type"], "blackbody", "{blackbody}");
    assert_eq!(
        blackbody["values"]["value"],
        json!([{"kind":"finite","value":6500.0}])
    );

    // One Undo step returns the rgb triple the switch replaced.
    request(&core, Method::ProjectUndo, json!({"project":reference})).unwrap();
    let reference =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let parameters = request(
        &core,
        Method::ProjectGetParameters,
        json!({"project":reference,"target":material,"offset":0,"limit":32}),
    )
    .unwrap();
    let restored = parameters["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .find(|parameter| parameter["name"] == "reflectance")
        .unwrap();
    assert_eq!(restored["value_type"], "rgb", "{restored}");
    assert_eq!(
        restored["values"]["value"],
        json!([
            {"kind":"finite","value":0.5},
            {"kind":"finite","value":0.25},
            {"kind":"finite","value":0.0}
        ]),
        "the same-type edit's value, one step further back is the scene's triple"
    );
    drop(host);
}

/// A SpectrumData resource's Inspector page lists its two sample
/// tables as known rows (the schema table behind `piecewise`), so the
/// page stops claiming "not in this implementation's parameter table"
/// — the wire serves the table and the desktop attaches it.
#[test]
fn the_spectrum_data_inspector_page_knows_its_sample_tables() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    // The wire answer for the new directive.
    let schema = request(
        &core,
        Method::ProjectGetParameterSchema,
        json!({"directive":"SpectrumData","implementation":"piecewise"}),
    )
    .unwrap();
    let names = schema["parameters"]
        .as_array()
        .unwrap()
        .iter()
        .map(|spec| spec["name"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(names, ["wavelengths", "values"]);
    // A created curve's Inspector details carry the rows the typed
    // fields build from.
    let applied = core.request(protocol::Request {
        protocol: "2".into(),
        request_id: "r2b-spectrum".into(),
        method: Method::ProjectApply,
        params: json!({"project":reference,"edits":[{"kind":"create_resource",
            "resource_kind":"spectrum_data","implementation":"piecewise",
            "label":"Gold k","folder":"Spectra","parameters":[
                {"name":"wavelengths","value_type":"float","color_space":null,
                 "values":{"kind":"floats","value":[400.,600.]}},
                {"name":"values","value_type":"float","color_space":null,
                 "values":{"kind":"floats","value":[1.,0.25]}}]}]}),
    });
    let id = applied.result.unwrap()["created"][0]["id"].clone();
    let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
    let project = state["context"]["project"].clone();
    let details = project_item_details(
        &core,
        &project,
        &json!({"kind":"resource","resource_kind":"spectrum_data","id":id}),
    )
    .unwrap();
    let rows = crate::editor::parameter_rows::rows(&details["parameters"]);
    assert_eq!(
        rows.iter().map(|row| row.name.as_str()).collect::<Vec<_>>(),
        ["wavelengths", "values"]
    );
    for row in &rows {
        assert!(row.known, "no amber note: {row:?}");
        assert!(row.declared);
        assert_eq!(row.value_type, "float");
        assert!(matches!(
            row.field,
            crate::editor::parameter_rows::Field::Json
        ));
    }
    drop(host);
}

#[test]
fn editor_open_result_carries_project_and_request_identity() {
    let (_temp, host, project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    let entry = json!({"kind":"resource","resource_kind":"material","id":"mat-a"});
    display.handle_commands([Command::OpenEditor {
        project: project.clone(),
        entry: entry.clone(),
        request_id: 73,
    }]);
    assert!(rx.try_iter().any(|update| matches!(update,
        Update::EditorReady { project: reply_project, request_id: 73, entry: reply_entry, path: Ok(None) }
            if reply_project == project && reply_entry == entry
    )));
}

#[test]
fn queued_contents_thumbnail_that_left_visibility_is_cancelled_before_core_submit() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    display.thumbnail_project = reference.clone();
    display.thumbnail_generation = 8;
    display.thumbnail_candidates = VecDeque::from([("queued-A".into(), PreviewKind::Texture)]);
    {
        let mut mailbox = display.thumbnail_mailbox.lock().unwrap();
        mailbox.project = reference;
        mailbox.generation = 8;
        mailbox.visible = vec![("visible-B".into(), PreviewKind::Material)];
    }

    display.start_thumbnail();
    let (request_id, result) = rx
        .try_iter()
        .find_map(|update| match update {
            Update::Thumbnail {
                request_id, result, ..
            } => Some((request_id, result)),
            _ => None,
        })
        .expect("stale visibility must produce a terminal completion");
    assert_eq!(result.unwrap_err().code, protocol::ErrorCode::Cancelled);
    assert!(
        display
            .thumbnail_flight
            .as_ref()
            .unwrap()
            .receiver
            .is_none(),
        "Core must not receive A"
    );
    assert_eq!(
        display.thumbnail_mailbox.lock().unwrap().visible,
        vec![("visible-B".into(), PreviewKind::Material)]
    );
    display.handle_commands([Command::ThumbnailDone(request_id)]);
    assert!(
        display.thumbnail_flight.is_none(),
        "terminal ACK releases the one-slot gate"
    );
}

#[test]
fn stale_contents_thumbnail_candidate_sends_completion_and_releases_the_slot() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    assert_eq!(display.last_project["context"]["project"], reference);
    let mut stale = reference.clone();
    stale["revision"] = json!("stale-revision");
    display.thumbnail_project = stale.clone();
    display.thumbnail_generation = 12;
    display.thumbnail_candidates =
        VecDeque::from([("stale-resource".into(), PreviewKind::Texture)]);

    display.start_thumbnail();

    let update = rx
        .try_iter()
        .find_map(|update| match update {
            Update::Thumbnail {
                project,
                id,
                kind,
                generation,
                request_id,
                result,
            } => Some((project, id, kind, generation, request_id, result)),
            _ => None,
        })
        .expect("stale candidates must receive a completion update");
    assert_eq!(update.0, stale);
    assert_eq!(update.1, "stale-resource");
    assert_eq!(update.2, PreviewKind::Texture);
    assert_eq!(update.3, 12);
    assert_eq!(
        update.5.unwrap_err().code,
        protocol::ErrorCode::StaleProject
    );
    assert!(display.thumbnail_flight.is_some());
    assert!(
        display
            .thumbnail_flight
            .as_ref()
            .unwrap()
            .receiver
            .is_none()
    );

    display.handle_commands([Command::ThumbnailDone(update.4)]);
    assert!(display.thumbnail_flight.is_none());
}

/// One preview reply, drained from the update stream (other update kinds
/// from the state sync may interleave).
fn take_preview_update(
    rx: &mpsc::Receiver<Update>,
) -> (
    Value,
    String,
    Result<(String, String), protocol::ErrorPayload>,
) {
    rx.try_iter()
        .find_map(|update| match update {
            Update::Preview {
                project,
                id,
                result,
            } => Some((project, id, result)),
            _ => None,
        })
        .expect("the preview flight must answer its Update stream")
}

/// Give the display a preview flight whose reply channel the test owns.
fn install_preview_flight(
    display: &mut Display,
    project: Value,
    request_id: &str,
    id: &str,
    receiver: mpsc::Receiver<protocol::Response>,
) {
    display.preview_flight = Some(PreviewFlight {
        request_id: request_id.to_owned(),
        project,
        id: id.to_owned(),
        kind: PreviewKind::Texture,
        receiver: Some(receiver),
    });
}

/// The Inspector/node-editor preview path. `Command::Preview`
/// used to call blocking `request` on the display thread; now the handler
/// only queues (nothing in flight yet), the display loop submits one
/// request at a time and polls the reply, and the reply's Update clears
/// the slot without any ack.
#[test]
fn inspector_previews_queue_without_blocking_and_flow_back_at_128px() {
    let (_temp, host, reference) = native_project(
        "WorldBegin\nTexture \"preview\" \"spectrum\" \"constant\" \"rgb value\" [0.2 0.5 0.8]\nMakeNamedMaterial \"wall\" \"string type\" [\"diffuse\"] \"texture reflectance\" [\"preview\"]\nNamedMaterial \"wall\"\nShape \"sphere\"\n",
    );
    let core = host.handle();
    let mut pending = vec![String::new()];
    let mut texture = None;
    while let Some(folder) = pending.pop() {
        let page = project_contents_page(&core, &reference, &folder, 0, 256).unwrap();
        for entry in page["entries"].as_array().unwrap() {
            if entry["kind"] == "folder" {
                pending.push(entry["location"].as_str().unwrap().to_owned());
            } else if entry["resource_kind"] == "texture" {
                texture = Some(entry["id"].as_str().unwrap().to_owned());
            }
        }
    }
    let texture = texture.expect("the texture is listed in Contents");
    let (mut display, rx) = bare_display(core);
    display.sync_state();

    // Handling only queues: core is not called from the command handler.
    display.handle_commands([Command::Preview {
        project: reference.clone(),
        id: texture.clone(),
        kind: PreviewKind::Texture,
    }]);
    assert_eq!(display.preview_queue.len(), 1);
    assert!(
        display.preview_flight.is_none(),
        "the handler must not submit synchronously"
    );

    // The display loop's start/poll pair submits one at a time.
    display.start_preview();
    let active = display.preview_flight.as_ref();
    assert_eq!(
        active.map(|flight| flight.id.as_str()),
        Some(texture.as_str())
    );
    assert!(active.is_some_and(|flight| flight.receiver.is_some()));
    display.start_preview();
    assert_eq!(
        display
            .preview_flight
            .as_ref()
            .map(|flight| flight.id.as_str()),
        Some(texture.as_str()),
        "a busy flight is never displaced"
    );
    // Asking again for the request in flight queues nothing: its reply
    // answers both.
    display.handle_commands([Command::Preview {
        project: reference.clone(),
        id: texture.clone(),
        kind: PreviewKind::Texture,
    }]);
    assert!(
        display.preview_queue.is_empty(),
        "the in-flight request already answers it"
    );

    let deadline = Instant::now() + Duration::from_secs(5);
    let update = loop {
        display.poll_preview();
        if let Ok(update) = rx.try_recv()
            && let Update::Preview { .. } = update
        {
            break update;
        }
        assert!(Instant::now() < deadline, "preview response timeout");
        thread::sleep(Duration::from_millis(2));
    };
    let Update::Preview {
        project,
        id,
        result,
    } = update
    else {
        unreachable!("already matched");
    };
    assert_eq!(project, reference);
    assert_eq!(id, texture);
    let (path, _key) = result.unwrap();
    let dimensions = image::ImageReader::open(&path)
        .unwrap()
        .decode()
        .unwrap()
        .dimensions();
    assert_eq!(dimensions, (128, 128), "path={path}");
    assert!(
        display.preview_flight.is_none(),
        "the Update that carries the reply clears the slot (no ack)"
    );
}

/// The queue deduplicates within a project, drops other-project
/// entries without answering them, and answers Busy over the cap.
#[test]
fn preview_queue_deduplicates_drops_other_projects_and_answers_busy_over_cap() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    let first = json!({"project_id":"p","revision":"1","activation":"a"});
    display.handle_commands([
        Command::Preview {
            project: first.clone(),
            id: "a".into(),
            kind: PreviewKind::Texture,
        },
        Command::Preview {
            project: first.clone(),
            id: "a".into(),
            kind: PreviewKind::Texture,
        },
        Command::Preview {
            project: first.clone(),
            id: "a".into(),
            kind: PreviewKind::Material,
        },
        Command::Preview {
            project: first.clone(),
            id: "b".into(),
            kind: PreviewKind::Material,
        },
    ]);
    assert_eq!(
        display.preview_queue,
        VecDeque::from([
            (first.clone(), "a".into(), PreviewKind::Texture),
            (first.clone(), "a".into(), PreviewKind::Material),
            (first.clone(), "b".into(), PreviewKind::Material),
        ]),
        "duplicates of the same id and kind are skipped; kind is part of the identity"
    );

    // A different project's request empties the old revision's queue
    // without answering: the window asks again at its own revision.
    let second = json!({"project_id":"p","revision":"2","activation":"a"});
    display.handle_commands([Command::Preview {
        project: second.clone(),
        id: "z".into(),
        kind: PreviewKind::Texture,
    }]);
    assert_eq!(
        display.preview_queue,
        VecDeque::from([(second.clone(), "z".into(), PreviewKind::Texture)])
    );
    assert!(rx.try_recv().is_err(), "dropped requests owe no update");

    // Filling to the cap keeps 64; one more answers the oldest with Busy.
    for index in 0..63 {
        display.handle_commands([Command::Preview {
            project: second.clone(),
            id: format!("c{index}"),
            kind: PreviewKind::Texture,
        }]);
    }
    assert_eq!(display.preview_queue.len(), 64);
    display.handle_commands([Command::Preview {
        project: second.clone(),
        id: "overflow".into(),
        kind: PreviewKind::Texture,
    }]);
    assert_eq!(display.preview_queue.len(), 64);
    let (project, id, result) = take_preview_update(&rx);
    assert_eq!(project, second);
    assert_eq!(id, "z", "overflow answers the oldest queued request");
    assert_eq!(result.unwrap_err().code, protocol::ErrorCode::Busy);

    // A duplicate of an already-queued id is a no-op even at the cap.
    display.handle_commands([Command::Preview {
        project: second,
        id: "overflow".into(),
        kind: PreviewKind::Texture,
    }]);
    assert_eq!(display.preview_queue.len(), 64);
    assert!(rx.try_recv().is_err());
}

/// Queued requests whose project the display thread has moved
/// past answer StaleProject instead of being submitted.
#[test]
fn stale_preview_candidates_answer_themselves_and_free_the_queue() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    display.sync_state();
    assert_eq!(display.last_project["context"]["project"], reference);
    let mut stale = reference.clone();
    stale["revision"] = json!("stale");
    display.handle_commands([
        Command::Preview {
            project: stale.clone(),
            id: "s1".into(),
            kind: PreviewKind::Texture,
        },
        Command::Preview {
            project: stale.clone(),
            id: "s2".into(),
            kind: PreviewKind::Material,
        },
    ]);
    display.start_preview();
    let mut replies = Vec::new();
    for update in rx.try_iter() {
        if let Update::Preview {
            project,
            id,
            result,
        } = update
        {
            replies.push((project, id, result));
        }
    }
    assert_eq!(replies.len(), 2, "every stale request answers");
    for (project, _, result) in &replies {
        assert_eq!(*project, stale);
        assert_eq!(
            result.as_ref().unwrap_err().code,
            protocol::ErrorCode::StaleProject
        );
    }
    let ids: Vec<_> = replies.iter().map(|(_, id, _)| id.clone()).collect();
    assert_eq!(ids, ["s1", "s2"]);
    assert!(display.preview_queue.is_empty(), "the queue drains itself");
    assert!(display.preview_flight.is_none());
}

/// `poll_preview` only lands a reply that answers its own
/// request, passes Busy through, and turns a dead channel into IoError —
/// each while clearing the flight, since no ack is owed.
#[test]
fn preview_poll_passes_busy_through_and_rejects_mismatched_and_dead_replies() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    let project = json!({"project_id":"p","revision":"1","activation":"a"});

    // A reply answering another request is a mismatch, not a result.
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response::ok(
            "other-request",
            json!({"project":&project,"resource_id":"tex","path":"p.png","key":"k"}),
        ))
        .unwrap();
    display.poll_preview();
    let (project_1, id, result) = take_preview_update(&rx);
    assert_eq!(project_1, project);
    assert_eq!(id, "tex");
    assert_eq!(
        result.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );
    assert!(display.preview_flight.is_none());

    // A reply for another project or another id never lands either.
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response::ok(
            "req",
            json!({"project":json!({"project_id":"other"}),"resource_id":"tex","path":"p.png","key":"k"}),
        ))
        .unwrap();
    display.poll_preview();
    assert_eq!(
        take_preview_update(&rx).2.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response::ok(
            "req",
            json!({"project":&project,"resource_id":"another","path":"p.png","key":"k"}),
        ))
        .unwrap();
    display.poll_preview();
    assert_eq!(
        take_preview_update(&rx).2.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );
    // A reply with an empty cache path or key is not a usable result.
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response::ok(
            "req",
            json!({"project":&project,"resource_id":"tex","path":"","key":"k"}),
        ))
        .unwrap();
    display.poll_preview();
    assert_eq!(
        take_preview_update(&rx).2.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );

    // A Busy reply passes through untouched.
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response::err(
            "req",
            protocol::ErrorPayload::new(protocol::ErrorCode::Busy, "both preview workers are busy"),
        ))
        .unwrap();
    display.poll_preview();
    let error = take_preview_update(&rx).2.unwrap_err();
    assert_eq!(error.code, protocol::ErrorCode::Busy);
    assert_eq!(error.message, "both preview workers are busy");
    assert!(display.preview_flight.is_none());

    // An empty reply is refused.
    let (reply, receiver) = mpsc::channel();
    install_preview_flight(&mut display, project.clone(), "req", "tex", receiver);
    reply
        .send(protocol::Response {
            protocol: protocol::PROTOCOL_VERSION.into(),
            request_id: "req".into(),
            result: None,
            error: None,
        })
        .unwrap();
    display.poll_preview();
    assert_eq!(
        take_preview_update(&rx).2.unwrap_err().code,
        protocol::ErrorCode::InvalidRequest
    );

    // A core that stopped answering turns into IoError for its request.
    let (reply, receiver) = mpsc::channel::<protocol::Response>();
    drop(reply);
    install_preview_flight(&mut display, project, "req", "tex", receiver);
    display.poll_preview();
    assert_eq!(
        take_preview_update(&rx).2.unwrap_err().code,
        protocol::ErrorCode::IoError
    );
    assert!(display.preview_flight.is_none());
}

#[test]
fn scene_graph_roots_start_at_the_scene_with_camera_and_world_contents() {
    let (_temp, host, reference) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nAttributeBegin\nTranslate 1 0 0\nShape \"sphere\"\nAttributeEnd\n",
    );
    let core = host.handle();
    let roots = expanded_children(&core, &reference, None);
    let roots = roots.as_array().unwrap();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0]["object"], "scene");
    let children = expanded_children(&core, &reference, roots[0]["node_id"].as_str());
    let objects = children
        .as_array()
        .unwrap()
        .iter()
        .map(|node| node["object"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(objects, ["camera", "group"]);
    // World is hidden as a row. State operations are not rows; the group
    // contains just the shape.
    let group = expanded_children(&core, &reference, children[1]["node_id"].as_str());
    assert_eq!(group[0]["object"], "shape");
    let tree = crate::project_preview::tree_node(&group[0]);
    assert_eq!(tree["components"][0], "transform");
}

#[test]
fn camera_rows_list_and_expand_their_attached_groups() {
    // The light group whose transform enters at the camera is
    // attached to the Camera row, and expanding the camera pages it in like
    // any branch — while the Scene root lists only the camera and free
    // content. Dropping onto the row is gated by node_edits, unit-tested
    // next to the move rules.
    let (_temp, host, reference) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\n\
         AttributeBegin\nCoordSysTransform \"camera\"\nLightSource \"point\" \"rgb I\" [10 10 10]\n\
         AttributeEnd\nShape \"sphere\"\n",
    );
    let core = host.handle();
    let roots = expanded_children(&core, &reference, None);
    let scene = roots.as_array().unwrap()[0]["node_id"]
        .as_str()
        .unwrap()
        .to_owned();
    let children = expanded_children(&core, &reference, Some(&scene));
    let rows = children.as_array().unwrap();
    let objects = rows
        .iter()
        .map(|node| node["object"].as_str().unwrap().to_owned())
        .collect::<Vec<_>>();
    assert_eq!(objects, ["camera", "shape"], "{children}");
    let camera = &rows[0];
    assert_eq!(camera["child_count"], 1, "the attached light group");

    let attached = expanded_children(&core, &reference, camera["node_id"].as_str());
    let groups = attached.as_array().unwrap();
    assert_eq!(groups.len(), 1, "{attached}");
    assert_eq!(groups[0]["object"], "group");
    let lights = expanded_children(&core, &reference, groups[0]["node_id"].as_str());
    assert_eq!(lights.as_array().unwrap()[0]["object"], "light");
}

#[test]
fn project_search_returns_hits_with_the_lazy_reveal_path() {
    let (_temp, host, reference) =
        native_project("WorldBegin\nTranslate 1 0 0\nShape \"sphere\"\nShape \"disk\"\n");
    let core = host.handle();
    let page = project_search(&core, &reference, "sHaPe", 0, 64).unwrap();
    assert_eq!(page["total"], 2, "{page}");
    let hits = page["nodes"].as_array().unwrap();
    assert_eq!(hits.len(), 2);
    for hit in hits {
        assert_eq!(hit["kind"], "entity", "{hit}");
        assert_eq!(
            hit["path"].as_array().unwrap().len(),
            2,
            "the import wraps world-direct transforms, so the wrapper and the Scene root are the visible ancestors: {hit}"
        );
    }
    // The same helper answers empty searches with no next page.
    assert!(project_search(&core, &reference, "brick", 0, 64).unwrap()["next_offset"].is_null());

    // Offset paging reaches the remaining hit without repeating the first.
    let first = project_search(&core, &reference, "shape", 0, 1).unwrap();
    assert_eq!(first["nodes"].as_array().unwrap().len(), 1);
    assert_eq!(first["next_offset"], 1);
    let second = project_search(&core, &reference, "shape", 1, 1).unwrap();
    assert_eq!(second["nodes"].as_array().unwrap().len(), 1);
    assert!(second["next_offset"].is_null());
    assert_ne!(
        first["nodes"][0]["node_id"], second["nodes"][0]["node_id"],
        "pages must not repeat a hit"
    );
}

#[test]
#[ignore = "requires a GPU adapter"]
fn native_viewport_picks_nodes_rejects_old_frames_and_preserves_selection_after_edit() {
    fn frame(backend: &Backend, key: &str) -> FrameUpdate {
        let deadline = std::time::Instant::now() + Duration::from_secs(15);
        loop {
            if let Some(frame) = backend.take_frame(key) {
                return frame;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "viewport did not publish {key}"
            );
            thread::sleep(Duration::from_millis(10));
        }
    }
    let (_temp, host, reference) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nAttributeBegin\nMaterial \"diffuse\"\nTranslate 0 0 0\nShape \"sphere\"\nAttributeEnd\n",
    );
    let core = host.handle();
    // The scene's frames are keyed by the serialized [project, active_scene]
    // pair; take_frame matches it exactly.
    let active_scene =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["active_scene"].clone();
    let frame_key = |project: &Value| project_preview_key(project, &active_scene);
    let roots = expanded_children(&core, &reference, None);
    let world = roots.as_array().unwrap().last().unwrap()["node_id"].clone();
    let nodes = expanded_children(&core, &reference, world.as_str());
    // The camera entity lists under the scene ahead of the world's group
    // wrapper, so the holder of the shape and its states is the scope.
    let attribute = nodes
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "scope")
        .unwrap()["node_id"]
        .clone();
    // The flattened scene-graph view hides the wrapper's state directives,
    // so ask the directives listing directly for the Translate state.
    let directives = request(
        &core,
        Method::ProjectListNodes,
        json!({"project":reference,"parent":attribute,"offset":0,"limit":1000}),
    )
    .unwrap()["nodes"]
        .clone();
    let transform = directives
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["label"] == "Translate")
        .unwrap()["node_id"]
        .clone();
    let shape = directives
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["kind"] == "entity")
        .unwrap()["node_id"]
        .clone();
    let backend = Backend::start(core.clone(), None);
    let first = frame(&backend, &frame_key(&reference));
    while backend.updates.try_recv().is_ok() {}
    let pixel = first.frame.pick_ids.iter().position(|id| *id != 0).unwrap();
    let xy = [
        (pixel as u32 % first.frame.width) as f64 / first.frame.width as f64
            + 0.5 / first.frame.width as f64,
        (pixel as u32 / first.frame.width) as f64 / first.frame.height as f64
            + 0.5 / first.frame.height as f64,
    ];
    // The frame arrives over the GPU worker's round trip, so the pick is
    // re-sent until the coordinator's inspector confirms the selection.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        backend
            .commands
            .send(Command::Pick {
                frame_id: first.id,
                xy,
            })
            .unwrap();
        if let Ok(Update::ProjectInspector(details)) =
            backend.updates.recv_timeout(Duration::from_millis(100))
        {
            assert_eq!(details["node"]["node_id"], shape);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "pick did not select a native shape"
        );
    }
    request(&core,Method::ProjectApply,json!({"project":reference,"edits":[{"kind":"set_transform","node_id":transform,"value":{"kind":"translate","value":[1,0,0]}}]})).unwrap();
    backend
        .commands
        .send(Command::Pick {
            frame_id: first.id,
            xy: [0., 0.],
        })
        .unwrap();
    let current =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    let second = frame(&backend, &frame_key(&current));
    assert!(second.id > first.id);
    let mut refreshed = false;
    while let Ok(update) = backend.updates.try_recv() {
        if let Update::ProjectInspector(details) = update {
            assert_eq!(details["node"]["node_id"], shape);
            refreshed = true;
        }
    }
    assert!(refreshed, "editing lost the current native selection");
    // A pick that carries the previous frame's id must be rejected
    // silently: no wrong-node selection and no inspector refresh.
    backend
        .commands
        .send(Command::Pick {
            frame_id: first.id,
            xy,
        })
        .unwrap();
    let silence = std::time::Instant::now() + Duration::from_millis(400);
    while std::time::Instant::now() < silence {
        if let Ok(Update::ProjectInspector(details)) = backend
            .updates
            .recv_timeout(silence - std::time::Instant::now())
        {
            panic!("a stale frame id produced an inspector: {details}");
        }
    }
    backend.commands.send(Command::FrameSelection).unwrap();
    assert!(frame(&backend, &frame_key(&current)).id > second.id);
    assert!(request(&core, Method::AppGetState, json!({})).unwrap()["view"].is_null());
}

#[test]
fn viewer_import_starts_read_only_and_saves_as_a_project_folder() {
    let temp = tempfile::tempdir().unwrap();
    // Parsed paths are canonical; macOS temp dirs sit behind the /var symlink.
    let root = protocol::path::canonicalize(temp.path()).unwrap();
    let source = root.join("scene.pbrt");
    std::fs::write(&source, "WorldBegin\nShape \"sphere\"\n").unwrap();
    let host = app_core::CoreHost::start(protocol::commands::Limits::default());
    let core = host.handle();
    let _ = import_viewer(&core, source.clone()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let viewer = loop {
        let state = request(&core, Method::ProjectGetState, json!({})).unwrap();
        if state["mode"] == "viewer" {
            break state;
        }
        assert!(std::time::Instant::now() < deadline, "{state}");
        thread::sleep(Duration::from_millis(5));
    };
    assert_eq!(viewer["read_only"], true);
    assert!(viewer["path"].is_null());
    assert_eq!(
        viewer["source_path"],
        protocol::path::canonicalize(source)
            .unwrap()
            .to_str()
            .unwrap()
    );
    let reference = viewer["context"]["project"].clone();
    let destination = root.join("saved");
    let job = request(
        &core,
        Method::ProjectSave,
        json!({"project":reference,"destination":destination.to_str().unwrap()}),
    )
    .unwrap();
    let done = wait_job(&core, job);
    assert_eq!(done["state"], "succeeded", "{done}");
    let saved = request(&core, Method::ProjectGetState, json!({})).unwrap();
    assert_eq!(saved["mode"], "project");
    assert_eq!(saved["read_only"], false);
    assert_eq!(saved["path"], destination.to_str().unwrap());
    assert_eq!(
        saved["context"]["project"]["project_id"],
        reference["project_id"]
    );
    assert_eq!(
        saved["context"]["project"]["activation"],
        reference["activation"]
    );
    assert!(destination.join("project.pbrtui.json").is_file());
}

#[test]
fn completed_frames_keep_only_the_latest_and_wait_for_the_matching_scene() {
    let (commands, _receiver) = mpsc::sync_channel(1);
    let (_sender, updates) = mpsc::channel();
    let backend = Backend {
        commands,
        updates,
        latest_frame: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        thread: None,
    };
    for id in 1..=3 {
        *backend.latest_frame.lock().unwrap() = Some(FrameUpdate {
            scene_key: "new-scene".into(),
            id,
            frame: PreviewFrame {
                width: 1,
                height: 1,
                rgba: vec![id as u8; 4],
                pick_ids: vec![id as u32],
                backend: "test".into(),
            },
            camera: ViewerCamera::frame(None, 1.),
        });
    }
    assert!(backend.take_frame("old-scene").is_none());
    let latest = backend.take_frame("new-scene").unwrap();
    assert_eq!(latest.id, 3);
    assert_eq!(latest.frame.pick_ids, vec![3]);
    assert_eq!(latest.frame.rgba, vec![3; 4]);
    assert!(backend.take_frame("new-scene").is_none());
}

#[test]
fn the_new_resource_menu_summary_lists_creatable_implementations() {
    // The summary is static and drives the Contents ▸ New
    // rows; marble only exists on the spectrum side.
    let host = app_core::CoreHost::start(protocol::commands::Limits::default());
    let core = host.handle();
    let summary = resource_schema_summary(&core).unwrap();
    let material = summary["material"]["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect::<Vec<_>>();
    assert!(material.contains(&"diffuse"), "{material:?}");
    assert!(material.contains(&"mix"), "{material:?}");
    let medium: Vec<&str> = summary["medium"]["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert_eq!(medium, vec!["homogeneous"]);
    let texture_float: Vec<&str> = summary["texture_float"]["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    let texture_spectrum: Vec<&str> = summary["texture_spectrum"]["implementations"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|value| value.as_str())
        .collect();
    assert!(texture_spectrum.contains(&"marble"), "{texture_spectrum:?}");
    assert!(
        !texture_float.contains(&"marble"),
        "marble has no float table: {texture_float:?}"
    );
    // The Inspector's Implementation rows: table rows where a
    // table exists, the export schema's list otherwise.
    let names = |key: &str| -> Vec<String> {
        summary[key]["implementations"]
            .as_array()
            .unwrap_or_else(|| panic!("no {key} in {summary}"))
            .iter()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    };
    assert!(names("shape").contains(&"disk".to_owned()));
    assert!(names("light").contains(&"spot".to_owned()));
    assert_eq!(names("area_light"), ["diffuse"]);
    assert!(names("camera").contains(&"orthographic".to_owned()));
    assert!(names("integrator").contains(&"volpath".to_owned()));
    for key in ["film", "sampler", "pixel_filter", "accelerator"] {
        assert!(!names(key).is_empty(), "{key}");
    }
    assert!(
        summary["spectra"]
            .as_array()
            .unwrap()
            .contains(&json!("metal-Cu-eta"))
    );
    // The kind folders the menu aims creations at exist in every project.
    let (_project_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    for folder in ["Materials", "Textures", "Media", "Objects"] {
        let page = project_contents_page(&core, &reference, folder, 0, 256).unwrap();
        assert_eq!(page["folder"], folder, "{folder} must exist");
    }
}

#[test]
fn a_created_resource_arrives_with_the_revision_that_holds_it() {
    // Contents ▸ New: the created entry reaches the window
    // before the state sync's new revision does, so it carries that
    // revision — inspecting it with the window's older one is refused.
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    let backend = Backend::start(core.clone(), None);
    backend
        .commands
        .send(Command::Apply {
            project: reference.clone(),
            edits: vec![json!({"kind":"create_resource","resource_kind":"material",
                               "implementation":"diffuse","label":"Fresh"})],
            follow: Follow::SelectResource {
                folder: "Materials".into(),
            },
            form: None,
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let (project, entry, editor_ask) = loop {
        if let Ok(Update::CreatedResource {
            project,
            entry,
            editor_ask,
        }) = backend.updates.recv_timeout(Duration::from_millis(100))
        {
            break (project, entry, editor_ask);
        }
        assert!(std::time::Instant::now() < deadline, "no created resource");
    };
    let current =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    assert_eq!(project, current);
    assert_ne!(project, reference);
    let details = project_item_details(&core, &project, &entry).unwrap();
    assert_eq!(details["resource"]["label"], "Fresh");
    assert_eq!(editor_ask, None, "the Contents ▸ New flow enters rename");
}

/// The node editor's background create asks for the same
/// selection but no rename — the response identifies the selection, and the update the
/// session hands to the editor carries it.
#[test]
fn a_created_resource_from_the_node_editor_skips_the_rename() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    let backend = Backend::start(core.clone(), None);
    backend
        .commands
        .send(Command::Apply {
            project: reference.clone(),
            edits: vec![json!({"kind":"create_resource","resource_kind":"texture",
                               "implementation":"checkerboard","label":"checkerboard",
                               "folder":"Textures","texture_value_type":"float"})],
            follow: Follow::SelectResourceInEditor {
                folder: "Textures".into(),
                ask: 7,
            },
            form: None,
        })
        .unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let (project, entry, editor_ask) = loop {
        if let Ok(Update::CreatedResource {
            project,
            entry,
            editor_ask,
        }) = backend.updates.recv_timeout(Duration::from_millis(100))
        {
            break (project, entry, editor_ask);
        }
        assert!(std::time::Instant::now() < deadline, "no created resource");
    };
    let current =
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone();
    assert_eq!(project, current);
    let details = project_item_details(&core, &project, &entry).unwrap();
    assert_eq!(details["resource"]["label"], "checkerboard");
    assert_eq!(editor_ask, Some(7), "the answer names the ask it settles");
    // A creation refused at a superseded reference settles its ask too.
    backend
        .commands
        .send(Command::Apply {
            project: reference.clone(),
            edits: vec![json!({"kind":"create_resource","resource_kind":"texture",
                               "implementation":"checkerboard","label":"late",
                               "folder":"Textures","texture_value_type":"float"})],
            follow: Follow::SelectResourceInEditor {
                folder: "Textures".into(),
                ask: 8,
            },
            form: None,
        })
        .unwrap();
    // A graph ask for a resource that does not exist names not_found.
    backend
        .commands
        .send(Command::ResourceGraph {
            project: current.clone(),
            resource_id: "00000000-0000-0000-0000-000000000000".into(),
        })
        .unwrap();
    let (mut failed, mut gone) = (None, None);
    while failed.is_none() || gone.is_none() {
        match backend.updates.recv_timeout(Duration::from_millis(100)) {
            Ok(Update::EditorCreateFailed(ask)) => failed = Some(ask),
            Ok(Update::ResourceGraph { graph, .. }) => gone = Some(graph),
            _ => {}
        }
        assert!(
            std::time::Instant::now() < deadline + Duration::from_secs(10),
            "no settle"
        );
    }
    assert_eq!(failed, Some(8));
    let gone = gone.unwrap();
    assert!(gone["error"].is_string(), "{gone}");
    assert_eq!(gone["error_code"], "not_found", "{gone}");
}

/// Two add-imports started together each deliver their
/// own report. The poll used one job slot, so the second import displaced
/// the first and its diagnostics never reached the Console.
/// An export the window sent puts what it left out in the Console once it
/// settles: here, an imagemap without an image nothing uses.
#[test]
fn an_export_puts_its_warnings_in_the_console() {
    let (temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    // The apply's result names the revision the export must pin.
    let current = request(
        &host.handle(),
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"create_resource","resource_kind":"texture",
            "implementation":"imagemap","label":"Pending","texture_value_type":"spectrum",
            "parameters":[]}]}),
    )
    .unwrap()["project"]
        .clone();
    let (tx, rx) = mpsc::channel();
    let mut display = Display {
        core: host.handle(),
        tx,
        frames: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        stopping: false,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        gpu: gpu::GpuHandle::orphan(),
        gpu_rx: gpu::orphan_rx(),
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
        preview_slot_owner: None,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::new(),
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
    display.handle_commands([Command::Project(
        Method::ProjectExport,
        json!({"project":current,"destination":temp.path().join("bundle").display().to_string()}),
    )]);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lines = Vec::new();
    while lines.is_empty() {
        assert!(
            Instant::now() < deadline,
            "no export warning reached the Console"
        );
        display.poll_export_warnings();
        lines.extend(rx.try_iter().filter_map(|update| match update {
            Update::Log(line) => Some(line),
            _ => None,
        }));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("imagemap_without_image_skipped: ") && lines[0].contains("Pending"),
        "{lines:?}"
    );
    assert!(
        display.export_jobs.is_empty(),
        "a settled export is polled once"
    );
}

/// A save the window started logs its end in the Console exactly once: the
/// line names the operation, its outcome and its measured duration, and
/// later polls never repeat it. The settled import job that built the
/// fixture stays silent — it was never observed active (first baseline).
#[test]
fn a_settled_save_logs_one_console_line() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let (tx, rx) = mpsc::channel();
    let mut display = Display {
        core: host.handle(),
        tx,
        frames: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        stopping: false,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        gpu: gpu::GpuHandle::orphan(),
        gpu_rx: gpu::orphan_rx(),
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
        preview_slot_owner: None,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::new(),
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
    display.handle_commands([Command::Project(
        Method::ProjectSave,
        json!({"project": reference}),
    )]);

    fn save_end_lines(rx: &mpsc::Receiver<Update>) -> Vec<String> {
        rx.try_iter()
            .filter_map(|update| match update {
                Update::Log(line) if line.starts_with("project.save · ") => Some(line),
                _ => None,
            })
            .collect()
    }

    let deadline = Instant::now() + Duration::from_secs(10);
    let mut lines = Vec::new();
    while lines.is_empty() {
        assert!(
            Instant::now() < deadline,
            "the save's end never reached the Console"
        );
        display.poll_results();
        lines.extend(save_end_lines(&rx));
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert!(
        lines[0].starts_with("project.save · succeeded (") && lines[0].ends_with(")"),
        "the line carries the outcome and the measured duration: {lines:?}"
    );
    // Settled once is settled: further polls stay quiet about it.
    for _ in 0..3 {
        thread::sleep(Duration::from_millis(5));
        display.poll_results();
    }
    assert!(save_end_lines(&rx).is_empty(), "a job end logs once");
}

/// A job sent from a form (the array editor) reports a refusal at
/// submission back to that form, which otherwise would wait for a job that
/// never comes.
#[test]
fn a_refused_job_submission_settles_its_form() {
    let (_temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    // Another edit moves the revision: the old reference is now stale.
    request(
        &host.handle(),
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"create_resource","resource_kind":"texture",
            "implementation":"constant","label":"Moved","texture_value_type":"float"}]}),
    )
    .unwrap();
    let (tx, rx) = mpsc::channel();
    let mut display = Display {
        core: host.handle(),
        tx,
        frames: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        stopping: false,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        gpu: gpu::GpuHandle::orphan(),
        gpu_rx: gpu::orphan_rx(),
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
        preview_slot_owner: None,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::new(),
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
    display.handle_commands([Command::Submit {
        method: Method::ProjectSetParameterValues,
        params: json!({"project":reference,"target":{"kind":"node","id":uuid::Uuid::new_v4()},
            "name":"P","windows":[{"offset":0,"values":{"kind":"floats","value":[1.0]}}]}),
        form: "parameter:n:P".into(),
    }]);
    let settled = rx
        .try_iter()
        .find_map(|update| match update {
            Update::EditSettled { form, error } => Some((form, error)),
            _ => None,
        })
        .expect("the form hears back");
    assert_eq!(settled.0, "parameter:n:P");
    assert!(
        settled
            .1
            .as_deref()
            .is_some_and(|error| error.contains("revision")),
        "{settled:?}"
    );
}

#[test]
fn parallel_imports_each_deliver_their_reports() {
    let (temp, host, reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let source_a = temp.path().join("a.pbrt");
    std::fs::write(&source_a, "WorldBegin\nShape \"sphere\"\nWorldEnd\n").unwrap();
    let source_b = temp.path().join("b.pbrt");
    std::fs::write(&source_b, "WorldBegin\nShape \"disk\"\nWorldEnd\n").unwrap();
    let (tx, rx) = mpsc::channel();
    let mut display = Display {
        core: host.handle(),
        tx,
        frames: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        stopping: false,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        gpu: gpu::GpuHandle::orphan(),
        gpu_rx: gpu::orphan_rx(),
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
        preview_slot_owner: None,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::new(),
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
    display.handle_commands([
        Command::Project(
            Method::ProjectImportScene,
            json!({"project":reference,"source_path":source_a.display().to_string()}),
        ),
        Command::Project(
            Method::ProjectImportScene,
            json!({"project":reference,"source_path":source_b.display().to_string()}),
        ),
    ]);
    // Let both jobs settle on the worker, then read their outcomes from the
    // core: the first publishes, the second is stale on the first's revision.
    let jobs = request(&display.core, Method::JobList, json!({})).unwrap();
    let imports: Vec<String> = jobs["jobs"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|job| job["kind"] == "project.import_scene")
        .map(|job| job["job_id"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(imports.len(), 2, "{jobs}");
    let mut settled = Vec::new();
    for job_id in &imports {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let job = request(&display.core, Method::JobGet, json!({"job_id": job_id})).unwrap();
            if ["succeeded", "failed", "cancelled"]
                .contains(&job["state"].as_str().unwrap_or_default())
            {
                settled.push(job);
                break;
            }
            assert!(Instant::now() < deadline, "import did not settle: {job}");
            thread::sleep(Duration::from_millis(5));
        }
    }
    assert_eq!(settled[0]["state"], "succeeded", "{:?}", settled);
    assert_eq!(
        settled[1]["state"], "failed",
        "the second import must land stale on the first's revision: {settled:?}"
    );
    let stale_reason = settled[1]["error"]["message"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_default();

    // A Viewer reload still on its way keeps its camera intent: the failed
    // import here is not that reload.
    let pending_owner = camera_preservation::ProjectOwner::from_ref(&reference).unwrap();
    display.preserve_camera = Some(camera_preservation::CameraPreservation::new(
        "reload-job".into(),
        pending_owner,
    ));
    display.poll_import_report();
    assert!(
        display
            .preserve_camera
            .as_ref()
            .is_some_and(|intent| intent.job_id == "reload-job"),
        "an unrelated import's failure must not clear a pending reload's camera intent"
    );
    let mut statuses = Vec::new();
    let mut logs = Vec::new();
    let mut warnings = 0usize;
    for update in rx.try_iter() {
        match update {
            Update::Status(status) => statuses.push(status),
            Update::Log(line) => logs.push(line),
            Update::ImportWarnings(count) => warnings += count,
            _ => {}
        }
    }
    assert!(
        statuses.iter().any(|status| status == "Imported 1 item"),
        "the first import's summary must reach the status line: {statuses:?}"
    );
    // A failed import reports the cause and names the file.
    // says the open project is unchanged - in the status line AND the
    // Console, because a later poll overwrites the status quickly.
    let failed = format!("Import failed: b.pbrt: {stale_reason}. The open project is unchanged.");
    assert!(
        statuses.contains(&failed),
        "the failure text must reach the status line: {statuses:?}"
    );
    assert!(
        logs.contains(&failed),
        "the failure text must reach the Console: {logs:?}"
    );
    assert_eq!(warnings, 0);
}

#[test]
fn import_failure_messages_name_the_file_and_keep_the_project() {
    let failed = json!({
        "state": "failed",
        "root_path": "/scenes/bathroom.scene.pbrt",
        "error": {"code": "parse_failed",
                  "message": "more than 250000 directives (import limit)"},
    });
    assert_eq!(
        commands::import_failure_message(&failed),
        "Import failed: bathroom.scene.pbrt: more than 250000 directives \
         (import limit). The open project is unchanged."
    );
    let cancelled = json!({"state": "cancelled", "root_path": "/x.pbrt", "error": null});
    assert_eq!(
        commands::import_failure_message(&cancelled),
        "Import cancelled"
    );
    // No readable file name: keep the rest of
    // the sentence rather than printing an empty segment.
    let anonymous = json!({
        "state": "failed",
        "root_path": "",
        "error": {"code": "io_error", "message": "boom"},
    });
    assert_eq!(
        commands::import_failure_message(&anonymous),
        "Import failed: boom. The open project is unchanged."
    );
    let cancelled_code = json!({
        "state": "running",
        "root_path": "/x.pbrt",
        "error": {"code": "cancelled", "message": "x"},
    });
    assert_eq!(
        commands::import_failure_message(&cancelled_code),
        "Import cancelled"
    );
}

/// A Display with no renderer or jobs, for driving its state by hand.
fn bare_display(core: app_core::CoreHandle) -> (Display, mpsc::Receiver<Update>) {
    let (tx, rx) = mpsc::channel();
    let display = Display {
        core,
        tx,
        frames: Arc::new(Mutex::new(None)),
        thumbnail_mailbox: Arc::new(Mutex::new(ThumbnailMailbox::default())),
        stopping: false,
        render_in_flight: false,
        install_in_flight: false,
        queued_install: None,
        gpu: gpu::GpuHandle::orphan(),
        gpu_rx: gpu::orphan_rx(),
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
        preview_slot_owner: None,
        project_geometry: None,
        project_preview_base: None,
        gizmo_delta: None,
        project_pick_nodes: BTreeMap::new(),
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
    (display, rx)
}

/// F (Focus) with nothing selected frames the whole scene, like
/// Shift+F, instead of failing with "Select a project node first".
#[test]
fn frame_selection_without_a_selection_frames_everything() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let (mut display, updates) = bare_display(host.handle());
    let bounds = protocol::Bounds {
        min: [-1., -1., -1.],
        max: [1., 1., 1.],
    };
    display.project_geometry = Some(protocol::geometry::GeometryManifest {
        bounds: Some(bounds.clone()),
        supported_times: vec![0.],
        ..Default::default()
    });
    display.handle_commands([Command::FrameSelection]);
    assert!(display.dirty, "framing should mark the next view dirty");
    assert_eq!(
        display.camera,
        ViewerCamera::frame(Some(&bounds), 1280. / 720.)
    );
    assert!(
        updates.try_recv().is_err(),
        "framing without a selection is not an error"
    );
}

/// The preview draws at the Scene viewport's pixels; renders, IPR and the
/// shared view keep its aspect at a 1280-pixel long side.
#[test]
fn the_preview_follows_the_viewport_and_renders_keep_its_aspect() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let (mut display, _rx) = bare_display(host.handle());
    assert_eq!(display.render_size(), [1280, 720]);
    display.handle_commands([Command::Resize([900, 500])]);
    assert_eq!(display.size, [900, 500]);
    assert!(display.dirty);
    assert_eq!(display.render_size(), [1280, 711]);
    display.handle_commands([Command::Resize([300, 600])]);
    assert_eq!(display.render_size(), [640, 1280]);
    // Degenerate and huge panels stay within the renderer's limits.
    display.handle_commands([Command::Resize([1, 99_999])]);
    assert_eq!(display.size, [16, 4096]);
    // A big panel also fits the renderer's pixel budget, keeping its aspect.
    display.handle_commands([Command::Resize([4000, 3200])]);
    let [width, height] = display.size;
    assert!(width as u64 * height as u64 <= scene_view::preview::MAX_PREVIEW_PIXELS);
    assert!((width as f64 / height as f64 - 1.25).abs() < 1e-2);
}

/// New preview bindings recompute the outline even when the selection and
/// the preview key are unchanged: pick ids follow draw order, so an edit can
/// give the selected node a different id.
#[test]
fn new_preview_bindings_move_the_selection_outline() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let (mut display, _rx) = bare_display(host.handle());
    display.selected_project_node = Some("b".into());
    display.install_pick_nodes(BTreeMap::from([(1, "b".to_owned())]));
    display.issue_gpu();
    assert_eq!(display.highlight, BTreeSet::from([1]));
    display.install_pick_nodes(BTreeMap::from([(1, "a".to_owned()), (2, "b".to_owned())]));
    display.issue_gpu();
    assert_eq!(display.highlight, BTreeSet::from([2]));
}

/// The node editor selecting one of its value nodes retires a scene
/// node selected before: the backend refreshes a selected node's
/// inspector on every revision the selection survives (state_sync's
/// per-reference pass), and the session's `ProjectInspector` handling
/// would then move the Inspector away from the value page it shows.
/// The retiral is silent — the page is showing the value, not the node.
#[test]
fn project_deselect_retires_the_scene_node_selection() {
    let (_temp, host, reference) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\nWorldEnd\n",
    );
    let (mut display, rx) = bare_display(host.handle());
    let shape = {
        let core = &display.core;
        let world = expanded_children(core, &reference, None)
            .as_array()
            .unwrap()
            .last()
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_owned();
        expanded_children(core, &reference, Some(&world))
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "entity")
            .map(|node| node["node_id"].as_str().unwrap().to_owned())
            .expect("the project holds a shape entity")
    };
    // A Hierarchy pick selects like this: the selection lands and the
    // node's inspector page is emitted.
    display.handle_commands([Command::ProjectSelect {
        project: reference,
        node_id: shape.clone(),
    }]);
    assert_eq!(
        display.selected_project_node.as_deref(),
        Some(shape.as_str())
    );
    let mut inspector = false;
    while let Ok(update) = rx.try_recv() {
        inspector |= matches!(update, Update::ProjectInspector(_));
    }
    assert!(inspector, "selecting a node emits its inspector page");
    // The value selection's retiral: it lands, emits nothing, and the
    // next revision's refresh has no selected node to re-emit.
    display.handle_commands([Command::ProjectDeselect]);
    assert_eq!(
        display.selected_project_node, None,
        "selecting a value retires the backend's scene node"
    );
    assert!(
        matches!(rx.try_recv(), Ok(Update::ProjectDeselected)),
        "the retiral emits only its acknowledgement — the value page keeps the Inspector"
    );
    assert!(rx.try_recv().is_err());
}

/// Edit ▸ Deselect and Esc clear a live node's selection. The
/// deselect first proves the selection — the node's Inspector details
/// were on the wire and held — then clears the node at the display, and
/// the retired node's details never come back: the deselect itself is
/// silent (the session writes the page a pick that hits nothing shows)
/// and every revision's refresh from here has no selected node to
/// re-emit.
#[test]
fn project_deselect_clears_the_selected_node_and_its_inspector_details() {
    let (_temp, host, reference) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\"\nWorldBegin\nShape \"sphere\"\nWorldEnd\n",
    );
    let (mut display, rx) = bare_display(host.handle());
    let shape = {
        let core = &display.core;
        let world = expanded_children(core, &reference, None)
            .as_array()
            .unwrap()
            .last()
            .unwrap()["node_id"]
            .as_str()
            .unwrap()
            .to_owned();
        expanded_children(core, &reference, Some(&world))
            .as_array()
            .unwrap()
            .iter()
            .find(|node| node["kind"] == "entity")
            .map(|node| node["node_id"].as_str().unwrap().to_owned())
            .expect("the project holds a shape entity")
    };
    // The selection the user wants to retire, as a Hierarchy pick makes
    // it: the node's inspector details are on the wire.
    display.handle_commands([Command::ProjectSelect {
        project: reference,
        node_id: shape.clone(),
    }]);
    assert_eq!(
        display.selected_project_node.as_deref(),
        Some(shape.as_str())
    );
    let details = rx.try_recv().ok().filter(|update| {
        matches!(update, Update::ProjectInspector(details) if details["node"]["node_id"] == shape)
    });
    assert!(
        details.is_some(),
        "selecting a node emits its inspector details"
    );
    // The deselect clears the node, and the details do not come back.
    display.handle_commands([Command::ProjectDeselect]);
    assert_eq!(
        display.selected_project_node, None,
        "deselect clears the selected node"
    );
    assert!(
        matches!(rx.try_recv(), Ok(Update::ProjectDeselected)),
        "the deselect is acknowledged after any queued inspector answer"
    );
    assert!(
        rx.try_recv().is_err(),
        "the retired node's inspector details never come back"
    );
}

/// A command a form sent names that form, so a send that never reaches
/// the backend (full queue) can still settle it.
#[test]
fn form_commands_name_their_form() {
    let submit = Command::Submit {
        method: Method::ProjectSetParameterValues,
        params: json!({}),
        form: "parameter:n:P".into(),
    };
    assert_eq!(submit.form(), Some("parameter:n:P"));
    let apply = Command::Apply {
        project: json!({}),
        edits: vec![],
        follow: Follow::Nothing,
        form: Some("label:x".into()),
    };
    assert_eq!(apply.form(), Some("label:x"));
    assert_eq!(Command::Stop.form(), None);
}

/// Each preview upload after an edit re-reports the same Lit notes; the
/// Console gets each note once, and again only after it went away.
#[test]
fn lit_notes_reach_the_console_once_per_change() {
    let (_temp, host, _reference) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, rx) = bare_display(host.handle());
    let note = |reason: &str| scene_view::preview::LitNote {
        reason: reason.into(),
        material: Some("m".into()),
        draws: 2,
    };
    let logs = |rx: &mpsc::Receiver<Update>| {
        rx.try_iter()
            .filter_map(|update| match update {
                Update::Log(line) => Some(line),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    display.log_lit_notes(&[note("hair: shown as gray diffuse")]);
    assert_eq!(
        logs(&rx),
        ["Lit approximation: hair: shown as gray diffuse (m) · 2 draws"]
    );
    display.log_lit_notes(&[note("hair: shown as gray diffuse")]);
    assert!(logs(&rx).is_empty(), "unchanged notes are not repeated");
    // Two shapes approximated the same way bring the note twice: the
    // Console still gets it once per package.
    display.log_lit_notes(&[
        note("metal: shown as gray diffuse"),
        note("metal: shown as gray diffuse"),
    ]);
    let duplicated = logs(&rx);
    assert_eq!(
        duplicated,
        ["Lit approximation: metal: shown as gray diffuse (m) · 2 draws"],
        "a duplicated note within one package prints once: {duplicated:?}"
    );
    display.log_lit_notes(&[]);
    display.log_lit_notes(&[note("hair: shown as gray diffuse")]);
    assert_eq!(
        logs(&rx).len(),
        1,
        "a note that came back is reported again"
    );
    display.project_tree_snapshot = json!({"nodes":[{"object":"shape","medium_bound":true}]});
    display.log_lit_notes(&[]);
    assert_eq!(
        logs(&rx),
        ["Lit approximation: participating media are not shown in Lit"]
    );
    display.log_lit_notes(&[]);
    assert!(
        logs(&rx).is_empty(),
        "the medium hint is deduplicated like other Lit notes"
    );
    display.project_tree_snapshot = json!({"nodes":[{"object":"shape","medium_bound":false}]});
    display.log_lit_notes(&[]);
    assert!(logs(&rx).is_empty());
}

/// A preview package's warnings reach the Console once per
/// change (a preview runs after every edit; the same line must not
/// repeat), and each one counts in the status line's `unsupported` —
/// no new count field.
#[test]
fn preview_warnings_reach_the_console_once_and_count_as_unsupported() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    let installed = |warnings: usize| gpu::InstalledData {
        key: "k".into(),
        bindings: BTreeMap::new(),
        bounds: BTreeMap::new(),
        lit_notes: vec![],
        manifest: protocol::geometry::GeometryManifest {
            supported_times: vec![0.],
            ..Default::default()
        },
        keeps_transform_base: false,
        slot: PathBuf::new(),
        // Two shapes referencing the same broken asset produce the same
        // warning twice in one package.
        diagnostics: vec![
            scene_parse::Diagnostic::warning(
                "geometry_failed",
                "cannot read broken.ply: no such file",
            );
            warnings
        ],
    };
    let drain = |rx: &mpsc::Receiver<Update>| {
        rx.try_iter()
            .fold((Vec::new(), Vec::new()), |mut both, update| {
                match update {
                    Update::Log(line) => both.0.push(line),
                    Update::Status(text) => both.1.push(text),
                    _ => {}
                }
                both
            })
    };
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed(2))))
        .unwrap();
    display.poll_gpu();
    let (first_logs, first_statuses) = drain(&updates);
    assert_eq!(
        first_logs,
        ["Preview: geometry_failed: cannot read broken.ply: no such file".to_owned()],
        "a duplicated warning within one package prints once: {first_logs:?}"
    );
    assert_eq!(first_statuses, ["Project preview: 0 draws · 2 unsupported"]);
    // The next edit produces the same warnings: said once, not repeated.
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed(2))))
        .unwrap();
    display.poll_gpu();
    let (repeat_logs, repeat_statuses) = drain(&updates);
    assert!(repeat_logs.is_empty(), "the same warning repeated");
    assert_eq!(
        repeat_statuses,
        ["Project preview: 0 draws · 2 unsupported"]
    );
    // A warning that goes away and comes back is reported again.
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed(0))))
        .unwrap();
    display.poll_gpu();
    let (healthy_logs, healthy_statuses) = drain(&updates);
    assert!(healthy_logs.is_empty(), "a clean preview must stay quiet");
    assert_eq!(
        healthy_statuses,
        ["Project preview: 0 draws · 0 unsupported"]
    );
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed(2))))
        .unwrap();
    display.poll_gpu();
    let (back_logs, back_statuses) = drain(&updates);
    assert_eq!(
        back_logs.len(),
        1,
        "a returning warning is news: {back_logs:?}"
    );
    assert_eq!(back_statuses, ["Project preview: 0 draws · 2 unsupported"]);
}

/// An informational `media_not_previewed` diagnostic (retained data the
/// preview cannot draw, nothing dropped) reaches the Console like a
/// warning does, but must not count in the status line's `unsupported`
/// number: the project keeps the medium.
#[test]
fn preview_info_notes_reach_the_console_without_counting_as_unsupported() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    let installed = gpu::InstalledData {
        key: "k".into(),
        bindings: BTreeMap::new(),
        bounds: BTreeMap::new(),
        lit_notes: vec![],
        manifest: protocol::geometry::GeometryManifest {
            supported_times: vec![0.],
            ..Default::default()
        },
        keeps_transform_base: false,
        slot: PathBuf::new(),
        diagnostics: vec![
            scene_parse::Diagnostic::warning(
                "geometry_failed",
                "cannot read broken.ply: no such file",
            ),
            scene_parse::Diagnostic::info(
                "media_not_previewed",
                "participating media are not drawn in Solid/Lit (MakeNamedMedium \"fog\", type cloud); render with r4 to see them",
            ),
        ],
    };
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed)))
        .unwrap();
    display.poll_gpu();
    let mut logs = Vec::new();
    let mut statuses = Vec::new();
    for update in updates.try_iter() {
        match update {
            Update::Log(line) => logs.push(line),
            Update::Status(text) => statuses.push(text),
            _ => {}
        }
    }
    assert!(logs.contains(
        &"Preview: media_not_previewed: participating media are not drawn in Solid/Lit (MakeNamedMedium \"fog\", type cloud); render with r4 to see them".to_owned()
    ), "{logs:?}");
    assert!(
        logs.contains(&"Preview: geometry_failed: cannot read broken.ply: no such file".to_owned()),
        "{logs:?}"
    );
    assert_eq!(statuses, ["Project preview: 0 draws · 1 unsupported"]);
}

// ---------------------------------------------------------------------
// GPU worker split: the coordinator applies worker results, keeps the
// pick state, and issues renders without ever owning the renderer.

fn display_with_gpu_channels(
    core: app_core::CoreHandle,
) -> (
    Display,
    mpsc::Receiver<Update>,
    mpsc::Sender<gpu::GpuResult>,
    mpsc::Receiver<gpu::GpuRequest>,
) {
    let (mut display, updates) = bare_display(core);
    let (requests, gpu_requests) = mpsc::channel();
    let (gpu_results, gpu_rx) = mpsc::channel();
    display.gpu = gpu::GpuHandle::for_tests(requests);
    display.gpu_rx = gpu_rx;
    (display, updates, gpu_results, gpu_requests)
}

#[test]
fn external_project_view_moves_the_desktop_camera() {
    let (_temp, host, project) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let mut display = bare_display(host.handle()).0;
    display.last_project = json!({"context":{"project":project},"active_handedness":"right"});
    display.project_geometry = Some(protocol::geometry::GeometryManifest::default());
    let mut remote_camera = ViewerCamera::frame(None, 1.).to_protocol();
    remote_camera.eye[0] += 10.;
    let initial = request(
        &display.core,
        Method::ProjectGetView,
        json!({"project":display.last_project["context"]["project"]}),
    )
    .unwrap();
    display.view_revision = Some(initial["view_revision"].clone());
    request(
        &display.core,
        Method::ProjectSetCamera,
        json!({
            "project":display.last_project["context"]["project"],
            "expected_view_revision":initial["view_revision"], "camera":remote_camera
        }),
    )
    .unwrap();
    remote_camera.mirror = true;
    display.view_due = Some(Instant::now() - Duration::from_millis(1));
    display.poll_view_sync();
    assert_eq!(display.camera.to_protocol(), remote_camera);
    assert!(display.camera.mirror);
    assert!(display.dirty);
}

#[test]
fn desktop_view_write_is_not_adopted_as_an_external_camera() {
    let (_temp, host, project) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let mut display = bare_display(host.handle()).0;
    display.last_project = json!({"context":{"project":project}});
    display.project_geometry = Some(protocol::geometry::GeometryManifest::default());
    display.view_due = Some(Instant::now() - Duration::from_millis(1));
    display.poll_view_sync();
    let written = display.camera.to_protocol();
    display.dirty = false;
    display.view_due = Some(Instant::now() - Duration::from_millis(1));
    display.poll_view_sync();
    assert_eq!(display.camera.to_protocol(), written);
    assert!(!display.dirty);
}

#[test]
fn view_scene_camera_restores_the_manifest_camera() {
    let (_temp, host, _) = native_project("WorldBegin\nShape \"sphere\"\nWorldEnd\n");
    let (mut display, _) = bare_display(host.handle());
    let manifest_camera = ViewerCamera::frame(None, 1.);
    display.scene_camera = Some(manifest_camera);
    display.handle_commands([Command::ViewSceneCamera]);
    assert_eq!(display.camera, manifest_camera);
    assert!(display.dirty);
}

fn tiny_frame() -> PreviewFrame {
    PreviewFrame {
        rgba: vec![1, 2, 3, 255],
        pick_ids: vec![0],
        width: 1,
        height: 1,
        backend: "headless".into(),
    }
}

#[test]
fn installed_applies_metadata_and_camera_owner() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    let manifest = protocol::geometry::GeometryManifest {
        supported_times: vec![0.],
        ..Default::default()
    };
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(gpu::InstalledData {
            key: "k".into(),
            bindings: BTreeMap::from([(7u32, "node-a".to_owned())]),
            bounds: BTreeMap::from([(
                1u64,
                protocol::Bounds {
                    min: [0.; 3],
                    max: [1.; 3],
                },
            )]),
            lit_notes: vec![],
            diagnostics: vec![],
            keeps_transform_base: false,
            slot: PathBuf::new(),
            manifest,
        })))
        .unwrap();
    display.poll_gpu();
    assert!(display.project_geometry.is_some());
    assert_eq!(
        display.project_pick_nodes.get(&7),
        Some(&"node-a".to_owned())
    );
    assert!(display.project_object_bounds.contains_key(&1));
    // No project reference is loaded, so the owner key is empty and the
    // manifest camera takes over.
    assert_eq!(display.project_camera_owner, Some(String::new()));
    assert!(display.preview_ready);
    assert!(display.dirty);
    let mut preview = false;
    let mut status = false;
    while let Ok(update) = updates.try_recv() {
        match update {
            Update::ProjectPreview { key, times, time } => {
                assert_eq!(key, "k");
                assert_eq!(times, vec![0.]);
                assert_eq!(time, 0.);
                preview = true;
            }
            Update::Status(text) => {
                assert!(text.starts_with("Project preview: "), "{text}");
                status = true;
            }
            _ => {}
        }
    }
    assert!(preview, "the ProjectPreview update is missing");
    assert!(status, "the status update is missing");
}

#[test]
fn stale_installed_result_is_dropped() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("tracked".into());
    let stale = gpu::InstalledData {
        key: "stale".into(),
        manifest: Default::default(),
        bindings: BTreeMap::new(),
        bounds: BTreeMap::new(),
        lit_notes: vec![],
        diagnostics: vec![],
        keeps_transform_base: false,
        slot: PathBuf::new(),
    };
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(stale)))
        .unwrap();
    display.poll_gpu();
    assert!(
        display.project_geometry.is_none(),
        "a stale install was applied"
    );
    assert!(!display.preview_ready);
    assert!(
        updates.try_recv().is_err(),
        "a stale install reached the console"
    );
}

#[test]
fn rendered_applies_pick_state_and_publishes_frame() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, _updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    display.preview_ready = true;
    display.render_in_flight = true;
    let camera = ViewerCamera::frame(None, 16. / 9.);
    gpu_results
        .send(gpu::GpuResult::RenderedOk {
            id: 4,
            key: "k".into(),
            frame: PreviewFrame {
                pick_ids: vec![7, 7],
                ..tiny_frame()
            },
            camera: camera.clone(),
        })
        .unwrap();
    display.poll_gpu();
    assert_eq!(display.frame_id, 4);
    assert!(!display.render_in_flight);
    let kept = display.last_frame.as_ref().unwrap();
    assert_eq!(kept.pick_ids, vec![7, 7]);
    assert!(
        kept.rgba.is_empty(),
        "the retained pick buffer carries no rgba"
    );
    let published = display.frames.lock().unwrap().take().unwrap();
    assert_eq!(published.scene_key, "k");
    assert_eq!(published.id, 4);
    assert_eq!(published.frame.pick_ids, vec![7, 7]);
    assert_eq!(published.camera.to_protocol(), camera.to_protocol());
}

#[test]
fn stale_rendered_result_is_dropped_and_in_flight_clears() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, _updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("tracked".into());
    display.preview_ready = true;
    display.render_in_flight = true;
    gpu_results
        .send(gpu::GpuResult::RenderedOk {
            id: 9,
            key: "other".into(),
            frame: tiny_frame(),
            camera: ViewerCamera::frame(None, 16. / 9.),
        })
        .unwrap();
    display.poll_gpu();
    assert!(
        !display.render_in_flight,
        "the in-flight flag must clear for a stale render too"
    );
    assert_eq!(display.frame_id, 0);
    assert!(display.frames.lock().unwrap().is_none());
}

#[test]
fn install_failed_maps_stage_to_message() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, _gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    gpu_results
        .send(gpu::GpuResult::InstallFailed {
            key: "k".into(),
            stage: gpu::InstallStage::Parse,
            message: "m".into(),
        })
        .unwrap();
    display.poll_gpu();
    match updates.recv_timeout(Duration::from_millis(500)).unwrap() {
        Update::PreviewUnavailable(Some(text)) => {
            assert_eq!(text, "Project preview parse: m")
        }
        _ => panic!("expected PreviewUnavailable after a parse failure"),
    }
    match updates.recv_timeout(Duration::from_millis(500)).unwrap() {
        Update::Status(text) => assert_eq!(text, "Project preview parse: m"),
        _ => panic!("expected a Status after a parse failure"),
    }
    gpu_results
        .send(gpu::GpuResult::InstallFailed {
            key: "k".into(),
            stage: gpu::InstallStage::Upload,
            message: "n".into(),
        })
        .unwrap();
    display.poll_gpu();
    match updates.recv_timeout(Duration::from_millis(500)).unwrap() {
        Update::PreviewUnavailable(Some(text)) => {
            assert_eq!(text, "Project preview upload: n")
        }
        _ => panic!("expected PreviewUnavailable after an upload failure"),
    }
    assert!(
        !display.preview_ready,
        "a failed install must not enable renders"
    );
}

#[test]
fn issue_gpu_renders_when_dirty_and_ready() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, _updates, gpu_results, gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k".into());
    display.preview_ready = true;
    display.dirty = false;
    display.issue_gpu();
    assert!(
        gpu_requests
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "a clean display must not render"
    );
    display.dirty = true;
    display.issue_gpu();
    assert!(display.render_in_flight);
    assert!(!display.dirty);
    match gpu_requests
        .recv_timeout(Duration::from_millis(500))
        .unwrap()
    {
        gpu::GpuRequest::Render { id, key, .. } => {
            assert_eq!(id, 1);
            assert_eq!(key, "k");
        }
        _ => panic!("expected a Render request"),
    }
    // While the result is in flight, no second render may be issued.
    display.dirty = true;
    display.issue_gpu();
    assert!(
        gpu_requests
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "a render was issued while one was in flight"
    );
    // After the result lands, the pending dirty flag re-issues.
    gpu_results
        .send(gpu::GpuResult::RenderedOk {
            id: 1,
            key: "k".into(),
            frame: tiny_frame(),
            camera: ViewerCamera::frame(None, 16. / 9.),
        })
        .unwrap();
    display.poll_gpu();
    display.issue_gpu();
    match gpu_requests
        .recv_timeout(Duration::from_millis(500))
        .unwrap()
    {
        gpu::GpuRequest::Render { id, .. } => assert_eq!(id, 2),
        _ => panic!("expected the second Render request"),
    }
}

#[test]
fn install_slot_keeps_newest_and_flushes_after_result() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let (mut display, updates, gpu_results, gpu_requests) =
        display_with_gpu_channels(host.handle());
    display.project_preview_key = Some("k2".into());
    display.install_in_flight = true;
    display.enqueue_install("k1".into(), "j1".into(), PathBuf::from("/a"), None);
    display.enqueue_install("k2".into(), "j2".into(), PathBuf::from("/b"), None);
    match display.queued_install.as_ref() {
        Some((key, job, slot, manifest)) => {
            assert_eq!(key, "k2");
            assert_eq!(job, "j2");
            assert_eq!(slot, &PathBuf::from("/b"));
            assert!(manifest.is_none(), "a job's install reads the slot");
        }
        None => panic!("the newest queued install was dropped"),
    }
    // A terminal result lets the queued install through; the stale k1
    // failure itself must stay silent.
    gpu_results
        .send(gpu::GpuResult::InstallFailed {
            key: "k1".into(),
            stage: gpu::InstallStage::Parse,
            message: "late".into(),
        })
        .unwrap();
    display.poll_gpu();
    assert!(display.install_in_flight);
    match gpu_requests
        .recv_timeout(Duration::from_millis(500))
        .unwrap()
    {
        gpu::GpuRequest::Install {
            key,
            job_id,
            slot,
            manifest,
        } => {
            assert_eq!(key, "k2");
            assert_eq!(job_id, "j2");
            assert_eq!(slot, PathBuf::from("/b"));
            assert!(manifest.is_none(), "a job's install reads the slot");
        }
        _ => panic!("expected the flushed Install"),
    }
    assert!(
        updates.try_recv().is_err(),
        "a stale install failure reached the console"
    );
    assert!(display.queued_install.is_none());
    // The second terminal result empties the slot: no further sends.
    gpu_results
        .send(gpu::GpuResult::InstallFailed {
            key: "k2".into(),
            stage: gpu::InstallStage::Parse,
            message: "m".into(),
        })
        .unwrap();
    display.poll_gpu();
    assert!(!display.install_in_flight);
    assert!(
        gpu_requests
            .recv_timeout(Duration::from_millis(100))
            .is_err(),
        "an install was sent with an empty queue"
    );
}

#[test]
fn poll_results_updates_cards_without_gpu_results() {
    let (_temp, host, _project) = native_project("WorldBegin\nShape \"sphere\"\n");
    let core = host.handle();
    let (mut display, updates, _gpu_results, _gpu_requests) =
        display_with_gpu_channels(core.clone());
    display.last_project =
        request(&core, Method::AppGetState, json!({})).unwrap()["project"].clone();
    display
        .foreground_jobs
        .push(("no-such-job".into(), "Op".into(), 0));
    display.poll_results();
    match updates.recv_timeout(Duration::from_secs(2)).unwrap() {
        Update::ForegroundJobs(jobs) => {
            let rows = jobs["jobs"].as_array().unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0]["job_id"], "no-such-job");
            assert_eq!(rows[0]["state"], "running");
            assert_eq!(rows[0]["phase"], "Checking status…");
        }
        _ => panic!("expected a ForegroundJobs update"),
    }
}

// ----------------------------------------------------------------------
// Transform-only changes use the fast path.

/// A transform-only edit answers `project.preview_transforms` first: a
/// fast answer retargets the installed draws through the gizmo's
/// `UpdateDraws` path and adopts the new preview key without a preview
/// job, an install, or a slot write, clearing a committed gizmo delta in
/// the same update. The retargeted draws equal a real full preview's at
/// the moved revision — and each move applies to the BASE draws, so two
/// moves in a row land exactly. A later fast:false (here: the display's
/// base revision no longer matches the core's, because the referee
/// preview advanced it) takes the unchanged full rebuild, which then
/// advances the base.
#[test]
fn a_fast_transform_answer_updates_draws_without_a_preview_job() {
    let (temp, host, project) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nTranslate 1 2 3\n\
         Shape \"sphere\" \"float radius\" [1]\nAttributeEnd\n",
    );
    let core = host.handle();
    let hits = request(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"translate"}),
    )
    .expect("the translate node is searchable");
    let translate = hits["nodes"][0]["node_id"].clone();
    let active_scene = request(&core, Method::ProjectGetState, json!({}))
        .expect("the project state answers")["active_scene"]
        .clone();
    let key_of = |reference: &Value| project_preview_key(reference, &active_scene);

    // The installed preview the fast path diff against: state_sync starts
    // the real export job, the test plays the GPU worker.
    let (mut display, updates, gpu_results, gpu_requests) = display_with_gpu_channels(core.clone());
    display.sync_state();
    let job_id = display
        .project_preview_job
        .as_ref()
        .map(|(job, _)| job.clone())
        .expect("the first sync prepares the preview");
    assert_eq!(
        wait_job(&core, json!({"job_id": job_id}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    // The very first sync also clears the activation.
    let (installed_key, slot) = loop {
        match gpu_requests.try_recv() {
            Ok(gpu::GpuRequest::Install { key, slot, .. }) => break (key, slot),
            Ok(gpu::GpuRequest::Clear) => continue,
            _ => panic!("the finished preview job installs its package"),
        }
    };
    let install_from_slot = |slot: &std::path::Path, key: String| {
        let manifest =
            scene_parse::read_preview_manifest(slot).expect("the exported package parses");
        let bindings = manifest
            .draws
            .iter()
            .filter_map(|draw| {
                draw.entity_id
                    .as_ref()
                    .map(|entity| (draw.pick_id, entity.as_str().to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        gpu::InstalledData {
            key,
            bindings,
            bounds: BTreeMap::new(),
            lit_notes: vec![],
            diagnostics: vec![],
            keeps_transform_base: false,
            slot: slot.to_owned(),
            manifest,
        }
    };
    let geometry_on_disk = std::fs::read(slot.join("geometry.json")).unwrap();
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(install_from_slot(
            &slot,
            installed_key.clone(),
        ))))
        .unwrap();
    display.poll_gpu();
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&project).as_str())
    );
    let base = display
        .project_preview_base
        .clone()
        .expect("the install records the fast path's base");
    assert_eq!(base.revision, project["revision"]);
    assert_eq!(base.draws, display.project_geometry.as_ref().unwrap().draws);

    // One move, with a gizmo delta committed just before it.
    let reference_now = || {
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone()
    };
    let apply_move = |to: [f64; 3]| {
        request(
            &core,
            Method::ProjectApply,
            json!({"project":reference_now(),"edits":[
                {"kind":"set_transform","node_id":translate,
                 "value":{"kind":"translate","value":to}}]}),
        )
        .expect("the move applies");
    };
    let drain_fast_path = |gpu_requests: &mpsc::Receiver<gpu::GpuRequest>,
                           expected_key: &str|
     -> Option<Vec<protocol::geometry::DrawRecord>> {
        let mut retargeted = None;
        let mut installs = 0;
        for request in gpu_requests.try_iter() {
            match request {
                gpu::GpuRequest::UpdateDraws { key, draws, adopt } => {
                    assert_eq!(key, expected_key, "keyed to the installed scene");
                    assert!(adopt.is_some(), "a fast answer adopts the new key");
                    retargeted = Some(draws);
                }
                gpu::GpuRequest::Install { .. } => installs += 1,
                _ => {}
            }
        }
        assert_eq!(installs, 0, "a fast answer must not rebuild");
        retargeted
    };
    // The install itself reported a status line and the base key's
    // ProjectPreview — move 1's loop sees only its own messages.
    for _ in updates.try_iter() {}
    apply_move([4., 5., 6.]);
    display.gizmo_delta = Some([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 42., 0., 0., 0., 1.,
    ]);
    display.sync_state();
    let first_draws = drain_fast_path(&gpu_requests, &key_of(&project))
        .expect("a fast answer retargets the draws");
    let moved = reference_now();
    assert_eq!(first_draws.len(), base.draws.len());
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&moved).as_str()),
        "the fast path advances the preview key without a job"
    );
    assert!(display.project_preview_job.is_none());
    assert!(display.preview_ready);
    assert!(display.project_geometry.is_some());
    assert_eq!(
        display.project_geometry.as_ref().unwrap().draws,
        first_draws,
        "the manifest the UI uses follows the draws"
    );
    assert_eq!(
        display.gizmo_delta, None,
        "a committed drag does not jump back"
    );
    assert!(display.dirty, "the retargeted scene repaints");
    let mut saw_new_key = false;
    for update in updates.try_iter() {
        match update {
            Update::ProjectPreview { key, .. } => {
                saw_new_key = saw_new_key || key == key_of(&moved);
            }
            Update::Status(text) => assert!(
                !text.starts_with("Preparing native project preview"),
                "the fast path announces no export: {text}"
            ),
            _ => {}
        }
    }
    assert!(saw_new_key, "the window learns the fast path's new key");
    assert_eq!(
        std::fs::read(slot.join("geometry.json")).unwrap(),
        geometry_on_disk,
        "the fast path never writes the slot"
    );

    // A second move before any rebuild: still applied to the BASE draws.
    apply_move([7., 8., 9.]);
    let twice_moved = reference_now();
    display.sync_state();
    let second_draws = drain_fast_path(&gpu_requests, &key_of(&moved))
        .expect("the second move is answered from the same base");
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&twice_moved).as_str())
    );
    for update in updates.try_iter() {
        match update {
            Update::ProjectPreview { key, .. } => {
                assert_eq!(key, key_of(&twice_moved));
            }
            Update::Status(text) => assert!(
                !text.starts_with("Preparing native project preview"),
                "the second fast answer announces no export: {text}"
            ),
            _ => {}
        }
    }

    // The referee: a real full preview at the twice-moved revision. Its
    // draws equal the delta-retargeted ones, matrix for matrix — which a
    // second move wrongly composed onto the first answer's draws would
    // not match.
    let referee_slot = temp.path().join("preview-referee");
    let job = request(
        &core,
        Method::ProjectPreview,
        json!({"project":twice_moved,"destination":referee_slot.display().to_string()}),
    )
    .unwrap();
    assert_eq!(wait_job(&core, job)["state"], "succeeded");
    let referee = scene_parse::read_preview_manifest(&referee_slot).unwrap();
    assert_eq!(referee.draws.len(), second_draws.len());
    for (full, retargeted) in referee.draws.iter().zip(&second_draws) {
        assert_eq!(full.entity_id, retargeted.entity_id);
        assert_eq!(
            full.world_from_object.len(),
            retargeted.world_from_object.len()
        );
        for (expected, moved) in full
            .world_from_object
            .iter()
            .zip(&retargeted.world_from_object)
        {
            for (a, b) in expected.iter().zip(moved) {
                assert!(
                    (a - b).abs() < 1e-9,
                    "delta · base draw {b:?} != the full preview's {a:?}"
                );
            }
        }
    }

    // The referee advanced the core's preview base past the display's, so
    // the next move answers fast:false and the unchanged full rebuild
    // runs: a job, an install — and the base moves with it.
    apply_move([10., 11., 12.]);
    let third = reference_now();
    display.sync_state();
    assert!(
        display.project_preview_job.is_some(),
        "a refused fast answer rebuilds the preview in full"
    );
    let job_id = display
        .project_preview_job
        .as_ref()
        .map(|(job, _)| job.clone())
        .unwrap();
    assert_eq!(
        wait_job(&core, json!({"job_id": job_id}))["state"],
        "succeeded"
    );
    // The refused answer retargeted nothing: no UpdateDraws was sent.
    let mut stray = None;
    for request in gpu_requests.try_iter() {
        if matches!(request, gpu::GpuRequest::UpdateDraws { .. }) {
            stray = Some(request);
        }
    }
    assert!(stray.is_none(), "a refused fast answer retargets nothing");
    display.poll_project_preview();
    let (rebuilt_key, rebuilt_slot) = match gpu_requests.try_recv() {
        Ok(gpu::GpuRequest::Install { key, slot, .. }) => (key, slot),
        _ => panic!("the full rebuild installs its package"),
    };
    assert_eq!(rebuilt_key, key_of(&third));
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(install_from_slot(
            &rebuilt_slot,
            rebuilt_key.clone(),
        ))))
        .unwrap();
    display.poll_gpu();
    assert!(display.project_preview_job.is_none());
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&third).as_str())
    );
    let base = display
        .project_preview_base
        .as_ref()
        .expect("the rebuilt install records a new base");
    assert_eq!(base.revision, third["revision"]);
    assert_eq!(base.draws, display.project_geometry.as_ref().unwrap().draws);
}

// ----------------------------------------------------------------------
// Light deltas use the transform fast path.

/// The project reference at the current revision.
fn current_reference(core: &app_core::CoreHandle) -> Value {
    request(core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone()
}

/// The world-space translation column of a row-major 4x4.
fn translation_column(matrix: [f64; 16]) -> [f64; 3] {
    [matrix[3], matrix[7], matrix[11]]
}

fn translations_match(matrix: [f64; 16], at: [f64; 3]) -> bool {
    translation_column(matrix)
        .iter()
        .zip(at)
        .all(|(moved, expected)| (moved - expected).abs() < 1e-9)
}

/// The first search hit for one query, with its ancestor path.
fn searched_hit(core: &app_core::CoreHandle, reference: &Value, query: &str) -> Value {
    let hits = request(
        core,
        Method::ProjectSearchNodes,
        json!({"project":reference,"query":query}),
    )
    .unwrap_or_else(|error| panic!("query {query} must answer: {error}"));
    assert_eq!(hits["total"], 1, "exactly one node matches {query}: {hits}");
    hits["nodes"][0].clone()
}

/// The Translate whose scope is the hit's (same parent), among the
/// search's translate hits.
fn translate_beside(core: &app_core::CoreHandle, reference: &Value, sibling: &Value) -> String {
    let hits = request(
        core,
        Method::ProjectSearchNodes,
        json!({"project":reference,"query":"translate"}),
    )
    .expect("the translates are searchable");
    hits["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|node| node["path"] == sibling["path"])
        .map(|node| node["node_id"].as_str().unwrap().to_owned())
        .unwrap_or_else(|| panic!("no Translate beside the hit {sibling}"))
}

/// A fast answer naming a light moves something
/// `UpdateDraws` cannot carry — Lit assembles every light from the
/// manifest — so the fast path re-installs the base package with the
/// moved lights (and draws) as a manifest override, without a preview
/// job: no badge, no slot write, the window gets the new key directly,
/// and a committed gizmo delta clears in the same update. The install
/// applies normally (geometry, bounds, bindings) but KEEPS the base —
/// the override's own manifest is the pre-move base retargeted — so a
/// second move still applies to the base lights exactly, matrix for
/// matrix like a real full preview's at the same revision. A shape-only
/// move afterwards still takes the `UpdateDraws` path.
#[test]
fn a_light_transform_answer_reinstalls_the_base_package_without_a_preview_job() {
    let (temp, host, project) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nTranslate 1 2 3\n\
         LightSource \"point\" \"rgb I\" [10 10 10]\nAttributeEnd\n\
         AttributeBegin\nTranslate -1 0 0\nShape \"sphere\" \"float radius\" [1]\nAttributeEnd\n",
    );
    let core = host.handle();
    let active_scene = request(&core, Method::ProjectGetState, json!({}))
        .expect("the project state answers")["active_scene"]
        .clone();
    let key_of = |reference: &Value| project_preview_key(reference, &active_scene);
    let light = searched_hit(&core, &project, "lightsource");
    let shape = searched_hit(&core, &project, "shape");
    let light_id = light["node_id"].as_str().unwrap().to_owned();
    let light_translate = translate_beside(&core, &project, &light);
    let shape_translate = translate_beside(&core, &project, &shape);

    // The installed preview the fast path diffs against: state_sync starts
    // the real export job, the test plays the GPU worker.
    let (mut display, updates, gpu_results, gpu_requests) = display_with_gpu_channels(core.clone());
    display.sync_state();
    let job_id = display
        .project_preview_job
        .as_ref()
        .map(|(job, _)| job.clone())
        .expect("the first sync prepares the preview");
    assert_eq!(
        wait_job(&core, json!({"job_id": job_id}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    let (installed_key, slot) = loop {
        match gpu_requests.try_recv() {
            Ok(gpu::GpuRequest::Install { key, slot, .. }) => break (key, slot),
            Ok(gpu::GpuRequest::Clear) => continue,
            _ => panic!("the finished preview job installs its package"),
        }
    };
    let installed_from_trace = |manifest: protocol::geometry::GeometryManifest,
                                key: String,
                                keeps_base: bool,
                                slot: &std::path::Path| {
        let bindings = manifest
            .draws
            .iter()
            .filter_map(|draw| {
                draw.entity_id
                    .as_ref()
                    .map(|entity| (draw.pick_id, entity.as_str().to_owned()))
            })
            .collect::<BTreeMap<_, _>>();
        gpu::InstalledData {
            key,
            bindings,
            bounds: BTreeMap::new(),
            lit_notes: vec![],
            diagnostics: vec![],
            keeps_transform_base: keeps_base,
            slot: slot.to_owned(),
            manifest,
        }
    };
    let install_from_slot = |slot: &std::path::Path, key: String| {
        let manifest =
            scene_parse::read_preview_manifest(slot).expect("the exported package parses");
        installed_from_trace(manifest, key, false, slot)
    };
    let geometry_on_disk = std::fs::read(slot.join("geometry.json")).unwrap();
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(install_from_slot(
            &slot,
            installed_key.clone(),
        ))))
        .unwrap();
    display.poll_gpu();
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&project).as_str())
    );
    let base = display
        .project_preview_base
        .clone()
        .expect("the install records the fast path's base");
    assert_eq!(base.revision, project["revision"]);
    assert_eq!(
        base.slot, slot,
        "the base remembers the package slot to re-install from"
    );
    // The project's export pins the light record to its light node —
    // the join the light delta needs.
    assert_eq!(
        base.lights
            .iter()
            .map(|light| light.entity_id.as_ref().map(|id| id.as_str().to_owned()))
            .collect::<Vec<_>>(),
        vec![Some(light_id.clone())],
        "the base carries the pinned light record: {:?}",
        base.lights
    );
    assert!(
        translations_match(base.lights[0].world_from_light, [1., 2., 3.]),
        "the base light sits at the declaration's transform: {:?}",
        base.lights[0].world_from_light
    );
    assert_eq!(base.draws.len(), 1, "the sphere draws: {:?}", base.draws);

    let reference_now = || {
        request(&core, Method::ProjectGetState, json!({})).unwrap()["context"]["project"].clone()
    };
    let apply_translate = |node: String, to: [f64; 3]| {
        request(
            &core,
            Method::ProjectApply,
            json!({"project":reference_now(),"edits":[
                {"kind":"set_transform","node_id":node,
                 "value":{"kind":"translate","value":to}}]}),
        )
        .expect("the move applies");
    };
    // The install's own status lines do not leak into the move's look.
    for _ in updates.try_iter() {}

    // A light move retargets through an override install, not UpdateDraws.
    apply_translate(light_translate.clone(), [4., 5., 6.]);
    display.gizmo_delta = Some([
        1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 42., 0., 0., 0., 1.,
    ]);
    display.sync_state();
    let moved_once = reference_now();
    let mut override_manifest = None;
    let mut update_draws = 0;
    let mut other_requests = 0;
    for request in gpu_requests.try_iter() {
        match request {
            gpu::GpuRequest::Clear => continue,
            gpu::GpuRequest::Install {
                key,
                job_id,
                slot: requested_slot,
                manifest,
            } => {
                assert_eq!(
                    key,
                    key_of(&moved_once),
                    "the override install adopts the moved revision"
                );
                assert_eq!(
                    job_id, "transform-fast-path",
                    "the fast path owns this install: {job_id}"
                );
                assert_eq!(
                    requested_slot, slot,
                    "the override installs from the base package slot"
                );
                override_manifest = manifest.map(|manifest| *manifest);
            }
            gpu::GpuRequest::UpdateDraws { .. } => update_draws += 1,
            _ => other_requests += 1,
        }
    }
    assert_eq!(
        update_draws, 0,
        "a light move must not go through UpdateDraws"
    );
    assert_eq!(
        other_requests, 0,
        "a light move reaches the GPU with exactly one request"
    );
    let moved_manifest = override_manifest.expect("a light move re-installs the base package");
    assert_eq!(moved_manifest.lights.len(), 1);
    assert_eq!(
        moved_manifest.lights[0]
            .entity_id
            .as_ref()
            .map(|id| id.as_str().to_owned()),
        Some(light_id.clone()),
        "the moved light record keeps its pin"
    );
    assert!(
        translations_match(moved_manifest.lights[0].world_from_light, [4., 5., 6.]),
        "the manifest the fast path installs carries the moved light: {:?}",
        moved_manifest.lights[0].world_from_light
    );
    assert_eq!(
        moved_manifest.draws, base.draws,
        "only the light moved — the draws stay the base's"
    );
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&moved_once).as_str()),
        "the key advances without a job"
    );
    assert!(
        display.project_preview_job.is_none(),
        "a light move starts no preview job"
    );
    assert!(display.preview_ready);
    assert_eq!(
        display.gizmo_delta, None,
        "a committed drag does not jump back"
    );
    assert!(display.dirty, "the moved scene repaints");
    // The UI manifest shows the move immediately, before the worker replies.
    assert!(translations_match(
        display.project_geometry.as_ref().unwrap().lights[0].world_from_light,
        [4., 5., 6.],
    ));
    let mut saw_new_key = false;
    for update in updates.try_iter() {
        match update {
            Update::ProjectPreview { key, .. } => {
                saw_new_key = saw_new_key || key == key_of(&moved_once);
            }
            Update::Status(text) => assert!(
                !text.starts_with("Preparing native project preview"),
                "no badge for a fast answer: {text}"
            ),
            _ => {}
        }
    }
    assert!(saw_new_key, "the window learns the fast path's new key");

    // The worker's reply applies normally but keeps the base.
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed_from_trace(
            moved_manifest.clone(),
            key_of(&moved_once),
            true,
            &slot,
        ))))
        .unwrap();
    display.poll_gpu();
    let kept = display
        .project_preview_base
        .as_ref()
        .expect("the override install keeps the base");
    assert_eq!(kept.revision, project["revision"], "the base stays R0");
    assert_eq!(kept.draws, base.draws, "the base draws stay untouched");
    assert!(
        translations_match(kept.lights[0].world_from_light, [1., 2., 3.]),
        "the base light stays unmoved: {:?}",
        kept.lights[0].world_from_light
    );
    assert_eq!(kept.slot, slot);
    assert!(translations_match(
        display.project_geometry.as_ref().unwrap().lights[0].world_from_light,
        [4., 5., 6.],
    ));

    // A second light move: applied to the SAME base lights, so the answer
    // is the base moved once more, not a compounding onto the displayed.
    apply_translate(light_translate.clone(), [7., 8., 9.]);
    display.sync_state();
    let moved_twice = reference_now();
    let mut twice_moved_manifest = None;
    let mut update_draws = 0;
    let mut other_requests = 0;
    for request in gpu_requests.try_iter() {
        match request {
            gpu::GpuRequest::Clear => continue,
            gpu::GpuRequest::Install {
                key,
                slot: requested_slot,
                manifest,
                ..
            } => {
                assert_eq!(key, key_of(&moved_twice));
                assert_eq!(requested_slot, slot);
                twice_moved_manifest = manifest.map(|manifest| *manifest);
            }
            gpu::GpuRequest::UpdateDraws { .. } => update_draws += 1,
            _ => other_requests += 1,
        }
    }
    assert_eq!(update_draws, 0, "the second light move also reinstalls");
    assert_eq!(
        other_requests, 0,
        "the second light move sends nothing else"
    );
    let twice_moved_manifest =
        twice_moved_manifest.expect("the second light move also carries its manifest");
    assert!(
        translations_match(
            twice_moved_manifest.lights[0].world_from_light,
            [7., 8., 9.],
        ),
        "the second move applies to the base, not to the first answer \
         (would land at [10, 11, 12]): {:?}",
        twice_moved_manifest.lights[0].world_from_light
    );
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed_from_trace(
            twice_moved_manifest.clone(),
            key_of(&moved_twice),
            true,
            &slot,
        ))))
        .unwrap();
    display.poll_gpu();

    // A shape edit after light moves answers the FULL diff against the
    // core's base — light and shape together — so it also re-installs,
    // and both retarget from the base: the sphere draw carries the shape
    // delta from the base, the light still carries its own (base, not
    // compounded).
    apply_translate(shape_translate.clone(), [2., 0., 0.]);
    display.sync_state();
    let moved_shape = reference_now();
    let mut update_draws = 0;
    let mut shape_manifest = None;
    for request in gpu_requests.try_iter() {
        match request {
            gpu::GpuRequest::Clear => continue,
            gpu::GpuRequest::Install {
                key,
                slot: requested_slot,
                manifest,
                ..
            } => {
                assert_eq!(key, key_of(&moved_shape));
                assert_eq!(requested_slot, slot);
                shape_manifest = manifest.map(|manifest| *manifest);
            }
            gpu::GpuRequest::UpdateDraws { .. } => update_draws += 1,
            _ => {}
        }
    }
    assert_eq!(
        update_draws, 0,
        "a shape moved beside a light moves the light too"
    );
    let shape_manifest = shape_manifest.expect("the mixed answer re-installs the base");
    // The base draw matrix carries the mesh's `origin_f64` inside it
    // `), so the sphere rests at x=-1 with the
    // sphere-local origin at z=-1 — the fast delta applies on the left of
    // the whole product, origin included.
    assert!(
        translations_match(base.draws[0].world_from_object[0], [-1., 0., -1.]),
        "the base sphere draw carries its mesh origin: {:?}",
        base.draws[0].world_from_object
    );
    assert_eq!(shape_manifest.draws.len(), 1);
    assert!(
        translations_match(shape_manifest.draws[0].world_from_object[0], [2., 0., -1.]),
        "the sphere's draw retargets from the base: {:?}",
        shape_manifest.draws[0].world_from_object
    );
    assert!(
        translations_match(shape_manifest.lights[0].world_from_light, [7., 8., 9.]),
        "the light carries its R0 delta, not a compounding: {:?}",
        shape_manifest.lights[0].world_from_light
    );
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&moved_shape).as_str()),
    );
    assert!(
        display.project_preview_job.is_none(),
        "the mixed shape and light move starts no preview job"
    );
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(installed_from_trace(
            shape_manifest.clone(),
            key_of(&moved_shape),
            true,
            &slot,
        ))))
        .unwrap();
    display.poll_gpu();

    // Lights and shape back at the base: the answer
    // names nothing, yet the displayed light was moved by an earlier
    // override, so the base lights must be re-installed, not left stale.
    apply_translate(shape_translate.clone(), [-1., 0., 0.]);
    display.sync_state();
    for request in gpu_requests.try_iter() {
        if let gpu::GpuRequest::Install {
            key,
            manifest: Some(manifest),
            ..
        } = request
        {
            gpu_results
                .send(gpu::GpuResult::Installed(Box::new(installed_from_trace(
                    *manifest, key, true, &slot,
                ))))
                .unwrap();
        }
    }
    display.poll_gpu();
    apply_translate(light_translate.clone(), [1., 2., 3.]);
    display.sync_state();
    let mut restored = None;
    for request in gpu_requests.try_iter() {
        if let gpu::GpuRequest::Install { manifest, .. } = request {
            restored = manifest.map(|manifest| *manifest);
        }
    }
    let restored = restored.expect("a light returning to its base re-installs");
    assert!(
        translations_match(restored.lights[0].world_from_light, [1., 2., 3.]),
        "the light is back at the base: {:?}",
        restored.lights[0].world_from_light
    );
    assert!(display.project_preview_job.is_none());

    // The referee: a real full preview at the same revision. Its light and
    // draw matrices equal the fast answers', and the slot never changed.
    let referee_slot = temp.path().join("preview-referee");
    let job = request(
        &core,
        Method::ProjectPreview,
        json!({"project":reference_now(),"destination":referee_slot.display().to_string()}),
    )
    .unwrap();
    assert_eq!(wait_job(&core, job)["state"], "succeeded");
    let referee = scene_parse::read_preview_manifest(&referee_slot).unwrap();
    let shown = display.project_geometry.as_ref().unwrap();
    assert_eq!(
        referee
            .lights
            .iter()
            .map(|light| (light.entity_id.clone(), light.world_from_light))
            .collect::<Vec<_>>(),
        shown
            .lights
            .iter()
            .map(|light| (light.entity_id.clone(), light.world_from_light))
            .collect::<Vec<_>>(),
        "the displayed lights equal the full preview's"
    );
    for (full, fast) in referee.draws.iter().zip(&shown.draws) {
        assert_eq!(full.entity_id, fast.entity_id);
        for (full_matrix, fast_matrix) in full.world_from_object.iter().zip(&fast.world_from_object)
        {
            for (a, b) in full_matrix.iter().zip(fast_matrix) {
                assert!((a - b).abs() < 1e-9, "full preview {a} != fast path {b}");
            }
        }
    }
    assert_eq!(
        std::fs::read(slot.join("geometry.json")).unwrap(),
        geometry_on_disk,
        "the fast path never writes the slot"
    );
}

/// A fast answer whose delta ids name nothing installed —
/// here a shape whose geometry failed to build (radius 0), so the base
/// has no draw pinned to it and no light either — declines, and the
/// unchanged full rebuild runs: the move starts a real preview job.
#[test]
fn a_fast_answer_naming_no_base_geometry_rebuilds_in_full() {
    let (_temp, host, project) = native_project(
        "LookAt 0 0 5 0 0 0 0 1 0\nCamera \"perspective\" \"float fov\" [45]\n\
         Film \"rgb\" \"integer xresolution\" [16] \"integer yresolution\" [16]\n\
         WorldBegin\nAttributeBegin\nTranslate 1 2 3\n\
         Shape \"sphere\" \"float radius\" [0]\nAttributeEnd\n\
         Shape \"sphere\"\nWorldEnd\n",
    );
    let core = host.handle();
    let active_scene = request(&core, Method::ProjectGetState, json!({}))
        .expect("the project state answers")["active_scene"]
        .clone();
    let key_of = |reference: &Value| project_preview_key(reference, &active_scene);
    let hits = request(
        &core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":"shape"}),
    )
    .expect("the shapes are searchable");
    assert_eq!(
        hits["total"], 2,
        "both shape nodes exist — one draws, one failed: {hits}"
    );
    let translate_node = searched_hit(&core, &project, "translate");
    let translate = translate_node["node_id"].as_str().unwrap().to_owned();

    let (mut display, updates, gpu_results, gpu_requests) = display_with_gpu_channels(core.clone());
    display.sync_state();
    let job_id = display
        .project_preview_job
        .as_ref()
        .map(|(job, _)| job.clone())
        .expect("the first sync prepares the preview");
    assert_eq!(
        wait_job(&core, json!({"job_id": job_id}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    let (installed_key, slot) = loop {
        match gpu_requests.try_recv() {
            Ok(gpu::GpuRequest::Install { key, slot, .. }) => break (key, slot),
            Ok(gpu::GpuRequest::Clear) => continue,
            _ => panic!("the finished preview job installs its package"),
        }
    };
    let manifest = scene_parse::read_preview_manifest(&slot).expect("the exported package parses");
    assert_eq!(
        manifest.draws.len(),
        1,
        "only the plain sphere draws — the radius-0 sphere failed: {manifest:?}"
    );
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(gpu::InstalledData {
            key: installed_key.clone(),
            bindings: BTreeMap::new(),
            bounds: BTreeMap::new(),
            lit_notes: vec![],
            diagnostics: vec![],
            keeps_transform_base: false,
            slot: slot.clone(),
            manifest,
        })))
        .unwrap();
    display.poll_gpu();
    for _ in updates.try_iter() {}

    // The move: the core answers fast (the failed shape is a transform-only
    // change naming its entity), but no base draw or light carries that
    // id — only a rebuild can produce what the edited document holds.
    request(
        &core,
        Method::ProjectApply,
        json!({"project":current_reference(&core),"edits":[
            {"kind":"set_transform","node_id":translate,
             "value":{"kind":"translate","value":[4., 5., 6.]}}]}),
    )
    .expect("the move applies");
    display.sync_state();
    assert!(
        display.project_preview_job.is_some(),
        "an answer naming no base draw or light rebuilds in full"
    );
    let mut retargeted = 0;
    for request in gpu_requests.try_iter() {
        if let gpu::GpuRequest::UpdateDraws { .. } = request {
            retargeted += 1;
        }
    }
    assert_eq!(retargeted, 0, "a declined answer retargets nothing");
    let job_id = display
        .project_preview_job
        .as_ref()
        .map(|(job, _)| job.clone())
        .unwrap();
    assert_eq!(
        wait_job(&core, json!({"job_id": job_id}))["state"],
        "succeeded"
    );
    display.poll_project_preview();
    let (rebuilt_key, rebuilt_slot) = match gpu_requests.try_recv() {
        Ok(gpu::GpuRequest::Install { key, slot, .. }) => (key, slot),
        _ => panic!("the full rebuild installs its package"),
    };
    let moved = current_reference(&core);
    assert_eq!(rebuilt_key, key_of(&moved));
    gpu_results
        .send(gpu::GpuResult::Installed(Box::new(gpu::InstalledData {
            key: rebuilt_key.clone(),
            bindings: BTreeMap::new(),
            bounds: BTreeMap::new(),
            lit_notes: vec![],
            diagnostics: vec![],
            keeps_transform_base: false,
            slot: rebuilt_slot.clone(),
            manifest: scene_parse::read_preview_manifest(&rebuilt_slot).unwrap(),
        })))
        .unwrap();
    display.poll_gpu();
    assert!(display.project_preview_job.is_none());
    assert_eq!(
        display.project_preview_key.as_deref(),
        Some(key_of(&moved).as_str())
    );
}
