//! The scene-level coordinate system: the up axis a scene is
//! authored around, plus whether its content came from a right-handed
//! source (a mirror in the camera chain) or pbrt's own left-handed
//! convention. It is display and authoring metadata only — it never
//! changes node matrices and is never written to a PBRT export.
use crate::{CameraPose, EntityKind, NodeId, NodeKind};
use serde::{Deserialize, Serialize};

/// One signed world axis. The wire spellings are `+X` … `-Z`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Axis {
    #[serde(rename = "+X")]
    PosX,
    #[serde(rename = "-X")]
    NegX,
    #[serde(rename = "+Y")]
    PosY,
    #[serde(rename = "-Y")]
    NegY,
    #[serde(rename = "+Z")]
    PosZ,
    #[serde(rename = "-Z")]
    NegZ,
}

impl std::fmt::Display for Axis {
    /// The same word serde writes (`+X` … `-Z`); a mismatch breaks the
    /// wire round trip, so the unit tests pin them together.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Axis::PosX => "+X",
            Axis::NegX => "-X",
            Axis::PosY => "+Y",
            Axis::NegY => "-Y",
            Axis::PosZ => "+Z",
            Axis::NegZ => "-Z",
        })
    }
}

impl Axis {
    pub const ALL: [Axis; 6] = [
        Axis::PosX,
        Axis::NegX,
        Axis::PosY,
        Axis::NegY,
        Axis::PosZ,
        Axis::NegZ,
    ];

    /// The same-sign axis that follows in X→Y→Z order: the world direction
    /// a default-placed camera looks along, chosen so `cross(up, successor)`
    /// lands on the remaining positive axis.
    pub fn successor(self) -> Axis {
        match self {
            Axis::PosX => Axis::PosY,
            Axis::NegX => Axis::NegY,
            Axis::PosY => Axis::PosZ,
            Axis::NegY => Axis::NegZ,
            Axis::PosZ => Axis::PosX,
            Axis::NegZ => Axis::NegX,
        }
    }

    /// The unit world vector of this axis.
    pub fn vector(self) -> [f64; 3] {
        match self {
            Axis::PosX => [1., 0., 0.],
            Axis::NegX => [-1., 0., 0.],
            Axis::PosY => [0., 1., 0.],
            Axis::NegY => [0., -1., 0.],
            Axis::PosZ => [0., 0., 1.],
            Axis::NegZ => [0., 0., -1.],
        }
    }

    /// The axis with the largest absolute component of `vector` and that
    /// component's sign. Ties keep the later axis, matching the preview's
    /// `world_up` snap (scene-parse `scene/program.rs`). `None` when the
    /// vector is zero or holds a non-finite component.
    pub fn largest(vector: [f64; 3]) -> Option<Axis> {
        if !vector.iter().all(|value| value.is_finite()) || vector == [0.; 3] {
            return None;
        }
        let axis = (0..3).max_by(|&a, &b| vector[a].abs().total_cmp(&vector[b].abs()))?;
        Some(match (axis, vector[axis] < 0.) {
            (0, false) => Axis::PosX,
            (0, true) => Axis::NegX,
            (1, false) => Axis::PosY,
            (1, true) => Axis::NegY,
            (2, false) => Axis::PosZ,
            (2, true) => Axis::NegZ,
            _ => unreachable!("axis index is 0..=2"),
        })
    }
}

/// Which way a scene's world was authored: pbrt's left-handed convention,
/// or right-handed original content (a mirror in the camera chain, like
/// the classic `Scale -1 1 1` right-handed-tool export).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Handedness {
    Left,
    Right,
}

impl Handedness {
    pub const ALL: [Handedness; 2] = [Handedness::Left, Handedness::Right];
}

/// A scene's up axis and world handedness. New scenes default
/// to `+Y` up, left-handed: it matches pbrt's own camera space and needs
/// no extra transform on export.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CoordinateSystem {
    pub up: Axis,
    pub handedness: Handedness,
}

impl std::fmt::Display for CoordinateSystem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.describe())
    }
}

impl Default for CoordinateSystem {
    fn default() -> Self {
        Self {
            up: Axis::PosY,
            handedness: Handedness::Left,
        }
    }
}

impl CoordinateSystem {
    pub fn new(up: Axis, handedness: Handedness) -> Self {
        Self { up, handedness }
    }

    /// The world vector this scene uses as "up".
    pub fn up_vector(self) -> [f64; 3] {
        self.up.vector()
    }

    /// The words a diagnostic prints: "+Z up, right-handed".
    pub fn describe(self) -> String {
        let handedness = match self.handedness {
            Handedness::Left => "left-handed",
            Handedness::Right => "right-handed",
        };
        format!("{} up, {handedness}", self.up)
    }

