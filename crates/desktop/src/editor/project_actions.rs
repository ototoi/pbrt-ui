//! Workspace actions that edit the project structure: scenes, Contents
//! folders and items, and the Create menu.
use super::contents_tree::{self, delete_edit, file_name, join, move_edit, parent, renamed};
use super::create_menu::{self, OBJECT_INSTANCE_PRESET};
use super::resource_menu;
use super::{PanelKind, Session, Workspace, prompt};
use crate::backend::{Command, Follow};
use gpui_kit::*;
use protocol::Method;
use serde_json::{Value, json};

pub(crate) fn align_camera_target(tree: &Value, selected: Option<&str>) -> Option<String> {
    fn find_render_camera(nodes: &[Value]) -> Option<String> {
        for node in nodes {
            if node["render_camera"] == true && node["object"] == "camera" {
                return node["id"].as_str().map(str::to_owned);
            }
            if let Some(found) = node["children"]
                .as_array()
                .and_then(|children| find_render_camera(children))
            {
                return Some(found);
            }
        }
        None
    }
    if let Some(selected) = selected
        && object_kind(tree, selected).as_deref() == Some("camera")
    {
        return Some(selected.to_owned());
    }
    find_render_camera(tree["nodes"].as_array()?)
}

pub(crate) fn definition_instances_for_node(inspector: &Value) -> Option<u64> {
    inspector["definition_instance_count"].as_u64()
}

impl Workspace {
    pub(super) fn align_camera_to_view(&mut self, cx: &mut Context<Self>) {
        let command = {
            let session = self.session.read(cx);
            let Some(camera) = session.view_camera.as_ref() else {
                self.set_status("Wait for the viewport camera to be ready".into(), cx);
                return;
            };
            let Some(node_id) = align_camera_target(
                &session.project_tree,
                session.selected_project_node.as_deref(),
            ) else {
                self.set_status("No scene render camera is available to align".into(), cx);
                return;
            };
            if session.document().is_read_only() {
                return;
            }
            let coordinate_system: project::CoordinateSystem = serde_json::from_value(json!({
                "up": session.project.active_up.unwrap_or(protocol::project::UpAxis::PosY),
                "handedness": session.project.active_handedness.unwrap_or(protocol::project::Handedness::Left),
            })).expect("protocol and project coordinate systems share their wire representation");
            let Some(scene_id) = session.project.active_scene else {
                return;
            };
            let project_ref = session.project_value();
            let image_size = session.image_size;
            Some(crate::backend::Command::AlignCamera {
                project: project_ref,
                scene_id: scene_id.to_string(),
                camera_id: node_id,
                coordinate_system,
                view: project::presets::PresetView {
                    position: camera.eye,
                    look_at: camera.target,
                },
                view_fov_y_degrees: camera.fov_y_degrees,
                viewport_aspect: image_size[0].max(1) as f64 / image_size[1].max(1) as f64,
            })
        };
        if let Some(command) = command {
            self.session
                .update(cx, |session, cx| session.send(command, cx));
        }
    }

    fn editable_project(&self, cx: &App) -> Option<Value> {
        let session = self.session.read(cx);
        let project = session.project_value();
        (project.is_object() && !session.document().is_read_only()).then_some(project)
    }

    fn apply(&mut self, edits: Vec<Value>, follow: Follow, cx: &mut Context<Self>) {
        let Some(project) = self.editable_project(cx) else {
            self.set_status("Viewer is read-only — Save as Project to edit".into(), cx);
            return;
        };
        self.session.update(cx, |session, cx| {
            session.send(
                Command::Apply {
                    project,
                    edits,
                    follow,
                    form: None,
                },
                cx,
            )
        });
    }

    /// Scenes in `project.list_scenes`: (id, label, location, active).
    fn scenes(&self, cx: &App) -> Vec<(String, String, String, bool)> {
        self.session.read(cx).project_scenes["scenes"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|scene| {
                Some((
                    scene["id"].as_str()?.to_owned(),
                    scene["label"].as_str().unwrap_or_default().to_owned(),
                    scene["location"].as_str()?.to_owned(),
                    scene["active"] == true,
                ))
            })
            .collect()
    }

