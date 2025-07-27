struct VertexOut {
    @location(0) color: vec4<f32>,
    @builtin(position) position: vec4<f32>,
};

struct Uniforms {
    @size(16) angle: f32, // pad to 16 bytes
};

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(
    @location(0) position: vec4<f32>,
    @location(1) color: vec4<f32>,
) -> VertexOut {
    var out: VertexOut;

    out.position = vec4<f32>(position.xy * 0.8, 0.0, 1.0);
    out.position.x = out.position.x * cos(uniforms.angle);
    out.color = color;
    //out.color = vec4<f32>(1.0, 1.0, 1.0, 1.0);

    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
