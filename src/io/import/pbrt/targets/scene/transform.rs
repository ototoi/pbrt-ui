use crate::model::base::Matrix4x4;

#[derive(Debug, Clone, Copy)]
pub struct Transform {
    pub m: Matrix4x4,
    pub im: Matrix4x4,
}

impl Transform {
    pub fn new() -> Self {
        return Transform {
            m: Matrix4x4::identity(),
            im: Matrix4x4::identity(),
        };
    }

    pub fn identity() -> Self {
        Transform {
            m: Matrix4x4::identity(),
            im: Matrix4x4::identity(),
        }
    }
    //------------------------------------------------------------
    pub fn mul_transform(&mut self, other: &Transform) {
        self.m = self.m * other.m;
        self.im = other.im * self.im;
    }

    pub fn set_transform(&mut self, other: &Transform) {
        self.m = other.m;
        self.im = other.im;
    }

    pub fn inverse(&self) -> Self {
        Transform {
            m: self.im,
            im: self.m,
        }
    }
    //------------------------------------------------------------
    pub fn translate(x: f32, y: f32, z: f32) -> Self {
        let m = Matrix4x4::translate(x, y, z);
        let im = Matrix4x4::translate(-x, -y, -z);
        Transform { m, im }
    }

    pub fn rotate(theta: f32, x: f32, y: f32, z: f32) -> Self {
        let m = Matrix4x4::rotate(theta, x, y, z);
        let im = m.transpose();
        Transform { m, im }
    }

    pub fn scale(x: f32, y: f32, z: f32) -> Self {
        let m = Matrix4x4::scale(x, y, z);
        let im = Matrix4x4::scale(1.0 / x, 1.0 / y, 1.0 / z);
        Transform { m, im }
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
        let im = Matrix4x4::camera_to_world(ex, ey, ez, lx, ly, lz, ux, uy, uz);
        let m = im.inverse().unwrap();
        Transform { m, im }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TransformBit {
    Start = 1,
    End = 2,
    All = 3,
}

#[derive(Debug, Clone)]
pub struct TransformSet {
    pub transforms: [Transform; 2],
    pub state: TransformBit,
}

impl TransformSet {
    pub fn new() -> Self {
        TransformSet {
            transforms: [Transform::new(), Transform::new()],
            state: TransformBit::All,
        }
    }

    //------------------------------------------------------------
    pub fn mul_transform(&mut self, t: &Transform) {
        for i in 0..2 {
            if self.state as u8 & (1 << i) != 0 {
                self.transforms[i].mul_transform(&t);
            }
        }
    }

    pub fn set_transform(&mut self, t: &Transform) {
        for i in 0..2 {
            if self.state as u8 & (1 << i) != 0 {
                self.transforms[i].set_transform(t);
            }
        }
    }

    pub fn set_transform_bit(&mut self, bit: TransformBit) {
        self.state = bit;
    }

    pub fn is_animated(&self) -> bool {
        if self.state != TransformBit::All {
            return true;
        }
        for i in 0..1 {
            if self.transforms[i].m != self.transforms[i + 1].m {
                return true;
            }
        }
        return false;
    }

    //------------------------------------------------------------
    pub fn get_world_matrix(&self) -> Matrix4x4 {
        return self.transforms[0].m;
    }

    pub fn get_world_inverse_matrix(&self) -> Matrix4x4 {
        return self.transforms[0].im;
    }
}
