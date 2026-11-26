# Control from scripts and agents

The desktop app (`pbrt-ui`) and the windowless host (`pbrt-ui-host`) both run the same core.
That core serves a control socket. `pbrt-ui-ctl` (CLI) and `pbrt-ui-mcp` (MCP server) talk to it.

GUI and CLI share everything through that core:

- the project and its revisions;
- the project view and the render settings;
- jobs and their artifacts.

Requests that name an old project reference or revision are rejected. A failed open or import
keeps the current document. The control protocol v2 schema is in [`schemas/`](../schemas).

## The control socket

Every host prints its socket path when it starts.

- **Default location:** `$XDG_RUNTIME_DIR/pbrt-ui/<session>/control.sock`. Without
  `XDG_RUNTIME_DIR` (as on macOS), the host uses a private temporary folder instead.
- **Choosing a path:** pass `--endpoint PATH`. The path must be absolute.
  - It can be at most 103 bytes on macOS and 107 on Linux and Windows.
  - On Linux and macOS, its folder must be yours with mode 0700:
    `mkdir -p /tmp/pu && chmod 700 /tmp/pu`.
- **Left-over sockets:** a socket left by a host that did not exit cleanly is replaced. A live
  host's socket is refused. A `<endpoint>.lock` file beside the socket serializes this between
  hosts and stays in place.
- **Windows:** GUI, CLI and host use the same AF_UNIX socket as on Unix. The default socket is
  under the user's `%TEMP%`. The ACL of a folder you choose with `--endpoint` is not checked.

To run a host with no display:

```sh
pbrt-ui-host --renderer-path /absolute/path/to/pbrt-r4
```

## CLI

```sh
pbrt-ui-ctl --endpoint /path/control.sock project new /absolute/project --title "My scene" --wait
pbrt-ui-ctl --endpoint /path/control.sock project import /absolute/scene.pbrt /absolute/project --wait
pbrt-ui-ctl --endpoint /path/control.sock project nodes --view scene_graph
pbrt-ui-ctl --endpoint /path/control.sock project node NODE_ID
pbrt-ui-ctl --endpoint /path/control.sock project contents Materials
pbrt-ui-ctl --endpoint /path/control.sock project apply '[{"kind":"rename_node","node_id":"…","label":"Floor"}]'
pbrt-ui-ctl --endpoint /path/control.sock project capture --wait
pbrt-ui-ctl --endpoint /path/control.sock settings '{"samples_per_pixel":null,"threads":4,"max_depth":null}'
pbrt-ui-ctl --endpoint /path/control.sock project render --wait --timeout-ms 300000
pbrt-ui-ctl --endpoint /path/control.sock job JOB_ID
pbrt-ui-ctl --endpoint /path/control.sock cancel JOB_ID
pbrt-ui-ctl --endpoint /path/control.sock artifact ARTIFACT_ID
pbrt-ui-ctl --endpoint /path/control.sock export ARTIFACT_ID /absolute/output.exr --wait
pbrt-ui-ctl --endpoint /path/control.sock project save /absolute/project
pbrt-ui-ctl --endpoint /path/control.sock quit --discard-unsaved
```

`pbrt-ui-ctl --help` lists every command. These are also available:

- `jobs` and `events AFTER`;
- `raw METHOD PARAMS_JSON`, to call any method.

Responses have both count and byte limits. Follow `next_offset` / `next_cursor` to page through
them.

### Projects

- **`project new`** (`project.create`; also New Project… in the app and MCP `project_create`)
  builds an editable project. The folder must be new or empty. The first scene holds a render
  Camera (fov 40), a 10x10 Floor and a uniform Environment Light.
  - `--up +Z` and `--handedness right` set the coordinate system the scene is authored in. The
    defaults are `+Y` and `left`.
  - The Create menu orients its presets to match.
- **PBRT import:** a Z-up or right-handed PBRT import infers the coordinate system from the
  render camera. It reports the result in the import diagnostics and the Scene inspector.
