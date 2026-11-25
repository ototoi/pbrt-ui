//! A node's local transform as TRS.
//!
//! With column vectors, `L = T · Ry(y) · Rx(x) · Rz(z) · S`: Euler angles in
//! degrees, applied Z, then X, then Y, with the signs of
//! [`Mat4::rotation`]. Decomposition succeeds only when composing the result
//! reproduces the matrix, so a TRS shown for a matrix is that matrix.
use crate::evaluator::GraphicsState;
use crate::math::{Mat4, inverse};
use serde::Serialize;

#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Trs {
    pub translation: [f64; 3],
    pub rotation_euler_degrees: [f64; 3],
    pub scale: [f64; 3],
}

/// Why a matrix has no TRS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TrsError {
    Shear,
    Projective,
    Singular,
    NonFinite,
}

/// Why a node has no single local matrix.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalError {
    /// The transform or its parent changes over the shutter interval.
    Animated,
    /// The run defines or uses a named coordinate system.
    CoordinateSystem,
    /// The parent transform cannot be inverted.
    Singular,
}

/// `abs <= 1e-9 + 1e-6 * magnitude`, the tolerance of every check here.
fn close(a: f64, b: f64, magnitude: f64) -> bool {
    (a - b).abs() <= 1e-9 + 1e-6 * magnitude
}

type Mat3 = [[f64; 3]; 3];

fn upper(m: &Mat4) -> Mat3 {
    std::array::from_fn(|r| std::array::from_fn(|c| m.0[r * 4 + c]))
}

fn magnitude(a: &Mat3) -> f64 {
    a.iter().flatten().fold(0., |m, v| m.max(v.abs()))
}

fn determinant(a: &Mat3) -> f64 {
    a[0][0] * (a[1][1] * a[2][2] - a[1][2] * a[2][1])
        - a[0][1] * (a[1][0] * a[2][2] - a[1][2] * a[2][0])
        + a[0][2] * (a[1][0] * a[2][1] - a[1][1] * a[2][0])
}

/// The inverse transpose, for the polar iteration.
fn inverse_transpose(a: &Mat3) -> Mat3 {
    let det = determinant(a);
    // Cofactor matrix divided by the determinant is the inverse transpose.
    let c =
        |r0: usize, r1: usize, c0: usize, c1: usize| a[r0][c0] * a[r1][c1] - a[r0][c1] * a[r1][c0];
    [
        [c(1, 2, 1, 2), -c(1, 2, 0, 2), c(1, 2, 0, 1)],
        [-c(0, 2, 1, 2), c(0, 2, 0, 2), -c(0, 2, 0, 1)],
        [c(0, 1, 1, 2), -c(0, 1, 0, 2), c(0, 1, 0, 1)],
    ]
    .map(|row| row.map(|v| v / det))
}

fn wrap_degrees(degrees: f64) -> f64 {
    let wrapped = (degrees + 180.).rem_euclid(360.) - 180.;
    // -0 and rounding at the boundary stay inside [-180, 180).
    if wrapped >= 180. {
        wrapped - 360.
    } else {
        wrapped + 0.
    }
}

/// Euler angles (degrees, Z→X→Y) of a rotation matrix. Near X = ±90° the Z
/// angle is 0 and Y takes the whole rotation about the vertical.
fn euler(r: &Mat3) -> [f64; 3] {
    let cos_x = r[1][0].hypot(r[1][1]);
    let (x, y, z) = if cos_x > 1e-9 {
        (
            (-r[1][2]).atan2(cos_x),
            r[0][2].atan2(r[2][2]),
            r[1][0].atan2(r[1][1]),
        )
    } else {
        ((-r[1][2]).atan2(0.), (-r[2][0]).atan2(r[0][0]), 0.)
    };
    [x, y, z].map(|a| wrap_degrees(a.to_degrees()))
}

/// `T · Ry · Rx · Rz · S`.
pub fn compose(trs: &Trs) -> Mat4 {
    let [x, y, z] = trs.rotation_euler_degrees;
    let [tx, ty, tz] = trs.translation;
    let [sx, sy, sz] = trs.scale;
    Mat4::translation(tx, ty, tz)
        .compose(Mat4::rotation(y, 0., 1., 0.))
        .compose(Mat4::rotation(x, 1., 0., 0.))
        .compose(Mat4::rotation(z, 0., 0., 1.))
        .compose(Mat4::scale(sx, sy, sz))
}

/// Whether two affine matrices agree: the 3×3 part relative to its own size
/// and the translation relative to its own, so a far position does not
/// loosen the check on the rest.
pub fn approx_eq(a: &Mat4, b: &Mat4) -> bool {
    let size = magnitude(&upper(a)).max(magnitude(&upper(b)));
    let reach = [3, 7, 11]
        .iter()
        .fold(0f64, |m, i| m.max(a.0[*i].abs()).max(b.0[*i].abs()));
    (0..16).all(|i| match i {
        3 | 7 | 11 => close(a.0[i], b.0[i], reach),
        12..=15 => close(a.0[i], b.0[i], 1.),
        _ => close(a.0[i], b.0[i], size),
    })
}

