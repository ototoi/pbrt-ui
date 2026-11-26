//! The transform gizmo overlays the Scene viewport. Handles are drawn and
//! hit in image pixels; a drag previews through the backend and commits one
//! `set_local_trs` / `set_local_matrix` on release.
//!
//! The selected light's thin wire-frame is drawn under these handles from the same
//! Inspector details — whatever the tool, and in the read-only Viewer
//! too.
use super::coordinates::image_placement;
use crate::backend::{Command, Follow};
use crate::editor::light_gizmo::LightTarget;
use crate::editor::{EditorPanel, GIZMO_FORM, gizmo::Target};
use gpui_kit::*;
use scene_view::preview::gizmo::{Drag, Gizmo, Handle, Projector, Shape, Tool};
use scene_view::preview::light_gizmo as light_wireframe;

/// Handle length in screen pixels.
const LENGTH: f64 = 90.;
/// The light wire-frame uses one warm yellow colour,
/// thinner than the gizmo's handles. A dragged or hovered spot handle
/// takes the lighter `LIGHT_HIGHLIGHT`, like the gizmo's hover.
pub(super) const LIGHT_WIDTH: f32 = 1.5;
const LIGHT_COLOUR: u32 = 0xf0b429;
const LIGHT_HIGHLIGHT: u32 = 0xffe699;
/// While a transform-gizmo handle is hovered or a gizmo drag
/// runs, the light keeps its colours but paints at this alpha: the
/// gizmo's drag highlight being the same warm yellow family, the lines
/// step back instead of changing colour.
const LIGHT_DIM_ALPHA: u32 = 0x80;

/// Which axes the gizmo uses. Scale is always local.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum Space {
    #[default]
    Local,
    World,
}

pub(crate) struct GizmoDrag {
    target: Target,
    gizmo: Gizmo,
    projector: Projector,
    /// The view the drag measures in; a change cancels the drag.
    camera: scene_view::ViewerCamera,
    image_size: [u32; 2],
    handle: Handle,
    from: [f64; 2],
    drag: Option<Drag>,
}

/// One light drag: what it drags against at the
/// press-time view — a spot's cone rim or tip, or a distant light's aim
/// arrow; only the drawn lines follow during the drag and release
/// commits the values as one Apply.
pub(crate) struct LightDrag {
    target: crate::editor::light_gizmo::LightTarget,
    /// The rim's frame at press; every move measures against it.
    frame: light_wireframe::SpotFrame,
    /// The light's inverse world transform, for the tip's `to`.
    light_from_world: [f64; 16],
    projector: Projector,
    /// The view the drag measures in; a change cancels the drag.
    camera: scene_view::ViewerCamera,
    image_size: [u32; 2],
    handle: light_wireframe::SpotHandle,
    /// The pointer's ray at press: the tip drag moves by its offset.
    press_ray: ([f64; 3], [f64; 3]),
    /// What the drag has settled on so far (`None` until it moves that
    /// value); `crate::editor::light_gizmo::apply_edits` paints it.
    edits: crate::editor::light_gizmo::SpotEdits,
}

/// The gizmo as it stands: what it moves and where its handles are.
pub(super) struct GizmoView {
    pub target: Target,
    pub gizmo: Gizmo,
    pub projector: Projector,
    /// Image top-left on screen and screen pixels per image pixel.
    pub origin: Point<Pixels>,
    pub scale: f64,
}

/// The selected light as it stands: what it draws and where.
pub(super) struct LightView {
    pub light: LightTarget,
    pub projector: Projector,
    /// The view the light projects with, captured for a drag.
    pub camera: scene_view::ViewerCamera,
    pub image_size: [u32; 2],
    /// Image top-left on screen and screen pixels per image pixel.
    pub origin: Point<Pixels>,
    pub scale: f64,
}

