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
    direction: vec4<f32>, // Example light direction
    intensity: vec4<f32>, // Example light intensity
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
    cos_inner_angle: f32,    // Angle of the spotlight
    cos_outer_angle: f32,    // Angle of the spotlight
    _pad2: vec2<f32>, // Padding for alignment
}

struct RectLight {
    position: vec4<f32>, // Position in world space
    direction: vec4<f32>, // Direction of the spotlight
    u_axis: vec4<f32>,    // U axis for rectangle // 4 * 4 = 16
    v_axis: vec4<f32>,    // V axis for rectangle // 4 * 4 = 16
    intensity: vec4<f32>, // Light intensity
}

struct InfiniteLight {
    intensity: vec4<f32>, // Light intensity
    indices: vec4<i32>, // Indices for the light texture
    inv_matrix: mat4x4<f32>, // Inverse matrix for the light texture
    //_pad2: vec4<f32>,     // Padding to ensure alignment
    //_pad3: vec4<f32>,     // Padding to ensure alignment
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
var<storage, read> directional_lights: array<DirectionalLight>;

@group(2)
@binding(2)
var<storage, read> sphere_lights: array<SphereLight>;

@group(2)
@binding(3)
var<storage, read> disk_lights: array<DiskLight>;

@group(2)
@binding(4)
var<storage, read> rect_lights: array<RectLight>;

@group(2)
@binding(5)
var<storage, read> infinite_lights: array<InfiniteLight>;

@group(2)
@binding(6)
var light_texture: texture_2d<f32>;//binding_array<texture_2d<f32>>;

@group(2)
@binding(7)
var light_sampler: sampler;

@group(2)
@binding(8)
var ltc_texture_array: texture_2d_array<f32>;// LTC lookup texture

@group(2)
@binding(9)
var ltc_sampler: sampler;

//-------------------------------------------------------
// LTC functions and definitions
const LUT_SIZE: f32 = 64.0; // ltc_texture size
const LUT_SCALE: f32 = (LUT_SIZE - 1.0) / LUT_SIZE;
const LUT_BIAS: f32 = 0.5 / LUT_SIZE;
const IDENTITY_MAT3: mat3x3<f32> = mat3x3<f32>(
    vec3<f32>(1.0, 0.0, 0.0),
    vec3<f32>(0.0, 1.0, 0.0),
    vec3<f32>(0.0, 0.0, 1.0)
);

fn IntegrateEdgeVec(v1: vec3<f32>, v2: vec3<f32>) -> vec3<f32>
{
    let x = dot(v1, v2);
    let y = abs(x);

    let a = 0.8543985 + (0.4965155 + 0.0145206*y)*y;
    let b = 3.4175940 + (4.1616724 + y)*y;
    let v = a / b;

    //let theta_sintheta = if (x > 0.0) { v } else { 0.5*inversesqrt(max(1.0 - x*x, 1e-7)) - v };
    let theta_sintheta = select(0.5*inverseSqrt(max(1.0 - x*x, 1e-7)) - v, v, x > 0.0);

    return cross(v1, v2)*theta_sintheta;
}

fn LTC_Evaluate_Polygon(N: vec3<f32>, V: vec3<f32>, P: vec3<f32>, MinvOrg: mat3x3<f32>, points: array<vec3<f32>, 4>) -> vec3<f32>
{
    // construct orthonormal basis around N
    let T1 = normalize(V - N * dot(V, N));
    let T2 = cross(N, T1);

    let Minv = MinvOrg * transpose(mat3x3<f32>(T1, T2, N));

    var L: array<vec3<f32>, 4>;
    L[0] = Minv * (points[0] - P); 
    L[1] = Minv * (points[1] - P);
    L[2] = Minv * (points[2] - P);
    L[3] = Minv * (points[3] - P);

    let dir = points[0] - P; 
    let lightNormal = cross(points[1] - points[0], points[3] - points[0]);
    let behind = (dot(dir, lightNormal) < 0.0); 

    L[0] = normalize(L[0]);
    L[1] = normalize(L[1]);
    L[2] = normalize(L[2]);
    L[3] = normalize(L[3]);

    var vsum = vec3<f32>(0.0);
    
    vsum += IntegrateEdgeVec(L[0], L[1]);
    vsum += IntegrateEdgeVec(L[1], L[2]);
    vsum += IntegrateEdgeVec(L[2], L[3]);
    vsum += IntegrateEdgeVec(L[3], L[0]);
    
    // form factor of the polygon in direction vsum
    let len = length(vsum);
    var z = vsum.z / len;

    if (behind) {
        z = -z;
    }

    var uv = vec2<f32>(z * 0.5 + 0.5, len); // range [0, 1]
    uv = uv * LUT_SCALE + LUT_BIAS;

    let scale = textureSample(ltc_texture_array, ltc_sampler, uv, 1).w;//
    let sum = len * scale;
    let Lo_i = vec3<f32>(sum, sum, sum);
    return Lo_i;
}

//-------------------------------------------------------
//Material specific definitions
struct BasicMaterialUniforms {
    kd: vec4<f32>,
    ks: vec4<f32>,
    _pad1: vec4<f32>,
    _pad2: vec4<f32>,
}

@group(3)
@binding(0)
var<uniform> material_uniforms: BasicMaterialUniforms;

const MAX_FLOAT: f32 = 1e+10;
const PI: f32 = 3.14159265359;
const INV_PI: f32 = 1.0 / 3.14159265359;

fn lambertian_reflection(r: vec3<f32>) -> vec3<f32> {;
    return r * INV_PI;
}

fn fr_dielectric_(cos_i: f32, eta_i: f32, eta_t: f32) -> f32 {
    let cos_theta_i = cos_i;
    let sin_theta_i = sqrt(max(0.0, 1.0 - cos_theta_i * cos_theta_i));
    let sin_theta_t = eta_i / eta_t * sin_theta_i;
    if sin_theta_t >= 1.0 {
        return 1.0; // Total internal reflection
    }
    let cos_theta_t = sqrt(max(0.0, 1.0 - sin_theta_t * sin_theta_t));
    let rparl = ((eta_t * cos_theta_i) - (eta_i * cos_theta_t)) / ((eta_t * cos_theta_i) + (eta_i * cos_theta_t));
    let rperp = ((eta_i * cos_theta_i) - (eta_t * cos_theta_t)) / ((eta_i * cos_theta_i) + (eta_t * cos_theta_t));
    return (rparl * rparl + rperp * rperp) / 2.0;
}

fn fr_dielectric(cos_i: f32, eta_i: f32, eta_t: f32) -> f32 {
    let cos_theta_i = clamp(cos_i, -1.0, 1.0);
    return select(fr_dielectric_(-cos_theta_i, eta_t, eta_i), fr_dielectric_(cos_theta_i, eta_i, eta_t), cos_theta_i > 0.0);
}

fn cos_2_theta(w: vec3<f32>) -> f32 {
    return w.z * w.z;
}

fn sin_2_theta(w: vec3<f32>) -> f32 {
    return max(0.0, 1.0 - w.z * w.z);
    //return w.x * w.x + w.y * w.y;//max(0.0, 1.0 - w.z * w.z);
}

fn tan_2_theta(w: vec3<f32>) -> f32 {
    return sin_2_theta(w) / cos_2_theta(w);
}

fn cos_phi(w: vec3<f32>) -> f32 {
    return w.x / sqrt(w.x * w.x + w.y * w.y);
}

fn cos_2_phi(w: vec3<f32>) -> f32 {
    let sin = sin_2_theta(w);
    return select(0.0, (w.x * w.x) / sin, sin > 0.0);
}

fn sin_2_phi(w: vec3<f32>) -> f32 {
    let sin = sin_2_theta(w);
    return select(0.0, (w.y * w.y) / sin, sin > 0.0);
}

fn trowbridge_reitz_d(wh: vec3<f32>, alpha: f32) -> f32 {
    let alpha2 = alpha * alpha;
    let tan_2_theta = tan_2_theta(wh);//sin_2_theta(w) / cos_2_theta(w)
    let cos_2_theta = cos_2_theta(wh);
    let cos_4_theta = cos_2_theta * cos_2_theta;
    //let cos_2_phi = cos_2_phi(wh);
    let e = tan_2_theta / alpha2;
    //let e = (cos_2_phi(wh) / (alpha2) + sin_2_phi(wh) / (alpha2)) * tan_2_theta;
    let e2 = pow(1.0 + e, 2.0);
    return 1.0 / (PI * alpha2 * e2);
}

fn trowbridge_reitz_lambda(w: vec3<f32>, alpha: f32) -> f32 {
    let alpha2 = alpha * alpha;
    let tan_2_theta = tan_2_theta(w);
    let alpha_2_tan_2_theta = alpha2 * tan_2_theta;
    return (-1.0 + sqrt(1.0 + alpha_2_tan_2_theta)) / 2.0;
}

fn trowbridge_reitz_g(wo: vec3<f32>, wi: vec3<f32>, alpha: f32) -> f32 {
    let lambda_o = trowbridge_reitz_lambda(wo, alpha);
    let lambda_i = trowbridge_reitz_lambda(wi, alpha);
    return 1.0 / (1.0 + lambda_o + lambda_i);
}

fn micro_facet_reflection(r: vec3<f32>, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    let cos_theta_o = abs(wo.z);
    let cos_theta_i = abs(wi.z);
    var wh = wo + wi;
    if wh.z <= 0.0 {
        return vec3<f32>(0.0);
    }
    wh = normalize(wh);
    let face_forward_wh = select(-wh, wh, dot(wh, vec3<f32>(0.0, 0.0, 1.0)) > 0.0);
    let wi_wh = dot(wi, face_forward_wh);
    let roughness = max(0.001, 0.5);//<roughness>
    //let f = fr_dielectric(wi_wh, 1.5, 1.0);
    let d = trowbridge_reitz_d(wh, roughness);
    //let g = trowbridge_reitz_g(wo, wi, roughness);
    return r * d / (4.0 * cos_theta_i * cos_theta_o);// * d * g / (4.0 * cos_theta_i * cos_theta_o);
}

fn simple_specular(ks: vec3<f32>, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    let wr = reflect(-wo, vec3<f32>(0.0, 0.0, 1.0));
    let cos_alpha = max(dot(wi, wr), 0.0);
    let f = pow(cos_alpha, 20.0);
    return ks * f;
}

fn matte(wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    let diffuse = max(dot(vec3<f32>(0.0, 0.0, 1.0), wi), 0.0);
    let kd = material_uniforms.kd.rgb;
    let c1 = lambertian_reflection(kd);
    return diffuse * c1;
}

fn plastic(wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    let diffuse = max(dot(vec3<f32>(0.0, 0.0, 1.0), wi), 0.0);
    let kd = material_uniforms.kd.rgb;
    let ks = material_uniforms.ks.rgb;
    let c1 = lambertian_reflection(kd);
    let c2 = micro_facet_reflection(ks, wo, wi);
    return diffuse * (c1 + c2);
}

fn shade(intensity: vec3<f32>, wo: vec3<f32>, wi: vec3<f32>) -> vec3<f32> {
    return matte(wo, wi) * intensity;
}
//-------------------------------------------------------

fn test_in_triangle(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, p: vec3<f32>) -> bool {
    let n1 = cross(b - a, p - b);
    let n2 = cross(c - b, p - c);
    let n3 = cross(a - c, p - a);
    let d0 = dot(n1, n2);
    let d1 = dot(n2, n3);
    return d0 > 0.0 && d1 > 0.0;
}

fn closest_point_on_line_segment(a: vec3<f32>, b: vec3<f32>, p: vec3<f32>) -> vec3<f32> {
    let ab = b - a;
    let t = dot(p - a, ab) / dot(ab, ab);
    return a + saturate(t) * ab;
}

fn closest_point_outside_rectangle(a: vec3<f32>, b: vec3<f32>, p: vec3<f32>, n: vec3<f32>, cp: vec4<f32>) -> vec4<f32> {
    let closest_point = closest_point_on_line_segment(a, b, p);
    let l = p - closest_point;
    let distance = dot(l, l);
    let pp = vec4<f32>(closest_point, distance);
    return select(cp, pp, pp.w < cp.w);
}

fn closest_point_on_rectangle(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, d: vec3<f32>, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if !test_in_triangle(a, b, c, p) && !test_in_triangle(a, c, d, p) {
        var cp = vec4<f32>(p, MAX_FLOAT);
        cp = closest_point_outside_rectangle(a, b, p, n, cp);
        cp = closest_point_outside_rectangle(b, c, p, n, cp);
        cp = closest_point_outside_rectangle(c, d, p, n, cp);
        cp = closest_point_outside_rectangle(d, a, p, n, cp);
        return cp.xyz;
    } else {
        return p;
    }
}


fn spherical_texture_lookup(direction: vec3<f32>) -> vec2<f32> {
    // Assumes direction is normalized
    //let u = 0.5 + atan2(direction.z, direction.x) / (2.0 * PI);
    //let v = 0.5 - asin(direction.y) / PI;
    let phi = atan2(direction.y, direction.x);
    let theta = acos(clamp(direction.z, -1.0, 1.0));//0..PI
    let u = phi / (2.0 * PI);
    let v = theta / PI;
    return vec2<f32>(u, v);
}

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

