struct View {
    projection: mat4x4<f32>,
    mode: vec4<u32>,
    exposure: vec4<f32>,
    background: vec4<f32>,
    // xyz: eye - shadow origin; moves eye-relative points into the shadow frame.
    shadow_origin: vec4<f32>,
    ray_forward: vec4<f32>,
    ray_right: vec4<f32>,
    ray_up: vec4<f32>,
    // x: AO radius in world units (0 = no AO).
    ao: vec4<f32>,
};
@group(0) @binding(0) var<uniform> view: View;

struct Out {
    // Invariant: Lit's PreZ and opaque passes must produce identical depth (`Equal`).
    @builtin(position) @invariant clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) pick: u32,
    @location(2) uv: vec2<f32>,
    @location(3) normal: vec3<f32>,
    @location(4) world: vec3<f32>,
    @location(5) @interpolate(flat) appearance: u32,
    // Packed alpha program of this draw's shape (0 = none).
    @location(6) @interpolate(flat) alpha: u32,
};
@vertex fn vs_main(
    @location(0) position: vec3<f32>, @location(1) nominal: vec3<f32>, @location(2) texel: vec2<f32>,
    @location(3) w0: vec4<f32>, @location(4) w1: vec4<f32>, @location(5) w2: vec4<f32>,
    @location(6) n0: vec4<f32>, @location(7) n1: vec4<f32>, @location(8) n2: vec4<f32>,
    @location(9) info: vec4<u32>,
) -> Out {
    let p=vec4<f32>(position,1.0);
    let world=vec4<f32>(dot(w0,p),dot(w1,p),dot(w2,p),1.0);
    let transformed=vec3<f32>(dot(n0.xyz,nominal),dot(n1.xyz,nominal),dot(n2.xyz,nominal));
    var n=vec3<f32>(0.0,0.0,1.0); if dot(transformed,transformed)>1e-20 { n=normalize(transformed); }
    var out:Out; out.clip=view.projection*world; out.color=vec4<f32>(0.0,0.0,0.0,1.0); out.pick=info.x; out.uv=texel; out.normal=n; out.world=world.xyz; out.appearance=info.y; out.alpha=info.z; return out;
}
