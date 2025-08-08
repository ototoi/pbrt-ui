struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_position: vec4<f32>,
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    local_to_world_inverse: mat4x4<f32>, // inverse of world to camera
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
    @location(0) world_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) world_normal:   vec3<f32>,
    @location(3) world_tangent:  vec3<f32>,
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uvw: vec3<f32>,
    @location(2) normal: vec3<f32>,
    @location(3) tangent: vec3<f32>,
) -> VertexOut {
    var out: VertexOut;
    // local_to_world * world_to_camera * camera_to_clip
    let m_world = local_uniforms.local_to_world;
    let m_clip = global_uniforms.camera_to_clip * global_uniforms.world_to_camera * m_world;
    let m_world_it = transpose(local_uniforms.local_to_world_inverse);
    let world_position = (m_world * vec4<f32>(position, 1.0)).xyz;
    let world_normal = normalize((m_world_it * vec4<f32>(normal, 0.0)).xyz);
    let world_tangent = normalize((m_world_it * vec4<f32>(tangent, 0.0)).xyz);
    
    out.position = m_clip * vec4<f32>(position, 1.0);
    out.world_position = world_position;
    out.uv = uvw.xy;
    out.world_normal = world_normal;
    out.world_tangent = world_tangent;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let camera_position = global_uniforms.camera_position.xyz;//world position
    let camera_to_object = normalize(in.world_position - camera_position);
    var normal = normalize(in.world_normal);
    var tangent = normalize(in.world_tangent);
    if dot(normal, camera_to_object) > 0.0 {
        normal = -normal;
    }
    //let color = 0.5 * (normal + vec3<f32>(1.0, 1.0, 1.0));
    let color = 0.5 * (tangent + vec3<f32>(1.0, 1.0, 1.0));
    //return vec4<f32>(in.uv, 0.0, 1.0);
    return vec4<f32>(color, 1.0);
}
