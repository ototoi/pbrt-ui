//! Small, explicit row-major affine transform implementation for native scene evaluation.

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
        let magnitude = x.abs().max(y.abs()).max(z.abs());
        if magnitude == 0. || !magnitude.is_finite() {
            return Self::identity();
        }
        let (x, y, z) = (x / magnitude, y / magnitude, z / magnitude);
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
    pub fn compose(self, rhs: Self) -> Self {
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

pub fn look_at(v: &[f64; 9]) -> Option<Mat4> {
    fn normalize(v: [f64; 3]) -> Option<[f64; 3]> {
        let n = v.iter().map(|x| x * x).sum::<f64>().sqrt();
        (n > 0. && n.is_finite()).then(|| v.map(|x| x / n))
    }
    fn cross(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
        [
            a[1] * b[2] - a[2] * b[1],
            a[2] * b[0] - a[0] * b[2],
            a[0] * b[1] - a[1] * b[0],
        ]
    }
    let forward = normalize([v[3] - v[0], v[4] - v[1], v[5] - v[2]])?;
    let right = normalize(cross(normalize([v[6], v[7], v[8]])?, forward))?;
    let up = cross(forward, right);
    let mut matrix = Mat4::identity();
    for (row, axis) in [right, up, forward].into_iter().enumerate() {
        for (col, value) in axis.iter().enumerate() {
            matrix.0[row * 4 + col] = *value;
        }
        matrix.0[row * 4 + 3] = -(0..3).map(|col| axis[col] * v[col]).sum::<f64>();
    }
    Some(matrix)
}

pub fn inverse(matrix: Mat4) -> Option<Mat4> {
    let mut rows = [[0.; 8]; 4];
    for row in 0..4 {
        rows[row][..4].copy_from_slice(&matrix.0[row * 4..row * 4 + 4]);
        rows[row][row + 4] = 1.;
    }
    for column in 0..4 {
        let pivot =
            (column..4).max_by(|&a, &b| rows[a][column].abs().total_cmp(&rows[b][column].abs()))?;
        rows.swap(column, pivot);
        let divisor = rows[column][column];
        if divisor == 0. || !divisor.is_finite() {
            return None;
        }
        for value in &mut rows[column] {
            *value /= divisor;
        }
        let pivot_row = rows[column];
        for (row, values) in rows.iter_mut().enumerate() {
            if row == column {
                continue;
            }
            let factor = values[column];
            for (value, pivot) in values.iter_mut().zip(pivot_row) {
                *value -= factor * pivot;
            }
        }
    }
    let mut result = [0.; 16];
    for row in 0..4 {
        result[row * 4..row * 4 + 4].copy_from_slice(&rows[row][4..]);
    }
    result.iter().all(|v| v.is_finite()).then_some(Mat4(result))
}
