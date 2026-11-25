// GGX specular (Heitz et al. 2016, "Real-Time Polygonal-Light Shading with
// Linearly Transformed Cosines"). Area lights integrate the clipped polygon
// in LTC space with `triangle_irradiance` from lights.wgsl; point, spot and
// distant lights evaluate the GGX BRDF directly.

// Bilinear fetch from the LTC table (textureLoad: Rgba32Float may not be
// filterable).
fn ltc_fetch(layer: i32, uv: vec2<f32>) -> vec4<f32> {
    let last = f32(LTC_SIZE - 1u);
    let position = clamp(uv, vec2<f32>(0.0), vec2<f32>(1.0)) * last;
    let base = vec2<i32>(floor(position));
    let next = min(base + vec2<i32>(1), vec2<i32>(i32(LTC_SIZE) - 1));
    let t = position - vec2<f32>(base);
    let a = textureLoad(ltc_table, base, layer, 0);
    let b = textureLoad(ltc_table, vec2<i32>(next.x, base.y), layer, 0);
    let c = textureLoad(ltc_table, vec2<i32>(base.x, next.y), layer, 0);
    let d = textureLoad(ltc_table, next, layer, 0);
    return mix(mix(a, b, t.x), mix(c, d, t.x), t.y);
}

// A GGX lobe for one shading point: the inverse LTC matrix (including the
// view-aligned basis) and the table's magnitude and Fresnel weights.
struct LtcLobe {
    minv: mat3x3<f32>,
    magnitude: f32,
    fresnel: f32,
    alpha: f32,
};

fn ltc_lobe(alpha: f32, n: vec3<f32>, v: vec3<f32>) -> LtcLobe {
    let cos_theta = clamp(dot(n, v), 0.0, 1.0);
    let uv = vec2<f32>(sqrt(clamp(alpha, 0.0, 1.0)), sqrt(1.0 - cos_theta));
    let packed = ltc_fetch(0, uv);
    let terms = ltc_fetch(1, uv);
    let table = mat3x3<f32>(
        vec3<f32>(packed.x, 0.0, packed.y),
        vec3<f32>(0.0, 1.0, 0.0),
        vec3<f32>(packed.z, 0.0, packed.w),
    );
    // Basis with the view in the x-z plane, as the table was fitted.
    var t1 = v - n * dot(v, n);
    if dot(t1, t1) < 1e-12 {
        t1 = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
        t1 = t1 - n * dot(t1, n);
    }
    t1 = normalize(t1);
    let t2 = cross(n, t1);
    var lobe: LtcLobe;
    lobe.minv = table * transpose(mat3x3<f32>(t1, t2, n));
    lobe.magnitude = terms.x;
    lobe.fresnel = terms.y;
    lobe.alpha = alpha;
    return lobe;
}

// Directional albedo of the lobe for normal-incidence reflectance f0
// (F90 = 1): what a uniform environment of radiance 1 reflects.
fn ltc_albedo(lobe: LtcLobe, f0: vec3<f32>) -> vec3<f32> {
    return f0 * lobe.magnitude + (vec3<f32>(1.0) - f0) * lobe.fresnel;
}

// Form factor (E / (pi L)) of a world triangle, relative to the shading
// point, under the lobe's transformed cosine.
fn ltc_triangle(lobe: LtcLobe, a: vec3<f32>, b: vec3<f32>, c: vec3<f32>) -> f32 {
    let up = vec3<f32>(0.0, 0.0, 1.0);
    return triangle_irradiance(lobe.minv * a, lobe.minv * b, lobe.minv * c, up) / PI;
}

fn ltc_polygon_light(light: Light, lobe: LtcLobe, p: vec3<f32>) -> f32 {
    let two_sided = (light.kind.y & 1u) != 0u;
    let first = light.kind.z;
    var sum = 0.0;
    for (var t = 0u; t + 2u < light.kind.w; t += 3u) {
        let a = emitter_vertices[first + t].xyz - p;
        let b = emitter_vertices[first + t + 1u].xyz - p;
        let c = emitter_vertices[first + t + 2u].xyz - p;
        if !two_sided && dot(cross(b - a, c - a), -a) <= 0.0 {
            continue;
        }
        sum += ltc_triangle(lobe, a, b, c);
    }
    return sum;
}

// A sphere seen from p as the disk facing p with the same solid angle,
// as a 16-gon fan.
fn ltc_sphere_light(light: Light, lobe: LtcLobe, p: vec3<f32>) -> f32 {
    let to_center = light.position.xyz - p;
    let distance = length(to_center);
    let radius = light.direction.w;
    if distance <= radius {
        // Inside: a two-sided sphere fills the whole hemisphere (the lobe
        // integrates to 1); a one-sided one emits only outward.
        return select(0.0, 1.0, (light.kind.y & 1u) != 0u);
    }
    let w = to_center / distance;
    let disk_radius = radius * distance / sqrt(distance * distance - radius * radius);
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(w.x) > 0.9);
    let u = normalize(cross(helper, w)) * disk_radius;
    let v = cross(w, normalize(u)) * disk_radius;
    let first = to_center + u;
    var sum = 0.0;
    var previous = to_center + u * cos(2.0 * PI / 16.0) + v * sin(2.0 * PI / 16.0);
    for (var i = 2u; i < 16u; i += 1u) {
        let angle = 2.0 * PI * f32(i) / 16.0;
        let current = to_center + u * cos(angle) + v * sin(angle);
        sum += ltc_triangle(lobe, first, previous, current);
        previous = current;
    }
    return sum;
}

