//! FX13: the Scene viewport's transform gizmo, without the GPU. Projection
//! shares the renderer's camera basis, so handles sit on the pixels the
//! preview draws; hit tests and drags work in image pixels.
use super::{ViewerCamera, cross, dot, norm, rotate_axis, sub};

/// The renderer's camera frame: pbrt's left-handed LookAt.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Basis {
    pub forward: [f64; 3],
    pub right: [f64; 3],
    pub up: [f64; 3],
    /// tan(fov_y / 2).
    pub ty: f64,
}
pub(crate) fn basis(camera: &ViewerCamera) -> Option<Basis> {
    let forward = norm(sub(camera.target, camera.eye))?;
    // camera-right is up × forward (pbrt's LookAt); a right-handed scene's
    // camera mirrors it, as its `Scale -1 1 1` does in the render.
    let right = norm(cross(camera.up, forward))?;
    let up = cross(forward, right);
    let right = if camera.mirror {
        right.map(|v| -v)
    } else {
        right
    };
    let ty = (camera.fov_y_degrees.to_radians() / 2.).tan();
    (ty.is_finite() && ty > 0.).then_some(Basis {
        forward,
        right,
        up,
        ty,
    })
}

/// World points to image pixels (top-left origin) and back to rays.
#[derive(Clone, Copy, Debug)]
pub struct Projector {
    eye: [f64; 3],
    basis: Basis,
    tx: f64,
    size: [f64; 2],
}
impl Projector {
    pub fn new(camera: &ViewerCamera, width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }
        let basis = basis(camera)?;
        Some(Self {
            eye: camera.eye,
            tx: width as f64 / height as f64 * basis.ty,
            basis,
            size: [width as f64, height as f64],
        })
    }
    /// The pixel `p` lands on; `None` behind the camera.
    pub fn project(&self, p: [f64; 3]) -> Option<[f64; 2]> {
        let v = sub(p, self.eye);
        let depth = dot(v, self.basis.forward);
        if depth <= 1e-9 {
            return None;
        }
        let x = dot(v, self.basis.right) / (self.tx * depth);
        let y = dot(v, self.basis.up) / (self.basis.ty * depth);
        Some([(x + 1.) / 2. * self.size[0], (1. - y) / 2. * self.size[1]])
    }
    /// The eye ray through a pixel: (origin, unit direction).
    pub fn ray(&self, xy: [f64; 2]) -> ([f64; 3], [f64; 3]) {
        let x = (xy[0] / self.size[0] * 2. - 1.) * self.tx;
        let y = (1. - xy[1] / self.size[1] * 2.) * self.basis.ty;
        let b = &self.basis;
        let d = [
            b.forward[0] + b.right[0] * x + b.up[0] * y,
            b.forward[1] + b.right[1] * x + b.up[1] * y,
            b.forward[2] + b.right[2] * x + b.up[2] * y,
        ];
        (self.eye, norm(d).unwrap_or(b.forward))
    }
    /// World length of one pixel at `p`'s depth.
    pub fn pixel_size(&self, p: [f64; 3]) -> f64 {
        let depth = dot(sub(p, self.eye), self.basis.forward).max(1e-9);
        2. * self.basis.ty * depth / self.size[1]
    }
    pub fn forward(&self) -> [f64; 3] {
        self.basis.forward
    }
}

/// Which transform the gizmo edits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// No handles (Unity's Q): selection and camera only.
    None,
    Move,
    Rotate,
    Scale,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Handle {
    /// Move or Scale along axis 0 / 1 / 2; Rotate about it (its ring).
    Axis(usize),
    /// Move in the plane of the other two axes.
    Plane(usize),
    /// Move in the view plane, Rotate about the view axis, Scale uniformly.
    Screen,
}

/// What a drag does: a world translation, a world rotation about `axis`
/// through the origin, or factors along the gizmo's axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Drag {
    Move([f64; 3]),
    Rotate { axis: [f64; 3], degrees: f64 },
    Scale([f64; 3]),
}
impl Drag {
    pub fn is_identity(&self) -> bool {
        match *self {
            Drag::Move(d) => d == [0.; 3],
            Drag::Rotate { degrees, .. } => degrees == 0.,
            Drag::Scale(f) => f == [1.; 3],
        }
    }
}

