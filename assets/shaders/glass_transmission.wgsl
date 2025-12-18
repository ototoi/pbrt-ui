//#define ENABLE_DIFFUSE 1
//#define ENABLE_SPECULAR 1
#define ENABLE_TRANSMISSION 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kt: vec4<f32>, // transmission color
    roughness: f32,
    eta: f32,
    _pad2: i32,
    _pad3: i32,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: MaterialUniforms;

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

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return max(material_uniforms.roughness, 0.08);// cannot < 0.08
}

fn shade(input: ShadeInput) -> vec3<f32> {
    return vec3<f32>(0.0);
}

fn transmitte(input: TransmitteInput) -> vec4<f32> {
    //let fresnel = input.fresnel;
    let eta = max(material_uniforms.eta, 1.0);
    let cos_theta_i = dot(normalize(input.wo), vec3<f32>(0.0, 0.0, 1.0));
    let fresnel = clamp(fresnel_dielectric(cos_theta_i, 1.0, eta), 0.0, 1.0);
    //return vec4<f32>(fresnel, fresnel, fresnel, 1.0);
    let m_t = max(1.0 - fresnel, 0.0);
    return vec4<f32>(m_t * material_uniforms.kt.rgb, fresnel * cos_theta_i);
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"