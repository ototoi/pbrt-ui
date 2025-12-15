#define ENABLE_DIFFUSE 1
#define ENABLE_SPECULAR 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    eta: vec4<f32>,
    k: vec4<f32>,
    roughness: f32,
    _pad1: i32,
    _pad2: i32,
    _pad3: i32,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: MaterialUniforms;

#ifdef USE_TEXTURE_ETA
@group(2)
@binding(1)
var eta_texture: texture_2d<f32>;

@group(2)
@binding(2)
var eta_sampler: sampler;

fn sample_eta(uv: vec2<f32>) -> vec3<f32> {
    let modified_uv = material_uniforms.eta.xy * uv + material_uniforms.eta.zw;
    let diff = textureSample(eta_texture, eta_sampler, modified_uv).rgb;
    return diff;
}
#else
fn sample_eta(uv: vec2<f32>) -> vec3<f32> {
    return material_uniforms.eta.xyz;
}
#endif

#ifdef USE_TEXTURE_K
@group(2)
@binding(1)
var k_texture: texture_2d<f32>;

@group(2)
@binding(2)
var k_sampler: sampler;

fn sample_k(uv: vec2<f32>) -> vec3<f32> {
    let modified_uv = material_uniforms.k.xy * uv + material_uniforms.k.zw;
    let spec = textureSample(k_texture, k_sampler, modified_uv).rgb;
    return spec;
}
#else
fn sample_k(uv: vec2<f32>) -> vec3<f32> {
    return material_uniforms.k.xyz;
}
#endif

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return max(material_uniforms.roughness, 0.08);// cannot < 0.08
}

fn shade(input: ShadeInput) -> vec3<f32> {
    let m_eta = sample_eta(input.uv);
    let m_k = sample_k(input.uv);
    return m_eta * input.diffuse + input.specular * (m_k * input.fresnel.x + (vec3<f32>(1.0) - m_k) * input.fresnel.y);
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"