//! Core requests, scene/project queries and shared-view operations.
use super::*;
pub(super) fn request(core: &CoreHandle, method: Method, params: Value) -> Result<Value, String> {
    request_error(core, method, params).map_err(|error| error.message)
}

/// Like [`request`], but keeps the structured error so callers can branch on
/// its code (the unsaved-changes confirmation).
pub(super) fn request_error(
    core: &CoreHandle,
    method: Method,
    params: Value,
) -> Result<Value, protocol::ErrorPayload> {
    let response = core.request(Request {
        protocol: protocol::PROTOCOL_VERSION.into(),
        request_id: uuid::Uuid::new_v4().to_string(),
        method,
        params,
    });
    response.result.ok_or_else(|| {
        response.error.unwrap_or_else(|| {
            protocol::ErrorPayload::new(protocol::ErrorCode::InvalidRequest, "Empty response")
        })
    })
}
pub(super) fn project_nodes_page(
    core: &CoreHandle,
    project: &Value,
    parent: &Option<String>,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    request(
        core,
        Method::ProjectListNodes,
        json!({"project":project,"parent":parent,"offset":offset,"limit":limit,"view":"scene_graph"}),
    )
}

/// Label search across the whole project. `path` on each hit is parent-first so
/// the lazy hierarchy can expand to it without walking the tree again.
pub(super) fn project_search(
    core: &CoreHandle,
    project: &Value,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    request(
        core,
        Method::ProjectSearchNodes,
        json!({"project":project,"query":query,"offset":offset,"limit":limit,"view":"scene_graph"}),
    )
}

pub(super) fn project_search_contents(
    core: &CoreHandle,
    project: &Value,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    request(
        core,
        Method::ProjectSearchContents,
        json!({"project":project,"query":query,"offset":offset,"limit":limit}),
    )
}