/// Decomposes an affine matrix into TRS. A mirror goes into a negative X
/// scale, so the rotation stays proper.
pub fn decompose(m: &Mat4) -> Result<Trs, TrsError> {
    if m.0.iter().any(|v| !v.is_finite()) {
        return Err(TrsError::NonFinite);
    }
    if !(close(m.0[12], 0., 1.) && close(m.0[13], 0., 1.) && close(m.0[14], 0., 1.))
        || !close(m.0[15], 1., 1.)
    {
        return Err(TrsError::Projective);
    }
    let mut a = upper(m);
    let size = magnitude(&a);
    let det = determinant(&a);
    if size == 0. || det.abs() <= 1e-12 * size.powi(3) {
        return Err(TrsError::Singular);
    }
    let mirrored = det < 0.;
    if mirrored {
        for row in &mut a {
            row[0] = -row[0];
        }
    }
    // Polar decomposition A = R · S by Higham's iteration R ← (R + R⁻ᵀ) / 2.
    let mut r = a;
    for _ in 0..64 {
        let it = inverse_transpose(&r);
        let next: Mat3 =
            std::array::from_fn(|i| std::array::from_fn(|j| 0.5 * (r[i][j] + it[i][j])));
        let change = (0..3)
            .flat_map(|i| (0..3).map(move |j| (i, j)))
            .fold(0f64, |m, (i, j)| m.max((next[i][j] - r[i][j]).abs()));
        r = next;
        if change <= 1e-15 {
            break;
        }
    }
    // S = Rᵀ · A must be diagonal; off-diagonal terms are shear.
    let s: Mat3 =
        std::array::from_fn(|i| std::array::from_fn(|j| (0..3).map(|k| r[k][i] * a[k][j]).sum()));
    let diagonal = [s[0][0], s[1][1], s[2][2]];
    let scale_size = diagonal.iter().fold(0f64, |m, v| m.max(v.abs()));
    if (0..3).any(|i| (0..3).any(|j| i != j && !close(s[i][j], 0., scale_size))) {
        return Err(TrsError::Shear);
    }
    let mut scale = diagonal;
    if mirrored {
        scale[0] = -scale[0];
    }
    let trs = Trs {
        translation: [m.0[3], m.0[7], m.0[11]],
        rotation_euler_degrees: euler(&r),
        scale,
    };
    if approx_eq(&compose(&trs), m) {
        Ok(trs)
    } else {
        Err(TrsError::Shear)
    }
}

