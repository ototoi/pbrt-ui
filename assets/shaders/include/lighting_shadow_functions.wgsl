#ifndef LIGHTING_SHADOW_FUNCTIONS_WGSL
#define LIGHTING_SHADOW_FUNCTIONS_WGSL

const DIRECTIONAL_SHADOW_CASCADE_COUNT: u32 = 4u;
const DIRECTIONAL_SHADOW_PCF_RADIUS: i32 = 1;

fn get_directional_shadow_cascade_index(
    base_shadow_index: i32,
    cascade_count: u32,
    view_depth: f32,
) -> i32 {
    if (base_shadow_index < 0 || cascade_count == 0u) {
        return -1;
    }
    let max_count = min(cascade_count, DIRECTIONAL_SHADOW_CASCADE_COUNT);
    for (var i: u32 = 0u; i < max_count; i++) {
        let idx = base_shadow_index + i32(i);
        if (view_depth <= directional_shadow_infos[u32(idx)].split_end) {
            return idx;
        }
    }
    return base_shadow_index + i32(max_count - 1u);
}

fn project_directional_shadow_uv_depth(
    base_shadow_index: i32,
    cascade_count: u32,
    view_depth: f32,
    world_position: vec3<f32>,
) -> vec4<f32> {
    let shadow_index =
        get_directional_shadow_cascade_index(base_shadow_index, cascade_count, view_depth);
    if (shadow_index < 0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }

    let shadow = directional_shadow_infos[u32(shadow_index)];
    let clip = shadow.light_view_proj * vec4<f32>(world_position, 1.0);
    if (clip.w <= 0.0) {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }

    let ndc = clip.xyz / clip.w;
    if (ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0)
    {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }

    let uv = vec2<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5);
    // w stores selected shadow-info index for debug/load path.
    return vec4<f32>(uv, ndc.z, f32(shadow_index));
}

fn sample_directional_shadow_depth(
    base_shadow_index: i32,
    cascade_count: u32,
    view_depth: f32,
    uv: vec2<f32>,
) -> f32 {
    let shadow_index =
        get_directional_shadow_cascade_index(base_shadow_index, cascade_count, view_depth);
    if (shadow_index < 0) {
        return 1.0;
    }

    let shadow = directional_shadow_infos[u32(shadow_index)];
    let dims = textureDimensions(directional_shadow_maps);
    let w = max(i32(dims.x), 1);
    let h = max(i32(dims.y), 1);
    let x = clamp(i32(uv.x * f32(w)), 0, w - 1);
    let y = clamp(i32(uv.y * f32(h)), 0, h - 1);
    return textureLoad(directional_shadow_maps, vec2<i32>(x, y), shadow.map_layer, 0);
}

fn sample_directional_shadow_factor(
    base_shadow_index: i32,
    cascade_count: u32,
    view_depth: f32,
    world_position: vec3<f32>,
    surface_normal: vec3<f32>,
    light_direction: vec3<f32>,
) -> f32 {
    let projected = project_directional_shadow_uv_depth(
        base_shadow_index,
        cascade_count,
        view_depth,
        world_position,
    );
    if (projected.w < 0.0) {
        return 1.0;
    }

    let shadow_index = i32(projected.w);
    let shadow = directional_shadow_infos[u32(shadow_index)];
    let uv = projected.xy;
    let n = normalize(surface_normal);
    let l = normalize(-light_direction);
    let ndotl = max(dot(n, l), 0.0);
    let slope_term = (1.0 - ndotl);
    let final_bias = max(shadow.bias, 0.0) + max(shadow.slope_bias, 0.0) * slope_term;
    let compare = projected.z - final_bias;
    let tex_dims = vec2<f32>(textureDimensions(directional_shadow_maps));
    let texel = vec2<f32>(1.0 / tex_dims.x, 1.0 / tex_dims.y);

    var sum = 0.0;
    var taps = 0.0;
    for (var y: i32 = -DIRECTIONAL_SHADOW_PCF_RADIUS; y <= DIRECTIONAL_SHADOW_PCF_RADIUS; y++) {
        for (
            var x: i32 = -DIRECTIONAL_SHADOW_PCF_RADIUS;
            x <= DIRECTIONAL_SHADOW_PCF_RADIUS;
            x++
        ) {
            let offset = vec2<f32>(f32(x), f32(y)) * texel;
            sum += textureSampleCompare(
                directional_shadow_maps,
                directional_shadow_sampler,
                uv + offset,
                shadow.map_layer,
                compare,
            );
            taps += 1.0;
        }
    }
    return sum / max(taps, 1.0);
}

#endif