/// One Contents folder (`""` is `Assets/`, `"@builtin"` the presets).
pub(super) fn project_contents_page(
    core: &CoreHandle,
    project: &Value,
    folder: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, String> {
    request(
        core,
        Method::ProjectListContents,
        json!({"project":project,"folder":folder,"offset":offset,"limit":limit}),
    )
}

/// The scene-graph roots: the `Scene` node, paged to completion.
pub(super) fn project_roots(core: &CoreHandle, project: &Value) -> Result<Vec<Value>, String> {
    let mut records = Vec::new();
    let mut offset = 0usize;
    loop {
        let page = project_nodes_page(core, project, &None, offset, 1000)?;
        let nodes = page["nodes"]
            .as_array()
            .ok_or("invalid project node page")?;
        records.extend(nodes.iter().cloned());
        if page["next_offset"].is_null() {
            return Ok(records);
        }
        offset = page["next_offset"]
            .as_u64()
            .ok_or("invalid project node offset")? as usize;
    }
}

/// Object-definition scope rows for the Hierarchy. The resource id shown in
/// Contents is distinct from the scope node id used to page its children.
pub(super) fn project_definition_roots(
    core: &CoreHandle,
    project: &Value,
) -> Result<Vec<Value>, String> {
    let items = project_items(core, project)?;
    let mut definitions = Vec::new();
    for entry in items
        .as_object()
        .into_iter()
        .flat_map(|items| items.values())
    {
        if entry["kind"] != "resource" || entry["resource_kind"] != "object_definition" {
            continue;
        }
        let resource = request(
            core,
            Method::ProjectGetResource,
            json!({"project":project,"resource_id":entry["id"]}),
        )?;
        let Some(scope) = resource["definition_scope"].as_str() else {
            continue;
        };
        let children = project_nodes_page(core, project, &Some(scope.to_owned()), 0, 1)?;
        let references = request(
            core,
            Method::ProjectListReferences,
            json!({"project":project,"item":{"kind":"resource","id":entry["id"]},"offset":0,"limit":1000}),
        )?;
        let instance_count = references["references"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|reference| reference["via"] == "instance")
            .count() as u64;
        definitions.push(serde_json::json!({
            "id":scope,
            "resource_id":entry["id"],
            "label":resource["label"],
            "entry":entry,
            "object":"definition_root",
            "kind":"definition_root",
            "instance_count":if instance_count == references["total"].as_u64().unwrap_or(instance_count) {
                references["total"].as_u64().unwrap_or(instance_count)
            } else {
                instance_count
            },
            "child_count":children["total"],
            "loaded":false,
            "children":[],
            "more_offset":null,
        }));
    }
    definitions.sort_by(|a, b| a["label"].as_str().cmp(&b["label"].as_str()));
    Ok(definitions)
}

/// Descendant node ids of one node, breadth-first by paging children. Used for
/// Frame Selection, so the bound stays generous but finite; lazily loaded
/// hierarchy snapshots never need the full subtree.
pub(super) fn project_descendants(
    core: &CoreHandle,
    project: &Value,
    root: &str,
) -> Result<Vec<String>, String> {
    const MAX_DESCENDANTS: usize = 100_000;
    let mut pending = vec![root.to_owned()];
    let mut ids = Vec::new();
    while let Some(parent) = pending.pop() {
        let mut offset = 0usize;
        loop {
            let page = project_nodes_page(core, project, &Some(parent.clone()), offset, 1000)?;
            for record in page["nodes"]
                .as_array()
                .ok_or("invalid project node page")?
            {
                let id = record["node_id"]
                    .as_str()
                    .ok_or("invalid project node id")?
                    .to_owned();
                if ids.len() >= MAX_DESCENDANTS {
                    return Err(
                        "selection scope exceeds the frame-selection descendant budget".to_owned(),
                    );
                }
                if record["child_count"].as_u64().unwrap_or(0) > 0 {
                    pending.push(id.clone());
                }
                ids.push(id);
            }
            if page["next_offset"].is_null() {
                break;
            }
            offset = page["next_offset"]
                .as_u64()
                .ok_or("invalid project node offset")? as usize;
        }
    }
    Ok(ids)
}
pub(super) fn project_node_details(
    core: &CoreHandle,
    project: &Value,
    node_id: &str,
    items: &Value,
) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(node_id).map_err(|e| e.to_string())?;
    let node = request(
        core,
        Method::ProjectGetNode,
        json!({"project":project,"node_id":id}),
    )?;
    let mut details = json!({"node":node,"items":items});
    if let Some(count) = definition_instance_count(core, project, &details["node"])? {
        details["definition_instance_count"] = json!(count);
    }
    if matches!(
        details["node"]["data"]["kind"].as_str(),
        Some("entity" | "configuration")
    ) {
        let target = json!({"kind":"node","id":id});
        details["parameters"] = parameter_pages(core, project, &target)?;
        details["parameter_target"] = target;
        let data = details["node"]["data"].clone();
        let kind = data["entity"].as_str().or(data["configuration"].as_str());
        attach_schema(
            core,
            &mut details["parameters"],
            kind.unwrap_or_default(),
            data["implementation"].as_str().unwrap_or_default(),
            &Value::Null,
        );
    }
    // Components that edit another node (the area light, the Scene root's
    // render settings) carry their own parameter target.
    let mut by_target = serde_json::Map::new();
    for component in details["node"]["components"]
        .as_array()
        .cloned()
        .unwrap_or_default()
    {
        let target = &component["parameter_target"];
        let Some(target_id) = target["id"].as_str() else {
            continue;
        };
        if target_id == node_id || by_target.contains_key(target_id) {
            continue;
        }
        let mut page = parameter_pages(core, project, target)?;
        attach_schema(
            core,
            &mut page,
            component["kind"].as_str().unwrap_or_default(),
            component["implementation"].as_str().unwrap_or_default(),
            &Value::Null,
        );
        by_target.insert(target_id.to_owned(), page);
    }
    details["component_parameters"] = Value::Object(by_target);
    Ok(details)
}

/// The instance count of the object definition holding `node` (core names it
/// in the node's `definition`), independent of which Hierarchy branches are
/// loaded.
fn definition_instance_count(
    core: &CoreHandle,
    project: &Value,
    node: &Value,
) -> Result<Option<u64>, String> {
    let Some(resource_id) = node["definition"].as_str() else {
        return Ok(None);
    };
    let references = request(
        core,
        Method::ProjectListReferences,
        json!({"project":project,"item":{"kind":"resource","id":resource_id},"offset":0,"limit":1000}),
    )?;
    Ok(Some(
        references["references"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|reference| reference["via"] == "instance")
            .count() as u64,
    ))
}

