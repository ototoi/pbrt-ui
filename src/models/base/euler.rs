use super::quaternion::Quaternion;

type F = f32;

impl Quaternion {
    pub fn to_euler_angles(&self) -> (f32, f32, f32) {
        let (qx, qy, qz, qw) = (self.x as F, self.y as F, self.z as F, self.w as F);
        let x = F::atan2(2.0 * (qw * qx + qy * qz), 1.0 - 2.0 * (qx * qx + qy * qy));
        let y = F::asin(2.0 * (qw * qy - qx * qz));
        let z = F::atan2(2.0 * (qz * qw + qx * qy), 1.0 - 2.0 * (qy * qy + qz * qz));
        return (x as f32, y as f32, z as f32);
    }

    pub fn from_euler_angles(x: f32, y: f32, z: f32) -> Self {
        let x = x as F;
        let y = y as F;
        let z = z as F;
        let rx = x * 0.5; //half angle
        let ry = y * 0.5; //half angle
        let rz = z * 0.5; //half angle
        let (sx, cx) = F::sin_cos(rx);
        let (sy, cy) = F::sin_cos(ry);
        let (sz, cz) = F::sin_cos(rz);
        let qw = sx * sy * sz + cx * cy * cz;
        let qx = sx * cy * cz - cx * sy * sz;
        let qy = sx * cy * sz + cx * sy * cz;
        let qz = cx * cy * sz - sx * sy * cz;
        let q = Quaternion::new(qx as f32, qy as f32, qz as f32, qw as f32);
        return q.normalize();
    }
}
