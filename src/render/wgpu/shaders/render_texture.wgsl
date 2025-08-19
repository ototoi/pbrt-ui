
@group(0)
@binding(0)
var<uniform> texture: GlobalUniforms;

@group(0)
@binding(1)
var<uniform> sampler: LocalUniforms;

struct VertexOut {
    @location(0) uv: vec2<f32>,
    @builtin(position) position: vec4<f32>,
};

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) uv: vec2<f32>,
) -> VertexOut {
    var out: VertexOut;
    out.position = vec4<f32>(position, 1.0);
    out.uv = uv;

    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let tex_color = texture.sample(sampler, in.uv);
    let color = vec4<f32>(tex_color.r, tex_color.g, tex_color.b, 1.0);
    return color;
}