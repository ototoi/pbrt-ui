#define ENABLE_DIFFUSE 1
#define ENABLE_SPECULAR 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kd: vec4<f32>,
    ks: vec4<f32>,
    roughness: f32,
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

#ifdef USE_TEXTURE_KS
@group(2)
@binding(1)
var ks_texture: texture_2d<f32>;

@group(2)
@binding(2)
var ks_sampler: sampler;

fn sample_ks(uv: vec2<f32>) -> vec3<f32> {
    let modified_uv = material_uniforms.ks.xy * uv + material_uniforms.ks.zw;
    let spec = textureSample(ks_texture, ks_sampler, modified_uv).rgb;
    return spec;
}
#else
fn sample_ks(uv: vec2<f32>) -> vec3<f32> {
    return material_uniforms.ks.xyz;
}
#endif

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return max(material_uniforms.roughness, 0.08);// cannot < 0.08
}

fn shade(input: ShadeInput) -> vec3<f32> {
    let m_kd = sample_kd(input.uv);
    let m_ks = sample_ks(input.uv);
    return m_kd * input.diffuse + input.specular * (m_ks * input.fresnel.x + (vec3<f32>(1.0) - m_ks) * input.fresnel.y);
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"