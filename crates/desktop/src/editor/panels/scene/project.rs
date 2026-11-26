//! Native project viewport and controls.
use super::Space;
use super::coordinates::image_coordinates_for_size;
use crate::backend::Command;
use crate::editor::EditorPanel;
use crate::editor::menu::{LitMode, NormalMode, SolidMode, WireframeMode};
use gpui_kit::component::slider::Slider;
use gpui_kit::component::{
    Sizable,
    button::{Button, ButtonVariants},
    spinner::Spinner,
};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::*;
use scene_view::PreviewMode;
use scene_view::preview::gizmo::Tool;
use scene_view::preview::light_gizmo::LightKind;
use std::time::Instant;

#[derive(Default)]
pub(in crate::editor) struct FlyClock {
    tick: Option<Instant>,
}

impl FlyClock {
    pub(in crate::editor) fn start_if_idle(&mut self, now: Instant) {
        self.tick.get_or_insert(now);
    }

    pub(in crate::editor) fn consume(&mut self, now: Instant) -> f64 {
        let Some(tick) = self.tick.replace(now) else {
            return 0.;
        };
        now.duration_since(tick).as_secs_f64().min(0.1)
    }

    pub(in crate::editor) fn reset(&mut self) {
        self.tick = None;
    }
}

#[derive(Default)]
pub(in crate::editor) struct FlyKeys {
    right_down: bool,
    held: std::collections::BTreeSet<String>,
}

