#[inline]
pub fn cos_2_theta(w: &glam::Vec3) -> f32 {
    return w.z * w.z;
}

#[inline]
pub fn abs_cos_theta(w: &glam::Vec3) -> f32 {
    return f32::abs(w.z);
}

#[inline]
pub fn sin_2_theta(w: &glam::Vec3) -> f32 {
    return f32::max(0.0, 1.0 - cos_2_theta(w));
}

#[inline]
pub fn sin_theta(w: &glam::Vec3) -> f32 {
    return f32::sqrt(sin_2_theta(w));
}

#[inline]
pub fn cos_phi(w: &glam::Vec3) -> f32 {
    let sin = sin_theta(w);
    return if sin <= 1e-6 {
        1.0
    } else {
        f32::clamp(w.x / sin, -1.0, 1.0)
    };
}

#[inline]
pub fn sin_phi(w: &glam::Vec3) -> f32 {
    let sin = sin_theta(w);
    return if sin <= 1e-6 {
        0.0
    } else {
        f32::clamp(w.y / sin, -1.0, 1.0)
    };
}
