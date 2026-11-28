//! Dock composition, layout persistence, menus, toolbar and workspace actions.
use super::dialogs::{
    AfterSave, DialogStep, PendingAction, UnsavedChoice, after_save, choose, resolve_new_project,
};
use super::document::{
    DocumentState, SaveAction, is_pbrt_scene_path, needs_unsaved_confirmation, save_action,
    suggested_project_name,
};
use super::menu::{
    self, AddSceneToProject, AlignCameraToView, AlignViewToSceneCamera, AlignViewToSelected,
    CancelRender, CloseDocument, CloseScene, ConfirmDeleteHierarchyNode, ContentsDelete,
    ContentsImportResource, ContentsMove, ContentsNewFolder, ContentsOpen, ContentsRename,
    CreateDefinitionPreset, CreatePreset, CreateResource, DeleteNode, Deselect, DuplicateNode,
    DuplicateScene, ExportPbrt, LitMode, MakeObjectDefinition, MenuState, NewProject, NewScene,
    NormalMode, OpenArrayEditor, OpenInEditor, OpenPbrtScene, OpenProject, OpenRecent,
    OpenRenderSettings, OpenSceneId, QuitApp, RedoEdit, ReloadSource, RenderCurrentView,
    RenderSceneCamera, ResetPanelLayout, RevealDefinition, RevealItem, RevealNode, SaveProject,
    SaveProjectAs, ShowAbout, ShowControlEndpoint, ShowInspector, SolidMode, ToggleGpu,
    TogglePanel, UndoEdit, UnpackObjectInstance, UnsavedCancel, UnsavedDiscard, UnsavedSave,
    WireframeMode,
};
use super::prompt;
use super::status;
use super::{EditorPanel, FrameAll, FrameSelected, PanelKind, Session, Workspace};
use super::{layout, recent};
use crate::backend::Backend;
use crate::backend::Command;
use gpui_kit::component::dialog::DialogFooter;
use gpui_kit::component::dock::{
    DockArea, DockAreaState, DockEvent, DockLayout, DockPlacement, DockSkin, InsertTarget, PanelId,
    PanelStyle, panel_handle, register_panel,
};
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::component::notification::Notification;
use gpui_kit::component::{Sizable, WindowExt as _, button::Button, spinner::Spinner};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use protocol::Method;
use scene_view::PreviewMode;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::{sync::Arc, time::Duration};

