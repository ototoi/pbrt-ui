use super::matrix4x4::Matrix4x4;
use super::quaternion::Quaternion;
use super::vector3::Vector3;

fn lerp(t: f32, a: f32, b: f32) -> f32 {
    return a + t * (b - a);
}

fn invertible(m: &Matrix4x4) -> bool {
    m.inverse().is_some()
}

fn suppress_for_scale(m: Matrix4x4) -> Matrix4x4 {
    let mut mm = Matrix4x4::identity();
    for i in 0..3 {
        mm.m[4 * i + i] = m.m[4 * i + i];
    }
    return mm;
}

fn length(x: f32, y: f32, z: f32) -> f32 {
    return f32::sqrt(x * x + y * y + z * z);
}

// Decompose a 4x4 matrix into translation, rotation, and scale.
impl Matrix4x4 {
    pub fn decompose(&self, epsilon: f32) -> Option<(Vector3, Quaternion, Vector3)> {
        decompose_matrix(self, epsilon, 100)
    }
}

fn decompose_matrix(
    m: &Matrix4x4,
    epsilon: f32,
    max_count: i32,
) -> Option<(Vector3, Quaternion, Vector3)> {
    // Extract translation _T_ from transformation matrix
    let t = Vector3::new(m.m[4 * 0 + 3], m.m[4 * 1 + 3], m.m[4 * 2 + 3]);

    // Compute new transformation matrix _M_ without translation
    let mut mm = *m;
    for i in 0..3 {
        mm.m[4 * i + 3] = 0.0;
    }
    mm.m[15] = 1.0;

    // Extract rotation _R_ from transformation matrix
    let mut r = mm;
    // pbrt-r3
    let sx = length(r.m[4 * 0 + 0], r.m[4 * 1 + 0], r.m[4 * 2 + 0]);
    let sy = length(r.m[4 * 0 + 1], r.m[4 * 1 + 1], r.m[4 * 2 + 1]);
    let sz = length(r.m[4 * 0 + 2], r.m[4 * 1 + 2], r.m[4 * 2 + 2]);
    if sx != 0.0 {
        r.m[4 * 0 + 0] /= sx;
        r.m[4 * 0 + 1] /= sx;
        r.m[4 * 0 + 2] /= sx;
    }
    if sy != 0.0 {
        r.m[4 * 1 + 0] /= sy;
        r.m[4 * 1 + 1] /= sy;
        r.m[4 * 1 + 2] /= sy;
    }
    if sz != 0.0 {
        r.m[4 * 2 + 0] /= sz;
        r.m[4 * 2 + 1] /= sz;
        r.m[4 * 2 + 2] /= sz;
    }
    // pbrt-r3

    let mut count = 0;
    let mut norm: f32 = 0.0;
    loop {
        // Compute inverse of _R_ and check for singularity
        //assert!(invertible(&r));

        let mut r_next = r;
        if let Some(r_it) = r_next.transpose().inverse() {
            // Compute next matrix _Rnext_ in series
            for i in 0..4 {
                for j in 0..4 {
                    r_next.m[4 * i + j] = lerp(0.5, r_next.m[4 * i + j], r_it.m[4 * i + j]);
                }
            }

            // pbrt-r3
            assert!(invertible(&r_next));
            if let Some(ir_next) = r_next.inverse() {
                let s = suppress_for_scale(ir_next * mm);
                if let Some(is) = s.inverse() {
                    r_next = mm * is;
                }
            }
            let q = Quaternion::from(r_next);
            let q = q.normalize();
            r_next = q.to_matrix();
            // pbrt-r3

            // Compute norm of difference between _R_ and _Rnext_
            for i in 0..3 {
                let n = f32::abs(r.m[4 * i + 0] - r_next.m[4 * i + 0])
                    + f32::abs(r.m[4 * i + 1] - r_next.m[4 * i + 1])
                    + f32::abs(r.m[4 * i + 2] - r_next.m[4 * i + 2]);
                norm = f32::max(norm, n);
            }
            r = r_next;
        } else {
            return None;
        }

        count += 1;
        if !(count < max_count && norm > epsilon) {
            break;
        }
    }

    if let Some(ir) = r.inverse() {
        let s = suppress_for_scale(ir * mm);
        if let Some(is) = s.inverse() {
            r = mm * is;
        }
        let q = Quaternion::from(r);
        let q = q.normalize();
        let s = Vector3::new(s.m[0], s.m[5], s.m[10]);
        return Some((t, q, s));
    }
    return None;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::base::*;

    fn near_equal(a: f32, b: f32, epsilon: f32) -> bool {
        (a - b).abs() < epsilon
    }

    #[test]
    fn test_decompose_001() {
        let m = Matrix4x4::translate(1.0, 2.0, 3.0)
            * Matrix4x4::rotate(1.0, 0.0, 1.0, 0.0)
            * Matrix4x4::scale(-2.0, 2.0, 3.0);
        let (t, q, s) = m.decompose(1e-6).unwrap();
        assert!(near_equal(t.x, 1.0, 1e-6));
        assert!(near_equal(t.y, 2.0, 1e-6));
        assert!(near_equal(t.z, 3.0, 1e-6));

        assert!(near_equal(s.x, -2.0, 1e-6));
        assert!(near_equal(s.y, 2.0, 1e-6));
        assert!(near_equal(s.z, 3.0, 1e-6));
    }

    #[test]
    fn test_decompose_002() {
        let m = Matrix4x4::scale(-1.0, 2.0, 3.0);
        let (t, _, s) = m.decompose(1e-6).unwrap();

        assert!(near_equal(t.x, 0.0, 1e-6));
        assert!(near_equal(t.y, 0.0, 1e-6));
        assert!(near_equal(t.z, 0.0, 1e-6));

        assert!(near_equal(s.x, -1.0, 1e-6));
        assert!(near_equal(s.y, 2.0, 1e-6));
        assert!(near_equal(s.z, 3.0, 1e-6));
    }
}
