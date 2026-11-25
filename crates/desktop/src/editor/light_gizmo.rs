//! The selected light's wire-frame is derived from the Inspector's node details,
//! which carry the light component and its declared-parameter page
//! and the transform
//! component's world transform — the same details the transform gizmo
//! reads. This is the pure hop from that page into scene-view's
//! `LightValues`; the Scene panel draws the result.
//!
//! Spot light cone edits are built here as `set_parameter` edits using the
//! parameter page's declared spelling, with one Apply per finished drag.
use scene_view::preview::light_gizmo as light_wireframe;
use scene_view::preview::light_gizmo::{LightValues, light_values};
use serde_json::{Value, json};

/// The form id of a cone drag's commit, so a refusal can name it.
pub(crate) const LIGHT_DRAG_FORM: &str = "light-drag";

/// The `set_parameter` spelling of one of the light rows the drag
/// commits: the declared name when one exists (an alias stays an
/// alias), else the r4 table's canonical name, and the declared type.
#[derive(Clone, Debug, PartialEq)]
pub struct Spelling {
    pub name: String,
    pub value_type: String,
}

fn spelling(rows: &[Value], names: &[&str], value_type: &str) -> Spelling {
    let declared = rows.iter().find(|row| {
        row["name"]
            .as_str()
            .is_some_and(|name| names.iter().any(|want| name.eq_ignore_ascii_case(want)))
    });
    Spelling {
        name: declared
            .and_then(|row| row["name"].as_str())
            .unwrap_or(names[0])
            .to_owned(),
        value_type: declared
            .and_then(|row| row["value_type"].as_str())
            .unwrap_or(value_type)
            .to_owned(),
    }
}

/// What a Delta-handle drag commits for one measured angle: the inner circle's
/// complement when the page's falloff is
/// open — or, when the falloff is closed and the pressed handle is the
/// marker ring drawn around the axis, the marker's own radius counts as
/// the axis, so an unmoved press leaves conedelta at coneangle and
/// committing nothing when the page already sits there.
pub(crate) fn conedelta_complement(
    measured: f64,
    coneangle_degrees: f64,
    falloff_degrees: f64,
) -> f64 {
    if falloff_degrees > 0. {
        light_wireframe::conedelta_from_handle_degrees(measured, coneangle_degrees)
    } else {
        light_wireframe::conedelta_from_marker_handle_degrees(measured, coneangle_degrees)
    }
}

/// The selected light, or `None` when its details draw nothing: a
/// non-light node, a light without a wire-frame (infinite, area, an
/// unknown implementation), or one without a usable world transform.
#[derive(Clone, Debug, PartialEq)]
pub struct LightTarget {
    /// The transform gizmo's view of the same selection: the world
    /// matrix, the node id, the project the details were read at.
    pub target: super::gizmo::Target,
    pub values: LightValues,
    /// The light component's own `set_parameter` target (the node
    /// itself; edits carry it verbatim).
    pub parameter_target: Value,
    /// The rows a cone drag commits, in the page's own spelling.
    pub coneangle: Spelling,
    pub conedelta: Spelling,
    pub to: Spelling,
}

/// The spellings of the r4 rows a cone drag writes: pbrt-v4 reads
/// `coneangle` and `conedeltaangle` (the table calls it `conedelta`)
/// and `to`.
const CONEANGLE: &[&str] = &["coneangle"];
const CONEDELTA: &[&str] = &["conedelta", "conedeltaangle"];
const TO: &[&str] = &["to"];

