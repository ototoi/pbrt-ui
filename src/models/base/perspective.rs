use super::matrix4x4::Matrix4x4;

impl Matrix4x4 {
    pub fn to_clip(l: f32, r: f32, b: f32, t: f32, n: f32, f: f32) -> Matrix4x4 {
        let m = Matrix4x4::new(
            (2.0 * n) / (r - l),
            0.0,
            -(r + l) / (r - l),
            0.0,
            0.0,
            (2.0 * n) / (t - b),
            -(t + b) / (t - b),
            0.0,
            0.0,
            0.0,
            (f + n) / (f - n),
            -(2.0 * f * n) / (f - n),
            0.0,
            0.0,
            1.0,
            0.0,
        );
        return m;
    }

    pub fn perspective(angle: f32, aspect: f32, n: f32, f: f32) -> Matrix4x4 {
        //(-1..+1), (-1..+1), (-1..+1)
        let h = f32::tan(angle / 2.0);
        let w = aspect * h;
        let l = -w * n;
        let r = w * n;
        let b = -h * n;
        let t = h * n;
        return Self::to_clip(l, r, b, t, n, f);
    }
}
