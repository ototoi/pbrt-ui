#define ENABLE_DIFFUSE 1
//define ENABLE_SPECULAR 0

#include "lighting_surface_header.wgsl"
#include "bump_map.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kd: vec4<f32>,
    bumpmap: vec4<f32>,
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
#endif

fn shade(input: ShadeInput) -> vec3<f32> {
    let m_kd = sample_kd(input.uv);
    return m_kd * input.diffuse;
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"

