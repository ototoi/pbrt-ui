#ifndef BUMP_MAP_WGSL
#define BUMP_MAP_WGSL

// Bump map (normal map) support

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

#endif // BUMP_MAP_WGSL
