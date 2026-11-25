//! MCP stdio adapter. All operations address an explicitly selected core host.
use base64::Engine;
use cli::Client;
pub use protocol as proto;
use protocol::Method;
use serde_json::{Value, json};

pub const MCP_IMPLEMENTED: bool = true;
const METHODS: &[&str] = &[
    "app.get_state",
    "app.get_capabilities",
    "app.quit",
    "project.get_state",
    "project.import",
    "project.import_scene",
    "project.import_resource",
    "project.open",
    "project.create",
    "project.close",
    "project.save",
    "project.export",
    "project.render",
    "project.capture",
    "project.pick",
    "project.list_nodes",
    "project.list_contents",
    "project.list_references",
    "project.list_scenes",
    "project.open_scene",
    "project.close_scene",
    "project.list_presets",
    "project.search_nodes",
    "project.get_view",
    "project.set_camera",
    "project.configure_view",
    "project.get_node",
    "project.get_resource",
    "project.get_asset",
    "project.get_cache",
    "project.put_cache",
    "project.get_parameters",
    "project.get_parameter_values",
    "project.set_parameter_values",
    "project.get_parameter_schema",
    "project.plan_implementation",
    "project.get_resource_graph",
    "project.list_spectra",
    "project.spectrum_color",
    "project.preview_texture",
    "project.preview_material",
    "project.preview_asset",
    "project.apply",
    "project.undo",
    "project.redo",
    "render.get_settings",
    "render.set_settings",
    "render.ipr_set",
    "render.ipr_submit",
    "job.get",
    "job.list",
    "job.cancel",
    "events.read",
    "artifact.get",
    "artifact.export",
    "workspace.reset_layout",
];
/// Read-only methods never change project, scene or view state.
fn is_read_only(method: &str) -> bool {
    let name = method.rsplit('.').next().unwrap_or(method);
    name.starts_with("get_")
        || name.starts_with("list")
        || name.starts_with("search")
        || name == "spectrum_color"
        || matches!(name, "get" | "read" | "pick")
}
/// Per-tool guidance so a client can choose tools without reading the schema first.
fn describe(method: &str) -> String {
    let text = match method {
        "app.get_state" => {
            "Start here. Returns the project state, the settings and layout revisions, the event cursor and IPR status required by mutating tools."
        }
        "app.get_capabilities" => {
            "Lists which features this host has enabled (render, project editing, capture). Tools for disabled features are hidden."
        }
        "app.quit" => {
            "Ends the host (the desktop app or pbrt-ui-host) after a clean shutdown. Refuses unsaved edits unless discard_unsaved is true; nothing is saved."
        }
        "project.get_state" => {
            "Returns the open native project, its revisions and dirty/undo state. Call before any project mutation."
        }
        "project.import" => {
            "Imports a .pbrt scene as a read-only Viewer when destination is omitted, or into a saved Project when destination is supplied. Runs as a job; poll job_get. Source files are never modified."
        }
        "project.import_scene" => {
            "Adds a PBRT scene to the editable project as one undoable job. Does not open the scene or save the project."
        }
        "project.import_resource" => {
            "Adds an image or PLY mesh as one asset in the given folder (required) as one undoable job. Creates no Texture; does not place it in a scene or save the project."
        }
        "project.open" => {
            "Opens a native project at path. Needs expected_current; refuses to drop unsaved edits unless discard_unsaved is true."
        }
        "project.close" => {
            "Closes the current project. Needs expected_current; refuses unsaved edits unless discard_unsaved is true."
        }
        "project.create" => {
            "Creates a new editable folder project at path and opens it like project.open. The directory must not exist or must be an empty folder. title names the project and its initial scene (1-128 characters, no path separators). Optional coordinate_system {up: one of +X -X +Y -Y +Z -Z, handedness: left (pbrt's own convention) or right (right-handed original content)} sets the up axis and handedness the scene is authored around; each part defaults (+Y, left). The initial scene holds a render Camera (fov 40, over the origin), a 10x10 Floor and a uniform Environment Light. Runs as a job; poll job_get. Mutating: supply current revisions from app_get_state."
        }
        "project.save" => {
            "Saves a Project in place. For a Viewer, destination is required to create a Project folder; for a Project, destination optionally saves a copy and switches the active project. PBRT files must be created with project.export."
        }
        "project.export" => {
            "Exports a pinned project snapshot as PBRT files to destination. Runs as a job; poll job_get."
        }
        "project.render" => {
            "Renders a pinned project snapshot with pbrt-r4. Camera \"scene\" requires the scene's explicitly selected Render Settings camera (otherwise refused with reason no_render_camera); an explicit view overrides camera, size and shutter time, and camera \"view\" without one uses the shared project view (refused with reason no_shared_view when there is none). Samples and maximum depth come from the scene Sampler and Integrator; render settings accepts only null for samples_per_pixel and max_depth. Threads is shared, where null lets the renderer choose. Runs as a job; poll job_get, then artifact_get."
        }
        "project.capture" => {
            "Fast approximate raster preview (PNG) of a pinned project snapshot, no ray tracing. Optional view sets camera, size, mode (solid, normal, wireframe, lit) and time; omitted uses the scene camera. Optional camera \"scene\" frames the scene's own camera at the film's aspect ratio (long side 1280 pixels), keeping only the shared view's mode and time; camera \"view\" or an omitted camera keeps the shared view behavior, and an explicit view cannot be combined with camera. Set debug_albedo true to capture linear diffuse reflectance without lighting or film exposure for texture comparisons, or debug_ao true to capture the environment light's screen-space AO (white = unoccluded). Lit captures include lit_notes describing approximations. Use it to check an edit visually; runs as a job, then artifact_get."
        }
        "project.pick" => {
            "Finds which project node (shape or instance placement) appears at normalized_xy ([0,1] x [0,1], origin top-left) in a project_capture-style view. Result node_id is null for background or unmapped pixels; result.node summarizes it (kind, label, entity/implementation, current=false if the project changed since); pass node_id to project_get_node. Runs as a job; read result from job_get."
        }
        "project.list_nodes" => {
            "Pages through the project hierarchy. view \"directives\" (default) lists every node in PBRT order; view \"scene_graph\" lists objects (groups, shapes, lights, cameras, instances) under a Scene root whose node_id is the scene ID. The World scope is hidden and its objects appear directly under Scene; a Camera row lists the groups attached to it, and dropping onto that row attaches a group (move_node onto the camera). Read-only; use returned node IDs with project_get_node."
        }
        "project.list_scenes" => {
            "Lists project scenes with their locations, default/active state, and node counts. Read-only."
        }
        "project.list_presets" => {
            "Lists the built-in presets (group, shapes, lights, area lights, object instance) with their fixed UUIDs. Pass an id to project_apply as {\"kind\":\"create_preset\",\"preset\":id,\"parent\":group_id}; never look presets up by name. Read-only."
        }
        "project.close_scene" => {
            "Closes the active scene without discarding edits or clearing project undo/redo history. Resource operations remain available."
        }
        "project.open_scene" => {
            "Switches the active scene. Unsaved edits in other scenes remain in memory; the project revision and dirty state do not change. The active scene becomes the default for the next open."
        }
        "project.list_contents" => {
            "Lists one Contents folder at a time: subfolders first, then scenes, resources and assets with reference counts. Omit folder (or pass an empty string) for the Assets root; \"@builtin\" lists the built-in presets. Read-only."
        }
        "project.list_references" => {
            "Answers where one project item is used (the where-used list, G6). Each row names its source - a scene node (scene + node), an object definition's inner node (through the definition's resource), or a resource - plus what the reference is through: \"material\", \"medium inside\", \"medium outside\", \"instance\", or \"parameter:NAME\". Page with offset/limit; the delete refusal's details use the same rows. Read-only."
        }
        "project.get_view" => {
            "Reads the project's shared native view (camera, size, mode, time) with its view revision. View is null until established; captures then frame the scene camera."
        }
        "project.set_camera" => {
            "Sets the shared native view's camera under expected_view_revision; stale requests fail with revision_conflict. Later captures, renders and IPR without an explicit view reuse it."
        }
        "project.configure_view" => {
            "Adjusts size, display mode or time of the shared native view under expected_view_revision. None fields keep their value."
        }
        "project.search_nodes" => {
            "Searches project nodes by label substring (case-insensitive) across the whole tree. Each hit reports its ancestor path; page with offset/limit. view \"scene_graph\" matches only objects, omits the World scope from paths, and ends paths at the Scene root. Read-only."
        }
        "project.get_node" => {
            "Returns one node with its source location and components. Transform.local is the static local matrix (row-major 16) and TRS, or a reason they are unavailable; an animated local has matrix=null and samples {times, start, end}, each with its endpoint matrix/TRS or a reason and its local parent (root, group, or {\"camera\": id} when the group is attached to a camera; a camera's own local parent is root). Animated samples are read-only; local transform edits still require one static matrix. world_trs is the world pose (World = ParentWorld x Local; a camera's local/world pose is camera-to-world). Shapes and lights expose a two-sided medium component; the Camera uses that component kind with one binding from PBRT's outside field, while preserved_inside retains the ignored stored value for round-tripping. Scene root includes Render Settings with the explicit camera selector; the active Film is a Scene root component — between Render Settings and Sampler — whether or not a render camera is set, never on the Camera. path lists visible scene-graph ancestor IDs, nearest first; the World scope is omitted."
        }
        "project.get_resource" => "Returns a material/texture/etc. resource declaration.",
        "project.get_asset" => {
            "Returns a managed asset file: location, media type, size, SHA-256, import origin and how many objects use it. A PLY mesh also reports mesh {vertices, faces, normals, uvs, format} from its header (null for other assets). Read-only."
        }
        "project.get_cache" => {
            "Reads one entry of the project's regenerable cache (Library/Cache/): {hit, path, media_type} or {hit: false}. Callers read the path directly; entries vanish and regenerate freely."
        }
        "project.put_cache" => {
            "Writes one cache entry (kind thumbnail or preview, base64 bytes up to 4 MiB) into Library/Cache/ and returns its path for direct reads. The cache is regenerable, never part of the saved project. Does not edit the document or bump its revision."
        }
        "project.get_parameters" => "Pages through the parameters of a node or resource.",
        "project.set_parameter_values" => {
            "Replaces windows of one array parameter's values without changing its length: windows [{offset, values}] are disjoint, each 1..4096 values of the array's kind, and land in one undoable transaction. Every window is checked when the job is asked for. Runs as a job (a data asset over 4096 values is rewritten as a new asset on the I/O executor); poll job_get. An edit made meanwhile fails it as a revision conflict, and switching the active scene fails a node's edit with reason scene_changed. project_apply's set_parameter_values edit works on inline arrays only (reason data_asset_edit_is_a_job otherwise)."
        }
        "project.get_parameter_values" => {
            "Reads a window of one parameter's values (offset, limit up to 4096), whether kept inline or in a data asset; project_get_parameters previews only the first 32. total and next_offset page through the rest. Floats come as FloatValue objects, integers as decimal strings."
        }
        "project.get_parameter_schema" => {
            "Returns the r4 parameter table of one implementation: name, aliases, accepted types, default (value or what it derives from), accepted texture lineage, role and required, plus the covered implementations of the directive. Texture implementations take a texture_value_type of float or spectrum; implementations without a table return parameters: null while staying legal. Static information: no project needs to be open."
        }
        "project.get_resource_graph" => {
            "Returns the upstream graph of one material or texture: nodes (id, kind, label, implementation, texture_value_type, and the inputs that take a connection with what they accept: float, spectrum or material), edges {from, to, parameter} meaning to's parameter references from, breadth first from root. Only upstream (what the resource uses). limit caps the nodes (default 256); truncated says more were left out. Read-only."
        }
        "project.preview_texture" => {
            "Evaluates a texture (with its upstream textures and images, using r4's defaults) into a square preview PNG (size 8-512, default 128) in the project cache and returns {path, key, cached}. Read the path directly. The key follows every input, so cached: true means the preview is current. Point sampling on the plane p = (u, v, 0), n = +z, uv in [0, 1]; values clamped to [0, 1] and shown as sRGB. Does not edit the document."
        }
        "project.preview_material" => {
            "Shades a material (with its mix inputs, textures and images, using r4's defaults) onto a unit sphere under a fixed studio light and returns a square preview PNG (size 8-512, default 128) in the project cache as {path, key, cached}. Read the path directly. An approximation, not a PBRT render: Lambert, a Blinn-Phong highlight, r4's Fresnel per RGB channel, a sky gradient for reflections and a checker backdrop for transmission; measured, hair and subsurface draw as a grey stand-in. The key follows every input, so cached: true means the preview is current. Does not edit the document."
        }
        "project.preview_asset" => {
            "Decodes an image asset (png, jpeg, exr, hdr, tga; edges up to 8192) into a square preview PNG (size 8-512, default 128) in the project cache and returns {path, key, cached}. Read the path directly. The image is fit to the square and centered with transparent letterbox; exr and hdr pass Reinhard tone mapping into sRGB like the desktop Image Viewer, other formats keep their stored pixels. The key follows the asset's content, so cached: true means the preview is current. Other assets are refused. Does not edit the document."
        }
        "project.list_spectra" => {
            "Lists the builtin spectrum names (sorted) a spectrum parameter may name, as r4 knows them. Static information: no project needs to be open."
        }
        "project.spectrum_color" => {
            "Resolves one parameter's current spectrum-ish value (rgb, blackbody, a builtin name, a SpectrumData reference, or wavelength/value pairs) to the linear RGB a colour swatch shows, the same resolution the Lit preview shades lights with. Returns {rgb, source, role}: source is the human spelling ('blackbody 6500 K', a builtin name, 'SpectrumData: <label>', rgb triples or 'spectrum samples (N pairs)') and role is the interpretation used. role, when the request says one, is albedo, unbounded, illuminant, chromaticity (the curve's colour, magnitude-free) or conductor; otherwise the target's parameter table picks it, and a SpectrumData resource asked directly (parameter 'values' or 'wavelengths') defaults to its chromaticity. The conductor role resolves a conductor or coatedconductor material's Metal colour — the normal-incidence F0 of eta/k under the coat's interface index, a missing side filled with r4's copper ('k: default (Cu)'), or the reflectance's own colour; whichever metal row (eta, k, reflectance, conductor.eta, conductor.k) asks, the answer is the same, and r4's refusal of a reflectance beside eta/k answers grey with that warning. A glass name on eta/k reads the table's channel samples the way an rgb declaration does, and a name that is not a refractive index (an illuminant such as stdillum-D65, an unknown name) answers grey with a source saying 'is not a refractive index (not evaluated)' instead of an error — Lit and the material thumbnail shade the same grey. A name without table values reads white, as Lit does. Read-only."
        }
        "project.plan_implementation" => {
            "Previews set_implementation with keep_compatible on a node or resource without applying it: the normalized implementation, changed (false when it already is that one), kept_unknown (parameters kept although the new implementation's table does not know them) and breaks_reference (parameters the new implementation cannot export; set_implementation refuses until they are removed with set_parameter value null, which may share the same project_apply), and edits: the edit list that applies the change in one transaction — those removals, then the set_implementation — ready to pass to project_apply (empty when nothing changes). Read-only."
        }
        "project.apply" => {
            "Applies edit operations in one revision-checked transaction. Fails if expected revisions are stale; re-read state and retry. set_render_camera(scene, camera) sets the Scene's explicit Render Scene camera by node ID, or null to unset it; only a Camera in that same Scene is accepted. set_scene_coordinate_system(scene, up, handedness) sets the Scene's up axis (one of +X -X +Y -Y +Z -Z) and handedness (left, pbrt's own convention, or right, right-handed original content with a mirror in the camera chain); it changes display and creation metadata only — no node moves, exports write nothing, and it is undone as one step. set_parameter_values replaces a fixed-size window in an existing array without changing its length; for a Data Asset it creates a copy-on-write replacement, so the operation remains one Undo step. create_preset adds a preset from project_list_presets in its own group and returns the new node IDs in created; parent=Scene ID inserts into the visible root (the underlying World scope); view {position, look_at}, the viewport's eye and look-at point, places the new object for the camera, shape, light and area presets — the camera takes its pose from it, a shape, light or area body opens with a Translate to the look-at point (a shape also scales by a tenth of the eye's distance, so it stays in frame); without a view (or for the group and object instance presets, which take none) the body is created at the origin, unit sized, and the camera takes the scene's default placement. create_configuration adds one render entity to the Scene's Options in PBRT file order — configuration one of film, sampler, integrator, pixel_filter, accelerator; implementation omitted picks the kind's default (rgb, zsobol, volpath, gaussian, bvh) — delete_node removes such an Options node again. set_local_binding gives one object its own material/medium; set_instance_definition points an object instance at another object definition; connect_texture wires a texture (texture id; create {implementation, label} for a new one of the slot's value type; or image, an image asset id, which reuses an imagemap of that image in the slot's value type or creates one under Textures/) into a material's, texture's or shape's (alpha, shadowalpha) texture-input parameter and disconnect_texture removes the connection (refusals carry reason not_a_texture_input, texture_lineage, not_a_texture, not_an_image or texture_value_type; any edit that would make resources reference themselves is refused with reason resource_cycle); set_implementation changes an entity's, configuration's or resource's implementation keeping every parameter (preview with project_plan_implementation; the result's kept_unknown lists the kept parameters the new table does not know, and parameters the new implementation cannot export are refused with reason breaks_reference); delete_node, duplicate_node and move_node act on a scene-graph object together with its dedicated group (duplicate_node returns the copies in created; deleting a camera deletes the groups attached to it; move_node accepts a group, the World scope, or a Camera as parent — a camera parent attaches the unit to that camera, and any other parent clears the attachment). set_local_trs (translation, rotation_euler_degrees in Z→X→Y order, scale) and set_local_matrix (16 row-major numbers) set a group's or object's local transform as project_get_node reports it in transform.local. set_local_trs/set_local_matrix on a Camera rewrite its camera-to-world pose directly; a camera without a pose (an older import) and an animated camera are refused. local.parent \"root\" means the local is the world pose; {\"camera\": id} means it is relative to that camera's pose. A Viewer is read-only: use project_save with destination before editing. Set capture (optionally with view) to also start a project_capture of the edited revision; the result then holds capture_job_id."
        }
        "project.undo" => "Reverts the last applied project transaction.",
        "project.redo" => "Re-applies the last undone project transaction.",
        "render.get_settings" => "Returns the current render settings and their revision.",
        "render.set_settings" => {
            "Updates shared execution settings under the current settings revision. threads may be a positive integer or null. For compatibility, samples_per_pixel and max_depth remain nullable fields but must be null; explicit values are rejected because scene Sampler and Integrator configurations control final render quality. IPR retains its separate sample budget."
        }
        "render.ipr_set" => {
            "Turns interactive rendering (IPR) on or off and sets its settings (samples_per_pixel <= 256, debounce_ms 10-5000). While on, committed project edits, camera changes and re-submits automatically launch low-sample renders of the latest state; results are ephemeral preview frames, not saved artifacts. The latest render.state / app state includes ipr status with the active IPR job."
        }
        "render.ipr_submit" => {
            "Forces one immediate IPR render of the current state (the open project's snapshot with its own camera; a project must be open). No revision guard; it always re-renders the latest state. Requires render.ipr_set enabled."
        }
        "job.get" => "Returns the state, progress and artifacts of one job.",
        "job.list" => "Lists recent jobs.",
        "job.cancel" => "Cancels a running job.",
        "events.read" => {
            "Reads host events after a counter cursor; wait_ms long-polls so progress can be followed without polling every job."
        }
        "artifact.get" => "Returns artifact metadata (path, size, mime type) for a job output.",
        "artifact.export" => {
            "Copies an artifact to absolute_path; set overwrite to replace an existing file."
        }
        "workspace.reset_layout" => "Resets the desktop dock layout to default.",
        _ => "pbrt-ui control method.",
    };
    let suffix = if is_read_only(method) {
        ""
    } else {
        " Mutating: supply current revisions from app_get_state."
    };
    format!("{text}{suffix}")
}
fn tool_schema(method: &str) -> Value {
    let schema: Value =
        serde_json::from_str(include_str!("../../../schemas/control-v2.schema.json")).unwrap();
    fn expand(value: &Value, defs: &Value) -> Value {
        if let Some(reference) = value["$ref"]
            .as_str()
            .and_then(|s| s.strip_prefix("#/$defs/"))
        {
            return expand(&defs[reference], defs);
        }
        match value {
            Value::Object(map) => Value::Object(
                map.iter()
                    .map(|(key, value)| (key.clone(), expand(value, defs)))
                    .collect(),
            ),
            Value::Array(values) => {
                Value::Array(values.iter().map(|value| expand(value, defs)).collect())
            }
            _ => value.clone(),
        }
    }
    let parameters = &schema["$defs"][format!("params.{method}")];
    let parameters = parameters
        .get("anyOf")
        .and_then(|v| v.as_array())
        .and_then(|values| values.iter().find(|value| value["type"] == "object"))
        .unwrap_or(parameters);
    expand(parameters, &schema["$defs"])
}
fn error(code: i32, message: &str) -> Value {
    json!({"code":code,"message":message})
}
pub struct Bridge {
    pub client: Client,
    initialized: bool,
    ready: bool,
}
impl Bridge {
    pub fn new(client: Client) -> Self {
        Self {
            client,
            initialized: false,
            ready: false,
        }
    }
    pub fn handle(&mut self, message: Value) -> Option<Value> {
        let id = message.get("id").cloned();
        if message["jsonrpc"] != "2.0" || !message["method"].is_string() {
            return Some(
                json!({"jsonrpc":"2.0","id":id,"error":error(-32600,"invalid JSON-RPC request")}),
            );
        }
        if id.is_none() {
            if message["method"] == "notifications/initialized" && self.initialized {
                self.ready = true;
            }
            return None;
        }
        let result = self.dispatch(message["method"].as_str().unwrap(), &message["params"]);
        Some(match result {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err(error) => json!({"jsonrpc":"2.0","id":id,"error":error}),
        })
    }
    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, Value> {
        match method {
            "initialize" => {
                if self.initialized {
                    return Err(error(-32600, "already initialized"));
                }
                if !params["protocolVersion"].is_string() || !params["clientInfo"].is_object() {
                    return Err(error(
                        -32602,
                        "initialize requires protocolVersion and clientInfo",
                    ));
                }
                self.initialized = true;
                let requested = params["protocolVersion"].as_str().unwrap();
                let version = if ["2024-11-05", "2025-03-26", "2025-06-18", "2025-11-25"]
                    .contains(&requested)
                {
                    requested
                } else {
                    "2025-11-25"
                };
                Ok(
                    json!({"protocolVersion":version,"capabilities":{"tools":{},"resources":{}},"serverInfo":{"name":"pbrt-ui","version":env!("CARGO_PKG_VERSION")},"instructions":"Use app_get_state before mutations; supply project contexts and expected revisions. Use project_get_state before project mutations; project integer edits use decimal strings. Jobs run asynchronously; poll job_get and retrieve artifacts with artifact_get. Source files are never edited."}),
                )
            }
            "ping" => Ok(json!({})),
            _ if !self.ready => Err(error(-32002, "MCP initialization is incomplete")),
            "tools/list" => {
                let capabilities = self
                    .client
                    .call(Method::AppGetCapabilities, json!({}))
                    .map_err(|e| error(-32603, &e.to_string()))?;
                let tools:Vec<_>=METHODS.iter().filter(|m|{
                    if m.starts_with("project.") {
                        if **m == "project.list_scenes" || **m == "project.open_scene" || **m == "project.close_scene" {return capabilities["project"]["multi_scene"]==true;}
                        if **m == "project.list_presets" {return capabilities["project"]["presets"]==true;}
                        if **m=="project.capture" || **m=="project.pick" {return capabilities["scene_capture"]==true && capabilities["project_export"]==true;}
                        if **m=="project.render" {return capabilities["render"]==true && capabilities["project_export"]==true;}
                        let capability = if ["project.apply","project.undo","project.redo"].contains(m) {"project_editing"} else if **m=="project.export" {"project_export"} else {"project_management"};
                        return capabilities[capability]==true;
                    }
                    if m.starts_with("render.") {
                        if **m == "render.ipr_set" || **m == "render.ipr_submit" {
                            return capabilities["ipr"] == true;
                        }
                    }
                    if **m=="workspace.reset_layout"{return capabilities["workspace_layout"]==true;}
                    true
                }).map(|method|json!({"name":method.replace('.',"_"),"description":describe(method),"inputSchema":tool_schema(method),"annotations":{"readOnlyHint":is_read_only(method),"openWorldHint":false}})).collect();
                Ok(json!({"tools":tools}))
            }
            "tools/call" => {
                let name = params["name"]
                    .as_str()
                    .ok_or_else(|| error(-32602, "missing tool name"))?;
                let method = METHODS
                    .iter()
                    .find(|method| method.replace('.', "_") == name)
                    .ok_or_else(|| error(-32602, "unknown tool"))?;
                let method: Method = serde_json::from_value(json!(method)).unwrap();
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));
                if !arguments.is_object() {
                    return Err(error(-32602, "arguments must be an object"));
                }
                Ok(match self.client.call(method, arguments) {
                    Ok(value) => {
                        json!({"content":[{"type":"text","text":value.to_string()}],"structuredContent":value,"isError":false})
                    }
                    Err(error) => {
                        json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                    }
                })
            }
            "resources/list" => Ok(
                json!({"resources":[{"uri":"pbrt://state","name":"Current application state","mimeType":"application/json"}]}),
            ),
            "resources/templates/list" => Ok(
                json!({"resourceTemplates":[{"uriTemplate":"pbrt://artifact/{artifact_id}","name":"Job artifact"}]}),
            ),
            "resources/read" => {
                let uri = params["uri"]
                    .as_str()
                    .ok_or_else(|| error(-32602, "missing resource URI"))?;
                if uri == "pbrt://state" {
                    let value = self
                        .client
                        .call(Method::AppGetState, json!({}))
                        .map_err(|e| error(-32603, &e.to_string()))?;
                    return Ok(
                        json!({"contents":[{"uri":uri,"mimeType":"application/json","text":value.to_string()}]}),
                    );
                }
                let id = uri
                    .strip_prefix("pbrt://artifact/")
                    .ok_or_else(|| error(-32602, "unknown resource URI"))?;
                let artifact = self
                    .client
                    .call(Method::ArtifactGet, json!({"artifact_id":id}))
                    .map_err(|e| error(-32603, &e.to_string()))?;
                if artifact["size"].as_u64().unwrap_or(u64::MAX) > 4 * 1024 * 1024 {
                    return Err(error(
                        -32602,
                        "artifact exceeds inline resource limit; use artifact_export",
                    ));
                }
                let bytes = std::fs::read(artifact["path"].as_str().unwrap())
                    .map_err(|e| error(-32603, &e.to_string()))?;
                Ok(
                    json!({"contents":[{"uri":uri,"mimeType":artifact["mime"],"blob":base64::engine::general_purpose::STANDARD.encode(bytes)}]}),
                )
            }
            _ => Err(error(-32601, "method not found")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn test_every_method_has_specific_description() {
        for method in METHODS {
            assert!(
                !describe(method).starts_with("pbrt-ui control method"),
                "{method}"
            );
        }
    }
    #[test]
    fn test_every_method_has_a_params_schema() {
        for method in METHODS {
            let schema = tool_schema(method);
            assert!(schema.is_object() && schema["type"] == "object", "{method}");
        }
    }
    #[test]
    fn test_list_methods_are_read_only_and_mutations_are_not() {
        for method in [
            "project.list_nodes",
            "project.search_nodes",
            "project.get_view",
            "job.list",
            "app.get_state",
            "artifact.get",
            "events.read",
        ] {
            assert!(is_read_only(method), "{method}");
        }
        for method in [
            "project.apply",
            "project.save",
            "render.ipr_set",
            "render.ipr_submit",
            "job.cancel",
        ] {
            assert!(!is_read_only(method), "{method}");
        }
    }
}
