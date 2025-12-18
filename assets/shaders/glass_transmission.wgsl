//#define ENABLE_DIFFUSE 1
#define ENABLE_SPECULAR 1
#define ENABLE_TRANSMISSION 1

#include "lighting_surface_header.wgsl"
//-------------------------------------------------------
// material definitions

// material uniforms
struct MaterialUniforms {
    kt: vec4<f32>, // transmission color
    roughness: f32,
    _pad1: i32,
    _pad2: i32,
    _pad3: i32,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: MaterialUniforms;

fn sample_roughness(uv: vec2<f32>) -> f32 {
    return max(material_uniforms.roughness, 0.08);// cannot < 0.08
}

fn shade(input: ShadeInput) -> vec3<f32> {
    return vec3<f32>(0.0);
}

fn transmitte(input: TransmitteInput) -> vec4<f32> {
    let fresnel = input.fresnel;
    let m_t = max(1.0 - fresnel, 0.0);
    return vec4<f32>(m_t * material_uniforms.kt.rgb, max(input.fresnel, 0.5));
}

//-------------------------------------------------------

#include "lighting_surface_footer.wgsl"