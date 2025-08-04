use crate::model::base::Matrix4x4;

impl From<Matrix4x4> for glam::Mat4 {
    fn from(m: Matrix4x4) -> Self {
        let tm = m.transpose();
        glam::Mat4::from_cols_array(&tm.m)
    }
}

impl From<&Matrix4x4> for glam::Mat4 {
    fn from(m: &Matrix4x4) -> Self {
        let tm = m.transpose();
        glam::Mat4::from_cols_array(&tm.m)
    }
}
