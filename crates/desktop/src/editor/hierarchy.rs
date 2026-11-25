//! Scene hierarchy presentation; synthetic folders never become scene entities.
use super::Session;
use crate::backend::Command;
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    assets::IconName,
    component::{
        Icon, Sizable,
        input::{Input, InputEvent, InputState},
        list::ListItem,
        menu::{PopupMenu, PopupMenuItem},
        scroll::ScrollableElement as _,
        tree::{TreeItem, TreeState, tree},
    },
    *,
};
use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

/// Presets an ObjectDefinition can hold: shapes and groups.
fn definition_presets(presets: &[super::menu::PresetItem]) -> Vec<super::menu::PresetItem> {
    presets
        .iter()
        .filter(|preset| matches!(preset.category.as_str(), "shape" | "group"))
        .cloned()
        .collect()
}

/// The right-click `Create ▸` submenu, laid out like the menu bar's Create
/// menu. The tree's menu builder has no `Context<PopupMenu>`, so the submenu
/// is its own entity; the parent menu wires it up when it renders.
fn create_submenu(
    menu: PopupMenu,
    presets: Vec<super::menu::PresetItem>,
    window: &mut Window,
    cx: &mut App,
    action: impl Fn(uuid::Uuid) -> Box<dyn Action> + 'static,
) -> PopupMenu {
    use super::create_menu::{CreateEntry, layout, with_breaks};
    if presets.is_empty() {
        return menu;
    }
    let action = std::rc::Rc::new(action);
    let submenu = PopupMenu::build(window, cx, move |mut create, window, cx| {
        for entry in layout(&presets) {
            match entry {
                CreateEntry::Item(preset) => {
                    create = create.menu(preset.label.clone(), action(preset.id));
                }
                CreateEntry::Submenu(title, items) => {
                    let items: Vec<_> = items.into_iter().cloned().collect();
                    let action = action.clone();
                    create = create.submenu(title, window, cx, move |mut rows, _, _| {
                        let items: Vec<_> = items.iter().collect();
                        for (separator, preset) in with_breaks(&items) {
                            if separator {
                                rows = rows.separator();
                            }
                            rows = rows.menu(preset.label.clone(), action(preset.id));
                        }
                        rows
                    });
                }
            }
        }
        create
    });
    menu.item(PopupMenuItem::submenu("Create", submenu))
}

/// How many label hits one search returns; the response reports the total.
const SEARCH_LIMIT: usize = 64;
/// Typing settles before a project-wide label scan is sent.
const SEARCH_DEBOUNCE: Duration = Duration::from_millis(150);

/// A label hit as the hierarchy needs it: the node id plus its ancestor path
/// (immediate parent first) for lazy expansion.
#[derive(Clone, Debug, PartialEq)]
struct SearchHit {
    node_id: String,
    label: String,
    kind: String,
    path: Vec<String>,
}

/// The next action for revealing a hit in the lazily loaded tree.
#[derive(Clone, Debug, PartialEq)]
enum RevealStep {
    Reveal,
    Expand(String),
    Wait,
}

/// Type the core response; malformed records are skipped rather than rendered.
fn search_hits(page: &serde_json::Value) -> Vec<SearchHit> {
    page["nodes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|node| {
            Some(SearchHit {
                node_id: node["node_id"].as_str()?.to_owned(),
                label: node["label"].as_str()?.to_owned(),
                kind: node["kind"].as_str().unwrap_or("node").to_owned(),
                path: node["path"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned))
                    .collect(),
            })
        })
        .collect()
}

fn hierarchy_rename_edit(
    node_id: &str,
    current_label: &str,
    label: &str,
) -> Option<serde_json::Value> {
    let details = serde_json::json!({
        "node": {
            "node_id": node_id,
            "label": current_label,
            "data": {"kind":"entity"}
        }
    });
    super::components::rename_edit(&details, label)
}

fn hierarchy_resource_rename_edit(
    resource_id: &str,
    current_label: &str,
    label: &str,
) -> Option<serde_json::Value> {
    let label = label.trim();
    (label != current_label && !label.is_empty()).then(
        || serde_json::json!({"kind":"rename_resource","resource_id":resource_id,"label":label}),
    )
}

fn definition_label_from_row(label: &str) -> String {
    label
        .split_once("  (")
        .map_or(label, |(name, _)| name)
        .to_owned()
}

fn hierarchy_selected_item(
    selected_node: Option<&str>,
    selected_content: Option<&serde_json::Value>,
    pending_reveal: Option<&str>,
) -> Option<String> {
    // Keep the target selected while a reveal is waiting for lazy pages or
    // the ProjectSelect response. In particular, the definition row's
    // Contents selection may still be present during that interval.
    if let Some(node_id) = pending_reveal {
        return Some(format!("p:{node_id}"));
    }
    // The tree selection is updated immediately, while the Inspector's
    // selected content can lag until the ProjectSelect response arrives.
    // Prefer that fresh node so a stale definition selection cannot pull the
    // tree back to its root during the response gap.
    if let Some(node_id) = selected_node {
        return Some(format!("p:{node_id}"));
    }
    if let Some(entry) = selected_content
        && entry["resource_kind"] == "object_definition"
        && let Some(resource_id) = entry["id"].as_str()
    {
        return Some(format!("d:{resource_id}"));
    }
    selected_node.map(|id| format!("p:{id}"))
}

/// `(loaded, child_count)` for a project-tree id, wherever it currently sits.
fn node_status(node: &serde_json::Value, id: &str) -> Option<(bool, u64)> {
    if node["id"].as_str() == Some(id) {
        return Some((
            node["loaded"] == true,
            node["child_count"].as_u64().unwrap_or(0),
        ));
    }
    ["children", "nodes"].into_iter().find_map(|key| {
        node[key]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|child| node_status(child, id))
    })
}

/// Walk ancestors from the root down: an unloaded ancestor must page its
/// children in before the next one can even exist in the tree.
fn next_reveal_step(
    tree: &serde_json::Value,
    hit: &SearchHit,
    requested: &HashSet<String>,
) -> RevealStep {
    for ancestor in hit.path.iter().rev() {
        let Some((loaded, child_count)) = node_status(tree, ancestor) else {
            return RevealStep::Wait;
        };
        if !loaded && child_count > 0 {
            if requested.contains(ancestor) {
                return RevealStep::Wait;
            }
            return RevealStep::Expand(ancestor.clone());
        }
    }
    if node_status(tree, &hit.node_id).is_some() {
        RevealStep::Reveal
    } else {
        RevealStep::Wait
    }
}

pub struct Hierarchy {
    session: Entity<Session>,
    tree: Entity<TreeState>,
    search: Entity<InputState>,
    project_key: String,
    project_selected: Option<String>,
    rename: Option<HierarchyRename>,
    items: HashMap<String, TreeItem>,
    definition_entries: HashMap<String, serde_json::Value>,
    /// Project node ids the user opened; drives lazy fetches and survives
    /// rebuilds, cleared when another project or activation becomes current.
    expansion: HashSet<String>,
    project_scope: String,
    search_generation: u64,
    hits: Vec<SearchHit>,
    search_total: u64,
    /// The session's results belong to the current search box text.
    search_ready: bool,
    /// A search for the current text has been sent but not answered yet.
    search_pending: bool,
    /// Project revision captured by the first search page; "load more" must
    /// stay on the same revision so pages cannot mix.
    search_project: serde_json::Value,
    /// Offset of an in-flight continuation request.
    more_offset: Option<u64>,
    pending_reveal: Option<SearchHit>,
    reveal_requests: HashSet<String>,
    /// Branches whose stale children are being refreshed (sent, not answered).
    refresh_requests: HashSet<String>,
    force_rebuild: bool,
    /// Icon per tree item id, from the scene-graph object kind.
    icons: std::rc::Rc<std::cell::RefCell<HashMap<String, TreeIcon>>>,
    /// The Scene root already opened for this activation.
    auto_expanded: Option<String>,
    /// Whether the left press in flight landed on a tree row. Rows
    /// mark it through their own mouse-down (the tree's rows never stop
    /// propagation), so the tree area can tell a row press — select,
    /// drag, expand — from one on the empty area, which retires
    /// the selection.
    row_press: std::rc::Rc<std::cell::Cell<bool>>,
}

#[derive(Clone)]
struct TreeIcon {
    icon: IconName,
    object: String,
    dimmed: bool,
}

struct HierarchyRename {
    target: RenameTarget,
    label: String,
    input: Entity<InputState>,
}

#[derive(Clone)]
enum RenameTarget {
    Node(String),
    Resource(String),
}

