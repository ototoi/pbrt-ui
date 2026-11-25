use super::*;
fn request(method: Method, params: Value) -> Request {
    Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: Uuid::new_v4().to_string(),
        method,
        params,
    }
}
fn document() -> (ProjectSnapshot, project::NodeId) {
    let mut document = project::ProjectDocument::new("Native");
    let shape = project::Node::new(
        "Sphere",
        project::NodeKind::Entity {
            entity: project::EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let id = shape.id;
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let world = scene.world;
    let project::NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    children.push(id);
    scene.nodes.insert(id, shape);
    (ProjectSession::new(document).unwrap().snapshot(), id)
}
fn document_with_camera(selected: bool) -> ProjectSnapshot {
    let (snapshot, _) = document();
    let mut document = snapshot.document().clone();
    let scene_id = document.default_scene.unwrap();
    let camera = project::Node::new(
        "Camera",
        project::NodeKind::Entity {
            entity: project::EntityKind::Camera,
            implementation: "perspective".into(),
            parameters: vec![],
            pose: None,
        },
    );
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    scene.options.push(camera.id);
    if selected {
        scene.render_camera = Some(camera.id);
    }
    scene.nodes.insert(camera.id, camera);
    ProjectSession::new(document).unwrap().snapshot()
}

#[test]
fn create_resource_with_id_and_bind_is_one_undoable_apply_and_duplicate_ids_fail() {
    let (snapshot, node) = document();
    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let id = Uuid::new_v4();
    let project = core.projects.reference().unwrap();
    let result = core.handle(request(Method::ProjectApply, json!({"project":project,"edits":[
        {"kind":"create_resource","id":id,"resource_kind":"material","implementation":"diffuse","label":"diffuse"},
        {"kind":"set_local_binding","node_id":node.0,"material":{"kind":"resource","value":id},"medium":null}
    ]})));
    assert!(result.error.is_none(), "{:?}", result.error);
    let active = core.projects.active.as_ref().unwrap();
    assert!(
        active
            .session
            .document()
            .resources
            .contains_key(&project::ResourceId(id))
    );
    assert!(active.session.can_undo());
    let reference = core.projects.reference().unwrap();
    let duplicate = core.handle(request(Method::ProjectApply, json!({"project":reference,"edits":[
        {"kind":"create_resource","id":id,"resource_kind":"material","implementation":"diffuse","label":"again"}
    ]})));
    assert_eq!(duplicate.error.unwrap().code, ErrorCode::InvalidRequest);
    let reference = core.projects.reference().unwrap();
    let undone = core.handle(request(Method::ProjectUndo, json!({"project":reference})));
    assert!(undone.error.is_none(), "{:?}", undone.error);
    let active = core.projects.active.as_ref().unwrap();
    assert!(
        !active
            .session
            .document()
            .resources
            .contains_key(&project::ResourceId(id))
    );
}
fn pending_open(core: &mut CoreState) -> JobId {
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectOpen,
        json!({"path":crate::TEST_PROJECT,"expected_current":context}),
    ));
    assert!(response.error.is_none());
    serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap()
}