impl FlyKeys {
    pub(in crate::editor) fn right_down(&mut self) {
        self.right_down = true;
    }
    pub(in crate::editor) fn right_up(&mut self) {
        self.right_down = false;
        self.held.clear();
    }
    pub(in crate::editor) fn clear(&mut self) {
        self.right_up();
    }
    pub(in crate::editor) fn key(&mut self, key: &str, pressed: bool) {
        if !self.right_down {
            return;
        }
        let key = key.to_ascii_lowercase();
        if matches!(key.as_str(), "w" | "a" | "s" | "d" | "q" | "e" | "shift") {
            if pressed {
                self.held.insert(key);
            } else {
                self.held.remove(&key);
            }
        }
    }
    pub(in crate::editor) fn movement(&self) -> [f64; 3] {
        let axis = |positive: &str, negative: &str| {
            (if self.held.contains(positive) { 1. } else { 0. })
                - (if self.held.contains(negative) { 1. } else { 0. })
        };
        [axis("d", "a"), axis("e", "q"), axis("w", "s")]
    }
    pub(in crate::editor) fn faster(&self) -> bool {
        self.held.contains("shift")
    }
    pub(in crate::editor) fn is_down(&self) -> bool {
        self.right_down
    }
    pub(in crate::editor) fn gizmo_shortcuts_allowed(&self) -> bool {
        !self.right_down
    }
}
impl EditorPanel {
    pub(super) fn project_scene(&self, cx: &mut Context<Self>) -> AnyElement {
        // The preview draws at the viewport's size, known after layout.
        let size = self.viewport.get().size;
        let size = [
            f32::from(size.width).max(0.) as u32,
            f32::from(size.height).max(0.) as u32,
        ];
        if size[0] > 1 && size[1] > 1 && self.viewport_requested.replace(size) != size {
            self.session
                .update(cx, |s, cx| s.send(Command::Resize(size), cx));
        }
        let model = self.session.read(cx);
        if model.project.active_scene.is_none() {
            return div()
                .size_full()
                .flex()
                .items_center()
                .justify_center()
                .child("No scene is open")
                .into_any_element();
        }
        let image = model.image.clone();
        let updating = model.updating;
        let stale_frame = model.stale_frame;
        let preview_error = model.preview_error.clone();
        let session_entity = self.session.clone();
        let mut toolbar = div().flex().items_center().gap_2().p_2().bg(rgb(0x303238));
        {
            let session = session_entity.clone();
            toolbar = toolbar.child(
                Button::new("project-frame-all")
                    .icon(assets::IconName::Scan)
                    .small()
                    .tooltip("Frame all geometry (Shift+F); Shift-click focuses the selection (F)")
                    .on_click(move |event, _, cx| {
                        session.update(cx, |s, cx| {
                            s.send(
                                if event.modifiers().shift {
                                    Command::FrameSelection
                                } else {
                                    Command::FrameAll
                                },
                                cx,
                            )
                        })
                    }),
            );
        }
        // The display mode is this viewport's own: its buttons live here and
        // dispatch the same actions as View ▸ Solid/Normal/Wireframe/Lit.
        toolbar = toolbar.child(div().w(px(1.)).h(px(20.)).bg(rgb(0x4a4d55)));
        // Lit identifies its preview mode rather than
        // adding reflection shading — a direct-light approximation
        // whose metals stay darker than a render.
        const LIT_TOOLTIP: &str = "Lit: approximate direct lighting, no reflections — metals look darker than in a render";
        for (id, icon, label, value) in [
            (
                "mode-solid",
                assets::IconName::Box,
                "Solid",
                PreviewMode::Solid,
            ),
            (
                "mode-normal",
                assets::IconName::ScanSearch,
                "Normal",
                PreviewMode::Normal,
            ),
            (
                "mode-wire",
                assets::IconName::Triangle,
                "Wireframe",
                PreviewMode::Wireframe,
            ),
            ("mode-lit", assets::IconName::Sun, "Lit", PreviewMode::Lit),
        ] {
            let action: Box<dyn Action> = match value {
                PreviewMode::Solid => Box::new(SolidMode),
                PreviewMode::Normal => Box::new(NormalMode),
                PreviewMode::Wireframe => Box::new(WireframeMode),
                PreviewMode::Lit => Box::new(LitMode),
            };
            toolbar = toolbar.child(
                Button::new(id)
                    .icon(icon)
                    .small()
                    // `toggled` is the accessible pressed state. The dark
                    // theme's selected fill matches the toolbar, so the
                    // current mode is painted as a primary button.
                    // All four are icon-only, like the gizmo tools; the
                    // hover tooltip names the mode.
                    .when(model.preview_mode == value, |button| button.primary())
                    .toggled(model.preview_mode == value)
                    .tooltip(if value == PreviewMode::Lit {
                        LIT_TOOLTIP
                    } else {
                        label
                    })
                    .accessibility_label(label)
                    .on_click(move |_, window, cx| {
                        window.dispatch_action(action.boxed_clone(), cx);
                    }),
            );
        }
        // Transform tools and the Local / World space.
        toolbar = toolbar.child(div().w(px(1.)).h(px(20.)).bg(rgb(0x4a4d55)));
        for (id, icon, label, tool) in [
            (
                "gizmo-none",
                assets::IconName::MousePointer,
                "Select (Q)",
                Tool::None,
            ),
            (
                "gizmo-move",
                assets::IconName::Move3d,
                "Move (W)",
                Tool::Move,
            ),
            (
                "gizmo-rotate",
                assets::IconName::Rotate3d,
                "Rotate (E)",
                Tool::Rotate,
            ),
            (
                "gizmo-scale",
                assets::IconName::Scale3d,
                "Scale (R)",
                Tool::Scale,
            ),
        ] {
            let on = self.gizmo_tool == tool;
            toolbar = toolbar.child(
                Button::new(id)
                    .icon(icon)
                    .small()
                    .when(on, |button| button.primary())
                    .toggled(on)
                    .tooltip(label)
                    .accessibility_label(label)
                    .on_click(cx.listener(move |this, _, _, cx| this.gizmo_set_tool(tool, cx))),
            );
        }
        // Scale is always local, so the space toggle shows only for Move/Rotate.
        if matches!(self.gizmo_tool, Tool::Move | Tool::Rotate) {
            let world = self.gizmo_space == Space::World;
            let (icon, tooltip) = gizmo_space_face(world);
            toolbar = toolbar.child(
                Button::new("gizmo-space")
                    .icon(icon)
                    .small()
                    // Icon-only like the preview-mode and gizmo-tool
                    // buttons; the icon and the tooltip name the
                    // axes the toggle switches away from.
                    .tooltip(tooltip)
                    .accessibility_label(tooltip)
                    .on_click(cx.listener(|this, _, _, cx| this.gizmo_toggle_space(cx))),
            );
        }
        if model.supported_times.len() > 1 {
            toolbar = toolbar
                .child(div().text_xs().child(format!("t={:.3}", model.view_time)))
                .child(Slider::new(&model.time_slider).w_48());
        }
        let bounds = self.viewport.clone();
        let mut viewport = div()
            .id("project-scene-viewport")
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .bg(rgb(0x17191e))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    // A press on a gizmo handle drags it instead of picking;
                    // then the light's own handles — the transform
                    // gizmo's press wins when both are under the pointer.
                    if this.gizmo_press(event.position, cx) {
                        cx.notify();
                        return;
                    }
                    if this.light_press(event.position, cx) {
                        cx.notify();
                        return;
                    }
                    let model = this.session.read(cx);
                    if crate::editor::session::scene_geometry_interactive(
                        model.updating,
                        model.stale_frame,
                    ) {
                        if let Some(xy) = image_coordinates_for_size(
                            this.viewport.get(),
                            event.position,
                            model.image_size,
                        ) {
                            let frame_id = model.frame_id;
                            this.session
                                .update(cx, |s, cx| s.send(Command::Pick { frame_id, xy }, cx));
                        }
                    }
                }),
            )
            .on_mouse_down(
                MouseButton::Right,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.fly_keys.right_down();
                    this.fly_clock.start_if_idle(Instant::now());
                    // A right click while dragging the gizmo cancels it.
                    if this.gizmo_cancel(cx) {
                        return;
                    }
                    this.last_mouse = Some(event.position);
                    let model = this.session.read(cx);
                    if crate::editor::session::scene_geometry_interactive(
                        model.updating,
                        model.stale_frame,
                    ) && let Some(xy) = image_coordinates_for_size(
                        this.viewport.get(),
                        event.position,
                        model.image_size,
                    ) {
                        this.session
                            .update(cx, |s, cx| s.send(Command::RetargetAt(xy), cx));
                    }
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.gizmo_release(cx)),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|this, _: &MouseUpEvent, _, cx| this.gizmo_release(cx)),
            )
            .on_mouse_up(
                MouseButton::Right,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.last_mouse = None;
                    this.fly_keys.right_up();
                    this.fly_clock.reset();
                }),
            )
            .on_mouse_up_out(
                MouseButton::Right,
                cx.listener(|this, _: &MouseUpEvent, _, _| {
                    this.last_mouse = None;
                    this.fly_keys.right_up();
                    this.fly_clock.reset();
                }),
            )
            .on_mouse_down(
                MouseButton::Middle,
                cx.listener(|this, event: &MouseDownEvent, window, cx| {
                    this.focus.focus(window, cx);
                    this.last_mouse = Some(event.position)
                }),
            )
            .on_mouse_move(cx.listener(|this, event: &MouseMoveEvent, window, cx| {
                // Ctrl snaps the drag to the grid.
                let snap = window.modifiers().secondary();
                match event.pressed_button {
                    Some(MouseButton::Left) => this.gizmo_move(event.position, true, snap, cx),
                    None => this.gizmo_move(event.position, false, snap, cx),
                    _ => {}
                }
                if this.gizmo_drag.is_none()
                    && this.light_drag.is_none()
                    && matches!(
                        event.pressed_button,
                        Some(MouseButton::Right | MouseButton::Middle)
                    )
                {
                    if let Some(last) = this.last_mouse.replace(event.position) {
                        let dx = f64::from(f32::from(event.position.x - last.x));
                        let dy = f64::from(f32::from(event.position.y - last.y));
                        let command = if event.pressed_button == Some(MouseButton::Right) {
                            Command::Orbit(-dx * 0.008, -dy * 0.008)
                        } else {
                            Command::Pan(-dx * 0.002, dy * 0.002)
                        };
                        this.session.update(cx, |s, cx| s.send(command, cx));
                    }
                } else {
                    this.last_mouse = None;
                }
            }))
            .on_scroll_wheel(cx.listener(|this, event: &ScrollWheelEvent, _, cx| {
                // A drag in progress keeps its press-time mapping; the
                // light cone drag is dropped by the same stale check.
                if this.gizmo_drag.is_some() || this.light_drag.is_some() {
                    return;
                }
                let delta = match event.delta {
                    ScrollDelta::Pixels(p) => f64::from(f32::from(p.y)) * 0.005,
                    ScrollDelta::Lines(p) => f64::from(p.y) * 0.12,
                };
                this.session
                    .update(cx, |s, cx| s.send(Command::Dolly(-delta), cx));
            }));
        if let Some(image) = image {
            viewport = viewport.child(
                img(image)
                    .absolute()
                    .size_full()
                    .object_fit(ObjectFit::Contain),
            );
        }
        let gizmo = if stale_frame || updating {
            None
        } else {
            self.gizmo_view(cx)
        };
        let gizmo_shapes = match &gizmo {
            Some(Ok(view)) => self.gizmo_paint_list(view),
            _ => Vec::new(),
        };
        let gizmo_reason = match gizmo {
            Some(Err(reason)) => Some(reason),
            _ => None,
        };
        // The selected light draws under the transform gizmo,
        // whatever the tool and in the read-only Viewer. A
        // spot's cone drag in progress paints its own lines — but
        // a stale or updating frame hides them all, and gizmo_move
        // drops the drag on the same condition, like the gizmo's.
        let light = if stale_frame || updating {
            None
        } else {
            self.light_view(cx)
        };
        let light_shapes = if stale_frame || updating {
            Vec::new()
        } else {
            match &self.light_drag {
                Some(_) => self.light_drag_paint_list(),
                None => match &light {
                    Some(view) => self.light_paint_list(view),
                    None => Vec::new(),
                },
            }
        };
        viewport = viewport.child(
            canvas(
                move |rect, window, _| {
                    if bounds.replace(rect).size != rect.size {
                        window.refresh();
                    }
                },
                move |rect, _, window, _| {
                    // Gizmo paths paint after the same layer's text
                    // and quads, so handle lines that run past the
                    // viewport would still land on the hint bar (and the
                    // toolbar) — clip them to the viewport while painting.
                    window.with_content_mask(Some(ContentMask { bounds: rect }), |window| {
                        // The light's thin wire-frame goes first: it sits
                        // under the transform gizmo's own handles.
                        super::gizmo::paint(&light_shapes, window, px(super::gizmo::LIGHT_WIDTH));
                        super::gizmo::paint(&gizmo_shapes, window, px(2.5))
                    })
                },
            )
            .absolute()
            .size_full(),
        );
        return div()
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(0x17191e))
            .child(toolbar)
            .child(
                viewport.when(updating || stale_frame || preview_error.is_some(), |view| {
                // Deferred draws the badge after the gizmo's paths,
                // so the gizmo can no longer paint over it. The badge has
                // no mouse handlers, so the viewport keeps its events.
                view.child(deferred(
                    div()
                        .absolute()
                        .top_2()
                        .right_2()
                        .p_2()
                        .bg(rgb(0x303238))
                        .flex()
                        .items_center()
                        .gap_2()
                        // Non-blocking status: a small spinner beside the
                        // "Updating preview…" text.
                        .when(updating && preview_error.is_none(), |row| {
                            row.child(Spinner::new().small())
                        })
                        .child(if let Some(error) = preview_error {
                            if stale_frame {
                                format!("{error} · showing last frame")
                            } else {
                                error
                            }
                        } else if stale_frame && !updating {
                            "Preview update failed · showing last frame".to_owned()
                        } else {
                            "Updating preview…".to_owned()
                        }),
                ))
            }),
            )
            .child(
                // The gizmo's paths paint after this layer's quads,
                // and its lines are clipped only to the viewport — the
                // hint bar draws deferred on the panel's
                // opaque background, so overlay UI stays readable and
                // above the gizmo. The bar registers no mouse handlers,
                // so the viewport keeps its events.
                deferred(
                    div()
                        .h(px(22.))
                        .px_2()
                        .text_xs()
                        .whitespace_nowrap()
                        .overflow_hidden()
                        .text_ellipsis()
                        .text_color(rgb(0x9298a5))
                        .bg(rgb(0x17191e))
                        .child(match gizmo_reason {
                            Some(reason) => format!("No gizmo: {reason}"),
                            None => {
                                let verb = match self.gizmo_tool {
                                    Tool::None => "W/E/R move/rotate/scale · click to select",
                                    Tool::Move => "Drag to move",
                                    Tool::Rotate => "Drag to rotate",
                                    Tool::Scale => "Drag to scale",
                                };
                                // Use one phrase about the
                                // selected light's drag, only where a
                                // drag can run at all.
                                let kind = light.as_ref().map(|view| view.light.values.kind);
                                let editable = !self
                                    .session
                                    .read(cx)
                                    .document()
                                    .is_read_only();
                                // The drag phrase rides right after the
                                // verb so a narrow panel's ellipsis cuts
                                // the generic keys, not it.
                                let drag_hint = light_drag_hint(kind, editable).unwrap_or("");
                                format!(
                                    "{verb}{drag_hint} · Ctrl snap · X Local/World · Esc cancel · RMB orbit · F focus"
                                )
                            }
                        }),
                ),
            )
            .into_any_element();
    }
}

