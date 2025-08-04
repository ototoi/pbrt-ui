use super::vector3::Vector3;

#[derive(Clone, Copy, PartialEq)]
pub struct Matrix4x4 {
    pub m: [f32; 16],
}

impl Default for Matrix4x4 {
    fn default() -> Self {
        Self::identity()
    }
}

impl From<[f32; 16]> for Matrix4x4 {
    fn from(m: [f32; 16]) -> Self {
        Self { m }
    }
}

impl Matrix4x4 {
    pub fn new(
        e0: f32,
        e1: f32,
        e2: f32,
        e3: f32,
        e4: f32,
        e5: f32,
        e6: f32,
        e7: f32,
        e8: f32,
        e9: f32,
        e10: f32,
        e11: f32,
        e12: f32,
        e13: f32,
        e14: f32,
        e15: f32,
    ) -> Self {
        Matrix4x4 {
            #[rustfmt::skip]
            m: [
                e0, e1, e2, e3,
                e4, e5, e6, e7,
                e8, e9, e10, e11,
                e12, e13, e14, e15,
            ],
        }
    }

    pub fn identity() -> Self {
        Self {
            #[rustfmt::skip]
            m: [
                1.0, 0.0, 0.0, 0.0,
                0.0, 1.0, 0.0, 0.0,
                0.0, 0.0, 1.0, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn translate(x: f32, y: f32, z: f32) -> Self {
        Self {
            #[rustfmt::skip]
            m: [
                1.0, 0.0, 0.0, x,
                0.0, 1.0, 0.0, y,
                0.0, 0.0, 1.0, z,
                0.0, 0.0, 0.0, 1.0
            ],
        }
    }

    pub fn rotate_x(theta: f32) -> Self {
        let s = f32::sin(f32::to_radians(theta));
        let c = f32::cos(f32::to_radians(theta));
        Self {
            #[rustfmt::skip]
            m: [
                1.0, 0.0, 0.0, 0.0,
                0.0,   c,  -s, 0.0,
                0.0,   s,   c, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_y(theta: f32) -> Self {
        let s = f32::sin(f32::to_radians(theta));
        let c = f32::cos(f32::to_radians(theta));
        Matrix4x4 {
            m: [
                c, 0.0, s, 0.0, 0.0, 1.0, 0.0, 0.0, -s, 0.0, c, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate_z(theta: f32) -> Self {
        let s = f32::sin(f32::to_radians(theta));
        let c = f32::cos(f32::to_radians(theta));
        Matrix4x4 {
            m: [
                c, -s, 0.0, 0.0, s, c, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn rotate(theta: f32, x: f32, y: f32, z: f32) -> Self {
        let a = Vector3::new(x, y, z).normalize();
        let sin_theta = f32::sin(f32::to_radians(theta));
        let cos_theta = f32::cos(f32::to_radians(theta));
        let mut m = Matrix4x4::identity();
        // Compute rotation of first basis vector
        m.m[4 * 0 + 0] = a.x * a.x + (1.0 - a.x * a.x) * cos_theta;
        m.m[4 * 0 + 1] = a.x * a.y * (1.0 - cos_theta) - a.z * sin_theta;
        m.m[4 * 0 + 2] = a.x * a.z * (1.0 - cos_theta) + a.y * sin_theta;
        m.m[4 * 0 + 3] = 0.0;

        // Compute rotations of second basis vector
        m.m[4 * 1 + 0] = a.x * a.y * (1.0 - cos_theta) + a.z * sin_theta;
        m.m[4 * 1 + 1] = a.y * a.y + (1.0 - a.y * a.y) * cos_theta;
        m.m[4 * 1 + 2] = a.y * a.z * (1.0 - cos_theta) - a.x * sin_theta;
        m.m[4 * 1 + 3] = 0.0;

        // Compute rotation of third basis vector
        m.m[4 * 2 + 0] = a.x * a.z * (1.0 - cos_theta) - a.y * sin_theta;
        m.m[4 * 2 + 1] = a.y * a.z * (1.0 - cos_theta) + a.x * sin_theta;
        m.m[4 * 2 + 2] = a.z * a.z + (1.0 - a.z * a.z) * cos_theta;
        m.m[4 * 2 + 3] = 0.0;
        return m;
    }

    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        Matrix4x4 {
            #[rustfmt::skip]
            m: [
                  x, 0.0, 0.0, 0.0,
                0.0,   y, 0.0, 0.0,
                0.0, 0.0,   z, 0.0,
                0.0, 0.0, 0.0, 1.0,
            ],
        }
    }

    pub fn look_at(
        ex: f32,
        ey: f32,
        ez: f32,
        lx: f32,
        ly: f32,
        lz: f32,
        ux: f32,
        uy: f32,
        uz: f32,
    ) -> Self {
        Self::camera_to_world(ex, ey, ez, lx, ly, lz, ux, uy, uz)
            .inverse()
            .unwrap()
    }

    pub fn camera_to_world(
        ex: f32,
        ey: f32,
        ez: f32,
        lx: f32,
        ly: f32,
        lz: f32,
        ux: f32,
        uy: f32,
        uz: f32,
    ) -> Self {
        let pos = Vector3::new(ex, ey, ez);
        let look = Vector3::new(lx, ly, lz);
        let up = Vector3::new(ux, uy, uz).normalize();

        let dir = (look - pos).normalize(); //z

        let mut right = Vector3::cross(&up, &dir); //x
        //println!("x:{:?}", right);
        assert!(right.length() != 0.0);
        right = right.normalize();
        let new_up = Vector3::cross(&dir, &right).normalize();
        #[rustfmt::skip]
        let m: [f32; 16] = [
            right.x, new_up.x, dir.x, pos.x, //
            right.y, new_up.y, dir.y, pos.y, //
            right.z, new_up.z, dir.z, pos.z, //
            0.0, 0.0, 0.0, 1.0,
        ];
        Matrix4x4 { m }
    }

    pub fn transpose(&self) -> Self {
        let mut t = [0.0; 16];
        for i in 0..4 {
            for j in 0..4 {
                t[i * 4 + j] = self.m[j * 4 + i];
            }
        }
        Self { m: t }
    }

    pub fn inverse(&self) -> Option<Self> {
        let mut indxc = [0; 4];
        let mut indxr = [0; 4];
        let mut ipiv = [0; 4];
        let mut minv: [f32; 16] = self.m;
        for i in 0..4 {
            let mut irow = 0;
            let mut icol = 0;
            let mut big: f32 = 0.0;
            // Choose pivot
            for j in 0..4 {
                if ipiv[j] != 1 {
                    for k in 0..4 {
                        if ipiv[k] == 0 {
                            if f32::abs(minv[4 * j + k]) >= big {
                                big = f32::abs(minv[4 * j + k]);
                                irow = j;
                                icol = k;
                            }
                        } else if ipiv[k] > 1 {
                            //Error("Singular matrix in MatrixInvert");
                            return None;
                        }
                    }
                }
            }
            ipiv[icol] += 1;
            if irow != icol {
                for k in 0..4 {
                    //let tmp = minv[4 * irow + k];
                    //minv[4 * irow + k] = minv[4 * icol + k];
                    //minv[4 * icol + k] = tmp;
                    minv.swap(4 * irow + k, 4 * icol + k);
                }
            }
            indxr[i] = irow;
            indxc[i] = icol;
            if minv[4 * icol + icol] == 0.0 {
                //Error("Singular matrix in MatrixInvert");
                return None;
            }

            // Set $m[icol][icol]$ to one by scaling row _icol_ appropriately
            let pivinv = 1.0 / minv[4 * icol + icol];
            minv[4 * icol + icol] = 1.0;
            for j in 0..4 {
                minv[4 * icol + j] *= pivinv;
            }

            // Subtract this row from others to zero out their columns
            for j in 0..4 {
                if j != icol {
                    let save = minv[4 * j + icol];
                    minv[4 * j + icol] = 0.0;
                    for k in 0..4 {
                        minv[4 * j + k] -= minv[4 * icol + k] * save;
                    }
                }
            }
        }

        // Swap columns to reflect permutation
        for j in [3, 2, 1, 0] {
            if indxr[j] != indxc[j] {
                for k in 0..4 {
                    let src = 4 * k + indxr[j];
                    let dst = 4 * k + indxc[j];
                    minv.swap(src, dst);
                }
            }
        }

        return Some(Matrix4x4 { m: minv });
    }

    pub fn transform_point(&self, p: &Vector3) -> Vector3 {
        let x = p.x;
        let y = p.y;
        let z = p.z;
        let xp = self.m[0] * x + self.m[1] * y + self.m[2] * z + self.m[3];
        let yp = self.m[4] * x + self.m[5] * y + self.m[6] * z + self.m[7];
        let zp = self.m[8] * x + self.m[9] * y + self.m[10] * z + self.m[11];
        let wp = self.m[12] * x + self.m[13] * y + self.m[14] * z + self.m[15];
        if wp == 1.0 {
            return Vector3::new(xp, yp, zp);
        } else {
            return Vector3::new(xp / wp, yp / wp, zp / wp);
        }
    }

    pub fn transform_vector(&self, p: &Vector3) -> Vector3 {
        let x = p.x;
        let y = p.y;
        let z = p.z;
        let xp = self.m[0] * x + self.m[1] * y + self.m[2] * z;
        let yp = self.m[4] * x + self.m[5] * y + self.m[6] * z;
        let zp = self.m[8] * x + self.m[9] * y + self.m[10] * z;
        return Vector3::new(xp, yp, zp);
    }

    pub fn transform_normal(&self, p: &Vector3) -> Vector3 {
        let x = p.x;
        let y = p.y;
        let z = p.z;
        let xp = self.m[0] * x + self.m[4] * y + self.m[8] * z;
        let yp = self.m[1] * x + self.m[5] * y + self.m[9] * z;
        let zp = self.m[2] * x + self.m[6] * y + self.m[10] * z;
        return Vector3::new(xp, yp, zp);
    }
}

#[inline(always)]
fn mul4x4(a: &[f32], b: &[f32]) -> f32 {
    return a[0] * b[0] + a[1] * b[1] + a[2] * b[2] + a[3] * b[3];
}

impl std::ops::Mul for Matrix4x4 {
    type Output = Self;

    fn mul(self, rhs: Self) -> Self::Output {
        let m = [
            mul4x4(&self.m[0..4], &[rhs.m[0], rhs.m[4], rhs.m[8], rhs.m[12]]),
            mul4x4(&self.m[0..4], &[rhs.m[1], rhs.m[5], rhs.m[9], rhs.m[13]]),
            mul4x4(&self.m[0..4], &[rhs.m[2], rhs.m[6], rhs.m[10], rhs.m[14]]),
            mul4x4(&self.m[0..4], &[rhs.m[3], rhs.m[7], rhs.m[11], rhs.m[15]]),
            mul4x4(&self.m[4..8], &[rhs.m[0], rhs.m[4], rhs.m[8], rhs.m[12]]),
            mul4x4(&self.m[4..8], &[rhs.m[1], rhs.m[5], rhs.m[9], rhs.m[13]]),
            mul4x4(&self.m[4..8], &[rhs.m[2], rhs.m[6], rhs.m[10], rhs.m[14]]),
            mul4x4(&self.m[4..8], &[rhs.m[3], rhs.m[7], rhs.m[11], rhs.m[15]]),
            mul4x4(&self.m[8..12], &[rhs.m[0], rhs.m[4], rhs.m[8], rhs.m[12]]),
            mul4x4(&self.m[8..12], &[rhs.m[1], rhs.m[5], rhs.m[9], rhs.m[13]]),
            mul4x4(&self.m[8..12], &[rhs.m[2], rhs.m[6], rhs.m[10], rhs.m[14]]),
            mul4x4(&self.m[8..12], &[rhs.m[3], rhs.m[7], rhs.m[11], rhs.m[15]]),
            mul4x4(&self.m[12..16], &[rhs.m[0], rhs.m[4], rhs.m[8], rhs.m[12]]),
            mul4x4(&self.m[12..16], &[rhs.m[1], rhs.m[5], rhs.m[9], rhs.m[13]]),
            mul4x4(&self.m[12..16], &[rhs.m[2], rhs.m[6], rhs.m[10], rhs.m[14]]),
            mul4x4(&self.m[12..16], &[rhs.m[3], rhs.m[7], rhs.m[11], rhs.m[15]]),
        ];
        Matrix4x4 { m }
    }
}

impl std::ops::MulAssign for Matrix4x4 {
    fn mul_assign(&mut self, rhs: Self) {
        *self = *self * rhs;
    }
}

impl std::fmt::Debug for Matrix4x4 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        for i in 0..16 {
            s.push_str(&format!("{:.2}", &self.m[i]));
            if i != 15 {
                s.push_str(", ");
            }
        }
        write!(f, "[{}]", s)
    }
}