impl EditorPanel {
    /// The axes for `tool` in the panel's current space (Scale is always
    /// local, World is the identity basis).
    fn gizmo_axes(&self, target: &Target) -> [[f64; 3]; 3] {
        if self.gizmo_space == Space::World && self.gizmo_tool != Tool::Scale {
            [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]]
        } else {
            target.local_axes()
        }
    }

    /// The gizmo for the inspected node, or why there is none (`None` when
    /// nothing is selected or the frame is not ready).
    pub(super) fn gizmo_view(&self, cx: &App) -> Option<Result<GizmoView, String>> {
        let session = self.session.read(cx);
        let details = &session.project_inspector;
        if !details["node"]["node_id"].is_string() {
            return None;
        }
        if let Some(count) = crate::editor::project_actions::definition_instances_for_node(details)
        {
            let plural = if count == 1 { "" } else { "s" };
            return Some(Err(format!(
                "inside a definition used by {count} instance{plural} — edit in the Inspector"
            )));
        }
        if self.gizmo_tool == Tool::None {
            return None;
        }
        if session.document().is_read_only() {
            return Some(Err("Viewer is read-only".into()));
        }
        let mut target = match crate::editor::gizmo::target(details) {
            Ok(target) => target,
            Err(reason) => return Some(Err(reason)),
        };
        let bounds_center = session.selection_bounds_center;
        if self.gizmo_center {
            target.pivot = bounds_center;
        }
        // The Inspector refreshes after an edit; until then the values are
        // stale and a drag would start from the old place.
        if target.project != session.project_value() {
            return None;
        }
        let camera = session.view_camera?;
        let size = session.image_size;
        let projector = Projector::new(&camera, size[0], size[1])?;
        let (origin, scale) = image_placement(self.viewport.get(), size)?;
        // A move in progress slides the gizmo with the object.
        let shift = match self.gizmo_drag.as_ref().and_then(|drag| drag.drag) {
            Some(Drag::Move(d)) => d,
            _ => [0.; 3],
        };
        let o = target.pivot.unwrap_or_else(|| target.origin());
        // A pivot off the image draws nothing, so say why instead of
        // "Drag to move"; a drag in progress keeps its gizmo wherever it goes.
        if self.gizmo_drag.is_none() && !on_image(projector.project(o), size) {
            let hint = if self.gizmo_center {
                "center is off screen · F focuses the selection"
            } else {
                "pivot is off screen · Z switches to Center · F focuses the selection"
            };
            return Some(Err(hint.into()));
        }
        let gizmo = Gizmo {
            tool: self.gizmo_tool,
            origin: [o[0] + shift[0], o[1] + shift[1], o[2] + shift[2]],
            axes: self.gizmo_axes(&target),
            length: LENGTH / scale,
        };
        Some(Ok(GizmoView {
            target,
            gizmo,
            projector,
            origin,
            scale,
        }))
    }

    /// The image pixel under a screen point, given the image's placement.
    fn image_point(origin: Point<Pixels>, scale: f64, position: Point<Pixels>) -> [f64; 2] {
        [
            f64::from(f32::from(position.x - origin.x)) / scale,
            f64::from(f32::from(position.y - origin.y)) / scale,
        ]
    }

    /// Start a drag when the press lands on a handle; `true` if it did.
    pub(super) fn gizmo_press(&mut self, position: Point<Pixels>, cx: &App) -> bool {
        let interactive = {
            let session = self.session.read(cx);
            crate::editor::session::scene_geometry_interactive(
                session.updating,
                session.stale_frame,
            )
        };
        if !interactive {
            return false;
        }
        let Some(Ok(view)) = self.gizmo_view(cx) else {
            return false;
        };
        let xy = Self::image_point(view.origin, view.scale, position);
        let Some(handle) = view.gizmo.hit(&view.projector, xy) else {
            return false;
        };
        let session = self.session.read(cx);
        let (Some(camera), image_size) = (session.view_camera, session.image_size) else {
            return false;
        };
        self.gizmo_drag = Some(GizmoDrag {
            target: view.target,
            gizmo: view.gizmo,
            projector: view.projector,
            camera,
            image_size,
            handle,
            from: xy,
            drag: None,
        });
        true
    }

    /// Follow the pointer: drag, or update the hovered handle.
    pub(super) fn gizmo_move(
        &mut self,
        position: Point<Pixels>,
        dragging: bool,
        snap: bool,
        cx: &mut Context<Self>,
    ) {
        let interactive = {
            let session = self.session.read(cx);
            crate::editor::session::scene_geometry_interactive(
                session.updating,
                session.stale_frame,
            )
        };
        if !interactive {
            self.gizmo_drag = None;
            self.gizmo_hover = None;
            self.light_drag = None;
            self.light_hover = None;
            return;
        }
        if dragging && let Some(drag) = &self.gizmo_drag {
            let session = self.session.read(cx);
            // The pointer maps to world units through the view at press;
            // after a dolly, frame or resize that mapping is wrong.
            if session.view_camera != Some(drag.camera) || session.image_size != drag.image_size {
                self.gizmo_cancel(cx);
                return;
            }
            let Some((origin, scale)) = image_placement(self.viewport.get(), drag.image_size)
            else {
                return;
            };
            let drag = self.gizmo_drag.as_mut().unwrap();
            let xy = [
                f64::from(f32::from(position.x - origin.x)) / scale,
                f64::from(f32::from(position.y - origin.y)) / scale,
            ];
            if let Some(value) = drag
                .gizmo
                .drag(&drag.projector, drag.handle, drag.from, xy, snap)
            {
                drag.drag = Some(value);
                let matrix = drag.target.delta_matrix(value).0;
                self.session.update(cx, |session, cx| {
                    session.send(Command::GizmoPreview(Some(matrix)), cx)
                });
                cx.notify();
            }
            return;
        }
        // A spot's cone drag never touches the preview or the
        // document during the drag, so the camera cannot have moved;
        // its lines show the values it will commit.
        if dragging && self.light_drag.is_some() {
            self.light_drag_move(position, cx);
            return;
        }
        let hover = match self.gizmo_view(cx) {
            Some(Ok(view)) => view
                .gizmo
                .hit(
                    &view.projector,
                    Self::image_point(view.origin, view.scale, position),
                )
                // The hover stays with the node it was hovered on.
                .map(|handle| (view.target.node_id.clone(), handle)),
            _ => None,
        };
        if hover.as_ref() != self.gizmo_hover.as_ref() {
            self.gizmo_hover = hover.clone();
            cx.notify();
        }
        // The light's handles hover only where the transform gizmo does
        // not: its press also wins — and only in an editable
        // document: the Viewer keeps the plain lines.
        let editable = !self.session.read(cx).document().is_read_only();
        let light_hover = match (self.light_view(cx), hover.is_none() && editable) {
            (Some(view), true)
                if matches!(
                    view.light.values.kind,
                    light_wireframe::LightKind::Spot | light_wireframe::LightKind::Distant
                ) =>
            {
                let length = LENGTH / view.scale;
                let shapes = light_wireframe::light_shapes(
                    &view.light.values,
                    self.light_world(&view.light).0,
                    &view.projector,
                    length,
                );
                light_wireframe::spot_hit(
                    &shapes,
                    Self::image_point(view.origin, view.scale, position),
                )
                .map(|handle| (view.light.target.node_id.clone(), handle))
            }
            _ => None,
        };
        if light_hover != self.light_hover {
            self.light_hover = light_hover;
            cx.notify();
        }
    }

    /// Release: commit the drag as one edit.
    pub(super) fn gizmo_release(&mut self, cx: &mut Context<Self>) {
        // A spot's cone drag commits its own one-Apply edit set;
        // the two drags never coexist, so one is always None here.
        if self.light_release(cx) {
            return;
        }
        let Some(drag) = self.gizmo_drag.take() else {
            return;
        };
        cx.notify();
        let Some(value) = drag.drag.filter(|d| !d.is_identity()) else {
            self.session.update(cx, |session, cx| {
                session.send(Command::GizmoPreview(None), cx)
            });
            return;
        };
        match drag.target.edit(value) {
            Ok(edit) => self.session.update(cx, |session, cx| {
                session.send(
                    Command::Apply {
                        project: drag.target.project.clone(),
                        edits: vec![edit],
                        follow: Follow::Nothing,
                        form: Some(GIZMO_FORM.into()),
                    },
                    cx,
                )
            }),
            Err(error) => self.session.update(cx, |session, cx| {
                session.send(Command::GizmoPreview(None), cx);
                session.status = error;
                cx.notify();
            }),
        }
    }

    /// Esc or a right click while dragging: put everything back.
    pub(crate) fn gizmo_cancel(&mut self, cx: &mut Context<Self>) -> bool {
        if self.gizmo_drag.take().is_some() {
            self.session.update(cx, |session, cx| {
                session.send(Command::GizmoPreview(None), cx)
            });
            cx.notify();
            return true;
        }
        // A cone drag in progress never touched the document or the
        // preview: dropping it restores the page's own values.
        if self.light_drag.take().is_some() {
            cx.notify();
            return true;
        }
        false
    }

    /// Switch the active tool (toolbar buttons and W / E / R / Q).
    pub(crate) fn gizmo_set_tool(&mut self, tool: Tool, cx: &mut Context<Self>) {
        self.gizmo_cancel(cx);
        self.gizmo_tool = tool;
        self.gizmo_hover = None;
        cx.notify();
    }

    /// Toggle the Local / World space.
    pub(crate) fn gizmo_toggle_space(&mut self, cx: &mut Context<Self>) {
        self.gizmo_cancel(cx);
        self.gizmo_space = match self.gizmo_space {
            Space::Local => Space::World,
            Space::World => Space::Local,
        };
        cx.notify();
    }

    /// Switch between the node pivot and the selection bounds centre.
    pub(crate) fn gizmo_toggle_pivot(&mut self, cx: &mut Context<Self>) {
        self.gizmo_cancel(cx);
        self.gizmo_center = !self.gizmo_center;
        self.gizmo_hover = None;
        cx.notify();
    }

    /// The handles to paint, in screen pixels, with their colours.
    pub(super) fn gizmo_paint_list(
        &self,
        view: &GizmoView,
    ) -> Vec<(Vec<Point<Pixels>>, bool, Hsla)> {
        let active = self
            .gizmo_drag
            .as_ref()
            .map(|drag| drag.handle)
            .or_else(|| hover_for_node(&self.gizmo_hover, &view.target.node_id));
        let screen = |p: [f64; 2]| {
            point(
                view.origin.x + px((p[0] * view.scale) as f32),
                view.origin.y + px((p[1] * view.scale) as f32),
            )
        };
        view.gizmo
            .shapes(&view.projector)
            .into_iter()
            .filter(|(handle, _)| {
                // While dragging, only the dragged handle stays.
                self.gizmo_drag
                    .as_ref()
                    .is_none_or(|drag| drag.handle == *handle)
            })
            .map(|(handle, shape)| {
                let base = match handle {
                    Handle::Axis(a) | Handle::Plane(a) => [0xe0524a, 0x6cc04a, 0x4a8fe0][a],
                    Handle::Screen => 0xe8e8e8,
                };
                let color = if active == Some(handle) {
                    0xf2d24b
                } else {
                    base
                };
                let alpha = if matches!(handle, Handle::Plane(_)) {
                    0x99
                } else {
                    0xff
                };
                let color: Hsla = rgba((color << 8) | alpha).into();
                match shape {
                    Shape::Line(a, b) => (vec![screen(a), screen(b)], false, color),
                    Shape::Fill(points) => (points.into_iter().map(screen).collect(), true, color),
                }
            })
            .collect()
    }

    /// The selected light as it draws: `None` when the selection is no
    /// light, its implementation has no wire-frame, the frame is not
    /// ready, or the Inspector page is stale. Unlike the transform
    /// gizmo, this shows with every tool (also `Tool::None`) and in the
    /// read-only Viewer.
    pub(super) fn light_view(&self, cx: &App) -> Option<LightView> {
        let session = self.session.read(cx);
        let details = &session.project_inspector;
        if !details["node"]["node_id"].is_string() {
            return None;
        }
        let light = crate::editor::light_gizmo::light(details)?;
        // The Inspector refreshes after an edit; drawing the stale values
        // would lag the scene the preview shows.
        if light.target.project != session.project_value() {
            return None;
        }
        let camera = session.view_camera?;
        let size = session.image_size;
        let projector = Projector::new(&camera, size[0], size[1])?;
        let (origin, scale) = image_placement(self.viewport.get(), size)?;
        Some(LightView {
            light,
            projector,
            camera,
            image_size: size,
            origin,
            scale,
        })
    }

    /// The light's handles to paint: thin lines in one light colour
    /// under the transform gizmo's own list so they never cover a
    /// handle. A hovered spot handle highlights like the gizmo's.
    pub(super) fn light_paint_list(
        &self,
        view: &LightView,
    ) -> Vec<(Vec<Point<Pixels>>, bool, Hsla)> {
        let length = LENGTH / view.scale;
        let world = self.light_world(&view.light);
        let shapes =
            light_wireframe::light_shapes(&view.light.values, world.0, &view.projector, length);
        // While a transform-gizmo handle is hovered or a gizmo
        // drag runs, the light's lines paint at half opacity — the two
        // drags never coexist, and the light drag's own lines (in
        // `light_drag_paint_list`) keep full opacity.
        let dimmed = self.gizmo_drag.is_some()
            || hover_for_node(&self.gizmo_hover, &view.light.target.node_id).is_some();
        let hover = self
            .light_hover
            .as_ref()
            .filter(|(node, _)| *node == view.light.target.node_id)
            .map(|(_, handle)| *handle);
        light_lines(shapes, hover, dimmed, view.origin, view.scale)
    }

    /// The dragged spot's own paint list: only its lines follow the
    /// drag — the Lit preview and everything else stay as they were —
    /// and the dragged handle is highlighted.
    pub(super) fn light_drag_paint_list(&self) -> Vec<(Vec<Point<Pixels>>, bool, Hsla)> {
        let Some(drag) = &self.light_drag else {
            return Vec::new();
        };
        let Some((origin, scale)) = image_placement(self.viewport.get(), drag.image_size) else {
            return Vec::new();
        };
        let values = crate::editor::light_gizmo::apply_edits(&drag.target.values, &drag.edits);
        let shapes = light_wireframe::light_shapes(
            &values,
            drag.target.target.world.0,
            &drag.projector,
            LENGTH / scale,
        );
        // The light drag's own lines keep full opacity because the
        // transform gizmo's drag never runs at the same time.
        light_lines(shapes, Some(drag.handle), false, origin, scale)
    }

    /// The world matrix the light draws with: its own, or the same with
    /// a transform-gizmo drag's preview delta pre-multiplied, so the
    /// wire-frame follows the drag the gizmo handles and the preview
    /// already show.
    fn light_world(&self, light: &LightTarget) -> project::math::Mat4 {
        if let Some(drag) = &self.gizmo_drag
            && let Some(value) = drag.drag
        {
            // `world' = delta · world`, the same left-multiplication the
            // gizmo slides its own origin with.
            return drag.target.delta_matrix(value).compose(light.target.world);
        }
        light.target.world
    }

    /// Start a light drag when the press lands on one of the drawn
    /// handles; `true` if it did. The transform gizmo is
    /// asked first, so its handles win; the Viewer never handles (it
    /// keeps the plain lines).
    pub(super) fn light_press(&mut self, position: Point<Pixels>, cx: &App) -> bool {
        let interactive = {
            let session = self.session.read(cx);
            crate::editor::session::scene_geometry_interactive(
                session.updating,
                session.stale_frame,
            )
        };
        if !interactive {
            return false;
        }
        if self.session.read(cx).document().is_read_only() {
            return false;
        }
        let Some(view) = self.light_view(cx) else {
            return false;
        };
        // A spot drags its cone, and a distant light its aim arrow; the
        // the other kinds draw no handle at all.
        if !matches!(
            view.light.values.kind,
            light_wireframe::LightKind::Spot | light_wireframe::LightKind::Distant
        ) {
            return false;
        }
        let length = LENGTH / view.scale;
        let world = self.light_world(&view.light);
        let shapes =
            light_wireframe::light_shapes(&view.light.values, world.0, &view.projector, length);
        let xy = Self::image_point(view.origin, view.scale, position);
        let Some(handle) = light_wireframe::spot_hit(&shapes, xy) else {
            return false;
        };
        let Some(frame) =
            light_wireframe::SpotFrame::of(&view.light.values, world.0, &view.projector, length)
        else {
            return false;
        };
        let Some(inverse) = project::math::inverse(world) else {
            return false;
        };
        self.light_hover = None;
        self.light_drag = Some(LightDrag {
            target: view.light,
            frame,
            light_from_world: inverse.0,
            projector: view.projector,
            camera: view.camera,
            image_size: view.image_size,
            handle,
            press_ray: view.projector.ray(xy),
            edits: Default::default(),
        });
        true
    }

    /// Follow the pointer with the spot drag: one value per handle,
    /// clamped, measured against the press-time frame; only the drawn
    /// lines follow — no preview refresh, no edit until release.
    fn light_drag_move(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let Some(drag) = &self.light_drag else {
            return;
        };
        // The pointer maps to world rays through the view at press;
        // after a dolly, frame or resize that mapping is wrong.
        let stale = {
            let session = self.session.read(cx);
            session.view_camera != Some(drag.camera) || session.image_size != drag.image_size
        };
        if stale {
            self.light_drag = None;
            cx.notify();
            return;
        }
        let (frame, handle, projector, image_size, press_ray) = (
            drag.frame,
            drag.handle,
            drag.projector,
            drag.image_size,
            drag.press_ray,
        );
        let Some((origin, scale)) = image_placement(self.viewport.get(), image_size) else {
            return;
        };
        let xy = [
            f64::from(f32::from(position.x - origin.x)) / scale,
            f64::from(f32::from(position.y - origin.y)) / scale,
        ];
        let ray = projector.ray(xy);
        let coneangle = drag.target.values.coneangle_degrees;
        if let Some(drag) = &mut self.light_drag {
            match handle {
                light_wireframe::SpotHandle::Cone => {
                    if let Some(degrees) = light_wireframe::cone_angle_degrees(&frame, ray) {
                        let (coneangle, conedelta) = light_wireframe::coneangle_drag(
                            degrees,
                            drag.target.values.conedelta_degrees,
                        );
                        drag.edits.coneangle_degrees = Some(coneangle);
                        // spot_edits commits it only when it really moved.
                        drag.edits.conedelta_degrees = Some(conedelta);
                    }
                }
                light_wireframe::SpotHandle::Delta => {
                    if let Some(degrees) = light_wireframe::cone_angle_degrees(&frame, ray) {
                        // With the page's falloff closed at
                        // press, the pressed handle is the marker ring —
                        // `conedelta_complement` counts its own radius
                        // as the axis, so an unmoved press commits
                        // nothing.
                        drag.edits.conedelta_degrees =
                            Some(crate::editor::light_gizmo::conedelta_complement(
                                degrees,
                                coneangle,
                                coneangle - drag.target.values.conedelta_degrees,
                            ));
                    }
                }
                light_wireframe::SpotHandle::Tip => {
                    if let Some(to) = light_wireframe::tip_to(
                        &frame,
                        projector.forward(),
                        press_ray,
                        ray,
                        drag.light_from_world,
                    ) {
                        drag.edits.to = Some(to);
                    }
                }
            }
        }
        cx.notify();
    }

    /// Release: commit the drag as one edit set — one Apply, one Undo.
    /// `true` when a light drag had been in progress.
    pub(super) fn light_release(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.light_drag.take() else {
            return false;
        };
        cx.notify();
        let edits = drag.target.spot_edits(&drag.edits);
        if edits.is_empty() {
            return true;
        }
        let project = drag.target.target.project.clone();
        self.session.update(cx, |session, cx| {
            session.send(
                Command::Apply {
                    project,
                    edits,
                    follow: Follow::Nothing,
                    form: Some(crate::editor::light_gizmo::LIGHT_DRAG_FORM.into()),
                },
                cx,
            )
        });
        true
    }
}

