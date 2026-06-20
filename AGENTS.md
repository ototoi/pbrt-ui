# Repository Guidelines

## Project Structure & Module Organization
`src/` contains the application code. Key areas are `src/panel/` for egui panels and inspectors, `src/render/` for WGPU rendering, `src/io/pbrt/` for PBRT import/export, `src/model/` for scene data, and `src/preprocessor/` for the built-in C-like preprocessor. Entry points live in `src/bin/` (`pbrt-ui`, `fit-ltc`, `create-uuid`). Integration tests are in `tests/`, runnable examples are in `examples/`, shader assets are in `assets/shaders/`, and design notes live in `docs/`.

## Build, Test, and Development Commands
Use Cargo for all common workflows:

- `cargo run --bin pbrt-ui` launches the desktop UI.
- `cargo build --release` builds optimized binaries.
- `cargo test` runs the full test suite.
- `cargo test preprocessor` runs the focused preprocessor tests documented in `src/preprocessor/README.md`.
- `cargo test --test shader_validation` validates WGSL shader preprocessing and parsing.
- `cargo fmt` formats Rust code.
- `cargo clippy --all-targets` checks for common Rust issues.

## Coding Style & Naming Conventions
Follow standard Rust formatting with 4-space indentation and `cargo fmt`. Use `snake_case` for modules, files, functions, and tests; `CamelCase` for types and traits; `SCREAMING_SNAKE_CASE` for constants. Keep modules narrowly scoped and colocate related helpers, such as renderer-specific code under `src/render/wgpu/`. Prefer descriptive filenames like `render_session.rs` or `from_trianglemesh.rs` over abbreviations.

## Testing Guidelines
Add integration tests under `tests/` for user-visible behavior and parser/import/export regressions. Keep small unit tests near the implementation when the scope is local, as in `src/preprocessor/tests.rs`. Name tests by behavior, for example `test_all_shaders_compile` or `pbrt_roundtrip_test`. Run targeted tests before submitting changes that touch shaders, parsing, or scene conversion.

## Commit & Pull Request Guidelines
Recent commits use short, imperative subjects such as `Stabilize CSM visibility and add debug tracing`. Keep the first line focused on the behavior change. Pull requests should explain the motivation, summarize risky areas, and list validation steps. Include screenshots or short recordings for UI or rendering changes, and note any PBRT scene or shader assets needed to reproduce the result.

## Configuration & Assets
Treat shader files in `assets/shaders/` as source code: update includes carefully and rerun shader validation after edits. Do not commit generated `target/` output or machine-specific cache artifacts.
