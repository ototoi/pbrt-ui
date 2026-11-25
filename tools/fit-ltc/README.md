# fit-ltc

This tool fits the Linearly Transformed Cosines (Heitz et al. 2016) table for
the GGX BRDF, which Lit's specular shading reads. It was restored from the
old implementation (`bd3edc3^:legacy/src/ltc/`), keeping only what GGX needs.

```sh
cargo run --release -p fit-ltc -- --brdf ggx --output crates/scene-view/src/preview/lit/ltc_ggx.bin
```

The fit takes about 80 seconds.

## Output

The output is `LTC1`, a `u32` size (64), then two RGBA `f32` layers of
64×64:

- Layer 0 holds the inverse matrix, packed by `pack_tab`.
- Layer 1 holds the magnitude, the Fresnel term, 0, and the sphere-clipping
  term.

The table axes are `sqrt(1 - cosθ)` across and `sqrt(α)` down (see
`fit_tab`).

## Checking the output

`crates/scene-view/src/preview/lit/ltc.rs` compares the output with the
published fit (selfshadow/ltc_code), at chosen entries:

- Layer 0 is a numerical fit and matches to 0.03 absolute. The median
  relative difference is 0.01%.
- Layer 1 is an integral and matches to 1e-3 relative.
