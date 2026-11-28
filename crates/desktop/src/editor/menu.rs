//! Menu bar definition, action enablement and keyboard shortcuts.
//!
//! `MenuState` is the single place that decides which actions are available;
//! menus, toolbar buttons and keys all dispatch the same actions.
use super::document::DocumentState;
use super::{FrameAll, FrameSelected};
use super::{PanelKind, editor_panels::EditorKind};
use gpui_kit::component::GlobalState;
use gpui_kit::component::menu::AppMenuBar;
use gpui_kit::*;
use scene_view::PreviewMode;

actions!(
    pbrt_editor,
    [
        OpenProject,
        OpenPbrtScene,
        NewProject,
        AddSceneToProject,
        ReloadSource,
        SaveProject,
        SaveProjectAs,
        ExportPbrt,
        CloseDocument,
        QuitApp,
        UndoEdit,
        RedoEdit,
        Deselect,
        RenderSceneCamera,
        OpenRenderSettings,
        RenderCurrentView,
        ToggleGpu,
        CancelRender,
        ResetPanelLayout,
        ShowAbout,
        ShowControlEndpoint,
        SolidMode,
        NormalMode,
        WireframeMode,
        LitMode,
        UnsavedSave,
        UnsavedDiscard,
        UnsavedCancel,
        NewScene,
        DuplicateScene,
        CloseScene,
        ContentsOpen,
        ContentsNewFolder,
        ContentsImportResource,
        ContentsRename,
        ContentsMove,
        ContentsDelete,
        DuplicateNode,
        DeleteNode,
        MakeObjectDefinition,
        UnpackObjectInstance,
        RenameHierarchyNode,
        AlignCameraToView,
        AlignViewToSceneCamera,
        AlignViewToSelected,
        /// Shift+↑ / Shift+↓ in a spin box: a step of 10. Separate actions,
        /// because the window's modifier state can lag a fresh focus.
        StepUpTen,
        StepDownTen,
    ]
);

/// Window > panel: show the panel, or close it when it is in the dock.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct TogglePanel {
    pub item: WindowItemKey,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WindowItemKey {
    Fixed(PanelKind),
    Resource(EditorKind),
}

/// File > Open Recent > entry (index into the recent list).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct OpenRecent {
    pub index: usize,
}

/// File > Open Scene > scene, by its UUID.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct OpenSceneId {
    pub scene: uuid::Uuid,
}

/// Create > preset, by its UUID. Labels are display only.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct CreatePreset {
    pub preset: uuid::Uuid,
}

/// Hierarchy ▸ Object Definitions: create directly in one definition scope.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct CreateDefinitionPreset {
    pub preset: uuid::Uuid,
    pub parent: uuid::Uuid,
    pub after: Option<uuid::Uuid>,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct ConfirmDeleteHierarchyNode;

/// Contents ▸ New: create one resource of this
/// implementation; textures carry their value type.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct CreateResource {
    pub kind: String,
    pub implementation: String,
    pub texture_value_type: Option<String>,
}

/// Inspector link: select the item in Contents and reveal its folder.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct RevealItem {
    pub id: String,
}

#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct RevealDefinition {
    pub id: String,
}

/// A node editor selection asks for the Inspector: open the panel when
/// it is closed (never close it) so the node's page has somewhere to
/// show.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct ShowInspector;

/// Reveal a where-used scene node in the
/// Hierarchy, opening its scene first when it is not the active one
/// (2026-10-29 ruling: the switch is immediate, no dialog).
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct RevealNode {
    pub scene: String,
    pub node_id: String,
    pub path: Vec<String>,
}

/// Open a Contents item in its dedicated editor tab.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct OpenInEditor {
    pub id: String,
}

/// Open the array parameter's full-page editor in its own Dock tab.
#[derive(Clone, PartialEq, Action)]
#[action(namespace = pbrt_editor, no_json)]
pub struct OpenArrayEditor {
    pub target: serde_json::Value,
    pub name: String,
    pub table: super::parameter_values::Table,
}

/// One File > Open Recent row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentItem {
    pub label: String,
    pub missing: bool,
}

/// One File > Open Scene row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SceneItem {
    pub id: uuid::Uuid,
    pub label: String,
    pub active: bool,
}

