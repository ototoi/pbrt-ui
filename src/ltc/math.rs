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

// Trowbridge-Reitz sampling functions

pub fn trowbridge_reitz_sample_11(cos_theta: f32, u1: f32, u2: f32) -> (f32, f32) {
    // Special case (normal incidence)
    if cos_theta > 0.9999 {
        let r = (u1 / (1.0 - u1)).sqrt();
        let phi = 2.0 * std::f32::consts::PI * u2;
        return (r * phi.cos(), r * phi.sin());
    }

    let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
    let tan_theta = sin_theta / cos_theta;
    let a = 1.0 / tan_theta;
    let g1 = 2.0 / (1.0 + (1.0 + 1.0 / (a * a)).sqrt());

    // Sample slope_x
    let a_sample = 2.0 * u1 / g1 - 1.0;
    let tmp = (1.0 / (a_sample * a_sample - 1.0)).min(1e10);
    let b = tan_theta;
    let d = (b * b * tmp * tmp - (a_sample * a_sample - b * b) * tmp).max(0.0).sqrt();
    let slope_x_1 = b * tmp - d;
    let slope_x_2 = b * tmp + d;
    let slope_x = if a_sample < 0.0 || slope_x_2 > 1.0 / tan_theta {
        slope_x_1
    } else {
        slope_x_2
    };

    // Sample slope_y
    let (s, u2) = if u2 > 0.5 {
        (1.0, 2.0 * (u2 - 0.5))
    } else {
        (-1.0, 2.0 * (0.5 - u2))
    };
    let z = (u2 * (u2 * (u2 * 0.27385 - 0.73369) + 0.46341))
        / (u2 * (u2 * (u2 * 0.093073 + 0.309420) - 1.000000) + 0.597999);
    let slope_y = s * z * (1.0 + slope_x * slope_x).sqrt();

    (slope_x, slope_y)
}

pub fn trowbridge_reitz_sample(
    wi: &glam::Vec3,
    alpha_x: f32,
    alpha_y: f32,
    u1: f32,
    u2: f32,
) -> glam::Vec3 {
    // 1. stretch wi
    let wi_stretched = glam::Vec3::new(alpha_x * wi.x, alpha_y * wi.y, wi.z).normalize();

    // 2. simulate P22_{wi}(x_slope, y_slope, 1, 1)
    let (mut slope_x, mut slope_y) = trowbridge_reitz_sample_11(cos_theta(&wi_stretched), u1, u2);

    // 3. rotate
    let tmp = cos_phi(&wi_stretched) * slope_x - sin_phi(&wi_stretched) * slope_y;
    slope_y = sin_phi(&wi_stretched) * slope_x + cos_phi(&wi_stretched) * slope_y;
    slope_x = tmp;

    // 4. unstretch
    slope_x *= alpha_x;
    slope_y *= alpha_y;

    // 5. compute normal
    glam::Vec3::new(-slope_x, -slope_y, 1.0).normalize()
}

pub fn sample_wh_helper(alphax: f32, alphay: f32, u1: f32, u2: f32) -> (f32, f32, f32) {
    let u1 = u1.clamp(1e-6, 1.0 - 1e-6);
    let u2 = u2.clamp(1e-6, 1.0 - 1e-6);

    if alphax == alphay {
        let tan_theta_2 = alphax * alphax * u1 / (1.0 - u1);
        let phi = 2.0 * std::f32::consts::PI * u2;
        let cos_theta = 1.0 / (1.0 + tan_theta_2).sqrt();
        let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
        (sin_theta, cos_theta, phi)
    } else {
        let mut phi = (alphay / alphax * (2.0 * std::f32::consts::PI * u2 + 0.5 * std::f32::consts::PI).tan()).atan();
        if u2 > 0.5 {
            phi += std::f32::consts::PI;
        }
        let sin_phi = phi.sin();
        let cos_phi = phi.cos();
        let alphax2 = alphax * alphax;
        let alphay2 = alphay * alphay;
        let alpha2 = 1.0 / (cos_phi * cos_phi / alphax2 + sin_phi * sin_phi / alphay2);
        let tan_theta_2 = alpha2 * u1 / (1.0 - u1);
        let cos_theta = 1.0 / (1.0 + tan_theta_2).sqrt();
        let sin_theta = (0.0_f32).max(1.0 - cos_theta * cos_theta).sqrt();
        (sin_theta, cos_theta, phi)
    }
}