    /// Estimate one camera's coordinate system from its pose (the
    /// camera-to-world start sample): up is the world image of camera-space
    /// +y (the `LookAt` up) snapped to its largest axis, handedness flips
    /// when the 3×3 mirrors (a `Scale -1 1 1`-style right-handed export).
    /// `None` when the pose is singular or holds non-finite values.
    pub fn estimate(start: &[f64; 16]) -> Option<Self> {
        if !start.iter().all(|value| value.is_finite()) {
            return None;
        }
        // Column 1: where camera-space +y points in the world.
        let up_world = [start[1], start[5], start[9]];
        let up = Axis::largest(up_world)?;
        let determinant = start[0] * (start[5] * start[10] - start[6] * start[9])
            - start[1] * (start[4] * start[10] - start[6] * start[8])
            + start[2] * (start[4] * start[9] - start[5] * start[8]);
        if determinant == 0. {
            return None;
        }
        Some(Self {
            up,
            handedness: if determinant < 0. {
                Handedness::Right
            } else {
                Handedness::Left
            },
        })
    }
}

/// The result of estimating a scene's coordinate system from its cameras
/// and reports the adopted value, the camera it came from, and every
/// camera whose own estimate disagrees.
#[derive(Clone, Debug, PartialEq)]
pub struct Inference {
    pub coordinate_system: CoordinateSystem,
    /// The camera the estimate came from; `None` when the default was
    /// used (no camera, or no usable pose on the chosen one).
    pub camera: Option<NodeId>,
    /// Cameras whose own estimates differ from `coordinate_system`.
    pub conflicts: Vec<(NodeId, CoordinateSystem)>,
}

/// Infer a scene's coordinate system from its cameras.
/// `cameras` holds the scene's cameras in Options order with their poses;
/// `render` names the render camera. The estimate comes from the render
/// camera — else the first camera — and defaults when that camera has no
/// usable pose. Cameras whose estimates disagree are reported as
/// conflicts for a diagnostic.
pub fn infer(cameras: &[(NodeId, Option<CameraPose>)], render: Option<NodeId>) -> Inference {
    let chosen = render
        .and_then(|id| cameras.iter().position(|(camera, _)| *camera == id))
        .unwrap_or(0);
    let mut coordinate_system = CoordinateSystem::default();
    let mut source = None;
    if let Some((camera, Some(pose))) = cameras.get(chosen)
        && let Some(estimate) = CoordinateSystem::estimate(&pose.start)
    {
        coordinate_system = estimate;
        source = Some(*camera);
    }
    let conflicts = cameras
        .iter()
        .filter_map(|(camera, pose)| {
            let estimate = pose
                .as_ref()
                .and_then(|pose| CoordinateSystem::estimate(&pose.start))?;
            (estimate != coordinate_system).then_some((*camera, estimate))
        })
        .collect();
    Inference {
        coordinate_system,
        camera: source,
        conflicts,
    }
}

/// Infer one scene's coordinate system from its own cameras: the render
/// camera when it is set, else the first camera in Options.
pub fn infer_scene(scene: &crate::model::SceneDocument) -> Inference {
    infer_cameras(&scene.options, &scene.nodes, scene.render_camera)
}