    /// A free `.scene` location next to the active scene, whose folder is
    /// known to exist.
    fn free_scene_location(&self, stem: &str, cx: &App) -> Option<String> {
        let scenes = self.scenes(cx);
        let folder = scenes
            .iter()
            .find(|scene| scene.3)
            .map(|scene| parent(&scene.2).to_owned())?;
        let mut taken = scenes
            .iter()
            .filter(|scene| parent(&scene.2) == folder)
            .map(|scene| file_name(&scene.2).to_owned())
            .collect::<Vec<_>>();
        taken.extend(self.session.read(cx).contents.names_in(&folder));
        Some(join(
            &folder,
            &contents_tree::unique_name(&taken, stem, ".scene"),
        ))
    }

    pub(super) fn new_scene(&mut self, cx: &mut Context<Self>) {
        let Some(location) = self.free_scene_location("Scene", cx) else {
            return;
        };
        self.apply(
            vec![json!({"kind":"create_scene","location":location})],
            Follow::OpenScene,
            cx,
        );
    }

    pub(super) fn duplicate_scene(&mut self, cx: &mut Context<Self>) {
        let Some((id, label, _, _)) = self.scenes(cx).into_iter().find(|scene| scene.3) else {
            return;
        };
        let Some(location) = self.free_scene_location(&format!("{label} copy"), cx) else {
            return;
        };
        self.apply(
            vec![json!({"kind":"duplicate_scene","scene":id,"location":location})],
            Follow::OpenScene,
            cx,
        );
    }

    pub(super) fn open_scene(&mut self, scene: String, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if project.is_null() {
            return;
        }
        self.session.update(cx, |session, cx| {
            session.send(
                Command::Project(
                    Method::ProjectOpenScene,
                    json!({"project":project,"scene":scene}),
                ),
                cx,
            )
        });
    }

    fn contents_selection(&self, cx: &App) -> Option<Value> {
        self.session.read(cx).contents.selected().cloned()
    }

