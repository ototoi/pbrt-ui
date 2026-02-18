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

@group(0) @binding(0)
var<uniform> global_uniforms: GlobalUniforms;

@group(1) @binding(0)
var<uniform> local_uniforms: LocalUniforms;

struct VertexOut {
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) _uvw: vec3<f32>,
    @location(2) _normal: vec3<f32>,
    @location(3) _tangent: vec3<f32>,
) -> VertexOut {
    var out: VertexOut;
    let m_clip = global_uniforms.camera_to_clip * global_uniforms.world_to_camera * local_uniforms.local_to_world;
    out.position = m_clip * vec4<f32>(position, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0);
}