impl Workspace {
    pub(super) fn new(
        backend: Backend,
        layout_path: Option<PathBuf>,
        endpoint: String,
        menu_bar: Entity<AppMenuBar>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        gpui_kit::component::Theme::change(gpui_kit::component::ThemeMode::Dark, Some(window), cx);
        gpui_kit::component::Theme::global_mut(cx).font_family = "DejaVu Sans".into();
        let session = cx.new(|cx| Session::new(backend, window, cx));
        cx.observe_in(&session, window, |this, session, window, cx| {
            let (revision, scope, title, settled, unsaved, recent) = {
                let session = session.read(cx);
                let document = session.document();
                let scene = status::scene_label(&session.project_tree);
                let scope = session
                    .project
                    .context
                    .project
                    .as_ref()
                    .map(|project| {
                        format!("{}:{}", project.project_id, project.activation.as_str())
                    })
                    .unwrap_or_default();
                (
                    session.layout_revision,
                    scope,
                    status::window_title(&document, scene.as_deref()),
                    session.save_settled.clone(),
                    session.unsaved_changes,
                    recent_entry(&document),
                )
            };
            if this.layout_revision != revision {
                this.layout_revision = revision;
                Self::reset(&this.dock, &this.panels, window, cx);
            }
            if this.scope != scope {
                // The document switch succeeded; no request is pending replay.
                this.scope = scope;
                this.last_attempt = None;
                // Editor tabs belong to the document that opened them.
                this.reset_editors_for_scope(window, cx);
            }
            this.sync_editors(window, cx);
            this.remember_recent(recent);
            this.refresh_menu(cx);
            if this.window_title != title {
                this.window_title = title.clone();
                window.set_window_title(&title);
            }
            if settled.is_some() {
                session.update(cx, |session, _| session.save_settled = None);
            }
            if unsaved {
                session.update(cx, |session, _| session.unsaved_changes = false);
            }
            if let Some((ok, error)) = settled {
                let pending = this.pending.take();
                if !ok {
                    let error = error.unwrap_or_else(|| "unsaved changes kept".into());
                    // The Console is often behind another tab; a failed save must be seen.
                    window.push_notification(
                        Notification::error(error.clone())
                            .title("Save failed")
                            .autohide(false),
                        cx,
                    );
                    this.set_status(format!("Save failed — {error}"), cx);
                }
                if let AfterSave::Run(action) = after_save(ok, pending) {
                    this.run_pending(action, false, window, cx);
                }
            }
            if unsaved && let Some(action) = this.last_attempt.take() {
                this.open_unsaved_dialog(action, window, cx);
            }
            cx.notify();
        })
        .detach();
        let workspace = cx.entity();
        window.on_window_should_close(cx, move |window, cx| {
            workspace.update(cx, |this, cx| this.confirm_window_close(window, cx))
        });
        let (dock, skin) =
            DockSkin::dock_area("pbrt-editor", Some(layout::LAYOUT_VERSION), window, cx);
        skin.set_panel_style(PanelStyle::TabBar, cx);
        skin.set_close_button_visible(true, cx);
        let panels = PanelKind::ALL
            .into_iter()
            .map(|kind| {
                let panel = cx.new(|cx| {
                    EditorPanel::new(kind, session.clone(), layout_path.clone(), window, cx)
                });
                let handle = panel.clone();
                register_panel(cx, kind.name(), move |_, _, _| panel_handle(handle.clone()));
                panel
            })
            .collect::<Vec<_>>();
        let loaded = layout_path
            .as_ref()
            .and_then(|path| std::fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice::<DockAreaState>(&bytes).ok())
            .filter(|state| layout::accepts_saved_layout(state.version))
            .is_some_and(|state| {
                dock.update(cx, |dock, cx| dock.load(state, window, cx))
                    .is_ok()
            });
        if !loaded {
            Self::reset(&dock, &panels, window, cx);
        }
        // Closing a tab or moving panels changes the Window menu checks.
        cx.subscribe(&dock, |this, _, event: &DockEvent, cx| {
            if matches!(event, DockEvent::LayoutChanged) {
                this.refresh_menu(cx);
            }
        })
        .detach();
        let recent_path = layout_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|directory| directory.join("recent.json"));
        let recent = recent_path.as_deref().map(recent::load).unwrap_or_default();
        if let Some(path) = layout_path
            .as_ref()
            .and_then(|path| path.parent())
            .map(|directory| directory.join("inspector.json"))
        {
            session.update(cx, |session, _| {
                session.folded = super::components::Folded::load(&path);
                session.folded_path = Some(path);
            });
        }
        if let Some(path) = layout_path {
            let latest = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let writer = Arc::new(std::sync::Mutex::new(()));
            cx.subscribe_in(
                &dock,
                window,
                move |this, dock, event: &DockEvent, window, cx| {
                    if matches!(event, DockEvent::LayoutChanged) {
                        let dock = dock.clone();
                        let path = path.clone();
                        let ticket = latest.fetch_add(1, std::sync::atomic::Ordering::AcqRel) + 1;
                        let latest = latest.clone();
                        let writer = writer.clone();
                        this.layout_save = Some(cx.spawn_in(window, async move |this, cx| {
                            // Wait for the new split tree to receive real layout
                            // measurements; its initial flexible slots are 100px.
                            cx.background_executor()
                                .timer(Duration::from_millis(200))
                                .await;
                            let Ok(state) = this.update_in(cx, |_, _, cx| dock.read(cx).dump(cx))
                            else {
                                return;
                            };
                            cx.background_executor()
                                .spawn(async move {
                                    let _guard = writer.lock().unwrap();
                                    if latest.load(std::sync::atomic::Ordering::Acquire) != ticket {
                                        return;
                                    }
                                    // Editor tabs are never restored.
                                    let mut state =
                                        serde_json::to_value(&state).unwrap_or_default();
                                    super::editor_panels::strip_editors(&mut state);
                                    if let (Some(parent), Ok(bytes)) =
                                        (path.parent(), serde_json::to_vec_pretty(&state))
                                        && std::fs::create_dir_all(parent).is_ok()
                                    {
                                        let temporary = path.with_extension(format!(
                                            "{}.tmp",
                                            uuid::Uuid::new_v4()
                                        ));
                                        if std::fs::write(&temporary, bytes).is_ok() {
                                            let _ = std::fs::rename(temporary, path);
                                        }
                                    }
                                })
                                .await;
                        }));
                    }
                },
            )
            .detach();
        }
        let mut workspace = Self {
            layout_revision: 0,
            focus: {
                let focus = cx.focus_handle();
                focus.focus(window, cx);
                focus
            },
            session,
            dock,
            panels,
            layout_save: None,
            menu_bar,
            menu_state: MenuState::from_parts(
                &DocumentState::Empty,
                false,
                false,
                false,
                false,
                &serde_json::json!({"jobs": []}),
                PreviewMode::Solid,
            ),
            pending: None,
            last_attempt: None,
            scope: String::new(),
            endpoint,
            window_title: "pbrt-ui".into(),
            recent_path,
            recent,
            recent_key: None,
            editors: Vec::new(),
            editors_checked: serde_json::Value::Null,
            editor_scope: String::new(),
            editor_revision: serde_json::Value::Null,
            next_editor_request: 0,
            latest_editor_requests: Default::default(),
        };
        workspace.refresh_menu(cx);
        workspace
    }

    pub(super) fn panel(&self, kind: PanelKind) -> &Entity<EditorPanel> {
        let index = PanelKind::ALL
            .iter()
            .position(|other| *other == kind)
            .expect("every panel kind is created");
        &self.panels[index]
    }

    /// Inspector link: show Contents, open the item's folders and select it.
    fn reveal_item(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.session.read(cx).known_item(id) else {
            self.set_status("That item is not listed in Contents".into(), cx);
            return;
        };
        if !self.panel_open(PanelKind::Contents, cx)
            && let Some(index) = PanelKind::ALL
                .iter()
                .position(|k| *k == PanelKind::Contents)
        {
            self.toggle_fixed_panel(index, window, cx);
        }
        let contents = PanelId::from(self.panel(PanelKind::Contents).entity_id());
        self.dock
            .update(cx, |dock, cx| dock.select_panel(contents, window, cx));
        self.session.update(cx, |session, cx| {
            let project = session.project_value();
            let location = entry["location"].as_str().unwrap_or_default().to_owned();
            let folders = if entry["kind"] == "folder" || entry["builtin"] == true {
                session.contents.open_folder(
                    if entry["builtin"] == true && entry["kind"] != "folder" {
                        super::contents_tree::BUILTIN
                    } else {
                        &location
                    },
                    &project,
                )
            } else {
                session.contents.reveal(&location, &project)
            };
            for folder in folders {
                session.send(
                    Command::ProjectContents {
                        project: project.clone(),
                        folder,
                        offset: 0,
                        limit: super::contents_tree::PAGE_LIMIT,
                    },
                    cx,
                );
            }
            session.select_contents(entry, project, cx);
        });
    }

    /// Reveal a where-used row's node in the
    /// Hierarchy. A row pointing at another scene switches to it first —
    /// immediately, with no confirmation (2026-10-29 ruling) — and the
    /// Hierarchy fires the pending reveal once its tree is that scene's.
    fn reveal_where_used_node(
        &mut self,
        action: &RevealNode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project = self.session.read(cx).project_value();
        if !project.is_object() {
            return;
        }
        if !self.panel_open(PanelKind::Hierarchy, cx)
            && let Some(index) = PanelKind::ALL
                .iter()
                .position(|k| *k == PanelKind::Hierarchy)
        {
            self.toggle_fixed_panel(index, window, cx);
        }
        let hierarchy = PanelId::from(self.panel(PanelKind::Hierarchy).entity_id());
        self.dock
            .update(cx, |dock, cx| dock.select_panel(hierarchy, window, cx));
        let RevealNode {
            scene,
            node_id,
            path,
        } = action;
        // The scene-graph tree's first root is the active Scene node.
        let active = self.session.read(cx).project_tree["nodes"][0]["id"].clone();
        self.session.update(cx, |session, cx| {
            session.reveal_where_used = Some((scene.clone(), node_id.clone(), path.clone()));
            if active != *scene {
                // The switch lands as an Update::Project, which itself
                // wakes the Hierarchy's session observer.
                session.send(
                    Command::Project(
                        Method::ProjectOpenScene,
                        json!({"project":project,"scene":scene}),
                    ),
                    cx,
                );
            } else {
                // Nothing is in flight, and the Hierarchy only picks the
                // pending reveal up from its session observer - wake it,
                // or the reveal sits unseen until an unrelated update.
                cx.notify();
            }
        });
    }

    /// Open (or show) the dedicated editor for a Contents item.
    fn open_in_editor(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        let entry = {
            let session = self.session.read(cx);
            session
                .contents
                .selected()
                .filter(|entry| entry["id"].as_str() == Some(id))
                .cloned()
                .or_else(|| session.known_item(id))
        };
        let Some(entry) = entry.filter(|entry| super::editor_panels::editor_for(entry).is_some())
        else {
            self.set_status("No editor is registered for this item".into(), cx);
            return;
        };
        let kind = super::editor_panels::editor_for(&entry).unwrap();
        let identity = super::editor_panels::identity(kind, &entry);
        if let Some(editor) = self
            .editors
            .iter()
            .find(|editor| {
                let editor = editor.read(cx);
                super::editor_panels::identity(editor.kind(), &editor.entry) == identity
                    && editor.item_id() == Some(id)
            })
            .cloned()
        {
            self.show_editor(editor, window, cx);
            return;
        }
        self.request_editor(entry, kind, window, cx);
    }

    fn open_array_editor(
        &mut self,
        action: &OpenArrayEditor,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let id = format!(
            "array:{}:{}",
            action.target["id"].as_str().unwrap_or_default(),
            action.name
        );
        let entry = serde_json::json!({
            "kind":"parameter", "editor_kind":"array", "id":id, "name":action.name,
            "target":action.target, "table":{
                "name":action.table.name,"value_type":action.table.value_type,"kind":action.table.kind,
                "count":action.table.count,"width":action.table.width,"columns":action.table.columns,
            },
        });
        let kind = super::editor_panels::EditorKind::ArrayEditor;
        let identity = super::editor_panels::identity(kind, &entry);
        if let Some(editor) = self
            .editors
            .iter()
            .find(|editor| {
                let editor = editor.read(cx);
                super::editor_panels::identity(editor.kind(), &editor.entry) == identity
                    && editor.item_id() == Some(id.as_str())
            })
            .cloned()
        {
            self.show_editor(editor, window, cx);
            return;
        }
        self.request_editor(entry, kind, window, cx);
    }

    fn request_editor(
        &mut self,
        entry: serde_json::Value,
        kind: super::editor_panels::EditorKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let identity = super::editor_panels::identity(kind, &entry);
        self.next_editor_request = self.next_editor_request.wrapping_add(1);
        let request_id = self.next_editor_request;
        let previous_request = self
            .latest_editor_requests
            .insert(identity.clone(), request_id);
        let existing = self
            .editors
            .iter()
            .find(|editor| {
                let editor = editor.read(cx);
                super::editor_panels::identity(editor.kind(), &editor.entry) == identity
            })
            .cloned();
        let requested_entry = entry.clone();
        let sent = self.session.update(cx, |session, _| {
            let project = session.project_value();
            session
                .backend
                .commands
                .try_send(Command::OpenEditor {
                    project,
                    entry,
                    request_id,
                })
                .is_ok()
        });
        if !sent {
            if let Some(previous) = previous_request {
                self.latest_editor_requests.insert(identity, previous);
            } else {
                self.latest_editor_requests.remove(&identity);
            }
            self.set_status("Editor request queue is full".into(), cx);
            return;
        }
        if let Some(editor) = existing {
            editor.update(cx, |editor, cx| {
                editor.begin_switch(requested_entry, window, cx)
            });
            self.show_editor(editor, window, cx);
        }
    }

    /// Create tabs the backend resolved, close tabs whose item is gone, and
    /// recheck open tabs after each revision.
    fn sync_editors(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (ready, missing, project) = self.session.update(cx, |session, _| {
            (
                std::mem::take(&mut session.editors_ready),
                std::mem::take(&mut session.items_missing),
                session.project_value(),
            )
        });
        let editor_scope = super::editor_panels::project_scope(&project);
        let revision = project["revision"].clone();
        if self.editor_scope != editor_scope {
            self.editor_scope = editor_scope;
            self.editor_revision = revision;
            self.latest_editor_requests.clear();
            self.reset_editors_for_scope(window, cx);
        } else if self.editor_revision != revision {
            self.editor_revision = revision;
            self.latest_editor_requests.clear();
            for editor in &self.editors {
                if editor.read(cx).is_loading() {
                    editor.update(cx, |editor, cx| editor.clear_target(window, cx));
                }
            }
        }
        for (response_project, request_id, entry, path) in ready {
            let Some(kind) = super::editor_panels::editor_for(&entry) else {
                continue;
            };
            let identity = super::editor_panels::identity(kind, &entry);
            if !super::editor_panels::response_is_current(
                &self.latest_editor_requests,
                &identity,
                request_id,
                &response_project,
                &project,
            ) {
                continue;
            }
            self.latest_editor_requests.remove(&identity);
            let path = match path {
                Ok(path) => path,
                Err(error) => {
                    let message = error.message.clone();
                    if let Some(editor) = self
                        .editors
                        .iter()
                        .find(|editor| {
                            let editor = editor.read(cx);
                            super::editor_panels::identity(editor.kind(), &editor.entry) == identity
                        })
                        .cloned()
                    {
                        let panel = PanelId::from(editor.entity_id());
                        let was_open = self.dock.read(cx).panel(panel).is_some();
                        editor.update(cx, |editor, cx| {
                            editor.show_open_error(entry.clone(), message.clone(), window, cx)
                        });
                        if super::editor_panels::should_show_editor_on_response(true, was_open) {
                            self.show_editor(editor, window, cx);
                        }
                    } else {
                        let session = self.session.clone();
                        let editor = cx.new(|cx| {
                            super::resource_editor::ResourceEditor::new(
                                kind,
                                entry.clone(),
                                None,
                                session,
                                window,
                                cx,
                            )
                        });
                        editor.update(cx, |editor, cx| {
                            editor.show_open_error(entry, message, window, cx)
                        });
                        self.editors.push(editor.clone());
                        self.show_editor(editor, window, cx);
                    }
                    self.set_status(error.message, cx);
                    continue;
                }
            };
            if let Some(editor) = self
                .editors
                .iter()
                .find(|editor| {
                    let editor = editor.read(cx);
                    super::editor_panels::identity(editor.kind(), &editor.entry) == identity
                })
                .cloned()
            {
                let panel = PanelId::from(editor.entity_id());
                let was_open = self.dock.read(cx).panel(panel).is_some();
                editor.update(cx, |editor, cx| {
                    editor.switch_target(entry, path, window, cx)
                });
                if super::editor_panels::should_show_editor_on_response(true, was_open) {
                    self.show_editor(editor, window, cx);
                }
                continue;
            }
            let editor = {
                let session = self.session.clone();
                cx.new(|cx| {
                    super::resource_editor::ResourceEditor::new(
                        kind, entry, path, session, window, cx,
                    )
                })
            };
            self.editors.push(editor.clone());
            self.show_editor(editor, window, cx);
        }
        for (missing_project, missing_ids) in missing {
            if missing_project != project {
                continue;
            }
            for editor in &self.editors {
                let remove_target = editor
                    .read(cx)
                    .item_id()
                    .is_some_and(|id| missing_ids.iter().any(|gone| gone == id));
                if remove_target {
                    editor.update(cx, |editor, cx| editor.clear_target(window, cx));
                }
            }
        }
        if !self.editors.is_empty() && project.is_object() && project != self.editors_checked {
            self.editors_checked = project.clone();
            let ids = self
                .editors
                .iter()
                .filter_map(|editor| {
                    let editor = editor.read(cx);
                    (editor.entry["kind"] != "parameter")
                        .then(|| editor.item_id().map(str::to_owned))
                        .flatten()
                })
                .collect();
            self.session.update(cx, |session, cx| {
                session.send(Command::CheckItems { project, ids }, cx)
            });
        }
    }

    pub(super) fn panel_open(&self, kind: PanelKind, cx: &App) -> bool {
        let id = PanelId::from(self.panel(kind).entity_id());
        self.dock.read(cx).panel(id).is_some()
    }

    fn show_editor(
        &mut self,
        editor: Entity<super::resource_editor::ResourceEditor>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = PanelId::from(editor.entity_id());
        if self.dock.read(cx).panel(panel).is_some() {
            self.dock
                .update(cx, |dock, cx| dock.select_panel(panel, window, cx));
            return;
        }
        let scene = PanelId::from(self.panel(PanelKind::Scene).entity_id());
        self.dock.update(cx, |dock, cx| {
            let group = dock
                .layout(DockPlacement::Center)
                .and_then(|tree| tree.find_panel_node(scene));
            dock.add_panel_view(
                panel_handle(editor.clone()),
                DockPlacement::Center,
                None,
                window,
                cx,
            );
            if let Some(node) = group {
                dock.move_panel(
                    panel,
                    InsertTarget::Tabs {
                        node,
                        ix: None,
                        activate: true,
                    },
                    window,
                    cx,
                );
            }
        });
    }

    fn reset_editors_for_scope(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.latest_editor_requests.clear();
        for editor in &self.editors {
            editor.update(cx, |editor, cx| editor.clear_target(window, cx));
        }
    }

    fn toggle_window_item(
        &mut self,
        item: &menu::WindowItemKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match item {
            menu::WindowItemKey::Fixed(kind) => {
                if let Some(index) = PanelKind::ALL
                    .iter()
                    .position(|candidate| candidate == kind)
                {
                    self.toggle_fixed_panel(index, window, cx);
                }
            }
            menu::WindowItemKey::Resource(kind) => {
                if let Some(editor) = self
                    .editors
                    .iter()
                    .find(|editor| editor.read(cx).kind() == *kind)
                    .cloned()
                {
                    let panel = PanelId::from(editor.entity_id());
                    if self.dock.read(cx).panel(panel).is_some() {
                        self.dock
                            .update(cx, |dock, cx| dock.remove_panel(editor.clone(), window, cx));
                    } else {
                        self.show_editor(editor, window, cx);
                    }
                } else {
                    let session = self.session.clone();
                    let editor = cx.new(|cx| {
                        super::resource_editor::ResourceEditor::new(
                            *kind,
                            serde_json::Value::Null,
                            None,
                            session,
                            window,
                            cx,
                        )
                    });
                    self.editors.push(editor.clone());
                    self.show_editor(editor, window, cx);
                }
            }
        }
    }

    /// Rebuild the menus when anything they show changed.
    fn refresh_menu(&mut self, cx: &mut Context<Self>) {
        let mut state = {
            let session = self.session.read(cx);
            MenuState::from_parts(
                &session.document(),
                session.project.can_undo,
                session.project.can_redo,
                session.settings["settings"]["gpu"] == true,
                session.render_start.active(),
                &session.jobs,
                session.preview_mode,
            )
        };
        state.panels = PanelKind::ALL
            .iter()
            .map(|kind| menu::WindowMenuItem {
                key: menu::WindowItemKey::Fixed(*kind),
                label: kind.name().into(),
                open: self.panel_open(*kind, cx),
            })
            .collect();
        for kind in [
            super::editor_panels::EditorKind::MaterialEditor,
            super::editor_panels::EditorKind::TextureEditor,
            super::editor_panels::EditorKind::ImageViewer,
            super::editor_panels::EditorKind::MeshViewer,
            super::editor_panels::EditorKind::ArrayEditor,
            super::editor_panels::EditorKind::SpectrumEditor,
        ] {
            let editor = self
                .editors
                .iter()
                .find(|editor| editor.read(cx).kind() == kind);
            let open = editor.is_some_and(|editor| {
                self.dock
                    .read(cx)
                    .panel(PanelId::from(editor.entity_id()))
                    .is_some()
            });
            state.panels.push(menu::WindowMenuItem {
                key: menu::WindowItemKey::Resource(kind),
                label: editor
                    .map(|editor| editor.read(cx).title_for_menu())
                    .unwrap_or_else(|| kind.name().into()),
                open,
            });
        }
        state.recent = self
            .recent
            .iter()
            .map(|entry| menu::RecentItem {
                label: recent::menu_label(&entry.path, &home_directory()),
                missing: !entry.path.exists(),
            })
            .collect();
        {
            let session = self.session.read(cx);
            state.has_scene = session.project.active_scene.is_some();
            state.scenes = session.project_scenes["scenes"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|scene| {
                    Some(menu::SceneItem {
                        id: uuid::Uuid::parse_str(scene["id"].as_str()?).ok()?,
                        label: scene["label"].as_str().unwrap_or_default().to_owned(),
                        active: scene["active"] == true,
                    })
                })
                .collect();
            state.presets = super::create_menu::presets(&session.project_presets);
            state.selected_project_node = session.selected_project_node.is_some();
            state.selected_camera = session
                .selected_project_node
                .as_deref()
                .is_some_and(|node| {
                    super::project_actions::object_kind(&session.project_tree, node)
                        .is_some_and(|object| object == "camera")
                });
            state.render_camera_available =
                super::project_actions::align_camera_target(&session.project_tree, None).is_some();
            state.node_selected = state.selected_project_node
                && session
                    .selected_project_node
                    .as_deref()
                    .and_then(|node| {
                        super::project_actions::object_kind(&session.project_tree, node)
                    })
                    .is_some_and(|object| super::node_edits::duplicate_delete(&object));
        }
        if self.menu_state != state {
            self.menu_state = state;
            let menu_bar = self.menu_bar.clone();
            menu::apply(cx, &self.menu_state, &menu_bar);
        }
    }

    /// Window > panel: close an open panel, or bring a closed one back into
    /// its default tab group (a new center tab when that group is gone).
    fn toggle_fixed_panel(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(kind) = PanelKind::ALL.get(index).copied() else {
            return;
        };
        if self.panel_open(kind, cx) {
            let panel = self.panel(kind).clone();
            self.dock
                .update(cx, |dock, cx| dock.remove_panel(panel, window, cx));
            return;
        }
        self.show_fixed_panel(kind, window, cx);
    }

    /// Bring a closed panel back into its default tab group (a new
    /// center tab when that group is gone). An open panel is left as
    /// it is — a node editor's selection asks with this and must never
    /// close what it wants shown.
    pub(super) fn show_fixed_panel(
        &mut self,
        kind: PanelKind,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let panel = self.panel(kind).clone();
        let id = PanelId::from(panel.entity_id());
        let siblings = layout::default_siblings(kind)
            .into_iter()
            .map(|sibling| PanelId::from(self.panel(sibling).entity_id()))
            .collect::<Vec<_>>();
        let (beside, placement, size) = layout::reopen_split(kind);
        let beside = match beside {
            layout::Beside::Root => None,
            layout::Beside::Panel(other) => Some(PanelId::from(self.panel(other).entity_id())),
        };
        self.dock.update(cx, |dock, cx| {
            let tree = dock.layout(DockPlacement::Center);
            let group = siblings
                .iter()
                .find_map(|sibling| tree?.find_panel_node(*sibling));
            let target = match group {
                Some(node) => Some(InsertTarget::Tabs {
                    node,
                    ix: None,
                    activate: true,
                }),
                // No group left: split beside where the default arrangement has it.
                None => match beside {
                    None => tree.map(|tree| tree.root().id()),
                    Some(other) => tree.and_then(|tree| tree.find_panel_node(other)),
                }
                .map(|node| InsertTarget::Split {
                    node,
                    placement,
                    size: size.map(px),
                }),
            };
            dock.add_panel_view(panel_handle(panel), DockPlacement::Center, None, window, cx);
            if let Some(target) = target {
                dock.move_panel(id, target, window, cx);
            }
        });
    }

    /// Add a document to Open Recent once it opened (Open, Import, Save As).
    fn remember_recent(&mut self, entry: Option<recent::RecentEntry>) {
        let Some(entry) = entry else {
            self.recent_key = None;
            return;
        };
        let key = (entry.kind, entry.path.clone());
        if self.recent_key.as_ref() == Some(&key) {
            return;
        }
        self.recent_key = Some(key);
        recent::merge(&mut self.recent, entry);
        self.save_recent();
    }

    pub(super) fn forget_recent(&mut self, path: &Path, cx: &mut Context<Self>) {
        recent::remove(&mut self.recent, path);
        self.save_recent();
        self.refresh_menu(cx);
        cx.notify();
    }

    fn save_recent(&self) {
        if let Some(path) = &self.recent_path {
            let _ = recent::save(path, &self.recent);
        }
    }

    fn open_recent(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.recent.get(index).cloned() else {
            return;
        };
        if !entry.path.exists() {
            self.set_status(format!("{} was not found", entry.path.display()), cx);
            return;
        }
        let action = match entry.kind {
            recent::RecentKind::Project => PendingAction::OpenProject(entry.path),
            recent::RecentKind::Pbrt => PendingAction::ImportScene(entry.path),
        };
        self.request_pending(action, window, cx);
    }

    fn reset(
        dock: &Entity<DockArea>,
        panels: &[Entity<EditorPanel>],
        window: &mut Window,
        cx: &mut App,
    ) {
        let tabs = |kinds: &[PanelKind], cx: &mut App| {
            kinds.iter().fold(DockLayout::tabs(), |tabs, kind| {
                let index = PanelKind::ALL
                    .iter()
                    .position(|other| other == kind)
                    .expect("every panel kind is created");
                tabs.panel_view(panel_handle(panels[index].clone()), cx)
            })
        };
        let [hierarchy, scene, render, bottom, inspector] = layout::DEFAULT_GROUPS;
        let center = DockLayout::v_split()
            .child(
                DockLayout::h_split()
                    .child(tabs(scene, cx), None)
                    .child(tabs(render, cx), None),
                None,
            )
            // Tall enough for the Contents search, path and one row of named items.
            .child(tabs(bottom, cx), Some(px(260.)));
        let layout = DockLayout::h_split()
            .child(tabs(hierarchy, cx), Some(px(235.)))
            .child(center, None)
            .child(tabs(inspector, cx), Some(px(330.)));
        dock.update(cx, |dock, cx| dock.set_center(layout, window, cx));
    }

    pub(super) fn set_status(&mut self, status: String, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            super::session::record_status(&mut session.status, &mut session.messages, status);
            cx.notify();
        });
    }

    /// Open Project: the portal picker is folder-only, so project folders and
    /// PBRT scenes have separate entry points.
    fn open_project_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let picker = super::path_picker::pick_paths(
            PathPromptOptions {
                files: false,
                directories: true,
                multiple: false,
                prompt: Some("Open project folder".into()),
            },
            super::path_picker::recent_directory(
                &self.recent,
                recent::RecentKind::Project,
                &home_directory(),
            ),
            cx,
        );
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        this.request_pending(PendingAction::OpenProject(path), window, cx)
                    });
                }
            }
            Ok(None) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.set_status(format!("File picker: {error:?}"), cx)
                });
            }
        })
        .detach();
    }

    /// Open PBRT Scene: import without a destination, so the document opens as
    /// a read-only Viewer until it is saved as a project.
    fn open_scene_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let picker = super::path_picker::pick_paths(
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Open PBRT scene".into()),
            },
            super::path_picker::recent_directory(
                &self.recent,
                recent::RecentKind::Pbrt,
                &home_directory(),
            ),
            cx,
        );
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, window, cx| {
                        if is_pbrt_scene_path(&path) {
                            this.request_pending(PendingAction::ImportScene(path), window, cx);
                        } else {
                            this.set_status("Choose a .pbrt or .pbrt.gz scene".into(), cx);
                        }
                    });
                }
            }
            Ok(None) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.set_status(format!("File picker: {error:?}"), cx)
                });
            }
        })
        .detach();
    }

    /// New Project: the dialog takes a destination folder —
    /// typed, or chosen with the same picker Open Project uses — and a
    /// title, then creates and switches like an open. The server owns
    /// the validation; an unusable folder text only explains itself in
    /// the status line.
    fn new_project_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let workspace = cx.entity();
        prompt::new_project(window, cx, move |folder, title, window, cx| {
            workspace.update(cx, |this, cx| {
                this.begin_new_project(&folder, &title, window, cx)
            });
        });
    }

    fn begin_new_project(
        &mut self,
        folder: &str,
        title: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match resolve_new_project(folder, title) {
            Some(new_project) => {
                let action = PendingAction::CreateProject(new_project);
                self.request_pending(action, window, cx);
            }
            None => self.set_status(
                "Name the folder the new project lives in (a new or empty one)".into(),
                cx,
            ),
        }
    }

    /// Ask about unsaved changes, then replace or close the document.
    fn request_pending(
        &mut self,
        action: PendingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if needs_unsaved_confirmation(&self.session.read(cx).document()) {
            self.open_unsaved_dialog(action, window, cx);
        } else {
            self.run_pending(action, false, window, cx);
        }
    }

    fn run_pending(
        &mut self,
        action: PendingAction,
        discard: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let context = self.session.read(cx).context_value();
        match action {
            PendingAction::OpenProject(path) => {
                let Some(path) = path.to_str() else {
                    self.set_status("Project path is not UTF-8".into(), cx);
                    return;
                };
                self.last_attempt = Some(PendingAction::OpenProject(PathBuf::from(path)));
                self.send_project(
                    Method::ProjectOpen,
                    serde_json::json!({
                        "path":path,
                        "expected_current":context,
                        "discard_unsaved":discard,
                    }),
                    cx,
                );
            }
            PendingAction::ImportScene(path) => {
                let Some(source) = path.to_str() else {
                    self.set_status("Scene path is not UTF-8".into(), cx);
                    return;
                };
                self.last_attempt = Some(PendingAction::ImportScene(PathBuf::from(source)));
                self.send_project(
                    Method::ProjectImport,
                    serde_json::json!({
                        "source_path":source,
                        "expected_current":context,
                        "discard_unsaved":discard,
                    }),
                    cx,
                );
            }
            PendingAction::CreateProject(new_project) => {
                let Some(path) = new_project.path.to_str() else {
                    self.set_status("Project path is not UTF-8".into(), cx);
                    return;
                };
                self.last_attempt = Some(PendingAction::CreateProject(new_project.clone()));
                self.send_project(
                    Method::ProjectCreate,
                    json!({
                        "path":path,
                        "title":new_project.title,
                        "expected_current":context,
                        "discard_unsaved":discard,
                    }),
                    cx,
                );
            }
            PendingAction::CloseDocument => {
                self.last_attempt = Some(PendingAction::CloseDocument);
                self.send_project(
                    Method::ProjectClose,
                    serde_json::json!({
                        "expected_current":context,
                        "discard_unsaved":discard,
                    }),
                    cx,
                );
            }
            PendingAction::CloseWindow => window.remove_window(),
            PendingAction::Quit => cx.quit(),
        }
    }

    fn send_project(&self, method: Method, params: serde_json::Value, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.send(Command::Project(method, params), cx)
        });
    }

    /// Import a PBRT scene into the project that is already open. Unlike
    /// Open PBRT Scene this never replaces the document: it adds one undoable
    /// batch, keeps the active scene and view and leaves saving to the user.
    fn add_scene_to_project_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let picker = super::path_picker::pick_paths(
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Add PBRT scene to project".into()),
            },
            super::path_picker::recent_directory(
                &self.recent,
                recent::RecentKind::Pbrt,
                &home_directory(),
            ),
            cx,
        );
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, _, cx| {
                        if is_pbrt_scene_path(&path) {
                            this.import_into_project(Method::ProjectImportScene, &path, None, cx);
                        } else {
                            this.set_status("Choose a .pbrt or .pbrt.gz scene".into(), cx);
                        }
                    });
                }
            }
            Ok(None) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.set_status(format!("File picker: {error:?}"), cx)
                });
            }
        })
        .detach();
    }

    /// Import one image or PLY as a resource from the Contents panel. A
    /// folder row aims the placement, a resource/asset row aims its folder,
    /// and a scene row aims nothing - the import places at the Assets root.
    fn import_resource_picker(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let selected = self.session.read(cx).contents.selected().cloned();
        // Always explicit: a single resource import has no default folder,
        // so the Assets root is sent as such.
        let folder = Some(super::contents_tree::resource_import_folder(
            selected.as_ref(),
        ));
        let picker = super::path_picker::pick_paths(
            PathPromptOptions {
                files: true,
                directories: false,
                multiple: false,
                prompt: Some("Import image, PLY or SPD".into()),
            },
            home_directory(),
            cx,
        );
        cx.spawn(async move |this, cx| match picker.await {
            Ok(Some(paths)) => {
                if let Some(path) = paths.into_iter().next() {
                    let _ = this.update_in(cx, |this, _, cx| {
                        if is_resource_import_path(&path) {
                            this.import_into_project(
                                Method::ProjectImportResource,
                                &path,
                                folder.clone(),
                                cx,
                            );
                        } else {
                            this.set_status(
                                "Choose a PNG, JPEG, TGA, HDR, EXR, PLY, PLY.GZ or SPD file".into(),
                                cx,
                            );
                        }
                    });
                }
            }
            Ok(None) => {}
            error => {
                let _ = this.update(cx, |this, cx| {
                    this.set_status(format!("File picker: {error:?}"), cx)
                });
            }
        })
        .detach();
    }

    /// One shared path for both import commands: an editable project, a
    /// UTF-8 source and the folder the Contents panel selected (if any).
    fn import_into_project(
        &mut self,
        method: Method,
        source: &std::path::Path,
        folder: Option<String>,
        cx: &mut Context<Self>,
    ) {
        let session = self.session.read(cx);
        if !session.project_value().is_object() || session.document().is_read_only() {
            self.set_status("Viewer is read-only — Save as Project to edit".into(), cx);
            return;
        }
        let project = session.project_value();
        let Some(source) = source.to_str() else {
            self.set_status("Source path is not UTF-8".into(), cx);
            return;
        };
        let mut params = serde_json::json!({"project":project,"source_path":source});
        if let Some(folder) = folder {
            params["folder"] = serde_json::json!(folder);
        }
        self.set_status("Importing…".into(), cx);
        self.send_project(method, params, cx);
    }

    /// Reload the Viewer's source, keeping the camera. A Viewer is never dirty.
    fn reload_source(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let source = match self.session.read(cx).document() {
            DocumentState::Viewer { source_path, .. } => source_path,
            _ => return,
        };
        let Some(source) = source.to_str() else {
            self.set_status("Scene path is not UTF-8".into(), cx);
            return;
        };
        self.session.update(cx, |session, cx| {
            session.send(Command::ReloadViewer(source.to_owned()), cx)
        });
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match save_action(&self.session.read(cx).document()) {
            SaveAction::Disabled => {}
            SaveAction::InPlace => self.save_in_place(cx),
            SaveAction::ChooseDestination => self.save_as_picker(window, cx),
        }
    }

    fn save_in_place(&mut self, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if project.is_null() {
            return;
        }
        self.session.update(cx, |session, cx| {
            session.save_in_flight = true;
            session.send(
                Command::Project(Method::ProjectSave, serde_json::json!({"project":project})),
                cx,
            );
        });
    }

    fn save_as_picker(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let document = self.session.read(cx).document();
        let title = match &document {
            DocumentState::Viewer { title, .. } | DocumentState::Project { title, .. } => {
                title.clone()
            }
            DocumentState::Empty => return,
        };
        let name = suggested_project_name(&title);
        let project_path = match &document {
            DocumentState::Project { path, .. } => Some(path.as_path()),
            _ => None,
        };
        let directory =
            super::path_picker::preferred_directory(project_path, &self.recent, &home_directory());
        let picker = cx.prompt_for_new_path(&directory, Some(&name));
        let session = self.session.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(path))) = picker.await
                && let Some(destination) = path.to_str()
            {
                session.update(cx, |session, cx| {
                    let project = session.project_value();
                    if !project.is_null() {
                        session.save_in_flight = true;
                        session.send(
                            Command::Project(
                                Method::ProjectSave,
                                serde_json::json!({
                                    "project":project,
                                    "destination":destination,
                                }),
                            ),
                            cx,
                        );
                    }
                });
            }
        })
        .detach();
    }

    fn export_picker(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if project.is_null() {
            return;
        }
        let document = self.session.read(cx).document();
        let project_path = match &document {
            DocumentState::Project { path, .. } => Some(path.as_path()),
            _ => None,
        };
        let directory =
            super::path_picker::preferred_directory(project_path, &self.recent, &home_directory());
        let picker = cx.prompt_for_new_path(&directory, Some("pbrt-export"));
        let session = self.session.clone();
        cx.spawn(async move |_, cx| {
            if let Ok(Ok(Some(path))) = picker.await
                && let Some(destination) = path.to_str()
            {
                session.update(cx, |session, cx| {
                    session.send(
                        Command::Project(
                            Method::ProjectExport,
                            serde_json::json!({
                                "project":project,
                                "destination":destination,
                            }),
                        ),
                        cx,
                    );
                });
            }
        })
        .detach();
    }

    fn undo(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).document().is_read_only() {
            return;
        }
        let project = self.session.read(cx).project_value();
        if !project.is_null() {
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::Project(Method::ProjectUndo, serde_json::json!({"project":project})),
                    cx,
                )
            });
        }
    }

    fn redo(&mut self, cx: &mut Context<Self>) {
        if self.session.read(cx).document().is_read_only() {
            return;
        }
        let project = self.session.read(cx).project_value();
        if !project.is_null() {
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::Project(Method::ProjectRedo, serde_json::json!({"project":project})),
                    cx,
                )
            });
        }
    }

    fn set_mode(&mut self, mode: PreviewMode, cx: &mut Context<Self>) {
        self.session.update(cx, |session, cx| {
            session.preview_mode = mode;
            session.send(Command::Mode(mode), cx);
        });
    }

    fn toggle_gpu(&mut self, cx: &mut Context<Self>) {
        let state = self.session.read(cx).settings.clone();
        if let Some((settings, revision)) = super::render_settings::flipped_gpu_settings(&state) {
            self.session.update(cx, |session, cx| {
                session.send(Command::Settings(settings, revision), cx)
            });
        }
    }

    fn confirm_window_close(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if needs_unsaved_confirmation(&self.session.read(cx).document()) {
            self.open_unsaved_dialog(PendingAction::CloseWindow, window, cx);
            false
        } else {
            true
        }
    }

    fn open_unsaved_dialog(
        &mut self,
        action: PendingAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pending = Some(action);
        let document = self.session.read(cx).document();
        let title = match &document {
            DocumentState::Viewer { title, .. } | DocumentState::Project { title, .. } => {
                title.clone()
            }
            DocumentState::Empty => "the document".into(),
        };
        let jobs = self.session.read(cx).jobs.clone();
        let running = status::active_job(&jobs, document.project()).is_some();
        let description = if running {
            "Running jobs will be interrupted."
        } else {
            "Choose Save, Don't Save or Cancel."
        };
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title(format!("Save changes to “{title}”?"))
                .description(description)
                .keyboard(false)
                .footer(
                    DialogFooter::new()
                        .justify_center()
                        .child(Button::new("unsaved-cancel").label("Cancel").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                                window.dispatch_action(Box::new(UnsavedCancel), cx);
                            },
                        ))
                        .child(Button::new("unsaved-discard").label("Don't Save").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                                window.dispatch_action(Box::new(UnsavedDiscard), cx);
                            },
                        ))
                        .child(Button::new("unsaved-save").label("Save").on_click(
                            |_, window, cx| {
                                window.close_dialog(cx);
                                window.dispatch_action(Box::new(UnsavedSave), cx);
                            },
                        )),
                )
        });
    }

    /// Apply one of the unsaved-changes dialog's buttons to the pending action.
    fn resolve_unsaved(
        &mut self,
        choice: UnsavedChoice,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(action) = self.pending.take() else {
            return;
        };
        match choose(choice, action) {
            DialogStep::Nothing => {}
            DialogStep::SaveFirst(action) => {
                self.pending = Some(action);
                self.save_in_place(cx);
            }
            DialogStep::Run(action, discard) => self.run_pending(action, discard, window, cx),
        }
    }

    fn about_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = self.endpoint.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert.title("pbrt-ui").description(format!(
                "Scene editor for PBRT scenes.\nProtocol {}\nControl endpoint: {endpoint}",
                protocol::PROTOCOL_VERSION
            ))
        });
    }

    fn endpoint_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let endpoint = self.endpoint.clone();
        window.open_alert_dialog(cx, move |alert, _, _| {
            alert
                .title("Control Endpoint")
                .description(endpoint.clone())
        });
    }

    fn show_console(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.panel_open(PanelKind::Console, cx) {
            let index = PanelKind::ALL
                .iter()
                .position(|kind| *kind == PanelKind::Console)
                .unwrap_or_default();
            self.toggle_fixed_panel(index, window, cx);
        }
        let id = PanelId::from(self.panel(PanelKind::Console).entity_id());
        let dock = self.dock.clone();
        dock.update(cx, |dock, cx| dock.select_panel(id, window, cx));
    }

    fn show_render_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.session.read(cx).project.active_scene.is_none() {
            return;
        }
        if !self.panel_open(PanelKind::Inspector, cx) {
            let index = PanelKind::ALL
                .iter()
                .position(|kind| *kind == PanelKind::Inspector)
                .unwrap();
            self.toggle_fixed_panel(index, window, cx);
        }
        let inspector = PanelId::from(self.panel(PanelKind::Inspector).entity_id());
        self.panel(PanelKind::Inspector)
            .read(cx)
            .inspector_scroll
            .scroll_to_top_of_item(0);
        self.dock
            .update(cx, |dock, cx| dock.select_panel(inspector, window, cx));
        let root = self.session.read(cx).project_tree["nodes"][0]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let project = self.session.read(cx).project_value();
        if !root.is_empty() {
            self.session.update(cx, |session, cx| {
                session.unfold_component("render_settings");
                session.send(
                    Command::ProjectSelect {
                        project,
                        node_id: root,
                    },
                    cx,
                )
            });
        }
    }

    fn status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let (bar, warnings) = {
            let session = self.session.read(cx);
            let document = session.document();
            let scene = status::scene_label(&session.project_tree);
            (
                status::status_bar(
                    &document,
                    scene.as_deref(),
                    &session.jobs,
                    document.project(),
                    session.import_warnings,
                    session.preview_mode,
                ),
                session.import_warnings,
            )
        };
        let mut left = div().flex().items_center().gap_2().min_w_0();
        if let Some(badge) = bar.badge {
            left = left.child(
                div()
                    .px_1p5()
                    .rounded_sm()
                    .bg(if badge == "VIEWER" {
                        rgb(0x4a3b6b)
                    } else {
                        rgb(0x2f4a3a)
                    })
                    .text_xs()
                    .font_weight(FontWeight::BOLD)
                    .child(badge),
            );
        }
        left = left
            .child(div().truncate().child(bar.title))
            .when(bar.dirty, |this| {
                this.child(div().text_color(rgb(0xe0b050)).child("●"))
            });
        div()
            .px_3()
            .h(px(26.))
            .text_xs()
            .flex()
            .items_center()
            .gap_3()
            .bg(rgb(0x303238))
            .child(left)
            .child(div().flex_1())
            .when_some(bar.job, |this, job| {
                this.child(div().text_color(rgb(0x9aa0aa)).child(job))
            })
            .when(warnings > 0, |this| {
                this.child(
                    div()
                        .id("status-diagnostics")
                        .cursor_pointer()
                        .text_color(rgb(0xe0b050))
                        .child(format!("⚠ {warnings}"))
                        .on_click(cx.listener(|this, _, window, cx| this.show_console(window, cx))),
                )
            })
            .child(div().text_color(rgb(0x9aa0aa)).child(bar.mode))
            .into_any_element()
    }
}

