# Repository Guidelines

## Project Structure & Module Organization
A Cargo workspace (`Cargo.toml`), with `README.md` as the user guide and `docs/development.md` for developers:

- `crates/protocol/`: control protocol v2 types (methods, params, errors, limits).
- `crates/project/`: the persistent project model and folder store (`.meta`, cache).
- `crates/pbrt-io/`: PBRT import/export against the project model.
- `crates/core/`: the host state machine and runtime (`pbrt-ui-host`); previews, jobs and the socket server.
- `crates/scene-parse/`: PBRT parsing, the preview manifest, tonemapping and HDR decode (`pbrt-ui-preview`).
- `crates/scene-view/`: the WGPU viewport and Lit preview (`pbrt-ui-capture`); WGSL shaders live in `src/preview/` and `src/preview/lit/`.
- `crates/desktop/`: the GPUI desktop app (`pbrt-ui`); `src/editor/` holds the panels, `src/backend/` the display thread and core bridge.
- `crates/cli/` (`pbrt-ui-ctl`) and `crates/mcp/` (`pbrt-ui-mcp`): control clients over the same protocol.
- `tools/fit-ltc/`: offline LTC table fitting. `schemas/`: the generated control schema. `scripts/`: build and smoke scripts. `tests/fixtures/`: shared scene fixtures.

## Build, Test, and Development Commands
- `cargo run -p desktop` launches the desktop UI; see `README.md` for launch options and `docs/control.md` for the endpoint.
- `cargo test --workspace` runs the test suite.
- `cargo test -p scene-view -- --ignored --test-threads=1` runs the GPU tests (needs a GPU); run them before changing preview rendering.
- `python3 scripts/generate-control-schema.py --check` verifies `schemas/` after protocol changes.
- `cargo fmt --all` formats; `cargo clippy --workspace --all-targets` must exit 0 without `-A` flags.

## Coding Style & Naming Conventions
Follow standard Rust formatting with 4-space indentation and `cargo fmt`. Use `snake_case` for modules, files, functions, and tests; `CamelCase` for types and traits; `SCREAMING_SNAKE_CASE` for constants. Keep modules narrowly scoped and prefer descriptive file names over abbreviations.

## Testing Guidelines
Keep unit tests next to the implementation; crate-level integration tests live in each crate's `tests/`. Name tests by behavior. Run the targeted crate's tests, plus the GPU tests for rendering changes, before submitting.

## Commit & Pull Request Guidelines
Commits use short, imperative subjects that name the behavior change, one fix per commit. Pull requests target `develop`, explain the motivation, summarize risky areas, and list validation steps. Include screenshots for UI or rendering changes.

## Configuration & Assets
Treat WGSL shaders as source code. Do not commit `target/` output or machine-specific cache artifacts.
