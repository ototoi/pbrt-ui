use super::*;
use project::{
    AssetId, ItemId, NodeId, NodeKind, ParameterValues, ProjectDocument,
    ReferenceIndex as ProjectReferenceIndex, ResourceId,
};
use std::collections::BTreeMap;
fn shortened(text: &str) -> String {
    text.chars().take(256).collect()
}
/// The cache root derived from the store's `Assets/` directory: its parent is
/// the project (or Viewer staging) directory, and the cache lives at
/// `Library/Cache/` under it.
fn cache_root(asset_root: &str) -> Option<String> {
    std::path::Path::new(asset_root).parent().map(|directory| {
        directory
            .join("Library/Cache")
            .to_string_lossy()
            .replace('\\', "/")
    })
}
impl CoreState {
    pub(crate) fn prepare_preview(
        &self,
        request: &Request,
    ) -> Result<super::preview_task::PreparedPreview, ErrorPayload> {
        let (target, project, requested) = match request.method {
            Method::ProjectPreviewMaterial => {
                let p: dto::ProjectPreviewMaterialParams = params(request)?;
                (
                    super::preview_task::PreviewTarget::Material(p.resource_id),
                    p.project,
                    p.size,
                )
            }
            Method::ProjectPreviewAsset => {
                let p: dto::ProjectPreviewAssetParams = params(request)?;
                (
                    super::preview_task::PreviewTarget::Asset(p.asset_id),
                    p.project,
                    p.size,
                )
            }
            _ => {
                let p: dto::ProjectPreviewTextureParams = params(request)?;
                (
                    super::preview_task::PreviewTarget::Texture(p.resource_id),
                    p.project,
                    p.size,
                )
            }
        };
        self.projects.check(&project)?;
        let size = requested.unwrap_or(128);
        if !(8..=512).contains(&size) {
            return Err(invalid("preview size must be 8 to 512 pixels"));
        }
        let active = self
            .projects
            .active
            .as_ref()
            .ok_or_else(|| ErrorPayload::new(ErrorCode::StaleProject, "no active project"))?;
        let document = active.session.document();
        match &target {
            super::preview_task::PreviewTarget::Texture(id)
            | super::preview_task::PreviewTarget::Material(id) => {
                if !document.resources.contains_key(&ResourceId(*id)) {
                    return Err(ErrorPayload::new(ErrorCode::NotFound, "resource not found"));
                }
            }
            super::preview_task::PreviewTarget::Asset(id) => {
                let asset = document
                    .assets
                    .get(&AssetId(*id))
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "asset not found"))?;
                if !dto::IMAGE_PREVIEW_MEDIA_TYPES.contains(&asset.media_type.as_str()) {
                    return Err(invalid("asset preview supports image assets only"));
                }
            }
        }
        let asset_root = active.asset_root.clone();
        let root = asset_root
            .as_deref()
            .and_then(cache_root)
            .ok_or_else(|| invalid("project has no cache"))?;
        Ok(super::preview_task::PreparedPreview {
            request_id: request.request_id.clone(),
            project,
            target,
            size,
            snapshot: active.session.snapshot(),
            asset_root,
            cache_root: std::path::PathBuf::from(root),
            _staging_owner: active.staging_owner.clone(),
        })
    }

    /// Short description of a node in the current project revision, if it still exists.
    pub(crate) fn project_node_summary(&self, id: uuid::Uuid, pinned: &dto::ProjectRef) -> Value {
        if self.projects.reference().as_ref() != Some(pinned) {
            return json!({"current": false});
        }
        let node = self.projects.active.as_ref().and_then(|active| {
            let document = active.session.document();
            document
                .scene_view(active.session.active_scene())
                .ok()
                .and_then(|view| view.node(NodeId(id)))
        });
        match node {
            Some(node) => {
                let detail = match &node.data {
                    NodeKind::Entity {
                        entity,
                        implementation,
                        ..
                    } => json!({"entity":entity,"implementation":implementation}),
                    NodeKind::Instance { definition } => json!({"definition":definition}),
                    _ => Value::Null,
                };
                json!({"current":true,"kind":node_kind(&node.data),"label":shortened(&node.label),"detail":detail})
            }
            None => json!({"current": false}),
        }
    }
    pub(super) fn project_query(&self, request: &Request) -> Result<Value, ErrorPayload> {
        let active = self
            .projects
            .active
            .as_ref()
            .ok_or_else(|| ErrorPayload::new(ErrorCode::StaleProject, "no active project"))?;
        let doc = active.session.document();
        let scene_view = || {
            doc.scene_view(active.session.active_scene())
                .map_err(crate::project_io::project_error)
        };
        let parameters_of =
            |target: &dto::ParameterTarget| -> Result<&Vec<project::Parameter>, ErrorPayload> {
                Ok(match target {
                    dto::ParameterTarget::Node(id) => match &scene_view()?
                        .node(NodeId(*id))
                        .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "node not found"))?
                        .data
                    {
                        NodeKind::Entity { parameters, .. }
                        | NodeKind::Configuration { parameters, .. } => parameters,
                        _ => return Err(invalid("node does not declare parameters")),
                    },
                    dto::ParameterTarget::Resource(id) => {
                        &doc.resources
                            .get(&ResourceId(*id))
                            .ok_or_else(|| {
                                ErrorPayload::new(ErrorCode::NotFound, "resource not found")
                            })?
                            .parameters
                    }
                })
            };
        match request.method {
            Method::ProjectListNodes => {
                let view = scene_view()?;
                let scene = view.scene;
                let p: dto::ListProjectNodesParams = params(request)?;
                self.projects.check(&p.project)?;
                self.check_limit(p.limit)?;
                if p.view == dto::NodeView::SceneGraph {
                    return list_scene_graph(&view, p);
                }
                let ids = if let Some(parent) = p.parent {
                    let node = view.node(NodeId(parent)).ok_or_else(|| {
                        ErrorPayload::new(ErrorCode::NotFound, "parent node not found")
                    })?;
                    match &node.data {
                        NodeKind::Scope { children, .. } => children.clone(),
                        _ => vec![],
                    }
                } else {
                    scene
                        .options
                        .iter()
                        .copied()
                        .chain([scene.world])
                        .collect::<Vec<_>>()
                };
                if p.offset > ids.len() {
                    return Err(invalid("node offset exceeds child count"));
                }
                let nodes=ids.iter().skip(p.offset).take(p.limit).map(|id|{
                    let node=view.node(*id).expect("listed node exists");let count=match &node.data{NodeKind::Scope{children,..}=>children.len(),_=>0};
                    json!({"node_id":id,"label":shortened(&node.label),"label_truncated":node.label.chars().count()>256,"edition":node.edition,"kind":node_kind(&node.data),"child_count":count})
                }).collect::<Vec<_>>();
                let next = p.offset + nodes.len();
                Ok(
                    json!({"project":p.project,"nodes":nodes,"total":ids.len(),"next_offset":(next<ids.len()).then_some(next)}),
                )
            }
            Method::ProjectListReferences => {
                // where one item is used. Each row
                // names its kind - a scene node, an object definition's inner
                // node through the definition's resource, or a resource -
                // and what the reference is through (`via`).
                let p: dto::ListReferencesParams = params(request)?;
                self.projects.check(&p.project)?;
                self.check_limit(p.limit)?;
                let item = match p.item {
                    dto::ProjectItemId::Scene(id) => project::ItemId::Scene(project::SceneId(id)),
                    dto::ProjectItemId::Resource(id) => {
                        project::ItemId::Resource(project::ResourceId(id))
                    }
                    dto::ProjectItemId::Asset(id) => project::ItemId::Asset(project::AssetId(id)),
                };
                let exists = match item {
                    project::ItemId::Scene(id) => doc.scenes.contains_key(&id),
                    project::ItemId::Resource(id) => doc.resources.contains_key(&id),
                    project::ItemId::Asset(id) => doc.assets.contains_key(&id),
                };
                if !exists {
                    return Err(ErrorPayload::new(
                        ErrorCode::NotFound,
                        "project item not found",
                    ));
                }
                let references = active.session.reference_index();
                let all = references.references(item);
                let total = all.len();
                if p.offset > total {
                    return Err(invalid("reference offset exceeds the total"));
                }
                let item = json!(match p.item {
                    dto::ProjectItemId::Scene(id) => json!({"kind":"scene","id":id}),
                    dto::ProjectItemId::Resource(id) => json!({"kind":"resource","id":id}),
                    dto::ProjectItemId::Asset(id) => json!({"kind":"asset","id":id}),
                });
                let parents = reference_parents(doc);
                let definitions = definition_parents(doc);
                let rows = all
                    .iter()
                    .skip(p.offset)
                    .take(p.limit)
                    .map(|reference| {
                        reference_row(doc, &references, &parents, &definitions, reference)
                    })
                    .collect::<Vec<_>>();
                let next = p.offset + rows.len();
                Ok(json!({
                    "project":p.project,"item":item,"total":total,
                    "next_offset":(next < total).then_some(next),"references":rows,
                }))
            }
            Method::ProjectListContents => {
                let p: dto::ListProjectContentsParams = params(request)?;
                self.projects.check(&p.project)?;
                self.check_limit(p.limit)?;
                // `folder` omitted lists the Assets root (the flat listing
                // This path was removed.
                let active = self.projects.active.as_ref().unwrap();
                list_folder_contents(
                    doc,
                    // one index per revision, shared with the counts
                    // and list_references.
                    &active.session.reference_index(),
                    &active.unregistered,
                    p.folder.as_deref().unwrap_or(""),
                    p.offset,
                    p.limit,
                )
            }
            Method::ProjectSearchContents => {
                let p: dto::SearchProjectContentsParams = params(request)?;
                self.projects.check(&p.project)?;
                self.check_limit(p.limit)?;
                let query = p.query.trim();
                if query.is_empty() {
                    return Err(invalid("Contents search query must not be empty"));
                }
                if query.chars().count() > 256 {
                    return Err(invalid("Contents search query exceeds 256 characters"));
                }
                search_project_contents(
                    doc,
                    &active.session.reference_index(),
                    &active.unregistered,
                    &p.project,
                    query,
                    p.offset,
                    p.limit,
                )
            }
            Method::ProjectListScenes => {
                let p: dto::ProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                let mut scenes = doc.scenes.values().collect::<Vec<_>>();
                scenes.sort_by(|a, b| {
                    a.label
                        .to_lowercase()
                        .cmp(&b.label.to_lowercase())
                        .then(a.id.cmp(&b.id))
                });
                let active_scene = active.session.active_scene();
                Ok(json!({
                    "project":p.project,
                    "scenes":scenes.into_iter().map(|scene| json!({
                        "id":scene.id,
                        "label":scene.label,
                        "location":scene.location,
                        "default":Some(scene.id) == doc.default_scene,
                        "active":Some(scene.id) == active_scene,
                        "node_count":scene.nodes.len(),
                        "coordinate_system":scene.coordinate_system,
                    })).collect::<Vec<_>>(),
                }))
            }
            Method::ProjectListPresets => {
                let p: dto::ProjectParams = params(request)?;
                self.projects.check(&p.project)?;
                Ok(json!({"project":p.project,"presets":builtin_presets()}))
            }
            Method::ProjectSearchNodes => {
                let view = scene_view()?;
                let scene = view.scene;
                let p: dto::SearchProjectNodesParams = params(request)?;
                self.projects.check(&p.project)?;
                let scene_graph = p.view == dto::NodeView::SceneGraph;
                self.check_limit(p.limit)?;
                if p.query.trim().is_empty() {
                    return Err(invalid("node search query must not be empty"));
                }
                if p.query.chars().count() > 256 {
                    return Err(invalid("node search query exceeds 256 characters"));
                }
                // Parent links from every scope once, so hit paths resolve without
                // walking the full tree per hit.
                let parents = parent_links(scene.nodes.values());
                let needle = p.query.to_lowercase();
                let matches: Vec<_> = scene
                    .nodes
                    .iter()
                    .filter(|(_, node)| {
                        !scene_graph || scene_graph::visible_object_kind(&view, node.id).is_some()
                    })
                    .filter(|(_, node)| node.label.to_lowercase().contains(&needle))
                    .map(|(id, _)| id)
                    .collect::<Vec<_>>();
                let total = matches.len();
                if p.offset > total {
                    return Err(invalid("node search offset exceeds match count"));
                }
                let mut search_hits = Vec::new();
                for id in matches.iter().skip(p.offset).take(p.limit) {
                    let node = &scene.nodes[id];
                    let count = match &node.data {
                        NodeKind::Scope { children, .. } => children.len(),
                        _ => 0,
                    };
                    let path = if scene_graph {
                        scene_graph::path(&view, **id)
                    } else {
                        ancestors(&parents, **id)
                    };
                    let count = if scene_graph {
                        scene_graph::object_children(&view, **id).len()
                    } else {
                        count
                    };
                    search_hits.push(
                        json!({"node_id":id.0,"label":shortened(&node.label),"label_truncated":node.label.chars().count()>256,"edition":node.edition,"kind":node_kind(&node.data),"child_count":count,"path":path}),
                    );
                }
                let next = p.offset + search_hits.len();
                Ok(json!({
                    "project":p.project,"nodes":search_hits,"total":total,
                    "next_offset":(next<total).then_some(next),
                }))
            }
            Method::ProjectGetNode => {
                let view = scene_view()?;
                let scene = view.scene;
                let p: dto::ProjectNodeParams = params(request)?;
                self.projects.check(&p.project)?;
                if p.node_id == scene.id.0 {
                    return Ok(json!({
                        "project":p.project,"node_id":scene.id,"label":scene.label,
                        "edition":Value::Null,"origin":Value::Null,
                        "data":{"kind":"scene","child_count":scene_graph::scene_children(&view).len(),
                                "coordinate_system":scene.coordinate_system},
                        "components":scene_graph::scene_components(&view),
                    }));
                }
                let node = view
                    .node(NodeId(p.node_id))
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "node not found"))?;
                let data = match &node.data {
                    NodeKind::Scope {
                        scope,
                        children,
                        owned_object,
                        ..
                    } => json!({
                        "kind":"scope","scope":scope,"child_count":children.len(),
                        "owned_object":owned_object,
                    }),
                    NodeKind::State { operation } => json!({"kind":"state","operation":operation}),
                    NodeKind::Entity {
                        entity,
                        implementation,
                        parameters,
                        ..
                    } => {
                        json!({"kind":"entity","entity":entity,"implementation":implementation,"parameter_count":parameters.len()})
                    }
                    NodeKind::Configuration {
                        configuration,
                        implementation,
                        parameters,
                    } => {
                        json!({"kind":"configuration","configuration":configuration,"implementation":implementation,"parameter_count":parameters.len()})
                    }
                    NodeKind::Instance { definition } => {
                        json!({"kind":"instance","definition":definition})
                    }
                    NodeKind::Option { name, value } => {
                        json!({"kind":"option","name":name,"value":value})
                    }
                };
                // Components are derived by evaluating the scene; a scene the
                // evaluator rejects still reports the node itself.
                let (components, components_error) = match scene_graph::Components::new(view) {
                    Ok(components) => (
                        json!(
                            components
                                .with_asset_root(active.asset_root.as_deref())
                                .detail(node.id)
                        ),
                        Value::Null,
                    ),
                    Err(error) => (Value::Null, json!(error.message)),
                };
                // Ancestors nearest first, as a directives search hit has
                // them, so a viewport pick can reveal the node in a tree.
                let path = if scene_graph::visible_object_kind(&view, node.id).is_some() {
                    scene_graph::path(&view, node.id)
                } else if let Some(owner) =
                    scene_graph::visible_owner(&view, node.id).or(match &node.data {
                        NodeKind::Scope {
                            owned_object: Some(owner),
                            ..
                        } => Some(*owner),
                        _ => None,
                    })
                {
                    scene_graph::path(&view, owner)
                } else {
                    ancestors(&parent_links(scene.nodes.values()), node.id)
                };
                // The object definition holding a definition node, so a client
                // can tell without having paged the definition's tree.
                let definition = view
                    .document
                    .definition_nodes
                    .contains_key(&node.id)
                    .then(|| {
                        let parents = parent_links(view.document.definition_nodes.values());
                        let root = ancestors(&parents, node.id)
                            .last()
                            .copied()
                            .map_or(node.id, project::NodeId);
                        view.document
                            .resources
                            .values()
                            .find(|resource| resource.definition_scope == Some(root))
                            .map(|resource| resource.id)
                    })
                    .flatten();
                Ok(
                    json!({"project":p.project,"node_id":node.id,"label":node.label,"active":node.active,"object":scene_graph::visible_object_kind(&view, node.id),"edition":node.edition,"origin":node.origin,"data":data,"components":components,"components_error":components_error,"path":path,"definition":definition}),
                )
            }
            Method::ProjectGetAsset => {
                let p: dto::ProjectAssetParams = params(request)?;
                self.projects.check(&p.project)?;
                let asset = doc
                    .assets
                    .get(&project::AssetId(p.asset_id))
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "asset not found"))?;
                // the session's per-revision cached index.
                let references = active.session.reference_index();
                // The file on disk, for viewers that read it directly (the
                // desktop Image Viewer); null for format 1 projects.
                let path = active.asset_root.as_deref().and_then(|root| {
                    active
                        .session
                        .snapshot()
                        .folder_asset_path(std::path::Path::new(root), asset.id)
                        .ok()
                        .and_then(|path| path.to_str().map(str::to_owned))
                });
                Ok(json!({
                                   "project":p.project,"asset_id":asset.id,"path":path,"name":location_name(&asset.location),
                                   "location":asset.location,"media_type":asset.media_type,
                                   "size":asset.size.to_string(),"sha256":asset.sha256,"origin":asset.origin,
                                   "reference_count":references.count(ItemId::Asset(asset.id)),
                // a PLY mesh's header statistics (null otherwise).
                // An unsaved import's bytes live in the session's staged
                // payload, so the statistics fall back to that path until
                // a save writes the assets folder.
                                   "mesh":(asset.media_type == "application/x-ply")
                                       .then(|| {
                                           active.asset_root.as_deref()
                                               .and_then(|root| super::scene_graph::ply_info_of(asset, root))
                                               .or_else(|| {
                                                   active.session.snapshot().payload_by(asset.id).and_then(
                                                       |lease| {
                                                           super::scene_graph::ply_info_at(
                                                               asset,
                                                               std::path::Path::new(lease.path()),
                                                           )
                                                       },
                                                   )
                                               })
                                       })
                                       .flatten(),
                               }))
            }
            // The regenerable cache. Reads and writes
            // only touch `Library/Cache/`; the document and its revision are
            // unaffected, and Viewer staging serves it as well.
            Method::ProjectGetCache => {
                let p: dto::ProjectCacheGetParams = params(request)?;
                self.projects.check(&p.project)?;
                let root = active.asset_root.as_deref().and_then(cache_root);
                let Some(root) = root else {
                    return Err(invalid("project has no cache"));
                };
                let hit =
                    project::cache_get(std::path::Path::new(&root), &p.item.to_string(), &p.key)
                        .map_err(crate::project_io::project_error)?;
                Ok(match hit {
                    Some(hit) => json!({
                        "project":p.project,"hit":true,
                        "path":hit.path.to_string_lossy().replace('\\', "/"),
                        "media_type":hit.media_type,
                    }),
                    None => json!({"project":p.project,"hit":false}),
                })
            }
            Method::ProjectPutCache => {
                let p: dto::ProjectCachePutParams = params(request)?;
                self.projects.check(&p.project)?;
                let root = active.asset_root.as_deref().and_then(cache_root);
                let Some(root) = root else {
                    return Err(invalid("project has no cache"));
                };
                let data = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    p.data_base64.as_bytes(),
                )
                .map_err(|_| invalid("cache data is not valid base64"))?;
                if data.len() > 4 * 1024 * 1024 {
                    return Err(invalid("cache entry exceeds 4 MiB"));
                }
                if p.media_type.is_empty() || p.media_type.len() > 128 {
                    return Err(invalid("cache media type must be 1 to 128 bytes"));
                }
                let path = project::cache_put(
                    std::path::Path::new(&root),
                    &p.item.to_string(),
                    &p.key,
                    &p.kind,
                    &p.media_type,
                    &data,
                    project::DEFAULT_CACHE_LIMIT,
                )
                .map_err(crate::project_io::project_error)?;
                Ok(json!({
                    "project":p.project,
                    "path":path.to_string_lossy().replace('\\', "/"),
                }))
            }
            Method::ProjectGetResource => {
                let p: dto::ProjectResourceParams = params(request)?;
                self.projects.check(&p.project)?;
                let r = doc
                    .resources
                    .get(&ResourceId(p.resource_id))
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "resource not found"))?;
                Ok(
                    json!({"project":p.project,"resource_id":r.id,"label":r.label,"edition":r.edition,"kind":r.kind,"implementation":r.implementation,"texture_value_type":r.texture_value_type,"definition_scope":r.definition_scope,"parameter_count":r.parameters.len(),"origin":r.origin}),
                )
            }
            // Evaluate a texture and its upstream,
            // or shade a material's sphere, into the project cache; the key
            // follows every input, so a hit is fresh. B: an image asset's
            // preview takes the same route, keyed by its content.
            Method::ProjectPreviewTexture
            | Method::ProjectPreviewMaterial
            | Method::ProjectPreviewAsset => self
                .prepare_preview(request)?
                .execute_inner(&std::sync::atomic::AtomicBool::new(false)),
            Method::ProjectGetResourceGraph => {
                let p: dto::ProjectResourceGraphParams = params(request)?;
                self.projects.check(&p.project)?;
                let limit = p.limit.unwrap_or(256);
                if !(1..=4096).contains(&limit) {
                    return Err(invalid("graph limit must be 1 to 4096 nodes"));
                }
                let mut graph =
                    super::resource_graph::upstream(doc, ResourceId(p.resource_id), limit)?;
                graph["project"] = json!(p.project);
                Ok(graph)
            }
            // one parameter's spectrum-ish value as the swatch rgb
            // and its human source, through the spectrum crate Lit uses.
            Method::ProjectSpectrumColor => {
                let p: dto::SpectrumColorParams = params(request)?;
                self.projects.check(&p.project)?;
                super::spectrum_color::resolve(doc, active, &p)
            }
            Method::ProjectPlanImplementation => {
                let p: dto::ProjectPlanImplementationParams = params(request)?;
                self.projects.check(&p.project)?;
                let super::edits::ImplementationPlan {
                    edit,
                    change,
                    removals,
                } = super::edits::plan_implementation(
                    doc,
                    active.session.active_scene(),
                    p.target.clone(),
                    &p.implementation,
                )?;
                let project::Edit::SetImplementation { implementation, .. } = edit else {
                    unreachable!("plan_implementation plans set_implementation");
                };
                // The edit list that applies the change in one transaction
                // (empty when nothing changes): the removals, then the
                // change itself.
                let mut edits = removals;
                if change.is_some() {
                    edits.push(json!({"kind":"set_implementation","target":p.target,
                                      "implementation":implementation,
                                      "parameters":"keep_compatible"}));
                }
                let changed = change.is_some();
                let (kept_unknown, breaks_reference) = change
                    .map(|change| (change.kept_unknown, change.breaks_reference))
                    .unwrap_or_default();
                Ok(json!({
                    "project":p.project,
                    "target":p.target,
                    "implementation":implementation,
                    "changed":changed,
                    "kept_unknown":kept_unknown,
                    "breaks_reference":breaks_reference,
                    "edits":edits,
                }))
            }
            Method::ProjectGetParameters => {
                let p: dto::ProjectParametersParams = params(request)?;
                self.projects.check(&p.project)?;
                self.check_limit(p.limit)?;
                let params = parameters_of(&p.target)?;
                if p.offset > params.len() {
                    return Err(invalid("parameter offset exceeds count"));
                }
                let values=params.iter().skip(p.offset).take(p.limit.min(32)).map(|p|{
                    let (count,values)=parameter_preview(&p.values);
                    let shortened_text = matches!(&p.values, ParameterValues::Strings(v) | ParameterValues::BuiltinSpectra(v) if v.iter().any(|s| s.chars().count() > 256));
                    json!({"name":p.name,"value_type":p.value_type,"color_space":p.color_space,"value_count":count.to_string(),"values":values,"preview_truncated":count>32 || shortened_text})
                }).collect::<Vec<_>>();
                let next = p.offset + values.len();
                Ok(
                    json!({"project":p.project,"parameters":values,"total":params.len(),"next_offset":(next<params.len()).then_some(next)}),
                )
            }
            Method::ProjectGetParameterValues => {
                let p: dto::ProjectParameterValuesParams = params(request)?;
                self.projects.check(&p.project)?;
                if !(1..=4096).contains(&p.limit) {
                    return Err(invalid("value limit must be between 1 and 4096"));
                }
                let parameter = parameters_of(&p.target)?
                    .iter()
                    .find(|parameter| parameter.name == p.name)
                    .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "parameter not found"))?;
                let (total, values) =
                    parameter_window(&parameter.values, p.offset, p.limit, |asset| {
                        let root = active
                            .asset_root
                            .as_deref()
                            .ok_or_else(|| invalid("this project keeps no data assets on disk"))?;
                        active
                            .session
                            .snapshot()
                            .folder_asset_path(std::path::Path::new(root), asset)
                            .map_err(crate::project_io::project_error)
                    })?;
                let count = values["value"]
                    .as_array()
                    .or_else(|| values["ids"].as_array())
                    .map_or(0, Vec::len);
                let next = p.offset + count;
                Ok(json!({
                    "project":p.project,"name":parameter.name,"value_type":parameter.value_type,
                    "total":total.to_string(),"offset":p.offset,"values":values,
                    "next_offset":((next as u64) < total).then_some(next),
                }))
            }
            _ => Err(ErrorPayload::new(
                ErrorCode::Unsupported,
                "unsupported project query",
            )),
        }
    }
}