/// One Create menu row from `project.list_presets`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetItem {
    pub id: uuid::Uuid,
    pub label: String,
    pub category: String,
}

/// Everything the menu bar needs to enable, check and label its items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuState {
    pub empty: bool,
    pub viewer: bool,
    pub has_scene: bool,
    pub can_undo: bool,
    pub can_redo: bool,
    pub gpu: bool,
    pub render_starting: bool,
    pub rendering: bool,
    pub mode: PreviewMode,
    /// Stable identities drive Window toggles; labels are presentation only.
    pub panels: Vec<WindowMenuItem>,
    pub recent: Vec<RecentItem>,
    pub scenes: Vec<SceneItem>,
    pub presets: Vec<PresetItem>,
    /// A Hierarchy object that Duplicate / Delete can act on is selected.
    pub node_selected: bool,
    /// A scene node holds the single selection: Deselect can
    /// retire it. Broader than `node_selected`, which asks whether the
    /// object's kind takes edits.
    pub selected_project_node: bool,
    pub selected_camera: bool,
    pub render_camera_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WindowMenuItem {
    pub key: WindowItemKey,
    pub label: String,
    pub open: bool,
}

impl MenuState {
    pub fn from_parts(
        document: &DocumentState,
        can_undo: bool,
        can_redo: bool,
        gpu: bool,
        render_starting: bool,
        jobs: &serde_json::Value,
        mode: PreviewMode,
    ) -> Self {
        Self {
            empty: matches!(document, DocumentState::Empty),
            viewer: document.is_read_only(),
            has_scene: !matches!(document, DocumentState::Empty),
            can_undo,
            can_redo,
            gpu,
            render_starting,
            rendering: rendering(jobs),
            mode,
            panels: Vec::new(),
            recent: Vec::new(),
            scenes: Vec::new(),
            presets: Vec::new(),
            node_selected: false,
            selected_project_node: false,
            selected_camera: false,
            render_camera_available: false,
        }
    }

    /// Edit > Duplicate / Delete act on the selected Hierarchy object.
    pub fn node_edit(&self) -> bool {
        self.edit() && self.node_selected
    }

    /// Edit > Deselect retires the selected scene node. Offered
    /// whenever a node holds the single selection — Inspector, Hierarchy
    /// row and gizmo all follow it — and greyed with nothing selected.
    pub fn deselect(&self) -> bool {
        self.selected_project_node
    }

    /// New / Duplicate Scene and the Create menu edit the project.
    pub fn edit(&self) -> bool {
        !self.empty && !self.viewer
    }

    pub fn save(&self) -> bool {
        !self.empty
    }

    pub fn save_as(&self) -> bool {
        !self.empty
    }

    pub fn export(&self) -> bool {
        !self.empty && self.has_scene
    }

    pub fn close(&self) -> bool {
        !self.empty
    }

    pub fn reload_source(&self) -> bool {
        self.viewer
    }

    pub fn undo(&self) -> bool {
        self.can_undo
    }

    pub fn redo(&self) -> bool {
        self.can_redo
    }

    pub fn document_action(&self) -> bool {
        !self.empty && self.has_scene
    }

    pub fn cancel_render(&self) -> bool {
        self.rendering
    }

    pub fn save_label(&self) -> &'static str {
        if self.viewer {
            "Save as Project…"
        } else {
            "Save"
        }
    }

    pub fn save_as_label(&self) -> &'static str {
        if self.viewer {
            "Save as Project…"
        } else {
            "Save As…"
        }
    }
}

fn rendering(jobs: &serde_json::Value) -> bool {
    jobs["jobs"].as_array().into_iter().flatten().any(|job| {
        matches!(
            job["state"].as_str(),
            Some("queued" | "running" | "cancelling")
        ) && matches!(job["kind"].as_str(), Some("render" | "ipr"))
    })
}