impl Hierarchy {
    pub(super) fn reveal_definition(&mut self, resource_id: &str, cx: &mut Context<Self>) {
        let tree_id = format!("d:{resource_id}");
        if !self.items.contains_key(&tree_id) {
            return;
        }
        self.expansion.insert(tree_id.clone());
        self.force_rebuild = true;
        self.sync(cx);
        if let Some(item) = self.items.get(&tree_id).cloned() {
            self.tree.update(cx, |tree, cx| {
                tree.set_selected_item(Some(&item), cx);
                tree.reveal_item(&item.id, ScrollStrategy::Center, cx);
            });
        }
        if let Some(scope_id) = self.definition_scope(resource_id, cx)
            && self.unloaded(&scope_id, cx)
        {
            let project = self.session.read(cx).project_value();
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::ProjectExpand {
                        project,
                        node_id: scope_id,
                        offset: 0,
                    },
                    cx,
                );
            });
        }
    }

    fn begin_rename(
        &mut self,
        target: RenameTarget,
        label: String,
        kind: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if (kind == "scene" && matches!(&target, RenameTarget::Node(_)))
            || self.session.read(cx).document().is_read_only()
        {
            return;
        }
        let input = cx.new(|cx| InputState::new(window, cx).default_value(&label));
        cx.subscribe(&input, |this, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::PressEnter { .. }) {
                this.finish_rename(cx);
            }
        })
        .detach();
        input.update(cx, |input, cx| input.focus(window, cx));
        // Typing replaces the old name, as in other inline renames.
        // After the focus lands, or the action finds no focused input.
        window.on_next_frame(|window, cx| {
            window.dispatch_action(Box::new(gpui_kit::component::input::SelectAll), cx);
        });
        self.rename = Some(HierarchyRename {
            target,
            label,
            input,
        });
        cx.notify();
    }

    fn finish_rename(&mut self, cx: &mut Context<Self>) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        let value = rename.input.read(cx).value().to_string();
        let edit = match &rename.target {
            RenameTarget::Node(node_id) => hierarchy_rename_edit(node_id, &rename.label, &value),
            RenameTarget::Resource(resource_id) => {
                hierarchy_resource_rename_edit(resource_id, &rename.label, &value)
            }
        };
        if let Some(edit) = edit {
            serde_json::from_value::<protocol::project::ProjectEdit>(edit.clone())
                .expect("Hierarchy rename edit must match ProjectEdit");
            let project = self.session.read(cx).project_value();
            self.session.update(cx, |session, cx| {
                session.send(
                    Command::Apply {
                        project,
                        edits: vec![edit],
                        follow: crate::backend::Follow::Nothing,
                        form: None,
                    },
                    cx,
                );
            });
        }
        cx.notify();
    }

    fn cancel_rename(&mut self, cx: &mut Context<Self>) {
        if self.rename.take().is_some() {
            cx.notify();
        }
    }

    pub fn new(session: Entity<Session>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let search = cx.new(|cx| InputState::new(window, cx).placeholder("Search nodes…"));
        cx.observe(&search, |_, _, cx| cx.notify()).detach();
        cx.subscribe(&search, |this, _, event: &InputEvent, cx| match event {
            InputEvent::Change => {
                let query = this.search.read(cx).value().trim().to_owned();
                if query.is_empty() {
                    this.clear_search(cx);
                } else {
                    this.schedule_search(cx);
                }
            }
            InputEvent::PressEnter { .. } => {
                // Enter cancels a pending debounce: search now, or reveal the
                // first hit when its results are already on screen.
                this.search_generation = this.search_generation.wrapping_add(1);
                let query = this.search.read(cx).value().trim().to_owned();
                if query.is_empty() {
                    this.clear_search(cx);
                } else if this.search_ready {
                    match this.hits.first().cloned() {
                        Some(hit) => this.activate_reveal(hit, cx),
                        None => this.send_search(query, 0, cx),
                    }
                } else {
                    this.send_search(query, 0, cx);
                }
            }
            InputEvent::Focus | InputEvent::Blur => {}
        })
        .detach();
        cx.observe(&session, |this, _, cx| this.sync(cx)).detach();
        cx.observe(&tree, |this, _, cx| {
            let id = this
                .tree
                .read(cx)
                .selected_item()
                .map(|item| item.id.to_string());
            if let Some(id) = id.filter(|id| this.items.contains_key(id)) {
                if let Some(resource_id) = id.strip_prefix("d:") {
                    // A definition row is selected in the Contents view; do
                    // not let the previously selected scene node win the
                    // hierarchy reconciliation below.
                    this.session
                        .update(cx, |session, _| session.selected_project_node = None);
                    if let Some(entry) = this.definition_entries.get(resource_id).cloned() {
                        let project = this.session.read(cx).project_value();
                        this.session.update(cx, |session, cx| {
                            session.select_contents(entry, project, cx);
                        });
                    }
                    return;
                }
                if let Some(node_id) = id.strip_prefix("p:") {
                    if this.session.read(cx).selected_project_node.as_deref() == Some(node_id) {
                        return;
                    }
                    let project = this.session.read(cx).project_value();
                    this.session.update(cx, |s, cx| {
                        s.selected_project_node = Some(node_id.to_owned());
                        s.send(
                            Command::ProjectSelect {
                                project,
                                node_id: node_id.to_owned(),
                            },
                            cx,
                        );
                    });
                }
            }
        })
        .detach();
        // Expansion drives the lazy fetch: unloaded project nodes page their
        // children in when the user opens them, "load more" rows page further.
        cx.subscribe(
            &tree,
            |this, _, event: &gpui_kit::base::TreeEvent, cx| match event {
                gpui_kit::base::TreeEvent::Expanded(id) => {
                    let id = id.to_string();
                    if let Some((parent_id, offset_text)) = id
                        .strip_prefix("p:more:")
                        .and_then(|rest| rest.split_once(':'))
                    {
                        let project = this.session.read(cx).project_value();
                        let offset = offset_text.parse::<u64>().unwrap_or_default();
                        this.session.update(cx, |s, cx| {
                            s.send(
                                Command::ProjectExpand {
                                    project,
                                    node_id: parent_id.to_owned(),
                                    offset,
                                },
                                cx,
                            )
                        });
                        return;
                    }
                    if !this.expansion.insert(id.clone()) {
                        return;
                    }
                    if let Some(resource_id) = id.strip_prefix("d:") {
                        let Some(scope_id) = this.definition_scope(resource_id, cx) else {
                            return;
                        };
                        if this.unloaded(&scope_id, cx) {
                            let project = this.session.read(cx).project_value();
                            this.session.update(cx, |s, cx| {
                                s.send(
                                    Command::ProjectExpand {
                                        project,
                                        node_id: scope_id,
                                        offset: 0,
                                    },
                                    cx,
                                )
                            });
                        }
                        return;
                    }
                    let Some(node_id) = id.strip_prefix("p:") else {
                        return;
                    };
                    // A retained branch from an older revision is loaded but
                    // stale; refresh it when it is opened again.
                    let stale =
                        crate::project_tree::is_stale(&this.session.read(cx).project_tree, node_id);
                    if stale || this.unloaded(node_id, cx) {
                        this.refresh_requests.insert(node_id.to_owned());
                        let project = this.session.read(cx).project_value();
                        this.session.update(cx, |s, cx| {
                            s.send(
                                Command::ProjectExpand {
                                    project,
                                    node_id: node_id.to_owned(),
                                    offset: 0,
                                },
                                cx,
                            )
                        });
                    }
                }
                gpui_kit::base::TreeEvent::Collapsed(id) => {
                    let id = id.to_string();
                    this.expansion.remove(&id);
                    if let Some(node_id) = crate::project_tree::refresh_node_for_row(
                        &this.session.read(cx).project_tree,
                        &id,
                    ) {
                        this.refresh_requests.remove(&node_id);
                    }
                }
            },
        )
        .detach();
        let mut result = Self {
            session,
            tree,
            search,
            project_key: String::new(),
            project_selected: None,
            rename: None,
            items: HashMap::new(),
            definition_entries: HashMap::new(),
            expansion: HashSet::new(),
            project_scope: String::new(),
            search_generation: 0,
            hits: Vec::new(),
            search_total: 0,
            search_ready: false,
            search_pending: false,
            search_project: serde_json::Value::Null,
            more_offset: None,
            pending_reveal: None,
            reveal_requests: HashSet::new(),
            refresh_requests: HashSet::new(),
            force_rebuild: false,
            icons: Default::default(),
            auto_expanded: None,
            row_press: Default::default(),
        };
        result.sync(cx);
        result
    }

    /// Debounce typing so one scan runs per settled query. The generation
    /// check drops timers that a newer keystroke superseded.
    fn schedule_search(&mut self, cx: &mut Context<Self>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.search_pending = true;
        let generation = self.search_generation;
        cx.spawn(async move |this, cx| {
            cx.background_executor().timer(SEARCH_DEBOUNCE).await;
            let _ = this.update(cx, |this, cx| {
                if this.search_generation != generation {
                    return;
                }
                let query = this.search.read(cx).value().trim().to_owned();
                if query.is_empty() {
                    this.clear_search(cx);
                } else {
                    this.send_search(query, 0, cx);
                }
            });
        })
        .detach();
    }

    fn send_search(&mut self, query: String, offset: usize, cx: &mut Context<Self>) {
        // Continuations pin the revision that produced page 0.
        let project = if offset == 0 {
            self.session.read(cx).project_value()
        } else if self.search_project.is_object() {
            self.search_project.clone()
        } else {
            return;
        };
        if !project.is_object() {
            return;
        }
        self.search_project = project.clone();
        if offset == 0 {
            self.search_pending = true;
            self.more_offset = None;
        } else {
            self.more_offset = Some(offset as u64);
        }
        self.session.update(cx, |session, cx| {
            session.send(
                Command::ProjectSearch {
                    project,
                    query,
                    offset,
                    limit: SEARCH_LIMIT,
                },
                cx,
            )
        });
    }

    /// Emptying the box hides results locally; the core rejects empty queries.
    fn clear_search(&mut self, cx: &mut Context<Self>) {
        self.search_generation = self.search_generation.wrapping_add(1);
        self.hits.clear();
        self.search_total = 0;
        self.search_ready = false;
        self.search_pending = false;
        self.search_project = serde_json::Value::Null;
        self.more_offset = None;
        self.pending_reveal = None;
        self.reveal_requests.clear();
        self.session.update(cx, |session, cx| {
            session.project_search_query.clear();
            session.project_search = serde_json::Value::Null;
            cx.notify();
        });
        cx.notify();
    }

    fn refresh_search(&mut self, cx: &Context<Self>) {
        let session = self.session.read(cx);
        let stored = session.project_search_query.trim().to_owned();
        let page = session.project_search.clone();
        let input = self.search.read(cx).value().trim().to_owned();
        // Results are only valid for the revision that produced them; an edit
        // or project switch hides them until the user searches again.
        let page_is_current = !page.is_object() || page["project"] == session.project_value();
        self.search_ready = !input.is_empty() && stored == input && page_is_current;
        if self.search_ready {
            self.search_pending = false;
            self.hits = search_hits(&page);
            self.search_total = page["total"].as_u64().unwrap_or(self.hits.len() as u64);
            // A continuation is answered once the page's next offset advances
            // Until then, keep the button in its loading state.
            if let Some(requested) = self.more_offset {
                if page["next_offset"].as_u64() != Some(requested) {
                    self.more_offset = None;
                }
            }
        } else {
            self.hits.clear();
            self.search_total = 0;
            self.more_offset = None;
        }
    }

    /// Append the next search page. The button is inert while one is in flight.
    fn load_more(&mut self, cx: &mut Context<Self>) {
        if self.more_offset.is_some() || !self.search_ready {
            return;
        }
        let next = self.session.read(cx).project_search["next_offset"].as_u64();
        let Some(next) = next else {
            return;
        };
        let query = self.search.read(cx).value().trim().to_owned();
        if query.is_empty() {
            return;
        }
        // An edit changed the project revision: restart from page 0 instead of
        // appending pages from two different trees.
        if self.session.read(cx).project_value() != self.search_project {
            self.send_search(query, 0, cx);
            return;
        }
        self.send_search(query, next as usize, cx);
    }

    /// Reveal a where-used row's node once its
    /// scene's tree is showing - `sync` picks up the Session's pending
    /// reveal, so a row that opened another scene waits for the switch.
    fn fire_where_used_reveal(&mut self, project_tree: &serde_json::Value, cx: &mut Context<Self>) {
        let Some((scene, node, path)) = self.session.read(cx).reveal_where_used.clone() else {
            return;
        };
        // The scene-graph tree's first root is the active Scene node, so
        // the pending fires only under the row's own scene.
        if project_tree["nodes"][0]["id"] != scene {
            return;
        }
        self.session
            .update(cx, |session, _| session.reveal_where_used = None);
        self.activate_reveal(
            SearchHit {
                node_id: node,
                label: String::new(),
                kind: String::new(),
                path,
            },
            cx,
        );
    }

    /// Expand the hit's ancestors and select it; `advance_reveal` then walks
    /// the lazy pages until the item exists.
    fn activate_reveal(&mut self, hit: SearchHit, cx: &mut Context<Self>) {
        let project = self.session.read(cx).project_value();
        if !project.is_object() {
            return;
        }
        for ancestor in &hit.path {
            let row = hierarchy_row_for_node(&self.session.read(cx).project_tree, ancestor)
                .unwrap_or_else(|| format!("p:{ancestor}"));
            self.expansion.insert(row);
        }
        self.force_rebuild = true;
        self.reveal_requests.clear();
        self.pending_reveal = Some(hit.clone());
        self.session.update(cx, |session, cx| {
            session.send(
                Command::ProjectSelect {
                    project,
                    node_id: hit.node_id,
                },
                cx,
            )
        });
        self.sync(cx);
    }

    /// One step per sync: request the next unloaded ancestor, or reveal once
    /// every ancestor page has arrived and the target item exists.
    fn advance_reveal(&mut self, cx: &mut Context<Self>) {
        let Some(hit) = self.pending_reveal.clone() else {
            return;
        };
        let tree = self.session.read(cx).project_tree.clone();
        let selected_node = self.session.read(cx).selected_project_node.as_deref();
        if selected_node == Some(hit.node_id.as_str())
            && self.items.contains_key(&format!("p:{}", hit.node_id))
        {
            // The target is visible and ProjectSelect has acknowledged it;
            // until both are true, stale Contents selection must not win.
            self.pending_reveal = None;
        }
        match next_reveal_step(&tree, &hit, &self.reveal_requests) {
            RevealStep::Wait => {}
            RevealStep::Expand(node_id) => {
                self.reveal_requests.insert(node_id.clone());
                let project = self.session.read(cx).project_value();
                if project.is_object() {
                    self.session.update(cx, |session, cx| {
                        session.send(
                            Command::ProjectExpand {
                                project,
                                node_id,
                                offset: 0,
                            },
                            cx,
                        )
                    });
                }
            }
            RevealStep::Reveal => {
                let Some(item) = self.items.get(&format!("p:{}", hit.node_id)).cloned() else {
                    return;
                };
                self.project_selected = Some(format!("p:{}", hit.node_id));
                self.tree.update(cx, |state, cx| {
                    state.set_selected_item(Some(&item), cx);
                    state.reveal_item(&item.id, ScrollStrategy::Center, cx);
                });
                if self.pending_reveal.is_none() {
                    self.reveal_requests.clear();
                }
            }
        }
    }

    /// Refresh one expanded branch retained from an older revision per sync.
    fn advance_refresh(&mut self, cx: &mut Context<Self>) {
        let tree = self.session.read(cx).project_tree.clone();
        let arrived = self.session.update(cx, |session, _| {
            std::mem::take(&mut session.children_arrived)
        });
        crate::project_tree::answer_refresh_requests(&arrived, &mut self.refresh_requests);
        crate::project_tree::prune_refresh_requests(&tree, &mut self.refresh_requests);
        let Some(node_id) =
            crate::project_tree::next_stale_refresh(&tree, &self.expansion, &self.refresh_requests)
        else {
            return;
        };
        let project = self.session.read(cx).project_value();
        if !project.is_object() {
            return;
        }
        self.refresh_requests.insert(node_id.clone());
        self.session.update(cx, |session, cx| {
            session.send(
                Command::ProjectExpand {
                    project,
                    node_id,
                    offset: 0,
                },
                cx,
            )
        });
    }

    /// A project node whose children have not been paged in yet.
    fn unloaded(&self, node_id: &str, cx: &Context<Self>) -> bool {
        let tree = self.session.read(cx).project_tree.clone();
        fn within(node: &serde_json::Value, id: &str) -> Option<bool> {
            if node["id"].as_str() == Some(id) {
                return Some(
                    node["loaded"] != true && node["child_count"].as_u64().unwrap_or(0) > 0,
                );
            }
            node["children"]
                .as_array()
                .into_iter()
                .flatten()
                .find_map(|child| within(child, id))
        }
        tree["nodes"]
            .as_array()
            .into_iter()
            .flatten()
            .find_map(|root| within(root, node_id))
            .unwrap_or(false)
    }

    fn definition_scope(&self, resource_id: &str, cx: &Context<Self>) -> Option<String> {
        fn find(nodes: &[serde_json::Value], resource_id: &str) -> Option<String> {
            for node in nodes {
                if node["resource_id"].as_str() == Some(resource_id) {
                    return node["id"].as_str().map(str::to_owned);
                }
                if let Some(found) = node["children"]
                    .as_array()
                    .and_then(|children| find(children, resource_id))
                {
                    return Some(found);
                }
            }
            None
        }
        find(
            self.session.read(cx).project_tree["nodes"].as_array()?,
            resource_id,
        )
    }

    fn sync(&mut self, cx: &mut Context<Self>) {
        let session = self.session.read(cx);
        if let Some(reference) = &session.project.context.project {
            let project_tree = session.project_tree.clone();
            let key = format!(
                "{}:{}:{project_tree}",
                reference.project_id,
                reference.activation.as_str()
            );
            // Expansion memory is per project activation; other projects'
            // node ids must not leak into this one.
            let scope = format!("{}:{}", reference.project_id, reference.activation.as_str());
            if self.project_scope != scope {
                self.project_scope = scope;
                self.expansion.retain(|id| !id.starts_with("p:"));
                self.pending_reveal = None;
                self.reveal_requests.clear();
                self.refresh_requests.clear();
                self.search_pending = false;
                self.search_project = serde_json::Value::Null;
                self.more_offset = None;
            }
            let selected_node = session.selected_project_node.clone();
            let selected_content = session.contents.selected().cloned();
            // The Scene root opens once per activation; its children arrive
            // with the roots.
            if let Some(root) = project_tree["nodes"][0]["id"].as_str() {
                if self.auto_expanded.as_deref() != Some(root) {
                    self.auto_expanded = Some(root.to_owned());
                    self.expansion.insert(format!("p:{root}"));
                    self.force_rebuild = true;
                }
            }
            let created = session.reveal_created.clone();
            let picked = session.reveal_picked.clone();
            let _ = session;
            if let Some((node, path)) = picked {
                // The pick already selected it; open its ancestors and
                // reveal it once their pages arrive.
                self.session
                    .update(cx, |session, _| session.reveal_picked = None);
                for ancestor in &path {
                    self.expansion.insert(format!("p:{ancestor}"));
                }
                self.force_rebuild = true;
                self.reveal_requests.clear();
                self.pending_reveal = Some(SearchHit {
                    node_id: node,
                    label: String::new(),
                    kind: String::new(),
                    path,
                });
                self.sync(cx);
                return;
            }
            if let Some((node, wrapper, parent)) = created {
                self.session
                    .update(cx, |session, _| session.reveal_created = None);
                let mut path = wrapper.into_iter().collect::<Vec<_>>();
                path.push(parent.clone());
                path.extend(super::create_menu::ancestors(&project_tree, &parent));
                self.activate_reveal(
                    SearchHit {
                        node_id: node,
                        label: String::new(),
                        kind: String::new(),
                        path,
                    },
                    cx,
                );
                return;
            }
            // A where-used row's pending reveal fires once the tree
            // is the row's scene tree.
            self.fire_where_used_reveal(&project_tree, cx);
            let forced = std::mem::take(&mut self.force_rebuild);
            let changed = self.project_key != key || forced;
            if changed {
                self.rename = None;
                self.project_key = key;
                let mut items = HashMap::new();
                let mut icons = HashMap::new();
                let roots = project_tree["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|root| project_items(root, &mut items, &mut icons, &self.expansion, false))
                    .collect::<Vec<_>>();
                self.definition_entries = project_tree["nodes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|root| root["kind"] == "definition_section")
                    .flat_map(|root| root["children"].as_array().into_iter().flatten())
                    .filter_map(|definition| {
                        Some((
                            definition["resource_id"].as_str()?.to_owned(),
                            definition["entry"].clone(),
                        ))
                    })
                    .collect();
                self.items = items;
                *self.icons.borrow_mut() = icons;
                self.tree.update(cx, |state, cx| state.set_items(roots, cx));
            }
            let pending_reveal = self
                .pending_reveal
                .as_ref()
                .filter(|hit| self.items.contains_key(&format!("p:{}", hit.node_id)))
                .map(|hit| hit.node_id.as_str());
            let selected = hierarchy_selected_item(
                selected_node.as_deref(),
                selected_content.as_ref(),
                pending_reveal,
            );
            let item = selected.as_ref().and_then(|id| self.items.get(id));
            if changed || self.project_selected != selected {
                self.project_selected = selected.clone();
                self.tree.update(cx, |state, cx| {
                    state.set_selected_item(item, cx);
                    if let Some(item) = item {
                        state.reveal_item(&item.id, ScrollStrategy::Center, cx);
                    }
                });
            }
            self.refresh_search(cx);
            self.advance_reveal(cx);
            self.advance_refresh(cx);
            cx.notify();
            return;
        }
        // No document: the Start screen replaces the Dock; drop the tree.
        if !self.project_key.is_empty() {
            self.rename = None;
            self.project_key.clear();
            self.project_selected = None;
            self.items.clear();
            self.tree
                .update(cx, |state, cx| state.set_items(Vec::new(), cx));
        }
        self.pending_reveal = None;
        self.reveal_requests.clear();
        self.hits.clear();
        self.search_total = 0;
        self.search_ready = false;
        self.search_pending = false;
        self.search_project = serde_json::Value::Null;
        self.more_offset = None;
        cx.notify();
    }
}

impl Render for Hierarchy {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let project_tree = self.session.read(cx).project_tree.clone();
        if self.session.read(cx).project.context.project.is_some()
            && self.session.read(cx).project.active_scene.is_none()
        {
            return div()
                .size_full()
                .p_3()
                .child("No scene is open")
                .into_any_element();
        }
        let query_empty = self.search.read(cx).value().trim().is_empty();
        let next_offset = self.session.read(cx).project_search["next_offset"].as_u64();
        let results = (!query_empty).then(|| {
            let header = if !self.search_ready {
                if self.search_pending {
                    "Searching…".to_owned()
                } else {
                    "Press Enter to search".to_owned()
                }
            } else if self.search_total == 0 {
                "No matches".to_owned()
            } else if self.search_total > self.hits.len() as u64 {
                format!("{} of {} hits", self.hits.len(), self.search_total)
            } else {
                format!("{} hits", self.search_total)
            };
            let mut column = div()
                .flex_shrink_0()
                .max_h(px(220.))
                .overflow_y_scrollbar()
                .border_b_1()
                .border_color(rgb(0x3a3d45))
                .px_2()
                .pb_1()
                .text_xs()
                .child(div().py_1().opacity(0.7).child(header));
            for hit in self.hits.clone() {
                let indent = 8. + hit.path.len() as f32 * 12.;
                let label = format!("{} · {}", hit.label, hit.kind);
                let id = format!("search:{}", hit.node_id);
                column =
                    column.child(
                        div()
                            .id(id)
                            .pl(px(indent))
                            .pr_1()
                            .py(px(2.))
                            .rounded(px(3.))
                            .cursor_pointer()
                            .hover(|style| style.bg(rgb(0x30333a)))
                            .child(label)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.activate_reveal(hit.clone(), cx)
                            })),
                    );
            }
            if self.search_ready && (next_offset.is_some() || self.more_offset.is_some()) {
                let label = if self.more_offset.is_some() {
                    "Loading…".to_owned()
                } else {
                    let remaining = self.search_total.saturating_sub(self.hits.len() as u64);
                    format!("Load more ({remaining} remaining)")
                };
                column = column.child(
                    div()
                        .id("search:more")
                        .pl(px(8.))
                        .py(px(3.))
                        .rounded(px(3.))
                        .cursor_pointer()
                        .opacity(if self.more_offset.is_some() { 0.5 } else { 1. })
                        .hover(|style| style.bg(rgb(0x30333a)))
                        .child(label)
                        .on_click(cx.listener(|this, _, _, cx| this.load_more(cx))),
                );
            }
            column
        });
        div()
            .key_context("HierarchyTree")
            .on_action(
                cx.listener(|this, _: &super::menu::RenameHierarchyNode, window, cx| {
                    let Some(item) = this.tree.read(cx).selected_item().cloned() else {
                        return;
                    };
                    let kind = this
                        .icons
                        .borrow()
                        .get(item.id.as_ref())
                        .map(|record| record.object.clone())
                        .unwrap_or_default();
                    let target = if let Some(resource_id) = item.id.strip_prefix("d:") {
                        RenameTarget::Resource(resource_id.to_owned())
                    } else if let Some(node_id) = item.id.strip_prefix("p:") {
                        RenameTarget::Node(node_id.to_owned())
                    } else {
                        return;
                    };
                    let label = if matches!(&target, RenameTarget::Resource(_)) {
                        definition_label_from_row(item.label.as_ref())
                    } else {
                        item.label.to_string()
                    };
                    this.begin_rename(target, label, kind, window, cx);
                }),
            )
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .flex_shrink_0()
                    .px_2()
                    .py_1()
                    .child(Input::new(&self.search).small().cleanable(true).w_full()),
            )
            .children(results)
            .child({
                let icons = self.icons.clone();
                let menu_icons = self.icons.clone();
                let session = self.session.clone();
                let menu_session = self.session.clone();
                let row_press = self.row_press.clone();
                let hierarchy = cx.entity();
                let rename_input = self
                    .rename
                    .as_ref()
                    .map(|rename| {
                        let key = match &rename.target {
                            RenameTarget::Node(id) => format!("p:{id}"),
                            RenameTarget::Resource(id) => format!("d:{id}"),
                        };
                        (key, rename.input.clone())
                    });
                let editable = {
                    let session = self.session.read(cx);
                    !session.document().is_read_only() && session.project.context.project.is_some()
                };
                let presets = {
                    let session = self.session.read(cx);
                    (!session.document().is_read_only()
                        && session.project.context.project.is_some())
                    .then(|| super::create_menu::presets(&session.project_presets))
                    .unwrap_or_default()
                };
                // A left press the rows do not take — the empty area
                // below and around them — retires the selection. Every
                // row marks the same flag first (its own mouse-down,
                // which never stops propagation), so a press on a row
                // reaches this handler marked and selects, drags or
                // expands as before.
                div()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|this, _, _, cx| {
                            if this.row_press.take() {
                                return;
                            }
                            this.session
                                .update(cx, |session, cx| session.deselect_node(cx));
                        }),
                    )
                    .flex_1()
                    .min_h_0()
                    .child(
                        tree(&self.tree, move |index, entry, _, window, _| {
                            let (icon, object, dimmed) = icons
                                .borrow()
                                .get(entry.item().id.as_ref())
                                .cloned()
                                .map_or((None, String::new(), false), |record| {
                                    (Some(record.icon), record.object.clone(), record.dimmed)
                                });
                            let tree_id = entry.item().id.to_string();
                            let node_id = tree_id.strip_prefix("p:").map(str::to_owned);
                            let label = entry.item().label.to_string();
                            let edit_target = if editable {
                                node_id.clone().map(RenameTarget::Node).or_else(|| {
                                    tree_id.strip_prefix("d:").map(|id| RenameTarget::Resource(id.into()))
                                })
                            } else {
                                None
                            };
                            let edit_key = edit_target.as_ref().map(|target| match target {
                                RenameTarget::Node(id) => format!("p:{id}"),
                                RenameTarget::Resource(id) => format!("d:{id}"),
                            });
                            let edit_label = if matches!(&edit_target, Some(RenameTarget::Resource(_))) {
                                definition_label_from_row(&label)
                            } else {
                                label.clone()
                            };
                            let edit_kind = object.clone();
                            let editing = rename_input
                                .as_ref()
                                .filter(|(id, _)| edit_key.as_ref() == Some(id))
                                .map(|(_, input)| input.clone());
                            let cancel_hierarchy = hierarchy.clone();
                            let folder = entry.is_folder();
                            let expanded = entry.is_expanded();
                            let disclosure = if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            };
                            let row_press = row_press.clone();
                            let row = ListItem::new(index)
                                .h(px(28.))
                                .w_full()
                                .pl(px(8. + entry.depth() as f32 * 16.))
                                .pr_2()
                                .on_mouse_down(MouseButton::Left, move |_, _, _| {
                                    row_press.set(true)
                                })
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap_1()
                                        .text_sm()
                                        .overflow_hidden()
                                        .child(div().w(px(16.)).flex_shrink_0().children(
                                            folder.then(|| Icon::new(disclosure).size(px(12.))),
                                        ))
                                        .child(
                                            Icon::new(icon.unwrap_or(if folder {
                                                if expanded {
                                                    IconName::FolderOpen
                                                } else {
                                                    IconName::Folder
                                                }
                                            } else {
                                                IconName::Box
                                            }))
                                            .size(px(14.))
                                            .when(dimmed, |icon| icon.text_color(rgb(0x6b717c))),
                                        )
                                        .child(if let Some(input) = editing {
                                            div()
                                                .flex_1()
                                                .min_w_0()
                                                .key_context("HierarchyRename")
                                                .on_action(window.listener_for(
                                                    &cancel_hierarchy,
                                                    |this, _: &gpui_kit::component::input::Escape, _, cx| {
                                                        this.cancel_rename(cx)
                                                    },
                                                ))
                                                .child(Input::new(&input).small().w_full())
                                                .into_any_element()
                                        } else {
                                            div()
                                                .flex_1()
                                                .text_color(if dimmed { rgb(0x6b717c) } else { rgb(0xd4d7dd) })
                                                .child(entry.item().label.clone())
                                                .into_any_element()
                                        }),
                                );
                            let row_hierarchy = hierarchy.clone();
                            let row = row.on_click(window.listener_for(
                                &row_hierarchy,
                                move |this, event: &ClickEvent, window, cx| {
                                    if event.click_count() < 2 {
                                        return;
                                    }
                                    let Some(target) = edit_target.clone() else {
                                        return;
                                    };
                                    let item_key = match &target {
                                        RenameTarget::Node(id) => format!("p:{id}"),
                                        RenameTarget::Resource(id) => format!("d:{id}"),
                                    };
                                    let selected = this.items.get(&item_key).cloned();
                                    this.tree.update(cx, |tree, cx| {
                                        tree.set_selected_item(selected.as_ref(), cx)
                                    });
                                    this.begin_rename(
                                        target,
                                        edit_label.clone(),
                                        edit_kind.clone(),
                                        window,
                                        cx,
                                    );
                                },
                            ));
                            let item = row;
                            let source_id = node_id.filter(|_| editable);
                            // Objects drag onto groups and the Scene root (the
                            // virtual parent of World content) to move there.
                            let item = if let Some(node_id) = source_id.as_ref()
                                && super::node_edits::movable(&object)
                            {
                                let label = entry.item().label.to_string();
                                item.on_drag(
                                    HierarchyDrag {
                                        node: node_id.clone(),
                                    },
                                    move |_, _, _, cx| cx.new(|_| DragRow(label.clone())),
                                )
                            } else {
                                item
                            };
                            let tree = project_tree.clone();
                            let parent = source_id
                                .filter(|_| super::node_edits::accepts_children(&object))
                                .or_else(|| {
                                    (object == "definition_root")
                                        .then(|| tree_id.strip_prefix("d:"))
                                        .flatten()
                                        .and_then(|resource| definition_scope_from_tree(&tree, resource))
                                });
                            let Some(parent) = parent else {
                                return item;
                            };
                            let session = session.clone();
                            let target_object = object.clone();
                            let is_definition_root = target_object == "definition_root";
                            let accepts_definition_drop = tree.clone();
                            let definition_parent = parent.clone();
                            item.drag_over::<HierarchyDrag>(move |style, drag, _, _| {
                                if !is_definition_root
                                    || definition_root_drop_allowed(
                                        &accepts_definition_drop,
                                        &drag.node,
                                        &definition_parent,
                                    )
                                {
                                    style.bg(rgb(0x2f4a3a))
                                } else {
                                    style
                                }
                            })
                            .on_drop(
                                move |drag: &HierarchyDrag, _, cx| {
                                    if target_object == "definition_root"
                                        && !definition_root_drop_allowed(&tree, &drag.node, &parent)
                                    {
                                        return;
                                    }
                                    if let Some(edit) =
                                        super::node_edits::move_edit(&drag.node, &parent)
                                    {
                                        session.update(cx, |session, cx| {
                                            let project = session.project_value();
                                            session.send(
                                                Command::Apply {
                                                    project,
                                                    edits: vec![edit],
                                                    follow: crate::backend::Follow::Nothing,
                                                    form: None,
                                                },
                                                cx,
                                            );
                                        });
                                    }
                                },
                            )
                        })
                        .context_menu(move |_, entry, menu, window, cx| {
                            // Right-click selects the row, so Create places the
                            // new node relative to it.
                            let item = entry.item().clone();
                            if item.id.starts_with("p:more:") {
                                return menu;
                            }
                            let item_id = item.id.to_string();
                            let state = cx.entity();
                            let selected_item = item.clone();
                            cx.defer(move |cx| {
                                state.update(cx, |state, cx| {
                                    state.set_selected_item(Some(&selected_item), cx)
                                })
                            });
                            let mut menu = menu;
                            let object = menu_icons
                                .borrow()
                                .get(item_id.as_str())
                                .map(|record| record.object.clone())
                                .unwrap_or_default();
                            if let Some(resource_id) = item_id.strip_prefix("d:") {
                                if !editable {
                                    return menu;
                                }
                                let selected_entry = definition_entry(
                                    &menu_session.read(cx).project_tree,
                                    resource_id,
                                );
                                let scope = definition_scope_from_tree(
                                    &menu_session.read(cx).project_tree,
                                    resource_id,
                                );
                                menu = menu
                                    .menu("Rename", Box::new(super::menu::ContentsRename))
                                    .menu("Delete…", Box::new(super::menu::ContentsDelete));
                                if let Some(scope) = scope.and_then(|id| uuid::Uuid::parse_str(&id).ok()) {
                                    menu = create_submenu(menu, definition_presets(&presets), window, cx, move |preset| {
                                        Box::new(super::menu::CreateDefinitionPreset { preset, parent: scope, after: None })
                                    });
                                }
                                if let Some(entry) = selected_entry {
                                    let session = menu_session.clone();
                                    cx.defer(move |cx| {
                                        session.update(cx, |session, _| session.contents.select_entry(&entry));
                                    });
                                }
                                return menu;
                            }
                            if !item.id.starts_with("p:") {
                                return menu;
                            }
                            let in_definition = is_definition_descendant(
                                &menu_session.read(cx).project_tree,
                                item_id.strip_prefix("p:").unwrap_or_default(),
                            );
                            if in_definition {
                                if !editable {
                                    return menu;
                                }
                                menu = menu.menu("Rename", Box::new(super::menu::RenameHierarchyNode));
                                if super::node_edits::duplicate_delete(&object) {
                                    menu = menu
                                        .menu("Duplicate", Box::new(super::menu::DuplicateNode))
                                        .menu(
                                            "Delete…",
                                            Box::new(super::menu::ConfirmDeleteHierarchyNode),
                                        );
                                }
                                if matches!(object.as_str(), "group" | "shape") {
                                    let tree = &menu_session.read(cx).project_tree;
                                    let target = definition_create_target(
                                        tree,
                                        item_id.strip_prefix("p:").unwrap_or_default(),
                                        &object,
                                    );
                                    if let Some((parent, after)) = target
                                        && let Ok(parent) = uuid::Uuid::parse_str(&parent)
                                    {
                                        let after = after.and_then(|id| uuid::Uuid::parse_str(&id).ok());
                                        menu = create_submenu(menu, definition_presets(&presets), window, cx, move |preset| {
                                            Box::new(super::menu::CreateDefinitionPreset { preset, parent, after })
                                        });
                                    }
                                }
                                return menu;
                            }
                            menu = menu.menu("Rename", Box::new(super::menu::RenameHierarchyNode));
                            if !presets.is_empty() && super::node_edits::duplicate_delete(&object) {
                                menu = menu
                                    .menu("Duplicate", Box::new(super::menu::DuplicateNode))
                                    .menu("Delete", Box::new(super::menu::DeleteNode))
                                    .separator();
                            }
                            if object == "group" {
                                menu = menu.menu("Make Object Definition", Box::new(super::menu::MakeObjectDefinition)).separator();
                            }
                            if object == "instance" {
                                menu = menu.menu("Unpack Instance", Box::new(super::menu::UnpackObjectInstance)).separator();
                            }
                            create_submenu(menu, presets.clone(), window, cx, |preset| {
                                Box::new(super::menu::CreatePreset { preset })
                            })
                        })
                        .size_full(),
                    )
            })
            .into_any_element()
    }
}

