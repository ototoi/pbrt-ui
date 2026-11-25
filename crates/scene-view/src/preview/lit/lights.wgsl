// One function per light kind; `light_irradiance` only dispatches. Adding a
// kind means one function here, one `case`, and its Rust conversion.

// Irradiance at `p` from a positional light of intensity `intensity` toward
// the surface; zero when the light sits on the shading point.
fn positional(position: vec3<f32>, intensity: vec3<f32>, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let delta = position - p;
    let d2 = dot(delta, delta);
    if d2 < 1e-8 {
        return vec3<f32>(0.0);
    }
    let l = delta / sqrt(d2);
    return intensity * max(dot(n, l), 0.0) / d2;
}

fn point_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    return positional(light.position.xyz, light.color.rgb, p, n);
}

// pbrt-v4 spot: smoothstep(cosEnd, cosStart, cos(angle to the axis)).
fn spot_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let l = normalize(light.position.xyz - p);
    let falloff = smoothstep(light.color.w, light.direction.w, dot(-l, light.direction.xyz));
    return positional(light.position.xyz, light.color.rgb * falloff, p, n);
}

fn distant_light(light: Light, n: vec3<f32>) -> vec3<f32> {
    return light.color.rgb * max(dot(n, light.direction.xyz), 0.0);
}

// Uniform infinite light: irradiance pi * L over the visible hemisphere.
fn infinite_light(light: Light) -> vec3<f32> {
    return PI * light.color.rgb;
}

// Image-based lights: the cosine-convolved radiance at normal `n`, from
// lit::environment's coefficients. Band 2 can ring slightly below zero.
fn environment_irradiance(n: vec3<f32>) -> vec3<f32> {
    let weights = array<f32, 9>(
        0.282095,
        0.488603 * n.y,
        0.488603 * n.z,
        0.488603 * n.x,
        1.092548 * n.x * n.y,
        1.092548 * n.y * n.z,
        0.315392 * (3.0 * n.z * n.z - 1.0),
        1.092548 * n.x * n.z,
        0.546274 * (n.x * n.x - n.y * n.y),
    );
    var sum = vec3<f32>(0.0);
    for (var i = 0u; i < 9u; i = i + 1u) {
        sum += weights[i] * environment_irradiance_sh[i].rgb;
    }
    return max(sum, vec3<f32>(0.0));
}

// Direction of `v`, or zero for a vertex lying on the shading point.
fn safe_normalize(v: vec3<f32>) -> vec3<f32> {
    let length2 = dot(v, v);
    if length2 < 1e-20 {
        return vec3<f32>(0.0);
    }
    return v / sqrt(length2);
}

// Irradiance per unit radiance from one triangle, given its vertices
// relative to the shading point. The triangle is clipped to the hemisphere
// above `n` (Sutherland-Hodgman), then Lambert's edge integral gives
// E / L = |sum(theta_i * dot(normalize(v_i x v_i+1), n))| / 2. This is LTC
// with the identity matrix; specular uses the GGX matrices in ltc.wgsl.
fn triangle_irradiance(a: vec3<f32>, b: vec3<f32>, c: vec3<f32>, n: vec3<f32>) -> f32 {
    var corners = array<vec3<f32>, 3>(a, b, c);
    var clipped: array<vec3<f32>, 4>;
    var count = 0u;
    for (var i = 0u; i < 3u; i += 1u) {
        let start = corners[i];
        let end = corners[(i + 1u) % 3u];
        let start_height = dot(start, n);
        let end_height = dot(end, n);
        if start_height >= 0.0 {
            clipped[count] = start;
            count += 1u;
        }
        if (start_height >= 0.0) != (end_height >= 0.0) {
            clipped[count] = mix(start, end, start_height / (start_height - end_height));
            count += 1u;
        }
    }
    if count < 3u {
        return 0.0;
    }
    var sum = 0.0;
    for (var i = 0u; i < count; i += 1u) {
        let from_dir = safe_normalize(clipped[i]);
        let to_dir = safe_normalize(clipped[(i + 1u) % count]);
        let edge_normal = cross(from_dir, to_dir);
        let edge_length = length(edge_normal);
        if edge_length > 1e-7 {
            let angle = acos(clamp(dot(from_dir, to_dir), -1.0, 1.0));
            sum += angle * dot(edge_normal / edge_length, n);
        }
    }
    return abs(sum) * 0.5;
}

