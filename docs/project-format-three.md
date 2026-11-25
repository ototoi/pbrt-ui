# Project format 3

`pbrt-ui.project/3` uses `pbrt-ui.meta/2` sidecars. Current scene and resource
bodies use `pbrt-ui.scene/4` and `pbrt-ui.resource/2`; scene/4 adds the scene's
coordinate system (up axis and handedness). Readers retain support for scene/1
to scene/3 and resource/1. For the folder layout, see
[project-folder.md](project-folder.md). Unknown versions and mixed project/sidecar
versions are rejected. An empty project and a null default scene are valid; the
active scene is session state.

Scene and ObjectDefinition scopes may include `owned_object`, identifying an
internally generated Attribute scope's sole visible object child. This metadata
lets the editor hide the implementation wrapper while preserving its PBRT state
boundary. Older scene/resource bodies have no such metadata and load with no
owned object; the editor does not infer ownership from child count or labels.
Saving writes the latest body versions, so older application versions that only
understand scene/2 and resource/1 cannot reopen the saved files.

## Images and textures

An image is an Asset: the original file plus an `asset` sidecar with its
AssetId, media type, content hash/size and import provenance. An imagemap is an
ordinary Texture resource (`.texture` body plus `resource` sidecar) whose
`filename` references the image's AssetId. Several Textures may share one image,
so replacing the image's bytes updates every Texture that uses it. The schema is
defined by `crates/project/src/store/meta_v2.rs`; all kinds reject unknown fields.

| Sidecar kind | Identity | Contents body |
| --- | --- | --- |
| `scene` | SceneId | `.scene` |
| `resource` | ResourceId | `.material`, `.texture`, `.medium`, `.object` |
| `asset` | AssetId | Images, `.ply`, arrays and other data |

Every item has its own location. Images and Textures move, rename and delete
independently; UUID references survive move/rename, and an image referenced by
any Texture or other item cannot be deleted. Sidecars are hidden from Contents.

A PBRT import stores each referenced source image once under `Images/` and
creates one Texture per declaration under `Textures/`. A single image import
places only the Asset in the folder the request names (the folder is required);
it creates no Texture. A New-menu imagemap may exist without an image. While
nothing references it, export and direct preview leave it out; once referenced,
they refuse it by name until an image is chosen.

Changing a Texture's value type is one edit (`set_texture_value_type`). The core
checks texture-reference lineage across all scenes, definitions and resources
before committing. A mismatch returns `texture_type_mismatch` details, including
consumers, with no history mutation. Inspector and Node Editor share the editor.

## Compatibility and migration

Formats 1 and 2 open read-only. Explicit Save As builds a detached migration plan
and publishes a new format-3 directory after byte and reference validation. The
source project is never overwritten. A failed or cancelled conversion does not
activate a partial destination. A cancelled plan can be retried as is.

Conversion keeps every UUID and location. Images stay Assets and imagemap
Textures keep their `.texture` bodies and references; nothing is copied.
Unregistered source files cause an explicit diagnostic rather than being
silently omitted. Missing required bytes also prevent migration.

Save As completion adopts converted records only for a compatibility session
without edit history; ordinary editable Save As preserves history and concurrent
edits. Staging is prepared before switching the active store. Existing snapshots
keep their leases after relocation or session close.

## Save recovery and snapshots

The save journal includes the intended folder set. Its recovery record survives
publication until folder cleanup and readback/baseline validation finish. Failure
leaves the store requiring reopen/recovery; another write cannot proceed with an
old baseline. Existing journals without a folder set remain readable.

I/O snapshots pin current folder asset bytes before Save/export/preview/render
work is queued. Export, array expansion and preview staging resolve snapshot
leases before persisted UUID locations. Thumbnail previews also resolve by UUID
and decode staged payloads independently of their temporary file extension.
Snapshot pinning currently copies assets eagerly, so large projects incur staging
I/O when jobs are issued.

Behavior coverage lives in `format_three`, `meta_v2`, `migration_three`,
`snapshot_payload`, core project-runtime tests and the CLI/MCP control smoke.

Value-type changes also check literal parameter declarations against the
destination table. Snapshot pinning builds one persisted-asset UUID index per
snapshot. Journal replay synchronizes the nearest surviving directory when a
completed deletion's original parent has already been pruned.
