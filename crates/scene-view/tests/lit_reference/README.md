# Lit reference images

r4 renders each PNG with direct lighting only (`--maxdepth 1`), because Lit
has no indirect light. It uses 1024 samples per pixel:

```sh
for f in *.pbrt; do pbrt-r4 --spp 1024 --quiet --maxdepth 1 -o "${f%.pbrt}.png" "$f"; done
```

`mix_shadow` uses `--maxdepth 2`: passing through its `interface` leaf costs r4
a bounce, which would leave those cells black at depth 1. No extra light reaches
the shadow regions at that depth.

The `env_*` scenes read `env.exr` and `env.png`, synthetic 64×64 maps with
gradients and a sun disc. `env_view_*` look along each axis at fov 100 with
`--spp 256`, so that every cube face and its seams are compared. They only hold
a convex sphere or a speck, since Lit has no occlusion for image-based lights.

Renderer: `../pbrt-r4/target/release/pbrt-r4`

r4 commit: `211eb9e` (4.5.2)

## Direction-mapped light scenes

`goniometric_floor`, `goniometric_power`, `projection_floor`,
`projection_power` and `spot_shadow` come from the 2026-11-10 light audit
adapted to 128 film and non-symmetric
control maps: `gonio_diagram.png` (64×64 gray quadrants 230/180/60/30) and
`projection_slide.png` (128×64 colored quadrants red/green/blue/yellow), each
asymmetric on both axes so a flipped or mirrored lookup fails the region
comparison instead of passing by symmetry. Lit reads the maps exactly as r4
(`Image::lookup_nearest`: floor(uv·res), row 0 the file's top row).

- The goniometric and projection lights sit at `Translate 0.4 0.3 3.5`; the
  projection adds `Rotate 180 1 0 0` to turn r4's light-space +z projection
  axis toward the floor (r4 takes no from/to for these kinds), fov 60.
- Both scenes keep the audit's occluders — the z = 1.2 plate and the sphere
  at `(1.6, 0.8, 0.5)` — so their cube shadows compare against r4's.
- The `_power` variants replace the intensity with `"float power"`: r4's
  power conversion multiplies scale by power/(4π·mean Y) (goniometric) and
  power/(A·mean(lum·cos³)) (projection), which Lit mirrors on the CPU.
  Powers (80, 20) keep every lit surface below 1.0 linear, so no clipping
  pushes the two renderers' film filters apart.
- `spot_shadow` is the audit's spot scene at 128 film: cone 45°, delta 5,
  aimed at the origin from `(0.4, 0.3, 3.5)`, with the same occluders.

## Scenes that need special handling

- **`plastic_v3_quad.pbrt`** uses the pbrt-v3 `plastic` material, which r4
  refuses unless the input is upgraded. Its PNG is rendered from
  `plastic_v3_quad.r4.pbrt`, the same scene rewritten by hand the way r4's
  `--upgrade` rewrites it (`parser/upgrade.rs`): `coateddiffuse`, with `Kd`
  as `reflectance` and `Ks` dropped. `pbrt-r4 --upgrade` itself fails on
  this file with an unrelated blackbody error.
- **Wider tolerance (±25%).** `lit_reference.rs` lists the scenes whose
  materials Lit only approximates:
  - layered coats (`coateddiffuse_point`, `plastic_v3_quad`, `env_coateddiffuse`);
  - a GGX reflection seen at grazing angles (`conductor_quad`).
