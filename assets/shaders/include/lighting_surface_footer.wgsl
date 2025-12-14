#ifndef LIGHTING_SURFACE_FOOTER_WGSL
#define LIGHTING_SURFACE_FOOTER_WGSL

// Lighting surface footer definitions
//-------------------------------------------------------
// light uniforms
@group(3)
@binding(0)
var<uniform> light_uniforms: LightUniforms;
//var<uniform> directional_lights: DirectionalLightUniforms;

@group(3)
@binding(1)
var<storage, read> directional_lights: array<DirectionalLight>;

@group(3)
@binding(2)
var<storage, read> sphere_lights: array<SphereLight>;

@group(3)
@binding(3)
var<storage, read> disk_lights: array<DiskLight>;

@group(3)
@binding(4)
var<storage, read> rect_lights: array<RectLight>;

@group(3)
@binding(5)
var<storage, read> infinite_lights: array<InfiniteLight>;

@group(3)
@binding(6)
var light_texture: texture_2d<f32>;//binding_array<texture_2d<f32>>;

@group(3)
@binding(7)
var light_sampler: sampler;

@group(4)
@binding(0)
var ltc_texture_array: texture_2d_array<f32>;// LTC lookup texture

@group(4)
@binding(1)
var ltc_sampler: sampler;

//-------------------------------------------------------
const MAX_FLOAT: f32 = 1e+10;
const PI: f32 = 3.14159265359;
const INV_PI: f32 = 1.0 / 3.14159265359;
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
    let theta_sintheta = select(0.5 * inverseSqrt(max(1.0 - x*x, 1e-7)) - v, v, x > 0.0);

    return cross(v1, v2)*theta_sintheta;
}

fn sign(x: f32) -> f32 {
    return select(1.0, -1.0, x < 0.0);
}

fn cbrt(x: f32) -> f32 {
    return pow(x, 1.0 / 3.0);
}

fn SolveCubicOneRoot(A: f32, B: f32, C: f32, D: f32, Discriminant: f32, Delta: vec3<f32>) -> vec4<f32>
{
    let B3D = B*B*B*D;
    let AC3 = A*C*C*C;

    var Ax: f32;
    var Cx: f32;
    var Dx: f32;
    if (B3D >= AC3) {
        Ax = A;
        Cx = Delta.x;//
        Dx = -2.0*B*Delta.x + A*Delta.y;//
    } else {
        Ax = D;
        Cx = Delta.z;//
        Dx = -D*Delta.y + 2.0*C*Delta.z;//
    }
    let T0 = -sign(Dx) * abs(Ax) * sqrt(-Discriminant);
    let T1 = -Dx + T0;
    let p = (0.5 * T1);
    let q = select(-Cx/p, -p, T1 == T0);
    let x1 = select(-Dx / (p*p + q*q + Cx), p + q, Cx <= 0.0);
    
    if (B3D >= AC3) {
        let xc = vec2<f32>(x1 - B, A);
        return vec4<f32>(xc.x/xc.y, 0.0, 0.0, 1.0);
    } else {
        let xc = vec2<f32>(-D, x1 + C);
        return vec4<f32>(xc.x/xc.y, 0.0, 0.0, 1.0);
    }
}

