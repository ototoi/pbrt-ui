// Helper functions for spherical coordinates

#[inline]
pub fn cos_theta(w: &glam::Vec3) -> f32 {
    w.z
}

#[inline]
pub fn abs_cos_theta(w: &glam::Vec3) -> f32 {
    w.z.abs()
}

#[inline]
pub fn cos_2_theta(w: &glam::Vec3) -> f32 {
    w.z * w.z
}

#[inline]
pub fn sin_2_theta(w: &glam::Vec3) -> f32 {
    (0.0_f32).max(1.0 - cos_2_theta(w))
}

#[inline]
pub fn sin_theta(w: &glam::Vec3) -> f32 {
    sin_2_theta(w).sqrt()
}

#[inline]
pub fn tan_theta(w: &glam::Vec3) -> f32 {
    sin_2_theta(w).sqrt() / cos_theta(w)
}

#[inline]
pub fn tan_2_theta(w: &glam::Vec3) -> f32 {
    sin_2_theta(w) / cos_2_theta(w)
}

#[inline]
pub fn cos_phi(w: &glam::Vec3) -> f32 {
    let sin_theta = sin_2_theta(w).sqrt();
    if sin_theta == 0.0 {
        1.0
    } else {
        (w.x / sin_theta).clamp(-1.0, 1.0)
    }
}

#[inline]
pub fn sin_phi(w: &glam::Vec3) -> f32 {
    let sin_theta = sin_2_theta(w).sqrt();
    if sin_theta == 0.0 {
        0.0
    } else {
        (w.y / sin_theta).clamp(-1.0, 1.0)
    }
}

#[inline]
pub fn cos_2_phi(w: &glam::Vec3) -> f32 {
    let cp = cos_phi(w);
    cp * cp
}

#[inline]
pub fn sin_2_phi(w: &glam::Vec3) -> f32 {
    let sp = sin_phi(w);
    sp * sp
}

#[inline]
pub fn spherical_direction(sin_theta: f32, cos_theta: f32, phi: f32) -> glam::Vec3 {
    glam::Vec3::new(
        sin_theta * phi.cos(),
        sin_theta * phi.sin(),
        cos_theta,
    )
}

#[inline]
pub fn same_hemisphere(w: &glam::Vec3, wp: &glam::Vec3) -> bool {
    w.z * wp.z > 0.0
}