/// Built-in presets in Create-menu order: category order, then label.
fn builtin_presets() -> Vec<Value> {
    let presets = project::presets::BUILTIN_PRESETS;
    let mut sorted = presets.iter().collect::<Vec<_>>();
    sorted.sort_by_key(|preset| {
        (
            presets
                .iter()
                .position(|other| other.category == preset.category),
            preset.label.to_lowercase(),
        )
    });
    sorted
        .into_iter()
        .map(|preset| {
            json!({
                "id":preset.id.0,"name":preset.name,"category":preset.category,
                "label":preset.label,"builtin":true,
                "creates":project::presets::creates(preset),
            })
        })
        .collect()
}

fn homogeneous_sigma_t(resource: &project::Resource) -> Option<f64> {
    if resource.kind != project::ResourceKind::Medium || resource.implementation != "homogeneous" {
        return None;
    }
    let coefficient = |name: &str| -> Option<f64> {
        let Some(parameter) = resource
            .parameters
            .iter()
            .find(|parameter| parameter.name == name)
        else {
            return Some(1.);
        };
        match &parameter.values {
            ParameterValues::Floats(values) if !values.is_empty() => {
                let values = values
                    .iter()
                    .map(|value| match value {
                        project::FloatValue::Finite(value) => Some(*value),
                        _ => None,
                    })
                    .collect::<Option<Vec<_>>>()?;
                Some(values.iter().sum::<f64>() / values.len() as f64)
            }
            _ => None,
        }
    };
    let sigma = coefficient("sigma_a")? + coefficient("sigma_s")?;
    (sigma.is_finite() && sigma >= 0.).then_some(sigma)
}

