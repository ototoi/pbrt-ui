#ifndef LIGHTING_SHADOW_FUNCTIONS_WGSL
#define LIGHTING_SHADOW_FUNCTIONS_WGSL

fn sample_directional_shadow_factor(shadow_index: i32, world_position: vec3<f32>) -> f32 {
    if (shadow_index < 0) {
        return 1.0;
    }

    let shadow = directional_shadow_infos[u32(shadow_index)];
    let clip = shadow.light_view_proj * vec4<f32>(world_position, 1.0);
    if (clip.w <= 0.0) {
        return 1.0;
    }

    let ndc = clip.xyz / clip.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0)
    {
        return 1.0;
    }

    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    let compare = ndc.z - max(shadow.bias, 0.0);
    let layer = i32(shadow_index);
    return textureSampleCompare(
        directional_shadow_maps,
        directional_shadow_sampler,
        uv,
        layer,
        compare,
    );
}

#endif