/// The PBRT directive behind a component, node or resource kind, as the
/// wire spells them (`shape`, `area_light`, `film`, `material`, …).
fn directive_of(kind: &str) -> Option<&'static str> {
    Some(match kind {
        "shape" => "Shape",
        "light" => "LightSource",
        "area_light" => "AreaLightSource",
        "camera" => "Camera",
        "film" => "Film",
        "sampler" => "Sampler",
        "integrator" => "Integrator",
        "pixel_filter" => "PixelFilter",
        "accelerator" => "Accelerator",
        "material" => "MakeNamedMaterial",
        "texture" => "Texture",
        "medium" => "MakeNamedMedium",
        // The SpectrumData resource's `piecewise` implementation
        // lists its two sample tables like every other resource.
        "spectrum_data" => "SpectrumData",
        _ => return None,
    })
}

/// Attach the parameter table's rows of `kind implementation` to a
/// parameter page as `schema` (null without a table) for the typed fields.
/// The cancelling area light (empty implementation) declares nothing.
fn attach_schema(
    core: &CoreHandle,
    page: &mut Value,
    kind: &str,
    implementation: &str,
    texture_value_type: &Value,
) {
    // The page's rows carry the implementation, for layouts that depend
    // on it.
    page["implementation"] = Value::String(implementation.to_owned());
    // The table describes r4, not a project: ask once per implementation.
    static TABLES: std::sync::OnceLock<Mutex<BTreeMap<String, Value>>> = std::sync::OnceLock::new();
    page["schema"] = match directive_of(kind) {
        Some(directive) if !implementation.is_empty() => {
            let key = format!("{directive}\0{implementation}\0{texture_value_type}");
            let tables = TABLES.get_or_init(Default::default);
            let cached = tables
                .lock()
                .ok()
                .and_then(|tables| tables.get(&key).cloned());
            cached.unwrap_or_else(|| {
                let rows = request(
                    core,
                    Method::ProjectGetParameterSchema,
                    json!({"directive":directive,"implementation":implementation,
                           "texture_value_type":texture_value_type}),
                )
                .map(|reply| reply["parameters"].clone())
                .unwrap_or(Value::Null);
                if let Ok(mut tables) = tables.lock() {
                    tables.insert(key, rows.clone());
                }
                rows
            })
        }
        _ => Value::Null,
    };
}

/// Every Contents entry of one revision by id (resources, assets, scenes),
/// for Inspector links and selectors. Walks the folders once.
pub(super) fn project_items(core: &CoreHandle, project: &Value) -> Result<Value, String> {
    let mut items = serde_json::Map::new();
    let mut pending = vec![String::new()];
    let mut folders = 0usize;
    while let Some(folder) = pending.pop() {
        folders += 1;
        if folders > 4096 {
            break;
        }
        let mut offset = 0;
        loop {
            let page = project_contents_page(core, project, &folder, offset, 1000)?;
            for entry in page["entries"].as_array().into_iter().flatten() {
                if entry["kind"] == "folder" {
                    if let Some(location) = entry["location"].as_str() {
                        pending.push(location.to_owned());
                    }
                } else if let Some(id) = entry["id"].as_str() {
                    items.insert(id.to_owned(), entry.clone());
                }
            }
            match page["next_offset"].as_u64() {
                Some(next) => offset = next as usize,
                None => break,
            }
        }
    }
    Ok(Value::Object(items))
}

/// Inspector details for a Contents entry: the entry itself and, for a
/// resource, its declaration and parameters.
pub(super) fn project_item_details(
    core: &CoreHandle,
    project: &Value,
    entry: &Value,
) -> Result<Value, String> {
    project_item_details_with(core, project, entry, project_items(core, project)?)
}

/// `project_item_details` with this revision's Contents items already
/// listed (the display caches them per revision).
pub(super) fn project_item_details_with(
    core: &CoreHandle,
    project: &Value,
    entry: &Value,
    items: Value,
) -> Result<Value, String> {
    let mut details = json!({"item":entry,"items":items});
    if entry["kind"] == "asset" {
        details["asset"] = request(
            core,
            Method::ProjectGetAsset,
            json!({"project":project,"asset_id":entry["id"]}),
        )?;
    }
    if entry["kind"] == "resource" && entry["builtin"] != true {
        let resource_id = entry["id"].clone();
        details["resource"] = request(
            core,
            Method::ProjectGetResource,
            json!({"project":project,"resource_id":resource_id}),
        )?;
        let target = json!({"kind":"resource","id":resource_id});
        details["parameters"] = parameter_pages(core, project, &target)?;
        details["parameter_target"] = target;
        let resource = details["resource"].clone();
        attach_schema(
            core,
            &mut details["parameters"],
            resource["kind"].as_str().unwrap_or_default(),
            resource["implementation"].as_str().unwrap_or_default(),
            &resource["texture_value_type"],
        );
    }
    Ok(details)
}

