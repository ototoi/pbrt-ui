//! The wire-frame of the one selected
//! light, drawn over the Scene viewport like the transform gizmo. The
//! shapes are pure world-space line segments — the light's implementation
//! and parameter values under its world transform — so the desktop only
//! projects and paints them. Area lights draw nothing (their shape is
//! the light) and neither does an infinite light (all of space).
//!
//! pbrt-v4 parameter semantics (src/pbrt/lights.cpp): point reads `from`;
//! spot reads `from`/`to` with `coneangle` default 30 and `conedelta`
//! (pbrt spells it `conedeltaangle`) default 5; distant reads
//! `from`/`to` default (0,0,0)→(0,0,1) and shines along `to - from`;
//! goniometric and projection look along the light's +z. All under the
//! light's world transform.
use super::gizmo::{Projector, Shape, perpendiculars, segment_distance};
use super::{dot, norm, sub};
use serde_json::Value;

/// Handle budget in pixels, like the gizmo's `LENGTH`: the spot's rings sit
/// this far along the axis, and a +z-looker's arrow is this long. A spot's
/// and a distant light's arrow runs to `SPOT_DEPTH + ARROW_CLEARANCE`.
/// `world_per_pixel` converts all of these at the light's own
/// screen distance, so the wire-frame keeps its on-screen size.
const SPOT_DEPTH: f64 = 90.;
/// How far past the rim plane a spot's or a distant light's arrow head
/// reaches in the handle budget's pixels: the head stands
/// clear of the cone's circles — and of the Move gizmo's centre square
/// at the apex, where the shaft stays untagged.
const ARROW_CLEARANCE: f64 = 22.;
/// The inner 60% of a drawn shaft, measured from its origin, is untagged,
/// so the Tip handle starts 40% out. Near the origin, the Move gizmo's
/// handles win anyway.
const SHAFT_UNTAGGED_PART: f64 = 0.6;
/// A point light's rings are this small, and a chevron is this long.
const POINT_RADIUS: f64 = 12.;
/// A chevron's spread relative to its length.
const WING: f64 = 0.66;
const POINT_SEGMENTS: usize = 32;
const RING_SEGMENTS: usize = 64;
/// coneangle's smallest sensible opening (degrees): a cone must show an
/// opening at all.
const MIN_CONEANGLE_DEGREES: f64 = 0.1;
/// The falloff marker drawn around the axis when there is no inner
/// circle: this many budget pixels in radius, with this many segments —
/// a small ring in the rim plane carrying the Delta handle.
const DELTA_MARKER_RADIUS: f64 = 7.;
const DELTA_MARKER_SEGMENTS: usize = 32;

/// The marker's own half-angle as `cone_angle_degrees` measures it
/// Both the marker's radius and the rim's distance convert at
/// the same screen-constant budget, so `atan(radius / SPOT_DEPTH)` never
/// moves with the view or the light.
fn delta_marker_angle_degrees() -> f64 {
    (DELTA_MARKER_RADIUS / SPOT_DEPTH).atan().to_degrees()
}

/// An implementation that draws a wire-frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LightKind {
    Point,
    Spot,
    Distant,
    /// Both look along the light's +z; both draw a direction arrow.
    Goniometric,
    Projection,
}

/// What dragging one drawn handle edits: a spot's outer
/// cone rim sets `coneangle`, its inner rim `conedelta`, and a spot's or
/// a distant light's axis arrow (its end is the tip) moves `to`. Every
/// other light draws untagged display lines only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpotHandle {
    /// The outer circle and the four apex lines → `coneangle`.
    Cone,
    /// The inner falloff circle → `conedelta`.
    Delta,
    /// The axis arrow (its end is the tip) → direction, by moving `to`.
    Tip,
}

/// A 3D world-space line segment.
pub type Segment = ([f64; 3], [f64; 3]);

/// The parameters the wire-frame reads, with pbrt-v4's defaults applied to
/// the missing ones.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LightValues {
    pub kind: LightKind,
    pub from: [f64; 3],
    pub to: [f64; 3],
    pub coneangle_degrees: f64,
    pub conedelta_degrees: f64,
}

/// The `LightKind` of an implementation name, or `None` when it draws
/// nothing (area, infinite, unknown). pbrt spells its implementations in
/// lower case and so does this table.
pub fn light_kind(implementation: &str) -> Option<LightKind> {
    Some(match implementation {
        "point" => LightKind::Point,
        "spot" => LightKind::Spot,
        "distant" => LightKind::Distant,
        "goniometric" => LightKind::Goniometric,
        "projection" => LightKind::Projection,
        _ => return None,
    })
}

/// The `[from, to, coneangle, conedelta]` the drawing needs, from the
/// Inspector's declared-parameter page `{"parameters":[...]}` (each row
/// `{"name","value_type","values":{"kind","value":[...]}}`); missing
/// parameters take pbrt-v4's defaults. `None` when the kind draws
/// nothing.
pub fn light_values(implementation: &str, page: &Value) -> Option<LightValues> {
    let kind = light_kind(implementation)?;
    let rows = page["parameters"].as_array().cloned().unwrap_or_default();
    // pbrt-v4 lights.cpp: from (0,0,0), to (0,0,1), coneangle 30,
    // conedeltaangle (the table's `conedelta`) 5.
    Some(LightValues {
        kind,
        from: declared_point3(&rows, &["from"], [0., 0., 0.]),
        to: declared_point3(&rows, &["to"], [0., 0., 1.]),
        coneangle_degrees: declared_scalar(&rows, &["coneangle"], 30.),
        conedelta_degrees: declared_scalar(&rows, &["conedelta", "conedeltaangle"], 5.),
    })
}

/// One value of a row's floats: a bare number, or the core's encoded
/// `{"kind":"finite","value":x}` spelling of a `FloatValue`.
fn number(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value["value"].as_f64())
        .filter(|v| v.is_finite())
}

