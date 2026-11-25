// Shadow lookups for Lit. See `lit/shadow.rs` for the layout. Points arrive
// eye-relative and move into the shadow frame with `view.shadow_origin`.

// Poisson disk on the unit disk, for blocker search and PCF.
const POISSON = array<vec2<f32>, 16>(
    vec2<f32>(-0.94201624, -0.39906216), vec2<f32>(0.94558609, -0.76890725),
    vec2<f32>(-0.09418410, -0.92938870), vec2<f32>(0.34495938, 0.29387760),
    vec2<f32>(-0.91588581, 0.45771432), vec2<f32>(-0.81544232, -0.87912464),
    vec2<f32>(-0.38277543, 0.27676845), vec2<f32>(0.97484398, 0.75648379),
    vec2<f32>(0.44323325, -0.97511554), vec2<f32>(0.53742981, -0.47373420),
    vec2<f32>(-0.26496911, -0.41893023), vec2<f32>(0.79197514, 0.19090188),
    vec2<f32>(-0.24188840, 0.99706507), vec2<f32>(-0.81409955, 0.91437590),
    vec2<f32>(0.19984126, 0.78641367), vec2<f32>(0.14383161, -0.14100790),
);
// Widest PCSS kernel, in texels.
const MAX_KERNEL_TEXELS: f32 = 24.0;

// Where a shadow-frame point falls in a shadow map.
struct ShadowSample {
    uv: vec2<f32>,
    layer: i32,
    // Linear depth of the point, comparable with the stored depth.
    depth: f32,
    // World size of one texel at the point.
    texel: f32,
};

// Cube face of a direction: +x, -x, +y, -y, +z, -z.
fn cube_face(d: vec3<f32>) -> u32 {
    let a = abs(d);
    if a.x >= a.y && a.x >= a.z {
        return select(1u, 0u, d.x >= 0.0);
    }
    if a.y >= a.z {
        return select(3u, 2u, d.y >= 0.0);
    }
    return select(5u, 4u, d.z >= 0.0);
}

fn project(m: mat4x4<f32>, p: vec3<f32>) -> vec3<f32> {
    let clip = m * vec4<f32>(p, 1.0);
    return clip.xyz / clip.w;
}

fn ndc_to_uv(ndc: vec2<f32>) -> vec2<f32> {
    return ndc * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
}

fn shadow_sample(entry: u32, p: vec3<f32>) -> ShadowSample {
    var s: ShadowSample;
    let size = f32(SHADOW_SIZE);
    if shadow_entries[entry].params.z == SHADOW_DISTANT {
        let ndc = project(shadow_entries[entry].matrices[0], p);
        s.uv = ndc_to_uv(ndc.xy);
        s.layer = i32(shadow_entries[entry].params.x);
        s.depth = ndc.z;
        // The orthographic view spans 2 * radius = depth range.
        s.texel = shadow_entries[entry].position_far.w / size;
        return s;
    }
    let d = p - shadow_entries[entry].position_far.xyz;
    let face = cube_face(d);
    let distance_to_light = length(d);
    s.uv = ndc_to_uv(project(shadow_entries[entry].matrices[face], p).xy);
    s.layer = i32(shadow_entries[entry].params.x + face);
    s.depth = distance_to_light / shadow_entries[entry].position_far.w;
    // A 90-degree face is 2 * distance wide at that distance.
    s.texel = 2.0 * distance_to_light / size;
    return s;
}

// Two unit vectors spanning the plane with normal `n`.
fn tangent_basis(n: vec3<f32>) -> mat2x3<f32> {
    let helper = select(vec3<f32>(1.0, 0.0, 0.0), vec3<f32>(0.0, 1.0, 0.0), abs(n.x) > 0.9);
    let t = normalize(cross(helper, n));
    return mat2x3<f32>(t, cross(n, t));
}

