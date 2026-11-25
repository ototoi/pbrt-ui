// Texture VM: postfix programs from `lit/texture_vm.rs`, shared by the Lit
// shader and the shadow pass (alpha cutouts).
const PI_VM: f32 = 3.141592653589793;
const TEX_FLOAT: u32 = 0u;
const TEX_SPECTRUM: u32 = 1u;
const TEX_IMAGE: u32 = 2u;
const TEX_SCALE: u32 = 3u;
const TEX_MIX: u32 = 4u;

const TEX_BILERP: u32 = 5u;
const TEX_CHECKERBOARD: u32 = 6u;
const TEX_DOTS: u32 = 7u;
const TEX_DIRECTIONMIX: u32 = 8u;
const TEX_UV: u32 = 9u;
const TEX_MAP: u32 = 10u;
const TEX_CHECKERBOARD3D: u32 = 11u;
const TEX_FBM: u32 = 12u;
const TEX_WRINKLED: u32 = 13u;
const TEX_WINDY: u32 = 14u;
const TEX_MARBLE: u32 = 15u;

const MAP_PLANAR: u32 = 0u;
const MAP_SPHERICAL: u32 = 1u;
const MAP_CYLINDRICAL: u32 = 2u;
const MAP_POINT: u32 = 3u;

@group(1) @binding(0) var images: texture_2d_array<f32>;
@group(1) @binding(1) var image_sampler: sampler;
@group(1) @binding(2) var repeat_sampler: sampler;

struct TextureInstruction {
    op: u32,
    a: u32,
    b: u32,
    c: u32,
    value: vec4<f32>,
};
@group(2) @binding(7) var<storage, read> texture_instructions: array<TextureInstruction>;
// r4's noise permutation, doubled so index + 1 + y stays in range.
@group(2) @binding(8) var<storage, read> noise_perm: array<u32>;
// Three rows per mapping, from the frame's origin (`LitScene::mapping_rows`).
@group(2) @binding(9) var<storage, read> texture_mappings: array<vec4<f32>>;

// What a program may read at this pixel. The derivatives are taken before
// any branch or discard.
struct TexturePoint {
    uv: vec2<f32>,
    uv_dx: vec2<f32>,
    uv_dy: vec2<f32>,
    // World-space shading normal (directionmix).
    normal: vec3<f32>,
    // Position from the frame's origin, which mappings' rows are relative to.
    position: vec3<f32>,
    position_dx: vec3<f32>,
    position_dy: vec3<f32>,
};

// 2D texture coordinates with their screen derivatives.
struct St {
    st: vec2<f32>,
    dx: vec2<f32>,
    dy: vec2<f32>,
};

struct Solid {
    p: vec3<f32>,
    dx: vec3<f32>,
    dy: vec3<f32>,
};

// Ports of r4 textures/noise.rs: same table, gradients and weights.
fn noise_gradient(x: u32, y: u32, z: u32, d: vec3<f32>) -> f32 {
    let h = noise_perm[noise_perm[noise_perm[x] + y] + z] & 15u;
    let u = select(d.y, d.x, h < 8u || h == 12u || h == 13u);
    let v = select(d.z, d.y, h < 4u || h == 12u || h == 13u);
    return select(u, -u, (h & 1u) != 0u) + select(v, -v, (h & 2u) != 0u);
}

fn noise_weight(t: f32) -> f32 {
    let t3 = t * t * t;
    let t4 = t3 * t;
    return 6.0 * t4 * t - 15.0 * t4 + 10.0 * t3;
}

fn perlin_noise(p: vec3<f32>) -> f32 {
    let wrapped = p % 1073741824.0;
    let cell = floor(wrapped);
    let d = wrapped - cell;
    let i = vec3<u32>(vec3<i32>(cell)) & vec3<u32>(255u);
    let w = vec3<f32>(noise_weight(d.x), noise_weight(d.y), noise_weight(d.z));
    // One call site, as with the helpers below: drivers inline every call, and
    // each extra one multiplies the Lit shader's compile time.
    var corners: array<f32, 8>;
    for (var c = 0u; c < 8u; c = c + 1u) {
        let corner = vec3<u32>(c & 1u, (c >> 1u) & 1u, (c >> 2u) & 1u);
        corners[c] = noise_gradient(i.x + corner.x, i.y + corner.y, i.z + corner.z, d - vec3<f32>(corner));
    }
    let x00 = mix(corners[0], corners[1], w.x);
    let x10 = mix(corners[2], corners[3], w.x);
    let x01 = mix(corners[4], corners[5], w.x);
    let x11 = mix(corners[6], corners[7], w.x);
    return mix(mix(x00, x10, w.y), mix(x01, x11, w.y), w.z);
}