/// The row's floats; non-finite values never reach a wire-frame.
fn row_floats(row: &Value) -> Vec<f64> {
    row["values"]["value"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(number)
        .collect()
}

fn declared_point3(rows: &[Value], names: &[&str], default: [f64; 3]) -> [f64; 3] {
    rows.iter()
        .filter(|row| is_row(row, names))
        .find_map(|row| match row_floats(row)[..] {
            // A row that does not carry three floats (a truncated preview,
            // an odd declaration) never reaches the values; a later row
            // of the same name may answer instead.
            [x, y, z, ..] => Some([x, y, z]),
            _ => None,
        })
        .unwrap_or(default)
}

fn declared_scalar(rows: &[Value], names: &[&str], default: f64) -> f64 {
    rows.iter()
        .filter(|row| is_row(row, names))
        .find_map(|row| row_floats(row).first().copied())
        .unwrap_or(default)
}

/// Whether `row` is one of `names` (pbrt's case-insensitive match).
fn is_row(row: &Value, names: &[&str]) -> bool {
    row["name"]
        .as_str()
        .is_some_and(|name| names.iter().any(|want| name.eq_ignore_ascii_case(want)))
}

/// The wire-frame as world-space segments: `implementation + parameter
/// values + world matrix`, sizes screen-constant through
/// `world_per_pixel` (the world length of one budget pixel at the
/// light). Each segment carries the spot handle it belongs to, when it
/// has one.
pub fn world_segments(
    values: &LightValues,
    world_from_light: [f64; 16],
    world_per_pixel: f64,
) -> Vec<(Option<SpotHandle>, Segment)> {
    let mut segments = Vec::new();
    match values.kind {
        // Three small circles, one per axis plane, around world `from`.
        LightKind::Point => {
            let apex = transform_point(world_from_light, values.from);
            let axes = world_axes(world_from_light);
            for normal in 0..3 {
                let (u, v) = (axes[(normal + 1) % 3], axes[(normal + 2) % 3]);
                ring(
                    apex,
                    u,
                    v,
                    POINT_RADIUS * world_per_pixel,
                    None,
                    POINT_SEGMENTS,
                    &mut segments,
                );
            }
        }
        // An arrow from `from` toward `to`, the cone's two rim circles at
        // a screen-constant distance along the axis, and four lines from
        // the apex to the outer circle.
        LightKind::Spot => {
            let Some((apex, axis)) = spot_frame_parts(values, world_from_light) else {
                return segments;
            };
            let distance = SPOT_DEPTH * world_per_pixel;
            arrow(
                apex,
                axis,
                arrow_distance(distance),
                world_per_pixel,
                Some(SpotHandle::Tip),
                &mut segments,
            );
            let Some((u, v)) = perpendiculars(axis) else {
                return segments;
            };
            let centre = add(apex, scale(axis, distance));
            // The drawn half-angle caps just under 90°: an exactly-90
            // cone (an allowed clamp value) would put this tan() at
            // 1.6e16 world units out. The committed value is not
            // capped — only what is drawn.
            let drawn = values.coneangle_degrees.min(89.9);
            let outer = distance * drawn.to_radians().tan();
            ring(
                centre,
                u,
                v,
                outer,
                Some(SpotHandle::Cone),
                RING_SEGMENTS,
                &mut segments,
            );
            // pbrt's falloff starts at `coneangle - conedelta`; a
            // non-positive half-angle draws no inner circle but a small
            // marker ring around the axis instead, so the falloff stays
            // grabbable.
            let falloff = values.coneangle_degrees - values.conedelta_degrees;
            if falloff > 0. {
                ring(
                    centre,
                    u,
                    v,
                    distance * falloff.to_radians().tan(),
                    Some(SpotHandle::Delta),
                    RING_SEGMENTS,
                    &mut segments,
                );
            } else {
                ring(
                    centre,
                    u,
                    v,
                    DELTA_MARKER_RADIUS * world_per_pixel,
                    Some(SpotHandle::Delta),
                    DELTA_MARKER_SEGMENTS,
                    &mut segments,
                );
            }
            for k in 0..4 {
                let t = k as f64 * std::f64::consts::FRAC_PI_2;
                let rim = add(
                    centre,
                    add(scale(u, outer * t.cos()), scale(v, outer * t.sin())),
                );
                // Untagged: a grab off the rim plane would measure a
                // different angle and jump the cone on the first move.
                segments.push((None, (apex, rim)));
            }
        }
        // A direction arrow from world `from` along `to - from`.
        LightKind::Distant => {
            let origin = transform_point(world_from_light, values.from);
            let Some(direction) = normalize(transform_vector(
                world_from_light,
                sub(values.to, values.from),
            )) else {
                return segments;
            };
            arrow(
                origin,
                direction,
                arrow_distance(SPOT_DEPTH * world_per_pixel),
                world_per_pixel,
                // The arrow aims the light by moving `to`, exactly as in
                // a spot's tip drag.
                Some(SpotHandle::Tip),
                &mut segments,
            );
        }
        // Both look along the light's +z from its origin.
        LightKind::Goniometric | LightKind::Projection => {
            let origin = transform_point(world_from_light, [0., 0., 0.]);
            let Some(direction) = normalize(transform_vector(world_from_light, [0., 0., 1.]))
            else {
                return segments;
            };
            arrow(
                origin,
                direction,
                SPOT_DEPTH * world_per_pixel,
                world_per_pixel,
                None,
                &mut segments,
            );
        }
    }
    segments
}

/// `world_segments` projected to image pixels; a segment whose end lands
/// behind the camera is dropped (like the transform gizmo's rings).
/// `length` is the handle budget in image pixels (the gizmo's
/// `LENGTH`-style budget after the viewport scale), converted at the
/// light's own screen distance.
pub fn light_shapes(
    values: &LightValues,
    world_from_light: [f64; 16],
    projector: &super::gizmo::Projector,
    length: f64,
) -> Vec<(Option<SpotHandle>, super::gizmo::Shape)> {
    let world_per_pixel =
        projector.pixel_size(apex_of(values, world_from_light)) * length / SPOT_DEPTH;
    world_segments(values, world_from_light, world_per_pixel)
        .into_iter()
        .filter_map(
            |(handle, (a, b))| match (projector.project(a), projector.project(b)) {
                (Some(a), Some(b)) => Some((handle, super::gizmo::Shape::Line(a, b))),
                _ => None,
            },
        )
        .collect()
}

/// The light's world anchor: `from` for the positional and directional
/// lights, the transform's translation for the +z-lookers.
fn apex_of(values: &LightValues, world_from_light: [f64; 16]) -> [f64; 3] {
    let local = match values.kind {
        LightKind::Goniometric | LightKind::Projection => [0., 0., 0.],
        _ => values.from,
    };
    transform_point(world_from_light, local)
}

/// `world_segments`'s cone axis and apex in world space. Distant light
/// presses use the same pair, shared with the drag maths so
/// the rim the pointer grabbed and the value the drag computes never
/// disagree.
pub(super) fn spot_frame_parts(
    values: &LightValues,
    world_from_light: [f64; 16],
) -> Option<([f64; 3], [f64; 3])> {
    let apex = transform_point(world_from_light, values.from);
    let axis = normalize(transform_vector(
        world_from_light,
        sub(values.to, values.from),
    ))?;
    Some((apex, axis))
}

/// The world frame of one light at press time: the facts its
/// handles drag against — a spot's cone rims and tip, or a
/// distant light's `to` drag, which shares this frame exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SpotFrame {
    /// The apex, world `from`.
    pub apex: [f64; 3],
    /// Unit cone axis, from `from` toward `to` in world space.
    pub axis: [f64; 3],
    /// The drawn arrowhead (`apex + axis * arrow_distance(distance)`,
    /// running clear past the rim plane): the tip drag's anchor.
    pub tip: [f64; 3],
    /// `from` in the light's own (parameter) space, and the parameter
    /// length |to - from| the tip drag keeps.
    pub from: [f64; 3],
    pub reach: f64,
    /// The rim circles' distance along the axis, in world units.
    pub distance: f64,
}