/// What to draw, in image pixels.
#[derive(Clone, Debug, PartialEq)]
pub enum Shape {
    Line([f64; 2], [f64; 2]),
    /// A filled polygon (arrow heads, plane squares, the centre square).
    Fill(Vec<[f64; 2]>),
}

/// A gizmo at `origin` along three unit `axes`, `length` pixels long.
#[derive(Clone, Copy, Debug)]
pub struct Gizmo {
    pub tool: Tool,
    pub origin: [f64; 3],
    pub axes: [[f64; 3]; 3],
    pub length: f64,
}
/// Axes closer to the view direction than this cannot be dragged.
const PARALLEL: f64 = 0.985;
/// Hit distance in pixels (the light wire-frame shares it).
pub(super) const TOLERANCE: f64 = 7.;
const RING_SEGMENTS: usize = 64;
/// Ctrl-drag steps: world units, degrees and scale factor.
const SNAP_MOVE: f64 = 0.1;
const SNAP_DEGREES: f64 = 15.;
const SNAP_SCALE: f64 = 0.1;
fn snap_value(value: f64, step: f64) -> f64 {
    (value / step).round() * step
}
fn square(c: [f64; 2], r: f64) -> Shape {
    Shape::Fill(vec![
        [c[0] - r, c[1] - r],
        [c[0] + r, c[1] - r],
        [c[0] + r, c[1] + r],
        [c[0] - r, c[1] + r],
    ])
}
fn cross2(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[1] - a[1] * b[0]
}
fn minus2(a: [f64; 2], b: [f64; 2]) -> [f64; 2] {
    [a[0] - b[0], a[1] - b[1]]
}
impl Gizmo {
    fn at(&self, axis: usize, t: f64, projector: &Projector) -> [f64; 3] {
        let l = t * self.length * projector.pixel_size(self.origin);
        add(self.origin, scale(self.axes[axis], l))
    }
    /// The normal of plane handle `a`: across the two axes it is drawn
    /// along, which need not be perpendicular to axis `a` under a shear.
    fn plane_normal(&self, a: usize) -> Option<[f64; 3]> {
        norm(cross(self.axes[(a + 1) % 3], self.axes[(a + 2) % 3]))
    }
    /// The axis a Rotate handle turns about.
    fn rotation_axis(&self, handle: Handle, projector: &Projector) -> [f64; 3] {
        match handle {
            Handle::Axis(a) => self.axes[a],
            _ => projector.forward(),
        }
    }
    /// Whether the handle can be dragged from this view.
    pub fn usable(&self, handle: Handle, projector: &Projector) -> bool {
        let f = projector.forward();
        match (self.tool, handle) {
            (Tool::Move, Handle::Plane(a)) => self
                .plane_normal(a)
                .is_some_and(|n| dot(n, f).abs() > 1. - PARALLEL),
            (_, Handle::Plane(_)) => false,
            // A ring seen edge-on gives no angle.
            (Tool::Rotate, Handle::Axis(a)) => dot(self.axes[a], f).abs() > 1. - PARALLEL,
            (_, Handle::Axis(a)) => dot(self.axes[a], f).abs() < PARALLEL,
            (_, Handle::Screen) => true,
        }
    }
    /// The handles' shapes, each with what it drags.
    pub fn shapes(&self, projector: &Projector) -> Vec<(Handle, Shape)> {
        let mut shapes = Vec::new();
        let Some(o) = projector.project(self.origin) else {
            return shapes;
        };
        match self.tool {
            Tool::None => {}
            Tool::Move => {
                self.plane_shapes(projector, &mut shapes);
                self.axis_shapes(o, projector, true, &mut shapes);
                shapes.push((Handle::Screen, square(o, 6.)));
            }
            Tool::Scale => {
                self.axis_shapes(o, projector, false, &mut shapes);
                shapes.push((Handle::Screen, square(o, 6.)));
            }
            Tool::Rotate => self.ring_shapes(o, projector, &mut shapes),
        }
        shapes
    }
    fn plane_shapes(&self, projector: &Projector, shapes: &mut Vec<(Handle, Shape)>) {
        for a in 0..3 {
            let handle = Handle::Plane(a);
            if !self.usable(handle, projector) {
                continue;
            }
            let (u, v) = ((a + 1) % 3, (a + 2) % 3);
            let corner = |s: f64, t: f64| {
                let p = add(
                    self.at(u, s, projector),
                    sub(self.at(v, t, projector), self.origin),
                );
                projector.project(p)
            };
            let quad = [
                corner(0.2, 0.2),
                corner(0.4, 0.2),
                corner(0.4, 0.4),
                corner(0.2, 0.4),
            ];
            if let [Some(a), Some(b), Some(c), Some(d)] = quad {
                shapes.push((handle, Shape::Fill(vec![a, b, c, d])));
            }
        }
    }
    /// Axis lines ending in an arrow head (Move) or a square (Scale).
    fn axis_shapes(
        &self,
        o: [f64; 2],
        projector: &Projector,
        arrow: bool,
        shapes: &mut Vec<(Handle, Shape)>,
    ) {
        for a in 0..3 {
            let handle = Handle::Axis(a);
            if !self.usable(handle, projector) {
                continue;
            }
            let Some(tip) = projector.project(self.at(a, 1., projector)) else {
                continue;
            };
            shapes.push((handle, Shape::Line(o, tip)));
            if !arrow {
                shapes.push((handle, square(tip, 5.)));
                continue;
            }
            let d = [tip[0] - o[0], tip[1] - o[1]];
            let n = (d[0] * d[0] + d[1] * d[1]).sqrt().max(1e-9);
            let (dx, dy) = (d[0] / n, d[1] / n);
            let base = [tip[0] - dx * 12., tip[1] - dy * 12.];
            shapes.push((
                handle,
                Shape::Fill(vec![
                    [tip[0] + dx * 2., tip[1] + dy * 2.],
                    [base[0] - dy * 5., base[1] + dx * 5.],
                    [base[0] + dy * 5., base[1] - dx * 5.],
                ]),
            ));
        }
    }
    /// A ring per axis (its front half) and the view ring around them.
    fn ring_shapes(&self, o: [f64; 2], projector: &Projector, shapes: &mut Vec<(Handle, Shape)>) {
        let r = self.length * projector.pixel_size(self.origin);
        let f = projector.forward();
        let step = std::f64::consts::TAU / RING_SEGMENTS as f64;
        for a in 0..3 {
            let handle = Handle::Axis(a);
            if !self.usable(handle, projector) {
                continue;
            }
            let Some((u, v)) = perpendiculars(self.axes[a]) else {
                continue;
            };
            let offset = |k: usize| {
                let t = k as f64 * step;
                add(scale(u, r * t.cos()), scale(v, r * t.sin()))
            };
            for k in 0..RING_SEGMENTS {
                let (p, q) = (offset(k), offset(k + 1));
                // The back half hides behind the object, as in other editors.
                if dot(add(p, q), f) > 0.1 * r {
                    continue;
                }
                if let (Some(p), Some(q)) = (
                    projector.project(add(self.origin, p)),
                    projector.project(add(self.origin, q)),
                ) {
                    shapes.push((handle, Shape::Line(p, q)));
                }
            }
        }
        let r = self.length * 1.15;
        let at = |k: usize| {
            let t = k as f64 * step;
            [o[0] + r * t.cos(), o[1] + r * t.sin()]
        };
        for k in 0..RING_SEGMENTS {
            shapes.push((Handle::Screen, Shape::Line(at(k), at(k + 1))));
        }
    }
    /// The handle under a pixel: a filled shape (the centre, then planes,
    /// then axis tips), else the nearest line within the tolerance.
    pub fn hit(&self, projector: &Projector, xy: [f64; 2]) -> Option<Handle> {
        let shapes = self.shapes(projector);
        let rank = |handle: &Handle| match handle {
            Handle::Screen => 0,
            Handle::Plane(_) => 1,
            Handle::Axis(_) => 2,
        };
        let filled = shapes
            .iter()
            .filter(|(_, shape)| matches!(shape, Shape::Fill(points) if contains(points, xy)))
            .map(|(handle, _)| *handle)
            .min_by_key(rank);
        filled.or_else(|| {
            shapes
                .iter()
                .filter_map(|(handle, shape)| match shape {
                    Shape::Line(a, b) => Some((segment_distance(*a, *b, xy), *handle)),
                    Shape::Fill(_) => None,
                })
                .filter(|(distance, _)| *distance <= TOLERANCE)
                .min_by(|a, b| a.0.total_cmp(&b.0))
                .map(|(_, handle)| handle)
        })
    }
    /// What dragging `handle` from pixel `from` to `to` does; `snap` rounds
    /// it to the Ctrl steps.
    pub fn drag(
        &self,
        projector: &Projector,
        handle: Handle,
        from: [f64; 2],
        to: [f64; 2],
        snap: bool,
    ) -> Option<Drag> {
        if !self.usable(handle, projector) {
            return None;
        }
        let drag = match self.tool {
            Tool::None => return None,
            Tool::Move => {
                let delta = sub(
                    self.move_point(projector, handle, to)?,
                    self.move_point(projector, handle, from)?,
                );
                Drag::Move(if snap {
                    self.snap_move(handle, delta)
                } else {
                    delta
                })
            }
            Tool::Rotate => {
                let axis = self.rotation_axis(handle, projector);
                // shortcut: the angle is measured from the press point, so a
                // drag turns at most ±180°; accumulate it if that matters.
                let degrees = match handle {
                    // An axis ring is an ellipse when seen obliquely, so its
                    // screen angle is not the turn about the axis; measure the
                    // pointer rays in the ring's own plane instead.
                    Handle::Axis(_) => {
                        let (fo, fd) = projector.ray(from);
                        let (to_o, td) = projector.ray(to);
                        let pa = plane_ray(self.origin, axis, fo, fd)?;
                        let pb = plane_ray(self.origin, axis, to_o, td)?;
                        if dot(pa, pa) < 1e-12 || dot(pb, pb) < 1e-12 {
                            return None;
                        }
                        dot(cross(pa, pb), axis).atan2(dot(pa, pb)).to_degrees()
                    }
                    // The view ring faces the camera, so its plane is the
                    // screen: the screen angle (with its sign) is exact.
                    _ => {
                        let c = projector.project(self.origin)?;
                        let (a, b) = (minus2(from, c), minus2(to, c));
                        if a[0].hypot(a[1]) < 1. || b[0].hypot(b[1]) < 1. {
                            return None;
                        }
                        let screen = cross2(a, b).atan2(a[0] * b[0] + a[1] * b[1]);
                        screen.to_degrees() * self.screen_sign(axis, projector)?
                    }
                };
                Drag::Rotate {
                    axis,
                    degrees: if snap {
                        snap_value(degrees, SNAP_DEGREES)
                    } else {
                        degrees
                    },
                }
            }
            Tool::Scale => {
                let factor = match handle {
                    Handle::Axis(a) => {
                        let c = projector.project(self.origin)?;
                        let dir = minus2(projector.project(self.at(a, 1., projector))?, c);
                        let at = |p: [f64; 2]| {
                            let d = minus2(p, c);
                            d[0] * dir[0] + d[1] * dir[1]
                        };
                        let base = at(from);
                        if base.abs() < 1. {
                            return None;
                        }
                        at(to) / base
                    }
                    // Right or up grows, left or down shrinks.
                    _ => 1. + ((to[0] - from[0]) - (to[1] - from[1])) / self.length,
                };
                let factor = if snap {
                    snap_value(factor, SNAP_SCALE)
                } else {
                    factor
                }
                .max(0.01);
                Drag::Scale(match handle {
                    Handle::Axis(a) => std::array::from_fn(|i| if i == a { factor } else { 1. }),
                    _ => [factor; 3],
                })
            }
        };
        let finite = match drag {
            Drag::Move(v) | Drag::Scale(v) => v.iter().all(|v| v.is_finite()),
            Drag::Rotate { degrees, .. } => degrees.is_finite(),
        };
        finite.then_some(drag)
    }
    fn move_point(&self, projector: &Projector, handle: Handle, xy: [f64; 2]) -> Option<[f64; 3]> {
        let (o, d) = projector.ray(xy);
        match handle {
            Handle::Axis(a) => {
                let t = line_ray(self.origin, self.axes[a], o, d)?;
                Some(scale(self.axes[a], t))
            }
            Handle::Plane(a) => plane_ray(self.origin, self.plane_normal(a)?, o, d),
            Handle::Screen => plane_ray(self.origin, projector.forward(), o, d),
        }
    }
    /// Round a move to `SNAP_MOVE` along the handle's own axes.
    fn snap_move(&self, handle: Handle, delta: [f64; 3]) -> [f64; 3] {
        match handle {
            Handle::Axis(a) => scale(
                self.axes[a],
                snap_value(dot(delta, self.axes[a]), SNAP_MOVE),
            ),
            Handle::Plane(a) => {
                // delta = α·u + β·v, solved exactly so a shear snaps too.
                let (u, v) = (self.axes[(a + 1) % 3], self.axes[(a + 2) % 3]);
                let (uu, uv, vv) = (dot(u, u), dot(u, v), dot(v, v));
                let (du, dv) = (dot(delta, u), dot(delta, v));
                let det = uu * vv - uv * uv;
                if det.abs() < 1e-12 {
                    return delta;
                }
                let alpha = snap_value((du * vv - dv * uv) / det, SNAP_MOVE);
                let beta = snap_value((dv * uu - du * uv) / det, SNAP_MOVE);
                add(scale(u, alpha), scale(v, beta))
            }
            Handle::Screen => delta.map(|v| snap_value(v, SNAP_MOVE)),
        }
    }
    /// +1 when a positive turn about `axis` runs the way positive screen
    /// angles do (image y points down), -1 the other way.
    fn screen_sign(&self, axis: [f64; 3], projector: &Projector) -> Option<f64> {
        let (u, _) = perpendiculars(axis)?;
        let r = self.length * projector.pixel_size(self.origin);
        let c = projector.project(self.origin)?;
        let (sin, cos) = 5f64.to_radians().sin_cos();
        let p = projector.project(add(self.origin, scale(u, r)))?;
        let q = projector.project(add(self.origin, scale(rotate_axis(u, axis, sin, cos), r)))?;
        let turn = cross2(minus2(p, c), minus2(q, c));
        (turn.abs() > 1e-6).then(|| turn.signum())
    }
}