impl Render for Workspace {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let foreground = self.session.read(cx).foreground_jobs.clone();
        let show_foreground = self.session.read(cx).foreground_visible;
        // The Working dialog's spinner stands out in the editor's highlight
        // blue (the theme's `accent` is a muted background tone in the dark
        // theme, which made the spinner read as grey).
        let accent: Hsla = rgb(0x7fb2f0).into();
        div()
            .id("editor-workspace")
            .relative()
            .key_context("EditorWorkspace")
            .track_focus(&self.focus)
            .on_action(
                cx.listener(|this, _: &OpenProject, window, cx| {
                    this.open_project_picker(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &NewProject, window, cx| this.new_project_dialog(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &OpenPbrtScene, window, cx| {
                    this.open_scene_picker(window, cx)
                }),
            )
            .on_action(cx.listener(|this, _: &AddSceneToProject, window, cx| {
                this.add_scene_to_project_picker(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ReloadSource, window, cx| this.reload_source(window, cx)),
            )
            .on_action(cx.listener(|this, _: &SaveProject, window, cx| this.save(window, cx)))
            .on_action(
                cx.listener(|this, _: &SaveProjectAs, window, cx| this.save_as_picker(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ExportPbrt, window, cx| this.export_picker(window, cx)),
            )
            .on_action(cx.listener(|this, _: &CloseDocument, window, cx| {
                this.request_pending(PendingAction::CloseDocument, window, cx)
            }))
            .on_action(cx.listener(|this, _: &QuitApp, window, cx| {
                this.request_pending(PendingAction::Quit, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UnsavedSave, window, cx| {
                this.resolve_unsaved(UnsavedChoice::Save, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UnsavedDiscard, window, cx| {
                this.resolve_unsaved(UnsavedChoice::Discard, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UnsavedCancel, window, cx| {
                this.resolve_unsaved(UnsavedChoice::Cancel, window, cx)
            }))
            .on_action(cx.listener(|this, _: &UndoEdit, _, cx| this.undo(cx)))
            .on_action(cx.listener(|this, _: &RedoEdit, _, cx| this.redo(cx)))
            .on_action(
                cx.listener(|this, _: &SolidMode, _, cx| this.set_mode(PreviewMode::Solid, cx)),
            )
            .on_action(
                cx.listener(|this, _: &NormalMode, _, cx| this.set_mode(PreviewMode::Normal, cx)),
            )
            .on_action(cx.listener(|this, _: &WireframeMode, _, cx| {
                this.set_mode(PreviewMode::Wireframe, cx)
            }))
            .on_action(cx.listener(|this, _: &LitMode, _, cx| this.set_mode(PreviewMode::Lit, cx)))
            .on_action(cx.listener(|this, _: &FrameSelected, _, cx| {
                if this.session.read(cx).project.active_scene.is_none() {
                    return;
                }
                this.session
                    .update(cx, |s, cx| s.send(Command::FrameSelection, cx));
            }))
            .on_action(cx.listener(|this, _: &FrameAll, _, cx| {
                if this.session.read(cx).project.active_scene.is_none() {
                    return;
                }
                this.session
                    .update(cx, |s, cx| s.send(Command::FrameAll, cx));
            }))
            .on_action(
                cx.listener(|this, _: &AlignCameraToView, _, cx| this.align_camera_to_view(cx)),
            )
            .on_action(cx.listener(|this, _: &AlignViewToSceneCamera, _, cx| {
                if this.session.read(cx).project.active_scene.is_some() {
                    this.session
                        .update(cx, |session, cx| session.send(Command::ViewSceneCamera, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &AlignViewToSelected, _, cx| {
                let (node_id, project) = {
                    let session = this.session.read(cx);
                    (
                        session.selected_project_node.clone(),
                        session.project_value(),
                    )
                };
                let Some(node_id) = node_id else {
                    this.set_status("Select a camera to align the view to".into(), cx);
                    return;
                };
                this.session.update(cx, |session, cx| {
                    session.send(Command::ViewSelectedCamera { project, node_id }, cx)
                });
            }))
            .on_action(cx.listener(|this, _: &RenderSceneCamera, _, cx| {
                if this.session.read(cx).project.active_scene.is_none() {
                    return;
                }
                this.session
                    .update(cx, |session, cx| session.start_render(false, cx));
            }))
            .on_action(cx.listener(|this, _: &OpenRenderSettings, window, cx| {
                this.show_render_settings(window, cx);
            }))
            .on_action(cx.listener(|this, _: &RenderCurrentView, _, cx| {
                if this.session.read(cx).project.active_scene.is_none() {
                    return;
                }
                this.session
                    .update(cx, |session, cx| session.start_render(true, cx));
            }))
            .on_action(cx.listener(|this, _: &ToggleGpu, _, cx| this.toggle_gpu(cx)))
            .on_action(cx.listener(|this, _: &CancelRender, _, cx| {
                this.session
                    .update(cx, |s, cx| s.send(Command::CancelRender, cx));
            }))
            .on_action(cx.listener(|this, _: &ResetPanelLayout, _, cx| {
                this.session
                    .update(cx, |s, cx| s.send(Command::ResetLayout, cx));
            }))
            .on_action(cx.listener(|this, action: &TogglePanel, window, cx| {
                this.toggle_window_item(&action.item, window, cx)
            }))
            .on_action(cx.listener(|this, _: &ShowInspector, window, cx| {
                if !this.panel_open(PanelKind::Inspector, cx) {
                    this.show_fixed_panel(PanelKind::Inspector, window, cx);
                }
            }))
            .on_action(cx.listener(|this, action: &OpenRecent, window, cx| {
                this.open_recent(action.index, window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewScene, _, cx| this.new_scene(cx)))
            .on_action(cx.listener(|this, _: &DuplicateScene, _, cx| this.duplicate_scene(cx)))
            .on_action(cx.listener(|this, _: &CloseScene, _, cx| {
                let project = this.session.read(cx).project_value();
                if project.is_object() {
                    this.session.update(cx, |session, cx| {
                        session.send(
                            Command::Project(Method::ProjectCloseScene, json!({"project":project})),
                            cx,
                        )
                    });
                }
            }))
            .on_action(cx.listener(|this, action: &OpenSceneId, _, cx| {
                this.open_scene(action.scene.to_string(), cx)
            }))
            .on_action(
                cx.listener(|this, _: &ContentsOpen, window, cx| this.contents_open(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ContentsNewFolder, _, cx| this.contents_new_folder(cx)),
            )
            .on_action(cx.listener(|this, _: &ContentsImportResource, window, cx| {
                this.import_resource_picker(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &ContentsRename, window, cx| {
                    this.contents_rename(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &ContentsMove, window, cx| this.contents_move(window, cx)),
            )
            .on_action(
                cx.listener(|this, _: &ContentsDelete, window, cx| {
                    this.contents_delete(window, cx)
                }),
            )
            .on_action(cx.listener(|this, action: &OpenInEditor, window, cx| {
                this.open_in_editor(&action.id, window, cx)
            }))
            .on_action(cx.listener(|this, action: &OpenArrayEditor, window, cx| {
                this.open_array_editor(action, window, cx)
            }))
            .on_action(cx.listener(|this, action: &RevealItem, window, cx| {
                this.reveal_item(&action.id, window, cx)
            }))
            .on_action(cx.listener(|this, action: &RevealDefinition, window, cx| {
                this.reveal_definition(&action.id, window, cx)
            }))
            .on_action(cx.listener(|this, action: &RevealNode, window, cx| {
                this.reveal_where_used_node(action, window, cx)
            }))
            .on_action(cx.listener(|this, _: &DuplicateNode, _, cx| this.duplicate_node(cx)))
            .on_action(cx.listener(|this, _: &DeleteNode, _, cx| this.delete_node(cx)))
            .on_action(
                cx.listener(|this, _: &ConfirmDeleteHierarchyNode, window, cx| {
                    this.confirm_delete_hierarchy_node(window, cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &MakeObjectDefinition, _, cx| {
                    this.make_object_definition(cx)
                }),
            )
            .on_action(
                cx.listener(|this, _: &UnpackObjectInstance, _, cx| {
                    this.unpack_object_instance(cx)
                }),
            )
            .on_action(cx.listener(|this, _: &Deselect, _, cx| {
                // Edit ▸ Deselect and the Hierarchy's Esc.
                this.session
                    .update(cx, |session, cx| session.deselect_node(cx));
            }))
            .on_action(cx.listener(|this, action: &CreatePreset, _, cx| {
                this.create_preset(action.preset, cx)
            }))
            .on_action(cx.listener(|this, action: &CreateDefinitionPreset, _, cx| {
                this.create_definition_preset(action.preset, action.parent, action.after, cx)
            }))
            .on_action(cx.listener(|this, action: &CreateResource, _, cx| {
                this.create_resource(
                    &action.kind,
                    &action.implementation,
                    action.texture_value_type.as_deref(),
                    cx,
                );
            }))
            .on_action(cx.listener(|this, _: &ShowAbout, window, cx| this.about_dialog(window, cx)))
            .on_action(cx.listener(|this, _: &ShowControlEndpoint, window, cx| {
                this.endpoint_dialog(window, cx)
            }))
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x202126))
            .text_color(rgb(0xd4d7dd))
            .child(
                div()
                    .h(px(28.))
                    .w_full()
                    .bg(rgb(0x2a2c31))
                    .child(self.menu_bar.clone()),
            )
            .child(self.toolbar(cx))
            .child(div().flex_1().min_h_0().child(
                // The Dock entity and its layout survive while the Start
                // screen replaces it.
                if matches!(self.session.read(cx).document(), DocumentState::Empty) {
                    self.start_screen(cx)
                } else {
                    self.dock.clone().into_any_element()
                },
            ))
            .when(show_foreground, |workspace| {
                let rows = foreground["jobs"].as_array().cloned().unwrap_or_default();
                workspace.child(
                    div()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left_0()
                        .right_0()
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            div()
                                .w(px(380.))
                                .p_4()
                                .rounded_md()
                                .bg(rgb(0x303238))
                                .shadow_lg()
                                .child(div().text_lg().child("Working"))
                                .children(rows.into_iter().enumerate().map(|(index, job)| {
                                    let id = job["job_id"].as_str().unwrap_or_default().to_owned();
                                    let operation = job["operation"]
                                        .as_str()
                                        .unwrap_or("Processing")
                                        .to_owned();
                                    let phase =
                                        job["phase"].as_str().unwrap_or_default().to_owned();
                                    let can_cancel = job["can_cancel"].as_bool().unwrap_or(false);
                                    let session = self.session.clone();
                                    div()
                                        .id(format!("foreground-job-{index}"))
                                        .mt_3()
                                        .flex()
                                        .items_center()
                                        .gap_3()
                                        // Long-blocking dialog: a large
                                        // spinner, vertically centred with
                                        // the row.
                                        .child(
                                            Spinner::new().color(accent).with_size(
                                                gpui_kit::component::Size::Size(px(32.)),
                                            ),
                                        )
                                        .child(div().flex_1().child(operation).when(
                                            !phase.is_empty(),
                                            |row| {
                                                row.child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(rgb(0x9aa0aa))
                                                        .child(phase),
                                                )
                                            },
                                        ))
                                        .when(can_cancel, |row| {
                                            row.child(
                                                Button::new(format!("cancel-foreground-{index}"))
                                                    .label("Cancel")
                                                    .on_click(move |_, _, cx| {
                                                        session.update(cx, |s, cx| {
                                                            s.send(
                                                                Command::CancelJob(id.clone()),
                                                                cx,
                                                            )
                                                        });
                                                    }),
                                            )
                                        })
                                })),
                        ),
                )
            })
            .child(self.status_bar(cx))
    }
}

/// The Open Recent entry for the current document, if it has a path.
fn recent_entry(document: &DocumentState) -> Option<recent::RecentEntry> {
    let (kind, path, title) = match document {
        DocumentState::Empty => return None,
        DocumentState::Viewer {
            source_path, title, ..
        } => (recent::RecentKind::Pbrt, source_path, title),
        DocumentState::Project { path, title, .. } => (recent::RecentKind::Project, path, title),
    };
    (!path.as_os_str().is_empty()).then(|| recent::RecentEntry {
        kind,
        path: path.clone(),
        title: title.clone(),
        opened_at: recent::timestamp(std::time::SystemTime::now()),
    })
}

fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
}

fn is_resource_import_path(path: &Path) -> bool {
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("");
    matches!(
        extension.to_ascii_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "tga" | "hdr" | "exr" | "ply" | "spd"
    ) || (extension.eq_ignore_ascii_case("gz")
        && path
            .file_stem()
            .and_then(|stem| Path::new(stem).extension())
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| extension.eq_ignore_ascii_case("ply")))
}

#[cfg(test)]
mod resource_import_path_tests {
    use super::is_resource_import_path;
    use std::path::Path;

    #[test]
    fn accepts_gzip_ply_but_not_unrelated_gzip_files() {
        assert!(is_resource_import_path(Path::new("model.ply.gz")));
        assert!(is_resource_import_path(Path::new("MODEL.PLY.GZ")));
        assert!(!is_resource_import_path(Path::new("model.gz")));
        assert!(!is_resource_import_path(Path::new("model.ply.gzip")));
    }

    /// Spectrum Data arrives through the same import-resource route.
    #[test]
    fn accepts_spd_files_for_spectrum_data() {
        assert!(is_resource_import_path(Path::new("Au.k.spd")));
        assert!(is_resource_import_path(Path::new("luminaire.SPD")));
        assert!(!is_resource_import_path(Path::new("spectra.spdx")));
    }
}
