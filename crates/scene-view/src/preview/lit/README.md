# Lit preview

Lit draws the scene in pbrt-v4 units and then applies the film exposure.

There is one shader, `pipeline::LIT_WGSL`, built by concatenating these files
in a fixed order:

1. `../vertex.wgsl`
2. `common.wgsl`
3. `lights.wgsl`
4. `main.wgsl`

Nothing is generated or selected per material or per light. Differences
between materials and lights are values in storage buffers, and the shader
branches on them.

## Passes

1. **PreZ** (`fs_prez`, `fs_prez_alpha`): the opaque draws write depth and
   their eye-facing interpolated normal (world axes, `n * 0.5 + 0.5` in
   `Rgb10a2Unorm`, about 0.1° of quantization). Draws with a shape alpha use
   `fs_prez_alpha`, which cuts the holes; the others skip the texture VM.
   Glass, mixes with a transmitting leaf and invisible draws stay out. The
   background keeps depth 0. The facet's screen derivatives, not the
   interpolated normal, decide which side faces the eye, so tessellated
   silhouettes keep their outward normal.
2. **AO** (`ao.wgsl`): GTAO from the PreZ depth and normals, 8 slices of 8
   steps each side, radius a fifth of the scene's diagonal, then a 4×4
   denoise over neighbors on the same surface (`R8Unorm`). Slices rotate by
   a 4×4 Bayer pattern, so each denoise window holds every rotation once;
   4 slices left streaks on grazing floors. Samples within 6°
   of the tangent plane do not occlude. A capture with `debug_ao` shows it.
   `tests/lit_ao_reference/` compares it with r4's visibility.
3. **Lit**: the environment background, then the opaque draws with depth
   `Equal` and no depth write, so each pixel is shaded once. They go in
   reverse order: of coincident surfaces, `Equal` keeps the last drawn and
   PreZ's `Greater` the first, so the first draw wins color, pick and
   normal, as before PreZ. The environment light's diffuse, specular and
   coat are scaled by the AO at the pixel; other lights keep their shadow
   maps. Glass follows, tested `Greater`, with AO 1: glass and transmitting
   mixes neither cast nor receive AO, and a note says so.
4. **Output**: film exposure, or the debug albedo or AO image.

The vertex position is `@invariant`, so both passes produce the same depth.
Solid, Normal and Wireframe skip PreZ. With PreZ, the reference scenes
render as before except one sphere-edge pixel off by 1/255. It cost about 0.2 s of cold pipeline build,
and cut a 1080p killeroo-simple frame from 95 ms to 61 ms.

## Light kinds

| kind | source | diffuse |
|---|---|---|
| point, spot | `LightSource "point"`, `"spot"` | `I·cosθ/d²` (spot: pbrt-v4 smoothstep cone) |
| distant | `LightSource "distant"` | `E·cosθ` |
| infinite | `LightSource "infinite"` without a map | `π·L` |
| polygon | area lights on triangle meshes, and disks as area-preserving 16-gons | each triangle is clipped to the horizon, then Lambert's edge integral (LTC with the identity matrix); one-sided per triangle |
| sphere | area lights on full spheres under a similarity transform | closed form with the horizon (Snyder) |
| goniometric | `LightSource "goniometric"` with a square map | pbrt-v4's direction map through the equal-area sphere-to-square; a point light with `I·Y(w)` |
| projection | `LightSource "projection"` | pbrt-v4's projected image through the pinhole; a point light with `scale·rgb(ps)` |