/// What a Hierarchy row carries while it is dragged.
struct HierarchyDrag {
    node: String,
}

/// The label that follows the pointer during a drag.
struct DragRow(String);

impl Render for DragRow {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .px_2()
            .py_1()
            .rounded_sm()
            .bg(rgb(0x3a3d45))
            .text_sm()
            .text_color(rgb(0xd4d7dd))
            .child(self.0.clone())
    }
}

/// Build tree items for the lazily loaded project hierarchy. Unloaded scopes
/// carry one disabled placeholder child so they still render as folders; a
/// partial branch ends in a "load more" row that pages the next slice.
/// Reused items keep their expansion state across rebuilds.
fn project_items(
    node: &serde_json::Value,
    items: &mut HashMap<String, TreeItem>,
    icons: &mut HashMap<String, TreeIcon>,
    expansion: &HashSet<String>,
    ancestor_inactive: bool,
) -> TreeItem {
    if node["kind"] == "definition_section" {
        let id = "s:@object-definitions";
        let mut item = TreeItem::new(id, "Object Definitions".to_owned());
        if expansion.contains(id) {
            item = item.expanded(true);
        }
        item.children = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|definition| {
                definition_item(definition, items, icons, expansion, ancestor_inactive)
            })
            .collect();
        items.insert(id.to_owned(), TreeItem::new(id, "Object Definitions"));
        return item;
    }
    if node["kind"] == "definition_root" {
        return definition_item(node, items, icons, expansion, ancestor_inactive);
    }
    let id = node["id"].as_str().unwrap_or_default();
    let tree_id = format!("p:{id}");
    let label = object_label(node);
    let inactive = row_is_dimmed(node["active"] != false, ancestor_inactive);
    icons.insert(
        tree_id.clone(),
        TreeIcon {
            icon: object_icon(node),
            object: node["object"].as_str().unwrap_or_default().to_owned(),
            dimmed: inactive,
        },
    );
    let mut item = TreeItem::new(tree_id.clone(), label.clone());
    if expansion.contains(&tree_id) {
        item = item.expanded(true);
    }
    let children: Vec<TreeItem> = if node["loaded"] == true {
        let mut loaded: Vec<_> = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| project_items(child, items, icons, expansion, inactive))
            .collect();
        if let Some(more) = node["more_offset"].as_u64() {
            let more_row = load_more_row(id, more);
            if inactive {
                mark_tree_subtree_dimmed(&more_row, icons);
            }
            loaded.push(more_row);
        }
        loaded
    } else if node["child_count"].as_u64().unwrap_or(0) > 0 {
        let pending = TreeItem::new(format!("p:pending:{id}"), "…".to_owned()).disabled(true);
        if inactive {
            icons.insert(
                pending.id.to_string(),
                TreeIcon {
                    icon: IconName::Ellipsis,
                    object: String::new(),
                    dimmed: true,
                },
            );
        }
        vec![pending]
    } else {
        Vec::new()
    };
    item.children = children;
    // Store leaf-like handles: cloned TreeItems otherwise copy all descendants.
    items.insert(tree_id.clone(), TreeItem::new(tree_id, label));
    item
}

