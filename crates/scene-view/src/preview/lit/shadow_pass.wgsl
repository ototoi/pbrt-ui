// Shadow-map pass: draws scene geometry from one light into one layer of the
// shadow array, writing linear depth. Geometry is placed relative to the
// shadow origin (scene center), never the eye, so camera moves keep the maps.
const SHADOW_POSITIONAL: u32 = 1u;

// Mirrors `lit::shadow::ShadowPassGpu`; one per pass at a dynamic offset.
struct ShadowPass {
    matrix: mat4x4<f32>,
    // xyz: light position in the shadow frame; w: far.
    position_far: vec4<f32>,
    // x: SHADOW_* kind.
    kind: vec4<u32>,
};
@group(0) @binding(0) var<uniform> pass_info: ShadowPass;

struct ShadowVertex {
    @builtin(position) clip: vec4<f32>,
    @location(0) point: vec3<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) normal: vec3<f32>,
    // Packed alpha program of this draw's shape (0 = none).
    @location(3) @interpolate(flat) alpha: u32,
};

@vertex fn vs_shadow(
    @location(0) position: vec3<f32>,
    @location(1) nominal: vec3<f32>,
    @location(2) texel: vec2<f32>,
    @location(3) w0: vec4<f32>,
    @location(4) w1: vec4<f32>,
    @location(5) w2: vec4<f32>,
    @location(6) n0: vec4<f32>,
    @location(7) n1: vec4<f32>,
    @location(8) n2: vec4<f32>,
    @location(9) info: vec4<u32>,
) -> ShadowVertex {
    let local = vec4<f32>(position, 1.0);
    let point = vec3<f32>(dot(w0, local), dot(w1, local), dot(w2, local));
    var out: ShadowVertex;
    out.clip = pass_info.matrix * vec4<f32>(point, 1.0);
    out.point = point;
    out.uv = texel;
    out.normal = vec3<f32>(dot(n0.xyz, nominal), dot(n1.xyz, nominal), dot(n2.xyz, nominal));
    out.alpha = info.z;
    return out;
}

@fragment fn fs_shadow(input: ShadowVertex) -> @builtin(frag_depth) f32 {
    // Derivatives before the branch, seen from the light.
    let point = TexturePoint(
        input.uv, dpdx(input.uv), dpdy(input.uv), normalize(input.normal),
        input.point, dpdx(input.point), dpdy(input.point),
    );
    if input.alpha != 0u && program_value(input.alpha, point).x < 0.5 {
        discard;
    }
    if pass_info.kind.x == SHADOW_POSITIONAL {
        return clamp(distance(input.point, pass_info.position_far.xyz) / pass_info.position_far.w, 0.0, 1.0);
    }
    // Orthographic: clip depth is already linear.
    return input.clip.z;
}
