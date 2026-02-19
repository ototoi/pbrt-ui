#ifndef LIGHTING_SHADOW_FUNCTIONS_WGSL
#define LIGHTING_SHADOW_FUNCTIONS_WGSL

fn project_directional_shadow_uv_depth(
    shadow_index: i32,
    world_position: vec3<f32>,
) -> vec4<f32> {
    if (shadow_index < 0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let shadow = directional_shadow_infos[u32(shadow_index)];
    let clip = shadow.light_view_proj * vec4<f32>(world_position, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let ndc = clip.xyz / clip.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0)
    {
        return vec4<f32>(0.0, 0.0, 0.0, 0.0);
    }

    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    return vec4<f32>(uv, ndc.z, 1.0);
}

fn sample_directional_shadow_depth(shadow_index: i32, uv: vec2<f32>) -> f32 {
    if (shadow_index < 0) {
        return 1.0;
    }

    let dims = textureDimensions(directional_shadow_maps);
    let w = max(i32(dims.x), 1);
    let h = max(i32(dims.y), 1);
    let x = clamp(i32(uv.x * f32(w)), 0, w - 1);
    let y = clamp(i32(uv.y * f32(h)), 0, h - 1);
    return textureLoad(directional_shadow_maps, vec2<i32>(x, y), shadow_index, 0);
}

fn sample_directional_shadow_factor(shadow_index: i32, world_position: vec3<f32>) -> f32 {
    let projected = project_directional_shadow_uv_depth(shadow_index, world_position);
    if (projected.w <= 0.0) {
        return 1.0;
    }

    let shadow = directional_shadow_infos[u32(shadow_index)];
    let uv = projected.xy;
    let compare = projected.z - max(shadow.bias, 0.0);
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