/// The Local / World toggle's icon and tooltip follow the space it
/// switches away from — Local shows a box, World the globe — naming the
/// current axes and the X key, like the preview-mode buttons name
/// their mode.
fn gizmo_space_face(world: bool) -> (assets::IconName, &'static str) {
    if world {
        (assets::IconName::Globe, "World axes (X)")
    } else {
        (assets::IconName::Box, "Local axes (X)")
    }
}

/// The hint bar's one phrase about a selected light's
/// drag — only where a drag can run (the Viewer shows its plain lines
/// with no handles). A spot drags its cone, a distant light its aim
/// arrow; point, goniometric and projection lights draw no handle.
fn light_drag_hint(kind: Option<LightKind>, editable: bool) -> Option<&'static str> {
    match (kind, editable) {
        (Some(LightKind::Spot), true) => Some(" · drag the cone to edit"),
        (Some(LightKind::Distant), true) => Some(" · drag the arrow to aim"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{FlyClock, FlyKeys, LightKind, gizmo_space_face, light_drag_hint};
    use gpui_kit::assets;
    use std::time::{Duration, Instant};

    /// The icon and tooltip follow the space the toggle switches
    /// from — Local shows a box, World the globe — and each names the
    /// current axes with the X key, like the mode buttons name their
    /// mode.
    #[test]
    fn space_toggle_names_the_current_axes_and_the_key() {
        assert_eq!(
            gizmo_space_face(false),
            (assets::IconName::Box, "Local axes (X)")
        );
        assert_eq!(
            gizmo_space_face(true),
            (assets::IconName::Globe, "World axes (X)")
        );
    }

    /// The hint names the drag only for a light with
    /// handles in an editable document — a spot's cone, a distant
    /// light's aim arrow; the Viewer never drags.
    #[test]
    fn the_drag_hint_names_a_light_with_handles_only() {
        assert_eq!(
            light_drag_hint(Some(LightKind::Spot), true),
            Some(" · drag the cone to edit")
        );
        assert_eq!(
            light_drag_hint(Some(LightKind::Distant), true),
            Some(" · drag the arrow to aim")
        );
        assert_eq!(
            light_drag_hint(Some(LightKind::Point), true),
            None,
            "a point light has no handle"
        );
        assert_eq!(
            light_drag_hint(Some(LightKind::Goniometric), true),
            None,
            "goniometric and projection lights draw no handle"
        );
        assert_eq!(
            light_drag_hint(Some(LightKind::Distant), false),
            None,
            "the Viewer never drags"
        );
        assert_eq!(light_drag_hint(None, true), None);
        assert_eq!(light_drag_hint(None, false), None);
    }

    #[test]
    fn fly_keys_track_press_release_and_clear_on_right_button_up() {
        let mut keys = FlyKeys::default();
        keys.key("w", true);
        assert_eq!(keys.movement(), [0., 0., 0.]);
        keys.right_down();
        keys.key("W", true);
        keys.key("d", true);
        assert_eq!(keys.movement(), [1., 0., 1.]);
        keys.key("w", false);
        assert_eq!(keys.movement(), [1., 0., 0.]);
        keys.key("shift", true);
        assert!(keys.faster());
        keys.right_up();
        assert!(!keys.is_down());
        assert_eq!(keys.movement(), [0., 0., 0.]);
        assert!(!keys.faster());
    }

    #[test]
    fn fly_clock_keeps_elapsed_time_across_repeated_keys_until_consumed() {
        let start = Instant::now();
        let mut clock = FlyClock::default();
        clock.start_if_idle(start);
        clock.start_if_idle(start + Duration::from_millis(20));
        assert!((clock.consume(start + Duration::from_millis(50)) - 0.05).abs() < 0.001);
        assert!((clock.consume(start + Duration::from_millis(80)) - 0.03).abs() < 0.001);
    }

    #[test]
    fn gizmo_keys_do_not_dispatch_as_tools_while_flying() {
        let mut keys = FlyKeys::default();
        keys.right_down();
        keys.key("w", true);
        assert!(keys.is_down());
        assert!(!keys.gizmo_shortcuts_allowed());
        assert_eq!(keys.movement(), [0., 0., 1.]);
        keys.right_up();
        assert!(keys.gizmo_shortcuts_allowed());
    }
}