// Depth bias of half a texel, in the map's normalized depth.
fn depth_bias(entry: u32, s: ShadowSample) -> f32 {
    return 0.5 * s.texel / shadow_entries[entry].position_far.w;
}

// 1 when the point is no deeper than the stored depth where it projects.
fn lit_at(entry: u32, q: vec3<f32>) -> f32 {
    let s = shadow_sample(entry, q);
    // Beyond the map's range (depth > 1) nothing was rendered to block it.
    if s.depth > 1.0 {
        return 1.0;
    }
    let half_texel = 0.5 / f32(SHADOW_SIZE);
    let uv = clamp(s.uv, vec2<f32>(half_texel), vec2<f32>(1.0 - half_texel));
    return textureSampleCompareLevel(shadow_maps, shadow_sampler, uv, s.layer, s.depth - depth_bias(entry, s));
}

// Distance from the light to the nearest surface where `q` projects, or a
// negative value when nothing lies in front of `q`.
fn blocker_at(entry: u32, q: vec3<f32>) -> f32 {
    let s = shadow_sample(entry, q);
    let size = i32(SHADOW_SIZE);
    let texel = clamp(vec2<i32>(s.uv * f32(SHADOW_SIZE)), vec2<i32>(0), vec2<i32>(size - 1));
    let stored = textureLoad(shadow_maps, texel, s.layer, 0);
    if stored < s.depth - depth_bias(entry, s) {
        return stored * shadow_entries[entry].position_far.w;
    }
    return -1.0;
}

// PCF over 16 Poisson taps spread `radius` world units across the
// receiving surface. Each tap is projected on its own, so its reference
// depth follows the surface (no acne on slanted receivers) and taps may
// cross cube faces.
fn pcf(entry: u32, p: vec3<f32>, n: vec3<f32>, radius: f32) -> f32 {
    let basis = tangent_basis(n);
    var lit = 0.0;
    for (var i = 0u; i < 16u; i += 1u) {
        lit += lit_at(entry, p + basis * POISSON[i] * radius);
    }
    return lit / 16.0;
}

// PCSS: find the average blocker within the light's footprint, then widen
// the PCF by light_radius * (receiver - blocker) / blocker.
fn pcss(entry: u32, p: vec3<f32>, n: vec3<f32>, texel: f32) -> f32 {
    let light_radius = shadow_entries[entry].size.x;
    let receiver = distance(p, shadow_entries[entry].position_far.xyz);
    let basis = tangent_basis(n);
    var blockers = 0.0;
    var blocker_sum = 0.0;
    for (var i = 0u; i < 16u; i += 1u) {
        let blocker = blocker_at(entry, p + basis * POISSON[i] * light_radius);
        if blocker >= 0.0 {
            blocker_sum += blocker;
            blockers += 1.0;
        }
    }
    if blockers == 0.0 {
        return 1.0;
    }
    let blocker = max(blocker_sum / blockers, 1e-6);
    let penumbra = light_radius * max(receiver - blocker, 0.0) / blocker;
    return pcf(entry, p, n, clamp(penumbra, texel, MAX_KERNEL_TEXELS * texel));
}

// Fraction of light `index` reaching the eye-relative point `p` with normal
// `n`; 1 for lights without a shadow map.
fn shadow_visibility(index: u32, p: vec3<f32>, n: vec3<f32>) -> f32 {
    // Read fields through the index, never copy the struct: on DX12 a
    // whole-struct copy corrupts its matrix array (naga row_major arrays).
    let entry = index;
    if shadow_entries[entry].params.z == SHADOW_NONE {
        return 1.0;
    }
    let point = p + view.shadow_origin.xyz;
    let texel = shadow_sample(entry, point).texel;
    // Normal offset of 1.5 texels keeps surfaces from shadowing themselves.
    let offset_point = point + n * (1.5 * texel);
    if shadow_entries[entry].size.x > 0.0 {
        return pcss(entry, offset_point, n, texel);
    }
    return pcf(entry, offset_point, n, texel);
}