fn scene_uses_medium(view: &project::SceneView, components: &scene_graph::Components<'_>) -> bool {
    let mut pending = scene_graph::scene_children(view);
    while let Some(id) = pending.pop() {
        let object = scene_graph::visible_object_kind(view, id).unwrap_or("");
        if matches!(object, "camera" | "shape")
            && components.detail(id).iter().any(|component| {
                component["kind"] == "medium"
                    && ["binding", "inside", "outside"]
                        .iter()
                        .any(|key| component[*key].is_string())
            })
        {
            return true;
        }
        pending.extend(scene_graph::object_children(view, id));
    }
    false
}

fn list_scene_graph(
    view: &project::SceneView,
    p: dto::ListProjectNodesParams,
) -> Result<Value, ErrorPayload> {
    let scene = view.scene;
    let components = scene_graph::Components::new(*view)?;
    let ids = match p.parent {
        None => Vec::new(),
        Some(parent) if parent == scene.id.0 => scene_graph::scene_children(view),
        Some(parent) => {
            view.node(NodeId(parent))
                .ok_or_else(|| ErrorPayload::new(ErrorCode::NotFound, "parent node not found"))?;
            if scene_graph::visible_object_kind(view, NodeId(parent)).is_none() {
                return Err(invalid("parent is not visible in the scene graph"));
            }
            scene_graph::object_children(view, NodeId(parent))
        }
    };
    let total = if p.parent.is_none() { 1 } else { ids.len() };
    if p.offset > total {
        return Err(invalid("node offset exceeds child count"));
    }
    let mut nodes = Vec::new();
    if p.parent.is_none() && p.offset == 0 && p.limit > 0 {
        nodes.push(json!({
            "node_id":scene.id,"label":shortened(&scene.label),
            "label_truncated":scene.label.chars().count()>256,"edition":Value::Null,
            "kind":"scene","object":"scene",
            "coordinate_system":scene.coordinate_system,
            "child_count":scene_graph::scene_children(view).len(),
            "components":[],
            "medium_used":scene_uses_medium(view, &components),
        }));
    }
    for id in ids.iter().skip(p.offset).take(p.limit) {
        let node = view.node(*id).expect("listed node exists");
        let kind = scene_graph::visible_object_kind(view, *id).unwrap_or("");
        let medium = components
            .detail(*id)
            .into_iter()
            .find(|component| component["kind"] == "medium");
        let medium_ids = medium
            .as_ref()
            .map(|component| {
                [
                    component["binding"].as_str(),
                    component["inside"].as_str(),
                    component["outside"].as_str(),
                ]
            })
            .unwrap_or([None, None, None]);
        let medium_bound = medium_ids.iter().any(Option::is_some);
        let medium_sigma_t = (kind == "camera")
            .then(|| {
                medium_ids.iter().flatten().find_map(|id| {
                    let id = id.parse::<uuid::Uuid>().ok()?;
                    homogeneous_sigma_t(view.document.resources.get(&ResourceId(id))?)
                })
            })
            .flatten();
        nodes.push(json!({
            "node_id":id,"label":shortened(&node.label),
            "label_truncated":node.label.chars().count()>256,"edition":node.edition,"active":node.active,
            "kind":node_kind(&node.data),"object":kind,
            "child_count":scene_graph::object_children(view, *id).len(),
            "components":components.summary(*id),
            "render_camera":view.scene.render_camera == Some(*id),
            "medium_bound":medium_bound,"medium_sigma_t":medium_sigma_t,
        }));
    }
    let next = p.offset + nodes.len();
    Ok(
        json!({"project":p.project,"nodes":nodes,"total":total,"next_offset":(next<total).then_some(next)}),
    )
}