/// Two unit vectors perpendicular to `n` and to each other.
pub(super) fn perpendiculars(n: [f64; 3]) -> Option<([f64; 3], [f64; 3])> {
    let other = if n[0].abs() < 0.9 {
        [1., 0., 0.]
    } else {
        [0., 1., 0.]
    };
    let u = norm(cross(n, other))?;
    Some((u, cross(n, u)))
}
fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
/// Parameter along the line `p + t·a` (unit `a`) closest to the ray.
fn line_ray(p: [f64; 3], a: [f64; 3], o: [f64; 3], d: [f64; 3]) -> Option<f64> {
    let w = sub(o, p);
    let b = dot(a, d);
    let denominator = 1. - b * b;
    (denominator > 1e-9).then(|| (dot(w, a) - dot(w, d) * b) / denominator)
}
/// Where the ray meets the plane through `p` with normal `n`, relative to `p`.
fn plane_ray(p: [f64; 3], n: [f64; 3], o: [f64; 3], d: [f64; 3]) -> Option<[f64; 3]> {
    let denominator = dot(n, d);
    if denominator.abs() < 1e-6 {
        return None;
    }
    let s = dot(sub(p, o), n) / denominator;
    (s > 0.).then(|| sub(add(o, scale(d, s)), p))
}
pub(super) fn segment_distance(a: [f64; 2], b: [f64; 2], p: [f64; 2]) -> f64 {
    let ab = [b[0] - a[0], b[1] - a[1]];
    let length = ab[0] * ab[0] + ab[1] * ab[1];
    let t = if length > 0. {
        (((p[0] - a[0]) * ab[0] + (p[1] - a[1]) * ab[1]) / length).clamp(0., 1.)
    } else {
        0.
    };
    let q = [a[0] + ab[0] * t - p[0], a[1] + ab[1] * t - p[1]];
    (q[0] * q[0] + q[1] * q[1]).sqrt()
}
/// Even-odd point in polygon.
fn contains(points: &[[f64; 2]], p: [f64; 2]) -> bool {
    let mut inside = false;
    let mut j = points.len().wrapping_sub(1);
    for i in 0..points.len() {
        let (a, b) = (points[i], points[j]);
        if (a[1] > p[1]) != (b[1] > p[1])
            && p[0] < (b[0] - a[0]) * (p[1] - a[1]) / (b[1] - a[1]) + a[0]
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

#[cfg(test)]
mod tests {
    use super::*;

    fn camera() -> ViewerCamera {
        ViewerCamera {
            eye: [0., 0., -10.],
            target: [0., 0., 0.],
            up: [0., 1., 0.],
            fov_y_degrees: 45.,
            near: 0.01,
            far: 100.,
            mirror: false,
        }
    }
    fn gizmo() -> Gizmo {
        Gizmo {
            tool: Tool::Move,
            origin: [0.; 3],
            axes: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            length: 90.,
        }
    }
    fn moved(drag: Drag) -> [f64; 3] {
        match drag {
            Drag::Move(d) => d,
            other => panic!("expected a move, got {other:?}"),
        }
    }

    #[test]
    fn projection_and_rays_invert_each_other() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        assert_eq!(projector.project([0.; 3]), Some([320., 240.]));
        // pbrt's left-handed LookAt: +x is to the right, +y up.
        let right = projector.project([1., 0., 0.]).unwrap();
        let up = projector.project([0., 1., 0.]).unwrap();
        assert!(right[0] > 320. && (right[1] - 240.).abs() < 1e-9);
        assert!(up[1] < 240.);
        assert!(projector.project([0., 0., -20.]).is_none());
        let p = [1.5, -0.75, 3.];
        let xy = projector.project(p).unwrap();
        let (o, d) = projector.ray(xy);
        let t = dot(sub(p, o), d);
        let closest = add(o, scale(d, t));
        assert!(dot(sub(closest, p), sub(closest, p)).sqrt() < 1e-9);
    }

    #[test]
    fn axis_drags_follow_the_axis_under_the_pointer() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = gizmo();
        let tip = projector.project([2., 0., 0.]).unwrap();
        let delta = moved(
            g.drag(&projector, Handle::Axis(0), [320., 240.], tip, false)
                .unwrap(),
        );
        assert!((delta[0] - 2.).abs() < 1e-9 && delta[1] == 0. && delta[2] == 0.);
        // Moving off the axis stays on it (perspective shifts the amount a
        // little).
        let off = [tip[0], tip[1] - 50.];
        let delta = moved(
            g.drag(&projector, Handle::Axis(0), [320., 240.], off, false)
                .unwrap(),
        );
        assert!((delta[0] - 2.).abs() < 0.05 && delta[1] == 0. && delta[2] == 0.);
        // The plane normal to z follows the pointer in x and y.
        let p = projector.project([1., 1., 0.]).unwrap();
        let delta = moved(
            g.drag(&projector, Handle::Plane(2), [320., 240.], p, false)
                .unwrap(),
        );
        assert!((delta[0] - 1.).abs() < 1e-9 && (delta[1] - 1.).abs() < 1e-9);
        assert!(delta[2].abs() < 1e-9);
    }

    #[test]
    fn a_sheared_plane_drag_stays_in_the_drawn_plane() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        // z leans toward x; the XY square is still spanned by x and y.
        let l = 1.25f64.sqrt();
        let g = Gizmo {
            axes: [[1., 0., 0.], [0., 1., 0.], [0.5 / l, 0., 1. / l]],
            ..gizmo()
        };
        let p = projector.project([1., 1., 0.]).unwrap();
        let delta = moved(
            g.drag(&projector, Handle::Plane(2), [320., 240.], p, false)
                .unwrap(),
        );
        assert!((delta[0] - 1.).abs() < 1e-9 && (delta[1] - 1.).abs() < 1e-9);
        assert!(delta[2].abs() < 1e-9);
    }

    #[test]
    fn a_snapped_move_rounds_to_the_grid() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = gizmo();
        let to = projector.project([0.736, 0., 0.]).unwrap();
        let delta = moved(
            g.drag(&projector, Handle::Axis(0), [320., 240.], to, true)
                .unwrap(),
        );
        assert!((delta[0] - 0.7).abs() < 1e-9 && delta[1] == 0. && delta[2] == 0.);
    }

    #[test]
    fn an_axis_along_the_view_cannot_be_dragged_or_hit() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = gizmo();
        assert!(!g.usable(Handle::Axis(2), &projector));
        assert!(
            g.drag(&projector, Handle::Axis(2), [0., 0.], [1., 1.], false)
                .is_none()
        );
        assert!(
            g.shapes(&projector)
                .iter()
                .all(|(h, _)| *h != Handle::Axis(2))
        );
        // Facing planes stay usable; edge-on ones do not.
        assert!(g.usable(Handle::Plane(2), &projector));
        assert!(!g.usable(Handle::Plane(0), &projector));
    }

    #[test]
    fn hits_pick_the_centre_then_planes_then_axes() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = gizmo();
        assert_eq!(g.hit(&projector, [321., 239.]), Some(Handle::Screen));
        // 90 px long: the x axis runs right of the centre, y above it.
        assert_eq!(g.hit(&projector, [380., 242.]), Some(Handle::Axis(0)));
        assert_eq!(g.hit(&projector, [318., 180.]), Some(Handle::Axis(1)));
        // The xy square sits between 0.2 and 0.4 of the length.
        assert_eq!(g.hit(&projector, [347., 213.]), Some(Handle::Plane(2)));
        assert_eq!(g.hit(&projector, [200., 400.]), None);
    }

    #[test]
    fn a_rotate_drag_turns_about_the_axis_and_snaps() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = Gizmo {
            tool: Tool::Rotate,
            ..gizmo()
        };
        // About +z (facing the camera, which looks along +z): a quarter turn
        // of the pointer around the centre.
        let c = projector.project([0.; 3]).unwrap();
        let from = [c[0] + 50., c[1]];
        let to = [c[0], c[1] + 50.];
        let Drag::Rotate { axis, degrees } = g
            .drag(&projector, Handle::Axis(2), from, to, false)
            .unwrap()
        else {
            panic!("expected a rotation");
        };
        assert_eq!(axis, [0., 0., 1.]);
        assert!((degrees.abs() - 90.).abs() < 1e-6);
        let sign = degrees.signum();
        // Snapping rounds a 20° drag to 15°.
        let to = [
            c[0] + 50. * 20f64.to_radians().cos(),
            c[1] - sign * 50. * 20f64.to_radians().sin(),
        ];
        let Drag::Rotate { degrees, .. } =
            g.drag(&projector, Handle::Axis(2), from, to, true).unwrap()
        else {
            panic!("expected a rotation");
        };
        assert!((degrees - sign * 15.).abs() < 1e-6, "got {degrees}");
    }

    #[test]
    fn a_scale_axis_drag_grows_only_its_axis() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let g = Gizmo {
            tool: Tool::Scale,
            ..gizmo()
        };
        let c = projector.project([0.; 3]).unwrap();
        let tip = projector.project(g.at(0, 1., &projector)).unwrap();
        // Dragging the x square out to twice its distance doubles x alone.
        let to = [c[0] + (tip[0] - c[0]) * 2., c[1] + (tip[1] - c[1]) * 2.];
        let Drag::Scale(f) = g.drag(&projector, Handle::Axis(0), tip, to, false).unwrap() else {
            panic!("expected a scale");
        };
        assert!((f[0] - 2.).abs() < 1e-6 && f[1] == 1. && f[2] == 1.);
        // The centre scales uniformly.
        let Drag::Scale(f) = g
            .drag(&projector, Handle::Screen, c, [c[0] + 90., c[1]], false)
            .unwrap()
        else {
            panic!("expected a scale");
        };
        assert!((f[0] - 2.).abs() < 1e-6 && f[0] == f[1] && f[1] == f[2]);
    }

    #[test]
    fn an_oblique_ring_measures_the_true_angle_not_its_ellipse() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        // A ring tilted 60° from the view; its projection is a narrow ellipse.
        let axis = [0., 3f64.sqrt() / 2., 0.5];
        let g = Gizmo {
            tool: Tool::Rotate,
            axes: [axis, [1., 0., 0.], [0., 0., 1.]],
            ..gizmo()
        };
        let (u, _) = perpendiculars(axis).unwrap();
        let r = 2.;
        let (sin, cos) = 45f64.to_radians().sin_cos();
        let from = projector.project(add([0.; 3], scale(u, r))).unwrap();
        let to = projector
            .project(add([0.; 3], scale(rotate_axis(u, axis, sin, cos), r)))
            .unwrap();
        let Drag::Rotate { degrees, .. } = g
            .drag(&projector, Handle::Axis(0), from, to, false)
            .unwrap()
        else {
            panic!("expected a rotation");
        };
        // Measured in the ring plane it is a clean 45°, not the ~26.6° the
        // screen-space angle of the ellipse would give.
        assert!((degrees - 45.).abs() < 1e-6, "got {degrees}");
    }
}