    pub(super) fn contents_open(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entry) = self
            .contents_selection(cx)
            .filter(|entry| entry["resource_kind"] == "object_definition")
        {
            if let Some(id) = entry["id"].as_str() {
                self.reveal_definition(id, window, cx);
            }
        } else if let Some(entry) = self.contents_selection(cx).filter(|e| e["kind"] == "scene")
            && let Some(id) = entry["id"].as_str()
        {
            self.open_scene(id.to_owned(), cx);
        }
    }

    pub(super) fn reveal_definition(
        &mut self,
        id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.panel_open(PanelKind::Hierarchy, cx) {
            self.show_fixed_panel(PanelKind::Hierarchy, window, cx);
        }
        let hierarchy_panel = self.panel(PanelKind::Hierarchy).clone();
        hierarchy_panel.update(cx, |panel, cx| panel.reveal_definition(id, cx));
    }

    pub(super) fn contents_new_folder(&mut self, cx: &mut Context<Self>) {
        let folder = contents_tree::target_folder(self.contents_selection(cx).as_ref());
        let taken = self.session.read(cx).contents.names_in(&folder);
        let path = join(
            &folder,
            &contents_tree::unique_name(&taken, "New Folder", ""),
        );
        self.apply(
            vec![json!({"kind":"create_folder","path":path})],
            Follow::Nothing,
            cx,
        );
    }

    /// Contents ▸ New: create one resource of this
    /// implementation. The selected Contents folder aims the creation —
    /// the kind's default folder when the selection is the root —
    /// and the result is selected and enters rename (`Follow::SelectResource`).
    pub(super) fn create_resource(
        &mut self,
        kind: &str,
        implementation: &str,
        texture_value_type: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let selected = self.contents_selection(cx);
        let folder = contents_tree::target_folder(selected.as_ref());
        let folder = if folder == contents_tree::ROOT {
            resource_menu::default_folder(kind).to_owned()
        } else {
            folder
        };
        let edit = resource_menu::edit(kind, implementation, texture_value_type, Some(&folder));
        self.apply(vec![edit], Follow::SelectResource { folder }, cx);
    }

    pub(super) fn contents_rename(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.contents_selection(cx).filter(contents_tree::editable) else {
            return;
        };
        let location = entry["location"].as_str().unwrap_or_default().to_owned();
        let workspace = cx.entity();
        let initial = file_name(&location).to_owned();
        prompt::text(window, cx, "Rename", &initial, move |name, _, cx| {
            let edit = renamed(&location, &name).and_then(|to| move_edit(&entry, &to));
            if let Some(edit) = edit {
                workspace.update(cx, |this, cx| this.apply(vec![edit], Follow::Nothing, cx));
            }
        });
    }

    pub(super) fn contents_move(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.contents_selection(cx).filter(contents_tree::editable) else {
            return;
        };
        let location = entry["location"].as_str().unwrap_or_default().to_owned();
        let workspace = cx.entity();
        let initial = parent(&location).to_owned();
        prompt::text(
            window,
            cx,
            "Move to folder (empty for Assets/)",
            &initial,
            move |folder, _, cx| {
                let folder = folder.trim_matches('/');
                let edit = move_edit(&entry, &join(folder, file_name(&location)));
                if let Some(edit) = edit {
                    workspace.update(cx, |this, cx| this.apply(vec![edit], Follow::Nothing, cx));
                }
            },
        );
    }

    pub(super) fn contents_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.contents_selection(cx).filter(contents_tree::editable) else {
            return;
        };
        let workspace = cx.entity();
        confirm_item_delete(
            self.session.clone(),
            entry,
            window,
            cx,
            move |edit, _, cx| {
                workspace.update(cx, |this, cx| {
                    this.session
                        .update(cx, |session, _| session.contents.select(None));
                    this.apply(vec![edit], Follow::Nothing, cx);
                });
            },
        );
    }

    /// The selected Hierarchy object and its scene-graph kind.
    fn selected_object(&self, cx: &App) -> Option<(String, String)> {
        let session = self.session.read(cx);
        let node = session.selected_project_node.clone()?;
        let object = object_kind(&session.project_tree, &node)?;
        super::node_edits::duplicate_delete(&object).then_some((node, object))
    }

    /// Edit > Duplicate: copy the object after itself, then select the copy.
    pub(super) fn duplicate_node(&mut self, cx: &mut Context<Self>) {
        let Some((node, _)) = self.selected_object(cx) else {
            return;
        };
        let parent =
            super::node_edits::duplicate_parent(&self.session.read(cx).project_tree, &node);
        let follow = match parent {
            Some(parent) => Follow::Reveal { parent },
            None => Follow::Nothing,
        };
        self.apply(vec![super::node_edits::duplicate_edit(&node)], follow, cx);
    }

    /// Edit > Delete: remove the object; Undo brings it back.
    pub(super) fn delete_node(&mut self, cx: &mut Context<Self>) {
        let Some((node, _)) = self.selected_object(cx) else {
            return;
        };
        self.session.update(cx, |session, _| {
            session.selected_project_node = None;
            session.project_inspector = json!({"message":"Select a project node"});
        });
        self.apply(
            vec![super::node_edits::delete_edit(&node)],
            Follow::Nothing,
            cx,
        );
    }

    pub(super) fn confirm_delete_hierarchy_node(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((node, object)) = self.selected_object(cx) else {
            return;
        };
        let label = node_label(&self.session.read(cx).project_tree, &node).unwrap_or(object);
        let workspace = cx.entity();
        prompt::confirm(
            window,
            cx,
            format!("Delete “{label}”?"),
            "Undo restores it.",
            "Delete",
            move |_, cx| {
                workspace.update(cx, |this, cx| {
                    this.delete_node(cx);
                });
            },
        );
    }

    pub(super) fn make_object_definition(&mut self, cx: &mut Context<Self>) {
        let node = {
            let session = self.session.read(cx);
            let Some(node) = session.selected_project_node.clone() else {
                return;
            };
            if object_kind(&session.project_tree, &node).as_deref() != Some("group") {
                return;
            }
            node
        };
        let parent =
            super::node_edits::duplicate_parent(&self.session.read(cx).project_tree, &node)
                .or_else(|| {
                    self.session.read(cx).project_tree["nodes"][0]["id"]
                        .as_str()
                        .map(str::to_owned)
                });
        let Some(parent) = parent else {
            return;
        };
        self.apply(
            vec![super::node_edits::make_object_definition_edit(&node)],
            Follow::RevealFirstCreated { parent },
            cx,
        );
    }

    pub(super) fn unpack_object_instance(&mut self, cx: &mut Context<Self>) {
        let (node, parent) = {
            let session = self.session.read(cx);
            let Some(node) = session.selected_project_node.clone() else {
                return;
            };
            if !super::node_edits::can_unpack_instance(
                &object_kind(&session.project_tree, &node).unwrap_or_default(),
                !session.document().is_read_only(),
            ) {
                return;
            }
            let parent =
                super::node_edits::duplicate_parent(&session.project_tree, &node).or_else(|| {
                    session.project_tree["nodes"][0]["id"]
                        .as_str()
                        .map(str::to_owned)
                });
            let Some(parent) = parent else {
                return;
            };
            (node, parent)
        };
        self.apply(
            vec![super::node_edits::unpack_object_instance_edit(&node)],
            Follow::RevealFirstCreated { parent },
            cx,
        );
    }

    /// Create menu / Built-in double-click: add a preset where says,
    /// then select and reveal it. Object Instance uses the object definition
    /// selected in Contents.
    pub(super) fn create_preset(&mut self, preset: uuid::Uuid, cx: &mut Context<Self>) {
        let (target, definition) = {
            let session = self.session.read(cx);
            let target = create_menu::target(
                &session.project_tree,
                session.selected_project_node.as_deref(),
            );
            let definition = session
                .contents
                .selected()
                .filter(|entry| entry["resource_kind"] == "object_definition")
                .and_then(|entry| entry["id"].as_str().map(str::to_owned));
            (target, definition)
        };
        let Some(target) = target else {
            self.set_status("Wait for the Hierarchy to load".into(), cx);
            return;
        };
        let definition = if preset == OBJECT_INSTANCE_PRESET {
            match definition {
                Some(definition) => Some(definition),
                None => {
                    self.set_status(
                        "Select an object definition in Contents, then create its instance".into(),
                        cx,
                    );
                    return;
                }
            }
        } else {
            None
        };
        let parent = target.parent.clone();
        // Every placeable preset takes the viewport's current
        // view — the camera poses with it, and a shape, light or area
        // body lands at its look-at point, where the user is looking.
        let session = self.session.read(cx);
        let up = session
            .project
            .active_up
            .unwrap_or(protocol::project::UpAxis::PosY)
            .vector();
        let view = create_menu::camera_view(preset, session.view_camera.as_ref(), up);
        self.apply(
            vec![json!({
                "kind":"create_preset","preset":preset,"parent":target.parent,
                "after":target.after,"definition":definition,"view":view,
            })],
            Follow::Reveal { parent },
            cx,
        );
    }

    pub(super) fn create_definition_preset(
        &mut self,
        preset: uuid::Uuid,
        parent: uuid::Uuid,
        after: Option<uuid::Uuid>,
        cx: &mut Context<Self>,
    ) {
        let view = {
            let session = self.session.read(cx);
            let up = session
                .project
                .active_up
                .unwrap_or(protocol::project::UpAxis::PosY)
                .vector();
            super::create_menu::camera_view(preset, session.view_camera.as_ref(), up)
                .unwrap_or(Value::Null)
        };
        self.apply(
            vec![super::create_menu::definition_preset_edit(
                preset, parent, after, view,
            )],
            Follow::Reveal {
                parent: parent.to_string(),
            },
            cx,
        );
    }
}