/// Build the application menu. Labels follow the document mode; disabled items
/// stay visible so the menu shows what the document can do.
pub fn build_menus(state: &MenuState) -> Vec<Menu> {
    let modes = Menu::new("Display Mode").items([
        MenuItem::action("Solid", SolidMode).checked(state.mode == PreviewMode::Solid),
        MenuItem::action("Normal", NormalMode).checked(state.mode == PreviewMode::Normal),
        MenuItem::action("Wireframe", WireframeMode).checked(state.mode == PreviewMode::Wireframe),
        MenuItem::action("Lit", LitMode).checked(state.mode == PreviewMode::Lit),
    ]);
    let recent = Menu::new("Open Recent").items(if state.recent.is_empty() {
        vec![MenuItem::action("No Recent Items", OpenRecent { index: usize::MAX }).disabled(true)]
    } else {
        state
            .recent
            .iter()
            .enumerate()
            .map(|(index, item)| {
                MenuItem::action(item.label.clone(), OpenRecent { index }).disabled(item.missing)
            })
            .collect()
    });
    let scenes = Menu::new("Open Scene").items(if state.scenes.is_empty() {
        vec![MenuItem::action("No Scenes", NewScene).disabled(true)]
    } else {
        state
            .scenes
            .iter()
            .map(|scene| {
                MenuItem::action(scene.label.clone(), OpenSceneId { scene: scene.id })
                    .checked(scene.active)
                    .disabled(state.empty)
            })
            .collect()
    });
    let mut file = vec![
        MenuItem::action("New Project…", NewProject),
        MenuItem::action("Open Project…", OpenProject),
        MenuItem::action("Open PBRT Scene…", OpenPbrtScene),
        MenuItem::action("Add Scene to Project…", AddSceneToProject).disabled(!state.edit()),
        MenuItem::submenu(recent),
        MenuItem::separator(),
        MenuItem::submenu(scenes),
        MenuItem::action("New Scene", NewScene).disabled(!state.edit()),
        MenuItem::action("Duplicate Scene", DuplicateScene)
            .disabled(!state.edit() || !state.has_scene),
        MenuItem::action("Close Scene", CloseScene).disabled(!state.has_scene),
        MenuItem::separator(),
        MenuItem::action("Reload Source", ReloadSource).disabled(!state.reload_source()),
        MenuItem::separator(),
        MenuItem::action(state.save_label(), SaveProject).disabled(!state.save()),
    ];
    // A Viewer saves and saves-as through the same destination picker, so
    // listing both would show the same item twice.
    if state.save_as_label() != state.save_label() {
        file.push(
            MenuItem::action(state.save_as_label(), SaveProjectAs).disabled(!state.save_as()),
        );
    }
    file.extend([
        MenuItem::action("Export as PBRT…", ExportPbrt).disabled(!state.export()),
        MenuItem::separator(),
        MenuItem::action("Close", CloseDocument).disabled(!state.close()),
        MenuItem::action("Quit", QuitApp),
    ]);
    vec![
        Menu::new("File").items(file),
        Menu::new("Edit").items([
            MenuItem::action("Undo", UndoEdit).disabled(!state.undo()),
            MenuItem::action("Redo", RedoEdit).disabled(!state.redo()),
            MenuItem::separator(),
            MenuItem::action("Deselect", Deselect).disabled(!state.deselect()),
            MenuItem::separator(),
            MenuItem::action("Duplicate", DuplicateNode).disabled(!state.node_edit()),
            MenuItem::action("Delete", DeleteNode).disabled(!state.node_edit()),
        ]),
        Menu::new("View").items([
            MenuItem::action("Align View to Scene Camera", AlignViewToSceneCamera)
                .disabled(!state.document_action()),
            MenuItem::action("Align View to Selected", AlignViewToSelected)
                .disabled(!state.document_action()),
            MenuItem::action("Focus Selected", FrameSelected).disabled(!state.document_action()),
            MenuItem::action("Frame All", FrameAll).disabled(!state.document_action()),
            MenuItem::separator(),
            MenuItem::submenu(modes),
            MenuItem::separator(),
            MenuItem::action("Align Camera to View", AlignCameraToView).disabled(
                !state.edit() || !(state.selected_camera || state.render_camera_available),
            ),
        ]),
        Menu::new("Render").items([
            MenuItem::action("Render Settings", OpenRenderSettings)
                .disabled(!state.document_action()),
            MenuItem::separator(),
            MenuItem::action(
                if state.render_starting {
                    "Starting…"
                } else {
                    "Render Scene Camera"
                },
                RenderSceneCamera,
            )
            .disabled(!state.document_action() || state.render_starting),
            MenuItem::action(
                if state.render_starting {
                    "Starting…"
                } else {
                    "Render View"
                },
                RenderCurrentView,
            )
            .disabled(!state.document_action() || state.render_starting),
            MenuItem::action("GPU", ToggleGpu)
                .checked(state.gpu)
                .disabled(!state.document_action()),
            MenuItem::action("Cancel Render", CancelRender).disabled(!state.cancel_render()),
        ]),
        Menu::new("Create").items(create_items(state)),
        Menu::new("Window").items(
            state
                .panels
                .iter()
                .map(|item| {
                    MenuItem::action(
                        item.label.clone(),
                        TogglePanel {
                            item: item.key.clone(),
                        },
                    )
                    .checked(item.open)
                })
                .chain([
                    MenuItem::separator(),
                    MenuItem::action("Reset Layout", ResetPanelLayout),
                ]),
        ),
        Menu::new("Help").items([
            MenuItem::action("About pbrt-ui", ShowAbout),
            MenuItem::action("Show Control Endpoint", ShowControlEndpoint),
        ]),
    ]
}