// The instruction's own UV mapping, unless a TEX_MAP just mapped the position.
fn st_of(mapping: vec4<f32>, point: TexturePoint, mapped: bool, map: St) -> St {
    if mapped {
        return map;
    }
    return St(point.uv * mapping.xy + mapping.zw, point.uv_dx * mapping.xy, point.uv_dy * mapping.xy);
}

// Ports of r4 textures/mapping2d.rs; the derivatives are analytic there too,
// so the seams do not show up as huge footprints.
fn spherical_st(p: Solid) -> St {
    let x2y2 = dot(p.p.xy, p.p.xy);
    let root = sqrt(x2y2);
    let dsdp = vec3<f32>(-p.p.y, p.p.x, 0.0) / (2.0 * PI_VM * x2y2);
    let dtdp = vec3<f32>(p.p.x * p.p.z / root, p.p.y * p.p.z / root, -root)
        / (PI_VM * (x2y2 + p.p.z * p.p.z));
    let v = normalize(p.p);
    var phi = atan2(v.y, v.x);
    if phi < 0.0 {
        phi += 2.0 * PI_VM;
    }
    // r4 and pbrt-v4 order these as (phi, theta) against an st of (theta, phi).
    // Their footprints shrink at high sample counts, but the preview's are whole
    // pixels, so here each derivative stays with its own coordinate.
    return St(
        vec2<f32>(acos(clamp(v.z, -1.0, 1.0)) / PI_VM, phi / (2.0 * PI_VM)),
        vec2<f32>(dot(dtdp, p.dx), dot(dsdp, p.dx)),
        vec2<f32>(dot(dtdp, p.dy), dot(dsdp, p.dy)),
    );
}

fn cylindrical_st(p: Solid) -> St {
    let dsdp = vec3<f32>(-p.p.y, p.p.x, 0.0) / (2.0 * PI_VM * dot(p.p.xy, p.p.xy));
    return St(
        vec2<f32>((PI_VM + atan2(p.p.y, p.p.x)) / (2.0 * PI_VM), p.p.z),
        vec2<f32>(dot(dsdp, p.dx), p.dx.z),
        vec2<f32>(dot(dsdp, p.dy), p.dy.z),
    );
}

// r4 noise.rs fbm and turbulence: the footprint limits the octaves, and the
// last partial one fades in.
fn octaves_of(p: Solid, max_octaves: u32) -> f32 {
    let len2 = max(max(dot(p.dx, p.dx), dot(p.dy, p.dy)), 1e-30);
    return clamp(-1.0 - 0.5 * log2(len2), 0.0, f32(max_octaves));
}

fn noise_sum(p: Solid, omega: f32, max_octaves: u32, turbulent: bool) -> f32 {
    let n = octaves_of(p, max_octaves);
    let whole = u32(floor(n));
    var sum = 0.0;
    var lambda = 1.0;
    var o = 1.0;
    for (var i = 0u; i <= whole; i = i + 1u) {
        let noise = perlin_noise(lambda * p.p);
        if i == whole {
            let fade = smoothstep(0.3, 0.7, n - floor(n));
            sum += o * select(fade * noise, mix(0.2, abs(noise), fade), turbulent);
        } else {
            sum += o * select(noise, abs(noise), turbulent);
            lambda *= 1.99;
            o *= omega;
        }
    }
    if turbulent {
        for (var i = whole; i < max_octaves; i = i + 1u) {
            sum += o * 0.2;
            o *= omega;
        }
    }
    return sum;
}

fn scaled_solid(p: Solid, scale: f32) -> Solid {
    return Solid(scale * p.p, scale * p.dx, scale * p.dy);
}

