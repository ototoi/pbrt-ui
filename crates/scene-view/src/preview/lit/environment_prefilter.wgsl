// Prefilters the baked environment for GGX: a mip chain of the radiance for
// filtered importance sampling, then one cube level per roughness.
struct Level {
    forward: vec4<f32>,
    // Per unit of s and t across the face, with t growing down the image.
    s: vec4<f32>,
    t: vec4<f32>,
    // x: GGX alpha; y: target face size; z: the source's level 0 face size.
    params: vec4<f32>,
};
@group(0) @binding(0) var<uniform> level: Level;
@group(0) @binding(1) var source: texture_cube<f32>;
@group(0) @binding(2) var source_sampler: sampler;

const PI_FILTER: f32 = 3.141592653589793;
const SAMPLES: u32 = 256u;

@vertex fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[index], 0.0, 1.0);
}

fn direction(position: vec2<f32>) -> vec3<f32> {
    let at = 2.0 * position / level.params.y - 1.0;
    return normalize(level.forward.xyz + at.x * level.s.xyz + at.y * level.t.xyz);
}

// The next level of the source chain: the view holds only the level above.
@fragment fn fs_downsample(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSampleLevel(source, source_sampler, direction(position.xy), 0.0).rgb, 1.0);
}

// With the view along the reflection, as the split-sum approximation assumes.
@fragment fn fs_prefilter(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let r = direction(position.xy);
    let alpha = level.params.x;
    if alpha <= 0.0 {
        return vec4<f32>(textureSampleLevel(source, source_sampler, r, 0.0).rgb, 1.0);
    }
    let up = select(vec3<f32>(0.0, 0.0, 1.0), vec3<f32>(1.0, 0.0, 0.0), abs(r.z) > 0.9);
    let tangent = normalize(cross(up, r));
    let bitangent = cross(r, tangent);
    let alpha2 = alpha * alpha;
    let texel_solid_angle = 4.0 * PI_FILTER / (6.0 * level.params.z * level.params.z);
    var sum = vec3<f32>(0.0);
    var total = 0.0;
    for (var i = 0u; i < SAMPLES; i = i + 1u) {
        let xi = vec2<f32>(f32(i) / f32(SAMPLES), f32(reverseBits(i)) * 2.3283064365386963e-10);
        let phi = 2.0 * PI_FILTER * xi.x;
        let cos_h = sqrt((1.0 - xi.y) / (1.0 + (alpha2 - 1.0) * xi.y));
        let sin_h = sqrt(max(1.0 - cos_h * cos_h, 0.0));
        let h = cos(phi) * sin_h * tangent + sin(phi) * sin_h * bitangent + cos_h * r;
        let l = 2.0 * dot(r, h) * h - r;
        let n_dot_l = dot(r, l);
        if n_dot_l <= 0.0 {
            continue;
        }
        // Read a level whose texels match this sample's share of the lobe, so
        // that a small bright sun does not turn into fireflies.
        let denominator = cos_h * cos_h * (alpha2 - 1.0) + 1.0;
        let pdf = alpha2 / (PI_FILTER * denominator * denominator) * 0.25;
        let sample_solid_angle = 1.0 / (f32(SAMPLES) * max(pdf, 1e-6));
        let lod = max(0.5 * log2(sample_solid_angle / texel_solid_angle) + 1.0, 0.0);
        sum += textureSampleLevel(source, source_sampler, l, lod).rgb * n_dot_l;
        total += n_dot_l;
    }
    return vec4<f32>(sum / max(total, 1e-6), 1.0);
}