/// one where-used row for one reference. Shared
/// by `project.list_references` and the delete refusal's `references`
/// detail, so the two always agree.
pub(super) fn reference_row(
    document: &ProjectDocument,
    index: &ProjectReferenceIndex,
    parents: &std::collections::BTreeMap<(project::SceneId, project::NodeId), project::NodeId>,
    definition_parents: &std::collections::BTreeMap<project::NodeId, project::NodeId>,
    reference: &project::ReferenceLocation,
) -> Value {
    let via = reference.via.wire();
    if let Some(resource) = reference.resource {
        // A resource's reference: its parameter, or the declaring
        // context's binding.
        let resource = &document.resources[&resource];
        return json!({
            "kind":"resource",
            "resource":{
                "id":resource.id,"label":shortened(&resource.label),
                "resource_kind":resource.kind,
            },
            "via":via,
        });
    }
    let Some(node_id) = reference.node else {
        return Value::Null;
    };
    // The index points at the node carrying the reference, usually a
    // binding state. The list names the object that holds it; for dedicated
    // wrappers, that is the containing group one hop up.
    match reference.scene {
        Some(scene_id) => {
            let scene = &document.scenes[&scene_id];
            let node = &scene.nodes[&node_id];
            #[allow(clippy::type_complexity)]
            let (shown_id, shown_label, object): (
                project::NodeId,
                String,
                Option<Value>,
            ) = match (
                scene_graph::visible_object_kind(&project::SceneView { document, scene }, node_id),
                parents.get(&(scene_id, node_id)),
            ) {
                (Some(kind), _) => (node_id, shortened(&node.label), Some(Value::from(kind))),
                (None, Some(parent)) => {
                    let visible_owner = scene_graph::visible_owner(
                        &project::SceneView { document, scene },
                        node_id,
                    );
                    let parent_id = visible_owner.unwrap_or(*parent);
                    let parent = &scene.nodes[&parent_id];
                    (
                        parent.id,
                        shortened(&parent.label),
                        scene_graph::visible_object_kind(
                            &project::SceneView { document, scene },
                            parent.id,
                        )
                        .map(Value::from),
                    )
                }
                (None, None) => (node_id, shortened(&node.label), None),
            };
            json!({
                           "kind":"node",
                           "scene":{"id":scene.id,"label":shortened(&scene.label)},
                           "node":{"id":shown_id,"label":shown_label,"object":object,
            // ancestor ids, immediate parent first, for
            // lazy-tree reveals - the same shape
            // `project.search_nodes` reports.
                                   "path":scene_graph::path(
                                       &project::SceneView { document, scene },
                                       shown_id,
                                   )},
                           "via":via,
                       })
        }
        None => {
            // A definition's inner node: name the owning definition,
            // revealed through its resource.
            let owner = index
                .definition_owner(node_id)
                .and_then(|id| document.resources.get(&id));
            let node = &document.definition_nodes[&node_id];
            #[allow(clippy::type_complexity)]
            let (shown_id, shown_label, object): (
                project::NodeId,
                String,
                Option<Value>,
            ) = match (
                definition_visible_object_kind(node),
                definition_parents.get(&node_id),
            ) {
                (Some(kind), _) => (node_id, shortened(&node.label), Some(Value::from(kind))),
                (None, Some(parent)) => {
                    let parent_node = &document.definition_nodes[parent];
                    let shown_parent = match &parent_node.data {
                        NodeKind::Scope {
                            owned_object: Some(owned),
                            ..
                        } => *owned,
                        _ => *parent,
                    };
                    let parent = &document.definition_nodes[&shown_parent];
                    (
                        parent.id,
                        shortened(&parent.label),
                        definition_visible_object_kind(parent).map(Value::from),
                    )
                }
                (None, None) => (node_id, shortened(&node.label), None),
            };
            json!({
                "kind":"definition_node",
                "definition":owner.map(|resource| json!({
                    "id":resource.id,"label":shortened(&resource.label),
                    "resource_kind":resource.kind,
                })),
                "node":{"id":shown_id,"label":shown_label,"object":object},
                "via":via,
            })
        }
    }
}

