//! Dock panel construction and dispatch to focused views.
mod contents;
/// `project_actions` reads the where-used row label from this module.
pub(in crate::editor) mod inspector;
mod jobs;
mod render;
pub(super) mod scene;
use super::{EditorPanel, PanelKind, Session, hierarchy, render_settings};
use crate::backend::Command;
use gpui_kit::component::dock::{BasePanel, Panel, PanelEvent};
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use std::{cell::Cell, rc::Rc};
impl EditorPanel {
    pub(super) fn reveal_definition(&mut self, id: &str, cx: &mut Context<Self>) {
        if let Some(hierarchy) = &self.hierarchy {
            hierarchy.update(cx, |hierarchy, cx| hierarchy.reveal_definition(id, cx));
        }
    }

    pub(in crate::editor) fn new(
        kind: PanelKind,
        session: Entity<Session>,
        layout_path: Option<std::path::PathBuf>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let overlays_path = layout_path
            .as_deref()
            .and_then(std::path::Path::parent)
            .map(|parent| parent.join("viewport-overlays.json"));
        let overlays_visible = overlays_path
            .as_deref()
            .and_then(|path| std::fs::read(path).ok())
            .as_deref()
            .and_then(|bytes| super::layout::overlays_visible(bytes, kind.name()))
            .unwrap_or(true);
        let contents_search_project_key = session.read(cx).project_value().to_string();
        cx.observe(&session, |this, _, cx| {
            let session = this.session.read(cx);
            let project_key = session.project_value().to_string();
            if this.contents_search_more_offset.is_some_and(|offset| {
                session.contents_search["next_offset"].as_u64() != Some(offset as u64)
                    || session.contents_search_settled
                        == Some((session.contents_search_generation, offset))
            }) {
                this.contents_search_more_offset = None;
            }
            if this.contents_search_project_key != project_key {
                this.contents_search_project_key = project_key;
                this.contents_search_generation = this.contents_search_generation.wrapping_add(1);
                this.contents_search_more_offset = None;
            }
            cx.notify();
        })
        .detach();
        let contents_search_input = matches!(kind, PanelKind::Contents)
            .then(|| cx.new(|cx| InputState::new(window, cx).placeholder("Search Assets…")));
        if let Some(input) = &contents_search_input {
            cx.observe(input, |_, _, cx| cx.notify()).detach();
            cx.subscribe(input, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.contents_search_changed(cx),
                InputEvent::PressEnter { .. } => this.contents_search_submit(cx),
                InputEvent::Focus | InputEvent::Blur => {}
            })
            .detach();
        }
        if matches!(kind, PanelKind::Scene) {
            if !overlays_visible {
                session.update(cx, |session, cx| {
                    session.send(crate::backend::Command::Overlays(false), cx)
                });
            }
            cx.observe_window_activation(window, |this, window, cx| {
                if !window.is_window_active() {
                    this.fly_keys.clear();
                    this.fly_clock.reset();
                    this.last_mouse = None;
                    this.navigation_drag = None;
                    cx.notify();
                }
            })
            .detach();
        }
        Self {
            settings_form: matches!(kind, PanelKind::Render).then(|| {
                cx.new(|cx| render_settings::SettingsForm::new(session.clone(), window, cx))
            }),
            transform_forms: Vec::new(),
            animated_transform_form: None,
            parameter_forms: Vec::new(),
            label_input: None,
            value_edit: None,
            hierarchy: matches!(kind, PanelKind::Hierarchy)
                .then(|| cx.new(|cx| hierarchy::Hierarchy::new(session.clone(), window, cx))),
            kind,
            session,
            contents_tree_scroll: UniformListScrollHandle::new(),
            contents_list_scroll: UniformListScrollHandle::new(),
            inspector_scroll: ScrollHandle::new(),
            contents_grid_columns: 1,
            contents_search_input,
            contents_search_generation: 0,
            contents_search_more_offset: None,
            contents_search_project_key,
            focus: cx.focus_handle(),
            viewport: Rc::new(Cell::new(Bounds::default())),
            viewport_requested: Cell::new([0, 0]),
            last_mouse: None,
            navigation_drag: None,
            fly_keys: scene::FlyKeys::default(),
            fly_clock: scene::FlyClock::default(),
            gizmo_hover: None,
            gizmo_drag: None,
            light_hover: None,
            light_drag: None,
            gizmo_tool: scene_view::preview::gizmo::Tool::Move,
            gizmo_space: scene::Space::Local,
            gizmo_center: false,
            overlays_visible,
            overlays_path: matches!(kind, PanelKind::Scene)
                .then_some(overlays_path)
                .flatten(),
            job_times: Default::default(),
        }
    }
}
impl BasePanel for EditorPanel {
    fn panel_name(&self) -> &'static str {
        self.kind.name()
    }
    /// Closed panels come back from the Window menu.
    fn closable(&self, _: &App) -> bool {
        true
    }
}
impl Panel for EditorPanel {
    fn title(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        self.kind.name()
    }
}
impl EventEmitter<PanelEvent> for EditorPanel {}
impl Focusable for EditorPanel {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for EditorPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if matches!(self.kind, PanelKind::Scene) && self.fly_keys.is_down() {
            // Not `is_window_active`: without a window manager (Xephyr) the
            // window never reports active and the held state would vanish.
            if !self.focus.is_focused(window) {
                self.fly_keys.clear();
                self.fly_clock.reset();
            } else {
                let now = std::time::Instant::now();
                let dt = self.fly_clock.consume(now);
                let mut movement = self.fly_keys.movement();
                if movement.iter().any(|axis| *axis != 0.) {
                    let magnitude = movement.iter().map(|axis| axis * axis).sum::<f64>().sqrt();
                    let speed = if self.fly_keys.faster() { 3. } else { 1. };
                    for axis in &mut movement {
                        *axis = *axis / magnitude * dt * speed;
                    }
                    self.session
                        .update(cx, |session, cx| session.send(Command::Fly(movement), cx));
                }
                window.request_animation_frame();
            }
        }
        if matches!(self.kind, PanelKind::Inspector) {
            self.prepare_inspector(window, cx);
        }
        let content = match self.kind {
            PanelKind::Scene => self.scene(cx),
            PanelKind::Render => self.render_panel(cx),
            PanelKind::Hierarchy => self.hierarchy.as_ref().unwrap().clone().into_any_element(),
            PanelKind::Inspector => self.inspector(cx),
            PanelKind::Contents => self.contents(cx),
            PanelKind::Jobs => self.jobs_panel(cx),
            PanelKind::Console => div()
                .id("console-body")
                .size_full()
                .overflow_y_scroll()
                .p_3()
                .text_xs()
                .children(
                    self.session
                        .read(cx)
                        .messages
                        .iter()
                        .rev()
                        .map(|s| div().py_1().child(s.clone())),
                )
                .into_any_element(),
        };
        div()
            .size_full()
            .track_focus(&self.focus)
            .when(matches!(self.kind, PanelKind::Scene), |panel| {
                panel
                    .key_context("SceneViewport")
                    // Esc while dragging the gizmo puts the selection back;
                    // with no drag in progress the same key retires the
                    // selection instead — the drag cancellation wins.
                    .on_action(cx.listener(|this, _: &super::CancelGizmo, _, cx| {
                        if this.gizmo_cancel(cx) {
                            return;
                        }
                        this.session
                            .update(cx, |session, cx| session.deselect_node(cx));
                    }))
                    .on_action(cx.listener(|this, _: &super::MoveTool, _, cx| {
                        if this.fly_keys.is_down() {
                            cx.propagate();
                        } else {
                            this.gizmo_set_tool(scene_view::preview::gizmo::Tool::Move, cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &super::RotateTool, _, cx| {
                        if this.fly_keys.is_down() {
                            cx.propagate();
                        } else {
                            this.gizmo_set_tool(scene_view::preview::gizmo::Tool::Rotate, cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &super::ScaleTool, _, cx| {
                        if this.fly_keys.is_down() {
                            cx.propagate();
                        } else {
                            this.gizmo_set_tool(scene_view::preview::gizmo::Tool::Scale, cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &super::NoTool, _, cx| {
                        if this.fly_keys.is_down() {
                            cx.propagate();
                        } else {
                            this.gizmo_set_tool(scene_view::preview::gizmo::Tool::None, cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &super::ToggleGizmoSpace, _, cx| {
                        if this.fly_keys.gizmo_shortcuts_allowed() {
                            this.gizmo_toggle_space(cx);
                        }
                    }))
                    .on_action(cx.listener(|this, _: &super::ToggleGizmoPivot, _, cx| {
                        if this.fly_keys.gizmo_shortcuts_allowed() {
                            this.gizmo_toggle_pivot(cx);
                        }
                    }))
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        if !this.fly_keys.is_down() {
                            return;
                        }
                        this.fly_keys.key(&event.keystroke.key, true);
                        if event.keystroke.modifiers.shift {
                            this.fly_keys.key("shift", true);
                        }
                        // Render keeps requesting frames while keys are held;
                        // request_animation_frame is only allowed during paint.
                        cx.notify();
                    }))
                    .on_key_up(cx.listener(|this, event: &KeyUpEvent, _, cx| {
                        this.fly_keys.key(&event.keystroke.key, false);
                        if !event.keystroke.modifiers.shift {
                            this.fly_keys.key("shift", false);
                        }
                        cx.notify();
                    }))
            })
            .bg(rgb(0x25272d))
            .text_color(rgb(0xd4d7dd))
            .child(content)
    }
}
