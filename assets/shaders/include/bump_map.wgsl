#ifndef BUMP_MAP_WGSL
#define BUMP_MAP_WGSL

// Bump map (normal map) support
// Note: Bindings must be defined in each shader file as binding numbers may vary

#ifdef USE_TEXTURE_BUMPMAP
fn apply_bump_map(normal: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>, uv: vec2<f32>, bumpmap_transform: vec4<f32>) -> vec3<f32> {
    // Apply UV scale and offset: xy = scale, zw = offset
    let modified_uv = bumpmap_transform.xy * uv + bumpmap_transform.zw;
    // Sample the normal map texture
    let normal_sample = textureSample(bumpmap_texture, bumpmap_sampler, modified_uv).xyz;
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
fn apply_bump_map(normal: vec3<f32>, tangent: vec3<f32>, bitangent: vec3<f32>, uv: vec2<f32>, bumpmap_transform: vec4<f32>) -> vec3<f32> {
    return normal;
}
#endif

#endif // BUMP_MAP_WGSL
