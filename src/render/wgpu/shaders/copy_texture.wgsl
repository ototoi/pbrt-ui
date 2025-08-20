// Copy from texture_copy.wgsl @egui-wgpu
struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

var<private> positions: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0),
    vec2<f32>(-1.0,  1.0),
    vec2<f32>( 1.0,  1.0),
    vec2<f32>(-1.0, -1.0),
    vec2<f32>( 1.0,  1.0),
    vec2<f32>( 1.0, -1.0),
);

var<private> uv: array<vec2<f32>, 6> = array<vec2<f32>, 6>(
    vec2<f32>( 0.0,  1.0),
    vec2<f32>( 0.0,  0.0),
    vec2<f32>( 1.0,  0.0),
    vec2<f32>( 0.0,  1.0),
    vec2<f32>( 1.0,  0.0),
    vec2<f32>( 1.0,  1.0),
);

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var result: VertexOutput;
    result.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    result.uv = uv[vertex_index];
    return result;
}

@group(0)
@binding(0)
var tex: texture_2d<f32>;

@group(0)
@binding(1)
var smp: sampler;

fn l2srgb(value: f32) -> f32 {
    return select(
        12.92 * value,
        1.055 * pow(value, 1.0 / 2.4) - 0.055,
        value > 0.0031308
    );
}

fn linear_to_srgb(value: vec3<f32>) -> vec3<f32> {
    return vec3<f32>(
        l2srgb(value.r),
        l2srgb(value.g),
        l2srgb(value.b)
    );
}

@fragment
fn fs_main(vertex: VertexOutput) -> @location(0) vec4<f32> {
    let uv = vertex.uv; // Convert from clip space to UV space
    let color = textureSample(tex, smp, uv);
    let rgb = clamp(linear_to_srgb(color.rgb), vec3<f32>(0.0), vec3<f32>(1.0));
    return vec4<f32>(rgb, color.a);
}