    var color = vec3<f32>(0.0);
    for (var i: u32 = 0; i < light_uniforms.num_directional_lights; i++) {
        let light = directional_lights[i];
        let intensity = light.intensity.rgb;
        var wi = tbn * -normalize(light.direction.xyz);
        color += shade(intensity, wo, wi);
    }
    for (var i: u32 = 0; i < light_uniforms.num_sphere_lights; i++) {
        let light = sphere_lights[i];
        let position = light.position.xyz;
        let intensity = light.intensity.rgb;
        let radius = light.radius;

        var light_to_surface = in.w_position - position;
        if radius > 0.0 {
            let l = light_to_surface;
            let r = reflect(normalize(camera_to_surface), normal);
            let center_to_ray = dot(l, r) * r - l;
            let closest_point = light.position.xyz + center_to_ray * saturate(radius / length(center_to_ray));
            light_to_surface = in.w_position - closest_point;
        }
        let distance = length(light_to_surface);
        let attenuation = 1.0 / pow(1.0 + distance, 2.0); // Simple quadratic attenuation
        var wi = tbn * -normalize(light_to_surface);
        color += shade(intensity * attenuation, wo, wi);
    }
    for (var i: u32 = 0; i < light_uniforms.num_disk_lights; i++) {
        let light = disk_lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;
        let radius = light.radius;
        let cos_inner = light.cos_inner_angle;//cos(light.inner_angle);
        let cos_outer = light.cos_outer_angle;//cos(light.outer_angle);

        var closest_point = position;
        if radius > 0.0 {
            let ray_origin = in.w_position;
            var ray_direction = reflect(camera_to_surface, normal);//ray direction
            let rn = dot(ray_direction, direction);
            if rn > 0.0 {
                ray_direction = ray_direction - 2.0 * rn * direction;
            }
            let pn = dot(position, direction);
            let on = dot(ray_origin, direction);
            let dn = dot(ray_direction, direction);
            let distance_to_plane = (pn - on) / dn;
            //if distance_to_plane < 1e-6 {
            //    continue;
            //}
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
        var attenuation = 1.0 / pow(1.0 + distance, 2.0); // Simple quadratic attenuation
        var wi = tbn * -normalize(light_to_surface);
        color += shade(intensity * attenuation * falloff, wo, wi);
    }

    for (var i: u32 = 0; i < light_uniforms.num_rect_lights; i++) 
    {
        let light = rect_lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;

        let a = position - light.u_axis.xyz - light.v_axis.xyz;
        let b = position - light.u_axis.xyz + light.v_axis.xyz;
        let c = position + light.u_axis.xyz + light.v_axis.xyz;
        let d = position + light.u_axis.xyz - light.v_axis.xyz;

        let V = -camera_to_surface;
        let P = in.w_position;
        let N = normal;
        let lightPoints = array<vec3<f32>, 4>(a, b, c, d);
        let diffuse = LTC_Evaluate_Polygon(N, V, P, IDENTITY_MAT3, lightPoints);

        //var closest_point = closest_point_on_rectangle(a, b, c, d, p, direction);
        let light_to_surface = in.w_position - position;
        let distance = length(light_to_surface);
        var attenuation = 1.0 / pow(1.0 + distance, 2.0); // Simple quadratic attenuation
        color += diffuse * intensity * attenuation;

        /*
        let ray_origin = in.w_position;
        var ray_direction = reflect(camera_to_surface, normal);//ray direction
        let rn = dot(ray_direction, direction);
        if rn > 0.0 {
            ray_direction = ray_direction - 2.0 * rn * direction;
        }
        let pn = dot(position, direction);
        let on = dot(ray_origin, direction);
        let dn = dot(ray_direction, direction);
        let distance_to_plane = (pn - on) / dn;
        if distance_to_plane < 1e-6 {
            continue;
        }
        let p = ray_origin + distance_to_plane * ray_direction;

        let a = position - light.u_axis.xyz - light.v_axis.xyz;
        let b = position - light.u_axis.xyz + light.v_axis.xyz;
        let c = position + light.u_axis.xyz + light.v_axis.xyz;
        let d = position + light.u_axis.xyz - light.v_axis.xyz;
        // Find the closest point on the rectangle
        var closest_point = closest_point_on_rectangle(a, b, c, d, p, direction);
        let light_to_surface = in.w_position - closest_point;
        let cos_theta = max(dot(normalize(light_to_surface), direction), 0.0);
        let falloff = cos_theta;//pow(cos_theta, 1.0);

        let distance = length(light_to_surface);
        var attenuation = 1.0 / pow(1.0 + distance, 2.0); // Simple quadratic attenuation
        var wi = tbn * -normalize(light_to_surface);
        color += shade(intensity * attenuation * falloff, wo, wi);
        */
    }

    for (var i: u32 = 0; i < light_uniforms.num_infinite_lights; i++) 
    {
        let light = infinite_lights[i];
        let intensity = light.intensity.rgb;
        let tex_index = light.indices.x;
        let inv_matrix = light.inv_matrix;
        let r = normalize(reflect(camera_to_surface, normal));
        var val = vec3<f32>(1.0);
        if tex_index >= 0 {
            let rt = inv_matrix * vec4<f32>(r, 0.0);
            let rt2 = normalize(rt.xyz);
            let uv = spherical_texture_lookup(rt2);
            val = textureSample(light_texture, light_sampler, uv).rgb;
        }
        //color += val * intensity;
        let wi = tbn * r;
        color += shade(intensity * val, wo, wi);
    }

    return vec4<f32>(color, 1.0);
}
