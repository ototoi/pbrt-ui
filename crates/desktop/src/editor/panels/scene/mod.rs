//! The Scene panel: the native project viewport.
mod coordinates;
mod gizmo;
mod navigation;
mod project;
use crate::editor::EditorPanel;
pub(crate) use gizmo::{GizmoDrag, LightDrag, Space};
use gpui_kit::*;
pub(in crate::editor) use navigation::CameraDrag;
pub(in crate::editor) use project::{FlyClock, FlyKeys};
impl EditorPanel {
    pub(super) fn scene(&self, cx: &mut Context<Self>) -> AnyElement {
        self.project_scene(cx)
    }
}