fn definition_item(
    node: &serde_json::Value,
    items: &mut HashMap<String, TreeItem>,
    icons: &mut HashMap<String, TreeIcon>,
    expansion: &HashSet<String>,
    ancestor_inactive: bool,
) -> TreeItem {
    let resource_id = node["resource_id"].as_str().unwrap_or_default();
    let scope_id = node["id"].as_str().unwrap_or_default();
    let tree_id = format!("d:{resource_id}");
    let count = node["instance_count"].as_u64().unwrap_or(0);
    let label = format!(
        "{}  ({count} instance{})",
        node["label"].as_str().unwrap_or("Object"),
        if count == 1 { "" } else { "s" }
    );
    let inactive = row_is_dimmed(node["active"] != false, ancestor_inactive);
    icons.insert(
        tree_id.clone(),
        TreeIcon {
            icon: IconName::Boxes,
            object: "definition_root".into(),
            dimmed: inactive,
        },
    );
    let mut item = TreeItem::new(tree_id.clone(), label.clone());
    if expansion.contains(&tree_id) {
        item = item.expanded(true);
    }
    item.children = if node["loaded"] == true {
        let mut loaded: Vec<_> = node["children"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|child| project_items(child, items, icons, expansion, inactive))
            .collect();
        if let Some(more) = node["more_offset"].as_u64() {
            let more_row = load_more_row(scope_id, more);
            if inactive {
                mark_tree_subtree_dimmed(&more_row, icons);
            }
            loaded.push(more_row);
        }
        loaded
    } else if node["child_count"].as_u64().unwrap_or(0) > 0 {
        let pending = TreeItem::new(format!("p:pending:{scope_id}"), "…".to_owned()).disabled(true);
        if inactive {
            icons.insert(
                pending.id.to_string(),
                TreeIcon {
                    icon: IconName::Ellipsis,
                    object: String::new(),
                    dimmed: true,
                },
            );
        }
        vec![pending]
    } else {
        Vec::new()
    };
    items.insert(tree_id.clone(), TreeItem::new(tree_id, label));
    item
}

