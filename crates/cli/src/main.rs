use cli::view_argument::interpret_view_update;
use cli::{Client, ClientError};
use protocol::Method;
use serde_json::{Value, json};
use std::path::PathBuf;
use std::time::Duration;
fn invalid(message: impl Into<String>) -> ClientError {
    ClientError::Invalid(message.into())
}
/// Presets are addressed by UUID on the wire; a name such as `shape.sphere`
/// is resolved through `project.list_presets` and must match exactly once.
fn resolve_preset(
    client: &mut Client,
    reference: &Value,
    preset: &str,
) -> Result<String, ClientError> {
    if uuid::Uuid::parse_str(preset).is_ok() {
        return Ok(preset.into());
    }
    let listed = client.call(Method::ProjectListPresets, json!({"project":reference}))?;
    let matches = listed["presets"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|candidate| candidate["name"] == preset)
        .filter_map(|candidate| candidate["id"].as_str())
        .collect::<Vec<_>>();
    match matches.as_slice() {
        [id] => Ok((*id).into()),
        [] => Err(invalid(format!("unknown preset: {preset}"))),
        _ => Err(invalid(format!("ambiguous preset name: {preset}"))),
    }
}
fn run() -> Result<Value, ClientError> {
    let mut endpoint = None;
    let mut wait = false;
    let mut discard_unsaved = false;
    let mut capture_after = false;
    let mut debug_albedo = false;
    let mut debug_ao = false;
    let mut timeout = None;
    let mut request_id = None;
    let mut positional = Vec::new();
    let mut query_options = serde_json::Map::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        match arg.to_str() {
            Some("--endpoint") => {
                endpoint = Some(PathBuf::from(
                    args.next().ok_or_else(|| invalid("missing endpoint"))?,
                ))
            }
            Some("--json") => {}
            Some("--wait") => wait = true,
            Some("--discard-unsaved") => discard_unsaved = true,
            Some("--capture") => capture_after = true,
            Some("--debug-albedo") => debug_albedo = true,
            Some("--debug-ao") => debug_ao = true,
            Some("--timeout-ms") => {
                let value = args.next().ok_or_else(|| invalid("missing timeout"))?;
                timeout = Some(Duration::from_millis(
                    value
                        .to_str()
                        .ok_or_else(|| invalid("timeout is not UTF-8"))?
                        .parse()
                        .map_err(|_| invalid("invalid timeout"))?,
                ));
            }
            Some("--request-id") => {
                request_id = Some(
                    args.next()
                        .and_then(|s| s.into_string().ok())
                        .ok_or_else(|| invalid("missing UTF-8 request id"))?,
                )
            }
            Some("--kind") | Some("--parent") | Some("--cursor") | Some("--limit")
            | Some("--folder") | Some("--scene") | Some("--view") | Some("--after")
            | Some("--definition") | Some("--camera") | Some("--title") | Some("--up")
            | Some("--handedness") | Some("--role") | Some("--mode") | Some("--size")
            | Some("--time") => {
                let key = arg.to_str().unwrap().trim_start_matches("--");
                let value = args
                    .next()
                    .and_then(|value| value.into_string().ok())
                    .ok_or_else(|| invalid("missing UTF-8 query option"))?;
                query_options.insert(
                    key.into(),
                    if key == "limit" {
                        json!(
                            value
                                .parse::<usize>()
                                .map_err(|_| invalid("invalid limit"))?
                        )
                    } else {
                        json!(value)
                    },
                );
            }
            Some("--help") => {
                return Ok(
                    json!({"usage":"pbrt-ui-ctl --endpoint PATH [--json] [--wait] [--timeout-ms N] [--request-id ID] project state|import SOURCE [DEST]|import-scene SOURCE [--folder FOLDER]|import-resource SOURCE --folder FOLDER|open DIR|new DIR [--title T] [--up +X|-X|+Y|-Y|+Z|-Z] [--handedness left|right]|close|save [DEST]|export DIR [--scene ID]|render [--scene ID] [--camera scene|view]|capture [--scene ID] [--camera scene|view] [--debug-albedo|--debug-ao]|pick X Y [--scene ID]|scenes|open-scene ID|close-scene|contents [FOLDER]|view [JSON] [--mode solid|normal|wireframe|lit] [--size WxH] [--time T]|nodes [PARENT] [--view directives|scene_graph]|node ID|resource ID|asset ID|cache-get ITEM KEY|cache-put ITEM KEY KIND MEDIA_TYPE FILE|schema DIRECTIVE IMPLEMENTATION [FLOAT|SPECTRUM]|parameters node|resource ID [OFFSET]|values node|resource ID NAME [OFFSET] [--limit N]|set-values node|resource ID NAME WINDOWS_JSON|spectrum-color node|resource ID PARAMETER [--role albedo|unbounded|illuminant|chromaticity|conductor]|plan-implementation node|resource ID IMPLEMENTATION|list-references RESOURCE|ASSET|SCENE ID [OFFSET]|graph RESOURCE_ID [LIMIT]|spectra|texture-preview RESOURCE_ID [SIZE]|material-preview RESOURCE_ID [SIZE]|asset-preview ASSET_ID [SIZE]|search LABEL [OFFSET] [--view directives|scene_graph]|presets|create PRESET [--parent GROUP_ID] [--after NODE_ID] [--definition OBJECT_ID]|apply EDITS_JSON [--capture]|undo|redo; project switches accept --discard-unsaved; state|capabilities|hello|job ID|jobs|cancel ID|events AFTER|settings [JSON]|ipr [status]|on [SETTINGS_JSON]|off|submit [--scene ID]|artifact ID|export ID PATH|reset-layout|quit [--discard-unsaved]|raw METHOD PARAMS_JSON", "settings_note":"settings accepts threads plus samples_per_pixel:null and max_depth:null; non-null quality overrides are rejected. IPR keeps its own sample budget.", "query_options":["--parent GROUP_ID","--limit N","--scene SCENE_ID","--view VIEW","--after NODE_ID","--definition OBJECT_ID","--folder FOLDER","--camera scene|view","--title T","--up +X|-X|+Y|-Y|+Z|-Z","--handedness left|right","--role albedo|unbounded|illuminant|chromaticity|conductor","--mode solid|normal|wireframe|lit","--size WxH","--time T"]}),
                );
            }
            Some(value) if value.starts_with("--") => {
                return Err(invalid(format!("unknown flag: {value}")));
            }
            _ => positional.push(arg),
        }
    }
    let command = positional
        .first()
        .and_then(|s| s.to_str())
        .ok_or_else(|| invalid("missing command (use --help)"))?;
    let argument = |index: usize| -> Result<&str, ClientError> {
        positional
            .get(index)
            .and_then(|s| s.to_str())
            .ok_or_else(|| invalid("missing or non-UTF-8 argument"))
    };
    let expected = match command {
        "project" => match argument(1)? {
            "state" | "close" | "undo" | "redo" | "render" | "capture" | "scenes" | "presets"
            | "close-scene" => 2,
            "pick" => 4,
            "import" if (3..=4).contains(&positional.len()) => positional.len(),
            "save" if (2..=3).contains(&positional.len()) => positional.len(),
            "import-scene" | "import-resource" | "open" | "export" | "apply" | "node"
            | "resource" | "asset" | "open-scene" | "create" | "new" => 3,
            "cache-get" => 4,
            "plan-implementation" => 5,
            "spectrum-color" => 5,
            "graph" if (3..=4).contains(&positional.len()) => positional.len(),
            "spectra" => 2,
            "texture-preview" | "material-preview" | "asset-preview"
                if (3..=4).contains(&positional.len()) =>
            {
                positional.len()
            }
            "cache-put" => 7,
            "schema" if (4..=5).contains(&positional.len()) => positional.len(),
            "contents" if (2..=3).contains(&positional.len()) => positional.len(),
            "view" if (2..=3).contains(&positional.len()) => positional.len(),
            "nodes" if positional.len() <= 3 => positional.len(),
            "search" if (3..=4).contains(&positional.len()) => positional.len(),
            "list-references" if (4..=5).contains(&positional.len()) => positional.len(),
            "parameters" if (4..=5).contains(&positional.len()) => positional.len(),
            "values" if (5..=6).contains(&positional.len()) => positional.len(),
            "set-values" => 6,
            _ => return Err(invalid("unknown project subcommand")),
        },
        "settings" if positional.len() <= 2 => positional.len(),
        "ipr" if positional.len() <= 3 => positional.len(),
        "artifact" => 2,
        "export" => 3,
        "reset-layout" => 1,
        "state" | "capabilities" | "hello" | "jobs" | "quit" => 1,
        "job" | "cancel" | "events" => 2,
        "raw" => 3,
        _ => return Err(invalid("unknown command")),
    };
    if positional.len() != expected {
        return Err(invalid("wrong number of command arguments"));
    }
    if !query_options.is_empty()
        && !(command == "project"
            && [
                "nodes",
                "parameters",
                "values",
                "search",
                "contents",
                "list-references",
            ]
            .contains(&argument(1)?)
            && query_options.keys().all(|key| key == "limit"))
        && !(command == "project"
            && ["nodes", "search"].contains(&argument(1)?)
            && query_options
                .keys()
                .all(|key| key == "limit" || key == "view"))
        && !(command == "project"
            && argument(1)? == "create"
            && query_options
                .keys()
                .all(|key| ["parent", "after", "definition"].contains(&key.as_str())))
        && !(command == "project"
            && argument(1)? == "new"
            && query_options
                .keys()
                .all(|key| ["title", "up", "handedness"].contains(&key.as_str())))
        && !(command == "project"
            && ["render", "capture", "pick", "export"].contains(&argument(1)?)
            && query_options.keys().all(|key| key == "scene"))
        && !(command == "project"
            && ["render", "capture"].contains(&argument(1)?)
            && query_options
                .keys()
                .all(|key| ["scene", "camera"].contains(&key.as_str())))
        && !(command == "project"
            && ["import-scene", "import-resource"].contains(&argument(1)?)
            && query_options.keys().all(|key| key == "folder"))
        && !(command == "project"
            && argument(1)? == "spectrum-color"
            && query_options.keys().all(|key| key == "role"))
        && !(command == "project"
            && argument(1)? == "view"
            && query_options
                .keys()
                .all(|key| ["mode", "size", "time"].contains(&key.as_str())))
        && !(command == "ipr"
            && argument(1)? == "submit"
            && query_options.keys().all(|key| key == "scene"))
    {
        return Err(invalid(
            "query options apply to project nodes, search, contents, parameters, spectrum-color, create, render, capture, pick, export, view and ipr submit",
        ));
    }
    if discard_unsaved
        && !(command == "project" && ["import", "open", "close"].contains(&argument(1)?))
        && command != "quit"
    {
        return Err(invalid(
            "--discard-unsaved applies to project import/open/close and quit",
        ));
    }
    if debug_albedo && !(command == "project" && argument(1)? == "capture") {
        return Err(invalid("--debug-albedo applies to project capture"));
    }
    if debug_ao && !(command == "project" && argument(1)? == "capture") {
        return Err(invalid("--debug-ao applies to project capture"));
    }
    if capture_after && !(command == "project" && argument(1)? == "apply") {
        return Err(invalid("--capture applies to project apply"));
    }
    let mut client = Client::connect(
        &endpoint.ok_or_else(|| invalid("--endpoint is required; choose a host explicitly"))?,
    )?;
    let (method, params) = match command {
        "project" => {
            let subcommand = argument(1)?;
            let state = client.call(Method::ProjectGetState, json!({}))?;
            let reference = state["context"]["project"].clone();
            let path = |index| -> Result<String, ClientError> {
                std::path::absolute(argument(index)?)?
                    .into_os_string()
                    .into_string()
                    .map_err(|_| invalid("project path is not UTF-8"))
            };
            let optional_path = |index| -> Result<Option<String>, ClientError> {
                if positional.get(index).is_some() {
                    path(index).map(Some)
                } else {
                    Ok(None)
                }
            };
            match subcommand {
                "state" => (Method::ProjectGetState, json!({})),
                "scenes" => (Method::ProjectListScenes, json!({"project":reference})),
                "close-scene" => (Method::ProjectCloseScene, json!({"project":reference})),
                "open-scene" => (
                    Method::ProjectOpenScene,
                    json!({"project":reference,"scene":argument(2)?}),
                ),
                "contents" => (
                    Method::ProjectListContents,
                    json!({"project":reference,"folder":if positional.len()==3 {argument(2)?} else {""},"limit":query_options.get("limit").cloned().unwrap_or(json!(100))}),
                ),
                "import-scene" | "import-resource" => (
                    if subcommand == "import-scene" {
                        Method::ProjectImportScene
                    } else {
                        Method::ProjectImportResource
                    },
                    json!({"project":reference,"source_path":path(2)?,"folder":query_options.get("folder")}),
                ),
                "import" => (
                    Method::ProjectImport,
                    json!({"source_path":path(2)?,"destination":optional_path(3)?,"expected_current":state["context"],"discard_unsaved":discard_unsaved}),
                ),
                "open" => (
                    Method::ProjectOpen,
                    json!({"path":path(2)?,"expected_current":state["context"],"discard_unsaved":discard_unsaved}),
                ),
                "new" => {
                    // The title defaults to the destination folder's name.
                    let default_title = std::path::Path::new(argument(2)?)
                        .file_name()
                        .and_then(|name| name.to_str())
                        .unwrap_or("Project")
                        .to_owned();
                    let title = query_options
                        .get("title")
                        .cloned()
                        .unwrap_or_else(|| json!(default_title));
                    let mut coordinate_system = serde_json::Map::new();
                    if let Some(value) = query_options.get("up") {
                        let up = value.as_str().unwrap_or_default();
                        if !["+X", "-X", "+Y", "-Y", "+Z", "-Z"].contains(&up) {
                            return Err(invalid(
                                "up must be one of +X -X +Y -Y +Z -Z (default +Y)",
                            ));
                        }
                        coordinate_system.insert("up".into(), json!(up));
                    }
                    if let Some(value) = query_options.get("handedness") {
                        let handedness = value.as_str().unwrap_or_default();
                        if !["left", "right"].contains(&handedness) {
                            return Err(invalid("handedness must be left or right (default left)"));
                        }
                        coordinate_system.insert("handedness".into(), json!(handedness));
                    }
                    (
                        Method::ProjectCreate,
                        json!({"path":path(2)?,"title":title,
                               "coordinate_system":(!coordinate_system.is_empty()).then(|| serde_json::Value::Object(coordinate_system)),
                               "expected_current":state["context"],"discard_unsaved":discard_unsaved}),
                    )
                }
                "close" => (
                    Method::ProjectClose,
                    json!({"expected_current":state["context"],"discard_unsaved":discard_unsaved}),
                ),
                // Static information: served without a project.
                "schema" => {
                    let implementation = argument(3)?;
                    (
                        Method::ProjectGetParameterSchema,
                        if positional.len() == 5 {
                            json!({"directive":argument(2)?,"implementation":implementation,
                                   "texture_value_type":argument(4)?})
                        } else {
                            json!({"directive":argument(2)?,"implementation":implementation})
                        },
                    )
                }
                // List the builtin spectrum names, which are static like the schema.
                "spectra" => (Method::ProjectListSpectra, json!({})),
                _ if reference.is_null() => return Err(invalid("no current project")),
                "save" => (
                    Method::ProjectSave,
                    json!({"project":reference,"destination":optional_path(2)?}),
                ),
                "render" => {
                    let settings = client.call(Method::RenderGetSettings, json!({}))?;
                    (
                        Method::ProjectRender,
                        json!({"project":reference,"expected_settings_revision":settings["settings_revision"],"scene":query_options.get("scene"),"camera":query_options.get("camera")}),
                    )
                }
                "view" => {
                    // Without a JSON argument and flags this prints the
                    // shared native view. The JSON is a Camera, a view
                    // state or
                    // a project.get_view result; its camera goes to
                    // set_camera (the CLI resolves the current revision
                    // through project.get_view first), and --mode/--size/
                    // --time, which win over the JSON, to configure_view
                    // chained on the previous call's revision and skipped
                    // when there is nothing to configure.
                    let current =
                        client.call(Method::ProjectGetView, json!({"project":reference}))?;
                    let json = if positional.len() == 3 {
                        Some(serde_json::from_str::<Value>(argument(2)?)?)
                    } else {
                        None
                    };
                    let update = interpret_view_update(
                        json.as_ref(),
                        query_options.get("mode").and_then(Value::as_str),
                        query_options.get("size").and_then(Value::as_str),
                        query_options.get("time").and_then(Value::as_str),
                    )?;
                    if update.camera.is_none() && update.nothing_to_configure() {
                        return Ok(current);
                    }
                    let mut revision = current["view_revision"].clone();
                    let configure = !update.nothing_to_configure();
                    if let Some(camera) = update.camera {
                        if !configure {
                            (
                                Method::ProjectSetCamera,
                                json!({"project":reference,"expected_view_revision":revision,"camera":camera}),
                            )
                        } else {
                            let result = client.call(
                                Method::ProjectSetCamera,
                                json!({"project":reference,"expected_view_revision":revision,"camera":camera}),
                            )?;
                            revision = result["view_revision"].clone();
                            (
                                Method::ProjectConfigureView,
                                json!({"project":reference,"expected_view_revision":revision,"size":update.size,"mode":update.mode,"time":update.time}),
                            )
                        }
                    } else {
                        (
                            Method::ProjectConfigureView,
                            json!({"project":reference,"expected_view_revision":revision,"size":update.size,"mode":update.mode,"time":update.time}),
                        )
                    }
                }
                "capture" => (
                    Method::ProjectCapture,
                    json!({"project":reference,"scene":query_options.get("scene"),"camera":query_options.get("camera"),"debug_albedo":debug_albedo,"debug_ao":debug_ao}),
                ),
                "pick" => {
                    let coordinate = |index| -> Result<f64, ClientError> {
                        argument(index)?
                            .parse()
                            .map_err(|_| invalid("pick coordinates must be numbers in 0..1"))
                    };
                    (
                        Method::ProjectPick,
                        json!({"project":reference,"normalized_xy":[coordinate(2)?,coordinate(3)?],"scene":query_options.get("scene")}),
                    )
                }
                "export" => (
                    Method::ProjectExport,
                    json!({"project":reference,"destination":path(2)?,"scene":query_options.get("scene")}),
                ),
                "undo" => (Method::ProjectUndo, json!({"project":reference})),
                "redo" => (Method::ProjectRedo, json!({"project":reference})),
                "apply" => (
                    Method::ProjectApply,
                    json!({"project":reference,"edits":serde_json::from_str::<Vec<protocol::project::ProjectEdit>>(argument(2)?)?,"capture":capture_after.then(|| json!({"view":null}))}),
                ),
                "nodes" => (
                    Method::ProjectListNodes,
                    json!({"project":reference,"parent":if positional.len()==3{json!(argument(2)?)}else{Value::Null},"view":query_options.get("view").cloned().unwrap_or(json!("directives")),"limit":query_options.get("limit").cloned().unwrap_or(json!(100))}),
                ),
                "presets" => (Method::ProjectListPresets, json!({"project":reference})),
                "create" => {
                    let preset = resolve_preset(&mut client, &reference, argument(2)?)?;
                    let parent = match query_options.get("parent") {
                        Some(parent) => parent.clone(),
                        // The Scene root stands for its World.
                        None => state["active_scene"].clone(),
                    };
                    let edit = json!({"kind":"create_preset","preset":preset,"parent":parent,
                        "after":query_options.get("after"),"definition":query_options.get("definition")});
                    (
                        Method::ProjectApply,
                        json!({"project":reference,"edits":[edit]}),
                    )
                }
                "list-references" => {
                    // List where each resource or asset is used. KIND is resource, asset, or
                    // scene; scenes have no where-used entries.
                    let kind = argument(2)?;
                    if !["resource", "asset", "scene"].contains(&kind) {
                        return Err(invalid("list-references takes resource, asset or scene"));
                    }
                    let id = argument(3)?;
                    id.parse::<uuid::Uuid>()
                        .map_err(|_| invalid("invalid list-references item id"))?;
                    let offset = if positional.len() == 5 {
                        argument(4)?
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid list-references offset"))?
                    } else {
                        0
                    };
                    (
                        Method::ProjectListReferences,
                        json!({"project":reference,"item":{"kind":kind,"id":id},"offset":offset,
                               "limit":query_options.get("limit").cloned().unwrap_or(json!(100))}),
                    )
                }
                "search" => {
                    let offset = if positional.len() == 4 {
                        argument(3)?
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid search offset"))?
                    } else {
                        0
                    };
                    (
                        Method::ProjectSearchNodes,
                        json!({"project":reference,"query":argument(2)?,"offset":offset,"view":query_options.get("view").cloned().unwrap_or(json!("directives")),"limit":query_options.get("limit").cloned().unwrap_or(json!(100))}),
                    )
                }
                "node" => (
                    Method::ProjectGetNode,
                    json!({"project":reference,"node_id":argument(2)?}),
                ),
                "resource" => (
                    Method::ProjectGetResource,
                    json!({"project":reference,"resource_id":argument(2)?}),
                ),
                "asset" => (
                    Method::ProjectGetAsset,
                    json!({"project":reference,"asset_id":argument(2)?}),
                ),
                "cache-get" => (
                    Method::ProjectGetCache,
                    json!({"project":reference,"item":argument(2)?,"key":argument(3)?}),
                ),
                "cache-put" => {
                    let kind = argument(4)?;
                    if !["thumbnail", "preview"].contains(&kind) {
                        return Err(invalid("cache-put kind must be thumbnail or preview"));
                    }
                    let media_type = argument(5)?;
                    let data = std::fs::read(argument(6)?)
                        .map_err(|error| invalid(format!("cannot read the entry: {error}")))?;
                    let encoded =
                        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, data);
                    (
                        Method::ProjectPutCache,
                        json!({"project":reference,"item":argument(2)?,"key":argument(3)?,
                               "kind":kind,"media_type":media_type,"data_base64":encoded}),
                    )
                }
                "set-values" => {
                    if !["node", "resource"].contains(&argument(2)?) {
                        return Err(invalid("project set-values requires node or resource"));
                    }
                    let windows: serde_json::Value = serde_json::from_str(argument(5)?)?;
                    (
                        Method::ProjectSetParameterValues,
                        json!({"project":reference,"target":{"kind":argument(2)?,"id":argument(3)?},"name":argument(4)?,"windows":windows}),
                    )
                }
                "values" => {
                    if !["node", "resource"].contains(&argument(2)?) {
                        return Err(invalid("project values requires node or resource"));
                    }
                    let offset = if positional.len() == 6 {
                        argument(5)?
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid value offset"))?
                    } else {
                        0
                    };
                    (
                        Method::ProjectGetParameterValues,
                        json!({"project":reference,"target":{"kind":argument(2)?,"id":argument(3)?},"name":argument(4)?,"offset":offset,"limit":query_options.get("limit").cloned().unwrap_or(json!(256))}),
                    )
                }
                "parameters" => {
                    if !["node", "resource"].contains(&argument(2)?) {
                        return Err(invalid("project parameters requires node or resource"));
                    }
                    let offset = if positional.len() == 5 {
                        argument(4)?
                            .parse::<usize>()
                            .map_err(|_| invalid("invalid parameter offset"))?
                    } else {
                        0
                    };
                    (
                        Method::ProjectGetParameters,
                        json!({"project":reference,"target":{"kind":argument(2)?,"id":argument(3)?},"offset":offset,"limit":query_options.get("limit").cloned().unwrap_or(json!(100))}),
                    )
                }
                // Write a texture or material preview PNG to the project cache. The asset
                // preview command writes an image asset's preview.
                "texture-preview" | "material-preview" | "asset-preview" => {
                    let size = if positional.len() == 4 {
                        json!(
                            argument(3)?
                                .parse::<u32>()
                                .map_err(|_| invalid("invalid preview size"))?
                        )
                    } else {
                        Value::Null
                    };
                    match subcommand {
                        "asset-preview" => (
                            Method::ProjectPreviewAsset,
                            json!({"project":reference,"asset_id":argument(2)?,"size":size}),
                        ),
                        "material-preview" => (
                            Method::ProjectPreviewMaterial,
                            json!({"project":reference,"resource_id":argument(2)?,"size":size}),
                        ),
                        _ => (
                            Method::ProjectPreviewTexture,
                            json!({"project":reference,"resource_id":argument(2)?,"size":size}),
                        ),
                    }
                }
                // Show a material's or texture's upstream graph.
                "graph" => {
                    let limit = if positional.len() == 4 {
                        json!(
                            argument(3)?
                                .parse::<usize>()
                                .map_err(|_| invalid("invalid graph limit"))?
                        )
                    } else {
                        Value::Null
                    };
                    (
                        Method::ProjectGetResourceGraph,
                        json!({"project":reference,"resource_id":argument(2)?,"limit":limit}),
                    )
                }
                // Preview set_implementation without applying it.
                "plan-implementation" => {
                    if !["node", "resource"].contains(&argument(2)?) {
                        return Err(invalid(
                            "project plan-implementation requires node or resource",
                        ));
                    }
                    (
                        Method::ProjectPlanImplementation,
                        json!({"project":reference,"target":{"kind":argument(2)?,"id":argument(3)?},"implementation":argument(4)?}),
                    )
                }
                // Show one parameter's spectrum-like colour and its source,
                // optionally filtered by role.
                "spectrum-color" => {
                    if !["node", "resource"].contains(&argument(2)?) {
                        return Err(invalid("project spectrum-color requires node or resource"));
                    }
                    let mut params = json!({"project":reference,
                        "target":{"kind":argument(2)?,"id":argument(3)?},
                        "parameter":argument(4)?});
                    if let Some(role) = query_options.get("role") {
                        let role = role.as_str().ok_or_else(|| invalid("role must be text"))?;
                        if ![
                            "albedo",
                            "unbounded",
                            "illuminant",
                            "chromaticity",
                            "conductor",
                        ]
                        .contains(&role)
                        {
                            return Err(invalid(
                                "role must be albedo, unbounded, illuminant, chromaticity or conductor",
                            ));
                        }
                        params["role"] = json!(role);
                    }
                    (Method::ProjectSpectrumColor, params)
                }
                _ => unreachable!(),
            }
        }
        "ipr" => {
            let subcommand = if positional.len() == 1 {
                "status"
            } else {
                argument(1)?
            };
            let state = client.call(Method::AppGetState, json!({}))?;
            match subcommand {
                "status" => return Ok(state["ipr"].clone()),
                "on" | "off" => {
                    let settings = if positional.len() == 3 {
                        json!(protocol::ndjson::decode::<protocol::jobs::IprSettings>(
                            argument(2)?.as_bytes()
                        )?)
                    } else {
                        state["ipr"]["settings"].clone()
                    };
                    (
                        Method::RenderIprSet,
                        json!({"enabled":subcommand == "on","settings":settings}),
                    )
                }
                "submit" => {
                    if positional.len() != 2 {
                        return Err(invalid("ipr submit takes no arguments"));
                    }
                    let projects = client.call(Method::ProjectGetState, json!({}))?;
                    let project = projects["context"]["project"].clone();
                    if project.is_null() {
                        return Err(invalid("ipr submit needs an open project"));
                    }
                    (
                        Method::RenderIprSubmit,
                        json!({"project":project,"scene":query_options.get("scene")}),
                    )
                }
                _ => return Err(invalid("ipr expects status|on [SETTINGS_JSON]|off|submit")),
            }
        }
        "settings" => {
            let state = client.call(Method::AppGetState, json!({}))?;
            if positional.len() == 1 {
                (Method::RenderGetSettings, json!({}))
            } else {
                (
                    Method::RenderSetSettings,
                    json!({"settings":protocol::ndjson::decode::<Value>(argument(1)?.as_bytes())?,"expected_settings_revision":state["settings_revision"]}),
                )
            }
        }
        "artifact" => (Method::ArtifactGet, json!({"artifact_id":argument(1)?})),
        "export" => {
            let path = std::path::absolute(argument(2)?)?;
            (
                Method::ArtifactExport,
                json!({"artifact_id":argument(1)?,"absolute_path":path.to_str().ok_or_else(||invalid("non UTF-8 export path"))?,"overwrite":false}),
            )
        }
        "reset-layout" => (Method::WorkspaceResetLayout, json!({})),
        "state" => (Method::AppGetState, json!({})),
        "capabilities" => (Method::AppGetCapabilities, json!({})),
        "quit" => (Method::AppQuit, json!({"discard_unsaved":discard_unsaved})),
        "hello" => (
            Method::AppHello,
            json!({"client_name":"pbrt-ui-ctl","supported_protocols":[protocol::PROTOCOL_VERSION]}),
        ),
        "job" => (Method::JobGet, json!({"job_id":argument(1)?})),
        "jobs" => (Method::JobList, json!({})),
        "cancel" => (Method::JobCancel, json!({"job_id":argument(1)?})),
        "events" => (Method::EventsRead, json!({"after":argument(1)?})),
        "raw" => {
            let method: Method = serde_json::from_value(json!(argument(1)?))?;
            let params: Value = protocol::ndjson::decode(argument(2)?.as_bytes())?;
            (method, params)
        }
        _ => unreachable!(),
    };
    let result = client.call_with_id(
        method,
        params,
        request_id.unwrap_or_else(|| uuid::Uuid::new_v4().to_string()),
    )?;
    if wait {
        let id = result
            .get("job_id")
            .ok_or_else(|| invalid("--wait requires a command returning a job_id"))?;
        let job = client.wait_job(serde_json::from_value(id.clone())?, timeout)?;
        Ok(serde_json::to_value(job)?)
    } else {
        Ok(result)
    }
}
fn main() {
    match run() {
        Ok(result) => println!("{}", result),
        Err(ClientError::Remote(error)) => {
            eprintln!("{}", json!({"error":error}));
            std::process::exit(1);
        }
        Err(error) => {
            eprintln!(
                "{}",
                json!({"error":{"code":"client_error","message":error.to_string()}})
            );
            std::process::exit(1);
        }
    }
}
