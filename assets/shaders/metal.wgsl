//#define ENABLE_DIFFUSE 1
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

//-------------------------------------------------------
// BRDF functions
fn fresnel_conductor(cos_theta: f32, eta: vec3<f32>, k: vec3<f32>) -> vec3<f32> {
    let cos2 = cos_theta * cos_theta;
    let sin2 = max(1.0 - cos2, 0.0);
    let eta2 = eta * eta;
    let k2 = k * k;

    let t0 = eta2 - k2 - vec3<f32>(sin2, sin2, sin2);
    let a2plusb2 = sqrt(t0 * t0 + 4.0 * eta2 * k2);
    let t1 = a2plusb2 + vec3<f32>(cos2, cos2, cos2);
    let a = sqrt(0.5 * abs(a2plusb2 + t0));
    let t2 = 2.0 * cos_theta * a;
    let Rs = (t1 - t2) / (t1 + t2);

    let sin4 = sin2 * sin2;
    let t3 = cos2 * a2plusb2 + vec3<f32>(sin4, sin4, sin4);
    let t4 = t2 * sin2;
    let Rp = Rs * (t3 - t4) / (t3 + t4);

    return 0.5 * (Rp + Rs);
}

fn shade(input: ShadeInput) -> vec3<f32> {
    let m_eta = sample_eta(input.uv);
    let m_k = sample_k(input.uv);
    let h = normalize(input.wi + input.wo);
    let cos_theta_i = max(dot(input.wi, h), 0.0);
    return input.specular * input.fresnel.x * fresnel_conductor(cos_theta_i, m_eta, m_k);
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"