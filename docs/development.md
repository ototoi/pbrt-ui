# Development

How to build, test and package pbrt-ui from source. For using the app, see the
[README](../README.md). For controlling it from scripts, see [control.md](control.md).

See also [architecture.md](architecture.md), [project-folder.md](project-folder.md) and
[pbrt-interchange.md](pbrt-interchange.md).

## Workspace layout

This repository is one Cargo workspace (`Cargo.toml`):

| Path | Contents |
|---|---|
| `crates/protocol/` | Control protocol v2 types: methods, parameters, errors and limits |
| `crates/project/` | The persistent project model and folder store: `.meta` files, the cache and undo history ([README](../crates/project/README.md)) |
| `crates/pbrt-io/` | PBRT import and export against the project model ([README](../crates/pbrt-io/README.md)) |
| `crates/core/` | The host state machine and runtime (`pbrt-ui-host`): previews, jobs and the socket server |
| `crates/scene-parse/` | PBRT parsing, the preview manifest, tone mapping and HDR decoding |
| `crates/scene-view/` | The wgpu viewport and Lit preview (`pbrt-ui-capture`). The WGSL shaders are in `src/preview/` |
| `crates/desktop/` | The GPUI desktop app (`pbrt-ui`). `src/editor/` holds the panels, `src/backend/` the display thread and the core bridge |
| `crates/cli/`, `crates/mcp/` | `pbrt-ui-ctl` and `pbrt-ui-mcp` |
| `tools/xtask/` | `cargo xtask dist` (packaging) |
| `tools/fit-ltc/` | Offline LTC table fitting ([README](../tools/fit-ltc/README.md)) |
| `schemas/` | The generated control schema |
| `scripts/` | Smoke tests |
| `tests/fixtures/` | Shared scene fixtures |

pbrt-r4 is an external program. No renderer library is linked into the workspace. pbrt-ui owns
parsing, metadata and preview geometry, and it never overwrites source scenes.

## Build and run

Install the platform requirements from the [README](../README.md#install), then:

```sh
cargo build --locked -p desktop -p core -p cli -p mcp -p scene-view
cargo run -p desktop -- \
  --renderer-path /absolute/path/to/pbrt-r4 \
  --scene "$PWD/tests/fixtures/render-preview.pbrt"
```

The fixture scenes:

- `editor-preview.pbrt` exercises the primitives.
- `instances-preview.pbrt` shares meshes across placements.
- `render-preview.pbrt` includes lighting for a small final render.

On Windows, the viewport uses Vulkan on a discrete GPU and DX12 otherwise. `WGPU_BACKEND`,
`WGPU_POWER_PREF` and `WGPU_DX12_COMPILER=fxc` override the choice. wgpu links the DirectX
shader compiler statically, which is why MSVC 14.40+ and the ATL component are needed.

## Tests

```sh
cargo fmt --all
cargo clippy --workspace --all-targets
cargo test --workspace
cargo test -p scene-view -- --ignored --test-threads=1     # GPU tests, run serially
python3 scripts/generate-control-schema.py --check         # after protocol changes
```

Run the GPU tests before changing preview rendering. The Lit preview's shader structure and
reference tests are described in
[crates/scene-view/src/preview/lit/README.md](../crates/scene-view/src/preview/lit/README.md). To see the timing of a cached render at
1280x720 with 100k triangles, run
`cargo test -p scene-view --test preview_depth -- --ignored --nocapture`.

On Windows, a few tests that need Unix socket semantics or `python3` are skipped.

The smoke tests drive the built binaries. Run `cargo build -p core -p cli -p mcp` first:

```sh
python3 scripts/project-control-smoke.py
python3 scripts/mcp-smoke.py
python3 scripts/project-render-control-smoke.py
python3 scripts/project-render-smoke.py /absolute/path/to/pbrt-r4
python3 scripts/distribution-smoke.py target/dist
```

## Packaging

The same command builds the release folder on Linux, macOS and Windows:

```sh
cargo xtask dist --bundle-r4 /absolute/output-directory    # build and bundle pbrt-r4
cargo xtask dist /absolute/output-directory /path/to/pbrt-r4  # bundle an existing binary
cargo xtask dist --dev                                      # debug build into target/dist
```

- **Contents:** it installs `pbrt-ui`, `pbrt-ui-host`, `pbrt-ui-ctl`, `pbrt-ui-capture` and
  `pbrt-ui-mcp` together, with `.exe` on Windows. It also copies `README.md` and `LICENSE`.
- **Discovery:** the host finds the capture adapter and pbrt-r4 beside its own executable.
  `--renderer-path` overrides that.
- **Bundling pbrt-r4:** `--bundle-r4` builds it with
  `cargo install pbrt-r4 --version =<R4_VERSION> --locked` into `target/pbrt-r4-<version>/`.
  - That uses pbrt-r4's own dependency resolution, not this workspace's. Its exact pins conflict
    with the UI's.
  - It copies the binary beside the others and adds its license as `LICENSE-pbrt-r4`.
  - The version is `R4_VERSION` in `tools/xtask/src/main.rs`.
- **Without `--bundle-r4`:** the UI build never downloads or builds the renderer.
- **Target machines:** Linux still needs the shared libraries and a Vulkan driver.

## Viewport notes

- **Mesh buffers:** the viewport caches object-space vertex and triangle/wireframe index
  buffers per mesh, and shares them across placements.
  - Camera, mode and animation-time changes reuse those buffers and update only the view and
    instance data.
  - A changed mesh invalidates only its own cache entries.
- **Render targets:** they and the readback buffers are reused until the viewport size changes.
- **Frames:** the desktop keeps only the latest completed viewport frame.
- **Animation:** animated transforms use PBRT-style translation, quaternion rotation and scale
  interpolation with 61 shutter samples. The Scene toolbar slider selects the viewport and
  capture time.

### Preview limits

| What | Limit |
|---|---|
| Loop subdivision | Five levels and one million output triangles |
| Bilinear patches | An 8×8 tessellation. Indices may be omitted for sequential groups of four control points |
| Curves | Swept surfaces with 24 samples per segment |
| Paraboloids and hyperboloids | 64×32 tessellations |
| Height fields | One million vertices |
| NURBS | At most 64 dice per axis |
| Source, PLY file or preview package | 128 MiB each |

- **Beyond the limits:** a source or asset over its limit fails with a resource-limit error.
  Geometry beyond the package limit is left out, with a partial-preview diagnostic.
- **What is supported:** includes and imports expand with their source locations and the
  call-site graphics state. Gzip scenes and gzip PLY files are supported.
- **Out of scope:**
  - non-decomposable animation;
  - GPU zero-copy display;
  - keeping artifacts after the host restarts.