/// Up to 256 parameters of one target, paged 32 at a time.
fn parameter_pages(core: &CoreHandle, project: &Value, target: &Value) -> Result<Value, String> {
    let mut parameters = Vec::new();
    let mut offset = 0;
    loop {
        let page = request(
            core,
            Method::ProjectGetParameters,
            json!({"project":project,"target":target,"offset":offset,"limit":32}),
        )?;
        parameters.extend(
            page["parameters"]
                .as_array()
                .ok_or("invalid parameter page")?
                .iter()
                .cloned(),
        );
        if page["next_offset"].is_null() || parameters.len() >= 256 {
            return Ok(
                json!({"parameters":parameters,"total":page["total"],"next_offset":page["next_offset"]}),
            );
        }
        offset = page["next_offset"]
            .as_u64()
            .ok_or("invalid parameter offset")?;
    }
}

/// Resolve the active Scene's Film component and read its resolution through
/// the same ProjectGetParameters path used to populate the Inspector.
pub(super) fn film_aspect(
    core: &CoreHandle,
    project: &Value,
    scene_id: &str,
) -> Result<f64, String> {
    let scene_id = uuid::Uuid::parse_str(scene_id).map_err(|error| error.to_string())?;
    let node = request(
        core,
        Method::ProjectGetNode,
        json!({"project":project,"node_id":scene_id}),
    )?;
    let Some(target) = node["components"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|component| component["kind"] == "film")
        .map(|component| component["parameter_target"].clone())
        .filter(|target| target["id"].is_string())
    else {
        // No Film directive: the renderer uses PBRT's default 1280x720 film.
        return Ok(1280.0 / 720.0);
    };
    let page = parameter_pages(core, project, &target)?;
    let integer = |name: &str, default: f64| {
        page["parameters"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|parameter| parameter["name"] == name)
            .and_then(|parameter| first_preview_number(&parameter["values"]))
            .filter(|value| *value > 0.0)
            .unwrap_or(default)
    };
    Ok(integer("xresolution", 1280.0) / integer("yresolution", 720.0))
}

/// Parameter previews arrive as `{"kind":"integers","value":["600"]}`: the
/// elements are strings so large integers survive JSON. Floats arrive as
/// `{"kind":"finite","value":42.0}` (non-finite floats have no number).
pub(super) fn first_preview_number(values: &Value) -> Option<f64> {
    let first = &values["value"][0];
    first
        .as_f64()
        .or_else(|| first.as_str().and_then(|text| text.trim().parse().ok()))
        .or_else(|| {
            (first["kind"] == "finite")
                .then(|| first["value"].as_f64())
                .flatten()
        })
}

pub(super) fn camera_parameter_page(
    core: &CoreHandle,
    project: &Value,
    camera_id: &str,
) -> Result<Value, String> {
    let id = uuid::Uuid::parse_str(camera_id).map_err(|error| error.to_string())?;
    parameter_pages(core, project, &json!({"kind":"node","id":id}))
}
/// `--scene` starts a Viewer: import the PBRT file without a destination so
/// the document stays read-only until the user saves it as a project. Returns
/// the job id so its import diagnostics can reach the Console.
pub(super) fn import_viewer(core: &CoreHandle, path: PathBuf) -> Result<Option<String>, String> {
    let path = protocol::path::canonicalize(path).map_err(|e| e.to_string())?;
    let source = path.to_str().ok_or("Scene path is not UTF-8")?;
    let state = request(core, Method::ProjectGetState, json!({}))?;
    let started = request(
        core,
        Method::ProjectImport,
        json!({
            "source_path":source,
            "destination":Value::Null,
            "expected_current":state["context"],
        }),
    )?;
    Ok(started["job_id"].as_str().map(str::to_owned))
}
/// `--project` opens a saved project folder, like File > Open Project.
pub(super) fn open_project(core: &CoreHandle, path: PathBuf) -> Result<Option<String>, String> {
    let path = protocol::path::canonicalize(path).map_err(|e| e.to_string())?;
    let folder = path.to_str().ok_or("Project path is not UTF-8")?;
    let state = request(core, Method::ProjectGetState, json!({}))?;
    let started = request(
        core,
        Method::ProjectOpen,
        json!({"path":folder,"expected_current":state["context"]}),
    )?;
    Ok(started["job_id"].as_str().map(str::to_owned))
}