// r4 marble.rs: a cubic Bezier over the segment of a nine-color spline.
fn marble_color(wave: f32) -> vec3<f32> {
    var colors = array<vec3<f32>, 9>(
        vec3<f32>(0.58, 0.58, 0.6),
        vec3<f32>(0.58, 0.58, 0.6),
        vec3<f32>(0.58, 0.58, 0.6),
        vec3<f32>(0.5, 0.5, 0.5),
        vec3<f32>(0.6, 0.59, 0.58),
        vec3<f32>(0.58, 0.58, 0.6),
        vec3<f32>(0.58, 0.58, 0.6),
        vec3<f32>(0.2, 0.2, 0.33),
        vec3<f32>(0.58, 0.58, 0.6),
    );
    let along = (0.5 + 0.5 * sin(wave)) * 6.0;
    let first = min(u32(floor(along)), 5u);
    let t = along - f32(first);
    let s0 = mix(colors[first], colors[first + 1u], t);
    let s1 = mix(colors[first + 1u], colors[first + 2u], t);
    let s2 = mix(colors[first + 2u], colors[first + 3u], t);
    return mix(mix(s0, s1, t), mix(s1, s2, t), t) * 1.5;
}

// fbm, wrinkled, windy and marble, on one noise_sum call site.
fn solid_noise(op: u32, p: Solid, octaves: u32, value: vec4<f32>) -> vec4<f32> {
    if op == TEX_WINDY {
        // abs(fbm(0.1 p, 0.5, 3)) * fbm(p, 0.5, 6)
        var result = 1.0;
        for (var step = 0u; step < 2u; step = step + 1u) {
            let far = step == 0u;
            let sum = noise_sum(
                scaled_solid(p, select(1.0, 0.1, far)), 0.5, select(6u, 3u, far), false,
            );
            result *= select(sum, abs(sum), far);
        }
        return vec4<f32>(result);
    }
    var at = p;
    if op == TEX_MARBLE {
        at = scaled_solid(p, value.y);
    }
    let sum = noise_sum(at, value.x, octaves, op == TEX_WRINKLED);
    if op == TEX_MARBLE {
        return vec4<f32>(marble_color(at.p.y + value.z * sum), 1.0);
    }
    return vec4<f32>(sum);
}

// r4 BumpInt: the integral of the checker's square wave.
fn checker_bump(x: f32) -> f32 {
    let half_x = x / 2.0;
    return floor(half_x) + 2.0 * max(half_x - floor(half_x) - 0.5, 0.0);
}

// Share of tex2 at st. closed_form filters over the pixel footprint with
// r4's closed form; otherwise the cell parity decides.
fn checker_tex2_share(st: vec2<f32>, dst_dx: vec2<f32>, dst_dy: vec2<f32>, closed_form: bool) -> f32 {
    let parity = (i32(floor(st.x)) + i32(floor(st.y))) & 1;
    let picked = select(0.0, 1.0, parity != 0);
    if !closed_form {
        return picked;
    }
    // The floor keeps an axis the footprint does not cross from dividing by 0.
    let footprint = max(max(abs(dst_dx), abs(dst_dy)), vec2<f32>(1e-6));
    let low = st - footprint;
    let high = st + footprint;
    if all(floor(low) == floor(high)) {
        return picked;
    }
    if any(footprint > vec2<f32>(1.0)) {
        return 0.5;
    }
    let s = (checker_bump(high.x) - checker_bump(low.x)) / (2.0 * footprint.x);
    let t = (checker_bump(high.y) - checker_bump(low.y)) / (2.0 * footprint.y);
    return s + t - 2.0 * s * t;
}

// r4 dots: whether st falls inside this cell's jittered dot.
fn inside_dot(st: vec2<f32>) -> bool {
    let cell = floor(st + vec2<f32>(0.5));
    let offsets = array<vec2<f32>, 3>(
        vec2<f32>(0.5, 0.5), vec2<f32>(1.5, 2.8), vec2<f32>(4.5, 9.8),
    );
    var noise: array<f32, 3>;
    for (var k = 0u; k < 3u; k = k + 1u) {
        var offset = offsets;
        noise[k] = perlin_noise(vec3<f32>(cell + offset[k], 0.5));
    }
    if noise[0] <= 0.0 {
        return false;
    }
    let radius = 0.35;
    let max_shift = 0.5 - radius;
    let center = cell + max_shift * vec2<f32>(noise[1], noise[2]);
    let delta = st - center;
    return dot(delta, delta) < radius * radius;
}


