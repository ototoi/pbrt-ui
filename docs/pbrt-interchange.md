# PBRT import and export

How a `.pbrt` scene becomes a pbrt-ui project, and how a project becomes a `.pbrt` scene again.
The code is in `crates/pbrt-io`.

## Import

`import_project` reads a PBRT v4 scene, including gzip files, `Include` and `Import`. It builds
a project from it: scenes, nodes, resources and assets.

- **Sources stay where they are.** The scene text is not copied into the project. Each node
  records where it came from in `Imports/<id>/import.json` (see
  [project-folder.md](project-folder.md#imports)). Files the scene refers to are copied into
  `Assets/`: PLY meshes, images, spectra, measured BRDFs and lens files.
- **Nothing is written next to the source.** A failed or cancelled import leaves no partial
  project behind, and an existing destination is never replaced.

### Skip and warn

Input that cannot be interpreted is skipped with a warning, and the rest of the scene still
imports. Examples are an unknown directive, an unknown implementation, a missing file, or a
reference to a name that was never defined.

- **Scope-opening directives:** if one (`AttributeBegin`, `TransformBegin`, `ObjectBegin`) is
  skipped, its whole scope is skipped with it. The warning says how many directives that was.
- **Partial work is undone:** anything the skipped directive had added, such as copied assets,
  is removed again.
- **What stops an import:** only cancellation, I/O errors and the size budgets.

The warnings are shown in the Console after an import, and they are kept in the import record.

### How PBRT maps onto the project

| PBRT | Project |
|---|---|
| The world block | A scene, under `Scenes/` |
| `AttributeBegin` / `TransformBegin` | Group nodes. Groups are named after the first material, instance or PLY inside them |
| `Shape`, `LightSource`, `Camera` | Shape, light and camera nodes |
| `AreaLightSource` | An Area Light component on the group whose shapes emit |
| `Material`, `MakeNamedMaterial` | `.material` resources. A node binds its material by UUID |
| `Texture` | `.texture` resources. An `imagemap` refers to an image asset by UUID, so textures can share one image |
| `MakeNamedMedium`, `MediumInterface` | `.medium` resources and medium bindings |
| `ObjectBegin` / `ObjectInstance` | An object definition (`.object`) and instance nodes that place it |
| Large inline arrays (more than 4096 values) | `.array` assets under `Data/` |

- **Transforms.** A node's transform is its transform relative to its parent node, so the
  Hierarchy parent and the transform parent are always the same. `Identity`, which resets the
  transform to the world origin, is resolved during import into an equivalent local
  transform. Animated transforms (`ActiveTransform`, `TransformTimes`) keep their start and
  end matrices.
- **Coordinate system.** The importer infers the scene's up axis and handedness from the
  render camera. It reports the result as the `scene_coordinate_system` diagnostic and in the
  Scene inspector. The Create menu orients new objects to match.
- **Dialect.** Import follows pbrt-r4's reading of the format where pbrt-r4 and pbrt-v4
  differ. The parameter tables are transcribed from pbrt-r4, and pbrt-r4 wins where the two
  disagree.

### Budgets

| What | Default |
|---|---|
| One source file | 128 MiB |
| All expanded source text | 128 MiB |
| Bytes copied into the project | 8 GiB |
| `Include` / `Import` depth | 64 |
| Tokens in one directive | Large enough for big volume grids (millions of values) |

## Export

File > Export as PBRT… (`project.export`) writes a new folder:

```
exported/
  scene.pbrt     the scene, written from the project
  assets/        every file the scene refers to
  export.json    a manifest: which asset is which file
```

- **The folder is self-contained.** It never refers back to the original import's files, and
  the destination must not exist yet.
- **Film output.** The Film's output file is set to `render.exr`. The project keeps its own
  declaration.
- **Names.** PBRT names for materials, textures and media are made from the item labels.
  - `"`, `\` and control characters become `_`.
  - Duplicate names within one kind get `_1`, `_2`, and so on.
  - The names are chosen per export and are not stored in the project.
- **Order.** Resources that name others, such as a mix material and its two materials, are
  written after them, because PBRT resolves names as it reads.
- **Missing files.** An export that would need a file the project does not have is refused
  with the name of the item, instead of writing a scene that cannot render.

The same writer feeds the renderer. Render Scene and Render View export a snapshot into a
private bundle in `Library/Jobs` and run pbrt-r4 there.
