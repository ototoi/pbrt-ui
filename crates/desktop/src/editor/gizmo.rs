//! This module defines what the Scene gizmo moves and
//! the one edit a drag commits. The core re-checks every edit.
use project::math::{Mat4, inverse};
use project::transform_trs::decompose;
use scene_view::preview::gizmo::Drag;
use serde_json::{Value, json};

/// The form id of gizmo commits, so a refusal can put the preview back.
pub(crate) const GIZMO_FORM: &str = "gizmo";

#[derive(Clone, Debug, PartialEq)]
pub struct Target {
    pub node_id: String,
    pub project: Value,
    pub world: Mat4,
    /// Optional world-space pivot used by Center mode.
    pub pivot: Option<[f64; 3]>,
    /// The transform parent's world: identity when `local.parent` is root.
    pub parent: Mat4,
    /// The current TRS, sent back unchanged for the parts a move keeps.
    pub trs: Option<Value>,
}

/// The selected node as a gizmo target, or why it has no gizmo.
pub fn target(details: &Value) -> Result<Target, String> {
    let node = &details["node"];
    if !matches!(
        node["object"].as_str(),
        Some("group" | "shape" | "light" | "instance")
    ) {
        return Err("Select a group, shape, light or instance to move it".into());
    }
    let component = node["components"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|component| component["kind"] == "transform")
        .ok_or("The selection has no transform")?;
    let local = &component["local"];
    let local_matrix = matrix(&local["matrix"]).ok_or_else(|| {
        format!(
            "Transform is not editable: {}",
            local["matrix_reason"]
                .as_str()
                .unwrap_or("no single local matrix")
        )
    })?;
    let world = &component["world"];
    let start = matrix(&world["start"]).ok_or("The selection has no world transform")?;
    if matrix(&world["end"]) != Some(start) {
        return Err("Transform is animated".into());
    }
    let parent = if local["parent"] == "root" {
        Mat4::identity()
    } else {
        start.compose(inverse(local_matrix).ok_or("Transform is singular")?)
    };
    Ok(Target {
        node_id: node["node_id"].as_str().unwrap_or_default().to_owned(),
        project: node["project"].clone(),
        world: start,
        pivot: None,
        parent,
        trs: local["trs"].is_object().then(|| local["trs"].clone()),
    })
}

fn matrix(value: &Value) -> Option<Mat4> {
    let values: Vec<f64> = value
        .as_array()?
        .iter()
        .map(Value::as_f64)
        .collect::<Option<_>>()?;
    Some(Mat4(values.try_into().ok()?))
}

impl Target {
    pub fn origin(&self) -> [f64; 3] {
        let m = &self.world.0;
        [m[3], m[7], m[11]]
    }
    /// The node's own axes (the world matrix's columns, normalised), or the
    /// world axes when one has collapsed (a zero scale).
    pub fn local_axes(&self) -> [[f64; 3]; 3] {
        let m = &self.world.0;
        let columns: Vec<[f64; 3]> = (0..3)
            .filter_map(|j| {
                let c = [m[j], m[4 + j], m[8 + j]];
                let length = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
                (length > 1e-9).then(|| [c[0] / length, c[1] / length, c[2] / length])
            })
            .collect();
        columns
            .try_into()
            .unwrap_or([[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]])
    }
    /// The world delta `D` of a drag: `world' = D · world`, which the
    /// preview left-multiplies into the selection's draws.
    pub fn delta_matrix(&self, drag: Drag) -> Mat4 {
        match drag {
            Drag::Move(d) => Mat4::translation(d[0], d[1], d[2]),
            Drag::Rotate { axis, degrees } => {
                let o = self.pivot.unwrap_or_else(|| self.origin());
                Mat4::translation(o[0], o[1], o[2])
                    .compose(Mat4::rotation(degrees, axis[0], axis[1], axis[2]))
                    .compose(Mat4::translation(-o[0], -o[1], -o[2]))
            }
            // Scale in the object's own frame: `world · diag · world⁻¹`.
            Drag::Scale(f) => inverse(self.world)
                .map(|wi| {
                    let scale = self
                        .world
                        .compose(Mat4::scale(f[0], f[1], f[2]))
                        .compose(wi);
                    let origin = self.origin();
                    let centre = self.pivot.unwrap_or(origin);
                    Mat4::translation(
                        centre[0] - origin[0],
                        centre[1] - origin[1],
                        centre[2] - origin[2],
                    )
                    .compose(scale)
                    .compose(Mat4::translation(
                        origin[0] - centre[0],
                        origin[1] - centre[1],
                        origin[2] - centre[2],
                    ))
                })
                .unwrap_or_else(Mat4::identity),
        }
    }
    /// The edit a drag commits: `L' = P⁻¹ · D · W`. A move keeps rotation
    /// and scale exactly; a rotate or scale re-derives the TRS (or sends a
    /// matrix when the local transform has none, e.g. a shared shear).
    pub fn edit(&self, drag: Drag) -> Result<Value, String> {
        let parent = inverse(self.parent).ok_or("The parent transform is singular")?;
        let local = parent.compose(self.delta_matrix(drag).compose(self.world));
        let m = local.0;
        if m.iter().any(|v| !v.is_finite()) {
            return Err("The move leaves the finite range".into());
        }
        let trs = match (&self.trs, drag) {
            (Some(trs), Drag::Move(_)) => json!({
                "translation":[m[3], m[7], m[11]],
                "rotation_euler_degrees":trs["rotation_euler_degrees"],
                "scale":trs["scale"],
            }),
            (Some(_), _) => match decompose(&local) {
                Ok(trs) => json!({
                    "translation":trs.translation,
                    "rotation_euler_degrees":trs.rotation_euler_degrees,
                    "scale":trs.scale,
                }),
                Err(_) => return Ok(matrix_edit(&self.node_id, m)),
            },
            (None, _) => return Ok(matrix_edit(&self.node_id, m)),
        };
        Ok(json!({
            "kind":"set_local_trs","node_id":self.node_id,
            "translation":trs["translation"],
            "rotation_euler_degrees":trs["rotation_euler_degrees"],
            "scale":trs["scale"],
        }))
    }
}