fn definition_entry(tree: &serde_json::Value, resource_id: &str) -> Option<serde_json::Value> {
    fn walk(nodes: &[serde_json::Value], resource_id: &str) -> Option<serde_json::Value> {
        for node in nodes {
            if node["resource_id"].as_str() == Some(resource_id) {
                return Some(node["entry"].clone());
            }
            if let Some(found) = node["children"]
                .as_array()
                .and_then(|children| walk(children, resource_id))
            {
                return Some(found);
            }
        }
        None
    }
    walk(tree["nodes"].as_array()?, resource_id)
}

fn definition_scope_from_tree(tree: &serde_json::Value, resource_id: &str) -> Option<String> {
    fn walk(nodes: &[serde_json::Value], resource_id: &str) -> Option<String> {
        for node in nodes {
            if node["resource_id"].as_str() == Some(resource_id) {
                return node["id"].as_str().map(str::to_owned);
            }
            if let Some(found) = node["children"]
                .as_array()
                .and_then(|children| walk(children, resource_id))
            {
                return Some(found);
            }
        }
        None
    }
    walk(tree["nodes"].as_array()?, resource_id)
}

fn definition_scope_for_node(tree: &serde_json::Value, node_id: &str) -> Option<String> {
    fn walk(nodes: &[serde_json::Value], node_id: &str) -> Option<String> {
        for node in nodes {
            if node["kind"] == "definition_root"
                && (node["id"].as_str() == Some(node_id)
                    || contains_node(&node["children"], node_id))
            {
                return node["id"].as_str().map(str::to_owned);
            }
            if let Some(found) = node["children"]
                .as_array()
                .and_then(|children| walk(children, node_id))
            {
                return Some(found);
            }
        }
        None
    }
    walk(tree["nodes"].as_array()?, node_id)
}