fn node_label(tree: &Value, id: &str) -> Option<String> {
    fn find(nodes: &[Value], id: &str) -> Option<String> {
        for node in nodes {
            if node["id"].as_str() == Some(id) {
                return node["label"].as_str().map(str::to_owned);
            }
            if let Some(found) = node["children"]
                .as_array()
                .and_then(|children| find(children, id))
            {
                return Some(found);
            }
        }
        None
    }
    find(tree["nodes"].as_array()?, id)
}

/// The one destructive confirmation shared by Contents and resource
/// cards. Its where-used detail is the same cached source text Contents
/// has historically shown.
pub(super) fn confirm_item_delete(
    session: Entity<Session>,
    entry: Value,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(Value, &mut Window, &mut App) + 'static,
) {
    let Some(edit) = delete_edit(&entry) else {
        return;
    };
    let name = entry["name"].as_str().unwrap_or_default().to_owned();
    let used = entry["reference_count"].as_u64().unwrap_or(0);
    let description = if used > 0 {
        let sources = session
            .update(cx, |session, cx| {
                session.item_references(
                    entry["kind"].as_str().unwrap_or_default(),
                    entry["id"].as_str().unwrap_or_default(),
                    cx,
                )
            })
            .and_then(|list| list["references"].as_array().cloned())
            .map(|rows| {
                rows.iter()
                    .take(3)
                    .map(super::panels::inspector::reference_label)
                    .collect::<Vec<_>>()
                    .join(", ")
            })
            .unwrap_or_default();
        if sources.is_empty() {
            format!("It is used in {used} places, so the project will refuse to delete it.")
        } else {
            format!(
                "It is used in {used} places — {sources} — so the project will refuse to delete it."
            )
        }
    } else {
        "Undo restores it.".to_owned()
    };
    prompt::confirm(
        window,
        cx,
        format!("Delete “{name}”?"),
        description,
        "Delete",
        move |window, cx| on_confirm(edit.clone(), window, cx),
    );
}

