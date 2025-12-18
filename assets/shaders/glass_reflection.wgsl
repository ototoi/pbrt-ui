//#define ENABLE_DIFFUSE 1
#define ENABLE_SPECULAR 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kr: vec4<f32>,
    roughness: f32,
    eta: f32,
    _pad2: i32,
    _pad3: i32,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: MaterialUniforms;

#ifdef USE_TEXTURE_KR
@group(2)
@binding(1)
var kr_texture: texture_2d<f32>;

@group(2)
@binding(2)
var kr_sampler: sampler;

fn sample_kr(uv: vec2<f32>) -> vec3<f32> {
    let modified_uv = material_uniforms.kr.xy * uv + material_uniforms.kr.zw;
    let spec = textureSample(kr_texture, kr_sampler, modified_uv).rgb;
    return spec;
}
#else
fn sample_kr(uv: vec2<f32>) -> vec3<f32> {
    return material_uniforms.kr.xyz;
}
#endif

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
    let m_kr = sample_kr(input.uv);
    let eta = max(material_uniforms.eta, 1.0);
    let wh = normalize(input.wo + input.wi);
    let cos_theta_i = dot(input.wo, wh);
    let fresnel = clamp(fresnel_dielectric(cos_theta_i, 1.0, eta), 0.0, 1.0);
    return input.specular * (m_kr * input.magnitude + (vec3<f32>(1.0) - m_kr) * input.fresnel);
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"