// Border texels are black before bilinear filtering, including footprints
// whose centers lie just outside the image.
fn black_image_level(slot: u32, uv: vec2<f32>, level: u32) -> vec4<f32> {
    let size = vec2<i32>(textureDimensions(images, level));
    let p = uv * vec2<f32>(size) - vec2<f32>(0.5);
    let base = vec2<i32>(floor(p));
    let fraction = fract(p);
    var result = vec4<f32>(0.0);
    for (var y = 0; y < 2; y = y + 1) {
        for (var x = 0; x < 2; x = x + 1) {
            let coord = base + vec2<i32>(x, y);
            if all(coord >= vec2<i32>(0)) && all(coord < size) {
                let weight = select(1.0 - fraction.x, fraction.x, x == 1)
                    * select(1.0 - fraction.y, fraction.y, y == 1);
                result += textureLoad(images, coord, i32(slot), i32(level)) * weight;
            }
        }
    }
    return result;
}

fn black_image(slot: u32, uv: vec2<f32>, dx: vec2<f32>, dy: vec2<f32>) -> vec4<f32> {
    let size = vec2<f32>(textureDimensions(images, 0));
    let footprint = max(length(dx * size), length(dy * size));
    let last = f32(textureNumLevels(images) - 1u);
    let lod = clamp(log2(max(footprint, 1.0)), 0.0, last);
    let low = u32(floor(lod));
    let high = min(low + 1u, u32(last));
    return mix(black_image_level(slot, uv, low), black_image_level(slot, uv, high), fract(lod));
}

// Nearest texel of a light control map at r4's uv (r4 `Image::
// lookup_nearest`: floor(uv * resolution), clamped, decoding row 0 as the
// image's top row exactly as the layers upload). `resolution` is the
// light's own, in its frame words: light maps upload native-size in the
// layer's top-left while the array layers stay square.
fn nearest_texel(slot: u32, uv: vec2<f32>, resolution: vec2<f32>) -> vec4<f32> {
    let at = clamp(uv * resolution, vec2<f32>(0.0), resolution - vec2<f32>(1.0));
    let coord = vec2<i32>(floor(at));
    let level = 0;
    return textureLoad(images, coord, i32(slot), i32(level));
}