/// The scene-graph `object` of a loaded Hierarchy node.
pub(super) fn object_kind(tree: &Value, id: &str) -> Option<String> {
    fn walk(nodes: &[Value], id: &str) -> Option<String> {
        nodes.iter().find_map(|node| {
            if node["id"].as_str() == Some(id) {
                return node["object"].as_str().map(str::to_owned);
            }
            walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                id,
            )
        })
    }
    walk(
        tree["nodes"].as_array().map(Vec::as_slice).unwrap_or(&[]),
        id,
    )
}

#[cfg(test)]
mod align_target_tests {
    use super::{align_camera_target, definition_instances_for_node};
    use serde_json::json;

    #[test]
    fn alignment_prefers_a_selected_camera_then_falls_back_to_render_camera() {
        let tree = json!({"nodes":[{"id":"scene","object":"scene","children":[
            {"id":"render","object":"camera","render_camera":true,"children":[]},
            {"id":"other","object":"camera","render_camera":false,"children":[]},
            {"id":"shape","object":"shape","children":[]}
        ]}]});
        assert_eq!(
            align_camera_target(&tree, Some("other")).as_deref(),
            Some("other")
        );
        assert_eq!(
            align_camera_target(&tree, Some("shape")).as_deref(),
            Some("render")
        );
        assert_eq!(align_camera_target(&tree, None).as_deref(), Some("render"));
        assert_eq!(align_camera_target(&json!({"nodes":[]}), None), None);
    }

    #[test]
    fn definition_nodes_report_the_shared_instance_count() {
        let inspector = json!({"node":{"node_id":"shape"},"definition_instance_count":3});
        assert_eq!(definition_instances_for_node(&inspector), Some(3));
        assert_eq!(
            definition_instances_for_node(&json!({"node":{"node_id":"scene"}})),
            None
        );
    }
}