impl SpotFrame {
    /// `distance` uses the same screen-constant conversion
    /// `light_shapes` draws with (`length` is its handle budget).
    pub fn of(
        values: &LightValues,
        world_from_light: [f64; 16],
        projector: &Projector,
        length: f64,
    ) -> Option<Self> {
        let (apex, axis) = spot_frame_parts(values, world_from_light)?;
        let distance = length * projector.pixel_size(apex);
        let reach = sub(values.to, values.from);
        Some(Self {
            apex,
            axis,
            tip: add(apex, scale(axis, arrow_distance(distance))),
            distance,
            from: values.from,
            reach: dot(reach, reach).sqrt(),
        })
    }
}

/// The spot handle under a pixel: the nearest tagged line within the
/// transform gizmo's tolerance. The desktop asks the transform gizmo
/// first, so a light handle never steals its press.
pub fn spot_hit(shapes: &[(Option<SpotHandle>, Shape)], xy: [f64; 2]) -> Option<SpotHandle> {
    shapes
        .iter()
        .filter_map(|(handle, shape)| match (handle, shape) {
            (Some(handle), Shape::Line(a, b)) => Some((segment_distance(*a, *b, xy), *handle)),
            _ => None,
        })
        .filter(|(distance, _)| *distance <= super::gizmo::TOLERANCE)
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, handle)| handle)
}

/// Where `ray` crosses the plane through `p` with normal `n` (both unit
/// `n` unnecessary); world space.
fn ray_plane(ray: ([f64; 3], [f64; 3]), p: [f64; 3], n: [f64; 3]) -> Option<[f64; 3]> {
    let (o, d) = ray;
    let denominator = dot(n, d);
    if denominator.abs() < 1e-9 {
        return None;
    }
    let t = dot(sub(p, o), n) / denominator;
    (t > 1e-9).then(|| add(o, scale(d, t)))
}

/// The cone half-angle the pointer's ray picks out (degrees, before any
/// clamp): where the ray crosses the rim's plane, the angle between the
/// axis and the line from the apex — the exact rim the user grabbed.
pub fn cone_angle_degrees(frame: &SpotFrame, ray: ([f64; 3], [f64; 3])) -> Option<f64> {
    let centre = add(frame.apex, scale(frame.axis, frame.distance));
    let p = ray_plane(ray, centre, frame.axis)?;
    let v = sub(p, frame.apex);
    let along = dot(v, frame.axis);
    let off = (dot(v, v) - along * along).max(0.).sqrt();
    off.is_finite().then(|| off.atan2(along).to_degrees())
}

/// The tip drag's new `to`, in the light's own (parameter) space: the
/// pointer's ray crosses the camera-facing plane through the drawn
/// arrowhead, the arrowhead moves by the crossing's offset from the
/// press (`press_ray`), so a grab anywhere on the shaft keeps its offset
/// and an unmoved pointer changes nothing; the new direction keeps the
/// parameter length |to - from|. `light_from_world` is the light's
/// inverse world transform; the Transform itself never changes.
pub fn tip_to(
    frame: &SpotFrame,
    camera_forward: [f64; 3],
    press_ray: ([f64; 3], [f64; 3]),
    ray: ([f64; 3], [f64; 3]),
    light_from_world: [f64; 16],
) -> Option<[f64; 3]> {
    if !frame.reach.is_finite() || frame.reach < 1e-12 {
        return None;
    }
    let grab = ray_plane(press_ray, frame.tip, camera_forward)?;
    let crossing = ray_plane(ray, frame.tip, camera_forward)?;
    let moved = add(frame.tip, sub(crossing, grab));
    let world = normalize(sub(moved, frame.apex))?;
    let local = normalize(transform_vector(light_from_world, world))?;
    Some(add(frame.from, scale(local, frame.reach)))
}

/// coneangle stays a real opening: (0, 90] degrees.
pub fn clamp_coneangle(degrees: f64) -> f64 {
    degrees.clamp(MIN_CONEANGLE_DEGREES, 90.)
}

/// conedelta stays within the cone: [0, coneangle] degrees.
pub fn clamp_conedelta(degrees: f64, coneangle_degrees: f64) -> f64 {
    degrees.clamp(0., coneangle_degrees.max(0.))
}

/// What dragging the outer rim commits: the clamped coneangle and
/// conedelta pulled in with it, so a cone narrowed past its falloff never
/// leaves conedelta > coneangle (pbrt would reverse the falloff).
pub fn coneangle_drag(measured: f64, conedelta_degrees: f64) -> (f64, f64) {
    let coneangle = clamp_coneangle(measured);
    (coneangle, clamp_conedelta(conedelta_degrees, coneangle))
}

/// What dragging the inner falloff ring commits: the ring sits at
/// the falloff-start half-angle `coneangle - conedelta`, so the
/// pointer's measured angle `degrees` commits its complement, clamped
/// into the cone.
pub fn conedelta_from_handle_degrees(measured: f64, coneangle_degrees: f64) -> f64 {
    clamp_conedelta(coneangle_degrees - measured, coneangle_degrees)
}

/// What dragging the falloff *marker* commits: the marker — the
/// small ring drawn around the axis when there is no inner circle —
/// sits at a tiny positive angle `delta_marker_angle_degrees`, so its
/// own radius counts as the axis: an unmoved press leaves conedelta at
/// coneangle (committing nothing when it was already there), and
/// dragging outward from the marker lowers conedelta below the cone.
pub fn conedelta_from_marker_handle_degrees(measured: f64, coneangle_degrees: f64) -> f64 {
    conedelta_from_handle_degrees(
        (measured - delta_marker_angle_degrees()).max(0.),
        coneangle_degrees,
    )
}

fn add(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
fn scale(a: [f64; 3], s: f64) -> [f64; 3] {
    [a[0] * s, a[1] * s, a[2] * s]
}
/// A unit vector, or `None` for a direction that does not exist.
fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
    norm(v)
}
fn transform_point(m: [f64; 16], p: [f64; 3]) -> [f64; 3] {
    [
        m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
        m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
        m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
    ]
}
fn transform_vector(m: [f64; 16], v: [f64; 3]) -> [f64; 3] {
    [
        m[0] * v[0] + m[1] * v[1] + m[2] * v[2],
        m[4] * v[0] + m[5] * v[1] + m[6] * v[2],
        m[8] * v[0] + m[9] * v[1] + m[10] * v[2],
    ]
}
/// The world matrix's basis, its columns normalised; a collapsed column
/// falls back like the gizmo's axes do.
fn world_axes(m: [f64; 16]) -> [[f64; 3]; 3] {
    let columns: Vec<[f64; 3]> = (0..3)
        .filter_map(|j| {
            let c = [m[j], m[4 + j], m[8 + j]];
            norm(c)
        })
        .collect();
    columns
        .try_into()
        .unwrap_or([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]])
}
/// One closed circle of `radius` around `centre` in the plane of `u`, `v`
/// (both unit, both perpendicular to the circle's axis).
fn ring(
    centre: [f64; 3],
    u: [f64; 3],
    v: [f64; 3],
    radius: f64,
    handle: Option<SpotHandle>,
    segments: usize,
    out: &mut Vec<(Option<SpotHandle>, Segment)>,
) {
    let at = |t: f64| {
        add(
            centre,
            add(scale(u, radius * t.cos()), scale(v, radius * t.sin())),
        )
    };
    let step = std::f64::consts::TAU / segments as f64;
    for k in 0..segments {
        let (t, next) = (k as f64 * step, (k + 1) as f64 * step);
        out.push((handle, (at(t), at(next))));
    }
}
/// The drawn arrow's length: the rim circles' `distance` along the axis
/// plus the head's clearance past the rim plane, sharing one
/// source of truth with `SpotFrame`'s tip anchor.
fn arrow_distance(distance: f64) -> f64 {
    distance * ((SPOT_DEPTH + ARROW_CLEARANCE) / SPOT_DEPTH)
}