/// Create menu: presets laid out by `create_menu::layout`, in
/// `project.list_presets` order. Every item dispatches the preset's UUID.
fn create_items(state: &MenuState) -> Vec<MenuItem> {
    use super::create_menu::{CreateEntry, layout, with_breaks};
    if state.presets.is_empty() {
        return vec![MenuItem::action("Empty Group", NewScene).disabled(true)];
    }
    let disabled = !state.edit() || !state.has_scene;
    let action = |preset: &PresetItem| {
        MenuItem::action(preset.label.clone(), CreatePreset { preset: preset.id })
            .disabled(disabled)
    };
    layout(&state.presets)
        .into_iter()
        .map(|entry| match entry {
            CreateEntry::Item(preset) => action(preset),
            CreateEntry::Submenu(title, items) => {
                let mut rows = Vec::new();
                for (separator, preset) in with_breaks(&items) {
                    if separator {
                        rows.push(MenuItem::separator());
                    }
                    rows.push(action(preset));
                }
                MenuItem::submenu(Menu::new(title).items(rows))
            }
        })
        .collect()
}

/// Install the menus for both the native (macOS) menu and the in-window bar.
pub fn apply(cx: &mut App, state: &MenuState, menu_bar: &Entity<AppMenuBar>) {
    let menus = build_menus(state);
    let owned = build_menus(state)
        .into_iter()
        .map(|menu| menu.owned())
        .collect();
    cx.set_menus(menus);
    GlobalState::global_mut(cx).set_app_menus(owned);
    menu_bar.update(cx, |bar, cx| bar.reload(cx));
}

/// The built-in Group preset (Create > Empty Group, Mod+Shift+N).
pub const EMPTY_GROUP_PRESET: uuid::Uuid = uuid::uuid!("19500901-220e-4577-a08a-6fa4173fa347");

/// Bind the editor's shortcuts. Menu items, toolbar buttons, and keys all
/// dispatch the same actions.
fn redo_shortcuts(modifier: &str) -> [String; 2] {
    [format!("{modifier}-shift-z"), format!("{modifier}-y")]
}

