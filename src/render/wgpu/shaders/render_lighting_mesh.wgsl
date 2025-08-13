struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_to_world: mat4x4<f32>,
    camera_position: vec4<f32>, // Camera position in world space
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    world_to_local: mat4x4<f32>, // inverse of world to camera
    base_color: vec4<f32>,
}

struct DirectionalLight {
    direction: vec4<f32>, // Example light direction
    intensity: vec4<f32>, // Example light intensity
}

const MAX_DIRECTIONAL_LIGHTS: u32 = 4; // Maximum number of directional lights
struct DirectionalLightUniforms {
    lights: array<DirectionalLight, MAX_DIRECTIONAL_LIGHTS>,
    count: u32,
}

// global uniforms
@group(0)
@binding(0)
var<uniform> global_uniforms: GlobalUniforms;

// local uniforms
@group(1)
@binding(0)
var<uniform> local_uniforms: LocalUniforms;

// light uniforms
@group(2)
@binding(0)
var<uniform> directional_lights: DirectionalLightUniforms;


struct VertexOut {
    @location(0) w_position: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) w_normal:   vec3<f32>,
    @location(3) w_tangent:  vec3<f32>,
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
    let m_world = local_uniforms.local_to_world;
    let m_clip = global_uniforms.camera_to_clip * global_uniforms.world_to_camera * local_uniforms.local_to_world;
    let m_world_it = transpose(local_uniforms.world_to_local);

    let w_position = (m_world * vec4<f32>(position, 1.0)).xyz;
    let w_normal = normalize((m_world_it * vec4<f32>(normal, 0.0)).xyz);
    let w_tangent = normalize((m_world_it * vec4<f32>(tangent, 0.0)).xyz);
    
    out.position = m_clip * vec4<f32>(position, 1.0);
    out.w_position = w_position;
    out.uv = uvw.xy;
    out.w_normal = w_normal;
    out.w_tangent = w_tangent;
    return out;
}

fn lambertian_reflection(reflectance: vec3<f32>) -> vec3<f32> {;
    return reflectance;
}

fn matte(wi: vec3<f32>, wo: vec3<f32>) -> vec3<f32> {
    // This is a placeholder for a more complex matte reflection model
    let diffuse = max(dot(vec3<f32>(0.0, 0.0, 1.0), wi), 0.0);
    let reflectance = local_uniforms.base_color.rgb;
    return diffuse * lambertian_reflection(reflectance);
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let camera_to_object = normalize(in.w_position - global_uniforms.camera_position.xyz);
    var normal = normalize(in.w_normal);
    if dot(normal, camera_to_object) > 0.0 {
        normal = -normal;
    }
    var tangent = normalize(in.w_tangent);
    var bitangent = normalize(cross(normal, tangent));
    tangent = normalize(cross(bitangent, normal)); // Recompute tangent to ensure orthogonality
    let tbn = transpose(mat3x3<f32>(tangent, bitangent, normal));//tangent space matrix

    var color = vec3<f32>(0.0);
    for (var i: u32 = 0; i < directional_lights.count; i++) {
        let light = directional_lights.lights[i];
        let intensity = light.intensity.rgb;
        var wi = -normalize(light.direction.xyz);
        wi = normalize(tbn * wi); // Transform light direction to tangent space
        var wo = -camera_to_object;// object to camera vector
        wo = normalize(tbn * wo); // Transform view direction to tangent space
        color += matte(wi, wo) * intensity;
    }
    return vec4<f32>(color, 1.0);
}
