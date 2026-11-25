# Lit AO reference images

`lit_ao_reference.rs` compares Lit's screen-space AO with r4's visibility.
Each scene is diffuse with reflectance 0.5 under a uniform `infinite` light
of radiance 1, rendered with direct lighting only, so a pixel shows
`0.5 · V`: twice its linear value is the cosine-weighted visibility that
the AO approximates.

```sh
for f in *.pbrt; do pbrt-r4 --spp 1024 --quiet --maxdepth 1 -o "${f%.pbrt}.png" "$f"; done
```

Renderer: `../pbrt-r4/target/release/pbrt-r4`, r4 commit `211eb9e` (4.5.2).

| scene | what it checks |
|---|---|
| `ao_contact` | a sphere on the floor |
| `ao_corner` | two walls and the floor meeting |
| `ao_stack` | stacked boxes: creases and occlusion inside the screen |
| `ao_alpha` | a cutout standing on the floor: its holes do not occlude |
| `ao_glossy_rough`, `ao_glossy_mid` | conductor floors (roughness 0.4, 0.15) under a diffuse sphere |
| `ao_coated` | a coateddiffuse floor under a diffuse sphere |
| `ao_glass` | a glass sphere on the floor: r4 darkens the floor, Lit does not |

`lit_ao_matches_r4_visibility_per_tile` compares the AO buffer with twice
r4's linear value in 8×8 tiles that lie wholly on geometry.
`lit_matches_r4_under_a_uniform_sky_per_tile` compares Lit's own color with
r4's, which also covers the metal and coated floors. Budgets are per scene,
from the measured error plus a margin.

## Specular occlusion

Lit's mean tile error on the metal and coated floors (rough / mid / coated):

| specular | errors |
|---|---|
| AO off | 0.083 / 0.106 / 0.092 |
| not occluded | 0.070 / 0.093 / 0.056 |
| Lagarde, roughness √α | 0.037 / 0.061 / 0.052 |
| Lagarde, roughness α | 0.037 / 0.061 / 0.052 |
| AO | 0.037 / 0.061 / 0.052 |

Lagarde's form stays within 0.001 of plain AO here, so Lit scales the
environment's specular and coat by the AO itself.

## Radius

The radius is a fraction of the scene's diagonal. Mean tile errors
(contact / corner / stack / alpha):

| fraction | errors |
|---|---|
| AO off | 0.133 / 0.540 / 0.149 / 0.140 |
| 0.1 | 0.079 / 0.448 / 0.060 / 0.100 |
| 0.2 | 0.043 / 0.353 / 0.048 / 0.084 |
| 0.3 | 0.041 / 0.276 / 0.045 / 0.089 |
| 0.5 | 0.049 / 0.252 / 0.049 / 0.098 |
| 1.0 | 0.054 / 0.282 / 0.056 / 0.099 |

0.2 is used. Larger radii fit the corner better but darken the floor seen
through the cutout's holes by up to 0.4: screen-space AO cannot tell how
thick the nearer surface is.

## Known limits

- **`ao_corner`** stays far off. A vertical surface on an open floor loses
  half its sky to the floor however far it extends. r4 counts that, and the
  walls also continue off screen. A screen-space radius sees neither.
- Occluders outside the screen are not counted, so AO changes as the camera
  moves.
