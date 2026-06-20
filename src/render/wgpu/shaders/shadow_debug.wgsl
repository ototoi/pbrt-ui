struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_to_world: mat4x4<f32>,
    camera_position: vec4<f32>,
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    world_to_local: mat4x4<f32>,
}

struct DebugUniforms {
    light_direction: vec4<f32>,
    shadow_index: i32,
    cascade_count: u32,
    debug_mode: u32,
    _pad0: u32,
}

struct DirectionalShadowInfo {
    light_view_proj: mat4x4<f32>,
    split_origin: vec4<f32>,
    split_forward: vec4<f32>,
    split_end: f32,
    bias: f32,
    slope_bias: f32,
    map_layer: i32,
}

@group(0) @binding(0)
var<uniform> global_uniforms: GlobalUniforms;
@group(1) @binding(0)
var<uniform> local_uniforms: LocalUniforms;
@group(2) @binding(0)
var<uniform> debug_uniforms: DebugUniforms;
@group(3) @binding(0)
var<storage, read> directional_shadow_infos: array<DirectionalShadowInfo>;
@group(3) @binding(1)
var directional_shadow_maps: texture_depth_2d_array;
@group(3) @binding(2)
var directional_shadow_sampler: sampler_comparison;

struct VertexOut {
    @location(0) w_position: vec3<f32>,
    @location(1) w_normal: vec3<f32>,
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) _uvw: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) _tangent: vec3<f32>,
) -> VertexOut {
    var out: VertexOut;
    let world = local_uniforms.local_to_world * vec4<f32>(position, 1.0);
    out.position =
        global_uniforms.camera_to_clip * global_uniforms.world_to_camera * world;
    out.w_position = world.xyz;
    out.w_normal = normalize((transpose(local_uniforms.world_to_local) * vec4<f32>(normal, 0.0)).xyz);
    return out;
}

fn get_shadow_index(split_depth: f32) -> i32 {
    if debug_uniforms.shadow_index < 0 || debug_uniforms.cascade_count == 0u {
        return -1;
    }
    return debug_uniforms.shadow_index;
}

fn project_shadow(world_position: vec3<f32>) -> vec4<f32> {
    let shadow = directional_shadow_infos[u32(debug_uniforms.shadow_index)];
    let clip = shadow.light_view_proj * vec4<f32>(world_position, 1.0);
    if clip.w <= 0.0 {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    let ndc = clip.xyz / clip.w;
    if ndc.x < -1.0 || ndc.x > 1.0 || ndc.y < -1.0 || ndc.y > 1.0 || ndc.z < 0.0 || ndc.z > 1.0 {
        return vec4<f32>(0.0, 0.0, 0.0, -1.0);
    }
    return vec4<f32>(ndc.x * 0.5 + 0.5, 0.5 - ndc.y * 0.5, ndc.z, f32(shadow.map_layer));
}

fn sample_shadow_factor(world_position: vec3<f32>, normal: vec3<f32>, light_direction: vec3<f32>) -> f32 {
    let projected = project_shadow(world_position);
    if projected.w < 0.0 {
        return 1.0;
    }
    let shadow = directional_shadow_infos[u32(debug_uniforms.shadow_index)];
    let l = normalize(-light_direction);
    let ndotl = max(dot(normalize(normal), l), 0.0);
    let compare = projected.z - (max(shadow.bias, 0.0) + max(shadow.slope_bias, 0.0) * (1.0 - ndotl));
    return textureSampleCompare(
        directional_shadow_maps,
        directional_shadow_sampler,
        projected.xy,
        shadow.map_layer,
        compare,
    );
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    if debug_uniforms.shadow_index < 0 {
        return vec4<f32>(1.0, 0.0, 1.0, 1.0);
    }
    let shadow = directional_shadow_infos[u32(debug_uniforms.shadow_index)];
    let light_direction = normalize(debug_uniforms.light_direction.xyz);
    let split_depth = dot(in.w_position - shadow.split_origin.xyz, shadow.split_forward.xyz);
    let projected = project_shadow(in.w_position);
    var shadow_depth = 1.0;
    if (projected.w >= 0.0) {
        let dims = textureDimensions(directional_shadow_maps);
        let x = clamp(i32(projected.x * f32(max(i32(dims.x), 1))), 0, max(i32(dims.x), 1) - 1);
        let y = clamp(i32(projected.y * f32(max(i32(dims.y), 1))), 0, max(i32(dims.y), 1) - 1);
        shadow_depth = textureLoad(directional_shadow_maps, vec2<i32>(x, y), shadow.map_layer, 0);
    }
    let shadow_factor = sample_shadow_factor(in.w_position, in.w_normal, light_direction);
    switch debug_uniforms.debug_mode {
        case 1u: {
            return vec4<f32>(vec3<f32>(shadow_factor), 1.0);
        }
        case 2u: {
            return vec4<f32>(vec3<f32>(shadow_depth), 1.0);
        }
        case 3u: {
            if projected.w < 0.0 {
                return vec4<f32>(1.0, 0.0, 1.0, 1.0);
            }
            return vec4<f32>(projected.xyz, 1.0);
        }
        case 4u: {
            return vec4<f32>(1.0, 0.2, 0.2, 1.0);
        }
        case 5u: {
            let t = clamp(log2(max(split_depth, 1e-4) + 1.0) / log2(max(shadow.split_end, 1e-4) + 1.0), 0.0, 1.0);
            return vec4<f32>(t, 1.0 - abs(t * 2.0 - 1.0), 1.0 - t, 1.0);
        }
        case 6u: {
            return vec4<f32>(0.2, 1.0, 1.0, 1.0);
        }
        default: {
            return vec4<f32>(vec3<f32>(shadow_factor), 1.0);
        }
    }
}