fn definition_visible_object_kind(node: &project::Node) -> Option<&'static str> {
    match &node.data {
        NodeKind::Scope {
            owned_object: Some(_),
            ..
        } => None,
        data => scene_graph::object_kind(data),
    }
}

/// Parents of every scene node: which node's children a node is, so a
/// binding state's row can name the object holding it.
pub(super) fn reference_parents(
    document: &ProjectDocument,
) -> std::collections::BTreeMap<(project::SceneId, project::NodeId), project::NodeId> {
    let mut parents = std::collections::BTreeMap::new();
    for (scene, document_scene) in &document.scenes {
        for node in document_scene.nodes.values() {
            if let project::NodeKind::Scope { children, .. } = &node.data {
                for child in children {
                    parents.insert((*scene, *child), node.id);
                }
            }
        }
    }
    parents
}

/// Parents of the definition tree's nodes.
pub(super) fn definition_parents(
    document: &ProjectDocument,
) -> std::collections::BTreeMap<project::NodeId, project::NodeId> {
    let mut parents = std::collections::BTreeMap::new();
    for node in document.definition_nodes.values() {
        if let project::NodeKind::Scope { children, .. } = &node.data {
            for child in children {
                parents.insert(*child, node.id);
            }
        }
    }
    parents
}

fn list_folder_contents(
    document: &ProjectDocument,
    references: &ProjectReferenceIndex,
    unregistered: &[String],
    folder: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, ErrorPayload> {
    if folder == "@builtin" {
        let mut entries = project::presets::BUILTIN_PRESETS
            .iter()
            .map(|preset| {
                json!({
                    "kind":"resource",
                    "id":preset.id.0,
                    "name":preset.label,
                    "location":"@builtin",
                    "resource_kind":"preset",
                    "media_type":Value::Null,
                    "reference_count":0,
                    "builtin":true,
                })
            })
            .collect::<Vec<_>>();
        entries.sort_by(|a, b| {
            a["name"]
                .as_str()
                .unwrap_or_default()
                .to_lowercase()
                .cmp(&b["name"].as_str().unwrap_or_default().to_lowercase())
        });
        return paged_folder_contents("@builtin", entries, offset, limit);
    }
    if !valid_folder_path(folder) {
        return Err(invalid("invalid Contents folder path"));
    }
    if !folder.is_empty() && !document.folders.contains(folder) {
        return Err(ErrorPayload::new(
            ErrorCode::NotFound,
            "Contents folder not found",
        ));
    }

    let mut entries = Vec::<(u8, String, String, Value)>::new();
    for path in &document.folders {
        if parent_location(path) == folder && path != folder {
            let name = location_name(path).to_owned();
            entries.push((
                0,
                name.to_lowercase(),
                path.to_lowercase(),
                json!({
                    "kind":"folder", "name":name, "location":path,
                }),
            ));
        }
    }
    for scene in document.scenes.values() {
        if parent_location(&scene.location) == folder {
            let name = scene.label.clone();
            entries.push((
                1,
                name.to_lowercase(),
                scene.location.to_lowercase(),
                json!({
                    "kind":"scene", "id":scene.id, "name":name, "location":scene.location,
                    "resource_kind":Value::Null, "media_type":Value::Null,
                    "reference_count":references.count(ItemId::Scene(scene.id)),
                }),
            ));
        }
    }
    for resource in document.resources.values() {
        if parent_location(&resource.location) == folder {
            let name = resource.label.clone();
            entries.push((
                1,
                name.to_lowercase(),
                resource.location.to_lowercase(),
                json!({
                    "kind":"resource", "id":resource.id, "name":name, "location":resource.location,
                    "resource_kind":resource.kind, "media_type":Value::Null,
                    "texture_value_type":resource.texture_value_type,
                    "reference_count":references.count(ItemId::Resource(resource.id)),
                }),
            ));
        }
    }
    for asset in document.assets.values() {
        if parent_location(&asset.location) == folder {
            let name = location_name(&asset.location).to_owned();
            entries.push((
                1,
                name.to_lowercase(),
                asset.location.to_lowercase(),
                json!({
                    "kind":"asset", "id":asset.id, "name":name, "location":asset.location,
                    "resource_kind":Value::Null, "media_type":asset.media_type,
                    "reference_count":references.count(ItemId::Asset(asset.id)),
                }),
            ));
        }
    }
    for location in unregistered {
        if parent_location(location) == folder {
            let name = location_name(location).to_owned();
            entries.push((
                1,
                name.to_lowercase(),
                location.to_lowercase(),
                json!({
                    "kind":"unregistered", "name":name, "location":location,
                    "resource_kind":asset_kind(location), "media_type":Value::Null,
                    "reference_count":0,
                }),
            ));
        }
    }
    entries.sort_by(|a, b| (a.0, &a.1, &a.2).cmp(&(b.0, &b.1, &b.2)));
    paged_folder_contents(
        folder,
        entries.into_iter().map(|(_, _, _, value)| value).collect(),
        offset,
        limit,
    )
}

fn search_project_contents(
    document: &ProjectDocument,
    references: &ProjectReferenceIndex,
    unregistered: &[String],
    project_ref: &dto::ProjectRef,
    query: &str,
    offset: usize,
    limit: usize,
) -> Result<Value, ErrorPayload> {
    let needle = query.to_lowercase();
    let mut rows = Vec::<(String, String, String, String, Value)>::new();
    let mut push = |row: Value| {
        let name = row["name"].as_str().unwrap_or_default().to_owned();
        let location = row["location"].as_str().unwrap_or_default().to_owned();
        let kind = row["kind"].as_str().unwrap_or_default().to_owned();
        let id = row["id"].as_str().unwrap_or_default().to_owned();
        if name.to_lowercase().contains(&needle) || location.to_lowercase().contains(&needle) {
            rows.push((name.to_lowercase(), location.to_lowercase(), kind, id, row));
        }
    };
    for location in &document.folders {
        push(json!({"kind":"folder","name":location_name(location),"location":location}));
    }
    for scene in document.scenes.values() {
        push(json!({
            "kind":"scene","id":scene.id,"name":scene.label,"location":scene.location,
            "resource_kind":Value::Null,"media_type":Value::Null,
            "reference_count":references.count(ItemId::Scene(scene.id)),
        }));
    }
    for resource in document.resources.values() {
        push(json!({
            "kind":"resource","id":resource.id,"name":resource.label,"location":resource.location,
            "resource_kind":resource.kind,"media_type":Value::Null,
            "texture_value_type":resource.texture_value_type,
            "reference_count":references.count(ItemId::Resource(resource.id)),
        }));
    }
    for asset in document.assets.values() {
        push(json!({
            "kind":"asset","id":asset.id,"name":location_name(&asset.location),"location":asset.location,
            "resource_kind":Value::Null,"media_type":asset.media_type,
            "reference_count":references.count(ItemId::Asset(asset.id)),
        }));
    }
    for location in unregistered {
        push(json!({
            "kind":"unregistered","name":location_name(location),"location":location,
            "resource_kind":asset_kind(location),"media_type":Value::Null,"reference_count":0,
        }));
    }
    for preset in project::presets::BUILTIN_PRESETS {
        push(json!({
            "kind":"resource","id":preset.id.0,"name":preset.label,"location":"@builtin",
            "resource_kind":"preset","media_type":Value::Null,"reference_count":0,"builtin":true,
        }));
    }
    rows.sort_by(|a, b| (&a.0, &a.1, &a.2, &a.3).cmp(&(&b.0, &b.1, &b.2, &b.3)));
    let total = rows.len();
    if offset > total {
        return Err(invalid("Contents search offset exceeds match count"));
    }
    let entries = rows
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, _, _, _, row)| row)
        .collect::<Vec<_>>();
    let next = offset + entries.len();
    Ok(json!({
        "project":project_ref,"query":query,"entries":entries,"total":total,
        "next_offset":(next < total).then_some(next),
    }))
}