pub fn bind_keys(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };
    cx.bind_keys([
        KeyBinding::new(
            &format!("{modifier}-o"),
            OpenProject,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            // The Group preset keeps Mod+Shift+N, so New Project
            // takes the alt form.
            &format!("{modifier}-alt-n"),
            NewProject,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-shift-o"),
            OpenPbrtScene,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-r"),
            ReloadSource,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-s"),
            SaveProject,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-shift-s"),
            SaveProjectAs,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-e"),
            ExportPbrt,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-w"),
            CloseDocument,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(&format!("{modifier}-q"), QuitApp, Some("EditorWorkspace")),
        KeyBinding::new(&format!("{modifier}-z"), UndoEdit, Some("EditorWorkspace")),
        KeyBinding::new(
            &redo_shortcuts(modifier)[0],
            RedoEdit,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &redo_shortcuts(modifier)[1],
            RedoEdit,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-enter"),
            RenderSceneCamera,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-shift-enter"),
            RenderCurrentView,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-shift-f"),
            AlignCameraToView,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-."),
            CancelRender,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-shift-n"),
            CreatePreset {
                preset: EMPTY_GROUP_PRESET,
            },
            Some("EditorWorkspace"),
        ),
        KeyBinding::new(
            &format!("{modifier}-d"),
            DuplicateNode,
            Some("EditorWorkspace"),
        ),
        KeyBinding::new("delete", DeleteNode, Some("HierarchyTree")),
        KeyBinding::new("f2", RenameHierarchyNode, Some("HierarchyTree")),
        // Esc clears the selection in the Hierarchy. The Scene
        // panel keeps its own escape binding: there a gizmo drag
        // counts as the cancellation to win.
        KeyBinding::new("escape", Deselect, Some("HierarchyTree")),
        KeyBinding::new("f", FrameSelected, Some("SceneViewport")),
        // Shift steps a spin box by 10 (Shift-click reads the modifier); the
        // plain arrow bindings come with gpui-kit.
        KeyBinding::new("shift-up", StepUpTen, Some("NumberInput")),
        KeyBinding::new("shift-down", StepDownTen, Some("NumberInput")),
        KeyBinding::new("shift-f", FrameAll, Some("SceneViewport")),
        KeyBinding::new("escape", super::CancelGizmo, Some("SceneViewport")),
        KeyBinding::new("w", super::MoveTool, Some("SceneViewport")),
        KeyBinding::new("e", super::RotateTool, Some("SceneViewport")),
        KeyBinding::new("r", super::ScaleTool, Some("SceneViewport")),
        KeyBinding::new("q", super::NoTool, Some("SceneViewport")),
        KeyBinding::new("x", super::ToggleGizmoSpace, Some("SceneViewport")),
        KeyBinding::new("z", super::ToggleGizmoPivot, Some("SceneViewport")),
    ]);
}

#[cfg(test)]
mod tests {
    use super::{MenuState, build_menus};
    use crate::editor::document::DocumentState;
    use gpui_kit::MenuItem;
    use protocol::ids::Revision;
    use protocol::project::ProjectRef;
    use scene_view::PreviewMode;
    use std::path::PathBuf;

    #[test]
    fn redo_shortcut_table_keeps_shift_z_and_adds_y() {
        assert_eq!(super::redo_shortcuts("ctrl"), ["ctrl-shift-z", "ctrl-y"]);
        assert_eq!(super::redo_shortcuts("cmd"), ["cmd-shift-z", "cmd-y"]);
    }

    fn project() -> ProjectRef {
        ProjectRef {
            project_id: uuid::Uuid::from_u128(7),
            revision: Revision::from_u64(11),
            activation: Revision::from_u64(3),
        }
    }

    fn state(document: &DocumentState) -> MenuState {
        MenuState::from_parts(
            document,
            false,
            false,
            false,
            false,
            &serde_json::json!({"jobs": []}),
            PreviewMode::Solid,
        )
    }

    fn viewer() -> DocumentState {
        DocumentState::Viewer {
            project: project(),
            title: "cornell-box".into(),
            source_path: PathBuf::from("/tmp/cornell-box.pbrt"),
        }
    }

    fn project_document(dirty: bool) -> DocumentState {
        DocumentState::Project {
            project: project(),
            title: "MyRoom".into(),
            path: PathBuf::from("/tmp/MyRoom"),
            dirty,
        }
    }

    #[test]
    fn empty_document_disables_document_actions() {
        let empty = state(&DocumentState::Empty);
        assert!(!empty.save());
        assert!(!empty.save_as());
        assert!(!empty.export());
        assert!(!empty.close());
        assert!(!empty.reload_source());
        assert!(!empty.document_action());
    }

    #[test]
    fn viewer_can_reload_and_saves_as_a_project() {
        let viewer = state(&viewer());
        assert!(viewer.reload_source());
        assert!(viewer.save());
        assert_eq!(viewer.save_label(), "Save as Project…");
        assert_eq!(viewer.save_as_label(), "Save as Project…");
        assert!(!viewer.undo());
        assert!(!viewer.redo());
    }

    #[test]
    fn project_saves_in_place_and_cannot_reload() {
        let project = state(&project_document(false));
        assert!(!project.reload_source());
        assert_eq!(project.save_label(), "Save");
        assert_eq!(project.save_as_label(), "Save As…");
        assert!(project.close());
    }

