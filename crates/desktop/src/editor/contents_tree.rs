//! Contents folder tree: per-folder pages from `project.list_contents`, the
//! expanded folders, the selection, and the edits the Contents menu sends.
//!
//! Pure so it can be tested without a window. `""` is the `Assets/` root and
//! `"@builtin"` the read-only Built-in folder of presets.
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};

pub const ROOT: &str = "";
pub const BUILTIN: &str = "@builtin";
pub const PAGE_LIMIT: usize = 256;

#[derive(Clone, Debug, Default, PartialEq)]
struct FolderPage {
    /// The project reference the entries were listed for.
    project: Value,
    entries: Vec<Value>,
    total: usize,
    next_offset: Option<usize>,
}

/// What a Contents row stands for.
#[derive(Clone, Debug, PartialEq)]
pub enum Row {
    Folder {
        path: String,
        name: String,
        depth: usize,
        expanded: bool,
        loading: bool,
    },
    Entry {
        entry: Value,
        depth: usize,
    },
    More {
        folder: String,
        offset: usize,
        depth: usize,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub struct ContentsTree {
    pages: BTreeMap<String, FolderPage>,
    expanded: BTreeSet<String>,
    /// Folder shown in the direct-children pane; independent from item selection.
    current_folder: String,
    /// The selected entry as the core listed it.
    selected: Option<Value>,
    pending_selection_refresh: Option<PendingSelectionRefresh>,
    generation: u64,
}

#[derive(Clone, Debug, PartialEq)]
struct PendingSelectionRefresh {
    project: Value,
    entry: Value,
    waiting_for: BTreeSet<String>,
}

impl Default for ContentsTree {
    fn default() -> Self {
        Self {
            pages: BTreeMap::new(),
            expanded: BTreeSet::from([ROOT.to_owned()]),
            current_folder: ROOT.to_owned(),
            selected: None,
            pending_selection_refresh: None,
            generation: 0,
        }
    }
}

impl ContentsTree {
    /// A new activation starts from the root folder only.
    pub fn reset(&mut self) {
        let generation = self.generation.wrapping_add(1);
        *self = Self::default();
        self.generation = generation;
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn current_folder(&self) -> &str {
        &self.current_folder
    }

    /// Set the folder shown on the right. Returns its path if the page needs
    /// to be fetched for this project.
    pub fn select_folder(&mut self, folder: &str, project: &Value) -> Option<String> {
        if self.current_folder != folder {
            self.generation = self.generation.wrapping_add(1);
        }
        self.current_folder = folder.to_owned();
        self.add_pending_folder(folder, project);
        if self.needs_fetch(folder, project) {
            Some(folder.to_owned())
        } else {
            None
        }
    }

    /// Open a folder from the direct-children pane, revealing its tree path.
    pub fn open_folder(&mut self, folder: &str, project: &Value) -> Vec<String> {
        self.current_folder = folder.to_owned();
        let mut path = ancestors(folder);
        if folder != ROOT && !path.iter().any(|candidate| candidate == folder) {
            path.push(folder.to_owned());
        }
        let mut fetch = Vec::new();
        for path in path {
            self.expanded.insert(path.clone());
            self.add_pending_folder(&path, project);
            if self.needs_fetch(&path, project) {
                fetch.push(path);
            }
        }
        self.generation = self.generation.wrapping_add(1);
        fetch
    }

    /// Select a file in either pane: keep the right pane at its parent.
    pub fn select_entry(&mut self, entry: &Value) {
        if let Some(location) = entry["location"].as_str() {
            let folder = if entry["builtin"] == true {
                BUILTIN.to_owned()
            } else if entry["kind"] == "folder" {
                location.to_owned()
            } else {
                parent(location).to_owned()
            };
            if self.current_folder != folder {
                self.generation = self.generation.wrapping_add(1);
            }
            self.current_folder = folder;
        }
        self.select(Some(entry.clone()));
    }

    /// The thumbnail gate for one preview kind — image assets
    /// answer for asset listings, textures and materials for resource
    /// listings. The id alone is not enough: only candidates of the
    /// previewed kind can land a thumbnail.
    pub fn contains_entry(&self, kind: crate::backend::PreviewKind, id: &str) -> bool {
        self.pages
            .values()
            .flat_map(|page| &page.entries)
            .any(|entry| entry_is(entry, kind, id))
    }

    /// Replace a folder's page at offset 0; append later pages of the same
    /// project revision. Pages for another revision restart the folder.
    pub fn merge_page(&mut self, project: &Value, folder: &str, offset: usize, page: &Value) {
        let Some(entries) = page["entries"].as_array() else {
            return;
        };
        let current = self.pages.entry(folder.to_owned()).or_default();
        if offset == 0 || current.project != *project {
            current.entries.clear();
        }
        current.project = project.clone();
        current.entries.extend(entries.iter().cloned());
        current.total = page["total"].as_u64().unwrap_or(0) as usize;
        current.next_offset = page["next_offset"].as_u64().map(|offset| offset as usize);
        self.generation = self.generation.wrapping_add(1);
        self.refresh_selection(project, folder);
    }

    /// A failed listing: forget it and collapse.
    pub fn drop_folder(&mut self, folder: &str) {
        if folder != ROOT {
            let project = self
                .pages
                .get(parent(folder))
                .map(|page| page.project.clone())
                .or_else(|| {
                    self.pending_selection_refresh
                        .as_ref()
                        .map(|pending| pending.project.clone())
                })
                .unwrap_or(Value::Null);
            let mut changed = self.expanded.remove(folder) | self.pages.remove(folder).is_some();
            if self.current_folder == folder
                || self.current_folder.starts_with(&format!("{folder}/"))
            {
                self.current_folder = parent(folder).to_owned();
                changed = true;
            }
            if let Some(selected) = self.selected.clone()
                && selected["location"].as_str().is_some_and(|location| {
                    location == folder || location.starts_with(&format!("{folder}/"))
                })
            {
                let mut waiting_for = self.expanded.clone();
                waiting_for.insert(self.current_folder.clone());
                waiting_for.insert(parent(folder).to_owned());
                waiting_for.remove(folder);
                waiting_for.retain(|candidate| {
                    candidate != folder && !candidate.starts_with(&format!("{folder}/"))
                });
                let expected_project = self
                    .pending_selection_refresh
                    .as_ref()
                    .filter(|pending| same_entry(&pending.entry, &selected))
                    .map(|pending| pending.project.clone())
                    .unwrap_or_else(|| project.clone());
                waiting_for.retain(|candidate| {
                    self.pages.get(candidate).is_none_or(|page| {
                        page.project != expected_project || page.next_offset.is_some()
                    })
                });
                if let Some(pending) = self.pending_selection_refresh.as_mut() {
                    if same_entry(&pending.entry, &selected) {
                        pending.waiting_for.extend(waiting_for);
                        pending.waiting_for.remove(folder);
                    }
                } else {
                    self.pending_selection_refresh = Some(PendingSelectionRefresh {
                        project,
                        entry: selected,
                        waiting_for,
                    });
                }
                if self
                    .pending_selection_refresh
                    .as_ref()
                    .is_some_and(|pending| pending.waiting_for.is_empty())
                {
                    self.pending_selection_refresh = None;
                    self.selected = None;
                }
            }
            if changed {
                self.generation = self.generation.wrapping_add(1);
            }
        }
    }

    /// Expand or collapse a folder. Returns true when its page must be fetched.
    pub fn toggle(&mut self, folder: &str, project: &Value) -> bool {
        self.generation = self.generation.wrapping_add(1);
        if !self.expanded.remove(folder) {
            self.expanded.insert(folder.to_owned());
            self.add_pending_folder(folder, project);
            return self.needs_fetch(folder, project);
        }
        false
    }

    /// Open every folder above `location`; returns the folders to fetch.
    pub fn reveal(&mut self, location: &str, project: &Value) -> Vec<String> {
        let mut fetch = Vec::new();
        let before = self.expanded.len();
        for folder in ancestors(location) {
            self.expanded.insert(folder.clone());
            self.add_pending_folder(&folder, project);
            if self.needs_fetch(&folder, project) {
                fetch.push(folder);
            }
        }
        if self.expanded.len() != before {
            self.generation = self.generation.wrapping_add(1);
        }
        fetch
    }

    fn needs_fetch(&self, folder: &str, project: &Value) -> bool {
        self.pages
            .get(folder)
            .is_none_or(|page| page.project != *project)
    }

    /// Expanded folders whose pages are not for `project` yet, root first.
    pub fn stale_folders(&self, project: &Value) -> Vec<String> {
        let mut folders: BTreeSet<_> = self
            .expanded
            .iter()
            .filter(|folder| self.needs_fetch(folder, project))
            .cloned()
            .collect();
        if self.needs_fetch(&self.current_folder, project) {
            folders.insert(self.current_folder.clone());
        }
        if let Some(pending) = &self.pending_selection_refresh
            && pending.project == *project
        {
            folders.extend(pending.waiting_for.iter().cloned());
        }
        folders.into_iter().collect()
    }

    pub fn is_expanded(&self, folder: &str) -> bool {
        self.expanded.contains(folder)
    }

    pub fn selected(&self) -> Option<&Value> {
        self.selected.as_ref()
    }

    pub fn select(&mut self, entry: Option<Value>) {
        self.pending_selection_refresh = None;
        self.selected = entry;
    }

    /// Whether `entry` is the selected row.
    pub fn is_selected(&self, entry: &Value) -> bool {
        self.selected
            .as_ref()
            .is_some_and(|selected| same_entry(selected, entry))
    }

    /// Keep the selection's row data current after a refresh.
    fn refresh_selection(&mut self, project: &Value, refreshed_folder: &str) {
        let Some(selected) = self.selected.clone() else {
            return;
        };
        if let Some(updated) = self
            .pages
            .values()
            .filter(|page| page.project == *project)
            .flat_map(|page| page.entries.iter())
            .find(|entry| same_entry(&selected, entry))
            .cloned()
        {
            if entry_location_changed(&selected, &updated) {
                self.set_current_folder_for_entry(&updated);
            }
            self.selected = Some(updated);
            self.pending_selection_refresh = None;
            return;
        }
        let source_folder = selection_source_folder(&selected);
        let source_complete = self
            .pages
            .get(source_folder)
            .is_some_and(|page| page.project == *project && page.next_offset.is_none());
        if !source_complete
            || (refreshed_folder != source_folder && self.pending_selection_refresh.is_none())
        {
            return;
        }
        if self
            .pending_selection_refresh
            .as_ref()
            .is_none_or(|pending| {
                pending.project != *project || !same_entry(&pending.entry, &selected)
            })
        {
            let mut waiting_for: BTreeSet<_> = self
                .expanded
                .iter()
                .chain([&self.current_folder])
                .filter(|folder| {
                    self.pages
                        .get(*folder)
                        .is_none_or(|page| page.project != *project || page.next_offset.is_some())
                })
                .cloned()
                .collect();
            waiting_for.insert(source_folder.to_owned());
            self.pending_selection_refresh = Some(PendingSelectionRefresh {
                project: project.clone(),
                entry: selected.clone(),
                waiting_for,
            });
        }
        let Some(pending) = self.pending_selection_refresh.as_mut() else {
            return;
        };
        if pending.project != *project {
            return;
        }
        if self
            .pages
            .get(refreshed_folder)
            .is_some_and(|page| page.project == *project && page.next_offset.is_none())
        {
            pending.waiting_for.remove(refreshed_folder);
        }
        if pending.waiting_for.is_empty() {
            self.selected = None;
            self.pending_selection_refresh = None;
        }
    }

    fn add_pending_folder(&mut self, folder: &str, project: &Value) {
        let is_fresh = self
            .pages
            .get(folder)
            .is_some_and(|page| page.project == *project && page.next_offset.is_none());
        if !is_fresh
            && let Some(pending) = self.pending_selection_refresh.as_mut()
            && pending.project == *project
        {
            pending.waiting_for.insert(folder.to_owned());
        }
    }

    fn set_current_folder_for_entry(&mut self, entry: &Value) {
        let location = entry["location"].as_str().unwrap_or_default();
        let folder = if entry["builtin"] == true {
            BUILTIN.to_owned()
        } else if entry["kind"] == "folder" {
            location.to_owned()
        } else {
            parent(location).to_owned()
        };
        if self.current_folder != folder {
            self.current_folder = folder;
            self.generation = self.generation.wrapping_add(1);
        }
    }

    /// Flatten the tree for drawing: `Assets` and its expanded folders, then
    /// the Built-in folder.
    pub fn rows(&self) -> Vec<Row> {
        let mut rows = Vec::new();
        for (path, name) in [(ROOT, "Assets"), (BUILTIN, "Built-in")] {
            rows.push(Row::Folder {
                path: path.into(),
                name: name.into(),
                depth: 0,
                expanded: self.is_expanded(path),
                loading: self.is_expanded(path) && !self.pages.contains_key(path),
            });
            if self.is_expanded(path) {
                self.append_folder_rows(path, 1, &mut rows);
            }
        }
        rows
    }

    /// Direct children for the right-hand folder view, sharing cached pages
    /// with the hierarchy above.
    pub fn folder_rows(&self) -> Vec<Row> {
        let folder = self.current_folder.as_str();
        let Some(page) = self.pages.get(folder) else {
            return Vec::new();
        };
        let mut rows = page
            .entries
            .iter()
            .map(|entry| {
                if entry["kind"] == "folder" {
                    let path = entry["location"].as_str().unwrap_or_default().to_owned();
                    Row::Folder {
                        name: entry["name"].as_str().unwrap_or_default().to_owned(),
                        expanded: self.is_expanded(&path),
                        loading: self.is_expanded(&path) && !self.pages.contains_key(&path),
                        path,
                        depth: 0,
                    }
                } else {
                    Row::Entry {
                        entry: entry.clone(),
                        depth: 0,
                    }
                }
            })
            .collect::<Vec<_>>();
        if let Some(offset) = page.next_offset {
            rows.push(Row::More {
                folder: folder.to_owned(),
                offset,
                depth: 0,
            });
        }
        rows
    }

    pub fn folder_is_loading(&self, project: &Value) -> bool {
        self.needs_fetch(&self.current_folder, project)
    }

    fn append_folder_rows(&self, folder: &str, depth: usize, rows: &mut Vec<Row>) {
        let Some(page) = self.pages.get(folder) else {
            return;
        };
        for entry in &page.entries {
            if entry["kind"] == "folder" {
                let path = entry["location"].as_str().unwrap_or_default().to_owned();
                let expanded = self.is_expanded(&path);
                rows.push(Row::Folder {
                    name: entry["name"].as_str().unwrap_or_default().to_owned(),
                    depth,
                    expanded,
                    loading: expanded && !self.pages.contains_key(&path),
                    path: path.clone(),
                });
                if expanded {
                    self.append_folder_rows(&path, depth + 1, rows);
                }
            } else {
                rows.push(Row::Entry {
                    entry: entry.clone(),
                    depth,
                });
            }
        }
        if let Some(offset) = page.next_offset {
            rows.push(Row::More {
                folder: folder.to_owned(),
                offset,
                depth,
            });
        }
    }

    /// Names already used in one folder, for New Folder / New Scene.
    pub fn names_in(&self, folder: &str) -> Vec<String> {
        self.pages
            .get(folder)
            .into_iter()
            .flat_map(|page| page.entries.iter())
            .filter_map(|entry| entry["location"].as_str().map(file_name).map(str::to_owned))
            .collect()
    }
}

fn same_entry(a: &Value, b: &Value) -> bool {
    a["kind"] == b["kind"]
        && match (a["id"].as_str(), b["id"].as_str()) {
            (Some(a), Some(b)) => a == b,
            _ => a["location"] == b["location"],
        }
}

fn entry_location_changed(previous: &Value, updated: &Value) -> bool {
    previous["location"] != updated["location"] || previous["builtin"] != updated["builtin"]
}

fn selection_source_folder(entry: &Value) -> &str {
    if entry["builtin"] == true {
        BUILTIN
    } else {
        parent(entry["location"].as_str().unwrap_or_default())
    }
}

/// `Materials/Wood` for `Materials/Wood/oak.material`; `""` at the root.
pub fn parent(location: &str) -> &str {
    location.rsplit_once('/').map_or(ROOT, |(parent, _)| parent)
}

pub fn file_name(location: &str) -> &str {
    location.rsplit('/').next().unwrap_or(location)
}

pub fn join(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// The root and every folder above `location`, outermost first.
fn ancestors(location: &str) -> Vec<String> {
    let mut folders = vec![ROOT.to_owned()];
    let mut folder = parent(location);
    let mut nested = Vec::new();
    while !folder.is_empty() {
        nested.push(folder.to_owned());
        folder = parent(folder);
    }
    folders.extend(nested.into_iter().rev());
    folders
}

/// `stem`, `stem`, `stem`, … whichever `name` is not taken.
pub fn unique_name(taken: &[String], stem: &str, extension: &str) -> String {
    let make = |suffix: Option<usize>| match suffix {
        None => format!("{stem}{extension}"),
        Some(n) => format!("{stem} ({n}){extension}"),
    };
    std::iter::once(None)
        .chain((1..).map(Some))
        .map(make)
        .find(|candidate| {
            !taken
                .iter()
                .any(|name| name.eq_ignore_ascii_case(candidate))
        })
        .expect("an unused name exists")
}

/// A renamed file keeps its extension when the new name has none.
pub fn renamed(location: &str, new_name: &str) -> Option<String> {
    let new_name = new_name.trim();
    if new_name.is_empty() || new_name.contains('/') {
        return None;
    }
    let old = file_name(location);
    let extension = old
        .find('.')
        .filter(|index| *index > 0)
        .map(|index| &old[index..]);
    let name = match extension {
        Some(extension) if !new_name.contains('.') => format!("{new_name}{extension}"),
        _ => new_name.to_owned(),
    };
    Some(join(parent(location), &name))
}

/// The item id `move_item` / `delete_item` expect for a listed entry.
fn item(entry: &Value) -> Option<Value> {
    let kind = entry["kind"].as_str()?;
    matches!(kind, "scene" | "resource" | "asset")
        .then(|| json!({"kind": kind, "id": entry["id"]}))
        .filter(|_| entry["builtin"] != true && entry["id"].is_string())
}

/// The edit that moves an entry to `location`.
pub fn move_edit(entry: &Value, location: &str) -> Option<Value> {
    let from = entry["location"].as_str()?;
    if from == location || location.is_empty() {
        return None;
    }
    if entry["kind"] == "folder" {
        return Some(json!({"kind":"move_folder","from":from,"to":location}));
    }
    Some(json!({"kind":"move_item","item":item(entry)?,"location":location}))
}

/// The edit for dropping an entry on a folder. None when
/// the drop would change nothing or put a folder inside itself.
pub fn drop_edit(entry: &Value, folder: &str) -> Option<Value> {
    if !editable(entry) || folder == BUILTIN {
        return None;
    }
    let location = entry["location"].as_str()?;
    if parent(location) == folder {
        return None;
    }
    if entry["kind"] == "folder"
        && (folder == location || folder.starts_with(&format!("{location}/")))
    {
        return None;
    }
    move_edit(entry, &join(folder, file_name(location)))
}

/// The edit that deletes an entry. Scenes use `delete_scene`.
pub fn delete_edit(entry: &Value) -> Option<Value> {
    match entry["kind"].as_str()? {
        "folder" => Some(json!({"kind":"delete_folder","path":entry["location"]})),
        "scene" => Some(json!({"kind":"delete_scene","scene":entry["id"]})),
        _ => Some(json!({"kind":"delete_item","item":item(entry)?})),
    }
}

/// Whether the Contents menu may rename, move or delete the entry.
pub fn editable(entry: &Value) -> bool {
    entry["builtin"] != true
        && matches!(
            entry["kind"].as_str(),
            Some("folder" | "scene" | "resource" | "asset")
        )
}

/// The folder new folders and scenes go into: the selected folder, the
/// selected entry's folder, or the root.
pub fn target_folder(selected: Option<&Value>) -> String {
    match selected {
        Some(entry) if entry["kind"] == "folder" => {
            entry["location"].as_str().unwrap_or_default().to_owned()
        }
        Some(entry) if entry["builtin"] != true => {
            parent(entry["location"].as_str().unwrap_or_default()).to_owned()
        }
        _ => ROOT.to_owned(),
    }
}

/// Whether a Contents row is the
/// thumbnail target `id` of `kind`: image assets answer for asset rows,
/// textures and materials for resource rows.
pub fn entry_is(entry: &Value, kind: crate::backend::PreviewKind, id: &str) -> bool {
    let expected = match kind {
        crate::backend::PreviewKind::ImageAsset => "asset",
        crate::backend::PreviewKind::Texture | crate::backend::PreviewKind::Material => "resource",
    };
    entry["kind"] == expected && entry["id"].as_str() == Some(id)
}

/// The folder an image/PLY import places into. A folder row aims itself; a
/// resource or asset row aims its own folder, whatever kind the picked
/// file turns out to be (by design the row's folder wins; kinds are not
/// matched). A scene row aims nothing: its folder holds scenes, not
/// resources. Scene rows, builtin rows and no selection place at the
/// Assets root.
pub fn resource_import_folder(selected: Option<&Value>) -> String {
    match selected {
        Some(entry) if entry["kind"] == "folder" => {
            entry["location"].as_str().unwrap_or_default().to_owned()
        }
        Some(entry) if matches!(entry["kind"].as_str(), Some("resource" | "asset")) => {
            parent(entry["location"].as_str().unwrap_or_default()).to_owned()
        }
        _ => ROOT.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::PreviewKind;

    fn page(entries: Value, next: Value) -> Value {
        let total = entries.as_array().unwrap().len();
        json!({"folder":"","entries":entries,"total":total,"next_offset":next})
    }

    #[test]
    fn contents_tree_shows_expanded_folders_then_built_in() {
        let p1 = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"Materials","location":"Materials"},
                    {"kind":"scene","id":"s","name":"Main","location":"Main.scene"}
                ]),
                Value::Null,
            ),
        );
        let rows = tree.rows();
        assert_eq!(rows.len(), 4);
        assert!(
            matches!(&rows[1], Row::Folder { path, expanded: false, depth: 1, .. } if path == "Materials")
        );
        assert!(matches!(&rows[3], Row::Folder { path, .. } if path == BUILTIN));
        assert!(tree.toggle("Materials", &p1));
        tree.merge_page(
            &p1,
            "Materials",
            0,
            &page(
                json!([{"kind":"resource","id":"m","name":"oak","location":"Materials/oak.material"}]),
                json!(1),
            ),
        );
        let rows = tree.rows();
        assert!(matches!(&rows[2], Row::Entry { depth: 2, .. }));
        assert!(matches!(&rows[3], Row::More { offset: 1, .. }));
        assert!(!tree.toggle("Materials", &p1));
    }