/// `infer_scene` over the pieces a scene reader holds before its
/// `SceneDocument` exists: the Options order, the parsed nodes and the
/// resolved render camera.
pub fn infer_cameras(
    options: &[NodeId],
    nodes: &std::collections::BTreeMap<NodeId, crate::model::Node>,
    render: Option<NodeId>,
) -> Inference {
    let cameras = options
        .iter()
        .filter_map(|id| {
            let NodeKind::Entity {
                entity: EntityKind::Camera,
                pose,
                ..
            } = nodes.get(id).map(|node| &node.data)?
            else {
                return None;
            };
            Some((*id, *pose))
        })
        .collect::<Vec<_>>();
    infer(&cameras, render)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::math::{Mat4, inverse, look_at};
    use crate::{EntityKind, Node, NodeKind};

    fn pose_from_matrix(matrix: Mat4) -> CameraPose {
        CameraPose {
            start: matrix.0,
            end: matrix.0,
        }
    }

    /// The pose a `LookAt eye look up` camera carries after import:
    /// the inverse of the world-to-camera CTM.
    fn look_at_pose(eye: [f64; 3], look: [f64; 3], up: [f64; 3]) -> Option<CameraPose> {
        let args = [
            eye[0], eye[1], eye[2], look[0], look[1], look[2], up[0], up[1], up[2],
        ];
        inverse(look_at(&args)?).map(pose_from_matrix)
    }

    /// A camera-space mirror composed into a pose provides the mirror for
    /// right-handed scenes; `Scale -1 1 1` flips the handedness determinant
    /// the way a right-handed-tool export does).
    fn mirrored(pose: CameraPose, mirror: Mat4) -> CameraPose {
        let matrix = Mat4(pose.start).compose(mirror);
        CameraPose {
            start: matrix.0,
            end: matrix.0,
        }
    }

    #[test]
    fn wire_words_are_signed_axes_and_handedness() {
        assert_eq!(
            serde_json::to_string(&CoordinateSystem::default()).unwrap(),
            r#"{"up":"+Y","handedness":"left"}"#
        );
        for up in Axis::ALL {
            for handedness in Handedness::ALL {
                let value = CoordinateSystem::new(up, handedness);
                let text = serde_json::to_string(&value).unwrap();
                let value_json = serde_json::from_str::<CoordinateSystem>(&text).unwrap();
                assert_eq!(value_json, value);
            }
        }
        assert!(
            serde_json::from_str::<CoordinateSystem>(r#"{"up":"y","handedness":"left"}"#).is_err()
        );
        assert!(
            serde_json::from_str::<CoordinateSystem>(r#"{"up":"+Y","handedness":"top"}"#).is_err()
        );
        assert!(serde_json::from_str::<CoordinateSystem>(r#"{"up":"+Y"}"#).is_err());
    }

    #[test]
    fn largest_axis_picks_abs_and_sign_with_later_axes_winning_ties() {
        assert_eq!(Axis::largest([0., 1., 0.]), Some(Axis::PosY));
        assert_eq!(Axis::largest([0., -2., 1.]), Some(Axis::NegY));
        assert_eq!(Axis::largest([-1., 0., 3.]), Some(Axis::PosZ));
        // The same tie rule as the preview's world-up snap: the later axis.
        assert_eq!(Axis::largest([1., 0., 1.]), Some(Axis::PosZ));
        assert_eq!(Axis::largest([5., 5., 0.]), Some(Axis::PosY));
        assert_eq!(Axis::largest([0.; 3]), None);
        assert_eq!(Axis::largest([f64::NAN, 1., 0.]), None);
        assert_eq!(Axis::largest([f64::INFINITY, 0., 0.]), None);
    }

    #[test]
    fn identity_pose_is_y_up_left_handed() {
        assert_eq!(
            CoordinateSystem::estimate(&Mat4::identity().0),
            Some(CoordinateSystem::default())
        );
    }

    #[test]
    fn y_up_look_at_reads_y_up_left_handed() {
        let pose = look_at_pose([0., 2., -6.], [0., 0.5, 0.], [0., 1., 0.]).unwrap();
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(CoordinateSystem::default())
        );
    }

    #[test]
    fn z_up_look_at_reads_z_up_left_handed() {
        let pose = look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]).unwrap();
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(CoordinateSystem::new(Axis::PosZ, Handedness::Left))
        );
    }

    #[test]
    fn mirrored_x_reads_right_handed_with_its_up_unchanged() {
        // `Scale -1 1 1` in the camera chain (the right-handed export idiom).
        let pose = mirrored(
            look_at_pose([0., 2., -6.], [0., 0.5, 0.], [0., 1., 0.]).unwrap(),
            Mat4::scale(-1., 1., 1.),
        );
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(CoordinateSystem::new(Axis::PosY, Handedness::Right))
        );
    }

    #[test]
    fn mirrored_z_also_reads_right_handed() {
        // `Scale 1 1 -1` mirrors about another plane but still flips the
        // handedness determinant.
        let pose = mirrored(
            look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]).unwrap(),
            Mat4::scale(1., 1., -1.),
        );
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(CoordinateSystem::new(Axis::PosZ, Handedness::Right))
        );
    }

    #[test]
    fn tilted_up_snaps_to_its_largest_component() {
        // An oblique `LookAt` up still decides by its largest component.
        let pose = look_at_pose([0., -10., 0.], [0., 0., 0.], [1., 0., 2.3]).unwrap();
        assert_eq!(
            CoordinateSystem::estimate(&pose.start),
            Some(CoordinateSystem::new(Axis::PosZ, Handedness::Left))
        );
    }

    #[test]
    fn singular_poses_default_instead_of_estimating() {
        // Zero matrix: no usable up column at all.
        assert_eq!(CoordinateSystem::estimate(&[0.; 16]), None);
        // Zero determinant: a collapsed basis.
        assert_eq!(CoordinateSystem::estimate(&Mat4::scale(0., 1., 1.).0), None);
        // A non-finite entry in the pose is singular for our purposes.
        let mut nan = Mat4::identity().0;
        nan[0] = f64::NAN;
        assert_eq!(CoordinateSystem::estimate(&nan), None);
    }

    #[test]
    fn no_camera_infers_the_default_and_marks_it() {
        let inference = infer(&[], None);
        assert_eq!(inference.coordinate_system, CoordinateSystem::default());
        assert_eq!(inference.camera, None);
        assert!(inference.conflicts.is_empty());
    }

    #[test]
    fn the_render_camera_is_preferred_over_the_first_camera() {
        let first = NodeId::new();
        let render = NodeId::new();
        let cameras = [
            (
                render,
                look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
            ),
            (
                first,
                look_at_pose([0., 2., -6.], [0., 0.5, 0.], [0., 1., 0.]),
            ),
        ];
        let inference = infer(&cameras, Some(render));
        assert_eq!(
            inference.coordinate_system,
            CoordinateSystem::new(Axis::PosZ, Handedness::Left)
        );
        assert_eq!(inference.camera, Some(render));
        // The Y-up camera disagrees: reported for a diagnostic.
        assert_eq!(
            inference.conflicts,
            vec![(first, CoordinateSystem::default())]
        );
    }

    #[test]
    fn without_a_render_camera_the_first_camera_is_used() {
        let camera = NodeId::new();
        let cameras = [(
            camera,
            look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
        )];
        let inference = infer(&cameras, None);
        assert_eq!(
            inference.coordinate_system,
            CoordinateSystem::new(Axis::PosZ, Handedness::Left)
        );
        assert_eq!(inference.camera, Some(camera));
    }

    #[test]
    fn an_unusable_pose_on_the_chosen_camera_defaults_and_reports_the_rest() {
        let render = NodeId::new();
        let usable = NodeId::new();
        let cameras = [
            (render, None), // an older import's pose-less camera
            (
                usable,
                look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
            ),
        ];
        let inference = infer(&cameras, Some(render));
        assert_eq!(inference.coordinate_system, CoordinateSystem::default());
        assert_eq!(inference.camera, None);
        // Surfaces the usable camera that disagrees with the adopted default.
        assert_eq!(
            inference.conflicts,
            vec![(usable, CoordinateSystem::new(Axis::PosZ, Handedness::Left))]
        );
    }

    #[test]
    fn interior_cameras_with_the_same_estimate_stay_silent() {
        let first = NodeId::new();
        let render = NodeId::new();
        let cameras = [
            (
                first,
                look_at_pose([0., -10., 3.], [0., 0., 0.], [0., 0., 1.]),
            ),
            (
                render,
                look_at_pose([0., 4., -3.], [0., 0., 0.], [0., 0., 1.]),
            ),
        ];
        let inference = infer(&cameras, Some(render));
        assert_eq!(inference.camera, Some(render));
        assert!(inference.conflicts.is_empty());
    }

    #[test]
    fn diagnose_words_match_the_wire_spellings() {
        assert_eq!(CoordinateSystem::default().describe(), "+Y up, left-handed");
        assert_eq!(
            CoordinateSystem::new(Axis::PosZ, Handedness::Right).describe(),
            "+Z up, right-handed"
        );
        assert_eq!(
            CoordinateSystem::new(Axis::NegX, Handedness::Left).describe(),
            "-X up, left-handed"
        );
    }

    /// A scene document with one posed camera in Options rendering from it.
    fn scene_with_camera(camera: Node) -> crate::model::SceneDocument {
        let mut scene = crate::model::SceneDocument {
            id: crate::model::SceneId::new(),
            label: "Scene".into(),
            location: "Scenes/Scene.scene".into(),
            options: Vec::new(),
            render_camera: None,
            coordinate_system: CoordinateSystem::default(),
            world: NodeId::new(),
            nodes: Default::default(),
        };
        scene.options.push(camera.id);
        scene.render_camera = Some(camera.id);
        scene.nodes.insert(camera.id, camera);
        scene
    }

    fn camera_node(pose: Option<CameraPose>) -> Node {
        Node::new(
            "Camera",
            NodeKind::Entity {
                entity: EntityKind::Camera,
                implementation: "perspective".into(),
                parameters: Vec::new(),
                pose,
            },
        )
    }

    #[test]
    fn infer_scene_reads_the_scene_options_and_render_camera() {
        let z_up = scene_with_camera(camera_node(look_at_pose(
            [0., -10., 3.],
            [0., 0., 0.],
            [0., 0., 1.],
        )));
        let inference = infer_scene(&z_up);
        assert_eq!(
            inference.coordinate_system,
            CoordinateSystem::new(Axis::PosZ, Handedness::Left)
        );
        assert_eq!(inference.camera, z_up.render_camera);
        let mut cameraless = z_up.clone();
        cameraless.options.clear();
        cameraless.render_camera = None;
        let inference = infer_scene(&cameraless);
        assert_eq!(inference.coordinate_system, CoordinateSystem::default());
        assert_eq!(inference.camera, None);
    }
}