pub fn light(details: &serde_json::Value) -> Option<LightTarget> {
    // The light component of the selection names the implementation; its
    // parameters ride the node's own page (the component's parameter
    // target is the node itself, which is what `details["parameters"]`
    // holds).
    let component = details["node"]["components"]
        .as_array()?
        .iter()
        .find(|component| component["kind"] == "light")?;
    let implementation = component["implementation"].as_str()?;
    let values = light_values(implementation, &details["parameters"])?;
    // The world transform comes the gizmo's way: a selection without one
    // usable world matrix draws nothing.
    let target = super::gizmo::target(details).ok()?;
    let rows = details["parameters"]["parameters"].as_array()?;
    Some(LightTarget {
        target,
        values,
        parameter_target: component["parameter_target"].clone(),
        coneangle: spelling(rows, CONEANGLE, "float"),
        conedelta: spelling(rows, CONEDELTA, "float"),
        to: spelling(rows, TO, "point3"),
    })
}

/// What one finished spot drag settled on: `None` until the drag
/// moves that value.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct SpotEdits {
    pub coneangle_degrees: Option<f64>,
    pub conedelta_degrees: Option<f64>,
    pub to: Option<[f64; 3]>,
}

/// The light as the drag presents it: only what the drag has moved
/// changes — the drawn lines follow exactly what release commits.
pub fn apply_edits(values: &LightValues, edits: &SpotEdits) -> LightValues {
    let mut values = *values;
    if let Some(degrees) = edits.coneangle_degrees {
        values.coneangle_degrees = degrees;
    }
    if let Some(degrees) = edits.conedelta_degrees {
        values.conedelta_degrees = degrees;
    }
    if let Some(to) = edits.to {
        values.to = to;
    }
    values
}

impl LightTarget {
    /// One `set_parameter` edit in the page's own spelling, shaped
    /// like the Inspector's own commits (`parameter_rows::set_edit`).
    fn set_edit(&self, spelling: &Spelling, floats: Vec<f64>) -> Value {
        json!({"kind":"set_parameter","target":self.parameter_target,
               "name":spelling.name,"value_type":spelling.value_type,
               "value":{"name":spelling.name,"value_type":spelling.value_type,
                        "color_space":null,
                        "values":{"kind":"floats","value":floats}}})
    }

