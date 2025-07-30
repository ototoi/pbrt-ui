struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    base_color: vec4<f32>,
}

// global uniforms
@group(0)
@binding(0)
var<uniform> global_uniforms: GlobalUniforms;

// local uniforms
@group(1)
@binding(0)
var<uniform> local_uniforms: LocalUniforms;

struct VertexOut {
    @location(0) color: vec4<f32>,
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> VertexOut {
    var out: VertexOut;
    // local_to_world * world_to_camera * camera_to_clip
    //let transform = global_uniforms.camera_to_clip * global_uniforms.world_to_camera * local_uniforms.local_to_world;
    let transform = local_uniforms.local_to_world * global_uniforms.world_to_camera * global_uniforms.camera_to_clip;

    out.position = vec4<f32>(position, 1.0) * transform;
    out.color = local_uniforms.base_color;

    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