/// The Contents ▸ New menu summary: one list of creatable
/// implementations per material / texture value type / medium, from the
/// static parameter-schema wire, plus the Inspector's Implementation
/// candidates per scene-graph component kind. For textures a value
/// type counts only if that implementation truly has a table for it (marble
/// has none for float), so the menu never offers a combination the core
/// would refuse.
pub(super) fn resource_schema_summary(core: &CoreHandle) -> Result<Value, String> {
    let schema = |directive: &str| {
        request(
            core,
            Method::ProjectGetParameterSchema,
            json!({"directive":directive,"implementation":"?"}),
        )
    };
    let fixed = |directive: &str| -> Result<Value, String> {
        let reply = schema(directive)?;
        Ok(json!({"implementations":reply["implementations"]}))
    };
    let texture = |texture_value_type: &str| -> Result<Value, String> {
        let reply = request(
            core,
            Method::ProjectGetParameterSchema,
            json!({"directive":"Texture","implementation":"?","texture_value_type":texture_value_type}),
        )?;
        let mut implementations = Vec::new();
        for name in reply["implementations"].as_array().into_iter().flatten() {
            let Some(name) = name.as_str() else { continue };
            let detailed = request(
                core,
                Method::ProjectGetParameterSchema,
                json!({"directive":"Texture","implementation":name,"texture_value_type":texture_value_type}),
            )?;
            if detailed["parameters"].as_array().is_some() {
                implementations.push(name);
            }
        }
        Ok(json!({"implementations":implementations}))
    };
    let mut summary = json!({
        "material":fixed("MakeNamedMaterial")?,
        "texture_float":texture("float")?,
        "texture_spectrum":texture("spectrum")?,
        "medium":fixed("MakeNamedMedium")?,
    });
    // The Inspector's Implementation rows, keyed by component
    // kind: the table's rows, else the export schema's list.
    for (kind, directive) in [
        ("shape", "Shape"),
        ("light", "LightSource"),
        ("area_light", "AreaLightSource"),
        ("camera", "Camera"),
        ("film", "Film"),
        ("sampler", "Sampler"),
        ("integrator", "Integrator"),
        ("pixel_filter", "PixelFilter"),
        ("accelerator", "Accelerator"),
    ] {
        summary[kind] = fixed(directive)?;
    }
    // Use the spectrum selector's names.
    summary["spectra"] = request(core, Method::ProjectListSpectra, json!({}))?["spectra"].clone();
    Ok(summary)
}

/// Locate a just-created resource's Contents entry so the window can select
/// and rename it: the folder the edit aimed at, first page.
pub(super) fn created_resource_entry(
    core: &CoreHandle,
    project: &Value,
    folder: &str,
    id: &Value,
) -> Result<Value, String> {
    // Page through the folder — a large one may sort the new item past
    // its first page.
    let mut offset = 0;
    loop {
        let page = project_contents_page(core, project, folder, offset, 256)?;
        if let Some(found) = page["entries"]
            .as_array()
            .and_then(|entries| entries.iter().find(|entry| entry["id"] == *id).cloned())
        {
            return Ok(found);
        }
        match page["next_offset"].as_u64() {
            Some(next) => offset = next as usize,
            None => {
                return Err("the created resource is not in its folder".to_string());
            }
        }
    }
}

#[cfg(test)]
mod film_aspect_tests {
    use super::first_preview_number;
    use serde_json::json;

    #[test]
    fn reads_the_tagged_floats_of_a_parameter_preview() {
        let fov = json!({"kind":"floats","value":[{"kind":"finite","value":42.0}]});
        assert_eq!(first_preview_number(&fov), Some(42.0));
        let infinite = json!({"kind":"floats","value":[{"kind":"positive_infinity"}]});
        assert_eq!(first_preview_number(&infinite), None);
    }

    #[test]
    fn reads_the_integer_strings_of_a_parameter_preview() {
        let values = json!({"kind":"integers","value":["600"]});
        assert_eq!(first_preview_number(&values), Some(600.0));
        assert_eq!(
            first_preview_number(&json!({"kind":"floats","value":[1.5]})),
            Some(1.5)
        );
        assert_eq!(first_preview_number(&json!([600])), None);
    }
}
