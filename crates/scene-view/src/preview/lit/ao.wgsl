// Lit screen-space ambient occlusion: GTAO (Jimenez et al. 2016) from the
// PreZ depth and normals, then a 4×4 denoise that stays on the surface.
// Output is R8Unorm, 1 = unoccluded; the background stays 1. Positions are
// eye-relative on world axes, like the Lit pass.
struct View {
    projection: mat4x4<f32>,
    mode: vec4<u32>,
    exposure: vec4<f32>,
    background: vec4<f32>,
    shadow_origin: vec4<f32>,
    // A pixel's view ray: forward + ndc.x * right + ndc.y * up.
    ray_forward: vec4<f32>,
    ray_right: vec4<f32>,
    ray_up: vec4<f32>,
    // x: AO radius in world units (0 = no AO).
    ao: vec4<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var depth_map: texture_depth_2d;
@group(0) @binding(2) var normal_map: texture_2d<f32>;
// The denoise pass's input: the raw GTAO.
@group(0) @binding(3) var ao_in: texture_2d<f32>;

const AO_PI: f32 = 3.141592653589793;
const AO_SLICES: u32 = 8u;
const AO_STEPS: u32 = 8u;

struct AoOut {
    @builtin(position) position: vec4<f32>,
};

// One triangle covering the viewport.
@vertex fn vs_ao(@builtin(vertex_index) index: u32) -> AoOut {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: AoOut;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

// Distance along the view axis: reversed Z stores depth = a + b / z.
fn view_z(depth: f32) -> f32 {
    let a = dot(vec3<f32>(view.projection[0].z, view.projection[1].z, view.projection[2].z), view.ray_forward.xyz);
    return view.projection[3].z / (depth - a);
}

// The eye-relative point a pixel center shows at `depth`.
fn ao_position(p: vec2<i32>, depth: f32) -> vec3<f32> {
    let size = vec2<f32>(textureDimensions(depth_map));
    let ndc = vec2<f32>((f32(p.x) + 0.5) / size.x * 2.0 - 1.0, 1.0 - (f32(p.y) + 0.5) / size.y * 2.0);
    return view_z(depth) * (view.ray_forward.xyz + ndc.x * view.ray_right.xyz + ndc.y * view.ray_up.xyz);
}

fn ao_normal(p: vec2<i32>) -> vec3<f32> {
    return normalize(textureLoad(normal_map, p, 0).xyz * 2.0 - 1.0);
}

// A 4×4 Bayer value in [0, 1): the denoise's 4×4 window holds each of the
// 16 values once, so the slices' rotations average out on flat surfaces
// (interleaved gradient noise left stripes through a 5×5 window).
fn ao_noise(p: vec2<u32>) -> f32 {
    let x = p.x & 3u;
    let y = p.y & 3u;
    let xy = x ^ y;
    // Bit-reverse interleave of (x ^ y, y).
    let b = ((xy & 1u) << 3u) | ((y & 1u) << 2u) | ((xy & 2u) >> 0u) | ((y & 2u) >> 1u);
    return (f32(b) + 0.5) / 16.0;
}

// A per-pixel random value in [0, 1) (PCG hash).
fn ao_hash(p: vec2<u32>, n: u32) -> f32 {
    var v = p.x * 1664525u + p.y * 22695477u + n * 2891336453u + 1013904223u;
    v = v * 747796405u + 2891336453u;
    v = ((v >> ((v >> 28u) + 4u)) ^ v) * 277803737u;
    v = (v >> 22u) ^ v;
    return f32(v >> 8u) / 16777216.0;
}

// The cosine-weighted visible arc of one side of a slice, horizon angle `h`
// from the view vector, projected normal at angle `n`.
fn ao_arc(h: f32, n: f32) -> f32 {
    return 0.25 * (-cos(2.0 * h - n) + cos(n) + 2.0 * h * sin(n));
}

@fragment fn fs_gtao(i: AoOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(i.position.xy);
    let size = vec2<i32>(textureDimensions(depth_map));
    let depth = textureLoad(depth_map, p, 0);
    let radius = view.ao.x;
    if depth <= 0.0 || radius <= 0.0 {
        return vec4<f32>(1.0);
    }
    let position = ao_position(p, depth);
    let normal = ao_normal(p);
    let to_eye = normalize(-position);
    // The radius in pixels at this depth.
    let pixels = radius / view_z(depth) * 0.5 * f32(size.y) / length(view.ray_up.xyz);
    if pixels < 1.0 {
        return vec4<f32>(1.0);
    }
    let right = normalize(view.ray_right.xyz);
    let up = normalize(view.ray_up.xyz);
    let cell = vec2<u32>(i.position.xy);
    let rotation = ao_noise(cell);
    // The steps' jitter, decorrelated from the rotation within the 4×4 cell.
    let jitter = ao_noise(cell.yx + vec2<u32>(1u, 2u));
    // Visible over unoccluded arcs, so a surface with no occluder gives
    // exactly 1 whatever the slices' discretization.
    var visible = 0.0;
    var open = 0.0;
    for (var slice = 0u; slice < AO_SLICES; slice++) {
        let phi = (f32(slice) + rotation) / f32(AO_SLICES) * AO_PI;
        // Screen direction, y down.
        let direction = vec2<f32>(cos(phi), sin(phi));
        let along = normalize(direction.x * right - direction.y * up);
        let ortho = normalize(along - dot(along, to_eye) * to_eye);
        let axis = cross(ortho, to_eye);
        let projected = normal - axis * dot(normal, axis);
        let length_n = length(projected);
        if length_n < 1e-4 {
            continue;
        }
        let cos_n = clamp(dot(projected, to_eye) / length_n, -1.0, 1.0);
        let n = sign(dot(ortho, projected)) * acos(cos_n);
        // Side 0 looks along +direction (angles toward `ortho` are positive).
        var horizon = array<f32, 2>(cos(n + 0.5 * AO_PI), cos(n - 0.5 * AO_PI));
        var low = horizon;
        for (var side = 0; side < 2; side++) {
            let step_direction = select(-direction, direction, side == 0);
            for (var step = 0u; step < AO_STEPS; step++) {
                let t = (f32(step) + jitter) / f32(AO_STEPS);
                // A random sub-pixel shift across the slice, so rounding does
                // not keep neighbors' samples on the same rows and columns
                // (vertical streaks on grazing floors). Random steps along it
                // mottled the image instead.
                let across = vec2<f32>(-step_direction.y, step_direction.x)
                    * (ao_hash(cell, slice * 64u + u32(side) * 32u + step) - 0.5);
                let offset = step_direction * mix(1.0, pixels, t * t) + across;
                let q = p + vec2<i32>(round(offset));
                if any(q < vec2<i32>(0)) || any(q >= size) {
                    break;
                }
                let sample_depth = textureLoad(depth_map, q, 0);
                if sample_depth <= 0.0 {
                    continue;
                }
                let delta = ao_position(q, sample_depth) - position;
                let distance = length(delta);
                if distance < 1e-6 * radius {
                    continue;
                }
                // Full weight inside 60% of the radius, none beyond it.
                // Samples within 6° of the tangent plane (facets of a
                // tessellated curve around the interpolated normal) do not
                // occlude, fading in by 12°.
                let elevation = dot(delta / distance, normal);
                let weight = clamp((radius - distance) / (0.4 * radius), 0.0, 1.0)
                    * clamp((elevation - 0.1) / 0.1, 0.0, 1.0);
                let cos_h = mix(low[side], dot(delta / distance, to_eye), weight);
                horizon[side] = max(horizon[side], cos_h);
            }
        }
        let h1 = n + clamp(acos(horizon[0]) - n, -0.5 * AO_PI, 0.5 * AO_PI);
        let h0 = n + clamp(-acos(horizon[1]) - n, -0.5 * AO_PI, 0.5 * AO_PI);
        visible += length_n * (ao_arc(h0, n) + ao_arc(h1, n));
        open += length_n * (ao_arc(n - 0.5 * AO_PI, n) + ao_arc(n + 0.5 * AO_PI, n));
    }
    if open <= 0.0 {
        return vec4<f32>(1.0);
    }
    return vec4<f32>(clamp(visible / open, 0.0, 1.0));
}

// Averages the 4×4 neighbors that lie on the same surface: near the
// pixel's tangent plane and facing the same way.
@fragment fn fs_ao_denoise(i: AoOut) -> @location(0) vec4<f32> {
    let p = vec2<i32>(i.position.xy);
    let size = vec2<i32>(textureDimensions(depth_map));
    let depth = textureLoad(depth_map, p, 0);
    let radius = view.ao.x;
    if depth <= 0.0 || radius <= 0.0 {
        return vec4<f32>(1.0);
    }
    let position = ao_position(p, depth);
    let normal = ao_normal(p);
    var sum = 0.0;
    var total = 0.0;
    for (var y = -2; y <= 1; y++) {
        for (var x = -2; x <= 1; x++) {
            let q = p + vec2<i32>(x, y);
            if any(q < vec2<i32>(0)) || any(q >= size) {
                continue;
            }
            let sample_depth = textureLoad(depth_map, q, 0);
            if sample_depth <= 0.0 {
                continue;
            }
            let plane = abs(dot(ao_position(q, sample_depth) - position, normal)) / (0.05 * radius);
            let facing = max(dot(ao_normal(q), normal), 0.0);
            let weight = max(1.0 - plane, 0.0) * facing * facing;
            sum += weight * textureLoad(ao_in, q, 0).x;
            total += weight;
        }
    }
    return vec4<f32>(sum / max(total, 1e-6));
}
