# Project folder

What a pbrt-ui project looks like on disk, and where pbrt-ui keeps its temporary files.

- **File formats:** the version strings and how older versions are migrated are in
  [project-format-three.md](project-format-three.md).
- **Code:** `crates/project/src/store/` and `crates/project/src/layout.rs`.

## Layout

```
my-project/
  project.pbrtui.json      the project manifest
  Assets/                  everything the user sees in Contents
    Scenes/room.scene      + room.scene.meta
    Materials/gold.material
    Textures/wood.texture
    Images/wood.png
    Meshes/teapot.ply
    ...
  Imports/<id>/import.json one record per PBRT import
  Library/                 rebuildable and per-process files
    Cache/                 thumbnails and previews
    Staging/  Jobs/  Preview/
  .pbrtui/                 the lock and the save journal
```

### `project.pbrtui.json`

The manifest names the format (`pbrt-ui.project/3`), the project UUID and the title. It also
holds these fields:

- **`default_scene`:** the scene to open first, or `null`.
- **`saved_revision`:** the revision of the last save.
- **`importer`:** for imported projects, the importer version and the renderer revision it
  targeted.

### `Assets/`

Every item in `Assets/` has a sidecar beside it: the item's name plus `.meta`. Contents hides
the sidecars. A sidecar is JSON with `"format": "pbrt-ui.meta/2"` and a `kind`:

| `kind` | Identity | Items | Extra fields |
|---|---|---|---|
| `scene` | SceneId | `.scene` | — |
| `resource` | ResourceId | `.material`, `.texture`, `.medium`, `.object`, `.spectrum` | — |
| `asset` | AssetId | Images, `.ply`, `.array` and other files | `media_type`, `sha256`, `size`, `origin` |

- **References use UUIDs.** A material that uses a texture names the texture's UUID, not its
  path. That is why items can be moved and renamed freely in Contents.
- **Deleting a used item is refused.** For example, an image that a texture uses cannot be
  deleted.
- **Unknown fields are refused.** A sidecar or body with a field the reader does not know is
  an error, and so is a newer format version. Nothing is silently dropped.

#### Default folders

New items and imports go to these folders. Users may move items anywhere under `Assets/`.

| Item | Folder | Extension |
|---|---|---|
| Scene | `Scenes/` | `.scene` |
| Material | `Materials/` | `.material` |
| Texture | `Textures/` | `.texture` |
| Medium | `Media/` | `.medium` |
| Object definition | `Objects/` | `.object` |
| Spectrum data | `Spectra/` | `.spectrum` |
| Image | `Images/` | as imported |
| PLY mesh | `Meshes/` | `.ply` |
| SPD spectrum file | `Spectra/` | as imported |
| Large parameter array | `Data/` | `.array` |
| Other referenced file (measured BRDFs, lens files) | `Other/` | as imported |

#### Names and locations

A location is a path relative to `Assets/`, separated by `/`. The store refuses:

- absolute paths;
- empty parts, `.` and `..`;
- backslashes and colons.

Two items in the same folder cannot have names that differ only in case.

Names made from labels, by imports and by New in Contents, follow `layout::sanitize`:

- **Unsafe characters:** `/ \ : * ? " < > |` and control characters become `_`.
- **Trimming:** surrounding spaces and trailing dots are removed.
- **Empty names:** a name left empty becomes the kind's fallback stem, such as `material` or
  `texture`.
- **Collisions:** the name becomes `name_1`, `name_2` and so on, ignoring case.

When the user renames or moves an item onto a name that is already taken, the request is
refused rather than renumbered.

### `Imports/`

Each PBRT import leaves `Imports/<id>/import.json` (`pbrt-ui.import/1`). It records:

- the importer version and dialect;
- the source files it read and their byte spans;
- the diagnostics it produced.

Scene nodes keep their origin here, so the Inspector can show "source line N". The source files
themselves are not copied into the project, and pbrt-ui never writes to them.

### `Library/`

`Library/` is not project content. Save As does not copy it, and deleting it loses nothing that
pbrt-ui cannot rebuild.

| Folder | What it holds | Lifetime |
|---|---|---|
| `Cache/` | Contents thumbnails and material/texture previews (`Thumbnails/`, `Previews/`, index `cache.json`) | Kept between runs. Least recently used entries are evicted past 1 GiB |
| `Staging/` | Copies or hard links of asset bytes that undo, snapshots and imports still need | Per process |
| `Jobs/` | Render and capture job folders: the exported bundle, logs and images | Per process |
| `Preview/` | The desktop's staged preview bundles | Per process |

The per-process folders are named `<kind><pid>-<random>` and are created with mode 0700.

- **Normal exit:** the process removes them.
- **Crash or kill:** each process holds an owner lock per folder (`pbrt-ui-stage-<pid>.owner`).
  The next process to use that folder removes the folders of any pid whose lock is free.
  - Old names made of random characters only are never removed.
  - If a removal fails, the owner file stays so a later process can retry.

### `.pbrtui/`

`.pbrtui/` holds the process lock (`lock`, `lock-state`) and the save journal
(`save-journal.json`).

- **The lock:** only one process can have a project open.
- **Saving:** a save writes only the changed files. It goes through the journal, so an
  interrupted save is recovered the next time the project opens.
- **Outside changes:** a save refuses files that were changed by another program since they
  were loaded.

## Outside a project

When no project is open, as in a Viewer or before a project exists, the same per-process
folders go to the user cache folder:

| Platform | Cache folder |
|---|---|
| Linux | `$XDG_CACHE_HOME/pbrt-ui`, or `~/.cache/pbrt-ui` |
| macOS | `~/Library/Caches/pbrt-ui` |
| Windows | `%LOCALAPPDATA%\pbrt-ui` |

A Viewer's temporary project lives there as `pbrt-ui-viewer-<pid>-*`. Nothing is written next to
the opened `.pbrt` file. Desktop preview slots go to `<cache>/Library/Preview/`.

The desktop's own settings are in the config folder:

- `layout.json`, the dock layout;
- `recent.json`, the recent projects;
- `inspector.json`, which components are collapsed.
 That folder is `$XDG_CONFIG_HOME/pbrt-ui` or `~/.config/pbrt-ui` on Linux, and
`~/Library/Application Support/pbrt-ui` on macOS.

## Limits

| What | Limit |
|---|---|
| One JSON file (manifest, scene or resource body) | 64 MiB |
| One sidecar | 1 MiB |
| Nodes, resources or assets in a project | 250,000 each |
| Scope depth | 256 |

Going over a limit is an explicit error, never silent truncation.