/// A shaft `length` long ending in a four-wing head (two crossed
/// chevrons, so it never vanishes edge-on); the shaft's outer
/// `1 - SHAFT_UNTAGGED_PART` and the head carry `handle` — the inner
/// stretch near the origin stays untagged, where it overlaps the Move
/// gizmo that a press asks first.
fn arrow(
    origin: [f64; 3],
    direction: [f64; 3],
    length: f64,
    world_per_pixel: f64,
    handle: Option<SpotHandle>,
    out: &mut Vec<(Option<SpotHandle>, Segment)>,
) {
    let tip = add(origin, scale(direction, length));
    let inner = add(origin, scale(direction, length * SHAFT_UNTAGGED_PART));
    out.push((None, (origin, inner)));
    out.push((handle, (inner, tip)));
    let Some((u, v)) = perpendiculars(direction) else {
        return;
    };
    let head = POINT_RADIUS * world_per_pixel;
    let back = sub(tip, scale(direction, head));
    let span = head * WING;
    for wing in [u, v] {
        out.push((handle, (tip, add(back, scale(wing, span)))));
        out.push((handle, (tip, sub(back, scale(wing, span)))));
    }
}

#[cfg(test)]
mod tests {
    use super::super::ViewerCamera;
    use super::*;
    use serde_json::json;

    fn page(rows: &[(&str, &str, Vec<f64>)]) -> Value {
        json!({"parameters": rows.iter().map(|(name, value_type, values)| json!({
            "name":name, "value_type":value_type,
            "values":{"kind":"floats","value":values},
        })).collect::<Vec<_>>()})
    }

    fn identity() -> [f64; 16] {
        [
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ]
    }

    /// Column-major like pbrt: translation in [3],[7],[11].
    fn translation(x: f64, y: f64, z: f64) -> [f64; 16] {
        let mut m = identity();
        m[3] = x;
        m[7] = y;
        m[11] = z;
        m
    }

    fn values(implementation: &str, rows: &[(&str, &str, Vec<f64>)]) -> Option<LightValues> {
        light_values(implementation, &page(rows))
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 1e-9
    }

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

    #[test]
    fn light_kinds_cover_the_analytic_lights_only() {
        assert_eq!(light_kind("point"), Some(LightKind::Point));
        assert_eq!(light_kind("spot"), Some(LightKind::Spot));
        assert_eq!(light_kind("distant"), Some(LightKind::Distant));
        assert_eq!(light_kind("goniometric"), Some(LightKind::Goniometric));
        assert_eq!(light_kind("projection"), Some(LightKind::Projection));
        // Nothing draws for the lights that are not points in the scene.
        assert_eq!(light_kind("infinite"), None);
        assert_eq!(light_kind("diffuse"), None);
        assert_eq!(light_kind(""), None);
        assert_eq!(light_kind("Spot"), None, "the wire names are lowercase");
    }

    #[test]
    fn a_light_without_declarations_uses_pbrt_v4_defaults() {
        let v = values("spot", &[]).unwrap();
        assert_eq!(v.kind, LightKind::Spot);
        assert_eq!(v.from, [0., 0., 0.]);
        assert_eq!(v.to, [0., 0., 1.]);
        assert!((v.coneangle_degrees - 30.).abs() < 1e-9);
        assert!((v.conedelta_degrees - 5.).abs() < 1e-9);
        let v = values("distant", &[]).unwrap();
        assert_eq!(v.to, [0., 0., 1.]);
        assert_eq!(v.coneangle_degrees, 30.);
        assert_eq!(v.conedelta_degrees, 5.);
    }

    #[test]
    fn declared_rows_win_over_the_defaults_and_aliases_match() {
        let v = values(
            "spot",
            &[
                ("from", "point3", vec![1., 2., 3.]),
                ("to", "point3", vec![4., 5., 6.]),
                ("coneangle", "float", vec![45.]),
                ("conedeltaangle", "float", vec![10.]),
            ],
        )
        .unwrap();
        assert_eq!(v.from, [1., 2., 3.]);
        assert_eq!(v.to, [4., 5., 6.]);
        assert!((v.coneangle_degrees - 45.).abs() < 1e-9);
        assert!((v.conedelta_degrees - 10.).abs() < 1e-9);
        // An empty or truncated row never reaches the values.
        let v = values(
            "point",
            &[
                ("from", "point3", vec![1., 2.]),
                ("FROM", "point3", vec![9., 9., 9.]),
            ],
        )
        .unwrap();
        assert_eq!(v.from, [9., 9., 9.], "names match case-insensitively");
    }

    #[test]
    fn area_and_infinite_lights_have_no_values() {
        assert!(values("infinite", &[]).is_none());
        assert!(values("diffuse", &[]).is_none());
    }

    /// The core's parameter pages encode each float as
    /// `{"kind":"finite","value":x}` (a `FloatValue` on the wire); a
    /// bare number is accepted the same way `parameter_rows::float`
    /// accepts both.
    #[test]
    fn declared_rows_accept_the_encoded_float_spelling() {
        let page = json!({"parameters":[
            {"name":"from","value_type":"point3",
             "values":{"kind":"floats","value":[{"kind":"finite","value":1.},
                                                 {"kind":"finite","value":0.},
                                                 {"kind":"finite","value":3.}]}},
            {"name":"coneangle","value_type":"float",
             "values":{"kind":"floats","value":[{"kind":"finite","value":40.}]}},
        ]});
        let v = light_values("spot", &page).unwrap();
        assert_eq!(v.from, [1., 0., 3.]);
        assert!((v.coneangle_degrees - 40.).abs() < 1e-9);
    }

