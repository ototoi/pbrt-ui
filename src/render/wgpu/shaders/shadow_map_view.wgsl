struct ViewUniforms {
    layer: u32,
    _pad0: u32,
    _pad1: u32,
    _pad2: u32,
}

@group(0) @binding(0)
var shadow_map: texture_depth_2d_array;

@group(0) @binding(1)
var<uniform> view_uniforms: ViewUniforms;

struct VertexOut {
    @location(0) uv: vec2<f32>,
    @builtin(position) position: vec4<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOut {
    var pos = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0, -1.0),
        vec2<f32>( 1.0,  1.0),
        vec2<f32>(-1.0,  1.0),
    );
    var out: VertexOut;
    let p = pos[vertex_index];
    out.position = vec4<f32>(p, 0.0, 1.0);
    out.uv = p * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5, 0.5);
    return out;
}

@fragment
fn fs_main(in: VertexOut) -> @location(0) vec4<f32> {
    let dims = textureDimensions(shadow_map);
    let x = clamp(i32(in.uv.x * f32(max(i32(dims.x), 1))), 0, max(i32(dims.x), 1) - 1);
    let y = clamp(i32(in.uv.y * f32(max(i32(dims.y), 1))), 0, max(i32(dims.y), 1) - 1);
    let d = textureLoad(shadow_map, vec2<i32>(x, y), i32(view_uniforms.layer), 0);
    let vis = pow(clamp(1.0 - d, 0.0, 1.0), 0.2);
    return vec4<f32>(vec3<f32>(vis), 1.0);
}