Both map-carrying kinds are point lights with a direction-varying
intensity ("gonio.rs" builds them from the r4 formulas):
- goniometric maps must be square (r4 errors otherwise; Lit notes and
  shades uniform). Their image rides a texture-VM image slot; the frame —
  light_from_world (with v4's swapYZ), slot, resolution, mean Y — appends
  to the emitter vertex buffer and `kind.z` indexes it like a polygon's.
- projection mirrors light space with `Scale(1, -1, 1)`, clips to the
  screen extent from the image aspect and fov, and modulates by the
  projected texel. Without a readable map Lit notes and skips the light,
  as r4 errors.
- Both count toward the 64-light limit, shade diffuse through
  `positional()` and specular through the point-light GGX path, and take
  positional cube shadows with a flux estimate from their means
  (`choose_shadowed` reads them off `direction.w`).
- A light map that misses the device's image budget, or whose slot the
  renderer cannot allocate, folds its uniform average into the intensity
  and notes it — the same treatment material textures get.

`emitters.rs` turns area lights into polygon or sphere entries within
`MAX_EMITTER_TRIANGLES`. A mesh that would exceed the budget becomes a
sphere that radiates the same power.

Analytic shapes are defined in shape space. Mesh positions are stored
relative to `MeshRecord::origin_f64`, so subtract that origin before you
apply a draw matrix.

## Shadows

`shadow.rs` picks the four lights with the largest estimated flux and gives
them maps in one `Depth32Float` array. Positional lights get six cube-face
layers each, and distant lights get one orthographic layer. Depth is linear.

The maps live in a frame centered on the scene bounds, never the eye. A
camera move therefore reuses them.

They are re-rendered when any of these change:
- the scene (`set_scene`);
- the placements (`update_draws`);
- the time sample;
- the shadowed lights' passes.

`PreviewRendererStats::shadow_renders` counts the re-renders. Each pass
reads its own uniform at a dynamic offset. A light's own emitter draw stays
out of its map.

`shadow.wgsl` looks the maps up as follows:
1. It moves the eye-relative point into the shadow frame, using
   `View::shadow_origin`.
2. It offsets the point by 1.5 texels along the normal.
3. It filters with 16 Poisson taps spread on the receiving surface. Each tap
   is projected on its own, so slanted receivers do not self-shadow.
4. For polygon and sphere lights, PCSS widens the taps with the light's
   radius.

## Materials

`lobe.rs` turns each pbrt material into one `GpuLobe`. A lobe has these
parts:
- Lambert diffuse;
- a GGX specular layer (F0, F90 = 1, α);
- an optional dielectric coat (F0, α, η, internal reflectance);
- a transmission share.

The shader branches only on these values.

- **pbrt-v3 names** are rewritten the way r4's `--upgrade` rewrites them,
  because r4 refuses them otherwise. A note says so.
- **Named metals** use F0 tables generated from r4's spectra by
  `tools/named-metal-f0.py`, one in vacuum and one under a 1.5 coat.

### Specular

- **Area lights** use LTC. The table is made by `tools/fit-ltc` and checked
  against the published fit.
  - Polygons are transformed, clipped and edge-integrated.
  - A sphere is the disk facing the point that subtends the same solid
    angle.
- **Point, spot and distant lights** evaluate pbrt-v4's Trowbridge-Reitz
  BRDF.
- **A uniform environment** reflects the lobe's directional albedo.

### Coats

The coat equations are closed forms of pbrt's stochastic layered BxDF:
- Light under the coat crosses it twice.
- A diffuse layer bounces inside the coat:
  `ρ·T_in·T_out / (η²(1 − ρ·r_i))`.
- A coated metal's eta and k are divided by the coat's eta.
- Of what the metal reflects, only the share inside the critical cone
  escapes. The totally reflected rest returns to the metal:
  `e / (1 − A(1 − e))`.

These forms are within 25% of r4. The diffuse layer comes out a little
high, and coated materials carry a note.

### Glass

Draws whose lobe transmits light are drawn in a second, blended pipeline:
- They are sorted far to near by bounds center. This is object-level
  ordering, not OIT.
- The blend is premultiplied: the reflection is added, and
  `alpha = 1 − transmission`.
- There is no refraction, so glass looks nearly clear apart from its
  highlights. r4 shows the lens, the caustics and the shadow.
- Glass stays out of the shadow maps.
- Glass is still pickable.

## Notes (`lit_notes`)

`lit::evaluate` collects each approximation as a `LitNote`. Notes are
grouped by reason and material, with the number of draws they affect.

| where | what you get |
|---|---|
| desktop | each note in the Console, once per change |
| capture daemon | `lit_notes` in its reply |
| core capture job | `lit_notes` copied into the job result |

One-shot captures that run without the daemon report no notes.

## Adding a light kind

1. In `mod.rs`, add a `LIGHT_*` constant.
2. In `common.wgsl`, add the same constant. `wgsl_constants_match_rust`
   checks that the two agree.
3. In `lights.rs`, convert the `LightRecord` into a `GpuLight` in pbrt-v4
   units, and give it a unit test.
4. In `lights.wgsl`, add one function and one `case` in `light_irradiance`.

## Adding a material

1. In `materials.rs`, map the pbrt material to its appearance.
2. Do not change the shader.

## Texture VM

Material reflectance, roughness, mix amount and shape alpha compile to bounded
postfix programs that run in WGSL. The supported nodes are constant, imagemap,
scale, mix, directionmix, checkerboard (2D and 3D), bilerp, dots, uv, fbm,
wrinkled, windy and marble, under uv, planar, spherical, cylindrical and point
mappings. Non-UV mappings keep their world-to-texture rows in f64 and convert
them per frame relative to the eye (Lit) or the shadow origin (shadow pass).
Missing references, cycles, singular transforms, invalid images, and instruction
or stack budget overflow produce a diagnostic and a constant fallback. Alpha is
a cutout at 0.5.

Keep one call site each for `surface`, `noise_sum`, `perlin_noise` and the
reflectance programs. Drivers inline every call, and extra sites multiplied the
cold pipeline build from seconds to minutes.

On adapters supporting texture binding arrays and nonuniform sampled-image
indices, the renderer keeps each image's native dimensions and mip chain.
Other adapters use a 1024×1024 mipmapped 2D array. Both paths use linear
RGBA16F, at most 64 images, and a 128 MiB image budget. The fallback budget
allows 12 full mip chains. Later images are replaced by their scaled linear
average and reported in `lit_notes`. sRGB, linear, and `gamma N` encodings
are decoded on the CPU. Float RGBA images use alpha for every pixel when
any pixel has nonopaque alpha, matching r4.

The v4 scale node takes `tex` and a float `scale`; legacy `tex1`/`tex2`
forms are handled explicitly.

Image-based `infinite` lights arrive as `light/{index}` blobs. They bake into
one world-space cube map (faces of 512², `Rgba16Float`) for the background, and
one set of l = 2 spherical harmonics for diffuse irradiance, with a single
`LIGHT_ENVIRONMENT` entry in the light list. Specular is a split sum: the cube
prefiltered for GGX times the LTC table's directional albedo, with
`level = sqrt(alpha) * 5`. They cast no shadows apart from the screen-space
AO, and a `portal` is ignored, both
with a note.

`debug_ao` (capture requests, the capture daemon, `pbrt-ui-capture ... debug_ao`,
`pbrt-ui-ctl project capture --debug-ao`) returns the AO buffer as gray,
encoded like any color with the sRGB transfer function. It excludes
`debug_albedo`.

`debug_albedo` is available through capture requests, the resident capture
daemon, `pbrt-ui-capture`, and `pbrt-ui-ctl project capture --debug-albedo`.
It returns diffuse reflectance without lighting or film exposure, encoded
with the sRGB display transfer function. The ignored Vulkan tests cover
VM execution, image layers beyond eight, both image resource paths, and
wrapping. `tests/lit_texture_reference/` holds pinned r4 imagemap/scale/mix
references compared per pixel in linear space with fixed geometric masks.
See its README for commands, tolerances, and sampling error.

## Checking against r4

`tests/lit_reference/` holds scenes and the PNGs r4 rendered from them. The
README there says how the PNGs were made.

`tests/lit_reference.rs` compares Lit with those PNGs, region by region. Run
it with:

`cargo test -p scene-view -- --ignored --test-threads=1`

The references are direct lighting only (`--maxdepth 1`), and each region
must match to 15%.

Never widen the tolerances to make a failing region pass. Fix the
implementation or the scene instead.