    /// The File menu adds scenes only to editable projects; a Viewer or
    /// an empty workspace keeps the entry disabled, and it never replaces
    /// the current document like Open PBRT Scene does.
    #[test]
    fn add_scene_to_project_follows_editability() {
        fn entry(state: &MenuState) -> bool {
            let file = build_menus(state)
                .into_iter()
                .find(|menu| menu.name == "File")
                .expect("a File menu");
            let item = file.items.iter().find(
                |item| matches!(item, MenuItem::Action { name, .. } if name == "Add Scene to Project…"),
            );
            match item {
                Some(MenuItem::Action { disabled, .. }) => !disabled,
                _ => panic!("the File menu is missing the Add Scene entry"),
            }
        }

        assert!(!entry(&state(&DocumentState::Empty)));
        assert!(!entry(&state(&viewer())));
        assert!(entry(&state(&project_document(true))));
        // A scene-less editable project can still accept a scene.
        assert!(entry(&state(&project_document(false))));
    }

    #[test]
    fn undo_and_redo_follow_the_project_flags() {
        let mut flags = MenuState::from_parts(
            &project_document(true),
            true,
            false,
            false,
            false,
            &serde_json::json!({"jobs": []}),
            PreviewMode::Solid,
        );
        assert!(flags.undo());
        assert!(!flags.redo());
        flags.can_undo = false;
        flags.can_redo = true;
        assert!(!flags.undo());
        assert!(flags.redo());
    }

    /// Edit ▸ Deselect follows the node selection — offered when a
    /// scene node holds it, greyed with nothing selected.
    #[test]
    fn edit_menu_deselect_follows_the_selection() {
        fn enabled(state: &MenuState) -> bool {
            let edit = build_menus(state)
                .into_iter()
                .find(|menu| menu.name == "Edit")
                .expect("an Edit menu");
            let item = edit
                .items
                .iter()
                .find(|item| matches!(item, MenuItem::Action { name, .. } if name == "Deselect"));
            match item {
                Some(MenuItem::Action { disabled, .. }) => !disabled,
                _ => panic!("the Edit menu is missing the Deselect entry"),
            }
        }

        let mut selected = state(&project_document(false));
        assert!(!selected.deselect());
        assert!(!enabled(&selected));
        selected.selected_project_node = true;
        assert!(selected.deselect());
        assert!(enabled(&selected));
    }

    #[test]
    fn cancel_render_needs_a_running_render_job() {
        let jobs = serde_json::json!({"jobs": [
            {"kind": "render", "state": "running", "phase": "rendering"}
        ]});
        let state = MenuState::from_parts(
            &project_document(false),
            false,
            false,
            false,
            false,
            &jobs,
            PreviewMode::Solid,
        );
        assert!(state.rendering);
        assert!(state.cancel_render());
        let idle = MenuState::from_parts(
            &project_document(false),
            false,
            false,
            false,
            false,
            &serde_json::json!({"jobs": [{"kind": "render", "state": "succeeded"}]}),
            PreviewMode::Solid,
        );
        assert!(!idle.cancel_render());
    }

    #[test]
    fn a_viewer_lists_the_save_action_once() {
        let menus = build_menus(&state(&viewer()));
        let file = menus.iter().find(|menu| menu.name == "File").unwrap();
        let saves = file
            .items
            .iter()
            .filter(
                |item| matches!(item, MenuItem::Action { name, .. } if name.starts_with("Save")),
            )
            .count();
        assert_eq!(saves, 1);

        let menus = build_menus(&state(&project_document(false)));
        let file = menus.iter().find(|menu| menu.name == "File").unwrap();
        let saves = file
            .items
            .iter()
            .filter(
                |item| matches!(item, MenuItem::Action { name, .. } if name.starts_with("Save")),
            )
            .count();
        assert_eq!(saves, 2);
    }

    #[test]
    fn window_item_keys_keep_fixed_resource_and_array_targets_distinct() {
        use super::WindowItemKey;
        use crate::editor::{PanelKind, editor_panels::EditorKind};

        assert_ne!(
            WindowItemKey::Fixed(PanelKind::Scene),
            WindowItemKey::Resource(EditorKind::MaterialEditor)
        );
        assert_eq!(
            WindowItemKey::Resource(EditorKind::MaterialEditor),
            WindowItemKey::Resource(EditorKind::MaterialEditor)
        );
        assert_ne!(
            WindowItemKey::Resource(EditorKind::ArrayEditor),
            WindowItemKey::Resource(EditorKind::MeshViewer)
        );
    }