/// The light's projected shapes as a paint list: `active` (the dragged
/// or hovered handle) takes the highlight colour; `dimmed` paints
/// the same colours at half opacity.
fn light_lines(
    shapes: Vec<(Option<light_wireframe::SpotHandle>, Shape)>,
    active: Option<light_wireframe::SpotHandle>,
    dimmed: bool,
    origin: Point<Pixels>,
    scale: f64,
) -> Vec<(Vec<Point<Pixels>>, bool, Hsla)> {
    let screen = |p: [f64; 2]| {
        point(
            origin.x + px((p[0] * scale) as f32),
            origin.y + px((p[1] * scale) as f32),
        )
    };
    let alpha = if dimmed { LIGHT_DIM_ALPHA } else { 0xff };
    shapes
        .into_iter()
        .map(|(handle, shape)| {
            let color: u32 = if handle.is_some_and(|handle| Some(handle) == active) {
                LIGHT_HIGHLIGHT
            } else {
                LIGHT_COLOUR
            };
            let color: Hsla = rgba((color << 8) | alpha).into();
            match shape {
                Shape::Line(a, b) => (vec![screen(a), screen(b)], false, color),
                // The light draws lines only; a fill here is a
                // programming slip the compiler cannot prove away.
                Shape::Fill(points) => (points.into_iter().map(screen).collect(), true, color),
            }
        })
        .collect()
}