    #[test]
    fn a_point_light_draws_three_small_axis_circles_around_from() {
        let v = values("point", &[("from", "point3", vec![1., 2., 3.])]).unwrap();
        let segments = world_segments(&v, translation(10., 20., 30.), 1.);
        let apex = [11., 22., 33.];
        const RING: usize = 32;
        assert_eq!(segments.len(), 3 * RING);
        for (handle, (a, b)) in &segments {
            assert_eq!(*handle, None, "a point light has no handles");
            for p in [a, b] {
                let radius = (p[0] - apex[0]).hypot(p[1] - apex[1]).hypot(p[2] - apex[2]);
                assert!(
                    near(radius, 12.),
                    "screen-constant 12px radius, got {radius}"
                );
            }
        }
        // One circle lies in each axis plane: the segments of each ring
        // are coplanar with two of the world axes.
        let planes: Vec<bool> = segments
            .chunks(RING)
            .map(|ring| {
                let flat_axis = (0..3).find(|axis| {
                    ring.iter()
                        .all(|(_, (a, b))| p_is_flat(*a, apex, *axis) && p_is_flat(*b, apex, *axis))
                });
                flat_axis.is_some()
            })
            .collect();
        assert!(planes.iter().all(|ok| *ok), "each ring is axis-aligned");
        assert_eq!(
            planes.len(),
            segments.len() / RING,
            "three rings, one per axis plane"
        );
    }

    fn p_is_flat(p: [f64; 3], apex: [f64; 3], axis: usize) -> bool {
        near(p[axis] - apex[axis], 0.)
    }

