# Architecture

How the parts of pbrt-ui fit together at run time. For the crate list see
[development.md](development.md). For the files on disk see
[project-folder.md](project-folder.md).

## Processes

```
pbrt-ui (desktop)                         pbrt-ui-host (no window)
 ├─ GPUI window and panels                 └─ core
 ├─ display thread  ── requests ──┐
 ├─ GPU thread (scene-view, wgpu) │
 └─ core  ◄───────────────────────┘
      ▲
      │ control socket (AF_UNIX, line JSON, protocol v2)
      ├── pbrt-ui-ctl
      └── pbrt-ui-mcp ── stdio ── MCP client

core ──spawns──► pbrt-ui-capture   (resident capture adapter: capture and pick)
core ──spawns──► pbrt-r4           (one process per render; progress over loopback Tev)
```

- **One core per process.** The desktop links the core in-process; `pbrt-ui-host` is the
  same core without a window. GUI requests and socket requests enter the same queue, so the
  CLI, MCP and the window always see the same project, revisions, view, settings and jobs.
- **The renderer is a separate program.** pbrt-r4 is never linked. A render exports a
  snapshot of the project to a private bundle and runs `pbrt-r4` on it. On Windows renderers
  run in a kill-on-close Job Object; on Linux they get `PR_SET_PDEATHSIG`.
- **The viewport renders in-process.** The desktop draws its viewport with `scene-view` on
  its own GPU thread. `pbrt-ui-capture` serves the same renderer to the core for
  `project.capture` and `project.pick`, so a windowless host can capture too. It stays
  resident after the first capture and falls back to one-shot runs if it breaks.

## Inside the core

The core is one serial state machine (`crates/core/src/state.rs`). It owns the open project
session and never blocks on I/O; slow work goes to executors and comes back as a completion.

| Thread | Work |
|---|---|
| Core driver | Takes one request at a time, applies edits, answers queries, starts jobs |
| Project I/O | Open, save, Save As, import, export, preview bundles. Holds the store locks |
| Capture executor | Captures and picks |
| Render executor | Renders, so a render never blocks a capture |
| Preview workers (2) | Inspector, node editor and Contents previews |

The command queue is bounded. When it is full, a request gets `busy` instead of waiting.

## Projects, revisions and snapshots

- **Activation.** Opening or importing a project creates an *activation*. Every request names
  the project reference it expects (`project_id` and activation). A request with an old
  reference is refused with `stale_project`, so a script can never edit a project that was
  replaced under it.
- **Revisions.** Every committed edit raises the project revision. Requests that depend on
  state, such as `project.apply`, carry the revision they were based on and are refused if it
  moved.
- **Transactions.** `project.apply` takes a list of edits and commits them all or none. A
  failed batch leaves values, revisions and history unchanged.
- **Undo history.** One shared history covers edits, imports, moves, renames and deletions.
  The last 128 operations are kept in memory; the history is not saved with the project.
- **Snapshots.** Save, export, preview and render work on an immutable snapshot taken when the
  job is issued. Editing continues while they run, and the result reports the revision it was
  made from. A save marks only that revision as saved.
- **Payload leases.** Asset bytes that undo or a running job may still need are staged under
  `Library/Staging` and reference-counted. A deleted image can come back with Undo even
  after its file is gone, including after Save As and deleting the old folder.

## Viewer and project

A `.pbrt` file opens as a *Viewer*: the scene is imported into a temporary project in the
user cache folder and marked read-only. Saving it asks for a folder and turns it into a
normal project. The source scene is never written.

A project may have no open scene. Closing a scene keeps edits and history; render, export
and node queries need an open scene.

## Desktop

The desktop (`crates/desktop`) has three kinds of threads:

- **GPUI:** the window, panels and input. It never waits on the core.
- **Display thread** (`src/backend/`): the bridge to the core. It submits requests, polls
  completions, stages previews and keeps the latest frame.
- **GPU thread:** runs `scene-view` for the viewport and readback.

Project edits, saves, renders and captures from the GUI use the same control methods as the
CLI, so they can also be scripted.