// Share of the lobe inside the cone of half-angle acos(cos_cone) about n:
// the light a layer under a coat of that critical angle can send out (the
// rest is totally reflected back in). The cone's rim, as a 16-gon of
// directions, is integrated like an area light.
fn ltc_cone_fraction(lobe: LtcLobe, n: vec3<f32>, cos_cone: f32) -> f32 {
    let sin_cone = sqrt(max(1.0 - cos_cone * cos_cone, 0.0));
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let u = normalize(cross(helper, n)) * sin_cone;
    let w = cross(n, normalize(u)) * sin_cone;
    let center = n * cos_cone;
    let first = center + u;
    var sum = 0.0;
    var previous = center + u * cos(2.0 * PI / 16.0) + w * sin(2.0 * PI / 16.0);
    for (var i = 2u; i < 16u; i += 1u) {
        let angle = 2.0 * PI * f32(i) / 16.0;
        let current = center + u * cos(angle) + w * sin(angle);
        sum += ltc_triangle(lobe, first, previous, current);
        previous = current;
    }
    return clamp(sum, 0.0, 1.0);
}

// pbrt-v4's Trowbridge-Reitz (GGX) BRDF with the height-correlated Smith
// term and Schlick Fresnel from f0.
fn ggx_brdf(n: vec3<f32>, v: vec3<f32>, l: vec3<f32>, alpha: f32, f0: vec3<f32>) -> vec3<f32> {
    let cos_l = dot(n, l);
    let cos_v = dot(n, v);
    if cos_l <= 0.0 || cos_v <= 0.0 {
        return vec3<f32>(0.0);
    }
    let h = normalize(v + l);
    let cos_h = max(dot(n, h), 0.0);
    let a2 = max(alpha * alpha, 1e-8);
    // (1 − cos²) + α²cos² rather than cos²(α² − 1) + 1, which cancels to
    // 0 in f32 for a smooth lobe at the exact specular direction.
    let denominator = max((1.0 - cos_h * cos_h) + a2 * cos_h * cos_h, 1e-12);
    let d = a2 / (PI * denominator * denominator);
    let g = 1.0 / (1.0 + smith_lambda(cos_v, a2) + smith_lambda(cos_l, a2));
    let f = f0 + (vec3<f32>(1.0) - f0) * pow(1.0 - clamp(dot(v, h), 0.0, 1.0), 5.0);
    return f * (d * g / (4.0 * cos_l * cos_v));
}

fn smith_lambda(cos_theta: f32, a2: f32) -> f32 {
    let c2 = max(cos_theta * cos_theta, 1e-8);
    return (-1.0 + sqrt(1.0 + a2 * (1.0 - c2) / c2)) * 0.5;
}

// Reflected radiance of one light through a GGX lobe with reflectance f0.
fn light_specular(light: Light, lobe: LtcLobe, f0: vec3<f32>, p: vec3<f32>, n: vec3<f32>, v: vec3<f32>) -> vec3<f32> {
    switch light.kind.x {
        case LIGHT_POINT, LIGHT_SPOT, LIGHT_GONIOMETRIC, LIGHT_PROJECTION: {
            let l = normalize(light.position.xyz - p);
            return ggx_brdf(n, v, l, lobe.alpha, f0) * light_irradiance(light, p, n);
        }
        case LIGHT_DISTANT: {
            return ggx_brdf(n, v, light.direction.xyz, lobe.alpha, f0) * light_irradiance(light, p, n);
        }
        case LIGHT_ENVIRONMENT: {
            // Split sum: the prefiltered radiance times the directional albedo.
            let level = sqrt(clamp(lobe.alpha, 0.0, 1.0)) * f32(ENVIRONMENT_LEVELS - 1u);
            let radiance = textureSampleLevel(environment_cube, environment_sampler, reflect(-v, n), level).rgb;
            return radiance * ltc_albedo(lobe, f0);
        }
        case LIGHT_INFINITE: {
            return light.color.rgb * ltc_albedo(lobe, f0);
        }
        case LIGHT_POLYGON: {
            return light.color.rgb * ltc_polygon_light(light, lobe, p) * ltc_albedo(lobe, f0);
        }
        case LIGHT_SPHERE: {
            return light.color.rgb * ltc_sphere_light(light, lobe, p) * ltc_albedo(lobe, f0);
        }
        default: { return vec3<f32>(0.0); }
    }
}
