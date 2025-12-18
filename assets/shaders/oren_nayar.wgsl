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

#ifdef USE_TEXTURE_BUMPMAP
@group(2)
@binding(3)
var bumpmap_texture: texture_2d<f32>;

@group(2)
@binding(4)
var bumpmap_sampler: sampler;

fn apply_bump_map(normal: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    // Sample the normal map texture
    let normal_sample = textureSample(bumpmap_texture, bumpmap_sampler, uv).xyz;
    // Convert from [0,1] range to [-1,1] range
    let tangent_normal = normal_sample * 2.0 - 1.0;
    // Transform from tangent space to world space
    let perturbed_normal = normalize(
        tangent_normal.x * tangent +
        tangent_normal.y * bitangent +
        tangent_normal.z * normal
    );
    return perturbed_normal;
}
#else
fn apply_bump_map(normal: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    return normal;
}
#endif

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return material_uniforms.sigma;//normalized sigma
}

fn shade(input: ShadeInput) -> vec3<f32> {
    let m_kd = sample_kd(input.uv);
    return m_kd * (input.diffuse + input.specular) / 2.0;// combined
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"