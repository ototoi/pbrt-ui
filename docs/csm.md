# CSM Split-Space Derivation and Projection Workflow

This note describes a practical workflow for directional-light shadow slicing.
The key idea is to separate:

- **Split selection space**: used only to decide per-slice near/far ranges.
- **Light projection space**: used to build the final shadow map matrices.

## 1. Define a split axis per light

For each directional light, define a split axis (`split_forward`) that is:

- Orthogonal to the light direction.
- As aligned as possible with the camera forward direction.

```rust
let right = cross(camera_forward, light_direction).normalize();
let split_forward = cross(light_direction, right).normalize();
```

Recommended safeguards:

- If `abs(dot(camera_forward, light_direction))` is near 1.0 (almost parallel),
  this basis is unstable, so use a fallback path.
- Optionally flip sign so `dot(split_forward, camera_forward) >= 0`.

## 2. Build split space (origin at camera position)

Define split-space basis:

- `z = split_forward`
- `x = right`
- `y = cross(z, x)` (normalize)

Use `camera_position` as split-space origin.

For each bound/caster point `p_world`:

```text
v = p_world - camera_position
sx = dot(v, x)
sy = dot(v, y)
sz = dot(v, z)
```

Collect `min(sz)` and `max(sz)` over the points:

- `split_near = min(sz)`
- `split_far  = max(sz)`

This produces per-light split depth bounds in split space.

## 3. Slice the camera frustum in split space

Use `split_near/split_far` to cut the camera frustum.

At this stage, split space is used only to determine where each cascade begins
and ends for the current light.

## 4. Reconstruct sliced frustum corners in world space

From the sliced intervals, reconstruct the corresponding frustum corners in
world space.

These world-space corners represent the receiver region for each cascade slice.

## 5. Build final shadow projection in light space

Create `light_view` from the directional light orientation.

Transform the sliced world-space corners into `light_view`, compute their AABB,
and build `light_proj` (typically orthographic) from that light-space AABB.

Then:

```text
light_view_proj = light_proj * light_view
```

This matrix is the final matrix used for shadow map rendering and sampling.

## 6. Why this separation matters

- Split space is good for robust per-light cascade partitioning.
- Light space is required for the actual shadow-map projection.

By keeping these roles separate, the implementation avoids mixing split logic
with final projection logic and remains easier to debug and extend.