fn paged_folder_contents(
    folder: &str,
    entries: Vec<Value>,
    offset: usize,
    limit: usize,
) -> Result<Value, ErrorPayload> {
    let total = entries.len();
    if offset > total {
        return Err(invalid("contents offset exceeds folder entry count"));
    }
    let page = entries
        .into_iter()
        .skip(offset)
        .take(limit)
        .collect::<Vec<_>>();
    let next = offset + page.len();
    Ok(json!({
        "folder":folder,
        "entries":page,
        "total":total,
        "next_offset":(next < total).then_some(next),
    }))
}

fn valid_folder_path(path: &str) -> bool {
    path.is_empty()
        || (!path.starts_with('/')
            && !path.contains('\\')
            && !path.contains('\0')
            && path
                .split('/')
                .all(|part| !part.is_empty() && part != "." && part != ".."))
}

fn parent_location(location: &str) -> &str {
    location.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn location_name(location: &str) -> &str {
    location.rsplit('/').next().unwrap_or(location)
}

fn asset_kind(logical_path: &str) -> &'static str {
    match std::path::Path::new(logical_path)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.to_ascii_lowercase())
        .as_deref()
    {
        Some("ply" | "obj") => "mesh",
        Some(
            "png" | "jpg" | "jpeg" | "exr" | "hdr" | "tga" | "bmp" | "tif" | "tiff" | "ppm" | "pfm",
        ) => "texture",
        _ => "asset",
    }
}
fn node_kind(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::Scope { .. } => "scope",
        NodeKind::State { .. } => "state",
        NodeKind::Entity { .. } => "entity",
        NodeKind::Configuration { .. } => "configuration",
        NodeKind::Instance { .. } => "instance",
        NodeKind::Option { .. } => "option",
    }
}
/// `limit` values of a parameter from `offset`, in the preview's forms
/// (floats as FloatValue, integers as decimal strings), reading a data
/// asset's window straight from its file. Errs past the end.
fn parameter_window(
    values: &ParameterValues,
    offset: usize,
    limit: usize,
    asset_path: impl Fn(project::AssetId) -> Result<std::path::PathBuf, ErrorPayload>,
) -> Result<(u64, Value), ErrorPayload> {
    fn window<T: Clone>(all: &[T], offset: usize, limit: usize) -> Result<Vec<T>, ErrorPayload> {
        if offset > all.len() {
            return Err(invalid("value offset exceeds count"));
        }
        Ok(all[offset..all.len().min(offset + limit)].to_vec())
    }
    Ok(match values {
        ParameterValues::Floats(v) => (
            v.len() as u64,
            json!({"kind":"floats","value":window(v, offset, limit)?}),
        ),
        ParameterValues::Integers(v) => (
            v.len() as u64,
            json!({"kind":"integers","value":window(v, offset, limit)?.iter().map(ToString::to_string).collect::<Vec<_>>()}),
        ),
        ParameterValues::Booleans(v) => (
            v.len() as u64,
            json!({"kind":"booleans","value":window(v, offset, limit)?}),
        ),
        ParameterValues::Strings(v) | ParameterValues::BuiltinSpectra(v) => (
            v.len() as u64,
            json!({"kind":if matches!(values,ParameterValues::Strings(_)){"strings"}else{"builtin_spectra"},"value":window(v, offset, limit)?}),
        ),
        ParameterValues::Assets(v) => (
            v.len() as u64,
            json!({"kind":"assets","value":window(v, offset, limit)?}),
        ),
        ParameterValues::Resources { kind, ids } => (
            ids.len() as u64,
            json!({"kind":"resources","resource_kind":kind,"ids":window(ids, offset, limit)?}),
        ),
        ParameterValues::Array {
            asset,
            encoding,
            count,
        } => {
            use std::io::{Read, Seek, SeekFrom};
            if offset as u64 > *count {
                return Err(invalid("value offset exceeds count"));
            }
            let stride = match encoding {
                project::ArrayEncoding::F64Le | project::ArrayEncoding::I64Le => 8,
                project::ArrayEncoding::BoolU8 => 1,
            };
            let length = (*count - offset as u64).min(limit as u64) as usize;
            let mut bytes = vec![0; length * stride];
            let mut file = std::fs::File::open(asset_path(*asset)?)
                .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            file.seek(SeekFrom::Start(offset as u64 * stride as u64))
                .and_then(|_| file.read_exact(&mut bytes))
                .map_err(|error| ErrorPayload::new(ErrorCode::IoError, error.to_string()))?;
            let chunks = bytes.chunks_exact(stride);
            let values = match encoding {
                project::ArrayEncoding::F64Le => json!({"kind":"floats","value":chunks
                    .map(|chunk| {
                        let n = f64::from_le_bytes(chunk.try_into().unwrap());
                        if n.is_nan() {
                            project::FloatValue::Nan
                        } else if n == f64::INFINITY {
                            project::FloatValue::PositiveInfinity
                        } else if n == f64::NEG_INFINITY {
                            project::FloatValue::NegativeInfinity
                        } else {
                            project::FloatValue::Finite(n)
                        }
                    })
                    .collect::<Vec<_>>()}),
                project::ArrayEncoding::I64Le => json!({"kind":"integers","value":chunks
                    .map(|chunk| i64::from_le_bytes(chunk.try_into().unwrap()).to_string())
                    .collect::<Vec<_>>()}),
                project::ArrayEncoding::BoolU8 => json!({"kind":"booleans","value":chunks
                    .map(|chunk| chunk[0] != 0)
                    .collect::<Vec<_>>()}),
            };
            (*count, values)
        }
    })
}

