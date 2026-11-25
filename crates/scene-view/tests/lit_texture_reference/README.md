# Texture VM pixel references

Generated with `../pbrt-r4/target/release/pbrt-r4`, revision
`211eb9e` (4.5.2):

```sh
for name in imagemap scale mix checkerboard bilerp dots uv composite directionmix material_mix alpha sphere_uv cylinder_uv cone_uv disk_uv paraboloid_uv hyperboloid_uv bilinear_uv bilinear_uv_tex heightfield_uv nurbs_uv map_planar map_spherical map_cylindrical map_instance solid_checkerboard solid_fbm solid_wrinkled solid_windy solid_marble; do
  ../pbrt-r4/target/release/pbrt-r4 --spp 4096 --quiet --maxdepth 1 \
    -o crates/scene-view/tests/lit_texture_reference/$name.png \
    crates/scene-view/tests/lit_texture_reference/$name.pbrt
done
```

The scenes use diffuse surfaces, a constant white infinite light, exposure 1,
a fixed 128×128 perspective camera, and Halton sampling. `quadrants.png` is a
64×32 RGB image with four constant quadrants, explicitly decoded as linear.
`scale` uses the v4 `tex`/float `scale` inputs. `mix` combines a literal RGB
input with a scaled imagemap, using a float imagemap amount.

The test captures `debug_albedo`, bypassing lighting and exposure. Both PNGs
and capture bytes are decoded with the piecewise sRGB transfer function before
comparison; every retained channel must satisfy
`abs(actual-reference) <= 0.02 + 0.03 * abs(reference)`.

The fixed geometric mask keeps x/y 35..92, excluding x/y 62..65. The square
projects to 33.1..94.9, so this removes its silhouette filter band. The central
bands remove the image's predefined UV discontinuities and their filter
footprint. The mask is defined before examining rendered errors. Its coverage
is asserted: 2916 compared pixels and 13468 excluded pixels (including the
background), or 17.80% compared / 82.20% excluded. Every compared pixel is
checked independently; no region averaging is used.

Run with:

```sh
cargo test -p scene-view --test lit_texture_reference -- --include-ignored
```

Sampling check: an additional 1024-spp render of `imagemap` was compared to
the stored 4096-spp image on the same fixed mask. Maximum linear channel
difference was 0.007539 and RMS difference 0.002085, below the minimum 0.02
comparison allowance. This check uses the same lighting, geometry, camera,
and sampling distribution as the scale/mix scenes.

Validation: this test initially failed 7290/8748 retained channel comparisons
because Lit omitted pbrt's imagemap V inversion. It passed after the shader
was corrected, with the same references, mask, and thresholds. Independently,
vertically flipping only `imagemap.png` caused 7290/8748 channel failures;
restoring the reference returned the two tests to green.

## Procedural textures

`checkerboard`, `bilerp`, `dots`, `uv`, `composite`, `directionmix`,
`material_mix`, `alpha` and the `*_uv` analytic shapes use the same comparison, but their discontinuities depend on the pattern, so the mask
comes from the reference image: a pixel is skipped when anything within two
pixels differs from it by more than 0.1 in r4's image. A misplaced pattern in
Lit still fails wherever it lands away from r4's edges. `solid_marble` also skips
Lit's edges: r4 averages the spline's small jump across a pixel, which leaves it
under the threshold on both sides. Each scene also asserts
a minimum number of compared pixels, so the mask cannot grow to swallow it.

Two scenes are built so that this mask is meaningful:

- `dots` uses a scale of 3. At 8 the dots were about 2.7 pixels in radius, and
  the two-pixel band hid their interiors.
- `composite` keeps its `amount` within 0.3..0.7, so a pattern's edge never
  fades below the mask's 0.1 threshold while its filter band still differs.

## The remaining analytic uv shapes

`sphere_uv`, `cylinder_uv`, `cone_uv` and `disk_uv` are joined by
`paraboloid_uv`, `hyperboloid_uv`, `bilinear_uv`, `bilinear_uv_tex`,
`heightfield_uv` and `nurbs_uv`, rendered the same way (`--spp 4096 --quiet
--maxdepth 1`). All show the bilerp corner colors through their shape uv: the
bilinear scenes compare the patch's own (u, v) against control-point
`texcoord2 uv` corners, heightfield keeps its grid position, and nurbs carries
its unnormalized knot parameters (clipped ranges [0.25, 0.75] × [0.3, 0.6],
`diceu`/`dicev` 64 — r4 honors both — so the two surfaces dice identically).

The bowls alias at their rims: Lit rasterizes whole pixels while r4's gaussian
pixel filter smears samples two pixels wide, and rays that graze the rim trade
a few percent of blended coverage. Three things keep that out of the
comparison, all set from the scene geometry before examining errors:

- both bowls pose opening away from the camera (an umbrella back), so no ray
  flips between the outer wall and an inner wall inside the compared region;
- they compare inside a disk — radius 24 for the paraboloid's 37-pixel
  silhouette, 21 for the hyperboloid's 31 — like `directionmix` near the
  sphere's limb;
- the bowls also skip Lit's own hard transitions (`solid_marble`-style
  masking), and use gentler bilerp corners (0.75/0.35) so the r4 filter's tail
  stays under the 0.02+3% threshold where both images look smooth.

The heightfield keeps its hills below a 0.6 footprint (Pz within 0.47..0.53):
Lit samples the pixel center while r4's filter averages the hills' parallax,
and the sampling difference left ±15% on the steeper slopes. The same physics
is why `nurbs_uv`'s saddle is flattened in z.

Validation: each mapping was mutation-tested by flipping or transposing it —
paraboloid's u axis (4051 of 4794 channels failing), hyperboloid's v (1813 of
3876), heightfield's uv pair (5768 of 9438), nurbs' knot normalization (4051 of
9147) and `bilinear_uv_tex`'s corner texcoords (3147 of 8133) — and restored,
with the same references, masks and thresholds, green.

`directionmix` compares inside radius 46 of the sphere, which projects to
radius 48.6; `debug_albedo` does not draw the light's background.

Scenes with a backing plane keep it 50 units behind, so that the shape occludes
a negligible share of the plane's light: `debug_albedo` has no occlusion.

## Mappings and solid textures

| Texture | uv | planar | spherical | cylindrical | point |
|---|---|---|---|---|---|
| imagemap | `imagemap` | - | - | - | n/a |
| checkerboard | `checkerboard` | `map_planar`, `map_instance` | - | - | `solid_checkerboard` |
| bilerp | `bilerp` | - | `map_spherical` | `map_cylindrical` | n/a |
| dots | `dots` | - | - | - | n/a |
| uv | `uv` | - | - | - | n/a |
| fbm, wrinkled, windy, marble | n/a | n/a | n/a | n/a | `solid_*` |

All 2D textures read their coordinates through one shared path, so `-` means
that combination has no scene of its own, not that it is disabled.

Those scenes use 1024 samples. r4 shrinks its footprints to at most 1/8 of a
pixel at high sample counts, while the preview uses the real pixel, so noise
sums more octaves in r4 where a footprint limits them. The `solid_*` scenes set
`octaves` low enough to reach the limit either way, and compare within radius 40
because grazing pixels near the limb have the largest footprints.