fn SolveCubicThreeRoots(A: f32, B: f32, C: f32, D: f32, Discriminant: f32, Delta: vec3<f32>) -> vec4<f32>
{
    var RootsA = vec3<f32>(0.0);
    var RootsD = vec3<f32>(0.0);

    var xlc = vec2<f32>(0.0);
    var xsc = vec2<f32>(0.0);
    // Algorithm A
    {
        let A_a = 1.0;
        let C_a = Delta.x;
        let D_a = -2.0*B*Delta.x + Delta.y;

        // Take the cubic root of a normalized complex number
        let Theta = atan2(sqrt(Discriminant), -D_a)/3.0;

        let x_1a = 2.0*sqrt(-C_a)*cos(Theta);
        let x_3a = 2.0*sqrt(-C_a)*cos(Theta + (2.0/3.0)*PI);

        var xl: f32;
        if ((x_1a + x_3a) > 2.0*B) {
            xl = x_1a;
        } else {
            xl = x_3a;
        }

        xlc = vec2(xl - B, A);
    }

    // Algorithm D
    {
        let A_d = D;
        let C_d = Delta.z;
        let D_d = -D*Delta.y + 2.0*C*Delta.z;

        // Take the cubic root of a normalized complex number
        let Theta = atan2(D*sqrt(Discriminant), -D_d)/3.0;

        let x_1d = 2.0*sqrt(-C_d)*cos(Theta);
        let x_3d = 2.0*sqrt(-C_d)*cos(Theta + (2.0/3.0)*PI);

        var xs: f32;
        if ((x_1d + x_3d) < 2.0*C) {
            xs = x_1d;
        } else {
            xs = x_3d;
        }

        xsc = vec2<f32>(-D, xs + C);
    }

    let E =  xlc.y*xsc.y;
    let F = -xlc.x*xsc.y - xlc.y*xsc.x;
    let G =  xlc.x*xsc.x;

    let xmc = vec2<f32>(C*F - B*G, -B*F + C*E);

    let Root_ = vec3<f32>(xsc.x/xsc.y, xmc.x/xmc.y, xlc.x/xlc.y);

    var Root = Root_;
    if (Root.x < Root.y && Root.x < Root.z) {
        Root = Root_.yxz;
    } else if (Root.z < Root.x && Root.z < Root.y) {
        Root = Root_.xzy;
    }
    return vec4<f32>(Root, 3.0);
}

fn SolveCubic(a: f32, b: f32, c: f32, d: f32) -> vec4<f32>
{
    let A = a;
    let B = b / (3.0 * a);
    let C = c / (3.0 * a);
    let D = d / a;

    // Compute the Hessian and the discriminant
    let Delta = vec3<f32>(
        -B*B + C,   //AC-BB
        -B*C + D,   //AD-BC
        -C*C + B*D  //BD-CC
    );

    let Discriminant = dot(vec2(4.0*Delta.x, -Delta.y), Delta.zy);//4*Delta.x * Delta.z - (Delta.y)^2
    if (Discriminant < 0.0) {
        return SolveCubicOneRoot(A, B, C, D, Discriminant, Delta);
    } else if (Discriminant > 0.0) {
        return SolveCubicThreeRoots(A, B, C, D, Discriminant, Delta);
    } else {
        return vec4<f32>(0.0);
    }
}

fn LTC_Evaluate_Polygon(N: vec3<f32>, V: vec3<f32>, P: vec3<f32>, MinvOrg: mat3x3<f32>, points: array<vec3<f32>, 4>) -> vec3<f32>
{
    // construct orthonormal basis around N
    let T1 = normalize(V - N * dot(V, N));
    let T2 = cross(N, T1);

    let Minv = MinvOrg * transpose(mat3x3<f32>(T1, T2, N));

    let L = array<vec3<f32>, 4>(
        normalize(Minv * (points[0] - P)),
        normalize(Minv * (points[1] - P)),
        normalize(Minv * (points[2] - P)),
        normalize(Minv * (points[3] - P))
    );

    let dir = points[0] - P; 
    let lightNormal = cross(points[1] - points[0], points[3] - points[0]);
    let behind = (dot(dir, lightNormal) < 0.0); 

    //L[0] = normalize(L[0]);
    //L[1] = normalize(L[1]);
    //L[2] = normalize(L[2]);
    //L[3] = normalize(L[3]);

    var vsum = vec3<f32>(0.0);
    
    vsum += IntegrateEdgeVec(L[0], L[1]);
    vsum += IntegrateEdgeVec(L[1], L[2]);
    vsum += IntegrateEdgeVec(L[2], L[3]);
    vsum += IntegrateEdgeVec(L[3], L[0]);
    
    // form factor of the polygon in direction vsum
    let len = length(vsum);
    //var z = vsum.z / len;
    //if (behind) {
    //    z = -z;
    //}
    let z = select(vsum.z / len, -vsum.z / len, behind);

    var uv = vec2<f32>(z * 0.5 + 0.5, len); // range [0, 1]
    uv = uv * LUT_SCALE + LUT_BIAS;

    let scale = textureSample(ltc_texture_array, ltc_sampler, uv, 1).w;//
    let sum = len * scale;
    let Lo_i = vec3<f32>(sum, sum, sum);
    return Lo_i;
}

