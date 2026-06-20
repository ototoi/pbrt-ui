# Shadow Redesign Notes

This document summarizes the current directional-shadow implementation and proposes a simpler replacement. The intent is to support deleting the current CSM path if needed and rebuilding from a cleaner base.

## Current Findings

The current implementation is centered in [`src/render/wgpu/shadow.rs`](/Users/toru/src/others/pbrt-ui/src/render/wgpu/shadow.rs) and uses per-light split-space CSM.

What is already working:

- CPU and GPU cascade selection use the same split basis data:
  - `split_origin`
  - `split_forward`
  - `split_end`
- Shadow rendering and sampling are wired through:
  - [`src/render/wgpu/directional_shadow_map_renderer.rs`](/Users/toru/src/others/pbrt-ui/src/render/wgpu/directional_shadow_map_renderer.rs)
  - [`assets/shaders/include/lighting_shadow_functions.wgsl`](/Users/toru/src/others/pbrt-ui/assets/shaders/include/lighting_shadow_functions.wgsl)

The main design problems are:

1. `shadow.rs` mixes too many responsibilities.
   - scene analysis
   - cascade split selection
   - frustum slicing
   - light-space fitting
   - shadow texture allocation

2. Receiver and caster selection are too approximate.
   - `build_shadow_scene_context()` uses mesh AABB corners only.
   - casters are not sliced per cascade; Z fitting is global and overlap-tested heuristically.

3. The split-space algorithm is more complex than the current renderer needs.
   - it adds instability/debug cost before the basic shadow pipeline is validated
   - LSPSM is still `todo!()`, so the abstraction is wider than the implementation

4. The current code is hard to inspect.
   - debug information is spread across CPU logs and shader debug modes
   - there is no explicit “shadow build result” structure for inspection

## Recommendation

Do not keep the current CSM design as the baseline. Replace it with a staged design:

1. First rebuild a stable single-cascade directional shadow.
2. After that works visually, add simple camera-depth CSM.
3. Only reintroduce split-space or other advanced partitioning if a real artifact remains.

This lowers risk and makes debugging much easier.

## Proposed Architecture

Split the shadow system into these layers:

### 1. Shadow Scene Extraction

Input:

- `RenderCamera`
- shadow-casting directional lights
- shadow-casting mesh items

Output:

- `ShadowSceneSnapshot`

Suggested contents:

- `camera_frustum_corners_world`
- `receiver_bounds_world`
- `caster_bounds_world`
- `mesh_instances`

This layer should not decide cascades or build matrices.

### 2. Shadow Partitioning

Input:

- `ShadowSceneSnapshot`
- light parameters

Output:

- `Vec<ShadowSlice>`

Start with the simplest version:

- one slice for the full visible camera range

Then add basic CSM:

- partition by camera depth only
- use standard practical split scheme
- do not use per-light split-space in the first rewrite

Each `ShadowSlice` should contain:

- `view_near`
- `view_far`
- `receiver_frustum_corners_world`

### 3. Shadow Projection Fitting

Input:

- `ShadowSlice`
- light direction
- caster bounds

Output:

- `ShadowCascade`

Responsibilities:

- build `light_view`
- fit orthographic projection from receiver footprint
- expand Z from relevant casters
- snap XY to texel grid

This should be a pure geometry step and should not allocate textures.

### 4. Shadow Resource Preparation

Input:

- `Vec<ShadowCascade>`

Output:

- GPU textures
- info buffer
- runtime shadow map handles

This layer should own:

- shadow-map array allocation
- cascade-to-layer mapping
- buffer upload

## Suggested Data Model

Prefer explicit intermediate structs instead of passing raw vectors around:

```rust
struct ShadowSceneSnapshot {
    camera_frustum_corners_world: [glam::Vec3; 8],
    receiver_points_world: Vec<glam::Vec3>,
    caster_points_world: Vec<glam::Vec3>,
}

struct ShadowSlice {
    near: f32,
    far: f32,
    frustum_corners_world: [glam::Vec3; 8],
}

struct FittedShadowCascade {
    light_view: glam::Mat4,
    light_proj: glam::Mat4,
    light_view_proj: glam::Mat4,
    split_near: f32,
    split_far: f32,
}
```

If split-space CSM is reintroduced later, add a separate type for it instead of overloading the basic slice model.

## Recommended Rewrite Order

1. Delete `DirectionalShadowProjection::Lspsm`.
2. Replace current CSM build path with a single-cascade orthographic directional shadow.
3. Validate:
   - light direction
   - projection fitting
   - bias behavior
   - shadow acne / peter-panning
4. Reintroduce cascades with camera-depth partitioning only.
5. Add better debug views:
   - selected cascade index
   - projected UV/depth
   - cascade bounds overlay

## What To Keep

These parts are still useful and can stay with small adjustments:

- directional shadow map renderer
- shader-side shadow sampling path
- texel snapping idea in orthographic fitting
- directional shadow info buffer layout

## What To Remove

These can be deleted in the rewrite if they get in the way:

- split-space-specific basis logic in `shadow.rs`
- `DirectionalShadowProjection::Lspsm`
- current mixed “scene extraction + fitting + allocation” flow
- any debug path that depends on the old split-space model

## Bottom Line

The current implementation is not fundamentally broken because of one bug; it is difficult because the initial CSM design is too ambitious for the current state of the renderer.

The safest path is:

- rebuild directional shadowing as one clear orthographic cascade
- add plain CSM second
- only add split-space logic after the simpler design proves insufficient