fn eval_texture(start: u32, count: u32, point: TexturePoint) -> vec3<f32> {
    // What the last TEX_MAP produced, for the instruction right after it.
    var mapped = false;
    var map: St;
    var solid: Solid;
    var stack: array<vec4<f32>, 32>;
    var depth = 0u;
    if count > 256u { return vec3<f32>(0.5); }
    for (var offset = 0u; offset < count; offset = offset + 1u) {
        let instruction = texture_instructions[start + offset];
        switch instruction.op {
            case TEX_FLOAT, TEX_SPECTRUM: {
                if depth >= 32u { return vec3<f32>(0.5); }
                stack[depth] = instruction.value;
                depth = depth + 1u;
            }
            case TEX_IMAGE: {
                if depth >= 32u { return vec3<f32>(0.5); }
                let image_st = st_of(instruction.value, point, mapped, map);
                mapped = false;
                let uv_dx = image_st.dx;
                let uv_dy = image_st.dy;
                var sample_uv = image_st.st;
                // The wrap mode keeps the low byte of `b`; bit 8 marks a
                // single-channel float image whose R the shader widens to
                // RGB (the binding path stores R8/R16 for float uses, and the
                // `node.kind == 0u` rule).
                let wrap = instruction.b & 0xffu;
                let scalar_image = (instruction.b & 0x100u) != 0u;
                if wrap == 0u {
                    sample_uv = fract(sample_uv);
                } else if wrap == 1u {
                    sample_uv = clamp(sample_uv, vec2<f32>(0.0), vec2<f32>(1.0));
                }
                sample_uv.y = 1.0 - sample_uv.y;
                let image_scale = vec2<f32>(1.0, -1.0);
                var texel: vec4<f32>;
                if wrap == 0u {
                    texel = textureSampleGrad(images, repeat_sampler, sample_uv, i32(instruction.a), uv_dx * image_scale, uv_dy * image_scale);
                } else if wrap == 2u {
                    texel = black_image(instruction.a, sample_uv, uv_dx * image_scale, uv_dy * image_scale);
                } else {
                    texel = textureSampleGrad(images, image_sampler, sample_uv, i32(instruction.a), uv_dx * image_scale, uv_dy * image_scale);
                }
                if scalar_image {
                    texel = vec4<f32>(texel.r, texel.r, texel.r, 1.0);
                }
                stack[depth] = texel * bitcast<f32>(instruction.c);
                depth = depth + 1u;
            }
            case TEX_SCALE: {
                if depth < 2u { return vec3<f32>(0.5); }
                let rhs = stack[depth - 1u];
                let lhs = stack[depth - 2u];
                stack[depth - 2u] = lhs * rhs;
                depth = depth - 1u;
            }
            case TEX_MIX: {
                if depth < 3u { return vec3<f32>(0.5); }
                // pbrt mix textures use linear interpolation without
                // clamping, so values outside [0, 1] extrapolate.
                let amount = stack[depth - 1u].x;
                let rhs = stack[depth - 2u];
                let lhs = stack[depth - 3u];
                stack[depth - 3u] = mix(lhs, rhs, amount);
                depth = depth - 2u;
            }
            case TEX_BILERP: {
                if depth < 4u { return vec3<f32>(0.5); }
                let st = st_of(instruction.value, point, mapped, map).st;
                mapped = false;
                let across = mix(stack[depth - 4u], stack[depth - 2u], st.x);
                let far = mix(stack[depth - 3u], stack[depth - 1u], st.x);
                stack[depth - 4u] = mix(across, far, st.y);
                depth = depth - 3u;
            }
            case TEX_CHECKERBOARD: {
                if depth < 2u { return vec3<f32>(0.5); }
                let at = st_of(instruction.value, point, mapped, map);
                mapped = false;
                let share = checker_tex2_share(at.st, at.dx, at.dy, instruction.a != 0u);
                stack[depth - 2u] = mix(stack[depth - 2u], stack[depth - 1u], share);
                depth = depth - 1u;
            }
            case TEX_DOTS: {
                if depth < 2u { return vec3<f32>(0.5); }
                if inside_dot(st_of(instruction.value, point, mapped, map).st) {
                    stack[depth - 2u] = stack[depth - 1u];
                }
                mapped = false;
                depth = depth - 1u;
            }
            case TEX_DIRECTIONMIX: {
                if depth < 2u { return vec3<f32>(0.5); }
                let amount = abs(dot(point.normal, instruction.value.xyz));
                stack[depth - 2u] = amount * stack[depth - 2u] + (1.0 - amount) * stack[depth - 1u];
                depth = depth - 1u;
            }
            case TEX_UV: {
                if depth >= 32u { return vec3<f32>(0.5); }
                stack[depth] = vec4<f32>(fract(st_of(instruction.value, point, mapped, map).st), 0.0, 1.0);
                mapped = false;
                depth = depth + 1u;
            }
            case TEX_MAP: {
                let rows = instruction.b * 3u;
                let m0 = texture_mappings[rows];
                let m1 = texture_mappings[rows + 1u];
                let m2 = texture_mappings[rows + 2u];
                let at = vec4<f32>(point.position, 1.0);
                let dx = vec4<f32>(point.position_dx, 0.0);
                let dy = vec4<f32>(point.position_dy, 0.0);
                solid = Solid(
                    vec3<f32>(dot(m0, at), dot(m1, at), dot(m2, at)),
                    vec3<f32>(dot(m0, dx), dot(m1, dx), dot(m2, dx)),
                    vec3<f32>(dot(m0, dy), dot(m1, dy), dot(m2, dy)),
                );
                switch instruction.a {
                    case MAP_PLANAR: { map = St(solid.p.xy, solid.dx.xy, solid.dy.xy); mapped = true; }
                    case MAP_SPHERICAL: { map = spherical_st(solid); mapped = true; }
                    case MAP_CYLINDRICAL: { map = cylindrical_st(solid); mapped = true; }
                    default: {}
                }
            }
            case TEX_CHECKERBOARD3D: {
                if depth < 2u { return vec3<f32>(0.5); }
                let cell = vec3<i32>(floor(solid.p));
                if ((cell.x + cell.y + cell.z) & 1) != 0 {
                    stack[depth - 2u] = stack[depth - 1u];
                }
                depth = depth - 1u;
            }
            case TEX_FBM, TEX_WRINKLED, TEX_WINDY, TEX_MARBLE: {
                if depth >= 32u { return vec3<f32>(0.5); }
                stack[depth] = solid_noise(instruction.op, solid, instruction.a, instruction.value);
                depth = depth + 1u;
            }
            default: { return vec3<f32>(0.5); }
        }
    }
    if depth != 1u { return vec3<f32>(0.5); }
    return stack[0].rgb;
}

// A program packed by `texture_vm::pack`: start << 9 | count.
fn program_value(packed: u32, point: TexturePoint) -> vec3<f32> {
    return eval_texture(packed >> 9u, packed & 511u, point);
}
