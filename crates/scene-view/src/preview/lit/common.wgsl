// Lit bindings and the GPU contract shared with `lit/mod.rs`. The constants
// mirror the Rust ones; `wgsl_constants_match_rust` checks they agree.
const PI: f32 = 3.141592653589793;

const LIGHT_POINT: u32 = 0u;
const LIGHT_SPOT: u32 = 1u;
const LIGHT_DISTANT: u32 = 2u;
const LIGHT_INFINITE: u32 = 3u;
const LIGHT_POLYGON: u32 = 4u;
const LIGHT_SPHERE: u32 = 5u;
const LIGHT_ENVIRONMENT: u32 = 6u;
const LIGHT_GONIOMETRIC: u32 = 7u;
const LIGHT_PROJECTION: u32 = 8u;
const SHADOW_SIZE: u32 = 1024u;
const LTC_SIZE: u32 = 64u;
const SHADOW_NONE: u32 = 0u;
const SHADOW_POSITIONAL: u32 = 1u;
const SHADOW_DISTANT: u32 = 2u;

const KIND_LAMBERT: u32 = 0u;
const KIND_EMISSIVE: u32 = 1u;
const KIND_INVISIBLE: u32 = 3u;
const KIND_MIX: u32 = 4u;
const LOBE_REFLECTANCE_IS_F0: u32 = 1u;
const LOBE_REMAP_ROUGHNESS: u32 = 2u;
// pbrt's default material is diffuse with reflectance 0.5.
const DEFAULT_ALBEDO: vec3<f32> = vec3<f32>(0.5);

struct Light {
    // xyz: eye-relative position (point, spot, emitter).
    position: vec4<f32>,
    // xyz: spot axis (light -> scene), distant direction (surface -> light)
    // or emitter normal; w: spot cosStart, emitter a_dir.
    direction: vec4<f32>,
    // rgb: I (point, spot), E (distant), L (infinite, emitter);
    // w: spot cosEnd, emitter a_iso.
    color: vec4<f32>,
    // x: LIGHT_* kind; y: emitter two-sided (0/1).
    kind: vec4<u32>,
};

struct Lobe {
    diffuse: vec3<f32>,
    kind: u32,
    spec_f0: vec3<f32>,
    spec_alpha: f32,
    spec_f90: vec3<f32>,
    transmission: f32,
    coat: vec4<f32>,
    // y: emitter two-sided (0/1).
    texture: vec4<f32>,
    // Packed texture programs (0 = none): reflectance, spec u, spec v, coat u.
    programs_a: vec4<u32>,
    // Coat v, mix amount, then a mix's two child lobes (indices).
    programs_b: vec4<u32>,
    // x: LOBE_* bits.
    flags: vec4<u32>,
};

// Mirrors `lit::shadow::ShadowGpu`; indexed like `lights`. Positions and
// matrices are in the shadow frame (world relative to the scene center).
struct ShadowEntry {
    matrices: array<mat4x4<f32>, 6>,
    // xyz: light position; w: far (positional) or depth range (distant).
    position_far: vec4<f32>,
    // x: first layer; y: layer count; z: SHADOW_* kind.
    params: vec4<u32>,
    // x: light radius for PCSS.
    size: vec4<f32>,
};

// Irradiance harmonics of the image-based lights, through l = 2.
// The environment light's screen-space AO from `ao.wgsl` (1 = unoccluded).
@group(0) @binding(1) var ao_map: texture_2d<f32>;
@group(3) @binding(0) var<uniform> environment_irradiance_sh: array<vec4<f32>, 9>;
// Their radiance prefiltered for GGX: level = sqrt(alpha) * (levels - 1).
@group(3) @binding(1) var environment_cube: texture_cube<f32>;
@group(3) @binding(2) var environment_sampler: sampler;
const ENVIRONMENT_LEVELS: u32 = 6u;
@group(2) @binding(0) var<storage, read> lights: array<Light>;
@group(2) @binding(1) var<storage, read> appearances: array<Lobe>;
@group(2) @binding(2) var<storage, read> emitter_vertices: array<vec4<f32>>;
@group(2) @binding(3) var<storage, read> shadow_entries: array<ShadowEntry>;
@group(2) @binding(4) var shadow_maps: texture_depth_2d_array;
@group(2) @binding(5) var shadow_sampler: sampler_comparison;
// GGX LTC table (lit/ltc.rs): layer 0 inverse matrix, layer 1 magnitude,
// Fresnel, 0, sphere clip. Across: sqrt(alpha); down: sqrt(1 - cos theta).
@group(2) @binding(6) var ltc_table: texture_2d_array<f32>;