- **Closing a scene:** `project close-scene` closes the scene and keeps your edits and the
  undo history. `project open-scene ID` opens it again.
  - Render, export and node queries need an open scene.
  - Deleting the open scene or the last scene can be undone, and does not open another one.

### Rendering and capture

- **`project render`** needs the current project reference and settings revision.
  - By default (`camera: "scene"`) the scene's own camera and Film render.
  - An optional `view` supplies the camera, size and shutter time.
  - `--camera view` with no view uses the shared project view. It is refused with
    `no_shared_view` when there is none.
  - Rendering does not need the original PBRT files.
- **`project view`** reads and sets the shared view (`project.get_view`, `set_camera`,
  `configure_view`).
  Setting its camera also moves the open desktop window's viewport.
  - Its options are `[JSON] [--mode solid|normal|wireframe|lit] [--size WxH] [--time T]`.
  - With no JSON and no flags it prints the view.
  - The JSON may be a camera, a view state, or a previous `project view` result. The flags
    override its fields.
- **`project capture`** (MCP `project_capture`) returns a fast approximate PNG without ray
  tracing.
  - It uses the optional `view` if one is given.
  - Without one, it uses the shared project view, or the scene camera at the Film's aspect
    ratio when there is no shared view.
  - `--camera scene` always uses the scene camera.
- **`project pick X Y`** (MCP `project_pick`) takes a normalized pixel (origin top-left) in the
  same view.
  - It returns the shape or instance node there, or `null` for the background.
  - Its `node` summary gives the kind, the label and the shape implementation.
- **`project apply EDITS --capture`** also starts a capture of the edited revision
  (`capture_job_id` in the result).
  - It is rejected before any edit if capture is unavailable.
  - Capture and pick need `pbrt-ui-capture` beside the host.
- **Render settings:** `settings` accepts `samples_per_pixel` from 1 through 1,048,576;
  null uses the scene's Sampler. `max_depth` must remain null and is controlled by the scene's
  Integrator. Threads remains a shared setting. IPR keeps its own sample budget.

### Jobs and artifacts

- **Where artifacts live:** jobs keep immutable artifacts in the host's temporary folder. Export
  anything you want to keep before the host exits.
- **Running jobs:** capture and render have separate executors, so a render does not block
  editing or capture.
- **Intermediate images:**
  - Tiles arrive over the renderer's Tev display-server connection on loopback.
  - A PNG snapshot is published when the image is created, then at 2 fps for the first 16 s,
    then less often (up to every 10 s). Only the latest snapshot is kept.
- **Progress:** progress reports job phases, intermediate images and the share of pixels the
  renderer has delivered. No percentage is estimated.
- **Logs:** they are kept as artifacts after the renderer exits, including for failed or
  cancelled renders.
- **Stopping the renderer:**
  - On Windows, renderers run in a kill-on-close Job Object, so they stop whenever the host
    exits.
  - On Linux they stop through `PR_SET_PDEATHSIG`.
  - On macOS a renderer keeps running if the host crashes. Quitting, closing the window and
    cancelling still stop it.

`quit` (`app.quit`) ends the host after the same unsaved-changes guard as `project.close`. This
host can be the desktop app or `pbrt-ui-host`. Nothing is saved.

## MCP

```json
{
  "mcpServers": {
    "pbrt-ui": {
      "command": "/absolute/path/pbrt-ui-mcp",
      "args": ["--endpoint", "/path/from/running/host/control.sock"]
    }
  }
}
```

The stdio bridge implements these MCP calls:

- `initialize`;
- `tools/list` and `tools/call`;
- `resources/list`, `resources/read` and artifact resource templates.

Tools use the same argument schemas and revision rules as the core. It provides two resources:

- `pbrt://state` returns the current state.
- `pbrt://artifact/ARTIFACT_ID` returns a binary artifact up to 4 MiB inline. Export larger
  artifacts instead.

Only MCP messages are written to stdout. The bridge follows the official
[stdio transport](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports) and
[tools contract](https://modelcontextprotocol.io/specification/2025-11-25/server/tools).
