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

struct PointLight {
    position: vec4<f32>, // Position in world space
    intensity: vec4<f32>, // Light intensity
    range: vec4<f32>, // Range of the light (min, max)
    _pad1: vec4<f32>, // Padding for alignment
}

const MAX_POINT_LIGHTS: u32 = 4; // Maximum number of point lights
struct PointLightUniforms {
    lights: array<PointLight, MAX_POINT_LIGHTS>,
    count: u32,
}

struct SpotLight {
    position: vec4<f32>, // Position in world space
    direction: vec4<f32>, // Direction of the spotlight
    intensity: vec4<f32>, // Light intensity
    range: vec4<f32>, // Range of the light (min, max)
    outer_angle: f32,    // Angle of the spotlight
    inner_angle: f32,    // Angle of the spotlight
    _pad1: vec2<f32>, // Padding for alignment
}

const MAX_SPOT_LIGHTS: u32 = 4; // Maximum number of spot lights
struct SpotLightUniforms {
    lights: array<SpotLight, MAX_SPOT_LIGHTS>,
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

@group(2)
@binding(1)
var<uniform> point_lights: PointLightUniforms;

@group(2)
@binding(2)
var<uniform> spot_lights: SpotLightUniforms;


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
    for (var i: u32 = 0; i < point_lights.count; i++) {
        let light = point_lights.lights[i];
        let intensity = light.intensity.rgb;
        let r0 = light.range[0];
        let r1 = light.range[1];
        var light_to_object = in.w_position - light.position.xyz;
        var distance = length(light_to_object);
        let attenuation = 1.0 / pow(max((distance - r0), 1e-6), 2.0); // Simple quadratic attenuation
        var wi = -normalize(light_to_object);
        wi = normalize(tbn * wi); // Transform light direction to tangent space
        var wo = -camera_to_object;// object to camera vector
        wo = normalize(tbn * wo); // Transform view direction to tangent space
        color += matte(wi, wo) * intensity * attenuation;
    }
    for (var i: u32 = 0; i < spot_lights.count; i++) {
        let light = spot_lights.lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;
        let r0 = light.range[0];
        let r1 = light.range[1];
        let a0 = cos(light.inner_angle * 0.5);
        let a1 = cos(light.outer_angle * 0.5);
        var light_to_object = in.w_position - light.position.xyz;
        var distance = length(light_to_object);
        light_to_object = normalize(light_to_object);
        let cos_angle = max(dot(light_to_object, direction), 0.0);
        var falloff = pow(select(step(a1, cos_angle), clamp((cos_angle - a1) / (a0 - a1), 0.0, 1.0), (a0 - a1) > 0.0), 4.0);//select(FALSE, TRUE, condition)
        let attenuation = 1.0 / pow(max((distance - r0), 1e-6), 2.0); // Simple quadratic attenuation
        var wi = -light_to_object;
        wi = normalize(tbn * wi); // Transform light direction to tangent space
        var wo = -camera_to_object;// object to camera vector
        wo = normalize(tbn * wo); // Transform view direction to tangent space
        color += matte(wi, wo) * intensity * attenuation * falloff;
    }

    return vec4<f32>(color, 1.0);
}