fn LTC_Evaluate_Disk(N: vec3<f32>, V: vec3<f32>, P: vec3<f32>, Minv: mat3x3<f32>, points: array<vec3<f32>, 4>) -> vec3<f32>
{
    // construct orthonormal basis around N
    let T1 = normalize(V - N*dot(V, N));
    let T2 = cross(N, T1);

    // rotate area light in (T1, T2, N) basis
    let R = transpose(mat3x3<f32>(T1, T2, N));

    // 3 of the 4 vertices around disk
    var L_ = array<vec3<f32>, 3>(
        R * (points[0] - P),
        R * (points[1] - P),
        R * (points[2] - P)
    );

    // init ellipse
    var C  = 0.5 * (L_[0] + L_[2]); // center
    var V1 = 0.5 * (L_[1] - L_[2]); // axis 1
    var V2 = 0.5 * (L_[1] - L_[0]); // axis 2

    // back to cosine distribution, but V1 and V2 no longer ortho.
    C  = Minv * C;
    V1 = Minv * V1;
    V2 = Minv * V2;

    // compute eigenvectors of ellipse
    var a: f32;
    var b: f32;
    let d11 = dot(V1, V1); // q11
    let d22 = dot(V2, V2); // q22
    let d12 = dot(V1, V2); // q12
    let d1122 = d11*d22;
    if (d1122 > 0.0 && abs(d12)/sqrt(d1122) > 0.0001) {
        let tr = d11 + d22;
        let det0 = -d12*d12 + d11*d22;

        var det = sqrt(det0);

        // use sqrt matrix to solve for eigenvalues
        let u = 0.5*sqrt(tr - 2.0*det);
        let v = 0.5*sqrt(tr + 2.0*det);
        let e_max = (u + v) * (u + v); // e2
        let e_min = (u - v) * (u - v); // e1

        // two eigenvectors
        var V1_: vec3<f32>;
        var V2_: vec3<f32>;

        // q11 > q22
        if (d11 > d22)
        {
            V1_ = d12*V1 + (e_max - d11)*V2; // E2
            V2_ = d12*V1 + (e_min - d11)*V2; // E1
        }
        else
        {
            V1_ = d12*V2 + (e_max - d22)*V1;
            V2_ = d12*V2 + (e_min - d22)*V1;
        }

        a = 1.0 / e_max;
        b = 1.0 / e_min;
        V1 = normalize(V1_); // Vx
        V2 = normalize(V2_); // Vy
    }
    else
    {
        // Eigenvalues are diagnoals
        a = 1.0 / dot(V1, V1);
        b = 1.0 / dot(V2, V2);
        V1 *= sqrt(a);
        V2 *= sqrt(b);
    }

    var V3 = cross(V1, V2);
    if (dot(C, V3) < 0.0) {
        V3 *= -1.0;
    }

    let L  = dot(V3, C);
    let x0 = dot(V1, C) / L;
    let y0 = dot(V2, C) / L;

    //let E1 = isqrt(a);
    //let E2 = isqrt(b);

    a *= L*L;
    b *= L*L;

    // parameters for solving cubic function
    let c0 = a*b;
    let c1 = a*b*(1.0 + x0*x0 + y0*y0) - a - b;
    let c2 = 1.0 - a*(1.0 + x0*x0) - b*(1.0 + y0*y0);
    let c3 = 1.0;

    let rotate = mat3x3<f32>(V1, V2, V3);
    // 3D eigen-decomposition: need to solve a cubic function
    let roots = SolveCubic(c3, c2, c1, c0);
    let numRoots = roots.w;
    if numRoots <= 1.0 {
        var e1 = roots.x;
        var e2 = roots.x;
        var e3 = roots.x;
        // direction to front-facing ellipse center
        //var avgDir = vec3<f32>(a*x0/(a - e2), b*y0/(b - e2), 1.0);
        var avgDir = vec3<f32>(0.0, 0.0, 1.0);
        //var avgDir = vec3<f32>(1.0, 1.0, 1.0);

        // transform to V1, V2, V3 basis
        avgDir = rotate * avgDir;
        avgDir = normalize(avgDir);

        // extends of front-facing ellipse
        // projected solid angle E, like the length(F) in rectangle light
        let formFactor = e2*e2;//(-1) * isqrt(0) ;//- 1 / sqrt(0) -> -infinity
        // use tabulated horizon-clipped sphere
        var uv = vec2<f32>(avgDir.z*0.5 + 0.5, formFactor);
        //uv = saturate(uv);
        uv = uv * LUT_SCALE + LUT_BIAS;
        let scale = textureSample(ltc_texture_array, ltc_sampler, uv, 1).w;

        let spec = formFactor * scale;
        let Lo_i = vec3<f32>(spec, spec, spec);
        //let Lo_i = vec3<f32>(c0, c1, c2);
        return Lo_i;
    } else {
        var e1 = roots.x;
        var e2 = roots.y;
        var e3 = roots.z;

        // direction to front-facing ellipse center
        var avgDir = vec3<f32>(a*x0/(a - e2), b*y0/(b - e2), 1.0); // third eigenvector: V-

        // transform to V1, V2, V3 basis
        avgDir = rotate * avgDir;
        avgDir = normalize(avgDir);

        // extends of front-facing ellipse
        let L1 = sqrt(-e2/e3);
        let L2 = sqrt(-e2/e1);

        // projected solid angle E, like the length(F) in rectangle light
        let formFactor = L1 * L2 * inverseSqrt((1.0 + L1 * L1) * (1.0 + L2 * L2)) ;
        // use tabulated horizon-clipped sphere
        var uv = vec2<f32>(avgDir.z*0.5 + 0.5, formFactor);
        // uv = saturate(uv);
        uv = uv * LUT_SCALE + LUT_BIAS;
        let scale = textureSample(ltc_texture_array, ltc_sampler, uv, 1).w;

        let spec = formFactor * scale;
        let Lo_i = vec3<f32>(spec, spec, spec);
        //let Lo_i = vec3<f32>(c0, c1, c2);
        return Lo_i;
    }
}



