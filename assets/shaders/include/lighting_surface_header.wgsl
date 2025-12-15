#ifndef LIGHTING_SURFACE_HEADER_WGSL
#define LIGHTING_SURFACE_HEADER_WGSL

// Lighting surface header definitions
struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_to_world: mat4x4<f32>,
    camera_position: vec4<f32>,
    // Camera position in world space
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    world_to_local: mat4x4<f32>,
    // inverse of world to cameraå
}

struct LightUniforms {
    num_directional_lights: u32,
    num_sphere_lights: u32,
    num_disk_lights: u32,
    num_rect_lights: u32,
    num_infinite_lights: u32,
    _pad1: u32,
    _pad2: u32,
    _pad3: u32,
}

struct DirectionalLight {
    direction: vec4<f32>,
    // Example light direction
    intensity: vec4<f32>,
    // Example light intensity
    radius: f32,
    _pad1: vec3<f32>,
    // Padding for alignment
}

struct SphereLight {
    position: vec4<f32>,
    // Position in world space
    intensity: vec4<f32>,
    // Light intensity
    radius: f32,
    range: f32,
    _pad1: vec2<f32>,
    // Padding for alignment
}

struct DiskLight {
    position: vec4<f32>,
    // Position in world space
    direction: vec4<f32>,
    // Direction of the spotlight
    intensity: vec4<f32>,
    // Light intensity
    radius: f32,
    // Radius of the disk
    range: f32,
    cos_inner_angle: f32,
    // Angle of the spotlight
    cos_outer_angle: f32,
    // Angle of the spotlight
    u_axis: vec4<f32>,
    // U axis for rectangle // 4 * 4 = 16
    v_axis: vec4<f32>,
    // V axis for rectangle // 4 * 4 = 16
    twosided: u32,
    // Whether the rectangle emits light on both sides
    _pad1: u32,
    // Padding to ensure alignment
    _pad2: u32,
    // Padding to ensure alignment
    _pad3: u32,
    // Padding to ensure alignment
}

struct RectLight {
    position: vec4<f32>,
    // Position in world space
    direction: vec4<f32>,
    // Direction of the spotlight
    u_axis: vec4<f32>,
    // U axis for rectangle // 4 * 4 = 16
    v_axis: vec4<f32>,
    // V axis for rectangle // 4 * 4 = 16
    intensity: vec4<f32>,
    // Light intensity
    twosided: u32,
    // Whether the rectangle emits light on both sides
    _pad1: u32,
    // Padding to ensure alignment
    _pad2: u32,
    // Padding to ensure alignment
    _pad3: u32,
    // Padding to ensure alignment
}

struct InfiniteLight {
    intensity: vec4<f32>,
    // Light intensity
    indices: vec4<i32>,
    // Indices for the light texture
    inv_matrix: mat4x4<f32>,
    // Inverse matrix for the light texture
    //_pad2: vec4<f32>,     // Padding to ensure alignment
    //_pad3: vec4<f32>,     // Padding to ensure alignment
}

struct ShadeInput {
    diffuse: vec3<f32>,
    specular: vec3<f32>,
    uv: vec2<f32>,
    fresnel: vec2<f32>,
}

// global uniforms
@group(0) @binding(0)
var<uniform> global_uniforms: GlobalUniforms;

// local uniforms
@group(1) @binding(0)
var<uniform> local_uniforms: LocalUniforms;

#endif