/// Paint `shapes` (`gizmo_paint_list`'s or `light_paint_list`'s).
pub(super) fn paint(
    shapes: &[(Vec<Point<Pixels>>, bool, Hsla)],
    window: &mut Window,
    width: Pixels,
) {
    for (points, fill, color) in shapes {
        let mut path = if *fill {
            PathBuilder::fill()
        } else {
            PathBuilder::stroke(width)
        };
        let Some(first) = points.first() else {
            continue;
        };
        path.move_to(*first);
        for p in &points[1..] {
            path.line_to(*p);
        }
        if *fill {
            path.close();
        }
        if let Ok(path) = path.build() {
            window.paint_path(path, *color);
        }
    }
}

/// Whether a projected pivot lands inside the image: off it, no
/// handle can be seen or grabbed.
fn on_image(point: Option<[f64; 2]>, size: [u32; 2]) -> bool {
    point.is_some_and(|[x, y]| {
        (0.0..=f64::from(size[0])).contains(&x) && (0.0..=f64::from(size[1])).contains(&y)
    })
}

/// Return the hovered gizmo handle when `node` is the one the hover was
/// keyed on: the hover follows its node, so a newly selected node's
/// gizmo draws no handle highlighted until the pointer moves over the
/// viewport.
fn hover_for_node(hover: &Option<(String, Handle)>, node: &str) -> Option<Handle> {
    hover
        .as_ref()
        .filter(|(hovered, _)| hovered == node)
        .map(|(_, handle)| *handle)
}