    /// The edits a finished drag commits — one per value the drag really
    /// moved; several ride one Apply, so one Undo returns them together
    /// A drag that changed nothing edits nothing.
    pub fn spot_edits(&self, edits: &SpotEdits) -> Vec<Value> {
        // A pointer drag has no sub-pixel meaning: commit angles to 0.01°
        // and `to` to 5 significant digits of |to - from| (relative, so a
        // short direction never rounds to zero) so the Inspector and the
        // export stay readable.
        let round = |value: f64, per_unit: f64| (value * per_unit).round() / per_unit;
        let from = self.values.from;
        let edits = SpotEdits {
            coneangle_degrees: edits.coneangle_degrees.map(|d| round(d, 100.)),
            conedelta_degrees: edits.conedelta_degrees.map(|d| round(d, 100.)),
            to: edits.to.map(|to| {
                let reach = to
                    .iter()
                    .zip(from)
                    .map(|(t, f)| (t - f).powi(2))
                    .sum::<f64>()
                    .sqrt();
                if reach > 0. && reach.is_finite() {
                    let per_unit = 10f64.powi(4 - reach.log10().floor() as i32);
                    to.map(|v| round(v, per_unit))
                } else {
                    to
                }
            }),
        };
        let mut out = Vec::new();
        let values = &self.values;
        if let Some(degrees) = edits.coneangle_degrees
            && (degrees - values.coneangle_degrees).abs() > 1e-9
        {
            out.push(self.set_edit(&self.coneangle, vec![degrees]));
        }
        if let Some(degrees) = edits.conedelta_degrees
            && (degrees - values.conedelta_degrees).abs() > 1e-9
        {
            out.push(self.set_edit(&self.conedelta, vec![degrees]));
        }
        if let Some(to) = edits.to
            && to
                .iter()
                .zip(values.to)
                .any(|(moved, was)| (moved - was).abs() > 1e-9)
        {
            out.push(self.set_edit(&self.to, to.to_vec()));
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn details(components: Value, parameters: Value) -> Value {
        json!({"node":{
            "node_id":"n","object":"light","project":{"revision":1},
            "components":components,
        },"parameters":parameters})
    }

    fn transform_component() -> Value {
        json!({"kind":"transform",
            "world":{"start":[1.,0.,0.,2., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.],
                     "end":[1.,0.,0.,2., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.]},
            "local":{"matrix":[1.,0.,0.,2., 0.,1.,0.,0., 0.,0.,1.,0., 0.,0.,0.,1.],
                      "parent":"root","trs":{"translation":[2.,0.,0.],
                      "rotation_euler_degrees":[0.,0.,0.],"scale":[1.,1.,1.]}}})
    }

    fn light_component(implementation: &str) -> Value {
        json!({"kind":"light","implementation":implementation,
               "parameter_target":{"kind":"node","id":"n"}})
    }

    fn page(rows: Value) -> Value {
        json!({"parameters":rows})
    }

    #[test]
    fn a_light_node_becomes_a_wire_frame_target() {
        let details = details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"coneangle","value_type":"float",
                 "values":{"kind":"floats","value":[40.]}},
                {"name":"conedelta","value_type":"float",
                 "values":{"kind":"floats","value":[12.]}}])),
        );
        let light = light(&details).unwrap();
        assert_eq!(
            light.values.kind,
            scene_view::preview::light_gizmo::LightKind::Spot
        );
        assert!((light.values.coneangle_degrees - 40.).abs() < 1e-9);
        assert!((light.values.conedelta_degrees - 12.).abs() < 1e-9);
        assert_eq!(light.target.node_id, "n");
        assert_eq!(light.target.origin(), [2., 0., 0.]);
    }

    #[test]
    fn lights_without_a_frame_draw_nothing() {
        let frame = details(
            json!([transform_component(), light_component("infinite")]),
            page(Value::Array(Vec::new())),
        );
        assert!(light(&frame).is_none(), "an infinite light is all of space");
        // An area light rides a shape as an `area_light` component, never
        // a `light` one.
        let mut shape = frame.clone();
        shape["node"]["object"] = json!("shape");
        shape["node"]["components"] = json!([transform_component(),
            {"kind":"area_light","node_id":"a","implementation":"diffuse"}]);
        assert!(light(&shape).is_none());
        // A node with no light component at all.
        let mut camera = frame;
        camera["node"]["object"] = json!("camera");
        camera["node"]["components"] = json!([transform_component()]);
        assert!(light(&camera).is_none());
    }

    #[test]
    fn a_light_without_an_editable_transform_draws_nothing() {
        let mut frame = details(
            json!([transform_component(), light_component("point")]),
            page(Value::Array(Vec::new())),
        );
        frame["node"]["components"][0]["world"]["end"] = json!([
            2., 0., 0., 0., 0., 2., 0., 0., 0., 0., 2., 0., 0., 0., 0., 1.
        ]);
        // An animated transform has no single world matrix to draw under.
        assert!(light(&frame).is_none());
    }

    #[test]
    fn the_drag_presentation_applies_only_the_moved_values() {
        let target = light(&details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"from","value_type":"point3",
                 "values":{"kind":"floats","value":[0.,0.,4.]}},
                {"name":"to","value_type":"point3",
                 "values":{"kind":"floats","value":[0.,0.,0.]}},
                {"name":"coneangle","value_type":"float",
                 "values":{"kind":"floats","value":[25.]}},
                {"name":"conedelta","value_type":"float",
                 "values":{"kind":"floats","value":[10.]}},
            ])),
        ))
        .unwrap();
        let base = target.values;
        // Nothing moved: the drag's picture is the page's own.
        assert_eq!(apply_edits(&base, &SpotEdits::default()), base);
        let dragged = apply_edits(
            &base,
            &SpotEdits {
                coneangle_degrees: Some(40.),
                to: Some([1., 0., 0.]),
                ..Default::default()
            },
        );
        assert!((dragged.coneangle_degrees - 40.).abs() < 1e-9);
        // conedelta untouched by this drag keeps the page's value.
        assert!((dragged.conedelta_degrees - base.conedelta_degrees).abs() < 1e-9);
        assert_eq!(dragged.to, [1., 0., 0.]);
        assert_eq!(dragged.from, base.from);
    }

    #[test]
    fn cone_drags_edit_the_declared_spelling() {
        let target = light(&details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"coneangle","value_type":"float",
                 "values":{"kind":"floats","value":[30.]}},
                {"name":"conedeltaangle","value_type":"float",
                 "values":{"kind":"floats","value":[5.]}},
            ])),
        ))
        .unwrap();
        assert_eq!(target.coneangle.name, "coneangle");
        // pbrt-v4 spells the falloff `conedeltaangle`; a declaration
        // stays under its own name.
        assert_eq!(target.conedelta.name, "conedeltaangle");
        // An undeclared `to` commits the table's canonical spelling.
        assert_eq!(target.to.name, "to");
        assert_eq!(target.to.value_type, "point3");
        let edits = target.spot_edits(&SpotEdits {
            conedelta_degrees: Some(8.),
            ..Default::default()
        });
        // Only the moved value commits; the spelling and the target are
        // the wire's own.
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["kind"], "set_parameter");
        assert_eq!(edits[0]["name"], "conedeltaangle");
        assert_eq!(edits[0]["value_type"], "float");
        assert_eq!(edits[0]["target"], json!({"kind":"node","id":"n"}));
        assert_eq!(
            edits[0]["value"],
            json!({"name":"conedeltaangle","value_type":"float","color_space":null,
                   "values":{"kind":"floats","value":[8.]}})
        );
    }

    #[test]
    fn a_tip_edit_moves_to_as_three_floats() {
        let target = light(&details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"to","value_type":"point3",
                 "values":{"kind":"floats","value":[0., 0., 1.]}},
            ])),
        ))
        .unwrap();
        let to = SpotEdits {
            to: Some([1., 0., 0.]),
            ..Default::default()
        };
        let edits = target.spot_edits(&to);
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["name"], "to");
        assert_eq!(edits[0]["value_type"], "point3");
        assert_eq!(
            edits[0]["value"]["values"],
            json!({"kind":"floats","value":[1., 0., 0.]})
        );
    }

    /// A Delta drag also updates the marker ring — drawn around
    /// the axis when the page's falloff is closed. The marker's own
    /// radius counts as the axis, so an unmoved press leaves conedelta
    /// at coneangle and, with the page already there, commits nothing.
    #[test]
    fn a_delta_drag_answers_the_marker_ring_when_the_falloff_is_closed() {
        let coneangle = 45.;
        let marker = (7f64 / 90.).atan().to_degrees();
        assert!(
            (conedelta_complement(marker, coneangle, 0.) - coneangle).abs() < 1e-12,
            "an unmoved press leaves conedelta == coneangle"
        );
        assert!(
            (conedelta_complement(marker + 10., coneangle, -15.) - 35.).abs() < 1e-12,
            "moving outward from the marker lowers conedelta"
        );
        // An open falloff keeps the inner circle's complement.
        assert!((conedelta_complement(30., coneangle, 25.) - 15.).abs() < 1e-12);
        // The unmoved press commits nothing when the page is already at
        // coneangle.
        let target = light(&details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"coneangle","value_type":"float",
                 "values":{"kind":"floats","value":[45.]}},
                {"name":"conedelta","value_type":"float",
                 "values":{"kind":"floats","value":[45.]}},
            ])),
        ))
        .unwrap();
        assert!(
            target
                .spot_edits(&SpotEdits {
                    conedelta_degrees: Some(45.),
                    ..Default::default()
                })
                .is_empty()
        );
    }

    /// A distant light's tip edit commits `to` alone, keeping
    /// |to − from| by the drag's own maths, in the page's spelling.
    #[test]
    fn a_distant_tip_edit_moves_to_as_three_floats() {
        let target = light(&details(
            json!([transform_component(), light_component("distant")]),
            page(json!([
                {"name":"from","value_type":"point3",
                 "values":{"kind":"floats","value":[1., 0., 0.]}},
            ])),
        ))
        .unwrap();
        assert_eq!(
            target.values.kind,
            scene_view::preview::light_gizmo::LightKind::Distant
        );
        // An undeclared `to` commits the table's canonical spelling.
        assert_eq!(target.to.name, "to");
        assert_eq!(target.to.value_type, "point3");
        let edits = target.spot_edits(&SpotEdits {
            to: Some([2., 0., 4.]),
            ..Default::default()
        });
        assert_eq!(edits.len(), 1);
        assert_eq!(edits[0]["name"], "to");
        assert_eq!(edits[0]["value_type"], "point3");
        assert_eq!(
            edits[0]["value"]["values"],
            json!({"kind":"floats","value":[2., 0., 4.]})
        );
        // A drag that keeps the page's own `to` commits nothing.
        assert!(
            target
                .spot_edits(&SpotEdits {
                    to: Some([0., 0., 1.]),
                    ..Default::default()
                })
                .is_empty()
        );
    }

    #[test]
    fn a_drag_that_changed_nothing_edits_nothing() {
        let target = light(&details(
            json!([transform_component(), light_component("spot")]),
            page(json!([
                {"name":"coneangle","value_type":"float",
                 "values":{"kind":"floats","value":[30.]}},
                {"name":"conedelta","value_type":"float",
                 "values":{"kind":"floats","value":[5.]}},
            ])),
        ))
        .unwrap();
        assert!(target.spot_edits(&SpotEdits::default()).is_empty());
        // A drag commits rounded values: 0.01° and 1e-4 for `to`; a
        // wiggle that rounds back to the page's value commits nothing.
        let rounded = target.spot_edits(&SpotEdits {
            coneangle_degrees: Some(41.434_860_126),
            to: Some([1.202_345_6, -1.764_449, 2.0]),
            ..Default::default()
        });
        assert_eq!(
            rounded[0]["value"]["values"]["value"][0].as_f64(),
            Some(41.43)
        );
        let to = rounded[1]["value"]["values"]["value"].as_array().unwrap();
        assert!((to[0].as_f64().unwrap() - 1.2023).abs() < 1e-12);
        assert!((to[1].as_f64().unwrap() + 1.7644).abs() < 1e-12);
        // A tiny direction keeps its precision: rounding is relative to
        // |to - from| and never collapses `to` onto `from`.
        let tiny = target.spot_edits(&SpotEdits {
            to: Some([0.000_006_123, 0., 0.000_008_456]),
            ..Default::default()
        });
        let to = tiny[0]["value"]["values"]["value"].as_array().unwrap();
        assert!((to[0].as_f64().unwrap() - 0.000_006_123).abs() < 1e-12);
        assert!((to[2].as_f64().unwrap() - 0.000_008_456).abs() < 1e-12);
        assert!(
            target
                .spot_edits(&SpotEdits {
                    coneangle_degrees: Some(30.001),
                    ..Default::default()
                })
                .is_empty()
        );
        // A value that came back to where it started does not commit
        // either.
        assert!(
            target
                .spot_edits(&SpotEdits {
                    coneangle_degrees: Some(30.),
                    ..Default::default()
                })
                .is_empty()
        );
        // A cone drag beside a tip drag commits both, in one Apply's
        // order: cone first.
        let edits = target.spot_edits(&SpotEdits {
            coneangle_degrees: Some(40.),
            to: Some([1., 0., 0.]),
            ..Default::default()
        });
        assert_eq!(
            edits
                .iter()
                .map(|edit| edit["name"].clone())
                .collect::<Vec<_>>(),
            vec![json!("coneangle"), json!("to")]
        );
    }
}