// Diffuse area light made of triangles (a mesh, or a disk's fan). pbrt emits
// on the side of each triangle's right-hand normal (b - a) x (c - a) unless
// the light is two-sided, so the side test is per triangle.
fn polygon_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let two_sided = (light.kind.y & 1u) != 0u;
    let first = light.kind.z;
    var irradiance = 0.0;
    for (var t = 0u; t + 2u < light.kind.w; t += 3u) {
        let a = emitter_vertices[first + t].xyz - p;
        let b = emitter_vertices[first + t + 1u].xyz - p;
        let c = emitter_vertices[first + t + 2u].xyz - p;
        // dot(normal, p - vertex) with vertices already relative to p.
        let facing = dot(cross(b - a, c - a), -a);
        if !two_sided && facing <= 0.0 {
            continue;
        }
        irradiance += triangle_irradiance(a, b, c, n);
    }
    return light.color.rgb * irradiance;
}

fn sphere_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    let delta = light.position.xyz - p;
    let distance = length(delta);
    let radius = light.direction.w;
    if distance <= radius {
        // Inside: a two-sided sphere surrounds the point; a one-sided one
        // emits only outward, as pbrt's sphere normals face out.
        return select(vec3<f32>(0.0), PI * light.color.rgb, (light.kind.y & 1u) != 0u);
    }
    let sin_sigma2 = (radius / distance) * (radius / distance);
    let cos_theta = dot(n, delta / distance);
    if cos_theta * cos_theta > sin_sigma2 {
        return PI * light.color.rgb * sin_sigma2 * max(cos_theta, 0.0);
    }
    let sin_theta = sqrt(max(1.0 - cos_theta * cos_theta, 1e-8));
    let x = sqrt(1.0 / sin_sigma2 - 1.0);
    let y = clamp(-x * cos_theta / sin_theta, -1.0, 1.0);
    let s = sin_theta * sqrt(max(1.0 - y * y, 0.0));
    let e = (cos_theta * acos(y) - x * s) * sin_sigma2 + atan(s / x);
    return light.color.rgb * max(e, 0.0);
}

// pbrt-v4's `EqualAreaSphereToSquare` (r4 util/geometry/misc.rs): the
// inverse of environment's equal-area map, with r4's minimax atan
// polynomial, for the goniometric diagram lookup.
// copysign(x, s) via the sign bit (naga has no copysign here): -0.0 keeps
// its sign, exactly as copysign does.
fn copysign_bit(x: f32, sign_of: f32) -> f32 {
    let bits = bitcast<u32>(abs(x)) | (bitcast<u32>(sign_of) & 0x80000000u);
    return bitcast<f32>(bits);
}

fn equal_area_sphere_to_square(d: vec3<f32>) -> vec2<f32> {
    let x = abs(d.x);
    let y = abs(d.y);
    let z = abs(d.z);
    let r = sqrt(max(1.0 - z, 0.0));
    let a = max(x, y);
    let b = select(0.0, min(x, y) / a, a > 0.0);
    var phi = (((((-0.25139097234348353e-1 * b + 0.41903881802916575e-1) * b
        + 0.8817706647753163e-1)
        * b
        - 0.24733373328126894)
        * b
        + 0.6157201789828022e-2)
        * b
        + 0.6362265452740161)
        * b
        + 0.4067585662467885e-5;
    phi = select(phi, 1.0 - phi, x < y);
    var v = phi * r;
    var u = r - v;
    if d.z < 0.0 {
        // r4 swaps u and v, then folds both into the far half.
        let swap = u;
        u = 1.0 - v;
        v = 1.0 - swap;
    }
    u = copysign_bit(u, d.x);
    v = copysign_bit(v, d.y);
    return vec2<f32>(0.5 * (u + 1.0), 0.5 * (v + 1.0));
}

