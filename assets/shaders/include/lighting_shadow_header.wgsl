#ifndef LIGHTING_SHADOW_HEADER_WGSL
#define LIGHTING_SHADOW_HEADER_WGSL

struct DirectionalShadowInfo {
    light_view_proj: mat4x4<f32>,
    bias: f32,
    _pad0: vec3<f32>,
}

@group(5)
@binding(0)
var<storage, read> directional_shadow_infos: array<DirectionalShadowInfo>;

@group(5)
@binding(1)
var directional_shadow_maps: texture_depth_2d_array;

@group(5)
@binding(2)
var directional_shadow_sampler: sampler_comparison;

#endif