    #[test]
    fn a_spot_light_draws_the_arrow_both_cone_circles_and_rim_lines() {
        let v = values(
            "spot",
            &[
                ("from", "point3", vec![0., 0., 0.]),
                ("to", "point3", vec![0., 0., 1.]),
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let segments = world_segments(&v, identity(), 1.);
        const RING: usize = 64;
        // inner shaft + outer shaft + chevron (4) + outer ring + inner
        // ring + 4 rim lines
        assert_eq!(segments.len(), 1 + 1 + 4 + RING + RING + 4);
        let count = |wanted: Option<SpotHandle>| {
            segments
                .iter()
                .filter(|(handle, _)| *handle == wanted)
                .count()
        };
        assert_eq!(count(Some(SpotHandle::Cone)), RING);
        assert_eq!(count(None), 5, "the rim lines and the inner shaft");
        assert_eq!(count(Some(SpotHandle::Delta)), RING);
        assert_eq!(count(Some(SpotHandle::Tip)), 5);

        // The shaft's inner stretch is untagged (it overlaps the
        // Move gizmo, which wins anyway); the outer 40% and the head
        // carry the Tip handle. The arrow runs clear past the rim plane
        // so its head stands free of the cone's circles.
        let arrow_length = (SPOT_DEPTH + ARROW_CLEARANCE) * 1.;
        let split = arrow_length * SHAFT_UNTAGGED_PART;
        let inner = segments
            .iter()
            .find(|(handle, (a, _))| {
                handle.is_none() && near(a[0], 0.) && near(a[1], 0.) && near(a[2], 0.)
            })
            .expect("the inner shaft is untagged");
        let (_, (_, b)) = inner;
        assert!(near(b[2], split), "shaft split at {split}, got {b:?}");
        let outer = segments
            .iter()
            .find(|(handle, (a, _))| *handle == Some(SpotHandle::Tip) && near(a[2], split))
            .expect("the outer shaft carries the Tip handle");
        let (_, (_, b)) = outer;
        assert!(near(b[2], arrow_length), "tip at {arrow_length}, got {b:?}");
        // 90 budget px * 1 world/px along +z: the ring plane sits there.
        let plane = [0., 0., 90.];
        let outer = 90. * 45f64.to_radians().tan();
        let inner = 30f64.to_radians().tan() * 90.;
        for handle in [Some(SpotHandle::Cone), Some(SpotHandle::Delta)] {
            let expected = if handle == Some(SpotHandle::Cone) {
                outer
            } else {
                inner
            };
            for (_, (a, b)) in segments.iter().filter(|(h, _)| *h == handle).take(RING) {
                for p in [a, b] {
                    assert!(
                        near(p[2] - plane[2], 0.),
                        "the ring lies in the plane through the axis"
                    );
                    let radius = (p[0] - plane[0]).hypot(p[1] - plane[1]);
                    assert!(
                        near(radius, expected),
                        "got radius {radius}, expected {expected}"
                    );
                }
            }
        }
        // The rim lines run from the apex to the outer circle.
        for (_, (a, b)) in segments.iter().filter(|(h, _)| h.is_none()) {
            // The inner shaft is the one None segment that stops short
            // of the rim plane.
            if !near(b[2], plane[2]) {
                assert!(near(b[2], split), "only the inner shaft stops short");
                continue;
            }
            assert!(near(a[0], 0.) && near(a[1], 0.) && near(a[2], 0.));
            let radius = b[0].hypot(b[1]);
            assert!(near(radius, outer), "the rim ends on the outer circle");
            assert!(near(b[2] - plane[2], 0.));
        }
    }

    /// With the falloff closed (`coneangle − conedelta` <= 0)
    /// the inner circle is never drawn, so its handle is gone — draw a
    /// small marker ring around the axis in the rim plane instead,
    /// tagged Delta, so the falloff stays grabbable and moving outward
    /// pulls conedelta back below coneangle.
    #[test]
    fn a_closed_falloff_draws_a_marker_ring_around_the_axis() {
        for conedelta in [45., 60.] {
            let v = values(
                "spot",
                &[
                    ("coneangle", "float", vec![45.]),
                    ("conedelta", "float", vec![conedelta]),
                ],
            )
            .unwrap();
            let segments = world_segments(&v, identity(), 1.);
            let marker: Vec<_> = segments
                .iter()
                .filter(|(handle, _)| *handle == Some(SpotHandle::Delta))
                .collect();
            assert_eq!(
                marker.len(),
                DELTA_MARKER_SEGMENTS,
                "conedelta {conedelta} draws the marker, not a circle"
            );
            for (_, (a, b)) in &marker {
                for p in [*a, *b] {
                    // The rim plane, 90 budget px along the axis.
                    assert!(near(p[2], 90.), "in the rim plane, got {p:?}");
                    let radius = p[0].hypot(p[1]);
                    assert!(
                        near(radius, DELTA_MARKER_RADIUS),
                        "a {DELTA_MARKER_RADIUS} budget px ring, got {radius}"
                    );
                }
            }
        }
        // A positive falloff keeps the inner circle and no marker.
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let segments = world_segments(&v, identity(), 1.);
        let inner = 30f64.to_radians().tan() * 90.;
        assert_eq!(
            segments
                .iter()
                .filter(|(h, _)| *h == Some(SpotHandle::Delta))
                .count(),
            64
        );
        for (_, (a, b)) in segments
            .iter()
            .filter(|(handle, _)| *handle == Some(SpotHandle::Delta))
        {
            for p in [*a, *b] {
                assert!(
                    near(p[0].hypot(p[1]), inner),
                    "the inner circle, not a marker"
                );
            }
        }
    }

    /// The marker answers the Delta handle — not the Tip arrow
    /// that crosses the rim plane right beside it.
    #[test]
    fn the_marker_ring_answers_the_delta_handle() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![60.]),
                ("to", "point3", vec![1., 0., 0.]),
            ],
        )
        .unwrap();
        let shapes = light_shapes(&v, identity(), &projector, 90.);
        let midpoint = shapes
            .iter()
            .find(|(handle, _)| *handle == Some(SpotHandle::Delta))
            .map(|(_, shape)| match shape {
                Shape::Line(a, b) => [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
                _ => panic!("lines only"),
            })
            .unwrap();
        assert_eq!(spot_hit(&shapes, midpoint), Some(SpotHandle::Delta));
    }

    /// The marker sits at a tiny positive angle, so its own
    /// radius counts as the axis — an unmoved press leaves conedelta ==
    /// coneangle (committing nothing when it is already there) and
    /// dragging outward from the marker lowers conedelta below the cone
    /// angle.
    #[test]
    fn the_marker_commits_from_its_own_radius_as_the_axis() {
        let coneangle = 45.;
        let marker = delta_marker_angle_degrees();
        assert!(
            (marker - 4.).abs() < 1.,
            "a 7 budget px marker at a 90 budget px depth sits just under 5° ({marker})"
        );
        // An unmoved press measures the marker's own angle: conedelta
        // stays at coneangle.
        assert!(
            (conedelta_from_marker_handle_degrees(marker, coneangle) - coneangle).abs() < 1e-9,
            "an unmoved press leaves conedelta == coneangle"
        );
        // One degree… ten degrees outward from the marker: conedelta
        // lowers the same amount below coneangle.
        assert!(
            (conedelta_from_marker_handle_degrees(marker + 10., coneangle) - 35.).abs() < 1e-9,
            "dragging outward lowers conedelta below the cone angle"
        );
        // Inward of the marker the clamp holds: never wider than the
        // cone, never below zero.
        assert_eq!(
            conedelta_from_marker_handle_degrees(0., coneangle),
            coneangle
        );
        assert_eq!(
            conedelta_from_marker_handle_degrees(180., coneangle),
            0.,
            "far past the cone the falloff closes"
        );
    }

    #[test]
    fn a_distant_light_draws_a_direction_arrow_along_to_minus_from() {
        let v = values(
            "distant",
            &[
                ("from", "point3", vec![1., 0., 0.]),
                ("to", "point3", vec![0., 0., 0.]),
            ],
        )
        .unwrap();
        let segments = world_segments(&v, identity(), 1.);
        // The shaft's two halves and a four-wing head; the head runs clear
        // past where a rim plane would sit.
        assert_eq!(segments.len(), 6);
        // The outer shaft and the head carry the Tip handle —
        // the arrow aims the light by moving `to` — the inner stretch
        // near the origin stays untagged for the Move gizmo.
        let count = |wanted: Option<SpotHandle>| {
            segments
                .iter()
                .filter(|(handle, _)| *handle == wanted)
                .count()
        };
        assert_eq!(count(Some(SpotHandle::Tip)), 5);
        assert_eq!(count(None), 1);
        let (a, b) = segments[0].1;
        assert!(near(a[0], 1.) && near(a[1], 0.) && near(a[2], 0.));
        assert!(near(b[0] + 67.2, 1.) && near(b[1], 0.) && near(b[2], 0.));
        let (a, b) = segments[1].1;
        assert!(near(a[0] + 67.2, 1.) && near(a[1], 0.) && near(a[2], 0.));
        assert!(near(b[0] + 111., 0.) && near(b[1], 0.) && near(b[2], 0.));
    }

    /// The distant light's arrow answers the Tip handle, and
    /// its drag keeps |to − from| exactly the way a spot's does — the
    /// frame is shared (`SpotFrame`), only the values differ. The arrow
    /// needs a side view: one parallel to the view direction projects
    /// onto a point.
    #[test]
    fn a_distant_arrow_aims_through_the_tip_handle() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "distant",
            &[
                ("from", "point3", vec![0., 0., 0.]),
                ("to", "point3", vec![1., 0., 0.]),
            ],
        )
        .unwrap();
        let shapes = light_shapes(&v, identity(), &projector, 90.);
        assert_eq!(
            spot_hit(
                &shapes,
                shapes
                    .iter()
                    .find(|(handle, _)| *handle == Some(SpotHandle::Tip))
                    .map(|(_, shape)| match shape {
                        Shape::Line(a, b) => [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
                        _ => panic!("lines only"),
                    })
                    .unwrap()
            ),
            Some(SpotHandle::Tip),
            "the outer shaft answers the Tip handle"
        );
        // The drag frame anchors the drawn head, and an unmoved pointer
        // commits nothing: `to` keeps the page's own value.
        let frame = SpotFrame::of(&v, identity(), &projector, 90.).unwrap();
        let world = world_segments(
            &v,
            identity(),
            projector.pixel_size([0., 0., 0.]) * 90. / SPOT_DEPTH,
        );
        // The drawn head: the far end of the Tip-tagged outer shaft.
        let head = world
            .iter()
            .find(|(handle, _)| *handle == Some(SpotHandle::Tip))
            .map(|(_, (_, b))| b)
            .expect("the distant light draws a Tip shaft");
        assert!(
            head.iter().zip(frame.tip).all(|(a, b)| near(*a, b)),
            "the frame anchors the drawn head {head:?} vs {:?}",
            frame.tip
        );
        let press = projector.ray(projector.project(frame.tip).unwrap());
        let to = tip_to(&frame, projector.forward(), press, press, identity()).unwrap();
        assert!(
            to.iter().zip(v.to).all(|(a, b)| (a - b).abs() < 1e-9),
            "an unmoved press keeps `to`, got {to:?}"
        );
    }

    #[test]
    fn goniometric_and_projection_arrows_follow_the_world_z_axis() {
        // A quarter turn about -x maps the light's +z to +y; the arrow
        // then runs from the translated origin toward +y.
        let world = [
            1., 0., 0., 5., //
            0., 0., 1., 0., //
            0., -1., 0., 0., //
            0., 0., 0., 1.,
        ];
        for implementation in ["goniometric", "projection"] {
            let v = values(implementation, &[]).unwrap();
            let segments = world_segments(&v, world, 1.);
            assert_eq!(segments.len(), 6, "{implementation}: shaft and head");
            assert!(segments.iter().all(|(handle, _)| handle.is_none()));
            let (a, b) = segments[0].1;
            assert!(near(a[0], 5.) && near(a[1], 0.) && near(a[2], 0.));
            assert!(
                near(b[0], 5.) && near(b[1], 54.) && near(b[2], 0.),
                "{implementation}: the shaft splits at 60% of its length, got {b:?}"
            );
            let (_, b) = segments[1].1;
            assert!(near(b[0], 5.) && near(b[1], 90.) && near(b[2], 0.));
        }
    }

    #[test]
    fn a_degenerate_light_axis_draws_nothing() {
        for rows in [
            [
                ("from", "point3", vec![0., 0., 0.]),
                ("to", "point3", vec![0., 0., 0.]),
            ]
            .as_slice(),
            [
                ("from", "point3", vec![1., 0., 0.]),
                ("to", "point3", vec![1., 0., 0.]),
            ]
            .as_slice(),
        ] {
            let spot = values("spot", rows).unwrap();
            assert!(world_segments(&spot, identity(), 1.).is_empty());
            let distant = values("distant", rows).unwrap();
            assert!(world_segments(&distant, identity(), 1.).is_empty());
        }
        // A collapsed +z cannot orient goniometric or projection either.
        let gonio = light_values("goniometric", &json!({"parameters":[]})).unwrap();
        let zero = [0.; 16];
        assert!(world_segments(&gonio, zero, 1.).is_empty());
    }

    #[test]
    fn projected_shapes_match_the_world_segments() {
        let camera = ViewerCamera {
            eye: [0., 0., -10.],
            target: [0., 0., 0.],
            up: [0., 1., 0.],
            fov_y_degrees: 45.,
            near: 0.01,
            far: 100.,
            mirror: false,
        };
        let projector = super::super::gizmo::Projector::new(&camera, 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let shapes = light_shapes(&v, identity(), &projector, 90.);
        let world_per_pixel = projector.pixel_size([0., 0., 0.]);
        assert_eq!(
            shapes.len(),
            world_segments(&v, identity(), world_per_pixel).len()
        );
        for (_, shape) in &shapes {
            let super::super::gizmo::Shape::Line(..) = shape else {
                panic!("a light draws lines only");
            };
        }
        // A light behind the camera draws nothing.
        let behind = values("spot", &[("from", "point3", vec![0., 0., -30.])]).unwrap();
        assert!(light_shapes(&behind, identity(), &projector, 90.).is_empty());
    }

    #[test]
    fn a_drag_on_each_spot_handle_hits_its_own_shape() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let shapes = light_shapes(&v, identity(), &projector, 90.);
        // The exact midpoint of one tagged segment is that handle's —
        // the rings sit hundreds of pixels apart at this budget.
        let midpoint = |wanted: Option<SpotHandle>| {
            shapes
                .iter()
                .find(|(handle, _)| *handle == wanted)
                .map(|(_, shape)| match shape {
                    Shape::Line(a, b) => [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
                    _ => panic!("lines only"),
                })
                .unwrap()
        };
        for wanted in [Some(SpotHandle::Cone), Some(SpotHandle::Delta)] {
            assert_eq!(spot_hit(&shapes, midpoint(wanted)), wanted);
        }
        // The outer shaft is the Tip handle's own line.
        assert_eq!(
            spot_hit(&shapes, midpoint(Some(SpotHandle::Tip))),
            Some(SpotHandle::Tip)
        );
        // Nothing answers far away.
        assert_eq!(spot_hit(&shapes, [10., 10.]), None);
    }

    /// The shaft's inner stretch carries no handle — it overlaps
    /// the Move gizmo, whose press wins anyway — while the outer stretch
    /// and the head answer the Tip handle. The arrow needs a side view:
    /// one parallel to the view direction projects onto a point.
    #[test]
    fn only_the_outer_shaft_and_head_answer_the_tip_handle() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("to", "point3", vec![1., 0., 0.]),
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let shapes = light_shapes(&v, identity(), &projector, 90.);
        let midpoint = |wanted: Option<SpotHandle>| {
            shapes
                .iter()
                .find(|(handle, _)| *handle == wanted)
                .map(|(_, shape)| match shape {
                    Shape::Line(a, b) => [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
                    _ => panic!("lines only"),
                })
                .unwrap()
        };
        // The untagged inner shaft is the first shape the drawing pushes.
        let inner = shapes
            .iter()
            .find(|(handle, _)| handle.is_none())
            .map(|(_, shape)| match shape {
                Shape::Line(a, b) => [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.],
                _ => panic!("lines only"),
            })
            .unwrap();
        assert_eq!(spot_hit(&shapes, inner), None, "the inner shaft");
        assert_eq!(
            spot_hit(&shapes, midpoint(Some(SpotHandle::Tip))),
            Some(SpotHandle::Tip),
            "the outer shaft still answers"
        );
    }

    #[test]
    fn the_cone_angle_comes_from_the_pointer_ray_against_the_axis() {
        // An axis straight down +z, the rim plane 90 world units along
        // it; a ray crossing the plane 30 units off the axis measures the
        // rim's own half-angle exactly.
        let frame = SpotFrame {
            apex: [0., 0., 4.],
            axis: [0., 0., -1.],
            tip: [0., 0., -86.],
            distance: 90.,
            from: [0., 0., 4.],
            reach: 4.,
        };
        let ray = ([900., 0., 1000.], norm([-870., 0., -1086.]).unwrap());
        let degrees = cone_angle_degrees(&frame, ray).unwrap();
        // atan(30/90): the rim's own half-angle.
        assert!(
            (degrees - (30f64 / 90.).atan().to_degrees()).abs() < 1e-9,
            "{degrees}"
        );
        // The ray parallel to the plane never crosses it.
        let ray = ([900., 0., 1000.], [1., 0., 0.]);
        assert!(cone_angle_degrees(&frame, ray).is_none());
    }

    /// The inner ring sits at the falloff-start half-angle
    /// (coneangle - conedelta), so a drag on it commits the complement
    /// of the measured angle: grabbing the drawn ring without moving
    /// changes nothing, dragging it out to the outer cone closes the
    /// falloff to zero, and onto the axis it widens it to the full cone.
    #[test]
    fn the_inner_ring_handle_commits_the_complement() {
        // coneangle 25, conedelta 10: the drawn ring sits at 15°.
        assert!(
            (conedelta_from_handle_degrees(15., 25.) - 10.).abs() < 1e-9,
            "a twitchless grab keeps the value"
        );
        assert!(
            conedelta_from_handle_degrees(25., 25.).abs() < 1e-9,
            "dragged onto the outer circle the falloff closes"
        );
        assert!(
            (conedelta_from_handle_degrees(0., 25.) - 25.).abs() < 1e-9,
            "dragged onto the axis the falloff widens to the cone"
        );
        // Beyond the cone the clamp holds: dragged past the outer
        // circle — or behind the apex, where the measured angle opens
        // past 90° — the falloff stays closed.
        assert!(conedelta_from_handle_degrees(30., 25.).abs() < 1e-12);
        assert!(conedelta_from_handle_degrees(150., 25.).abs() < 1e-12);
        assert!(
            (conedelta_from_handle_degrees(-5., 25.) - 25.).abs() < 1e-12,
            "never wider than the cone"
        );
    }

    /// A fully open cone (coneangle exactly 90, an allowed clamp value)
    /// still draws a finite rim: the drawn half-angle is capped just
    /// below 90 so the tan() of the rim stays a number. The committed
    /// value itself is not capped.
    #[test]
    fn a_fully_open_cone_still_draws_a_finite_rim() {
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![90.]),
                ("conedelta", "float", vec![10.]),
            ],
        )
        .unwrap();
        let segments = world_segments(&v, identity(), 1.);
        assert_eq!(segments[0].1.0, [0., 0., 0.], "the apex stays put");
        for (_, (a, b)) in &segments {
            for point in [a, b] {
                for value in point {
                    assert!(value.is_finite(), "{point:?}");
                    assert!(
                        value.abs() < 1e6,
                        "a rim point sits on screen, got {point:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn narrowing_the_cone_past_its_falloff_pulls_conedelta_in() {
        assert_eq!(coneangle_drag(40., 8.), (40., 8.), "conedelta untouched");
        assert_eq!(
            coneangle_drag(5., 8.),
            (5., 5.),
            "conedelta follows the cone"
        );
        assert_eq!(coneangle_drag(-3., 8.), (0.1, 0.1));
        assert_eq!(coneangle_drag(120., 8.), (90., 8.));
    }

    #[test]
    fn cone_values_are_clamped_to_a_sensible_cone() {
        assert!(
            (clamp_coneangle(0.) - 0.1).abs() < 1e-12,
            "a cone needs an opening"
        );
        assert!((clamp_coneangle(-5.) - 0.1).abs() < 1e-12);
        assert!((clamp_coneangle(90.) - 90.).abs() < 1e-12, "90 is allowed");
        assert!((clamp_coneangle(94.2) - 90.).abs() < 1e-12);
        assert!((clamp_coneangle(45.) - 45.).abs() < 1e-12);
        assert!(
            (clamp_conedelta(-1., 30.) - 0.).abs() < 1e-12,
            "the falloff may close"
        );
        assert!(
            (clamp_conedelta(37., 30.) - 30.).abs() < 1e-12,
            "never wider than the cone"
        );
        assert!((clamp_conedelta(12., 30.) - 12.).abs() < 1e-12);
    }

    #[test]
    fn a_tip_drag_keeps_the_length_and_tracks_the_drawn_arrowhead() {
        // from (0,0,4) toward to (0,0,0): reach 4, the arrowhead drawn
        // 90 units down the axis at (0,0,-86).
        let frame = SpotFrame {
            apex: [0., 0., 4.],
            axis: [0., 0., -1.],
            tip: [0., 0., -86.],
            distance: 90.,
            from: [0., 0., 4.],
            reach: 4.,
        };
        let forward = [0., 0., -1.];
        let press = ([0., 0., 1000.], [0., 0., -1.]);
        // An unmoved pointer gives back the page's own `to`.
        let to = tip_to(&frame, forward, press, press, identity()).unwrap();
        assert!(
            to.iter()
                .zip([0., 0., 0.])
                .all(|(a, b)| (a - b).abs() < 1e-9),
            "{to:?}"
        );
        // Moved by (10, 5) in the arrowhead's plane: the direction runs
        // from the apex to (10, 5, -86), the length stays 4.
        let ray = ([10., 5., 1000.], [0., 0., -1.]);
        let to = tip_to(&frame, forward, press, ray, identity()).unwrap();
        let direction = [10., 5., -90.];
        let k = 4. / (100f64 + 25. + 8100.).sqrt();
        let expected = [direction[0] * k, direction[1] * k, 4. + direction[2] * k];
        for i in 0..3 {
            assert!(
                (to[i] - expected[i]).abs() < 1e-9,
                "to[{i}] {} vs {}",
                to[i],
                expected[i]
            );
        }
        // A shaft grab keeps its offset: pressed mid-shaft off the
        // arrowhead, the same pointer move gives the same direction.
        let mid = ([0., 3., 1000.], [0., 0., -1.]);
        let moved = ([10., 8., 1000.], [0., 0., -1.]);
        let same = tip_to(&frame, forward, mid, moved, identity()).unwrap();
        for i in 0..3 {
            assert!((same[i] - expected[i]).abs() < 1e-9);
        }
        // Degenerate: no length to keep, or a ray that never crosses.
        let flat = SpotFrame { reach: 0., ..frame };
        assert!(tip_to(&flat, forward, press, ray, identity()).is_none());
        let parallel = ([10., 5., 1000.], [1., 0., 0.]);
        assert!(tip_to(&frame, forward, press, parallel, identity()).is_none());
    }

    #[test]
    fn the_drag_frame_matches_the_drawn_ring() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let frame = SpotFrame::of(&v, identity(), &projector, 90.).unwrap();
        // The frame's rim plane is exactly where the shapes draw their
        // outer circle, so a drag and its drawing never disagree.
        let centre = add(frame.apex, scale(frame.axis, frame.distance));
        let radius = frame.distance * 45f64.to_radians().tan();
        let world = world_segments(
            &v,
            identity(),
            projector.pixel_size([0., 0., 0.]) * 90. / SPOT_DEPTH,
        );
        for (handle, (a, b)) in world.iter().take(3 + 1 + RING_SEGMENTS) {
            if *handle != Some(SpotHandle::Cone) {
                continue;
            }
            for p in [*a, *b] {
                let along = dot(sub(p, frame.apex), frame.axis);
                assert!((along - frame.distance).abs() < 1e-9, "along {along}");
                let off = (p[0] - centre[0])
                    .hypot(p[1] - centre[1])
                    .hypot(p[2] - centre[2]);
                assert!((off - radius).abs() < 1e-9, "off {off} vs {radius}");
            }
        }
    }

    /// The tip drag's anchor is the head the drawing shows. The
    /// arrow runs clear past the rim plane, so the head stands apart
    /// from the circles — and an unmoved press on it commits nothing.
    #[test]
    fn the_tip_drag_anchor_is_the_drawn_arrowhead() {
        let projector = Projector::new(&camera(), 640, 480).unwrap();
        let v = values(
            "spot",
            &[
                ("coneangle", "float", vec![45.]),
                ("conedelta", "float", vec![15.]),
            ],
        )
        .unwrap();
        let frame = SpotFrame::of(&v, identity(), &projector, 90.).unwrap();
        let world = world_segments(
            &v,
            identity(),
            projector.pixel_size([0., 0., 0.]) * 90. / SPOT_DEPTH,
        );
        // The drawn head: the far end of the Tip-tagged outer shaft.
        let head = world
            .iter()
            .find(|(handle, _)| *handle == Some(SpotHandle::Tip))
            .map(|(_, (_, b))| b)
            .expect("the spot draws a Tip shaft");
        assert!(
            head.iter().zip(frame.tip).all(|(a, b)| near(*a, b)),
            "the frame anchors the drawn head {head:?} vs {:?}",
            frame.tip
        );
        // The head stands clear past the rim plane.
        let along = dot(sub(frame.tip, frame.apex), frame.axis);
        assert!(
            along - frame.distance > 1e-6,
            "the head is {along} along, past the rim at {d}",
            d = frame.distance
        );
        // An unmoved pointer on the head commits nothing.
        let press = projector.ray(projector.project(frame.tip).unwrap());
        let to = tip_to(&frame, projector.forward(), press, press, identity()).unwrap();
        assert!(
            to.iter().zip(v.to).all(|(a, b)| (a - b).abs() < 1e-9),
            "an unmoved press keeps `to`, got {to:?}"
        );
    }
}