fn matrix_edit(node_id: &str, matrix: [f64; 16]) -> Value {
    json!({"kind":"set_local_matrix","node_id":node_id,"matrix":matrix})
}

#[cfg(test)]
mod tests {
    use super::{Drag, Mat4, inverse, matrix, target};
    use serde_json::{Value, json};

    fn details(world: Mat4, local: Mat4, parent: &str, trs: Value) -> Value {
        json!({"node":{
            "node_id":"n","object":"shape","project":{"revision":1},
            "components":[{"kind":"transform",
                "world":{"start":world.0,"end":world.0},
                "local":{"matrix":local.0,"parent":parent,"trs":trs}}],
        }})
    }

    #[test]
    fn a_move_under_a_rotated_parent_changes_only_the_position() {
        let parent = Mat4::translation(5., 0., 0.).compose(Mat4::rotation(90., 0., 0., 1.));
        let local = Mat4::translation(1., 0., 0.).compose(Mat4::scale(2., 2., 2.));
        let trs = json!({"translation":[1.,0.,0.],"rotation_euler_degrees":[0.,0.,0.],"scale":[2.,2.,2.]});
        let t = target(&details(parent.compose(local), local, "group", trs)).unwrap();
        assert!((t.origin()[1] - 1.).abs() < 1e-12);
        // World +x under a parent turned 90° about z is the local -y.
        let edit = t.edit(Drag::Move([3., 0., 0.])).unwrap();
        assert_eq!(edit["kind"], "set_local_trs");
        let p = edit["translation"].as_array().unwrap();
        let p: Vec<f64> = p.iter().map(|v| v.as_f64().unwrap()).collect();
        assert!((p[0] - 1.).abs() < 1e-12 && (p[1] + 3.).abs() < 1e-12 && p[2].abs() < 1e-12);
        assert_eq!(edit["scale"], json!([2., 2., 2.]));
    }

    #[test]
    fn a_root_parented_run_moves_in_world_space() {
        let world = Mat4::translation(3., 0.5, 0.);
        let trs = json!({"translation":[3.,0.5,0.],"rotation_euler_degrees":[0.,0.,0.],"scale":[1.,1.,1.]});
        let t = target(&details(world, world, "root", trs)).unwrap();
        let edit = t.edit(Drag::Move([0., 1., 0.])).unwrap();
        assert_eq!(edit["translation"], json!([3., 1.5, 0.]));
    }

    #[test]
    fn a_shear_moves_as_a_matrix_and_reasons_block_the_gizmo() {
        let mut shear = Mat4::identity();
        shear.0[1] = 0.5;
        let t = target(&details(shear, shear, "group", Value::Null)).unwrap();
        let edit = t.edit(Drag::Move([1., 2., 3.])).unwrap();
        assert_eq!(edit["kind"], "set_local_matrix");
        assert_eq!(edit["matrix"][3], json!(1.));
        let mut animated = details(shear, shear, "group", Value::Null);
        animated["node"]["components"][0]["local"]["matrix"] = Value::Null;
        animated["node"]["components"][0]["local"]["matrix_reason"] = json!("animated");
        assert!(target(&animated).unwrap_err().contains("animated"));
        let mut camera = details(shear, shear, "group", Value::Null);
        camera["node"]["object"] = json!("camera");
        assert!(target(&camera).is_err());
    }

