struct VertexOut {
    @location(0) color: vec4<f32>,
    @builtin(position) position: vec4<f32>,
};

//struct Uniforms {
//    @size(16) angle: f32, // pad to 16 bytes
//};

//@group(0) @binding(0)
//var<uniform> uniforms: Uniforms;

@group(0)
@binding(0)
var<uniform> transform: mat4x4<f32>;

@vertex
fn vs_main(
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
) -> VertexOut {
    var out: VertexOut;

    out.position = vec4<f32>(position * 0.8, 1.0);
    out.position = transform * out.position;
    out.position.z = out.position.z * 0.5 + 0.5;  //0..1
    out.color = vec4<f32>(color, 1.0);

    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    return in.color;
}