fn parameter_preview(values: &ParameterValues) -> (u64, Value) {
    match values {
        ParameterValues::Floats(v) => (
            v.len() as u64,
            json!({"kind":"floats","value":v.iter().take(32).collect::<Vec<_>>()}),
        ),
        ParameterValues::Integers(v) => (
            v.len() as u64,
            json!({"kind":"integers","value":v.iter().take(32).map(ToString::to_string).collect::<Vec<_>>()}),
        ),
        ParameterValues::Booleans(v) => (
            v.len() as u64,
            json!({"kind":"booleans","value":v.iter().take(32).collect::<Vec<_>>()}),
        ),
        ParameterValues::Strings(v) | ParameterValues::BuiltinSpectra(v) => (
            v.len() as u64,
            json!({"kind":if matches!(values,ParameterValues::Strings(_)){"strings"}else{"builtin_spectra"},"value":v.iter().take(32).map(|s|shortened(s)).collect::<Vec<_>>()}),
        ),
        ParameterValues::Assets(v) => (
            v.len() as u64,
            json!({"kind":"assets","value":v.iter().take(32).collect::<Vec<_>>()}),
        ),
        ParameterValues::Resources { kind, ids } => (
            ids.len() as u64,
            json!({"kind":"resources","resource_kind":kind,"ids":ids.iter().take(32).collect::<Vec<_>>()}),
        ),
        ParameterValues::Array {
            asset,
            encoding,
            count,
        } => (
            *count,
            json!({"kind":"array","asset":asset,"encoding":encoding,"count":count.to_string()}),
        ),
    }
}

/// One parameter row of the schema table. Defaults render as
/// `{value_type, values}` ready for a form, or null when r4 derives them.
fn schema_field_values(values: &ParameterValues) -> Value {
    match values {
        ParameterValues::Floats(v) => json!(
            v.iter()
                .map(|value| match value {
                    project::FloatValue::Finite(number) => json!(number),
                    _ => Value::Null,
                })
                .collect::<Vec<_>>()
        ),
        ParameterValues::Integers(v) => json!(v),
        ParameterValues::Booleans(v) => json!(v),
        ParameterValues::Strings(v) | ParameterValues::BuiltinSpectra(v) => json!(v),
        // No default of the table lands here.
        _ => Value::Null,
    }
}

