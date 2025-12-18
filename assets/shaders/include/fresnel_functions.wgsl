#ifndef FRESNEL_FUNCTIONS_WGSL
#define FRESNEL_FUNCTIONS_WGSL

fn fresnel_dielectric(cos_theta_i_: f32, eta_i_: f32, eta_t_: f32) -> f32 {
    var cos_theta_i = clamp(cos_theta_i_, -1.0, 1.0);
    var eta_i = eta_i_;
    var eta_t = eta_t_;
    let entering = cos_theta_i > 0.0;
    if (!entering) {
        let temp = eta_i;
        eta_i = eta_t;
        eta_t = temp;
        cos_theta_i = abs(cos_theta_i);
    }
    let sin_theta_i = sqrt(max(0.0, 1.0 - cos_theta_i * cos_theta_i));
    let sin_theta_t = eta_i / eta_t * sin_theta_i;
    if (sin_theta_t >= 1.0) {
        return 1.0; // total internal reflection
    }
    let cos_theta_t = sqrt(max(0.0, 1.0 - sin_theta_t * sin_theta_t));
    let r_parl = ((eta_t * cos_theta_i) - (eta_i * cos_theta_t)) /
                 ((eta_t * cos_theta_i) + (eta_i * cos_theta_t));
    let r_perp = ((eta_i * cos_theta_i) - (eta_t * cos_theta_t)) /
                 ((eta_i * cos_theta_i) + (eta_t * cos_theta_t));
    return (r_parl * r_parl + r_perp * r_perp) * 0.5;
}

#endif