    #[test]
    fn window_menu_displays_fixed_and_resource_items_by_label() {
        use super::{WindowItemKey, WindowMenuItem};
        use crate::editor::editor_panels::EditorKind;

        let mut state = state(&project_document(false));
        state.panels = vec![
            WindowMenuItem {
                key: WindowItemKey::Fixed(crate::editor::PanelKind::Scene),
                label: "Scene".into(),
                open: true,
            },
            WindowMenuItem {
                key: WindowItemKey::Resource(EditorKind::MaterialEditor),
                label: "Material Editor".into(),
                open: false,
            },
            WindowMenuItem {
                key: WindowItemKey::Resource(EditorKind::ArrayEditor),
                label: "Array Editor: P".into(),
                open: true,
            },
        ];
        let window = build_menus(&state)
            .into_iter()
            .find(|menu| menu.name == "Window")
            .unwrap();
        let labels = window
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } if name != "Reset Layout" => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(labels, ["Scene", "Material Editor", "Array Editor: P"]);
    }

    #[test]
    fn menus_check_the_current_display_mode() {
        let mut state = state(&project_document(false));
        state.mode = PreviewMode::Wireframe;
        let menus = build_menus(&state);
        let view = menus.iter().find(|menu| menu.name == "View").unwrap();
        let MenuItem::Submenu(modes) = &view.items[5] else {
            panic!("expected the display-mode submenu");
        };
        let checked = modes.items.iter().filter(|item| item.is_checked()).count();
        assert_eq!(checked, 1);
        assert!(matches!(
            &modes.items[2],
            MenuItem::Action { name, .. } if name == "Wireframe"
        ));
    }

    #[test]
    fn align_camera_to_view_uses_the_selected_or_render_camera() {
        let mut state = state(&project_document(false));
        let view = |state: &MenuState| {
            let menu = build_menus(state)
                .into_iter()
                .find(|menu| menu.name == "View")
                .unwrap();
            match menu.items.iter().find(|item| matches!(item, MenuItem::Action { name, .. } if name == "Align Camera to View")).unwrap() {
                MenuItem::Action { disabled, .. } => *disabled,
                _ => unreachable!(),
            }
        };
        assert!(view(&state));
        state.selected_camera = true;
        assert!(!view(&state));
        state.selected_camera = false;
        state.render_camera_available = true;
        assert!(!view(&state));
    }
    #[test]
    fn project_without_scene_keeps_save_and_history_but_disables_scene_actions() {
        let mut flags = state(&project_document(true));
        flags.has_scene = false;
        flags.can_undo = true;
        flags.can_redo = true;
        assert!(flags.save());
        assert!(flags.edit());
        assert!(flags.undo());
        assert!(flags.redo());
        assert!(!flags.export());
        assert!(!flags.document_action());
    }

    /// New Project is offered everywhere — the empty workspace, a
    /// Viewer and an open project — and it asks about unsaved changes
    /// like Open Project does, so nothing disables it.
    #[test]
    fn new_project_is_always_available() {
        fn enabled(state: &MenuState) -> bool {
            let file = build_menus(state)
                .into_iter()
                .find(|menu| menu.name == "File")
                .expect("a File menu");
            let item = file.items.iter().find(
                |item| matches!(item, MenuItem::Action { name, .. } if name == "New Project…"),
            );
            match item {
                Some(MenuItem::Action { disabled, .. }) => !disabled,
                _ => panic!("the File menu is missing the New Project entry"),
            }
        }

        assert!(enabled(&state(&DocumentState::Empty)));
        assert!(enabled(&state(&viewer())));
        assert!(enabled(&state(&project_document(true))));
        assert!(enabled(&state(&project_document(false))));
    }

    /// F frames the selection under the name Focus, with
    /// Shift+F still Frame All.
    #[test]
    fn view_menu_labels_the_focus_commands() {
        let view = build_menus(&state(&project_document(false)))
            .into_iter()
            .find(|menu| menu.name == "View")
            .unwrap();
        let names = view
            .items
            .iter()
            .filter_map(|item| match item {
                MenuItem::Action { name, .. } => Some(name.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            names[..4],
            [
                "Align View to Scene Camera",
                "Align View to Selected",
                "Focus Selected",
                "Frame All"
            ]
        );
    }
}