#[cfg(test)]
mod tests {
    use super::{Handle, LIGHT_COLOUR, LIGHT_HIGHLIGHT, hover_for_node, light_lines, on_image};
    use gpui_kit::Hsla;
    use gpui_kit::{point, px, rgba};
    use scene_view::preview::gizmo::Shape;
    use scene_view::preview::light_gizmo::SpotHandle;

    fn alpha(hsla: &Hsla) -> f32 {
        hsla.a
    }

    /// While a transform-gizmo handle is hovered or a gizmo drag
    /// runs, the light keeps its colours but paints at half opacity; the
    /// light drag's own lines stay at full opacity.
    #[test]
    fn a_gizmo_interaction_dims_the_light_lines_without_changing_colours() {
        let line = |y: f64| Shape::Line([0., y], [10., y]);
        let shapes = vec![(Some(SpotHandle::Cone), line(0.)), (None, line(12.))];
        let origin = point(px(0.), px(0.));
        let colour = |color: u32, a: u32| rgba((color << 8) | a).into();

        // Plain painting: the light_colour, full opacity, and the active
        // handle takes the highlight.
        let lines = light_lines(shapes.clone(), None, false, origin, 1.);
        assert_eq!(lines[0].2, colour(LIGHT_COLOUR, 0xff));
        assert_eq!(lines[1].2, colour(LIGHT_COLOUR, 0xff));
        let lines = light_lines(shapes.clone(), Some(SpotHandle::Cone), false, origin, 1.);
        assert_eq!(lines[0].2, colour(LIGHT_HIGHLIGHT, 0xff));
        assert_eq!(lines[1].2, colour(LIGHT_COLOUR, 0xff));

        // Dimmed: same colours, half opacity (an active handle ride
        // along at the same half opacity).
        let lines = light_lines(shapes, Some(SpotHandle::Cone), true, origin, 1.);
        assert_eq!(alpha(&lines[0].2), alpha(&colour(LIGHT_HIGHLIGHT, 0x80)));
        assert_eq!(lines[1].2, colour(LIGHT_COLOUR, 0x80));
    }