// A goniometric light (r4 lights/goniometric.rs): a point light whose
// intensity samples its luminance diagram by the direction to the surface,
// light space = world_from_light x swapYZ so +y is up in the diagram. The
// frame words at kind.z: three rows of the rotation, the slot and
// resolution in their w components, then the diagram's mean Y. kind.w 0 is
// the uniform fallback (mean folded into color).
fn goniometric_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if light.kind.w == 0u {
        return positional(light.position.xyz, light.color.rgb, p, n);
    }
    let first = light.kind.z;
    // Rows as columns: rotation * v maps world to light space.
    let rotation = transpose(mat3x3<f32>(
        emitter_vertices[first].xyz,
        emitter_vertices[first + 1u].xyz,
        emitter_vertices[first + 2u].xyz,
    ));
    let slot = u32(emitter_vertices[first].w);
    let resolution = vec2<f32>(emitter_vertices[first + 1u].w, emitter_vertices[first + 2u].w);
    // r4's wl: world_to_light of the light-to-surface direction, the
    // negation of the incident wi (surface-to-light).
    // Normalized after the transform (r4 normalizes wl): a scaled
    // world_from_light leaves its inverse scale in `rotation`.
    let w_light = safe_normalize(rotation * (p - light.position.xyz));
    let uv = equal_area_sphere_to_square(w_light);
    let texel = nearest_texel(slot, uv, resolution);
    // r4 reduces the diagram to the mean of its channels before lookup.
    let mean_y = (texel.r + texel.g + texel.b) / 3.0;
    return positional(light.position.xyz, light.color.rgb * mean_y, p, n);
}

// A projection light (r4 lights/projection.rs): a point light whose
// intensity projects its image along +z in light space (light space =
// world_from_light x Scale(1, -1, 1)), a pinhole of the frame words'
// tan(fov/2) and screen bounds. The frame: three rows, the slot and
// resolution, the bounds [minx, miny, maxx, maxy], [tan, A], the mean rgb.
// Without a slot (kind.w 0) the mean rgb rides color as a uniform.
fn projection_light(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    if light.kind.w == 0u {
        return positional(light.position.xyz, light.color.rgb, p, n);
    }
    let first = light.kind.z;
    let rotation = transpose(mat3x3<f32>(
        emitter_vertices[first].xyz,
        emitter_vertices[first + 1u].xyz,
        emitter_vertices[first + 2u].xyz,
    ));
    let slot = u32(emitter_vertices[first].w);
    let resolution = vec2<f32>(emitter_vertices[first + 1u].w, emitter_vertices[first + 2u].w);
    let bounds = emitter_vertices[first + 3u];
    let tan_half_fov = emitter_vertices[first + 4u].x;
    // r4's wl: world_to_light of the light-to-surface direction (the
    // negated incident wi). The pinhole divides by wl.z.
    let w_light = rotation * safe_normalize(p - light.position.xyz);
    if w_light.z < 1e-3 {
        return vec3<f32>(0.0);
    }
    let ps = w_light.xy / (w_light.z * tan_half_fov);
    if ps.x < bounds.x || ps.x > bounds.z || ps.y < bounds.y || ps.y > bounds.w {
        return vec3<f32>(0.0);
    }
    // Screen point relative to the bounds, r4's screenBounds.Offset.
    let uv = (ps - bounds.xy) / (bounds.zw - bounds.xy);
    let texel = nearest_texel(slot, uv, resolution);
    // I = scale * the image's color, negative channels clamped zero (r4).
    let rgb = max(texel.rgb, vec3<f32>(0.0));
    return positional(light.position.xyz, light.color.rgb * rgb, p, n);
}

fn light_irradiance(light: Light, p: vec3<f32>, n: vec3<f32>) -> vec3<f32> {
    switch light.kind.x {
        case LIGHT_POINT: { return point_light(light, p, n); }
        case LIGHT_SPOT: { return spot_light(light, p, n); }
        case LIGHT_DISTANT: { return distant_light(light, n); }
        case LIGHT_INFINITE: { return infinite_light(light); }
        case LIGHT_ENVIRONMENT: { return environment_irradiance(n); }
        case LIGHT_POLYGON: { return polygon_light(light, p, n); }
        case LIGHT_SPHERE: { return sphere_light(light, p, n); }
        case LIGHT_GONIOMETRIC: { return goniometric_light(light, p, n); }
        case LIGHT_PROJECTION: { return projection_light(light, p, n); }
        default: { return vec3<f32>(0.0); }
    }
}