//-------------------------------------------------------
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

fn get_u_axis(z: vec3<f32>) -> vec3<f32> {
    if (abs(z.x) <= abs(z.y) && abs(z.x) <= abs(z.z)) {
        return vec3<f32>(1.0, 0.0, 0.0);
    } else if (abs(z.y) <= abs(z.x) && abs(z.y) <= abs(z.z)) {
        return vec3<f32>(0.0, 1.0, 0.0);
    } else {
        return vec3<f32>(0.0, 0.0, 1.0);
    }
}

fn calc_attenuation(distance: f32) -> f32 {
    return 1.0 / (1.0 + distance);
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

    let V = -camera_to_surface;//point to camera
    let P = in.w_position;
    let N = normal;
    let NdotV = saturate(dot(N, V));

    
#ifdef ENABLE_SPECULAR
    let roughness = sample_roughness(in.uv);
    var ltc_uv = vec2<f32>(roughness, sqrt(1.0 - NdotV));
    ltc_uv = ltc_uv * LUT_SCALE + LUT_BIAS;
    let t1 = textureSample(ltc_texture_array, ltc_sampler, ltc_uv, 0);
    // Construct inverse matrix
    let Minv = mat3x3<f32>(
        vec3<f32>(t1.x, 0.0, t1.y),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(t1.z, 0.0, t1.w)
    );
    let t2 = textureSample(ltc_texture_array, ltc_sampler, ltc_uv, 1);
    let fresnel = vec2<f32>(t2.x, t2.y);
#else 
    let fresnel = vec2<f32>(1.0, 0.0);
#endif
    // Accumulate lighting
    
    var color = vec3<f32>(0.0);
    for (var i: u32 = 0; i < light_uniforms.num_directional_lights; i++) {
        let light = directional_lights[i];
        let intensity = light.intensity.rgb;
        var wi = tbn * -normalize(light.direction.xyz);
        color += shade(intensity, wo, wi, in.uv);
    }

    for (var i: u32 = 0; i < light_uniforms.num_sphere_lights; i++) {
        let light = sphere_lights[i];
        let position = light.position.xyz;//center of sphere
        let intensity = light.intensity.rgb;
        let radius = light.radius;

        let center_to_surface = in.w_position - position;
        let distance = length(center_to_surface);
        if (distance < radius) {
            continue;
        }
        if radius > 0.0 {
            let direction = normalize(center_to_surface);
            let theta = atan2(radius, distance);
            let delta = radius * sin(theta);
            let disk_distance = distance - delta;
            if (disk_distance < 1e-6) {
                continue;
            }
            let disk_radius = radius * cos(theta);
            if (disk_radius < 1e-6) {
                continue;
            }
            var u_axis = get_u_axis(direction);
            var v_axis = normalize(cross(direction, u_axis));
            u_axis = normalize(cross(v_axis, direction));
            //TODO: avoid precision issue
            //
            let ex = u_axis * disk_radius * 0.999;//avoid precision issue
            let ey = v_axis * disk_radius * 1.001;//avoid precision issue
            let disk_center = position + direction * delta;

            let a = disk_center - ex - ey;
            let b = disk_center + ex - ey;
            let c = disk_center + ex + ey;
            let d = disk_center - ex + ey;

            let lightPoints = array<vec3<f32>, 4>(a, b, c, d);
            //let lightPoints = array<vec3<f32>, 4>(d, c, b, a);
#ifdef ENABLE_DIFFUSE
            let diffuse = LTC_Evaluate_Disk(N, V, P, IDENTITY_MAT3, lightPoints);
#else
            let diffuse = vec3<f32>(0.0);
#endif
#ifdef ENABLE_SPECULAR
            let specular = LTC_Evaluate_Disk(N, V, P, Minv, lightPoints);
#else
            let specular = vec3<f32>(0.0);
#endif

            let k = 4.0;// 1.0 / (2.0 * PI * PI);
            var area = PI * disk_radius * disk_radius;          // Area of the disk
            var attenuation = calc_attenuation(disk_distance);
            let ltc_input = LTCShadeInput(diffuse, specular, in.uv, fresnel);
            color += k * area * intensity * attenuation * shade_ltc(ltc_input);
        } else {
            let light_to_surface = in.w_position - position;
            let distance = length(light_to_surface);
            let attenuation = 1.0 / pow(1.0 + distance, 2.0); // Simple quadratic attenuation
            var wi = tbn * -normalize(light_to_surface);
            color += shade(intensity * attenuation, wo, wi, in.uv);
        }
    }

    for (var i: u32 = 0; i < light_uniforms.num_disk_lights; i++) {
        let light = disk_lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;
        var radius = light.radius;
        let cos_inner = light.cos_inner_angle;//cos(light.inner_angle);
        let cos_outer = light.cos_outer_angle;//cos(light.outer_angle);
        let u_axis = light.u_axis.xyz;
        let v_axis = light.v_axis.xyz;
        let twosided = light.twosided;
        let is_spot = light.radius <= 0.0;
        let center_to_surface = in.w_position - position;
        let distance = length(center_to_surface);
        if (distance < 1e-6) {
            continue;
        }
        let dd = dot(center_to_surface, direction);
        if (dd < 0.0 && twosided == 0) {
            continue;
        }
        //let ndotl = dot(normal, direction);
        //if (ndotl >= 0.0 && twosided == 0) {
        //    continue;
        //}

        if is_spot {
            radius = 1.0;//radius = distance * tan(acos(cos_outer));
        }
        let ex = radius * u_axis * 0.5 * 0.999;
        let ey = radius * v_axis * 0.5 * 1.001;

        let a = position - ex - ey;
        let b = position + ex - ey;
        let c = position + ex + ey;
        let d = position - ex + ey;

        let lightPoints = array<vec3<f32>, 4>(a, b, c, d);
#ifdef ENABLE_DIFFUSE
        let diffuse = LTC_Evaluate_Disk(N, V, P, IDENTITY_MAT3, lightPoints);
#else
        let diffuse = vec3<f32>(0.0);
#endif
#ifdef ENABLE_SPECULAR
        let specular = LTC_Evaluate_Disk(N, V, P, Minv, lightPoints);
#else
        let specular = vec3<f32>(0.0);
#endif
        var k = 1.0 / (2.0 * PI * PI);
        var area = PI * radius * radius;
        var falloff = 1.0; // full light for disk area light
        if is_spot {
            k = 2.0;
            area = 4.0 * PI;
            let cos_theta = max(dot(normalize(center_to_surface), direction), 0.0);
            let cos_delta = cos_inner - cos_outer;
            falloff = select(step(cos_outer, cos_theta), clamp((cos_theta - cos_outer) / cos_delta, 0.0, 1.0), cos_delta > 0.0);
            falloff = pow(falloff, 4.0);
        }
        var attenuation = calc_attenuation(distance);
        let ltc_input = LTCShadeInput(diffuse, specular, in.uv, fresnel);
        color += k * area * intensity * attenuation * falloff * shade_ltc(ltc_input);
    }

    for (var i: u32 = 0; i < light_uniforms.num_rect_lights; i++) 
    {
        let light = rect_lights[i];
        let position = light.position.xyz;
        let direction = normalize(light.direction.xyz);
        let intensity = light.intensity.rgb;
        let twosided = light.twosided;
        let center_to_surface = in.w_position - position;
        let distance = length(center_to_surface);
        if (distance < 1e-6) {
            continue;
        }
        let dd = dot(center_to_surface, direction);
        if (dd < 0.0 && twosided == 0) {
            continue;
        }
        //let ndotl = dot(normal, direction);
        //if (ndotl >= 0.0 && twosided == 0) {
        //    continue;
        //}

        let a = position - light.u_axis.xyz - light.v_axis.xyz;
        let b = position - light.u_axis.xyz + light.v_axis.xyz;
        let c = position + light.u_axis.xyz + light.v_axis.xyz;
        let d = position + light.u_axis.xyz - light.v_axis.xyz;

        let lightPoints = array<vec3<f32>, 4>(a, b, c, d);
#ifdef ENABLE_DIFFUSE
        let diffuse = LTC_Evaluate_Polygon(N, V, P, IDENTITY_MAT3, lightPoints);
#else
        let diffuse = vec3<f32>(0.0);
#endif
#ifdef ENABLE_SPECULAR
        let specular = LTC_Evaluate_Polygon(N, V, P, Minv, lightPoints);
        //let specular = specular * (mSpecular * t2.x + (1.0 - mSpecular) * t2.y);//
#else   
        let specular = vec3<f32>(0.0);
#endif
        let k = 1.0 / (2.0 * PI * PI);
        let area = 1.0;
        let attenuation = calc_attenuation(distance);
        let ltc_input = LTCShadeInput(diffuse, specular, in.uv, fresnel);
        color += k * area * intensity * attenuation * shade_ltc(ltc_input);
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
        color += shade(intensity * val, wo, wi, in.uv);
    }
    
    return vec4<f32>(color, 1.0);
}

#endif // LIGHTING_SURFACE_FOOTER_WGSL