    /// A gizmo handle's hover is keyed by the node it was hovered
    /// on, highlights only that node's gizmo — a selection change gives
    /// the newly selected node a clean gizmo until the pointer moves.
    #[test]
    fn a_gizmo_hover_highlights_only_the_node_it_was_hovered_on() {
        let hover = Some(("lamp".to_owned(), Handle::Axis(0)));
        assert_eq!(hover_for_node(&hover, "lamp"), Some(Handle::Axis(0)));
        assert_eq!(
            hover_for_node(&hover, "shade"),
            None,
            "a newly selected node does not inherit the hover"
        );
        assert_eq!(hover_for_node(&None, "lamp"), None);
    }

    #[test]
    fn a_pivot_is_on_the_image_only_inside_its_bounds() {
        let size = [640, 480];
        assert!(on_image(Some([320., 240.]), size));
        assert!(on_image(Some([0., 0.]), size), "the corner is on the image");
        assert!(on_image(Some([640., 480.]), size), "the far edge too");
        assert!(!on_image(Some([-1., 240.]), size), "left");
        assert!(!on_image(Some([641., 240.]), size), "right");
        assert!(!on_image(Some([320., -1.]), size), "above");
        assert!(!on_image(Some([320., 481.]), size), "below");
        assert!(!on_image(None, size), "behind the camera");
    }
}
