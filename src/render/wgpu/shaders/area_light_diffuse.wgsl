struct GlobalUniforms {
    world_to_camera: mat4x4<f32>,
    camera_to_clip: mat4x4<f32>,
    camera_to_world: mat4x4<f32>,
    camera_position: vec4<f32>, // Camera position in world space
}

struct LocalUniforms {
    local_to_world: mat4x4<f32>,
    world_to_local: mat4x4<f32>, // inverse of world to camera
    _pad1: vec4<f32>,
    _pad2: vec4<f32>,
    _pad3: vec4<f32>,
    _pad4: vec4<f32>,
}

// global uniforms
@group(0)
@binding(0)
var<uniform> global_uniforms: GlobalUniforms;

// local uniforms
@group(1)
@binding(0)
var<uniform> local_uniforms: LocalUniforms;

struct AreaLightDiffuseMaterialUniforms {
    l: vec4<f32>,
    scale: vec4<f32>,
    _pad1: vec4<f32>,
    _pad2: vec4<f32>,
}

@group(2)
@binding(0)
var<uniform> material_uniforms: AreaLightDiffuseMaterialUniforms;
//-------------------------------------------------------
const MAX_FLOAT: f32 = 1e+10;
const PI: f32 = 3.14159265359;
const INV_PI: f32 = 1.0 / 3.14159265359;
//-------------------------------------------------------


fn shade_nolight(wo: vec3<f32>, uv: vec2<f32>) -> vec3<f32> {
    let l = material_uniforms.l.xyz;
    let scale = material_uniforms.scale.xyz;
    let color = scale * l;
    //let d = max(dot(wo, vec3<f32>(0.0, 0.0, 1.0)), 0.0);
    //let d_color = color * d * INV_PI;
    return color;
}



//-------------------------------------------------------
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
    let uv = in.uv;

    let color = shade_nolight(wo, uv);
    return vec4<f32>(color, 1.0);
}
