#define ENABLE_DIFFUSE 1
#define ENABLE_SPECULAR 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kd: vec4<f32>,
    sigma: f32,
    _pad1: i32,
    _pad2: i32,
    _pad3: i32,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: MaterialUniforms;

#ifdef USE_TEXTURE_KD
@group(2)
@binding(1)
var kd_texture: texture_2d<f32>;

@group(2)
@binding(2)
var kd_sampler: sampler;

fn sample_kd(uv: vec2<f32>) -> vec3<f32> {
    let modified_uv = material_uniforms.kd.xy * uv + material_uniforms.kd.zw;
    let diff = textureSample(kd_texture, kd_sampler, modified_uv).rgb;
    return diff;
}
#else
fn sample_kd(uv: vec2<f32>) -> vec3<f32> {
    return material_uniforms.kd.xyz;
}
#endif

fn lambertian_reflection(r: vec3<f32>) -> vec3<f32> {;
    return r * INV_PI;
}

fn matte(wo: vec3<f32>, wi: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let diffuse = max(dot(vec3<f32>(0.0, 0.0, 1.0), wi), 0.0);
    let m_kd = sample_kd(uv);
    let c1 = lambertian_reflection(m_kd);
    return diffuse * c1;
}

fn shade(intensity: vec3<f32>, wo: vec3<f32>, wi: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    return matte(wo, wi, uv) * intensity;
}

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return material_uniforms.sigma;//normalized sigma
}

fn shade_ltc(diffuse: vec3<f32>, specular: vec3<f32>, uv: vec2<f32>, fresnel: vec2<f32>) -> vec3<f32> {
    let m_kd = sample_kd(uv);
    return m_kd * (diffuse + specular) / 2.0;// combined
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"