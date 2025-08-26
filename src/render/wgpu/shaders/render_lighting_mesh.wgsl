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

struct LightUniforms {
    num_directional_lights: u32,
    num_sphere_lights: u32,
    num_disk_lights: u32,
    _pad1: u32, // Padding for alignment
}

struct SphereLight {
    position: vec4<f32>, // Position in world space
    intensity: vec4<f32>, // Light intensity
    radius: f32,
    range: f32,
    _pad1: vec2<f32>, // Padding for alignment
}

struct DiskLight {
    position: vec4<f32>, // Position in world space
    direction: vec4<f32>, // Direction of the spotlight
    intensity: vec4<f32>, // Light intensity
    radius: f32,    // Radius of the disk
    range: f32,
    _pad1: vec2<f32>, // Padding for alignment
    inner_angle: f32,    // Angle of the spotlight
    outer_angle: f32,    // Angle of the spotlight
    _pad2: vec2<f32>, // Padding for alignment
}

struct DirectionalLight {
    direction: vec4<f32>, // Example light direction
    intensity: vec4<f32>, // Example light intensity
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
var<uniform> light_uniforms: LightUniforms;
//var<uniform> directional_lights: DirectionalLightUniforms;

@group(2)
@binding(1)
var<storage, read> sphere_lights: array<SphereLight>;

@group(2)
@binding(2)
var<storage, read> disk_lights: array<DiskLight>;

@group(2)
@binding(3)
var<storage, read> directional_lights: array<DirectionalLight>;


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
    let camera_to_surface = normalize(in.w_position - global_uniforms.camera_position.xyz);
    var normal = normalize(in.w_normal);
    if dot(normal, camera_to_surface) > 0.0 {
        normal = -normal;
    }
    var tangent = normalize(in.w_tangent);
    var bitangent = normalize(cross(normal, tangent));
    tangent = normalize(cross(bitangent, normal)); // Recompute tangent to ensure orthogonality
    let tbn = transpose(mat3x3<f32>(tangent, bitangent, normal));//tangent space matrix
    let wo = tbn * -camera_to_surface;// object to camera vector

    var color = vec3<f32>(0.0);
    for (var i: u32 = 0; i < light_uniforms.num_directional_lights; i++) {
        let light = directional_lights[i];
        let intensity = light.intensity.rgb;
        var wi = tbn * -normalize(light.direction.xyz);
        color += matte(wi, wo) * intensity;
    }
    for (var i: u32 = 0; i < light_uniforms.num_sphere_lights; i++) {
        let light = sphere_lights[i];
        let intensity = light.intensity.rgb;
        let radius = light.radius;

        var light_to_surface = in.w_position - light.position.xyz;
        if radius > 0.0 {
            let l = light_to_surface;
            let r = reflect(camera_to_surface, normal);
            let center_to_ray = dot(l, r) * r - l;
            let closest_point = light.position.xyz + center_to_ray * saturate(radius / length(center_to_ray));
            light_to_surface = in.w_position - closest_point;
        }
        let distance = length(light_to_surface);
        let attenuation = 1.0 / pow(distance, 2.0); // Simple quadratic attenuation
        var wi = tbn * -normalize(light_to_surface);
        color += matte(wi, wo) * intensity * attenuation;
    }
    for (var i: u32 = 0; i < light_uniforms.num_disk_lights; i++) {
        let light = disk_lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;
        let radius = light.radius;
        let cos_inner = cos(light.inner_angle);
        let cos_outer = cos(light.outer_angle);

        var closest_point = position;
        let center_to_surface = in.w_position - position;
        
        if radius > 0.0 {
            let center_to_surface = in.w_position - position;
            let direction_to_surface = select(-direction, direction, dot(center_to_surface, direction) > 0.0);
            let ray_origin = in.w_position;
            let ray_direction = reflect(camera_to_surface, normal);//ray direction
            let d = dot(position, direction_to_surface);
            let distance_to_plane = (d - dot(ray_origin, direction_to_surface)) / dot(ray_direction, direction_to_surface);
            let intersection_point = ray_origin + distance_to_plane * ray_direction;
            let center_to_intersection = intersection_point - position;
            closest_point = position + center_to_intersection * saturate(radius / length(center_to_intersection));
        }
        let light_to_surface = in.w_position - closest_point;
        let cos_theta = max(dot(normalize(light_to_surface), direction), 0.0);
        let cos_delta = cos_inner - cos_outer;
        var falloff = select(step(cos_outer, cos_theta), clamp((cos_theta - cos_outer) / cos_delta, 0.0, 1.0), cos_delta > 0.0);
        if radius <= 0.0 {
            falloff = pow(falloff, 4.0);
        }
        let distance = length(light_to_surface);
        var attenuation = 1.0 / (pow(distance + 1.0, 2.0) + 1e-6); // Simple quadratic attenuation
        var wi = tbn * -normalize(light_to_surface);
        color += matte(wo, wo) * intensity * attenuation * falloff;
    }

    return vec4<f32>(color, 1.0);
}

/*
let a0 = cos(light.inner_angle);
let a1 = cos(light.outer_angle);
var light_to_surface = in.w_position - light.position.xyz;
var distance = length(light_to_surface);
light_to_surface = normalize(light_to_surface);
let cos_theta = max(dot(light_to_surface, direction), 0.0);
var falloff = select(step(a1, cos_theta), pow(clamp((cos_theta - a1) / (a0 - a1), 0.0, 1.0), 4.0), (a0 - a1) > 0.0);//select(FALSE, TRUE, condition)
let attenuation = 1.0 / pow(distance, 2.0); // Simple quadratic attenuation
var wi = tbn * -light_to_surface;
color += matte(wi, wo) * intensity * attenuation * falloff;

*/
