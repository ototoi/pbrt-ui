# project

The editable project model and its folder store. It has no GPUI, GPU or renderer
dependencies.

- `ProjectDocument` holds the scenes, nodes, resources and assets, all with stable UUIDs.
- `ProjectSession` applies revision-checked edit transactions and keeps the undo/redo history
  (128 operations, in memory).
- `ProjectSnapshot` is immutable and `Send + Sync`, so saves, exports and renders run on a
  snapshot while editing continues.
- `store/` reads and writes the project folder (format 3), with a save journal for recovery.
  It opens formats 1 and 2 read-only and converts them with Save As.
- `presets.rs` holds the built-in Create menu presets, each with a fixed UUID.

See [docs/architecture.md](../../docs/architecture.md),
[docs/project-folder.md](../../docs/project-folder.md) and
[docs/project-format-three.md](../../docs/project-format-three.md).

```sh
cargo test -p project
```
