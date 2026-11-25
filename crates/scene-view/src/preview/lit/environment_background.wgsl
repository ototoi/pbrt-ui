// The environment behind everything: drawn first, added to the cleared
// uniform background.
struct View {
    projection: mat4x4<f32>,
    mode: vec4<u32>,
    exposure: vec4<f32>,
    background: vec4<f32>,
    shadow_origin: vec4<f32>,
    // A pixel's view ray: forward + ndc.x * right + ndc.y * up.
    ray_forward: vec4<f32>,
    ray_right: vec4<f32>,
    ray_up: vec4<f32>,
    // x: AO radius in world units (0 = no AO).
    ao: vec4<f32>,
};
@group(0) @binding(0) var<uniform> view: View;
@group(3) @binding(1) var environment_cube: texture_cube<f32>;
@group(3) @binding(2) var environment_sampler: sampler;

struct Corner {
    @builtin(position) position: vec4<f32>,
    @location(0) ndc: vec2<f32>,
};

@vertex fn vs_background(@builtin(vertex_index) index: u32) -> Corner {
    var corners = array<vec2<f32>, 3>(vec2<f32>(-1.0, -1.0), vec2<f32>(3.0, -1.0), vec2<f32>(-1.0, 3.0));
    var out: Corner;
    out.position = vec4<f32>(corners[index], 0.0, 1.0);
    out.ndc = corners[index];
    return out;
}

struct Fragment {
    @location(0) color: vec4<f32>,
    @location(1) pick: u32,
};

@fragment fn fs_background(i: Corner) -> Fragment {
    let ray = view.ray_forward.xyz + i.ndc.x * view.ray_right.xyz + i.ndc.y * view.ray_up.xyz;
    var out: Fragment;
    out.color = vec4<f32>(textureSampleLevel(environment_cube, environment_sampler, ray, 0.0).rgb, 1.0);
    out.pick = 0u;
    return out;
}
