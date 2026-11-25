//! Small, explicit row-major affine transform implementation for preview data.

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mat4(pub [f64; 16]);

impl Mat4 {
    pub const fn identity() -> Self {
        Self([
            1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1., 0., 0., 0., 0., 1.,
        ])
    }
    pub fn translation(x: f64, y: f64, z: f64) -> Self {
        let mut m = Self::identity();
        m.0[3] = x;
        m.0[7] = y;
        m.0[11] = z;
        m
    }
    pub fn scale(x: f64, y: f64, z: f64) -> Self {
        Self([x, 0., 0., 0., 0., y, 0., 0., 0., 0., z, 0., 0., 0., 0., 1.])
    }
    pub fn rotation(degrees: f64, x: f64, y: f64, z: f64) -> Self {
        let l = (x * x + y * y + z * z).sqrt();
        if l == 0. || !l.is_finite() {
            return Self::identity();
        }
        let (x, y, z) = (x / l, y / l, z / l);
        let a = degrees.to_radians();
        let (s, c) = a.sin_cos();
        let t = 1. - c;
        Self([
            t * x * x + c,
            t * x * y - s * z,
            t * x * z + s * y,
            0.,
            t * x * y + s * z,
            t * y * y + c,
            t * y * z - s * x,
            0.,
            t * x * z - s * y,
            t * y * z + s * x,
            t * z * z + c,
            0.,
            0.,
            0.,
            0.,
            1.,
        ])
    }
    pub fn mul(self, rhs: Self) -> Self {
        let mut out = [0.; 16];
        for r in 0..4 {
            for c in 0..4 {
                for k in 0..4 {
                    out[r * 4 + c] += self.0[r * 4 + k] * rhs.0[k * 4 + c];
                }
            }
        }
        Self(out)
    }
    pub fn transform_point(self, p: [f64; 3]) -> [f64; 3] {
        let m = self.0;
        [
            m[0] * p[0] + m[1] * p[1] + m[2] * p[2] + m[3],
            m[4] * p[0] + m[5] * p[1] + m[6] * p[2] + m[7],
            m[8] * p[0] + m[9] * p[1] + m[10] * p[2] + m[11],
        ]
    }
}

#[derive(Clone, Copy)]
struct Quaternion {
    x: f64,
    y: f64,
    z: f64,
    w: f64,
}

impl Quaternion {
    fn from_rotation(m: [f64; 16]) -> Option<Self> {
        let trace = m[0] + m[5] + m[10];
        let q = if trace > 0.0 {
            let s = (trace + 1.0).sqrt() * 2.0;
            Self {
                x: (m[9] - m[6]) / s,
                y: (m[2] - m[8]) / s,
                z: (m[4] - m[1]) / s,
                w: s * 0.25,
            }
        } else if m[0] > m[5] && m[0] > m[10] {
            let s = (1.0 + m[0] - m[5] - m[10]).sqrt() * 2.0;
            Self {
                x: s * 0.25,
                y: (m[4] + m[1]) / s,
                z: (m[2] + m[8]) / s,
                w: (m[9] - m[6]) / s,
            }
        } else if m[5] > m[10] {
            let s = (1.0 + m[5] - m[0] - m[10]).sqrt() * 2.0;
            Self {
                x: (m[4] + m[1]) / s,
                y: s * 0.25,
                z: (m[9] + m[6]) / s,
                w: (m[2] - m[8]) / s,
            }
        } else {
            let s = (1.0 + m[10] - m[0] - m[5]).sqrt() * 2.0;
            Self {
                x: (m[2] + m[8]) / s,
                y: (m[9] + m[6]) / s,
                z: s * 0.25,
                w: (m[4] - m[1]) / s,
            }
        };
        let length = (q.x * q.x + q.y * q.y + q.z * q.z + q.w * q.w).sqrt();
        (length.is_finite() && length > 1e-12).then_some(Self {
            x: q.x / length,
            y: q.y / length,
            z: q.z / length,
            w: q.w / length,
        })
    }

    fn dot(self, other: Self) -> f64 {
        self.x * other.x + self.y * other.y + self.z * other.z + self.w * other.w
    }

    fn slerp(self, mut other: Self, t: f64) -> Self {
        let mut cosine = self.dot(other);
        if cosine < 0.0 {
            other = Self {
                x: -other.x,
                y: -other.y,
                z: -other.z,
                w: -other.w,
            };
            cosine = -cosine;
        }
        if cosine > 0.9995 {
            let q = Self {
                x: self.x + t * (other.x - self.x),
                y: self.y + t * (other.y - self.y),
                z: self.z + t * (other.z - self.z),
                w: self.w + t * (other.w - self.w),
            };
            let length = (q.dot(q)).sqrt();
            return Self {
                x: q.x / length,
                y: q.y / length,
                z: q.z / length,
                w: q.w / length,
            };
        }
        let theta = cosine.clamp(-1.0, 1.0).acos();
        let denominator = theta.sin();
        let a = ((1.0 - t) * theta).sin() / denominator;
        let b = (t * theta).sin() / denominator;
        Self {
            x: a * self.x + b * other.x,
            y: a * self.y + b * other.y,
            z: a * self.z + b * other.z,
            w: a * self.w + b * other.w,
        }
    }

    fn matrix(self) -> [f64; 16] {
        let (x, y, z, w) = (self.x, self.y, self.z, self.w);
        [
            1. - 2. * (y * y + z * z),
            2. * (x * y - z * w),
            2. * (x * z + y * w),
            0.,
            2. * (x * y + z * w),
            1. - 2. * (x * x + z * z),
            2. * (y * z - x * w),
            0.,
            2. * (x * z - y * w),
            2. * (y * z + x * w),
            1. - 2. * (x * x + y * y),
            0.,
            0.,
            0.,
            0.,
            1.,
        ]
    }
}

