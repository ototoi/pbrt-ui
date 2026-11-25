//! The main toolbar: project file and history actions. Rendering lives in the
//! Render panel, the Render menu and its keys.
use super::Workspace;
use super::menu::{OpenPbrtScene, OpenProject, RedoEdit, SaveProject, UndoEdit};
use gpui_kit::component::button::DropdownButton;
use gpui_kit::component::{Disableable, Sizable, button::Button};
use gpui_kit::*;

impl Workspace {
    pub(super) fn toolbar(&self, cx: &mut Context<Self>) -> AnyElement {
        let session = self.session.read(cx);
        let document = session.document();
        let empty = matches!(document, super::document::DocumentState::Empty);
        let read_only = document.is_read_only();
        let can_undo = !read_only && session.project.can_undo;
        let can_redo = !read_only && session.project.can_redo;
        let save_hint = if read_only {
            "Save as Project"
        } else {
            "Save project"
        };
        div()
            .flex()
            .items_center()
            .gap_2()
            .px_3()
            .h(px(40.))
            .bg(rgb(0x303238))
            .child(div().font_weight(FontWeight::BOLD).child("pbrt-ui"))
            .child(
                DropdownButton::new("open")
                    .button(
                        Button::new("open-project")
                            .icon(assets::IconName::FolderOpen)
                            .tooltip(if cfg!(target_os = "macos") {
                                "Open project folder (Cmd+O)"
                            } else {
                                "Open project folder (Ctrl+O)"
                            })
                            .accessibility_label("Open project folder")
                            .small()
                            .on_click(|_, window, cx| {
                                window.dispatch_action(Box::new(OpenProject), cx);
                            }),
                    )
                    .dropdown_menu(|menu, _, _| {
                        menu.menu("Open Project…", Box::new(OpenProject))
                            .menu("Open PBRT Scene…", Box::new(OpenPbrtScene))
                    })
                    .menu_accessibility_label("Open another project or PBRT scene"),
            )
            .child(
                Button::new("project-save")
                    .icon(assets::IconName::Save)
                    .small()
                    .disabled(empty)
                    .tooltip(save_hint)
                    .accessibility_label(save_hint)
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(SaveProject), cx);
                    }),
            )
            .child(div().w(px(1.)).h(px(20.)).bg(rgb(0x4a4d55)))
            .child(
                Button::new("project-undo")
                    .icon(assets::IconName::Undo2)
                    .small()
                    .disabled(!can_undo)
                    .tooltip("Undo project edit")
                    .accessibility_label("Undo project edit")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(UndoEdit), cx);
                    }),
            )
            .child(
                Button::new("project-redo")
                    .icon(assets::IconName::Redo2)
                    .small()
                    .disabled(!can_redo)
                    .tooltip("Redo project edit")
                    .accessibility_label("Redo project edit")
                    .on_click(|_, window, cx| {
                        window.dispatch_action(Box::new(RedoEdit), cx);
                    }),
            )
            .into_any_element()
    }
}