#[test]
fn set_transform_refuses_identity_and_replace_operations() {
    // re-parenting resets (PBRT Identity / Transform) enter the session
    // only through import; the wire cannot write them.
    let mut document = project::ProjectDocument::new("Wire guards");
    let translate = project::Node::new(
        "Move",
        project::NodeKind::State {
            operation: project::StateOperation::Transform(project::TransformOperation::Translate(
                [1., 2., 3.],
            )),
        },
    );
    let translate_id = translate.id;
    let scene_id = document.default_scene.unwrap();
    let scene = std::sync::Arc::make_mut(document.scenes.get_mut(&scene_id).unwrap());
    let world = scene.world;
    let project::NodeKind::Scope { children, .. } = &mut scene.nodes.get_mut(&world).unwrap().data
    else {
        unreachable!()
    };
    children.push(translate_id);
    scene.nodes.insert(translate_id, translate);
    document.validate().unwrap();

    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document).unwrap().snapshot(),
            store_id: Uuid::new_v4(),
            path: None,
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let mut apply = |edits: Value| {
        let project = core.projects.reference().unwrap();
        core.handle(request(
            Method::ProjectApply,
            json!({"project":project,"edits":edits}),
        ))
    };
    for value in [
        json!({"kind":"identity"}),
        json!({"kind":"replace","value":[1.,0.,0.,0., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.]}),
    ] {
        let rejected =
            apply(json!([{"kind":"set_transform","node_id":translate_id,"value":value}]));
        let error = rejected.error.expect("identity/replace must be refused");
        assert_eq!(error.code, ErrorCode::InvalidRequest, "{:?}", error);
        assert!(
            error.message.contains("set_transform"),
            "unexpected message: {error:?}"
        );
    }
    // Ordinary transform edits still apply, and nothing above moved the
    // session (each refusal rejects before it).
    let applied = apply(json!([{"kind":"set_transform","node_id":translate_id,
        "value":{"kind":"translate","value":[4.,5.,6.]}}]));
    assert!(applied.error.is_none(), "{:?}", applied.error);
}

#[test]
fn legacy_project_loads_read_only_and_rejects_edits() {
    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    let (snapshot, shape) = document();
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: None,
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Viewer,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let project = core.projects.reference().unwrap();
    let result = core.handle(request(
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"rename_node","node_id":shape,"label":"Changed"}]}),
    ));
    assert_eq!(result.error.unwrap().code, ErrorCode::ReadOnly);
    assert!(core.projects.is_read_only());
}

#[test]
fn deleting_an_asset_after_removing_its_reference_in_one_transaction_succeeds() {
    let mut core = CoreState::new(Limits::default());
    let mut document = project::ProjectDocument::new("Asset transaction");
    let asset = project::Asset {
        id: project::AssetId::new(),
        location: "Texture.exr".into(),
        media_type: "application/octet-stream".into(),
        sha256: "0".repeat(64),
        size: 0,
        origin: None,
    };
    let asset_id = asset.id;
    document.assets.insert(asset_id, asset);
    let shape = project::Node::new(
        "Sphere",
        project::NodeKind::Entity {
            entity: project::EntityKind::Shape,
            implementation: "sphere".into(),
            parameters: vec![project::Parameter {
                value_type: "string".into(),
                name: "filename".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: project::ParameterValues::Assets(vec![asset_id]),
            }],
            pose: None,
        },
    );
    let node_id = shape.id;
    let scene = std::sync::Arc::make_mut(
        document
            .scenes
            .get_mut(&document.default_scene.unwrap())
            .unwrap(),
    );
    let project::NodeKind::Scope { children, .. } =
        &mut scene.nodes.get_mut(&scene.world).unwrap().data
    else {
        unreachable!()
    };
    children.push(node_id);
    scene.nodes.insert(node_id, shape.clone());
    document.validate().unwrap();
    let snapshot = project::ProjectSession::new(document).unwrap().snapshot();

    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let project = core.projects.reference().unwrap();
    let rejected = core.handle(request(
        Method::ProjectApply,
        json!({"project":project,"edits":[{"kind":"delete_item","item":{"kind":"asset","id":asset_id}}]}),
    ));
    assert_eq!(
        rejected.error.unwrap().details.unwrap()["reason"],
        "referenced"
    );

    let applied = core.handle(request(
        Method::ProjectApply,
        json!({
            "project":core.projects.reference().unwrap(),
            "edits":[
                {"kind":"set_parameter","target":{"kind":"node","id":node_id},"name":"filename","value_type":"string","value":null},
                {"kind":"delete_item","item":{"kind":"asset","id":asset_id}}
            ]
        }),
    ));
    assert!(applied.error.is_none(), "{:?}", applied.error);
    assert!(
        !core
            .projects
            .active
            .as_ref()
            .unwrap()
            .session
            .document()
            .assets
            .contains_key(&asset_id)
    );
}