#[derive(Clone, Copy)]
struct TransformParts {
    translation: [f64; 3],
    rotation: Quaternion,
    scale: [f64; 3],
}

impl TransformParts {
    fn decompose(matrix: Mat4) -> Option<Self> {
        let m = matrix.0;
        if m.iter().any(|value| !value.is_finite())
            || m[12].abs() > 1e-10
            || m[13].abs() > 1e-10
            || m[14].abs() > 1e-10
            || (m[15] - 1.0).abs() > 1e-10
        {
            return None;
        }
        let mut rotation = [m[0], m[1], m[2], m[4], m[5], m[6], m[8], m[9], m[10]];
        for _ in 0..100 {
            let determinant = rotation[0] * (rotation[4] * rotation[8] - rotation[5] * rotation[7])
                - rotation[1] * (rotation[3] * rotation[8] - rotation[5] * rotation[6])
                + rotation[2] * (rotation[3] * rotation[7] - rotation[4] * rotation[6]);
            if !determinant.is_finite() || determinant.abs() < 1e-14 {
                return None;
            }
            let inverse_transpose = [
                (rotation[4] * rotation[8] - rotation[5] * rotation[7]) / determinant,
                (rotation[5] * rotation[6] - rotation[3] * rotation[8]) / determinant,
                (rotation[3] * rotation[7] - rotation[4] * rotation[6]) / determinant,
                (rotation[2] * rotation[7] - rotation[1] * rotation[8]) / determinant,
                (rotation[0] * rotation[8] - rotation[2] * rotation[6]) / determinant,
                (rotation[1] * rotation[6] - rotation[0] * rotation[7]) / determinant,
                (rotation[1] * rotation[5] - rotation[2] * rotation[4]) / determinant,
                (rotation[2] * rotation[3] - rotation[0] * rotation[5]) / determinant,
                (rotation[0] * rotation[4] - rotation[1] * rotation[3]) / determinant,
            ];
            let mut next = [0.0; 9];
            let mut delta: f64 = 0.0;
            for row in 0..3 {
                for column in 0..3 {
                    next[row * 3 + column] =
                        0.5 * (rotation[row * 3 + column] + inverse_transpose[row * 3 + column]);
                    delta = delta.max((next[row * 3 + column] - rotation[row * 3 + column]).abs());
                }
            }
            rotation = next;
            if delta < 1e-10 {
                break;
            }
        }
        let scale = [
            rotation[0] * m[0] + rotation[3] * m[4] + rotation[6] * m[8],
            rotation[1] * m[1] + rotation[4] * m[5] + rotation[7] * m[9],
            rotation[2] * m[2] + rotation[5] * m[6] + rotation[8] * m[10],
        ];
        if scale
            .iter()
            .any(|value| !value.is_finite() || value.abs() < 1e-12)
        {
            return None;
        }
        let mut rotation_matrix = Mat4::identity().0;
        for row in 0..3 {
            for column in 0..3 {
                rotation_matrix[row * 4 + column] = rotation[row * 3 + column];
            }
        }
        Some(Self {
            translation: [m[3], m[7], m[11]],
            rotation: Quaternion::from_rotation(rotation_matrix)?,
            scale,
        })
    }

    fn interpolate(self, other: Self, t: f64) -> Mat4 {
        let translation: [f64; 3] = std::array::from_fn(|axis| {
            self.translation[axis] * (1.0 - t) + other.translation[axis] * t
        });
        let scale: [f64; 3] =
            std::array::from_fn(|axis| self.scale[axis] * (1.0 - t) + other.scale[axis] * t);
        let rotation = self.rotation.slerp(other.rotation, t).matrix();
        let mut matrix = [0.0; 16];
        for row in 0..3 {
            for column in 0..3 {
                matrix[row * 4 + column] = rotation[row * 4 + column] * scale[column];
            }
        }
        matrix[3] = translation[0];
        matrix[7] = translation[1];
        matrix[11] = translation[2];
        matrix[15] = 1.0;
        Mat4(matrix)
    }
}

/// Interpolates an affine transform using translation, quaternion rotation and scale.
pub fn interpolate(start: Mat4, end: Mat4, fraction: f64) -> Option<Mat4> {
    if !fraction.is_finite() {
        return None;
    }
    let start_parts = TransformParts::decompose(start)?;
    let end_parts = TransformParts::decompose(end)?;
    Some(start_parts.interpolate(end_parts, fraction.clamp(0.0, 1.0)))
}

#[derive(Clone, Copy, Debug)]
pub struct TransformSet {
    pub start: Mat4,
    pub end: Mat4,
}
impl Default for TransformSet {
    fn default() -> Self {
        Self {
            start: Mat4::identity(),
            end: Mat4::identity(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Mat4, interpolate};

    #[test]
    fn animated_transform_interpolates_translation_rotation_and_scale() {
        let start = Mat4::identity();
        let end = Mat4::translation(10.0, 0.0, 0.0)
            .mul(Mat4::rotation(180.0, 0.0, 0.0, 1.0))
            .mul(Mat4::scale(3.0, 3.0, 3.0));
        let middle = interpolate(start, end, 0.5).unwrap();
        let point = middle.transform_point([1.0, 0.0, 0.0]);
        assert!((point[0] - 5.0).abs() < 1e-8);
        assert!((point[1] - 2.0).abs() < 1e-8);
    }

    #[test]
    fn animated_transform_rejects_singular_endpoints() {
        assert!(interpolate(Mat4::identity(), Mat4::scale(0.0, 1.0, 1.0), 0.5).is_none());
    }
}
