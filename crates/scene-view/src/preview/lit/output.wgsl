// Output pass: film exposure, clamp to [0, 1] as r4 does when it writes a
// PNG, then the sRGB color target encodes the result.
struct View {
    projection: mat4x4<f32>,
    mode: vec4<u32>,
    // x: film exposure (scale * iso / 100).
    exposure: vec4<f32>,
    background: vec4<f32>,
    shadow_origin: vec4<f32>,
    ray_forward: vec4<f32>,
    ray_right: vec4<f32>,
    ray_up: vec4<f32>,
    // x: AO radius in world units (0 = no AO).
    ao: vec4<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(0) @binding(1) var hdr: texture_2d<f32>;
@group(0) @binding(2) var ao: texture_2d<f32>;

struct Out {
    @builtin(position) position: vec4<f32>,
};

// One triangle covering the viewport.
@vertex fn vs_output(@builtin(vertex_index) index: u32) -> Out {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Out;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    return out;
}

@fragment fn fs_output(i: Out) -> @location(0) vec4<f32> {
    let p = vec2<i32>(i.position.xy);
    // mode.w: 0 = shaded, 1 = debug albedo, 2 = debug AO.
    if view.mode.w == 2u { return vec4<f32>(vec3<f32>(textureLoad(ao, p, 0).x), 1.0); }
    var rgb = textureLoad(hdr, p, 0).rgb;
    if view.mode.w == 0u { rgb *= view.exposure.x; }
    return vec4<f32>(clamp(rgb, vec3<f32>(0.0), vec3<f32>(1.0)), 1.0);
}
