# pbrt-io

PBRT import and export for pbrt-ui projects.

- `import_project` / `import_into` read a PBRT v4 scene into a project. Input that cannot be
  interpreted is skipped with a warning.
- `export_project` writes a project snapshot as a self-contained folder with `scene.pbrt`,
  `assets/` and `export.json`.
- `parameter_schema.rs` holds the per-implementation parameter tables, transcribed from
  pbrt-r4. They inform the editors and never restrict what a scene may contain.

The behaviour is described in [docs/pbrt-interchange.md](../../docs/pbrt-interchange.md).

```sh
cargo test -p pbrt-io
python3 scripts/project-render-smoke.py /absolute/path/to/pbrt-r4
```

The render smoke test needs numpy, OpenEXR and Pillow. It renders a fixture before and after a
round trip through the project, and compares the EXR pixels.
