// The binding-array path's mip chains render on the
// GPU. One fullscreen-triangle pass per level samples the previous level's
// single-level view bilinearly and overwrites the next level — and the same
// pass downscales one capped source's temporary full-size texture into
// level 0 of its stored texture. Sampling an sRGB view filters in linear
// light and writing to an sRGB view re-encodes, so the chain keeps the
// linear-space averaging of the CPU chain it replaced.
@group(0) @binding(0) var previous: texture_2d<f32>;
@group(0) @binding(1) var tap: sampler;

struct VsOut {
    @builtin(position) position: vec4<f32>,
    // The NDC square [-1,1] mapped over [0,1] of the source view; the parts
    // of the triangle outside the target never rasterize.
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_fullscreen(@builtin(vertex_index) index: u32) -> VsOut {
    var corners = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(3.0, -1.0),
        vec2<f32>(-1.0, 3.0),
    );
    var out: VsOut;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    // Framebuffer row 0 is NDC y = +1 and texel row 0 is v = 0: flip y so
    // each pass keeps the source orientation.
    out.uv = vec2<f32>(corners[index].x * 0.5 + 0.5, 0.5 - corners[index].y * 0.5);
    return out;
}

// One explicit level-0 tap of the single-level source view: a 2x downscale
// lands its sample centres exactly between the previous level's texel
// centres, so the bilinear weights are the box average the CPU chain took.
@fragment
fn fs_mip(input: VsOut) -> @location(0) vec4<f32> {
    return textureSampleLevel(previous, tap, input.uv, 0.0);
}