/// The wire view of one `ParameterSpec`.
fn schema_field(spec: &pbrt_io::parameter_schema::ParameterSpec) -> Value {
    let default_parameter = spec.default_parameter();
    let default = default_parameter.as_ref().map(|parameter| {
        json!({"value_type":parameter.value_type,"values":schema_field_values(&parameter.values)})
    });
    let (default_kind, default_depends) = match &spec.default {
        Some(pbrt_io::parameter_schema::DefaultValue::Illuminant) => ("illuminant", None),
        Some(pbrt_io::parameter_schema::DefaultValue::Depends {
            parameter,
            operation,
        }) => (
            "depends",
            Some(json!({"parameter":parameter,"operation":operation.as_str()})),
        ),
        Some(pbrt_io::parameter_schema::DefaultValue::PresetElse(_)) => ("medium_preset", None),
        Some(_) => ("constant", None),
        None => ("none", None),
    };
    json!({
        "name":spec.name,
        "aliases":spec.aliases,
        "value_types":spec.value_types,
        "default":default,
        "default_kind":default_kind,
        "default_depends":default_depends,
        "texture":spec.texture.map(|texture| json!({"kind":texture.as_str()})),
        "role":spec.role.as_str(),
        "required":spec.required,
        "group":spec.group.map(|group| group.as_str()),
        "choices":spec.choices,
        "suggestions":spec.suggestions,
        "when":spec.when.iter().map(|condition| json!({"parameter":condition.parameter,"values":condition.values})).collect::<Vec<_>>(),
    })
}

impl CoreState {
    /// `project.get_parameter_schema`: the static
    /// per-implementation parameter table. No project must be open — the
    /// table describes r4, not any document.
    pub(crate) fn handle_parameter_schema(&self, request: &Request) -> Result<Value, ErrorPayload> {
        let p: dto::ProjectParameterSchemaParams = params(request)?;
        if !pbrt_io::parameter_schema::is_directive(&p.directive) {
            return Err(invalid("unknown PBRT directive"));
        }
        // The area light's default name is the one legal empty string;
        // everything else must name an implementation.
        if p.implementation.len() > 128
            || (p.implementation.is_empty() && p.directive != "AreaLightSource")
        {
            return Err(invalid(
                "implementation must be 1 to 128 bytes (empty only for AreaLightSource)",
            ));
        }
        let texture_value_type = p.texture_value_type.map(|value| match value {
            dto::TextureValueType::Float => project::TextureValueType::Float,
            dto::TextureValueType::Spectrum => project::TextureValueType::Spectrum,
        });
        if texture_value_type.is_some() && p.directive != "Texture" {
            return Err(invalid("texture_value_type only applies to Texture"));
        }
        let table = pbrt_io::parameter_schema::schema(&p.directive);
        // Selector candidates are the table's rows, or every
        // implementation the export schema supports.
        let implementations = pbrt_io::implementation_change::candidates(&p.directive);
        let resolved = table
            .and_then(|table| pbrt_io::parameter_schema::implementation(table, &p.implementation));
        let (implementation, parameters) = match resolved {
            Some(found) => (
                found.name,
                found
                    .parameters(texture_value_type)
                    .map(|parameters| parameters.iter().map(schema_field).collect::<Vec<_>>()),
            ),
            // Implementations without a table stay legal to use.
            None => (p.implementation.as_str(), None),
        };
        Ok(json!({
            "directive":p.directive,
            "implementation":implementation,
            "texture_value_type":p.texture_value_type,
            "implementations":implementations,
            "parameters":parameters,
            "exclusive":resolved.map(|found| found.exclusive).unwrap_or(&[]),
        }))
    }
}

/// Each node's parent scope, for every scope in `scene`.
fn parent_links<'a>(
    nodes: impl IntoIterator<Item = &'a project::Node>,
) -> BTreeMap<project::NodeId, project::NodeId> {
    let mut parents = BTreeMap::new();
    for node in nodes {
        if let NodeKind::Scope { children, .. } = &node.data {
            for child in children {
                parents.insert(*child, node.id);
            }
        }
    }
    parents
}

/// `node`'s ancestors, nearest first.
fn ancestors(
    parents: &BTreeMap<project::NodeId, project::NodeId>,
    node: project::NodeId,
) -> Vec<uuid::Uuid> {
    let mut path = Vec::new();
    let mut ancestor = parents.get(&node).copied();
    while let Some(parent) = ancestor {
        path.push(parent.0);
        ancestor = parents.get(&parent).copied();
    }
    path
}

#[cfg(test)]
mod image_contents_tests {
    use super::*;

    #[test]
    fn homogeneous_medium_sigma_t_uses_explicit_coefficients_or_pbrt_defaults() {
        let resource = project::Resource {
            id: ResourceId::new(),
            label: "fog".into(),
            location: "Media/fog.medium".into(),
            edition: project::Revision::initial(),
            kind: project::ResourceKind::Medium,
            texture_value_type: None,
            implementation: "homogeneous".into(),
            parameters: vec![],
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        assert_eq!(homogeneous_sigma_t(&resource), Some(2.));
        let mut explicit = resource.clone();
        explicit.parameters = vec![
            project::Parameter::floats("sigma_a", [1., 1., 1.]),
            project::Parameter::floats("sigma_s", [0.5, 0.5, 0.5]),
        ];
        assert_eq!(homogeneous_sigma_t(&explicit), Some(1.5));
    }

    #[test]
    fn an_image_and_its_texture_are_separate_contents_entries() {
        let mut document = project::ProjectDocument::new_empty("Image");
        let asset = project::Asset {
            id: project::AssetId::new(),
            location: "Images/wood.png".into(),
            media_type: "image/png".into(),
            sha256: "a".repeat(64),
            size: 1,
            origin: None,
        };
        let resource = project::Resource {
            id: project::ResourceId::new(),
            label: "wood".into(),
            location: "Textures/wood.texture".into(),
            edition: project::Revision::initial(),
            kind: project::ResourceKind::Texture,
            texture_value_type: Some(project::TextureValueType::Spectrum),
            implementation: "imagemap".into(),
            parameters: vec![project::Parameter {
                name: "filename".into(),
                value_type: "string".into(),
                color_space: project::ColorSpaceBinding::Inherit,
                values: project::ParameterValues::Assets(vec![asset.id]),
            }],
            definition_scope: None,
            declared_in: None,
            origin: None,
        };
        let resource_id = resource.id;
        let asset_id = asset.id;
        document.assets.insert(asset.id, asset);
        document.resources.insert(resource.id, resource);
        document.folders.insert("Images".into());
        let references = document.reference_index();
        let result = list_folder_contents(&document, &references, &[], "Images", 0, 100).unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["kind"], "asset");
        assert_eq!(entries[0]["id"], asset_id.to_string());
        assert_eq!(entries[0]["reference_count"], 1);
        let result = list_folder_contents(&document, &references, &[], "Textures", 0, 100).unwrap();
        let entries = result["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0]["kind"], "resource");
        assert_eq!(entries[0]["id"], resource_id.to_string());
        assert_eq!(entries[0]["resource_kind"], "texture");
    }
}