/// The local matrix of a run of transform operations: `inverse(P) · W`,
/// where `entry` is the state before the run (P) and `exit` the state after
/// it (W), so that `P · L = W`.
pub fn local_matrix(
    entry: &GraphicsState,
    exit: &GraphicsState,
    uses_coordinates: bool,
) -> Result<Mat4, LocalError> {
    if entry.start != entry.end || exit.start != exit.end {
        return Err(LocalError::Animated);
    }
    if uses_coordinates {
        return Err(LocalError::CoordinateSystem);
    }
    let parent = inverse(entry.start).ok_or(LocalError::Singular)?;
    let local = parent.compose(exit.start);
    if approx_eq(&entry.start.compose(local), &exit.start) {
        Ok(local)
    } else {
        Err(LocalError::Singular)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A small deterministic generator, so failures reproduce.
    struct Lcg(u64);
    impl Lcg {
        fn next(&mut self) -> f64 {
            self.0 = self
                .0
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
        fn range(&mut self, lo: f64, hi: f64) -> f64 {
            lo + (hi - lo) * self.next()
        }
    }

    fn assert_round_trip(trs: Trs) {
        let m = compose(&trs);
        let back = decompose(&m).unwrap_or_else(|e| panic!("{trs:?}: {e:?}"));
        assert!(approx_eq(&compose(&back), &m), "{trs:?} -> {back:?}");
        for v in back.rotation_euler_degrees {
            assert!((-180. ..180.).contains(&v), "{back:?}");
        }
    }

    #[test]
    fn random_trs_round_trip() {
        let mut rng = Lcg(7);
        for _ in 0..2000 {
            let trs = Trs {
                translation: std::array::from_fn(|_| rng.range(-1e3, 1e3)),
                rotation_euler_degrees: std::array::from_fn(|_| rng.range(-180., 180.)),
                scale: std::array::from_fn(|_| {
                    let s = rng.range(0.01, 100.);
                    if rng.next() < 0.2 { -s } else { s }
                }),
            };
            assert_round_trip(trs);
        }
    }

    #[test]
    fn plain_values_come_back_unchanged() {
        let trs = Trs {
            translation: [1., 2.8, -3.],
            rotation_euler_degrees: [10., 45., -30.],
            scale: [2., 0.5, 3.],
        };
        let back = decompose(&compose(&trs)).unwrap();
        for i in 0..3 {
            assert!((back.translation[i] - trs.translation[i]).abs() < 1e-12);
            assert!((back.rotation_euler_degrees[i] - trs.rotation_euler_degrees[i]).abs() < 1e-9);
            assert!((back.scale[i] - trs.scale[i]).abs() < 1e-12);
        }
    }

    /// Each axis at ±90° and the gimbal lock at X = ±90°, where Z becomes 0.
    #[test]
    fn right_angles_and_gimbal_lock() {
        for axis in 0..3 {
            for angle in [-90., 90., 180., -180.] {
                let mut rotation = [0.; 3];
                rotation[axis] = angle;
                assert_round_trip(Trs {
                    translation: [0.; 3],
                    rotation_euler_degrees: rotation,
                    scale: [1.; 3],
                });
            }
        }
        for x in [90., -90.] {
            let m = compose(&Trs {
                translation: [0.; 3],
                rotation_euler_degrees: [x, 20., 35.],
                scale: [1.; 3],
            });
            let back = decompose(&m).unwrap();
            assert_eq!(back.rotation_euler_degrees[2], 0., "{back:?}");
            assert!(
                (back.rotation_euler_degrees[0] - x).abs() < 1e-6,
                "{back:?}"
            );
            assert!(approx_eq(&compose(&back), &m));
        }
    }

    /// The angles follow `Mat4::rotation`: Y = 90° turns +X into -Z.
    #[test]
    fn signs_follow_the_evaluator() {
        let m = compose(&Trs {
            translation: [0.; 3],
            rotation_euler_degrees: [0., 90., 0.],
            scale: [1.; 3],
        });
        let p = m.transform_point([1., 0., 0.]);
        assert!((p[2] + 1.).abs() < 1e-12 && p[0].abs() < 1e-12, "{p:?}");
        assert_eq!(m, Mat4::rotation(90., 0., 1., 0.));
    }

    #[test]
    fn a_mirror_goes_into_negative_x_scale() {
        let back = decompose(&Mat4::scale(1., -2., 1.)).unwrap();
        assert!(
            back.scale[0] < 0. && back.scale[1] > 0. && back.scale[2] > 0.,
            "{back:?}"
        );
        assert!(approx_eq(&compose(&back), &Mat4::scale(1., -2., 1.)));
    }

    #[test]
    fn rejects_what_trs_cannot_express() {
        let mut shear = Mat4::identity();
        shear.0[1] = 0.5;
        assert_eq!(decompose(&shear), Err(TrsError::Shear));
        // A rotated non-uniform scale (S · R order) is shear too.
        let skew = Mat4::scale(1., 3., 1.).compose(Mat4::rotation(30., 0., 0., 1.));
        assert_eq!(decompose(&skew), Err(TrsError::Shear));
        let mut projective = Mat4::identity();
        projective.0[14] = 0.1;
        assert_eq!(decompose(&projective), Err(TrsError::Projective));
        assert_eq!(decompose(&Mat4::scale(1., 0., 1.)), Err(TrsError::Singular));
        let mut nan = Mat4::identity();
        nan.0[5] = f64::NAN;
        assert_eq!(decompose(&nan), Err(TrsError::NonFinite));
    }

    /// A far translation does not loosen the shear check.
    #[test]
    fn far_positions_keep_the_shear_check() {
        let mut m = Mat4::translation(1e7, -1e7, 1e7);
        m.0[1] = 1e-3;
        assert_eq!(decompose(&m), Err(TrsError::Shear));
        assert_round_trip(Trs {
            translation: [1e7, -1e7, 1e7],
            rotation_euler_degrees: [10., 20., 30.],
            scale: [1.; 3],
        });
    }

    fn state(m: Mat4) -> GraphicsState {
        GraphicsState {
            start: m,
            end: m,
            ..Default::default()
        }
    }

    #[test]
    fn local_matrix_undoes_the_parent() {
        let parent = Mat4::translation(5., 0., 0.)
            .compose(Mat4::rotation(30., 0., 1., 0.))
            .compose(Mat4::scale(2., 2., 2.));
        let local = Mat4::translation(1., 2., 3.).compose(Mat4::rotation(45., 1., 0., 0.));
        let found = local_matrix(&state(parent), &state(parent.compose(local)), false).unwrap();
        assert!(approx_eq(&found, &local));
        // A replacing run (PBRT Identity) under a moved parent: the local
        // matrix is whatever takes the parent to the result.
        let found = local_matrix(&state(parent), &state(Mat4::identity()), false).unwrap();
        assert!(approx_eq(&parent.compose(found), &Mat4::identity()));
    }

    #[test]
    fn local_matrix_reports_what_it_cannot_express() {
        let mut moving = state(Mat4::identity());
        moving.end = Mat4::translation(1., 0., 0.);
        assert_eq!(
            local_matrix(&moving, &moving, false),
            Err(LocalError::Animated)
        );
        assert_eq!(
            local_matrix(&state(Mat4::identity()), &moving, false),
            Err(LocalError::Animated)
        );
        let still = state(Mat4::identity());
        assert_eq!(
            local_matrix(&still, &still, true),
            Err(LocalError::CoordinateSystem)
        );
        assert_eq!(
            local_matrix(&state(Mat4::scale(0., 1., 1.)), &still, false),
            Err(LocalError::Singular)
        );
    }
}
