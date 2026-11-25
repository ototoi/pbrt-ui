// Image-based lights: baked into one world-space cube map, then drawn behind
// everything. Separate from the Lit shader, which only reads the results.
struct Face {
    forward: vec4<f32>,
    // Per unit of s and t across the face, with t growing down the image.
    s: vec4<f32>,
    t: vec4<f32>,
    light_from_world: mat3x4<f32>,
    // x: radiance scale; y: face size in texels.
    scale: vec4<f32>,
};
@group(0) @binding(0) var<uniform> face: Face;
@group(0) @binding(1) var source: texture_2d<f32>;

@vertex fn vs_fullscreen(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    return vec4<f32>(corners[index], 0.0, 1.0);
}

// pbrt's EqualAreaSphereToSquare.
fn sphere_to_square(d: vec3<f32>) -> vec2<f32> {
    let a = abs(d);
    let r = sqrt(max(1.0 - a.z, 0.0));
    let high = max(a.x, a.y);
    let low = min(a.x, a.y);
    let b = select(low / high, 0.0, high == 0.0);
    var phi = ((((((-0.025139097 * b + 0.041903882) * b + 0.088177066) * b - 0.24733373) * b + 0.0061572018) * b + 0.63622655) * b + 0.0000040675857);
    if a.x < a.y {
        phi = 1.0 - phi;
    }
    var v = phi * r;
    var u = r - v;
    if d.z < 0.0 {
        let swapped = u;
        u = 1.0 - v;
        v = 1.0 - swapped;
    }
    return 0.5 * (vec2<f32>(select(u, -u, d.x < 0.0), select(v, -v, d.y < 0.0)) + 1.0);
}

// Additive: every image-based light adds to the cube. r4 reads the nearest
// texel; four samples per cube texel keep a small bright sun from aliasing.
@fragment fn fs_bake(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<f32>(textureDimensions(source));
    let offsets = array<vec2<f32>, 4>(
        vec2<f32>(-0.25, -0.25), vec2<f32>(0.25, -0.25), vec2<f32>(-0.25, 0.25), vec2<f32>(0.25, 0.25),
    );
    var sum = vec3<f32>(0.0);
    for (var k = 0u; k < 4u; k = k + 1u) {
        let at = 2.0 * (position.xy + offsets[k]) / face.scale.y - 1.0;
        let world = vec4<f32>(normalize(face.forward.xyz + at.x * face.s.xyz + at.y * face.t.xyz), 0.0);
        let uv = sphere_to_square(normalize(world * face.light_from_world));
        let texel = min(vec2<i32>(uv * size), vec2<i32>(size) - vec2<i32>(1));
        sum += textureLoad(source, max(texel, vec2<i32>(0)), 0).rgb;
    }
    return vec4<f32>(sum * 0.25 * face.scale.x, 1.0);
}
