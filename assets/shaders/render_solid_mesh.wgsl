struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_to_world: mat4x4<f32>,
    light_direction: vec4<f32>, // Example light direction
    light_intensity: vec4<f32>, // Example light intensity
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    world_to_local: mat4x4<f32>, // inverse of world to camera
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
    @location(0) camera_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) camera_normal:   vec3<f32>,
    @location(3) camera_tangent:  vec3<f32>,
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
    let m_camera = global_uniforms.world_to_camera * local_uniforms.local_to_world;
    let m_clip = global_uniforms.camera_to_clip * m_camera;
    let m_camera_it = transpose(local_uniforms.world_to_local * global_uniforms.camera_to_world);

    let camera_position = (m_camera * vec4<f32>(position, 1.0)).xyz;
    let camera_normal = normalize((m_camera_it * vec4<f32>(normal, 0.0)).xyz);
    let camera_tangent = normalize((m_camera_it * vec4<f32>(tangent, 0.0)).xyz);
    
    out.position = m_clip * vec4<f32>(position, 1.0);
    out.camera_position = camera_position;
    out.uv = uvw.xy;
    out.camera_normal = camera_normal;
    out.camera_tangent = camera_tangent;
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let camera_to_object = normalize(in.camera_position);
    var normal = normalize(in.camera_normal);
    if dot(normal, camera_to_object) > 0.0 {
        normal = -normal;
    }
    let light_direction = global_uniforms.light_direction.xyz; // Example light direction
    let light_intensity = global_uniforms.light_intensity.x; // Example light intensity
    let diffuse = (max(dot(normal, light_direction), 0.0) * 0.9 + 0.1) * light_intensity;
    let diffuse_color = vec3<f32>(diffuse);
    return vec4<f32>(diffuse_color, 1.0);
}