    fn vec3(value: &Value) -> Vec<f64> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_f64().unwrap())
            .collect()
    }

    #[test]
    fn a_rotate_keeps_position_and_scale() {
        let world = Mat4::translation(1., 2., 3.);
        let trs = json!({"translation":[1.,2.,3.],"rotation_euler_degrees":[0.,0.,0.],"scale":[1.,1.,1.]});
        let t = target(&details(world, world, "root", trs)).unwrap();
        // A turn about the node's own origin: position stays put.
        let edit = t
            .edit(Drag::Rotate {
                axis: [0., 0., 1.],
                degrees: 90.,
            })
            .unwrap();
        assert_eq!(edit["kind"], "set_local_trs");
        let p = vec3(&edit["translation"]);
        assert!((p[0] - 1.).abs() < 1e-9 && (p[1] - 2.).abs() < 1e-9 && (p[2] - 3.).abs() < 1e-9);
        assert_eq!(vec3(&edit["scale"]), vec![1., 1., 1.]);
        assert!((vec3(&edit["rotation_euler_degrees"])[2] - 90.).abs() < 1e-6);
    }

    #[test]
    fn a_local_scale_changes_only_the_scale() {
        let world = Mat4::translation(1., 0., 0.).compose(Mat4::scale(2., 2., 2.));
        let trs = json!({"translation":[1.,0.,0.],"rotation_euler_degrees":[0.,0.,0.],"scale":[2.,2.,2.]});
        let t = target(&details(world, world, "root", trs)).unwrap();
        let edit = t.edit(Drag::Scale([3., 1., 1.])).unwrap();
        assert_eq!(edit["kind"], "set_local_trs");
        assert_eq!(vec3(&edit["translation"]), vec![1., 0., 0.]);
        let s = vec3(&edit["scale"]);
        assert!((s[0] - 6.).abs() < 1e-9 && (s[1] - 2.).abs() < 1e-9 && (s[2] - 2.).abs() < 1e-9);
    }

    #[test]
    fn a_world_rotate_under_a_nonuniform_parent_becomes_a_matrix() {
        // parent = world · local⁻¹ = scale; a rotation shears the local.
        let world = Mat4::scale(2., 1., 1.);
        let trs = json!({"translation":[0.,0.,0.],"rotation_euler_degrees":[0.,0.,0.],"scale":[1.,1.,1.]});
        let t = target(&details(world, Mat4::identity(), "group", trs)).unwrap();
        let edit = t
            .edit(Drag::Rotate {
                axis: [0., 0., 1.],
                degrees: 45.,
            })
            .unwrap();
        assert_eq!(edit["kind"], "set_local_matrix");
    }

    #[test]
    fn local_axes_follow_the_world_rotation() {
        let world = Mat4::rotation(90., 0., 0., 1.).compose(Mat4::scale(3., 1., 1.));
        let t = target(&details(world, world, "root", Value::Null)).unwrap();
        let axes = t.local_axes();
        assert!((axes[0][1] - 1.).abs() < 1e-12 && axes[0][0].abs() < 1e-12);
        assert!((axes[1][0] + 1.).abs() < 1e-12);
    }

    #[test]
    fn center_rotate_and_scale_keep_the_selection_centre_fixed_under_a_parent() {
        let parent = Mat4::translation(4., -2., 1.).compose(Mat4::rotation(37., 0., 0., 1.));
        let local = Mat4::translation(2., 1., -3.)
            .compose(Mat4::rotation(21., 0., 1., 0.))
            .compose(Mat4::scale(2., 3., 1.));
        let world = parent.compose(local);
        let mut target = target(&details(world, local, "group", Value::Null)).unwrap();
        let centre = [3.5, -1.25, 2.];
        target.pivot = Some(centre);

        for drag in [
            Drag::Rotate {
                axis: [0., 0., 1.],
                degrees: 63.,
            },
            Drag::Scale([1.7, 0.6, 1.2]),
        ] {
            let delta = target.delta_matrix(drag);
            let moved = delta.transform_point(centre);
            for axis in 0..3 {
                assert!((moved[axis] - centre[axis]).abs() < 1e-9);
            }
            let edit = target.edit(drag).unwrap();
            assert_eq!(edit["kind"], "set_local_matrix");
            let committed = matrix(&edit["matrix"]).unwrap();
            let expected = inverse(parent).unwrap().compose(delta.compose(world));
            for (actual, expected) in committed.0.iter().zip(expected.0) {
                assert!((actual - expected).abs() < 1e-9);
            }
        }
    }

    #[test]
    fn move_is_independent_of_the_pivot() {
        let world = Mat4::translation(2., 3., 4.).compose(Mat4::rotation(30., 0., 0., 1.));
        let trs = json!({"translation":[2.,3.,4.],"rotation_euler_degrees":[0.,0.,30.],"scale":[1.,1.,1.]});
        let mut target = target(&details(world, world, "root", trs)).unwrap();
        let drag = Drag::Move([1., -2., 3.]);
        let pivot_mode = target.delta_matrix(drag);
        target.pivot = Some([90., -40., 12.]);
        assert_eq!(pivot_mode, target.delta_matrix(drag));
    }
}