    #[test]
    fn tree_rows_include_files_while_folder_rows_only_show_direct_children() {
        let project = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &project,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"Wood","location":"Wood"},
                    {"kind":"asset","id":"root","name":"root.png","location":"root.png"}
                ]),
                json!(2),
            ),
        );
        assert!(tree.select_folder(ROOT, &project).is_none());
        assert_eq!(tree.folder_rows().len(), 3);
        assert!(
            matches!(tree.folder_rows().last(), Some(Row::More { folder, offset: 2, .. }) if folder.is_empty())
        );
        assert!(matches!(tree.rows()[2], Row::Entry { .. }));

        assert!(tree.toggle("Wood", &project));
        tree.merge_page(
            &project,
            "Wood",
            0,
            &page(
                json!([
                    {"kind":"folder","name":"Oak","location":"Wood/Oak"},
                    {"kind":"asset","id":"wood","name":"wood.png","location":"Wood/wood.png"}
                ]),
                Value::Null,
            ),
        );
        assert_eq!(
            tree.folder_rows()
                .iter()
                .filter(|row| matches!(row, Row::Folder { .. } | Row::Entry { .. }))
                .count(),
            2,
            "Assets shows only Wood and root.png"
        );
        assert_eq!(tree.select_folder("Wood", &project), None);
        let direct = tree.folder_rows();
        assert_eq!(direct.len(), 2);
        assert!(matches!(&direct[0], Row::Folder { path, .. } if path == "Wood/Oak"));
        assert!(matches!(&direct[1], Row::Entry { entry, .. } if entry["id"] == "wood"));
        assert!(
            tree.rows()
                .iter()
                .any(|row| { matches!(row, Row::Entry { entry, .. } if entry["id"] == "wood") })
        );
        assert!(
            tree.folder_rows()
                .iter()
                .all(|row| { !matches!(row, Row::Entry { entry, .. } if entry["id"] == "root") })
        );
    }

    /// The thumbnail acceptance gate matches the row kind and the
    /// id together — an asset and a resource can share an id string, but
    /// only the listings of the previewed kind answer.
    #[test]
    fn contains_entry_distinguishes_an_asset_from_a_resource_with_the_same_id() {
        let project = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &project,
            "Images",
            0,
            &page(
                json!([{"kind":"asset","id":"same","media_type":"image/png",
                        "name":"same.png","location":"Images/same.png"}]),
                Value::Null,
            ),
        );
        assert!(tree.contains_entry(PreviewKind::ImageAsset, "same"));
        assert!(
            !tree.contains_entry(PreviewKind::Texture, "same"),
            "an asset id alone must not pass the resource gate"
        );
        tree.merge_page(
            &project,
            "Materials",
            0,
            &page(
                json!([{"kind":"resource","resource_kind":"texture","id":"same",
                        "name":"oak","location":"Materials/same.texture"}]),
                Value::Null,
            ),
        );
        assert!(tree.contains_entry(PreviewKind::Texture, "same"));
        assert!(tree.contains_entry(PreviewKind::ImageAsset, "same"));
        // The gate is coarse between texture and material on purpose:
        // `of_entry` already fixed the id↔kind pairing when the candidate
        // was queued; here only asset-vs-resource still needs separating.
        assert!(tree.contains_entry(PreviewKind::Material, "same"));
        assert!(!tree.contains_entry(PreviewKind::Texture, "missing"));
        assert!(!tree.contains_entry(PreviewKind::ImageAsset, "missing"));
    }

    /// Search hits share the row shape, so the same test admits a hit whose
    /// folder was never opened.
    #[test]
    fn entry_is_matches_search_hits_by_kind_and_id() {
        let hit = json!({"kind":"asset","id":"sky","media_type":"image/x-exr",
                         "name":"sky.exr","location":"Images/sky.exr"});
        assert!(entry_is(&hit, PreviewKind::ImageAsset, "sky"));
        assert!(!entry_is(&hit, PreviewKind::Texture, "sky"));
        assert!(!entry_is(&hit, PreviewKind::ImageAsset, "other"));
        let texture = json!({"kind":"resource","resource_kind":"texture","id":"oak"});
        assert!(entry_is(&texture, PreviewKind::Texture, "oak"));
        assert!(entry_is(&texture, PreviewKind::Material, "oak"));
    }

    #[test]
    fn selecting_files_and_opening_folders_keep_navigation_and_selection_coherent() {
        let project = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &project,
            ROOT,
            0,
            &page(
                json!([{"kind":"folder","name":"A","location":"A"}]),
                Value::Null,
            ),
        );
        tree.merge_page(
            &project,
            "A",
            0,
            &page(
                json!([
                    {"kind":"folder","name":"B","location":"A/B"},
                    {"kind":"asset","id":"asset","name":"x.png","location":"A/x.png"}
                ]),
                Value::Null,
            ),
        );
        let asset = json!({"kind":"asset","id":"asset","name":"x.png","location":"A/x.png"});
        tree.select_entry(&asset);
        assert_eq!(tree.current_folder(), "A");
        assert!(tree.is_selected(&asset));

        let fetch = tree.open_folder("A/B", &project);
        assert_eq!(tree.current_folder(), "A/B");
        assert!(tree.is_expanded(ROOT));
        assert!(tree.is_expanded("A"));
        assert!(tree.is_expanded("A/B"));
        assert_eq!(fetch, ["A/B"]);

        let selected_folder = json!({"kind":"folder","name":"A","location":"A"});
        tree.select_entry(&selected_folder);
        assert_eq!(tree.current_folder(), "A");

        let builtin = json!({
            "kind":"resource", "id":"builtin-preset", "builtin":true,
            "location":"@builtin"
        });
        tree.select_entry(&builtin);
        assert_eq!(tree.current_folder(), BUILTIN);
    }

    #[test]
    fn opening_a_folder_keeps_single_click_selection_separate_from_navigation() {
        let project = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &project,
            ROOT,
            0,
            &page(
                json!([{"kind":"folder","name":"A","location":"A"}]),
                Value::Null,
            ),
        );
        let folder = json!({"kind":"folder","name":"A","location":"A"});
        tree.select(Some(folder));
        assert_eq!(
            tree.current_folder(),
            ROOT,
            "selecting a right-pane folder alone does not navigate"
        );
        assert!(!tree.is_expanded("A"));

        tree.open_folder("A", &project);
        assert_eq!(tree.current_folder(), "A");
        assert!(tree.is_expanded("A"));
    }

    #[test]
    fn refreshing_a_selected_right_pane_folder_does_not_open_it() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let folder = json!({"kind":"folder","name":"A","location":"A"});
        let mut tree = ContentsTree::default();
        tree.merge_page(&p1, ROOT, 0, &page(json!([folder.clone()]), Value::Null));
        tree.select(Some(folder.clone()));
        tree.merge_page(&p2, ROOT, 0, &page(json!([folder]), Value::Null));
        assert_eq!(tree.current_folder(), ROOT);
        assert_eq!(tree.selected().unwrap()["location"], "A");
    }

    #[test]
    fn current_unexpanded_folder_is_refreshed_and_deleted_folder_falls_back_to_parent() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([{"kind":"folder","name":"A","location":"A"}]),
                Value::Null,
            ),
        );
        tree.merge_page(&p1, "A", 0, &page(json!([]), Value::Null));
        assert!(tree.select_folder("A", &p2).is_some());
        assert_eq!(tree.stale_folders(&p2), ["", "A"]);
        tree.select(Some(json!({"kind":"folder","location":"A"})));
        tree.drop_folder("A");
        assert_eq!(tree.current_folder(), ROOT);
        assert!(tree.selected().is_none());
    }

    #[test]
    fn contents_tree_refetches_expanded_folders_for_a_new_revision() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        tree.merge_page(&p1, ROOT, 0, &page(json!([]), Value::Null));
        tree.toggle("Wood", &p1);
        tree.merge_page(&p1, "Wood", 0, &page(json!([]), Value::Null));
        assert!(tree.stale_folders(&p1).is_empty());
        assert_eq!(tree.stale_folders(&p2), ["", "Wood"]);
        tree.drop_folder("Wood");
        assert_eq!(tree.stale_folders(&p2), [""]);
    }

    #[test]
    fn contents_tree_generation_changes_when_rows_change_and_survives_reset() {
        let mut tree = ContentsTree::default();
        let initial = tree.generation();
        tree.toggle("Materials", &json!({"revision":"1"}));
        assert!(tree.generation() > initial);
        let expanded = tree.generation();
        tree.reset();
        assert!(tree.generation() > expanded);
    }

    #[test]
    fn contents_tree_appends_pages_of_one_revision_only() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([{"kind":"asset","id":"a","location":"a.ply"}]),
                json!(1),
            ),
        );
        tree.merge_page(
            &p1,
            ROOT,
            1,
            &page(
                json!([{"kind":"asset","id":"b","location":"b.ply"}]),
                Value::Null,
            ),
        );
        assert_eq!(tree.names_in(ROOT), ["a.ply", "b.ply"]);
        tree.merge_page(
            &p2,
            ROOT,
            1,
            &page(
                json!([{"kind":"asset","id":"c","location":"c.ply"}]),
                Value::Null,
            ),
        );
        assert_eq!(tree.names_in(ROOT), ["c.ply"]);
    }

    #[test]
    fn contents_tree_reveal_opens_every_ancestor() {
        let p1 = json!({"revision":"1"});
        let mut tree = ContentsTree::default();
        tree.merge_page(&p1, ROOT, 0, &page(json!([]), Value::Null));
        assert_eq!(
            tree.reveal("Materials/Wood/oak.material", &p1),
            ["Materials", "Materials/Wood"]
        );
        assert!(tree.is_expanded("Materials/Wood"));
    }

    #[test]
    fn contents_tree_selection_follows_refreshed_rows() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        let entry = json!({"kind":"resource","id":"m","name":"oak","location":"oak.material","reference_count":1});
        tree.merge_page(&p1, ROOT, 0, &page(json!([entry]), Value::Null));
        tree.select(Some(entry.clone()));
        let moved = json!({"kind":"resource","id":"m","name":"oak","location":"oak.material","reference_count":3});
        tree.merge_page(&p2, ROOT, 0, &page(json!([moved]), Value::Null));
        assert_eq!(tree.selected().unwrap()["reference_count"], 3);
    }

    #[test]
    fn moved_selection_tracks_its_new_folder_and_deleted_selection_is_cleared() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"A","location":"A"},
                    {"kind":"folder","name":"B","location":"B"}
                ]),
                Value::Null,
            ),
        );
        tree.merge_page(
            &p1,
            "A",
            0,
            &page(
                json!([{"kind":"asset","id":"x","location":"A/x.png"}]),
                Value::Null,
            ),
        );
        let asset = json!({"kind":"asset","id":"x","location":"A/x.png"});
        tree.select_entry(&asset);
        assert_eq!(tree.current_folder(), "A");
        tree.toggle("B", &p1);
        tree.merge_page(&p1, "B", 0, &page(json!([]), Value::Null));
        tree.select_folder("B", &p1);
        tree.select_entry(&asset);
        tree.merge_page(
            &p2,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"A","location":"A"},
                    {"kind":"folder","name":"B","location":"B"}
                ]),
                Value::Null,
            ),
        );
        tree.merge_page(&p2, "A", 0, &page(json!([]), Value::Null));
        assert!(
            tree.is_selected(&asset),
            "retain selection while destination refresh is pending"
        );
        tree.merge_page(
            &p2,
            "B",
            0,
            &page(
                json!([{"kind":"asset","id":"x","location":"B/x.png"}]),
                Value::Null,
            ),
        );
        assert_eq!(tree.current_folder(), "B");
        assert_eq!(tree.selected().unwrap()["location"], "B/x.png");

        let mut deleted = ContentsTree::default();
        deleted.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"A","location":"A"},
                    {"kind":"folder","name":"B","location":"B"}
                ]),
                Value::Null,
            ),
        );
        deleted.merge_page(&p1, "A", 0, &page(json!([asset.clone()]), Value::Null));
        deleted.merge_page(&p1, "B", 0, &page(json!([]), Value::Null));
        deleted.select_entry(&asset);
        deleted.toggle("B", &p1);
        deleted.select_folder("B", &p1);
        deleted.select_entry(&asset);
        deleted.merge_page(
            &p2,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"A","location":"A"},
                    {"kind":"folder","name":"B","location":"B"}
                ]),
                Value::Null,
            ),
        );
        deleted.merge_page(&p2, "A", 0, &page(json!([]), Value::Null));
        assert!(deleted.is_selected(&asset));
        deleted.merge_page(&p2, "B", 0, &page(json!([]), Value::Null));
        assert!(
            deleted.selected().is_none(),
            "clear only after destination scope is refreshed"
        );
    }

    #[test]
    fn dropped_source_folder_keeps_selection_until_refreshed_scope_resolves_it() {
        let project = json!({"revision":"1"});
        let asset = json!({"kind":"asset","id":"x","location":"A/x.png"});
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &project,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"A","location":"A"},
                    {"kind":"folder","name":"B","location":"B"}
                ]),
                Value::Null,
            ),
        );
        tree.merge_page(&project, "A", 0, &page(json!([asset.clone()]), Value::Null));
        tree.merge_page(&project, "B", 0, &page(json!([]), Value::Null));
        tree.toggle("B", &project);
        tree.select_entry(&asset);
        let refreshed = json!({"revision":"2"});
        tree.merge_page(&refreshed, "A", 0, &page(json!([]), Value::Null));
        assert!(tree.is_selected(&asset));
        tree.drop_folder("A");
        assert!(tree.is_selected(&asset));
        assert!(tree.stale_folders(&refreshed).contains(&"B".to_owned()));
        tree.merge_page(
            &refreshed,
            ROOT,
            0,
            &page(
                json!([
                    {"kind":"folder","name":"B","location":"B"},
                    {"kind":"folder","name":"A2","location":"A2"}
                ]),
                Value::Null,
            ),
        );
        tree.merge_page(
            &refreshed,
            "B",
            0,
            &page(
                json!([{
                    "kind":"asset","id":"x","location":"B/x.png"
                }]),
                Value::Null,
            ),
        );
        assert_eq!(tree.current_folder(), "B");
        assert_eq!(tree.selected().unwrap()["location"], "B/x.png");
    }

    #[test]
    fn renamed_or_deleted_current_folder_returns_to_its_parent() {
        let (p1, p2) = (json!({"revision":"1"}), json!({"revision":"2"}));
        let mut tree = ContentsTree::default();
        tree.merge_page(
            &p1,
            ROOT,
            0,
            &page(
                json!([{"kind":"folder","name":"A","location":"A"}]),
                Value::Null,
            ),
        );
        tree.select(Some(json!({"kind":"folder","name":"A","location":"A"})));
        tree.select_folder("A", &p1);
        tree.merge_page(
            &p2,
            ROOT,
            0,
            &page(
                json!([{"kind":"folder","name":"A2","location":"A2"}]),
                Value::Null,
            ),
        );
        tree.drop_folder("A");
        assert_eq!(tree.current_folder(), ROOT);
        assert!(tree.selected().is_none());
    }

    #[test]
    fn contents_tree_names_and_locations() {
        assert_eq!(parent("Materials/Wood/oak.material"), "Materials/Wood");
        assert_eq!(parent("oak.material"), "");
        assert_eq!(join("", "a"), "a");
        assert_eq!(join("A", "b"), "A/b");
        let taken = vec!["New Folder".to_owned(), "new folder (1)".to_owned()];
        assert_eq!(unique_name(&taken, "New Folder", ""), "New Folder (2)");
        assert_eq!(unique_name(&[], "Scene", ".scene"), "Scene.scene");
        assert_eq!(
            renamed("Materials/oak.material", "pine").as_deref(),
            Some("Materials/pine.material")
        );
        assert_eq!(
            renamed("Materials/oak.material", "pine.texture").as_deref(),
            Some("Materials/pine.texture")
        );
        assert_eq!(renamed("Wood", "Oak").as_deref(), Some("Oak"));
        assert_eq!(renamed("a.ply", " "), None);
        assert_eq!(renamed("a.ply", "x/y"), None);
    }

    #[test]
    fn contents_drop_moves_into_the_target_folder() {
        let resource = json!({"kind":"resource","id":"r","location":"Materials/oak.material"});
        assert_eq!(
            drop_edit(&resource, "Wood"),
            Some(
                json!({"kind":"move_item","item":{"kind":"resource","id":"r"},"location":"Wood/oak.material"})
            )
        );
        assert_eq!(
            drop_edit(&resource, ""),
            Some(
                json!({"kind":"move_item","item":{"kind":"resource","id":"r"},"location":"oak.material"})
            )
        );
        assert_eq!(drop_edit(&resource, "Materials"), None, "same folder");
        assert_eq!(drop_edit(&resource, BUILTIN), None);
        let folder = json!({"kind":"folder","location":"Materials/Wood"});
        assert_eq!(
            drop_edit(&folder, ""),
            Some(json!({"kind":"move_folder","from":"Materials/Wood","to":"Wood"}))
        );
        assert_eq!(drop_edit(&folder, "Materials/Wood"), None, "into itself");
        assert_eq!(
            drop_edit(&folder, "Materials/Wood/Oak"),
            None,
            "into a child"
        );
        assert!(drop_edit(&folder, "Materials/Woodwork").is_some());
        let preset = json!({"kind":"resource","id":"p","location":"@builtin","builtin":true});
        assert_eq!(drop_edit(&preset, ""), None);
    }

    #[test]
    fn contents_tree_edits_match_the_wire() {
        let resource = json!({"kind":"resource","id":"r","location":"oak.material"});
        assert_eq!(
            move_edit(&resource, "Wood/oak.material"),
            Some(
                json!({"kind":"move_item","item":{"kind":"resource","id":"r"},"location":"Wood/oak.material"})
            )
        );
        assert_eq!(move_edit(&resource, "oak.material"), None);
        let folder = json!({"kind":"folder","location":"Wood"});
        assert_eq!(
            move_edit(&folder, "Materials/Wood"),
            Some(json!({"kind":"move_folder","from":"Wood","to":"Materials/Wood"}))
        );
        assert_eq!(
            delete_edit(&folder),
            Some(json!({"kind":"delete_folder","path":"Wood"}))
        );
        let scene = json!({"kind":"scene","id":"s","location":"Night.scene"});
        assert_eq!(
            delete_edit(&scene),
            Some(json!({"kind":"delete_scene","scene":"s"}))
        );
        let preset = json!({"kind":"resource","id":"p","location":"@builtin","builtin":true});
        assert!(!editable(&preset));
        assert_eq!(move_edit(&preset, "x"), None);
        assert!(!editable(
            &json!({"kind":"unregistered","location":"x.png"})
        ));
        assert_eq!(target_folder(Some(&folder)), "Wood");
        assert_eq!(
            target_folder(Some(&json!({"kind":"asset","location":"Meshes/a.ply"}))),
            "Meshes"
        );
        assert_eq!(target_folder(Some(&preset)), "");
        assert_eq!(target_folder(None), "");
    }

    /// review minor: a scene row never aims a resource import. Its
    /// folder holds scenes, so an import from a scene selection places at
    /// the Assets root.
    #[test]
    fn resource_imports_ignore_scene_rows_but_follow_item_folders() {
        let folder = json!({"kind":"folder","location":"Wood"});
        let texture = json!({"kind":"resource","id":"t","location":"Textures/oak.texture"});
        let mesh = json!({"kind":"asset","id":"m","location":"Meshes/box.ply"});
        let scene = json!({"kind":"scene","id":"s","location":"Scenes/main.scene"});
        let preset = json!({"kind":"resource","id":"p","location":"@builtin","builtin":true});
        assert_eq!(resource_import_folder(Some(&folder)), "Wood");
        assert_eq!(resource_import_folder(Some(&texture)), "Textures");
        assert_eq!(resource_import_folder(Some(&mesh)), "Meshes");
        assert_eq!(resource_import_folder(Some(&scene)), "");
        assert_eq!(resource_import_folder(Some(&preset)), "");
        assert_eq!(resource_import_folder(None), "");
    }
}