/// Map a backend node id in a reveal path to the id used by its Hierarchy
/// row. Definition roots are selected and expanded through their resource id.
fn hierarchy_row_for_node(tree: &serde_json::Value, node_id: &str) -> Option<String> {
    fn walk(nodes: &[serde_json::Value], node_id: &str) -> Option<String> {
        for node in nodes {
            if node["id"].as_str() == Some(node_id) {
                if node["kind"] == "definition_root" {
                    return node["resource_id"]
                        .as_str()
                        .map(|resource| format!("d:{resource}"));
                }
                return Some(format!("p:{node_id}"));
            }
            if let Some(found) = walk(
                node["children"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or(&[]),
                node_id,
            ) {
                return Some(found);
            }
        }
        None
    }
    walk(tree["nodes"].as_array()?, node_id)
}

fn definition_create_target(
    tree: &serde_json::Value,
    node_id: &str,
    object: &str,
) -> Option<(String, Option<String>)> {
    definition_scope_for_node(tree, node_id)?;
    if object == "group" {
        return Some((node_id.to_owned(), None));
    }
    if object != "shape" {
        return None;
    }
    fn parent_of(nodes: &[serde_json::Value], node_id: &str) -> Option<String> {
        for node in nodes {
            if node["children"]
                .as_array()?
                .iter()
                .any(|child| child["id"] == node_id)
            {
                return node["id"].as_str().map(str::to_owned);
            }
            if let Some(parent) = parent_of(node["children"].as_array()?, node_id) {
                return Some(parent);
            }
        }
        None
    }
    let parent = parent_of(tree["nodes"].as_array()?, node_id)?;
    Some((parent, Some(node_id.to_owned())))
}

fn contains_node(nodes: &serde_json::Value, node_id: &str) -> bool {
    nodes.as_array().into_iter().flatten().any(|node| {
        node["id"].as_str() == Some(node_id) || contains_node(&node["children"], node_id)
    })
}

fn is_definition_descendant(tree: &serde_json::Value, node_id: &str) -> bool {
    definition_scope_for_node(tree, node_id).is_some()
}

fn definition_root_drop_allowed(
    tree: &serde_json::Value,
    dragged_node: &str,
    target_scope: &str,
) -> bool {
    definition_scope_for_node(tree, dragged_node).as_deref() == Some(target_scope)
}

/// The row text: the label, plus the area-light / medium marks a shape carries.
fn object_label(node: &serde_json::Value) -> String {
    let mut label = node["label"].as_str().unwrap_or("Node").to_owned();
    if label.is_empty() {
        label = match node["object"].as_str() {
            Some("group") => "Attribute".into(),
            Some(kind) => kind.to_owned(),
            None => "Node".into(),
        };
    }
    // A shape shows its area light by its icon instead.
    if node["object"] != "shape" && has_component(node, "area_light") {
        label.push_str("  ✦");
    }
    label
}

/// Tree rows list component kinds; `get_node` lists component objects.
fn has_component(node: &serde_json::Value, kind: &str) -> bool {
    node["components"]
        .as_array()
        .is_some_and(|all| all.iter().any(|c| c == kind || c["kind"] == kind))
}

/// Inspector header icon from `get_node`'s data kind and entity kind.
pub(crate) fn object_icon_for(
    data_kind: &str,
    kind: &str,
    components: &serde_json::Value,
) -> IconName {
    let object = match (data_kind, kind) {
        ("scene", _) => "scene",
        ("scope", _) => "group",
        ("instance", _) => "instance",
        (_, "shape" | "light" | "camera") => kind,
        _ => "",
    };
    object_icon(&serde_json::json!({ "object": object, "components": components }))
}

/// Hierarchy icon per scene-graph object kind.
fn object_icon(node: &serde_json::Value) -> IconName {
    match node["object"].as_str() {
        Some("scene") => IconName::Clapperboard,
        Some("world") => IconName::Globe,
        Some("group") => IconName::Folder,
        Some("camera") => IconName::Camera,
        Some("light") => IconName::Lightbulb,
        Some("instance") => IconName::Copy,
        Some("shape") if has_component(node, "area_light") => IconName::LampCeiling,
        Some("shape") => IconName::Triangle,
        _ => IconName::Box,
    }
}

/// A notification row that pages the next slice of a large branch. Expanded
/// state is the click target; carrying a placeholder child keeps it a folder.
fn load_more_row(parent: &str, offset: u64) -> TreeItem {
    TreeItem::new(
        format!("p:more:{parent}:{offset}"),
        "… load more".to_owned(),
    )
    .child(TreeItem::new(format!("p:pending:{parent}:{offset}"), "…".to_owned()).disabled(true))
}

fn mark_tree_subtree_dimmed(item: &TreeItem, icons: &mut HashMap<String, TreeIcon>) {
    icons.insert(
        item.id.to_string(),
        TreeIcon {
            icon: IconName::Ellipsis,
            object: String::new(),
            dimmed: true,
        },
    );
    for child in &item.children {
        mark_tree_subtree_dimmed(child, icons);
    }
}

fn row_is_dimmed(active: bool, ancestor_inactive: bool) -> bool {
    !active || ancestor_inactive
}

#[cfg(test)]
mod lazy_tests {
    use super::{
        definition_create_target, definition_entry, definition_root_drop_allowed,
        definition_scope_for_node, hierarchy_row_for_node, hierarchy_selected_item,
        is_definition_descendant, project_items, row_is_dimmed,
    };
    use std::collections::{HashMap, HashSet};

    #[test]
    fn inactive_ancestor_dims_descendants_and_active_branches_do_not() {
        assert!(row_is_dimmed(false, false));
        assert!(row_is_dimmed(true, true));
        assert!(!row_is_dimmed(true, false));
    }

    fn loaded_node(
        id: &str,
        children: serde_json::Value,
        more_offset: serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({"id":id,"label":id,"kind":"scope","child_count":1,
            "loaded":true,"children":children,"more_offset":more_offset})
    }

    #[test]
    fn test_unloaded_scopes_carry_a_placeholder_and_more_rows_page_slices() {
        let unloaded = serde_json::json!({"id":"world","label":"World","kind":"scope","child_count":7,
            "loaded":false,"children":[],"more_offset":null});
        let mut items = HashMap::new();
        let item = project_items(
            &unloaded,
            &mut items,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        assert_eq!(item.children.len(), 1);
        assert_eq!(item.children[0].id.as_str(), "p:pending:world");
        assert!(item.children[0].is_disabled());
        assert!(
            !items.contains_key("p:pending:world"),
            "placeholder rows are never selectable"
        );

        // A partially loaded branch ends in a paging row for the next offset.
        let partial = loaded_node(
            "world",
            serde_json::json!([{"id":"a","label":"A","kind":"entity","child_count":0,
                "loaded":true,"children":[],"more_offset":null}]),
            serde_json::json!(1000),
        );
        let mut items = HashMap::new();
        let item = project_items(
            &partial,
            &mut items,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        let more = item.children.last().unwrap();
        assert_eq!(more.id.as_str(), "p:more:world:1000");
        assert_eq!(
            more.children.len(),
            1,
            "the paging row needs a folder child"
        );
        assert!(
            !items.contains_key(more.id.as_str()),
            "paging rows are never selectable"
        );

        // Completed branches drop both placeholders and paging rows.
        let done = loaded_node("world", serde_json::json!([]), serde_json::Value::Null);
        let mut items = HashMap::new();
        let item = project_items(
            &done,
            &mut items,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        assert!(item.children.is_empty());
    }

    #[test]
    fn test_rebuilt_items_keep_expansion_state() {
        let node = loaded_node("world", serde_json::json!([]), serde_json::Value::Null);
        // Reopened nodes: remembered expansion must survive lazy rebuilds.
        let expanded: HashSet<String> = ["p:world".to_owned()].into_iter().collect();
        let mut rebuilt = HashMap::new();
        let item = project_items(&node, &mut rebuilt, &mut HashMap::new(), &expanded, false);
        assert!(item.is_expanded());
        // Collapsed nodes rebuild collapsed.
        let mut collapsed = HashMap::new();
        let item = project_items(
            &node,
            &mut collapsed,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        assert!(!item.is_expanded());
    }

    #[test]
    fn definition_section_builds_resource_rows_and_tracks_inner_node_scope() {
        let tree = serde_json::json!({"nodes":[{"id":"@object-definitions","kind":"definition_section",
            "children":[{"id":"scope","resource_id":"resource","label":"Tree",
                "kind":"definition_root","object":"definition_root","instance_count":2,
                "entry":{"kind":"resource","resource_kind":"object_definition","id":"resource"},
                "loaded":true,"child_count":1,"children":[{"id":"shape","object":"shape",
                    "kind":"entity","label":"Sphere","loaded":true,"child_count":0,"children":[]}]}]}]});
        let mut items = HashMap::new();
        let item = project_items(
            &tree["nodes"][0],
            &mut items,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        assert_eq!(item.label.as_ref(), "Object Definitions");
        assert_eq!(item.children[0].id.as_str(), "d:resource");
        assert!(item.children[0].label.contains("2 instances"));
        assert!(is_definition_descendant(&tree, "shape"));
        assert_eq!(
            definition_scope_for_node(&tree, "shape").as_deref(),
            Some("scope")
        );
        assert_eq!(
            hierarchy_row_for_node(&tree, "scope").as_deref(),
            Some("d:resource")
        );
        assert_eq!(
            hierarchy_row_for_node(&tree, "shape").as_deref(),
            Some("p:shape")
        );
        assert_eq!(
            definition_entry(&tree, "resource").unwrap()["id"],
            "resource"
        );
        assert!(!item.is_expanded());
        let open: HashSet<String> = ["s:@object-definitions".to_owned()].into_iter().collect();
        let item = project_items(
            &tree["nodes"][0],
            &mut items,
            &mut HashMap::new(),
            &open,
            false,
        );
        assert!(item.is_expanded(), "the section stays open across rebuilds");
    }

    #[test]
    fn selected_definition_contents_entry_selects_its_hierarchy_row() {
        let entry = serde_json::json!({
            "kind":"resource",
            "resource_kind":"object_definition",
            "id":"definition-id"
        });
        assert_eq!(
            hierarchy_selected_item(None, Some(&entry), None).as_deref(),
            Some("d:definition-id")
        );
        assert_eq!(
            hierarchy_selected_item(Some("page-two-node"), Some(&entry), None).as_deref(),
            Some("p:page-two-node"),
            "a newly clicked node wins while the Inspector still shows its definition"
        );
        assert_eq!(
            hierarchy_selected_item(Some("scene-node"), None, None).as_deref(),
            Some("p:scene-node")
        );
        assert_eq!(
            hierarchy_selected_item(None, Some(&entry), Some("created-in-definition")).as_deref(),
            Some("p:created-in-definition"),
            "the pending node reveal wins while Contents still selects its definition"
        );
    }

    #[test]
    fn definition_root_accepts_only_its_own_definition_nodes() {
        let tree = serde_json::json!({"nodes":[
            {"id":"@object-definitions","kind":"definition_section","children":[
                {"id":"scope-a","resource_id":"resource-a","kind":"definition_root","children":[
                    {"id":"shape-a","kind":"entity","children":[]}]},
                {"id":"scope-b","resource_id":"resource-b","kind":"definition_root","children":[
                    {"id":"shape-b","kind":"entity","children":[]}]}
            ]},
            {"id":"scene","kind":"scope","children":[{"id":"shape-scene","kind":"entity","children":[]}]}
        ]});
        assert!(definition_root_drop_allowed(&tree, "shape-a", "scope-a"));
        assert!(!definition_root_drop_allowed(&tree, "shape-b", "scope-a"));
        assert!(!definition_root_drop_allowed(
            &tree,
            "shape-scene",
            "scope-a"
        ));
    }

    #[test]
    fn definition_create_uses_selected_group_or_shape_sibling_position() {
        let tree = serde_json::json!({"nodes":[{"id":"@object-definitions","kind":"definition_section","children":[
            {"id":"scope","kind":"definition_root","children":[
                {"id":"group","object":"group","children":[
                    {"id":"shape","object":"shape","children":[]},
                    {"id":"other","object":"shape","children":[]}]}]}
        ]}]});
        assert_eq!(
            definition_create_target(&tree, "group", "group"),
            Some(("group".into(), None))
        );
        assert_eq!(
            definition_create_target(&tree, "shape", "shape"),
            Some(("group".into(), Some("shape".into())))
        );
        assert_eq!(definition_create_target(&tree, "shape", "light"), None);
    }

    #[test]
    fn definition_root_appends_a_load_more_row_for_partial_pages() {
        let definition = serde_json::json!({"id":"scope","resource_id":"resource",
            "kind":"definition_root","loaded":true,"child_count":2,"more_offset":800,
            "children":[{"id":"shape","object":"shape","kind":"entity","label":"Sphere",
                "loaded":true,"child_count":0,"children":[]}]});
        let mut items = HashMap::new();
        let item = super::definition_item(
            &definition,
            &mut items,
            &mut HashMap::new(),
            &HashSet::new(),
            false,
        );
        assert_eq!(item.children.len(), 2);
        assert_eq!(item.children[1].id.as_str(), "p:more:scope:800");
        assert!(!items.contains_key("p:more:scope:800"));
    }
}

#[cfg(test)]
mod search_tests {
    use super::{
        RevealStep, SearchHit, hierarchy_rename_edit, hierarchy_resource_rename_edit,
        next_reveal_step, search_hits,
    };
    use serde_json::json;
    use std::collections::HashSet;

    fn hit(node_id: &str, path: &[&str]) -> SearchHit {
        SearchHit {
            node_id: node_id.into(),
            label: node_id.into(),
            kind: "entity".into(),
            path: path.iter().map(|id| (*id).to_owned()).collect(),
        }
    }

    fn requested(ids: &[&str]) -> HashSet<String> {
        ids.iter().map(|id| (*id).to_owned()).collect()
    }

    #[test]
    fn hierarchy_rename_builds_a_project_edit_on_the_wire() {
        let edit = hierarchy_rename_edit("6f0d8a4e-1111-4222-8333-444455556666", "Sphere", "Floor")
            .unwrap();
        serde_json::from_value::<protocol::project::ProjectEdit>(edit)
            .expect("Hierarchy rename edit must match ProjectEdit");
        assert!(
            hierarchy_rename_edit("6f0d8a4e-1111-4222-8333-444455556666", "Sphere", " ").is_none()
        );
        assert!(
            hierarchy_rename_edit("6f0d8a4e-1111-4222-8333-444455556666", "Sphere", "Sphere")
                .is_none()
        );
        let edit =
            hierarchy_resource_rename_edit("6f0d8a4e-1111-4222-8333-444455556666", "Tree", "Oak")
                .unwrap();
        serde_json::from_value::<protocol::project::ProjectEdit>(edit)
            .expect("Hierarchy resource rename edit must match ProjectEdit");
    }

    fn unloaded(id: &str, child_count: u64) -> serde_json::Value {
        json!({"id":id,"label":id,"kind":"scope","child_count":child_count,
            "loaded":false,"children":[],"more_offset":null})
    }

    fn loaded(id: &str, children: serde_json::Value) -> serde_json::Value {
        json!({"id":id,"label":id,"kind":"scope","child_count":children.as_array().map(|c|c.len()).unwrap_or(0),
            "loaded":true,"children":children,"more_offset":null})
    }

    fn tree(children: serde_json::Value) -> serde_json::Value {
        json!({"nodes":children})
    }

    #[test]
    fn search_hits_read_the_wire_shape_and_skip_malformed_records() {
        let page = json!({"nodes":[
            {"node_id":"shape","label":"Sphere","kind":"entity","path":["attribute","world"]},
            {"node_id":"world","label":"World","kind":"scope","path":[]},
            {"label":"no id"},
            {"node_id":"bare"}
        ],"total":2});
        let hits = search_hits(&page);
        assert_eq!(
            hits,
            vec![
                SearchHit {
                    label: "Sphere".into(),
                    ..hit("shape", &["attribute", "world"])
                },
                SearchHit {
                    label: "World".into(),
                    kind: "scope".into(),
                    ..hit("world", &[])
                }
            ]
        );
        assert!(search_hits(&json!({"nodes":[]})).is_empty());
        assert!(search_hits(&serde_json::Value::Null).is_empty());
    }

    #[test]
    fn reveal_expands_the_rootmost_unloaded_ancestor_first() {
        // Immediate parent first in `path`; the walk must start at the root.
        let target = hit("shape", &["attribute", "world"]);
        assert_eq!(
            next_reveal_step(
                &tree(json!([unloaded("world", 1)])),
                &target,
                &requested(&[])
            ),
            RevealStep::Expand("world".into())
        );
        let partly_loaded = tree(json!([loaded("world", json!([unloaded("attribute", 1)]))]));
        assert_eq!(
            next_reveal_step(&partly_loaded, &target, &requested(&[])),
            RevealStep::Expand("attribute".into())
        );
    }

    #[test]
    fn reveal_waits_for_a_requested_page_and_for_missing_parents() {
        let target = hit("shape", &["attribute", "world"]);
        assert_eq!(
            next_reveal_step(
                &tree(json!([unloaded("world", 1)])),
                &target,
                &requested(&["world"])
            ),
            RevealStep::Wait
        );
        let loaded_world = tree(json!([loaded("world", json!([]))]));
        assert_eq!(
            next_reveal_step(&loaded_world, &target, &requested(&[])),
            RevealStep::Wait,
            "the attribute page has not arrived yet"
        );
    }

    #[test]
    fn reveal_completes_once_every_ancestor_and_the_target_are_visible() {
        let target = hit("shape", &["attribute", "world"]);
        let full = tree(json!([loaded(
            "world",
            json!([loaded("attribute", json!([unloaded("shape", 0)]))])
        )]));
        assert_eq!(
            next_reveal_step(&full, &target, &requested(&[])),
            RevealStep::Reveal
        );
        // A root-level hit needs no expansion.
        assert_eq!(
            next_reveal_step(
                &tree(json!([unloaded("world", 1)])),
                &hit("world", &[]),
                &requested(&[])
            ),
            RevealStep::Reveal
        );
        assert_eq!(
            next_reveal_step(
                &tree(json!([unloaded("world", 1)])),
                &hit("gone", &[]),
                &requested(&[])
            ),
            RevealStep::Wait
        );
    }

    #[test]
    fn reveal_pages_a_definition_root_before_selecting_its_created_node() {
        let target = hit("created", &["definition-scope", "@object-definitions"]);
        let unloaded_definition = tree(json!([{
            "id":"@object-definitions","kind":"definition_section","loaded":true,
            "children":[{"id":"definition-scope","kind":"definition_root",
                "loaded":false,"child_count":1,"children":[]}]
        }]));
        assert_eq!(
            next_reveal_step(&unloaded_definition, &target, &requested(&[])),
            RevealStep::Expand("definition-scope".into())
        );
        let loaded_definition = tree(json!([{
            "id":"@object-definitions","kind":"definition_section","loaded":true,
            "children":[{"id":"definition-scope","kind":"definition_root",
                "loaded":true,"child_count":1,"children":[unloaded("created", 0)]}]
        }]));
        assert_eq!(
            next_reveal_step(&loaded_definition, &target, &requested(&[])),
            RevealStep::Reveal
        );
    }
}

#[cfg(test)]
mod icon_tests {
    use super::{IconName, object_icon, object_icon_for, object_label};
    use serde_json::json;

    #[test]
    fn an_area_light_shape_shows_the_lamp_in_the_tree_and_the_inspector() {
        let tree_row =
            json!({"object":"shape","label":"Quad","components":["transform","area_light"]});
        assert_eq!(object_icon(&tree_row), IconName::LampCeiling);
        assert_eq!(object_label(&tree_row), "Quad");
        let plain = json!({"object":"shape","components":["transform"]});
        assert_eq!(object_icon(&plain), IconName::Triangle);
        let detail = json!([{"kind":"transform"},{"kind":"area_light","local":true}]);
        assert_eq!(
            object_icon_for("entity", "shape", &detail),
            IconName::LampCeiling
        );
        // A group keeps its folder and the mark.
        let group = json!({"object":"group","label":"G","components":["area_light"]});
        assert_eq!(object_icon(&group), IconName::Folder);
        assert_eq!(object_label(&group), "G  ✦");
    }
}