#[test]
fn save_reconciles_files_registered_from_unregistered_contents() {
    let mut core = CoreState::new(Limits::default());
    let (snapshot, _) = document();
    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot: snapshot.clone(),
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec!["External.bin".into()],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let save = core.handle(request(
        Method::ProjectSave,
        json!({"project":core.projects.reference().unwrap()}),
    ));
    assert!(save.error.is_none(), "{:?}", save.error);
    let save_id = serde_json::from_value(save.result.unwrap()["job_id"].clone()).unwrap();

    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("project");
    let mut store = project::FolderStore::create(
        &path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    std::fs::write(path.join("Assets/External.bin"), b"external asset").unwrap();
    let receipt = store.save(&snapshot).unwrap();
    let registered = store.read_document().unwrap();
    assert_eq!(registered.assets.len(), 1);
    core.project_io_finished(
        save_id,
        Ok(IoOutput::Saved {
            receipt,
            registered,
        }),
    );

    let active = core.projects.active.as_ref().unwrap();
    assert!(active.unregistered.is_empty());
    assert_eq!(active.session.document().assets.len(), 1);
    let contents = core.handle(request(
        Method::ProjectListContents,
        json!({"project":core.projects.reference().unwrap(),"folder":""}),
    ));
    assert!(contents.error.is_none(), "{:?}", contents.error);
    assert!(
        contents.result.unwrap()["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| entry["kind"] == "asset" && entry["name"] == "External.bin")
    );
}

#[test]
fn edits_after_open_was_requested_prevent_a_late_switch_from_discarding_them() {
    let mut core = CoreState::new(Limits::default());
    let (snapshot, shape) = document();
    let id = pending_open(&mut core);
    core.project_io_finished(
        id,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some("/first".into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let switch = pending_open(&mut core);
    let reference = core.projects.reference().unwrap();
    let edited=core.handle(request(Method::ProjectApply,json!({"project":reference,"edits":[{"kind":"rename_node","node_id":shape,"label":"Edited while loading"}]})));
    assert!(edited.error.is_none());
    let before = core.projects.context();
    let (other, _) = document();
    let discarded = Uuid::new_v4();
    core.project_io_finished(
        switch,
        Ok(IoOutput::Loaded {
            snapshot: other,
            store_id: discarded,
            path: Some("/second".into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    assert_eq!(core.projects.context(), before);
    assert!(core.projects.state(false).dirty);
    assert_eq!(
        core.job(switch).unwrap().error.as_ref().unwrap().code,
        ErrorCode::RevisionConflict
    );
    assert!(
        core.projects
            .queue
            .iter()
            .any(|command| matches!(command,IoCommand::Release(token) if *token==discarded))
    );
}
#[test]
fn cancellation_after_import_publication_reports_the_file_but_does_not_activate_it() {
    let (source, destination) = if cfg!(windows) {
        (r"C:\scene.pbrt", r"C:\new-project")
    } else {
        ("/scene.pbrt", "/new-project")
    };
    let mut core = CoreState::new(Limits::default());
    let context = core.projects.context();
    let response = core.handle(request(
        Method::ProjectImport,
        json!({"source_path":source,"destination":destination,"expected_current":context}),
    ));
    let id = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
    core.handle(request(Method::JobCancel, json!({"job_id":id})));
    let (snapshot, _) = document();
    core.project_io_finished(
        id,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(destination.into()),
            source_path: None,
            unregistered: vec![],
            published: true,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let job = core.job(id).unwrap();
    assert_eq!(job.state, JobState::Succeeded);
    assert_eq!(job.result.as_ref().unwrap()["activated"], false);
    assert_eq!(job.result.as_ref().unwrap()["published"], true);
    assert!(core.projects.active.is_none());
}

#[test]
fn cancellation_after_save_as_publication_keeps_the_viewer_active() {
    let mut core = CoreState::new(Limits::default());
    let (snapshot, _) = document();
    let old_store_id = Uuid::new_v4();
    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot: snapshot.clone(),
            store_id: old_store_id,
            path: None,
            source_path: Some("/scene.pbrt".into()),
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Viewer,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let reference = core.projects.reference().unwrap();
    let temp = tempfile::tempdir().unwrap();
    let save = core.handle(request(
        Method::ProjectSave,
        json!({"project":reference,"destination":temp.path().join("published")}),
    ));
    let job_id = serde_json::from_value(save.result.unwrap()["job_id"].clone()).unwrap();
    let cancelled = core.handle(request(Method::JobCancel, json!({"job_id":job_id})));
    assert!(cancelled.error.is_none());

    let receipt_path = temp.path().join("receipt");
    let mut folder = project::FolderStore::create(
        &receipt_path,
        snapshot.document().id,
        &snapshot.document().title,
        snapshot.revision().clone(),
    )
    .unwrap();
    let receipt = folder.save(&snapshot).unwrap();
    let new_store_id = Uuid::new_v4();
    core.project_io_finished(
        job_id,
        Ok(IoOutput::SavedAs {
            receipt,
            registered: snapshot.document().clone(),
            store_id: new_store_id,
            path: "/published".into(),
            relocated: None,
        }),
    );

    let active = core.projects.active.as_ref().unwrap();
    assert_eq!(active.store_id, old_store_id);
    assert_eq!(active.mode, dto::ProjectMode::Viewer);
    assert_eq!(active.source_path.as_deref(), Some("/scene.pbrt"));
    assert!(active.path.is_none());
    assert_eq!(core.job(job_id).unwrap().state, JobState::Succeeded);
    let result = core.job(job_id).unwrap().result.as_ref().unwrap();
    assert_eq!(result["published"], true);
    assert_eq!(result["activated"], false);
    assert_eq!(result["path"], "/published");
    assert!(
        core.projects
            .queue
            .iter()
            .any(|command| matches!(command, IoCommand::Release(token) if *token == new_store_id))
    );
    assert!(
        !core
            .projects
            .queue
            .iter()
            .any(|command| matches!(command, IoCommand::Release(token) if *token == old_store_id))
    );
}

#[test]
fn cancelled_or_failed_render_preparation_never_launches_a_renderer() {
    for cancel in [true, false] {
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
        let snapshot = document_with_camera(true);
        let open = pending_open(&mut core);
        core.project_io_finished(
            open,
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: Uuid::new_v4(),
                path: Some(crate::TEST_PROJECT.into()),
                source_path: None,
                unregistered: vec![],
                published: false,
                report: None,
                mode: dto::ProjectMode::Project,
                asset_root: None,
                staging_owner: None,
            }),
        );
        let project = core.projects.reference().unwrap();
        let response = core.handle(request(
            Method::ProjectRender,
            json!({"project":project,"expected_settings_revision":"0"}),
        ));
        assert!(response.error.is_none());
        let id = serde_json::from_value(response.result.unwrap()["job_id"].clone()).unwrap();
        let result = if cancel {
            core.cancel_job(id);
            Ok(IoOutput::Exported {
                path: "/published/scene.pbrt".into(),
                warnings: vec![],
            })
        } else {
            Err(ErrorPayload::new(
                ErrorCode::Unsupported,
                "export refused an unsupported dependency",
            ))
        };
        core.project_io_finished(id, result);
        assert_eq!(
            core.job(id).unwrap().state,
            if cancel {
                JobState::Cancelled
            } else {
                JobState::Failed
            }
        );
        assert_eq!(core.job(id).unwrap().project.as_ref(), Some(&project));
        assert_eq!(core.take_services().count(), 0);
    }
}

#[test]
fn render_scene_without_selected_camera_fails_before_export_or_renderer_start() {
    for snapshot in [document().0, document_with_camera(false)] {
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
        let open = pending_open(&mut core);
        core.project_io_finished(
            open,
            Ok(IoOutput::Loaded {
                snapshot,
                store_id: Uuid::new_v4(),
                path: Some(crate::TEST_PROJECT.into()),
                source_path: None,
                unregistered: vec![],
                published: false,
                report: None,
                mode: dto::ProjectMode::Project,
                asset_root: None,
                staging_owner: None,
            }),
        );
        let project = core.projects.reference().unwrap();
        let response = core.handle(request(
            Method::ProjectRender,
            json!({"project":project,"expected_settings_revision":"0"}),
        ));
        let error = response.error.unwrap();
        assert!(error.message.contains("No camera is set for rendering"));
        assert_eq!(error.details.unwrap()["reason"], "no_render_camera");
        assert_eq!(core.take_services().count(), 0);
    }
}

#[test]
fn closing_and_deleting_scenes_keeps_resource_queries_and_history_available() {
    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    let (snapshot, _) = document();
    let scene = snapshot.scene().unwrap();
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let reference = core.projects.reference().unwrap();
    let response = core.handle(request(
        Method::ProjectCloseScene,
        json!({"project":reference}),
    ));
    assert!(response.error.is_none(), "{:?}", response.error);
    let reference = core.projects.reference().unwrap();
    let state = core.projects.state(false);
    assert!(state.active_scene.is_none());
    assert_eq!(state.scene_count, 1);
    assert!(!state.dirty);
    for method in [
        Method::ProjectListContents,
        Method::ProjectListScenes,
        Method::ProjectListPresets,
    ] {
        let result = core.handle(request(method.clone(), json!({"project":reference})));
        assert!(result.error.is_none(), "{method:?}: {:?}", result.error);
    }
    let nodes = core.handle(request(
        Method::ProjectListNodes,
        json!({"project":reference}),
    ));
    assert_eq!(
        nodes.error.unwrap().details.unwrap()["reason"],
        "no_active_scene"
    );
    let create = core.handle(request(Method::ProjectApply, json!({"project":reference,"edits":[{
        "kind":"create_resource", "resource_kind":"material", "implementation":"diffuse", "label":"Matte"
    }]})));
    assert!(create.error.is_none(), "{:?}", create.error);
    let resource = create.result.unwrap()["created"][0]["id"].clone();
    let reference = core.projects.reference().unwrap();
    let get = core.handle(request(
        Method::ProjectGetResource,
        json!({"project":reference,"resource_id":resource}),
    ));
    assert!(get.error.is_none(), "{:?}", get.error);
    let opened = core.handle(request(
        Method::ProjectOpenScene,
        json!({"project":reference,"scene":scene}),
    ));
    assert!(opened.error.is_none(), "{:?}", opened.error);
    assert!(core.projects.state(false).can_undo);
    let reference = core.projects.reference().unwrap();
    let deleted = core.handle(request(
        Method::ProjectApply,
        json!({"project":reference,"edits":[{"kind":"delete_scene","scene":scene}]}),
    ));
    assert!(deleted.error.is_none(), "{:?}", deleted.error);
    assert_eq!(core.projects.state(false).scene_count, 0);
    assert!(core.projects.state(false).active_scene.is_none());
    let reference = core.projects.reference().unwrap();
    let undone = core.handle(request(Method::ProjectUndo, json!({"project":reference})));
    assert!(undone.error.is_none(), "{:?}", undone.error);
    assert_eq!(core.projects.state(false).scene_count, 1);
    assert!(core.projects.state(false).active_scene.is_none());
}

#[test]
fn texture_type_change_rejects_incompatible_references_without_mutating_history() {
    let mut core = CoreState::new(Limits::default());
    let mut document = project::ProjectDocument::new_empty("Types");
    let texture = project::Resource {
        id: project::ResourceId::new(),
        label: "Source".into(),
        location: "Source.texture".into(),
        edition: project::Revision::initial(),
        kind: project::ResourceKind::Texture,
        texture_value_type: Some(project::TextureValueType::Spectrum),
        implementation: "constant".into(),
        parameters: vec![],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    let material = project::Resource {
        id: project::ResourceId::new(),
        label: "Mat".into(),
        location: "Mat.material".into(),
        edition: project::Revision::initial(),
        kind: project::ResourceKind::Material,
        texture_value_type: None,
        implementation: "diffuse".into(),
        parameters: vec![project::Parameter {
            name: "reflectance".into(),
            value_type: "texture".into(),
            color_space: project::ColorSpaceBinding::Inherit,
            values: project::ParameterValues::Resources {
                kind: project::ResourceKind::Texture,
                ids: vec![texture.id],
            },
        }],
        definition_scope: None,
        declared_in: None,
        origin: None,
    };
    document.resources.insert(texture.id, texture.clone());
    document.resources.insert(material.id, material.clone());
    let open = pending_open(&mut core);
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot: ProjectSession::new(document.clone()).unwrap().snapshot(),
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let project = core.projects.reference().unwrap();
    let rejected = core.handle(request(
        Method::ProjectApply,
        json!({"project":project,"edits":[{
            "kind":"set_texture_value_type","resource_id":texture.id,"value":"float"
        }]}),
    ));
    assert_eq!(
        rejected.error.unwrap().details.unwrap()["reason"],
        "texture_type_mismatch"
    );
    assert_eq!(
        core.projects.active.as_ref().unwrap().session.document(),
        &document
    );
    assert!(!core.projects.active.as_ref().unwrap().session.can_undo());
    let accepted = core.handle(request(Method::ProjectApply, json!({"project":project,"edits":[
        {"kind":"set_parameter","target":{"kind":"resource","id":material.id},"name":"reflectance","value_type":"texture","value":null},
        {"kind":"set_texture_value_type","resource_id":texture.id,"value":"float"}
    ]})));
    assert!(accepted.error.is_none(), "{:?}", accepted.error);
    let active = core.projects.active.as_mut().unwrap();
    assert_eq!(
        active.session.document().resources[&texture.id].texture_value_type,
        Some(project::TextureValueType::Float)
    );
    active.session.undo(&active.session.revision()).unwrap();
    assert_eq!(
        active.session.document().resources[&texture.id].texture_value_type,
        Some(project::TextureValueType::Spectrum)
    );
    assert_eq!(
        active.session.document().resources[&material.id].parameters,
        material.parameters
    );
}

#[test]
fn an_imagemap_can_be_created_before_an_image_is_chosen() {
    let mut core = CoreState::new(Limits::default());
    let open = pending_open(&mut core);
    let (snapshot, _) = document();
    let before = snapshot.document().clone();
    core.project_io_finished(
        open,
        Ok(IoOutput::Loaded {
            snapshot,
            store_id: Uuid::new_v4(),
            path: Some(crate::TEST_PROJECT.into()),
            source_path: None,
            unregistered: vec![],
            published: false,
            report: None,
            mode: dto::ProjectMode::Project,
            asset_root: None,
            staging_owner: None,
        }),
    );
    let response = core.handle(request(Method::ProjectApply, json!({"project":core.projects.reference(),"edits":[{
        "kind":"create_resource","resource_kind":"texture","implementation":"imagemap","label":"Empty","texture_value_type":"spectrum","parameters":[]
    }]})));
    assert!(response.error.is_none(), "{:?}", response.error);
    let session = &core.projects.active.as_ref().unwrap().session;
    assert_eq!(
        session.document().resources.len(),
        before.resources.len() + 1
    );
    assert!(session.can_undo());
}
